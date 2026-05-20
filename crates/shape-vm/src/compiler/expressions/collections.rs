//! Collection expression compilation (arrays, objects)

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::type_tracking::{NumericType, VariableTypeInfo};
use shape_ast::ast::{
    EnumConstructorPayload, Expr, Literal, Span, Spanned, TypeAnnotation, TypeParam,
};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::{FieldType, TypeSchema};

/// Infer the FieldType of a compile-time expression (literals only).
/// Returns None if the type can't be determined statically (skip check).
fn infer_field_type_from_expr(expr: &Expr) -> Option<FieldType> {
    match expr {
        Expr::Literal(lit, _) => match lit {
            Literal::Int(_) => Some(FieldType::I64),
            Literal::Number(_) => Some(FieldType::F64),
            Literal::Decimal(_) => Some(FieldType::Decimal),
            Literal::Bool(_) => Some(FieldType::Bool),
            Literal::String(_) => Some(FieldType::String),
            // W17.2-B (audit §4.D.9 subsumption per §4.D.1 PROPAGATE
            // rebuild + supervisor ratify 2026-05-19): `None` literal IS
            // by definition an unresolved-element-type carrier (the
            // schema can't know what `T` is at the literal site without
            // bidirectional inference). Project to
            // `FieldType::Option(Box<FieldType::Any>)` — the outer
            // discriminator is concrete (Option), and bidirectional
            // narrowing at the context site refines the inner Any
            // when the surrounding expression's expected type is known.
            // The post_inference_verify pass surfaces the inner Any
            // if it persists past the narrowing window.
            Literal::None => Some(FieldType::Option(Box::new(FieldType::Any))),
            _ => None,
        },
        _ => None,
    }
}

fn infer_array_literal_numeric_type(elements: &[Expr]) -> Option<NumericType> {
    let mut acc: Option<NumericType> = None;
    for elem in elements {
        let elem_ty = match elem {
            Expr::Literal(Literal::Int(_), _) => Some(NumericType::Int),
            Expr::Literal(Literal::Number(_), _) => Some(NumericType::Number),
            Expr::Literal(Literal::Decimal(_), _) => Some(NumericType::Decimal),
            _ => None,
        };
        let elem_ty = elem_ty?;
        if let Some(prev) = acc {
            if prev != elem_ty {
                return None;
            }
        } else {
            acc = Some(elem_ty);
        }
    }
    acc
}

/// Detect if all elements in the array are bool literals.
fn is_homogeneous_bool_array(elements: &[Expr]) -> bool {
    !elements.is_empty()
        && elements
            .iter()
            .all(|e| matches!(e, Expr::Literal(Literal::Bool(_), _)))
}

fn field_type_to_type_annotation(field_type: FieldType) -> Option<TypeAnnotation> {
    match field_type {
        FieldType::I64 => Some(TypeAnnotation::Basic("int".to_string())),
        FieldType::F64 => Some(TypeAnnotation::Basic("number".to_string())),
        FieldType::Decimal => Some(TypeAnnotation::Basic("decimal".to_string())),
        FieldType::Bool => Some(TypeAnnotation::Basic("bool".to_string())),
        FieldType::String => Some(TypeAnnotation::Basic("string".to_string())),
        _ => None,
    }
}

fn default_type_annotation_for_param(param: &TypeParam) -> Option<TypeAnnotation> {
    // `default_type()` returns `None` for `TypeParam::Const` — a const generic
    // has a default *expression*, not a default *type annotation*.
    // B.3/B.4 resolves const defaults at monomorphization; don't attempt to
    // infer a type-level default from a const param here.
    param.default_type().cloned()
}

/// v0.3 Phase 4b Round 5c-2-β-β (d) jit-generic-ctor-default-param-vm-sigsegv
/// (ADR-006 §2.7.5 producer-side stamp + §2.7.24 typed-carrier monomorphization).
///
/// Substitute generic type-parameter references inside a base struct schema's
/// `FieldType` with the concrete `FieldType` resolved at the monomorphization
/// site. `type Box<T> { value: T }` registers its base schema field `value`
/// as `FieldType::Object("T")` (the parser emits `TypeAnnotation::Basic("T")`
/// for a bare type-parameter name, and `type_annotation_to_field_type` maps any
/// non-primitive name to `Object(name)`). When the literal `Box { value: 9 }`
/// monomorphizes to `Box<int>`, the specialized schema must carry `value: I64`
/// — NOT the unsound `Object("T")` residue. Leaving `Object("T")` makes the
/// downstream `MakeFieldRef` stamp `FIELD_TAG_OBJECT` on a slot holding an
/// inline scalar; the VM's `clone_with_kind` then dereferences the raw scalar
/// bits as a `*const TypedObjectStorage` (misaligned-pointer SIGSEGV at
/// `executor/vm_impl/stack.rs`).
///
/// `substitution` maps type-parameter name → resolved concrete `TypeAnnotation`
/// (from `resolve_struct_runtime_type_name`). Substitution recurses through
/// `Array` / `Option` field types so a `value: Array<T>` or `value: T?` field
/// is monomorphized correctly. `FieldType` variants that cannot carry a
/// type-parameter reference are returned unchanged.
fn substitute_type_param_field_type(
    ft: &FieldType,
    substitution: &std::collections::HashMap<String, TypeAnnotation>,
) -> FieldType {
    match ft {
        FieldType::Object(name) => match substitution.get(name) {
            // A type-parameter reference resolved to a concrete annotation —
            // re-lower it through the canonical annotation→FieldType mapper.
            Some(ann) => BytecodeCompiler::type_annotation_to_field_type(ann),
            // `Object(name)` where `name` is NOT a type parameter — a genuine
            // nested-struct reference. Leave it unchanged.
            None => ft.clone(),
        },
        FieldType::Array(inner) => FieldType::Array(Box::new(
            substitute_type_param_field_type(inner, substitution),
        )),
        FieldType::Option(inner) => FieldType::Option(Box::new(
            substitute_type_param_field_type(inner, substitution),
        )),
        // Primitive / non-parametric field types carry no type-parameter
        // reference; return unchanged.
        _ => ft.clone(),
    }
}

