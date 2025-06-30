use anyhow::Context;
use indexmap::IndexMap;
use semver::Version;
use spin_app::locked::InheritConfiguration as LockedInheritConfiguration;
use spin_common::{ui::quoted_path, url::parse_file_url};
use spin_serde::{DependencyName, KebabId};
use std::collections::BTreeMap;
use thiserror::Error;
use wac_graph::types::{Package, SubtypeChecker, WorldId};
use wac_graph::{CompositionGraph, NodeId};

/// Composes a Spin AppComponent using the dependencies specified in the
/// component's dependencies section.
///
/// To compose the dependent component with its dependencies, the composer will
/// first prepare the dependencies by maximally matching depenedency names to
/// import names and register dependency components with the composition graph
/// with the `deny-all` adapter applied if the set of configurations to inherit
/// is the empty set. Once this mapping of import names to dependency infos is
/// constructed the composer will build the instantiation arguments for the
/// dependent component by ensuring that the export type of the dependency is a
/// subtype of the import type of the dependent component. If the dependency has
/// an export name specified, the composer will use that export name to satisfy
/// the import. If the dependency does not have an export name specified, the
/// composer will use an export of import name to satisfy the import. The
/// composer will then alias the export of the dependency to the import of the
/// dependent component. Finally, the composer will export all exports from the
/// dependent component to its dependents. The composer will then encode the
/// composition graph into a byte array and return it.
pub async fn compose<L: ComponentSourceLoader>(
    loader: &L,
    component: &L::Component,
) -> Result<Vec<u8>, ComposeError> {
    Composer::new(loader).compose(component).await
}

/// A Spin component dependency. This abstracts over the metadata associated with the
/// dependency. The abstraction allows both manifest and lockfile types to participate in composition.
#[async_trait::async_trait]
pub trait DependencyLike {
    fn inherit(&self) -> InheritConfiguration;
    fn export(&self) -> &Option<String>;
}

pub enum InheritConfiguration {
    All,
    Some(Vec<String>),
}

/// A Spin component. This abstracts over the list of dependencies for the component.
/// The abstraction allows both manifest and lockfile types to participate in composition.
#[async_trait::async_trait]
pub trait ComponentLike {
    type Dependency: DependencyLike;

    fn dependencies(
        &self,
    ) -> impl std::iter::ExactSizeIterator<Item = (&DependencyName, &Self::Dependency)>;
    fn id(&self) -> &str;
}

#[async_trait::async_trait]
impl ComponentLike for spin_app::locked::LockedComponent {
    type Dependency = spin_app::locked::LockedComponentDependency;

    fn dependencies(
        &self,
    ) -> impl std::iter::ExactSizeIterator<Item = (&DependencyName, &Self::Dependency)> {
        self.dependencies.iter()
    }

    fn id(&self) -> &str {
        &self.id
    }
}

#[async_trait::async_trait]
impl DependencyLike for spin_app::locked::LockedComponentDependency {
    fn inherit(&self) -> InheritConfiguration {
        match &self.inherit {
            LockedInheritConfiguration::All => InheritConfiguration::All,
            LockedInheritConfiguration::Some(cfgs) => InheritConfiguration::Some(cfgs.clone()),
        }
    }

    fn export(&self) -> &Option<String> {
        &self.export
    }
}

/// This trait is used to load component source code from a locked component source across various embdeddings.
#[async_trait::async_trait]
pub trait ComponentSourceLoader {
    type Component: ComponentLike<Dependency = Self::Dependency>;
    type Dependency: DependencyLike;
    async fn load_component_source(&self, source: &Self::Component) -> anyhow::Result<Vec<u8>>;
    async fn load_dependency_source(&self, source: &Self::Dependency) -> anyhow::Result<Vec<u8>>;
}

/// A ComponentSourceLoader that loads component sources from the filesystem.
pub struct ComponentSourceLoaderFs;

