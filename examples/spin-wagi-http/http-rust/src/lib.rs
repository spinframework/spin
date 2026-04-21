use anyhow::Result;
use spin_sdk::http::{IntoResponse, Request};
use spin_sdk::http_service;

/// A simple Spin HTTP component.
#[http_service]
async fn goodbye_world(req: Request) -> Result<impl IntoResponse> {
    println!("{:?}", req.headers());
    Ok(http::Response::builder()
        .status(200)
        .header("foo", "bar")
        .body("Goodbye, World!\n".to_string())?)
}
