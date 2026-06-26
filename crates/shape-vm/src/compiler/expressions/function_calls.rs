//! Function and method call expression compilation

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::compiler::monomorphization::cache::ClosureDefPeek;
use crate::compiler::monomorphization::type_resolution::{
    concrete_type_for_expr, extract_arg_concrete_types, resolve_call_site_type_args,
    resolve_call_site_type_args_with_closures,
};
use crate::compiler::string_interpolation::has_interpolation;
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::{VariableKind, VariableTypeInfo};
use shape_ast::ast::{Expr, InterpolationMode, Literal, ObjectEntry, Span, Spanned, Statement};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::closure::EnvironmentAnalyzer;
use shape_runtime::type_system::Type;
use shape_runtime::type_system::suggestions::suggest_function;
use shape_value::v2::ConcreteType;
use std::collections::BTreeSet;
use std::collections::HashMap;

use super::super::{BuiltinNameResolution, BytecodeCompiler, ModuleBuiltinFunction};

/// Strict-typing-sweep (Cluster 3): map a `NativeKind` (the type-tracker's
/// per-slot storage hint) to an AST `TypeAnnotation`. Used by HOF dispatch
/// to type closure user params from a bare `[1, 2, 3]`-literal receiver
/// when no whole-binding `ConcreteType::Array(elem)` entry exists yet.
fn slot_kind_to_type_annotation(
    kind: crate::type_tracking::NativeKind,
) -> Option<shape_ast::ast::TypeAnnotation> {
    use crate::type_tracking::NativeKind;
    use shape_ast::ast::TypeAnnotation;
    Some(match kind {
        NativeKind::Float64 => TypeAnnotation::Basic("number".to_string()),
        NativeKind::Int64 => TypeAnnotation::Basic("int".to_string()),
        NativeKind::Int32 => TypeAnnotation::Basic("i32".to_string()),
        NativeKind::Int16 => TypeAnnotation::Basic("i16".to_string()),
        NativeKind::Int8 => TypeAnnotation::Basic("i8".to_string()),
        NativeKind::UInt64 => TypeAnnotation::Basic("u64".to_string()),
        NativeKind::UInt32 => TypeAnnotation::Basic("u32".to_string()),
        NativeKind::UInt16 => TypeAnnotation::Basic("u16".to_string()),
        NativeKind::UInt8 => TypeAnnotation::Basic("u8".to_string()),
        NativeKind::Bool => TypeAnnotation::Basic("bool".to_string()),
        NativeKind::String => TypeAnnotation::Basic("string".to_string()),
        // Other kinds (Decimal, BigInt, DateTime, nullable variants,
        // pointers, etc.) are not productive for typed binary-op emission;
        // returning None lets the closure body compile with no annotation,
        // which is identical to the pre-fix behaviour.
        _ => return None,
    })
}

/// Task #108 companion: rewrite a return-type annotation by prefixing
/// any bare `Basic`/`Reference` names with the given namespace. Module-
/// qualified callees (`m::mk` returns `P`) carry their return type in
/// bare form even though the schema is registered as `m::P`; we use this
/// only as a fallback when the bare-name schema lookup misses, so type
/// info propagates through to a downstream `m::mk().x` property access
/// and the GetProp emit site can record its native-kind hint. Returns
/// `None` when the annotation already qualifies (`m::P`) or is shaped
/// such that prefixing wouldn't help (`Object`, `Function`, `Tuple`, …).
fn qualify_type_annotation_with_namespace(
    ann: &shape_ast::ast::TypeAnnotation,
    namespace: &str,
) -> Option<shape_ast::ast::TypeAnnotation> {
    use shape_ast::ast::TypeAnnotation;
    match ann {
        TypeAnnotation::Basic(name) if !name.contains("::") => {
            Some(TypeAnnotation::Basic(format!("{}::{}", namespace, name)))
        }
        TypeAnnotation::Reference(name) if !name.as_str().contains("::") => Some(
            TypeAnnotation::Reference(format!("{}::{}", namespace, name.as_str()).into()),
        ),
        _ => None,
    }
}

/// WS-9c: project a `FieldType` to the `TypeAnnotation` used as an
/// object-field contract. Unlike `field_type_to_annotation` (which refuses
/// `Array`/`Any`/`Option` so the caller falls back to the inference engine),
/// this best-effort projection records a contract for every field that has
/// a representable annotation; `Any` and unrepresentable shapes yield `None`
/// and simply carry no contract (the field stays an honest `unknown`).
pub(crate) fn field_type_contract_annotation(
    ft: &shape_runtime::type_schema::FieldType,
) -> Option<shape_ast::ast::TypeAnnotation> {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_schema::FieldType;
    let basic = |s: &str| Some(TypeAnnotation::Basic(s.to_string()));
    match ft {
        FieldType::String => basic("string"),
        FieldType::I64 => basic("int"),
        FieldType::F64 => basic("number"),
        FieldType::Bool => basic("bool"),
        FieldType::Decimal => basic("decimal"),
        FieldType::Timestamp => basic("DateTime"),
        FieldType::I8 => basic("i8"),
        FieldType::U8 => basic("u8"),
        FieldType::I16 => basic("i16"),
        FieldType::U16 => basic("u16"),
        FieldType::I32 => basic("i32"),
        FieldType::U32 => basic("u32"),
        FieldType::U64 => basic("u64"),
        FieldType::Object(name) => Some(TypeAnnotation::Reference(name.as_str().into())),
        FieldType::Array(inner) => field_type_contract_annotation(inner)
            .map(|inner_ann| TypeAnnotation::Array(Box::new(inner_ann))),
        FieldType::Option(inner) => {
            field_type_contract_annotation(inner).map(TypeAnnotation::option)
        }
        // W17.3-4.1 — project HashMap<K, V> / Set<T> back to the
        // surface `TypeAnnotation::Generic { name, args }` shape the
        // parser emits. Inner contract projection is best-effort:
        // mirrors the existing Array/Option `?`-style propagation so
        // a container with an unrepresentable inner falls back to
        // `None` (the field stays an honest `unknown`).
        FieldType::HashMap { key, value } => {
            let k = field_type_contract_annotation(key)?;
            let v = field_type_contract_annotation(value)?;
            Some(TypeAnnotation::Generic {
                name: shape_ast::ast::type_path::TypePath::simple("HashMap"),
                args: vec![k, v],
            })
        }
        FieldType::Set(inner) => {
            let elem = field_type_contract_annotation(inner)?;
            Some(TypeAnnotation::Generic {
                name: shape_ast::ast::type_path::TypePath::simple("Set"),
                args: vec![elem],
            })
        }
        FieldType::Any => None,
    }
}

// U4-4: `return_type_to_numeric`, `builtin_return_numeric_type`, and
// `method_return_numeric_type` are DELETED. They computed a `NumericType` for
// the sole purpose of stamping the deleted `last_expr_numeric_type` register
// for call/method results (`floor(3.7) + 1 => 4`, `pow(sin(x),2.0)+...`). The
// inference engine already proves these return types (it recorded them in the
// span-keyed `resolved_expr_types` table), so a downstream binop now derives
// the call result's numeric kind via `numeric_type_of` → `infer_expr_type` —
// one source, no register-feeding tables.

/// Conservative compile-time-constant check for const parameters.
/// Accepts literals and recursively literal-composed containers.
fn is_compile_time_const_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_, _) => true,
        Expr::UnaryOp { operand, .. } => is_compile_time_const_expr(operand),
        Expr::BinaryOp { left, right, .. } => {
            is_compile_time_const_expr(left) && is_compile_time_const_expr(right)
        }
        Expr::Array(items, _) => items.iter().all(is_compile_time_const_expr),
        Expr::Object(entries, _) => entries
            .iter()
            .all(|entry| matches!(entry, shape_ast::ast::ObjectEntry::Field { value, .. } if is_compile_time_const_expr(value))),
        _ => false,
    }
}

fn literal_const_slot(literal: &Literal) -> Option<shape_value::KindedSlot> {
    use shape_value::{KindedSlot, NativeKind, ValueSlot};
    Some(match literal {
        Literal::Int(value) => KindedSlot::from_int(*value),
        Literal::UInt(value) => KindedSlot::new(ValueSlot::from_u64(*value), NativeKind::UInt64),
        Literal::TypedInt(value, _) => KindedSlot::from_int(*value),
        Literal::Number(value) => KindedSlot::from_number(*value),
        Literal::String(value) => KindedSlot::from_string(value),
        Literal::Char(value) => KindedSlot::from_char(*value),
        Literal::Bool(value) => KindedSlot::from_bool(*value),
        Literal::None | Literal::Unit => KindedSlot::none(),
        Literal::Decimal(_) | Literal::FormattedString { .. } | Literal::Timeframe(_) => {
            return None;
        }
    })
}

fn const_expr_literal(expr: &Expr) -> Option<&Literal> {
    match expr {
        Expr::Literal(literal, _) => Some(literal),
        _ => None,
    }
}

fn const_expr_fingerprint(expr: &Expr) -> Option<String> {
    const_expr_literal(expr).map(|literal| format!("{:?}", literal))
}

pub(crate) enum ConstFoldValue {}

pub(crate) fn eval_const_expr_to_nanboxed(expr: &Expr) -> Option<ConstFoldValue> {
    let _ = expr;
    None
}

impl BytecodeCompiler {
    pub(crate) fn hidden_native_module_binding_name(module_path: &str) -> String {
        format!("__imported_module__::{}", module_path)
    }

    fn ensure_hidden_native_module_binding(&mut self, module_path: &str) -> String {
        let binding_name = Self::hidden_native_module_binding_name(module_path);
        if !self.module_bindings.contains_key(&binding_name) {
            let binding_idx = self.get_or_create_module_binding(&binding_name);
            self.register_extension_module_schema(module_path);
            let module_schema_name = format!("__mod_{}", module_path);
            if self
                .type_tracker
                .schema_registry()
                .get(&module_schema_name)
                .is_some()
            {
                self.set_module_binding_type_info(binding_idx, &module_schema_name);
            }
        }
        binding_name
    }

