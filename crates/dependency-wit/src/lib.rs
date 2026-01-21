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
    let loader = WasmLoader::new(app_root.as_ref().to_owned(), None, None).await?;

    let mut package_wits = indexmap::IndexMap::new();
    let mut world_wits = vec![];

    // TODO: figure out what to do if we import two itfs from same dep
    for (dependency_name, dependency) in source {
        // TODO: map `export`
        let (wasm_path, _export) = loader
            .load_component_dependency(dependency_name, dependency)
            .await?;
        let wasm_bytes = tokio::fs::read(&wasm_path).await?;

        let decoded = read_wasm(&wasm_bytes)?; // this erases the package name, hurrah
        let importised = importize(decoded, None, None)?;

        let root_pkg = importised.package(); // the useless root:component one
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

        let output = wit_component::OutputToString::default();
        let mut printer = wit_component::WitPrinter::new(output);
        // TODO: limit to the imported interfaces
        printer.print(importised.resolve(), root_pkg, &[])?;
        world_wits.push(format!("{}", printer.output));
    }

    tokio::fs::create_dir_all(dest_file.as_ref().parent().unwrap()).await?;

    use tokio::io::AsyncWriteExt;
    let mut dest_file = tokio::fs::File::create(dest_file.as_ref()).await?;

    // TODO: ugh!
    dest_file
        .write_all("package root:component;\n\nworld root {\n".as_bytes())
        .await?;
    for world_wit in world_wits {
        let text = world_wit.replace("package root:component;", "");
        let text = text.replace("world root-importized", "");
        let text = text.trim();
        let text = text.trim_matches('{').trim_matches('}');
        let text = text.trim();
        dest_file.write_all(text.trim().as_bytes()).await?;
        dest_file.write_all("\n".as_bytes()).await?;
    }
    dest_file.write_all("}\n\n".as_bytes()).await?;

    for package_wit in package_wits.values() {
        dest_file.write_all(package_wit.as_bytes()).await?;
    }

    dest_file.flush().await?;

    Ok(())
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
