# kalpa

> A unified CLI and Rust library for AI generative models

**kalpa** provides a beautiful, consistent interface to interact with multiple AI providers (OpenAI, Google Gemini, Vertex AI, Fal.ai) from your terminal or Rust code.

## ✨ Features

- 🎨 **Multi-Modal**: Generate text, images, and videos
- 🔌 **Multiple Providers**: OpenAI, Gemini, Vertex AI, Fal.ai
- 📚 **Library + CLI**: Use as a Rust library or command-line tool
- 🎯 **Type-Safe**: Generated from OpenAPI specs where possible
- ⚡ **Async**: Built on tokio for high performance
- 🎨 **Beautiful UI**: Colored output, progress indicators

## 🚀 Quick Start

### Installation

```bash
# Clone the repository
git clone https://github.com/storyvis/kalpa.git
cd kalpa

# Build the project
cargo build --release

# The binary will be at target/release/kalpa
```

### Basic Usage

```bash
# 1. Configure a provider
kalpa configure

# 2. Verify authentication
kalpa auth -f

# 3. See available models
kalpa models -f

# 4. Generate content
kalpa generate -f image "A cyberpunk city at night"
```

## 📋 Provider Support

| Provider | Text | Images | Videos | Authentication |
|----------|------|--------|--------|----------------|
| **Fal.ai** | ❌ | ✅ | ✅ | API Key |
| **OpenAI** | ✅ | ✅ | ❌ | API Key |
| **Gemini** | ✅ | ❌ | ❌ | API Key |
| **Vertex AI** | ✅ | ✅ | ✅ | Service Account JSON |

## 🔧 Configuration

### Fal.ai

```bash
kalpa configure
# Select "Fal.ai"
# Enter your API key from https://fal.ai/dashboard/keys
```

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
# Provide path to service account JSON: ~/keys/my-project-key.json
# ✓ Detected project ID: my-project-123
# Enter GCS bucket (optional): gs://my-bucket
# Enter region: us-central1
```

**Get Vertex AI credentials:**
1. Go to [Google Cloud Console](https://console.cloud.google.com/)
2. Create a service account with Vertex AI permissions
3. Download the JSON key file
4. Use the path to this JSON file in configuration

## 💻 Command Reference

### `kalpa configure`

Interactive configuration wizard for setting up providers.

```bash
kalpa configure              # Interactive setup
kalpa configure --show       # View current configuration
kalpa configure --set gemini.api_key YOUR_KEY
```

### `kalpa auth`

Verify API keys and authentication.

```bash
kalpa auth -f               # Verify Fal.ai
kalpa auth -o               # Verify OpenAI
kalpa auth -g               # Verify Gemini
kalpa auth -v               # Verify Vertex AI
kalpa auth --all            # Verify all configured providers
```

### `kalpa models`

List available models for each provider.

```bash
kalpa models                # List all models
kalpa models -f             # List Fal.ai models only
kalpa models -o             # List OpenAI models only
kalpa models -g             # List Gemini models only
kalpa models -v             # List Vertex AI models only
```

### `kalpa generate`

Generate content (text, images, videos).

**Basic syntax:**
```bash
kalpa generate -<provider> [--model <model-name>] <type> "<prompt>"
```

**Fal.ai Examples:**

```bash
# Text-to-Image (default model)
kalpa generate -f image "A red apple on a wooden table"

# Text-to-Image (specific model)
kalpa generate -f --model fal-ai/flux/dev image "Cyberpunk cityscape"
kalpa generate -f --model fal-ai/flux-pro image "Photorealistic portrait"
kalpa generate -f --model fal-ai/flux-realism image "Mountain landscape"

# Text-to-Video
kalpa generate -f video "Ocean waves crashing on shore"
kalpa generate -f --model fal-ai/minimax/video-01 video "Flying bird"
kalpa generate -f --model fal-ai/hunyuan-video video "Sunset timelapse"

# Image-to-Video
kalpa generate -f \
    --model fal-ai/luma-dream-machine \
    --image-url "https://example.com/image.jpg" \
    video "Animate this scene"
```

**OpenAI Examples:**

```bash
# Text generation
kalpa generate -o text "Explain quantum computing"
kalpa generate -o --model gpt-4.1 text "Write a haiku about code"
kalpa generate -o --model gpt-4.1-mini text "Quick summary please"

# Image generation
kalpa generate -o image "Abstract geometric art"
kalpa generate -o --model dall-e-3 image "Surreal landscape"
kalpa generate -o --model dall-e-2 image "Vintage poster design"
```

**Gemini Examples:**

```bash
# Text generation
kalpa generate -g text "Hello, how are you?"
kalpa generate -g --model gemini-3.1-flash text "Fast response needed"
kalpa generate -g --model gemini-1.5-pro text "Complex reasoning task"
```

**Vertex AI Examples:**

```bash
# Text (Gemini)
kalpa generate -v text "Explain machine learning"
kalpa generate -v --model gemini-3.1-flash text "Quick question"

# Images (Imagen)
kalpa generate -v image "Beautiful sunset over mountains"
kalpa generate -v --model imagen-3.0-generate-001 image "Modern architecture"

