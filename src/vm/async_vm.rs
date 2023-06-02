use super::*;

/// A Forth VM in which some builtin words are implemented by `async fn`s (or
/// [`Future`]s).
///
/// # Asynchronous Forth VMs
///
/// Asynchronous builtins are asynchronous relative to the *host* context (i.e.,
/// the Rust program in which the Forth VM is embedded), rather than the Forth
/// program that executes within the VM. This means that, unlike a
/// synchronous [`Forth`] VM, the [`AsyncForth::process_line`] method is an
/// [`async fn`]. When the Forth program executes a builtin word that is
/// implemented by an [`async fn`] on the host, the [`AsyncForth::process_line`]
/// will [`.await`] the [`Future`] that implements the builtin word, and will
/// yield if the `Future` is not ready. This allows multiple [`AsyncForth`] VMs
/// to run asynchronously in an async context on the host, yielding when the
/// Forth programs in those VMs sleep or perform asynchronous I/O operations.
///
/// # Providing Async Builtins
///
/// Unlike synchronous builtins, which are provided to the VM as a slice of
/// [`BuiltinEntry`]s, asynchronous builtins require an implementation of the
/// [`AsyncBuiltins`] trait, which provides both a slice of
/// [`AsyncBuiltinEntry`]s and a [method to dispatch builtin names to
/// `Future`s](AsyncBuiltins::dispatch_async). See the documentation for the
/// [`AsyncBuiltins`] trait for details on providing async builtins.
///
/// # Synchronous Builtins
///
/// An `AsyncForth` VM may also have synchronous builtin words. These behave
/// identically to the synchronous builtins in a non-async [`Forth`] VM.
/// Synchronous builtins should be used for any builtin word that does not
/// require performing an asynchronous operation on the host, such as those
/// which perform mathematical operations.
/// 
/// Synchronous builtins can be provided when the VM is constructed as a static
/// slice of [`BuiltinEntry`]s. They may also be added at runtime using the
/// [`AsyncForth::add_sync_builtin`] and
/// [`AsyncForth::add_sync_builtin_static_name`] method. These methods are
/// identical to the [`Forth::add_builtin`] and
/// [`Forth::add_builtin_static_name`] methods.
///
/// [`Future`]: core::future::Future
/// [`async fn`]: https://doc.rust-lang.org/stable/std/keyword.async.html
/// [`.await`]: https://doc.rust-lang.org/stable/std/keyword.await.html
pub struct AsyncForth<T: 'static, A> {
    vm: Forth<T>,
    builtins: A,
}

