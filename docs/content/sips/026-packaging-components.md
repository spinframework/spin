title = "SIP 026 - Packaging Components"
template = "main"
date = "2026-08-25T00:00:00Z"
---

Summary: This SIP introduces a standalone component manifest (`component.toml`) and the CLI support to build and distribute an individual Spin component on its own, independent of any application. Reusable components — HTTP middleware being the prime example — can then be built, versioned, published to a registry, and pulled back down.

Owner(s): Michelle Dhanani <mdhanani@akamai.com>

Created: August 25, 2026

## Background

A Spin application is, today, always a *runnable* unit: the manifest is required
to declare one or more triggers, and the tooling (`spin up`, `spin build`,
`spin registry push`) is oriented around that assumption. This works well for
services, but it does not describe a component that is meant to be *consumed by
other applications* rather than run on its own (i.e. dependencies)

The motivating case is HTTP middleware. A component such as a GitHub OAuth gate
is not an application — it has no trigger and does nothing on its own — yet it is
highly reusable across applications. [SIP 020 (Component
Dependencies)](../sips/020-component-dependencies.md) already lets an application
consume such a component from a registry, and [SIP 024 (`spin deps` CLI
DX)](../sips/024-spin-deps-cli-dx.md) makes wiring one up interactive. The gap is
on the *author* side: there is no way to describe a single component, build it,
and publish it so that others can depend on it.

Tools such as wkg can already push and pull a bare component wasm to a registry. But a component alone advertises only its WIT world — the imports and exports it declares — not the operational configuration a consuming application must grant it: variables and secrets, allowed outbound hosts, key-value stores, SQLite databases, AI models, or mounted files. Spin needs a manifest that travels with the component so tooling can determine programmatically what a consumer must provide.

An earlier draft proposed a manifest dedicated to packaging *middleware*. But
nothing about building and distributing a component is specific to middleware: a
plain library component, a trigger-less utility, and a piece of HTTP middleware
all share the same needs. This SIP therefore supersedes that middleware-only
proposal with a general **component manifest** that applies to any component.

## Proposal

