# Kalpa

A unified CLI and Rust library for AI generative models.

Kalpa provides a consistent interface to interact with multiple AI providers from your terminal or Rust code.

## Features

- **Multi-Modal Generation** — Text, images, and video from a single tool
- **Multiple Providers** — OpenAI, Google Gemini, and Vertex AI
- **Dual Interface** — Use as a Rust library or command-line tool
- **Type-Safe** — Generated from OpenAPI specs
- **Async** — Built on Tokio for high performance

## Installation

```bash
git clone https://github.com/storyvis/kalpa.git
cd kalpa
cargo build --release
```

The binary will be available at `target/release/kalpa`.

## Quick Start

```bash
# Configure a provider
kalpa configure

# Verify authentication
kalpa auth --all

# List available models
kalpa models

# Generate content
kalpa generate -g text "Explain quantum computing in simple terms"
```

## Provider Support

| Provider | Text | Images | Video | Authentication |
|----------|------|--------|-------|----------------|
| OpenAI | ✅ | ✅ | — | API Key |
| Gemini | ✅ | — | — | API Key |
| Vertex AI | ✅ | ✅ | ✅ | Service Account JSON |

## Configuration

### OpenAI

```bash
kalpa configure
# Select "OpenAI"
# Enter your API key from https://platform.openai.com/api-keys
```

### Google Gemini

```bash
kalpa configure
# Select "Google Gemini"
# Enter your API key from https://makersuite.google.com/app/apikey
```

### Vertex AI

```bash
kalpa configure
# Select "Google Vertex AI"
# Provide path to service account JSON
# Enter GCS bucket (optional)
# Enter region (e.g., us-central1)
```

To obtain Vertex AI credentials:

1. Open the [Google Cloud Console](https://console.cloud.google.com/)
2. Create a service account with Vertex AI permissions
3. Download the JSON key file
4. Provide the path to this file during configuration

See [docs/VERTEX_AI_SETUP.md](docs/VERTEX_AI_SETUP.md) for detailed instructions.

## Usage Examples

### Text Generation

```bash
# Gemini
kalpa generate -g text "Explain machine learning"
kalpa generate -g --model gemini-3.1-flash text "Quick summary"
kalpa generate -g --model gemini-1.5-pro text "Complex reasoning task"

# OpenAI
kalpa generate -o text "Explain quantum computing"
kalpa generate -o --model gpt-4.1 text "Write a haiku about code"
kalpa generate -o --model gpt-4.1-mini text "Summarize this concept"

# Vertex AI
kalpa generate -v text "Explain distributed systems"
kalpa generate -v --model gemini-3.1-flash text "Quick question"
```

### Image Generation

```bash
# OpenAI (DALL·E)
kalpa generate -o image "Abstract geometric art"
kalpa generate -o --model dall-e-3 image "Surreal landscape"

# Vertex AI (Imagen)
kalpa generate -v image "Beautiful sunset over mountains"
kalpa generate -v --model imagen-3.0-generate-001 image "Modern architecture"
```

### Video Generation

```bash
# Vertex AI (Veo)
kalpa generate -v video "A spinning globe"
kalpa generate -v --model veo-001 video "Time-lapse of city life"
```

## Command Reference

| Command | Description |
|---------|-------------|
| `kalpa configure` | Interactive provider setup |
| `kalpa configure --show` | View current configuration |
| `kalpa auth --all` | Verify all provider credentials |
| `kalpa auth -o / -g / -v` | Verify a specific provider |
| `kalpa models` | List all available models |
| `kalpa models -o / -g / -v` | List models for a specific provider |
| `kalpa generate` | Generate text, images, or video |
| `kalpa status` | Check provider status |

## Available Models

### Gemini

| Model | Notes |
|-------|-------|
| `gemini-3.1-flash` | Fast and efficient |
| `gemini-1.5-flash` | Previous generation fast model |
| `gemini-1.5-pro` | Most capable |

### OpenAI

| Model | Type | Notes |
|-------|------|-------|
| `gpt-4.1` | Text | Latest GPT-4 Turbo |
| `gpt-4.1-mini` | Text | Fast and affordable |
| `dall-e-3` | Image | Highest quality |
| `dall-e-2` | Image | Classic version |

### Vertex AI

| Model | Type | Notes |
|-------|------|-------|
| `gemini-3.1-flash` | Text | Fast |
| `gemini-1.5-pro` | Text | Most capable |
| `imagen-3.0-generate-001` | Image | Latest Imagen |
| `veo-001` | Video | Google's video generation |

## Library Usage

Add `kalpa-core` to your `Cargo.toml`:

```toml
[dependencies]
kalpa-core = { git = "https://github.com/storyvis/kalpa" }
tokio = { version = "1", features = ["full"] }
```

### Example: Text Generation with Gemini

```rust
use kalpa_core::{
    providers::GeminiProvider,
    TextGenerationProvider,
    TextGenerationRequest,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gemini = GeminiProvider::new("your-api-key".to_string());

    let request = TextGenerationRequest {
        prompt: "Explain quantum computing briefly".to_string(),
        model: "gemini-3.1-flash".to_string(),
    };

    let response = gemini.generate_text(&request).await?;
    println!("{}", response.text);

    Ok(())
}
```

### Example: Image Generation with OpenAI

```rust
use kalpa_core::{
    providers::OpenAIProvider,
    ImageGenerationProvider,
    ImageGenerationRequest,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let openai = OpenAIProvider::new("sk-...".to_string());

    let request = ImageGenerationRequest {
        prompt: "A cyberpunk cityscape at night".to_string(),
        model: "dall-e-3".to_string(),
        size: Some("1024x1024".to_string()),
    };

    let response = openai.generate_image(&request).await?;
    println!("Image URL: {:?}", response.images[0].url);

    Ok(())
}
```

## Architecture

```
kalpa/
├── crates/
│   ├── core/           # Library: traits, types, provider implementations
│   ├── cli/            # CLI application
│   └── libgen/         # Code generator from OpenAPI specs
└── Cargo.toml          # Workspace definition
```

## Configuration File

Stored at `~/.config/kalpa/config.toml`:

```toml
[defaults]
provider = "gemini"

[providers.openai]
api_key = "sk-..."
default_model = "gpt-4.1-mini"

[providers.gemini]
api_key = "..."
default_model = "gemini-3.1-flash"

[providers.vertex]
service_account_path = "/path/to/service-account.json"
gcs_bucket = "gs://my-bucket"
location = "us-central1"
default_model = "gemini-3.1-flash"
```


