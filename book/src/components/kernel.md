# Kernel Binary

## Theory

The kernel is still in early and active development, however here are a couple of design choices that have been made:

### User Isolation

Currently, the kernel runs with its own stack, using the Cortex-M MSP (Main Stack Pointer). Additionally, it runs in priviledged mode, while userspace runs in unpriviledged mode. The userspace has its own separate stack, using the Cortex-M PSP (Process Stack Pointer).

Support for Cortex-M Memory Protection Units (MPUs) for additional isolaton is planned, but not currently implemented.

As the userspace is in unpriviledged, it is limited to interacting with kernel through the `SVCall` interrupt, which triggers a system call.

### System Calls

In order to interact with the kernel, the userspace application makes system calls, which trigger an interrupt which the kernel responds to.

Before making a system call, the userspace prepares two things:

* An "input slice", a pointer and length pair, which can together be considered as a Rust `&[u8]`. The contents of this slice is the requested system call action.
* An "output slice", a pointer and length pair, which can together be considered as a Rust `&mut [u8]`. Initially this contains nothing, and the length represents the maximum output contents. The kernel will fill the contents of this slice with the result of the requested system call, and the length of the output slice will be reduced to the used output area.

As Rust does not have a stable ABI, MnemOS instead relies on serialized data. MnemOS uses the [`postcard`] crate (built on top of Serde) to define the message format for system calls.

Put together, the process of making a system call is generally:

1. The userspace prepares the request, and serializes it to the input slice
2. The userspace prepares a destination buffer, to be used as the output slice
3. The userspace triggers an SVCall interrupt
4. The kernel receives the SVCall interrupt
5. The kernel deserializes the input slice, and performs the requested action
6. The kernel prepares a response, and serializes it to the output slice
7. The kernel returns control to userspace
8. The userspace deserializes the contents of the output slice

More information on the details of the system call protocol can be found in the [`common` chapter] of this book.

[`postcard`]: https://docs.rs/postcard/latest/postcard/
[Serde]: https://serde.rs/
[`common` chapter]: https://github.com/jamesmunns/pellegrino/blob/main/firmware/common/README.md

### Program Loading

At the moment, user applications are loaded to RAM, and executed from RAM. Applications declare their stack size, which is prepared by the operating system.
