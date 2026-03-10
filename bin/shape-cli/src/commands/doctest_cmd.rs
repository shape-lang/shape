use anyhow::{Context, Result, bail};
use shape_runtime::engine::ShapeEngine;
use shape_runtime::output_adapter::SharedCaptureAdapter;
use shape_vm::BytecodeExecutor;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Run doctests on markdown files
pub async fn run_doctest(path: PathBuf, verbose: bool) -> Result<()> {
    run_doctests(&path, verbose).await
}

// =============================================================================
// Doctest Infrastructure
// =============================================================================

/// Represents a code block extracted from markdown
#[derive(Debug)]
struct DocTest {
    file: PathBuf,
    line: usize,
    language: String,
    code: String,
    should_fail: bool,
    ignore: bool,
    /// Expected output lines parsed from `// Output:` comments
    expected_output: Option<String>,
}

/// Extract code blocks from a markdown file
fn extract_code_blocks(path: &Path, content: &str) -> Vec<DocTest> {
    let mut tests = Vec::new();
    let mut in_code_block = false;
    let mut current_lang = String::new();
    let mut current_code = String::new();
    let mut block_start_line = 0;
    let mut should_fail = false;
    let mut ignore = false;

    for (line_num, line) in content.lines().enumerate() {
        if line.starts_with("```") {
            if in_code_block {
                // End of code block
                if !current_code.is_empty() {
                    let (code, expected_output) = extract_expected_output(&current_code);
                    tests.push(DocTest {
                        file: path.to_path_buf(),
                        line: block_start_line + 1, // 1-indexed
                        language: current_lang.clone(),
                        code,
                        should_fail,
                        ignore,
                        expected_output,
                    });
                }
                in_code_block = false;
                current_code.clear();
                should_fail = false;
                ignore = false;
            } else {
                // Start of code block
                in_code_block = true;
                block_start_line = line_num;
                let lang_spec = line.trim_start_matches('`').trim();

                // Parse language and modifiers: shape,should_fail or shape,ignore
                let parts: Vec<&str> = lang_spec.split(',').collect();
                current_lang = parts.first().unwrap_or(&"").to_string();
                should_fail = parts.iter().any(|p| p.trim() == "should_fail");
                ignore = parts
                    .iter()
                    .any(|p| p.trim() == "ignore" || p.trim() == "no_run");
            }
        } else if in_code_block {
            if !current_code.is_empty() {
                current_code.push('\n');
            }
            current_code.push_str(line);
        }
    }

    tests
}

/// Extract expected output from `// Output:` comments in doctest code.
///
/// Lines starting with `// Output:` mark expected output. The text after
/// `// Output:` is the expected line. Multiple consecutive `// Output:` lines
/// are joined with newlines. The `// Output:` comments are stripped from
/// the returned code so they don't affect execution.
///
/// Example:
/// ```text
/// print("hello")
/// // Output: hello
/// print("world")
/// // Output: world
/// ```
/// Returns ("print(\"hello\")\nprint(\"world\")", Some("hello\nworld"))
fn extract_expected_output(code: &str) -> (String, Option<String>) {
    let mut code_lines = Vec::new();
    let mut output_lines = Vec::new();

    for line in code.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("// Output:") {
            output_lines.push(rest.trim_start_matches(' ').to_string());
        } else {
            code_lines.push(line.to_string());
        }
    }

    let expected = if output_lines.is_empty() {
        None
    } else {
        Some(output_lines.join("\n"))
    };

    (code_lines.join("\n"), expected)
}

