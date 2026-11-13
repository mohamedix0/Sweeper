fn main() {
    println!("cargo:rerun-if-changed=allowlist.toml");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(&manifest_dir).join("allowlist.toml");
    let toml_text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    if let Err(e) = sweeper_knowledge_lint::lint_allowlist_toml(&toml_text) {
        panic!("allowlist.toml lint failed: {e}");
    }
}

// Compile the shared linter implementation for build-time enforcement.
#[path = "src/lint.rs"]
mod sweeper_knowledge_lint;
