mod initial_kv_setter;
mod launch_metadata;
mod max_instance_memory;
mod sqlite_statements;
mod stdio;
mod summary;

use std::path::PathBuf;
use std::{future::Future, sync::Arc};

use anyhow::{Context, Result};
use clap::{Args, IntoApp, Parser};
use spin_app::App;
use spin_common::sloth;
use spin_common::ui::quoted_path;
use spin_common::url::parse_file_url;
use spin_factors::RuntimeFactors;
use spin_factors_executor::{ComponentLoader, FactorsExecutor};

use crate::{loader::ComponentLoader as ComponentLoaderImpl, Trigger, TriggerApp};
pub use initial_kv_setter::InitialKvSetterHook;
pub use launch_metadata::LaunchMetadata;
pub use max_instance_memory::MaxInstanceMemoryHook;
pub use sqlite_statements::SqlStatementExecutorHook;
use stdio::FollowComponents;
pub use stdio::StdioLoggingExecutorHooks;
pub use summary::{KeyValueDefaultStoreSummaryHook, SqliteDefaultStoreSummaryHook};

pub const APP_LOG_DIR: &str = "APP_LOG_DIR";
pub const DISABLE_WASMTIME_CACHE: &str = "DISABLE_WASMTIME_CACHE";
pub const FOLLOW_LOG_OPT: &str = "FOLLOW_ID";
pub const WASMTIME_CACHE_FILE: &str = "WASMTIME_CACHE_FILE";
pub const RUNTIME_CONFIG_FILE: &str = "RUNTIME_CONFIG_FILE";

// Set by `spin up`
pub const SPIN_LOCKED_URL: &str = "SPIN_LOCKED_URL";
pub const SPIN_LOCAL_APP_DIR: &str = "SPIN_LOCAL_APP_DIR";
pub const SPIN_WORKING_DIR: &str = "SPIN_WORKING_DIR";

/// A command that runs a TriggerExecutor.
#[derive(Parser, Debug)]
#[clap(
    usage = "spin [COMMAND] [OPTIONS]",
    next_help_heading = help_heading::<T, B::Factors>()
)]
pub struct FactorsTriggerCommand<T: Trigger<B::Factors>, B: RuntimeFactorsBuilder> {
    /// Log directory for the stdout and stderr of components. Setting to
    /// the empty string disables logging to disk.
    #[clap(
        name = APP_LOG_DIR,
        short = 'L',
        long = "log-dir",
        env = "SPIN_LOG_DIR",
    )]
    pub log: Option<PathBuf>,

    /// Disable Wasmtime cache.
    #[clap(
        name = DISABLE_WASMTIME_CACHE,
        long = "disable-cache",
        env = DISABLE_WASMTIME_CACHE,
        conflicts_with = WASMTIME_CACHE_FILE,
        takes_value = false,
    )]
    pub disable_cache: bool,

    /// Wasmtime cache configuration file.
    #[clap(
        name = WASMTIME_CACHE_FILE,
        long = "cache",
        env = WASMTIME_CACHE_FILE,
        conflicts_with = DISABLE_WASMTIME_CACHE,
    )]
    pub cache: Option<PathBuf>,

    /// Disable Wasmtime's pooling instance allocator.
    #[clap(long = "disable-pooling")]
    pub disable_pooling: bool,

    /// Print output to stdout/stderr only for given component(s)
    #[clap(
        name = FOLLOW_LOG_OPT,
        long = "follow",
        multiple_occurrences = true,
    )]
    pub follow_components: Vec<String>,

    /// Silence all component output to stdout/stderr
    #[clap(
        long = "quiet",
        short = 'q',
        aliases = &["sh", "shush"],
        conflicts_with = FOLLOW_LOG_OPT,
        )]
    pub silence_component_logs: bool,

    /// Configuration file for config providers and wasmtime config.
    #[clap(
        name = RUNTIME_CONFIG_FILE,
        long = "runtime-config-file",
        env = RUNTIME_CONFIG_FILE,
    )]
    pub runtime_config_file: Option<PathBuf>,

    /// Set the application state directory path. This is used in the default
    /// locations for logs, key value stores, etc.
    ///
    /// For local apps, this defaults to `.spin/` relative to the `spin.toml` file.
    /// For remote apps, this has no default (unset).
    /// Passing an empty value forces the value to be unset.
    #[clap(long)]
    pub state_dir: Option<String>,

    #[clap(flatten)]
    pub trigger_args: T::CliArgs,

    #[clap(flatten)]
    pub builder_args: B::CliArgs,

    #[clap(long = "help-args-only", hide = true)]
    pub help_args_only: bool,

    #[clap(long = "launch-metadata-only", hide = true)]
    pub launch_metadata_only: bool,
}

