//! Vertex AI OAuth2 authentication using service account JSON.
//!
//! Flow: Service Account JSON → JWT → OAuth Access Token → Bearer Token

use crate::error::{KalpaError, KalpaResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Google service account credentials from JSON file
#[derive(Deserialize, Serialize)]
pub struct ServiceAccount {
    #[serde(rename = "type")]
    pub account_type: String,
    pub project_id: String,
    pub private_key_id: String,
    pub private_key: String,
    pub client_email: String,
    pub client_id: String,
    pub auth_uri: String,
    pub token_uri: String,
}

/// OAuth access token with expiration
#[derive(Clone)]
pub struct VertexAuthToken {
    pub access_token: String,
    pub expires_at: SystemTime,
    pub project_id: String,
}

impl VertexAuthToken {
    /// Load service account from JSON file and get an access token
    pub async fn from_service_account_file<P: AsRef<Path>>(path: P) -> KalpaResult<Self> {
        let json_content = tokio::fs::read_to_string(path.as_ref())
            .await
            .map_err(|e| {
                KalpaError::Config(format!(
                    "Failed to read service account file {}: {}",
                    path.as_ref().display(),
                    e
                ))
            })?;

        let service_account: ServiceAccount = serde_json::from_str(&json_content)
            .map_err(|e| KalpaError::Config(format!("Invalid service account JSON: {}", e)))?;

        Self::from_service_account(service_account).await
    }

    /// Generate OAuth token from service account credentials
    pub async fn from_service_account(sa: ServiceAccount) -> KalpaResult<Self> {
        validate_token_uri(&sa.token_uri)?;
        // Create JWT
        let jwt = create_jwt(&sa)?;

        // Exchange JWT for access token
        let token_response = exchange_jwt_for_token(&sa.token_uri, &jwt).await?;

        let expires_at = SystemTime::now()
            .checked_add(Duration::from_secs(token_response.expires_in))
            .ok_or_else(|| KalpaError::Auth("Invalid OAuth token lifetime".into()))?;

        Ok(Self {
            access_token: token_response.access_token,
            expires_at,
            project_id: sa.project_id,
        })
    }

    /// Check if the token is expired or about to expire (within 60 seconds)
    pub fn is_expired(&self) -> bool {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(now) => match self.expires_at.duration_since(UNIX_EPOCH) {
                Ok(exp) => now.as_secs() + 60 >= exp.as_secs(),
                Err(_) => true,
            },
            Err(_) => true,
        }
    }

    /// Refresh the token if needed
    pub async fn refresh_if_needed(&mut self, service_account_path: &Path) -> KalpaResult<()> {
        if self.is_expired() {
            let new_token = Self::from_service_account_file(service_account_path).await?;
            *self = new_token;
        }
        Ok(())
    }
}

/// Create a JWT for Google OAuth
fn create_jwt(sa: &ServiceAccount) -> KalpaResult<String> {
    use base64::Engine;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| KalpaError::Auth("System clock is before Unix epoch".into()))?
        .as_secs();

    // JWT Header
    let header = serde_json::json!({
        "alg": "RS256",
        "typ": "JWT"
    });

    // JWT Claims
    let claims = serde_json::json!({
        "iss": sa.client_email,
        "scope": "https://www.googleapis.com/auth/cloud-platform",
        "aud": sa.token_uri,
        "exp": now + 3600,
        "iat": now,
    });

    let header_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_string(&header).unwrap());
    let claims_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_string(&claims).unwrap());

    let message = format!("{}.{}", header_b64, claims_b64);

    // Sign with private key
    let signature = sign_rs256(&message, &sa.private_key)?;
    let signature_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature);

    Ok(format!("{}.{}", message, signature_b64))
}

/// Sign message with RS256 using private key
fn sign_rs256(message: &str, private_key_pem: &str) -> KalpaResult<Vec<u8>> {
    use rustls_pki_types::{pem::PemObject, PrivateKeyDer};
    let key = PrivateKeyDer::from_pem_slice(private_key_pem.as_bytes())
        .map_err(|_| KalpaError::Config("Invalid private key PEM".into()))?;
    let pair = match key {
        PrivateKeyDer::Pkcs8(key) => {
            ring::signature::RsaKeyPair::from_pkcs8(key.secret_pkcs8_der())
        }
        PrivateKeyDer::Pkcs1(key) => ring::signature::RsaKeyPair::from_der(key.secret_pkcs1_der()),
        _ => {
            return Err(KalpaError::Config(
                "OAuth requires an RSA private key".into(),
            ))
        }
    }
    .map_err(|_| KalpaError::Config("Invalid RSA private key".into()))?;
    let mut signature = vec![0; pair.public().modulus_len()];
    pair.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        message.as_bytes(),
        &mut signature,
    )
    .map_err(|_| KalpaError::Auth("Failed to sign OAuth assertion".into()))?;
    Ok(signature)
}

/// Token response from Google OAuth
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    #[allow(dead_code)]
    token_type: String,
}

/// Exchange JWT for OAuth access token
async fn exchange_jwt_for_token(token_uri: &str, jwt: &str) -> KalpaResult<TokenResponse> {
    validate_token_uri(token_uri)?;
    let client = crate::http::client_builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    let params = [
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", jwt),
    ];

    let response = client
        .post(token_uri)
        .form(&params)
        .send()
        .await
        .map_err(KalpaError::from)?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(KalpaError::Auth(format!(
            "OAuth token exchange failed ({status})"
        )));
    }

    let bytes = crate::http::read_bounded(response, 64 * 1024).await?;
    let token_response: TokenResponse = serde_json::from_slice(&bytes)
        .map_err(|_| KalpaError::Auth("Invalid OAuth token response".into()))?;

    Ok(token_response)
}

fn validate_token_uri(uri: &str) -> KalpaResult<()> {
    if !matches!(
        uri,
        "https://oauth2.googleapis.com/token" | "https://accounts.google.com/o/oauth2/token"
    ) {
        return Err(KalpaError::Config(
            "Service account token_uri must be a Google OAuth token endpoint".into(),
        ));
    }
    Ok(())
}

impl std::fmt::Debug for ServiceAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceAccount")
            .field("project_id", &self.project_id)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for VertexAuthToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VertexAuthToken")
            .field("project_id", &self.project_id)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_untrusted_token_endpoints() {
        for uri in [
            "http://oauth2.googleapis.com/token",
            "https://evil.example/token",
            "https://oauth2.googleapis.com/token?redirect=evil",
        ] {
            assert!(validate_token_uri(uri).is_err());
        }
        assert!(validate_token_uri("https://oauth2.googleapis.com/token").is_ok());
    }
    #[test]
    fn signs_pkcs8_and_pkcs1_with_verifiable_rs256() {
        use ring::signature::KeyPair;
        use rustls_pki_types::{pem::PemObject, PrivateKeyDer};
        let pkcs8 = include_str!("../../tests/fixtures/test-rsa-pkcs8.pem");
        let pkcs1 = include_str!("../../tests/fixtures/test-rsa-pkcs1.pem");
        let key = PrivateKeyDer::from_pem_slice(pkcs8.as_bytes()).unwrap();
        let pair = ring::signature::RsaKeyPair::from_pkcs8(key.secret_der()).unwrap();
        for pem in [pkcs8, pkcs1] {
            let signature = sign_rs256("test assertion", pem).unwrap();
            ring::signature::UnparsedPublicKey::new(
                &ring::signature::RSA_PKCS1_2048_8192_SHA256,
                pair.public_key().as_ref(),
            )
            .verify(b"test assertion", &signature)
            .unwrap();
        }
    }
}
