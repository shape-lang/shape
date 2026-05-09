//! Compile-time (comptime) execution infrastructure.
//!
//! Provides a mini-VM executor that compiles and runs statements at compile time,
//! used for meta function methods with statement bodies.

use crate::bytecode::BytecodeProgram;
use crate::compiler::BytecodeCompiler;
use shape_ast::ast::{
    AnnotationHandlerParam, Expr, FunctionDef, Item, ObjectTypeField, Program, Span, Statement,
    TypeAnnotation,
};
use shape_ast::error::Result;
use shape_value::KindedSlot;

/// (name, arity, target_method, return_type)
const COMPTIME_BUILTIN_FORWARDERS: &[(&str, usize, &str, Option<&[&str]>)] = &[
    ("implements", 2, "implements", None),
    ("warning", 1, "warning", None),
    ("error", 1, "error", None),
    (
        "build_config",
        0,
        "build_config",
        Some(&["debug", "target_arch", "target_os", "version"]),
    ),
];

/// Comptime execution result.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.4.** The `value` carrier
/// migrated from the deleted `ValueWord` to `KindedSlot` (ADR-006 §2.7 /
/// Q7) — the post-§2.7.4 GENERIC_CARRIER shape for runtime values whose
/// `NativeKind` is not statically known to the consumer (the comptime VM
/// can return arbitrary heap-typed values to the outer compiler). The
/// in-VM execution path that actually populates `value` from `vm.execute`'s
/// raw bits + top-level `return_kind` is part of the comptime rebuild.
pub(crate) struct ComptimeExecutionResult {
    pub value: KindedSlot,
    pub directives: Vec<super::comptime_builtins::ComptimeDirective>,
}

fn comptime_target_param_type() -> TypeAnnotation {
    TypeAnnotation::Object(vec![
        ObjectTypeField {
            name: "kind".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "name".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "fields".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                "unknown".to_string(),
            ))),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "params".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                "unknown".to_string(),
            ))),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "return_type".to_string(),
            optional: true,
            type_annotation: TypeAnnotation::Basic("string".to_string()),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "annotations".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                "unknown".to_string(),
            ))),
            annotations: vec![],
        },
        ObjectTypeField {
            name: "captures".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                "unknown".to_string(),
            ))),
            annotations: vec![],
        },
    ])
}

fn comptime_builtin_forwarders() -> Vec<Item> {
    COMPTIME_BUILTIN_FORWARDERS
        .iter()
        .map(|(name, arity, target_method, return_fields)| {
            let params: Vec<shape_ast::ast::FunctionParameter> = (0..*arity)
                .map(|i| shape_ast::ast::FunctionParameter {
                    pattern: shape_ast::ast::DestructurePattern::Identifier(
                        format!("arg{}", i),
                        Span::DUMMY,
                    ),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    type_annotation: None,
                    default_value: None,
                })
                .collect();

            let args: Vec<Expr> = (0..*arity)
                .map(|i| Expr::Identifier(format!("arg{}", i), Span::DUMMY))
                .collect();

            let body_expr = Expr::QualifiedFunctionCall {
                namespace: "__comptime__".to_string(),
                function: (*target_method).to_string(),
                args,
                named_args: Vec::new(),
                span: Span::DUMMY,
            };

            // If the forwarder has known return fields, generate an Object
            // type annotation so the compiler can emit GetFieldTyped for
            // property access on the return value.
            let return_type = return_fields.map(|fields| {
                TypeAnnotation::Object(
                    fields
                        .iter()
                        .map(|f| ObjectTypeField {
                            name: f.to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("unknown".to_string()),
                            annotations: vec![],
                        })
                        .collect(),
                )
            });

            Item::Function(
                FunctionDef {
                    name: (*name).to_string(),
                    name_span: Span::DUMMY,
                    declaring_module_path: None,
                    doc_comment: None,
                    params,
                    return_type,
                    body: vec![Statement::Return(Some(body_expr), Span::DUMMY)],
                    type_params: Some(Vec::new()),
                    annotations: Vec::new(),
                    where_clause: None,
                    is_async: false,
                    is_comptime: false,
                },
                Span::DUMMY,
            )
        })
        .collect()
}

/// Ensure that the last statement in a body is a tail value (returns its result).
///
/// When the last statement is `Statement::If`, its value is discarded by the
/// compiler because it's compiled as a statement (not an expression).  This
/// helper recursively wraps the last expressions of each branch in explicit
/// `Statement::Return` so the wrapping comptime function returns the value.
fn ensure_tail_return(body: &mut Vec<Statement>) {
    let Some(last) = body.last_mut() else {
        return;
    };
    match last {
        // If the last statement is an if/else, ensure each branch returns.
        Statement::If(if_stmt, _span) => {
            ensure_tail_return(&mut if_stmt.then_body);
            if let Some(else_body) = &mut if_stmt.else_body {
                ensure_tail_return(else_body);
            }
        }
        // An expression statement at the end should become a return.
        Statement::Expression(expr, span) => {
            *last = Statement::Return(Some(expr.clone()), *span);
        }
        // Explicit return is already fine.
        Statement::Return(_, _) => {}
        // Other statements: do nothing (function will return null).
        _ => {}
    }
}

