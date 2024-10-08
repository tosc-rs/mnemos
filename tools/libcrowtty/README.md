# `libcrowtty`: the library parts of crowtty

[`crowtty`] is a host tool, aimed at speaking the sermux protocol with a simulator
or physical target. It allows for receiving tracing messages, as well as mapping
multiplexed "ports" as TCP sockets on the host. It is the primary way of
connecting to hardware platforms that speak the
sermux-proto over a physical serial port.

This crate contains the generic, library bits of `crowtty`, without the
command-line application, or code for connecting to serial TTY devices (which
requires `libuv` on Linux). It's factored out primarily so that it can be used
by the [`x86_64-bootimager`] tool to connect to QEMU virtual serial devices.

[crowtty]: ../crowtty/
[`x86_64-bootimager`]: ../x86_64-bootimager
