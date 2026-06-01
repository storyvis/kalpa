# Vertex AI Integration Guide

## Overview

Kalpa now supports Vertex AI with automatic OAuth2 authentication using service account credentials. The authentication flow is fully automated:

```
Service Account JSON → JWT → OAuth Access Token → Bearer Token → API Requests
```

## Prerequisites

1. **Google Cloud Project** with Vertex AI API enabled
2. **Service Account** with appropriate permissions:
   - Vertex AI User (`roles/aiplatform.user`)
   - Or Custom role with `aiplatform.endpoints.predict` permission

3. **Service Account Key** downloaded as JSON file

## Creating a Service Account (if needed)

```bash
# Set your project
gcloud config set project YOUR_PROJECT_ID

# Create service account
gcloud iam service-accounts create kalpa-vertex \
    --display-name="Kalpa Vertex AI" \
    --description="Service account for Kalpa CLI to access Vertex AI"

# Grant Vertex AI User role
gcloud projects add-iam-policy-binding YOUR_PROJECT_ID \
    --member="serviceAccount:kalpa-vertex@YOUR_PROJECT_ID.iam.gserviceaccount.com" \
    --role="roles/aiplatform.user"

# Create and download key
gcloud iam service-accounts keys create ~/kalpa-vertex-key.json \
    --iam-account=kalpa-vertex@YOUR_PROJECT_ID.iam.gserviceaccount.com
```

## Configuration

### Option 1: Using CLI (recommended)

```bash
# Configure Vertex AI with service account
kalpa configure --provider vertex \
    --service-account ~/kalpa-vertex-key.json \
    --gcs-bucket gs://my-bucket-name \
    --location us-central1

# Or configure step by step
kalpa configure --provider vertex --service-account ~/kalpa-vertex-key.json
kalpa configure --provider vertex --gcs-bucket gs://my-vertex-outputs
kalpa configure --provider vertex --location us-central1
```

### Option 2: Manual Configuration

Edit `~/.config/kalpa/config.toml`:

```toml
[providers.vertex]
service_account_path = "/home/username/kalpa-vertex-key.json"
default_model = "gemini-2.0-flash"
gcs_bucket = "gs://my-vertex-outputs"
location = "us-central1"
```

### GCS Bucket Setup

For video generation and large outputs, you need a GCS bucket:

```bash
# Create a bucket
gsutil mb -p YOUR_PROJECT_ID -l us-central1 gs://my-vertex-outputs

# Grant service account access
gsutil iam ch serviceAccount:kalpa-vertex@YOUR_PROJECT_ID.iam.gserviceaccount.com:objectAdmin \
    gs://my-vertex-outputs
```

## Usage Examples

### Text Generation

```bash
# Using Vertex AI Gemini
kalpa generate --provider vertex \
    --model gemini-2.0-flash \
    --prompt "Explain quantum computing in simple terms"
```

### Image Generation

```bash
# Using Imagen 3
kalpa generate --provider vertex \
    --model imagen-3.0-generate-001 \
    --prompt "A serene mountain landscape at sunset" \
    --output mountain.png
```

## Programmatic Usage

```rust
use kalpa_core::{
    auth::VertexAuthToken,
    providers::VertexProvider,
    provider::CompletionProvider,
    types::{CompletionRequest, Message, Role},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load service account and get OAuth token
    let auth = VertexAuthToken::from_service_account_file(
        "/path/to/service-account.json"
    ).await?;

    // Create provider
    let provider = VertexProvider::new(
        auth.access_token,
        auth.project_id,
        "us-central1".to_string(),
    );

    // Make a request
    let request = CompletionRequest {
        model: "gemini-2.0-flash".to_string(),
        messages: vec![Message {
            role: Role::User,
            content: "Hello, Gemini!".to_string(),
        }],
        max_tokens: Some(100),
        temperature: Some(0.7),
        top_p: None,
        stop_sequences: None,
    };

    let response = provider.complete(&request).await?;
    println!("Response: {}", response.content);

    Ok(())
}
```

## Supported Models

### Text Generation (Gemini)

**Latest Models (Gemini 3.x series):**
- `gemini-3.1-flash` - Nano Banana 2 - Latest fast model
- `gemini-3-pro` - Nano Banana Pro - Most capable Gemini 3
- `gemini-2.5-flash` - Original Nano Banana - Fast & efficient

**Gemini 2.0 series:**
- `gemini-2.0-flash` - Fast Gemini 2.0 model
- `gemini-2.0-flash-exp` - Experimental Gemini 2.0

**Gemini 1.5 series:**
- `gemini-1.5-pro` - Most capable 1.5 model
- `gemini-1.5-flash` - Balanced 1.5 model

**Note:** All Gemini models use the same API structure, so any model available in Vertex AI will work.

### Image Generation (Imagen)
- `imagen-3.0-generate-001` - High quality images
- `imagen-3.0-fast-generate-001` - Faster generation

### Video Generation (Veo)
- `veo-2.0-generate-001` - Text-to-video generation (requires GCS bucket)

**Note:** Video generation requires a configured GCS bucket to store outputs.

## Regions

Available regions:
- `us-central1` (default)
- `us-east4`
- `us-west1`
- `europe-west4`
- `asia-southeast1`

## Authentication Details

### Token Management
- OAuth tokens are automatically generated from service account
- Tokens are valid for 1 hour
- Auto-refresh happens 60 seconds before expiration
- Project ID is extracted from service account JSON

### Security
- Service account key is read only during token generation
- Bearer token is used for API requests
- Tokens are not persisted to disk
- Follow Google Cloud security best practices for key storage

## Troubleshooting

### "Invalid private key" error
- Ensure your service account JSON is valid
- Check that the private key is in PKCS#8 or PKCS#1 format

### "OAuth token exchange failed" error
- Verify service account has correct permissions
- Check that Vertex AI API is enabled in your project
- Ensure service account is not disabled

### "No candidates in response" error
- Content may have been filtered by safety settings
- Try adjusting your prompt
- Check model availability in your region

## Cost Considerations

Vertex AI charges based on:
- **Gemini**: Input/output tokens
- **Imagen**: Number of images generated
- **Storage**: Model outputs stored in Cloud Storage

Check [Vertex AI Pricing](https://cloud.google.com/vertex-ai/pricing) for current rates.

## Comparison: Gemini API vs. Vertex AI

| Feature | Gemini API | Vertex AI |
|---------|------------|-----------|
| Authentication | API Key | Service Account + OAuth |
| Billing | Per-request | GCP Project billing |
| Rate Limits | Higher | Enterprise-grade |
| SLA | None | 99.9% uptime SLA |
| Data Residency | Global | Region-specific |
| Enterprise Features | Limited | Full support |

Choose Vertex AI if you need:
- Enterprise-grade SLA
- Data residency control
- Integration with GCP services
- Higher rate limits
