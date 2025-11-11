use anyhow::{bail, Context};
use wasm_compose::{
    composer::{ComponentComposer, ROOT_COMPONENT_NAME},
    config::{Config, Dependency, Instantiation, InstantiationArg},
};

use std::collections::HashMap;
use std::path::PathBuf;

use spin_factors_executor::{Complication, ComplicationData, Complicator};

#[derive(Default)]
pub(crate) struct HttpMiddlewareComplicator;

#[spin_core::async_trait]
impl Complicator for HttpMiddlewareComplicator {
    async fn complicate(
        &self,
        complications: &HashMap<String, Vec<Complication>>,
        component: Vec<u8>,
    ) -> anyhow::Result<Vec<u8>> {
        let Some(middlewares) = complications.get("middleware") else {
            return Ok(component);
        };
        if complications.len() > 1 {
            bail!("the HTTP trigger's only allowed complication is `middleware`");
        }
        if middlewares.is_empty() {
            return Ok(component);
        }

        let middleware_blobs = middlewares.iter().map(|cm| &cm.data);
        compose_middlewares(component, middleware_blobs).await
    }
}

async fn compose_middlewares<'a>(
    primary: Vec<u8>,
    middleware_blobs: impl Iterator<Item = &'a ComplicationData>,
) -> anyhow::Result<Vec<u8>> {
    const MW_NEXT_INBOUND: &str = "wasi:http/handler@0.3.0-rc-2026-01-06";
    const MW_NEXT_OUTBOUND: &str = "wasi:http/handler@0.3.0-rc-2026-01-06";

    // TODO: I wonder if we can shorten/simplify this (and avoid all the tempfile
    // crap) with a sequence of `wac_graph::plug`s now inbound and outbound are the same?

    // `wasm-tools compose` relies on the components it's composing being in
    // files, so write all any in-memory blobs to a temp dir.
    let temp_dir = tempfile::tempdir().context("creating working dir for middleware")?;
    let temp_path = temp_dir.path();

    let mut mw_blob_paths = write_blobs_to(primary, middleware_blobs, temp_path).await?;

    // We will use the first item in the chain as the composition root.
    // This means it does not get mapped in the list of dependencies, but
    // is provided directly to the ComponentComposer. So we set it
    // aside for now.
    let first = mw_blob_paths.remove(0);
    let last_index = mw_blob_paths.len() - 1; // points to the end of the composition chain (which is the primary)

    // All blobs except the (already set aside) root are mapped in via dependencies
    let dependencies = mw_blob_paths
        .iter()
        .enumerate()
        .map(|(index, mw_path)| {
            (
                dep_ref(index),
                Dependency {
                    path: mw_path.clone(),
                },
            )
        })
        .collect();

    let mut config = Config {
        skip_validation: true,
        dependencies,
        ..Default::default()
    };

    // The composition root hooks up to the start of the (remaining)
    // pipeline (which we will soon create as inst ref 0).
    config.instantiations.insert(
        ROOT_COMPONENT_NAME.to_owned(),
        Instantiation {
            dependency: None,
            arguments: [(
                MW_NEXT_OUTBOUND.to_owned(),
                InstantiationArg {
                    instance: inst_ref(0),
                    export: Some(MW_NEXT_INBOUND.to_owned()),
                },
            )]
            .into(),
        },
    );

    // Go through the remaining items of of the pipeline except for the last.
    // For each, create an instantiation (named by index) of the
    // middleware at hand with its 'next' import hooked up to the next instance's (named by inst ref) handler export.
    //
    // The range is deliberately non-inclusive: the last item needs different
    // handling, because we do *not* want to fulfil its dependencies.
    for index in 0..last_index {
        let next_inst_ref = InstantiationArg {
            instance: inst_ref(index + 1),
            export: Some(MW_NEXT_INBOUND.to_owned()),
        };
        let inst = Instantiation {
            dependency: Some(dep_ref(index)),
            arguments: [(MW_NEXT_OUTBOUND.to_owned(), next_inst_ref)]
                .into_iter()
                .collect(),
        };
        config.instantiations.insert(inst_ref(index), inst);
    }

    // Create an instantiation of the primary
    // (which is the last thing in the pipeline) with its imports open.
    let primary = Instantiation {
        dependency: Some(dep_ref(last_index)),
        arguments: Default::default(),
    };
    config.instantiations.insert(inst_ref(last_index), primary);

    // Run the composition, using the previously set aside first item the composition root.
    let composer = ComponentComposer::new(&first, &config);

    composer.compose()
}

/// The return vector has the written-out paths in chain order:
/// the middlewares in order, followed by the primary. This matters!
async fn write_blobs_to(
    primary: Vec<u8>,
    middleware_blobs: impl Iterator<Item = &ComplicationData>,
    temp_path: &std::path::Path,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut mw_blob_paths = vec![];

    for (mw_index, mw_blob) in middleware_blobs.enumerate() {
        let mw_blob_path = match mw_blob {
            ComplicationData::InMemory(data) => {
                let mw_blob_path = temp_path.join(format!("middleware-blob-idx{mw_index}.wasm"));
                tokio::fs::write(&mw_blob_path, data)
                    .await
                    .context("writing middleware blob to temp dir")?;
                mw_blob_path
            }
            ComplicationData::OnDisk(path) => path.clone(),
        };
        mw_blob_paths.push(mw_blob_path);
    }

    let primary_path = temp_path.join("primary.wasm");
    tokio::fs::write(&primary_path, primary)
        .await
        .context("writing component to temp dir for middleware composition")?;
    mw_blob_paths.push(primary_path);

    Ok(mw_blob_paths)
}

/// The identifier in the composition graph for the index'th item
/// in the 'middlewares + primary' list. The config maps these
/// identifiers to physical files.
fn dep_ref(index: usize) -> String {
    format!("mw{index}")
}

/// The identifier in the composition graph for the instantiation of the
/// index'th item in the 'middlewares + primary' list. This is used when
/// hooking up the imports of one instantiation to the exports of another.
fn inst_ref(index: usize) -> String {
    format!("mw{index}inst")
}
