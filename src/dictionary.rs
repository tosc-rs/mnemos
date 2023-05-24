use crate::fastr::FaStr;
use crate::{Word, WordFunc};
use core::{
    alloc::{Layout, LayoutError},
    marker::PhantomData,
    mem::{self, MaybeUninit},
    ptr::{self, addr_of_mut, NonNull},
    ops::{Deref, DerefMut}
};
use portable_atomic::{Ordering::*, AtomicUsize};

#[derive(Debug, PartialEq)]
pub enum BumpError {
    OutOfMemory,
    CantAllocUtf8,
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
#[non_exhaustive]
pub enum EntryKind {
    StaticBuiltin,
    RuntimeBuiltin,
    Dictionary,
    #[cfg(feature = "async")]
    AsyncBuiltin,
}

/// Where a dictionary entry was found
pub enum DictLocation<T: 'static> {
    /// The entry was found in the current (mutable) dictionary.
    Parent(NonNull<DictionaryEntry<T>>),
    /// The entry was found in a parent (frozen) dictionary.
    Current(NonNull<DictionaryEntry<T>>),
}

#[repr(C)]
pub struct EntryHeader<T: 'static> {
    pub name: FaStr,
    pub kind: EntryKind, // todo
    pub len: u16,
    pub _pd: PhantomData<T>,
}

#[repr(C)]
pub struct BuiltinEntry<T: 'static> {
    pub hdr: EntryHeader<T>,
    pub func: WordFunc<T>,
}

/// A dictionary entry for an asynchronous builtin word.
///
/// This type is typically created using the [`async_builtin!`
/// macro](crate::async_builtin), and is used with the
/// [`AsyncForth`](crate::AsyncForth) VM type only. See the [documentation for
/// `AsyncForth`](crate::AsyncForth) for details on using asynchronous builtin
/// words.
#[repr(C)]
#[cfg(feature = "async")]
pub struct AsyncBuiltinEntry<T: 'static> {
    pub hdr: EntryHeader<T>,
}

// Starting FORTH: page 220
#[repr(C)]
pub struct DictionaryEntry<T: 'static> {
    pub hdr: EntryHeader<T>,
    pub func: WordFunc<T>,

    /// Link field, points back to the previous entry
    pub(crate) link: Option<NonNull<DictionaryEntry<T>>>,

    /// data OR an array of compiled code.
    /// the first word is the "p(arameter)fa" or "c(ode)fa"
    pub(crate) parameter_field: [Word; 0],
}

/// A handle to an owned, mutable dictionary allocation.
pub struct OwnedDict<T: 'static>(NonNull<Dictionary<T>>);

/// A handle to a shared, atomically reference counted dictionary allocation.
///
/// The contents of this dictionary are frozen and can no longer be mutated.
/// However, a `SharedDict` can be inexpensively cloned by incrementing its
/// reference count.
///
/// When a VM is forked into a child VM, its current [`OwnedDict`] is
/// transformed into a `SharedDict`, which both its new `OwnedDict` and the
/// child VM's `OwnedDict` will reference as their parents.
pub(crate) struct SharedDict<T: 'static>(NonNull<Dictionary<T>>);

pub struct Dictionary<T: 'static> {
    pub(crate) tail: Option<NonNull<DictionaryEntry<T>>>,
    pub(crate) alloc: DictionaryBump,
    /// Reference count, used to determine when the dictionary can be dropped.
    /// If this is `usize::MAX`, the dictionary is mutable.
    refs: portable_atomic::AtomicUsize,
    /// Parent dictionary.
    ///
    /// When looking up a binding that isn't present in `self`, we traverse this
    /// chain of references. When dropping the dictionary, we decrement the
    /// parent's ref count.
    parent: Option<SharedDict<T>>,
    deallocate: unsafe fn (ptr: NonNull<u8>, layout: Layout),
}

pub trait DropDict {
    /// Deallocate a dictionary.
    unsafe fn drop_dict(ptr: NonNull<u8>, layout: Layout);
}

pub(crate) struct EntryBuilder<'dict, T: 'static> {
    dict: &'dict mut Dictionary<T>,
    len: u16,
    base: NonNull<DictionaryEntry<T>>,
    kind: EntryKind,
}

pub(crate) struct DictionaryBump {
    pub(crate) start: *mut u8,
    pub(crate) cur: *mut u8,
    pub(crate) end: *mut u8,
}

