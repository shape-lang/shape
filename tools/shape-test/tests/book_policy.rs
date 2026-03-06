use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

fn book_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("../docs/book")
}

fn book_src_dir() -> PathBuf {
    book_root().join("src")
}

fn book_snippets_dir() -> PathBuf {
    book_root().join("snippets")
}

fn markdown_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
        .map(|entry| entry.path().to_path_buf())
        .collect();
    files.sort();
    files
}

fn shape_snippet_files(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("shape"))
        .map(|entry| entry.path().to_path_buf())
        .collect();
    files.sort();
    files
}

fn extract_markdown_links(line: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = line;

    while let Some(start) = rest.find("](") {
        let after = &rest[start + 2..];
        if let Some(end) = after.find(')') {
            links.push(after[..end].trim().to_string());
            rest = &after[end + 1..];
        } else {
            break;
        }
    }

    links
}

fn extract_include_targets(line: &str) -> Vec<String> {
    let mut includes = Vec::new();
    let mut rest = line;
    const MARKER: &str = "{{#include ";

    while let Some(start) = rest.find(MARKER) {
        let after = &rest[start + MARKER.len()..];
        if let Some(end) = after.find("}}") {
            includes.push(after[..end].trim().to_string());
            rest = &after[end + 2..];
        } else {
            break;
        }
    }

    includes
}

fn resolve_include_path_and_selector(
    base_dir: &Path,
    include: &str,
) -> Option<(PathBuf, Option<String>)> {
    let mut raw_path = include.to_string();
    let mut selector_parts: Vec<String> = Vec::new();

    loop {
        let candidate = base_dir.join(&raw_path);
        if candidate.exists() {
            let selector = if selector_parts.is_empty() {
                None
            } else {
                selector_parts.reverse();
                Some(selector_parts.join(":"))
            };
            return Some((candidate, selector));
        }

        let Some((left, right)) = raw_path.rsplit_once(':') else {
            return None;
        };
        selector_parts.push(right.to_string());
        raw_path = left.to_string();
    }
}

fn selector_is_line_range(selector: &str) -> bool {
    selector
        .split(':')
        .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

fn include_anchor_exists(path: &Path, anchor: &str) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let marker = format!("ANCHOR: {}", anchor);
    text.lines().any(|line| line.contains(&marker))
}

fn is_remote_or_anchor(target: &str) -> bool {
    target.starts_with('#')
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
}

fn normalize_link_target(target: &str) -> &str {
    target
        .split('#')
        .next()
        .unwrap_or(target)
        .split('?')
        .next()
        .unwrap_or(target)
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn token_positions(line: &str, token: &str) -> Vec<usize> {
    let bytes = line.as_bytes();
    let token_bytes = token.as_bytes();
    let mut out = Vec::new();

    if token_bytes.is_empty() || bytes.len() < token_bytes.len() {
        return out;
    }

    let mut idx = 0;
    while idx + token_bytes.len() <= bytes.len() {
        if &bytes[idx..idx + token_bytes.len()] == token_bytes {
            let left_ok = idx == 0 || !is_ident_byte(bytes[idx - 1]);
            let right_idx = idx + token_bytes.len();
            let right_ok = right_idx == bytes.len() || !is_ident_byte(bytes[right_idx]);
            if left_ok && right_ok {
                out.push(idx);
            }
            idx += token_bytes.len();
        } else {
            idx += 1;
        }
    }

    out
}

fn sanitize_shape_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_single {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                in_single = false;
            }
            out.push(' ');
            continue;
        }

        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
            out.push(' ');
            continue;
        }

        if ch == '/' && chars.peek() == Some(&'/') {
            break;
        }

        if ch == '\'' {
            in_single = true;
            out.push(' ');
            continue;
        }

        if ch == '"' {
            in_double = true;
            out.push(' ');
            continue;
        }

        out.push(ch);
    }

    out
}

fn has_series_call(line: &str) -> bool {
    let bytes = line.as_bytes();
    for start in token_positions(line, "series") {
        let mut idx = start + "series".len();
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx < bytes.len() && bytes[idx] == b'(' {
            return true;
        }
    }
    false
}

fn has_null_token(line: &str) -> bool {
    !token_positions(line, "null").is_empty()
}

fn has_legacy_function_syntax(line: &str) -> bool {
    let bytes = line.as_bytes();
    for start in token_positions(line, "function") {
        let mut idx = start + "function".len();
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx < bytes.len() {
            let byte = bytes[idx];
            if byte == b'(' || byte == b'_' || (byte as char).is_ascii_alphabetic() {
                return true;
            }
        }
    }
    false
}

fn has_bare_load_call(line: &str) -> bool {
    let bytes = line.as_bytes();
    for start in token_positions(line, "load") {
        if start > 0 {
            let prev = bytes[start - 1];
            if prev == b'.' || prev == b':' {
                continue;
            }
        }

        let mut idx = start + "load".len();
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx < bytes.len() && bytes[idx] == b'(' {
            return true;
        }
    }
    false
}

