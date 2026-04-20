use anyhow::Result;
use async_trait::async_trait;

use crate::{Diagnosis, Diagnostic, PatientApp, StopDiagnosing, Treatment};

/// VersionDiagnostic detects problems with the app manifest version field.
#[derive(Default)]
pub struct TargetDiagnostic;

#[async_trait]
impl Diagnostic for TargetDiagnostic {
    type Diagnosis = TargetDiagnosis;

    async fn diagnose(&self, patient: &PatientApp) -> Result<Vec<Self::Diagnosis>> {
        // TODO: this, down to the "does the app use Rust" check, probably ought to move up to the Rust level
        // but we can defer this until we have more Rust diagnoses
        let manifest_str = patient.manifest_doc.to_string();
        let manifest = spin_manifest::manifest_from_str(&manifest_str)?;
        let uses_rust = manifest.components.values().any(|c| {
            c.build
                .as_ref()
                .map(|b| b.commands().any(|c| c.starts_with("cargo")))
                .unwrap_or_default()
        });

        if uses_rust {
            diagnose_rust_wasi_target().await
        } else {
            Ok(vec![])
        }
    }
}

async fn diagnose_rust_wasi_target() -> Result<Vec<TargetDiagnosis>> {
    // does any component contain a build command with `cargo` as the program?
    // if so, run rustup target list --installed and:
    // - if rustup is not present, check if cargo is present
    //   - if not, return RustNotInstalled
    //   - if so, warn but return empty list (Rust is installed but not via rustup, so we can't perform a diagnosis - bit of an edge case this one, and the user probably knows what they're doing...?)
    // - if rustup is present but the list does not contain wasm32-wasip2, return WasmTargetNotInstalled
    // - if the list does contain wasm32-wasip2, return an empty list
    // NOTE: this does not currently check against the Rust SDK MSRV - that could
    // be a future enhancement or separate diagnosis, but at least the Rust compiler
    // should give a clear error for that!

    let diagnosis = match get_rustup_target_status().await? {
        RustupStatus::AllInstalled => vec![],
        RustupStatus::WasiNotInstalled => vec![TargetDiagnosis::WasmTargetNotInstalled],
        RustupStatus::RustupNotInstalled => match get_cargo_status().await? {
            CargoStatus::Installed => {
                terminal::warn!(
                    "Spin Doctor can't determine if the Rust wasm32-wasip2 target is installed."
                );
                vec![]
            }
            CargoStatus::NotInstalled => vec![TargetDiagnosis::RustNotInstalled],
        },
    };
    Ok(diagnosis)
}

#[allow(clippy::enum_variant_names)]
enum RustupStatus {
    RustupNotInstalled,
    WasiNotInstalled,
    AllInstalled,
}

async fn get_rustup_target_status() -> Result<RustupStatus> {
    let target_list_output = tokio::process::Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .await;
    let status = match target_list_output {
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                RustupStatus::RustupNotInstalled
            } else {
                anyhow::bail!("Failed to run `rustup target list --installed`: {e:#}")
            }
        }
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout.lines().any(|line| line == "wasm32-wasip2") {
                RustupStatus::AllInstalled
            } else {
                RustupStatus::WasiNotInstalled
            }
        }
    };
    Ok(status)
}

enum CargoStatus {
    Installed,
    NotInstalled,
}

async fn get_cargo_status() -> Result<CargoStatus> {
    let cmd_output = tokio::process::Command::new("cargo")
        .arg("--version")
        .output()
        .await;
    let status = match cmd_output {
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                CargoStatus::NotInstalled
            } else {
                anyhow::bail!("Failed to run `cargo --version`: {e:#}")
            }
        }
        Ok(_) => CargoStatus::Installed,
    };
    Ok(status)
}

/// TargetDiagnosis represents a problem with the Rust target.
#[derive(Debug)]
pub enum TargetDiagnosis {
    /// Rust is not installed: neither cargo nor rustup is present
    RustNotInstalled,
    /// The Rust wasm32-wasip2 target is not installed: rustup is present but the target isn't
    WasmTargetNotInstalled,
}

