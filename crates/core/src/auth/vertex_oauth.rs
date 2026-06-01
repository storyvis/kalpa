//! Vertex AI OAuth2 authentication using service account JSON.
//!
//! Flow: Service Account JSON → JWT → OAuth Access Token → Bearer Token

use crate::error::{KalpaError, KalpaResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Google service account credentials from JSON file
#[derive(Debug, Deserialize, Serialize)]
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
#[derive(Debug, Clone)]
pub struct VertexAuthToken {
    pub access_token: String,
    pub expires_at: SystemTime,
    pub project_id: String,
}

impl VertexAuthToken {
    /// Load service account from JSON file and get an access token
    pub async fn from_service_account_file<P: AsRef<Path>>(path: P) -> KalpaResult<Self> {
        let json_content = std::fs::read_to_string(path.as_ref()).map_err(|e| {
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
        // Create JWT
        let jwt = create_jwt(&sa)?;

        // Exchange JWT for access token
        let token_response = exchange_jwt_for_token(&sa.token_uri, &jwt).await?;

        let expires_at = SystemTime::now() + Duration::from_secs(token_response.expires_in);

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
    pub async fn refresh_if_needed(
        &mut self,
        service_account_path: &Path,
    ) -> KalpaResult<()> {
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
        .unwrap()
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
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use rsa::pkcs8::DecodePrivateKey;
    use rsa::RsaPrivateKey;
    use rsa::Pkcs1v15Sign;
    use sha2::{Sha256, Digest};

    // Try PKCS#8 first, then PKCS#1
    let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(private_key_pem))
        .map_err(|e| KalpaError::Config(format!("Invalid private key: {}", e)))?;

    // Hash the message
    let mut hasher = Sha256::new();
    hasher.update(message.as_bytes());
    let hashed = hasher.finalize();

    // Sign with PKCS1v15 (required for RS256 JWT)
    let signature = private_key
        .sign(Pkcs1v15Sign::new::<Sha256>(), &hashed)
        .map_err(|e| KalpaError::Config(format!("Failed to sign JWT: {}", e)))?;

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
    let client = reqwest::Client::new();

    let params = [
        ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
        ("assertion", jwt),
    ];

    let response = client
        .post(token_uri)
        .form(&params)
        .send()
        .await
        .map_err(|e| KalpaError::Http(e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(KalpaError::Auth(format!(
            "OAuth token exchange failed ({}): {}",
            status, body
        )));
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .map_err(|e| KalpaError::Auth(format!("Failed to parse token response: {}", e)))?;

    Ok(token_response)
}
