use anyhow::Context as _;
use testing_framework::runtimes::spin_cli::{SpinCli, SpinConfig};

/// Run a single conformance test against the supplied spin binary.
pub fn run_test(
    test: conformance_tests::Test,
    spin_binary: &std::path::Path,
) -> anyhow::Result<()> {
    let mut services = Vec::new();
    for precondition in test.config.preconditions {
        match precondition {
            conformance_tests::config::Precondition::HttpEcho => {
                services.push("http-echo");
            }
            conformance_tests::config::Precondition::TcpEcho => {
                services.push("tcp-echo");
            }
            conformance_tests::config::Precondition::Redis => {
                if should_run_docker_based_tests() {
                    services.push("redis")
                } else {
                    // Skip the test if docker is not installed.
                    return Ok(());
                }
            }
            conformance_tests::config::Precondition::Mqtt => {
                if should_run_docker_based_tests() {
                    services.push("mqtt")
                } else {
                    // Skip the test if docker is not installed.
                    return Ok(());
                }
            }
            conformance_tests::config::Precondition::Postgres => {
                if should_run_docker_based_tests() {
                    services.push("postgres")
                } else {
                    // Skip the test if docker is not installed.
                    return Ok(());
                }
            }
            conformance_tests::config::Precondition::KeyValueStore(_) => {}
            conformance_tests::config::Precondition::Sqlite => {}
        }
    }
    let env_config = SpinCli::config(
        SpinConfig {
            binary_path: spin_binary.to_owned(),
            spin_up_args: Vec::new(),
            app_type: testing_framework::runtimes::SpinAppType::Http,
        },
        test_environment::services::ServicesConfig::new(services)?,
        move |e| {
            let mut manifest =
                test_environment::manifest_template::EnvTemplate::from_file(&test.manifest)?;
            manifest.substitute(e, |_| None)?;
            e.write_file("spin.toml", manifest.contents())?;
            e.copy_into(&test.component, test.component.file_name().unwrap())?;
            Ok(())
        },
    );
    let mut env = test_environment::TestEnvironment::up(env_config, |_| Ok(()))?;
    for invocation in test.config.invocations {
        let conformance_tests::config::Invocation::Http(mut invocation) = invocation;
        invocation.request.substitute_from_env(&mut env)?;
        let spin = env.runtime_mut();
        let actual = invocation
            .request
            .send(|request| spin.make_http_request(request))?;

        conformance_tests::assertions::assert_response(&invocation.response, &actual)
            .with_context(|| {
                format!(
                    "Failed assertion.\nstdout: {}\nstderr: {}",
                    spin.stdout().to_owned(),
                    spin.stderr()
                )
            })?;
    }
    Ok(())
}

/// Whether or not docker is installed on the system.
fn should_run_docker_based_tests() -> bool {
    std::env::var("SPIN_CONFORMANCE_TESTS_DOCKER_OPT_OUT").is_err()
}
