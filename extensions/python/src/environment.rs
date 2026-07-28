//! The environment this runtime was DECLARED to run in (ADR-019 §4 / #198).
//!
//! Everything in this module is a consequence of one rule: the interpreter's
//! import surface is decided by the project's `[foreign.python]` declaration and
//! by nothing else. There is no discovery step, no fallback order, and no
//! "if the host happens to have one" branch — the previous constructor had all
//! three, and they are what made a `fn python` body mean different things on
//! different machines.
//!
//! The host resolves the declaration, verifies the environment is present, and
//! passes the resolved absolute paths through the `init` config. This module
//! reads them and adds exactly those to `sys.path`. A host that resolved
//! nothing passes nothing, and then nothing is added — the base interpreter,
//! which is a real declaration and not a degraded one.

/// The key the host hands environments under.
///
/// Mirrors `shape_runtime::project::FOREIGN_ENVIRONMENTS_CONFIG_KEY`. Spelled
/// again rather than shared because this crate is an EXTENSION: it links
/// against `shape-abi-v1` only, which is what lets an extension be built
/// out-of-tree. `config_carries_the_hosts_key` pins the two spellings together.
pub const FOREIGN_ENVIRONMENTS_CONFIG_KEY: &str = "shape_foreign_environments";

/// This runtime's declared environment, as the host resolved it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeclaredEnvironment {
    /// The environment root, absolute. `None` = base interpreter.
    pub root: Option<String>,
    /// Absolute paths to prepend to `sys.path`, in declared order.
    pub search_paths: Vec<String>,
    /// The environment digest, for diagnostics.
    pub digest: Option<String>,
}

impl DeclaredEnvironment {
    /// Read the declared environment out of the host's `init` config.
    ///
    /// Total: an undecodable, absent, or malformed config yields the empty
    /// environment. That is not leniency covering for a bug — the empty
    /// environment adds no path, so the worst a garbled config can do is leave
    /// the base interpreter in place. The alternative, guessing at a partially
    /// understood config, is how an extension ends up activating something
    /// nobody declared.
    pub fn from_config(config_msgpack: &[u8]) -> Self {
        let Ok(config) = rmp_serde::from_slice::<serde_json::Value>(config_msgpack) else {
            return Self::default();
        };
        let Some(env) = config
            .get(FOREIGN_ENVIRONMENTS_CONFIG_KEY)
            .and_then(|envs| envs.get("python"))
        else {
            return Self::default();
        };
        Self {
            root: env.get("root").and_then(|v| v.as_str()).map(str::to_string),
            search_paths: env
                .get("search_paths")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            digest: env
                .get("digest")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }
    }

    /// Whether the host declared anything for this runtime.
    pub fn is_empty(&self) -> bool {
        self.root.is_none() && self.search_paths.is_empty()
    }

