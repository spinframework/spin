//! Spin doctor: check and automatically fix problems with Spin apps.
#![deny(missing_docs)]

use std::{collections::VecDeque, fmt::Debug, fs, path::PathBuf};

use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use spin_common::ui::quoted_path;
use toml_edit::DocumentMut;

/// Diagnoses for app manifest format problems.
pub mod manifest;
/// Diagnose for Rust-specific problems.
pub mod rustlang;
/// Test helpers.
pub(crate) mod test;
/// Diagnoses for Wasm source problems.
pub mod wasm;

/// Configuration for an app to be checked for problems.
pub struct Checkup {
    patient: PatientApp,
    diagnostics: VecDeque<Box<dyn BoxingDiagnostic>>,
    unprocessed_diagnoses: VecDeque<Box<dyn Diagnosis>>,
}

impl Checkup {
    /// Return a new checkup for the app manifest at the given path.
    pub fn new(manifest_path: impl Into<PathBuf>) -> Result<Self> {
        let patient = PatientApp::new(manifest_path)?;
        let mut checkup = Self {
            patient,
            diagnostics: Default::default(),
            unprocessed_diagnoses: Default::default(),
        };
        checkup
            .add_diagnostic::<manifest::upgrade::UpgradeDiagnostic>()
            .add_diagnostic::<manifest::version::VersionDiagnostic>()
            .add_diagnostic::<manifest::trigger::TriggerDiagnostic>()
            .add_diagnostic::<rustlang::target::TargetDiagnostic>() // Do toolchain checks _before_ build check
            .add_diagnostic::<wasm::missing::WasmMissingDiagnostic>();
        Ok(checkup)
    }

    /// Returns the [`PatientApp`] being checked.
    pub fn patient(&self) -> &PatientApp {
        &self.patient
    }

    /// Add a detectable problem to this checkup.
    pub fn add_diagnostic<D: Diagnostic + Default + 'static>(&mut self) -> &mut Self {
        self.diagnostics.push_back(Box::<D>::default());
        self
    }

    /// Returns the next detected problem.
    pub async fn next_diagnosis(&mut self) -> Result<Option<PatientDiagnosis<'_>>> {
        while self.unprocessed_diagnoses.is_empty() {
            let Some(diagnostic) = self.diagnostics.pop_front() else {
                return Ok(None);
            };
            self.unprocessed_diagnoses = diagnostic
                .diagnose_boxed(&self.patient)
                .await
                .unwrap_or_else(|err| {
                    tracing::debug!("Diagnose failed: {err:?}");
                    vec![]
                })
                .into()
        }
        Ok(Some(PatientDiagnosis {
            patient: &mut self.patient,
            diagnosis: self.unprocessed_diagnoses.pop_front().unwrap(),
        }))
    }
}

/// An app "patient" to be checked for problems.
#[derive(Clone)]
pub struct PatientApp {
    /// Path to an app manifest file.
    pub manifest_path: PathBuf,
    /// Parsed app manifest TOML document.
    pub manifest_doc: DocumentMut,
}

impl PatientApp {
    fn new(manifest_path: impl Into<PathBuf>) -> Result<Self> {
        let path = manifest_path.into();
        ensure!(
            path.is_file(),
            "No Spin app manifest file found at {}",
            quoted_path(&path),
        );

        let contents = fs::read_to_string(&path).with_context(|| {
            format!(
                "Couldn't read Spin app manifest file at {}",
                quoted_path(&path)
            )
        })?;

        let manifest_doc: DocumentMut = contents.parse().with_context(|| {
            format!(
                "Couldn't parse manifest file at {} as valid TOML",
                quoted_path(&path)
            )
        })?;

        Ok(Self {
            manifest_path: path,
            manifest_doc,
        })
    }
}

/// A PatientDiagnosis bundles a [`Diagnosis`] with its (borrowed) [`PatientApp`].
pub struct PatientDiagnosis<'a> {
    /// The diagnosis
    pub diagnosis: Box<dyn Diagnosis>,
    /// A reference to the patient this diagnosis applies to
    pub patient: &'a mut PatientApp,
}

/// The Diagnose trait implements the detection of a particular Spin app problem.
#[async_trait]
pub trait Diagnostic: Send + Sync {
    /// A [`Diagnosis`] representing the problem(s) this can detect.
    type Diagnosis: Diagnosis;

    /// Check the given [`PatientApp`], returning any problem(s) found.
    ///
    /// If multiple _independently addressable_ problems are found, this may
    /// return multiple instances. If two "logically separate" problems would
    /// have the same fix, they should be represented with the same instance.
    async fn diagnose(&self, patient: &PatientApp) -> Result<Vec<Self::Diagnosis>>;
}

/// The Diagnosis trait represents a detected problem with a Spin app.
pub trait Diagnosis: Debug + Send + Sync + 'static {
    /// Return a human-friendly description of this problem.
    fn description(&self) -> String;

    /// Return true if this problem is "critical", i.e. if the app's
    /// configuration or environment is invalid. Return false for
    /// "non-critical" problems like deprecations.
    fn is_critical(&self) -> bool {
        true
    }

    /// Return a [`Treatment`] that can (potentially) fix this problem, or
    /// None if there is no automatic fix.
    fn treatment(&self) -> Option<&dyn Treatment> {
        None
    }
}

/// The Treatment trait represents a (potential) fix for a detected problem.
#[async_trait]
pub trait Treatment: Sync {
    /// Return a short (single line) description of what this fix will do, as
    /// an imperative, e.g. "Upgrade the library".
    fn summary(&self) -> String;

    /// Return a detailed description of what this fix will do, such as a file
    /// diff or list of commands to be executed.
    ///
    /// May return `Err(DryRunNotSupported.into())` if no such description is
    /// available, which is the default implementation.
    async fn dry_run(&self, patient: &PatientApp) -> Result<String> {
        let _ = patient;
        Err(DryRunNotSupported.into())
    }

    /// Attempt to fix this problem. Return Ok only if the problem is
    /// successfully fixed.
    async fn treat(&self, patient: &mut PatientApp) -> Result<()>;
}

/// Error returned by [`Treatment::dry_run`] if dry run isn't supported.
#[derive(Debug)]
pub struct DryRunNotSupported;

impl std::fmt::Display for DryRunNotSupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dry run not implemented for this treatment")
    }
}

impl std::error::Error for DryRunNotSupported {}

#[async_trait]
trait BoxingDiagnostic {
    async fn diagnose_boxed(&self, patient: &PatientApp) -> Result<Vec<Box<dyn Diagnosis>>>;
}

#[async_trait]
impl<Factory: Diagnostic> BoxingDiagnostic for Factory {
    async fn diagnose_boxed(&self, patient: &PatientApp) -> Result<Vec<Box<dyn Diagnosis>>> {
        Ok(self
            .diagnose(patient)
            .await?
            .into_iter()
            .map(|diag| Box::new(diag) as Box<dyn Diagnosis>)
            .collect())
    }
}

/// Return this as an error from a treatment to stop further diagnoses when
/// the user needs to intervene before the doctor can proceed.
#[derive(Debug)]
pub struct StopDiagnosing {
    message: String,
}

impl std::fmt::Display for StopDiagnosing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl StopDiagnosing {
    /// Creates a new instance.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The message to be displayed to the user indicating what they must do
    /// before resuming diagnosing.
    pub fn message(&self) -> &str {
        &self.message
    }
}
