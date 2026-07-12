//! The 9-test frozen-identity matrix (ongoing identity-stability tripwire).
//!
//! S2: constructions go through the single freeze barrier
//! (`SemanticFreeze::freeze` over a populated compiler) — the old
//! `TypeReflectionSnapshot::default()` + field-poking pattern is deleted
//! with the per-site rebuild. Identity semantics under test are unchanged.

use super::*;
use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::semantic_freeze::{FreezeOverlay, SemanticFreeze};
use std::sync::Arc;

fn freeze_of(configure: impl FnOnce(&mut BytecodeCompiler)) -> Arc<SemanticFreeze> {
    let mut compiler = BytecodeCompiler::new();
    configure(&mut compiler);
    SemanticFreeze::freeze(&compiler).expect("test compiler state must freeze")
}

fn add_alias(compiler: &mut BytecodeCompiler, alias: &str, target: &str) {
    compiler
        .type_aliases
        .insert(alias.to_string(), target.to_string());
}

fn add_struct(compiler: &mut BytecodeCompiler, name: &str) {
    compiler
        .struct_types
        .insert(name.to_string(), (Vec::new(), shape_ast::ast::Span::DUMMY));
}

#[test]
fn aliases_reuse_stable_underlying_identity() {
    let freeze = freeze_of(|compiler| add_alias(compiler, "UserId", "int"));
    let int_identity = freeze.identity_of("int").expect("int identity");
    assert_eq!(freeze.identity_of("UserId"), Some(int_identity));

    // Adding an unrelated nominal cannot renumber an existing identity.
    let grown = freeze_of(|compiler| {
        add_alias(compiler, "UserId", "int");
        add_struct(compiler, "Unrelated");
    });
    assert_eq!(grown.identity_of("int"), Some(int_identity));
    assert_eq!(grown.identity_of("UserId"), Some(int_identity));
}

#[test]
fn declared_parameter_receives_parameter_identity() {
    let freeze = freeze_of(|_| {});
    let overlay = FreezeOverlay::new(freeze, "map", &["T".to_string()]);

    let identity = overlay.identity_of("T").expect("T identity");
    assert_eq!(
        overlay.category_of(identity),
        Ok(FrozenTypeCategory::Parameter)
    );
}

#[test]
fn active_function_type_parameters_are_discovered_from_compiler_state() {
    let program = shape_ast::parse_program("fn identity<T>(value: T) -> T { value }")
        .expect("generic function parses");
    let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
        panic!("expected function item");
    };
    let mut compiler = BytecodeCompiler::new();
    compiler
        .register_function(definition)
        .expect("generic signature registers");
    compiler.current_function = compiler
        .program
        .functions
        .iter()
        .position(|function| function.name == "identity");
    compiler
        .install_semantic_freeze()
        .expect("registration-complete state freezes");

    let overlay = compiler
        .comptime_freeze_overlay()
        .expect("post-barrier site obtains the handle");
    let identity = overlay.identity_of("T").expect("active T identity");
    assert_eq!(
        overlay.category_of(identity),
        Ok(FrozenTypeCategory::Parameter)
    );
}

#[test]
fn reachable_builtin_categories_match_the_closed_catalog() {
    let freeze = freeze_of(|_| {});

    for (name, expected) in [
        ("int", FrozenTypeCategory::Primitive),
        ("never", FrozenTypeCategory::Never),
        ("any", FrozenTypeCategory::Erased),
        ("Array", FrozenTypeCategory::Nominal),
    ] {
        let identity = freeze.identity_of(name).expect("known type identity");
        assert_eq!(freeze.category_of(identity), Ok(expected));
    }
}

#[test]
fn primitive_synonyms_share_identity_but_distinct_types_do_not() {
    let freeze = freeze_of(|_| {});

    assert_eq!(freeze.identity_of("int"), freeze.identity_of("i64"));
    assert_eq!(freeze.identity_of("number"), freeze.identity_of("f64"));
    assert_eq!(freeze.identity_of("unit"), freeze.identity_of("void"));
    assert_ne!(freeze.identity_of("int"), freeze.identity_of("bool"));
}

#[test]
fn alias_chains_reuse_the_terminal_identity() {
    let freeze = freeze_of(|compiler| {
        add_alias(compiler, "First", "Second");
        add_alias(compiler, "Second", "int");
    });

    let int_identity = freeze.identity_of("int");
    assert_eq!(freeze.identity_of("First"), int_identity);
    assert_eq!(freeze.identity_of("Second"), int_identity);
}

