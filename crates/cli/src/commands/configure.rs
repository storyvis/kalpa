//! Configure command — interactive setup of API keys and settings.

use clap::Args;
use colored::Colorize;
use dialoguer::{Input, Select};
use kalpa_core::{KalpaConfig, Provider};

/// Arguments for the `configure` subcommand.
#[derive(Debug, Args)]
#[command(
    about = "Configure API keys and provider settings",
    long_about = "Interactively configure API keys and provider settings.\n\
                  Keys are stored in ~/.config/kalpa/config.toml\n\n\
                  Examples:\n  \
                  kalpa configure              # Interactive setup\n  \
                  kalpa configure --set gemini.api_key YOUR_KEY\n  \
                  kalpa configure --show"
)]
pub struct ConfigureArgs {
    /// Set a config value directly (key=value format).
    /// Supported keys: <provider>.api_key, <provider>.default_model, defaults.provider
    #[arg(long = "set", value_name = "KEY VALUE", num_args = 2)]
    pub set: Option<Vec<String>>,

    /// Show current configuration (keys are masked).
    #[arg(long)]
    pub show: bool,
}

/// Execute the configure command.
pub async fn execute(args: ConfigureArgs) -> anyhow::Result<()> {
    if args.show {
        return show_config().await;
    }

    if let Some(ref kv) = args.set {
        return set_config_value(&kv[0], &kv[1]).await;
    }

    // Interactive configuration
    interactive_configure().await
}

/// Show current configuration with masked keys.
async fn show_config() -> anyhow::Result<()> {
    let config = KalpaConfig::load()?;
    let config_path = KalpaConfig::config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    println!();
    println!("{}", "╔═══════════════════════════════════════════════════════════╗".cyan());
    println!("{}", "║                                                           ║".cyan());
    println!("{}                 {} Configuration                  {}", "║".cyan(), "kalpa".bold().bright_cyan(), "║".cyan());
    println!("{}", "║                                                           ║".cyan());
    println!("{}", "╚═══════════════════════════════════════════════════════════╝".cyan());
    println!();
    
    println!("  {} {}", "📁 Config file:".bright_yellow(), config_path.bright_black());
    println!(
        "  {} {}",
        "🎯 Default provider:".bright_yellow(),
        config.defaults.provider.bright_cyan().bold()
    );
    println!();
    println!("{}", "  Configured Providers:".bold().white());
    println!();

    for provider in Provider::all() {
        let (icon, key_status) = match config.get_api_key(*provider) {
            Some(key) => {
                let masked = mask_key(key);
                ("✓".green().bold(), masked.green())
            }
            None => ("○".bright_black(), "not configured".bright_black()),
        };

        let model = config
            .get_default_model(*provider)
            .unwrap_or("(default)");

        println!(
            "    {} {:<18} {} {}   {} {}",
            icon,
            provider.display_name().cyan(),
            "key:".dimmed(),
            key_status,
            "model:".dimmed(),
            model.bright_black()
        );
    }

    println!();
    Ok(())
}

/// Set a configuration value directly.
async fn set_config_value(key: &str, value: &str) -> anyhow::Result<()> {
    let mut config = KalpaConfig::load()?;

    let parts: Vec<&str> = key.split('.').collect();
    match parts.as_slice() {
        [provider_name, "api_key"] => {
            let provider = Provider::from_str(provider_name)?;
            config.set_api_key(provider, value.to_string());
            config.save()?;
            println!(
                "{} Set API key for {}",
                "✓".green().bold(),
                provider.display_name().cyan()
            );
        }
        [provider_name, "default_model"] => {
            let provider = Provider::from_str(provider_name)?;
            config.set_default_model(provider, value.to_string());
            config.save()?;
            println!(
                "{} Set default model for {} to {}",
                "✓".green().bold(),
                provider.display_name().cyan(),
                value.white()
            );
        }
        ["defaults", "provider"] => {
            // Validate the provider name
            Provider::from_str(value)?;
            config.defaults.provider = value.to_string();
            config.save()?;
            println!(
                "{} Set default provider to {}",
                "✓".green().bold(),
                value.cyan()
            );
        }
        _ => {
            anyhow::bail!(
                "Unknown config key '{}'. Valid keys:\n  \
                 <provider>.api_key\n  \
                 <provider>.default_model\n  \
                 defaults.provider\n\n\
                 Providers: gemini, vertex, fal, openai, claude",
                key
            );
        }
    }

    Ok(())
}

