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
        * A channel that can be used for sending requests TO a driver
        * If the driver supports serializable/deserializable types, it will also contain a function that can be used to deserialize and send messages via the driver channel.
* Drivers would be responsible for registering themselves at `init()` time
* All `(Uuid, TypeId)` keys must be unique.

> **NOTE**: Large portions of this proposal were prototyped in the [`typed-poc`]
> repository. Some details were approximated, as this was developed with the
> standard library instead of MnemOS types for ease.

[`typed-poc`]: https://github.com/jamesmunns/typeid-poc

## Details

Some details about why parts of this proposal was chosen:

### The Message Type

All drivers will use a standardized message type with two generic parameters, `T` and `U`. `T` is the type of the **request** to the driver, while `U` is the type of the **response** from the driver.

```rust
struct Message<T, U> {
    msg: T,
    reply: ReplyTo<U>,
}
```

This message type contains a `ReplyTo<U>` field, which can be used to reply to the sender of the **request**. This serves as the "return address" of a request. This enum will look roughly as follows:

```rust
enum ReplyTo<U> {
    // This can be used to reply directly to another kernel entity,
    // without a serialization step
    Kernel(KProducer<U>),

    // This can be used to reply to userspace. Responses are serialized
    // and sent over the bbq::MpscProducer
    Userspace {
        nonce: u32,
        outgoing: bbq::MpscProducer,
    },
}
```

The driver is responsible for definining the `T` and `U` request/response types associated with a given UUID.

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

For this reason, non-static users of these drivers will need to use the serialization interface instead. Userspace would send a serialized message to the expected UUID. The kernel would then use the associated deserialization function to deserialize the message, and send it to the associated channel

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

### Interfaces: Kernel Only

If a type used in the channel does NOT support serialization/deserialization, it is limited to only being usable in the kernel, where this step is not required.

The registry would provide two main functions in this case:

```rust
impl Registry {
    // Register a given KProducer for a (uuid, T, U) pair
    fn set_konly<T, U>(
        &mut self,
        uuid: Uuid,
        kch: &KProducer<Message<T, U>>,
    ) -> Result<(), ()> { /* ... */ }

    // Obtain a given KProducer for a (uuid, T, U) pair
    fn get_konly<T, U>(
        &self,
        uuid: Uuid,
    ) -> Option<KProducer<Message<T, U>>> { /* ... */ }
}
```

This code would be used as such:

```rust
struct SerialPortReq {
    // ...
}

struct SerialPortResp {
    // ...
}

// We have a given "SerialPort" driver
impl SerialPort {
    type Request = SerialPortReq;
    type Response = Result<SerialPortResp, ()>;
    type Handle = KProducer<Message<Self::Request, Self::Response>>;
    const UUID: Uuid = ...;
}

let serial_port = SerialPort::new();

// In `init()`, the SerialPort is registered
registry.set_konly::<SerialPort::Request, SerialPort::Response>(
    SerialPort::UUID,
    serial_port.producer(),
).unwrap();

// Later, in another driver, we attempt to retrieve this driver's handle
struct SerialUser {
    hdl: SerialPort::Handle,

    // These are used for the "reply address"
    resp_prod: KProducer<SerialPort::Response>,
    resp_cons: KConsumer<SerialPort::Response>,
}

let user = SerialUser {
    hdl: registry.get_konly(SerialPort::UUID).unwrap(),
};

// Send a message to the driver:
user.hdl.enqueue_async(Message {
    msg: SerialPortReq::get_port(0),
    reply_to: ReplyTo::Kernel(user.resp_prod.clone()),
}).await.unwrap();

// Then get a reply:
let resp = user.resp_cons.dequeue_async().await.unwrap();
```

## Interface: Userspace

If a given type DOES implement Serialize/Deserialize, it can also be used for communication with userspace.

The registry provides two additional functions in this case:

```rust
// This is a type that is capable of deserializing and processing
// messages for a given UUID type
impl UserspaceHandler {
    fn process_msg(
        &self,
        user_msg: UserMessage<'_>,
        user_ring: &bbq::MpscProducer,
    ) -> Result<(), ()> { /* ... */ }
}

impl Registry {
    // Register a given KProducer for a (uuid, T, U) pair
    fn set<T, U>(
        &mut self,
        uuid: Uuid,
        kch: &KProducer<Message<T, U>>,
    ) -> Result<(), ()>
    where
        T: Serialize + DeserializeOwned,
        U: Serialize + DeserializeOwned,
    {
        /* ... */
    }

    // Obtain a userspace handler for the given UUID
    fn get_userspace_handler(
        &self,
        uuid: Uuid,
    ) -> Option<UserspaceHandler>;
}
```

The `get_userspace_handler()` function does NOT take the `T` and `U` types as parameters. If the registry entry for the given `uuid` was added through the `set_konly` instead of the `set` interface, the `get_userspace_handler()` function will never return a handler for that `uuid`.

