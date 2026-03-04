title = "SIP 022 - Environment Variable Isolation for Composed Components"
template = "main"
date = "2026-02-15T00:00:00Z"
---

Summary: Automatically isolate WASI environment variables within a component composition so that each component in a dependency graph sees only its own declared variables, despite sharing a single host process.

Owner(s): till@tillschneidereit.net

Created: Feb 15, 2026

## Background

Spin applications can compose multiple WebAssembly components together using component dependencies (see [SIP 020](../sips/020-component-dependencies.md)). When a parent component declares a dependency, Spin statically composes the two components into a single Wasm component at build time or at load time.

A consequence of this composition is that all components in the resulting graph share the same host `wasi:cli/environment` import. Any environment variables set on the parent component are visible to every dependency, and dependencies cannot have their own distinct environment configurations. This is a problem for several reasons:

1. **Leaking configuration.** A parent component's secrets or internal configuration (e.g. API keys set via environment variables) are visible to dependencies that should not see them.

2. **No per-dependency configuration.** Reusable library components that are parameterised by environment variables cannot be given different values when the same library is used by different parent components, or when a dependency needs different settings than its parent.

3. **Name collisions.** Two components that both read `DATABASE_URL` will see the same value, with no way to differentiate.

## Proposal

To address these issues, this SIP proposes to automatically isolate the environment variables of a component and all its dependencies from each other.

At a high level, the isolation works by adding prefixes based on the components' name / dependency IDs when loading an application manifest, and then filtering by those prefixes in a synthesized component that interposes imports of `wasi:cli/environment`, removing the prefix of allowed variables before forwarding them.

This process introduces a single very small (about 1.5KB to 2KB) core WebAssembly module to the composed graph, containing a 1-page (64KB) linear memory. Actual resource allocation should normally be a small fraction of those 64KB, unless the component graph has large amounts of data passed in as env vars.

### Manifest changes

A new `environment` field is added to each dependency specification within the `[component.<id>.dependencies]` section. This field is an inline table of string key-value pairs, identical in syntax to the existing component-level `environment` field:

```toml
spin_manifest_version = 2

[application]
name = "my-app"
version = "1.0.0"

[[trigger.http]]
route = "/..."
component = "main"

[component.main]
source = "main.wasm"
environment = { GREETING = "hello from main", MAIN_ONLY = "secret" }

[component.main.dependencies."acme:lib/helper"]
component = "helper"
environment = { GREETING = "hello from helper", HELPER_SETTING = "42" }

[component.helper]
source = "helper.wasm"
```

Additionally, components are now allowed to specify their own `environment` table and still be used as dependencies, instead of being rejected. When a dependency references another component in the same application (the `component = "..."` form), the referenced component may itself declare an `environment`. In that case, the two sets of variables are merged: the referenced component's `environment` provides the base values, and the dependency specification's `environment` provides overrides and additions. This merge happens during manifest normalisation, before any composition takes place.

For example, given:

```toml
[component.helper]
source = "helper.wasm"
environment = { GREETING = "default greeting", LOG_LEVEL = "info" }

[component.main.dependencies."acme:lib/helper"]
component = "helper"
environment = { GREETING = "overridden greeting" }
```

The effective environment for the `helper` dependency is `{ GREETING = "overridden greeting", LOG_LEVEL = "info" }`.

The `environment` field is available on all dependency forms (`component`, `path`, `url`, and registry version specs), consistent with the existing `export` field.

### How isolation works

Spin automatically activates environment variable isolation for compositions with at least one dependency. The mechanism works in two coordinated stages:

#### Stage 1: Prefixed environment variables (manifest load time)

At manifest load time, Spin's WASI factor prepends a deterministic prefix to every environment variable key before placing it in the WASI context. The prefix is derived from the component's ID (for the main component) or the dependency name (for dependencies):

- Convert the last segment of the name (after the final `/` if present) to uppercase.
- Replace non-alphanumeric characters with underscores.
- Append a trailing underscore.

Examples:
| Component / Dependency Name | Prefix |
|---|---|
| `main` | `MAIN_` |
| `my-service` | `MY_SERVICE_` |
| `acme:lib/helper` | `HELPER_` |
| `hello:components/dependable` | `DEPENDABLE_` |

The resulting flat list of environment variables is used in the locked application, so except for stage 2 below, all further parts of the processing pipeline aren't changed in any way, which results in the WASI environment being populated with the prefixed variables.

Given the manifest example above, the actual WASI environment would contain:

```
MAIN_GREETING=hello from main
MAIN_MAIN_ONLY=secret
HELPER_GREETING=hello from helper
HELPER_HELPER_SETTING=42
```

**NOTE**: Dynamically provided environment variables, e.g. those added using `spin up -e SOME=var` aren't automatically prefixed, and as a result not visible to any component, unless they already have the right prefix. E.g. to expose `SOME` to the `main` component, it has to be changed to `MAIN_SOME`.

#### Stage 2: Isolator component (composition time)

At composition time, Spin generates and wires in a synthetic WebAssembly component — the **env-isolator** — that interposes on each component's `wasi:cli/environment` import. For each component in the composition, the isolator:

1. Intercepts calls to `get-environment`.
2. Filters the flat environment to only those keys matching the component's prefix.
3. Strips the prefix before returning the key-value pairs.

The result is that each component sees only its own environment variables, with unprefixed keys, as if those were the only variables that existed.

The isolator also passes through `get-arguments` and `initial-cwd` without modification, since these are process-level values that do not benefit from per-component filtering.

Components that do not import `wasi:cli/environment` are skipped; no isolator is wired up for them.

### Activation rule

