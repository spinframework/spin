title = "SIP xxx - Spin SDK CloudEvents Support"
template = "main"
date = "2022-04-25T14:53:30Z"
---

Summary: Spin SDK support CloudEvents as a alternative message schema.

Owner: jiazho@microsoft.com

Created: April 24, 2022

Updated: May 10, 2022

## Background

Currently Spin supports two triggers, one for Redis messages and one for HTTP requests. [CloudEvents](https://cloudevents.io/) is a new specification for eventing and received huge interests from the major cloud providers. Supporting CloudEvents could make Spin a great solution for writing serverless applications. 


## Proposal

This document proposes adding features in the Spin SDK to support CloudEvents. CloudEvents itself is an envelope for the underlying transport protocol, such as AMQP, Kafka, HTTP, etc. This proposal aims at providing a CloudEvents component for the [HTTP Protocol Bindings](https://github.com/cloudevents/spec/blob/main/cloudevents/bindings/http-protocol-binding.md). Here is an example shows the mapping of an event with an HTTP POST request in CloudEvents's binary format.
```
POST /someresource HTTP/1.1
Host: webhook.example.com
ce-specversion: 1.0
ce-type: com.example.someevent
ce-time: 2018-04-05T03:56:24Z
ce-id: 1234-1234-1234
ce-source: /mycontext/subcontext
    .... further attributes ...
Content-Type: application/json; charset=utf-8
Content-Length: nnnn
{
    ... application data ...
}
```

Creating a HTTP CloudEvents trigger is done by defining the top level application trigger in Spin. The following code snippet shows the definition of a HTTP CloudEvents trigger.
```toml
# spin.toml
trigger = { type = "http", base = "/", schema = "cloudevents" }
```

The added `schema` attribute in trigger will tell Spin application that it will expect the HTTP request and responses are CloudEvents.

> Note that the `schema` attribute is not the same as the `schema` attribute in the CloudEvents spec. The `schema` attribute in the spec is the schema of the payload.

We also allow users to define individual component to be CloudEvents components. For example, we could define a HTTP CloudEvents component in spin.toml:

```toml
# spin.toml
trigger = { type = "http", base = "/" }

[[component]]
id = "hello"
source = "target/wasm32-wasi/release/spinhelloworld.wasm"
description = "A simple component that returns hello."
[component.trigger]
route = "/hello"
schema = "cloudevents"
```

> Note that if there is no `schema` attribute in the application or individual component, the HTTP request and responses are not CloudEvents.

## The WebAssembly interface

The CloudEvents HTTP trigger is built on top of the
[WebAssembly component model](https://github.com/WebAssembly/component-model).
The current interface is defined using the
[WebAssembly Interface (WIT)](https://github.com/bytecodealliance/wit-bindgen/blob/main/WIT.md)
format, and is a function that takes the event payload as its only parameter:

```fsharp
// wit/ephemeral/Spin-ce.wit

// The event payload.
record event {
    // The event type.
    type: string,
    // The event id.
    id: string,
    // The event source.
    source: string,
    // The event specversion.
    specversion: string,
    // The event data content type.
    datacontenttype: string,
    // The event data schema.
    dataschema: string,
    // The event subject.
    subject: string,
    // The event time.
    time: option<string>,
    // The event data.
    data: option<list<u8>>
}

// The entry point for a CloudEvents handler.
handle-cloudevent: function(event: event) -> expected<event, error>
```


This is the function that all CloudEvents components must implement, and which is
used by the Spin HTTP executor when instantiating and invoking the component.

Notice that the function will return a `expected<event, error>` value.

## The CloudEvents Spin SDK
```rust
// A Spin CloudEvents component written in Rust
use anyhow::Result;
use spin_sdk::{
    event::{Event},
    event_component,
};

/// A simple Spin event component.
#[cloud_event_component]
fn trigger(event: Event) -> Result<Event, _> {
    println!("event is {}", event.id());
    Ok(event)
}
```

```go
// A Spin CloudEvents component written in Go
package main

import (
 "fmt"
 "context"

 Spin "github.com/fermyon/Spin/sdk/go/event"
)

func init() {
	spincloudevents.Handle(func(in Spin.Event) (out Spin.Event, err error) {
		fmt.Printf("%s", event)
        return in, nil
	})
}

func main() {}
```

## Implementation
### Funtional Requirements
1. Enable Rust SDK to serialize/deserialize CloudEvents using CloudEvnets SDK.
2. Enable Go SDK to serialize/deserialize CloudEvents using CloudEvnets SDK.
3. Add a new CloudEvents component macro to Rust SDK.
4. Add CloudEvents webhook to both Rust and Go SDK.
5. Fully test the new CloudEvents component.
6. Write examples for new CloudEvents component.

### Non-Functional Requirements
1. Scale Spin to support thousands of CloudEvents requests per second.
2. Establish reasonable performance metrics for the new CloudEvents component.

## Future design considerations

#### More transport protocols bindings
- Kafka binding
- AMQP binding
- NATS binding

#### Filtering based on event attributes
```toml
[[component]]
id = "filter"
source = "target/wasm32-wasi/release/spinhelloworld.wasm"
description = "A simple component that filters events based on event attributes."
schema = "cloudevents"
[component.trigger]
route = "/filter"
[component.filter]
ce.type = "com.example.someevent"
ce.source = "/mycontext/subcontext"
```

#### Generic CloudEvents component
A generic CloudEvents component is defined in the following way:
```rust
// A Spin CloudEvents component written in Rust
use anyhow::Result;
use spin_sdk::{
    event::{Event},
    cloud_event_component,
};

/// A simple Spin event component.
#[cloud_event_component]
fn trigger(event: Event) -> Result<Event, _> {
    println!("event is {}", event.id());
    // do something with the event
    Ok(event)
}
```

It is trigger-agnostic, at least within the supported CloudEvents protocol bindings. You can see the list of supported protocols in the [CloudEvents documentation](https://github.com/cloudevents/spec/blob/main/cloudevents/bindings). The benefits of doing this are:
1. Rapid prototyping: you can quickly prototype your event components and test them locally using HTTP bindings. Once you are confident that they are working, You can switch the trigger to a different type, without having to modify the code.
2. Reusability: you can reuse the same event components with different protocols bindings, such as AMQP and Kafka.
3. Chaining: you can chain multiple event components together since they share the same component signature.

#### CloudEvents trigger

Creating an CloudEvents trigger is done when [configuring the application](/configuration)
by defining the top-level application trigger:

```toml
# spin.toml
trigger = { type = "cloudevent" }
```

Then, when defining the component (in `spin.toml`), you can set the protocol binding for the component. For example:

- an HTTP CloudEvents component:

```toml
[component.trigger]
binding = "http"
```

- an Kafka CloudEvents component (optional):

```toml
[component.trigger]
binding = "kafka"
broker = ["localhost:9092", "localhost:9093"]
topic = "mytopic"
group = "mygroup"
```

- an AMQP CloudEvents component (optional WIP):

```toml
[component.trigger]
binding = "amqp"
broker = "localhost:5672"
exchange = "myexchange"
routing_key = "myroutingkey"
```

You can also set the sink address for the component that returns a CloudEvents. For example:

```toml
[component.trigger]
binding = "http"
sink = "http://localhost:8080/someresource"
```

Note that the sink address is only used when the component is invoked. The component will make a outbound HTTP request that includes the CloudEvents to the sink address.