Introduce a standalone component manifest, `component.toml`, that describes a
single component: its identity, how to build it, and the host capabilities and
configuration it requires. Extend `spin build` to build from a component manifest and
embed required configuration in component binary (see [Alternatives considered](#alternatives-considered)), and extend `spin registry` to
publish and fetch components as registry packages.

### The component manifest (`component.toml`)

```toml
component_manifest_version = 1

[component]
name = "github-oauth"
source = "target/wasm32-wasip2/release/github_oauth.wasm"
version = "0.1.0"
description = "HTTP middleware that gates requests behind GitHub OAuth"
authors = ["Michelle Dhanani <mdhanani@akamai.com>"]
repository = "https://github.com/michellen/github-oauth-middleware"
license = "Apache-2.0"

[build]
command = "cargo build --target wasm32-wasip2 --release"

[requires]
variables = [
    "github_client_id",
    { name = "github_client_secret", secret = true },
]
allowed_outbound_hosts = ["https://github.com", "https://api.github.com"]
key_value_stores = ["default"]
sql_variables = ["default"]
ai_models = ["llama2-chat"]
environment_variables = ["staging", { name = "region", default = "us" }]
files = ["assets/**/*", { source = "local/path", destination = "/mounted/path" }]
```

The manifest is intentionally close to a single `[component.<id>]` entry in
`spin.toml`, but reorganised so a component can stand on its own.

#### `component_manifest_version`

`component_manifest_version = 1` identifies the file as a component manifest and
distinguishes it from an application manifest (`spin_manifest_version`). The
value is a fixed `1`; future revisions will bump it.

#### `[component]` — identity and artifact

| Field | Required | Description |
| --- | --- | --- |
| `name` | yes | The component's identifier. |
| `source` | yes | Path to the built Wasm artifact. Required because it is the file that is published and packaged. |
| `version` | for publishing | Semver version. Used as the version when publishing. |
| `description`, `authors`, `repository`, `license` | no | Human-readable metadata. |

`version` is only required when the component is published; a component that is only built locally may omit them.

`source` lives under `[component]` (not `[build]`) because it is the component's
artifact — the thing that is packaged — and exists whether or not the component
is built locally (for example, a pre-built component that is only being
republished).

#### `[build]` — how to build (optional)

Mirrors the `[component.<id>.build]` table in `spin.toml`:

| Field | Required | Description |
| --- | --- | --- |
| `command` | yes (if `[build]` present) | The build command, or an array of commands run in sequence. |
| `workdir` | no | Working directory for the build, relative to the manifest. |

`[build]` is optional: a pre-built component may be described and published with
only a `source`. A `watch` field (globs for `spin watch`) is intentionally *not*
included yet — `spin watch` does not operate on component manifests, so the field
would have no effect. It will be added together with `spin watch` support for
component manifests (see [Future work](#future-work)).

#### `[requires]` — host capabilities (optional)

Declares the capabilities the component expects the host application to grant it.
These mirror the capability fields of a `spin.toml` component:

| Field | Description |
| --- | --- |
| `variables` | Configuration variables the component consumes. Each entry is a bare name, or `{ name, default, secret }`. |
| `key_value_stores` | Key-value store labels the component accesses. |
| `sqlite_databases` | SQLite database labels the component accesses. |
| `ai_models` | AI models the component accesses. |
| `allowed_outbound_hosts` | Outbound network destinations the component is allowed to reach. |
| `environments_variables` | Environment variables the component needs. Each entry is a bare name, or `{ name, default }`. |
| `files` | Files the component may read: a glob, or `{ source, destination }` mount. |

`[requires]` is descriptive: it documents what an application must provide when it
adopts the component. It is consumed at application-assembly time (see [Future
work](#future-work)) rather than at build time.

### Building a component

`spin build` recognizes a component manifest, runs its `[build].command`, and embeds
the component manifest (omitted the `[build]` section) as JSON in a custom section of the built binary:

```console
$ spin build -f component.toml
Building component github-oauth with `cargo build --target wasm32-wasip2 --release`
Finished building all Spin components
```

When invoked without `-f`, `spin build` searches for a manifest, preferring an
application manifest (`spin.toml`) and falling back to a component manifest
(`component.toml`). If the component manifest has no `[build]` section, `spin
build` reports that there is nothing to build (the component is treated as
pre-built).

Only `spin build` operates on component manifests. `spin up`, `spin deploy`, and
similar commands continue to require an application manifest, because a lone
component has no trigger and cannot be run on its own.

### Publishing and fetching components

Reusable components are distributed as
[wasm-pkg](https://github.com/bytecodealliance/wasm-pkg-tools) component packages
— the same package format Spin already resolves when a component declares a
registry dependency (SIP 020). This makes a published component immediately
consumable as a dependency by other applications.

#### `spin registry push`

```console
$ spin registry push -f component.toml --registry ghcr.io/michellen/spin-components
Pushed component spin-components:github-oauth@0.1.0
```

- The published **package reference** is `namespace:name` where namespace is derived from
registry or is overriden by the `--package-namespace` flag, name is `[component].name` and
the version is `[component].version`.
- The component's built `source` must exist; otherwise Spin reports an error and
  suggests building first (`spin registry push --build`).
- `--build` performs a default `spin build` (component-aware) before publishing.
- `spin registry push` detects a component manifest and takes the component path;
  an application manifest continues to be pushed as a Spin application OCI
  artifact (with its registry reference argument), unchanged.

#### `spin registry pull`

```console
$ spin registry pull ghcr.io/michellen/spin-components/github-oauth:0.1.0 --output github-oauth.wasm
Pulled component to github-oauth.wasm
```

- The version portion is a semver requirement; when omitted, the latest
  non-yanked release is pulled.
- `--output` selects where the component Wasm is written; it defaults to
  `<name>.wasm` in the current directory.

## Relationship to other SIPs

- **[SIP 020 — Component Dependencies](../sips/020-component-dependencies.md):**
  this SIP is the producer side of that consumer feature. A component published
  here can be referenced in another application's `[component.dependencies]` and `[trigger.dependencies]`.
- **[SIP 024 — `spin deps` CLI DX](../sips/024-spin-deps-cli-dx.md):** components
  published here are exactly what `spin deps add` resolves and wires up,
  including HTTP middleware.
- **[SIP 008 — OCI registries](../sips/008-using-oci-registries.md):**
  application distribution continues to use OCI application artifacts.
  Component distribution uses wasm-pkg component packages so components are
  resolvable as dependencies.

## Future work

- **Assembling an application from `[requires]`.** Tooling like `spin deps` could
  read `[requires]` and scaffold or validate the host application's grants when a
  component is adopted.
- **`component.toml` discovery for more commands.** Only `spin build` and `spin
  registry` recognise component manifests today; `spin watch` and others could
  follow if there is demand.
- **Enabling composition** for standalone components. Components should be able to
  consume dependencies the same as a component in a Spin application manifest.

## Alternatives considered

- **Publish a component as a single-component Spin application.** A component
  could be wrapped in a synthetic `spin.toml` and pushed as an application OCI
  artifact. This reuses the application pipeline but produces an artifact that is
  semantically an *application* (with no trigger) and is **not** resolvable as a
  component dependency, defeating the primary purpose. Publishing a wasm-pkg
  component package instead makes the result directly consumable by spin, wkg and
  potentially other tools.
- **A middleware-specific manifest.** The original draft targeted middleware
  only. Since building and distributing a component is not middleware-specific,
  a general component manifest serves middleware and all other reusable
  components with one mechanism.
- **Using OCI annotations** to relay the `[requires]` information was considered rather
  than embedding component manifest in the component binary but that would lock component packages to OCI registries and we may want the option to distribute via github releases or other distribution platforms. 