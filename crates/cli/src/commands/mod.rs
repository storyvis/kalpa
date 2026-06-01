//! CLI command definitions and execution logic.

pub mod auth;
pub mod configure;
pub mod generate;
pub mod jobs;
pub mod models;
pub mod status;

use clap::{Parser, Subcommand};

/// kalpa — a unified CLI for AI generative models.
#[derive(Debug, Parser)]
#[command(
    name = "kalpa",
    version,
    about = "A unified CLI for AI generative models",
    long_about = "kalpa provides a beautiful, unified interface to interact with multiple AI providers \
                  (Gemini, Vertex AI, Fal, OpenAI, and more) from your terminal.\n\n\
                  Get started:\n  \
                  kalpa configure              # Set up API keys\n  \
                  kalpa auth -g                # Verify Gemini authentication\n  \
                  kalpa generate -g text \"Hello!\"  # Generate text with Gemini"
)]
pub struct Cli {
    /// Enable verbose/debug output.
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Output as JSON instead of formatted text.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// Top-level subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Authenticate and verify API keys for providers.
    Auth(auth::AuthArgs),

    /// Generate content (text, image, video) from AI providers.
    Generate(generate::GenerateArgs),

    /// List available models for each provider.
    Models(models::ModelsArgs),

    /// Check the status of configured providers.
    Status(status::StatusArgs),

    /// Configure API keys and provider settings.
    Configure(configure::ConfigureArgs),

    /// Manage async generation jobs.
    Jobs(jobs::JobsArgs),
}

/// Execute the parsed CLI command.
pub async fn execute(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Auth(args) => auth::execute(args, cli.json).await,
        Commands::Generate(args) => generate::execute(args, cli.json).await,
        Commands::Models(args) => models::execute(args, cli.json).await,
        Commands::Status(args) => status::execute(args, cli.json).await,
        Commands::Configure(args) => configure::execute(args).await,
        Commands::Jobs(args) => jobs::execute(args, cli.json).await,
    }
}