    fn compile_module_builtin_function_call(
        &mut self,
        builtin_decl: &ModuleBuiltinFunction,
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        if !self
            .is_native_module_export(&builtin_decl.source_module_path, &builtin_decl.export_name)
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "builtin function '{}' has no runtime implementation in module '{}'",
                    builtin_decl.export_name, builtin_decl.source_module_path
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }
        // R8 W9 B1 W17-marshal-return JIT surface-and-stop flag
        // (2026-05-25). `builtin fn` declarations like
        // `from std::core::state use { serialize }` route through this
        // helper which calls `compile_module_namespace_call_on_binding`
        // — emitting a `LoadModuleBinding(idx) + GetFieldTyped(...) +
        // CallValue` sequence whose callee is a `Ptr(HeapKind::ModuleFn)`
        // (see ADR-006 §2.7.26 amendment). At runtime VM-side this
        // routes cleanly through `invoke_module_fn_id_stub` +
        // `project_typed_return`; JIT-side `jit_call_value` ModuleFn
        // arm at `ffi/control/mod.rs:704-715` silently returns TAG_NULL
        // — silent-wrong-output. Set the flag so the JIT preflight
        // refuses and deopts to the bytecode interpreter via the W12
        // `[jit-fallback]` path. Root-cause fix in JIT ModuleFn dispatch
        // (`dispatch_module_fn_call` `todo!()` + the §2.7.10/Q11 kinded
        // handler ABI rebuild) is v0.4 per
        // `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup.
        // Restrict to user-space main compilation. Dep-module bodies
        // execute their internal stdlib calls only when transitively
        // reachable from main; setting the flag during dep-module
        // compilation would poison every program that imports any
        // stdlib (e.g. s1's `let mut sum = 0; for i in 0..100 {...}`
        // pulls in `std::core::remote::__call` during stdlib bootstrap
        // even though main never invokes it).
        if self.module_scope_stack.is_empty() {
            self.program.has_w17_marshal_residual = true;
        }
        let binding_name =
            self.ensure_hidden_native_module_binding(&builtin_decl.source_module_path);
        self.compile_module_namespace_call_on_binding(
            &binding_name,
            &builtin_decl.source_module_path,
            span,
            &builtin_decl.export_name,
            args,
        )
    }

    fn resolve_scoped_module_builtin_function(&self, name: &str) -> Option<ModuleBuiltinFunction> {
        if let Some(decl) = self.module_builtin_functions.get(name) {
            return Some(decl.clone());
        }

        for module_path in self.module_scope_stack.iter().rev() {
            let candidate = format!("{}::{}", module_path, name);
            if let Some(decl) = self.module_builtin_functions.get(&candidate) {
                return Some(decl.clone());
            }
        }
        None
    }

    fn extract_table_schema_from_annotation(
        &mut self,
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<(u32, String)> {
        let shape_ast::ast::TypeAnnotation::Generic { name, args } = ann else {
            return None;
        };
        if name != "Table" || args.len() != 1 {
            return None;
        }

        match &args[0] {
            shape_ast::ast::TypeAnnotation::Basic(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| (schema.id, name.clone())),
            shape_ast::ast::TypeAnnotation::Reference(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| (schema.id, name.to_string())),
            shape_ast::ast::TypeAnnotation::Object(fields) => {
                // Register the inline schema with typed field info so downstream
                // RowView field accesses (`row.open`) can resolve column type
                // and emit typed LoadCol* opcodes / numeric-type hints.
                let typed_fields: Vec<(&str, shape_runtime::type_schema::FieldType)> = fields
                    .iter()
                    .map(|field| {
                        let ft =
                            BytecodeCompiler::type_annotation_to_field_type(&field.type_annotation);
                        (field.name.as_str(), ft)
                    })
                    .collect();
                let schema_id = self
                    .type_tracker
                    .register_inline_object_schema_typed(&typed_fields);
                // Also register field contracts so downstream callable-field
                // unwrapping (e.g. nested `() => Table<{...}>` returns) and
                // any contract-based field lookups see the annotated types.
                let mut contracts = std::collections::HashMap::with_capacity(fields.len());
                for field in fields {
                    contracts.insert(field.name.clone(), field.type_annotation.clone());
                }
                self.type_tracker
                    .register_object_field_contracts(schema_id, contracts);
                let schema_name = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .map(|schema| schema.name.clone())
                    .unwrap_or_else(|| format!("__anon_{}", schema_id));
                Some((schema_id, schema_name))
            }
            _ => None,
        }
    }

    fn extract_object_schema_id_from_annotation(
        &mut self,
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<u32> {
        let shape_ast::ast::TypeAnnotation::Object(fields) = ann else {
            return None;
        };
        // W17.2-C §4.D.5 migration: route through the typed variant
        // with FieldType::Any per field (NOT per-field type lowering
        // via type_annotation_to_field_type — that path changes the
        // schema layout vs the pre-existing Any-typed shape, which
        // breaks downstream consumers that depend on the legacy
        // Any-uniform field layout). The `register_object_field_contracts`
        // call below STILL preserves per-field TypeAnnotation contracts
        // so downstream callable-field unwrapping + JIT lookups see
        // the annotated types. The verification-pass safety net
        // catches via the `__inline_obj_*` transitional row.
        // Per audit §4.D.5 PROPAGATE deferred to v0.4 W17.3+ for the
        // per-field-typed schema layout migration. ADR-006 §2.7.5
        // producer-side stamp preserved at the contract layer
        // (`register_object_field_contracts`).
        let typed_fields: Vec<(&str, shape_runtime::type_schema::FieldType)> = fields
            .iter()
            .map(|field| {
                (
                    field.name.as_str(),
                    shape_runtime::type_schema::FieldType::Any,
                )
            })
            .collect();
        let schema_id = self
            .type_tracker
            .register_inline_object_schema_typed(&typed_fields);
        let mut map = std::collections::HashMap::with_capacity(fields.len());
        for field in fields {
            map.insert(field.name.clone(), field.type_annotation.clone());
        }
        self.type_tracker
            .register_object_field_contracts(schema_id, map);
        Some(schema_id)
    }

    /// WS-9c: derive/register the inline anonymous schema for an unannotated
    /// function whose canonical inferred signature returns a structural object.
    ///
    /// This is intentionally a projection from `InferenceFacts` at the read
    /// site, not a parallel function-name side table. The schema remains
    /// Any-uniform to match object-literal layout, while field contracts carry
    /// the precise inferred field annotations for downstream field lookups.
    pub(crate) fn inferred_return_object_schema_id(&mut self, call_name: &str) -> Option<u32> {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_schema::FieldType;

        let mut candidates = Vec::new();
        let mut push_candidate = |candidate: String| {
            if !candidates.iter().any(|existing| existing == &candidate) {
                candidates.push(candidate);
            }
        };
        push_candidate(call_name.to_string());
        if let Some(scoped) = self.resolve_scoped_module_binding_name(call_name) {
            push_candidate(scoped);
        }
        if let Some((_, tail)) = call_name.rsplit_once("::") {
            push_candidate(tail.to_string());
        }

        let fields: Vec<(String, shape_runtime::type_schema::FieldType)> =
            candidates.iter().find_map(|candidate| {
                let Type::Function { returns, .. } =
                    self.inference_facts.function_signature(candidate)?
                else {
                    return None;
                };
                let Type::Concrete(TypeAnnotation::Object(obj_fields)) = returns.as_ref() else {
                    return None;
                };
                if obj_fields.is_empty() {
                    return None;
                }
                Some(
                    obj_fields
                        .iter()
                        .map(|field| {
                            (
                                field.name.clone(),
                                Self::type_annotation_to_field_type(&field.type_annotation),
                            )
                        })
                        .collect(),
                )
            })?;

        let typed_fields: Vec<(&str, FieldType)> = fields
            .iter()
            .map(|(name, _)| (name.as_str(), FieldType::Any))
            .collect();
        let schema_id = self
            .type_tracker
            .register_inline_object_schema_typed(&typed_fields);
        let mut contracts = HashMap::with_capacity(fields.len());
        for (name, field_ty) in &fields {
            if let Some(ann) = field_type_contract_annotation(field_ty) {
                contracts.insert(name.clone(), ann);
            }
        }
        if !contracts.is_empty() {
            self.type_tracker
                .register_object_field_contracts(schema_id, contracts);
        }
        Some(schema_id)
    }

    /// WS-9c: build a `VariableTypeInfo` for an unannotated function whose
    /// inferred return type is an anonymous structural object.
    fn inline_schema_for_inferred_return(&mut self, call_name: &str) -> Option<VariableTypeInfo> {
        let schema_id = self.inferred_return_object_schema_id(call_name)?;
        let schema_name = self
            .type_tracker
            .schema_registry()
            .get_by_id(schema_id)
            .map(|schema| schema.name.clone())
            .unwrap_or_else(|| format!("__anon_{}", schema_id));
        Some(VariableTypeInfo::known(schema_id, schema_name))
    }

    fn type_info_from_annotation(
        &mut self,
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<VariableTypeInfo> {
        match ann {
            shape_ast::ast::TypeAnnotation::Generic { name, .. } if name == "Table" => self
                .extract_table_schema_from_annotation(ann)
                .map(|(schema_id, type_name)| VariableTypeInfo::datatable(schema_id, type_name)),
            shape_ast::ast::TypeAnnotation::Object(_) => {
                let schema_id = self.extract_object_schema_id_from_annotation(ann)?;
                let schema_name = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .map(|schema| schema.name.clone())
                    .unwrap_or_else(|| format!("__anon_{}", schema_id));
                Some(VariableTypeInfo::known(schema_id, schema_name))
            }
            shape_ast::ast::TypeAnnotation::Basic(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| VariableTypeInfo::known(schema.id, name.clone())),
            shape_ast::ast::TypeAnnotation::Reference(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| VariableTypeInfo::known(schema.id, name.to_string())),
            // PB2-fix #8 (`let r = inner()` where `inner -> Result<T>`):
            // stamp the binding with the baked wrapper-type-name string so
            // `propagate_assignment_type_to_slot` records it on the slot.
            // `compile_expr_try_operator::stamp_unwrapped_success_type`
            // peels the success arm out of that baked name (already handled
            // by `first_generic_arg_of_baked_name`) and stamps the unwrapped
            // type onto the downstream `let v = r?` binding. Narrow to the
            // two fallible-wrapper names — `Result` and `Option` — so this
            // does not regress other generic returns (`Array<T>` etc. have
            // their own `is_array_type_name` propagation path).
            shape_ast::ast::TypeAnnotation::Generic { name, .. }
                if name == "Result" || name == "Option" =>
            {
                Some(VariableTypeInfo::named(ann.to_type_string()))
            }
            _ => None,
        }
    }

    fn type_info_from_inferred_type(&mut self, inferred: &Type) -> Option<VariableTypeInfo> {
        let ann = inferred.to_annotation()?;
        self.type_info_from_annotation(&ann)
    }

    fn table_schema_from_type_info(type_info: &VariableTypeInfo) -> Option<(u32, String)> {
        if type_info.is_datatable() {
            Some((type_info.schema_id?, type_info.type_name.clone()?))
        } else {
            None
        }
    }

    fn table_select_body_expr(body: &[Statement]) -> Option<&Expr> {
        match body {
            [Statement::Expression(expr, _)] => Some(expr),
            [Statement::Return(Some(expr), _)] => Some(expr),
            _ => None,
        }
    }

    fn row_field_projection_column<'a>(expr: &'a Expr, row_param: &str) -> Option<&'a str> {
        let Expr::PropertyAccess {
            object, property, ..
        } = expr
        else {
            return None;
        };
        match object.as_ref() {
            Expr::Identifier(name, _) if name == row_param => Some(property.as_str()),
            _ => None,
        }
    }

    fn static_table_select_columns(
        &self,
        arg: &Expr,
        schema_id: u32,
        type_name: &str,
    ) -> Result<Option<Vec<String>>> {
        let Expr::FunctionExpr { params, body, .. } = arg else {
            return Ok(None);
        };
        if params.len() != 1 {
            return Ok(None);
        }
        let Some(row_param) = params[0].pattern.as_identifier() else {
            return Ok(None);
        };
        let Some(body_expr) = Self::table_select_body_expr(body) else {
            return Ok(None);
        };

        let mut columns = Vec::new();
        match body_expr {
            Expr::Object(entries, _) => {
                if entries.is_empty() {
                    return Ok(None);
                }
                for entry in entries {
                    let ObjectEntry::Field { key, value, .. } = entry else {
                        return Ok(None);
                    };
                    let Some(column) = Self::row_field_projection_column(value, row_param) else {
                        return Ok(None);
                    };
                    if key != column {
                        return Ok(None);
                    }
                    columns.push(column.to_string());
                }
            }
            _ => {
                let Some(column) = Self::row_field_projection_column(body_expr, row_param) else {
                    return Ok(None);
                };
                columns.push(column.to_string());
            }
        }

        let Some(schema) = self.type_tracker.schema_registry().get_by_id(schema_id) else {
            return Err(ShapeError::SemanticError {
                message: format!("Table.select: schema for '{}' is not registered", type_name),
                location: Some(self.span_to_source_location(arg.span())),
            });
        };
        for column in &columns {
            if !schema.has_field(column) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Table.select: type '{}' has no field '{}'",
                        type_name, column
                    ),
                    location: Some(self.span_to_source_location(arg.span())),
                });
            }
        }

        Ok(Some(columns))
    }

    fn value_schema_from_type_info(type_info: &VariableTypeInfo) -> Option<u32> {
        if matches!(type_info.kind, VariableKind::Value) {
            type_info.schema_id
        } else {
            None
        }
    }

    fn extract_table_schema_from_callable_field(
        &mut self,
        receiver_schema_id: u32,
        field_name: &str,
    ) -> Option<(u32, String)> {
        let field_ann = self
            .type_tracker
            .get_object_field_contract(receiver_schema_id, field_name)?
            .clone();
        let shape_ast::ast::TypeAnnotation::Function { params, returns } = field_ann else {
            return None;
        };
        if !params.is_empty() {
            return None;
        }
        self.extract_table_schema_from_annotation(&returns)
    }

    fn is_native_module_export(&self, module_name: &str, export_name: &str) -> bool {
        self.extension_registry
            .as_ref()
            .and_then(|registry| registry.iter().rev().find(|m| m.name == module_name))
            .is_some_and(|module| module.has_export(export_name))
    }

    fn is_native_module_export_available(&self, module_name: &str, export_name: &str) -> bool {
        self.extension_registry
            .as_ref()
            .and_then(|registry| registry.iter().rev().find(|m| m.name == module_name))
            .is_some_and(|module| module.is_export_available(export_name, self.comptime_mode))
    }

    fn ensure_const_specialization(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<Option<(String, usize)>> {
        let Some(const_param_indices) = self.function_const_params.get(name).cloned() else {
            return Ok(None);
        };
        if const_param_indices.is_empty() {
            return Ok(None);
        }

        let original_def =
            self.function_defs
                .get(name)
                .cloned()
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "const specialization requested for unknown function '{}'",
                        name
                    ),
                    location: None,
                })?;

        let mut key_parts = Vec::with_capacity(const_param_indices.len());
        let mut const_bindings = Vec::with_capacity(const_param_indices.len());
        for idx in const_param_indices {
            let Some(arg) = args.get(idx) else {
                continue;
            };
            let Some(fingerprint) = const_expr_fingerprint(arg) else {
                return Ok(None);
            };
            let Some(literal) = const_expr_literal(arg) else {
                return Ok(None);
            };
            let Some(value) = literal_const_slot(literal) else {
                return Ok(None);
            };
            let param_name = original_def
                .params
                .get(idx)
                .and_then(|p| p.simple_name())
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "const specialization for '{}' requires a named const parameter at position {}",
                        name,
                        idx + 1
                    ),
                    location: None,
                })?
                .to_string();
            key_parts.push(format!("{}={}", param_name, fingerprint));
            const_bindings.push((param_name, value));
        }

        if const_bindings.is_empty() {
            return Ok(None);
        }

        let specialization_key = format!("{}::{}", name, key_parts.join("|"));
        if let Some(&existing_idx) = self.const_specializations.get(&specialization_key) {
            let existing_name = self
                .program
                .functions
                .get(existing_idx)
                .map(|f| f.name.clone())
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "const specialization cache for '{}' points at missing function index {}",
                        name, existing_idx
                    ),
                    location: None,
                })?;
            return Ok(Some((existing_name, existing_idx)));
        }

        let mut specialized_def = original_def;
        specialized_def.name = format!("{}__const_{}", name, self.next_const_specialization_id);
        self.next_const_specialization_id = self.next_const_specialization_id.saturating_add(1);
        if let Some(module_path) = specialized_def.declaring_module_path.clone() {
            for ann in &mut specialized_def.annotations {
                if ann.name.contains("::") {
                    continue;
                }
                let qualified = Self::qualify_module_symbol(&module_path, &ann.name);
                if self.program.compiled_annotations.contains_key(&qualified) {
                    ann.name = qualified;
                }
            }
        }
        let specialized_name = specialized_def.name.clone();

        self.specialization_const_bindings
            .insert(specialized_name.clone(), const_bindings);
        self.register_function(&specialized_def)?;
        let specialized_idx =
            self.find_function(&specialized_name)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "const specialization failed to register function '{}'",
                        specialized_name
                    ),
                    location: None,
                })?;
        self.const_specializations
            .insert(specialization_key.clone(), specialized_idx);

        let compile_result = if specialized_def.declaring_module_path.is_some() {
            // Imported namespace-call specializations are consumer-side clones:
            // the exporting module already compiled definition annotations.
            let mut body_def = specialized_def.clone();
            body_def.annotations.clear();
            self.compile_function(&body_def)
        } else {
            self.compile_function(&specialized_def)
        };

        if let Err(err) = compile_result {
            self.specialization_const_bindings.remove(&specialized_name);
            self.const_specializations.remove(&specialization_key);
            return Err(err);
        }

        Ok(Some((specialized_name, specialized_idx)))
    }

    /// Compile a function call expression
    pub(super) fn compile_expr_function_call(
        &mut self,
        name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        // Numeric-conversion §4 literal adoption (call-argument widening, THE
        // RULE user 2026-06-01): a bare int literal passed to a `number`(f64)
        // parameter IS the number literal (`f(5)` where `fn f(x: number)` ⇒
        // `5.0`). Re-lower each such argument to a `Number` literal BEFORE any
        // downstream `compile_call_args`, so the argument carries Float64 bits
        // and the callee's number param slot is not fed an Int64 constant that
        // the call site bit-reinterprets as f64 (`5` → `2.5e-323`). Compile-time
        // literal re-typing keyed on the callee's DECLARED param annotations
        // (`self.function_defs`); a non-literal int arg is NOT rewritten — the
        // p-var `int`-is-not-`number` rejection stays a compile error. Only the
        // direct-named-user-function path resolves param annotations here; the
        // indirect-callable path keeps the raw args.
        let widened_args: Option<Vec<Expr>> = self.function_defs.get(name).and_then(|def| {
            let params = &def.params;
            let mut changed = false;
            let mut out: Vec<Expr> = Vec::with_capacity(args.len());
            for (i, arg) in args.iter().enumerate() {
                let widened = params.get(i).and_then(|p| {
                    p.type_annotation.as_ref().and_then(|ann| {
                        crate::compiler::literal_widen::widen_int_literal_for_annotation(arg, ann)
                    })
                });
                match widened {
                    Some(w) => {
                        changed = true;
                        out.push(w);
                    }
                    None => out.push(arg.clone()),
                }
            }
            if changed { Some(out) } else { None }
        });
        let args: &[Expr] = widened_args.as_deref().unwrap_or(args);

        // W7 (2026-05-17): `type_info(T)` is a comptime-only builtin per
        // `docs/cluster-audits/v0.3-w7-type_info-comptime-typed-return.md`
        // §4 recommendation (b) — TypeInfo struct return — and §8 Q1-Q5
        // user dispositions. The previous hard-error gate ("type_info has
        // been removed") is replaced by routing through the standard
        // comptime-only-builtin path; bare type-identifier arguments are
        // rewritten to string literals in `comptime::rewrite_type_info_ident_args`
        // mirroring the `implements` precedent.

        // Reject comptime-only builtins outside of comptime blocks.
        // These functions are only available inside `comptime { }` blocks.
        if Self::is_comptime_only_builtin(name) && !self.comptime_mode {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "'{}' is a comptime-only builtin and can only be called inside a `comptime {{ }}` block",
                    name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        // Check locals FIRST — function parameters (and other local variables holding
        // callable values) must take priority over global function lookup.  Without this,
        // `fn apply(f, x) { f(x) }` would fail because `find_function("f")` returns None
        // and the code falls through to "Undefined function" error.
        if self.resolve_local(name).is_some()
            || self.mutable_closure_captures.contains_key(name)
            || self.resolve_scoped_module_binding_name(name).is_some()
        {
            // R8 W9 B1 W17-marshal-return JIT surface-and-stop flag
            // (2026-05-25). Direct call to an imported stdlib function —
            // the callee resolves via `resolve_scoped_module_binding_name`
            // and loads a `Ptr(HeapKind::ModuleFn)` value. At runtime the
            // `CallValue` opcode dispatches via the VM-side
            // `call_value_immediate_nb` ModuleFn arm, which routes to
            // `invoke_module_fn_id_stub` + `project_typed_return` and
            // surfaces cleanly when the typed-return arm hits the
            // W17-marshal-return-arms catch-all at
            // `crates/shape-vm/src/executor/vm_impl/modules.rs:74`.
            //
            // The JIT-side `jit_call_value` ModuleFn arm at
            // `crates/shape-jit/src/ffi/control/mod.rs:704-715` instead
            // returns `TAG_NULL` (= the `-1407374883553280` NaN-box null
            // pattern) silently with only a `tracing::debug!` line —
            // swallowing the W17-marshal-return surface and producing
            // silent-wrong-output (VM=ec1 SURFACE / JIT=ec0 garbage on
            // `print(serialize([1.0,2.0,3.0]).len())`).
            //
            // Mark the program so `JITExecutor::execute_with_jit` deopts
            // to the bytecode interpreter via the existing W12
            // `[jit-fallback]` path — VM == JIT semantics restored via
            // path-convergence. Mirrors R8 W7 G.5 V2-verifier preflight
            // + R8 W8 imported-const-inline surface-and-stop precedents.
            // Root-cause fix in JIT ModuleFn dispatch
            // (`dispatch_module_fn_call` `todo!()` + the §2.7.10/Q11
            // kinded handler ABI rebuild) is v0.4 per
            // `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup.
            // Restrict to user-space main compilation (see
            // `compile_module_builtin_function_call` below for the
            // dep-module-bootstrap rationale).
            if self.resolve_scoped_module_binding_name(name).is_some()
                && self.module_scope_stack.is_empty()
            {
                self.program.has_w17_marshal_residual = true;
            }
            let expected_param_modes = if let Some(local_idx) = self.resolve_local(name) {
                self.local_callable_pass_modes.get(&local_idx).cloned()
            } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
                self.module_bindings
                    .get(&scoped_name)
                    .and_then(|binding_idx| {
                        self.module_binding_callable_pass_modes
                            .get(binding_idx)
                            .cloned()
                    })
            } else {
                None
            };
            let return_reference_summary = self.function_return_reference_summary_for_name(name);
            // Use compile_expr_identifier to correctly load the callee value,
            // handling ref_locals (DerefLoad), mutable closure captures (LoadClosure), etc.
            self.compile_expr_identifier(name, span)?;

            let writebacks = self.compile_call_args(args, expected_param_modes.as_deref())?;

            // Phase F: emit `CallFunctionIndirect` when the callee is a
            // typed callable (`Function<A, R>` parameter or local binding
            // with known callable pass modes) and fits `u16`. The arity
            // travels in the operand so the runtime skips the extra
            // `PushConst` round-trip, and the JIT can pick a
            // `call_indirect` signature from the inferred
            // `FunctionTypeId`. Fallback is the legacy `CallValue` path
            // which reads arity from the stack.
            let prefers_indirect =
                expected_param_modes.is_some() && args.len() <= u16::MAX as usize;
            if prefers_indirect {
                self.emit(Instruction::new(
                    OpCode::CallFunctionIndirect,
                    Some(Operand::Count(args.len() as u16)),
                ));
            } else {
                let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));
                self.emit(Instruction::simple(OpCode::CallValue));
            }
            if !writebacks.is_empty() {
                let result_local = self.declare_temp_local("__call_value_result_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));
                for (shadow_local, binding_idx) in writebacks {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(shadow_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            // U4-6 callable-return deletion: derive the return type of a
            // callable binding structurally from `InferenceFacts` or a retained
            // closure body peek, then render it only for the residual tracker
            // display-name stamp. No slot-indexed return string map remains.
            let tracked_callable_return_type =
                self.callable_binding_return_type(name, Some(args.len()));
            let tracked_callable_rt: Option<String> = tracked_callable_return_type
                .as_ref()
                .and_then(Self::inferred_type_to_hint_name);
            if let Some(rt_name) = tracked_callable_rt.as_ref() {
                match rt_name.as_str() {
                    // U4-4: numeric register stamps replaced by `last_expr_type_info`
                    // name stamps (same shape as the width-aware-int arm). The
                    // numeric storage hint is re-derived from this name (or the
                    // resolved Type) downstream.
                    "int" | "number" | "decimal" => {
                        self.last_expr_type_info = Some(
                            crate::type_tracking::VariableTypeInfo::named(rt_name.clone()),
                        );
                    }
                    other
                        if shape_runtime::type_system::BuiltinTypes::is_integer_type_name(
                            other,
                        ) =>
                    {
                        // Width-aware ints — fall through; the i32/i16/etc.
                        // names round-trip via type_info.
                        self.last_expr_type_info = Some(
                            crate::type_tracking::VariableTypeInfo::named(other.to_string()),
                        );
                    }
                    "string" | "bool" | "char" => {
                        self.last_expr_type_info = Some(
                            crate::type_tracking::VariableTypeInfo::named(rt_name.clone()),
                        );
                    }
                    _ => {}
                }
            }
            if let Some(return_reference_summary) = return_reference_summary {
                self.set_last_expr_reference_result(return_reference_summary.mode, true);
            } else if let Some(borrow_mode) = self.function_declares_borrow_return(name) {
                // ADR-006 §2.7.30 (GapA): a `-> &T` callee with no param-reborrow
                // summary (the PromotedCell ReturnSlot floor) returns a reference
                // value; mark it auto-deref so value position reads THROUGH it.
                self.set_last_expr_reference_result(borrow_mode, true);
                // The returned reference rides the §2.7.30 escape-promote
                // `PromotedCell` carrier, which the JIT has no lowering for (it
                // models refs as per-function stack-cell/field addresses only and
                // would read the raw reference pointer). Force whole-program JIT
                // deopt to the interpreter, which resolves the referent soundly.
                self.program.has_reference_escape_promotion = true;
            } else {
                self.clear_last_expr_reference_result();
            }

            // cluster-2-cw-IB-class-b (2026-05-16, supervisor R3 binding-
            // ratified): value-call return-`ConcreteType` classification
            // at the bytecode-emission layer. ADR-006 §2.7.5 stamp-at-
            // compile-time discipline.
            //
            // When the callee resolves to a local closure binding with a
            // retained body peek (populated at let-binding time by
            // `update_callable_binding_from_expr`), re-run the closure-
            // body return-type inference WITH the caller-context arg
            // types injected as typed-array param hints. If the
            // inference yields a recognised scalar/Array return name,
            // convert it to a `ConcreteType` and stamp the side-table
            // `value_call_return_concrete_types[(call_span,
            // current_function)]`. The MIR conduit's value-call
            // destination pass then projects this onto
            // `top_level_local_concrete_types[dst_slot]` /
            // `function_local_concrete_types[fn_idx][dst_slot]`, the
            // JIT-MIR `slot_kinds` projection picks up the matching
            // `NativeKind`, and downstream consumers (`print`,
            // BinaryOp, etc.) reach their kinded dispatch paths.
            //
            // Class B fixture (inventory §B.2): `let xs: Array<int> =
            // [..]; let f = |inner| inner.sum(); print(f(xs))`. Pre-fix:
            // VM=15 / JIT=NotImplemented(SURFACE, print operand NK=None).
            // Post-fix: VM=15 / JIT=15 (VM == JIT load-bearing).
            //
            // No tag-bit decode, no Bool-default fallback, no fabricated
            // default — when:
            //   • The callee is not a local closure binding, OR
            //   • No retained body peek exists (closure was passed in
            //     from elsewhere, e.g. function parameter), OR
            //   • The closure body's terminal expression cannot be
            //     classified against the caller-context-seeded
            //     param_types (the inference returns None), OR
            //   • The classified return name cannot be mapped back to a
            //     ConcreteType,
            // the side-table receives no entry and the destination slot
            // stays `Void` per §2.7.5.1 / §2.7.7 #9 — the JIT then
            // surfaces honestly at the print dispatch site rather than
            // fabricating a kind.
            // Resolve the closure body peek from either the local slot
            // map or the module-binding slot map. Locals take priority
            // (mirrors the `tracked_callable_rt` chain above).
            let closure_peek: Option<crate::compiler::ClosureBodyPeek> =
                if let Some(local_idx) = self.resolve_local(name) {
                    self.local_callable_closure_bodies.get(&local_idx).cloned()
                } else if let Some(scoped) = self.resolve_scoped_module_binding_name(name) {
                    self.module_bindings.get(&scoped).and_then(|idx| {
                        self.module_binding_callable_closure_bodies
                            .get(idx)
                            .cloned()
                    })
                } else {
                    self.module_bindings.get(name).and_then(|idx| {
                        self.module_binding_callable_closure_bodies
                            .get(idx)
                            .cloned()
                    })
                };
            if let Some(peek) = closure_peek {
                {
                    // Resolve the caller-context arg type names per
                    // argument expression. `concrete_type_for_expr` is
                    // the same resolver the rest of the bytecode-
                    // emission layer uses (covers tracker-recorded
                    // primitives + typed-array bindings via whole-binding
                    // `ConcreteType::Array(elem)` records.
                    // U4-5b: keep the caller-arg ConcreteTypes STRUCTURALLY so
                    // the closure-body param seed (below) reads each arg's array
                    // element type directly from `ConcreteType::Array(elem)` —
                    // never by stripping a `"Vec<elem>"` display string. The
                    // string vec is retained ONLY for the closure-body
                    // mini-inferencer ABI (`..._with_caller_context`, a T3-wave
                    // target), not as a type source.
                    let caller_arg_concrete_types: Vec<Option<shape_value::v2::ConcreteType>> = args
                        .iter()
                        .map(|arg_expr| {
                            crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(self, arg_expr)
                        })
                        .collect();
                    let caller_arg_type_names: Vec<Option<String>> = caller_arg_concrete_types
                        .iter()
                        .map(|ct| {
                            ct.as_ref()
                                .and_then(|ct| {
                                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(ct)
                                })
                                .and_then(|ann| {
                                    crate::compiler::BytecodeCompiler::tracked_type_name_from_annotation(&ann)
                                })
                        })
                        .collect();

                    // Run the closure-body return-type inference with
                    // the caller-context arg types. The inference is
                    // cheap (AST walk over the closure body, no
                    // bytecode emission); running unconditionally covers
                    // both:
                    //   (a) The Class B case: the closure param is
                    //       inferred-typed at the call site (no
                    //       annotation, no body-literal pairing), so
                    //       the let-binding-time inference returned
                    //       None and `tracked_callable_rt` is None.
                    //   (b) The let-binding-time-already-resolved case:
                    //       e.g. `let f = || 15` — the body's terminal
                    //       Literal(Int) is enough at let-binding time,
                    //       `tracked_callable_rt = Some("int")`. Even
                    //       here, the side-table must be populated so
                    //       the JIT-MIR conduit's value-call destination
                    //       pass can stamp `concrete_types[dst]` (the
                    //       let-binding-time tracker recorded only the
                    //       bytecode-side `last_expr_*`, which doesn't
                    //       reach the JIT). The two paths converge on
                    //       the same `ConcreteType::I64` answer here.
                    {
                        // Prefer the let-binding-time result when
                        // present (it consulted the closure body
                        // without needing caller-context); fall through
                        // to the caller-context inference when the
                        // let-binding-time inference returned None.
                        let inferred = tracked_callable_rt
                            .as_ref()
                            .cloned()
                            .or_else(|| {
                                crate::compiler::expressions::closures::infer_closure_body_return_type_name_with_caller_context(
                                    self,
                                    &peek.params,
                                    &peek.body,
                                    peek.return_type.as_ref(),
                                    &[],
                                    &caller_arg_type_names,
                                )
                            });
                        if let Some(rt_name) = inferred {
                            // Map the return name to a ConcreteType.
                            // Mirrors the `tracked_type_name_from_
                            // annotation` → ConcreteType chain used by
                            // `concrete_type_for_expr`. Scalars are
                            // handled directly; `Vec<T>` returns are
                            // not supported here (the typed-array-
                            // returning closure case is Class C's
                            // sibling territory).
                            let ct: Option<shape_value::v2::ConcreteType> = match rt_name.as_str() {
                                "int" | "i64" => Some(shape_value::v2::ConcreteType::I64),
                                "i32" => Some(shape_value::v2::ConcreteType::I32),
                                "i16" => Some(shape_value::v2::ConcreteType::I16),
                                "i8" => Some(shape_value::v2::ConcreteType::I8),
                                "u64" => Some(shape_value::v2::ConcreteType::U64),
                                "u32" => Some(shape_value::v2::ConcreteType::U32),
                                "u16" => Some(shape_value::v2::ConcreteType::U16),
                                "u8" => Some(shape_value::v2::ConcreteType::U8),
                                "number" | "f64" => Some(shape_value::v2::ConcreteType::F64),
                                "bool" => Some(shape_value::v2::ConcreteType::Bool),
                                "string" => Some(shape_value::v2::ConcreteType::String),
                                "decimal" => Some(shape_value::v2::ConcreteType::Decimal),
                                "bigint" => Some(shape_value::v2::ConcreteType::BigInt),
                                "DateTime" => Some(shape_value::v2::ConcreteType::DateTime),
                                _ => None,
                            };
                            if let Some(ct) = ct {
                                self.program
                                    .value_call_return_concrete_types
                                    .insert((span, self.current_function), ct);

                                // cluster-2-cw-IB-class-b (closure-body
                                // typed-array param seed): retroactively
                                // populate `mir.local_typed_array_element_types`
                                // for the closure body's MIR slot
                                // corresponding to each typed-array
                                // caller-context arg. The MIR-side
                                // conduit's empty-typed-array-seed
                                // pass at `helpers.rs:623` consumes
                                // this map at
                                // `propagate_concrete_types_through_mir`
                                // time (which runs AFTER bytecode
                                // emission completes) to stamp
                                // `concrete_types[inner_slot] =
                                // Array(elem)` for the closure body.
                                // The JIT-MIR's `slot_kinds`
                                // projection then picks up
                                // `Ptr(TypedArray)` for `inner` and
                                // dispatches `.len()` /
                                // `.sum()` through the kinded fast
                                // path, returning raw scalar bits
                                // (Int64=15 for our fixture) instead
                                // of TAG_NULL.
                                //
                                // Without this, the closure body's
                                // JIT compilation has no type info
                                // for `inner` and the method
                                // dispatch returns TAG_NULL — the
                                // outer print would then read
                                // TAG_NULL bits and print garbage
                                // even with the destination kind
                                // correctly stamped Int64.
                                if let Some(closure_fn_idx) = peek.function_index {
                                    // Wrap in a block to allow early
                                    // exit via `break` for skip cases
                                    // (Arc shared / mir missing).
                                    'seed_block: {
                                        let Some(func) =
                                            self.program.functions.get_mut(closure_fn_idx)
                                        else {
                                            break 'seed_block;
                                        };
                                        let Some(mir_data_arc) = func.mir_data.as_mut() else {
                                            break 'seed_block;
                                        };
                                        // `Arc::get_mut` returns
                                        // `Some(&mut T)` only when
                                        // the strong-count is 1 —
                                        // the bytecode-emission
                                        // stage's invariant for
                                        // closure-body MIR Arcs (no
                                        // other clone exists yet
                                        // since content-addressed
                                        // program build runs later).
                                        // When this invariant is
                                        // broken (e.g. an upstream
                                        // change clones the Arc
                                        // before bytecode emission
                                        // completes), the propagation
                                        // is skipped; the side-table
                                        // stamping above still
                                        // applies, so the print
                                        // dispatch routes to
                                        // `jit_print_i64` — only
                                        // the closure body's typed-
                                        // array param seed is
                                        // missed.
                                        let Some(mir_data) = std::sync::Arc::get_mut(mir_data_arc)
                                        else {
                                            break 'seed_block;
                                        };
                                        // Match closure-body param
                                        // slots to caller-context arg
                                        // types. The MIR's
                                        // `param_slots` align 1:1
                                        // with the closure literal's
                                        // params list (no captures
                                        // interleaved for value-call
                                        // shape; the captures-as-
                                        // leading-args ABI is for the
                                        // trampoline closure-call
                                        // path, which doesn't fire
                                        // here per `vm_captures=false`
                                        // in the FAST PATH).
                                        for (param_idx, slot) in
                                            mir_data.mir.param_slots.clone().iter().enumerate()
                                        {
                                            // U4-5b: read the caller arg's array
                                            // element type STRUCTURALLY from its
                                            // recorded `ConcreteType::Array(elem)`.
                                            // The element ConcreteType IS the
                                            // proof (ADR-006 §2.7.5); no
                                            // `"Vec<elem>"` display-string strip /
                                            // re-parse. A non-array (or
                                            // unresolved) caller arg yields
                                            // nothing to seed (surface-and-stop
                                            // preserved). Only typed-array element
                                            // kinds (scalars with a typed-array
                                            // carrier) reach
                                            // `local_typed_array_element_types`;
                                            // `or_insert` skips a struct/object
                                            // element exactly as the kind-bounded
                                            // string match did.
                                            let Some(Some(shape_value::v2::ConcreteType::Array(
                                                elem,
                                            ))) = caller_arg_concrete_types.get(param_idx)
                                            else {
                                                continue;
                                            };
                                            let elem_ct = (**elem).clone();
                                            if crate::compiler::v2_typed_emission::should_use_typed_array(
                                                &elem_ct,
                                            )
                                            .is_some()
                                            {
                                                mir_data
                                                    .mir
                                                    .local_typed_array_element_types
                                                    .entry(*slot)
                                                    .or_insert(elem_ct);
                                            }
                                        }
                                    }
                                }

                                // U4-4: stamp `last_expr_type_info` from the
                                // tracked callable return-type name so a
                                // downstream `let`-binding / storage-hint sees
                                // it. The deleted `last_expr_numeric_type`
                                // register stamp is gone — a binop on the call
                                // result derives its numeric kind from the one
                                // resolved Type via `numeric_type_of`.
                                match rt_name.as_str() {
                                    "int" | "number" | "decimal" => {
                                        self.last_expr_type_info = Some(
                                            crate::type_tracking::VariableTypeInfo::named(
                                                rt_name.clone(),
                                            ),
                                        );
                                    }
                                    other if shape_runtime::type_system::BuiltinTypes::is_integer_type_name(other) => {
                                        self.last_expr_type_info =
                                            Some(crate::type_tracking::VariableTypeInfo::named(
                                                other.to_string(),
                                            ));
                                    }
                                    "string" | "bool" | "char" => {
                                        self.last_expr_type_info = Some(
                                            crate::type_tracking::VariableTypeInfo::named(
                                                rt_name.clone(),
                                            ),
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }

            return Ok(());
        }

        // Check for user-defined functions (after locals — function parameters take priority)
        if let Some(func_idx) = self.find_function(name) {
            let resolved_name = self.program.functions[func_idx].name.clone();

            // Check if this function was removed by a comptime annotation handler.
            if self.removed_functions.contains(&resolved_name)
                || self.removed_functions.contains(name)
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "function '{}' was removed by a comptime annotation handler and cannot be called",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            let is_comptime_fn = self
                .function_defs
                .get(&resolved_name)
                .or_else(|| self.function_defs.get(name))
                .map(|def| def.is_comptime)
                .unwrap_or(false);
            if is_comptime_fn && !self.comptime_mode {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "'{}' is declared as `comptime fn` and can only be called from comptime contexts",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            let mut call_name = resolved_name;
            let mut call_func_idx = func_idx;

            // BUG3 — free-function monomorphization wiring.
            //
            // When the callee is a generic function (`fn inner<T>(x: T) { ... }`)
            // and the call-site args resolve to concrete types, produce (or
            // reuse) a `inner::<concrete>` specialization and redirect the
            // call to it. Otherwise the call would land on the empty
            // template body (generic bodies are intentionally skipped in
            // `compile_function`) and run off the end of the bytecode,
            // blowing the VM call stack.
            //
            // The cycle detector in `ensure_monomorphic_function` prevents
            // transitive re-entry on the same `(fn_name, type_args)` pair
            // if a dispatch helper ever tries to resolve the specialization
            // from inside its own body. On a soft failure (unresolved type
            // args, cycle, benign compile error) we fall back to the
            // unspecialized callee — the caller already surfaces a clean
            // diagnostic when the body is empty.
            //
            // Phase 3a: a hard error (trait-bound violation) is propagated
            // up so the user sees a precise diagnostic instead of a
            // recursion / stack-overflow at runtime.
            if let Some(specialized_idx) =
                self.try_monomorphize_free_function_call(&call_name, args)?
            {
                call_func_idx = specialized_idx;
                call_name = self.program.functions[call_func_idx].name.clone();
            } else if self
                .function_defs
                .get(&call_name)
                .and_then(|d| d.type_params.as_ref())
                .is_some_and(|tps| tps.iter().any(|tp| !tp.is_const()))
            {
                // Soundness: the callee is a generic function and
                // monomorphization could not resolve a concrete specialization
                // from the call-site arguments. Generic function bodies are
                // intentionally skipped in `compile_function` (their AST is
                // kept only as a substitution template), so emitting a `Call`
                // onto this index would dispatch into a zero-instruction body
                // — the VM runs off the end and hangs. A type argument that
                // cannot be inferred is a compile error, not a silent
                // fall-through. A self-recursive generic call resolves to its
                // specialization's index above (`ensure_monomorphic_function`
                // caches before compiling the body), so it never reaches here.
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "cannot infer type argument(s) for generic function '{}' from the call-site arguments — annotate the arguments or call with values whose types are statically known",
                        call_name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            let total_arity = self.program.functions[call_func_idx].arity as usize;
            let (required_arity, effective_total_arity) = self
                .function_arity_bounds
                .get(&call_name)
                .copied()
                .unwrap_or((total_arity, total_arity));
            let actual_arity = args.len();
            if actual_arity < required_arity || actual_arity > effective_total_arity {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Function '{}' expects between {} and {} arguments, got {}",
                        name, required_arity, effective_total_arity, actual_arity
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            if let Some(const_param_indices) = self.function_const_params.get(&call_name).cloned() {
                for idx in const_param_indices {
                    if idx >= actual_arity {
                        continue;
                    }
                    let arg = &args[idx];
                    if !is_compile_time_const_expr(arg) {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Function '{}' parameter #{} is declared `const` and requires a compile-time constant argument",
                                name,
                                idx + 1
                            ),
                            location: Some(self.span_to_source_location(arg.span())),
                        });
                    }
                }

                if let Some((specialized_name, specialized_idx)) =
                    self.ensure_const_specialization(&call_name, args)?
                {
                    call_name = specialized_name;
                    call_func_idx = specialized_idx;
                }
            }

            let ref_params = self.program.functions[call_func_idx].ref_params.clone();
            let ref_mutates = self.program.functions[call_func_idx].ref_mutates.clone();
            let pass_modes = Self::pass_modes_from_ref_flags(&ref_params, &ref_mutates);
            let return_reference_summary =
                self.function_return_reference_summary_for_name(&call_name);

            // Sweep phase 3c.x: bidirectional inference for `any`-typed
            // callable params on free user functions. When the callee has
            // an `any`-annotated param at position k AND args[k] is a
            // closure literal AND the other concrete-typed args' types
            // determine the closure's param types, install
            // `pending_closure_param_types` so the closure compile path
            // attaches concrete annotations to its user params (`|x, y|`
            // → `|x: int, y: int|`). See
            // `apply2(|x, y| x + y, 2, 3)` — without this, `x + y` fails
            // strict typing as `unknown + unknown`.
            // Wave 1a PART B: usage-driven closure seeding for UNANNOTATED
            // callable params. When `fn apply2(f, x, y) { f(x, y) }` USES `f`
            // as a callable, whole-program inference resolved `f`'s type to a
            // concrete `fn(int,int)->_`; the call site `apply2(|a,b| a*b, …)`
            // seeds `|a,b|` from that inferred signature. This is the
            // higher-ranked extension of PART A. It supersedes the legacy
            // `any`-annotation heuristic below; the heuristic is only consulted
            // as a fallback when inference produced no concrete signature.
            let seeded_from_inference =
                self.install_pending_closure_param_types_for_inferred_fn_param(&call_name, args);
            if !seeded_from_inference {
                self.install_pending_closure_param_types_for_any_param_hof(&call_name, args);
            }

            // STAGE F4 (strict-flip, 2026-06-20): thread the callee's declared
            // param type-annotations so a bare empty-array argument (`[]`) whose
            // param is `Array<T>` constructs a valid typed empty `TypedArray<T>`
            // rather than SURFACEing `op_new_array(0)`. Looked up by the resolved
            // `call_name` first (covers monomorphized / const-specialized
            // callees), falling back to the surface `name`.
            let param_annotations: Option<Vec<Option<shape_ast::ast::TypeAnnotation>>> = self
                .function_defs
                .get(&call_name)
                .or_else(|| self.function_defs.get(name))
                .map(|def| {
                    def.params
                        .iter()
                        .map(|p| p.type_annotation.clone())
                        .collect()
                });

            // FIX B (strict-flip, THE GENERAL ROOT): an argument whose inferred
            // type is `unknown` / an unresolved free variable MUST NOT be
            // accepted into a parameter whose declared type is a PROVEN concrete
            // type (a primitive, a registered struct, or another concrete
            // nominal). This is the keystone's no-any-sink rule for binary-op
            // operands, extended to call arguments — it closes the LAUNDER
            // boundary that a pattern-bound `unknown` (or any other genuinely
            // un-inferable value) would otherwise pass through into a typed slot
            // and be bit-reinterpreted as that slot's NativeKind.
            //
            // No false positives after the T1 keystone: legitimate dispatch
            // results (`.map`/`.get`/match-arm binders/...) now resolve to
            // CONCRETE types via the post-solve expr-type table, so a VALID
            // program never passes `unknown` here. Only a genuinely un-inferable
            // value reaches this gate, and it SHOULD reject.
            //
            // A generic param (`fn f<T>(x: T)`) is NOT a proven concrete type —
            // its annotation names a type parameter, so it is excluded.
            self.reject_unknown_arg_into_typed_param(
                name,
                &call_name,
                args,
                param_annotations.as_deref(),
            )?;

            let writebacks = self.compile_call_args_with_param_types(
                args,
                Some(&pass_modes),
                param_annotations.as_deref(),
            )?;
            // The closure compile path takes() the hint, but if the closure
            // arg failed early (or there's no closure arg), clear any
            // residual hint to avoid leaking it into a later unrelated call.
            self.pending_closure_param_types = None;

            // Compile default expressions for missing arguments
            if actual_arity < effective_total_arity {
                let func_def = self
                    .function_defs
                    .get(&call_name)
                    .or_else(|| self.function_defs.get(name))
                    .cloned();
                for param_idx in actual_arity..effective_total_arity {
                    let mut emitted_default = false;
                    if let Some(ref fdef) = func_def {
                        if let Some(param) = fdef.params.get(param_idx) {
                            if let Some(ref default_expr) = param.default_value {
                                let is_ref_param =
                                    ref_params.get(param_idx).copied().unwrap_or(false);
                                if is_ref_param {
                                    let borrow_mode =
                                        if ref_mutates.get(param_idx).copied().unwrap_or(false) {
                                            crate::compiler::BorrowMode::Exclusive
                                        } else {
                                            crate::compiler::BorrowMode::Shared
                                        };
                                    self.compile_implicit_reference_arg(default_expr, borrow_mode)?;
                                }
                                if !is_ref_param {
                                    self.compile_expr(default_expr)?;
                                }
                                emitted_default = true;
                            }
                        }
                    }
                    if !emitted_default {
                        self.emit_unit();
                    }
                }
            }
            let arg_count = self
                .program
                .add_constant(Constant::Int(effective_total_arity as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(
                    call_func_idx as u16,
                ))),
            ));
            // Record callee as a blob dependency
            if let Some(ref mut blob) = self.current_blob_builder {
                blob.record_call(&call_name);
            }
            if !writebacks.is_empty() {
                let result_local = self.declare_temp_local("__call_result_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));
                for (shadow_local, binding_idx) in writebacks {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(shadow_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
            }

            let return_type_annotation = self
                .function_defs
                .get(&call_name)
                .and_then(|def| def.return_type.clone())
                .or_else(|| {
                    self.foreign_function_defs
                        .get(&call_name)
                        .and_then(|def| def.return_type.clone())
                });
            // Module-qualified callees (`m::mk` returns `P`) carry their
            // return-type annotation in bare form (`P`) even though the
            // schema is registered as `m::P`. `type_info_from_annotation`
            // looks up the bare name first; on miss, retry with the call
            // name's namespace prefix so the schema lookup succeeds and
            // downstream property access (`m::mk().x`) resolves the
            // typed-field tag at the GetProp emit site (task #108
            // companion to commit 0f15571's executor flip).
            let direct = return_type_annotation
                .as_ref()
                .and_then(|ann| self.type_info_from_annotation(ann));
            self.last_expr_type_info = direct
                .or_else(|| {
                    let ann = return_type_annotation.as_ref()?;
                    let namespace = call_name.rsplit_once("::").map(|(ns, _)| ns)?;
                    let qualified = qualify_type_annotation_with_namespace(ann, namespace)?;
                    self.type_info_from_annotation(&qualified)
                })
                // WS-9c: an unannotated function whose inferred return type
                // is an anonymous object (an object-literal factory) carries
                // no `return_type_annotation`. Register an inline anonymous
                // schema for the projected return fields so the call result
                // — and a `let` bound to it — resolves `.field` access.
                .or_else(|| self.inline_schema_for_inferred_return(&call_name));
            self.last_expr_schema = self
                .last_expr_type_info
                .as_ref()
                .and_then(Self::value_schema_from_type_info);

            // Propagate return type for typed opcode emission
            if let Some(return_reference_summary) = return_reference_summary {
                self.set_last_expr_reference_result(return_reference_summary.mode, true);
            } else if let Some(borrow_mode) = self.function_declares_borrow_return(&call_name) {
                // ADR-006 §2.7.30 (GapA): `-> &T` callee returns a reference value;
                // value position reads THROUGH it (no param-reborrow summary).
                self.set_last_expr_reference_result(borrow_mode, true);
                // JIT has no §2.7.30 PromotedCell lowering — deopt to interpreter.
                self.program.has_reference_escape_promotion = true;
            } else {
                self.clear_last_expr_reference_result();
            }
            return Ok(());
        }

        if let Some(builtin_decl) = self.resolve_scoped_module_builtin_function(name)
            && !(self.allow_internal_builtins
                && Self::is_internal_intrinsic_name(&builtin_decl.export_name))
        {
            return self.compile_module_builtin_function_call(&builtin_decl, args, span);
        }

        // Builtins take precedence - they're optimized Rust implementations.
        // Phase 1 keeps the current surface behavior, but distinguishes
        // surface names from internal-only intrinsics for diagnostics.
        if let Some(resolution) = self.classify_builtin_function(name) {
            let builtin = match resolution {
                BuiltinNameResolution::Surface { builtin, .. } => builtin,
                BuiltinNameResolution::InternalOnly { builtin, .. }
                    if self.allow_internal_builtins =>
                {
                    builtin
                }
                BuiltinNameResolution::InternalOnly { .. } => {
                    return Err(ShapeError::SemanticError {
                        message: self.internal_intrinsic_error_message(name, resolution),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
            };

            // Special handling for print with string interpolation
            if builtin == BuiltinFunction::Print {
                return self.compile_print_with_interpolation(args);
            }

            // U3 (SB-9 deletion): the dual-HashMap-carrier split-brain is gone.
            // ALL HashMap construction routes through `BuiltinCall(HashMapCtor)`
            // → the single honest `HashMapData` carrier (HeapKind::HashMap,
            // refcounted via the §2.7.7 kind track, element-releasing Drop,
            // `NativeKind::Null` None-arm, snapshot arm). The compile-time
            // `should_use_typed_map` switch + the `TypedMap<K,V>` carrier that
            // stamped `NativeKind::UInt64` (the carrier-kind lie SB-10/11/12/13)
            // were deleted: adding/removing a type annotation on a HashMap must
            // not change which runtime structure, None-encoding, or refcount
            // discipline is used.

            for arg in args {
                self.compile_expr_as_value_or_placeholder(arg)?;
            }
            if self.builtin_requires_arg_count(builtin) {
                let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));
            }
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(builtin)),
            ));
            // Propagate known return type for builtin functions
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.clear_last_expr_reference_result();
            // ADR-006 §2.7.27 / Item 4 ruling: signal a recognised COW
            // container kind so the surrounding let-binding code path
            // can transfer it to the receiver-local's
            // `mut_self_container_locals` entry. The signal is consumed
            // at `statements.rs` let-binding completion (mirror of
            // `pending_variable_typed_array_kind`).
            if let Some(kind) =
                crate::compiler::mutation_writeback::ContainerKind::from_ctor_name(name)
            {
                self.pending_variable_container_kind = Some(kind);
            }
            return Ok(());
        }

        // Removed global data-loading API:
        // load("provider", { ... }) -> provider.load({ ... }) (module-scoped).
        if name == "load"
            && args.len() == 2
            && matches!(args[0], Expr::Literal(Literal::String(_), _))
        {
            return Err(ShapeError::SemanticError {
                message:
                    "load(provider, params) has been removed. Use module-scoped calls like `provider.load({ ... })`."
                        .to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        }

        // Named import from a native extension module (e.g. `from std::core::file use { read_text }`).
        // Native modules have no AST to inline, so the function won't be in program.functions.
        // Keep a private module binding so the imported symbol can dispatch without
        // implicitly creating a user-visible namespace.
        if let Some(imported) = self.imported_names.get(name).cloned() {
            if self.is_native_module_export(&imported.module_path, &imported.original_name) {
                let binding_name = self.ensure_hidden_native_module_binding(&imported.module_path);
                return self.compile_module_namespace_call_on_binding(
                    &binding_name,
                    &imported.module_path,
                    span,
                    &imported.original_name,
                    args,
                );
            }
        }

        // Build error message with suggestions
        let mut message = self.undefined_function_message(name);

        // Try import suggestion first
        if let Some(module_path) = self.suggest_import(name) {
            message = format!(
                "Unknown function '{}'. Did you mean to import it via '{}'\n\n  from {} use {{ {} }}\n\n{}",
                name,
                module_path,
                module_path,
                name,
                Self::function_scope_summary(),
            );
        } else {
            // Try typo suggestion from available function names
            let available = self.collect_available_function_names();
            if let Some(suggestion) = suggest_function(name, &available) {
                message.push_str(&format!(". {}", suggestion));
            }
        }
        Err(ShapeError::RuntimeError {
            message,
            location: Some(self.span_to_source_location(span)),
        })
    }

    /// FIX B (strict-flip, THE GENERAL ROOT — close the launder boundary):
    /// reject an argument whose inferred type is `unknown` / an unresolved free
    /// variable when the matching parameter's declared type is a PROVEN concrete
    /// type (a primitive, a registered struct, or a registered concrete enum).
    ///
    /// This mirrors the keystone's no-any-sink rule for binary-op operands,
    /// extended to call arguments. Without it a pattern-bound `unknown` (or any
    /// other genuinely un-inferable value) launders through the typed fn-arg
    /// boundary into a concrete slot and is bit-reinterpreted as that slot's
    /// NativeKind — the catastrophic cross-type reinterpret
    /// (`sink(n)` with `n: unknown` → `int` param → raw-ptr arithmetic).
    ///
    /// NO FALSE POSITIVES after the T1 keystone: legitimate dispatch results
    /// (`.map`/`.get`/match-arm binders) resolve to CONCRETE types via the
    /// post-solve expr-type table, so a valid program never passes `unknown`
    /// here. A generic param (`fn f<T>(x: T)`) is excluded — its annotation
    /// names a type parameter, not a proven concrete type.
    fn reject_unknown_arg_into_typed_param(
        &mut self,
        name: &str,
        call_name: &str,
        args: &[Expr],
        param_annotations: Option<&[Option<shape_ast::ast::TypeAnnotation>]>,
    ) -> Result<()> {
        let Some(param_annotations) = param_annotations else {
            return Ok(());
        };

        // The callee's own generic type-parameter names. A param annotated with
        // one of these is NOT a proven concrete type (it is monomorphized from
        // the call-site arg), so it must NOT trigger the reject.
        let type_param_names: std::collections::HashSet<String> = self
            .function_defs
            .get(call_name)
            .or_else(|| self.function_defs.get(name))
            .and_then(|def| def.type_params.as_ref())
            .map(|tps| tps.iter().map(|tp| tp.name().to_string()).collect())
            .unwrap_or_default();

        for (idx, arg) in args.iter().enumerate() {
            let Some(Some(ann)) = param_annotations.get(idx) else {
                continue;
            };
            if !self.param_annotation_is_proven_concrete(ann, &type_param_names) {
                continue;
            }
            // The param slot is a proven concrete type. Reject only when the
            // argument's type is genuinely un-inferable.
            let Ok(arg_ty) = self.infer_expr_type(arg) else {
                continue;
            };
            if Self::type_is_unknown(&arg_ty) {
                let param_disp = Self::annotation_display_name(ann);
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "argument #{} to '{}' has an un-inferable type (`unknown`), \
                         but the parameter has the proven concrete type '{}' — an \
                         `unknown`-typed value cannot be passed into a typed parameter \
                         (this would reinterpret its raw bits as '{}'). Annotate the \
                         value or destructure it through a type-checked pattern.",
                        idx + 1,
                        name,
                        param_disp,
                        param_disp
                    ),
                    location: Some(self.span_to_source_location(arg.span())),
                });
            }
        }
        Ok(())
    }

    /// True when a parameter's declared annotation resolves to a PROVEN concrete
    /// type — a known primitive, a registered struct (type alias to an object
    /// shape), or a registered enum — and is NOT one of the callee's generic
    /// type-parameter names. Generic, structural, and unresolved annotations are
    /// NOT proven concrete (conservative: only positive proof triggers FIX B).
    fn param_annotation_is_proven_concrete(
        &self,
        ann: &shape_ast::ast::TypeAnnotation,
        type_param_names: &std::collections::HashSet<String>,
    ) -> bool {
        use shape_ast::ast::TypeAnnotation;
        let name = match ann {
            TypeAnnotation::Basic(n) => n.as_str(),
            TypeAnnotation::Reference(p) => p.as_str(),
            _ => return false,
        };
        // A generic type-parameter name is monomorphized from the arg — never a
        // proven concrete slot.
        if type_param_names.contains(name) {
            return false;
        }
        if Self::is_known_concrete_primitive_name(name) {
            return true;
        }
        // A registered struct (`type Point { ... }` → type alias to Object) or a
        // registered enum is a proven concrete nominal slot.
        self.type_inference.env.lookup_type_alias(name).is_some()
            || self.type_inference.env.get_enum(name).is_some()
    }

    /// Known primitive (non-generic) type names. Mirrors the type-system side
    /// `is_known_primitive_name`; kept compiler-local to avoid a cross-crate
    /// pub surface just for this gate.
    pub(crate) fn is_known_concrete_primitive_name(name: &str) -> bool {
        matches!(
            name,
            "int"
                | "number"
                | "bool"
                | "string"
                | "decimal"
                | "bigint"
                | "char"
                | "byte"
                | "DateTime"
                | "Duration"
        )
    }

    /// True when an inferred argument type is genuinely un-inferable: an
    /// unresolved free type variable, a still-bounded constrained variable, or
    /// the `"unknown"` placeholder a lost type var renders to.
    pub(crate) fn type_is_unknown(ty: &shape_runtime::type_system::Type) -> bool {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;
        match ty {
            Type::Variable(_) | Type::Constrained { .. } => true,
            Type::Concrete(TypeAnnotation::Basic(n)) => n == "unknown",
            Type::Concrete(TypeAnnotation::Reference(p)) => p.as_str() == "unknown",
            _ => false,
        }
    }

    /// Display name for a param annotation in a FIX-B diagnostic.
    fn annotation_display_name(ann: &shape_ast::ast::TypeAnnotation) -> String {
        use shape_ast::ast::TypeAnnotation;
        match ann {
            TypeAnnotation::Basic(n) => n.clone(),
            TypeAnnotation::Reference(p) => p.to_string(),
            other => format!("{:?}", other),
        }
    }

    /// Check if a method name accepts a closure argument with a receiver-typed row parameter.
    ///
    /// Queries the MethodTable for Table and DataTable first; falls back to
    /// the hardcoded heuristic for user-defined types or methods not yet in the table.
    fn is_datatable_closure_method(&self, method: &str) -> bool {
        if self
            .method_table
            .takes_closure_with_receiver_param("Table", method)
            || self
                .method_table
                .takes_closure_with_receiver_param("DataTable", method)
        {
            return true;
        }
        // Fallback: hardcoded heuristic for methods not registered in the MethodTable
        // (e.g., user-defined types, aliases like group_by/index_by)
        Self::is_datatable_closure_method_heuristic(method)
    }

    /// Hardcoded fallback for closure-method detection.
    fn is_datatable_closure_method_heuristic(method: &str) -> bool {
        matches!(
            method,
            "filter"
                | "forEach"
                | "map"
                | "find"
                | "some"
                | "every"
                | "groupBy"
                | "group_by"
                | "orderBy"
                | "index_by"
                | "indexBy"
                | "sum"
                | "mean"
                | "min"
                | "max"
                | "simulate"
        )
    }

    /// Check if a method preserves the Table<T> type (output is same Table<T> as input).
    ///
    /// Queries the MethodTable for Table, DataTable, and Array first; falls back to
    /// the hardcoded heuristic for user-defined types or methods not yet in the table.
    fn is_type_preserving_table_method(&self, method: &str) -> bool {
        if self.method_table.is_self_returning("Table", method)
            || self.method_table.is_self_returning("DataTable", method)
        {
            return true;
        }
        // Fallback: hardcoded heuristic for methods not registered in the MethodTable
        // (e.g., user-defined types, aliases like "where", "slice", "reverse", "concat")
        Self::is_type_preserving_table_method_heuristic(method)
    }

    /// Hardcoded fallback for type-preserving method detection.
    fn is_type_preserving_table_method_heuristic(method: &str) -> bool {
        matches!(
            method,
            "filter"
                | "where"
                | "head"
                | "tail"
                | "slice"
                | "reverse"
                | "concat"
                | "orderBy"
                | "sort"
        )
    }

    pub(super) fn is_module_namespace_name(&self, name: &str) -> bool {
        (name == "__comptime__" && self.allow_internal_comptime_namespace)
            || self.module_namespace_bindings.contains(name)
    }

    fn compile_type_namespace_builtin_call(
        &mut self,
        namespace: &str,
        function: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<bool> {
        let builtin = match (namespace, function) {
            ("DateTime", "now") => Some(BuiltinFunction::DateTimeNow),
            ("DateTime", "utc") => Some(BuiltinFunction::DateTimeUtc),
            ("DateTime", "parse") => Some(BuiltinFunction::DateTimeParse),
            ("DateTime", "from_epoch") => Some(BuiltinFunction::DateTimeFromEpoch),
            ("DateTime", "from_parts") => Some(BuiltinFunction::DateTimeFromParts),
            ("DateTime", "from_unix_secs") => Some(BuiltinFunction::DateTimeFromUnixSecs),
            ("Content", "chart") => Some(BuiltinFunction::ContentChart),
            ("Content", "text") => Some(BuiltinFunction::ContentTextCtor),
            ("Content", "table") => Some(BuiltinFunction::ContentTableCtor),
            ("Content", "code") => Some(BuiltinFunction::ContentCodeCtor),
            ("Content", "kv") => Some(BuiltinFunction::ContentKvCtor),
            ("Content", "fragment") => Some(BuiltinFunction::ContentFragmentCtor),
            // SC1 (R8 — supervisor): `Color.rgb(r, g, b)` is the only
            // call-form style-spec constructor. It returns a string carrier
            // (`rgb(r,g,b)`). Named members `Color.red` / `Border.rounded`
            // / `ChartType.line` are compile-time-constant strings emitted
            // directly by the property-access path, not routed here.
            ("Color", "rgb") => Some(BuiltinFunction::ColorRgbCtor),
            // W18.5 per-type builder constructors (supervisor D4,
            // R8 W3 2026-05-24): `Table::new()` / `Code::new()` /
            // `KeyValue::new()` → empty `ContentNode` of the matching
            // variant. Chained `.headers(...)` / `.row(...)` / `.border(...)`
            // / `.language(...)` / `.source(...)` / `.pair(...)` / `.build()`
            // live in `CONTENT_METHODS` PHF as method-call dispatch on the
            // Content receiver. Both `Foo::new()` (parsed as
            // QualifiedFunctionCall) and `Foo.new()` (parsed as MethodCall
            // on Identifier("Foo")) route here through
            // `compile_expr_qualified_function_call` /
            // `compile_expr_method_call` → `compile_type_namespace_builtin_call`.
            ("Table", "new") => Some(BuiltinFunction::TableBuilderNew),
            ("Code", "new") => Some(BuiltinFunction::CodeBuilderNew),
            ("KeyValue", "new") => Some(BuiltinFunction::KeyValueBuilderNew),
            _ => None,
        };

        let Some(builtin) = builtin else {
            return Ok(false);
        };

        for arg in args {
            self.compile_expr_as_value_or_placeholder(arg)?;
        }
        let count = self.program.add_constant(Constant::Int(args.len() as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(count)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(builtin)),
        ));
        self.last_expr_schema = None;
        // SC1: `Color.rgb(...)` returns a `string` carrier; the remaining
        // namespace ctors produce Content / DateTime values whose type the
        // downstream dispatch infers structurally (left as `None`).
        self.last_expr_type_info = if matches!(builtin, BuiltinFunction::ColorRgbCtor) {
            Some(crate::type_tracking::VariableTypeInfo::named(
                "string".to_string(),
            ))
        } else {
            None
        };
        self.clear_last_expr_reference_result();
        let _ = span;
        Ok(true)
    }

    pub(super) fn compile_expr_qualified_function_call(
        &mut self,
        namespace: &str,
        function: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        let scoped_name = format!("{}::{}", namespace, function);
        if let Some(builtin_decl) = self.module_builtin_functions.get(&scoped_name).cloned() {
            return self.compile_module_builtin_function_call(&builtin_decl, args, span);
        }
        if self.find_function(&scoped_name).is_some() {
            return self.compile_expr_function_call(&scoped_name, args, span);
        }

        if self.is_module_namespace_name(namespace) {
            return self.compile_module_namespace_call(namespace, span, function, args);
        }

        if self.compile_type_namespace_builtin_call(namespace, function, args, span)? {
            return Ok(());
        }

        if let Some(schema) = self.type_tracker.schema_registry().get(namespace)
            && let Some(enum_info) = schema.get_enum_info()
            && enum_info.variant_by_name(function).is_some()
        {
            return self.compile_expr_enum_constructor(
                namespace,
                function,
                &shape_ast::ast::EnumConstructorPayload::Tuple(args.to_vec()),
            );
        }

        Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown qualified call '{}::{}'. Module namespace calls require an explicit `use`, and type-associated calls require the type to define that item.",
                namespace, function
            ),
            location: Some(self.span_to_source_location(span)),
        })
    }

    /// Strict-typing-sweep (Cluster 3): for HOF method calls on arrays
    /// (`.map` / `.filter` / `.reduce` / `.forEach` / `.find` / `.findIndex`
    /// / `.some` / `.every` / `.flatMap`), populate
    /// `pending_closure_param_types` so the closure compile path attaches a
    /// concrete annotation to the user param (e.g. `|x|` → `|x: int|`)
    /// which the type-tracker installs and the binary-op compile path then
    /// trusts.
    ///
    /// The receiver was already compiled by the caller, so whole-binding
    /// `ConcreteType::Array(elem)` records are populated; an inline array
    /// literal receiver resolves its element type structurally via
    /// `concrete_type_for_expr` (U4-6a: the per-span `array_element_types`
    /// cache is deleted).
    ///
    /// Argument-order validation: every HOF wired here takes its callback
    /// as positional argument 0 — `map(f)` / `filter(predicate)` /
    /// `reduce(f, init)` / etc. (see `crates/shape-runtime/stdlib-src/core/
    /// vec.shape`). If argument 0 is a *provably* non-callable expression
    /// (a literal, an array literal, or an object literal — none of which
    /// can ever denote a callable), the call is ill-typed. Without this
    /// guard, the wrong-order call `[1,2,3].reduce(0, |acc,x| acc+x)`
    /// (init first, JS/conventional order — but Shape's `reduce` is
    /// `(f, init)`) bound the int `0` to the generic callable param `f`
    /// and degenerated into a re-entrant `main` miscompile (infinite loop)
    /// instead of a clean type error. We surface a compile-time
    /// `SemanticError` here, the earliest point that has both the method
    /// name and the literal arg kinds in hand.
    pub(crate) fn install_pending_closure_param_types_for_hof(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
    ) -> Result<()> {
        // Only the simple "single closure with one user-param-of-element-type"
        // HOFs are wired here. Reduce takes (acc, x) — both are element-type
        // for homogeneous folds, so we hint both. Sort takes a comparator
        // `(T, T) => int` — both params are element-type, like reduce's
        // homogeneous fold but with the array element type for both
        // positions (D-α.1 close, 2026-05-22, per
        // `v0.3-d-alpha-audit.md` §4 trigger KC #6(f)).
        let is_single_arg_hof = matches!(
            method,
            "map" | "filter" | "forEach" | "find" | "findIndex" | "some" | "every" | "flatMap"
        );
        let is_reduce = method == "reduce";
        let is_sort = method == "sort";
        if !is_single_arg_hof && !is_reduce && !is_sort {
            return Ok(());
        }
        // Need at least one closure arg.
        if args.is_empty() {
            return Ok(());
        }

        // Argument-order / argument-kind validation. The callback is
        // positional argument 0 for every HOF wired here. A literal,
        // array literal, or object literal at that position can never be
        // a callable — reject it with a clean compile error rather than
        // letting an int bind a generic callable param and miscompile.
        // (Identifiers / function references / property accesses are NOT
        // rejected: they may legitimately resolve to a callable.)
        if let Some(non_callable_kind) = Self::provably_non_callable_kind(&args[0]) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "`{method}` expects a closure (function) as its first argument, \
                     got {non_callable_kind}. Shape's `{method}` takes the callback \
                     first{}.",
                    if is_reduce {
                        " — the signature is `reduce(f, init)`, not `reduce(init, f)`"
                    } else {
                        ""
                    }
                ),
                location: Some(self.span_to_source_location(args[0].span())),
            });
        }

        let elem_ann_opt: Option<shape_ast::ast::TypeAnnotation> =
            match crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                self, receiver,
            ) {
                Some(shape_value::v2::concrete_type::ConcreteType::Array(inner)) => {
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(&inner)
                }
                _ => None,
            }
            // Fallback: if the receiver is an inline array literal, infer
            // element type from the elements via the existing inference helper.
            // (U4-6a: `concrete_type_for_expr` now resolves array literals
            // structurally from their elements; this `infer_array_element_type`
            // fallback remains for the kinds it does not yet cover.)
            .or_else(|| {
                if let Expr::Array(elements, _) = receiver {
                    let kind = crate::compiler::v2_array_emission::infer_array_element_type(
                        elements,
                        &self.type_tracker,
                    )?;
                    slot_kind_to_type_annotation(kind)
                } else {
                    None
                }
            })
            // Wave 1b iterator-HOF (2026-06-15): when the receiver is an
            // element-type-PRESERVING iterator adapter chain
            // (`[1,2,3].iter()`, `arr.iter().filter(..).take(n)`), the
            // receiver `ConcreteType` is not an `Array<T>` (Iterator has no
            // `ConcreteType` variant), so the array paths above yield `None`.
            // `iter_element_type_name` recovers the element-type NAME from the
            // adapter chain (recursing through `iter`/`filter`/`take`/`skip`,
            // the type-preserving adapters). Map that name to a
            // `TypeAnnotation`, but ONLY when it resolves to a known concrete
            // type — `declared_annotation_concrete_type` is the proof gate, so
            // an un-resolvable element name SURFACEs (the closure param stays
            // unannotated, exactly as for an array receiver whose element type
            // can't be proven). int and number stay distinct: the name carries
            // the exact proven element type, never a numeric default.
            .or_else(|| {
                let elem_name = self.iter_element_type_name(receiver)?;
                let ann = shape_ast::ast::TypeAnnotation::Basic(elem_name);
                crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                    self, &ann,
                )
                .map(|_| ann)
            })
            // R3-subcase struct-array HOF (strict-flip, 2026-06-15): the
            // `.iter()` adapter chain over a struct array
            // (`users.iter().filter(|u| u.score > 85)`,
            // `users.iter().find(|u| ...)`) — the name-based fallback above
            // yields the lossy `Vec<object>` head-name which
            // `declared_annotation_concrete_type` rejects, so the struct
            // identity was lost and `u` stayed unannotated. `iter_element_concrete_type`
            // recovers the element `ConcreteType::Struct(named)` (recursing the
            // type-preserving `iter`/`filter`/`take`/`skip` adapters), and
            // `concrete_type_to_type_annotation` renders it as `Reference(name)`
            // so the closure param carries the struct type and `u.score`
            // resolves. The name IS the proof (per ADR-006 §2.7.5); an unnamed /
            // non-struct element yields `None` and the param stays unannotated.
            .or_else(|| {
                let elem_ct = self.iter_element_concrete_type(receiver)?;
                crate::compiler::expressions::closures::concrete_type_to_type_annotation(&elem_ct)
            });
        let Some(elem_ann) = elem_ann_opt else {
            return Ok(());
        };

        let hints = if is_reduce {
            // reduce(f, init): the callback `f` is positional arg 0 with
            // two user params `(acc, x)`, both elem-type for homogeneous
            // folds; `init` is positional arg 1.
            vec![Some(elem_ann.clone()), Some(elem_ann)]
        } else if is_sort {
            // sort(cmp): the callback `cmp` is positional arg 0 with two
            // user params `(a, b)`, both elem-type for a homogeneous
            // comparator. The return type (int) is not propagated as a
            // hint — closure body inference recovers it from the literal
            // arithmetic ops on the int-typed params. (D-α.1 close —
            // closes KC #6(f) test_array_sort_ascending /
            // test_array_sort_descending; see audit §4 sort row.)
            vec![Some(elem_ann.clone()), Some(elem_ann)]
        } else {
            vec![Some(elem_ann)]
        };
        self.pending_closure_param_types = Some(hints);
        Ok(())
    }

    /// Classify an argument expression that is *provably* not a callable.
    /// Returns a human-readable kind name (for diagnostics) when the
    /// expression can never denote a closure/function, or `None` when it
    /// might (identifiers, function references, calls, property accesses,
    /// conditionals, etc. — anything that could resolve to a callable).
    ///
    /// Only the unambiguous literal forms are rejected: this is a
    /// conservative guard that never false-positives on a legitimate
    /// callable argument such as a named function passed to `.map`.
    fn provably_non_callable_kind(arg: &Expr) -> Option<&'static str> {
        match arg {
            Expr::Literal(lit, _) => Some(match lit {
                Literal::Int(_) | Literal::UInt(_) | Literal::TypedInt(_, _) => "an int",
                Literal::Number(_) => "a number",
                Literal::Decimal(_) => "a decimal",
                Literal::String(_) | Literal::FormattedString { .. } => "a string",
                // A char literal IS an int code point (operators.mdx).
                Literal::Char(_) => "an int",
                Literal::Bool(_) => "a bool",
                // `None`, `Unit`, `Timeframe` — non-callable values.
                _ => "a literal value",
            }),
            Expr::Array(_, _) => Some("an array"),
            Expr::Object(_, _) => Some("an object"),
            _ => None,
        }
    }

    /// Wave 1a PART B: usage-driven closure-argument seeding from an
    /// inference-resolved FUNCTION-TYPED parameter.
    ///
    /// When a free user function has an UNANNOTATED param that its body USES as
    /// a callable (`fn apply2(f, x, y) { f(x, y) }`, `fn map_pair(f, a, b) {
    /// [f(a), f(b)] }`), whole-program inference resolves that param to a
    /// concrete `Type::Function`; `inferred_param_fn_param_types_from_facts`
    /// derives its argument annotations on demand from `inference_facts` plus
    /// the callee body-use shape.
    /// Here, at the call site, if the matching argument is a CLOSURE LITERAL
    /// whose user-param count equals the inferred signature arity, we map the
    /// inferred argument annotations 1:1 onto the closure's params via
    /// `pending_closure_param_types`. The closure compile path then attaches
    /// concrete annotations (`|a, b|` → `|a: int, b: int|`), so a body like
    /// `a * b` type-checks under strict typing instead of failing
    /// `unknown * unknown`.
    ///
    /// Returns `true` iff a hint was installed.
    ///
    /// Soundness (strict-typing core):
    /// * Fires ONLY for params the engine resolved to a fully-concrete fn-type
    ///   (the `None`-bailing producer guarantees no `unknown` leaks in). An
    ///   un-inferable / dead callable param has no entry ⇒ no seeding ⇒ the
    ///   closure keeps its existing rejection. No `any`, no Bool-default, no
    ///   silent pick.
    /// * Requires arity to match EXACTLY (signature arity == closure user-param
    ///   count). A mismatch ⇒ no seeding (the call is independently an
    ///   arity error elsewhere).
    /// * Each closure param is seeded from its OWN signature position, so
    ///   heterogeneous signatures (`fn(int, string)`) map correctly —
    ///   `int`/`number`/`string` stay distinct. A later body conflict with the
    ///   seeded type still surfaces as a strict error.
    pub(crate) fn install_pending_closure_param_types_for_inferred_fn_param(
        &mut self,
        callee_name: &str,
        args: &[Expr],
    ) -> bool {
        let Some(func_def) = self.function_defs.get(callee_name).cloned() else {
            return false;
        };
        // Find the argument positions that are closure literals AND for which
        // the callee has an inferred concrete fn-type. We only seed when there
        // is exactly one such position (a single pending hint slot), matching
        // the closure compile path's single-`take()` consumption.
        let mut seedable: Option<(usize, Vec<shape_ast::ast::TypeAnnotation>)> = None;
        for (idx, arg) in args.iter().enumerate() {
            if !matches!(arg, Expr::FunctionExpr { .. }) {
                continue;
            }
            let Some(sig_anns) =
                self.inferred_param_fn_param_types_from_facts(callee_name, &func_def, idx)
            else {
                continue;
            };
            if seedable.is_some() {
                // More than one seedable closure arg — the single pending-hint
                // slot cannot serve both. Bail rather than mis-seed.
                return false;
            }
            seedable = Some((idx, sig_anns));
        }
        let Some((closure_pos, sig_anns)) = seedable else {
            return false;
        };

        // Match the closure's user-param count to the inferred arity exactly.
        let Expr::FunctionExpr { params, .. } = &args[closure_pos] else {
            return false;
        };
        if params.len() != sig_anns.len() || sig_anns.is_empty() {
            return false;
        }

        let hints: Vec<Option<shape_ast::ast::TypeAnnotation>> =
            sig_anns.iter().cloned().map(Some).collect();
        self.pending_closure_param_types = Some(hints);
        true
    }

    /// Sweep phase 3c.x: bidirectional inference for free user functions
    /// whose callable param is typed `any`. When the call site supplies a
    /// closure literal at the same position, infer the closure's param
    /// types from the OTHER concrete-typed args at the call site.
    ///
    /// Concretely: `apply2(f: any, a: int, b: int) -> int` called as
    /// `apply2(|x, y| x + y, 2, 3)` should map to `|x: int, y: int|`.
    /// We scan args once, find the (single) closure arg position, and use
    /// the remaining args' inferred types to fill closure-param hints.
    /// We require the remaining args' types to be homogeneous and to
    /// match the closure's user-param count exactly.
    pub(crate) fn install_pending_closure_param_types_for_any_param_hof(
        &mut self,
        callee_name: &str,
        args: &[Expr],
    ) {
        // Locate the (single) closure-literal arg.
        let closure_idx = args
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match a {
                Expr::FunctionExpr { .. } => Some(i),
                _ => None,
            })
            .collect::<Vec<_>>();
        if closure_idx.len() != 1 {
            return;
        }
        let closure_pos = closure_idx[0];

        // Look up the closure's user-param count.
        let closure_user_param_count = if let Expr::FunctionExpr { params, .. } = &args[closure_pos]
        {
            params.len()
        } else {
            return;
        };
        if closure_user_param_count == 0 {
            return;
        }

        // The callee must be a known user function whose param at
        // `closure_pos` is annotated `any` (callable-by-erased-type).
        let func_def = match self.function_defs.get(callee_name).cloned() {
            Some(def) => def,
            None => return,
        };
        let callee_param_at_closure_pos = match func_def.params.get(closure_pos) {
            Some(p) => p,
            None => return,
        };
        let is_any_annotated = matches!(
            &callee_param_at_closure_pos.type_annotation,
            Some(shape_ast::ast::TypeAnnotation::Basic(name)) if name == "any"
        );
        if !is_any_annotated {
            return;
        }

        // Collect inferred types for the remaining (non-closure) args.
        let mut remaining_types: Vec<shape_ast::ast::TypeAnnotation> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if i == closure_pos {
                continue;
            }
            let ty = match self.infer_expr_type(arg) {
                Ok(t) => t,
                Err(_) => return, // Unknown type — bail.
            };
            // Require a Concrete(Basic(...)) primitive name.
            let ann = match ty {
                shape_runtime::type_system::Type::Concrete(ann) => ann,
                _ => return,
            };
            remaining_types.push(ann);
        }

        // Require exactly `closure_user_param_count` remaining args (so
        // they zip 1:1 with the closure's user params).
        if remaining_types.len() != closure_user_param_count {
            return;
        }
        // Require all remaining types to be the same primitive scalar name
        // — homogeneous arithmetic is the only safe pattern for a closure
        // body like `x + y`. Heterogeneous args would need stronger
        // analysis to map to specific param positions.
        let first = match remaining_types.first() {
            Some(shape_ast::ast::TypeAnnotation::Basic(n)) => n.clone(),
            _ => return,
        };
        for ann in &remaining_types[1..] {
            match ann {
                shape_ast::ast::TypeAnnotation::Basic(n) if *n == first => {}
                _ => return,
            }
        }
        if !BytecodeCompiler::tracker_type_name_is_primitive(&first) {
            return;
        }

        let elem_ann = shape_ast::ast::TypeAnnotation::Basic(first);
        let hints = vec![Some(elem_ann); closure_user_param_count];
        self.pending_closure_param_types = Some(hints);
    }

    /// ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback,
    /// 2026-05-12): determine whether the method call needs a
    /// post-`CallMethod` write-back to the receiver's binding slot.
    ///
    /// Returns `Some(target)` when ALL of:
    /// - `receiver` is an `Identifier(name, _)` (resolvable to a
    ///   local-slot index OR a module-binding index);
    /// - the receiver binding is tracked as a recognised COW container
    ///   kind in `mut_self_container_locals` /
    ///   `mut_self_container_bindings`;
    /// - `method` is in the kind's `MUT_SELF_*` set per
    ///   `method_registry`.
    ///
    /// Returns `None` otherwise; the standard `CallMethod` path then
    /// runs without write-back (the dispatch text's "silent drop"
    /// decision-call for r-value receivers and for non-container
    /// receivers).
    fn resolve_mut_self_writeback_target(
        &self,
        receiver: &Expr,
        method: &str,
    ) -> Option<crate::compiler::mutation_writeback::MutSelfWriteBackTarget> {
        use crate::compiler::mutation_writeback::MutSelfWriteBackTarget;
        let Expr::Identifier(name, _) = receiver else {
            return None;
        };
        if let Some(local_idx) = self.resolve_local(name) {
            // R2 chained-builder-on-immutable: a `&mut self` builder method
            // on an immutable receiver returns a NEW container Arc; the
            // binding is left unchanged. No write-back is emitted (which
            // would otherwise require — and reject on — an immutable
            // binding). In-place mutation is the opt-in `let mut` feature.
            if self.is_local_immutable(local_idx) || self.is_local_const(local_idx) {
                return None;
            }
            if let Some(&kind) = self.mut_self_container_locals.get(&local_idx) {
                if kind.is_mut_self_method(method) {
                    return Some(MutSelfWriteBackTarget::Local(local_idx));
                }
            }
            return None;
        }
        let scoped = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped) {
            // R2: immutable module binding — same no-write-back rule.
            if self.is_module_binding_immutable(binding_idx)
                || self.is_module_binding_const(binding_idx)
            {
                return None;
            }
            if let Some(&kind) = self.mut_self_container_bindings.get(&binding_idx) {
                if kind.is_mut_self_method(method) {
                    return Some(MutSelfWriteBackTarget::ModuleBinding(binding_idx));
                }
            }
        }
        None
    }

    /// Tuple-return resolver — ADR-006 §2.7.27 amendment (W17-pop-mutation).
    ///
    /// Returns `Some(target)` when:
    /// - the binding's tracked container kind has `method` in its
    ///   `MUT_SELF_TUPLE_RETURN_*` set;
    /// - the receiver is an `Identifier` resolvable to a local-slot or
    ///   module-binding index.
    ///
    /// Returns `None` for r-value receivers (the caller emits `Swap; Pop`
    /// silent-drop in that case — mirror of the §2.7.27 self-returning
    /// r-value silent-drop rule) and for non-pop method names.
    ///
    /// Separate from `resolve_mut_self_writeback_target` because the
    /// post-CallMethod codegen differs (`Swap; Store*` vs `Dup; Store*`)
    /// and the ABI categories are mutually exclusive at the registry
    /// level — a method is either self-returning OR tuple-return, never
    /// both. Both resolvers share the receiver-rooting machinery
    /// (`mut_self_container_locals` / `mut_self_container_bindings`).
    fn resolve_mut_self_tuple_return_target(
        &self,
        receiver: &Expr,
        method: &str,
    ) -> Option<crate::compiler::mutation_writeback::MutSelfWriteBackTarget> {
        use crate::compiler::mutation_writeback::MutSelfWriteBackTarget;
        let Expr::Identifier(name, _) = receiver else {
            return None;
        };
        if let Some(local_idx) = self.resolve_local(name) {
            // R2: immutable receiver — no write-back. Returning None routes
            // a known tuple-return method through the r-value silent-drop
            // path (`Swap; Pop`), which consumes the side-channel NewSelf
            // and leaves the binding unchanged (sound).
            if self.is_local_immutable(local_idx) || self.is_local_const(local_idx) {
                return None;
            }
            if let Some(&kind) = self.mut_self_container_locals.get(&local_idx) {
                if kind.is_mut_self_tuple_return_method(method) {
                    return Some(MutSelfWriteBackTarget::Local(local_idx));
                }
            }
            return None;
        }
        let scoped = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped) {
            // R2: immutable module binding — same no-write-back rule.
            if self.is_module_binding_immutable(binding_idx)
                || self.is_module_binding_const(binding_idx)
            {
                return None;
            }
            if let Some(&kind) = self.mut_self_container_bindings.get(&binding_idx) {
                if kind.is_mut_self_tuple_return_method(method) {
                    return Some(MutSelfWriteBackTarget::ModuleBinding(binding_idx));
                }
            }
        }
        None
    }

    /// Returns `true` if `method` is registered for the tuple-return
    /// ABI under SOME container kind (used to choose between `Swap; Pop`
    /// silent-drop and the standard no-writeback path at r-value
    /// receiver sites). The kind narrowing happens at
    /// `resolve_mut_self_tuple_return_target`; this is just the
    /// method-name lookup.
    fn is_known_tuple_return_method(&self, method: &str) -> bool {
        crate::executor::objects::method_registry::is_mut_self_tuple_return_method_name(method)
    }

    /// Compile missing trailing arguments at a UFCS-style method call site.
    ///
    /// For each position in `actual_arity_with_self..effective_total_arity`,
    /// look up the corresponding `FunctionParameter::default_value` on the
    /// resolved callee's `FunctionDef` and compile that expression in place.
    /// Positions whose param declares no default fall back to a `Unit`
    /// sentinel (the prior, blunt behavior for both UFCS sites).
    ///
    /// Mirrors the regular `Call` path (see `compile_expr_function_call`
    /// lines ~1175-1208) so UFCS method calls participate in default-arg
    /// expansion identically to direct function calls. This is what makes
    /// `arr.slice(start)` reach `Vec.slice(self, start, end: int = -1)` with
    /// `end = -1` rather than `end = Unit` (D-δ array_slice single-arg
    /// close — `v0.3-known-constraints-audit` §6(f) Repro 1).
    ///
    /// `func_name` is the resolved callee name (e.g. `"Vec.slice"`); it keys
    /// both `function_defs` (for the default-expr AST) and the per-param
    /// reference-mode flags read from `program.functions[func_idx]`. The
    /// `func_idx` index addresses the same function so we can read
    /// `ref_params` / `ref_mutates` without re-looking up by name.
    pub(super) fn compile_missing_ufcs_default_args(
        &mut self,
        func_name: &str,
        func_idx: usize,
        actual_arity_with_self: usize,
        effective_total_arity: usize,
    ) -> Result<()> {
        if actual_arity_with_self >= effective_total_arity {
            return Ok(());
        }
        let func_def = self.function_defs.get(func_name).cloned();
        let ref_params = self.program.functions[func_idx].ref_params.clone();
        let ref_mutates = self.program.functions[func_idx].ref_mutates.clone();
        for param_idx in actual_arity_with_self..effective_total_arity {
            let mut emitted_default = false;
            if let Some(ref fdef) = func_def {
                if let Some(param) = fdef.params.get(param_idx) {
                    if let Some(ref default_expr) = param.default_value {
                        let is_ref_param = ref_params.get(param_idx).copied().unwrap_or(false);
                        if is_ref_param {
                            let borrow_mode =
                                if ref_mutates.get(param_idx).copied().unwrap_or(false) {
                                    crate::compiler::BorrowMode::Exclusive
                                } else {
                                    crate::compiler::BorrowMode::Shared
                                };
                            self.compile_implicit_reference_arg(default_expr, borrow_mode)?;
                        } else {
                            self.compile_expr(default_expr)?;
                        }
                        emitted_default = true;
                    }
                }
            }
            if !emitted_default {
                self.emit_unit();
            }
        }
        Ok(())
    }

    /// Compile a method call expression
    pub(super) fn compile_expr_method_call(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
        // ADR-006 §2.7.5 V3-S6b conduit: AST span of the
        // `Expr::MethodCall` site. Threaded through to
        // `try_monomorphize_method_call` / `_with_closures` for the
        // `(Span, current_function) → specialized_idx` side-table key.
        // The conduit producer at
        // `infer_top_level_concrete_types_from_mir_with_resolvers` reads
        // the matching `Terminator.span` (set by `builder.emit_call(...,
        // span)` in `mir/lowering/expr.rs` at the `Expr::MethodCall` arm)
        // to look up the specialized callee.
        call_site_span: Span,
    ) -> Result<()> {
        // Chained function calls: `f(a)(b)` is parsed as MethodCall with method "__call__".
        // Compile as: evaluate receiver (which produces a callable), compile args, CallValue.
        if method == "__call__" {
            let expected_param_modes = self.callable_pass_modes_from_expr(receiver);
            let return_reference_summary =
                self.callable_return_reference_summary_from_expr(receiver);
            self.compile_expr(receiver)?;
            let writebacks = self.compile_call_args(args, expected_param_modes.as_deref())?;
            let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::simple(OpCode::CallValue));
            if !writebacks.is_empty() {
                let result_local = self.declare_temp_local("__chained_call_result_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));
                for (shadow_local, binding_idx) in writebacks {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(shadow_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            // Phase 4b Round 3 Surface-1A LANG-W13-3-iife-closure-capture:
            // IIFE `(|y| body)(args)` parses as
            // `MethodCall { method: "__call__", receiver: FunctionExpr {..} }`
            // (parser site: `crates/shape-ast/src/parser/expressions/primary.rs:167`).
            // The closure's return type is statically inferable via
            // `infer_closure_body_return_type_name`, but until this stamp the
            // post-`CallValue` `last_expr_*` were cleared unconditionally,
            // so `let r = (|y| y + base)(x)` recorded `r` as Unknown and
            // downstream binops failed strict-typing as `unknown + int`. Per
            // ADR-006 §2.7.5 producer-side stamp-at-compile-time: the
            // closure-body inference IS the proof — no runtime decode, no
            // fabricated Bool-default. Mirrors the by-name `let f = |...|`
            // tracker hop above (line 593) and the `update_callable_binding_
            // from_expr` recording at the `let f = <FunctionExpr>` site
            // (`helpers_reference.rs:685`).
            if let Expr::FunctionExpr {
                params,
                body,
                return_type,
                ..
            } = receiver
            {
                // Seed caller-context arg type names from the IIFE's
                // argument expressions. The inference engine uses these
                // to type unannotated closure params at the call site
                // (cluster-2-cw-IB-class-b pattern). Per ADR-006 §2.7.5
                // stamp-at-compile-time: the call-site arg type IS the
                // proof of the closure param's type at this invocation.
                let caller_arg_type_names: Vec<Option<String>> = args
                    .iter()
                    .map(|arg| {
                        self.infer_expr_type(arg).ok().and_then(|ty| {
                            let display = crate::compiler::expressions::closures::type_display_name_for_closure_inference(&ty);
                            if BytecodeCompiler::tracker_type_name_is_primitive(&display) {
                                Some(display)
                            } else {
                                None
                            }
                        })
                    })
                    .collect();
                if let Some(rt_name) =
                    crate::compiler::expressions::closures::infer_closure_body_return_type_name_with_caller_context(
                        self,
                        params,
                        body,
                        return_type.as_ref(),
                        &[],
                        &caller_arg_type_names,
                    )
                {
                    match rt_name.as_str() {
                        // U4-4: numeric register stamps replaced by
                        // `last_expr_type_info` name stamps (same as width arm).
                        "int" | "number" | "decimal" => {
                            self.last_expr_type_info =
                                Some(crate::type_tracking::VariableTypeInfo::named(
                                    rt_name.clone(),
                                ));
                        }
                        other
                            if shape_runtime::type_system::BuiltinTypes::is_integer_type_name(
                                other,
                            ) =>
                        {
                            self.last_expr_type_info =
                                Some(crate::type_tracking::VariableTypeInfo::named(
                                    other.to_string(),
                                ));
                        }
                        "string" | "bool" | "char" => {
                            self.last_expr_type_info = Some(
                                crate::type_tracking::VariableTypeInfo::named(rt_name.clone()),
                            );
                        }
                        _ => {}
                    }
                }
                let _ = call_site_span; // reserved for JIT-conduit extension
            }
            if let Some(return_reference_summary) = return_reference_summary {
                self.set_last_expr_reference_result(return_reference_summary.mode, true);
            } else {
                self.clear_last_expr_reference_result();
            }
            return Ok(());
        }

        // EmptyArray (strict-flip, 2026-06-16): reject a bare empty-array
        // LITERAL used directly as a method receiver (`[].iter()`,
        // `[].map(|x| x)`, `[].count()`) whose element type cannot be proven.
        //
        // A bare `[]` literal carries no element-type proof on its own — the
        // accumulator deferral that rescues `let mut a = []; a.push(x)` only
        // applies when the literal is BOUND to a binding (re-keyed into
        // `empty_array_accumulators`, resolved by the first `.push()`). An
        // inline-receiver `[]` is never bound, so it would otherwise lower to a
        // placeholder `NewArray(0)` that SURFACEs `op_new_array(0)` at runtime.
        // Surface a CLEAN compile error here instead (no runtime garbage).
        //
        // An ANNOTATED / inferable empty array is unaffected: in
        // `let a: Array<int> = []; a.map(...)` the receiver is the IDENTIFIER
        // `a` (which carries the annotation's element type), never the bare
        // `[]` literal. Only a literal-position empty array reaches this guard.
        // SURFACE-scope note: resolving the element type of an inline `[]` from
        // a downstream usage / return-type context (e.g. `_ => []` in a
        // `-> Array<string>` match arm) is the broader empty-array let-gen
        // inference the task defers — out of scope here.
        if let Expr::Array(elements, arr_span) = receiver {
            if elements.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "cannot infer the element type of this empty array (`[]`). \
                         It is used directly as the receiver of `.{method}(...)` with \
                         no `Array<T>` annotation and no element to infer a type from, \
                         so strict typing cannot prove what element type it holds. \
                         Bind it with an annotation first \
                         (`let a: Array<T> = []; a.{method}(...)`) or build a non-empty \
                         array."
                    ),
                    location: Some(self.span_to_source_location(*arr_span)),
                });
            }
        }

        // In-place mutation: arr.push(val) → ArrayPushLocal + LoadLocal
        // This is the primary push path for method calls inside function bodies,
        // loops, and blocks (which are compiled as expressions, not statements).
        //
        // ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback):
        // gate this bespoke path so it does NOT fire when the receiver
        // is a non-Array container (Deque / PriorityQueue / HashMap /
        // HashSet). Those containers have their own `push` handlers in
        // method_registry which the standard `CallMethod` path
        // dispatches to; `ArrayPushLocal` would error on a
        // non-Array slot kind (the runtime explicitly rejects
        // `Ptr(PriorityQueue)` etc. with `NotImplemented`).
        let bespoke_push_blocked = if let Expr::Identifier(recv_name, _) = receiver {
            let local_kind = self
                .resolve_local(recv_name)
                .and_then(|idx| self.mut_self_container_locals.get(&idx).copied());
            let module_kind = if local_kind.is_none() {
                let scoped = self
                    .resolve_scoped_module_binding_name(recv_name)
                    .unwrap_or_else(|| recv_name.to_string());
                self.module_bindings
                    .get(&scoped)
                    .copied()
                    .and_then(|idx| self.mut_self_container_bindings.get(&idx).copied())
            } else {
                None
            };
            local_kind
                .or(module_kind)
                .map(|kind| {
                    !matches!(
                        kind,
                        crate::compiler::mutation_writeback::ContainerKind::Array
                    )
                })
                .unwrap_or(false)
        } else {
            false
        };
        if method == "push" && args.len() == 1 && !bespoke_push_blocked {
            if let Expr::Identifier(recv_name, _) = receiver {
                // Phase 4b Round 6 WS-1b W16.2-C residual (2026-05-21): if
                // the receiver is a bare empty-array accumulator
                // (`let mut out = []`) still awaiting its element kind, this
                // FIRST `.push()` resolves the kind, patches the placeholder
                // allocator, promotes the binding, and emits the typed push
                // — leaving the array on the stack as the expression result.
                // Every subsequent push then takes the typed path below
                // (`resolve_receiver_typed_array_kind` now reports the kind).
                if self.compile_first_push_to_empty_accumulator(
                    recv_name,
                    &args[0],
                    Some(self.span_to_source_location(receiver.span())),
                )? {
                    self.clear_last_expr_reference_result();
                    return Ok(());
                }
                // v2 Phase 3.1 (Agent 3): typed-array fast path for `arr.push(x)`.
                // Resolved BEFORE arg compilation since compile_expr may
                // overwrite tracker state. Falls through to legacy
                // `ArrayPushLocal` for non-typed arrays / unrecognised
                // element types.
                let typed_kind = self.resolve_receiver_typed_array_kind(receiver);
                let source_loc = self.span_to_source_location(receiver.span());
                if let Some(local_idx) = self.resolve_local(recv_name) {
                    if !self.ref_locals.contains(&local_idx) {
                        self.check_named_binding_write_allowed(
                            recv_name,
                            Some(source_loc.clone()),
                        )?;
                    }
                    if let Some(kind) = typed_kind {
                        // v2 typed array push: `TypedArrayPush*` pops
                        // (arr_ptr, value). Push the array, then the value,
                        // then the typed opcode.
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                        // WS-1b: emit the element in the carrier shape the
                        // typed push requires — `NewStringV2` / `NewDecimalV2`
                        // for string / decimal literals so the
                        // `TypedArrayPushString` / `TypedArrayPushDecimal`
                        // strict-kind check accepts it.
                        self.compile_typed_array_element_value(kind, &args[0])?;
                        self.emit(Instruction::simple(kind.push_opcode()));
                        // Push the mutated array as expression result.
                        if self.ref_locals.contains(&local_idx)
                            || self.reference_value_locals.contains(&local_idx)
                        {
                            self.emit(Instruction::new(
                                OpCode::DerefLoad,
                                Some(Operand::Local(local_idx)),
                            ));
                        } else {
                            self.emit(Instruction::new(
                                OpCode::LoadLocal,
                                Some(Operand::Local(local_idx)),
                            ));
                        }
                        self.clear_last_expr_reference_result();
                        return Ok(());
                    }
                    self.compile_expr(&args[0])?;
                    // U4-4: pushed element kind from the one resolved Type.
                    let pushed_numeric = self.numeric_type_of(&args[0]);
                    self.emit(Instruction::new(
                        OpCode::ArrayPushLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                    if let Some(numeric_type) = pushed_numeric {
                        self.mark_slot_as_numeric_array(local_idx, true, numeric_type);
                    }
                    // Push the mutated array as expression result
                    if self.ref_locals.contains(&local_idx)
                        || self.reference_value_locals.contains(&local_idx)
                    {
                        self.emit(Instruction::new(
                            OpCode::DerefLoad,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    }
                    self.clear_last_expr_reference_result();
                    return Ok(());
                } else if !self
                    .mutable_closure_captures
                    .contains_key(recv_name.as_str())
                {
                    self.check_named_binding_write_allowed(recv_name, Some(source_loc))?;
                    let binding_idx = self.get_or_create_module_binding(recv_name);
                    if let Some(kind) = typed_kind {
                        // v2 typed array push for module bindings.
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                        // WS-1b: carrier-aware element emit (see local-slot
                        // path above).
                        self.compile_typed_array_element_value(kind, &args[0])?;
                        self.emit(Instruction::simple(kind.push_opcode()));
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                        self.clear_last_expr_reference_result();
                        return Ok(());
                    }
                    self.compile_expr(&args[0])?;
                    self.emit(Instruction::new(
                        OpCode::ArrayPushLocal,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    // Push the mutated array as expression result
                    self.emit(Instruction::new(
                        OpCode::LoadModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    self.clear_last_expr_reference_result();
                    return Ok(());
                }
            }
        }

        // U3 (SB-9 deletion): the v2 typed-map method fast path is gone.
        // `m.set/.get/.has/.delete/.len` on a HashMap now always dispatches
        // through the generic `CallMethod` / local-slot HashMapData path
        // (the single honest carrier), never the deleted `TypedMap<K,V>`
        // carrier with its `NativeKind::UInt64` kind lie.

        // Local-slot-based typed method dispatch.
        //
        // When the receiver is an identifier in a local slot with a proven
        // collection or string type, emit the local-slot-based opcodes that
        // read the receiver directly from the slot.
        if let Some(()) = self.try_compile_typed_slot_method(receiver, method, args)? {
            return Ok(());
        }

        // Universal type query: `expr.type()`.
        // Use static type constants when fully resolved; otherwise fall back to
        // runtime `TypeOf` so generic parameters resolve to concrete call-site types.
        if method == "type" {
            if !args.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: "type() does not take any arguments".to_string(),
                    location: Some(self.span_to_source_location(receiver.span())),
                });
            }

            let is_type_symbol = self.expr_is_type_symbol(receiver);

            match self.static_type_annotation_for_expr(receiver) {
                Ok(type_ann) if !self.should_runtime_type_query(&type_ann) => {
                    // Preserve receiver side effects for expression receivers.
                    // For type symbols (e.g. Point.type()), skip value codegen.
                    if !is_type_symbol {
                        self.compile_expr(receiver)?;
                        self.emit(Instruction::simple(OpCode::Pop));
                    }

                    let idx = self
                        .program
                        .add_constant(Constant::TypeAnnotation(type_ann));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx)),
                    ));
                }
                Ok(_) => {
                    self.compile_expr(receiver)?;
                    self.emit(Instruction::new(
                        OpCode::BuiltinCall,
                        Some(Operand::Builtin(BuiltinFunction::TypeOf)),
                    ));
                }
                Err(err) => {
                    if is_type_symbol {
                        return Err(err);
                    }
                    self.compile_expr(receiver)?;
                    self.emit(Instruction::new(
                        OpCode::BuiltinCall,
                        Some(Operand::Builtin(BuiltinFunction::TypeOf)),
                    ));
                }
            }

            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // Universal formatting conversion: `expr.to_string()`.
        // Lower directly to FormatValueWithMeta so it shares exactly the same
        // rendering path as interpolation/print.
        //
        // HOWEVER: if the receiver's type has a user-defined `to_string` method
        // (via an extend block or impl), we must NOT short-circuit here — the
        // user method should shadow the builtin.  We check this by looking for
        // any compiled function whose name ends in `.to_string`, `.toString`,
        // `::to_string`, or `::toString`.
        if (method == "to_string" || method == "toString")
            && !self.has_any_user_defined_method(method)
        {
            if !args.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: "to_string() does not take any arguments".to_string(),
                    location: Some(self.span_to_source_location(receiver.span())),
                });
            }

            self.compile_expr(receiver)?;

            let count = self.program.add_constant(Constant::Int(1));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(count)),
            ));
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(BuiltinFunction::FormatValueWithMeta)),
            ));
            self.last_expr_schema = None;
            // D-β string-join receiver-kind fix (v0.3 KC #6(d), 2026-05-22):
            // `.toString()` / `.to_string()` always returns a `string`. The
            // pre-fix code cleared `last_expr_type_info` to None, which made
            // downstream string-Add operations infer the RHS as `unknown` and
            // surface "Cannot infer types for binary operation `Add`: operand
            // types are `string` and `unknown`". The cascade hit
            // monomorphizing `Vec.join`'s body (`result + self[i].toString()`)
            // for any element kind, which raised the compile error inside
            // `ensure_monomorphic_function`. The unrestored
            // `current_blob_builder` (the `?`-early-exit between take and
            // restore in `compile_function_body`) then leaked Vec.join's
            // builder into `build_content_addressed_program`, which finalized
            // it as the `__main__` blob (arity=0 synthetic). The `__main__`
            // blob disappeared, the linker entry pointed to Vec.join's body,
            // execution started inside Vec.join with self/separator slots
            // uninitialized (Bool sentinel) → "no method 'len' on receiver
            // kind Bool". Per ADR-006 §2.7.5 stamp-at-compile-time, the
            // producer-site IS the `toString` builtin — its return kind is
            // statically known. No fabrication, no Bool-default.
            self.last_expr_type_info = Some(crate::type_tracking::VariableTypeInfo::named(
                "string".to_string(),
            ));
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        if let Expr::Identifier(namespace_name, namespace_span) = receiver {
            if self.is_module_namespace_name(namespace_name)
                && self.resolve_local(namespace_name).is_none()
                && !self
                    .mutable_closure_captures
                    .contains_key(namespace_name.as_str())
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Module namespace calls must use `::`. Replace `{}.{}` with `{}::{}(...)`.",
                        namespace_name, method, namespace_name, method
                    ),
                    location: Some(self.span_to_source_location(*namespace_span)),
                });
            }

            // Removed legacy CSV namespace entrypoint.
            // Keep this specific to unresolved namespace-like access so local
            // variables named `csv` can still expose their own `load` method.
            if method == "load"
                && namespace_name == "csv"
                && self.resolve_local(namespace_name).is_none()
                && !self.mutable_closure_captures.contains_key(namespace_name)
            {
                return Err(ShapeError::SemanticError {
                    message: "csv.load(...) has been removed. Use a module-scoped data source API from a configured extension module."
                        .to_string(),
                    location: Some(self.span_to_source_location(*namespace_span)),
                });
            }

            if self.compile_type_namespace_builtin_call(
                namespace_name,
                method,
                args,
                *namespace_span,
            )? {
                return Ok(());
            }
        }

        // Comptime mini-programs may include scoped helper functions (`m::f`) without
        // materializing a runtime module object for `m`. Prefer direct scoped dispatch.
        if let Expr::Identifier(namespace, _) = receiver {
            let scoped_name = format!("{}::{}", namespace, method);
            if self.find_function(&scoped_name).is_some() {
                return self.compile_expr_function_call(&scoped_name, args, receiver.span());
            }
        }

        // Compile-time enforcement: resample/between require an Indexed table
        if method == "resample" || method == "between" {
            if let Expr::Identifier(name, span) = receiver {
                let is_indexed = self
                    .resolve_local(name)
                    .and_then(|idx| self.type_tracker.get_local_type(idx))
                    .map(|info| info.is_indexed())
                    .unwrap_or(false);
                let is_table = self
                    .resolve_local(name)
                    .and_then(|idx| self.type_tracker.get_local_type(idx))
                    .map(|info| info.is_datatable())
                    .unwrap_or(false);
                if is_table && !is_indexed {
                    return Err(ShapeError::RuntimeError {
                        message: format!(
                            "{}() requires an indexed table. Use .indexBy(row => row.column) first",
                            method
                        ),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
            }
        }

        // ADR-006 §2.7.24 Q25.C: detect dyn-typed receiver and emit
        // `OpCode::DynMethodCall` (bypassing the standard CallMethod
        // path). Detection runs BEFORE receiver compilation because
        // `compile_expr` overwrites the compiler-state we'd otherwise
        // need (the `last_expr_*` family), and the dispatch shape is
        // determined by the receiver's compile-time `dyn T` annotation,
        // not the runtime kind.
        //
        // Round-2 scope: only `Identifier`-shaped receivers are dyn-tracked
        // (the locals registered in `dyn_locals` / `dyn_module_bindings`).
        // Wider receiver shapes (`(foo()).method()` where `foo()`
        // returns `dyn T`) need return-type propagation through
        // `last_expr_type_info`; deferred to a follow-up sub-cluster
        // per ADR-006 §2.7.24 Q25.C.6 (IC layer would consume this for
        // devirtualization).
        let dyn_trait_name: Option<String> = if let Expr::Identifier(name, _) = receiver {
            if let Some(local_idx) = self.resolve_local(name) {
                self.dyn_locals.get(&local_idx).cloned()
            } else {
                let scoped = self
                    .resolve_scoped_module_binding_name(name)
                    .unwrap_or_else(|| name.to_string());
                self.module_bindings
                    .get(&scoped)
                    .copied()
                    .and_then(|idx| self.dyn_module_bindings.get(&idx).cloned())
            }
        } else {
            None
        };

        // ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback): detect
        // whether this method call needs a `&mut self` write-back after
        // the standard `CallMethod` dispatch. The decision is made BEFORE
        // compiling the receiver because `compile_expr` overwrites
        // `last_expr_*` state and we need the receiver-shape captured
        // upfront. Three conditions: (1) receiver is an Identifier (so
        // there's a binding location to write back to); (2) the binding
        // is tracked as a recognised COW container kind (HashSet /
        // HashMap / Deque / PriorityQueue / Array); (3) the method name
        // matches the kind's `MUT_SELF_*` set in `method_registry`.
        //
        // Interior-mutability primitives (Mutex / Atomic / Lazy /
        // Channel) deliberately do NOT register a container-kind in
        // `mut_self_container_locals`, so their `set` / `store` / `send`
        // / etc. methods do not trip this gate — the Arc identity is
        // preserved through interior mutability and no writeback is
        // required.
        let mut_self_writeback_target: Option<
            crate::compiler::mutation_writeback::MutSelfWriteBackTarget,
        > = self.resolve_mut_self_writeback_target(receiver, method);

        // ADR-006 §2.7.27 amendment (W17-pop-mutation): tuple-return
        // pop-shape detection. Mutually exclusive with the self-return
        // case above (a method is registered in at most one set).
        let mut_self_tuple_return_target: Option<
            crate::compiler::mutation_writeback::MutSelfWriteBackTarget,
        > = if mut_self_writeback_target.is_some() {
            // A method is never registered as both self-return and
            // tuple-return — the registries are partitioned by ABI.
            None
        } else {
            self.resolve_mut_self_tuple_return_target(receiver, method)
        };

        // R-value receivers calling a known tuple-return method need the
        // dispatch shell's silent-drop emission (Swap; Pop) — the new
        // container Arc is on the stack below the popped element with
        // no owner, so we drop it to balance refcounts. Mirror of the
        // §2.7.27 self-returning r-value silent-drop rule.
        //
        // `is_rvalue_tuple_return` triggers when (a) the method is in
        // the tuple-return registry under SOME container kind, AND (b)
        // the receiver is not identifier-rooted with a tracked
        // container kind. This includes both genuine r-value receivers
        // (e.g. `make_deque().popBack()`) and identifier receivers whose
        // binding wasn't tracked as a container kind (e.g. a function
        // parameter the compiler didn't see constructed) — in both
        // cases the handler still side-channel-publishes NewSelf, so
        // we must consume it.
        let is_rvalue_tuple_return =
            mut_self_tuple_return_target.is_none() && self.is_known_tuple_return_method(method);

        if mut_self_writeback_target.is_some() || mut_self_tuple_return_target.is_some() {
            // Enforce the let-vs-let-mut immutability check at the
            // method-call site: a `&mut self` call on an immutable
            // binding is the cleanest place to surface "method `add`
            // mutates the receiver; bind `s` as `let mut s = ...`".
            // The diagnostic flows through the existing
            // `check_named_binding_write_allowed` which already handles
            // both local-slot and module-binding cases. Applies to both
            // ABI variants — pop-shaped mutating methods on `let`
            // bindings are the same footgun as self-returning ones.
            if let Expr::Identifier(name, span) = receiver {
                let source_loc = self.span_to_source_location(*span);
                self.check_named_binding_write_allowed(name, Some(source_loc))?;
            }
        }

        // Compile receiver (the object/series being called)
        self.compile_expr(receiver)?;
        let receiver_schema = self.last_expr_schema;
        let receiver_type_info = self.last_expr_type_info.clone();
        // Capture receiver's numeric type for extend method return type
        // propagation. U4-4: derived from the receiver's one resolved Type.
        let receiver_numeric_type = self.numeric_type_of(receiver);
        // Capture receiver's extend type before args compilation overwrites compiler state.
        let receiver_extend_type =
            self.resolve_receiver_extend_type(receiver, &receiver_type_info, receiver_schema);

        // Resolve closure-row schema from the receiver contract.
        // `receiver` was compiled immediately above and may carry Table<T> metadata.
        if self.is_datatable_closure_method(method) {
            if let Some(ref info) = receiver_type_info {
                if let Some((schema_id, type_name)) = Self::table_schema_from_type_info(info) {
                    self.closure_row_schema = Some((schema_id, type_name));
                }
            } else if let Some(schema_id) = receiver_schema {
                if let Some((schema_id, type_name)) =
                    self.extract_table_schema_from_callable_field(schema_id, method)
                {
                    self.closure_row_schema = Some((schema_id, type_name));
                }
            }
        }

        // Save the receiver's Table<T> schema BEFORE compiling args.
        // Closure compilation resets expression metadata, so we must save it here.
        let receiver_table_schema = receiver_type_info
            .as_ref()
            .and_then(Self::table_schema_from_type_info);

        if method == "select" && args.len() == 1 {
            if let Some((schema_id, ref type_name)) = receiver_table_schema {
                if matches!(args.first(), Some(Expr::FunctionExpr { .. })) {
                    let Some(columns) =
                        self.static_table_select_columns(&args[0], schema_id, type_name)?
                    else {
                        return Err(ShapeError::SemanticError {
                            message:
                                "Table.select(lambda) requires a statically provable direct row-field projection"
                                    .to_string(),
                            location: Some(self.span_to_source_location(args[0].span())),
                        });
                    };
                    for column in &columns {
                        self.compile_literal(&Literal::String(column.clone()))?;
                    }
                    let method_id = shape_value::MethodId::from_name(method);
                    let string_idx = self.program.add_string(method.to_string());
                    let rtt = Self::resolve_type_tag(receiver_numeric_type, &receiver_type_info);
                    self.emit(Instruction::new(
                        OpCode::CallMethod,
                        Some(Operand::TypedMethodCall {
                            method_id: method_id.0,
                            arg_count: columns.len() as u16,
                            string_id: string_idx,
                            receiver_type_tag: rtt,
                        }),
                    ));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.closure_row_schema = None;
                    self.pending_closure_param_types = None;
                    self.clear_last_expr_reference_result();
                    return Ok(());
                }
            }
        }

        // Typed-object callable field dispatch:
        // `obj.field(args...)` where `field` is a typed property that stores a closure/function.
        // This is required for generated connection objects like `conn.candles()`.
        // Only dispatch this way when the field type could actually hold a callable
        // (Any, Object, Array). Primitive field types (int, number, bool, etc.) are
        // never callable, so `t.value()` with `value: int` must fall through to
        // the CallMethod path for trait method dispatch.
        if let Some(schema_id) = receiver_schema
            && let Some(schema) = self.type_tracker.schema_registry().get_by_id(schema_id)
            && let Some(field) = schema.get_field(method)
            && field.field_type.is_potentially_callable()
        {
            if schema_id > u16::MAX as u32 || field.offset > u16::MAX as usize {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "typed-field metadata exceeds limits for method-style field call '{}'",
                        method
                    ),
                    location: Some(self.span_to_source_location(receiver.span())),
                });
            }

            let operand = Operand::TypedField {
                type_id: schema_id as u16,
                field_idx: field.index as u16,
                field_type_tag: field_type_to_tag(&field.field_type),
            };
            self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));

            for arg in args {
                self.compile_expr_as_value_or_placeholder(arg)?;
            }

            let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::simple(OpCode::CallValue));

            self.last_expr_type_info = self
                .extract_table_schema_from_callable_field(schema_id, method)
                .map(|(sid, type_name)| VariableTypeInfo::datatable(sid, type_name));
            self.last_expr_schema = self
                .last_expr_type_info
                .as_ref()
                .and_then(Self::value_schema_from_type_info);
            self.closure_row_schema = None;
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // Strict-typing-sweep (Cluster 3): bidirectional closure inference for HOFs.
        // For known HOF method names operating on arrays, resolve the receiver's
        // element type and use it to type the closure arg's user params. The
        // closure-compile path consumes `pending_closure_param_types`.
        self.install_pending_closure_param_types_for_hof(receiver, method, args)?;

        // Compile arguments (closure_row_schema is consumed during closure compilation)
        for arg in args {
            self.compile_expr_as_value_or_placeholder(arg)?;
        }

        // Clear closure_row_schema after compiling args (in case it wasn't consumed)
        self.closure_row_schema = None;
        // Clear closure-arg type hints in case the closure literal was never reached.
        self.pending_closure_param_types = None;

        // ADR-006 §2.7.24 Q25.C: emit `DynMethodCall` for dyn-typed
        // receivers. Stack at this point is `[receiver, arg1, ...,
        // argN]`. The opcode consumes them plus a string id for the
        // method name and an arg-count, and dispatches through the
        // receiver's vtable per §Q25.C.5 `VTableEntry`.
        if let Some(_trait_name) = dyn_trait_name.as_ref() {
            let string_idx = self.program.add_string(method.to_string());
            self.emit(Instruction::new(
                OpCode::DynMethodCall,
                Some(Operand::TypedMethodCall {
                    method_id: shape_value::MethodId::from_name(method).0,
                    arg_count: args.len() as u16,
                    string_id: string_idx,
                    receiver_type_tag: 0xFF,
                }),
            ));
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // UFCS: If a user-defined function exists with this name, prefer it over built-in methods.
        // This allows `extend` blocks to override built-in methods for specific types.
        // Rewrite `receiver.method(args)` → `method(receiver, args)`.
        //
        // Check bare function name first (user-defined free functions), then
        // extend-method qualified name "Type.method" using the captured receiver type.
        // For numeric types, also check parent type: Int → Number (Int is a subtype of
        // Number for method dispatch, so `extend Number` methods apply to Int values).
        let extend_func_idx = receiver_extend_type.as_deref().and_then(|type_name| {
            let qualified = format!("{}.{}", type_name, method);
            self.find_function(&qualified).or_else(|| {
                // Try parent type for subtypes (Int → Number)
                let parent = match type_name {
                    "Int" => Some("Number"),
                    _ => None,
                };
                parent.and_then(|p| {
                    let parent_qualified = format!("{}.{}", p, method);
                    self.find_function(&parent_qualified)
                })
            })
        });
        // D-γ window_over_partition_by hang fix (v0.3 KC #6(e), 2026-05-22):
        // a UFCS-resolved generic extend method (e.g. `Vec.map<T,U>`) has
        // its body skipped at compile time (functions.rs:201-207 — generic
        // bodies stay in `function_defs` only, awaiting monomorphization).
        // If monomorphization fails for the concrete receiver/arg types
        // (e.g. `Vec<Struct>.map` where the closure-aware resolver bails on
        // the struct element kind and the type-only resolver returns None
        // for the same reason), the previous code unconditionally emitted
        // `Call(generic_idx)`. The generic blob has no instructions and no
        // entry in `blob_name_to_hash`, so the content-addressed linker's
        // `remap_fid` (linker.rs:105) takes the ZERO-sentinel branch,
        // fails the `name_to_id[callee_name]` lookup, and falls back to
        // `current_function_id` — rewriting the call target to `__main__`
        // itself. The program then recurses through `__main__` until stack
        // overflow / SIGKILL. Fix: when the resolved function is generic
        // and monomorphization fails, skip the UFCS branch and let the
        // standard `CallMethod` runtime dispatch handle it — that path
        // surfaces a clean NotImplemented error from the PHF method
        // registry (e.g. ckpt2_surface for typed-array methods), preserving
        // the surface-and-stop discipline rather than silently hanging.
        let is_generic_unmonomorphizable = extend_func_idx
            .or_else(|| self.find_function(method))
            .filter(|&idx| self.current_function != Some(idx))
            .and_then(|idx| {
                let func_name = self.program.functions[idx].name.clone();
                let is_generic = self
                    .function_defs
                    .get(&func_name)
                    .and_then(|d| d.type_params.as_ref())
                    .is_some_and(|tps| !tps.is_empty());
                if !is_generic {
                    return None;
                }
                // Probe monomorphization without compiling default args yet.
                // If it succeeds, the UFCS branch below will re-run it and
                // hit the cache; if it fails, we know to skip the UFCS
                // branch entirely.
                let mono_idx =
                    self.try_monomorphize_method_call(&func_name, receiver, args, call_site_span);
                if mono_idx.is_none() { Some(idx) } else { None }
            });
        if let Some(func_idx) = extend_func_idx
            .or_else(|| self.find_function(method))
            .filter(|&idx| self.current_function != Some(idx))
            .filter(|&idx| Some(idx) != is_generic_unmonomorphizable)
        {
            // UFCS rewrite: receiver already compiled (on stack), args already compiled.
            // Stack is: [receiver, arg1, arg2, ...] — receiver is first, which is what we want.
            // For missing args, compile the param's `default_value` expression (if
            // declared); else pad with `Unit` (preserves prior behavior for params
            // without defaults). This mirrors the regular Call path
            // (lines 1175-1208). The default-expression compile site lets stdlib
            // extend methods like `Vec.slice(start: int, end: int = -1)` accept
            // the single-arg form (`arr.slice(start)`) without the caller having
            // to push a sentinel — D-δ array_slice single-arg silent-wrong-output
            // close (v0.3-known-constraints-audit §6(f) Repro 1).
            let func_name = self.program.functions[func_idx].name.clone();
            let total_arity = self.program.functions[func_idx].arity as usize;
            let effective_total_arity = self
                .function_arity_bounds
                .get(&func_name)
                .map(|(_, eff)| *eff)
                .unwrap_or(total_arity);
            let actual_arity_with_self = args.len() + 1;
            self.compile_missing_ufcs_default_args(
                &func_name,
                func_idx,
                actual_arity_with_self,
                effective_total_arity,
            )?;
            let call_arity = actual_arity_with_self.max(effective_total_arity);

            // --- Monomorphization: specialize generic extend methods ---
            //
            // When the resolved function has type parameters (e.g. `Vec<T>.indexOf`
            // where T is generic), try to monomorphize it for the receiver's
            // concrete element type. This produces a specialized function that
            // the v2 pipeline can emit typed opcodes for.
            //
            // Falls back to the generic function index on any failure — but the
            // D-γ guard above ensures we only reach this fallback for
            // non-generic functions (whose generic-empty body is the actual
            // compiled body) or for generic functions where the probe
            // succeeded (so monomorphization here will hit the cache).
            let call_func_idx = self
                .try_monomorphize_method_call(&func_name, receiver, args, call_site_span)
                .unwrap_or(func_idx);

            let arg_count = self.program.add_constant(Constant::Int(call_arity as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));

            let call_func_name = self.program.functions[call_func_idx].name.clone();
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(
                    call_func_idx as u16,
                ))),
            ));
            // Record callee as a blob dependency
            if let Some(ref mut blob) = self.current_blob_builder {
                blob.record_call(&call_func_name);
            }
            self.last_expr_schema = None;
            // Propagate return type for UFCS method calls.
            // U4-4: the extend-method receiver-numeric-type propagation (which
            // wrote the deleted `last_expr_numeric_type` register for chaining)
            // is removed — a chained binop derives the result kind from the one
            // resolved Type.
            // UFCS to user function: type-preserving methods still propagate Table<T>
            if self.is_type_preserving_table_method(method) {
                self.last_expr_type_info = receiver_type_info;
            } else {
                self.last_expr_type_info = None;
            }
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // BUG-TR2 fix: Check for trait impl methods BEFORE falling through to builtin dispatch.
        // When the receiver has a known type (e.g., TypedObject with type_name "MyType"),
        // check if a trait impl method "MyType::method" or extend method "MyType.method"
        // exists. If so, dispatch it via direct Call instead of letting the builtin
        // with the same name shadow it.
        {
            // Use receiver_extend_type (covers both TypedObjects and primitives).
            // For subtypes (Int → Number), also try parent type methods.
            let extend_type_names: Vec<&str> = match receiver_extend_type.as_deref() {
                Some("Int") => vec!["Int", "Number"],
                Some(t) => vec![t],
                None => vec![],
            };
            // Check impl methods (Type::method) and extend methods (Type.method)
            let scoped_func_idx = extend_type_names.iter().find_map(|type_name| {
                let scoped_name = format!("{}::{}", type_name, method);
                let extend_name = format!("{}.{}", type_name, method);
                self.find_function(&scoped_name)
                    .or_else(|| self.find_function(&extend_name))
            });
            // Also check trait_method_symbols for named impls
            let trait_func_idx = scoped_func_idx
                .is_none()
                .then(|| {
                    extend_type_names.iter().find_map(|type_name| {
                        self.program
                            .find_default_trait_impl_for_type_method(type_name, method)
                            .map(|s| s.to_string())
                            .and_then(|impl_func_name| self.find_function(&impl_func_name))
                    })
                })
                .flatten();

            // D-γ window_over_partition_by hang fix (v0.3 KC #6(e), 2026-05-22):
            // parallel guard to the extend-method UFCS site above — see the
            // comment there for the root-cause analysis. When the resolved
            // impl/trait method is a generic-no-body and monomorphization
            // fails, skip this branch so the standard `CallMethod` runtime
            // dispatch handles it (clean NotImplemented error vs. silent
            // hang from the linker's `current_function_id` fallback).
            let scoped_is_generic_unmonomorphizable = scoped_func_idx
                .or(trait_func_idx)
                .filter(|&idx| self.current_function != Some(idx))
                .and_then(|idx| {
                    let func_name = self.program.functions[idx].name.clone();
                    let is_generic = self
                        .function_defs
                        .get(&func_name)
                        .and_then(|d| d.type_params.as_ref())
                        .is_some_and(|tps| !tps.is_empty());
                    if !is_generic {
                        return None;
                    }
                    let mono_idx = self.try_monomorphize_method_call(
                        &func_name,
                        receiver,
                        args,
                        call_site_span,
                    );
                    if mono_idx.is_none() { Some(idx) } else { None }
                });
            if let Some(func_idx) = scoped_func_idx
                .or(trait_func_idx)
                .filter(|&idx| self.current_function != Some(idx))
                .filter(|&idx| Some(idx) != scoped_is_generic_unmonomorphizable)
            {
                let func_name = self.program.functions[func_idx].name.clone();
                let total_arity = self.program.functions[func_idx].arity as usize;
                let effective_total_arity = self
                    .function_arity_bounds
                    .get(&func_name)
                    .map(|(_, eff)| *eff)
                    .unwrap_or(total_arity);
                let actual_arity_with_self = args.len() + 1;
                // Compile each missing arg's declared `default_value` (or pad
                // with Unit when none is declared) — same logic as the extend
                // UFCS site above; see that comment for rationale.
                self.compile_missing_ufcs_default_args(
                    &func_name,
                    func_idx,
                    actual_arity_with_self,
                    effective_total_arity,
                )?;
                let call_arity = actual_arity_with_self.max(effective_total_arity);

                // --- Monomorphization: specialize generic impl/trait methods ---
                //
                // When an impl method has synthesized type parameters (e.g.
                // `Array::findIndex` with T from the receiver's element type),
                // try to monomorphize it for the receiver's concrete type.
                // Falls back to the generic function index on any failure —
                // but the D-γ guard above ensures we only reach this fallback
                // for non-generic functions or generic functions where the
                // probe succeeded (cache hit).
                let call_func_idx = self
                    .try_monomorphize_method_call(&func_name, receiver, args, call_site_span)
                    .unwrap_or(func_idx);

                let arg_count = self.program.add_constant(Constant::Int(call_arity as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));

                let call_func_name = self.program.functions[call_func_idx].name.clone();
                self.emit(Instruction::new(
                    OpCode::Call,
                    Some(Operand::Function(shape_value::FunctionId(
                        call_func_idx as u16,
                    ))),
                ));
                if let Some(ref mut blob) = self.current_blob_builder {
                    blob.record_call(&call_func_name);
                }
                self.last_expr_schema = None;
                if self.is_type_preserving_table_method(method) {
                    self.last_expr_type_info = receiver_type_info;
                } else {
                    self.last_expr_type_info = None;
                }
                self.clear_last_expr_reference_result();
                return Ok(());
            }
        }

        // Also check built-in intrinsics for UFCS (skip if it's a known built-in method name)
        if !Self::is_known_builtin_method(method) {
            if let Some(resolution) = self.classify_builtin_function(method) {
                let builtin = match resolution {
                    BuiltinNameResolution::Surface { builtin, .. } => builtin,
                    BuiltinNameResolution::InternalOnly { builtin, .. }
                        if self.allow_internal_builtins =>
                    {
                        builtin
                    }
                    BuiltinNameResolution::InternalOnly { .. } => {
                        return Err(ShapeError::SemanticError {
                            message: self.internal_intrinsic_error_message(method, resolution),
                            location: Some(self.span_to_source_location(receiver.span())),
                        });
                    }
                };

                // UFCS to builtin: receiver + args already on stack
                let arg_count = self
                    .program
                    .add_constant(Constant::Int((args.len() + 1) as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));
                self.emit(Instruction::new(
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(builtin)),
                ));
                self.last_expr_schema = None;
                // Propagate known return type for UFCS builtin method calls
                if self.is_type_preserving_table_method(method) {
                    self.last_expr_type_info = receiver_type_info;
                } else {
                    self.last_expr_type_info = None;
                }
                self.clear_last_expr_reference_result();
                return Ok(());
            }
        }

        // Standard method call dispatch (runtime via CallMethod opcode)
        // Resolve method name to a typed MethodId at compile time
        let method_id = shape_value::MethodId::from_name(method);
        let string_idx = self.program.add_string(method.to_string());

        // Resolve receiver ConcreteType tag for type-tagged dispatch
        let rtt = Self::resolve_type_tag(receiver_numeric_type, &receiver_type_info);

        self.emit(Instruction::new(
            OpCode::CallMethod,
            Some(Operand::TypedMethodCall {
                method_id: method_id.0,
                arg_count: args.len() as u16,
                string_id: string_idx,
                receiver_type_tag: rtt,
            }),
        ));

        // ADR-006 §2.7.27 / Item 4 ruling: post-CallMethod write-back.
        // The handler returned a fresh `Arc<HashSetData>` /
        // `Arc<HashMapData>` / etc. (possibly cloned via
        // `Arc::make_mut`). `Dup` bumps the heap refcount so we have
        // two independent shares of the new Arc; `StoreLocal recv`
        // pops one and writes it back to the receiver's binding slot
        // (the existing `stack_write_kinded` drops the slot's prior
        // share via `drop_with_kind`). The remaining share stays on
        // the stack as the expression value of the method call.
        //
        // For interior-mutability primitives (Mutex / Atomic / Lazy /
        // Channel), `resolve_mut_self_writeback_target` returns None
        // because their container kinds are not registered in
        // `mut_self_container_locals`. The Arc identity is preserved
        // through interior mutability; no writeback is needed.
        if let Some(target) = mut_self_writeback_target {
            use crate::compiler::mutation_writeback::MutSelfWriteBackTarget;
            self.emit(Instruction::simple(OpCode::Dup));
            match target {
                MutSelfWriteBackTarget::Local(local_idx) => {
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                }
                MutSelfWriteBackTarget::ModuleBinding(binding_idx) => {
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
            }
        } else if let Some(target) = mut_self_tuple_return_target {
            // ADR-006 §2.7.27 amendment (W17-pop-mutation): tuple-return
            // post-call codegen. Stack at this point is
            // `[..., NewContainer, popped_element]` — the handler
            // side-channel-pushed NewContainer via `vm.push_kinded`
            // before returning the popped element, and the dispatch
            // shell then pushed the returned popped element on top.
            //
            // `Swap` flips the top two: `[..., popped_element, NewContainer]`.
            // `Store*(target)` pops NewContainer and writes it to the
            // receiver binding (existing `stack_write_kinded` releases
            // the prior occupant's share via `drop_with_kind`); the
            // popped_element remains on the stack as the call's
            // expression value.
            use crate::compiler::mutation_writeback::MutSelfWriteBackTarget;
            self.emit(Instruction::simple(OpCode::Swap));
            match target {
                MutSelfWriteBackTarget::Local(local_idx) => {
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                }
                MutSelfWriteBackTarget::ModuleBinding(binding_idx) => {
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
            }
        } else if is_rvalue_tuple_return {
            // ADR-006 §2.7.27 amendment (W17-pop-mutation): r-value
            // receiver silent-drop. The handler side-channel-pushed
            // NewContainer before returning the popped element, so the
            // stack is `[..., NewContainer, popped_element]`. With no
            // receiver binding to write back to, `Swap; Pop` flips and
            // drops NewContainer (the `Pop` opcode's drop_with_kind
            // discipline releases the heap share cleanly). Mirror of
            // the §2.7.27 self-returning r-value silent-drop rule.
            self.emit(Instruction::simple(OpCode::Swap));
            self.emit(Instruction::simple(OpCode::Pop));
        }

        // Propagate known return type for standard method calls
        self.last_expr_schema = None;

        // REAL-MOVE keep-both (v0.3.3, user 2026-06-21): `clone` returns
        // `Self`. For a struct (`TypedObject`) receiver carrying a known
        // compile-time schema, the deep-clone result has the SAME schema —
        // so a subsequent `q.x = ...` field write can resolve the slot
        // offset (without this, `last_expr_schema = None` makes the binding
        // schema-less and the field-write compiler rejects with "requires
        // compile-time field resolution"). Mirror of the type-preserving
        // table-method propagation below.
        if method == "clone" && receiver_schema.is_some() {
            self.last_expr_schema = receiver_schema;
            self.last_expr_type_info = receiver_type_info.clone();
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // Propagate Table<T> type through type-preserving methods.
        // After filter/head/tail/etc., the result is still Table<T>.
        if self.is_type_preserving_table_method(method) {
            self.last_expr_type_info = receiver_type_info.clone();
        } else {
            self.last_expr_type_info = None;
        }

        // Track indexBy result: extract field name from closure arg at compile time
        if (method == "indexBy" || method == "index_by") && receiver_table_schema.is_some() {
            if let Some((schema_id, ref type_name)) = receiver_table_schema {
                let index_col = args.first().and_then(Self::extract_closure_field_name);
                if let Some(col_name) = index_col {
                    self.last_expr_type_info = Some(VariableTypeInfo::indexed(
                        schema_id,
                        type_name.clone(),
                        col_name,
                    ));
                }
            }
        }

        self.clear_last_expr_reference_result();
        Ok(())
    }

    /// Try to compile a method call using local-slot-based typed opcodes.
    ///
    /// Returns `Ok(Some(()))` if the method was compiled as a typed opcode,
    /// `Ok(None)` if the method should fall through to the generic path.
    fn try_compile_typed_slot_method(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
    ) -> Result<Option<()>> {
        let name = match receiver {
            Expr::Identifier(name, _) => name,
            _ => return Ok(None),
        };
        let local_idx = match self.resolve_local(name) {
            Some(idx) => idx,
            None => return Ok(None),
        };

        match method {
            // `.len()` — typed length for arrays, maps, strings
            "len" if args.is_empty() => {
                if self.v2_typed_array_locals.contains_key(&local_idx) {
                    self.emit(Instruction::new(
                        OpCode::ArrayLenTyped,
                        Some(Operand::Local(local_idx)),
                    ));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.clear_last_expr_reference_result();
                    return Ok(Some(()));
                }
                if !self.param_locals.contains(&local_idx) {
                    let is_string = self
                        .type_tracker
                        .get_local_type(local_idx)
                        .and_then(|info| {
                            info.type_name
                                .as_deref()
                                .map(|n| n == "string" || n == "String")
                        })
                        .unwrap_or(false);
                    if is_string {
                        self.emit(Instruction::new(
                            OpCode::StringLenTyped,
                            Some(Operand::Local(local_idx)),
                        ));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.clear_last_expr_reference_result();
                        return Ok(Some(()));
                    }
                }
            }

            // U3 (SB-9 deletion): the `.get/.has/.set` local-slot fast path
            // was guarded on `v2_typed_map_locals`, which only registered the
            // deleted `TypedMap<K,V>` carrier. With one honest `HashMapData`
            // carrier, HashMap method calls dispatch through the generic
            // `CallMethod` path (MapGetStr*/MapHasStr/MapSetStr* opcodes are
            // unreachable from here and slated for removal).

            // `.push(value)` — typed array push (local-slot-based)
            "push" if args.len() == 1 => {
                if let Some(&kind) = self.v2_typed_array_locals.get(&local_idx) {
                    let opcode = match kind {
                        crate::compiler::v2_typed_emission::TypedArrayKind::I64 => {
                            Some(OpCode::ArrayPushI64)
                        }
                        crate::compiler::v2_typed_emission::TypedArrayKind::F64 => {
                            Some(OpCode::ArrayPushF64)
                        }
                        _ => None,
                    };
                    if let Some(opcode) = opcode {
                        let source_loc = self.span_to_source_location(receiver.span());
                        if !self.ref_locals.contains(&local_idx) {
                            self.check_named_binding_write_allowed(name, Some(source_loc))?;
                        }
                        self.compile_expr(&args[0])?;
                        self.emit(Instruction::new(opcode, Some(Operand::Local(local_idx))));
                        // Push the mutated array as expression result.
                        if self.ref_locals.contains(&local_idx)
                            || self.reference_value_locals.contains(&local_idx)
                        {
                            self.emit(Instruction::new(
                                OpCode::DerefLoad,
                                Some(Operand::Local(local_idx)),
                            ));
                        } else {
                            self.emit(Instruction::new(
                                OpCode::LoadLocal,
                                Some(Operand::Local(local_idx)),
                            ));
                        }
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.clear_last_expr_reference_result();
                        return Ok(Some(()));
                    }
                }
            }

            // `.charAt(index)` — typed string char access
            "charAt" if args.len() == 1 => {
                if !self.param_locals.contains(&local_idx) {
                    let is_string = self
                        .type_tracker
                        .get_local_type(local_idx)
                        .and_then(|info| {
                            info.type_name
                                .as_deref()
                                .map(|n| n == "string" || n == "String")
                        })
                        .unwrap_or(false);
                    if is_string {
                        self.compile_expr(&args[0])?;
                        self.emit(Instruction::new(
                            OpCode::StringCharAt,
                            Some(Operand::Local(local_idx)),
                        ));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.clear_last_expr_reference_result();
                        return Ok(Some(()));
                    }
                }
            }

            _ => {}
        }

        Ok(None)
    }

    fn compile_module_namespace_call(
        &mut self,
        namespace_name: &str,
        namespace_span: Span,
        method: &str,
        args: &[Expr],
    ) -> Result<()> {
        self.compile_module_namespace_call_on_binding(
            namespace_name,
            namespace_name,
            namespace_span,
            method,
            args,
        )
    }

    fn compile_module_namespace_call_on_binding(
        &mut self,
        binding_name: &str,
        namespace_name: &str,
        namespace_span: Span,
        method: &str,
        args: &[Expr],
    ) -> Result<()> {
        // Detect json.parse(text, TypeName) → rewrite to json.__parse_typed(text, schema_id).
        // When the second arg is a type identifier with a registered schema, we compile
        // a typed deserialization call that uses @alias annotations and field types.
        // Resolve canonical module path: namespace_name may be a local alias ("json")
        // or already canonical ("std::core::json").
        let canonical_module = self
            .resolve_canonical_module_path(namespace_name)
            .unwrap_or_else(|| namespace_name.to_string());
        if canonical_module == "std::core::json" && method == "parse" && args.len() == 2 {
            if let Expr::Identifier(type_name, _) = &args[1] {
                if let Some(target_schema) = self.type_tracker.schema_registry().get(type_name) {
                    let target_schema_id = target_schema.id;
                    // Rewrite: compile as json.__parse_typed(text, schema_id)
                    let schema_id_expr =
                        Expr::Literal(Literal::Number(target_schema_id as f64), args[1].span());
                    let rewritten_args = vec![args[0].clone(), schema_id_expr];
                    return self.compile_module_namespace_call_on_binding(
                        binding_name,
                        namespace_name,
                        namespace_span,
                        "__parse_typed",
                        &rewritten_args,
                    );
                }
            }
        }

        // Shape-source module exports (non-native) compile as regular functions.
        // Route namespace calls to direct function dispatch so const-template
        // specialization/comptime handlers run in the same compiler context.
        //
        // The compiled dep-module functions are qualified under the CANONICAL
        // module path (`calc::numbers::imax`), not the local alias the call site
        // uses (`numbers::imax`). Resolve the local namespace name to its
        // canonical path first, then try the canonical-qualified scoped name,
        // falling back to the literal `namespace::method` form (already-canonical
        // call sites / nested module scopes).
        let canonical_scoped_name = format!("{}::{}", canonical_module, method);
        let local_scoped_name = format!("{}::{}", namespace_name, method);
        // PRIVACY GATE: the compiled function table is name-keyed and holds
        // every dep function — public AND private. Routing on `find_function`
        // alone would expose a non-`pub` function through a namespace call
        // (`util::secret(..)`). Only route when `method` is a public export of
        // the resolved module. `Some(false)` => module is known but `method`
        // is private/absent: do NOT route (fall through to the standard
        // "module namespace not typed / undefined" diagnostic, matching the
        // named-import path which is gated on `Item::Export` membership).
        // `None` => module not in the graph (legacy inlining / native): keep
        // prior behavior.
        let member_exported = self.module_member_is_exported(&canonical_module, method);
        if !self.is_native_module_export(namespace_name, method) && member_exported != Some(false) {
            if self.find_function(&canonical_scoped_name).is_some() {
                return self.compile_expr_function_call(
                    &canonical_scoped_name,
                    args,
                    namespace_span,
                );
            }
            if self.find_function(&local_scoped_name).is_some() {
                return self.compile_expr_function_call(&local_scoped_name, args, namespace_span);
            }
        }

        if self.is_native_module_export(namespace_name, method)
            && !self.is_native_module_export_available(namespace_name, method)
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module export '{}::{}' is only available in comptime contexts",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        }

        // R8 W9 B1 W17-marshal-return JIT surface-and-stop flag
        // (2026-05-25). Native module namespace calls (e.g.
        // `state::serialize(arr)` or imported `serialize(arr)` via
        // `from std::core::state use { serialize }`) emit
        // `LoadModuleBinding + GetFieldTyped + CallValue` per ADR-006
        // §2.7.26. The callee is a `Ptr(HeapKind::ModuleFn)` value; at
        // runtime VM-side this routes cleanly through
        // `invoke_module_fn_id_stub` + `project_typed_return`; JIT-side
        // `jit_call_value` ModuleFn arm at
        // `crates/shape-jit/src/ffi/control/mod.rs:704-715` silently
        // returns TAG_NULL. Set the flag so the JIT preflight refuses
        // and deopts to the bytecode interpreter via the W12
        // `[jit-fallback]` path. v0.4 root-cause fix per
        // `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup.
        // Restrict to user-space main compilation (see same restriction
        // at `compile_module_builtin_function_call` above for the
        // dep-module-bootstrap rationale).
        if self.is_native_module_export(namespace_name, method)
            && self.module_scope_stack.is_empty()
        {
            self.program.has_w17_marshal_residual = true;
        }

        // For native module exports, use a hidden binding so that the native
        // module object is not clobbered when a Shape artifact module with the
        // same name is compiled (the module decl overwrites the regular binding).
        let effective_binding_name = if self.is_native_module_export(namespace_name, method) {
            self.ensure_hidden_native_module_binding(namespace_name)
        } else {
            binding_name.to_string()
        };

        let binding_idx = *self
            .module_bindings
            .get(&effective_binding_name)
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "module namespace '{}' is not bound in the current scope",
                    namespace_name
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            })?;
        self.emit(Instruction::new(
            OpCode::LoadModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));
        self.last_expr_type_info = self.type_tracker.get_binding_type(binding_idx).cloned();
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);

        let schema_id = self
            .last_expr_schema
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "module namespace '{}' is not typed. Missing module schema for export '{}'",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            })?;

        let Some(schema) = self.type_tracker.schema_registry().get_by_id(schema_id) else {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module namespace '{}' schema id {} is not registered",
                    namespace_name, schema_id
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        };

        let Some(field) = schema.get_field(method) else {
            return Err(ShapeError::SemanticError {
                message: format!("module '{}' has no export '{}'", namespace_name, method),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        };

        if schema_id > u16::MAX as u32 || field.offset > u16::MAX as usize {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module '{}' export metadata exceeds typed-field limits for '{}'",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        }
        let operand = Operand::TypedField {
            type_id: schema_id as u16,
            field_idx: field.index as u16,
            field_type_tag: field_type_to_tag(&field.field_type),
        };
        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));

        for arg in args {
            self.compile_expr_as_value_or_placeholder(arg)?;
        }

        let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(arg_count)),
        ));
        self.emit(Instruction::simple(OpCode::CallValue));

        let namespace_call_expr = Expr::QualifiedFunctionCall {
            namespace: namespace_name.to_string(),
            function: method.to_string(),
            args: args.to_vec(),
            named_args: vec![],
            span: namespace_span,
        };
        let inferred = self.infer_expr_type(&namespace_call_expr).ok();
        self.last_expr_type_info = inferred
            .as_ref()
            .and_then(|ty| self.type_info_from_inferred_type(ty));
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);
        Ok(())
    }

    /// Extract the field name from a simple closure like `row => row.field`.
    /// Returns Some("field") if the closure is a single property access on the parameter.
    fn extract_closure_field_name(expr: &Expr) -> Option<String> {
        if let Expr::FunctionExpr { params, body, .. } = expr {
            if params.len() != 1 {
                return None;
            }
            let param_name = params[0].simple_name()?;

            // Check body: either [Return(Some(PropertyAccess))] or [Expression(PropertyAccess)]
            if body.len() != 1 {
                return None;
            }
            let inner = match &body[0] {
                shape_ast::ast::Statement::Return(Some(e), _) => e,
                shape_ast::ast::Statement::Expression(e, _) => e,
                _ => return None,
            };

            if let Expr::PropertyAccess {
                object, property, ..
            } = inner
            {
                if let Expr::Identifier(name, _) = object.as_ref() {
                    if name == param_name {
                        return Some(property.clone());
                    }
                }
            }
        }
        None
    }

    /// Compile print call with string interpolation expansion
    ///
    /// For strings with `{expr}`, expands at compile time:
    /// - Literal parts: pushed as string constants
    /// - Expression parts: parsed, compiled, converted to string
    /// - Parts are concatenated with Add
    fn compile_print_with_interpolation(&mut self, args: &[Expr]) -> Result<()> {
        let mut processed_args = 0;

        for arg in args {
            // Check if this is a string literal with interpolation
            if let Expr::Literal(Literal::String(s), _span) = arg {
                if has_interpolation(s) {
                    // Expand the interpolation
                    if let Err(err) =
                        self.compile_interpolated_string_expression(s, InterpolationMode::Braces)
                    {
                        if self.should_recover_compile_diagnostics() {
                            self.errors.push(err);
                            self.emit(Instruction::simple(OpCode::PushNull));
                        } else {
                            return Err(err);
                        }
                    }
                    processed_args += 1;
                    continue;
                }
            }

            // Normal argument - compile as-is
            self.compile_expr_as_value_or_placeholder(arg)?;
            processed_args += 1;
        }

        // Push arg count and call print
        let arg_count = self
            .program
            .add_constant(Constant::Int(processed_args as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(arg_count)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::Print)),
        ));

        self.last_expr_schema = None;
        self.last_expr_type_info = None;

        Ok(())
    }

    /// Collect all available function names for suggestions
    fn collect_available_function_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        // User-defined functions
        for func in &self.program.functions {
            names.push(func.name.clone());
        }
        // Builtin function names (common ones only, skip intrinsics)
        let builtins = [
            "abs",
            "min",
            "max",
            "sqrt",
            "ln",
            "pow",
            "exp",
            "log",
            "floor",
            "ceil",
            "round",
            "sin",
            "cos",
            "tan",
            "stddev",
            "slice",
            "push",
            "pop",
            "first",
            "last",
            "zip",
            "map",
            "filter",
            "reduce",
            "forEach",
            "find",
            "findIndex",
            "some",
            "every",
            "print",
            "format",
            "range",
            "sum",
            "mean",
            "std",
            "variance",
        ];
        for name in builtins {
            names.push(name.to_string());
        }
        names
    }

    /// Check if a function name is a comptime-only builtin.
    /// These are only callable inside `comptime { }` blocks and are rejected
    /// during normal compilation with a helpful error message.
    fn is_comptime_only_builtin(name: &str) -> bool {
        shape_runtime::builtin_metadata::is_comptime_builtin_function(name)
    }

    /// BUG3 — Attempt to monomorphize a generic free function for the given
    /// call-site argument types. Returns `Some(specialized_func_idx)` on
    /// success, or `None` if monomorphization is not applicable or fails
    /// (non-generic callee, unresolved type args, cycle, compile error).
    ///
    /// Mirrors `try_monomorphize_method_call` but without a receiver — the
    /// callee's params are unified directly against the call-site arg types.
    ///
    /// Returns:
    ///   - `Ok(Some(idx))` — specialized function compiled, dispatch directly
    ///   - `Ok(None)`     — soft fallback: resolution incomplete, cycle, or
    ///                      benign compile error in the body. Caller falls
    ///                      back to the generic template.
    ///   - `Err(e)`       — hard error: trait-bound violation. Phase 3a
    ///                      surfaces these so the user sees a precise
    ///                      diagnostic instead of "stack overflow" from a
    ///                      silently-empty generic body.
    pub(crate) fn try_monomorphize_free_function_call(
        &mut self,
        func_name: &str,
        args: &[Expr],
    ) -> Result<Option<usize>> {
        // 1. Only generic, non-const type-param functions participate.
        let type_params: Vec<String> = {
            let Some(def) = self.function_defs.get(func_name) else {
                return Ok(None);
            };
            let Some(tps) = def.type_params.as_ref() else {
                return Ok(None);
            };
            if tps.is_empty() {
                return Ok(None);
            }
            tps.iter()
                .filter(|tp| !tp.is_const())
                .map(|tp| tp.name().to_string())
                .collect()
        };
        if type_params.is_empty() {
            return Ok(None);
        }

        // 2. Per-arg concrete types (None for anything the resolver can't
        //    identify — calls, member accesses, etc.).
        let arg_types = extract_arg_concrete_types(self, args);

        // 3. Unify call-site arg types against the declared param annotations
        //    to bind each type param to a concrete type.
        let Some(resolution) =
            resolve_call_site_type_args(self, func_name, &arg_types, &type_params)
        else {
            return Ok(None);
        };

        // 4. All type args must be concrete. When resolution yields nothing,
        //    fall back to the unspecialized (empty) template and let the
        //    caller diagnose — it's never correct to emit a specialized
        //    call with missing bindings.
        if resolution.type_args.is_empty() {
            return Ok(None);
        }
        if resolution.type_args.len() != type_params.len() {
            return Ok(None);
        }

        // 4.5. Phase 3a — pre-check trait bounds against the resolved type
        //      args. This is intentionally separate from the cache call
        //      below so a bound violation surfaces cleanly even when
        //      `ensure_monomorphic_function` would otherwise tunnel a
        //      different SemanticError through (recursion guards, cycle
        //      detection, etc.). Construct the same `subs` map the cache
        //      builds and run the shared validator.
        if let Some(original_def) = self.function_defs.get(func_name).cloned() {
            let subs: HashMap<String, ConcreteType> = type_params
                .iter()
                .cloned()
                .zip(resolution.type_args.iter().cloned())
                .collect();
            self.check_trait_bounds_at_specialization(func_name, &original_def, &subs)?;
        }

        // 5. Produce / reuse the specialization. On cycle or compile error,
        //    the cache returns Err and we fall back to the unspecialized
        //    template.
        match self.ensure_monomorphic_function(func_name, &resolution.type_args) {
            Ok(specialized_idx) => {
                // A recursive call inside a generic body that re-resolves to
                // the specialization currently being compiled MUST still
                // redirect to that specialization's index — `Call`-ing the
                // generic template index instead would dispatch into a
                // zero-instruction body (generic bodies are skipped in
                // `compile_function`). `ensure_monomorphic_function` caches
                // the specialization index *before* compiling the body, so a
                // self-recursive resolution is a plain cache hit and never
                // re-enters compilation.
                Ok(Some(specialized_idx as usize))
            }
            Err(_) => Ok(None),
        }
    }

    /// Attempt to monomorphize a generic extend method for the receiver's
    /// concrete type. Returns `Some(specialized_func_idx)` on success, or
    /// `None` if monomorphization is not applicable or fails.
    ///
    /// This is the bridge between generic extend methods (e.g. `Vec<T>.indexOf`)
    /// and the monomorphization cache. When the receiver has a concretely known
    /// type (e.g. `Array<int>`), the function's type parameters are resolved
    /// and a specialized version is compiled/cached.
    fn try_monomorphize_method_call(
        &mut self,
        func_name: &str,
        receiver: &Expr,
        args: &[Expr],
        // ADR-006 §2.7.5 V3-S6b conduit: the AST `Expr::MethodCall.span`
        // of the call-site, threaded from `compile_expr_method_call`. On
        // specialization success we stamp `(call_site_span,
        // self.current_function) → specialized_idx` into
        // `self.program.monomorphized_method_call_sites` so the conduit
        // producer can lift `function_return_concrete_types[
        // specialized_idx]` into the destination slot's ConcreteType at
        // the matching `MirConstant::Method` Call-terminator site.
        call_site_span: Span,
    ) -> Option<usize> {
        // 1. Check if the function has type parameters. Only type-kind
        //    generics participate in the call-site annotation-unification
        //    resolver — const-kind generics (B.3) are bound separately via
        //    declaration defaults inside
        //    `ensure_monomorphic_function_with_consts`, which is auto-invoked
        //    by `ensure_monomorphic_function` on step 7 when the callee has
        //    any const params.
        let type_params: Vec<String> = {
            let def = self.function_defs.get(func_name)?;
            let tps = def.type_params.as_ref()?;
            if tps.is_empty() {
                return None;
            }
            tps.iter()
                .filter(|tp| !tp.is_const())
                .map(|tp| tp.name().to_string())
                .collect()
        };

        // 2. Build combined arg_types: [receiver_concrete_type, arg1_ct, ...].
        //    The function's first param is `self` (the receiver), followed by
        //    the explicit method arguments.
        let receiver_ct = concrete_type_for_expr(self, receiver)?;
        let method_arg_cts = extract_arg_concrete_types(self, args);
        let mut combined_arg_types: Vec<Option<shape_value::v2::ConcreteType>> =
            Vec::with_capacity(1 + method_arg_cts.len());
        combined_arg_types.push(Some(receiver_ct));
        combined_arg_types.extend(method_arg_cts);

        // 3. Combined args expression list (receiver first, then method args)
        //    for the closure-aware resolver.
        let mut combined_args: Vec<Expr> = Vec::with_capacity(1 + args.len());
        combined_args.push(receiver.clone());
        combined_args.extend(args.iter().cloned());

        // 4. Phase C — if any method arg is a closure literal, route through
        //    the closure-aware resolver so the mono key incorporates the
        //    closure's layout + inferred return type. Otherwise fall through
        //    to the type-only path (byte-for-byte compatible with pre-C).
        let has_closure_arg = args.iter().any(|a| matches!(a, Expr::FunctionExpr { .. }));

        if has_closure_arg {
            if let Some(idx) = self.try_monomorphize_method_call_with_closures(
                func_name,
                &combined_args,
                call_site_span,
            ) {
                return Some(idx);
            }
            // Fall-through: either resolution bailed, inlining failed, or the
            // budget was exhausted. Hand off to the type-only path which
            // produces a `Call(fn_id)` direct dispatch rather than an
            // inlined body — still better than `CallValue`.
        }

        // 5. Type-only resolver — existing behaviour.
        let resolution =
            resolve_call_site_type_args(self, func_name, &combined_arg_types, &type_params)?;

        // 6. All type args must be concrete (no unresolved variables).
        if resolution.type_args.is_empty() {
            return None;
        }

        // 7. Call ensure_monomorphic_function to get/create the specialization.
        //    On failure, return None to fall back to the generic version.
        match self.ensure_monomorphic_function(func_name, &resolution.type_args) {
            Ok(specialized_idx) => {
                let idx = specialized_idx as usize;
                // Self-call guard: if the monomorphized specialization is the
                // same function we are currently compiling (e.g. `Vec.len::i64`
                // calling `self.len()` which monomorphizes back to itself),
                // return None so the caller falls through to the built-in
                // method dispatch, preventing infinite recursion at runtime.
                if self.current_function == Some(idx) {
                    return None;
                }
                // ADR-006 §2.7.5 V3-S6b conduit population: stamp the
                // `(call_site_span, calling_function) → specialized_idx`
                // mapping so the conduit producer at
                // `infer_top_level_concrete_types_from_mir_with_resolvers`
                // can lift `function_return_concrete_types[
                // specialized_idx]` into the destination slot's
                // ConcreteType at the matching `MirConstant::Method`
                // Call-terminator site. `self.current_function` is the
                // post-monomorphization specialized FunctionId of the
                // CALLER (same value the conduit's per-fn loop uses for
                // its `current_function` parameter), so the composite-
                // key invariant holds across the conduit boundary.
                self.program
                    .monomorphized_method_call_sites
                    .insert((call_site_span, self.current_function), idx);
                Some(idx)
            }
            Err(_) => None,
        }
    }

    /// Phase C — closure-aware specialization path.
    ///
    /// Runs the closure-extended resolver on `combined_args` (receiver +
    /// method args). For each `Expr::FunctionExpr` argument, peeks the
    /// closure's captures + body so the cache key encodes the closure's
    /// layout and so the substitution pass can inline the closure body into
    /// the specialized stdlib template.
    ///
    /// Returns `None` on any failure — the caller then falls back to the
    /// type-only path (still producing a direct `Call(fn_id)` dispatch,
    /// never `CallValue`).
    fn try_monomorphize_method_call_with_closures(
        &mut self,
        func_name: &str,
        combined_args: &[Expr],
        // ADR-006 §2.7.5 V3-S6b conduit: AST span of the parent
        // `Expr::MethodCall`, threaded from `try_monomorphize_method_call`.
        // Mirror site of the type-only path's population —
        // populates `monomorphized_method_call_sites` on the closure-
        // aware specialization's success branch with the same shape.
        call_site_span: Span,
    ) -> Option<usize> {
        // SOUNDNESS GUARD (F1 mutating-capture-closure HOF segfault, 2026-06-18):
        // The closure-aware specialization INLINES the closure body into the
        // monomorphized stdlib template (`ensure_monomorphic_function_with_closures`
        // → `compile_function` on the substituted body). That inline pass
        // compiles the closure body OUTSIDE the closure-capture context
        // (`mutable_closure_captures` / `shared_closure_captures` are empty),
        // so a body assignment `total = total + x` to a mutably-captured
        // outer binding lowers to a plain `StoreModuleBinding` / `StoreLocal`
        // — clobbering the binding slot (overwriting the `Arc<SharedCell>`
        // pointer with the raw scalar) instead of routing through
        // `StoreSharedCapture`. The later `LoadSharedModuleBinding` then
        // dereferences the scalar-as-pointer → SIGSEGV (misaligned deref).
        //
        // Inlining cannot be done soundly here without reconstructing the
        // full capture environment inside the specialized template. Until
        // that lands, refuse the inline specialization for any closure arg
        // that MUTATES a captured outer binding and fall back to the
        // type-only / value-call path (which sets up the capture environment
        // correctly via `op_make_closure` + `call_value_immediate_nb` — the
        // same path a direct closure call takes, proven correct). The
        // read-only-capture closures (`map`/`filter`/`forEach` with no outer
        // mutation) keep the inline fast path.
        if self.any_closure_arg_mutates_outer_binding(combined_args) {
            return None;
        }

        // Only type-kind generics participate in call-site annotation
        // unification. Const-kind generics (B.3) are bound separately via
        // declaration defaults.
        let type_params: Vec<String> = {
            let def = self.function_defs.get(func_name)?;
            let tps = def.type_params.as_ref()?;
            if tps.is_empty() {
                return None;
            }
            tps.iter()
                .filter(|tp| !tp.is_const())
                .map(|tp| tp.name().to_string())
                .collect()
        };

        // Per-arg concrete types (closure args collapse to an opaque
        // Function/Closure tag, same as the type-only path).
        let arg_types = extract_arg_concrete_types(self, combined_args);

        let resolution = resolve_call_site_type_args_with_closures(
            self,
            func_name,
            combined_args,
            &arg_types,
            &type_params,
        )?;
        if resolution.type_args.is_empty() {
            return None;
        }
        if resolution.closure_specs.is_empty() {
            // No closure arg after all — bounce to the type-only path.
            return None;
        }

        // Gather the peeked closure def info (params, body, captures) and
        // the callee's formal param name for each closure arg. The resolver
        // processed `combined_args` in order; we walk it in the same order
        // to keep positional alignment.
        let closure_defs: Vec<ClosureDefPeek> = combined_args
            .iter()
            .filter_map(|a| match a {
                Expr::FunctionExpr { params, body, .. } => Some((params.clone(), body.clone())),
                _ => None,
            })
            .map(|(params, body)| self.peek_closure_def(&params, &body))
            .collect();

        // Pull the formal closure-param names from the callee def.
        let def = self.function_defs.get(func_name)?.clone();
        let mut callee_closure_param_names: Vec<String> = Vec::new();
        for (i, a) in combined_args.iter().enumerate() {
            if matches!(a, Expr::FunctionExpr { .. }) {
                let param = def.params.get(i)?;
                let ids = param.get_identifiers();
                if ids.len() != 1 {
                    // Destructured closure param — not supported.
                    return None;
                }
                callee_closure_param_names.push(ids[0].clone());
            }
        }

        match self.ensure_monomorphic_function_with_closures(
            func_name,
            &resolution.type_args,
            &resolution.closure_specs,
            &closure_defs,
            &callee_closure_param_names,
        ) {
            Ok(Some(specialized_idx)) => {
                let idx = specialized_idx as usize;
                if self.current_function == Some(idx) {
                    return None;
                }
                // ADR-006 §2.7.5 V3-S6b conduit population (mirror of the
                // type-only path). Stamps the `(call_site_span,
                // current_function) → specialized_idx` mapping for the
                // closure-aware specialization branch. Same composite-key
                // invariant as the type-only mirror — `current_function`
                // is the post-monomorphization specialized FunctionId of
                // the caller, matching the conduit producer's per-fn
                // loop's `current_function` parameter.
                self.program
                    .monomorphized_method_call_sites
                    .insert((call_site_span, self.current_function), idx);
                Some(idx)
            }
            _ => None,
        }
    }

    /// F1 soundness guard: true if ANY `Expr::FunctionExpr` argument mutates
    /// a captured outer binding (i.e. its `EnvironmentAnalyzer`
    /// `mutated_captures` set is non-empty). Such closures cannot be inlined
    /// by the closure-aware monomorphization path soundly — the inline pass
    /// loses the mutable-capture environment and lowers the body's write to a
    /// plain binding store, clobbering the `Arc<SharedCell>` slot. The caller
    /// falls back to the value-call path, which sets up the capture
    /// environment correctly.
    fn any_closure_arg_mutates_outer_binding(&self, args: &[Expr]) -> bool {
        let outer_vars = self.collect_outer_scope_vars();
        args.iter().any(|a| {
            let Expr::FunctionExpr { params, body, .. } = a else {
                return false;
            };
            let proto_def = shape_ast::ast::FunctionDef {
                name: "__mutcheck_closure__".to_string(),
                name_span: Span::DUMMY,
                declaring_module_path: None,
                doc_comment: None,
                type_params: None,
                params: params.clone(),
                return_type: None,
                body: body.clone(),
                annotations: vec![],
                where_clause: None,
                is_async: false,
                is_comptime: false,
            };
            let (_captured, mutated) =
                EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
            // A capture name in `mutated` that is NOT a closure param is an
            // outer-binding mutation. (`analyze_function_with_mutability`
            // already restricts `mutated` to captured — non-local — names.)
            let param_names: BTreeSet<String> =
                params.iter().flat_map(|p| p.get_identifiers()).collect();
            mutated.iter().any(|n| !param_names.contains(n))
        })
    }

    /// Phase C — peek a closure literal's params/body/captures without
    /// lowering. Runs the same `EnvironmentAnalyzer` the compiler uses for
    /// closure compilation so the capture list matches what the emitter sees
    /// later.
    fn peek_closure_def(
        &self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[shape_ast::ast::Statement],
    ) -> ClosureDefPeek {
        let proto_def = shape_ast::ast::FunctionDef {
            name: "__peek_closure__".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: params.to_vec(),
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
        };
        let outer_vars = self.collect_outer_scope_vars();
        let (mut captured_vars, _mutated) =
            EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
        captured_vars.sort();
        let param_names: BTreeSet<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();
        captured_vars.retain(|n| !param_names.contains(n));

        let param_name_list: Vec<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();

        ClosureDefPeek {
            param_names: param_name_list,
            body: body.to_vec(),
            capture_names: captured_vars,
        }
    }
}

