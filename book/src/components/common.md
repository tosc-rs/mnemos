# MnemOS Common Library

At the moment, the `common` crate primarily defines the message protocol used by system calls, and helper functions necessary for handling system calls in a convenient, idiomatic Rust way.

Message defintions live in [`src/syscall/mod.rs`](https://github.com/jamesmunns/pellegrino/blob/main/firmware/common/src/syscall/mod.rs), and helper functions live in [`src/porcelain/mod.rs`](https://github.com/jamesmunns/pellegrino/blob/main/firmware/common/src/porcelain/mod.rs).

For more information, refer to the [Common Library API documentation].

[Common Library API documentation]: https://docs.rs/mnemos-common/latest/mnemos_common/
