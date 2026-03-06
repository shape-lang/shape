//! Build script for shape-core
//!
//! - Extracts grammar rule names from shape.pest for feature tracking

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() {
    // Extract grammar features from pest file
    extract_grammar_features();
}

fn extract_grammar_features() {
    // Re-run if grammar changes
    println!("cargo:rerun-if-changed=src/shape.pest");

    let pest_path = Path::new("src/shape.pest");
    if !pest_path.exists() {
        return;
    }

    let pest_content = match fs::read_to_string(pest_path) {
        Ok(content) => content,
        Err(_) => return,
    };

    // Extract all rule names from the pest file
    let rules = extract_pest_rules(&pest_content);

    // Generate the output file
    let out_dir = env::var("OUT_DIR").unwrap_or_else(|_| ".".to_string());
    let out_path = Path::new(&out_dir).join("grammar_features.rs");

    let mut output = match fs::File::create(&out_path) {
        Ok(f) => f,
        Err(_) => return,
    };

    writeln!(output, "// Auto-generated from shape.pest - DO NOT EDIT").unwrap();
    writeln!(output, "/// All grammar rules extracted from shape.pest").unwrap();
    writeln!(output, "pub const PEST_RULES: &[&str] = &[").unwrap();

    for rule in &rules {
        writeln!(output, "    \"{}\",", rule).unwrap();
    }

    writeln!(output, "];").unwrap();
    writeln!(
        output,
        "pub const PEST_RULE_COUNT: usize = {};",
        rules.len()
    )
    .unwrap();
}

/// Extract rule names from pest grammar
fn extract_pest_rules(content: &str) -> BTreeSet<String> {
    let mut rules = BTreeSet::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.starts_with("//") || line.is_empty() {
            continue;
        }

        // Match rule definitions: rule_name = { ... }
        if let Some(eq_pos) = line.find('=') {
            let rule_name = line[..eq_pos].trim();

            if is_valid_rule_name(rule_name) && rule_name != "WHITESPACE" && rule_name != "COMMENT"
            {
                rules.insert(rule_name.to_string());
            }
        }
    }

    rules
}

fn is_valid_rule_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
}
