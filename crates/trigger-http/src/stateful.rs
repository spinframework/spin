use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use http_body_util::BodyExt;
use spin_factor_outbound_http::MutexBody;
use spin_factors::RuntimeFactors;
use spin_factors_executor::InstanceState;
use tokio::sync::{mpsc, oneshot, RwLock};
use wasmtime::component::Accessor;
use wasmtime_wasi_http::{
    body::HyperIncomingBody as Body,
    p3::{
        bindings::{http::types, ServiceIndices},
        WasiHttpCtxView,
    },
};

use crate::TriggerApp;

const LIFECYCLE_EXPORT: &str = "spin:stateful-component/lifecycle@0.1.0";

type StoreData<F> = InstanceState<<F as RuntimeFactors>::InstanceState, ()>;

struct HttpTask {
    request: http::Request<Body>,
    response_tx: oneshot::Sender<Result<http::Response<Body>>>,
}

/// Task for handling a single HTTP request, spawned concurrently within
/// a stateful instance's `run_concurrent` loop.
struct HandleRequestTask<F: RuntimeFactors> {
    service: Arc<wasmtime_wasi_http::p3::bindings::Service>,
    getter: fn(&mut StoreData<F>) -> WasiHttpCtxView<'_>,
    task: HttpTask,
}

impl<F: RuntimeFactors> wasmtime::component::AccessorTask<StoreData<F>>
    for HandleRequestTask<F>
{
    fn run(
        self,
        accessor: &Accessor<StoreData<F>>,
    ) -> impl std::future::Future<Output = wasmtime::Result<()>> + Send {
        async move {
            let result =
                handle_single_request::<F>(accessor, &self.service, self.getter, self.task.request)
                    .await;
            let _ = self.task.response_tx.send(result);
            Ok(())
        }
    }
}

/// Handle to a background worker managing a live stateful component instance.
struct StatefulWorker {
    task_tx: mpsc::UnboundedSender<HttpTask>,
    last_activity: Arc<std::sync::Mutex<Instant>>,
}

/// Manages long-lived stateful component instances keyed by (component_id, instance_id).
///
/// On first request for an ID, the manager:
/// 1. Spawns a background worker that creates a Store + Instance
/// 2. Calls the component's `lifecycle::instantiate(id)` export
/// 3. Enters `store.run_concurrent` to handle HTTP requests via a task channel
///
/// On idle timeout, the worker exits `run_concurrent`, calls `lifecycle::suspend()`,
/// and drops the instance.
pub struct StatefulInstanceManager<F: RuntimeFactors> {
    workers: RwLock<HashMap<(String, String), StatefulWorker>>,
    trigger_app: Arc<TriggerApp<F>>,
    idle_timeout: Duration,
}

impl<F: RuntimeFactors> StatefulInstanceManager<F> {
    pub fn new(trigger_app: Arc<TriggerApp<F>>, idle_timeout: Duration) -> Self {
        Self {
            workers: RwLock::new(HashMap::new()),
            trigger_app,
            idle_timeout,
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

        // Fast path: worker already exists
        {
            let guard = self.workers.read().await;
            if let Some(worker) = guard.get(&key) {
                *worker.last_activity.lock().unwrap() = Instant::now();
                return Self::send_request(worker, req).await;
            }
        }

        // Slow path: create worker
        let mut write_guard = self.workers.write().await;
        // Double-check after acquiring write lock
        if let Some(worker) = write_guard.get(&key) {
            *worker.last_activity.lock().unwrap() = Instant::now();
            return Self::send_request(worker, req).await;
        }

        let worker = self.spawn_worker(component_id, instance_id);
        let result = Self::send_request(&worker, req).await;
        write_guard.insert(key, worker);
        result
    }

    /// Send an HTTP request to a worker and await its response.
    async fn send_request(
        worker: &StatefulWorker,
        req: http::Request<Body>,
    ) -> Result<http::Response<Body>> {
        let (response_tx, response_rx) = oneshot::channel();
        worker
            .task_tx
            .send(HttpTask {
                request: req,
                response_tx,
            })
            .map_err(|_| anyhow::anyhow!("stateful instance worker has exited"))?;
        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("stateful instance worker dropped the request"))?
    }

