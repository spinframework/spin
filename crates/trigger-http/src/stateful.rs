use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use futures::{channel::oneshot, FutureExt};
use http_body_util::BodyExt;
use spin_factor_outbound_http::MutexBody;
use spin_factors::RuntimeFactors;
use spin_factors_executor::InstanceState;
use tokio::sync::RwLock;
use tracing::Instrument;
use wasmtime::component::Accessor;
use wasmtime_wasi_http::{
    body::HyperIncomingBody as Body,
    handler::{HandlerState, Proxy, ProxyHandler, StoreBundle},
    p3::{bindings::http::types, WasiHttpCtxView},
};

use crate::TriggerApp;

const DEFAULT_STATEFUL_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// A live stateful component instance, keyed by (component_id, instance_id).
struct StatefulInstance<F: RuntimeFactors> {
    handler: ProxyHandler<StatefulHandlerState<F>>,
    last_activity: Instant,
}

/// Handler state for stateful component instances.
/// Each stateful instance gets a long-lived ProxyHandler that persists across requests.
struct StatefulHandlerState<F: RuntimeFactors> {
    trigger_app: Arc<TriggerApp<F>>,
    component_id: String,
}

impl<F: RuntimeFactors> HandlerState for StatefulHandlerState<F> {
    type StoreData = InstanceState<F::InstanceState, ()>;

    fn new_store(&self, _req_id: Option<u64>) -> wasmtime::Result<StoreBundle<Self::StoreData>> {
        use wasmtime::ToWasmtimeResult;
        Ok(StoreBundle {
            store: self
                .trigger_app
                .prepare(&self.component_id)
                .to_wasmtime_result()?
                .instantiate_store(())
                .to_wasmtime_result()?
                .into_inner(),
            write_profile: Box::new(|_| ()),
        })
    }

    fn request_timeout(&self) -> Duration {
        Duration::MAX
    }

    fn idle_instance_timeout(&self) -> Duration {
        // Stateful instances are long-lived; the idle check is handled
        // by StatefulInstanceManager, not by the ProxyHandler.
        Duration::MAX
    }

    fn max_instance_reuse_count(&self) -> usize {
        usize::MAX
    }

    fn max_instance_concurrent_reuse_count(&self) -> usize {
        64
    }

    fn handle_worker_error(&self, error: wasmtime::Error) {
        tracing::warn!("stateful instance worker error: {error:?}")
    }
}

/// Manages long-lived stateful component instances keyed by (component_id, instance_id).
///
/// When a request arrives for a stateful component instance that doesn't exist yet,
/// the manager creates a new WASIp3 ProxyHandler for it. Subsequent requests to the
/// same (component_id, instance_id) are dispatched to the same handler.
///
/// After an idle timeout with no requests, the instance is dropped.
pub struct StatefulInstanceManager<F: RuntimeFactors> {
    instances: RwLock<HashMap<(String, String), StatefulInstance<F>>>,
    trigger_app: Arc<TriggerApp<F>>,
    idle_timeout: Duration,
}

impl<F: RuntimeFactors> StatefulInstanceManager<F> {
    /// Create a new manager backed by the given trigger app.
    pub fn new(trigger_app: Arc<TriggerApp<F>>) -> Self {
        Self {
            instances: RwLock::new(HashMap::new()),
            trigger_app,
            idle_timeout: DEFAULT_STATEFUL_IDLE_TIMEOUT,
        }
    }

