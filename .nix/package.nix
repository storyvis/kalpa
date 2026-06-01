# Production build configuration for kalpa
# This file defines how to build the kalpa binary

{ pkgs }:

pkgs.rustPlatform.buildRustPackage {
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
  ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
    pkgs.darwin.apple_sdk.frameworks.Security
    pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
  ];

  # Run tests during build
  doCheck = true;

  meta = with pkgs.lib; {
    description = "A unified CLI for AI generative models";
    homepage = "https://github.com/storyvis/kalpa";
    license = licenses.mit;
    maintainers = [ ];
    mainProgram = "kalpa";
  };
}
