use anyhow::Context;
use std::path::{Path, PathBuf};
use test_environment::{
    TestEnvironment,
    http::{Method, Request},
    manifest_template::EnvTemplate,
    services::ServicesConfig,
};
use testing_framework::{
    OnTestError, TestError, TestResult,
    runtimes::{
        in_process_spin::InProcessSpin,
        spin_cli::{SpinCli, SpinConfig},
    },
};

/// Configuration for a runtime test
pub struct RuntimeTestConfig<R> {
    /// Path to the test
    pub test_path: PathBuf,
    /// Specific configuration for the runtime
    pub runtime_config: R,
    /// What to do when a test errors
    pub on_error: OnTestError,
}

/// A single runtime test
pub struct RuntimeTest<R> {
    test_path: PathBuf,
    on_error: OnTestError,
    env: TestEnvironment<R>,
}

impl RuntimeTest<SpinCli> {
    /// Run the runtime tests suite.
    ///
    /// Error represents an error in bootstrapping the tests. What happens on individual test failures
    /// is controlled by `on_error`.
    pub fn run_all(
        tests_dir_path: &Path,
        spin_binary: PathBuf,
        on_error: OnTestError,
    ) -> anyhow::Result<()> {
        Self::run_on_all(tests_dir_path, |test_path| {
            let config = RuntimeTestConfig {
                test_path,
                runtime_config: SpinConfig {
                    binary_path: spin_binary.clone(),
                    spin_up_args: Vec::new(),
                    app_type: testing_framework::runtimes::SpinAppType::Http,
                },
                on_error,
            };
            Self::bootstrap(config)?.run();
            Ok(())
        })
    }

    pub fn bootstrap(config: RuntimeTestConfig<SpinConfig>) -> anyhow::Result<Self> {
        log::info!("Testing: {}", config.test_path.display());
        let test_path_clone = config.test_path.to_owned();
        let spin_binary = config.runtime_config.binary_path.clone();
        let preboot = move |env: &mut TestEnvironment<SpinCli>| {
            copy_manifest(&test_path_clone, env)?;
            Ok(())
        };
        let services_config = services_config(&config)?;
        let env_config = SpinCli::config(
            SpinConfig {
                binary_path: spin_binary,
                spin_up_args: Vec::new(),
                app_type: testing_framework::runtimes::SpinAppType::Http,
            },
            services_config,
            preboot,
        );
        let env = TestEnvironment::up(env_config, |_| Ok(()))?;
        Ok(Self {
            test_path: config.test_path,
            on_error: config.on_error,
            env,
        })
    }

    /// Run an individual test
    pub fn run(&mut self) {
        self.run_test(|env| {
            let runtime = env.runtime_mut();
            let request: Request<String> = Request::full(Method::Get, "/", &[("Host", "localhost")], None);
            let response = runtime.make_http_request(request)?;
            if response.status() == 200 {
                return Ok(());
            }
            if response.status() != 500 {
                return Err(anyhow::anyhow!("Runtime tests are expected to return either either a 200 or a 500, but it returned a {}", response.status()).into());
            }
            let text = response
                .text()
                .context("could not get runtime test HTTP response")?;
            if text.is_empty() {
                let stderr = runtime.stderr();
                return Err(anyhow::anyhow!("Runtime tests are expected to return a response body, but the response body was empty.\nstderr:\n{stderr}").into());
            }

            Err(TestError::Failure(RuntimeTestFailure {
                error: text,
                stderr: Some(runtime.stderr().to_owned()),
            }))
        })
    }
}

impl RuntimeTest<InProcessSpin> {
    /// Run the runtime tests suite.
    ///
    /// Error represents an error in bootstrapping the tests. What happens on individual test failures
    /// is controlled by `on_error`.
    pub fn run_all(tests_dir_path: &Path, on_error: OnTestError) -> anyhow::Result<()> {
        Self::run_on_all(tests_dir_path, |test_path| {
            let config = RuntimeTestConfig {
                test_path,
                runtime_config: (),
                on_error,
            };
            Self::bootstrap(config)?.run();
            Ok(())
        })
    }

    pub fn bootstrap(config: RuntimeTestConfig<()>) -> anyhow::Result<Self> {
        log::info!("Testing: {}", config.test_path.display());
        let test_path_clone = config.test_path.to_owned();
        let preboot = move |env: &mut TestEnvironment<InProcessSpin>| {
            copy_manifest(&test_path_clone, env)?;
            Ok(())
        };
        let services_config = services_config(&config)?;
        let env_config = InProcessSpin::config(services_config, preboot);
        let env = TestEnvironment::up(env_config, |_| Ok(()))?;
        Ok(Self {
            test_path: config.test_path,
            on_error: config.on_error,
            env,
        })
    }

