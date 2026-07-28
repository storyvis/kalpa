//! # kalpa-libgen
//!
//! Type-safe Rust clients generated from OpenAPI 3.0 specs via
//! [progenitor](https://docs.rs/progenitor).
//!
//! ## Directory Structure
//!
//! ```text
//! crates/libgen/
//! ├── specs/          # OpenAPI 3.0 spec files (JSON)
//! ├── sdk/            # Generated SDK copies (for inspection)
//! └── src/
//!     └── lib.rs      # Re-exports generated SDKs
//! ```
//!
//! ## Adding a New Spec
//!
//! 1. Place the OpenAPI 3.0 spec in `specs/<name>.json`
//! 2. Add the module re-export below (`include!` from `OUT_DIR`)
//!
//! The build script generates client code at compile time.
//!
//! ## Using Slurm from another project
//!
//! Depend on this crate and pass your `slurmrestd` base URL at runtime:
//!
//! ```toml
//! kalpa-libgen = { path = "../kalpa/crates/libgen" }
//! ```
//!
//! ```rust,ignore
//! use kalpa_libgen::{slurm, slurm_client};
//!
//! let client = slurm_client("http://slurmrestd:6820", "alice", "secret-token");
//! let _ = client.slurm_v0045_get_ping().await?;
//! let _ = client.slurm_v0045_get_jobs(/* ... */).await?;
//! // request/response types: slurm::types::...
//! ```
//!
//! Prefer [`slurm_client`] / [`slurm_client_bearer`] so auth headers are set.
//! Or build your own `reqwest::Client` and call
//! [`slurm::Client::new_with_client`](slurm::Client::new_with_client).

mod slurm_auth;

pub use slurm_auth::{slurm_client, slurm_client_bearer};

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

/// Slurm REST API client generated from `specs/slurm.json` (slurmrestd OpenAPI).
///
/// Construct with [`crate::slurm_client`] or [`crate::slurm_client_bearer`],
/// passing the cluster `slurmrestd` URL at runtime.
pub mod slurm {
    include!(concat!(env!("OUT_DIR"), "/slurm.rs"));
}

/// Re-export for convenience in downstream crates.
pub use progenitor_client;
