# Building User Applications

The following guide walks through the process to make a minimal userspace application.

This guide assumes you are (vaguely) familiar with building and running `#![no_std]` Rust applications.

## Create a new project

You will need to create a new project with `cargo`, and move into the directory.

```sh
cargo new --bin demo-app
     Created binary (application) `demo-app` package
cd ./demo-app
```

## Add Required Files

We will need to create a couple of files needed to correctly build a MnemOS application.

The first is the `.cargo/config.toml` file. You'll need to create the folder and file inside the project folder. For example:

```sh
mkdir ./.cargo
touch ./.cargo/config.toml
```

Inside that file, you should add the following contents:

```toml
[target.'cfg(all(target_arch = "arm", target_os = "none"))']
rustflags = [
  # Use the MnemOS linker script
  "-C", "link-arg=-Tlink.x",
  # This is needed if your flash or ram addresses are not aligned to 0x10000 in memory.x
  # See https://github.com/rust-embedded/cortex-m-quickstart/pull/95
  "-C", "link-arg=--nmagic",
]

[build]
# MnemOS only supports thumbv7em-none-eabihf (or above) currently.
target = "thumbv7em-none-eabihf" # Cortex-M4F and Cortex-M7F (with FPU)
```

Save and exit this file.

Next, we will need to add a stack configuration file at the root of your project (e.g. in the `demo-app/` folder).

For example:

```sh
touch ./stack.x
```

> # ⚠️ IMPORTANT
>
> You **MUST** have a `stack.x` file, even if it is empty. Otherwise you will get a linker error.

You'll probably want to place the following contents in the `stack.x` file:

```text
/* You must have a stack.x file, even if you    */
/* accept the defaults.                         */

/* How large is the stack? Defaults to 16KiB    */
/*                                              */
/* _stack_size = 0x4000;                        */

/* Where should the stack start? Defaults to    */
/* _stack_size bytes after the end of all other */
/* application contents (__eapp), which is four */
/* byte aligned.                                */
/*                                              */
/* _stack_start = __eapp + _stack_size;         */
```

If you'd like to change the `_stack_size` or `_stack_start` variables, you can uncomment or add lines as follows:

```text
/* You must have a stack.x file, even if you    */
/* accept the defaults.                         */

/* How large is the stack? Defaults to 16KiB    */
/*                                              */
/* _stack_size = 0x4000;                        */

/* Set the stack size to 64KiB                  */
_stack_size = 0x10000;

/* Where should the stack start? Defaults to    */
/* _stack_size bytes after the end of all other */
/* application contents (__eapp), which is four */
/* byte aligned.                                */
/*                                              */
/* _stack_start = __eapp + _stack_size;         */

/* Trivially change the _stack_start for demo   */
/* reasons. You probably should never do this.  */
_stack_start = __eapp + _stack_size + 4;
```

### Add the `userspace` library as a Cargo dependency

You'll need to add the `userspace` library as a dependency. You will
need a version that matches the version of the kernel you are using.

The `userspace` library is published to crates.io as the [`mnemos-userspace` crate](https://crates.io/crates/mnemos-userspace).

For more information about the `userspace` library, refer to the [Library Documentation on docs.rs](https://docs.rs/mnemos-userspace)

In your `Cargo.toml`:

```toml
# Using crates.io - Check the version is correct!
[dependencies]
mnemos-userspace = "0.1.0"

# OR - using git (don't do both!)
[dependencies.mnemos-userspace]
version = "0.1.0"
git = "https://github.com/jamesmunns/pellegrino"
rev = "main"
```

### Update your `main.rs`

The following is a minimal template you can use for your `main.rs` file. Delete the existing contents, and replace
it with the following code:

```rust
// Your application will generally be no_std, MnemOS does not currently provide
// a version of the standard library
#![no_std]

// Your application will generally need the no_main attribute (similar to
// embedded rust programs) - as we do not use Rust's built-in main function,
// and instead use `entry() -> !`
#![no_main]

/// Even if you use no system calls, you should probably include the
/// userspace library as shown here, to ensure the panic handler (and
/// other necessary components) are linked in.
///
/// Note: Although the crate name is `mnemos-userspace`, it is imported
/// as just `userspace`.
use userspace as _;

/// Though in this example, we will use a couple of system calls for
/// demonstration purposes.
use userspace::common::porcelain::{serial, time};

/// The entry point function MUST:
///
/// * Be declared with the #[no_mangle] attribute
/// * Must never return
#[no_mangle]
fn entry() -> ! {
    loop {
        // Note: You probably should handle errors, but this is a demo.
        serial::write_port(0, b"Hello, world!\r\n").ok();
        time::sleep_micros(500_000).ok();
    }
}
```

### Build and Create a binary file

You should now be able to compile your application.

```sh
cargo build --release
...
   Compiling demo-app v0.1.0 (/tmp/demo-app)
    Finished release [optimized] target(s) in 8.31s
```

Once it has compiled, we can also make a binary file that can be uploaded using the `app-loader` tool.

Here, we take the produced `demo-app`, in `target/thumbv7em-none-eabihf/release`, and place the binary file in `./target`, so it won't be version controlled.

```sh
arm-none-eabi-objcopy \
    -O binary \
    target/thumbv7em-none-eabihf/release/demo-app \
    ./target/demo-app.bin
```

Now you can follow the steps in the [Uploading and Running](./../user-guide/upload-and-run.md) section of the [Users Guide](./../user-guide/intro.md) to see how to upload and run your new project.
