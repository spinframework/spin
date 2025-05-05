use crate::{directory_rels::notify_if_nondefault_rel, opts::*};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use spin_common::arg_parser::parse_kv;
use spin_oci::{client::InferPredefinedAnnotations, Client, ComposeMode};
use std::{io::Read, path::PathBuf, time::Duration};

/// Commands for working with OCI registries to distribute applications.
#[derive(Subcommand, Debug)]
pub enum RegistryCommands {
    /// Push a Spin application to a registry.
    Push(Push),
    /// Pull a Spin application from a registry.
    Pull(Pull),
    /// Log in to a registry.
    Login(Login),
}

impl RegistryCommands {
    pub async fn run(self) -> Result<()> {
        match self {
            RegistryCommands::Push(cmd) => cmd.run().await,
            RegistryCommands::Pull(cmd) => cmd.run().await,
            RegistryCommands::Login(cmd) => cmd.run().await,
        }
    }
}

#[derive(Parser, Debug)]
pub struct Push {
    /// The application to push. This may be a manifest (spin.toml) file, or a
    /// directory containing a spin.toml file.
    /// If omitted, it defaults to "spin.toml".
    #[clap(
        name = APP_MANIFEST_FILE_OPT,
        short = 'f',
        long = "from",
        alias = "file",
    )]
    pub app_source: Option<PathBuf>,

    /// Ignore server certificate errors
    #[clap(
        name = INSECURE_OPT,
        short = 'k',
        long = "insecure",
        takes_value = false,
    )]
    pub insecure: bool,

    /// Compose component dependencies before pushing the application.
    ///
    /// The default is to compose before pushing, which maximises compatibility with
    /// different Spin runtime hosts. Turning composition off can optimise
    /// bandwidth for shared dependencies, but makes the pushed image incompatible
    /// with hosts that cannot carry out composition themselves.
    #[clap(long = "compose", default_value_t = true)]
    pub compose: bool,

    /// Specifies to perform `spin build` (with the default options) before pushing the application.
    #[clap(long, takes_value = false, env = ALWAYS_BUILD_ENV)]
    pub build: bool,

    /// Reference in the registry of the Spin application.
    /// This is a string whose format is defined by the registry standard, and generally consists of <registry>/<username>/<application-name>:<version>. E.g. ghcr.io/ogghead/spin-test-app:0.1.0
    #[clap()]
    pub reference: String,

    /// Cache directory for downloaded registry data.
    #[clap(long)]
    pub cache_dir: Option<PathBuf>,

    /// Specifies the OCI image manifest annotations (in key=value format).
    /// Any existing value will be overwritten. Can be used multiple times.
    #[clap(long = "annotation", parse(try_from_str = parse_kv))]
    pub annotations: Vec<(String, String)>,
}

impl Push {
    pub async fn run(self) -> Result<()> {
        let (app_file, distance) =
            spin_common::paths::find_manifest_file_path(self.app_source.as_ref())?;
        notify_if_nondefault_rel(&app_file, distance);

        if self.build {
            spin_build::build_default(&app_file, self.cache_dir.clone()).await?;
        }

        let annotations = if self.annotations.is_empty() {
            None
        } else {
            Some(self.annotations.iter().cloned().collect())
        };

        let mut client = spin_oci::Client::new(self.insecure, self.cache_dir.clone()).await?;

        let _spinner = create_dotted_spinner(2000, "Pushing app to the Registry".to_owned());

        let compose_mode = if self.compose {
            ComposeMode::All
        } else {
            ComposeMode::Skip
        };

        let digest = client
            .push(
                &app_file,
                &self.reference,
                annotations,
                InferPredefinedAnnotations::All,
                compose_mode,
            )
            .await?;
        match digest {
            Some(digest) => println!("Pushed with digest {digest}"),
            None => println!("Pushed; the registry did not return the digest"),
        };

        Ok(())
    }
}

#[derive(Parser, Debug)]
pub struct Pull {
    /// Ignore server certificate errors
    #[clap(
        name = INSECURE_OPT,
        short = 'k',
        long = "insecure",
        takes_value = false,
    )]
    pub insecure: bool,

    /// Reference in the registry of the published Spin application.
    /// This is a string whose format is defined by the registry standard, and generally consists of <registry>/<username>/<application-name>:<version>. E.g. ghcr.io/ogghead/spin-test-app:0.1.0
    #[clap()]
    pub reference: String,

    /// Cache directory for downloaded registry data.
    #[clap(long)]
    pub cache_dir: Option<PathBuf>,
}

impl Pull {
    /// Pull a Spin application from an OCI registry
    pub async fn run(self) -> Result<()> {
        let mut client = spin_oci::Client::new(self.insecure, self.cache_dir.clone()).await?;

        let _spinner = create_dotted_spinner(2000, "Pulling app from the Registry".to_owned());

        client.pull(&self.reference).await?;
        println!("Successfully pulled the app from the registry");
        Ok(())
    }
}

#[derive(Parser, Debug)]
pub struct Login {
    /// Username for the registry
    #[clap(long = "username", short = 'u')]
    pub username: Option<String>,

    /// Password for the registry
    #[clap(long = "password", short = 'p')]
    pub password: Option<String>,

    /// Take the password from stdin
    #[clap(
        long = "password-stdin",
        takes_value = false,
        conflicts_with = "password"
    )]
    pub password_stdin: bool,

    /// OCI registry server (e.g. ghcr.io)
    #[clap()]
    pub server: String,
}

impl Login {
    pub async fn run(self) -> Result<()> {
        let username = match self.username {
            Some(u) => u,
            None => {
                let prompt = "Username";
                loop {
                    let result = dialoguer::Input::<String>::new()
                        .with_prompt(prompt)
                        .interact_text()?;
                    if result.trim().is_empty() {
                        continue;
                    } else {
                        break result;
                    }
                }
            }
        };

        // If the --password-stdin flag is passed, read the password from standard input.
        // Otherwise, if the --password flag was passed with a value, use that value. Finally, if
        // neither was passed, prompt the user to input the password.
        let password = if self.password_stdin {
            let mut buf = String::new();
            let mut stdin = std::io::stdin().lock();
            stdin.read_to_string(&mut buf)?;
            buf
        } else {
            match self.password {
                Some(p) => p,
                None => rpassword::prompt_password("Password: ")?,
            }
        };

        Client::login(&self.server, &username, &password)
            .await
            .context("cannot log in to the registry")?;

        println!(
            "Successfully logged in as {} to registry {}",
            username, &self.server
        );
        Ok(())
    }
}

fn create_dotted_spinner(interval: u64, message: String) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    spinner.enable_steady_tick(Duration::from_millis(interval));
    spinner.set_style(
        ProgressStyle::with_template("{msg}{spinner}\n")
            .unwrap()
            .tick_strings(&[".", "..", "...", "....", "....."]),
    );
    spinner.set_message(message);
    spinner
}
