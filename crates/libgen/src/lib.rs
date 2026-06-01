//! # kalpa-libgen
//!
//! Client SDK generation library for AI provider APIs.
//!
//! This crate uses [progenitor](https://docs.rs/progenitor) to generate
//! type-safe Rust client libraries from OpenAPI 3.0 specification files.
//!
//! ## Directory Structure
//!
//! ```text
//! crates/libgen/
//! ├── specs/          # OpenAPI 3.0 spec files (JSON/YAML)
//! │   ├── gemini.json
//! │   ├── vertex.json
//! │   └── fal.json
//! ├── sdk/            # Generated SDK modules (output)
//! │   ├── gemini.rs
//! │   ├── vertex.rs
//! │   └── fal.rs
//! └── src/
//!     └── lib.rs      # This file - re-exports generated SDKs
//! ```
//!
//! ## Adding a New Provider
//!
//! 1. Place the OpenAPI 3.0 spec in `specs/<provider>.json`
//! 2. Add the `generate_api!` invocation in `build.rs`
//! 3. Add the module re-export below
//!
//! The build script will automatically generate the client code at compile time.

// Generated SDK modules - automatically included from build.rs output

pub mod gemini {
    include!(concat!(env!("OUT_DIR"), "/gemini.rs"));
}

pub mod vertex {
    include!(concat!(env!("OUT_DIR"), "/vertex.rs"));
}

pub mod openai {
    include!(concat!(env!("OUT_DIR"), "/openai.rs"));
}

pub mod falai {
    include!(concat!(env!("OUT_DIR"), "/falai.rs"));
}

pub mod claude {
    include!(concat!(env!("OUT_DIR"), "/claude.rs"));
}

/// Re-export for convenience in downstream crates.
pub use progenitor_client;
