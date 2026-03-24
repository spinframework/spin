# Environment Variable Isolation Example

This example demonstrates per-dependency environment variable isolation in a Spin application.

The `main` component calls a dependency (`dependable`), and each component receives a different environment configuration, including different values for the same variable name.

## Prerequisites

Install [Rust](https://rustup.rs) and [Spin](https://github.com/spinframework/spin).

If needed, add the WebAssembly target used by this example:

```bash
rustup target add wasm32-wasip2
```

## Building and Running

From this directory, build and run the app:

```bash
spin build
spin up
```

## Testing

In another terminal, send a request:

```bash
curl -s http://127.0.0.1:3000/
```

## What to look for

The response includes two lines:

- `main's env vars: ...`
- `dependable's env vars: ...`

You should see that environment values are isolated per component:

- `MAIN_ONLY` appears only in `main`.
- `DEPENDABLE_ONLY` appears only in `dependable`.
- `GREETING` is different for each component (`hello from main` vs `hello from dependable`).

## How it works

- `spin.toml` defines component-level environment variables for `main` and dependency-specific variables for `hello:components/dependable`.
- `crates/main/src/lib.rs` returns `main` env values and calls the dependency.
- `crates/dependable/src/lib.rs` returns env values seen by the dependency component.