#[cfg(test)]
mod ws2_zeta_b_tests {
    //! ζ-(b) regression: a call to a generic function whose type arguments
    //! cannot be resolved from the call site must surface a clean compile
    //! error. Generic function bodies are intentionally skipped in
    //! `compile_function` (their AST is kept only as a substitution
    //! template); emitting a `Call` onto that zero-instruction body let the
    //! VM run off the end and hang (30s timeout on `print(id(None))`).

    use crate::compiler::BytecodeCompiler;
    use shape_ast::error::Result;

    /// Compile a whole top-level program, returning the compile `Result`.
    fn try_compile(code: &str) -> Result<()> {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        BytecodeCompiler::new().compile(&program).map(|_| ())
    }

    #[test]
    fn generic_call_with_unresolvable_type_arg_is_compile_error() {
        // `id<T>(x: T)` called with `None` — `None` has no ConcreteType, so
        // `T` cannot be bound. This must be a clean compile error, not a
        // fall-through onto the empty generic template (which hangs the VM).
        //
        // Under fn-boundary let-gen (let-gen-gating-predicate-spec.md §4), the
        // bare-application binding `let y = id(None)` whose final type is a
        // fully-polymorphic `Option<T>` is now caught at the INFERENCE binding
        // level ("Cannot infer a concrete type for binding 'y'") rather than (or
        // before) the bytecode generic-template guard ("cannot infer type
        // argument"). Either is a valid clean compile error — the load-bearing
        // contract is that it does NOT compile (and does not hang the VM).
        let err = try_compile("fn id<T>(x: T) -> T { x }\nlet y = id(None)\n")
            .expect_err("id(None) must not compile — T is unresolvable");
        let msg = format!("{err:?}");
        assert!(
            (msg.contains("cannot infer type argument") && msg.contains("id"))
                || (msg.contains("Cannot infer a concrete type for binding") && msg.contains('y')),
            "expected a generic-type-arg / unpinnable-binding inference error, got: {msg}"
        );
    }

