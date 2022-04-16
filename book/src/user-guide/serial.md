# Connecting to Virtual Serial Ports

MnemOS has a concept of **Virtual Serial Ports**. These are used to provide different services (known as "multiplexing") over a single "hardware" serial port. You can think of these sort of like TCP or UDP ports, but for a serial port.

By convention, Virtual Port 0 acts like `stdio` on a desktop PC, and typically provides a human-readable terminal interface. Other ports may be human-readable, or a specific binary format, defined by the application.

At the moment, MnemOS only supports talking over the USB port of the nRF52840 Feather.

You generally shouldn't open the serial port with your operating system directly (using tools like `screen`, `minicom`, or `putty`), as the MnemOS system won't "understand" your messages. The format used on the wire is [described below](#wire-format).

## Using the CrowTTY tool

For convenience, MnemOS provides a tool called `crowtty` that maps Virtual Serial Ports to TCP ports on the local system.
This allows you to connect to individual Virtual Serial Ports separately.

At the moment, the CrowTTY tool only supports the following ports:

* Virtual Port 0: Mapped to TCP IP Address **127.0.0.1** and TCP Port **10000**
* Virtual Port 1: Mapped to TCP IP Address **127.0.0.1** and TCP Port **10001**

You can run the CrowTTY tool with the following command:

```sh
cd tools/crowtty
cargo run --release
    Finished release [optimized] target(s) in 0.90s
     Running `target/release/crowtty`
```

You will need to leave this running in the background to act as the TCP server.

In another window, you can connect to the mapped TCP port. In the following example, `stty` is also used to disable local echo of characters (which would duplicate the output of the MnemOS hardware), as well as disable line buffering. `ncat` is used to open a persistent TCP connection to the port, acting similar to a "console" for our MnemOS device.

There may not initially be a prompt on the target device. You may need to hit 'ENTER' once to see output. The following is the output if the `app-loader` program, which is the default user program loaded by the operating system.

```sh
stty -icanon -echo && ncat 127.0.0.1 10000

>
Input was empty?

> help
AVAILABLE ITEMS:
  info
  block <idx>
  upload <idx>
  boot <idx>
  ustat
  ucomplete <kind> <name>
  help [ <command> ]

>
```

For more information on how to use the `app-loader` program, see the [Uploading and Running Programs](./upload-and-run.md) section.

## Wire Format

> # ⚠️ WARNING - unstable!
>
> It is not generally recommended (for now) to directly communicate to the MnemOS device over the serial port
> (for example, if you are writing your own tools), as the format described above may change at any time
> in breaking kernel updates.
>
> Instead, it is recommended to have your tool connect to a TCP port via the CrowTTY tool, which will be
> updated with whatever changes are made between kernel versions.
>
> The protocol is described below for educational reasons.

Messages over the serial port transported using Postcard as a protocol, and COBS as framing.

This means if you want to send the following four data bytes to virtual port 1:

```
[ 0x00 ][ 0x01 ][ 0x02 ][ 0x03 ]
```

We will need to do two things:

1. Append the virtual port to the FRONT of the message, in a little-endian order
2. COBS encode the message, to provide message framing

After appending the Port ID, the message will look like this:

```
[ 0x01 ][ 0x00 ][ 0x00 ][ 0x01 ][ 0x02 ][ 0x03 ]
  ^^^^^^^^^^^^    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^
           |                               |
           |                               `----------------->  Data Bytes
           |
           `------------------------------------------------->  Port (u16, LE)
```

The message will then be COBS encoded. COBS encoding replaces all of the `0x00` bytes of the message
with the number of bytes to the next 0x00 in the message. It places one byte at the front that has
the number of bytes to the first (replaced) 0x00 byte. It also places a "real" `0x00` byte at the
end of the message, so the receiver knows it has reached the end of a single frame.

The message actually sent over the serial port like this:

```
   .---------------.-------.-------------------------------.
   |               |       |                               |    COBS Linked List
   |               v       v                               v
[ 0x02 ][ 0x01 ][ 0x01 ][ 0x04 ][ 0x01 ][ 0x02 ][ 0x03 ][ 0x00 ]
  ^^^^    ^^^^^^^^^^^^    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^    ^^^^
   |               |                               |       |
   |               |                               |       `->  End of COBS frame
   |               |                               |
   |               |                               `--------->  Data Bytes
   |               |
   |               `----------------------------------------->  Port (u16, LE)
   |
   `--------------------------------------------------------->  COBS header
```

