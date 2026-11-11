//! Shared transport policy: bounded responses, deadlines, and no credential redirects.

use crate::error::{KalpaError, KalpaResult};
use serde::de::DeserializeOwned;
use std::time::Duration;

pub use kalpa_libgen::MAX_RESPONSE_BYTES;

pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::none())
}

/// Mark credentials sensitive so reqwest/header debug output redacts them.
pub fn secret_header(value: &str) -> reqwest::header::HeaderValue {
    // Retain the infallible constructors' API; invalid keys produce an empty
    // credential (authentication fails) instead of panicking a server process.
    let mut header = reqwest::header::HeaderValue::from_str(value)
        .unwrap_or_else(|_| reqwest::header::HeaderValue::from_static(""));
    header.set_sensitive(true);
    header
}

pub async fn read_bounded(response: reqwest::Response, cap: usize) -> KalpaResult<Vec<u8>> {
    kalpa_libgen::read_bounded(response, cap)
        .await
        .map_err(|error| match error {
            kalpa_libgen::BodyError::Http(error) => error.into(),
            kalpa_libgen::BodyError::TooLarge(_) => KalpaError::ProviderError {
                status: 502,
                message: error.to_string(),
            },
        })
}

pub async fn read_json<T: DeserializeOwned>(response: reqwest::Response) -> KalpaResult<T> {
    let bytes = read_bounded(response, MAX_RESPONSE_BYTES).await?;
    serde_json::from_slice(&bytes).map_err(|_| KalpaError::ProviderError {
        status: 502,
        message: "Provider returned an invalid response".into(),
    })
}

/// Preserve status. The body itself is never reflected (it can echo keys); only
/// a JSON `error.message` (and `error.code`), scrubbed by [`scrub`], is kept.
pub async fn check_response(
    resp: reqwest::Response,
    provider_name: &str,
) -> KalpaResult<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let reason = error_message(&read_error_body(resp).await)
        .map(|m| format!(": {m}"))
        .unwrap_or_default();
    if status.as_u16() == 429 {
        return Err(KalpaError::RateLimited(format!(
            "{provider_name} HTTP 429{reason}"
        )));
    }
    Err(KalpaError::ProviderError {
        status: status.as_u16(),
        message: format!("{provider_name} request failed{reason}"),
    })
}

/// The most of an error body read for its message; the rest is not read.
const MAX_ERROR_BYTES: usize = 16 * 1024;

/// The longest provider message kept.
const MAX_MESSAGE_CHARS: usize = 300;

async fn read_error_body(mut resp: reqwest::Response) -> Vec<u8> {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = resp.chunk().await {
        let room = MAX_ERROR_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if body.len() >= MAX_ERROR_BYTES {
            break;
        }
    }
    body
}

/// `error.message` (with a string `error.code`) from a JSON error body, the
/// shape OpenAI, Anthropic and Google answer with. Anything else is `None`.
pub(crate) fn error_message(body: &[u8]) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Body {
        error: Inner,
    }
    #[derive(serde::Deserialize)]
    struct Inner {
        message: Option<String>,
        code: Option<serde_json::Value>,
    }
    let Body { error } = serde_json::from_slice(body).ok()?;
    let message = error.message.filter(|m| !m.trim().is_empty())?;
    let message = match error.code.as_ref().and_then(|c| c.as_str()) {
        Some(code) if !code.is_empty() => format!("{message} ({code})"),
        _ => message,
    };
    Some(scrub(&message))
}

