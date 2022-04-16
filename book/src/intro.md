# MnemOS introduction

## What is MnemOS?

MnemOS is a small, general purpose operating system (as opposed to "real time operating system", or RTOS), designed for constrained environments.

As an operating system, it is aimed at making it easy to write portable applications, without having to directly interact with the hardware.

At the moment, MnemOS is not a multitasking operating system, so only one application (and the kernel) can run at any given time.

## Why MnemOS?

I guess there's two parts to this: (for me, James) "Why did I write it", and (as a user/developer) "Why should I use MnemOS"?

### Why did I write it?

For "Why did I write it": This is a "spin off" of part of my previous project, Powerbus, which is a networked home automation project. It honestly started getting too complicated with too many "crazy ideas" (it's a network stack! and a scripting language! and an operating system!) for one single hobby project. So I split the "networking" part off to a much smaller, simple project (lovingly named "Dumb Bus" at the moment), and MnemOS, which was more focused on building a single computer system.

This split helped to better focus BOTH parts of the (former) Powerbus system, and may in the future be recombined, when the separate parts have had more time to bake and solidify on their own.

### Why should I [or you] use MnemOS?

As to "Why should I [or you] use MnemOS?", I don't have a good answer! There is certanly no commercial or technical reasons you would choose MnemOS over any of its peers in the "hobbyist space" (e.g. Monotron OS, or projects like RC2014), or even choose it over existing commercial or popular open source projects (like FreeRTOS, or even Linux). It's mainly for me to scratch a personal itch, to learn more about implementing software within the realm of an OS, which is relatively "high level" (from the perspective of embedded systems), while also being relatively "low level" (from the perspective of an application developer).

At the moment, it has the benefit of being relatively small (compared to operating system/kernel projects like Linux, or Redox), which makes it easier to change and influence aspects of the OS. I don't think it will ever be anything "serious", but I do plan to use to it to create a number of projects, including a portable text editor, a music player, and maybe even music making/sythesizer components. Some day I'd like to offer hardware kits, to make it easier for more software-minded folks to get started.

For me, it's a blank slate, where I can build things that I intrinsically understand, using tools and techniques that appeal to me and are familiar to me. I'd love to have others come along and contribute to it (I am highly motivated by other people's feedback and excitement!), but I'll keep working on it even if no one else ever shows up. By documenting what I do, I'll gain a better understanding (and an easier route to picking it up if I have to put it down for a while), and that work serves to "keep the lights on" for any kindred spirits interested in building a tiny, simple, PC in Rust.

If that appeals to you, I invite you to try it out. I am more than happy to explain any part of MnemOS. Much like the Rust Programming Language project - I believe that if any part of the OS is not clear, that is a bug (at least in the docs), and should be remedied, regardless of your technical skill level.

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
