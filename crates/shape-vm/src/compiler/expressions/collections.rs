//! Collection expression compilation (arrays, objects)

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::type_tracking::{NumericType, VariableTypeInfo};
use shape_ast::ast::{EnumConstructorPayload, Expr, Literal, Spanned, TypeAnnotation, TypeParam};
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
            Literal::None => Some(FieldType::Any), // None must be NaN-boxed, correct as-is
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
    if let Some(default_type) = &param.default_type {
        return Some(default_type.clone());
    }
    None
}

fn type_annotations_equivalent(left: &TypeAnnotation, right: &TypeAnnotation) -> bool {
    if left == right {
        return true;
    }
    matches!(
        (left, right),
        (TypeAnnotation::Basic(a), TypeAnnotation::Reference(b))
            | (TypeAnnotation::Reference(a), TypeAnnotation::Basic(b))
            if a == b
    )
}

fn type_annotation_to_compact_string(annotation: &TypeAnnotation) -> String {
    match annotation {
        TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => name.clone(),
        TypeAnnotation::Array(inner) => {
            format!("Vec<{}>", type_annotation_to_compact_string(inner))
        }
        TypeAnnotation::Optional(inner) => {
            format!("Option<{}>", type_annotation_to_compact_string(inner))
        }
        TypeAnnotation::Generic { name, args } => {
            if args.is_empty() {
                name.clone()
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
    /// Compile an array expression
    pub(super) fn compile_expr_array(&mut self, elements: &[Expr]) -> Result<()> {
        // Reject references in array literals — refs are scoped borrows
        // that cannot be stored in collections (would escape scope).
        for elem in elements {
            if let Expr::Reference { span, .. } = elem {
                return Err(ShapeError::SemanticError {
                    message: "cannot store a reference in an array — references are scoped borrows that cannot escape into collections. Use owned values instead".to_string(),
                    location: Some(self.span_to_source_location(*span)),
                });
            }
        }
        let literal_numeric = infer_array_literal_numeric_type(elements);
        let is_bool = is_homogeneous_bool_array(elements);
        if elements.iter().any(|elem| matches!(elem, Expr::Spread(..))) {
            self.compile_array_with_spread(elements)?;
        } else {
            for elem in elements {
                self.compile_expr_as_value_or_placeholder(elem)?;
            }
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
        // with future property assignments (optimistic hoisting pre-pass).
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

        // Build typed field list by inferring types from expressions
        let typed_fields: Vec<(&str, FieldType)> = entries
            .iter()
            .filter_map(|e| match e {
                ObjectEntry::Field { key, value, .. } => {
                    let ft = infer_field_type_from_expr(value).unwrap_or(FieldType::Any);
                    Some((key.as_str(), ft))
                }
                ObjectEntry::Spread(_) => None,
            })
            .chain(hoisted.iter().map(|h| (h.as_str(), FieldType::Any)))
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
                    self.compile_expr_as_value_or_placeholder(value)?;
                    pending_field_names.push(key.clone());
                }
                ObjectEntry::Spread(spread_expr) => {
                    // Create TypedObject from pending fields before the spread
                    if !pending_field_names.is_empty() || !has_initial_object {
                        let field_refs: Vec<&str> =
                            pending_field_names.iter().map(|s| s.as_str()).collect();
                        let schema_id =
                            self.type_tracker.register_inline_object_schema(&field_refs);
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
            let field_refs: Vec<&str> = pending_field_names.iter().map(|s| s.as_str()).collect();
            let schema_id = self.type_tracker.register_inline_object_schema(&field_refs);
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
            let schema_id = self.type_tracker.register_inline_object_schema(&[]);
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
    fn resolve_struct_runtime_type_name(
        &self,
        type_name: &str,
        fields: &[(String, Expr)],
    ) -> Option<String> {
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

            match expected_ann {
                TypeAnnotation::Basic(param_name) | TypeAnnotation::Reference(param_name)
                    if info.type_params.iter().any(|tp| tp.name == *param_name) =>
                {
                    inferred_args
                        .entry(param_name.clone())
                        .or_insert(inferred_ann);
                }
                _ => {}
            }
        }

        let mut resolved_args = Vec::with_capacity(info.type_params.len());
        for tp in &info.type_params {
            if let Some(inferred) = inferred_args.get(&tp.name) {
                resolved_args.push(inferred.clone());
                continue;
            }
            if let Some(default) = default_type_annotation_for_param(tp) {
                resolved_args.push(default);
                continue;
            }
            return None;
        }

        let all_defaults = info
            .type_params
            .iter()
            .zip(resolved_args.iter())
            .all(|(tp, arg)| {
                default_type_annotation_for_param(tp)
                    .map(|default| type_annotations_equivalent(&default, arg))
                    .unwrap_or(false)
            });
        if all_defaults {
            return None;
        }

        let rendered_args = resolved_args
            .iter()
            .map(type_annotation_to_compact_string)
            .collect::<Vec<_>>();
        Some(format!("{}<{}>", type_name, rendered_args.join(", ")))
    }

    pub(super) fn compile_struct_literal(
        &mut self,
        type_name: &str,
        fields: &[(String, Expr)],
        literal_span: shape_ast::ast::Span,
    ) -> Result<()> {
        let literal_loc = self.span_to_source_location(literal_span);
        // Look up struct type definition, resolving through type aliases if needed
        let struct_info = self.struct_types.get(type_name).cloned().or_else(|| {
            self.type_aliases
                .get(type_name)
                .and_then(|resolved| self.struct_types.get(resolved).cloned())
        });

        match struct_info {
            Some((expected_fields, type_def_span)) => {
                let runtime_type_name = self
                    .resolve_struct_runtime_type_name(type_name, fields)
                    .unwrap_or_else(|| type_name.to_string());

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
                    .map(|info| info.type_params.iter().map(|tp| tp.name.as_str()).collect())
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
                // (with correct FieldTypes), instead of creating a duplicate with FieldType::Any
                let schema_id = if let Some(schema) =
                    self.type_tracker.schema_registry().get(&runtime_type_name)
                {
                    schema.id
                } else if runtime_type_name != type_name {
                    if let Some(base_schema) = self.type_tracker.schema_registry().get(type_name) {
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
                        // Fallback: register if not found (shouldn't happen for valid struct types)
                        let typed_fields: Vec<(&str, FieldType)> = expected_fields
                            .iter()
                            .map(|s| (s.as_str(), FieldType::Any))
                            .collect();
                        self.type_tracker
                            .register_named_object_schema(&runtime_type_name, &typed_fields)
                    }
                } else if let Some(schema) = self.type_tracker.schema_registry().get(type_name) {
                    schema.id
                } else {
                    // Fallback: register if not found (shouldn't happen for valid struct types)
                    let typed_fields: Vec<(&str, FieldType)> = expected_fields
                        .iter()
                        .map(|s| (s.as_str(), FieldType::Any))
                        .collect();
                    self.type_tracker
                        .register_named_object_schema(&runtime_type_name, &typed_fields)
                };

                // Compile field values in the order defined by the struct (not user order)
                for expected_name in &expected_fields {
                    let (_, value) = fields
                        .iter()
                        .find(|(name, _)| name == expected_name)
                        .expect("field existence validated above");
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
        // Look up enum schema - must be registered
        let schema = self
            .type_tracker
            .schema_registry()
            .get(enum_name)
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
                    self.compile_expr_as_value_or_placeholder(value)?;
                }
                values.len() as u16
            }
            EnumConstructorPayload::Struct(fields) => {
                // For struct payloads, we only push the values (not keys)
                // The schema knows the field order
                for (_key, value) in fields {
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
                    TypeAnnotation::Reference(t) | TypeAnnotation::Basic(t) => t.clone(),
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
                    message: "table row literal `[...], [...]` requires a `Table<T>` type annotation".to_string(),
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
                    message: format!("unknown type '{}' in Table<{}>", inner_type_name, inner_type_name),
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
                self.compile_expr_as_value_or_placeholder(elem)?;
            }
        }

        // Call MakeTableFromRows builtin
        // Convention: push arg_count as constant, then BuiltinCall
        let total_args = 3 + row_count * field_count;
        let ac_const = self.program.add_constant(Constant::Number(total_args as f64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(ac_const)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::MakeTableFromRows)),
        ));

        self.last_expr_schema = None;
        self.last_expr_type_info = Some(super::super::VariableTypeInfo::named(
            format!("Table<{}>", inner_type_name),
        ));
        self.last_expr_numeric_type = None;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use shape_ast::parser::parse_program;

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
}