/// Interactive configuration wizard.
async fn interactive_configure() -> anyhow::Result<()> {
    // Beautiful welcome banner
    println!();
    println!("{}", "╔═══════════════════════════════════════════════════════════╗".cyan());
    println!("{}", "║                                                           ║".cyan());
    println!("{}                    {} {}                      {}", "║".cyan(), "Welcome to".white(), "kalpa".bold().bright_cyan(), "║".cyan());
    println!("{}", "║                                                           ║".cyan());
    println!("{}", "║          A unified CLI for AI generative models           ║".cyan());
    println!("{}", "║                                                           ║".cyan());
    println!("{}", "╚═══════════════════════════════════════════════════════════╝".cyan());
    println!();
    
    println!("  {} {}", "✨".bright_yellow(), "Let's get you set up with AI providers!".bold().white());
    println!();
    println!("  {} API keys are stored securely and locally at:", "🔐".bright_yellow());
    println!("     {}", 
        KalpaConfig::config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~/.config/kalpa/config.toml".into())
            .bright_black()
    );
    println!();
    println!("  {} Choose a provider to configure:", "📋".bright_yellow());
    println!();

    let mut config = KalpaConfig::load()?;

    let providers: Vec<&str> = Provider::all()
        .iter()
        .map(|p| p.display_name())
        .collect();

    let selection = Select::new()
        .with_prompt("Which provider would you like to configure?")
        .items(&providers)
        .default(0)
        .interact()?;

    let provider = Provider::all()[selection];

    // Handle Vertex AI configuration specially
    if matches!(provider, Provider::Vertex) {
        configure_vertex_ai(&mut config)?;
    } else {
        // Standard API key configuration for other providers
        let current_key = config.get_api_key(provider);
        let prompt_text = match current_key {
            Some(key) => format!("API key for {} (current: {})", provider.display_name(), mask_key(key)),
            None => format!("API key for {}", provider.display_name()),
        };

        let key: String = Input::new()
            .with_prompt(&prompt_text)
            .interact_text()?;

        if !key.is_empty() {
            config.set_api_key(provider, key);
        }

        // Ask for default model
        let model: String = Input::new()
            .with_prompt(format!("Default model for {} (press Enter to skip)", provider.display_name()))
            .allow_empty(true)
            .interact_text()?;

        if !model.is_empty() {
            config.set_default_model(provider, model);
        }
    }

    config.save()?;

    println!();
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".green());
    println!(
        "  {} Configuration saved for {}!",
        "✓".green().bold(),
        provider.display_name().bright_cyan().bold()
    );
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".green());
    println!();
    println!("  {} Next steps:", "🚀".bright_yellow());
    println!();
    println!("     {} Verify authentication:", "1.".bold().white());
    println!("        {}", format!("kalpa auth -{}", provider_flag_char(provider)).cyan());
    println!();
    println!("     {} Generate content:", "2.".bold().white());
    println!("        {}", format!("kalpa generate -{} text \"Hello!\"", provider_flag_char(provider)).cyan());
    println!();
    println!("  {} Check all providers:", "💡".bright_yellow());
    println!("     {}", "kalpa status".cyan());
    println!();

    Ok(())
}

/// Mask an API key for display (show first 4 and last 4 chars).
fn mask_key(key: &str) -> String {
    if key.len() <= 8 {
        return "****".to_string();
    }
    format!("{}...{}", &key[..4], &key[key.len() - 4..])
}

/// Configure Vertex AI with service account JSON file.
fn configure_vertex_ai(config: &mut KalpaConfig) -> anyhow::Result<()> {
    use std::fs;
    use serde_json::Value;

    println!();
    println!("  {} Vertex AI uses a service account JSON key file for authentication.", "ℹ️ ".bright_cyan());
    println!();

    // Ask for service account JSON path
    let json_path: String = Input::new()
        .with_prompt("Path to service account JSON key file")
        .interact_text()?;

    // Validate and read the JSON file
    let expanded_path = shellexpand::tilde(&json_path).to_string();
    
    if !std::path::Path::new(&expanded_path).exists() {
        anyhow::bail!("File not found: {}", expanded_path);
    }

    // Read and parse JSON to extract project_id
    let json_content = fs::read_to_string(&expanded_path)
        .map_err(|e| anyhow::anyhow!("Failed to read JSON file: {}", e))?;
    
    let json: Value = serde_json::from_str(&json_content)
        .map_err(|e| anyhow::anyhow!("Failed to parse JSON: {}", e))?;

    // Extract and validate project_id
    let project_id = json.get("project_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("'project_id' not found in JSON key file"))?;

    println!();
    println!("  {} Detected project ID: {}", "✓".green().bold(), project_id.bright_cyan());
    
    // Store the service account path
    config.set_service_account_path(Provider::Vertex, expanded_path);

    // Ask for GCS bucket (optional)
    println!();
    println!("  {} GCS bucket for video outputs (optional, required for video generation):", "💾".bright_yellow());
    let gcs_bucket: String = Input::new()
        .with_prompt("GCS bucket URL (e.g., gs://my-bucket)")
        .allow_empty(true)
        .interact_text()?;

    if !gcs_bucket.is_empty() {
        config.set_gcs_bucket(Provider::Vertex, gcs_bucket);
    }

    // Ask for location/region
    println!();
    let location: String = Input::new()
        .with_prompt("GCP region/location")
        .default("us-central1".into())
        .interact_text()?;

    if !location.is_empty() {
        config.set_location(Provider::Vertex, location);
    }

    // Ask for default model
    println!();
    let model: String = Input::new()
        .with_prompt("Default model (press Enter to skip)")
        .default("gemini-3.1-flash".into())
        .allow_empty(true)
        .interact_text()?;

    if !model.is_empty() {
        config.set_default_model(Provider::Vertex, model);
    }

    Ok(())
}

/// Get the short flag character for a provider.
fn provider_flag_char(provider: Provider) -> char {
    match provider {
        Provider::Gemini => 'g',
        Provider::Vertex => 'v',
        Provider::Fal => 'f',
        Provider::OpenAI => 'o',
        Provider::Claude => 'c',
    }
}
