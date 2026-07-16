#![deny(warnings)]

wit_bindgen::generate!({
    path: "../../../../wit",
    world: "wasi:http/middleware@0.3.0",
    generate_all,
});

use {
    crate::{
        exports::wasi::http0_3_0::handler::Guest,
        wasi::{
            http0_3_0::{
                types::{ErrorCode, Request, Response},
            },
        },
    },
};

struct Component;

export!(Component);

impl Guest for Component {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        let authority = request.get_authority();
        let scheme = request.get_scheme();
        let path_with_query = request.get_path_with_query();
        let headers = request.get_headers().clone();
        headers.append("Header-Added-By", b"middleware-header-adder").unwrap();

        let (_consume_body_fw, consume_body_fr) = wit_future::new(|| Ok(()));
        let (req_body, trailers_fr) = Request::consume_body(request, consume_body_fr);
        let (request, _efr) = Request::new(headers, Some(req_body), trailers_fr, None);
        request.set_authority(authority.as_deref()).unwrap();
        request.set_scheme(scheme.as_ref()).unwrap();
        request.set_path_with_query(path_with_query.as_deref()).unwrap();

        let resp = wasi::http0_3_0::handler::handle(request).await.unwrap();

        let status_code = resp.get_status_code();
        let headers = resp.get_headers().clone();
        headers.append("Response-Header-Added-By", b"yep me again!").unwrap();

        let (_consume_body_fw, consume_body_fr) = wit_future::new(|| Ok(()));
        let (resp_body, trailers_fr) = Response::consume_body(resp, consume_body_fr);

        let (resp, _efr) = Response::new(headers, Some(resp_body), trailers_fr);
        resp.set_status_code(status_code).unwrap();

        Ok(resp)
    }
}
