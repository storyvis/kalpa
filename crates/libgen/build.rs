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
    // Fail closed if a Progenitor upgrade changes this boundary: no generated
    // operation may bypass the bounded body reader. Its public hooks only borrow.
    let response_boundary = "let response = result ? ;";
    let operations = spec
        .paths
        .iter()
        .map(|(_, item)| item.as_item().map(|item| item.iter().count()).unwrap_or(0))
        .sum::<usize>();
    assert_eq!(
        content.matches(response_boundary).count(),
        operations,
        "Generated response boundary changed for {name}"
    );
    content = content.replace(
        response_boundary,
        "let response = crate :: bounded_response (result ?) . await ? ;",
    );

    // Fal model IDs contain multiple segments; preserve only their separators.
    // Keep request IDs encoded and fail closed on generator changes.
    if name == "falai" {
        let model_boundary = "encode_path (& model_id . to_string ())";
        assert_eq!(
            content.matches(model_boundary).count(),
            operations,
            "Fal model path boundary changed"
        );
        content = content.replace(model_boundary, "crate :: encode_model_path (model_id) ?");
    }

    // Format with prettyplease for readable output
    let ast = syn::parse_file(&content)
        .unwrap_or_else(|e| panic!("Failed to parse generated code for {name}: {e}"));

    let formatted = prettyplease::unparse(&ast);
    fs::write(&output_path, formatted).expect("Failed to write generated SDK");

    eprintln!(
        "cargo:warning=Generated SDK for '{}' at {:?}",
        name, output_path
    );
}
