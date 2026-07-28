//! Helpers for constructing authenticated Slurm REST (`slurmrestd`) clients.
//!
//! The generated [`crate::slurm::Client`] takes a base URL at construction time.
//! Pass your cluster's `slurmrestd` origin (for example `http://slurmrestd:6820`)
//! from the calling project at runtime.

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

use crate::slurm;

/// Build a Slurm client using `X-SLURM-USER-NAME` and `X-SLURM-USER-TOKEN`.
///
/// # Arguments
/// * `base_url` — `slurmrestd` origin, e.g. `http://cluster.example:6820`
/// * `user` — Slurm username (`X-SLURM-USER-NAME`)
/// * `token` — Slurm user token (`X-SLURM-USER-TOKEN`)
pub fn slurm_client(base_url: &str, user: &str, token: &str) -> slurm::Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-SLURM-USER-NAME",
        HeaderValue::from_str(user).expect("invalid X-SLURM-USER-NAME header value"),
    );
    headers.insert(
        "X-SLURM-USER-TOKEN",
        HeaderValue::from_str(token).expect("invalid X-SLURM-USER-TOKEN header value"),
    );

    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("failed to build reqwest client");

    slurm::Client::new_with_client(base_url, http)
}

/// Build a Slurm client using `Authorization: Bearer <token>` (JWT).
///
/// # Arguments
/// * `base_url` — `slurmrestd` origin, e.g. `http://cluster.example:6820`
/// * `bearer` — JWT / bearer token (without the `Bearer ` prefix)
pub fn slurm_client_bearer(base_url: &str, bearer: &str) -> slurm::Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", bearer))
            .expect("invalid Authorization header value"),
    );

    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("failed to build reqwest client");

    slurm::Client::new_with_client(base_url, http)
}