/// Rewrite bare identifier arguments to `implements()` calls as string literals.
/// This allows `implements(Dog, Speak)` (bare type/trait names) to work in
/// comptime blocks where those identifiers don't exist as variables.
fn rewrite_implements_ident_args(stmt: &mut Statement) {
    match stmt {
        Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => {
            rewrite_implements_in_expr(expr);
        }
        Statement::VariableDecl(decl, _) => {
            if let Some(init) = &mut decl.value {
                rewrite_implements_in_expr(init);
            }
        }
        Statement::If(if_stmt, _) => {
            for s in &mut if_stmt.then_body {
                rewrite_implements_ident_args(s);
            }
            if let Some(else_body) = &mut if_stmt.else_body {
                for s in else_body {
                    rewrite_implements_ident_args(s);
                }
            }
        }
        _ => {}
    }
}

fn rewrite_implements_in_expr(expr: &mut Expr) {
    if let Expr::FunctionCall { name, args, .. } = expr {
        if name == "implements" {
            for arg in args.iter_mut() {
                if let Expr::Identifier(ident, span) = arg {
                    *arg = Expr::Literal(shape_ast::ast::Literal::String(ident.clone()), *span);
                }
            }
        }
    }
}

/// Execute statements at compile time (comptime) and return the result.
///
/// Used for meta function methods with statement bodies. The statements are
/// wrapped in a function, compiled into a standalone BytecodeProgram, and
/// executed with a 5-second timeout.
///
/// Extension async functions (e.g., `postgres.connect()`) are supported:
/// `populate_module_objects()` wraps them with `block_in_place` + `block_on`,
/// which requires a tokio runtime. If no runtime exists (e.g., running from
/// tests or non-async CLI), a temporary single-threaded runtime is created.
pub(crate) fn execute_comptime(
    statements: &[Statement],
    comptime_helpers: &[FunctionDef],
    extensions: &[shape_runtime::module_exports::ModuleExports],
    trait_impl_keys: std::collections::HashSet<String>,
    known_type_symbols: std::collections::HashSet<String>,
) -> Result<ComptimeExecutionResult> {
    // Wrap statements in a function so the compiler produces a callable entry point.
    // Ensure the last statement is a tail return so if/else values aren't discarded.
    let mut body = statements.to_vec();
    // Transform bare identifiers in implements() calls to string literals,
    // since type/trait names aren't variables in the comptime scope.
    for stmt in &mut body {
        rewrite_implements_ident_args(stmt);
    }
    ensure_tail_return(&mut body);

    let func_name = "__comptime_block__".to_string();
    let func_def = FunctionDef {
        name: func_name.clone(),
        name_span: Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        params: Vec::new(),
        return_type: None,
        body,
        type_params: Some(Vec::new()),
        annotations: Vec::new(),
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };

    let mut items = comptime_builtin_forwarders();
    items.extend(
        comptime_helpers
            .iter()
            .cloned()
            .map(|helper| Item::Function(helper, Span::DUMMY)),
    );
    items.push(Item::Function(func_def, Span::DUMMY));
    items.push(Item::Expression(
        Expr::FunctionCall {
            name: func_name,
            args: Vec::new(),
            named_args: Vec::new(),
            span: Span::DUMMY,
        },
        Span::DUMMY,
    ));
    let program = Program {
        items,
        docs: shape_ast::ast::ProgramDocs::default(),
    };

    compile_and_execute_comptime_program(
        &program,
        vec!["__comptime__".to_string()],
        Vec::new(),
        extensions,
        trait_impl_keys,
        known_type_symbols,
    )
}

