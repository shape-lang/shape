//! Static schema definitions for shape.toml.
//!
//! Drives completions, hover, and diagnostics by describing every valid
//! section, key, type, and default in the shape.toml format.

/// The type a TOML key expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Str,
    Integer,
    Bool,
    ArrayOfStrings,
    /// Inline table or complex value (e.g. dependency spec)
    Table,
}

impl ValueType {
    pub fn display_name(self) -> &'static str {
        match self {
            ValueType::Str => "string",
            ValueType::Integer => "integer",
            ValueType::Bool => "boolean",
            ValueType::ArrayOfStrings => "array of strings",
            ValueType::Table => "table",
        }
    }
}

/// A key within a TOML section.
#[derive(Debug, Clone)]
pub struct KeyDef {
    pub name: &'static str,
    pub value_type: ValueType,
    pub required: bool,
    pub description: &'static str,
    /// Optional list of known valid values for completion.
    pub known_values: &'static [&'static str],
}

/// A top-level section (e.g. `[project]`) in shape.toml.
#[derive(Debug, Clone)]
pub struct SectionDef {
    pub name: &'static str,
    pub description: &'static str,
    /// Whether this is an array-of-tables section (e.g. `[[extensions]]`).
    pub is_array_table: bool,
    /// Whether this section holds arbitrary key-value pairs (like [dependencies]).
    pub is_free_form: bool,
    pub keys: &'static [KeyDef],
}

/// All known sections in shape.toml.
pub static SECTIONS: &[SectionDef] = &[
    SectionDef {
        name: "project",
        description: "Project metadata and configuration.",
        is_array_table: false,
        is_free_form: false,
        keys: &[
            KeyDef {
                name: "name",
                value_type: ValueType::Str,
                required: true,
                description: "The project name. Must be non-empty.",
                known_values: &[],
            },
            KeyDef {
                name: "version",
                value_type: ValueType::Str,
                required: true,
                description: "The project version (semver recommended, e.g. \"0.1.0\").",
                known_values: &[],
            },
            KeyDef {
                name: "entry",
                value_type: ValueType::Str,
                required: false,
                description: "Entry script for `shape` with no args (project mode). E.g. \"src/main.shape\".",
                known_values: &[],
            },
            KeyDef {
                name: "authors",
                value_type: ValueType::ArrayOfStrings,
                required: false,
                description: "List of project authors.",
                known_values: &[],
            },
            KeyDef {
                name: "shape-version",
                value_type: ValueType::Str,
                required: false,
                description: "Required Shape language version (e.g. \">=0.5.0\").",
                known_values: &[],
            },
            KeyDef {
                name: "license",
                value_type: ValueType::Str,
                required: false,
                description: "SPDX license identifier (e.g. \"MIT\", \"Apache-2.0\").",
                known_values: &[
                    "MIT",
                    "Apache-2.0",
                    "GPL-3.0",
                    "BSD-3-Clause",
                    "ISC",
                    "Unlicense",
                ],
            },
            KeyDef {
                name: "repository",
                value_type: ValueType::Str,
                required: false,
                description: "URL of the project's source repository.",
                known_values: &[],
            },
        ],
    },
    SectionDef {
        name: "modules",
        description: "Module resolution configuration.",
        is_array_table: false,
        is_free_form: false,
        keys: &[KeyDef {
            name: "paths",
            value_type: ValueType::ArrayOfStrings,
            required: false,
            description: "Additional directories to search for modules (relative to project root).",
            known_values: &[],
        }],
    },
    SectionDef {
        name: "dependencies",
        description: "Project dependencies. Each key is a package name, value is a version string or detailed table.",
        is_array_table: false,
        is_free_form: true,
        keys: &[],
    },
    SectionDef {
        name: "dev-dependencies",
        description: "Development-only dependencies (not included in production builds).",
        is_array_table: false,
        is_free_form: true,
        keys: &[],
    },
    SectionDef {
        name: "build",
        description: "Build configuration.",
        is_array_table: false,
        is_free_form: false,
        keys: &[
            KeyDef {
                name: "target",
                value_type: ValueType::Str,
                required: false,
                description: "Build target: \"bytecode\" or \"native\".",
                known_values: &["bytecode", "native"],
            },
            KeyDef {
                name: "opt_level",
                value_type: ValueType::Integer,
                required: false,
                description: "Optimization level (0-3). Higher is more optimized.",
                known_values: &["0", "1", "2", "3"],
            },
            KeyDef {
                name: "output",
                value_type: ValueType::Str,
                required: false,
                description: "Output directory for build artifacts.",
                known_values: &[],
            },
        ],
    },
    SectionDef {
        name: "extensions",
        description: "Extension module libraries to load. Each `[[extensions]]` entry defines one module.",
        is_array_table: true,
        is_free_form: false,
        keys: &[
            KeyDef {
                name: "name",
                value_type: ValueType::Str,
                required: true,
                description: "Name of the extension module.",
                known_values: &[],
            },
            KeyDef {
                name: "path",
                value_type: ValueType::Str,
                required: true,
                description: "Path to the shared library (.so/.dylib/.dll).",
                known_values: &[],
            },
            KeyDef {
                name: "config",
                value_type: ValueType::Table,
                required: false,
                description: "Module-specific configuration table.",
                known_values: &[],
            },
        ],
    },
];

/// Look up a section definition by name (case-sensitive).
pub fn find_section(name: &str) -> Option<&'static SectionDef> {
    SECTIONS.iter().find(|s| s.name == name)
}

/// Look up a key definition within a section.
pub fn find_key(section_name: &str, key_name: &str) -> Option<&'static KeyDef> {
    find_section(section_name).and_then(|s| s.keys.iter().find(|k| k.name == key_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_section() {
        assert!(find_section("project").is_some());
        assert!(find_section("dependencies").is_some());
        assert!(find_section("nonexistent").is_none());
    }

    #[test]
    fn test_find_key() {
        let key = find_key("project", "name");
        assert!(key.is_some());
        let key = key.unwrap();
        assert!(key.required);
        assert_eq!(key.value_type, ValueType::Str);
    }

    #[test]
    fn test_all_sections_present() {
        let names: Vec<&str> = SECTIONS.iter().map(|s| s.name).collect();
        assert!(names.contains(&"project"));
        assert!(names.contains(&"modules"));
        assert!(names.contains(&"dependencies"));
        assert!(names.contains(&"dev-dependencies"));
        assert!(names.contains(&"build"));
        assert!(names.contains(&"extensions"));
    }

    #[test]
    fn test_extensions_is_array_table() {
        let ext = find_section("extensions").unwrap();
        assert!(ext.is_array_table);
    }

    #[test]
    fn test_dependencies_is_free_form() {
        let deps = find_section("dependencies").unwrap();
        assert!(deps.is_free_form);
        let dev_deps = find_section("dev-dependencies").unwrap();
        assert!(dev_deps.is_free_form);
    }
}
