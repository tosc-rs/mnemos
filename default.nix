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
      openssl
      shellcheck
      (glibcLocales.override { locales = [ locale ]; })
    ] ++ lib.optional stdenv.isDarwin [ Security libiconv ];

  nativeBuildInputs = [ pkg-config cmake rustup ];
  buildInputs = [
    clang
    libclang
    systemd
    udev
    SDL2
    SDL2.dev
    # other stuff
    bash
  ];
  passthru = with pkgs; {
    SSL_CERT_FILE = "${cacert}/etc/ssl/certs/ca-bundle.crt";
    GIT_SSL_CAINFO = "${cacert}/etc/ssl/certs/ca-bundle.crt";
    CURL_CA_BUNDLE = "${cacert}/etc/ca-bundle.crt";

    CARGO_TERM_COLOR = "always";
    RUST_BACKTRACE = "true";

    LIBCLANG_PATH = "${llvmPackages.libclang.lib}/lib";

    OPENSSL_DIR = "${openssl.dev}";
    OPENSSL_LIB_DIR = "${openssl.out}/lib";

    LD_LIBRARY_PATH = "${lib.makeLibraryPath buildInputs}";

  };
}
