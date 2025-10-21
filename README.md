<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./docs/static/image/logo-dark.png">
    <img alt="spin logo" src="./docs/static/image/logo.png" width="300" height="128">
  </picture>
  <p>Spin is a framework for building, deploying, and running fast, secure, and composable cloud microservices with WebAssembly.</p>
      <a href="https://github.com/spinframework/spin/actions/workflows/build.yml"><img src="https://github.com/spinframework/spin/actions/workflows/build.yml/badge.svg" alt="build status" /></a>
      <a href="https://cloud-native.slack.com/archives/C089NJ9G1V0"><img alt="Slack" src="https://img.shields.io/badge/slack-spin-green.svg?logo=slack"></a>
      <a href="https://www.bestpractices.dev/projects/10373"><img src="https://www.bestpractices.dev/projects/10373/badge"></a>
</div>

## What is Spin?

Spin is an open source framework for building and running fast, secure, and
composable cloud microservices with WebAssembly. It aims to be the easiest way
to get started with WebAssembly microservices, and takes advantage of the latest
developments in the
[WebAssembly component model](https://github.com/WebAssembly/component-model)
and [Wasmtime](https://wasmtime.dev/) runtime.

Spin offers a simple CLI that helps you create, distribute, and execute
applications, and in the next sections we will learn more about Spin
applications and how to get started.

## Getting started

See the [Install Spin](https://spinframework.dev/install) page of the [Spin documentation](https://spinframework.dev) for a detailed
guide on installing and configuring Spin, but in short run the following commands:
```bash
curl -fsSL https://spinframework.dev/downloads/install.sh | bash
sudo mv ./spin /usr/local/bin/spin
```

Alternatively, you could [build Spin from source](https://spinframework.dev/contributing-spin).

To get started writing apps, follow the [quickstart guide](https://spinframework.dev/quickstart/),
and then follow the
[Rust](https://spinframework.dev/rust-components/), [JavaScript](https://spinframework.dev/javascript-components), [Python](https://spinframework.dev/python-components), or [Go](https://spinframework.dev/go-components/)
language guides, and the [guide on writing Spin applications](https://spinframework.dev/writing-apps/).

## Language support

WebAssembly is a language-agnostic runtime: you can build WebAssembly components from a variety of source languages. Spin SDKs are available for several languages, including:

* JavaScript: https://github.com/spinframework/spin-js-sdk
* Rust: https://crates.io/crates/spin-sdk
* Go: https://pkg.go.dev/github.com/fermyon/spin/sdk/go/v2
* Python: https://github.com/spinframework/spin-python-sdk
* Zig: https://github.com/dasimmet/zig-spin (third party)
* Moonbit: https://github.com/gmlewis/spin-moonbit-sdk (third party)

> The Spin framework team supports the JavaScript, Rust, Go, and Python SDKs. Other language integrations are supported by their authors, and we're grateful to them for their work!

## Usage

Below is an example of using the `spin` CLI to create a new Spin application.  To run the example you will need to install the `wasm32-wasip1` target for Rust.

```bash
$ rustup target add wasm32-wasip1
```

First, run the `spin new` command to create a Spin application from a template.
```bash
# Create a new Spin application named 'hello-rust' based on the Rust http template, accepting all defaults
$ spin new --accept-defaults -t http-rust hello-rust
```
Running the `spin new` command created a `hello-rust` directory with all the necessary files for your application. Change to the `hello-rust` directory and build the application with `spin build`, then run it locally with `spin up`:

```bash
# Compile to Wasm by executing the `build` command.
$ spin build
Executing the build command for component hello-rust: cargo build --target wasm32-wasip1 --release
    Finished release [optimized] target(s) in 0.03s
Successfully ran the build command for the Spin components.

# Run the application locally.
$ spin up
Logging component stdio to ".spin/logs/"

Serving http://127.0.0.1:3000
Available Routes:
  hello-rust: http://127.0.0.1:3000 (wildcard)
```

That's it! Now that the application is running, use your browser or cURL in another shell to try it out:

```bash
# Send a request to the application.
$ curl -i 127.0.0.1:3000
HTTP/1.1 200 OK
content-type: text/plain
transfer-encoding: chunked
date: Sun, 02 Mar 2025 20:09:11 GMT

Hello World!
```

You can make the app do more by editting the `src/lib.rs` file in the `hello-rust` directory using your favorite editor or IDE. To learn more about writing Spin applications see [Writing Applications](https://spinframework.dev/writing-apps) in the Spin documentation.  To learn how to publish and distribute your application see the [Publishing and Distribution](https://spinframework.dev/distributing-apps) guide in the Spin documentation.

## Language Support for Spin Features

The table below summarizes the [feature support](https://spinframework.dev/language-support-overview) in each of the language SDKs.

| Feature | Rust SDK Supported? | TypeScript SDK Supported? | Python SDK Supported? | Tiny Go SDK Supported? | C# SDK Supported? |
|-----|-----|-----|-----|-----|-----|
| **Triggers** |
| [HTTP](https://spinframework.dev/http-trigger) | Supported | Supported | Supported | Supported | Supported |
| [Redis](https://spinframework.dev/redis-trigger) | Supported | Supported | Supported | Supported | Not Supported |
| **APIs** |
| [Outbound HTTP](https://spinframework.dev/rust-components.md#sending-outbound-http-requests) | Supported | Supported | Supported | Supported | Supported |
| [Configuration Variables](https://spinframework.dev/variables) | Supported | Supported | Supported | Supported | Supported |
| [Key Value Storage](https://spinframework.dev/kv-store-api-guide) | Supported | Supported | Supported | Supported | Not Supported |
| [SQLite Storage](https://spinframework.dev/sqlite-api-guide) | Supported | Supported | Supported | Supported | Not Supported |
| [MySQL](https://spinframework.dev/rdbms-storage#using-mysql-and-postgresql-from-applications) | Supported | Supported | Not Supported | Supported | Not Supported |
| [PostgreSQL](https://spinframework.dev/rdbms-storage#using-mysql-and-postgresql-from-applications) | Supported | Supported | Not Supported | Supported | Supported |
| [Outbound Redis](https://spinframework.dev/rust-components.md#storing-data-in-redis-from-rust-components) | Supported | Supported | Supported | Supported | Supported |
| [Serverless AI](https://spinframework.dev/serverless-ai-api-guide) | Supported | Supported | Supported | Supported | Not Supported |
| **Extensibility** |
| [Authoring Custom Triggers](https://spinframework.dev/extending-and-embedding) | Supported | Not Supported | Not Supported | Not Supported | Not Supported |

## Getting Involved and Contributing

We are delighted that you are interested in making Spin better! Thank you!

Each Monday at 2:30pm UTC (odd weeks) and 9:00pm UTC (even weeks), we meet to discuss Spin issues, roadmap, and ideas in our Spin Project Meetings. Link to the meeting can be found in the Spin Project Meeting agenda below.

The [Spin Project Meeting agenda](https://docs.google.com/document/d/1EG392gb8Eg-1ZEPDy18pgFZvMMrdAEybpCSufFXoe00/edit?usp=sharing) is a public document. The document contains a rolling agenda with the date and time of each meeting, the Zoom link, and topics of discussion for the day. You will also find the meeting minutes for each meeting and the link to the recording. If you have something you would like to demo or discuss at the project meeting, we encourage you to add it to the agenda.

You can find the contributing guide [here](https://spinframework.dev/contributing-spin).

## Stay in Touch

Follow us on Twitter: [@spinframework](https://twitter.com/spinframework)

You can join the Spin community in the [Spin CNCF Slack channel](https://cloud-native.slack.com/archives/C089NJ9G1V0) where you can ask questions, get help, and show off the cool things you are doing with Spin!