/// Iterator over a [`Dictionary`]'s entries.
pub(crate) struct Entries<'dict, T: 'static> {
    next: Option<NonNull<DictionaryEntry<T>>>,
    dict: CurrDict<'dict, T>,
}

enum CurrDict<'dict, T: 'static> {
    Leaf(&'dict Dictionary<T>),
    Parent(SharedDict<T>),
}

#[cfg(feature = "async")]
/// A set of asynchronous builtin words, and a method to dispatch builtin names
/// to [`Future`]s.
///
/// This trait is used along with the [`AsyncForth`] type to
/// allow some builtin words to be implemented by `async fn`s (or [`Future`]s),
/// rather than synchronous functions. See [here][async-vms] for an overview of
/// how asynchronous Forth VMs work.
///
/// # Implementing Async Builtins
///
/// Synchronous builtins are provided to the Forth VM as a static slice of
/// [`BuiltinEntry`]s. These entries allow the VM to lookup builtin words by
/// name, and also contain a function pointer to the host function that
/// implements that builtin. Asynchronous builtins work somewhat differently: a
/// slice of [`AsyncBuiltinEntry`]s is still used in order to define the names
/// of the asynchronous builtin words, but because asynchronous functions return
/// a [`Future`] whose type must be known, an [`AsyncBuiltinEntry`] does *not*
/// contain a function pointer to a host function. Instead, once the name of an
/// async builtin is looked up, it is passed to the
/// [`AsyncBuiltins::dispatch_async`] method, which returns the [`Future`]
/// corresponding to that builtin function.
///
/// This indirection allows the `AsyncBuiltins` trait to erase the various
/// [`Future`] types which are returned by the async builtin functions, allowing
/// the [`AsyncForth`] VM to have only a single additional generic parameter for
/// the `AsyncBuiltins` implementation itself. Without the indirection of
/// [`dispatch_async`], the [`AsyncForth`] VM would need to be generic over
/// *every* possible [`Future`] type that may be returned by an async builtin
/// word, which would be impractical.[^1]
///
/// In order to erase multiple [`Future`] types, one of several approaches may
/// be used:
///
/// - The [`Future`] returned by [`dispatch_async`] can be an [`enum`] of each
///   builtin word's [`Future`] type. This requires all builtin words to be
///   implemented as named [`Future`] types, rather than [`async fn`]s, but
///   does not require heap allocation or unstable Rust features.
/// - The [`Future`] type can be a `Pin<Box<dyn Future<Output = Result<(),
///   Error>> + 'forth>`. This requires heap allocation, but can erase the type
///   of any number of async builtin futures, which may be [`async fn`]s _or_
///   named [`Future`] types.
/// - If using nightly Rust, the
///   [`#![feature(impl_trait_in_assoc_type)]`][63063] unstable feature can be
///   enabled, allowing the [`AsyncBuiltins::Future`] associated type to be
///   `impl Future<Output = Result(), Error> + 'forth`. This does not require
///   heap allocation, and allows the [`dispatch_async`] method to return an
///   [`async`] block [`Future`] which [`match`]es on the builtin's name and
///   calls any number of [`async fn`]s or named [`Future`] types. This is the
///   preferred approach when nightly features may be used.
///
/// Since the [`AsyncBuiltins`] trait is generic over the lifetime for which the
/// [`Forth`] vm is borrowed mutably, the [`AsyncBuiltins::Future`] associated
/// type may also be generic over that lifetime. This allows the returned
/// [`Future`] to borrow the [`Forth`] VM so that its stacks can be mutated
/// while the builtin [`Future`] executes (e.g. the result of the asynchronous
/// operation can be pushed to the VM's `data` stack, et cetera).
///
/// [^1]: If the [`AsyncForth`] type was generic over every possible async
///     builtin future, it would have a large number of generic type parameters
///     which would all need to be filled in by the user. Additionally, because
///     Rust does not allow a type to have a variadic number of generic
///     parameters, there would have to be an arbitrary limit on the maximum
///     number of async builtin words.
///
/// [`AsyncForth`]: crate::AsyncForth
/// [`Future`]: core::future::Future
/// [async-vms]: crate::AsyncForth#asynchronous-forth-vms
/// [`async fn`]: https://doc.rust-lang.org/stable/std/keyword.async.html
/// [`async`]: https://doc.rust-lang.org/stable/std/keyword.async.html
/// [`dispatch_async`]: Self::dispatch_async
/// [`enum`]: https://doc.rust-lang.org/stable/std/keyword.enum.html
/// [`match`]: https://doc.rust-lang.org/stable/std/keyword.match.html
/// [`Forth`]: crate::Forth
/// [63063]: https://github.com/rust-lang/rust/issues/63063
pub trait AsyncBuiltins<'forth, T: 'static> {
    /// The [`Future`] type returned by [`Self::dispatch_async`].
    ///
    /// Since the `AsyncBuiltins` trait is generic over the lifetime of the
    /// [`Forth`](crate::Forth) VM, the [`Future`] type may mutably borrow the
    /// VM. This allows the VM's stacks to be mutated by the async builtin function.
    ///
    /// [`Future`]: core::future::Future
    type Future: core::future::Future<Output = Result<(), crate::Error>>;

