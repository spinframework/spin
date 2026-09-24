# Deliberately not wasm32-wasip2: std's WASI imports are toolchain-dependent and can
# collide with the adapter's own exports (e.g. sockets/udp@0.2.9 vs @0.2.6), rebinding
# exported resources to the host's and breaking composition. Guarded by tests in deny.rs.
.PHONY: adapter
adapter:
	cargo build --release --target wasm32-unknown-unknown --manifest-path deny-adapter/Cargo.toml
	wasm-tools component new deny-adapter/target/wasm32-unknown-unknown/release/deny_adapter.wasm -o deny_adapter.wasm
