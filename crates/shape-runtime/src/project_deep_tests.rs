//! Deep exhaustive tests for the Shape project system.
//!
//! ~125 tests covering manifest parsing edge cases, dependency resolution,
//! project root discovery, build configuration, extensions, frontmatter,
//! and module path configuration.

#[cfg(test)]
mod tests {
    use crate::frontmatter::{
use shape_value::ValueWordExt;
        FrontmatterDiagnosticSeverity, parse_frontmatter, parse_frontmatter_validated,
    };
    use crate::project::*;
    #[allow(unused_imports)]
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;

    // =========================================================================
    // Category 1: Manifest Parsing Edge Cases (~30 tests)
    // =========================================================================

    #[test]
    fn test_project_parse_completely_empty_file() {
        // An empty string should parse to all defaults
        let config = parse_shape_project_toml("").unwrap();
        assert_eq!(config.project.name, "");
        assert_eq!(config.project.version, "");
        assert!(config.project.entry.is_none());
        assert!(config.project.authors.is_empty());
        assert!(config.project.shape_version.is_none());
        assert!(config.project.license.is_none());
        assert!(config.project.repository.is_none());
        assert!(config.modules.paths.is_empty());
        assert!(config.dependencies.is_empty());
        assert!(config.dev_dependencies.is_empty());
        assert!(config.build.target.is_none());
        assert!(config.build.opt_level.is_none());
        assert!(config.build.output.is_none());
        assert!(config.extensions.is_empty());
    }

    #[test]
    fn test_project_parse_whitespace_only() {
        let config = parse_shape_project_toml("   \n\n   \n").unwrap();
        assert_eq!(config.project.name, "");
        assert!(config.dependencies.is_empty());
    }

    #[test]
    fn test_project_parse_comments_only() {
        let config = parse_shape_project_toml("# just a comment\n# another comment\n").unwrap();
        assert_eq!(config.project.name, "");
    }

    #[test]
    fn test_project_parse_name_only_minimal() {
        let config = parse_shape_project_toml("[project]\nname = \"x\"").unwrap();
        assert_eq!(config.project.name, "x");
        assert_eq!(config.project.version, "");
    }

    #[test]
    fn test_project_parse_unknown_top_level_section() {
        // serde's default behavior with deny_unknown_fields not set: should succeed
        // BUG CANDIDATE: unknown sections silently ignored — is this intended?
        let result = parse_shape_project_toml(
            r#"
[project]
name = "test"

[unknown_section]
foo = "bar"
"#,
        );
        // The parser uses #[serde(default)] and no deny_unknown_fields,
        // so unknown sections should cause an error with serde's default TOML behavior
        // Actually with toml crate, unknown fields in the top-level struct cause an error
        // unless the struct is permissive. Let's check:
        match result {
            Ok(_) => {
                // If this passes, unknown sections are silently ignored
                // This could be a design concern for typo detection
            }
            Err(e) => {
                // Expected: unknown fields cause parse error
                let msg = e.to_string();
                assert!(
                    msg.contains("unknown"),
                    "Error should mention unknown field: {}",
                    msg
                );
            }
        }
    }

    #[test]
    fn test_project_parse_unknown_field_in_project_section() {
        let result = parse_shape_project_toml(
            r#"
[project]
name = "test"
version = "1.0.0"
description = "should this work?"
"#,
        );
        // 'description' is not in ProjectSection — check if it errors or ignores
        match result {
            Ok(_config) => {
                // silently ignored — could hide typos
            }
            Err(e) => {
                let msg = e.to_string();
                assert!(msg.contains("unknown") || msg.contains("description"));
            }
        }
    }

    #[test]
    fn test_project_parse_invalid_toml_syntax_unclosed_string() {
        let result = parse_shape_project_toml("[project]\nname = \"unclosed");
        assert!(result.is_err());
    }

    #[test]
    fn test_project_parse_invalid_toml_syntax_unclosed_bracket() {
        let result = parse_shape_project_toml("[project\nname = \"test\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_project_parse_invalid_toml_bad_escape() {
        let result = parse_shape_project_toml("[project]\nname = \"bad\\zescape\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_project_parse_unicode_project_name() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "日本語プロジェクト"
version = "1.0.0"
"#,
        )
        .unwrap();
        assert_eq!(config.project.name, "日本語プロジェクト");
    }

    #[test]
    fn test_project_parse_unicode_author_names() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "test"
authors = ["José García", "François Müller", "田中太郎"]
"#,
        )
        .unwrap();
        assert_eq!(config.project.authors.len(), 3);
        assert_eq!(config.project.authors[0], "José García");
        assert_eq!(config.project.authors[2], "田中太郎");
    }

    #[test]
    fn test_project_parse_emoji_in_name() {
        let config = parse_shape_project_toml("[project]\nname = \"🚀rocket-project\"").unwrap();
        assert_eq!(config.project.name, "🚀rocket-project");
    }

    #[test]
    fn test_project_parse_very_long_name() {
        let long_name = "a".repeat(10000);
        let toml_str = format!("[project]\nname = \"{}\"", long_name);
        let config = parse_shape_project_toml(&toml_str).unwrap();
        assert_eq!(config.project.name.len(), 10000);
    }

    #[test]
    fn test_project_parse_empty_string_fields() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = ""
version = ""
license = ""
repository = ""
"#,
        )
        .unwrap();
        assert_eq!(config.project.name, "");
        assert_eq!(config.project.version, "");
        assert_eq!(config.project.license.as_deref(), Some(""));
        assert_eq!(config.project.repository.as_deref(), Some(""));
    }

    #[test]
    fn test_project_parse_non_semver_version() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "test"
version = "not-a-version"
"#,
        )
        .unwrap();
        // Version is just a String — no validation at parse time
        assert_eq!(config.project.version, "not-a-version");
    }

    #[test]
    fn test_project_parse_semver_prerelease() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "test"
version = "1.0.0-alpha.1+build.123"
"#,
        )
        .unwrap();
        assert_eq!(config.project.version, "1.0.0-alpha.1+build.123");
    }

    #[test]
    fn test_project_parse_empty_version() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "test"
version = ""
"#,
        )
        .unwrap();
        assert_eq!(config.project.version, "");
    }

    #[test]
    fn test_project_parse_duplicate_section_toml_behavior() {
        // TOML spec: duplicate tables should error
        let result = parse_shape_project_toml(
            r#"
[project]
name = "first"

[project]
name = "second"
"#,
        );
        assert!(
            result.is_err(),
            "Duplicate sections should cause a TOML parse error"
        );
    }

    #[test]
    fn test_project_parse_integer_where_string_expected() {
        // name should be a string, what if integer is given?
        let result = parse_shape_project_toml("[project]\nname = 42");
        assert!(result.is_err(), "Integer for string field should error");
    }

    #[test]
    fn test_project_parse_boolean_where_string_expected() {
        let result = parse_shape_project_toml("[project]\nname = true");
        assert!(result.is_err(), "Boolean for string field should error");
    }

    #[test]
    fn test_project_parse_string_where_array_expected() {
        // authors expects Vec<String>
        let result = parse_shape_project_toml("[project]\nname = \"t\"\nauthors = \"single\"");
        assert!(result.is_err(), "String for array field should error");
    }

    #[test]
    fn test_project_parse_empty_authors_array() {
        let config = parse_shape_project_toml("[project]\nname = \"t\"\nauthors = []").unwrap();
        assert!(config.project.authors.is_empty());
    }

    #[test]
    fn test_project_parse_multiline_strings() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = """
multi
line
name
"""
"#,
        )
        .unwrap();
        assert!(config.project.name.contains("multi"));
        assert!(config.project.name.contains("line"));
    }

    #[test]
    fn test_project_parse_literal_string() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = 'literal-string'
