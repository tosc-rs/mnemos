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
_d1_bin_path := "target/riscv64imac-unknown-none-elf"
_d1_pkg := "mnemos-d1"

_espbuddy_pkg := "mnemos-esp32c3-buddy"

_x86_bootloader_pkg := "mnemos-x86_64-bootloader"

# arguments to pass to all RustDoc invocations
_rustdoc := _cargo + " doc --no-deps --all-features"

alias melpo := melpomene

# default recipe to display help information
default:
    @echo "justfile for Mnemos"
    @echo "see https://just.systems/man for more details"
    @echo ""
    @just --list

# check all crates, across workspaces
check: && (check-crate _d1_pkg) (check-crate _espbuddy_pkg) (check-crate _x86_bootloader_pkg)
    {{ _cargo }} check \
        --lib --bins --examples --tests --benches \
        {{ _fmt }}

# check a crate.
check-crate crate:
    {{ _cargo }} check \
        --lib --bins --examples --tests --benches --all-features \
        --package {{ crate }} \
        {{ _fmt }}

# run Clippy checks for all crates, across workspaces.
clippy: && (clippy-crate _d1_pkg) (clippy-crate _espbuddy_pkg) (clippy-crate _x86_bootloader_pkg)
    {{ _cargo }} clippy \
        --lib --bins --examples --tests --benches --all-features \
        {{ _fmt }}

# run clippy checks for a crate.
clippy-crate crate:
    {{ _cargo }} clippy \
        --lib --bins --examples --tests --benches \
        --package {{ crate }} \
        {{ _fmt }}

# test all packages, across workspaces
test: (_get-cargo-command "nextest" "cargo-nextest" no-nextest)
    {{ _cargo }} nextest run --all-features

# run rustfmt for all crates, across workspaces
fmt:
    {{ _cargo }} fmt
    {{ _cargo }} fmt --package {{ _d1_pkg }}
    {{ _cargo }} fmt --package {{ _espbuddy_pkg }}
    {{ _cargo }} fmt --package {{ _x86_bootloader_pkg }}

# build a Mnemos binary for the Allwinner D1
build-d1 board='mq-pro': (_get-cargo-command "objcopy" "cargo-binutils")
    {{ _cargo }} build \
        --package {{ _d1_pkg }} \
        --bin {{ board }} \
        --release
    {{ _cargo }} objcopy \
        --package {{ _d1_pkg }} \
        --bin {{ board }} \
        --release \
        -- \
        -O binary {{ _d1_bin_path }}/mnemos-{{ board }}.bin

# flash an Allwinner D1 using xfel
flash-d1 board='mq-pro': (build-d1 board)
    xfel ddr d1
    xfel write {{ _d1_start_addr }} {{ _d1_bin_path }}/mnemos-{{ board }}.bin
    xfel exec {{ _d1_start_addr }}

# build a MnemOS binary for the ESP32-C3
build-c3 board:
    {{ _cargo }} build --release \
        --package {{ _espbuddy_pkg }} \
        --bin {{ board }}

# flash an ESP32-C3 with the MnemOS WiFi Buddy firmware
flash-c3 board *espflash-args: (_get-cargo-command "espflash" "cargo-espflash") (build-c3 board)
    {{ _cargo }} espflash flash \
        --release \
        --package {{ _espbuddy_pkg }} \
        --bin {{ board }} \
        {{ espflash-args }}

# build a bootable x86_64 disk image, using rust-osdev/bootloader.
build-x86 *args='':
    {{ _cargo }} build --package {{ _x86_bootloader_pkg }} {{ args }}

# run an x86_64 MnemOS image in QEMU
run-x86 *args='':
    {{ _cargo }} run -p {{ _x86_bootloader_pkg }} -- {{ args }}

# run crowtty (a host serial multiplexer, log viewer, and pseudo-keyboard)
crowtty *FLAGS:
    @{{ _cargo }} run --release --bin crowtty -- {{ FLAGS }}

# run the Melpomene simulator
melpomene *FLAGS:
    @{{ _cargo }} run --release --bin melpomene -- {{ FLAGS }}

# build all RustDoc documentation
all-docs *FLAGS: (docs FLAGS) (docs "-p " + _d1_pkg + FLAGS) (docs "-p " + _espbuddy_pkg + FLAGS)  (docs "-p " + _x86_bootloader_pkg + FLAGS)

# run RustDoc
docs *FLAGS:
    env RUSTDOCFLAGS="--cfg docsrs -Dwarnings" \
        {{ _cargo }} doc \
        --all-features \
        {{ FLAGS }} \
        {{ _fmt }}

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