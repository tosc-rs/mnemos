#!/bin/bash

set -euxo pipefail

defaultmembers=$(cargo metadata --format-version 1 | jq .workspace_default_members | grep -E '  ".*' | cut -d" " -f3 | cut -d'"' -f2 | sed -E 's/(.*)/-p \1 /g' | tr -d '\n')

./just docs $defaultmembers

rm -rf ./target/ci-publish || :
mkdir -p ./target/ci-publish/
cp -r ./target/doc ./target/ci-publish/

# Build with oranda
oranda build

# Copy to publish directory
cp -r ./public/* ./target/ci-publish