    pub fn run(&mut self) {
        self.run_test(|env| {
            let runtime = env.runtime_mut();
            let response = runtime.make_http_request(Request::full(Method::Get, "/", &[("Host", "localhost")],None))?;
            if response.status() == 200 {
                return Ok(());
            }
            if response.status() != 500 {
                return Err(anyhow::anyhow!("Runtime tests are expected to return either either a 200 or a 500, but it returned a {}", response.status()).into());
            }
            let text = response
                .text()
                .context("could not get runtime test HTTP response")?;
            if text.is_empty() {
                return Err(anyhow::anyhow!("Runtime tests are expected to return a response body, but the response body was empty.").into());
            }

            Err(TestError::Failure(RuntimeTestFailure {
                error: text,
                stderr: None
            }))
        })
    }
}

impl<R> RuntimeTest<R> {
    /// Run a closure against all tests in the given tests directory
    pub fn run_on_all(
        tests_dir_path: &Path,
        run: impl Fn(PathBuf) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        for test in std::fs::read_dir(tests_dir_path).with_context(|| {
            format!(
                "failed to read test directory '{}'",
                tests_dir_path.display()
            )
        })? {
            let test = test.context("I/O error reading entry from test directory")?;
            if !test.file_type()?.is_dir() {
                log::debug!(
                    "Ignoring non-sub-directory in test directory: {}",
                    test.path().display()
                );
                continue;
            }
            run(test.path())?;
        }
        Ok(())
    }

    /// Run an individual test
    pub(crate) fn run_test(
        &mut self,
        test: impl FnOnce(&mut TestEnvironment<R>) -> TestResult<RuntimeTestFailure>,
    ) {
        let on_error = self.on_error;
        // macro which will look at `on_error` and do the right thing
        macro_rules! error {
            ($on_error:expr, $($arg:tt)*) => {
                match $on_error {
                    OnTestError::Panic => panic!($($arg)*),
                    OnTestError::Log => {
                        eprintln!($($arg)*);
                        return;
                    }
                }
            };
        }
        let response = test(&mut self.env);
        let error_file = self.test_path.join("error.txt");
        match response {
            Ok(()) if !error_file.exists() => log::info!("Test passed!"),
            Ok(()) => {
                let expected = match std::fs::read_to_string(&error_file) {
                    Ok(e) => e,
                    Err(e) => error!(on_error, "failed to read error.txt file: {}", e),
                };
                error!(
                    on_error,
                    "Test passed but should have failed with error: {expected}"
                )
            }
            Err(TestError::Failure(RuntimeTestFailure { error, stderr }))
                if error_file.exists() =>
            {
                let expected = match std::fs::read_to_string(&error_file) {
                    Ok(e) => e,
                    Err(e) => error!(on_error, "failed to read error.txt file: {e}"),
                };
                if error.contains(&expected) {
                    log::info!("Test passed!");
                } else {
                    error!(
                        on_error,
                        "Test errored but not in the expected way.\n\texpected: {expected}\n\tgot: {error}{}",
                        stderr
                            .map(|e| format!("\n\nstderr:\n{e}"))
                            .unwrap_or_default()
                    )
                }
            }
            Err(TestError::Failure(RuntimeTestFailure { error, stderr })) => {
                error!(
                    on_error,
                    "Test '{}' errored: {error}{}",
                    self.test_path.display(),
                    stderr
                        .map(|e| format!("\nstderr:\n{e}"))
                        .unwrap_or_default()
                );
            }
            Err(TestError::Fatal(extra)) => {
                error!(
                    on_error,
                    "Test '{}' failed to run: {extra}",
                    self.test_path.display()
                );
            }
        }
        if let OnTestError::Log = on_error {
            println!("'{}' passed", self.test_path.display())
        }
    }
}

fn services_config<R>(config: &RuntimeTestConfig<R>) -> anyhow::Result<ServicesConfig> {
    let required_services = required_services(&config.test_path)?;
    let services_config = ServicesConfig::new(
        required_services
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    )?;
    Ok(services_config)
}

/// Get the services that a test requires.
fn required_services(test_path: &Path) -> anyhow::Result<Vec<String>> {
    let services_config_path = test_path.join("services");
    if !services_config_path.exists() {
        return Ok(Vec::new());
    }
    let services_config_file =
        std::fs::read_to_string(&services_config_path).context("could not read services file")?;
    let iter = services_config_file.lines().filter_map(|s| {
        let s = s.trim();
        (!s.is_empty()).then_some(s.to_owned())
    });
    Ok(iter.collect())
}

/// Copies the test dir's manifest file into the temporary directory
///
/// Replaces template variables in the manifest file with components from the components path.
fn copy_manifest<R>(test_dir: &Path, env: &mut TestEnvironment<R>) -> anyhow::Result<()> {
    let manifest_path = test_dir.join("spin.toml");
    let mut manifest = EnvTemplate::from_file(manifest_path).with_context(|| {
        format!(
            "no spin.toml manifest found in test directory {}",
            test_dir.display()
        )
    })?;
    manifest.substitute(env, |s| Some(PathBuf::from(test_components::path(s)?)))?;
    env.write_file("spin.toml", manifest.contents())
        .context("failed to copy spin.toml manifest to temporary directory")?;
    Ok(())
}

/// A runtime test failure
struct RuntimeTestFailure {
    /// The error message returned by the runtime
    error: String,
    /// The runtime's stderr if there is one
    stderr: Option<String>,
}
