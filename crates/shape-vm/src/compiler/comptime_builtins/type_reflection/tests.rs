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

// ADR-009 A3 (Wave-46 gap #4): compiling (not fabricating) a specialization of
// a generic function whose body reflects on its own declared type parameter
// must succeed — the declared base type params reach the reflection snapshot
// during the specialized-body compile at monomorphization time.
#[test]
fn specialized_generic_body_resolves_own_type_param_through_comptime() {
    // Control: the same comptime reflection at module scope compiles with a
    // bare BytecodeCompiler — proves any failure below is specialization-
    // specific, not test-harness setup.
    let control = r#"
let control_label = comptime {
  match type_category(type_ref(int)) {
    FrozenTypeCategory::Primitive => "primitive"
    _ => "other"
  }
}
"#;
    let control_program = shape_ast::parse_program(control).expect("control parses");
    let control_compiler = crate::compiler::BytecodeCompiler::new();
    control_compiler
        .compile(&control_program)
        .expect("module-scope comptime reflection compiles with a bare compiler");

    let source = r#"
fn describe<T>(value: T) -> string {
  let label = comptime {
    match type_category(type_ref(T)) {
      FrozenTypeCategory::Parameter => "parameter"
      _ => "other"
    }
  }
  label
}

let result = describe(1)
"#;
    let program = shape_ast::parse_program(source).expect("generic program parses");
    let compiler = crate::compiler::BytecodeCompiler::new();
    let bytecode = compiler
        .compile(&program)
        .expect("specialized generic body with type_ref(T) compiles");
    assert!(
        bytecode
            .functions
            .iter()
            .any(|function| function.name.starts_with("describe::")),
        "expected a registered specialization of 'describe', got: {:?}",
        bytecode
            .functions
            .iter()
            .map(|function| function.name.clone())
            .collect::<Vec<_>>()
    );
}

// ADR-009 A3 — the specialization overlay supplies the BASE generic function's
// declared type params to the freeze overlay when the active (mono) def
// carries `type_params = None`, and the Parameter identity is scoped to the
// BASE function name, never the mono key (declaration stability across
// instantiations). Post-A1 merge this goes through the single freeze barrier
// + `comptime_freeze_overlay` (the per-site snapshot builder is deleted).
#[test]
fn specialization_overlay_supplies_base_scoped_parameter_identity() {
    let program = shape_ast::parse_program("fn describe(value: int) -> int { value }")
        .expect("mono-shaped function parses");
    let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
        panic!("expected function item");
    };
    // Register under a mono-key name with type_params = None, exactly like a
    // substituted specialization.
    let mut mono_def = definition.clone();
    mono_def.name = "describe::i64".to_string();
    assert!(mono_def.type_params.is_none());
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    compiler
        .register_function(&mono_def)
        .expect("mono def registers");
    compiler.current_function = compiler
        .program
        .functions
        .iter()
        .position(|function| function.name == "describe::i64");
    compiler
        .install_semantic_freeze()
        .expect("registration-complete state freezes");

    // Without the overlay, the mono def exposes no type params.
    let bare = compiler
        .comptime_freeze_overlay()
        .expect("post-barrier site obtains the handle");
    assert_eq!(bare.identity_of("T"), None);

    // With the overlay, T resolves to a Parameter identity owned by the BASE name.
    compiler.specialization_type_param_overlay =
        Some(("describe".to_string(), vec!["T".to_string()]));
    let overlay = compiler
        .comptime_freeze_overlay()
        .expect("post-barrier site obtains the handle");
    let identity = overlay.identity_of("T").expect("overlay T identity");
    assert_eq!(
        overlay.category_of(identity),
        Ok(FrozenTypeCategory::Parameter)
    );

    // Identity contract: the canonical descriptor is scoped to the BASE fn
    // name, never the mono key (same `parameter:{owner}:{name}` grammar the
    // freeze overlay interns).
    assert_eq!(
        identity,
        FrozenTypeIdentity::from_canonical_descriptor("parameter:describe:T"),
        "overlay Parameter identity must be scoped to the BASE fn name"
    );
    assert_ne!(
        identity,
        FrozenTypeIdentity::from_canonical_descriptor("parameter:describe::i64:T"),
        "overlay Parameter identity must NOT be scoped to the mono key"
    );
}

