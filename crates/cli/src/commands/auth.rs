//! Auth command — authenticate and verify API keys for providers.

use clap::Args;
use colored::Colorize;
use kalpa_core::{KalpaConfig, Provider};

/// Arguments for the `auth` subcommand.
#[derive(Debug, Args)]
#[command(
    about = "Authenticate and verify API keys for providers",
    long_about = "Verify that your API keys are valid and providers are accessible.\n\
                  You can also supply a key directly to authenticate in one step.\n\n\
                  Examples:\n  \
                  kalpa auth -g              # Verify Gemini key from config\n  \
                  kalpa auth -g --key YOUR_KEY  # Auth with a new key\n  \
                  kalpa auth --all           # Verify all configured providers"
)]
pub struct AuthArgs {
    /// Authenticate with Google Gemini.
    #[arg(short = 'g', long)]
    pub gemini: bool,

    /// Authenticate with Google Vertex AI.
    #[arg(short = 'v', long)]
    pub vertex: bool,

    /// Authenticate with Fal.ai.
    #[arg(short = 'f', long)]
    pub fal: bool,

    /// Authenticate with OpenAI.
    #[arg(short = 'o', long)]
    pub openai: bool,

    /// Authenticate with Anthropic Claude.
    #[arg(short = 'c', long)]
    pub claude: bool,

    /// Verify all configured providers.
    #[arg(short = 'a', long)]
    pub all: bool,

    /// Supply an API key directly (saves it to config).
    #[arg(short = 'k', long)]
    pub key: Option<String>,
}