    /// Handle an HTTP request to a stateful component instance.
    pub async fn handle_request(
        &self,
        req: http::Request<Body>,
        component_id: &str,
        instance_id: &str,
    ) -> Result<http::Response<Body>> {
        let key = (component_id.to_string(), instance_id.to_string());

        // Fast path: instance already exists
        {
            let mut read_guard = self.instances.write().await;
            if let Some(instance) = read_guard.get_mut(&key) {
                instance.last_activity = Instant::now();
                return Self::dispatch_to_handler(&instance.handler, req).await;
            }
        }

        // Slow path: create instance
        let mut write_guard = self.instances.write().await;

        // Double-check after acquiring write lock
        if let Some(instance) = write_guard.get_mut(&key) {
            instance.last_activity = Instant::now();
            return Self::dispatch_to_handler(&instance.handler, req).await;
        }

        tracing::info!(
            component_id,
            instance_id,
            "Creating new stateful component instance"
        );

        let pre = self.trigger_app.get_instance_pre(component_id)?;
        let proxy_pre = wasmtime_wasi_http::p3::bindings::ServicePre::new(pre.clone())
            .context("stateful components must export wasi:http/handler@0.3")?;

        let handler_state = StatefulHandlerState {
            trigger_app: self.trigger_app.clone(),
            component_id: component_id.to_string(),
        };

        let handler = ProxyHandler::new(
            handler_state,
            wasmtime_wasi_http::handler::ProxyPre::P3(proxy_pre),
        );

        // TODO: Call lifecycle::instantiate(instance_id) on the component instance.
        // This requires wasmtime bindings generated for the stateful-component world
        // that include both wasi:http/handler and spin:stateful-component/lifecycle.
        // For now, the SDK-side #[stateful_component] macro handles lifecycle via
        // the generated WIT exports, and a future iteration will add host-side calls.

        let result = Self::dispatch_to_handler(&handler, req).await;

        write_guard.insert(
            key,
            StatefulInstance {
                handler,
                last_activity: Instant::now(),
            },
        );

        result
    }

    async fn dispatch_to_handler(
        handler: &ProxyHandler<StatefulHandlerState<F>>,
        req: http::Request<Body>,
    ) -> Result<http::Response<Body>> {
        let getter = (|data: &mut InstanceState<F::InstanceState, ()>| {
            spin_factor_outbound_http::OutboundHttpFactor::get_wasi_p3_http_impl(
                data.factors_instance_state_mut(),
            )
            .unwrap()
        })
            as fn(&mut InstanceState<F::InstanceState, ()>) -> WasiHttpCtxView<'_>;

        let (request, body) = req.into_parts();
        let body = body.map_err(spin_factor_outbound_http::p2_to_p3_error_code);
        let request = http::Request::from_parts(request, body);
        let (request, request_io_result) = types::Request::from_http(request);

        let (tx, rx) = oneshot::channel();
        handler.spawn(
            None,
            Box::new(move |store: &Accessor<_>, guest: &Proxy| {
                Box::pin(
                    async move {
                        let Proxy::P3(guest) = guest else {
                            unreachable!();
                        };

                        let request = store.with(|mut store| {
                            let view =
                                spin_factor_outbound_http::OutboundHttpFactor::get_wasi_p3_http_impl(
                                    store.data_mut().factors_instance_state_mut(),
                                )?;
                            anyhow::Ok(view.table.push(request)?)
                        })?;

                        let (response, task) = guest
                            .wasi_http_handler()
                            .call_handle(store, request)
                            .await?;
                        let response = store.with(|mut store| {
                            let view =
                                spin_factor_outbound_http::OutboundHttpFactor::get_wasi_p3_http_impl(
                                    store.get().factors_instance_state_mut(),
                                )?;
                            anyhow::Ok(view.table.delete(response?)?)
                        })?;
                        let response = store.with(|mut store| {
                            response.into_http_with_getter(
                                &mut store,
                                request_io_result,
                                getter,
                            )
                        })?;

                        _ = tx.send(response);

                        task.block(store).await;

                        anyhow::Ok(())
                    }
                    .in_current_span()
                    .map(|result| {
                        if let Err(error) = result {
                            tracing::error!(
                                "Stateful component error handling request: {error:?}"
                            );
                        }
                    }),
                )
            }),
        );

        Ok(rx.await?.map(|body| {
            MutexBody::new(body.map_err(spin_factor_outbound_http::p3_to_p2_error_code))
                .boxed_unsync()
        }))
    }

    /// Start a background task that periodically checks for idle instances
    /// and removes them.
    pub fn start_idle_checker(self: &Arc<Self>) {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let expired: Vec<(String, String)> = {
                    let read_guard = manager.instances.read().await;
                    read_guard
                        .iter()
                        .filter(|(_, inst)| inst.last_activity.elapsed() > manager.idle_timeout)
                        .map(|(key, _)| key.clone())
                        .collect()
                };

                for (component_id, instance_id) in expired {
                    tracing::info!(
                        component_id,
                        instance_id,
                        "Suspending idle stateful component instance"
                    );

                    // TODO: Call lifecycle::suspend() on the component instance
                    // before dropping it. See TODO in handle_request above.

                    let mut write_guard = manager.instances.write().await;
                    write_guard.remove(&(component_id, instance_id));
                }
            }
        });
    }
}
