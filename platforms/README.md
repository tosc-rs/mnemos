# MnemOS Hardware Platforms

This directory contains code for running MnemOS on the supported hardware and simulation platforms.

## Folder Layout

* [`allwinner-d1/`] - MnemOS for the Allwinner D1 RISC-V SoC. See [here](allwinner-d1#getting-started-with-mnemos-on-the-d1) for list of supported boards and how to get started.
* [`esp32c3-buddy/`] - MnemOS ESP32-C3 WiFi Buddy firmware
* [`melpomene/`] - Melpomene is a desktop simulator for MnemOS development
* [`pomelo/`] - Pomelo is a web/wasm simulator for MnemOS development
* [`x86_64/`] - MnemOS for x86_64/amd64 CPUs
  - [`x86_64/bootloader/`] - Target for building a bootable kernel image using
    [`rust-osdev/bootloader`] as the bootloader.
  - [`x86_64/core/`] - MnemOS core kernel for x86_64

[`allwinner-d1/`]: ./allwinner-d1/
[`esp32c3-buddy/`]: ./esp32c3-buddy/
[`melpomene/`]: ./melpomene
[`pomelo/`]: ./pomelo
[`x86_64/`]: ./x86_64
[`x86_64/bootloader/`]: ./x86_64/bootloader/
[`x86_64/core/`]: ./x86_64/core/
[`rust-osdev/bootloader`]: https://github.com/rust-osdev/bootloader

## License

[MIT] + [Apache 2.0].

[MIT]: https://github.com/tosc-rs/mnemos/blob/main/LICENSE-MIT
[Apache 2.0]:https://github.com/tosc-rs/mnemos/blob/main/LICENSE-APACHE
