//! Google Gemini (Generative Language API) provider — direct API-key access.
//!
//! Distinct from [`super::vertex`] (which uses OAuth + a GCP project), though
//! both speak the same `generateContent` and share this module's request and
//! response mapping.
//!
//! - `{model}:generateContent` returns the model's interleaved
//!   `candidates[].content.parts[]` as our uniform [`Part`] envelope — so an
//!   image model ("Nano Banana") yields text and image parts in one
//!   synchronous response.
//! - Video is an Omni model (`gemini-omni-…`) through the Interactions API
//!   (`/interactions`): one call, the video delivered by reference and
//!   fetched with the key. Veo is no longer served by the Gemini API; its
//!   mapping lives here for Vertex, which still runs it as an operation.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{KalpaError, KalpaResult};
use crate::generation::{GenerationRequest, GenerationResponse, Modality, Part};
use crate::http::check_response;
use crate::provider::{GenerationProvider, JobHandle, PollStatus, SubmitOutcome};
use crate::types::Usage;

const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// The most images one request may ask for (one call each, run together).
const MAX_IMAGES: u64 = 10;

/// Direct Gemini API provider.
#[derive(Clone)]
pub struct GeminiProvider {
    api_key: String,
    http: reqwest::Client,
    base_url: String,
}

// ── wire types (subset of the generateContent schema) ──

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
    usage_metadata: Option<UsageMetadata>,
    prompt_feedback: Option<PromptFeedback>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    content: Option<GContent>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct GContent {
    #[serde(default)]
    parts: Vec<GPart>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GPart {
    text: Option<String>,
    /// The model's reasoning, not its answer.
    #[serde(default)]
    thought: bool,
    inline_data: Option<InlineData>,
    file_data: Option<FileData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: Option<String>,
    data: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileData {
    mime_type: Option<String>,
    file_uri: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageMetadata {
    #[serde(default)]
    prompt_token_count: u32,
    #[serde(default)]
    candidates_token_count: u32,
    /// Billed as output, and not part of `candidatesTokenCount`.
    #[serde(default)]
    thoughts_token_count: u32,
    #[serde(default)]
    total_token_count: u32,
    /// The output by modality: an image's own tokens are `IMAGE`.
    #[serde(default)]
    candidates_tokens_details: Vec<ModalityTokens>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModalityTokens {
    modality: Option<String>,
    #[serde(default)]
    token_count: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptFeedback {
    block_reason: Option<String>,
}

// ── request and response mapping, shared with Vertex ──

/// The aspect ratios Gemini image models and Veo take.
const ASPECT_RATIOS: [(&str, f64); 14] = [
    ("1:1", 1.0),
    ("1:4", 0.25),
    ("4:1", 4.0),
    ("1:8", 0.125),
    ("8:1", 8.0),
    ("2:3", 2.0 / 3.0),
    ("3:2", 3.0 / 2.0),
    ("3:4", 3.0 / 4.0),
    ("4:3", 4.0 / 3.0),
    ("4:5", 4.0 / 5.0),
    ("5:4", 5.0 / 4.0),
    ("9:16", 9.0 / 16.0),
    ("16:9", 16.0 / 9.0),
    ("21:9", 21.0 / 9.0),
];

/// OpenAI's `"1536x1024"` as (width, height).
fn dimensions(size: &str) -> Option<(u32, u32)> {
    let (w, h) = size.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// The nearest aspect ratio Google names, from `allowed`.
fn nearest_ratio(width: u32, height: u32, allowed: &[&str]) -> &'static str {
    let want = (width as f64 / height as f64).ln();
    ASPECT_RATIOS
        .iter()
        .filter(|(name, _)| allowed.is_empty() || allowed.contains(name))
        .min_by(|a, b| (a.1.ln() - want).abs().total_cmp(&(b.1.ln() - want).abs()))
        .map_or("1:1", |(name, _)| name)
}

fn param<'a>(params: Option<&'a Value>, names: &[&str]) -> Option<&'a Value> {
    let params = params?.as_object()?;
    names
        .iter()
        .find_map(|n| params.get(*n))
        .filter(|v| !v.is_null())
}

fn bad_request(message: impl Into<String>) -> KalpaError {
    KalpaError::ProviderError {
        status: 400,
        message: message.into(),
    }
}

/// `data:image/png;base64,AAAA` as (mime, base64).
fn data_url(url: &str) -> Option<(&str, &str)> {
    let (meta, data) = url.strip_prefix("data:")?.split_once(',')?;
    Some((meta.strip_suffix(";base64")?, data))
}

/// Our input parts as `generateContent` parts. Media is sent inline: a
/// `fileData` URI must be one Google already holds, so any other URL is
/// refused rather than dropped.
fn input_parts(parts: &[Part]) -> KalpaResult<Vec<Value>> {
    let inline = |mime: &str, data: &str| json!({"inlineData": {"mimeType": mime, "data": data}});
    let media = |url: Option<&str>, b64: Option<&str>, mime: Option<&str>, default: &str| {
        if let Some(data) = b64 {
            return Ok(inline(mime.unwrap_or(default), data));
        }
        match url.and_then(data_url) {
            Some((mime, data)) => Ok(inline(mime, data)),
            None => Err(bad_request(
                "gemini takes input media inline (base64 or a data: URL), not by URL",
            )),
        }
    };
    parts
        .iter()
        .filter_map(|part| match part {
            Part::Text { text } => Some(Ok(json!({"text": text}))),
            Part::Image {
                url,
                b64_data,
                mime,
            } => Some(media(
                url.as_deref(),
                b64_data.as_deref(),
                mime.as_deref(),
                "image/png",
            )),
            Part::Audio {
                url,
                b64_data,
                mime,
            } => Some(media(
                url.as_deref(),
                b64_data.as_deref(),
                mime.as_deref(),
                "audio/wav",
            )),
            Part::Video { url, mime } => Some(media(Some(url), None, mime.as_deref(), "video/mp4")),
            Part::Embedding { .. } => None,
        })
        .collect()
}

/// The `generateContent` body for a request. `params` are read by their
/// OpenAI names: `size` (`"1536x1024"`: the nearest aspect ratio, and a 2K or
/// 4K output when it asks for one), `aspect_ratio`, `image_size`,
/// `temperature`, `seed`, `max_output_tokens`.
pub(crate) fn generate_content_body(request: &GenerationRequest) -> KalpaResult<Value> {
    let params = request.params.as_ref();
    let mut config = serde_json::Map::new();
    if let Some(modalities) = &request.response_modalities {
        let names: Vec<&str> = modalities
            .iter()
            .map(|m| match m {
                Modality::Image => "IMAGE",
                Modality::Audio => "AUDIO",
                _ => "TEXT",
            })
            .collect();
        config.insert("responseModalities".into(), json!(names));
    }

    let mut image = serde_json::Map::new();
    let size = param(params, &["size"])
        .and_then(Value::as_str)
        .and_then(dimensions);
    if let Some(ratio) = param(params, &["aspect_ratio", "aspectRatio"]).and_then(Value::as_str) {
        image.insert("aspectRatio".into(), ratio.into());
    } else if let Some((w, h)) = size {
        image.insert("aspectRatio".into(), nearest_ratio(w, h, &[]).into());
    }
    if let Some(res) = param(params, &["image_size", "imageSize"]).and_then(Value::as_str) {
        image.insert("imageSize".into(), res.into());
    } else if let Some(longest) = size.map(|(w, h)| w.max(h)).filter(|l| *l > 1536) {
        // 1K is every model's default; only a larger ask is spelled out.
        image.insert(
            "imageSize".into(),
            if longest > 2048 { "4K" } else { "2K" }.into(),
        );
    }
    if !image.is_empty() {
        config.insert("imageConfig".into(), Value::Object(image));
    }

    for (name, key) in [
        ("temperature", "temperature"),
        ("seed", "seed"),
        ("max_output_tokens", "maxOutputTokens"),
    ] {
        if let Some(v) = param(params, &[name]).filter(|v| v.is_number()) {
            config.insert(key.into(), v.clone());
        }
    }

    let mut body = json!({"contents": [{"role": "user", "parts": input_parts(&request.parts)?}]});
    if !config.is_empty() {
        body["generationConfig"] = Value::Object(config);
    }
    Ok(body)
}

/// A finish reason that means the safety system withheld the output.
fn refused(reason: &str) -> bool {
    // Vertex documents the same values with a prefix.
    matches!(
        reason.strip_prefix("FINISH_REASON_").unwrap_or(reason),
        "SAFETY"
            | "IMAGE_SAFETY"
            | "PROHIBITED_CONTENT"
            | "IMAGE_PROHIBITED_CONTENT"
            | "BLOCKLIST"
            | "RECITATION"
            | "IMAGE_RECITATION"
            | "SPII"
    )
}

/// A `generateContent` reply as our response. A blocked prompt or a withheld
/// output is a 400 the caller can tell apart (`content_policy`); an image
/// request that produced no image is an error, not an empty success.
pub(crate) fn parse_generate_content(
    model: &str,
    wants_image: bool,
    body: Value,
) -> KalpaResult<GenerationResponse> {
    let parsed: GenerateContentResponse =
        serde_json::from_value(body).map_err(|_| KalpaError::ProviderError {
            status: 502,
            message: "Provider returned an invalid response".into(),
        })?;
    let usage = parsed.usage_metadata.map(|u| {
        let completion = u.candidates_token_count + u.thoughts_token_count;
        // An image model counts its thinking inside `candidatesTokenCount`
        // and details only the image: what is left over is billed as text.
        let media: u32 = u
            .candidates_tokens_details
            .iter()
            .filter(|d| matches!(d.modality.as_deref(), Some("IMAGE" | "VIDEO" | "AUDIO")))
            .map(|d| d.token_count)
            .sum();
        let reasoning = if media > 0 {
            completion.saturating_sub(media)
        } else {
            u.thoughts_token_count
        };
        Usage {
            prompt_tokens: u.prompt_token_count,
            completion_tokens: completion,
            total_tokens: u.total_token_count.max(u.prompt_token_count + completion),
            reasoning_tokens: (reasoning > 0).then_some(reasoning),
        }
    });
    if let Some(reason) = parsed.prompt_feedback.and_then(|f| f.block_reason) {
        return Err(bad_request(format!(
            "content_policy: the prompt was blocked ({reason})"
        )));
    }

    let candidate = parsed.candidates.into_iter().next();
    let finish = candidate.as_ref().and_then(|c| c.finish_reason.clone());
    let mut parts = Vec::new();
    for part in candidate
        .and_then(|c| c.content)
        .map(|c| c.parts)
        .unwrap_or_default()
    {
        if part.thought {
            continue;
        }
        if let Some(text) = part.text.filter(|t| !t.is_empty()) {
            parts.push(Part::Text { text });
        } else if let Some(inline) = part.inline_data {
            let mime = inline.mime_type.unwrap_or_else(|| "image/png".into());
            parts.push(if mime.starts_with("audio/") {
                Part::Audio {
                    url: None,
                    b64_data: Some(inline.data),
                    mime: Some(mime),
                }
            } else if mime.starts_with("video/") {
                Part::Video {
                    url: format!("data:{mime};base64,{}", inline.data),
                    mime: Some(mime),
                }
            } else {
                Part::Image {
                    url: None,
                    b64_data: Some(inline.data),
                    mime: Some(mime),
                }
            });
        } else if let Some(file) = part.file_data {
            let mime = file.mime_type;
            parts.push(match mime.as_deref() {
                Some(m) if m.starts_with("video/") => Part::Video {
                    url: file.file_uri,
                    mime,
                },
                _ => Part::Image {
                    url: Some(file.file_uri),
                    b64_data: None,
                    mime,
                },
            });
        }
    }

    let has_image = parts.iter().any(|p| matches!(p, Part::Image { .. }));
    if wants_image && !has_image {
        let reason = finish.as_deref().unwrap_or("no finish reason");
        return Err(if refused(reason) {
            bad_request(format!("content_policy: the image was withheld ({reason})"))
        } else {
            KalpaError::ProviderError {
                status: 502,
                message: format!("gemini returned no image ({reason})"),
            }
        });
    }
    if parts.is_empty() && finish.as_deref().is_some_and(refused) {
        return Err(bad_request(format!(
            "content_policy: the output was withheld ({})",
            finish.unwrap_or_default()
        )));
    }

    Ok(GenerationResponse {
        model: model.to_string(),
        parts,
        usage,
    })
}

/// How many images a request asks for (`n`), within [`MAX_IMAGES`].
pub(crate) fn image_count(request: &GenerationRequest) -> usize {
    param(request.params.as_ref(), &["n"])
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, MAX_IMAGES) as usize
}

/// `n` single-image replies as one: every image, the usage summed. A failed
/// call is dropped while another succeeded (those are billed); with none, the
/// first failure is the answer.
pub(crate) fn merge_images(
    results: Vec<KalpaResult<GenerationResponse>>,
) -> KalpaResult<GenerationResponse> {
    let mut merged: Option<GenerationResponse> = None;
    let mut first_error = None;
    for result in results {
        match (result, &mut merged) {
            (Ok(response), None) => merged = Some(response),
            (Ok(response), Some(all)) => {
                all.parts.extend(response.parts);
                all.usage = match (all.usage.take(), response.usage) {
                    (Some(a), Some(b)) => Some(Usage {
                        prompt_tokens: a.prompt_tokens + b.prompt_tokens,
                        completion_tokens: a.completion_tokens + b.completion_tokens,
                        total_tokens: a.total_tokens + b.total_tokens,
                        reasoning_tokens: match (a.reasoning_tokens, b.reasoning_tokens) {
                            (None, None) => None,
                            (x, y) => Some(x.unwrap_or(0) + y.unwrap_or(0)),
                        },
                    }),
                    (a, b) => a.or(b),
                };
            }
            (Err(error), _) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match (merged, first_error) {
        (Some(response), _) => Ok(response),
        (None, Some(error)) => Err(error),
        (None, None) => Err(KalpaError::Other("no generation was attempted".into())),
    }
}

/// Whether a model is one of the Veo video models (a long-running job).
pub(crate) fn is_veo(model: &str) -> bool {
    model.starts_with("veo")
}

/// The `predictLongRunning` body for a Veo request. `params`: `seconds` (or
/// `duration`), `size` (`"1280x720"`: aspect ratio and resolution),
/// `aspect_ratio`, `resolution`, `negative_prompt`. A first input image makes
/// it image-to-video.
pub(crate) fn veo_body(request: &GenerationRequest) -> KalpaResult<Value> {
    let params = request.params.as_ref();
    let mut instance = json!({"prompt": request.text_prompt()?});
    let image = request.parts.iter().find_map(|p| match p {
        Part::Image {
            url,
            b64_data,
            mime,
        } => Some((url.as_deref(), b64_data.as_deref(), mime.as_deref())),
        _ => None,
    });
    if let Some((url, b64, mime)) = image {
        let (mime, data) = match (b64, url.and_then(data_url)) {
            (Some(data), _) => (mime.unwrap_or("image/png"), data),
            (None, Some(inline)) => inline,
            (None, None) => {
                return Err(bad_request(
                    "veo takes its input image inline (base64 or a data: URL), not by URL",
                ));
            }
        };
        instance["image"] = json!({"bytesBase64Encoded": data, "mimeType": mime});
    }

    let mut parameters = serde_json::Map::new();
    let size = param(params, &["size"])
        .and_then(Value::as_str)
        .and_then(dimensions);
    if let Some(ratio) = param(params, &["aspect_ratio", "aspectRatio"]).and_then(Value::as_str) {
        parameters.insert("aspectRatio".into(), ratio.into());
    } else if let Some((w, h)) = size {
        parameters.insert(
            "aspectRatio".into(),
            nearest_ratio(w, h, &["16:9", "9:16"]).into(),
        );
    }
    if let Some(res) = param(params, &["resolution"]).and_then(Value::as_str) {
        parameters.insert("resolution".into(), res.into());
    } else if let Some((w, h)) = size {
        let short = w.min(h);
        let res = if short > 1080 {
            "4k"
        } else if short > 720 {
            "1080p"
        } else {
            "720p"
        };
        parameters.insert("resolution".into(), res.into());
    }
    let seconds = param(params, &["seconds", "duration"]).and_then(|s| match s {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    });
    if let Some(seconds) = seconds {
        parameters.insert("durationSeconds".into(), seconds.into());
    }
    if let Some(negative) =
        param(params, &["negative_prompt", "negativePrompt"]).and_then(Value::as_str)
    {
        parameters.insert("negativePrompt".into(), negative.into());
    }

    let mut body = json!({"instances": [instance]});
    if !parameters.is_empty() {
        body["parameters"] = Value::Object(parameters);
    }
    Ok(body)
}

/// What a polled Veo operation says.
pub(crate) enum VeoState {
    Running,
    Failed(String),
    /// Where the video is: a URI to fetch, or its bytes (base64) and type.
    Uri(String),
    Inline {
        mime: String,
        data: String,
    },
}

/// Read a Veo operation: the Gemini API's `generateVideoResponse` and
/// Vertex's `videos[]` are both understood.
pub(crate) fn veo_state(operation: &Value) -> VeoState {
    if operation["done"] != true {
        return VeoState::Running;
    }
    if let Some(message) = operation["error"]["message"].as_str() {
        return VeoState::Failed(crate::http::scrub(message));
    }
    let response = &operation["response"];
    let gemini = &response["generateVideoResponse"];
    if let Some(uri) = gemini["generatedSamples"][0]["video"]["uri"].as_str() {
        return VeoState::Uri(uri.to_string());
    }
    let video = &response["videos"][0];
    if let Some(data) = video["bytesBase64Encoded"].as_str() {
        return VeoState::Inline {
            mime: video["mimeType"]
                .as_str()
                .unwrap_or("video/mp4")
                .to_string(),
            data: data.to_string(),
        };
    }
    if let Some(uri) = video["gcsUri"].as_str() {
        return VeoState::Uri(uri.to_string());
    }
    // Withheld by the safety filters: say why, as the caller's refusal.
    let filtered = [gemini, response]
        .into_iter()
        .find_map(|r| r["raiMediaFilteredReasons"][0].as_str());
    VeoState::Failed(match filtered {
        Some(reason) => format!(
            "content_policy: the video was withheld: {}",
            crate::http::scrub(reason)
        ),
        None => "the operation finished without a video".into(),
    })
}

// ── video on the Gemini API: an Omni model through the Interactions API ──

/// Output tokens a second of Omni video is billed, by resolution.
const OMNI_TOKENS_PER_SECOND: [(&str, u32); 4] = [
    ("360p", 1_931),
    ("720p", 5_792),
    ("1080p", 8_688),
    ("4k", 17_376),
];

/// The resolution and length (seconds) an Omni request asks for.
fn omni_shape(params: Option<&Value>) -> (&'static str, u32) {
    let named = param(params, &["resolution"])
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    let short = param(params, &["size"])
        .and_then(Value::as_str)
        .and_then(dimensions)
        .map(|(w, h)| w.min(h));
    let resolution = OMNI_TOKENS_PER_SECOND
        .iter()
        .map(|(name, _)| *name)
        .find(|name| named.as_deref() == Some(name))
        .unwrap_or(match short {
            Some(s) if s > 1080 => "4k",
            Some(s) if s > 720 => "1080p",
            Some(s) if s < 720 => "360p",
            _ => "720p",
        });
    let seconds = param(params, &["seconds", "duration"])
        .and_then(|s| match s {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().trim_end_matches('s').parse().ok(),
            _ => None,
        })
        .unwrap_or(10)
        .clamp(3, 15) as u32;
    (resolution, seconds)
}

/// The Interactions body for an Omni video. `params`: `seconds` (or
/// `duration`; 3–15, default 10), `size` (`"1280x720"`: aspect ratio and
/// resolution), `aspect_ratio`, `resolution`. An input image makes it
/// image-to-video. `by_reference`: ask for the video as a file to fetch
/// (the Gemini API, where an inline one is limited to a few megabytes);
/// Vertex delivers a reference only to a storage bucket, so it is asked
/// for the bytes.
pub(crate) fn omni_body(
    model: &str,
    request: &GenerationRequest,
    by_reference: bool,
) -> KalpaResult<Value> {
    let params = request.params.as_ref();
    let mut input = Vec::new();
    for part in &request.parts {
        match part {
            Part::Text { text } => input.push(json!({"type": "text", "text": text})),
            Part::Image {
                url,
                b64_data,
                mime,
            } => {
                let (mime, data) = match (b64_data.as_deref(), url.as_deref().and_then(data_url)) {
                    (Some(data), _) => (mime.as_deref().unwrap_or("image/png"), data),
                    (None, Some(inline)) => inline,
                    (None, None) => {
                        return Err(bad_request(
                            "gemini takes input media inline (base64 or a data: URL), not by URL",
                        ));
                    }
                };
                input.push(json!({"type": "image", "data": data, "mime_type": mime}));
            }
            _ => {}
        }
    }
    let (resolution, seconds) = omni_shape(params);
    let mut format = json!({
        "type": "video",
        "resolution": resolution,
        "duration": format!("{seconds}s"),
    });
    if by_reference {
        format["delivery"] = "uri".into();
    }
    if let Some(ratio) = param(params, &["aspect_ratio", "aspectRatio"]).and_then(Value::as_str) {
        format["aspect_ratio"] = ratio.into();
    } else if let Some((w, h)) = param(params, &["size"])
        .and_then(Value::as_str)
        .and_then(dimensions)
    {
        format["aspect_ratio"] = nearest_ratio(w, h, &["16:9", "9:16"]).into();
    }
    Ok(json!({"model": model, "input": input, "response_format": format}))
}

/// The video in an interaction: `(mime, data or uri)` from the model's
/// output step, wherever its content nests it.
pub(crate) fn omni_video(value: &Value) -> Option<(String, OmniVideo)> {
    match value {
        Value::Object(map) if map.get("type").and_then(Value::as_str) == Some("video") => {
            let mime = map
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("video/mp4")
                .to_string();
            if let Some(data) = map.get("data").and_then(Value::as_str) {
                return Some((mime, OmniVideo::Data(data.to_string())));
            }
            let uri = map.get("uri").and_then(Value::as_str)?;
            Some((mime, OmniVideo::Uri(uri.to_string())))
        }
        Value::Object(map) => map.values().find_map(omni_video),
        Value::Array(items) => items.iter().find_map(omni_video),
        _ => None,
    }
}

pub(crate) enum OmniVideo {
    Data(String),
    Uri(String),
}

/// An interaction's usage; when it reports none, what the video's length and
/// resolution are billed (so a video is never metered as free).
pub(crate) fn omni_usage(interaction: &Value, params: Option<&Value>) -> Usage {
    let usage = &interaction["usage"];
    let n = |names: &[&str]| {
        names
            .iter()
            .find_map(|k| usage[*k].as_u64())
            .map(|v| v.min(u32::MAX as u64) as u32)
    };
    let prompt = n(&["total_input_tokens", "input_tokens"]).unwrap_or(0);
    let output = n(&["total_output_tokens", "output_tokens"]);
    let thoughts = n(&["total_thought_tokens", "thought_tokens"]).unwrap_or(0);
    let completion = match output {
        Some(output) => output + thoughts,
        None => {
            let (resolution, seconds) = omni_shape(params);
            let per_second = OMNI_TOKENS_PER_SECOND
                .iter()
                .find(|(name, _)| *name == resolution)
                .map_or(0, |(_, tokens)| *tokens);
            per_second * seconds
        }
    };
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: prompt + completion,
        // Thinking beside the video is billed at the text rate.
        reasoning_tokens: (output.is_some() && thoughts > 0).then_some(thoughts),
    }
}

/// Whether a request is for video on the Gemini API.
fn wants_video(model: &str, request: &GenerationRequest) -> bool {
    model.starts_with("gemini-omni")
        || request
            .response_modalities
            .as_ref()
            .is_some_and(|m| m.contains(&Modality::Video))
}

impl GeminiProvider {
    /// Create a provider from a Gemini API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            // A thinking model or a video takes minutes: Google asks for a
            // client timeout of 15 minutes or more.
            http: crate::http::client_builder()
                .timeout(std::time::Duration::from_secs(900))
                .build()
                .expect("HTTP client initialization"),
            base_url: BASE_URL.into(),
        }
    }

    /// Point at another endpoint (a proxy, a test server). `base` includes
    /// the version path, e.g. `https://host/v1beta`.
    pub fn with_base_url(mut self, base: &str) -> Self {
        self.base_url = base.trim_end_matches('/').to_string();
        self
    }

    fn key(&self) -> reqwest::header::HeaderValue {
        crate::http::secret_header(&self.api_key)
    }

    async fn generate(
        &self,
        model: &str,
        wants_image: bool,
        body: &Value,
    ) -> KalpaResult<GenerationResponse> {
        let url = format!("{}/models/{model}:generateContent", self.base_url);
        let resp = self
            .http
            .post(&url)
            .header("x-goog-api-key", self.key())
            .json(body)
            .send()
            .await?;
        let resp = check_response(resp, "gemini").await?;
        parse_generate_content(model, wants_image, crate::http::read_json(resp).await?)
    }

    /// A finished video's bytes as a `data:` URL part. The file is fetched
    /// with the key; a redirect is followed without it, to `https` only.
    async fn video(&self, uri: &str) -> KalpaResult<Part> {
        use base64::Engine;
        let invalid = || KalpaError::ProviderError {
            status: 502,
            message: "gemini returned an invalid video location".into(),
        };
        let own_host = reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string));
        let mut url = reqwest::Url::parse(uri).map_err(|_| invalid())?;
        for _ in 0..4 {
            let same_host = url.host_str().map(str::to_string) == own_host;
            if !same_host && url.scheme() != "https" {
                return Err(invalid());
            }
            let mut request = self.http.get(url.clone());
            if same_host {
                request = request.header("x-goog-api-key", self.key());
            }
            let resp = request.send().await?;
            if resp.status().is_redirection() {
                let next = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .and_then(|l| url.join(l).ok())
                    .ok_or_else(invalid)?;
                url = next;
                continue;
            }
            let resp = check_response(resp, "gemini").await?;
            let mime = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .filter(|m| m.starts_with("video/"))
                .unwrap_or("video/mp4")
                .to_string();
            let bytes = crate::http::read_bounded(resp, crate::http::MAX_RESPONSE_BYTES).await?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            return Ok(Part::Video {
                url: format!("data:{mime};base64,{b64}"),
                mime: Some(mime),
            });
        }
        Err(invalid())
    }
}

