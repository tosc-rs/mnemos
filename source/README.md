# MnemOS Source

This folder contains the source code of the kernel, userspace, simulator, and related libraries.

Currently, MnemOS is being rewritten as part of the v0.2 version. The current source may not
match the [currently published documentation](https://mnemos.jamesmunns.com)!

## Development status

**As of 2022-07-17**, development is primarily focused on the `kernel`, and the use of the `melpomene` simulator.

Other userspace components, such as the `abi` and `mstd` crates are likely partially or fully out of date.

Focus on userspace will resume after more progress has been made on the kernel.

## Folder Layout

* [`abi/`] - This library contains elements that are stable and shared across the kernel/userspace boundary
* [`alloc/`] - The MnemOS memory allocator.
* [`forth3/`] - This library implements a [Forth] virtual machine for the MnemOS userspace.
* [`kernel/`] - This is the kernel library for MnemOS
* [`melpomene/`] - Melpomene is the simulator for MnemOS development
* [`mstd/`] - This is the userspace "standard library", which wraps mnemos-specific capabilities
* [`notes/`] - Miscellaneous development notes
* [`spitebuf/`] - This is an async, mpsc library which powers the Kernel's `KChannel` data type

[`abi/`]: ./abi/
[`alloc/`]: ./alloc
[`forth3/`]: ./forth3
[`kernel/`]: ./kernel/
[`melpomene/`]: ./melpomene/
[`mstd/`]: ./mstd/
[`notes/`]: ./notes/
[`spitebuf/`]: ./spitebuf

[Forth]: https://forth-standard.org/

## License

[MIT] + [Apache 2.0].

[MIT]: ./../LICENSE-MIT
[Apache 2.0]: ./../LICENSE-APACHE
