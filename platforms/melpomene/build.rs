fn main() {
    config::buildtime::render_project::<melpo_config::PlatformConfig>("melpo.toml").unwrap();
}
