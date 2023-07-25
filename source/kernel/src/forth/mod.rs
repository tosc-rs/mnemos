use crate::services::forth_spawnulator::SpawnulatorClient;
use crate::tracing;
use crate::{
    comms::bbq,
    services::serial_mux::{PortHandle, SerialMuxClient},
    Kernel,
};
use core::{any::TypeId, future::Future, ptr::NonNull, time::Duration};
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
    containers::{ArrayBuf, Box, FixedVec},
    heap::{alloc, dealloc},
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
    pub spawnulator_timeout: Duration,
}

pub struct Forth {
    pub(crate) forth: AsyncForth<MnemosContext, Dispatcher>,
    stdio: bbq::BidiHandle,
    _bufs: Bufs,
}

/// Owns the heap allocations for a `Forth` task.
struct Bufs {
    dstack: ArrayBuf<Word>,
    rstack: ArrayBuf<Word>,
    cstack: ArrayBuf<CallContext<MnemosContext>>,
    input: ArrayBuf<u8>,
    output: ArrayBuf<u8>,
}

impl Forth {
    pub async fn new(
        kernel: &'static Kernel,
        params: Params,
    ) -> Result<(Self, bbq::BidiHandle), &'static str> {
        let (stdio, streams) = params.alloc_stdio().await;
        let forth = Self::new_with_stdio(kernel, params, stdio).await?;
        Ok((forth, streams))
    }

    pub async fn new_with_stdio(
        kernel: &'static Kernel,
        params: Params,
        stdio: bbq::BidiHandle,
    ) -> Result<Self, &'static str> {
        let bufs = params.alloc_bufs().await;
        let dict = params.alloc_dict().await?;

        let input = WordStrBuf::new(bufs.input.ptrlen().0.as_ptr().cast(), params.input_buf_size);
        let output = OutputBuf::new(
            bufs.output.ptrlen().0.as_ptr().cast(),
            params.output_buf_size,
        );
        let host_ctxt = MnemosContext::new(kernel, params).await;

        let forth = unsafe {
            AsyncForth::new(
                (bufs.dstack.ptrlen().0.as_ptr().cast(), params.stack_size),
                (bufs.rstack.ptrlen().0.as_ptr().cast(), params.stack_size),
                (bufs.cstack.ptrlen().0.as_ptr().cast(), params.stack_size),
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
            _bufs: bufs,
        };
        Ok(forth)
    }

    #[tracing::instrument(
        level = tracing::Level::INFO,
        "Forth",
        skip(self),
        fields(id = self.forth.host_ctxt().id)
    )]
    pub async fn run(mut self) {
        tracing::info!("VM running");
        loop {
            self.forth.output_mut().clear();

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
                    const ERROR: &[u8] = b"ERROR.\n";
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
        }
    }
}

pub(crate) struct MnemosContext {
    kernel: &'static Kernel,
    boh: BagOfHolding,
    /// Used for allocating child VMs
    params: Params,
    /// Forth task ID.
    // TODO(eliza): should we just use the `maitake` task ID, instead?
    id: usize,
    /// Handle for spawning child tasks.
    spawnulator: SpawnulatorClient,
}

impl MnemosContext {
    pub fn id(&self) -> usize {
        self.id
    }
}

#[derive(Copy, Clone)]
pub(crate) struct Dispatcher;

struct DropDict;

impl<'forth> AsyncBuiltins<'forth, MnemosContext> for Dispatcher {
    type Future = impl Future<Output = Result<(), forth3::Error>> + 'forth;

    const BUILTINS: &'static [AsyncBuiltinEntry<MnemosContext>] = &[
        async_builtin!("sermux::open_port"),
        async_builtin!("sermux::write_outbuf"),
        async_builtin!("spawn"),
        // sleep for a number of microseconds
        async_builtin!("sleep::us"),
        // sleep for a number of milliseconds
        async_builtin!("sleep::ms"),
        // sleep for a number of seconds
        async_builtin!("sleep::s"),
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
                "spawn" => spawn_forth_task(forth).await,
                "sleep::us" => sleep(forth, Duration::from_micros).await,
                "sleep::ms" => sleep(forth, Duration::from_millis).await,
                "sleep::s" => sleep(forth, Duration::from_secs).await,
                _ => {
                    tracing::warn!("unimplemented async builtin: {}", id.as_str());
                    Err(forth3::Error::WordNotInDict)
                }
            }?;
            Ok(())
        }
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
            spawnulator_timeout: Duration::from_secs(5),
        }
    }

    /// Allocate new input and output streams with the configured capacity.
    async fn alloc_stdio(&self) -> (bbq::BidiHandle, bbq::BidiHandle) {
        bbq::new_bidi_channel(self.stdout_capacity, self.stdin_capacity).await
    }

    /// Allocate the buffers for a new `Forth` task, based on the provided `Params`.
    async fn alloc_bufs(&self) -> Bufs {
        Bufs {
            dstack: ArrayBuf::new_uninit(self.stack_size).await,
            rstack: ArrayBuf::new_uninit(self.stack_size).await,
            cstack: ArrayBuf::new_uninit(self.stack_size).await,
            input: ArrayBuf::new_uninit(self.input_buf_size).await,
            output: ArrayBuf::new_uninit(self.output_buf_size).await,
        }
    }

    /// Allocate a new `OwnedDict` with this `Params`' dictionary size.
    async fn alloc_dict(&self) -> Result<OwnedDict<MnemosContext>, &'static str> {
        let layout = Dictionary::<MnemosContext>::layout(self.dictionary_size)
            .map_err(|_| "invalid dictionary size")?;
        let dict_buf = alloc(layout)
            .await
            .cast::<core::mem::MaybeUninit<Dictionary<MnemosContext>>>();
        tracing::trace!(
            size = self.dictionary_size,
            addr = &format_args!("{dict_buf:p}"),
            "Allocated dictionary"
        );
        Ok(OwnedDict::new::<DropDict>(dict_buf, self.dictionary_size))
    }
}

