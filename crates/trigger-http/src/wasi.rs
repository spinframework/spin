use std::future;
use std::io::IsTerminal;
use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, anyhow};
use futures::TryFutureExt;
use http::{HeaderName, HeaderValue};
use http_body_util::BodyExt;
use hyper::{Request, Response};
use spin_core::Store;
use spin_factor_outbound_http::wasi_2023_10_18::Proxy as Proxy2023_10_18;
use spin_factor_outbound_http::wasi_2023_11_10::Proxy as Proxy2023_11_10;
use spin_factor_outbound_http::wasi_2026_03_15::Service as Service2026_03_15;
use spin_factors::{RuntimeFactors, RuntimeFactorsInstanceState};
use spin_factors_executor::InstanceState;
use spin_http::routes::RouteMatch;
use spin_http::trigger::HandlerType;
use tokio::{sync::oneshot, task};
use tracing::{Instrument, Level, instrument};
use wasmtime::AsContextMut;
use wasmtime_wasi_http::handler::HandlerState;
use wasmtime_wasi_http::p2::bindings::http::types::Scheme;
use wasmtime_wasi_http::p2::{bindings::Proxy, body::HyperIncomingBody as Body};
use wasmtime_wasi_http::p3;

use crate::{TriggerInstanceBuilder, headers::prepare_request_headers, server::set_request_deadline};

pub(super) fn prepare_request(
    route_match: &RouteMatch<'_, '_>,
    req: &mut Request<Body>,
    client_addr: SocketAddr,
) -> Result<()> {
    let spin_http::routes::TriggerLookupKey::Component(component_id) = route_match.lookup_key()
    else {
        unreachable!()
    };

    tracing::trace!("Executing request using the Wasi executor for component {component_id}");

    let headers = prepare_request_headers(req, route_match, client_addr)?;
    req.headers_mut().clear();
    req.headers_mut()
        .extend(headers.into_iter().filter_map(|(n, v)| {
            let Ok(name) = n.parse::<HeaderName>() else {
                return None;
            };
            let Ok(value) = HeaderValue::from_bytes(v.as_bytes()) else {
                return None;
            };
            Some((name, value))
        }));

    Ok(())
}

/// An [`HttpExecutor`] that uses the `wasi:http/incoming-handler` interface.
pub struct WasiHttpExecutor<'a, S: HandlerState> {
    pub handler_type: &'a HandlerType<S>,
}

