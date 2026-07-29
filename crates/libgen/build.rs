//! Build script for kalpa-libgen.
//!
//! This script reads OpenAPI 3.0 spec files from `specs/` and generates
//! type-safe Rust client code using progenitor. Generated files are placed
//! in the cargo OUT_DIR and included via `include!()` in lib.rs.
//!
//! ## How to add a new provider:
//!
//! 1. Place the spec file at `specs/<name>.json`
//! 2. Add a `generate_sdk("name")` call below
//! 3. Add the corresponding module in `src/lib.rs`

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let spec_dir = Path::new("specs");

    // Only generate if the specs directory exists and has files
    if !spec_dir.exists() {
        eprintln!("cargo:warning=No specs/ directory found. Skipping SDK generation.");
        return;
    }

    let entries = fs::read_dir(spec_dir).expect("Failed to read specs directory");

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "json") {
            let stem = path
                .file_stem()
                .expect("file has no stem")
                .to_str()
                .expect("non-utf8 filename")
                .to_string();

            generate_sdk(&path, &stem, &out_dir);
        }
    }

    // Re-run if specs change
    println!("cargo:rerun-if-changed=specs/");
}

fn generate_sdk(spec_path: &Path, name: &str, out_dir: &Path) {
    let spec_content = fs::read_to_string(spec_path)
        .unwrap_or_else(|e| panic!("Failed to read spec file {}: {}", spec_path.display(), e));

    let spec: openapiv3::OpenAPI = serde_json::from_str(&spec_content)
        .unwrap_or_else(|e| panic!("Failed to parse spec file {}: {}", spec_path.display(), e));

    let mut generator = progenitor::Generator::default();
    let tokens = generator
        .generate_tokens(&spec)
        .unwrap_or_else(|e| panic!("Failed to generate SDK for {}: {}", name, e));

    let output_path = out_dir.join(format!("{}.rs", name));
    let mut content = tokens.to_string();

    // Special handling for falai: remove encode_path() for model_id to preserve slashes
    // Apply patch on raw token output BEFORE formatting (no line breaks in token stream)
    if name == "falai" {
        let before_count = content.matches("encode_path").count();
        // Replace all encode_path calls - model_id contains slashes, request_id is UUID (safe)
        content = content.replace("encode_path (& model_id . to_string ())", "model_id");
        content = content.replace("encode_path (& request_id . to_string ())", "& request_id");
        let after_count = content.matches("encode_path").count();
        eprintln!("cargo:warning=Applied falai-specific patch: disabled path encoding ({} replacements)", before_count - after_count);
    }

    // Format with prettyplease for readable output
    let ast = syn::parse_file(&content).unwrap_or_else(|e| {
        // If parsing fails, write unformatted
        eprintln!("cargo:warning=Failed to parse generated code for {}: {}", name, e);
        fs::write(&output_path, &content).expect("Failed to write generated SDK");
        return syn::parse_file("").unwrap();
    });

    let formatted = prettyplease::unparse(&ast);
    fs::write(&output_path, formatted).expect("Failed to write generated SDK");

    eprintln!("cargo:warning=Generated SDK for '{}' at {:?}", name, output_path);
}