#[async_trait]
impl GenerationProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    async fn submit(&self, request: &GenerationRequest) -> KalpaResult<SubmitOutcome> {
        let model = request.model.split().1;
        if wants_video(model, request) {
            let resp = self
                .http
                .post(format!("{}/interactions", self.base_url))
                .header("x-goog-api-key", self.key())
                .json(&omni_body(model, request, true)?)
                .send()
                .await?;
            let resp = check_response(resp, "gemini").await?;
            let interaction: Value = crate::http::read_json(resp).await?;
            let part = match omni_video(&interaction["steps"]) {
                Some((mime, OmniVideo::Data(data))) => Part::Video {
                    url: format!("data:{mime};base64,{data}"),
                    mime: Some(mime),
                },
                // The video exists and is billed by now. A rate limit on
                // fetching it must not read as a refused submission, which a
                // caller may send again: one more try, then a plain failure.
                Some((_, OmniVideo::Uri(uri))) => match self.video(&uri).await {
                    Err(KalpaError::RateLimited(limit)) => {
                        let wait = limit
                            .retry_after
                            .unwrap_or(std::time::Duration::from_secs(1));
                        tokio::time::sleep(wait.min(std::time::Duration::from_secs(10))).await;
                        self.video(&uri).await.map_err(|e| match e {
                            KalpaError::RateLimited(limit) => KalpaError::ProviderError {
                                status: 502,
                                message: format!(
                                    "gemini generated the video but its download was refused: {}",
                                    limit.message
                                ),
                            },
                            other => other,
                        })?
                    }
                    other => other?,
                },
                None => {
                    let status = interaction["status"].as_str().unwrap_or("no status");
                    return Err(KalpaError::ProviderError {
                        status: 502,
                        message: format!("gemini returned no video ({status})"),
                    });
                }
            };
            return Ok(SubmitOutcome::Sync(GenerationResponse {
                model: model.to_string(),
                parts: vec![part],
                usage: Some(omni_usage(&interaction, request.params.as_ref())),
            }));
        }

