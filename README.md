# MnemOS

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

## Get Involved

Join us on Matrix: [#mnemos-dev:beeper.com](https://matrix.to/#/#mnemos-dev:beeper.com)

## License

[MIT] + [Apache 2.0].

[MIT]: ./LICENSE-MIT
[Apache 2.0]: ./LICENSE-APACHE
