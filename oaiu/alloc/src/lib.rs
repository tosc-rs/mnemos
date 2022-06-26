#![no_std]

pub mod heap;
pub mod containers;
pub mod node;



// /// An Anachro Heap Array Type
// pub struct HeapFixedVec<T> {
//     pub(crate) len: usize,
//     pub(crate) capacity: usize,
//     pub(crate) ptr: *mut MaybeUninit<T>,
// }


// unsafe impl<T> Send for HeapFixedVec<T> {}

// impl<T> Deref for HeapFixedVec<T> {
//     type Target = [T];

//     fn deref(&self) -> &Self::Target {
//         // SAFETY: We can assume that all items 0..len are initialized
//         unsafe { core::slice::from_raw_parts(self.ptr.cast::<T>(), self.len) }
//     }
// }

// impl<T> DerefMut for HeapFixedVec<T> {
//     fn deref_mut(&mut self) -> &mut Self::Target {
//         // SAFETY: We can assume that all items 0..len are initialized
//         unsafe { core::slice::from_raw_parts_mut(self.ptr.cast::<T>(), self.len) }
//     }
// }

// impl<T> HeapFixedVec<T> {
//     pub fn push(&mut self, data: T) -> Result<(), T> {
//         if self.len >= self.capacity {
//             return Err(data);
//         }

//         unsafe {
//             self.ptr.add(self.len).write(MaybeUninit::new(data));
//         }
//         self.len += 1;

//         Ok(())
//     }

//     #[inline]
//     pub fn is_empty(&self) -> bool {
//         self.len == 0
//     }

//     #[inline]
//     pub fn len(&self) -> usize {
//         self.len
//     }

//     #[inline]
//     pub fn pop(&mut self) -> Option<T> {
//         if self.len == 0 {
//             None
//         } else {
//             unsafe {
//                 self.len -= 1;
//                 Some(core::ptr::read(self.ptr.add(self.len()).cast::<T>()))
//             }
//         }
//     }

//     pub fn try_remove(&mut self, index: usize) -> Result<T, ()> {
//         let len = self.len();
//         if index > len {
//             return Err(());
//         }

//         unsafe {
//             // infallible
//             let ret;
//             {
//                 // the place we are taking from.
//                 let ptr = self.ptr.add(index);
//                 // copy it out, unsafely having a copy of the value on
//                 // the stack and in the vector at the same time.
//                 ret = core::ptr::read(ptr.cast::<T>());

//                 // Shift everything down to fill in that spot.
//                 core::ptr::copy(ptr.offset(1), ptr, len - index - 1);
//             }
//             self.len -= 1;
//             Ok(ret)
//         }
//     }

//     /// Create a free_box, with location and layout information necessary
//     /// to free the box.
//     ///
//     /// SAFETY: This function creates aliasing pointers for the allocation. It
//     /// should ONLY be called in the destructor of the HeapBox when deallocation
//     /// is about to occur, and access to the Box will not be allowed again.
//     unsafe fn free_box(&mut self) -> FreeBox {
//         // SAFETY: If we allocated this item, it must have been small enough
//         // to properly construct a layout. Avoid Layout::array, as it only
//         // offers a checked method.
//         let layout = {
//             let array_size = size_of::<T>() * self.capacity;
//             Layout::from_size_align_unchecked(array_size, align_of::<T>())
//         };
//         FreeBox {
//             ptr: NonNull::new_unchecked(self.ptr.cast::<u8>()),
//             layout,
//         }
//     }
// }

// impl<T> Drop for HeapFixedVec<T> {
//     fn drop(&mut self) {
//         for i in 0..self.len {
//             unsafe {
//                 self.ptr.add(i).drop_in_place();
//             }
//         }
//         // Calculate the pointer, size, and layout of this allocation
//         let free_box = unsafe { self.free_box() };
//         // defmt::println!("[ALLOC] dropping array: {=usize}", free_box.layout.size());
//         free_box.box_drop();
//     }
// }

// /// A type representing a request to free a given allocation of memory.
// struct FreeBox {
//     ptr: NonNull<u8>,
//     layout: Layout,
// }

// impl FreeBox {
//     fn box_drop(self) {
//         // Attempt to immediately drop, if possible
//         if let Ok(mut hg) = HEAP.lock() {
//             unsafe {
//                 hg.free_raw(self.ptr, self.layout);
//             }
//             return;
//         } else {
//             // Nope, couldn't lock the heap.
//             //
//             // Try to store the allocation into the free list, and it will be
//             // reclaimed before the next alloc.
//             //
//             // SAFETY: A HeapBox can only be created if the Heap, and by extension the
//             // FreeQueue, has been previously initialized
//             let free_q = unsafe { FREE_Q.get_unchecked() };

