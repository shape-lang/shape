//! Destructure patterns - extracting values from compound structures

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::VariableTypeInfo;
use shape_ast::ast::{DecompositionBinding, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};

use crate::compiler::BytecodeCompiler;

impl BytecodeCompiler {
    fn schema_from_last_expr_type_info(&self) -> Option<u32> {
        self.last_expr_type_info
            .as_ref()
            .and_then(|info| info.schema_id)
    }

    fn resolve_object_destructure_schema(
        &mut self,
        fields: &[shape_ast::ast::ObjectPatternField],
    ) -> Option<u32> {
        if let Some(schema_id) = self.last_expr_schema {
            return Some(schema_id);
        }
        if let Some(schema_id) = self.schema_from_last_expr_type_info() {
            return Some(schema_id);
        }

        let explicit_fields: Vec<&str> = fields
            .iter()
            .filter_map(|field| match field.pattern {
                shape_ast::ast::DestructurePattern::Rest(_) => None,
                _ => Some(field.key.as_str()),
            })
            .collect();
        // W17.2-C §4.D.5 migration: destructure-pattern field types are
        // unavailable here (we're inferring schema FROM the pattern);
        // route through typed variant with FieldType::Any per field.
        // Verification-pass safety net catches via `__inline_obj_*`.
        let typed_fields: Vec<(&str, shape_runtime::type_schema::FieldType)> = explicit_fields
            .iter()
            .map(|n| (*n, shape_runtime::type_schema::FieldType::Any))
            .collect();
        Some(
            self.type_tracker
                .register_inline_object_schema_typed(&typed_fields),
        )
    }

    fn resolve_decomposition_source_schema(
        &mut self,
        resolved_bindings: &[(Vec<String>, u32)],
    ) -> Option<u32> {
        if let Some(schema_id) = self.last_expr_schema {
            return Some(schema_id);
        }
        if let Some(schema_id) = self.schema_from_last_expr_type_info() {
            return Some(schema_id);
        }

        let mut ordered_fields: Vec<&str> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (fields, _) in resolved_bindings {
            for field in fields {
                if seen.insert(field.as_str()) {
                    ordered_fields.push(field.as_str());
                }
            }
        }
        // W17.2-C §4.D.5 migration: decomposition-source schema is
        // inferred from the destructure bindings; no per-field types
        // available. Route through typed variant with FieldType::Any.
        let typed_fields: Vec<(&str, shape_runtime::type_schema::FieldType)> = ordered_fields
            .iter()
            .map(|n| (*n, shape_runtime::type_schema::FieldType::Any))
            .collect();
        Some(
            self.type_tracker
                .register_inline_object_schema_typed(&typed_fields),
        )
    }

    fn resolve_typed_field_operand_destructure(
        &self,
        schema_id: u32,
        field_name: &str,
    ) -> Option<Operand> {
        if schema_id > u16::MAX as u32 {
            return None;
        }
        self.type_tracker
            .schema_registry()
            .get_by_id(schema_id)
            .and_then(|schema| {
                schema
                    .get_field(field_name)
                    .map(|field| Operand::TypedField {
                        type_id: schema_id as u16,
                        field_idx: field.index as u16,
                        field_type_tag: field_type_to_tag(&field.field_type),
                    })
            })
    }

