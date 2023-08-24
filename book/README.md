# MnemOS Book

This folder contains long-form documentation for the MnemOS operating system
generated using [mdBook].

See <https://mnemos.dev/mnemosprojectoverview/book/> for a rendered version of the
MnemOS book.

## Contributing to the Book

For details on the mdBook documentation format, see the ["Format"
section of the mdBook documentation][mdbook-format].

## Building the Book

When building the published documentation on <https://mnemos.dev>, the mdBook is
built using [Oranda], rather than the mdBook command-line tool. Therefore,
[Oranda] is the preferred way to build the documentation.

Unlike other mdBook projects, this directory does not contain a SUMMARY.md
file. This is because the SUMMARY.md file is automatically generated, in order
to append the [mnemOS RFCs](../rfcs/) to the mdBook. This is performed
by the [`rfc2book.py`][rfc2book] script in [`scripts/`](../scripts/).

> [!IMPORTANT]
>
> Therefore, **[`scripts/rfc2book.py`][rfc2book] must be run**
> before running any [mdBook] or [Oranda] commands to build the book.

### Just Recipes

The `just oranda` [Just recipe] will automatically run [`rfc2book.py]` prior
to running an [Oranda] command. This is the recommended way to build the book
locally.

Similarly, mdBook CLI commands can also be run using the `just mdbook` [Just
recipe], which will run [`rfc2book.py`][rfc2book] beforehand. Both of these
commands will also offer to install [mdBook] and [Oranda] if they are not
already present on the system.

[mdBook]: https://rust-lang.github.io/mdBook/
[mdbook-format]: https://rust-lang.github.io/mdBook/format/index.html
[Oranda]: https://opensource.axo.dev/oranda/
[rfc2book]: ../scripts/rfc2book.py
[Just recipe]: ../justfile
