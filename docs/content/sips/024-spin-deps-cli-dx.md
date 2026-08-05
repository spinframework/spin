title = "SIP 024 - spin deps cli dx"
template = "main"
date = "2026-04-09T00:00:00Z"

---

Summary: A CLI command (`spin deps add`) for adding component dependencies to a Spin application, with interactive prompts for selecting components, exports, and capability inheritance. The command also detects when the resolved component is HTTP middleware and guides attaching it to a trigger selected by route.

Owner(s): [bhardock@akamai.com](mailto:bhardock@akamai.com)

Created: April 9, 2026

# Background

[SIP 020](docs/content/sips/020-component-dependencies.md) introduced the concept of component dependencies in Spin, allowing developers to compose components together by declaring dependencies in `spin.toml`. [SIP 023](docs/content/sips/023-granular-capability-inheritance.md) extended this with per-dependency, granular capability inheritance — replacing the all-or-nothing `dependencies_inherit_configuration` boolean with a flexible `inherit_configuration` field that accepts `true`, `false`, or a list of specific capability keys.

Spin also supports **HTTP middleware**: components that process an incoming request before it reaches the application component, and process the outgoing response on the way back out (for example, an authorisation middleware that inspects credentials and either passes the request through or short-circuits with a "not authorised" response). Unlike a component dependency, middleware is attached to an HTTP *trigger* — via the trigger's `dependencies.middleware` array — rather than to a component, and the entries form an ordered pipeline. See the [HTTP middleware documentation](https://github.com/spinframework/spin-docs/pull/235) for details.

However, authoring either kind of entry by hand requires understanding the TOML schema, knowing which exports a component offers (or that a component is middleware at all), and correctly configuring capability inheritance — all of which are error-prone.

`spin deps add` provides a guided CLI experience for adding either kind of dependency. It resolves the source and inspects the Wasm component. For a regular component dependency, it writes the entry into the parent component's `[dependencies]` table and regenerates the `spin-dependencies.wit` file. When it detects that the component is HTTP middleware, it instead guides the developer to select a target HTTP trigger by route and appends the middleware to that trigger's pipeline.

# Proposal

## Command Syntax

```
spin deps add <source> [options]
```

### Source Formats

The `<source>` positional argument accepts four forms:

| Form | Example | Description |
|------|---------|-------------|
| Local path | `./my-component.wasm` | A path to a Wasm component on disk |
| HTTP URL | `https://example.com/component.wasm` | A remote Wasm component (requires `--digest`) |
| Registry reference | `aws:client@1.0.0` | A package from a component registry |
| Component reference | `ensure-admin` | An existing component defined in the manifest, recorded as `{ component = ... }`. Matched when the source equals a component id. Most commonly used for middleware. |

### Options

| Flag | Description |
|------|-------------|
| `--to <component-id>` | Parent component to add the dependency to. Component dependencies only. Prompted if omitted and the app has multiple components. |
| `-f, --file <path>` | Path to the `spin.toml` manifest. Defaults to the current directory. |
| `--export <name>` | Export to use from the dependency. Component dependencies only. Prompted if omitted and the dependency has multiple exports. |
| `-d, --digest <sha256>` | SHA-256 digest for verifying HTTP downloads. Required for HTTP sources. |
| `-r, --registry <url>` | Override the default registry. Only applies to registry sources. |
| `--inherit <value>` | Capability inheritance: `true`/`all`, `false`/`none`, or comma-separated capabilities. Prompted if omitted and the dependency requires capabilities. |
| `--middleware` | Force treating the dependency as HTTP middleware. By default this is auto-detected from the component's imports and exports. |
| `--route <route>` | HTTP route of the trigger to attach middleware to. Middleware only. Prompted if omitted and the app has multiple HTTP triggers. |
| `--trigger <id>` | Target a trigger by its `id` instead of its route. Middleware only; useful for private endpoints, which have no route. |
| `--before <ref>`, `--after <ref>` | Position the middleware before/after an existing entry in the trigger's pipeline. Middleware only. Defaults to appending to the end of the pipeline. |

