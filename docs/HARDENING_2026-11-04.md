# Provider hardening and schema refresh (2026-11-04)

This change fixes confirmed transport, concurrency, credential and generated-wire
contract defects. It retains existing public provider constructors and model
selection. No live model inference, billable generation or production service
changes were used for validation.

## Changes and contracts

- All core provider clients use a 10-second connection timeout, 300-second request
  timeout and no redirects. Vertex polling and GCS reads reuse its authenticated
  client. OAuth exchanges have a 30-second deadline.
- Successful provider bodies are streamed with a 64 MiB cap before deserialization,
  including every generated operation. OAuth token responses are limited to 64 KiB;
  downloaded transcription input is limited to 25 MiB. Oversized responses fail
  without buffering the entire stream. Generated-code transformations assert their
  expected boundaries so generator drift fails the build.
- Error bodies are not reflected into errors/logs. Generated errors preserve HTTP
  status, including 429. HTTP errors remove URLs. Credentials use sensitive header
  values; Gemini sends its key in `x-goog-api-key`, not the query string. Debug output
  for configuration, service accounts and OAuth tokens redacts secrets.
- Fal queue URLs must use `https://queue.fal.run` on port 443 without URL credentials.
  Redirects cannot forward default authentication headers. Composite model paths
  preserve separators while escaping individual segments; request IDs retain path
  encoding. The optional official backup domain is not enabled implicitly.
- Audio URL downloads require public HTTPS on port 443. DNS is resolved once,
  checked for private/reserved/link-local/mapped addresses and pinned into the
  client; proxies and redirects are disabled. Legacy HTTP/internal audio URLs now
  fail. Callers that need local audio should supply an upload flow outside this
  downloader, rather than weakening its SSRF policy.
- Dispatcher generations have a configurable 15-minute deadline including admission, speech and transcription.
  Poll-time 429/503/timeouts back off without reducing admission. Submit-time
  back-pressure and successful completions adapt both provider and binding limits.
  Binding admission happens before taking shared provider capacity. Known Fal jobs
  receive best-effort cancellation on timeout/error/caller abort, bounded to 10
  seconds. Legacy Fal waits also have a 15-minute limit; Vertex operation polling
  has a 10-minute wall-clock deadline.
- AIMD normalizes invalid seeds to positive bounded capacity, avoids coalesced wakeup
  losses, uses saturating arithmetic, and preserves outstanding permits on catalog
  reload. Registry binding keys are tuples, avoiding separator collisions.
- Invalid retry configurations return errors instead of panicking. Only connection
  errors/timeouts, rate limits and selected 5xx responses are retried. Retry logs
  omit provider error bodies. Callers remain responsible for idempotency when using
  the generic retry helper; generation submission retries only explicit 429 rejections, preserving the upstream idempotency policy.
- Job IDs use UUIDs, do not slice UTF-8 provider names and cannot traverse paths.
  Job/config writes replace files atomically with owner-only permissions on Unix.
- OAuth allows only documented Google token endpoints. RS256 signing uses `ring`
  with its secure RNG and supports PKCS#1 and PKCS#8 keys. Synthetic fixture tests
  verify signatures. RustCrypto `rsa`, including the dependency flagged by
  [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html), is removed.
  The old use was JWT signing, not RSA decryption; this review does not claim a
  demonstrated exploitable decryption oracle or recovered key.

## Dependencies and generated API changes

The lockfile is refreshed to current compatible dependencies, including Progenitor
0.15.1, reqwest 0.13.5, toml 1.2, dialoguer 0.12, indicatif 0.18 and colored 3.1.
The unused Progenitor macro feature is disabled, removing deprecated `serde_yaml`
and unmaintained `unsafe-libyaml`. Final cargo-audit reports **zero vulnerabilities
and zero warnings**. `KalpaError::Http` now contains reqwest 0.13's error type.

See [schema provenance and regeneration](../crates/libgen/specs/README.md) for the
four complete official snapshots and their bounded compatibility projections.
Generated API changes include vectors for defaulted array fields, nonzero numeric
counts where constrained, OpenAI chat usage names and `max_completion_tokens`,
nullable Fal queue URLs, and generic Vertex operation results. Gemini's previous
incorrect single-object streaming method was removed; non-streaming generation
remains supported. Claude's schema is explicitly local, not claimed as a fresh
upstream specification.

## Validation

- Six focused baseline tests failed before fixes: invalid limiter min/max and zero
  floor, retry zero attempts/invalid multiplier, Unicode job ID and path traversal.
- `cargo test --workspace --locked`: 57 passing tests (43 core, five catalog integration tests, five generated wire
  contracts, four loopback generated HTTP tests), plus the pre-existing ignored
  retry documentation example. A separately invoked ignored microbenchmark checks
  the connection-reuse improvement: 100 loopback requests opened 100 connections
  with a new client per call versus one pooled connection (22.3 ms versus 9.5 ms
  in one local run). This is a connection-overhead microbenchmark, not a provider
  latency or production throughput claim.
- Four official snapshot hashes and exact offline regenerated projections pass
  `python3 scripts/schema-refresh.py --check`.
- Loopback tests cover chunked body caps, early Content-Length rejection, preserved
  429 status without reflected secrets, model/request path encoding, and disabled
  credential redirects. No provider credentials are used.
- Concurrency tests cover 6,400 acquisitions across 64 tasks on four workers, hot
  reload with an outstanding permit, poll throttling, timeout and caller abort.
- Cargo audit ran against the final lockfile on an isolated CPU host; temporary
  lockfile-only audit directories were removed afterward.

To repeat locally in the development shell:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test --workspace --locked
python3 scripts/schema-refresh.py --check
cargo test -p kalpa-libgen --test transport benchmark_client_reuse -- --ignored --nocapture
cargo audit --file Cargo.lock
```

## Remaining limits

There is no claim of full provider API coverage or live model availability.
Snapshots preserve full upstream sources, while generated clients intentionally
cover implemented operations and selected fields. Vertex model-specific prediction
payloads remain documented local overlays. Existing model aliases may refer to
retired models; they are not silently remapped to another paid model.

The upstream Vertex service-account token cache and serialized refresh APIs are
preserved and tested with concurrent callers, failed refresh and fresh credentials.
Providers constructed with only a bearer-token string still rely on the caller to
rotate that token. The existing global-endpoint selection remains unchanged.
Slurm generation and model-catalog APIs from origin/main f1dc241 are retained;
Slurm helper clients now also disable redirects, mark credentials sensitive and
apply 10-second connection/30-second request deadlines. Existing Slurm schemas are
preserved; this refresh does not claim a newly acquired Slurm API version. Cancellation is best-effort, requires a known job handle and a live Tokio runtime, and providers without
cancel support retain the trait's default behavior. An interrupted submission whose
response never supplies a handle cannot be cancelled reliably. Existing unrelated
compiler warnings in CLI authentication and Fal image dimensions remain.