    /// A static slice of [`AsyncBuiltinEntry`]s describing the builtins
    /// provided by this implementation of `AsyncBuiltin`s.
    ///
    /// [`AsyncBuiltinEntry`]s may be created using the
    /// [`async_builtin!`](crate::async_builtin) macro.
    const BUILTINS: &'static [AsyncBuiltinEntry<T>];

    /// Dispatch a builtin name (`id`) to an asynchronous builtin [`Future`].
    ///
    /// The returned [`Future`] may borrow the [`Forth`](crate::Forth) VM
    /// provided as an argument to this function, allowing it to mutate the VM's
    /// stacks as it executes.
    ///
    /// This method should return a [`Future`] for each builtin function
    /// definition in [`Self::BUILTINS`]. Typically, this is implemented by
    /// [`match`]ing the provided `id`, and returning the appropriate [`Future`]
    /// for each builtin name. See [the `AsyncBuiltin` trait's
    /// documentation][impling] for details on implementing this method.
    ///
    /// [`Future`]: core::future::Future
    /// [`match`]: https://doc.rust-lang.org/stable/std/keyword.match.html
    /// [impling]: #implementing-async-builtins
    fn dispatch_async(&self, id: &FaStr, forth: &'forth mut crate::Forth<T>) -> Self::Future;
}

impl<T: 'static> DictionaryEntry<T> {
    pub unsafe fn pfa(this: NonNull<Self>) -> NonNull<Word> {
        let ptr = this.as_ptr();
        let pfp: *mut [Word; 0] = addr_of_mut!((*ptr).parameter_field);
        NonNull::new_unchecked(pfp.cast::<Word>())
    }
}

impl<T: 'static> Dictionary<T> {
    const MUTABLE: usize = usize::MAX;

    /// Returns the [`Layout`] that must be allocated for a `Dictionary` of the
    /// given `size`.
    pub fn layout(size: usize) -> Result<Layout, LayoutError> {
        let (layout, _) = Layout::new::<Self>().extend(Layout::array::<u8>(size)?)?;
        Ok(layout.pad_to_align())
    }

    pub(crate) fn add_bi_fastr(&mut self, name: FaStr, bi: WordFunc<T>) -> Result<(), BumpError> {
        debug_assert_eq!(self.refs.load(Acquire), Self::MUTABLE);
        // Allocate and initialize the dictionary entry
        let dict_base = self.alloc.bump::<DictionaryEntry<T>>()?;
        unsafe {
            dict_base.as_ptr().write(DictionaryEntry {
                hdr: EntryHeader {
                    name,
                    kind: EntryKind::RuntimeBuiltin,
                    len: 0,
                    _pd: PhantomData,
                },
                func: bi,
                link: self.tail.take(),
                parameter_field: [],
            });
        }
        self.tail = Some(dict_base);
        Ok(())
    }

    pub(crate) fn build_entry(&mut self) -> Result<EntryBuilder<'_, T>, BumpError> {
        let base = self.alloc.bump::<DictionaryEntry<T>>()?;
        Ok(EntryBuilder {
            base,
            len: 0,
            dict: self,
            kind: EntryKind::Dictionary,
        })
    }

    pub(crate) fn entries(&self) -> Entries<'_, T> {
        Entries {
            next: self.tail,
            dict: CurrDict::Leaf(self),
        }
    }
}

// === SharedDict ===