# Videos (Veo)
kalpa generate -v video "A spinning globe"
kalpa generate -v --model veo-001 video "Time-lapse of city life"
```

### `kalpa status`

Check the status of all configured providers.

```bash
kalpa status                # View all provider statuses
```

## 🎨 Available Models

### Fal.ai

**Text-to-Image (8 models):**
- `fal-ai/flux/dev` ⭐ Balanced quality/speed
- `fal-ai/flux/schnell` ⚡ Fastest
- `fal-ai/flux-pro` 💎 Highest quality
- `fal-ai/flux-realism` 📷 Photorealistic
- `fal-ai/recraft-v3`
- `fal-ai/aura-flow`
- `fal-ai/stable-diffusion-v3-medium`
- `fal-ai/fast-sdxl`

**Text-to-Video (7 models):**
- `fal-ai/minimax/video-01` ⭐ Great for human motion, 6-second clips
- `fal-ai/minimax/video-01-live` 📹 Live version with 6-second clips at 1280×720
- `fal-ai/hunyuan-video` 🎬 Open-weight, high visual quality, strong text-video alignment
- `fal-ai/mochi-v1` 🎯 High-fidelity motion, strong prompt adherence
- `fal-ai/kling-video/v1/standard/text-to-video`
- `fal-ai/kling-video/v1.5/standard/text-to-video`
- `fal-ai/wan/v2.2-a14b/text-to-video`

**Image-to-Video (5 models):**
- `fal-ai/luma-dream-machine` ⭐ Recommended
- `fal-ai/kling-video/v1/standard/image-to-video`
- `fal-ai/kling-video/v1.5/standard/image-to-video`
- `fal-ai/minimax/video-01/image-to-video`
- `fal-ai/wan/v2.2-a14b/image-to-video`

### OpenAI

**Text Models:**
- `gpt-4.1` ⭐ Latest GPT-4 Turbo
- `gpt-4.1-mini` ⚡ Fast and affordable
- `gpt-4`
- `gpt-3.5-turbo`

**Image Models:**
- `dall-e-3` ⭐ Latest, highest quality
- `dall-e-2` Classic version

### Gemini

**Text Models:**
- `gemini-3.1-flash` ⭐ Fast and efficient
- `gemini-1.5-flash`
- `gemini-1.5-pro` 💎 Most capable

### Vertex AI

**Text (Gemini):**
- `gemini-3.1-flash` ⭐ Fast
- `gemini-1.5-flash`
- `gemini-1.5-pro` 💎 Most capable

**Images (Imagen):**
- `imagen-3.0-generate-001` ⭐ Latest
- `imagegeneration@006`

**Videos (Veo):**
- `veo-001` ⭐ Google's video generation

## 📚 Using as a Library

Add `kalpa-core` to your `Cargo.toml`:

```toml
[dependencies]
kalpa-core = { git = "https://github.com/shaswot16/kalpa" }
tokio = { version = "1", features = ["full"] }
```

**Example usage:**

```rust
use kalpa_core::{
    providers::FalAIProvider,
    ImageGenerationProvider,
    ImageGenerationRequest,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create provider
    let fal = FalAIProvider::new("your-api-key".to_string());
    
    // Create request
    let request = ImageGenerationRequest {
        prompt: "A cyberpunk cityscape at night".to_string(),
        model: "fal-ai/flux/dev".to_string(),
        size: Some("1024x1024".to_string()),
    };
    
    // Generate image
    let response = fal.generate_image(&request).await?;
    
    // Get image URL
    println!("Image URL: {:?}", response.images[0].url);
    
    Ok(())
}
```

**Video generation:**

```rust
use kalpa_core::{
    providers::FalAIProvider,
    VideoGenerationProvider,
    VideoGenerationRequest,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let fal = FalAIProvider::new("your-api-key".to_string());
    
    let request = VideoGenerationRequest {
        prompt: "Ocean waves crashing".to_string(),
        model: "fal-ai/minimax/video-01".to_string(),
        image_url: None,
        duration: Some(5),
    };
    
    let response = fal.generate_video(&request).await?;
    println!("Video URL: {}", response.videos[0].url);
    
    Ok(())
}
```

## 🏗️ Architecture

```
kalpa/
├── crates/
│   ├── core/           # Library: Traits, types, providers
│   │   ├── src/
│   │   │   ├── provider.rs      # Traits (ImageGenerationProvider, etc.)
│   │   │   ├── providers/       # Implementations (FalAI, OpenAI, etc.)
│   │   │   ├── types.rs         # Request/Response types
│   │   │   ├── config.rs        # Configuration management
│   │   │   ├── auth/            # Authentication modules
│   │   │   └── error.rs         # Error types
│   │   └── Cargo.toml
│   │
│   ├── cli/            # CLI Application
│   │   ├── src/
│   │   │   ├── main.rs          # Entry point
│   │   │   └── commands/        # CLI commands
│   │   └── Cargo.toml
│   │
│   └── libgen/         # Code generator from OpenAPI specs
│       ├── specs/               # OpenAPI JSON files
│       ├── build.rs             # Generates code at build time
│       └── Cargo.toml
│
└── Cargo.toml          # Workspace definition
```

## 🔑 Configuration File

Configuration is stored at `~/.config/kalpa/config.toml`:

```toml
[defaults]
provider = "gemini"
format = "text"

[providers.falai]
api_key = "..."
default_model = "fal-ai/flux/dev"

[providers.openai]
api_key = "sk-..."
default_model = "gpt-4.1-mini"

[providers.gemini]
api_key = "..."
default_model = "gemini-3.1-flash"

[providers.vertex]
service_account_path = "/home/user/keys/project-key.json"
gcs_bucket = "gs://my-bucket"
location = "us-central1"
default_model = "gemini-3.1-flash"
```

## 🙏 Acknowledgments

- Built with [progenitor](https://github.com/oxidecomputer/progenitor) for OpenAPI code generation
- Uses [tokio](https://tokio.rs/) for async runtime
- CLI powered by [clap](https://github.com/clap-rs/clap)


