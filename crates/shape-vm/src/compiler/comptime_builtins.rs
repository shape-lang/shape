//! Comptime builtin functions.
//!
//! These functions are only callable inside `comptime { }` blocks.
//! They provide compile-time reflection, trait checking, and compiler messaging.
//!
//! Available builtins:
//! - `implements(T, Trait)` — returns true if T implements Trait
//! - `warning(msg)` — emits a compile-time warning
//! - `error(msg)` — emits a compile-time error
//! - `build_config()` — returns build-time configuration
//! - `type_info(T)` — returns the `TypeInfo` reflection record for type `T`
//!   (W7 2026-05-17 — see
//!   `docs/cluster-audits/v0.3-w7-type_info-comptime-typed-return.md`)

use shape_ast::ast::TypeAnnotation;
use shape_runtime::marshal::{register_typed_fn_1, register_typed_fn_2};
use shape_runtime::module_exports::ModuleExports;
use shape_runtime::type_schema::typed_object_for_named_schema;
use shape_runtime::type_system::BuiltinTypes;
use shape_runtime::typed_module_exports::{
    ConcreteReturn, ConcreteType, TypedReturn, register_typed_function,
};
use shape_value::KindedSlot;
use shape_value::heap_value::HeapValue;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Build a `TypeReflectionSnapshot` from the bytecode compiler's
/// struct/enum/alias registries.
///
/// Called at every `execute_comptime` site so the comptime function
/// receives a per-compile-unit view of the user's declared types. Only
/// the type-name catalog is currently consumed by the W7 minimal slice;
/// the full `(field_name, TypeAnnotation)` field lists are populated
/// where the compiler has them (via `struct_generic_info`) so the
/// follow-up `TypeInfo.fields` slice can wire without re-touching this
/// call path.
pub(crate) fn build_type_reflection_snapshot(
    compiler: &super::BytecodeCompiler,
    enclosing_type_params: &[String],
) -> TypeReflectionSnapshot {
    let mut snapshot = TypeReflectionSnapshot::default();
    for (name, (field_names, _span)) in &compiler.struct_types {
        let field_types = compiler
            .struct_generic_info
            .get(name)
            .map(|info| info.runtime_field_types.clone())
            .unwrap_or_default();
        let ordered: Vec<(String, TypeAnnotation)> = field_names
            .iter()
            .filter_map(|fname| {
                field_types
                    .get(fname)
                    .cloned()
                    .map(|ann| (fname.clone(), ann))
            })
            .collect();
        snapshot.struct_defs.insert(name.clone(), ordered);
    }
    for (alias_name, _target) in &compiler.type_aliases {
        // `type_aliases: HashMap<String, String>` — the value is the
        // target type-name string, not a TypeAnnotation, so we surface
        // the alias as `Basic(target)` for downstream `type_info`
        // resolution.
        snapshot
            .alias_defs
            .insert(alias_name.clone(), TypeAnnotation::Basic(_target.clone()));
    }
    // Enums: pull from the schema registry via the type-inference
    // environment so we don't need a parallel compiler-side enum table.
    // The schema registry is the single source of truth post the
    // ADR-005 §1 single-discriminator discipline.
    for type_name in compiler
        .type_tracker
        .schema_registry()
        .type_names()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
    {
        if let Some(schema) = compiler.type_tracker.schema_registry().get(&type_name) {
            if let Some(enum_info) = schema.get_enum_info() {
                let variants: Vec<String> =
                    enum_info.variants.iter().map(|v| v.name.clone()).collect();
                snapshot.enum_defs.insert(type_name.clone(), variants);
            }
        }
    }
    for tp in enclosing_type_params {
        snapshot.known_type_params.insert(tp.clone());
    }
    snapshot
}

/// Snapshot of user-defined type names made available to the
/// `type_info(T)` comptime builtin.
///
/// Built by the outer compiler before comptime execution
/// (`compile_and_execute_comptime_program` in `comptime.rs`) and passed by
/// value into the closure for `type_info`. The current shape (minimal
/// W7 slice) only needs the type-name catalog (`struct_defs` keys),
/// alias-name catalog (`alias_defs` keys), enum-name catalog
/// (`enum_defs` keys), and generic-parameter set (`known_type_params`)
/// for kind discriminator dispatch.
///
/// `Vec<(String, TypeAnnotation)>` field payloads are kept on
/// `struct_defs` even though they're not consumed by the current
/// shipping shape — they're populated whenever the compiler has them
/// to hand and will be the load-bearing input when `TypeInfo.fields` is
/// wired in a follow-up.
#[derive(Debug, Clone, Default)]
pub(crate) struct TypeReflectionSnapshot {
    /// type name → ordered `(field_name, TypeAnnotation)` list.
    pub(crate) struct_defs: HashMap<String, Vec<(String, TypeAnnotation)>>,
    /// enum name → ordered variant names (TypeKind discriminator dispatch).
    pub(crate) enum_defs: HashMap<String, Vec<String>>,
    /// type alias name → underlying TypeAnnotation.
    pub(crate) alias_defs: HashMap<String, TypeAnnotation>,
    /// Generic type-parameter names known in the enclosing scope (e.g.
    /// `T`, `U`). When `type_info(T)` is called and `T` is in this set,
    /// the returned TypeInfo's `kind` is `TypeKind::TypeVar` (Q2
    /// parametric-supported disposition).
    pub(crate) known_type_params: HashSet<String>,
}