impl<T: 'static> SharedDict<T> {
    const MAX_REFCOUNT: usize = Dictionary::<T>::MUTABLE - 1;

    // Non-inlined part of `drop`.
    #[inline(never)]
    unsafe fn drop_slow(&mut self) {
        unsafe {
            let dealloc = self.deallocate;
            let layout = Dictionary::<T>::layout(self.alloc.capacity()).unwrap();
            ptr::drop_in_place(self.0.as_ptr());
            (dealloc)(self.0.cast(), layout);
        }
    }
}

impl <T: 'static> Deref for SharedDict<T> {
    type Target = Dictionary<T>;
    fn deref(&self) -> &Self::Target {
        unsafe { self.0.as_ref() }
    }
}

impl<T: 'static> Clone for SharedDict<T>{
    #[inline]
    fn clone(&self) -> Self {
        // Using a relaxed ordering is alright here, as knowledge of the
        // original reference prevents other threads from erroneously deleting
        // the object.
        //
        // As explained in the [Boost documentation][1], Increasing the
        // reference counter can always be done with memory_order_relaxed: New
        // references to an object can only be formed from an existing
        // reference, and passing an existing reference from one thread to
        // another must already provide any required synchronization.
        //
        // [1]: (www.boost.org/doc/libs/1_55_0/doc/html/atomic/usage_examples.html)
        let old_size = self.refs.fetch_add(1, Relaxed);

        // However we need to guard against massive refcounts in case someone is `mem::forget`ing
        // `SharedDict`s. If we don't do this the count can overflow and users will use-after free. This
        // branch will never be taken in any realistic program. We abort because such a program is
        // incredibly degenerate, and we don't care to support it.
        //
        // This check is not 100% water-proof: we error when the refcount grows beyond `isize::MAX`.
        // But we do that check *after* having done the increment, so there is a chance here that
        // the worst already happened and we actually do overflow the `usize` counter. However, that
        // requires the counter to grow from `isize::MAX` to `usize::MAX` between the increment
        // above and the `abort` below, which seems exceedingly unlikely.
        if old_size == Self::MAX_REFCOUNT {
            unreachable!("bad news")
        }

        Self(self.0)
    }
}


impl<T: 'static> Drop for SharedDict<T>{
    #[inline]
    fn drop(&mut self) {
        // Because `fetch_sub` is already atomic, we do not need to synchronize
        // with other threads unless we are going to delete the object. This
        // same logic applies to the below `fetch_sub` to the `weak` count.
        if self.refs.fetch_sub(1, Release) != 1 {
            return;
        }

        // This fence is needed to prevent reordering of use of the data and
        // deletion of the data. Because it is marked `Release`, the decreasing
        // of the reference count synchronizes with this `Acquire` fence. This
        // means that use of the data happens before decreasing the reference
        // count, which happens before this fence, which happens before the
        // deletion of the data.
        //
        // As explained in the [Boost documentation][1],
        //
        // > It is important to enforce any possible access to the object in one
        // > thread (through an existing reference) to *happen before* deleting
        // > the object in a different thread. This is achieved by a "release"
        // > operation after dropping a reference (any access to the object
        // > through this reference must obviously happened before), and an
        // > "acquire" operation before deleting the object.
        //
        // In particular, while the contents of an Arc are usually immutable, it's
        // possible to have interior writes to something like a Mutex<T>. Since a
        // Mutex is not acquired when it is deleted, we can't rely on its
        // synchronization logic to make writes in thread A visible to a destructor
        // running in thread B.
        //
        // Also note that the Acquire fence here could probably be replaced with an
        // Acquire load, which could improve performance in highly-contended
        // situations. See [2].
        //
        // [1]: (www.boost.org/doc/libs/1_55_0/doc/html/atomic/usage_examples.html)
        // [2]: (https://github.com/rust-lang/rust/pull/41714)
        portable_atomic::fence(Acquire);

        unsafe {
            self.drop_slow();
        }
    }
}

// === OwnedDict ===

