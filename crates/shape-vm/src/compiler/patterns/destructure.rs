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

        let Some(field_type) = self
            .type_tracker
            .schema_registry()
            .get_by_id(schema_id)
            .and_then(|schema| schema.get_field(field_key))
            .map(|field| field.field_type.clone())
        else {
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
                        FieldType::I64 => {
                        }
                        FieldType::F64 => {
                        }
                        FieldType::Decimal => {
                        }
                        FieldType::I8 => {
                        }
                        FieldType::U8 => {
                        }
                        FieldType::I16 => {
                        }
                        FieldType::U16 => {
                        }
                        FieldType::I32 => {
                        }
                        FieldType::U32 => {
                        }
                        FieldType::U64 => {
                        }
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
        let Some(field_type) = self
            .type_tracker
            .schema_registry()
            .get_by_id(schema_id)
            .and_then(|schema| schema.get_field(field_key))
            .map(|field| field.field_type.clone())
        else {
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

    /// strict-flip S1 (array-destructure element-kind, 2026-06-22): stamp a
    /// freshly-bound array-element identifier with the array's PROVEN element
    /// type name. The element type name is supplied by the VariableDecl
    /// destructure site in `pending_array_destructure_element_type` (resolved
    /// there from `concrete_type_for_expr(init) == Array(elem)` — the same
    /// structural proof the operator path consumes; ADR-006 §2.7.5 stamp-at-
    /// compile-time). When the receiver is not a proven concrete `Array<T>`
    /// the pending slot is `None` and nothing is stamped — the binding keeps
    /// its prior (possibly `unknown`) kind exactly as before, and a later
    /// `let x: <concrete> = a` over an un-provable element meets the
    /// let-annotation Unknown-accept guard (FIX A). NO fabrication, NO
    /// `int`/`number` unify.
    /// strict-flip S1 (array-destructure element-kind, 2026-06-22; nested
    /// extension 2026-06-22): resolve the PROVEN ELEMENT `ConcreteType` of the
    /// array produced by `init_expr`, for a `let [a, b] = init_expr` (or nested
    /// `let [[a,b],[c,d]] = init_expr`) destructure. Returns
    /// `concrete_type_for_expr(init).Array(elem) => *elem` when the receiver
    /// proves a concrete `Array<T>`; `None` otherwise (genuinely-untyped or
    /// non-array receiver — no fabrication). The destructure recursion peels one
    /// `Array<…>` layer per nesting level from this element type.
    pub(in crate::compiler) fn array_destructure_element_concrete_type(
        &self,
        init_expr: &shape_ast::ast::Expr,
    ) -> Option<shape_value::v2::ConcreteType> {
        use shape_value::v2::ConcreteType;
        let ct = crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
            self, init_expr,
        )?;
        let ConcreteType::Array(elem) = ct else {
            return None;
        };
        Some(*elem)
    }

    /// Map a proven element `ConcreteType` to the type-info NAME used to stamp a
    /// destructured leaf binding (`"int"` / `"number"` / `"string"` / struct
    /// name / …). `None` for shapes `concrete_type_to_type_annotation` cannot
    /// render to a `Basic`/`Reference` name (no fabrication).
    fn destructure_element_type_name(ct: &shape_value::v2::ConcreteType) -> Option<String> {
        let ann = crate::compiler::expressions::closures::concrete_type_to_type_annotation(ct)?;
        match ann {
            shape_ast::ast::TypeAnnotation::Basic(n) => Some(n),
            shape_ast::ast::TypeAnnotation::Reference(p) => Some(p.to_string()),
            _ => None,
        }
    }

    /// strict-flip S1 (nested array-destructure element-kind, 2026-06-22):
    /// stamp the bindings introduced by one element sub-pattern of an array
    /// destructure. `elem_ct` is the PROVEN element `ConcreteType` of the array
    /// at THIS nesting level (peeled one `Array<…>` layer per level by the
    /// caller). Three sub-pattern shapes are handled:
    ///   - leaf `Identifier(name)` — stamp `name` with `elem_ct`'s type name.
    ///   - nested `Array(inner_pats)` — the element is itself an array; peel one
    ///     more layer (`elem_ct == Array(inner) => *inner`) and recurse per
    ///     inner sub-pattern, so `let [[a,b],..]` stamps a,b to the innermost
    ///     proven element type. When `elem_ct` is not a proven `Array<…>` the
    ///     nested level carries no hint (binding keeps its prior kind — same as
    ///     before; a later `let x: <concrete> = a` meets the let-annotation
    ///     Unknown-accept guard).
    ///   - anything else (Object / Rest / …) — not stamped (owned by its own
    ///     path).
    /// NO fabrication, NO `int`/`number` unify.
    fn stamp_destructure_element_binding(
        &mut self,
        pat: &shape_ast::ast::DestructurePattern,
        elem_ct: &shape_value::v2::ConcreteType,
        is_global: bool,
    ) {
        use shape_ast::ast::DestructurePattern;
        use shape_value::v2::ConcreteType;
        match pat {
            DestructurePattern::Identifier(name, _) => {
                let Some(elem_type) = Self::destructure_element_type_name(elem_ct) else {
                    return;
                };
                if is_global {
                    if let Some(slot) = self.module_bindings.get(name).copied() {
                        self.set_module_binding_type_info(slot, &elem_type);
                    }
                } else if let Some(local_idx) = self.resolve_local(name) {
                    self.set_local_type_info(local_idx, &elem_type);
                }
            }
            DestructurePattern::Array(inner_pats) => {
                // Nested array element — peel one `Array<…>` layer and stamp
                // each inner sub-pattern with the inner element type. Only
                // when `elem_ct` proves a concrete `Array<inner>`.
                let ConcreteType::Array(inner) = elem_ct else {
                    return;
                };
                for inner_pat in inner_pats {
                    self.stamp_destructure_element_binding(inner_pat, inner, is_global);
                }
            }
            _ => {}
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
            DestructurePattern::Identifier(name, _) => {
                // Simple case - store in local
                let local_idx = self.declare_local(name)?;
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

                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let idx_const = self.program.add_constant(Constant::Number(index as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx_const)),
                    ));
                    self.emit(Instruction::simple(OpCode::GetProp));
                    // strict-flip S1 (nested array-destructure element-kind,
                    // 2026-06-22): the parent owns ALL element-kind stamping for
                    // this array (via `stamp_destructure_element_binding`, which
                    // has its own peeling recursion). Suppress the pending
                    // element type across the bytecode recursion so a nested
                    // `compile_destructure_pattern` does NOT stamp inner leaves
                    // with the un-peeled OUTER element type.
                    let saved_pending = self.pending_array_destructure_element_type.take();
                    self.compile_destructure_pattern(pat)?;
                    self.pending_array_destructure_element_type = saved_pending;
                    // Stamp the freshly-bound element sub-pattern with the
                    // array's PROVEN element type (set at the VariableDecl site
                    // from `concrete_type_for_expr(init)`). Recurses into nested
                    // `Array` sub-patterns, peeling one `Array<…>` layer per
                    // level so `let [[a,b],..]` stamps a,b to the innermost
                    // proven element type. Without this the binding kept an
                    // `unknown` kind and a later `let bad: int = a` (a: number)
                    // was silently accepted (HOLE-1). NO fabrication: only
                    // stamps when the receiver resolved to a concrete `Array<T>`.
                    if let Some(elem_ct) = self.pending_array_destructure_element_type.clone() {
                        self.stamp_destructure_element_binding(pat, &elem_ct, false);
                    }
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
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit_object_rest(&rest_excluded, object_schema)?;
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
                    resolved_bindings.push((binding.name.clone(), fields, schema_id));
                }
                let source_schema_id = self
                    .resolve_decomposition_source_schema(
                        &resolved_bindings
                            .iter()
                            .map(|(_, fields, schema_id)| (fields.clone(), *schema_id))
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

                for (binding_name, fields, schema_id) in resolved_bindings {
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
            DestructurePattern::Identifier(name, _) => {
                let binding_idx = self.get_or_create_module_binding(name);
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

                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let idx_const = self.program.add_constant(Constant::Number(index as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx_const)),
                    ));
                    self.emit(Instruction::simple(OpCode::GetProp));
                    // strict-flip S1 (nested array-destructure element-kind,
                    // 2026-06-22): module-scope twin of the local-path
                    // pending-suppression — the parent owns all element-kind
                    // stamping for this array.
                    let saved_pending = self.pending_array_destructure_element_type.take();
                    self.compile_destructure_pattern_global(pat)?;
                    self.pending_array_destructure_element_type = saved_pending;
                    // Stamp the freshly-bound element sub-pattern with the
                    // array's PROVEN element type, recursing into nested
                    // `Array` sub-patterns peeling one `Array<…>` layer per
                    // level.
                    if let Some(elem_ct) = self.pending_array_destructure_element_type.clone() {
                        self.stamp_destructure_element_binding(pat, &elem_ct, true);
                    }
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
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit_object_rest(&rest_excluded, object_schema)?;
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
                    resolved_bindings.push((binding.name.clone(), fields, schema_id));
                }
                let source_schema_id = self
                    .resolve_decomposition_source_schema(
                        &resolved_bindings
                            .iter()
                            .map(|(_, fields, schema_id)| (fields.clone(), *schema_id))
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

                for (binding_name, fields, schema_id) in resolved_bindings {
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

                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    let idx_const = self.program.add_constant(Constant::Number(index as f64));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx_const)),
                    ));
                    self.emit(Instruction::simple(OpCode::GetProp));
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
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(value_local)),
                    ));
                    self.emit_object_rest(&rest_excluded, object_schema)?;
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