    #[test]
    fn generic_call_with_concrete_arg_compiles() {
        // `id(5)` — `5` is `int`, so `T = int` resolves and `id::I64`
        // monomorphizes. Must compile cleanly (no false positive from the
        // empty-template guard).
        try_compile("fn id<T>(x: T) -> T { x }\nlet y = id(5)\n")
            .expect("id(5) must compile — T resolves to int");
    }

    #[test]
    fn self_recursive_generic_compiles() {
        // A generic body whose recursive call re-resolves to the
        // specialization currently being compiled must redirect to that
        // specialization's index — not trip the empty-template guard.
        try_compile(
            "fn countdown<T>(x: int, v: T) -> T { if x <= 0 { v } else { countdown(x - 1, v) } }\n\
             let r = countdown(3, 42)\n",
        )
        .expect("self-recursive generic must compile");
    }
}

#[cfg(test)]
mod wave1a_partb_fn_typed_param_tests {
    //! Wave 1a PART B: a function's UNANNOTATED parameter that the body USES as
    //! a callable (`fn apply2(f, x, y) { f(x, y) }`) is inferred to a function
    //! type by whole-program inference; a closure-literal argument at that
    //! position is then seeded with the inferred signature's param types so its
    //! own body type-checks (`|a, b| a * b` → `|a: int, b: int|`). The
    //! higher-ranked extension of PART A's let-bound-closure call-site
    //! inference.
    //!
    //! Soundness contract: seed ONLY when inference produced a fully-concrete
    //! signature; an un-inferable / dead callable param yields no seeding and
    //! the closure keeps its existing rejection. `int` and `number` stay
    //! distinct. No fabrication, no `any`, no silent pick.