impl<T: 'static> OwnedDict<T> {
    pub fn new<D: DropDict>(dict: NonNull<MaybeUninit<Dictionary<T>>>, size: usize) -> Self {

        // A helper type to provide proper layout generation for initialization
        #[repr(C)]
        struct DictionaryInner<T: 'static> {
            pub(crate) header: Dictionary<T>,
            bytes: [MaybeUninit<u8>; 0],
        }

        let ptr = dict.as_ptr().cast::<DictionaryInner<T>>();
        unsafe {
            let bump_base = addr_of_mut!((*ptr).bytes)
                // TODO(eliza): don't ignore the `MaybeUninit`ness of the bump region...
                .cast::<u8>();
            // Initialize the header, using `write` instead of assignment via
            // `=` to not call `drop` on the old, uninitialized value.
            addr_of_mut!((*ptr).header).write(Dictionary {
                tail: None,
                refs: AtomicUsize::new(Dictionary::<T>::MUTABLE),
                parent: None,
                alloc: DictionaryBump::new(bump_base, size),
                deallocate: D::drop_dict,
            });
        }
        Self(dict.cast::<Dictionary<T>>())
    }

    fn into_shared(self) -> SharedDict<T> {
        // don't let the destructor run, as it will deallocate the dictionary.
        let this = mem::ManuallyDrop::new(self);
        this.refs.compare_exchange(
            Dictionary::<T>::MUTABLE,
            1, AcqRel, Acquire
        ).expect("dictionary must have been mutable");
        SharedDict(this.0)
    }

    /// We swap `self` to the new, empty OwnedDict, and turn the old `self`
    /// into a SharedDict, both as the parent of our new self, as well as
    /// returning it for other use.
    pub(crate) fn fork_onto(&mut self, new: OwnedDict<T>) -> SharedDict<T> {
        let this = mem::replace(self, new).into_shared();
        self.set_parent(this.clone());
        this
    }

    pub(crate) fn set_parent(&mut self, parent: SharedDict<T>) {
        let _prev = self.parent.replace(parent);
        debug_assert!(_prev.is_none(), "parent dictionary shouldn't be clobbered!");
    }
}

impl<T: 'static> Deref for OwnedDict<T> {
    type Target = Dictionary<T>;
    fn deref(&self) -> &Self::Target {
        unsafe { self.0.as_ref() }
    }
}

impl<T: 'static> DerefMut for OwnedDict<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe {
            debug_assert_eq!(self.0.as_ref().refs.load(Acquire), Dictionary::<T>::MUTABLE);
            self.0.as_mut()
        }
    }
}

impl<T: 'static> Drop for OwnedDict<T> {
    fn drop(&mut self) {
        unsafe {
            let dealloc = self.deallocate;
            let layout = Dictionary::<T>::layout(self.alloc.capacity()).unwrap();
            ptr::drop_in_place(self.0.as_ptr());
            (dealloc)(self.0.cast(), layout);
        }
    }
}

// === EntryBuilder ===

impl<T: > EntryBuilder<'_, T> {
    pub(crate) fn write_word(mut self, word: Word) -> Result<Self, BumpError> {
        self.dict.alloc.bump_write(word)?;
        self.len += 1;
        Ok(self)
    }

    pub(crate) fn kind(self, kind: EntryKind) -> Self {
        Self { kind, ..self }
    }

    pub(crate) fn finish(self, name: FaStr, func: WordFunc<T>) -> NonNull<DictionaryEntry<T>> {
        unsafe {
            self.base.as_ptr().write(DictionaryEntry {
                hdr: EntryHeader {
                    name,
                    kind: self.kind,
                    len: self.len,
                    _pd: PhantomData
                },
                // TODO: Should arrays push length and ptr? Or just ptr?
                //
                // TODO: Should we look up `(variable)` for consistency?
                // Use `find_word`?
                func,

                // Don't link until we know we have a "good" entry!
                link: self.dict.tail.take(),
                parameter_field: [],
            });
        }
        self.dict.tail = Some(self.base);
        self.base
    }
}

// === impl Entries ===

impl<'dict, T: 'static> Iterator for Entries<'dict, T> {
    type Item = DictLocation<T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = match self.next.take() {
                Some(entry) => entry,
                None => {
                    // try to traverse the parent link
                    if let Some(parent) = self.dict.dict().parent.clone() {
                        self.next = parent.tail;
                        self.dict = CurrDict::Parent(parent);
                        continue;
                    } else {
                        return None;
                    }
                }
            };
            self.next = unsafe {
                // Safety: `self.next` must be a pointer into the VM's dictionary
                // entries. The caller who constructs a `Entries` iterator is
                // responsible for ensuring this.
                entry.as_ref().link
            };
            let found = match self.dict {
                CurrDict::Leaf(_) => DictLocation::Current(entry),
                CurrDict::Parent(_) => DictLocation::Parent(entry),
            };
            return Some(found);
        }
    }
}

