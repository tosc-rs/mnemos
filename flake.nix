{
  description = "Flake providing a development shell for MnemOS";

  inputs = {
    # unstable is necessary for Oranda's flake, which depends on `tailwindcss`
    # (not available in stable nixpkgs yet).
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    oranda = {
      url = "github:axodotdev/oranda";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # see https://fasterthanli.me/series/building-a-rust-service-with-nix/part-10#a-flake-with-a-dev-shell
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
  };

  outputs = { self, nixpkgs, flake-utils, oranda, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
        # use the Rust toolchain specified in the project's rust-toolchain.toml
        rustToolchain = pkgs.pkgsBuildHost.rust-bin.fromRustupToolchainFile
          ./rust-toolchain.toml;
      in
      {
        devShell = with pkgs;
          mkShell rec {
            name = "mnemos-dev";

            nativeBuildInputs = [
              # needed for building C deps such as SDL2 for embedded-graphics
              # simulator.
              pkg-config
              cmake

              # compilers
              rustToolchain
              clang

              # devtools
              just
              cargo-nextest
              # rust esp32 tools
              cargo-espflash
              cargo-espmonitor
              # for testing the x86_64 kernel
              qemu
              # for building the website
              oranda.packages.${system}.default
              python3 # needed by rfc2book
              # for building pomelo
              trunk
            ];

            buildInputs = [
              libclang
              zlib
              # dependencies of the host tools
              # TODO(eliza): add separate packages for each of crowtty,
              # melpomene, and other host tools...
              systemd
              udev.dev
              SDL2
              SDL2.dev
            ];

            # Fix missing OpenSSL
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
            LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
            LD_LIBRARY_PATH = "${lib.makeLibraryPath buildInputs}";
          };
      });
}