    /// Map a schema `FieldType` to a tracked-type-name (`int`, `number`,
    /// `bool`, `string`, `decimal`) for destructured-binding type
    /// propagation. Mirrors the match-path mapping at
    /// `compiler/patterns/binding.rs` (`Pattern::Object` arm). Returns
    /// `None` for non-scalar field types (the binding then carries no
    /// scalar hint — a nested struct field is handled separately via
    /// `last_expr_schema`).
    fn destructure_field_scalar_type_name(
        field_type: &shape_runtime::type_schema::FieldType,
    ) -> Option<&'static str> {
        use shape_runtime::type_schema::FieldType;
        match field_type {
            FieldType::I64 => Some("int"),
            FieldType::F64 => Some("number"),
            FieldType::Bool => Some("bool"),
            FieldType::String => Some("string"),
            FieldType::Decimal => Some("decimal"),
            FieldType::I8 => Some("i8"),
            FieldType::U8 => Some("u8"),
            FieldType::I16 => Some("i16"),
            FieldType::U16 => Some("u16"),
            FieldType::I32 => Some("i32"),
            FieldType::U32 => Some("u32"),
            FieldType::U64 => Some("u64"),
            _ => None,
        }
    }

    fn destructure_field_type(
        &self,
        schema_id: u32,
        field_key: &str,
    ) -> Option<shape_runtime::type_schema::FieldType> {
        let schema_field = self
            .type_tracker
            .schema_registry()
            .get_by_id(schema_id)
            .and_then(|schema| schema.get_field(field_key))
            .map(|field| field.field_type.clone());
        let contract_field = self
            .type_tracker
            .get_object_field_contract(schema_id, field_key)
            .map(Self::type_annotation_to_field_type);

        match (schema_field, contract_field) {
            (Some(shape_runtime::type_schema::FieldType::Any), Some(contract)) => Some(contract),
            (Some(schema), _) => Some(schema),
            (None, Some(contract)) => Some(contract),
            (None, None) => None,
        }
    }

    fn seed_last_expr_schema_for_destructure(&mut self, schema_id: u32) {
        let schema_name = self
            .type_tracker
            .schema_registry()
            .get_by_id(schema_id)
            .map(|schema| schema.name.clone())
            .unwrap_or_else(|| format!("__typed_obj_{}", schema_id));
        self.last_expr_schema = Some(schema_id);
        self.last_expr_type_info = Some(VariableTypeInfo::known(schema_id, schema_name));
    }

    fn object_rest_schema_id_for_destructure(
        &mut self,
        base_schema_id: u32,
        excluded_keys: &[String],
    ) -> Option<u32> {
        let mut excluded_sorted: Vec<&String> = excluded_keys.iter().collect();
        excluded_sorted.sort();
        let cache_name = format!(
            "__sub_{}_exc_{}",
            base_schema_id,
            excluded_sorted
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        if let Some(schema) = self.type_tracker.schema_registry().get(&cache_name) {
            return Some(schema.id);
        }

        let subset_fields = {
            let registry = self.type_tracker.schema_registry();
            registry.get_by_id(base_schema_id).map(|base| {
                base.fields
                    .iter()
                    .filter(|field| !excluded_keys.contains(&field.name))
                    .map(|field| (field.name.clone(), field.field_type.clone()))
                    .collect::<Vec<_>>()
            })
        }?;

        Some(
            self.type_tracker
                .schema_registry_mut()
                .register_type(cache_name, subset_fields),
        )
    }

    fn seed_last_expr_type_name_for_destructure(&mut self, type_name: String) {
        self.last_expr_schema = self
            .type_tracker
            .schema_registry()
            .get(type_name.as_str())
            .map(|schema| schema.id);
        self.last_expr_type_info = Some(match self.last_expr_schema {
            Some(schema_id) => VariableTypeInfo::known(schema_id, type_name),
            None => VariableTypeInfo::named(type_name),
        });
    }

    fn array_element_type_name_from_info(type_info: &VariableTypeInfo) -> Option<String> {
        let type_name = type_info.type_name.as_deref()?;
        let inner = type_name
            .strip_prefix("Vec<")
            .or_else(|| type_name.strip_prefix("Array<"))?
            .strip_suffix('>')?;
        if inner == "unknown" || inner.contains(',') {
            None
        } else {
            Some(inner.to_string())
        }
    }

    fn typed_array_kind_from_array_type_info(
        &self,
        type_info: Option<&VariableTypeInfo>,
    ) -> Option<crate::compiler::v2_typed_emission::TypedArrayKind> {
        let elem_name = Self::array_element_type_name_from_info(type_info?)?;
        let annotation = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(elem_name)));
        self.resolve_typed_array_kind_from_annotation(&annotation)
    }

    fn emit_destructure_array_element_load(
        &mut self,
        value_local: u16,
        index: usize,
        typed_array_kind: Option<crate::compiler::v2_typed_emission::TypedArrayKind>,
    ) {
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(value_local)),
        ));
        let idx_const = if typed_array_kind.is_some() {
            self.program.add_constant(Constant::Int(index as i64))
        } else {
            self.program.add_constant(Constant::Number(index as f64))
        };
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(idx_const)),
        ));
        if let Some(kind) = typed_array_kind {
            self.emit(Instruction::simple(kind.get_opcode()));
        } else {
            self.emit(Instruction::simple(OpCode::GetProp));
        }
    }

    fn seed_array_element_context_for_pattern(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
        fallback_element_type: Option<&str>,
    ) {
        if let Some(type_name) = self.destructure_pattern_fact_type_name(pattern) {
            self.seed_last_expr_type_name_for_destructure(type_name);
        } else if let Some(type_name) = fallback_element_type {
            self.seed_last_expr_type_name_for_destructure(type_name.to_string());
        } else {
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
        }
    }

    fn destructure_pattern_fact_type_name(
        &self,
        pattern: &shape_ast::ast::DestructurePattern,
    ) -> Option<String> {
        use shape_ast::ast::DestructurePattern;
        match pattern {
            DestructurePattern::Identifier(_, span) => {
                self.destructure_binding_fact_type_name(*span)
            }
            _ => None,
        }
    }

    /// WS-4 4b: propagate the schema field's type onto the
    /// `last_expr_*` tracker state BEFORE the recursive
    /// `compile_destructure_pattern*` call, so the destructured binding
    /// inherits a proven compile-time kind. Without this, `let { x, y }
    /// = p` over a `Point { x: int, y: int }` leaves `x` and `y` with
    /// no proven kind and `x + y` fails `prove_native_kind()`.
    ///
    /// The match-path enum/struct codegen (`binding.rs`) already does
    /// this; this is the destructure-path twin. Sets `last_expr_schema`
    /// for nested `Object`-typed fields so a nested `let { a: { b } }`
    /// destructure resolves the inner schema; sets
    /// `last_expr_numeric_type` / `last_expr_type_info` for scalar
    /// fields so `let_decl_storage_hint()` emits a typed `StoreLocal`.
    fn seed_destructure_field_type(&mut self, schema_id: u32, field_key: &str) {
        use shape_runtime::type_schema::FieldType;
        // Default: clear — the extracted value is a scalar, not the
        // parent TypedObject. Overridden below when the field type is
        // recognised.
        self.last_expr_schema = None;
        self.last_expr_type_info = None;

        let Some(field_type) = self.destructure_field_type(schema_id, field_key) else {
            return;
        };

        match &field_type {
            FieldType::Object(type_name) => {
                // Nested struct field — propagate the inner schema so a
                // nested object destructure (`let { p: { x } } = …`)
                // resolves the inner field operands.
                if let Some(nested) = self.type_tracker.schema_registry().get(type_name.as_str()) {
                    self.last_expr_schema = Some(nested.id);
                    self.last_expr_type_info =
                        Some(VariableTypeInfo::known(nested.id, type_name.clone()));
                }
            }
            // W17.3-4.2 — per-container destructure narrowing. When the
            // schema field is a typed container (`Array<T>`, `HashMap<K,
            // V>`, `Set<T>`), the destructured binding inherits the
            // container's surface type-info for downstream inference.
            // Per audit §4.B.2 + close-gate signal §5.B "pattern
            // destructuring on HashMap/Set extracts key/value/element
            // types correctly".
            //
            // The bidirectional-narrowing call site in
            // `compile_destructure_pattern` consumes
            // `last_expr_type_info` for typed-receiver dispatch; setting
            // it to the container's outer-shape name lets the downstream
            // `.iter()` / `.entries()` / `.get(k)` calls resolve to the
            // correct PHF method registry without falling back to a
            // dynamic dispatch path. The element/key/value `FieldType`s
            // remain accessible via the schema-registry lookup at the
            // consumer site (no separate parallel discriminator — ADR-005
            // §1 single-discriminator preserved; the container variants
            // carry the inner FieldTypes inline).
            FieldType::Array(_) => {
                self.last_expr_type_info = Some(VariableTypeInfo::named("array".to_string()));
            }
            FieldType::HashMap { .. } => {
                self.last_expr_type_info = Some(VariableTypeInfo::named("hashmap".to_string()));
            }
            FieldType::Set(_) => {
                self.last_expr_type_info = Some(VariableTypeInfo::named("set".to_string()));
            }
            _ => {
                if let Some(tn) = Self::destructure_field_scalar_type_name(&field_type) {
                    let info = VariableTypeInfo::named(tn.to_string());
                    match &field_type {
                        FieldType::I64 => {}
                        FieldType::F64 => {}
                        FieldType::Decimal => {}
                        FieldType::I8 => {}
                        FieldType::U8 => {}
                        FieldType::I16 => {}
                        FieldType::U16 => {}
                        FieldType::I32 => {}
                        FieldType::U32 => {}
                        FieldType::U64 => {}
                        _ => {}
                    }
                    self.last_expr_type_info = Some(info);
                }
            }
        }
    }

    /// WS-4 4b: after the recursive destructure call has declared a
    /// binding for a plain `Identifier` field pattern, stamp the schema
    /// field's type onto that binding's tracker entry. The `Identifier`
    /// arm only records type info when `last_expr_schema` is set
    /// (nested-object fields); scalar fields need this explicit fixup so
    /// `get_local_type()` / `get_binding_type()` resolves for the
    /// downstream `x + y`.
    ///
    /// `is_global` selects the module-binding tracker
    /// (`set_module_binding_type_info`) for the top-level
    /// `compile_destructure_pattern_global` path, or the local tracker
    /// (`set_local_type_info`) for the function-scope path.
    fn stamp_destructure_binding_type(
        &mut self,
        field_pattern: &shape_ast::ast::DestructurePattern,
        schema_id: u32,
        field_key: &str,
        is_global: bool,
    ) {
        use shape_ast::ast::DestructurePattern;
        let DestructurePattern::Identifier(name, _) = field_pattern else {
            return;
        };
        let Some(field_type) = self.destructure_field_type(schema_id, field_key) else {
            return;
        };
        let type_name: Option<String> = match &field_type {
            shape_runtime::type_schema::FieldType::Object(tn) => Some(tn.clone()),
            other => Self::destructure_field_scalar_type_name(other).map(|s| s.to_string()),
        };
        let Some(tn) = type_name else {
            return;
        };
        if is_global {
            if let Some(slot) = self.module_bindings.get(name).copied() {
                self.set_module_binding_type_info(slot, &tn);
            }
        } else if let Some(local_idx) = self.resolve_local(name) {
            self.set_local_type_info(local_idx, &tn);
        }
    }

    fn destructure_binding_fact_type_name(&self, span: shape_ast::ast::Span) -> Option<String> {
        if span.is_dummy() {
            return None;
        }
        let ann = self.inference_facts.binding_type(span)?.to_annotation()?;
        Self::tracked_type_name_from_annotation(&ann)
    }

    fn stamp_local_destructure_binding_fact_type(
        &mut self,
        local_idx: u16,
        span: shape_ast::ast::Span,
    ) {
        if let Some(type_name) = self.destructure_binding_fact_type_name(span) {
            self.set_local_type_info(local_idx, &type_name);
            self.last_expr_schema = None;
            self.last_expr_type_info = Some(VariableTypeInfo::named(type_name));
        }
    }

    fn stamp_module_destructure_binding_fact_type(
        &mut self,
        binding_idx: u16,
        span: shape_ast::ast::Span,
    ) {
        if let Some(type_name) = self.destructure_binding_fact_type_name(span) {
            self.set_module_binding_type_info(binding_idx, &type_name);
        }
    }

    /// Compile destructuring pattern for value on stack
    /// Assumes value is already on the stack
    pub(in crate::compiler) fn compile_destructure_pattern(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
    ) -> Result<()> {
        use shape_ast::ast::DestructurePattern;

        match pattern {
            DestructurePattern::Identifier(name, span) => {
                // Simple case - store in local
                let local_idx = self.declare_local(name)?;
                if !span.is_dummy() {
                    self.local_binding_spans.insert(local_idx, *span);
                }
                self.stamp_local_destructure_binding_fact_type(local_idx, *span);
                // E+5.5 Unit C step 1: emit typed `StoreLocal<Kind>` for
                // proven Int / Bool / F64 / sub-i64-width slots so the
                // post-Unit-A native producer (PushConst Int / typed
                // arithmetic result) round-trips through the slot
                // without NaN-tag injection. Polymorphic fallback for
                // unproven hints.
                //
                // Per ADR-006 §2.7.5.1, `let_decl_storage_hint` returns
                // `Option<StorageHint>` (no `Unknown` sentinel). On `None`
                // emit the polymorphic legacy `StoreLocal`.
                //
                // U4-4: the destructured value was compiled by the caller and
                // is on the stack; no single value expr is threaded into the
                // recursive pattern walk, so the hint comes from
                // `last_expr_type_info` (numeric `value_expr` is `None`). The
                // binding's tracker numeric type is stamped separately by the
                // statement-level `propagate_initializer_type_to_slot`, which
                // DOES carry the resolved-Type-derived kind.
                match self.let_decl_storage_hint(None) {
                    Some(hint) => self.emit_store_local_for_hint(local_idx, hint),
                    None => {
                        self.emit(Instruction::new(
                            OpCode::StoreLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    }
                }
                // Track schema for typed merge optimization
                if let Some(schema_id) = self.last_expr_schema {
                    self.type_tracker.set_local_type(
                        local_idx,
                        VariableTypeInfo::known(schema_id, format!("__typed_obj_{}", schema_id)),
                    );
                }
                Ok(())
            }

            DestructurePattern::Array(patterns) => {
                // WS-3: the array rest-pattern `let [a, ...rest] = xs` is
                // v0.4-out-of-scope as a feature. At HEAD the `Rest`
                // sub-pattern compiled a `SliceAccess` path that, on the
                // common runtime shape, surfaced the internal-jargon
                // uncaught-exception dump. Reject it CLEANLY at compile
                // time instead — a compile-time `SemanticError` never
                // reaches `handle_exception`.
                if patterns
                    .iter()
                    .any(|p| matches!(p, DestructurePattern::Rest(_)))
                {
                    return Err(ShapeError::SemanticError {
                        message: "array rest-pattern (`[a, ...rest]`) is not supported".to_string(),
                        location: None,
                    });
                }

                let value_local = self.declare_temp_local("__destructure_array_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "array",
                    "Cannot destructure non-array value as array",
                )?;

                let typed_array_kind =
                    self.typed_array_kind_from_array_type_info(self.last_expr_type_info.as_ref());
                if let Some(kind) = typed_array_kind {
                    self.v2_typed_array_locals.insert(value_local, kind);
                }
                let element_type_name = self
                    .last_expr_type_info
                    .as_ref()
                    .and_then(Self::array_element_type_name_from_info);
                for (index, pat) in patterns.iter().enumerate() {
                    if let DestructurePattern::Rest(inner) = pat {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let idx_const = self.program.add_constant(Constant::Number(index as f64));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(idx_const)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::Length));
                        self.emit(Instruction::simple(OpCode::SliceAccess));
                        self.compile_destructure_pattern(inner)?;
                        break;
                    }

                    self.emit_destructure_array_element_load(value_local, index, typed_array_kind);
                    self.seed_array_element_context_for_pattern(pat, element_type_name.as_deref());
                    self.compile_destructure_pattern(pat)?;
                }

                Ok(())
            }

            DestructurePattern::Object(fields) => {
                let value_local = self.declare_temp_local("__destructure_object_")?;
                let object_schema = self.resolve_object_destructure_schema(fields);
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "object",
                    "Cannot destructure non-object value as object",
                )?;

                let mut rest_pattern: Option<&DestructurePattern> = None;
                let mut rest_excluded = Vec::new();

                let schema_id = object_schema.ok_or_else(|| ShapeError::SemanticError {
                    message: "Object destructuring requires a compile-time known schema. Runtime property lookup is disabled.".to_string(),
                    location: None,
                })?;

                for field in fields {
                    if let DestructurePattern::Rest(inner) = &field.pattern {
                        rest_pattern = Some(inner.as_ref());
                        continue;
                    }

                    rest_excluded.push(field.key.clone());
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let operand = self
                        .resolve_typed_field_operand_destructure(schema_id, &field.key)
                        .ok_or_else(|| ShapeError::SemanticError {
                            message: format!(
                                "Field '{}' is not declared in object schema for destructuring.",
                                field.key
                            ),
                            location: None,
                        })?;
                    self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    // WS-4 4b: propagate the schema field's type onto the
                    // tracker state so the destructured binding inherits a
                    // proven compile-time kind (mirrors the match-path
                    // propagation in `binding.rs`). Replaces the old
                    // unconditional `last_expr_*` clear that left `x`/`y`
                    // with an unknown kind.
                    self.seed_destructure_field_type(schema_id, &field.key);
                    self.compile_destructure_pattern(&field.pattern)?;
                    self.stamp_destructure_binding_type(
                        &field.pattern,
                        schema_id,
                        &field.key,
                        false,
                    );
                }

                if let Some(rest) = rest_pattern {
                    let rest_schema_id =
                        self.object_rest_schema_id_for_destructure(schema_id, &rest_excluded);
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit_object_rest(&rest_excluded, object_schema)?;
                    if let Some(rest_schema_id) = rest_schema_id {
                        self.seed_last_expr_schema_for_destructure(rest_schema_id);
                    }
                    self.compile_destructure_pattern(rest)?;
                }

                Ok(())
            }

            DestructurePattern::Rest(_) => {
                // Rest patterns not yet supported in VM
                Err(ShapeError::RuntimeError {
                    message: "Rest pattern cannot be used at top level".to_string(),
                    location: None,
                })
            }
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition extracts component types from intersection (A+B)
                // Splits the intersection value into separate objects by type
                let value_local = self.declare_temp_local("__decomposition_")?;
                let mut resolved_bindings = Vec::with_capacity(bindings.len());
                for binding in bindings {
                    let (fields, schema_id) = self.resolve_decomposition_binding(binding)?;
                    resolved_bindings.push((binding.name.clone(), binding.span, fields, schema_id));
                }
                let source_schema_id = self
                    .resolve_decomposition_source_schema(
                        &resolved_bindings
                            .iter()
                            .map(|(_, _, fields, schema_id)| (fields.clone(), *schema_id))
                            .collect::<Vec<_>>(),
                    )
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: "Decomposition requires compile-time known source schema. Runtime property lookup is disabled.".to_string(),
                        location: None,
                    })?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));

                for (binding_name, binding_span, fields, schema_id) in resolved_bindings {
                    for field_name in &fields {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let operand = self
                            .resolve_typed_field_operand_destructure(source_schema_id, field_name)
                            .ok_or_else(|| ShapeError::SemanticError {
                                message: format!(
                                    "Field '{}' is not declared in decomposition source schema.",
                                    field_name
                                ),
                                location: None,
                            })?;
                        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    }

                    self.emit(Instruction::new(
                        OpCode::NewTypedObject,
                        Some(Operand::TypedObjectAlloc {
                            schema_id: schema_id as u16,
                            field_count: fields.len() as u16,
                        }),
                    ));

                    let local_idx = self.declare_local(&binding_name)?;
                    if !binding_span.is_dummy() {
                        self.local_binding_spans.insert(local_idx, binding_span);
                    }
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                    let schema_name = self
                        .type_tracker
                        .schema_registry()
                        .get_by_id(schema_id)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| format!("__typed_obj_{}", schema_id));
                    self.type_tracker
                        .set_local_type(local_idx, VariableTypeInfo::known(schema_id, schema_name));
                }
                Ok(())
            }
        }
    }

    pub(in crate::compiler) fn compile_destructure_pattern_global(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
    ) -> Result<()> {
        use shape_ast::ast::DestructurePattern;

        match pattern {
            DestructurePattern::Identifier(name, span) => {
                let binding_idx = self.get_or_create_module_binding(name);
                if !span.is_dummy() {
                    self.module_binding_spans.insert(binding_idx, *span);
                }
                self.stamp_module_destructure_binding_fact_type(binding_idx, *span);
                self.emit(Instruction::new(
                    OpCode::StoreModuleBinding,
                    Some(Operand::ModuleBinding(binding_idx)),
                ));
                // Track schema for typed merge optimization
                if let Some(schema_id) = self.last_expr_schema {
                    self.type_tracker.set_binding_type(
                        binding_idx,
                        VariableTypeInfo::known(schema_id, format!("__typed_obj_{}", schema_id)),
                    );
                }
                Ok(())
            }
            DestructurePattern::Array(patterns) => {
                // WS-3: the array rest-pattern `let [a, ...rest] = xs` is
                // v0.4-out-of-scope as a feature. At HEAD the `Rest`
                // sub-pattern compiled a `SliceAccess` path that, on the
                // common runtime shape, surfaced the internal-jargon
                // uncaught-exception dump. Reject it CLEANLY at compile
                // time instead — a compile-time `SemanticError` never
                // reaches `handle_exception`.
                if patterns
                    .iter()
                    .any(|p| matches!(p, DestructurePattern::Rest(_)))
                {
                    return Err(ShapeError::SemanticError {
                        message: "array rest-pattern (`[a, ...rest]`) is not supported".to_string(),
                        location: None,
                    });
                }

                let value_local = self.declare_temp_local("__destructure_array_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "array",
                    "Cannot destructure non-array value as array",
                )?;

                let typed_array_kind =
                    self.typed_array_kind_from_array_type_info(self.last_expr_type_info.as_ref());
                if let Some(kind) = typed_array_kind {
                    self.v2_typed_array_locals.insert(value_local, kind);
                }
                let element_type_name = self
                    .last_expr_type_info
                    .as_ref()
                    .and_then(Self::array_element_type_name_from_info);
                for (index, pat) in patterns.iter().enumerate() {
                    if let DestructurePattern::Rest(inner) = pat {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let idx_const = self.program.add_constant(Constant::Number(index as f64));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(idx_const)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::Length));
                        self.emit(Instruction::simple(OpCode::SliceAccess));
                        self.compile_destructure_pattern_global(inner)?;
                        break;
                    }

                    self.emit_destructure_array_element_load(value_local, index, typed_array_kind);
                    self.seed_array_element_context_for_pattern(pat, element_type_name.as_deref());
                    self.compile_destructure_pattern_global(pat)?;
                }

                Ok(())
            }
            DestructurePattern::Object(fields) => {
                let value_local = self.declare_temp_local("__destructure_object_")?;
                let object_schema = self.resolve_object_destructure_schema(fields);
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "object",
                    "Cannot destructure non-object value as object",
                )?;

                let mut rest_pattern: Option<&DestructurePattern> = None;
                let mut rest_excluded = Vec::new();

                let schema_id = object_schema.ok_or_else(|| ShapeError::SemanticError {
                    message: "Object destructuring requires a compile-time known schema. Runtime property lookup is disabled.".to_string(),
                    location: None,
                })?;

                for field in fields {
                    if let DestructurePattern::Rest(inner) = &field.pattern {
                        rest_pattern = Some(inner.as_ref());
                        continue;
                    }

                    rest_excluded.push(field.key.clone());
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let operand = self
                        .resolve_typed_field_operand_destructure(schema_id, &field.key)
                        .ok_or_else(|| ShapeError::SemanticError {
                            message: format!(
                                "Field '{}' is not declared in object schema for destructuring.",
                                field.key
                            ),
                            location: None,
                        })?;
                    self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    // WS-4 4b: propagate the schema field's type onto the
                    // tracker state so the destructured binding inherits a
                    // proven compile-time kind (mirrors the match-path
                    // propagation in `binding.rs`). Replaces the old
                    // unconditional `last_expr_*` clear that left `x`/`y`
                    // with an unknown kind.
                    self.seed_destructure_field_type(schema_id, &field.key);
                    self.compile_destructure_pattern_global(&field.pattern)?;
                    self.stamp_destructure_binding_type(
                        &field.pattern,
                        schema_id,
                        &field.key,
                        true,
                    );
                }

                if let Some(rest) = rest_pattern {
                    let rest_schema_id =
                        self.object_rest_schema_id_for_destructure(schema_id, &rest_excluded);
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit_object_rest(&rest_excluded, object_schema)?;
                    if let Some(rest_schema_id) = rest_schema_id {
                        self.seed_last_expr_schema_for_destructure(rest_schema_id);
                    }
                    self.compile_destructure_pattern_global(rest)?;
                }

                Ok(())
            }
            DestructurePattern::Rest(_) => Err(ShapeError::RuntimeError {
                message: "Rest pattern cannot be used at top level".to_string(),
                location: None,
            }),
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition extracts component types from intersection (module_binding version)
                let value_local = self.declare_temp_local("__decomposition_")?;
                let mut resolved_bindings = Vec::with_capacity(bindings.len());
                for binding in bindings {
                    let (fields, schema_id) = self.resolve_decomposition_binding(binding)?;
                    resolved_bindings.push((binding.name.clone(), binding.span, fields, schema_id));
                }
                let source_schema_id = self
                    .resolve_decomposition_source_schema(
                        &resolved_bindings
                            .iter()
                            .map(|(_, _, fields, schema_id)| (fields.clone(), *schema_id))
                            .collect::<Vec<_>>(),
                    )
                    .ok_or_else(|| ShapeError::SemanticError {
                        message: "Decomposition requires compile-time known source schema. Runtime property lookup is disabled.".to_string(),
                        location: None,
                    })?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));

                for (binding_name, binding_span, fields, schema_id) in resolved_bindings {
                    for field_name in &fields {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let operand = self
                            .resolve_typed_field_operand_destructure(source_schema_id, field_name)
                            .ok_or_else(|| ShapeError::SemanticError {
                                message: format!(
                                    "Field '{}' is not declared in decomposition source schema.",
                                    field_name
                                ),
                                location: None,
                            })?;
                        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    }

                    self.emit(Instruction::new(
                        OpCode::NewTypedObject,
                        Some(Operand::TypedObjectAlloc {
                            schema_id: schema_id as u16,
                            field_count: fields.len() as u16,
                        }),
                    ));

                    let binding_idx = self.get_or_create_module_binding(&binding_name);
                    if !binding_span.is_dummy() {
                        self.module_binding_spans.insert(binding_idx, binding_span);
                    }
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    let schema_name = self
                        .type_tracker
                        .schema_registry()
                        .get_by_id(schema_id)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| format!("__typed_obj_{}", schema_id));
                    self.type_tracker.set_binding_type(
                        binding_idx,
                        VariableTypeInfo::known(schema_id, schema_name),
                    );
                }
                Ok(())
            }
        }
    }

    pub(in crate::compiler) fn compile_destructure_assignment(
        &mut self,
        pattern: &shape_ast::ast::DestructurePattern,
    ) -> Result<()> {
        use shape_ast::ast::DestructurePattern;

        match pattern {
            DestructurePattern::Identifier(name, _) => self.emit_store_identifier(name),
            DestructurePattern::Array(patterns) => {
                // WS-3: the array rest-pattern `[a, ...rest] = xs` is
                // v0.4-out-of-scope as a feature. At HEAD the `Rest`
                // sub-pattern compiled a `SliceAccess` path that, on the
                // common runtime shape, surfaced the internal-jargon
                // uncaught-exception dump. Reject it CLEANLY at compile
                // time instead — a compile-time `SemanticError` never
                // reaches `handle_exception`.
                if patterns
                    .iter()
                    .any(|p| matches!(p, DestructurePattern::Rest(_)))
                {
                    return Err(ShapeError::SemanticError {
                        message: "array rest-pattern (`[a, ...rest]`) is not supported".to_string(),
                        location: None,
                    });
                }

                let value_local = self.declare_temp_local("__assign_array_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "array",
                    "Cannot destructure non-array value as array",
                )?;

                let typed_array_kind =
                    self.typed_array_kind_from_array_type_info(self.last_expr_type_info.as_ref());
                if let Some(kind) = typed_array_kind {
                    self.v2_typed_array_locals.insert(value_local, kind);
                }
                let element_type_name = self
                    .last_expr_type_info
                    .as_ref()
                    .and_then(Self::array_element_type_name_from_info);
                for (index, pat) in patterns.iter().enumerate() {
                    if let DestructurePattern::Rest(inner) = pat {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let idx_const = self.program.add_constant(Constant::Number(index as f64));
                        self.emit(Instruction::new(
                            OpCode::PushConst,
                            Some(Operand::Const(idx_const)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        self.emit(Instruction::simple(OpCode::Length));
                        self.emit(Instruction::simple(OpCode::SliceAccess));
                        self.compile_destructure_assignment(inner)?;
                        break;
                    }

                    self.emit_destructure_array_element_load(value_local, index, typed_array_kind);
                    self.seed_array_element_context_for_pattern(pat, element_type_name.as_deref());
                    self.compile_destructure_assignment(pat)?;
                }

                Ok(())
            }
            DestructurePattern::Object(fields) => {
                let value_local = self.declare_temp_local("__assign_object_")?;
                let object_schema = self.last_expr_schema;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));
                self.emit_destructure_type_check(
                    value_local,
                    "object",
                    "Cannot destructure non-object value as object",
                )?;

                let mut rest_pattern: Option<&DestructurePattern> = None;
                let mut rest_excluded = Vec::new();

                let schema_id = object_schema.ok_or_else(|| ShapeError::SemanticError {
                    message: "Object destructuring assignment requires a compile-time known schema. Runtime property lookup is disabled.".to_string(),
                    location: None,
                })?;

                for field in fields {
                    if let DestructurePattern::Rest(inner) = &field.pattern {
                        rest_pattern = Some(inner.as_ref());
                        continue;
                    }

                    rest_excluded.push(field.key.clone());
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let operand = self.resolve_typed_field_operand_destructure(schema_id, &field.key).ok_or_else(|| ShapeError::SemanticError {
                        message: format!(
                            "Field '{}' is not declared in object schema for destructuring assignment.",
                            field.key
                        ),
                        location: None,
                    })?;
                    self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    self.compile_destructure_assignment(&field.pattern)?;
                }

                if let Some(rest) = rest_pattern {
                    let rest_schema_id =
                        self.object_rest_schema_id_for_destructure(schema_id, &rest_excluded);
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit_object_rest(&rest_excluded, object_schema)?;
                    if let Some(rest_schema_id) = rest_schema_id {
                        self.seed_last_expr_schema_for_destructure(rest_schema_id);
                    }
                    self.compile_destructure_assignment(rest)?;
                }

                Ok(())
            }
            DestructurePattern::Rest(_) => Err(ShapeError::RuntimeError {
                message: "Rest pattern cannot be used at top level".to_string(),
                location: None,
            }),
            DestructurePattern::Decomposition(bindings) => {
                // Decomposition extracts component types from intersection (assignment version)
                let value_local = self.declare_temp_local("__decomposition_")?;
                let source_schema_id = self.last_expr_schema.ok_or_else(|| {
                    ShapeError::SemanticError {
                        message: "Decomposition assignment requires compile-time known source schema. Runtime property lookup is disabled.".to_string(),
                        location: None,
                    }
                })?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(value_local)),
                ));

                for binding in bindings {
                    let (fields, schema_id) = self.resolve_decomposition_binding(binding)?;

                    for field_name in &fields {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(value_local)),
                        ));
                        let operand = self
                            .resolve_typed_field_operand_destructure(source_schema_id, field_name)
                            .ok_or_else(|| ShapeError::SemanticError {
                                message: format!(
                                    "Field '{}' is not declared in decomposition source schema.",
                                    field_name
                                ),
                                location: None,
                            })?;
                        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));
                    }

                    self.emit(Instruction::new(
                        OpCode::NewTypedObject,
                        Some(Operand::TypedObjectAlloc {
                            schema_id: schema_id as u16,
                            field_count: fields.len() as u16,
                        }),
                    ));

                    self.emit_store_identifier(&binding.name)?;
                }
                Ok(())
            }
        }
    }

    /// Resolve a decomposition binding's type annotation into field names and a schema ID.
    /// Handles both named types (e.g. `TypeA`) and inline object types (e.g. `{x, y}`).
    fn resolve_decomposition_binding(
        &mut self,
        binding: &DecompositionBinding,
    ) -> Result<(Vec<String>, u32)> {
        match &binding.type_annotation {
            TypeAnnotation::Object(obj_fields) => {
                // Inline object type: {x, y, z} or {x: int, y: string}
                // W17.2-C §4.D.5 migration: route through typed-with-Any
                // (NOT per-field lowering; schema layout changes from
                // Any-uniform break downstream consumers that assume
                // the legacy layout — same disposition as
                // `extract_object_schema_id_from_annotation` +
                // function-param Object case at `functions.rs`). Per-
                // field-typed schema layout migration is v0.4 W17.3+
                // territory.
                let fields: Vec<String> = obj_fields.iter().map(|f| f.name.clone()).collect();
                let typed_fields: Vec<(&str, shape_runtime::type_schema::FieldType)> = obj_fields
                    .iter()
                    .map(|f| (f.name.as_str(), shape_runtime::type_schema::FieldType::Any))
                    .collect();
                let schema_id = self
                    .type_tracker
                    .register_inline_object_schema_typed(&typed_fields);
                Ok((fields, schema_id))
            }
            _ => {
                // Named type: look up from struct registry
                let type_name = binding.type_annotation.as_simple_name().ok_or_else(|| {
                    ShapeError::SemanticError {
                        message: "Decomposition binding requires a named type or object field set"
                            .to_string(),
                        location: Some(self.span_to_source_location(binding.span)),
                    }
                })?;

                let fields = self
                    .struct_types
                    .get(type_name)
                    .map(|(f, _)| f.clone())
                    .unwrap_or_default();

                let schema_id = self
                    .type_tracker
                    .schema_registry()
                    .get(type_name)
                    .map(|s| s.id)
                    .unwrap_or_else(|| {
                        // W17.2-C §4.D.5 migration: registry miss falls
                        // back to typed-with-Any (no upstream field-type
                        // info available); verification-pass safety net.
                        let typed_fields: Vec<(&str, shape_runtime::type_schema::FieldType)> =
                            fields
                                .iter()
                                .map(|n| (n.as_str(), shape_runtime::type_schema::FieldType::Any))
                                .collect();
                        self.type_tracker
                            .register_inline_object_schema_typed(&typed_fields)
                    });

                Ok((fields, schema_id))
            }
        }
    }
}

