#![deny(warnings)]

wit_bindgen::generate!({
    path: "../../../../wit",
    world: "wasi:http/service@0.3.0-rc-2026-03-15",
    generate_all,
});

use crate::{
    exports::wasi::http0_3_0_rc_2026_03_15::handler::Guest,
    wasi::http0_3_0_rc_2026_03_15::{
        types::{ErrorCode, Fields, Request, Response},
    },
};

struct Component;

export!(Component);

impl Guest for Component {
    async fn handle(req: Request) -> Result<Response, ErrorCode> {
        let inbound_rel_path = req.get_path_with_query().unwrap();

        let (_body_fw, body_fr) = wit_future::new(|| Ok(()));
        let (mut body_sr, _fr) = Request::consume_body(req, body_fr);
        let buf = [0; 1024].to_vec();
        let (stm_res, read) = body_sr.read(buf).await;
        let read = match stm_res {
            wit_bindgen::StreamResult::Complete(len) => &read[..len],
            wit_bindgen::StreamResult::Dropped => &[],
            wit_bindgen::StreamResult::Cancelled => return Err(ErrorCode::InternalError(None)),
        };
        let inbound_body = String::from_utf8_lossy(read).to_string();

        let fields = Fields::from_list(&[
            ("spin-component".into(), "internal-http-back-component".into()),
            ("back-received-path".into(), inbound_rel_path.into()),
            ("back-received-body".into(), inbound_body.into()),
        ]).unwrap();
        let (mut resp_body_tx, resp_body_rx) = wit_stream::new();
        wit_bindgen::spawn(async move {
            resp_body_tx.write("Response body from back".into()).await;
        });
        let (_trailers_fw, trailers_fr) = wit_future::new(|| Ok(None));
        let (resp, _fr) = Response::new(fields, Some(resp_body_rx), trailers_fr);


        Ok(resp)
    }
}
