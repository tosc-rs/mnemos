//! A bbqueue based collection of single- and double- ended, async/await
//! byte buffer queues.
//!
//! This extends the underlying bbqueue type exposed by the ABI crate, allowing
//! for async kernel-to-kernel (including driver services) usage.

use core::ops::{Deref, DerefMut};

use crate::fmt;
use crate::tracing::{self, info, trace};
use abi::bbqueue_ipc::{BBBuffer, Consumer as InnerConsumer, Producer as InnerProducer};
use abi::bbqueue_ipc::{GrantR as InnerGrantR, GrantW as InnerGrantW};
use maitake::sync::Mutex;
use maitake::sync::WaitCell;
use mnemos_alloc::containers::{Arc, ArrayBuf};

struct BBQStorage {
    commit_waitcell: WaitCell,
    release_waitcell: WaitCell,
    // note: producer lives here so we don't need a separate Arc just for the
    // Mutex<InnerProducer>. consumer is owned by the consumer handle.
    producer: Mutex<Option<InnerProducer<'static>>>,

    ring: BBBuffer,
    _array: ArrayBuf<u8>,
}

pub struct BidiHandle {
    producer: SpscProducer,
    consumer: Consumer,
}

impl BidiHandle {
    pub fn producer(&self) -> &SpscProducer {
        &self.producer
    }

    pub fn consumer(&self) -> &Consumer {
        &self.consumer
    }

    pub fn split(self) -> (SpscProducer, Consumer) {
        (self.producer, self.consumer)
    }
}

pub async fn new_bidi_channel(capacity_a: usize, capacity_b: usize) -> (BidiHandle, BidiHandle) {
    let (a_prod, a_cons) = new_spsc_channel(capacity_a).await;
    let (b_prod, b_cons) = new_spsc_channel(capacity_b).await;
    let a = BidiHandle {
        producer: a_prod,
        consumer: b_cons,
    };
    let b = BidiHandle {
        producer: b_prod,
        consumer: a_cons,
    };
    (a, b)
}

pub struct SpscProducer {
    storage: Arc<BBQStorage>,
    producer: InnerProducer<'static>,
}

#[derive(Clone)]
pub struct MpscProducer {
    storage: Arc<BBQStorage>,
}

pub struct Consumer {
    storage: Arc<BBQStorage>,
    consumer: InnerConsumer<'static>,
}

impl SpscProducer {
    pub async fn into_mpmc_producer(self) -> MpscProducer {
        let SpscProducer { storage, producer } = self;
        *storage.producer.lock().await = Some(producer);
        MpscProducer { storage }
    }
}

pub async fn new_spsc_channel(capacity: usize) -> (SpscProducer, Consumer) {
    info!(capacity, "Creating new mpsc BBQueue channel");
    let mut _array = ArrayBuf::new_uninit(capacity).await;

    let ring = BBBuffer::new();

    unsafe {
        let (ptr, len) = _array.ptrlen();
        ring.initialize(ptr.as_ptr().cast(), len);
    }

    let storage = Arc::new(BBQStorage {
        commit_waitcell: WaitCell::new(),
        release_waitcell: WaitCell::new(),
        producer: Mutex::new(None),
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
    let cons = Consumer {
        storage,
        consumer: cons,
    };

    info!("Channel created successfully");

    (prod, cons)
}

pub struct GrantW {
    grant: InnerGrantW<'static>,
    storage: Arc<BBQStorage>,
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
            self.storage.commit_waitcell.wake();
        }
    }
}

pub struct GrantR {
    grant: InnerGrantR<'static>,
    storage: Arc<BBQStorage>,
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
            self.storage.release_waitcell.wake();
        }
    }
}

unsafe impl Send for GrantR {}
unsafe impl Sync for GrantR {}

#[inline]
async fn producer_send_grant_max(
    max: usize,
    producer: &InnerProducer<'static>,
    storage: &Arc<BBQStorage>,
) -> GrantW {
    loop {
        match producer.grant_max_remaining(max) {
            Ok(wgr) => {
                trace!(size = wgr.len(), "Got bbqueue max write grant");
                return GrantW {
                    grant: wgr,
                    storage: storage.clone(),
                };
            }
            Err(_) => {
                trace!("awaiting bbqueue max write grant");
                // Uh oh! Couldn't get a send grant. We need to wait for the reader
                // to release some bytes first.
                storage.release_waitcell.wait().await.unwrap();

                trace!("awoke for bbqueue max write grant");
            }
        }
    }
}

