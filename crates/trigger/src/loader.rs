use spin_common::{ui::quoted_path, url::parse_file_url};
use spin_compose::ComponentSourceLoaderFs;
use spin_core::{Component, async_trait, wasmtime};
use spin_factors::{AppComponent, RuntimeFactors};
use spin_factors_executor::TriggerDependencyData;
use wasmtime::error::Context as _;

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
    ) -> wasmtime::Result<Component> {
        assert!(self.aot_compilation_enabled);
        match wasmtime::Engine::detect_precompiled_file(path)? {
            Some(wasmtime::Precompiled::Component) => unsafe {
                Component::deserialize_file(engine, path)
            },
            Some(wasmtime::Precompiled::Module) => {
                wasmtime::bail!("expected AOT compiled component but found module");
            }
            None => {
                wasmtime::bail!("expected AOT compiled component but found other data");
            }
        }
    }

    pub(crate) async fn load_composed(
        &self,
        component: &AppComponent<'_>,
        trigger_dependencies_composer: &impl spin_factors_executor::TriggerDependenciesComposer,
    ) -> anyhow::Result<Vec<u8>> {
        let loader = ComponentSourceLoaderFs;

        let trigger_deps = &component.locked.trigger_dependencies;

        let trigger_deps = load_trigger_dependencies(&mut trigger_deps.iter(), &loader).await?;

        let apply_trigger_deps = async |c: Vec<u8>| {
            trigger_dependencies_composer
                .compose_trigger_dependencies(&trigger_deps, c)
                .await
                .map_err(spin_compose::ComposeError::PrepareError)
        };

        let composed = spin_compose::compose(&loader, component.locked, apply_trigger_deps)
            .await
            .with_context(|| {
                format!(
                    "failed to resolve dependencies for component {:?}",
                    component.locked.id
                )
            })?;

        Ok(composed)
    }
}

#[async_trait]
impl<T: RuntimeFactors, U> spin_factors_executor::ComponentLoader<T, U> for ComponentLoader {
    async fn load_component(
        &self,
        engine: &wasmtime::Engine,
        component: &AppComponent,
        trigger_dependencies_composer: &impl spin_factors_executor::TriggerDependenciesComposer,
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
            let component = self
                .load_precompiled_component(engine, &path)
                .with_context(|| format!("error deserializing component from {path:?}"))?;
            return Ok(component);
        }

        let composed = self
            .load_composed(component, trigger_dependencies_composer)
            .await?;

        let component = spin_core::Component::new(engine, composed)
            .with_context(|| format!("failed to compile component from {}", quoted_path(&path)))?;
        Ok(component)
    }
}

pub(crate) async fn load_trigger_dependencies(
    trigger_dependencies: &mut impl ExactSizeIterator<
        Item = (&String, &Vec<spin_app::locked::LockedComponentDependency>),
    >,
    loader: &spin_compose::ComponentSourceLoaderFs,
) -> Result<
    std::collections::HashMap<String, Vec<spin_factors_executor::TriggerDependency>>,
    anyhow::Error,
> {
    use spin_factors_executor::TriggerDependency;
    use std::collections::HashMap;

    let mut resolved_trigger_deps = HashMap::with_capacity(trigger_dependencies.len());

    for (role, role_components) in trigger_dependencies {
        let mut deps_for_role = Vec::with_capacity(role_components.len());

        for locked_dep in role_components {
            let data = load_trigger_dep_data(loader, &locked_dep.source).await?;
            deps_for_role.push(TriggerDependency {
                data,
                dependency: locked_dep.clone(),
            });
        }
        resolved_trigger_deps.insert(role.clone(), deps_for_role);
    }

    Ok(resolved_trigger_deps)
}

async fn load_trigger_dep_data(
    loader: &ComponentSourceLoaderFs,
    source: &spin_app::locked::LockedComponentSource,
) -> anyhow::Result<TriggerDependencyData> {
    use spin_compose::ComponentSourceLoader;

    if let Some(path) = source
        .content
        .source
        .as_ref()
        .and_then(|url| parse_file_url(url).ok())
    {
        Ok(TriggerDependencyData::OnDisk(path))
    } else {
        Ok(TriggerDependencyData::InMemory(
            loader.load_source(source).await?,
        ))
    }
}
