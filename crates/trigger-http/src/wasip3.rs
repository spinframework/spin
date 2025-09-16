use crate::{server::HttpExecutor, TriggerInstanceBuilder};
use anyhow::{Context, Result};
use futures::{channel::oneshot, FutureExt};
use http_body_util::BodyExt;
use spin_factors::RuntimeFactors;
use spin_factors_executor::InstanceState;
use spin_http::routes::RouteMatch;
use std::net::SocketAddr;
use tokio::task;
use tracing::{instrument, Level};
use wasmtime_wasi_http::{
    body::HyperIncomingBody as Body,
    p3::{
        bindings::{http::types, ProxyIndices},
        WasiHttpCtxView,
    },
};

/// An [`HttpExecutor`] that uses the `wasi:http@0.3.*/handler` interface.
pub(super) struct Wasip3HttpExecutor<'a>(pub(super) &'a ProxyIndices);

impl HttpExecutor for Wasip3HttpExecutor<'_> {
    #[instrument(name = "spin_trigger_http.execute_wasm", skip_all, err(level = Level::INFO), fields(otel.name = format!("execute_wasm_component {}", route_match.lookup_key().to_string())))]
    async fn execute<F: RuntimeFactors>(
        &self,
        instance_builder: TriggerInstanceBuilder<'_, F>,
        route_match: &RouteMatch<'_, '_>,
        mut req: http::Request<Body>,
        client_addr: SocketAddr,
    ) -> Result<http::Response<Body>> {
        super::wasi::prepare_request(route_match, &mut req, client_addr)?;

        let (instance, mut store) = instance_builder.instantiate(()).await?;

        let getter = (|data| wasi_http::<F>(data).unwrap())
            as fn(&mut InstanceState<F::InstanceState, ()>) -> WasiHttpCtxView<'_>;

        let (request, body) = req.into_parts();
        let body = body.map_err(spin_factor_outbound_http::p2_to_p3_error_code);
        let request = http::Request::from_parts(request, body);
        let (request, request_io_result) = types::Request::from_http(request);
        let request = wasi_http::<F>(store.data_mut())?.table.push(request)?;

        let guest = self.0.load(&mut store, &instance)?;

        let (tx, rx) = oneshot::channel();
        task::spawn(
            async move {
                instance
                    .run_concurrent(&mut store, async move |store| {
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
                    })
                    .await?
            }
            .inspect(|result| {
                if let Err(error) = result {
                    tracing::error!("Component error handling request: {error:?}");
                }
            }),
        );

        Ok(rx.await?.map(|body| {
            body.map_err(spin_factor_outbound_http::p3_to_p2_error_code)
                .boxed()
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
