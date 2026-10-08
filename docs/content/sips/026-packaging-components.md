title = "SIP 026 - Packaging Components"
template = "main"
date = "2026-08-25T00:00:00Z"
---

Summary: This SIP introduces a standalone component manifest (`component.toml`) and the
CLI support to build and distribute an individual Spin component on its own, independent
of any application. Reusable components — HTTP middleware being the prime example — can
then be built, versioned, published to a registry, and pulled back down.

# Background

A Spin application is, today, always a *runnable* unit: the manifest is required
to declare one or more triggers, and the tooling (`spin up`, `spin build`,
`spin registry push`) is oriented around that assumption. This works well for
services, but it does not describe a component that is meant to be *consumed by
other applications* rather than run on its own (i.e. dependencies)

A component such as a GitHub OAuth gate is not an application — it
has no trigger and does nothing on its own — yet it is highly reusable
across applications. [SIP 020 (Component Dependencies)](../sips/020-component-dependencies.md)
already lets an application consume such a component from a registry, and
[SIP 024 (`spin deps` CLI DX)](../sips/024-spin-deps-cli-dx.md) makes wiring
one up interactive. The gap is on the *author* side: there is no way to describe
and build a single component.

A Wasm component binary can already be pushed to / pulled from an OCI registry (i.e. wkg).
But a component binary alone advertises only its WIT world — the imports and exports it declares.
To be convenient to consume, a component also needs to describe its operational requirements:
that is, the configuration a consuming application must grant it. Such configuration includes
the variables and secrets the component needs to access, the network hosts it needs to make
calls to, storage, or mounted files.

In addition, it would be nice to be able to use the Spin `build-up-push` workflow for
library component development.

# Describing components

Describing operational requirements falls into two halves:

