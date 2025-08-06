use std::{fmt::Debug, path::PathBuf};

use anyhow::Result;
use clap::Parser;
use dialoguer::{console::Emoji, Confirm, Select};
use spin_doctor::{Diagnosis, DryRunNotSupported, PatientDiagnosis};

use crate::opts::APP_MANIFEST_FILE_OPT;

#[derive(Parser, Debug)]
#[clap(about = "Detect and fix problems with Spin applications")]
pub struct DoctorCommand {
    /// The application to check. This may be a manifest (spin.toml) file, or a
    /// directory containing a spin.toml file.
    /// If omitted, it defaults to "spin.toml".
    #[clap(
        name = APP_MANIFEST_FILE_OPT,
        short = 'f',
        long = "from",
        alias = "file",
        value_hint = clap::ValueHint::AnyPath,
    )]
    pub app_source: Option<PathBuf>,
}

impl DoctorCommand {
    pub async fn run(self) -> Result<()> {
        let (manifest_file, distance) =
            spin_common::paths::find_manifest_file_path(self.app_source.as_ref())?;
        if distance > 0 {
            anyhow::bail!(
                "No spin.toml in current directory - did you mean '--from {}'?",
                manifest_file.display()
            );
        }

        println!("{icon}The Spin Doctor is in.", icon = Emoji("📟 ", ""));
        println!(
            "{icon}Checking {}...",
            manifest_file.display(),
            icon = Emoji("🩺 ", "")
        );

        let mut checkup = spin_doctor::Checkup::new(manifest_file)?;
        let mut has_problems = false;
        while let Some(PatientDiagnosis { diagnosis, patient }) = checkup.next_diagnosis().await? {
            show_diagnosis(&*diagnosis);
            has_problems = true;

            if let Some(treatment) = diagnosis.treatment() {
                let dry_run = match treatment.dry_run(patient).await {
                    Ok(desc) => Some(desc),
                    Err(err) => {
                        if !err.is::<DryRunNotSupported>() {
                            show_error("Treatment dry run failed: ", err);
                            return Ok(());
                        }
                        None
                    }
                };

                let should_treat =
                    prompt_treatment(treatment.summary(), dry_run).unwrap_or_else(|err| {
                        show_error("Prompt error: ", err);
                        false
                    });

                if should_treat {
                    match treatment.treat(patient).await {
                        Ok(()) => {
                            println!("{icon}Treatment applied!", icon = Emoji("❤  ", ""));
                        }
                        Err(err) => {
                            match err.downcast_ref::<spin_doctor::StopDiagnosing>() {
                                Some(stop) => {
                                    terminal::einfo!("Action required!", "{}", stop.message());
                                    return Ok(());
                                }
                                None => show_error("Treatment failed: ", err),
                            };
                        }
                    }
                }
            }
        }
        if !has_problems {
            println!("{icon}No problems found.", icon = Emoji("❤  ", ""));
        }
        Ok(())
    }
}

fn show_diagnosis(diagnosis: &dyn Diagnosis) {
    let icon = if diagnosis.is_critical() {
        Emoji("❗ ", "")
    } else {
        Emoji("⚠  ", "")
    };
    println!("\n{icon}Diagnosis: {}", diagnosis.description());
}

fn prompt_treatment(summary: String, dry_run: Option<String>) -> Result<bool> {
    let prompt = format!(
        "{icon}The Spin Doctor can help! Would you like to",
        icon = Emoji("🩹 ", "")
    );
    let mut items = vec![summary.as_str(), "Do nothing"];
    if dry_run.is_some() {
        items.push("See more details about the recommended changes");
    }
    let selection = Select::new()
        .with_prompt(prompt)
        .items(&items)
        .default(0)
        .interact_opt()?;

    match selection {
        Some(2) => {
            println!(
                "\n{icon}{}\n",
                dry_run.unwrap_or_default().trim_end(),
                icon = Emoji("📋 ", "")
            );
            Ok(Confirm::new()
                .with_prompt("Would you like to apply this fix?")
                .default(true)
                .interact_opt()?
                .unwrap_or_default())
        }
        Some(0) => Ok(true),
        _ => Ok(false),
    }
}

fn show_error(prefix: &str, err: impl Debug) {
    let icon = Emoji("⁉️ ", "");
    println!("{icon}{prefix}{err:?}");
}
