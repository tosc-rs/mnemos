use forth3::{AsyncForth, dictionary::{AsyncBuiltinEntry, AsyncBuiltins, OwnedDict, Dictionary, self}, fastr::FaStr, word::Word, CallContext, input::WordStrBuf, output::OutputBuf}; 
use mnemos_alloc::{containers::{HeapFixedVec}, heap};
use core::{future::Future, ptr::NonNull};
use portable_atomic::{AtomicUsize, Ordering};
use crate::{Kernel, comms::bbq};


#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct Params {
    pub stack_size: usize,
    pub input_buf_size: usize,
    pub output_buf_size: usize,
    pub dictionary_size: usize,
    pub stdin_capacity: usize,
    pub stdout_capacity: usize,
}

pub struct Forth {
    forth: AsyncForth<MnemosContext, Dispatcher>,
    stdio: bbq::BidiHandle,
    _payload_dstack: HeapFixedVec<Word>,
    _payload_rstack: HeapFixedVec<Word>,
    _payload_cstack: HeapFixedVec<CallContext<MnemosContext>>,
    _input_buf: HeapFixedVec<u8>,
    _output_buf: HeapFixedVec<u8>,
    id: usize,
}

impl Forth {
    pub async fn new(kernel: &'static Kernel, params: Params) -> Result<(Self, bbq::BidiHandle), &'static str> {
        static NEXT_TASK_ID: AtomicUsize = AtomicUsize::new(0);

        let heap = kernel.heap();
        let (stdio, streams) = bbq::new_bidi_channel(heap, params.stdout_capacity, params.stdin_capacity).await;
        // TODO(eliza): group all of these into one struct so that we don't have
        // to do a bunch of waiting for separate allocations?
        let mut dstack_buf = heap.allocate_fixed_vec(params.stack_size).await;
        let mut rstack_buf = heap.allocate_fixed_vec(params.stack_size).await;
        let mut cstack_buf = heap.allocate_fixed_vec(params.stack_size).await;
        let mut input_buf = heap.allocate_fixed_vec(params.input_buf_size).await;
        let mut output_buf = heap.allocate_fixed_vec(params.output_buf_size).await;

        let input = WordStrBuf::new(input_buf.as_mut_ptr(), params.input_buf_size);
        let output = OutputBuf::new(output_buf.as_mut_ptr(), params.output_buf_size);
        let dict = {
            let layout = Dictionary::<MnemosContext>::layout(params.dictionary_size).map_err(|_| "invalid dictionary size")?;
            let dict_buf = heap.allocate_raw(layout).await.cast::<core::mem::MaybeUninit<Dictionary<MnemosContext>>>();
            OwnedDict::new::<DropDict>(dict_buf, params.dictionary_size)
        };
        let host_ctxt = MnemosContext {
            kernel,
        };
        let forth = unsafe {
            AsyncForth::new(
                (dstack_buf.as_mut_ptr(), params.stack_size),
                (rstack_buf.as_mut_ptr(), params.stack_size),
                (cstack_buf.as_mut_ptr(), params.stack_size),
                dict,
                input,
                output,
                host_ctxt,
                forth3::Forth::FULL_BUILTINS,
                Dispatcher
            )
                .map_err(|err| {
                    tracing::error!(?err, "Failed to construct Forth VM");
                    "failed to construct Forth VM"
                })?

        };
        let forth = Self {
            forth, stdio,
            _payload_dstack: dstack_buf,
            _payload_cstack: cstack_buf,
            _payload_rstack: rstack_buf,
            _input_buf: input_buf, _output_buf: output_buf,
            id: NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed),
        };
        Ok((forth, streams))
    }

    #[tracing::instrument(
        level = tracing::Level::INFO,
        "Forth",
        skip(self),
        fields(id = self.id)
    )]
    pub async fn run(mut self) {
        tracing::info!("VM running");
        loop {
            // read from stdin
            {
                let read = self.stdio.consumer().read_grant().await;
                let len = read.len();
                match core::str::from_utf8(&read) {
                    Ok(input) => {
                        tracing::debug!(len, "> {:?}", input.trim());
                        self.forth.input_mut().fill(input).expect("eliza: why would this fail?");
                        read.release(len);
                    },
                    Err(_e) => todo!("eliza: what to do if the input is not utf8?"),
                };
            }

            match self.forth.process_line().await {
                Ok(()) => {
                    let out_str = self.forth.output().as_str();
                    let output = out_str.as_bytes();
                    // write the task's output to stdout
                    let len = output.len();
                    tracing::debug!(len, "< {out_str}");
                    let mut send = self.stdio.producer().send_grant_exact(output.len()).await;
                    send.copy_from_slice(output);
                    send.commit(len);
                }
                Err(error) => {
                    tracing::error!(?error);
                    // TODO(eliza): send the error to the task's output stream?
                }
            }

            self.forth.output_mut().clear();
        }
    }
}

struct MnemosContext {
    kernel: &'static Kernel,
}

struct Dispatcher;

struct DropDict;

impl<'forth> AsyncBuiltins<'forth, MnemosContext> for Dispatcher {
    type Future = impl Future<Output = Result<(), forth3::Error>>;

    const BUILTINS: &'static [AsyncBuiltinEntry<MnemosContext>] = &[];

    fn dispatch_async(&self, id: &FaStr, forth: &'forth mut forth3::Forth<MnemosContext>) -> Self::Future {
        tracing::warn!("unimplemented async builtin: {}", id.as_str());
        async {
            Ok(())
        }
    }
}

impl dictionary::DropDict for DropDict {
    unsafe fn drop_dict(ptr: NonNull<u8>, layout: core::alloc::Layout) {
        heap::deallocate_raw(ptr.cast(), layout);
    }
}

impl Params {
    pub const fn new() -> Self {
        Self {
            stack_size: 256,
            input_buf_size: 256,
            output_buf_size: 256,
            dictionary_size: 4096,
            stdin_capacity: 1024,
            stdout_capacity: 1024,
        }
    }
}