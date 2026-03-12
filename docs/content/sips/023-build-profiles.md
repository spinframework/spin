title = "SIP 023 - Build profiles"
template = "main"
date = "2025-03-31T12:00:00Z"

---

Summary: Support defining multiple build profiles for components, and running a Spin application with those profiles.

Owner(s): [till@fermyon.com](mailto:till@fermyon.com)

Created: March 31, 2025

# Background

Building and running individual components or entire applications in configurations other than the default one is currently difficult to do with Spin: one has to manually edit the `[component.name.build]` tables for some or all components. This is laborious and error-prone, and can lead to accidentally committing and deploying debug/testing/profiling builds to production.

# Proposal

This proposal consists of two parts:

1. Adding an optional `[component.name.profile.profile-name]` table to use for defining additional build profiles of a component
2. Adding a `--profile [profile-name]` CLI flag to `spin {build, watch, up, deploy, registry push}` to use specific build profiles of all components (where available, with fallback to the default otherwise)

## Example

The following `spin.toml` file shows how these parts are applied:

```toml
# (Source: https://github.com/fermyon/ai-examples/blob/main/sentiment-analysis-ts/spin.toml,
#  with parts not relevant to profiles omitted.)

spin_manifest_version = 2

[application]
name = "sentiment-analysis"
...

[component.sentiment-analysis]
source = "target/spin-http-js.wasm"
...

[component.sentiment-analysis.build]
command = "npm run build"
watch = ["src/**/*", "package.json", "package-lock.json"]

# Debug build of the `sentiment-analysis` component:
[component.sentiment-analysis.profile.debug]
source = "target/spin-http-js.debug.wasm"

[component.sentiment-analysis.profile.debug.build]
command = "npm run build:debug"

# The `ui` component doesn't have a debug build, so the default build will always be used.
[component.ui]
source = { url = ".../spin_static_fs.wasm", digest = "..." }
...

[component.kv-explorer]
source = { url = ".../spin-kv-explorer.wasm", digest = "..." }
...

# Uses a pre-built debug version of the component.
[component.kv-explorer.profile.debug]
source = { url = ".../spin-kv-explorer.debug.wasm", digest = "..." }
```

The application defined in this manifest can be run in various configurations:

- `spin up`: uses the release/default builds for everything
- `spin up --profile debug`: uses builds of the profile named `debug` of all components that have them, default builds for the rest

## Details of profile selection

A profile can be selected `--profile=[profile-name]` CLI flag to `spin {build, up, watch, deploy, registry push}`

## Profile-overridable fields

This proposal is focused on build configurations. As such, the fields that are supported in `[component.component-name.profile.profile-name]` tables are limited to:

- `source`
- `build.command`
- `environment`
- `dependencies`

This set can be expanded in changes building on this initial support, as adding additional fields should always be backwards-compatible.

## Profiles are not atomic

When running an application with a named profile applied, components that don’t define that profile fall back to the default build configuration. All the individual fields in a profile are optional overrides.

## Deployment plugins

Spin transparently passes command arguments to deployment plugins when the `deploy` subcommand is invoked. It is the responsibility of each deployment plugin to interpret these arguments, handle different profiles, and understand the updated manifest structure accordingly.

# Alternatives considered

Instead of adding generalized profiles support, we could add just a hard-coded `debug` profile. This would simplify the syntax a little bit, but at the cost of flexibility.

We could do so by using syntax such as

```toml
[component.sentiment-analysis.debug]
...
```

and changing the `--profile [profile-name]` flag to instead be `--debug`, and `--component-profile sentiment-analysis=debug` be `--debug-component sentiment-analysis`.

I’m not proposing this alternative because the benefits seem much smaller than the downsides: it's useful to be able to use profiles with properties such as "optimize, but retain information required for profiling", or "skip debug asserts, but include debug symbols". Other build tools such a Rust's `cargo` started out with just `release` and `debug` and then had to retrofit support for custom profiles later on to satisfy this kind of requirement.
