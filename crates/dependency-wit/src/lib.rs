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
        // TODO: map `export`
        let (wasm_path, _export) = loader
            .load_component_dependency(dependency_name, dependency)
            .await?;
        let wasm_bytes = tokio::fs::read(&wasm_path).await?;

        let decoded = read_wasm(&wasm_bytes)?;
        let impo_world = format!("impo-world{index}");
        let importised = importize(decoded, None, Some(&impo_world))?;

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

        let imports = match dependency_name {
            DependencyName::Plain(_) => all_imports(&importised),
            DependencyName::Package(dependency_package_name) => {
                match dependency_package_name.interface.as_ref() {
                    Some(itf) => one_import(&importised, itf),
                    None => all_imports(&importised),
                }
            }
        };

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

fn one_import(wasm: &DecodedWasm, name: &spin_serde::KebabId) -> Vec<wit_parser::InterfaceId> {
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

// fn all_imports(wasm: &DecodedWasm) -> Vec<(wit_parser::PackageName, String)> {
//     let mut itfs = vec![];

//     for (_pid, pp) in &wasm.resolve().packages {
//         for (_w, wid) in &pp.worlds {
//             if let Some(world) = wasm.resolve().worlds.get(*wid) {
//                 for (_wk, witem) in &world.imports {
//                     if let wit_parser::WorldItem::Interface { id, .. } = witem {
//                         if let Some(itf) = wasm.resolve().interfaces.get(*id) {
//                             if let Some(itfp) = itf.package.as_ref() {
//                                 if let Some(ppp) = wasm.resolve().packages.get(*itfp) {
//                                     if let Some(itfname) = itf.name.as_ref() {
//                                         itfs.push((ppp.name.clone(), itfname.clone()));
//                                     }
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }
//         }
//     }

//     itfs
// }
