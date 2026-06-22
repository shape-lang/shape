//! Expression-level type inference
//!
//! Handles type inference for all expression types.

use super::{CheckMode, TypeInferenceEngine};
use crate::type_system::checking::MethodTable;
use crate::type_system::exhaustiveness;
use crate::type_system::*;
use shape_ast::ast::{Expr, Literal, Span, TypeAnnotation};
use shape_ast::interpolation::{InterpolationPart, parse_interpolation_with_mode};

impl TypeInferenceEngine {
    /// True for an exact built-in NAMESPACE static-constructor pair
    /// (`DateTime.now`, `Content.text`, `Table.new`, …) that the bytecode
    /// compiler lowers to a dedicated `BuiltinFunction` via
    /// `compile_type_namespace_builtin_call`
    /// (`crates/shape-vm/src/compiler/expressions/function_calls.rs:1694`).
    /// Kept in lockstep with that table — these are namespace constructors,
    /// not instance methods, so inference must NOT emit a `HasField` /
    /// `HasMethod` constraint on the namespace reference. Callers guard this
    /// with a user-shadowing check (struct / alias / variable named the same
    /// take precedence), so the match is sound: a real instance access never
    /// reaches here.
    fn is_namespace_constructor(namespace: &str, method: &str) -> bool {
        matches!(
            (namespace, method),
            ("DateTime", "now")
                | ("DateTime", "utc")
                | ("DateTime", "parse")
                | ("DateTime", "from_epoch")
                | ("DateTime", "from_parts")
                | ("DateTime", "from_unix_secs")
                | ("Content", "chart")
                | ("Content", "text")
                | ("Content", "table")
                | ("Content", "code")
                | ("Content", "kv")
                | ("Content", "fragment")
                // SC1 (R8 — supervisor): `Color.rgb(r,g,b)` is the only
                // call-form style-spec constructor (returns a `string`
                // carrier). Named members are PropertyAccess, handled
                // separately in `infer_expr`'s PropertyAccess arm.
                | ("Color", "rgb")
                | ("Table", "new")
                | ("Code", "new")
                | ("KeyValue", "new")
        )
    }

    /// SC1: whether `namespace.member` is a known style-spec member access
    /// (`Color.red`, `Border.rounded`, `ChartType.line`, …). These lower to
    /// a `string` carrier; the call-form `Color.rgb(r,g,b)` is handled via
    /// `is_namespace_constructor`, not here. Kept in lockstep with the
    /// bytecode compiler's `style_spec_members`
    /// (`compiler/expressions/property_access.rs`).
    fn is_style_spec_member(namespace: &str, member: &str) -> bool {
        let members: &[&str] = match namespace {
            "Color" => &[
                "red", "green", "blue", "yellow", "magenta", "cyan", "white", "default",
            ],
            "Border" => &["rounded", "sharp", "heavy", "double", "minimal", "none"],
            "ChartType" => &[
                "line",
                "bar",
                "scatter",
                "area",
                "candlestick",
                "histogram",
                "boxplot",
                "heatmap",
                "bubble",
            ],
            _ => &[],
        };
        members.contains(&member)
    }

    /// Infer type of an expression
    /// T1 keystone (strict-flip, 2026-06-22): infer an expression's type AND
    /// record the synthesized (pre-substitution) type in the per-expression
    /// type table keyed by the expression's source span. The recorded type is
    /// still pre-solve here — it may carry fresh `Type::Variable`s; the
    /// post-solve pass in `infer_program_best_effort` rewrites every table
    /// entry through the final substitution and DROPS any entry that remains a
    /// free variable (no Unknown-default — an un-inferable expression stays
    /// absent so the compiler boundary surfaces a genuine compile error).
    ///
    /// This is the ROOT fix for the static-type-erasure class: the engine
    /// already computes these types while walking FUNCTION BODIES (via
    /// `infer_item` -> `infer_function`), but never recorded them keyed by
    /// span, so the bytecode-compiler bridge `infer_expr_type` re-ran inference
    /// at module scope (empty function-local env) and erased the result. The
    /// table captures the body-walk's own output at the site it was computed.
    pub fn infer_expr(&mut self, expr: &Expr) -> TypeResult<Type> {
        let ty = self.infer_expr_inner(expr)?;
        // Record under the expression's own span. Skip dummy spans (synthetic /
        // desugared nodes with no source location): they collide on `(0,0)` and
        // would alias unrelated expressions.
        let span = shape_ast::ast::Spanned::span(expr);
        if !span.is_dummy() {
            self.expr_type_table.insert(span, ty.clone());
        }
        Ok(ty)
    }

    /// Inner body of `infer_expr` — the actual structural inference. Kept
    /// separate so the public `infer_expr` can transparently record every
    /// synthesized type into `expr_type_table` (T1 keystone) without threading
    /// the recording through every `match` arm and early-return.
    fn infer_expr_inner(&mut self, expr: &Expr) -> TypeResult<Type> {
        match expr {
            Expr::Literal(Literal::FormattedString { value, mode }, span) => {
                self.infer_formatted_string_interpolations(value, *mode, *span)?;
                // R8 W4 W18.4 (supervisor 2026-05-24 D1): syntax-determined
                // return type. F-string with NO content-styling syntax →
                // `string` (preserves existing 500+ call sites). F-string
                // with ≥1 `ContentStyle` interpolation → `content`. The
                // presence test re-parses the value; this is a cheap pure-
                // syntax check (re-uses the same parser the lowering will
                // call).
                let parts = parse_interpolation_with_mode(value, *mode)
                    .map_err(|err| TypeError::ConstraintViolation(err.to_string()))?;
                let has_content_style = parts.iter().any(|p| {
                    matches!(
                        p,
                        shape_ast::interpolation::InterpolationPart::Expression {
                            format_spec: Some(
                                shape_ast::interpolation::InterpolationFormatSpec::ContentStyle(_),
                            ),
                            ..
                        }
                    )
                });
                if has_content_style {
                    Ok(BuiltinTypes::content())
                } else {
                    Ok(BuiltinTypes::string())
                }
            }

            Expr::Literal(lit, _) => self.infer_literal(lit),

            Expr::Identifier(name, span) => {
                let scheme_clone = self.env.lookup(name).cloned();
                scheme_clone
                    .map(|scheme| scheme.instantiate(&mut self.type_var_gen))
                    .or_else(|| {
                        // Fall back to a type reference for known struct type names.
                        // This enables static-path expressions like `Currency.symbol`
                        // where `Currency` is a type name, not a variable.
                        if self.struct_type_defs.contains_key(name.as_str())
                            || self.env.lookup_type_alias(name).is_some()
                        {
                            Some(Type::Concrete(TypeAnnotation::Reference(
                                name.as_str().into(),
                            )))
                        } else {
                            None
                        }
                    })
                    .or_else(|| {
                        // Recognize built-in namespace identifiers that have static
                        // constructor methods (e.g. DateTime.now(), Content.chart()).
                        // `Color` is included for the call-form `Color.rgb(r,g,b)`
                        // (SC1); its named members are PropertyAccess, intercepted
                        // before the object is inferred.
                        match name.as_str() {
                            "DateTime" | "Content" | "Color" => Some(Type::Concrete(
                                TypeAnnotation::Reference(name.as_str().into()),
                            )),
                            _ => None,
                        }
                    })
                    .ok_or_else(|| {
                        self.register_undefined_variable_origin(name, *span);
                        TypeError::UndefinedVariable(name.clone())
                    })
            }

            Expr::BinaryOp {
                left,
                op,
                right,
                span,
            } => {
                let mut left_type = self.infer_expr(left)?;
                let mut right_type = self.infer_expr(right)?;

                // Numeric-conversion LITERAL ADOPTION (spec §4): a bare integer
                // literal operand adopts the OTHER operand's concrete numeric
                // type when the literal value losslessly fits it. So
                // `val:number > 10` (literal `10` adopts `number`),
                // `val:number == 5`, `a:number * 3`, and `1 + 2.0` all unify as
                // a same-family op instead of rejecting the int-literal vs
                // number-value pair under the tightened §2 lattice. A value /
                // variable (not a literal) never adopts — `int_var + number_var`
                // still rejects. An out-of-range literal does not adopt.
                if let Some(adopted) = Self::adopt_int_literal_in_context(left, &right_type) {
                    left_type = adopted;
                } else if let Some(adopted) = Self::adopt_int_literal_in_context(right, &left_type)
                {
                    right_type = adopted;
                }
                // ROOT-1 (comparison-literal-adoption ordering): when the
                // concrete-context adoption above does NOT fire because the
                // literal's partner is a still-unresolved inference VARIABLE
                // (not a concrete numeric type), the literal must adopt the
                // partner var's identity rather than staying its natural `int`.
                // Otherwise the comparison/equality arm's same-type constraint
                // (`effective_left ~ effective_right` at operators.rs) PINS the
                // var to `int` from the literal — colliding with a later
                // `number` resolution (e.g. `Ok(n)` into `Result<number>`) and
                // spuriously rejecting valid code. Arithmetic ops were already
                // immune (they route through `numeric_result_type`, whose
                // `(Variable, Concrete-numeric)` arm propagates the var instead
                // of pinning). This makes the literal defer to the var
                // uniformly at the BinaryOp seam — pure literal deferral, no
                // int-VALUE->number widening (delegates the literal-shape gate
                // to `adopt_int_literal_in_context`, which rejects every
                // non-literal operand).
                else if let Some(adopted) = Self::adopt_int_literal_into_var(left, &right_type) {
                    left_type = adopted;
                } else if let Some(adopted) = Self::adopt_int_literal_into_var(right, &left_type) {
                    right_type = adopted;
                }

                self.infer_binary_op(&left_type, op, &right_type, *span)
            }

            Expr::UnaryOp { op, operand, .. } => {
                let operand_type = self.infer_expr(operand)?;
                self.infer_unary_op(op, &operand_type)
            }

            Expr::PropertyAccess {
                object,
                property,
                span,
                ..
            } => {
                // SC1 (R8 — supervisor): style-spec namespace member access.
                // `Color.red` / `Border.rounded` / `ChartType.line` are NOT
                // variables — `Color` etc. are compile-time-constant
                // namespaces, not values. Infer the result as `string`
                // (the runtime carrier) WITHOUT inferring the object, which
                // would otherwise reject with UndefinedVariable. Bounded
                // tightly: bare-identifier object that is not a user struct
                // / type-alias / variable, AND an exact style-spec member.
                if let Expr::Identifier(ns, _) = object.as_ref() {
                    let is_user_shadow = self.struct_type_defs.contains_key(ns.as_str())
                        || self.env.lookup_type_alias(ns).is_some()
                        || self.env.lookup(ns).is_some();
                    if !is_user_shadow {
                        if Self::is_style_spec_member(ns, property) {
                            return Ok(BuiltinTypes::string());
                        }
                        // An unknown member on a style-spec namespace rejects
                        // cleanly here (e.g. `Color.bogus`) with a precise
                        // message rather than the generic "Reference(..) cannot
                        // have fields" from `infer_property_access`.
                        if matches!(ns.as_str(), "Color" | "Border" | "ChartType") {
                            return Err(TypeError::UnknownProperty(
                                ns.clone(),
                                property.clone(),
                            ));
                        }
                    }
                }

                // Track the variable name for hoisting lookup
                let var_name = if let Expr::Identifier(name, _) = object.as_ref() {
                    Some(name.clone())
                } else {
                    None
                };

                // Set current access variable so hoisting can be looked up
                self.env.set_current_access_variable(var_name);

                let object_type = self.infer_expr(object)?;
                let result = self.infer_property_access(&object_type, property);
                if let Err(TypeError::UnknownProperty(_, missing)) = &result {
                    self.register_unknown_property_origin(missing, *span);
                }

                // Clear the current access variable
                self.env.set_current_access_variable(None);

                result
            }

            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                let object_type = self.infer_expr(object)?;
                let index_type = self.infer_expr(index)?;

                if let Some(end) = end_index {
                    let _end_type = self.infer_expr(end)?;
                    // For range indexing, return the same array type
                    Ok(object_type)
                } else {
                    // Tuple element access (book `fundamentals/variables`
                    // §Tuple Types): `pair[0]` / `pair[1]` on a `[T0, T1]`-typed
                    // value resolves to the per-POSITION element type. The index
                    // must be a compile-time constant non-negative integer so the
                    // position is statically known; a tuple has no single
                    // element type, so a non-constant index is a compile error.
                    if let Type::Concrete(shape_ast::ast::TypeAnnotation::Tuple(elem_types)) =
                        &object_type
                    {
                        return self.infer_tuple_index(elem_types, index);
                    }
                    // Apply the current substitution before the index dispatch
                    // so a `Borrow`-typed receiver that only resolves via the
                    // unifier (e.g. `let r = &a` with no annotation, indexed
                    // inside a function as `r[1]`) is seen as its concrete
                    // `Borrow { inner }` form by `infer_index_access`'s
                    // RefDispatch deref arm — not as a still-unresolved variable
                    // that falls to the constraint path with the referent never
                    // recovered (leaving the element `unknown`).
                    let object_type = self.unifier.apply_substitutions(&object_type);
                    self.infer_index_access(&object_type, &index_type)
                }
            }

            Expr::FunctionCall {
                name, args, span, ..
            } => self.infer_function_call(name, args, *span),

            Expr::QualifiedFunctionCall {
                namespace,
                function,
                args,
                span,
                ..
            } => {
                // Check if this is an enum constructor (e.g. Signal::Market(1, 2)).
                // The parser can't distinguish enum tuple constructors from qualified
                // function calls, so we resolve it here using type information.
                if self.env.get_enum(namespace).is_some() {
                    for arg in args {
                        self.infer_expr(arg)?;
                    }
                    Ok(Type::Concrete(TypeAnnotation::Reference(
                        namespace.as_str().into(),
                    )))
                } else if self.env.lookup(namespace).is_some()
                    || self.struct_type_defs.contains_key(namespace.as_str())
                    || self.env.lookup_type_alias(namespace).is_some()
                    || matches!(namespace.as_str(), "DateTime" | "Content")
                {
                    let synthetic = Expr::MethodCall {
                        receiver: Box::new(Expr::Identifier(namespace.clone(), *span)),
                        method: function.clone(),
                        args: args.clone(),
                        named_args: vec![],
                        optional: false,
                        span: *span,
                    };
                    self.infer_expr(&synthetic)
                } else {
                    for arg in args {
                        self.infer_expr(arg)?;
                    }
                    // A module-qualified call's value is its RETURN value, never
                    // the module. The inference tier has no module-export
                    // signatures (Item::Import is a no-op in predeclare_item;
                    // known bindings are bare fresh vars), so the precise return
                    // type is unknown HERE — return a fresh result var rather than
                    // the namespace Reference, which would wrongly type the result
                    // as the module and reject any member access on it. The
                    // bytecode compiler resolves the real signature via its module
                    // schema registry. No concrete type is fabricated.
                    Ok(self.fresh_type_var())
                }
            }

            Expr::EnumConstructor { enum_name, .. } => {
                Ok(Type::Concrete(TypeAnnotation::Reference(enum_name.clone())))
            }

            Expr::Array(elements, _) => {
                if elements.is_empty() {
                    // Empty array — the element type is an unresolved fresh
                    // variable. R5a-literal sibling-adoption (USER RULING
                    // 2026-06-01): an inner `[]` in `[[], [1], []]` must adopt
                    // the sibling-proven element type (`int`). Construct the
                    // var-PRESERVING `Type::Generic { base: Array, args:
                    // [Variable] }` form so the fresh var unifies with a
                    // sibling `Array<int>` via the `(Generic, Generic)` arm in
                    // `solve_constraint`. The legacy `BuiltinTypes::array`
                    // helper routes through `to_annotation()`, which drops the
                    // `Type::Variable` to `Basic("unknown")` (documented TypeVar
                    // loss) — a CONCRETE `Array<unknown>` that can never unify
                    // with `Array<int>` (the `Vec<unknown> not compatible with
                    // Vec<int>` rejection). Keeping the var preserves strict
                    // typing: the empty array stays unresolved until a sibling
                    // or annotation pins it; if nothing does, the var surfaces
                    // downstream as an unresolved-element error (no `unknown`
                    // fabrication).
                    let elem_type = self.fresh_type_var();
                    Ok(Type::Generic {
                        base: Box::new(Type::Concrete(TypeAnnotation::Reference("Array".into()))),
                        args: vec![elem_type],
                    })
                } else {
                    // Compute the contributed *element* type for each entry. A
                    // spread element `...a` contributes the element type of `a`'s
                    // array (not the whole array type), so `[0, ...a, 3]` unifies
                    // `int` with `int`, not `int` with `Vec<int>`.
                    let mut elem_types: Vec<Type> = Vec::with_capacity(elements.len());
                    for elem in elements {
                        elem_types.push(self.array_literal_element_contribution(elem)?);
                    }

                    // Numeric-conversion §4 literal adoption (array-element
                    // context): if the array mixes bare int literals with a
                    // float/number element (`[1, 2.5, 3]`), the int literals
                    // adopt `number` so the element type unifies to `number`
                    // instead of rejecting `(int, number)` under the tightened
                    // §2 lattice. The unifying element type is the float family
                    // when ANY element contributes it and EVERY non-float element
                    // is a bare int literal that losslessly fits `number`.
                    let elem_ctx =
                        self.array_literal_numeric_element_context(elements, &elem_types);
                    let first_type = elem_ctx.clone().unwrap_or_else(|| elem_types[0].clone());

                    for (i, elem_type) in elem_types.iter().enumerate() {
                        // A bare int literal element that adopts the unified
                        // numeric element type pushes no rejecting constraint.
                        if let Some(ctx) = &elem_ctx {
                            if Self::adopt_int_literal_in_context(&elements[i], ctx).is_some() {
                                continue;
                            }
                        }
                        if i == 0 && elem_ctx.is_none() {
                            continue;
                        }
                        self.constraints
                            .push((first_type.clone(), elem_type.clone()));
                    }

                    Ok(BuiltinTypes::array(first_type))
                }
            }

            Expr::TableRows(_, _) => {
                // Table row literals — type inference not yet implemented
                Ok(self.fresh_type_var())
            }

            Expr::Object(entries, _) => {
                use shape_ast::ast::ObjectEntry;
                let mut field_types = Vec::new();

                for entry in entries {
                    match entry {
                        ObjectEntry::Field {
                            key,
                            value,
                            type_annotation,
                        } => {
                            let value_type = self.infer_expr(value)?;
                            let field_annotation = if let Some(ta) = type_annotation {
                                let annotated_type = Type::Concrete(ta.clone());
                                self.constraints.push((value_type.clone(), annotated_type));
                                ta.clone()
                            } else {
                                // Resolve through the unifier first — a field whose
                                // value already unified with a concrete type freezes
                                // to that type. A field value that is still an
                                // unresolved variable is encoded as a `tyvar` marker
                                // (not `unknown`) so callsite substitution can later
                                // resolve it: `fn aabb(lo, hi) { {min: lo, max: hi} }`
                                // returns `{min: <tyvar lo>, max: <tyvar hi>}`, and a
                                // call `aabb(1, 5)` substitutes the markers to `int`.
                                let resolved = self.unifier.apply_substitutions(&value_type);
                                match &resolved {
                                    Type::Variable(var) => tyvar_to_annotation(var),
                                    Type::Constrained { var, .. } => tyvar_to_annotation(var),
                                    // A closure-valued field (e.g. `{ greet: |name| ... }`)
                                    // must keep its unresolved param/return TypeVars as tyvar
                                    // markers, not collapse them to "unknown" via the lossy
                                    // Function->annotation path (the documented
                                    // `Type::Function` TypeVar loss in to_annotation).
                                    // Otherwise `obj.greet("World")` checks the arg against an
                                    // "unknown" param and rejects. Mirrors the Variable/
                                    // Constrained arms so call-site substitution resolves it.
                                    Type::Function { params, returns } => {
                                        let unifier = &self.unifier;
                                        let conv = |t: &Type| -> TypeAnnotation {
                                            match &unifier.apply_substitutions(t) {
                                                Type::Variable(v) => tyvar_to_annotation(v),
                                                Type::Constrained { var, .. } => {
                                                    tyvar_to_annotation(var)
                                                }
                                                other => {
                                                    other.to_annotation().unwrap_or_else(|| {
                                                        TypeAnnotation::Basic("unknown".to_string())
                                                    })
                                                }
                                            }
                                        };
                                        let param_anns = params
                                            .iter()
                                            .map(|p| shape_ast::ast::FunctionParam {
                                                name: None,
                                                optional: false,
                                                type_annotation: conv(p),
                                            })
                                            .collect();
                                        TypeAnnotation::Function {
                                            params: param_anns,
                                            returns: Box::new(conv(returns)),
                                        }
                                    }
                                    _ => resolved.to_annotation().unwrap_or_else(|| {
                                        TypeAnnotation::Basic("unknown".to_string())
                                    }),
                                }
                            };
                            field_types.push(shape_ast::ast::ObjectTypeField {
                                name: key.clone(),
                                optional: false,
                                type_annotation: field_annotation,
                                annotations: vec![],
                            });
                        }
                        ObjectEntry::Spread(spread_expr) => {
                            // For spread, we infer the type and merge fields if it's an object
                            let spread_type = self.infer_expr(spread_expr)?;
                            match &spread_type {
                                Type::Concrete(TypeAnnotation::Object(spread_fields)) => {
                                    field_types.extend(spread_fields.clone());
                                }
                                Type::Concrete(TypeAnnotation::Reference(name)) => {
                                    // Named type (e.g., Point) — look up struct fields
                                    if let Some(struct_def) =
                                        self.struct_type_defs.get(name.as_str()).cloned()
                                    {
                                        for field in &struct_def.fields {
                                            field_types.push(shape_ast::ast::ObjectTypeField {
                                                name: field.name.clone(),
                                                optional: false,
                                                type_annotation: field.type_annotation.clone(),
                                                annotations: vec![],
                                            });
                                        }
                                    }
                                }
                                _ => {
                                    // Fields from spread will be determined at runtime
                                }
                            }
                        }
                    }
                }

                Ok(Type::Concrete(TypeAnnotation::Object(field_types)))
            }

            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                let cond_type = self.infer_expr(condition)?;
                self.constraints.push((cond_type, BuiltinTypes::boolean()));

                let then_type = self.infer_expr(then_expr)?;

                if let Some(else_expr) = else_expr {
                    let else_type = self.infer_expr(else_expr)?;
                    // Both branches should have the same type
                    self.constraints.push((then_type.clone(), else_type));
                }

                Ok(then_type)
            }

