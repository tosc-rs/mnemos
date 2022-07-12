//! Wrappers around the IPC bbqueue
//!
//! These types are intended to be used ONLY in the kernel, where we
//! can expect a "single executor" async operation. At some point, this
//! may inform later design around user-to-kernel bbqueue communication.

use core::{
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
};

use crate::fmt;
use abi::bbqueue_ipc::{BBBuffer, Consumer, Producer};
use maitake::wait::WaitCell;
use mnemos_alloc::{containers::{HeapArc, HeapArray}, heap::AHeap};
use tracing::{info, trace};
use abi::bbqueue_ipc::{GrantR as InnerGrantR, GrantW as InnerGrantW};

struct BBQStorage {
    commit_waitcell: WaitCell,
    release_waitcell: WaitCell,
    ring: BBBuffer,
    _array: HeapArray<MaybeUninit<u8>>,
}

pub struct SpscProducer {
    storage: HeapArc<BBQStorage>,
    producer: Producer<'static>,
}

pub struct SpscConsumer {
    storage: HeapArc<BBQStorage>,
    consumer: Consumer<'static>,
}

pub async fn new_spsc_channel(
    alloc: &'static AHeap,
    capacity: usize,
) -> (SpscProducer, SpscConsumer) {
    info!(
        capacity,
        "Creating new mpsc BBQueue channel"
    );
    let mut _array = alloc
        .allocate_array_with(MaybeUninit::<u8>::uninit, capacity)
        .await;

    let ring = BBBuffer::new();

    unsafe {
        ring.initialize(_array.as_mut_ptr().cast(), capacity);
    }

    let storage = alloc
        .allocate_arc(BBQStorage {
            commit_waitcell: WaitCell::new(),
            release_waitcell: WaitCell::new(),
            ring,
            _array,
        })
        .await;

    // Now that we've allocated storage, the producer can be created.

    let bbbuffer = &storage.ring as *const BBBuffer as *mut BBBuffer;

    let (prod, cons) = unsafe {
        let prod = BBBuffer::take_producer(bbbuffer);
        let cons = BBBuffer::take_consumer(bbbuffer);

        (prod, cons)
    };

    let prod = SpscProducer {
        storage: storage.clone(),
        producer: prod,
    };
    let cons = SpscConsumer {
        storage,
        consumer: cons,
    };

    info!("Channel created successfully");

    (prod, cons)
}


pub struct GrantW {
    grant: InnerGrantW<'static>,
    storage: HeapArc<BBQStorage>,
}

impl Deref for GrantW {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.grant.deref()
    }
}

impl DerefMut for GrantW {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.grant.deref_mut()
    }
}

impl GrantW {
    pub fn commit(self, used: usize) {
        self.grant.commit(used);
        // If we freed up any space, notify the waker on the reader side
        if used != 0 {
            self
            .storage
            .commit_waitcell
            .wake();
        }
    }
}

pub struct GrantR {
    grant: InnerGrantR<'static>,
    storage: HeapArc<BBQStorage>,
}

impl Deref for GrantR {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.grant.deref()
    }
}

impl DerefMut for GrantR {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.grant.deref_mut()
    }
}

impl GrantR {
    pub fn release(self, used: usize) {
        self.grant.release(used);
        // If we freed up any space, notify the waker on the reader side
        if used != 0 {
            self
            .storage
            .release_waitcell
            .wake();
        }
    }
}

// async methods
impl SpscProducer {
    #[tracing::instrument(
        name = "SpscProducer::send_grant_max",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub async fn send_grant_max(&self, max: usize) -> GrantW {
        loop {
            match self.producer.grant_max_remaining(max) {
                Ok(wgr) => {
                    trace!(size = wgr.len(), "Got bbqueue max write grant");
                    return GrantW {
                        grant: wgr,
                        storage: self.storage.clone(),
                    };
                }
                Err(_) => {
                    trace!("awaiting bbqueue max write grant");
                    // Uh oh! Couldn't get a send grant. We need to wait for the reader
                    // to release some bytes first.
                    self
                    .storage
                    .release_waitcell
                    .wait()
                    .await
                    .unwrap();

                    trace!("awoke for bbqueue max write grant");
                }
            }
        }
    }

    #[tracing::instrument(
        name = "SpscProducer::send_grant_exact",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub async fn send_grant_exact(&self, size: usize) -> GrantW {
        loop {
            match self.producer.grant_exact(size) {
                Ok(wgr) => {
                    trace!("Got bbqueue exact write grant",);
                    return GrantW {
                        grant: wgr,
                        storage: self.storage.clone(),
                    };
                }
                Err(_) => {
                    trace!("awaiting bbqueue exact write grant");
                    // Uh oh! Couldn't get a send grant. We need to wait for the reader
                    // to release some bytes first.
                    self
                    .storage
                    .release_waitcell
                    .wait()
                    .await
                    .unwrap();
                    trace!("awoke for bbqueue exact write grant");
                }
            }
        }
    }
}

impl SpscConsumer {
    #[tracing::instrument(
        name = "SpscConsumer::read_grant",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub async fn read_grant(&self) -> GrantR {
        loop {
            match self.consumer.read() {
                Ok(rgr) => {
                    trace!(size = rgr.len(), "Got bbqueue read grant",);
                    return GrantR {
                        grant: rgr,
                        storage: self.storage.clone(),
                    };
                }
                Err(_) => {
                    trace!("awaiting bbqueue read grant");
                    // Uh oh! Couldn't get a read grant. We need to wait for the writer
                    // to commit some bytes first.
                    self
                    .storage
                    .commit_waitcell
                    .wait()
                    .await
                    .unwrap();
                    trace!("awoke for bbqueue read grant");
                }
            }
        }
    }
}

// // sync methods
// impl BBQBidiHandle {
//     #[tracing::instrument(
//         name = "BBQueue::send_grant_max_sync",
//         level = "trace",
//         skip(self),
//         fields(queue = ?fmt::ptr(self.storage.deref()), side = ?self.side),
//     )]
//     pub fn send_grant_max_sync(&self, max: usize) -> Option<GrantW> {
//         self.producer
//             .grant_max_remaining(max)
//             .ok()
//             .map(|wgr| GrantW {
//                 grant: wgr,
//                 storage: self.storage.clone(),
//                 side: self.side,
//             })
//     }

//     #[tracing::instrument(
//         name = "BBQueue::send_grant_exact_sync",
//         level = "trace",
//         skip(self),
//         fields(queue = ?fmt::ptr(self.storage.deref()), side = ?self.side),
//     )]
//     pub fn send_grant_exact_sync(&self, size: usize) -> Option<GrantW> {
//         self.producer.grant_exact(size).ok().map(|wgr| GrantW {
//             grant: wgr,
//             storage: self.storage.clone(),
//             side: self.side,
//         })
//     }

//     #[tracing::instrument(
//         name = "BBQueue::read_grant_sync",
//         level = "trace",
//         skip(self),
//         fields(queue = ?fmt::ptr(self.storage.deref()), side = ?self.side),
//     )]
//     pub fn read_grant_sync(&self) -> Option<GrantR> {
//         self.consumer.read().ok().map(|rgr| GrantR {
//             grant: rgr,
//             storage: self.storage.clone(),
//             side: self.side,
//         })
//     }
// }
