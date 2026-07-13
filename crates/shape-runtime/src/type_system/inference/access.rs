//! Access pattern type inference
//!
//! Handles type inference for property access, index access, function calls, and iterators.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{Expr, ObjectTypeField, Span, TypeAnnotation};
use std::collections::HashMap;

impl TypeInferenceEngine {
    fn type_source_var(ty: &Type) -> Option<TypeVar> {
        match ty {
            Type::Variable(var) | Type::Constrained { var, .. } => Some(var.clone()),
            _ => None,
        }
    }

    fn deferred_closure_numeric_source_var(&self, ty: &Type) -> Option<TypeVar> {
        let var = Self::type_source_var(ty)?;
        if self.deferred_closure_numeric_param_vars.contains(&var) {
            Some(var)
        } else {
            None
        }
    }

    /// Infer type of property access
    pub(crate) fn infer_property_access(
        &mut self,
        object_type: &Type,
        property: &str,
    ) -> TypeResult<Type> {
        self.infer_property_access_internal(object_type, property, false)
    }

    /// Infer type of a property assignment target.
    ///
    /// Unlike reads, assignment targets may reference hoisted fields before first write.
    pub(crate) fn infer_property_assignment_target(
        &mut self,
        object_type: &Type,
        property: &str,
    ) -> TypeResult<Type> {
        self.infer_property_access_internal(object_type, property, true)
    }

