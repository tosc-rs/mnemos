use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    vergen::EmitBuilder::builder()
        .all_cargo()
        .all_git()
        .all_rustc()
        .emit()?;
    Ok(())
}