#[cfg(test)]
mod ws3_array_rest_tests {
    //! WS-3: the array rest-pattern `[a, ...rest]` is v0.4-out-of-scope.
    //!
    //! At HEAD the `Rest` sub-pattern compiled a `SliceAccess` path that,
    //! on the common runtime shape, surfaced the internal-jargon
    //! uncaught-exception dump. It must now reject CLEANLY at compile
    //! time with a plain `SemanticError`.
    use crate::compiler::BytecodeCompiler;
    use crate::test_utils::eval_typed_i64;
    use shape_ast::parser::parse_program;

    #[test]
    fn ws3_array_rest_pattern_errors_cleanly() {
        let code = r#"
            fn run() {
                let xs = [1, 2, 3, 4]
                let [a, ...rest] = xs
                print(a)
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "array rest-pattern must be rejected at compile time"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("array rest-pattern") && msg.contains("not supported"),
            "expected a clean array rest-pattern error, got: {}",
            msg
        );
        // The clean error must NOT carry internal jargon.
        assert!(
            !msg.contains("phase-2c") && !msg.contains("ADR-006"),
            "array rest-pattern error must not dump internal jargon: {}",
            msg
        );
    }

    #[test]
    fn ws3_non_rest_array_destructure_compiles() {
        // The non-rest `let [a, b, c] = xs` form must still compile —
        // it was collateral damage of the missing `type_check_kinded`
        // `"array"` Basic-name arm before the WS-3 fix.
        let code = r#"
            fn run() {
                let xs = [10, 20, 30]
                let [a, b, c] = xs
                print(b)
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "non-rest array destructure must compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn u4_6_array_destructure_binding_fact_rejects_number_into_int() {
        let program =
            parse_program("let [a, b] = [3.0, 4.0]\nlet bad: int = a\nbad").expect("parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "array destructure should preserve number binding fact: {:?}",
            result.ok()
        );
    }

    #[test]
    fn u4_6_nested_array_destructure_binding_fact_keeps_inner_int() {
        assert_eq!(
            eval_typed_i64("let [[a, b]] = [[3, 4]]\nlet s: int = a + b\ns"),
            7
        );
    }
}

#[cfg(test)]
mod ws4_destructure_tests {
    //! WS-4 — object-destructuring regression tests (v0.3, 2026-05-21).
    //!
    //! Covers 4a (VM `let { … } = obj` no longer throws) and 4b
    //! (destructured binding types are propagated so `x + y` proves a
    //! native kind). The 4c match struct-pattern classification tests
    //! live in `compiler/patterns/binding.rs`.
    use crate::test_utils::{eval, eval_result};