/// Run doctests on markdown files
async fn run_doctests(path: &Path, verbose: bool) -> Result<()> {
    let mut files = Vec::new();

    if path.is_dir() {
        // Recursively find all markdown files
        collect_markdown_files(path, &mut files).await?;
    } else if path.extension().is_some_and(|e| e == "md") {
        files.push(path.to_path_buf());
    } else {
        bail!("path must be a markdown file or directory");
    }

    if files.is_empty() {
        println!("No markdown files found");
        return Ok(());
    }

    println!("Running doctests on {} markdown files...\n", files.len());

    let mut total_tests = 0;
    let mut passed = 0;
    let mut failed = 0;
    let mut ignored = 0;
    let mut failures: Vec<(DocTest, String)> = Vec::new();

    // Create engine for testing - no pre-loaded data, doctests must use data() with extensions
    let mut engine = ShapeEngine::new().context("failed to create Shape engine")?;
    engine.load_stdlib().context("failed to load stdlib")?;

    for file in &files {
        let content = fs::read_to_string(file)
            .await
            .with_context(|| format!("failed to read {}", file.display()))?;

        let tests = extract_code_blocks(file, &content);
        let shape_tests: Vec<_> = tests
            .into_iter()
            .filter(|t| t.language == "shape" || t.language == "cql")
            .collect();

        if shape_tests.is_empty() {
            continue;
        }

        if verbose {
            println!("Testing: {}", file.display());
        }

        for test in shape_tests {
            total_tests += 1;

            if test.ignore {
                ignored += 1;
                if verbose {
                    println!("  [IGNORED] line {}", test.line);
                }
                continue;
            }

            // Reset engine state for each test - doctests must use data() with extensions
            let mut test_engine = ShapeEngine::new()?;
            test_engine.load_stdlib()?;

            // Set up output capture adapter for output validation
            let capture_adapter = SharedCaptureAdapter::new();
            if let Some(ctx) = test_engine.get_runtime_mut().persistent_context_mut() {
                ctx.set_output_adapter(Box::new(capture_adapter.clone()));
            }

            let result = {
                let mut executor = BytecodeExecutor::new();
                let context_file = test
                    .file
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("__shape_doctest__.shape");
                crate::module_loading::wire_vm_executor_module_loading(
                    &mut test_engine,
                    &mut executor,
                    Some(&context_file),
                    Some(&test.code),
                )?;
                test_engine.execute(&mut executor, &test.code)
            };

            let test_passed = match (&result, test.should_fail) {
                (Ok(_), false) => {
                    // Check output validation if expected output is specified
                    if let Some(ref expected) = test.expected_output {
                        let actual_lines = capture_adapter.output();
                        let actual = actual_lines.join("\n");
                        if actual.trim() == expected.trim() {
                            true
                        } else {
                            false
                        }
                    } else {
                        true
                    }
                }
                (Err(_), true) => true,   // Expected failure, got failure
                (Ok(_), true) => false,   // Expected failure, got success
                (Err(_), false) => false, // Expected success, got failure
            };

            if test_passed {
                passed += 1;
                if verbose {
                    println!("  [PASS] line {}", test.line);
                }
            } else {
                failed += 1;
                let error_msg = match &result {
                    Ok(_) => {
                        if test.should_fail {
                            "expected failure but test passed".to_string()
                        } else if let Some(ref expected) = test.expected_output {
                            let actual_lines = capture_adapter.output();
                            let actual = actual_lines.join("\n");
                            format!(
                                "output mismatch:\n  expected: {}\n  actual:   {}",
                                expected.trim(),
                                actual.trim()
                            )
                        } else {
                            "unexpected error".to_string()
                        }
                    }
                    Err(e) => e.to_string(),
                };
                failures.push((test, error_msg));
            }
        }
    }

    // Print summary
    println!("\n{}", "=".repeat(60));
    println!("Doctest Results");
    println!("{}", "=".repeat(60));
    println!("Total tests: {}", total_tests);
    println!("  Passed:  {} (green)", passed);
    println!("  Failed:  {} (red)", failed);
    println!("  Ignored: {}", ignored);

    if !failures.is_empty() {
        println!("\n{}", "=".repeat(60));
        println!("Failures:");
        println!("{}", "=".repeat(60));

        for (test, error) in &failures {
            println!("\n{}:{}", test.file.display(), test.line);
            println!("Code:\n{}", indent_code(&test.code, "  "));
            println!("Error: {}", error);
        }

        bail!("{} doctest(s) failed", failed);
    }

    println!("\nAll doctests passed!");
    Ok(())
}