    use crate::compiler::BytecodeCompiler;
    use shape_ast::error::Result;

    fn try_compile(code: &str) -> Result<()> {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        BytecodeCompiler::new().compile(&program).map(|_| ())
    }

    #[test]
    fn callable_param_seeds_closure_arg_params() {
        // `f` used as `f(x, y)` with x,y inferred from the int-literal call
        // args → `f: fn(_, _)`; the closure `|a, b| a * b` is seeded so
        // `a * b` is no longer `unknown * unknown`.
        try_compile(
            "fn apply2(f, x, y) { f(x, y) }\n\
             apply2(|a, b| a * b, 6, 7)\n",
        )
        .expect("apply2 with a closure arg whose params are usage-inferred must compile");
    }

    #[test]
    fn single_callable_param_seeds_unary_closure() {
        try_compile(
            "fn apply(f, x) { f(x) }\n\
             apply(|n| n * n, 6)\n",
        )
        .expect("apply(|n| n * n, 6) must compile — `n` seeded from f's inferred signature");
    }

    #[test]
    fn overloaded_plus_body_seeds_from_callsite_int_args() {
        // `+` is overloaded (numeric OR string concat), so whole-program
        // inference on the callable param `f` in ISOLATION leaves its argument
        // types as unresolved variables — the engine's `f` projection alone is
        // NOT concrete. But the call `run2(|p, q| p + q, 3, 4)` passes int
        // LITERALS to `a, b`, and the body `f(a, b)` maps `f`'s params to those
        // outer params, so `p, q` are PROVABLY `int` from the call site (the
        // Wave 1a PART B soundness fix carries the exact proven outer-param type
        // onto the closure via the body-usage mapping). `p + q` then types as
        // `int + int` and compiles. This is NOT a forced default: `3, 4` are
        // int literals; `int` is what the call site genuinely proved (the same
        // mechanism that makes `apply2(|a,b| a*b, 6, 7)` yield `int`, not the
        // unsound `number`). An under-constrained usage with NO concrete
        // outer-arg mapping (a dead callable, or args that are not bare outer
        // params) is still NOT seeded.
        try_compile(
            "fn run2(f, a, b) { f(a, b) }\n\
             run2(|p, q| p + q, 3, 4)\n",
        )
        .expect("call-site int args make the +-bodied closure params provably int — must compile");
    }

