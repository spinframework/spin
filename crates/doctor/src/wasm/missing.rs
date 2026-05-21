use std::process::Command;

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use spin_common::ui::quoted_path;

use crate::{Diagnosis, PatientApp, Treatment};

use super::{PatientWasm, WasmDiagnostic};

/// WasmMissingDiagnostic detects missing Wasm sources.
#[derive(Default)]
pub struct WasmMissingDiagnostic;

#[async_trait]
impl WasmDiagnostic for WasmMissingDiagnostic {
    type Diagnosis = WasmMissing;

    async fn diagnose_wasm(
        &self,
        _app: &PatientApp,
        wasm: PatientWasm,
    ) -> anyhow::Result<Vec<Self::Diagnosis>> {
        if let Some(abs_path) = wasm.abs_source_path()
            && !abs_path.exists()
        {
            return Ok(vec![WasmMissing(wasm)]);
        }
        Ok(vec![])
    }
}

/// WasmMissing represents a missing Wasm source.
#[derive(Debug)]
pub struct WasmMissing(PatientWasm);

impl WasmMissing {
    fn build_cmd(&self, patient: &PatientApp) -> Result<Command> {
        let spin_bin = std::env::current_exe().context("Couldn't find spin executable")?;
        let mut cmd = Command::new(spin_bin);
        cmd.arg("build")
            .arg("-f")
            .arg(&patient.manifest_path)
            .arg("--component-id")
            .arg(self.0.component_id());
        Ok(cmd)
    }
}

impl Diagnosis for WasmMissing {
    fn description(&self) -> String {
        let id = self.0.component_id();
        let Some(rel_path) = self.0.source_path() else {
            unreachable!("unsupported source");
        };
        format!(
            "Component {id:?} source {} is missing",
            quoted_path(rel_path)
        )
    }

    fn treatment(&self) -> Option<&dyn Treatment> {
        self.0.has_build().then_some(self)
    }
}

#[async_trait]
impl Treatment for WasmMissing {
    fn summary(&self) -> String {
        "Run `spin build`".into()
    }

    async fn dry_run(&self, patient: &PatientApp) -> anyhow::Result<String> {
        let args = self
            .build_cmd(patient)?
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(format!("Run `spin {args}`"))
    }

    async fn treat(&self, patient: &mut PatientApp) -> anyhow::Result<()> {
        let mut cmd = self.build_cmd(patient)?;
        let status = cmd.status()?;
        ensure!(status.success(), "Build command {cmd:?} failed: {status:?}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{TestPatient, assert_single_diagnosis};

    use super::*;

    const MINIMUM_VIABLE_MANIFEST: &str = r#"
            spin_manifest_version = "1"
            name = "wasm-missing-test"
            version = "0.0.0"
            trigger = { type = "test" }
            [[component]]
            id = "missing-source"
            source = "does-not-exist.wasm"
            trigger = {}
        "#;

    #[tokio::test]
    async fn test_without_build() {
        let patient = TestPatient::from_toml_str(MINIMUM_VIABLE_MANIFEST);
        let diag = assert_single_diagnosis::<WasmMissingDiagnostic>(&patient).await;
        assert!(diag.treatment().is_none());
    }

    #[tokio::test]
    async fn test_with_build() {
        let manifest = format!("{MINIMUM_VIABLE_MANIFEST}\nbuild.command = 'true'");
        let patient = TestPatient::from_toml_str(manifest);
        let diag = assert_single_diagnosis::<WasmMissingDiagnostic>(&patient).await;
        assert!(diag.treatment().is_some());
        assert!(
            diag.build_cmd(&patient)
                .unwrap()
                .get_args()
                .any(|arg| arg == "missing-source")
        );
    }
}
