#!/usr/bin/env -S just --justfile
# justfile for Mnemos
# see https://just.systems/man for more details


# Overrides the default Rust toolchain set in `rust-toolchain.toml`.
toolchain := ""

# disables cargo nextest
no-nextest := ''

_cargo := "cargo" + if toolchain != "" { " +" + toolchain } else { "" }

_rustflags := env_var_or_default("RUSTFLAGS", "")

# If we're running in Github Actions and cargo-action-fmt is installed, then add
# a command suffix that formats errors.
_fmt := if env_var_or_default("GITHUB_ACTIONS", "") != "true" { "" } else {
    ```
    if command -v cargo-action-fmt >/dev/null 2>&1; then
        echo "--message-format=json | cargo-action-fmt"
    fi
    ```
}

_d1_start_addr := "0x40000000"
_d1_bin_path := "target/riscv64imac-unknown-none-elf/mnemos.bin"

# arguments to pass to all RustDoc invocations
_rustdoc := _cargo + " doc --no-deps --all-features"

# default recipe to display help information
default:
    @echo "justfile for Mnemos"
    @echo "see https://just.systems/man for more details"
    @echo ""
    @just --list

# build a Mnemos binary for the Allwinner D1
build-d1: (_get-cargo-binutils)
    (cd platforms/allwinner-d1/boards/lichee-rv; \
        {{ _cargo }} objcopy --release -- \
            -O binary \
            ../../../../{{ _d1_bin_path }} \
    )

# flash an Allwinner D1 using xfel
flash-d1: (build-d1)
    xfel ddr d1
    xfel write {{ _d1_start_addr }} {{ _d1_bin_path }}
    xfel exec {{ _d1_start_addr }}

_get-cargo-binutils:
    #!/usr/bin/env bash
    set -euo pipefail
    source "./scripts/_util.sh"

    if {{ _cargo }} --list | grep -q 'objcopy'; then
        status "Found" "cargo objcopy"
        exit 0
    fi

    err "missing cargo-objcopy executable"
    if confirm "      install it?"; then
        cargo install cargo-binutils
    fi


_get-nextest:
    #!/usr/bin/env bash
    set -euo pipefail
    source "./scripts/_util.sh"

    if [ -n "{{ no-nextest }}" ]; then
        status "Configured" "not to use cargo nextest"
        exit 0
    fi

    if {{ _cargo }} --list | grep -q 'nextest'; then
        status "Found" "cargo nextest"
        exit 0
    fi

    err "missing cargo-nextest executable"
    if confirm "      install it?"; then
        cargo install cargo-nextest
    fi
