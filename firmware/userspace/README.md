# MnemnOS Userspace Library

This Rust library (or crate) serves as the primary interface for userspace applications to the services provided by the kernel.

It contains a couple of important things:

## Linker Scripts

The userspace contains two linkerscripts:

`link.x` - the main linker script, which tells the compiler and linker how to properly lay out your application so that it can be loaded by the kernel. You typically should not ever modify this file, and it will be copied automatically into the build directory via the included `build.rs` script.

If you have experience with embedded Rust development, this is similar to how `cortex-m-rt` works.

You WILL need to configure your application project to use this linkerscript, typically by creating a `.cargo/config.toml` file in your project.

For an example (that you can copy), see the [`.cargo/config.toml` of the `app-loader` application](../apps/app-loader/.cargo/config.toml).

The second linkerscript is `stack.x`. You should copy this file into your application project.

By default, the linkerscript will be configured to use 16KiB of space as a stack for your program.

This can be modified by editing the `stack.x` file (in your project) to change the amount of space to be allocated as stack memory. For example, to set the stack size to 64KiB, you would add this line to your project's `stack.x`:

```
_stack_size = 0x10000;
```

Again, if you are familiar with embedded rust, this is similar to the `device.x` file you are expected to provide in each project.

## Library Code

The other main component of the `userspace` crate are the types and functions necessary to interact with the kernel.

To view the documentation for the provided interfaces and types, from the `userspace` folder, you can run:

```shell
cargo doc --open
```

Which will open the developer API documentation for these functions.
