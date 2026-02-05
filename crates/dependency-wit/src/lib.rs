use std::path::Path;

use anyhow::Context;
use spin_loader::WasmLoader;
use spin_manifest::schema::v2::ComponentDependency;
use spin_serde::DependencyName;
use wit_component::DecodedWasm;

pub async fn extract_wits_into(
    source: impl Iterator<Item = (&DependencyName, &ComponentDependency)>,
    app_root: impl AsRef<Path>,
    dest_file: impl AsRef<Path>,
) -> anyhow::Result<()> {
    let wit_text = extract_wits(source, app_root).await?;

    tokio::fs::create_dir_all(dest_file.as_ref().parent().unwrap()).await?;
    tokio::fs::write(dest_file, wit_text.as_bytes()).await?;

    Ok(())
}

pub async fn extract_wits(
    source: impl Iterator<Item = (&DependencyName, &ComponentDependency)>,
    app_root: impl AsRef<Path>,
) -> anyhow::Result<String> {
    let loader = WasmLoader::new(app_root.as_ref().to_owned(), None, None).await?;

    let mut package_wits = indexmap::IndexMap::new();

    let mut aggregating_resolve = wit_parser::Resolve::default();
    let aggregating_pkg_id =
        aggregating_resolve.push_str("dummy.wit", "package root:component;\n\nworld root {}")?;
    let aggregating_world_id =
        aggregating_resolve.select_world(&[aggregating_pkg_id], Some("root"))?;

    // TODO: figure out what to do if we import two itfs from same dep
    for (index, (dependency_name, dependency)) in source.enumerate() {
        let import_name = match dependency_name {
            DependencyName::Plain(_) => None,
            DependencyName::Package(dependency_package_name) => {
                dependency_package_name.interface.as_ref()
                // match dependency_package_name.interface.as_ref() {
                //     Some(itf) => Some(itf),
                //     None => None,
                // }
            }
        };

        let (wasm_path, export) = loader
            .load_component_dependency(dependency_name, dependency)
            .await?;
        let wasm_bytes = tokio::fs::read(&wasm_path).await?;

        let decoded = read_wasm(&wasm_bytes)?;
        let decoded = match export {
            None => decoded,
            Some(export) => munge_aliased_export(decoded, &export, dependency_name)?,
        };
        let impo_world = format!("impo-world{index}");
        let importised = importize(decoded, None, Some(&impo_world))?;

        let imports = match import_name {
            None => all_imports(&importised),
            Some(itf) => one_import(&importised, itf.as_ref()),
        };

        // Capture WITs for all packages used in the importised thing.
        // Things like WASI packages may be depended on my multiple packages
        // so we index on the package name to avoid emitting them twice.

        let root_pkg = importised.package();
        let useful_pkgs = importised
            .resolve()
            .packages
            .iter()
            .map(|p| p.0)
            .filter(|pid| *pid != root_pkg)
            .collect::<Vec<_>>();

        for p in &useful_pkgs {
            let pkg_name = importised.resolve().packages.get(*p).unwrap().name.clone();
            let output = wit_component::OutputToString::default();
            let mut printer = wit_component::WitPrinter::new(output);
            printer.print_package(importised.resolve(), *p, false)?;
            package_wits.insert(pkg_name, printer.output.to_string());
        }

        // Now add the imports to the aggregating component import world

        let remap = aggregating_resolve.merge(importised.resolve().clone())?;
        for iid in imports {
            let mapped_iid = remap.map_interface(iid, None)?;
            let wk = wit_parser::WorldKey::Interface(mapped_iid);
            let world_item = wit_parser::WorldItem::Interface {
                id: mapped_iid,
                stability: wit_parser::Stability::Unknown,
            };
            aggregating_resolve
                .worlds
                .get_mut(aggregating_world_id)
                .unwrap()
                .imports
                .insert(wk, world_item);
        }
    }

    // Text for the root package and world(s)
    let world_output = wit_component::OutputToString::default();
    let mut world_printer = wit_component::WitPrinter::new(world_output);
    world_printer.print(&aggregating_resolve, aggregating_pkg_id, &[])?;

    let mut buf = String::new();

    // Print the root package and the world(s) with the imports
    buf.push_str(&world_printer.output.to_string());

    // Print each package
    for package_wit in package_wits.values() {
        buf.push_str(package_wit);
    }

    Ok(buf)
}