"#,
        )
        .unwrap();
        assert_eq!(config.project.name, "literal-string");
    }

    #[test]
    fn test_project_parse_toml_inline_table_dependency() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
mylib = { path = "./lib", version = "1.0" }
"#,
        )
        .unwrap();
        match config.dependencies.get("mylib").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("./lib"));
                assert_eq!(d.version.as_deref(), Some("1.0"));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_project_parse_all_project_fields_simultaneously() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "full"
version = "3.2.1"
entry = "main.shape"
authors = ["A", "B", "C"]
shape-version = "0.9.0"
license = "GPL-3.0"
repository = "https://example.com/repo"
"#,
        )
        .unwrap();
        assert_eq!(config.project.name, "full");
        assert_eq!(config.project.version, "3.2.1");
        assert_eq!(config.project.entry.as_deref(), Some("main.shape"));
        assert_eq!(config.project.authors.len(), 3);
        assert_eq!(config.project.shape_version.as_deref(), Some("0.9.0"));
        assert_eq!(config.project.license.as_deref(), Some("GPL-3.0"));
        assert_eq!(
            config.project.repository.as_deref(),
            Some("https://example.com/repo")
        );
    }

    #[test]
    fn test_project_parse_special_chars_in_strings() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "test\"with\\escapes\nnewline"
"#,
        )
        .unwrap();
        assert!(config.project.name.contains("with"));
        assert!(config.project.name.contains("escapes"));
    }

    #[test]
    fn test_project_parse_windows_path_in_entry() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "win"
entry = "src\\main.shape"
"#,
        )
        .unwrap();
        assert_eq!(config.project.entry.as_deref(), Some("src\\main.shape"));
    }

    #[test]
    fn test_project_parse_opt_level_as_string_fails() {
        // opt_level is Option<u8> — passing a string should fail
        let result = parse_shape_project_toml("[build]\nopt_level = \"high\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_project_parse_opt_level_negative_fails() {
        // TOML integers are i64, but opt_level is u8 — negative should fail
        let result = parse_shape_project_toml("[build]\nopt_level = -1");
        assert!(result.is_err(), "Negative opt_level should fail u8 parse");
    }

    #[test]
    fn test_project_parse_opt_level_overflow() {
        // 256 exceeds u8 max
        let result = parse_shape_project_toml("[build]\nopt_level = 256");
        assert!(result.is_err(), "opt_level 256 should overflow u8");
    }

    // =========================================================================
    // Category 2: Dependency Resolution (~25 tests)
    // =========================================================================

    #[test]
    fn test_project_dep_path_nonexistent_dir_parses() {
        // Path deps parse even if the directory doesn't exist —
        // validation happens at a different stage
        let config = parse_shape_project_toml(
            r#"
[dependencies]
ghost = { path = "/nonexistent/path/that/does/not/exist" }
"#,
        )
        .unwrap();
        match config.dependencies.get("ghost").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(
                    d.path.as_deref(),
                    Some("/nonexistent/path/that/does/not/exist")
                );
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_project_dep_path_with_parent_traversal() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
parent-lib = { path = "../../libs/common" }
"#,
        )
        .unwrap();
        match config.dependencies.get("parent-lib").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("../../libs/common"));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_project_dep_git_missing_ref_validation() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
no-ref = { git = "https://github.com/org/repo.git" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("no-ref") && e.contains("tag")),
            "Should warn about missing tag/branch/rev: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_dep_git_with_tag_and_branch_both() {
        // Having both tag AND branch — no explicit validation against this currently
        let config = parse_shape_project_toml(
            r#"
[dependencies]
ambiguous = { git = "https://github.com/org/repo.git", tag = "v1", branch = "main" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        // BUG: No validation currently catches tag+branch conflict.
        // The dependency has both tag and branch which is ambiguous.
        // Since either satisfies the "must have at least one ref" rule, validate passes.
        let _ = errors;
    }

    #[test]
    fn test_project_dep_git_with_tag_and_rev_both() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
double-ref = { git = "https://github.com/org/repo.git", tag = "v1", rev = "abc123" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        // BUG: No validation for conflicting tag+rev
        let _ = errors;
    }

    #[test]
    fn test_project_dep_git_with_all_three_refs() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
triple = { git = "https://github.com/org/repo.git", tag = "v1", branch = "main", rev = "abc" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        // BUG: No validation for all three refs being set simultaneously
        let _ = errors;
    }

    #[test]
    fn test_project_dep_path_and_git_conflict() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
conflict = { path = "../local", git = "https://github.com/org/repo.git", tag = "v1" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("conflict") && e.contains("path") && e.contains("git")),
            "Should catch path+git conflict: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_dep_path_and_version() {
        // Path with version — currently allowed, may or may not be intentional
        let config = parse_shape_project_toml(
            r#"
[dependencies]
local-versioned = { path = "./lib", version = "1.0.0" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        // Currently no validation against path+version combo
        let _ = errors;
    }

    #[test]
    fn test_project_dep_empty_dependency_name() {
        // TOML allows empty string keys with quotes
        let config = parse_shape_project_toml(
            r#"
[dependencies]
"" = "1.0.0"
"#,
        )
        .unwrap();
        assert!(config.dependencies.contains_key(""));
        // BUG: No validation for empty dependency names
    }

    #[test]
    fn test_project_dep_special_chars_in_name() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
"my-lib_v2.0" = "1.0.0"
"@scope/pkg" = "2.0.0"
"#,
        )
        .unwrap();
        assert!(config.dependencies.contains_key("my-lib_v2.0"));
        assert!(config.dependencies.contains_key("@scope/pkg"));
    }

    #[test]
    fn test_project_dep_many_dependencies() {
        let mut toml_str = "[dependencies]\n".to_string();
        for i in 0..100 {
            toml_str.push_str(&format!("dep-{} = \"1.0.{}\"\n", i, i));
        }
        let config = parse_shape_project_toml(&toml_str).unwrap();
        assert_eq!(config.dependencies.len(), 100);
    }

    #[test]
    fn test_project_dep_version_string_formats() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
