title = "SIP 023 - Fine-grained Capability Inheritance for Component Dependencies"
template = "main"
date = "2026-04-06T12:00:00Z"

---

Summary: This proposal extends [SIP 020 - Component Dependencies](020-component-dependencies.md) to support per-dependency, granular capability inheritance, replacing the all-or-nothing `dependencies_inherit_configuration` boolean with a flexible `inherit_configuration` field on each dependency.

Owner(s): [brian.hardock@fermyon.com](mailto:brian.hardock@fermyon.com)

Created: April 6, 2026

# Background

[SIP 020](020-component-dependencies.md) introduced component dependencies in Spin, along with a component-level `dependencies_inherit_configuration` boolean that toggled whether *all* dependencies could inherit the parent component's configuration (e.g. `allowed_outbound_hosts`, `key_value_stores`, etc.). While useful, this mechanism was coarse: either every dependency inherited every configuration, or none did. The SIP itself called out fine-grained, per-dependency configuration inheritance as future work.

In practice, a parent component may have dependencies with very different trust profiles. For example, you might trust an `aws:client/s3` dependency to make outbound HTTP requests (so it can reach S3), but you don't want it to access your key-value store or SQLite databases. With the previous design, you had to grant all configurations or none at all.

This proposal implements the "future work" section of SIP 020 by introducing a per-dependency `inherit_configuration` field that supports three modes: inherit all, inherit none, or inherit a specific subset of named configurations.

# Proposal

## Per-dependency `inherit_configuration`

Each dependency in the `[component.<name>.dependencies]` section can now include an `inherit_configuration` field that controls which parent component configurations the dependency may access.

The field accepts three forms:

### Inherit all configurations

```toml
[component."infra-dashboard".dependencies]
"aws:client" = { version = "1.0.0", inherit_configuration = true }
```

The dependency inherits access to every configuration declared on the parent component (equivalent to the previous `dependencies_inherit_configuration = true`, but scoped to this single dependency).

### Inherit no configurations (default)

```toml
[component."infra-dashboard".dependencies]
"aws:client" = { version = "1.0.0", inherit_configuration = false }
```

This is the default behavior when `inherit_configuration` is omitted. The dependency is fully isolated from the parent's configurations — all capability imports are satisfied by deny adapters.

### Inherit specific configurations

```toml
[component."infra-dashboard"]
allowed_outbound_hosts = ["https://s3.us-west-2.amazonaws.com"]
key_value_stores = ["my-key-value-cache"]
ai_models = ["llama2-chat"]

[component."infra-dashboard".dependencies]
"aws:client" = { version = "1.0.0", inherit_configuration = ["allowed_outbound_hosts"] }
```

Only the listed configuration keys are inherited. All other capabilities are denied. In this example, the `aws:client` dependency can make outbound HTTP requests to the hosts allowed by the parent component, but it cannot access `my-key-value-cache` or `llama2-chat`.

The supported configuration keys are:

| Key | Capabilities Allowed |
|---|---|
| `ai_models` | `fermyon:spin/llm`, `fermyon:spin/llm@2.0.0` |
| `allowed_outbound_hosts` | `fermyon:spin/http`, `fermyon:spin/mysql`, `fermyon:spin/postgres`, `fermyon:spin/redis`, `wasi:http/outgoing-handler@0.2.6`, `wasi:http/client@0.3.0-rc-2026-03-15`, `wasi:sockets/*`, `fermyon:spin/mqtt@2.0.0`, `fermyon:spin/redis@2.0.0`, `spin:mqtt/mqtt@3.0.0`, `spin:redis/redis@3.0.0`, `fermyon:spin/mysql@2.0.0`, `fermyon:spin/postgres@2.0.0`, `spin:postgres/postgres@3.0.0`, `spin:postgres/postgres@4.2.0` |
| `environment` | `wasi:cli/environment@0.2.6`, `wasi:cli/environment@0.3.0-rc-2026-03-15` |
| `files` | `wasi:filesystem/preopens@0.2.6`, `wasi:filesystem/preopens@0.3.0-rc-2026-03-15` |
| `key_value_stores` | `fermyon:spin/key-value`, `fermyon:spin/key-value@2.0.0`, `spin:key-value/key-value@3.0.0`, `wasi:keyvalue/store@0.2.0-draft2` |
| `sqlite_databases` | `fermyon:spin/sqlite`, `fermyon:spin/sqlite@2.0.0`, `spin:sqlite/sqlite@3.1.0` |
| `variables` | `fermyon:spin/config`, `fermyon:spin/variables@2.0.0`, `spin:variables/variables@3.0.0`, `wasi:config@0.2.0-draft-2024-09-27` |

### Applicable to all dependency source types

The `inherit_configuration` field applies uniformly to all dependency source types — registry packages, local paths, HTTP URLs, and app component references:

```toml
[component."my-app".dependencies]
# Registry dependency
"aws:client" = { version = "1.0.0", inherit_configuration = ["allowed_outbound_hosts"] }

# Local dependency
"my:lib/utils" = { path = "lib/utils.wasm", inherit_configuration = true }

# HTTP dependency
"vendor:dep/api" = { url = "https://example.com/dep.wasm", digest = "sha256:abc123", inherit_configuration = ["variables"] }

# App component reference
"infra:dep/svc" = { component = "svc-component", inherit_configuration = ["key_value_stores", "variables"] }
```

> ⚠️ NOTE: The shorthand version string form (`"fizz:buzz" = ">=0.1.0"`) does not support `inherit_configuration`. Use the expanded table form to specify it.

## Backward compatibility with `dependencies_inherit_configuration`

The existing component-level `dependencies_inherit_configuration = true` boolean continues to work as a convenience for applying `inherit_configuration = true` to every dependency. During manifest normalization, the component-level field is expanded into per-dependency `inherit_configuration` values.

However, mixing both forms is **not** allowed. The following manifest is invalid:

```toml
[component."my-app"]
source = "app.wasm"
dependencies_inherit_configuration = true

[component."my-app".dependencies]
# ERROR: cannot mix component-level and per-dependency inherit_configuration
"aws:client" = { version = "1.0.0", inherit_configuration = false }
```

Spin will report an error:

```
Component `my-app` specifies both `dependencies_inherit_configuration` and per-dependency
`inherit_configuration`. These are mutually exclusive; use one or the other.
```