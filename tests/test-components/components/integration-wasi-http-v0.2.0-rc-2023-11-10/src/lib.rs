wit_bindgen::generate!({
    path: "../../../../wit",
    world: "wasi:http/proxy@0.2.0-rc-2023-11-10",
    generate_all,
});

use {
    exports::wasi::http0_2_0_rc_2023_11_10::incoming_handler,
    url::Url,
    wasi::{
        http0_2_0_rc_2023_11_10::{
            outgoing_handler,
            types::{
                Headers, IncomingRequest, Method, OutgoingBody, OutgoingRequest, OutgoingResponse,
                ResponseOutparam, Scheme,
            },
        },
        io0_2_0_rc_2023_11_10::streams::StreamError,
    },
};

struct Component;

export!(Component);

impl incoming_handler::Guest for Component {
    fn handle(request: IncomingRequest, outparam: ResponseOutparam) {
        let headers = request.headers().entries();

        if let Some(url) = headers.iter().find_map(|(k, v)| {
            (k == "url")
                .then_some(v)
                .and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|v| Url::parse(v).ok())
        }) {
            let headers = Headers::new();
            headers
                .append(&"Content-Length".into(), &"13".into())
                .unwrap();
            let outgoing_request = OutgoingRequest::new(headers);
            outgoing_request.set_method(&Method::Post).unwrap();
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
            // Write the request body.
            let outgoing_body = outgoing_request.body().unwrap();
            {
                let outgoing_stream = outgoing_body.write().unwrap();
                let message = b"Hello, world!";
                let mut offset = 0;
                loop {
                    let write = outgoing_stream.check_write().unwrap();
                    if write == 0 {
                        outgoing_stream.subscribe().block();
                    } else {
                        let count = (write as usize).min(message.len() - offset);
                        outgoing_stream.write(&message[offset..][..count]).unwrap();
                        offset += count;
                        if offset == message.len() {
                            outgoing_stream.flush().unwrap();
                            break;
                        }
                    }
                }
                // The outgoing stream must be dropped before the outgoing body is finished.
            }
            OutgoingBody::finish(outgoing_body, None).unwrap();

            let incoming_response = outgoing_handler::handle(outgoing_request, None).unwrap();
            let response = loop {
                if let Some(response) = incoming_response.get() {
                    break response.unwrap().unwrap();
                } else {
                    incoming_response.subscribe().block()
                }
            };

            let incoming_body = response.consume().unwrap();
            let incoming_stream = incoming_body.stream().unwrap();
            let status = response.status();
            let response = OutgoingResponse::new(response.headers().clone());
            response.set_status_code(status).unwrap();
            let outgoing_body = response.body().unwrap();
            {
                let outgoing_stream = outgoing_body.write().unwrap();
                ResponseOutparam::set(outparam, Ok(response));

                loop {
                    match incoming_stream.read(1024) {
                        Ok(buffer) => {
                            if buffer.is_empty() {
                                incoming_stream.subscribe().block();
                            } else {
                                outgoing_stream.blocking_write_and_flush(&buffer).unwrap();
                            }
                        }
                        Err(StreamError::Closed) => break,
                        Err(StreamError::LastOperationFailed(error)) => {
                            panic!("{}", error.to_debug_string())
                        }
                    }
                }
            }

            OutgoingBody::finish(outgoing_body, None).unwrap();
        } else {
            let response = OutgoingResponse::new(Headers::new());
            response.set_status_code(400).unwrap();
            let body = response.body().unwrap();

            ResponseOutparam::set(outparam, Ok(response));

            OutgoingBody::finish(body, None).unwrap();
        }
    }
}
