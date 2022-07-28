# D1 RGB LCD Demo

```
cargo objcopy --release -- -Obinary out.bin
xfel ddr d1
xfel write 0x40000000 out.bin
xfel exec 0x40000000
```
