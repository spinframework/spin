# Installs latest stable toolchain for Rust and clippy/fmt for this toolchain
rustup update stable && rustup default stable && rustup component add clippy rustfmt

# Installs wasm32 compiler targets
rustup target add wasm32-wasip1 wasm32-wasip2 wasm32-unknown-unknown