//             // If the free list is completely full, for now, just panic.
//             free_q.enqueue(self).map_err(drop).expect("Should have had room in the free list...");
//         }
//     }
// }

// /// A guard type that provides mutually exclusive access to the allocator as
// /// long as the guard is held.
// pub struct HeapGuard {
//     heap: &'static mut Heap,
// }

// // Public HeapGuard methods
// impl HeapGuard {
//     pub unsafe fn free_raw(&mut self, ptr: NonNull<u8>, layout: Layout) {
//         self.deref_mut().deallocate(ptr, layout);
//         HEAP.any_frees.store(true, Ordering::Relaxed);
//     }

//     /// The free space (in bytes) available to the allocator
//     pub fn free_space(&self) -> usize {
//         self.deref().free()
//     }

//     /// The used space (in bytes) available to the allocator
//     pub fn used_space(&self) -> usize {
//         self.deref().used()
//     }

//     fn clean_allocs(&mut self) {
//         // First, grab the Free Queue.
//         //
//         // SAFETY: A HeapGuard can only be created if the Heap, and by extension the
//         // FreeQueue, has been previously initialized
//         let free_q = unsafe { FREE_Q.get_unchecked() };

//         let mut any = false;
//         // Then, free all pending memory in order to maximize space available.
//         while let Some(FreeBox { ptr, layout }) = free_q.dequeue() {
//             // defmt::println!("[ALLOC] FREE: {=usize}", layout.size());
//             // SAFETY: We have mutually exclusive access to the allocator, and
//             // the pointer and layout are correctly calculated by the relevant
//             // FreeBox types.
//             unsafe {
//                 self.deref_mut().deallocate(ptr, layout);
//                 any = true;
//             }
//         }

//         if any {
//             HEAP.any_frees.store(true, Ordering::Relaxed);
//         }
//     }

//     /// Attempt to allocate a HeapBox using the allocator
//     ///
//     /// If space was available, the allocation will be returned. If not, an
//     /// error will be returned
//     pub fn alloc_box<T>(&mut self, data: T) -> Result<HeapBox<T>, T> {
//         // Clean up any pending allocs
//         self.clean_allocs();

//         // Then, attempt to allocate the requested T.
//         let nnu8 = match self.deref_mut().allocate_first_fit(Layout::new::<T>()) {
//             Ok(t) => t,
//             Err(_) => return Err(data),
//         };
//         let ptr = nnu8.as_ptr().cast::<T>();

//         // And initialize it with the contents given to us
//         unsafe {
//             ptr.write(data);
//         }

//         Ok(HeapBox { ptr })
//     }

//     pub fn alloc_pin_box<T: Unpin>(&mut self, data: T) -> Result<Pin<HeapBox<T>>, T> {
//         Ok(Pin::new(self.alloc_box(data)?))
//     }

//     pub fn leak_send<T>(&mut self, inp: T) -> Result<&'static mut T, T>
//     where
//         T: Send + Sized + 'static,
//     {
//         let boxed = self.alloc_box(inp)?;
//         Ok(boxed.leak())
//     }

//     /// Attempt to allocate a HeapArray using the allocator
//     ///
//     /// If space was available, the allocation will be returned. If not, an
//     /// error will be returned
//     pub fn alloc_box_array<T: Copy + ?Sized>(
//         &mut self,
//         data: T,
//         count: usize,
//     ) -> Result<HeapArray<T>, ()> {
//         let f = || { data };
//         self.alloc_box_array_with(f, count)
//     }

//     pub fn alloc_box_array_with<T, F>(
//         &mut self,
//         f: F,
//         count: usize,
//     ) -> Result<HeapArray<T>, ()>
//     where
//         F: Fn() -> T,
//     {
//         // Clean up any pending allocs
//         self.clean_allocs();

//         // Then figure out the layout of the requested array. This call fails
//         // if the total size exceeds ISIZE_MAX, which is exceedingly unlikely
//         // (unless the caller calculated something wrong)
//         let layout = Layout::array::<T>(count).map_err(drop)?;

//         // Then, attempt to allocate the requested T.
//         let nnu8 = self.deref_mut().allocate_first_fit(layout)?;
//         let ptr = nnu8.as_ptr().cast::<T>();

//         // And initialize it with the contents given to us
//         unsafe {
//             for i in 0..count {
//                 ptr.add(i).write((f)());
//             }
//         }

//         Ok(HeapArray { ptr, count })
//     }

//     pub fn alloc_arc<T>(
//         &mut self,
//         data: T,
//     ) -> Result<HeapArc<T>, T> {
//         // Clean up any pending allocs
//         self.clean_allocs();

//         // Then, attempt to allocate the requested T.
//         let nnu8 = match self.deref_mut().allocate_first_fit(Layout::new::<HeapArcInner<T>>()) {
//             Ok(t) => t,
//             Err(_) => return Err(data),
//         };
//         let ptr = nnu8.cast::<HeapArcInner<T>>();