async fn producer_send_grant_exact(
    size: usize,
    producer: &InnerProducer<'static>,
    storage: &Arc<BBQStorage>,
) -> GrantW {
    loop {
        match producer.grant_exact(size) {
            Ok(wgr) => {
                trace!("Got bbqueue exact write grant",);
                return GrantW {
                    grant: wgr,
                    storage: storage.clone(),
                };
            }
            Err(_) => {
                trace!("awaiting bbqueue exact write grant");
                // Uh oh! Couldn't get a send grant. We need to wait for the reader
                // to release some bytes first.
                storage.release_waitcell.wait().await.unwrap();
                trace!("awoke for bbqueue exact write grant");
            }
        }
    }
}

// async methods
impl MpscProducer {
    #[tracing::instrument(
        name = "MpscProducer::send_grant_max",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub async fn send_grant_max(&self, max: usize) -> GrantW {
        let producer = self.storage.producer.lock().await;
        let producer = producer.as_ref().unwrap();
        producer_send_grant_max(max, producer, &self.storage).await
    }

    #[tracing::instrument(
        name = "MpscProducer::send_grant_exact",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub async fn send_grant_exact(&self, size: usize) -> GrantW {
        let producer = self.storage.producer.lock().await;
        let producer = producer.as_ref().unwrap();
        producer_send_grant_exact(size, producer, &self.storage).await
    }
}

impl SpscProducer {
    #[tracing::instrument(
        name = "SpscProducer::send_grant_max",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub async fn send_grant_max(&self, max: usize) -> GrantW {
        producer_send_grant_max(max, &self.producer, &self.storage).await
    }

    #[tracing::instrument(
        name = "SpscProducer::send_grant_exact",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub async fn send_grant_exact(&self, size: usize) -> GrantW {
        producer_send_grant_exact(size, &self.producer, &self.storage).await
    }
}

impl Consumer {
    #[tracing::instrument(
        name = "Consumer::read_grant",
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
                    self.storage.commit_waitcell.wait().await.unwrap();
                    trace!("awoke for bbqueue read grant");
                }
            }
        }
    }
}

// sync methods
impl SpscProducer {
    #[tracing::instrument(
        name = "SpscProducer::send_grant_exact_sync",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub fn send_grant_exact_sync(&self, size: usize) -> Option<GrantW> {
        self.producer.grant_exact(size).ok().map(|wgr| GrantW {
            grant: wgr,
            storage: self.storage.clone(),
        })
    }

    #[tracing::instrument(
        name = "SpscProducer::send_grant_max_sync",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub fn send_grant_max_sync(&self, max: usize) -> Option<GrantW> {
        self.producer
            .grant_max_remaining(max)
            .ok()
            .map(|wgr| GrantW {
                grant: wgr,
                storage: self.storage.clone(),
            })
    }
}

impl MpscProducer {
    #[tracing::instrument(
        name = "MpscProducer::send_grant_exact_sync",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub fn send_grant_exact_sync(&self, size: usize) -> Option<GrantW> {
        let producer = self.storage.producer.try_lock()?;
        let wgr = producer.as_ref()?.grant_exact(size).ok()?;
        Some(GrantW {
            grant: wgr,
            storage: self.storage.clone(),
        })
    }

    #[tracing::instrument(
        name = "MpscProducer::send_grant_max_sync",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub fn send_grant_max_sync(&self, max: usize) -> Option<GrantW> {
        let producer = self.storage.producer.try_lock()?;
        let wgr = producer.as_ref()?.grant_max_remaining(max).ok()?;
        Some(GrantW {
            grant: wgr,
            storage: self.storage.clone(),
        })
    }
}

impl Consumer {
    #[tracing::instrument(
        name = "Consumer::read_grant_sync",
        level = "trace",
        skip(self),
        fields(queue = ?fmt::ptr(self.storage.deref())),
    )]
    pub fn read_grant_sync(&self) -> Option<GrantR> {
        self.consumer.read().ok().map(|rgr| GrantR {
            grant: rgr,
            storage: self.storage.clone(),
        })
    }
}
