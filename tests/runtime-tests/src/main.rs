use std::path::PathBuf;

use runtime_tests::RuntimeTest;
use testing_framework::OnTestError;
use testing_framework::runtimes::spin_cli::SpinCli;

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let mut args = std::env::args().skip(1).map(PathBuf::from);
    let spin_binary_path = args.next().unwrap_or_else(|| PathBuf::from("spin"));
    let tests_path = args
        .next()
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests"));

    let config = OnTestError::Log;
    RuntimeTest::<SpinCli>::run_all(&tests_path, spin_binary_path, config)
}
