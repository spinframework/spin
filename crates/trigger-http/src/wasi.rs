use std::io::IsTerminal;
use std::net::SocketAddr;

use anyhow::{anyhow, Context, Result};
use futures::TryFutureExt;
use http::{HeaderName, HeaderValue};
use hyper::{Request, Response};
use spin_factor_outbound_http::wasi_2023_10_18::Proxy as Proxy2023_10_18;
use spin_factor_outbound_http::wasi_2023_11_10::Proxy as Proxy2023_11_10;
use spin_factors::RuntimeFactors;
use spin_http::routes::RouteMatch;
use spin_http::trigger::HandlerType;
use tokio::{sync::oneshot, task};
use tracing::{instrument, Instrument, Level};
use wasmtime_wasi_http::bindings::http::types::Scheme;
use wasmtime_wasi_http::{bindings::Proxy, body::HyperIncomingBody as Body, WasiHttpView};

use crate::{headers::prepare_request_headers, server::HttpExecutor, TriggerInstanceBuilder};

/// An [`HttpExecutor`] that uses the `wasi:http/incoming-handler` interface.
pub struct WasiHttpExecutor<'a> {
    pub handler_type: &'a HandlerType,
}

impl HttpExecutor for WasiHttpExecutor<'_> {
    #[instrument(name = "spin_trigger_http.execute_wasm", skip_all, err(level = Level::INFO), fields(otel.name = format!("execute_wasm_component {}", route_match.component_id())))]
    async fn execute<F: RuntimeFactors>(
        &self,
        instance_builder: TriggerInstanceBuilder<'_, F>,
        route_match: &RouteMatch<'_, '_>,
        mut req: Request<Body>,
        client_addr: SocketAddr,
    ) -> Result<Response<Body>> {
        let component_id = route_match.component_id();

        tracing::trace!("Executing request using the Wasi executor for component {component_id}");

        let (instance, mut store) = instance_builder.instantiate(()).await?;

        let headers = prepare_request_headers(&req, route_match, client_addr)?;
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

        let mut wasi_http = spin_factor_outbound_http::OutboundHttpFactor::get_wasi_http_impl(
            store.data_mut().factors_instance_state_mut(),
        )
        .context("missing OutboundHttpFactor")?;

        let (parts, body) = req.into_parts();
        let body = wasmtime_wasi_http::body::HostIncomingBody::new(
            body,
            std::time::Duration::from_secs(600),
        );
        let request = wasmtime_wasi_http::types::HostIncomingRequest::new(
            &mut wasi_http,
            parts,
            Scheme::Http,
            Some(body),
        )?;
        let request = wasi_http.table().push(request)?;

        let (response_tx, response_rx) = oneshot::channel();
        let response = wasi_http.new_response_outparam(response_tx)?;

        drop(wasi_http);

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
            HandlerType::Wasi0_2(indices) => Handler::Latest(indices.load(&mut store, &instance)?),
            HandlerType::Spin => unreachable!("should have used SpinHttpExecutor"),
            HandlerType::Wagi(_) => unreachable!("should have used WagiExecutor instead"),
        };

        let span = tracing::debug_span!("execute_wasi");
        let handle = task::spawn(
            async move {
                let result = match handler {
                    Handler::Latest(handler) => {
                        handler
                            .wasi_http_incoming_handler()
                            .call_handle(&mut store, request, response)
                            .instrument(span)
                            .await
                    }
                    Handler::Handler2023_10_18(handler) => {
                        handler
                            .wasi_http0_2_0_rc_2023_10_18_incoming_handler()
                            .call_handle(&mut store, request, response)
                            .instrument(span)
                            .await
                    }
                    Handler::Handler2023_11_10(handler) => {
                        handler
                            .wasi_http0_2_0_rc_2023_11_10_incoming_handler()
                            .call_handle(&mut store, request, response)
                            .instrument(span)
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
                    .context("guest invocation failed")?;

                Err(anyhow!(
                    "guest failed to produce a response prior to returning"
                ))
            }
        }
    }
}