/// Configuration options that are common to all triggers.
#[derive(Debug, Default)]
pub struct FactorsConfig {
    /// The Spin working directory.
    pub working_dir: PathBuf,
    /// Path to the runtime config file.
    pub runtime_config_file: Option<PathBuf>,
    /// Path to the state directory.
    pub state_dir: UserProvidedPath,
    /// Path to the local app directory.
    pub local_app_dir: Option<String>,
    /// Which components should have their logs followed.
    pub follow_components: FollowComponents,
    /// Log directory for component stdout/stderr.
    pub log_dir: UserProvidedPath,
}

/// An empty implementation of clap::Args to be used as TriggerExecutor::RunConfig
/// for executors that do not need additional CLI args.
#[derive(Args)]
pub struct NoCliArgs;

impl<T: Trigger<B::Factors>, B: RuntimeFactorsBuilder> FactorsTriggerCommand<T, B> {
    /// Create a new TriggerExecutorBuilder from this TriggerExecutorCommand.
    pub async fn run(self) -> Result<()> {
        // Handle --help-args-only
        if self.help_args_only {
            Self::command()
                .disable_help_flag(true)
                .help_template("{all-args}")
                .print_long_help()?;
            return Ok(());
        }

        // Handle --launch-metadata-only
        if self.launch_metadata_only {
            let lm = LaunchMetadata::infer::<T, B>();
            let json = serde_json::to_string_pretty(&lm)?;
            eprintln!("{json}");
            return Ok(());
        }

        // Required env vars
        let working_dir = std::env::var(SPIN_WORKING_DIR).context(SPIN_WORKING_DIR)?;
        let locked_url = std::env::var(SPIN_LOCKED_URL).context(SPIN_LOCKED_URL)?;
        let local_app_dir = std::env::var(SPIN_LOCAL_APP_DIR).ok();

        let follow_components = self.follow_components();

        // Load App
        let app = {
            let path = parse_file_url(&locked_url)?;
            let contents = std::fs::read(&path)
                .with_context(|| format!("failed to read manifest at {}", quoted_path(&path)))?;
            let locked =
                serde_json::from_slice(&contents).context("failed to parse app lock file JSON")?;
            App::new(locked_url, locked)
        };

        // Validate required host features
        if let Err(unmet) = app.ensure_needs_only(T::TYPE, &T::supported_host_requirements()) {
            anyhow::bail!("This application requires the following features that are not available in this version of the '{}' trigger: {unmet}", T::TYPE);
        }

        let trigger = T::new(self.trigger_args, &app)?;
        let mut builder: TriggerAppBuilder<T, B> = TriggerAppBuilder::new(trigger);
        let config = builder.engine_config();

        // Apply --cache / --disable-cache
        if !self.disable_cache {
            config.enable_cache(&self.cache)?;
        }

        if self.disable_pooling {
            config.disable_pooling();
        }

        let state_dir = match &self.state_dir {
            // Make sure `--state-dir=""` unsets the state dir
            Some(s) if s.is_empty() => UserProvidedPath::Unset,
            Some(s) => UserProvidedPath::Provided(PathBuf::from(s)),
            None => UserProvidedPath::Default,
        };
        let log_dir = match &self.log {
            // Make sure `--log-dir=""` unsets the log dir
            Some(p) if p.as_os_str().is_empty() => UserProvidedPath::Unset,
            Some(p) => UserProvidedPath::Provided(p.clone()),
            None => UserProvidedPath::Default,
        };
        let common_options = FactorsConfig {
            working_dir: PathBuf::from(working_dir),
            runtime_config_file: self.runtime_config_file.clone(),
            state_dir,
            local_app_dir: local_app_dir.clone(),
            follow_components,
            log_dir,
        };

        let run_fut = builder
            .run(
                app,
                common_options,
                self.builder_args,
                &ComponentLoaderImpl::new(),
            )
            .await?;

        let (abortable, abort_handle) = futures::future::abortable(run_fut);
        ctrlc::set_handler(move || abort_handle.abort())?;
        match abortable.await {
            Ok(Ok(())) => {
                tracing::info!("Trigger executor shut down: exiting");
                Ok(())
            }
            Ok(Err(err)) => {
                tracing::error!("Trigger executor failed");
                Err(err)
            }
            Err(_aborted) => {
                tracing::info!("User requested shutdown: exiting");
                Ok(())
            }
        }
    }

    fn follow_components(&self) -> FollowComponents {
        if self.silence_component_logs {
            FollowComponents::None
        } else if self.follow_components.is_empty() {
            FollowComponents::All
        } else {
            let followed = self.follow_components.clone().into_iter().collect();
            FollowComponents::Named(followed)
        }
    }
}

const SLOTH_WARNING_DELAY_MILLIS: u64 = 1250;

fn warn_if_wasm_build_slothful() -> sloth::SlothGuard {
    #[cfg(debug_assertions)]
    let message = "\
        This is a debug build - preparing Wasm modules might take a few seconds\n\
        If you're experiencing long startup times please switch to the release build";

    #[cfg(not(debug_assertions))]
    let message = "Preparing Wasm modules is taking a few seconds...";

    sloth::warn_if_slothful(SLOTH_WARNING_DELAY_MILLIS, format!("{message}\n"))
}

