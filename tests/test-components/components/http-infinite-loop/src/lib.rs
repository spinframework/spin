use anyhow::Result;
use spin_sdk::http_component;

#[http_component]
fn loop_forever(_request: http::Request<()>) -> Result<http::Response<String>> {
    loop {
        std::hint::spin_loop();
    }
}
