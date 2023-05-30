use forth3::{AsyncForth, dictionary::{AsyncBuiltinEntry, AsyncBuiltins, OwnedDict}, fastr::FaStr, word::Word, CallContext,};
use mnemos_alloc::{containers::{HeapBox, HeapFixedVec}, heap::Heap};
use core::future::Future;
use portable_atomic::{Ordering, AtomicPtr}

use crate::{Kernel, comms::bbq};


#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct Params {
    pub stack_size: usize,
    pub input_buf_size: usize,
    pub output_buf_size: usize,
    pub dictionary_size: usize,
}

pub struct Forth {
    forth: AsyncForth<MnemosContext, Dispatcher>,
    bidi: bbq::BidiHandle,
    _payload_dstack: HeapFixedVec<Word>,
    _payload_rstack: HeapFixedVec<Word>,
    _payload_cstack: HeapFixedVec<CallContext<MnemosContext>>,
    _input_buf: HeapFixedVec<u8>,
    _output_buf: HeapFixedVec<u8>,
}

impl Forth {
    pub async fn new(kernel: &'static Kernel, params: Params) -> (Self, bbq::BidiHandle) {
        let heap = kernel.heap();
        let dstack_buf = heap.allocate_fixed_vec(params.stack_size).await;
        let rstack_buf = heap.allocate_fixed_vec(params.stack_size).await;
        let cstack_buf = heap.allocate_fixed_vec(params.stack_size).await;
        let input_buf = heap.allocate_fixed_vec(params.input_buf_size).await;
        let output_buf = heap.allocate_fixed_vec(params.output_buf_size).await;
        let dict_buf = heap.allocate_fixed_vec(params.dictionary_size).await;
        let dict = OwnedDict::new();
        todo!("eliza")
    }

    pub async fn run(self) {
        todo!("eliza")
    }
}

struct MnemosContext {
    // TODO(eliza): eventually the host context will have stuff in it...
}

struct Dispatcher;

struct DropDict;

impl<'forth> AsyncBuiltins<'forth, MnemosContext> for Dispatcher {
    type Future = impl Future<Output = Result<(), forth3::Error>>;

    const BUILTINS: &'static [AsyncBuiltinEntry<MnemosContext>] = &[];

    fn dispatch_async(&self, id: &FaStr, forth: &'forth mut forth3::Forth<MnemosContext>) -> Self::Future {
        async {
            Ok(())
        }
    }
}

// workaround for no global allocator
static ALLOCATOR: AtomicPtr<AHeap> = AtomicPtr::new(ptr::null_mut());