impl Default for Params {
    fn default() -> Self {
        Self::new()
    }
}

impl MnemosContext {
    async fn new(kernel: &'static Kernel, params: Params) -> Self {
        static NEXT_TASK_ID: AtomicUsize = AtomicUsize::new(0);
        let boh = BagOfHolding::new(params.bag_of_holding_capacity).await;
        Self {
            boh,
            kernel,
            params,
            id: NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed),
            spawnulator: kernel
                .timeout(
                    params.spawnulator_timeout,
                    SpawnulatorClient::from_registry(kernel),
                )
                .await
                .expect("Spawnulator client timed out - is the spawnulator running?"),
        }
    }
}

/// Temporary helper extension trait. Should probably be upstreamed
/// into `forth3` at a later date.
trait ConvertWord {
    fn into_usize(self) -> Result<usize, forth3::Error>;
    fn into_u16(self) -> Result<u16, forth3::Error>;
    fn into_i32(self) -> i32;
}

impl ConvertWord for Word {
    fn into_usize(self) -> Result<usize, forth3::Error> {
        let data: i32 = unsafe { self.data };
        data.try_into()
            .map_err(|_| forth3::Error::WordToUsizeInvalid(data))
    }

    fn into_u16(self) -> Result<u16, forth3::Error> {
        let data: i32 = unsafe { self.data };
        // TODO: not totally correct error type
        data.try_into()
            .map_err(|_| forth3::Error::WordToUsizeInvalid(data))
    }

    fn into_i32(self) -> i32 {
        unsafe { self.data }
    }
}

/// Binding for [`SerialMuxClient::open_port()`]
///
/// Call: `PORT SZ sermux::open_port`
/// Return: BOH_TOKEN on stack
///
/// Errors on any invalid parameters. See [`BagOfHolding`] for details
/// on bag of holding tokens
async fn sermux_open_port(forth: &mut forth3::Forth<MnemosContext>) -> Result<(), forth3::Error> {
    let sz = forth.data_stack.try_pop()?.into_usize()?;
    let port = forth.data_stack.try_pop()?.into_u16()?;

    // TODO: These two steps could be considered "non-execeptional" if failed.
    // We could codify that zero is an invalid BOH_TOKEN, and put zero on the
    // stack instead, to allow userspace to handle errors if wanted.
    //
    let mut mux_hdl = SerialMuxClient::from_registry(forth.host_ctxt.kernel).await;

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

/// Binding for [`PortHandle::send()`]
///
/// Writes the current contents of the output buffer to the [`PortHandle`].
///
/// Call: `BOH_TOKEN sermux::write_outbuf`
/// Return: No change
///
/// Errors if the provided handle is incorrect. See [`BagOfHolding`] for details
/// on bag of holding tokens
async fn sermux_write_outbuf(
    forth: &mut forth3::Forth<MnemosContext>,
) -> Result<(), forth3::Error> {
    let idx = forth.data_stack.try_pop()?.into_i32();
    let port: &PortHandle = forth
        .host_ctxt
        .boh
        .get(idx)
        .ok_or(forth3::Error::InternalError)?;

    port.send(forth.output.as_str().as_bytes()).await;
    Ok(())
}

/// Binding for [`Kernel::spawn()`]
///
/// Spawns a new Forth task that inherits from this task's dictionary. The task
/// will begin executing the provided function address.
///
/// Call: `XT spawn`.
/// Return: the task ID of the spawned Forth task.
async fn spawn_forth_task(forth: &mut forth3::Forth<MnemosContext>) -> Result<(), forth3::Error> {
    let xt = forth.data_stack.try_pop()?;
    tracing::debug!("Forking Forth VM...");
    let params = forth.host_ctxt.params;
    let kernel = forth.host_ctxt.kernel;

    // TODO(eliza): store the child's stdio in the
    // parent's host context so we can actually do something with it...
    let (stdio, _streams) = params.alloc_stdio().await;
    let bufs = params.alloc_bufs().await;
    let new_dict = params.alloc_dict().await.map_err(|error| {
        tracing::error!(?error, "Failed to allocate dictionary for child VM");
        forth3::Error::InternalError
    })?;
    let my_dict = params.alloc_dict().await.map_err(|error| {
        tracing::error!(
            ?error,
            "Failed to allocate replacement dictionary for parent VM"
        );
        forth3::Error::InternalError
    })?;
    let host_ctxt = MnemosContext::new(kernel, params).await;
    let child_id = host_ctxt.id;
    let input = WordStrBuf::new(bufs.input.ptrlen().0.as_ptr().cast(), params.input_buf_size);
    let output = OutputBuf::new(
        bufs.output.ptrlen().0.as_ptr().cast(),
        params.output_buf_size,
    );

    let mut child = unsafe {
        forth.fork(
            new_dict,
            my_dict,
            (bufs.dstack.ptrlen().0.as_ptr().cast(), params.stack_size),
            (bufs.rstack.ptrlen().0.as_ptr().cast(), params.stack_size),
            (bufs.cstack.ptrlen().0.as_ptr().cast(), params.stack_size),
            input,
            output,
            host_ctxt,
        )
    }
    .map_err(|error| {
        tracing::error!(?error, "Failed to construct Forth VM");
        forth3::Error::InternalError
    })?;

    // start the child running the popped execution token.
    child.data_stack.push(xt)?;
    // TODO(eliza): it would be nicer if we could just push a call context for
    // the execution token...
    child.input.fill("execute").map_err(|error| {
        tracing::error!(?error, "Failed to set child input!");
        forth3::Error::InternalError
    })?;

    let child = Forth {
        forth: AsyncForth::from_forth(child, Dispatcher),
        stdio,
        _bufs: bufs,
    };

    tracing::info!(
        parent.id = forth.host_ctxt.id,
        child.id = child_id,
        "Forked Forth VM!"
    );

    let spawn_fut = forth.host_ctxt.spawnulator.spawn(child);

    let timeout_res = kernel.timeout(params.spawnulator_timeout, spawn_fut).await;

    match timeout_res {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            tracing::error!(?error, "Failed to enqueue child task to spawn!");
            Err(forth3::Error::InternalError)
        }
        Err(e) => {
            tracing::error!(
                ?e,
                "Spawning child task failed - is the spawnulator running?"
            );
            Err(forth3::Error::InternalError)
        }
    }
}

