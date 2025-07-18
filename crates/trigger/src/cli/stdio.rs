use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    task::Poll,
};

use anyhow::{Context, Result};
use spin_common::ui::quoted_path;
use spin_core::async_trait;
use spin_factor_wasi::WasiFactor;
use spin_factors::RuntimeFactors;
use spin_factors_executor::ExecutorHooks;
use tokio::io::AsyncWrite;

pub const STDOUT_LOG_FILE_SUFFIX: &str = "stdout";
pub const STDERR_LOG_FILE_SUFFIX: &str = "stderr";

/// Which components should have their logs followed on stdout/stderr.
#[derive(Clone, Debug, Default)]
pub enum FollowComponents {
    #[default]
    /// No components should have their logs followed.
    None,
    /// Only the specified components should have their logs followed.
    Named(HashSet<String>),
    /// All components should have their logs followed.
    All,
}

impl FollowComponents {
    /// Whether a given component should have its logs followed on stdout/stderr.
    pub fn should_follow(&self, component_id: &str) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Named(ids) => ids.contains(component_id),
        }
    }
}

/// Implements TriggerHooks, writing logs to a log file and (optionally) stderr
pub struct StdioLoggingExecutorHooks {
    follow_components: FollowComponents,
    log_dir: Option<PathBuf>,
    truncate_log: bool,
}

impl StdioLoggingExecutorHooks {
    pub fn new(
        follow_components: FollowComponents,
        log_dir: Option<PathBuf>,
        truncate_log: bool,
    ) -> Self {
        Self {
            follow_components,
            log_dir,
            truncate_log,
        }
    }

    fn component_stdio_writer(
        &self,
        component_id: &str,
        log_suffix: &str,
        log_dir: Option<&Path>,
    ) -> Result<ComponentStdioWriter> {
        let sanitized_component_id = sanitize_filename::sanitize(component_id);
        let log_path = log_dir
            .map(|log_dir| log_dir.join(format!("{sanitized_component_id}_{log_suffix}.txt",)));
        let log_path = log_path.as_deref();

        let follow = self.follow_components.should_follow(component_id);
        match log_path {
            Some(log_path) => ComponentStdioWriter::new_forward(log_path, follow)
                .with_context(|| format!("Failed to open log file {}", quoted_path(log_path))),
            None => ComponentStdioWriter::new_inherit(),
        }
    }

    fn validate_follows(&self, app: &spin_app::App) -> anyhow::Result<()> {
        match &self.follow_components {
            FollowComponents::Named(names) => {
                let component_ids: HashSet<_> =
                    app.components().map(|c| c.id().to_owned()).collect();
                let unknown_names: Vec<_> = names.difference(&component_ids).collect();
                if unknown_names.is_empty() {
                    Ok(())
                } else {
                    let unknown_list = bullet_list(&unknown_names);
                    let actual_list = bullet_list(&component_ids);
                    let message = anyhow::anyhow!("The following component(s) specified in --follow do not exist in the application:\n{unknown_list}\nThe following components exist:\n{actual_list}");
                    Err(message)
                }
            }
            _ => Ok(()),
        }
    }

    fn truncate_log_files(&self, log_dir: &Option<PathBuf>) {
        let Some(dir) = log_dir else { return };
        if let Ok(entries) = dir.read_dir() {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };

                if name.ends_with(&format!("{STDOUT_LOG_FILE_SUFFIX}.txt"))
                    || name.ends_with(&format!("{STDERR_LOG_FILE_SUFFIX}.txt"))
                {
                    _ = std::fs::File::create(dir.join(name));
                }
            }
        }
    }
}

#[async_trait]
impl<F: RuntimeFactors, U> ExecutorHooks<F, U> for StdioLoggingExecutorHooks {
    async fn configure_app(
        &self,
        configured_app: &spin_factors::ConfiguredApp<F>,
    ) -> anyhow::Result<()> {
        self.validate_follows(configured_app.app())?;

        if let Some(dir) = &self.log_dir {
            // Ensure log dir exists if set
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create log dir {}", quoted_path(dir)))?;

            if self.truncate_log {
                self.truncate_log_files(&self.log_dir);
            }

            println!("Logging component stdio to {}", quoted_path(dir.join("")))
        }
        Ok(())
    }

    fn prepare_instance(
        &self,
        builder: &mut spin_factors_executor::FactorsInstanceBuilder<F, U>,
    ) -> anyhow::Result<()> {
        let component_id = builder.app_component().id().to_string();
        let Some(wasi_builder) = builder.factor_builder::<WasiFactor>() else {
            return Ok(());
        };
        wasi_builder.stdout_pipe(self.component_stdio_writer(
            &component_id,
            STDOUT_LOG_FILE_SUFFIX,
            self.log_dir.as_deref(),
        )?);
        wasi_builder.stderr_pipe(self.component_stdio_writer(
            &component_id,
            STDERR_LOG_FILE_SUFFIX,
            self.log_dir.as_deref(),
        )?);
        Ok(())
    }
}

/// ComponentStdioWriter forwards output to a log file, (optionally) stderr, and (optionally) to a
/// tracing compatibility layer.
pub struct ComponentStdioWriter {
    inner: ComponentStdioWriterInner,
}

