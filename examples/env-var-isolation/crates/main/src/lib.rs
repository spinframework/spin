use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_component;

use crate::hello::components::dependable;

/// A simple Spin HTTP component that demonstrates per-dependency env isolation.
#[http_component]
fn handle_hello_rust(_req: Request) -> anyhow::Result<impl IntoResponse> {
    let body = format!("{}\n{}\n", get_message(), dependable::get_message());
    Ok(Response::new(200, body))
}

fn get_message() -> String {
    format!(
        "main's env vars: {}",
        std::env::vars()
            .map(|(key, value)| format!("{key}='{value}'"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

spin_sdk::wit_bindgen::generate!({
    world: "main",
    path: "../../wit",
    runtime_path: "::spin_sdk::wit_bindgen::rt",
});
