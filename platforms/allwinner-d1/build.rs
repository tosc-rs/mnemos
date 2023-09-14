use std::{
    env,
    path::{Path, PathBuf},
};

use d1_config::PlatformConfig;
use miette::{IntoDiagnostic, Result, WrapErr};
use mnemos_config::buildtime;

fn main() -> Result<()> {
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");

    // render mnemos-config definitions
    let config_dir = {
        let root = env::var("CARGO_MANIFEST_DIR")
            .into_diagnostic()
            .context("No CARGO_MANIFEST_DIR")?;
        PathBuf::from(root).join("board-configs")
    };
    buildtime::render_all::<PlatformConfig>(config_dir)?;

    copy_linker_script().wrap_err("Copying linker script to OUT_DIR failed!")
}

fn copy_linker_script() -> Result<()> {
    use std::{fs::File, io::Write};

    let out_dir = env::var("OUT_DIR")
        .into_diagnostic()
        .context("No OUT_DIR")?;
    let dest_path = Path::new(&out_dir);
    let mut f = File::create(dest_path.join("memory.x")).into_diagnostic()?;
    f.write_all(include_bytes!("memory.x")).into_diagnostic()?;

    println!("cargo:rustc-link-search={}", dest_path.display());

    println!("cargo:rustc-link-arg=-Tmemory.x");
    println!("cargo:rustc-link-arg=-Tlink.x");

    Ok(())
}
