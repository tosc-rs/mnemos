fn main() {
    mnemos_config::buildtime::render_project::<melpo_config::PlatformConfig>("melpo.toml").unwrap();
}
