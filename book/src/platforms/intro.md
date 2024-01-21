# platform support

mnemOS attempts to make the bringup process for supporting a new hardware
platform as straightforward as possible. The core cross-platform kernel is
implemented as a [Rust library crate][kernel-lib], which platform
implementations depend on. The platform implementation, which builds the final
mnemOS kernel binary artifact for that platform, is responsible for performing
hardware-specific initialization, [initializing drivers] for hardware devices
provided by that platform, and [running the kernel]. In addition, the kernel
crate provides a set of cross-platform [default services], which may be run on
any hardware platform.

## supported platforms

Currently, mnemOS has [in-tree platform implementations][platforms] for several
hardware platforms, with various degrees of completeness.

The following platforms are "fully supported" --- they are capable of running
the complete mnemOS operating system, with all core functionality implemented:

- **[Allwinner D1]**, a [single-core, 64-bit RISC-V SoC from
  Allwinner](https://linux-sunxi.org/D1). The following D1 single-board
  computers are currently supported:
    - [MangoPi MQ Pro](https://linux-sunxi.org/MangoPi_MQ-Pro), a board in the
      Raspberry Pi Zero form factor
    - [Sipheed Lichee RV](https://linux-sunxi.org/Sipeed_Lichee_RV), a
      system-on-module capable of being connected to carrier boards using a dual
      M.2 slot design
- **Simulators**, which run an interactive mnemOS kernel as a userspace process in a
  development machine. These include:
    - [Melpomene](https://mnemos.dev/melpomene/), a desktop simulator for
      running the mnemOS kernel locally
    - [Pomelo](https://mnemos.dev/pomelo/), a WebAssembly browser-based
      simulator. A hosted version of Pomelo is available at
      <https://anatol.versteht.es/mlem/>

Other platform implementations are less complete, and undergoing active
development:

- **[ESP32-C3]**, a [small, WiFi-enabled 32-bit RISC-V
  microcontroller][c3-website] from Espressif. This port is intended primarily
  to support the use of the ESP32-C3 as a WiFi co-processor for an Allwinner D1
  system; see [RFC 0196: WiFi Buddy Interface Design][rfc0196] for details.
- **[x86_64]**, for 64-bit x86 (amd64) desktop and server platforms. The x86_64
  implementation is in the early phases of development, and most mnemOS
  functionality is not yet available.

[kernel-lib]: https://mnemos.dev/doc/kernel
[initializing drivers]: https://mnemos.dev/doc/kernel/#initialization-phase
[running the kernel]: https://mnemos.dev/doc/kernel/#running-mode
[default services]:
    https://mnemos.dev/doc/kernel/struct.kernel#method.initialize_default_services
[platforms]: https://github.com/tosc-rs/mnemos/tree/main/platforms
[Allwinner D1]: https://mnemos.dev/allwinner-d1/
[esp32-c3]: https://mnemos.dev/esp32c3-buddy
[c3-website]: https://www.espressif.com/en/products/socs/esp32-c3
[rfc0196]: ../rfcs/0196-wifi-buddy-interface.md
[x86_64]: https://mnemos.dev/x86_64