fn compile_and_execute_comptime_program(
    program: &Program,
    mut known_bindings: Vec<String>,
    runtime_module_bindings: Vec<(String, KindedSlot)>,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    trait_impl_keys: std::collections::HashSet<String>,
    known_type_symbols: std::collections::HashSet<String>,
) -> Result<ComptimeExecutionResult> {
    // Build the full extension list first so module namespace bindings
    // (e.g. `__comptime__`) are typed during compilation.
    let comptime_builtins =
        super::comptime_builtins::create_comptime_builtins_module(trait_impl_keys);
    let mut all_extensions: Vec<shape_runtime::module_exports::ModuleExports> = extensions.to_vec();
    all_extensions.push(comptime_builtins);

    // Extension module namespaces are valid bindings in comptime handlers.
    // This enables generic annotation code to call module-scoped intrinsics
    // (e.g. `duckdb.connect_codegen(uri)`) without hardcoded exceptions.
    for module in &all_extensions {
        if !known_bindings.iter().any(|name| name == &module.name) {
            known_bindings.push(module.name.clone());
        }
    }

    // Compile the mini-program
    // Note: Do NOT inject prelude items here. Comptime mini-programs only need
    // their own helpers + extension builtins. Injecting the prelude would cause
    // name collisions (e.g., prelude's `sum` vs a comptime-generated `sum` method).
    let mut compiler = BytecodeCompiler::new().with_extensions(all_extensions.clone());
    compiler.set_comptime_mode(true);
    compiler.allow_internal_comptime_namespace = true;
    compiler.register_known_bindings(&known_bindings);
    for type_name in known_type_symbols {
        compiler
            .struct_types
            .entry(type_name)
            .or_insert_with(|| (Vec::new(), Span::DUMMY));
    }
    let mut bytecode = compiler.compile(program)?;

    rebind_typed_object_bindings_to_bytecode_schemas(&bytecode, &runtime_module_bindings);

    for module in &all_extensions {
        ensure_module_object_schema(&mut bytecode, module);
    }

    // Execute inside a function that guarantees a tokio runtime is available.
    // Extension async functions (wrapped by populate_module_objects) need
    // `tokio::runtime::Handle::current()` to work.
    execute_in_runtime_with_module_bindings(bytecode, &all_extensions, runtime_module_bindings)
}

/// Re-register comptime module bindings against the freshly-compiled
/// bytecode's schema registry.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.4.** The previous body read
/// each binding as a `HeapValue::TypedObject { schema_id, slots, heap_mask }`
/// (the deleted inline-struct shape), looked up the matching bytecode schema
/// by field-name set, and rebuilt a new `HeapValue::TypedObject` against the
/// new schema id via `ValueSlot::from_value_word`. After the strict-typing
/// bulldozer:
///
/// - `HeapValue::TypedObject` now wraps `Arc<TypedObjectStorage>` per
///   ADR-006 §2.3 — there is no inline `slots` slice to walk.
/// - `ValueSlot::from_value_word` is replaced by per-FieldType constructors
///   (ADR-006 §2.4 / Q6) that take typed `Arc<T>` directly.
/// - The schema-rebind round-trip itself needs a kind-threaded
///   `read_typed_object_field(slot, kind, field_idx) -> KindedSlot` helper
///   to walk the comptime-VM's TypedObjectStorage and re-emit per-field
///   typed slots into the outer bytecode's schema.
///
/// All three pieces are part of the comptime-rebuild surface; until that
/// lands, this function is a structural no-op so callers continue to
/// compile. Comptime annotation handlers that pass typed-object module
/// bindings between the comptime VM and the outer compiler will lose the
/// re-registration step — surfacing as schema-id mismatches on read in the
/// outer compiler. That is the deferral cost; placeholder TypedObject
/// rebuilds are explicitly forbidden by playbook §7 #4 because they would
/// silently corrupt schema-keyed field reads.
fn rebind_typed_object_bindings_to_bytecode_schemas(
    _bytecode: &BytecodeProgram,
    _module_bindings: &[(String, KindedSlot)],
) {
    // todo!("phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4")
    //
    // No-op deferral: callers compile, schema mismatch surfaces at read
    // time rather than corruption at rebind time. See playbook §7 #4 and
    // ADR-006 §2.4 / §2.7.4.
}

fn ensure_module_object_schema(
    bytecode: &mut BytecodeProgram,
    module: &shape_runtime::module_exports::ModuleExports,
) {
    let schema_name = format!("__mod_{}", module.name);
    if bytecode.type_schema_registry.get(&schema_name).is_some() {
        return;
    }

    let mut export_names: Vec<String> = module
        .export_names_available(true)
        .into_iter()
        .map(|name| name.to_string())
        .collect();
    export_names.sort();
    export_names.dedup();

    let fields: Vec<(String, shape_runtime::type_schema::FieldType)> = export_names
        .into_iter()
        .map(|name| (name, shape_runtime::type_schema::FieldType::Any))
        .collect();
    bytecode
        .type_schema_registry
        .register_type(schema_name, fields);
}

/// Execute a comptime handler with a target parameter bound.
///
/// Used for comptime annotation handlers that accept an explicit target parameter.
/// The handler body
/// is wrapped in a function that takes one parameter (the target object), which
/// is passed as an argument when calling the function.
///
/// Returns the `KindedSlot` result of the handler execution (ADR-006 §2.7).
#[cfg(test)]
pub(crate) fn execute_comptime_with_target(
    handler_body: &Expr,
    handler_param: &str,
    target_value: KindedSlot,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    trait_impl_keys: std::collections::HashSet<String>,
    known_type_symbols: std::collections::HashSet<String>,
) -> Result<ComptimeExecutionResult> {
    let handler_params = vec![AnnotationHandlerParam {
        name: handler_param.to_string(),
        is_variadic: false,
    }];
    execute_comptime_with_annotation_handler(
        handler_body,
        &handler_params,
        target_value,
        &[],
        &[],
        &[],
        &[],
        extensions,
        trait_impl_keys,
        known_type_symbols,
    )
}