    /// Spawn a background worker for a new stateful component instance.
    fn spawn_worker(&self, component_id: &str, instance_id: &str) -> StatefulWorker {
        let (task_tx, task_rx) = mpsc::unbounded_channel();
        let last_activity = Arc::new(std::sync::Mutex::new(Instant::now()));

        let trigger_app = Arc::clone(&self.trigger_app);
        let cid = component_id.to_string();
        let iid = instance_id.to_string();

        tokio::spawn(async move {
            if let Err(e) = run_stateful_worker::<F>(trigger_app, &cid, &iid, task_rx).await {
                tracing::error!(
                    component_id = cid,
                    instance_id = iid,
                    "stateful instance worker failed: {e:?}"
                );
            }
        });

        StatefulWorker {
            task_tx,
            last_activity,
        }
    }

    /// Start the background idle-checker that suspends timed-out instances.
    pub fn start_idle_checker(self: &Arc<Self>) {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;

                let expired: Vec<(String, String)> = {
                    let guard = manager.workers.read().await;
                    guard
                        .iter()
                        .filter(|(_, w)| {
                            w.last_activity.lock().unwrap().elapsed() > manager.idle_timeout
                        })
                        .map(|(key, _)| key.clone())
                        .collect()
                };

                if expired.is_empty() {
                    continue;
                }

                let mut write_guard = manager.workers.write().await;
                for key in expired {
                    tracing::info!(
                        component_id = key.0,
                        instance_id = key.1,
                        "Suspending idle stateful component instance"
                    );
                    // Dropping the worker closes the task channel, which causes
                    // the background worker to exit run_concurrent and call
                    // lifecycle::suspend before cleaning up.
                    write_guard.remove(&key);
                }
            }
        });
    }
}

