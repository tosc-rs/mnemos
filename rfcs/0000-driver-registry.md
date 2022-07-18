# RFC - Drivers and Discovery

## Goal

MnemOS should have some way of "automatically discovering" drivers. This would be used by three main "consumers":

* The kernel itself, which may need to use drivers that are provided at initialization time
* Drivers, which may depend on other drivers (such as a virtual serial mux driver, which depends on a "physical serial port" driver)
* Userspace, which will typically interact with drivers through either the stdlib, or other driver crates.

## Background

Right now, when spawning drivers, the process goes roughly as follows:

* The `init()` code manually constructs drivers, and manually creates any shared resources necessary between drivers
* The `init()` code spawns the driver task(s) onto the kernel executor

This is a little manual, and also won't really work for userspace.

There are also some half baked ideas in the `abi`/`mstd` crates about having a single "message kind" that is an enum of all possible driver types, but this isn't very extensible

## The proposal

This change proposes inntroducing a **registry** of drivers, maintained by the kernel.

* This **registry** would be based on a Key:Value type arrangement:
    * The **Key** would be a tuple of:
        * A pre-defined **UUID**, that uniquely defines a "service" that the driver fulfills, e.g. the ability to process a given command type
        * The `TypeId` of that message
    * The **Value** would be contain two things:
        * a `KProducer<T>`, where `type_id::<T>()` matches the `TypeId` in the key
        * Optionally, if the driver should process messages from userspace, it should provide a function with the signature `fn(&[u8]) -> Result<T>`, where `type_id::<T>()` matches the `TypeId` in the key
* Drivers would be responsible for registering themselves at `init()` time
* All `(Uuid, TypeId)` keys must be unique.
* The registry would have the following APIs:
    * `kernel.registry().set::<T>(&Uuid, &KProducer<T>) -> Result<()>`
    * `kernel.registry().get::<T>(&Uuid) -> Option<KProducer<T>>`
        * This should have a blocking version (that returns an Option)
        * And also an async version (that just returns a KProducer)

## Details

Some details about why parts of this proposal was chosen:

### Using `Uuid`s to identify drivers

There are a lot of different ways to discover drivers used in other operating systems, including using some kind of path, or a type erased file interface.

This proposal takes an approach somewhat similar to Bluetooth's characteristics:

* A UUID is chosen ahead of time to uniquely identify a service
* Some "official" UUIDs are reserved for MnemOS and built-in drivers
* A UUID represents a single service interface
    * Although any driver could implement the UUID, all implementors are expected to have the same interface
    * The UUID has no concept of versioning. If a breaking change is needed, a new UUID should be used

In practice, any unique/opaque identifier could be used, but a UUID is used for familiarity.

When implementing a driver, the driver would be expected to export its UUID as part of the crate. It's likely the MnemOS project would maintain some registry of "known UUIDs", to attempt to avoid collisions. This could be in a text file, or wiki, etc.

### Using `TypeId`

Although all drivers are accessed by a `KChannel<T>`, which is a heap allocated structure, attempting to hold many channels of different `T`s in the same structure wouldn't work.

Instead, the key:value map would need to type-erase the channel, likely just holding some kind of pointer. Theoretically, the UUID *should* be enough to ensure that the correct `T` matches the expected type, however the cost of getting this wrong (undefined behavior) due to a developer mistake is too high.

In order to avoid this, the `get` function ALSO takes a `TypeId`, which is a unique type identifier provided by the compiler. Since all drivers are statically compiled together, they should all see the same `T` have the same `TypeId`.

This WON'T necessarily work for userspace, or if MnemOS ever gets dynamically loaded drivers, which may be compiled with a different version of Rust, or potentially not Rust at all.

For this reason, non-static users of these drivers will need to use the serialization interface instead. Userspace would send a serialized message to the expected UUID. The kernel would then use the associated `fn(&[u8]) -> Result<T>` to deserialize the message, and send it to the associated channel

### Uniqueness of drivers

This scheme requires that only one version of any kind of driver can be registered.

This simplifies things, but could be potentially annoying for certain drivers are expected to have multiple instances, like keyboards, or storage devices.

This proposal sort of ignores that problem, and suggests that those drivers shouldn't be discoverable. Instead, some kind of "higher level" driver that hands out instances of those drivers should be registered.

### (Exclusively) using `KChannel<T>`

Although a lot of discussion of using "message passing" as a primary interface has been had so far, this would pretty much cement this design choice.

Although there are other communication primitives available ready, such as the various flavors of the `bbq` based queues, they could be registered and provided through the KChannel as a "side channel". This is how the current (as of 2022-07-18) serial mux interface works:

* The client who wants to open a virtual port gets the `KProducer`
* The client sends a "register port" message
* On success, the client gets back a `bbq` channel handle from the driver

This also ONLY provides a "one way" interface, however it is possible to define a KChannel message type that has a "reply address" by providing a `KProducer<Result<U>>` handle in the message itself. This is relatively low cost, because KChannel/KProducers are reference counted and heap allocated types.

# Wait, how do you respond to userspace?

If the driver takes:

```rust
struct Message {
    inner: Inner,
    reply: KProducer<Result<Response>>,
}
```

How does the reply channel "get" the response?

Really, there needs to be some kind of task that does user <-> kernelspace translation.

I need some kind of official reply mechanism.

Something like:

```rust
// This is opaque to the driver
enum ReplyTo<U> {
    Kernel(KChannel<Result<U>>),
    Userspace {
        nonce: u32,
        ring_producer: MpscProducer,
    }
}

impl<U> ReplyTo<U>
where
    U: Serialize,
{
    // Automatically handles the reply
    async fn reply(self, reply: Result<U>);
}

impl<U> ReplyTo<U> {
    // Replies to the kernel, sends a "not supported"
    // error back to userspace
    async fn reply_konly(self, reply: Result<U>);
}
```

But this isn't great, because it allows the driver to *receive* messages and process them, but not *reply*.

How can I enforce that? Have two registries?

