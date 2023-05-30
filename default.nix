scope@{ pkgs ? import <nixpkgs> { } }:

let locale = "en_US.UTF8";
in with pkgs;
buildEnv {
  name = "mnemos-env";
  paths = with pkgs;
    [
      git
      bash
      direnv
      binutils
      stdenv
      bashInteractive
      docker
      cacert
      gcc
      cmake
      rustup
      openssl
      protobuf
      docker
      just
      shellcheck
      (glibcLocales.override { locales = [ locale ]; })
    ] ++ lib.optional stdenv.isDarwin [ Security libiconv ];

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ clang libclang systemd udev ];

  passthru = with pkgs; {
    PROTOC = "${protobuf}/bin/protoc";
    PROTOC_INCLUDE = "${protobuf}/include";

    LOCALE_ARCHIVE = "${glibcLocales}/lib/locale/locale-archive";
    LC_ALL = locale;

    SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
    GIT_SSL_CAINFO = "${cacert}/etc/ssl/certs/ca-bundle.crt";
    CURL_CA_BUNDLE = "${cacert}/etc/ca-bundle.crt";
    CARGO_TERM_COLOR = "always";
    RUST_BACKTRACE = "full";

    LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";

    OPENSSL_DIR = "${openssl.dev}";
    OPENSSL_LIB_DIR = "${openssl.out}/lib";
  };
}
