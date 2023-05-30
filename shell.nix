scope@{ pkgs ? import <nixpkgs> { } }:
with pkgs;
let
  mnemos = import ./default.nix { inherit pkgs; };

  env = buildEnv {
    name = "mnemos-env";
    paths = [ ] ++ lib.optional stdenv.isDarwin libiconv ++ mnemos.buildInputs
      ++ mnemos.nativeBuildInputs;
  };
in mkShell {
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ clang libclang systemd udev SDL2 SDL2.dev ];

  CARGO_TERM_COLOR = "always";
  RUST_BACKTRACE = "full";
}
