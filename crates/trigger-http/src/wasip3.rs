use crate::server::HttpHandlerState;
use anyhow::{Context as _, Result};
use futures::{channel::oneshot, FutureExt};
use http_body_util::BodyExt;
use spin_factor_outbound_http::MutexBody;
use spin_factors::RuntimeFactors;
use spin_factors_executor::InstanceState;
use spin_http::routes::RouteMatch;
use std::net::SocketAddr;
use tracing::{instrument, Instrument, Level};
use wasmtime::component::Accessor;
use wasmtime_wasi_http::{
    body::HyperIncomingBody as Body,
    handler::{Proxy, ProxyHandler},
    p3::{bindings::http::types, WasiHttpCtxView},
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

        let getter = (|data| wasi_http::<F>(data).unwrap())
            as fn(&mut InstanceState<F::InstanceState, ()>) -> WasiHttpCtxView<'_>;

        let (request, body) = req.into_parts();
        let body = body.map_err(spin_factor_outbound_http::p2_to_p3_error_code);
        let request = http::Request::from_parts(request, body);
        let (request, request_io_result) = types::Request::from_http(request);

        let (tx, rx) = oneshot::channel();
        self.0.spawn(
            None,
            Box::new(move |store: &Accessor<_>, guest: &Proxy| {
                Box::pin(
                    async move {
                        let Proxy::P3(guest) = guest else {
                            unreachable!();
                        };

                        let request = store.with(|mut store| {
                            anyhow::Ok(wasi_http::<F>(store.data_mut())?.table.push(request)?)
                        })?;

                        let (response, task) = guest
                            .wasi_http_handler()
                            .call_handle(store, request)
                            .await?;
                        let response = store.with(|mut store| {
                            anyhow::Ok(wasi_http::<F>(store.get())?.table.delete(response?)?)
                        })?;
                        let response = store.with(|mut store| {
                            response.into_http_with_getter(&mut store, request_io_result, getter)
                        })?;

                        _ = tx.send(response);

                        task.block(store).await;

                        anyhow::Ok(())
                    }
                    .in_current_span()
                    .map(|result| {
                        if let Err(error) = result {
                            tracing::error!("Component error handling request: {error:?}");
                        }
                    }),
                )
            }),
        );

        Ok(rx.await?.map(|body| {
            MutexBody::new(body.map_err(spin_factor_outbound_http::p3_to_p2_error_code)).boxed()
        }))
    }
}

fn wasi_http<F: RuntimeFactors>(
    data: &mut InstanceState<F::InstanceState, ()>,
) -> Result<WasiHttpCtxView<'_>> {
    spin_factor_outbound_http::OutboundHttpFactor::get_wasi_p3_http_impl(
        data.factors_instance_state_mut(),
    )
    .context("missing OutboundHttpFactor")
}
