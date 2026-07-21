//! Absence sentinel for the ADR-009 C3 #14 slice-6 pure deletion (C3-G7).
//!
//! The S6 capstone deleted the legacy runtime-annotation weave whole
//! (deletion commit on `adr009/c3`): the specialized runtime-handler
//! compiler (self-as-f64 receiver + the name-keyed args/result/ctx magic
//! params), the per-target runtime-handler specializer, the raw-bytecode
//! wrapper emitter (homogeneous args array, per-invocation config eval, the
//! `{result:}` before-short-circuit), the homogeneous-args-array derivation
//! and both of its heterogeneous hard errors, the per-annotation compiled
//! handler-id slots and the per-target handler-template AST-clone carrier
//! fields, and the S4 transitional surface classification whose `Legacy`
//! arm routed all of it. Runtime `before`/`after` hooks have exactly ONE
//! implementation: the typed hook-template weave (`template_specialization`
//! — sugar lowering onto the public comptime API, per-target
//! specialization, ordinary typed-AST wrapper through the ordinary
//! pipeline, bytecode AND MIR from the same wrapped definition).
//!
//! Additionally, C3-G14 (A′, user-ratified 2026-07-21) cut the `@remote`
//! annotation out of `std::core/remote.shape` — the module keeps its
//! builtin exports but must carry NO annotation declaration until E4
//! re-implements `@remote` on the typed HookDecision protocol (GitHub
//! issue #68).
//!
//! This guard fails the workspace build if any deleted symbol is
//! reintroduced. It mirrors `no_dynamic.rs` / `no_json_comptime_protocol.rs`:
//! it scans the source trees (`crates/`, `bin/`, `tools/`, `extensions/`)
//! at the Rust-test layer so the prohibition survives independently of any
//! shell gate. Documentation trees (`docs/`, `CLAUDE.md`, `AGENTS.md`)
//! intentionally discuss the deleted machinery by name and are NOT scanned.
//!
//! NOTE: like its precedents, this file must not itself contain any needle
//! contiguously (the scan reads every `.rs`, including this one). Each
//! needle is assembled from fragments at runtime; every comment and assert
//! message below describes the needles by part or by role only — never
//! spelled contiguously.

use std::path::{Path, PathBuf};

/// Walk the repo's source scope and collect every `.rs` file's contents.
fn source_rs_contents() -> Vec<(PathBuf, String)> {
    // CARGO_MANIFEST_DIR = <repo>/crates/shape-vm — repo root is two up.
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("repo root (two levels above crates/shape-vm)");

    let scope = ["crates", "bin", "tools", "extensions"];
    let mut out = Vec::new();
    for dir in scope {
        collect_rs(&repo_root.join(dir), &mut out);
    }
    out
}

fn collect_rs(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip build artifacts.
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            if let Ok(src) = std::fs::read_to_string(&path) {
                out.push((path, src));
            }
        }
    }
}

fn files_containing(sources: &[(PathBuf, String)], needle: &str) -> Vec<PathBuf> {
    sources
        .iter()
        .filter(|(_, src)| src.contains(needle))
        .map(|(p, _)| p.clone())
        .collect()
}

/// The deleted legacy weave FN NAMES must not reappear (as definitions,
/// calls, or any other contiguous spelling — the S6 tombstones deliberately
/// describe them by deletion-fate, never by name). Needles are reassembled
/// from fragments at runtime.
#[test]
fn no_legacy_weave_functions() {
    let sources = source_rs_contents();
    // Each entry: (fragment-a, fragment-b) — the needle is a ++ b.
    let split_names = [
        // The specialized runtime-handler compiler (self-as-f64 + the
        // name-keyed magic params).
        ("compile_specialized_", "annotation_handler"),
        // The per-target runtime-handler specializer (the sole reader of
        // the deleted handler-template carrier fields).
        ("specialize_annotation_", "runtime_handlers"),
        // The raw-bytecode wrapper emitter (homogeneous args array,
        // per-invocation config eval, the before-object short-circuit).
        ("compile_annotation_", "wrapper"),
        // The homogeneous-args-array derivation pair (the two heterogeneous
        // hard errors lived here).
        ("annotation_arg_array_", "element_annotation"),
        ("annotation_arg_array_", "kind"),
    ];
    for (a, b) in split_names {
        let needle = [a, b].concat();
        let hits = files_containing(&sources, &needle);
        assert!(
            hits.is_empty(),
            "A deleted legacy-weave fn name (assembled: {a}…{b}) was deleted in \
             ADR-009 C3 #14 slice 6 (C3-G7 — the typed hook-template weave is \
             the ONE implementation). It must not be reintroduced under the \
             same spelling. Found in: {hits:#?}"
        );
    }
}

