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
use mnemos_alloc::{containers::HeapArc, heap::AHeap};
use tracing::{error, info, trace};

struct BBQWaitCells {
    commit_waitcell: WaitCell,
    release_waitcell: WaitCell,
}

struct BBQStorage {
    _ring_a: BBBuffer,
    _ring_b: BBBuffer,
    a_wait: BBQWaitCells,
    b_wait: BBQWaitCells,
}

#[derive(Clone, Copy, Debug)]
enum Side {
    ASide,
    BSide,
}

pub struct BBQBidiHandle {
    producer: Producer<'static>,
    consumer: Consumer<'static>,
    side: Side,

    // SAFETY: all above items are ONLY valid for the lifetime of `storage`
    storage: HeapArc<BBQStorage>,
}

pub async fn new_bidi_channel(
    alloc: &'static AHeap,
    capacity_a_tx: usize,
    capacity_b_tx: usize,
) -> (BBQBidiHandle, BBQBidiHandle) {
    info!(
        a_capacity = capacity_a_tx,
        b_capacity = capacity_b_tx,
        "Creating new bidirectional BBQueue channel"
    );
    let (sto_a_ptr, _) = alloc
        .allocate_array_with(MaybeUninit::<u8>::uninit, capacity_a_tx)
        .await
        .leak();
    let (sto_b_ptr, _) = alloc
        .allocate_array_with(MaybeUninit::<u8>::uninit, capacity_b_tx)
        .await
        .leak();

    let ring_a = BBBuffer::new();
    let ring_b = BBBuffer::new();

    unsafe {
        ring_a.initialize(sto_a_ptr.as_ptr().cast(), capacity_a_tx);
        ring_b.initialize(sto_b_ptr.as_ptr().cast(), capacity_b_tx);
    }

    let storage = alloc
        .allocate_arc(BBQStorage {
            _ring_a: ring_a,
            _ring_b: ring_b,
            a_wait: BBQWaitCells {
                commit_waitcell: WaitCell::new(),
                release_waitcell: WaitCell::new(),
            },
            b_wait: BBQWaitCells {
                commit_waitcell: WaitCell::new(),
                release_waitcell: WaitCell::new(),
            },
        })
        .await;

    let a_bbbuffer = &storage._ring_a as *const BBBuffer as *mut BBBuffer;
    let b_bbbuffer = &storage._ring_b as *const BBBuffer as *mut BBBuffer;

    let hdl_a = unsafe {
        // handle A gets the PRODUCER from ring A, and the CONSUMER from ring B.
        let a_prod = BBBuffer::take_producer(a_bbbuffer);
        let b_cons = BBBuffer::take_consumer(b_bbbuffer);

        BBQBidiHandle {
            producer: a_prod,
            consumer: b_cons,
            side: Side::ASide,
            storage: storage.clone(),
        }
    };

    let hdl_b = unsafe {
        // handle B gets the PRODUCER from ring B, and the CONSUMER from ring A.
        let b_prod = BBBuffer::take_producer(b_bbbuffer);
        let a_cons = BBBuffer::take_consumer(a_bbbuffer);

        BBQBidiHandle {
            producer: b_prod,
            consumer: a_cons,
            side: Side::BSide,
            storage: storage.clone(),
        }
    };

    info!("Channel created successfully");

    (hdl_a, hdl_b)
}

impl Drop for BBQStorage {
    fn drop(&mut self) {
        error!("Dropping two BBQueues inside of a bidirection channel! Unleaking is not yet supported!");
    }
}

use abi::bbqueue_ipc::{GrantR as InnerGrantR, GrantW as InnerGrantW};

pub struct GrantW {
    grant: InnerGrantW<'static>,
    storage: HeapArc<BBQStorage>,
    side: Side,
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
            match self.side {
                Side::ASide => &self.storage.a_wait,
                Side::BSide => &self.storage.b_wait,
            }
            .commit_waitcell
            .wake();
        }
    }
}

pub struct GrantR {
    grant: InnerGrantR<'static>,
    storage: HeapArc<BBQStorage>,
    side: Side,
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
            match self.side {
                Side::ASide => &self.storage.a_wait,
                Side::BSide => &self.storage.b_wait,
            }
            .release_waitcell
            .wake();
        }
    }
}

impl BBQBidiHandle {
    // async fn send_grant(buf_len: usize) -> GrantW
    // async fn read_grant() -> GrantR
    #[tracing::instrument(
        name = "BBQueue::send_grant_max",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref()), side = ?self.side),
    )]
    pub async fn send_grant_max(&self, max: usize) -> GrantW {
        loop {
            match self.producer.grant_max_remaining(max) {
                Ok(wgr) => {
                    trace!(size = wgr.len(), "Got bbqueue max write grant");
                    return GrantW {
                        grant: wgr,
                        side: self.side,
                        storage: self.storage.clone(),
                    };
                }
                Err(_) => {
                    trace!("awaiting bbqueue max write grant");
                    // Uh oh! Couldn't get a send grant. We need to wait for the OTHER reader
                    // to release some bytes first.
                    match self.side {
                        Side::ASide => &self.storage.b_wait,
                        Side::BSide => &self.storage.a_wait,
                    }
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
        name = "BBQueue::send_grant_exact",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref()), side = ?self.side),
    )]
    pub async fn send_grant_exact(&self, size: usize) -> GrantW {
        loop {
            match self.producer.grant_exact(size) {
                Ok(wgr) => {
                    trace!("Got bbqueue exact write grant",);
                    return GrantW {
                        grant: wgr,
                        side: self.side,
                        storage: self.storage.clone(),
                    };
                }
                Err(_) => {
                    trace!("awaiting bbqueue exact write grant");
                    // Uh oh! Couldn't get a send grant. We need to wait for the OTHER reader
                    // to release some bytes first.
                    match self.side {
                        Side::ASide => &self.storage.b_wait,
                        Side::BSide => &self.storage.a_wait,
                    }
                    .release_waitcell
                    .wait()
                    .await
                    .unwrap();
                    trace!("awoke for bbqueue exact write grant");
                }
            }
        }
    }

    #[tracing::instrument(
        name = "BBQueue::read_grant",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref()), side = ?self.side),
    )]
    pub async fn read_grant(&self) -> GrantR {
        loop {
            match self.consumer.read() {
                Ok(rgr) => {
                    trace!(size = rgr.len(), "Got bbqueue read grant",);
                    return GrantR {
                        grant: rgr,
                        side: self.side,
                        storage: self.storage.clone(),
                    };
                }
                Err(_) => {
                    trace!("awaiting bbqueue read grant");
                    // Uh oh! Couldn't get a read grant. We need to wait for the OTHER writer
                    // to commit some bytes first.
                    match self.side {
                        Side::ASide => &self.storage.b_wait.commit_waitcell,
                        Side::BSide => &self.storage.a_wait.commit_waitcell,
                    }
                    .wait()
                    .await
                    .unwrap();
                    trace!("awoke for bbqueue read grant");
                }
            }
        }
    }
}
