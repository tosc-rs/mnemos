# MnemOS for the Allwinner D1

This directory contains MnemOS platform support for the Allwinner D1 RISC-V SoC.

## Folder Layout

* [`boards/`]: Platform implementations for supported D1
    single-board computers. This crate contains the actual bin targets for
    building MnemOS for D1 SBCs.
* [`core/`]: Core platform implementation shared by all D1 boards.

[`boards/`]: ./boards/
[`core/`]: ./core/

## Building MnemOS for the D1

> **Note** The `boards/` directory is its own Cargo workspace. This is in order
> to avoid blowing away artifacts for host tools cached in the main workspace when
> building the MnemOS binary for a target.

To build for the Allwinner D1 platform, either build from within the
`allwinner-d1/boards/` directory, or use the [`just build-d1` Just
recipe][just].

This crate contains a separate Cargo bin target for each supported D1 board.
These bin targets depend on the [`mnemos-d1-core` crate] for the majority of the
platform implementation, and configure the D1's I/O pins based on how those pins
are mapped to pins on the board. The following bin targets are currently
provided:

* `mq-pro`: MnemOS for the [MangoPi MQ Pro]
* `lichee-rv`: MnemOS for the [Sipeed Lichee RV]

The `just build-d1` recipe takes an optional argument to select which bin target
is built; by default, the `mq-pro` bin target is selected. For example:

```console
$ just build-d1             # builds MnemOS for the MangoPi MQ Pro
$ just build-d1 mq-pro      # also builds MnemOS for the MQ Pro
$ just build-d1 lichee-rv   # builds MnemOS for the Lichee RV
```

## Flashing a D1 SBC

In addition, the `just flash-d1` recipe will build MnemOS and then flash a D1
board using [`xfel`]. Like `build-d1`, this recipe takes an optional argument to
select which board target to build, and defaults to building for the MQ Pro if
none is provided.

For example, running `just flash-d1 mq-pro` should print output like this:

```console
$ just flash-d1 mq-pro
       Found cargo objcopy
   Compiling mnemos-d1 v0.1.0 (/home/eliza/Code/mnemos/platforms/allwinner-d1/boards)
    Finished release [optimized] target(s) in 2.66s
    Finished release [optimized] target(s) in 0.07s
xfel ddr d1
xfel write 0x40000000 platforms/allwinner-d1/boards/target/riscv64imac-unknown-none-elf/mnemos-mq-pro.bin
100% [================================================] 241.281 KB, 450.510 KB/s
xfel exec 0x40000000
```

> **Note**
> When flashing the MangoPi MQ Pro using `just flash-d1`, ensure that the USB
> cable is plugged in to the USB-C port on the board labeled as "OTG" on the
> silkscreen, *not* the one labeled as "HOST".

Once a board has been successfully flashed, attempting to flash it again using
`xfel` may fail. This can be fixed by unplugging the USB cable from the board
and then plugging it back in.

#### Dependencies

In order to use the `just flash-d1` recipe, the [`cargo-binutils`] Cargo plugin
is required. If it is not found, the `flash-d1` recipe will prompt the user to
install it.

The `llvm-tools-preview` Rustup component is a dependency of `cargo-binutils`.
It should be automatically installed by the [`rust-toolchain.toml`] file in this
repo, but can be manually installed by running
`$ rustup component add llvm-tools-preview`.

Finally, [`xfel`] is necessary to actually flash the board. Instructions for
building `xfel` from source for Linux, MacOS, and Windows can be found
[here][xfel-build]. Pre-built `xfel` binaries for Windows are available
[here][xfel-win].

> **Note**
> In addition to the official distribution channels, I (Eliza) have written [a
> Nix derivation for `xfel`][xfel-nix]. Eventually, I'd like to upstream this to
> Nixpkgs, but it can currently be used as a git dependency. Note that when
> using this, `xfel`'s udev rules must be added to the system's udev rules; see
> [here][xfel-nix-udev] for an example.

[just]: ./../../../justfile
[`mnemos-d1-core` crate]: ./../core/
[MangoPi MQ Pro]: https://github.com/mangopi-sbc/MQ-Pro
[Sipeed Lichee RV]: https://wiki.sipeed.com/hardware/en/lichee/RV/RV.html
[`xfel`]: https://xboot.org/xfel/#/
[`cargo-binutils`]: https://crates.io/crates/cargo-binutils
[`rust-toolchain.toml`]: ./../../../rust-toolchain.toml
[xfel-build]: https://xboot.org/xfel/#/?id=build-from-source
[xfel-win]: https://xboot.org/xfel/#/?id=windows-platform
[`xfel-nix`]: https://github.com/hawkw/dotfiles/blob/736d80487687b0610a1b17f5bbec6b22a501207c/nixos/pkgs/xfel.nix
[`xfel-nix-udev`]: https://github.com/hawkw/dotfiles/blob/736d80487687b0610a1b17f5bbec6b22a501207c/nixos/machines/noctis.nix#L102-L104

## License

[MIT] + [Apache 2.0].

[MIT]: ./../../../LICENSE-MIT
[Apache 2.0]: ./../../../LICENSE-APACHE
