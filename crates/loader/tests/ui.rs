use std::{ffi::OsStr, path::Path};

use futures::Future;
use spin_loader::cache::Cache;
use ui_testing::{Failed, Normalizer, UiTestsRunner};

fn main() -> anyhow::Result<()> {
    // Insert dummy wasm into cache to avoid network traffic
    block_on(async {
        let cache = Cache::new(None).await.expect("Cache::new to work");
        cache
            .write_wasm(
                b"",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .await
            .expect("write_wasm should work");
    });

    let mut runner = UiTestsRunner::default();
    for entry in std::fs::read_dir("tests/ui")? {
        let entry_path = entry?.path();
        let mut path = entry_path.canonicalize()?;

        // */spin.toml are tests too!
        if path.is_dir() {
            path = path.join("spin.toml");
            if !path.exists() {
                continue;
            }
        } else if path.extension() != Some(OsStr::new("toml")) {
            continue;
        }

        let test_name = format!("ui::{}", entry_path.file_stem().unwrap().to_string_lossy());
        let snapshot_path = path.with_extension("lock");
        runner.add_test(test_name, snapshot_path, move |n| run_test(&path, n));
    }
    runner.run_tests()
}

fn run_test(input: &Path, normalizer: &mut Normalizer) -> Result<String, Failed> {
    let temp_dir = tempfile::tempdir().unwrap();
    let temp_path = temp_dir.path().canonicalize().unwrap();
    let files_mount_root = temp_path.join("assets");
    normalizer.replace_path(temp_path, "<temp-dir>");

    block_on(async {
        let locked = spin_loader::from_file(
            input,
            spin_loader::FilesMountStrategy::Copy(files_mount_root),
            None,
            None,
        )
        .await
        .map_err(|err| format!("{err:?}"))?;
        Ok(serde_json::to_string_pretty(&locked).expect("serialization should work"))
    })
}

fn block_on<T>(fut: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime builder should work")
        .block_on(fut)
}
