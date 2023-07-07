<img src = "./assets/logo-mnemos-1280px.png" width = "600" alt="MnemOS" />

This repository is for the MnemOS Operating System.

## Stable Docs

Currently, MnemOS is being rewritten as part of the v0.2 version. The current source may not
match the currently published documentation!

[hosted mnemOS v0.1 documentation](https://mnemos.jamesmunns.com)

## Development and API Docs

`rustdoc` output for the current `main` branch can be built locally with `cargo doc --open`.

[hosted mnemOS `main` branch documentation](https://mnemos-dev.jamesmunns.com/) - includes "the mnemOS book" and source/API documentation.

## Folder Layout

The project layout contains the following folders:

* [`assets/`] - images and files used for READMEs and other documentation
* [`book/`] - This is the source of "the mnemOS book"
* [`source/`] - This folder contains the source code of the cross-platform kernel, userspace, simulator, and related libraries
* [`platforms/`] - This kernel contains code specific to each targeted hardware platform
* [`tools/`] - This folder contains desktop tools used for working with MnemOS

[`assets/`]: ./assets/
[`book/`]: ./book/
[`source/`]: ./source/
[`platforms/`]: .platforms/
[`tools/`]: ./tools/

## Getting Started

Currently, the primary supported hardware platform for MnemOS is the
Allwinner D1, a RISC-V system-on-chip (SOC). Instructions for running MnemOS on
D1 single-board computer can be found in [`platforms/allwinner-d1/README.md`].

If you don't have access to a supported D1 board, or want a quicker development
workflow for testing cross-platform changes, MnemOS also has a software
simulator, called [Melpomene]. Melpomene runs as a userspace application binary on
a development machine, and runs the MnemOS kernel with simulated hardware.
Melpomene can be run using the `cargo melpomene` Cargo alias.

[`platforms/allwinner-d1/README.md`]: ./platforms/allwinner-d1/README.md
[Melpomene]: ./source/melpomene

## Getting Involved

Join us on Matrix: [#mnemos-dev:beeper.com](https://matrix.to/#/#mnemos-dev:beeper.com)

## License

[MIT] + [Apache 2.0].

[MIT]: ./LICENSE-MIT
[Apache 2.0]: ./LICENSE-APACHE
