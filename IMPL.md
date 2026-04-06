
# Implementation

## New `spin-capabilities` crate

The monolithic `deny-all` adapter and associated logic in the `spin-compose` crate has been replaced by a new `spin-capabilities` crate (`crates/capabilities/`) that provides:

1. **`InheritConfiguration` enum** — The canonical representation of the inheritance mode (`All`, `None`, or `Some(Vec<String>)`), shared between the manifest schema and composition logic.

2. **`apply_deny_adapter(source, inherits)` function** — Given a dependency's Wasm component bytes and an `InheritConfiguration` value, this function selectively composes deny adapter exports into the dependency. Only the interfaces that are *not* in the allow list are plugged with the deny adapter; allowed interfaces pass through to the host.

3. **`InheritConfiguration::collect(source)` function** — Introspects a Wasm component's imports to determine which capability sets it requires. This enables tooling to report or suggest the minimal set of configurations a dependency needs.

### Selective deny adapter composition

The previous approach used `wac_graph::plug` to blanket-plug the `deny-all` adapter into a dependency. The new approach enumerates the deny adapter's exports and selectively wires each one into the dependency's matching imports — but only for interfaces *not* in the computed allow list. This means:

- `inherit_configuration = "all"` → no deny exports are applied; the original component is returned as-is
- `inherit_configuration = "none"` (or omitted) → all matching deny exports are applied, identical to the previous `deny-all` behavior
- `inherit_configuration = ["allowed_outbound_hosts", "variables"]` → only the HTTP/sockets/MQTT/Redis and variables interfaces pass through; all other matching imports are denied

### The deny adapter component

The deny adapter component is a Wasm component that exports stub implementations of every capability interface Spin provides. Each stub returns a deny/error result. The adapter is built from `crates/capabilities/deny-adapter/`, a WASI component written in Rust using `wit-bindgen`, and the compiled `.wasm` is embedded into the `spin-capabilities` crate at build time.

## Manifest schema changes

The `ComponentDependency` enum in `crates/manifest/src/schema/v2.rs` is extended with an optional `inherit_configuration` field on each variant (`Package`, `Local`, `HTTP`, `AppComponent`). The field is represented as the `InheritConfiguration` enum:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum InheritConfiguration {
    /// "all" or "none"
    All(InheritConfigurationAll),
    /// ["key1", "key2", ...]
    Some(Vec<String>),
}
```

The component-level `dependencies_inherit_configuration` field is changed from `bool` to `Option<bool>` to support the mutual-exclusion check during normalization.

## Manifest normalization

A new normalization step (`normalize_dependency_inherit_configuration`) runs during manifest processing. It:

1. Checks that `dependencies_inherit_configuration` and per-dependency `inherit_configuration` are not used simultaneously on the same component (raises an error if both are present).
2. If `dependencies_inherit_configuration = true`, expands it into `inherit_configuration = "all"` on every dependency of that component, then clears the component-level field.

## Loader changes

The loader (`crates/loader/src/local.rs`) no longer threads a `bool` through the dependency loading pipeline. Instead, it reads the `inherit_configuration` from each dependency directly and maps it to the locked app's `InheritConfiguration` representation. The `load_component_dependency` method now returns a fully resolved `LockedComponentDependency` with the appropriate `inherit` field, and a separate `load_dependency_content` method is extracted for callers that only need the Wasm path and export name.