// ADR-009 A3 (S3): on the specialization path, the overlay-derived Parameter
// identity is STABLE across instantiations of one base generic function
// (identity::i64 and identity::string agree — owner is the BASE name, so two
// mono compiles intern the same "parameter:{base}:{name}" descriptor) and
// DISTINCT across owning functions. Never renumber existing identities — the
// SHA-256 descriptor scheme is fixed; any red here means owner scoping in
// cache.rs/semantic_freeze.rs state threading is wrong.
#[test]
fn specialization_overlay_identity_is_stable_across_instantiations() {
    let overlay_identity = |mono_name: &str, base_name: &str| {
        let program = shape_ast::parse_program("fn placeholder(value: int) -> int { value }")
            .expect("mono-shaped function parses");
        let shape_ast::ast::Item::Function(definition, _) = &program.items[0] else {
            panic!("expected function item");
        };
        let mut mono_def = definition.clone();
        mono_def.name = mono_name.to_string();
        assert!(mono_def.type_params.is_none());
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        compiler
            .register_function(&mono_def)
            .expect("mono def registers");
        compiler.current_function = compiler
            .program
            .functions
            .iter()
            .position(|function| function.name == mono_name);
        compiler
            .install_semantic_freeze()
            .expect("registration-complete state freezes");
        compiler.specialization_type_param_overlay =
            Some((base_name.to_string(), vec!["T".to_string()]));
        let overlay = compiler
            .comptime_freeze_overlay()
            .expect("post-barrier site obtains the handle");
        overlay.identity_of("T").expect("overlay T identity")
    };

    // Same base, different instantiations: identical identity.
    assert_eq!(
        overlay_identity("identity::i64", "identity"),
        overlay_identity("identity::string", "identity"),
        "Parameter identity must be stable across instantiations of one base fn"
    );
    // Different owning functions: distinct identities.
    assert_ne!(
        overlay_identity("identity::i64", "identity"),
        overlay_identity("filter::i64", "filter"),
        "Parameter identities must be distinct across owning functions"
    );
}

#[test]
fn unknown_identity_is_rejected_at_the_freeze_boundary() {
    let freeze = freeze_of(|_| {});

    assert_eq!(
        freeze.category_of(FrozenTypeIdentity::INVALID),
        Err("type_ref received an unknown semantic type identity".to_string())
    );
}

// ─── ADR-009 B1 S2: freeze payload query + payload descriptor builders ────

mod payload_query {
    use super::payloads::{FrozenPayloadDescriptor, pending_payload_rejection};
    use super::*;
    use crate::compiler::comptime_builtins::semantic_freeze;
    use shape_runtime::comptime_reflection::{
        FLOAT_WIDTH_SCHEMA_NAME, FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES, FloatWidth,
        FrozenPrimitive, INTEGER_WIDTH_SCHEMA_NAME, IntegerWidth,
    };
    use shape_runtime::type_schema::builtin_schemas::{
        COMPTIME_FROZEN_ERASED_SCHEMA, COMPTIME_FROZEN_NEVER_SCHEMA,
        COMPTIME_FROZEN_PRIMITIVE_SCHEMA, COMPTIME_FROZEN_TYPE_SCHEMA,
    };
    use shape_value::heap_value::{HeapValue, TypedObjectStorage};
    use shape_value::{KindedSlot, NativeKind};

