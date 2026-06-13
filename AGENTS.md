# Repository Guidelines

## Project Structure & Module Organization

Spin is a Rust workspace for the `spin` CLI and supporting crates. The `spin` binary is `src/bin/spin.rs`; shared CLI modules live in `src/`. Reusable implementation is split across `crates/*` with names such as `spin-trigger-http`, `spin-factor-key-value`, and `spin-templates`. Tests live in `tests/`: `tests/runtime-tests` validates runtime behavior, `tests/test-components` provides WebAssembly components, and `tests/testing-framework` contains shared helpers. Examples are in `examples/`, docs in `docs/`, templates in `templates/`, and WIT interfaces in `wit/`.

## Build, Test, and Development Commands

- `make build`: builds the release binary with `cargo build --release`; run this before opening PRs that change dependencies, features, or production imports because it does not rely on dev/test dependencies.
- `make install`: installs the local CLI with `cargo install --path . --locked`.
- `cargo run --bin spin -- --help`: runs the development CLI from source.
- `make lint`: runs workspace `clippy -D warnings` and checks `rustfmt`.
- `make test`: runs lint, unit tests, and integration tests.
- `make test-unit`: runs workspace unit tests, excluding integration/runtime suites.
- `make test-crate crate=spin-key-value-azure`: runs tests for one crate.
- `make test-integration`: runs runtime and integration tests without heavy external dependencies.
- `make test-integration-full`: includes Docker or extra toolchain tests.

## Coding Style & Naming Conventions

Use Rust 2024 with the minimum Rust version in `Cargo.toml`. Format with `cargo fmt --all`; do not hand-format around rustfmt. Keep `clippy` clean under `make lint`. Use `snake_case` for modules, functions, variables, and tests; `PascalCase` for types and traits; and `SCREAMING_SNAKE_CASE` for constants. New workspace crates should follow the `spin-*` pattern in `crates/`.

## Testing Guidelines

Prefer focused crate tests during development, then run `make test-unit`, `make build`, and the relevant integration target before opening a PR. Use `make build` specifically to catch production build issues that may be hidden when dev dependencies, tests, or `--all-targets` accidentally enable missing dependency features. Put crate-local tests near the code or in that crate's `tests/` directory. Add runtime scenarios under `tests/runtime-tests/tests` when validating Spin manifests and compliant WebAssembly components. Integration tests cover multi-subsystem CLI behavior and belong under `tests/`.

## Commit & Pull Request Guidelines

Recent history uses imperative commit subjects such as `Remove unnecessary module` or `Allow config for how low we await connection semaphore`; scoped conventional subjects appear in automation, for example `chore(rust-sdk-release): ...`. Keep commits focused and explain user-visible changes in the PR body. PRs should link related issues, describe test coverage, note new dependencies or services, and include screenshots only for UI or documentation rendering changes.

## Security & Configuration Tips

Do not commit credentials, certificates, or service-specific secrets. Keep generated TLS files, local runtime config, and dependency-heavy test setup out of commits unless intentionally part of a checked-in fixture.
