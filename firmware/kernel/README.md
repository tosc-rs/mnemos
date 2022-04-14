# MnemOS Kernel

This is the kernel for the MnemOS general purpose operating system.

At the moment, the [Adafruit Feather nRF52840 Express] is the only supported kernel hardware platform. Support for other targets and architectures is planned.

[Adafruit Feather nRF52840 Express]: https://www.adafruit.com/product/4062

The kernel is built on top of [RTIC](https://rtic.rs), a concurrency framework for Rust.

For more information about the kernel, refer to:

* The [Kernel Chapter of the MnemOS book] for theory and design information
* The [Kernel API documentation] for software documentation

[Kernel Chapter of the MnemOS book]: https://mnemos.jamesmunns.com/components/kernel.html
[Kernel API documentation]: https://docs.rs/mnemos/latest/kernel/