    // ─── 4a: `let { … } = obj` runs in the VM ───────────────────────

    #[test]
    fn ws4_4a_global_object_destructure_runs() {
        // Pre-fix: the VM `emit_destructure_type_check("object")` guard
        // threw because `type_check_kinded` had no `"object"` arm.
        let result = eval(
            r#"
            type Point { x: int, y: int }
            let p = Point { x: 1, y: 2 }
            let { x, y } = p
            x
            "#,
        );
        assert_eq!(result.as_i64(), Some(1));
    }

    #[test]
    fn ws4_4a_global_object_destructure_second_field() {
        let result = eval(
            r#"
            type Point { x: int, y: int }
            let p = Point { x: 1, y: 2 }
            let { x, y } = p
            y
            "#,
        );
        assert_eq!(result.as_i64(), Some(2));
    }

    #[test]
    fn ws4_4a_function_scope_object_destructure_runs() {
        let result = eval(
            r#"
            type Point { x: int, y: int }
            fn first(p: Point) -> int { let { x, y } = p; x }
            first(Point { x: 7, y: 9 })
            "#,
        );
        assert_eq!(result.as_i64(), Some(7));
    }

    // ─── 4b: destructured binding types are propagated ──────────────

    #[test]
    fn ws4_4b_function_scope_destructured_int_add() {
        // Pre-fix: `x + y` failed `prove_native_kind()` — the
        // destructure path cleared `last_expr_*` and never propagated
        // the schema field's `FieldType` onto the bindings.
        let result = eval(
            r#"
            type Point { x: int, y: int }
            fn sum(p: Point) -> int { let { x, y } = p; x + y }
            sum(Point { x: 6, y: 8 })
            "#,
        );
        assert_eq!(result.as_i64(), Some(14));
    }

