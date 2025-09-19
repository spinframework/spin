use anyhow::{anyhow, Result};
use helper::ensure_some;
use spin_sdk::{
    http::{Request, Response},
    http_component,
};

#[http_component]
fn test_routing_headers(req: Request) -> Result<Response> {
    test_routing_headers_impl(req).map_err(|e| anyhow!("{e}"))
}

fn test_routing_headers_impl(req: Request) -> Result<Response, String> {
    let header_userid = req
        .header("spin-path-match-userid")
        .and_then(|v| v.as_str());
    let header_userid = ensure_some!(header_userid);

    let trailing = req.header("spin-path-info").and_then(|v| v.as_str());
    let trailing = ensure_some!(trailing);

    let response = format!("{header_userid}:{trailing}");

    Ok(Response::builder().status(200).body(response).build())
}
