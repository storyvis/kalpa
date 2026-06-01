//! Output formatting utilities for the CLI.
//!
//! Provides consistent, beautiful terminal output with spinners,
//! progress bars, and colored text.

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;
use tracing;

/// Create a spinner for long-running operations.
pub fn spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.cyan} {msg}")
            .expect("invalid spinner template"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

/// Print a success message.
pub fn success(message: &str) {
    println!("{} {}", "✓".green().bold(), message);
}

/// Print an error message.
pub fn error(message: &str) {
    tracing::error!("{}", message);
}

/// Print a warning message.
pub fn warning(message: &str) {
    println!("{} {}", "⚠".yellow().bold(), message);
}

/// Print an info message.
pub fn info(message: &str) {
    println!("{} {}", "ℹ".blue().bold(), message);
}

/// Format and print an AI response in a visually distinct block.
pub fn ai_response(provider: &str, model: &str, content: &str) {
    println!();
    println!(
        "{}",
        format!("─── {} ({}) ", provider, model)
            .dimmed()
    );
    println!();
    println!("{}", content);
    println!();
    println!("{}", "───────────────────────────────".dimmed());
}
