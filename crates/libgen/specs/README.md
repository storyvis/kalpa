# Provider schema sources and regeneration

Four of the five generated providers now derive from official, pinned machine-readable
sources fetched on 2026-11-04. The root JSON files are **Kalpa compatibility
projections**, not complete provider specifications. The complete source bytes are
preserved as deterministic gzip snapshots under `upstream/`; `manifest.json` records
the source URL, upstream revision, retrieval date, uncompressed byte count and SHA-256.

| Provider | Official source | Projection coverage |
| --- | --- | --- |
| OpenAI | [OpenAI OpenAPI repository](https://github.com/openai/openai-openapi), pinned commit in the manifest | Non-streaming chat completions, image generation and video job creation; fields consumed by Kalpa plus `max_completion_tokens` |
| Gemini | [Google Discovery v1beta](https://generativelanguage.googleapis.com/$discovery/rest?version=v1beta) | `models.generateContent`: text, inline/file media, generation configuration and usage |
| Vertex | [Google Discovery v1](https://aiplatform.googleapis.com/$discovery/rest?version=v1) | Publisher-model generateContent, predict, predictLongRunning, and operation get |
| Fal | [Official Queue OpenAPI](https://fal.ai/docs/api-reference/inference-api/openapi/queue.json) | Queue submit, status and result; arbitrary model input/output |
| Claude | Existing locally maintained `claude.json`; [official SDK](https://github.com/anthropics/anthropic-sdk-python) is an upstream reference | Messages text subset; **not refreshed from an official OpenAPI document** |

No public official Anthropic raw OpenAPI document was established in this review.
The existing Claude schema is retained explicitly as a local adapter; it must not
be represented as an upstream OpenAPI snapshot or a complete Messages contract.

## Reproduce and update

From the repository development shell:

```sh
python3 scripts/schema-refresh.py --check  # offline: verify hashes and exact generated JSON
python3 scripts/schema-refresh.py          # offline: regenerate from pinned source bytes
python3 scripts/schema-refresh.py --fetch  # network: acquire current sources and regenerate
cargo test -p kalpa-libgen --test schema_contracts
cargo test --workspace
```

`--fetch` makes only unauthenticated schema/metadata GET requests. It resolves the
OpenAI main branch to an immutable commit before downloading. Google Discovery
revisions and Fal content hashes pin APIs without immutable source URLs. Review all
resulting diffs; changed upstream properties cause projection generation to fail
rather than silently restoring stale hand-authored fields. Cargo runs `build.rs`
to regenerate Rust clients from the root JSON files; no generated Rust is committed.

The projection script uses only the Python standard library. Build-time network
access is unnecessary. The full snapshots occupy about 1.1 MB compressed; generating
only implemented operations avoids pulling thousands of unrelated API types into
Kalpa consumers.

## Deliberate transformations and limits

- OpenAI and Fal publish OAS 3.1; Progenitor uses OAS 3.0. Nullable primitive type
  arrays/unions become OAS 3.0 `nullable`. Supported scalar branches are selected
  explicitly (for example, text message content); full multimodal/tool unions are
  retained in upstream snapshots but are outside these client projections.
- `allOf` object schemas are resolved before selecting fields. Every selected field
  must exist upstream. Request/response schema names and operation IDs retain
  Kalpa names to minimize consumer changes.
- String enums remain open strings except the existing OpenAI message role enum.
  This avoids rejecting new provider model IDs or output enum values. Numeric and
  timestamp formats remain ordinary Rust scalars for compatibility. Provider-side
  validation still applies; this projection is not a complete request validator.
- Response fields remain optional for forward/backward compatibility. Existing
  request requirements are retained where Kalpa requires them, even where upstream
  supports an omitted value (for example an image/video model). Provider-owned
  defaults are not materialized in requests.
- Google Discovery is an official schema format, **not OpenAPI**. Conversion keeps
  selected fields and canonical REST JSON names, maps schema references, and uses
  the documented flat paths with Kalpa parameter names. The Vertex publisher is
  fixed to `google`, matching the provider implementation.
- `overlays/vertex-models.json` contains the existing local Imagen/Veo model payload
  shapes. Google Discovery declares prediction instances, parameters and outputs
  as arbitrary JSON; it does not certify these model-specific overlays. They are
  marked local and remain separate from the official envelope. Operation responses
  are now generic objects so Veo-specific results survive deserialization.
- Fal uses a composite `model_id` parameter in place of official `{owner}/{app}`
  (plus optional endpoint path on submit), preserving Kalpa callers. Results and
  status use the canonical server-returned URLs. Queue URLs are nullable in the
  official shared response; consumers must check availability before polling.
- Gemini `streamGenerateContent` was removed from the projection: its previous
  generated signature incorrectly decoded a streaming body as a single response.
  Kalpa implements non-streaming generation. No streaming support is claimed.
- OpenAI chat usage now uses `prompt_tokens` and `completion_tokens`, separately
  from image usage `input_tokens`/`output_tokens`. Video `seconds` is a wire string;
  unsupported `quality` fields were removed. `max_completion_tokens` is available
  alongside deprecated `max_tokens` so consumers can choose the model contract.

`schema_contracts.rs` exercises wire regressions using synthetic response fixtures:
chat usage, Gemini media names, nullable Fal URLs, generic Vertex video results and
OpenAI string video durations. It does not make paid inference requests or establish
that every provider/model combination works live.