        let wants_image = request
            .response_modalities
            .as_ref()
            .is_some_and(|m| m.contains(&Modality::Image));
        let body = generate_content_body(request)?;
        let calls = if wants_image { image_count(request) } else { 1 };
        let response = if calls == 1 {
            self.generate(model, wants_image, &body).await?
        } else {
            // One image per call: the calls run together, and stop with
            // this future (a dropped set aborts its tasks).
            let mut set = tokio::task::JoinSet::new();
            for _ in 0..calls {
                let (this, model, body) = (self.clone(), model.to_string(), body.clone());
                set.spawn(async move { this.generate(&model, wants_image, &body).await });
            }
            let mut results = Vec::with_capacity(calls);
            while let Some(joined) = set.join_next().await {
                results.push(joined.unwrap_or_else(|e| Err(KalpaError::Other(e.to_string()))));
            }
            merge_images(results)?
        };
        Ok(SubmitOutcome::Sync(response))
    }

    async fn poll(&self, _handle: &JobHandle) -> KalpaResult<PollStatus> {
        Err(KalpaError::Other(
            "gemini is synchronous; poll not supported".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(server: &MockServer) -> GeminiProvider {
        GeminiProvider::new("test-key".into()).with_base_url(&format!("{}/v1beta/", server.uri()))
    }

    fn image_request(params: Value) -> GenerationRequest {
        GenerationRequest {
            model: "gemini-3.1-flash-image".into(),
            parts: vec![Part::text("a fox")],
            params: Some(params),
            response_modalities: Some(vec![Modality::Image]),
        }
    }

    fn image_reply() -> Value {
        json!({
            "candidates": [{"content": {"parts": [
                {"text": "planning", "thought": true},
                {"inlineData": {"mimeType": "image/png", "data": "AAAA"}, "thoughtSignature": "sig"},
            ]}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 7, "candidatesTokenCount": 1120, "thoughtsTokenCount": 30, "totalTokenCount": 1157},
        })
    }

    async fn sync(provider: &GeminiProvider, request: &GenerationRequest) -> GenerationResponse {
        match provider.submit(request).await.unwrap() {
            SubmitOutcome::Sync(response) => response,
            SubmitOutcome::Async(_) => panic!("expected a synchronous reply"),
        }
    }

    #[tokio::test]
    async fn an_image_request_sends_the_key_in_a_header_and_maps_size_and_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/v1beta/models/gemini-3.1-flash-image:generateContent",
            ))
            .and(header("x-goog-api-key", "test-key"))
            .and(body_partial_json(json!({
                "contents": [{"role": "user", "parts": [{"text": "a fox"}]}],
                "generationConfig": {
                    "responseModalities": ["IMAGE"],
                    "imageConfig": {"aspectRatio": "16:9", "imageSize": "2K"},
                },
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(image_reply()))
            .expect(1)
            .mount(&server)
            .await;

        let request = image_request(json!({"size": "2048x1152"}));
        let response = sync(&provider(&server), &request).await;
        // The thought is not output; its tokens are billed as output.
        assert_eq!(response.parts.len(), 1);
        assert!(matches!(&response.parts[0], Part::Image { b64_data: Some(d), .. } if d == "AAAA"));
        let usage = response.usage.unwrap();
        assert_eq!(
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens
            ),
            (7, 1150, 1157)
        );
        // Nothing of the key is in a URL.
        let seen = &server.received_requests().await.unwrap()[0];
        assert_eq!(seen.url.query(), None);
    }

    #[test]
    fn sizes_become_the_nearest_ratio_and_only_a_larger_output_is_spelled_out() {
        let config = |params: Value| {
            generate_content_body(&image_request(params)).unwrap()["generationConfig"]
                ["imageConfig"]
                .clone()
        };
        assert_eq!(
            config(json!({"size": "1024x1024"})),
            json!({"aspectRatio": "1:1"})
        );
        assert_eq!(
            config(json!({"size": "1024x1536"})),
            json!({"aspectRatio": "2:3"})
        );
        assert_eq!(
            config(json!({"size": "4096x4096"})),
            json!({"aspectRatio": "1:1", "imageSize": "4K"})
        );
        assert_eq!(
            config(json!({"aspect_ratio": "21:9", "image_size": "512"})),
            json!({"aspectRatio": "21:9", "imageSize": "512"})
        );
        assert_eq!(config(json!({"size": "auto"})), Value::Null);
        // An input image is sent inline, from base64 or a data: URL; any
        // other URL is refused.
        let mut request = image_request(json!({}));
        request
            .parts
            .push(Part::image_url("data:image/jpeg;base64,QUJD"));
        let body = generate_content_body(&request).unwrap();
        assert_eq!(
            body["contents"][0]["parts"][1],
            json!({"inlineData": {"mimeType": "image/jpeg", "data": "QUJD"}})
        );
        request
            .parts
            .push(Part::image_url("https://example.com/a.png"));
        assert!(matches!(
            generate_content_body(&request),
            Err(KalpaError::ProviderError { status: 400, .. })
        ));
    }

    #[test]
    fn a_blocked_prompt_or_withheld_image_is_a_content_policy_refusal_not_an_empty_success() {
        let parse = |body: Value| {
            parse_generate_content("m", true, body)
                .unwrap_err()
                .to_string()
        };
        let blocked = parse(json!({"promptFeedback": {"blockReason": "PROHIBITED_CONTENT"}}));
        assert!(
            blocked.contains("(400)")
                && blocked.contains("content_policy")
                && blocked.contains("PROHIBITED_CONTENT"),
            "{blocked}"
        );
        let withheld = parse(json!({"candidates": [{"finishReason": "IMAGE_SAFETY"}]}));
        assert!(
            withheld.contains("(400)") && withheld.contains("content_policy"),
            "{withheld}"
        );
        let none = parse(json!({"candidates": [
            {"content": {"parts": [{"text": "I cannot draw"}]}, "finishReason": "NO_IMAGE"},
        ]}));
        assert!(
            none.contains("(502)") && none.contains("NO_IMAGE"),
            "{none}"
        );
        // Text is a fine answer when no image was asked for.
        let reply = json!({"candidates": [
            {"content": {"parts": [{"text": "hi"}]}, "finishReason": "STOP"},
        ]});
        let text = parse_generate_content("m", false, reply).unwrap();
        assert!(matches!(&text.parts[0], Part::Text { text } if text == "hi"));
    }

    #[test]
    fn an_images_own_tokens_are_told_apart_from_the_thinking_around_them() {
        // The live API's shape (gemini-3.1-flash-image, 2026-11): thinking is
        // inside `candidatesTokenCount`, and only the image is detailed.
        let reply = json!({
            "candidates": [{"content": {"parts": [
                {"inlineData": {"mimeType": "image/jpeg", "data": "AAAA"}, "thoughtSignature": "s"},
            ]}, "finishReason": "STOP"}],
            "usageMetadata": {
                "promptTokenCount": 10, "candidatesTokenCount": 1517, "totalTokenCount": 1527,
                "candidatesTokensDetails": [{"modality": "IMAGE", "tokenCount": 1120}],
            },
        });
        let usage = parse_generate_content("m", true, reply)
            .unwrap()
            .usage
            .unwrap();
        assert_eq!(
            (usage.completion_tokens, usage.reasoning_tokens),
            (1517, Some(397))
        );
        // A text reply: thinking is its own count, outside the candidates'.
        let reply = json!({
            "candidates": [{"content": {"parts": [{"text": "pong"}]}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 38, "candidatesTokenCount": 5, "thoughtsTokenCount": 61, "totalTokenCount": 104,
                "candidatesTokensDetails": [{"modality": "TEXT", "tokenCount": 5}]},
        });
        let usage = parse_generate_content("m", false, reply)
            .unwrap()
            .usage
            .unwrap();
        assert_eq!(
            (usage.completion_tokens, usage.reasoning_tokens),
            (66, Some(61))
        );
    }

    #[tokio::test]
    async fn several_images_are_one_call_each_with_the_usage_summed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(image_reply()))
            .expect(3)
            .mount(&server)
            .await;
        let response = sync(&provider(&server), &image_request(json!({"n": 3}))).await;
        assert_eq!(response.parts.len(), 3);
        assert_eq!(response.usage.unwrap().completion_tokens, 3450);
    }

    #[tokio::test]
    async fn a_rate_limit_carries_googles_retry_delay_and_other_refusals_are_told_apart() {
        let server = MockServer::start().await;
        let error = |code: u16, status: &str, message: &str, details: Value| {
            ResponseTemplate::new(code).set_body_json(json!({"error": {
                "code": code, "message": message, "status": status, "details": details,
            }}))
        };
        let retry = json!([
            {"@type": "type.googleapis.com/google.rpc.QuotaFailure",
             "violations": [{"quotaId": "GenerateRequestsPerMinutePerProjectPerModel"}]},
            {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "17.5s"},
        ]);
        Mock::given(path("/v1beta/models/limited:generateContent"))
            .respond_with(error(
                429,
                "RESOURCE_EXHAUSTED",
                "You exceeded your current quota",
                retry,
            ))
            .mount(&server)
            .await;
        Mock::given(path("/v1beta/models/header:generateContent"))
            .respond_with(
                error(429, "RESOURCE_EXHAUSTED", "slow down", json!([]))
                    .insert_header("retry-after", "3"),
            )
            .mount(&server)
            .await;
        Mock::given(path("/v1beta/models/broke:generateContent"))
            .respond_with(error(
                402,
                "RESOURCE_EXHAUSTED",
                "Your prepayment credits are depleted.",
                json!([]),
            ))
            .mount(&server)
            .await;
        let bad_key = json!([{
            "@type": "type.googleapis.com/google.rpc.ErrorInfo",
            "reason": "API_KEY_INVALID", "domain": "googleapis.com",
        }]);
        Mock::given(path("/v1beta/models/badkey:generateContent"))
            .respond_with(error(
                400,
                "INVALID_ARGUMENT",
                "API key not valid. Please pass a valid API key.",
                bad_key,
            ))
            .mount(&server)
            .await;

        let provider = provider(&server);
        let call = |model: &'static str| {
            let provider = provider.clone();
            async move {
                provider
                    .submit(&GenerationRequest::prompt(model, "hi"))
                    .await
                    .err()
                    .unwrap()
            }
        };
        let limited = call("limited").await;
        assert_eq!(limited.retry_after(), Some(Duration::from_millis(17_500)));
        assert!(crate::retry::is_rate_limit_rejection(&limited));
        assert!(
            limited.to_string().contains("RESOURCE_EXHAUSTED"),
            "{limited}"
        );
        assert_eq!(
            call("header").await.retry_after(),
            Some(Duration::from_secs(3))
        );
        // Out of credit is not a rate limit: retrying the key cannot help.
        let broke = call("broke").await;
        assert!(
            matches!(broke, KalpaError::ProviderError { status: 402, .. }),
            "{broke}"
        );
        assert!(!crate::retry::is_retryable(&broke));
        // Google answers a bad key with a 400.
        assert!(matches!(call("badkey").await, KalpaError::Auth(_)));
    }

    #[tokio::test]
    async fn an_omni_video_is_one_interaction_its_file_fetched_with_the_key() {
        let server = MockServer::start().await;
        let file = format!("{}/v1beta/files/abc:download?alt=media", server.uri());
        Mock::given(method("POST"))
            .and(path("/v1beta/interactions"))
            .and(header("x-goog-api-key", "test-key"))
            .and(body_partial_json(json!({
                "model": "gemini-omni-1.2-flash",
                "input": [
                    {"type": "text", "text": "a wave"},
                    {"type": "image", "data": "QUJD", "mime_type": "image/png"},
                ],
                "response_format": {
                    "type": "video", "aspect_ratio": "9:16", "resolution": "720p",
                    "duration": "6s", "delivery": "uri",
                },
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "v1_x", "status": "completed",
                "steps": [
                    {"type": "user_input", "content": [{"type": "text", "text": "a wave"}]},
                    {"type": "model_output", "content": [
                        {"type": "video", "mime_type": "video/mp4", "uri": file},
                    ]},
                ],
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1beta/files/abc:download"))
            .and(header("x-goog-api-key", "test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "video/mp4")
                    .set_body_bytes(b"mp4!".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let request = GenerationRequest {
            model: "gemini-omni-1.2-flash".into(),
            parts: vec![
                Part::text("a wave"),
                Part::Image {
                    url: None,
                    b64_data: Some("QUJD".into()),
                    mime: None,
                },
            ],
            params: Some(json!({"seconds": "6", "size": "720x1280"})),
            response_modalities: Some(vec![Modality::Video]),
        };
        let response = sync(&provider(&server), &request).await;
        assert!(matches!(
            &response.parts[0],
            Part::Video { url, .. } if url == "data:video/mp4;base64,bXA0IQ=="
        ));
        // No usage was reported: six seconds of 720p are metered, not free.
        assert_eq!(response.usage.unwrap().completion_tokens, 6 * 5_792);
    }

    #[test]
    fn a_veo_operation_is_read_in_both_dialects() {
        assert!(matches!(
            veo_state(&json!({"name": "op"})),
            VeoState::Running
        ));
        let failed = veo_state(&json!({"done": true, "error": {"message": "quota"}}));
        assert!(matches!(failed, VeoState::Failed(m) if m == "quota"));
        let vertex = veo_state(&json!({"done": true, "response": {"videos": [
            {"bytesBase64Encoded": "QUJD", "mimeType": "video/mp4"},
        ]}}));
        assert!(matches!(vertex, VeoState::Inline { data, .. } if data == "QUJD"));
        let filtered = veo_state(&json!({"done": true, "response": {
            "raiMediaFilteredCount": 1, "raiMediaFilteredReasons": ["violence"],
        }}));
        assert!(matches!(filtered, VeoState::Failed(m) if m.starts_with("content_policy")));
        let request = GenerationRequest {
            model: "veo-3.1-generate-001".into(),
            parts: vec![
                Part::text("a wave"),
                Part::image_url("data:image/png;base64,QUJD"),
            ],
            params: Some(json!({"seconds": 8, "size": "1080x1920"})),
            response_modalities: None,
        };
        assert_eq!(
            veo_body(&request).unwrap(),
            json!({
                "instances": [{
                    "prompt": "a wave",
                    "image": {"bytesBase64Encoded": "QUJD", "mimeType": "image/png"},
                }],
                "parameters": {"aspectRatio": "9:16", "resolution": "1080p", "durationSeconds": 8},
            })
        );
    }
}
