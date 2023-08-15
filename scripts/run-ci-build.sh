#!/bin/bash

set -euxo pipefail

./just docs

rm -rf ./target/ci-publish || :
mkdir -p ./target/ci-publish/
cp -r ./target/doc ./target/ci-publish/

# Build with oranda
oranda build

# Copy to publish directory
cp -r ./public/* ./target/ci-publish
