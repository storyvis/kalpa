//! Generate command — produce text, images, or video from AI provi
//! 
use clap::{Args, Subcommand};
use colored::Colorize;
use kalpa_core::{KalpaConfig, Provider};
use kalpa_core::auth::VertexAuthToken;
use kalpa_core::provider::ImageGenerationProvider;
use kalpa_core::providers::VertexProvider;
use kalpa_core::types::ImageGenerationRequest;
use tracing::{debug, info};

/// Arguments for the `generate` subcommand.
#[derive(Debug, Args)]
#[command(
    about = "Generate content (text, image, video) from AI providers",
    long_about = "Generate various types of content using AI providers.\n\n\
                  Provider flags:\n  \
                  -g  Google Gemini\n  \
                  -v  Google Vertex AI\n  \
                  -f  Fal.ai\n  \
                  -o  OpenAI\n  \
                  -c  Anthropic Claude\n\n\
                  Examples:\n  \
                  kalpa generate -g text \"Explain quantum computing\"\n  \
                  kalpa generate -c text \"Write a poem\"\n  \
                  kalpa generate -f image \"A cat on mars\"\n  \
                  kalpa generate -g text --model gemini-2.5-flash \"Hello\""
)]
pub struct GenerateArgs {
    /// Use Google Gemini.
    #[arg(short = 'g', long)]
    pub gemini: bool,

    /// Use Google Vertex AI.
    #[arg(short = 'v', long)]
    pub vertex: bool,

    /// Use Fal.ai.
    #[arg(short = 'f', long)]
    pub fal: bool,

    /// Use OpenAI.
    #[arg(short = 'o', long)]
    pub openai: bool,

    /// Use Anthropic Claude.
    #[arg(short = 'c', long)]
    pub claude: bool,

    /// Override the model to use.
    #[arg(short = 'm', long)]
    pub model: Option<String>,

    /// The generation type and prompt.
    #[command(subcommand)]
    pub content_type: ContentType,
}

/// The type of content to generate.
#[derive(Debug, Subcommand)]
pub enum ContentType {
    /// Generate text content.
    Text {
        /// The prompt text.
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },

    /// Generate an image.
    Image {
        /// The image description prompt.
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },

    /// Generate a video.
    Video {
        /// The video description prompt.
        #[arg(required = true)]
        prompt: String,

        /// Path to input image for image-to-video generation (optional).
        /// If provided, the model must support image-to-video.
        #[arg(short = 'i', long = "image")]
        image_path: Option<String>,

        /// Start video generation without waiting for completion (async mode).
        /// Returns immediately with operation details. Use 'kalpa jobs' to check status.
        #[arg(long = "async")]
        async_mode: bool,
    },
}

/// Execute the generate command.
pub async fn execute(args: GenerateArgs, json: bool) -> anyhow::Result<()> {
    let config = KalpaConfig::load()?;

    // Determine provider
    let provider = resolve_provider(&args, &config)?;
    debug!(provider = %provider.display_name(), "Resolved provider");

    // Get API key (not needed for Vertex AI)
    let api_key = if provider == Provider::Vertex {
        String::new() // Vertex uses service account, not API key
    } else {
        config
            .get_api_key(provider)
            .ok_or_else(|| anyhow::anyhow!(
                "No API key configured for {}. Run: kalpa configure --set {}.api_key YOUR_KEY",
                provider.display_name(),
                provider.as_str()
            ))?
            .to_string()
    };

    // Get model - prefer content-type-specific defaults over provider defaults
    let model = if let Some(ref explicit_model) = args.model {
        explicit_model.as_str()
    } else {
        // For image/video generation, always use content-type-specific defaults
        // to avoid using text models for image generation
        match &args.content_type {
            ContentType::Image { .. } | ContentType::Video { .. } => {
                default_model(provider, &args.content_type)
            }
            ContentType::Text { .. } => {
                config.get_default_model(provider)
                    .unwrap_or_else(|| default_model(provider, &args.content_type))
            }
        }
    };

    info!(provider = %provider.display_name(), model = %model, "Starting content generation");

    match &args.content_type {
        ContentType::Text { prompt } => {
            let prompt_text = prompt.join(" ");
            generate_text(provider, &api_key, model, &prompt_text, json).await
        }
        ContentType::Image { prompt } => {
            let prompt_text = prompt.join(" ");
            generate_image(provider, &config, &api_key, model, &prompt_text, json).await
        }
        ContentType::Video { prompt, image_path, async_mode } => {
            generate_video(provider, &config, &api_key, model, prompt, image_path.as_deref(), *async_mode, json).await
        }
    }
}

/// Resolve which provider to use from flags or config defaults.
fn resolve_provider(args: &GenerateArgs, config: &KalpaConfig) -> anyhow::Result<Provider> {
    let count = [args.gemini, args.vertex, args.fal, args.openai, args.claude]
        .iter()
        .filter(|&&x| x)
        .count();

    if count > 1 {
        anyhow::bail!("Please specify only one provider flag (-g, -v, -f, -o, -c)");
    }

    if args.gemini { return Ok(Provider::Gemini); }
    if args.vertex { return Ok(Provider::Vertex); }
    if args.fal { return Ok(Provider::Fal); }
    if args.openai { return Ok(Provider::OpenAI); }
    if args.claude { return Ok(Provider::Claude); }

    // Use default from config
    Provider::from_str(&config.defaults.provider).map_err(|e| anyhow::anyhow!("{}", e))
}

