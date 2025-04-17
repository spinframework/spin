mod testcases;

mod integration_tests {
    use sha2::Digest;
    use std::collections::HashMap;
    use test_environment::{
        http::{Method, Request, Response},
        services::ServicesConfig,
    };
    use testing_framework::runtimes::{spin_cli::SpinConfig, SpinAppType};

    use super::testcases::{
        assert_spin_request, bootstap_env, http_smoke_test_template, run_test, spin_binary,
    };
    use anyhow::Context;

    /// Helper macro to assert that a condition is true eventually
    #[cfg(feature = "extern-dependencies-tests")]
    macro_rules! assert_eventually {
        ($e:expr, $t:expr) => {
            let mut i = 0;
            loop {
                if $e {
                    break;
                } else if i > $t * 10 {
                    assert!($e);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                i += 1;
            }
        };
    }

    #[test]
    /// Test that the --key-value flag works as expected
    fn key_value_cli_flag() -> anyhow::Result<()> {
        let test_key = uuid::Uuid::new_v4().to_string();
        let test_value = uuid::Uuid::new_v4().to_string();
        run_test(
            "key-value",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: vec!["--key-value".into(), format!("{test_key}={test_value}")],
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                assert_spin_request(
                    spin,
                    Request::new(Method::Get, &format!("/test?key={test_key}")),
                    Response::new_with_body(200, test_value),
                )
            },
        )?;
        Ok(())
    }

    #[test]
    /// Test that basic http trigger support works
    fn http_smoke_test() -> anyhow::Result<()> {
        run_test(
            "http-smoke-test",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                assert_spin_request(
                    spin,
                    Request::new(Method::Get, "/hello"),
                    Response::new_with_body(200, "I'm a teapot"),
                )?;
                assert_spin_request(
                    spin,
                    Request::new(Method::Get, "/hello/wildcards/should/be/handled"),
                    Response::new_with_body(200, "I'm a teapot"),
                )?;
                assert_spin_request(
                    spin,
                    Request::new(Method::Get, "/thishsouldfail"),
                    Response::new(404),
                )?;
                assert_spin_request(
                    spin,
                    Request::new(Method::Get, "/hello/test-placement"),
                    Response::new_with_body(200, "text for test"),
                )
            },
        )?;

