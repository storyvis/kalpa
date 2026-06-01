//! Models command — list available models for each provider.

use clap::Args;
use colored::Colorize;
use kalpa_core::registry::{self, ContentKind};
use kalpa_core::Provider;

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
    // Determine which providers to show
    let show_all = !args.gemini && !args.vertex && !args.fal && !args.openai && !args.claude;

    if json {
        // JSON output — use registry as single source of truth
        let mut providers_data = serde_json::Map::new();

        let providers_to_show: Vec<Provider> = if show_all {
            Provider::all().to_vec()
        } else {
            let mut v = Vec::new();
            if args.gemini { v.push(Provider::Gemini); }
            if args.vertex { v.push(Provider::Vertex); }
            if args.fal { v.push(Provider::Fal); }
            if args.openai { v.push(Provider::OpenAI); }
            if args.claude { v.push(Provider::Claude); }
            v
        };

        for provider in providers_to_show {
            let text_models = registry::model_ids(provider, ContentKind::Text);
            let image_models = registry::model_ids(provider, ContentKind::Image);
            let video_models = registry::model_ids(provider, ContentKind::Video);

            let mut entry = serde_json::Map::new();
            entry.insert("name".into(), serde_json::json!(provider.display_name()));
            if !text_models.is_empty() {
                entry.insert("text_models".into(), serde_json::json!(text_models));
            }
            if !image_models.is_empty() {
                entry.insert("image_models".into(), serde_json::json!(image_models));
            }
            if !video_models.is_empty() {
                entry.insert("video_models".into(), serde_json::json!(video_models));
            }

            providers_data.insert(provider.as_str().to_string(), serde_json::Value::Object(entry));
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
    println!("  {} {}", "🤖".bright_yellow(), "OpenAI".bold().white());
    println!();
    
    println!("    {} Image Models", "📸".bright_green());
    for model in registry::model_ids(Provider::OpenAI, ContentKind::Image) {
        let desc = match model {
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
    
    println!();
    println!("    {} Text/Chat Models", "💬".bright_green());
    for model in registry::model_ids(Provider::OpenAI, ContentKind::Text) {
        let desc = match model {
            "gpt-4.1" => "⭐ Latest GPT-4 Turbo",
            "gpt-4.1-mini" => "⚡ Fast and affordable",
            _ => "",
        };
        if desc.is_empty() {
            println!("      • {}", model.dimmed());
        } else {
            println!("      • {} {}", model.cyan(), desc.bright_black());
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
