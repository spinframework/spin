#![deny(warnings)]

wit_bindgen::generate!({
    path: "../../../../wit",
    world: "wasi:http/service@0.3.0-rc-2026-01-06",
    generate_all,
});

use {
    crate::{
        exports::wasi::http0_3_0_rc_2026_01_06::handler::Guest,
        wasi::http0_3_0_rc_2026_01_06::{
            client,
            types::{ErrorCode, Fields, Method, Request, Response, Scheme},
        },
    },
    core::mem,
    futures::{stream, StreamExt},
    url::Url,
    wit_bindgen::{rt::async_support, StreamResult},
};

const MAX_CONCURRENCY: usize = 16;

struct Component;

export!(Component);

impl Guest for Component {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let headers = request.get_headers().copy_all();

        Ok(
            match (
                request.get_method(),
                request.get_path_with_query().as_deref(),
            ) {
                (Method::Get, Some("/hash-all")) => {
                    // Send outgoing GET requests to the specified URLs and stream
                    // the hashes of the response bodies as they arrive.

                    let urls = headers.iter().filter_map(|(k, v)| {
                        (k == "url")
                            .then_some(v)
                            .and_then(|v| std::str::from_utf8(v).ok())
                            .and_then(|v| Url::parse(v).ok())
                    });

                    let results = urls
                        .map(|url| async move {
                            let result = hash(&url).await;
                            (url, result)
                        })
                        .collect::<Vec<_>>();

                    let mut results = stream::iter(results).buffer_unordered(MAX_CONCURRENCY);
                    let (mut tx, rx) = wit_stream::new();
                    async_support::spawn(async move {
                        while let Some((url, result)) = results.next().await {
                            tx.write_all(
                                match result {
                                    Ok(hash) => format!("{url}: {hash}\n"),
                                    Err(e) => format!("{url}: {e:?}\n"),
                                }
                                .into_bytes(),
                            )
                            .await;
                        }
                    });

                    Response::new(
                        Fields::from_list(&[("content-type".to_string(), b"text/plain".to_vec())])
                            .unwrap(),
                        Some(rx),
                        wit_future::new(|| Ok(None)).1,
                    )
                    .0
                }

                (Method::Post, Some("/echo")) => {
                    // Echo the request body without buffering it.

                    let (rx, trailers) =
                        Request::consume_body(request, wit_future::new(|| Ok(())).1);
                    Response::new(
                        Fields::from_list(
                            &headers
                                .into_iter()
                                .filter_map(|(k, v)| (k == "content-type").then_some((k, v)))
                                .collect::<Vec<_>>(),
                        )
                        .unwrap(),
                        Some(rx),
                        trailers,
                    )
                    .0
                }

                (Method::Post, Some("/double-echo")) => {
                    // Pipe the request body to an outgoing request and stream the response back to the client.

                    if let Some(url) = headers.iter().find_map(|(k, v)| {
                        (k == "url")
                            .then_some(v)
                            .and_then(|v| std::str::from_utf8(v).ok())
                            .and_then(|v| Url::parse(v).ok())
                    }) {
                        let method = request.get_method();
                        let (rx, trailers) =
                            Request::consume_body(request, wit_future::new(|| Ok(())).1);
                        let outgoing_request =
                            Request::new(Fields::new(), Some(rx), trailers, None).0;
                        outgoing_request.set_method(&method).unwrap();
                        outgoing_request
                            .set_path_with_query(Some(url.path()))
                            .unwrap();
                        outgoing_request
                            .set_scheme(Some(&match url.scheme() {
                                "http" => Scheme::Http,
                                "https" => Scheme::Https,
                                scheme => Scheme::Other(scheme.into()),
                            }))
                            .unwrap();
                        outgoing_request
                            .set_authority(Some(url.authority()))
                            .unwrap();
                        client::send(outgoing_request).await?
                    } else {
                        bad_request()
                    }
                }

                _ => method_not_allowed(),
            },
        )
    }
}

fn bad_request() -> Response {
    respond(400)
}

fn method_not_allowed() -> Response {
    respond(405)
}

fn respond(status: u16) -> Response {
    let response = Response::new(Fields::new(), None, wit_future::new(|| Ok(None)).1).0;
    response.set_status_code(status).unwrap();
    response
}

async fn hash(url: &Url) -> Result<String, ErrorCode> {
    let request = Request::new(Fields::new(), None, wit_future::new(|| Ok(None)).1, None).0;
    request.set_path_with_query(Some(url.path())).unwrap();
    request
        .set_scheme(Some(&match url.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            scheme => Scheme::Other(scheme.into()),
        }))
        .unwrap();
    request.set_authority(Some(url.authority())).unwrap();

    let response = client::send(request).await?;

    let status = response.get_status_code();

    if !(200..300).contains(&status) {
        return Err(ErrorCode::InternalError(Some(format!(
            "unexpected status: {status}"
        ))));
    }

    let (mut rx, trailers) = Response::consume_body(response, wit_future::new(|| Ok(())).1);

    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = Vec::with_capacity(16 * 1024);
    let mut result = StreamResult::Complete(0);
    while let StreamResult::Complete(_) = result {
        (result, buffer) = rx.read(mem::take(&mut buffer)).await;
        hasher.update(&buffer);
        buffer.clear();
    }

    trailers.await?;

    Ok(hex::encode(hasher.finalize()))
}
