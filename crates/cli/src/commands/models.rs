//! Models command — list available models for each provider.

use clap::Args;
use colored::Colorize;
use kalpa_core::provider::{CompletionProvider, ImageGenerationProvider, VideoGenerationProvider};

/// Arguments for the `models` subcommand.
#[derive(Debug, Args)]
#[command(
    about = "List available models for providers",
    long_about = "Display all available models for each provider, organized by type.\n\n\
                  Examples:\n  \
                  kalpa models                # List all models\n  \
                  kalpa models -f             # List Fal.ai models\n  \
                  kalpa models -g             # List Gemini models"
)]
pub struct ModelsArgs {
    /// Show models for Google Gemini.
    #[arg(short = 'g', long)]
    pub gemini: bool,

    /// Show models for Google Vertex AI.
    #[arg(short = 'v', long)]
    pub vertex: bool,

    /// Show models for Fal.ai.
    #[arg(short = 'f', long)]
    pub fal: bool,

    /// Show models for OpenAI.
    #[arg(short = 'o', long)]
    pub openai: bool,

    /// Show models for Anthropic Claude.
    #[arg(short = 'c', long)]
    pub claude: bool,
}

/// Execute the models command.
pub async fn execute(args: ModelsArgs, json: bool) -> anyhow::Result<()> {
    use kalpa_core::providers::{FalAIProvider, OpenAIProvider};

    // Determine which providers to show
    let show_all = !args.gemini && !args.vertex && !args.fal && !args.openai && !args.claude;

    if json {
        // JSON output
        let mut providers_data = serde_json::Map::new();

        if show_all || args.fal {
            let fal = FalAIProvider::new("dummy".to_string());
            providers_data.insert(
                "fal".to_string(),
                serde_json::json!({
                    "name": "Fal.ai",
                    "image_models": <FalAIProvider as ImageGenerationProvider>::supported_models(&fal),
                    "video_models": <FalAIProvider as VideoGenerationProvider>::supported_models(&fal),
                }),
            );
        }

        if show_all || args.openai {
            let openai = OpenAIProvider::new("dummy".to_string());
            providers_data.insert(
                "openai".to_string(),
                serde_json::json!({
                    "name": "OpenAI",
                    "image_models": <OpenAIProvider as ImageGenerationProvider>::supported_models(&openai),
                    "chat_models": <OpenAIProvider as CompletionProvider>::supported_models(&openai),
                }),
            );
        }

        println!("{}", serde_json::to_string_pretty(&providers_data)?);
    } else {
        // Human-readable output
        println!();
        println!("{}", "╔═══════════════════════════════════════════════════════════╗".cyan());
        println!("{}", "║                                                           ║".cyan());
        println!("{}              {} Available Models                 {}", "║".cyan(), "kalpa".bold().bright_cyan(), "║".cyan());
        println!("{}", "║                                                           ║".cyan());
        println!("{}", "╚═══════════════════════════════════════════════════════════╝".cyan());
        println!();

        if show_all || args.fal {
            print_fal_models();
        }

        if show_all || args.openai {
            print_openai_models();
        }

        if show_all || args.gemini {
            print_gemini_models();
        }

        if show_all || args.vertex {
            print_vertex_models();
        }

        if show_all || args.claude {
            print_claude_models();
        }

        println!();
        println!("  {} To use a specific model:", "💡".bright_yellow());
        println!("     {}", "kalpa generate -f --model fal-ai/flux/dev image \"prompt\"".cyan());
        println!();
    }

    Ok(())
}

fn print_fal_models() {
    println!("  {} {}", "🎨".bright_yellow(), "Fal.ai".bold().white());
    println!();
    
    // Text-to-Image models
    println!("    {} Text-to-Image Models (8)", "📸".bright_green());
    for model in &[
        ("fal-ai/flux/dev", "⭐ Balanced quality/speed"),
        ("fal-ai/flux/schnell", "⚡ Fastest generation"),
        ("fal-ai/flux-pro", "💎 Highest quality"),
        ("fal-ai/flux-realism", "📷 Photorealistic"),
        ("fal-ai/recraft-v3", ""),
        ("fal-ai/aura-flow", ""),
        ("fal-ai/stable-diffusion-v3-medium", ""),
        ("fal-ai/fast-sdxl", ""),
    ] {
        if model.1.is_empty() {
            println!("      • {}", model.0.dimmed());
        } else {
            println!("      • {} {}", model.0.cyan(), model.1.bright_black());
        }
    }
    
    println!();
    println!("    {} Text-to-Video Models (7)", "🎬".bright_green());
    for model in &[
        ("fal-ai/minimax/video-01", "⭐ Great for human motion, 6-second clips"),
        ("fal-ai/minimax/video-01-live", "📹 Live version, 1280×720"),
        ("fal-ai/hunyuan-video", "🎬 High quality, strong text-video alignment"),
        ("fal-ai/mochi-v1", "🎯 High-fidelity motion, strong prompt adherence"),
        ("fal-ai/kling-video/v1/standard/text-to-video", ""),
        ("fal-ai/kling-video/v1.5/standard/text-to-video", ""),
        ("fal-ai/wan/v2.2-a14b/text-to-video", ""),
    ] {
        if model.1.is_empty() {
            println!("      • {}", model.0.dimmed());
        } else {
            println!("      • {} {}", model.0.cyan(), model.1.bright_black());
        }
    }
    
    println!();
    println!("    {} Image-to-Video Models (5)", "🎞️ ".bright_green());
    for model in &[
        ("fal-ai/luma-dream-machine", "⭐ Recommended"),
        ("fal-ai/kling-video/v1/standard/image-to-video", ""),
        ("fal-ai/kling-video/v1.5/standard/image-to-video", ""),
        ("fal-ai/minimax/video-01/image-to-video", ""),
        ("fal-ai/wan/v2.2-a14b/image-to-video", ""),
    ] {
        if model.1.is_empty() {
            println!("      • {}", model.0.dimmed());
        } else {
            println!("      • {} {}", model.0.cyan(), model.1.bright_black());
        }
    }
    println!();
}

