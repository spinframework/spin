#!/bin/bash
# This script builds and publishes the spin:up WIT package
# NOTE: The package name and version are inferred from the
# encoded wasm binary.
set -euo pipefail 

# Build the package
wasm-tools component wit wit/ -w -o spin_up_wit.wasm

# Publish to registry
wkg publish --registry spinframework.dev spin_up_wit.wasm