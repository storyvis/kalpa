# Nix Configuration for kalpa

Production-ready Nix flake setup for building and running kalpa.

## Files

- `package.nix` — Production build configuration
- `shell.nix` — Development environment
- `default.nix` — Legacy nix-shell support
- `build.nix` — Standalone build (legacy)
- `run.nix` — Standalone run (legacy)

## Quick Start (Flakes)

```bash
# Build the project
nix build .#kalpa

# Run kalpa
nix run .#kalpa -- --help
nix run .#kalpa -- configure
nix run .#kalpa -- auth -g
nix run .#kalpa -- generate -g text "Hello"

# Enter development shell
nix develop
```

## Development

```bash
# Enter dev shell
nix develop

# Or with direnv
direnv allow

# Build and test
cargo build
cargo test
cargo run -- --help

# Use just for common tasks
just build
just test
just run --help
```

## CI/CD

```bash
# Build for production
nix build .#kalpa

# Check flake
nix flake check

# Binary is at
./result/bin/kalpa
```

## Without Flakes

For systems without flakes enabled:

```bash
# Development shell
nix-shell .nix/default.nix

# Build
nix-build .nix/build.nix

# Run
nix-shell .nix/run.nix --run "kalpa --help"
```

## Without Nix

Standard Rust workflow:

```bash
cargo build --release
cargo run -- --help
```
