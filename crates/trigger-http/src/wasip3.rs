use crate::server::HttpHandlerState;
use anyhow::Result;
use http_body_util::BodyExt;
use spin_factors::RuntimeFactors;
use spin_http::routes::RouteMatch;
use std::net::SocketAddr;
use tracing::{Level, instrument};
use wasmtime_wasi_http::{
    handler::{ErrorCode, ProxyHandler},
    p2::{bindings::http::types as p2_types, body::HyperIncomingBody as Body},
    p3::bindings::http::types as p3_types,
};

/// An [`HttpExecutor`] that uses the `wasi:http@0.3.*/handler` interface.
pub(super) struct Wasip3HttpExecutor<'a, F: RuntimeFactors>(
    pub(super) &'a ProxyHandler<HttpHandlerState<F>>,
);

impl<F: RuntimeFactors> Wasip3HttpExecutor<'_, F> {
    #[instrument(name = "spin_trigger_http.execute_wasm", skip_all, err(level = Level::INFO), fields(otel.name = format!("execute_wasm_component {}", route_match.lookup_key().to_string())))]
    pub async fn execute(
        &self,
        route_match: &RouteMatch<'_, '_>,
        mut req: http::Request<Body>,
        client_addr: SocketAddr,
    ) -> Result<http::Response<Body>> {
        super::wasi::prepare_request(route_match, &mut req, client_addr)?;

        Ok(self
            .0
            .handle(
                (),
                req.map(|body| body.map_err(ErrorCode::from).boxed_unsync()),
            )
            .await?
            .map(|body| {
                body.map_err(|e| match e.downcast::<p3_types::ErrorCode>() {
                    Ok(e) => e.into(),
                    Err(e) => p2_types::ErrorCode::InternalError(Some(e.to_string())),
                })
                .boxed_unsync()
            }))
    }
}
