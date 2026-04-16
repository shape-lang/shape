//! Compile-time (comptime) execution infrastructure.
//!
//! Provides a mini-VM executor that compiles and runs statements at compile time,
//! used for meta function methods with statement bodies.

use crate::bytecode::BytecodeProgram;
use crate::compiler::BytecodeCompiler;
use crate::executor::{VMConfig, VirtualMachine};
use shape_ast::ast::{
    AnnotationHandlerParam, DestructurePattern, Expr, FunctionDef, FunctionParameter, Item,
    ObjectEntry, ObjectTypeField, Program, Span, Statement, TypeAnnotation, VarKind, VariableDecl,
};
use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

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

#[derive(Debug, Clone)]
pub(crate) struct ComptimeExecutionResult {
    pub value: ValueWord,
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
    mut runtime_module_bindings: Vec<(String, ValueWord)>,
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

    rebind_typed_object_bindings_to_bytecode_schemas(&bytecode, &mut runtime_module_bindings);

    for module in &all_extensions {
        ensure_module_object_schema(&mut bytecode, module);
    }

    // Execute inside a function that guarantees a tokio runtime is available.
    // Extension async functions (wrapped by populate_module_objects) need
    // `tokio::runtime::Handle::current()` to work.
    if runtime_module_bindings.is_empty() {
        execute_in_runtime(bytecode, &all_extensions)
    } else {
        execute_in_runtime_with_module_bindings(bytecode, &all_extensions, runtime_module_bindings)
    }
}

fn rebind_typed_object_bindings_to_bytecode_schemas(
    bytecode: &BytecodeProgram,
    module_bindings: &mut [(String, ValueWord)],
) {
    use shape_value::{HeapValue, ValueSlot};

    for (_, value) in module_bindings.iter_mut() {
        let Some(field_map) = shape_runtime::type_schema::typed_object_to_hashmap_nb(value) else {
            continue;
        };

        let mut field_names: Vec<&str> = field_map.keys().map(|k| k.as_str()).collect();
        field_names.sort_unstable();

        let target_schema = bytecode
            .type_schema_registry
            .type_names()
            .filter_map(|name| bytecode.type_schema_registry.get(name))
            .filter(|schema| {
                field_names
                    .iter()
                    .all(|name| schema.get_field(name).is_some())
            })
            .min_by_key(|schema| schema.fields.len());

        let Some(schema) = target_schema else {
            continue;
        };

        let mut slots: Vec<ValueSlot> = Vec::with_capacity(schema.fields.len());
        let mut heap_mask: u64 = 0;
        for (idx, field) in schema.fields.iter().enumerate() {
            let field_value = field_map
                .get(&field.name)
                .cloned()
                .unwrap_or_else(ValueWord::none);
            let (slot, is_heap) = ValueSlot::from_value_word(&field_value);
            slots.push(slot);
            if is_heap && idx < 64 {
                heap_mask |= 1u64 << idx;
            }
        }

        *value = ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema.id as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        });
    }
}