This code would be used as follows:

```rust
#[derive(Serialize, Deserialize)] // Added!
struct SerialPortReq {
    // ...
}

#[derive(Serialize, Deserialize)] // Added!
struct SerialPortResp {
    // ...
}

// We have a given "SerialPort" driver
impl SerialPort {
    type Request = SerialPortReq;
    type Response = Result<SerialPortResp, ()>;
    type Handle = KProducer<Message<Self::Request, Self::Response>>;
    const UUID: Uuid = ...;
}

let serial_port = SerialPort::new();

// In `init()`, the SerialPort is registered, this time with `set` instead
// of `set_only`.
registry.set::<SerialPort::Request, SerialPort::Response>(
    SerialPort::UUID,
    &serial_port.producer(),
).unwrap();

// This is generally the shape of user request messages "on the wire" between
// userspace and kernel space.
struct UserMessage<'a> {
    uuid: Uuid,
    nonce: u32,
    payload: &'a [u8],
}

// Lets say that we have the User Ring worker
impl UserRing {
    async fn get_message<'a>(&'a self) -> UserMessage<'a> { /* ... */ }
    fn producer(&self) -> &bbq::MpscProducer { /* ... */ }
}

let user_ring = UserRing::new();

// okay, first we get a message off the wire
let ser_request = user_ring.get_message().await;

// Now we get the handler for the given uuid on the wire:
let user_handler = registry.get_userspace_handler(ser_request.uuid).unwrap();

// Now we use that handler to process the message:
user_handler.process_msg(ser_request, user_ring.producer()).unwrap();

// Once the user handler processes the message, it will send the serialized
// response directly into the userspace buffer.
```

### Implementing the `UserspaceHandler`

There is one main trick used to generate the `UserspaceHandler` described above is that the `set()` function is used to generate a monomorphized free function that can handle the deserialization.

The contents of the `UserspaceHandler` will look roughly like this:

```rust
// This is the "guts" of a leaked `KProducer`. It has been type erased
//   from: MpMcQueue<Message< T,  U>, sealed::SpiteData<Message< T,  U>>>
//   into: MpMcQueue<Message<(), ()>, sealed::SpiteData<Message<(), ()>>>
type ErasedKProducer = *const MpMcQueue<Message<(), ()>, sealed::SpiteData<Message<(), ()>>>;
type TypedKProducer<T, U> = *const MpMcQueue<Message<T, U>, sealed::SpiteData<Message<T, U>>>;

struct UserspaceHandler {
    req_producer_leaked: ErasedKProducer
    req_deser: unsafe fn(
        UserMessage<'_>,
        ErasedKProducer,
        &bbq::MpscProducer,
    ) -> Result<(), ()>,
}
```

The `req_deser` function will look something like this:

```rust
type TypedKProducer<T, U> = *const MpMcQueue<Message<T, U>, sealed::SpiteData<Message<T, U>>>;

unsafe fn map_deser<T, U>(
    umsg: UserMessage<'_>,
    req_tx: ErasedKProducer,
    user_resp: &bbq::MpscProducer,
) -> Result<(), ()>
where
    T: Serialize + DeserializeOwned + 'static,
    U: Serialize + DeserializeOwned + 'static,
{
    // Un-type-erase the producer channel
    let req_tx = req_tx.cast::<TypedKProducer<T, U>>();

    // Deserialize the request, if it doesn't have the right contents, deserialization will fail.
    let u_payload: T = postcard::from_bytes(umsg.req_bytes).map_err(drop)?;

    // Create the message type to be sent on the channel
    let msg: Message<T, U> = Message {
        msg: u_payload,
        reply: ReplyTo::Userspace {
            nonce: umsg.nonce,
            outgoing: user_resp.clone(),
        },
    };

    // Send the message, and report any failures
    (*req_tx).enqueue_sync(msg).map_err(drop)
}
```

The fun trick here is that while `map_deser` is generic, once we turbofish it, it is no longer generic, because we've specified types! Using psuedo syntax:

```rust
// without the turbofish, `map_deser` as a function has the type (not real syntax):
let _: fn<T, U>(UserMessage<'_>, ErasedKProducer, &bbq::MpscProducer) -> Result<(), ()> = map_deser;

// WITH the turbofish, `map_deser` as a function has the type:
let _: fn(UserMessage<'_>, ErasedKProducer, &bbq::MpscProducer) -> Result<(), ()> = map_deser::<T, U>;
```

This means we now have a type erased function handler, that still knows (internally) what types it should use! That means we can put a bunch of them
into the same structure.

With this, we can now implement the `process_msg()` function shown above:

```rust
impl UserspaceHandler {
    fn process_msg(
        &self,
        user_msg: UserMessage<'_>,
        user_ring: &bbq::MpscProducer,
    ) -> Result<(), ()> {
        unsafe {
            self.req_deser(user_msg, self.req_producer_leaked, user_ring)
        }
    }
}
```
