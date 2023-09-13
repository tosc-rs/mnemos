fn main() {
    mnemos_config::buildtime::render_file::<melpo_config::PlatformConfig>("melpo.toml").unwrap();
}
