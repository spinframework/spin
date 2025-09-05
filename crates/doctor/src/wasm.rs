/// Diagnose missing Wasm sources.
pub mod missing;

use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use spin_common::paths::parent_dir;
use spin_manifest::schema::v2;

use crate::{Diagnosis, Diagnostic, PatientApp};

/// PatientWasm represents a Wasm source to be checked for problems.
#[derive(Debug)]
pub struct PatientWasm {
    app_dir: PathBuf,
    component_id: String,
    component: v2::Component,
}

#[allow(missing_docs)] // WIP
impl PatientWasm {
    fn new(app_dir: impl AsRef<Path>, component_id: String, component: v2::Component) -> Self {
        Self {
            app_dir: app_dir.as_ref().to_owned(),
            component_id,
            component,
        }
    }

    pub fn component_id(&self) -> &str {
        &self.component_id
    }

    pub fn source_path(&self) -> Option<&Path> {
        match &self.component.source(None) {
            v2::ComponentSource::Local(path) => Some(Path::new(path)),
            _ => None,
        }
    }

    pub fn abs_source_path(&self) -> Option<PathBuf> {
        match &self.component.source(None) {
            v2::ComponentSource::Local(path) => {
                // TODO: We probably need a doctor check to see if the path can be expanded!
                // For now, fall back to the literal path.
                let can_path = Path::new(path)
                    .canonicalize()
                    .unwrap_or(self.app_dir.join(path));
                Some(can_path)
            }
            _ => None,
        }
    }

    pub fn has_build(&self) -> bool {
        self.component.build(None).is_some()
    }
}

/// WasmDiagnostic helps implement [`Diagnostic`] for Wasm source problems.
#[async_trait]
pub trait WasmDiagnostic {
    /// A [`Diagnosis`] representing the problem(s) this can detect.
    type Diagnosis: Diagnosis;

    /// Check the given [`PatientWasm`], returning any problem(s) found.
    async fn diagnose_wasm(
        &self,
        app: &PatientApp,
        wasm: PatientWasm,
    ) -> Result<Vec<Self::Diagnosis>>;
}

#[async_trait]
impl<T: WasmDiagnostic + Send + Sync> Diagnostic for T {
    type Diagnosis = <Self as WasmDiagnostic>::Diagnosis;

    async fn diagnose(&self, patient: &PatientApp) -> Result<Vec<Self::Diagnosis>> {
        let manifest_str = patient.manifest_doc.to_string();
        let manifest = spin_manifest::manifest_from_str(&manifest_str)?;
        let app_dir = parent_dir(&patient.manifest_path)?;
        let mut diagnoses = vec![];
        for (component_id, component) in manifest.components {
            let wasm = PatientWasm::new(&app_dir, component_id.to_string(), component);
            diagnoses.extend(self.diagnose_wasm(patient, wasm).await?);
        }
        Ok(diagnoses)
    }
}
