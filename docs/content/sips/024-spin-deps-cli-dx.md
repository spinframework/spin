title = "SIP 024 - spin deps cli dx"
template = "main"
date = "2026-04-09T00:00:00Z"

---

Summary: A CLI command (`spin deps add`) for adding component dependencies to a Spin application, with interactive prompts for selecting components, exports, and capability inheritance.

Owner(s): [brian.hardock@fermyon.com](mailto:brian.hardock@fermyon.com)

Created: April 9, 2026

# Background

[SIP 020](docs/content/sips/020-component-dependencies.md) introduced the concept of component dependencies in Spin, allowing developers to compose components together by declaring dependencies in `spin.toml`. [SIP 023](docs/content/sips/023-granular-capability-inheritance.md) extended this with per-dependency, granular capability inheritance — replacing the all-or-nothing `dependencies_inherit_configuration` boolean with a flexible `inherit_configuration` field that accepts `true`, `false`, or a list of specific capability keys.

However, authoring the dependency entries by hand requires understanding the TOML schema, knowing which exports a component offers, and correctly configuring capability inheritance — all of which are error-prone.

`spin deps add` provides a guided CLI experience for adding a component dependency. It resolves the source, inspects the Wasm component's exports, and writes the correct entry into `spin.toml`, along with regenerating the `spin-dependencies.wit` file.

# Proposal
∏
## Command Syntax

```
spin deps add <source> [options]
```

### Source Formats

The `<source>` positional argument accepts three forms:

| Form | Example | Description |
|------|---------|-------------|
| Local path | `./my-component.wasm` | A path to a Wasm component on disk |
| HTTP URL | `https://example.com/component.wasm` | A remote Wasm component (requires `--digest`) |
| Registry reference | `aws:client@1.0.0` | A package from a component registry |

### Options

| Flag | Description |
|------|-------------|
| `--to <component-id>` | Target component to add the dependency to. Prompted if omitted and the app has multiple components. |
| `-f, --from <path>` | Path to the `spin.toml` manifest. Defaults to the current directory. |
| `--export <name>` | Export to use from the dependency. Prompted if omitted and the component has multiple exports. |
| `-d, --digest <sha256>` | SHA-256 digest for verifying HTTP downloads. Required for HTTP sources. |
| `-r, --registry <url>` | Override the default registry. Only applies to registry sources. |
| `--inherit <value>` | Capability inheritance: `true`/`all`, `false`/`none`, or comma-separated capabilities. Prompted if omitted and the dependency requires capabilities. |

## Interactive Prompts

When optional flags are omitted, `spin deps add` presents interactive prompts to guide the developer through each decision. The following sections illustrate the prompt flow.

### Step 1: Select the target component

If `--to` is omitted and the application has more than one component, the user is prompted:

```
$ spin deps add aws:client@1.0.0

? Which component should the dependency be added to?
> api-server
  worker
  dashboard
```

If the application has exactly one component, it is selected automatically.

### Step 2: Select the export

The command inspects the resolved Wasm component to enumerate its exports. If `--export` is omitted, the prompt flow depends on the number of packages and interfaces.

#### Single export — auto-selected

If the component exports only one interface, it is selected automatically with no prompt.

#### Multiple packages — select a package first

If the component exports interfaces from multiple packages, the user first selects a package:

```
? Which package should be used?
> aws:client@1.0.0
  aws:util@1.0.0
```

#### Within-package selection — all or a specific interface

After a package is selected (or if there is only one), the user chooses between all exports from that package or a single specific interface:

```
? Which export should be used?
> All from aws:client@1.0.0
  aws:client/s3@1.0.0
  aws:client/dynamodb@1.0.0
  aws:client/sqs@1.0.0
```

Selecting **"All from aws:client@1.0.0"** records `aws:client@1.0.0` as the dependency name (a package-level selector). Selecting a specific interface records that interface (e.g. `aws:client/s3@1.0.0`).

#### Explicit `--export` flag

The `--export` flag accepts the same forms:

- **Specific interface:** `--export aws:client/s3@1.0.0`
- **Package selector:** `--export aws:client@1.0.0` (selects all matching exports)
- **Plain name:** `--export my-export`

### Step 3: Select capability inheritance

The command inspects the dependency's imports and matches them against known capability sets (e.g. `allowed_outbound_hosts`, `ai_models`, `key_value_stores`) using semver-compatible matching. If the dependency requires any capabilities and `--inherit` is omitted, the user is prompted:

```
This dependency requires the following capabilities: allowed_outbound_hosts, ai_models

? Select capabilities to inherit from the parent component
> All
  allowed_outbound_hosts
  ai_models
```

