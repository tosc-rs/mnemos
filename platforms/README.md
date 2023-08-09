# MnemOS Hardware Platforms

This directory contains code for running MnemOS on the supported hardware and simulation platforms.

## Folder Layout

* [`allwinner-d1/`] - MnemOS for the Allwinner D1 RISC-V SoC
  - [`allwinner-d1/boards/`]: Platform implementations for supported D1
        single-board computers.
  - [`allwinner-d1/core/`]: MnemOS core for all Allwinner D1 boards
* [`esp32c3-buddy/`] - MnemOS ESP32-C3 WiFi Buddy firmware
* [`melpomene/`] - Melpomene is a desktop simulator for MnemOS development
* [`pomelo/`] - Pomelo is a web/wasm simulator for MnemOS development
* [`x86_64`] - MnemOS for x86_64/amd64 CPUs
  - [`x86_64/bootloader/] - Target for building a bootable kernel image using
    [`rust-osdev/bootloader`] as the bootloader.
  - [`x86_64/core/`] - MnemOS core kernel for x86_64

[`allwinner-d1/`]: ./allwinner-d1/
[`allwinner-d1/boards/`]: ./allwinner-d1/boards/
[`allwinner-d1/core/`]: ./allwinner-d1/core/
[`esp32c3-buddy/`]: ./esp32c3-buddy/
[`melpomene/`]: ./melpomene
[`pomelo/`]: ./pomelo
[`x86_64/`]: ./x86_64
[`x86_64/bootloader/`]: ./x86_64/bootloader/
[`x86_64/core/`]: ./x86_64/core/
[`rust-osdev/bootloader`]: https://github.com/rust-osdev/bootloader

## License

[MIT] + [Apache 2.0].

[MIT]: ./../../LICENSE-MIT
[Apache 2.0]: ./../../LICENSE-APACHE
