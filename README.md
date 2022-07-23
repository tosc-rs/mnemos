# MnemOS

This repository is for the MnemOS Operating System.

Currently, MnemOS is being rewritten as part of the v0.2 version. The current source may not
match the [currently published documentation](https://mnemos.jamesmunns.com)!

## Development and API Docs

`rustdoc` output for the current `main` branch can be [viewed online here], or built locally with `cargo doc --open`.

[viewed online here]: https://mnemos-dev.jamesmunns.com/kernel/

## Folder Layout

The project layout contains the following folders:

* [`assets/`] - images and files used for READMEs and other documentation
* [`book/`] - This is the source of the [currently published documentation], and is NOT up to date for v0.2.
* [`source/`] - This folder contains the source code of the kernel, userspace, simulator, and related libraries
* [`tools/`] - This folder contains desktop tools used for working with MnemOS

[`assets/`]: ./assets/
[`book/`]: ./book/
[`source/`]: ./source/
[`tools/`]: ./tools/

## License

[MIT] + [Apache 2.0].

[MIT]: ./LICENSE-MIT
[Apache 2.0]: ./LICENSE-APACHE