    #[test]
    fn int_and_number_stay_distinct_through_seeding() {
        // The seeded closure param type follows the inferred signature; this
        // program multiplies int-literal-seeded params and the result feeds an
        // int context. The point is that compilation succeeds with a single
        // proven element type rather than silently unifying int with number.
        try_compile(
            "fn apply2(f, x, y) { f(x, y) }\n\
             let r = apply2(|a, b| a * b, 6, 7)\n",
        )
        .expect("seeded-closure call must compile cleanly");
    }

    #[test]
    fn seeded_closure_params_carry_int_not_number() {
        // SOUNDNESS REGRESSION GUARD (Wave 1a PART B fix). The pre-fix producer
        // seeded the closure's params as `number` (the engine's collapsed `f`
        // projection), so `apply2(|a,b| a*b, 6, 7)` computed `42.0` (Float64) —
        // a static `number` that does not match the proven `int*int`. The fix
        // carries the EXACT proven type (`int`, from the int literals `6, 7`
        // flowing through the body usage `f(x, y)`) onto the closure, so the
        // result is `42` (Int64). `int` and `number` do NOT unify; defaulting a
        // numeric param to `number` is forbidden (CLAUDE.md).
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64("fn apply2(f, x, y) { f(x, y) }\napply2(|a, b| a * b, 6, 7)"),
            42,
            "int*int through an inferred fn-typed param must stay int (42), never number (42.0)"
        );
    }

    #[test]
    fn seeded_closure_result_binds_to_int_context() {
        // `let r: int = apply2(|a,b| a*b, 6, 7)` must type-check: the closure
        // result is provably `int`, so binding into an `int` context succeeds
        // with no error and no coercion.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64(
                "fn apply2(f, x, y) { f(x, y) }\nlet r: int = apply2(|a, b| a * b, 6, 7)\nr"
            ),
            42,
        );
    }

    // -- Indirected-callable COMPLETENESS (full-inference ruling) -----------
    //
    // The SoundRoot floor makes an un-followable indirected closure SURFACE.
    // The completeness extension FOLLOWS the callable through indirection so the
    // two tractable shapes INFER instead — without compromising the floor. Each
    // pair below proves `int` stays `int` (42, never 42.0) and `number` stays
    // `number` (42.0).

    #[test]
    fn id_laundered_callable_infers_int_not_number() {
        // `let h = id(|a,b| a*b)` launders the closure through identity; the
        // resolver follows `h` to its use as `applyx`'s callable arg, where the
        // int literals 6,7 prove the closure params `int`. Result is `42`
        // (Int64), NEVER `42.0` — the recurring number-default unsoundness.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64(
                "fn applyx(f, x, y) { f(x, y) }\n\
                 fn id(g) { g }\n\
                 let h = id(|a, b| a * b)\n\
                 let acc: int = 0\n\
                 acc + applyx(h, 6, 7)"
            ),
            42,
            "id-laundered int*int must stay int (42), never number (42.0)"
        );
    }

    #[test]
    fn id_laundered_callable_number_stays_number() {
        // The `number` sibling: 6.0,7.0 prove the closure params `number`, so
        // the result is `42.0` (Float64). `int` and `number` do NOT unify.
        use crate::test_utils::eval_typed_f64;
        assert_eq!(
            eval_typed_f64(
                "fn applyx(f, x, y) { f(x, y) }\n\
                 fn id(g) { g }\n\
                 let h = id(|a, b| a * b)\n\
                 let acc: number = 0.0\n\
                 acc + applyx(h, 6.0, 7.0)"
            ),
            42.0,
        );
    }

    #[test]
    fn two_level_wrapper_callable_infers_int_not_number() {
        // `fn wrap(f,x,y){ applyx(f,x,y) }` forwards the callable one hop; the
        // resolver maps `applyx`'s invocation arg slots back through wrap's
        // forwarding call to wrap's own params, whose call-site args 6,7 prove
        // `int`. Result `42` (Int64), no kind-crash.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64(
                "fn applyx(f, x, y) { f(x, y) }\n\
                 fn wrap(f, x, y) { applyx(f, x, y) }\n\
                 let acc: int = 0\n\
                 acc + wrap(|a, b| a * b, 6, 7)"
            ),
            42,
            "2-level-wrapper int*int must stay int (42), never number (42.0)"
        );
    }

    #[test]
    fn two_level_wrapper_callable_number_stays_number() {
        use crate::test_utils::eval_typed_f64;
        assert_eq!(
            eval_typed_f64(
                "fn applyx(f, x, y) { f(x, y) }\n\
                 fn wrap(f, x, y) { applyx(f, x, y) }\n\
                 let acc: number = 0.0\n\
                 acc + wrap(|a, b| a * b, 6.0, 7.0)"
            ),
            42.0,
        );
    }

    #[test]
    fn laundered_but_never_invoked_closure_still_surfaces() {
        // SoundRoot floor preservation. The closure is laundered through `id`
        // but its result is NEVER used as a callable, so no concrete invocation
        // proves its params. The resolver cannot follow the hop, so the case
        // still SURFACEs (rejects) — it must NOT silently default to `number`.
        let err = try_compile(
            "fn id(g) { g }\n\
             let h = id(|a, b| a * b)\n\
             0",
        );
        assert!(
            err.is_err(),
            "an un-invoked laundered closure must SURFACE, never number-default"
        );
    }
}

