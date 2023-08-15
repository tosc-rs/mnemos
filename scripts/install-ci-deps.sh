#!/bin/bash

set -euxo pipefail

# make sure the target output directory exists
mkdir -p ./target

# Make sure netlify has sudo
apt-get update || :
apt-get install -y sudo || :

# Install libudev
sudo apt-get update
sudo apt-get install -y libudev-dev

# Install Oranda
curl \
    --proto '=https' \
    --tlsv1.2 \
    -LsSf \
    https://github.com/axodotdev/oranda/releases/download/v0.3.0-prerelease.4/oranda-installer.sh \
    | sh

# Install just
curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | bash -s -- --to .
