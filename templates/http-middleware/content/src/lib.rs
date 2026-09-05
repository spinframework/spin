use spin_sdk::http::{self, Request, Response};
use spin_sdk::http_service;

/// Headers that WASI HTTP manages itself and forbids setting on an outgoing
/// request. They must be stripped from the incoming request before it is
/// forwarded, otherwise the runtime rejects it with `HeaderError::Forbidden`.
const FORBIDDEN_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

#[http_service]
async fn handle(mut req: Request) -> http::Result<Response> {
    // Request runs on the way in, before the next handler.
    eprintln!("[middleware] --> {} {}", req.method(), req.uri().path());

    // Strip runtime-managed headers so forwarding doesn't hit `Forbidden`.
    let headers = req.headers_mut();
    for name in FORBIDDEN_HEADERS {
        headers.remove(*name);
    }

    // MIDDLEWARE LOGIC GOES HERE
    
    // Forward the (modified) request to the next handler in the chain and wait
    // for its response.
    let resp = http::next(req).await?;

    // Response runs on the way out, after the next handler
    eprintln!("[middleware] <-- {}", resp.status());

    Ok(resp)
}