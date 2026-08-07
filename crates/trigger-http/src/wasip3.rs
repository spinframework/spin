use crate::{HttpServer, server::HttpHandlerState};
use anyhow::Result;
use http_body_util::BodyExt;
use spin_factors::RuntimeFactors;
use spin_http::routes::RouteMatch;
use std::{net::SocketAddr, sync::Arc};
use tracing::{Level, instrument};
use wasmtime_wasi_http::{
    Error as WasiHttpError, handler::ProxyHandler, p2::body::HyperIncomingBody as Body,
};

/// An [`HttpExecutor`] that uses the `wasi:http@0.3.*/handler` interface.
pub(super) struct Wasip3HttpExecutor<'a, F: RuntimeFactors>(
    pub(super) &'a ProxyHandler<HttpHandlerState<F>>,
);

impl<F: RuntimeFactors> Wasip3HttpExecutor<'_, F> {
    #[instrument(name = "spin_trigger_http.execute_wasm", skip_all, err(level = Level::INFO), fields(otel.name = format!("execute_wasm_component {}", route_match.lookup_key().to_string())))]
    pub async fn execute(
        &self,
        server: &Arc<HttpServer<F>>,
        route_match: &RouteMatch<'_, '_>,
        mut req: http::Request<Body>,
        client_addr: SocketAddr,
    ) -> Result<http::Response<Body>> {
        self.0.state().init_once(server, req.uri());
        super::wasi::prepare_request(route_match, &mut req, client_addr)?;

        Ok(self
            .0
            .handle(
                (),
                req.map(|body| body.map_err(WasiHttpError::from).boxed_unsync()),
            )
            .await?)
    }
}
