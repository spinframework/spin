.PHONY: adapter
adapter:
	cargo build --release --target wasm32-unknown-unknown --manifest-path deny-adapter/Cargo.toml
	wasm-tools component new deny-adapter/target/wasm32-unknown-unknown/release/deny_adapter.wasm -o deny_adapter.wasm
