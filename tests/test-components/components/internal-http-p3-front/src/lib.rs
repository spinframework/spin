#![deny(warnings)]

wit_bindgen::generate!({
    path: "../../../../wit",
    world: "wasi:http/service@0.3.0-rc-2026-03-15",
    generate_all,
});

use crate::{
    exports::wasi::http0_3_0_rc_2026_03_15::handler::Guest,
    wasi::http0_3_0_rc_2026_03_15::{
        client,
        types::{ErrorCode, Fields, Method, Request, Response, Scheme},
    },
};
use wit_bindgen::rt::async_support;

struct Component;

export!(Component);

impl Guest for Component {
    async fn handle(_request: Request) -> Result<Response, ErrorCode> {
        let request = Request::new(Fields::new(), None, wit_future::new(|| Ok(None)).1, None).0;
        request.set_method(&Method::Get).unwrap();
        request.set_scheme(Some(&Scheme::Http)).unwrap();
        request.set_authority(Some("one.spin.internal")).unwrap();
        request.set_path_with_query(Some("/hello/from/p3")).unwrap();

        let response = client::send(request).await?;
        let status = response.get_status_code();
        if status != 200 {
            return Ok(text_response(
                500,
                format!("expected 200 from chained component, got {status}"),
            ));
        }

        let headers = response.get_headers().copy_all();
        let Some((_, component)) = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("spin-component"))
        else {
            return Ok(text_response(
                500,
                "missing spin-component header from chained component",
            ));
        };
        let component = String::from_utf8_lossy(component);
        if !component.contains("internal-http-back-component") {
            return Ok(text_response(
                500,
                format!("unexpected spin-component header: {component}"),
            ));
        }

        Ok(text_response(200, "ok"))
    }
}

fn text_response(status: u16, body: impl Into<Vec<u8>>) -> Response {
    let (mut tx, rx) = wit_stream::new();
    let body = body.into();
    async_support::spawn(async move {
        tx.write_all(body).await;
    });
    let response = Response::new(
        Fields::from_list(&[("content-type".to_string(), b"text/plain".to_vec())]).unwrap(),
        Some(rx),
        wit_future::new(|| Ok(None)).1,
    )
    .0;
    response.set_status_code(status).unwrap();
    response
}
