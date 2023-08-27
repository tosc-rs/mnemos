#![doc = include_str!("../README.md")]
use anyhow::{Context, Result};
use std::{env, path::Path, process};

const MN_CARGO_BINS: Option<&str> = option_env!("MN_CARGO_BINS");

const PATH_KEY: &str = "PATH";

fn main() -> Result<()> {
    if cfg!(not(feature = "_any-deps")) {
        eprintln!("warning: running `mn` without the 'install-deps' feature falls back to just doing nothing!");
    }

    let verbose = env::var("MN_VERBOSE")
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if verbose {
        println!("Manganese: MN_CARGO_BINS={MN_CARGO_BINS:?}");
    }

    let args: Vec<String> = env::args().collect();
    let just_args = &args[1..];

    let (path, mut cmd) = {
        let prev_path = env::var(PATH_KEY).ok().unwrap_or_default();
        if let Some(cargo_bins) = MN_CARGO_BINS {
            let mut path = String::with_capacity(cargo_bins.len() + 1 + prev_path.len());
            path.push_str(cargo_bins);
            path.push(':');
            path.push_str(&prev_path);
            let just_path = Path::new(&cargo_bins).join("just");

            (path, process::Command::new(just_path))
        } else {
            if cfg!(feature = "_any-deps") {
                panic!("if the 'install-deps' feature is enabled, MN_CARGO_BINS must be set! seems like the build script broke...");
            }

            (prev_path, process::Command::new("just"))
        }
    };

    cmd.args(just_args).env(PATH_KEY, path);

    if verbose {
        println!("Manganese: {cmd:?}");
    }

    let status = cmd
        .spawn()
        .with_context(|| format!("failed to spawn `just` command\ncommand: {cmd:?}"))?
        .wait()
        .with_context(|| {
            format!("failed to wait for `just` command to complete\ncommand: {cmd:?}")
        })?;

    if verbose {
        println!("Manganese: exit status {status:?}");
    }

    let exit_code = status
        .code()
        .unwrap_or(if status.success() { 0 } else { 1 });

    process::exit(exit_code);
}