/// Execute a comptime annotation handler.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.4.** The body wires three
/// pieces that depend on the deleted `ValueWord` carrier:
///
/// 1. The `target_value: KindedSlot` is bound as a comptime module binding
///    keyed by `__target_arg__`. The set-module-binding path in
///    `execute_in_runtime_with_module_bindings` consumes the deleted
///    `set_module_binding_by_name_nb(&str, ValueWord)` API; the kinded
///    replacement is part of the comptime rebuild.
/// 2. `const_bindings` are materialized into the comptime AST via
///    `nb_to_expr`, which round-trips through deleted `ValueWord` accessors
///    (`as_any_array`, `as_str`, `as_decimal`, `as_heap_ref`, …). The
///    kinded replacement reads `(slot, kind)` directly and dispatches on
///    `NativeKind` for scalars + `slot.as_heap_value()` + `HeapValue::*`
///    match for heap arms (per ADR-006 §2.7.6 / Q8).
/// 3. The `ctx_nb` typed-object construction below uses the deleted
///    `typed_object_from_pairs` shape that takes `&[(&str, ValueWord)]`.
///    The kinded replacement takes `&[(&str, KindedSlot)]` and builds
///    `Arc<TypedObjectStorage>` directly.
///
/// All three pieces are part of the comptime-rebuild surface; the
/// signature is preserved so callers in `functions_annotations.rs` /
/// `statements.rs` / `expressions/mod.rs` continue to compile, but the
/// body panics until the rebuild lands rather than synthesizing a
/// placeholder result that would silently mis-bind handler params.
pub(crate) fn execute_comptime_with_annotation_handler(
    handler_body: &Expr,
    handler_params: &[AnnotationHandlerParam],
    target_value: KindedSlot,
    annotation_args: &[Expr],
    annotation_def_param_names: &[String],
    const_bindings: &[(String, KindedSlot)],
    comptime_helpers: &[FunctionDef],
    extensions: &[shape_runtime::module_exports::ModuleExports],
    trait_impl_keys: std::collections::HashSet<String>,
    known_type_symbols: std::collections::HashSet<String>,
) -> Result<ComptimeExecutionResult> {
    let _ = (
        handler_body,
        handler_params,
        target_value,
        annotation_args,
        annotation_def_param_names,
        const_bindings,
        comptime_helpers,
        extensions,
        trait_impl_keys,
        known_type_symbols,
        comptime_target_param_type as fn() -> TypeAnnotation,
        compile_and_execute_comptime_program
            as fn(
                &Program,
                Vec<String>,
                Vec<(String, KindedSlot)>,
                &[shape_runtime::module_exports::ModuleExports],
                std::collections::HashSet<String>,
                std::collections::HashSet<String>,
            ) -> Result<ComptimeExecutionResult>,
    );
    todo!("phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4")
}

/// Run compiled bytecode on a fresh VM with extensions and pre-set
/// module-binding variables.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.4.** The previous body
/// instantiated a `VirtualMachine`, called the deleted
/// `set_module_binding_by_name_nb(&str, ValueWord)` per pre-set binding,
/// invoked `vm.execute()` (which itself returns a synthesized `ValueWord`
/// via the forbidden `synthesize_value_word_from_raw`), and finally ran
/// `normalize_comptime_value` over the result to rebuild the comptime VM's
/// TypedObjects against the module-binding anonymous schema registry. All
/// three steps depend on the deleted dynamic-word value carrier:
///
/// - The kinded set-module-binding API takes `(slot, kind)` and threads
///   the kind into the parallel module-binding kind track per ADR-006
///   §2.7.7 / §2.7.8 (cell-storage kind-awareness — Q10).
/// - The kinded execute path returns `(bits, kind)` from `vm.execute_raw()`
///   plus `program_top_level_return_kind()`. The synthesis step is
///   forbidden by playbook §1 and `synthesize_value_word_from_raw` is on
///   the deletion list.
/// - The TypedObject normalization walks the comptime VM's
///   `Arc<TypedObjectStorage>` per ADR-006 §2.3 and rebuilds against the
///   outer registry, dispatching on `FieldType` per slot. That helper is
///   part of the comptime-rebuild surface.
///
/// Until Phase 2c lands, this function panics rather than running a
/// placeholder VM execution that would silently corrupt the kind-tracked
/// stack/binding state. The signature is preserved so the call chain
/// continues to type-check.
fn execute_in_runtime_with_module_bindings(
    bytecode: BytecodeProgram,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    module_bindings: Vec<(String, KindedSlot)>,
) -> Result<ComptimeExecutionResult> {
    let _ = (bytecode, extensions, module_bindings);
    todo!("phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4")
}

