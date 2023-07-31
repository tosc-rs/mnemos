#!/bin/bash

set -euxo pipefail

# make sure the target output directory exists
mkdir -p ./target

# Install docs deps
rustup toolchain install stable

# Install Oranda
# TODO: switch to curlbash once they release a prerelease binary!
cargo install oranda -f \
    --git https://github.com/axodotdev/oranda \
    --rev ec4b0b8360b0adc1aa5240e213bf275b262670e2