    #[test]
    fn ws4_4b_global_scope_destructured_int_add() {
        let result = eval(
            r#"
            type Point { x: int, y: int }
            let p = Point { x: 6, y: 8 }
            let { x, y } = p
            x + y
            "#,
        );
        assert_eq!(result.as_i64(), Some(14));
    }

    #[test]
    fn ws4_4b_destructured_number_field_add() {
        let result = eval(
            r#"
            type P { x: number, y: number }
            fn sum(p: P) -> number { let { x, y } = p; x + y }
            sum(P { x: 1.5, y: 2.5 })
            "#,
        );
        assert_eq!(result.as_f64(), Some(4.0));
    }

    #[test]
    fn ws4_4b_nested_object_destructure() {
        // The nested struct field's schema must be propagated so the
        // inner `let { v } = inner` resolves its field operand and the
        // inner binding inherits the `int` kind.
        let result = eval(
            r#"
            type Inner { v: int }
            type Outer { inner: Inner, k: int }
            fn f(o: Outer) -> int {
                let { inner, k } = o
                let { v } = inner
                v + k
            }
            f(Outer { inner: Inner { v: 5 }, k: 7 })
            "#,
        );
        assert_eq!(result.as_i64(), Some(12));
    }

    #[test]
    fn ws4_4b_destructured_string_field() {
        // String field destructure must not fail compilation; the
        // binding carries the `string` type.
        let result = eval_result(
            r#"
            type Name { first: string, last: string }
            let n = Name { first: "Ada", last: "Lovelace" }
            let { first, last } = n
            first
            "#,
        );
        assert!(
            result.is_ok(),
            "string-field destructure failed: {result:?}"
        );
    }
}