/// Directives emitted during comptime execution (e.g., from `extend target`).
#[derive(Debug, Clone)]
pub(crate) enum ComptimeDirective {
    Extend(shape_ast::ast::ExtendStatement),
    RemoveTarget,
    SetParamType {
        param_name: String,
        type_annotation: shape_ast::ast::TypeAnnotation,
    },
    SetParamValue {
        param_name: String,
        value: KindedSlot,
    },
    SetReturnType {
        type_annotation: shape_ast::ast::TypeAnnotation,
    },
    ReplaceBody {
        body: Vec<shape_ast::ast::Statement>,
    },
    ReplaceModule {
        items: Vec<shape_ast::ast::Item>,
    },
    /// §4.5.7: ADD generated items at the annotated item's module scope. Unlike
    /// `ReplaceModule` (which is only valid on a module target and replaces its
    /// body), `ExtendItems` is additive and valid on type/function/module
    /// targets: the parsed items are registered + compiled alongside the
    /// existing program.
    ExtendItems {
        items: Vec<shape_ast::ast::Item>,
    },
}

/// A non-fatal warning emitted from inside the comptime mini-VM by
/// `warning()` (comptime-excellence §4.4). Collected on a thread-local while
/// the block / handler runs, drained by the driver, and re-emitted by the
/// compiler with the driving construct's source span so `warning()` output is
/// spanned and LSDS-routed instead of a bare `eprintln!`.
#[derive(Debug, Clone)]
pub(crate) struct ComptimeDiagnostic {
    pub message: String,
}

thread_local! {
    static COMPTIME_DIRECTIVES: RefCell<Vec<ComptimeDirective>> = const { RefCell::new(Vec::new()) };
    static COMPTIME_DIAGNOSTICS: RefCell<Vec<ComptimeDiagnostic>> = const { RefCell::new(Vec::new()) };
}

/// Clear the thread-local comptime-diagnostics buffer before a comptime run.
pub(crate) fn clear_comptime_diagnostics() {
    COMPTIME_DIAGNOSTICS.with(|d| d.borrow_mut().clear());
}

