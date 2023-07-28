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

[`allwinner-d1/`]: ./allwinner-d1/
[`allwinner-d1/boards/`]: ./allwinner-d1/boards/
[`allwinner-d1/core/`]: ./allwinner-d1/core/
[`esp32c3/`]: ./esp32c3
[`melpomene/`]: ./melpomene

## License

[MIT] + [Apache 2.0].

[MIT]: ./../../LICENSE-MIT
[Apache 2.0]: ./../../LICENSE-APACHE
