# Build kalpa application
# Usage: nix-build .nix/build.nix

{ pkgs ? import <nixpkgs> {} }:

pkgs.rustPlatform.buildRustPackage rec {
  pname = "kalpa";
  version = "0.1.0";

  src = pkgs.lib.cleanSource ../.;

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  nativeBuildInputs = with pkgs; [
    pkg-config
  ];

  buildInputs = with pkgs; [
    openssl
  ];

  meta = with pkgs.lib; {
    description = "A unified CLI for AI generative models";
  };
}