impl<S: HandlerState> WasiHttpExecutor<'_, S> {
    #[instrument(name = "spin_trigger_http.execute_wasm", skip_all, err(level = Level::INFO), fields(otel.name = format!("execute_wasm_component {}", route_match.lookup_key().to_string())))]
    pub async fn execute<F: RuntimeFactors>(
        &self,
        instance_builder: TriggerInstanceBuilder<'_, F>,
        route_match: &RouteMatch<'_, '_>,
        mut req: Request<Body>,
        client_addr: SocketAddr,
        request_deadline: Option<Duration>,
    ) -> Result<Response<Body>> {
        prepare_request(route_match, &mut req, client_addr)?;

        let (instance, mut store) = instance_builder.instantiate(()).await?;
        set_request_deadline(&mut store, request_deadline);

        enum Handler {
            Latest(Proxy),
            Handler2023_11_10(Proxy2023_11_10),
            Handler2023_10_18(Proxy2023_10_18),
        }

        let handler = match self.handler_type {
            HandlerType::Wasi2023_10_18(indices) => {
                let guest = indices.load(&mut store, &instance)?;
                Handler::Handler2023_10_18(guest)
            }
            HandlerType::Wasi2023_11_10(indices) => {
                let guest = indices.load(&mut store, &instance)?;
                Handler::Handler2023_11_10(guest)
            }
            HandlerType::Wasi2026_03_15(indices) => {
                let guest = indices.load(&mut store, &instance)?;
                return handle_2026_03_15(store, guest, req).await;
            }
            HandlerType::Wasi0_2(indices) => Handler::Latest(indices.load(&mut store, &instance)?),
            HandlerType::Wasi0_3(_) => unreachable!("should have used Wasip3HttpExecutor"),
            HandlerType::Spin => unreachable!("should have used SpinHttpExecutor"),
            HandlerType::Wagi(_) => unreachable!("should have used WagiExecutor instead"),
        };

        let mut wasi_http = spin_factor_outbound_http::OutboundHttpFactor::get_wasi_http_impl(
            store.data_mut().factors_instance_state_mut(),
        )
        .context("missing OutboundHttpFactor")?;

        let request = wasi_http.new_incoming_request(Scheme::Http, req)?;

        let (response_tx, response_rx) = oneshot::channel();
        let response = wasi_http.new_response_outparam(response_tx)?;

        let handle = task::spawn(
            async move {
                let result = match handler {
                    Handler::Latest(handler) => {
                        handler
                            .wasi_http_incoming_handler()
                            .call_handle(&mut store, request, response)
                            .in_current_span()
                            .await
                    }
                    Handler::Handler2023_10_18(handler) => {
                        handler
                            .wasi_http0_2_0_rc_2023_10_18_incoming_handler()
                            .call_handle(&mut store, request, response)
                            .in_current_span()
                            .await
                    }
                    Handler::Handler2023_11_10(handler) => {
                        handler
                            .wasi_http0_2_0_rc_2023_11_10_incoming_handler()
                            .call_handle(&mut store, request, response)
                            .in_current_span()
                            .await
                    }
                };

                tracing::trace!(
                    "wasi-http memory consumed: {}",
                    store.data().core_state().memory_consumed()
                );

                result
            }
            .in_current_span(),
        );

        match response_rx.await {
            Ok(response) => {
                task::spawn(
                    async move {
                        handle
                            .await
                            .context("guest invocation panicked")?
                            .map_err(anyhow::Error::from)
                            .context("guest invocation failed")?;

                        Ok(())
                    }
                    .map_err(|e: anyhow::Error| {
                        if std::io::stderr().is_terminal() {
                            tracing::error!("Component error after response started. The response may not be fully sent: {e:?}");
                        } else {
                            terminal::warn!("Component error after response started: {e:?}");
                        }
                    }),
                );

                Ok(response.context("guest failed to produce a response")?)
            }

            Err(_) => {
                handle
                    .await
                    .context("guest invocation panicked")?
                    .map_err(anyhow::Error::from)
                    .context("guest invocation failed")?;

                Err(anyhow!(
                    "guest failed to produce a response prior to returning"
                ))
            }
        }
    }
}

async fn handle_2026_03_15<T: RuntimeFactorsInstanceState, U: Send>(
    mut store: Store<InstanceState<T, U>>,
    guest: Service2026_03_15,
    req: Request<Body>,
) -> Result<Response<Body>> {
    let (request, request_io_result) = p3::Request::from_http(req);
    let view: fn(&mut InstanceState<_, _>) -> p3::WasiHttpCtxView =
        |data: &mut InstanceState<_, _>| {
            spin_factor_outbound_http::OutboundHttpFactor::get_wasi_p3_http_impl(
                data.factors_instance_state_mut(),
            )
            .unwrap()
        };
    let request = view(store.data_mut()).table.push(request)?;

    let (tx, rx) = oneshot::channel();
    task::spawn(
        async move {
            store
                .as_context_mut()
                .run_concurrent(async |accessor| {
                    let response = guest
                        .wasi_http0_3_0_rc_2026_03_15_handler()
                        .call_handle(accessor, request)
                        .await??;

                    let response = accessor.with(|mut store| {
                        view(store.get())
                            .table
                            .delete(response)?
                            .into_http_with_getter(&mut store, request_io_result, view)
                    })?;

                    _ = tx.send(response);

                    future::poll_fn(|cx| accessor.poll_no_interesting_tasks(cx)).await;

                    Ok(())
                })
                .await?
        }
        .map_err(|e: anyhow::Error| {
            if std::io::stderr().is_terminal() {
                tracing::error!(
                    "Component error while handling request. \
                                 The response may not be fully sent: {e:?}"
                );
            } else {
                terminal::warn!("Component error while handling request: {e:?}");
            }
        })
        .in_current_span(),
    );

    Ok(rx
        .await?
        .map(|body| body.map_err(|e| e.into()).boxed_unsync()))
}
