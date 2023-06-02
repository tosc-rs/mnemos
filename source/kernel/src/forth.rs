use crate::{
    comms::bbq,
    drivers::serial_mux::{PortHandle, SerialMuxHandle},
    Kernel,
};
use core::{any::TypeId, future::Future, ptr::NonNull};
use forth3::{
    async_builtin,
    dictionary::{self, AsyncBuiltinEntry, AsyncBuiltins, Dictionary, OwnedDict},
    fastr::FaStr,
    input::WordStrBuf,
    output::OutputBuf,
    word::Word,
    AsyncForth, CallContext,
};
use mnemos_alloc::{
    containers::{HeapBox, HeapFixedVec},
    heap,
};
use portable_atomic::{AtomicUsize, Ordering};

#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct Params {
    pub stack_size: usize,
    pub input_buf_size: usize,
    pub output_buf_size: usize,
    pub dictionary_size: usize,
    pub stdin_capacity: usize,
    pub stdout_capacity: usize,
    pub bag_of_holding_capacity: usize,
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
    pub async fn new(
        kernel: &'static Kernel,
        params: Params,
    ) -> Result<(Self, bbq::BidiHandle), &'static str> {
        static NEXT_TASK_ID: AtomicUsize = AtomicUsize::new(0);

        let heap = kernel.heap();
        let (stdio, streams) =
            bbq::new_bidi_channel(heap, params.stdout_capacity, params.stdin_capacity).await;
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
            let layout = Dictionary::<MnemosContext>::layout(params.dictionary_size)
                .map_err(|_| "invalid dictionary size")?;
            let dict_buf = heap
                .allocate_raw(layout)
                .await
                .cast::<core::mem::MaybeUninit<Dictionary<MnemosContext>>>();
            OwnedDict::new::<DropDict>(dict_buf, params.dictionary_size)
        };
        let host_ctxt = MnemosContext {
            kernel,
            boh: BagOfHolding::new(kernel, params.bag_of_holding_capacity).await,
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
                Dispatcher,
            )
            .map_err(|err| {
                tracing::error!(?err, "Failed to construct Forth VM");
                "failed to construct Forth VM"
            })?
        };
        let forth = Self {
            forth,
            stdio,
            _payload_dstack: dstack_buf,
            _payload_cstack: cstack_buf,
            _payload_rstack: rstack_buf,
            _input_buf: input_buf,
            _output_buf: output_buf,
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
                        self.forth
                            .input_mut()
                            .fill(input)
                            .expect("eliza: why would this fail?");
                        read.release(len);
                    }
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
                    // TODO(ajm): Provide some kind of fixed length error string?
                    const ERROR: &[u8] = b"ERROR.";
                    let mut send = self.stdio.producer().send_grant_exact(ERROR.len()).await;
                    send.copy_from_slice(ERROR);
                    send.commit(ERROR.len());
                    // TODO(ajm): I need a "clear" function for the input. This wont properly
                    // clear string literals either.
                    let inp = self.forth.input_mut();
                    while inp.cur_word().is_some() {
                        inp.advance();
                    }
                }
            }

            self.forth.output_mut().clear();
        }
    }
}

struct MnemosContext {
    kernel: &'static Kernel,
    boh: BagOfHolding,
}

struct Dispatcher;

struct DropDict;

impl<'forth> AsyncBuiltins<'forth, MnemosContext> for Dispatcher {
    type Future = impl Future<Output = Result<(), forth3::Error>> + 'forth;

    const BUILTINS: &'static [AsyncBuiltinEntry<MnemosContext>] = &[
        async_builtin!("sermux::open_port"),
        async_builtin!("sermux::write_outbuf"),
    ];

    fn dispatch_async(
        &self,
        id: &'static FaStr,
        forth: &'forth mut forth3::Forth<MnemosContext>,
    ) -> Self::Future {
        async {
            match id.as_str() {
                "sermux::open_port" => sermux_open_port(forth).await,
                "sermux::write_outbuf" => sermux_write_outbuf(forth).await,
                _ => {
                    tracing::warn!("unimplemented async builtin: {}", id.as_str());
                    Err(forth3::Error::WordNotInDict)
                }
            }?;
            Ok(())
        }
    }
}