impl Diagnosis for TargetDiagnosis {
    fn description(&self) -> String {
        match self {
            Self::RustNotInstalled => "The Rust compiler isn't installed".into(),
            Self::WasmTargetNotInstalled => {
                "The required Rust target 'wasm32-wasip2' isn't installed".into()
            }
        }
    }

    fn treatment(&self) -> Option<&dyn Treatment> {
        Some(self)
    }
}

#[async_trait]
impl Treatment for TargetDiagnosis {
    fn summary(&self) -> String {
        match self {
            Self::RustNotInstalled => "Install the Rust compiler and the wasm32-wasip2 target",
            Self::WasmTargetNotInstalled => "Install the Rust wasm32-wasip2 target",
        }
        .into()
    }

    async fn dry_run(&self, _patient: &PatientApp) -> Result<String> {
        let message = match self {
            Self::RustNotInstalled => "Download and run the Rust installer from https://rustup.rs, with the `--target wasm32-wasip2` option",
            Self::WasmTargetNotInstalled => "Run the following command:\n    `rustup target add wasm32-wasip2`",
        };
        Ok(message.into())
    }

    async fn treat(&self, _patient: &mut PatientApp) -> Result<()> {
        match self {
            Self::RustNotInstalled => {
                install_rust_with_wasi_target().await?;
            }
            Self::WasmTargetNotInstalled => {
                install_wasi_target()?;
            }
        }
        Ok(())
    }
}

async fn install_rust_with_wasi_target() -> Result<()> {
    let status = run_rust_installer().await?;
    anyhow::ensure!(status.success(), "Rust installation failed: {status:?}");
    let stop = StopDiagnosing::new("Because Rust was just installed, you may need to run a script or restart your command shell to add Rust to your PATH. Please follow the instructions at the end of the installer output above before re-running `spin doctor`.");
    Err(anyhow::anyhow!(stop))
}

#[cfg(not(windows))]
async fn run_rust_installer() -> Result<std::process::ExitStatus> {
    use std::io::Write;

    let resp = reqwest::get("https://sh.rustup.rs").await?;
    let script = resp.bytes().await?;

    let mut cmd = std::process::Command::new("sh");
    cmd.args(["-s", "--", "--target", "wasm32-wasip2"]);
    cmd.stdin(std::process::Stdio::piped());
    let mut shell = cmd.spawn()?;
    let mut stdin = shell.stdin.take().unwrap();
    std::thread::spawn(move || {
        stdin.write_all(&script).unwrap();
    });

    let output = shell.wait_with_output()?;
    Ok(output.status)
}

#[cfg(windows)]
async fn run_rust_installer() -> Result<std::process::ExitStatus> {
    // We currently distribute Windows builds only for x64, so hopefully
    // this won't be an issue.
    if std::env::consts::ARCH != "x86_64" {
        anyhow::bail!("Spin Doctor can only install Rust for Windows on x64 processors");
    }

    let temp_dir = tempfile::TempDir::new()?;
    let installer_path = temp_dir.path().join("rustup-init.exe");

    let resp = reqwest::get("https://win.rustup.rs/x86_64").await?;
    let installer_bin = resp.bytes().await?;

    std::fs::write(&installer_path, &installer_bin)?;

    let mut cmd = std::process::Command::new(installer_path);
    cmd.args(["--target", "wasm32-wasip2"]);
    let status = cmd.status()?;
    Ok(status)
}

fn install_wasi_target() -> Result<()> {
    let mut cmd = std::process::Command::new("rustup");
    cmd.args(["target", "add", "wasm32-wasip2"]);
    let status = cmd.status()?;
    anyhow::ensure!(
        status.success(),
        "Installation command {cmd:?} failed: {status:?}"
    );
    Ok(())
}
