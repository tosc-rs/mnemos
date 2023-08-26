# mn: the MnemOS Package Manager

`mn`, pronounced "manganese", is the (optional!) MnemOS package manager.

this allows us to depend on various build dependencies as *normal cargo
dependencies*, so you don't have to install them manually.

yes, this is a wildly deranged idea. i'm so smart.

## how it works

manganese is a binary crate which has Cargo [artifact dependencies] on other
binary crates, such as [`just`] and [`cargo-nextest`]. this means that when the
`mn` binary is built, Cargo will also build the binaries for these dependencies.
the `mn` binary is a simple Cargo subcommand that forwards all its arguments to
the `just` binary installed through the bin dep, allowing it to act as a simple
wrapper for the commands defined in the [`justfile`](../justfile).

the magic, though, is that Manganese has a build script which symlinks all of the
requisite artifact dependencies into a `manganese-bins` directory in Manganese's
cargo `OUT_DIR`. then, when running `just` commands through the `mn` binary,
this directory is prepended to the `$PATH` environment variable for the `just`
command invocation. this means that any binaries produced by `mn`'s artifact
deps will be visible to the `just` recipe that executes.

> [!NOTE]
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