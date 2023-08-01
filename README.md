<img src = "./assets/logo-mnemos-1280px.png" width = "600" alt="MnemOS" />

This repository is for the MnemOS Operating System.

## Development and API Docs

`rustdoc` output for the current `main` branch can be built locally with `cargo doc --open`.

[hosted mnemOS `main` branch documentation](https://mnemos-dev.jamesmunns.com/) - includes "the mnemOS book" and source/API documentation.

## Development Blogs

We've written a series of development blogs following the story of MnemOS'
implementation. You can find them here:

- [MnemOS Moment 1][moment-1], by James Munns (2023-06-02)
- [MnemOS Moment 2: Search for a Shell][moment-2], by James Munns (2023-07-10)

[moment-1]: https://onevariable.com/blog/mnemos-moment-1/
[moment-2]: https://onevariable.com/blog/mnemos-moment-2/

## Folder Layout

The project layout contains the following folders:

* [`assets/`] - images and files used for READMEs and other documentation
* [`book/`] - This is the source of "the mnemOS book"
* [`hardware/`] - Hardware designs for MnemOS systems, including CAD files and documentation
* [`source/`] - This folder contains the source code of the cross-platform kernel, userspace, and related libraries
* [`platforms/`] - This folder contains code specific to each targeted hardware and simulation platform
* [`rfcs/`] - MnemOS design RFCs
* [`tools/`] - This folder contains desktop tools used for working with MnemOS

[`assets/`]: ./assets/
[`book/`]: ./book/
[`hardware/`]: ./hardware/
[`source/`]: ./source/
[`platforms/`]: .platforms/
[`tools/`]: ./tools/
[`rfcs/`]: ./rfcs/

## Getting Started

Currently, the primary supported hardware platform for MnemOS is the
Allwinner D1, a RISC-V system-on-chip (SOC). Instructions for running MnemOS on
D1 single-board computer can be found in [`platforms/allwinner-d1/README.md`].

If you don't have access to a supported D1 board, or want a quicker development
workflow for testing cross-platform changes, MnemOS also has a software
simulator, called [Melpomene]. Melpomene runs as a userspace application binary on
a development machine, and runs the MnemOS kernel with simulated hardware.
Melpomene can be run using the `just melpomene` [`just` recipe], or using
`cargo run --bin melpomene`.

[`platforms/allwinner-d1/README.md`]: ./platforms/allwinner-d1/README.md
[Melpomene]: ./platforms/melpomene
[`just` recipe]: ./justfile

## Getting Involved

Join us on Matrix: [#mnemos-dev:beeper.com](https://matrix.to/#/#mnemos-dev:beeper.com)

## License

[MIT] + [Apache 2.0].

[MIT]: ./LICENSE-MIT
[Apache 2.0]: ./LICENSE-APACHE

## Code of Conduct

The MnemOS project follows the [Contributor Covenant Code of Conduct](./code_of_conduct.md).