        Ok(())
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    #[allow(dependency_on_unit_never_type_fallback)]
    /// Test that basic redis trigger support works
    fn redis_smoke_test() -> anyhow::Result<()> {
        use anyhow::Context;
        use redis::Commands;
        run_test(
            "redis-smoke-test",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Redis,
            },
            ServicesConfig::new(vec!["redis"])?,
            move |env| {
                let redis_port = env
                    .services_mut()
                    .get_port(6379)?
                    .context("no redis port was exposed by test services")?;

                let mut redis = redis::Client::open(format!("redis://localhost:{redis_port}"))
                    .context("could not connect to redis in test")?;
                redis
                    .publish("my-channel", "msg-from-test")
                    .context("could not publish test message to redis")?;
                assert_eventually!(
                    {
                        match env.read_file(".spin/logs/hello_stdout.txt") {
                            Ok(logs) => {
                                let logs = String::from_utf8_lossy(&logs);
                                logs.contains("Got message: 'msg-from-test'")
                            }
                            Err(e)
                                if e.downcast_ref()
                                    .map(|e: &std::io::Error| {
                                        e.kind() == std::io::ErrorKind::NotFound
                                    })
                                    .unwrap_or_default() =>
                            {
                                false
                            }
                            Err(e) => {
                                return Err(
                                    anyhow::anyhow!("could not read stdout file: {e}").into()
                                )
                            }
                        }
                    },
                    2
                );
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    /// Test that basic otel tracing works
    fn otel_smoke_test() -> anyhow::Result<()> {
        use anyhow::Context;

        use crate::testcases::run_test_inited;
        run_test_inited(
            "otel-smoke-test",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::new(vec!["jaeger"])?,
            |env| {
                let otel_port = env
                    .services_mut()
                    .get_port(4318)?
                    .context("expected a port for Jaeger")?;
                env.set_env_var(
                    "OTEL_EXPORTER_OTLP_ENDPOINT",
                    format!("http://localhost:{}", otel_port),
                );
                Ok(())
            },
            move |env| {
                let spin = env.runtime_mut();
                assert_spin_request(
                    spin,
                    Request::new(Method::Get, "/hello"),
                    Response::new_with_body(200, "Hello World!\n"),
                )?;

                assert_eventually!(
                    {
                        let jaeger_port = env
                            .services_mut()
                            .get_port(16686)?
                            .context("no jaeger port was exposed by test services")?;
                        let url = format!("http://localhost:{jaeger_port}/api/traces?service=spin");
                        match reqwest::blocking::get(&url).context("failed to get jaeger traces")? {
                            resp if resp.status().is_success() => {
                                let traces: serde_json::Value =
                                    resp.json().context("failed to parse jaeger traces")?;
                                let traces =
                                    traces.get("data").context("jaeger traces has no data")?;
                                let traces =
                                    traces.as_array().context("jaeger traces is not an array")?;
                                !traces.is_empty()
                            }
                            _resp => {
                                eprintln!("failed to get jaeger traces:");
                                false
                            }
                        }
                    },
                    20
                );
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    /// Test dynamic environment variables
    fn dynamic_env_test() -> anyhow::Result<()> {
        run_test(
            "dynamic-env-test",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: vec!["--env".to_owned(), "foo=bar".to_owned()],
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                assert_spin_request(
                    spin,
                    Request::new(Method::Get, "/env"),
                    Response::full(
                        200,
                        [
                            ("env_some_key".to_owned(), "some_value".to_owned()),
                            ("env_foo".to_owned(), "bar".to_owned()),
                        ]
                        .into_iter()
                        .collect(),
                        "I'm a teapot",
                    ),
                )?;
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    /// Test that mounting works properly
    fn assets_routing_test() -> anyhow::Result<()> {
        run_test(
            "assets-test",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                let mut assert_file = |name: &str, content: &str| {
                    assert_spin_request(
                        spin,
                        Request::new(Method::Get, &format!("/static/thisshouldbemounted/{name}")),
                        Response::new_with_body(200, content),
                    )
                };
                let mut assert_file_content_eq_name =
                    |name: &str| assert_file(name, &format!("{name}\n"));

                assert_file_content_eq_name("1")?;
                assert_file_content_eq_name("2")?;
                assert_file_content_eq_name("3")?;
                assert_file("empty", "")?;
                assert_file("one-byte", "{")?;

                let mut assert_not_found = |path: &str| {
                    assert_spin_request(
                        spin,
                        Request::new(Method::Get, &format!("/static/{path}")),
                        Response::new_with_body(404, "Not Found"),
                    )
                };

                assert_not_found("donotmount/a")?;
                assert_not_found("thisshouldbemounted/thisshouldbeexcluded/4")?;
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    /// Test a bunch of old apps for backwards compatibility
    fn legacy_apps() -> anyhow::Result<()> {
        run_test(
            "legacy-apps-test",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                let mut test = |lang: &str, body: &str| {
                    assert_spin_request(
                        spin,
                        Request::new(Method::Get, &format!("/{lang}")),
                        Response::new_with_body(200, body),
                    )
                };

                test("golang", "Hello Fermyon!\n")?;
                test("rust", "Hello, Fermyon")?;
                test("javascript", "Hello from JS-SDK")?;
                test("typescript", "Hello from TS-SDK")?;
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    fn bad_build_test() -> anyhow::Result<()> {
        let mut env = bootstap_env(
            "error",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::None,
            },
            ServicesConfig::none(),
            |env| {
                // Since this test asserts exact stderr output, disable logging
                env.set_env_var("RUST_LOG", "off");
                Ok(())
            },
        )?;

        let expected = "Error: No triggers in app\n";
        assert_eq!(env.runtime_mut().stderr(), expected);

        Ok(())
    }

    #[test]
    fn outbound_http_works() -> anyhow::Result<()> {
        run_test(
            "outbound-http-to-same-app",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                assert_spin_request(
                    spin,
                    Request::new(Method::Get, "/outbound-allowed"),
                    Response::new_with_body(200, "Hello World!\n"),
                )?;
                assert_spin_request(
                    spin,
                    Request::new(Method::Get, "/outbound-allowed-alt"),
                    Response::new_with_body(200, "Hello World!\n"),
                )?;

                assert_spin_request(
                    spin,
                    Request::new(Method::Get, "/outbound-not-allowed"),
                    Response::new_with_body(
                        500,
                        "Error::UnexpectedError(\"ErrorCode::HttpRequestDenied\")",
                    ),
                )?;

                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    fn http_rust_template_smoke_test() -> anyhow::Result<()> {
        http_smoke_test_template(
            "http-rust",
            None,
            None,
            &[],
            |_| Ok(()),
            HashMap::default(),
            "Hello World!",
        )
    }

    #[test]
    fn test_simple_rust_local() -> anyhow::Result<()> {
        run_test(
            "simple-test",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            |env| {
                let spin = env.runtime_mut();
                let mut ensure_success = |uri, expected_status, expected_body| {
                    let request = Request::new(Method::Get, uri);
                    assert_spin_request(
                        spin,
                        request,
                        Response::new_with_body(expected_status, expected_body),
                    )
                };
                ensure_success("/hello", 200, "I'm a teapot")?;
                ensure_success("/hello/wildcards/should/be/handled", 200, "I'm a teapot")?;
                ensure_success("/thisshouldfail", 404, "")?;
                ensure_success("/hello/test-placement", 200, "text for test")?;
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    fn test_duplicate_rust_local() -> anyhow::Result<()> {
        run_test(
            "simple-double-test",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            |env| {
                let spin = env.runtime_mut();
                let mut ensure_success = |uri, expected_status, expected_body| {
                    let request = Request::new(Method::Get, uri);
                    assert_spin_request(
                        spin,
                        request,
                        Response::new_with_body(expected_status, expected_body),
                    )
                };
                ensure_success("/route1", 200, "I'm a teapot")?;
                ensure_success("/route2", 200, "I'm a teapot")?;
                ensure_success("/thisshouldfail", 404, "")?;
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn test_vault_config_provider() -> anyhow::Result<()> {
        use std::collections::HashMap;

        use crate::testcases::run_test_inited;
        const VAULT_ROOT_TOKEN: &str = "root";
        run_test_inited(
            "vault-variables-test",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: vec!["--runtime-config-file".into(), "runtime_config.toml".into()],
                app_type: SpinAppType::Http,
            },
            ServicesConfig::new(vec!["vault"])?,
            |env| {
                // Vault can take a few moments to be ready
                std::thread::sleep(std::time::Duration::from_secs(2));
                let http_client = reqwest::blocking::Client::new();
                let body: HashMap<String, HashMap<String, String>> =
                    serde_json::from_value(serde_json::json!(
                        {
                            "data": {
                                "value": "test_password"
                            }

                        }
                    ))
                    .unwrap();
                let status = http_client
                    .post(format!(
                        "http://localhost:{}/v1/secret/data/password",
                        env.get_port(8200)?.context("vault port not found")?
                    ))
                    .header("X-Vault-Token", VAULT_ROOT_TOKEN)
                    .json(&body)
                    .send()
                    .context("failed to send request to Vault")?
                    .status();
                assert_eq!(status, 200);
                Ok(())
            },
            |env| {
                let spin = env.runtime_mut();
                let request = Request::new(Method::Get, "/");
                assert_spin_request(
                    spin,
                    request,
                    Response::new_with_body(200, "Hello! Got password test_password"),
                )?;
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    #[ignore = "https://github.com/spinframework/spin/issues/2457"]
    // TODO: Check why python is not picking up the spin_sdk from site_packages
    // Currently installing to the local directory to get around it.
    fn http_python_template_smoke_test() -> anyhow::Result<()> {
        let prebuild = |env: &mut test_environment::TestEnvironment<_>| {
            let mut tidy = std::process::Command::new("pip3");
            tidy.args(["install", "-r", "requirements.txt", "-t", "."]);
            env.run_in(&mut tidy)?;
            Ok(())
        };
        let env_vars = HashMap::from([
            ("PATH".to_owned(), "./bin/".to_owned()),
            ("PYTHONPATH".to_owned(), ".".to_owned()),
        ]);
        http_smoke_test_template(
            "http-py",
            Some("https://github.com/spinframework/spin-python-sdk"),
            Some("v2.0"),
            &[],
            prebuild,
            env_vars,
            "Hello from Python!",
        )
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn http_c_template_smoke_test() -> anyhow::Result<()> {
        http_smoke_test_template(
            "http-c",
            None,
            None,
            &[],
            |_| Ok(()),
            HashMap::default(),
            "Hello from WAGI/1\n",
        )
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn http_go_template_smoke_test() -> anyhow::Result<()> {
        let prebuild = |env: &mut test_environment::TestEnvironment<_>| {
            let mut tidy = std::process::Command::new("go");
            tidy.args(["mod", "tidy"]);
            env.run_in(&mut tidy)?;
            Ok(())
        };
        http_smoke_test_template(
            "http-go",
            None,
            None,
            &[],
            prebuild,
            HashMap::default(),
            "Hello World!\n",
        )
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn http_js_template_smoke_test() -> anyhow::Result<()> {
        let prebuild = |env: &mut test_environment::TestEnvironment<_>| {
            let mut tidy = std::process::Command::new("npm");
            tidy.args(["install"]);
            env.run_in(&mut tidy)?;
            Ok(())
        };
        http_smoke_test_template(
            "http-js",
            Some("https://github.com/spinframework/spin-js-sdk"),
            None,
            &[],
            prebuild,
            HashMap::default(),
            "Hello, Spin!",
        )
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn http_ts_template_smoke_test() -> anyhow::Result<()> {
        let prebuild = |env: &mut test_environment::TestEnvironment<_>| {
            let mut tidy = std::process::Command::new("npm");
            tidy.args(["install"]);
            env.run_in(&mut tidy)?;
            Ok(())
        };
        http_smoke_test_template(
            "http-ts",
            Some("https://github.com/spinframework/spin-js-sdk"),
            None,
            &[],
            prebuild,
            HashMap::default(),
            "Hello, Spin!",
        )
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    #[cfg(feature = "extern-dependencies-tests")]
    fn http_grain_template_smoke_test() -> anyhow::Result<()> {
        http_smoke_test_template(
            "http-grain",
            None,
            None,
            &[],
            |_| Ok(()),
            HashMap::default(),
            "Hello, World\n",
        )
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn http_zig_template_smoke_test() -> anyhow::Result<()> {
        http_smoke_test_template(
            "http-zig",
            None,
            None,
            &[],
            |_| Ok(()),
            HashMap::default(),
            "Hello World!\n",
        )
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn http_php_template_smoke_test() -> anyhow::Result<()> {
        super::testcases::http_smoke_test_template_with_route(
            "http-php",
            None,
            None,
            &[],
            |_| Ok(()),
            HashMap::default(),
            "/index.php",
            "Hello World!",
        )
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn redis_go_template_smoke_test() -> anyhow::Result<()> {
        super::testcases::redis_smoke_test_template(
            "redis-go",
            None,
            None,
            &[],
            |port| {
                vec![
                    "--value".into(),
                    "redis-channel=redis-channel".into(),
                    "--value".into(),
                    format!("redis-address=redis://localhost:{port}"),
                ]
            },
            |env| {
                let mut tidy = std::process::Command::new("go");
                tidy.args(["mod", "tidy"]);
                env.run_in(&mut tidy)?;
                Ok(())
            },
        )
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn redis_rust_template_smoke_test() -> anyhow::Result<()> {
        super::testcases::redis_smoke_test_template(
            "redis-rust",
            None,
            None,
            &[],
            |port| {
                vec![
                    "--value".into(),
                    "redis-channel=redis-channel".into(),
                    "--value".into(),
                    format!("redis-address=redis://localhost:{port}"),
                ]
            },
            |_| Ok(()),
        )
    }

    #[test]
    #[cfg(feature = "extern-dependencies-tests")]
    fn registry_works() -> anyhow::Result<()> {
        let services = ServicesConfig::new(vec!["registry"])?;
        let spin_up_args = |env: &mut test_environment::TestEnvironment<()>| {
            let registry_url = format!(
                "localhost:{}/spin-e2e-tests/registry-works/v1",
                env.get_port(5000)?
                    .context("no registry port was exposed by test services")?
            );
            let mut registry_push = std::process::Command::new(spin_binary());
            registry_push.args(["registry", "push", &registry_url, "--insecure"]);
            env.run_in(&mut registry_push)?;
            Ok(vec![
                "--from-registry".into(),
                registry_url,
                "--insecure".into(),
            ])
        };
        let mut env = super::testcases::bootstrap_smoke_test(
            services,
            None,
            None,
            &[],
            "http-rust",
            |_| Ok(Vec::new()),
            |_| Ok(()),
            HashMap::default(),
            spin_up_args,
            SpinAppType::Http,
        )?;
        assert_spin_request(
            env.runtime_mut(),
            Request::new(Method::Get, "/"),
            Response::new_with_body(200, "Hello World!"),
        )?;
        Ok(())
    }

    #[test]
    fn test_wasi_http_rc_11_10() -> anyhow::Result<()> {
        test_wasi_http_rc("wasi-http-0.2.0-rc-2023-11-10")
    }

    #[test]
    fn test_wasi_http_0_2_0() -> anyhow::Result<()> {
        test_wasi_http_rc("wasi-http-0.2.0")
    }

    fn test_wasi_http_rc(test_name: &str) -> anyhow::Result<()> {
        let body = "So rested he by the Tumtum tree";

        run_test(
            test_name,
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::new(vec!["http-echo"])?,
            move |env| {
                let port = env
                    .get_port(80)?
                    .context("no http-echo port was exposed by test services")?;
                assert_spin_request(
                    env.runtime_mut(),
                    Request::full(
                        Method::Get,
                        "/",
                        &[("url", &format!("http://127.0.0.1:{port}/",))],
                        Some(body.as_bytes()),
                    ),
                    Response::new_with_body(200, "Hello, world!"),
                )?;
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    fn spin_up_gives_help_on_new_app() -> anyhow::Result<()> {
        let mut env = test_environment::TestEnvironment::<()>::boot(ServicesConfig::none())?;

        // We still don't see full help if there are no components.
        let toml_text = r#"spin_version = "1"
name = "unbuilt"
trigger = { type = "http" }
version = "0.1.0"
[[component]]
id = "unbuilt"
source = "fake.wasm"
[component.trigger]
route = "/..."
"#;
        env.write_file("spin.toml", toml_text)?;
        env.write_file("fake.wasm", [])?;

        testing_framework::runtimes::spin_cli::SpinCli::start(
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::None,
            },
            &mut env,
        )?;

        let mut up = std::process::Command::new(spin_binary());
        up.args(["up", "--help"]);
        let output = env.run_in(&mut up)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("--quiet"));
        assert!(stdout.contains("--listen"));

        Ok(())
    }

    #[test]
    fn spin_up_help_build_does_not_build() -> anyhow::Result<()> {
        let mut env = test_environment::TestEnvironment::<()>::boot(ServicesConfig::none())?;

        // We still don't see full help if there are no components.
        let toml_text = r#"spin_version = "1"
name = "unbuilt"
trigger = { type = "http" }
version = "0.1.0"
[[component]]
id = "unbuilt"
source = "fake.wasm"
[component.build]
command = "echo should_not_print"
[component.trigger]
route = "/..."
"#;
        env.write_file("spin.toml", toml_text)?;
        env.write_file("fake.wasm", [])?;

        testing_framework::runtimes::spin_cli::SpinCli::start(
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::None,
            },
            &mut env,
        )?;

        let mut up = std::process::Command::new(spin_binary());
        up.args(["up", "--build", "--help"]);
        let output = env.run_in(&mut up)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("--quiet"));
        assert!(stdout.contains("--listen"));
        assert!(!stdout.contains("should_not_print"));

        Ok(())
    }

    // TODO: Test on Windows
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_spin_plugin_install_command() -> anyhow::Result<()> {
        let env = test_environment::TestEnvironment::<()>::boot(ServicesConfig::none())?;

        let path_to_test_dir = std::env::current_dir()?;
        let file_url = format!(
            "file:{}/tests/testcases/plugin/example.tar.gz",
            path_to_test_dir.to_str().unwrap()
        );
        let mut plugin_manifest_json = serde_json::json!(
        {
            "name": "example",
            "description": "A description of the plugin.",
            "homepage": "www.example.com",
            "version": "0.2.0",
            "spinCompatibility": ">=0.5",
            "license": "MIT",
            "packages": [
                {
                    "os": "linux",
                    "arch": "amd64",
                    "url": file_url,
                    "sha256": "f7a5a8c16a94fe934007f777a1bf532ef7e42b02133e31abf7523177b220a1ce"
                },
                {
                    "os": "macos",
                    "arch": "aarch64",
                    "url": file_url,
                    "sha256": "f7a5a8c16a94fe934007f777a1bf532ef7e42b02133e31abf7523177b220a1ce"
                },
                {
                    "os": "macos",
                    "arch": "amd64",
                    "url": file_url,
                    "sha256": "f7a5a8c16a94fe934007f777a1bf532ef7e42b02133e31abf7523177b220a1ce"
                }
            ]
        });
        let contents = serde_json::to_string(&plugin_manifest_json).unwrap();
        env.write_file("example-plugin-manifest.json", contents)?;

        // Install plugin
        let mut install = std::process::Command::new(spin_binary());
        install
            .args([
                "plugins",
                "install",
                "--file",
                "example-plugin-manifest.json",
                "--yes",
            ])
            // Ensure that spin installs the plugins into the temporary directory
            .env("SPIN_DATA_DIR", "./plugins");
        env.run_in(&mut install)?;

        /// Make sure that the plugin is uninstalled after the test
        struct Uninstaller<'a>(&'a test_environment::TestEnvironment<()>);
        impl Drop for Uninstaller<'_> {
            fn drop(&mut self) {
                let mut uninstall = std::process::Command::new(spin_binary());
                uninstall.args(["plugins", "uninstall", "example"]);
                self.0.run_in(&mut uninstall).unwrap();
            }
        }
        let _u = Uninstaller(&env);

        let mut install = std::process::Command::new(spin_binary());
        install
            .args([
                "plugins",
                "install",
                "--file",
                "example-plugin-manifest.json",
                "--yes",
            ])
            // Ensure that spin installs the plugins into the temporary directory
            .env("SPIN_DATA_DIR", "./plugins");
        env.run_in(&mut install)?;

        let mut execute = std::process::Command::new(spin_binary());
        execute.args(["example"]).env("SPIN_DATA_DIR", "./plugins");
        let output = env.run_in(&mut execute)?;

        // Verify plugin successfully wrote to output file
        assert!(std::str::from_utf8(&output.stdout)?
            .trim()
            .contains("This is an example Spin plugin!"));

        // Upgrade plugin to newer version
        *plugin_manifest_json.get_mut("version").unwrap() = serde_json::json!("0.2.1");
        env.write_file(
            "example-plugin-manifest.json",
            serde_json::to_string(&plugin_manifest_json).unwrap(),
        )?;
        let mut upgrade = std::process::Command::new(spin_binary());
        upgrade
            .args([
                "plugins",
                "upgrade",
                "example",
                "--file",
                "example-plugin-manifest.json",
                "--yes",
            ])
            .env("SPIN_DATA_DIR", "./plugins");
        env.run_in(&mut upgrade)?;

        // Check plugin version
        let installed_manifest = std::path::PathBuf::from("plugins")
            .join("plugins")
            .join("manifests")
            .join("example.json");
        let manifest = String::from_utf8(env.read_file(installed_manifest)?).unwrap();
        assert!(manifest.contains("0.2.1"));

        Ok(())
    }

    // TODO: Test on Windows
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_cloud_plugin_autoinstall() -> anyhow::Result<()> {
        let env = test_environment::TestEnvironment::<()>::boot(ServicesConfig::none())?;

        let mut login = std::process::Command::new(spin_binary());
        login
            .args(["login", "--help"])
            // Ensure that spin installs the plugins into the temporary directory
            .env("SPIN_DATA_DIR", "./plugins");
        let output = env.run_in(&mut login)?;

        // Verify plugin successfully wrote to output file
        assert!(std::str::from_utf8(&output.stdout)?
            .trim()
            .contains("The `cloud` plugin is required. Installing now."));
        // Ensure login help info is displayed
        assert!(std::str::from_utf8(&output.stdout)?
            .trim()
            .contains("Log into Fermyon Cloud"));
        Ok(())
    }

    #[test]
    fn test_build_command() -> anyhow::Result<()> {
        do_test_build_command("tests/testcases/simple-build")
    }

    /// Build an app whose component `workdir` is a subdirectory.
    #[test]
    fn test_build_command_nested_workdir() -> anyhow::Result<()> {
        do_test_build_command("tests/testcases/nested-build")
    }

    /// Builds app in `dir` and verifies the build succeeded. Expects manifest
    /// in `spin.toml` inside `dir`.
    fn do_test_build_command(dir: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
        let dir = dir.as_ref();
        let manifest_file = dir.join("spin.toml");
        let manifest = spin_manifest::manifest_from_file(manifest_file)?;

        let sources = manifest
            .components
            .iter()
            .map(|(id, component)| {
                let spin_manifest::schema::v2::ComponentSource::Local(file) = &component.source
                else {
                    panic!(
                        "{}.{}: source is not a file reference",
                        manifest.application.name, id
                    )
                };
                (id, std::path::PathBuf::from(file))
            })
            .collect::<std::collections::HashMap<_, _>>();

        let env = test_environment::TestEnvironment::<()>::boot(ServicesConfig::none())?;
        env.copy_into(dir, ".")?;

        let mut build = std::process::Command::new(spin_binary());
        build.arg("build");
        env.run_in(&mut build)?;

        let mut missing_sources_count = 0;
        for (component_id, source) in sources.iter() {
            if env.read_file(source).is_err() {
                missing_sources_count += 1;
                println!(
                    "{}.{} source file '{}' was not generated by build",
                    manifest.application.name,
                    component_id,
                    source.display()
                );
            }
        }
        assert_eq!(missing_sources_count, 0);

        Ok(())
    }

    #[test]
    fn test_wasi_http_echo() -> anyhow::Result<()> {
        wasi_http_echo("echo".into(), None)
    }

    #[test]
    fn test_wasi_http_double_echo() -> anyhow::Result<()> {
        wasi_http_echo("double-echo".into(), Some("echo".into()))
    }

    fn wasi_http_echo(uri: String, url_header_uri: Option<String>) -> anyhow::Result<()> {
        let body_bytes = {
            // A sorta-random-ish megabyte
            let mut n = 0_u8;
            std::iter::repeat_with(move || {
                n = n.wrapping_add(251);
                n
            })
            .take(1024 * 1024)
            .collect::<Vec<_>>()
        };
        let chunks: Vec<_> = body_bytes
            .chunks(16 * 1024)
            .map(ToOwned::to_owned)
            .collect();
        let body = reqwest::Body::wrap_stream(futures::stream::iter(
            chunks
                .iter()
                .map(|chunk| Ok::<_, anyhow::Error>(bytes::Bytes::copy_from_slice(chunk)))
                .collect::<Vec<_>>(),
        ));

        run_test(
            "wasi-http-streaming",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                let mut headers = vec![("content-type", "application/octet-stream")];
                let url;
                if let Some(url_header_uri) = url_header_uri {
                    url = format!("{}/{url_header_uri}", spin.http_url().unwrap());
                    headers.push(("url", &url));
                }
                let uri = format!("/{uri}");
                let request = Request::full(Method::Post, &uri, &headers, Some(body));
                assert_spin_request(
                    spin,
                    request,
                    Response::full(
                        200,
                        [(
                            "content-type".to_owned(),
                            "application/octet-stream".to_owned(),
                        )]
                        .into_iter()
                        .collect(),
                        chunks,
                    ),
                )?;

                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    fn test_wasi_http_hash_all() -> anyhow::Result<()> {
        let requests = [
            ("/a", "’Twas brillig, and the slithy toves"),
            ("/b", "Did gyre and gimble in the wabe:"),
            ("/c", "All mimsy were the borogoves,"),
            ("/d", "And the mome raths outgrabe."),
        ]
        .into_iter()
        .collect::<HashMap<_, _>>();
        run_test(
            "wasi-http-streaming",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::new(vec!["http-responses-from-file"])?,
            move |env| {
                let service_url = format!(
                    "http://localhost:{}",
                    env.get_port(80)?
                        .context("no http-responses-from-file port was exposed by test services")?
                );
                // Write responses for the `http-responses-from-file` service to a config
                let response_config =
                    requests
                        .iter()
                        .fold(String::new(), |mut sum, (path, body)| {
                            sum.push_str(path);
                            sum.push(' ');
                            sum.push_str(body);
                            sum.push('\n');
                            sum
                        });
                env.write_file("responses.txt", response_config)?;

                // Make a request and get a response
                let headers = requests
                    .keys()
                    .map(|path| ("url", format!("{service_url}{path}")))
                    .collect::<Vec<_>>();
                let headers = headers
                    .iter()
                    .map(|(k, v)| (*k, v.as_str()))
                    .collect::<Vec<_>>();
                let request =
                    Request::full(Method::Get, "/hash-all", &headers, Option::<Vec<u8>>::None);
                let response = env.runtime_mut().make_http_request(request)?;

                // Assert the response
                assert_eq!(response.status(), 200);
                let body = response
                    .text()
                    .context("expected response body to be a string but wasn't")?;
                assert!(body.lines().count() == requests.len());
                for line in body.lines() {
                    let (url, hash) = line.split_once(": ").with_context(|| {
                        format!("expected string of form `<url>: <sha-256>`; got {line}")
                    })?;

                    let path = url.strip_prefix(&service_url).with_context(|| {
                        format!("expected string with service url {service_url}; got {url}")
                    })?;

                    let mut hasher = sha2::Sha256::new();
                    hasher.update(
                        requests
                            .get(path)
                            .with_context(|| format!("unexpected path: {path}"))?,
                    );
                    assert_eq!(hash, hex::encode(hasher.finalize()));
                }

                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    fn test_spin_inbound_http() -> anyhow::Result<()> {
        run_test(
            "spin-inbound-http",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                assert_spin_request(
                    spin,
                    Request::full(Method::Get, "/echo", &[], Some("Echo...")),
                    Response::new_with_body(200, "Echo..."),
                )?;
                assert_spin_request(
                    spin,
                    Request::full(
                        Method::Get,
                        "/assert-headers?k=v",
                        &[("X-Custom-Foo", "bar")],
                        Some(r#"{"x-custom-foo": "bar"}"#),
                    ),
                    Response::new(200),
                )?;
                Ok(())
            },
        )?;
        Ok(())
    }

    #[test]
    fn test_wagi_http() -> anyhow::Result<()> {
        run_test(
            "wagi-http",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                assert_spin_request(
                    spin,
                    Request::full(Method::Get, "/echo", &[], Some("Echo...")),
                    Response::new_with_body(200, "Echo..."),
                )?;
                assert_spin_request(
                    spin,
                    Request::full(
                        Method::Get,
                        "/assert-args?x=y",
                        &[],
                        Some(r#"["/assert-args", "x=y"]"#),
                    ),
                    Response::new(200),
                )?;
                assert_spin_request(
                    spin,
                    Request::full(
                        Method::Get,
                        "/assert-env",
                        &[("X-Custom-Foo", "bar")],
                        Some(r#"{"HTTP_X_CUSTOM_FOO": "bar"}"#),
                    ),
                    Response::new(200),
                )?;
                Ok(())
            },
        )?;

        Ok(())
    }

    #[test]
    fn test_http_routing() -> anyhow::Result<()> {
        run_test(
            "http-routing",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                assert_spin_request(
                    spin,
                    Request::full(Method::Get, "/users/42", &[], Some("")),
                    Response::new_with_body(200, "42:"),
                )?;
                assert_spin_request(
                    spin,
                    Request::full(Method::Get, "/users/42/", &[], Some("")),
                    Response::new_with_body(200, "42:/"),
                )?;
                assert_spin_request(
                    spin,
                    Request::full(Method::Get, "/users/42/hello", &[], Some("")),
                    Response::new_with_body(200, "42:/hello"),
                )?;
                assert_spin_request(
                    spin,
                    Request::full(Method::Get, "/users/42/hello/", &[], Some("")),
                    Response::new_with_body(200, "42:/hello/"),
                )?;
                Ok(())
            },
        )?;
        Ok(())
    }

    #[test]
    fn test_outbound_post() -> anyhow::Result<()> {
        run_test(
            "wasi-http-outbound-post",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::new(vec!["http-echo"])?,
            move |env| {
                let service_url = format!(
                    "http://localhost:{}",
                    env.get_port(80)?
                        .context("no http-echo port was exposed by test services")?
                );
                let headers = [("url", service_url.as_str())];
                let request: Request<'_, Vec<u8>> = Request::full(Method::Get, "/", &headers, None);
                let expected = Response::new_with_body(200, "Hello, world!");
                assert_spin_request(env.runtime_mut(), request, expected)
            },
        )?;

        Ok(())
    }

    // TODO: consider removing this test or rewriting
    // We are treating the timer trigger both as an example and as a test which makes maintenance hard.
    #[tokio::test]
    async fn test_timer_trigger() -> anyhow::Result<()> {
        use std::fs;
        use std::path::Path;

        const TIMER_TRIGGER_INTEGRATION_TEST: &str = "examples/spin-timer/app-example";
        const TIMER_TRIGGER_DIRECTORY: &str = "examples/spin-timer";

        let trigger_dir = Path::new(TIMER_TRIGGER_DIRECTORY);

        // Conventionally, we would do all Cargo builds of test code in build.rs, but this one can take a lot
        // longer than the tiny tests we normally build there (and it's pointless if the user just wants to build
        // Spin without running any tests) so we do it here instead.  Subsequent builds after the first one should
        // be very fast.
        assert!(std::process::Command::new("cargo")
            .arg("build")
            .arg("--target-dir")
            .arg(trigger_dir.join("target"))
            .arg("--manifest-path")
            .arg(trigger_dir.join("Cargo.toml"))
            .status()?
            .success());

        // Create a test plugin store so we don't modify the user's real one.
        let plugin_store_dir = Path::new(concat!(env!("OUT_DIR"), "/plugin-store"));
        let plugins_dir = plugin_store_dir.join("plugins");

        let plugin_dir = plugins_dir.join("trigger-timer");
        fs::create_dir_all(&plugin_dir)?;
        fs::copy(
            trigger_dir.join("target/debug/trigger-timer"),
            plugin_dir.join("trigger-timer"),
        )
        .context("could not copy plugin binary into plugin directory")?;

        let manifests_dir = plugins_dir.join("manifests");
        fs::create_dir_all(&manifests_dir)?;
        // Note that the hash and path in the manifest aren't accurate, but they won't be used anyway for this
        // test. We just need something that parses without throwing errors here.
        fs::copy(
            Path::new(TIMER_TRIGGER_DIRECTORY).join("trigger-timer.json"),
            manifests_dir.join("trigger-timer.json"),
        )
        .context("could not copy plugin manifest into manifests directory")?;

        let out = std::process::Command::new(spin_binary())
            .args([
                "up",
                "--file",
                &format!("{TIMER_TRIGGER_INTEGRATION_TEST}/spin.toml"),
                "--test",
            ])
            .env("SPIN_DATA_DIR", plugin_store_dir)
            .output()?;
        assert!(
            out.status.success(),
            "Running `spin up` returned error: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        Ok(())
    }

    #[test]
    /// Test that the HOST header does not allow outbound http arbitrary calls to hosts
    fn test_spin_inbound_http_host_header() -> anyhow::Result<()> {
        run_test(
            "outbound-http-to-same-app",
            SpinConfig {
                binary_path: spin_binary(),
                spin_up_args: Vec::new(),
                app_type: SpinAppType::Http,
            },
            ServicesConfig::none(),
            move |env| {
                let spin = env.runtime_mut();
                assert_spin_request(
                    spin,
                    Request::full(
                        Method::Get,
                        "/outbound-allowed/hello",
                        &[("Host", "google.com")],
                        Some(""),
                    ),
                    Response::new_with_body(200, "Hello World!\n"),
                )?;
                Ok(())
            },
        )?;
        Ok(())
    }
}
