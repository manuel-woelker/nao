{ pkgs, ... }:

{
  languages.rust = {
    enable = true;
    channel = "stable";
    version = "1.93.1";
    components = [
      "rustc"
      "cargo"
      "clippy"
      "rustfmt"
      "rust-src"
      "rust-analyzer"
    ];
  };

  packages = [
    pkgs.cargo-nextest
    pkgs.git
    pkgs.openssl
    pkgs.pkg-config
  ];

  enterShell = ''
    export CARGO_HOME="$PWD/.devenv/state/cargo-home"
    export CARGO_TARGET_DIR="$PWD/.devenv/state/cargo-target"

    mkdir -p "$CARGO_HOME" "$CARGO_TARGET_DIR"

    echo "devenv ready"
    rustc --version
    cargo --version
  '';
}
