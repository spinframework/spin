title = "SIP 024 - spin deps cli dx"
template = "main"
date = "2026-04-09T00:00:00Z"

---

Summary: A CLI command (`spin deps add`) for adding component dependencies to a Spin application, with interactive prompts for selecting components, interfaces, and capability inheritance. The command also detects when the resolved component is HTTP middleware and guides attaching it to a trigger selected by route.

Owner(s): Brian Hardock

Created: April 9, 2026

# Background

[SIP 020](docs/content/sips/020-component-dependencies.md) introduced the concept of component dependencies in Spin, allowing developers to compose components together by declaring dependencies in `spin.toml`. [SIP 023](docs/content/sips/023-granular-capability-inheritance.md) extended this with per-dependency, granular capability inheritance — replacing the all-or-nothing `dependencies_inherit_configuration` boolean with a flexible `inherit_configuration` field that accepts `true`, `false`, or a list of specific capability keys.

Spin also supports **HTTP middleware**: components that process an incoming request before it reaches the application component, and process the outgoing response on the way back out (for example, an authorization middleware that inspects credentials and either passes the request through or short-circuits with a "not authorised" response). Unlike a component dependency, middleware is attached to an HTTP *trigger* — via the trigger's `dependencies.middleware` array — rather than to a component, and the entries form an ordered pipeline. See the [HTTP middleware documentation](https://github.com/spinframework/spin-docs/pull/235) for details.

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

Only flags that identify or resolve the source are supported. All decisions — which component to add the dependency to, which interface to import, which capabilities to inherit, and where to position middleware — are always interactive. Whether the source is a regular component dependency or HTTP middleware is auto-detected from its interface signatures.

| Flag | Description |
|------|-------------|
| `-f, --file <path>` | Path to the `spin.toml` manifest. Defaults to the current directory. |
| `-d, --digest <sha256>` | SHA-256 digest for verifying HTTP downloads. Required for HTTP sources. |
| `-r, --registry <url>` | Override the default registry. Only applies to registry sources. |

## Interactive Flow