impl<T> CurrDict<'_, T> {
    fn dict(&self) -> &'_ Dictionary<T> {
        match self {
            Self::Leaf(dict) => dict,
            Self::Parent(parent) => parent,
        }
    }
}

impl DictionaryBump {
    pub fn new(bottom: *mut u8, size: usize) -> Self {
        let end = bottom.wrapping_add(size);
        debug_assert!(end >= bottom);
        Self {
            end,
            start: bottom,
            cur: bottom,
        }
    }

    pub fn bump_str(&mut self, s: &str) -> Result<FaStr, BumpError> {
        debug_assert!(!s.is_empty());

        let len = s.len().min(31);
        let astr = &s.as_bytes()[..len];

        if !astr.iter().all(|b| b.is_ascii()) {
            return Err(BumpError::CantAllocUtf8);
        }
        let stir = self.bump_u8s(len).ok_or(BumpError::OutOfMemory)?.as_ptr();
        for (i, ch) in astr.iter().enumerate() {
            unsafe {
                stir.add(i).write(ch.to_ascii_lowercase());
            }
        }
        unsafe { Ok(FaStr::new(stir, len)) }
    }

    pub fn bump_u8s(&mut self, n: usize) -> Option<NonNull<u8>> {
        if n == 0 {
            return None;
        }

        let req = self.cur.wrapping_add(n);

        if req > self.end {
            None
        } else {
            let ptr = self.cur;
            self.cur = req;
            Some(unsafe { NonNull::new_unchecked(ptr) })
        }
    }

    pub fn bump_u8(&mut self) -> Option<NonNull<u8>> {
        if self.cur >= self.end {
            None
        } else {
            let ptr = self.cur;
            self.cur = self.cur.wrapping_add(1);
            Some(unsafe { NonNull::new_unchecked(ptr) })
        }
    }

    pub fn bump<T: Sized>(&mut self) -> Result<NonNull<T>, BumpError> {
        let offset = self.cur.align_offset(Layout::new::<T>().align());

        // Zero out any padding bytes!
        unsafe {
            self.cur.write_bytes(0x00, offset);
        }

        let align_cur = self.cur.wrapping_add(offset);
        let new_cur = align_cur.wrapping_add(Layout::new::<T>().size());

        if new_cur > self.end {
            Err(BumpError::OutOfMemory)
        } else {
            self.cur = new_cur;
            Ok(unsafe { NonNull::new_unchecked(align_cur.cast()) })
        }
    }

    pub fn bump_write<T: Sized>(&mut self, val: T) -> Result<(), BumpError> {
        let nnt = self.bump::<T>()?;
        unsafe {
            nnt.as_ptr().write(val);
        }
        Ok(())
    }

    /// Is the given pointer within the dictionary range?
    pub fn contains(&self, ptr: *mut ()) -> bool {
        let pau = ptr as usize;
        let sau = self.start as usize;
        let eau = self.end as usize;
        (pau >= sau) && (pau < eau)
    }

    pub fn capacity(&self) -> usize {
        (self.end as usize) - (self.start as usize)
    }

    pub fn used(&self) -> usize {
        (self.cur as usize) - (self.start as usize)
    }
}

impl<T: 'static> DictLocation<T> {
    pub(crate) fn entry(&self) -> NonNull<DictionaryEntry<T>> {
        match self {
            Self::Current(entry) => *entry,
            Self::Parent(entry) => *entry,
        }
    }
}

#[cfg(test)]
pub mod test {
    use core::{mem::size_of, sync::atomic::Ordering};
    use std::alloc::Layout;

    use crate::{
        dictionary::{DictionaryBump, DictionaryEntry, BuiltinEntry, DictLocation},
        leakbox::{LeakBox, alloc_dict, LeakBoxDict},
        Word, Error, Forth,
    };

    #[cfg(feature = "async")]
    use super::AsyncBuiltinEntry;

    use super::{EntryHeader, OwnedDict};

    #[test]
    fn sizes() {
        assert_eq!(size_of::<EntryHeader<()>>(), 3 * size_of::<usize>());
        assert_eq!(size_of::<BuiltinEntry<()>>(), 4 * size_of::<usize>());
        #[cfg(feature = "async")]
        assert_eq!(size_of::<AsyncBuiltinEntry<()>>(), 3 * size_of::<usize>());
    }

