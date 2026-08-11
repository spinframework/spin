title = "SIP 022 - Target Environment Validation"
template = "main"
date = "2025-08-07T00:00:00Z"
---

Summary: Provide a mechanism to validate that a Spin application is compatible with its intended deployment environment(s).

Owner(s): till@tillschneidereit.net

Created: Aug 7, 2025

## Background

When developing a Spin application, developers might use features that are only available in a specific Spin environment. For example, an application might use a custom trigger or a host capability that is only available in a specific deployment environment, or in a self-hosted Spin instance with specific plugins.

Currently, there is no way to declare the intended deployment environment for an application and have Spin validate that the application is compatible with it. This can lead to situations where an application builds successfully but fails at runtime in the target environment due to missing dependencies, unsupported features, or other incompatibilities.

This SIP proposes a new mechanism to address this problem by allowing developers to specify the target environments for their applications and have Spin validate the compatibility of the application with those environments at build time.

## Proposal

The proposal is to introduce a new `targets` field in the `[application]` section of the `spin.toml` manifest file. This field will contain a list of identifiers for the target environments. Spin will use these identifiers to fetch environment definition files and validate the application against them.

### The `targets` field in `spin.toml`

The `targets` field is a list of strings or tables that identify the target environments. It can be specified in the `[application]` section of `spin.toml` like this:

```toml
[application]
name = "my-app"
version = "1.0.0"
targets = ["spin-up:3.3", "spinkube:0.4", { registry = "ghcr.io/my-org/environments", id = "my-env:1.0" }, { path = "local-env.toml" }]
```

Each entry in the list can be:
- A string that identifies an environment in the default registry (e.g., `"spin-up:3.3"`).
- A table with `registry` and `id` keys to identify an environment in a custom OCI registry.
- A table with a `path` key to specify a local environment definition file.

### Environment Definition Files

An environment definition is a TOML file that describes the capabilities of a Spin environment. It specifies the triggers that are available in the environment, and for each trigger, the WIT worlds that are compatible with it and the host capabilities that it supports.

Here is an example of an environment definition file:

```toml
# spin-up.3.2.toml
[triggers.http]
worlds = ["spin:up/http-trigger@3.2.0", "spin:up/http-trigger-rc20231018@3.2.0"]
capabilities = ["local_service_chaining"]

[triggers.redis]
worlds = ["spin:up/redis-trigger@3.2.0"]
```

This file defines an environment that supports the `http` and `redis` triggers. The `http` trigger is compatible with two different WIT worlds and supports the `local_service_chaining` capability. The `redis` trigger is compatible with one WIT world and has no specific capabilities.

### Validation at Build Time

When the user runs `spin build`, Spin will perform the following steps if the `targets` field is present in `spin.toml`:

1.  For each target environment specified in the `targets` field, Spin will fetch the corresponding environment definition file.
2.  For each component in the application, Spin will check if its trigger is supported in the target environment.
3.  If the trigger is supported, Spin will check if the component's WIT world is compatible with one of the worlds defined for that trigger in the environment definition.
4.  Spin will also check if all the capabilities required by the component are supported by the environment.

If any of these checks fail for any of the target environments, the build will fail with an error message indicating the incompatibility.

The user can bypass the validation by using the `--skip-target-checks` flag with the `spin build` command.

## Future work

-   **Per-component target environments:** We could allow overriding the `targets` field on a per-component basis. This would be useful for applications that end up being deployed into multiple environments in a modular fashion.
-   **Tooling for environment authors:** We could provide tools to help authors create and publish environment definition files.
-   **Using environment definitions for bindings generation:** With this SIP, we propose to use target worlds in a validation predicate. Building on this, we could also use them to better support authors in targeting the right environment to begin with.
