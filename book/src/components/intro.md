# Parts of mnemOS

mnemOS conceptually is broken up into three main parts:

* the **kernel**, which provides resources like an allocator, an async executor/scheduler, and a registry of active/running drivers
* the **drivers**, which are async tasks that are responsible for all other hardware and system related functionality
* the **user programs**, which use portable interfaces to be able to run on any mnemOS system that provides the drivers it needs.
