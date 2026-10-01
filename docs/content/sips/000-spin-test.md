title = "SIP 000 - `spin-test`"
template = "main"
date = "2024-04-18"
---

Summary: create a component based testing framework for testing Spin applications within a WebAssembly sandbox.

Owner(s): ryan.levick@fermyon.com

Created: April 18, 2024

This SIP describes the `spin-test` tool that allows running unmodified Spin applications in WebAssembly component based testing environment where all potential imports to the Spin app are virtualized *and* can be configured by the user in a language independent way.

Implementation of this SIP has already begun [here](https://github.com/fermyon/spin-test).

## Requirements

The requirements for such a tool are the following:

- Does not require the user to modify their application in any way - what runs in the test is what runs in production.
- The app and all the imports to the app run in WebAssembly and not on the host.
- The test itself also runs in WebAssembly allowing tests to be written in any language with WebAssembly component guest support.
- Allows the user to modify the testing environment in a programmatic way in any language they want to use.
- Have the test environment match functionality as much as is possible with other Spin runtimes such as found in Spin CLI, Fermyon Cloud, and SpinKube.

## Proposal

To achieve the goals above, we propose a new tool tentatively called `spin-test`.

The `spin-test` binary would be callable from any Spin project in much the same way that `spin up` is project aware (i.e., it doesn’t need to be told where the `spin.toml` manifest is). 

Invoking `spin-test` will automatically resolve where the Spin app component lives. The tool will automatically find tests that should be run and will run those tests against the Spin app component reporting whether the tests passed or failed. `spin-test` will come with basic ways of filtering tests so that a subset of tests can be run.

**Note:** See the *Open Questions* section for discussion on how `spin-test` discovers tests to run.

### Implementation

At the core of `spin-test` is component composition. The user’s Spin app is composed together with other components to ultimately produce a component (referred to as “the composition” below) that has none of the original Spin or WASI imports. The composition has the following components internal to it:

- A single component from the Spin app (see the *Open Questions* section for why only one component from a Spin app can be supported)
- A Spin virtualization component that virtualizes all `fermyon:spin/platform` interfaces.
- A WASI virtualization component that virtualizes all the `wasi:cli/imports` interfaces.
- A router component that decides from a `spin.toml` manifest file whether an `incoming-request` should be routed to the Spin component or not.
- A test driver component that can configure the Spin and WASI virtualization components, make one or more requests to the router, and then make assertions on the response returned and the state of the Spin and WASI virtualizations.

The composition only has the following imports: 

- A way to receive a `spin.toml` manifest file. This is needed by the router, Spin virtualization, and WASI virtualization components to know how they should be configured.
- A few imports for working around limitations of guest support for `wasi:http`:
    * An `http-helper` interface which allows creation of `wasi:http/types@0.2.0.{incoming-request, response-outparam, incoming-response}` resources since these can only be created by a host.
    * Another function that can turn `wasi:http@types@0.2.0.{outgoing-response}` into a `wasi:http@types@0.2.0.{future-incoming-response}`

*Note*: 

> **A few notes:** 
* We may still wish to have the composition import `wasi:cli/stdout` and `wasi:cli/stderr` so that it can easily log things.
* It might be possible to statically configure the virtualized Spin runtime component given a manifest file by synthesizing the component on the file (in much the way that WASI-virt works). This would make it possible to no longer require that the manifest is passed to the composition component at runtime since it has already been statically configured to run correctly with that Spin manifest. This has its downsides though as composition components for the same app but different manifests cannot be shared between tests.
* The `wasi:http` work around imports may hopefully eventually overcome with changes to the `wasi:http` package. 
* If this and the manifest import are eliminated, tests could potentially be run by an component runtime with no imports whatsoever.
> 

## Wit definitions

**NOTE**: These wit definitions are still very much under development! Bike-shedding welcome!

### The `fermyon:spin-test/test` world

This is the world that all `spin-test` compliant test components target.

```
package fermyon:spin-test;

world test {
	/// The following allow the test to both configure and observe 
	/// the environment the Spin app is running in.
		
	/// Gives the test the ability to read and write to the kv store
	/// independently of the Spin app.
    import fermyon:spin/key-value@2.0.0;
    /// A handle into the configuration of the `wasi:http/outbound-handler`
    /// implementation.
    import fermyon:spin-test-virt/http-handler;
    /// A handle into an interface that tracks calls to the key-value store
    import fermyon:spin-test-virt/key-value-calls;
    /// TODO: more handles to configure and observe the other Spin
    /// and WASI interfaces.
    
    /// The following allow the test to make requests against the Spin app
    /// and view its response.
    import http-helper;
    /// The ability to call the Spin app
    import wasi:http/incoming-handler@0.2.0;

	/// Actually call the test
    export run: func();
}

interface http-helper {
    use wasi:http/types@0.2.0.{incoming-request, response-outparam, incoming-response, outgoing-request};
    resource response-receiver {
        get: func() -> option<incoming-response>;
    }
    new-request: func(request: outgoing-request) -> incoming-request;
    new-response: func() -> tuple<response-outparam, response-receiver>;
}
```

### The `fermyoin:spin-test-virt/plug` world

This is the world of the component that virtualizes a Spin runtime. 

```
package fermyon:spin-test-virt;

/// The exports that can be composed with a Spin app creating
/// a virtualized component.
world plug {
	/// All of the `fermyon:spin/platform` world's interfaces are
	/// exported here.
    export fermyon:spin/key-value@2.0.0;
    export fermyon:spin/llm@2.0.0;
    export fermyon:spin/redis@2.0.0;
    export fermyon:spin/postgres@2.0.0;
    export fermyon:spin/mqtt@2.0.0;
    export fermyon:spin/mysql@2.0.0;
    export fermyon:spin/sqlite@2.0.0;
    export fermyon:spin/variables@2.0.0;
    export wasi:http/outgoing-handler@0.2.0;
    
    /// The virtualization needs the component's id and
    /// the `spin.toml` manifest because many of the spin
    /// interfaces are configured through that combination.
    export set-component-id: func(component-id: string);
    import get-manifest: func() -> string;

	/// A way to say that a certain URL is associated with a response
    export http-handler;
    /// How the calls to the kv store are tracked
    export key-value-calls;
    
}

interface http-handler {
    use wasi:http/types@0.2.0.{future-incoming-response};
    set-response: func(url: string, response: future-incoming-response);
}

interface key-value-calls {
    reset: func();
    get: func() -> list<tuple<string, list<get-call>>>;
    set: func() -> list<tuple<string, list<set-call>>>;

    record get-call {
        key: string
    }

    record set-call {
        key: string,
        value: list<u8>
    }
}
```

### The `fermyoin:spin-test/runner` world

```
world runner {
	/// The host must supply a manifest
    import get-manifest: func() -> string;
    /// The host must give the runner the ability to create HTTP resources
    import http-helper;
    /// The host can then run the test
    export run: func();
}
```

## Open Questions

- Should `spin-test` be a stand alone CLI experience, a plugin for `spin` CLI, or both?
- How do we handle Spin applications with more than component?
    - There’s no way to compose a statically defined component with N number of components. This means the router component must know how many components it will be composed with ahead of time, and so the reasonable number of 1 is picked.
    - We could work around this by moving the router out of a component and into the host, but this makes the host smarter which would like to avoid. We could also potentially synthesize a router component based on the `spin.toml` manifest instead of relying on a statically defined one, but this is certainly not trivial.
- How should test discovery work?
    - The most straightforward way would be for `spin-test` to require the user to specify the path to a `spin-test` compliant test component binary.
    - Another way would be for there to be a convention around a `tests` directory. This `tests` directory could include both built `spin-test` compliant test component binaries as well as directories with source code and a `spin-test.toml` configuration file that specifies how the source code is built into a `spin-test` compliant test component binary. `spin-test` would then build all the tests that need to be built and run each test binary.
    - We could also carve away space in the `spin.toml` manifest (e.g., by using the `[component.<id>.tool]` section). Such a solution might be more appropriate if we determine that users will normally want very few test component binaries. If, however, it proves to be useful to potentially have many test binaries, splitting the configuration out to each test binary in the form of a bespoke `spin-test.toml` per test binary might be the better solution.
- How exactly should the various worlds look like?
    - The proposal above is enough to get things working, but there is plenty of room for bike-shedding the exact shape of the worlds.
- Which triggers should we support?
    - Can we support an arbitrary number of triggers through some sort of plugin system? How would that work?
