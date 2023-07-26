# esp32c3-wifi-dev

![esp32c3-wifi-dev-005](https://github.com/tosc-rs/mnemos/assets/2097964/120f5a1c-170b-45ec-b688-e6ad1881b8ad)

![esp32c3-wifi-dev-006](https://github.com/tosc-rs/mnemos/assets/2097964/05facb18-456e-4dfb-bb50-a64d6150a5ae)


This is a breakout board for the [XIAO ESP32C3](https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/) and the [XIAO RP2040](https://wiki.seeedstudio.com/XIAO-RP2040/).

The intent is to use these boards to implement a wifi modem firmware on the ESP32C3, and to speak to it using the RP2040 as a USB-to-UART+SPI adapter.

The initial plan is to implement a melpo service that acts as a SPI port, but tunnels the communication via USB to the RP2040, similar to (or perhaps exactly like) [`pretty-hal-machine`](https://github.com/jamesmunns/pretty-hal-machine).

```
.---------.           .---------.
|         +<-- SPI -->+         |
| RP2040  |           | ESP32C3 |
|         +<- UART -->+         |
'---USB---'           '---USB---'
     ^                     ^
     |                     |
     '-------> HUB <-------'
                ^
                |
                v
           .---USB---.
           +         |
           |  Melpo  |
           +   (PC)  |
           '---------'
```
