//! Loading an application for validation.

use std::path::Path;

use anyhow::{anyhow, Context};
use futures::future::try_join_all;
use spin_common::ui::quoted_path;

pub(crate) struct ComponentToValidate<'a> {
    id: &'a str,
    source_description: String,
    wasm: Vec<u8>,
    host_requirements: Vec<String>,
}

impl ComponentToValidate<'_> {
    pub fn id(&self) -> &str {
        self.id
    }

    pub fn source_description(&self) -> &str {
        &self.source_description
    }

    pub fn wasm_bytes(&self) -> &[u8] {
        &self.wasm
    }

    pub fn host_requirements(&self) -> &[String] {
        &self.host_requirements
    }

    #[cfg(test)]
    pub(crate) fn new(
        id: &'static str,
        description: &str,
        wasm: Vec<u8>,
        host_requirements: Vec<String>,
    ) -> Self {
        Self {
            id,
            source_description: description.to_owned(),
            wasm,
            host_requirements,
        }
    }
}

pub struct ApplicationToValidate {
    manifest: spin_manifest::schema::v2::AppManifest,
    wasm_loader: spin_loader::WasmLoader,
}

impl ApplicationToValidate {
    pub async fn new(
        manifest: spin_manifest::schema::v2::AppManifest,
        base_dir: impl AsRef<Path>,
    ) -> anyhow::Result<Self> {
        let wasm_loader =
            spin_loader::WasmLoader::new(base_dir.as_ref().to_owned(), None, None).await?;
        Ok(Self {
            manifest,
            wasm_loader,
        })
    }

    fn component_source<'a>(
        &'a self,
        trigger: &'a spin_manifest::schema::v2::Trigger,
    ) -> anyhow::Result<ComponentSource<'a>> {
        let component_spec = trigger
            .component
            .as_ref()
            .ok_or_else(|| anyhow!("No component specified for trigger {}", trigger.id))?;
        let (id, source, dependencies, service_chaining) = match component_spec {
            spin_manifest::schema::v2::ComponentSpec::Inline(c) => (
                trigger.id.as_str(),
                &c.source,
                &c.dependencies,
                spin_loader::requires_service_chaining(c),
            ),
            spin_manifest::schema::v2::ComponentSpec::Reference(r) => {
                let id = r.as_ref();
                let Some(component) = self.manifest.components.get(r) else {
                    anyhow::bail!(
                        "Component {id} specified for trigger {} does not exist",
                        trigger.id
                    );
                };
                (
                    id,
                    &component.source,
                    &component.dependencies,
                    spin_loader::requires_service_chaining(component),
                )
            }
        };

        Ok(ComponentSource {
            id,
            source,
            dependencies: WrappedComponentDependencies::new(dependencies),
            requires_service_chaining: service_chaining,
        })
    }

    pub fn trigger_types(&self) -> impl Iterator<Item = &String> {
        self.manifest.triggers.keys()
    }

    pub fn triggers(
        &self,
    ) -> impl Iterator<Item = (&String, &Vec<spin_manifest::schema::v2::Trigger>)> {
        self.manifest.triggers.iter()
    }

    pub(crate) async fn components_by_trigger_type(
        &self,
    ) -> anyhow::Result<Vec<(String, Vec<ComponentToValidate<'_>>)>> {
        use futures::FutureExt;

        let components_by_trigger_type_futs = self.triggers().map(|(ty, ts)| {
            self.components_for_trigger(ts)
                .map(|css| css.map(|css| (ty.to_owned(), css)))
        });
        let components_by_trigger_type = try_join_all(components_by_trigger_type_futs)
            .await
            .context("Failed to prepare components for target environment checking")?;
        Ok(components_by_trigger_type)
    }

    async fn components_for_trigger<'a>(
        &'a self,
        triggers: &'a [spin_manifest::schema::v2::Trigger],
    ) -> anyhow::Result<Vec<ComponentToValidate<'a>>> {
        let component_futures = triggers.iter().map(|t| self.load_and_resolve_trigger(t));
        try_join_all(component_futures).await
    }

    async fn load_and_resolve_trigger<'a>(
        &'a self,
        trigger: &'a spin_manifest::schema::v2::Trigger,
    ) -> anyhow::Result<ComponentToValidate<'a>> {
        let component = self.component_source(trigger)?;

        let loader = ComponentSourceLoader::new(&self.wasm_loader);

        let wasm = spin_compose::compose(&loader, &component).await.with_context(|| format!("Spin needed to compose dependencies for {} as part of target checking, but composition failed", component.id))?;

        let host_requirements = if component.requires_service_chaining {
            vec!["local_service_chaining".to_string()]
        } else {
            vec![]
        };

        Ok(ComponentToValidate {
            id: component.id,
            source_description: source_description(component.source),
            wasm,
            host_requirements,
        })
    }
}