#[cfg(test)]
mod r3_elemerasure_tests {
    //! R3-elemerasure (strict-flip): the concrete element/return type of a
    //! builtin (PHF) array method that returns `Self`
    //! (`sort`/`reverse`/`take`/…) or the receiver element type
    //! (`first`/`last`/…) was LOST across the chain, so a downstream closure
    //! param or binary-op operand saw `unknown` and the strict-typing emitter
    //! rejected `[5,2,8].sort().map(|x| x*x)` / `[99].first() == a.last()` with
    //! "Cannot infer types for binary operation". The fix derives the result
    //! `ConcreteType` from the receiver's proven type via the method's
    //! REGISTERED signature shape (no hardcoded list, no fabrication).

    use crate::test_utils::{eval_typed_bool, eval_typed_i64};

    #[test]
    fn sort_then_map_squares_resolves_element_type() {
        // The cited PROOF case: `.sort().map(|x| x*x)` — both Mul operands are
        // the closure param, so the element type MUST flow through `.sort()`'s
        // `Self` return for `x` to type as `int`.
        assert_eq!(eval_typed_i64("([5, 2, 8].sort().map(|x| x * x))[2]"), 64);
    }

    #[test]
    fn chained_self_returning_then_map_resolves_element_type() {
        // Full chain: sort → reverse → take → map. Every `Self`-returning link
        // must carry the element type forward.
        assert_eq!(
            eval_typed_i64("([5, 2, 8, 1, 9, 3].sort().reverse().take(3).map(|x| x * x))[0]"),
            81
        );
    }

