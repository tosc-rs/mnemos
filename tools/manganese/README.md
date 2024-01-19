# mn: the MnemOS Package Manager

`mn`, pronounced "manganese", is the (optional!) MnemOS package manager.

this allows us to depend on various build dependencies as *normal cargo
dependencies*, so you don't have to install them manually.

yes, this is a wildly deranged idea. i'm so smart.

> **Note**
>
> you don't actually need this to build mnemOS.
>
> you can just run the [`just` recipes][`justfile`] directly, using the
> versions of mnemOS' build dependencies which you've installed using your
> package manager or `cargo install`. this is a purely optional tool intended to
> provide a way to build mnemOS using *only* Cargo, so that the project can be
> built without requiring a complicated dev environment setup.
>
> if you'd rather use versions of mnemOS' build dependencies that you've
> installed through other means, simply "don't run `cargo mn`", and you'll be
> fine.

## using it

`mn` is a [cargo alias] that invokes the Manganese binary. run Manganese using
`cargo mn <subcommand>`.

invoking `cargo mn` without a subcommand will print a list of available
commands, which will look sort of like this:

```shell
$ cargo mn
   Compiling manganese v0.1.0 (/home/eliza/Code/mnemos/tools/manganese)
    Finished release [optimized + debuginfo] target(s) in 3.99s
     Running `target/release/manganese`

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

Available recipes:
    all-docs *FLAGS               # build all RustDoc documentation
    build-c3 board                # build a MnemOS binary for the ESP32-C3
    build-d1 board='mq-pro'       # build a Mnemos binary for the Allwinner D1
    build-x86 *args=''            # build a bootable x86_64 disk image, using rust-osdev/bootloader.
    check *ARGS                   # check all crates, across workspaces
    check-crate crate *ARGS       # check a crate.
    clippy *ARGS                  # run Clippy checks for all crates, across workspaces.
    clippy-crate crate *ARGS      # NOTE: -Dwarnings is added by _fmt because reasons
    crowtty *FLAGS                # run crowtty (a host serial multiplexer, log viewer, and pseudo-keyboard)
    default                       # default recipe to display help information
    docs *FLAGS                   # run RustDoc
    flash-c3 board *espflash-args # flash an ESP32-C3 with the MnemOS WiFi Buddy firmware
    flash-d1 board='mq-pro'       # flash an Allwinner D1 using xfel
    fmt                           # run rustfmt for all crates, across workspaces
    mdbook CMD="build --open"     # Run a mdBook command, generating the book's RFC section first.
    melpomene *FLAGS              # run the Melpomene simulator
    melpo *FLAGS                  # alias for `melpomene`
    nextest *ARGS                 # run a Nextest command
    oranda CMD="dev"              # Run an Oranda command, generating the book's RFC section first.
    run-x86 *args=''              # run an x86_64 MnemOS image in QEMU
    test *ARGS="--all-features"   # test all packages, across workspaces
    trunk *CMD                    # Run a Trunk command
```

## how it works

manganese is a binary crate which has Cargo [artifact dependencies] on other
binary crates, such as [`just`] and [`cargo-nextest`]. this means that when the
`mn` binary is built, Cargo will also build the binaries for these dependencies.
the `mn` binary is a simple Cargo subcommand that forwards all its arguments to
the `just` binary installed through the bin dep, allowing it to act as a simple
wrapper for the commands defined in the [`justfile`].

the magic, though, is that Manganese has a build script which symlinks all of the
requisite artifact dependencies into a `manganese-bins` directory in Manganese's
cargo `OUT_DIR`. then, when running `just` commands through the `mn` binary,
this directory is prepended to the `$PATH` environment variable for the `just`
command invocation. this means that any binaries produced by `mn`'s artifact
deps will be visible to the `just` recipe that executes.

> **Note**
>
> why is this implemented by modifying the `$PATH` for a `just` invocation,
> rather than just having the `mn` binary know the paths at which all the bin
> deps were installed to and invoking them directly?
>
> this approach is necessary because many of the bin deps managed by Manganese
> are *cargo subcommands*. these binaries expect to be invoked not by running
> the bare `cargo-nextest` or `cargo-espflash` binaries, but *by Cargo*. cargo
> subcommands must be invoked by cargo because these binaries' command-line
> parsing typically expects the first argument to be "cargo" followed by the
> name of the subcommand. and, more importantly, these binaries often depend on
> Cargo environment variables, such as `CARGO_MANIFEST_DIR`. if they were
> invoked directly by `mn`, they would have the versions of those env vars *set
> by cargo when running `mn` through `cargo run`, so (for example)
> `CARGO_MANIFEST_DIR` will point to the *`manganese` crate, rather than to the
> workspace, which is what it would be if cargo invoked that subcommand
> directly. this causes problems when a Cargo subcommand needs this information
> to...do whatever it is the subcommand does.
>
> since cargo discovers subcommands by searching the `$PATH` for binaries
> beginning with `cargo-`, we can simply add our installed bins to the `$PATH`
> and have `cargo` invoke them for us. this indirection results in the correct
> cargo metadata environment variables being set.

[artifact dependencies]: https://doc.rust-lang.org/cargo/reference/unstable.html#artifact-dependencies
[`just`]: https://just.systems
[`cargo-nextest`]: https://crates.io/crates/cargo-nextest
[`justfile`]: https://github.com/tosc-rs/mnemos/blob/main/justfile
[cargo alias]: https://github.com/tosc-rs/mnemos/blob/main/.cargo/config.toml