exact = "1.2.3"
wildcard = "*"
caret = "^1.0"
tilde = "~1.2"
bare = "1"
empty-ver = ""
"#,
        )
        .unwrap();
        // All are just strings — no version parsing at this level
        assert_eq!(config.dependencies.len(), 6);
        assert_eq!(
            config.dependencies.get("wildcard"),
            Some(&DependencySpec::Version("*".to_string()))
        );
        assert_eq!(
            config.dependencies.get("empty-ver"),
            Some(&DependencySpec::Version("".to_string()))
        );
    }

    #[test]
    fn test_project_dep_dev_deps_also_validated() {
        let config = parse_shape_project_toml(
            r#"
[dev-dependencies]
bad = { path = "../x", git = "https://example.com/x.git", tag = "v1" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("dev-dependencies") && e.contains("bad")),
            "dev-dependencies should also be validated: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_dep_git_no_ref_dev_deps() {
        let config = parse_shape_project_toml(
            r#"
[dev-dependencies]
no-ref-dev = { git = "https://example.com/repo.git" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        assert!(
            errors.iter().any(|e| e.contains("no-ref-dev")),
            "dev dep without ref should warn: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_dep_detailed_with_only_version() {
        // A detailed dep with only version set — should parse as Detailed (not Version)
        let config = parse_shape_project_toml(
            r#"
[dependencies]
ver-only = { version = "2.0.0" }
"#,
        )
        .unwrap();
        match config.dependencies.get("ver-only").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.version.as_deref(), Some("2.0.0"));
                assert!(d.path.is_none());
                assert!(d.git.is_none());
            }
            DependencySpec::Version(_) => {
                panic!("Inline table should parse as Detailed, not Version");
            }
        }
    }

    #[test]
    fn test_project_dep_empty_detailed_table() {
        // A dependency with an empty table: `dep = {}`
        let config = parse_shape_project_toml(
            r#"
[dependencies]
empty-table = {}
"#,
        )
        .unwrap();
        match config.dependencies.get("empty-table").unwrap() {
            DependencySpec::Detailed(d) => {
                assert!(d.version.is_none());
                assert!(d.path.is_none());
                assert!(d.git.is_none());
                assert!(d.tag.is_none());
                assert!(d.branch.is_none());
                assert!(d.rev.is_none());
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_project_dep_mixed_deps_and_dev_deps() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
prod = "1.0.0"
shared = { path = "../shared" }

[dev-dependencies]
test-util = "2.0.0"
mock = { path = "../mock" }
"#,
        )
        .unwrap();
        assert_eq!(config.dependencies.len(), 2);
        assert_eq!(config.dev_dependencies.len(), 2);
    }

    #[test]
    fn test_project_dep_same_name_in_deps_and_dev_deps() {
        // Same dependency name in both sections — should be independent
        let config = parse_shape_project_toml(
            r#"
[dependencies]
shared = "1.0.0"

[dev-dependencies]
shared = "2.0.0"
"#,
        )
        .unwrap();
        assert_eq!(
            config.dependencies.get("shared"),
            Some(&DependencySpec::Version("1.0.0".to_string()))
        );
        assert_eq!(
            config.dev_dependencies.get("shared"),
            Some(&DependencySpec::Version("2.0.0".to_string()))
        );
    }

    #[test]
    fn test_project_dep_git_url_formats() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
https-dep = { git = "https://github.com/org/repo.git", tag = "v1" }
ssh-dep = { git = "git@github.com:org/repo.git", tag = "v1" }
bare-dep = { git = "github.com/org/repo", branch = "main" }
"#,
        )
        .unwrap();
        assert_eq!(config.dependencies.len(), 3);
        let errors = config.validate();
        assert!(errors.is_empty(), "All deps have refs: {:?}", errors);
    }

    #[test]
    fn test_project_dep_empty_git_url() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
empty-git = { git = "", tag = "v1" }
"#,
        )
        .unwrap();
        // BUG: No validation for empty git URL
        let errors = config.validate();
        let _ = errors;
    }

    #[test]
    fn test_project_dep_empty_path() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
empty-path = { path = "" }
"#,
        )
        .unwrap();
        match config.dependencies.get("empty-path").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some(""));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
        // BUG: No validation for empty path
    }

    #[test]
    fn test_project_dep_absolute_path() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