struct ComponentSource<'a> {
    id: &'a str,
    source: &'a spin_manifest::schema::v2::ComponentSource,
    dependencies: WrappedComponentDependencies,
    requires_service_chaining: bool,
}

struct ComponentSourceLoader<'a> {
    wasm_loader: &'a spin_loader::WasmLoader,
}

impl<'a> ComponentSourceLoader<'a> {
    pub fn new(wasm_loader: &'a spin_loader::WasmLoader) -> Self {
        Self { wasm_loader }
    }
}

#[async_trait::async_trait]
impl<'a> spin_compose::ComponentSourceLoader for ComponentSourceLoader<'a> {
    type Component = ComponentSource<'a>;
    type Dependency = WrappedComponentDependency;
    async fn load_component_source(&self, source: &Self::Component) -> anyhow::Result<Vec<u8>> {
        let path = self
            .wasm_loader
            .load_component_source(source.id, source.source)
            .await?;
        let bytes = tokio::fs::read(&path).await?;
        let component = spin_componentize::componentize_if_necessary(&bytes)?;
        Ok(component.into())
    }

    async fn load_dependency_source(&self, source: &Self::Dependency) -> anyhow::Result<Vec<u8>> {
        let (path, _) = self
            .wasm_loader
            .load_component_dependency(&source.name, &source.dependency)
            .await?;
        let bytes = tokio::fs::read(&path).await?;
        let component = spin_componentize::componentize_if_necessary(&bytes)?;
        Ok(component.into())
    }
}

// This exists only to thwart the orphan rule
struct WrappedComponentDependency {
    name: spin_serde::DependencyName,
    dependency: spin_manifest::schema::v2::ComponentDependency,
}

// To manage lifetimes around the thwarting of the orphan rule
struct WrappedComponentDependencies {
    dependencies: indexmap::IndexMap<spin_serde::DependencyName, WrappedComponentDependency>,
}

impl WrappedComponentDependencies {
    fn new(deps: &spin_manifest::schema::v2::ComponentDependencies) -> Self {
        let dependencies = deps
            .inner
            .clone()
            .into_iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    WrappedComponentDependency {
                        name: k,
                        dependency: v,
                    },
                )
            })
            .collect();
        Self { dependencies }
    }
}

#[async_trait::async_trait]
impl spin_compose::ComponentLike for ComponentSource<'_> {
    type Dependency = WrappedComponentDependency;

    fn dependencies(
        &self,
    ) -> impl std::iter::ExactSizeIterator<Item = (&spin_serde::DependencyName, &Self::Dependency)>
    {
        self.dependencies.dependencies.iter()
    }

    fn id(&self) -> &str {
        self.id
    }
}

#[async_trait::async_trait]
impl spin_compose::DependencyLike for WrappedComponentDependency {
    fn inherit(&self) -> spin_compose::InheritConfiguration {
        // We don't care because this never runs - it is only used to
        // verify import satisfaction. Choosing All avoids the compose
        // algorithm meddling with it using the deny adapter.
        spin_compose::InheritConfiguration::All
    }

    fn export(&self) -> &Option<String> {
        match &self.dependency {
            spin_manifest::schema::v2::ComponentDependency::Version(_) => &None,
            spin_manifest::schema::v2::ComponentDependency::Package { export, .. } => export,
            spin_manifest::schema::v2::ComponentDependency::Local { export, .. } => export,
            spin_manifest::schema::v2::ComponentDependency::HTTP { export, .. } => export,
        }
    }
}

fn source_description(source: &spin_manifest::schema::v2::ComponentSource) -> String {
    match source {
        spin_manifest::schema::v2::ComponentSource::Local(path) => {
            format!("file {}", quoted_path(path))
        }
        spin_manifest::schema::v2::ComponentSource::Remote { url, .. } => format!("URL {url}"),
        spin_manifest::schema::v2::ComponentSource::Registry { package, .. } => {
            format!("package {package}")
        }
    }
}