    /// The full primitive payload matrix: every sealed-sub-algebra family
    /// member AND every synonym resolves to the same exact width/domain
    /// payload through the ONE query API (`payload_of` beside
    /// `identity_of`/`category_of`). `bigint` is the named
    /// `SignedInteger(Arbitrary)` decision.
    #[test]
    fn primitive_payload_matrix_covers_every_family_and_synonym() {
        let freeze = freeze_of(|_| {});
        let matrix: &[(&[&str], FrozenPrimitive)] = &[
            (&["unit", "void", "()"], FrozenPrimitive::Unit),
            (&["bool"], FrozenPrimitive::Bool),
            (&["char"], FrozenPrimitive::Char),
            (
                &["int", "i64"],
                FrozenPrimitive::SignedInteger(IntegerWidth::W64),
            ),
            (&["i8"], FrozenPrimitive::SignedInteger(IntegerWidth::W8)),
            (&["i16"], FrozenPrimitive::SignedInteger(IntegerWidth::W16)),
            (&["i32"], FrozenPrimitive::SignedInteger(IntegerWidth::W32)),
            (&["u8"], FrozenPrimitive::UnsignedInteger(IntegerWidth::W8)),
            (
                &["u16"],
                FrozenPrimitive::UnsignedInteger(IntegerWidth::W16),
            ),
            (
                &["u32"],
                FrozenPrimitive::UnsignedInteger(IntegerWidth::W32),
            ),
            (
                &["u64"],
                FrozenPrimitive::UnsignedInteger(IntegerWidth::W64),
            ),
            (
                &["bigint"],
                FrozenPrimitive::SignedInteger(IntegerWidth::Arbitrary),
            ),
            (
                &["number", "f64", "float"],
                FrozenPrimitive::BinaryFloat(FloatWidth::W64),
            ),
            (&["f32"], FrozenPrimitive::BinaryFloat(FloatWidth::W32)),
            (&["decimal"], FrozenPrimitive::Decimal),
            (&["string", "str"], FrozenPrimitive::String),
            (&["null"], FrozenPrimitive::Null),
            (&["undefined"], FrozenPrimitive::Undefined),
        ];
        for (names, expected) in matrix {
            for name in *names {
                let identity = freeze
                    .identity_of(name)
                    .unwrap_or_else(|| panic!("{name} must be frozen"));
                assert_eq!(
                    freeze.payload_of(identity),
                    Ok(FrozenPayloadDescriptor::Primitive(*expected)),
                    "payload for {name}"
                );
            }
        }
    }

    /// `never` reflects to the `Never` payload; `any` reflects to `Erased`
    /// with the empty bound set (the only reachable erased spelling until
    /// A2 lands trait-bound syntax).
    #[test]
    fn never_and_erased_payloads_are_complete_for_reachable_forms() {
        let freeze = freeze_of(|_| {});

        let never = freeze.identity_of("never").expect("never identity");
        assert_eq!(freeze.payload_of(never), Ok(FrozenPayloadDescriptor::Never));

        let any = freeze.identity_of("any").expect("any identity");
        match freeze.payload_of(any) {
            Ok(FrozenPayloadDescriptor::Erased { bounds }) => {
                assert!(bounds.is_empty(), "any carries the empty bound set");
            }
            other => panic!("any must reflect to Erased, got {other:?}"),
        }
    }

    /// Rejection-matrix row R1: each of the 7 non-enabled categories has ONE
    /// named per-category diagnostic — naming the category, stating the
    /// payload descriptor has not landed, and pointing at `type_category` —
    /// never a partial descriptor. Parameter is asserted end-to-end through
    /// a scoped overlay identity (`parameter:{owner}:{name}`), Nominal
    /// end-to-end through a frozen struct.
    #[test]
    fn non_enabled_categories_reject_with_named_per_category_diagnostics() {
        for category in FrozenTypeCategory::ALL {
            if FROZEN_TYPE_ENABLED_PAYLOAD_CATEGORIES.contains(&category) {
                continue;
            }
            let diagnostic = pending_payload_rejection(category);
            assert_eq!(
                diagnostic,
                format!(
                    "reflect: the {} payload descriptor has not landed \
                     (pending payload ticket); use type_category for the \
                     exhaustive category",
                    category.variant_name()
                )
            );
        }

        // Parameter — through a scoped overlay identity.
        let overlay = FreezeOverlay::new(freeze_of(|_| {}), "map", &["T".to_string()]);
        let t = overlay.identity_of("T").expect("T identity");
        assert_eq!(
            overlay.payload_of(t),
            Err(pending_payload_rejection(FrozenTypeCategory::Parameter))
        );

        // Nominal — through the base freeze.
        let freeze = freeze_of(|compiler| add_struct(compiler, "Alpha"));
        let alpha = freeze.identity_of("Alpha").expect("Alpha identity");
        let error = freeze.payload_of(alpha).expect_err("Nominal must reject");
        assert!(
            error.contains("the Nominal payload descriptor has not landed")
                && error.contains("use type_category"),
            "R1 Nominal diagnostic missing: {error}"
        );
    }

