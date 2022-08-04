#!/bin/bash

set -euxo pipefail

cargo doc \
    --no-deps \
    --all-features \
    --document-private-items \
    --workspace \
    --exclude crowtty

rm -rf ./target/ci-publish || :
mkdir -p ./target/ci-publish/
cp -r ./target/doc ./target/ci-publish/

./target/mdbook \
    build \
    --dest-dir ./target/ci-publish/book \
    ./book

cp ./assets/index.html ./target/ci-publish
