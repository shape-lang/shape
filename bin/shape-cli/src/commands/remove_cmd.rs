use anyhow::{Context, Result};
use shape_runtime::project::parse_shape_project_toml;

/// Run the `shape remove` command: remove a dependency from the current project.
pub async fn run_remove(name: String) -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;
    let project = shape_runtime::project::try_find_project_root(&cwd)
        .map_err(|e| anyhow::anyhow!("{}", e))?
        .ok_or_else(|| {
            anyhow::anyhow!("No shape.toml found. Run `shape remove` from within a Shape project.")
        })?;

    let toml_path = project.root_path.join("shape.toml");
    let toml_text = std::fs::read_to_string(&toml_path)
        .with_context(|| format!("failed to read {}", toml_path.display()))?;

    // Verify the dependency exists
    let config = parse_shape_project_toml(&toml_text)
        .map_err(|e| anyhow::anyhow!("failed to parse shape.toml: {}", e))?;

    if !config.dependencies.contains_key(&name) {
        anyhow::bail!("dependency '{}' not found in [dependencies]", name);
    }

    // Remove the dependency line using string manipulation to preserve formatting
    let updated = remove_dependency_from_toml(&toml_text, &name).ok_or_else(|| {
        anyhow::anyhow!("could not find dependency '{}' line in shape.toml", name)
    })?;

    std::fs::write(&toml_path, &updated)
        .with_context(|| format!("failed to write {}", toml_path.display()))?;

    eprintln!("Removed '{}' from dependencies.", name);
    Ok(())
}

/// Remove a dependency line from the TOML text, preserving formatting.
fn remove_dependency_from_toml(toml_text: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = toml_text.lines().collect();

    // Find [dependencies] section
    let mut dep_section_start = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == "[dependencies]" {
            dep_section_start = Some(i);
            break;
        }
    }

    let section_start = dep_section_start?;

    // Find the dependency line within the section
    let mut section_end = lines.len();
    for i in (section_start + 1)..lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
            section_end = i;
            break;
        }
    }

    for i in (section_start + 1)..section_end {
        let trimmed = lines[i].trim();
        if let Some(eq_pos) = trimmed.find('=') {
            let key = trimmed[..eq_pos].trim();
            if key == name {
                let mut result: Vec<&str> = Vec::new();
                result.extend_from_slice(&lines[..i]);
                result.extend_from_slice(&lines[i + 1..]);
                let joined = result.join("\n");
                return Some(joined + if toml_text.ends_with('\n') { "\n" } else { "" });
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_dep() {
        let input = r#"[project]
name = "my-app"
version = "0.1.0"

[dependencies]
foo = "1.0.0"
bar = "2.0.0"
"#;
        let result = remove_dependency_from_toml(input, "foo").unwrap();
        assert!(!result.contains("foo"));
        assert!(result.contains("bar = \"2.0.0\""));
        assert!(result.contains("[dependencies]"));
    }

    #[test]
    fn test_remove_only_dep() {
        let input = r#"[project]
name = "my-app"

[dependencies]
foo = "1.0.0"

[build]
target = "bytecode"
"#;
        let result = remove_dependency_from_toml(input, "foo").unwrap();
        assert!(!result.contains("foo"));
        assert!(result.contains("[dependencies]"));
        assert!(result.contains("[build]"));
    }

    #[test]
    fn test_remove_nonexistent_returns_none() {
        let input = r#"[project]
name = "my-app"

[dependencies]
foo = "1.0.0"
"#;
        let result = remove_dependency_from_toml(input, "bar");
        assert!(result.is_none());
    }

    #[test]
    fn test_remove_no_deps_section_returns_none() {
        let input = r#"[project]
name = "my-app"
"#;
        let result = remove_dependency_from_toml(input, "foo");
        assert!(result.is_none());
    }

    #[test]
    fn test_remove_table_dep() {
        let input = r#"[project]
name = "my-app"

[dependencies]
foo = { path = "../foo" }
bar = "2.0.0"
"#;
        let result = remove_dependency_from_toml(input, "foo").unwrap();
        assert!(!result.contains("foo"));
        assert!(result.contains("bar = \"2.0.0\""));
    }
}
