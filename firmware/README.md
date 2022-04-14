# MnemOS

This is the main repository for the MnemOS general purpose operating system.

## What is MnemOS?

MnemOS is a small, general purpose operating system designed for constrained environments.

As an operating system, it is aimed at making it easy to write portable applications, without having to directly interact with the hardware.

## Where can I find more information?

You can find information in [The MnemOS book](https://mnemos.jamesmunns.com).

## Contents

Here are the main subfolders of MnemOS:

* `kernel/`
    * This folder contains the kernel image of MnemOS
    * It is a Rust binary crate.
    * At the moment, the [Adafruit Feather nRF52840 Express] is the only supported kernel hardware platform
* `common/`
    * These are the components common between the kernel and userspace of MnemOS.
    * It is a Rust library crate.
* `userspace/`
    * This folder contains the MnemOS userspace library. It is intended to be included by any userspace applications.
    * It also includes a linker script necessary for building a correct application binary.
    * It is a Rust library crate
* `c-userspace/`
    * This folder contains C FFI bindings to the interfaces defined by the `userspace` crate.
    * It produces a static library (`.a`) and header file (`.h`), which can be used by C and other languages to interface with the Kernel
* `apps/`
    * This folder contains a number of applications used for the development of MnemOS

[Adafruit Feather nRF52840 Express]: https://www.adafruit.com/product/4062