fn munge_aliased_export(
    decoded: DecodedWasm,
    export: &str,
    new_name: &DependencyName,
) -> anyhow::Result<DecodedWasm> {
    // TODO: I am not sure how `export` is meant to work if you are
    // depping on a package rather than an itf

    let export_qname = spin_serde::DependencyPackageName::try_from(export.to_string())?;
    let Some(export_itf_name) = export_qname.interface.as_ref() else {
        anyhow::bail!("the export name should be a qualified interface name - missing interface");
    };
    let export_pkg_name = wit_parser::PackageName {
        namespace: export_qname.package.namespace().to_string(),
        name: export_qname.package.name().to_string(),
        version: export_qname.version,
    };

    let DependencyName::Package(new_name) = new_name else {
        anyhow::bail!("the dependency name should be a qualified interface name - not qualified");
    };
    let Some(new_itf_name) = new_name.interface.as_ref() else {
        anyhow::bail!(
            "the dependency name should be a qualified interface name - missing interface"
        );
    };
    let new_pkg_name = wit_parser::PackageName {
        namespace: new_name.package.namespace().to_string(),
        name: new_name.package.name().to_string(),
        version: new_name.version.clone(),
    };

    let (mut resolve, decode_id) = match decoded {
        DecodedWasm::WitPackage(resolve, id) => (resolve, WorldOrPackageId::Package(id)),
        DecodedWasm::Component(resolve, id) => (resolve, WorldOrPackageId::World(id)),
    };

    // Two scenarios:
    // 1. The new name is in a package that is already in the Resolve
    //    1a. The package already contains an interface with the right name
    //    1b. The package does not already contain an interface with the right name
    // 2. The new name is in a package that is NOT already in the Resolve

    let existing_pkg = resolve
        .packages
        .iter()
        .find(|(_pkg_id, pkg)| pkg.name == new_pkg_name);

    // We address the first level by creating the new-name package if it doesn't exist
    let (inserting_into_pkg_id, inserting_into_pkg) = match existing_pkg {
        Some(tup) => tup,
        None => {
            // insert the needed package
            let package_wit = format!("package {new_pkg_name};");
            let pkg_id = resolve
                .push_str(std::env::current_dir().unwrap(), &package_wit)
                .context("failed at setting up fake pkg")?;
            let pkg = resolve.packages.get(pkg_id).unwrap();
            (pkg_id, pkg)
        }
    };

    // Second level asks if the package already contains the interface
    let existing_itf = inserting_into_pkg.interfaces.get(new_itf_name.as_ref());
    if existing_itf.is_some() {
        // no rename is needed, but we might need to do some extra work to make sure
        // that the export, rather than the import, gets included in the aggregated world
        return Ok(decode_id.make_decoded_wasm(resolve));
    }

    // It does not: we need to slurp the EXPORTED itf into the `inserting_into`
    // package under the NEW (importing) interface name
    let Some(export_pkg_id) = resolve.package_names.get(&export_pkg_name) else {
        anyhow::bail!("export is from a package that doesn't exist");
    };
    let Some(export_pkg) = resolve.packages.get(*export_pkg_id) else {
        anyhow::bail!("export pkg id doesn't point to a package wtf");
    };
    let Some(export_itf_id) = export_pkg.interfaces.get(export_itf_name.as_ref()) else {
        anyhow::bail!("export pkg doesn't contain export itf");
    };
    let Some(export_itf) = resolve.interfaces.get(*export_itf_id) else {
        anyhow::bail!("export pkg doesn't contain export itf part 2");
    };

    let mut export_itf = export_itf.clone();
    export_itf.package = Some(inserting_into_pkg_id);
    export_itf.name = Some(new_itf_name.to_string());
    let export_itf_id_2 = resolve.interfaces.alloc(export_itf);

    // OKAY TIME TO ADD THIS UNDER THE WRONG NAME TO THE THINGY
    // oh man there is some nonsense about worlds as well
    let inserting_into_pkg_mut = resolve.packages.get_mut(inserting_into_pkg_id).unwrap(); // SHENANIGANS to get around a "mutable borrow at the same time as immutable borrow" woe
    inserting_into_pkg_mut
        .interfaces
        .insert(new_itf_name.to_string(), export_itf_id_2);

    let thingy = decode_id.make_decoded_wasm(resolve);

    Ok(thingy)
}

