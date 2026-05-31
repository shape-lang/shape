//! Build script to extract grammar rules from pest file
//!
//! This generates a Rust file containing all rule names from the Shape
//! pest grammar, which is used for coverage analysis.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

fn main() {
    generate_grammar_features();
    emit_compiler_fingerprint();
    emit_prelude_bundle_presence();
}

/// DESIGN decision 4a — set `cfg(prelude_bundle_present)` only when the baked
/// prelude `SHAPEPKG` bundle (`embedded/core_prelude.shapec`) exists. This lets
/// `stdlib.rs`'s `include_bytes_optional!` resolve the artifact when present and
/// expand to `None` when it is not yet generated — so the crate builds before
/// `stdlib_gen` has written the bundle (the R6 fallback chain then applies), and
/// picks the bundle up on the next build once it is baked.
fn emit_prelude_bundle_presence() {
    // Declare the custom cfg so `--check-cfg` (warn-by-default on recent
    // toolchains) does not flag it.
    println!("cargo:rustc-check-cfg=cfg(prelude_bundle_present)");

    let path = Path::new("embedded/core_prelude.shapec");
    if path.exists() {
        println!("cargo:rustc-cfg=prelude_bundle_present");
    }
    println!("cargo:rerun-if-changed=embedded/core_prelude.shapec");
}

/// Emit `SHAPE_COMPILER_FINGERPRINT` (compile-cache DESIGN §2.2 AMENDMENT B /
/// CLOSURE B). The cache key folds in a build content-id that changes on every
/// *meaningful* compiler rebuild, NOT the coarse `CARGO_PKG_VERSION` semver
/// (which stays `"0.3.3"` across every dev rebuild of the checker → stale-cache
/// silent-wrong during exactly the checker churn this cache must survive).
///
/// Shape of the fingerprint:
/// - `<short-sha>`               — clean checkout at a committed compiler.
/// - `<short-sha>-dirty-<epoch>` — uncommitted working tree; the build timestamp
///   forces a distinct id on every dirty rebuild (the inference engine can change
///   without any commit, so the sha alone would alias).
/// - `<CARGO_PKG_VERSION>`       — fallback when git is unavailable (e.g. a
///   release tarball with no `.git`). Semver alone is acceptable ONLY for
///   immutable published releases; the fingerprint covers both cases.
fn emit_compiler_fingerprint() {
    let fingerprint = compute_compiler_fingerprint();
    println!("cargo:rustc-env=SHAPE_COMPILER_FINGERPRINT={}", fingerprint);

    // Re-run when HEAD moves or the index changes so the fingerprint stays in
    // sync with the committed/dirty state. (Best-effort: paths may be absent in
    // a non-git build, in which case Cargo simply re-runs whenever build.rs's
    // own inputs change.)
    if let Some(git_dir) = locate_git_dir() {
        println!("cargo:rerun-if-changed={}/HEAD", git_dir.display());
        println!("cargo:rerun-if-changed={}/index", git_dir.display());
    }
    // A dirty tree mints a timestamped id every build, so always re-run.
    println!("cargo:rerun-if-env-changed=SHAPE_COMPILER_FINGERPRINT");
}

fn compute_compiler_fingerprint() -> String {
    let Some(short_sha) = git_output(&["rev-parse", "--short", "HEAD"]) else {
        // No git available — fall back to the crate semver.
        return env_semver();
    };

    let dirty = git_output(&["status", "--porcelain"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if dirty {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("{}-dirty-{}", short_sha, epoch)
    } else {
        short_sha
    }
}

fn env_semver() -> String {
    std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".to_string())
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn locate_git_dir() -> Option<std::path::PathBuf> {
    let dir = git_output(&["rev-parse", "--absolute-git-dir"])?;
    Some(std::path::PathBuf::from(dir))
}

fn generate_grammar_features() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("grammar_features.rs");

    // Path to the pest grammar file (relative to shape-vm)
    let pest_path = Path::new("../shape-ast/src/shape.pest");

    let rules = if pest_path.exists() {
        extract_pest_rules(pest_path)
    } else {
        // Fallback if file not found (shouldn't happen in normal builds)
        eprintln!("Warning: pest grammar not found at {:?}", pest_path);
        BTreeSet::new()
    };

    // Generate Rust code
    let rules_array: String = rules
        .iter()
        .map(|r| format!("    \"{}\",", r))
        .collect::<Vec<_>>()
        .join("\n");

    let generated = format!(
        r#"// Auto-generated from shape.pest - DO NOT EDIT
// This file contains all grammar rule names extracted from the pest grammar.
// Generated by shape-vm/build.rs

/// All grammar rules extracted from shape.pest
pub const PEST_RULES: &[&str] = &[
{}
];
"#,
        rules_array
    );

    fs::write(&dest_path, generated).expect("Failed to write grammar_features.rs");

    // Tell Cargo to re-run if the pest file changes
    println!("cargo:rerun-if-changed=../shape-ast/src/shape.pest");
}

/// Extract rule names from a pest grammar file
///
/// Pest rules have the format:
/// - `rule_name = { ... }`
/// - `rule_name = _{ ... }` (silent)
/// - `rule_name = @{ ... }` (atomic)
/// - `rule_name = ${ ... }` (compound atomic)
/// - `rule_name = !{ ... }` (non-atomic)
fn extract_pest_rules(path: &Path) -> BTreeSet<String> {
    let content = fs::read_to_string(path).expect("Failed to read pest grammar");
    let mut rules = BTreeSet::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        // Match rule definitions: `rule_name = ...{`
        // Rule names are identifiers followed by `=` and optional modifier before `{`
        if let Some(eq_pos) = line.find('=') {
            let before_eq = line[..eq_pos].trim();

            // Rule name should be a valid identifier (alphanumeric + underscore)
            if is_valid_rule_name(before_eq) {
                // Check that what follows looks like a rule body
                let after_eq = line[eq_pos + 1..].trim();
                if after_eq.starts_with('{')
                    || after_eq.starts_with("_{")
                    || after_eq.starts_with("@{")
                    || after_eq.starts_with("${")
                    || after_eq.starts_with("!{")
                {
                    rules.insert(before_eq.to_string());
                }
            }
        }
    }

    rules
}

/// Check if a string is a valid pest rule name
fn is_valid_rule_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
