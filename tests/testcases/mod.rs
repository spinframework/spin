use anyhow::Context;
use std::{collections::HashMap, path::PathBuf};
use test_environment::{
    TestEnvironment, http, manifest_template::EnvTemplate, services::ServicesConfig,
};
use testing_framework::runtimes::spin_cli::{SpinCli, SpinConfig};

/// Run an integration test
pub fn run_test(
    test_name: impl Into<String>,
    spin_config: SpinConfig,
    services_config: ServicesConfig,
    test: impl FnOnce(
        &mut TestEnvironment<testing_framework::runtimes::spin_cli::SpinCli>,
    ) -> testing_framework::TestResult<anyhow::Error>
    + 'static,
) -> testing_framework::TestResult<anyhow::Error> {
    run_test_inited(test_name, spin_config, services_config, |_| Ok(()), test)
}

/// Run an integration test, initialising the environment before running Spin
pub fn run_test_inited(
    test_name: impl Into<String>,
    spin_config: SpinConfig,
    services_config: ServicesConfig,
    init_env: impl FnOnce(
        &mut TestEnvironment<testing_framework::runtimes::spin_cli::SpinCli>,
    ) -> anyhow::Result<()>
    + 'static,
    test: impl FnOnce(
        &mut TestEnvironment<testing_framework::runtimes::spin_cli::SpinCli>,
    ) -> testing_framework::TestResult<anyhow::Error>
    + 'static,
) -> testing_framework::TestResult<anyhow::Error> {
    let mut env = bootstap_env(test_name, spin_config, services_config, init_env)
        .context("failed to boot test environment")?;
    test(&mut env)?;
    Ok(())
}

/// Bootstrap a test environment
pub fn bootstap_env(
    test_name: impl Into<String>,
    spin_config: SpinConfig,
    services_config: ServicesConfig,
    init_env: impl FnOnce(
        &mut TestEnvironment<testing_framework::runtimes::spin_cli::SpinCli>,
    ) -> anyhow::Result<()>
    + 'static,
) -> anyhow::Result<TestEnvironment<testing_framework::runtimes::spin_cli::SpinCli>> {
    let test_name = test_name.into();
    let config = SpinCli::config(spin_config, services_config, move |env| {
        preboot(&test_name, env)
    });
    TestEnvironment::up(config, init_env)
}

/// Assert that a request to the spin server returns the expected status and body
pub fn assert_spin_request<B: Into<reqwest::Body>>(
    spin: &mut testing_framework::runtimes::spin_cli::SpinCli,
    request: http::Request<'_, B>,
    expected: http::Response,
) -> testing_framework::TestResult<anyhow::Error> {
    let uri = request.path;
    let r = spin.make_http_request(request)?;
    let status = r.status();
    let expected_status = expected.status();
    let headers = r.headers();
    let expected_headers = expected.headers();
    let body_string = r
        .text()
        .unwrap_or_else(|_| format!("{}", TruncatedSlice(&r.body())));
    let expected_body_string = expected
        .text()
        .unwrap_or_else(|_| format!("{}", TruncatedSlice(&expected.body())));
    if status != expected.status() {
        let stderr = spin.stderr();
        return Err(testing_framework::TestError::Failure(anyhow::anyhow!(
            "Expected status {expected_status} for {uri} but got {status}\nBody: '{body_string}'\nStderr: '{stderr}'",
        )));
    }
    let wrong_headers: std::collections::HashMap<_, _> = expected_headers
        .iter()
        .filter(|(ek, ev)| headers.get(*ek).map(String::as_str) != Some(ev))
        .collect();
    if !wrong_headers.is_empty() {
        return Err(testing_framework::TestError::Failure(anyhow::anyhow!(
            "Expected headers {headers:?}  to contain {wrong_headers:?}\nBody:\n{body_string}"
        )));
    }
    if r.body() != expected.body() {
        return Err(testing_framework::TestError::Failure(anyhow::anyhow!(
            "expected body chunk '{expected_body_string}', got '{body_string}'",
        )));
    }
    Ok(())
}

struct TruncatedSlice<'a, T>(&'a [T]);

impl<T: std::fmt::Display> std::fmt::Display for TruncatedSlice<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[")?;
        for item in self.0.iter().take(10) {
            f.write_fmt(format_args!("{item}, "))?;
        }
        if self.0.len() > 10 {
            f.write_fmt(format_args!("...({} more items)]", self.0.len() - 10))?;
        } else {
            f.write_str("]")?;
        }
        Ok(())
    }
}

