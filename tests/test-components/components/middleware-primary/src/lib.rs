#![deny(warnings)]

wit_bindgen::generate!({
    path: "../../../../wit",
    world: "wasi:http/service@0.3.0",
    generate_all,
});

use {
    crate::{
        exports::wasi::http0_3_0::handler::Guest,
        wasi::{
            http0_3_0::{
                types::{ErrorCode, Fields, Request, Response},
            },
        },
    },
    wit_bindgen::{rt::async_support},
};

struct Component;

export!(Component);

impl Guest for Component {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let mut headers = request.get_headers().copy_all();
        for (header_name, _) in &mut headers {
            *header_name = format!("Request-{header_name}");
        }
        let echoed_headers= Fields::from_list(&headers).unwrap();

        let (mut sw, sr) = wit_stream::new();
        let (_tfw, tfr) = wit_future::new(|| Ok(None));
        let (resp, _efr) = Response::new(echoed_headers, Some(sr), tfr);

        let (_consume_body_fw, consume_body_fr) = wit_future::new(|| Ok(()));

        let (mut req_body, _trailers_fr) = Request::consume_body(request, consume_body_fr);

        async_support::spawn_local(async move {
            sw.write_all("Request body:\n\n".into()).await;
            loop {
                let Some(chunk) = req_body.next().await else {
                    break;
                };
                sw.write_one(chunk).await;
            }
        });

        Ok(resp)
    }
}
