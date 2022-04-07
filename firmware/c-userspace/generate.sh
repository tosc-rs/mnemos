#! /bin/bash

set -euxo pipefail

# Build in release mode - Requires Rust and the
# thumbv7em-none-eabihf target. Install rust  with the
# instructions here:
#
# https://www.rust-lang.org/tools/install
#
# Then, install the embedded target with:
#
# rustup target add thumbv7em-none-eabihf
cargo build --release

# Make an output directory, and put stuff there
mkdir -p ./c-output

# Copy the static library
cp target/thumbv7em-none-eabihf/release/libc_userspace.a ./c-output/libc_userspace.a

# Copy the linker script(s) from the userspace library
cp ../userspace/link.x ./c-output/link.x
cp ../userspace/stack.x ./c-output/stack.x

# Generate the C header files. Requires the `cbindgen` tool.
# Install with:
#
# cargo install cbindgen
cbindgen -l c . > ./c-output/userspace.h