fn indent_code(code: &str, prefix: &str) -> String {
    code.lines()
        .map(|line| format!("{}{}", prefix, line))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn collect_markdown_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories and node_modules
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if !name.starts_with('.') && name != "node_modules" {
                    Box::pin(collect_markdown_files(&path, files)).await?;
                }
            }
        } else if path.extension().is_some_and(|e| e == "md") {
            files.push(path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_extract_expected_output_no_output_comments() {
        let code = "let x = 1\nlet y = 2";
        let (cleaned, expected) = extract_expected_output(code);
        assert_eq!(cleaned, "let x = 1\nlet y = 2");
        assert!(expected.is_none());
    }

    #[test]
    fn test_extract_expected_output_single_line() {
        let code = "print(\"hello\")\n// Output: hello";
        let (cleaned, expected) = extract_expected_output(code);
        assert_eq!(cleaned, "print(\"hello\")");
        assert_eq!(expected.as_deref(), Some("hello"));
    }

    #[test]
    fn test_extract_expected_output_multiple_lines() {
        let code = "print(\"hello\")\n// Output: hello\nprint(\"world\")\n// Output: world";
        let (cleaned, expected) = extract_expected_output(code);
        assert_eq!(cleaned, "print(\"hello\")\nprint(\"world\")");
        assert_eq!(expected.as_deref(), Some("hello\nworld"));
    }

    #[test]
    fn test_extract_expected_output_preserves_empty_output() {
        let code = "print(\"\")\n// Output: ";
        let (cleaned, expected) = extract_expected_output(code);
        assert_eq!(cleaned, "print(\"\")");
        assert_eq!(expected.as_deref(), Some(""));
    }

    #[test]
    fn test_extract_expected_output_ignores_regular_comments() {
        let code = "// This is a comment\nlet x = 1\n// Output: 1";
        let (cleaned, expected) = extract_expected_output(code);
        assert_eq!(cleaned, "// This is a comment\nlet x = 1");
        assert_eq!(expected.as_deref(), Some("1"));
    }

    #[test]
    fn test_extract_code_blocks_with_expected_output() {
        let md = r#"# Test

```shape
print("hello")
// Output: hello
```
"#;
        let tests = extract_code_blocks(Path::new("test.md"), md);
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].code, "print(\"hello\")");
        assert_eq!(tests[0].expected_output.as_deref(), Some("hello"));
    }

    #[test]
    fn test_extract_code_blocks_without_output() {
        let md = r#"# Test

```shape
let x = 1
```
"#;
        let tests = extract_code_blocks(Path::new("test.md"), md);
        assert_eq!(tests.len(), 1);
        assert_eq!(tests[0].code, "let x = 1");
        assert!(tests[0].expected_output.is_none());
    }

    #[test]
    fn test_extract_code_blocks_should_fail() {
        let md = r#"# Test

```shape,should_fail
undefined_variable
```
"#;
        let tests = extract_code_blocks(Path::new("test.md"), md);
        assert_eq!(tests.len(), 1);
        assert!(tests[0].should_fail);
    }

    #[test]
    fn test_extract_code_blocks_ignore() {
        let md = r#"# Test

```shape,ignore
// not run
```
"#;
        let tests = extract_code_blocks(Path::new("test.md"), md);
        assert_eq!(tests.len(), 1);
        assert!(tests[0].ignore);
    }

    #[test]
    fn test_extract_code_blocks_non_shape_filtered() {
        let md = r#"# Test

```javascript
console.log("hi")
```

```shape
let x = 1
```
"#;
        let tests = extract_code_blocks(Path::new("test.md"), md);
        // Both are extracted, filtering happens later
        assert_eq!(tests.len(), 2);
        assert_eq!(tests[0].language, "javascript");
        assert_eq!(tests[1].language, "shape");
    }
}
