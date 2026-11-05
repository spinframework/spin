title = "SIP 026 - Spin Agent Sandbox"
template = "main"
date = "2026-10-26T00:00:00Z"
---

Summary: Propose a new Spin Framework project, Spin Agent Sandbox, that provides a
maintained, reusable component that enables running untrusted JavaScript/TypeScript and Python code inside isolated WebAssembly components on Spin.

Owner(s): Karthik Ganeshram ([@karthik2804](https://github.com/karthik2804))

Created: Oct 30, 2026

## Background

Running untrusted code is becoming a common requirement, most visibly for AI
agents that write and run small programs as part of their work. Today, everyone
who needs this has to build and maintain their own sandbox, each with its own
egress policy, storage isolation, and error handling.

Spin already runs code in isolated WebAssembly components. Spin Agent Sandbox builds
on that to provide a maintained sandbox component that Spin users can adopt
instead of writing their own.

## Proposal

We propose creating Spin Agent Sandbox as a new supporting project in the Spin
Framework organization, hosted at `github.com/spinframework/spin-sandbox` (name
open for discussion). The project would be seeded from a working prototype we at Akamai have built and can be found at https://github.com/fermyon/spin-agent-sandbox.

Spin Agent Sandbox is a way to run generated Python and TypeScript/JavaScript code on Spin with additional limitations and security guarantees. A
caller sends source code over HTTP and gets back the result: captured
stdout/stderr for a script, or the HTTP response for a handler. Each language
sandbox includes a curated set of commonly used libraries (for example `numpy`,
`pydantic`, and `jinja2` for Python; `zod`, `cheerio`, and `lodash` for
TypeScript), so typical code runs without an install step.

```bash
curl -s localhost:3000/py/run -d '{"code":"print(6 * 7)","sandboxId":"demo"}'
# {"stdout":"42\n","stderr":"","error":null}
```

Karthik Ganeshram ([@karthik2804](https://github.com/karthik2804)) and Radu
Matei ([@radu-matei](https://github.com/radu-matei)) will be the initial
maintainers, as listed in the repository's `MAINTAINERS.md`. The project will
follow the [Spin Framework governance](https://github.com/spinframework/governance/blob/main/GOVERNANCE.md)
and be licensed under Apache-2.0 with LLVM exception, consistent with other
Spin Framework projects.

## Architecture

The architecture below reflects our prototype. Each sandbox is a single Spin
component built by composing three components with `wac`:

```text
 caller (agent / Spin app, via service chaining or the SDK)
    |
    |  POST { code, sandboxId, allowedHosts, kvEnabled, input }
    v
+-----------+     +------------------------+     +-----------+
|   guard   | --> |  language guest        | --> | guard-io  | --> Spin host
| (HTTP in) |     |  (ts-sandbox / py-     |     | (policy)  |     (KV, outbound
+-----------+     |   sandbox: runs code)  |     +-----------+      HTTP)
                 +------------------------+
```

The released binary is pre-composed; consumers deploy a single component with no need to 
manually compose. The composition uses three components internally to avoid cyclic 
dependencies:

- **`guard`** is the HTTP entry point. It validates and size-limits the request, sets the per-request context (sandbox ID, allowed hosts, KV on/off), calls `guard-io` to establish the context, then executes the guest. It returns the result as JSON. The context is set once and never reset, so the component relies on Spin running each request in a fresh instance (see [Security model](#security-model)). The components currently target wasip2 for which Spin enforces a new instacnce per call.
- **The language guest** (ts-sandbox or py-sandbox) runs the user's JavaScript/TypeScript or Python code, either as a script (with captured stdout/stderr) or as an HTTP handler.
- **`guard-io`** sits between the guest and the host and enforces per-request policy on every host call: KV keys are namespaced by the sandbox ID, outbound HTTP is denied unless explicitly allowed in per-request config, stdio is captured and capped, and every other capability the guests can reach is denied or stubbed (see [Security model](#security-model)). The language guest component cannot override this policy.

The sandbox ID is a namespace for KV storage and enables multi-tenant isolation within a single Spin application. The application, not the model or the end user, sets the ID, and it is the application's responsibility to supply the correct one; the sandbox does not authenticate it (see [Trust model](#trust-model)). Allowed hosts are likewise set per-request by the application
and operate as additional constraints on top of the `spin.toml` outbound host allowlist
(deny-by-default narrowing, not expansion). Entries use the same syntax as `allowed_outbound_hosts` in
`spin.toml` (optional `scheme://`, `host[:port]`, `*.` subdomain wildcards, `:*` for any port; no port means the scheme's default port), with three differences: a bare `*` grants nothing, paths are ignored, and loopback, link-local, private-IP, and `.internal` targets are always refused.

Alongside the sandboxes, the project includes a TypeScript client SDK, Spin templates for adding sandboxes to an app, an experimental npm bundler for TypeScript dependencies, and examples.

## Trust model

The sandbox ID provides multi-tenant isolation on the premise that the application sets it. The model cannot set the sandbox ID or the allow-list, so a prompt-injected agent cannot select another tenant's namespace or widen its egress. The endpoint itself does not authenticate callers: anyone who can reach it can supply any `sandboxId`, `allowedHosts`, and `kvEnabled`, and so can read and write the KV namespace of any sandbox ID. The trust boundary is therefore the route and the calling application, not the request body:

- The templates default to a private route (`route = { private = true }`), reachable only through service chaining over `*.spin.internal`. Exposing a sandbox publicly requires the user to add authentication in front of it on purpose.
- The calling application owns the sandbox ID and the allow-list. It is responsible for mapping each request to a sandbox ID and for verifying that the correct ID is used. Sandbox IDs are not secrets and must not be accepted from untrusted input, including model output.
- The client SDK's model-facing tool does not let the model set the sandbox ID (`identity` in the SDK), `allowedHosts`, or `kvEnabled`; the developer sets these in execution options, and the model never sees them. The application decides which sandbox ID each request uses. This is an SDK-level control, not enforcement by the endpoint, so callers that bypass the SDK must apply the same discipline.

## Security model

The guest can reach only the host capabilities below. `tests/check-imports.mjs` in the prototype fails the build if a guest, or the composed component, imports any host interface that is not on an allowlist, and asserts that guests never import the context interface that sets the sandbox ID, allowed hosts, and KV flag.

| Capability | Behavior |
| --- | --- |
| `fermyon:spin/key-value` | Allowed only when `kvEnabled`; keys are prefixed with a length-prefixed sandbox ID (`{len}:{id}:{key}`) so IDs cannot collide as prefixes, and `get-keys` returns only the caller's keys. |
| `wasi:keyvalue` (store, atomics, batch) | Denied: `open` returns `access-denied`. |
| `spin:key-value` | Not provided by `guard-io`; the guests do not import it, and the import check fails the build if one starts to. |
| `wasi:http/outgoing-handler` | Denied unless the request's `allowedHosts` permits the destination, in addition to the `spin.toml` allowlist. |
| `wasi:cli` stdout, stderr, stdin | Output is captured and capped at 256 KB each; stdin is empty. |
| `wasi:sockets` | Stubbed; every call returns a denied error. |
| `wasi:filesystem` | Stubbed; no preopens. |
| `spin:variables`, `wasi:config` | Denied for guest code; only `guard` reads its own log-level variable. |
| Outbound Redis, PostgreSQL, MySQL, MQTT, SQLite, LLM | Denied: `open`/`infer` return an error, so user code sees an ordinary exception. |
| `wasi:cli/environment` | Not wrapped, so user code can read every environment variable configured on the component. The sandbox template manifests set none. Users may add some |
| Clocks, random | Passed through to the host. |

### Fresh instance per request

The per-request context lives in component state and is not cleared after the request. This is safe only if every request runs in a fresh instance. Spin runs WASIp2 components this way by default (the `max_instance_reuse_count` and `max_instance_concurrent_reuse_count` options default to 1 and have no effect on WASIp2 components). Instance reuse, which Spin enables by default for WASIp3 components, would let one request's context, interpreter globals, and state carry over to another. Once the sandbox moves to WASIp3, its components must be configured to run without instance reuse.

### Resource bounds

- Request body is limited to 8 MB, `code` to 5 MB, and `sandboxId` to 256 bytes.
- Captured stdout and stderr are each capped at 256 KB, with a truncation marker.
- CPU time, wall-clock time, and memory are not limited inside the component. A host or platform deployment must enforce them.
- Per-sandbox KV quotas (key count, value size, total size) are not enforced. One sandbox ID can fill the shared store, so deployments that need to bound this should apply quotas in the store or host, or use a dedicated store.

## Why the Spin Framework organization

- Users get one maintained, openly governed sandbox instead of each building
 their own.
- It is built entirely on Spin and runs anywhere Spin runs, with no host
 changes.
- Like `spin-fileserver`, it is a reusable component consumed as a prebuilt
 artifact.

## Known limitations

- No built-in CPU, time, or memory limits; these must be enforced by the host or platform.
- No per-sandbox KV quotas; one sandbox ID can exhaust a shared store.
- Response bodies from HTTP handlers are streamed back without a size cap; only request size and captured stdout/stderr are bounded.
- No endpoint-level authentication of callers or sandbox IDs; callers must be private or authenticated (see [Trust model](#trust-model)).
- No current built-in way to keep secrets or variables outside of the agent code’s purview.