/// Run the compiled bytecode on a fresh VM with extensions registered.
/// Ensures a tokio runtime exists for async extension function support.
fn execute_in_runtime(
    bytecode: BytecodeProgram,
    extensions: &[shape_runtime::module_exports::ModuleExports],
) -> Result<ComptimeExecutionResult> {
    execute_in_runtime_with_module_bindings(bytecode, extensions, Vec::new())
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
/// Returns the ValueWord result of the handler execution.
#[cfg(test)]
pub(crate) fn execute_comptime_with_target(
    handler_body: &Expr,
    handler_param: &str,
    target_value: ValueWord,
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

pub(crate) fn execute_comptime_with_annotation_handler(
    handler_body: &Expr,
    handler_params: &[AnnotationHandlerParam],
    target_value: ValueWord,
    annotation_args: &[Expr],
    annotation_def_param_names: &[String],
    const_bindings: &[(String, ValueWord)],
    comptime_helpers: &[FunctionDef],
    extensions: &[shape_runtime::module_exports::ModuleExports],
    trait_impl_keys: std::collections::HashSet<String>,
    known_type_symbols: std::collections::HashSet<String>,
) -> Result<ComptimeExecutionResult> {
    if handler_params.iter().filter(|p| p.is_variadic).count() > 1 {
        return Err(ShapeError::RuntimeError {
            message: "comptime annotation handlers support at most one variadic parameter"
                .to_string(),
            location: None,
        });
    }
    if let Some((idx, _)) = handler_params
        .iter()
        .enumerate()
        .find(|(_, p)| p.is_variadic)
    {
        if idx != handler_params.len().saturating_sub(1) {
            return Err(ShapeError::RuntimeError {
                message: "variadic comptime annotation handler parameter must be last".to_string(),
                location: None,
            });
        }
    }

    let params: Vec<FunctionParameter> = handler_params
        .iter()
        .enumerate()
        .map(|(idx, p)| FunctionParameter {
            pattern: DestructurePattern::Identifier(p.name.clone(), Span::DUMMY),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: if idx == 0 {
                Some(comptime_target_param_type())
            } else if idx == 1 {
                Some(TypeAnnotation::Object(Vec::new()))
            } else {
                None
            },
            default_value: None,
        })
        .collect();

    let mut call_args: Vec<Expr> = Vec::with_capacity(handler_params.len());
    let mut ann_idx = 0usize;
    for (idx, param) in handler_params.iter().enumerate() {
        if idx == 0 {
            call_args.push(Expr::Identifier("__target_arg__".to_string(), Span::DUMMY));
            continue;
        }
        if idx == 1 {
            call_args.push(Expr::Identifier("__ctx_arg__".to_string(), Span::DUMMY));
            continue;
        }
        if param.is_variadic {
            call_args.push(Expr::Array(
                annotation_args.get(ann_idx..).unwrap_or_default().to_vec(),
                Span::DUMMY,
            ));
            ann_idx = annotation_args.len();
            continue;
        }
        let Some(arg) = annotation_args.get(ann_idx) else {
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "missing annotation argument for comptime handler parameter '{}'",
                    param.name
                ),
                location: None,
            });
        };
        call_args.push(arg.clone());
        ann_idx += 1;
    }
    // If the handler has extra declared params beyond (target, ctx) that explicitly
    // consume annotation args, enforce that all args are consumed. But if the handler
    // only declares (target, ctx), silently ignore leftover annotation args — those
    // are the annotation definition's own params and may only be used in before/after hooks.
    let extra_handler_params = handler_params.len().saturating_sub(2);
    if extra_handler_params > 0
        && ann_idx < annotation_args.len()
        && !handler_params.iter().any(|p| p.is_variadic)
    {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "too many annotation arguments: expected {}, got {}",
                ann_idx,
                annotation_args.len()
            ),
            location: None,
        });
    }

    // If the handler only has (target, ctx) but the annotation definition has params,
    // inject them as extra function params so the handler body can reference them by name.
    let mut params = params;
    if extra_handler_params == 0 && !annotation_def_param_names.is_empty() {
        for (i, def_param_name) in annotation_def_param_names.iter().enumerate() {
            if let Some(arg) = annotation_args.get(i) {
                params.push(FunctionParameter {
                    pattern: DestructurePattern::Identifier(def_param_name.clone(), Span::DUMMY),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    type_annotation: None,
                    default_value: None,
                });
                call_args.push(arg.clone());
            }
        }
    }

    // Keep comptime ctx structured so annotations can grow into richer APIs.
    let ctx_nb = shape_runtime::type_schema::typed_object_from_nb_pairs(&[]);

    // Wrap the handler body in a function that takes the target parameter.
    let func_name = "__comptime_handler_fn__".to_string();
    let func_def = FunctionDef {
        name: func_name.clone(),
        name_span: Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        params,
        return_type: None,
        body: vec![Statement::Return(Some(handler_body.clone()), Span::DUMMY)],
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
    for (name, value) in const_bindings {
        let expr = nb_to_expr(value, Span::DUMMY).map_err(|message| ShapeError::RuntimeError {
            message: format!(
                "failed to materialize comptime const binding '{}': {}",
                name, message
            ),
            location: None,
        })?;
        items.push(Item::VariableDecl(
            VariableDecl {
                kind: VarKind::Const,
                is_mut: false,
                pattern: DestructurePattern::Identifier(name.clone(), Span::DUMMY),
                type_annotation: None,
                value: Some(expr),
                ownership: Default::default(),
            },
            Span::DUMMY,
        ));
    }
    items.push(Item::Function(func_def, Span::DUMMY));
    items.push(Item::Expression(
        Expr::FunctionCall {
            name: func_name,
            args: call_args,
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
        vec![
            "__target_arg__".to_string(),
            "__ctx_arg__".to_string(),
            "__comptime__".to_string(),
        ],
        vec![
            ("__target_arg__".to_string(), target_value),
            ("__ctx_arg__".to_string(), ctx_nb),
        ],
        extensions,
        trait_impl_keys,
        known_type_symbols,
    )
}

/// Run compiled bytecode on a fresh VM with extensions and pre-set module_binding variables.
///
/// Returns a normalized ValueWord: TypedObjects are re-registered in the module_binding
/// anonymous schema registry so callers don't need the comptime VM's registry.
fn execute_in_runtime_with_module_bindings(
    bytecode: BytecodeProgram,
    extensions: &[shape_runtime::module_exports::ModuleExports],
    module_bindings: Vec<(String, ValueWord)>,
) -> Result<ComptimeExecutionResult> {
    let run = || -> Result<ComptimeExecutionResult> {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);

        for ext in extensions {
            vm.register_extension(ext.clone());
        }
        vm.populate_module_objects();

        // Set module_binding variables (e.g., __target_arg__)
        for (name, value) in &module_bindings {
            vm.set_module_binding_by_name_nb(name, value.clone());
        }

        // Set up 5-second timeout
        let interrupt = Arc::new(AtomicU8::new(0));
        vm.set_interrupt(interrupt.clone());

        let timeout_interrupt = interrupt.clone();
        let _timer_handle = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            timeout_interrupt.store(1, Ordering::SeqCst);
        });

        super::comptime_builtins::clear_comptime_directives();
        let result = vm.execute(None).map_err(|e| ShapeError::RuntimeError {
            message: format!("Comptime handler execution failed: {}", e),
            location: None,
        })?;
        let directives = super::comptime_builtins::take_comptime_directives();

        // Normalize TypedObjects so callers don't need
        // access to the comptime VM's schema registry.
        Ok(ComptimeExecutionResult {
            value: normalize_comptime_value(&result, &vm).clone(),
            directives,
        })
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        run()
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ShapeError::RuntimeError {
                message: format!("Failed to create tokio runtime for comptime: {}", e),
                location: None,
            })?;
        rt.block_on(async { run() })
    }
}

