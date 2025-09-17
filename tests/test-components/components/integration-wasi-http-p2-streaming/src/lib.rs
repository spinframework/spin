use anyhow::{bail, Result};
use futures::{future, stream, Future, SinkExt, StreamExt, TryStreamExt};
use spin_sdk::{
    http::{
        self, Headers, IncomingRequest, IncomingResponse, Method, OutgoingBody, OutgoingRequest,
        OutgoingResponse, ResponseOutparam, Scheme,
    },
    http_component,
};
use url::Url;

const MAX_CONCURRENCY: usize = 16;

#[http_component]
async fn handle_request(request: IncomingRequest, response_out: ResponseOutparam) {
    let headers = request.headers().entries();

    match (request.method(), request.path_with_query().as_deref()) {
        (Method::Get, Some("/hash-all")) => {
            // Send outgoing GET requests to the specified URLs and stream the hashes of the response bodies as
            // they arrive.

            let urls = headers.iter().filter_map(|(k, v)| {
                (k == "url")
                    .then_some(v)
                    .and_then(|v| std::str::from_utf8(v).ok())
                    .and_then(|v| Url::parse(v).ok())
            });

            let results = urls.map(|url| async move {
                let result = hash(&url).await;
                (url, result)
            });

            let mut results = stream::iter(results).buffer_unordered(MAX_CONCURRENCY);

            let response = OutgoingResponse::new(
                200,
                &Headers::new(&[("content-type".to_string(), b"text/plain".to_vec())]),
            );

            let mut body = response.take_body();

            response_out.set(response);

            while let Some((url, result)) = results.next().await {
                let payload = match result {
                    Ok(hash) => format!("{url}: {hash}\n"),
                    Err(e) => format!("{url}: {e:?}\n"),
                }
                .into_bytes();
                if let Err(e) = body.send(payload).await {
                    eprintln!("Error sending payload: {e}");
                }
            }
        }

        (Method::Post, Some("/echo")) => {
            // Echo the request body without buffering it.

            let response = OutgoingResponse::new(
                200,
                &Headers::new(
                    &headers
                        .into_iter()
                        .filter_map(|(k, v)| (k == "content-type").then_some((k, v)))
                        .collect::<Vec<_>>(),
                ),
            );

            let mut body = response.take_body();

            response_out.set(response);

            let mut stream = request.into_body_stream();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => {
                        if let Err(e) = body.send(chunk).await {
                            eprintln!("Error sending body: {e}");
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Error receiving body: {e}");
                        break;
                    }
                }
            }
        }

        (Method::Post, Some("/double-echo")) => {
            // Pipe the request body to an outgoing request and stream the response back to the client.

            if let Some(url) = headers.iter().find_map(|(k, v)| {
                (k == "url")
                    .then_some(v)
                    .and_then(|v| std::str::from_utf8(v).ok())
                    .and_then(|v| Url::parse(v).ok())
            }) {
                match double_echo(request, &url).await {
                    Ok((request_copy, incoming_response)) => {
                        let mut incoming_response_body = incoming_response.take_body_stream();

                        let outgoing_response = OutgoingResponse::new(
                            200,
                            &Headers::new(
                                &headers
                                    .into_iter()
                                    .filter(|(k, _)| k == "content-type")
                                    .collect::<Vec<_>>(),
                            ),
                        );

                        let mut outgoing_response_body = outgoing_response.take_body();

                        response_out.set(outgoing_response);

                        let response_copy = async move {
                            while let Some(chunk) = incoming_response_body.next().await {
                                outgoing_response_body.send(chunk?).await?;
                            }
                            Ok::<_, anyhow::Error>(())
                        };

                        let (request_copy, response_copy) =
                            future::join(request_copy, response_copy).await;

                        if let Err(e) = request_copy.and(response_copy) {
                            eprintln!("error piping to and from {url}: {e}");
                        }
                    }

                    Err(e) => {
                        eprintln!("Error sending outgoing request to {url}: {e}");
                        server_error(response_out);
                    }
                }
            } else {
                bad_request(response_out);
            }
        }

        _ => method_not_allowed(response_out),
    }
}

async fn double_echo(
    incoming_request: IncomingRequest,
    url: &Url,
) -> Result<(impl Future<Output = Result<()>>, IncomingResponse)> {
    let outgoing_request = OutgoingRequest::new(
        &Method::Post,
        Some(url.path()),
        Some(&match url.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            scheme => Scheme::Other(scheme.into()),
        }),
        Some(url.authority()),
        &Headers::new(&[]),
    );

    let mut body = outgoing_request.take_body();

    let response = http::send::<_, IncomingResponse>(outgoing_request).await?;

    let status = response.status();

    if !(200..300).contains(&status) {
        bail!("unexpected status: {status}");
    }

    let mut stream = incoming_request.into_body_stream();

    let copy = async move {
        while let Some(chunk) = stream.next().await {
            body.send(chunk?).await?;
        }
        Ok::<_, anyhow::Error>(())
    };

    Ok((copy, response))
}

fn server_error(response_out: ResponseOutparam) {
    respond(500, response_out)
}

fn bad_request(response_out: ResponseOutparam) {
    respond(400, response_out)
}

fn method_not_allowed(response_out: ResponseOutparam) {
    respond(405, response_out)
}

fn respond(status: u16, response_out: ResponseOutparam) {
    let response = OutgoingResponse::new(status, &Headers::new(&[]));

    let body = response.write().expect("response should be writable");

    response_out.set(response);

    OutgoingBody::finish(body, None);
}

async fn hash(url: &Url) -> Result<String> {
    let request = OutgoingRequest::new(
        &Method::Get,
        Some(url.path()),
        Some(&match url.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            scheme => Scheme::Other(scheme.into()),
        }),
        Some(url.authority()),
        &Headers::new(&[]),
    );

    let response: IncomingResponse = http::send(request).await?;

    let status = response.status();

    if !(200..300).contains(&status) {
        bail!("unexpected status: {status}");
    }

    let mut body = response.take_body_stream();

    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    while let Some(chunk) = body.try_next().await? {
        hasher.update(&chunk);
    }

    Ok(hex::encode(hasher.finalize()))
}
