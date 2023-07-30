use melpo_config::PlatformConfig;
use config;
use mnemos_kernel::DefaultServiceSettings;

fn main() {
    let cfg = std::fs::read_to_string("melpo.toml").unwrap();
    let c: config::MnemosConfig::<DefaultServiceSettings<'static>, PlatformConfig> = config::buildtime::from_toml(&cfg).unwrap();
}