## Interactive Prompts

When optional flags are omitted, `spin deps add` presents interactive prompts to guide the developer through each decision. The prompt flow depends on whether the resolved component is a regular component dependency or HTTP middleware; the command detects this automatically, as described next. Steps 1–4 cover the component-dependency flow, and the [Middleware flow](#middleware-flow) covers middleware.

### Detecting the dependency kind

After resolving the source, `spin deps add` inspects the component to decide whether it is a regular component dependency or HTTP middleware.

A component is treated as **middleware** when it both **imports and exports** the `wasi:http/handler` interface (matched semver-compatibly — currently `wasi:http/handler@0.3.0`). The exported `handler` is how the middleware receives a request; the imported `handler` is how it forwards that request to the next component in the pipeline. By contrast, a regular HTTP component only *exports* `handler`, and an ordinary component dependency exports some other interface to satisfy a parent import.

When middleware is detected, the command follows the [Middleware flow](#middleware-flow): it skips export selection (middleware always uses the fixed `handler` interface) and prompts for a target trigger by route instead of a target component. The `--middleware` flag forces this behavior if detection is ever ambiguous.

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

## Middleware flow

When the resolved component is [detected as middleware](#detecting-the-dependency-kind), `spin deps add` guides the developer through attaching it to an HTTP trigger. This flow replaces Steps 1 and 2 (target component and export selection) with trigger and pipeline-position selection; capability inheritance and writing the manifest then proceed as below.

### Step M1: Select the target trigger by route

Middleware is attached to a trigger, not a component. If `--route` is omitted and the application has more than one HTTP trigger, the user selects the trigger by its route:

```
$ spin deps add ./ensure-admin.wasm

Detected HTTP middleware (imports and exports wasi:http/handler).

? Which HTTP route should the middleware be added to?
> /admin/...
  /api/...
  (private) [component: health-check]
```

If the application has exactly one HTTP trigger, it is selected automatically. Private endpoints (which have no route) are listed by their component id and can be targeted non-interactively with `--trigger <id>`.

### Step M2: Choose the pipeline position

The `middleware` field is an ordered pipeline. A request flows through the middlewares from front to back before reaching the application component, and the response flows from back to front. For example, authentication middleware should sit ahead of authorisation middleware. If the selected trigger already has middleware, the user chooses where the new entry goes:

```
? Where should this middleware run in the pipeline?
> Last (closest to the component)
  First (outermost — receives the request first)
  Before ensure-admin
  After ensure-admin
```

By default — and whenever the trigger has no existing middleware — the entry is appended to the end of the pipeline. The `--before <ref>` and `--after <ref>` flags set the position non-interactively.

### Step M3: Select capability inheritance

Capability inheritance works exactly as for component dependencies (see [Step 3](#step-3-select-capability-inheritance)), producing an `inherit_configuration` value on the middleware entry. Because middleware is attached to a trigger rather than a component, the capabilities it inherits come from whichever component the trigger runs, so the command emphasises that **every** component served by the trigger must grant those capabilities:

```
This middleware requires the following capabilities: allowed_outbound_hosts

? Select capabilities to inherit from the parent component
> allowed_outbound_hosts
```

See [Middleware permissions](#middleware-permissions) for details.

### Step M4: Write to manifest

The command appends the middleware entry to the selected trigger's `dependencies.middleware` array and prints a confirmation. Unlike the component-dependency flow, no `spin-dependencies.wit` is generated — middleware is composed onto the component by the HTTP trigger at load time and does not satisfy a component import.

```
Added middleware './ensure-admin.wasm' to the trigger for route '/admin/...'

NOTE: This middleware requires the following capabilities: allowed_outbound_hosts
Ensure every component served by this route grants these capabilities.
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

### Middleware, fully interactive

```
$ spin deps add ./ensure-admin.wasm

Detected HTTP middleware (imports and exports wasi:http/handler).

? Which HTTP route should the middleware be added to?
> /admin/...

? Where should this middleware run in the pipeline?
> Last (closest to the component)

This middleware requires the following capabilities: allowed_outbound_hosts

? Select capabilities to inherit from the parent component
> allowed_outbound_hosts

Added middleware './ensure-admin.wasm' to the trigger for route '/admin/...'

NOTE: This middleware requires the following capabilities: allowed_outbound_hosts
Ensure every component served by this route grants these capabilities.
```

### Middleware, fully non-interactive

```
$ spin deps add authn:ensure-admin@1.0.0 \
    --middleware \
    --route "/admin/..." \
    --inherit allowed_outbound_hosts

Added middleware 'authn:ensure-admin@1.0.0' to the trigger for route '/admin/...'
```

### Middleware referencing an existing component, positioned first

```
$ spin deps add ensure-admin --route "/admin/..." --before audit-log --inherit none

Added middleware 'ensure-admin' to the trigger for route '/admin/...'
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
"my-export" = { path = "./my-component.wasm" }

# HTTP dependency
[component.dashboard.dependencies]
"foo:bar/baz@0.1.0" = { url = "https://example.com/component.wasm", digest = "sha256:abc123..." }
```

### Middleware entries

Middleware is written to the HTTP trigger's `dependencies.middleware` array rather than to a component's `[dependencies]` table. Entries accept the same source and `inherit_configuration` forms as component dependencies, and the array order defines the pipeline (front to back):

```toml
# Reference an existing component in the app
[[trigger.http]]
route = "/admin/..."
component = "admin-ops"
dependencies.middleware = [{ component = "ensure-admin", inherit_configuration = ["allowed_outbound_hosts"] }]

# A pipeline of two middlewares: authentication runs before authorisation
[[trigger.http]]
route = "/secure/..."
component = "secure-ops"
dependencies.middleware = [
  { component = "authn" },
  { url = "https://example.com/authz.wasm", digest = "sha256:...", inherit_configuration = ["allowed_outbound_hosts"] },
]

# Because 'ensure-admin' inherits allowed_outbound_hosts, the trigger's component must grant it
[component.admin-ops]
allowed_outbound_hosts = ["https://authorisation.example.com"]
```

## Capability Detection

The command detects required capabilities by inspecting the dependency's component-level imports and matching them against the capability sets defined in [SIP 023](docs/content/sips/023-granular-capability-inheritance.md) using **semver-compatible** matching. This means a dependency importing `wasi:http/outgoing-handler@0.2.7` correctly matches the `allowed_outbound_hosts` capability set even though the set is defined with `@0.2.6`.

### Middleware permissions

In the current version of Spin, capabilities (network access, key-value stores, and so on) are owned by application components; a dependency — including middleware — can at best *inherit* them. Because middleware is attached to a trigger rather than a component, the capabilities it inherits come from whichever component the trigger runs. `spin deps add` therefore warns that **every** component served by a trigger carrying the middleware must grant the capabilities the middleware needs. For example, a GitHub-authentication middleware that needs outbound access to `api.github.com` requires every such component to declare that host in `allowed_outbound_hosts`, so that the middleware can inherit it. See the [HTTP middleware documentation](https://github.com/spinframework/spin-docs/pull/235)'s Middleware Permissions section for more information.

## Potential Future Work

### Multiple selections within a single package

The current design allows selecting either **all** exports from a package or a **single** specific interface. A natural extension would be to support selecting **multiple** (but not all) interfaces from the same package in a single invocation. For example, a multi-select prompt could allow the user to pick both `aws:client/s3@1.0.0` and `aws:client/dynamodb@1.0.0` without selecting the entire `aws:client@1.0.0` package. This would generate one dependency entry per selected interface and avoid requiring the user to run `spin deps add` multiple times for the same package.

### Applying middleware to multiple routes

The current middleware flow attaches a component to a single trigger per invocation. A natural extension would allow selecting multiple routes — or an "all routes" option — in one command, writing the same middleware entry into each matching trigger's pipeline. This would also help keep capabilities consistent across the components served by those routes.