enum WorldOrPackageId {
    Package(wit_parser::PackageId),
    World(wit_parser::WorldId),
}

impl WorldOrPackageId {
    pub fn make_decoded_wasm(&self, resolve: wit_parser::Resolve) -> DecodedWasm {
        match self {
            Self::Package(id) => DecodedWasm::WitPackage(resolve, *id),
            Self::World(id) => DecodedWasm::Component(resolve, *id),
        }
    }
}

fn all_imports(wasm: &DecodedWasm) -> Vec<wit_parser::InterfaceId> {
    wasm.resolve()
        .worlds
        .iter()
        .flat_map(|(_wid, w)| w.imports.values())
        .flat_map(as_interface)
        .collect()
}

fn as_interface(wi: &wit_parser::WorldItem) -> Option<wit_parser::InterfaceId> {
    match wi {
        wit_parser::WorldItem::Interface { id, .. } => Some(*id),
        _ => None,
    }
}

fn one_import(wasm: &DecodedWasm, name: &str) -> Vec<wit_parser::InterfaceId> {
    let id = wasm
        .resolve()
        .interfaces
        .iter()
        .find(|i| i.1.name == Some(name.to_string()))
        .map(|t| t.0);
    id.into_iter().collect()
}

fn read_wasm(wasm_bytes: &[u8]) -> anyhow::Result<DecodedWasm> {
    if wasmparser::Parser::is_component(wasm_bytes) {
        wit_component::decode(wasm_bytes)
    } else {
        let (wasm, bindgen) = wit_component::metadata::decode(wasm_bytes)?;
        if wasm.is_none() {
            anyhow::bail!(
                "input is a core wasm module with no `component-type*` \
                    custom sections meaning that there is not WIT information; \
                    is the information not embedded or is this supposed \
                    to be a component?"
            )
        }
        Ok(DecodedWasm::Component(bindgen.resolve, bindgen.world))
    }
}

fn importize(
    decoded: DecodedWasm,
    world: Option<&str>,
    out_world_name: Option<&String>,
) -> anyhow::Result<DecodedWasm> {
    let (mut resolve, world_id) = match (decoded, world) {
        (DecodedWasm::Component(resolve, world), None) => (resolve, world),
        (DecodedWasm::Component(..), Some(_)) => {
            anyhow::bail!(
                "the `--importize-world` flag is not compatible with a \
                    component input, use `--importize` instead"
            );
        }
        (DecodedWasm::WitPackage(resolve, id), world) => {
            let world = resolve.select_world(&[id], world)?;
            (resolve, world)
        }
    };

    resolve
        .importize(world_id, out_world_name.cloned())
        .context("failed to move world exports to imports")?;

    Ok(DecodedWasm::Component(resolve, world_id))
}

#[cfg(test)]
mod test {
    use super::*;

    fn parse_wit(wit: &str) -> anyhow::Result<wit_parser::Resolve> {
        let mut resolve = wit_parser::Resolve::new();
        resolve.push_str("dummy.wit", wit)?;
        Ok(resolve)
    }

