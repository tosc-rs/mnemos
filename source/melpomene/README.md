# Melpomene

Melpomene is a desktop simulator for the MnemOS kernel.

Rather than emulating any specific hardware, it runs the kernel in one thread, and userspace in a second thread.

Drivers to implement platform-specific behavior, such as serial ports, are provided.

## Running the simulator

The simulator can be run using `cargo run` from this folder.

The level of tracing can be configured with `MELPOMENE_TRACE`, e.g.

```shell
# trace level logging
MELPOMENE_TRACE=trace cargo run

# warn level logging
MELPOMENE_TRACE=warn cargo run
```

## License

[MIT] + [Apache 2.0].

[MIT]: ./../../LICENSE-MIT
[Apache 2.0]: ./../../LICENSE-APACHE
