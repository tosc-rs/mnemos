use anyhow::Context;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    // I *hate* this stupid goddamn Clippy lint. On a new enough nightly, the
    // compiler warns that `'static` lifetimes for constants will become
    // mandatory in a future Rust edition, so the Clippy lint is actually
    // telling you to do the opposite of what's compatible with future Rustc
    // changes...
    #![allow(clippy::redundant_static_lifetimes)]

    const PKG_NAME: &'static str = "mnemos-x86_64-core";
    const BIN_NAME: &'static str = "bootloader";
    const TARGET_TRIPLE: &'static str = "x86_64-unknown-none";
    const ENV_OUT_DIR: &'static str = "OUT_DIR";
    const ENV_PROFILE: &'static str = "PROFILE";

    // set by cargo, build scripts should use this directory for output files
    let out_dir = PathBuf::from(
        std::env::var_os(ENV_OUT_DIR)
            .with_context(|| format!("missing {ENV_OUT_DIR} environment variable!"))?,
    );
    let release = match std::env::var_os(ENV_PROFILE) {
        Some(x) if x == "release" => true,
        Some(x) if x == "debug" => false,
        x => {
            println!("cargo:warning={ENV_PROFILE} env var either unset or weird: {x:?}");
            false
        }
    };

    // XXX(eliza): it's sad that this way of building the binary by just
    // shelling out to a child `cargo` invocation will eat things like the
    // compiler output. but, the alternative approach where the same cargo
    // invocation can build everything would be to use artifact deps, which are
    // unfortunately broken due to this Cargo bug:
    // https://github.com/rust-lang/cargo/issues/12358
    //
    // If upstream PR https://github.com/rust-lang/cargo/pull/13207 ever merges,
    // we should revisit this approach and see if we can switch back to artifact
    // deps...
    let mut build = escargot::CargoBuild::new()
        .package(PKG_NAME)
        .bin(BIN_NAME)
        .target(TARGET_TRIPLE)
        .target_dir(&out_dir)
        .features("bootloader_api");
    if release {
        build = build.release();
    }

    let cargo_output = build
        .run()
        .context("failed to execute cargo build command")?;

    let kernel = cargo_output.path();

    let uefi_path = out_dir.join("mnemos-x86_64-uefi.img");
    bootloader::UefiBoot::new(kernel)
        .create_disk_image(&uefi_path)
        .unwrap();

    // create a BIOS disk image
    let bios_path = out_dir.join("mnemos-x86_64-bios.img");
    bootloader::BiosBoot::new(kernel)
        .create_disk_image(&bios_path)
        .unwrap();

    // pass the disk image paths as env variables to the `main.rs`
    println!("cargo:rustc-env=UEFI_PATH={}", uefi_path.display());
    println!("cargo:rustc-env=BIOS_PATH={}", bios_path.display());
    Ok(())
}