/// A provider message made safe to pass on: words that look like a key or a
/// bearer token are replaced, whitespace collapsed, the length capped.
pub(crate) fn scrub(message: &str) -> String {
    let secret = |w: &str| {
        w.contains("sk-")
            || w.contains("AIza")
            || w.eq_ignore_ascii_case("bearer")
            || (w.len() >= 32
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.*".contains(c)))
    };
    let joined = message
        .split_whitespace()
        .map(|w| if secret(w) { "[redacted]" } else { w })
        .collect::<Vec<_>>()
        .join(" ");
    match joined.char_indices().nth(MAX_MESSAGE_CHARS) {
        Some((i, _)) => format!("{}…", &joined[..i]),
        None => joined,
    }
}

pub fn generated_error<E>(error: kalpa_libgen::progenitor_client::Error<E>) -> KalpaError {
    use kalpa_libgen::progenitor_client::Error;
    if let Error::CommunicationError(error) = error {
        return error.into();
    }
    let status = error.status().map(|s| s.as_u16()).unwrap_or(502);
    if status == 429 {
        KalpaError::RateLimited("Provider HTTP 429".into())
    } else {
        KalpaError::ProviderError {
            status,
            message: "Provider request failed".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_body_over_cap_without_content_length() {
        let response = reqwest::Response::from(http::Response::new("12345"));
        assert!(read_bounded(response, 4).await.is_err());
        let response = reqwest::Response::from(http::Response::new("1234"));
        assert_eq!(read_bounded(response, 4).await.unwrap(), b"1234");
    }
    #[tokio::test]
    async fn error_does_not_reflect_provider_secrets() {
        let response = reqwest::Response::from(
            http::Response::builder()
                .status(429)
                .body("secret key")
                .unwrap(),
        );
        let error = check_response(response, "test").await.unwrap_err();
        assert!(matches!(error, KalpaError::RateLimited(_)));
        assert!(!error.to_string().contains("secret"));
    }
    #[tokio::test]
    async fn a_json_error_message_is_kept_without_keys() {
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-proj-abc***xyz. See https://platform.openai.com","code":"invalid_api_key"}}"#;
        let response =
            reqwest::Response::from(http::Response::builder().status(401).body(body).unwrap());
        let error = check_response(response, "openai").await.unwrap_err();
        let KalpaError::ProviderError { status, message } = error else {
            panic!("wrong variant")
        };
        assert_eq!(status, 401);
        assert!(
            message.starts_with("openai request failed: Incorrect API key provided: [redacted]"),
            "{message}"
        );
        assert!(message.ends_with("(invalid_api_key)"), "{message}");
        assert!(!message.contains("sk-"), "{message}");
    }
    #[test]
    fn scrub_redacts_token_like_words_and_caps_length() {
        assert_eq!(
            scrub("bad key AIzaSyA1234 and Bearer x"),
            "bad key [redacted] and [redacted] x"
        );
        assert_eq!(
            scrub("token abcdefghijklmnopqrstuvwxyz0123456789 here"),
            "token [redacted] here"
        );
        assert_eq!(
            scrub(&"word ".repeat(200)).chars().count(),
            MAX_MESSAGE_CHARS + 1
        );
        assert_eq!(error_message(b"<html>502</html>"), None);
    }
    #[tokio::test]
    async fn malformed_success_response_does_not_echo_enum_values() {
        #[derive(serde::Deserialize, Debug)]
        enum Status {
            Pending,
        }
        let response = reqwest::Response::from(http::Response::new("\"synthetic-secret-key\""));
        let error = read_json::<Status>(response).await.unwrap_err();
        assert!(matches!(
            error,
            KalpaError::ProviderError { status: 502, .. }
        ));
        assert!(!error.to_string().contains("synthetic-secret-key"));
    }
    #[test]
    fn credentials_are_redacted_and_invalid_keys_do_not_panic() {
        assert!(!format!("{:?}", secret_header("secret key")).contains("secret key"));
        assert!(secret_header("invalid\nkey").is_empty());
    }
    #[tokio::test]
    async fn credential_client_does_not_follow_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_url = format!("http://{}", target.local_addr().unwrap());
        let source_url = format!("http://{}", redirect.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = redirect.accept().await.unwrap();
            let mut bytes = [0; 4096];
            stream.read(&mut bytes).await.unwrap();
            stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        });
        let response = client_builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(source_url)
            .header("x-api-key", secret_header("synthetic-secret"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 302);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), target.accept())
                .await
                .is_err()
        );
        server.await.unwrap();
    }
}

/// Download external media only from public HTTPS addresses. Resolve once and
/// pin every connection to those vetted addresses; redirects/proxies are off.
pub async fn download_public_audio(value: &str) -> KalpaResult<Vec<u8>> {
    let url =
        reqwest::Url::parse(value).map_err(|_| KalpaError::Config("Invalid audio URL".into()))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err(KalpaError::Config(
            "Audio URL must use public HTTPS on port 443".into(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| KalpaError::Config("Audio URL has no host".into()))?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addresses: Vec<_> = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::lookup_host((host, 443)),
    )
    .await
    .map_err(|_| KalpaError::Config("Audio host lookup timed out".into()))?
    .map_err(|_| KalpaError::Config("Audio host lookup failed".into()))?
    .take(33)
    .collect();
    if addresses.is_empty()
        || addresses.len() > 32
        || addresses.iter().any(|address| !public_ip(address.ip()))
    {
        return Err(KalpaError::Config(
            "Audio URL must resolve only to public addresses".into(),
        ));
    }
    let client = client_builder()
        .no_proxy()
        .resolve_to_addrs(host, &addresses)
        .build()?;
    let response = check_response(client.get(url).send().await?, "audio download").await?;
    read_bounded(response, 25 * 1024 * 1024).await
}

fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192
                    && (b == 168 || (b == 0 && (c == 0 || c == 2)) || (b == 88 && c == 99)))
                || (a == 198 && ((b == 18 || b == 19) || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        std::net::IpAddr::V6(ip) => {
            let s = ip.segments();
            // Accept native global unicast, excluding special-use, documentation
            // and transition ranges (including IPv4-mapped addresses).
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x200 || s[1] == 0xdb8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

#[cfg(test)]
mod media_tests {
    use super::*;
    #[test]
    fn rejects_private_reserved_and_mapped_addresses() {
        for address in [
            "127.0.0.1",
            "10.2.3.4",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "224.0.0.1",
            "0.0.0.0",
            "198.18.0.1",
            "192.0.2.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:7f00:1::1",
        ] {
            assert!(!public_ip(address.parse().unwrap()), "{address}");
        }
        for address in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public_ip(address.parse().unwrap()), "{address}");
        }
    }
    #[tokio::test]
    async fn rejects_non_https_and_local_audio_urls() {
        for url in [
            "http://example.com/audio",
            "file:///etc/passwd",
            "https://127.0.0.1/audio",
            "https://[::1]/audio",
            "https://user:password@example.com/audio",
        ] {
            assert!(download_public_audio(url).await.is_err());
        }
    }
}