fn print_openai_models() {
    use kalpa_core::providers::OpenAIProvider;
    
    let openai = OpenAIProvider::new("dummy".to_string());
    
    println!("  {} {}", "🤖".bright_yellow(), "OpenAI".bold().white());
    println!();
    
    println!("    {} Image Models", "📸".bright_green());
    for model in <OpenAIProvider as ImageGenerationProvider>::supported_models(&openai) {
        if model.contains("dall-e") {
            let desc = match *model {
                "dall-e-3" => "⭐ Latest, highest quality",
                "dall-e-2" => "Classic version",
                _ => "",
            };
            if desc.is_empty() {
                println!("      • {}", model.dimmed());
            } else {
                println!("      • {} {}", model.cyan(), desc.bright_black());
            }
        }
    }
    
    println!();
    println!("    {} Text/Chat Models", "💬".bright_green());
    for model in &[
        ("gpt-4.1", "⭐ Latest GPT-4 Turbo"),
        ("gpt-4.1-mini", "⚡ Fast and affordable"),
        ("gpt-4", ""),
        ("gpt-3.5-turbo", ""),
    ] {
        if model.1.is_empty() {
            println!("      • {}", model.0.dimmed());
        } else {
            println!("      • {} {}", model.0.cyan(), model.1.bright_black());
        }
    }
    println!();
}

fn print_gemini_models() {
    println!("  {} {}", "✨".bright_yellow(), "Google Gemini".bold().white());
    println!();
    
    println!("    {} Text/Chat Models", "💬".bright_green());
    for model in &[
        ("gemini-3.1-flash", "⭐ Fast and efficient"),
        ("gemini-1.5-flash", ""),
        ("gemini-1.5-pro", "💎 Most capable"),
    ] {
        if model.1.is_empty() {
            println!("      • {}", model.0.dimmed());
        } else {
            println!("      • {} {}", model.0.cyan(), model.1.bright_black());
        }
    }
    println!();
}

fn print_vertex_models() {
    println!("  {} {}", "☁️ ".bright_yellow(), "Google Vertex AI".bold().white());
    println!();
    
    println!("    {} Text/Chat Models (Gemini)", "💬".bright_green());
    for model in &[
        ("gemini-3.1-flash", "⭐ Fast and efficient"),
        ("gemini-1.5-flash", ""),
        ("gemini-1.5-pro", "💎 Most capable"),
    ] {
        if model.1.is_empty() {
            println!("      • {}", model.0.dimmed());
        } else {
            println!("      • {} {}", model.0.cyan(), model.1.bright_black());
        }
    }
    
    println!();
    println!("    {} Image Models (Imagen)", "📸".bright_green());
    for model in &[
        ("imagen-3.0-generate-001", "⭐ Latest Imagen"),
        ("imagegeneration@006", ""),
    ] {
        if model.1.is_empty() {
            println!("      • {}", model.0.dimmed());
        } else {
            println!("      • {} {}", model.0.cyan(), model.1.bright_black());
        }
    }
    
    println!();
    println!("    {} Video Models (Veo)", "🎬".bright_green());
    println!("      • {} {}", "veo-001".cyan(), "⭐ Google's video generation".bright_black());
    println!();
}

fn print_claude_models() {
    println!("  {} {}", "🧠".bright_yellow(), "Anthropic Claude".bold().white());
    println!();
    
    println!("    {} Text/Chat Models", "💬".bright_green());
    for model in &[
        ("claude-opus-4-7", "💎 Most capable, highest quality"),
        ("claude-opus-4-6", ""),
        ("claude-sonnet-4-6", "⭐ Balanced performance"),
        ("claude-haiku-4-5-20251001", "⚡ Fast and efficient"),
        ("claude-3-opus", "Previous generation"),
        ("claude-3-sonnet", ""),
        ("claude-3-haiku", ""),
    ] {
        if model.1.is_empty() {
            println!("      • {}", model.0.dimmed());
        } else {
            println!("      • {} {}", model.0.cyan(), model.1.bright_black());
        }
    }
    println!();
}