/// Normalize a comptime result by re-packaging TypedObjects into portable
/// TypedObjects that live in the module_binding anonymous schema registry.
///
/// The comptime VM's schema IDs are only valid within its own registry.
/// We extract the fields and rebuild via `typed_object_from_nb_pairs`, which
/// registers an anonymous schema in the module_binding registry so the result can
/// be consumed by the outer compiler.
fn normalize_comptime_value(nb: &ValueWord, vm: &VirtualMachine) -> ValueWord {
    use shape_runtime::type_schema::{register_predeclared_any_schema, typed_object_from_nb_pairs};
    use shape_value::heap_value::HeapValue;

    // Handle unified arrays.
    if let Some(view) = nb.as_any_array() {
        let normalized: Vec<ValueWord> = (0..view.len())
            .map(|i| {
                let elem = view.get_nb(i).unwrap_or_else(ValueWord::none);
                normalize_comptime_value(&elem, vm)
            })
            .collect();
        return ValueWord::from_array(Arc::new(normalized));
    }

    // cold-path: as_heap_ref retained — comptime value normalization
    match nb.as_heap_ref() { // cold-path
        Some(HeapValue::TypedObject {
            schema_id,
            slots,
            heap_mask,
        }) => {
            let schema = vm.lookup_schema(*schema_id as u32);
            let mut pairs: Vec<(String, ValueWord)> = Vec::new();
            if let Some(schema) = schema {
                for field_def in schema.fields.iter() {
                    let idx = field_def.index as usize;
                    if idx < slots.len() {
                        let field_nb = if *heap_mask & (1u64 << idx) != 0 {
                            let heap_nb = slots[idx].as_heap_nb();
                            normalize_comptime_value(&heap_nb, vm)
                        } else {
                            match field_def.field_type {
                                shape_runtime::type_schema::FieldType::I64 => {
                                    ValueWord::from_i64(slots[idx].as_i64())
                                }
                                shape_runtime::type_schema::FieldType::Bool => {
                                    ValueWord::from_bool(slots[idx].as_f64() != 0.0)
                                }
                                _ => ValueWord::from_f64(slots[idx].as_f64()),
                            }
                        };
                        pairs.push((field_def.name.clone(), field_nb));
                    }
                }
            }
            let pair_refs: Vec<(&str, ValueWord)> =
                pairs.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            let field_names: Vec<String> = pairs.iter().map(|(k, _)| k.clone()).collect();
            let _ = register_predeclared_any_schema(&field_names);
            typed_object_from_nb_pairs(&pair_refs)
        }
        Some(HeapValue::Array(arr)) => {
            let normalized: Vec<ValueWord> = arr
                .iter()
                .map(|elem_nb| normalize_comptime_value(elem_nb, vm))
                .collect();
            ValueWord::from_array(Arc::new(normalized))
        }
        _ => nb.clone(),
    }
}

