#!/bin/bash

set -euxo pipefail

# make sure the target output directory exists
mkdir -p ./target

# Install docs deps
rustup toolchain install stable

# Install mdbook deps
curl -L https://github.com/rust-lang/mdBook/releases/download/v0.4.17/mdbook-v0.4.17-x86_64-unknown-linux-gnu.tar.gz | tar xvz
mv ./mdbook ./target
