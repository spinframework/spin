use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use spin_common::ui::quoted_path;
use spin_manifest::{ManifestVersion, compat::v1_to_v2_app, schema::v1::AppManifestV1};
use toml_edit::{Item, Table, de::from_document, ser::to_document};

use crate::{Diagnosis, Diagnostic, PatientApp, Treatment};

/// UpgradeDiagnostic detects old manifest versions and upgrades them.
#[derive(Default)]
pub struct UpgradeDiagnostic;

#[async_trait]
impl Diagnostic for UpgradeDiagnostic {
    type Diagnosis = UpgradeDiagnosis;

    async fn diagnose(&self, patient: &PatientApp) -> Result<Vec<Self::Diagnosis>> {
        Ok(
            match ManifestVersion::detect(&patient.manifest_doc.to_string())? {
                ManifestVersion::V1 => vec![UpgradeDiagnosis],
                _ => vec![],
            },
        )
    }
}

/// UpgradeDiagnosis represents an upgradable manifest.
#[derive(Debug)]
pub struct UpgradeDiagnosis;

impl Diagnosis for UpgradeDiagnosis {
    fn description(&self) -> String {
        "Version 1 manifest can be upgraded to version 2".into()
    }

    fn is_critical(&self) -> bool {
        false
    }

    fn treatment(&self) -> Option<&dyn crate::Treatment> {
        Some(self)
    }
}

#[async_trait]
impl Treatment for UpgradeDiagnosis {
    fn summary(&self) -> String {
        "Upgrade manifest to version 2".into()
    }

    async fn treat(&self, patient: &mut PatientApp) -> Result<()> {
        let v1: AppManifestV1 = from_document(patient.manifest_doc.clone())
            .context("failed to decode AppManifestV1")?;
        let v2 = v1_to_v2_app(v1).context("failed to upgrade version 1 manifest to version 2")?;
        let mut v2_doc = to_document(&v2)?;

        // Format [application] table
        let application = uninline_table(&mut v2_doc["application"])?;
        if let Some(application_trigger) = application.get_mut("trigger") {
            let application_trigger = uninline_table(application_trigger)?;
            application_trigger.set_dotted(true);
            for (_, trigger_config) in application_trigger.iter_mut() {
                uninline_table(trigger_config)?.set_implicit(true);
            }
        }

        // Format [variables]
        if let Some(variables) = v2_doc.get_mut("variables") {
            uninline_table(variables)?;
        }

        // Format [[trigger.*]] tables
        if let Some(triggers) = v2_doc.get_mut("trigger") {
            let triggers = uninline_table(triggers)?;
            triggers.set_dotted(true);
            for (_, typed_triggers) in triggers.iter_mut() {
                *typed_triggers = Item::ArrayOfTables(
                    std::mem::take(typed_triggers)
                        .into_array_of_tables()
                        .map_err(expected_table)?,
                );
            }
        }

        // Format [component.*] tables
        if let Some(components) = v2_doc.get_mut("component") {
            let components = uninline_table(components)?;
            components.set_dotted(true);
            for (_, component) in components.iter_mut() {
                let component = uninline_table(component)?;
                if let Some(build) = component.get_mut("build") {
                    uninline_table(build)?;
                }
            }
        }

        // Back-up original V1 manifest
        let v1_backup_path = patient.manifest_path.with_extension("toml.v1_backup");
        std::fs::rename(&patient.manifest_path, &v1_backup_path)
            .context("failed to back up existing manifest")?;
        println!(
            "Version 1 manifest backed up to {}.",
            quoted_path(&v1_backup_path)
        );

        // Write new V2 manifest
        std::fs::write(&patient.manifest_path, v2_doc.to_string())
            .context("failed to write version 2 manifest")?;
        patient.manifest_doc = v2_doc;

        Ok(())
    }
}

fn uninline_table(item: &mut Item) -> Result<&mut Table> {
    *item = Item::Table(std::mem::take(item).into_table().map_err(expected_table)?);
    Ok(item.as_table_mut().unwrap())
}

fn expected_table(got: Item) -> anyhow::Error {
    anyhow!("expected table, got {}", got.type_name())
}
