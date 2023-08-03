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
  buildInputs = [
    clang
    libclang
    systemd
    udev
    # for melpomene
    SDL2
    SDL2.dev
  ];
  packages = [
    # devtools
    just
    cargo-nextest
    direnv
    # rust esp32 tools
    cargo-espflash
    cargo-espmonitor
    # for testing the x86_64 kernel
    qemu
  ];

  PROTOC = "${protobuf}/bin/protoc";
  PROTOC_INCLUDE = "${protobuf}/include";

  SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
  GIT_SSL_CAINFO = "${cacert}/etc/ssl/certs/ca-bundle.crt";
  CURL_CA_BUNDLE = "${cacert}/etc/ca-bundle.crt";
  CARGO_TERM_COLOR = "always";

  LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";

  OPENSSL_DIR = "${openssl.dev}";
  OPENSSL_LIB_DIR = "${openssl.out}/lib";
}
