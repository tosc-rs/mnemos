# mnemOS introduction

## what is mnemOS?

mnemOS is a small, general purpose OS. It aims to bridge the gap between project specific bare-metal applications or real-time operating systems (RTOS), and larger more complete general purpose operating systems (like Linux).

It's a hobby project of mine, though more people have started contributing lately! It doesn't have any specific financial backer or particular goals, but I am using it to research areas of "what is possible" on lightweight embedded systems. The project is written entirely in Rust.

It comes from making (or making part of) a bunch of projects that were really too big and complex to be a bare metal project, and I realized I kept building a bunch of fragile, incompatible parts of an operating system for each of the projects. After getting overwhelmed on the last one, I decided to just build an OS I could use.

mnemOS is targeted at supporting both microcontroller and microprocessor systems, at least for now. I tend to do a lot of projects that span the sort of range from a medium sized microcontroller (32-bit, 64-128MHz, 128+KiB SRAM, 256KiB+ Flash), up to a small sized microprocessor (32/64-bit, 500MHz+, 64-512MiB DRAM, 8MiB+ Flash). At least for my hobby projects, there isn't a lot of price difference between the price points of those two classes of chips, though sometimes one will have features (CPU, RAM, Peripherals, existing drivers) that make one choice more appropriate.

As a general purpose OS, it doesn't aim to necessarily be suitable for super time-critical functionality (it may have non-deterministic scheduling or resource usage), and instead is aimed at making other capabilities like networking, file system support, user interface support, and code re-use a higher priority.

## why should I (or you) use mnemOS?

I don't have a good answer! There is certanly no commercial or technical reasons you would choose mnemOS over any of its peers in the "hobbyist space" (e.g. Monotron OS, or projects like RC2014), or even choose it over existing commercial or popular open source projects (like FreeRTOS, or even Linux). It's mainly for me to scratch a personal itch, to learn more about implementing software within the realm of an OS, which is relatively "high level" (from the perspective of embedded systems), while also being relatively "low level" (from the perspective of an application developer).

At the moment, it has the benefit of being relatively small (compared to operating system/kernel projects like Linux, or Redox), which makes it easier to change and influence aspects of the OS. I don't think it will ever be anything "serious", but I do plan to use to it to create a number of projects, including a portable text editor, a music player, and maybe even music making/sythesizer components. Some day I'd like to offer hardware kits, to make it easier for more software-minded folks to get started.

For me, it's a blank slate, where I can build things that I intrinsically understand, using tools and techniques that appeal to me and are familiar to me. I'd love to have others come along and contribute to it (I am highly motivated by other people's feedback and excitement!), but I'll keep working on it even if no one else ever shows up. By documenting what I do, I'll gain a better understanding (and an easier route to picking it up if I have to put it down for a while), and that work serves to "keep the lights on" for any kindred spirits interested in building a tiny, simple, PC in Rust.

If that appeals to you, I invite you to try it out. I am more than happy to explain any part of mnemOS. Much like the Rust Programming Language project - I believe that if any part of the OS is not clear, that is a bug (at least in the docs), and should be remedied, regardless of your technical skill level.

## where does the name come from/how do I pronounce it?

"mnemOS" is named after [Mnemosyne](https://en.wikipedia.org/wiki/Mnemosyne), the greek goddess of memory, and the mother of the 9 muses. Since one of the primary responsibilities of an OS is to manage memory, I figured it made sense.

In IPA/Greek, it would be [`mnɛːmos`](https://en.wikipedia.org/wiki/Help:IPA/Greek). Roughly transcribed, it sounds like "mneh-moss".

To listen to someone pronounce "Mnemosyne", you can listen to [this youtube clip](https://youtu.be/xliDJCBxHAo?t=99), and pretend he isn't saying the back half of the name.

If you pronounce it wrong, I won't be upset.
