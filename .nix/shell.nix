# Development shell configuration for kalpa
# This file defines the development environment

{ pkgs }:

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
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  kalpa development environment"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "  Commands:"
    echo "    cargo build          Build the project"
    echo "    cargo run -- <args>  Run kalpa CLI"
    echo "    just --list          Show all tasks"
    echo "    cargo test           Run tests"
    echo ""
    echo "  Quick start:"
    echo "    cargo run -- configure"
    echo "    cargo run -- auth -g"
    echo "    cargo run -- generate -g text \"Hello\""
    echo ""
  '';

  # Environment variables
  RUST_BACKTRACE = "1";
  RUST_LOG = "info";
}