/// Temporary helper extension trait. Should probably be upstreamed
/// into `forth3` at a later date.
trait ConvertWord {
    fn as_usize(self) -> Result<usize, forth3::Error>;
    fn as_u16(self) -> Result<u16, forth3::Error>;
    fn as_i32(self) -> i32;
}

impl ConvertWord for Word {
    fn as_usize(self) -> Result<usize, forth3::Error> {
        let data: i32 = unsafe { self.data };
        data.try_into()
            .map_err(|_| forth3::Error::WordToUsizeInvalid(data))
    }

    fn as_u16(self) -> Result<u16, forth3::Error> {
        let data: i32 = unsafe { self.data };
        // TODO: not totally correct error type
        data.try_into()
            .map_err(|_| forth3::Error::WordToUsizeInvalid(data))
    }

    fn as_i32(self) -> i32 {
        unsafe { self.data }
    }
}

/// Binding for [SerialMuxHandle::open_port()]
///
/// Call: `PORT SZ sermux::open_port`
/// Return: BOH_TOKEN on stack
///
/// Errors on any invalid parameters. See [BagOfHolding] for details
/// on bag of holding tokens
async fn sermux_open_port(forth: &mut forth3::Forth<MnemosContext>) -> Result<(), forth3::Error> {
    let sz = forth.data_stack.try_pop()?.as_usize()?;
    let port = forth.data_stack.try_pop()?.as_u16()?;

    // TODO: These two steps could be considered "non-execeptional" if failed.
    // We could codify that zero is an invalid BOH_TOKEN, and put zero on the
    // stack instead, to allow userspace to handle errors if wanted.
    //
    let mut mux_hdl = SerialMuxHandle::from_registry(forth.host_ctxt.kernel)
        .await
        .ok_or(forth3::Error::InternalError)?;

    let port = mux_hdl
        .open_port(port, sz)
        .await
        .ok_or(forth3::Error::InternalError)?;
    //
    // End TODO

    let idx = forth
        .host_ctxt
        .boh
        .register(port)
        .await
        .ok_or(forth3::Error::InternalError)?;

    forth.data_stack.push(Word::data(idx))?;
    Ok(())
}

/// Binding for [PortHandle::send()]
///
/// Writes the current contents of the output buffer to the [PortHandle].
///
/// Call: `BOH_TOKEN sermux::write_outbuf`
/// Return: No change
///
/// Errors if the provided handle is incorrect. See [BagOfHolding] for details
/// on bag of holding tokens
async fn sermux_write_outbuf(
    forth: &mut forth3::Forth<MnemosContext>,
) -> Result<(), forth3::Error> {
    let idx = forth.data_stack.try_pop()?.as_i32();
    let port: &PortHandle = forth
        .host_ctxt
        .boh
        .get(idx)
        .ok_or(forth3::Error::InternalError)?;

    port.send(forth.output.as_str().as_bytes()).await;
    Ok(())
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
            bag_of_holding_capacity: 16,
        }
    }
}

// ----

/// The Bag of Holding
///
/// The Bag of Holding contains type-erased items that can be retrieved with
/// a provided `i32` token. A token is provided on calling [BagOfHolding::register()].
/// At the registration time, the TypeId of the item is also recorded, and the item
/// is moved into [`HeapBox<T>`], which is leaked and type erased.
///
/// When retrieving items from the Bag of Holding, the same token and type parameter
/// `T` must be used for access. This access is made by calling [BagOfHolding::get()].
///
/// The purpose of this structure is to allow the forth userspace to use an i32 token,
/// which fits into a single stack value, to refer to specific instances of data. This
/// allows for forth-bound builtin functions to retrieve the referred objects in a
/// type safe way.
pub struct BagOfHolding {
    idx: i32,
    inner: HeapFixedVec<(i32, BohValue)>,
    kernel: &'static Kernel,
}

