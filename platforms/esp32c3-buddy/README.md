# MnemOS ESP32-C3 WiFi Buddy

This directory contains the MnemOS firmware for the [ESP32-C3] WiFi Buddy.

## Target Boards

The currently targeted [ESP32-C3] development boards are the [Seeedstudio XIAO
ESP32-C3][xiao] and its cousin the [Adafruit QT Py ESP32-C3][qtpy]. These boards
are more or less the same --- they are pinout-compatible and can be flashed over
USB-C.

The only substantial difference is that the Adafruit board includes a
WS2812 "NeoPixel" RGB LED and a Stemma QT JST connector for I²C devices. Neither
of these components are required for the MnemOS WiFi Buddy application, so
either board can be used interchangeably.

## Getting started with MnemOS on the ESP32-C3

### Building

> **Note**
>
> This crate is its own Cargo workspace. This is in order to avoid
> blowing away artifacts for host tools cached in the main workspace when
> building the MnemOS binary for a target.

To build for the [ESP32-C3] platform, either build from within the
`platforms/esp32c3-buddy` directory, or use the [`just build-c3` Just
recipe][just].

The two supported ESP32-C3 dev boards are pinout-compatible, but route different
pins on the ESP32 to the pins on the dev board. Therefore, this crate contains
separate Cargo bin targets for each supported dev board, which configure the I/O
pin assignment to match the target devboard before calling into shared code:

* `qtpy`: MnemOS for the [Adafruit QT Py ESP32-C3][qtpy]
* `xiao`: MnemOS for the [Seeedstudio XIAO ESP32-C3][xiao]

The `just build-c3` recipe takes a required argument to select which bin target
is built. For example:

```console
$ just build-c3 qtpy   # builds MnemOS for the Adafruit QT Py ESP32-C3
$ just build-c3 xiao   # builds MnemOS for the Seeedstudio XIAO ESP32-C3
```

### Flashing & Running

ESP32-C3 dev boards can be flashed over USB using [`cargo-espflash`]. To flash
an ESP32-C3 board with the MnemOS firmware, either run `cargo espflash flash`
from within this directory, or run the [`just flash-c3` Just recipe][just] from
anywhere in the MnemOS repository.

Like `just build-c3`, the target board must be provided:

```console
$ just flash-c3 qtpy   # build and flash the Adafruit QT Py ESP32-C3
$ just flash-c3 xiao   # build and flash the Seeedstudio XIAO ESP32-C3
```

> **Note**
>
> In order to flash an ESP32-C3 board, the [`cargo-espflash`] executable
> must be installed. The `just flash-c3` Just recipe will check if
> `cargo-espflash` is present, and prompt to install it if it is not found.

If everything worked successfully, you should see output similar to this:

```console
$ just flash-c3 qtpy
       Found cargo-espflash
cd platforms/esp32c3-buddy && cargo build --release
    Finished release [optimized] target(s) in 0.04s
cd platforms/esp32c3-buddy && cargo espflash flash --monitor
[2023-07-28T16:40:37Z INFO ] Serial port: '/dev/ttyACM0'
[2023-07-28T16:40:37Z INFO ] Connecting...
[2023-07-28T16:40:38Z INFO ] Using flash stub
    Finished dev [unoptimized + debuginfo] target(s) in 0.04s
Chip type:         esp32c3 (revision v0.3)
Crystal frequency: 40MHz
Flash size:        4MB
Features:          WiFi, BLE
MAC address:       34:b4:72:ea:44:18
App/part. size:    209,760/4,128,768 bytes, 5.08%
[00:00:00] [========================================]      13/13      0x0
[00:00:00] [========================================]       1/1       0x8000
[00:00:01] [========================================]      67/67      0x10000
[2023-07-28T16:40:41Z INFO ] Flashing has completed!
```

[ESP32-C3]: https://www.espressif.com/en/products/socs/esp32-c3
[xiao]: https://www.seeedstudio.com/Seeed-XIAO-ESP32C3-p-5431.html
[qtpy]: https://www.adafruit.com/product/5405
[just]: ./../../../justfile
[`cargo-espflash`]: https://github.com/esp-rs/espflash/blob/main/cargo-espflash/README.md

## License

[MIT] + [Apache 2.0].

[MIT]: ./LICENSE-MIT
[Apache 2.0]: ./LICENSE-APACHE
