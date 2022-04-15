# The MnemOS App Loader

The app-loader is the default program loaded by the kernel.

It serves a couple purposes:

* It allows you to query and view the contents of the "block device", typically a QSPI flash, attached to the main CPU
* It allows you to upload new programs and data files into blocks of the block storage device, by providing an file upload client over virtual port 1
* It allows you to select and boot a program

The app-loader acts as a CLI/REPL on virtual port 0, the stdio port.

## Commands

The following commands are provided by the app-loader:

* `info` - shows information about the block device, such as the number of blocks available, and the size of each block
* `block N` - shows the contents and status of a given block. `N` is a number from zero to the number of available blocks.
* `upload N` - Start an upload client for a given block.
* `ustat` - Show the current status of the upload client.
* `ucomplete KIND NAME` - Once an upload is completed, the block file table is updated with this command.
    * `KIND` can be either `program` or `storage`.
    * `NAME` can be any UTF-8 string <= 128 bytes in length.

## Uploading

At the moment, the only tool provided to act as an "upload server" to pair with the "upload client" provided by the app loader is the [`dumbloader` tool](../../../tools/dumbloader/README.md). See that readme for more information on how to use it.

A video of this process can be [seen here on twitter](https://twitter.com/bitshiftmask/status/1513318406065381387).
