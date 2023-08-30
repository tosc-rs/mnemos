<img src = "./assets/logo-mnemos-1280px.png" width = "600" alt="MnemOS" />

MnemOS ([`mnɛːmos`][name]) is a hobby-grade, experimental operating system for
[small computers][d1] (and [bigger ones, too][x86]). The MnemOS kernel and
userspace use a design based on asynchronous message passing, inspired by
[Erlang] and [microkernels], although MnemOS is not a true microkernel.

This repository contains the [cross-platform core of the MnemOS kernel][kernel],
which is implemented as a Rust library crate, [platform-specific
implementation][platforms] for supported hardware and simulator targets,
[development tools][tools] for working on MnemOS, and reusable libraries which
the kernel, tools, and platform implementations depend on.

[name]: https://mnemos.dev/mnemosprojectoverview/book/#where-does-the-name-come-fromhow-do-i-pronounce-it
[d1]: https://github.com/tosc-rs/mnemos/tree/main/platforms/allwinner-d1
[x86]: https://github.com/tosc-rs/mnemos/tree/main/platforms/x86_64
[Erlang]: https://en.wikipedia.org/wiki/Erlang_(programming_language)#Processes
[microkernels]: https://en.wikipedia.org/wiki/Microkernel
[kernel]: https://mnemos.dev/doc/kernel/
[platforms]: https://github.com/tosc-rs/mnemos/tree/main/platforms
[tools]: https://github.com/tosc-rs/mnemos/tree/main/tools

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

[Pomelo] is a web-based simulator, which runs the MnemOS kernel in the browser
using WebAssembly. A hosted version of Pomelo can be found at
[https://anatol.versteht.es/mlem/][mlem].

[`platforms/allwinner-d1/README.md`]: ./platforms/allwinner-d1/README.md
[Melpomene]: ./platforms/melpomene
[Pomelo]: https://github.com/tosc-rs/mnemos/tree/main/platforms/pomelo
[mlem]: https://anatol.versteht.es/mlem/
[`just` recipe]: ./justfile

## Learn More

### Development and API Docs

`rustdoc` output for the current `main` branch can be built locally with `cargo doc --open`.

- [the MnemOS book][book], a high-level discussion of MnemOS' design
- [hosted `main` branch API documentation][rustdoc] for the kernel and other
  MnemOS crates
- [a series of automatically-generated weekly updates][updates], which track
  MnemOS implementation progress over time

[book]: https://mnemos.dev/mnemosprojectoverview/book/
[rustdoc]: https://mnemos.dev/doc/kernel/
[updates]: https://mnemos.dev/mnemosprojectoverview/changelog/

### Development Blogs

We've written a series of development blogs following the story of MnemOS'
implementation. You can find them here:

- [MnemOS Moment 1][moment-1], by James Munns (2023-06-02)
- [MnemOS Moment 2: Search for a Shell][moment-2], by James Munns (2023-07-10)


[moment-1]: https://onevariable.com/blog/mnemos-moment-1/
[moment-2]: https://onevariable.com/blog/mnemos-moment-2/

## Getting Involved

Join us on Matrix: [#mnemos-dev:beeper.com](https://matrix.to/#/#mnemos-dev:beeper.com)

### License

[MIT] + [Apache 2.0].

[MIT]: https://github.com/tosc-rs/mnemos/blob/main/LICENSE-MIT
[Apache 2.0]: https://github.com/tosc-rs/mnemos/blob/main/LICENSE-APACHE

### Code of Conduct

The MnemOS project follows the [Contributor Covenant Code of Conduct](./code_of_conduct.md).
