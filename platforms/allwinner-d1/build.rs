use std::{
    env,
    path::{Path, PathBuf},
};

use d1_config::PlatformConfig;
use miette::{IntoDiagnostic, Result, WrapErr};
use mnemos_config::buildtime;

fn main() -> Result<()> {
    let out_dir = env::var("OUT_DIR")
        .into_diagnostic()
        .context("No OUT_DIR")?;
    let dest_path = Path::new(&out_dir);

    println!("cargo:rustc-link-search={}", dest_path.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-arg=-Tmemory.x");
    println!("cargo:rustc-link-arg=-Tlink.x");

    // render mnemos-config definitions
    let config_dir = {
        let root = env::var("CARGO_MANIFEST_DIR")
            .into_diagnostic()
            .context("No CARGO_MANIFEST_DIR")?;
        PathBuf::from(root).join("board-configs")
    };
    buildtime::render_all::<PlatformConfig>(config_dir)
}
