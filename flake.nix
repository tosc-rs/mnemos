{
  description = "Flake providing a development shell for MnemOS";

  inputs = {
    flake-utils = {
      url = "github:numtide/flake-utils";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils }:
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
            ];

            buildInputs = [ libclang zlib ];

            # Fix missing OpenSSL
            PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
            LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";
          };
      });
}
