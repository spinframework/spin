#!/bin/bash
set -euo pipefail 

oras push ghcr.io/spinframework/environments/spin-up:$VERSION target-envs/spin-up.$VERSION.toml
