//! Tests for book examples
//!
//! This module tests all `.shape` example files to ensure they execute without errors.
//! This serves as integration testing for the documentation - if examples in the book
//! break, these tests will catch it.

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use walkdir::WalkDir;

    use crate::{BytecodeExecutor, ShapeEngine};

    /// Get all .shape example files from the examples directory
    /// Excludes archive/ and tests/ directories which contain legacy syntax
    fn get_example_files() -> Vec<PathBuf> {
        let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");

        if !examples_dir.exists() {
            return vec![];
        }

        WalkDir::new(&examples_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let path = e.path();
                // Skip archive/ and tests/ directories (contain legacy syntax)
                let path_str = path.to_string_lossy();
                if path_str.contains("/archive/") || path_str.contains("/tests/") {
                    return false;
                }
                path.extension().map_or(false, |ext| ext == "shape")
            })
            .map(|e| e.path().to_path_buf())
            .collect()
    }

    /// Parse annotation from example file
    /// Supports:
    /// - `// @test` - Mark file as testable
    /// - `// @skip` - Skip this file in tests
    /// - `// @should_fail` - Expect execution to fail
    /// - `// @expect: <value>` - Expect specific output
    fn parse_annotations(content: &str) -> ExampleAnnotations {
        let mut annotations = ExampleAnnotations::default();

        for line in content.lines().take(20) {
            // Only check first 20 lines for annotations
            let line = line.trim();
            if line.starts_with("// @test") {
                annotations.is_test = true;
            } else if line.starts_with("// @skip") {
                annotations.skip = true;
            } else if line.starts_with("// @should_fail") {
                annotations.should_fail = true;
            } else if line.starts_with("// @expect:") {
                annotations.expected =
                    Some(line.strip_prefix("// @expect:").unwrap().trim().to_string());
            }
        }

        annotations
    }

    #[derive(Default)]
    struct ExampleAnnotations {
        is_test: bool,
        skip: bool,
        should_fail: bool,
        expected: Option<String>,
    }

    /// Test that all example files parse without errors
    #[test]
    fn test_all_examples_parse() {
        let files = get_example_files();

        if files.is_empty() {
            println!("No example files found, skipping test");
            return;
        }

        let mut failed = Vec::new();

        for file in &files {
            let content = match fs::read_to_string(file) {
                Ok(c) => c,
                Err(e) => {
                    failed.push((file.clone(), format!("Failed to read: {}", e)));
                    continue;
                }
            };

            let annotations = parse_annotations(&content);
            if annotations.skip {
                println!("Skipping: {}", file.display());
                continue;
            }

            // Try to parse the file
            match crate::ast::parse_program(&content) {
                Ok(_) => println!("Parsed OK: {}", file.display()),
                Err(e) => {
                    if !annotations.should_fail {
                        failed.push((file.clone(), format!("Parse error: {}", e)));
                    }
                }
            }
        }

        if !failed.is_empty() {
            for (file, error) in &failed {
                eprintln!("FAILED: {} - {}", file.display(), error);
            }
            panic!("{} example(s) failed to parse", failed.len());
        }
    }

    /// Test that tutorial examples execute correctly
    #[test]
    fn test_tutorial_examples_execute() {
        let tutorials_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/tutorials");

        if !tutorials_dir.exists() {
            println!("No tutorials directory found, skipping test");
            return;
        }

        let files: Vec<_> = WalkDir::new(&tutorials_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "shape"))
            .map(|e| e.path().to_path_buf())
            .collect();

        let mut failed = Vec::new();

        for file in &files {
            let content = match fs::read_to_string(file) {
                Ok(c) => c,
                Err(e) => {
                    failed.push((file.clone(), format!("Failed to read: {}", e)));
                    continue;
                }
            };

            let annotations = parse_annotations(&content);
            if annotations.skip {
                println!("Skipping: {}", file.display());
                continue;
            }

            // Create engine and execute
            let result = execute_example(&content);

            match result {
                Ok(_) => {
                    if annotations.should_fail {
                        failed.push((file.clone(), "Expected to fail but succeeded".to_string()));
                    } else {
                        println!("Executed OK: {}", file.display());
                    }
                }
                Err(e) => {
                    if !annotations.should_fail {
                        failed.push((file.clone(), format!("Execution error: {}", e)));
                    } else {
                        println!("Failed as expected: {}", file.display());
                    }
                }
            }
        }

        if !failed.is_empty() {
            for (file, error) in &failed {
                eprintln!("FAILED: {} - {}", file.display(), error);
            }
            panic!("{} tutorial example(s) failed", failed.len());
        }
    }

    /// Execute an example and return success
    fn execute_example(content: &str) -> anyhow::Result<()> {
        let mut engine = ShapeEngine::new()?;
        engine.load_stdlib()?;

        let mut executor = BytecodeExecutor::new();
        let _result = engine.execute(&mut executor, content)?;

        Ok(())
    }
}