/// The deleted `CompiledAnnotation` carrier slots must not reappear: the
/// compiled handler-id fields (needle = field declaration with its exact
/// `Option<u16>` type, so the unrelated import-permissions test fn whose
/// snake_case name embeds the field spelling does not trip it) and the
/// per-target handler-template AST-clone fields (bare name — nothing may
/// spell them at all).
#[test]
fn no_legacy_handler_carrier_fields() {
    let sources = source_rs_contents();
    for (a, b) in [
        ("before_handler: ", "Option<u16>"),
        ("after_handler: ", "Option<u16>"),
        ("before_handler_", "template"),
        ("after_handler_", "template"),
    ] {
        let needle = [a, b].concat();
        let hits = files_containing(&sources, &needle);
        assert!(
            hits.is_empty(),
            "A deleted legacy handler carrier slot (assembled: {a}…{b}) was \
             deleted from `CompiledAnnotation` in ADR-009 C3 #14 slice 6 — \
             declarative hooks travel exclusively on the sugar carrier \
             (`sugar_post_handler` + `sugar_body_fns`). It must not be \
             reintroduced. Found in: {hits:#?}"
        );
    }
}

/// The S4 transitional surface classification must not reappear: no code may
/// construct or match a variant of the deleted evidence enum (needle = the
/// enum name followed by the path separator, so the surviving prose tombstone
/// that names the bare enum in backticks does not trip it).
#[test]
fn no_legacy_surface_classification() {
    let sources = source_rs_contents();
    let needle = ["AnnotationSurface", "Class::"].concat();
    let hits = files_containing(&sources, &needle);
    assert!(
        hits.is_empty(),
        "The S4 transitional surface-classification enum (assembled name shows \
         in the needle) was deleted at the ADR-009 C3-S6 classification \
         collapse — one annotation surface, one named untyped-config \
         declaration rejection. It must not be reintroduced. Found in: {hits:#?}"
    );
}

/// C3-G14 (A′): `std::core/remote.shape` carries NO annotation declaration
/// during the dark window — `@remote` returns only as E4's typed
/// HookDecision consumer (#68). The keep-set (the `remote.execute` builtin
/// export) must stay present, which also proves the right file was read.
#[test]
fn remote_stdlib_module_carries_no_annotation_block() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let remote_shape = manifest
        .parent()
        .expect("crates/ dir above shape-vm")
        .join("shape-runtime")
        .join("stdlib-src")
        .join("core")
        .join("remote.shape");
    let src = std::fs::read_to_string(&remote_shape)
        .unwrap_or_else(|e| panic!("std::core remote module must exist at {remote_shape:?}: {e}"));

    // Positive keep-set guard (non-vacuity): the live builtin surface.
    assert!(
        src.contains("pub builtin fn execute"),
        "remote.shape keep-set regressed: the `remote.execute` builtin export \
         must stay live through the dark window (C3-G14 A′)"
    );

    // The dark window: no annotation declaration of any name, and no
    // spelling of the deleted declaration head. Both needles assembled.
    for (a, b) in [("pub ", "annotation "), ("annotation ", "remote(")] {
        let needle = [a, b].concat();
        assert!(
            !src.contains(&needle),
            "std::core/remote.shape declares an annotation again (needle \
             assembled: {a}…{b}) — the C3-G14 A′ dark window says `@remote` \
             comes back ONLY as E4's typed HookDecision re-implementation \
             (GitHub issue #68), not by resurrecting the deleted legacy \
             declaration in the stdlib module"
        );
    }
}
