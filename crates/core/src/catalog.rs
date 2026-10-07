//! Ask a provider which models it serves, from its own API.
//!
//! `supported_models()` on the completion traits is a fixed list written at
//! build time. A [`ModelCatalog`] asks the provider instead, so a new model, a
//! fine-tune or an account-specific model is found, and a mistyped identifier
//! is caught. Lookups read metadata only: nothing is generated or billed.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::error::{KalpaError, KalpaResult};
use crate::http::check_response;

const OPENAI_BASE: &str = "https://api.openai.com/v1";
const CLAUDE_BASE: &str = "https://api.anthropic.com/v1";
const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";
const TIMEOUT: Duration = Duration::from_secs(15);
/// A stop for a provider whose paging never ends.
const MAX_PAGES: usize = 20;

/// One model a provider lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelListing {
    /// The provider's own identifier: what a request must name.
    pub id: String,
    pub display_name: Option<String>,
}

/// A provider that can list its models.
#[async_trait]
pub trait ModelCatalog: Send + Sync {
    /// The provider's name (`openai`, `claude`, `gemini`, or a compatible server's).
    fn name(&self) -> &str;

    /// Every model the credential can see. Errors carry the provider's status:
    /// `ProviderError { status: 401 | 403, .. }` for a refused key.
    async fn list_models(&self) -> KalpaResult<Vec<ModelListing>>;

    /// Whether `id` exists. `Ok(false)` only when the provider says it does
    /// not; a refused key, a timeout or a server error is an `Err`, so the
    /// caller can tell "no such model" from "could not find out".
    async fn model_exists(&self, id: &str) -> KalpaResult<bool> {
        Ok(self.list_models().await?.iter().any(|m| m.id == id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire {
    OpenAi,
    Claude,
    Gemini,
}

/// A catalog over a provider's HTTP API.
pub struct HttpModelCatalog {
    name: String,
    wire: Wire,
    base: String,
    key: Option<String>,
    http: reqwest::Client,
    /// The provider answers `GET /models/{id}` with 404 for an unknown model,
    /// so one lookup is enough. Without it the list is read.
    single_lookup: bool,
}

impl HttpModelCatalog {
    fn new(name: &str, wire: Wire, base: &str, key: Option<String>, single_lookup: bool) -> Self {
        Self {
            name: name.to_string(),
            wire,
            base: base.trim_end_matches('/').to_string(),
            key: key.filter(|k| !k.trim().is_empty()),
            http: reqwest::Client::builder().timeout(TIMEOUT).build().unwrap_or_default(),
            single_lookup,
        }
    }

    /// OpenAI.
    pub fn openai(api_key: String) -> Self {
        Self::new("openai", Wire::OpenAi, OPENAI_BASE, Some(api_key), true)
    }

    /// Anthropic Claude.
    pub fn claude(api_key: String) -> Self {
        Self::new("claude", Wire::Claude, CLAUDE_BASE, Some(api_key), true)
    }

    /// The Gemini API (API-key access).
    pub fn gemini(api_key: String) -> Self {
        Self::new("gemini", Wire::Gemini, GEMINI_BASE, Some(api_key), true)
    }

    /// Any server speaking OpenAI's `/models` (OpenRouter, vLLM, Ollama, ...).
    /// `base` includes the version path, e.g. `http://host:8000/v1`. Ids may
    /// contain slashes, so one model is found by reading the list.
    pub fn openai_compatible(name: &str, base: &str, api_key: Option<String>) -> Self {
        Self::new(name, Wire::OpenAi, base, api_key, false)
    }

    /// Point at another endpoint (a proxy, a regional host, a test server).
    pub fn with_base_url(mut self, base: &str) -> Self {
        self.base = base.trim_end_matches('/').to_string();
        self
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        let req = self.http.get(format!("{}{path}", self.base));
        match (self.wire, &self.key) {
            (_, None) => req,
            (Wire::OpenAi, Some(k)) => req.bearer_auth(k),
            (Wire::Claude, Some(k)) => req.header("x-api-key", k).header("anthropic-version", "2023-06-01"),
            // A header, not `?key=`: a URL ends up in logs and error text.
            (Wire::Gemini, Some(k)) => req.header("x-goog-api-key", k),
        }
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> KalpaResult<reqwest::Response> {
        let resp = req.send().await.map_err(|e| KalpaError::Http(e.without_url()))?;
        check_response(resp, &self.name).await
    }
}

/// A path segment with the characters that would end it escaped.
fn segment(id: &str) -> String {
    id.replace('%', "%25").replace('/', "%2F").replace('?', "%3F").replace('#', "%23")
}

#[derive(Deserialize)]
struct OpenAiList {
    data: Vec<OpenAiItem>,
}
#[derive(Deserialize)]
struct OpenAiItem {
    id: String,
}
#[derive(Deserialize)]
struct ClaudeList {
    data: Vec<ClaudeItem>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    last_id: Option<String>,
}
#[derive(Deserialize)]
struct ClaudeItem {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
}
#[derive(Deserialize)]
struct GeminiList {
    #[serde(default)]
    models: Vec<GeminiItem>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}
#[derive(Deserialize)]
struct GeminiItem {
    name: String,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
}

#[async_trait]
impl ModelCatalog for HttpModelCatalog {
    fn name(&self) -> &str {
        &self.name
    }

    async fn list_models(&self) -> KalpaResult<Vec<ModelListing>> {
        let mut out = Vec::new();
        match self.wire {
            Wire::OpenAi => {
                let list: OpenAiList = self.send(self.get("/models")).await?.json().await?;
                out.extend(list.data.into_iter().map(|m| ModelListing { id: m.id, display_name: None }));
            }
            Wire::Claude => {
                let mut after: Option<String> = None;
                for _ in 0..MAX_PAGES {
                    let mut req = self.get("/models").query(&[("limit", "1000")]);
                    if let Some(a) = &after {
                        req = req.query(&[("after_id", a.as_str())]);
                    }
                    let page: ClaudeList = self.send(req).await?.json().await?;
                    out.extend(page.data.into_iter().map(|m| ModelListing { id: m.id, display_name: m.display_name }));
                    match (page.has_more, page.last_id) {
                        (true, Some(last)) => after = Some(last),
                        _ => break,
                    }
                }
            }
            Wire::Gemini => {
                let mut token: Option<String> = None;
                for _ in 0..MAX_PAGES {
                    let mut req = self.get("/models").query(&[("pageSize", "1000")]);
                    if let Some(t) = &token {
                        req = req.query(&[("pageToken", t.as_str())]);
                    }
                    let page: GeminiList = self.send(req).await?.json().await?;
                    out.extend(page.models.into_iter().map(|m| ModelListing {
                        id: m.name.strip_prefix("models/").unwrap_or(&m.name).to_string(),
                        display_name: m.display_name,
                    }));
                    match page.next_page_token.filter(|t| !t.is_empty()) {
                        Some(t) => token = Some(t),
                        None => break,
                    }
                }
            }
        }
        Ok(out)
    }

    async fn model_exists(&self, id: &str) -> KalpaResult<bool> {
        if !self.single_lookup {
            return Ok(self.list_models().await?.iter().any(|m| m.id == id));
        }
        let id = if self.wire == Wire::Gemini { id.strip_prefix("models/").unwrap_or(id) } else { id };
        let resp = self
            .get(&format!("/models/{}", segment(id)))
            .send()
            .await
            .map_err(|e| KalpaError::Http(e.without_url()))?;
        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        check_response(resp, &self.name).await?;
        Ok(true)
    }
}
