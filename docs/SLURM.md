# Using kalpa-libgen Slurm from another project

Your backend owns the HTTP API and config. `kalpa-libgen` only supplies a typed Slurm REST client. Pass your cluster’s `slurmrestd` URL at runtime.

## 1. Depend on the crate

Path dependency (local checkout):

```toml
[dependencies]
kalpa-libgen = { path = "../kalpa/crates/libgen" }
tokio = { version = "1", features = ["full"] }
```

Or git:

```toml
kalpa-libgen = { git = "https://github.com/storyvis/kalpa.git" }
```

## 2. Create a client

```rust
use kalpa_libgen::slurm_client;

// base_url comes from your app config / env, e.g. http://slurmrestd:6820
let client = slurm_client(&base_url, &user, &token);
```

Auth options:

| Helper | Headers |
|--------|---------|
| `slurm_client(base_url, user, token)` | `X-SLURM-USER-NAME`, `X-SLURM-USER-TOKEN` |
| `slurm_client_bearer(base_url, jwt)` | `Authorization: Bearer …` |

## 3. Call Slurm from your API handlers

```rust
use kalpa_libgen::{slurm, slurm_client};

async fn list_jobs(base_url: &str, user: &str, token: &str) -> anyhow::Result<()> {
    let client = slurm_client(base_url, user, token);

    // Health
    let _ = client.slurm_v0045_get_ping().await?;

    // Jobs / nodes / partitions / accounting — all generated methods on Client
    let jobs = client.slurm_v0045_get_jobs(None, None).await?;

    // Types live under slurm::types
    let _submit: slurm::types::V0045JobSubmitReq;
    Ok(())
}
```

Your routes decide which cluster URL and credentials to pass; kalpa does not store them.

## 4. Useful surface

- Module: `kalpa_libgen::slurm`
- Client: `slurm::Client`
- Types: `slurm::types::*`
- Methods follow OpenAPI `operationId`s, e.g. `slurm_v0045_post_job_submit`, `slurm_v0045_get_nodes`, `slurmdb_v0045_get_users`

Inspect generated APIs under `crates/libgen/sdk/slurm.rs` after a build (copy for browsing only; compile uses `OUT_DIR`).

## Notes

- Spec: `crates/libgen/specs/slurm.json` (Slurm REST / slurmrestd).
- Rebuild `kalpa-libgen` after changing the spec: `cargo build -p kalpa-libgen`.
- No `kalpa-core` dependency is required for Slurm-only backends.
