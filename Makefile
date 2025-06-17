LOG_LEVEL_VAR ?= RUST_LOG=spin=trace
CERT_NAME ?= local
SPIN_DOC_NAME ?= new-doc.md
export PATH := target/debug:target/release:$(HOME)/.cargo/bin:$(PATH)

## overrides for Windows
ifeq ($(OS),Windows_NT)
	LOG_LEVEL_VAR =
endif

.PHONY: build
build:
	cargo build --release

.PHONY: install
install:
	cargo install --path . --locked

.PHONY: test
test: lint test-unit test-integration

.PHONY: lint
lint:
	cargo clippy --all --all-targets --features "all-tests unsafe-aot-compilation" -- -D warnings
	cargo fmt --all -- --check

.PHONY: lint-rust-examples
lint-rust-examples:
	for manifest_path in $$(find examples  -name Cargo.toml); do \
		echo "Linting $${manifest_path}" \
		&& cargo clippy --manifest-path "$${manifest_path}" -- -D warnings \
		&& cargo fmt --manifest-path "$${manifest_path}" -- --check \
		|| exit 1 ; \
	done

.PHONY: lint-all
lint-all: lint lint-rust-examples

## Bring all of the checked in `Cargo.lock` files up-to-date
.PHONY: update-cargo-locks
update-cargo-locks:
	echo "Updating Cargo.toml"
	cargo update -w --offline; \
	for manifest_path in $$(find examples -name Cargo.toml); do \
		echo "Updating $${manifest_path}" && \
		cargo update --manifest-path "$${manifest_path}" -w --offline; \
	done

.PHONY: test-unit
test-unit:
	$(LOG_LEVEL_VAR) cargo test --all --no-fail-fast -- --skip integration_tests --skip runtime_tests --nocapture

.PHONY: test-crate
test-crate:
	$(LOG_LEVEL_VAR) cargo test -p $(crate) --no-fail-fast -- --skip integration_tests --skip runtime_tests --nocapture

# Run the runtime tests without the tests that use some sort of assumed external dependency (e.g., Docker, a language toolchain, etc.)
.PHONY: test-runtime
test-runtime:
	cargo test --release runtime_tests --no-default-features --no-fail-fast -- --nocapture

# Run all of the runtime tests including those that use some sort of assumed external dependency (e.g., Docker, a language toolchain, etc.)
.PHONY: test-runtime-full
test-runtime-full:
	cargo test --release runtime_tests --no-default-features --features extern-dependencies-tests --no-fail-fast -- --nocapture

# Run the integration tests without the tests that use some sort of assumed external dependency (e.g., Docker, a language toolchain, etc.)
.PHONY: test-integration
test-integration: test-runtime
	cargo test --release integration_tests --no-default-features --no-fail-fast -- --nocapture

# Run all of the integration tests including those that use some sort of assumed external dependency (e.g., Docker, a language toolchain, etc.)
.PHONY: test-integration-full
test-integration-full: test-runtime-full
	cargo test --release integration_tests --no-default-features --features extern-dependencies-tests --no-fail-fast -- --nocapture

# simple convenience for developing with TLS
.PHONY: tls
tls: ${CERT_NAME}.crt.pem

$(CERT_NAME).crt.pem:
	openssl req -newkey rsa:2048 -nodes -keyout $(CERT_NAME).key.pem -x509 -days 365 -out $(CERT_NAME).crt.pem
