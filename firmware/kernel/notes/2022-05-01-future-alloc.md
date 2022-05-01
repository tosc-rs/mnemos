# Futures and Allocations

So, I want some way of doing the following:

* Userspace allocates some space that will be used for something (e.g. SPI transmission)
* Userspace fills that space
* Userspace submits that allocation to the kernel/driver to do something
* Kernel uses up that buffer, and frees it (or marks it to be returned?)

Misc thoughts:

* I should probably make the allocator owned again. We don't want a lower prio interrupt (SVC) holding the mutex lock and being interrupted, preventing a higher prio interrupt (USB) from being able to execute.
* BUT, I still sort of want allocs to be able to quickly free themselves, which becomes impossible outside of rtic context.
* This also has odd implication of the machine traits: do I need to always provide an allocator? This sort of exposes implementation details to the API...

Okay, so how about this:

* One syscall to allocate. This does:
    * the alloc (may fail)
    * reserves space in the driver for pending items (may fail) - driver increases refcount
    * returns the alloc in 'waiting on app' state
* App does the filling
* App can `drop()`, which cancels the request, or `send()`, which moves the request to 'waiting on driver' state
    * TODO: How to pend the driver to process the pended item? Periodic check?
    * TODO: App can hold on to a SendHandle, in case it wants to check when the transfer actually completed?
* Driver scans through pending items:
    * If dropped, also drop to free the refcnt
    * If ready, put into transmit queue
    * Do sending...
