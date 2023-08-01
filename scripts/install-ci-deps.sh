#!/bin/bash

set -euxo pipefail

# make sure the target output directory exists
mkdir -p ./target

# Install docs deps
rustup toolchain install stable

# Install Oranda
curl \
    --proto '=https' \
    --tlsv1.2 \
    -LsSf \
    https://github.com/axodotdev/oranda/releases/download/v0.3.0-prerelease.4/oranda-installer.sh \
    | sh
