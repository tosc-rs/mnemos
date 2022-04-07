# C userspace for Anachro

This is a simple project to expose syscalls for C FFI.

To generate up-to-date libraries, headers, and linker scripts, move to this directory, then run:

```shell
./generate.sh
```

See the contents of `generate.sh` for installation instructions.

When building your application, you MUST use the provided `link.x` as a linkerscript. Optionally, you can modify `stack.x` to set the stack size (defaults to 16KiB). Your program + stack size must be less than 128 KiB.

You must ALSO provide an entrypoint, with the signature:

```c
void entry(void);
```

This function must never return.

After building your embedded binary, you must generate a binary file to be uploaded. You can do this with the following command:

```shell
arm-none-eabi-objcopy \
    -O binary \
    PATH/TO/YOUR/program.elf \
    ./your-application.bin
```

## Loading the program

At the moment, I don't have dynamic loading working yet. The binary is compiled in as part of the kernel (then loaded to RAM by the kernel).

To make this happen for your app, you will need to copy your binary file to the following location:

```
pellegrino/firmware/kernel/appbins/your-application.bin
```

And then modify the kernel source to include that image.

At the time of this writing, the relevant file is:

```
pellegrino/firmware/kernel/src/main.rs
```

And you should update this line:

```diff
- static DEFAULT_IMAGE: &[u8] = include_bytes!("../appbins/p1echo.bin");
+ static DEFAULT_IMAGE: &[u8] = include_bytes!("../appbins/your-application.bin");
```

You can then flash and run the kernel (with your application) using the command:

```shell
# NOTE: Requires the `probe-run` tool. Install with `cargo install probe-run`
cargo run --release
```

Then hopefully you should see your program working! In order to connect to the virtual serial ports, you will need to run the `crowtty` tool.

```shell
cd pellegrino/tools/crowtty
cargo run --release
```

Then you can connect using a tool like `ncat`:

```shell
stty -icanon -echo && ncat 127.0.0.1 $PORT
```

Where `$PORT` is the virtual port number plus 10000 (e.g. 10000 for virtual port 0, 10001 for virtual port 1). If you need more than ports 0 and 1, you may need to modify the crowtty binary.