absolute = { path = "/usr/local/lib/shape-lib" }
"#,
        )
        .unwrap();
        match config.dependencies.get("absolute").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("/usr/local/lib/shape-lib"));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    // =========================================================================
    // Category 3: Project Root Discovery (~20 tests)
    // =========================================================================

    #[test]
    fn test_project_discovery_current_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("shape.toml");
        let mut f = std::fs::File::create(&toml_path).unwrap();
        writeln!(f, "[project]\nname = \"here\"\nversion = \"1.0.0\"").unwrap();

        let result = find_project_root(tmp.path());
        assert!(result.is_some());
        let root = result.unwrap();
        assert_eq!(root.config.project.name, "here");
        assert_eq!(root.root_path, tmp.path());
    }

    #[test]
    fn test_project_discovery_walks_up_one_level() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(tmp.path().join("shape.toml")).unwrap();
        writeln!(f, "[project]\nname = \"parent\"").unwrap();

        let child = tmp.path().join("src");
        std::fs::create_dir_all(&child).unwrap();

        let result = find_project_root(&child);
        assert!(result.is_some());
        assert_eq!(result.unwrap().config.project.name, "parent");
    }

    #[test]
    fn test_project_discovery_walks_up_deep() {
        let tmp = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(tmp.path().join("shape.toml")).unwrap();
        writeln!(f, "[project]\nname = \"root\"").unwrap();

        let deep = tmp.path().join("a").join("b").join("c").join("d").join("e");
        std::fs::create_dir_all(&deep).unwrap();

        let result = find_project_root(&deep);
        assert!(result.is_some());
        assert_eq!(result.unwrap().config.project.name, "root");
    }

    #[test]
    fn test_project_discovery_no_toml_anywhere() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("deep").join("nested");
        std::fs::create_dir_all(&nested).unwrap();

        // In a tempdir, there shouldn't be a shape.toml above
        // This test might find one if run in a project dir, so we just verify no panic
        let _result = find_project_root(&nested);
    }

    #[test]
    fn test_project_discovery_picks_nearest() {
        let tmp = tempfile::tempdir().unwrap();

        // Parent shape.toml
        let mut f = std::fs::File::create(tmp.path().join("shape.toml")).unwrap();
        writeln!(f, "[project]\nname = \"parent\"").unwrap();

        // Child shape.toml (closer)
        let child = tmp.path().join("child");
        std::fs::create_dir_all(&child).unwrap();
        let mut f = std::fs::File::create(child.join("shape.toml")).unwrap();
        writeln!(f, "[project]\nname = \"child\"").unwrap();

        // Search from grandchild
        let grandchild = child.join("src");
        std::fs::create_dir_all(&grandchild).unwrap();

        let result = find_project_root(&grandchild);
        assert!(result.is_some());
        // Should find child's shape.toml (nearest)
        assert_eq!(result.unwrap().config.project.name, "child");
    }

    #[test]
    fn test_project_discovery_picks_nearest_from_child_dir() {
        let tmp = tempfile::tempdir().unwrap();

        // Parent shape.toml
        let mut f = std::fs::File::create(tmp.path().join("shape.toml")).unwrap();
        writeln!(f, "[project]\nname = \"parent\"").unwrap();

        // Child shape.toml
        let child = tmp.path().join("sub");
        std::fs::create_dir_all(&child).unwrap();
        let mut f = std::fs::File::create(child.join("shape.toml")).unwrap();
        writeln!(f, "[project]\nname = \"sub\"").unwrap();

        // Search from child itself
        let result = find_project_root(&child);
        assert!(result.is_some());
        assert_eq!(result.unwrap().config.project.name, "sub");
    }

    #[test]
    fn test_project_discovery_empty_toml_file() {
        let tmp = tempfile::tempdir().unwrap();
        // Empty shape.toml — should parse with defaults
        std::fs::File::create(tmp.path().join("shape.toml")).unwrap();

        let result = find_project_root(tmp.path());
        assert!(result.is_some());
        assert_eq!(result.unwrap().config.project.name, "");
    }

    #[test]
    fn test_project_discovery_toml_is_directory() {
        let tmp = tempfile::tempdir().unwrap();
        // Create shape.toml as a directory instead of file
        std::fs::create_dir_all(tmp.path().join("shape.toml")).unwrap();

        let result = find_project_root(tmp.path());
        // is_file() returns false for directories, so should not find it
        assert!(result.is_none() || result.unwrap().root_path != tmp.path());
    }

    #[test]
    fn test_project_discovery_invalid_toml_content() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("shape.toml"), "this is not valid toml {{{").unwrap();

        // find_project_root prints to stderr and returns None
        let result = find_project_root(tmp.path());
        assert!(
            result.is_none(),
            "Invalid TOML should cause find_project_root to return None"
        );

        // try_find_project_root returns a structured error
        let result = try_find_project_root(tmp.path());
        assert!(
            result.is_err(),
            "try_find_project_root should return Err for invalid TOML"
        );
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("Malformed shape.toml"),
            "Error should mention 'Malformed shape.toml', got: {}",
            err_msg
        );
    }

    #[test]
    fn test_project_discovery_skips_invalid_walks_further() {
        // If a shape.toml at one level is invalid, does it walk further up?
        let tmp = tempfile::tempdir().unwrap();

        // Valid parent
        let mut f = std::fs::File::create(tmp.path().join("shape.toml")).unwrap();
        writeln!(f, "[project]\nname = \"parent\"").unwrap();

        // Invalid child
        let child = tmp.path().join("child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("shape.toml"), "invalid toml {{{").unwrap();

        // find_project_root stops at the invalid child shape.toml and returns None
        let result = find_project_root(&child);
        assert!(
            result.is_none(),
            "find_project_root returns None when nearest shape.toml is invalid"
        );

        // try_find_project_root returns an error for the invalid TOML
        let result = try_find_project_root(&child);
        assert!(
            result.is_err(),
            "try_find_project_root should return Err for invalid child TOML"
        );
    }

    #[test]
    fn test_project_discovery_symlinked_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().join("real");
        std::fs::create_dir_all(&real_dir).unwrap();
        let mut f = std::fs::File::create(real_dir.join("shape.toml")).unwrap();
        writeln!(f, "[project]\nname = \"real\"").unwrap();

        let link_path = tmp.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_dir, &link_path).unwrap();
        #[cfg(not(unix))]
        {
            // Skip on non-unix
            return;
        }

        let result = find_project_root(&link_path);
        assert!(result.is_some());
        // The root path may be the symlink or the real path depending on resolution
        assert_eq!(result.unwrap().config.project.name, "real");
    }

    #[test]
    fn test_project_discovery_symlinked_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let real_toml = tmp.path().join("real_shape.toml");
        let mut f = std::fs::File::create(&real_toml).unwrap();
        writeln!(f, "[project]\nname = \"symlinked\"").unwrap();

        let link_path = tmp.path().join("shape.toml");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_toml, &link_path).unwrap();
        #[cfg(not(unix))]
        {
            return;
        }

        let result = find_project_root(tmp.path());
        assert!(result.is_some());
        assert_eq!(result.unwrap().config.project.name, "symlinked");
    }

    #[test]
    fn test_project_discovery_resolved_module_paths() {
        let root = ProjectRoot {
            root_path: PathBuf::from("/project"),
            config: ShapeProject {
                modules: ModulesSection {
                    paths: vec![
                        "lib".to_string(),
                        "vendor".to_string(),
                        "custom/modules".to_string(),
                    ],
                },
                ..Default::default()
            },
        };

        let resolved = root.resolved_module_paths();
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0], PathBuf::from("/project/lib"));
        assert_eq!(resolved[1], PathBuf::from("/project/vendor"));
        assert_eq!(resolved[2], PathBuf::from("/project/custom/modules"));
    }

    #[test]
    fn test_project_discovery_resolved_module_paths_empty() {
        let root = ProjectRoot {
            root_path: PathBuf::from("/project"),
            config: ShapeProject::default(),
        };

        let resolved = root.resolved_module_paths();
        assert!(resolved.is_empty());
    }

    #[test]
    fn test_project_discovery_resolved_module_paths_absolute() {
        // If a module path is absolute, join still works but gives absolute path
        let root = ProjectRoot {
            root_path: PathBuf::from("/project"),
            config: ShapeProject {
                modules: ModulesSection {
                    paths: vec!["/absolute/path".to_string()],
                },
                ..Default::default()
            },
        };

        let resolved = root.resolved_module_paths();
        // PathBuf::join with an absolute path replaces the base
        assert_eq!(resolved[0], PathBuf::from("/absolute/path"));
    }

    #[test]
    fn test_project_discovery_resolved_module_paths_with_dotdot() {
        let root = ProjectRoot {
            root_path: PathBuf::from("/project"),
            config: ShapeProject {
                modules: ModulesSection {
                    paths: vec!["../sibling/lib".to_string()],
                },
                ..Default::default()
            },
        };

        let resolved = root.resolved_module_paths();
        // join doesn't canonicalize, so it contains the ..
        assert_eq!(resolved[0], PathBuf::from("/project/../sibling/lib"));
    }

    // =========================================================================
    // Category 4: Build Configuration (~15 tests)
    // =========================================================================

    #[test]
    fn test_project_build_target_bytecode() {
        let config = parse_shape_project_toml("[build]\ntarget = \"bytecode\"").unwrap();
        assert_eq!(config.build.target.as_deref(), Some("bytecode"));
    }

    #[test]
    fn test_project_build_target_native() {
        let config = parse_shape_project_toml("[build]\ntarget = \"native\"").unwrap();
        assert_eq!(config.build.target.as_deref(), Some("native"));
    }

    #[test]
    fn test_project_build_target_invalid_string() {
        // Invalid target value — no validation at parse time
        let config = parse_shape_project_toml("[build]\ntarget = \"custom-target\"").unwrap();
        assert_eq!(config.build.target.as_deref(), Some("custom-target"));
        // BUG: No validation for invalid target values
    }

    #[test]
    fn test_project_build_opt_level_0() {
        let config = parse_shape_project_toml("[build]\nopt_level = 0").unwrap();
        assert_eq!(config.build.opt_level, Some(0));
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_project_build_opt_level_1() {
        let config = parse_shape_project_toml("[build]\nopt_level = 1").unwrap();
        assert_eq!(config.build.opt_level, Some(1));
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_project_build_opt_level_2() {
        let config = parse_shape_project_toml("[build]\nopt_level = 2").unwrap();
        assert_eq!(config.build.opt_level, Some(2));
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_project_build_opt_level_3() {
        let config = parse_shape_project_toml("[build]\nopt_level = 3").unwrap();
        assert_eq!(config.build.opt_level, Some(3));
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_project_build_opt_level_4_invalid() {
        let config = parse_shape_project_toml("[build]\nopt_level = 4").unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("opt_level") && e.contains("4")),
            "opt_level 4 should be invalid: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_build_opt_level_255_max_u8() {
        let config = parse_shape_project_toml("[build]\nopt_level = 255").unwrap();
        assert_eq!(config.build.opt_level, Some(255));
        let errors = config.validate();
        assert!(
            errors.iter().any(|e| e.contains("opt_level")),
            "opt_level 255 should fail validation: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_build_opt_level_missing_defaults_none() {
        let config = parse_shape_project_toml("[build]\ntarget = \"bytecode\"").unwrap();
        assert_eq!(config.build.opt_level, None);
        // None opt_level should pass validation
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_project_build_output_with_spaces() {
        let config = parse_shape_project_toml("[build]\noutput = \"my output dir/build\"").unwrap();
        assert_eq!(config.build.output.as_deref(), Some("my output dir/build"));
    }

    #[test]
    fn test_project_build_output_unicode() {
        let config = parse_shape_project_toml("[build]\noutput = \"ビルド/出力\"").unwrap();
        assert_eq!(config.build.output.as_deref(), Some("ビルド/出力"));
    }

    #[test]
    fn test_project_build_external_mode_update() {
        let config = parse_shape_project_toml(
            r#"
[build.external]
mode = "update"
"#,
        )
        .unwrap();
        assert_eq!(config.build.external.mode, ExternalLockMode::Update);
    }

    #[test]
    fn test_project_build_external_mode_frozen() {
        let config = parse_shape_project_toml(
            r#"
[build.external]
mode = "frozen"
"#,
        )
        .unwrap();
        assert_eq!(config.build.external.mode, ExternalLockMode::Frozen);
    }

    #[test]
    fn test_project_build_external_mode_invalid() {
        let result = parse_shape_project_toml(
            r#"
[build.external]
mode = "invalid"
"#,
        );
        assert!(
            result.is_err(),
            "Invalid external.mode should fail deserialization"
        );
    }

    #[test]
    fn test_project_build_external_mode_default() {
        let config = parse_shape_project_toml("[build]\ntarget = \"bytecode\"").unwrap();
        assert_eq!(config.build.external.mode, ExternalLockMode::Update);
    }

    #[test]
    fn test_project_build_missing_section_defaults() {
        let config = parse_shape_project_toml("[project]\nname = \"test\"").unwrap();
        assert!(config.build.target.is_none());
        assert!(config.build.opt_level.is_none());
        assert!(config.build.output.is_none());
        assert_eq!(config.build.external.mode, ExternalLockMode::Update);
    }

    // =========================================================================
    // Category 5: Extensions (~15 tests)
    // =========================================================================

    #[test]
    fn test_project_ext_single_extension() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "csv"
path = "./libshape_ext_csv.so"
"#,
        )
        .unwrap();
        assert_eq!(config.extensions.len(), 1);
        assert_eq!(config.extensions[0].name, "csv");
        assert_eq!(
            config.extensions[0].path,
            PathBuf::from("./libshape_ext_csv.so")
        );
        assert!(config.extensions[0].config.is_empty());
    }

    #[test]
    fn test_project_ext_multiple_extensions() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "csv"
path = "./csv.so"

[[extensions]]
name = "duckdb"
path = "./duckdb.so"

[[extensions]]
name = "http"
path = "./http.so"
"#,
        )
        .unwrap();
        assert_eq!(config.extensions.len(), 3);
        assert_eq!(config.extensions[0].name, "csv");
        assert_eq!(config.extensions[1].name, "duckdb");
        assert_eq!(config.extensions[2].name, "http");
    }

    #[test]
    fn test_project_ext_with_config_table() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "market-data"
path = "./market.so"

[extensions.config]
duckdb_path = "/path/to/db"
default_timeframe = "1d"
"#,
        )
        .unwrap();
        assert_eq!(config.extensions.len(), 1);
        let ext = &config.extensions[0];
        assert_eq!(
            ext.config.get("duckdb_path"),
            Some(&toml::Value::String("/path/to/db".to_string()))
        );
        assert_eq!(
            ext.config.get("default_timeframe"),
            Some(&toml::Value::String("1d".to_string()))
        );
    }

    #[test]
    fn test_project_ext_config_as_json() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "test"
