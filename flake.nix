{
  description = "Flake providing a development shell for MnemOS";

  inputs = {
    # unstable is necessary for Oranda's flake, which depends on `tailwindcss`
    # (not available in stable nixpkgs yet).
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    oranda = {
      # use my fork of Oranda until upstream merges PR
      # https://github.com/axodotdev/oranda/pull/609 (this is necessary to fix
      # the flake)
      url = "github:hawkw/oranda?rev=8e5eff3d1f9c4e3642d8c327032d4072d2ca4a00";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, oranda }:
    flake-utils.lib.eachDefaultSystem (system:
      let pkgs = import nixpkgs { inherit system; };
      in {
        devShell = with pkgs;
          mkShell {
            name = "mnemos-dev";

            nativeBuildInputs = [
              # needed for building C deps such as SDL2 for embedded-graphics
              # simulator.
              pkg-config
              cmake

              # these are in nativeBuildInputs as they are dependencies of the
              # host tools.
              # TODO(eliza): add separate packages for each of crowtty,
              # melpomene, and other host tools...
              systemd
              udev
              SDL2
              SDL2.dev

              # compilers
              rustup
              clang

              # devtools
              rust-analyzer
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
            ];

            buildInputs = [ libclang zlib ];

            # Fix missing OpenSSL
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
            LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
          };
      });
}