fn is_shape_fence(info: &str) -> bool {
    let info = info.trim();
    if info.is_empty() {
        return false;
    }

    let lang = info
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    lang == "shape" || lang == "shape-repl"
}

#[test]
fn book_summary_links_resolve() {
    let summary = book_src_dir().join("SUMMARY.md");
    let text = fs::read_to_string(&summary).expect("failed to read SUMMARY.md");
    let mut errors = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        for raw_target in extract_markdown_links(line) {
            if is_remote_or_anchor(&raw_target) {
                continue;
            }
            let target = normalize_link_target(&raw_target);
            if !target.ends_with(".md") {
                continue;
            }
            let resolved = summary.parent().unwrap().join(target);
            if !resolved.exists() {
                errors.push(format!(
                    "{}:{} -> missing {}",
                    summary.display(),
                    line_no + 1,
                    target
                ));
            }
        }
    }

    assert!(
        errors.is_empty(),
        "Broken links in SUMMARY.md:\n{}",
        errors.join("\n")
    );
}

#[test]
fn book_md_links_and_includes_resolve() {
    let mut errors = Vec::new();

    for file in markdown_files(&book_src_dir()) {
        let text = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));

        for (line_no, line) in text.lines().enumerate() {
            for raw_target in extract_markdown_links(line) {
                if is_remote_or_anchor(&raw_target) {
                    continue;
                }
                let target = normalize_link_target(&raw_target);
                if !target.ends_with(".md") {
                    continue;
                }
                let resolved = file.parent().unwrap().join(target);
                if !resolved.exists() {
                    errors.push(format!(
                        "{}:{} -> missing link target {}",
                        file.display(),
                        line_no + 1,
                        target
                    ));
                }
            }

            for include in extract_include_targets(line) {
                if is_remote_or_anchor(&include) {
                    continue;
                }
                let Some((resolved, selector)) =
                    resolve_include_path_and_selector(file.parent().unwrap(), &include)
                else {
                    errors.push(format!(
                        "{}:{} -> missing include target {}",
                        file.display(),
                        line_no + 1,
                        include
                    ));
                    continue;
                };

                if let Some(selector) = selector {
                    if !selector_is_line_range(&selector)
                        && !include_anchor_exists(&resolved, &selector)
                    {
                        errors.push(format!(
                            "{}:{} -> missing include anchor '{}' in {}",
                            file.display(),
                            line_no + 1,
                            selector,
                            resolved.display()
                        ));
                    }
                }
            }
        }
    }

    assert!(
        errors.is_empty(),
        "Broken markdown links/includes in book source:\n{}",
        errors.join("\n")
    );
}

#[test]
fn book_shape_examples_use_current_syntax() {
    let mut errors = Vec::new();

    for file in shape_snippet_files(&book_snippets_dir()) {
        let text = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));

        for (line_no, raw_line) in text.lines().enumerate() {
            let line = sanitize_shape_line(raw_line);
            if has_series_call(&line) {
                errors.push(format!(
                    "{}:{} uses removed series(...) call",
                    file.display(),
                    line_no + 1
                ));
            }
            if has_legacy_function_syntax(&line) {
                errors.push(format!(
                    "{}:{} uses legacy function keyword syntax",
                    file.display(),
                    line_no + 1
                ));
            }
            if has_null_token(&line) {
                errors.push(format!(
                    "{}:{} uses null token (use Option/Result flow)",
                    file.display(),
                    line_no + 1
                ));
            }
            if has_bare_load_call(&line) {
                errors.push(format!(
                    "{}:{} uses removed global load(...) form",
                    file.display(),
                    line_no + 1
                ));
            }
        }
    }

    for file in markdown_files(&book_src_dir()) {
        let text = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));

        let mut in_fence = false;
        let mut shape_fence = false;

        for (line_no, raw_line) in text.lines().enumerate() {
            let trimmed = raw_line.trim_start();
            if trimmed.starts_with("```") {
                if !in_fence {
                    in_fence = true;
                    let info = trimmed.trim_start_matches('`').trim();
                    shape_fence = is_shape_fence(info);
                } else {
                    in_fence = false;
                    shape_fence = false;
                }
                continue;
            }

            if !(in_fence && shape_fence) {
                continue;
            }

            let line = sanitize_shape_line(raw_line);
            if has_series_call(&line) {
                errors.push(format!(
                    "{}:{} uses removed series(...) call",
                    file.display(),
                    line_no + 1
                ));
            }
            if has_legacy_function_syntax(&line) {
                errors.push(format!(
                    "{}:{} uses legacy function keyword syntax",
                    file.display(),
                    line_no + 1
                ));
            }
            if has_null_token(&line) {
                errors.push(format!(
                    "{}:{} uses null token (use Option/Result flow)",
                    file.display(),
                    line_no + 1
                ));
            }
            if has_bare_load_call(&line) {
                errors.push(format!(
                    "{}:{} uses removed global load(...) form",
                    file.display(),
                    line_no + 1
                ));
            }
        }
    }

    assert!(
        errors.is_empty(),
        "Book syntax policy violations:\n{}",
        errors.join("\n")
    );
}
