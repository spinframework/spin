/// Run the tests found in `tests/runtime-tests` directory.
mod runtime_tests {
    use std::path::PathBuf;

    use testing_framework::runtimes::in_process_spin::InProcessSpin;

    // The macro inspects the tests directory and
    // generates individual tests for each one.
    test_codegen_macro::codegen_runtime_tests!(
        ignore: []
    );

    fn run(test_path: PathBuf) {
        let config = runtime_tests::RuntimeTestConfig {
            test_path,
            runtime_config: (),
            on_error: testing_framework::OnTestError::Panic,
        };
        runtime_tests::RuntimeTest::<InProcessSpin>::bootstrap(config)
            .expect("failed to bootstrap runtime tests tests")
            .run();
    }

    #[test]
    fn conformance_tests() -> anyhow::Result<()> {
        let config = conformance_tests::Config::new("canary");
        let conclusion = conformance_tests::run_tests(config, move |test| {
            conformance::run_test(test, &spin_binary())
        })?;
        if conclusion.has_failed() {
            anyhow::bail!("One or more errors occurred in the conformance tests");
        }
        Ok(())
    }

    fn spin_binary() -> PathBuf {
        env!("CARGO_BIN_EXE_spin").into()
    }
}
