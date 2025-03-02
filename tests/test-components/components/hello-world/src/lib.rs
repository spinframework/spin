use spin_sdk::http_component;

/// A simple Spin HTTP component.
#[http_component]
fn hello_world(req: http::Request<()>) -> anyhow::Result<http::Response<&'static str>> {
    println!("{:?}", req.headers());
    Ok(http::Response::builder()
        .status(200)
        .header("Content-Type", "text/plain")
        .body("Hello World!\n")?)
}
