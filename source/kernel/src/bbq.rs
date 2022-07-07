//! Wrappers around the IPC bbqueue
//!
//! These types are intended to be used ONLY in the kernel, where we
//! can expect a "single executor" async operation. At some point, this
//! may inform later design around user-to-kernel bbqueue communication.

use core::mem::MaybeUninit;

use mnemos_alloc::{containers::HeapArc, heap::AHeap};
use abi::bbqueue_ipc::{BBBuffer, Producer, Consumer};
use tracing::{error, info};

struct BBQStorage {
    _ring_a: BBBuffer,
    _ring_b: BBBuffer,
}

pub struct BBQBidiHandle {
    producer: Producer<'static>,
    consumer: Consumer<'static>,

    // SAFETY: `producer` and `consumer` are only valid for the lifetime of `_storage`
    _storage: HeapArc<BBQStorage>,
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
    let (sto_a_ptr, _) = alloc.allocate_array_with(MaybeUninit::<u8>::uninit, capacity_a_tx).await.leak();
    let (sto_b_ptr, _) = alloc.allocate_array_with(MaybeUninit::<u8>::uninit, capacity_b_tx).await.leak();

    let ring_a = BBBuffer::new();
    let ring_b = BBBuffer::new();

    unsafe {
        ring_a.initialize(sto_a_ptr.as_ptr().cast(), capacity_a_tx);
        ring_b.initialize(sto_b_ptr.as_ptr().cast(), capacity_b_tx);
    }

    let storage = alloc.allocate_arc(BBQStorage { _ring_a: ring_a, _ring_b: ring_b }).await;

    let a_bbbuffer = &storage._ring_a as *const BBBuffer as *mut BBBuffer;
    let b_bbbuffer = &storage._ring_b as *const BBBuffer as *mut BBBuffer;

    let hdl_a = unsafe {
        // handle A gets the PRODUCER from ring A, and the CONSUMER from ring B.
        let a_prod = BBBuffer::take_producer(a_bbbuffer);
        let b_cons = BBBuffer::take_consumer(b_bbbuffer);

        BBQBidiHandle {
            producer: a_prod,
            consumer: b_cons,
            _storage: storage.clone(),
        }
    };

    let hdl_b = unsafe {
        // handle B gets the PRODUCER from ring B, and the CONSUMER from ring A.
        let b_prod = BBBuffer::take_producer(b_bbbuffer);
        let a_cons = BBBuffer::take_consumer(a_bbbuffer);

        BBQBidiHandle {
            producer: b_prod,
            consumer: a_cons,
            _storage: storage.clone(),
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

impl BBQBidiHandle {
    #[inline(always)]
    pub fn producer(&self) -> &Producer<'static> {
        &self.producer
    }

    #[inline(always)]
    pub fn consumer(&self) -> &Consumer<'static> {
        &self.consumer
    }
}
