title = "SIP 026 - Packaging Middleware Components"
template = "main"
date = "2026-08-14T00:00:00Z"
---

Summary: Introduce a first-class `middleware` application type so that reusable,
trigger-less components (for example authentication or request-enrichment logic)
can be built, published, and distributed with the existing Spin tooling.

Owner(s): [mdhanani@akamai.com](mailto:mdhanani@akamai.com)

Created: August 14, 2026

## Background

A Spin application is, today, always a *runnable* unit: the manifest is required
to declare one or more triggers, and the tooling (`spin up`, `spin build`,
`spin registry push`) is oriented around that assumption. This works well for
services, but it does not describe a component that is meant to be *consumed by
other applications* rather than run on its own (i.e. dependencies)

Concrete examples include:

* an OAuth / OpenID Connect handler that other apps wire in front of their routes,
* an authentication or authorization component shared across a team's services,
* request/response enrichment, logging, or policy components.

These "middleware" components are reusable building blocks. With
[SIP 024 - Spin Deps CLI](../sips/024-spin-deps-cli-dx.md) an application can fetch a middleware component
and add it to an application trigger. What is missing is a supported way to 
**author, build, and publish** such component as a standalone artifact.

Attempting to build and package one today in the `spin.toml` runs into the problem
that the manifest schema requires a `[[trigger.*]]` table.

Passing around the compiled `.wasm` component on its own is also not enough. A
component's WIT interfaces describe its imports and exports, but they say nothing
about the *operational configuration* a consumer must supply for it to actually
work — for example the application variables it requires (`github_client_id`, the
`github_client_secret` secret, `github_redirect_uri`), the outbound hosts it must
be allowed to reach, or the key-value stores it expects. That information lives in
the `spin.toml` manifest. Packaging the manifest together with the component lets
tooling **programmatically determine what configuration a user must provide** to
consume the middleware, instead of relying on out-of-band documentation or
trial-and-error at runtime.

This SIP proposes a small, explicit manifest addition that removes the trigger
requirement in the `spin.toml` while keeping all existing behavior for standard
applications, and describes how such components are distributed and consumed.

## Proposal

### The `middleware` application type

We add an optional `type` field to the `[application]` section of a v2 manifest.
For a standard application it may be omitted (or set to `"application"`); for a
middleware it is written as a table that names the main component:

```toml
spin_manifest_version = 2

[application]
name = "github-oauth-middleware"
version = "0.1.0"
# The application is a middleware whose main component is `github-oauth`.
# Other components in the manifest may be used as dependencies of it.
type = { middleware = "github-oauth" }

[variables]
github_client_id = { required = true }
github_client_secret = { required = true, secret = true }
github_redirect_uri = { required = true }

[component.github-oauth]
source = "github-oauth/target/wasm32-wasip2/release/github_oauth.wasm"
[component.github-oauth.build]
command = "cargo build --target wasm32-wasip2 --release"
watch = ["src/**/*.rs", "Cargo.toml"]
```

The `type` field accepts:

* `"application"` (the default, and may be omitted) — a standard, runnable Spin
  application. Behavior is unchanged; the manifest **must** declare at least one
  trigger.
* `{ middleware = "<component-id>" }` — a component intended for reuse.
  The manifest **may** omit triggers, and the value names the middleware's main
  component.

When `type` is omitted, the application is a standard application, so all
existing manifests are unaffected.

### The main middleware component

A middleware always has exactly one *main component* — the component that
"is" the middleware and whose exported interface(s) downstream applications
consume. Rather than a separate field, the main component is named directly on
the `type` line, so a middleware cannot be declared without one:

```toml
[application]
type = { middleware = "github-oauth" }   # main component = github-oauth
```

Any additional `[component.<id>]` entries in the manifest are permitted, but
they are *supporting* components: they exist to be wired in as
[dependencies](../sips/020-component-dependencies.md) of the main component
(for example, a shared client library the OAuth handler composes in). They are
not independently addressable as "the middleware."

### Manifest schema and validation

* A new `ApplicationType` enum is added, defaulting to `Application`. Its
  middleware variant carries the main component ID, so it serializes as
  `type = { middleware = "<id>" }`. The field is omitted from serialized output
  when it holds the default, preserving byte-compatibility for existing
  manifests and lockfiles.
