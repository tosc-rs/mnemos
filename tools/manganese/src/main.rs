use anyhow::{Context, Result};
use std::{env, process};

const JUST_PATH: &str = env!("CARGO_BIN_FILE_JUST_just");
const NEXTEST_PATH: &str = env!("CARGO_BIN_FILE_CARGO_NEXTEST_cargo-nextest");

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let args = &args[1..];
    let verbose = env::var("MN_VERBOSE")
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    let just_args = if args.is_empty() { &[] } else { &args[1..] };

    let mut cmd = process::Command::new(JUST_PATH);
    cmd.args(just_args).env("MN_NEXTEST_PATH", NEXTEST_PATH);

    if verbose {
        println!("Manganese: {cmd:?}");
    }

    let status = cmd
        .spawn()
        .context("failed to spawn `just` command")?
        .wait()
        .context("failed to spawn `just` command")?;

    if verbose {
        println!("Manganese: exit status {status:?}");
    }

    let exit_code = status
        .code()
        .unwrap_or(if status.success() { 0 } else { 1 });

    process::exit(exit_code);
}
