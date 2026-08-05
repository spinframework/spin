use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_service;

use futures::{SinkExt, StreamExt};

/// A middleware component to (naively) convert all responses to upper case.
#[http_service]
async fn yell_it(request: Request) -> anyhow::Result<impl IntoResponse> {
    // Certain headers are forbidden, so we need to remove them
    let (mut parts, body) = request.into_parts();
    parts.headers.remove("connection");
    parts.headers.remove("host");
    let request = Request::from_parts(parts, body);

    // Pass the request on down the middleware pipeline.
    let response = spin_sdk::http::next(request).await?;

    // Crack open the response and transform the body
    let (parts, body) = response.into_parts();
    let mut body_stm = body.stream();

    let (mut tx, transformed_body) = spin_sdk::http::body::stream();

    spin_sdk::wasip3::spawn(async move {
        // Transform the body (presumed to be text) in a streaming manner.
        while let Some(chunk) = body_stm.next().await {
            let Ok(chunk) = chunk else {
                // can't read the response - bail out
                break;
            };
            let text = String::from_utf8_lossy(chunk.as_ref());
            let upper_text = text.to_uppercase();
            let bytes = bytes::Bytes::from_owner(upper_text.into_bytes());
            if tx.send(bytes).await.is_err() {
                // client has gone away - bail out
                break;
            }
        }
    });

    // Build a new response from the original parts and transformed body
    let response = Response::from_parts(parts, transformed_body);

    Ok(response)
}