Selecting **"All"** sets `inherit_configuration = true` in the manifest. Selecting individual capabilities records them as a list (e.g. `inherit_configuration = ["allowed_outbound_hosts"]`). Selecting nothing results in no inheritance.

#### Explicit `--inherit` flag

- `--inherit true` or `--inherit all` → inherits all capabilities
- `--inherit false` or `--inherit none` → inherits nothing
- `--inherit allowed_outbound_hosts,ai_models` → inherits only those capabilities

### Step 4: Write to manifest and regenerate WIT

After all selections are made, the command:

1. Serializes the dependency into the `[component.<id>.dependencies]` table in `spin.toml`
2. Regenerates `spin-dependencies.wit` in the component's build directory
3. Prints a confirmation message

```
Added aws:client@1.0.0 to component 'api-server'

NOTE: This dependency requires the following capabilities: allowed_outbound_hosts, ai_models
You may need to add configuration for these capabilities to your component.
```

## End-to-End Examples

### Fully interactive

```
$ spin deps add aws:client@1.0.0

? Which component should the dependency be added to?
> api-server

? Which package should be used?
> aws:client@1.0.0

? Which export should be used?
> aws:client/s3@1.0.0

This dependency requires the following capabilities: allowed_outbound_hosts

? Select capabilities to inherit from the parent component
> allowed_outbound_hosts

Added aws:client/s3@1.0.0 to component 'api-server'

NOTE: This dependency requires the following capabilities: allowed_outbound_hosts
You may need to add configuration for these capabilities to your component.
```

### Fully non-interactive

```
$ spin deps add aws:client@1.0.0 \
    --to api-server \
    --export aws:client/s3@1.0.0 \
    --inherit allowed_outbound_hosts

Added aws:client/s3@1.0.0 to component 'api-server'

NOTE: This dependency requires the following capabilities: allowed_outbound_hosts
You may need to add configuration for these capabilities to your component.
```

### Local component with all capabilities

```
$ spin deps add ./my-component.wasm --to worker --export my-export --inherit all

Added my-export to component 'worker'
```

### HTTP source

```
$ spin deps add https://example.com/component.wasm \
    --digest abc123... \
    --to dashboard \
    --export foo:bar/baz@0.1.0 \
    --inherit false

Added foo:bar/baz@0.1.0 to component 'dashboard'
```

## Resulting Manifest Entries

The command produces entries in `spin.toml` matching the schema defined in [SIP 020](docs/content/sips/020-component-dependencies.md) and the per-dependency `inherit_configuration` field introduced in [SIP 023](docs/content/sips/023-granular-capability-inheritance.md):

```toml
# Package-level selector with full inheritance
[component.api-server.dependencies]
"aws:client@1.0.0" = { version = "=1.0.0", package = "aws:client", inherit_configuration = true }

# Specific interface with selective inheritance
[component.api-server.dependencies]
"aws:client/s3@1.0.0" = { version = "=1.0.0", package = "aws:client", inherit_configuration = ["allowed_outbound_hosts"] }

# Local dependency with no inheritance
[component.worker.dependencies]
"my-export" = { path = "my-component.wasm" }

# HTTP dependency
[component.dashboard.dependencies]
"foo:bar/baz@0.1.0" = { url = "https://example.com/component.wasm", digest = "sha256:abc123..." }
```

## Capability Detection

The command detects required capabilities by inspecting the dependency's component-level imports and matching them against the capability sets defined in [SIP 023](docs/content/sips/023-granular-capability-inheritance.md) using **semver-compatible** matching (via `wac_graph::types::are_semver_compatible`). This means a dependency importing `wasi:http/outgoing-handler@0.2.7` correctly matches the `allowed_outbound_hosts` capability set even though the set is defined with `@0.2.6`.

The recognized capability sets are:

| Capability | Example interfaces |
|---|---|
| `ai_models` | `fermyon:spin/llm` |
| `allowed_outbound_hosts` | `wasi:http/outgoing-handler`, `wasi:sockets/tcp`, `fermyon:spin/mqtt` |
| `environment` | `wasi:cli/environment` |
| `files` | `wasi:filesystem/preopens` |
| `key_value_stores` | `fermyon:spin/key-value` |
| `sqlite_databases` | `fermyon:spin/sqlite` |
| `variables` | `fermyon:spin/variables` |

## Potential Future Work

### Multiple selections within a single package

The current design allows selecting either **all** exports from a package or a **single** specific interface. A natural extension would be to support selecting **multiple** (but not all) interfaces from the same package in a single invocation. For example, a multi-select prompt could allow the user to pick both `aws:client/s3@1.0.0` and `aws:client/dynamodb@1.0.0` without selecting the entire `aws:client@1.0.0` package. This would generate one dependency entry per selected interface and avoid requiring the user to run `spin deps add` multiple times for the same package.