/// Convert a ValueWord (comptime execution result) to an AST Literal for compilation.
///
/// Used to replace `Expr::Comptime` nodes with their evaluated literal values.
pub(crate) fn vmvalue_to_literal(value: &ValueWord) -> shape_ast::ast::Literal {
    nb_to_literal(&value.clone())
}

/// Convert a ValueWord (comptime execution result) to an AST Literal for compilation.
///
/// Used by comptime for-loop unrolling where elements are already ValueWord.
pub(crate) fn nb_to_literal(nb: &ValueWord) -> shape_ast::ast::Literal {
    use shape_ast::ast::Literal;
    use shape_runtime::type_system::annotation_to_string;
    use shape_value::heap_value::HeapValue;

    use shape_value::tags::{is_tagged, get_tag, TAG_INT, TAG_BOOL, TAG_NONE, TAG_UNIT, TAG_HEAP};
    let bits = nb.raw_bits();
    if !is_tagged(bits) {
        return Literal::Number(nb.as_f64().unwrap_or(0.0));
    }
    match get_tag(bits) {
        TAG_INT => Literal::Int(nb.as_i64().unwrap_or(0)),
        TAG_BOOL => Literal::Bool(nb.as_bool().unwrap_or(false)),
        TAG_NONE => Literal::None,
        TAG_UNIT => Literal::Unit,
        TAG_HEAP => {
            if let Some(s) = nb.as_str() {
                Literal::String(s.to_string())
            } else if let Some(d) = nb.as_decimal() {
                Literal::Decimal(d)
            // cold-path: as_heap_ref retained — comptime literal conversion
            } else if let Some(HeapValue::Rare(shape_value::RareHeapData::TypeAnnotation(ann))) = nb.as_heap_ref() { // cold-path
                // Comptime substitution currently supports literal splicing only.
                // Preserve type-query usefulness by materializing canonical type text.
                Literal::String(annotation_to_string(ann))
            } else {
                // For complex types that don't have literal representations,
                // fall back to a string representation
                Literal::String(format!("{}", nb))
            }
        }
        // Function/ModuleFunction don't have literal representations
        _ => Literal::String(format!("{}", nb)),
    }
}