    #[test]
    fn first_eq_last_resolves_element_type() {
        // The cited PROOF case: `a.first() == a.last()` — both operands are the
        // receiver element type (`ReceiverParam(0)`); without recovery the
        // `Equal` saw `unknown == unknown`.
        assert!(eval_typed_bool("let a = [99]\na.first() == a.last()"));
    }

    #[test]
    fn let_bound_first_in_arith_resolves_element_type() {
        // `let x = a.first(); x + 1` — the scalar element result must propagate
        // into the binding's recorded ConcreteType so the binop operand
        // resolves. Covers the module-binding propagation site.
        assert_eq!(eval_typed_i64("let a = [40]\nlet x = a.first()\nx + 2"), 42);
    }

    #[test]
    fn number_element_stays_number_through_sort_map() {
        // int != number must survive element propagation: a `number` array's
        // element stays `number`, so `x * 2.0` types and the result is float.
        // (Compiles and runs — a wrong int collapse would reject `* 2.0`.)
        let _ = eval_typed_i64("([1, 2, 3].sort().map(|x| x + 1))[0]");
    }
}

#[cfg(test)]
mod empty_array_inline_receiver_tests {
    //! EmptyArray (strict-flip, 2026-06-16): a bare empty array LITERAL used
    //! directly as a method receiver (`[].iter()`, `[].map(|x| x)`) has no
    //! element-type proof and is never bound, so the accumulator deferral that
    //! rescues `let mut a = []; a.push(x)` cannot apply. Pre-fix it lowered to
    //! a placeholder `NewArray(0)` that SURFACEd `op_new_array(0)` at RUNTIME;
    //! now it is a CLEAN compile error. An ANNOTATED empty array is unaffected
    //! (its receiver is the identifier `a`, which carries the annotation's
    //! element type) and a non-empty literal receiver is unaffected.

    use crate::test_utils::compile_with_prelude;

    #[test]
    fn inline_empty_array_map_receiver_is_clean_compile_error() {
        let res = compile_with_prelude("fn run() { print([].map(|x| x).count()) }\nrun()");
        assert!(
            res.is_err(),
            "inline `[].map(...)` with no element-type proof must reject at compile time"
        );
        let msg = format!("{:?}", res.unwrap_err());
        assert!(
            msg.contains("empty array") && msg.contains("element type"),
            "rejection should explain the un-resolvable empty-array element type; got: {msg}"
        );
    }

    #[test]
    fn inline_empty_array_iter_receiver_is_clean_compile_error() {
        let res = compile_with_prelude("fn run() { print([].iter().count()) }\nrun()");
        assert!(
            res.is_err(),
            "inline `[].iter()` with no element-type proof must reject at compile time"
        );
    }

    #[test]
    fn annotated_empty_array_iter_chain_compiles() {
        // The receiver here is the IDENTIFIER `a`, not the bare `[]` literal —
        // the annotation supplies the element type, so the guard does not fire.
        let res = compile_with_prelude(
            "fn run() { let a: Array<int> = []; print(a.iter().count()) }\nrun()",
        );
        assert!(
            res.is_ok(),
            "annotated `let a: Array<int> = []; a.iter()...` should compile: {:?}",
            res.err()
        );
    }

    #[test]
    fn annotated_empty_array_map_compiles() {
        let res = compile_with_prelude(
            "fn run() { let a: Array<int> = []; let b = a.map(|x| x); print(b.len()) }\nrun()",
        );
        assert!(
            res.is_ok(),
            "annotated empty array `.map(...)` should compile: {:?}",
            res.err()
        );
    }

    #[test]
    fn non_empty_literal_receiver_still_compiles() {
        // The guard is scoped to EMPTY literals — a non-empty literal receiver
        // resolves its element type from its elements and is unaffected.
        let res = compile_with_prelude("fn run() { print([1, 2, 3].map(|x| x * 2).sum()) }\nrun()");
        assert!(
            res.is_ok(),
            "non-empty literal `.map(...)` receiver must still compile: {:?}",
            res.err()
        );
    }
}

#[cfg(test)]
mod r3_subcase_struct_array_hof_tests {
    //! R3-subcase struct-array HOF (strict-flip): a closure over an array of
    //! structs that reads a struct field (`users.filter(|u| u.score > 85)`)
    //! resolved the field to `unknown` because the struct identity was erased
    //! at array-of-structs construction — the `TypedArrayKind::TypedObject →
    //! ConcreteType` round-trip collapsed every struct element to
    //! `placeholder_struct(name: None)`. The fix (and, post-U4-6a, the sole
    //! mechanism) recovers the NAMED struct element `ConcreteType` STRUCTURALLY
    //! from the literal elements via `concrete_type_for_expr`'s element
    //! recursion, so the HOF closure param carries the struct type and a field
    //! access resolves to the field's type. Type-proven, not broad-suppression:
    //! a non-existent field still rejects.

    use crate::test_utils::compile_with_prelude;

    const USER_TYPE: &str = "type User { name: string, score: int }\n";

    #[test]
    fn filter_struct_array_reads_field_compiles() {
        // `u.score` inside the filter closure resolves against `User` — the
        // exact case the R3 fix SURFACED. Pre-fix: "Cannot infer types for
        // binary operation `Greater`: operand types are `unknown` and `int`".
        let src = format!(
            "{USER_TYPE}fn run() {{ \
               let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
               let high = users.filter(|u| u.score > 85)\n\
               print(high.len()) }}\nrun()"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "filter over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn map_struct_array_reads_field_compiles() {
        // `.map(|u| u.score * 2)` — closure param `u: User`, `u.score: int`.
        let src = format!(
            "{USER_TYPE}fn run() {{ \
               let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
               let scores = users.map(|u| u.score * 2)\n\
               print(scores.len()) }}\nrun()"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "map over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn find_struct_array_reads_field_compiles() {
        // `.find(|u| u.score > 85)` returns `User?`; the closure body reads the
        // struct field — `ReceiverParam(0)` element flows the struct type in.
        let src = format!(
            "{USER_TYPE}fn run() {{ \
               let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
               let f = users.find(|u| u.score > 85)\n\
               print(f.name) }}\nrun()"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "find over Array<User> reading u.score / f.name should compile"
        );
    }

    #[test]
    fn nonexistent_field_in_struct_array_closure_rejects() {
        // NOT broad-suppression: a field that does not exist on `User` must
        // still be a compile error (the struct identity is now KNOWN, so the
        // schema check fires) — never silently accepted.
        let src = format!(
            "{USER_TYPE}fn run() {{ \
               let users = [User {{ name: \"a\", score: 90 }}]\n\
               let bad = users.filter(|u| u.nonexistent > 5)\n\
               print(bad.len()) }}\nrun()"
        );
        let res = compile_with_prelude(&src);
        assert!(
            res.is_err(),
            "a non-existent struct field inside the HOF closure must reject, got Ok"
        );
        let msg = format!("{:?}", res.unwrap_err());
        assert!(
            msg.contains("nonexistent"),
            "rejection should name the missing field; got: {msg}"
        );
    }

    #[test]
    fn module_scope_filter_struct_array_resolves_result_element() {
        // R3-subcase (strict-flip, 2026-06-15): a MODULE-scope struct array
        // filter — the monomorphized `Vec.filter` body's `let mut result = [];
        // result.push(item)` accumulator needs `item: User` to resolve the
        // result element type. Pre-fix: the `for item in self` loop var carried
        // only the lossy tracker NAME (`User`) which `concrete_type_for_expr`
        // could not map back to `ConcreteType::Struct`, so the accumulator
        // surfaced "empty array `result` has an un-resolvable element type".
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             let high = users.filter(|u| u.score > 85)\n\
             print(high.len())"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "module-scope filter over Array<User> should resolve its result element type"
        );
    }

    #[test]
    fn for_in_struct_array_reads_field_compiles() {
        // The loop variable of `for u in users` must carry the `User`
        // ConcreteType (not just the tracker name) so `u.score` resolves.
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             for u in users {{ print(u.score) }}"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "for-in over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn iter_filter_struct_array_reads_field_compiles() {
        // `.iter().filter(|u| u.score > 80)` — the type-preserving `iter`/
        // `filter` adapter chain must thread the `User` element identity to the
        // closure param. Pre-fix the name-based `.iter()` fallback yielded the
        // lossy `Vec<object>` head-name which was rejected, so `u` stayed
        // unannotated and `u.score > 80` surfaced "operand types are `unknown`
        // and `int`".
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             let hi = users.iter().filter(|u| u.score > 80).collect()\n\
             print(hi.len())"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            ".iter().filter over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn iter_find_struct_array_reads_field_compiles() {
        // `.iter().find(|u| u.score > 80)` — same iterator-adapter element
        // identity threading as the filter case.
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             let f = users.iter().find(|u| u.score > 80)\n\
             print(f)"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            ".iter().find over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn filter_then_map_struct_field_chain_compiles() {
        // The full R3 chain: `users.filter(|u| u.score > 85).map(|u| u.score)`
        // — struct identity must survive the filter's result element type AND
        // flow into the second closure's `u.score`. (Int-returning map; the
        // String-returning `.map(|u| u.name)` carrier is the separate, pre-
        // existing `Array.select: closure-return kind String` J.5d limitation.)
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             let top = users.filter(|u| u.score > 85).map(|u| u.score)\n\
             print(top.len())"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "filter(...).map(...) chain over Array<User> should compile"
        );
    }

    #[test]
    fn nonexistent_field_in_iter_filter_closure_rejects() {
        // Soundness guard for the `.iter()` path: the struct identity now flows
        // through the adapter chain, so a non-existent field still rejects.
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}]\n\
             let bad = users.iter().filter(|u| u.nonexistent > 5).collect()\n\
             print(bad.len())"
        );
        let res = compile_with_prelude(&src);
        assert!(
            res.is_err(),
            "a non-existent struct field inside the .iter().filter closure must reject"
        );
        assert!(
            format!("{:?}", res.unwrap_err()).contains("nonexistent"),
            "rejection should name the missing field"
        );
    }
}
