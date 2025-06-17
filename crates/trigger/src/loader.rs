use anyhow::Context as _;
use spin_common::{ui::quoted_path, url::parse_file_url};
use spin_compose::ComponentSourceLoaderFs;
use spin_core::{async_trait, wasmtime, Component};
use spin_factors::{AppComponent, RuntimeFactors};

#[derive(Default)]
pub struct ComponentLoader {
    _private: (),
    #[cfg(feature = "unsafe-aot-compilation")]
    aot_compilation_enabled: bool,
}

impl ComponentLoader {
    /// Create a new `ComponentLoader`
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates the TriggerLoader to load AOT precompiled components
    ///
    /// **Warning: This feature may bypass important security guarantees of the
    /// Wasmtime security sandbox if used incorrectly! Read this documentation
    /// carefully.**
    ///
    /// Usually, components are compiled just-in-time from portable Wasm
    /// sources. This method causes components to instead be loaded
    /// ahead-of-time as Wasmtime-precompiled native executable binaries.
    /// Precompiled binaries must be produced with a compatible Wasmtime engine
    /// using the same Wasmtime version and compiler target settings - typically
    /// by a host with the same processor that will be executing them. See the
    /// Wasmtime documentation for more information:
    /// https://docs.rs/wasmtime/latest/wasmtime/struct.Module.html#method.deserialize
    ///
    /// # Safety
    ///
    /// This method is marked as `unsafe` because it enables potentially unsafe
    /// behavior if used to load malformed or malicious precompiled binaries.
    /// Loading sources from an incompatible Wasmtime engine will fail but is
    /// otherwise safe. This method is safe if it can be guaranteed that
    /// `<TriggerLoader as Loader>::load_component` will only ever be called
    /// with a trusted `LockedComponentSource`. **Precompiled binaries must
    /// never be loaded from untrusted sources.**
    #[cfg(feature = "unsafe-aot-compilation")]
    pub unsafe fn enable_loading_aot_compiled_components(&mut self) {
        self.aot_compilation_enabled = true;
    }

    #[cfg(feature = "unsafe-aot-compilation")]
    fn load_precompiled_component(
        &self,
        engine: &wasmtime::Engine,
        path: &std::path::Path,
    ) -> anyhow::Result<Component> {
        assert!(self.aot_compilation_enabled);
        match wasmtime::Engine::detect_precompiled_file(path)? {
            Some(wasmtime::Precompiled::Component) => unsafe {
                Component::deserialize_file(engine, path)
            },
            Some(wasmtime::Precompiled::Module) => {
                anyhow::bail!("expected AOT compiled component but found module");
            }
            None => {
                anyhow::bail!("expected AOT compiled component but found other data");
            }
        }
    }
}

#[async_trait]
impl<T: RuntimeFactors, U> spin_factors_executor::ComponentLoader<T, U> for ComponentLoader {
    async fn load_component(
        &self,
        engine: &wasmtime::Engine,
        component: &AppComponent,
    ) -> anyhow::Result<Component> {
        let source = component
            .source()
            .content
            .source
            .as_ref()
            .context("LockedComponentSource missing source field")?;
        let path = parse_file_url(source)?;

        #[cfg(feature = "unsafe-aot-compilation")]
        if self.aot_compilation_enabled {
            return self
                .load_precompiled_component(engine, &path)
                .with_context(|| format!("error deserializing component from {path:?}"));
        }

        let composed = spin_compose::compose(&ComponentSourceLoaderFs, component.locked)
            .await
            .with_context(|| {
                format!(
                    "failed to resolve dependencies for component {:?}",
                    component.locked.id
                )
            })?;

        spin_core::Component::new(engine, composed)
            .with_context(|| format!("failed to compile component from {}", quoted_path(&path)))
    }
}