/// Get the default model for a provider/content-type combination.
fn default_model(provider: Provider, content_type: &ContentType) -> &'static str {
    match (provider, content_type) {
        (Provider::Gemini, ContentType::Text { .. }) => "gemini-2.5-flash",
        (Provider::Gemini, ContentType::Image { .. }) => "gemini-2.5-flash",
        (Provider::Gemini, ContentType::Video { .. }) => "gemini-2.5-flash",
        (Provider::OpenAI, ContentType::Text { .. }) => "gpt-4.1",
        (Provider::OpenAI, ContentType::Image { .. }) => "dall-e-3",
        (Provider::OpenAI, ContentType::Video { .. }) => "gpt-4.1",
        (Provider::Vertex, ContentType::Text { .. }) => "gemini-2.5-flash",
        (Provider::Vertex, ContentType::Image { .. }) => "imagen-4.0-generate-001",
        (Provider::Vertex, ContentType::Video { .. }) => "veo-2.0-generate-001",
        (Provider::Fal, ContentType::Image { .. }) => "fal-ai/fast-sdxl",
        (Provider::Fal, ContentType::Video { .. }) => "fal-ai/kling-video/v1/standard/text-to-video",
        (Provider::Fal, ContentType::Text { .. }) => "fal-ai/fast-sdxl",
        (Provider::Claude, ContentType::Text { .. }) => "claude-sonnet-4-6",
        (Provider::Claude, _) => "claude-sonnet-4-6",
    }
}

/// Get supported models for a provider and content type
fn get_supported_models(provider: Provider, content_type: &ContentType) -> Vec<&'static str> {
    match (provider, content_type) {
        (Provider::Gemini, ContentType::Text { .. }) => {
            vec!["gemini-2.5-flash", "gemini-2.0-flash", "gemini-2.0-flash-exp", "gemini-1.5-pro", "gemini-1.5-flash"]
        }
        (Provider::Vertex, ContentType::Text { .. }) => {
            vec![
                "gemini-2.5-flash",
                "gemini-2.0-flash", "gemini-2.0-flash-exp",
                "gemini-1.5-pro", "gemini-1.5-flash"
            ]
        }
        (Provider::Vertex, ContentType::Image { .. }) => {
            vec!["imagen-4.0-generate-001", "imagen-3.0-generate-001", "imagen-3.0-generate-002", "imagen-3.0-fast-generate-001"]
        }
        (Provider::Vertex, ContentType::Video { .. }) => {
            vec!["veo-3.0-generate", "veo-3.0-fast-generate-preview", "veo-2.0-generate-001"]
        }
        (Provider::OpenAI, ContentType::Text { .. }) => {
            vec![
                "gpt-4.1", "gpt-4.1-mini", "gpt-4.1-preview",
                "gpt-4", "gpt-4-turbo", "gpt-4-turbo-preview",
                "gpt-3.5-turbo", "gpt-3.5-turbo-16k"
            ]
        }
        (Provider::OpenAI, ContentType::Image { .. }) => {
            vec!["dall-e-3", "dall-e-2", "gpt-image-1.5"]
        }
        (Provider::Fal, ContentType::Image { .. }) => {
            vec![
                "fal-ai/flux/dev", "fal-ai/flux/schnell", "fal-ai/flux-pro", "fal-ai/flux-realism",
                "fal-ai/recraft-v3", "fal-ai/aura-flow", "fal-ai/stable-diffusion-v3-medium", "fal-ai/fast-sdxl"
            ]
        }
        (Provider::Fal, ContentType::Video { .. }) => {
            vec![
                // Text-to-Video models
                "fal-ai/minimax/video-01",
                "fal-ai/minimax/video-01-live",
                "fal-ai/hunyuan-video",
                "fal-ai/mochi-v1",
                "fal-ai/kling-video/v1/standard/text-to-video",
                "fal-ai/kling-video/v1.5/standard/text-to-video",
                "fal-ai/kling-video/v1.6/standard/text-to-video",
                "fal-ai/kling-video/v2.1/master/text-to-video",
                "fal-ai/kling-video/v2.6/pro/text-to-video",
                "fal-ai/wan/v2.2-a14b/text-to-video",
                "fal-ai/ltx-2/text-to-video",
                "fal-ai/ltx-2.3/text-to-video",
                "fal-ai/veo3",
                "fal-ai/veo3.1",
                "bytedance/seedance-2.0/text-to-video",
                "bytedance/seedance-2.0/fast/text-to-video",
                // Image-to-Video models
                "fal-ai/veo2/image-to-video",
                "fal-ai/veo3/image-to-video",
                "fal-ai/luma-dream-machine/image-to-video",
                "fal-ai/kling-video/v2.1/master/image-to-video",
                "fal-ai/kling-video/v1.6/pro/image-to-video",
                "fal-ai/minimax/video-01-live/image-to-video",
                "fal-ai/pixverse/v4.5/image-to-video",
                "bytedance/seedance-2.0/image-to-video",
                // Legacy models
                "fal-ai/kling-video/v1/standard/image-to-video",
                "fal-ai/kling-video/v1.5/standard/image-to-video",
                "fal-ai/minimax/video-01/image-to-video",
                "fal-ai/wan/v2.2-a14b/image-to-video",
                "fal-ai/luma-dream-machine"
            ]
        }
        (Provider::Claude, ContentType::Text { .. }) => {
            vec![
                "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6", "claude-haiku-4-5-20251001",
                "claude-3-opus", "claude-3-sonnet", "claude-3-haiku"
            ]
        }
        _ => vec![],
    }
}

