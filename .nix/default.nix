# Development shell for kalpa
# Usage: nix-shell .nix/default.nix

{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    # Rust toolchain
    rustc
    cargo
    rustfmt
    clippy
    rust-analyzer

    # Build dependencies
    pkg-config
    openssl

    # Development tools
    just
    git
  ];

  shellHook = ''
    echo "kalpa development environment"
    echo "  cargo build    - Build the project"
    echo "  cargo run      - Run the CLI"
    echo "  just --list    - Show available tasks"
  '';

  RUST_BACKTRACE = "1";
}
