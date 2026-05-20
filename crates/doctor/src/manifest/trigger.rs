use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use toml::Value;
use toml_edit::{DocumentMut, InlineTable, Item, Table};

use crate::{Diagnosis, Diagnostic, PatientApp, Treatment};

use super::ManifestTreatment;

/// TriggerDiagnostic detects problems with app trigger config.
#[derive(Default)]
pub struct TriggerDiagnostic;

#[async_trait]
impl Diagnostic for TriggerDiagnostic {
    type Diagnosis = TriggerDiagnosis;

    async fn diagnose(&self, patient: &PatientApp) -> Result<Vec<Self::Diagnosis>> {
        let manifest: toml::Value = toml_edit::de::from_document(patient.manifest_doc.clone())?;

        if manifest.get("spin_manifest_version") == Some(&Value::Integer(2)) {
            // Not applicable to manifest V2
            return Ok(vec![]);
        }

        let mut diags = vec![];

        // Top-level trigger config
        diags.extend(TriggerDiagnosis::for_app_trigger(manifest.get("trigger")));

        // Component-level HTTP trigger config
        let trigger_type = manifest
            .get("trigger")
            .and_then(|item| item.get("type"))
            .and_then(|item| item.as_str());
        if let Some("http") = trigger_type
            && let Some(Value::Array(components)) = manifest.get("component")
        {
            let single_component = components.len() == 1;
            for component in components {
                let id = component
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("<missing ID>")
                    .to_string();
                diags.extend(TriggerDiagnosis::for_http_component_trigger(
                    id,
                    component.get("trigger"),
                    single_component,
                ));
            }
        }

        Ok(diags)
    }
}

/// TriggerDiagnosis represents a problem with app trigger config.
#[derive(Debug)]
pub enum TriggerDiagnosis {
    /// Missing app trigger section
    MissingAppTrigger,
    /// Invalid app trigger config
    InvalidAppTrigger(&'static str),
    /// HTTP component trigger missing route field
    HttpComponentTriggerMissingRoute(String, bool),
    /// Invalid HTTP component trigger config
    InvalidHttpComponentTrigger(String, &'static str),
}

impl TriggerDiagnosis {
    fn for_app_trigger(trigger: Option<&Value>) -> Option<Self> {
        let Some(trigger) = trigger else {
            return Some(Self::MissingAppTrigger);
        };
        let Some(trigger) = trigger.as_table() else {
            return Some(Self::InvalidAppTrigger("not a table"));
        };
        let Some(trigger_type) = trigger.get("type") else {
            return Some(Self::InvalidAppTrigger("trigger table missing type"));
        };
        let Some(_) = trigger_type.as_str() else {
            return Some(Self::InvalidAppTrigger("type must be a string"));
        };
        None
    }

    fn for_http_component_trigger(
        id: String,
        trigger: Option<&Value>,
        single_component: bool,
    ) -> Option<Self> {
        let Some(trigger) = trigger else {
            return Some(Self::HttpComponentTriggerMissingRoute(id, single_component));
        };
        let Some(trigger) = trigger.as_table() else {
            return Some(Self::InvalidHttpComponentTrigger(id, "not a table"));
        };
        let Some(route) = trigger.get("route") else {
            return Some(Self::HttpComponentTriggerMissingRoute(id, single_component));
        };
        if route.as_str().is_none() {
            return Some(Self::InvalidHttpComponentTrigger(
                id,
                "route is not a string",
            ));
        }
        None
    }
}

impl Diagnosis for TriggerDiagnosis {
    fn description(&self) -> String {
        match self {
            Self::MissingAppTrigger => "missing top-level trigger config".into(),
            Self::InvalidAppTrigger(msg) => {
                format!("Invalid app trigger config: {msg}")
            }
            Self::HttpComponentTriggerMissingRoute(id, _) => {
                format!("HTTP component {id:?} missing trigger.route")
            }
            Self::InvalidHttpComponentTrigger(id, msg) => {
                format!("Invalid trigger config for http component {id:?}: {msg}")
            }
        }
    }

    fn treatment(&self) -> Option<&dyn Treatment> {
        match self {
            Self::MissingAppTrigger => Some(self),
            // We can reasonably fill in default "route" iff there is only one component
            Self::HttpComponentTriggerMissingRoute(_, single_component) if *single_component => {
                Some(self)
            }
            _ => None,
        }
    }
}

#[async_trait]
impl ManifestTreatment for TriggerDiagnosis {
    fn summary(&self) -> String {
        match self {
            TriggerDiagnosis::MissingAppTrigger => "Add default HTTP trigger config".into(),
            TriggerDiagnosis::HttpComponentTriggerMissingRoute(id, _) => {
                format!("Set trigger.route '/...' for component {id:?}")
            }
            _ => "[invalid treatment]".into(),
        }
    }

    async fn treat_manifest(&self, doc: &mut DocumentMut) -> anyhow::Result<()> {
        match self {
            Self::MissingAppTrigger => {
                // Get or insert missing trigger config
                if doc.get("trigger").is_none() {
                    doc.insert("trigger", Item::Value(InlineTable::new().into()));
                }
                let trigger = doc
                    .get_mut("trigger")
                    .unwrap()
                    .as_table_like_mut()
                    .context("existing trigger value is not a table")?;

                // Get trigger type or insert default "http"
                let trigger_type = trigger.entry("type").or_insert(Item::Value("http".into()));
                if let Some("http") = trigger_type.as_str() {
                    // Strip "type" trailing space
                    if let Some(decor) = trigger_type.as_value_mut().map(|v| v.decor_mut())
                        && let Some(suffix) = decor.suffix().and_then(|s| s.as_str())
                    {
                        decor.set_suffix(suffix.to_string().trim());
                    }
                }
            }
            Self::HttpComponentTriggerMissingRoute(_, true) => {
                // Get the only component
                let components = doc
                    .get_mut("component")
                    .context("missing components")?
                    .as_array_of_tables_mut()
                    .context("component sections aren't an 'array of tables'")?;
                ensure!(
                    components.len() == 1,
                    "can only set default trigger route if there is exactly one component; found {}",
                    components.len()
                );
                let component = components.get_mut(0).unwrap();

                // Get or insert missing trigger config
                if component.get("trigger").is_none() {
                    component.insert("trigger", Item::Table(Table::new()));
                }
                let trigger = component
                    .get_mut("trigger")
                    .unwrap()
                    .as_table_like_mut()
                    .context("existing trigger value is not a table")?;

                // Set missing "route"
                trigger.entry("route").or_insert(Item::Value("/...".into()));
            }
            _ => bail!("cannot be fixed"),
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
        run_correct_test::<TriggerDiagnostic>("manifest_trigger").await;
    }

    #[tokio::test]
    async fn test_missing_app_trigger() {
        let diag =
            run_broken_test::<TriggerDiagnostic>("manifest_trigger", "missing_app_trigger").await;
        assert!(matches!(diag, TriggerDiagnosis::MissingAppTrigger));
    }

    #[tokio::test]
    async fn test_http_component_trigger_missing_route() {
        let diag = run_broken_test::<TriggerDiagnostic>(
            "manifest_trigger",
            "http_component_trigger_missing_route",
        )
        .await;
        assert!(matches!(
            diag,
            TriggerDiagnosis::HttpComponentTriggerMissingRoute(_, _)
        ));
    }
}
