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

- **`guard`** is the HTTP entry point. It validates and size-limits the
 request, sets the per-request context (sandbox ID, allowed hosts, KV on/off),
 runs the guest, and returns the result as JSON.
- **The language guest** runs the user's JavaScript/TypeScript or Python code,
 either as a script (captured stdout/stderr) or as an HTTP handler.
- **`guard-io`** sits between the guest and the host and applies the
 per-request policy to every host call: KV keys are scoped to the sandbox ID,
 outbound HTTP is denied unless allow-listed, and stdio is captured. The language guest component cannot change this policy.

Alongside the sandboxes, the project includes a TypeScript client SDK, Spin templates for adding sandboxes to an app, an experimental npm bundler for TypeScript dependencies, and examples.

## Why the Spin Framework organization

- Users get one maintained, openly governed sandbox instead of each building
 their own.
- It is built entirely on Spin and runs anywhere Spin runs, with no host
 changes.
- Like `spin-fileserver`, it is a reusable component consumed as a prebuilt
 artifact.

## Known limitations


- No built-in CPU, time, or memory limits; these must be enforced by the host
 or platform.
- No current built-in way to keep secrets or variables outside of the agent code’s purview
025-validate-target-environment copy