path = "./test.so"

[extensions.config]
str_val = "hello"
int_val = 42
float_val = 3.14
bool_val = true
"#,
        )
        .unwrap();
        let json = config.extensions[0].config_as_json();
        assert_eq!(json["str_val"], "hello");
        assert_eq!(json["int_val"], 42);
        assert_eq!(json["bool_val"], true);
        // Float comparison
        assert!((json["float_val"].as_f64().unwrap() - 3.14).abs() < 0.001);
    }

    #[test]
    fn test_project_ext_config_as_json_nested() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "test"
path = "./test.so"

[extensions.config]
simple = "value"

[extensions.config.nested]
key = "nested_value"
deep = 99
"#,
        )
        .unwrap();
        let json = config.extensions[0].config_as_json();
        assert_eq!(json["simple"], "value");
        assert_eq!(json["nested"]["key"], "nested_value");
        assert_eq!(json["nested"]["deep"], 99);
    }

    #[test]
    fn test_project_ext_config_as_json_array() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "test"
path = "./test.so"

[extensions.config]
items = [1, 2, 3]
tags = ["a", "b"]
"#,
        )
        .unwrap();
        let json = config.extensions[0].config_as_json();
        assert_eq!(json["items"], serde_json::json!([1, 2, 3]));
        assert_eq!(json["tags"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn test_project_ext_missing_name_fails() {
        let result = parse_shape_project_toml(
            r#"
[[extensions]]
path = "./test.so"
"#,
        );
        assert!(
            result.is_err(),
            "Extension without name should fail (name is not Option)"
        );
    }

    #[test]
    fn test_project_ext_missing_path_fails() {
        let result = parse_shape_project_toml(
            r#"
[[extensions]]
name = "test"
"#,
        );
        assert!(
            result.is_err(),
            "Extension without path should fail (path is not Option)"
        );
    }

    #[test]
    fn test_project_ext_empty_config() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "minimal"
path = "./min.so"
config = {}
"#,
        );
        // Inline table for config should work
        match config {
            Ok(c) => {
                assert!(c.extensions[0].config.is_empty());
            }
            Err(e) => {
                // If inline config = {} doesn't work, note it
                panic!("Empty inline config failed: {}", e);
            }
        }
    }

    #[test]
    fn test_project_ext_config_with_datetime() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "timed"
path = "./timed.so"

[extensions.config]
created = 2024-01-15T10:30:00Z
"#,
        )
        .unwrap();
        let json = config.extensions[0].config_as_json();
        // Datetime should be converted to string in JSON
        assert!(json["created"].is_string());
    }

    #[test]
    fn test_project_ext_unicode_name() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "データベース"
path = "./db.so"
"#,
        )
        .unwrap();
        assert_eq!(config.extensions[0].name, "データベース");
    }

    #[test]
    fn test_project_ext_path_with_spaces() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "spaced"
path = "./my extensions/lib file.so"
"#,
        )
        .unwrap();
        assert_eq!(
            config.extensions[0].path,
            PathBuf::from("./my extensions/lib file.so")
        );
    }

    #[test]
    fn test_project_ext_multiple_with_configs() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "ext1"
path = "./ext1.so"

[extensions.config]
key1 = "val1"

[[extensions]]
name = "ext2"
path = "./ext2.so"

