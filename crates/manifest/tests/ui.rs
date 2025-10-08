use std::path::Path;

use spin_manifest::normalize::normalize_manifest;
use ui_testing::{Failed, UiTestsRunner};

fn main() -> anyhow::Result<()> {
    let mut runner = UiTestsRunner::default();
    for entry in glob::glob("tests/ui/**/*.toml")? {
        let path = entry?.canonicalize()?;
        let test_name = format!("ui::{}", path.file_stem().unwrap().to_string_lossy());
        let snapshot_path = path.with_extension("json");
        runner.add_test(test_name, snapshot_path, move |_| run_test(&path));
    }
    for entry in glob::glob("tests/ui/v1/*.toml")? {
        let path = entry?.canonicalize()?;
        let test_name = format!(
            "ui::v1_to_v2::{}",
            path.file_stem().unwrap().to_string_lossy()
        );
        let snapshot_path = path.with_extension("toml.v2");
        runner.add_test(test_name, snapshot_path, move |_| run_v1_to_v2_test(&path));
    }
    runner.add_test(
        "ui::normalization".into(),
        "tests/ui/normalization.toml.norm",
        |_| run_normalization_test("tests/ui/normalization.toml"),
    );

    runner.run_tests()
}

fn run_test(input: &Path) -> Result<String, Failed> {
    let manifest = spin_manifest::manifest_from_file(input)?;
    Ok(serde_json::to_string_pretty(&manifest).expect("serialization should work"))
}

fn run_v1_to_v2_test(input: &Path) -> Result<String, Failed> {
    let manifest_str =
        std::fs::read_to_string(input).unwrap_or_else(|err| panic!("reading {input:?}: {err:?}"));
    let v1_manifest = toml::from_str(&manifest_str)
        .unwrap_or_else(|err| panic!("parsing v1 manifest {input:?}: {err:?}"));
    let v2_manifest = spin_manifest::compat::v1_to_v2_app(v1_manifest)?;
    Ok(toml::to_string(&v2_manifest).expect("serialization should work"))
}

fn run_normalization_test(input: impl AsRef<Path>) -> Result<String, Failed> {
    let mut manifest = spin_manifest::manifest_from_file(input)?;
    normalize_manifest(&mut manifest)?;
    Ok(toml::to_string(&manifest).expect("serialization should work"))
}
