use spin_sdk::http::{IntoResponse, Request, Response};
use spin_sdk::http_component;

mod wit {
    wit_bindgen::generate!({});
}

#[http_component]
fn calls_unsupported_import(_req: Request) -> anyhow::Result<impl IntoResponse> {
    wit::nonspin::crimes::unsupported_in_spin::spin_cant_do_this();
    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Crime done")
        .build())
}
