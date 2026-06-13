//! The Spin CLI runtime (i.e., the `spin` command-line tool).

use test_environment::{
    TestEnvironment, TestEnvironmentConfig,
    http::{Request, Response},
    io::OutputStream,
    services::ServicesConfig,
};

use super::SpinAppType;
use crate::Runtime;
use std::process::{Command, Stdio};

/// A wrapper around a running Spin CLI instance
pub struct SpinCli {
    process: std::process::Child,
    #[allow(dead_code)]
    stdout: OutputStream,
    stderr: OutputStream,
    io_mode: IoMode,
}

impl SpinCli {
    /// Configure a test environment that uses a local Spin binary as a runtime
    ///
    /// * `spin_binary` - the path to the Spin binary
    /// * `spin_up_args` - the arguments to pass to `spin up`
    /// * `preboot` - a callback that happens after the services have started but before the runtime is
    /// * `services_config` - the services that the test requires
    /// * `app_type` - the type of trigger for the app that Spin is running
    pub fn config(
        spin_config: SpinConfig,
        services_config: ServicesConfig,
        preboot: impl FnOnce(&mut TestEnvironment<SpinCli>) -> anyhow::Result<()> + 'static,
    ) -> TestEnvironmentConfig<Self> {
        TestEnvironmentConfig {
            services_config,
            create_runtime: Box::new(move |env| {
                preboot(env)?;
                SpinCli::start(spin_config, env)
            }),
        }
    }

    /// Start Spin using the binary at `spin_binary_path` in the `env` testing environment
    pub fn start<R>(spin_config: SpinConfig, env: &mut TestEnvironment<R>) -> anyhow::Result<Self> {
        match spin_config.app_type {
            SpinAppType::Http => Self::start_http(spin_config, env),
            SpinAppType::Redis => Self::start_redis(spin_config, env),
            SpinAppType::None => Self::attempt_start(spin_config, env),
        }
    }

