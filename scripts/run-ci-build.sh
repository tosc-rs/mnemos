#!/bin/bash

set -euxo pipefail

cargo doc \
    --no-deps \
    --all-features \
    --document-private-items \
    --workspace \
    --exclude crowtty \
    --exclude lichee-rv

rm -rf ./target/ci-publish || :
mkdir -p ./target/ci-publish/
cp -r ./target/doc ./target/ci-publish/

# note: --dest-dir is relative from the book.toml
./target/mdbook \
    build \
    --dest-dir ./../target/ci-publish/book \
    ./book

cp ./assets/index.html ./target/ci-publish
