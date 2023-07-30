# RFC - ESP32 Wifi Buddy Interface Design

## Goal

To bring wireless networking to MnemOS Beepy devices, an [ESP32-C3] SoM will be
integrated to serve as a WiFi coprocessor. This device will run MnemOS-based
firmware, which we are referring to as "WiFi Buddy". See [#121] for details on
the rationale for choosing the ESP32-C3 as the WiFi device, and for discussion
of the overall design for ESP32-C3 integration.

The goal of this document is to propose an overall design for the hardware and
software interfaces between MnemOS on the CPU and MnemOS on the WiFi Buddy.

## Background

### Hardware

The WiFi Buddy hardware device is a [Seeedstudio XIAO] or [Adafruit
QT Py] ESP32-C3 development board. These are development boards for the ESP32-C3
which are pinout compatible, so they can be used interchangeably. The ESP32-C3
board will be treated as a system-on-module (SoM) and integrated with the MnemOS
Beepy shield using pin headers (or potentially soldered onto the board).

The following pins are broken out by the WiFi Buddy hardware and can be used for
communication between the CPU and WiFi Buddy:

- One SPI interface (`MOSI`, `MISO`, and `SCK`, and a GPIO as chip select)
- One I²C interface (`SDA` and `SCL`)
- One UART (`TX` and `RX`)
- Four additional GPIO pins (minus one if used as chip sel)

For development, @jamesmunns has designed a [development jig PCB][jig] which
marries a WiFi Buddy with a XIAO RP2040 board. This board will be used to
develop WiFi Buddy firmware with Melpomene replacing the D1 as the "CPU".

With the dev board, we'll be able to access:

- 4-pin SPI
- 2-pin UART
- 2 signal GPIOs (or 4, if we don't use the UART pins for UART)

This limits the number of potential IRQ pins in the Melpomene dev configuration,
if the UART is used.

### Software

Communication between the CPU and the WiFi Buddy will include both a _control
path_ and a _data path_. Data path communication refers to the actual network
frames exchanged between the WiFi Buddy and the CPU, while control path
communication consists of messages that configure the operation of the WiFi
Buddy.

Potential control path communication may include:

- The CPU asking the WiFi Buddy to scan for WiFi access points
- The CPU asking the WiFi Buddy to connect to or disconnect from a WiFi access
  point
- The ability for the CPU to reset the WiFi Buddy (?)
- Information about the network interface, such as signal strength
- Errors and other diagnostic information reported by the WiFi Buddy
- `trace-proto`-formatted logs from the WiFi buddy (?)
- Potentially, messages related to Bluetooth Low Energy operation (out of scope
  for this document)

It may also be desirable for the CPU to eventually be able to update the
firmware on the WiFI buddy. However, for now, both of the target WiFi Buddy
devices can be flashed over USB, so the MnemOS CPU being able to update the WiFi
Buddy firmware is not a high priority at this time.

Data path communication will be performed over the SPI bus, as it's the
highest-bandwidth link available with this hardware. In [#121], we have
tentatively concluded that the firmware will use a "dumb WiFi Buddy" design,
where TCP or any other higher-level network protocols are implemented on the
CPU, and the CPU and WiFi Buddy communicate by exchanging MAC-layer [802.3]
Ethernet frames.

## Design Questions

Based on the background information discussed above, we can begin to enumerate
design questions for the WiFi Buddy interface:

- **Is control path communication in-band or out of band?** If data path frames
  are exchanged between the host CPU and the WiFi Buddy over the SPI bus, do we
  also exchange control messages on the same bus? Or, are control messages sent
  using another interface, such as I²C?

  Using a single interface for both control and data path communication is
  conceptually simpler, and has the advantage of a clear and obvious ordering
  between data and control messages. It also means that we don't necessarily
  need to implement drivers for all the interfaces available on the ESP32-C3.
  Avoiding the need to synchronize between messages on two different
  communication links is a significant advantage.

  Downsides of in-band control messages include that we must introduce some
  additional form of tagging of data sent on the SPI bus, in order  to indicate
  what bytes are part of a control message and what bytes are actual network
  data. This may introduce some additional overhead. On the other hand, if we
  want to eventually include Bluetooth Low Energy as well as WiFi, we will need
  the ability to indicate what data is an Ethernet frame and what is BLE,
  anyway, so we'll already be introducing some overhead for tagging data.

- **Do we want the CPU to be able to receive `trace-proto` from WiFi Buddy?**
  Both the CPU and WiFi Buddy devices will emit debugging information using
  [`mnemos-trace-proto`]. Do we want `trace-proto`-formatted logs from the WiFi
  Buddy to be exposed to the CPU, so that it can proxy them to an attached debug
  host? Or, is using the WiFi Buddy SoM's onboard USB-serial hardware sufficient
  for debugging purposes?

  * If we do forward `trace-proto` to the CPU, is this done over the same SPI
    link as the modem data path? Or do we use the WiFi Buddy's UART pins?

- **What control messages are necessary?** Eventually, we'll need to enumerate
  what control messages will need to be exchanged in order to implement the
  required functionality.

- **How does the WiFi Buddy signal that data is ready?** Since the CPU is the
  SPI bus controller, it will always be responsible for initiating communication
  with the WiFi Buddy. If SPI is the *only* interface between the CPU and WiFi
  Buddy, then there is no mechanism for the WiFi Buddy to proactively inform the
  CPU that data is ready, requiring the CPU to busy-poll for WiFi frames. This
  is unfortunate, because the overall design of MnemOS is event-driven, and we
  would like to be able to put the CPU in a low-power state when it is idle.
  Therefore, the WiFi buddy will need to be able to raise an interrupt to the
  CPU to signal that data is ready and that an operation has completed.

  * The simplest design is a single interrupt line raised by the WiFi Buddy
    whenever it wants the CPU's attention. Is this sufficient? Or would there be a
    benefit in having separate IRQ lines for "RX data is ready" and "TX
    done"/"operation done"?

  * The WiFi buddy hardware has 4 GPIO pins that could potentially be used for
    interrupts. How many of the CPU's GPIO pins do we want to use for WiFi Buddy
    IRQ lines?

- **What additional constraints are necessary if we want to support BLE?** A
  design for Bluetooth Low Energy support in the WiFi Buddy interface is out of
  scope for this document. However, let's not paint ourselves into a corner, if
  we can help it. Are there additional constraints imposed on our design if we
  want to leave space for adding BLE in the future?

[ESP32-C3]: https://www.espressif.com/en/products/socs/esp32-c3
[#121]: https://github.com/tosc-rs/mnemos/issues/191
[Seeedstudio XIAO]: https://www.seeedstudio.com/Seeed-XIAO-ESP32C3-p-5431.html
[Adafruit QT Py]: https://www.adafruit.com/product/5405
[802.3]: https://en.wikipedia.org/wiki/IEEE_802.3
[jig]: https://github.com/tosc-rs/mnemos/tree/main/hardware/esp32c3-wifi-dev
[`mnemos-trace-proto`]: https://github.com/tosc-rs/mnemos/tree/main/source/trace-proto