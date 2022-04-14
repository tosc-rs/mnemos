# Components

As a user or developer of MnemOS, you are likely to run into two main parts, the **kernel** and **userspace**.

The [kernel](./kernel.md) handles hardware-level operations, including memory management, event handling for hardware and driver events, and isolation of userspace from the hardware.

The [userspace](./userspace.md) is where applications run. Applications are provided a standard interface from the kernel, that allows them to perform operations like reading or writing to a serial port, or reading or writing to a block storage device (sort of like a hard drive).

Additionally, there is a [common](./common.md) library, which contains software components used by both the kernel and userspace.