/// Get providers that support a given content type
fn get_providers_for_content_type(content_type: &ContentType) -> Vec<(Provider, &'static str)> {
    match content_type {
        ContentType::Text { .. } => vec![
            (Provider::Gemini, "-g"),
            (Provider::Vertex, "-v"),
            (Provider::OpenAI, "-o"),
            (Provider::Claude, "-c"),
        ],
        ContentType::Image { .. } => vec![
            (Provider::Vertex, "-v"),
            (Provider::OpenAI, "-o"),
            (Provider::Fal, "-f"),
        ],
        ContentType::Video { .. } => vec![
            (Provider::Vertex, "-v"),
            (Provider::Fal, "-f"),
        ],
    }
}

/// Validate provider and model before making API request
fn validate_provider_and_model(
    provider: Provider,
    model: &str,
    content_type: &ContentType,
) -> anyhow::Result<()> {
    let supported_models = get_supported_models(provider, content_type);
    
    // Check if provider supports this content type
    if supported_models.is_empty() {
        let content_name = match content_type {
            ContentType::Text { .. } => "text",
            ContentType::Image { .. } => "image",
            ContentType::Video { .. } => "video",
        };
        
        let available_providers = get_providers_for_content_type(content_type);
        let provider_list = available_providers
            .iter()
            .map(|(p, flag)| format!("{} {}", flag, p.display_name()))
            .collect::<Vec<_>>()
            .join(", ");
        
        anyhow::bail!(
            "{} generation is not supported for {}.\n\nAvailable providers for {} generation:\n  {}",
            content_name.chars().next().unwrap().to_uppercase().to_string() + &content_name[1..],
            provider.display_name(),
            content_name,
            provider_list
        );
    }
    
    // Check if model is supported
    if !supported_models.contains(&model) {
        let content_name = match content_type {
            ContentType::Text { .. } => "text",
            ContentType::Image { .. } => "image",
            ContentType::Video { .. } => "video",
        };
        
        anyhow::bail!(
            "Model '{}' is not supported for {} generation with {}.\n\nAvailable models:\n  {}",
            model,
            content_name,
            provider.display_name(),
            supported_models.join("\n  ")
        );
    }
    
    Ok(())
}

