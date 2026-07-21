use spin_sdk::http::{self, HeaderName, HeaderValue, Request, Response};
use spin_sdk::http_service;

#[http_service]
async fn handle(mut req: Request) -> http::Result<Response> {
    // Request runs on the way in, before the next handler.
    eprintln!("[middleware] --> {} {}", req.method(), req.uri().path());

    // Forward the (modified) request to the next handler in the chain and wait
    // for its response.
    let mut resp = http::next(req).await?;

    // Response runs on the way out, after the next handler
    eprintln!("[middleware] <-- {}", resp.status());

    Ok(resp)
}