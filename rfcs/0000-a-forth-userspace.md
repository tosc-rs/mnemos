# RFC - Forth Userspace

## Goal

In order to make early prototyping and iterating on the design of mnemos **easier**, this RFC proposes the
addition of a single kind of "userspace" environment, a `forth` vm that can act as a program
or interactive shell interface. This RFC also proposes a minimal set of capabilities and interfaces
necessary to make this possible.

## Background

mnemos has some design around what it means to launch kernel services, but has no kind of userspace
separate from the kernel itself. Designing this "properly" requires a lot of subtle decisions to be
made, so this rfc proposes to ignore these problems until we are much smarter people later in the
project.

James has been working on an extensible forth vm written in Rust, and recently Eliza has been helping
to expand the vm, including the ability to have async host-provided intrinsic functions. While limited,
this environment is likely suitable for two main things:

* "gluing together syscalls" to test out kernel capabilities and bring up hardware support
* Crossing the "embedded system"/"computer" rubicon, allowing for runtime developed programs

## The proposal

* The OS will spawn a single userspace entity at boot time
    * This entity will be a cooperative async task running an interactive forth vm
    * It will be capable of interacting with the userspace/kernel interface (sending/receiving kernel messages)
* This "task 0" forth environment will be capable of spawning other userspace tasks
    * It can only spawn other forth vms, running their own separate environments
    * When spawning a new task, a snapshot of the current task's dictionary will be made and shared with the child environment
    * All forth vm tasks will be cooperatively scheduled

## Details

### Changes to `forth3` - the current forth vm

* We'll need to be able to provide a cow-like, reference counted, linked list of previous dictionary fragments
* We'll need some way of determining "write-like" behaviors that will cause a deepcopy
    * like obtaining the address of a variable
    * todo: is this all we need it for? what about variable addrs in variables?
* We'll need some way of allocating + deepcopying dictionary fragments

### Changes to `mnemos`

* We'll need to add some kind of pattern/interface for managing cooperative userspace tasks
    * We will probably NOT enforce preemptive multitasking for now
    * We will probably NOT enforce memory protection/mapping for now
    * userspace tasks yield when they feel like it, kernel then serves each tasks's executor like a metaexecutor
    * userspace tasks stay sleeping as long as nothing in the kernel sends them messages
* We'll need some sort of "trampoline" code to bolt the forth3 vm to whatever userspace tasks look like
    * Start the vm, get it running
    * Alloc dictionary fragments as necessary
    * interface with the OS for certain calls like "spawn" via intrinsic functions
    * handle stdin/stdout
* We'll need to add some kind of allocation service for bigpages
    * Define a service
    * Needs a way for allocating unshared r/w pages
    * Needs a way to exchange unshared r/w pages for shared r/o pages
    * These would *normally* be used to add memory slabs to the userspace heap allocator
* We need some sort of "spawn" service
    * ONLY for spawning forth vms now
    * good practice for later, more general, "spawn" interface
* Call the "spawn" service with our `task-id-0`, giving it some reasonable stdin/stdout

## Other motivations and thoughts

* Having something useful to play with NOW is more important than designing something perfect
* We will learn a lot about what "spawn" needs to do with a relatively restricted interface
    * We own the VM, we can force it to cooperate before we have preemption
    * We can impl memory safety checks (probably not fortified, but good for "oops" checking) before we have MMU support
    * All code is interpreted with the same vm engine, no need to worry about relocations, elf loading, or memory mapping yet