/// Public entry point for converting a comptime ValueWord to an AST expression.
pub(crate) fn nb_to_expr_public(nb: &ValueWord, span: Span) -> std::result::Result<Expr, String> {
    nb_to_expr(nb, span)
}

fn nb_to_expr(nb: &ValueWord, span: Span) -> std::result::Result<Expr, String> {
    use shape_value::heap_value::HeapValue;

    if let Some(view) = nb.as_any_array() {
        let arr = view.to_generic();
        let mut values = Vec::with_capacity(arr.len());
        for value in arr.iter() {
            values.push(nb_to_expr(value, span)?);
        }
        return Ok(Expr::Array(values, span));
    }

    if let Some(fields) = shape_runtime::type_schema::typed_object_to_hashmap_nb(nb) {
        let mut names: Vec<String> = fields.keys().cloned().collect();
        names.sort();
        let mut entries = Vec::with_capacity(names.len());
        for name in names {
            let value = fields
                .get(&name)
                .ok_or_else(|| format!("missing typed-object field '{}'", name))?;
            entries.push(ObjectEntry::Field {
                key: name,
                value: nb_to_expr(value, span)?,
                type_annotation: None,
            });
        }
        return Ok(Expr::Object(entries, span));
    }

    if let Some(decimal) = nb.as_decimal() {
        return Ok(Expr::Literal(
            shape_ast::ast::Literal::Decimal(decimal),
            span,
        ));
    }

    if let Some(string) = nb.as_str() {
        return Ok(Expr::Literal(
            shape_ast::ast::Literal::String(string.to_string()),
            span,
        ));
    }

    if let Some(value) = nb.as_i64() {
        return Ok(Expr::Literal(shape_ast::ast::Literal::Int(value), span));
    }

    if let Some(value) = nb.as_f64() {
        return Ok(Expr::Literal(shape_ast::ast::Literal::Number(value), span));
    }

    if let Some(value) = nb.as_bool() {
        return Ok(Expr::Literal(shape_ast::ast::Literal::Bool(value), span));
    }

    if nb.is_none() {
        return Ok(Expr::Literal(shape_ast::ast::Literal::None, span));
    }

    if nb.is_unit() {
        return Ok(Expr::Literal(shape_ast::ast::Literal::Unit, span));
    }

    // cold-path: as_heap_ref retained — comptime literal error reporting
    if let Some(heap) = nb.as_heap_ref() { // cold-path
        return Err(match heap {
            HeapValue::DataTable(_) | HeapValue::TableView(shape_value::TableViewData::TypedTable { .. }) => {
                "table values are not valid comptime literals".to_string()
            }
            HeapValue::TableView(shape_value::TableViewData::ColumnRef { .. }) | HeapValue::TableView(shape_value::TableViewData::RowView { .. }) => {
                "row/column view values are not valid comptime literals".to_string()
            }
            HeapValue::Closure { .. } | HeapValue::HostClosure(_) => {
                "function values are not valid comptime literals".to_string()
            }
            _ => format!("unsupported comptime literal value: {}", nb),
        });
    }

    Err(format!("unsupported comptime literal value: {}", nb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::{BinaryOp, Expr, Literal, Span, Statement};
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
        ext.add_function(
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
        ext.add_function(
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
            ("fields", ValueWord::from_array(Arc::new(vec![]))),
            ("params", ValueWord::from_array(Arc::new(vec![]))),
            ("return_type", ValueWord::none()),
            ("annotations", ValueWord::from_array(Arc::new(vec![]))),
            ("captures", ValueWord::from_array(Arc::new(vec![]))),
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