            Expr::TypeAssertion {
                expr,
                type_annotation,
                ..
            } => {
                let expr_type = self.infer_expr(expr)?;
                if let TypeAnnotation::Generic { name, args } = type_annotation {
                    if name == "Option" && args.len() == 1 {
                        // `as Type?` is the typed fallible-conversion form.
                        // It compiles to `Result<Type, AnyError>` and is validated
                        // statically against source/target strong types.
                        let target_type = self.resolve_type_annotation(&args[0]);
                        self.validate_fallible_conversion(&expr_type, &target_type)?;
                        return Ok(self.wrap_result_type(target_type));
                    }
                }

                let asserted_type = self.resolve_type_annotation(type_annotation);

                // Width integer cast: `expr as i8`, `expr as u16`, etc. is a
                // Rust-style bit-truncating conversion (the compiler emits
                // `OpCode::CastWidth`, NOT an Into dispatch — see
                // compiler/expressions/type_ops.rs:876-888). It is statically
                // infallible (truncates, never rejects), so it must bypass the
                // Into<Target> validation below. Mirrors the compiler's cast
                // ordering. Scoped to the 7 real width names via
                // IntWidth::from_name (i8/u8/i16/u16/i32/u32/u64); i64/int/number
                // are intentionally NOT width names and are unaffected.
                if let TypeAnnotation::Basic(name) = type_annotation {
                    if shape_ast::IntWidth::from_name(name).is_some() {
                        return Ok(asserted_type);
                    }
                }

                // D1 (numeric-conversion GREEN Stage 1) — primitive numeric
                // cast gate. A cast whose TARGET is a primitive numeric type
                // (`int`/`i64`, the width names, `number`/`f32`, `decimal`) and
                // whose SOURCE is also a primitive numeric type is a BUILT-IN
                // cast: it is unconditionally legal for the numeric lattice and
                // lowers to the already-existing typed `ConvertToInt` /
                // `ConvertToNumber` / `ConvertToDecimal` / `CastWidth` opcodes
                // (`compiler/expressions/type_ops.rs:701`,`:878`,
                // executor bodies `executor/builtins/type_ops.rs:591-659`).
                // Per spec §3 / §8 (numeric-conversion-spec.md:404-405,:462) it
                // is "always permitted for any numeric src→dst, with §3
                // semantics, bypassing the user-`Into` requirement" — it must
                // NOT route through `validate_infallible_conversion`'s
                // Into-impl lookup (which has no entry for width-typed sources
                // such as `i32 as number`, nor for the lossy `number as int`
                // direction, since the prelude only declares the fallible
                // `TryInto<int> for number`). This mirrors the width-cast
                // bypass above and is COMPILE-TIME acceptance only — the
                // runtime conversion correctness (truncation toward zero,
                // out-of-range / non-finite handling) is a separate stage. No
                // runtime coercion is introduced; the implicit-conversion paths
                // are handled separately (constraint solver), this gate only
                // governs the EXPLICIT `as` cast.
                if let TypeAnnotation::Basic(target_name) = type_annotation {
                    if BuiltinTypes::is_numeric_type_name(target_name)
                        && self.source_is_numeric_for_cast(&expr_type)
                    {
                        return Ok(asserted_type);
                    }
                }

                // Plain `as Type` is trait-dispatched conversion when Type is a
                // concrete named target supported by Into<Target>.
                if self.try_into_selector(&asserted_type).is_some() {
                    self.validate_infallible_conversion(&expr_type, &asserted_type)?;
                    return Ok(asserted_type);
                }

                // Plain `as Type` remains a strict assertion.
                self.constraints.push((expr_type, asserted_type.clone()));

                Ok(asserted_type)
            }

            Expr::InstanceOf {
                expr,
                type_annotation: _,
                ..
            } => {
                self.infer_expr(expr)?;
                Ok(BuiltinTypes::boolean())
            }

