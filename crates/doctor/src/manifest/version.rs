use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use toml::Value;
use toml_edit::{DocumentMut, Item, de::from_document};

use crate::{Diagnosis, Diagnostic, PatientApp, Treatment};

use super::ManifestTreatment;

const SPIN_MANIFEST_VERSION: &str = "spin_manifest_version";
const SPIN_VERSION: &str = "spin_version";

/// VersionDiagnostic detects problems with the app manifest version field.
#[derive(Default)]
pub struct VersionDiagnostic;

#[async_trait]
impl Diagnostic for VersionDiagnostic {
    type Diagnosis = VersionDiagnosis;

    async fn diagnose(&self, patient: &PatientApp) -> Result<Vec<Self::Diagnosis>> {
        let doc = &patient.manifest_doc;
        let test: VersionProbe =
            from_document(doc.clone()).context("failed to decode VersionProbe")?;

        if let Some(value) = test.spin_manifest_version {
            if corrected_version(&value).is_some() {
                return Ok(vec![VersionDiagnosis::WrongValue(value)]);
            }
        } else if test.spin_version.is_some() {
            return Ok(vec![VersionDiagnosis::OldVersionKey]);
        } else {
            return Ok(vec![VersionDiagnosis::MissingVersion]);
        }
        Ok(vec![])
    }
}

fn corrected_version(value: &Value) -> Option<toml_edit::Value> {
    match value {
        Value::String(s) if s == "1" => None,
        Value::Integer(2) => None,
        Value::Integer(1) => Some("1".into()),
        _ => Some(2.into()),
    }
}

#[derive(Debug, Deserialize)]
struct VersionProbe {
    spin_manifest_version: Option<Value>,
    spin_version: Option<Value>,
}

/// VersionDiagnosis represents a problem with the app manifest version field.
#[derive(Debug)]
pub enum VersionDiagnosis {
    /// Missing any known version key
    MissingVersion,
    /// Using old spin_version key
    OldVersionKey,
    /// Wrong version value
    WrongValue(Value),
}

impl Diagnosis for VersionDiagnosis {
    fn description(&self) -> String {
        match self {
            Self::MissingVersion => "Manifest missing 'spin_manifest_version' key".into(),
            Self::OldVersionKey => "Manifest using old 'spin_version' key".into(),
            Self::WrongValue(val) => {
                format!(r#"Manifest 'spin_manifest_version' must be "1" or 2, not {val}"#)
            }
        }
    }

    fn is_critical(&self) -> bool {
        !matches!(self, Self::OldVersionKey)
    }

    fn treatment(&self) -> Option<&dyn Treatment> {
        Some(self)
    }
}

#[async_trait]
impl ManifestTreatment for VersionDiagnosis {
    fn summary(&self) -> String {
        match self {
            Self::MissingVersion => "Add spin_manifest_version to manifest".into(),
            Self::OldVersionKey => "Replace 'spin_version' with 'spin_manifest_version'".into(),
            Self::WrongValue(value) => format!(
                "Set manifest version to {}",
                corrected_version(value).unwrap()
            ),
        }
    }

    async fn treat_manifest(&self, doc: &mut DocumentMut) -> anyhow::Result<()> {
        doc.remove(SPIN_VERSION);

        let item = Item::Value(match self {
            Self::MissingVersion => 2.into(),
            Self::OldVersionKey => "1".into(),
            Self::WrongValue(value) => corrected_version(value).unwrap(),
        });
        if let Some(existing) = doc.get_mut(SPIN_MANIFEST_VERSION) {
            *existing = item;
        } else {
            doc.insert(SPIN_MANIFEST_VERSION, item);
            // (ab)use stable sorting to move the inserted item to the top
            doc.sort_values_by(|k1, _, k2, _| {
                let k1_is_version = k1.get() == SPIN_MANIFEST_VERSION;
                let k2_is_version = k2.get() == SPIN_MANIFEST_VERSION;
                // true > false
                k2_is_version.cmp(&k1_is_version)
            })
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{run_broken_test, run_correct_test};

    use super::*;

    #[tokio::test]
    async fn test_correct() {
        run_correct_test::<VersionDiagnostic>("manifest_version").await;
    }

    #[tokio::test]
    async fn test_old_key() {
        let diag = run_broken_test::<VersionDiagnostic>("manifest_version", "old_key").await;
        assert!(matches!(diag, VersionDiagnosis::OldVersionKey));
    }

    #[tokio::test]
    async fn test_wrong_value() {
        let diag = run_broken_test::<VersionDiagnostic>("manifest_version", "wrong_value").await;
        assert!(matches!(diag, VersionDiagnosis::WrongValue(_)));
    }
}