fn help_heading<T: Trigger<F>, F: RuntimeFactors>() -> Option<&'static str> {
    if T::TYPE == <help::HelpArgsOnlyTrigger as Trigger<F>>::TYPE {
        Some("TRIGGER OPTIONS")
    } else {
        let heading = format!("{} TRIGGER OPTIONS", T::TYPE.to_uppercase());
        let as_str = Box::new(heading).leak();
        Some(as_str)
    }
}

/// A builder for a [`TriggerApp`].
pub struct TriggerAppBuilder<T, B> {
    engine_config: spin_core::Config,
    pub trigger: T,
    _factors_builder: std::marker::PhantomData<B>,
}

impl<T: Trigger<B::Factors>, B: RuntimeFactorsBuilder> TriggerAppBuilder<T, B> {
    pub fn new(trigger: T) -> Self {
        Self {
            engine_config: spin_core::Config::default(),
            trigger,
            _factors_builder: Default::default(),
        }
    }

    pub fn engine_config(&mut self) -> &mut spin_core::Config {
        &mut self.engine_config
    }

    /// Build a [`TriggerApp`] from the given [`App`] and options.
    pub async fn build(
        &mut self,
        app: App,
        common_options: FactorsConfig,
        options: B::CliArgs,
        loader: &impl ComponentLoader<B::Factors, T::InstanceState>,
    ) -> anyhow::Result<TriggerApp<T, B::Factors>> {
        let mut core_engine_builder = {
            self.trigger.update_core_config(&mut self.engine_config)?;

            spin_core::Engine::builder(&self.engine_config)?
        };
        self.trigger.add_to_linker(core_engine_builder.linker())?;

        let (factors, runtime_config) = B::build(&common_options, &options)?;

        let mut executor = FactorsExecutor::new(core_engine_builder, factors)?;
        B::configure_app(&mut executor, &runtime_config, &common_options, &options)?;
        let executor = Arc::new(executor);

        let configured_app = {
            let _sloth_guard = warn_if_wasm_build_slothful();
            executor
                .load_app(app, runtime_config.into(), loader)
                .await?
        };

        Ok(configured_app)
    }

    /// Run the [`TriggerApp`] with the given [`App`] and options.
    pub async fn run(
        mut self,
        app: App,
        common_options: FactorsConfig,
        options: B::CliArgs,
        loader: &impl ComponentLoader<B::Factors, T::InstanceState>,
    ) -> anyhow::Result<impl Future<Output = anyhow::Result<()>>> {
        let configured_app = self.build(app, common_options, options, loader).await?;
        Ok(self.trigger.run(configured_app))
    }
}

/// A builder for runtime factors.
pub trait RuntimeFactorsBuilder {
    /// The factors type to build.
    type Factors: RuntimeFactors;
    /// CLI arguments not included in [`FactorsConfig`] needed  to build the [`RuntimeFactors`].
    type CliArgs: clap::Args;
    /// The wrapped runtime config type.
    type RuntimeConfig: Into<<Self::Factors as RuntimeFactors>::RuntimeConfig>;

    /// Build the factors and runtime config from the given options.
    fn build(
        config: &FactorsConfig,
        args: &Self::CliArgs,
    ) -> anyhow::Result<(Self::Factors, Self::RuntimeConfig)>;

    /// Configure the factors in the executor.
    fn configure_app<U: Send + 'static>(
        executor: &mut FactorsExecutor<Self::Factors, U>,
        runtime_config: &Self::RuntimeConfig,
        config: &FactorsConfig,
        args: &Self::CliArgs,
    ) -> anyhow::Result<()> {
        let _ = (executor, runtime_config, config, args);
        Ok(())
    }
}

pub mod help {
    use super::*;

    /// Null object to support --help-args-only in the absence of
    /// a `spin.toml` file.
    pub struct HelpArgsOnlyTrigger;

    impl<F: RuntimeFactors> Trigger<F> for HelpArgsOnlyTrigger {
        const TYPE: &'static str = "help-args-only";
        type CliArgs = NoCliArgs;
        type InstanceState = ();

        fn new(_cli_args: Self::CliArgs, _app: &App) -> anyhow::Result<Self> {
            Ok(Self)
        }

        async fn run(self, _configured_app: TriggerApp<Self, F>) -> anyhow::Result<()> {
            Ok(())
        }
    }
}

/// A user provided option which be either be provided, default, or explicitly none.
#[derive(Clone, Debug, Default)]
pub enum UserProvidedPath {
    /// Use the explicitly provided directory.
    Provided(PathBuf),
    /// Use the default.
    #[default]
    Default,
    /// Explicitly unset.
    Unset,
}
