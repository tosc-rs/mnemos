# Mnemos' async allocator

This project is the async allocator layer for [MnemOS].

It serves more as a `liballoc` replacement than a `malloc` replacement - it doesn't actually handle "raw" allocations at all, and instead (currently) leaves that up to the underlying allocator, at this point only [linked_list_allocator].

[MnemOS]: https://mnemos.jamesmunns.com
[linked_list_allocator]: https://docs.rs/linked_list_allocator/

## How does it work?

> NOTE: Not everything described here has been ported over from `MnemOS`. You can see some of this
> behavior in the [pre-export] version, before this was broken out into a standalone crate.

[pre-export]: https://github.com/jamesmunns/pellegrino/blob/1ad0f53e4d23a4a8683cce80f8d239504bac440c/oaiu/alloc/src/lib.rs

When you go to allocate something, you don't get back a `T` or a `Result<T>`, you get back an `impl Future<Output = T>`.
If there is space in the allocator, and it is not currently locked doing something else, this will resolved on the first
poll. If not, the waiter is placed into an [intrusive waitqueue], which will be woken the next time a free occurs, and there
might be space.

[intrusive waitqueue]: https://mycelium.elizas.website/maitake/wait/struct.waitqueue

## But why?

This allocator is designed for memory constrained systems. In many cases, especially for small systems with limited memory,
"OOM" is a temporary state of things, rather than a foregone conclusion.
By using async/await to allow waiting a little bit, you might be able to resolve this once a couple of chonky allocs have cleared.

Or you'll deadlock. But hey, that's the problem of the OS or the executor, not the allocator!

## But wait, `drop` isn't async. How do you handle that?

Good question! When a `drop` would occur, we try to immediately lock the allocator, and free the memory. If this fails, we
stick the allocation in a [lock-free mpscqueue]. It will then be processed at the allocator's next convenience, such as
before allocating the next item, or at some kind of periodic "cleanup" time.

[lock-free mpscqueue]: https://docs.rs/cordyceps/latest/cordyceps/struct.MpscQueue.html

## Why would the allocator be "locked"? `malloc` doesn't need that

Since this is an allocator designed **for** operating systems, we don't necessarily have the benefits of threads to make
forward progress or to yield when the allocator is busy. The allocator itself is not (currently) thread safe, which means
that we need to handle the case where we (accidentally or intentionally) are holding the allocator mutex and an allocated
piece of data is freed

## But wait, what if the queue is full?

It can't be! We use an *intrusive* queue, which means that the space to store the queue elements are in the elements themselves!
When we do the allocation, there is a small header that allows us to chain it to a linked list (in a thread safe way), which means
that the free list is only limited to the number of allocations that actually exist!

## But doesn't that add a lot of overhead?

No! We use the power of `union`s, because when an allocation is freed, it no longer needs that juicy space to contain data anymore,
so we just stick the linked list header there instead! This means that the only overhead over an allocation itself is a single
pointer - a pointer back to the allocator that created this node. (unless you are allocating something smaller than two pointers,
then yes, there will be a bit of overhead, but why are you doing that?)

## That sounds terribly unsafe!

Oh it is! Egregiously! See the reviews below:

> "this code is like a fractal footgun forest
>
> - [@dirbaio]

[@dirbaio]: https://twitter.com/Dirbaio/

However, we have the power of [miri](https://github.com/rust-lang/miri) to check our reckless use of `unsafe`, which currently
gives us a clean bill of health! Try it yourself:

```shell
MIRIFLAGS="-Zmiri-disable-isolation -Zmiri-strict-provenance -Zmiri-tag-raw-pointers -Zmiri-ignore-leaks" cargo +nightly miri test
     Running tests/smoke.rs (target/miri/x86_64-unknown-linux-gnu/debug/deps/smoke-d242f69140f2a6cb)

running 2 tests
test basic ... ok
test basic_arr ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

To be fair, there's not a LOT of testing yet, but fundamental actions like allocating, freeing (with and without the lock available),
and using some silly Dynamically Sized Type allocations for arrays, it still hasn't made Miri upset yet!

## Why didn't you just use `alloc`?

I dunno, that didn't sound as fun.
