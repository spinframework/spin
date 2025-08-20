use spin_app::locked::LockedApp;
use spin_factors::{
    anyhow::{self, Context},
    wasmtime::{component::Linker, Config, Engine},
    App, RuntimeFactors,
};
use spin_loader::FilesMountStrategy;

pub use toml::toml;

/// A test environment for building [`RuntimeFactors`] instances.
pub struct TestEnvironment<T: RuntimeFactors> {
    /// The RuntimeFactors under test.
    pub factors: T,
    /// The `spin.toml` manifest.
    pub manifest: toml::Table,
    /// Runtime configuration for the factors.
    pub runtime_config: T::RuntimeConfig,
}

impl<T: RuntimeFactors> TestEnvironment<T> {
    /// Creates a new test environment by initializing the given
    /// [`RuntimeFactors`].
    pub fn new(mut factors: T) -> Self {
        spin_telemetry::testing::init_test_telemetry();

        let engine = Engine::new(Config::new().async_support(true))
            .expect("wasmtime engine failed to initialize");
        let mut linker = Linker::<T::InstanceState>::new(&engine);
        factors
            .init(&mut linker)
            .expect("RuntimeFactors::init failed");

        let manifest = toml! {
            spin_manifest_version = 2

            [application]
            name = "test-app"

            [[trigger.test-trigger]]

            [component.empty]
            source = "does-not-exist.wasm"
        };
        Self {
            factors,
            manifest,
            runtime_config: Default::default(),
        }
    }

    /// Extends the manifest with the given TOML.
    ///
    /// The default manifest includes boilerplate like the
    /// `spin_manifest_version` and `[application]` section, so you typically
    /// need to pass only a `[component.test-component]` section.
    pub fn extend_manifest(mut self, manifest_merge: toml::Table) -> Self {
        self.manifest.extend(manifest_merge);
        self
    }

    /// Sets the runtime config.
    pub fn runtime_config<C, E>(mut self, runtime_config: C) -> anyhow::Result<Self>
    where
        C: TryInto<T::RuntimeConfig, Error = E>,
        E: Into<anyhow::Error>,
    {
        self.runtime_config = runtime_config
            .try_into()
            .map_err(Into::into)
            .context("failed to build runtime config")?;
        Ok(self)
    }

    /// Run through the [`Factor`]s' lifecycle(s) to build a
    /// [`RuntimeFactors::InstanceState`] for the last component defined in the
    /// manifest.
    pub async fn build_instance_state(self) -> anyhow::Result<T::InstanceState> {
        let locked_app = self
            .build_locked_app()
            .await
            .context("failed to build locked app")?;
        let app = App::new("test-app", locked_app);
        let configured_app = self.factors.configure_app(app, self.runtime_config)?;

        let component =
            configured_app.app().components().last().context(
                "expected configured app to have at least one component, but it did not",
            )?;
        let builders = self.factors.prepare(&configured_app, component.id())?;

        Ok(self.factors.build_instance_state(builders)?)
    }

    pub async fn build_locked_app(&self) -> anyhow::Result<LockedApp> {
        build_locked_app(&self.manifest).await
    }
}

pub async fn build_locked_app(manifest: &toml::Table) -> anyhow::Result<LockedApp> {
    let toml_str = toml::to_string(manifest).context("failed serializing manifest")?;
    let dir = tempfile::tempdir().context("failed creating tempdir")?;
    let path = dir.path().join("spin.toml");
    std::fs::write(&path, toml_str).context("failed writing manifest")?;
    spin_loader::from_file(&path, FilesMountStrategy::Direct, None).await
}
