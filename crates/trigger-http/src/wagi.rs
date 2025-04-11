use std::{io::Cursor, net::SocketAddr};

use anyhow::{ensure, Context, Result};
use http_body_util::BodyExt;
use hyper::{Request, Response};
use spin_factor_wasi::WasiFactor;
use spin_factors::RuntimeFactors;
use spin_http::{config::WagiTriggerConfig, routes::RouteMatch, wagi};
use tracing::{instrument, Level};
use wasmtime_wasi::bindings::CommandIndices;
use wasmtime_wasi::pipe::MemoryOutputPipe;
use wasmtime_wasi_http::body::HyperIncomingBody as Body;

use crate::{headers::compute_default_headers, server::HttpExecutor, TriggerInstanceBuilder};

pub struct WagiHttpExecutor<'a> {
    pub wagi_config: &'a WagiTriggerConfig,
    pub indices: &'a CommandIndices,
}

impl HttpExecutor for WagiHttpExecutor<'_> {
    #[instrument(name = "spin_trigger_http.execute_wagi", skip_all, err(level = Level::INFO), fields(otel.name = format!("execute_wagi_component {}", route_match.component_id())))]
    async fn execute<F: RuntimeFactors>(
        &self,
        mut instance_builder: TriggerInstanceBuilder<'_, F>,
        route_match: &RouteMatch<'_, '_>,
        req: Request<Body>,
        client_addr: SocketAddr,
    ) -> Result<Response<Body>> {
        let component = route_match.component_id();

        tracing::trace!(
            "Executing request using the Wagi executor for component {}",
            component
        );

        let uri_path = req.uri().path();

        // Build the argv array by starting with the config for `argv` and substituting in
        // script name and args where appropriate.
        let script_name = uri_path.to_string();
        let args = req.uri().query().unwrap_or_default().replace('&', " ");
        let argv = self
            .wagi_config
            .argv
            .clone()
            .replace("${SCRIPT_NAME}", &script_name)
            .replace("${ARGS}", &args);

        let (parts, body) = req.into_parts();

        let body = body.collect().await?.to_bytes().to_vec();
        let len = body.len();

        // TODO
        // The default host and TLS fields are currently hard-coded.
        let mut headers =
            wagi::build_headers(route_match, &parts, len, client_addr, "default_host", false);

        let default_host = http::HeaderValue::from_str("localhost")?;
        let host = std::str::from_utf8(
            parts
                .headers
                .get("host")
                .unwrap_or(&default_host)
                .as_bytes(),
        )?;

        // Add the default Spin headers.
        // This sets the current environment variables Wagi expects (such as
        // `PATH_INFO`, or `X_FULL_URL`).
        // Note that this overrides any existing headers previously set by Wagi.
        for (keys, val) in compute_default_headers(&parts.uri, host, route_match, client_addr)? {
            headers.insert(keys[1].to_string(), val);
        }

        let stdout = MemoryOutputPipe::new(usize::MAX);

        let wasi_builder = instance_builder
            .factor_builder::<WasiFactor>()
            .context("The wagi HTTP trigger was configured without the required wasi support")?;

        // Set up Wagi environment
        wasi_builder.args(argv.split(' '));
        wasi_builder.env(headers);
        wasi_builder.stdin_pipe(Cursor::new(body));
        wasi_builder.stdout(stdout.clone());

        let (instance, mut store) = instance_builder.instantiate(()).await?;

        let command = self.indices.load(&mut store, &instance)?;

        tracing::trace!("Calling Wasm entry point");
        if let Err(()) = command
            .wasi_cli_run()
            .call_run(&mut store)
            .await
            .or_else(ignore_successful_proc_exit_trap)?
        {
            tracing::error!("Wagi main function returned unsuccessful result");
        }
        tracing::info!("Wagi execution complete");

        // Drop the store so we're left with a unique reference to `stdout`:
        drop(store);

        let stdout = stdout.try_into_inner().unwrap();
        ensure!(
            !stdout.is_empty(),
            "The {component:?} component is configured to use the WAGI executor \
             but did not write to stdout. Check the `executor` in spin.toml."
        );

        wagi::compose_response(&stdout)
    }
}

fn ignore_successful_proc_exit_trap(guest_err: anyhow::Error) -> Result<Result<(), ()>> {
    match guest_err
        .root_cause()
        .downcast_ref::<wasmtime_wasi::I32Exit>()
    {
        Some(trap) => match trap.0 {
            0 => Ok(Ok(())),
            _ => Err(guest_err),
        },
        None => Err(guest_err),
    }
}
