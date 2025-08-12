# Recreating the test components

```
cd source/test-command
cargo build --release --target wasm32-wasip1
```

then copy from `target/wasm32-wasip1/release/` to this directory.

**IMPORTANT:** Do not use the `wasm32-wasip2` target. It generates to the 0.2.x world (0.2.3 at time of writing), and Component Model tooling does not yet accept that as compatible with the 0.2.0 world.
