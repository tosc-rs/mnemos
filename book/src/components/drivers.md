Another day, another chunk of writing.

## Parts of mnemOS - continued

In the [last post](https://cohost.org/jamesmunns/post/72485-thoughts-about-mnem-o), I covered the **kernel**. This post moves on to the **drivers**, which are async tasks that are responsible for all other hardware and system related functionality.

### the drivers

---

As a micro-kernel-ish operating system, most functionality is provided by drivers. Some of the "how" is still being finalized, but this is a look at current/planned capabilities.

#### drivers as a service

In mnemOS' driver model, drivers are expected to act as services, in a sort of REST or RPC sort of way (drawing parallels to web or microservice style techniques). They take a specific request type (as a Rust data type), and provide a specific response type (also as a Rust data type). For example, a driver that hands out virtual serial ports might have a request type that looks like this:

```rust
pub enum Request {
    /// Register a virtual serial port with a given port ID and
    /// buffer capacity
    RegisterPort { port_id: u16, capacity: usize },
    /* other request types not shown... */
}
```

and a matching response type that looks like this:

```rust
pub enum Response {
    /// A port has been successfully registered
    PortRegistered(PortHandle),
    /* other response types not shown... */
}

/// A PortHandle is the interface received after
/// opening a virtual serial port
pub struct PortHandle {
    port: u16,
    cons: bbq::Consumer,
    outgoing: bbq::MpscProducer,
    max_frame: usize,
}
```

This message passing is done in an async way, so if the message queues are ever full, the sender can await until there is capacity available, and the receiver is transparently notified (and scheduled to run).

Each service is identified using a UUID, such as `54c983fa-736f-4223-b90d-c4360a308647`, for the virtual serial port service. This UUID can be registered by any driver implementer that uses the same request and response types, which means that different platform-specific drivers can implement the same interface, when necessary or preferable. This allows drivers services to be "generic" over their implementation, without having complicated type relationships.

This use of UUID borrows heavily from [how Bluetooth Low Energy works](https://www.bluetooth.com/blog/a-developers-guide-to-bluetooth/), with a UUID identifying a Characteristic, or a specific kind of API.

These request and response types and UUID value are specified through a trait called `RegisteredDriver`, which is explained in the [driver registry RFC](https://github.com/tosc-rs/mnemos/blob/main/rfcs/0025-driver-registry.md#the-registereddriver-trait). The RFC goes into a great bit more detail of how this "type safe service discovery" mechanism actually works under the hood.

#### two kinds of drivers

In practice, this means that there will end up being two main kinds of drivers:

* platform specific drivers
* portable drivers

**Platform specific drivers** are drivers that are expected to only work on a specific device, or family of devices. Although many microcontrollers and microprocessors have "Serial", "UART", or "USART" ports, the code necessary to configure them, and have them efficiently send and receive bytes, varies incredibly widely. However as we've seen with the [embedded-hal traits](https://docs.rs/embedded-hal/latest/embedded_hal/) in bare-metal Rust, it is often very possible to have a portable interface that covers MOST common use cases.

Even though these platform specific drivers are implemented in very different ways, they would be expected to use a common interface at a high level. This consistent lower interface allows for portable drivers to work regardless of the underlying **platform specific drivers** in use.

In contrast, **portable drivers** ONLY rely on other driver services, meaning that as long as the services they depend on exist, they will be able to operate regardless of the actual system they are running on. For example, we might have a couple of high level driver services, like:

* logging and tracing info
* a command line interface
* a system status display

These driver services would ONLY rely on the virtual serial port interface, which provides multiple "ports" over a single serial port. In turn, the virtual serial port interface relies on the platform-specific hardware serial port interface.

By implementing ONLY a platform specific driver for a serial port, someone porting mnemOS to a new platform would gain access to use all four of these services (virtual serial port, logging/tracing, CLI, and system status) automatically.

#### exposing drivers to userspace

> note: this is an area that is still under construction. some parts as described already exist in the code, but some do not yet.

In the "kernelspace", where the kernel and drivers exist, we can leverage compile time type safety, because all drivers and the kernel will be compiled together into one binary (at the moment mnemOS does not support "dynamically loading" drivers, they must be statically compiled together).

This is an important distinction because Rust does NOT have a stable ABI, and types and layout can change at any time, even between compilations. In "userspace", where user applications execute, we will not have compiled the applications at the same time as the kernel. They are two completely separate binaries!

To get around this, we can still use an async message-passing interface, but the requests and response will be serialized and then deserialized. Using [serde](https://serde.rs) and the [postcard wire format](https://postcard.jamesmunns.com/wire-format.html), we can be sure that data will be consistently interpreted.

Driver services with request/response types that can be serialized can also make their interfaces available to user applications, though this is not required. The userspace can ask if a certain driver service UUID is registered (and available to userspace), and if it is, it can send serialized messages to the kernel, to be forwarded to the drivers. A full round trip looks something like this:

* The userspace prepares a request, and then serializes it
* The userspace sends the serialized request to the kernel
* The kernel determines which service is being messaged, and if it exists, the message is deserialized and sent to the driver
* The driver processes the request, and sends a response to the kernel to be returned to userspace
* The kernel serializes the response, and sends it to userspace
* The userspace deserializes the response, and processes it

How this actual userspace to kernel messaging works will be covered later, when I talk about the userspace itself works.