//         // And initialize it with the contents given to us
//         unsafe {
//             ptr.as_ptr().write(HeapArcInner {
//                 refcount: AtomicUsize::new(1),
//                 data,
//             });
//         }

//         Ok(HeapArc { inner: ptr })
//     }

//     /// Attempt to allocate a HeapFixedVec using the allocator
//     ///
//     /// If space was available, the allocation will be returned. If not, an
//     /// error will be returned
//     pub fn alloc_fixed_vec<T>(
//         &mut self,
//         capacity: usize,
//     ) -> Result<HeapFixedVec<T>, ()> {
//         // Clean up any pending allocs
//         self.clean_allocs();

//         // Then figure out the layout of the requested array. This call fails
//         // if the total size exceeds ISIZE_MAX, which is exceedingly unlikely
//         // (unless the caller calculated something wrong)
//         let layout = Layout::array::<T>(capacity).map_err(drop)?;

//         // Then, attempt to allocate the requested T.
//         let nnu8 = self.deref_mut().allocate_first_fit(layout)?;
//         let ptr = nnu8.as_ptr().cast::<MaybeUninit<T>>();

//         Ok(HeapFixedVec { ptr, capacity, len: 0 })
//     }
// }

// pub struct HeapArc<T> {
//     inner: NonNull<HeapArcInner<T>>,
// }

// impl<T> HeapArc<T> {
//     unsafe fn free_box(&mut self) -> FreeBox {
//         FreeBox {
//             ptr: NonNull::new_unchecked(self.inner.as_ptr().cast::<u8>()),
//             layout: Layout::new::<HeapArcInner<T>>(),
//         }
//     }
// }

// impl<T> Deref for HeapArc<T> {
//     type Target = T;

//     #[inline(always)]
//     fn deref(&self) -> &Self::Target {
//         unsafe {
//             &self.inner.as_ref().data
//         }
//     }
// }

// impl<T> Clone for HeapArc<T> {
//     fn clone(&self) -> Self {
//         let inner = unsafe { &self.inner.as_ref() };
//         inner.refcount.fetch_add(1, Ordering::Relaxed);
//         Self { inner: self.inner }
//     }
// }

// impl<T> Drop for HeapArc<T> {
//     fn drop(&mut self) {
//         let old = {
//             let inner = unsafe { &self.inner.as_ref() };
//             // TODO: I could do something more complicated, but it'll be okay for now
//             inner.refcount.fetch_sub(1, Ordering::SeqCst)
//         };

//         if old == 1 {
//             unsafe {
//                 core::ptr::drop_in_place(self.inner.as_ptr());

//                 // Calculate the pointer, size, and layout of this allocation
//                 let free_box = self.free_box();
//                 free_box.box_drop();
//             }
//         }
//     }
// }

// struct HeapArcInner<T> {
//     refcount: AtomicUsize,
//     data: T,
// }

// // Private HeapGuard methods.
// //
// // NOTE: These are NOT impls of the Deref/DerefMut traits, as I don't actually
// // want those methods to be available to downstream users of the HeapGuard
// // type. For now, I'd like them to only use the "public" allocation interfaces.
// impl HeapGuard {
//     fn deref(&self) -> &Heap {
//         &*self.heap
//     }

//     fn deref_mut(&mut self) -> &mut Heap {
//         self.heap
//     }
// }

// impl Drop for HeapGuard {
//     #[track_caller]
//     fn drop(&mut self) {
//         // A HeapGuard represents exclusive access to the AHeap. Because of
//         // this, a regular store is okay.
//         HEAP.state.store(AHeap::INIT_IDLE, Ordering::SeqCst);
//     }
// }

// pub async fn allocate<T>(mut item: T) -> HeapBox<T> {
//     loop {
//         // Is the heap inhibited?
//         if !HEAP.inhibit_alloc.load(Ordering::Acquire) {
//             // Can we get an exclusive heap handle?
//             if let Ok(mut hg) = HEAP.lock() {
//                 // Can we allocate our item?
//                 match hg.alloc_box(item) {
//                     Ok(hb) => {
//                         // Yes! Return our allocated item
//                         return hb;
//                     }
//                     Err(it) => {
//                         // Nope, the allocation failed.
//                         item = it;
//                     },
//                 }
//             }
//             // We weren't inhibited before, but something failed. Inhibit
//             // further allocations to prevent starving waiting allocations
//             HEAP.inhibit_alloc.store(true, Ordering::Release);
//         }

//         // Didn't succeed, wait until we've done some de-allocations
//         HEAP.heap_wait
//             .wait()
//             .await
//             .unwrap();
//     }
// }

