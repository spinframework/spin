mod build;

pub use build::FactorsBuilder;

use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context as _;
use spin_common::arg_parser::parse_kv;
use spin_factor_key_value::KeyValueFactor;
use spin_factor_llm::LlmFactor;
use spin_factor_outbound_http::OutboundHttpFactor;
use spin_factor_outbound_mqtt::{NetworkedMqttClient, OutboundMqttFactor};
use spin_factor_outbound_mysql::OutboundMysqlFactor;
use spin_factor_outbound_networking::OutboundNetworkingFactor;
use spin_factor_outbound_pg::OutboundPgFactor;
use spin_factor_outbound_redis::OutboundRedisFactor;
use spin_factor_sqlite::SqliteFactor;
use spin_factor_variables::VariablesFactor;
use spin_factor_wasi::{spin::SpinFilesMounter, WasiFactor};
use spin_factors::RuntimeFactors;
use spin_runtime_config::{ResolvedRuntimeConfig, TomlRuntimeConfigSource};
use spin_variables_static::VariableSource;

#[derive(RuntimeFactors)]
pub struct TriggerFactors {
    pub wasi: WasiFactor,
    pub variables: VariablesFactor,
    pub key_value: KeyValueFactor,
    pub outbound_networking: OutboundNetworkingFactor,
    pub outbound_http: OutboundHttpFactor,
    pub sqlite: SqliteFactor,
    pub redis: OutboundRedisFactor,
    pub mqtt: OutboundMqttFactor,
    pub pg: OutboundPgFactor,
    pub mysql: OutboundMysqlFactor,
    pub llm: LlmFactor,
}

impl TriggerFactors {
    pub fn new(
        state_dir: Option<PathBuf>,
        working_dir: impl Into<PathBuf>,
        allow_transient_writes: bool,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            wasi: wasi_factor(working_dir, allow_transient_writes),
            variables: VariablesFactor::default(),
            key_value: KeyValueFactor::new(),
            outbound_networking: outbound_networking_factor(),
            outbound_http: OutboundHttpFactor::default(),
            sqlite: SqliteFactor::new(),
            redis: OutboundRedisFactor::new(),
            mqtt: OutboundMqttFactor::new(NetworkedMqttClient::creator()),
            pg: OutboundPgFactor::new(),
            mysql: OutboundMysqlFactor::new(),
            llm: LlmFactor::new(
                spin_factor_llm::spin::default_engine_creator(state_dir)
                    .context("failed to configure LLM factor")?,
            ),
        })
    }
}

fn wasi_factor(working_dir: impl Into<PathBuf>, allow_transient_writes: bool) -> WasiFactor {
    WasiFactor::new(SpinFilesMounter::new(working_dir, allow_transient_writes))
}

fn outbound_networking_factor() -> OutboundNetworkingFactor {
    fn disallowed_host_handler(scheme: &str, authority: &str) {
        let host_pattern = format!("{scheme}://{authority}");
        tracing::error!("Outbound network destination not allowed: {host_pattern}");
        if scheme.starts_with("http") && authority == "self" {
            terminal::warn!("A component tried to make an HTTP request to its own app but it does not have permission.");
        } else {
            terminal::warn!(
                "A component tried to make an outbound network connection to disallowed destination '{host_pattern}'."
            );
        };
        eprintln!("To allow this request, add 'allowed_outbound_hosts = [\"{host_pattern}\"]' to the manifest component section.");
    }

    let mut factor = OutboundNetworkingFactor::new();
    factor.set_disallowed_host_handler(disallowed_host_handler);
    factor
}

/// Options for building a [`TriggerFactors`].
#[derive(Default, clap::Args)]
pub struct TriggerAppArgs {
    /// Set the static assets of the components in the temporary directory as writable.
    #[clap(long = "allow-transient-write")]
    pub allow_transient_write: bool,

    /// Set a key/value pair (key=value) in the application's
    /// default store. Any existing value will be overwritten.
    /// Can be used multiple times.
    #[clap(long = "key-value", parse(try_from_str = parse_kv))]
    pub key_values: Vec<(String, String)>,

    /// Run a SQLite statement such as a migration against the default database.
    /// To run from a file, prefix the filename with @ e.g. spin up --sqlite @migration.sql
    #[clap(long = "sqlite")]
    pub sqlite_statements: Vec<String>,

    /// Sets the maxmimum memory allocation limit for an instance in bytes.
    #[clap(long, env = "SPIN_MAX_INSTANCE_MEMORY")]
    pub max_instance_memory: Option<usize>,

    /// Variable(s) to be passed to the app
    ///
    /// A single key-value pair can be passed as `key=value`. Alternatively, the
    /// path to a JSON or TOML file may be given as `@file.json` or
    /// `@file.toml`.
    ///
    /// This option may be repeated. If the same key is specified multiple times
    /// the last value will be used.
    #[clap(long, multiple = true, value_parser = clap::value_parser!(VariableSource),
        value_name = "KEY=VALUE | @FILE.json | @FILE.toml")]
    pub variable: Vec<VariableSource>,

    /// Cache variables to avoid reading files twice
    #[clap(skip)]
    variables_cache: OnceCell<HashMap<String, String>>,
}

impl From<ResolvedRuntimeConfig<TriggerFactorsRuntimeConfig>> for TriggerFactorsRuntimeConfig {
    fn from(value: ResolvedRuntimeConfig<TriggerFactorsRuntimeConfig>) -> Self {
        value.runtime_config
    }
}

impl TryFrom<TomlRuntimeConfigSource<'_, '_>> for TriggerFactorsRuntimeConfig {
    type Error = anyhow::Error;

    fn try_from(value: TomlRuntimeConfigSource<'_, '_>) -> Result<Self, Self::Error> {
        Self::from_source(value)
    }
}

impl TriggerAppArgs {
    /// Parse all variable sources into a single merged map.
    pub fn get_variables(&self) -> anyhow::Result<&HashMap<String, String>> {
        if self.variables_cache.get().is_none() {
            let mut variables = HashMap::new();
            for source in &self.variable {
                variables.extend(source.get_variables()?);
            }
            self.variables_cache.set(variables).unwrap();
        }
        Ok(self.variables_cache.get().unwrap())
    }
}