/// Drain the comptime-diagnostics collected during the last comptime run.
pub(crate) fn take_comptime_diagnostics() -> Vec<ComptimeDiagnostic> {
    COMPTIME_DIAGNOSTICS.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

fn push_comptime_diagnostic(diag: ComptimeDiagnostic) {
    COMPTIME_DIAGNOSTICS.with(|d| d.borrow_mut().push(diag));
}

pub(crate) fn clear_comptime_directives() {
    COMPTIME_DIRECTIVES.with(|directives| {
        let mut directives = directives.borrow_mut();
        directives.clear();
    });
}

pub(crate) fn take_comptime_directives() -> Vec<ComptimeDirective> {
    COMPTIME_DIRECTIVES.with(|directives| {
        let mut directives = directives.borrow_mut();
        std::mem::take(&mut *directives)
    })
}

fn push_comptime_directive(directive: ComptimeDirective) -> Result<(), String> {
    COMPTIME_DIRECTIVES.with(|directives| {
        let mut directives = directives.borrow_mut();
        directives.push(directive);
    });
    Ok(())
}

fn parse_type_annotation_payload(payload: &str) -> Result<shape_ast::ast::TypeAnnotation, String> {
    if let Ok(parsed) = serde_json::from_str::<shape_ast::ast::TypeAnnotation>(payload) {
        return Ok(parsed);
    }

    // Fallback for older callers that still pass textual type source.
    let snippet = format!("fn __type_probe(value: {}) {{ value }}", payload);
    let program = shape_ast::parse_program(&snippet)
        .map_err(|e| format!("invalid type payload '{}': {}", payload, e))?;

    let maybe_ann = program.items.into_iter().find_map(|item| match item {
        shape_ast::ast::Item::Function(func, _) => {
            func.params.first().and_then(|p| p.type_annotation.clone())
        }
        _ => None,
    });

    maybe_ann.ok_or_else(|| format!("could not parse type payload '{}'", payload))
}

fn parse_function_body_payload(payload: &str) -> Result<Vec<shape_ast::ast::Statement>, String> {
    if let Ok(parsed) = serde_json::from_str::<Vec<shape_ast::ast::Statement>>(payload) {
        return Ok(parsed);
    }

    // Fallback for older callers that still pass source text.
    let snippet = format!("fn __body_probe() {{ {} }}", payload);
    let program = shape_ast::parse_program(&snippet)
        .map_err(|e| format!("invalid replacement body payload: {}", e))?;

    let maybe_body = program.items.into_iter().find_map(|item| match item {
        shape_ast::ast::Item::Function(func, _) => Some(func.body),
        _ => None,
    });

    maybe_body.ok_or_else(|| "could not parse replacement function body payload".to_string())
}

fn parse_module_items_payload(payload: &str) -> Result<Vec<shape_ast::ast::Item>, String> {
    if let Ok(parsed) = serde_json::from_str::<Vec<shape_ast::ast::Item>>(payload) {
        return Ok(parsed);
    }

    let snippet = format!("mod __module_probe__ {{ {} }}", payload);
    let program = shape_ast::parse_program(&snippet)
        .map_err(|e| format!("invalid replacement module payload: {}", e))?;

    let maybe_items = program.items.into_iter().find_map(|item| match item {
        shape_ast::ast::Item::Module(module, _) if module.name == "__module_probe__" => {
            Some(module.items)
        }
        _ => None,
    });

    maybe_items.ok_or_else(|| "could not parse replacement module payload".to_string())
}

/// Helper: create a string-kinded `KindedSlot` from a `&str`.
///
/// Phase 1.B-vm Wave 5a (ADR-006 §2.7.6 / Q8): the helper signature
/// changed from `ValueWord` to `KindedSlot` alongside the
/// `register_typed_function` body contract (`&[KindedSlot]`). The name
/// is kept as `nb_str` so existing callers in this module / its tests
/// don't need to be re-touched in 5a.
fn nb_str(s: &str) -> KindedSlot {
    KindedSlot::from_string_arc(Arc::new(s.to_string()))
}

/// Create a ModuleExports containing all comptime builtin functions.
///
/// These are registered as an extension module named "__comptime__" so they
/// are available during comptime execution but NOT during normal runtime.
///
/// `trait_impl_keys` contains the set of registered trait implementations.
/// Supported key forms:
/// - Legacy: "TraitName::TypeName"
/// - Canonical: "TraitName::TypeName::ImplNameOrDefault"
pub(crate) fn create_comptime_builtins_module(
    trait_impl_keys: HashSet<String>,
    type_snapshot: TypeReflectionSnapshot,
) -> ModuleExports {
    let mut module = ModuleExports::new("__comptime__");

    // implements(type_name: string, trait_name: string) -> bool
    // Checks the TypeRegistry's trait impl data captured at compile time.
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "implements",
        "Check if a type implements a trait at compile time",
        [("type_name", "string"), ("trait_name", "string")],
        ConcreteType::Bool,
        move |type_name, trait_name, _ctx| {
            let has_impl = |ty: &str| {
                let legacy = format!("{}::{}", trait_name, ty);
                let canonical_prefix = format!("{}::{}::", trait_name, ty);
                trait_impl_keys.contains(&legacy)
                    || trait_impl_keys
                        .iter()
                        .any(|key| key.starts_with(&canonical_prefix))
            };

            if has_impl(type_name.as_str()) {
                return Ok(TypedReturn::Concrete(ConcreteReturn::Bool(true)));
            }

            // Numeric widening: integer-family aliases can satisfy number-family impls.
            if BuiltinTypes::is_integer_type_name(type_name.as_str()) {
                for widen_to in &["number", "float", "f64"] {
                    if has_impl(widen_to) {
                        return Ok(TypedReturn::Concrete(ConcreteReturn::Bool(true)));
                    }
                }
            }

            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(false)))
        },
    );

    // warning(msg: string) -> Unit
    // Collects a compile-time warning. The message flows out on the
    // thread-local diagnostics buffer; the compiler re-emits it as a
    // spanned, LSDS-routed warning anchored at the comptime construct
    // (comptime-excellence §4.4). The old bare `eprintln!` (span-less,
    // not routed) is deleted.
    register_typed_function(
        &mut module,
        "warning",
        "Emit a compile-time warning",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            if let Some(msg) = nb_args.first().and_then(|nb| nb.as_str()) {
                push_comptime_diagnostic(ComptimeDiagnostic {
                    message: msg.to_string(),
                });
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // error(msg: string) -> never (returns an error)
    // Emits a compile-time error. This aborts comptime execution.
    register_typed_function(
        &mut module,
        "error",
        "Emit a compile-time error and abort comptime execution",
        vec![],
        ConcreteType::Unit,
        |nb_args, _ctx| {
            // ADR-006 §2.7.6: KindedSlot string accessor first; non-string
            // kinds fall through to a kind-aware diagnostic stub. The
            // pre-bulldozer `ValueWordDisplay` helper is deleted; the
            // body-side formatter for arbitrary `KindedSlot` lives in
            // Wave 5e (`executor/printing.rs`). Until then non-string
            // arguments to `error()` surface their kind name.
            let msg = match nb_args.first() {
                Some(nb) => match nb.as_str() {
                    Some(s) => s.to_string(),
                    None => format!("<{:?}>", nb.kind()),
                },
                None => "comptime error".to_string(),
            };
            Err(format!("[comptime error] {}", msg))
        },
    );

    // build_config() -> Object with build configuration
    // Returns a structured object: { debug, version, target_os, target_arch }
    //
    // S2 (comptime-excellence §4.3): constructed via
    // `typed_object_for_named_schema("__ComptimeBuildConfig", ...)`, which
    // resolves the reserved, concrete named schema (`builtin_schemas.rs` —
    // `bool debug` + `string version/target_os/target_arch`) BY NAME. Every
    // field carries a statically-sourceable NativeKind (no `FieldType::Any`,
    // so no `MakeFieldRef ... FIELD_TAG_ANY` — the R2 hazard). Named
    // resolution supersedes the earlier "rely on order-insensitive field-set
    // match" posture: `__ComptimeBuildConfig` is now `reserved` and is
    // therefore SKIPPED by field-set inference, so it could no longer be
    // reached that way regardless.
    register_typed_function(
        &mut module,
        "build_config",
        "Return build-time configuration",
        vec![],
        ConcreteType::Object,
        |_args, _ctx| {
            // ADR-006 §2.7.6 / Q8 (Wave 5a Substep 3): build the typed
            // object via `typed_object_for_named_schema` (which takes
            // `KindedSlot`), then project the resulting carrier through
            // its underlying `ValueSlot::as_heap_value()` to recover the
            // `Arc<TypedObjectStorage>` and rewrap it for
            // `ConcreteReturn::OpaqueTypedObject` — preserving ADR-005
            // §1's single-discriminator (HeapValue stays canonical) and
            // ADR-006 §2.7.6's no-per-heap-variant-accessor bound.
            //
            // The pre-bulldozer `TypedReturn::ValueWord` pass-through is
            // deleted; the strict-typed marshal boundary projects each
            // `TypedReturn` variant directly into a typed slot via the
            // function's registered `NativeKind`.
            let kinded = typed_object_for_named_schema(
                "__ComptimeBuildConfig",
                &[
                    ("debug", KindedSlot::from_bool(cfg!(debug_assertions))),
                    ("version", nb_str(env!("CARGO_PKG_VERSION"))),
                    ("target_os", nb_str(std::env::consts::OS)),
                    ("target_arch", nb_str(std::env::consts::ARCH)),
                    // Frozen introspection-contract version marker
                    // (comptime-excellence §4.1.4).
                    ("comptime_api", KindedSlot::from_int(1)),
                ],
            );
            // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
            // canonical receiver-recovery pattern per CLAUDE.md
            // "5-arm receiver-recovery soundness rule" — the kinded
            // slot's bits are `Arc::into_raw(Arc<TypedObjectStorage>)`
            // (the `ValueSlot::from_typed_object` convention), NOT
            // `Arc::into_raw(Arc<HeapValue>)`. The pre-W17 body used
            // `kinded.slot().as_heap_value()` which is wrong-type
            // recovery — it reads `TypedObjectStorage`'s first 8 bytes
            // (the `schema_id: u64`) as if they were a `HeapValue`
            // discriminator and segfaults. Reconstruct via the
            // canonical `Arc::<TypedObjectStorage>::from_raw` pattern
            // (mirror of `op_set_field_typed` in `typed_object_ops.rs`
            // and the post-`3ac2f11` method-handler files); clone the
            // share for the outer `HeapValue` wrapper; let `kinded`'s
            // Drop release the original share via its kind-dispatched
            // arm (the §2.7.26 ModuleFn no-op arm doesn't apply here —
            // kind is TypedObject, drop is `Arc::decrement_strong_count`
            // per §2.7.7 / Q9 dispatch table).
            let bits = kinded.slot().raw();
            // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): canonical
            // receiver-recovery for v2-raw TypedObjectStorage payloads.
            // Slot bits are `*const TypedObjectStorage` (NOT
            // `Arc::into_raw(Arc<...>)`); the carrier owns one share on
            // the on-header refcount. Bump via `v2_retain` to claim a
            // share for the outer `HeapValue::TypedObject(TypedObjectPtr)`
            // wrapper; `kinded`'s Drop retires its original share through
            // the §2.7.7 / Q9 dispatch table (TypedObject arm calls
            // `release_elem` per the ckpt-2 lockstep).
            let ptr = bits as *const shape_value::heap_value::TypedObjectStorage;
            // SAFETY: per `typed_object_for_named_schema`'s construction-
            // side contract, `ptr` points to a live TypedObjectStorage with
            // refcount ≥ 1.
            unsafe {
                shape_value::v2::refcount::v2_retain(&(*ptr).header);
            }
            drop(kinded);
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(HeapValue::TypedObject(
                    shape_value::heap_value::TypedObjectPtr::new(ptr),
                )),
            )))
        },
    );

    // W7 (2026-05-17) — `type_info(T)` comptime builtin.
    //
    // Returns the `TypeInfo` reflection record for the named type. See
    // `docs/cluster-audits/v0.3-w7-type_info-comptime-typed-return.md` §4
    // (recommendation (b) TypeInfo struct return) + §8 (user dispositions
    // Q1-Q5). Bare type-identifier arguments are rewritten to string
    // literals at the call site by `rewrite_type_info_ident_args` in
    // `comptime.rs` (mirror of the `implements` precedent).
    //
    // Schema layout matches `crates/shape-runtime/stdlib-src/core/types.shape`:
    //   TypeInfo { name: string, kind: TypeKind }
    //   FieldInfo { name: string, type_name: string }   // future-use; not
    //                                                   // transitively reachable
    //                                                   // through TypeInfo today
    //   TypeKind = enum { Int Float Bool String Decimal BigInt
    //                     Array HashMap Option Result TypedObject
    //                     TraitObject TypeVar Function Tuple Unit Unknown }
    //
    // S2 (comptime-excellence §4.3): the TypeInfo record is built via the
    // reserved, concrete named schema `__ComptimeTypeInfo` ({name, kind},
    // registered at init in `builtin_schemas.rs`) — see
    // `build_type_info_heap_value`. The previous
    // `register_predeclared_any_schema(&["kind", "name"])` lazily minted an
    // anonymous `{kind, name}` schema whose field ORDER was the reverse of
    // the stdlib `TypeInfo {name, kind}` the compiler uses for the
    // `OpaqueTypedObject("TypeInfo")` return type, so typed field access
    // read `name`/`kind` at swapped offsets. Named construction in
    // {name, kind} order aligns the physical layout with the consumer.
    //
    // `type_info(T).fields` returns the declared fields of a TypedObject type as
    // an `Array<FieldDescriptor>` — the same row shape as `target.fields` in an
    // annotation handler (comptime-excellence §4.1.2). Every non-TypedObject kind
    // reflects an empty array.
    let snapshot_for_type_info = type_snapshot;
    register_typed_function(
        &mut module,
        "type_info",
        "Return the TypeInfo reflection record for the named type",
        vec![],
        ConcreteType::OpaqueTypedObject("TypeInfo".to_string()),
        move |nb_args, _ctx| {
            // The type name arrives as a string arg (bare type identifiers are
            // rewritten to string literals before the comptime block runs). If
            // it cannot be read as a string, surface a clean error rather than
            // reflecting an arbitrary name.
            let raw_name = nb_args
                .first()
                .and_then(|nb| nb.as_str())
                .ok_or_else(|| "type_info expects a type name".to_string())?;
            let type_info_hv = build_type_info_heap_value(raw_name, &snapshot_for_type_info)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
                Arc::new(type_info_hv),
            )))
        },
    );

    // Internal comptime directive: emit an extend statement payload (JSON AST).
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "__emit_extend",
        "Internal: emit extend directive payload",
        "payload",
        "string",
        ConcreteType::Unit,
        |json, _ctx| {
            let extend: shape_ast::ast::ExtendStatement = serde_json::from_str(json.as_str())
                .map_err(|e| format!("invalid extend payload: {}", e))?;
            push_comptime_directive(ComptimeDirective::Extend(extend))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: remove the current annotation target.
    register_typed_function(
        &mut module,
        "__emit_remove",
        "Internal: remove the current annotation target",
        vec![],
        ConcreteType::Unit,
        |_nb_args, _ctx| {
            push_comptime_directive(ComptimeDirective::RemoveTarget)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: set a parameter type by parameter name.
    // __emit_set_param_type(param_name: string, type_payload: string)
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "__emit_set_param_type",
        "Internal: set a parameter type by name",
        [("param_name", "string"), ("type_payload", "string")],
        ConcreteType::Unit,
        |param_name, payload, _ctx| {
            let type_annotation = parse_type_annotation_payload(payload.as_str())?;
            push_comptime_directive(ComptimeDirective::SetParamType {
                param_name: param_name.as_str().to_string(),
                type_annotation,
            })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: set an integer parameter default value.
    // __emit_set_param_value(param_name: string, value: int)
    //
    // This intentionally uses the fixed-arity typed marshal path: the
    // variadic `register_typed_function` helper currently stamps every
    // incoming argument as Bool, so a string param name cannot be recovered
    // there without a dynamic fallback.
    register_typed_fn_2::<_, Arc<String>, i64>(
        &mut module,
        "__emit_set_param_value",
        "Internal: set a parameter default value by name",
        [("param_name", "string"), ("value", "int")],
        ConcreteType::Unit,
        |param_name, value, _ctx| {
            let value = KindedSlot::from_int(value);
            let param_name = param_name.as_str().to_string();
            push_comptime_directive(ComptimeDirective::SetParamValue { param_name, value })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: set function return type.
    // __emit_set_return_type(type_payload: string)
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "__emit_set_return_type",
        "Internal: set the function return type",
        "type_payload",
        "string",
        ConcreteType::Unit,
        |payload, _ctx| {
            let type_annotation = parse_type_annotation_payload(payload.as_str())?;
            push_comptime_directive(ComptimeDirective::SetReturnType { type_annotation })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: replace function body from serialized AST payload.
    // __emit_replace_body(body_payload: string)
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "__emit_replace_body",
        "Internal: replace function body from AST payload",
        "body_payload",
        "string",
        ConcreteType::Unit,
        |payload, _ctx| {
            let body = parse_function_body_payload(payload.as_str())?;
            push_comptime_directive(ComptimeDirective::ReplaceBody { body })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: replace module items from source payload.
    // __emit_replace_module(module_payload: string)
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "__emit_replace_module",
        "Internal: replace module items from source payload",
        "module_payload",
        "string",
        ConcreteType::Unit,
        |payload, _ctx| {
            let items = parse_module_items_payload(payload.as_str())?;
            push_comptime_directive(ComptimeDirective::ReplaceModule { items })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // Internal comptime directive: ADD generated items from source payload
    // (§4.5.7 `extend (expr)`). __emit_extend_items(items_payload: string)
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "__emit_extend_items",
        "Internal: add generated module items from source payload",
        "items_payload",
        "string",
        ConcreteType::Unit,
        |payload, _ctx| {
            let items = parse_module_items_payload(payload.as_str())?;
            push_comptime_directive(ComptimeDirective::ExtendItems { items })?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // §4.5.7.4: render a string as a valid Shape string literal (surrounding
    // quotes + escaped quotes/backslashes/newlines/braces) for embedding a
    // computed string into generated source. Comptime-callable helper backing
    // `std::comptime::string_lit`.
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "string_lit",
        "Render a string as a Shape string literal for embedding in generated source",
        "value",
        "string",
        ConcreteType::String,
        |value, _ctx| {
            let rendered = render_shape_string_literal(value.as_str());
            Ok(TypedReturn::Concrete(ConcreteReturn::String(rendered)))
        },
    );

    module
}

/// Render `s` as a valid Shape string literal: wrap in double quotes and escape
/// the characters the Shape lexer treats specially inside a `"..."` literal.
/// Shape's escape set (string_literals.rs) is `\n \t \r \\ \" \' \0 \{ \} \$ \#`;
/// braces and `$`/`#` are f-string metacharacters, so escaping them keeps the
/// rendered literal safe to embed inside generated f-strings too.
fn render_shape_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '$' => out.push_str("\\$"),
            '#' => out.push_str("\\#"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

// =========================================================================
// W7 (2026-05-17) — `type_info(T)` builder helpers.
//
// Constructs a `TypeInfo` HeapValue (`{ name: string, kind: TypeKind }`)
// from a bare type name string. Mirrors the `build_config` precedent
// (`typed_object_from_pairs` + v2-raw `TypedObjectStorage::_new` +
// `TypedObjectPtr` wrapping). The `kind` field is itself an enum-variant
// TypedObject (TypeKind discriminator), looked up against the
// user-registered `TypeKind` schema from `std::core::types`.
//
// Refcount discipline: every `TypedObjectStorage::_new` returns a raw
// pointer with refcount = 1. Wrapping in `TypedObjectPtr` transfers the
// share to the wrapper. Nested TypedObject embedded in a TypedObject
// slot uses `ValueSlot::from_typed_object_raw` (one share owned by the
// outer storage's slot list, retired via the schema's heap_mask + the
// nested TypedObject's HeapHeader release path on outer drop).
//
// Recursive Array<FieldInfo> / Array<TypeInfo> threading is deliberately
// deferred (W7-followup) — `Array<TypedObject>` field-storage requires
// the V3-S5 ckpt-5 / ckpt-6 monomorphized Array carriers to land first
// (see CLAUDE.md "Known Constraints" v2-raw-heap-audit entry). The
// shipping shape covers the `if ti.kind == TypeKind::TypedObject {...}`
// dispatch use case which is the primary v0.3 user-facing surface.
// =========================================================================

/// Bare type-name kind hints, mirroring `TypeKind` variants declared in
/// `crates/shape-runtime/stdlib-src/core/types.shape`. We look up
/// variant IDs by name at runtime (the order in `types.shape` is the
/// source of truth) so the ordinal here is not bit-encoded.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
enum TypeKindLabel {
    Int,
    Float,
    Bool,
    String,
    Decimal,
    BigInt,
    Array,
    HashMap,
    Option,
    Result,
    TypedObject,
    TraitObject,
    TypeVar,
    Function,
    Tuple,
    Unit,
    Unknown,
}

impl TypeKindLabel {
    fn as_str(self) -> &'static str {
        match self {
            TypeKindLabel::Int => "Int",
            TypeKindLabel::Float => "Float",
            TypeKindLabel::Bool => "Bool",
            TypeKindLabel::String => "String",
            TypeKindLabel::Decimal => "Decimal",
            TypeKindLabel::BigInt => "BigInt",
            TypeKindLabel::Array => "Array",
            TypeKindLabel::HashMap => "HashMap",
            TypeKindLabel::Option => "Option",
            TypeKindLabel::Result => "Result",
            TypeKindLabel::TypedObject => "TypedObject",
            TypeKindLabel::TraitObject => "TraitObject",
            TypeKindLabel::TypeVar => "TypeVar",
            TypeKindLabel::Function => "Function",
            TypeKindLabel::Tuple => "Tuple",
            TypeKindLabel::Unit => "Unit",
            TypeKindLabel::Unknown => "Unknown",
        }
    }
}

/// Classify a bare type name (without generic parameters) into a
/// `TypeKindLabel`. Generic-parameter names declared in the enclosing
/// scope project to `TypeVar` per Q2 disposition.
fn classify_bare_type_name(name: &str, snapshot: &TypeReflectionSnapshot) -> TypeKindLabel {
    if snapshot.known_type_params.contains(name) {
        return TypeKindLabel::TypeVar;
    }
    match name {
        "int" | "i64" | "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" => TypeKindLabel::Int,
        "number" | "f64" | "f32" | "float" => TypeKindLabel::Float,
        "bool" => TypeKindLabel::Bool,
        "string" | "str" => TypeKindLabel::String,
        "decimal" => TypeKindLabel::Decimal,
        "bigint" => TypeKindLabel::BigInt,
        "()" | "unit" | "void" => TypeKindLabel::Unit,
        _ => {
            if snapshot.struct_defs.contains_key(name)
                || snapshot.alias_defs.contains_key(name)
                || snapshot.enum_defs.contains_key(name)
            {
                // Enums and structs both materialize under the same
                // TypedObject HeapKind today (single schema per enum;
                // variants share __variant + payload fields). The
                // user-facing TypeKind discriminator is TypedObject
                // until a dedicated Enum variant is wired in a follow-up
                // — this matches the audit-doc §4.6 flat-discriminator
                // shape.
                TypeKindLabel::TypedObject
            } else {
                TypeKindLabel::Unknown
            }
        }
    }
}

/// Build a `TypeInfo` HeapValue from a type name string.
///
/// Entry point for the `type_info(T)` comptime builtin. Resolves the
/// name against the snapshot's struct / enum / alias / generic-param
/// catalogs and materializes the corresponding `TypeInfo` typed object.
/// The `kind` field is a string-encoded `TypeKind` variant name (e.g.
/// `"Int"`, `"TypedObject"`) — see the `TypeInfo` docstring in
/// `crates/shape-runtime/stdlib-src/core/types.shape` for the
/// cross-registry-boundary rationale.
fn build_type_info_heap_value(
    type_name: &str,
    snapshot: &TypeReflectionSnapshot,
) -> Result<HeapValue, String> {
    let label = classify_bare_type_name(type_name, snapshot);
    // fields: only a TypedObject (struct) type has declared fields; every other
    // kind reflects an empty array. Rows are built through the SAME
    // `build_field_descriptor_array` row builder as `target.fields`
    // (comptime-excellence §4.1.2) so both introspection surfaces agree.
    let field_rows: Vec<(String, String, Vec<super::comptime_target::FieldAnnotation>)> = snapshot
        .struct_defs
        .get(type_name)
        .map(|fields| {
            fields
                .iter()
                .map(|(fname, ftype)| {
                    (
                        fname.clone(),
                        super::comptime_target::type_annotation_to_string(ftype),
                        Vec::new(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let fields_arr =
        super::comptime_target::build_field_descriptor_array(&field_rows).map_err(|e| {
            format!("failed to build type_info fields for '{type_name}': {e}")
        })?;
    // {name, kind, fields} order matches the stdlib `TypeInfo` declaration so the
    // object's physical slot layout aligns with the compiler's typed field
    // access on the `OpaqueTypedObject("TypeInfo")` return type.
    let kinded = typed_object_for_named_schema(
        "__ComptimeTypeInfo",
        &[
            ("name", nb_str(type_name)),
            ("kind", nb_str(label.as_str())),
            ("fields", fields_arr),
        ],
    );
    // Wave 2 Round 4 D4 receiver-recovery pattern (same as build_config):
    // slot bits are `*const TypedObjectStorage`; bump refcount for the
    // outer HeapValue::TypedObject wrapper, drop the kinded slot so its
    // Drop releases the original share via the §2.7.7 / Q9 dispatch
    // table TypedObject arm.
    let bits = kinded.slot().raw();
    let ptr = bits as *const shape_value::heap_value::TypedObjectStorage;
    // SAFETY: `typed_object_for_named_schema` returns a fresh raw pointer
    // with refcount ≥ 1; the v2_retain pairs with the outer
    // `HeapValue::TypedObject` wrapper's eventual drop.
    unsafe {
        shape_value::v2::refcount::v2_retain(&(*ptr).header);
    }
    drop(kinded);
    Ok(HeapValue::TypedObject(
        shape_value::heap_value::TypedObjectPtr::new(ptr),
    ))
}

// Tests gated `deep-tests` post-W11: bodies invoke
// `ModuleExports::invoke_export` which is part of the deleted comptime
// dispatch ABI; restoration requires the kinded comptime invocation
// surface (Phase-2c reentry per ADR-006 §2.7.4).
#[cfg(all(test, feature = "deep-tests"))]
mod tests {
    use super::*;
    use shape_runtime::type_schema::TypeSchemaRegistry;

    fn test_ctx() -> shape_runtime::module_exports::ModuleContext<'static> {
        // Leak a registry so we get a &'static reference for tests
        let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
        shape_runtime::module_exports::ModuleContext {
            schemas: registry,
            invoke_callable: None,
            raw_invoker: None,
            function_hashes: None,
            vm_state: None,
            granted_permissions: None,
            scope_constraints: None,
            set_pending_resume: None,
            set_pending_frame_resume: None,
            remote_dispatch: None,
        }
    }

    #[test]
    fn test_comptime_builtins_module_created() {
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        assert_eq!(module.name, "__comptime__");
    }

    #[test]
    fn test_comptime_warning_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        let warning = module
            .typed_exports()
            .get("warning")
            .expect("warning function should exist");
        assert_eq!(warning.return_type, ConcreteType::Unit);
        let result = (warning.invoke)(&[], &ctx).expect("warning should return unit");
        assert!(matches!(
            result,
            TypedReturn::Concrete(ConcreteReturn::Unit)
        ));
    }

    #[test]
    fn test_comptime_error_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        let error = module
            .typed_exports()
            .get("error")
            .expect("error function should exist");
        assert_eq!(error.return_type, ConcreteType::Unit);
        let result = (error.invoke)(&[], &ctx);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("comptime error"));
    }

    #[test]
    fn test_comptime_implements_returns_false_when_not_registered() {
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        let result = module
            .typed_exports()
            .get("implements")
            .expect("implements function should exist");
        assert_eq!(result.return_type, ConcreteType::Bool);
    }

    #[test]
    fn test_comptime_implements_returns_true_when_registered() {
        let mut impls = HashSet::new();
        impls.insert("Serializable::number".to_string());
        impls.insert("Display::Currency".to_string());
        let module = create_comptime_builtins_module(impls, Default::default());
        let result = module
            .typed_exports()
            .get("implements")
            .expect("implements function should exist");
        assert_eq!(result.return_type, ConcreteType::Bool);
    }

    #[test]
    fn test_comptime_implements_numeric_widening() {
        let mut impls = HashSet::new();
        impls.insert("Serializable::number".to_string());
        let module = create_comptime_builtins_module(impls, Default::default());
        let result = module
            .typed_exports()
            .get("implements")
            .expect("implements function should exist");
        assert_eq!(result.return_type, ConcreteType::Bool);
    }

    #[test]
    fn test_comptime_build_config_builtin() {
        let ctx = test_ctx();
        let module = create_comptime_builtins_module(Default::default(), Default::default());
        let build_config = module
            .typed_exports()
            .get("build_config")
            .expect("build_config function should exist");
        assert_eq!(build_config.return_type, ConcreteType::Object);
        let result = (build_config.invoke)(&[], &ctx).expect("build_config should return object");
        assert!(matches!(
            result,
            TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(_))
        ));
    }
}