/// Convert a comptime execution result to an AST Literal for compilation.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.4.** The previous body
/// dispatched on the deleted `tag_bits::*` discriminator (`is_tagged`,
/// `get_tag`, `TAG_INT`, `TAG_BOOL`, `TAG_NONE`, `TAG_UNIT`, `TAG_HEAP`)
/// reading from `nb.raw_bits()`, plus heap fallbacks via `as_heap_ref` to
/// inspect `RareHeapData::TypeAnnotation`. After the strict-typing
/// bulldozer, dispatch is `match slot.kind { NativeKind::* => … }` for
/// scalars + `slot.as_heap_value()` + `HeapValue::*` match for heap arms
/// (per ADR-006 §2.7.6 / Q8 — heap dispatch goes through HeapValue, never
/// per-variant accessors). The kind-side discriminator is the parallel
/// `Vec<NativeKind>` track per ADR-006 §2.7.7 / Q9.
///
/// Used to replace `Expr::Comptime` nodes with their evaluated literal
/// values. Until Phase 2c lands, this panics rather than emitting a
/// placeholder Literal that would silently drop type-annotation comptime
/// returns (the cold-path `RareHeapData::TypeAnnotation` arm — needed for
/// `@ai`-style structured-output type queries).
pub(crate) fn vmvalue_to_literal(value: &KindedSlot) -> shape_ast::ast::Literal {
    let _ = value;
    todo!("phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4")
}

/// Convert a comptime execution result to an AST Literal for compilation.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.4.** Same surface as
/// `vmvalue_to_literal`. Used by comptime for-loop unrolling where
/// elements are already individual KindedSlots (extracted from the
/// `HeapValue::TypedArray(Arc<TypedArrayData>)` per-element shape per
/// ADR-006 §2.3).
pub(crate) fn nb_to_literal(nb: &KindedSlot) -> shape_ast::ast::Literal {
    let _ = nb;
    todo!("phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4")
}

/// Public entry point for converting a comptime KindedSlot to an AST
/// expression.
pub(crate) fn nb_to_expr_public(
    nb: &KindedSlot,
    span: Span,
) -> std::result::Result<Expr, String> {
    nb_to_expr(nb, span)
}

/// Convert a comptime KindedSlot to an AST expression.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.4.** The previous body
/// walked the deleted ValueWord shape (`as_any_array`, `as_decimal`,
/// `as_str`, `as_i64`, `as_f64`, `as_bool`, `is_none`, `is_unit`,
/// `as_heap_ref`) plus the deleted `typed_object_to_hashmap_nb` helper to
/// reconstruct `Expr::Array`/`Expr::Object`/`Expr::Literal`. After the
/// strict-typing bulldozer:
///
/// - Dispatch is `match slot.kind { NativeKind::Int64 => … }` per ADR-006
///   §2.7.7 (Q9), with heap-arm fall-through via `slot.as_heap_value()` +
///   `HeapValue::*` match (Q8).
/// - The TypedObject readback (`typed_object_to_hashmap_nb` replacement)
///   walks `Arc<TypedObjectStorage>` per ADR-006 §2.3, dispatching on
///   each field's `FieldType` to extract a kinded slot per slot index.
/// - The `HeapValue::TypedArray(Arc<TypedArrayData>)` arm replaces
///   `as_any_array` + `view.to_generic()`; element extraction reads the
///   typed buffer directly per `TypedArrayData::element_kind()`.
///
/// Until Phase 2c lands, this panics rather than emitting a placeholder
/// `Expr` shape that would silently drop array/object structure on
/// comptime substitution.
fn nb_to_expr(nb: &KindedSlot, span: Span) -> std::result::Result<Expr, String> {
    let _ = (nb, span);
    todo!("phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4")
}

// Phase-2c rebuild pending — see ADR-006 §2.4. The comptime test suite
// asserts on the deleted `ValueWord` carrier (`from_i64`, `from_f64`,
// `from_string`, `from_bool`, `none`, `unit`, `from_array`,
// `vmarray_from_vec`, `as_arc_string`, `as_number_coerce`, `as_heap_ref`)
// plus the deleted `vm.execute()` synthesis path. The whole module is
// stubbed and ignored until the comptime rebuild lands; re-enable
// per-test as the rebuild walks each path.
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4"]
    fn placeholder_phase_2c_comptime_tests() {}
}

#[cfg(any())]
#[cfg(test)]
mod tests_deferred {
    use super::*;
    use shape_ast::ast::{BinaryOp, Expr, Literal, Span, Statement};
    use shape_runtime::typed_module_exports::register_test_function;
    use shape_value::heap_value::HeapValue;

