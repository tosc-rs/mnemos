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
_d1_dir := "platforms/allwinner-d1/boards"

_pomelo_dir := "platforms/pomelo"

_melpo_dir := "platforms/melpomene"

_espbuddy_dir := "platforms/esp32c3-buddy"

# arguments to pass to all RustDoc invocations
_rustdoc := _cargo + " doc --no-deps --all-features"

alias melpo := melpomene

# default recipe to display help information
default:
    @echo "justfile for Mnemos"
    @echo "see https://just.systems/man for more details"
    @echo ""
    @just --list

# check all packages, across workspaces
check: && (_check-dir _pomelo_dir ) (_check-dir _melpo_dir) (_check-dir _espbuddy_dir)
    {{ _cargo }} check \
        --workspace \
        --lib --bins --examples --tests --benches --all-features \
        {{ _fmt }}

# run Clippy checks for all packages, across workspaces.
clippy: && (_clippy-dir _pomelo_dir ) (_clippy-dir _melpo_dir) (_clippy-dir _espbuddy_dir)
    {{ _cargo }} clippy --workspace \
        --lib --bins --examples --tests --benches --all-features \
        {{ _fmt }}

# test all packages, across workspaces
test: (_get-cargo-command "nextest" "cargo-nextest" no-nextest)
    {{ _cargo }} nextest run --workspace --all-features
    # uncomment this if we actually add tests to the D1 platform
    # (cd {{ _d1_dir }}; {{ _cargo }} nextest run --workspace)

# run rustfmt for all packages, across workspaces
fmt:
    {{ _cargo }} fmt --all
    (cd {{ _d1_dir }}; {{ _cargo }} fmt --all)

# build a Mnemos binary for the Allwinner D1
build-d1 board='mq-pro': (_get-cargo-command "objcopy" "cargo-binutils")
    cd {{ _d1_dir}} && {{ _cargo }} build --bin {{ board }} --release
    cd {{ _d1_dir}} && \
        {{ _cargo }} objcopy \
        --bin {{ board }} \
        --release \
        -- \
        -O binary \
        ./{{ _d1_bin_path }}/mnemos-{{ board }}.bin

# flash an Allwinner D1 using xfel
flash-d1 board='mq-pro': (build-d1 board)
    xfel ddr d1
    xfel write {{ _d1_start_addr }} {{ _d1_dir}}/{{ _d1_bin_path }}/mnemos-{{ board }}.bin
    xfel exec {{ _d1_start_addr }}

# build a MnemOS binary for the ESP32-C3
build-c3 board:
    cd {{ _espbuddy_dir }} && \
    {{ _cargo }} build \
        --release \
        --bin {{ board }}

# flash an ESP32-C3 with the MnemOS WiFi Buddy firmware
flash-c3 board *espflash-args: (_get-cargo-command "espflash" "cargo-espflash") (build-c3 board)
    cd {{ _espbuddy_dir }} && \
        {{ _cargo }} espflash flash \
            --release \
            --bin {{ board }} \
            {{ espflash-args }}

# run crowtty (a host serial multiplexer, log viewer, and pseudo-keyboard)
crowtty *FLAGS:
    @{{ _cargo }} run --release --bin crowtty -- {{ FLAGS }}

# run the Melpomene simulator
melpomene *FLAGS:
    @cd {{ _melpo_dir }}; \
        cargo run --release --bin melpomene -- {{ FLAGS }}

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

# run clippy for a subdirectory
_clippy-dir dir:
    cd {{ dir }}; {{ _cargo }} clippy --lib --bins {{ _fmt }}

# run cargo check for a subdirectory
_check-dir dir:
    cd {{ dir }}; {{ _cargo }} check --lib --bins {{ _fmt }}