enum ComponentStdioWriterInner {
    /// Inherit stdout/stderr from the parent process.
    Inherit,
    /// Forward stdout/stderr to a file in addition to the inherited stdout/stderr.
    Forward {
        sync_file: std::fs::File,
        async_file: tokio::fs::File,
        state: ComponentStdioWriterState,
        follow: bool,
    },
}

#[derive(Debug)]
enum ComponentStdioWriterState {
    File,
    Follow(std::ops::Range<usize>),
}

impl ComponentStdioWriter {
    fn new_forward(log_path: &Path, follow: bool) -> anyhow::Result<Self> {
        let sync_file = std::fs::File::options()
            .create(true)
            .append(true)
            .open(log_path)?;

        let async_file = sync_file
            .try_clone()
            .context("could not get async file handle")?
            .into();

        Ok(Self {
            inner: ComponentStdioWriterInner::Forward {
                sync_file,
                async_file,
                state: ComponentStdioWriterState::File,
                follow,
            },
        })
    }

    fn new_inherit() -> anyhow::Result<Self> {
        Ok(Self {
            inner: ComponentStdioWriterInner::Inherit,
        })
    }
}

impl AsyncWrite for ComponentStdioWriter {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<std::result::Result<usize, std::io::Error>> {
        let this = self.get_mut();

        loop {
            match &mut this.inner {
                ComponentStdioWriterInner::Inherit => {
                    let written = futures::ready!(
                        std::pin::Pin::new(&mut tokio::io::stderr()).poll_write(cx, buf)
                    );
                    let written = match written {
                        Ok(w) => w,
                        Err(e) => return Poll::Ready(Err(e)),
                    };
                    return Poll::Ready(Ok(written));
                }
                ComponentStdioWriterInner::Forward {
                    async_file,
                    state,
                    follow,
                    ..
                } => match &state {
                    ComponentStdioWriterState::File => {
                        let written =
                            futures::ready!(std::pin::Pin::new(async_file).poll_write(cx, buf));
                        let written = match written {
                            Ok(w) => w,
                            Err(e) => return Poll::Ready(Err(e)),
                        };
                        if *follow {
                            *state = ComponentStdioWriterState::Follow(0..written);
                        } else {
                            return Poll::Ready(Ok(written));
                        }
                    }
                    ComponentStdioWriterState::Follow(range) => {
                        let written = futures::ready!(std::pin::Pin::new(&mut tokio::io::stderr())
                            .poll_write(cx, &buf[range.clone()]));
                        let written = match written {
                            Ok(w) => w,
                            Err(e) => return Poll::Ready(Err(e)),
                        };
                        if range.start + written >= range.end {
                            let end = range.end;
                            *state = ComponentStdioWriterState::File;
                            return Poll::Ready(Ok(end));
                        } else {
                            *state = ComponentStdioWriterState::Follow(
                                (range.start + written)..range.end,
                            );
                        };
                    }
                },
            }
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::result::Result<(), std::io::Error>> {
        let this = self.get_mut();

        match &mut this.inner {
            ComponentStdioWriterInner::Inherit => {
                std::pin::Pin::new(&mut tokio::io::stderr()).poll_flush(cx)
            }
            ComponentStdioWriterInner::Forward {
                async_file, state, ..
            } => match state {
                ComponentStdioWriterState::File => std::pin::Pin::new(async_file).poll_flush(cx),
                ComponentStdioWriterState::Follow(_) => {
                    std::pin::Pin::new(&mut tokio::io::stderr()).poll_flush(cx)
                }
            },
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<std::result::Result<(), std::io::Error>> {
        let this = self.get_mut();

        match &mut this.inner {
            ComponentStdioWriterInner::Inherit => {
                std::pin::Pin::new(&mut tokio::io::stderr()).poll_flush(cx)
            }
            ComponentStdioWriterInner::Forward {
                async_file, state, ..
            } => match state {
                ComponentStdioWriterState::File => std::pin::Pin::new(async_file).poll_shutdown(cx),
                ComponentStdioWriterState::Follow(_) => {
                    std::pin::Pin::new(&mut tokio::io::stderr()).poll_flush(cx)
                }
            },
        }
    }
}

impl std::io::Write for ComponentStdioWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        spin_telemetry::logs::handle_app_log(buf);

        match &mut self.inner {
            ComponentStdioWriterInner::Inherit => {
                std::io::stderr().write_all(buf)?;
                Ok(buf.len())
            }
            ComponentStdioWriterInner::Forward {
                sync_file, follow, ..
            } => {
                let written = sync_file.write(buf)?;
                if *follow {
                    std::io::stderr().write_all(&buf[..written])?;
                }
                Ok(written)
            }
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.inner {
            ComponentStdioWriterInner::Inherit => std::io::stderr().flush(),
            ComponentStdioWriterInner::Forward {
                sync_file, follow, ..
            } => {
                sync_file.flush()?;
                if *follow {
                    std::io::stderr().flush()?;
                }
                Ok(())
            }
        }
    }
}

fn bullet_list<S: std::fmt::Display>(items: impl IntoIterator<Item = S>) -> String {
    items
        .into_iter()
        .map(|item| format!("  - {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}
