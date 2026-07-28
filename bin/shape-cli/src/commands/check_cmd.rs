use anyhow::Result;
use std::path::PathBuf;

/// Run `shape check [path]` — validate a Shape file or project without executing.
pub async fn run_check(path: Option<PathBuf>, link: bool, fix: bool) -> Result<()> {
    let path = match path {
        Some(p) => p,
        None => std::env::current_dir()?,
    };

    let (source, display_path) = if path.is_dir() {
        // Project directory — find entry point from shape.toml
        let project = shape_runtime::project::find_project_root(&path)
            .ok_or_else(|| anyhow::anyhow!("No shape.toml found in '{}'", path.display()))?;

        let entry = project.config.project.entry.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "shape.toml at '{}' has no [project].entry field",
                project.root_path.join("shape.toml").display()
            )
        })?;

        let entry_path = project.root_path.join(entry);
        let src = std::fs::read_to_string(&entry_path)
            .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", entry_path.display(), e))?;
        (src, entry_path)
    } else {
        let src = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("Failed to read '{}': {}", path.display(), e))?;
        (src, path)
    };

    let mut errors = 0u32;
    let warnings = 0u32;

    // Strip optional script front-matter (shebang + `--- TOML ---` block) so a
    // script that declares `[native-dependencies]` / `[permissions]` in
    // front-matter is checkable — the front-matter block is not Shape source
    // and would otherwise fail to parse. Mirrors `shape run` (ffi-rebuild
    // §4.10 S6). `--link` alias resolution (`resolve_library_target`) is the
    // same built-in-table path `shape run` links through, so stripping the
    // front-matter is all `check` needs to match a real run's link behavior.
    let (_frontmatter, source) = shape_runtime::frontmatter::parse_frontmatter(&source);
    let source = source.to_string();

    // Parse
    match shape_ast::parse_program(&source) {
        Ok(ast) => {
            // Compile (type-check) without executing.
            //
            // `set_source_with_file` is what lets the checker bind a proved fix
            // to a source revision (ADR-017 §4): a fix carries byte spans plus a
            // digest of the text they were proved against, and a compiler with no
            // source has neither. Without this, `--fix` would work under
            // `shape run` and silently offer nothing under `shape check` — the
            // #179 residual this closes.
            let mut compiler = shape_vm::compiler::BytecodeCompiler::new();
            compiler.set_source_with_file(&source, &display_path.to_string_lossy());
            match compiler.compile(&ast) {
                Ok(bytecode) => {
                    // WF-2A stage 1: `--link` additionally verifies that every
                    // foreign function resolves (extern C: dlopen + symbol
                    // resolution), reporting ALL failures. Executes nothing.
                    if link {
                        let mut vm = shape_vm::executor::VirtualMachine::new(
                            shape_vm::executor::VMConfig::default(),
                        );
                        vm.load_program(bytecode);
                        if let Err(link_errors) = vm.eager_link_all() {
                            for le in &link_errors {
                                errors += 1;
                                eprintln!(
                                    "\x1b[31merror\x1b[0m: link: {} ({})",
                                    le,
                                    display_path.display()
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    errors += 1;
                    eprintln!("\x1b[31merror\x1b[0m: {} ({})", e, display_path.display());
                    if fix {
                        match apply_proved_fixes(&e, &source, &display_path) {
                            Ok(applied) if applied > 0 => {
                                eprintln!(
                                    "\x1b[32mfixed\x1b[0m: applied {applied} fix(es) to {}; \
                                     re-run `shape check` to confirm",
                                    display_path.display()
                                );
                            }
                            Ok(_) => {}
                            Err(refusal) => eprintln!(
                                "\x1b[33mnot fixed\x1b[0m: {} ({})",
                                refusal,
                                display_path.display()
                            ),
                        }
                    }
                }
            }
        }
        Err(e) => {
            errors += 1;
            eprintln!("\x1b[31merror\x1b[0m: {} ({})", e, display_path.display());
        }
    }

    // Summary
    if errors == 0 && warnings == 0 {
        eprintln!(
            "\x1b[32mcheck passed\x1b[0m: {} (no errors)",
            display_path.display()
        );
        Ok(())
    } else {
        eprintln!(
            "\x1b[31mcheck failed\x1b[0m: {} error(s), {} warning(s)",
            errors, warnings
        );
        std::process::exit(1);
    }
}

/// Apply every machine-applicable fix the compiler proved for `err`, rewriting
/// `path` in place. Returns how many were applied.
///
/// The compiler is the only authority on what a fix is: this reads
/// `EditPlan`s off the diagnostics and applies them through
/// `EditPlan::apply`, the same entry point the LSP projects onto `TextEdit`s.
/// Nothing here derives an edit, adjusts a span, or reinterprets a message.
///
/// Fixes are applied one at a time, each re-proved against the text as it
/// stands, because applying one invalidates the digests of the rest. A plan
/// that no longer matches is skipped, not forced — a stale fix misapplied is
/// worse than a fix not applied.
fn apply_proved_fixes(
    err: &shape_runtime::error::ShapeError,
    source: &str,
    path: &std::path::Path,
) -> Result<usize> {
    let diagnostics = crate::diagnostics_json::shape_error_to_diagnostics(err);
    let mut text = source.to_string();
    let mut applied = 0usize;

    for diagnostic in &diagnostics {
        for suggested in &diagnostic.fixes {
            let Some(plan) = suggested.edit_plan.as_ref() else {
                continue;
            };
            match plan.apply(&text) {
                Ok(next) => {
                    text = next;
                    applied += 1;
                    eprintln!("  applied: {}", suggested.label);
                }
                // A plan proved against a revision this text no longer is.
                // Expected once an earlier fix has edited the buffer.
                Err(_) => continue,
            }
        }
    }

    if applied > 0 {
        std::fs::write(path, &text)
            .map_err(|e| anyhow::anyhow!("failed to write '{}': {}", path.display(), e))?;
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program whose match is missing one variant — the one diagnostic that
    /// ships a proved fix today, so it is what proves the wiring.
    const NON_EXHAUSTIVE: &str = "enum Status { Active, Inactive }\n\
                                  fn describe(s: Status) -> string {\n\
                                  \x20 match s {\n\
                                  \x20   Status::Active => \"on\",\n\
                                  \x20 }\n\
                                  }\n";

    /// Compile the way `run_check` does.
    fn check_like_run_check(
        source: &str,
        file: &str,
    ) -> Result<(), shape_runtime::error::ShapeError> {
        let program = shape_ast::parse_program(source).expect("parse");
        let mut compiler = shape_vm::compiler::BytecodeCompiler::new();
        compiler.set_source_with_file(source, file);
        compiler.compile(&program).map(|_| ())
    }

    /// The #179 residual: `check` built its compiler without a source, so a
    /// proved fix had nothing to bind to and `--fix` would have been a no-op
    /// here while working under `shape run`. Assert the diagnostic `check`
    /// produces actually carries an applicable plan.
    #[test]
    fn check_binds_proved_fixes_to_the_source_it_checked() {
        let err = check_like_run_check(NON_EXHAUSTIVE, "check.shape").expect_err("must fail");
        let diagnostics = crate::diagnostics_json::shape_error_to_diagnostics(&err);

        let plan = diagnostics
            .iter()
            .flat_map(|diagnostic| diagnostic.fixes.iter())
            .find_map(|fix| fix.edit_plan.as_ref())
            .expect("the checker's fix reaches `shape check`, not only `shape run`");

        assert!(
            plan.validate(NON_EXHAUSTIVE).is_ok(),
            "the plan must be bound to the text `check` compiled"
        );
    }

    /// End to end: `--fix` rewrites the file, and the rewritten file checks
    /// clean. A fix that does not close its own diagnostic is not a fix.
    #[test]
    fn applying_the_fix_rewrites_the_file_and_it_then_checks_clean() {
        let dir = std::env::temp_dir().join(format!(
            "shape-check-fix-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let file = dir.join("main.shape");
        std::fs::write(&file, NON_EXHAUSTIVE).expect("write");

        let err = check_like_run_check(NON_EXHAUSTIVE, &file.to_string_lossy()).expect_err("fails");
        let applied = apply_proved_fixes(&err, NON_EXHAUSTIVE, &file).expect("apply");
        assert_eq!(applied, 1);

        let fixed = std::fs::read_to_string(&file).expect("read back");
        assert!(fixed.contains("Status::Inactive"), "{fixed}");
        assert!(
            check_like_run_check(&fixed, &file.to_string_lossy()).is_ok(),
            "the fixed source must check clean: {fixed}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A plan proved against a different revision is skipped, not forced onto
    /// whatever bytes happen to be at those offsets.
    #[test]
    fn a_stale_plan_is_skipped_and_nothing_is_written() {
        let dir = std::env::temp_dir().join(format!(
            "shape-check-fix-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let file = dir.join("main.shape");
        let edited = format!("// the user typed a line\n{NON_EXHAUSTIVE}");
        std::fs::write(&file, &edited).expect("write");

        // The error was proved against the ORIGINAL text.
        let err = check_like_run_check(NON_EXHAUSTIVE, &file.to_string_lossy()).expect_err("fails");
        let applied = apply_proved_fixes(&err, &edited, &file).expect("apply");

        assert_eq!(applied, 0, "a stale plan must not apply");
        assert_eq!(
            std::fs::read_to_string(&file).expect("read back"),
            edited,
            "and the file must be untouched"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