#[async_trait::async_trait]
impl ComponentSourceLoader for ComponentSourceLoaderFs {
    type Component = spin_app::locked::LockedComponent;
    type Dependency = spin_app::locked::LockedComponentDependency;

    async fn load_component_source(&self, source: &Self::Component) -> anyhow::Result<Vec<u8>> {
        Self::load_from_locked_source(&source.source).await
    }

    async fn load_dependency_source(&self, source: &Self::Dependency) -> anyhow::Result<Vec<u8>> {
        Self::load_from_locked_source(&source.source).await
    }
}

impl ComponentSourceLoaderFs {
    async fn load_from_locked_source(
        source: &spin_app::locked::LockedComponentSource,
    ) -> anyhow::Result<Vec<u8>> {
        let source = source
            .content
            .source
            .as_ref()
            .context("LockedComponentSource missing source field")?;

        let path = parse_file_url(source)?;

        let bytes: Vec<u8> = tokio::fs::read(&path).await.with_context(|| {
            format!(
                "failed to read component source from disk at path {}",
                quoted_path(&path)
            )
        })?;

        let component = spin_componentize::componentize_if_necessary(&bytes)?;

        Ok(component.into())
    }
}

/// Represents an error that can occur when composing dependencies.
#[derive(Debug, Error)]
pub enum ComposeError {
    /// A dependency name does not match any import names.
    #[error(
        "dependency '{dependency_name}' doesn't match any imports of component '{component_id}'"
    )]
    UnmatchedDependencyName {
        component_id: String,
        dependency_name: DependencyName,
    },
    /// A component has dependency conflicts.
    #[error("component '{component_id}' has dependency conflicts: {}", format_conflicts(.conflicts))]
    DependencyConflicts {
        component_id: String,
        conflicts: Vec<(String, Vec<DependencyName>)>,
    },
    /// Dependency doesn't contain an export to satisfy the import.
    #[error("dependency '{dependency_name}' doesn't export '{export_name}' to satisfy import '{import_name}'")]
    MissingExport {
        dependency_name: DependencyName,
        export_name: String,
        import_name: String,
    },
    /// An error occurred when building the composition graph
    #[error("an error occurred when preparing dependencies")]
    PrepareError(#[source] anyhow::Error),
    /// An error occurred while encoding the composition graph.
    #[error("failed to encode composition graph: {0}")]
    EncodeError(#[source] anyhow::Error),
}

fn format_conflicts(conflicts: &[(String, Vec<DependencyName>)]) -> String {
    conflicts
        .iter()
        .map(|(import_name, dependency_names)| {
            format!(
                "import '{}' satisfied by dependencies: '{}'",
                import_name,
                dependency_names
                    .iter()
                    .map(|name| name.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

struct Composer<'a, L> {
    graph: CompositionGraph,
    loader: &'a L,
}

impl<'a, L: ComponentSourceLoader> Composer<'a, L> {
    async fn compose(mut self, component: &L::Component) -> Result<Vec<u8>, ComposeError> {
        let source = self
            .loader
            .load_component_source(component)
            .await
            .map_err(ComposeError::PrepareError)?;

        if component.dependencies().len() == 0 {
            return Ok(source);
        }

        let (world_id, instantiation_id) = self
            .register_package(component.id(), None, source)
            .map_err(ComposeError::PrepareError)?;

        let prepared = self.prepare_dependencies(world_id, component).await?;

        let arguments = self
            .build_instantiation_arguments(world_id, prepared)
            .await?;

        for (argument_name, argument) in arguments {
            self.graph
                .set_instantiation_argument(instantiation_id, &argument_name, argument)
                .map_err(|e| ComposeError::PrepareError(e.into()))?;
        }

        self.export_dependents_exports(world_id, instantiation_id)
            .map_err(ComposeError::PrepareError)?;

        self.graph
            .encode(Default::default())
            .map_err(|e| ComposeError::EncodeError(e.into()))
    }

    fn new(loader: &'a L) -> Self {
        Self {
            graph: CompositionGraph::new(),
            loader,
        }
    }

    // This function takes the dependencies specified by the locked component
    // and builds a mapping of import names to dependency infos which contains
    // information about the registered dependency into the composition graph.
    // Additionally if conflicts are detected (where an import name can be
    // satisfied by multiple dependencies) the set of conflicts is returned as
    // an error.
    async fn prepare_dependencies(
        &mut self,
        world_id: WorldId,
        component: &L::Component,
    ) -> Result<IndexMap<String, DependencyInfo>, ComposeError> {
        let imports = self.graph.types()[world_id].imports.clone();

        let import_keys = imports.keys().cloned().collect::<Vec<_>>();

        let mut mappings: BTreeMap<String, Vec<DependencyInfo>> = BTreeMap::new();

        for (dependency_name, dependency) in component.dependencies() {
            let mut matched = Vec::new();

            for import_name in &import_keys {
                if matches_import(dependency_name, import_name)
                    .map_err(ComposeError::PrepareError)?
                {
                    matched.push(import_name.clone());
                }
            }

            if matched.is_empty() {
                return Err(ComposeError::UnmatchedDependencyName {
                    component_id: component.id().to_owned(),
                    dependency_name: dependency_name.clone(),
                });
            }

            let info = self
                .register_dependency(dependency_name.clone(), dependency)
                .await
                .map_err(ComposeError::PrepareError)?;

            // Insert the expanded dependency name into the map detecting duplicates
            for import_name in matched {
                mappings
                    .entry(import_name.to_string())
                    .or_default()
                    .push(info.clone());
            }
        }

        let (conflicts, prepared): (Vec<_>, Vec<_>) =
            mappings.into_iter().partition(|(_, infos)| infos.len() > 1);

        if !conflicts.is_empty() {
            return Err(ComposeError::DependencyConflicts {
                component_id: component.id().to_owned(),
                conflicts: conflicts
                    .into_iter()
                    .map(|(import_name, infos)| {
                        (
                            import_name,
                            infos.into_iter().map(|info| info.manifest_name).collect(),
                        )
                    })
                    .collect(),
            });
        }

        Ok(prepared
            .into_iter()
            .map(|(import_name, mut infos)| {
                assert_eq!(infos.len(), 1);
                (import_name, infos.remove(0))
            })
            .collect())
    }

    // This function takes the set of prepared dependences and builds a mapping
    // of import name to the node in the composition graph used to satisfy the
    // import. If an export could not be found or the export is not comptaible
    // with the type of the import, an error is returned.
    async fn build_instantiation_arguments(
        &mut self,
        world_id: WorldId,
        dependencies: IndexMap<String, DependencyInfo>,
    ) -> Result<IndexMap<String, NodeId>, ComposeError> {
        let mut cache = Default::default();
        let mut checker = SubtypeChecker::new(&mut cache);

        let mut arguments = IndexMap::new();

        for (import_name, dependency_info) in dependencies {
            let (export_name, export_ty) = match dependency_info.export_name {
                Some(export_name) => {
                    let Some(export_ty) = self.graph.types()[dependency_info.world_id]
                        .exports
                        .get(&export_name)
                    else {
                        return Err(ComposeError::MissingExport {
                            dependency_name: dependency_info.manifest_name,
                            export_name,
                            import_name: import_name.clone(),
                        });
                    };

                    (export_name, export_ty)
                }
                None => {
                    let Some(export_ty) = self.graph.types()[dependency_info.world_id]
                        .exports
                        .get(&import_name)
                    else {
                        return Err(ComposeError::MissingExport {
                            dependency_name: dependency_info.manifest_name,
                            export_name: import_name.clone(),
                            import_name: import_name.clone(),
                        });
                    };

                    (import_name.clone(), export_ty)
                }
            };

            let import_ty = self.graph.types()[world_id]
                .imports
                .get(&import_name)
                .unwrap();

            // Ensure that export_ty is a subtype of import_ty
            checker.is_subtype(
                *export_ty,
                self.graph.types(),
                *import_ty,
                self.graph.types(),
            ).with_context(|| {
                format!(
                    "dependency '{dependency_name}' exports '{export_name}' which is not compatible with import '{import_name}'",
                    dependency_name = dependency_info.manifest_name,
                )
            })
            .map_err(ComposeError::PrepareError)?;

            let export_id = self
                .graph
                .alias_instance_export(dependency_info.instantiation_id, &import_name)
                .map_err(|e| ComposeError::PrepareError(e.into()))?;

            assert!(arguments.insert(import_name, export_id).is_none());
        }

        Ok(arguments)
    }

    // This function registers a dependency with the composition graph.
    // Additionally if the locked component specifies that configuration
    // inheritance is disabled, the `deny-all` adapter is applied to the
    // dependency.
    async fn register_dependency(
        &mut self,
        dependency_name: DependencyName,
        dependency: &L::Dependency,
    ) -> anyhow::Result<DependencyInfo> {
        let mut dependency_source = self.loader.load_dependency_source(dependency).await?;

        let package_name = match &dependency_name {
            DependencyName::Package(name) => name.package.to_string(),
            DependencyName::Plain(name) => name.to_string(),
        };

        match dependency.inherit() {
            InheritConfiguration::Some(configurations) => {
                if configurations.is_empty() {
                    // Configuration inheritance is disabled, apply deny_all adapter
                    dependency_source = apply_deny_all_adapter(&package_name, &dependency_source)?;
                } else {
                    panic!("granular configuration inheritance is not yet supported");
                }
            }
            InheritConfiguration::All => {
                // Do nothing, allow configuration to be inherited
            }
        }

        let (world_id, instantiation_id) =
            self.register_package(&package_name, None, dependency_source)?;

        Ok(DependencyInfo {
            manifest_name: dependency_name,
            instantiation_id,
            world_id,
            export_name: dependency.export().clone(),
        })
    }

    fn register_package(
        &mut self,
        name: &str,
        version: Option<&Version>,
        source: impl Into<Vec<u8>>,
    ) -> anyhow::Result<(WorldId, NodeId)> {
        let package = Package::from_bytes(name, version, source, self.graph.types_mut())?;
        let world_id = package.ty();
        let package_id = self.graph.register_package(package)?;
        let instantiation_id = self.graph.instantiate(package_id);

        Ok((world_id, instantiation_id))
    }

    fn export_dependents_exports(
        &mut self,
        world_id: WorldId,
        instantiation_id: NodeId,
    ) -> anyhow::Result<()> {
        // Export all exports from the root component
        for export_name in self.graph.types()[world_id]
            .exports
            .keys()
            .cloned()
            .collect::<Vec<_>>()
        {
            let export_id = self
                .graph
                .alias_instance_export(instantiation_id, &export_name)?;

            self.graph.export(export_id, &export_name)?;
        }

        Ok(())
    }
}

#[derive(Clone)]
struct DependencyInfo {
    // The name of the dependency as it appears in the component's dependencies section.
    // This is used to correlate errors when composing back to what was specified in the
    // manifest.
    manifest_name: DependencyName,
    // The instantiation id for the dependency node.
    instantiation_id: NodeId,
    // The world id for the dependency node.
    world_id: WorldId,
    // Name of optional export to use to satisfy the dependency.
    export_name: Option<String>,
}

fn apply_deny_all_adapter(
    dependency_name: &str,
    dependency_source: &[u8],
) -> anyhow::Result<Vec<u8>> {
    const SPIN_VIRT_DENY_ALL_ADAPTER_BYTES: &[u8] = include_bytes!("../deny_all.wasm");
    let mut graph = CompositionGraph::new();

    let dependency_package =
        Package::from_bytes(dependency_name, None, dependency_source, graph.types_mut())?;

    let dependency_id = graph.register_package(dependency_package)?;

    let deny_adapter_package = Package::from_bytes(
        "spin-virt-deny-all-adapter",
        None,
        SPIN_VIRT_DENY_ALL_ADAPTER_BYTES,
        graph.types_mut(),
    )?;

    let deny_adapter_id = graph.register_package(deny_adapter_package)?;

    match wac_graph::plug(&mut graph, vec![deny_adapter_id], dependency_id) {
        Err(wac_graph::PlugError::NoPlugHappened) => {
            // Dependencies may not depend on any interfaces that the plug fills so we shouldn't error here.
            // Just return the origin `dependency_source` as is.
            return Ok(dependency_source.to_vec());
        }
        Err(other) => {
            anyhow::bail!(
                "failed to plug deny-all adapter into dependency: {:?}",
                other
            );
        }
        Ok(_) => {}
    }

    let bytes = graph.encode(Default::default())?;
    Ok(bytes)
}

enum ImportName {
    Plain(KebabId),
    Package {
        package: String,
        interface: String,
        version: Option<Version>,
    },
}

impl std::str::FromStr for ImportName {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains([':', '/']) {
            let (package, rest) = s
                .split_once('/')
                .with_context(|| format!("invalid import name: {s}"))?;

            let (interface, version) = match rest.split_once('@') {
                Some((interface, version)) => {
                    let version = Version::parse(version)
                        .with_context(|| format!("invalid version in import name: {s}"))?;

                    (interface, Some(version))
                }
                None => (rest, None),
            };

            Ok(Self::Package {
                package: package.to_string(),
                interface: interface.to_string(),
                version,
            })
        } else {
            Ok(Self::Plain(
                s.to_string()
                    .try_into()
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
            ))
        }
    }
}

/// Returns true if the dependency name matches the provided import name string.
fn matches_import(dependency_name: &DependencyName, import_name: &str) -> anyhow::Result<bool> {
    let import_name = import_name.parse::<ImportName>()?;

    match (dependency_name, import_name) {
        (DependencyName::Plain(dependency_name), ImportName::Plain(import_name)) => {
            // Plain names only match if they are equal.
            Ok(dependency_name == &import_name)
        }
        (
            DependencyName::Package(dependency_name),
            ImportName::Package {
                package: import_package,
                interface: import_interface,
                version: import_version,
            },
        ) => {
            if import_package != dependency_name.package.to_string() {
                return Ok(false);
            }

            if let Some(interface) = dependency_name.interface.as_ref() {
                if import_interface != interface.as_ref() {
                    return Ok(false);
                }
            }

            if let Some(version) = dependency_name.version.as_ref() {
                if import_version != Some(version.clone()) {
                    return Ok(false);
                }
            }

            Ok(true)
        }
        (_, _) => {
            // All other combinations of dependency and import names cannot match.
            Ok(false)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_matches_import() {
        for (dep_name, import_names) in [
            ("foo:bar/baz@0.1.0", vec!["foo:bar/baz@0.1.0"]),
            ("foo:bar/baz", vec!["foo:bar/baz@0.1.0", "foo:bar/baz"]),
            ("foo:bar", vec!["foo:bar/baz@0.1.0", "foo:bar/baz"]),
            ("foo:bar@0.1.0", vec!["foo:bar/baz@0.1.0"]),
            ("foo-bar", vec!["foo-bar"]),
        ] {
            let dep_name: DependencyName = dep_name.parse().unwrap();
            for import_name in import_names {
                assert!(matches_import(&dep_name, import_name).unwrap());
            }
        }

        for (dep_name, import_names) in [
            ("foo:bar/baz@0.1.0", vec!["foo:bar/baz"]),
            ("foo:bar/baz", vec!["foo:bar/bub", "foo:bar/bub@0.1.0"]),
            ("foo:bar", vec!["foo:bub/bib"]),
            ("foo:bar@0.1.0", vec!["foo:bar/baz"]),
            ("foo:bar/baz", vec!["foo:bar/baz-bub", "foo-bar"]),
        ] {
            let dep_name: DependencyName = dep_name.parse().unwrap();
            for import_name in import_names {
                assert!(!matches_import(&dep_name, import_name).unwrap());
            }
        }
    }
}