fn type_annotation_to_compact_string(annotation: &TypeAnnotation) -> String {
    match annotation {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(name) => name.to_string(),
        TypeAnnotation::Array(inner) => {
            format!("Vec<{}>", type_annotation_to_compact_string(inner))
        }
        TypeAnnotation::Generic { name, args } => {
            if args.is_empty() {
                name.to_string()
            } else {
                let rendered = args
                    .iter()
                    .map(type_annotation_to_compact_string)
                    .collect::<Vec<_>>();
                format!("{}<{}>", name, rendered.join(", "))
            }
        }
        TypeAnnotation::Union(variants) => variants
            .iter()
            .map(type_annotation_to_compact_string)
            .collect::<Vec<_>>()
            .join(" | "),
        _ => "unknown".to_string(),
    }
}

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Reject reference storage in collections/aggregates for **top-level code only**.
    /// Inside function bodies the MIR solver detects these via `array_store_loans`,
    /// `object_store_loans`, and `enum_store_loans` facts, so we defer to it.
    pub(super) fn reject_direct_reference_storage(
        &self,
        expr: &Expr,
        message: &'static str,
    ) -> Result<()> {
        if let Expr::Reference { span, .. } = expr {
            // Inside a function body, MIR handles this — only reject at top level.
            if self.current_function.is_some() {
                return Ok(());
            }
            return Err(ShapeError::SemanticError {
                message: message.to_string(),
                location: Some(self.span_to_source_location(*span)),
            });
        }
        Ok(())
    }

    /// Compile an array expression
    pub(super) fn compile_expr_array(&mut self, elements: &[Expr], span: Span) -> Result<()> {
        use super::super::v2_array_emission::infer_array_element_type;
        use super::super::v2_typed_emission::{
            should_use_typed_array_from_slot_kind, TypedArrayKind,
        };

        // Inside function bodies the MIR solver handles ref-in-collection;
        // at top level reject_direct_reference_storage still fires.
        const ARRAY_REF_STORAGE_ERROR: &str = "cannot store a reference in an array — references are scoped borrows that cannot escape into collections. Use owned values instead";
        for elem in elements {
            self.reject_direct_reference_storage(elem, ARRAY_REF_STORAGE_ERROR)?;
        }
        let literal_numeric = infer_array_literal_numeric_type(elements);
        let is_bool = is_homogeneous_bool_array(elements);

        // v2 Phase 3.1 (Agent 1 + Agent 3): typed-array fast path.
        //
        // Resolve a homogeneous element type from the literal (or from
        // tracked variable types) and pick a typed-array kind. When that
        // succeeds, lower the literal to v2 typed-array allocation
        // (`NewTypedArrayF64/I64/I32/Bool`) followed by per-element
        // `TypedArrayPush*`. Falls through to the legacy v1 path
        // (`NewArray`) for spreads, heterogeneous literals, empty
        // literals without annotation, and element types that don't map
        // to a typed kind.
        //
        // Order of preference:
        //   1. Explicit `let arr: Array<T> = [...]` annotation
        //      (`pending_variable_typed_array_kind`).
        //   2. Inferred element type from the literal itself
        //      (`infer_array_element_type`) — handles the bare-literal
        //      case `let x = [1, 2, 3]` without an annotation.
        //
        // When the inferred path picks a typed kind we ALSO set
        // `pending_variable_typed_array_kind` so the post-init capture
        // in `Statement::VarDecl` records the typed kind against the
        // local slot / module binding (Phase 3.1 Agent 3 wiring).
        // R5.4B: detect nested-array literal shape upfront. When any
        // element is itself an array literal, the outer array CANNOT use
        // the typed fast path — `NewTypedArrayF64/I64/I32/Bool` store
        // scalars, and splicing inner typed-array pointers in as f64
        // bits produces a value that can't be decoded downstream (see
        // `intrinsic_matmul_mat`'s `as_any_array()` failure). Also, the
        // inner arrays themselves must be forced off the typed path so
        // they round-trip as heap-ref pointers through the outer
        // generic `NewArray`; `nested_array_literal_depth` propagates
        // that signal into the recursive `compile_expr_array` call.
        let has_nested_array_elem = elements
            .iter()
            .any(|e| matches!(e, Expr::Array(..)));
        let in_nested_context = self.nested_array_literal_depth > 0;
        let typed_kind: Option<TypedArrayKind> = if elements
            .iter()
            .any(|e| matches!(e, Expr::Spread(..)))
            || has_nested_array_elem
            || in_nested_context
        {
            None
        } else if let Some(kind) = self.pending_variable_typed_array_kind {
            // The enclosing `let arr: Array<T> = [...]` already proved
            // the element type via annotation; trust it.
            Some(kind)
        } else if let Some(slot_kind) =
            infer_array_element_type(elements, &self.type_tracker)
        {
            // Bare literal with a homogeneous, statically-resolvable
            // element type. Pick a typed kind if we have one and signal
            // it back to the binding code path.
            let inferred = should_use_typed_array_from_slot_kind(slot_kind);
            if inferred.is_some() {
                self.pending_variable_typed_array_kind = inferred;
            }
            inferred
        } else if self.array_elements_all_typed_object(elements) {
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18) —
            // function-call-result-element case (`let boxes = [aabb(...), aabb(...)]`
            // where `aabb` returns a registered struct). The producer-side proof is
            // the function's declared return type tracked in
            // `type_tracker.function_return_types`; per ADR-006 §2.7.5
            // stamp-at-compile-time the kind is statically known here without
            // runtime inspection. Routes the literal to the v2-raw
            // `TypedArray<*const TypedObjectStorage>` carrier fast path.
            self.pending_variable_typed_array_kind = Some(TypedArrayKind::TypedObject);
            Some(TypedArrayKind::TypedObject)
        } else {
            None
        };

        if let Some(kind) = typed_kind {
            // LANG-9 fix (Phase 4b round 2, 2026-05-18): record the
            // proven element `ConcreteType` against this array literal's
            // AST span so subsequent `try_monomorphize_method_call` on an
            // inline receiver (`[1,2,3].map(|x| x*2)`) can reach the
            // typed-array specialization. Pre-fix, `concrete_type_for_expr`
            // hit the `Expr::Array` arm at
            // `monomorphization/type_resolution.rs:1381`, looked up
            // `array_element_types[span]`, found nothing, returned `None`,
            // and `try_monomorphize_method_call` fell back to the generic
            // `Vec.map` (entry_point=0 stub). The bound form
            // (`let xs = [...]; xs.map(...)`) succeeded because
            // `identifier_concrete_type` reads from
            // `local_array_element_types`/`type_tracker`, which are
            // populated by the binding propagation path. Per ADR-006
            // §2.7.5 stamp-at-compile-time, the producer-side `typed_kind`
            // IS the proof of element type — record it now so the
            // bytecode-time monomorphizer can consume it. No
            // Bool-default, no inference fabrication: the typed-kind
            // branch only fires when `infer_array_literal_numeric_type` /
            // `infer_array_element_type` / `pending_variable_typed_array_kind`
            // already proved the element type at the producer site.
            self.record_array_element_type(
                span,
                super::super::v2_typed_emission::concrete_type_for_typed_array_kind(kind),
            );
            // Allocate the typed array with capacity = element count.
            self.emit(Instruction::new(
                kind.new_opcode(),
                Some(Operand::Count(elements.len() as u16)),
            ));
            // Stack: [arr]
            // For each element: [arr] -> [arr, arr] -> [arr, arr, val]
            //                   -> [arr] (TypedArrayPush* pops arr+val).
            //
            // Wave 3 Stabilize Round 1 V3-A2-followup-producer-cascade
            // (2026-05-15): for String/Decimal element kinds, the element
            // must be produced with `NativeKind::StringV2` / `NativeKind::DecimalV2`
            // to round-trip through `TypedArrayPushString` /
            // `TypedArrayPushDecimal`'s strict-kind check (v2_handlers/array.rs:
            // 687/703). The legacy `LoadConst` path produces Arc<String> /
            // Arc<Decimal> with NativeKind::String / NativeKind::Decimal, which
            // the strict-kind check rejects. For string/decimal literal
            // elements, emit `NewStringV2` / `NewDecimalV2` directly (reads
            // from the string / constant pool, allocates a fresh `StringObj`
            // / `DecimalObj` with refcount = 1, push as the v2-raw kind). The
            // caller's share transfers to the array via the `TypedArrayPush*`
            // refcount discipline (per v2_handlers/array.rs:696/762 comment).
            //
            // Non-literal element expressions (`f(x)`, `x` identifier, etc.)
            // are deferred to V3-S5 Round 2 consumer cascade: the call sites
            // that produce string/decimal values (function returns, identifier
            // loads) must themselves migrate to StringV2/DecimalV2 carriers
            // before non-literal Array<string>/Array<decimal> literals can
            // round-trip without surfacing the kind-mismatch RuntimeError.
            // Until then, non-literal elements emit through the legacy path
            // and surface the same structured kind-mismatch RuntimeError at
            // push time that Round 3a' gate-flip introduced — NOT a SIGSEGV.
            for elem in elements {
                self.plan_flexible_binding_escape_from_expr(elem);
                self.emit(Instruction::simple(OpCode::Dup));
                match (kind, elem) {
                    (
                        super::super::v2_typed_emission::TypedArrayKind::String,
                        Expr::Literal(Literal::String(s), _),
                    ) => {
                        // String literal → v2-raw StringObj with NativeKind::StringV2.
                        let str_id = self.program.add_string(s.clone());
                        self.emit(Instruction::new(
                            OpCode::NewStringV2,
                            Some(Operand::Property(str_id)),
                        ));
                    }
                    (
                        super::super::v2_typed_emission::TypedArrayKind::Decimal,
                        Expr::Literal(Literal::Decimal(d), _),
                    ) => {
                        // Decimal literal → v2-raw DecimalObj with NativeKind::DecimalV2.
                        let const_idx = self.program.add_constant(Constant::Decimal(*d));
                        self.emit(Instruction::new(
                            OpCode::NewDecimalV2,
                            Some(Operand::Const(const_idx)),
                        ));
                    }
                    _ => {
                        // Legacy element path (numeric / bool scalar kinds OR
                        // non-literal string/decimal expressions). For
                        // string/decimal non-literals this still produces the
                        // legacy Arc-wrapped carrier; the typed array push
                        // handler will surface a structured RuntimeError at
                        // runtime per the Round 3a' gate-flip note. Consumer-
                        // side migration is V3-S5 Round 2 territory.
                        self.compile_expr_as_value_or_placeholder(elem)?;
                    }
                }
                self.emit(Instruction::simple(kind.push_opcode()));
            }
        } else if elements.iter().any(|elem| matches!(elem, Expr::Spread(..))) {
            self.compile_array_with_spread(elements)?;
        } else {
            // R5.4B: while compiling elements of a generic-array literal,
            // mark inner array-literal children as nested so they also
            // refuse the typed fast path (see comment above).
            self.nested_array_literal_depth += 1;
            for elem in elements {
                self.plan_flexible_binding_escape_from_expr(elem);
                // Phase F: closure literals stored into an array escape
                // per `docs/v2-closure-specialization.md` §2.1 row 2.
                // Force heap-ABI emission so the JIT (and Phase H cleanup)
                // can rely on the signal.
                if matches!(elem, Expr::FunctionExpr { .. }) {
                    self.emit_make_closure_heap_next = true;
                }
                self.compile_expr_as_value_or_placeholder(elem)?;
            }
            self.nested_array_literal_depth -= 1;
            // Emit NewTypedArray for homogeneous int/number/bool literals
            let use_typed = !elements.is_empty()
                && (matches!(
                    literal_numeric,
                    Some(NumericType::Int | NumericType::Number)
                ) || is_bool);
            if use_typed {
                self.emit(Instruction::new(
                    OpCode::NewTypedArray,
                    Some(Operand::Count(elements.len() as u16)),
                ));
            } else {
                self.emit(Instruction::new(
                    OpCode::NewArray,
                    Some(Operand::Count(elements.len() as u16)),
                ));
            }
        }
        // Arrays don't produce TypedObjects
        self.last_expr_schema = None;
        self.last_expr_type_info = if is_bool {
            Some(VariableTypeInfo::named("Vec<bool>".to_string()))
        } else {
            literal_numeric.map(|nt| {
                let type_name = match nt {
                    NumericType::Int | NumericType::IntWidth(_) => "Vec<int>",
                    NumericType::Number => "Vec<number>",
                    NumericType::Decimal => "Vec<decimal>",
                };
                VariableTypeInfo::named(type_name.to_string())
            })
        };
        // LANG-9 fix (legacy path): the spread / nested-array / heterogeneous
        // fall-through above still produces a homogeneous-numeric receiver
        // when `literal_numeric` is `Some` or `is_bool` (`NewTypedArray`
        // emission). Record the element type at the same producer site so
        // the inline `[...].method(...)` monomorphizer can reach
        // `array_element_types[span]` from this branch too. Idempotent with
        // the typed-kind branch's `record_array_element_type` above (which
        // covers the v2 typed-array fast path) — both lower into the same
        // map keyed by span.
        let legacy_elem: Option<shape_value::v2::ConcreteType> = if is_bool {
            Some(shape_value::v2::ConcreteType::Bool)
        } else {
            literal_numeric.and_then(|nt| match nt {
                NumericType::Int => Some(shape_value::v2::ConcreteType::I64),
                NumericType::Number => Some(shape_value::v2::ConcreteType::F64),
                NumericType::Decimal => Some(shape_value::v2::ConcreteType::Decimal),
                NumericType::IntWidth(_) => None,
            })
        };
        if let Some(elem_ct) = legacy_elem {
            self.record_array_element_type(span, elem_ct);
        }
        self.last_expr_numeric_type = None;
        Ok(())
    }

    /// Compile an object expression
    ///
    /// ALL object literals produce TypedObject with O(1) field access.
    /// The compiler registers an inline schema for every object literal —
    /// field names are always known at compile time.
    /// Spread objects use the dynamic path (temporary — Phase 1b).
    pub(super) fn compile_expr_object(
        &mut self,
        entries: &[shape_ast::ast::ObjectEntry],
    ) -> Result<()> {
        use shape_ast::ast::ObjectEntry;
        // Inside function bodies the MIR solver handles ref-in-object;
        // at top level reject_direct_reference_storage still fires.
        const OBJECT_REF_STORAGE_ERROR: &str = "cannot store a reference in an object or struct literal — references are scoped borrows that cannot escape into aggregate values. Use owned values instead";
        for entry in entries {
            match entry {
                ObjectEntry::Field { value, .. } => {
                    self.reject_direct_reference_storage(value, OBJECT_REF_STORAGE_ERROR)?;
                }
                ObjectEntry::Spread(expr) => {
                    self.reject_direct_reference_storage(expr, OBJECT_REF_STORAGE_ERROR)?;
                }
            }
        }

        let has_spreads = entries.iter().any(|e| matches!(e, ObjectEntry::Spread(_)));

        if !has_spreads {
            // ALL non-spread objects use TypedObject — field names known at compile time
            self.compile_typed_object_literal(entries)
        } else {
            // Spread objects: field set not fully known at compile time (Phase 1b)
            self.compile_dynamic_object(entries)
        }
    }

    /// Compile an object literal as a TypedObject
    ///
    /// ALL non-spread objects use this path for O(1) field access via compile-time schemas.
    /// Hoisted fields (from future property assignments like `a.y = 2`) are included in the
    /// schema from the start — their slots are initialized to None.
    fn compile_typed_object_literal(
        &mut self,
        entries: &[shape_ast::ast::ObjectEntry],
    ) -> Result<()> {
        use shape_ast::ast::ObjectEntry;

        // Collect explicit field names from the object literal
        let explicit_fields: Vec<&str> = entries
            .iter()
            .filter_map(|e| match e {
                ObjectEntry::Field { key, .. } => Some(key.as_str()),
                ObjectEntry::Spread(_) => None,
            })
            .collect();

        // Include hoisted fields if this object is being assigned to a variable
        // with future property assignments (Phase 1: AST pre-pass hoisting).
        let hoisted: Vec<String> = self
            .pending_variable_name
            .as_ref()
            .and_then(|var| self.hoisted_fields.get(var))
            .map(|fields| {
                fields
                    .iter()
                    .filter(|f| !explicit_fields.contains(&f.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        // MIR field analysis integration note:
        // Phase 2 (MIR) can identify dead hoisted fields — fields that were
        // included in the schema by the AST pre-pass but are never actually
        // read within the function. To prune these, the compiler would need to
        // map `mir_field_analyses[func].dead_fields` (which uses `(SlotId,
        // FieldIdx)`) back to field names via the schema registry. This mapping
        // is not available during object construction because the schema is
        // being *created* here. A future optimization can perform a post-MIR
        // schema compaction pass that shrinks schemas after all field accesses
        // are known.

        // Build typed field list by inferring types from expressions.
        // Phase 3e: hoisted fields use the inferred FieldType from the
        // pre-pass when the assigned RHS is a literal — otherwise default
        // to Any. Looking up by var name through pending_variable_name
        // keeps the legacy un-named code path behavior identical.
        let hoisted_type_lookup = self
            .pending_variable_name
            .as_ref()
            .and_then(|var| self.hoisted_field_types.get(var))
            .cloned()
            .unwrap_or_default();
        // v0.3 Phase 4b Round 5b W17.2-C (audit §4.D.3 PROPAGATE):
        // `infer_field_type_from_expr` only resolves compile-time literals.
        // Non-literal expressions (function calls, complex expressions,
        // unresolved variables) cannot project a concrete `FieldType` at
        // this site — full inference for the entire RHS would require
        // running the type system over the expression tree, which is not
        // in scope at the inline-object construction call site.
        //
        // PROPAGATE: fall back to `FieldType::Any` and let the
        // post_inference_verify pass at
        // `crates/shape-vm/src/compiler/post_inference_verify.rs` absorb
        // via the `__inline_obj_*` transitional whitelist row (W17.2-C
        // narrowed-row). The `register_inline_object_schema_typed` call
        // at line 526 below auto-generates the `__inline_obj_N` schema
        // name, which is recognized by the verification pass's prefix
        // rule. Per audit §5 + §9.B.3 supervisor ratify 2026-05-19 +
        // ADR-006 §2.7.5 producer-side stamp (the schema-side Any is
        // bounded by the verification-pass-side absorber).
        let typed_fields: Vec<(&str, FieldType)> = entries
            .iter()
            .filter_map(|e| match e {
                ObjectEntry::Field { key, value, .. } => {
                    let ft = infer_field_type_from_expr(value).unwrap_or(FieldType::Any);
                    Some((key.as_str(), ft))
                }
                ObjectEntry::Spread(_) => None,
            })
            .chain(hoisted.iter().map(|h| {
                // Hoisted-field type lookup: the AST pre-pass at
                // `compiler/mod.rs::hoisted_field_types` populates inferred
                // types when the assigned RHS is a literal. Non-literal
                // RHS falls back to `FieldType::Any` here — same
                // verification-pass absorber per the §4.D.3 disposition
                // above (the inline-object schema name `__inline_obj_N`
                // routes through the transitional prefix rule).
                let ft = hoisted_type_lookup
                    .get(h.as_str())
                    .cloned()
                    .unwrap_or(FieldType::Any);
                (h.as_str(), ft)
            }))
            .collect();

        // Register inline schema with ALL fields (explicit + hoisted), with inferred types
        let schema_id = self
            .type_tracker
            .register_inline_object_schema_typed(&typed_fields);

        // Build combined field list for NewTypedObject field_count
        let all_field_names: Vec<&str> = typed_fields.iter().map(|(n, _)| *n).collect();

        // Compile each explicit field value (in order)
        for entry in entries {
            if let ObjectEntry::Field { value, .. } = entry {
                self.plan_flexible_binding_escape_from_expr(value);
                self.compile_expr_as_value_or_placeholder(value)?;
            }
        }

        // Push None for each hoisted field (allocated but uninitialized)
        for _ in &hoisted {
            self.emit(Instruction::simple(OpCode::PushNull));
        }

        // Emit NewTypedObject with the full field count (explicit + hoisted)
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_id as u16,
                field_count: all_field_names.len() as u16,
            }),
        ));

        // Track result schema for typed merge optimization
        self.last_expr_schema = Some(schema_id);

        Ok(())
    }

    /// Compile an object with spread operators (dynamic path)
    ///
    /// Each group of consecutive fields gets a compile-time schema (NewTypedObject).
    /// Spreads merge via MergeObject (Phase 4.2 handles TypedObject+TypedObject).
    fn compile_dynamic_object(&mut self, entries: &[shape_ast::ast::ObjectEntry]) -> Result<()> {
        use shape_ast::ast::ObjectEntry;

        let mut pending_field_names: Vec<String> = Vec::new();
        let mut has_initial_object = false;
        let mut current_schema: Option<shape_runtime::type_schema::SchemaId> = None;

        for entry in entries {
            match entry {
                ObjectEntry::Field { key, value, .. } => {
                    // Push ONLY the value (keys are embedded in the schema)
                    self.plan_flexible_binding_escape_from_expr(value);
                    self.compile_expr_as_value_or_placeholder(value)?;
                    pending_field_names.push(key.clone());
                }
                ObjectEntry::Spread(spread_expr) => {
                    // Create TypedObject from pending fields before the spread
                    if !pending_field_names.is_empty() || !has_initial_object {
                        // W17.2-C §4.D.5 migration: pending spread-fields
                        // have no per-field type info at this dynamic
                        // construction site; route through typed-with-Any.
                        let typed_fields: Vec<(&str, FieldType)> = pending_field_names
                            .iter()
                            .map(|s| (s.as_str(), FieldType::Any))
                            .collect();
                        let schema_id = self
                            .type_tracker
                            .register_inline_object_schema_typed(&typed_fields);
                        self.emit(Instruction::new(
                            OpCode::NewTypedObject,
                            Some(Operand::TypedObjectAlloc {
                                schema_id: schema_id as u16,
                                field_count: pending_field_names.len() as u16,
                            }),
                        ));
                        if let Some(base_schema) = current_schema {
                            let merged_schema =
                                self.register_object_merge_schema(base_schema, schema_id)?;
                            self.emit(Instruction::new(OpCode::MergeObject, None));
                            current_schema = Some(merged_schema);
                            self.last_expr_schema = Some(merged_schema);
                        } else {
                            current_schema = Some(schema_id);
                            self.last_expr_schema = Some(schema_id);
                        }
                        pending_field_names.clear();
                        has_initial_object = true;
                    }

                    // Compile the spread expression (should evaluate to an object)
                    self.plan_flexible_binding_escape_from_expr(spread_expr);
                    self.compile_expr(spread_expr)?;
                    let spread_schema = self.last_expr_schema.take();
                    let Some(base_schema) = current_schema else {
                        return Err(ShapeError::SemanticError {
                            message: "Object spread requires a compile-time known object schema"
                                .to_string(),
                            location: Some(self.span_to_source_location(spread_expr.span())),
                        });
                    };
                    let Some(right_schema) = spread_schema else {
                        return Err(ShapeError::SemanticError {
                            message: "Object spread source must have a compile-time known schema"
                                .to_string(),
                            location: Some(self.span_to_source_location(spread_expr.span())),
                        });
                    };
                    let merged_schema =
                        self.register_object_merge_schema(base_schema, right_schema)?;

                    // Merge the spread object into the current object
                    self.emit(Instruction::new(OpCode::MergeObject, None));
                    current_schema = Some(merged_schema);
                    self.last_expr_schema = Some(merged_schema);
                }
            }
        }

        // Finalize remaining fields
        if !pending_field_names.is_empty() {
            // W17.2-C §4.D.5 migration: pending-fields finalize site, no
            // per-field type info available; typed-with-Any + verification.
            let typed_fields: Vec<(&str, FieldType)> = pending_field_names
                .iter()
                .map(|s| (s.as_str(), FieldType::Any))
                .collect();
            let schema_id = self
                .type_tracker
                .register_inline_object_schema_typed(&typed_fields);
            self.emit(Instruction::new(
                OpCode::NewTypedObject,
                Some(Operand::TypedObjectAlloc {
                    schema_id: schema_id as u16,
                    field_count: pending_field_names.len() as u16,
                }),
            ));
            if has_initial_object {
                let Some(base_schema) = current_schema else {
                    return Err(ShapeError::SemanticError {
                        message: "Object spread requires a compile-time known object schema"
                            .to_string(),
                        location: None,
                    });
                };
                let merged_schema = self.register_object_merge_schema(base_schema, schema_id)?;
                self.emit(Instruction::new(OpCode::MergeObject, None));
                current_schema = Some(merged_schema);
                self.last_expr_schema = Some(merged_schema);
            } else {
                current_schema = Some(schema_id);
                self.last_expr_schema = Some(schema_id);
            }
        } else if !has_initial_object {
            // Empty object
            // W17.2-C §4.D.5 migration: empty-fields case uses typed variant.
            let schema_id = self.type_tracker.register_inline_object_schema_typed(&[]);
            self.emit(Instruction::new(
                OpCode::NewTypedObject,
                Some(Operand::TypedObjectAlloc {
                    schema_id: schema_id as u16,
                    field_count: 0,
                }),
            ));
            current_schema = Some(schema_id);
            self.last_expr_schema = Some(schema_id);
        }

        if self.last_expr_schema.is_none() {
            self.last_expr_schema = current_schema;
        }

        Ok(())
    }

    fn register_object_merge_schema(
        &mut self,
        left_schema_id: shape_runtime::type_schema::SchemaId,
        right_schema_id: shape_runtime::type_schema::SchemaId,
    ) -> Result<shape_runtime::type_schema::SchemaId> {
        let schema_name = format!("__merged_{}_{}", left_schema_id, right_schema_id);
        if let Some(existing) = self.type_tracker.schema_registry().get(&schema_name) {
            return Ok(existing.id);
        }

        let (left_fields, right_fields) = {
            let registry = self.type_tracker.schema_registry();
            let left =
                registry
                    .get_by_id(left_schema_id)
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: format!("Unknown left schema ID: {}", left_schema_id),
                        location: None,
                    })?;
            let right =
                registry
                    .get_by_id(right_schema_id)
                    .ok_or_else(|| ShapeError::RuntimeError {
                        message: format!("Unknown right schema ID: {}", right_schema_id),
                        location: None,
                    })?;
            (left.fields.clone(), right.fields.clone())
        };

        let right_names: std::collections::HashSet<&str> =
            right_fields.iter().map(|f| f.name.as_str()).collect();
        let mut merged_fields: Vec<(String, shape_runtime::type_schema::FieldType)> =
            Vec::with_capacity(left_fields.len() + right_fields.len());

        for f in &left_fields {
            if !right_names.contains(f.name.as_str()) {
                merged_fields.push((f.name.clone(), f.field_type.clone()));
            }
        }
        for f in &right_fields {
            merged_fields.push((f.name.clone(), f.field_type.clone()));
        }

        Ok(self
            .type_tracker
            .schema_registry_mut()
            .register_type(schema_name, merged_fields))
    }

    /// Compile a struct literal: TypeName { field: value, ... }
    ///
    /// For user types (Point, Candle): creates a TypedObject with field validation.
    ///
    /// v0.3 Phase 4b Round 5c-2-β-β (d) jit-generic-ctor-default-param-vm-sigsegv
    /// (ADR-006 §2.7.5 producer-side stamp + §2.7.24 typed-carrier
    /// monomorphization): returns the monomorphized runtime type name AND the
    /// per-type-param resolved `TypeAnnotation` substitution map. The map is
    /// REQUIRED by the caller — without it, the specialized `Box<int>` schema
    /// (and even the bare `Box` schema in the all-defaults case) carries field
    /// types `Object("T")` for type-parameter fields, and a downstream
    /// `MakeFieldRef`/`DerefLoad` stamps `FIELD_TAG_OBJECT` on a slot that
    /// actually holds an inline scalar. The VM's `clone_with_kind` then
    /// dereferences the raw scalar bits as a `TypedObjectStorage` pointer
    /// (misaligned-pointer SIGSEGV at `vm_impl/stack.rs`).
    ///
    /// The pre-fix `all_defaults` early-`None` was an optimization to avoid
    /// registering a redundant `Box<int>` schema when every type param resolves
    /// to its declared default — but that "redundancy" was exactly the bug: the
    /// bare `Box` schema is structurally unsound because its type-parameter
    /// fields were never substituted. The early-return is removed; a generic
    /// struct literal ALWAYS resolves to a monomorphized name + substitution
    /// map when all params are resolvable.
    fn resolve_struct_runtime_type_name(
        &self,
        type_name: &str,
        fields: &[(String, Expr)],
    ) -> Option<(String, std::collections::HashMap<String, TypeAnnotation>)> {
        let info = self.struct_generic_info.get(type_name)?;
        if info.type_params.is_empty() {
            return None;
        }

        let mut inferred_args: std::collections::HashMap<String, TypeAnnotation> =
            std::collections::HashMap::new();

        for (field_name, value_expr) in fields {
            let Some(expected_ann) = info.runtime_field_types.get(field_name) else {
                continue;
            };
            let Some(inferred_field_type) = infer_field_type_from_expr(value_expr) else {
                continue;
            };
            let Some(inferred_ann) = field_type_to_type_annotation(inferred_field_type) else {
                continue;
            };

            if let Some(param_name) = expected_ann.as_type_name_str() {
                if info.type_params.iter().any(|tp| tp.name() == param_name) {
                    inferred_args
                        .entry(param_name.to_string())
                        .or_insert(inferred_ann);
                }
            }
        }

        let mut resolved_args = Vec::with_capacity(info.type_params.len());
        let mut substitution: std::collections::HashMap<String, TypeAnnotation> =
            std::collections::HashMap::new();
        for tp in &info.type_params {
            // TODO(B.3): const generics fall through here but have no type-
            // level inference story yet. The `None` return below bails out of
            // inference, which is the right conservative stub until B.3 lands.
            if let Some(inferred) = inferred_args.get(tp.name()) {
                resolved_args.push(inferred.clone());
                substitution.insert(tp.name().to_string(), inferred.clone());
                continue;
            }
            if let Some(default) = default_type_annotation_for_param(tp) {
                resolved_args.push(default.clone());
                substitution.insert(tp.name().to_string(), default);
                continue;
            }
            return None;
        }

        let rendered_args = resolved_args
            .iter()
            .map(type_annotation_to_compact_string)
            .collect::<Vec<_>>();
        Some((
            format!("{}<{}>", type_name, rendered_args.join(", ")),
            substitution,
        ))
    }

    pub(super) fn compile_struct_literal(
        &mut self,
        type_name: &str,
        fields: &[(String, Expr)],
        literal_span: shape_ast::ast::Span,
    ) -> Result<()> {
        // Inside function bodies the MIR solver handles ref-in-struct;
        // at top level reject_direct_reference_storage still fires.
        const OBJECT_REF_STORAGE_ERROR: &str = "cannot store a reference in an object or struct literal — references are scoped borrows that cannot escape into aggregate values. Use owned values instead";
        for (_, value) in fields {
            self.reject_direct_reference_storage(value, OBJECT_REF_STORAGE_ERROR)?;
        }
        let literal_loc = self.span_to_source_location(literal_span);
        // Resolve through module scope for qualified type lookups
        let type_name = &self.resolve_type_name(type_name);
        // Look up struct type definition, resolving through type aliases if needed
        let struct_info = self.struct_types.get(type_name.as_str()).cloned().or_else(|| {
            self.type_aliases
                .get(type_name.as_str())
                .and_then(|resolved| self.struct_types.get(resolved).cloned())
        });

        match struct_info {
            Some((expected_fields, type_def_span)) => {
                // v0.3 Phase 4b Round 5c-2-β-β (d) jit-generic-ctor-default-
                // param-vm-sigsegv (ADR-006 §2.7.5 producer-side stamp +
                // §2.7.24 typed-carrier monomorphization): a generic struct
                // literal resolves to a monomorphized runtime name plus a
                // per-type-param substitution map. The substitution map is
                // applied below when the specialized schema is registered, so
                // type-parameter fields (`value: T`) carry the concrete
                // `FieldType` (`I64`) instead of the unsound `Object("T")`
                // residue that segfaults `clone_with_kind` at field-read time.
                let (runtime_type_name, type_param_substitution) = self
                    .resolve_struct_runtime_type_name(type_name, fields)
                    .map(|(name, subst)| (name, Some(subst)))
                    .unwrap_or_else(|| (type_name.to_string(), None));

                // Validate fields match the struct definition
                // Check for missing fields
                for expected in &expected_fields {
                    if !fields.iter().any(|(name, _)| name == expected) {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Missing field '{}' in {} struct literal",
                                expected, type_name
                            ),
                            location: Some(
                                literal_loc.clone().with_hint(format!(
                                    "add `{}` to this struct literal",
                                    expected
                                )),
                            ),
                        });
                    }
                }
                // Check for unknown fields (including comptime fields which can't be set at runtime)
                for (name, _) in fields {
                    if !expected_fields.contains(name) {
                        // Check if this is a comptime field — give a specific error
                        if self
                            .comptime_fields
                            .get(type_name)
                            .map_or(false, |m| m.contains_key(name))
                        {
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "Cannot set comptime field '{}' in {} struct literal — it is a compile-time constant",
                                    name, type_name
                                ),
                                location: Some(literal_loc.clone()),
                            });
                        }
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Unknown field '{}' in {} struct literal",
                                name, type_name
                            ),
                            location: Some(literal_loc.clone()),
                        });
                    }
                }

                // Type-check field values against schema
                // Collect generic type parameter names so we can skip validation
                // for fields whose declared type is a type parameter (e.g. `x: T`).
                let generic_param_names: std::collections::HashSet<&str> = self
                    .struct_generic_info
                    .get(type_name)
                    .map(|info| info.type_params.iter().map(|tp| tp.name()).collect())
                    .unwrap_or_default();
                if let Some(schema) = self.type_tracker.schema_registry().get(type_name) {
                    for (field_name, value_expr) in fields {
                        if let Some(inferred) = infer_field_type_from_expr(value_expr) {
                            if let Some(field_def) =
                                schema.fields.iter().find(|f| f.name == *field_name)
                            {
                                // Skip check for generic type parameters (stored as Object("T"))
                                if let shape_runtime::type_schema::FieldType::Object(ref obj_name) =
                                    field_def.field_type
                                {
                                    if generic_param_names.contains(obj_name.as_str()) {
                                        continue;
                                    }
                                }
                                if !field_def.field_type.is_compatible_with(&inferred) {
                                    let value_loc = self.span_to_source_location(value_expr.span());
                                    let mut loc = value_loc;
                                    loc.hints.push(format!(
                                        "expected `{}`, found `{}`",
                                        field_def.field_type, inferred
                                    ));
                                    loc.notes.push(shape_ast::error::ErrorNote {
                                        message: format!(
                                            "field `{}` declared as `{}` here",
                                            field_name, field_def.field_type
                                        ),
                                        location: Some(self.span_to_source_location(type_def_span)),
                                    });
                                    return Err(ShapeError::SemanticError {
                                        message: format!(
                                            "type mismatch: field `{}` of `{}` expects `{}`, found `{}`",
                                            field_name, type_name, field_def.field_type, inferred
                                        ),
                                        location: Some(loc),
                                    });
                                }
                            }
                        }
                    }
                }

                // Look up the schema that was already registered during type definition compilation
                // (with correct FieldTypes), instead of creating a duplicate with FieldType::Any.
                //
                // W15.2-LANG-8 jit-toplevel-render fix (Phase 4b Round 3 Surface-1c, ADR-006 §2.7.5
                // producer-side stamp): the previous fallback at the third `else` branch created a
                // schema with every field typed `FieldType::Any` when neither the resolved
                // `runtime_type_name` nor `type_name` had a registered schema. For a type alias
                // `type P = Point` followed by `let origin = P { x: 0, y: 0 }`, only `Point` is
                // registered — looking up `P` missed, falling through to the all-`Any` fallback.
                // Subsequent `origin.x` access then emitted `MakeFieldRef` with `FIELD_TAG_ANY`,
                // which the VM SURFACEs at runtime per ADR-006 §2.7.13 / Q14 — the producer must
                // stamp a concrete tag. Resolve through `type_aliases` so the alias inherits the
                // base type's concrete FieldTypes.
                let alias_target = self.type_aliases.get(type_name.as_str()).cloned();
                let schema_id = if let Some(schema) =
                    self.type_tracker.schema_registry().get(&runtime_type_name)
                {
                    schema.id
                } else if runtime_type_name != *type_name {
                    if let Some(base_schema) = self.type_tracker.schema_registry().get(type_name) {
                        // v0.3 Phase 4b Round 5c-2-β-β (d) jit-generic-ctor-
                        // default-param-vm-sigsegv (ADR-006 §2.7.5 producer-
                        // side stamp + §2.7.24 typed-carrier monomorphization):
                        // when the base type is generic, its type-parameter
                        // fields (`value: T` → base `FieldType::Object("T")`)
                        // MUST be substituted with the concrete `FieldType`
                        // resolved at the monomorphization site. Copying the
                        // base `Object("T")` verbatim is the SIGSEGV root
                        // cause — `MakeFieldRef` stamps `FIELD_TAG_OBJECT` on
                        // a slot holding an inline scalar and the VM's
                        // `clone_with_kind` dereferences the scalar bits as a
                        // `*const TypedObjectStorage`. `type_param_substitution`
                        // is `Some` exactly when `runtime_type_name` is a
                        // generic monomorphization (`Box<int>`).
                        let fields = base_schema
                            .fields
                            .iter()
                            .map(|f| {
                                let ft = match &type_param_substitution {
                                    Some(subst) => substitute_type_param_field_type(
                                        &f.field_type,
                                        subst,
                                    ),
                                    None => f.field_type.clone(),
                                };
                                (f.name.clone(), ft)
                            })
                            .collect::<Vec<_>>();
                        let schema = TypeSchema::new(runtime_type_name.clone(), fields);
                        let schema_id = schema.id;
                        self.type_tracker.schema_registry_mut().register(schema);
                        schema_id
                    } else if let Some(alias_base) = alias_target.as_deref()
                        && let Some(base_schema) =
                            self.type_tracker.schema_registry().get(alias_base)
                    {
                        // Type-alias indirection: `runtime_type_name` may differ from
                        // `*type_name`, but the alias resolves to a base type whose schema
                        // is registered with concrete field types.
                        let fields = base_schema
                            .fields
                            .iter()
                            .map(|f| (f.name.clone(), f.field_type.clone()))
                            .collect::<Vec<_>>();
                        let schema = TypeSchema::new(runtime_type_name.clone(), fields);
                        let schema_id = schema.id;
                        self.type_tracker.schema_registry_mut().register(schema);
                        schema_id
                    } else {
                        // v0.3 Phase 4b Round 5b W17.2-C (audit §4.D.4 ERROR
                        // disposition): the deleted `FieldType::Any` fallback
                        // here covered the "shouldn't happen for valid struct
                        // types" residual where `runtime_type_name` differs
                        // from `*type_name`, neither base-resolution path
                        // resolves a registered schema, and the type-alias
                        // chain also misses. Per audit §4.D.4: schema-lookup
                        // failure at a non-alias non-base struct literal is a
                        // user-facing soundness issue; surface the structured
                        // diagnostic instead of registering an all-`Any`
                        // schema that would route through MakeFieldRef with
                        // FIELD_TAG_ANY at downstream property access (the
                        // §2.7.13/Q14 surface). Per ADR-006 §2.7.5 producer-
                        // side stamp + §4.D.2 same-pattern discipline.
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Cannot resolve schema for struct literal: \
                                 `{}` (runtime name `{}`) has no registered \
                                 TypeSchema. Per audit §4.D.4 (W17.2-C close \
                                 commit + ADR-006 §2.7.5 producer-side stamp): \
                                 struct-literal field types must be statically \
                                 known at the literal site. If `{}` is a type \
                                 alias, ensure its base type is defined; if \
                                 it's a generic instantiation, ensure all \
                                 type parameters are concrete.",
                                type_name, runtime_type_name, type_name
                            ),
                            location: Some(literal_loc.clone()),
                        });
                    }
                } else if let Some(schema) = self.type_tracker.schema_registry().get(type_name) {
                    schema.id
                } else if let Some(alias_base) = alias_target.as_deref()
                    && let Some(base_schema) =
                        self.type_tracker.schema_registry().get(alias_base)
                {
                    // Type-alias indirection at the `runtime_type_name == type_name` branch:
                    // `let origin = P { ... }` where `type P = Point` — `runtime_type_name`
                    // and `type_name` are both `"P"`, but only `"Point"`'s schema is
                    // registered. Inherit its FieldTypes under the alias's name so
                    // downstream property access stamps a concrete tag (ADR-006 §2.7.5).
                    let fields = base_schema
                        .fields
                        .iter()
                        .map(|f| (f.name.clone(), f.field_type.clone()))
                        .collect::<Vec<_>>();
                    let schema = TypeSchema::new(runtime_type_name.clone(), fields);
                    let schema_id = schema.id;
                    self.type_tracker.schema_registry_mut().register(schema);
                    schema_id
                } else {
                    // v0.3 Phase 4b Round 5b W17.2-C (audit §4.D.4 ERROR
                    // disposition): parallel-fallback to the :971-980 branch,
                    // covering the `runtime_type_name == type_name` case
                    // where neither direct lookup nor type-alias resolution
                    // succeeds. Same structured diagnostic; same audit cite.
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Cannot resolve schema for struct literal: \
                             `{}` has no registered TypeSchema. Per audit \
                             §4.D.4 (W17.2-C close commit + ADR-006 §2.7.5 \
                             producer-side stamp): struct-literal field types \
                             must be statically known at the literal site. \
                             Define `{}` with `type {} {{ ... }}` syntax or \
                             check for typos in the type name.",
                            type_name, type_name, type_name
                        ),
                        location: Some(literal_loc.clone()),
                    });
                };

                // Compile field values in the order defined by the struct (not user order)
                for expected_name in &expected_fields {
                    let (_, value) = fields
                        .iter()
                        .find(|(name, _)| name == expected_name)
                        .expect("field existence validated above");
                    self.plan_flexible_binding_escape_from_expr(value);
                    self.compile_expr_as_value_or_placeholder(value)?;
                }

                // Emit NewTypedObject — no WrapTypeAnnotation needed,
                // `.type()` uses schema_id → type_name lookup instead.
                self.emit(Instruction::new(
                    OpCode::NewTypedObject,
                    Some(Operand::TypedObjectAlloc {
                        schema_id: schema_id as u16,
                        field_count: expected_fields.len() as u16,
                    }),
                ));

                self.last_expr_schema = Some(schema_id);
                self.last_expr_numeric_type = None;
                self.last_expr_type_info = Some(crate::type_tracking::VariableTypeInfo::known(
                    schema_id,
                    runtime_type_name.clone(),
                ));
                Ok(())
            }
            None => Err(ShapeError::SemanticError {
                message: format!("Unknown struct type '{}'", type_name),
                location: None,
            }),
        }
    }

    /// Compile an enum constructor into a TypedObject
    ///
    /// All enums must be registered in TypeSchemaRegistry at compile time.
    /// Layout:
    /// - Field 0: variant_id (as Int/i64 discriminator)
    /// - Field 1+: payload values (for tuple: values in order, for struct: values only)
    pub(super) fn compile_expr_enum_constructor(
        &mut self,
        enum_name: &str,
        variant: &str,
        payload: &EnumConstructorPayload,
    ) -> Result<()> {
        const ENUM_REF_STORAGE_ERROR: &str = "cannot store a reference in an enum payload — references are scoped borrows that cannot escape into aggregate values. Use owned values instead";
        // Resolve through module scope for qualified enum lookups
        let enum_name = &self.resolve_type_name(enum_name);

        // Check if this is actually a qualified struct literal: `mod::Type { fields }`
        // The grammar parses `mod::Type { ... }` as EnumConstructor(enum=mod, variant=Type, payload=Struct)
        // If `enum_name::variant` resolves to a known struct type, reinterpret as struct literal.
        if let EnumConstructorPayload::Struct(fields) = payload {
            let qualified_struct_name = format!("{}::{}", enum_name, variant);
            let resolved = self.resolve_type_name(&qualified_struct_name);
            if self.struct_types.contains_key(resolved.as_str())
                || self.type_aliases.contains_key(resolved.as_str())
            {
                let fields_as_exprs: Vec<(String, Expr)> =
                    fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                return self.compile_struct_literal(
                    &resolved,
                    &fields_as_exprs,
                    shape_ast::ast::Span::default(),
                );
            }
        }
        // Also handle unit-payload case: `mod::Type` where Type is a struct with no fields
        // (but this is unusual, most struct types have fields)

        // Look up enum schema - must be registered
        let schema = self
            .type_tracker
            .schema_registry()
            .get(enum_name.as_str())
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!("Unknown enum type: {}", enum_name),
                location: None,
            })?;

        let enum_info = schema
            .get_enum_info()
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!("Type '{}' is not an enum", enum_name),
                location: None,
            })?;

        let variant_info =
            enum_info
                .variant_by_name(variant)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!("Unknown variant '{}' for enum '{}'", variant, enum_name),
                    location: None,
                })?;

        let schema_id = schema.id;
        let variant_id = variant_info.id;

        // Push variant_id as first field (stored as i64 in __variant).
        let variant_const = self.program.add_constant(Constant::Int(variant_id as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(variant_const)),
        ));

        // Push payload fields
        let payload_count = match payload {
            EnumConstructorPayload::Unit => 0u16,
            EnumConstructorPayload::Tuple(values) => {
                for value in values {
                    self.reject_direct_reference_storage(value, ENUM_REF_STORAGE_ERROR)?;
                    self.plan_flexible_binding_escape_from_expr(value);
                    self.compile_expr_as_value_or_placeholder(value)?;
                }
                values.len() as u16
            }
            EnumConstructorPayload::Struct(fields) => {
                // For struct payloads, we only push the values (not keys)
                // The schema knows the field order
                for (_key, value) in fields {
                    self.reject_direct_reference_storage(value, ENUM_REF_STORAGE_ERROR)?;
                    self.plan_flexible_binding_escape_from_expr(value);
                    self.compile_expr_as_value_or_placeholder(value)?;
                }
                fields.len() as u16
            }
        };

        // Emit NewTypedObject: allocates TypedObject and stores fields
        // field_count = 1 (variant_id) + payload_count
        let field_count = 1 + payload_count;
        self.emit(Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_id as u16,
                field_count,
            }),
        ));

        // The result is a TypedObject, not a numeric value.
        // Without this, the last payload sub-expression's numeric type leaks
        // (e.g. `Status::Ok(1)` would leave NumericType::Int from the `1`),
        // causing typed opcodes like EqInt to be emitted for enum comparisons.
        self.last_expr_schema = Some(schema_id);
        self.last_expr_numeric_type = None;

        Ok(())
    }

    /// Compile a table row literal: `[a, b, c], [d, e, f]`
    ///
    /// Requires a `Table<T>` type annotation to resolve the struct type T.
    /// Each row's positional elements are mapped to T's fields in declaration order.
    /// Emits: push schema_id, row_count, field_count, then all field values row-major,
    /// then CallBuiltin MakeTableFromRows.
    pub(crate) fn compile_table_rows(
        &mut self,
        rows: &[Vec<shape_ast::ast::Expr>],
        type_annotation: &Option<shape_ast::ast::TypeAnnotation>,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        use crate::bytecode::BuiltinFunction;
        use shape_ast::ast::TypeAnnotation;

        // Extract Table<T> annotation → inner type name
        let inner_type_name = match type_annotation {
            Some(TypeAnnotation::Generic { name, args }) if name == "Table" && args.len() == 1 => {
                match &args[0] {
                    TypeAnnotation::Basic(t) => t.clone(),
                    TypeAnnotation::Reference(t) => t.to_string(),
                    _ => {
                        return Err(ShapeError::SemanticError {
                            message: "Table row literal requires a concrete type parameter, e.g. Table<MyType>".to_string(),
                            location: Some(self.span_to_source_location(span)),
                        });
                    }
                }
            }
            _ => {
                return Err(ShapeError::SemanticError {
                    message:
                        "table row literal `[...], [...]` requires a `Table<T>` type annotation"
                            .to_string(),
                    location: Some(self.span_to_source_location(span)),
                });
            }
        };

        // Look up the struct type to get field names and schema
        let struct_info = self.struct_types.get(&inner_type_name).cloned();
        let (field_names, _type_def_span) = match struct_info {
            Some(info) => info,
            None => {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "unknown type '{}' in Table<{}>",
                        inner_type_name, inner_type_name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
        };

        let field_count = field_names.len();

        // Validate row widths
        for (i, row) in rows.iter().enumerate() {
            if row.len() != field_count {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "row {} has {} values but type '{}' has {} fields ({})",
                        i + 1,
                        row.len(),
                        inner_type_name,
                        field_count,
                        field_names.join(", ")
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
        }

        // Look up schema ID
        let schema_id = self
            .type_tracker
            .schema_registry()
            .get(&inner_type_name)
            .map(|s| s.id)
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!("no schema registered for type '{}'", inner_type_name),
                location: Some(self.span_to_source_location(span)),
            })?;

        let row_count = rows.len();

        // Emit args: schema_id, row_count, field_count (as constants)
        let sid_const = self.program.add_constant(Constant::Int(schema_id as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(sid_const)),
        ));
        let rc_const = self.program.add_constant(Constant::Int(row_count as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(rc_const)),
        ));
        let fc_const = self.program.add_constant(Constant::Int(field_count as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(fc_const)),
        ));

        // Emit all field values in row-major order
        for row in rows {
            for elem in row {
                self.plan_flexible_binding_escape_from_expr(elem);
                self.compile_expr_as_value_or_placeholder(elem)?;
            }
        }

        // Call MakeTableFromRows builtin
        // Convention: push arg_count as constant, then BuiltinCall.
        // The count MUST be an integer constant — `pop_builtin_args`
        // reads it via `int_operand` (the W17-make-closure arg-count emit
        // migration); a `Number` constant produces a `Float64` slot kind
        // that `int_operand` rejects.
        let total_args = 3 + row_count * field_count;
        let ac_const = self
            .program
            .add_constant(Constant::Int(total_args as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(ac_const)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::MakeTableFromRows)),
        ));

        self.last_expr_schema = None;
        self.last_expr_type_info = Some(super::super::VariableTypeInfo::named(format!(
            "Table<{}>",
            inner_type_name
        )));
        self.last_expr_numeric_type = None;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use shape_ast::parser::parse_program;
    use shape_runtime::type_schema::FieldType;

    #[test]
    fn test_struct_literal_type_mismatch_decimal_for_int() {
        let code = r#"
            type T { i: int }
            let x = T { i: 10.2D }
        "#;
        let program = parse_program(code).unwrap();
        let result = BytecodeCompiler::new().compile_with_source(&program, code);
        assert!(
            result.is_err(),
            "Decimal assigned to int field should error"
        );
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("type mismatch"),
            "Error should mention type mismatch: {}",
            err
        );
        assert!(
            err.contains("int"),
            "Error should mention expected type 'int': {}",
            err
        );
        assert!(
            err.contains("decimal"),
            "Error should mention found type 'decimal': {}",
            err
        );
    }

    #[test]
    fn test_struct_literal_type_mismatch_string_for_int() {
        let code = r#"
            type T { i: int }
            let x = T { i: "hello" }
        "#;
        let program = parse_program(code).unwrap();
        let result = BytecodeCompiler::new().compile_with_source(&program, code);
        assert!(result.is_err(), "String assigned to int field should error");
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("type mismatch"),
            "Error should mention type mismatch: {}",
            err
        );
    }

    #[test]
    fn test_struct_literal_type_mismatch_int_for_string() {
        let code = r#"
            type T { name: string }
            let x = T { name: 42 }
        "#;
        let program = parse_program(code).unwrap();
        let result = BytecodeCompiler::new().compile_with_source(&program, code);
        assert!(result.is_err(), "Int assigned to string field should error");
        let err = format!("{:?}", result.unwrap_err());
        assert!(
            err.contains("type mismatch"),
            "Error should mention type mismatch: {}",
            err
        );
    }

    #[test]
    fn test_struct_literal_matching_types_ok() {
        let code = r#"
            type T { i: int }
            let x = T { i: 10 }
        "#;
        let program = parse_program(code).unwrap();
        let result = BytecodeCompiler::new().compile_with_source(&program, code);
        assert!(
            result.is_ok(),
            "Int assigned to int field should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_struct_literal_int_widens_to_number() {
        let code = r#"
            type Point { x: number, y: number }
            let p = Point { x: 1, y: 2 }
        "#;
        let program = parse_program(code).unwrap();
        let result = BytecodeCompiler::new().compile_with_source(&program, code);
        assert!(
            result.is_ok(),
            "Int assigned to number field should compile (widening): {:?}",
            result.err()
        );
    }

    #[test]
    fn test_struct_literal_error_message_quality() {
        let code = r#"
            type MyType { i: int }
            let b = MyType { i: 10.2D }
        "#;
        let program = parse_program(code).unwrap();
        let result = BytecodeCompiler::new().compile_with_source(&program, code);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("type mismatch"),
            "Should contain 'type mismatch': {}",
            msg
        );
        assert!(msg.contains("MyType"), "Should mention type name: {}", msg);
        assert!(msg.contains("int"), "Should mention expected type: {}", msg);
        assert!(
            msg.contains("decimal"),
            "Should mention found type: {}",
            msg
        );

        // Check that format_with_source produces rich output
        let formatted = err.format_with_source();
        assert!(
            formatted.contains("E0100"),
            "Should use E0100 error code: {}",
            formatted
        );
    }

    // W15.2-LANG-8 jit-toplevel-render fix regression tests (Phase 4b Round 3
    // Surface-1c, ADR-006 §2.7.5 producer-side stamp).
    //
    // Pre-fix, these programs surfaced `MakeFieldRef SURFACE: field_type_tag 8
    // (FIELD_TAG_ANY / FIELD_TAG_UNKNOWN)` from `variables/mod.rs:2501` because
    // the producer-side emitter emitted `Operand::TypedField { field_type_tag:
    // FIELD_TAG_ANY }` for fields whose `FieldType` was `Any` (nested object
    // literals or unresolved type-alias schemas). Per ADR-006 §2.7.13 / Q14 the
    // producer must stamp a concrete tag — the post-fix path skips the
    // MakeFieldRef fast path for `FieldType::Any` fields and resolves type-alias
    // schemas to their base type's concrete FieldTypes.
    //
    // Pin: VM execution must complete without surfacing the MakeFieldRef SURFACE
    // marker for each of the three book reproducer shapes
    // (`fundamentals/objects-arrays.mdx:113` nested-object-host,
    //  `fundamentals/variables.mdx:207` type-alias-constructor,
    //  plus the equivalent inner-object-literal shape that the audit's §6.8
    //  table conflated under the `jit-toplevel-render` label).

    use crate::test_utils::eval_result;

    /// Reproducer 3 (`objects-arrays.mdx:113`): nested object literal whose
    /// outer field's inferred `FieldType` is `Any`. Pre-fix, accessing
    /// `cfg.server` via the MakeRef + MakeFieldRef + DerefLoad fast path
    /// SURFACEd at runtime. Post-fix, falls through to `GetFieldTyped` which
    /// sources the kind from the storage's parallel `field_kinds` track.
    #[test]
    fn test_w15_2_lang_8_nested_object_literal_host_field_access() {
        // The print result is irrelevant; we only assert execution completes
        // without the `MakeFieldRef SURFACE` marker from variables/mod.rs:2501.
        let code = r#"
            let cfg = {
              server: {
                host: "localhost",
                port: 9091
              }
            }
            print(cfg.server.host)
        "#;
        let result = eval_result(code);
        assert!(
            result.is_ok(),
            "nested object literal field access must not SURFACE: got {:?}",
            result.err()
        );
    }

    /// Reproducer 4 (`variables.mdx:207`): type-alias `type P = Point` used as
    /// a constructor `P { x: 0, y: 0 }`. Pre-fix, the struct-literal compiler
    /// at `collections.rs:933-940` fell through to a `FieldType::Any` fallback
    /// schema because only `Point` (not `P`) was registered. Subsequent
    /// `origin.x` access then emitted MakeFieldRef with FIELD_TAG_ANY. Post-fix,
    /// the alias resolves to `Point`'s schema and inherits its concrete
    /// FieldTypes (`x: I64`), so MakeFieldRef carries FIELD_TAG_I64 and the
    /// kind is statically sourceable.
    #[test]
    fn test_w15_2_lang_8_type_alias_constructor_field_access() {
        let code = r#"
            type Point { x: int, y: int }
            type P = Point

            let origin = P { x: 0, y: 0 }
            print(origin.x)
        "#;
        let result = eval_result(code);
        assert!(
            result.is_ok(),
            "type-alias constructor + field access must not SURFACE: got {:?}",
            result.err()
        );
    }

    /// Inner-object-literal field access alone (the inner shape of rep3 that
    /// also occurs on its own in many `objects-arrays.mdx` paragraphs). Pre-fix
    /// `obj.field` on an object literal whose field is itself an object
    /// emitted a MakeFieldRef carrying FIELD_TAG_ANY. Post-fix, that fast path
    /// is gated off for `FieldType::Any` and the GetFieldTyped fallback runs.
    #[test]
    fn test_w15_2_lang_8_object_literal_any_field_via_get_field_typed() {
        let code = r#"
            let host = { server: { name: "x" } }
            let s = host.server
            print(s)
        "#;
        let result = eval_result(code);
        assert!(
            result.is_ok(),
            "object literal with Any-typed inner field must not SURFACE: got {:?}",
            result.err()
        );
    }

    /// Type-alias field access returns the concrete int value through the
    /// GetFieldTyped path with a concrete FIELD_TAG_I64 stamp. The variable
    /// name `pt` (not `origin`) avoids a `or`-keyword tokenization quirk in
    /// the grammar that's unrelated to this fix.
    #[test]
    fn test_w15_2_lang_8_type_alias_constructor_field_typed_value() {
        let code = r#"
            type Point { x: int, y: int }
            type P = Point
            let pt = P { x: 42, y: 0 }
            pt.x
        "#;
        let result = eval_result(code).expect("should not SURFACE");
        assert_eq!(
            result.as_i64(),
            Some(42),
            "type-alias field access must return the concrete int value"
        );
    }

    // ───────────────────────────────────────────────────────────────────
    // v0.3 Phase 4b Round 5c-2-β-β (d) jit-generic-ctor-default-param-vm-
    // sigsegv regression tests (ADR-006 §2.7.5 producer-side stamp +
    // §2.7.24 typed-carrier monomorphization).
    //
    // Pre-fix, `type Box<T> { value: T }` registered the base `Box` schema
    // with field `value` typed `FieldType::Object("T")` (the parser emits
    // `TypeAnnotation::Basic("T")` for a bare type-parameter name, and
    // `type_annotation_to_field_type` maps any non-primitive name to
    // `Object(name)`). The struct-literal `Box { value: 9 }` monomorphized
    // to `Box<int>` but COPIED the base field type `Object("T")` verbatim
    // into the specialized schema. Reading `b.value` then emitted a
    // `MakeFieldRef` stamping `FIELD_TAG_OBJECT` on a slot that actually
    // held an inline `i64` — and the VM's `clone_with_kind`
    // (`executor/vm_impl/stack.rs`) dereferenced the raw scalar bits as a
    // `*const TypedObjectStorage` (misaligned-pointer SIGSEGV, exit 139).
    // The JIT path produced the correct value (inverse-direction
    // divergence). Per supervisor disposition 2026-05-20 this is a pure
    // soundness bug: the VM gets the JIT-correct behavior.
    //
    // The fix substitutes type-parameter fields with the concrete
    // `FieldType` resolved at the monomorphization site
    // (`substitute_type_param_field_type`), so the specialized `Box<int>`
    // schema carries `value: I64` and `MakeFieldRef` stamps
    // `FIELD_TAG_I64`. The pre-fix `all_defaults` early-`None` was removed
    // — a generic struct literal always resolves to a monomorphized name +
    // substitution map.
    //
    // These regression tests pin the DETERMINISTIC producer-side invariant:
    // the monomorphized schema carries a concrete (non-`Object("<param>")`)
    // `FieldType` for every type-parameter field. The pre-fix bug was a
    // 100%-deterministic SIGSEGV; pinning the producer-side schema stamp
    // catches any regression of the `Object("T")` residue without depending
    // on the runtime path (which carries a SEPARATE, pre-existing,
    // VM-wide TypedObject-reference double-free — `RefTarget::TypedField`
    // holds the receiver as `Arc<TypedObjectStorage>` but the runtime
    // carrier is v2-raw `_new`-allocated, so `resolve_typed_object_receiver`
    // at `executor/variables/mod.rs` runs `Arc::increment_strong_count` /
    // `Arc::from_raw` against the wrong allocator layout; surfaced at close,
    // empirically reproduces ~6% on a non-generic `type Box { value: int }`
    // on baseline `6b6b50d8`, distinct root-cause family, NOT in (d) scope).

    /// Plain generic struct (no default type param). Pre-fix: the
    /// monomorphized `Box<int>` schema carried `value: Object("T")` → VM
    /// SIGSEGV on `b.value`. Post-fix: the schema carries the concrete
    /// `FieldType::I64`.
    #[test]
    fn test_r5c2bb_d_generic_struct_no_default_concrete_field_type() {
        let code = r#"
            type Box<T> { value: T }
            let b = Box { value: 9 }
            b.value
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("compile must succeed");
        let schema = bytecode
            .type_schema_registry
            .get("Box<int>")
            .expect("monomorphized `Box<int>` schema must be registered");
        let value_field = schema
            .fields
            .iter()
            .find(|f| f.name == "value")
            .expect("`value` field must exist");
        assert_eq!(
            value_field.field_type,
            FieldType::I64,
            "monomorphized `Box<int>` field `value` must be concrete I64, \
             not the unsound `Object(\"T\")` type-parameter residue"
        );
    }

    /// Generic struct with a DEFAULT type param, instantiated relying on
    /// the default (`Box<T = int>` then `Box { value: 9 }`). Pre-fix: the
    /// `all_defaults` early-return left the bare `Box` schema's
    /// `Object("T")` field in place and the literal resolved to the bare
    /// `Box` schema → VM SIGSEGV. Post-fix: the literal monomorphizes to a
    /// `Box<int>` schema carrying `value: I64`.
    #[test]
    fn test_r5c2bb_d_generic_struct_default_param_concrete_field_type() {
        let code = r#"
            type Box<T = int> { value: T }
            let b = Box { value: 9 }
            b.value
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("compile must succeed");
        // The all-defaults monomorphization must register a `Box<int>`
        // schema (the pre-fix `all_defaults` early-`None` skipped this and
        // left the bare `Box` schema's `Object("T")` field in play).
        let schema = bytecode
            .type_schema_registry
            .get("Box<int>")
            .expect(
                "default-type-param generic literal must monomorphize to a \
                 `Box<int>` schema (pre-fix the `all_defaults` early-return \
                 left the bare `Box` schema's `Object(\"T\")` field in play)",
            );
        let value_field = schema
            .fields
            .iter()
            .find(|f| f.name == "value")
            .expect("`value` field must exist");
        assert_eq!(
            value_field.field_type,
            FieldType::I64,
            "default-type-param monomorphized field `value` must be concrete I64"
        );
    }

    /// Generic struct whose field resolves to a non-numeric concrete type
    /// (`Box<T>` instantiated with a string). Verifies the substitution
    /// re-lowers `Object("T")` to `FieldType::String`, not a numeric tag.
    #[test]
    fn test_r5c2bb_d_generic_struct_string_concrete_field_type() {
        let code = r#"
            type Box<T> { value: T }
            let b = Box { value: "hello" }
            b.value
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("compile must succeed");
        let schema = bytecode
            .type_schema_registry
            .get("Box<string>")
            .expect("monomorphized `Box<string>` schema must be registered");
        let value_field = schema
            .fields
            .iter()
            .find(|f| f.name == "value")
            .expect("`value` field must exist");
        assert_eq!(
            value_field.field_type,
            FieldType::String,
            "monomorphized `Box<string>` field `value` must be concrete String"
        );
    }

    /// Multi-type-parameter generic struct with defaults
    /// (`Pair<A = int, B = string>`). Verifies each parameter is
    /// independently substituted into its own field's schema type — the
    /// substitution map is keyed per type-parameter name.
    #[test]
    fn test_r5c2bb_d_generic_struct_multi_param_concrete_field_types() {
        let code = r#"
            type Pair<A = int, B = string> { first: A, second: B }
            let p = Pair { first: 1, second: "two" }
            p.first
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("compile must succeed");
        let schema = bytecode
            .type_schema_registry
            .get("Pair<int, string>")
            .expect("monomorphized `Pair<int, string>` schema must be registered");
        let first = schema
            .fields
            .iter()
            .find(|f| f.name == "first")
            .expect("`first` field must exist");
        let second = schema
            .fields
            .iter()
            .find(|f| f.name == "second")
            .expect("`second` field must exist");
        assert_eq!(
            first.field_type,
            FieldType::I64,
            "type-parameter `A` field must monomorphize to concrete I64"
        );
        assert_eq!(
            second.field_type,
            FieldType::String,
            "type-parameter `B` field must monomorphize to concrete String"
        );
    }

    /// Negative pin: a non-generic struct type has no type parameters, so
    /// `resolve_struct_runtime_type_name` returns `None` and no monomorphized
    /// name is produced — the literal binds directly to the base schema. The
    /// type-param substitution path must NOT fire for non-generic types.
    #[test]
    fn test_r5c2bb_d_non_generic_struct_no_monomorphization() {
        let code = r#"
            type Plain { value: int }
            let p = Plain { value: 9 }
            p.value
        "#;
        let program = parse_program(code).unwrap();
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("compile must succeed");
        // No `Plain<...>` monomorphized schema — only the base `Plain`.
        assert!(
            bytecode.type_schema_registry.get("Plain<int>").is_none(),
            "non-generic struct must not produce a monomorphized `Plain<int>` schema"
        );
        let schema = bytecode
            .type_schema_registry
            .get("Plain")
            .expect("base `Plain` schema must be registered");
        let value_field = schema
            .fields
            .iter()
            .find(|f| f.name == "value")
            .expect("`value` field must exist");
        assert_eq!(
            value_field.field_type,
            FieldType::I64,
            "non-generic struct field type must be the declared concrete I64"
        );
    }
}
