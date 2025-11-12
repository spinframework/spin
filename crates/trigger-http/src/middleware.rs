use anyhow::bail;

#[derive(Default)]
pub(crate) struct HttpMiddlewareComplicator;

#[spin_core::async_trait]
impl spin_factors_executor::Complicator for HttpMiddlewareComplicator {
    async fn complicate(&self, complications: &std::collections::HashMap<String, Vec<spin_factors_executor::Complication>>, component: Vec<u8>) -> anyhow::Result<Vec<u8>> {
        let Some(pipeline) = complications.get("middleware") else {
            return Ok(component);
        };
        if complications.len() > 1 {
            bail!("whoa too complicated, only allowed complication is `middleware`");
        }
        if pipeline.is_empty() {
            return Ok(component);
        }

        let pipey_blobs = pipeline.iter().map(|cm| &cm.data);
        let compo = complicate_the_living_shit_out_of_all_the_things(component, pipey_blobs);

        Ok(compo)
    }
}

fn complicate_the_living_shit_out_of_all_the_things<'a>(depped_source: Vec<u8>, pipey_blobs: impl Iterator<Item = &'a Vec<u8>>) -> Vec<u8> {
    let td = tempfile::tempdir().unwrap();
    let mut pipey_blob_paths = vec![];
    for (pbindex, pb) in pipey_blobs.enumerate() {
        let pb_path = td.path().join(format!("pipey-blob-idx{pbindex}.wasm"));
        std::fs::write(&pb_path, pb).unwrap();
        pipey_blob_paths.push(pb_path);
    }
    let final_path = td.path().join("final-final-v2.wasm");
    std::fs::write(&final_path, depped_source).unwrap();
    pipey_blob_paths.push(final_path);

    let mut config = wasm_compose::config::Config::default();
    config.skip_validation = true;
    config.dependencies = pipey_blob_paths.iter().skip(1).enumerate().map(|(i, p)| (format!("pipe{i}"), wasm_compose::config::Dependency { path: p.clone() })).collect();

    config.instantiations.insert(wasm_compose::composer::ROOT_COMPONENT_NAME.to_owned(), wasm_compose::config::Instantiation {
        dependency: None,
        arguments: [("spin:up/next@3.5.0".to_owned(), wasm_compose::config::InstantiationArg { instance: "pipe0inst".to_owned(), export: Some("wasi:http/handler@0.3.0-rc-2025-09-16".to_owned()) })].into(),
    });

    let last = pipey_blob_paths.iter().skip(1).enumerate().next_back().unwrap().0;

    for (i, _p) in pipey_blob_paths.iter().skip(1).enumerate() {
        let dep_ref = format!("pipe{i}");
        let inst_ref = format!("{dep_ref}inst");
        let instarg = wasm_compose::config::InstantiationArg {
            instance: format!("pipe{}inst", i + 1),
            export: Some("wasi:http/handler@0.3.0-rc-2025-09-16".to_owned()),
        };
        let inst = if i == last {
            wasm_compose::config::Instantiation {
                dependency: Some(dep_ref.clone()),
                arguments: Default::default(),
            }
        } else {
            wasm_compose::config::Instantiation {
                dependency: Some(dep_ref.clone()),
                arguments: [("spin:up/next@3.5.0".to_owned(), instarg)].into_iter().collect(),
            }
        };
        config.instantiations.insert(inst_ref.clone(), inst);
        //curr = inst_ref;
    }

    // eprintln!("{config:?}");

    let composer = wasm_compose::composer::ComponentComposer::new(&pipey_blob_paths[0], &config);

    composer.compose().unwrap()
}