/// Get the test environment ready to run a test
fn preboot(
    test: &str,
    env: &mut TestEnvironment<testing_framework::runtimes::spin_cli::SpinCli>,
) -> anyhow::Result<()> {
    let test_path = format!("tests/testcases/{test}");
    for file in std::fs::read_dir(test_path)? {
        let file = file?;
        let path = file.path();
        if path.is_dir() {
            env.copy_into(&path, path.file_name().unwrap())?;
        } else {
            let content = std::fs::read(&path)
                .with_context(|| format!("failed to read file '{}' for copying", path.display()))?;
            match String::from_utf8(content) {
                Ok(content) => {
                    let mut template = EnvTemplate::new(content)?;
                    template.substitute(env, |name| {
                        Some(PathBuf::from(test_components::path(name)?))
                    })?;
                    env.write_file(path.file_name().unwrap(), template.contents())?;
                }
                Err(e) => {
                    env.write_file(path.file_name().unwrap(), e.as_bytes())?;
                }
            };
        }
    }

    Ok(())
}

/// Run a smoke test against a `spin new` http template
pub fn http_smoke_test_template(
    template_name: &str,
    template_url: Option<TemplateSource>,
    plugins: &[&str],
    prebuild_hook: impl FnOnce(&mut TestEnvironment<()>) -> anyhow::Result<()>,
    build_env_vars: HashMap<String, String>,
    expected_body: &str,
) -> anyhow::Result<()> {
    http_smoke_test_template_with_route(
        template_name,
        template_url,
        plugins,
        prebuild_hook,
        build_env_vars,
        "/",
        expected_body,
    )
}

/// Run a smoke test against a given http route for a `spin new` http template
// TODO: refactor this function to not take so many arguments
#[allow(clippy::too_many_arguments)]
pub fn http_smoke_test_template_with_route(
    template_name: &str,
    template_url: Option<TemplateSource>,
    plugins: &[&str],
    prebuild_hook: impl FnOnce(&mut TestEnvironment<()>) -> anyhow::Result<()>,
    build_env_vars: HashMap<String, String>,
    route: &str,
    expected_body: &str,
) -> anyhow::Result<()> {
    let mut env = bootstrap_smoke_test(
        ServicesConfig::none(),
        template_url,
        plugins,
        template_name,
        |_| Ok(Vec::new()),
        prebuild_hook,
        build_env_vars,
        |_| Ok(Vec::new()),
        testing_framework::runtimes::SpinAppType::Http,
    )?;

    assert_spin_request(
        env.runtime_mut(),
        http::Request::new(http::Method::Get, route),
        http::Response::full(200, Default::default(), expected_body),
    )?;

    Ok(())
}

/// Run a smoke test for a `spin new` redis template
#[cfg(feature = "extern-dependencies-tests")]
#[allow(dependency_on_unit_never_type_fallback)]
pub fn redis_smoke_test_template(
    template_name: &str,
    template_url: Option<TemplateSource>,
    plugins: &[&str],
    new_app_args: impl FnOnce(u16) -> Vec<String>,
    prebuild_hook: impl FnOnce(&mut TestEnvironment<()>) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    use redis::Commands;
    let mut env = bootstrap_smoke_test(
        test_environment::services::ServicesConfig::new(vec!["redis"])?,
        template_url,
        plugins,
        template_name,
        |env| {
            let redis_port = env
                .services_mut()
                .get_port(6379)?
                .context("no redis port was exposed by test services")?;
            Ok(new_app_args(redis_port))
        },
        prebuild_hook,
        HashMap::default(),
        |_| Ok(Vec::new()),
        testing_framework::runtimes::SpinAppType::Redis,
    )?;
    let redis_port = env
        .get_port(6379)?
        .context("no redis port was exposed by test services")?;
    let mut client = redis::Client::open(format!("redis://localhost:{redis_port}"))?;
    let mut conn = client.get_connection()?;
    let mut pubsub = conn.as_pubsub();
    pubsub.subscribe("redis-channel")?;
    client.publish::<_, _, ()>("redis-channel", "hello from redis")?;

    // Wait for the message to be received (as an approximation for when Spin receives the message)
    let _ = pubsub.get_message()?;
    // Leave some time for the message to be processed
    std::thread::sleep(std::time::Duration::from_millis(100));

    let stderr = env.runtime_mut().stderr();
    assert!(stderr.contains("hello from redis"));

    Ok(())
}

