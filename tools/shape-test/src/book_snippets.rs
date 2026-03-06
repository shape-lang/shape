//! Book snippet collector — walks a directory of .shape files for doctest-style testing.
//!
//! Each `.shape` file can have a companion `.expected` file containing the expected
//! stdout output. The test runner asserts the captured output matches. The same
//! `.expected` file is `{{#include}}`'d into the book markdown — single source of truth.

use std::path::{Path, PathBuf};

/// A single book snippet extracted from a .shape file.
pub struct BookSnippet {
    /// Path to the .shape file.
    pub file: PathBuf,
    /// Full file contents (ANCHOR comments are just comments — no effect on execution).
    pub code: String,
    /// Filename stem (e.g. "hello", "mean_example").
    pub name: String,
    /// Expected stdout output, loaded from companion `.expected` file if present.
    pub expected_output: Option<String>,
}

/// Collect all `.shape` files from the given directory (recursively).
/// For each `.shape` file, also loads a companion `.expected` file if present.
pub fn collect_book_snippets(snippets_dir: &Path) -> Vec<BookSnippet> {
    let mut snippets = Vec::new();
    if !snippets_dir.is_dir() {
        return snippets;
    }
    for entry in walkdir::WalkDir::new(snippets_dir)
        .min_depth(1)
        .sort_by_file_name()
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "shape") {
            if let Ok(code) = std::fs::read_to_string(path) {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                // Look for companion .expected file
                let expected_path = path.with_extension("expected");
                let expected_output = std::fs::read_to_string(&expected_path).ok();

                snippets.push(BookSnippet {
                    file: path.to_path_buf(),
                    code,
                    name,
                    expected_output,
                });
            }
        }
    }
    snippets
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn collect_from_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let snippets = collect_book_snippets(dir.path());
        assert!(snippets.is_empty());
    }

    #[test]
    fn collect_finds_shape_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.shape"), "print(\"hello\")\n").unwrap();
        fs::write(dir.path().join("math.shape"), "let x = 1 + 2\n").unwrap();
        fs::write(dir.path().join("readme.md"), "# not a shape file\n").unwrap();

        let snippets = collect_book_snippets(dir.path());
        assert_eq!(snippets.len(), 2);
        assert!(snippets.iter().any(|s| s.name == "hello"));
        assert!(snippets.iter().any(|s| s.name == "math"));
    }

    #[test]
    fn collect_recurses_into_chapter_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let ch1 = dir.path().join("getting-started");
        let ch2 = dir.path().join("fundamentals");
        fs::create_dir_all(&ch1).unwrap();
        fs::create_dir_all(&ch2).unwrap();
        fs::write(ch1.join("hello.shape"), "print(\"hello\")\n").unwrap();
        fs::write(ch2.join("ops.shape"), "1 + 2\n").unwrap();
        fs::write(ch2.join("vars.shape"), "let x = 1\n").unwrap();

        let snippets = collect_book_snippets(dir.path());
        assert_eq!(snippets.len(), 3);
        assert!(snippets.iter().any(|s| s.name == "hello"));
        assert!(snippets.iter().any(|s| s.name == "ops"));
        assert!(snippets.iter().any(|s| s.name == "vars"));
    }

    #[test]
    fn collect_loads_expected_output() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.shape"), "print(\"hello\")\n").unwrap();
        fs::write(dir.path().join("hello.expected"), "hello\n").unwrap();
        fs::write(dir.path().join("math.shape"), "let x = 1 + 2\n").unwrap();
        // math.shape has no .expected file

        let snippets = collect_book_snippets(dir.path());
        assert_eq!(snippets.len(), 2);

        let hello = snippets.iter().find(|s| s.name == "hello").unwrap();
        assert_eq!(hello.expected_output.as_deref(), Some("hello\n"));

        let math = snippets.iter().find(|s| s.name == "math").unwrap();
        assert!(math.expected_output.is_none());
    }

    #[test]
    fn collect_from_nonexistent_dir() {
        let snippets = collect_book_snippets(Path::new("/nonexistent/path"));
        assert!(snippets.is_empty());
    }
}
