#!/bin/bash
# This script updates the `spin-sdk` dependency in Rust template
# `Cargo.toml.tmpl` files under the `templates` directory.
set -euo pipefail

VERSION=$1

# -i syntax differs between GNU and Mac sed; this usage is supported by both
SED_INPLACE='sed -i.bak'

# cleanup
trap 'find templates -name "*.bak" -delete' EXIT

usage() {
  echo "Usage: $0 <VERSION>"
  echo "Updates the Rust templates SDK dependency to the specified version"
  echo "Example: $0 6.0.0"
}

if [[ $# -ne 1 ]]
then
  usage
  exit 1
fi

# Ensure version is an 'official' release
if [[ ! "${VERSION}" =~ ^[0-9]+.[0-9]+.[0-9]+$ ]]
then
  echo "VERSION doesn't match [0-9]+.[0-9]+.[0-9]+ and may be a prerelease; skipping."
  exit 1
fi


# Update the version in the Cargo.toml.tmpl files for each Rust template
find templates -type f -path "templates/*-rust/content/Cargo.toml.tmpl" -exec $SED_INPLACE "/^\[dependencies\]/,/^\[/ s/^spin-sdk = \".*\"/spin-sdk = \"${VERSION}\"/" {} +