/// Binding for [`Kernel::sleep()`]
///
/// Sleep for the provided duration.
///
/// Call: `DURATION {sleep::us, sleep::ms, sleep::s}`.
/// Return: No change
async fn sleep(
    forth: &mut forth3::Forth<MnemosContext>,
    into_duration: impl FnOnce(u64) -> Duration,
) -> Result<(), forth3::Error> {
    let duration = {
        let duration = forth.data_stack.try_pop()?.into_i32();
        if duration.is_negative() {
            tracing::warn!(duration, "Cannot sleep for a negative duration!");
            return Err(forth3::Error::WordToUsizeInvalid(duration));
        }
        into_duration(duration as u64)
    };
    tracing::trace!(?duration, "sleeping...");
    forth.host_ctxt.kernel.sleep(duration).await;
    tracing::trace!(?duration, "...slept!");
    Ok(())
}

impl dictionary::DropDict for DropDict {
    unsafe fn drop_dict(ptr: NonNull<u8>, layout: core::alloc::Layout) {
        dealloc(ptr.as_ptr().cast(), layout);
    }
}

// ----

/// The Bag of Holding
///
/// The Bag of Holding contains type-erased items that can be retrieved with
/// a provided `i32` token. A token is provided on calling [BagOfHolding::register()].
/// At the registration time, the TypeId of the item is also recorded, and the item
/// is moved into [`Box<T>`], which is leaked and type erased.
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
    inner: FixedVec<(i32, BohValue)>,
}

impl BagOfHolding {
    /// Create a new BagOfHolding with a given max size
    ///
    /// The `kernel` parameter is used to allocate `HeapBox` elements to
    /// store the type-erased elements residing in the BagOfHolding.
    pub async fn new(max: usize) -> Self {
        BagOfHolding {
            idx: 0,
            inner: FixedVec::new(max).await,
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
            if !self
                .inner
                .as_slice()
                .iter()
                .any(|(idx, _)| *idx == self.idx)
            {
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
        let value_ptr = NonNull::new(Box::into_raw(Box::new(item).await))?.cast();
        let idx = self.next_idx();
        let tid = TypeId::of::<T>();

        let _ = self
            .inner
            .try_push((
                idx,
                BohValue {
                    tid,
                    value_ptr,
                    dropfn: dropfn::<T>,
                },
            ))
            .ok()
            .unwrap_or_else(|| {
                debug_assert!(false, "Push failed after checking we aren't full?");
            });

        Some(idx)
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
        let val = &self.inner.as_slice().iter().find(|(i, _item)| *i == idx)?.1;
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
        let _ = Box::from_raw(i.as_ptr());
    }
}
