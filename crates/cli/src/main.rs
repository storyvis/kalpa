//! # kalpa CLI
//!
//! The main entry point for the kalpa command-line tool.

use clap::Parser;
use tracing_subscriber::EnvFilter;

mod commands;
#[allow(dead_code)]
mod output;

use commands::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing/logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    commands::execute(cli).await
}
