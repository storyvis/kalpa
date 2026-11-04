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
        HeaderValue::from_str(user).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert("X-SLURM-USER-TOKEN", sensitive(token));

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
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
    headers.insert(AUTHORIZATION, sensitive(&format!("Bearer {}", bearer)));

    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .default_headers(headers)
        .build()
        .expect("failed to build reqwest client");

    slurm::Client::new_with_client(base_url, http)
}

fn sensitive(value: &str) -> HeaderValue {
    let mut header = HeaderValue::from_str(value).unwrap_or_else(|_| HeaderValue::from_static(""));
    header.set_sensitive(true);
    header
}
