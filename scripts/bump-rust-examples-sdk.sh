#!/bin/bash
# This script updates the `spin-sdk` dependency in Rust examples
# `Cargo.toml` files under the `examples` directory.
set -euo pipefail

VERSION=$1

# -i syntax differs between GNU and Mac sed; this usage is supported by both
SED_INPLACE='sed -i.bak'

# cleanup
trap 'find examples -name "*.bak" -delete' EXIT

usage() {
  echo "Usage: $0 <VERSION>"
  echo "Updates the Rust examples SDK dependency to the specified version"
  echo "Example: $0 v6.0.0"
}

if [[ $# -ne 1 ]]
then
  usage
  exit 1
fi

# Ensure version is an 'official' release
if [[ ! "${VERSION}" =~ ^v[0-9]+.[0-9]+.[0-9]+$ ]]
then
  echo "VERSION doesn't match v[0-9]+.[0-9]+.[0-9]+ and may be a prerelease; skipping."
  exit 1
fi

# Strip the leading v for Cargo.toml
STRIPPED_VERSION="${VERSION#v}"

# Update the version in the Cargo.toml files for each Rust example
find examples -type f -path "examples/*-rust/Cargo.toml" -exec $SED_INPLACE "/^\[dependencies\]/,/^\[/ s/^spin-sdk = \".*\"/spin-sdk = \"${STRIPPED_VERSION}\"/" {} +