    #[test]
    fn do_a_bump() {
        let payload: LeakBox<u8> = LeakBox::new(256);

        let mut bump = DictionaryBump::new(payload.ptr(), payload.len());

        // Be annoying
        let _b = bump.bump_u8().unwrap();

        // ALLOT 10
        let d = bump.bump::<DictionaryEntry<()>>().unwrap();
        assert_eq!(
            d.as_ptr()
                .align_offset(Layout::new::<DictionaryEntry<()>>().align()),
            0
        );

        let walign = Layout::new::<DictionaryEntry<()>>().align();
        for _w in 0..10 {
            let w = bump.bump::<Word>().unwrap();
            assert_eq!(w.as_ptr().align_offset(walign), 0);
        }
    }

    // This test just checks that we can properly allocate and deallocate an OwnedDict
    //
    // Intended to be run with miri or valgrind where leaks are made apparent
    #[test]
    fn just_one_dict() {
        let buf: OwnedDict<()> = alloc_dict::<(), LeakBoxDict>(512);
        assert_eq!(buf.refs.load(Ordering::Relaxed), usize::MAX);
    }

    // This test just checks that we can properly allocate and deallocate a chain of dicts
    //
    // Intended to be run with miri or valgrind where leaks are made apparent
    #[test]
    fn nested_dicts() {
        let buf_1: OwnedDict<()> = alloc_dict::<(), LeakBoxDict>(512);
        let mut buf_2: OwnedDict<()> = alloc_dict::<(), LeakBoxDict>(256);
        let buf_1 = buf_1.into_shared();
        buf_2.parent = Some(buf_1);
    }

    // Similar to above, but making sure refcounting works properly
    #[test]
    fn shared_dicts() {
        let buf_1: OwnedDict<()> = alloc_dict::<(), LeakBoxDict>(512);
        let mut buf_2: OwnedDict<()> = alloc_dict::<(), LeakBoxDict>(256);
        let mut buf_3: OwnedDict<()> = alloc_dict::<(), LeakBoxDict>(128);
        let buf_1 = buf_1.into_shared();
        assert_eq!(buf_1.refs.load(Ordering::Relaxed), 1);
        buf_2.parent = Some(buf_1.clone());
        assert_eq!(buf_1.refs.load(Ordering::Relaxed), 2);
        buf_3.parent = Some(buf_1.clone());
        assert_eq!(buf_1.refs.load(Ordering::Relaxed), 3);

        drop(buf_2);
        assert_eq!(buf_1.refs.load(Ordering::Relaxed), 2);

        drop(buf_3);
        assert_eq!(buf_1.refs.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn allocs_work() {
        fn stubby(_f: &mut Forth<()>) -> Result<(), Error> {
            panic!("Don't ACTUALLY call me!");
        }

        let mut buf: OwnedDict<()> = alloc_dict::<(), LeakBoxDict>(512);
        assert!(buf.tail.is_none());

        let strname = buf.alloc.bump_str("stubby").unwrap();
        buf.add_bi_fastr(strname, stubby).unwrap();
        assert_eq!(unsafe { buf.tail.as_ref().unwrap().as_ref().hdr.name.as_str() }, "stubby");
    }

    #[test]
    fn fork_onto_works() {
        fn stubby(_f: &mut Forth<()>) -> Result<(), Error> {
            panic!("Don't ACTUALLY call me!");
        }

        // Put a builtin into the first slab
        let mut buf_1: OwnedDict<()> = alloc_dict::<(), LeakBoxDict>(512);
        let strname = buf_1.alloc.bump_str("stubby").unwrap();
        buf_1.add_bi_fastr(strname, stubby).unwrap();

        // Make a new dict slab, which "becomes" the mutable tip, with the original
        // slab as the parent of the new mutable tip
        let buf_2: OwnedDict<()> = alloc_dict::<(), LeakBoxDict>(512);
        let buf_1_ro = buf_1.fork_onto(buf_2);

        // Find the builtin in the original slab, it should say "current" here
        let ro_find = buf_1_ro.entries().find(|e| {
            unsafe { e.entry().as_ref() }.hdr.name.as_str() == "stubby"
        }).unwrap();
        assert!(matches!(ro_find, DictLocation::Current(_)));

        // Now find the builtin in the new mutable slab, it should say "parent" here
        let rw_find = buf_1.entries().find(|e| {
            unsafe { e.entry().as_ref() }.hdr.name.as_str() == "stubby"
        }).unwrap();
        assert!(matches!(rw_find, DictLocation::Parent(_)));
    }
}
