title = "SIP 022 - Modularizing spin.toml"
template = "main"
date = "2025-03-31T12:00:00Z"

---

Summary: Support moving component definitions into separate `spin.toml` files, similar to Cargo workspaces.

Owner(s): [till@fermyon.com](mailto:till@fermyon.com)

Created: March 31, 2025

# Background

As Spin applications grow in complexity, their `spin.toml` manifest files can become large and unwieldy. Large applications with many triggers and components can result in manifest files that span hundreds of lines, making them difficult to navigate, understand, and maintain. It’s very hard to keep related settings grouped, making the application architecture harder to follow.

The current monolithic nature of `spin.toml` files also means that they tend to be very conservative when it comes to adding additional features that’d require additions to the manifest. This proposal is directly motivated by the desire to add features that’d make `spin.toml` files in their current form substantially more unwieldy. Those will be described in separate proposals.

Besides specific current goals, moving to a modular manifest format will bring Spin into alignment with related other formats, such as Rust’s [Cargo.toml](https://doc.rust-lang.org/cargo/reference/workspaces.html) or npm’s [package.json](https://docs.npmjs.com/cli/v11/using-npm/workspaces).

Modularizing `spin.toml` files has come up previously, with an attempt at implementing them [here](https://github.com/spinframework/spin/pull/2396).

# Proposal

`spin.toml` files contain multiple different kinds of items: application-level metadata, variables, trigger definitions, and component definitions. I propose to support modularizing manifests in the following way:

- Manifests can only be nested one level deep: an application can have a single top-level manifest, and zero or more direct sub-manifests, which cannot have any sub-manifests themselves
- Application-level metadata must fully be contained in the application’s top-level manifest
- Triggers must be defined in the top-level manifest
- Variables can be contained in all manifests
    - A variable defined in the top-level manifest is visible in all manifests
    - A variable defined in a sub-manifest is only visible in that manifest
- Components can be defined in the top-level, and in sub-manifests
    - Only a single component can be defined in each sub-manifest

## Example

To give a high-level overview of the proposal’s impact before discussing the details of the syntax, let’s look at an [existing](https://github.com/fermyon/ai-examples/blob/main/sentiment-analysis-ts/spin.toml) `spin.toml` file first:

```toml
# (Source: https://github.com/fermyon/ai-examples/blob/main/sentiment-analysis-ts/spin.toml)
spin_manifest_version = 2

[application]
name = "sentiment-analysis"
version = "0.1.0"
authors = ["Caleb Schoepp <caleb.schoepp@fermyon.com>"]
description = "A sentiment analysis API that demonstrates using LLM inference and KV stores together"

[[trigger.http]]
route = "/api/..."
component = "sentiment-analysis"

[component.sentiment-analysis]
source = "target/spin-http-js.wasm"
allowed_outbound_hosts = []
exclude_files = ["**/node_modules"]
key_value_stores = ["default"]
ai_models = ["llama2-chat"]

[component.sentiment-analysis.build]
command = "npm run build"
watch = ["src/**/*", "package.json", "package-lock.json"]

[[trigger.http]]
route = "/..."
component = "ui"

[component.ui]
source = { url = "https://github.com/fermyon/spin-fileserver/releases/download/v0.0.1/spin_static_fs.wasm", digest = "sha256:650376c33a0756b1a52cad7ca670f1126391b79050df0321407da9c741d32375" }
allowed_outbound_hosts = []
files = [{ source = "../sentiment-analysis-assets", destination = "/" }]

[variables]
kv_explorer_user = { required = true }
kv_explorer_password = { required = true }

[[trigger.http]]
component = "kv-explorer"
route = "/internal/kv-explorer/..."

[component.kv-explorer]
source = { url = "https://github.com/fermyon/spin-kv-explorer/releases/download/v0.10.0/spin-kv-explorer.wasm", digest = "sha256:65bc286f8315746d1beecd2430e178f539fa487ebf6520099daae09a35dbce1d" }
allowed_outbound_hosts = ["redis://*:*", "mysql://*:*", "postgres://*:*"]
# add or remove stores you want to explore here
key_value_stores = ["default"]

[component.kv-explorer.variables]
kv_credentials = "{{ kv_explorer_user }}:{{ kv_explorer_password }}"
```

Here’s what a modular version of this manifest could look like:

```toml
# [project]/spin.toml

spin_manifest_version = ?

[application]
name = "sentiment-analysis"
version = "0.1.0"
authors = ["Caleb Schoepp <caleb.schoepp@fermyon.com>"]
description = "A sentiment analysis API that demonstrates using LLM inference and

# Components defined in sub-manifests must be included in this list.
# Entries are treated as folder names, relative to the folder containing the application manifest.
components = ["sentiment-analysis", "kv-explorer"]

[[trigger.http]]
route = "/api/..."
component = "sentiment-analysis"

[[trigger.http]]
route = "/..."
component = "ui"

[[trigger.http]]
component = "kv-explorer"
route = "/internal/kv-explorer/..."

# The UI component is defined inline, because it's small.
[component.ui]
source = { url = "https://github.com/fermyon/spin-fileserver/releases/download/v0.0.1/spin_static_fs.wasm", digest = "sha256:650376c33a0756b1a52cad7ca670f1126391b79050df0321407da9c741d32375" }
allowed_outbound_hosts = []
files = [{ source = "../sentiment-analysis-assets", destination = "/" }]
```

```toml
# [project]/sentiment-analysis/spin.toml

# Sections and keys don't need to be scoped explicitly.
[component]
name = "sentiment-analysis"
description = "Component-specific description"
# Certain keys can be inherited from the application manifest
authors.application = true
version.application = true
source = "target/spin-http-js.wasm"
allowed_outbound_hosts = []
exclude_files = ["**/node_modules"]
key_value_stores = ["default"]
ai_models = ["llama2-chat"]

# Additional sections don't need to be namespaced under [component]
[build]
command = "npm run build"
watch = ["src/**/*", "package.json", "package-lock.json"]
```

```toml
# [project]/kv-explorer/Spin.toml

[component]
name = "kv-explorer"
source = { url = "https://github.com/fermyon/spin-kv-explorer/releases/download/v0.10.0/spin-kv-explorer.wasm", digest = "sha256:65bc286f8315746d1beecd2430e178f539fa487ebf6520099daae09a35dbce1d" }
allowed_outbound_hosts = ["redis://*:*", "mysql://*:*", "postgres://*:*"]
# add or remove stores you want to explore here
key_value_stores = ["default"]

# Locally defined variables don't need to be namespaced manually.
[variables]
user = { required = true }
password = { required = true }

# Variables exposed to content (this needs bikeshedding!)
[variables.exposed]
kv_credentials = "{{ user }}:{{ password }}"
```

As can be seen in this example, this modular structure allows us to not only isolate aspects of individual components in their respective manifests, but also reduce repetition, since keys and sections don’t have to be explicitly scoped to the component anymore.

## Details

### Sub-manifests for components

In this proposal, only components (and variables only accessible locally) can be defined in sub-manifests; triggers still need to be defined in the top-level manifest.

There are multiple reasons for this constraint:

1. Triggers are application-level concerns; they describe the application’s structure and high-level organization. As such, keeping them “at the surface” seems right.
2. Trigger definitions tend to be very short, so moving them into sub-manifests individually as is done for components seems very boilerplate-y.
3. Trigger definitions don’t come with their own assets, so any folder containing a sub-manifest for them would only ever contain the manifest, and nothing else.
4. If, on the other hand, we were to allow defining triggers together with components in the same sub-manifest, that’d make it very hard to reason about the application’s structure—see the first point above.
    1. It’s common to have multiple triggers using the same component, e.g. to expose it under multiple routes using multiple http triggers. That means we’d want to allow multiple trigger definitions per sub-manifest, making the previous point even stronger.

### One component per component manifest

Similar to other tools such as cargo or npm, under this proposal Spin would only support a single component to be defined in each sub-manifest. This allows reducing boilerplate in each manifest, because now sections such as `[build]` are unambiguously scoped and don’t need to be expanded to `[component.foo.build]`. It also allows grasping the application’s general structure just by looking at the application manifest, instead of having to look at each sub-manifest to find the full list of components.

*Note that this doesn’t preclude future additions for defining nested components visible only in the scope of an individual top-level component, and destined for composition into said top-level component.*

### Support for defining multiple components in the application manifest

This proposal would not take away support for defining components using the existing syntax; it’d purely add support for the modular approach. The reason is that there are component definitions that are very light-weight; moving them into separate folders would add boilerplate and cognitive overhead, not reduce it.

### Inheriting definitions from the application manifest

Similar to [crates in a Cargo workspace](https://doc.rust-lang.org/1.85.1/cargo/reference/workspaces.html#the-package-table), component manifests can inherit certain definitions from the application manifest, such as the version, description, and authors:

```toml
# [project]/Spin.toml

[application]
version = "0.1.0"
...

# [project]/my-component/Spin.toml
...
# Inherit the `version` field from the application
version.application = true
```

### Handling of variables

Variables are among the most tricky aspects to sort out in this approach. The proposal aims to make it easy to use both application-wide and component-scoped variables. An additional design goal is to support adoption without changes to deployment pipelines and their handling of variables.

To achieve these goals, the proposal

- treats application-level variables as inheritable in the same way as the definitions mentioned above
- namespaces component-level variables by prefixing them with the component name when interpreting manifests

To illustrate this, consider these manifests:

```toml
# [project]/Spin.toml

...
[variables]
my_variable = { default = "my-value" }
...

components = ["my-component"]
```

```toml
# [project]/my-component/Spin.toml

...
# Inherit the `my_variable` variable from the application
[variables]
my_variable.application = true
my_other_variable = { default = "some-value" }
...
```

Combined, they’d be interpreted like this:

```toml
...
[variables]
my_variable = { default = "my-value" }
my_component_my_other_variable = { default = "some-value" }
...
```

Any uses in template strings are adapted to use the prefixed variable names.

### Exposing variables to content

Currently, manifests have the `[variables]` table for defining variables available to use in the manifest itself, e.g. as part of template strings. They also have `[component.name.variables]` tables for exposing variables to content.

In component manifests, the role of the latter is taken by the new `[variables.exposed]` table.

*Note: we should evaluate whether this table is really needed. Can we instead just expose all variables in a component manifest’s `[variables]` table to content directly?*