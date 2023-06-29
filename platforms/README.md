# MnemOS Hardware Platforms

This directory contains code for running MnemOS on the supported hardware
platforms.

## Folder Layout

* [`allwinner-d1/`] - MnemOS for the Allwinner D1 RISC-V SoC
  - [`allwinner-d1/boards/`]: Platform implementations for supported D1
        single-board computers
    + [`allwinner-d1/boards/lichee-rv/`]: MnemOS for the [Lichee RV]
    + [`allwinner-d1/boards/mq-pro/`]: MnemOS for the [MangoPi MQ Pro]
  - [`allwinner-d1/core/`]: MnemOS core for all Allwinner D1 boards

[`allwinner-d1/`]: ./allwinner-d1/
[`allwinner-d1/boards/`]: ./allwinner-d1/boards/
[`allwinner-d1/boards/lichee-rv/`]: ./allwinner-d1/boards/lichee-rv
[`allwinner-d1/boards/mq-pro/`]: ./allwinner-d1/boards/mq-pro
[`allwinner-d1/core/`]: ./allwinner-d1/core/

Note that the `boards/` directory is its own Cargo workspace. This is in order
to avoid blowing away artifacts for host tools cached in the main workspace when
building the MnemOS binary for a target.

To build for the Allwinner D1 platform, either build from within the
`allwinner-d1/boards/` directory, or use the [`just build-d1` Just
recipe][just].

[just]: ../justfile

## License

[MIT] + [Apache 2.0].

[MIT]: ./../../LICENSE-MIT
[Apache 2.0]: ./../../LICENSE-APACHE