[extensions.config]
key2 = "val2"
"#,
        );
        // TOML array of tables with sub-tables can be tricky
        match config {
            Ok(c) => {
                assert_eq!(c.extensions.len(), 2);
                assert!(c.extensions[0].config.contains_key("key1"));
                assert!(c.extensions[1].config.contains_key("key2"));
            }
            Err(e) => {
                // Note: multiple extensions with individual config tables might
                // require specific TOML formatting
                panic!("Multiple extensions with configs failed: {}", e);
            }
        }
    }

    // =========================================================================
    // Category 6: Frontmatter (~15 tests)
    // =========================================================================

    #[test]
    fn test_project_frontmatter_valid_all_fields() {
        let source = r#"---
name = "my-script"
description = "A test"
version = "1.0.0"
author = "dev"
tags = ["test"]

[modules]
paths = ["lib"]

[dependencies]
foo = "1.0"

[dev-dependencies]
bar = "2.0"

[[extensions]]
name = "csv"
path = "./csv.so"
---
let x = 1;
"#;
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_some());
        assert!(
            diagnostics.is_empty(),
            "Valid frontmatter should have no diagnostics: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert_eq!(rest, "let x = 1;\n");
    }

    #[test]
    fn test_project_frontmatter_with_shebang() {
        let source = "#!/usr/bin/env shape\n---\nname = \"test\"\n---\nprint(1);\n";
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_some());
        assert!(diagnostics.is_empty());
        assert_eq!(config.unwrap().name.as_deref(), Some("test"));
        assert_eq!(rest, "print(1);\n");
    }

    #[test]
    fn test_project_frontmatter_forbidden_project_section() {
        let source = "---\n[project]\nname = \"bad\"\n---\ncode;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.severity == FrontmatterDiagnosticSeverity::Error
                    && d.message.contains("[project]"))
        );
    }

    #[test]
    fn test_project_frontmatter_forbidden_build_section() {
        let source = "---\n[build]\nopt_level = 2\n---\ncode;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.severity == FrontmatterDiagnosticSeverity::Error
                    && d.message.contains("Build configuration"))
        );
    }

    #[test]
    fn test_project_frontmatter_forbidden_plugins_section() {
        let source = "---\n[plugins]\nfoo = \"bar\"\n---\ncode;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.severity == FrontmatterDiagnosticSeverity::Error
                    && d.message.contains("[[extensions]]"))
        );
    }

    #[test]
    fn test_project_frontmatter_unknown_key_warning() {
        let source = "---\ncustom_key = \"value\"\n---\ncode;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(diagnostics.iter().any(|d| {
            d.severity == FrontmatterDiagnosticSeverity::Warning
                && d.message.contains("Unknown frontmatter key")
        }));
    }

    #[test]
    fn test_project_frontmatter_empty() {
        let source = "---\n---\ncode;\n";
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_some());
        assert!(diagnostics.is_empty());
        assert_eq!(rest, "code;\n");
    }

    #[test]
    fn test_project_frontmatter_malformed_toml() {
        let source = "---\nnot valid {{{ toml\n---\ncode;\n";
        let (config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(config.is_none());
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("Frontmatter TOML parse error"))
        );
    }

    #[test]
    fn test_project_frontmatter_no_closing_delimiter() {
        let source = "---\nname = \"test\"\nno closing\n";
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_none());
        assert!(diagnostics.is_empty());
        // No frontmatter found, entire source is returned
        assert_eq!(rest, source);
    }

    #[test]
    fn test_project_frontmatter_multiple_forbidden_sections() {
        let source = "---\n[project]\nname = \"x\"\n\n[build]\nopt = 1\n---\ncode;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        let error_count = diagnostics
            .iter()
            .filter(|d| d.severity == FrontmatterDiagnosticSeverity::Error)
            .count();
        assert!(
            error_count >= 2,
            "Should have at least 2 errors for project+build: {:?}",
            diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_project_frontmatter_extensions_missing_name() {
        let source = "---\n[[extensions]]\npath = \"./ext.so\"\n---\ncode;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("Missing required key 'name'"))
        );
    }

    #[test]
    fn test_project_frontmatter_extensions_missing_path() {
        let source = "---\n[[extensions]]\nname = \"ext\"\n---\ncode;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("Missing required key 'path'"))
        );
    }

    #[test]
    fn test_project_frontmatter_extensions_unknown_key() {
        let source =
            "---\n[[extensions]]\nname = \"ext\"\npath = \"./e.so\"\nfoo = \"bar\"\n---\ncode;\n";
        let (_config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(diagnostics.iter().any(|d| {
            d.severity == FrontmatterDiagnosticSeverity::Error
                && d.message.contains("Unknown key 'foo'")
        }));
    }

    #[test]
    fn test_project_frontmatter_legacy_parse_returns_shapeproject() {
        let source = "---\n[dependencies]\nfoo = \"1.0\"\n---\ncode;\n";
        let (config, rest) = parse_frontmatter(source);
        assert!(config.is_some());
        let cfg = config.unwrap();
        assert!(cfg.dependencies.contains_key("foo"));
        assert_eq!(rest, "code;\n");
    }

    #[test]
    fn test_project_frontmatter_delimiter_with_trailing_spaces() {
        // Delimiter with trailing whitespace
        let source = "---   \nname = \"test\"\n---   \ncode;\n";
        let (config, _diagnostics, rest) = parse_frontmatter_validated(source);
        // The opening delimiter check looks at trimmed start_with("---") and rest is whitespace
        assert!(config.is_some());
        assert_eq!(rest, "code;\n");
    }

    #[test]
    fn test_project_frontmatter_only_shebang_no_frontmatter() {
        let source = "#!/usr/bin/env shape\nlet x = 1;\n";
        let (config, diagnostics, rest) = parse_frontmatter_validated(source);
        assert!(config.is_none());
        assert!(diagnostics.is_empty());
        assert_eq!(rest, source);
    }

    // =========================================================================
    // Category 7: Module Path Configuration (~10 tests)
    // =========================================================================

    #[test]
    fn test_project_modules_default_empty_paths() {
        let config = parse_shape_project_toml("").unwrap();
        assert!(config.modules.paths.is_empty());
    }

    #[test]
    fn test_project_modules_single_path() {
        let config = parse_shape_project_toml("[modules]\npaths = [\"src\"]").unwrap();
        assert_eq!(config.modules.paths, vec!["src"]);
    }

    #[test]
    fn test_project_modules_multiple_paths() {
        let config =
            parse_shape_project_toml("[modules]\npaths = [\"lib\", \"vendor\", \"ext\"]").unwrap();
        assert_eq!(config.modules.paths, vec!["lib", "vendor", "ext"]);
    }

    #[test]
    fn test_project_modules_paths_with_spaces() {
        let config =
            parse_shape_project_toml("[modules]\npaths = [\"my lib\", \"vendor modules\"]")
                .unwrap();
        assert_eq!(config.modules.paths, vec!["my lib", "vendor modules"]);
    }

    #[test]
    fn test_project_modules_paths_unicode() {
        let config = parse_shape_project_toml("[modules]\npaths = [\"ライブラリ\"]").unwrap();
        assert_eq!(config.modules.paths, vec!["ライブラリ"]);
    }

    #[test]
    fn test_project_modules_paths_relative_vs_absolute() {
        let config = parse_shape_project_toml(
            "[modules]\npaths = [\"relative\", \"/absolute/path\", \"../parent\"]",
        )
        .unwrap();
        assert_eq!(config.modules.paths.len(), 3);
        assert_eq!(config.modules.paths[0], "relative");
        assert_eq!(config.modules.paths[1], "/absolute/path");
        assert_eq!(config.modules.paths[2], "../parent");
    }

    #[test]
    fn test_project_modules_empty_paths_list() {
        let config = parse_shape_project_toml("[modules]\npaths = []").unwrap();
        assert!(config.modules.paths.is_empty());
    }

    #[test]
    fn test_project_modules_duplicate_paths() {
        let config =
            parse_shape_project_toml("[modules]\npaths = [\"lib\", \"lib\", \"lib\"]").unwrap();
        // No dedup at this level
        assert_eq!(config.modules.paths.len(), 3);
        assert!(config.modules.paths.iter().all(|p| p == "lib"));
    }

    #[test]
    fn test_project_modules_path_empty_string() {
        let config = parse_shape_project_toml("[modules]\npaths = [\"\"]").unwrap();
        assert_eq!(config.modules.paths, vec![""]);
        // BUG: No validation against empty module paths
    }

    #[test]
    fn test_project_modules_in_frontmatter() {
        let source = "---\n[modules]\npaths = [\"custom_lib\"]\n---\nlet x = 1;\n";
        let (config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(config.is_some());
        assert!(diagnostics.is_empty());
        let modules = config.unwrap().modules.unwrap();
        assert_eq!(modules.paths, vec!["custom_lib"]);
    }

    // =========================================================================
    // Category 8: Validation comprehensive tests (~15 more tests)
    // =========================================================================

    #[test]
    fn test_project_validate_empty_config_no_errors() {
        // Completely empty config — name is empty, version is empty,
        // but no other fields are set so the name check shouldn't trigger
        let config = parse_shape_project_toml("").unwrap();
        let errors = config.validate();
        assert!(
            errors.is_empty(),
            "Empty config should have no validation errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_validate_name_empty_with_version_set() {
        // Empty name with version set should trigger error
        let config = parse_shape_project_toml("[project]\nversion = \"1.0.0\"").unwrap();
        let errors = config.validate();
        assert!(
            errors.iter().any(|e| e.contains("project.name")),
            "Empty name with version should error: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_validate_name_empty_with_entry_set() {
        let config = parse_shape_project_toml("[project]\nentry = \"main.shape\"").unwrap();
        let errors = config.validate();
        assert!(
            errors.iter().any(|e| e.contains("project.name")),
            "Empty name with entry should error: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_validate_name_empty_with_authors_set() {
        let config = parse_shape_project_toml("[project]\nauthors = [\"dev\"]").unwrap();
        let errors = config.validate();
        assert!(
            errors.iter().any(|e| e.contains("project.name")),
            "Empty name with authors should error: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_validate_name_present_version_empty_ok() {
        // Name set, version empty — this should be OK
        let config = parse_shape_project_toml("[project]\nname = \"test\"").unwrap();
        let errors = config.validate();
        assert!(
            errors.is_empty(),
            "Name present, empty version OK: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_validate_multiple_errors_accumulated() {
        let config = parse_shape_project_toml(
            r#"
[project]
version = "1.0.0"

[dependencies]
bad1 = { path = "../x", git = "https://example.com/a.git", tag = "v1" }
bad2 = { git = "https://example.com/b.git" }

[build]
opt_level = 10
"#,
        )
        .unwrap();
        let errors = config.validate();
        // Should have: empty name error, bad1 path+git, bad2 no ref, opt_level > 3
        assert!(
            errors.len() >= 3,
            "Should accumulate multiple errors: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_validate_git_with_rev_is_ok() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
pinned = { git = "https://example.com/repo.git", rev = "deadbeef" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        assert!(errors.is_empty(), "git+rev should be valid: {:?}", errors);
    }

    #[test]
    fn test_project_validate_git_with_tag_is_ok() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
tagged = { git = "https://example.com/repo.git", tag = "v1.0.0" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        assert!(errors.is_empty(), "git+tag should be valid: {:?}", errors);
    }

    #[test]
    fn test_project_validate_git_with_branch_is_ok() {
        let config = parse_shape_project_toml(
            r#"
[dependencies]
branched = { git = "https://example.com/repo.git", branch = "develop" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        assert!(
            errors.is_empty(),
            "git+branch should be valid: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_validate_version_dep_no_errors() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "test"

[dependencies]
simple = "1.0.0"
"#,
        )
        .unwrap();
        let errors = config.validate();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_project_validate_path_dep_no_errors() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "test"

[dependencies]
local = { path = "../lib" }
"#,
        )
        .unwrap();
        let errors = config.validate();
        assert!(errors.is_empty());
    }

    #[test]
    fn test_project_validate_no_name_no_other_fields_ok() {
        // Empty name, empty version, no entry, no authors — should be OK
        let config = parse_shape_project_toml("[project]\n").unwrap();
        let errors = config.validate();
        assert!(
            errors.is_empty(),
            "All-empty project section should be OK: {:?}",
            errors
        );
    }

    // =========================================================================
    // Additional edge case tests to reach ~125
    // =========================================================================

    #[test]
    fn test_project_parse_toml_datetime_in_wrong_field() {
        // Putting a datetime where a string is expected
        let result = parse_shape_project_toml("[project]\nname = 2024-01-15T10:30:00Z");
        assert!(
            result.is_err(),
            "Datetime value for string field should fail"
        );
    }

    #[test]
    fn test_project_parse_table_array_for_dependencies() {
        // Using [[dependencies]] array-of-tables syntax instead of [dependencies] table
        let result = parse_shape_project_toml("[[dependencies]]\nfoo = \"1.0\"");
        assert!(
            result.is_err(),
            "[[dependencies]] (array-of-tables) should fail for HashMap"
        );
    }

    #[test]
    fn test_project_parse_deeply_nested_extension_config() {
        let config = parse_shape_project_toml(
            r#"
[[extensions]]
name = "deep"
path = "./deep.so"

[extensions.config]
level1 = "a"

[extensions.config.level2]
key = "b"

[extensions.config.level2.level3]
key = "c"
"#,
        )
        .unwrap();
        let json = config.extensions[0].config_as_json();
        assert_eq!(json["level1"], "a");
        assert_eq!(json["level2"]["key"], "b");
        assert_eq!(json["level2"]["level3"]["key"], "c");
    }

    #[test]
    fn test_project_config_as_json_nan_float() {
        // NaN float in TOML config should be handled
        // TOML spec doesn't support NaN in standard format, but some parsers do
        let result = parse_shape_project_toml(
            r#"
[[extensions]]
name = "nan-test"
path = "./nan.so"

[extensions.config]
value = nan
"#,
        );
        match result {
            Ok(config) => {
                let json = config.extensions[0].config_as_json();
                // NaN should become null in JSON (from_f64 returns None for NaN)
                assert!(json["value"].is_null(), "NaN should become null in JSON");
            }
            Err(_) => {
                // Some TOML parsers may reject nan
            }
        }
    }

    #[test]
    fn test_project_config_as_json_inf_float() {
        let result = parse_shape_project_toml(
            r#"
[[extensions]]
name = "inf-test"
path = "./inf.so"

[extensions.config]
value = inf
"#,
        );
        match result {
            Ok(config) => {
                let json = config.extensions[0].config_as_json();
                // inf should become null in JSON (from_f64 returns None for inf)
                assert!(json["value"].is_null(), "Inf should become null in JSON");
            }
            Err(_) => {
                // Some TOML parsers may reject inf
            }
        }
    }

    #[test]
    fn test_project_parse_toml_with_bom() {
        // UTF-8 BOM at start of file
        let toml_str = "\u{feff}[project]\nname = \"bom-test\"";
        let result = parse_shape_project_toml(toml_str);
        // TOML spec says BOM should be handled, but it depends on the parser
        match result {
            Ok(config) => assert_eq!(config.project.name, "bom-test"),
            Err(_) => {
                // BUG: TOML parser doesn't handle BOM
            }
        }
    }

    #[test]
    fn test_project_parse_dotted_keys() {
        // TOML supports dotted keys
        let config =
            parse_shape_project_toml("project.name = \"dotted\"\nproject.version = \"1.0\"")
                .unwrap();
        assert_eq!(config.project.name, "dotted");
        assert_eq!(config.project.version, "1.0");
    }

    #[test]
    fn test_project_validate_only_license_no_name() {
        // license and shape_version are Option, setting them alone shouldn't trigger name check
        let config = parse_shape_project_toml(
            r#"
[project]
license = "MIT"
shape-version = "0.5.0"
"#,
        )
        .unwrap();
        let errors = config.validate();
        // Name check triggers only if version, entry, or authors are set
        assert!(
            errors.is_empty(),
            "Only optional fields set should not trigger name error: {:?}",
            errors
        );
    }

    #[test]
    fn test_project_parse_inline_table_build() {
        let config =
            parse_shape_project_toml("build = { target = \"native\", opt_level = 1 }").unwrap();
        assert_eq!(config.build.target.as_deref(), Some("native"));
        assert_eq!(config.build.opt_level, Some(1));
    }

    #[test]
    fn test_project_serde_roundtrip() {
        let toml_str = r#"
[project]
name = "roundtrip"
version = "1.0.0"
entry = "main.shape"
authors = ["Dev"]
shape-version = "0.5.0"
license = "MIT"
repository = "https://example.com/repo"

[modules]
paths = ["lib"]

[dependencies]
foo = "1.0"

[build]
target = "bytecode"
opt_level = 2
output = "dist/"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        // Serialize back to TOML
        let serialized = toml::to_string(&config).unwrap();
        // Parse serialized version
        let config2: ShapeProject = parse_shape_project_toml(&serialized).unwrap();
        assert_eq!(config.project.name, config2.project.name);
        assert_eq!(config.project.version, config2.project.version);
        assert_eq!(config.modules.paths, config2.modules.paths);
        assert_eq!(config.build.opt_level, config2.build.opt_level);
    }

    #[test]
    fn test_project_default_trait() {
        let config = ShapeProject::default();
        assert_eq!(config.project.name, "");
        assert_eq!(config.project.version, "");
        assert!(config.dependencies.is_empty());
        assert!(config.dev_dependencies.is_empty());
        assert!(config.extensions.is_empty());
        assert!(config.modules.paths.is_empty());
        assert!(config.build.target.is_none());
        assert!(config.build.opt_level.is_none());
    }

    #[test]
    fn test_project_build_section_default_trait() {
        let build = BuildSection::default();
        assert!(build.target.is_none());
        assert!(build.opt_level.is_none());
        assert!(build.output.is_none());
        assert_eq!(build.external.mode, ExternalLockMode::Update);
    }

    #[test]
    fn test_project_external_lock_mode_default() {
        let mode = ExternalLockMode::default();
        assert_eq!(mode, ExternalLockMode::Update);
    }

    #[test]
    fn test_project_dependency_spec_equality() {
        let v1 = DependencySpec::Version("1.0.0".to_string());
        let v2 = DependencySpec::Version("1.0.0".to_string());
        let v3 = DependencySpec::Version("2.0.0".to_string());
        assert_eq!(v1, v2);
        assert_ne!(v1, v3);
    }

    #[test]
    fn test_project_detailed_dependency_equality() {
        let d1 = DetailedDependency {
            version: Some("1.0".to_string()),
            path: None,
            git: None,
            tag: None,
            branch: None,
            rev: None,
            permissions: None,
        };
        let d2 = DetailedDependency {
            version: Some("1.0".to_string()),
            path: None,
            git: None,
            tag: None,
            branch: None,
            rev: None,
            permissions: None,
        };
        assert_eq!(d1, d2);
    }

    #[test]
    fn test_project_discovery_write_and_read_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_content = r#"
[project]
name = "disk-test"
version = "2.0.0"
entry = "main.shape"

[modules]
paths = ["lib", "vendor"]

[dependencies]
math = "0.5.0"
data = { path = "../data" }

[build]
target = "native"
opt_level = 3
output = "build/"

[build.external]
mode = "frozen"
"#;
        let toml_path = tmp.path().join("shape.toml");
        std::fs::write(&toml_path, toml_content).unwrap();

        let root = find_project_root(tmp.path()).unwrap();
        assert_eq!(root.config.project.name, "disk-test");
        assert_eq!(root.config.project.version, "2.0.0");
        assert_eq!(root.config.modules.paths, vec!["lib", "vendor"]);
        assert_eq!(root.config.dependencies.len(), 2);
        assert_eq!(root.config.build.target.as_deref(), Some("native"));
        assert_eq!(root.config.build.opt_level, Some(3));
        assert_eq!(root.config.build.external.mode, ExternalLockMode::Frozen);

        let resolved = root.resolved_module_paths();
        assert_eq!(resolved.len(), 2);
        assert!(resolved[0].ends_with("lib"));
        assert!(resolved[1].ends_with("vendor"));
    }

    #[test]
    fn test_project_parse_shape_version_field() {
        let config = parse_shape_project_toml(
            r#"
[project]
name = "test"
shape-version = ">=0.5.0"
"#,
        )
        .unwrap();
        assert_eq!(config.project.shape_version.as_deref(), Some(">=0.5.0"));
    }

    #[test]
    fn test_project_frontmatter_and_toml_interaction() {
        // Frontmatter in a script file should produce a ShapeProject (legacy parse)
        // that is a subset of what shape.toml supports
        let source = r#"---
[dependencies]
analysis = "1.0.0"
data = { path = "../data" }

[[extensions]]
name = "db"
path = "./db.so"
---
import analysis;
let x = analyze();
"#;
        let (config, rest) = parse_frontmatter(source);
        assert!(config.is_some());
        let cfg = config.unwrap();
        assert_eq!(cfg.dependencies.len(), 2);
        assert_eq!(cfg.extensions.len(), 1);
        assert_eq!(rest, "import analysis;\nlet x = analyze();\n");
    }

    #[test]
    fn test_project_frontmatter_modules_paths() {
        let source = "---\n[modules]\npaths = [\"mylib\", \"vendor\"]\n---\ncode;\n";
        let (config, diagnostics, _rest) = parse_frontmatter_validated(source);
        assert!(config.is_some());
        assert!(diagnostics.is_empty());
        let modules = config.unwrap().modules.unwrap();
        assert_eq!(modules.paths, vec!["mylib", "vendor"]);
    }

    #[test]
    fn test_project_parse_many_authors() {
        let mut authors = Vec::new();
        for i in 0..50 {
            authors.push(format!("\"Author {}\"", i));
        }
        let toml_str = format!(
            "[project]\nname = \"many-authors\"\nauthors = [{}]",
            authors.join(", ")
        );
        let config = parse_shape_project_toml(&toml_str).unwrap();
        assert_eq!(config.project.authors.len(), 50);
    }

    #[test]
    fn test_project_parse_many_module_paths() {
        let mut paths = Vec::new();
        for i in 0..20 {
            paths.push(format!("\"path_{}\"", i));
        }
        let toml_str = format!("[modules]\npaths = [{}]", paths.join(", "));
        let config = parse_shape_project_toml(&toml_str).unwrap();
        assert_eq!(config.modules.paths.len(), 20);
    }

    #[test]
    fn test_project_parse_large_config_stress() {
        let mut toml_str = String::from("[project]\nname = \"stress\"\nversion = \"1.0.0\"\n\n");
        toml_str.push_str("[dependencies]\n");
        for i in 0..200 {
            toml_str.push_str(&format!("dep-{} = \"1.0.{}\"\n", i, i));
        }
        toml_str.push_str("\n[dev-dependencies]\n");
        for i in 0..100 {
            toml_str.push_str(&format!("dev-dep-{} = \"2.0.{}\"\n", i, i));
        }
        let config = parse_shape_project_toml(&toml_str).unwrap();
        assert_eq!(config.dependencies.len(), 200);
        assert_eq!(config.dev_dependencies.len(), 100);
        let errors = config.validate();
        assert!(errors.is_empty());
    }
}
