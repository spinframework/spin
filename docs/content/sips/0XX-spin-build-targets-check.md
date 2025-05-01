title = "SIP 0XX - Spin Build Targets Check"
template = "main"
date = "2024-06-13T12:00:00Z"
---
Summary: Extend `spin build` to validate component targets against trigger types.

Owner(s): 
    [brian.hardock@fermyon.com](mailto:brian.hardock@fermyon.com)
    [michelle@fermyon.com](mailto:michelle@fermyon.com)

Created: June 13, 2024

# Background
When associating a Spin component to a specific trigger in the Spin manifest, there is an implied world that the Spin component is targeting (e.g. the http trigger type implies `fermyon:spin/http-trigger`). However, we don't know if the Spin component actually implements that world until instantiation time (i.e. when a request / event comes in).

As a visual example, if a user mistankenly associates an http trigger with a component that targeted the redis world:

`examples/hello-world/spin.toml:`
```examples/hello-world/spin.toml
[[trigger.http]]
route = "/hello"
component = "hello"

[component.hello]
source = "redis-component.wasm" # this is a redis component which is wrong
```

When issuing a curl to this endpoint:
```
❯ curl -i http://127.0.0.1:3000/hello
HTTP/1.1 500 Internal Server Error
content-length: 0
date: Tue, 04 Jun 2024 21:53:41 GMT
```

And in the output of `spin up` we see:
```
❯ spin up -f examples/hello-world 
Logging component stdio to "examples/hello-world/.spin/logs/"

Serving http://127.0.0.1:3000
Available Routes:
  hello: http://127.0.0.1:3000/hello
    A simple component that returns hello.
2024-06-04T21:53:16.160773Z ERROR spin_trigger_http: Error processing request: Expected component to either export `wasi:http/incoming-handler@0.2.0-rc-2023-10-18` or `fermyon:spin/inbound-http` but it exported neither    
```

# Proposal

Validate that each Spin component structurally conforms to (or targets) the expected world implied by the associated trigger type at build time rather than runtime.

Prior art for performing a structural target of components against a world is [`wasm-tools component targets ...`](https://github.com/bytecodealliance/wasm-tools/blob/9340ed2466a50b4dbc580b13ba18a417dee91433/src/bin/wasm-tools/component.rs#L683) subcommand. We could potentially use [wac target predicate implemtation](https://github.com/bytecodealliance/wac/blob/4c96def294e6e779c52cfc5a93e05ed4c73ee60f/crates/wac-parser/src/resolution.rs#L2708) for a cleaner more actionable user diagnostic. Effectively, spin build could invoke the targets check using the trigger implied world and the source component.

An "approximate" visual for the spin build experience we could enable:

```
❯ spin build -f examples/hello-world 
Component "hello" is not a valid http component.

Error: 
    Expected component to either export `wasi:http/incoming-handler@0.2.0-rc-2023-10-18` or `fermyon:spin/inbound-http` but it exported neither    
```

# Open question: How does this work with custom triggers?
One possible solution is to allow trigger extensions the abilitiy to hook into build time validation via the `TriggerExecutor` trait. For each `spin build`, the `spin-cli` could construct a `build` command to invoke each trigger with allowing the custom trigger an opportunity to enforce validation on the target of each component before `up`.
