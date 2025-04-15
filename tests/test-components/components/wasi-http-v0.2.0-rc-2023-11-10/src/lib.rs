wit_bindgen::generate!({
    path: "wit",
    world: "wasi:http/handler@0.2.0-rc-2023-11-10",
    generate_all,
});

use helper::{ensure_eq, ensure_ok};
use wasi::http::outgoing_handler;
use wasi::http::types::{Headers, Method, OutgoingBody, OutgoingRequest, Scheme};
use wasi::io::streams::StreamError;

helper::define_component!(Component);

impl Component {
    fn main() -> Result<(), String> {
        let url = ensure_ok!(url::Url::parse(&ensure_ok!(std::env::var("URL"))));

        let headers = Headers::new();
        headers
            .append("Content-Length", &"13".as_bytes())
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
        let outgoing_stream = outgoing_body.write().unwrap();
        let message = b"Hello, world!";
        let mut offset = 0;
        loop {
            let write = ensure_ok!(outgoing_stream.check_write());
            if write == 0 {
                outgoing_stream.subscribe().block();
            } else {
                let count = (write as usize).min(message.len() - offset);
                ensure_ok!(outgoing_stream.write(&message[offset..][..count]));
                offset += count;
                if offset == message.len() {
                    ensure_ok!(outgoing_stream.flush());
                    break;
                }
            }
        }
        // The outgoing stream must be dropped before the outgoing body is finished.
        drop(outgoing_stream);
        ensure_ok!(OutgoingBody::finish(outgoing_body, None));

        // Send the request and get the response.
        let incoming_response = ensure_ok!(outgoing_handler::handle(outgoing_request, None));
        let incoming_response = loop {
            if let Some(incoming_response) = incoming_response.get() {
                break ensure_ok!(incoming_response.unwrap());
            } else {
                incoming_response.subscribe().block()
            }
        };

        let incoming_body = incoming_response.consume().unwrap();
        let incoming_stream = incoming_body.stream().unwrap();

        // Read the response body.
        let mut incoming_buffer = Vec::new();
        loop {
            match incoming_stream.read(1024) {
                Ok(buffer) => {
                    if buffer.is_empty() {
                        incoming_stream.subscribe().block();
                    } else {
                        incoming_buffer.extend_from_slice(&buffer);
                    }
                }
                Err(StreamError::Closed) => break,
                Err(StreamError::LastOperationFailed(error)) => {
                    panic!("{}", error.to_debug_string())
                }
            }
        }

        ensure_eq!(incoming_buffer, message.to_vec());

        Ok(())
    }
}
