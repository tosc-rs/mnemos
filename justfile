#!/usr/bin/env -S just --justfile
_docstring := "
justfile for MnemOS
see https://just.systems/man for more details

Available variables:
    toolchain       # overrides the default Rust toolchain set in the
                    # rust-toolchain.toml file.
    no-nextest      # disables cargo nextest (use cargo test) instead.
    profile         # configures what Cargo profile (release or debug) to use
                    # for builds.

Variables can be set using `just VARIABLE=VALUE ...` or
`just --set VARIABLE VALUE ...`.

See https://just.systems/man/en/chapter_36.html for details.
"

# Overrides the default Rust toolchain set in `rust-toolchain.toml`.
toolchain := ""

# disables cargo nextest
no-nextest := ''

# configures what profile to use for builds. the default depends on the target
# being built.
profile := 'release'

_cargo := "cargo" + if toolchain != "" { " +" + toolchain } else { "" }

_rustflags := env_var_or_default("RUSTFLAGS", "")

# If we're running in Github Actions and cargo-action-fmt is installed, then add
# a command suffix that formats errors.
_fmt_clippy := if env_var_or_default("GITHUB_ACTIONS", "") != "true" { "" } else {
    ```
    if command -v cargo-action-fmt >/dev/null 2>&1; then
        echo "--message-format=json -- -Dwarnings | cargo-action-fmt"
    fi
    ```
}

_fmt_check_doc := if env_var_or_default("GITHUB_ACTIONS", "") != "true" { "" } else {
    ```
    if command -v cargo-action-fmt >/dev/null 2>&1; then
        echo "--message-format=json | cargo-action-fmt"
    fi
    ```
}

_d1_start_addr := "0x40000000"
_d1_bin_path := "target/riscv64imac-unknown-none-elf"
_d1_pkg := "mnemos-d1"

_espbuddy_pkg := "mnemos-esp32c3-buddy"

_x86_pkg := "mnemos-x86_64"

_mn_pkg := "manganese"

_pomelo_pkg := "pomelo"
_pomelo_index_path := "platforms"/_pomelo_pkg/"index.html"

# arguments to pass to all RustDoc invocations
_rustdoc := _cargo + " doc --no-deps --all-features"

alias melpo := melpomene

# default recipe to display help information
default:
    @echo '{{ _docstring }}'
    @just --list

# check all crates, across workspaces
check *ARGS: && (check-crate _d1_pkg ARGS) (check-crate _espbuddy_pkg ARGS) (check-crate _x86_pkg ARGS) (check-crate _pomelo_pkg ARGS) (check-crate _mn_pkg ARGS)
    #!/usr/bin/env bash
    set -euxo pipefail
    {{ _cargo }} check \
        --lib --bins --examples --tests --benches \
        {{ ARGS }} \
        {{ _fmt_check_doc }}

# check a crate.
check-crate crate *ARGS:
    #!/usr/bin/env bash
    set -euxo pipefail
    {{ _cargo }} check \
        --package {{ crate }} \
        {{ if crate == _mn_pkg { "" } else { "--all-features --lib" } }} \
        --bins --examples --tests --benches \
        {{ ARGS }} \
        {{ _fmt_check_doc }}

# run Clippy checks for all crates, across workspaces.
clippy *ARGS: && (clippy-crate _d1_pkg ARGS) (clippy-crate _espbuddy_pkg ARGS) (clippy-crate _x86_pkg ARGS) (clippy-crate _mn_pkg ARGS) (clippy-crate _pomelo_pkg ARGS)
    #!/usr/bin/env bash
    set -euxo pipefail
    {{ _cargo }} clippy \
        --lib --bins --examples --tests --benches --all-features \
        {{ ARGS }} \
        {{ _fmt_clippy }}

# run clippy checks for a crate.
# NOTE: -Dwarnings is added by _fmt because reasons
clippy-crate crate *ARGS:
    #!/usr/bin/env bash
    set -euxo pipefail
    {{ _cargo }} clippy \
        --package {{ crate }} \
        {{ if crate == _mn_pkg { "" } else { "--all-features --lib" } }} \
        --bins --examples --tests --benches \
        {{ ARGS }} \
        {{ _fmt_clippy }}

# test all packages, across workspaces
test *ARGS="--all-features": (nextest "run " + ARGS)
    {{ _cargo }} test --doc {{ ARGS }}

# run a Nextest command
nextest *ARGS: (_get-cargo-command "nextest" "cargo-nextest" no-nextest)
    {{ _cargo }} nextest {{ ARGS }}

# run rustfmt for all crates, across workspaces
fmt:
    {{ _cargo }} fmt
    {{ _cargo }} fmt --package {{ _d1_pkg }}
    {{ _cargo }} fmt --package {{ _espbuddy_pkg }}
    {{ _cargo }} fmt --package {{ _x86_pkg }}
    {{ _cargo }} fmt --package {{ _mn_pkg }}