    /// The unknown-identity freeze-boundary rejection is unchanged by the
    /// payload query.
    #[test]
    fn unknown_identity_payload_rejection_is_unchanged() {
        let freeze = freeze_of(|_| {});
        assert_eq!(
            freeze.payload_of(FrozenTypeIdentity::INVALID),
            Err("type_ref received an unknown semantic type identity".to_string())
        );
    }

    // ── heap-value builders ──────────────────────────────────────────────

    fn storage_of(value: &HeapValue) -> &TypedObjectStorage {
        let HeapValue::TypedObject(ptr) = value else {
            panic!("descriptor must be a TypedObject, got {:?}", value.kind());
        };
        unsafe { &*ptr.as_ptr() }
    }

    fn schema_name_of(storage: &TypedObjectStorage) -> String {
        shape_runtime::type_schema::lookup_schema_by_id_public(storage.schema_id as u32)
            .expect("descriptor schema id must resolve")
            .name
            .clone()
    }

    /// Read `__variant` (field 0) and `__payload_0` (field 1) from an
    /// enum-layout descriptor object.
    fn variant_and_payload(storage: &TypedObjectStorage) -> (i64, KindedSlot) {
        let variant = storage
            .clone_field_kinded(0)
            .and_then(|slot| slot.as_i64())
            .expect("__variant must be an int");
        let payload = storage
            .clone_field_kinded(1)
            .expect("__payload_0 must be readable");
        (variant, payload)
    }

    /// `build_frozen_type_heap_value` composes schema-correct NESTED typed
    /// objects: FrozenType{__variant: catalog ordinal, __payload_0:
    /// FrozenPrimitive{__variant, __payload_0: width-domain enum}} — typed
    /// descriptor data all the way down, no rendered type-name strings.
    #[test]
    fn builder_produces_schema_correct_nested_primitive_descriptor() {
        let overlay = semantic_freeze::overlay_for_tests(&BytecodeCompiler::new());

        // int → Primitive(SignedInteger(W64)), FrozenType variant ordinal 0.
        let int_identity = overlay.identity_of("int").expect("int identity");
        let frozen = payloads::build_frozen_type_heap_value(int_identity, &overlay)
            .expect("int payload builds");
        let frozen_storage = storage_of(&frozen);
        assert_eq!(schema_name_of(frozen_storage), COMPTIME_FROZEN_TYPE_SCHEMA);
        let (variant, payload) = variant_and_payload(frozen_storage);
        assert_eq!(variant, 0, "Primitive is catalog ordinal 0");

        let primitive_storage = payload
            .as_typed_object_storage()
            .expect("payload must be a typed object");
        assert_eq!(
            schema_name_of(primitive_storage),
            COMPTIME_FROZEN_PRIMITIVE_SCHEMA
        );
        let (primitive_variant, width_slot) = variant_and_payload(primitive_storage);
        // SignedInteger is declaration index 3 in the FrozenPrimitive catalog.
        assert_eq!(primitive_variant, 3);
        let width_storage = width_slot
            .as_typed_object_storage()
            .expect("width payload must be a typed object");
        assert_eq!(schema_name_of(width_storage), INTEGER_WIDTH_SCHEMA_NAME);
        let (width_variant, _) = (
            width_storage
                .clone_field_kinded(0)
                .and_then(|slot| slot.as_i64())
                .expect("width __variant"),
            (),
        );
        assert_eq!(width_variant, 3, "W64 is IntegerWidth declaration index 3");

        // bigint → SignedInteger(Arbitrary): width variant 4.
        let bigint_identity = overlay.identity_of("bigint").expect("bigint identity");
        let frozen = payloads::build_frozen_type_heap_value(bigint_identity, &overlay)
            .expect("bigint payload builds");
        let (_, payload) = variant_and_payload(storage_of(&frozen));
        let (_, width_slot) =
            variant_and_payload(payload.as_typed_object_storage().expect("primitive object"));
        let width_storage = width_slot
            .as_typed_object_storage()
            .expect("width payload must be a typed object");
        let arbitrary = width_storage
            .clone_field_kinded(0)
            .and_then(|slot| slot.as_i64())
            .expect("width __variant");
        assert_eq!(arbitrary, 4, "Arbitrary is IntegerWidth declaration index 4");

        // number → BinaryFloat(W64): FloatWidth schema, variant 1.
        let number_identity = overlay.identity_of("number").expect("number identity");
        let frozen = payloads::build_frozen_type_heap_value(number_identity, &overlay)
            .expect("number payload builds");
        let (_, payload) = variant_and_payload(storage_of(&frozen));
        let primitive_storage = payload.as_typed_object_storage().expect("primitive object");
        let (primitive_variant, width_slot) = variant_and_payload(primitive_storage);
        assert_eq!(primitive_variant, 5, "BinaryFloat is declaration index 5");
        let width_storage = width_slot
            .as_typed_object_storage()
            .expect("width payload must be a typed object");
        assert_eq!(schema_name_of(width_storage), FLOAT_WIDTH_SCHEMA_NAME);

        // bool → scalar member: Null payload slot (no width domain).
        let bool_identity = overlay.identity_of("bool").expect("bool identity");
        let frozen = payloads::build_frozen_type_heap_value(bool_identity, &overlay)
            .expect("bool payload builds");
        let (_, payload) = variant_and_payload(storage_of(&frozen));
        let primitive_storage = payload.as_typed_object_storage().expect("primitive object");
        let (primitive_variant, scalar_payload) = variant_and_payload(primitive_storage);
        assert_eq!(primitive_variant, 1, "Bool is declaration index 1");
        assert_eq!(scalar_payload.kind(), NativeKind::Null);
    }