/// Background worker that owns a Wasm instance and processes HTTP requests.
///
/// Lifecycle:
/// 1. Create store + component instance
/// 2. Call `lifecycle::instantiate(id)`
/// 3. Enter `store.run_concurrent` — loop receiving HTTP tasks from the channel
/// 4. When the channel closes (idle timeout), exit and call `lifecycle::suspend()`
async fn run_stateful_worker<F: RuntimeFactors>(
    trigger_app: Arc<TriggerApp<F>>,
    component_id: &str,
    instance_id: &str,
    mut task_rx: mpsc::UnboundedReceiver<HttpTask>,
) -> Result<()> {
    tracing::info!(component_id, instance_id, "Starting stateful component instance");

    // 1. Prepare store with all host factors
    let mut store: wasmtime::Store<StoreData<F>> = trigger_app
        .prepare(component_id)?
        .instantiate_store(())?
        .into_inner();

    // 2. Instantiate the Wasm component
    let pre = trigger_app.get_instance_pre(component_id)?;
    let instance = pre.instantiate_async(&mut store).await?;

    // 3. Look up lifecycle exports
    let lifecycle_idx = instance
        .get_export_index(&mut store, None, LIFECYCLE_EXPORT)
        .with_context(|| format!("component does not export {LIFECYCLE_EXPORT}"))?;
    let instantiate_idx = instance
        .get_export_index(&mut store, Some(&lifecycle_idx), "instantiate")
        .context("missing lifecycle::instantiate export")?;
    let suspend_idx = instance
        .get_export_index(&mut store, Some(&lifecycle_idx), "suspend")
        .context("missing lifecycle::suspend export")?;

    let instantiate_func =
        instance.get_typed_func::<(&str,), ()>(&mut store, &instantiate_idx)?;
    let suspend_func = instance.get_typed_func::<(), ()>(&mut store, &suspend_idx)?;

    // 4. Call lifecycle::instantiate(id)
    instantiate_func
        .call_async(&mut store, (instance_id,))
        .await
        .map_err(|e| anyhow::anyhow!("lifecycle::instantiate failed: {e}"))?;

    tracing::info!(component_id, instance_id, "Stateful instance activated");

    // 5. Prepare the HTTP Service from the same instance
    let service_indices =
        ServiceIndices::new(pre).map_err(|e| anyhow::anyhow!("missing wasi:http/handler: {e}"))?;
    let service = Arc::new(
        service_indices
            .load(&mut store, &instance)
            .map_err(|e| anyhow::anyhow!("failed to load HTTP service: {e}"))?,
    );

    // 6. Enter run_concurrent to handle HTTP requests concurrently.
    //    Each incoming request is spawned as a separate task so that multiple
    //    requests to the same instance can interleave at async yield points.
    let getter = (|data: &mut StoreData<F>| wasi_http::<F>(data).unwrap())
        as fn(&mut StoreData<F>) -> WasiHttpCtxView<'_>;

    let run_result = store
        .run_concurrent(async |accessor: &Accessor<StoreData<F>>| {
            while let Some(task) = task_rx.recv().await {
                accessor.spawn(HandleRequestTask::<F> {
                    service: Arc::clone(&service),
                    getter,
                    task,
                });
            }
            anyhow::Ok(())
        })
        .await;

    if let Err(e) = &run_result {
        tracing::error!(component_id, instance_id, "run_concurrent failed: {e:?}");
    }

    // 7. run_concurrent has returned (channel closed) — call lifecycle::suspend
    tracing::info!(component_id, instance_id, "Suspending stateful instance");
    if let Err(e) = suspend_func.call_async(&mut store, ()).await {
        tracing::error!(
            component_id,
            instance_id,
            "lifecycle::suspend failed: {e:?}"
        );
    }

    Ok(())
}

/// Dispatch a single HTTP request to the component's handler within run_concurrent.
async fn handle_single_request<F: RuntimeFactors>(
    accessor: &Accessor<StoreData<F>>,
    service: &wasmtime_wasi_http::p3::bindings::Service,
    getter: fn(&mut StoreData<F>) -> WasiHttpCtxView<'_>,
    req: http::Request<Body>,
) -> Result<http::Response<Body>> {
    let (parts, body) = req.into_parts();
    let body = body.map_err(spin_factor_outbound_http::p2_to_p3_error_code);
    let request = http::Request::from_parts(parts, body);
    let (request, request_io_result) = types::Request::from_http(request);

    // Push request resource into the table
    let request_handle = accessor.with(|mut store| {
        anyhow::Ok(wasi_http::<F>(store.data_mut())?.table.push(request)?)
    })?;

    // Call the component's HTTP handler (async WIT export)
    let (response_result, task) = service
        .wasi_http_handler()
        .call_handle(accessor, request_handle)
        .await?;

    // Extract response from resource table
    let response = accessor.with(|mut store| {
        anyhow::Ok(wasi_http::<F>(store.data_mut())?.table.delete(response_result?)?)
    })?;

    // Convert to http::Response
    let response = accessor.with(|mut store| {
        response.into_http_with_getter(&mut store, request_io_result, getter)
    })?;

    // Wait for any async streaming work to complete
    task.block(accessor).await;

    Ok(response.map(|body| {
        MutexBody::new(body.map_err(spin_factor_outbound_http::p3_to_p2_error_code))
            .boxed_unsync()
    }))
}

fn wasi_http<F: RuntimeFactors>(data: &mut StoreData<F>) -> Result<WasiHttpCtxView<'_>> {
    spin_factor_outbound_http::OutboundHttpFactor::get_wasi_p3_http_impl(
        data.factors_instance_state_mut(),
    )
    .context("missing OutboundHttpFactor")
}
