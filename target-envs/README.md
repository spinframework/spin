# Spin target environments

This folder contains target environment definitions for Spin. Each target
environment is a TOML file which maps triggers to the WIT worlds supported by
those triggers. When pushed to an OCI registry, these can be referenced
in the Spin manifest `application.targets` array.

Pushed environment definitions should not include the `.toml` extension
and should be versioned using OCI versioning, e.g. `spin-up:3.4`
We avoid using this convention for the source files because 1. syntax
highlighting and 2. Windows filenames.

Versions should include _minor version only_ because WITs should not
change in patch releases.
