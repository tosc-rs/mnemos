# MnemOS introduction

## What is MnemOS?

MnemOS is a small, general purpose operating system (as opposed to "real time operating system", or RTOS), designed for constrained environments.

As an operating system, it is aimed at making it easy to write portable applications, without having to directly interact with the hardware.

At the moment, MnemOS is not a multitasking operating system, so only one application (and the kernel) can run at any given time.

## Where does the name come from/how do I pronounce it?

"MnemOS" is named after [Mnemosyne](https://en.wikipedia.org/wiki/Mnemosyne), the greek goddess of memory, and the mother of the 9 muses. Since one of the primary responsibilities of an OS is to manage memory, I figured it made sense.

In IPA/Greek, it would be [`mnɛːmos`](https://en.wikipedia.org/wiki/Help:IPA/Greek). Roughly transcribed, it sounds like "mneh-moss".

To listen to someone pronounce "Mnemosyne", you can listen to [this youtube clip](https://www.youtube.com/watch?v=xliDJCBxHAo&t=939s), and pretend he isn't saying the back half of the name.

If you pronounce it wrong, I won't be upset.

## Where can I get a MnemOS computer?

You can't buy one "off the shelf" currently. For the currently required parts, you can refer to the [Supported (and Required) Hardware](./dev-guide/hardware.md) section of the [Developers Guide](./dev-guide/intro.md) chapter.

You can find the source code and hardware design files for components of a MnemOS based computer [on GitHub](https://github.com/jamesmunns/pellegrino)

## How do I use a MnemOS based computer?

You can find information on how to use a MnemOS based computer, including uploading and running applications, in the [Users Guide](./user-guide/intro.md) section of this book.

## How do I write applications for a MnemOS based computer?

The primary development environment is in the Rust programming language.

MnemOS provides libraries that can be included in your project to create an application.

You can find information on how to create and build applications in the [Building User Applications](./dev-guide/build-apps.md) section of the [Developers Guide](./dev-guide/intro.md) chapter.

## How do I modify or debug the MnemOS kernel?

You can find required hardware and software for modifying or debugging the MnemOS kernel in the [Developers Guide](./dev-guide/intro.md) section of this book.