* The `trigger` table becomes optional at the schema level (it defaults to an
  empty set) so that a middleware manifest parses cleanly.
* A validation step enforces the rule that *triggers are only optional for
  middleware*: loading a non-`middleware` manifest that declares neither triggers
  nor a trigger-type global config fails with a clear error:

  > application defines no triggers; only middleware applications (`type = { middleware = "..." }`) may omit triggers

* Because the main component is part of the `type` value, a middleware cannot be
  declared without naming one — `type = "middleware"` (the bare string form) is a
  parse error. The named component must also exist in the manifest; otherwise
  loading fails:

  > `type.middleware` refers to unknown component "…"

* The `type` field is a v2 concept only. A v1 manifest that includes a top-level
  `type` continues to be rejected by the existing "unknown field" error; no new
  v1 surface area is introduced.

### Building

`spin build` treats a middleware manifest as a fully valid application. Each
component's `[component.<id>.build]` command runs exactly as it does for a standard
app, and the manifest loads without warnings emitted.

```console
$ spin build
Building component github-oauth with `cargo build --target wasm32-wasip2 --release`
    Finished `release` profile [optimized] target(s)
Finished building all Spin components
```

### Distributing via OCI registries

Publishing reuses the OCI distribution mechanism from
[SIP 008](../sips/008-using-oci-registries.md) with no changes to the push/pull
path. Loading a middleware manifest produces a `LockedApp` with zero triggers
and the bundle's components intact; layer assembly and push operate per
component and never require a trigger.

```console
$ spin registry login ghcr.io
$ spin registry push --build ghcr.io/<org>/github-oauth-middleware:0.1.0
```

Notes:

* Push does not build by default; use `--build` or run `spin build` first.
* Push composes component dependencies by default (`--compose`). A middleware
  with no manifest dependencies composes to a no-op; `--compose=false` ships
  uncomposed layers.
* The pushed artifact carries the standard Spin application media types, so it is
  a normal registry reference that can be pulled and inspected.

### Consuming a published middleware

A downstream application consumes a published middleware through the existing
[component dependency](../sips/020-component-dependencies.md) mechanism, by
referencing the published component(s) from the registry and composing them to
satisfy an import:

```toml
[component.api.dependencies]
"example:auth/oauth" = { registry = "ghcr.io", package = "org:github-oauth-middleware", version = "0.1.0" }
```

Because a middleware artifact is a normal registry reference containing
components, no new resolution machinery is required: the middleware simply
provides the component(s) whose exported interfaces the consuming component
imports.

### Runtime semantics

A middleware component is not itself runnable. `spin up` on a middleware manifest
loads and locks successfully but stops with the existing `No triggers in app`
error, because there is nothing to trigger. This is the intended behavior: a
middleware is consumed by other applications, not executed directly.

## Rationale and alternatives

* **A library type field** to encompass all dependencies both middleware and library
  components. A valid argument can be made for this. An alias could also be an option.
  ` type = {library = "..."}`. I think we'll learn more as the deps CLI evolves and can
  defer this decision to the future.
* **A single explicit field vs. inference.** We could infer "middleware" from the
  absence of triggers. That is ambiguous — a trigger-less standard app is almost
  always a mistake, and we want to keep catching it. Deployment platforms may
  currently be relying on trigger information to efficiently store runnable components
  in an application. An explicit `type` makes intent unambiguous and lets validation
  stay strict for standard Spin apps.
* **A separate manifest kind / file.** Introducing a wholly separate manifest
  schema for reusable components would fragment tooling. Reusing the existing v2
  manifest with one additive field keeps `spin build`, `spin registry push`, and
  the component-dependency system working with minimal change.
* **Scope.** This SIP intentionally does not add new distribution or resolution
  formats; it leans on SIP 008 (OCI) and SIP 020 (component dependencies).

## Future work

* First-class `spin deps` ergonomics for inspecting and relaying required configuration
to consumer (e.g. variables required)
* Guidance and templates (`spin new`, `spin add`) for authoring middleware components.
* Tooling for locally testing standalone middleware components.