`spin deps add` is an interactive command. After resolving the source, it walks the developer through each decision — target component, interface to import, and capability inheritance — with a series of prompts, auto-selecting whenever there is only one valid choice. The flags in the previous section only affect how the source is located and resolved; they never gate or replace a prompt. The prompt flow depends on whether the resolved component is a regular component dependency or HTTP middleware; the command detects this automatically, as described next. Steps 1–4 cover the component-dependency flow, and the [Middleware flow](#middleware-flow) covers middleware.

> **Note on `dependencies_inherit_configuration`:** If the target component already has the legacy `dependencies_inherit_configuration` field set, the command skips the capability inheritance prompt entirely and does not write a per-dependency `inherit_configuration` value — the blanket setting already covers all dependencies.

### Detecting the dependency kind

After resolving the source, `spin deps add` inspects the component to decide whether it is a regular component dependency or HTTP middleware.

A component is treated as **middleware** when it both **imports and exports** the `wasi:http/handler` interface (matched semver-compatibly — currently `wasi:http/handler@0.3.0`). The exported `handler` is how the middleware receives a request; the imported `handler` is how it forwards that request to the next component in the pipeline. By contrast, a regular HTTP component only *exports* `handler`, and an ordinary component dependency exports some other interface to satisfy a parent import.

When middleware is detected, the command follows the [Middleware flow](#middleware-flow): it skips interface selection (middleware always uses the fixed `handler` interface) and prompts for a target trigger by route instead of a target component. Detection is based purely on the component's interface signatures, so no flag is required to opt in.

### Step 1: Select the target component

If the application has more than one component, the user is prompted:

```
$ spin deps add aws:client@1.0.0

? Which component should the dependency be added to?
> api-server
  worker
  dashboard
```

If the application has exactly one component, it is selected automatically.

### Step 2: Select the interface to import

The command inspects the resolved Wasm component to enumerate its exports. The prompt flow depends on the number of exports.

#### Single export — auto-selected

If the component exports only one interface, it is selected automatically with no prompt.

#### Multiple exports — flat list

All available interfaces are presented in a single flat list. For each package that has multiple interfaces, an "All from …" entry is included so the user can select the entire package in one step:

```
? Which interface do you want to import?
> All from aws:client@1.0.0
  aws:client/s3@1.0.0
  aws:client/dynamodb@1.0.0
  aws:client/sqs@1.0.0
```

Selecting **"All from aws:client@1.0.0"** records `aws:client@1.0.0` as the dependency name (a package-level selector). Selecting a specific interface records that interface (e.g. `aws:client/s3@1.0.0`).

### Step 3: Select capability inheritance

The command inspects the dependency's imports and matches them against known capability sets (e.g. `allowed_outbound_hosts`, `ai_models`, `key_value_stores`) using semver-compatible matching. If the dependency requires any capabilities, the user is prompted:

```
This dependency requires the following capabilities: allowed_outbound_hosts, ai_models

? Select capabilities to inherit from the parent component
  [ ] allowed_outbound_hosts
  [ ] ai_models
```

The prompt is a multi-select (checkboxes). The selected capabilities are always recorded as an explicit list — for example, selecting both options writes `inherit_configuration = ["allowed_outbound_hosts", "ai_models"]`. Selecting nothing omits the field, resulting in no inheritance.

The command never writes `inherit_configuration = true`, even when every currently-listed capability is selected. `true` is a wildcard that would also grant whatever capabilities a *future* version of the dependency imports. Recording the explicit list instead ensures that choosing `allowed_outbound_hosts` and `ai_models` today does not silently opt the dependency into, say, `key_value_stores` if a later release starts importing it. (The manifest schema still accepts `true` for users who deliberately want that behavior; `spin deps add` simply does not generate it.)

### Step 4: Write to manifest and regenerate WIT

After all selections are made, the command:

1. Serializes the dependency into the `[component.<id>.dependencies]` table in `spin.toml`
2. Regenerates `spin-dependencies.wit` in the component's build directory
3. Prints a confirmation message with contextual warnings

The confirmation message varies based on whether the parent component already satisfies the dependency's capability requirements:

```
Added aws:client@1.0.0 to component 'api-server'

Run `spin build` to generate language bindings for the new dependency.
```

If capabilities were inherited but the parent component does not yet declare them:

```
Added aws:client@1.0.0 to component 'api-server'

WARNING: This dependency inherits 'ai_models' but component 'api-server' does not
currently declare any ai_models. The dependency will have no model access at runtime
until you configure ai_models on the parent component.

Run `spin build` to generate language bindings for the new dependency.
```

If the user declined to inherit a capability the dependency requires:

```
Added aws:client@1.0.0 to component 'api-server'

WARNING: This dependency uses the LLM API but 'ai_models' was not inherited.
The dependency will receive 'access denied' errors for model operations at runtime.

Run `spin build` to generate language bindings for the new dependency.
```

## Middleware flow

When the resolved component is [detected as middleware](#detecting-the-dependency-kind), `spin deps add` guides the developer through attaching it to an HTTP trigger. This flow replaces Steps 1 and 2 (target component and interface selection) with trigger and pipeline-position selection; capability inheritance and writing the manifest then proceed as below.

The middleware flow is **interactive-only** in the initial implementation. Non-interactive flags (e.g. for CI/scripting scenarios) may be added in a future iteration once usage patterns are clear.

### Step M1: Select the target trigger by route

Middleware is attached to a trigger, not a component. If the application has more than one HTTP trigger, the user selects the trigger by its route:

```
$ spin deps add ./ensure-admin.wasm

Detected HTTP middleware (imports and exports wasi:http/handler).

? Which HTTP route should the middleware be added to?
> /admin/...
  /api/...
```

If the application has exactly one HTTP trigger, it is selected automatically.

> **Private endpoints are excluded.** Middleware is not offered for private (route-less) endpoints. Service-chaining requests go directly to the component without passing through a trigger, so trigger-attached middleware would not execute. If middleware-like behavior is needed between chained components, use a regular component dependency instead.

### Step M2: Choose the pipeline position

The `middleware` field is an ordered pipeline. A request flows through the middlewares from front to back before reaching the application component, and the response flows from back to front. For example, authentication middleware should sit ahead of authorization middleware.

If the selected trigger already has middleware, the command displays the current pipeline and lets the user position the new entry using arrow keys:

```
? Position the middleware in the pipeline (use ↑↓ to move):
  existing-auth
▶ [ensure-admin]  ← new
  existing-logger
  ─── application component ───
```

By default — and whenever the trigger has no existing middleware — the entry is appended to the end of the pipeline (closest to the application component).

### Step M3: Select capability inheritance

Capability inheritance works exactly as for component dependencies (see [Step 3](#step-3-select-capability-inheritance)), producing an `inherit_configuration` value on the middleware entry. Because middleware is attached to a trigger rather than a component, the capabilities it inherits come from the component the trigger routes to:

```
This middleware requires the following capabilities: allowed_outbound_hosts

? Select capabilities to inherit from the trigger's component
  [x] allowed_outbound_hosts
```

See [Middleware permissions](#middleware-permissions) for details.

### Step M4: Write to manifest

The command appends the middleware entry to the selected trigger's `dependencies.middleware` array and prints a confirmation. Unlike the component-dependency flow, no `spin-dependencies.wit` is generated — middleware is composed onto the component by the HTTP trigger at load time and does not satisfy a component import.

If the trigger's component does not currently declare the inherited capabilities, the command warns specifically:

```
Added middleware './ensure-admin.wasm' to the trigger for route '/admin/...'

WARNING: This middleware inherits 'allowed_outbound_hosts' but component 'admin-ops'
does not currently declare any allowed_outbound_hosts. The middleware will have no
network access at runtime until you configure allowed_outbound_hosts on 'admin-ops'.
```

## End-to-End Examples

### Registry source, fully interactive

```
$ spin deps add aws:client@1.0.0

? Which component should the dependency be added to?
> api-server

? Which interface do you want to import?
> aws:client/s3@1.0.0

This dependency requires the following capabilities: allowed_outbound_hosts

? Select capabilities to inherit from the parent component
  [x] allowed_outbound_hosts

Added aws:client/s3@1.0.0 to component 'api-server'
Run `spin build` to generate language bindings for the new dependency.
```

### Local component, single-component app

When the app has exactly one component, the dependency exports exactly one interface, and it requires no capabilities, everything is auto-selected and no prompts appear:

```
$ spin deps add ./my-component.wasm

Added my-export to component 'worker'
Run `spin build` to generate language bindings for the new dependency.
```

### HTTP source

```
$ spin deps add https://example.com/component.wasm --digest sha256:abc123...

? Which component should the dependency be added to?
> dashboard

? Which interface do you want to import?
> foo:bar/baz@0.1.0

Added foo:bar/baz@0.1.0 to component 'dashboard'
Run `spin build` to generate language bindings for the new dependency.
```

### Middleware (interactive)

Here the `/admin/...` route already has an `authn` middleware, so the position prompt appears; the new authorization middleware is placed after it:

```
$ spin deps add ./ensure-admin.wasm

Detected HTTP middleware (imports and exports wasi:http/handler).

? Which HTTP route should the middleware be added to?
> /admin/...

? Position the middleware in the pipeline (use ↑↓ to move):
  authn
▶ [ensure-admin]  ← new
  ─── application component ───

This middleware requires the following capabilities: allowed_outbound_hosts

? Select capabilities to inherit from the trigger's component
  [x] allowed_outbound_hosts

Added middleware './ensure-admin.wasm' to the trigger for route '/admin/...'
```

### Middleware referencing an existing component

```
$ spin deps add ensure-admin

Detected HTTP middleware (imports and exports wasi:http/handler).

? Which HTTP route should the middleware be added to?
> /admin/...

Added middleware 'ensure-admin' to the trigger for route '/admin/...'
```

## Resulting Manifest Entries

The command produces entries in `spin.toml` matching the schema defined in [SIP 020](docs/content/sips/020-component-dependencies.md) and the per-dependency `inherit_configuration` field introduced in [SIP 023](docs/content/sips/023-granular-capability-inheritance.md):

```toml
# Package-level selector inheriting explicitly-chosen capabilities
[component.api-server.dependencies]
"aws:client@1.0.0" = { version = "=1.0.0", package = "aws:client", inherit_configuration = ["allowed_outbound_hosts", "ai_models"] }

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

# A pipeline of two middlewares: authentication runs before authorization
[[trigger.http]]
route = "/secure/..."
component = "secure-ops"
dependencies.middleware = [
  { component = "authn" },
  { url = "https://example.com/authz.wasm", digest = "sha256:...", inherit_configuration = ["allowed_outbound_hosts"] },
]

# Because 'ensure-admin' inherits allowed_outbound_hosts, the trigger's component must grant it
[component.admin-ops]
allowed_outbound_hosts = ["https://authorization.example.com"]
```

## Capability Detection

The command detects required capabilities by inspecting the dependency's component-level imports and matching them against the capability sets defined in [SIP 023](docs/content/sips/023-granular-capability-inheritance.md) using **semver-compatible** matching. This means a dependency importing `wasi:http/outgoing-handler@0.2.7` correctly matches the `allowed_outbound_hosts` capability set even though the set is defined with `@0.2.6`.

### Middleware permissions

In the current version of Spin, capabilities (network access, key-value stores, and so on) are owned by application components; a dependency — including middleware — can at best *inherit* them. Because middleware is attached to a trigger rather than a component, the capabilities it inherits come from the component the trigger routes to. `spin deps add` checks that component's manifest declarations and warns specifically when the component does not currently grant the capabilities the middleware needs. For example, a GitHub-authentication middleware that needs outbound access to `api.github.com` requires the trigger's component to declare that host in `allowed_outbound_hosts`, so that the middleware can inherit it. See the [HTTP middleware documentation](https://github.com/spinframework/spin-docs/pull/235)'s Middleware Permissions section for more information.

## Potential Future Work

### Non-interactive flags for scripting

Component selection, interface selection, capability inheritance, and middleware placement are interactive-only in the initial implementation, keeping the flag surface small and prompting for each decision. If CI/scripting use cases emerge, future work could reintroduce these as flags (e.g. `--to <component-id>`, `--import <name>`, `--inherit <value>`, and `--route <route>` / `--position <index>` for middleware) so an invocation can run without prompts.

### Multiple selections within a single package

The current design allows selecting either **all** exports from a package or a **single** specific interface. A natural extension would be to support selecting **multiple** (but not all) interfaces from the same package in a single invocation. For example, a multi-select prompt could allow the user to pick both `aws:client/s3@1.0.0` and `aws:client/dynamodb@1.0.0` without selecting the entire `aws:client@1.0.0` package. This would generate one dependency entry per selected interface and avoid requiring the user to run `spin deps add` multiple times for the same package.

### Applying middleware to multiple routes

The current middleware flow attaches a component to a single trigger per invocation. A natural extension would allow selecting multiple routes — or an "all routes" option — in one command, writing the same middleware entry into each matching trigger's pipeline. This would also help keep capabilities consistent across the components served by those routes.
