# Required (and Recommended) Software Tools

## Definitely Required Tools

In order to develop the MnemOS Kernel or Userspace applications, you will DEFINITELY need the following tools installed:

* The Rust Compiler (at least v1.59.0, though this may change in the future)
    * See the [Rust installation instructions](https://www.rust-lang.org/tools/install) for how to do this
* The `thumbv7em-none-eabihf` target, needed for compiling for the nRF52840
    * If you have `rustup`, this can be accomplished with `rustup target add thumbv7em-none-eabihf`
* The `probe-run` tool, used for flashing and debugging the kernel
    * See the [Probe Run Documentation](https://github.com/knurling-rs/probe-run#installation) for installation instructions
* A version of the `objcopy` binutil, used for preparing MnemOS applications. You will need a version that supports Cortex-M4F applications. This can typically be obtained through one of the following ways:
    * The [cargo-binutils](https://github.com/rust-embedded/cargo-binutils) package
    * The Arm GNU Toolchain, either from [Arm's main website](https://developer.arm.com/tools-and-software/open-source-software/developer-tools/gnu-toolchain), or your system's package manager
    * A "multiarch" build of the GNU toolchain, typically provided by your system's package manager

## Practically Required Tools

The following tools are generally required, but are a bit more flexible in terms of the specific tool you use.

* You will need a tool that allows you to use a TCP port as a terminal interface. On my Linux PC:
    * I use `stty` (to configure the terminal not to echo characters or buffer lines - this is probably installed on a typical linux system)
    * I use the [ncat](https://nmap.org/ncat/) tool to provide the TCP-port to terminal adapter
    * I am unsure what tools will work on OSX/Windows, but I am happy to help you figure it out! Feel free to open an issue on the repo, or reach out to me via Twitter or Matrix.

## Useful (But Not Required) Tools

Other tools I find helpful for development include:

* Other parts of the Arm GNU toolchain, including:
    * `[arm-none-eabi]-nm`, for viewing compiled code component sizes and locations in memory
    * The [Cargo Bloat](https://github.com/RazrFalcon/cargo-bloat) tool, for similar information
    * `[arm-none-eabi]-gdb`, in case step-through debugging is necessary
* A GDB Server, such as `JLinkGDBServer` or `OpenOCD`, for use with `[arm-none-eabi]-gdb`.
