# MnemOS

This is the main repository for the MnemOS general purpose operating system.

## What is MnemOS?

MnemOS is a small, general purpose operating system designed for constrained environments.

As an operating system, it is aimed at making it easy to write portable applications, without having to directly interact with the hardware.

As a user or developer of MnemOS, you are likely to run into two main parts, the "kernel" and "userspace".

The kernel handles hardware-level operations, including memory management, event handling for hardware and driver events, and isolation of userspace from the hardware.

The userspace is where applications run. Applications are provided a standard interface from the kernel, that allows them to perform operations like reading or writing to a serial port, or reading or writing to a block storage device (sort of like a hard drive).

At the moment, MnemOS is not a multitasking operating system, so only one application (and the kernel) can run at any given time.

## Where does the name come from/how do I pronounce it?

"MnemOS" is named after [Mnemosyne](https://en.wikipedia.org/wiki/Mnemosyne), the greek goddess of memory, and the mother of the 9 muses. Since one of the primary responsibilities of an OS is to manage memory, I figured it made sense.

In IPA/Greek, it would be [`mnɛːmos`](https://en.wikipedia.org/wiki/Help:IPA/Greek).

To listen to someone pronounce "Mnemosyne", you can listen to [this youtube clip](https://www.youtube.com/watch?v=xliDJCBxHAo&t=939s), and pretend he isn't saying the back half of the name.

If you pronounce it wrong, I won't be upset.

## How do I use a MnemOS based computer?

When you first power on the computer, the kernel will start, and initialize any hardware components.

It will then start the [app-loader program](./apps/app-loader/README.md), which will allow you to query, select, upload, or boot applications into the attached block storage device.

At the moment, MnemOS only supports communication over a USB CDC-ACM serial port. This connection allows for multiple "virtual" serial ports to be shared over a single connection. Typical "user stdio" interactions, such as using the CLI interface of the app-loader program happen on virtual port 0, while binary communications, such as uploading applications, happen on virtual port 1.

You can use the [crowtty tool](../tools/crowtty/README.md) to obtain a console to your MnemOS computer, and use the [dumbloader tool](../tools/dumbloader/README.md) to upload new applications to your MnemOS computer's storage.

## How do I write applications for a MnemOS based computer?

MnemOS provides libraries that can be included in your project to create an application.

The primary development environment is in the Rust programming language.

To create a MnemOS application in Rust, create a new bare metal application, include the [`userspace` crate](./userspace/README.md). This crate includes the necessary linker script, as well as library functions for accessing the features (like reading or writing to the serial console) provided by the kernel.

MnemOS also provides a [`c-userspace` library](./c-userspace/README.md), which can be used for interacting with the MnemOS kernel from languages other than Rust.

Once you have created a MnemOS application, it can be uploaded using the app-loader program described above.

## How do I modify or debug the MnemOS kernel?

To modify or debug the kernel, you will need a SWD adapter attached to the main CPU of your computer.

The [kernel project](./kernel/README.md) provides tooling based on the [probe run tool](https://github.com/knurling-rs/probe-run), and offers kernel logging capabilities via [defmt](https://defmt.ferrous-systems.com/).

Programming and debugging of the kernel is performed by running `cargo run --release` inside of the `kernel/` folder.

At the moment, it is not possible to update the kernel without an attached debugger.

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
