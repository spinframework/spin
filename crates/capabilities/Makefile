# Deliberately not wasm32-wasip2: std's WASI imports are toolchain-dependent and can
# collide with the adapter's own exports (e.g. sockets/udp@0.2.9 vs @0.2.6), rebinding
# exported resources to the host's and breaking composition. Guarded by tests in deny.rs.
ADAPTER_OUT ?= deny_adapter.wasm
CHECK_DIR := deny-adapter/target/check

.PHONY: adapter
adapter:
	cargo build --release --target wasm32-unknown-unknown --manifest-path deny-adapter/Cargo.toml
	wasm-tools component new deny-adapter/target/wasm32-unknown-unknown/release/deny_adapter.wasm -o $(ADAPTER_OUT)

# Compares WIT rather than bytes, since codegen differs across rustc versions.
.PHONY: check-adapter
check-adapter:
	mkdir -p $(CHECK_DIR)
	$(MAKE) adapter ADAPTER_OUT=$(CHECK_DIR)/deny_adapter.wasm
	wasm-tools component wit deny_adapter.wasm > $(CHECK_DIR)/committed.wit
	wasm-tools component wit $(CHECK_DIR)/deny_adapter.wasm > $(CHECK_DIR)/rebuilt.wit
	@diff -u $(CHECK_DIR)/committed.wit $(CHECK_DIR)/rebuilt.wit || \
		(echo "deny_adapter.wasm is stale: run 'make adapter' in crates/capabilities and commit the result" && exit 1)
