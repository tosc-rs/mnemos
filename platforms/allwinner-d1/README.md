# MnemOS for the Allwinner D1

This directory contains MnemOS platform support for the Allwinner D1 RISC-V SoC.

## Folder Layout

* [`boards/`]: Platform implementations for supported D1
    single-board computers. This crate contains the actual bin targets for
    building MnemOS for D1 SBCs.
* [`core/`]: Core platform implementation shared by all D1 boards.

[`boards/`]: ./boards/
[`core/`]: ./core/

## Getting started with MnemOS on the D1

### Building

> **Note**
>
> The `boards/` directory is its own Cargo workspace. This is in order  to avoid
> blowing away artifacts for host tools cached in the main workspace when
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

### Running

The quickest way to get MnemOS running on a D1 SBC is using [`xfel`].
An explanation of `xfel` and alternative ways to run MnemOS follows
in a [later section](#boot-procedure).

The `just flash-d1` recipe will build MnemOS and then flash it to your D1 board.
Like `build-d1`, this recipe takes an optional argument to
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
>
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
>
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
[xfel-nix]: https://github.com/hawkw/dotfiles/blob/736d80487687b0610a1b17f5bbec6b22a501207c/nixos/pkgs/xfel.nix
[xfel-nix-udev]: https://github.com/hawkw/dotfiles/blob/736d80487687b0610a1b17f5bbec6b22a501207c/nixos/machines/noctis.nix#L102-L104

## Boot Procedure
On reset, the D1 executes its internal *Boot ROM* (`BROM`), which either loads
a first stage bootloader or enters *FEL mode*.

The `BROM` will:
* Do some initial setup of the clocks
* Check the *FEL pin*: if it is low (connected to GND), it will enter FEL mode
* Check the SD card (SMHC0) and connected SPI flash for a valid [eGON header]
* Fall back to FEL mode if no valid header is found

You can find additional info about [`BROM`] and [FEL] on the *linux-sunxi* wiki.
In short, in FEL mode the D1 will present itself as a USB device and allow
(using a custom protocol) things like reading data, writing data and starting
code execution.
Different tools like [`sunxi-fel`] and [`xfel`] have been developed that speak
this protocol and implement functionality like initializing DRAM
(by loading code that does this into SRAM and then executing it).
However, this is all volatile, so these actions have to be repeated on reset.

To have a persistent boot, we can use one of the media that is probed
by the `BROM` for an eGON header: the SD card or SPI flash
(if you have this on your board).
On this persistent medium, a first stage bootloader needs to be present,
preceded by a valid eGON header.
The `BROM` will then load it into SRAM and execute it.

This bootloader has to, at a minimum, initialize DRAM and load either
a second stage bootloader or the actual application (in our case MnemOS)
into DRAM so it can transfer control of execution to it.

### DRAM initialization
The initialization code for the DDR3 RAM is somewhat of a
[black box][sunxi wiki DRAM]. It is hard to determine who was first in
reverse-engineering the necessary Allwinner blobs, but one candidate
is [this][pnru boot0], which served as a basis for the original
[SPL work][sun20i_d1_spl] by smaeul. This is now deprecated, as it is included
in smaeul's [u-boot fork][u-boot mctl], which will hopefully be merged upstream
one day.

[Oreboot] has a [Rust port][oreboot mctl] of this code.
TODO: figure out why some of the DRAM parameters depend on
which board is selected.

### SD card layout
The *linux-sunxi* wiki has some [more information][sdcard-layout] on
the required layout of the SD card.
The eGON header with first stage bootloader has to be located at an offset
of either 8KB or 128KB, in order to leave some space for a partition table
(so you can have, e.g., a FAT filesystem on your SD card at the same time).

To prepare your SD card, you can do the following (where `sdX` is the SD card):
```sh
sudo dd if=first-stage-boot.bin of=/dev/sdX bs=1024 seek=8 conv=sync
sudo dd if=mnemos.bin of=/dev/sdX bs=1024 seek=40 conv=sync
```

MnemOS currently does not have its own first stage bootloader,
but it is possible to adapt the [oreboot bt0] for this role.

[`BROM`]: https://linux-sunxi.org/BROM
[FEL]: https://linux-sunxi.org/FEL
[eGON header]: https://linux-sunxi.org/EGON
[`sunxi-fel`]: https://github.com/linux-sunxi/sunxi-tools/
[Oreboot]: https://github.com/oreboot/oreboot
[oreboot mctl]: https://github.com/oreboot/oreboot/blob/main/src/mainboard/sunxi/nezha/bt0/src/mctl.rs
[oreboot bt0]: https://github.com/oreboot/oreboot/tree/main/src/mainboard/sunxi/nezha/bt0
[u-boot mctl]: https://github.com/smaeul/u-boot/blob/d1-wip/drivers/ram/sunxi/mctl_hal-sun20iw1p1.c
[sun20i_d1_spl]: https://github.com/smaeul/sun20i_d1_spl/blob/mainline/drivers/dram/sun20iw1p1/lib-dram/mctl_hal.c
[pnru boot0]: https://gitlab.com/pnru/boot0
[sunxi wiki DRAM]: https://linux-sunxi.org/Allwinner_Nezha#DRAM_Driver
[sdcard-layout]: https://linux-sunxi.org/Bootable_SD_card#SD_Card_Layout

## License

[MIT] + [Apache 2.0].

[MIT]: ./../../../LICENSE-MIT
[Apache 2.0]: ./../../../LICENSE-APACHE