    /// Start Spin assuming an HTTP app in `env` testing directory using the binary at `spin_binary_path`
    pub fn start_http<R>(
        spin_config: SpinConfig,
        env: &mut TestEnvironment<R>,
    ) -> anyhow::Result<Self> {
        let mut spin_cmd = Command::new(spin_config.binary_path);
        let child = spin_cmd
            .envs(env.env_vars())
            .arg("up")
            .current_dir(env.path())
            // Bind an OS-assigned free port on Spin's own listener and read the actual
            // port back from its startup output. Pre-allocating a port here is racy
            // because parallel tests can claim it before `spin up` binds.
            .args(["--listen", "127.0.0.1:0"])
            .args(spin_config.spin_up_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env.env_vars() {
            child.env(key, value);
        }
        let mut child = child.spawn()?;
        let stdout = OutputStream::new(child.stdout.take().unwrap());
        let stderr = OutputStream::new(child.stderr.take().unwrap());
        log::debug!("Awaiting spin binary to report its listening port...");
        let mut spin = Self {
            process: child,
            stdout,
            stderr,
            io_mode: IoMode::None,
        };
        let start = std::time::Instant::now();
        loop {
            // `spin up` prints its base URL once the listener is bound and the app is
            // loaded. Observing it confirms readiness and tells us the real port the OS
            // assigned to this Spin instance.
            let found_port = spin.stdout.output_as_str().and_then(parse_serving_port);
            if let Some(port) = found_port {
                match std::net::TcpStream::connect(("127.0.0.1", port)) {
                    Ok(_) => {
                        log::debug!("Spin started on port {port}.");
                        spin.io_mode = IoMode::Http(port);
                        return Ok(spin);
                    }
                    Err(e) => {
                        log::trace!(
                            "Spin reported port {port}, but it is not accepting connections yet: {e}"
                        );
                    }
                }
            }
            if let Some(status) = spin.try_wait()? {
                anyhow::bail!(
                    "Spin exited early with status code {:?}\n{}{}",
                    status.code(),
                    spin.stdout.output_as_str().unwrap_or("<non-utf8>"),
                    spin.stderr.output_as_str().unwrap_or("<non-utf8>")
                );
            }

            if start.elapsed() > std::time::Duration::from_secs(2 * 60) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        anyhow::bail!(
            "`spin up` did not report a listening port within two minutes.\nstdout:\n\t{}\nstderr:\n\t{}",
            spin.stdout.output_as_str().unwrap_or("<non-utf8>"),
            spin.stderr.output_as_str().unwrap_or("<non-utf8>")
        )
    }

    /// Start Spin assuming a Redis app in `env` testing directory using the binary at `spin_binary_path`
    pub fn start_redis<R>(
        spin_config: SpinConfig,
        env: &mut TestEnvironment<R>,
    ) -> anyhow::Result<Self> {
        let mut child = Command::new(spin_config.binary_path)
            .envs(env.env_vars())
            .arg("up")
            .current_dir(env.path())
            .args(spin_config.spin_up_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = OutputStream::new(child.stdout.take().unwrap());
        let stderr = OutputStream::new(child.stderr.take().unwrap());
        let mut spin = Self {
            process: child,
            stdout,
            stderr,
            io_mode: IoMode::Redis,
        };
        // TODO this is a hack to wait for the redis service to start
        std::thread::sleep(std::time::Duration::from_millis(10000));
        if let Some(status) = spin.try_wait()? {
            anyhow::bail!(
                "Spin exited early with status code {:?}\n{}{}",
                status.code(),
                spin.stdout.output_as_str().unwrap_or("<non-utf8>"),
                spin.stderr.output_as_str().unwrap_or("<non-utf8>")
            );
        }
        Ok(spin)
    }

    fn attempt_start<R>(
        spin_config: SpinConfig,
        env: &mut TestEnvironment<R>,
    ) -> anyhow::Result<Self> {
        let mut child = Command::new(spin_config.binary_path)
            .envs(env.env_vars())
            .arg("up")
            .current_dir(env.path())
            .args(spin_config.spin_up_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = OutputStream::new(child.stdout.take().unwrap());
        let stderr = OutputStream::new(child.stderr.take().unwrap());
        child.wait()?;
        Ok(Self {
            process: child,
            stdout,
            stderr,
            io_mode: IoMode::None,
        })
    }

    /// Make an HTTP request against Spin
    ///
    /// Will fail if Spin has already exited or if the io mode is not HTTP
    pub fn make_http_request<B: Into<reqwest::Body>>(
        &mut self,
        request: Request<'_, B>,
    ) -> anyhow::Result<Response> {
        let IoMode::Http(port) = self.io_mode else {
            anyhow::bail!("Spin is not running in HTTP mode");
        };
        if let Some(status) = self.try_wait()? {
            anyhow::bail!("Spin exited early with status code {:?}", status.code());
        }
        log::debug!("Connecting to HTTP server on port {port}...");
        let response = request.send("localhost", port)?;
        log::debug!("Awaiting response from server");
        if let Some(status) = self.try_wait()? {
            anyhow::bail!("Spin exited early with status code {:?}", status.code());
        }
        Ok(response)
    }

    /// Get the HTTP URL of the Spin server if running in http mode
    pub fn http_url(&self) -> Option<String> {
        match self.io_mode {
            IoMode::Http(port) => Some(format!("http://localhost:{}", port)),
            _ => None,
        }
    }

    pub fn stdout(&mut self) -> &str {
        self.stdout.output_as_str().unwrap_or("<non-utf8>")
    }

    pub fn wait_for_non_empty_stdout(&mut self) -> anyhow::Result<&str> {
        const WAIT_FOR_STDOUT_MILLIS: u64 = 5000;
        const POLL_STDOUT_INTERVAL_MILLIS: u64 = 50;
        const NUM_WAITS: u64 = WAIT_FOR_STDOUT_MILLIS / POLL_STDOUT_INTERVAL_MILLIS;

        for _ in 0..NUM_WAITS {
            if !self.stdout().is_empty() {
                // We don't mind if additional stuff has arrived on stdout since
                // the empty check. As long as stdout can't go backwards we're okay.
                // (And snapshotting makes the borrow checker mad, so...)
                return Ok(self.stdout());
            }

            std::thread::sleep(std::time::Duration::from_millis(
                POLL_STDOUT_INTERVAL_MILLIS,
            ));
        }

        anyhow::bail!("waited for text to appear on stdout but it didn't");
    }

    pub fn stderr(&mut self) -> &str {
        self.stderr.output_as_str().unwrap_or("<non-utf8>")
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.process.try_wait()
    }
}

impl Drop for SpinCli {
    fn drop(&mut self) {
        kill_process(&mut self.process);
    }
}

impl Runtime for SpinCli {
    fn error(&mut self) -> anyhow::Result<()> {
        if !matches!(self.io_mode, IoMode::None) && self.try_wait()?.is_some() {
            anyhow::bail!("Spin exited early: {}", self.stderr());
        }

        Ok(())
    }
}

/// Configuration for how the Spin CLI will run
pub struct SpinConfig {
    pub binary_path: std::path::PathBuf,
    pub spin_up_args: Vec<String>,
    pub app_type: SpinAppType,
}

fn kill_process(process: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = process.kill();
    }
    #[cfg(not(windows))]
    {
        let pid = nix::unistd::Pid::from_raw(process.id() as i32);
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::SIGTERM);
    }
}

/// How this Spin instance is communicating with the outside world
enum IoMode {
    /// An http server is running on this port
    Http(u16),
    /// Spin is running in redis mode
    Redis,
    /// Spin may or may not be running
    None,
}

fn parse_serving_port(output: &str) -> Option<u16> {
    parse_plain_serving_port(output).or_else(|| parse_json_serving_port(output))
}

fn parse_plain_serving_port(output: &str) -> Option<u16> {
    output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Serving "))
        .find_map(parse_base_url_port)
}

fn parse_json_serving_port(output: &str) -> Option<u16> {
    let output: serde_json::Value = serde_json::from_str(output).ok()?;
    output
        .get("base_url")
        .and_then(serde_json::Value::as_str)
        .and_then(parse_base_url_port)
}

fn parse_base_url_port(base_url: &str) -> Option<u16> {
    url::Url::parse(base_url).ok()?.port_or_known_default()
}
