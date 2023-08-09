# MnemOS x86_64

This directory contains the MnemOS platform implementation for x86_64/amd64
CPUs.


## Folder Layout

* [`bootloader/`] - Target for building a bootable image using
  [`rust-osdev/bootloader`] as the bootloader
* [`core/`] - MnemOS core kernel for x86_64

[`bootloader/`]: ./bootloader/
[`core/`]: ./core/

## Supported Bootloaders

Currently, [`rust-osdev/bootloader`] is the only supported bootloader. A MnemOS
image using this bootloader is built by the [`bootloader/`] crate in this
directory.[`rust-osdev/bootloader`] can be used to build both BIOS and UEFI
images.

## Getting started with MnemOS on the ESP32-C3

### Building

To build x86_64 boot images, either run
`cargo build -p mnemos-x86_64-bootloader`, or use the `just build-x86`
[`just` recipe][just].

After running this command, BIOS and UEFI boot image files (named
`mnemos-x86_64-bios.img` and `mnemos-x86_64-uefi.img`, respectively) will be
output to the build script's [Cargo `$OUT_DIR`][outdir]. By default, the
`$OUT_DIR` is `target/{debug,
release}/build/mnemos-x86_64-bootloader-{hash}/out`.

### Running in QEMU

To run MnemOS in [QEMU], either run `cargo run -p mnemos-x86_64-bootloader` or
use the `just run-x86` [`just` recipe][just].

> [!IMPORTANT]
>
> In order to run either of these commands, a [`qemu-system-x86_64`][QEMU]
> binary must beinstalled.

MnemOS can boot using either legacy [BIOS] or [UEFI] (using [`ovmf-prebuilt`]).
The `--boot` argument can be passed to `just run-x86` to determine which boot
method is used:

```console
$ just run-x86 --boot uefi # boots using UEFI
$ just run-x86 --boot bios # boots using legacy BIOS
```

UEFI-based boot is the default if no argument is passed.

Additional command-line arguments can be passed to configure the behavior of the
bootimage builder. Run `just run-x86 --help` to list them.

[QEMU]: https://www.qemu.org
[just]: ./../../../justfile
[`rust-osdev/bootloader`]: https://github.com/rust-osdev/bootloader
[BIOS]: https://en.wikipedia.org/wiki/BIOS
[UEFI]: https://en.wikipedia.org/wiki/UEFI
[`ovmf-prebuilt`]: https://github.com/rust-osdev/ovmf-prebuilt
[outdir]: https://doc.rust-lang.org/cargo/reference/environment-variables.html#environment-variables-cargo-sets-for-build-scripts

## License

[MIT] + [Apache 2.0].

[MIT]: ./LICENSE-MIT
[Apache 2.0]: ./LICENSE-APACHE