/// Generate video via Fal.ai.
async fn generate_video_fal(
    api_key: &str,
    model: &str,
    prompt: &str,
    image_path: Option<&str>,
    is_async: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    use kalpa_core::providers::FalAIProvider;
    use kalpa_core::types::FalQueueStatus;
    use kalpa_core::jobs::{Job, JobStore, JobType, JobStatus};

    // Create Fal.ai provider
    let provider = FalAIProvider::new(api_key.to_string());

    // Handle image upload if provided
    let image_url = if let Some(path) = image_path {
        if !json_output {
            println!("  {} Uploading image to Fal.ai storage...", "→".blue());
        }
        
        let url = provider.upload_file(path).await
            .map_err(|e| anyhow::anyhow!("Failed to upload image: {}", e))?;
        
        if !json_output {
            println!("  {} Image uploaded successfully", "✓".green());
        }
        Some(url)
    } else {
        None
    };

    // Build the request body
    let request_body = provider.build_video_request_body(
        model,
        prompt,
        image_url.as_deref(),
        None, // duration
    ).map_err(|e| anyhow::anyhow!("Failed to build request: {}", e))?;

    // Create job
    let mut job = Job::new(
        JobType::Video,
        "fal".to_string(),
        model.to_string(),
        prompt.to_string(),
    );

    // Save job store
    let store = JobStore::new()?;

    if !json_output {
        println!(
            "  {} Submitting video generation to {} ({})...",
            "→".blue(),
            "Fal.ai".cyan(),
            model
        );
    }

    // Submit to queue
    let submit_response = provider.queue_submit(model, &request_body).await
        .map_err(|e| anyhow::anyhow!("Failed to submit to Fal.ai queue: {}", e))?;

    let request_id = submit_response.request_id.clone();
    let status_url = submit_response.status_url.clone();
    let response_url = submit_response.response_url.clone();

    // Update job with operation ID (request_id)
    job.operation_id = Some(request_id.clone());
    job.update_status(JobStatus::Running);
    store.save(&job)?;

    if !json_output {
        println!("  {} Job created: {}", "✓".green(), job.id.cyan());
        println!("  {} Request ID: {}", "ℹ".dimmed(), request_id.dimmed());
    }

    // ASYNC MODE: Return immediately after submitting
    if is_async {
        if json_output {
            let output = serde_json::json!({
                "provider": "fal",
                "model": model,
                "type": "video",
                "prompt": prompt,
                "job_id": job.id,
                "request_id": request_id,
                "status": "running",
                "message": "Video generation submitted to queue. Use 'kalpa jobs' to check status."
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!();
            println!("{} Video generation submitted!", "✓".green().bold());
            println!("  {} Check status with: {}", "→".dimmed(), format!("kalpa jobs {}", job.id).cyan());
            println!();
        }
        return Ok(());
    }

    // SYNC MODE: Poll for completion with progress display
    if !json_output {
        println!("  {} Waiting for video generation to complete...", "◷".yellow());
    }

    let poll_interval = std::time::Duration::from_secs(3);
    let start_time = std::time::Instant::now();

    // Track state transitions and periodic updates
    #[derive(PartialEq, Clone)]
    enum PollState { Initial, Queued, Processing }
    let mut current_state = PollState::Initial;
    let mut last_log_count: usize = 0;
    let mut last_progress_print = std::time::Instant::now();

    loop {
        let status = provider.queue_status_by_url(&status_url).await
            .map_err(|e| anyhow::anyhow!("Failed to check queue status: {}", e))?;

        match status {
            FalQueueStatus::InQueue { queue_position, logs } => {
                if !json_output {
                    let elapsed = start_time.elapsed().as_secs();
                    let pos_str = queue_position
                        .map(|p| format!(" — {} request(s) ahead", p))
                        .unwrap_or_default();

                    // Print on state change, or every 15 seconds as a heartbeat
                    if current_state != PollState::Queued || last_progress_print.elapsed().as_secs() >= 15 {
                        println!("  {} Queued{} ({}s elapsed)", "○".blue(), pos_str, elapsed);
                        current_state = PollState::Queued;
                        last_progress_print = std::time::Instant::now();
                    }
                    // Print new log entries
                    for log in logs.iter().skip(last_log_count) {
                        println!("    {} {}", "│".dimmed(), log.message.dimmed());
                    }
                    last_log_count = logs.len();
                }
            }
            FalQueueStatus::InProgress { logs } => {
                if !json_output {
                    let elapsed = start_time.elapsed().as_secs();

                    // Print on state change, or every 10 seconds as a heartbeat
                    if current_state != PollState::Processing || last_progress_print.elapsed().as_secs() >= 10 {
                        println!("  {} Processing video... ({}s elapsed)", "◷".yellow(), elapsed);
                        current_state = PollState::Processing;
                        last_progress_print = std::time::Instant::now();
                    }
                    // Print new log entries
                    for log in logs.iter().skip(last_log_count) {
                        println!("    {} {}", "│".dimmed(), log.message.dimmed());
                    }
                    last_log_count = logs.len();
                }
            }
            FalQueueStatus::Completed { .. } => {
                // Fetch the result
                let result_json = provider.queue_result_by_url(&response_url).await
                    .map_err(|e| anyhow::anyhow!("Failed to fetch result: {}", e))?;

                // Try to extract video URL from response
                let video_url = result_json.get("video")
                    .and_then(|v| v.get("url"))
                    .and_then(|u| u.as_str())
                    .or_else(|| {
                        result_json.get("videos")
                            .and_then(|v| v.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|v| v.get("url"))
                            .and_then(|u| u.as_str())
                    });

                if let Some(url) = video_url {
                    // Update job as completed
                    job.complete(url.to_string());
                    store.save(&job)?;

                    if json_output {
                        let output = serde_json::json!({
                            "provider": "fal",
                            "model": model,
                            "type": "video",
                            "prompt": prompt,
                            "url": url,
                            "job_id": job.id,
                            "request_id": request_id,
                            "elapsed_seconds": start_time.elapsed().as_secs()
                        });
                        println!("{}", serde_json::to_string_pretty(&output)?);
                    } else {
                        println!();
                        println!("{} Video generated! ({}s)", "✓".green().bold(), start_time.elapsed().as_secs());
                        println!("  {} {}", "URL:".dimmed(), url.cyan());
                        println!("  {} {}", "Job:".dimmed(), job.id);
                        println!();
                    }
                } else {
                    // No video URL found - save raw response for debugging
                    let raw_str = serde_json::to_string_pretty(&result_json).unwrap_or_default();
                    job.fail(format!("No video URL in response: {}", &raw_str[..raw_str.len().min(200)]));
                    store.save(&job)?;
                    anyhow::bail!("No video URL found in Fal.ai response. Raw response:\n{}", raw_str);
                }
                return Ok(());
            }
            FalQueueStatus::Failed { error, .. } => {
                job.fail(error.clone());
                store.save(&job)?;
                anyhow::bail!("Fal.ai video generation failed: {}", error);
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Generate text using the specified provider.
async fn generate_text(
    provider: Provider,
    api_key: &str,
    model: &str,
    prompt: &str,
    json_output: bool,
) -> anyhow::Result<()> {
    // Validate before making API call
    validate_provider_and_model(provider, model, &ContentType::Text { prompt: vec![] })?;
    
    match provider {
        Provider::Gemini => generate_text_gemini(api_key, model, prompt, json_output).await,
        Provider::OpenAI => generate_text_openai(api_key, model, prompt, json_output).await,
        Provider::Claude => generate_text_claude(api_key, model, prompt, json_output).await,
        Provider::Vertex => {
            let config = KalpaConfig::load()?;
            generate_text_vertex(&config, model, prompt, json_output).await
        }
        _ => {
            // This should not be reached due to validation above
            anyhow::bail!(
                "Text generation not yet implemented for {}. Use -g (Gemini), -v (Vertex), -o (OpenAI), or -c (Claude).",
                provider.display_name()
            )
        }
    }
}

/// Generate text via Gemini API.
async fn generate_text_gemini(
    api_key: &str,
    model: &str,
    prompt: &str,
    json_output: bool,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, api_key
    );

    let body = serde_json::json!({
        "contents": [{
            "parts": [{"text": prompt}]
        }]
    });

    let resp = client.post(&url).json(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Gemini API error (HTTP {}): {}", status, err_body);
    }

    let response: serde_json::Value = resp.json().await?;

    let text = response["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("(no response)");

    if json_output {
        let output = serde_json::json!({
            "provider": "gemini",
            "model": model,
            "type": "text",
            "prompt": prompt,
            "response": text,
            "raw": response
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!();
        println!(
            "{}",
            format!("─── {} ({}) ", "Gemini", model).dimmed()
        );
        println!();
        println!("{}", text);
        println!();
        println!("{}", "───────────────────────────────".dimmed());
    }

    Ok(())
}

/// Generate text via OpenAI API.
async fn generate_text_openai(
    api_key: &str,
    model: &str,
    prompt: &str,
    json_output: bool,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ]
    });

    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        anyhow::bail!("OpenAI API error (HTTP {}): {}", status, err_body);
    }

    let response: serde_json::Value = resp.json().await?;

    let text = response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("(no response)");

    if json_output {
        let output = serde_json::json!({
            "provider": "openai",
            "model": model,
            "type": "text",
            "prompt": prompt,
            "response": text,
            "raw": response
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!();
        println!(
            "{}",
            format!("─── {} ({}) ", "OpenAI", model).dimmed()
        );
        println!();
        println!("{}", text);
        println!();
        println!("{}", "───────────────────────────────".dimmed());
    }

    Ok(())
}

/// Generate text via Claude API.
async fn generate_text_claude(
    api_key: &str,
    model: &str,
    prompt: &str,
    json_output: bool,
) -> anyhow::Result<()> {
    let client = reqwest::Client::new();

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "user", "content": prompt}
        ],
        "max_tokens": 4096
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Claude API error (HTTP {}): {}", status, err_body);
    }

    let response: serde_json::Value = resp.json().await?;

    let text = response["content"][0]["text"]
        .as_str()
        .unwrap_or("(no response)");

    if json_output {
        let output = serde_json::json!({
            "provider": "claude",
            "model": model,
            "type": "text",
            "prompt": prompt,
            "response": text,
            "raw": response
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!();
        println!(
            "{}",
            format!("─── {} ({}) ", "Claude", model).dimmed()
        );
        println!();
        println!("{}", text);
        println!();
        println!("{}", "───────────────────────────────".dimmed());
    }

    Ok(())
}

/// Generate image (placeholder for non-Gemini providers).
async fn generate_image(
    provider: Provider,
    config: &KalpaConfig,
    api_key: &str,
    model: &str,
    prompt: &str,
    json_output: bool,
) -> anyhow::Result<()> {
    // Validate before making API call
    validate_provider_and_model(provider, model, &ContentType::Image { prompt: vec![] })?;
    
    match provider {
        Provider::Gemini => {
            // Gemini uses generateContent for multimodal
            if !json_output {
                println!(
                    "  {} Generating image with {} ({})...",
                    "→".blue(),
                    "Gemini".cyan(),
                    model
                );
            }
            generate_text_gemini(api_key, model, &format!("Generate an image of: {}", prompt), json_output).await
        }
        Provider::OpenAI => {
            let client = reqwest::Client::new();
            let body = serde_json::json!({
                "model": model,
                "prompt": prompt,
                "n": 1,
                "size": "1024x1024"
            });

            let resp = client
                .post("https://api.openai.com/v1/images/generations")
                .header("Authorization", format!("Bearer {}", api_key))
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                anyhow::bail!("OpenAI Images API error (HTTP {}): {}", status, err_body);
            }

            let response: serde_json::Value = resp.json().await?;
            let url = response["data"][0]["url"].as_str().unwrap_or("(no URL)");

            if json_output {
                let output = serde_json::json!({
                    "provider": "openai",
                    "model": model,
                    "type": "image",
                    "prompt": prompt,
                    "url": url,
                    "raw": response
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!();
                println!("{} Image generated!", "✓".green().bold());
                println!("  {} {}", "URL:".dimmed(), url.cyan());
                println!();
            }

            Ok(())
        }
        Provider::Fal => {
            let client = reqwest::Client::new();
            let body = serde_json::json!({
                "prompt": prompt
            });

            let url = format!("https://fal.run/{}", model);
            let resp = client
                .post(&url)
                .header("Authorization", format!("Key {}", api_key))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Fal API error (HTTP {}): {}", status, err_body);
            }

            let response: serde_json::Value = resp.json().await?;
            let image_url = response["images"][0]["url"]
                .as_str()
                .unwrap_or("(no URL in response)");

            if json_output {
                let output = serde_json::json!({
                    "provider": "fal",
                    "model": model,
                    "type": "image",
                    "prompt": prompt,
                    "url": image_url,
                    "raw": response
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!();
                println!("{} Image generated!", "✓".green().bold());
                println!("  {} {}", "URL:".dimmed(), image_url.cyan());
                println!();
            }

            Ok(())
        }
        Provider::Vertex => {
            generate_image_vertex(config, model, prompt, json_output).await
        }
        _ => {
            anyhow::bail!(
                "Image generation not supported for {}. Use -v (Vertex), -f (Fal), or -o (OpenAI).",
                provider.display_name()
            )
        }
    }
}

/// Generate image via Vertex AI.
async fn generate_image_vertex(
    config: &KalpaConfig,
    model: &str,
    prompt: &str,
    json_output: bool,
) -> anyhow::Result<()> {
    use std::path::Path;
    
    // Get service account path
    let service_account_path = config
        .get_service_account_path(Provider::Vertex)
        .ok_or_else(|| anyhow::anyhow!(
            "No service account JSON configured for Vertex AI. Run: kalpa configure"
        ))?;

    // Get project ID and location from config
    let location = config
        .get_location(Provider::Vertex)
        .unwrap_or("us-central1");

    if !json_output {
        println!(
            "  {} Generating image with {} ({})...",
            "→".blue(),
            "Vertex AI".cyan(),
            model
        );
    }

    // Get OAuth token
    let auth_token = VertexAuthToken::from_service_account_file(Path::new(service_account_path))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to authenticate with Vertex AI: {}", e))?;

    let project_id = auth_token.project_id.clone();

    // Get GCS bucket from config
    let gcs_bucket = config.get_gcs_bucket(Provider::Vertex).map(|s| s.to_string());

    // Create Vertex provider
    let provider = VertexProvider::new(
        auth_token.access_token,
        project_id.clone(),
        location.to_string(),
        gcs_bucket,
    );

    // Create request
    let request = ImageGenerationRequest {
        model: model.to_string(),
        prompt: prompt.to_string(),
        n: Some(1),
        size: None,
    };

    // Generate image
    let response = provider
        .generate_image(&request)
        .await
        .map_err(|e| anyhow::anyhow!("Vertex AI image generation failed: {}", e))?;

    // Process response
    if let Some(image) = response.images.first() {
        if let Some(b64_data) = &image.b64_data {
            // Save image to a file
            use base64::Engine;
            let image_data = base64::engine::general_purpose::STANDARD
                .decode(b64_data)
                .map_err(|e| anyhow::anyhow!("Failed to decode base64 image: {}", e))?;

            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let filename = format!("vertex_image_{}.png", timestamp);
            std::fs::write(&filename, &image_data)
                .map_err(|e| anyhow::anyhow!("Failed to write image file: {}", e))?;

            if json_output {
                let output = serde_json::json!({
                    "provider": "vertex",
                    "model": model,
                    "type": "image",
                    "prompt": prompt,
                    "file": filename,
                    "size_bytes": image_data.len()
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!();
                println!("{} Image generated!", "✓".green().bold());
                println!("  {} {}", "File:".dimmed(), filename.cyan());
                println!("  {} {} bytes", "Size:".dimmed(), image_data.len());
                println!();
            }
        } else if let Some(url) = &image.url {
            if json_output {
                let output = serde_json::json!({
                    "provider": "vertex",
                    "model": model,
                    "type": "image",
                    "prompt": prompt,
                    "url": url
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!();
                println!("{} Image generated!", "✓".green().bold());
                println!("  {} {}", "URL:".dimmed(), url.cyan());
                println!();
            }
        }
    } else {
        anyhow::bail!("No images returned from Vertex AI");
    }

    Ok(())
}

/// Generate video.
async fn generate_video(
    provider: Provider,
    config: &KalpaConfig,
    _api_key: &str,
    model: &str,
    prompt: &str,
    image_path: Option<&str>,
    is_async: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    // Validate before making API call
    validate_provider_and_model(provider, model, &ContentType::Video { prompt: "".to_string(), image_path: None, async_mode: false })?;
    
    match provider {
        Provider::Vertex => {
            if image_path.is_some() {
                anyhow::bail!("Image-to-video is not supported for Vertex AI. Use Fal.ai (-f) for image-to-video generation.");
            }
            generate_video_vertex(config, model, prompt, is_async, json_output).await
        }
        Provider::Fal => {
            generate_video_fal(_api_key, model, prompt, image_path, is_async, json_output).await
        }
        _ => {
            if json_output {
                let output = serde_json::json!({
                    "provider": provider.as_str(),
                    "model": model,
                    "type": "video",
                    "prompt": prompt,
                    "status": "not_yet_implemented",
                    "message": "Video generation is coming soon"
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!(
                    "  {} Video generation with {} ({}) coming soon.",
                    "ℹ".blue(),
                    provider.display_name().cyan(),
                    model
                );
                println!("  {} Prompt: \"{}\"", "→".dimmed(), prompt);
            }
            Ok(())
        }
    }
}

/// Generate text via Vertex AI.
async fn generate_text_vertex(
    config: &KalpaConfig,
    model: &str,
    prompt: &str,
    json_output: bool,
) -> anyhow::Result<()> {
    use kalpa_core::provider::CompletionProvider;
    use kalpa_core::types::{CompletionRequest, Message, Role};
    use std::path::Path;
    
    // Get service account path
    let service_account_path = config
        .get_service_account_path(Provider::Vertex)
        .ok_or_else(|| anyhow::anyhow!(
            "No service account JSON configured for Vertex AI. Run: kalpa configure"
        ))?;

    // Get location from config
    let location = config
        .get_location(Provider::Vertex)
        .unwrap_or("us-central1");

    if !json_output {
        println!(
            "  {} Generating text with {} ({})...",
            "→".blue(),
            "Vertex AI".cyan(),
            model
        );
    }

    // Get OAuth token
    let auth_token = VertexAuthToken::from_service_account_file(Path::new(service_account_path))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to authenticate with Vertex AI: {}", e))?;

    let project_id = auth_token.project_id.clone();

    // Get GCS bucket from config
    let gcs_bucket = config.get_gcs_bucket(Provider::Vertex).map(|s| s.to_string());

    // Create Vertex provider
    let provider = VertexProvider::new(
        auth_token.access_token,
        project_id,
        location.to_string(),
        gcs_bucket,
    );

    // Create request
    let request = CompletionRequest {
        model: model.to_string(),
        messages: vec![Message {
            role: Role::User,
            content: prompt.to_string(),
        }],
        max_tokens: None,
        temperature: None,
        top_p: None,
        stop_sequences: None,
    };

    // Generate text
    let response = provider
        .complete(&request)
        .await
        .map_err(|e| anyhow::anyhow!("Vertex AI text generation failed: {}", e))?;

    if json_output {
        let output = serde_json::json!({
            "provider": "vertex",
            "model": model,
            "type": "text",
            "prompt": prompt,
            "response": response.content,
            "usage": response.usage
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!();
        println!(
            "{}",
            format!("─── {} ({}) ", "Vertex AI", model).dimmed()
        );
        println!();
        println!("{}", response.content);
        println!();
        if let Some(usage) = response.usage {
            println!(
                "  {} {} tokens",
                "Usage:".dimmed(),
                usage.total_tokens
            );
        }
        println!("{}", "───────────────────────────────".dimmed());
    }

    Ok(())
}

/// Generate video via Vertex AI.
async fn generate_video_vertex(
    config: &KalpaConfig,
    model: &str,
    prompt: &str,
    is_async: bool,
    json_output: bool,
) -> anyhow::Result<()> {
    use kalpa_core::types::VideoGenerationRequest;
    use kalpa_core::jobs::{Job, JobStore, JobType, JobStatus};
    use std::path::Path;
    
    // Model validation is done by validate_provider_and_model() before reaching here
    
    // Get service account path
    let service_account_path = config
        .get_service_account_path(Provider::Vertex)
        .ok_or_else(|| anyhow::anyhow!(
            "No service account JSON configured for Vertex AI. Run: kalpa configure"
        ))?;

    // Get location from config
    let location = config
        .get_location(Provider::Vertex)
        .unwrap_or("us-central1");

    if !json_output {
        if is_async {
            println!(
                "  {} Starting async video generation with {} ({})...",
                "→".blue(),
                "Vertex AI".cyan(),
                model
            );
        } else {
            println!(
                "  {} Starting video generation with {} ({})...",
                "→".blue(),
                "Vertex AI".cyan(),
                model
            );
            println!("  {} Video generation can take several minutes", "ℹ".dimmed());
        }
    }

    // Get OAuth token
    let auth_token = VertexAuthToken::from_service_account_file(Path::new(service_account_path))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to authenticate with Vertex AI: {}", e))?;

    let project_id = auth_token.project_id.clone();

    // Get GCS bucket from config
    let gcs_bucket = config.get_gcs_bucket(Provider::Vertex).map(|s| s.to_string());

    // Create Vertex provider
    let provider = VertexProvider::new(
        auth_token.access_token.clone(),
        project_id.clone(),
        location.to_string(),
        gcs_bucket,
    );

    // Create job
    let mut job = Job::new(
        JobType::Video,
        "vertex".to_string(),
        model.to_string(),
        prompt.to_string(),
    );
    job.update_status(JobStatus::Running);

    // Save job
    let store = JobStore::new()?;
    store.save(&job)?;

    if !json_output {
        println!("  {} Job created: {}", "✓".green(), job.id.cyan());
    }

    // Create request
    let request = VideoGenerationRequest {
        model: model.to_string(),
        prompt: prompt.to_string(),
        image_url: None,
        duration: None,
    };

    // ASYNC MODE: Start generation and return immediately
    if is_async {
        // Start the video generation in the background
        use kalpa_libgen::vertex;
        use uuid::Uuid;
        
        let request_id = Uuid::new_v4().to_string();
        let video_instance = vertex::types::VideoInstance {
            prompt: request.prompt.clone(),
            image: None,
        };

        // Get bucket name for storage URI
        let bucket_name = if let Some(ref gcs_uri) = config.get_gcs_bucket(Provider::Vertex) {
            gcs_uri.strip_prefix("gs://")
                .and_then(|s| s.split('/').next())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("{}-kalpa-videos", project_id))
        } else {
            format!("{}-kalpa-videos", project_id)
        };
        
        let storage_uri = format!("gs://{}/generations/{}/", bucket_name, request_id);
        let parameters = Some(vertex::types::VideoParameters {
            sample_count: Some(1),
            aspect_ratio: None,
            storage_uri: Some(storage_uri.clone()),
        });

        let video_request = vertex::types::VideoPredictRequest {
            instances: vec![video_instance],
            parameters,
        };

        // Create client directly for async mode
        let base_url = format!("https://{}-aiplatform.googleapis.com", location);
        let client = vertex::Client::new_with_client(
            &base_url,
            reqwest::Client::builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert(
                        reqwest::header::AUTHORIZATION,
                        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", auth_token.access_token))
                            .unwrap(),
                    );
                    headers
                })
                .build()
                .unwrap(),
        );

        // Start the long-running operation
        let response = client
            .predict_long_running(
                &project_id,
                &location,
                &request.model,
                &video_request,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start video generation: {}", e))?;

        let operation_response = response.into_inner();
        let operation_name = operation_response.name.clone()
            .ok_or_else(|| anyhow::anyhow!("No operation name returned"))?;

        // Store operation name in job
        job.operation_id = Some(operation_name.clone());
        store.save(&job)?;

        if json_output {
            let output = serde_json::json!({
                "provider": "vertex",
                "model": model,
                "type": "video",
                "prompt": prompt,
                "status": "started",
                "job_id": job.id,
                "operation_name": operation_name,
                "request_id": request_id,
                "bucket_name": bucket_name,
                "storage_uri": storage_uri,
                "message": "Video generation started. Use 'kalpa jobs status' to check progress."
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!();
            println!("{} Video generation started in async mode!", "✓".green().bold());
            println!("  {} {}", "Job ID:".dimmed(), job.id.cyan());
            println!("  {} {}", "Operation:".dimmed(), operation_name);
            println!("  {} {}", "Storage:".dimmed(), storage_uri);
            println!();
            println!("  {} Use {} to check status", "ℹ".blue(), "kalpa jobs status".cyan());
            println!();
        }

        return Ok(());
    }

    // SYNC MODE: Wait for completion (existing behavior)
    use kalpa_core::provider::VideoGenerationProvider;
    match provider.generate_video(&request).await {
        Ok(response) => {
            // Save video to file
            if let Some(video) = response.videos.first() {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let filename = format!("vertex_video_{}.mp4", timestamp);
                
                // Handle different video URL formats
                if video.url.starts_with("data:video/mp4;base64,") {
                    // Base64-encoded video data
                    use base64::Engine;
                    let b64_data = video.url.strip_prefix("data:video/mp4;base64,").unwrap();
                    let video_data = base64::engine::general_purpose::STANDARD
                        .decode(b64_data)
                        .map_err(|e| anyhow::anyhow!("Failed to decode base64 video: {}", e))?;
                    std::fs::write(&filename, &video_data)
                        .map_err(|e| anyhow::anyhow!("Failed to write video file: {}", e))?;
                } else if video.url.starts_with("gs://") {
                    // GCS URI - download using gsutil or gcloud storage
                    if !json_output {
                        println!("  {} Downloading video from GCS: {}", "→".blue(), video.url);
                    }
                    
                    // Try gsutil first, then fall back to gcloud storage cp
                    let download_result = std::process::Command::new("gsutil")
                        .args(&["cp", &video.url, &filename])
                        .output();
                    
                    let success = match download_result {
                        Ok(output) if output.status.success() => true,
                        _ => {
                            // Fallback to gcloud storage cp
                            let gcloud_result = std::process::Command::new("gcloud")
                                .args(&["storage", "cp", &video.url, &filename])
                                .output();
                            
                            match gcloud_result {
                                Ok(output) if output.status.success() => true,
                                Ok(output) => {
                                    let stderr = String::from_utf8_lossy(&output.stderr);
                                    anyhow::bail!("Failed to download video from GCS: {}", stderr);
                                }
                                Err(e) => {
                                    anyhow::bail!("Failed to download video from GCS (gsutil/gcloud not found or failed): {}. Video available at: {}", e, video.url);
                                }
                            }
                        }
                    };
                    
                    if !success {
                        anyhow::bail!("Failed to download video from GCS. Video available at: {}", video.url);
                    }
                } else {
                    anyhow::bail!("Unexpected video URL format: {}", video.url);
                };

                // Update job as completed
                job.complete(filename.clone());
                store.save(&job)?;

                // Get file size
                let file_size = std::fs::metadata(&filename)
                    .map(|m| m.len())
                    .unwrap_or(0);

                if json_output {
                    let output = serde_json::json!({
                        "provider": "vertex",
                        "model": model,
                        "type": "video",
                        "prompt": prompt,
                        "file": filename,
                        "size_bytes": file_size,
                        "job_id": job.id
                    });
                    println!("{}", serde_json::to_string_pretty(&output)?);
                } else {
                    println!();
                    println!("{} Video generated!", "✓".green().bold());
                    println!("  {} {}", "File:".dimmed(), filename.cyan());
                    println!("  {} {} bytes", "Size:".dimmed(), file_size);
                    println!("  {} {}", "Job:".dimmed(), job.id);
                    println!();
                }
            } else {
                job.fail("No videos returned from Vertex AI".to_string());
                store.save(&job)?;
                anyhow::bail!("No videos returned from Vertex AI");
            }
        }
        Err(e) => {
            job.fail(e.to_string());
            store.save(&job)?;
            return Err(anyhow::anyhow!("Vertex AI video generation failed: {}", e));
        }
    }

    Ok(())
}
