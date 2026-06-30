# Prerequisites

You will need:

* The WASI SDK
  * Installation: https://github.com/WebAssembly/wasi-sdk#install
  * Releases: https://github.com/WebAssembly/wasi-sdk/releases

# Building

* Set the `WASI_SDK_PATH` environment variable to the root of your WASI SDK installation (per the installation instructions at https://github.com/WebAssembly/wasi-sdk#install)
* Run `spin build`

# WASI and Spin host bindings

The `bindings` directory contains generated bindings for Spin. If you need to regenerate bindings:

* Install `wit-bindgen` (https://github.com/bytecodealliance/wit-bindgen#cli-installation) - requires Rust (https://rust-lang.org/tools/install/)
* Copy the `wit` directory from https://github.com/spinframework/spin into this folder
* Delete the existing `bindings` directory 
* Run `wit-bindgen c ./wit/ --out-dir bindings -w http-trigger`

However, C bindings for WASI P3 and other async APIs are not yet mature, so if you do need to regenerate bindings, it is currently recommended that you **NOT** do so from Spin `HEAD`.
