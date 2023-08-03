use std::env;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use d1_config::PlatformConfig;
use mnemos_config::buildtime::render_project;

fn main() {
    let out_dir = env::var("OUT_DIR").expect("No out dir");
    let dest_path = Path::new(&out_dir);
    let mut f = File::create(dest_path.join("memory.x")).expect("Could not create file");

    f.write_all(include_bytes!("memory.x"))
        .expect("Could not write file");

    println!("cargo:rustc-link-search={}", dest_path.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-arg=-Tmemory.x");
    println!("cargo:rustc-link-arg=-Tlink.x");

    let lichee_rv = cfg!(feature = "lichee-rv");
    let mq_pro = cfg!(feature = "mq-pro");
    let beepy = cfg!(feature = "beepy");

    let name = match (lichee_rv, mq_pro, beepy) {
        (false, false, false) => panic!("Must select a board target"),
        (true, false, false) => "lichee-rv.toml",
        (false, true, false) => "mq-pro.toml",
        (false, false, true) => "beepy.toml",
        _ => panic!("Must only select one board target"),
    };

    render_project::<PlatformConfig>(name).unwrap();
}