impl<T, A> AsyncForth<T, A>
where
    T: 'static,
    A: for<'forth> AsyncBuiltins<'forth, T>,
{
    pub unsafe fn new(
        dstack_buf: (*mut Word, usize),
        rstack_buf: (*mut Word, usize),
        cstack_buf: (*mut CallContext<T>, usize),
        dict: OwnedDict<T>,
        input: WordStrBuf,
        output: OutputBuf,
        host_ctxt: T,
        sync_builtins: &'static [BuiltinEntry<T>],
        async_builtins: A,
    ) -> Result<Self, Error> {
        let vm = Forth::new_async(dstack_buf, rstack_buf, cstack_buf, dict, input, output, host_ctxt, sync_builtins, A::BUILTINS)?;
        Ok(Self { vm, builtins: async_builtins })
    }

    /// Constructs a new VM whose dictionary is a fork of this VM's dictionary.
    ///
    /// The current dictionary owned by this VM is frozen (made immutable), and
    /// a reference to it is shared with this VM and the new child VM. When both
    /// this VM and the child are dropped, the frozen dictionary is deallocated.
    ///
    /// This function takes two [`OwnedDict`]s as arguments: `new_dict` is the
    /// dictionary allocation for the forked child VM, while `my_dict` is a new
    /// allocation for this VM's mutable dictionary (which replaces the current
    /// dictionary, as it will become frozen).
    ///
    /// The child VM is created with empty stacks, and the provided input and
    /// output buffers.
    ///
    /// # Safety
    ///
    /// This method requires the same invariants be upheld as
    /// [`AsyncForth::new`].
    pub unsafe fn fork(
        &mut self,
        new_dict: OwnedDict<T>,
        my_dict: OwnedDict<T>,
        dstack_buf: (*mut Word, usize),
        rstack_buf: (*mut Word, usize),
        cstack_buf: (*mut CallContext<T>, usize),
        input: WordStrBuf,
        output: OutputBuf,
        host_ctxt: T,
    ) -> Result<Self, Error>
    where A: Clone,
    {
        let vm = self.vm.fork(new_dict, my_dict, dstack_buf, rstack_buf, cstack_buf, input, output, host_ctxt)?;
        Ok(Self { vm, builtins: self.builtins.clone() })
    }

    /// Borrows this VM's [`OutputBuf`].
    #[inline]
    #[must_use]
    pub fn output(&self) -> &OutputBuf {
        &self.vm.output
    }

    /// Mutably borrows this VM's [`OutputBuf`].
    #[inline]
    #[must_use]
    pub fn output_mut(&mut self) -> &mut OutputBuf {
        &mut self.vm.output
    }

    /// Mutably borrows this VM's input [`WordStrBuf`].
    #[inline]
    #[must_use]
    pub fn input_mut(&mut self) -> &mut WordStrBuf {
        &mut self.vm.input
    }

    /// Borrows this VM's host context.
    #[inline]
    #[must_use]
    pub fn host_ctxt(&self) -> &T {
        &self.vm.host_ctxt
    }

    /// Mutably borrows this VM's host context.
    #[inline]
    #[must_use]
    pub fn host_ctxt_mut(&mut self) -> &mut T {
        &mut self.vm.host_ctxt
    }

    pub fn add_sync_builtin_static_name(
        &mut self,
        name: &'static str,
        bi: WordFunc<T>,
    ) -> Result<(), Error> {
        self.vm.add_builtin_static_name(name, bi)
    }

    pub fn add_sync_builtin(&mut self, name: &str, bi: WordFunc<T>) -> Result<(), Error> {
        self.vm.add_builtin(name, bi)
    }

    #[cfg(test)]
    pub(crate) fn vm_mut(&mut self) -> &mut Forth<T> {
        &mut self.vm
    }

    pub async fn process_line(&mut self) -> Result<(), Error> {
        let res = async {
            loop {
                match self.vm.start_processing_line()? {
                    ProcessAction::Done => {
                        self.vm.output.push_str("ok.\n")?;
                        break Ok(());
                    },
                    ProcessAction::Continue => {},
                    ProcessAction::Execute =>
                        while self.async_pig().await? != Step::Done {},
                }
            }
        }.await;
        match res {
            Ok(_) => Ok(()),
            Err(e) => {
                self.vm.data_stack.clear();
                self.vm.return_stack.clear();
                self.vm.call_stack.clear();
                Err(e)
            }
        }
    }

    // Single step execution (async version).
    async fn async_pig(&mut self) -> Result<Step, Error> {
        let Self { ref mut vm, ref builtins } = self;

        let top = match vm.call_stack.try_peek() {
            Ok(t) => t,
            Err(StackError::StackEmpty) => return Ok(Step::Done),
            Err(e) => return Err(Error::Stack(e)),
        };

        let kind = unsafe { top.eh.as_ref().kind };
        let res = unsafe { match kind {
            EntryKind::StaticBuiltin => (top.eh.cast::<BuiltinEntry<T>>().as_ref().func)(vm),
            EntryKind::RuntimeBuiltin => (top.eh.cast::<BuiltinEntry<T>>().as_ref().func)(vm),
            EntryKind::Dictionary => (top.eh.cast::<DictionaryEntry<T>>().as_ref().func)(vm),
            EntryKind::AsyncBuiltin => {
                builtins.dispatch_async(&top.eh.as_ref().name, vm).await
            },
        }};

        match res {
            Ok(_) => {
                let _ = vm.call_stack.pop();
            }
            Err(Error::PendingCallAgain) => {
                // ok, just don't pop
            }
            Err(e) => return Err(e),
        }

        Ok(Step::NotDone)
    }

    pub fn release(self) -> T {
        self.vm.release()
    }
}