    #[test]
    fn test_comptime_simple_return() {
        let stmts = vec![Statement::Return(
            Some(Expr::Literal(Literal::Int(42), Span::DUMMY)),
            Span::DUMMY,
        )];

        let result = execute_comptime(&stmts, &[], &[], Default::default(), Default::default());
        assert!(
            result.is_ok(),
            "Comptime should succeed: {:?}",
            result.err()
        );
        assert_eq!(result.unwrap().value, ValueWord::from_i64(42));
    }

    #[test]
    fn test_comptime_string_return() {
        let stmts = vec![Statement::Return(
            Some(Expr::Literal(
                Literal::String("hello".to_string()),
                Span::DUMMY,
            )),
            Span::DUMMY,
        )];

        let result = execute_comptime(&stmts, &[], &[], Default::default(), Default::default());
        assert!(
            result.is_ok(),
            "Comptime should succeed: {:?}",
            result.err()
        );
        let val = result.unwrap().value;
        assert_eq!(
            val.as_arc_string().expect("Expected String").as_ref() as &str,
            "hello"
        );
    }

    #[test]
    fn test_comptime_arithmetic() {
        // Parse and execute: return 2 + 3
        let stmts = vec![Statement::Return(
            Some(Expr::BinaryOp {
                left: Box::new(Expr::Literal(Literal::Int(2), Span::DUMMY)),
                op: BinaryOp::Add,
                right: Box::new(Expr::Literal(Literal::Int(3), Span::DUMMY)),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];

        let result = execute_comptime(&stmts, &[], &[], Default::default(), Default::default());
        assert!(
            result.is_ok(),
            "Comptime arithmetic should succeed: {:?}",
            result.err()
        );
        assert_eq!(
            result
                .unwrap()
                .value
                .as_number_coerce()
                .expect("Expected 5"),
            5.0
        );
    }

    #[test]
    fn test_comptime_with_sync_extension() {
        // Create a mock extension with a sync function that returns a value.
        // Verify execute_comptime can call extension functions.
        use shape_runtime::module_exports::ModuleExports;

        let mut ext = ModuleExports::new("mock_db");
        register_test_function(&mut ext, 
            "get_schema",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(ValueWord::from_string(Arc::new(
                    "id:int,name:string".to_string(),
                )))
            },
        );

        // Parse a program that imports and calls the extension.
        // Extension modules are available as module_bindings (e.g., mock_db::get_schema()).
        // We need to register "mock_db" as a module_binding in the compiled program.
        let code = r#"
            use mock_db
            mock_db::get_schema()
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");

        // Compile with extension awareness
        let mut compiler = BytecodeCompiler::new();
        compiler.extension_registry = Some(Arc::new(vec![ext.clone()]));
        let bytecode = compiler.compile(&program).expect("compile");

        // Execute with extension registered
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.register_extension(ext);
        vm.populate_module_objects();

        let result = vm.execute(None);
        assert!(
            result.is_ok(),
            "Extension call should succeed: {:?}",
            result.err()
        );
        let val = result.unwrap().clone();
        assert_eq!(
            val.as_arc_string()
                .expect("Expected schema string")
                .as_ref() as &str,
            "id:int,name:string"
        );
    }

    #[test]
    fn test_comptime_extension_registry_flows_through_compiler() {
        // Verify that when BytecodeCompiler has an extension_registry set,
        // it is available during meta method compilation.
        use shape_runtime::module_exports::ModuleExports;

        let mut ext = ModuleExports::new("test_ext");
        register_test_function(&mut ext, 
            "version",
            |_args, _ctx: &shape_runtime::module_exports::ModuleContext| {
                Ok(ValueWord::from_string(Arc::new("1.0".to_string())))
            },
        );

        let mut compiler = BytecodeCompiler::new();
        compiler.extension_registry = Some(Arc::new(vec![ext]));

        // The extension_registry should be set
        assert!(compiler.extension_registry.is_some());
        assert_eq!(compiler.extension_registry.as_ref().unwrap().len(), 1);
        assert_eq!(
            compiler.extension_registry.as_ref().unwrap()[0].name,
            "test_ext"
        );
    }

    #[test]
    fn test_vmvalue_to_literal_int() {
        let lit = vmvalue_to_literal(&ValueWord::from_i64(42));
        assert_eq!(lit, Literal::Int(42));
    }

    #[test]
    fn test_vmvalue_to_literal_number() {
        let lit = vmvalue_to_literal(&ValueWord::from_f64(3.14));
        assert_eq!(lit, Literal::Number(3.14));
    }

    #[test]
    fn test_vmvalue_to_literal_string() {
        let lit = vmvalue_to_literal(&ValueWord::from_string(Arc::new("hello".to_string())));
        assert_eq!(lit, Literal::String("hello".to_string()));
    }

    #[test]
    fn test_vmvalue_to_literal_bool() {
        let lit = vmvalue_to_literal(&ValueWord::from_bool(true));
        assert_eq!(lit, Literal::Bool(true));
    }

    #[test]
    fn test_vmvalue_to_literal_none() {
        let lit = vmvalue_to_literal(&ValueWord::none());
        assert_eq!(lit, Literal::None);
    }

    #[test]
    fn test_vmvalue_to_literal_unit() {
        let lit = vmvalue_to_literal(&ValueWord::unit());
        assert_eq!(lit, Literal::Unit);
    }

    #[test]
    fn test_comptime_block_parsed_and_executed() {
        // Test that a comptime block in expression position can be parsed
        // and the statements are well-formed.
        let stmts = vec![Statement::Return(
            Some(Expr::BinaryOp {
                left: Box::new(Expr::Literal(Literal::Int(10), Span::DUMMY)),
                op: BinaryOp::Mul,
                right: Box::new(Expr::Literal(Literal::Int(5), Span::DUMMY)),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];

        let result = execute_comptime(&stmts, &[], &[], Default::default(), Default::default());
        assert!(
            result.is_ok(),
            "Comptime multiplication should succeed: {:?}",
            result.err()
        );
        assert_eq!(
            result
                .unwrap()
                .value
                .as_number_coerce()
                .expect("Expected 50"),
            50.0
        );
    }

    #[test]
    fn test_comptime_builtins_available_in_comptime_block() {
        // Verify that comptime builtins (build_config, etc.) are available via
        // execute_comptime() wiring.
        let stmts = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "build_config".to_string(),
                args: Vec::new(),
                named_args: Vec::new(),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        )];
        let result = execute_comptime(&stmts, &[], &[], Default::default(), Default::default())
            .map(|r| r.value);
        assert!(
            result.is_ok(),
            "build_config() should work in comptime: {:?}",
            result.err()
        );
        let val = result.unwrap();
        // build_config now returns TypedObject
        // cold-path: as_heap_ref retained — test assertion
        let is_typed_object_or_string = val
            .as_heap_ref() // cold-path
            .is_some_and(|h| matches!(h, HeapValue::TypedObject { .. } | HeapValue::String(_)));
        assert!(
            is_typed_object_or_string,
            "Expected TypedObject or String, got {:?}",
            val,
        );
    }

    #[test]
    fn test_comptime_print_build_config_no_stack_overflow() {
        // Regression: `__comptime__.build_config()` must dispatch through the
        // module object, not UFCS rewrite, otherwise it recurses infinitely.
        let stmts = vec![Statement::Expression(
            Expr::FunctionCall {
                name: "print".to_string(),
                args: vec![Expr::FunctionCall {
                    name: "build_config".to_string(),
                    args: Vec::new(),
                    named_args: Vec::new(),
                    span: Span::DUMMY,
                }],
                named_args: Vec::new(),
                span: Span::DUMMY,
            },
            Span::DUMMY,
        )];

        let result = execute_comptime(&stmts, &[], &[], Default::default(), Default::default());
        assert!(
            result.is_ok(),
            "print(build_config()) should execute in comptime: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_comptime_only_builtins_rejected_outside_comptime() {
        // type_info() is removed entirely and should produce a migration error.
        let code = r#"let x = type_info("Point")"#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(result.is_err(), "type_info() outside comptime should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("type_info has been removed"),
            "Error should mention removal: {}",
            err_msg
        );

        // implements()/build_config() remain comptime-only.
        let code2 = r#"let y = build_config()"#;
        let program2 = shape_ast::parser::parse_program(code2).expect("parse");
        let result2 = BytecodeCompiler::new().compile(&program2);
        assert!(
            result2.is_err(),
            "build_config() outside comptime should fail"
        );
    }

    #[test]
    fn test_comptime_with_target_simple() {
        // Execute a comptime handler that reads target.name
        let handler_body = Expr::PropertyAccess {
            object: Box::new(Expr::Identifier("target".to_string(), Span::DUMMY)),
            property: "name".to_string(),
            optional: false,
            span: Span::DUMMY,
        };

        let target_value = shape_runtime::type_schema::typed_object_from_pairs(&[
            (
                "kind",
                ValueWord::from_string(Arc::new("function".to_string())),
            ),
            (
                "name",
                ValueWord::from_string(Arc::new("my_func".to_string())),
            ),
            ("fields", ValueWord::from_array(shape_value::vmarray_from_vec(vec![]))),
            ("params", ValueWord::from_array(shape_value::vmarray_from_vec(vec![]))),
            ("return_type", ValueWord::none()),
            ("annotations", ValueWord::from_array(shape_value::vmarray_from_vec(vec![]))),
            ("captures", ValueWord::from_array(shape_value::vmarray_from_vec(vec![]))),
        ]);

        let result = execute_comptime_with_target(
            &handler_body,
            "target",
            target_value,
            &[],
            Default::default(),
            Default::default(),
        );
        assert!(
            result.is_ok(),
            "Comptime with target should succeed: {:?}",
            result.err()
        );
        let val = result.unwrap().value;
        assert_eq!(
            val.as_arc_string()
                .expect("Expected String(\"my_func\")")
                .as_ref() as &str,
            "my_func"
        );
    }

    #[test]
    fn test_comptime_with_target_from_function() {
        use crate::compiler::comptime_target::ComptimeTarget;
        use shape_ast::ast::{DestructurePattern, FunctionParameter, TypeAnnotation};

        // Build a target from a real function definition
        let func = FunctionDef {
            name: "greet".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            params: vec![FunctionParameter {
                pattern: DestructurePattern::Identifier("name".to_string(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(TypeAnnotation::Basic("string".to_string())),
                default_value: None,
            }],
            return_type: Some(TypeAnnotation::Basic("string".to_string())),
            body: Vec::new(),
            type_params: None,
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
            where_clause: None,
        };

        let target = ComptimeTarget::from_function(&func);
        let target_value = target.to_nanboxed();

        // Handler body: return target.kind
        let handler_body = Expr::PropertyAccess {
            object: Box::new(Expr::Identifier("t".to_string(), Span::DUMMY)),
            property: "kind".to_string(),
            optional: false,
            span: Span::DUMMY,
        };

        let result = execute_comptime_with_target(
            &handler_body,
            "t",
            target_value,
            &[],
            Default::default(),
            Default::default(),
        );
        assert!(
            result.is_ok(),
            "Comptime with function target should succeed: {:?}",
            result.err()
        );
        let val = result.unwrap().value;
        assert_eq!(
            val.as_arc_string()
                .expect("Expected String(\"function\")")
                .as_ref() as &str,
            "function"
        );
    }

    #[test]
    fn test_comptime_handler_end_to_end() {
        // Full end-to-end: define annotation with comptime phase handler, apply to function, compile
        let code = r#"
            annotation inspect() {
                comptime post(target, ctx) {
                    target.name
                }
            }
            @inspect()
            function greet(name) {
                return "hello " + name
            }
            greet("world")
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "Comptime handler end-to-end should compile: {:?}",
            result.err()
        );

        // The function should still work normally at runtime
        let bytecode = result.unwrap();
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let exec_result = vm.execute(None);
        assert!(
            exec_result.is_ok(),
            "Execution should succeed: {:?}",
            exec_result.err()
        );
        let val = exec_result.unwrap().clone();
        assert_eq!(
            val.as_arc_string()
                .expect("Expected String(\"hello world\")")
                .as_ref() as &str,
            "hello world"
        );
    }

    #[test]
    fn test_comptime_handler_accesses_target_params() {
        // Comptime handler that accesses target.params — verifies the target object is fully populated
        let code = r#"
            annotation check_params() {
                comptime post(target, ctx) {
                    target.params
                }
            }
            @check_params()
            function add(x, y) {
                return x + y
            }
            add(1, 2)
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "Comptime handler with params access should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let exec_result = vm.execute(None);
        assert!(
            exec_result.is_ok(),
            "Should execute: {:?}",
            exec_result.err()
        );
        assert_eq!(
            exec_result
                .unwrap()
                .clone()
                .as_number_coerce()
                .expect("Expected 3"),
            3.0
        );
    }

    #[test]
    fn test_comptime_fn_not_compiled_into_runtime_bytecode() {
        // Comptime fn functions should NOT produce bytecode in the runtime program.
        // They only exist as AST in function_defs for collect_comptime_helpers.
        let code = r#"
            comptime fn helper() {
                42
            }
            comptime {
                helper()
            }
            100
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let bytecode = BytecodeCompiler::new().compile(&program).expect("compile");

        // The comptime fn should NOT appear as a compiled function with a valid entry point.
        // It may still be in the function table (from registration), but its body
        // should not have been compiled.
        let helper_func = bytecode.functions.iter().find(|f| f.name == "helper");
        if let Some(func) = helper_func {
            // If the function is in the table, it must not have a compiled body
            // (body_length should be 0, entry_point should still be 0 from registration)
            assert_eq!(
                func.body_length, 0,
                "comptime fn should not have compiled body in runtime bytecode"
            );
        }

        // Runtime code should still work
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm.execute(None).expect("execute");
        assert_eq!(result.as_number_coerce().expect("Expected 100"), 100.0);
    }

    #[test]
    fn test_comptime_fn_not_callable_at_runtime() {
        // Calling a comptime fn at runtime should produce a clear compile error
        let code = r#"
            comptime fn secret() {
                42
            }
            secret()
        "#;
        let program = shape_ast::parser::parse_program(code).expect("parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "Calling comptime fn at runtime should fail"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("comptime"),
            "Error should mention comptime: {}",
            err_msg
        );
    }
}
