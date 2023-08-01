use std::{io::Write, path::PathBuf};

use config::{self, buildtime::to_postcard};
use melpo_config::PlatformConfig;
use mnemos_kernel::{DefaultServiceSettings, KernelConfig};

fn main() {
    let cfg = std::fs::read_to_string("melpo.toml").unwrap();
    let c: config::MnemosConfig<KernelConfig, DefaultServiceSettings, PlatformConfig> =
        config::buildtime::from_toml(&cfg).unwrap();

    let out_dir = std::env::var("OUT_DIR").unwrap();
    assert!(!out_dir.is_empty());
    let mut out = PathBuf::from(out_dir);
    out.push("melpo.postcard");
    let bin_cfg = to_postcard(&c).unwrap();
    let mut f = std::fs::File::create(&out).unwrap();
    f.write_all(&bin_cfg).unwrap();
    println!("cargo:rustc-env=MNEMOS_CONFIG={}", out.display());
    println!("cargo:rerun-if-changed=melpo.toml");
}
