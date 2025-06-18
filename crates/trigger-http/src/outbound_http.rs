use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

use http::uri::Scheme;
use spin_core::async_trait;
use spin_factor_outbound_http::intercept::{self, InterceptOutcome, InterceptRequest};
use spin_factor_outbound_networking::config::allowed_hosts::parse_service_chaining_target;
use spin_factors::RuntimeFactors;
use spin_http::routes::RouteMatch;
use wasmtime_wasi_http::{HttpError, HttpResult};

use crate::HttpServer;

/// An outbound HTTP interceptor that handles service chaining requests.
pub struct OutboundHttpInterceptor<F: RuntimeFactors> {
    server: Arc<HttpServer<F>>,
}

impl<F: RuntimeFactors> OutboundHttpInterceptor<F> {
    pub fn new(server: Arc<HttpServer<F>>) -> Self {
        Self { server }
    }
}

const CHAINED_CLIENT_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0);

#[async_trait]
impl<F: RuntimeFactors> intercept::OutboundHttpInterceptor for OutboundHttpInterceptor<F> {
    async fn intercept(&self, request: InterceptRequest) -> HttpResult<InterceptOutcome> {
        // Handle service chaining requests
        if let Some(component_id) = parse_service_chaining_target(request.uri()) {
            let req = request.into_hyper_request();
            let path = req.uri().path().to_owned();
            let route_match = RouteMatch::synthetic(component_id, path);
            let resp = self
                .server
                .handle_trigger_route(req, route_match, Scheme::HTTP, CHAINED_CLIENT_ADDR)
                .await
                .map_err(HttpError::trap)?;
            Ok(InterceptOutcome::Complete(resp))
        } else {
            Ok(InterceptOutcome::Continue(request))
        }
    }
}
