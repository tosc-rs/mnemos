# CrowTTY Virtual Serial Multiplexor

crowtty is a host tool, aimed at speaking the sermux protocol with a simulator or physical target. It allows for receiving tracing messages, as well as mapping multiplexed "ports" as TCP sockets on the host.

It is the primary way of connecting to hardware platforms that speak the sermux-proto over a physical serial port.

## Usage

```
Usage: crowtty [OPTIONS] <COMMAND>

Commands:
  tcp     open listener on localhost:PORT
  serial  open listener on PATH
  help    Print this message or the help of the given subcommand(s)

Options:
  -v, --verbose
          whether to include verbose logging of bytes in/out

  -t, --trace <TRACE_FILTER>
          a comma-separated list of `tracing` targets and levels to enable.

          for example, `info,kernel=debug,kernel::comms::bbq=trace` will enable:

          - the `INFO` level globally (regardless of module path), - the `DEBUG` level for all modules in the `kernel` crate, - and the `TRACE` level for the `comms::bbq` submodule in `kernel`.

          enabling a more verbose level enables all levels less verbose than that level. for example, enabling the `INFO` level for a given target will also enable the `WARN` and `ERROR` levels for that target.

          see <https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/targets/struct.Targets.html#filtering-with-targets> for more details on this syntax.

          [env: MNEMOS_TRACE=]
          [default: info]

  -k, --keyboard-port <KEYBOARD_PORT>
          SerMux port for a pseudo-keyboard for the graphical Forth shell on the target

          [default: 2]

      --no-keyboard
          disables STDIN as the pseudo-keyboard.

          if this is set, the pseudo-keyboard port can be written to as a standard TCP port on the host, instead of reading from crowtty's STDIN.

      --tcp-port-base <TCP_PORT_BASE>
          offset for host TCP ports.

          SerMux port `n` will be mapped to TCP port `n + tcp-port-base` on localhost.

          [default: 10000]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```