            // Method call: receiver.method(args)
            Expr::MethodCall {
                receiver,
                method,
                args,
                span: _,
                ..
            } => {
                let receiver_type = self.infer_expr(receiver)?;

                // Method dispatch through a reference (v0.3.3 RefDispatch):
                // `r.len()` on a `r: &Array<T>` / `&mut Array<T>` dispatches the
                // method THROUGH the reference. Deref the `Borrow { inner }` to
                // its referent so method resolution (PHF builtin table, struct
                // method registry, generic-signature lookup) runs on the
                // referent — exactly as `a.len()` would. Mirrors the
                // field-access auto-deref in `infer_property_access_internal`
                // (access.rs:46-52) and the index auto-deref in
                // `infer_index_access`. Without this the `Borrow` receiver falls
                // through to the property-access fallback -> `HasField` ->
                // "Borrow(..) cannot have fields". The referent annotation is
                // forwarded verbatim (no coercion). Bounded to method calls only
                // (this arm); the namespace-constructor receiver below is a bare
                // identifier, never a `Borrow`, so it is untouched.
                let receiver_type = match &receiver_type {
                    Type::Concrete(TypeAnnotation::Borrow { inner, .. }) => {
                        Type::Concrete((**inner).clone())
                    }
                    _ => receiver_type,
                };

                // STRICT-FLIP namespace-constructor regression fix (SC0,
                // 2026-06-16). A static constructor call on a built-in
                // NAMESPACE — `DateTime.now()`, `Content.text("x")`,
                // `Table.new()`, `Code.new()`, `KeyValue.new()` — has its
                // receiver typed as `Type::Concrete(Reference("DateTime"))`
                // etc. by the `Expr::Identifier` arm (lines 65-74). These are
                // NOT instance method calls: the receiver is a namespace, not
                // a value. The bytecode compiler lowers each known
                // (namespace, method) pair to a dedicated `BuiltinFunction`
                // (`compile_type_namespace_builtin_call`,
                // `function_calls.rs:1694-1721`) — that mapping is the
                // authoritative resolution.
                //
                // Without this guard the call falls through to the
                // callable-field fallback (-> `infer_property_access` ->
                // `HasField` -> "Reference(..) cannot have fields") for
                // `DateTime.now`, OR to the `HasMethod` fallback ->
                // "Method 'text' not found on type 'Content'" for
                // `Content.text` (because `Content` is also registered as a
                // trait with no `text` member). Both worked on main; the
                // strict-flip lattice tightening surfaced the missing arm.
                //
                // SOUNDNESS: bounded TIGHTLY to (a) a bare-identifier receiver
                // whose name is NOT a user-defined struct / type-alias /
                // variable (so a user `type DateTime { .. }` shadows and is
                // NOT intercepted — it resolves through the struct path), and
                // (b) an EXACT (namespace, method) pair from the compiler's
                // builtin table. A genuine bad field/method access on a user
                // value is untouched: it never matches a bare namespace
                // identifier here, so it still rejects downstream. The result
                // is a fresh var (the constructor's return type is resolved
                // authoritatively by the bytecode compiler) — no concrete type
                // is fabricated.
                if let Expr::Identifier(recv_name, _) = receiver.as_ref() {
                    if Self::is_namespace_constructor(recv_name, method)
                        && !self.struct_type_defs.contains_key(recv_name.as_str())
                        && self.env.lookup_type_alias(recv_name).is_none()
                        && self.env.lookup(recv_name).is_none()
                    {
                        // Still type-check the arguments so arg-level errors
                        // (and literal-adoption side effects) surface.
                        for arg in args {
                            self.infer_expr(arg)?;
                        }
                        // The `DateTime` namespace constructors
                        // (`now`/`utc`/`parse`/`from_epoch`/`from_parts`/
                        // `from_unix_secs`) all yield a `DateTime` value per the
                        // datetime book chapter. Returning the concrete
                        // `Reference("DateTime")` (rather than a fresh,
                        // never-pinned var) is what lets a `let a =
                        // DateTime.parse(..)` binding carry a known type into
                        // downstream operator arithmetic (`a - b`,
                        // `a + 3d`) — without it both operands lower to
                        // `unknown` and strict typing rejects the op. The other
                        // namespace constructors (Content/Color/Table/…) keep
                        // the fresh-var behavior: their return types are
                        // resolved authoritatively by the bytecode compiler and
                        // are not consumed by the temporal operator rules.
                        if recv_name == "DateTime" {
                            return Ok(Type::Concrete(TypeAnnotation::Reference(
                                "DateTime".into(),
                            )));
                        }
                        return Ok(self.fresh_type_var());
                    }
                }

                // IIFE / chained-call (`(|y| body)(args)`, `f(a)(b)`) — the parser
                // models these as `MethodCall { method: "__call__", receiver: <callable-expr> }`
                // per `crates/shape-ast/src/parser/expressions/primary.rs:167`. When
                // the receiver type resolves to a `Function`, the call site's
                // result type is the function's declared return type. Producer-side
                // stamp (ADR-006 §2.7.5): the closure's return type is computed
                // at the `Expr::FunctionExpr` arm below (line 713) and propagated
                // here so the IIFE result has a concrete kind instead of an
                // unresolved type variable. Without this, downstream uses of the
                // result (e.g. `total + (|y| y + base)(x)` inside a for-loop)
                // see `unknown` and either fail strict-typing at compile time
                // or surface as JIT garbage at runtime (Phase 4b Round 3
                // Surface-1A LANG-W13-3-iife-closure-capture).
                if method == "__call__" {
                    let func_shape = match &receiver_type {
                        Type::Function { params, returns } => {
                            Some((params.clone(), returns.as_ref().clone()))
                        }
                        Type::Concrete(TypeAnnotation::Function {
                            params: concrete_params,
                            returns: concrete_returns,
                        }) => {
                            let params: Vec<Type> = concrete_params
                                .iter()
                                .map(|p| Type::Concrete(p.type_annotation.clone()))
                                .collect();
                            let returns = Type::Concrete(*concrete_returns.clone());
                            Some((params, returns))
                        }
                        _ => None,
                    };
                    if let Some((params, returns)) = func_shape {
                        let arg_types: Vec<Type> = args
                            .iter()
                            .map(|arg| self.infer_expr(arg))
                            .collect::<Result<_, _>>()?;
                        if params.len() == arg_types.len() {
                            for (i, (arg_ty, param_ty)) in
                                arg_types.iter().zip(params.iter()).enumerate()
                            {
                                // Numeric-conversion §4 literal adoption
                                // (IIFE/value-call argument context): a bare int
                                // literal arg adopts the closure parameter's
                                // concrete numeric type when it losslessly fits
                                // (`(|y| y * 2.0)(3)`), parallel to the named-call
                                // adoption in `infer_function_call`. A non-literal
                                // int arg into a `number` param still rejects.
                                if args
                                    .get(i)
                                    .and_then(|a| Self::adopt_int_literal_in_context(a, param_ty))
                                    .is_some()
                                {
                                    continue;
                                }
                                self.constraints.push((arg_ty.clone(), param_ty.clone()));
                            }
                            return Ok(returns);
                        }
                        // Arity mismatch — fall through to the generic dispatch
                        // path which produces a clearer diagnostic via
                        // `HasMethod`/property-access. Don't fabricate.
                    }
                }

                // Look up expected parameter types BEFORE inferring arguments
                // so closures get their param types from the method signature.
                let (type_name, receiver_params) =
                    MethodTable::extract_receiver_info(&receiver_type);
                let gsig_opt = type_name.as_ref().and_then(|tn| {
                    self.method_table
                        .lookup_generic_signature(tn, method)
                        .cloned()
                });
                // Retain the gsig + the per-callsite fresh `method_vars` so the
                // method's RETURN type can be resolved against the SAME vars the
                // expected param types reference. This is what lets a closure's
                // proven return type flow into a `MethodParam`-position result
                // element (`[1,2,3].map(|x| x*2)` → `MethodParam(0)` bound to
                // `int` → result `Array<int>`, parity with `filter`'s
                // `SelfType` element preservation). See the closure-return
                // binding block after `arg_types` below.
                let method_vars: Vec<Type> = gsig_opt
                    .as_ref()
                    .map(|gsig| {
                        (0..gsig.method_type_params)
                            .map(|_| self.fresh_type_var())
                            .collect()
                    })
                    .unwrap_or_default();
                let expected_arg_types: Option<Vec<Type>> = gsig_opt.as_ref().map(|gsig| {
                    gsig.param_types
                        .iter()
                        .map(|pt| {
                            MethodTable::resolve_type_param_expr(
                                pt,
                                &receiver_type,
                                &receiver_params,
                                &method_vars,
                            )
                        })
                        .collect()
                });

                // Infer arguments WITH expected types (bidirectional).
                //
                // Most args are checked with `CheckMode::Synth`: a SOFT probe
                // that, on a successful `try_unify`, RETURNS the hint type. This
                // is the established behavior (closures get their params from the
                // method signature; an empty-array arg adopts an expected
                // `Vec<int>`; etc.) and is preserved unchanged.
                //
                // The ONE exception is a bare receiver-type-param hint — a
                // `Type::Variable` such as K or V on the fresh-per-callsite
                // `HashMap<K, V>` minted by the `HashMap()` constructor. For
                // those, `Synth` would collapse a concrete `string`/`int` arg
                // back to the bare K/V variable, hiding the very type we need to
                // flow into the receiver's `<K,V>` slots below. So a variable
                // hint uses plain `infer_expr` to keep the concrete arg type.
                let arg_types: Vec<Type> = if let Some(ref expected) = expected_arg_types {
                    args.iter()
                        .enumerate()
                        .map(|(i, arg)| match expected.get(i) {
                            Some(Type::Variable(_)) | None => self.infer_expr(arg),
                            Some(ty) => self.check_expr(arg, CheckMode::Synth(ty.clone())),
                        })
                        .collect::<Result<_, _>>()?
                } else {
                    args.iter()
                        .map(|arg| self.infer_expr(arg))
                        .collect::<Result<_, _>>()?
                };

                // Bind value-position args into the receiver's type PARAMS.
                //
                // The generic-method signature resolution above (via
                // `resolve_type_param_expr` against `receiver_params`) only
                // computes the *expected* param types and the method's RETURN
                // type — it never unifies the actual argument types back into
                // the receiver's `<K,V>` slots. For a polymorphic receiver such
                // as `HashMap<K, V>` minted fresh by the `HashMap()`
                // constructor, that left K,V unbound and `.set(k, v)` returned
                // `HashMap<_oob, _oob>` (the out-of-bounds placeholder). The
                // `CheckMode::Synth` hint above is a SOFT constraint (a
                // read-only `try_unify` probe), so it doesn't bind them either.
                //
                // This is bounded TIGHTLY to expected param types that are a
                // bare `Type::Variable` — i.e. a `TypeParamExpr::ReceiverParam`
                // that resolved to one of the receiver's own type-param vars
                // (K/V on `HashMap<K, V>`). For each such arg we (a) push a hard
                // constraint so the deferred solver binds the var, and (b) when
                // the arg is fully concrete, eagerly bind it into the engine's
                // unifier so the method-call RESULT type is concrete
                // immediately (`HashMap<string, int>` rather than
                // `HashMap<K, V>`). The eager concreteness matters for the
                // let-gen function-return gate (`ensure_no_unresolved_generic_args`,
                // which runs BEFORE the deferred solver), so
                // `fn build() { HashMap().set("x", 42) }` no longer trips it.
                // The fresh constructor vars are unique to this callsite, so the
                // bind never aliases another use.
                //
                // Expected param types that are NOT a bare variable are LEFT
                // ALONE: `E::SelfType` params (`concat`/`zip` → `Vec<int>`),
                // concrete element params (`includes` on `Vec<int>` → `int`),
                // and function params (closures, inferred bidirectionally) keep
                // their prior soft-`Synth` behavior — force-constraining those
                // would reject valid calls like `[1,2,3].concat([])`,
                // `[1,2,3].zip(["a"])`, or `[1,2,3].includes(None)`.
                if let Some(ref expected) = expected_arg_types {
                    for (i, expected_ty) in expected.iter().enumerate() {
                        let Type::Variable(var) = expected_ty else {
                            continue;
                        };
                        if args
                            .get(i)
                            .and_then(|a| Self::adopt_int_literal_in_context(a, expected_ty))
                            .is_some()
                        {
                            continue;
                        }
                        if let Some(arg_ty) = arg_types.get(i) {
                            self.constraints.push((arg_ty.clone(), expected_ty.clone()));
                            let resolved_arg = self.unifier.apply_substitutions(arg_ty);
                            if !self.type_contains_unresolved_vars(&resolved_arg)
                                && self.unifier.lookup(var).is_none()
                            {
                                self.unifier.bind(var.clone(), resolved_arg);
                            }
                        }
                    }
                }

                // STRICT-FLIP (v0.3.3 map/collect OUTPUT element stamp): bind the
                // method's RETURN-position `MethodParam` vars from the actual
                // closure argument's proven type. `map`'s signature is
                // `(fn(ReceiverParam(0)) -> MethodParam(0)) -> Vec<MethodParam(0)>`;
                // the result element IS the closure's return type. The bare-
                // variable block above only binds value-position params (HashMap
                // K/V); a function-typed param's INNER return var (`MethodParam(0)`
                // inside the `fn(...) -> MethodParam(0)` expected type) is never
                // bound there, so the result stayed `Vec<freshvar>` — a FREE
                // tyvar that later unified with ANY annotation (`let r =
                // [1,2,3].map(|x| x*2); let n: number = r[0]` wrongly ACCEPTED).
                //
                // Parity with `filter` (which returns `SelfType` → the receiver's
                // concrete `Array<int>` element): we unify the expected function-
                // param's return position against the actual closure arg's return
                // position so the closure's proven return type (`int` from
                // `x * 2` with `x: ReceiverParam(0) = int`) binds `MethodParam(0)`.
                // Per ADR-006 §2.7.5 stamp-at-compile-time: the closure's inferred
                // return type IS the proof — no coercion, no fabrication. An
                // un-inferable closure return leaves `MethodParam(0)` a var, so a
                // numeric annotation on the result still REJECTS (not coerces).
                // `int` and `number` do NOT unify (CLAUDE.md §Type-System-Rules).
                if let Some(ref expected) = expected_arg_types {
                    for (i, expected_ty) in expected.iter().enumerate() {
                        // Only the engine-level `Type::Function` form carries a
                        // `MethodParam`-resolved var in return position (the
                        // `resolve_type_param_expr` `Function` arm builds a
                        // `Type::Function`); a `Concrete(Function)` expected type
                        // is fully concrete already and needs no binding.
                        let Type::Function {
                            returns: exp_ret, ..
                        } = expected_ty
                        else {
                            continue;
                        };
                        // BOUNDED TIGHTLY to a return position that is a bare
                        // `MethodParam` var (the `map` / `flatMap` element var).
                        // A CONCRETE expected return (`sort`'s comparator
                        // `(T,T) -> number`, `findIndex` `-> int`, `every`/`some`
                        // `-> bool`) is LEFT ALONE — its prior soft-`Synth`
                        // unify-probe behavior must be preserved: a named
                        // comparator `fn asc(a:int,b:int)->int` passed to `sort`
                        // (whose registered comparator return is `number`) must
                        // NOT push a hard `int ~ number` constraint here (that
                        // wrongly rejected `[..].sort(asc)`; the comparator result
                        // is discarded, so its exact numeric family is not
                        // load-bearing). Only a `MethodParam`-derived result var
                        // (whose value flows into the method RESULT element type)
                        // needs binding.
                        let Type::Variable(ret_var) = exp_ret.as_ref() else {
                            continue;
                        };
                        let actual_ret = match arg_types.get(i) {
                            Some(Type::Function { returns, .. }) => returns.as_ref().clone(),
                            Some(Type::Concrete(TypeAnnotation::Function {
                                returns: cret,
                                ..
                            })) => Type::Concrete(*cret.clone()),
                            _ => continue,
                        };
                        // Deferred constraint (sound default) + eager bind when the
                        // closure return is concrete, so the method-call result
                        // element type is concrete immediately (the strict
                        // annotation check + index-access type both run before the
                        // deferred solver).
                        self.constraints
                            .push((actual_ret.clone(), exp_ret.as_ref().clone()));
                        let resolved_actual = self.unifier.apply_substitutions(&actual_ret);
                        if !self.type_contains_unresolved_vars(&resolved_actual)
                            && self.unifier.lookup(ret_var).is_none()
                        {
                            self.unifier.bind(ret_var.clone(), resolved_actual);
                        }
                    }
                }

                // STRICT-FLIP (v0.3.3): resolve the method-call RESULT type using
                // the SAME `method_vars` whose return-position vars were just
                // bound from the closure argument — NOT `resolve_method_call`,
                // which mints its own fresh (and therefore unbound) `method_vars`,
                // re-introducing the free-tyvar hole. Only fires for a generic
                // signature that actually carries method type params (`map`,
                // `flatMap`, `reduce`, …); the `method_vars.is_empty()` /
                // no-gsig cases fall through to the established
                // `resolve_method_call` path below unchanged.
                if let Some(ref gsig) = gsig_opt {
                    if !method_vars.is_empty() {
                        let result_type = MethodTable::resolve_type_param_expr(
                            &gsig.return_type,
                            &receiver_type,
                            &receiver_params,
                            &method_vars,
                        );
                        return Ok(self.unifier.apply_substitutions(&result_type));
                    }
                }

                // J-CT.1: reject calls to `comptime impl`-registered methods
                // outside a `comptime { ... }` context. We check before the
                // normal method-table resolution so the error surfaces with
                // the comptime-specific message (and not a generic
                // "method not found"). Receiver-name extraction reuses the
                // same `extract_receiver_info` that drives normal resolution
                // — if the receiver type is too unresolved to extract a
                // name, the gate is a no-op (the method-not-found path
                // surfaces a clearer diagnostic anyway).
                if !self.in_comptime_context() {
                    // Primary path: receiver name extracted via the same helper
                    // that drives normal resolution. Reuses the type-name slot
                    // that already powers `lookup_generic_signature` above so
                    // we never invent a name the resolver wouldn't see.
                    let mut gated = false;
                    if let Some(ref tn) = type_name {
                        if self.method_table.is_comptime_method(tn, method) {
                            return Err(TypeError::ComptimeMethodCallOutsideComptime {
                                type_name: tn.clone(),
                                method_name: method.clone(),
                            });
                        }
                        gated = true;
                    }
                    // Fallback: `type T { ... }` is registered as a type alias
                    // that `resolve_type_annotation` recursively expands to
                    // its `Object(...)` shape, so a function parameter typed
                    // `T` arrives here as `Type::Concrete(Object(...))` —
                    // `extract_receiver_info` returns `None` and the primary
                    // path can't fire. Recover the original struct name by
                    // matching the object shape against `struct_type_defs`.
                    // This keeps the gate sound for the public API surface
                    // (struct fields are a stable identity) without leaking
                    // any new naming convention.
                    if !gated {
                        if let Type::Concrete(TypeAnnotation::Object(actual_fields)) =
                            &receiver_type
                            && let Some(struct_name) =
                                self.struct_name_for_object_shape(actual_fields)
                            && self.method_table.is_comptime_method(&struct_name, method)
                        {
                            return Err(TypeError::ComptimeMethodCallOutsideComptime {
                                type_name: struct_name,
                                method_name: method.clone(),
                            });
                        }
                    }
                }

                // Try to resolve the method statically using the method table.
                // D1 (S4): apply substitutions to the receiver FIRST so an
                // `ElementOf(ReceiverParam(0))` return projection
                // (`Array<int>.sum()` → `int`) sees the receiver's element
                // type once it has been bound by the array-literal elements.
                // Without this, an inline literal receiver (`[1,2,3].sum()`)
                // still has an un-applied element var at resolution time and
                // `ElementOf` falls back to an OOB placeholder.
                let resolved_receiver = self.unifier.apply_substitutions(&receiver_type);
                if let Some(result_type) = self.method_table.resolve_method_call(
                    &resolved_receiver,
                    method,
                    &arg_types,
                    &mut self.type_var_gen,
                ) {
                    // Apply substitutions so receiver type params bound just
                    // above from the value-position args (e.g. K=string, V=int
                    // for `HashMap().set("a", 1)`) are reflected in the result
                    // type. Without this the result stays `HashMap<K, V>` and the
                    // let-gen function-return gate rejects it as unresolved.
                    return Ok(self.unifier.apply_substitutions(&result_type));
                }

                // REAL-MOVE keep-both (v0.3.3, user 2026-06-21): `clone p`
                // desugars to `p.clone()` (desugar.rs:640). Arrays / strings
                // resolve `clone` via the method-table (handled above); a
                // user-defined struct (`type P { ... }`) has no PHF `clone`
                // entry, so it would otherwise fall through to the deferred
                // `HasMethod` constraint and reject with
                // "Method 'clone' not found on type 'P'". `clone` is a
                // universal deep-copy returning `Self`: for any CONCRETE
                // object/struct receiver with zero args, resolve the result
                // type to the receiver type itself. Bounded to concrete
                // object-like receivers so unresolved generics and real
                // missing-method cases still surface their own diagnostics.
                if method == "clone" && arg_types.is_empty() {
                    let is_concrete_objectlike = matches!(
                        &receiver_type,
                        Type::Concrete(TypeAnnotation::Object(_))
                            | Type::Concrete(TypeAnnotation::Reference(_))
                    );
                    if is_concrete_objectlike {
                        return Ok(self.unifier.apply_substitutions(&receiver_type));
                    }
                }

                // STRICT-FLIP (v0.3.3, SMOKE-s5): method call on a `dyn Trait`
                // receiver resolves against the trait's declared method
                // signatures. `let arr: Array<dyn HasX> = [...]` makes
                // `arr[0]` a `dyn HasX`; `arr[0].x_str()` must find `x_str`'s
                // return type on the trait. Without this the call falls through
                // to `infer_property_access` → a `HasField` constraint on the
                // Dyn type → "cannot have fields". Look up each trait in the
                // dyn set and return the first matching method's return type.
                // Sound: only resolves a method the trait actually declares
                // (required signature or default body); an unknown method still
                // falls through and is rejected.
                if let Type::Concrete(TypeAnnotation::Dyn(traits)) = &receiver_type {
                    use shape_ast::ast::{TraitMember, TraitMemberSignature};
                    for trait_path in traits {
                        let Some(trait_def) = self.env.lookup_trait(trait_path.as_str()) else {
                            continue;
                        };
                        for member in &trait_def.members {
                            let method_return = match member {
                                TraitMember::Required(TraitMemberSignature::Method {
                                    name,
                                    return_type,
                                    ..
                                }) if name == method => Some(return_type.clone()),
                                TraitMember::Default(method_def) if method_def.name == *method => {
                                    method_def.return_type.clone()
                                }
                                _ => None,
                            };
                            if let Some(ret_ann) = method_return {
                                return Ok(Type::Concrete(ret_ann));
                            }
                        }
                    }
                }

                // Fallback: treat receiver.method(...) as a callable field access
                // when the receiver is concretely object-like.
                //
                // For unresolved generic/constrained receivers (e.g. T: Displayable),
                // forcing a HasField constraint here over-constrains the receiver to a
                // structural object shape and breaks trait-bound method dispatch.
                let can_try_callable_field =
                    !matches!(&receiver_type, Type::Variable(_) | Type::Constrained { .. });
                if can_try_callable_field {
                    if let Ok(field_type) = self.infer_property_access(&receiver_type, method) {
                        match field_type {
                            Type::Concrete(TypeAnnotation::Function { params, returns }) => {
                                let required_count = params.iter().filter(|p| !p.optional).count();
                                if arg_types.len() < required_count
                                    || arg_types.len() > params.len()
                                {
                                    return Err(TypeError::ArityMismatch(
                                        required_count,
                                        arg_types.len(),
                                    ));
                                }

                                for (arg_ty, param) in arg_types.iter().zip(params.iter()) {
                                    // Decode a tyvar marker (a closure-valued field whose
                                    // param is still an unresolved TypeVar — e.g.
                                    // `{ greet: |name| ... }`) back to a Variable so the
                                    // call arg substitutes it, instead of failing the
                                    // unsolvable constraint `arg ~ "tyvar:Tn"`.
                                    let param_ty = match annotation_as_tyvar(&param.type_annotation)
                                    {
                                        Some(var) => Type::Variable(var),
                                        None => Type::Concrete(param.type_annotation.clone()),
                                    };
                                    self.constraints.push((arg_ty.clone(), param_ty));
                                }

                                let ret = match annotation_as_tyvar(&returns) {
                                    Some(var) => {
                                        self.unifier.apply_substitutions(&Type::Variable(var))
                                    }
                                    None => Type::Concrete(*returns),
                                };
                                return Ok(ret);
                            }
                            Type::Function { params, returns } => {
                                if params.len() != arg_types.len() {
                                    return Err(TypeError::ArityMismatch(
                                        params.len(),
                                        arg_types.len(),
                                    ));
                                }

                                for (arg_ty, param_ty) in arg_types.iter().zip(params.iter()) {
                                    self.constraints.push((arg_ty.clone(), param_ty.clone()));
                                }

                                return Ok(*returns);
                            }
                            _ => {}
                        }
                    }
                }

                // Method not found in table - create a fresh type variable
                // This allows code to compile while deferring to runtime resolution
                // for user-defined methods or extension methods
                let result_type = self.fresh_type_var();
                let hm_var = self.fresh_var();

                // Create a constraint that receiver must have this method
                self.constraints.push((
                    Type::Constrained {
                        var: hm_var,
                        constraint: Box::new(TypeConstraint::HasMethod {
                            method_name: method.clone(),
                            arg_types: arg_types.clone(),
                            return_type: Box::new(result_type.clone()),
                        }),
                    },
                    receiver_type,
                ));

                Ok(result_type)
            }

