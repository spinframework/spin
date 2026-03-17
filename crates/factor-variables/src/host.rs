use spin_core::wasmtime::component::Accessor;
use spin_factors::anyhow;
use spin_telemetry::traces::{self, Blame};
use spin_world::{
    spin::variables::variables as v3, v1, v2::variables as v2, wasi::config as wasi_config,
};
use tracing::instrument;

use crate::{InstanceState, VariablesFactorData};

impl v3::HostWithStore for VariablesFactorData {
    #[instrument(name = "spin_variables.get", skip(accessor), fields(otel.kind = "client"))]
    async fn get<T: Send>(accessor: &Accessor<T, Self>, key: String) -> Result<String, v3::Error> {
        let (resolver, component_id) = accessor.with(|mut access| {
            let host = access.get();
            host.otel.reparent_tracing_span();
            (host.expression_resolver.clone(), host.component_id.clone())
        });

        let key = spin_expressions::Key::new(&key).map_err(expressions_to_variables_err_v3)?;

        resolver
            .resolve(&component_id, key)
            .await
            .map_err(expressions_to_variables_err_v3)
    }
}

impl v3::Host for InstanceState {
    fn convert_error(&mut self, err: v3::Error) -> anyhow::Result<v3::Error> {
        Ok(err)
    }
}

impl v2::Host for InstanceState {
    #[instrument(name = "spin_variables.get", skip(self), fields(otel.kind = "client"))]
    async fn get(&mut self, key: String) -> Result<String, v2::Error> {
        self.otel.reparent_tracing_span();
        let key = spin_expressions::Key::new(&key).map_err(expressions_to_variables_err)?;
        self.expression_resolver
            .resolve(&self.component_id, key)
            .await
            .map_err(expressions_to_variables_err)
    }

    fn convert_error(&mut self, error: v2::Error) -> anyhow::Result<v2::Error> {
        Ok(error)
    }
}

impl v1::config::Host for InstanceState {
    #[instrument(name = "spin_config.get", skip(self), fields(otel.kind = "client"))]
    async fn get_config(&mut self, key: String) -> Result<String, v1::config::Error> {
        <Self as v2::Host>::get(self, key)
            .await
            .map_err(|err| match err {
                v2::Error::InvalidName(msg) => v1::config::Error::InvalidKey(msg),
                v2::Error::Undefined(msg) => v1::config::Error::Provider(msg),
                other => v1::config::Error::Other(format!("{other}")),
            })
    }

    fn convert_error(&mut self, err: v1::config::Error) -> anyhow::Result<v1::config::Error> {
        Ok(err)
    }
}

impl wasi_config::store::Host for InstanceState {
    #[instrument(name = "wasi_config.get", skip(self), fields(otel.kind = "client"))]
    async fn get(&mut self, key: String) -> Result<Option<String>, wasi_config::store::Error> {
        match <Self as v2::Host>::get(self, key).await {
            Ok(value) => Ok(Some(value)),
            Err(v2::Error::Undefined(_)) => Ok(None),
            Err(v2::Error::InvalidName(_)) => Ok(None), // this is the guidance from https://github.com/WebAssembly/wasi-runtime-config/pull/19)
            Err(v2::Error::Provider(msg)) => Err(wasi_config::store::Error::Upstream(msg)),
            Err(v2::Error::Other(msg)) => Err(wasi_config::store::Error::Io(msg)),
        }
    }

    #[instrument(name = "wasi_config.get_all", skip(self), fields(otel.kind = "client"))]
    async fn get_all(&mut self) -> Result<Vec<(String, String)>, wasi_config::store::Error> {
        let all = self
            .expression_resolver
            .resolve_all(&self.component_id)
            .await;
        all.map_err(|e| {
            match expressions_to_variables_err(e) {
                v2::Error::Undefined(msg) => wasi_config::store::Error::Io(msg), // this shouldn't happen but just in case
                v2::Error::InvalidName(msg) => wasi_config::store::Error::Io(msg), // this shouldn't happen but just in case
                v2::Error::Provider(msg) => wasi_config::store::Error::Upstream(msg),
                v2::Error::Other(msg) => wasi_config::store::Error::Io(msg),
            }
        })
    }

    fn convert_error(
        &mut self,
        err: wasi_config::store::Error,
    ) -> anyhow::Result<wasi_config::store::Error> {
        Ok(err)
    }
}

/// Convert a `spin_expressions::Error` to a `v2::Error`, setting the current span's status and fault attribute.
fn expressions_to_variables_err(err: spin_expressions::Error) -> v2::Error {
    use spin_expressions::Error;
    let blame = match err {
        Error::InvalidName(_) | Error::InvalidTemplate(_) | Error::Undefined(_) => Blame::Guest,
        Error::Provider(_) => Blame::Host,
    };
    traces::mark_as_error(&err, Some(blame));
    match err {
        Error::InvalidName(msg) => v2::Error::InvalidName(msg),
        Error::Undefined(msg) => v2::Error::Undefined(msg),
        Error::InvalidTemplate(_) => v2::Error::Other(format!("{err}")),
        Error::Provider(err) => v2::Error::Provider(err.to_string()),
    }
}

/// Convert a `spin_expressions::Error` to a `v3::Error`, setting the current span's status and fault attribute.
fn expressions_to_variables_err_v3(err: spin_expressions::Error) -> v3::Error {
    use spin_expressions::Error;
    let blame = match err {
        Error::InvalidName(_) | Error::InvalidTemplate(_) | Error::Undefined(_) => Blame::Guest,
        Error::Provider(_) => Blame::Host,
    };
    traces::mark_as_error(&err, Some(blame));
    match err {
        Error::InvalidName(msg) => v3::Error::InvalidName(msg),
        Error::Undefined(msg) => v3::Error::Undefined(msg),
        Error::InvalidTemplate(_) => v3::Error::Other(format!("{err}")),
        Error::Provider(err) => v3::Error::Provider(err.to_string()),
    }
}
