# RFC - Basic Configuration

This RFC proposes a basic configuration system for use in the kernel.

The goal of this system is to allow for a reduction in target-specific code, including potentially removing separate projects for similar targets, such as the lichee-rv, mq-pro, and beepy projects.

This system aims to address current problems, and get us to the next "order of magnitude" of problems (by ignoring many problems we know will be a "later problem").

## In Broad Strokes

### The `config` crate

We would add a new `config` crate that provides helpers for loading configuration at compile time from a `json` or `toml` format.

This crate would also provide helpers for "baking" this data at compile time using the [databake] crate in a `build.rs` file, and compiling into the target as code.

The `config` crate would also provide a `PlatformConfig` type that is generic over two types:

* A `KernelConfig` type, which contains all kernel-defined configuration, provided by the `kernel` crate
* A `PlatformConfig` type, which contains all target- or family-specific configuration, expected to be provided by any of the defined `platforms` that mnemos supports

This crate uses the generics to avoid circular dependencies between the config crate and kernel/platform crates. It is intended that "user facing" applications of the config crate (primarily in platform crates) is done by providing type aliases to "hide" the generics.

[databake]: https://docs.rs/databake/latest/databake/

### Kernel changes

The kernel will provide a single `KernelConfig` type, which is a struct that contains all service and daemon configuration values. We will add functionality to the kernel to take a provided `KernelConfig` type, and automatically launch the configured services.

This capability will allow most platform targets to avoid repeated boilerplate `kernel.initialize(...)` calls at startup.

### Platform crate changes

Platform crates will be expected to provide a `PlatformConfig` type, containing all drivers or daemons that are specific or applicable to this platform (other than the ones defined in the kernel itself). Similar to the kernel, they should also provide a single call that takes a `KernelConfig` and `PlatformConfig`, and automatically launches all kernel and platform services.

One platform may have many targets: For example, the Allwinner D1 platform has three targets currently: the `lichee-rv` dev board, the `mq-pro` dev board, and the `beepy`.

With this system in place, each of these targets would no longer have specific binary targets, and would instead share a single binary target with different configuration files, loaded at compile time.

This configuration file could be set via an environment variable such as `MNEMOS_CONFIG=/path/to/beepy.toml`, which would be loaded by a `build.rs` script, and set by `just`.

In this way, customized targets can be made by creating a new configuration file, rather than an entire cargo project.

## Compile Time vs Runtime Configuration

At the moment, most configuration will be done at compile time, with configuration values being "baked into" the binary.

However, we should ensure that all configuration values also implement `serde` traits, so that platforms that CAN load configuration at runtime (from an SD Card or other external sources) are able to.

Platforms that can load configuration at runtime may need to provide a "two stage" initialization process: using enough of the "baked in" configuration to initialize the SD Card or other external source, then continuing with the initialization process after merging or replacing the baked in data with the loaded data.

## Inspiration

This approach is largely inspired by [ICU4X]'s [Data Provider] model, which aims to "bake in" some common assets, but also allow for loading additional assets upon request.

[ICU4X]: https://github.com/unicode-org/icu4x
[Data Provider]: https://github.com/unicode-org/icu4x/blob/main/docs/design/data_pipeline.md

## "Tomorrows Problems"

This initial approach avoids many complexities by ignoring them completely. Some of these complexities are described below:

### Compile time feature enablement

At some point, we may have many drivers, some of which that are not relevant or possible to support on some platforms. Since this system will support runtime configuration, the compiler will still need to compile them into the kernel image, even though there is no chance of them being used.

This approach does not address compile time gating of features to prevent certain capabilities from being compiled in at all.

While this may lead to some "code bloat", we are not at a stage where this is an immediate concern. Once we are, our configuration approach may need to be revised.

### Extensibility

Similarly as new features are added that COULD be supported on all platforms, we will need to add them manually to the configuration items of each crate.

For example: if we have SPI drivers in many platforms, and write a bunch of drivers for external components connected over SPI, we will need to add configuration for each driver to all platforms.

This might be mitigated by grouping these into a common "all spi drivers" configuration type that is included "by composition" into all platform crates, but at some point this will cause friction and/or maintenance burden.

We may want to revisit how to support that in the future.

### "demand based" configuration

This configuration approach makes not attempt at addressing validation of configuration, particularly looking at the dependencies of each service to determine if the configuration is suitable.

For example, a platform may configure the sermux service, but forget to provide a serial port for the sermux service to use.

At the moment, this RFC just says "don't do that", or "catch that in testing", which will not scale past a certain complexity level

### Generating or building a platform config

This RFC makes no suggestion on how platforms should generate a comprehensive config file. We will probably want this eventually.
