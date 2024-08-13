# Melpomene

Melpomene is a desktop simulator for the MnemOS kernel.

Rather than emulating any specific hardware, it runs the kernel in one thread, and userspace in a second thread.

Drivers to implement platform-specific behavior, such as serial ports, are provided.

## Running the simulator

### MacOS specific notes

`embedded-graphics-simulator`, which melpomeme depends on, requires sdl2. If you installed sdl2 via homebrew,
you probably need to add this to your shell environment:

```sh
export LIBRARY_PATH="$LIBRARY_PATH:$(brew --prefix)/lib"
```

### Using an alias

You can run the simulator from anywhere in the project using the `cargo melpo` alias. Use `cargo melpo --help` to see all available options:

```shell
$ cargo melpo --help
    Finished dev [unoptimized + debuginfo] target(s) in 0.08s
     Running `target/debug/melpomene --help`
melpomene 0.1.0

USAGE:
    melpomene [OPTIONS]

OPTIONS:
    -h, --help
            Print help information

        --serial-addr <SERIAL_ADDR>
            Address to bind the TCP listener for the simulated serial port

            [default: 127.0.0.1:9999]

    -V, --version
            Print version information

TRACING OPTIONS:
        --trace <ENV_FILTER>
            Trace filter for `tracing-subscriber::fmt`.

            This requires that Melpomene be built with the "trace-fmt" feature flag enabled.

            [env: MELPOMENE_TRACE=]
            [default: info]

TRACING OPTIONS (TOKIO-CONSOLE):
        --console-addr <ADDR>
            Address to bind the `tokio-console` listener on.

            This requires that Melpomene be built with the "trace-console" feature flag enabled.

            [env: TOKIO_CONSOLE_BIND=]
            [default: 127.0.0.1:6669]

        --console-publish-interval <PUBLISH_INTERVAL>
            The interval between publishing updates to connected `tokio-console` clients.

            This requires that Melpomene be built with the "trace-console" feature flag enabled.

            [env: TOKIO_CONSOLE_PUBLISH_INTERVAL=]
            [default: 1s]

        --console-record-path <RECORD_PATH>
            A file path to save a `tokio-console` recording to.

            If a value is present, a recording will be output to that file. Otherwise, no recording
            will be saved.

            This requires that Melpomene be built with the "trace-console" feature flag enabled.

            [env: TOKIO_CONSOLE_RECORD_PATH=]

        --console-retention <RETENTION>
            How long to retain `tokio-console` data for completed tasks.

            This requires that Melpomene be built with the "trace-console" feature flag enabled.

            [env: TOKIO_CONSOLE_RETENTION=]
            [default: 1h]
```

### From this folder

The simulator can also be run using `cargo run` from this folder. All of the same options are available as above.

The level of tracing can be configured with `MELPOMENE_TRACE`, e.g.

```shell
# trace level logging
MELPOMENE_TRACE=trace cargo run

# warn level logging
MELPOMENE_TRACE=warn cargo run
```

## License

[MIT] + [Apache 2.0].

[MIT]: https://github.com/tosc-rs/mnemos/blob/main/LICENSE-MIT
[Apache 2.0]: https://github.com/tosc-rs/mnemos/blob/main/LICENSE-APACHE