    /// never → FrozenType{__variant: 1, __payload_0: FrozenNever{}} and
    /// any → FrozenType{__variant: 9, __payload_0: FrozenErased{bounds: []}}
    /// — ordinal-pinned variant ids (1/9, never dense).
    #[test]
    fn builder_produces_never_and_erased_descriptors_at_pinned_ordinals() {
        let overlay = semantic_freeze::overlay_for_tests(&BytecodeCompiler::new());

        let never_identity = overlay.identity_of("never").expect("never identity");
        let frozen = payloads::build_frozen_type_heap_value(never_identity, &overlay)
            .expect("never payload builds");
        let frozen_storage = storage_of(&frozen);
        let (variant, payload) = variant_and_payload(frozen_storage);
        assert_eq!(variant, 1, "Never is catalog ordinal 1");
        let never_storage = payload
            .as_typed_object_storage()
            .expect("payload must be a typed object");
        assert_eq!(schema_name_of(never_storage), COMPTIME_FROZEN_NEVER_SCHEMA);

        let any_identity = overlay.identity_of("any").expect("any identity");
        let frozen = payloads::build_frozen_type_heap_value(any_identity, &overlay)
            .expect("any payload builds");
        let frozen_storage = storage_of(&frozen);
        let (variant, payload) = variant_and_payload(frozen_storage);
        assert_eq!(variant, 9, "Erased is catalog ordinal 9, never dense 2");
        let erased_storage = payload
            .as_typed_object_storage()
            .expect("payload must be a typed object");
        assert_eq!(
            schema_name_of(erased_storage),
            COMPTIME_FROZEN_ERASED_SCHEMA
        );
        // The bound set is the empty array (the only reachable form).
        let bounds = erased_storage
            .clone_field_kinded(0)
            .expect("bounds must be readable");
        assert_eq!(
            bounds.kind(),
            NativeKind::Ptr(shape_value::heap_value::HeapKind::TypedArray)
        );
    }

    /// The builder inherits the R1 rejection: a scoped Parameter identity
    /// (and every other non-enabled category) is a named compile-time
    /// rejection at the builder too — never a partial descriptor.
    #[test]
    fn builder_rejects_non_enabled_categories_with_the_named_diagnostic() {
        let overlay = FreezeOverlay::new(freeze_of(|_| {}), "map", &["T".to_string()]);
        let t = overlay.identity_of("T").expect("T identity");
        let error = payloads::build_frozen_type_heap_value(t, &overlay)
            .map(|_| ())
            .expect_err("Parameter must reject at the builder");
        assert_eq!(
            error,
            pending_payload_rejection(FrozenTypeCategory::Parameter)
        );
    }
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
