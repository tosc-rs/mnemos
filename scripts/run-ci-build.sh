#!/bin/bash

# TODO -euo pipefail isn't optimal but I haven't got the energy to improve further -Anatol
set -euxo pipefail

# ensure cargo builds in the default directory (./target)
unset CARGO_TARGET_DIR

# the old filter removed crowtty and manganese, but manganese isn't in deps anymore
# TODO crowtty is currently being filtered because of netlify; when we migrate off of them
# the del part can be done away with.
# 
# explanation of jq pipe:
# 1. extract only contents of `workspace_default_members`
# 2. remove entries containing the string "crowtty"
# 3. split each element on " ", and keep the first item of the split result
# 4. convert each element to a `{key: array_idx, value: elem_value}` dict 
# 5. map these dicts to "-p elem_dict['value']"

defaultmembers=$( \
    cargo metadata --format-version 1 | \
    jq -r '.workspace_default_members 
    | del(.[] | select(contains("crowtty")))
    | [ .[] | split(" ")[0] ] 
    | to_entries[]
    |"-p \(.value)"'
)

./just docs --document-private-items $defaultmembers

rm -rf ./target/ci-publish || :
mkdir -p ./target/ci-publish/
cp -r ./target/doc ./target/ci-publish/

# Add RFCs to the mdbook before building the Oranda site
./scripts/rfc2book.py

# Build with oranda
oranda build

# Copy to publish directory
cp -r ./public/* ./target/ci-publish
