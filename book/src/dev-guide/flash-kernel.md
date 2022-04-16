# Flashing the Kernel

The following steps can be used to build and flash the kernel.

Make sure you've taken a look at the [required hardware](./hardware.md) and [required software](./software.md) pages first.

## 1. Attach the debugger to the CPU

You'll need to attach your SWD debugger of choice to your nRF52840 Feather board.

If you are using the top connector, you should only need to connect the debugging cable.

If you are using the bottom test points, make sure you have Ground (GND), SWDIO, and SWCLK connected.

## 2. Power on the devices

Connect the debugger and Feather to your PC. The order does not matter.

## 3. Flash with probe-run

From the top of the repository, move to the `kernel` folder.

```sh
cd firmware/kernel
```

Then, flash the firmware with the following command:

```sh
cargo run --release
```

After building the kernel, you should see roughly the following output:

```
cargo run --release
...
   Compiling mnemos-common v0.0.1 (/home/james/hardware-v6/pellegrino/firmware/common)
   Compiling mnemos v0.0.1 (/home/james/hardware-v6/pellegrino/firmware/kernel)
    Finished release [optimized + debuginfo] target(s) in 3.71s
     Running `probe-run --chip nRF52840_xxAA target/thumbv7em-none-eabihf/release/mnemos`
(HOST) INFO  flashing program (20 pages / 80.00 KiB)
(HOST) INFO  success!
────────────────────────────────────────────────────────────────────────────────
Hello, world!
└─ mnemos::app::idle @ src/main.rs:180
...
```

## 4. You're done!

At this point, you have flashed the kernel.

If you press Control-C, the kernel will be halted. However if you un-plug/replug the power to the CPU, the kernel will boot and run again.

If you'd like to build your own user applications, you can move on the the [Building Applications](./build-apps.md) section.

If you'd like to upload or run existing applications, you can move on to the [User's Guide](./../user-guide/intro.md) section.