            // Match expression
            Expr::Match(match_expr, span) => {
                let raw_scrutinee_type = self.infer_expr(&match_expr.scrutinee)?;
                // F5 (v0.3.3 strict-flip): zonk the scrutinee through the
                // unifier's substitution store BEFORE binding pattern vars.
                // `match Ok(5) { Ok(v) => v * 2 }` infers the scrutinee as
                // `Result<T, E>` where the payload var `T` is constrained to
                // `int` (the literal-defers-to-var rule at
                // `bidirectional.rs:constructor_literal_payload_defers_to_var`
                // unifies `T = int` rather than pinning the literal). Without
                // applying substitutions, the builtin Result/Option payload
                // extractor below reads `args[0]` as the raw `Variable`, so
                // `v` degrades to a fresh var and `v * 2` rejects as
                // `unknown * int`. Applying substitutions resolves `T → int`;
                // no fabrication — the binder type comes verbatim from the
                // already-registered constraint.
                let scrutinee_type = self.unifier.apply_substitutions(&raw_scrutinee_type);

                // Collect all arm return types
                let mut arm_types: Vec<Type> = Vec::new();

                for arm in &match_expr.arms {
                    self.env.push_scope();

                    // Bind pattern variables. WS-4 4b: pass the scrutinee
                    // type so object/struct patterns bind each field to
                    // its declared type rather than a fresh type var.
                    self.bind_pattern_vars_typed(&arm.pattern, Some(&scrutinee_type))?;

                    // Check guard if present
                    if let Some(guard) = &arm.guard {
                        let guard_type = self.infer_expr(guard)?;
                        self.constraints.push((guard_type, BuiltinTypes::boolean()));
                    }

                    let body_type = self.infer_expr(&arm.body)?;
                    arm_types.push(body_type);

                    self.env.pop_scope();
                }

                // Check exhaustiveness for closed types (enums, unions).
                // Union-typed scrutinees use typed-pattern coverage based on concrete variants.
                let result = if matches!(
                    scrutinee_type.to_annotation(),
                    Some(TypeAnnotation::Union(_))
                ) {
                    exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type)
                } else if let Some(semantic_type) = scrutinee_type.to_semantic() {
                    let resolved_type = self.resolve_named_to_enum(&semantic_type);
                    match resolved_type {
                        crate::type_system::semantic::SemanticType::Enum { .. } => {
                            exhaustiveness::check_exhaustiveness(match_expr, &resolved_type)
                        }
                        _ => exhaustiveness::check_exhaustiveness_for_type(
                            match_expr,
                            &scrutinee_type,
                        ),
                    }
                } else {
                    exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type)
                };
                if let Some(error) = result.to_error() {
                    if let TypeError::NonExhaustiveMatch { enum_name, .. } = &error {
                        self.register_non_exhaustive_match_origin(enum_name, *span);
                    }
                    return Err(error);
                }

                // Reconcile match-arm result types before union formation.
                //
                // Two reconciliations, both established Shape behavior that the
                // structural `all_types_equal` check misses:
                //
                // (1) Free-variable arms. A payload binder for a built-in
                //     Result/Option whose element type was never pinned (e.g.
                //     `match b { Ok(v) => v, ... }` where `b = Err("fail")`
                //     leaves the success element a free var) yields an arm whose
                //     type is a bare `Type::Variable`. A free variable has NO
                //     committed type, so it must unify with the other arms'
                //     common type — match arms share a single type. Constrain
                //     each free-variable arm to the common concrete type rather
                //     than poisoning the result with a `Union<unknown, …>`.
                //
                // (2) Numeric int/number arms. An integer literal arm
                //     (`Err(_) => 0`) sitting beside a `number` arm collapses to
                //     `number` — the integer literal is number-compatible (cf.
                //     `let x: number = 0`, `match b { true => 1.0, false => 0 }`
                //     typed `number`). This is not int↔number unification of
                //     distinct values; it is literal widening at the arm join,
                //     and a genuine int/number mismatch still rejects downstream
                //     against an `-> int` return.
                //
                // Pure inference completeness; no coercion opcode, no dynamic
                // fallback. A genuinely heterogeneous match (non-numeric,
                // non-variable arms that disagree) still forms the nominal union
                // below.
                let non_var_types: Vec<Type> = arm_types
                    .iter()
                    .filter(|t| !matches!(t, Type::Variable(_)))
                    .cloned()
                    .collect();
                let has_var_arm = non_var_types.len() < arm_types.len();

                // Determine the common concrete arm type, if one exists:
                //   - all non-var arms structurally equal → that type; or
                //   - all non-var arms numeric (int/number) → `number` if any
                //     is `number`, else `int`.
                let is_int =
                    |t: &Type| matches!(t, Type::Concrete(TypeAnnotation::Basic(n)) if n == "int");
                let is_number = |t: &Type| matches!(t, Type::Concrete(TypeAnnotation::Basic(n)) if n == "number");
                // Same-base generic arms (e.g. every arm a `Result<…>` /
                // `Option<…>`): a match whose arms all build the same generic
                // family yields that family, not a nominal union of the
                // per-arm instantiations. Take the first arm's type and
                // constrain the rest to it; the solver's (Generic, Generic)
                // arm — including the Result error-param / AnyError lattice —
                // reconciles the args (e.g. `Result<int, AnyError>` absorbs
                // `Result<int, string>`). Without this, concretely-typed arms
                // (now that the Ok/Some payload binder is no longer a free var)
                // would form a nominal union that the constructor-pattern
                // exhaustiveness path cannot cover.
                let generic_base = |t: &Type| match t {
                    Type::Generic { base, .. } => Some((**base).clone()),
                    _ => None,
                };
                let all_same_base_generic = non_var_types.len() > 1
                    && non_var_types
                        .iter()
                        .all(|t| matches!(t, Type::Generic { .. }))
                    && {
                        let first_base = generic_base(&non_var_types[0]);
                        first_base.is_some()
                            && non_var_types
                                .iter()
                                .all(|t| match (&first_base, generic_base(t)) {
                                    (Some(b0), Some(b)) => self.types_equal(b0, &b),
                                    _ => false,
                                })
                    };

                let common: Option<Type> = if non_var_types.is_empty() {
                    None
                } else if non_var_types
                    .iter()
                    .all(|t| self.types_equal(&non_var_types[0], t))
                {
                    Some(non_var_types[0].clone())
                } else if non_var_types.iter().all(|t| is_int(t) || is_number(t)) {
                    if non_var_types.iter().any(is_number) {
                        Some(BuiltinTypes::number())
                    } else {
                        Some(BuiltinTypes::integer())
                    }
                } else if all_same_base_generic {
                    let head = non_var_types[0].clone();
                    for t in non_var_types.iter().skip(1) {
                        self.constraints.push((t.clone(), head.clone()));
                    }
                    Some(head)
                } else {
                    None
                };

                if let Some(common) = common {
                    if has_var_arm {
                        for t in &arm_types {
                            if matches!(t, Type::Variable(_)) {
                                self.constraints.push((t.clone(), common.clone()));
                            }
                        }
                    }
                    return Ok(common);
                }

                // Determine result type: unify if same, create nominal union if different
                let result_type = if arm_types.is_empty() {
                    self.fresh_type_var()
                } else if self.all_types_equal(&arm_types) {
                    // All arms have the same type - use that type
                    arm_types[0].clone()
                } else {
                    // Heterogeneous arms - create NOMINAL union type with auto-generated brand
                    self.create_nominal_union(&arm_types)?
                };

                Ok(result_type)
            }

            // If expression
            Expr::If(if_expr, _) => {
                let cond_type = self.infer_expr(&if_expr.condition)?;
                self.constraints.push((cond_type, BuiltinTypes::boolean()));

                let then_type = self.infer_expr(&if_expr.then_branch)?;

                if let Some(else_branch) = &if_expr.else_branch {
                    let else_type = self.infer_expr(else_branch)?;
                    self.constraints.push((then_type.clone(), else_type));
                }

                Ok(then_type)
            }

            // While expression
            Expr::While(while_expr, _) => {
                let cond_type = self.infer_expr(&while_expr.condition)?;
                self.constraints.push((cond_type, BuiltinTypes::boolean()));

                // Barrier scope: catches `break <value>` so it does not leak
                // into an enclosing `loop`'s break-type collection. `while`
                // itself is always Void.
                self.push_break_scope();
                let body_result = self.infer_expr(&while_expr.body);
                self.pop_break_scope();
                body_result?;
                Ok(BuiltinTypes::void())
            }

            // For expression
            Expr::For(for_expr, _) => {
                self.env.push_scope();

                let iter_type = self.infer_expr(&for_expr.iterable)?;
                let element_type = self.infer_iterator_element_type(&iter_type)?;

                // Bind every identifier the loop pattern introduces — simple
                // (`for n in ...`), object (`for {x, y} in ...`), or array
                // (`for [a, b] in ...`). `as_simple_name()` returns `Some` only
                // for Identifier/Typed, so object/array destructure patterns
                // previously bound NOTHING and the body's references failed with
                // `Undefined variable`. The element-type granularity matches the
                // prior behavior (bind the WHOLE element type to each name);
                // destructure-precise field typing is not required to keep the
                // bindings in scope.
                fn collect_pattern_names(p: &shape_ast::ast::Pattern, out: &mut Vec<String>) {
                    use shape_ast::ast::Pattern::*;
                    match p {
                        Identifier(n) => out.push(n.clone()),
                        Typed { name, .. } => out.push(name.clone()),
                        Object(fields) => {
                            for (_k, sub) in fields {
                                collect_pattern_names(sub, out);
                            }
                        }
                        Array(items) => {
                            for sub in items {
                                collect_pattern_names(sub, out);
                            }
                        }
                        Constructor { .. } | Literal(_) | Wildcard => {}
                    }
                }
                // ROOT-1 (strict-flip, 2026-06-18): an OBJECT-destructuring
                // for-in (`for {x, y} in [P{..}]`) must type each binder from
                // the element struct's declared FIELD annotation, not the whole
                // element struct — else the body's `x + y` rejects with "P does
                // not implement Numeric" (the engine's trait-bound check on
                // `+`). Resolve the element's struct name and bind each field by
                // its declared field type. A non-object pattern, or a field with
                // no resolvable type, falls back to the WHOLE element type
                // (parity with the prior bind-all behavior — no fabrication).
                let bound_via_fields = if let shape_ast::ast::Pattern::Object(fields) =
                    &for_expr.pattern
                {
                    let resolved_elem = self.unifier.apply_substitutions(&element_type);
                    if let Some(struct_name) = self
                        .struct_name_of_type(&resolved_elem)
                        .or_else(|| self.struct_name_of_type(&element_type))
                    {
                        for (key, sub) in fields {
                            let binder = match sub {
                                shape_ast::ast::Pattern::Identifier(n) => n.as_str(),
                                _ => key.as_str(),
                            };
                            let field_ty = self
                                .struct_field_annotation(&struct_name, key)
                                .map(|ann| self.resolve_type_annotation(&ann))
                                .unwrap_or_else(|| element_type.clone());
                            self.env.define(binder, TypeScheme::mono(field_ty));
                        }
                        true
                    } else if let Type::Concrete(TypeAnnotation::Object(elem_fields)) =
                        &resolved_elem
                    {
                        // T1 sub-case (d) (strict-flip, 2026-06-20): an ANONYMOUS
                        // object-literal element (`for {x, y} in [{x: 1, y: 2}]`)
                        // has no registered struct name, so the struct-name path
                        // above misses and the prior code bound the WHOLE object
                        // type to each field — making `x + y` reject (`int` field
                        // vs whole-object operand) / collapse the field to a
                        // number-vs-int unification clash. Bind each destructured
                        // field from the element's own recorded field annotation
                        // (the object-literal inference at `Expr::Object` already
                        // froze `1` -> `int`, ADR-006 §2.7.5). A field absent from
                        // the element type, or a destructure key with no matching
                        // field, falls back to the whole element type (parity,
                        // no fabrication). PER-SITE-ARM, int != number preserved.
                        for (key, sub) in fields {
                            let binder = match sub {
                                shape_ast::ast::Pattern::Identifier(n) => n.as_str(),
                                _ => key.as_str(),
                            };
                            let field_ty = elem_fields
                                .iter()
                                .find(|f| &f.name == key)
                                .map(|f| self.resolve_type_annotation(&f.type_annotation))
                                .unwrap_or_else(|| element_type.clone());
                            self.env.define(binder, TypeScheme::mono(field_ty));
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };
                if !bound_via_fields {
                    let mut pattern_names = Vec::new();
                    collect_pattern_names(&for_expr.pattern, &mut pattern_names);
                    for name in pattern_names {
                        self.env
                            .define(&name, TypeScheme::mono(element_type.clone()));
                    }
                }

                // Barrier scope: catches `break <value>` so it does not leak
                // into an enclosing `loop`'s break-type collection. `for`
                // itself is always Void.
                self.push_break_scope();
                let body_result = self.infer_expr(&for_expr.body);
                self.pop_break_scope();
                self.env.pop_scope();
                body_result?;

                // For expressions return void (or collected values if used as expression)
                Ok(BuiltinTypes::void())
            }

            // Loop expression — `loop { ... break v }` is an expression whose
            // type is the unified type of every `break <value>` it can take
            // (control-flow.mdx "Break with Value"). A `loop` with no
            // value-carrying break stays Void (it never produces a value).
            Expr::Loop(loop_expr, _) => {
                self.push_break_scope();
                let body_result = self.infer_expr(&loop_expr.body);
                let break_types = self.pop_break_scope();
                body_result?;

                if break_types.is_empty() {
                    Ok(BuiltinTypes::void())
                } else {
                    // Every `break <value>` in the same `loop` must agree on a
                    // single type — `loop` is no more permissive than `if`/
                    // `match`, which push a direct `(then, else)` / arm-vs-arm
                    // unify constraint (see `Expr::If` above). Without these
                    // pairwise constraints, `combine_return_types` would brand a
                    // *nominal union* over mismatched break types and let it leak
                    // through the binding annotation (e.g. `let r: int = loop {
                    // break "s"; break 7 }` would compile and bind a string into
                    // an `int` slot). int!=number never unify here either — a
                    // mixed `break 1` / `break 2.0` is a compile error, no silent
                    // coercion. Constrain each break type to the first; the
                    // constraint solver surfaces the mismatch as a real type
                    // error rather than a fabricated union.
                    let head = break_types[0].clone();
                    for other in break_types.iter().skip(1) {
                        self.constraints.push((head.clone(), other.clone()));
                    }
                    self.combine_return_types(&break_types)
                }
            }

            // Let expression
            Expr::Let(let_expr, _) => {
                self.env.push_scope();

                let var_type = if let Some(ann) = &let_expr.type_annotation {
                    self.resolve_type_annotation(ann)
                } else {
                    self.fresh_type_var()
                };

                if let Some(value) = &let_expr.value {
                    let value_type = self.infer_expr(value)?;
                    self.constraints.push((var_type.clone(), value_type));
                }

                if let Some(name) = let_expr.pattern.as_simple_name() {
                    self.env.define(name, TypeScheme::mono(var_type));
                }

                let body_type = self.infer_expr(&let_expr.body)?;
                self.env.pop_scope();

                Ok(body_type)
            }

            // Assignment expression
            Expr::Assign(assign_expr, _) => {
                let value_type = self.infer_expr(&assign_expr.value)?;
                let target_type = if let Expr::PropertyAccess {
                    object, property, ..
                } = assign_expr.target.as_ref()
                {
                    if let Expr::Identifier(var_name, _) = object.as_ref() {
                        // Mark first so `a.y = ...` can resolve a hoisted field target even
                        // before it has been read once.
                        self.env.mark_hoisted_field_initialized(var_name, property);

                        self.env.set_current_access_variable(Some(var_name.clone()));
                        let target = self.infer_expr(object).and_then(|object_type| {
                            self.infer_property_assignment_target(&object_type, property)
                        });
                        self.env.set_current_access_variable(None);
                        target?
                    } else {
                        self.infer_expr(&assign_expr.target)?
                    }
                } else {
                    self.infer_expr(&assign_expr.target)?
                };

                // Assignment must be type-compatible with the target field/variable.
                // Numeric-conversion §4 literal adoption (field/property-assignment
                // context): a bare int-literal RHS into a concrete numeric target
                // (`p.x = 10` where `x: number`) adopts the target type when the
                // literal value is losslessly representable — no rejecting
                // constraint, mirroring the construction-side adoption
                // (`collections.rs::int_literal_adopts_field_type`) and the scalar
                // reassignment path (`statements.rs::infer_assignment`). A
                // non-literal RHS (`p.x = int_var`) keeps the value-level §2 lattice
                // constraint, so an int VARIABLE into a number field still rejects.
                if Self::adopt_int_literal_in_context(&assign_expr.value, &target_type).is_some() {
                    // literal fits the target — no rejecting constraint.
                } else {
                    self.constraints
                        .push((target_type.clone(), value_type.clone()));
                }

                // Record field evolution for property assignments (a.x = v)
                if let Expr::PropertyAccess {
                    object, property, ..
                } = assign_expr.target.as_ref()
                {
                    if let Expr::Identifier(var_name, _) = object.as_ref() {
                        // Keep the variable's object shape in sync for later expressions.
                        self.env
                            .upsert_object_field(var_name, property, value_type.clone());

                        // Convert inference type to semantic type for evolution tracking
                        if let Some(semantic_type) = value_type.to_semantic() {
                            // Ignore errors - evolution tracking is best-effort
                            let _ =
                                self.env
                                    .record_field_assignment(var_name, property, semantic_type);
                        }
                    }
                }

                // Assignment returns the value type
                Ok(value_type)
            }

            // Block expression
            Expr::Block(block, block_span) => {
                self.env.push_scope();
                let mut last_type = BuiltinTypes::void();

                for item in &block.items {
                    last_type = match item {
                        shape_ast::ast::BlockItem::VariableDecl(decl) => {
                            self.infer_variable_decl(decl)?;
                            BuiltinTypes::void()
                        }
                        // A loop body is parsed as an `Expr::Block` whose items
                        // are `BlockItem::Assignment` — the RHS of a loop-body
                        // assignment (`last = dbl(11)`) must be walked here, or
                        // its function callsites never get recorded and an
                        // unannotated callee parameter collapses to the
                        // `number` default (kind-confused silent-wrong result).
                        shape_ast::ast::BlockItem::Assignment(assign) => {
                            self.infer_assignment(assign, *block_span)?;
                            BuiltinTypes::void()
                        }
                        // A `BlockItem::Statement` is a non-tail / `;`-terminated
                        // item — the parser only leaves the trailing value as
                        // `BlockItem::Expression` (and promotes a trailing
                        // value-`if`/`match` to one). A statement therefore
                        // contributes NOTHING to the block's value: it discards
                        // to Unit. This is what honors `;`-discard at an if/else
                        // branch tail (`if c { f(); } else { g(); }` is Unit, not
                        // forced into f()/g()'s type). We still walk the
                        // statement for its constraints / implicit-return effects.
                        shape_ast::ast::BlockItem::Statement(stmt) => {
                            self.infer_statement(stmt)?;
                            BuiltinTypes::void()
                        }
                        shape_ast::ast::BlockItem::Expression(expr) => self.infer_expr(expr)?,
                    };
                }

                self.env.pop_scope();
                Ok(last_type)
            }

            // Function expression
            Expr::FunctionExpr {
                params,
                return_type,
                body,
                ..
            } => {
                self.env.push_scope();
                self.push_fallible_scope();

                let mut param_types = Vec::new();
                for param in params {
                    let param_type = if let Some(ann) = &param.type_annotation {
                        // Resolve through resolve_type_annotation (as
                        // infer_function does for top-level fns) so a generic
                        // annotation becomes canonical `Type::Generic { base,
                        // args }` rather than the non-canonical
                        // `Type::Concrete(Generic)` the solver has no arm for.
                        self.resolve_type_annotation(ann)
                    } else {
                        self.fresh_type_var()
                    };
                    param_types.push(param_type.clone());
                    // Define all identifiers from the pattern
                    for name in param.get_identifiers() {
                        self.env.define(&name, TypeScheme::mono(param_type.clone()));
                    }
                }

                let local_constraint_start = self.constraints.len();
                let inferred_result = self.infer_callable_return_type(body, return_type.is_some());
                let numeric_param_indices = self
                    .refine_callable_param_types_from_local_constraints(
                        &mut param_types,
                        &self.constraints[local_constraint_start..],
                        true,
                    );
                // ROOT-2: a `Numeric`-bounded unannotated closure param is no
                // longer collapsed to `number` inside the refine helper (that
                // severed the call-site link for `let f = |x| x * 2; f(i: int)`).
                // Record its source variable so a NEVER-called closure still
                // defaults to `number` post-solve
                // (`default_unresolved_closure_numeric_params`), while a called
                // closure resolves its param from the concrete argument type.
                for &index in &numeric_param_indices {
                    if let Some(Type::Variable(var)) = param_types.get(index) {
                        self.deferred_closure_numeric_param_vars.insert(var.clone());
                    }
                }
                let is_fallible = self.pop_fallible_scope();
                self.env.pop_scope();
                let inferred_return = inferred_result?;

                let ret_type = if let Some(ann) = return_type {
                    // Resolve through resolve_type_annotation (as infer_function
                    // does) so a generic return annotation becomes canonical
                    // `Type::Generic { base, args }`; the constraint then routes
                    // through the existing (Generic, Generic) solver arm instead
                    // of falling to the unsolved wildcard.
                    let annotated = self.resolve_type_annotation(ann);
                    self.constraints.push((inferred_return, annotated.clone()));
                    annotated
                } else {
                    inferred_return
                };
                let ret_type = self.apply_fallibility_to_return_type(ret_type, is_fallible);

                Ok(BuiltinTypes::function(param_types, ret_type))
            }

            // List comprehension
            Expr::ListComprehension(comp, _) => {
                self.env.push_scope();

                // Process each clause (for x in items, for y in other_items, etc.)
                for clause in &comp.clauses {
                    let iter_type = self.infer_expr(&clause.iterable)?;
                    let element_type = self.infer_iterator_element_type(&iter_type)?;

                    if let Some(name) = clause.pattern.as_identifier() {
                        self.env.define(name, TypeScheme::mono(element_type));
                    }

                    if let Some(filter) = &clause.filter {
                        let cond_type = self.infer_expr(filter)?;
                        self.constraints.push((cond_type, BuiltinTypes::boolean()));
                    }
                }

                let elem_type = self.infer_expr(&comp.element)?;
                self.env.pop_scope();

                Ok(BuiltinTypes::array(elem_type))
            }

            // Data references - return generic object type
            Expr::DataRef(_, _) | Expr::DataDateTimeRef(_, _) => {
                Ok(Type::Concrete(TypeAnnotation::Basic("object".to_string())))
            }

            // Data relative access
            Expr::DataRelativeAccess { .. } => {
                Ok(Type::Concrete(TypeAnnotation::Basic("object".to_string())))
            }

            // Time references / `@"..."` datetime literals.
            //
            // Infer the canonical `DateTime` reference type (NOT a lowercase
            // `Basic("datetime")`). The strict method-checker's `MethodTable`
            // registers the 30 DateTime instance methods under the key
            // `"DateTime"` (`checking/method_table.rs::register_datetime_methods`),
            // and `MethodTable::lookup` keys off the receiver type name. A
            // lowercase `Basic("datetime")` produced a `("datetime", "year")`
            // key that misses every seeded signature, so `let d = @"..."` then
            // `d.year()` reported "Method 'year' not found on type 'datetime'".
            // The downstream concrete-conversion / compiler arithmetic sites
            // already accept both `"DateTime"` and `"datetime"`.
            Expr::TimeRef(_, _) | Expr::DateTime(_, _) => Ok(Type::Concrete(
                TypeAnnotation::Reference("DateTime".into()),
            )),

            // Duration
            Expr::Duration(_, _) => Ok(Type::Concrete(TypeAnnotation::Basic(
                "duration".to_string(),
            ))),

            // Pattern reference
            Expr::PatternRef(_, _) => Ok(BuiltinTypes::pattern()),

            // Spread expression
            Expr::Spread(inner, _) => self.infer_expr(inner),

            // Range expression
            Expr::Range { start, end, .. } => {
                let element_type = if let Some(s) = start {
                    let start_type = self.infer_expr(s)?;
                    if let Some(e) = end {
                        let end_type = self.infer_expr(e)?;
                        self.constraints.push((start_type.clone(), end_type));
                    }
                    start_type
                } else if let Some(e) = end {
                    self.infer_expr(e)?
                } else {
                    self.fresh_type_var()
                };
                Ok(Type::Concrete(TypeAnnotation::Generic {
                    name: "Range".into(),
                    args: vec![
                        element_type
                            .to_annotation()
                            .unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
                    ],
                }))
            }

            // Timeframe context
            Expr::TimeframeContext { expr, span: _, .. } => self.infer_expr(expr),

            // Control flow - these return void or break/continue semantics
            Expr::Break(value, _) => {
                if let Some(val) = value {
                    let val_type = self.infer_expr(val)?;
                    // Record into the innermost enclosing loop construct so a
                    // `loop` can collect+unify its break-value types. `for`/
                    // `while` install barrier scopes that absorb (and discard)
                    // the value. `break` itself diverges, so it yields the
                    // never-type (void) as its own expression value.
                    self.record_break_type(val_type);
                }
                Ok(BuiltinTypes::void())
            }

            Expr::Continue(_) => Ok(BuiltinTypes::void()),

            Expr::Return(value, _) => {
                let return_type = if let Some(val) = value {
                    self.infer_expr(val)?
                } else {
                    BuiltinTypes::void()
                };
                self.record_return_type(return_type.clone());
                Ok(return_type)
            }

            // Unit
            Expr::Unit(_) => Ok(BuiltinTypes::void()),

            // Try operator for Result/Option propagation
            // The ? operator:
            // 1. Supports Result<T> and Option<T> / T? values
            // 2. Extracts and returns the inner success type
            // 3. Marks the containing function as fallible (contagious Result)
            Expr::TryOperator(inner, _) => {
                let inner_type = self.infer_expr(inner)?;

                // Mark the current function scope as fallible
                self.mark_current_scope_fallible();

                if let Some(unwrapped) = self.try_unwrap_inner_type(&inner_type) {
                    return Ok(unwrapped);
                }

                // When the inner type is an unresolved type variable (e.g. untyped
                // lambda parameter), we cannot reject it — it may later resolve to
                // Result<T,E> or Option<T>.  Return a fresh type variable for the
                // unwrapped value and let downstream constraints refine it.
                if self.type_contains_unresolved_vars(&inner_type) {
                    return Ok(self.fresh_type_var());
                }

                Err(TypeError::ConstraintViolation(format!(
                    "try operator '?' expects Result<T, E> or Option<T>, found '{}'",
                    self.render_type_for_diag(&inner_type)
                )))
            }

            // Named impl selector does not change the value type.
            // Trait-specific validation happens in call sites (e.g. formatting).
            Expr::UsingImpl { expr, .. } => self.infer_expr(expr),

            // Simulation call with inline parameters
            Expr::SimulationCall {
                name: _, params, ..
            } => {
                // Infer types for all parameter expressions
                for (_, value_expr) in params {
                    self.infer_expr(value_expr)?;
                }
                // Return a fresh type variable - actual type depends on runtime
                Ok(self.fresh_type_var())
            }

            // Window expressions return numbers
            Expr::WindowExpr(_, _) => Ok(BuiltinTypes::number()),

            // Fuzzy comparisons return boolean
            Expr::FuzzyComparison { left, right, .. } => {
                self.infer_expr(left)?;
                self.infer_expr(right)?;
                Ok(BuiltinTypes::boolean())
            }

            // FromQuery should be desugared before type inference
            // If we see one, treat it as returning Array of the select type
            Expr::FromQuery(from_query, _) => {
                // Infer source type (should be an array)
                let _source_ty = self.infer_expr(&from_query.source)?;
                // Infer clause expressions
                for clause in &from_query.clauses {
                    match clause {
                        shape_ast::QueryClause::Where(pred) => {
                            self.infer_expr(pred)?;
                        }
                        shape_ast::QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                self.infer_expr(&spec.key)?;
                            }
                        }
                        shape_ast::QueryClause::GroupBy { element, key, .. } => {
                            self.infer_expr(element)?;
                            self.infer_expr(key)?;
                        }
                        shape_ast::QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            self.infer_expr(source)?;
                            self.infer_expr(left_key)?;
                            self.infer_expr(right_key)?;
                        }
                        shape_ast::QueryClause::Let { value, .. } => {
                            self.infer_expr(value)?;
                        }
                    }
                }
                let select_ty = self.infer_expr(&from_query.select)?;
                Ok(BuiltinTypes::array(select_ty))
            }
            Expr::StructLiteral {
                type_name, fields, ..
            } => self.infer_struct_literal_type(type_name, fields),

            // Await expression - infer the type of the inner expression
            Expr::Await(inner, _) => self.infer_expr(inner),

            // Join expression - infer types of all branches
            Expr::Join(join_expr, _) => {
                for branch in &join_expr.branches {
                    self.infer_expr(&branch.expr)?;
                }
                Ok(self.fresh_type_var())
            }

            // Annotated expression - infer the type of the target
            Expr::Annotated { target, .. } => self.infer_expr(target),

            // Async let - spawns a task and binds a future handle. Without a
            // surface `Future<T>` type in the inference lattice, the binding's
            // type unifies with the inner expression's type — `await x` then
            // re-uses the same `infer_expr(inner)` shape and returns the
            // inner kind cleanly. The sync-resolution + op_spawn_task
            // non-callable path (async_ops/mod.rs::op_spawn_task) preserves
            // the inner value's kind end-to-end at runtime, so the inference
            // shape matches the runtime behavior. Without this, multi-binding
            // patterns like `let va = await a; let vb = await b; print(va + vb)`
            // surface "Cannot infer types for binary operation Add: operand
            // types are unknown and unknown" because both va and vb were
            // typed as fresh type vars.
            Expr::AsyncLet(async_let, _) => {
                let inner_type = self.infer_expr(&async_let.expr)?;
                // Register the binding into the CURRENT scope (no new scope —
                // `async let x = expr` is statement-positioned, so `x` must
                // remain visible to the sibling statements that follow,
                // including a later `await x`. Mirrors the ordinary `let`
                // binding registration but without the let-in body/scope shape.
                // Without this the analyzer scope never learns `x` and the
                // subsequent `await x` is wrongly rejected as an undefined
                // variable, even though the compiler + VM bind it correctly.
                self.env
                    .define(&async_let.name, TypeScheme::mono(inner_type.clone()));
                Ok(inner_type)
            }

            // Async scope - cancellation boundary, type is the body's type
            Expr::AsyncScope(inner, _) => self.infer_expr(inner),

            // Comptime block - evaluated at compile time, returns Any for now
            //
            // J-CT.1: also push the engine's comptime-depth so nested method
            // calls on `comptime impl`-registered methods are accepted here.
            // Statements are walked for side-effect type checking; per-stmt
            // errors are tolerated the same way `infer_item` tolerates them
            // for the top-level fallthrough path.
            Expr::Comptime(stmts, _) => {
                self.enter_comptime();
                for stmt in stmts {
                    let _ = self.infer_statement(stmt);
                }
                self.exit_comptime();
                Ok(self.fresh_type_var())
            }

            // Comptime for - unrolled at compile time, returns Unit
            //
            // J-CT.1: ComptimeFor is itself a comptime context — calls to
            // `comptime impl` methods inside its body must type-check.
            Expr::ComptimeFor(cf, _) => {
                self.enter_comptime();
                for stmt in &cf.body {
                    let _ = self.infer_statement(stmt);
                }
                self.exit_comptime();
                Ok(Type::Concrete(TypeAnnotation::Void))
            }

            // Reference expression (R1/GAP-2): `&expr` / `&mut expr`.
            // `&expr` where `expr: T` is typed as `&T`
            // (`Type::Concrete(Borrow { mutable, inner: T })`), NOT as the bare
            // referent `T`. This lets a `-> &int` return annotation unify
            // against the inferred `&int` (Borrow-vs-Borrow recursion in
            // `annotations_equal`) instead of reporting "int is not compatible
            // with &int". The inner type is resolved through substitutions; if
            // it is still an unresolved variable (no annotation), fall back to
            // the bare inner type rather than fabricating a kind.
            Expr::Reference {
                expr: inner,
                is_mutable,
                ..
            } => {
                let inner_ty = self.infer_expr(inner)?;
                let resolved_inner = self.unifier.apply_substitutions(&inner_ty);
                match resolved_inner.to_annotation() {
                    Some(inner_ann) => Ok(Type::Concrete(TypeAnnotation::Borrow {
                        mutable: *is_mutable,
                        inner: Box::new(inner_ann),
                    })),
                    // Inner type not yet known concretely — cannot build a
                    // Borrow annotation. Keep the bare inner type (honest
                    // "not inferred"); a later pass / explicit annotation
                    // resolves it. No Bool-default, no fabricated kind.
                    None => Ok(inner_ty),
                }
            }
        }
    }

    /// Element type that an array-literal entry contributes for homogeneity
    /// unification. A spread element `...a` contributes the *element* type of
    /// `a`'s array (so `[0, ...a, 3]` unifies `int` with `int`); a plain element
    /// contributes its own type. Spreading a value whose resolved type is a
    /// concrete non-array is a genuine error and is rejected here.
    /// Numeric-conversion §4 literal adoption (array-element context): when an
    /// array literal mixes bare int literals with a float/number element
    /// (`[1, 2.5, 3]`), return the unifying `number` element type so the int
    /// literals adopt it. Returns `Some(number)` ONLY when at least one element
    /// is concretely float-family AND every other element is a bare int literal
    /// that losslessly fits `number`. Otherwise `None` (keep the existing
    /// first-element unification — homogeneous int / homogeneous number /
    /// non-numeric arrays are unaffected). Conservative on purpose: a value
    /// (non-literal) int element does NOT trigger adoption (a `Array<int>` with
    /// a number element is still a genuine mismatch under §5).
    fn array_literal_numeric_element_context(
        &self,
        elements: &[Expr],
        elem_types: &[Type],
    ) -> Option<Type> {
        let is_float_concrete = |ty: &Type| {
            let name = match ty {
                Type::Concrete(TypeAnnotation::Basic(n)) => Some(n.as_str()),
                Type::Concrete(TypeAnnotation::Reference(p)) => Some(&**p),
                _ => None,
            };
            name.is_some_and(BuiltinTypes::is_number_type_name)
        };
        let any_float = elem_types.iter().any(is_float_concrete);
        if !any_float {
            return None;
        }
        let number_ty = BuiltinTypes::number();
        // Every element must be either a float-family contribution or a bare int
        // literal that fits `number`.
        let all_adoptable = elements.iter().zip(elem_types.iter()).all(|(expr, ty)| {
            is_float_concrete(ty) || Self::adopt_int_literal_in_context(expr, &number_ty).is_some()
        });
        if all_adoptable { Some(number_ty) } else { None }
    }

    fn array_literal_element_contribution(&mut self, elem: &Expr) -> TypeResult<Type> {
        if let Expr::Spread(inner, _) = elem {
            let spread_type = self.infer_expr(inner)?;
            let resolved = self.unifier.apply_substitutions(&spread_type);
            match &resolved {
                // Concrete array forms: unwrap the element type directly.
                Type::Concrete(TypeAnnotation::Array(inner_ann)) => {
                    return Ok(Type::Concrete((**inner_ann).clone()));
                }
                Type::Concrete(TypeAnnotation::Generic { name, args })
                    if (name == "Array" || name == "Vec") && args.len() == 1 =>
                {
                    return Ok(Type::Concrete(args[0].clone()));
                }
                Type::Generic { base, args }
                    if args.len() == 1
                        && matches!(
                            base.as_ref(),
                            Type::Concrete(ann)
                                if matches!(ann.as_type_name_str(), Some("Array") | Some("Vec"))
                        ) =>
                {
                    return Ok(args[0].clone());
                }
                // Still-unresolved type variable: constrain the spread source to
                // be an array of a fresh element type so the element flows into
                // homogeneity unification without prematurely deciding the type.
                Type::Variable(_) | Type::Constrained { .. } => {
                    let elem_ty = self.fresh_type_var();
                    self.constraints
                        .push((resolved.clone(), BuiltinTypes::array(elem_ty.clone())));
                    return Ok(elem_ty);
                }
                // Concrete non-array: spreading this is a genuine type error.
                _ => {
                    let actual = self.type_name_for_union(&resolved);
                    return Err(TypeError::TypeMismatch("array".to_string(), actual));
                }
            }
        }
        self.infer_expr(elem)
    }

    fn infer_struct_literal_type(
        &mut self,
        type_name: &str,
        fields: &[(String, Expr)],
    ) -> TypeResult<Type> {
        use std::collections::HashMap;

        let mut inferred_field_types: HashMap<String, Type> = HashMap::new();
        for (field_name, value_expr) in fields {
            let field_type = self.infer_expr(value_expr)?;
            inferred_field_types.insert(field_name.clone(), field_type);
        }

        let Some(struct_def) = self.struct_type_defs.get(type_name).cloned() else {
            return Ok(Type::Concrete(TypeAnnotation::Reference(type_name.into())));
        };

        // Numeric-conversion §5 (value-level invariant) + §4 (literal adoption),
        // struct-field producer side. For each non-comptime field whose DECLARED
        // type is a concrete numeric type, the field VALUE must satisfy the §2
        // lattice against it: an `int` value into a `number` field rejects
        // (`P { x: int_var }`), while a bare int literal adopts the field type
        // when it losslessly fits (`P { x: 1 }`). A field whose declared type is
        // a generic type parameter (`x: T`) is skipped — its concrete type is
        // resolved by the monomorphization binding below. This is the
        // inference-engine twin of the compiler-side construction check
        // (`collections.rs::int_literal_adopts_field_type`); literals are
        // accepted here without a rejecting constraint so the construction site
        // can adopt them.
        let struct_param_names: std::collections::HashSet<&str> = struct_def
            .type_params
            .as_ref()
            .map(|ps| ps.iter().map(|p| p.name()).collect())
            .unwrap_or_default();
        for (field_name, value_expr) in fields {
            let Some(field_def) = struct_def.fields.iter().find(|f| f.name == *field_name) else {
                continue;
            };
            if field_def.is_comptime {
                continue;
            }
            let declared_name = match &field_def.type_annotation {
                TypeAnnotation::Basic(n) => n.as_str(),
                TypeAnnotation::Reference(n) => n.as_str(),
                _ => continue,
            };
            if struct_param_names.contains(declared_name)
                || !BuiltinTypes::is_numeric_type_name(declared_name)
            {
                continue;
            }
            // LITERAL field values are validated (and adopted) by the
            // compiler-side construction check
            // (`collections.rs::int_literal_adopts_field_type` + the c2a E0100
            // strict reject), which preserves the rich "cannot construct field"
            // diagnostic for literal mismatches (`T { i: 10.2D }`). This
            // inference-engine check only adds coverage for NON-LITERAL field
            // values (`P { x: int_var }`), which the compiler-side check skips
            // (`infer_field_type_from_expr` returns `None` for non-literals).
            if matches!(value_expr, Expr::Literal(..)) {
                continue;
            }
            let declared_type = self.resolve_type_annotation(&field_def.type_annotation);
            // The §2 lattice governs the directional `(value, field)` flow: an
            // `int` value into a `number` field rejects; a lossless-widening
            // value (`i32` into `number`) is accepted.
            if let Some(actual) = inferred_field_types.get(field_name) {
                self.constraints.push((actual.clone(), declared_type));
            }
        }

        let type_params = struct_def.type_params.unwrap_or_default();
        if type_params.is_empty() {
            return Ok(Type::Concrete(TypeAnnotation::Reference(type_name.into())));
        }

        let mut param_bindings: HashMap<String, Vec<Type>> = HashMap::new();
        for field in struct_def.fields.iter().filter(|f| !f.is_comptime) {
            let Some(actual_field_type) = inferred_field_types.get(&field.name) else {
                continue;
            };
            self.bind_type_params_from_annotation(
                &field.type_annotation,
                actual_field_type,
                &type_params,
                &mut param_bindings,
            );
        }

        let mut resolved_args: Vec<Type> = Vec::with_capacity(type_params.len());
        for tp in &type_params {
            let candidates = param_bindings.remove(tp.name()).unwrap_or_default();
            let resolved = self.resolve_struct_type_param_arg(tp, candidates)?;
            resolved_args.push(resolved);
        }

        let all_default = type_params
            .iter()
            .zip(resolved_args.iter())
            .all(|(tp, arg)| {
                self.default_type_for_type_param(tp)
                    .map_or(false, |default_type| {
                        if self.types_equal(&default_type, arg) {
                            return true;
                        }
                        match (&default_type, arg) {
                            (Type::Concrete(a), Type::Concrete(b)) => {
                                a.as_type_name_str().is_some()
                                    && a.as_type_name_str() == b.as_type_name_str()
                            }
                            _ => false,
                        }
                    })
            });

        if all_default {
            Ok(Type::Concrete(TypeAnnotation::Reference(type_name.into())))
        } else {
            Ok(Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference(type_name.into()))),
                args: resolved_args,
            })
        }
    }

    fn bind_type_params_from_annotation(
        &mut self,
        annotation: &TypeAnnotation,
        actual: &Type,
        type_params: &[shape_ast::ast::TypeParam],
        bindings: &mut std::collections::HashMap<String, Vec<Type>>,
    ) {
        let is_type_param = |name: &str| type_params.iter().any(|tp| tp.name() == name);

        match annotation {
            ann @ (TypeAnnotation::Basic(_) | TypeAnnotation::Reference(_))
                if ann.as_type_name_str().is_some_and(|n| is_type_param(n)) =>
            {
                let name = ann.as_type_name_str().unwrap();
                let entry = bindings.entry(name.to_string()).or_default();
                if !entry
                    .iter()
                    .any(|existing| self.types_equal(existing, actual))
                {
                    entry.push(actual.clone());
                }
            }
            TypeAnnotation::Array(inner) => {
                if let Type::Concrete(TypeAnnotation::Array(actual_inner)) = actual {
                    self.bind_type_params_from_annotation(
                        inner,
                        &Type::Concrete((**actual_inner).clone()),
                        type_params,
                        bindings,
                    );
                }
            }
            TypeAnnotation::Generic { name, args } => {
                if let Type::Generic {
                    base,
                    args: actual_args,
                } = actual
                {
                    let base_name = match base.as_ref() {
                        Type::Concrete(ann) => ann.as_type_name_str(),
                        _ => None,
                    };
                    if base_name == Some(name.as_str()) {
                        for (expected_arg, actual_arg) in args.iter().zip(actual_args.iter()) {
                            self.bind_type_params_from_annotation(
                                expected_arg,
                                actual_arg,
                                type_params,
                                bindings,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn resolve_struct_type_param_arg(
        &mut self,
        tp: &shape_ast::ast::TypeParam,
        candidates: Vec<Type>,
    ) -> TypeResult<Type> {
        if candidates.is_empty() {
            if let Some(default_type) = self.default_type_for_type_param(tp) {
                return Ok(default_type);
            }
            return Err(TypeError::GenericTypeError {
                message: format!(
                    "Could not infer type argument '{}' for generic struct",
                    tp.name()
                ),
                symbol: None,
            });
        }

        if candidates.len() == 1 {
            return Ok(candidates.into_iter().next().unwrap());
        }

        self.combine_return_types(&candidates)
    }

    fn default_type_for_type_param(&self, tp: &shape_ast::ast::TypeParam) -> Option<Type> {
        // Const generics carry a default *expression*, not a default *type* —
        // B.3/B.4 will resolve that to a value. Treat as "no default type".
        if let Some(default_ann) = tp.default_type() {
            return Some(Type::Concrete(default_ann.clone()));
        }
        None
    }

    fn infer_formatted_string_interpolations(
        &mut self,
        value: &str,
        mode: shape_ast::ast::InterpolationMode,
        span: Span,
    ) -> TypeResult<()> {
        let parts = parse_interpolation_with_mode(value, mode)
            .map_err(|err| TypeError::ConstraintViolation(err.to_string()))?;

        for part in parts {
            if let InterpolationPart::Expression { expr, .. } = part {
                let parsed_expr = shape_ast::parser::parse_expression_str(&expr)
                    .map_err(|err| TypeError::ConstraintViolation(err.to_string()))?;
                match self.infer_expr(&parsed_expr) {
                    Ok(_) => {}
                    Err(TypeError::UnknownProperty(type_name, property)) => {
                        self.overwrite_unknown_property_origin(&property, span);
                        return Err(TypeError::UnknownProperty(type_name, property));
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        Ok(())
    }

    /// Bind pattern variables to the type environment
    ///
    /// This recursively processes all patterns, creating fresh type variables
    /// for each bound identifier. Handles:
    /// - Simple identifiers: `x` binds x to a fresh type var
    /// - Array patterns: `[a, b]` binds a and b
    /// - Object patterns: `{x, y}` binds x and y
    /// - Constructor patterns: `Some(x)` binds x
    /// - Wildcards: `_` binds nothing
    pub(crate) fn bind_pattern_vars(
        &mut self,
        pattern: &shape_ast::ast::Pattern,
    ) -> TypeResult<()> {
        self.bind_pattern_vars_typed(pattern, None)
    }

    /// WS-4 4b: scrutinee-aware variant of [`bind_pattern_vars`]. When
    /// `scrutinee` resolves to a registered struct, an `Object` or
    /// struct-`Constructor` pattern binds each field to that field's
    /// declared type instead of a fresh type var — keeping
    /// `match p { Point { x, y } => x + y }` type-sound. `scrutinee ==
    /// None` reproduces the prior fresh-var behaviour for callers
    /// without a scrutinee type.
    pub(crate) fn bind_pattern_vars_typed(
        &mut self,
        pattern: &shape_ast::ast::Pattern,
        scrutinee: Option<&Type>,
    ) -> TypeResult<()> {
        use shape_ast::ast::{Pattern, PatternConstructorFields};

        match pattern {
            Pattern::Identifier(name) => {
                let var_type = self.fresh_type_var();
                self.env.define(name, TypeScheme::mono(var_type));
            }
            Pattern::Typed {
                name,
                type_annotation,
            } => {
                let var_type = self.resolve_type_annotation(type_annotation);
                self.env.define(name, TypeScheme::mono(var_type));
            }
            Pattern::Literal(_) => {
                // Literals don't bind variables
            }
            Pattern::Wildcard => {
                // Wildcards don't bind variables
            }
            Pattern::Array(patterns) => {
                for p in patterns {
                    self.bind_pattern_vars_typed(p, None)?;
                }
            }
            Pattern::Object(fields) => {
                let struct_name = scrutinee.and_then(|ty| self.struct_name_of_type(ty));
                for (key, p) in fields {
                    let field_ty = struct_name.as_deref().and_then(|name| {
                        self.struct_field_annotation(name, key)
                            .map(|ann| self.resolve_type_annotation(&ann))
                    });
                    self.bind_pattern_vars_typed(p, field_ty.as_ref())?;
                    // For a plain identifier field, override the
                    // fresh-var binding with the resolved field type.
                    if let (Pattern::Identifier(bind_name), Some(ft)) = (p, &field_ty) {
                        self.env.define(bind_name, TypeScheme::mono(ft.clone()));
                    }
                }
            }
            Pattern::Constructor {
                enum_name,
                variant,
                fields,
            } => {
                // R8 W7: resolve the enum's `EnumDef` from the scrutinee
                // type so enum-payload binders carry the variant's
                // declared payload types instead of unconstrained fresh
                // vars. Mirrors WS-4 4b's struct-field flow but indexed
                // positionally for tuple payloads and by name for struct
                // payloads. Falls back to fresh vars when the scrutinee
                // is non-enum (e.g. a registered struct via the Struct
                // arm, or no scrutinee at all).
                //
                // ROOT-3 (v0.3.3 strict-flip): a `match s { Shape::Circle(r)
                // => … }` where `s` is an UNANNOTATED parameter has a
                // scrutinee that is still a bare type variable, so
                // `enum_name_of_type` returns `None` and every payload binder
                // (`r`, `side`) degrades to an unconstrained fresh var. The
                // arm bodies (`3 * r * r`) then carry only a `Numeric` bound,
                // never a concrete `number`, so the match — and therefore the
                // function's inferred return type — stays an unresolved type
                // variable. `inferred_type_to_hint_name` yields `None`, the
                // compiler's `function_return_types` hint is empty, and a
                // downstream `"area=" + area(s)` rejects with `string` and
                // `unknown`.
                //
                // The constructor pattern itself names the enum + variant
                // (`Shape::Circle`), so the variant's DECLARED payload type
                // (`number`) is known WITHOUT the scrutinee type. Resolve the
                // `EnumDef` from the pattern's own `enum_name` when the
                // scrutinee could not supply it. This is the standard payload
                // propagation, just keyed off the pattern instead of the
                // scrutinee — it does not fabricate a kind and does not widen
                // an int value to number: the `number` comes verbatim from the
                // enum's declared `Circle(number)` payload annotation.
                let enum_kind = scrutinee
                    .and_then(|ty| self.enum_name_of_type(ty))
                    .or_else(|| enum_name.as_ref().map(|p| p.name().to_string()))
                    .and_then(|name| {
                        self.env.get_enum(&name).and_then(|def| {
                            def.members
                                .iter()
                                .find(|m| &m.name == variant)
                                .map(|m| m.kind.clone())
                        })
                    });

                match fields {
                    PatternConstructorFields::Unit => {
                        // No variables to bind
                    }
                    PatternConstructorFields::Tuple(patterns) => {
                        let payload_tys: Option<Vec<TypeAnnotation>> = match &enum_kind {
                            Some(shape_ast::ast::EnumMemberKind::Tuple(types)) => {
                                Some(types.clone())
                            }
                            _ => None,
                        };

                        // Built-in Result<T,E> / Option<T> payload binders.
                        // `Ok`/`Some` → args[0] (success/inner element); `Err` →
                        // args[1] (error element). Result/Option are NOT user
                        // enums (get_enum → None, so enum_kind is None and
                        // payload_tys is None), so without this the binder is an
                        // unconstrained fresh var that drifts the match result to
                        // a `Union<unknown, …>` and widens later arithmetic to
                        // `number`. Derive the binder type directly from the
                        // scrutinee's already-resolved generic arg `Type` — no
                        // re-resolution, no fabrication.
                        let builtin_payload: Option<Type> = if payload_tys.is_none() {
                            match scrutinee {
                                Some(Type::Generic { base, args })
                                    if matches!(
                                        base.as_ref(),
                                        Type::Concrete(ann)
                                            if matches!(
                                                ann.as_type_name_str(),
                                                Some("Result") | Some("Option")
                                            )
                                    ) =>
                                {
                                    match variant.as_str() {
                                        "Ok" | "Some" => args.first().cloned(),
                                        "Err" => args.get(1).cloned(),
                                        _ => None,
                                    }
                                }
                                _ => None,
                            }
                        } else {
                            None
                        };

                        for (idx, p) in patterns.iter().enumerate() {
                            let field_ty = payload_tys
                                .as_ref()
                                .and_then(|tys| {
                                    tys.get(idx).map(|ann| self.resolve_type_annotation(ann))
                                })
                                .or_else(|| {
                                    if idx == 0 {
                                        builtin_payload.clone()
                                    } else {
                                        None
                                    }
                                });
                            self.bind_pattern_vars_typed(p, field_ty.as_ref())?;
                            // For a plain identifier binder, override the
                            // fresh-var define with the resolved payload
                            // type — same shape as the Object/Struct arms.
                            if let (Pattern::Identifier(bind_name), Some(ft)) = (p, &field_ty) {
                                self.env.define(bind_name, TypeScheme::mono(ft.clone()));
                            }
                        }
                    }
                    PatternConstructorFields::Struct(field_pats) => {
                        // Two scrutinee shapes can drive this arm:
                        //
                        // (a) A registered struct (`Point { x, y }`): look up
                        //     via `struct_type_defs` (WS-4 4b).
                        // (b) An enum struct-variant (`Shape::Circle { r }`):
                        //     look up via `EnumDef` members and use the
                        //     variant's `EnumMemberKind::Struct(fields)`.
                        let struct_name = scrutinee.and_then(|ty| self.struct_name_of_type(ty));
                        let enum_struct_fields: Option<Vec<shape_ast::ast::ObjectTypeField>> =
                            match &enum_kind {
                                Some(shape_ast::ast::EnumMemberKind::Struct(fields)) => {
                                    Some(fields.clone())
                                }
                                _ => None,
                            };
                        for (key, p) in field_pats {
                            let field_ty = if let Some(name) = struct_name.as_deref() {
                                self.struct_field_annotation(name, key)
                                    .map(|ann| self.resolve_type_annotation(&ann))
                            } else {
                                enum_struct_fields.as_ref().and_then(|fields| {
                                    fields
                                        .iter()
                                        .find(|f| &f.name == key)
                                        .map(|f| self.resolve_type_annotation(&f.type_annotation))
                                })
                            };
                            self.bind_pattern_vars_typed(p, field_ty.as_ref())?;
                            if let (Pattern::Identifier(bind_name), Some(ft)) = (p, &field_ty) {
                                self.env.define(bind_name, TypeScheme::mono(ft.clone()));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Best-effort static unwrapping for `expr?`.
    fn try_unwrap_inner_type(&self, ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { base, args } if !args.is_empty() => match base.as_ref() {
                Type::Concrete(ann)
                    if ann
                        .as_type_name_str()
                        .is_some_and(|n| n == "Result" || n == "Option") =>
                {
                    Some(args[0].clone())
                }
                _ => None,
            },
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if (name == "Result" || name == "Option") && !args.is_empty() =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            _ => None,
        }
    }

    fn validate_fallible_conversion(&self, source: &Type, target: &Type) -> TypeResult<()> {
        if self.type_contains_unresolved_vars(source) || self.type_contains_unresolved_vars(target)
        {
            return Ok(());
        }

        if self.types_equal(source, target) {
            return Ok(());
        }

        let source_name = self.try_into_type_name(source).ok_or_else(|| {
            TypeError::InvalidAssertion(
                self.render_type_for_diag(source),
                format!("{}?", self.render_type_for_diag(target)),
            )
        })?;
        let target_selector = self.try_into_selector(target).ok_or_else(|| {
            TypeError::InvalidAssertion(
                self.render_type_for_diag(source),
                format!("{}?", self.render_type_for_diag(target)),
            )
        })?;

        if self.has_try_into_impl(&source_name, &target_selector) {
            return Ok(());
        }

        // Check Option/Result lifting: Option<T> as M? is valid if T has TryInto<M>
        if source_name == "Option" || source_name == "Result" {
            if let Some(inner_type) = self.try_unwrap_inner_type(source) {
                if let Some(inner_name) = self.try_into_type_name(&inner_type) {
                    if self.has_try_into_impl(&inner_name, &target_selector) {
                        return Ok(());
                    }
                }
            }
        }

        Err(TypeError::InvalidAssertion(
            self.render_type_for_diag(source),
            format!("{}?", self.render_type_for_diag(target)),
        ))
    }

    fn validate_infallible_conversion(&self, source: &Type, target: &Type) -> TypeResult<()> {
        if self.type_contains_unresolved_vars(source) || self.type_contains_unresolved_vars(target)
        {
            return Ok(());
        }

        if self.types_equal(source, target) {
            return Ok(());
        }

        let source_name = self.try_into_type_name(source).ok_or_else(|| {
            TypeError::InvalidAssertion(
                self.render_type_for_diag(source),
                self.render_type_for_diag(target),
            )
        })?;
        let target_selector = self.try_into_selector(target).ok_or_else(|| {
            TypeError::InvalidAssertion(
                self.render_type_for_diag(source),
                self.render_type_for_diag(target),
            )
        })?;

        if self.has_into_impl(&source_name, &target_selector) {
            return Ok(());
        }

        // Check Option/Result lifting: Option<T> as M is valid if T has Into<M>
        if source_name == "Option" || source_name == "Result" {
            if let Some(inner_type) = self.try_unwrap_inner_type(source) {
                if let Some(inner_name) = self.try_into_type_name(&inner_type) {
                    if self.has_into_impl(&inner_name, &target_selector) {
                        return Ok(());
                    }
                }
            }
        }

        Err(TypeError::InvalidAssertion(
            self.render_type_for_diag(source),
            self.render_type_for_diag(target),
        ))
    }

    pub(crate) fn render_type_for_diag(&self, ty: &Type) -> String {
        if matches!(ty, Type::Variable(_) | Type::Constrained { .. }) {
            return "unknown".to_string();
        }
        ty.to_annotation()
            .map(|ann| match &ann {
                _ if ann.as_type_name_str().is_some() => {
                    ann.as_type_name_str().unwrap().to_string()
                }
                other => format!("{other:?}"),
            })
            .unwrap_or_else(|| format!("{ty:?}"))
    }

    fn has_try_into_impl(&self, source_type: &str, target_selector: &str) -> bool {
        self.env
            .lookup_trait_impl_named("TryInto", source_type, target_selector)
            .is_some()
            || self.env.lookup_trait_impl("TryInto", source_type).is_some()
    }

    fn has_into_impl(&self, source_type: &str, target_selector: &str) -> bool {
        self.env
            .lookup_trait_impl_named("Into", source_type, target_selector)
            .is_some()
            || self.env.lookup_trait_impl("Into", source_type).is_some()
    }

    /// D1 (numeric-conversion GREEN Stage 1): whether a cast SOURCE type is a
    /// concrete primitive numeric type (any int width, float width, or
    /// `decimal`), eligible for the built-in primitive-numeric `as` cast gate
    /// in `Expr::TypeAssertion`. Returns `false` for unresolved type vars (so
    /// the gate never fires speculatively — the existing Into-dispatch /
    /// strict-assertion path handles the unresolved case) and for any
    /// non-numeric concrete type (so e.g. `myStruct as int` still falls through
    /// to the normal validation and is rejected).
    fn source_is_numeric_for_cast(&self, source: &Type) -> bool {
        if self.type_contains_unresolved_vars(source) {
            return false;
        }
        self.try_into_type_name(source)
            .map(|name| BuiltinTypes::is_numeric_type_name(&name))
            .unwrap_or(false)
    }

    fn try_into_type_name(&self, ty: &Type) -> Option<String> {
        fn extract_name(ann: &TypeAnnotation) -> Option<&str> {
            match ann {
                TypeAnnotation::Basic(name) => Some(name.as_str()),
                TypeAnnotation::Reference(path) => Some(path.as_str()),
                TypeAnnotation::Generic { name, .. } => Some(name.as_str()),
                _ => None,
            }
        }
        match ty {
            Type::Concrete(ann) => {
                extract_name(ann).map(TypeInferenceEngine::canonical_try_into_name)
            }
            Type::Generic { base, .. } => match base.as_ref() {
                Type::Concrete(ann) => {
                    extract_name(ann).map(TypeInferenceEngine::canonical_try_into_name)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn try_into_selector(&self, ty: &Type) -> Option<String> {
        fn extract_name(ann: &TypeAnnotation) -> Option<&str> {
            match ann {
                TypeAnnotation::Basic(name) => Some(name.as_str()),
                TypeAnnotation::Reference(path) => Some(path.as_str()),
                TypeAnnotation::Generic { name, .. } => Some(name.as_str()),
                _ => None,
            }
        }
        match ty {
            Type::Concrete(ann) => {
                extract_name(ann).map(TypeInferenceEngine::canonical_try_into_name)
            }
            Type::Generic { base, .. } => match base.as_ref() {
                Type::Concrete(ann) => {
                    extract_name(ann).map(TypeInferenceEngine::canonical_try_into_name)
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn canonical_try_into_name(name: &str) -> String {
        match name {
            "boolean" | "Boolean" | "Bool" => "bool".to_string(),
            "String" => "string".to_string(),
            "Number" => "number".to_string(),
            "Int" => "int".to_string(),
            "Decimal" => "decimal".to_string(),
            _ => name.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Span;

    fn test_span() -> Span {
        Span { start: 0, end: 0 }
    }

    #[test]
    fn try_operator_unwraps_result_and_marks_scope_fallible() {
        let mut engine = TypeInferenceEngine::new();
        engine.push_fallible_scope();

        let result_number = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Result".into()))),
            args: vec![BuiltinTypes::number()],
        };
        engine.env.define("value", TypeScheme::mono(result_number));

        let expr = Expr::TryOperator(
            Box::new(Expr::Identifier("value".to_string(), test_span())),
            test_span(),
        );

        let inferred = engine.infer_expr(&expr).expect("result? should infer");
        assert_eq!(inferred, BuiltinTypes::number());
        assert!(engine.pop_fallible_scope());
    }

    #[test]
    fn try_operator_unwraps_optional_type_and_marks_scope_fallible() {
        let mut engine = TypeInferenceEngine::new();
        engine.push_fallible_scope();

        let optional_number = Type::Concrete(TypeAnnotation::Generic {
            name: "Option".into(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        });
        engine
            .env
            .define("value", TypeScheme::mono(optional_number));

        let expr = Expr::TryOperator(
            Box::new(Expr::Identifier("value".to_string(), test_span())),
            test_span(),
        );

        let inferred = engine.infer_expr(&expr).expect("option? should infer");
        assert_eq!(inferred, BuiltinTypes::number());
        assert!(engine.pop_fallible_scope());
    }

    #[test]
    fn try_operator_unwraps_ok_constructor_call() {
        let mut engine = TypeInferenceEngine::new();
        engine.push_fallible_scope();

        let expr =
            shape_ast::parser::parse_expression_str("Ok(1)?").expect("expression should parse");
        let inferred = engine.infer_expr(&expr).expect("Ok(1)? should infer");
        assert_eq!(inferred, BuiltinTypes::integer());
        assert!(engine.pop_fallible_scope());
    }

    #[test]
    fn try_operator_rejects_non_fallible_operand() {
        let mut engine = TypeInferenceEngine::new();
        engine.push_fallible_scope();
        let expr = shape_ast::parser::parse_expression_str("42?").expect("expression should parse");
        let err = engine
            .infer_expr(&expr)
            .expect_err("plain value ? should be rejected");
        assert!(
            matches!(err, TypeError::ConstraintViolation(_)),
            "expected ConstraintViolation, got {:?}",
            err
        );
    }

    #[test]
    fn fallible_type_assertion_as_optional_returns_typed_result() {
        let mut engine = TypeInferenceEngine::new();
        let _ = engine.env.register_trait_impl_named(
            "TryInto",
            "string",
            "int",
            vec!["tryInto".to_string()],
        );
        let expr = shape_ast::parser::parse_expression_str("\"42\" as int?")
            .expect("fallible cast expression should parse");

        let inferred = engine
            .infer_expr(&expr)
            .expect("fallible cast should infer");

        match inferred {
            Type::Generic { base, args } => {
                assert!(matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                ));
                assert_eq!(args.len(), 2);
                assert_eq!(args[0], BuiltinTypes::integer());
                assert_eq!(
                    args[1],
                    Type::Concrete(TypeAnnotation::Reference("AnyError".into()))
                );
            }
            other => panic!("expected Result<int, AnyError>, got {:?}", other),
        }
    }

    #[test]
    fn infallible_type_assertion_as_uses_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let _ =
            engine
                .env
                .register_trait_impl_named("Into", "string", "int", vec!["into".to_string()]);
        let expr = shape_ast::parser::parse_expression_str("\"42\" as int")
            .expect("cast expression should parse");

        let inferred = engine
            .infer_expr(&expr)
            .expect("into-backed cast should infer");
        assert_eq!(inferred, BuiltinTypes::integer());
    }

    #[test]
    fn infallible_type_assertion_rejects_unsupported_static_conversion() {
        let mut engine = TypeInferenceEngine::new();
        let expr =
            shape_ast::parser::parse_expression_str("{ x: 1 } as int").expect("expression parse");
        let err = engine
            .infer_expr(&expr)
            .expect_err("object -> int cast should fail without Into impl");
        assert!(
            matches!(err, TypeError::InvalidAssertion(_, _)),
            "expected InvalidAssertion, got {:?}",
            err
        );
    }

    #[test]
    fn fallible_type_assertion_accepts_named_try_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let _ = engine.env.register_trait_impl_named(
            "TryInto",
            "Price",
            "int",
            vec!["tryInto".to_string()],
        );
        engine.env.define(
            "value",
            TypeScheme::mono(Type::Concrete(TypeAnnotation::Reference("Price".into()))),
        );

        let expr =
            shape_ast::parser::parse_expression_str("value as int?").expect("expression parses");
        let inferred = engine
            .infer_expr(&expr)
            .expect("named TryInto impl should satisfy static validation");

        match inferred {
            Type::Generic { base, args } => {
                assert!(matches!(
                    base.as_ref(),
                    Type::Concrete(TypeAnnotation::Reference(name)) if name == "Result"
                ));
                assert_eq!(args.len(), 2);
                assert_eq!(args[0], BuiltinTypes::integer());
            }
            other => panic!("expected Result<int, AnyError>, got {:?}", other),
        }
    }

    #[test]
    fn fallible_type_assertion_rejects_unsupported_static_conversion() {
        let mut engine = TypeInferenceEngine::new();
        let expr = shape_ast::parser::parse_expression_str("{ x: 1 } as int?")
            .expect("expression should parse");
        let err = engine
            .infer_expr(&expr)
            .expect_err("object -> int fallible cast should fail statically");
        assert!(
            matches!(err, TypeError::InvalidAssertion(_, _)),
            "expected InvalidAssertion, got {:?}",
            err
        );
    }

    #[test]
    fn fallible_type_assertion_in_program_uses_preceding_try_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let program = shape_ast::parser::parse_program(
            r#"
impl TryInto<int> for string as int {
  method tryInto() {
    self as int?
  }
}

fn parse(raw: string) -> Result<int> {
  let n = (raw as int?)?
  Ok(n)
}
"#,
        )
        .expect("program should parse");

        let types = engine
            .infer_program(&program)
            .expect("program-level inference should see prior TryInto impl");

        assert!(
            types.contains_key("parse"),
            "expected inferred function type"
        );
    }

    #[test]
    fn fallible_type_assertion_in_program_with_callsite_uses_preceding_try_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let program = shape_ast::parser::parse_program(
            r#"
impl TryInto<int> for string as int {
  method tryInto() {
    self as int?
  }
}

fn parse(raw: string) -> Result<int> {
  let n = (raw as int?)?
  Ok(n)
}

match parse("not-int") {
  Ok(v) => v
  Err(_) => -1
}
"#,
        )
        .expect("program should parse");

        let types = engine
            .infer_program(&program)
            .expect("program-level inference should keep TryInto impl with callsite");

        assert!(
            types.contains_key("parse"),
            "expected inferred function type"
        );
    }

    #[test]
    fn infallible_type_assertion_in_program_uses_preceding_into_impl() {
        let mut engine = TypeInferenceEngine::new();
        let program = shape_ast::parser::parse_program(
            r#"
impl Into<int> for string as int {
  method into() {
    self as int
  }
}

fn parse(raw: string) -> int {
  raw as int
}
"#,
        )
        .expect("program should parse");

        let types = engine
            .infer_program(&program)
            .expect("program-level inference should see prior Into impl");

        assert!(
            types.contains_key("parse"),
            "expected inferred function type"
        );
    }
}
