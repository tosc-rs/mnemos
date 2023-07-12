# Pomelo

A browser-based MnemOS ... Client? Emulator? Userspace? [Citrus tentacle](https://en.wikipedia.org/wiki/Buddha%27s_hand)?

## See it in action

A prebuilt version is hosted [here](https://anatol.versteht.es/mlem/)

## What can you do with it?

Pomelo lets you interact with MnemOS using a bespoke command line interface with (rudimentary) tab completion and history (using cursor up/down).
Since everything is async, output from services will be mixed with user input at arbitrary times.
At this point this is considered kind of a "feature" - think several background jobs in a Linux shell.
The syntax is `<CMD> <ONE_ARG>` - in other words, arguments are not separated by spaces, nor can they be welded together using quotes. It is up to the individual command to split things up if so desired.

```
$ help

Try some of the commands below.

  help       Prints this help message
  history    Prints command history
  echo       Contender for world's most contorted echo implementation
  hello      Start a cheerful Hello Server
  forth      Execute a line of forth
  iforth     Interactive forth (REPL) - exit with Ctrl-C

$ echo hello, citrus!
$ hello, citrus!

$ forth 2 2 + .
$ 4 ok.
```

### Interactive FORTH REPL

The `iforth` command can be used to enter an interpreter read/eval/print loop (REPL). It also uses the history buffer, but offers no tab completion. Control+C returns to the regular shell.

```
$ iforth
F : star 42 emit ;                         
F ok.
         
F star
F *ok.

F : stars 0 do star loop ;                 
F ok.

F 20 stars
F ********************ok.

F ^C
$ 
```

`iforth` and `forth` share a common session and quitting the REPL does not delete the stack or definitions:

```
$ forth 10 stars
$ **********ok.
```

## Installation

[trunk](https://trunkrs.dev/#install) is a prerequisite. It can be used to build a version suitable for static hosting, or run a hot-reloading local development server.


## Local development server

```shell
$ trunk serve
# (...)
INFO 📡 server listening at http://127.0.0.1:8080
```

## Static build
```shell
# build to dist/, suitable for http(s)://host/
$ trunk build --release

# build to dist/ with a custom webroot dir, e.g. http(s)://host/my/webroot
$ trunk build --release --public-url='/my/webroot/'
```

## Development/Architecture

- Commands are defined in `src/js/glue.js`. Some (like the REPL mode, or `help`) are handled entirely in JS, others that actually talk to the OS are dispatched as JavaScript objects and deserialized into `term_iface::Command`s.

- The [tracing-wasm](https://crates.io/crates/tracing-wasm) crate supports performance tracing, which at the moment seems to be supported only by Chrome.

- tracing messages are sent to the browser's debug console.


## Licensing

This package contains a vendored copy of [Xterm.js](http://xtermjs.org/). Its license (MIT) is preserved in `src/js/xterm.css`.