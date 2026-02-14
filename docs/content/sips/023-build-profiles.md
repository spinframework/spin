title = "SIP 023 - Build profiles"
template = "main"
date = "2025-03-31T12:00:00Z"

---

Summary: Support defining multiple build profiles for components, and running a Spin application in with those profiles.

Owner(s): [till@fermyon.com](mailto:till@fermyon.com)

Created: March 31, 2025

# Background

Building and running individual components or entire applications in configurations other than the default one is currently difficult to do with Spin: one has to manually edit the `[component.name.build]` tables for some or all components. This is laborious and error-prone, and can lead to accidentally committing and deploying debug/testing/profiling builds to production.

# Proposal

This very simple proposal consists of four parts:

1. Adding an optional `[component.name.profile.profile-name]` table to use for defining additional build profiles of a component
2. Adding a `-profile [profile-name]` CLI flag and `SPIN_PROFILE` env var to `spin {build, watch, up}` to use specific build profiles of all components (where available, with fallback to the default otherwise)
   1. By popular demand, the `debug` profile can be selected using the `--debug` CLI flag
   2. `release` is the default profile, but can also be explicitly named for regularity, including with the `--release` alias
3. Adding a (repeatable) `-component-profile [component-name]=[profile-name]` CLI flag and `SPIN_COMPONENT_PROFILE="[component-name]=[profile-name],..."` env var to use specific profiles for specific components
4. Any operations that publish a Spin application to a remote location (such as `spin registry push`) must implicitly run a `spin build --release` step before publishing build results

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
watch = ["src/**/*", "package.json", "package-lock.json"]

# The `ui` component doesn't have a debug build, so the release build will always be used.
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
- `spin up --profile debug` or `spin up --debug`: uses builds of the profile named `debug` of all components that have them, default builds for the rest
- `spin up --component-profile sentiment-analysis=debug`: uses the debug build of just the `sentiment-analysis` component, default builds for everything else

## Details of profile selection

### Application-wide profile selection

A profile can be selected for the entire application in three ways:
1. Via the `--profile=[profile-name]` CLI flag to `spin {build, up, watch}`
2. Via the `--debug` CLI flag to `spin {build, up, watch}` as an alias for `--profile=debug`
3. Via the `SPIN_PROFILE=[profile-name]` env var

If the `SPIN_PROFILE` env var is set and `--profile` / `--debug` are passed as CLI flags, the value of the CLI flag is used, and the env var ignored. `--profile` and `--debug` are mutually exclusive and lead `spin` to exit with an error.

### Component-specific profile selection

A profile can be selected for a specific component in two ways:
1. Via the `--component-profile [component-name]=[profile-name],...` CLI flag
2. Via the `SPIN_COMPONENT_PROFILE="[component-name]=[profile-name],..."` env var

The CLI flag is repeatable, but can also support multiple comma-separated entries in a single instance.

If both the `SPIN_COMPONENT_PROFILE` env var is set and `--component-profile` provided, they will be merged into a single list, with settings from the CLI flag taking precedence: if a profile is specified in both for the same component, the value from the CLI flag is used.

Component-specific profile selection overrides the application-wide profile. E.g. the command `spin up --debug --component-profile foo=release` will cause all components to use the `debug` profile, except for the `foo` component, which will use the default/`release` profile.

## Profile-overridable fields

This proposal is focused on build configurations. As such, the fields that are supported in `[component.component-name.profile.profile-name]` tables are limited to:

- `source`
- `command`
- `watch`

This set can be expanded in changes building on this initial support, as adding additional fields should always be backwards-compatible.

## Profiles are atomic

When running an application with a named profile applied, components that don’t define that profile fall back to the default build configuration. The same isn’t true for individual fields in a profile: if e.g. a profile contains the `source` field but no `command` field, running `spin build` will not try to run the default configuration’s build command. That also means that if a field is required, it must be included in all profiles, and omitting it in any profiles will cause the manifest to be rejected.

## Local testing only

At least for now, this proposal is focused entirely on local testing: deployment through plugins is not proposed to gain support for a `--profile` flag.

## Deploy implies release build

To ensure that deployment operations always publishes up-to-date artifacts, they must implicitly run a build of the release profile of all components before performing the publishing itself. E.g., where currently one has to run `spin build && spin registry push` to ensure that the latest source code is compiled before publishing `.wasm` components to a registry, implementing this proposal would change behavior such that `spin registry push` automatically runs `SPIN_COMPONENT_PROFILE="" spin build --release`.

We probably can't apply this to plugin-based deployment operations automatically, but should ensure that we provide documentation to plugin authors that instructs them to do this, and ideally an easy way to do it via a well-named function in the API.

# Backwards-compatibility

This proposal should be fully backwards-compatible in practice, but entails a change in behavior around publishing operations, as discussed in the previous section.

# Implementation concerns

I can't see any major implementation concerns: this should hopefully be pretty straightforward.

# Alternatives considered

Instead of adding generalized profiles support, we could add just a hard-coded `debug` profile. This would simplify the syntax a little bit, but at the cost of flexibility.

We could do so by using syntax such as

```toml
[component.sentiment-analysis.debug]
...
```

and changing the `--profile [profile-name]` flag to instead be `--debug`, and `--component-profile sentiment-analysis=debug` be `--debug-component sentiment-analysis`.

I’m not proposing this alternative because the benefits seem much smaller than the downsides: it's useful to be able to use profiles with properties such as "optimize, but retain information required for profiling", or "skip debug asserts, but include debug symbols". Other build tools such a Rust's `cargo` started out with just `release` and `debug` and then had to retrofit support for custom profiles later on to satisfy this kind of requirement.
