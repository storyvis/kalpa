# Run kalpa application
# Usage: nix-shell .nix/run.nix --run "kalpa --help"

{ pkgs ? import <nixpkgs> {} }:

let
  kalpa = import ./build.nix { inherit pkgs; };
in
pkgs.mkShell {
  buildInputs = [ kalpa ];

  shellHook = ''
    echo "kalpa is available in PATH"
    echo "Run: kalpa --help"
  '';
}