    fn generate_dummy_component(wit: &str, world: &str) -> Vec<u8> {
        let mut resolve = wit_parser::Resolve::default();
        let package_id = resolve.push_str("test", wit).expect("should parse WIT");
        let world_id = resolve
            .select_world(&[package_id], Some(world))
            .expect("should select world");

        let mut wasm = wit_component::dummy_module(
            &resolve,
            world_id,
            wit_parser::ManglingAndAbi::Legacy(wit_parser::LiftLowerAbi::Sync),
        );
        wit_component::embed_component_metadata(
            &mut wasm,
            &resolve,
            world_id,
            wit_component::StringEncoding::UTF8,
        )
        .expect("should embed component metadata");

        let mut encoder = wit_component::ComponentEncoder::default()
            .validate(true)
            .module(&wasm)
            .expect("should set module");
        encoder.encode().expect("should encode component")
    }

    #[tokio::test]
    async fn if_no_dependencies_then_empty_valid_wit() -> anyhow::Result<()> {
        let wit = extract_wits(std::iter::empty(), ".").await?;

        let resolve = parse_wit(&wit).expect("should have emitted valid WIT");

        assert_eq!(1, resolve.packages.len());
        assert_eq!(
            "root:component",
            resolve.packages.iter().next().unwrap().1.name.to_string()
        );

        assert_eq!(0, resolve.interfaces.len());

        assert_eq!(1, resolve.worlds.len());

        let world = resolve.worlds.iter().next().unwrap().1;
        assert_eq!("root", world.name);
        assert_eq!(0, world.imports.len());

        Ok(())
    }

    #[tokio::test]
    async fn single_dep_wit_extracted() -> anyhow::Result<()> {
        let tempdir = tempfile::TempDir::new()?;
        let dep_file = tempdir.path().join("regex.wasm");

        let dep_wit = "package my:regex@1.0.0;\n\ninterface regex {\n  matches: func(s: string) -> bool;\n}\nworld matcher {\n  export regex;\n}";
        let dep_wasm = generate_dummy_component(dep_wit, "matcher");
        tokio::fs::write(&dep_file, &dep_wasm).await?;

        let dep_name =
            DependencyName::Package("my:regex/regex@1.0.0".to_string().try_into().unwrap());
        let dep_src = ComponentDependency::Local {
            path: dep_file,
            export: None,
        };
        let deps = std::iter::once((&dep_name, &dep_src));

        let wit = extract_wits(deps, ".").await?;

        let resolve = parse_wit(&wit).expect("should have emitted valid WIT");

        assert_eq!(2, resolve.packages.len()); // root:component and my:regex
        let (_rc_pkg_id, rc_pkg) = resolve
            .packages
            .iter()
            .find(|(_, p)| p.name.to_string() == "root:component")
            .expect("should have had `root:component`");
        let (_mr_pkg_id, _mr_pkg) = resolve
            .packages
            .iter()
            .find(|(_, p)| p.name.to_string() == "my:regex@1.0.0")
            .expect("should have had `my:regex`");

        assert_eq!(1, resolve.interfaces.len());
        assert_eq!(
            "regex",
            resolve
                .interfaces
                .iter()
                .next()
                .unwrap()
                .1
                .name
                .as_ref()
                .unwrap()
        );
        let regex_itf_id = resolve.interfaces.iter().next().unwrap().0;

        assert_eq!(2, rc_pkg.worlds.len()); // root and synthetic "impo*" wart
        let root_world_id = rc_pkg
            .worlds
            .iter()
            .find(|w| w.0 == "root")
            .expect("should have had `root` world")
            .1;

        let world = resolve.worlds.get(*root_world_id).unwrap();
        assert_eq!(1, world.imports.len());
        let expected_import = wit_parser::WorldItem::Interface {
            id: regex_itf_id,
            stability: wit_parser::Stability::Unknown,
        };
        let import = world.imports.values().next().unwrap();
        assert_eq!(&expected_import, import);

        Ok(())
    }
}
