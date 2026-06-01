{
  description = "kalpa - A unified CLI for AI generative models";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        
        # Import modular Nix files
        kalpa-package = import ./.nix/package.nix { inherit pkgs; };
        kalpa-shell = import ./.nix/shell.nix { inherit pkgs; };
        
      in {
        # nix build .#kalpa
        packages = {
          default = kalpa-package;
          kalpa = kalpa-package;
        };

        # nix run .#kalpa
        apps = {
          default = {
            type = "app";
            program = "${kalpa-package}/bin/kalpa";
          };
          kalpa = {
            type = "app";
            program = "${kalpa-package}/bin/kalpa";
          };
        };

        # nix develop
        devShells.default = kalpa-shell;
      }
    );
}
