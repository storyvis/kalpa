//! Status command — check the health and configuration of providers.

use clap::Args;
use colored::Colorize;
use kalpa_core::{KalpaConfig, Provider};

/// Arguments for the `status` subcommand.
#[derive(Debug, Args)]
#[command(
    about = "Check the status of configured providers",
    long_about = "Check connectivity and configuration status for all or specific providers.\n\n\
                  Examples:\n  \
                  kalpa status           # Show all providers\n  \
                  kalpa status -g        # Check only Gemini"
)]
pub struct StatusArgs {
    /// Check Google Gemini status.
    #[arg(short = 'g', long)]
    pub gemini: bool,

    /// Check Google Vertex AI status.
    #[arg(short = 'v', long)]
    pub vertex: bool,

    /// Check Fal.ai status.
    #[arg(short = 'f', long)]
    pub fal: bool,

    /// Check OpenAI status.
    #[arg(short = 'o', long)]
    pub openai: bool,

    /// Check Anthropic Claude status.
    #[arg(short = 'c', long)]
    pub claude: bool,
}

/// Execute the status command.
pub async fn execute(args: StatusArgs, json: bool) -> anyhow::Result<()> {
    let config = KalpaConfig::load()?;

    let providers = if args.gemini || args.vertex || args.fal || args.openai || args.claude {
        let mut p = Vec::new();
        if args.gemini { p.push(Provider::Gemini); }
        if args.vertex { p.push(Provider::Vertex); }
        if args.fal { p.push(Provider::Fal); }
        if args.openai { p.push(Provider::OpenAI); }
        if args.claude { p.push(Provider::Claude); }
        p
    } else {
        Provider::all().to_vec()
    };

    if json {
        let mut statuses = Vec::new();
        for provider in &providers {
            let configured = config.is_configured(*provider);
            statuses.push(serde_json::json!({
                "provider": provider.as_str(),
                "display_name": provider.display_name(),
                "configured": configured,
                "default_model": config.get_default_model(*provider)
            }));
        }

        let output = serde_json::json!({
            "providers": statuses,
            "config_path": KalpaConfig::config_path().ok().map(|p| p.display().to_string())
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", "Provider Status".bold().underline());
        println!();

        for provider in &providers {
            let configured = config.is_configured(*provider);

            let config_icon = if configured {
                "✓".green().bold()
            } else {
                "✗".red().bold()
            };

            let status_str = if configured {
                format!("{}", "configured".green())
            } else {
                format!("{}", "no key".dimmed())
            };

            let model = config
                .get_default_model(*provider)
                .unwrap_or("(default)");

            println!(
                "  {} {:<16}  {}  model: {}",
                config_icon,
                provider.display_name(),
                status_str,
                model.dimmed()
            );
        }

        println!();

        let unconfigured: Vec<&Provider> = providers
            .iter()
            .filter(|p| !config.is_configured(**p))
            .collect();

        if !unconfigured.is_empty() {
            println!(
                "  {} To configure: {}",
                "ℹ".blue(),
                "kalpa configure".cyan()
            );
        }
    }

    Ok(())
}
