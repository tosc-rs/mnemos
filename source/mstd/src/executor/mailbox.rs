// So the goal of the mailbox is basically a Request/Response server,
// with some additional messages sent unsolicited
//
// In an ideal form, it looks like this:
//
// 1. The userspace submits a message to be sent
// 2. Once there is room in the ring, it is serialized
// 3. The userspace waits on the response to come back
// 4. The response is deserialized, and the caller is given the response
// 5. The user reacts to the response
//
// So, we have a finite amount of resources, and there will need to be
// SOME kind of backpressure mechanism somewhere.
//
// This could be:
//
// ## Submission backpressure
//
// * The mailbox gives back a future when the user asks to submit a message
// * The mailbox readies the future when it has room in the "response map"
//   AND there is room in the ring to serialize the message
//     * TODO: How to "wake" the pending slots? Do we do a "jailbreak"
//       wake all? Or just wake the next N items based on available slots?
// * The mailbox exchanges the "send" future with a "receive" future
// * Once the response comes in, the task/future is retrieved from the
//     "response map", and awoken
// * The task "picks up" its message, and frees the space in the response map
//
// Downsides:
//
// A lot of small, slow responses could cause large and/or fast responses to be
// blocked on a pending response slot. Ideally, you could spam messages into
// the outgoing queue immediately (allowing them to be processed), but you'd need
// SOME way to process the response messages, and if we get back a response before
// the request has made it into the "response map", it'll be a problem.

use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};

use abi::{
    bbqueue_ipc::framed::{FrameConsumer, FrameProducer},
    syscall::{
        KernelMsg, KernelResponse, KernelResponseBody, UserRequest, UserRequestBody,
        UserRequestHeader,
    },
};
use maitake::sync::{
    wait_map::{self, WaitMap},
    WaitQueue,
};

pub static MAILBOX: MailBox = MailBox::new();

// TODO: There's a bit of mutexing going on here. `send_wait` and `recv_wait` BOTH have
pub struct MailBox {
    nonce: AtomicU32,
    inhibit_send: AtomicBool,
    send_wait: WaitQueue,
    recv_wait: WaitMap<u32, KernelResponseBody>,
    rings: OnceRings,
}

impl MailBox {
    pub const fn new() -> Self {
        Self {
            nonce: AtomicU32::new(0),
            inhibit_send: AtomicBool::new(false),
            send_wait: WaitQueue::new(),
            recv_wait: WaitMap::new(),
            rings: OnceRings::new(),
        }
    }

    pub fn set_rings(&self, rings: Rings) {
        self.rings.set(rings);
    }

    pub fn poll(&self) {
        let rings = self.rings.get();

        while let Some(msg) = rings.k2u.read() {
            match postcard::from_bytes::<KernelMsg>(&msg) {
                Ok(KernelMsg::Response(KernelResponse { header, body })) => {
                    // Attempt to wake a relevant waiting task, OR drop the response
                    self.recv_wait.wake(&header.nonce, body);
                }
                Ok(_) => todo!(),
                Err(_) => {
                    // todo: print something? Relax this panic later with a graceful
                    // warning
                    panic!("Decoded bad message from kernel?");
                }
            }

            msg.release();
        }

        if self.inhibit_send.load(Ordering::Acquire) && rings.u2k.grant(128).is_ok() {
            self.inhibit_send.store(false, Ordering::Release);
            self.send_wait.wake_all();
        }
    }

    async fn send_inner(&'static self, nonce: u32, msg: UserRequestBody) -> Result<(), ()> {
        let rings = self.rings.get();
        let outgoing = UserRequest {
            header: UserRequestHeader { nonce },
            body: msg,
        };

        // Wait for a successful send
        loop {
            if !self.inhibit_send.load(Ordering::Acquire) {
                // TODO: Max Size
                if let Ok(mut wgr) = rings.u2k.grant(128) {
                    let used = postcard::to_slice(&outgoing, &mut wgr).map_err(drop)?.len();
                    wgr.commit(used);
                    break;
                } else {
                    // Inhibit further sending until there is room, in order to prevent
                    // starving waiters
                    self.inhibit_send.store(true, Ordering::Release);
                }
            }
            self.send_wait.wait().await.map_err(drop)?;
        }

        Ok(())
    }

    /// Send a message to the kernel without waiting for a response
    pub async fn send(&'static self, msg: UserRequestBody) -> Result<(), ()> {
        let nonce = self.nonce.fetch_add(1, Ordering::AcqRel);
        self.send_inner(nonce, msg).await
    }

    /// Send a message to the kernel, waiting for a response
    pub async fn request(&'static self, msg: UserRequestBody) -> Result<KernelResponseBody, ()> {
        let nonce = self.nonce.fetch_add(1, Ordering::AcqRel);

        // Start listening for the response BEFORE we send the request
        let mut rx = core::pin::pin!(MAILBOX.recv_wait.wait(nonce));
        rx.as_mut().enqueue().await.map_err(drop)?;
        self.send_inner(nonce, msg).await?;

        rx.await.map_err(drop)
    }
}

unsafe impl Sync for OnceRings {}

struct OnceRings {
    set: AtomicBool,
    queues: UnsafeCell<MaybeUninit<Rings>>,
}

impl OnceRings {
    const fn new() -> Self {
        Self {
            set: AtomicBool::new(false),
            queues: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    fn set(&self, rings: Rings) {
        unsafe {
            self.queues.get().cast::<Rings>().write(rings);
            let old = self.set.swap(true, Ordering::SeqCst);
            assert!(!old);
        }
    }

    fn get(&self) -> &Rings {
        assert!(self.set.load(Ordering::Relaxed));
        unsafe { &*self.queues.get().cast::<Rings>() }
    }
}

pub struct Rings {
    pub u2k: FrameProducer<'static>,
    pub k2u: FrameConsumer<'static>,
}

// impl Ma