# build a Mnemos binary for the Allwinner D1
build-d1 board='mq-pro' *CARGO_ARGS='': (_get-cargo-command "objcopy" "cargo-binutils")
    {{ _cargo }} build \
        --profile {{ profile }} \
        --package {{ _d1_pkg }} \
        --bin {{ board }} \
        {{ CARGO_ARGS }}
    {{ _cargo }} objcopy \
        --profile {{ profile }} \
        --package {{ _d1_pkg }} \
        --bin {{ board }} \
        {{ CARGO_ARGS }} \
        -- \
        -O binary {{ _d1_bin_path }}/mnemos-{{ board }}.bin

# flash an Allwinner D1 using xfel
flash-d1 board='mq-pro' *CARGO_ARGS='': (build-d1 board CARGO_ARGS)
    xfel ddr d1
    xfel write {{ _d1_start_addr }} {{ _d1_bin_path }}/mnemos-{{ board }}.bin
    xfel exec {{ _d1_start_addr }}

# build a MnemOS binary for the ESP32-C3
build-c3 board *CARGO_ARGS='':
    {{ _cargo }} build \
        --profile {{ profile }} \
        --package {{ _espbuddy_pkg }} \
        --bin {{ board }} \
        {{ CARGO_ARGS }}

# flash an ESP32-C3 with the MnemOS WiFi Buddy firmware
flash-c3 board *espflash-args: (_get-cargo-command "espflash" "cargo-espflash") (build-c3 board)
    {{ _cargo }} espflash \
        --profile {{ profile }} \
        --package {{ _espbuddy_pkg }} \
        --bin {{ board }} \
        {{ espflash-args }}

# build a bootable x86_64 disk image, using rust-osdev/bootloader.
build-x86 *args='': (run-x86 "build " + args)

# run an x86_64 MnemOS image in QEMU
run-x86 *args='':
    {{ _cargo }} run --package {{ _x86_pkg }} \
        --target=x86_64-unknown-none \
        --features=bootloader_api \
        -- {{ args }}

# run crowtty (a host serial multiplexer, log viewer, and pseudo-keyboard)
crowtty *FLAGS:
    {{ _cargo }} run --profile {{ profile }} --bin crowtty -- {{ FLAGS }}

# run the Melpomene simulator
melpomene *FLAGS:
    {{ _cargo }} run --profile {{ profile }} --bin melpomene -- {{ FLAGS }}

# build all RustDoc documentation
all-docs *FLAGS: (docs FLAGS) (docs "-p " + _d1_pkg + FLAGS) (docs "-p " + _espbuddy_pkg + FLAGS) ( docs "-p" + _mn_pkg + FLAGS) (docs "-p " + _pomelo_pkg + FLAGS)

# serve Pomelo and open it in the browser
pomelo *ARGS="--release --open": (trunk "serve " + ARGS + " " + _pomelo_index_path)

# run an arbitrary Trunk command (used for running WASM projects, such as pomelo)
trunk CMD: (_get-cargo-bin "trunk")
    trunk {{ CMD }}

# run RustDoc
docs *FLAGS:
    env RUSTDOCFLAGS="--cfg docsrs -Dwarnings" \
        {{ _cargo }} doc \
        --all-features \
        {{ FLAGS }} \
        {{ _fmt_check_doc }}

# Run a mdBook command, generating the book's RFC section first.
mdbook CMD="build --open": (_get-cargo-bin "mdbook")
    ./scripts/rfc2book.py
    cd book && mdbook {{ CMD }}

# Run an Oranda command, generating the book's RFC section first.
oranda CMD="dev": (_get-cargo-bin "oranda")
    ./scripts/rfc2book.py
    oranda {{ CMD }}

_get-cargo-command name pkg skip='':
    #!/usr/bin/env bash
    set -euo pipefail
    source "./scripts/_util.sh"

    if [ -n "{{ skip }}" ]; then
        status "Configured" "not to use cargo-{{ name }}"
        exit 0
    fi

    if {{ _cargo }} --list | grep -q {{ name }}; then
        status "Found" "cargo-{{ name }}"
        exit 0
    fi

    err "missing cargo-{{ name }} executable"
    if confirm "       install it?"; then
        cargo install {{ pkg }}
    fi

_get-cargo-bin name:
    #!/usr/bin/env bash
    set -euo pipefail
    source "./scripts/_util.sh"

    if command -v {{ name }} >/dev/null 2>&1; then
        status "Found" "{{ name }}"
        exit 0
    fi

    err "missing {{ name }} executable"
    if confirm "       install it?"; then
        cargo install {{ name }}
    fi