#[test]
fn nominal_identity_is_order_independent_stable_and_distinct() {
    let left = freeze_of(|compiler| {
        add_struct(compiler, "Alpha");
        add_struct(compiler, "Beta");
    });
    let right = freeze_of(|compiler| {
        add_struct(compiler, "Beta");
        add_struct(compiler, "Alpha");
    });

    assert_eq!(left.identity_of("Alpha"), right.identity_of("Alpha"));
    assert_eq!(left.identity_of("Beta"), right.identity_of("Beta"));
    assert_ne!(left.identity_of("Alpha"), left.identity_of("Beta"));

    let alpha = left.identity_of("Alpha");
    let grown = freeze_of(|compiler| {
        add_struct(compiler, "Alpha");
        add_struct(compiler, "Beta");
        add_struct(compiler, "Unrelated");
    });
    assert_eq!(grown.identity_of("Alpha"), alpha);
}

#[test]
fn parameter_identity_is_scoped_by_owning_function() {
    let parameter_identity = |owner: &str| {
        let overlay = FreezeOverlay::new(freeze_of(|_| {}), owner, &["T".to_string()]);
        overlay.identity_of("T").expect("T identity")
    };

    assert_eq!(parameter_identity("map"), parameter_identity("map"));
    assert_ne!(parameter_identity("map"), parameter_identity("filter"));
}

#[test]
fn unknown_identity_is_rejected_at_the_freeze_boundary() {
    let freeze = freeze_of(|_| {});

    assert_eq!(
        freeze.category_of(FrozenTypeIdentity::INVALID),
        Err("type_ref received an unknown semantic type identity".to_string())
    );
}

/// ADR-009 §4.1 "one kind vocabulary" (ticket A1, slice S5): confinement
/// sentinel for the legacy `type_info` vocabulary.
///
/// `TypeKindLabel` / `classify_legacy_type_info` / `build_type_info_heap_value`
/// and the `__ComptimeTypeInfo` schema survive ONLY on the legacy `type_info`
/// intrinsic path until ticket E5 deletes them. This sentinel (file-read, same
/// pattern as `executor/tests/no_dynamic.rs`) pins that confinement so the
/// vocabulary cannot silently re-spread into the semantic-freeze module, the
/// shared runtime reflection catalog, or a crate-wide re-export before E5.
#[test]
fn legacy_type_info_vocabulary_is_confined_to_the_legacy_intrinsic_path() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |relative: &str| {
        std::fs::read_to_string(manifest.join(relative))
            .unwrap_or_else(|error| panic!("sentinel could not read {relative}: {error}"))
    };

    const LEGACY_VOCABULARY: [&str; 4] = [
        "TypeKindLabel",
        "classify_legacy_type_info",
        "build_type_info_heap_value",
        "__ComptimeTypeInfo",
    ];

    // 1. The semantic-freeze module and the shared runtime catalog must not
    //    mention the legacy vocabulary at all (not even in comments — the new
    //    surface describes the legacy path by ticket, never by symbol).
    for relative in [
        "src/compiler/comptime_builtins/semantic_freeze.rs",
        "../shape-runtime/src/comptime_reflection.rs",
    ] {
        let source = read(relative);
        for symbol in LEGACY_VOCABULARY {
            assert!(
                !source.contains(symbol),
                "legacy type_info vocabulary `{symbol}` leaked into {relative} \
                 (E5-deletes territory; confined to the legacy type_info path)"
            );
        }
    }

    // 2. No crate-wide re-export: in `comptime_builtins.rs`,
    //    `build_type_info_heap_value` may appear only path-qualified at the
    //    single legacy `type_info` intrinsic call site (or in comments),
    //    never inside a `use` list.
    let parent = read("src/compiler/comptime_builtins.rs");
    for (index, _) in parent.match_indices("build_type_info_heap_value") {
        let qualified = parent[..index].ends_with("type_reflection::");
        let line_start = parent[..index].rfind('\n').map_or(0, |p| p + 1);
        let comment = parent[line_start..index].trim_start().starts_with("//");
        assert!(
            qualified || comment,
            "`build_type_info_heap_value` must stay confined: every non-comment \
             use in comptime_builtins.rs must be path-qualified \
             `type_reflection::build_type_info_heap_value` at the legacy \
             type_info intrinsic (found an unqualified occurrence, e.g. a \
             crate-wide `use` re-export)"
        );
    }

    // 3. Visibility confinement: the legacy builder is `pub(super)` (reachable
    //    from the parent intrinsic-registration module only), never
    //    `pub(crate)`.
    let reflection = read("src/compiler/comptime_builtins/type_reflection.rs");
    assert!(
        reflection.contains("pub(super) fn build_type_info_heap_value"),
        "build_type_info_heap_value must be pub(super) (legacy-path confinement)"
    );

    // 4. E5-deletes markers: each confined symbol's definition site carries
    //    the `E5-deletes` confinement marker for grep visibility.
    assert!(
        reflection.contains("E5-deletes"),
        "type_reflection.rs must carry the E5-deletes confinement marker"
    );
    let schemas = read("../shape-runtime/src/type_schema/builtin_schemas.rs");
    assert!(
        schemas.contains("E5-deletes"),
        "builtin_schemas.rs __ComptimeTypeInfo registration must carry the \
         E5-deletes confinement marker"
    );
}
