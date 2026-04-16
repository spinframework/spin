.PHONY: adapter
adapter:
	cargo build --release --target wasm32-wasip2 --manifest-path deny-adapter/Cargo.toml
	cp deny-adapter/target/wasm32-wasip2/release/deny_adapter.wasm .