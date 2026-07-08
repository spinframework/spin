#![deny(warnings)]

wit_bindgen::generate!({
    path: "../../../../wit",
    inline: r#"package root:component;
    world w {
        include wasi:http/middleware@0.3.0;
        import spin:key-value/key-value@3.0.0;
    }"#,
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
};

struct Component;

export!(Component);

impl Guest for Component {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let Ok(store) = spin::key_value::key_value::Store::open("default".into()).await else {
            let (_fw, fr) = wit_future::new(|| Ok(None));
            let (resp, _fr) = Response::new(Fields::new(), None, fr);
            resp.set_status_code(500).unwrap();
            return Ok(resp);
        };

        let path = request.get_path_with_query().unwrap_or_default().into_bytes();
        store.set("last".into(), path.clone()).await.unwrap();

        let resp = wasi::http0_3_0::handler::handle(request).await.unwrap();

        let stored_path_bytes = store.get("last".into()).await.unwrap().unwrap();
        let stored_path = String::from_utf8_lossy(&stored_path_bytes).to_string();

        let status_code = resp.get_status_code();
        let headers = resp.get_headers();
        let (_fw, fr) = wit_future::new(|| Ok(()));
        let (mut resp_body, _fr) = Response::consume_body(resp, fr);

        let (mut body_wr, body) = wit_stream::new();
        let (_fw, fr) = wit_future::new(|| Ok(None));
        let (resp_munged, _fr) = Response::new(headers, Some(body), fr);
        resp_munged.set_status_code(status_code).unwrap();

        wit_bindgen::spawn_local(async move {
            body_wr.write_all(format!("Path: {stored_path}\n").into_bytes()).await;
            loop {
                let Some(chunk) = resp_body.next().await else {
                    break;
                };
                body_wr.write_one(chunk).await;
            }
        });

        Ok(resp_munged)
    }
}