impl BagOfHolding {
    /// Create a new BagOfHolding with a given max size
    ///
    /// The `kernel` parameter is used to allocate `HeapBox` elements to
    /// store the type-erased elements residing in the BagOfHolding.
    pub async fn new(kernel: &'static Kernel, max: usize) -> Self {
        let inner = kernel.heap().allocate_fixed_vec(max).await;
        BagOfHolding {
            idx: 0,
            inner,
            kernel,
        }
    }

    /// Generate a new, currently unused, Bag of Holding token.
    ///
    /// This token will never be zero, and a given bag of holding will never
    /// contain two items with the same token.
    fn next_idx(&mut self) -> i32 {
        // todo we could do better lol
        loop {
            self.idx = self.idx.wrapping_add(1);
            if self.idx == 0 {
                continue;
            }
            if !self.inner.iter().any(|(idx, _)| *idx == self.idx) {
                return self.idx;
            }
        }
    }

    /// Place an item into the bag of holding
    ///
    /// The item will be allocated into a `HeapBox`, and a non-zero i32 token will
    /// be returned on success.
    ///
    /// Returns an error if the Bag of Holding is full.
    ///
    /// At the moment there is no way to "unregister" an item, it will exist until
    /// the BagOfHolding is dropped.
    pub async fn register<T>(&mut self, item: T) -> Option<i32>
    where
        T: 'static,
    {
        if self.inner.is_full() {
            return None;
        }
        let value_ptr = self.kernel.heap().allocate(item).await.leak().cast::<()>();
        let idx = self.next_idx();
        let tid = TypeId::of::<T>();

        // Should never fail - we checked whether we are full above already
        let pushed = self.inner.push((
            idx,
            BohValue {
                tid,
                value_ptr,
                dropfn: dropfn::<T>,
            },
        ));

        match pushed {
            Ok(_) => Some(idx),
            Err(_) => {
                debug_assert!(false, "We already checked if this was full?");
                None
            }
        }
    }

    /// Attempt to retrieve an item from the Bag of Holding
    ///
    /// This will only succeed if the same `T` is used as was used when calling
    /// [`BagOfHolding::register()`], and if the token matches the one returned
    /// by `register()`.
    ///
    /// If the token is unknown, or the `T` does not match, `None` will be returned
    ///
    /// At the moment, no `get_mut` functionality is provided. It *could* be, as the
    /// Bag of Holding represents ownership of the contained items, however it would
    /// not be possible to retrieve multiple mutable items at the same time. This
    /// could be added in the future if desired.
    pub fn get<T>(&self, idx: i32) -> Option<&T>
    where
        T: 'static,
    {
        let val = &self.inner.iter().find(|(i, _item)| *i == idx)?.1;
        let tid = TypeId::of::<T>();
        if val.tid != tid {
            return None;
        }
        let t = val.value_ptr.cast::<T>();
        unsafe { Some(t.as_ref()) }
    }
}

/// A container item for a type-erased object
struct BohValue {
    /// The type id of the `T` pointed to by `leaked`
    tid: TypeId,
    /// A non-null pointer to a `T`, contained in a leaked `HeapBox<T>`.
    value_ptr: NonNull<()>,
    /// A type-erased function that will un-leak the `HeapBox<T>`, and drop it
    dropfn: fn(NonNull<()>),
}

/// Implementing drop on BohValue allows for dropping of a `BagOfHolding` to properly
/// drop the elements it is holding, without knowing what types they are.
impl Drop for BohValue {
    fn drop(&mut self) {
        (self.dropfn)(self.value_ptr);
    }
}

/// A free function which is used by [BagOfHolding::register()] to
/// monomorphize a drop function to be held in a [BohValue].
fn dropfn<T>(bs: NonNull<()>) {
    let i: NonNull<T> = bs.cast();
    unsafe {
        let _ = HeapBox::from_leaked(i);
    }
}