    /// Apply the declared environment to the live interpreter.
    ///
    /// Prepends the declared search paths to `sys.path` and, when a root was
    /// declared, points `sys.prefix` / `sys.exec_prefix` at it — the two things
    /// `source .venv/bin/activate` does that matter to `import`.
    ///
    /// `site.addsitedir` is deliberately NOT used: it processes `.pth` files,
    /// which can append further directories of their own choosing, and a
    /// declared environment that grows extra search paths at activation time is
    /// not the environment that was declared.
    #[cfg(feature = "pyo3")]
    pub fn activate(&self) -> Result<(), String> {
        use pyo3::prelude::*;

        if self.is_empty() {
            return Ok(());
        }
        Python::attach(|py| {
            let sys = py
                .import("sys")
                .map_err(|e| format!("declared environment: cannot import sys: {e}"))?;
            if let Some(root) = &self.root {
                sys.setattr("prefix", root.as_str())
                    .and_then(|()| sys.setattr("exec_prefix", root.as_str()))
                    .map_err(|e| format!("declared environment: cannot set sys.prefix: {e}"))?;
            }
            let path = sys
                .getattr("path")
                .map_err(|e| format!("declared environment: cannot read sys.path: {e}"))?;
            // Reverse, because each insert goes to index 0: the declared order
            // ends up as the search order.
            for entry in self.search_paths.iter().rev() {
                path.call_method1("insert", (0, entry.as_str()))
                    .map_err(|e| {
                        format!("declared environment: cannot extend sys.path with {entry}: {e}")
                    })?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(value: serde_json::Value) -> Vec<u8> {
        rmp_serde::to_vec(&value).expect("encodes")
    }

    #[test]
    fn a_declared_environment_is_read_whole() {
        let bytes = config(serde_json::json!({
            FOREIGN_ENVIRONMENTS_CONFIG_KEY: {
                "python": {
                    "runtime": "cpython",
                    "version": "3.11.7",
                    "digest": "abc123",
                    "root": "/proj/.venv",
                    "search_paths": ["/proj/.venv/lib/python3.11/site-packages"],
                }
            }
        }));
        let declared = DeclaredEnvironment::from_config(&bytes);
        assert_eq!(declared.root.as_deref(), Some("/proj/.venv"));
        assert_eq!(
            declared.search_paths,
            vec!["/proj/.venv/lib/python3.11/site-packages".to_string()]
        );
        assert_eq!(declared.digest.as_deref(), Some("abc123"));
        assert!(!declared.is_empty());
    }

    #[test]
    fn another_languages_environment_is_not_ours() {
        let bytes = config(serde_json::json!({
            FOREIGN_ENVIRONMENTS_CONFIG_KEY: {
                "typescript": { "root": "/proj/vendor", "search_paths": ["/proj/vendor"] }
            }
        }));
        assert!(DeclaredEnvironment::from_config(&bytes).is_empty());
    }

    #[test]
    fn an_absent_declaration_is_the_empty_environment() {
        assert!(DeclaredEnvironment::from_config(&config(serde_json::json!({}))).is_empty());
        assert!(DeclaredEnvironment::from_config(&[]).is_empty());
        assert!(DeclaredEnvironment::from_config(b"not msgpack at all").is_empty());
    }

    #[test]
    fn a_malformed_declaration_adds_no_paths() {
        // Wrong shapes throughout: the result must still be "add nothing",
        // never "add something we half-understood".
        let bytes = config(serde_json::json!({
            FOREIGN_ENVIRONMENTS_CONFIG_KEY: {
                "python": { "root": 7, "search_paths": "not-an-array" }
            }
        }));
        let declared = DeclaredEnvironment::from_config(&bytes);
        assert!(declared.is_empty(), "got: {declared:?}");
    }

    /// The negative control for the deleted sniffer (ADR-019 §4 / #198).
    ///
    /// Stages exactly what `activate_virtualenv` used to find — a `.venv`
    /// directory in the working directory, a `venv` beside it, a
    /// `pyrightconfig.json` pointing at a third, and `$VIRTUAL_ENV` naming a
    /// fourth — and then initializes the runtime with an EMPTY config. Not one
    /// of the four may reach `sys.path`.
    ///
    /// This is what proves the fallback is gone rather than merely unused: it
    /// re-creates the discovery inputs, not the discovery call.
    #[cfg(feature = "pyo3")]
    #[test]
    fn an_ambient_venv_does_not_reach_sys_path() {
        use pyo3::prelude::*;

        let stage = std::env::temp_dir().join(format!(
            "shape-198-ambient-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for name in [
            ".venv/lib/python3.11/site-packages",
            "venv/lib/python3.11/site-packages",
            "pyright-venv/lib/python3.11/site-packages",
            "env-venv/lib/python3.11/site-packages",
        ] {
            std::fs::create_dir_all(stage.join(name)).expect("stage the decoy");
        }
        std::fs::write(
            stage.join("pyrightconfig.json"),
            r#"{"venvPath": ".", "venv": "pyright-venv"}"#,
        )
        .expect("stage pyrightconfig");

        let previous_dir = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&stage).expect("chdir into the staged tree");
        // SAFETY: single-threaded within this test's own body; restored below.
        // The variable is one of the four inputs the deleted sniffer read, so
        // the control is not honest without it.
        unsafe { std::env::set_var("VIRTUAL_ENV", stage.join("env-venv")) };

        let runtime = crate::runtime::PythonRuntime::new(&[]);

        let paths: Vec<String> = Python::attach(|py| {
            py.import("sys")
                .and_then(|sys| sys.getattr("path"))
                .and_then(|path| path.extract::<Vec<String>>())
                .expect("read sys.path")
        });

        // SAFETY: same single-threaded window.
        unsafe { std::env::remove_var("VIRTUAL_ENV") };
        std::env::set_current_dir(&previous_dir).expect("restore cwd");
        let _ = std::fs::remove_dir_all(&stage);

        assert!(runtime.is_ok(), "an empty config is a valid declaration");
        let stage_prefix = stage.display().to_string();
        let leaked: Vec<&String> = paths
            .iter()
            .filter(|p| p.starts_with(&stage_prefix))
            .collect();
        assert!(
            leaked.is_empty(),
            "an undeclared ambient environment reached sys.path: {leaked:?}"
        );
    }

    /// The positive control the negative one needs to mean anything: a
    /// DECLARED path does reach `sys.path`, so "nothing reached it" above is a
    /// statement about the declaration and not about a broken activator.
    #[cfg(feature = "pyo3")]
    #[test]
    fn a_declared_path_does_reach_sys_path() {
        use pyo3::prelude::*;

        let declared_dir = std::env::temp_dir().join(format!(
            "shape-198-declared-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&declared_dir).expect("stage the declared path");
        let declared_str = declared_dir.display().to_string();

        let config = config(serde_json::json!({
            FOREIGN_ENVIRONMENTS_CONFIG_KEY: {
                "python": { "search_paths": [declared_str.clone()] }
            }
        }));
        crate::runtime::PythonRuntime::new(&config).expect("declared environment activates");

        let paths: Vec<String> = Python::attach(|py| {
            py.import("sys")
                .and_then(|sys| sys.getattr("path"))
                .and_then(|path| path.extract::<Vec<String>>())
                .expect("read sys.path")
        });
        let _ = std::fs::remove_dir_all(&declared_dir);
        assert!(
            paths.contains(&declared_str),
            "declared path missing from sys.path: {declared_str}"
        );
    }

    #[test]
    fn config_carries_the_hosts_key() {
        // The host's constant is `shape_runtime::project::
        // FOREIGN_ENVIRONMENTS_CONFIG_KEY`; an extension cannot depend on
        // shape-runtime, so the two spellings are pinned here instead.
        assert_eq!(
            FOREIGN_ENVIRONMENTS_CONFIG_KEY,
            "shape_foreign_environments"
        );
    }
}