static TEMPLATE_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub enum TemplateSource {
    Url {
        repo: &'static str,
        branch: Option<&'static str>,
    },
    Local,
}

/// Bootstrap a test environment for a smoke test
// TODO: refactor this function to not take so many arguments
#[allow(clippy::too_many_arguments)]
pub fn bootstrap_smoke_test(
    services: ServicesConfig,
    template_url: Option<TemplateSource>,
    plugins: &[&str],
    template_name: &str,
    new_app_args: impl FnOnce(&mut TestEnvironment<()>) -> anyhow::Result<Vec<String>>,
    prebuild_hook: impl FnOnce(&mut TestEnvironment<()>) -> anyhow::Result<()>,
    build_env_vars: HashMap<String, String>,
    spin_up_args: impl FnOnce(&mut TestEnvironment<()>) -> anyhow::Result<Vec<String>>,
    spin_app_type: testing_framework::runtimes::SpinAppType,
) -> anyhow::Result<TestEnvironment<testing_framework::runtimes::spin_cli::SpinCli>> {
    let mut env: TestEnvironment<()> = TestEnvironment::boot(services)?;

    let this_dir = std::env::current_dir()?.display().to_string();

    let template_source = template_url.unwrap_or(TemplateSource::Url {
        repo: "https://github.com/spinframework/spin",
        branch: None,
    });
    let install_loc_args = match template_source {
        TemplateSource::Url { repo, branch: None } => &["--git", repo],
        TemplateSource::Url {
            repo,
            branch: Some(br),
        } => &["--git", repo, "--branch", br][..],
        TemplateSource::Local => &["--dir", &this_dir],
    };

    let mut template_install = std::process::Command::new(spin_binary());
    template_install.args(["templates", "install", "--update"]);
    template_install.args(install_loc_args);

    // We need to serialize template installs since they can't be run in parallel
    {
        let _guard = TEMPLATE_MUTEX.lock().unwrap();
        env.run_in(&mut template_install)?;
    }

    if !plugins.is_empty() {
        let mut plugin_update = std::process::Command::new(spin_binary());
        plugin_update.args(["plugin", "update"]);
        if let Err(e) = env.run_in(&mut plugin_update) {
            // We treat plugin updates as best efforts since it only needs to be run once
            if !e
                .to_string()
                .contains("update operation is already in progress")
            {
                return Err(e);
            }
        }
    }
    for plugin in plugins {
        let mut plugin_install = std::process::Command::new(spin_binary());
        plugin_install.args(["plugin", "install", plugin, "--yes"]);
        env.run_in(&mut plugin_install)?;
    }
    let mut new_app = std::process::Command::new(spin_binary());
    new_app
        .args([
            "new",
            "test",
            "-t",
            template_name,
            "-o",
            ".",
            "--accept-defaults",
            "--allow-overwrite",
        ])
        .args(new_app_args(&mut env)?);
    env.run_in(&mut new_app)?;
    prebuild_hook(&mut env)?;
    let path = std::env::var("PATH").unwrap_or_default();
    let path = if path.is_empty() {
        spin_binary().parent().unwrap().display().to_string()
    } else {
        format!("{path}:{}", spin_binary().parent().unwrap().display())
    };
    let mut build = std::process::Command::new(spin_binary());
    // Ensure `spin` is on the path
    build.env("PATH", &path).args(["build"]);
    build_env_vars.iter().for_each(|(key, value)| {
        if key == "PATH" {
            let mut custom_path = value.to_owned();
            if value.starts_with('.') {
                let current_dir = env.path();
                current_dir
                    .join(value)
                    .to_str()
                    .unwrap_or_default()
                    .clone_into(&mut custom_path);
            }
            build.env(key, format!("{custom_path}:{path}"));
        } else {
            build.env(key, value);
        }
    });
    env.run_in(&mut build)?;
    let spin = testing_framework::runtimes::spin_cli::SpinCli::start(
        SpinConfig {
            binary_path: spin_binary(),
            spin_up_args: spin_up_args(&mut env)?,
            app_type: spin_app_type,
        },
        &mut env,
    )?;
    let env = env.start_runtime(spin)?;
    Ok(env)
}

/// Get the spin binary path
pub fn spin_binary() -> PathBuf {
    env!("CARGO_BIN_EXE_spin").into()
}