/// Execute the auth command.
pub async fn execute(args: AuthArgs, json: bool) -> anyhow::Result<()> {
    let mut config = KalpaConfig::load()?;

    // Determine which providers to authenticate
    let providers = if args.all {
        Provider::all().to_vec()
    } else {
        let mut p = Vec::new();
        if args.gemini { p.push(Provider::Gemini); }
        if args.vertex { p.push(Provider::Vertex); }
        if args.fal { p.push(Provider::Fal); }
        if args.openai { p.push(Provider::OpenAI); }
        if args.claude { p.push(Provider::Claude); }
        p
    };

    if providers.is_empty() {
        anyhow::bail!(
            "Please specify a provider: -g (gemini), -v (vertex), -f (fal), -o (openai), -c (claude), or --all"
        );
    }

    // If a key is provided and there's exactly one provider, save it
    if let Some(ref key) = args.key {
        if providers.len() == 1 {
            config.set_api_key(providers[0], key.clone());
            config.save()?;
            if !json {
                println!(
                    "{} Saved API key for {}",
                    "✓".green().bold(),
                    providers[0].display_name().cyan()
                );
            }
        } else {
            anyhow::bail!("--key can only be used with a single provider flag");
        }
    }

    let mut results = Vec::new();

    for provider in &providers {
        // Vertex AI uses service account path, not API key
        if *provider == Provider::Vertex {
            let result = verify_vertex_auth(&config).await;
            match result {
                Ok(()) => results.push((*provider, true, "authenticated".to_string())),
                Err(e) => results.push((*provider, false, e.to_string())),
            }
            continue;
        }

        let api_key = match config.get_api_key(*provider) {
            Some(k) => k.to_string(),
            None => {
                results.push((*provider, false, "no API key configured".to_string()));
                continue;
            }
        };

        let result = verify_auth(*provider, &api_key).await;
        match result {
            Ok(()) => results.push((*provider, true, "authenticated".to_string())),
            Err(e) => results.push((*provider, false, e.to_string())),
        }
    }

    if json {
        let output: Vec<_> = results
            .iter()
            .map(|(p, success, msg)| {
                serde_json::json!({
                    "provider": p.as_str(),
                    "display_name": p.display_name(),
                    "authenticated": success,
                    "message": msg
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        for (provider, success, msg) in &results {
            if *success {
                println!(
                    "  {} {} — {}",
                    "✓".green().bold(),
                    provider.display_name().cyan(),
                    msg.green()
                );
            } else {
                println!(
                    "  {} {} — {}",
                    "✗".red().bold(),
                    provider.display_name(),
                    msg.red()
                );
            }
        }
    }

    Ok(())
}

/// Verify Vertex AI authentication with service account JSON.
async fn verify_vertex_auth(config: &KalpaConfig) -> anyhow::Result<()> {
    use std::fs;
    use serde_json::Value;

    // Check if service account path is configured
    let service_account_path = config
        .get_service_account_path(Provider::Vertex)
        .ok_or_else(|| anyhow::anyhow!("No service account JSON configured. Run: kalpa configure"))?;

    // Check if file exists
    if !std::path::Path::new(service_account_path).exists() {
        anyhow::bail!("Service account file not found: {}", service_account_path);
    }

    // Read and parse JSON to validate it's a proper service account key
    let json_content = fs::read_to_string(service_account_path)
        .map_err(|e| anyhow::anyhow!("Failed to read service account JSON: {}", e))?;
    
    let json: Value = serde_json::from_str(&json_content)
        .map_err(|e| anyhow::anyhow!("Invalid JSON format: {}", e))?;

    // Validate required fields
    let project_id = json.get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'project_id' not found in service account JSON"))?;

    let client_email = json.get("client_email")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'client_email' not found in service account JSON"))?;

    let private_key = json.get("private_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'private_key' not found in service account JSON"))?;

    // Validate private key format (should start with -----BEGIN PRIVATE KEY-----)
    if !private_key.trim().starts_with("-----BEGIN") {
        anyhow::bail!("Invalid private key format in service account JSON");
    }

    // Success - JSON is valid with all required fields
    Ok(())
}

/// Verify authentication with a specific provider.
async fn verify_auth(provider: Provider, api_key: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    match provider {
        Provider::Gemini => {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={}",
                api_key
            );
            let resp = client.get(&url).send().await?;
            if resp.status().is_success() {
                Ok(())
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("HTTP {} — {}", status, body.chars().take(100).collect::<String>())
            }
        }
        Provider::OpenAI => {
            let resp = client
                .get("https://api.openai.com/v1/models")
                .header("Authorization", format!("Bearer {}", api_key))
                .send()
                .await?;
            if resp.status().is_success() {
                Ok(())
            } else {
                let status = resp.status();
                anyhow::bail!("HTTP {}", status)
            }
        }
        Provider::Vertex => {
            // This shouldn't be called - Vertex uses verify_vertex_auth
            anyhow::bail!("Use verify_vertex_auth for Vertex AI")
        }
        Provider::Fal => {
            // Use the platform API to list models - this validates auth
            let resp = client
                .get("https://api.fal.ai/v1/models?limit=1")
                .header("Authorization", format!("Key {}", api_key))
                .send()
                .await?;
            
            if resp.status().is_success() {
                Ok(())
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                
                // Check for specific authentication errors
                if status == 401 || status == 403 {
                    anyhow::bail!("Invalid API key - authentication failed")
                } else {
                    anyhow::bail!("HTTP {} — {}", status, body.chars().take(100).collect::<String>())
                }
            }
        }
        Provider::Claude => {
            // Verify using the Messages API with minimal request
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&serde_json::json!({
                    "model": "claude-3-haiku-20240307",
                    "max_tokens": 1,
                    "messages": [
                        {"role": "user", "content": "Hi"}
                    ]
                }))
                .send()
                .await?;
            
            if resp.status().is_success() {
                Ok(())
            } else {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                
                if status == 401 || status == 403 {
                    anyhow::bail!("Invalid API key - authentication failed")
                } else {
                    anyhow::bail!("HTTP {} — {}", status, body.chars().take(100).collect::<String>())
                }
            }
        }
    }
}