Environment variable isolation is activated **automatically** for components with at least one dependency. When active, **all** components in the composition are isolated — including the main component. A component with no declared `environment` will see an empty environment when isolation is active.

## Implementation

The implementation spans five crates:

### `spin-env-isolator` (new crate)

A new `crates/env-isolator/` crate containing the core Wasm generation logic. It has no dependency on Spin's application model and can be tested independently. It provides:

- **`compute_prefix(id: &str) -> String`** — The shared prefix derivation function used by both the composition pipeline and the runtime factor, guaranteeing agreement.

- **`isolator::generate_isolator(targets, wasi_env_version) -> Vec<u8>`** — Generates the isolator component. Each `IsolationTarget` specifies a name and prefix. The generated component imports `wasi:cli/environment@{version}` and exports flat functions (`environment-{name}-get-environment`, `environment-{name}-get-arguments`, `environment-{name}-get-cwd`) for each target.

- **`wrapper::build_env_wrapper_component(name, wasi_env_version) -> Vec<u8>`** — Generates a small adapter component per target that imports the flat functions from the isolator and re-exports them bundled as a `wasi:cli/environment` instance, which can be wired into the target component's import. Note that this wrapper component only contains component model instructions, no core WebAssembly bytecode, and is fully erased at link time without leaving overhead.

- **`core_module`** — Generates the core Wasm module that performs the actual byte-level prefix matching and stripping in linear memory, operating on the canonical ABI representation of `list<tuple<string, string>>`.

### `spin-manifest`

- The `ComponentDependency` enum gains an `environment: Map<String, String>` field on its `Package`, `Local`, `HTTP`, and `AppComponent` variants.
- A `ComponentDependency::environment()` accessor method is added.
- In `normalize.rs`, `ensure_is_acceptable_dependency` is relaxed to allow dependency components to declare `environment` (other restrictions — files, networking, key-value, etc. — remain).
- During normalisation of `AppComponent` references, the referenced component's `environment` is merged with the dependency spec's `environment` (spec values take precedence), and the merged result is carried forward into the concrete dependency variant.

### `spin-loader`

- `load_component` is changed to apply prefixes to all components' environment variable names if at least one dependency exists.

### `spin-compose`

- For components with at least one dependency, it:
  1. Detects the WASI `cli/environment` version from component imports.
  2. Generates the isolator component via `spin_env_isolator::isolator::generate_isolator`.
  3. For each target, generates a wrapper component via `spin_env_isolator::wrapper::build_env_wrapper_component`.
  4. Wires isolator → wrapper → target in the composition graph, replacing each target's `wasi:cli/environment` import.

## Example

The following complete example demonstrates the feature. Given two components — `main` (an HTTP handler) and `dependable` (a library) — where each reads environment variables:

**`spin.toml`:**

```toml
spin_manifest_version = 2

[application]
name = "env-isolator-hello"
version = "0.1.0"

[[trigger.http]]
route = "/..."
component = "main"

[component.main]
source = "target/wasm32-wasip2/release/main.wasm"
environment = { GREETING = "hello from main", MAIN_ONLY = "only visible to main" }

[component.main.build]
command = "cargo build --target wasm32-wasip2 --release"

[component.main.dependencies."hello:components/dependable"]
component = "dependable"
environment = { GREETING = "hello from dependable", DEPENDABLE_ONLY = "only visible to dependable" }

[component.dependable]
source = "target/wasm32-wasip2/release/dependable.wasm"

[component.dependable.build]
command = "cargo build --target wasm32-wasip2 --release"
```

Both `main` and `dependable` call `std::env::vars()`. A request to `http://127.0.0.1:3000/` returns:

```
main's env vars: GREETING='hello from main', MAIN_ONLY='only visible to main'
dependable's env vars: GREETING='hello from dependable', DEPENDABLE_ONLY='only visible to dependable'
```

Each component sees only its own variables. The shared key `GREETING` has a different value for each, and `MAIN_ONLY` / `DEPENDABLE_ONLY` are invisible to the other component.

## Design decisions

### Automatic activation

Environment isolation is activated implicitly when any component in a composition imports `wasi:cli/environment`, without an opt-out. This choice follows from the goal to make safe behavior the default or even enforced, but could be weakened by providing an opt-out.

### Prefix is not user-configurable

The prefix is derived deterministically from the component or dependency name. Exposing the prefix to users would add configuration surface for an implementation detail that has no user-visible effect (components never see the prefix). Keeping it internal simplifies the UX and prevents misconfiguration.

### Only environment variables are isolated

This proposal isolates only `wasi:cli/environment`. Other WASI capabilities (filesystem, networking, etc.) continue to be managed by the existing `dependencies_inherit_configuration` / deny-all adapter mechanism from SIP 020. Isolating those would require different interposition strategies and is out of scope.

### Backward compatibility

There are two compatibility-breaking aspects to this change:
1. The main component's `environment` isn't visible to dependencies anymore, so existing applications that assume it is won't work as expected.
2. Environment variables provided dynamically, e.g. using the `spin up -e` need to be prefixed to be exposed to specific components.

At the purely syntactical manifest level the change is backwards-compatible: the `environment` table in a dependency spec is optional, as is the `env` table in the lock file.

## Future possibilities

- **Template expressions in environment.** The `environment` field currently accepts only static strings. A future enhancement could allow Spin template expressions (e.g. `{{ my_variable }}`), leveraging the existing variables/expressions system for dynamic configuration.

- **Transitive dependency isolation.** The current implementation handles only direct dependencies. As the component model ecosystem matures and compositions become deeper, extending isolation to transitive dependencies may become necessary.