1. Authoring the requirements, e.g. using a TOML file
2. Embedding the requirement, e.g. using a custom section in the Wasm binary (see [Alternatives considered](#alternatives-considered))

# Authoring components

We propose to a standalone component manifest, `component.toml`, that describes a
single component: its identity, how to build it, and the host capabilities and
configuration it requires.  `spin build` will understand the component manifest
format and will:

1. Build the component as a standalone Wasm file
2. Embed required configuration in component binary

In addition, `spin registry push` will be updated to publish and fetch components
to an OCI registry. The registry behaviour should interoperate with `wkg` based
workflows because the file is just a Wasm binary, albeit potentially with a custom
section.

(Note that `spin up` cannot, in general, run components directly: the component
needs to be included in an application for that to work, e.g. for testing.)

## The component manifest (`component.toml`)

```toml
component_manifest_version = 1

[component]
name = "mycomponents:github-oauth"
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
sqlite_databases = ["default"]
ai_models = ["llama2-chat"]
environment = ["staging", { name = "region", default = "us" }]
files = [{ destination = "/mounted/path" }]
```

The manifest is intentionally close to a single `[component.<id>]` entry in
`spin.toml`, but reorganised so a component can stand on its own.

### Open questions on manifest format

1. Is `license` meaningful in this context? We are referring to a binary
   distribution.
2. Is it worth having a separate URL for docs or other information, as
   distinct from the source code repository?
3. Should `component.toml` support build profiles?

### `component_manifest_version`

`component_manifest_version = 1` identifies the file as a component manifest and
distinguishes it from an application manifest (`spin_manifest_version`). The
value is a fixed `1`; future revisions will bump it.

### `[component]` — identity and artifact

| Field | Required | Description |
| --- | --- | --- |
| `name` | yes | The component's package identity, `<namespace>:<name>` (for example `mycomponents:github-oauth`). Both `<namespace>` and `<name>` must be valid [package identifier](https://component-model.bytecodealliance.org/design/packages.html) segments. This is the identity under which the component is published and by which consumers depend on it. |
| `source` | yes | Path to the built Wasm artifact. Required because it is the file that is published and packaged. |
| `version` | for publishing | Semver version. Used as the version when publishing. |
| `description`, `authors`, `repository`, `license` | no | Human-readable metadata. |

`version` is only required when the component is published; a component that is only built locally may omit them.

`source` lives under `[component]` (not `[build]`) because it is the component's
artifact — the thing that is packaged — and exists whether or not the component
is built locally (for example, a pre-built component that is only being
republished).

### `[build]` — how to build (optional)

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

### `[requires]` — host capabilities (optional)

Declares the capabilities the component expects the host application to grant it.

| Field | Description |
| --- | --- |
| `variables` | Configuration variables the component consumes. Each entry is a bare name, or `{ name, default, secret }`. |
| `key_value_stores` | Key-value store labels the component accesses. |
| `sqlite_databases` | SQLite database labels the component accesses. |
| `ai_models` | AI models the component accesses. |
| `allowed_outbound_hosts` | Outbound network destinations the component is allowed to reach. |
| `environment` | Environment variables the component needs. Each entry is a bare name, or `{ name, default }`. |
| `files` | Guest paths the component reads. The component declares only the path it expects to find files at; the consuming application decides what to mount there. |

`[requires]` is descriptive: it documents what an application must provide when it
adopts the component. It is consumed at application-assembly time (see [Future
work](#future-work)) rather than at build time.

### Open question: Variables interpolation in `requires` fields

Spin applications allow variables to be interpolated in some fields - for example,
`allowed_outbound_hosts = ["https://{{ auth_server }}"]`. Spin app interpolation uses
application variables (operator-facing configuration knobs) rather than component variables
(guest-facing values). But the author of a standalone component doesn't know
what configuration knobs the application will define.

Options:

1. Standalone components cannot use interpolation. This seems like a frustrating
   limitation. For example, an authentication middleware may be designed to be reusable
   across different auth services.
2. Interpolation uses component variables. This might work. It does mix concerns -
   it means that any string used for interpolation will also be available to the
   guest. But perhaps that's okay.
3. Have two variables sections - configuration knobs and guest-facing variables.
   Config knobs could be set only by the application (or defaulted); guest-facing
   variables could be derived from config knobs or set directly by
   the application. This replicates the app/component variables distinction that
   Spin draws, but perhaps it's onerous and confusing?
4. A variation of 2 and 3 is to have a single variables section but have a new
   variable field "do/don't surface to the guest."

### Open question: `files` and static assets

The current specification for `files` says "these are the directories I'm gonna look
at, put stuff there if you want me to find it." It is on the application to populate
these directories. But what the component depends on static assets, e.g. a geolocation
component that depends on a database, or an auth component which wants to return a
cheerfully coloured SVG of a raised middle finger?

This can be handled in Rust by using `include_*` macros, and maybe that's enough for
now.

An alternative is to embed static files in another custom section, in which case we
need a way for the manifest to specify files to be included at build time
(and upacked at run time), as opposed to being supplied by the application.
If we want to allow for this, it would be good to define it now so we don't need
to rev the manifest in three weeks' time.

# Building components

`spin build` recognizes a component manifest by file name and manifest version declaration, runs its `[build].command`, and embeds
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

After running the build commands, `spin build` embeds the requirements in a custom
section as discussed in the next section.

## Self-describing binary format

When we build one of these component manifests, the result is a standalone binary which can
be included in an application, consumed by `spin deps`, etc.  However, we want the binary to
describe its requirements independently of the TOML manifest.

When `spin build` builds a standalone component (as opposed to an application component),
it will add a custom section named `spin:requires`.  The contents of the section are a
JSON document, equivalent to serialising the `[requires]` section of the manifest:

| Key                   | Type             | Value  |
|-----------------------|------------------|--------|
| `format_version`      | Number           | The `component_manifest_version` that should be used to interpret the remaining fields. Currently must be `1` |
| `variables`           | Array (strings or tables) | Variables the component consumes. Table entries are `{ name, default, secret }` |
| `key_value_stores`    | Array of strings | Key-value store labels the component accesses. |
| `sqlite_databases`    | Array of strings | SQLite database labels the component accesses. |
| `ai_models`           | Array of strings | LLMs the component accesses. |
| `allowed_outbound_hosts` | Array of strings | Outbound network destinations the component wants to be able reach. |
| `environment`         | Array (strings or tables) | Environment variables the component needs. Table entries are `{ name, default }` |
| `files`               | Array of strings | Guest paths the component reads. |

## Open question: `spin build` is optional

We have previously avoided requiring people to run `spin build`, so that they can use
more _cough_ fully-featured build systems instead. If `spin build` embeds the
`requires` section, then developers are forced to use `spin build`.  Options:

1. This is acceptable for now. Developers can call `spin build` from their fancy
   schmancy build systems for now, and we will listen for feedback on if
   this works for them or if we need to do more.
2. Embed the custom section during `spin registry push`. But this ties us to OCI
   for distribution: a component referenced via a GitHub release asset URL could
   not be self-describing. Additionally, it means that the binary being deployed
   is not the one you tested with, and that could be a mare to debug if something
   went wrong (heaven forfend).
3. Provide a command (or command option) to inject the custom section into an
   existing Wasm file built by another source. E.g. `spin build self-describe foo.wasm -f component.toml`

# Local experience

Again, `spin up` cannot be used with component manifests: it continues to require
an application manifest, because a lone component has no trigger and cannot be run on its own.
(We _could_ allow `spin up` to operate as if on a bare Wasm file, but given that
component manifests are typically for building middleware or libraries, this is
likely to produce confusing errors about WASI interfaces: by disallowing it, we
can provide meaningful errors.)

So the current test story is "build the component separately and then reference
it in your test app" (as a component, middleware or a dependency).

Future work could allow for referencing component manifests in a `spin.toml`,
bringing us closer to the much-longed-for manifest modularisation story.
But we will need to think about how to handle component `requires` and app
fulfilment of `requires`, in a way that is not too onerous.

# Distributing components

Reusable components are published to and pulled from an OCI registry — the same
registries Spin already resolves against when a component declares a registry
dependency (SIP 020). Publishing a component this way makes it immediately
consumable as a dependency by other applications.

## Open question: OCI reference or `wkg` package name?

The Spin manifest allows users to reference registry packages by registry and
package name - that is, a `wkg` style reference. The current `spin registry push`
code deals only in OCI-style references (`ghcr.io/itowlson/myapp:1`). While
a `wkg` registry can be - and normally is - backed by OCI, we need to define
which is these naming formats we want to use.

## `spin registry push`

```console
$ spin registry push -f component.toml --registry ghcr.io/michellen
Pushed component mycomponents:github-oauth@0.1.0 to ghcr.io/michellen/github-oauth:0.1.0
```

- To be consumable as a registry dependency, a component is published under the
  package identity `<namespace>:<name>` declared by `[component].name`. The
  package identity is metadata carried with the component; it does not dictate
  the registry path.
- The published **reference** is `<registry>/<name>:<version>`, where
  `<registry>` is the value of `--registry`, `<name>` is the `<name>` segment of
  `[component].name`, and `<version>` is `[component].version`.
- The component's built `source` must exist; otherwise Spin reports an error and
  suggests building first (`spin registry push --build`).
- `--build` performs a default `spin build` (component-aware) before publishing.
- `spin registry push` detects a component manifest and takes the component path;
  an application manifest continues to be pushed as a Spin application OCI
  artifact (with its registry reference argument), unchanged.

## `spin registry pull`

```console
$ spin registry pull ghcr.io/michellen/github-oauth:0.1.0 --output github-oauth.wasm
Pulled component mycomponents:github-oauth@0.1.0 to github-oauth.wasm
```

- The version portion is a semver requirement; when omitted, the latest
  release is pulled.
- `--output` selects where the component Wasm is written; it defaults to
  `<name>.wasm` in the current directory.

# Open question: integration with `spin.toml`

There are two scenarios for referencing component manifests from `spin.toml`:

1. **Testing:** I want to use a component manifest in `spin.toml`, but this is
   primarily to give me a local dev experience for the standalone component.
2. **Modularisation:** I want to use a component manifest to move information out of
   a long `spin.toml`, so that my `spin.toml` gets shorter (and I have no
   plans to publish the moved-out component as a standalone).

In the standalone test scenario, the application does not fully trust
the component (or at least pretends not to fully trust it), and so permission grants
are reserved to the application: the component cannot grant itself permissions, only
advertise the permissions it needs.

In the modularisation scenario, the application and component are both within
the trust boundary. It's purely a matter of convenience to break up a big
manifest (ease of reading, history/diffing, etc.). The component can grant itself
permissions just as it could if it was inline in `spin.toml`.

I am not sure that the current proposal can be readily reconciled with the
modularisation requirement, because the current proposal would require a separate
document (either `spin.toml` or some other modularisation helper) to fulfil
the `[requires]` section: the developer writes the same things twice, first as
requirements and then as grants. This is onerous.

That said, there is something to be said for a trust boundary. Suppose I build
an app and write an auth component using modularisation style. Later, I need
an auth component for another app: oh, I already wrote that, let me copy the
directory across. If my auth component grants itself permissions, I've accidentally
copied those permissions into my new app. But I'm not sure if this is truly a huge concern:
copying and pasting without reviewing is always going to risk an "oops I forgot
it did that."

It has been suggested that a `[requires]` style component manifest could provide
defaults, which a modularisation use case could accept or override. I am suspicious
of this because it provides a binary component with a way to smuggle self-granted
permissions past a developer who is not on the lookout for such things (and really,
who is). The same commenter noted this, and suggested that defaults could be offered
as part of an interactive `spin deps add` rather than granted automatically. But
then the modularisation use case is back to "the application has to recapitulate
what it says in the component" and our main `spin.toml` barely gets shorter at all.

TLDR: I don't think we have a good answer to this yet. The two scenarios seem
naggingly similar but... maybe they're not?

# Relationship to other SIPs

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

# Future work

- **Assembling an application from `[requires]`.** Tooling like `spin deps` could
  read `[requires]` and scaffold or validate the host application's grants when a
  component is adopted.
- **Fetching and inspecting components** with the spin deps CLI.
- **Enabling composition** for standalone components. Standalone components today
  cannot have dependencies but components should be able to consume dependencies
  the same as a component in a Spin application manifest.
- **Split out the requirements embedder.** So that developers can build the Wasm
  binary with their favourite build tool, then add the requirements from the manifest,
  rather than being forced to use `spin build`.

# Alternatives considered

- **Publish a component as a single-component Spin application.** A component
  could be wrapped in a synthetic `spin.toml` and pushed as an application OCI
  artifact. This reuses the application pipeline but produces an artifact that is
  semantically an *application* (with no trigger) and is **not** resolvable as a
  component dependency, defeating the primary purpose. Publishing a wasm-pkg
  component package instead makes the result directly consumable by Spin, wkg and
  potentially other tools.
- **A middleware-specific manifest.** The original draft targeted middleware
  only. Since building and distributing a component is not middleware-specific,
  a general component manifest serves middleware and all other reusable
  components with one mechanism.
- **Using OCI annotations** to relay the `[requires]` information was considered
  rather than embedding component manifest in the component binary but that would
  lock component packages to OCI registries and we may want the option to distribute
  via github releases or other distribution platforms.
 