    fn infer_property_access_internal(
        &mut self,
        object_type: &Type,
        property: &str,
        assignment_target: bool,
    ) -> TypeResult<Type> {
        // Field access through a reference (v0.3.3 B4, references slice D2):
        // `p.x` on a `p: &Point` / `&mut Point` parameter reads the field
        // THROUGH the reference. Deref the `Borrow { inner }` to its referent
        // and recurse so the field resolves on `Point` (the
        // references-borrowing.mdx ref-param field-access form). Mirrors the
        // value-position auto-deref already wired for `-> &T` call results; the
        // referent annotation is forwarded verbatim (no coercion). Without this
        // the `Borrow` type falls through to the `HasField` fallback and rejects
        // with "Borrow(..) cannot have fields".
        if let Type::Concrete(TypeAnnotation::Borrow { inner, .. }) = object_type {
            return self.infer_property_access_internal(
                &Type::Concrete((**inner).clone()),
                property,
                assignment_target,
            );
        }
        // U4-0 P2 (struct-name carrier normalization): a struct name may arrive
        // as `Basic("Emp")` rather than `Reference("Emp")` depending on the
        // annotation parse path — notably a CLOSURE param annotation (`|p: Emp|`)
        // resolves to `Basic`, whereas a struct literal / named-fn return yields
        // `Reference`. The struct-field projection below keys off the `Reference`
        // arm; a `Basic`-carried struct name falls through to the `HasField`
        // fallback, which (for `Basic`) only *tentatively accepts* the constraint
        // WITHOUT binding the field-result var to the declared field type. The
        // var then stays free post-solve and the whole closure-body field-read
        // (`|p: Emp| { p.salary }`) is dropped from the span table — the live U4
        // bug. Normalize a struct-named `Basic` to its `Reference` form so the
        // field resolves to its declared type (`int`) identically to `e.salary`.
        // Bounded to KNOWN struct defs / type aliases — an unregistered `Basic`
        // name (a genuine builtin/record scalar) is untouched and keeps its
        // existing record-schema / fallback path, so strictness (STAGE-F1) is
        // unaffected: f1's array element back-propagates to a `Reference`, never
        // a `Basic`, so this normalization never touches it.
        if let Type::Concrete(TypeAnnotation::Basic(name)) = object_type {
            if self.struct_type_defs.contains_key(name.as_str())
                || self.env.lookup_type_alias(name).is_some()
            {
                return self.infer_property_access_internal(
                    &Type::Concrete(TypeAnnotation::Reference(name.as_str().into())),
                    property,
                    assignment_target,
                );
            }
        }
        if let Type::Concrete(TypeAnnotation::Reference(name)) = object_type {
            // Check struct type definitions FIRST (includes comptime fields),
            // before type aliases (which only contain runtime fields).
            if let Some(struct_def) = self.struct_type_defs.get(name.as_str()).cloned() {
                for field in &struct_def.fields {
                    if field.name == property {
                        // A bare `Reference("Pair")` reaches here when an
                        // all-default generic literal (`Pair { first: 1,
                        // second: 2 }` where `type Pair<A=int,B=int>`)
                        // short-circuits to a bare reference in
                        // `infer_struct_literal_type`. The field's annotation
                        // is then the abstract param name (`A`/`B`); returning
                        // it raw makes `p.first + p.second` typecheck as
                        // `A + B`. If the annotation names a type param that
                        // carries a default, substitute that default so the
                        // field resolves to its defaulted concrete type. A
                        // param with no default (or a field that names no
                        // param) is unchanged — genuinely-abstract fields
                        // stay abstract, genuine UnknownProperty still errors.
                        if let Some(default) =
                            Self::default_for_named_type_param(&struct_def, &field.type_annotation)
                        {
                            return Ok(default);
                        }
                        return Ok(Type::Concrete(field.type_annotation.clone()));
                    }
                }
                return Err(TypeError::UnknownProperty(
                    name.to_string(),
                    property.to_string(),
                ));
            }
            // Fall back to type alias resolution
            if let Some(alias_entry) = self.env.lookup_type_alias(name) {
                return self.infer_property_access_internal(
                    &Type::Concrete(alias_entry.type_annotation.clone()),
                    property,
                    assignment_target,
                );
            }
        }

        // D-α.2: built-in `.length` property on length-bearing types
        // (Array<T>, String, HashMap<K,V>, Set<T>, Deque<T>,
        // PriorityQueue<T>, Range). The compiler emits typed length opcodes
        // for these (see `compiler/expressions/property_access.rs:288-335`,
        // method registry entries at `executor/objects/method_registry.rs`).
        // Without this arm the inference layer falls through to the
        // `HasField` fallback and the result variable stays `unknown`,
        // breaking expressions like `arr.length - 1` (KC #6(c)).
        if property == "length" && Self::is_length_bearing_type(object_type) {
            return Ok(BuiltinTypes::integer());
        }

        // Special handling for known types
        match object_type {
            Type::Generic { base, args } => {
                if let Some(type_name) = Self::generic_base_name(base) {
                    if type_name == "Table" {
                        return self.infer_property_access_fallback(object_type, property);
                    }
                    if type_name == "Row" && args.len() == 1 {
                        return self.infer_property_access_internal(
                            &args[0],
                            property,
                            assignment_target,
                        );
                    }

                    if let Some(field_type) =
                        self.resolve_struct_generic_field(type_name, args, property)
                    {
                        return Ok(field_type);
                    }

                    if self.struct_type_defs.contains_key(type_name) {
                        return Err(TypeError::UnknownProperty(
                            type_name.to_string(),
                            property.to_string(),
                        ));
                    }
                }

                self.infer_property_access_fallback(object_type, property)
            }
            // Row<T> property access: resolve field against T's schema
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Row" && args.len() == 1 =>
            {
                // Extract the inner type T and resolve the property on T
                let inner_type = Type::Concrete(args[0].clone());
                self.infer_property_access_internal(&inner_type, property, assignment_target)
            }
            // Table<T> property access: delegate to DataTable methods
            Type::Concrete(TypeAnnotation::Generic { name, .. }) if name == "Table" => {
                self.infer_property_access_fallback(object_type, property)
            }
            Type::Concrete(TypeAnnotation::Generic { name, args }) => {
                let generic_args: Vec<Type> = args.iter().cloned().map(Type::Concrete).collect();
                if let Some(field_type) =
                    self.resolve_struct_generic_field(name, &generic_args, property)
                {
                    return Ok(field_type);
                }
                if self.struct_type_defs.contains_key(name.as_str()) {
                    return Err(TypeError::UnknownProperty(
                        name.to_string(),
                        property.to_string(),
                    ));
                }
                self.infer_property_access_fallback(object_type, property)
            }
            // Check if this is a registered record schema type
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                // Look up record schema from environment
                if let Some(field_type) = self.env.get_record_field_type(name, property) {
                    return Ok(Type::Concrete(field_type.clone()));
                }
                // If schema exists but field doesn't, that's an error
                if self.env.lookup_record_schema(name).is_some() {
                    return Err(TypeError::UnknownProperty(
                        name.clone(),
                        property.to_string(),
                    ));
                }
                // Fall through to other cases for non-schema types
                self.infer_property_access_fallback(object_type, property)
            }
            Type::Concrete(TypeAnnotation::Intersection(types)) => {
                for ty in types {
                    if let Ok(field_type) = self.infer_property_access_internal(
                        &Type::Concrete(ty.clone()),
                        property,
                        assignment_target,
                    ) {
                        return Ok(field_type);
                    }
                }
                Err(TypeError::UnknownProperty(
                    "intersection".to_string(),
                    property.to_string(),
                ))
            }
            Type::Concrete(TypeAnnotation::Object(fields)) => {
                // Object type with known fields - check declared fields first
                if let Some(field) = fields.iter().find(|f| f.name == property) {
                    // A field may carry a `tyvar` marker (an object literal
                    // built from an unannotated parameter). Decode it back to
                    // a `Type::Variable` so a binop / comparison on the field
                    // resolves through the unifier once callsite propagation
                    // has bound the parameter.
                    if let Some(var) = annotation_as_tyvar(&field.type_annotation) {
                        return Ok(self
                            .solver
                            .unifier()
                            .apply_substitutions(&Type::Variable(var)));
                    }
                    return Ok(Type::Concrete(field.type_annotation.clone()));
                }

                // Check hoisted fields (from optimistic hoisting pre-pass).
                // Read access requires prior initialization; assignment targets do not.
                let hoisted_type = if assignment_target {
                    self.env.get_hoisted_field_for_assignment(property)
                } else {
                    self.env.get_hoisted_field(property)
                };
                if let Some(hoisted_type) = hoisted_type {
                    return Ok(hoisted_type);
                }

                // Field not found in declared or hoisted fields
                Err(TypeError::UnknownProperty(
                    "object".to_string(),
                    property.to_string(),
                ))
            }
            _ => {
                // If this is a tracked variable with hoisted fields, resolve from hoisting
                // even when the base type is still a type variable.
                let hoisted_type = if assignment_target {
                    self.env.get_hoisted_field_for_assignment(property)
                } else {
                    self.env.get_hoisted_field(property)
                };
                if let Some(hoisted_type) = hoisted_type {
                    return Ok(hoisted_type);
                }

                // Field was hoisted for assignment but not yet initialized for reads.
                if !assignment_target
                    && self
                        .env
                        .get_hoisted_field_for_assignment(property)
                        .is_some()
                {
                    return Err(TypeError::UnknownProperty(
                        "object".to_string(),
                        property.to_string(),
                    ));
                }

                // For unknown types, create a constraint
                let result_type = self.fresh_type_var();
                let var = self.fresh_var();

                self.constraints.push((
                    object_type.clone(),
                    Type::Constrained {
                        var,
                        constraint: Box::new(TypeConstraint::HasField(
                            property.to_string(),
                            Box::new(result_type.clone()),
                        )),
                    },
                ));

                Ok(result_type)
            }
        }
    }

    fn generic_base_name(base: &Type) -> Option<&str> {
        match base {
            Type::Concrete(ann) => ann.as_type_name_str(),
            _ => None,
        }
    }

    /// Returns true when `object_type` is one of the built-in length-bearing
    /// types whose `.length` property the compiler lowers to a typed length
    /// opcode (`ArrayLenTyped`, `MapLenTyped`, `StringLenTyped`, etc.). Used
    /// by `infer_property_access_internal` to short-circuit the
    /// `HasField` fallback so binary ops on the result are typed `int`.
    ///
    /// D-α.2. Mirrors the receiver-classification done by
    /// `try_resolve_typed_length_local` in
    /// `crates/shape-vm/src/compiler/expressions/property_access.rs` and the
    /// `"length"` PHF entries in
    /// `crates/shape-vm/src/executor/objects/method_registry.rs`.
    fn is_length_bearing_type(object_type: &Type) -> bool {
        let name = match object_type {
            Type::Concrete(TypeAnnotation::Array(_)) => return true,
            Type::Concrete(TypeAnnotation::Basic(n)) => n.as_str(),
            Type::Concrete(TypeAnnotation::Generic { name, .. }) => name.as_str(),
            Type::Generic { base, .. } => match Self::generic_base_name(base) {
                Some(n) => n,
                None => return false,
            },
            _ => return false,
        };
        matches!(
            name,
            "string"
                | "String"
                | "Vec"
                | "Array"
                | "HashMap"
                | "Set"
                | "Deque"
                | "PriorityQueue"
                | "Range"
        )
    }

    /// If `field_annotation` names one of `struct_def`'s declared type params,
    /// and that param has a default type, return the defaulted type. Used by
    /// the bare-`Reference` struct field-access arm so an all-default generic
    /// literal (`Pair { first: 1 }` for `type Pair<A = int>`) resolves field
    /// access to the param's default rather than the abstract param name.
    ///
    /// Returns `None` when the annotation is not a bare type-name, names no
    /// type param, or names a param without a default — in those cases the
    /// caller keeps the field's original (possibly abstract) annotation.
    fn default_for_named_type_param(
        struct_def: &shape_ast::ast::StructTypeDef,
        field_annotation: &TypeAnnotation,
    ) -> Option<Type> {
        // Only a bare name can alias a type param (e.g. `A`, not `Array<A>`).
        let field_name = match field_annotation {
            TypeAnnotation::Basic(_) | TypeAnnotation::Reference(_) => {
                field_annotation.as_type_name_str()?
            }
            _ => return None,
        };
        let type_params = struct_def.type_params.as_ref()?;
        let tp = type_params.iter().find(|tp| tp.name() == field_name)?;
        let default_ann = tp.default_type()?;
        Some(Type::Concrete(default_ann.clone()))
    }

    fn resolve_struct_generic_field(
        &self,
        type_name: &str,
        args: &[Type],
        property: &str,
    ) -> Option<Type> {
        let struct_def = self.struct_type_defs.get(type_name)?;
        let field = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .find(|f| f.name == property)?;
        let type_params = struct_def.type_params.as_ref()?;
        if type_params.is_empty() {
            return Some(Type::Concrete(field.type_annotation.clone()));
        }

        let mut bindings: HashMap<String, TypeAnnotation> = HashMap::new();
        for (tp, arg) in type_params.iter().zip(args.iter()) {
            if let Some(arg_ann) = arg.to_annotation() {
                // TODO(B.3): const generic params alias into this map under
                // their name; B.3 will route const args through a separate
                // value-level substitution pass.
                bindings.insert(tp.name().to_string(), arg_ann);
            }
        }
        let resolved =
            Self::substitute_type_params_in_annotation(&field.type_annotation, &bindings);
        Some(Type::Concrete(resolved))
    }

    fn substitute_type_params_in_annotation(
        annotation: &TypeAnnotation,
        bindings: &HashMap<String, TypeAnnotation>,
    ) -> TypeAnnotation {
        match annotation {
            ann @ (TypeAnnotation::Basic(_) | TypeAnnotation::Reference(_)) => {
                let name = ann.as_type_name_str().unwrap();
                bindings
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| annotation.clone())
            }
            TypeAnnotation::Borrow { mutable, inner } => TypeAnnotation::Borrow {
                mutable: *mutable,
                inner: Box::new(Self::substitute_type_params_in_annotation(inner, bindings)),
            },
            TypeAnnotation::Array(inner) => TypeAnnotation::Array(Box::new(
                Self::substitute_type_params_in_annotation(inner, bindings),
            )),
            TypeAnnotation::Tuple(items) => TypeAnnotation::Tuple(
                items
                    .iter()
                    .map(|item| Self::substitute_type_params_in_annotation(item, bindings))
                    .collect(),
            ),
            TypeAnnotation::Object(fields) => TypeAnnotation::Object(
                fields
                    .iter()
                    .map(|field| shape_ast::ast::ObjectTypeField {
                        name: field.name.clone(),
                        optional: field.optional,
                        type_annotation: Self::substitute_type_params_in_annotation(
                            &field.type_annotation,
                            bindings,
                        ),
                        annotations: vec![],
                    })
                    .collect(),
            ),
            TypeAnnotation::Function { params, returns } => TypeAnnotation::Function {
                params: params
                    .iter()
                    .map(|param| shape_ast::ast::FunctionParam {
                        name: param.name.clone(),
                        optional: param.optional,
                        type_annotation: Self::substitute_type_params_in_annotation(
                            &param.type_annotation,
                            bindings,
                        ),
                    })
                    .collect(),
                returns: Box::new(Self::substitute_type_params_in_annotation(
                    returns, bindings,
                )),
            },
            TypeAnnotation::Union(types) => TypeAnnotation::Union(
                types
                    .iter()
                    .map(|ty| Self::substitute_type_params_in_annotation(ty, bindings))
                    .collect(),
            ),
            TypeAnnotation::Intersection(types) => TypeAnnotation::Intersection(
                types
                    .iter()
                    .map(|ty| Self::substitute_type_params_in_annotation(ty, bindings))
                    .collect(),
            ),
            TypeAnnotation::Generic { name, args } => TypeAnnotation::Generic {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| Self::substitute_type_params_in_annotation(arg, bindings))
                    .collect(),
            },
            // ADR-009 B3 (S1): existential descriptor package type. The
            // witnesses are locally bound, so outer type-param bindings must not
            // substitute them — shadow them out before recursing into the inner
            // descriptor.
            TypeAnnotation::Existential { witnesses, inner } => {
                let mut inner_bindings = bindings.clone();
                for w in witnesses {
                    inner_bindings.remove(w);
                }
                TypeAnnotation::Existential {
                    witnesses: witnesses.clone(),
                    inner: Box::new(Self::substitute_type_params_in_annotation(
                        inner,
                        &inner_bindings,
                    )),
                }
            }
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined
            | TypeAnnotation::Dyn(_) => annotation.clone(),
        }
    }

    /// Fallback for property access when record schema doesn't apply
    fn infer_property_access_fallback(
        &mut self,
        object_type: &Type,
        property: &str,
    ) -> TypeResult<Type> {
        // For unknown types, create a constraint
        let result_type = self.fresh_type_var();
        let var = self.fresh_var();

        self.constraints.push((
            object_type.clone(),
            Type::Constrained {
                var,
                constraint: Box::new(TypeConstraint::HasField(
                    property.to_string(),
                    Box::new(result_type.clone()),
                )),
            },
        ));

        Ok(result_type)
    }

    /// Push an element-carrying `Indexable` constraint for an `obj[i]`
    /// access whose object type is not yet a concrete array.
    ///
    /// Returns the fresh element-type variable. The constraint records that
    /// variable inside `TypeConstraint::Indexable`, so when the object's
    /// type later resolves to a concrete `Array<T>` (e.g. via callsite
    /// unification of an unannotated parameter), `ConstraintSolver::
    /// apply_bounds` binds the element variable to `T`. Without the carried
    /// element type the access would return a disconnected fresh variable
    /// and a downstream `obj[i] + ...` would see `unknown` operands.
    fn push_indexable_constraint(&mut self, object_type: &Type) -> Type {
        let result_type = self.fresh_type_var();
        let var = self.fresh_var();
        self.constraints.push((
            object_type.clone(),
            Type::Constrained {
                var,
                constraint: Box::new(TypeConstraint::Indexable(Box::new(result_type.clone()))),
            },
        ));
        result_type
    }

    /// Infer type of index access
    pub(crate) fn infer_index_access(
        &mut self,
        object_type: &Type,
        index_type: &Type,
    ) -> TypeResult<Type> {
        match object_type {
            // Index access through a reference (v0.3.3 RefDispatch):
            // `r[i]` on a `r: &Array<T>` / `&mut Array<T>` indexes THROUGH the
            // reference. Deref the `Borrow { inner }` to its referent and recurse
            // so the element type resolves on the referent (mirrors the
            // field-access auto-deref at access.rs:46-52). Without this the
            // `Borrow` type falls to the `_` wildcard → a fresh indexable var
            // and, in the constraint checker, the `Indexable` arm rejects with
            // "Borrow(..) does not support index access".
            Type::Concrete(TypeAnnotation::Borrow { inner, .. }) => {
                self.infer_index_access(&Type::Concrete((**inner).clone()), index_type)
            }
            // Row<T> disallows dynamic string indexing - use row.field instead
            Type::Concrete(TypeAnnotation::Generic { name, .. }) if name == "Row" => {
                Err(TypeError::TypeMismatch(
                    "static field access (row.field)".to_string(),
                    "dynamic index access (row[...]) on typed Row<T>".to_string(),
                ))
            }
            Type::Concrete(TypeAnnotation::Array(elem_type)) => {
                // Array indexing — the index must be `int` (WF-4, STRICT). See
                // `require_int_index_constraint`: `number`/float/decimal indices
                // are a compile error (explicit `as int` cast required), not a
                // silent i64 reinterpretation.
                self.require_int_index_constraint(index_type)?;
                Ok(Type::Concrete(*elem_type.clone()))
            }
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if (name == "Vec" || name == "Array") && args.len() == 1 =>
            {
                self.require_int_index_constraint(index_type)?;
                Ok(Type::Concrete(args[0].clone()))
            }
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Mat" && args.len() == 1 =>
            {
                self.require_int_index_constraint(index_type)?;
                Ok(Type::Concrete(TypeAnnotation::Generic {
                    name: "Vec".into(),
                    args: args.clone(),
                }))
            }
            // STRICT-FLIP (v0.3.3 map/collect OUTPUT element stamp): the canonical
            // `Type::Generic { base: Vec/Array, args: [elem] }` form. A `.map()`
            // result resolves through `resolve_type_param_expr`'s
            // `GenericContainer` arm to this engine-level `Type::Generic` shape
            // (NOT `Type::Concrete(Array(_))` — that form is reserved for
            // `SelfType`-returning methods like `filter` whose receiver is already
            // concrete). Without this arm `r[0]` on `let r = [1,2,3].map(|x| x*2)`
            // fell to the `_` wildcard → a fresh `push_indexable_constraint` var →
            // a FREE element type that wrongly unified with a `number` annotation.
            // Extract the element type so `r[0]` resolves to the exact element
            // (`int` stays `int`; `int` and `number` do NOT unify — CLAUDE.md
            // §Type-System-Rules). Same `Numeric`-bound index constraint as the
            // concrete-array arm.
            Type::Generic { base, args }
                if args.len() == 1
                    && matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(n))
                            if n.as_str() == "Vec" || n.as_str() == "Array"
                    ) =>
            {
                self.require_int_index_constraint(index_type)?;
                Ok(args[0].clone())
            }
            // String index `s[i]` — the i-th character (book
            // `fundamentals/strings.mdx` llm_summary + operators.mdx
            // §Indexing). Shape has NO first-class `char` type (STAGE-S4): a
            // single character is a real 1-char `string`, so `s[i]: string`
            // (exact parity with `s.charAt(i)`). The index must be `int` like
            // array indexing (WF-4 STRICT). Must precede the generic
            // `Basic(name)` record-schema arm below (a "string" Basic would
            // otherwise fall to a fresh indexable var → `unknown`, breaking
            // strict-typed downstream uses like `acc + s[i]`).
            Type::Concrete(TypeAnnotation::Basic(name)) if name.as_str() == "string" => {
                self.require_int_index_constraint(index_type)?;
                Ok(Type::Concrete(TypeAnnotation::Basic("string".into())))
            }
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                // Check if this is a registered record schema (e.g., "rows" returns "row")
                if self.env.lookup_record_schema(name).is_some() {
                    self.require_int_index_constraint(index_type)?;
                    Ok(Type::Concrete(TypeAnnotation::Basic(name.clone())))
                } else {
                    // For unknown types, create an element-carrying index
                    // constraint so the element type connects to the
                    // object's resolved type (mirrors `HasField`).
                    Ok(self.push_indexable_constraint(object_type))
                }
            }
            _ => {
                // For unknown types, create an element-carrying index
                // constraint (mirrors `HasField`) — see above.
                Ok(self.push_indexable_constraint(object_type))
            }
        }
    }

    /// Resolve a tuple element access `tup[k]` to its positional element type
    /// (book `fundamentals/variables` §Tuple Types). A tuple fixes both its
    /// length and the per-position element type at compile time, so the index
    /// MUST be a compile-time-constant non-negative integer literal — a tuple
    /// has no single uniform element type, so a runtime/variable index cannot
    /// be resolved and is a compile error (surface, do not fabricate a type).
    pub(crate) fn infer_tuple_index(
        &mut self,
        elem_types: &[TypeAnnotation],
        index: &shape_ast::ast::Expr,
    ) -> TypeResult<Type> {
        use shape_ast::ast::{Expr, Literal};
        let k: i64 = match index {
            Expr::Literal(Literal::Int(i), _) => *i,
            Expr::Literal(Literal::TypedInt(i, _), _) => *i,
            _ => {
                return Err(TypeError::TypeMismatch(
                    "constant integer index into a tuple".to_string(),
                    "non-constant tuple index (tuple element types are \
                     position-specific; index with a literal like tup[0])"
                        .to_string(),
                ));
            }
        };
        if k < 0 || (k as usize) >= elem_types.len() {
            return Err(TypeError::TypeMismatch(
                format!("tuple index in 0..{}", elem_types.len()),
                format!("out-of-range tuple index {}", k),
            ));
        }
        Ok(Type::Concrete(elem_types[k as usize].clone()))
    }

    /// Require an array/string/record index operand to be `int` (STRICT,
    /// WF-4 index-type). `int` and `number` are SEPARATE families and do NOT
    /// unify (CLAUDE.md §Type-System-Rules); a `number`/`decimal`/float index
    /// silently reinterpreted as an `i64` index is the top strict-typing hole
    /// (reliableonly_strict_bypass class): `arr[1.5]` / `arr[n: number]` /
    /// `arr[time::millis()]` all used to compile and read a garbage element or
    /// run off the end. The index MUST be proven `int` at compile time.
    ///
    /// - A CONCRETE non-`int` index (number/decimal/float/any non-int) is a
    ///   compile error — the user must write an explicit `as int` cast
    ///   (`number -> int` is lossy per the numeric-conversion rule, so it is
    ///   NOT implicit; NO `IntToNumber`/`NumberToInt`/`Convert*To` coercion
    ///   opcode is emitted — that is a §Forbidden Patterns dynamic-fallback).
    /// - A still-unresolved index VARIABLE is unified with `int` (hard equality,
    ///   NOT a `Numeric` bound that `number` would satisfy). A genuine int var
    ///   binds; a var that later resolves to `number` conflicts → compile error.
    ///   This is not the "unbound-var-unifies-with-anything" hole: the index USE
    ///   site legitimately constrains the operand to `int`.
    fn require_int_index_constraint(&mut self, index_type: &Type) -> TypeResult<()> {
        let resolved = self.solver.unifier().apply_substitutions(index_type);
        let int_ty = Type::Concrete(TypeAnnotation::Basic("int".to_string()));
        let concrete_name = match &resolved {
            Type::Concrete(TypeAnnotation::Basic(name)) => Some(name.as_str().to_string()),
            Type::Concrete(TypeAnnotation::Reference(name)) => Some(name.as_str().to_string()),
            _ => None,
        };
        match &resolved {
            _ if concrete_name.is_some() => {
                let name = concrete_name.unwrap();
                if BuiltinTypes::is_integer_type_name(&name) {
                    Ok(())
                } else {
                    Err(TypeError::TypeMismatch(
                        "int".to_string(),
                        format!(
                            "{} — array index must be `int`; add an explicit `as int` \
                             cast if truncation is intended",
                            name
                        ),
                    ))
                }
            }
            // Any other concrete shape (array/object/tuple/function/...) is not
            // an integer index.
            Type::Concrete(_) => Err(TypeError::TypeMismatch(
                "int".to_string(),
                "non-integer value — array index must be `int`".to_string(),
            )),
            // Unresolved variable / constrained var: constrain to `int` by
            // unification (see doc comment above).
            _ => {
                self.constraints.push((resolved, int_ty));
                Ok(())
            }
        }
    }

    /// Extract `T` from an `Array<T>` / `Vec<T>` type shape, for the series
    /// form of `min` / `max`. Returns `None` for non-array types (the call
    /// then falls through to the "requires a numeric array" error). Mirrors
    /// the private `array_element_type` in `operators.rs`.
    fn min_max_array_element_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Concrete(TypeAnnotation::Array(inner)) => Some(Type::Concrete((**inner).clone())),
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if (name == "Array" || name == "Vec") && args.len() == 1 =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            Type::Generic { base, args }
                if args.len() == 1
                    && matches!(
                        base.as_ref(),
                        Type::Concrete(ann)
                            if matches!(ann.as_type_name_str(), Some("Array") | Some("Vec"))
                    ) =>
            {
                Some(args[0].clone())
            }
            _ => None,
        }
    }

    fn check_comptime_builtin_args(
        &mut self,
        arg_types: &[Type],
        expected: &[Type],
        call_span: Span,
    ) -> TypeResult<()> {
        if arg_types.len() != expected.len() {
            return Err(TypeError::ArityMismatch(expected.len(), arg_types.len()));
        }
        for (arg_ty, expected_ty) in arg_types.iter().zip(expected.iter()) {
            self.push_constraint_with_origin(arg_ty.clone(), expected_ty.clone(), call_span);
        }
        Ok(())
    }

    fn typed_object_field(name: &str, type_annotation: TypeAnnotation) -> ObjectTypeField {
        ObjectTypeField {
            name: name.to_string(),
            optional: false,
            type_annotation,
            annotations: Vec::new(),
        }
    }

    fn comptime_object_type(fields: Vec<(&str, TypeAnnotation)>) -> Type {
        Type::Concrete(TypeAnnotation::Object(
            fields
                .into_iter()
                .map(|(name, type_annotation)| Self::typed_object_field(name, type_annotation))
                .collect(),
        ))
    }

    /// The `FieldDescriptor` row shape (comptime-excellence §4.1.1) as a
    /// concrete object annotation, so `type_info(T).fields[i]` /
    /// `target.fields[i]` subscript access resolves to a real object type with
    /// `.name` / `.type` / `.optional` / `.annotations` fields (an
    /// `unknown`-element array is iterable but not indexable, which regressed
    /// the flagship `fields[0].name` form).
    fn comptime_field_descriptor_annotation() -> TypeAnnotation {
        TypeAnnotation::Object(vec![
            Self::typed_object_field("name", TypeAnnotation::Basic("string".to_string())),
            Self::typed_object_field("type", TypeAnnotation::Basic("string".to_string())),
            Self::typed_object_field(
                "annotations",
                TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("string".to_string()))),
            ),
            Self::typed_object_field("optional", TypeAnnotation::Basic("bool".to_string())),
            Self::typed_object_field("type_ref", Self::comptime_type_ref_annotation()),
        ])
    }

    fn comptime_type_ref_annotation() -> TypeAnnotation {
        TypeAnnotation::Object(vec![
            Self::typed_object_field("name", TypeAnnotation::Basic("string".to_string())),
            Self::typed_object_field("kind", TypeAnnotation::Basic("string".to_string())),
            Self::typed_object_field("source", TypeAnnotation::Basic("string".to_string())),
        ])
    }

    fn infer_comptime_builtin_call(
        &mut self,
        name: &str,
        arg_types: &[Type],
        call_span: Span,
    ) -> TypeResult<Option<Type>> {
        if !crate::builtin_metadata::is_comptime_builtin_function(name) {
            return Ok(None);
        }
        if !self.in_comptime_context() {
            return Err(TypeError::ConstraintViolation(format!(
                "'{}' is a comptime-only builtin and can only be called inside a `comptime {{ }}` block",
                name
            )));
        }

        let string = BuiltinTypes::string();
        let result = match name {
            "implements" => {
                self.check_comptime_builtin_args(arg_types, &[string.clone(), string], call_span)?;
                BuiltinTypes::boolean()
            }
            "warning" => {
                self.check_comptime_builtin_args(arg_types, &[string], call_span)?;
                BuiltinTypes::void()
            }
            "error" => {
                self.check_comptime_builtin_args(arg_types, &[string], call_span)?;
                Type::Concrete(TypeAnnotation::Never)
            }
            "string_lit" => {
                self.check_comptime_builtin_args(arg_types, &[string.clone()], call_span)?;
                string
            }
            "build_config" => {
                self.check_comptime_builtin_args(arg_types, &[], call_span)?;
                Self::comptime_object_type(vec![
                    // `comptime_api` is the frozen introspection-contract
                    // version marker (comptime-excellence §4.1.4); must stay in
                    // sync with the `__ComptimeBuildConfig` schema
                    // (`builtin_schemas.rs`) and the `build_config` value
                    // builder (`comptime_builtins.rs`).
                    ("comptime_api", TypeAnnotation::Basic("int".to_string())),
                    ("debug", TypeAnnotation::Basic("bool".to_string())),
                    ("target_arch", TypeAnnotation::Basic("string".to_string())),
                    ("target_os", TypeAnnotation::Basic("string".to_string())),
                    ("version", TypeAnnotation::Basic("string".to_string())),
                ])
            }
            "type_info" => {
                self.check_comptime_builtin_args(arg_types, &[string], call_span)?;
                Self::comptime_object_type(vec![
                    ("kind", TypeAnnotation::Basic("string".to_string())),
                    ("name", TypeAnnotation::Basic("string".to_string())),
                    // `fields` is the declared-field descriptor array
                    // (comptime-excellence §4.1.2). A concrete `FieldDescriptor`
                    // element (not `unknown`) so `type_info(T).fields[i].name`
                    // subscript access resolves identically to `target.fields`.
                    (
                        "fields",
                        TypeAnnotation::Array(Box::new(
                            Self::comptime_field_descriptor_annotation(),
                        )),
                    ),
                    ("type_ref", Self::comptime_type_ref_annotation()),
                ])
            }
            "type_ref" => {
                self.check_comptime_builtin_args(
                    arg_types,
                    &[Type::Concrete(TypeAnnotation::Basic(
                        crate::builtin_metadata::COMPTIME_TYPE_SYNTAX_MARKER.to_string(),
                    ))],
                    call_span,
                )?;
                Type::Concrete(TypeAnnotation::Basic(
                    crate::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA
                        .to_string(),
                ))
            }
            "type_category" => {
                self.check_comptime_builtin_args(
                    arg_types,
                    &[Type::Concrete(TypeAnnotation::Basic(
                        crate::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA
                            .to_string(),
                    ))],
                    call_span,
                )?;
                Type::Concrete(TypeAnnotation::Basic("FrozenTypeCategory".to_string()))
            }
            // ADR-009 B1 S3 — `reflect(TypeRef<T>) -> FrozenType<T>`. R4
            // arg-form rejections are NAMED (mirroring the `type_ref`
            // arg-form rejections below in `infer_function_call`): wrong
            // arity, and any argument whose resolved type is concretely NOT
            // the opaque TypeRef schema (string, int, the legacy
            // `__ComptimeTypeRef` descriptor object, arbitrary objects).
            // Only a still-unresolved argument falls through to the
            // standard constraint push.
            "reflect" => {
                let type_ref_schema =
                    crate::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA;
                if arg_types.len() != 1 {
                    return Err(TypeError::ConstraintViolation(
                        "reflect expects exactly one TypeRef argument".to_string(),
                    ));
                }
                let resolved = self.solver.unifier().apply_substitutions(&arg_types[0]);
                match resolved.to_annotation() {
                    Some(TypeAnnotation::Basic(name)) if name == type_ref_schema => {}
                    Some(_) => {
                        return Err(TypeError::ConstraintViolation(
                            "reflect expects a TypeRef value (create one with type_ref(T)); \
                             strings and other values cannot be reflected"
                                .to_string(),
                        ));
                    }
                    None => {
                        self.push_constraint_with_origin(
                            arg_types[0].clone(),
                            Type::Concrete(TypeAnnotation::Basic(type_ref_schema.to_string())),
                            call_span,
                        );
                    }
                }
                Type::Concrete(TypeAnnotation::Basic(
                    crate::comptime_reflection::FROZEN_TYPE_PAYLOAD_ENUM_NAME.to_string(),
                ))
            }
            // ADR-009 B2 (slice S4): `trait_ref(Tr)` types as the opaque
            // TraitRef carrier — a DISTINCT identity kind from TypeRef (a
            // trait is not a value type, Dec 49). Argument is the same
            // compiler-resolved type-syntax marker as `type_ref` (bare
            // identifier; strings are rejected in `infer_function_call`).
            "trait_ref" => {
                self.check_comptime_builtin_args(
                    arg_types,
                    &[Type::Concrete(TypeAnnotation::Basic(
                        crate::builtin_metadata::COMPTIME_TYPE_SYNTAX_MARKER.to_string(),
                    ))],
                    call_span,
                )?;
                Type::Concrete(TypeAnnotation::Basic(
                    crate::type_schema::builtin_schemas::COMPTIME_FROZEN_TRAIT_REF_SCHEMA
                        .to_string(),
                ))
            }
            // ADR-009 B2 (slice S4): `find_impl(TypeRef, TraitRef)` types as
            // `Option<ImplRef>` — branch-scoped evidence consumed through the
            // `Some(proof)` match arm; an unimplemented pair is `None` (R9).
            //
            // Slice S5 named rejection rows precede the generic constraint
            // path so the forbidden forms fail with their Dec 49 names:
            // R8 (arity), R4 (boolean-authorized generation), R3 (name-string
            // lookup). Anything else still flows through the schema-typed
            // constraints below (R7's checker tier: arbitrary values and
            // legacy descriptors never unify with the opaque carriers).
            "find_impl" => {
                if arg_types.len() != 2 {
                    return Err(TypeError::ConstraintViolation(
                        "find_impl expects exactly two arguments: a TypeRef from type_ref and a \
                         TraitRef from trait_ref"
                            .to_string(),
                    ));
                }
                for arg_type in arg_types {
                    if let Type::Concrete(TypeAnnotation::Basic(basic)) = arg_type {
                        if basic == "bool" {
                            return Err(TypeError::ConstraintViolation(
                                "a boolean cannot authorize an operation that requires \
                                 implementation evidence: find_impl consumes a compiler-issued \
                                 TypeRef and TraitRef and answers with ImplRef evidence — a bool \
                                 (including an implements(...) result) never unifies with \
                                 implementation evidence"
                                    .to_string(),
                            ));
                        }
                        if basic == "string" {
                            return Err(TypeError::ConstraintViolation(
                                "trait lookup cannot use text: find_impl expects a \
                                 compiler-issued TypeRef and TraitRef — strings cannot name a \
                                 type or a trait here"
                                    .to_string(),
                            ));
                        }
                    }
                }
                self.check_comptime_builtin_args(
                    arg_types,
                    &[
                        Type::Concrete(TypeAnnotation::Basic(
                            crate::type_schema::builtin_schemas::COMPTIME_FROZEN_TYPE_REF_SCHEMA
                                .to_string(),
                        )),
                        Type::Concrete(TypeAnnotation::Basic(
                            crate::type_schema::builtin_schemas::COMPTIME_FROZEN_TRAIT_REF_SCHEMA
                                .to_string(),
                        )),
                    ],
                    call_span,
                )?;
                Type::Concrete(TypeAnnotation::option(TypeAnnotation::Basic(
                    crate::type_schema::builtin_schemas::COMPTIME_FROZEN_IMPL_REF_SCHEMA
                        .to_string(),
                )))
            }
            _ => {
                return Err(TypeError::ConstraintViolation(format!(
                    "comptime builtin '{}' has no type-analysis signature",
                    name
                )));
            }
        };

        Ok(Some(result))
    }

    /// Infer type of function call
    pub(crate) fn infer_function_call(
        &mut self,
        name: &str,
        args: &[Expr],
        call_span: Span,
    ) -> TypeResult<Type> {
        // Bidirectional closure-arg param inference (R4): when an argument is a
        // closure literal (`Expr::FunctionExpr`) and the callee's corresponding
        // parameter is a CONCRETE function type, drive the closure through
        // `check_against` with that expected function type. This binds the
        // closure's unannotated parameters to the expected concrete param types
        // (e.g. `apply(|x| x * 2, 21)` with `apply(f: (x: int) => int, …)` binds
        // `x: int`) BEFORE body inference, so the closure-eager
        // `refine_callable_param_types_from_local_constraints` numeric-collapse
        // (which would otherwise pin a `Numeric`-bounded `x` to `number`) never
        // fires. No type kind is fabricated and no value widening is introduced:
        // the param simply adopts the annotated parameter type, exactly as a
        // closure with an explicit `|x: int|` annotation would.
        //
        // CLAUDE.md "Bidirectional closure inference": extends the method-call
        // closure-param inference to fn-param-typed closures. Only fires when
        // the expected param is a concrete `Function` shape; everything else
        // falls through to plain `infer_expr` (unchanged behavior).
        let expected_closure_param_types = self.callee_concrete_param_fn_types(name, args);
        // `type_info(User)` / `type_ref(User)` / `implements(Bar, Serialize)`
        // name a type or trait directly as the argument: a bare identifier that
        // is not a value binding. The comptime driver rewrites legacy reflection
        // arguments to strings, while `type_ref` receives an unspellable typed
        // syntax marker and is then lowered to its compiler-issued identity.
        // Mirror those distinct carriers here so the outer type-check accepts
        // the bare-identifier form without making TypeRef constructible from
        // source strings.
        let is_type_ref_builtin =
            name == "type_ref" && crate::builtin_metadata::is_comptime_builtin_function(name);
        if is_type_ref_builtin {
            match args {
                // ADR-009 A2 (slice S4) lockstep contract: this accepted-shape
                // set mirrors the comptime rewrite arm
                // (shape-vm compiler/comptime.rs
                // `rewrite_comptime_type_symbol_args_expr`) — a bare
                // compiler-resolved identifier OR the checked type-expression
                // carrier `Expr::TypeSyntax` (tuples, records, callables,
                // references, unions, erased dyn, applied generics). Any
                // drift between the two produces compile-passes-then-
                // comptime-fails splits; change both together.
                [Expr::Identifier(..)] | [Expr::TypeSyntax(..)] => {}
                [Expr::Literal(shape_ast::ast::Literal::String(_), _)] => {
                    return Err(TypeError::ConstraintViolation(
                        "type_ref expects compiler-resolved type syntax; strings cannot construct TypeRef"
                            .to_string(),
                    ));
                }
                [_] => {
                    return Err(TypeError::ConstraintViolation(
                        "type_ref expects compiler-resolved type syntax such as type_ref(int)"
                            .to_string(),
                    ));
                }
                _ => {
                    return Err(TypeError::ConstraintViolation(
                        "type_ref expects exactly one type argument".to_string(),
                    ));
                }
            }
        }
        // ADR-009 B1 S3 — R4: reflect's arity rejection fires EARLY (before
        // argument inference and the generic callee-scheme arity check can
        // produce an unnamed diagnostic first), mirroring the type_ref
        // arg-form block above. Value-form rejections (string / int /
        // legacy descriptor) need the inferred argument type and live in
        // `infer_comptime_builtin_call`.
        if name == "reflect"
            && crate::builtin_metadata::is_comptime_builtin_function(name)
            && args.len() != 1
        {
            return Err(TypeError::ConstraintViolation(
                "reflect expects exactly one TypeRef argument".to_string(),
            ));
        }
        // ADR-009 B2 (slice S4): `trait_ref` mirrors `type_ref`'s
        // bare-identifier carrier — a declared trait named directly. Trait
        // lookup cannot use text (Dec 49): strings never construct a
        // TraitRef. (S5 lands the full named rejection matrix; these forms
        // must not type-check meanwhile.)
        let is_trait_ref_builtin =
            name == "trait_ref" && crate::builtin_metadata::is_comptime_builtin_function(name);
        if is_trait_ref_builtin {
            match args {
                [Expr::Identifier(..)] => {}
                [Expr::Literal(shape_ast::ast::Literal::String(_), _)] => {
                    return Err(TypeError::ConstraintViolation(
                        "trait_ref expects a declared trait named directly; strings cannot construct TraitRef"
                            .to_string(),
                    ));
                }
                [_] => {
                    return Err(TypeError::ConstraintViolation(
                        "trait_ref expects a declared trait named directly, such as trait_ref(Serializable)"
                            .to_string(),
                    ));
                }
                _ => {
                    return Err(TypeError::ConstraintViolation(
                        "trait_ref expects exactly one trait argument".to_string(),
                    ));
                }
            }
        }
        let type_symbol_ident_args = crate::builtin_metadata::is_comptime_builtin_function(name)
            && ((self.in_comptime_context() && matches!(name, "type_info" | "implements"))
                || is_type_ref_builtin
                || is_trait_ref_builtin);
        let mut arg_types: Vec<Type> = Vec::with_capacity(args.len());
        for (i, arg) in args.iter().enumerate() {
            // ADR-009 A2 (S4): the type-syntax carrier is never inferred as a
            // value expression — it carries the same unspellable typed-syntax
            // marker as the bare-identifier form (one marker, two accepted
            // spellings; identical to the comptime rewrite's two arms).
            if is_type_ref_builtin && matches!(arg, Expr::TypeSyntax(..)) {
                arg_types.push(Type::Concrete(TypeAnnotation::Basic(
                    crate::builtin_metadata::COMPTIME_TYPE_SYNTAX_MARKER.to_string(),
                )));
                continue;
            }
            if type_symbol_ident_args && matches!(arg, Expr::Identifier(..)) {
                arg_types.push(if matches!(name, "type_ref" | "trait_ref") {
                    Type::Concrete(TypeAnnotation::Basic(
                        crate::builtin_metadata::COMPTIME_TYPE_SYNTAX_MARKER.to_string(),
                    ))
                } else {
                    BuiltinTypes::string()
                });
                continue;
            }
            let arg_type = match (
                arg,
                expected_closure_param_types
                    .as_ref()
                    .and_then(|v| v.get(i).cloned().flatten()),
            ) {
                (Expr::FunctionExpr { .. }, Some(expected_fn_ty)) => {
                    self.check_against(arg, &expected_fn_ty)?
                }
                _ => {
                    let inferred = self.infer_expr(arg)?;
                    // Indirected-callable soundness discriminator. A closure
                    // LITERAL passed as a value argument to a USER (source-located)
                    // function — with NO concrete expected fn type to drive
                    // bidirectional param inference — ESCAPES into a callee that
                    // may invoke it. Record its still-unresolved numeric param
                    // source vars so `default_unresolved_closure_numeric_params`
                    // SURFACEs (rejects) rather than silently defaulting them to
                    // `number` if the call graph never pins them (the `id(|a,b|
                    // a*b)` / 2-level-wrapper severed-link case). A closure whose
                    // param the engine CAN thread (direct `applyx(|a,b| a*b,6,7)`)
                    // resolves before the default and is never affected; a
                    // never-called closure (`let f = |x| x*3`) is never recorded
                    // because it escapes into no user call. Builtins (`print`)
                    // have no source span, so they do not trigger the surface.
                    // S1 indirected-callable extension (forwarded-by-NAME
                    // closure): a closure bound to a `let` and then forwarded
                    // through an untyped HOF param by IDENTIFIER (`let mul =
                    // |x| x * 2; use_it(mul)`) escapes into the callee exactly
                    // like an inline-literal closure arg, but `arg` is an
                    // `Expr::Identifier`, not an `Expr::FunctionExpr`, so the
                    // literal-only gate below missed it. The closure's numeric
                    // param then hit the `default_unresolved_closure_numeric_
                    // params` `number` default (silent int->number widening:
                    // `use_it(mul)` returned `10.0`/the f64 bit-pattern, not
                    // int `10` — the worst-class soundness leak). Widen the
                    // gate to ALSO fire when the arg's inferred type is a
                    // `Function` whose param vars are tracked in
                    // `deferred_closure_numeric_param_vars` (i.e. it genuinely
                    // came from an unannotated closure literal). The per-var
                    // `deferred_closure_numeric_param_vars.contains(v)` check
                    // in the body is the precise discriminator — a forwarded
                    // ANNOTATED closure or a named-function reference carries
                    // no such vars and is left untouched. No fabrication, no
                    // default: the closure either gets pinned by a downstream
                    // concrete call site (via `escaping_closure_arg_sites`
                    // follow-the-callable) or is REJECTED cleanly.
                    let forwarded_binding_hints = if let Expr::Identifier(arg_name, _) = arg {
                        self.deferred_closure_numeric_binding_hints
                            .get(arg_name)
                            .cloned()
                    } else {
                        None
                    };
                    let arg_carries_deferred_closure_param = matches!(arg, Expr::Identifier(..))
                        && (forwarded_binding_hints.is_some()
                            || matches!(&inferred, Type::Function { params, .. }
                                if params
                                    .iter()
                                    .any(|p| self.deferred_closure_numeric_source_var(p).is_some())));
                    if (matches!(arg, Expr::FunctionExpr { .. })
                        || arg_carries_deferred_closure_param)
                        && self.lookup_callable_origin_for_name(name).is_some()
                    {
                        if let Type::Function { params, .. } = &inferred {
                            let mut site_vars: Vec<TypeVar> = Vec::with_capacity(params.len());
                            let mut all_param_vars = !params.is_empty();
                            for (param_index, p) in params.iter().enumerate() {
                                if let Some(v) = Self::type_source_var(p) {
                                    if self.deferred_closure_numeric_source_var(p).is_some() {
                                        self.escaping_closure_numeric_param_vars.insert(v.clone());
                                    } else if let Some(hints) = forwarded_binding_hints.as_ref() {
                                        if let Some((true, hint)) = hints.get(param_index) {
                                            self.deferred_closure_numeric_param_vars
                                                .insert(v.clone());
                                            self.escaping_closure_numeric_param_vars
                                                .insert(v.clone());
                                            if let Some(hint) = hint {
                                                self.deferred_closure_numeric_param_body_hint
                                                    .insert(v.clone(), hint.clone());
                                            }
                                        }
                                    }
                                    site_vars.push(v.clone());
                                } else {
                                    all_param_vars = false;
                                }
                            }
                            // COMPLETENESS extension: record the full site so the
                            // post-inference indirected-callable resolver can FOLLOW
                            // the callable (forwarding wrapper / id-laundered let)
                            // to a concrete invocation and pin these param vars. We
                            // only record when EVERY closure param is a bare var
                            // (an annotated param needs no inference); a partially
                            // annotated closure is left to its existing path.
                            if all_param_vars && !site_vars.is_empty() {
                                self.escaping_closure_arg_sites.push((
                                    name.to_string(),
                                    i,
                                    site_vars,
                                    args.to_vec(),
                                ));
                            }
                        }
                    }
                    inferred
                }
            };
            // GAP-2 boundary: in pass-by-reference ARGUMENT position, `&x` is the
            // by-reference call mechanism, NOT a `&T` Borrow value. The call-shape
            // constraint + `propagate_ref_arg_param_types` both expect the
            // REFERENT type here (`fn triple(&x) { x = x * 3 }` requires `x: int`
            // from the `triple(&val)` site, not `&int`). The standalone
            // `Expr::Reference` inference correctly yields `&T` (needed for
            // `-> &T` return unification); we unwrap that Borrow back to its
            // referent for argument flow only. `&mut` unwraps identically — the
            // mutability lives in the param's `is_reference`/`is_mut_reference`
            // flags, not the arg type.
            let arg_type = if matches!(arg, Expr::Reference { .. }) {
                match self.solver.unifier().apply_substitutions(&arg_type) {
                    Type::Concrete(TypeAnnotation::Borrow { inner, .. }) => Type::Concrete(*inner),
                    other => other,
                }
            } else {
                arg_type
            };
            arg_types.push(arg_type);
        }

        // Builtin arity special-cases that cannot be represented by a single
        // fixed-arity function type in the symbol table.
        if name == "print" {
            if arg_types.is_empty() {
                return Err(TypeError::ConstraintViolation(
                    "Function 'print' expects at least 1 argument, got 0".to_string(),
                ));
            }
            return Ok(BuiltinTypes::void());
        }

        if name == "range" {
            let actual_arity = arg_types.len();
            if !(1..=3).contains(&actual_arity) {
                return Err(TypeError::ConstraintViolation(format!(
                    "Function 'range' expects between 1 and 3 arguments, got {}",
                    actual_arity
                )));
            }
            let origin = self
                .lookup_callable_origin_for_name(name)
                .unwrap_or(call_span);
            for arg_ty in &arg_types {
                self.push_constraint_with_origin(arg_ty.clone(), BuiltinTypes::integer(), origin);
            }
            return Ok(BuiltinTypes::array(BuiltinTypes::integer()));
        }

        // `min` / `max` are documented (stdlib/native/math.mdx) as working
        // "across arguments OR a series" — two non-fixed-arity shapes the
        // single `(T, T) -> T` symbol-table scheme cannot represent:
        //
        //   1. Variadic: two-or-more scalar numerics, all the SAME numeric
        //      type T (`min(3.0, 7.0, 2.0)`), returning T.
        //   2. Series: exactly one `Array<T>` (`min([3.0, 7.0, 2.0])`),
        //      returning the element type T.
        //
        // Strict-typing: every argument (or the array's element type) is
        // unified to a single `Numeric`-bounded var T, and the call returns
        // T. `min` over `Array<int>` returns `int`; over `Array<number>`
        // returns `number`. `int` and `number` never unify — a heterogeneous
        // `min(1, 2.0)` is rejected by the same-T unification, no silent
        // coercion. (The legacy fixed 2-arg form is the `actual_arity == 2`
        // scalar case, preserved.)
        if name == "min" || name == "max" {
            let origin = self
                .lookup_callable_origin_for_name(name)
                .unwrap_or(call_span);

            // Series form: a single Array<T> argument.
            if arg_types.len() == 1 {
                let resolved = self.solver.unifier().apply_substitutions(&arg_types[0]);
                if let Some(elem) = Self::min_max_array_element_type(&resolved) {
                    // Constrain the element type to Numeric and return it.
                    let bound = self.fresh_var();
                    self.push_constraint_with_origin(
                        elem.clone(),
                        Type::Constrained {
                            var: bound,
                            constraint: Box::new(TypeConstraint::ImplementsTrait {
                                trait_name: "Numeric".to_string(),
                            }),
                        },
                        origin,
                    );
                    return Ok(elem);
                }
                return Err(TypeError::ConstraintViolation(format!(
                    "Function '{}' over a single argument requires a numeric array (Array<int> or Array<number>)",
                    name
                )));
            }

            // Variadic form: two-or-more scalar numerics, all the same T.
            if arg_types.len() >= 2 {
                // Pin the common result type T to the FIRST argument's
                // (substitution-resolved) type, and unify every other argument
                // against it. Returning the resolved first-arg type (rather than
                // a bare fresh var) keeps the result CONCRETE for downstream
                // typed-opcode proof — `min(10, 5, 8) * 2` must type as
                // `int * int`, not leave an unresolved `?T` the binary-op
                // checker cannot prove. A mixed `min(1, 2.0)` still fails: the
                // second arg's `number` unifies against the first arg's `int`
                // and the solver rejects (int !~ number) — no silent coercion.
                let result = self.solver.unifier().apply_substitutions(&arg_types[0]);
                for arg_ty in arg_types.iter().skip(1) {
                    self.push_constraint_with_origin(arg_ty.clone(), result.clone(), origin);
                }
                // T must be Numeric.
                let bound = self.fresh_var();
                self.push_constraint_with_origin(
                    result.clone(),
                    Type::Constrained {
                        var: bound,
                        constraint: Box::new(TypeConstraint::ImplementsTrait {
                            trait_name: "Numeric".to_string(),
                        }),
                    },
                    origin,
                );
                return Ok(result);
            }

            return Err(TypeError::ConstraintViolation(format!(
                "Function '{}' expects at least 2 arguments or a single numeric array, got {}",
                name,
                arg_types.len()
            )));
        }

        // Look up function type after argument inference so argument errors
        // (e.g. unknown property access) surface even when callee is undefined.
        if let Some(result) = self.infer_comptime_builtin_call(name, &arg_types, call_span)? {
            return Ok(result);
        }
        let func_scheme = self
            .env
            .lookup(name)
            .ok_or_else(|| TypeError::UndefinedFunction(name.to_string()))?;

        // Instantiate with bounds to emit ImplementsTrait constraints for trait-bounded generics
        let (func_type, bound_constraints, default_substitutions) =
            func_scheme.instantiate_with_bounds(&mut self.type_var_gen);
        self.constraints.extend(bound_constraints);
        self.record_function_callsite(name, &arg_types);

        // v0.3.3 c4-4D — HOF callee-param-inference propagation.
        //
        // When a named function is passed as a value argument to another
        // function (`apply(double, 21)`), the standard callsite-union scheme
        // records `arg_types` against the OUTER callee (`apply`) only —
        // `record_function_callsite("apply", [Fn(...), int])`. The INNER
        // callee (`double`) gets no direct call site, so
        // `callsite_param_types["double"]` stays empty. The
        // `refine_numeric_params_post_callsite` last-resort default then
        // collapses double's parameter to `number`, and the bytecode
        // compiler emits `MulNumber` on call-supplied `int` bits, producing
        // a Float64 denormal at runtime (audit-04 §8 c4-4D root).
        //
        // The body of the outer callee constrains how its function-typed
        // parameter is called against its other parameters: the body of
        // `fn apply(f, x) { f(x) }` pushes the constraint
        // `Type::Variable(f_src) ~ Function { params: [Variable(x_src)], returns: _ }`
        // before the outer call site is processed (each function's body is
        // inferred eagerly in `infer_item`). Reading that constraint here
        // lets us derive a synthetic callsite for the inner callee:
        // "double's parameter[0] gets called with whatever the OUTER
        // callsite passed as the corresponding parameter of `apply`".
        //
        // Specifically: when args[i] is `Expr::Identifier(callee_name)` for
        // a known named function and `callable_param_source_vars[name][i]`
        // is bound to a `Function { params: [Variable(p_var)], returns: _ }`
        // shape via constraints, find which of `name`'s OTHER params has
        // source var `p_var` — call its index `j`. Then record a synthetic
        // callsite for `callee_name` with the arg type at position `j`.
        //
        // Only fires for HOF args that are bare named-function identifiers
        // (`Expr::Identifier`). Closures and lambdas use the closure-eager
        // refinement at `refine_callable_param_types_from_local_constraints`
        // (is_closure=true), which already collapses their numeric params
        // to `number` at body-inference time; they are out of scope here.
        //
        // Soundness: only the source vars are read, only existing constraints
        // are inspected (no new ones pushed), and the synthetic callsite
        // recording goes through the same `record_function_callsite` path
        // that direct call sites use — `refine_numeric_params_post_callsite`
        // then resolves the inner callee's parameter from the recorded
        // concrete type. No type kind is fabricated.
        self.propagate_hof_arg_callsites(name, args, &arg_types);

        // Wave 1a PART B (soundness): the CLOSURE-LITERAL analog of
        // `propagate_hof_arg_callsites`. When a closure literal is passed to an
        // unannotated callable param whose body invokes it on the outer
        // function's OWN params (`fn apply2(f, x, y) { f(x, y) }`,
        // `apply2(|a,b| a*b, 6, 7)`), the named-fn synthetic-callsite path does
        // not fire (the arg is a closure, not an `Expr::Identifier`), and the
        // call-shape constraint binds the arg occurrence into the per-call
        // INSTANCE var, never the closure's own param vars. The closure's
        // `Numeric`-bounded params then DEFER to the post-solve `number`
        // default — yielding `(number,number)->number` while the compiler seeds
        // the closure params from the SAME outer types as `int`. That divergence
        // is the t4/t5 unsoundness (static `number` result, runtime `int`).
        //
        // Fix: push DIRECT constraints `closure_param_var[k] ~
        // arg_types[outer_idx[k]]` so the solver resolves the closure's params
        // (and hence its body + return) to the EXACT outer-param types the call
        // site proved (`int`, never widened). The closure's return then flows
        // into the call result, so `acc + apply2(...)` types as `int + int`
        // (t4) and `let r: number = apply2(...)` correctly rejects (t5).
        //
        // Soundness: the closure's param vars are read off its inferred
        // `Type::Function` (no fabrication); the constraint targets are the
        // already-inferred outer arg types (`int` from the literals `6, 7`).
        // The solver UNIFIES — a conflicting later site is a genuine mismatch,
        // never a silent pick. `int`/`number` stay distinct.
        self.propagate_closure_arg_callsites(name, args, &arg_types);

        // v0.3.3 ref-param caller->param inference (sibling of the closure /
        // HOF caller->param propagation above). For a reference argument
        // `f(&x, …)`, the callee's reference parameter (`fn f(&p, …)`) carries
        // NO annotation and its body (`p = p + v`) imposes no concrete type
        // when both operands are still unresolved vars (the J3 `Add`-defer in
        // `infer_binary_op` pushes no Numeric bound). So the param stays a
        // `Type::Variable` that NEITHER the callsite-union fixpoint resolves in
        // time (the call-shape constraint binds the arg occurrence-var into the
        // per-call param INSTANCE var, not the body's SOURCE var) NOR the
        // numeric default touches (the param is not in
        // `callable_numeric_param_indices`). The runtime symptom is a wrong
        // result, because the unannotated reference param's slot kind is never
        // proven.
        //
        // The fix flows the caller's ref-target type into the param's SOURCE
        // var directly, BEFORE the constraint solver runs: pushing
        // `Variable(param_source_var) ~ ref_target_type` makes the solver unify
        // the param's body variable with the ref target's binding (e.g. the
        // `int` of `let mut total = 0`). The body's `Add` then types as
        // `AddInt`. This mirrors the closure caller->param inference (R4 /
        // ROOT-2): an unannotated callable param adopts the type the caller
        // supplies at the call site. `infer_expr(&x)` already inferred the
        // ref-target's type into `arg_types[i]` (the `Expr::Reference` arm
        // forwards to its inner expr).
        //
        // Multiple ref call sites each push their own target type at the SAME
        // source var, so the solver UNIFIES them: matching targets collapse to
        // one type; CONFLICTING targets (int at one site, number at another)
        // produce a genuine unification mismatch the solver rejects — the
        // annotation-required error the bug specifies (no silent pick, no value
        // widening, no fabricated kind).
        self.propagate_ref_arg_param_types(name, args, &arg_types);

        let origin = self
            .lookup_callable_origin_for_name(name)
            .unwrap_or(call_span);

        // Unknown callee types (e.g. unannotated higher-order params) are
        // constrained to callable shapes from this call site.
        if !matches!(
            &func_type,
            Type::Function { .. } | Type::Concrete(TypeAnnotation::Function { .. })
        ) {
            if matches!(&func_type, Type::Variable(_) | Type::Constrained { .. }) {
                let result_type = self.fresh_type_var();
                let expected_func_type =
                    BuiltinTypes::function(arg_types.clone(), result_type.clone());
                self.push_constraint_with_origin(func_type, expected_func_type, origin);
                return Ok(result_type);
            }
            return Err(TypeError::ConstraintViolation(format!(
                "'{}' is not callable",
                name
            )));
        }
        let (params, returns) = match &func_type {
            Type::Function { params, returns } => (params.clone(), returns.as_ref().clone()),
            Type::Concrete(TypeAnnotation::Function {
                params: concrete_params,
                returns: concrete_returns,
            }) => {
                // Decode tyvar markers back into real Variables so call-arg
                // substitution can resolve them — e.g. a closure-valued object field
                // `{ greet: |name| ... }` stores greet's param as an unresolved
                // `tyvar:Tn` marker; `obj.greet("World")` must unify "World" with Tn,
                // not against the literal marker string.
                let params: Vec<Type> = concrete_params
                    .iter()
                    .map(|p| match annotation_as_tyvar(&p.type_annotation) {
                        Some(var) => Type::Variable(var),
                        None => Type::Concrete(p.type_annotation.clone()),
                    })
                    .collect();
                let returns = match annotation_as_tyvar(concrete_returns) {
                    Some(var) => Type::Variable(var),
                    None => Type::Concrete(*concrete_returns.clone()),
                };
                (params, returns)
            }
            _ => unreachable!("non-function callees are handled above"),
        };

        // GAP-2 param-side normalization (v0.3.3 B4, references slice D2): an
        // explicitly `&T`-annotated parameter (`fn shift(p: &Point)`) carries a
        // `Borrow { inner: T }` annotation in the callee's signature, but the
        // by-reference ARGUMENT flow above (the GAP-2 boundary unwrap at the
        // `Expr::Reference { .. }` arg) reduces the matching `&pt` argument to its
        // REFERENT type `T`. Without unwrapping the param side too, the call-shape
        // constraint compares `(&Point) -> R` (declared) against `(Point) -> R`
        // (call) and rejects with "(&Point) is not compatible with (Point)". The
        // sigil form `fn shift(&p)` records the param as bare `T` + an
        // `is_reference` flag and already matches; this makes the typed form
        // `fn shift(p: &Point)` produce the SAME `(Point) -> R` signature so both
        // book-documented ref-param forms type-check identically. The
        // reference-ness/mutability is tracked by the param flags, not the param
        // type — same contract the arg-side unwrap relies on. NOT a coercion: the
        // inner referent annotation is forwarded verbatim (`&Point` -> `Point`,
        // `&int` -> `int`); `int` and `number` never meet here.
        let had_borrow_param = params
            .iter()
            .any(|p| matches!(p, Type::Concrete(TypeAnnotation::Borrow { .. })));
        let params: Vec<Type> = params
            .into_iter()
            .map(|p| match p {
                Type::Concrete(TypeAnnotation::Borrow { inner, .. }) => Type::Concrete(*inner),
                other => other,
            })
            .collect();
        // Rebuild the callee's function type from the referent-normalized params
        // so the LHS of the call-shape constraint pushed below is `(Point) -> R`,
        // matching the referent-normalized RHS. Without this the LHS keeps the
        // `&Point` param and the constraint rejects (`(&Point) !~ (Point)`).
        let func_type = if had_borrow_param {
            BuiltinTypes::function(params.clone(), returns.clone())
        } else {
            func_type
        };

        let total_arity = params.len();
        let default_flags = self
            .callable_param_defaults
            .get(name)
            .cloned()
            .unwrap_or_else(|| vec![false; total_arity]);
        let required_arity = default_flags
            .iter()
            .position(|has_default| *has_default)
            .unwrap_or(total_arity);
        let actual_arity = arg_types.len();

        if actual_arity < required_arity || actual_arity > total_arity {
            return Err(TypeError::ConstraintViolation(format!(
                "Function '{}' expects between {} and {} arguments, got {}",
                name, required_arity, total_arity, actual_arity
            )));
        }

        let mut substitutions: std::collections::HashMap<TypeVar, Type> =
            std::collections::HashMap::new();
        for (i, (param_ty, arg_ty)) in params.iter().zip(arg_types.iter()).enumerate() {
            // ROOT-B: a bare int LITERAL payload of an `Ok`/`Err`/`Some`
            // constructor DEFERS to its fresh payload var instead of pinning it
            // to `int`. Skipping the `T -> int` substitution here (and the
            // matching call-shape constraint below) leaves `T` unresolved, so
            // the constructor's `Result<T>` / `Option<T>` later unifies with the
            // function's `Result<number>` / `Option<number>` return carrier
            // (T = number) rather than conflicting as `Result<int> !~
            // Result<number>`. LITERALS ONLY — a non-literal int VALUE keeps its
            // normal pinning (no value widening).
            if args.get(i).is_some_and(|arg| {
                Self::constructor_literal_payload_defers_to_var(name, arg, param_ty)
            }) {
                // Record the (unresolved) payload var so the post-solve
                // `default_unresolved_constructor_literal_payload_vars` pass can
                // bind it to `int` if no carrier ever resolves it.
                if let Type::Variable(var) = param_ty {
                    self.deferred_constructor_literal_payload_vars
                        .insert(var.clone());
                }
                continue;
            }
            Self::collect_call_substitutions(param_ty, arg_ty, &mut substitutions);
        }

        // Generic defaults apply only for unresolved type variables.
        for (var, default_type) in default_substitutions {
            if substitutions.contains_key(&var) {
                continue;
            }
            substitutions.insert(var.clone(), default_type.clone());
            self.push_constraint_with_origin(Type::Variable(var), default_type, origin);
        }

        let inferred_result_type = Self::apply_substitutions_to_type(&returns, &substitutions);
        self.record_function_callsite_return(name, inferred_result_type.clone());

        // Numeric-conversion §4 literal adoption (call-argument context): a bare
        // integer literal argument adopts the corresponding parameter's concrete
        // numeric type when its value losslessly fits, so `abs(-42)` (param
        // `number`) and `(|y| y * 2.0)(3)` type-check without re-introducing the
        // deleted implicit int->number value widening. A NON-literal int
        // argument to a `number` parameter still rejects (§5 value-level
        // invariant). The adoption substitutes the param's resolved concrete
        // type for the literal's inferred type in the call-shape constraint.
        let mut expected_param_types: Vec<Type> = Vec::with_capacity(arg_types.len());
        for (i, arg_ty) in arg_types.iter().enumerate() {
            let resolved_param = params
                .get(i)
                .map(|p| Self::apply_substitutions_to_type(p, &substitutions));
            // ROOT-B: for a deferred `Ok`/`Err`/`Some` literal payload, push the
            // (unresolved) payload param itself as the expected type so the
            // call-shape constraint stays `T ~ T` (a no-op) rather than pinning
            // `T ~ int`. The literal defers to `T`; the return carrier resolves
            // it. Matches the substitution skip above.
            if let (Some(param_ty), Some(arg_expr)) = (params.get(i), args.get(i)) {
                if Self::constructor_literal_payload_defers_to_var(name, arg_expr, param_ty) {
                    expected_param_types.push(resolved_param.unwrap_or_else(|| param_ty.clone()));
                    continue;
                }
            }
            let adopted = match (&resolved_param, args.get(i)) {
                (Some(param_ty), Some(arg_expr)) => {
                    Self::adopt_int_literal_in_context(arg_expr, param_ty)
                }
                _ => None,
            };
            expected_param_types.push(adopted.unwrap_or_else(|| arg_ty.clone()));
        }
        for param_ty in params.iter().skip(actual_arity) {
            expected_param_types.push(Self::apply_substitutions_to_type(param_ty, &substitutions));
        }
        let expected_func_type =
            BuiltinTypes::function(expected_param_types, inferred_result_type.clone());
        self.push_constraint_with_origin(func_type, expected_func_type, origin);

        // ADR-006 §2.7.30 (GapA — value-position auto-deref): a `-> &T` callee
        // returns a reference value. In a VALUE-expecting context (arithmetic
        // operand, comparison, `print` arg, a `-> T` return), the call result is
        // READ THROUGH the reference to its referent `T`. This is a fundamental
        // reference-read — NOT a numeric coercion: `&int` derefs to `int`, never
        // to `number` (`Borrow` is a distinct constructor; the inner is forwarded
        // verbatim). It mirrors the bytecode-side auto-deref wired at the
        // `function_declares_borrow_return` call sites (which emit `DerefLoad` in
        // value position via `auto_deref_last_expr_result_if_needed`).
        //
        // Context-sensitivity: the inferred return type is derefed here so the
        // call flows as `T`. The remaining ref-EXPECTING position is a literal
        // `&x` passed to a ref-typed param — that is an `Expr::Reference`
        // argument handled by the unwrap above (GAP-2 boundary), never a
        // `FunctionCall` result, so it is untouched. The constraint pushed above
        // still carries the `&T` return so the callee's own `-> &T` annotation
        // unification (Borrow-vs-Borrow) is unaffected.
        if let Type::Concrete(TypeAnnotation::Borrow { inner, .. }) = self
            .solver
            .unifier()
            .apply_substitutions(&inferred_result_type)
        {
            return Ok(Type::Concrete(*inner));
        }

        // HOF return-type aliasing at the CALL site (the sg2 root, int/number
        // guard). When `name` is a wrapper whose return value is precisely the
        // result of invoking one of its own fn-typed params in tail position
        // (`fn apply2(f, x, y) { f(x, y) }`, recorded in
        // `callable_return_from_fn_param`), the call's result type IS the
        // GENUINE return type of the function passed at that param position
        // (`apply2(|a,b| a*b, 6, 7)` returns `int`, the closure's return). The
        // HM scheme instantiates apply2's stored return var fresh here, so
        // without this the result stays an unconstrained var that a USE site
        // (`n: number; n + apply2(…)`) would unify against its OWN demanded type
        // — silently accepting a `number + int` the bytecode emitter then widens
        // (the deleted int->number coercion; int and number do NOT unify). We
        // pin the result to the genuine `R`, so a conflicting use rejects at
        // solve time and an agreeing use (`acc: int; acc + apply2(…)`) lowers to
        // the correct typed opcode. Fires only when the arg's function type is
        // fully concrete (int stays int, number stays number); an unresolved arg
        // return leaves the result as-is (no fabrication).
        if let Some(&fn_param_idx) = self.callable_return_from_fn_param.get(name) {
            if let Some(arg_ty) = arg_types.get(fn_param_idx) {
                let resolved_arg = self.solver.unifier().apply_substitutions(arg_ty);
                let arg_return = match &resolved_arg {
                    Type::Function { returns, .. } => Some((**returns).clone()),
                    Type::Concrete(TypeAnnotation::Function { returns, .. }) => {
                        match annotation_as_tyvar(returns) {
                            Some(_) => None,
                            None => Some(Type::Concrete((**returns).clone())),
                        }
                    }
                    _ => None,
                };
                if let Some(genuine) = arg_return {
                    let genuine = self.solver.unifier().apply_substitutions(&genuine);
                    if !matches!(genuine, Type::Variable(_)) {
                        // Unify (don't blindly replace): an agreeing instantiated
                        // result var binds to `genuine`; a conflict surfaces.
                        self.push_constraint_with_origin(
                            inferred_result_type.clone(),
                            genuine.clone(),
                            origin,
                        );
                        return Ok(genuine);
                    }
                    // The arg's return is still a variable (a closure literal
                    // whose body return resolves only after the solver pins its
                    // params from the call-site args). Link the call result to
                    // that SAME variable so both resolve together to the genuine
                    // type (`int` for `|a,b| a*b` over int args) — keeping the
                    // result tied to the closure's actual return rather than a
                    // disconnected fresh instantiation var. No type is fabricated:
                    // if the closure return never resolves, the result stays a
                    // variable exactly as before.
                    self.push_constraint_with_origin(
                        inferred_result_type.clone(),
                        genuine.clone(),
                        origin,
                    );
                    return Ok(genuine);
                }
            }
        }
        if let Some(&fn_param_idx) = self.callable_array_return_from_fn_param.get(name) {
            if let Some(arg_ty) = arg_types.get(fn_param_idx) {
                let resolved_arg = self.solver.unifier().apply_substitutions(arg_ty);
                let arg_return = match &resolved_arg {
                    Type::Function { returns, .. } => Some((**returns).clone()),
                    Type::Concrete(TypeAnnotation::Function { returns, .. }) => {
                        match annotation_as_tyvar(returns) {
                            Some(_) => None,
                            None => Some(Type::Concrete((**returns).clone())),
                        }
                    }
                    _ => None,
                };
                if let Some(genuine) = arg_return {
                    let array_return = BuiltinTypes::array(genuine);
                    self.push_constraint_with_origin(
                        inferred_result_type.clone(),
                        array_return.clone(),
                        origin,
                    );
                    return Ok(array_return);
                }
            }
        }

        Ok(inferred_result_type)
    }

    /// R4 bidirectional closure-arg support: peek the callee's parameter list
    /// and return, per argument position, the expected CONCRETE function type
    /// if (a) the argument at that position is a closure literal and (b) the
    /// callee's parameter is a fully-concrete `Function` shape.
    ///
    /// Returns `None` (the whole vector) when the callee is not a known
    /// named function in the environment, so builtins and value-call sites are
    /// completely untouched. Per-slot `None` means "infer this arg normally".
    ///
    /// The scheme is only PEEKED (no instantiation, no constraint emission, no
    /// state mutation). A parameter that still mentions a type variable (a
    /// generic param, e.g. `fn map<T,U>(f: (T) => U, …)`) yields `None` for its
    /// slot — we only propagate a closed concrete function type, never a
    /// partially-resolved one. Method-call closure inference already handles
    /// the generic case via its own bidirectional path.
    fn callee_concrete_param_fn_types(
        &self,
        name: &str,
        args: &[Expr],
    ) -> Option<Vec<Option<Type>>> {
        // Only fires when at least one arg is a closure literal — avoids the
        // env lookup on the overwhelmingly common closure-free call.
        if !args.iter().any(|a| matches!(a, Expr::FunctionExpr { .. })) {
            return None;
        }
        let scheme = self.env.lookup(name)?;
        match &scheme.ty {
            Type::Function { params, .. } => Some(
                args.iter()
                    .enumerate()
                    .map(|(i, arg)| {
                        if !matches!(arg, Expr::FunctionExpr { .. }) {
                            return None;
                        }
                        params.get(i).and_then(|param_ty| {
                            if matches!(param_ty, Type::Function { .. })
                                && !Self::type_contains_variable(param_ty)
                            {
                                Some(param_ty.clone())
                            } else {
                                None
                            }
                        })
                    })
                    .collect(),
            ),
            Type::Concrete(TypeAnnotation::Function { params, .. }) => Some(
                args.iter()
                    .enumerate()
                    .map(|(i, arg)| {
                        if !matches!(arg, Expr::FunctionExpr { .. }) {
                            return None;
                        }
                        params.get(i).and_then(|p| {
                            let ty = Type::Concrete(p.type_annotation.clone());
                            if matches!(&ty, Type::Concrete(TypeAnnotation::Function { .. }))
                                && !Self::type_contains_variable(&ty)
                            {
                                Some(ty)
                            } else {
                                None
                            }
                        })
                    })
                    .collect(),
            ),
            _ => None,
        }
    }

    /// True if the type mentions any `Type::Variable` / `Type::Constrained`
    /// anywhere in its structure, or carries an encoded `tyvar:Tn` annotation
    /// marker. Used to gate R4 closure-arg propagation to fully-closed concrete
    /// function types only.
    fn type_contains_variable(ty: &Type) -> bool {
        match ty {
            Type::Variable(_) | Type::Constrained { .. } => true,
            Type::Concrete(ann) => Self::annotation_contains_tyvar(ann),
            Type::Generic { base, args } => {
                Self::type_contains_variable(base) || args.iter().any(Self::type_contains_variable)
            }
            Type::Function { params, returns } => {
                params.iter().any(Self::type_contains_variable)
                    || Self::type_contains_variable(returns)
            }
        }
    }

    /// True if a `TypeAnnotation` carries an encoded `tyvar:Tn` marker (the
    /// closure-valued-field representation) anywhere in its structure.
    fn annotation_contains_tyvar(ann: &TypeAnnotation) -> bool {
        if annotation_as_tyvar(ann).is_some() {
            return true;
        }
        match ann {
            TypeAnnotation::Array(inner) => Self::annotation_contains_tyvar(inner),
            TypeAnnotation::Function { params, returns } => {
                params
                    .iter()
                    .any(|p| Self::annotation_contains_tyvar(&p.type_annotation))
                    || Self::annotation_contains_tyvar(returns)
            }
            TypeAnnotation::Generic { args, .. } => {
                args.iter().any(Self::annotation_contains_tyvar)
            }
            _ => false,
        }
    }

    /// Infer element type from iterator type
    pub(crate) fn infer_iterator_element_type(&mut self, iter_type: &Type) -> TypeResult<Type> {
        match iter_type {
            Type::Concrete(TypeAnnotation::Array(elem_type)) => {
                Ok(Type::Concrete(*elem_type.clone()))
            }
            // Iterating Table<T> produces Row<T>
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Table" && args.len() == 1 =>
            {
                Ok(Type::Concrete(TypeAnnotation::Generic {
                    name: "Row".into(),
                    args: args.clone(),
                }))
            }
            // ROOT-1: array-literal + map/split/collect outputs carry the
            // inference-tier `Type::Generic { base: Array/Vec, args: [elem] }`
            // carrier (var-preserving), distinct from the
            // `Concrete(TypeAnnotation::Array)` carrier handled above. Mirror
            // the Concrete arm: iterating an `Array<T>`/`Vec<T>` yields `T`.
            // PER-SITE-ARM (no carrier unification — see ADR-006).
            Type::Generic { base, args } if !args.is_empty() => {
                let base_name = match base.as_ref() {
                    Type::Concrete(ann) => ann.as_type_name_str().map(str::to_string),
                    _ => None,
                };
                match base_name.as_deref() {
                    Some("Array") | Some("Vec") => Ok(args[0].clone()),
                    _ => Ok(self.fresh_type_var()),
                }
            }
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                // Special case: "rows" iterates to produce "row" elements
                if name == "rows" {
                    Ok(BuiltinTypes::row())
                } else if self.env.lookup_record_schema(name).is_some() {
                    // If this is a registered schema type, it likely iterates to itself
                    Ok(Type::Concrete(TypeAnnotation::Basic(name.clone())))
                } else {
                    // For unknown iterators, return a fresh type variable
                    Ok(self.fresh_type_var())
                }
            }
            _ => {
                // For unknown iterators, return a fresh type variable
                Ok(self.fresh_type_var())
            }
        }
    }

    fn collect_call_substitutions(
        expected: &Type,
        actual: &Type,
        substitutions: &mut std::collections::HashMap<TypeVar, Type>,
    ) {
        match expected {
            Type::Variable(var) => {
                substitutions
                    .entry(var.clone())
                    .or_insert_with(|| actual.clone());
            }
            Type::Constrained { var, .. } => {
                substitutions
                    .entry(var.clone())
                    .or_insert_with(|| actual.clone());
            }
            Type::Generic {
                base: expected_base,
                args: expected_args,
            } => {
                if let Type::Generic {
                    base: actual_base,
                    args: actual_args,
                } = actual
                {
                    Self::collect_call_substitutions(expected_base, actual_base, substitutions);
                    for (exp_arg, act_arg) in expected_args.iter().zip(actual_args.iter()) {
                        Self::collect_call_substitutions(exp_arg, act_arg, substitutions);
                    }
                }
            }
            Type::Function {
                params: expected_params,
                returns: expected_returns,
            } => {
                if let Type::Function {
                    params: actual_params,
                    returns: actual_returns,
                } = actual
                {
                    for (exp_param, act_param) in expected_params.iter().zip(actual_params.iter()) {
                        Self::collect_call_substitutions(exp_param, act_param, substitutions);
                    }
                    Self::collect_call_substitutions(
                        expected_returns,
                        actual_returns,
                        substitutions,
                    );
                }
            }
            Type::Concrete(_) => {}
        }
    }

    /// v0.3.3 c4-4D: HOF callee-param-inference propagation.
    ///
    /// At the call site of an outer function (`apply(double, 21)`), find
    /// each argument that is a bare named-function identifier and propagate
    /// a synthetic callsite for that inner callee using the outer body's
    /// call-shape constraint.
    ///
    /// See the comment block at the call site in `infer_function_call` for
    /// the full rationale + soundness argument. The mechanism in three
    /// steps:
    ///
    ///  1. Locate `args[i]` of the form `Expr::Identifier(callee_name)`
    ///     where `callee_name` resolves to a known named function (i.e.
    ///     has its own callable_param_source_vars entry).
    ///  2. Look up the outer function's i-th parameter source-var. Scan
    ///     `self.constraints` for `Variable(outer_param_i_src) ~
    ///     Function { params: [...], returns: _ }` (the body-imposed
    ///     call-shape constraint pushed by `infer_function_call` when the
    ///     outer body invokes its own parameter).
    ///  3. For each parameter slot in that call-shape, if the slot is
    ///     itself a bare `Variable(outer_param_j_src)`, find which of the
    ///     outer's other parameters has source var `outer_param_j_src` —
    ///     index `j`. Use `arg_types[j]` as the synthetic callsite arg for
    ///     `callee_name` at the same parameter position.
    ///
    /// Stays in inference-engine state: no new constraints are pushed; no
    /// type kind is fabricated. The synthetic callsite goes through the
    /// existing `record_function_callsite` path, so downstream widening +
    /// `refine_numeric_params_post_callsite` resolve the inner callee's
    /// parameter from the recorded concrete type exactly as if a direct
    /// `double(21)` call site had existed.
    pub(crate) fn propagate_hof_arg_callsites(
        &mut self,
        outer_name: &str,
        args: &[Expr],
        arg_types: &[Type],
    ) {
        // Read the outer function's parameter source-vars. An annotated
        // outer parameter has `None` here; we need an unannotated
        // function-typed slot whose body imposed a call-shape constraint,
        // so `None` slots are skipped.
        let Some(outer_source_vars) = self.callable_param_source_vars.get(outer_name).cloned()
        else {
            return;
        };

        // Build outer_param_j_src → j lookup for resolving "this slot
        // references the outer's parameter j" once we read the call-shape
        // constraint.
        let outer_var_to_index: HashMap<TypeVar, usize> = outer_source_vars
            .iter()
            .enumerate()
            .filter_map(|(j, sv)| sv.as_ref().map(|v| (v.clone(), j)))
            .collect();

        for (i, arg) in args.iter().enumerate() {
            let Expr::Identifier(callee_name, _) = arg else {
                continue;
            };
            // The argument must reference a known named function whose
            // callsite-union scheme can consume the synthetic record.
            // (Closures/lambdas use the eager closure-collapse path at
            // body-inference time; they're out of scope.)
            if !self.callable_param_source_vars.contains_key(callee_name) {
                continue;
            }
            // The outer's i-th source-var must be the unannotated kind
            // whose body imposed a Fn-shape constraint.
            let Some(Some(outer_param_i_src)) = outer_source_vars.get(i) else {
                continue;
            };

            // Find a constraint `Variable(outer_param_i_src) ~
            // Function { params: [...], returns: _ }` in either direction.
            // Multiple such constraints can exist (e.g. if the outer body
            // calls `f` more than once); record from each.
            let outer_param_i_src = outer_param_i_src.clone();
            let snapshot: Vec<(Type, Type)> = self.constraints.clone();
            for (lhs, rhs) in &snapshot {
                let body_params = match (lhs, rhs) {
                    (Type::Variable(v), Type::Function { params, .. })
                    | (Type::Function { params, .. }, Type::Variable(v))
                        if v == &outer_param_i_src =>
                    {
                        params
                    }
                    _ => continue,
                };

                // Map each body-call parameter slot back to one of the
                // outer's parameter source vars (the call-shape `f(x)`
                // where `x` is outer param j tells us "callee receives
                // outer's param j"). Then take the synthetic arg type
                // from `arg_types[j]`.
                let mut synthetic_arg_types: Vec<Type> = Vec::with_capacity(body_params.len());
                let mut ok = true;
                for slot in body_params {
                    let Type::Variable(slot_var) = slot else {
                        ok = false;
                        break;
                    };
                    let Some(&j) = outer_var_to_index.get(slot_var) else {
                        ok = false;
                        break;
                    };
                    let Some(arg_ty) = arg_types.get(j) else {
                        ok = false;
                        break;
                    };
                    synthetic_arg_types.push(arg_ty.clone());
                }
                if !ok {
                    continue;
                }

                // The synthetic callsite goes through the normal record
                // path — `apply_callsite_unions` + `refine_numeric_params_post_callsite`
                // pick it up exactly as if `callee_name(arg_ty)` had been
                // written directly.
                self.record_function_callsite(callee_name, &synthetic_arg_types);
            }
        }
    }

    /// Wave 1a PART B (soundness): closure-literal analog of
    /// `propagate_hof_arg_callsites`. When `args[i]` is a CLOSURE LITERAL passed
    /// to an unannotated callable param of `outer_name` whose body invokes it on
    /// the outer's OWN params (`fn apply2(f, x, y) { f(x, y) }`), push direct
    /// constraints binding the closure's k-th param var to the OUTER call's arg
    /// type at the position the body passes (`f(x, y)` ⇒ closure param 0 ~
    /// arg_types[x_index], param 1 ~ arg_types[y_index]). This resolves the
    /// closure's `Numeric`-bounded params to the EXACT call-site types BEFORE
    /// the post-solve `number` default fires, so the closure body + return type
    /// (and thus the call result) match the compiler's closure-param seeding.
    ///
    /// Three-step mechanism (mirrors `propagate_hof_arg_callsites`):
    ///  1. `args[i]` is `Expr::FunctionExpr` and `arg_types[i]` is a resolved
    ///     `Type::Function` exposing the closure's param vars.
    ///  2. The outer's i-th param source var has a body call-shape constraint
    ///     `Variable(outer_param_i_src) ~ Function { params: [Variable(outer_param_j_src), …] }`.
    ///  3. Each body-call slot maps to an outer param index `j`; push
    ///     `closure_param_var[k] ~ arg_types[j]`.
    ///
    /// Soundness: closure param vars come from the inferred arg type (no
    /// fabrication); targets are already-inferred outer arg types. The solver
    /// UNIFIES — conflicting sites mismatch, never silently picked. No int VALUE
    /// is widened; `int`/`number` stay distinct.
    pub(crate) fn propagate_closure_arg_callsites(
        &mut self,
        outer_name: &str,
        args: &[Expr],
        arg_types: &[Type],
    ) {
        let Some(outer_source_vars) = self.callable_param_source_vars.get(outer_name).cloned()
        else {
            return;
        };
        let outer_var_to_index: HashMap<TypeVar, usize> = outer_source_vars
            .iter()
            .enumerate()
            .filter_map(|(j, sv)| sv.as_ref().map(|v| (v.clone(), j)))
            .collect();

        // Collect the constraints to push after the immutable borrows end.
        let mut to_push: Vec<(Type, Type)> = Vec::new();

        for (i, arg) in args.iter().enumerate() {
            if !matches!(arg, Expr::FunctionExpr { .. }) {
                continue;
            }
            // The closure's inferred param vars (its `Type::Function`).
            let Some(Type::Function {
                params: closure_params,
                ..
            }) = arg_types.get(i)
            else {
                continue;
            };
            // The outer's i-th param must be an unannotated callable whose body
            // imposed a call-shape constraint.
            let Some(Some(outer_param_i_src)) = outer_source_vars.get(i) else {
                continue;
            };
            let outer_param_i_src = outer_param_i_src.clone();

            let snapshot: Vec<(Type, Type)> = self.constraints.clone();
            for (lhs, rhs) in &snapshot {
                let body_params = match (lhs, rhs) {
                    (Type::Variable(v), Type::Function { params, .. })
                    | (Type::Function { params, .. }, Type::Variable(v))
                        if v == &outer_param_i_src =>
                    {
                        params
                    }
                    _ => continue,
                };
                // Arity must match the closure's param count exactly.
                if body_params.len() != closure_params.len() {
                    continue;
                }
                // Map each body-call slot to an outer param index, then bind the
                // closure's k-th param var to that outer arg type.
                let mut mapped: Vec<(usize, Type)> = Vec::with_capacity(body_params.len());
                let mut ok = true;
                for (k, slot) in body_params.iter().enumerate() {
                    let Type::Variable(slot_var) = slot else {
                        ok = false;
                        break;
                    };
                    let Some(&j) = outer_var_to_index.get(slot_var) else {
                        ok = false;
                        break;
                    };
                    let Some(outer_arg_ty) = arg_types.get(j) else {
                        ok = false;
                        break;
                    };
                    mapped.push((k, outer_arg_ty.clone()));
                }
                if !ok {
                    continue;
                }
                for (k, outer_arg_ty) in mapped {
                    // Only bind closure param SLOTS that are still bare vars
                    // (the deferred Numeric params). An annotated closure param
                    // is already a concrete type and must not be overwritten.
                    if let Some(Type::Variable(_)) = closure_params.get(k) {
                        to_push.push((closure_params[k].clone(), outer_arg_ty));
                    }
                }
            }
        }

        let origin = self
            .lookup_callable_origin_for_name(outer_name)
            .unwrap_or_else(Span::default);
        for (lhs, rhs) in to_push {
            self.push_constraint_with_origin(lhs, rhs, origin);
        }
    }

    /// v0.3.3 ref-param caller->param inference. For a call `f(&x, …)` where
    /// argument `i` is a reference expression and the callee's parameter `i` is
    /// an UNANNOTATED parameter (source var present), push a direct constraint
    /// unifying the parameter's BODY source variable with the ref-target's
    /// already-inferred type (`arg_types[i]`).
    ///
    /// This is the reference sibling of the closure caller->param inference (R4
    /// / ROOT-2): an unannotated reference parameter adopts the type the caller
    /// supplies through the reference, the same way `apply(|x| …, 21)` binds the
    /// closure's `x`. Without it, the call-shape constraint binds only the
    /// per-call param INSTANCE var (a fresh instantiation), never the SOURCE var
    /// the body uses, so a body like `p = p + v` (whose `Add` defers under J3
    /// when both operands are unresolved vars) leaves the param a bare
    /// `Type::Variable` whose slot kind is never proven — the wrong-result bug.
    ///
    /// Constraining the source var directly lets the constraint solver (which
    /// runs after all items are inferred) propagate the ref-target's concrete
    /// type (`int` for `let mut total = 0`) into the body; the body's `Add` then
    /// types as `AddInt`. Each ref call site pushes its own constraint at the
    /// SAME source var, so the solver UNIFIES them: matching targets collapse;
    /// CONFLICTING targets (int at one site, number at another) are a genuine
    /// unification mismatch the solver rejects — the annotation-required error
    /// the bug specifies, not a silent pick. No new opcode, no fabricated kind,
    /// no value widening: the constraint is the same `(Type, Type)` shape every
    /// other call-arg uses.
    pub(crate) fn propagate_ref_arg_param_types(
        &mut self,
        callee_name: &str,
        args: &[Expr],
        arg_types: &[Type],
    ) {
        // Only named functions whose parameter source vars are tracked
        // participate. Annotated params have a `None` source-var slot and are
        // skipped — their type is already fixed and the normal call-arg
        // constraint already validates the ref target against the annotation.
        let Some(source_vars) = self.callable_param_source_vars.get(callee_name).cloned() else {
            return;
        };

        let origin = self.lookup_callable_origin_for_name(callee_name);
        for (i, arg) in args.iter().enumerate() {
            // Only reference arguments. A plain `f(x)` arg already flows through
            // the call-shape constraint normally.
            if !matches!(arg, Expr::Reference { .. }) {
                continue;
            }
            // The callee's i-th parameter must be an unannotated (source-var
            // present) slot.
            let Some(Some(param_src)) = source_vars.get(i) else {
                continue;
            };
            let Some(target_ty) = arg_types.get(i) else {
                continue;
            };
            // Skip a vacuous self-constraint: when the ref target is itself the
            // param's source variable (a forwarded `&p` of an outer reference
            // param), the constraint adds nothing. Such transitive chains
            // resolve via the other call site's concrete target.
            if let Type::Variable(tv) = target_ty {
                if tv == param_src {
                    continue;
                }
            }
            // Scope this inference to the SCALAR accumulator case it exists for
            // (`fn add_to(&sum, val) { sum = sum + val }` — `sum`'s slot kind
            // must be proven so the body binop emits `AddInt`/`AddNumber`). A
            // CONCRETE non-numeric ref target — most importantly a nominal
            // struct `Reference("Point")` for `fn get_x(&obj) { obj.x }` called
            // as `get_x(&p)` — must NOT be unified into the param var: doing so
            // freezes the param to the nominal name and the body's `obj.x`
            // field access then fails to resolve through the struct's
            // structural object shape ("Reference(Point) cannot have fields").
            // Those params don't need this inference at all — their
            // field/index/method resolution already works with the param left
            // as a fresh var. A still-unresolved type VARIABLE target is kept:
            // it is exactly the `&total` (`let mut total = 0`) accumulator whose
            // var only chases to `int` later via the constraint solver, and the
            // constraint is what links the param's source var into that chain.
            // No silent numeric pick, no widening: a concrete numeric stays
            // exact, a var stays linked, a concrete non-numeric is left to the
            // existing (correct) resolution path.
            if matches!(target_ty, Type::Concrete(_))
                && Self::concrete_numeric_type_name(target_ty).is_none()
            {
                continue;
            }
            let param_var_ty = Type::Variable(param_src.clone());
            match origin {
                Some(span) => {
                    self.push_constraint_with_origin(param_var_ty, target_ty.clone(), span)
                }
                None => self.constraints.push((param_var_ty, target_ty.clone())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_engine() -> TypeInferenceEngine {
        TypeInferenceEngine::new()
    }

    fn table_type(inner: &str) -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Table".into(),
            args: vec![TypeAnnotation::Basic(inner.to_string())],
        })
    }

    fn row_type(inner: &str) -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Row".into(),
            args: vec![TypeAnnotation::Basic(inner.to_string())],
        })
    }

    #[test]
    fn test_table_iteration_produces_row() {
        let mut engine = make_engine();
        let table = table_type("Candle");
        let element = engine.infer_iterator_element_type(&table).unwrap();

        // Iterating Table<Candle> should produce Row<Candle>
        match element {
            Type::Concrete(TypeAnnotation::Generic { name, args }) => {
                assert_eq!(name, "Row");
                assert_eq!(args.len(), 1);
                assert!(matches!(&args[0], TypeAnnotation::Basic(n) if n == "Candle"));
            }
            other => panic!("expected Row<Candle>, got {:?}", other),
        }
    }

    #[test]
    fn test_row_index_access_rejected() {
        let mut engine = make_engine();
        let row = row_type("Candle");
        let index = BuiltinTypes::string();

        // Dynamic index access on Row<T> should produce a type error
        let result = engine.infer_index_access(&row, &index);
        assert!(result.is_err());
    }

    #[test]
    fn test_row_property_access_falls_through() {
        let mut engine = make_engine();
        let row = row_type("Candle");

        // Property access on Row<T> should attempt to resolve on T
        // Since "Candle" isn't registered as a record schema, it falls through
        // to constraint-based inference (returns a fresh type variable)
        let result = engine.infer_property_access(&row, "open");
        assert!(result.is_ok());
    }

    // WF-4 index-type (reliableonly_strict_bypass class): an array index MUST
    // be proven `int` at compile time. `int` and `number` are SEPARATE families
    // and do NOT unify; a `number`/float/decimal index silently reinterpreted as
    // an `i64` index is the top strict-typing hole. These regressions lock the
    // strict behavior in at the inference layer.

    fn int_array() -> Type {
        Type::Concrete(TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
            "int".to_string(),
        ))))
    }

    #[test]
    fn test_array_number_index_is_compile_error() {
        let mut engine = make_engine();
        let arr = int_array();
        let result = engine.infer_index_access(&arr, &BuiltinTypes::number());
        assert!(
            result.is_err(),
            "arr[n: number] must be a compile error (no implicit number->int coercion), got {:?}",
            result
        );
    }

    #[test]
    fn test_array_decimal_index_is_compile_error() {
        let mut engine = make_engine();
        let arr = int_array();
        let decimal = Type::Concrete(TypeAnnotation::Basic("decimal".to_string()));
        let result = engine.infer_index_access(&arr, &decimal);
        assert!(
            result.is_err(),
            "arr[d: decimal] must be a compile error, got {:?}",
            result
        );
    }

    #[test]
    fn test_array_int_index_type_checks() {
        let mut engine = make_engine();
        let arr = int_array();
        let result = engine.infer_index_access(&arr, &BuiltinTypes::integer());
        assert!(
            result.is_ok(),
            "arr[i: int] must type-check, got {:?}",
            result
        );
        // and it yields the element type
        assert!(matches!(
            result.unwrap(),
            Type::Concrete(TypeAnnotation::Basic(ref n)) if n == "int"
        ));
    }

    #[test]
    fn test_array_literal_carrier_number_index_is_compile_error() {
        // The array-literal / empty-array path uses the `Type::Generic` carrier
        // (`BuiltinTypes::array`), which is resolved by the engine-level index
        // arm — that arm must be strict too.
        let mut engine = make_engine();
        let arr = BuiltinTypes::array(BuiltinTypes::integer());
        let result = engine.infer_index_access(&arr, &BuiltinTypes::number());
        assert!(
            result.is_err(),
            "generic-carrier array number index must be a compile error, got {:?}",
            result
        );
    }

    #[test]
    fn test_array_literal_carrier_int_index_type_checks() {
        let mut engine = make_engine();
        let arr = BuiltinTypes::array(BuiltinTypes::integer());
        let result = engine.infer_index_access(&arr, &BuiltinTypes::integer());
        assert!(
            result.is_ok(),
            "generic-carrier array int index must type-check, got {:?}",
            result
        );
    }

    #[test]
    fn test_string_number_index_is_compile_error() {
        let mut engine = make_engine();
        let result = engine.infer_index_access(&BuiltinTypes::string(), &BuiltinTypes::number());
        assert!(
            result.is_err(),
            "s[n: number] must be a compile error, got {:?}",
            result
        );
    }

    #[test]
    fn test_intersection_property_access_resolves_member_field() {
        let mut engine = make_engine();
        let ty = Type::Concrete(TypeAnnotation::Intersection(vec![
            TypeAnnotation::Object(vec![shape_ast::ast::ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            }]),
            TypeAnnotation::Object(vec![shape_ast::ast::ObjectTypeField {
                name: "z".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            }]),
        ]));

        let result = engine.infer_property_access(&ty, "z");
        assert!(result.is_ok(), "intersection member field should resolve");
    }
}
