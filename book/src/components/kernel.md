# the kernel

The kernel is fairly limited, both at the moment, and in general by design. It provides a couple of specific abilities:

## the allocator

The kernel provides an allocator intended for use by drivers. Rather than RTOS or other more "embedded style" projects, dynamic allocation can be used to spawn driver tasks, and allocate resources, like how large buffers are, how many concurrent requests can be made at once, etc. mnemOS [does NOT use the standard Rust allocator API](https://cohost.org/jamesmunns/post/69963-async-allocations-m), and provides its own.

Although an allocator is available, it is not intended to be used in the "normal case", such as sending or receiving messages. Instead, buffers should be allocated at setup or configuration time, to reduce allocator impact. For example: setting up the TCP stack, or opening a new port might incur an allocation, but sending or receiving a TCP frame should not. This is not currently enforced, and is a soft goal for limiting memory usage and fragmentation.

## the executor/scheduler

As initial versions of mnemOS are intended to run on single core devices, and hard-realtime is not a specific goal, the kernel provides no concepts of threading for concurrency. Instead, all driver tasks are expected to run as cooperative async/await tasks. This allows for efficient use of CPU time in the kernel, while allowing events like hardware interrupts to serve as simple wakers for async tasks.

The executor is based largely on the [maitake](https://mycelium.elizas.website/maitake/) library. Maitake is a collection of no-std compatible executor building blocks, which are used extensively. This executor serve the purpose of scheduling all driver behaviors, as well as kernel-time scheduling of user applications.

## the driver registry

In order to dynamically discover what drivers are running, the kernel [provides a driver registry](https://github.com/tosc-rs/mnemos/blob/main/rfcs/0025-driver-registry.md), which uses UUIDs to uniquely identify a "driver service".

By default, drivers are expected to operate in a message-oriented, request-reply fashion. This means to interact with a driver, you send it messages of a certain type (defined by the UUID), and it will send you a response of a certain type (defined by the UUID). This message passing is all async/await, and is type-safe, meaning it is not necessary to do text or binary parsing.

Additionally, drivers may choose whether they also make their services available to user programs as well. This interface will be explained later, when discussing user space programs.
