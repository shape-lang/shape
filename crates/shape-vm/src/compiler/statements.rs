//! Statement and item compilation

use crate::bytecode::{Function, Instruction, OpCode, Operand};
use shape_ast::ast::{
    AnnotationTargetKind, DestructurePattern, EnumDef, EnumMemberKind, ExportItem, Expr,
    FunctionDef, FunctionParameter, Item, Literal, ModuleDecl, ObjectEntry, Query, Span, Spanned,
    Statement, TypeAnnotation, VarKind,
};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::{EnumVariantInfo, FieldType};

use super::{
    BytecodeCompiler, DropKind, ImportedAnnotationSymbol, ImportedSymbol, ModuleBuiltinFunction,
    ParamPassMode, StructGenericInfo,
};

#[derive(Debug, Clone)]
struct NativeFieldLayoutSpec {
    c_type: String,
    size: u64,
    align: u64,
}

impl BytecodeCompiler {
    fn comptime_field_slot_from_literal(
        struct_name: &str,
        field_name: &str,
        field_type: &FieldType,
        literal: &Literal,
    ) -> Result<shape_value::KindedSlot> {
        let slot = match (field_type, literal) {
            (FieldType::I64, Literal::Int(value)) => shape_value::KindedSlot::from_int(*value),
            (FieldType::I64, Literal::UInt(value)) if *value <= i64::MAX as u64 => {
                shape_value::KindedSlot::from_int(*value as i64)
            }
            (FieldType::F64, Literal::Number(value)) => {
                shape_value::KindedSlot::from_number(*value)
            }
            (FieldType::F64, Literal::Int(value))
                if (*value as i128) >= -(1i128 << 53) && (*value as i128) <= (1i128 << 53) =>
            {
                shape_value::KindedSlot::from_number(*value as f64)
            }
            (FieldType::F64, Literal::UInt(value)) if *value <= (1u64 << 53) => {
                shape_value::KindedSlot::from_number(*value as f64)
            }
            (FieldType::String, Literal::String(value)) => {
                shape_value::KindedSlot::from_string(value)
            }
            (FieldType::Bool, Literal::Bool(value)) => shape_value::KindedSlot::from_bool(*value),
            (FieldType::Any | FieldType::Option(_), Literal::None) => {
                shape_value::KindedSlot::none()
            }
            _ => {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Comptime field '{}' on type '{}' has default value incompatible with declared type '{}'",
                        field_name, struct_name, field_type
                    ),
                    location: None,
                });
            }
        };

        Ok(slot)
    }

    fn register_builtin_function_decl(
        &mut self,
        def: &shape_ast::ast::BuiltinFunctionDecl,
    ) -> Result<()> {
        let export_name = def
            .name
            .rsplit("::")
            .next()
            .unwrap_or(def.name.as_str())
            .to_string();
        let source_module_path = if let Some((owner_module, _)) = def.name.rsplit_once("::") {
            self.resolve_canonical_module_path(owner_module)
                .unwrap_or_else(|| owner_module.to_string())
        } else {
            return Ok(());
        };

        self.module_builtin_functions.insert(
            def.name.clone(),
            ModuleBuiltinFunction {
                export_name,
                source_module_path,
            },
        );
        Ok(())
    }

    fn emit_comptime_internal_call(
        &mut self,
        method: &str,
        args: Vec<Expr>,
        span: Span,
    ) -> Result<()> {
        let call = Expr::QualifiedFunctionCall {
            namespace: "__comptime__".to_string(),
            function: method.to_string(),
            args,
            named_args: Vec::new(),
            span,
        };
        let prev = self.allow_internal_comptime_namespace;
        self.allow_internal_comptime_namespace = true;
        let compile_result = self.compile_expr(&call);
        self.allow_internal_comptime_namespace = prev;
        compile_result?;
        self.emit(Instruction::simple(OpCode::Pop));
        Ok(())
    }

    /// Serialize a value to JSON for comptime directive payloads.
    ///
    /// Wraps serde_json serialization errors into ShapeError with the given
    /// directive label for diagnostics.
    fn serialize_directive_payload(
        &self,
        value: &(impl serde::Serialize + ?Sized),
        directive_label: &str,
        span: Span,
    ) -> Result<String> {
        serde_json::to_string(value).map_err(|e| ShapeError::RuntimeError {
            message: format!(
                "Failed to serialize comptime {} directive: {}",
                directive_label, e
            ),
            location: Some(self.span_to_source_location(span)),
        })
    }

    /// Check that the compiler is in comptime mode, returning an error otherwise.
    fn require_comptime_mode(&self, directive_name: &str, span: Span) -> Result<()> {
        if !self.comptime_mode {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "`{}` is only valid inside `comptime {{}}` context",
                    directive_name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }
        Ok(())
    }

    /// strict-flip (map/collect OUTPUT element-type stamp): reject a `let`
    /// binding whose explicit `Array<T_decl>` annotation does NOT match the
    /// PROVEN result element type of a computed initializer.
    ///
    /// The hole this closes: `let r: Array<number> = [1,2,3].map(|x| x*2)`
    /// (and the `.iter().map(...).collect()` form). The map's OUTPUT element
    /// type is the closure RETURN type — `int` here — so the result is
    /// `Array<int>`. `int` and `number` do NOT unify (CLAUDE.md §Type-System
    /// Rules): an `Array<int>` result must NOT coerce to `Array<number>`.
    /// Without this check the binding's slot was stamped `Float64` from the
    /// annotation while the runtime array carries `Int64` bits — reading them
    /// as `number` is a bit-reinterpret (the overflow/garbage surface).
    ///
    /// Per ADR-006 §2.7.5 stamp-at-compile-time, the proof is the closure's
    /// RETURN type carried through `specialized_call_return_concrete_type`
    /// (the substituted callee return annotation). We compare the declared and
    /// proven `ConcreteType`s structurally; a mismatch is a hard compile error.
    ///
    /// Scope is deliberately narrow to avoid regressing lossless literal
    /// adoption:
    ///   - An `Expr::Array` literal initializer is SKIPPED — per-element
    ///     literal adoption (`let a: Array<number> = [1,2,3]`) is handled by
    ///     the array-emission element path and is lossless/allowed.
    ///   - We only reject when BOTH the declared annotation AND the
    ///     initializer resolve to a concrete `Array<T>` whose element types
    ///     differ. An un-inferable initializer element type yields `None` and
    ///     the binding falls through to the existing annotation-driven path
    ///     (a numeric annotation on an unknown-element computed result is the
    ///     caller's responsibility — this helper never fabricates a match).
    /// strict-flip S1 (let-annotation Unknown-accept guard, FIX B,
    /// 2026-06-22): reject `let x: <proven-concrete> = <init>` when the
    /// initializer's type is genuinely un-inferable (`unknown` / an unresolved
    /// free type variable). This is the binding-site mirror of
    /// `reject_unknown_arg_into_typed_param` (function_calls.rs): an
    /// `unknown`-typed value must NOT launder through a typed binding into a
    /// concrete slot, where its raw bits would be reinterpreted as that slot's
    /// `NativeKind` (the catastrophic cross-type reinterpret —
    /// `let bad: int = apply(ret_num, 3.0)` ⇒ `6.0`'s f64 bits read as an i64
    /// for `bad % 4`).
    ///
    /// NO FALSE POSITIVES after the T1 keystone + the call-site HOF return
    /// propagation (FIX A): a legitimate dispatch result
    /// (`let n: int = arr.map(..)[0]`, `let x: int = someTypedCall()`,
    /// `let r: number = apply(ret_num, 3.0)`) resolves to a CONCRETE type via
    /// `concrete_type_for_expr` / the post-solve expr-type table, so it never
    /// reaches the `unknown` reject here. Only a genuinely-unknown result — an
    /// un-annotated HOF whose param return type cannot be resolved — rejects,
    /// which is correct: the user must annotate the HOF or type its parameter.
    ///
    /// Scope is deliberately narrow: the annotation must be a PROVEN concrete
    /// PRIMITIVE scalar (`int`/`number`/`bool`/`string`/…). Generic, structural,
    /// trait-object, and nominal annotations fall through (the existing
    /// annotation-driven path owns them) so this guard never regresses a
    /// program whose binding type the tracker legitimately could not prove
    /// concretely.
    fn check_let_annotation_scalar_unknown_strict(
        &mut self,
        type_ann: &TypeAnnotation,
        init_expr: &Expr,
    ) -> Result<()> {
        // Only a bare proven-concrete primitive scalar annotation triggers the
        // guard. (`Array<T>` element mismatch is handled by the sibling
        // element-type check; generic/structural/nominal annotations are not
        // "proven concrete primitive" and fall through.)
        let prim_name = match type_ann {
            TypeAnnotation::Basic(n) => n.as_str(),
            TypeAnnotation::Reference(p) => p.as_str(),
            _ => return Ok(()),
        };
        if !Self::is_known_concrete_primitive_name(prim_name) {
            return Ok(());
        }

        // If the initializer resolves to a CONCRETE type by FIX-A propagation
        // (the call-site HOF return resolver) or any other proof path, compare
        // it to the declared annotation directly. An un-annotated HOF whose
        // return resolves to a concrete primitive (`apply(ret_num, 3.0)` ⇒
        // `number`) is NOT seen by the type-inference constraint solver (the
        // callee has no return annotation, so the engine left it `unknown`),
        // so the solver never raises the mismatch — we must catch it HERE.
        //   - resolved == declared  → well-typed; never reject.
        //   - resolved != declared, both proven primitives → hard mismatch
        //     (`number` resolved into an `int` binding): `int`/`number` do not
        //     unify (CLAUDE.md §Type-System-Rules). Reject — NO silent widen.
        //   - resolved is a non-primitive concrete type → fall through (the
        //     element/nominal paths + solver own it).
        if let Some(resolved) =
            crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                self, init_expr,
            )
        {
            use shape_value::v2::ConcreteType;
            let resolved_prim = match &resolved {
                ConcreteType::I64 => Some("int"),
                ConcreteType::F64 => Some("number"),
                ConcreteType::Bool => Some("bool"),
                ConcreteType::String => Some("string"),
                _ => None,
            };
            let Some(resolved_prim) = resolved_prim else {
                return Ok(());
            };
            // `int`/`uint`-family aliasing: treat the declared name's canonical
            // primitive against the resolved one. A direct string match covers
            // the load-bearing `int`/`number`/`bool`/`string` cases.
            if resolved_prim == prim_name {
                return Ok(());
            }
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type mismatch: binding declares '{}' but the initializer \
                     produces '{}' — `int` and `number` do not unify, so the \
                     type must match exactly (cast explicitly with `as` if a \
                     conversion is intended)",
                    prim_name, resolved_prim
                ),
                location: Some(self.span_to_source_location(init_expr.span())),
            });
        }

        // strict-flip S1 REVERT (angle-A, 2026-06-22): the prior
        // `init_rests_on_unprovable_unannotated_fn` guard rejected ANY
        // `let x: <prim> = f(<concrete args>)` where `f` is an un-return-typed
        // user fn — over-rejecting a large idiomatic class (`fn f(x){x+1};
        // let r: int = f(5)` genuinely returns int from its body). A correct
        // matching annotation must NEVER turn a working program into a compile
        // error. Removed. The genuinely-`unknown` cases the structural detector
        // was meant to cover are still caught by the post-solve
        // `Type::Variable`/`"unknown"` fallback below (which does NOT fire when
        // inference proves a usable concrete primitive for such a call).
        //
        // HOLE-2: a mixed-type arithmetic binary op (`apply(ret_num, 3.0) % 4`
        // = `number % int`). `concrete_type_for_expr` correctly yields `None`
        // for the disagreeing operands (no fabrication), but the engine's
        // `infer_expr_type` reads back the WRONG `int` from the integer literal
        // operand, masking the `number`-typed result. When an operand of an
        // arithmetic binop resolves (structurally, annotation-independently) to
        // a concrete primitive that disagrees with the declared annotation, the
        // result cannot be the declared type — reject. `int`/`number` do not
        // unify; no silent widen.
        if let Some(operand_prim) = self.binop_operand_disagreeing_primitive(init_expr, prim_name) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type mismatch: binding declares '{}' but an operand of the \
                     initializer's arithmetic expression produces '{}' — `int` and \
                     `number` do not unify, so the type must match exactly (cast \
                     explicitly with `as` if a conversion is intended)",
                    prim_name, operand_prim
                ),
                location: Some(self.span_to_source_location(init_expr.span())),
            });
        }

        // Fall back to the post-solve inferred type for the genuinely-unknown
        // (`Type::Variable` / `"unknown"`) case the structural detector does not
        // cover (e.g. a bare `let x: int = some_unknown_local`). A concrete
        // inferred type falls through to the constraint solver.
        let Ok(init_ty) = self.infer_expr_type(init_expr) else {
            return Ok(());
        };
        if !Self::type_is_unknown(&init_ty) {
            return Ok(());
        }

        Err(ShapeError::SemanticError {
            message: format!(
                "the initializer has an un-inferable type (`unknown`), but the \
                 binding declares the proven concrete type '{}' — an \
                 `unknown`-typed value cannot be accepted into a typed binding \
                 (this would reinterpret its raw bits as '{}'). Annotate the \
                 source (e.g. give the higher-order function a return type or \
                 type its callable parameter) so the type is proven.",
                prim_name, prim_name
            ),
            location: Some(self.span_to_source_location(init_expr.span())),
        })
    }

    /// strict-flip S1 (HOLE-2, 2026-06-22): for an arithmetic binary-op init,
    /// return the proven primitive type NAME of an operand that DISAGREES with
    /// the declared annotation `decl_prim`. The arithmetic result must be the
    /// operands' common type; an operand proven to a different primitive means
    /// the binding can NEVER hold the declared type (`number % int` into an
    /// `int` binding). Resolves each operand structurally via
    /// `concrete_type_for_expr` (annotation-independent — the engine's
    /// `infer_expr_type` is unreliable here, it echoes the integer literal).
    /// Returns `None` for non-arithmetic ops, fully-agreeing operands, or
    /// operands the resolver cannot prove (those route through the other
    /// guards). NO fabrication.
    fn binop_operand_disagreeing_primitive(&self, expr: &Expr, decl_prim: &str) -> Option<String> {
        use shape_ast::ast::BinaryOp;
        use shape_value::v2::ConcreteType;
        let Expr::BinaryOp {
            left, op, right, ..
        } = expr
        else {
            return None;
        };
        if !matches!(
            op,
            BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::Pow
        ) {
            return None;
        }
        let prim_name_of = |ct: &ConcreteType| -> Option<&'static str> {
            match ct {
                ConcreteType::I64 => Some("int"),
                ConcreteType::F64 => Some("number"),
                ConcreteType::Bool => Some("bool"),
                ConcreteType::String => Some("string"),
                _ => None,
            }
        };
        for operand in [left.as_ref(), right.as_ref()] {
            // Numeric literals adopt the surrounding context losslessly (an
            // `int` literal in a `number` arithmetic is fine) — never the
            // source of a genuine mismatch here. Skip them so `let r: number =
            // x % 4` is not falsely flagged by the integer literal `4`.
            if matches!(operand, Expr::Literal(..)) {
                continue;
            }
            if let Some(ct) =
                crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                    self, operand,
                )
            {
                if let Some(op_prim) = prim_name_of(&ct) {
                    if op_prim != decl_prim {
                        return Some(op_prim.to_string());
                    }
                }
            }
        }
        None
    }

    fn check_let_annotation_element_type_strict(
        &mut self,
        type_ann: &TypeAnnotation,
        init_expr: &Expr,
    ) -> Result<()> {
        use shape_value::v2::ConcreteType;

        // Flat array-literal initializers adopt element types per-element
        // (lossless literal adoption); never reject them here. Nested array
        // literals still have structural element shape, so `Array<number> =
        // [[...]]` must reject instead of bypassing this check.
        if matches!(
            init_expr,
            Expr::Array(elements, _)
                if !elements.iter().any(|e| matches!(e, Expr::Array(..)))
        ) {
            return Ok(());
        }

        // Declared annotation must resolve to a concrete `Array<T_decl>`.
        let Some(ConcreteType::Array(decl_elem)) =
            crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                self, type_ann,
            )
        else {
            return Ok(());
        };

        // Proven result type of the computed initializer must be a concrete
        // `Array<T_init>`. Two sources of proof:
        //   1. The eager `recv.map(|x| ...)` form + receiver-derived builtin
        //      array methods, resolved by `concrete_type_for_expr` (which
        //      threads the closure RETURN type through
        //      `specialized_call_return_concrete_type`).
        //   2. The lazy `recv.iter().map(|x| ...).collect()` form, whose
        //      iterator-receiver `.map()` / `.collect()` are NOT monomorphized
        //      stdlib functions, so source (1) finds nothing. The dedicated
        //      iterator-chain resolver below stamps the result element = the
        //      iterator's current element type = the LAST map closure's RETURN
        //      type (or the source array element when no map adapter is
        //      present).
        // Unknown → fall through (the annotation-driven path owns it; this
        // helper never fabricates a match).
        let init_elem = if let Some(ConcreteType::Array(e)) =
            crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                self, init_expr,
            ) {
            e
        } else if let Some(e) = self.iterator_collect_result_element_concrete_type(init_expr) {
            Box::new(e)
        } else {
            return Ok(());
        };

        if *decl_elem != *init_elem {
            let reason = match (decl_elem.as_ref(), init_elem.as_ref()) {
                (ConcreteType::I64, ConcreteType::F64) | (ConcreteType::F64, ConcreteType::I64) => {
                    "`int` and `number` do not unify, so the element type must match exactly \
                     (cast explicitly with `as` if a conversion is intended)"
                }
                _ => {
                    "the initializer's element structure must match the annotation exactly \
                     (nested arrays require an array-of-array or matrix-shaped annotation)"
                }
            };
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type mismatch: binding annotated `Array<{}>` but the \
                     initializer produces `Array<{}>` — {reason}",
                    decl_elem.mono_key(),
                    init_elem.mono_key(),
                ),
                location: Some(self.span_to_source_location(init_expr.span())),
            });
        }

        Ok(())
    }

    /// strict-flip (map/collect OUTPUT element-type stamp): resolve the PROVEN
    /// result element `ConcreteType` of a lazy iterator-collect chain
    /// (`recv.iter().map(f1)…collect()` / `.toArray()`).
    ///
    /// The iterator-receiver `.map()` / `.collect()` are NOT monomorphized
    /// stdlib functions, so the normal
    /// `specialized_call_return_concrete_type` chain finds no call-site record
    /// and `concrete_type_for_expr` returns `None`. This walks the chain
    /// structurally and stamps the result element = the iterator's CURRENT
    /// element type, per the registered iterator signatures
    /// (`method_table.rs` §iterator_methods):
    ///   - `collect` / `toArray` → `Vec<T>` where `T` is the receiver
    ///     iterator's element.
    ///   - `iter()` over `Array<T>` → element `T`.
    ///   - `map(closure)` → element = the closure RETURN type (the OUTPUT
    ///     element-type stamp — `int` stays `int`, `number` stays `number`).
    ///   - `filter` / `take` / `skip` → element unchanged.
    ///
    /// Per ADR-006 §2.7.5 stamp-at-compile-time, the map output element is the
    /// closure's RETURN type inferred from its body against the proven input
    /// element type — no runtime probe, no coercion. Any un-inferable link
    /// yields `None` (the caller then falls through; the strict check never
    /// fabricates a match).
    fn iterator_collect_result_element_concrete_type(
        &mut self,
        expr: &Expr,
    ) -> Option<shape_value::v2::ConcreteType> {
        let Expr::MethodCall {
            receiver, method, ..
        } = expr
        else {
            return None;
        };
        match method.as_str() {
            // Eager terminal: the result element = the receiver iterator's
            // current element type.
            "collect" | "toArray" => self.iterator_element_concrete_type(receiver),
            _ => None,
        }
    }

    /// Resolve the element `ConcreteType` of an iterator-producing expression
    /// (the lazy-adapter chain rooted at `.iter()`). See
    /// [`Self::iterator_collect_result_element_concrete_type`] for the stamping
    /// rules and the ADR-006 §2.7.5 proof discipline.
    fn iterator_element_concrete_type(
        &mut self,
        expr: &Expr,
    ) -> Option<shape_value::v2::ConcreteType> {
        use shape_value::v2::ConcreteType;

        let Expr::MethodCall {
            receiver,
            method,
            args,
            ..
        } = expr
        else {
            return None;
        };

        match method.as_str() {
            // `iter()` over an `Array<T>` receiver → element `T`. The receiver
            // is a concrete array expr (literal, identifier, or an eager
            // array-returning chain), resolved by the immutable
            // `concrete_type_for_expr`.
            "iter" => {
                match crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                    self, receiver,
                ) {
                    Some(ConcreteType::Array(elem)) => Some(*elem),
                    _ => None,
                }
            }
            // `map(closure)` → element = the closure RETURN type, inferred from
            // its body against the receiver iterator's CURRENT element type.
            "map" => {
                let recv_elem = self.iterator_element_concrete_type(receiver)?;
                let Some(Expr::FunctionExpr {
                    params: cparams,
                    body: cbody,
                    return_type,
                    ..
                }) = args.first()
                else {
                    return None;
                };
                // Seed the closure param's type with the proven input element
                // type (the §2.7.5 proof of the closure param at this site).
                let elem_ann =
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(
                        &recv_elem,
                    );
                let caller_arg_type_names: Vec<Option<String>> = vec![
                    elem_ann
                        .as_ref()
                        .and_then(Self::tracked_type_name_from_annotation),
                ];
                let return_type_name =
                    crate::compiler::expressions::closures::infer_closure_body_return_type_name_with_caller_context(
                        self,
                        cparams,
                        cbody,
                        return_type.as_ref(),
                        &[],
                        &caller_arg_type_names,
                    )?;
                let vec_emission = crate::compiler::v2_map_emission::concrete_type_from_annotation;
                vec_emission(&shape_ast::ast::TypeAnnotation::Basic(return_type_name))
            }
            // Element-preserving lazy adapters.
            "filter" | "take" | "skip" => self.iterator_element_concrete_type(receiver),
            _ => None,
        }
    }

    fn emit_comptime_extend_directive(
        &mut self,
        extend: &shape_ast::ast::ExtendStatement,
        span: Span,
    ) -> Result<()> {
        let payload = self.serialize_directive_payload(extend, "extend", span)?;
        self.emit_comptime_internal_call(
            "__emit_extend",
            vec![Expr::Literal(Literal::String(payload), span)],
            span,
        )
    }

    fn emit_comptime_remove_directive(&mut self, span: Span) -> Result<()> {
        self.emit_comptime_internal_call("__emit_remove", Vec::new(), span)
    }

    fn emit_comptime_set_param_value_directive(
        &mut self,
        param_name: &str,
        expression: &Expr,
        span: Span,
    ) -> Result<()> {
        self.emit_comptime_internal_call(
            "__emit_set_param_value",
            vec![
                Expr::Literal(Literal::String(param_name.to_string()), span),
                expression.clone(),
            ],
            span,
        )
    }

    fn emit_comptime_set_param_type_directive(
        &mut self,
        param_name: &str,
        type_annotation: &TypeAnnotation,
        span: Span,
    ) -> Result<()> {
        let payload = self.serialize_directive_payload(type_annotation, "param type", span)?;
        self.emit_comptime_internal_call(
            "__emit_set_param_type",
            vec![
                Expr::Literal(Literal::String(param_name.to_string()), span),
                Expr::Literal(Literal::String(payload), span),
            ],
            span,
        )
    }

    fn emit_comptime_set_return_type_directive(
        &mut self,
        type_annotation: &TypeAnnotation,
        span: Span,
    ) -> Result<()> {
        let payload = self.serialize_directive_payload(type_annotation, "return type", span)?;
        self.emit_comptime_internal_call(
            "__emit_set_return_type",
            vec![Expr::Literal(Literal::String(payload), span)],
            span,
        )
    }

    fn emit_comptime_set_return_expr_directive(
        &mut self,
        expression: &Expr,
        span: Span,
    ) -> Result<()> {
        self.emit_comptime_internal_call("__emit_set_return_type", vec![expression.clone()], span)
    }

    fn emit_comptime_replace_body_directive(
        &mut self,
        body: &[Statement],
        span: Span,
    ) -> Result<()> {
        let payload = self.serialize_directive_payload(body, "replace-body", span)?;
        self.emit_comptime_internal_call(
            "__emit_replace_body",
            vec![Expr::Literal(Literal::String(payload), span)],
            span,
        )
    }

    fn emit_comptime_replace_body_expr_directive(
        &mut self,
        expression: &Expr,
        span: Span,
    ) -> Result<()> {
        self.emit_comptime_internal_call("__emit_replace_body", vec![expression.clone()], span)
    }

    fn emit_comptime_replace_module_expr_directive(
        &mut self,
        expression: &Expr,
        span: Span,
    ) -> Result<()> {
        self.emit_comptime_internal_call("__emit_replace_module", vec![expression.clone()], span)
    }

    pub(super) fn register_item_functions(&mut self, item: &Item) -> Result<()> {
        match item {
            Item::Function(func_def, _) => self.register_function(func_def),
            Item::BuiltinFunctionDecl(def, _) => self.register_builtin_function_decl(def),
            Item::Module(module_def, _) => {
                let module_path = self.current_module_path_for(module_def.name.as_str());
                self.module_scope_stack.push(module_path.clone());
                let register_result = (|| -> Result<()> {
                    for inner in &module_def.items {
                        let qualified = self.qualify_module_item(inner, &module_path)?;
                        self.register_item_functions(&qualified)?;
                    }
                    Ok(())
                })();
                self.module_scope_stack.pop();
                register_result
            }
            Item::Trait(trait_def, _) => {
                self.known_traits.insert(trait_def.name.clone());
                self.trait_defs
                    .insert(trait_def.name.clone(), trait_def.clone());
                // Register in type inference environment so supertrait checking works
                self.type_inference.env.define_trait(trait_def);
                Ok(())
            }
            Item::ForeignFunction(def, _) => {
                // Register as a normal function so call sites resolve the name.
                // Caller-visible arity excludes `out` params.
                let caller_visible = def.params.iter().filter(|p| !p.is_out).count();
                self.function_arity_bounds
                    .insert(def.name.clone(), (caller_visible, caller_visible));
                self.function_const_params
                    .insert(def.name.clone(), Vec::new());
                let (ref_params, ref_mutates) = Self::native_param_reference_contract(def);
                let (vis_ref_params, vis_ref_mutates) = if def.params.iter().any(|p| p.is_out) {
                    let mut vrp = Vec::new();
                    let mut vrm = Vec::new();
                    for (i, p) in def.params.iter().enumerate() {
                        if !p.is_out {
                            vrp.push(ref_params.get(i).copied().unwrap_or(false));
                            vrm.push(ref_mutates.get(i).copied().unwrap_or(false));
                        }
                    }
                    (vrp, vrm)
                } else {
                    (ref_params, ref_mutates)
                };

                let func = crate::bytecode::Function {
                    name: def.name.clone(),
                    arity: caller_visible as u16,
                    param_names: def
                        .params
                        .iter()
                        .filter(|p| !p.is_out)
                        .flat_map(|p| p.get_identifiers())
                        .collect(),
                    locals_count: 0,
                    entry_point: 0,
                    body_length: 0,
                    is_closure: false,
                    captures_count: 0,
                    is_async: def.is_async,
                    ref_params: vis_ref_params,
                    ref_mutates: vis_ref_mutates,
                    mutable_captures: Vec::new(),
                    frame_descriptor: None,
                    osr_entry_points: Vec::new(),
                    mir_data: None,
                };
                self.program.functions.push(func);

                // Store the foreign function def so call sites can resolve
                // the declared return type (must be Result<T> for dynamic languages).
                self.foreign_function_defs
                    .insert(def.name.clone(), def.clone());

                Ok(())
            }
            Item::Export(export, _) => match &export.item {
                ExportItem::Function(func_def) => self.register_function(func_def),
                ExportItem::BuiltinFunction(def) => self.register_builtin_function_decl(def),
                ExportItem::Trait(trait_def) => {
                    self.known_traits.insert(trait_def.name.clone());
                    self.trait_defs
                        .insert(trait_def.name.clone(), trait_def.clone());
                    // Register in type inference environment so supertrait checking works
                    self.type_inference.env.define_trait(trait_def);
                    Ok(())
                }
                ExportItem::Annotation(annotation_def) => {
                    self.compile_annotation_def(annotation_def)
                }
                ExportItem::ForeignFunction(def) => {
                    // Same registration as Item::ForeignFunction
                    let caller_visible = def.params.iter().filter(|p| !p.is_out).count();
                    self.function_arity_bounds
                        .insert(def.name.clone(), (caller_visible, caller_visible));
                    self.function_const_params
                        .insert(def.name.clone(), Vec::new());
                    let (ref_params, ref_mutates) = Self::native_param_reference_contract(def);
                    let (vis_ref_params, vis_ref_mutates) = if def.params.iter().any(|p| p.is_out) {
                        let mut vrp = Vec::new();
                        let mut vrm = Vec::new();
                        for (i, p) in def.params.iter().enumerate() {
                            if !p.is_out {
                                vrp.push(ref_params.get(i).copied().unwrap_or(false));
                                vrm.push(ref_mutates.get(i).copied().unwrap_or(false));
                            }
                        }
                        (vrp, vrm)
                    } else {
                        (ref_params, ref_mutates)
                    };

                    let func = crate::bytecode::Function {
                        name: def.name.clone(),
                        arity: caller_visible as u16,
                        param_names: def
                            .params
                            .iter()
                            .filter(|p| !p.is_out)
                            .flat_map(|p| p.get_identifiers())
                            .collect(),
                        locals_count: 0,
                        entry_point: 0,
                        body_length: 0,
                        is_closure: false,
                        captures_count: 0,
                        is_async: def.is_async,
                        ref_params: vis_ref_params,
                        ref_mutates: vis_ref_mutates,
                        mutable_captures: Vec::new(),
                        frame_descriptor: None,
                        osr_entry_points: Vec::new(),
                        mir_data: None,
                    };
                    self.program.functions.push(func);

                    self.foreign_function_defs
                        .insert(def.name.clone(), def.clone());

                    Ok(())
                }
                _ => Ok(()),
            },
            Item::Extend(extend, _) => {
                // Desugar extend methods to functions with implicit `self` receiver param.
                for method in &extend.methods {
                    let func_def = self.desugar_extend_method(method, &extend.type_name)?;
                    self.register_function(&func_def)?;
                }
                Ok(())
            }
            Item::Impl(impl_block, _) => {
                // J-CT.2 (2026-05-23) — comptime impl blocks are deferred
                // for in-mini-VM registration. The outer compiler does not
                // desugar/register/compile their methods into the runtime
                // program. They are stored on `comptime_impl_blocks` so the
                // comptime evaluator (`compiler/comptime.rs::execute_comptime`)
                // can prepend them as `Item::Impl` items into the mini-VM
                // program, where the in-comptime-mode compiler then
                // processes them through this same arm normally. Audit
                // §2.D carve-out: no new dispatch shape — comptime-trait
                // methods reuse the standard UFCS / `Type::method`
                // resolution path; the difference is *when* they're
                // available (only inside `comptime { }`), not *how*.
                if impl_block.is_comptime {
                    self.comptime_impl_blocks.push(impl_block.clone());
                    return Ok(());
                }
                // Impl blocks use scoped UFCS names.
                // - default impl: "Type::method" (legacy compatibility)
                // - named impl: "Trait::Type::ImplName::method"
                // This prevents conflicts when multiple named impls exist.
                let raw_trait_name = match &impl_block.trait_name {
                    shape_ast::ast::types::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::types::TypeName::Generic { name, .. } => name.as_str(),
                };
                let type_name = match &impl_block.target_type {
                    shape_ast::ast::types::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::types::TypeName::Generic { name, .. } => name.as_str(),
                };
                let impl_name = impl_block.impl_name.as_deref();

                // Resolve trait name: canonical for def lookup, basename for dispatch
                let (canonical_trait, trait_basename) = self.resolve_trait_name(raw_trait_name);

                // From/TryFrom impls use reverse-conversion desugaring:
                // the method takes an explicit `value` param (no implicit self),
                // and we auto-derive Into/TryInto trait symbols on the source type.
                if trait_basename == "From" || trait_basename == "TryFrom" {
                    return self.compile_from_impl(impl_block, &trait_basename, type_name);
                }

                // Collect names of methods explicitly provided in the impl block
                let overridden: std::collections::HashSet<&str> =
                    impl_block.methods.iter().map(|m| m.name.as_str()).collect();

                for method in &impl_block.methods {
                    let func_def = self.desugar_impl_method(
                        method,
                        &trait_basename,
                        type_name,
                        impl_name,
                        &impl_block.target_type,
                    )?;
                    // Async `Drop::drop` is registered under the
                    // disambiguated symbol name `drop_async` (mirrors the
                    // `func_def.name` disambiguation in
                    // `desugar_impl_method`). Registering both the sync and
                    // async variant under the bare `drop` key would let the
                    // last-declared impl overwrite the first — making
                    // drop-variant selection DECLARATION-ORDER dependent
                    // (a sync `DropCall` would resolve to whichever variant
                    // happened to be declared last). The runtime
                    // (`op_drop_call_impl`) looks up `drop_async` for the
                    // async opcode and `drop` for the sync opcode, so the
                    // symbol key must carry the same distinction.
                    let symbol_method_name =
                        if trait_basename == "Drop" && method.name == "drop" && method.is_async {
                            "drop_async"
                        } else {
                            method.name.as_str()
                        };
                    self.program.register_trait_method_symbol(
                        &trait_basename,
                        type_name,
                        impl_name,
                        symbol_method_name,
                        &func_def.name,
                    );
                    self.register_function(&func_def)?;

                    // Track drop kind per type (sync, async, or both)
                    if trait_basename == "Drop" && method.name == "drop" {
                        let type_key = type_name.to_string();
                        let existing = self.drop_type_info.get(&type_key).copied();
                        let new_kind = if method.is_async {
                            match existing {
                                Some(DropKind::SyncOnly) | Some(DropKind::Both) => DropKind::Both,
                                _ => DropKind::AsyncOnly,
                            }
                        } else {
                            match existing {
                                Some(DropKind::AsyncOnly) | Some(DropKind::Both) => DropKind::Both,
                                _ => DropKind::SyncOnly,
                            }
                        };
                        self.drop_type_info.insert(type_key, new_kind);
                    }
                }

                // Install default methods from the trait definition that were not overridden
                if let Some(trait_def) = self.trait_defs.get(&canonical_trait).cloned() {
                    for member in &trait_def.members {
                        if let shape_ast::ast::types::TraitMember::Default(default_method) = member
                        {
                            if !overridden.contains(default_method.name.as_str()) {
                                let func_def = self.desugar_impl_method(
                                    default_method,
                                    &trait_basename,
                                    type_name,
                                    impl_name,
                                    &impl_block.target_type,
                                )?;
                                self.program.register_trait_method_symbol(
                                    &trait_basename,
                                    type_name,
                                    impl_name,
                                    &default_method.name,
                                    &func_def.name,
                                );
                                self.register_function(&func_def)?;
                            }
                        }
                    }
                }

                // ADR-006 §2.7.24 Q25.C: build a VTable for this
                // `(impl Trait for Type)` pair. The VTable is the
                // runtime artifact `op_box_trait_object` consults to
                // allocate `Arc<TraitObjectStorage>` at coerce-to-dyn
                // sites, and the artifact `op_dyn_method_call` consults
                // to dispatch methods through `dyn T`. Built once per
                // impl, shared via `Arc<VTable>` (the vtable half of
                // the fat pointer; see §Q25.C row 1).
                //
                // Trait method shapes handled at this round (Wave 2.6
                // round-2):
                //  - `Direct`: return type does not name `Self`, no
                //    Self-typed args, no method generics. Plain
                //    function-id dispatch.
                //  - `BoxedReturn`: return type is `Self` (path=[]).
                //    The auto-boxing thunk is generated by reusing the
                //    impl's function and post-processing its return at
                //    `op_dyn_method_call` time.
                //
                // Defection-attractor-safe surface for unhandled shapes:
                // `SelfArg` (§Q25.C.2), `Generic` (§Q25.C.3), `Compound`
                // (§Q25.C.5), and `BoxedReturn` with nested `Self`
                // (`Result<Self, E>`, `(Self, Self)`, etc.) surface as
                // `VMError::NotImplemented(SURFACE: ...)` at dispatch
                // time — see `op_dyn_method_call`. No silent default,
                // no Bool-default fallback (CLAUDE.md "Renames to refuse
                // on sight"; phase-2d-hardening item (a)).
                self.build_and_register_vtable(&trait_basename, type_name, impl_block)?;

                // BUG-4.6 fix: Register the trait impl in the type inference
                // environment so that `implements()` can see it at comptime.
                let all_method_names: Vec<String> =
                    impl_block.methods.iter().map(|m| m.name.clone()).collect();
                if let Some(selector) = impl_name {
                    let _ = self.type_inference.env.register_trait_impl_named(
                        &trait_basename,
                        type_name,
                        selector,
                        all_method_names,
                    );
                } else {
                    let _ = self.type_inference.env.register_trait_impl(
                        &trait_basename,
                        type_name,
                        all_method_names,
                    );
                }

                // C3: Verify supertrait constraints.
                // If the trait has supertraits (e.g. `trait Foo: Bar + Baz`),
                // check that the target type also implements each supertrait.
                if let Some(trait_def) = self.trait_defs.get(&canonical_trait).cloned() {
                    for super_ann in &trait_def.super_traits {
                        let super_name = match super_ann {
                            TypeAnnotation::Basic(name) => name.clone(),
                            TypeAnnotation::Reference(name) => name.to_string(),
                            TypeAnnotation::Generic { name, .. } => name.to_string(),
                            _ => continue,
                        };
                        let (_canonical_super, super_basename) =
                            self.resolve_trait_name(&super_name);
                        if !self
                            .type_inference
                            .env
                            .type_implements_trait(type_name, &super_basename)
                        {
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "impl {} for {} requires supertrait '{}' to be implemented first",
                                    trait_basename, type_name, super_basename
                                ),
                                location: None,
                            });
                        }
                    }
                }

                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// WS-9b pass-1 prepass: pre-register the runtime `TypeSchema` of every
    /// struct type declared anywhere in the program (including inside
    /// modules), so struct schemas are available before any function body
    /// compiles. Mirrors `register_item_functions`'s module recursion and
    /// name qualification so a module-scoped `type` registers under its
    /// fully-qualified name.
    ///
    /// Schema registration only — comptime-annotation handlers and the
    /// annotation lifecycle stay in pass 2's `register_struct_type`
    /// (`predeclare_struct_schema`'s `is_some()` guard makes that pass-2
    /// schema registration a no-op once the prepass has run).
    pub(super) fn predeclare_item_struct_schemas(&mut self, item: &Item) {
        match item {
            Item::StructType(struct_def, _) => {
                self.predeclare_struct_schema(struct_def);
            }
            Item::Export(export, _) => {
                if let ExportItem::Struct(struct_def) = &export.item {
                    self.predeclare_struct_schema(struct_def);
                }
            }
            Item::Module(module_def, _) => {
                let module_path = self.current_module_path_for(module_def.name.as_str());
                self.module_scope_stack.push(module_path.clone());
                for inner in &module_def.items {
                    if let Ok(qualified) = self.qualify_module_item(inner, &module_path) {
                        self.predeclare_item_struct_schemas(&qualified);
                    }
                }
                self.module_scope_stack.pop();
            }
            _ => {}
        }
    }

    /// Register a function definition
    pub(super) fn register_function(&mut self, func_def: &FunctionDef) -> Result<()> {
        // Detect duplicate function definitions (Shape does not support overloading).
        // Skip names containing "::" (trait impl methods) or "." (extend methods)
        // — those are type-qualified and live in separate namespaces.
        if !func_def.name.contains("::") && !func_def.name.contains('.') {
            if let Some(existing) = self
                .program
                .functions
                .iter()
                .find(|f| f.name == func_def.name)
            {
                // Allow idempotent re-registration from module inlining: when the
                // prelude and an explicitly imported module both define the same helper
                // function (e.g., `percentile`), silently keep the first definition
                // if arities match. Different arities indicate a genuine conflict.
                if existing.arity == func_def.params.len() as u16 {
                    return Ok(());
                }
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Duplicate function definition: '{}' is already defined",
                        func_def.name
                    ),
                    location: Some(self.span_to_source_location(func_def.name_span)),
                });
            }
        }

        self.function_defs
            .insert(func_def.name.clone(), func_def.clone());

        let total_params = func_def.params.len();
        let mut required_params = total_params;
        let mut saw_default = false;
        let mut const_params = Vec::new();
        for (idx, param) in func_def.params.iter().enumerate() {
            if param.is_const {
                const_params.push(idx);
            }
            if param.default_value.is_some() {
                if !saw_default {
                    required_params = idx;
                    saw_default = true;
                }
            } else if saw_default {
                return Err(ShapeError::SemanticError {
                    message: "Required parameter cannot follow a parameter with a default value"
                        .to_string(),
                    location: Some(self.span_to_source_location(param.span())),
                });
            }
        }

        self.function_arity_bounds
            .insert(func_def.name.clone(), (required_params, total_params));
        self.function_const_params
            .insert(func_def.name.clone(), const_params);

        let inferred_param_modes = self
            .inferred_param_pass_modes
            .get(&func_def.name)
            .cloned()
            .unwrap_or_default();
        let mut ref_params: Vec<bool> = Vec::with_capacity(func_def.params.len());
        let mut ref_mutates: Vec<bool> = Vec::with_capacity(func_def.params.len());
        for (idx, param) in func_def.params.iter().enumerate() {
            let fallback = if param.is_reference {
                ParamPassMode::ByRefShared
            } else {
                ParamPassMode::ByValue
            };
            let mode = inferred_param_modes.get(idx).copied().unwrap_or(fallback);
            ref_params.push(mode.is_reference());
            ref_mutates.push(mode.is_exclusive());
        }

        let func = Function {
            name: func_def.name.clone(),
            arity: func_def.params.len() as u16,
            param_names: func_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect(),
            locals_count: 0, // Will be updated during compilation
            entry_point: 0,  // Will be updated during compilation
            body_length: 0,  // Will be updated during compilation
            is_closure: false,
            captures_count: 0,
            is_async: func_def.is_async,
            ref_params,
            ref_mutates,
            mutable_captures: Vec::new(),
            frame_descriptor: None,
            osr_entry_points: Vec::new(),
            mir_data: None,
        };

        self.program.functions.push(func);

        // Register function return type for typed opcode emission.
        // When a function has an explicit return type annotation (e.g., `: int`),
        // record its ConcreteType so call sites can propagate the numeric type
        // through expressions like `fib(n-1) + fib(n-2)` and emit AddInt instead
        // of generic Add. U4-5b: registered STRUCTURALLY as a `ConcreteType` — no
        // `as_simple_name()` display string. A shape with no `ConcreteType`
        // projection (unresolved annotation) registers nothing (surface-and-stop).
        if let Some(ref return_type) = func_def.return_type {
            // Resolve via the SCHEMA-AWARE `declared_annotation_concrete_type`
            // (not the bare `concrete_type_from_annotation`) so a named
            // struct/enum return (`fn make() -> Box`) resolves to
            // `ConcreteType::Struct`/`Enum` — the v2-typed-array element-carrier
            // detection (`array_elements_all_typed_object`) needs the struct
            // identity. An unresolvable annotation registers nothing
            // (surface-and-stop).
            if let Some(ct) =
                crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                    self,
                    return_type,
                )
            {
                self.type_tracker
                    .register_function_return_concrete_type(&func_def.name, ct);
            }
        }

        Ok(())
    }

    /// Compile a top-level item with context about whether it's the last item
    /// If is_last is true and the item is an expression, keep the result on the stack
    pub(super) fn compile_item_with_context(&mut self, item: &Item, is_last: bool) -> Result<()> {
        match item {
            Item::Function(func_def, _) => self.compile_function(func_def)?,
            Item::Module(module_def, span) => {
                self.compile_module_decl(module_def, *span)?;
            }
            Item::VariableDecl(var_decl, span) => {
                // R8 W8 Cluster A (2026-05-24): module-level `const`
                // initializers must be comptime-evaluable. Reject
                // runtime-only initializers (function calls, identifier
                // references to non-const bindings, etc.) with a clean
                // compile error. ADR-006 §2.7.5 stamp-at-compile-time
                // invariant: the const's value must be known at compile
                // time so the bytecode emits `PushConst(<value>)` instead
                // of a deferred runtime computation.
                if var_decl.kind == shape_ast::ast::VarKind::Const {
                    if let Some(ref init_expr) = var_decl.value {
                        if !Self::const_initializer_is_comptime_evaluable(init_expr) {
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "module-level `const` initializer must be comptime-evaluable \
                                     (literal, comptime block, or unary `-`/`!` on a literal). \
                                     Function calls and other runtime-dependent expressions are \
                                     rejected per R8 W8 Cluster A (2026-05-24). \
                                     Extending the comptime evaluator is v0.4-concurrency-design-pass \
                                     territory per docs/v0.3-close-summary.md \u{a7}5.15."
                                ),
                                location: Some(self.span_to_source_location(*span)),
                            });
                        }
                    }
                }
                // ModuleBinding variable — register the variable even if the initializer fails,
                // to prevent cascading "Undefined variable" errors on later references.
                let mut ref_borrow = None;
                let init_err = if let Some(init_expr) = &var_decl.value {
                    let saved_pending_variable_name = self.pending_variable_name.clone();
                    let saved_pending_variable_span = self.pending_variable_span;
                    let saved_pending_variable_typed_array_kind =
                        self.pending_variable_typed_array_kind;
                    self.pending_variable_name = var_decl
                        .pattern
                        .as_identifier()
                        .map(|name| name.to_string());
                    self.pending_variable_span = var_decl.pattern.as_identifier_span();
                    // v2 Phase 3.1 (Agent 3): when the binding has an
                    // explicit `Array<T>` annotation whose element type
                    // maps to a typed-array kind, signal it to
                    // `compile_expr_array` so the literal is lowered to
                    // the v2 typed-array path.
                    //
                    // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element
                    // (2026-05-18): route through the compiler-aware
                    // `resolve_typed_array_kind_from_annotation` so `Array<B>`
                    // for a registered user struct B also maps to
                    // `TypedArrayKind::TypedObject` per audit §2.1 + §3.A row 1.
                    self.pending_variable_typed_array_kind = var_decl
                        .type_annotation
                        .as_ref()
                        .and_then(|ann| self.resolve_typed_array_kind_and_record_trait(ann));
                    match self.compile_expr_for_reference_binding_with_expected_return(
                        init_expr,
                        var_decl.type_annotation.as_ref(),
                    ) {
                        Ok(tracked_borrow) => {
                            ref_borrow = tracked_borrow;
                            self.pending_variable_name = saved_pending_variable_name;
                            self.pending_variable_span = saved_pending_variable_span;
                            self.pending_variable_typed_array_kind =
                                saved_pending_variable_typed_array_kind;
                            None
                        }
                        Err(e) => {
                            self.pending_variable_name = saved_pending_variable_name;
                            self.pending_variable_span = saved_pending_variable_span;
                            self.pending_variable_typed_array_kind =
                                saved_pending_variable_typed_array_kind;
                            // Push null as placeholder so the variable still gets registered
                            self.emit(Instruction::simple(OpCode::PushNull));
                            Some(e)
                        }
                    }
                } else {
                    self.emit(Instruction::simple(OpCode::PushNull));
                    None
                };
                if init_err.is_none() {
                    self.patch_static_set_ctor_from_annotation(
                        var_decl.value.as_ref(),
                        var_decl.type_annotation.as_ref(),
                    );
                }
                // Phase 4b Round 6 WS-1b W16.2-C residual: capture the bare
                // empty-array-accumulator placeholder index now, before any
                // downstream emission can reset it.
                let captured_empty_array_alloc_idx = self.pending_empty_array_alloc_idx.take();

                if let Some(name) = var_decl.pattern.as_identifier() {
                    // v0.3.3 c6 (Wave 1): re-add narrow B0003 guard for
                    // module-scope `let r = &x`. Module-level top-level
                    // statements are NOT lowered to MIR (the MIR solver
                    // runs per-function only), so the solver cannot see
                    // this binding. Without this categorical guard the
                    // ref-binding silently runs (returns Bool(false)) and
                    // bypasses borrow analysis entirely. Per audit
                    // `docs/cluster-audits/v0.3.3/06-borrow-check-bypass.md`
                    // §5(a). Defense-in-depth alongside the MIR solver's
                    // new `LoanSinkKind::ModuleBindingStore` (which
                    // catches the in-function `module_g = &local` shape
                    // that this site doesn't see). The R8 W9 B9 deletion
                    // (commit 8bbd2f99) was wrong to remove this — the
                    // claim "MIR is the sole authority" only holds inside
                    // function bodies; module-scope top-level statements
                    // need a dedicated guard.
                    //
                    // ADR-006 §2.7.30 (FlipLive): the guard now flips for the
                    // EXACT `ModuleBindingStore` floor sink — `let r = &x`
                    // where `x` is itself a program-lifetime module binding.
                    // Such a referent outlives every reference to it, so the
                    // escape→RC promotion is unconditionally sound and the
                    // binding compiles + reads through the `RefTarget::
                    // ModuleBinding` carrier. Any OTHER reference escape
                    // (referent rooted at a local, etc.) is NOT a floor sink
                    // and still rejects with B0003.
                    let referent_is_module_floor = var_decl
                        .value
                        .as_ref()
                        .is_some_and(|expr| self.reference_root_is_module_binding(expr));
                    if ref_borrow.is_some() && !referent_is_module_floor {
                        return Err(ShapeError::SemanticError {
                            message:
                                "[B0003] cannot return or store a reference that outlives its owner"
                                    .to_string(),
                            location: var_decl
                                .value
                                .as_ref()
                                .map(|expr| self.span_to_source_location(expr.span())),
                        });
                    }
                    let binding_idx = self.get_or_create_module_binding(name);
                    if let Some(span) = var_decl.pattern.as_identifier_span()
                        && !span.is_dummy()
                    {
                        self.module_binding_spans.insert(binding_idx, span);
                    }
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    // v2 Phase 3.1 (Agent 3): record that this binding holds
                    // a v2 typed array if the annotation drove the typed
                    // path during initializer compilation. The kind was
                    // captured in `pending_variable_typed_array_kind` BEFORE
                    // the initializer compiled and is still set if the
                    // typed path was taken.
                    if let Some(kind) = self.pending_variable_typed_array_kind {
                        self.v2_typed_array_module_bindings
                            .insert(binding_idx, kind);
                    }
                    // Phase 4b Round 6 WS-1b W16.2-C residual: re-key a bare
                    // empty-array-accumulator placeholder against this module
                    // binding so the first downstream `.push()` can resolve
                    // its element kind and patch the allocator.
                    self.register_empty_array_accumulator(
                        crate::compiler::EmptyArrayAccumulatorKey::ModuleBinding(binding_idx),
                        var_decl.value.as_ref(),
                        captured_empty_array_alloc_idx,
                        name,
                        var_decl.value.as_ref().map(|v| v.span()),
                    );
                    if let Some(value) = &var_decl.value {
                        self.finish_reference_binding_from_expr(
                            binding_idx,
                            false,
                            name,
                            value,
                            ref_borrow,
                        );
                        self.update_callable_binding_from_expr(binding_idx, false, value);
                    } else {
                        self.clear_reference_binding(binding_idx, false);
                        self.clear_callable_binding(binding_idx, false);
                    }

                    // Propagate type info from annotation or initializer expression
                    if let Some(ref type_ann) = var_decl.type_annotation {
                        if let Some(type_name) = Self::tracked_type_name_from_annotation(type_ann) {
                            self.set_module_binding_type_info(binding_idx, &type_name);
                        }
                    } else {
                        let is_mutable = var_decl.kind == shape_ast::ast::VarKind::Var;
                        self.propagate_initializer_type_to_slot(
                            binding_idx,
                            false,
                            is_mutable,
                            var_decl.value.as_ref(),
                        );
                    }

                    // Track for auto-drop at program exit
                    let binding_type_name = self
                        .type_tracker
                        .get_binding_type(binding_idx)
                        .and_then(|info| info.type_name.clone());
                    let drop_kind = binding_type_name
                        .as_ref()
                        .and_then(|tn| self.drop_type_info.get(tn).copied())
                        .or_else(|| {
                            var_decl
                                .type_annotation
                                .as_ref()
                                .and_then(|ann| self.annotation_drop_kind(ann))
                        });
                    if drop_kind.is_some() {
                        let is_async = match drop_kind {
                            Some(DropKind::AsyncOnly) => true,
                            Some(DropKind::Both) => false,
                            Some(DropKind::SyncOnly) | None => false,
                        };
                        self.track_drop_module_binding(binding_idx, is_async);
                    }
                } else {
                    self.compile_destructure_pattern_global(&var_decl.pattern)?;
                }

                if let Some(e) = init_err {
                    return Err(e);
                }
            }
            Item::Assignment(assign, _) => {
                self.compile_statement(&Statement::Assignment(assign.clone(), Span::DUMMY))?;
            }
            Item::Expression(expr, _) => {
                self.compile_expr(expr)?;
                // Only pop if not the last item - keep last expression result on stack
                if !is_last {
                    self.emit(Instruction::simple(OpCode::Pop));
                }
            }
            Item::Statement(stmt, stmt_item_span) => {
                // R8 W8 Cluster A (2026-05-24): reject runtime-only `const`
                // initializers at the top-level script-item path. Module-
                // scoped `const`s reach `Item::VariableDecl` via the
                // qualify pass; script-level `const`s reach
                // `Item::Statement(Statement::VariableDecl)` per the
                // grammar (`item_core → statement → variable_decl`).
                if let Statement::VariableDecl(var_decl, decl_span) = stmt {
                    if var_decl.kind == shape_ast::ast::VarKind::Const {
                        if let Some(ref init_expr) = var_decl.value {
                            if !Self::const_initializer_is_comptime_evaluable(init_expr) {
                                return Err(ShapeError::SemanticError {
                                    message: format!(
                                        "`const` initializer must be comptime-evaluable \
                                         (literal, comptime block, or unary `-`/`!` on a literal). \
                                         Function calls and other runtime-dependent expressions \
                                         are rejected per R8 W8 Cluster A (2026-05-24). \
                                         Extending the comptime evaluator is v0.4-concurrency-\
                                         design-pass territory per docs/v0.3-close-summary.md \u{a7}5.15."
                                    ),
                                    location: Some(self.span_to_source_location(*decl_span)),
                                });
                            }
                        }
                    }
                }
                let _ = stmt_item_span;
                // For expression statements that are the last item, keep result on stack
                if is_last {
                    if let Statement::Expression(expr, _) = stmt {
                        self.compile_expr(expr)?;
                        // Don't emit Pop - keep result on stack
                        return Ok(());
                    }
                }
                self.compile_statement(stmt)?;
            }
            Item::Export(export, export_span) => {
                // If the export has a source variable declaration (pub let/const/var),
                // compile it so the initialization is actually executed.
                if let Some(ref var_decl) = export.source_decl {
                    let mut ref_borrow = None;
                    if let Some(init_expr) = &var_decl.value {
                        let saved_pending_variable_name = self.pending_variable_name.clone();
                        let saved_pending_variable_span = self.pending_variable_span;
                        let saved_pending_variable_typed_array_kind =
                            self.pending_variable_typed_array_kind;
                        self.pending_variable_name = var_decl
                            .pattern
                            .as_identifier()
                            .map(|name| name.to_string());
                        self.pending_variable_span = var_decl.pattern.as_identifier_span();
                        // v2 Phase 3.1 (Agent 3): see ModuleBinding case above.
                        // Phase 4b Round 4 W16.2-A (2026-05-18): user-struct
                        // annotation support via `resolve_typed_array_kind_from_annotation`.
                        self.pending_variable_typed_array_kind = var_decl
                            .type_annotation
                            .as_ref()
                            .and_then(|ann| self.resolve_typed_array_kind_and_record_trait(ann));
                        let compile_result = self
                            .compile_expr_for_reference_binding_with_expected_return(
                                init_expr,
                                var_decl.type_annotation.as_ref(),
                            );
                        self.pending_variable_name = saved_pending_variable_name;
                        self.pending_variable_span = saved_pending_variable_span;
                        self.pending_variable_typed_array_kind =
                            saved_pending_variable_typed_array_kind;
                        ref_borrow = compile_result?;
                        // ADR-006 §2.7.30 (FlipLive): flip EXACTLY the
                        // `ModuleBindingStore` floor sink — `pub let r = &x`
                        // where `x` is a program-lifetime module binding.
                        // Same scoping predicate as the non-export site above.
                        let referent_is_module_floor =
                            self.reference_root_is_module_binding(init_expr);
                        if ref_borrow.is_some() && !referent_is_module_floor {
                            return Err(ShapeError::SemanticError {
                                message:
                                    "[B0003] cannot return or store a reference that outlives its owner"
                                        .to_string(),
                                location: Some(self.span_to_source_location(init_expr.span())),
                            });
                        }
                    } else {
                        self.emit(Instruction::simple(OpCode::PushNull));
                    }
                    self.patch_static_set_ctor_from_annotation(
                        var_decl.value.as_ref(),
                        var_decl.type_annotation.as_ref(),
                    );
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        let binding_idx = self.get_or_create_module_binding(name);
                        if let Some(span) = var_decl.pattern.as_identifier_span()
                            && !span.is_dummy()
                        {
                            self.module_binding_spans.insert(binding_idx, span);
                        }
                        self.emit(Instruction::new(
                            OpCode::StoreModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                        if let Some(value) = &var_decl.value {
                            self.finish_reference_binding_from_expr(
                                binding_idx,
                                false,
                                name,
                                value,
                                ref_borrow,
                            );
                            self.update_callable_binding_from_expr(binding_idx, false, value);
                        } else {
                            self.clear_reference_binding(binding_idx, false);
                            self.clear_callable_binding(binding_idx, false);
                        }
                    }
                }
                match &export.item {
                    ExportItem::Function(func_def) => self.compile_function(func_def)?,
                    ExportItem::Annotation(annotation_def) => {
                        self.compile_annotation_def(annotation_def)?;
                    }
                    ExportItem::Enum(enum_def) => self.register_enum(enum_def)?,
                    ExportItem::Struct(struct_def) => {
                        self.register_struct_type(struct_def, *export_span)?;
                        if self.struct_types.contains_key(&struct_def.name) {
                            self.emit_annotation_lifecycle_calls_for_type(
                                &struct_def.name,
                                &struct_def.annotations,
                            )?;
                        }
                    }
                    ExportItem::Trait(_) => {} // no-op for now (trait registration happens in type system)
                    ExportItem::ForeignFunction(def) => self.compile_foreign_function(def)?,
                    _ => {}
                }
            }
            Item::Stream(_stream, _) => {
                return Err(ShapeError::StreamError {
                    message: "Streaming functionality has been removed".to_string(),
                    stream_name: None,
                });
            }
            Item::TypeAlias(type_alias, _) => {
                // Track type alias for meta validation
                let base_type_name = match &type_alias.type_annotation {
                    TypeAnnotation::Basic(name) => Some(name.clone()),
                    TypeAnnotation::Reference(name) => Some(name.to_string()),
                    _ => None,
                };
                self.type_aliases.insert(
                    type_alias.name.clone(),
                    base_type_name
                        .clone()
                        .unwrap_or_else(|| format!("{:?}", type_alias.type_annotation)),
                );
                // Register in type inference environment so lookup_type_alias works
                self.type_inference.env.define_type_alias(
                    &type_alias.name,
                    &type_alias.type_annotation,
                    type_alias.meta_param_overrides.clone(),
                );

                // Apply comptime field overrides from type alias
                // (e.g., `type EUR = Currency { symbol: "€" }` overrides
                // Currency's comptime symbol).
                //
                // **Phase-2c rebuild pending — see ADR-006 §2.4.** The
                // previous body materialized the override RHS literals into
                // a `shape_value::ValueMap` (alias for `HashMap<String,
                // ValueWord>`), inserting per-FieldType `ValueWord::from_*`
                // constructions keyed by override field name. After the
                // strict-typing bulldozer:
                //
                // - `ValueWord` and the `ValueMap` typedef are deleted from
                //   `shape-value` (per CLAUDE.md "Renames to refuse on
                //   sight" and ADR-006 §2.4 / Q6).
                // - `comptime_fields: HashMap<String, ValueMap>` on the
                //   compiler struct (`compiler/mod.rs:906`) is itself the
                //   forbidden carrier — its rebuild lives in the cluster
                //   that owns `mod.rs`.
                // - The kinded replacement is `HashMap<String,
                //   HashMap<String, KindedSlot>>` per ADR-006 §2.7.1.3
                //   (vector storage with parallel kind tracks): each
                //   override RHS becomes a `KindedSlot` constructed by
                //   per-Literal arm (`Literal::Int(n) => KindedSlot::from_int(n)`,
                //   etc.), preserving the kind without round-tripping
                //   through a tagged dynamic word.
                //
                // Until Phase 2c lands, this branch is a structural no-op:
                // type-alias comptime field overrides are silently dropped
                // rather than panicking, so the compile succeeds for the
                // 99% case (no overrides). Property-access reads against
                // an aliased comptime field will fall through to the base
                // type's value (also a phase-2c surface). Suppressing the
                // override is the correct boundary per playbook §7 #4: we
                // do not synthesize a placeholder ValueMap that would
                // poison the comptime_fields registry.
                let _ = (&base_type_name, &type_alias.meta_param_overrides);
            }
            Item::StructType(struct_def, span) => {
                self.register_struct_type(struct_def, *span)?;
                if self.struct_types.contains_key(&struct_def.name) {
                    self.emit_annotation_lifecycle_calls_for_type(
                        &struct_def.name,
                        &struct_def.annotations,
                    )?;
                }
            }
            Item::Enum(enum_def, _) => {
                self.register_enum(enum_def)?;
            }
            // Meta/Format definitions removed — formatting now uses Display trait
            Item::Import(import_stmt, _) => {
                // Import resolution is handled by the module graph pipeline
                // before compilation. At this point imports should already
                // have been resolved via `register_graph_imports_for_module`.
                // If we reach here, the import is either:
                // 1. Being compiled standalone (no module context) - skip for now
                // 2. A future extension point for runtime imports
                //
                // For now, we register the imported names as known functions
                // that can be resolved later.
                self.register_import_names(import_stmt)?;
            }
            Item::Extend(extend, _) => {
                // Compile desugared extend methods
                for method in &extend.methods {
                    let func_def = self.desugar_extend_method(method, &extend.type_name)?;
                    self.compile_function(&func_def)?;
                }
            }
            Item::Impl(impl_block, _) => {
                // J-CT.2 — comptime impl blocks are deferred; the
                // first-pass arm captured them in `comptime_impl_blocks`
                // and skipped runtime processing, so the second pass also
                // skips. Their bodies are compiled inside the comptime
                // mini-VM (`execute_comptime`).
                if impl_block.is_comptime {
                    return Ok(());
                }
                // Compile impl block methods with scoped names
                let raw_trait_name = match &impl_block.trait_name {
                    shape_ast::ast::types::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::types::TypeName::Generic { name, .. } => name.as_str(),
                };
                let type_name = match &impl_block.target_type {
                    shape_ast::ast::types::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::types::TypeName::Generic { name, .. } => name.as_str(),
                };
                let impl_name = impl_block.impl_name.as_deref();

                // Resolve trait name: canonical for def lookup, basename for dispatch
                let (canonical_trait, trait_basename) = self.resolve_trait_name(raw_trait_name);

                // From/TryFrom: compile the from/tryFrom method + synthetic wrapper
                if trait_basename == "From" || trait_basename == "TryFrom" {
                    return self.compile_from_impl_bodies(impl_block, &trait_basename, type_name);
                }

                // Collect names of methods explicitly provided in the impl block
                let overridden: std::collections::HashSet<&str> =
                    impl_block.methods.iter().map(|m| m.name.as_str()).collect();

                for method in &impl_block.methods {
                    let func_def = self.desugar_impl_method(
                        method,
                        &trait_basename,
                        type_name,
                        impl_name,
                        &impl_block.target_type,
                    )?;
                    self.compile_function(&func_def)?;
                }

                // Compile default methods from the trait definition that were not overridden
                if let Some(trait_def) = self.trait_defs.get(&canonical_trait).cloned() {
                    for member in &trait_def.members {
                        if let shape_ast::ast::types::TraitMember::Default(default_method) = member
                        {
                            if !overridden.contains(default_method.name.as_str()) {
                                let func_def = self.desugar_impl_method(
                                    default_method,
                                    &trait_basename,
                                    type_name,
                                    impl_name,
                                    &impl_block.target_type,
                                )?;
                                self.compile_function(&func_def)?;
                            }
                        }
                    }
                }
            }
            Item::AnnotationDef(ann_def, _) => {
                self.compile_annotation_def(ann_def)?;
            }
            Item::Comptime(stmts, span) => {
                // Execute comptime block at compile time (side-effects only; result discarded)
                let extensions: Vec<_> = self
                    .extension_registry
                    .as_ref()
                    .map(|r| r.as_ref().clone())
                    .unwrap_or_default();
                let trait_impls = self.type_inference.env.trait_impl_keys();
                let known_type_symbols: std::collections::HashSet<String> = self
                    .struct_types
                    .keys()
                    .chain(self.type_aliases.keys())
                    .cloned()
                    .collect();
                let comptime_helpers = self.collect_comptime_helpers();
                // W7 (2026-05-17): build the TypeReflectionSnapshot for
                // `type_info(T)` resolution. Top-level comptime block has
                // no enclosing generic-type-param scope.
                let type_snapshot =
                    super::comptime_builtins::build_type_reflection_snapshot(self, &[]);
                // J-CT.2 — see `expressions/mod.rs::Expr::Comptime` for
                // rationale on comptime-context items.
                let comptime_impl_blocks = self.comptime_impl_blocks.clone();
                let comptime_context_trait_defs: Vec<_> =
                    self.trait_defs.values().cloned().collect();
                let comptime_context_struct_defs: Vec<_> = self
                    .comptime_context_struct_defs
                    .values()
                    .cloned()
                    .collect();
                let execution = super::comptime::execute_comptime_with_context(
                    stmts,
                    &comptime_helpers,
                    &comptime_impl_blocks,
                    &comptime_context_trait_defs,
                    &comptime_context_struct_defs,
                    &extensions,
                    trait_impls,
                    known_type_symbols,
                    type_snapshot,
                )
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!(
                        "Comptime block evaluation failed: {}",
                        super::helpers::strip_error_prefix(&e)
                    ),
                    location: Some(self.span_to_source_location(*span)),
                })?;
                self.process_comptime_directives(execution.directives, "")
                    .map_err(|e| ShapeError::RuntimeError {
                        message: format!("Comptime block directive processing failed: {}", e),
                        location: Some(self.span_to_source_location(*span)),
                    })?;
            }
            Item::Query(query, _span) => {
                self.compile_query(query)?;
                // Pop the query result unless self is the last item
                if !is_last {
                    self.emit(Instruction::simple(OpCode::Pop));
                }
            }
            Item::ForeignFunction(def, _) => self.compile_foreign_function(def)?,
            _ => {} // Skip other items for now
        }
        Ok(())
    }

    /// Register imported names for symbol resolution
    ///
    /// This allows the compiler to recognize imported functions when
    /// they are called later in the code.
    fn register_import_names(&mut self, import_stmt: &shape_ast::ast::ImportStmt) -> Result<()> {
        use shape_ast::ast::ImportItems;

        // Check permissions before registering imports.
        // Clone to avoid borrow conflict with &mut self in check_import_permissions.
        if let Some(pset) = self.permission_set.clone() {
            self.check_import_permissions(import_stmt, &pset)?;
        }

        match &import_stmt.items {
            ImportItems::Named(specs) => {
                for spec in specs {
                    if spec.is_annotation {
                        // W9: register against the canonical module path so the
                        // use-site lookup matches `compiled_annotations`'s
                        // qualified key produced by the dep module's qualify pass.
                        self.module_scope_sources
                            .entry(import_stmt.from.clone())
                            .or_insert_with(|| import_stmt.from.clone());
                        self.imported_annotations.insert(
                            spec.name.clone(),
                            ImportedAnnotationSymbol {
                                original_name: spec.name.clone(),
                                _module_path: import_stmt.from.clone(),
                                hidden_module_name: import_stmt.from.clone(),
                            },
                        );
                        continue;
                    }
                    let local_name = spec.alias.as_ref().unwrap_or(&spec.name);
                    // Register as a known import - actual function resolution
                    // happens when the imported module's bytecode is merged
                    self.imported_names.insert(
                        local_name.clone(),
                        ImportedSymbol {
                            original_name: spec.name.clone(),
                            module_path: import_stmt.from.clone(),
                            kind: None, // legacy path
                        },
                    );
                }
            }
            ImportItems::Namespace { name, alias } => {
                // `use module.path` or `use module.path as alias`
                // Register the local namespace binding as a module_binding.
                let local_name = alias.as_ref().unwrap_or(name);
                let binding_idx = self.get_or_create_module_binding(local_name);
                self.module_namespace_bindings.insert(local_name.clone());
                self.module_scope_sources
                    .entry(local_name.clone())
                    .or_insert_with(|| import_stmt.from.clone());
                let module_path = if import_stmt.from.is_empty() {
                    name.as_str()
                } else {
                    import_stmt.from.as_str()
                };
                // Predeclare module object schema so runtime can instantiate
                // module module_bindings without synthesizing schemas dynamically.
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
                // The module object will be provided at runtime by the VM
                let _ = binding_idx;
            }
        }
        Ok(())
    }

    /// Check whether the imported symbols are allowed by the active permission set.
    ///
    /// For named imports (`from std::core::file use { read_text }`), checks each function
    /// individually. For namespace imports (`use std::core::http`), checks the whole module.
    fn check_import_permissions(
        &mut self,
        import_stmt: &shape_ast::ast::ImportStmt,
        pset: &shape_abi_v1::PermissionSet,
    ) -> Result<()> {
        use shape_ast::ast::ImportItems;
        use shape_runtime::stdlib::capability_tags;

        // Pass the full canonical path (e.g. "std::core::file") to capability tags.
        let module_name = &import_stmt.from as &str;

        match &import_stmt.items {
            ImportItems::Named(specs) => {
                for spec in specs {
                    let required = capability_tags::required_permissions(module_name, &spec.name);
                    if !required.is_empty() && !required.is_subset(pset) {
                        let missing = required.difference(pset);
                        let missing_names: Vec<&str> = missing.iter().map(|p| p.name()).collect();
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Permission denied: {module_name}::{} requires {} capability, \
                                 but the active permission set does not include it. \
                                 Add the permission to [permissions] in shape.toml or use a less \
                                 restrictive preset.",
                                spec.name,
                                missing_names.join(", "),
                            ),
                            location: None,
                        });
                    }
                    self.record_blob_permissions(module_name, &spec.name);
                }
            }
            ImportItems::Namespace { .. } => {
                // For namespace imports, check the entire module's permission envelope.
                // If the module requires any permissions not granted, deny the import.
                let required = capability_tags::module_permissions(module_name);
                if !required.is_empty() && !required.is_subset(pset) {
                    let missing = required.difference(pset);
                    let missing_names: Vec<&str> = missing.iter().map(|p| p.name()).collect();
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Permission denied: module '{module_name}' requires {} capabilities, \
                             but the active permission set does not include them. \
                             Add the permissions to [permissions] in shape.toml or use a less \
                             restrictive preset.",
                            missing_names.join(", "),
                        ),
                        location: None,
                    });
                }
                // Record module-level permissions for namespace imports in the current blob
                if let Some(ref mut blob) = self.current_blob_builder {
                    let module_perms = capability_tags::module_permissions(module_name);
                    blob.record_permissions(&module_perms);
                }
            }
        }
        Ok(())
    }

    /// Register imports for a module from the module graph.
    ///
    /// This is the graph-driven replacement for `register_import_names`.
    /// For each `ResolvedImport` on the node:
    /// - Namespace: creates canonical + alias bindings, registers schemas
    /// - Named: populates `imported_names`, `imported_annotations`, `module_builtin_functions`
    pub(super) fn register_graph_imports_for_module(
        &mut self,
        module_id: crate::module_graph::ModuleId,
        graph: &crate::module_graph::ModuleGraph,
    ) -> Result<()> {
        use crate::module_graph::{ModuleSourceKind, ResolvedImport};

        let node = graph.node(module_id);
        let resolved_imports = node.resolved_imports.clone();

        for ri in &resolved_imports {
            match ri {
                ResolvedImport::Namespace {
                    local_name,
                    canonical_path,
                    module_id: dep_id,
                } => {
                    let dep_node = graph.node(*dep_id);

                    // 1. Ensure canonical binding exists
                    let canonical_idx = self.get_or_create_module_binding(canonical_path);

                    // Register native schema on canonical binding for NativeModule/Hybrid
                    if matches!(
                        dep_node.source_kind,
                        ModuleSourceKind::NativeModule | ModuleSourceKind::Hybrid
                    ) {
                        self.register_extension_module_schema(canonical_path);
                        let module_schema_name = format!("__mod_{}", canonical_path);
                        if self
                            .type_tracker
                            .schema_registry()
                            .get(&module_schema_name)
                            .is_some()
                        {
                            self.set_module_binding_type_info(canonical_idx, &module_schema_name);
                        }
                    }

                    // 2. Create alias binding if local_name != canonical_path
                    if local_name != canonical_path {
                        let alias_idx = self.get_or_create_module_binding(local_name);

                        // Copy type info from canonical to alias. Shape-source
                        // modules get their object schema when the dependency
                        // module is compiled; native modules use the synthetic
                        // __mod_* schema registered above.
                        if let Some(type_info) =
                            self.type_tracker.get_binding_type(canonical_idx).cloned()
                        {
                            self.type_tracker.set_binding_type(alias_idx, type_info);
                        } else {
                            let module_schema_name = format!("__mod_{}", canonical_path);
                            if self
                                .type_tracker
                                .schema_registry()
                                .get(&module_schema_name)
                                .is_some()
                            {
                                self.set_module_binding_type_info(alias_idx, &module_schema_name);
                            }
                        }

                        // Emit runtime binding copy: alias = canonical
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(canonical_idx)),
                        ));
                        self.emit(Instruction::new(
                            OpCode::StoreModuleBinding,
                            Some(Operand::ModuleBinding(alias_idx)),
                        ));
                    }

                    // 3. Register namespace
                    self.module_namespace_bindings.insert(local_name.clone());
                    self.graph_namespace_map
                        .insert(local_name.clone(), canonical_path.clone());

                    // 4. W9: Register annotation defs from imported module so
                    // bare `@ann` and qualified `@local::ann` resolve at use-site.
                    // The compiled annotation lives in `compiled_annotations` under
                    // its qualified name `canonical_path::ann_name` (set during the
                    // dep module's own compile via `qualify_module_item`).
                    for (export_name, exp) in &dep_node.interface.exports {
                        if matches!(
                            exp.kind,
                            shape_ast::module_utils::ModuleExportKind::Annotation
                        ) {
                            self.imported_annotations
                                .entry(export_name.clone())
                                .or_insert_with(|| ImportedAnnotationSymbol {
                                    original_name: export_name.clone(),
                                    _module_path: canonical_path.clone(),
                                    hidden_module_name: canonical_path.clone(),
                                });
                        }
                    }
                }
                ResolvedImport::Named {
                    canonical_path,
                    module_id: dep_id,
                    symbols,
                } => {
                    let dep_node = graph.node(*dep_id);

                    for sym in symbols {
                        if sym.is_annotation {
                            // W9: register annotation symbol against the canonical
                            // module path. The compiled annotation is stored in
                            // `compiled_annotations` under `canonical_path::name`
                            // (the dep module's own qualify_module_item pass
                            // produces that qualified key), so use-site resolution
                            // just looks up `canonical_path::original_name`.
                            self.module_scope_sources
                                .entry(canonical_path.clone())
                                .or_insert_with(|| canonical_path.clone());
                            // Vacant-only: explicit imports win over prelude
                            self.imported_annotations
                                .entry(sym.local_name.clone())
                                .or_insert_with(|| ImportedAnnotationSymbol {
                                    original_name: sym.original_name.clone(),
                                    _module_path: canonical_path.clone(),
                                    hidden_module_name: canonical_path.clone(),
                                });
                            continue;
                        }

                        // Register as imported name (vacant-only: explicit imports
                        // are processed first and win over prelude entries)
                        self.imported_names
                            .entry(sym.local_name.clone())
                            .or_insert_with(|| ImportedSymbol {
                                original_name: sym.original_name.clone(),
                                module_path: canonical_path.clone(),
                                kind: Some(sym.kind),
                            });

                        // For native exports, register as module builtin function
                        if matches!(
                            dep_node.source_kind,
                            ModuleSourceKind::NativeModule | ModuleSourceKind::Hybrid
                        ) && matches!(
                            sym.kind,
                            shape_ast::module_utils::ModuleExportKind::Function
                                | shape_ast::module_utils::ModuleExportKind::BuiltinFunction
                        ) {
                            self.module_builtin_functions
                                .entry(sym.local_name.clone())
                                .or_insert_with(|| ModuleBuiltinFunction {
                                    export_name: sym.original_name.clone(),
                                    source_module_path: canonical_path.clone(),
                                });
                        }

                        // R8 W8 Cluster A: imported `pub const NAME = expr` —
                        // capture the initializer expression so the consumer-
                        // side identifier-load path can emit it inline as
                        // `PushConst(<comptime-value>)`. ADR-006 §2.7.5
                        // stamp-at-compile-time invariant preserved: the
                        // constant's kind is stamped from the literal at
                        // compile time when the compiler reaches the
                        // identifier reference.
                        if matches!(sym.kind, shape_ast::module_utils::ModuleExportKind::Value) {
                            if let Some(ref dep_ast) = dep_node.ast {
                                for item in &dep_ast.items {
                                    if let shape_ast::ast::Item::Export(export, _) = item {
                                        if let Some(ref decl) = export.source_decl {
                                            if decl.kind == shape_ast::ast::VarKind::Const {
                                                if let Some(decl_name) =
                                                    decl.pattern.as_identifier()
                                                {
                                                    if decl_name == sym.original_name {
                                                        if let Some(ref init) = decl.value {
                                                            self.imported_consts
                                                                .entry(sym.local_name.clone())
                                                                .or_insert_with(|| init.clone());
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub(super) fn register_extension_module_schema(&mut self, module_path: &str) {
        let Some(registry) = self.extension_registry.as_ref() else {
            return;
        };
        let Some(module) = registry.iter().rev().find(|m| m.name == module_path) else {
            return;
        };

        // Mirror extension type schemas into the per-bytecode registry,
        // preserving their pre-allocated IDs. To prevent the synthetic
        // `__mod_*` schemas (allocated below from the registry's per-instance
        // counter) from colliding with any extension schema's pre-allocated
        // ID, bump the per-instance counter past the highest ID observed
        // across *all* registered extensions. Scoping the bump to a single
        // module is insufficient because later `register_extension_module_schema`
        // calls can still introduce extension schemas with IDs <= the counter
        // we already advanced past.
        let mut global_max_ext_id: Option<shape_runtime::type_schema::SchemaId> = None;
        for ext_module in registry.iter() {
            for schema in &ext_module.type_schemas {
                global_max_ext_id = Some(match global_max_ext_id {
                    Some(prev) => prev.max(schema.id),
                    None => schema.id,
                });
            }
        }
        for schema in &module.type_schemas {
            if self
                .type_tracker
                .schema_registry()
                .get(&schema.name)
                .is_none()
            {
                self.type_tracker
                    .schema_registry_mut()
                    .register(schema.clone());
            }
        }
        if let Some(max_id) = global_max_ext_id {
            self.type_tracker
                .schema_registry()
                .ensure_next_id_above(max_id);
        }

        let schema_name = format!("__mod_{}", module_path);
        let mut export_names: Vec<String> = module
            .export_names_available(self.comptime_mode)
            .into_iter()
            .map(|name| name.to_string())
            .collect();

        for artifact in &module.module_artifacts {
            if artifact.module_path != module_path {
                continue;
            }
            let Some(source) = artifact.source.as_deref() else {
                continue;
            };
            if let Ok(names) =
                shape_runtime::module_loader::collect_exported_function_names_from_source(
                    &artifact.module_path,
                    source,
                )
            {
                export_names.extend(names);
            }
        }

        export_names.sort();
        export_names.dedup();

        let fields: Vec<(String, FieldType)> = export_names
            .into_iter()
            .map(|name| (name, FieldType::Any))
            .collect();
        // Allocate the synthetic `__mod_*` schema ID from the per-bytecode
        // registry's own counter, not the ambient (process-wide / per-Runtime)
        // counter that `register_type` consults via `TypeSchema::new`. The
        // ambient counter is shared with `state_builtins::create_state_module`
        // and other extension-schema constructors; sharing it here lets a
        // synthetic `__mod_<name>` schema receive the same ID as a previously
        // baked extension schema (e.g. `ModuleState`), causing the
        // per-bytecode `by_id` map to overwrite one with the other and
        // surfacing as "module 'X' has no export 'Y'" at compile time.
        self.type_tracker
            .schema_registry_mut()
            .upsert_type_scoped_union_fields(schema_name, fields);
    }

    /// Register an enum definition in the TypeSchemaRegistry
    fn register_enum(&mut self, enum_def: &EnumDef) -> Result<()> {
        let variants: Vec<EnumVariantInfo> = enum_def
            .members
            .iter()
            .enumerate()
            .map(|(id, member)| {
                // W18.0 (User 2026-05-23 Item 1): carry variant payload
                // shape into the runtime EnumVariantInfo so print() can
                // render `Red` / `Blue(42)` / `Point { x: 1, y: 2 }` per
                // the source-syntax form. The runtime TypedObject layout
                // is unchanged (`__payload_N` slots at offset 8/16/...);
                // the kind here only descriptively shapes the print form.
                match &member.kind {
                    EnumMemberKind::Unit { .. } => EnumVariantInfo::new(&member.name, id as u16, 0),
                    EnumMemberKind::Tuple(types) => {
                        EnumVariantInfo::new(&member.name, id as u16, types.len() as u16)
                    }
                    EnumMemberKind::Struct(fields) => {
                        let names: Vec<String> = fields.iter().map(|f| f.name.clone()).collect();
                        EnumVariantInfo::new_struct(&member.name, id as u16, names)
                    }
                }
            })
            .collect();

        // Sweep phase 3c.x: cache the struct-variant fields so match
        // patterns like `m::E::V { x, y }` can recover x and y's types
        // for strict-typing binop dispatch (`x + y`). The runtime schema
        // collapses per-variant struct fields into `__payload_N: Any`, so
        // we keep the named-field annotations on the side. Register
        // under both the qualified name (e.g. `m::E`) and the bare name
        // to mirror the schema-registration aliasing below.
        for member in &enum_def.members {
            if let EnumMemberKind::Struct(fields) = &member.kind {
                let kv: Vec<(String, shape_ast::ast::TypeAnnotation)> = fields
                    .iter()
                    .map(|f| (f.name.clone(), f.type_annotation.clone()))
                    .collect();
                self.enum_struct_variant_fields
                    .insert((enum_def.name.clone(), member.name.clone()), kv.clone());
                if let Some(basename) = enum_def.name.rsplit("::").next() {
                    if basename != enum_def.name {
                        self.enum_struct_variant_fields
                            .insert((basename.to_string(), member.name.clone()), kv);
                    }
                }
            }
            // R8 W7: parallel cache for tuple variants so positional
            // payload types are recoverable at pattern-compile time
            // (the schema collapses them into `__payload_N: Any`).
            if let EnumMemberKind::Tuple(types) = &member.kind {
                let tv = types.clone();
                self.enum_tuple_variant_fields
                    .insert((enum_def.name.clone(), member.name.clone()), tv.clone());
                if let Some(basename) = enum_def.name.rsplit("::").next() {
                    if basename != enum_def.name {
                        self.enum_tuple_variant_fields
                            .insert((basename.to_string(), member.name.clone()), tv);
                    }
                }
            }
        }

        let schema =
            shape_runtime::type_schema::TypeSchema::new_enum(&enum_def.name, variants.clone());
        self.type_tracker.schema_registry_mut().register(schema);

        // Also register under bare name if the qualified name contains "::"
        // so runtime code that uses bare enum names (e.g., "Snapshot") can find the schema.
        if let Some(basename) = enum_def.name.rsplit("::").next() {
            if basename != enum_def.name
                && self.type_tracker.schema_registry().get(basename).is_none()
            {
                let alias_schema =
                    shape_runtime::type_schema::TypeSchema::new_enum(basename, variants);
                self.type_tracker
                    .schema_registry_mut()
                    .register(alias_schema);
            }
        }
        Ok(())
    }

    /// Pre-register items from an imported module (enums, struct types, functions).
    ///
    /// Called by the LSP before compilation to make imported enums/types known
    /// to the compiler's type tracker. Reuses `register_enum` as single source of truth.
    pub fn register_imported_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Export(export, _) => {
                    match &export.item {
                        ExportItem::Enum(enum_def) => {
                            let _ = self.register_enum(enum_def);
                        }
                        ExportItem::Struct(struct_def) => {
                            // Register struct type fields so the compiler knows about them
                            let _ = self.register_struct_type(struct_def, Span::DUMMY);
                        }
                        ExportItem::Function(func_def) => {
                            // Register function so it's known during compilation
                            let _ = self.register_function(func_def);
                        }
                        _ => {}
                    }
                }
                Item::Enum(enum_def, _) => {
                    let _ = self.register_enum(enum_def);
                }
                _ => {}
            }
        }
    }

    /// Register a meta definition in the format registry
    ///
    // Meta compilation methods removed — formatting now uses Display trait

    /// Desugar an extend method to a FunctionDef with implicit `self` first param.
    ///
    /// `extend Number { method double() { self * 2 } }`
    /// becomes: `function double(self) { self * 2 }`
    ///
    /// UFCS handles the rest: `(5).double()` → `double(5)` → self = 5
    pub(super) fn desugar_extend_method(
        &self,
        method: &shape_ast::ast::types::MethodDef,
        target_type: &shape_ast::ast::TypeName,
    ) -> Result<FunctionDef> {
        let receiver_type = Some(Self::type_name_to_annotation(target_type));
        let (params, body) = self.desugar_method_signature_and_body(method, receiver_type)?;

        // Extend methods use qualified "Type.method" names to avoid collisions
        // with free functions (e.g., prelude's `sum` vs extend Point { method sum() }).
        let type_str = match target_type {
            shape_ast::ast::TypeName::Simple(n) => n.clone(),
            shape_ast::ast::TypeName::Generic { name, .. } => name.clone(),
        };

        // Propagate type params from the extend block's generic target type.
        // `extend Vec<T> { method indexOf(value: T) { ... } }` produces a
        // FunctionDef with type_params = [T]. This enables monomorphization
        // at call sites (e.g., `[1,2,3].indexOf(2)` → T=int).
        //
        // Heuristic: a type arg is a type PARAMETER (not a concrete type) if
        // it's a Basic annotation whose name is a single uppercase letter
        // (the standard convention for type variables: T, U, K, V).
        let extend_type_params: Vec<shape_ast::ast::TypeParam> = match target_type {
            shape_ast::ast::TypeName::Generic { type_args, .. } => type_args
                .iter()
                .filter_map(|ta| match ta {
                    shape_ast::ast::TypeAnnotation::Basic(name)
                        if name.len() == 1
                            && name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) =>
                    {
                        Some(shape_ast::ast::TypeParam::Type {
                            name: name.clone(),
                            span: Span::DUMMY,
                            doc_comment: None,
                            default_type: None,
                            trait_bounds: Vec::new(),
                        })
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        // V3-S6a resolver-extension follow-up: merge method-level type
        // params (`method map<U>(...)` — the `<U>`) with the extend-block
        // type params (`extend Vec<T> { ... }` — the `<T>`). The previous
        // shape silently dropped method-level generics, leaving the
        // monomorphizer unable to resolve generics like `U` in
        // `Vec.map<U>(f: (T) => U) -> Vec<U>` and forcing the call site
        // to the generic-template (un-monomorphized) path. The Smoke 2
        // regression `[1,2,3,4,5].map(|x|x*2).sum()` surfaced at the
        // empty-array `let mut result = []` in the un-specialized
        // Vec.map body.
        //
        // Order: extend params first (the receiver-positional generic
        // `T` is conceptually the outer generic), then method-level
        // params (`U` is inner / nested). The substitution pass walks
        // bindings by name, so positional order matters only for the
        // `mono_key`'s stable ordering — extend-first matches the
        // user-visible declaration order `Vec<T>.map<U>`.
        let mut merged_type_params: Vec<shape_ast::ast::TypeParam> = extend_type_params;
        if let Some(method_tps) = method.type_params.as_ref() {
            for tp in method_tps {
                // Skip duplicates (defensive — if a method redeclares a
                // type param of the extend block, prefer the outer).
                let name = tp.name();
                if !merged_type_params.iter().any(|m| m.name() == name) {
                    merged_type_params.push(tp.clone());
                }
            }
        }

        Ok(FunctionDef {
            name: format!("{}.{}", type_str, method.name),
            name_span: Span::DUMMY,
            declaring_module_path: method.declaring_module_path.clone(),
            doc_comment: None,
            params,
            return_type: method.return_type.clone(),
            body,
            type_params: Some(merged_type_params),
            annotations: method.annotations.clone(),
            is_async: method.is_async,
            is_comptime: false,
            where_clause: None,
        })
    }

    /// Desugar an impl method to a scoped FunctionDef.
    ///
    /// - Default impl:
    ///   `impl Queryable for DbTable { method filter(pred) { ... } }`
    ///   becomes: `function DbTable::filter(self, pred) { ... }`
    /// - Named impl:
    ///   `impl Display for User as JsonDisplay { method display() { ... } }`
    ///   becomes: `function Display::User::JsonDisplay::display(self) { ... }`
    ///
    /// Named impls use trait/type/impl prefixes to avoid collisions.
    ///
    /// # Trait declaration return-type substitution (Phase 3 cluster-0 Round 13 T1', gap 3)
    ///
    /// When the impl source omits the return-type annotation
    /// (`impl T for X { method name() { "x" } }` — the impl doesn't repeat
    /// the trait's `: string`), `method.return_type` is `None`. Without
    /// substitution, the synthesized `FunctionDef.return_type` is also
    /// `None` and the Round 6A `function_return_concrete_types[X::name]`
    /// side-table holds `ConcreteType::Void`, leaving the JIT MIR
    /// conduit's destination-stamp pass unable to classify
    /// `t.name() → NativeKind::String` (Smoke 3 surface, T1 close
    /// `76b01cf8`).
    ///
    /// Closing this surface: when the impl's `return_type` is `None`, look
    /// up the trait's declared return type for the matching method via
    /// `self.trait_defs` (with `resolve_trait_name`-shaped lookup since
    /// `trait_name` is the basename) and substitute it into the
    /// synthesized `FunctionDef.return_type`. The 6A populator at
    /// `compile_post_assembly` (`compiler_impl_reference_model.rs:1474`)
    /// then runs `concrete_type_from_annotation` on the substituted
    /// annotation, populating `function_return_concrete_types[fn_idx]`
    /// with the trait's declared `ConcreteType` (`String` for Smoke 3).
    ///
    /// This is in-compiler source-side completion of contract information
    /// that already exists in source code — trait declarations carry the
    /// return type, the compiler just wasn't propagating it.
    fn desugar_impl_method(
        &self,
        method: &shape_ast::ast::types::MethodDef,
        trait_name: &str,
        type_name: &str,
        impl_name: Option<&str>,
        target_type: &shape_ast::ast::TypeName,
    ) -> Result<FunctionDef> {
        // When the target type is a known generic container (Array, HashMap, etc.),
        // synthesize type parameters (T, K, V) and enrich the receiver annotation
        // to `Array<T>` (etc.). This enables the monomorphization pipeline to
        // resolve T from the receiver's concrete element type at call sites,
        // producing specialized functions that use typed opcodes.
        //
        // All methods in `impl Trait for Array` benefit from this because their
        // body operates on `self` (the generic receiver) — even methods with no
        // explicit parameters like `flatten()`.
        let (impl_type_params, receiver_type) = Self::synthesize_impl_type_params(target_type);

        let (params, body) = self.desugar_method_signature_and_body(method, receiver_type)?;

        // Async drop methods are named "drop_async" so both sync and async
        // variants can coexist in the function name index.
        let method_name = if trait_name == "Drop" && method.name == "drop" && method.is_async {
            "drop_async".to_string()
        } else {
            method.name.clone()
        };
        let fn_name = if let Some(name) = impl_name {
            format!("{}::{}::{}::{}", trait_name, type_name, name, method_name)
        } else {
            format!("{}::{}", type_name, method_name)
        };

        // ADR-006 §2.7.5 — Phase 3 cluster-0 Round 13 T1' gap 3 closure.
        //
        // If the impl method omits its return type, look up the trait
        // declaration and substitute the trait's declared return type.
        // The trait's required-method signature (`TraitMemberSignature::Method
        // { return_type, .. }`) is the contract; if the impl chose not
        // to repeat it, the contract still applies.
        //
        // `resolve_trait_name` returns the canonical key into `self.trait_defs`;
        // we accept the bare `trait_name` here (callers pass `trait_basename`)
        // and walk the resolver. When the trait isn't registered (e.g. built-in
        // traits without an entry in `self.trait_defs`), the original `None`
        // is preserved per §2.7.7 #9 — no fabricated default.
        let return_type = method.return_type.clone().or_else(|| {
            let (canonical_trait, _) = self.resolve_trait_name(trait_name);
            self.trait_defs.get(&canonical_trait).and_then(|trait_def| {
                // Match on method name. Both Required and Default trait
                // members carry the return type — Required via
                // `TraitMemberSignature::Method { return_type, .. }` (always
                // present), Default via `MethodDef.return_type:
                // Option<TypeAnnotation>` (may itself be None — in which
                // case there's nothing to backfill).
                for member in &trait_def.members {
                    match member {
                        shape_ast::ast::types::TraitMember::Required(
                            shape_ast::ast::TraitMemberSignature::Method {
                                name, return_type, ..
                            },
                        ) if name == &method.name => {
                            return Some(return_type.clone());
                        }
                        shape_ast::ast::types::TraitMember::Default(default_method)
                            if default_method.name == method.name =>
                        {
                            return default_method.return_type.clone();
                        }
                        _ => {}
                    }
                }
                None
            })
        });

        // V3-S6a resolver-extension follow-up: merge method-level type
        // params with impl-block type params (mirrors the parallel fix in
        // `desugar_extend_method`). See that function for rationale.
        let mut merged_impl_type_params: Vec<shape_ast::ast::TypeParam> = impl_type_params;
        if let Some(method_tps) = method.type_params.as_ref() {
            for tp in method_tps {
                let name = tp.name();
                if !merged_impl_type_params.iter().any(|m| m.name() == name) {
                    merged_impl_type_params.push(tp.clone());
                }
            }
        }

        Ok(FunctionDef {
            name: fn_name,
            name_span: Span::DUMMY,
            declaring_module_path: method.declaring_module_path.clone(),
            doc_comment: None,
            params,
            return_type,
            body,
            type_params: Some(merged_impl_type_params),
            annotations: method.annotations.clone(),
            is_async: method.is_async,
            is_comptime: false,
            where_clause: None,
        })
    }

    /// Synthesize type parameters and a receiver annotation for impl methods
    /// on known generic container types.
    ///
    /// For `impl Iterable for Array`, the target type `Simple("Array")` becomes
    /// receiver `Array<T>` with a synthetic type param `T`. This mirrors how
    /// `extend Vec<T>` propagates type params, enabling monomorphization at
    /// call sites (e.g. `[1,2,3].findIndex(...)` → T=int).
    ///
    /// Returns `(type_params, receiver_annotation)`.
    fn synthesize_impl_type_params(
        target_type: &shape_ast::ast::TypeName,
    ) -> (
        Vec<shape_ast::ast::TypeParam>,
        Option<shape_ast::ast::TypeAnnotation>,
    ) {
        let type_base = match target_type {
            shape_ast::ast::TypeName::Simple(n) => n.as_str(),
            shape_ast::ast::TypeName::Generic { name, .. } => name.as_str(),
        };

        // Known single-element generic containers: Array/Vec → T
        let is_single_param_generic = matches!(type_base, "Array" | "Vec");
        // Known dual-element generic containers: HashMap/Map → K, V
        let is_dual_param_generic = matches!(type_base, "HashMap" | "Map");

        if is_single_param_generic {
            let type_params = vec![shape_ast::ast::TypeParam::Type {
                name: "T".to_string(),
                span: Span::DUMMY,
                doc_comment: None,
                default_type: None,
                trait_bounds: Vec::new(),
            }];
            let receiver_ann = shape_ast::ast::TypeAnnotation::Generic {
                name: shape_ast::ast::type_path::TypePath::simple(type_base),
                args: vec![shape_ast::ast::TypeAnnotation::Basic("T".to_string())],
            };
            (type_params, Some(receiver_ann))
        } else if is_dual_param_generic {
            let type_params = vec![
                shape_ast::ast::TypeParam::Type {
                    name: "K".to_string(),
                    span: Span::DUMMY,
                    doc_comment: None,
                    default_type: None,
                    trait_bounds: Vec::new(),
                },
                shape_ast::ast::TypeParam::Type {
                    name: "V".to_string(),
                    span: Span::DUMMY,
                    doc_comment: None,
                    default_type: None,
                    trait_bounds: Vec::new(),
                },
            ];
            let receiver_ann = shape_ast::ast::TypeAnnotation::Generic {
                name: shape_ast::ast::type_path::TypePath::simple(type_base),
                args: vec![
                    shape_ast::ast::TypeAnnotation::Basic("K".to_string()),
                    shape_ast::ast::TypeAnnotation::Basic("V".to_string()),
                ],
            };
            (type_params, Some(receiver_ann))
        } else {
            // Unknown type — no synthetic params, plain receiver.
            (Vec::new(), Some(Self::type_name_to_annotation(target_type)))
        }
    }

    /// Build desugared method params/body with implicit receiver handling.
    ///
    /// Canonical receiver is `self`.
    fn desugar_method_signature_and_body(
        &self,
        method: &shape_ast::ast::types::MethodDef,
        receiver_type: Option<shape_ast::ast::TypeAnnotation>,
    ) -> Result<(Vec<FunctionParameter>, Vec<Statement>)> {
        if let Some(receiver) = method
            .params
            .first()
            .and_then(|p| p.pattern.as_identifier())
        {
            if receiver == "self" {
                let location = method
                    .params
                    .first()
                    .map(|p| self.span_to_source_location(p.span()));
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Method '{}' has an explicit `self` parameter, but method receivers are implicit. Use `method {}(...)` without `self`.",
                        method.name, method.name
                    ),
                    location,
                });
            }
        }

        let mut params = vec![FunctionParameter {
            pattern: shape_ast::ast::DestructurePattern::Identifier(
                "self".to_string(),
                Span::DUMMY,
            ),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: receiver_type,
            default_value: None,
        }];
        params.extend(method.params.clone());

        Ok((params, method.body.clone()))
    }

    /// Compile a `From` or `TryFrom` impl block.
    ///
    /// Unlike normal impl methods (which inject implicit `self`), From/TryFrom
    /// methods are constructors: `from(value: Source) -> Target`. The value
    /// parameter sits at local slot 0 with no receiver.
    ///
    /// Auto-derives:
    /// - `impl From<S> for T`  → `Into<T>::into` on S (direct alias)
    ///                          + `TryInto<T>::tryInto` on S (wrapper → Ok())
    /// - `impl TryFrom<S> for T` → `TryInto<T>::tryInto` on S (direct alias)
    fn compile_from_impl(
        &mut self,
        impl_block: &shape_ast::ast::types::ImplBlock,
        trait_name: &str,
        target_type: &str,
    ) -> Result<()> {
        // Extract source type from generic args: From<Source> → Source
        let source_type = match &impl_block.trait_name {
            shape_ast::ast::types::TypeName::Generic { type_args, .. } if !type_args.is_empty() => {
                match &type_args[0] {
                    TypeAnnotation::Basic(name) => name.clone(),
                    TypeAnnotation::Reference(name) => name.to_string(),
                    other => {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "{} impl requires a simple source type, found {:?}",
                                trait_name, other
                            ),
                            location: None,
                        });
                    }
                }
            }
            _ => {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "{} impl requires a generic type argument, e.g., {}<string>",
                        trait_name, trait_name
                    ),
                    location: None,
                });
            }
        };

        // Named impl selector defaults to the target type name so that
        // `as TargetType` / `as TargetType?` dispatch finds the right symbol.
        let selector = impl_block.impl_name.as_deref().unwrap_or(target_type);

        for method in &impl_block.methods {
            let func_def =
                self.desugar_from_method(method, trait_name, target_type, &source_type)?;
            let from_fn_name = func_def.name.clone();

            // Register From/TryFrom trait method symbol on the target type
            self.program.register_trait_method_symbol(
                trait_name,
                target_type,
                Some(&source_type),
                &method.name,
                &from_fn_name,
            );
            self.register_function(&func_def)?;

            // Auto-derive Into/TryInto on the source type
            if trait_name == "From" {
                // From<S> for T → Into<T>::into on S = direct alias (same fn)
                self.program.register_trait_method_symbol(
                    "Into",
                    &source_type,
                    Some(selector),
                    "into",
                    &from_fn_name,
                );

                // From<S> for T → TryInto<T>::tryInto on S = wrapper (from + Ok)
                let wrapper_name =
                    self.emit_from_to_tryinto_wrapper(&from_fn_name, &source_type, target_type)?;
                self.program.register_trait_method_symbol(
                    "TryInto",
                    &source_type,
                    Some(selector),
                    "tryInto",
                    &wrapper_name,
                );

                // Register trait impls in type inference environment
                let _ = self.type_inference.env.register_trait_impl_named(
                    "Into",
                    &source_type,
                    selector,
                    vec!["into".to_string()],
                );
                let _ = self.type_inference.env.register_trait_impl_named(
                    "TryInto",
                    &source_type,
                    selector,
                    vec!["tryInto".to_string()],
                );
            } else {
                // TryFrom<S> for T → TryInto<T>::tryInto on S = direct alias
                self.program.register_trait_method_symbol(
                    "TryInto",
                    &source_type,
                    Some(selector),
                    "tryInto",
                    &from_fn_name,
                );

                // Register TryInto trait impl in type inference environment
                let _ = self.type_inference.env.register_trait_impl_named(
                    "TryInto",
                    &source_type,
                    selector,
                    vec!["tryInto".to_string()],
                );
            }
        }

        // Register From/TryFrom trait impl on target type
        let all_method_names: Vec<String> =
            impl_block.methods.iter().map(|m| m.name.clone()).collect();
        let _ = self.type_inference.env.register_trait_impl_named(
            trait_name,
            target_type,
            &source_type,
            all_method_names,
        );

        Ok(())
    }

    /// Compile From/TryFrom impl method bodies (and the synthetic TryInto wrapper).
    ///
    /// Called from `compile_item_with_context` — the registration pass already
    /// happened in `compile_from_impl` / `register_item_functions`.
    fn compile_from_impl_bodies(
        &mut self,
        impl_block: &shape_ast::ast::types::ImplBlock,
        trait_name: &str,
        target_type: &str,
    ) -> Result<()> {
        let source_type = match &impl_block.trait_name {
            shape_ast::ast::types::TypeName::Generic { type_args, .. } if !type_args.is_empty() => {
                match &type_args[0] {
                    TypeAnnotation::Basic(name) => name.clone(),
                    TypeAnnotation::Reference(name) => name.to_string(),
                    _ => return Ok(()), // error already reported in registration
                }
            }
            _ => return Ok(()),
        };

        for method in &impl_block.methods {
            let func_def =
                self.desugar_from_method(method, trait_name, target_type, &source_type)?;
            self.compile_function(&func_def)?;
        }

        // Also compile the synthetic TryInto wrapper for From impls
        if trait_name == "From" {
            for method in &impl_block.methods {
                let from_fn_name = format!(
                    "{}::{}::{}::{}",
                    trait_name, target_type, source_type, method.name
                );
                let wrapper_name = format!("__from_tryinto_{}_{}", source_type, target_type);
                // The wrapper was already registered; now compile its body
                if let Some(func_def) = self.function_defs.get(&wrapper_name).cloned() {
                    let _ = self.compile_function(&func_def);
                    // Suppress errors: if Ok() or the from fn is not yet available, it
                    // will be resolved at link time.
                    let _ = from_fn_name; // used above in the format
                }
            }
        }

        Ok(())
    }

    /// Desugar a From/TryFrom method WITHOUT implicit self injection.
    ///
    /// `From::from(value: S)` is a constructor — `value` sits at local slot 0.
    /// Function name: `"From::TargetType::SourceType::method_name"`
    fn desugar_from_method(
        &self,
        method: &shape_ast::ast::types::MethodDef,
        trait_name: &str,
        target_type: &str,
        source_type: &str,
    ) -> Result<FunctionDef> {
        // Verify no explicit `self` parameter
        if let Some(first) = method
            .params
            .first()
            .and_then(|p| p.pattern.as_identifier())
        {
            if first == "self" {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "{}::{} methods are constructors and must not have a `self` parameter",
                        trait_name, method.name
                    ),
                    location: None,
                });
            }
        }

        let fn_name = format!(
            "{}::{}::{}::{}",
            trait_name, target_type, source_type, method.name
        );

        Ok(FunctionDef {
            name: fn_name,
            name_span: Span::DUMMY,
            declaring_module_path: method.declaring_module_path.clone(),
            doc_comment: None,
            params: method.params.clone(),
            return_type: method.return_type.clone(),
            body: method.body.clone(),
            type_params: Some(Vec::new()),
            annotations: Vec::new(),
            is_async: method.is_async,
            is_comptime: false,
            where_clause: None,
        })
    }

    /// Emit a synthetic wrapper function that calls a From::from function
    /// and wraps its result in Ok() for TryInto compatibility.
    ///
    /// Generated function: `__from_tryinto_{source}_{target}(value) -> Ok(from(value))`
    fn emit_from_to_tryinto_wrapper(
        &mut self,
        from_fn_name: &str,
        source_type: &str,
        target_type: &str,
    ) -> Result<String> {
        let wrapper_name = format!("__from_tryinto_{}_{}", source_type, target_type);

        // Create a synthetic FunctionDef whose body calls from() and wraps in Ok()
        let span = Span::DUMMY;
        let body = vec![Statement::Return(
            Some(Expr::FunctionCall {
                name: "Ok".to_string(),
                args: vec![Expr::FunctionCall {
                    name: from_fn_name.to_string(),
                    args: vec![Expr::Identifier("value".to_string(), span)],
                    named_args: Vec::new(),
                    span,
                }],
                named_args: Vec::new(),
                span,
            }),
            span,
        )];

        let func_def = FunctionDef {
            name: wrapper_name.clone(),
            name_span: span,
            declaring_module_path: None,
            doc_comment: None,
            params: vec![FunctionParameter {
                pattern: DestructurePattern::Identifier("value".to_string(), span),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: None,
                default_value: None,
            }],
            return_type: None,
            body,
            type_params: Some(Vec::new()),
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
            where_clause: None,
        };

        self.register_function(&func_def)?;

        Ok(wrapper_name)
    }

    fn type_name_to_annotation(
        type_name: &shape_ast::ast::TypeName,
    ) -> shape_ast::ast::TypeAnnotation {
        match type_name {
            shape_ast::ast::TypeName::Simple(name) => {
                shape_ast::ast::TypeAnnotation::Basic(name.to_string())
            }
            shape_ast::ast::TypeName::Generic { name, type_args } => {
                shape_ast::ast::TypeAnnotation::Generic {
                    name: name.clone(),
                    args: type_args.clone(),
                }
            }
        }
    }

    /// Compile an annotation definition.
    ///
    /// Each handler is compiled as an internal function:
    /// - before(args, ctx) → `{name}___before(self, period, args, ctx)`
    /// - after(args, result, ctx) → `{name}___after(self, period, args, result, ctx)`
    ///
    /// `self` is the annotated item (function/method/property).
    /// Annotation params (e.g., `period`) are prepended after `self`.
    fn compile_annotation_def(&mut self, ann_def: &shape_ast::ast::AnnotationDef) -> Result<()> {
        use crate::bytecode::CompiledAnnotation;
        use shape_ast::ast::AnnotationHandlerType;

        let mut compiled = CompiledAnnotation {
            name: ann_def.name.clone(),
            param_names: ann_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect(),
            param_defs: ann_def.params.clone(),
            before_handler: None,
            after_handler: None,
            on_define_handler: None,
            metadata_handler: None,
            comptime_pre_handler: None,
            comptime_post_handler: None,
            before_handler_template: None,
            after_handler_template: None,
            allowed_targets: Vec::new(),
        };

        for handler in &ann_def.handlers {
            // Comptime handlers are stored as AST (not compiled to bytecode).
            // They are executed at compile time when the annotation is applied.
            match handler.handler_type {
                AnnotationHandlerType::ComptimePre => {
                    compiled.comptime_pre_handler = Some(handler.clone());
                    continue;
                }
                AnnotationHandlerType::ComptimePost => {
                    compiled.comptime_post_handler = Some(handler.clone());
                    continue;
                }
                _ => {}
            }

            if handler.params.iter().any(|p| p.is_variadic) {
                return Err(ShapeError::SemanticError {
                    message:
                        "Variadic annotation handler params (`...args`) are only supported on comptime handlers"
                            .to_string(),
                    location: Some(self.span_to_source_location(handler.span)),
                });
            }

            let handler_type_str = match handler.handler_type {
                AnnotationHandlerType::Before => "before",
                AnnotationHandlerType::After => "after",
                AnnotationHandlerType::OnDefine => "on_define",
                AnnotationHandlerType::Metadata => "metadata",
                AnnotationHandlerType::ComptimePre => unreachable!(),
                AnnotationHandlerType::ComptimePost => unreachable!(),
            };

            let func_name = format!("{}___{}", ann_def.name, handler_type_str);

            if matches!(
                handler.handler_type,
                AnnotationHandlerType::Before | AnnotationHandlerType::After
            ) {
                let placeholder = FunctionDef {
                    name: func_name.clone(),
                    name_span: Span::DUMMY,
                    declaring_module_path: None,
                    doc_comment: None,
                    params: Vec::new(),
                    return_type: handler.return_type.clone(),
                    body: Vec::new(),
                    type_params: Some(Vec::new()),
                    annotations: Vec::new(),
                    is_async: false,
                    is_comptime: false,
                    where_clause: None,
                };
                self.register_function(&placeholder)?;
                let func_id = self.find_function(&func_name).ok_or_else(|| {
                    ShapeError::RuntimeError {
                        message: format!(
                            "Internal error: annotation handler function '{}' was not registered",
                            func_name
                        ),
                        location: None,
                    }
                })? as u16;
                match handler.handler_type {
                    AnnotationHandlerType::Before => {
                        compiled.before_handler = Some(func_id);
                        compiled.before_handler_template = Some(handler.clone());
                    }
                    AnnotationHandlerType::After => {
                        compiled.after_handler = Some(func_id);
                        compiled.after_handler_template = Some(handler.clone());
                    }
                    _ => unreachable!(),
                }
                continue;
            }

            // Build function params: self + annotation_params + handler_params
            let mut params = vec![FunctionParameter {
                pattern: shape_ast::ast::DestructurePattern::Identifier(
                    "self".to_string(),
                    Span::DUMMY,
                ),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: None,
                default_value: None,
            }];
            // Add annotation params (e.g., period)
            for ann_param in &ann_def.params {
                params.push(ann_param.clone());
            }
            // Add handler params (e.g., args, ctx)
            for param in &handler.params {
                let inferred_type = if param.name == "ctx" {
                    Some(TypeAnnotation::Object(vec![
                        shape_ast::ast::ObjectTypeField {
                            name: "state".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("unknown".to_string()),
                            annotations: vec![],
                        },
                        shape_ast::ast::ObjectTypeField {
                            name: "event_log".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Array(Box::new(
                                TypeAnnotation::Basic("unknown".to_string()),
                            )),
                            annotations: vec![],
                        },
                    ]))
                } else if matches!(
                    handler.handler_type,
                    AnnotationHandlerType::OnDefine | AnnotationHandlerType::Metadata
                ) && (param.name == "fn" || param.name == "target")
                {
                    Some(TypeAnnotation::Object(vec![
                        shape_ast::ast::ObjectTypeField {
                            name: "name".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("string".to_string()),
                            annotations: vec![],
                        },
                        shape_ast::ast::ObjectTypeField {
                            name: "kind".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("string".to_string()),
                            annotations: vec![],
                        },
                        shape_ast::ast::ObjectTypeField {
                            name: "id".to_string(),
                            optional: false,
                            type_annotation: TypeAnnotation::Basic("int".to_string()),
                            annotations: vec![],
                        },
                    ]))
                } else {
                    None
                };

                params.push(FunctionParameter {
                    pattern: shape_ast::ast::DestructurePattern::Identifier(
                        param.name.clone(),
                        Span::DUMMY,
                    ),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    type_annotation: inferred_type,
                    default_value: None,
                });
            }

            // Convert handler body (Expr) to function body (Vec<Statement>)
            let body = vec![Statement::Return(Some(handler.body.clone()), Span::DUMMY)];

            let func_def = FunctionDef {
                name: func_name,
                name_span: Span::DUMMY,
                declaring_module_path: None,
                doc_comment: None,
                params,
                return_type: handler.return_type.clone(),
                body,
                type_params: Some(Vec::new()),
                annotations: Vec::new(),
                is_async: false,
                is_comptime: false,
                where_clause: None,
            };

            self.register_function(&func_def)?;
            self.compile_function(&func_def)?;

            // Capture frame descriptor for the annotation handler function.
            // Mirrors functions.rs:1750 — required for v2 typed opcode verification
            // (ADR-006 §2.7.5.1) and JIT deoptimization map construction. Without
            // this, v2 typed opcodes emitted in the handler body (e.g. NewTypedArrayI64,
            // TypedArrayPushI64 from CoW array mutations) fail verification and the
            // JIT SEGFAULTs on deopt paths.
            let func_idx = self.program.functions.len() - 1;
            self.program.functions[func_idx].locals_count = self.next_local;
            self.capture_function_local_storage_hints(func_idx);

            let func_id = (self.program.functions.len() - 1) as u16;

            match handler.handler_type {
                AnnotationHandlerType::Before => compiled.before_handler = Some(func_id),
                AnnotationHandlerType::After => compiled.after_handler = Some(func_id),
                AnnotationHandlerType::OnDefine => compiled.on_define_handler = Some(func_id),
                AnnotationHandlerType::Metadata => compiled.metadata_handler = Some(func_id),
                AnnotationHandlerType::ComptimePre => {} // handled above
                AnnotationHandlerType::ComptimePost => {} // handled above
            }
        }

        // Resolve allowed target kinds.
        // Explicit `targets: [...]` in the annotation definition has priority.
        // Otherwise infer from handlers:
        // before/after handlers only make sense on functions (they wrap calls),
        // lifecycle handlers (on_define/metadata) are definition-time only.
        if let Some(explicit) = &ann_def.allowed_targets {
            compiled.allowed_targets = explicit.clone();
        } else if compiled.before_handler.is_some()
            || compiled.after_handler.is_some()
            || compiled.comptime_pre_handler.is_some()
            || compiled.comptime_post_handler.is_some()
        {
            compiled.allowed_targets =
                vec![shape_ast::ast::functions::AnnotationTargetKind::Function];
        } else if compiled.on_define_handler.is_some() || compiled.metadata_handler.is_some() {
            compiled.allowed_targets = vec![
                shape_ast::ast::functions::AnnotationTargetKind::Function,
                shape_ast::ast::functions::AnnotationTargetKind::Type,
                shape_ast::ast::functions::AnnotationTargetKind::Module,
            ];
        }

        // Enforce that definition-time lifecycle hooks only target definition
        // sites (`function` / `type`).
        if compiled.on_define_handler.is_some() || compiled.metadata_handler.is_some() {
            if compiled.allowed_targets.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Annotation '{}' uses `on_define`/`metadata` and cannot have unrestricted targets. Allowed targets are: function, type, module",
                        ann_def.name
                    ),
                    location: Some(self.span_to_source_location(ann_def.span)),
                });
            }
            if let Some(invalid) = compiled
                .allowed_targets
                .iter()
                .find(|kind| !Self::is_definition_annotation_target(**kind))
            {
                let invalid_label = format!("{:?}", invalid).to_lowercase();
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Annotation '{}' uses `on_define`/`metadata`, but target '{}' is not a definition target. Allowed targets are: function, type, module",
                        ann_def.name, invalid_label
                    ),
                    location: Some(self.span_to_source_location(ann_def.span)),
                });
            }
        }

        self.program
            .compiled_annotations
            .insert(ann_def.name.clone(), compiled);
        Ok(())
    }

    /// Register ONLY a struct's runtime `TypeSchema` into the compiler's
    /// schema registry — no comptime-handler execution, no annotation
    /// lifecycle, no native-layout / generic-info side effects.
    ///
    /// WS-9b: this is the single source of truth for "the struct's runtime
    /// fields, indexed by name, are known to the compiler". It is called
    /// from two places:
    ///
    /// * the pass-1 prepass (`predeclare_item_struct_schemas`), so a
    ///   struct's schema is available *before any function body compiles* —
    ///   making `type` definitions order-independent the same way function
    ///   definitions already are (pass-1 `register_item_functions`). Without
    ///   this, a function declared *before* the `type` it accepts as a
    ///   parameter could not resolve `param.field`: `tracker_schema_id_for_
    ///   expr` would miss the not-yet-registered schema, so `a.lo` in
    ///   `fn ov(a, b) { a.lo <= b.hi }` (with `type Box` declared after `ov`)
    ///   typed as `unknown` and the binop was spuriously rejected.
    ///
    /// * `register_struct_type` (pass 2), guarded by the same `is_none()`
    ///   check — when the prepass already registered the schema this is a
    ///   no-op, so pass 2 only runs the comptime handlers / lifecycle once.
    fn predeclare_struct_schema(&mut self, struct_def: &shape_ast::ast::StructTypeDef) {
        use shape_ast::ast::Literal;
        use shape_runtime::type_schema::FieldAnnotation;

        let runtime_field_names: Vec<String> = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| f.name.clone())
            .collect();
        let runtime_field_types = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| (f.name.clone(), f.type_annotation.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        self.struct_types
            .entry(struct_def.name.clone())
            .or_insert_with(|| (runtime_field_names, Span::DUMMY));
        self.struct_generic_info
            .entry(struct_def.name.clone())
            .or_insert_with(|| StructGenericInfo {
                type_params: struct_def.type_params.clone().unwrap_or_default(),
                runtime_field_types,
            });

        if self
            .type_tracker
            .schema_registry()
            .get(&struct_def.name)
            .is_some()
        {
            return;
        }
        let runtime_fields: Vec<(String, shape_runtime::type_schema::FieldType)> = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| {
                (
                    f.name.clone(),
                    Self::type_annotation_to_field_type(&f.type_annotation),
                )
            })
            .collect();
        // Collect field annotations (e.g. @alias) so that JSON
        // deserialization can map wire names to field names.
        let field_annotations: Vec<Vec<FieldAnnotation>> = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| {
                f.annotations
                    .iter()
                    .map(|ann| FieldAnnotation {
                        name: ann.name.clone(),
                        args: ann
                            .args
                            .iter()
                            .filter_map(|arg| match arg {
                                Expr::Literal(Literal::String(s), _) => Some(s.clone()),
                                _ => None,
                            })
                            .collect(),
                    })
                    .collect()
            })
            .collect();
        self.type_tracker
            .schema_registry_mut()
            .register_type_with_annotations(
                struct_def.name.clone(),
                runtime_fields,
                field_annotations,
            );
    }

    /// Register a struct type definition.
    ///
    /// Comptime fields are baked at compile time and excluded from the runtime TypeSchema.
    /// Their values are stored in `self.comptime_fields` for constant-folded access.
    fn register_struct_type(
        &mut self,
        struct_def: &shape_ast::ast::StructTypeDef,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        use shape_runtime::type_schema::{FieldAnnotation, TypeSchemaBuilder};

        // Validate annotation target kinds before type registration.
        for ann in &struct_def.annotations {
            self.validate_annotation_target_usage(
                ann,
                shape_ast::ast::functions::AnnotationTargetKind::Type,
                span,
            )?;
        }

        if struct_def.native_layout.is_some() {
            self.native_layout_types.insert(struct_def.name.clone());
        } else {
            self.native_layout_types.remove(&struct_def.name);
        }

        // Pre-register runtime field layout so comptime-generated methods on
        // `extend target { ... }` can resolve `self.field` statically.
        // If the target is later removed by comptime directives, these
        // placeholders are rolled back below.
        let runtime_field_names: Vec<String> = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| f.name.clone())
            .collect();
        let runtime_field_types = struct_def
            .fields
            .iter()
            .filter(|f| !f.is_comptime)
            .map(|f| (f.name.clone(), f.type_annotation.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        self.struct_types
            .insert(struct_def.name.clone(), (runtime_field_names, span));
        self.struct_generic_info.insert(
            struct_def.name.clone(),
            StructGenericInfo {
                type_params: struct_def.type_params.clone().unwrap_or_default(),
                runtime_field_types,
            },
        );
        // J-CT.2 (2026-05-23) — snapshot full struct AST for the comptime
        // mini-VM. `comptime_impl_blocks` referencing this type need the
        // original AST (field annotations, generic info, default values) to
        // compile struct-literal constructions + field access inside
        // `comptime { }` blocks. `struct_types` retains only field NAMES;
        // the mini-VM gets the full def via `comptime_context_struct_defs`.
        // Stored here in the canonical `register_struct_type` site so both
        // `Item::StructType` and `Item::Export(ExportItem::Struct)` paths
        // populate it uniformly.
        self.comptime_context_struct_defs
            .insert(struct_def.name.clone(), struct_def.clone());
        self.predeclare_struct_schema(struct_def);

        // Execute comptime annotation handlers before registration so
        // `remove target` can suppress type emission entirely.
        if self.execute_struct_comptime_handlers(struct_def)? {
            self.struct_types.remove(&struct_def.name);
            self.struct_generic_info.remove(&struct_def.name);
            self.comptime_context_struct_defs.remove(&struct_def.name);
            return Ok(());
        }

        if struct_def.native_layout.is_some() {
            self.register_native_struct_layout(struct_def, span)?;
        }

        // Build TypeSchema for runtime fields only
        if self
            .type_tracker
            .schema_registry()
            .get(&struct_def.name)
            .is_none()
        {
            let mut builder = TypeSchemaBuilder::new(struct_def.name.clone());
            for field in &struct_def.fields {
                if field.is_comptime {
                    continue;
                }
                let field_type = Self::type_annotation_to_field_type(&field.type_annotation);
                let mut annotations = Vec::new();
                for ann in &field.annotations {
                    let args: Vec<String> = ann
                        .args
                        .iter()
                        .filter_map(Self::eval_annotation_arg)
                        .collect();
                    annotations.push(FieldAnnotation {
                        name: ann.name.clone(),
                        args,
                    });
                }
                builder = builder.field_with_meta(field.name.clone(), field_type, annotations);
            }
            builder.register(self.type_tracker.schema_registry_mut());
        }

        // Bake comptime field values into the strict `KindedSlot` registry
        // for constant-folded property access. The field's declared type is
        // the producer-side kind oracle: `number = 2` adopts the integer
        // literal into a `Float64` slot, while incompatible defaults surface
        // here instead of reaching runtime as an inferred/probed value.
        for field in &struct_def.fields {
            if !field.is_comptime {
                continue;
            }
            if let Some(ref default_expr) = field.default_value {
                match default_expr {
                    Expr::Literal(literal @ Literal::Number(_), _)
                    | Expr::Literal(literal @ Literal::Int(_), _)
                    | Expr::Literal(literal @ Literal::UInt(_), _)
                    | Expr::Literal(literal @ Literal::String(_), _)
                    | Expr::Literal(literal @ Literal::Bool(_), _)
                    | Expr::Literal(literal @ Literal::None, _) => {
                        let field_type =
                            Self::type_annotation_to_field_type(&field.type_annotation);
                        let slot = Self::comptime_field_slot_from_literal(
                            &struct_def.name,
                            &field.name,
                            &field_type,
                            literal,
                        )?;
                        self.comptime_fields
                            .entry(struct_def.name.clone())
                            .or_default()
                            .insert(field.name.clone(), slot);
                    }
                    _ => {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Comptime field '{}' on type '{}' must have a literal default value",
                                field.name, struct_def.name
                            ),
                            location: None,
                        });
                    }
                }
            }
            // Comptime fields without a default are allowed — they must be
            // provided via type alias overrides (e.g., type EUR = Currency { symbol: "€" })
        }

        self.maybe_generate_native_type_conversions(&struct_def.name, span)?;

        Ok(())
    }

    fn register_native_struct_layout(
        &mut self,
        struct_def: &shape_ast::ast::StructTypeDef,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        if struct_def.type_params.is_some() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type C '{}' cannot be generic in this version",
                    struct_def.name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        if struct_def.fields.iter().any(|f| f.is_comptime) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type C '{}' cannot contain comptime fields",
                    struct_def.name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let abi = struct_def
            .native_layout
            .as_ref()
            .map(|b| b.abi.clone())
            .unwrap_or_else(|| "C".to_string());
        if abi != "C" {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type '{}' uses unsupported native ABI '{}'; only C is supported",
                    struct_def.name, abi
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let mut struct_align: u64 = 1;
        let mut offset: u64 = 0;
        let mut field_layouts = Vec::with_capacity(struct_def.fields.len());

        for field in &struct_def.fields {
            let field_spec =
                self.native_field_layout_spec(&field.type_annotation, span, &struct_def.name)?;
            struct_align = struct_align.max(field_spec.align);
            offset = Self::align_to(offset, field_spec.align);
            if offset > u32::MAX as u64
                || field_spec.size > u32::MAX as u64
                || field_spec.align > u32::MAX as u64
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "type C '{}' layout exceeds supported size/alignment limits",
                        struct_def.name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            field_layouts.push(crate::bytecode::NativeStructFieldLayout {
                name: field.name.clone(),
                c_type: field_spec.c_type,
                offset: offset as u32,
                size: field_spec.size as u32,
                align: field_spec.align as u32,
            });
            offset = offset.saturating_add(field_spec.size);
        }

        let size = Self::align_to(offset, struct_align);
        if size > u32::MAX as u64 || struct_align > u32::MAX as u64 {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "type C '{}' layout exceeds supported size/alignment limits",
                    struct_def.name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let entry = crate::bytecode::NativeStructLayoutEntry {
            name: struct_def.name.clone(),
            abi,
            size: size as u32,
            align: struct_align as u32,
            fields: field_layouts,
        };

        if let Some(existing) = self
            .program
            .native_struct_layouts
            .iter_mut()
            .find(|existing| existing.name == entry.name)
        {
            *existing = entry;
        } else {
            self.program.native_struct_layouts.push(entry);
        }

        Ok(())
    }

    fn align_to(value: u64, align: u64) -> u64 {
        debug_assert!(align > 0);
        let mask = align - 1;
        (value + mask) & !mask
    }

    fn native_field_layout_spec(
        &self,
        ann: &shape_ast::ast::TypeAnnotation,
        span: shape_ast::ast::Span,
        struct_name: &str,
    ) -> Result<NativeFieldLayoutSpec> {
        use shape_ast::ast::TypeAnnotation;

        let pointer = std::mem::size_of::<usize>() as u64;

        let fail = || -> Result<NativeFieldLayoutSpec> {
            Err(ShapeError::SemanticError {
                message: format!(
                    "unsupported type C field type '{}' in '{}'",
                    ann.to_type_string(),
                    struct_name
                ),
                location: Some(self.span_to_source_location(span)),
            })
        };

        if let Some(name) = ann.as_type_name_str() {
            if let Some(existing) = self
                .program
                .native_struct_layouts
                .iter()
                .find(|layout| layout.name == name)
            {
                return Ok(NativeFieldLayoutSpec {
                    c_type: name.to_string(),
                    size: existing.size as u64,
                    align: existing.align as u64,
                });
            }

            let spec = match name {
                "f64" | "number" | "Number" | "float" => ("f64", 8, 8),
                "f32" => ("f32", 4, 4),
                "i64" | "int" | "integer" | "Int" | "Integer" => ("i64", 8, 8),
                "i32" => ("i32", 4, 4),
                "i16" => ("i16", 2, 2),
                "i8" | "char" => ("i8", 1, 1),
                "u64" => ("u64", 8, 8),
                "u32" => ("u32", 4, 4),
                "u16" => ("u16", 2, 2),
                "u8" | "byte" => ("u8", 1, 1),
                "bool" | "boolean" => ("bool", 1, 1),
                "isize" => ("isize", pointer, pointer),
                "usize" | "ptr" | "pointer" => ("ptr", pointer, pointer),
                "string" | "str" | "cstring" => ("cstring", pointer, pointer),
                _ => return fail(),
            };
            return Ok(NativeFieldLayoutSpec {
                c_type: spec.0.to_string(),
                size: spec.1,
                align: spec.2,
            });
        }
        match ann {
            TypeAnnotation::Generic { name, args } if name == "Option" && args.len() == 1 => {
                let inner = self.native_field_layout_spec(&args[0], span, struct_name)?;
                if inner.c_type == "cstring" {
                    Ok(NativeFieldLayoutSpec {
                        c_type: "cstring?".to_string(),
                        size: pointer,
                        align: pointer,
                    })
                } else {
                    fail()
                }
            }
            _ => fail(),
        }
    }

    fn maybe_generate_native_type_conversions(
        &mut self,
        type_name: &str,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        let pair = if self.native_layout_types.contains(type_name) {
            let Some(object_type) = Self::object_type_name_for_native_layout(type_name) else {
                return Ok(());
            };
            if !self.struct_types.contains_key(&object_type)
                || self.native_layout_types.contains(&object_type)
            {
                return Ok(());
            }
            (type_name.to_string(), object_type)
        } else {
            let candidates: Vec<String> = Self::native_layout_name_candidates_for_object(type_name)
                .into_iter()
                .filter(|candidate| self.native_layout_types.contains(candidate))
                .collect();
            if candidates.is_empty() {
                return Ok(());
            }
            if candidates.len() > 1 {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "type '{}' matches multiple `type C` companions ({}) - use one canonical name",
                        type_name,
                        candidates.join(", ")
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            (candidates[0].clone(), type_name.to_string())
        };

        let pair_key = format!("{}::{}", pair.0, pair.1);
        if self.generated_native_conversion_pairs.contains(&pair_key) {
            return Ok(());
        }

        self.validate_native_conversion_pair(&pair.0, &pair.1, span)?;
        self.generate_native_conversion_direction(&pair.0, &pair.1, span)?;
        self.generate_native_conversion_direction(&pair.1, &pair.0, span)?;
        self.generated_native_conversion_pairs.insert(pair_key);
        Ok(())
    }

    fn object_type_name_for_native_layout(name: &str) -> Option<String> {
        if let Some(base) = name.strip_suffix("Layout")
            && !base.is_empty()
        {
            return Some(base.to_string());
        }
        if let Some(base) = name.strip_suffix('C')
            && !base.is_empty()
        {
            return Some(base.to_string());
        }
        if let Some(base) = name.strip_prefix('C')
            && !base.is_empty()
            && base
                .chars()
                .next()
                .map(|ch| ch.is_ascii_uppercase())
                .unwrap_or(false)
        {
            return Some(base.to_string());
        }
        None
    }

    fn native_layout_name_candidates_for_object(name: &str) -> Vec<String> {
        vec![
            format!("{}Layout", name),
            format!("{}C", name),
            format!("C{}", name),
        ]
    }

    fn validate_native_conversion_pair(
        &self,
        c_type: &str,
        object_type: &str,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        if !self.native_layout_types.contains(c_type) {
            return Err(ShapeError::SemanticError {
                message: format!("'{}' is not declared as `type C`", c_type),
                location: Some(self.span_to_source_location(span)),
            });
        }
        if self.native_layout_types.contains(object_type) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "auto conversion target '{}' cannot also be declared as `type C`",
                    object_type
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let c_type_info =
            self.struct_generic_info
                .get(c_type)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!("missing compiler metadata for `type C {}`", c_type),
                    location: Some(self.span_to_source_location(span)),
                })?;
        let object_type_info =
            self.struct_generic_info
                .get(object_type)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "missing compiler metadata for companion type '{}'",
                        object_type
                    ),
                    location: Some(self.span_to_source_location(span)),
                })?;

        if !c_type_info.type_params.is_empty() || !object_type_info.type_params.is_empty() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "auto `type C` conversions currently require non-generic types (`{}` <-> `{}`)",
                    c_type, object_type
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        let c_fields = self
            .struct_types
            .get(c_type)
            .map(|(fields, _)| fields)
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!("missing field metadata for `type C {}`", c_type),
                location: Some(self.span_to_source_location(span)),
            })?;
        let object_fields = self
            .struct_types
            .get(object_type)
            .map(|(fields, _)| fields)
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "missing field metadata for companion type '{}'",
                    object_type
                ),
                location: Some(self.span_to_source_location(span)),
            })?;

        let c_field_set: std::collections::HashSet<&str> =
            c_fields.iter().map(String::as_str).collect();
        let object_field_set: std::collections::HashSet<&str> =
            object_fields.iter().map(String::as_str).collect();
        if c_field_set != object_field_set {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "auto conversion pair '{}' <-> '{}' must have identical runtime fields",
                    c_type, object_type
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        for field_name in c_field_set {
            let c_ann = c_type_info
                .runtime_field_types
                .get(field_name)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "missing type metadata for field '{}.{}'",
                        c_type, field_name
                    ),
                    location: Some(self.span_to_source_location(span)),
                })?;
            let object_ann = object_type_info
                .runtime_field_types
                .get(field_name)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "missing type metadata for field '{}.{}'",
                        object_type, field_name
                    ),
                    location: Some(self.span_to_source_location(span)),
                })?;
            if c_ann != object_ann {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "field type mismatch for auto conversion '{}.{}' (`{}`) vs '{}.{}' (`{}`)",
                        c_type,
                        field_name,
                        c_ann.to_type_string(),
                        object_type,
                        field_name,
                        object_ann.to_type_string()
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
        }

        Ok(())
    }

    fn generate_native_conversion_direction(
        &mut self,
        source_type: &str,
        target_type: &str,
        span: shape_ast::ast::Span,
    ) -> Result<()> {
        let fn_name = format!(
            "__auto_native_from_{}_to_{}",
            Self::sanitize_auto_symbol(source_type),
            Self::sanitize_auto_symbol(target_type)
        );
        if self.function_defs.contains_key(&fn_name) && self.find_function(&fn_name).is_some() {
            return Ok(());
        }

        let target_fields = self
            .struct_types
            .get(target_type)
            .map(|(fields, _)| fields.clone())
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "missing target type metadata for auto conversion '{}'",
                    target_type
                ),
                location: Some(self.span_to_source_location(span)),
            })?;

        let source_expr = Expr::Identifier("value".to_string(), span);
        let struct_fields = target_fields
            .iter()
            .map(|field| {
                (
                    field.clone(),
                    Expr::PropertyAccess {
                        object: Box::new(source_expr.clone()),
                        property: field.clone(),
                        optional: false,
                        span,
                    },
                )
            })
            .collect::<Vec<_>>();
        let body = vec![Statement::Return(
            Some(Expr::StructLiteral {
                type_name: target_type.into(),
                fields: struct_fields,
                span,
            }),
            span,
        )];
        let fn_def = FunctionDef {
            name: fn_name.clone(),
            name_span: span,
            declaring_module_path: None,
            doc_comment: None,
            params: vec![FunctionParameter {
                pattern: DestructurePattern::Identifier("value".to_string(), span),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(TypeAnnotation::Reference(source_type.into())),
                default_value: None,
            }],
            return_type: Some(TypeAnnotation::Reference(target_type.into())),
            body,
            type_params: Some(Vec::new()),
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
            where_clause: None,
        };
        self.register_function(&fn_def)?;
        self.compile_function(&fn_def)?;

        self.program.register_trait_method_symbol(
            "From",
            target_type,
            Some(source_type),
            "from",
            &fn_name,
        );
        self.program.register_trait_method_symbol(
            "Into",
            source_type,
            Some(target_type),
            "into",
            &fn_name,
        );
        let _ = self.type_inference.env.register_trait_impl_named(
            "From",
            target_type,
            source_type,
            vec!["from".to_string()],
        );
        let _ = self.type_inference.env.register_trait_impl_named(
            "Into",
            source_type,
            target_type,
            vec!["into".to_string()],
        );
        Ok(())
    }

    fn sanitize_auto_symbol(name: &str) -> String {
        let mut out = String::with_capacity(name.len());
        for ch in name.chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
            } else {
                out.push('_');
            }
        }
        out
    }

    /// Execute comptime annotation handlers for a struct type definition.
    ///
    /// Mirrors `execute_comptime_handlers` in functions.rs but uses
    /// `ComptimeTarget::from_type()` to build the target from struct fields.
    fn execute_struct_comptime_handlers(
        &mut self,
        struct_def: &shape_ast::ast::StructTypeDef,
    ) -> Result<bool> {
        let mut removed = false;
        for ann in &struct_def.annotations {
            if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
                let handlers = [
                    compiled.comptime_pre_handler,
                    compiled.comptime_post_handler,
                ];
                for handler in handlers.into_iter().flatten() {
                    // Build field info for ComptimeTarget::from_type()
                    // Include per-field annotations so comptime handlers can inspect them.
                    let fields: Vec<(
                        String,
                        Option<shape_ast::ast::TypeAnnotation>,
                        Vec<shape_ast::ast::functions::Annotation>,
                    )> = struct_def
                        .fields
                        .iter()
                        .map(|f| {
                            (
                                f.name.clone(),
                                Some(f.type_annotation.clone()),
                                f.annotations.clone(),
                            )
                        })
                        .collect();

                    let target = super::comptime_target::ComptimeTarget::from_type(
                        &struct_def.name,
                        &fields,
                    );
                    // R8 W9 G.2 Step 2 Bucket 7: to_nanboxed now returns
                    // Result; surface the V3-S5 ckpt-5 SURFACE through the
                    // caller's Result chain instead of panicking.
                    let target_value = target.to_nanboxed()?;
                    let target_name = struct_def.name.clone();
                    let handler_span = handler.span;
                    let execution = self.execute_comptime_annotation_handler(
                        ann,
                        &handler,
                        target_value,
                        &compiled.param_names,
                        &[],
                    )?;

                    if self
                        .process_comptime_directives(execution.directives, &target_name)
                        .map_err(|e| ShapeError::RuntimeError {
                            message: format!(
                                "Comptime handler '{}' directive processing failed: {}",
                                ann.name, e
                            ),
                            location: Some(self.span_to_source_location(handler_span)),
                        })?
                    {
                        removed = true;
                        break;
                    }
                }
            }
            if removed {
                break;
            }
        }
        Ok(removed)
    }

    fn current_module_path_for(&self, module_name: &str) -> String {
        if let Some(parent) = self.module_scope_stack.last() {
            format!("{}::{}", parent, module_name)
        } else {
            module_name.to_string()
        }
    }

    pub(super) fn qualify_module_symbol(module_path: &str, name: &str) -> String {
        format!("{}::{}", module_path, name)
    }

    /// R8 W8 Cluster A (2026-05-24): predicate for the module-level
    /// `const` reject-runtime-init validation. Returns `true` when the
    /// initializer expression can be resolved entirely before runtime:
    ///   - literals (any kind)
    ///   - `comptime { ... }` expressions, which the expression compiler
    ///     evaluates immediately and lowers to a constant value
    ///   - unary `-` / `!` / `~` applied to a comptime-evaluable operand
    /// Function calls, identifiers, binary ops, etc. return `false` and
    /// surface a clean compile error per the dispatch's reject test.
    /// ADR-006 §2.7.5 stamp-at-compile-time alignment: accepted forms must
    /// be resolved before runtime bytecode observes the binding.
    fn const_initializer_is_comptime_evaluable(expr: &shape_ast::ast::Expr) -> bool {
        use shape_ast::ast::Expr;
        match expr {
            Expr::Literal(_, _) => true,
            Expr::Comptime(_, _) => true,
            Expr::UnaryOp { operand, .. } => Self::const_initializer_is_comptime_evaluable(operand),
            _ => false,
        }
    }

    /// Returns true if a name refers to a builtin/primitive type that should
    /// not be module-qualified.
    fn is_builtin_type_name(name: &str) -> bool {
        matches!(
            name,
            "int" | "number" | "string" | "bool" | "decimal" | "bigint"
                | "Array" | "HashMap" | "Set" | "Option" | "Result" | "DateTime"
                | "Content" | "Table" | "DataTable" | "Mat"
                // W18.5 per-type content builders (supervisor D4,
                // R8 W3 2026-05-24): `Code::new()` / `KeyValue::new()`
                // namespaces ride the same builtin-type-name path as
                // `Table` / `Content` so module-qualification doesn't
                // wrap them. The runtime ctors are wired in
                // `function_calls.rs::compile_type_namespace_builtin_call`.
                | "Code" | "KeyValue"
                | "Json" | "Duration" | "Regex"
                | "Vec"
                | "int8" | "int16" | "int32" | "int64"
                | "uint8" | "uint16" | "uint32" | "uint64"
                | "float32" | "float64"
                | "IoHandle"
        )
    }

    fn qualify_type_name(
        type_name: &shape_ast::ast::TypeName,
        module_path: &str,
    ) -> shape_ast::ast::TypeName {
        match type_name {
            shape_ast::ast::TypeName::Simple(path)
                if !path.is_qualified() && !Self::is_builtin_type_name(path.as_str()) =>
            {
                shape_ast::ast::TypeName::Simple(
                    Self::qualify_module_symbol(module_path, path.as_str()).into(),
                )
            }
            shape_ast::ast::TypeName::Generic { name, type_args }
                if !name.is_qualified() && !Self::is_builtin_type_name(name.as_str()) =>
            {
                shape_ast::ast::TypeName::Generic {
                    name: Self::qualify_module_symbol(module_path, name.as_str()).into(),
                    type_args: type_args.clone(),
                }
            }
            _ => type_name.clone(),
        }
    }

    fn collect_type_params_from_type_name(
        type_name: &shape_ast::ast::TypeName,
    ) -> std::collections::HashSet<String> {
        let mut params = std::collections::HashSet::new();
        if let shape_ast::ast::TypeName::Generic { type_args, .. } = type_name {
            for arg in type_args {
                let Some(name) = arg.as_type_name_str() else {
                    continue;
                };
                if !name.contains("::")
                    && name.len() <= 2
                    && name
                        .chars()
                        .next()
                        .is_some_and(|ch| ch.is_ascii_uppercase())
                {
                    params.insert(name.to_string());
                }
            }
        }
        params
    }

    fn collect_type_params_from_type_params(
        type_params: &Option<Vec<shape_ast::ast::TypeParam>>,
    ) -> std::collections::HashSet<String> {
        type_params
            .as_ref()
            .map(|params| {
                params
                    .iter()
                    .map(|param| param.name().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn qualify_module_type_annotation(
        annotation: &shape_ast::ast::TypeAnnotation,
        module_path: &str,
        type_params: &std::collections::HashSet<String>,
    ) -> shape_ast::ast::TypeAnnotation {
        use shape_ast::ast::TypeAnnotation;

        let qualify_name = |name: &str| -> String {
            if name.contains("::")
                || Self::is_builtin_type_name(name)
                || name == "Self"
                || type_params.contains(name)
            {
                name.to_string()
            } else {
                Self::qualify_module_symbol(module_path, name)
            }
        };

        match annotation {
            TypeAnnotation::Basic(name) => TypeAnnotation::Basic(qualify_name(name)),
            TypeAnnotation::Reference(path) => {
                TypeAnnotation::Reference(qualify_name(path.as_str()).into())
            }
            TypeAnnotation::Generic { name, args } => TypeAnnotation::Generic {
                name: qualify_name(name.as_str()).into(),
                args: args
                    .iter()
                    .map(|arg| Self::qualify_module_type_annotation(arg, module_path, type_params))
                    .collect(),
            },
            TypeAnnotation::Array(inner) => TypeAnnotation::Array(Box::new(
                Self::qualify_module_type_annotation(inner, module_path, type_params),
            )),
            TypeAnnotation::Tuple(items) => TypeAnnotation::Tuple(
                items
                    .iter()
                    .map(|item| {
                        Self::qualify_module_type_annotation(item, module_path, type_params)
                    })
                    .collect(),
            ),
            TypeAnnotation::Function { params, returns } => TypeAnnotation::Function {
                params: params
                    .iter()
                    .cloned()
                    .map(|mut param| {
                        param.type_annotation = Self::qualify_module_type_annotation(
                            &param.type_annotation,
                            module_path,
                            type_params,
                        );
                        param
                    })
                    .collect(),
                returns: Box::new(Self::qualify_module_type_annotation(
                    returns,
                    module_path,
                    type_params,
                )),
            },
            TypeAnnotation::Union(items) => TypeAnnotation::Union(
                items
                    .iter()
                    .map(|item| {
                        Self::qualify_module_type_annotation(item, module_path, type_params)
                    })
                    .collect(),
            ),
            TypeAnnotation::Intersection(items) => TypeAnnotation::Intersection(
                items
                    .iter()
                    .map(|item| {
                        Self::qualify_module_type_annotation(item, module_path, type_params)
                    })
                    .collect(),
            ),
            TypeAnnotation::Object(fields) => TypeAnnotation::Object(
                fields
                    .iter()
                    .cloned()
                    .map(|mut field| {
                        field.type_annotation = Self::qualify_module_type_annotation(
                            &field.type_annotation,
                            module_path,
                            type_params,
                        );
                        field
                    })
                    .collect(),
            ),
            TypeAnnotation::Borrow { mutable, inner } => TypeAnnotation::Borrow {
                mutable: *mutable,
                inner: Box::new(Self::qualify_module_type_annotation(
                    inner,
                    module_path,
                    type_params,
                )),
            },
            TypeAnnotation::Dyn(traits) => TypeAnnotation::Dyn(
                traits
                    .iter()
                    .map(|trait_path| qualify_name(trait_path.as_str()).into())
                    .collect(),
            ),
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined => annotation.clone(),
        }
    }

    fn qualify_module_function_params(
        params: &mut [FunctionParameter],
        module_path: &str,
        type_params: &std::collections::HashSet<String>,
    ) {
        for param in params {
            if let Some(annotation) = param.type_annotation.as_mut() {
                *annotation =
                    Self::qualify_module_type_annotation(annotation, module_path, type_params);
            }
            if let Some(default_value) = param.default_value.as_mut() {
                Self::qualify_module_expr(default_value, module_path, type_params);
            }
        }
    }

    fn qualify_module_function_signature(
        func: &mut FunctionDef,
        module_path: &str,
    ) -> std::collections::HashSet<String> {
        let type_params = Self::collect_type_params_from_type_params(&func.type_params);
        Self::qualify_module_function_params(&mut func.params, module_path, &type_params);
        if let Some(return_type) = func.return_type.as_mut() {
            *return_type =
                Self::qualify_module_type_annotation(return_type, module_path, &type_params);
        }
        type_params
    }

    fn qualify_module_method_signature(
        method: &mut shape_ast::ast::types::MethodDef,
        module_path: &str,
        receiver_type_params: &std::collections::HashSet<String>,
    ) -> std::collections::HashSet<String> {
        let mut type_params = receiver_type_params.clone();
        if let Some(method_type_params) = &method.type_params {
            for param in method_type_params {
                type_params.insert(param.name().to_string());
            }
        }
        Self::qualify_module_function_params(&mut method.params, module_path, &type_params);
        if let Some(return_type) = method.return_type.as_mut() {
            *return_type =
                Self::qualify_module_type_annotation(return_type, module_path, &type_params);
        }
        type_params
    }

    fn qualify_module_variable_decl(
        decl: &mut shape_ast::ast::VariableDecl,
        module_path: &str,
        type_params: &std::collections::HashSet<String>,
    ) {
        if let Some(annotation) = decl.type_annotation.as_mut() {
            *annotation =
                Self::qualify_module_type_annotation(annotation, module_path, type_params);
        }
        if let Some(value) = decl.value.as_mut() {
            Self::qualify_module_expr(value, module_path, type_params);
        }
    }

    fn qualify_module_assignment(
        assignment: &mut shape_ast::ast::Assignment,
        module_path: &str,
        type_params: &std::collections::HashSet<String>,
    ) {
        Self::qualify_module_expr(&mut assignment.value, module_path, type_params);
    }

    fn qualify_module_statements(
        statements: &mut [Statement],
        module_path: &str,
        type_params: &std::collections::HashSet<String>,
    ) {
        for statement in statements {
            Self::qualify_module_statement(statement, module_path, type_params);
        }
    }

    fn qualify_module_statement(
        statement: &mut Statement,
        module_path: &str,
        type_params: &std::collections::HashSet<String>,
    ) {
        match statement {
            Statement::Return(Some(expr), _)
            | Statement::Expression(expr, _)
            | Statement::SetParamValue {
                expression: expr, ..
            }
            | Statement::SetReturnExpr {
                expression: expr, ..
            }
            | Statement::ReplaceBodyExpr {
                expression: expr, ..
            }
            | Statement::ReplaceModuleExpr {
                expression: expr, ..
            } => Self::qualify_module_expr(expr, module_path, type_params),
            Statement::VariableDecl(decl, _) => {
                Self::qualify_module_variable_decl(decl, module_path, type_params);
            }
            Statement::Assignment(assignment, _) => {
                Self::qualify_module_assignment(assignment, module_path, type_params);
            }
            Statement::For(for_loop, _) => match &mut for_loop.init {
                shape_ast::ast::ForInit::ForIn { iter, .. } => {
                    Self::qualify_module_expr(iter, module_path, type_params);
                    Self::qualify_module_statements(&mut for_loop.body, module_path, type_params);
                }
                shape_ast::ast::ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    Self::qualify_module_statement(init, module_path, type_params);
                    Self::qualify_module_expr(condition, module_path, type_params);
                    Self::qualify_module_expr(update, module_path, type_params);
                    Self::qualify_module_statements(&mut for_loop.body, module_path, type_params);
                }
            },
            Statement::While(while_loop, _) => {
                Self::qualify_module_expr(&mut while_loop.condition, module_path, type_params);
                Self::qualify_module_statements(&mut while_loop.body, module_path, type_params);
            }
            Statement::If(if_stmt, _) => {
                Self::qualify_module_expr(&mut if_stmt.condition, module_path, type_params);
                Self::qualify_module_statements(&mut if_stmt.then_body, module_path, type_params);
                if let Some(else_body) = if_stmt.else_body.as_mut() {
                    Self::qualify_module_statements(else_body, module_path, type_params);
                }
            }
            Statement::Extend(extend, _) => {
                let receiver_type_params =
                    Self::collect_type_params_from_type_name(&extend.type_name);
                for method in &mut extend.methods {
                    let method_type_params = Self::qualify_module_method_signature(
                        method,
                        module_path,
                        &receiver_type_params,
                    );
                    Self::qualify_module_statements(
                        &mut method.body,
                        module_path,
                        &method_type_params,
                    );
                }
            }
            Statement::SetParamType {
                type_annotation, ..
            }
            | Statement::SetReturnType {
                type_annotation, ..
            } => {
                *type_annotation =
                    Self::qualify_module_type_annotation(type_annotation, module_path, type_params);
            }
            Statement::ReplaceBody { body, .. } => {
                Self::qualify_module_statements(body, module_path, type_params);
            }
            _ => {}
        }
    }

    fn qualify_module_block_items(
        items: &mut [shape_ast::ast::BlockItem],
        module_path: &str,
        type_params: &std::collections::HashSet<String>,
    ) {
        for item in items {
            match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => {
                    Self::qualify_module_variable_decl(decl, module_path, type_params);
                }
                shape_ast::ast::BlockItem::Assignment(assignment) => {
                    Self::qualify_module_assignment(assignment, module_path, type_params);
                }
                shape_ast::ast::BlockItem::Statement(statement) => {
                    Self::qualify_module_statement(statement, module_path, type_params);
                }
                shape_ast::ast::BlockItem::Expression(expr) => {
                    Self::qualify_module_expr(expr, module_path, type_params);
                }
            }
        }
    }

    fn qualify_module_expr(
        expr: &mut Expr,
        module_path: &str,
        type_params: &std::collections::HashSet<String>,
    ) {
        let should_qualify_name = |name: &str| {
            !name.contains("::")
                && name != module_path
                && !Self::is_builtin_type_name(name)
                && !type_params.contains(name)
        };

        match expr {
            Expr::FunctionCall {
                args, named_args, ..
            }
            | Expr::QualifiedFunctionCall {
                args, named_args, ..
            } => {
                for arg in args {
                    Self::qualify_module_expr(arg, module_path, type_params);
                }
                for (_, arg) in named_args {
                    Self::qualify_module_expr(arg, module_path, type_params);
                }
            }
            Expr::EnumConstructor {
                enum_name, payload, ..
            } => {
                if should_qualify_name(enum_name.as_str()) {
                    *enum_name =
                        Self::qualify_module_symbol(module_path, enum_name.as_str()).into();
                }
                match payload {
                    shape_ast::ast::EnumConstructorPayload::Unit => {}
                    shape_ast::ast::EnumConstructorPayload::Tuple(values) => {
                        for value in values {
                            Self::qualify_module_expr(value, module_path, type_params);
                        }
                    }
                    shape_ast::ast::EnumConstructorPayload::Struct(fields) => {
                        for (_, value) in fields {
                            Self::qualify_module_expr(value, module_path, type_params);
                        }
                    }
                }
            }
            Expr::PropertyAccess { object, .. } => {
                Self::qualify_module_expr(object, module_path, type_params);
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                Self::qualify_module_expr(object, module_path, type_params);
                Self::qualify_module_expr(index, module_path, type_params);
                if let Some(end_index) = end_index.as_mut() {
                    Self::qualify_module_expr(end_index, module_path, type_params);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                Self::qualify_module_expr(left, module_path, type_params);
                Self::qualify_module_expr(right, module_path, type_params);
            }
            Expr::UnaryOp { operand, .. } => {
                Self::qualify_module_expr(operand, module_path, type_params);
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                Self::qualify_module_expr(condition, module_path, type_params);
                Self::qualify_module_expr(then_expr, module_path, type_params);
                if let Some(else_expr) = else_expr.as_mut() {
                    Self::qualify_module_expr(else_expr, module_path, type_params);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field {
                            value,
                            type_annotation,
                            ..
                        } => {
                            if let Some(annotation) = type_annotation.as_mut() {
                                *annotation = Self::qualify_module_type_annotation(
                                    annotation,
                                    module_path,
                                    type_params,
                                );
                            }
                            Self::qualify_module_expr(value, module_path, type_params);
                        }
                        ObjectEntry::Spread(spread) => {
                            Self::qualify_module_expr(spread, module_path, type_params);
                        }
                    }
                }
            }
            Expr::Array(items, _) => {
                for item in items {
                    Self::qualify_module_expr(item, module_path, type_params);
                }
            }
            Expr::Block(block, _) => {
                Self::qualify_module_block_items(&mut block.items, module_path, type_params);
            }
            Expr::TypeAssertion {
                expr,
                type_annotation,
                meta_param_overrides,
                ..
            } => {
                Self::qualify_module_expr(expr, module_path, type_params);
                *type_annotation =
                    Self::qualify_module_type_annotation(type_annotation, module_path, type_params);
                if let Some(overrides) = meta_param_overrides.as_mut() {
                    for value in overrides.values_mut() {
                        Self::qualify_module_expr(value, module_path, type_params);
                    }
                }
            }
            Expr::InstanceOf {
                expr,
                type_annotation,
                ..
            } => {
                Self::qualify_module_expr(expr, module_path, type_params);
                *type_annotation =
                    Self::qualify_module_type_annotation(type_annotation, module_path, type_params);
            }
            Expr::FunctionExpr {
                params,
                return_type,
                body,
                ..
            } => {
                Self::qualify_module_function_params(params, module_path, type_params);
                if let Some(return_type) = return_type.as_mut() {
                    *return_type =
                        Self::qualify_module_type_annotation(return_type, module_path, type_params);
                }
                Self::qualify_module_statements(body, module_path, type_params);
            }
            Expr::Spread(inner, _)
            | Expr::Await(inner, _)
            | Expr::AsyncScope(inner, _)
            | Expr::TryOperator(inner, _)
            | Expr::UsingImpl { expr: inner, .. }
            | Expr::Reference { expr: inner, .. }
            | Expr::TimeframeContext { expr: inner, .. } => {
                Self::qualify_module_expr(inner, module_path, type_params);
            }
            Expr::If(if_expr, _) => {
                Self::qualify_module_expr(&mut if_expr.condition, module_path, type_params);
                Self::qualify_module_expr(&mut if_expr.then_branch, module_path, type_params);
                if let Some(else_branch) = if_expr.else_branch.as_mut() {
                    Self::qualify_module_expr(else_branch, module_path, type_params);
                }
            }
            Expr::While(while_expr, _) => {
                Self::qualify_module_expr(&mut while_expr.condition, module_path, type_params);
                Self::qualify_module_expr(&mut while_expr.body, module_path, type_params);
            }
            Expr::For(for_expr, _) => {
                Self::qualify_module_expr(&mut for_expr.iterable, module_path, type_params);
                Self::qualify_module_expr(&mut for_expr.body, module_path, type_params);
            }
            Expr::Loop(loop_expr, _) => {
                Self::qualify_module_expr(&mut loop_expr.body, module_path, type_params);
            }
            Expr::Let(let_expr, _) => {
                if let Some(annotation) = let_expr.type_annotation.as_mut() {
                    *annotation =
                        Self::qualify_module_type_annotation(annotation, module_path, type_params);
                }
                if let Some(value) = let_expr.value.as_mut() {
                    Self::qualify_module_expr(value, module_path, type_params);
                }
                Self::qualify_module_expr(&mut let_expr.body, module_path, type_params);
            }
            Expr::Assign(assign_expr, _) => {
                Self::qualify_module_expr(&mut assign_expr.target, module_path, type_params);
                Self::qualify_module_expr(&mut assign_expr.value, module_path, type_params);
            }
            Expr::Return(Some(inner), _) | Expr::Break(Some(inner), _) => {
                Self::qualify_module_expr(inner, module_path, type_params);
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                Self::qualify_module_expr(receiver, module_path, type_params);
                for arg in args {
                    Self::qualify_module_expr(arg, module_path, type_params);
                }
                for (_, arg) in named_args {
                    Self::qualify_module_expr(arg, module_path, type_params);
                }
            }
            Expr::Match(match_expr, _) => {
                Self::qualify_module_expr(&mut match_expr.scrutinee, module_path, type_params);
                for arm in &mut match_expr.arms {
                    if let Some(guard) = arm.guard.as_mut() {
                        Self::qualify_module_expr(guard, module_path, type_params);
                    }
                    Self::qualify_module_expr(&mut arm.body, module_path, type_params);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start.as_mut() {
                    Self::qualify_module_expr(start, module_path, type_params);
                }
                if let Some(end) = end.as_mut() {
                    Self::qualify_module_expr(end, module_path, type_params);
                }
            }
            Expr::SimulationCall { params, .. } => {
                for (_, value) in params {
                    Self::qualify_module_expr(value, module_path, type_params);
                }
            }
            Expr::StructLiteral {
                type_name, fields, ..
            } => {
                if should_qualify_name(type_name.as_str()) {
                    *type_name =
                        Self::qualify_module_symbol(module_path, type_name.as_str()).into();
                }
                for (_, value) in fields {
                    Self::qualify_module_expr(value, module_path, type_params);
                }
            }
            Expr::Annotated {
                annotation, target, ..
            } => {
                for arg in &mut annotation.args {
                    Self::qualify_module_expr(arg, module_path, type_params);
                }
                Self::qualify_module_expr(target, module_path, type_params);
            }
            Expr::AsyncLet(async_let, _) => {
                Self::qualify_module_expr(&mut async_let.expr, module_path, type_params);
            }
            Expr::Comptime(statements, _) => {
                Self::qualify_module_statements(statements, module_path, type_params);
            }
            Expr::ComptimeFor(comptime_for, _) => {
                Self::qualify_module_expr(&mut comptime_for.iterable, module_path, type_params);
                Self::qualify_module_statements(&mut comptime_for.body, module_path, type_params);
            }
            Expr::TableRows(rows, _) => {
                for row in rows {
                    for value in row {
                        Self::qualify_module_expr(value, module_path, type_params);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn qualify_module_item(&self, item: &Item, module_path: &str) -> Result<Item> {
        match item {
            Item::Function(func, span) => {
                let mut qualified = func.clone();
                qualified.name = Self::qualify_module_symbol(module_path, &func.name);
                if qualified.declaring_module_path.is_none() {
                    qualified.declaring_module_path = Some(module_path.to_string());
                }
                let type_params =
                    Self::qualify_module_function_signature(&mut qualified, module_path);
                Self::qualify_module_statements(&mut qualified.body, module_path, &type_params);
                Ok(Item::Function(qualified, *span))
            }
            Item::Export(export, span) if export.source_decl.is_none() => {
                let mut qualified = export.clone();
                match &mut qualified.item {
                    ExportItem::Function(func) => {
                        func.name = Self::qualify_module_symbol(module_path, &func.name);
                        if func.declaring_module_path.is_none() {
                            func.declaring_module_path = Some(module_path.to_string());
                        }
                        let type_params =
                            Self::qualify_module_function_signature(func, module_path);
                        Self::qualify_module_statements(&mut func.body, module_path, &type_params);
                    }
                    ExportItem::BuiltinFunction(func) => {
                        func.name = Self::qualify_module_symbol(module_path, &func.name);
                    }
                    ExportItem::ForeignFunction(func) => {
                        func.name = Self::qualify_module_symbol(module_path, &func.name);
                    }
                    ExportItem::Annotation(annotation) => {
                        annotation.name =
                            Self::qualify_module_symbol(module_path, &annotation.name);
                    }
                    ExportItem::Struct(def) => {
                        def.name = Self::qualify_module_symbol(module_path, &def.name);
                        let type_params =
                            Self::collect_type_params_from_type_params(&def.type_params);
                        for field in &mut def.fields {
                            field.type_annotation = Self::qualify_module_type_annotation(
                                &field.type_annotation,
                                module_path,
                                &type_params,
                            );
                            if let Some(default_value) = field.default_value.as_mut() {
                                Self::qualify_module_expr(default_value, module_path, &type_params);
                            }
                        }
                    }
                    ExportItem::Enum(def) => {
                        def.name = Self::qualify_module_symbol(module_path, &def.name);
                    }
                    ExportItem::TypeAlias(def) => {
                        def.name = Self::qualify_module_symbol(module_path, &def.name);
                        let type_params =
                            Self::collect_type_params_from_type_params(&def.type_params);
                        def.type_annotation = Self::qualify_module_type_annotation(
                            &def.type_annotation,
                            module_path,
                            &type_params,
                        );
                    }
                    ExportItem::Trait(def) => {
                        def.name = Self::qualify_module_symbol(module_path, &def.name);
                    }
                    _ => {}
                }
                Ok(Item::Export(qualified, *span))
            }
            Item::BuiltinFunctionDecl(def, span) => {
                let mut qualified = def.clone();
                qualified.name = Self::qualify_module_symbol(module_path, &def.name);
                Ok(Item::BuiltinFunctionDecl(qualified, *span))
            }
            Item::AnnotationDef(def, span) => {
                let mut qualified = def.clone();
                qualified.name = Self::qualify_module_symbol(module_path, &def.name);
                Ok(Item::AnnotationDef(qualified, *span))
            }
            Item::VariableDecl(decl, span) => {
                if decl.kind != VarKind::Const {
                    return Err(ShapeError::SemanticError {
                        message: "module-level variable declarations currently require `const`"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                let mut qualified = decl.clone();
                let Some(name) = decl.pattern.as_identifier() else {
                    return Err(ShapeError::SemanticError {
                        message:
                            "module-level constants currently require a simple identifier binding"
                                .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                };
                qualified.pattern = DestructurePattern::Identifier(
                    Self::qualify_module_symbol(module_path, name),
                    *span,
                );
                Ok(Item::VariableDecl(qualified, *span))
            }
            Item::Statement(Statement::VariableDecl(decl, stmt_span), item_span) => {
                if decl.kind != VarKind::Const {
                    return Err(ShapeError::SemanticError {
                        message: "module-level variable declarations currently require `const`"
                            .to_string(),
                        location: Some(self.span_to_source_location(*stmt_span)),
                    });
                }
                let mut qualified = decl.clone();
                let Some(name) = decl.pattern.as_identifier() else {
                    return Err(ShapeError::SemanticError {
                        message:
                            "module-level constants currently require a simple identifier binding"
                                .to_string(),
                        location: Some(self.span_to_source_location(*stmt_span)),
                    });
                };
                qualified.pattern = DestructurePattern::Identifier(
                    Self::qualify_module_symbol(module_path, name),
                    *stmt_span,
                );
                Ok(Item::Statement(
                    Statement::VariableDecl(qualified, *stmt_span),
                    *item_span,
                ))
            }
            Item::Statement(Statement::Assignment(assign, stmt_span), item_span) => {
                let mut qualified = assign.clone();
                if let Some(name) = assign.pattern.as_identifier() {
                    qualified.pattern = DestructurePattern::Identifier(
                        Self::qualify_module_symbol(module_path, name),
                        *stmt_span,
                    );
                }
                Ok(Item::Statement(
                    Statement::Assignment(qualified, *stmt_span),
                    *item_span,
                ))
            }
            Item::Export(export, span) if export.source_decl.is_some() => {
                // pub const/let/var: unwrap the source_decl and qualify it as a VariableDecl
                let decl = export.source_decl.as_ref().unwrap();
                if decl.kind != VarKind::Const {
                    return Err(ShapeError::SemanticError {
                        message: "module-level variable declarations currently require `const`"
                            .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
                let mut qualified = decl.clone();
                let Some(name) = decl.pattern.as_identifier() else {
                    return Err(ShapeError::SemanticError {
                        message:
                            "module-level constants currently require a simple identifier binding"
                                .to_string(),
                        location: Some(self.span_to_source_location(*span)),
                    });
                };
                qualified.pattern = DestructurePattern::Identifier(
                    Self::qualify_module_symbol(module_path, name),
                    *span,
                );
                Ok(Item::VariableDecl(qualified, *span))
            }
            Item::StructType(def, span) => {
                let mut q = def.clone();
                q.name = Self::qualify_module_symbol(module_path, &def.name);
                let type_params = Self::collect_type_params_from_type_params(&q.type_params);
                for field in &mut q.fields {
                    field.type_annotation = Self::qualify_module_type_annotation(
                        &field.type_annotation,
                        module_path,
                        &type_params,
                    );
                    if let Some(default_value) = field.default_value.as_mut() {
                        Self::qualify_module_expr(default_value, module_path, &type_params);
                    }
                }
                Ok(Item::StructType(q, *span))
            }
            Item::Enum(def, span) => {
                let mut q = def.clone();
                q.name = Self::qualify_module_symbol(module_path, &def.name);
                Ok(Item::Enum(q, *span))
            }
            Item::TypeAlias(def, span) => {
                let mut q = def.clone();
                q.name = Self::qualify_module_symbol(module_path, &def.name);
                let type_params = Self::collect_type_params_from_type_params(&q.type_params);
                q.type_annotation = Self::qualify_module_type_annotation(
                    &q.type_annotation,
                    module_path,
                    &type_params,
                );
                Ok(Item::TypeAlias(q, *span))
            }
            Item::Trait(def, span) => {
                let mut q = def.clone();
                q.name = Self::qualify_module_symbol(module_path, &def.name);
                Ok(Item::Trait(q, *span))
            }
            Item::Extend(extend, span) => {
                let mut q = extend.clone();
                q.type_name = Self::qualify_type_name(&extend.type_name, module_path);
                let receiver_type_params =
                    Self::collect_type_params_from_type_name(&extend.type_name);
                for method in &mut q.methods {
                    if method.declaring_module_path.is_none() {
                        method.declaring_module_path = Some(module_path.to_string());
                    }
                    let method_type_params = Self::qualify_module_method_signature(
                        method,
                        module_path,
                        &receiver_type_params,
                    );
                    Self::qualify_module_statements(
                        &mut method.body,
                        module_path,
                        &method_type_params,
                    );
                }
                Ok(Item::Extend(q, *span))
            }
            Item::Impl(impl_block, span) => {
                let mut q = impl_block.clone();
                q.target_type = Self::qualify_type_name(&impl_block.target_type, module_path);
                let receiver_type_params =
                    Self::collect_type_params_from_type_name(&impl_block.target_type);
                for method in &mut q.methods {
                    if method.declaring_module_path.is_none() {
                        method.declaring_module_path = Some(module_path.to_string());
                    }
                    let method_type_params = Self::qualify_module_method_signature(
                        method,
                        module_path,
                        &receiver_type_params,
                    );
                    Self::qualify_module_statements(
                        &mut method.body,
                        module_path,
                        &method_type_params,
                    );
                }
                // Do NOT qualify trait_name — traits may be imported from other scopes
                Ok(Item::Impl(q, *span))
            }
            _ => Ok(item.clone()),
        }
    }

    pub(super) fn collect_module_runtime_exports(
        &self,
        items: &[Item],
        module_path: &str,
    ) -> Vec<(String, String)> {
        let mut exports = Vec::new();
        let has_explicit_exports = items.iter().any(|item| matches!(item, Item::Export(..)));

        if has_explicit_exports {
            for item in items {
                let Item::Export(export, _) = item else {
                    continue;
                };
                if let Some(ref decl) = export.source_decl {
                    if let Some(name) = decl.pattern.as_identifier() {
                        exports.push((
                            name.to_string(),
                            Self::qualify_module_symbol(module_path, name),
                        ));
                    }
                }
                match &export.item {
                    ExportItem::Function(func) => {
                        let exported_name = func
                            .name
                            .rsplit("::")
                            .next()
                            .unwrap_or(func.name.as_str())
                            .to_string();
                        exports.push((
                            exported_name.clone(),
                            Self::qualify_module_symbol(module_path, &exported_name),
                        ));
                    }
                    ExportItem::ForeignFunction(func) => {
                        let exported_name = func
                            .name
                            .rsplit("::")
                            .next()
                            .unwrap_or(func.name.as_str())
                            .to_string();
                        exports.push((
                            exported_name.clone(),
                            Self::qualify_module_symbol(module_path, &exported_name),
                        ));
                    }
                    ExportItem::Named(specs) => {
                        for spec in specs {
                            let exported_name =
                                spec.alias.clone().unwrap_or_else(|| spec.name.clone());
                            exports.push((
                                exported_name,
                                Self::qualify_module_symbol(module_path, &spec.name),
                            ));
                        }
                    }
                    // W9: Annotations are compile-time only — they live in
                    // `compiled_annotations`, not as runtime values. Including
                    // them as runtime exports would synthesize a module-object
                    // entry like `{ remote: std::core::remote::remote }` whose
                    // RHS has no runtime variable binding. Annotation imports
                    // are resolved through `imported_annotations` + use-site
                    // lookup; the module object only carries runtime values.
                    ExportItem::Annotation(_) => {}
                    _ => {}
                }
            }
            exports.sort_by(|a, b| a.0.cmp(&b.0));
            exports.dedup_by(|a, b| a.0 == b.0);
            return exports;
        }

        for item in items {
            match item {
                Item::Function(func, _) => {
                    exports.push((
                        func.name.clone(),
                        Self::qualify_module_symbol(module_path, &func.name),
                    ));
                }
                Item::VariableDecl(decl, _) => {
                    if decl.kind == VarKind::Const
                        && let Some(name) = decl.pattern.as_identifier()
                    {
                        exports.push((
                            name.to_string(),
                            Self::qualify_module_symbol(module_path, name),
                        ));
                    }
                }
                Item::Statement(Statement::VariableDecl(decl, _), _) => {
                    if decl.kind == VarKind::Const
                        && let Some(name) = decl.pattern.as_identifier()
                    {
                        exports.push((
                            name.to_string(),
                            Self::qualify_module_symbol(module_path, name),
                        ));
                    }
                }
                Item::Export(export, _) => {
                    if let Some(ref decl) = export.source_decl {
                        if let Some(name) = decl.pattern.as_identifier() {
                            exports.push((
                                name.to_string(),
                                Self::qualify_module_symbol(module_path, name),
                            ));
                        }
                    }
                }
                Item::Module(module, _) => {
                    exports.push((
                        module.name.clone(),
                        Self::qualify_module_symbol(module_path, &module.name),
                    ));
                }
                // W9: Annotations are compile-time only — see the explicit-
                // export arm above. Skipped here for the same reason.
                Item::AnnotationDef(_, _) => {}
                // Note: Type items (StructType, Enum, TypeAlias, Trait, Interface) are NOT
                // included as runtime exports. They are resolved through the type system
                // (struct_types, schema_registry, type_aliases) via resolve_type_name(),
                // not through runtime module bindings.
                _ => {}
            }
        }
        exports.sort_by(|a, b| a.0.cmp(&b.0));
        exports.dedup_by(|a, b| a.0 == b.0);
        exports
    }

    fn module_target_fields(items: &[Item]) -> Vec<(String, String)> {
        let mut fields = Vec::new();
        for item in items {
            match item {
                Item::Function(func, _) => fields.push((func.name.clone(), "function".to_string())),
                Item::VariableDecl(decl, _) => {
                    if let Some(name) = decl.pattern.as_identifier() {
                        let type_name = decl
                            .type_annotation
                            .as_ref()
                            .and_then(TypeAnnotation::as_simple_name)
                            .unwrap_or("any")
                            .to_string();
                        fields.push((name.to_string(), type_name));
                    }
                }
                Item::Statement(Statement::VariableDecl(decl, _), _) => {
                    if let Some(name) = decl.pattern.as_identifier() {
                        let type_name = decl
                            .type_annotation
                            .as_ref()
                            .and_then(TypeAnnotation::as_simple_name)
                            .unwrap_or("any")
                            .to_string();
                        fields.push((name.to_string(), type_name));
                    }
                }
                Item::Export(export, _) => {
                    if let Some(ref decl) = export.source_decl {
                        if let Some(name) = decl.pattern.as_identifier() {
                            let type_name = decl
                                .type_annotation
                                .as_ref()
                                .and_then(TypeAnnotation::as_simple_name)
                                .unwrap_or("any")
                                .to_string();
                            fields.push((name.to_string(), type_name));
                        }
                    }
                }
                Item::StructType(def, _) => fields.push((def.name.clone(), "type".to_string())),
                Item::Enum(def, _) => fields.push((def.name.clone(), "type".to_string())),
                Item::TypeAlias(def, _) => fields.push((def.name.clone(), "type".to_string())),
                Item::Module(def, _) => fields.push((def.name.clone(), "module".to_string())),
                // H4: Include annotation definitions in module target fields
                Item::AnnotationDef(def, _) => {
                    fields.push((def.name.clone(), "annotation".to_string()))
                }
                _ => {}
            }
        }
        fields
    }

    fn process_comptime_directives_for_module(
        &mut self,
        directives: Vec<super::comptime_builtins::ComptimeDirective>,
        module_name: &str,
        module_items: &mut Vec<Item>,
    ) -> std::result::Result<bool, String> {
        let mut removed = false;
        for directive in directives {
            match directive {
                super::comptime_builtins::ComptimeDirective::Extend(extend) => {
                    self.apply_comptime_extend(extend, module_name)
                        .map_err(|e| e.to_string())?;
                }
                super::comptime_builtins::ComptimeDirective::RemoveTarget => {
                    removed = true;
                    break;
                }
                super::comptime_builtins::ComptimeDirective::ReplaceModule { items } => {
                    *module_items = items;
                }
                super::comptime_builtins::ComptimeDirective::SetParamType { .. }
                | super::comptime_builtins::ComptimeDirective::SetParamValue { .. } => {
                    return Err(
                        "`set param` directives are only valid when compiling function targets"
                            .to_string(),
                    );
                }
                super::comptime_builtins::ComptimeDirective::SetReturnType { .. } => {
                    return Err(
                        "`set return` directives are only valid when compiling function targets"
                            .to_string(),
                    );
                }
                super::comptime_builtins::ComptimeDirective::ReplaceBody { .. } => {
                    return Err(
                        "`replace body` directives are only valid when compiling function targets"
                            .to_string(),
                    );
                }
            }
        }
        Ok(removed)
    }

    fn execute_module_comptime_handlers(
        &mut self,
        module_def: &ModuleDecl,
        module_path: &str,
        module_items: &mut Vec<Item>,
    ) -> Result<bool> {
        let mut removed = false;
        for ann in &module_def.annotations {
            if let Some((_, compiled)) = self.lookup_compiled_annotation(ann) {
                let handlers = [
                    compiled.comptime_pre_handler,
                    compiled.comptime_post_handler,
                ];
                for handler in handlers.into_iter().flatten() {
                    let target = super::comptime_target::ComptimeTarget::from_module(
                        module_path,
                        &Self::module_target_fields(module_items),
                    );
                    // R8 W9 G.2 Step 2 Bucket 7: to_nanboxed now returns
                    // Result; surface the V3-S5 ckpt-5 SURFACE through the
                    // caller's Result chain instead of panicking.
                    let target_value = target.to_nanboxed()?;
                    let handler_span = handler.span;
                    let execution = self.execute_comptime_annotation_handler(
                        ann,
                        &handler,
                        target_value,
                        &compiled.param_names,
                        &[],
                    )?;
                    if self
                        .process_comptime_directives_for_module(
                            execution.directives,
                            module_path,
                            module_items,
                        )
                        .map_err(|e| ShapeError::RuntimeError {
                            message: format!(
                                "Comptime handler '{}' directive processing failed: {}",
                                ann.name, e
                            ),
                            location: Some(self.span_to_source_location(handler_span)),
                        })?
                    {
                        removed = true;
                        break;
                    }
                }
            }
            if removed {
                break;
            }
        }
        Ok(removed)
    }

    fn inject_module_local_comptime_helper_aliases(
        &self,
        module_path: &str,
        helpers: &mut Vec<FunctionDef>,
    ) {
        let module_prefix = format!("{}::", module_path);
        let mut seen: std::collections::HashSet<String> =
            helpers.iter().map(|h| h.name.clone()).collect();
        let mut aliases = Vec::new();

        for helper in helpers.iter() {
            let Some(local_name) = helper.name.strip_prefix(&module_prefix) else {
                continue;
            };
            if local_name.contains("::") || !seen.insert(local_name.to_string()) {
                continue;
            }
            let mut alias = helper.clone();
            alias.name = local_name.to_string();
            aliases.push(alias);
        }

        helpers.extend(aliases);
    }

    fn execute_module_inline_comptime_blocks(
        &mut self,
        module_path: &str,
        module_items: &mut Vec<Item>,
    ) -> Result<bool> {
        loop {
            let Some(idx) = module_items
                .iter()
                .position(|item| matches!(item, Item::Comptime(_, _)))
            else {
                break;
            };

            let (stmts, span) = match module_items[idx].clone() {
                Item::Comptime(stmts, span) => (stmts, span),
                _ => unreachable!("index is guarded by position() matcher"),
            };

            let extensions: Vec<_> = self
                .extension_registry
                .as_ref()
                .map(|r| r.as_ref().clone())
                .unwrap_or_default();
            let trait_impls = self.type_inference.env.trait_impl_keys();
            let known_type_symbols: std::collections::HashSet<String> = self
                .struct_types
                .keys()
                .chain(self.type_aliases.keys())
                .cloned()
                .collect();
            let mut comptime_helpers = self.collect_comptime_helpers();
            self.inject_module_local_comptime_helper_aliases(module_path, &mut comptime_helpers);

            // W7 (2026-05-17): TypeReflectionSnapshot for `type_info(T)`
            // resolution from a module-scoped comptime block.
            let type_snapshot = super::comptime_builtins::build_type_reflection_snapshot(self, &[]);
            // J-CT.2 — see `expressions/mod.rs::Expr::Comptime` for
            // rationale on comptime-context items.
            let comptime_impl_blocks = self.comptime_impl_blocks.clone();
            let comptime_context_trait_defs: Vec<_> = self.trait_defs.values().cloned().collect();
            let comptime_context_struct_defs: Vec<_> = self
                .comptime_context_struct_defs
                .values()
                .cloned()
                .collect();
            let execution = super::comptime::execute_comptime_with_context(
                &stmts,
                &comptime_helpers,
                &comptime_impl_blocks,
                &comptime_context_trait_defs,
                &comptime_context_struct_defs,
                &extensions,
                trait_impls,
                known_type_symbols,
                type_snapshot,
            )
            .map_err(|e| ShapeError::RuntimeError {
                message: format!(
                    "Comptime block evaluation failed: {}",
                    super::helpers::strip_error_prefix(&e)
                ),
                location: Some(self.span_to_source_location(span)),
            })?;

            if self
                .process_comptime_directives_for_module(
                    execution.directives,
                    module_path,
                    module_items,
                )
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!("Comptime block directive processing failed: {}", e),
                    location: Some(self.span_to_source_location(span)),
                })?
            {
                return Ok(true);
            }

            if idx < module_items.len() && matches!(module_items[idx], Item::Comptime(_, _)) {
                module_items.remove(idx);
            }
        }

        Ok(false)
    }

    pub(super) fn register_missing_module_items(&mut self, item: &Item) -> Result<()> {
        match item {
            Item::Function(func, _) => {
                if !self.function_defs.contains_key(&func.name) {
                    self.register_function(func)?;
                }
                Ok(())
            }
            Item::Trait(trait_def, _) => {
                if !self.trait_defs.contains_key(&trait_def.name) {
                    self.known_traits.insert(trait_def.name.clone());
                    self.trait_defs
                        .insert(trait_def.name.clone(), trait_def.clone());
                    self.type_inference.env.define_trait(trait_def);
                }
                Ok(())
            }
            Item::Enum(enum_def, _) => {
                self.register_enum(enum_def)?;
                Ok(())
            }
            Item::StructType(struct_def, span) => {
                // Pre-declare struct type layout without running full
                // register_struct_type (which does annotation validation,
                // comptime handlers, native layout, and schema registration).
                // This makes the type name resolvable for forward references
                // during first-pass registration.
                //
                // J-CT.2 — also overwrite empty-field placeholders inserted
                // by the comptime mini-VM bootstrapping via
                // `compile_and_execute_comptime_program`'s `known_type_symbols`
                // pre-population (which inserts `Vec::new()` field lists to
                // mark types as "known" for downstream resolution). If the
                // existing entry is empty and we have real fields, replace
                // it. Without this, the `contains_key` guard short-circuits
                // and the real fields never land — struct literals inside
                // `comptime { }` blocks then fail with "Unknown field 'x'".
                let existing_is_empty = self
                    .struct_types
                    .get(&struct_def.name)
                    .map(|(names, _)| names.is_empty())
                    .unwrap_or(false);
                let has_real_fields = struct_def.fields.iter().any(|f| !f.is_comptime);
                if !self.struct_types.contains_key(&struct_def.name)
                    || (existing_is_empty && has_real_fields)
                {
                    let runtime_field_names: Vec<String> = struct_def
                        .fields
                        .iter()
                        .filter(|f| !f.is_comptime)
                        .map(|f| f.name.clone())
                        .collect();
                    let runtime_field_types = struct_def
                        .fields
                        .iter()
                        .filter(|f| !f.is_comptime)
                        .map(|f| (f.name.clone(), f.type_annotation.clone()))
                        .collect::<std::collections::HashMap<_, _>>();
                    self.struct_types
                        .insert(struct_def.name.clone(), (runtime_field_names, *span));
                    self.struct_generic_info.insert(
                        struct_def.name.clone(),
                        StructGenericInfo {
                            type_params: struct_def.type_params.clone().unwrap_or_default(),
                            runtime_field_types,
                        },
                    );
                    // J-CT.2 — snapshot full struct AST for the comptime
                    // mini-VM. `comptime_impl_blocks` referencing this
                    // type need the original AST (field annotations,
                    // generic info, default values) to compile
                    // struct-literal constructions + field access inside
                    // `comptime { }` blocks. `struct_types` retains only
                    // field NAMES; the mini-VM gets the full def via
                    // `comptime_context_struct_defs`.
                    self.comptime_context_struct_defs
                        .insert(struct_def.name.clone(), struct_def.clone());
                }
                Ok(())
            }
            Item::TypeAlias(type_alias, _) => {
                if !self.type_aliases.contains_key(&type_alias.name) {
                    let base_type_name = match &type_alias.type_annotation {
                        TypeAnnotation::Basic(name) => Some(name.clone()),
                        TypeAnnotation::Reference(name) => Some(name.to_string()),
                        _ => None,
                    };
                    self.type_aliases.insert(
                        type_alias.name.clone(),
                        base_type_name
                            .unwrap_or_else(|| format!("{:?}", type_alias.type_annotation)),
                    );
                    self.type_inference.env.define_type_alias(
                        &type_alias.name,
                        &type_alias.type_annotation,
                        type_alias.meta_param_overrides.clone(),
                    );
                }
                Ok(())
            }
            Item::BuiltinFunctionDecl(def, _) => self.register_builtin_function_decl(def),
            Item::ForeignFunction(def, _) => {
                if !self.function_defs.contains_key(&def.name) {
                    // Register arity + foreign def (same as register_item_functions)
                    let caller_visible = def.params.iter().filter(|p| !p.is_out).count();
                    self.function_arity_bounds
                        .insert(def.name.clone(), (caller_visible, caller_visible));
                    self.function_const_params
                        .insert(def.name.clone(), Vec::new());
                    self.foreign_function_defs
                        .insert(def.name.clone(), def.clone());
                }
                Ok(())
            }
            Item::Export(export, _) => match &export.item {
                ExportItem::Function(func) => {
                    if !self.function_defs.contains_key(&func.name) {
                        self.register_function(func)?;
                    }
                    Ok(())
                }
                ExportItem::Trait(trait_def) => {
                    if !self.trait_defs.contains_key(&trait_def.name) {
                        self.known_traits.insert(trait_def.name.clone());
                        self.trait_defs
                            .insert(trait_def.name.clone(), trait_def.clone());
                        self.type_inference.env.define_trait(trait_def);
                    }
                    Ok(())
                }
                ExportItem::Enum(enum_def) => {
                    self.register_enum(enum_def)?;
                    Ok(())
                }
                ExportItem::Struct(struct_def) => {
                    // Pre-declare only — full registration happens in second pass
                    if !self.struct_types.contains_key(&struct_def.name) {
                        let runtime_field_names: Vec<String> = struct_def
                            .fields
                            .iter()
                            .filter(|f| !f.is_comptime)
                            .map(|f| f.name.clone())
                            .collect();
                        let runtime_field_types = struct_def
                            .fields
                            .iter()
                            .filter(|f| !f.is_comptime)
                            .map(|f| (f.name.clone(), f.type_annotation.clone()))
                            .collect::<std::collections::HashMap<_, _>>();
                        self.struct_types
                            .insert(struct_def.name.clone(), (runtime_field_names, Span::DUMMY));
                        self.struct_generic_info.insert(
                            struct_def.name.clone(),
                            StructGenericInfo {
                                type_params: struct_def.type_params.clone().unwrap_or_default(),
                                runtime_field_types,
                            },
                        );
                        // J-CT.2 — see Item::StructType arm above for
                        // rationale; mirror the snapshot for exported structs
                        // so `comptime { }` blocks in the same compilation
                        // unit can resolve them.
                        self.comptime_context_struct_defs
                            .insert(struct_def.name.clone(), struct_def.clone());
                    }
                    Ok(())
                }
                ExportItem::TypeAlias(type_alias) => {
                    if !self.type_aliases.contains_key(&type_alias.name) {
                        let base_type_name = match &type_alias.type_annotation {
                            TypeAnnotation::Basic(name) => Some(name.clone()),
                            TypeAnnotation::Reference(name) => Some(name.to_string()),
                            _ => None,
                        };
                        self.type_aliases.insert(
                            type_alias.name.clone(),
                            base_type_name
                                .unwrap_or_else(|| format!("{:?}", type_alias.type_annotation)),
                        );
                        self.type_inference.env.define_type_alias(
                            &type_alias.name,
                            &type_alias.type_annotation,
                            type_alias.meta_param_overrides.clone(),
                        );
                    }
                    Ok(())
                }
                ExportItem::BuiltinFunction(def) => self.register_builtin_function_decl(def),
                ExportItem::ForeignFunction(def) => {
                    if !self.function_defs.contains_key(&def.name) {
                        let caller_visible = def.params.iter().filter(|p| !p.is_out).count();
                        self.function_arity_bounds
                            .insert(def.name.clone(), (caller_visible, caller_visible));
                        self.function_const_params
                            .insert(def.name.clone(), Vec::new());
                        self.foreign_function_defs
                            .insert(def.name.clone(), def.clone());
                    }
                    Ok(())
                }
                _ => Ok(()),
            },
            // Impl and Extend blocks: delegate to register_item_functions
            // which handles the full registration (desugar methods, trait symbols,
            // type inference impls, drop tracking, etc.)
            Item::Impl(..) | Item::Extend(..) => self.register_item_functions(item),
            Item::Module(module, _) => {
                let module_path = self.current_module_path_for(module.name.as_str());
                self.module_scope_stack.push(module_path.clone());
                let register_result = (|| -> Result<()> {
                    for inner in &module.items {
                        let qualified = self.qualify_module_item(inner, &module_path)?;
                        self.register_missing_module_items(&qualified)?;
                    }
                    Ok(())
                })();
                self.module_scope_stack.pop();
                register_result
            }
            _ => Ok(()),
        }
    }

    fn compile_module_decl(&mut self, module_def: &ModuleDecl, span: Span) -> Result<()> {
        for ann in &module_def.annotations {
            self.validate_annotation_target_usage(ann, AnnotationTargetKind::Module, span)?;
        }

        let module_path = self.current_module_path_for(&module_def.name);
        if let Some(parent_path) = self.module_scope_stack.last().cloned()
            && let Some(parent_source) = self.resolve_canonical_module_path(&parent_path)
        {
            self.module_scope_sources
                .entry(module_path.clone())
                .or_insert_with(|| format!("{}::{}", parent_source, module_def.name));
        }
        self.module_scope_stack.push(module_path.clone());
        self.push_module_reference_scope();

        let mut module_items = module_def.items.clone();
        if self.execute_module_comptime_handlers(module_def, &module_path, &mut module_items)? {
            self.pop_module_reference_scope();
            self.module_scope_stack.pop();
            return Ok(());
        }
        if self.execute_module_inline_comptime_blocks(&module_path, &mut module_items)? {
            self.pop_module_reference_scope();
            self.module_scope_stack.pop();
            return Ok(());
        }

        let mut qualified_items = Vec::with_capacity(module_items.len());
        for inner in &module_items {
            qualified_items.push(self.qualify_module_item(inner, &module_path)?);
        }

        for qualified in &qualified_items {
            self.register_missing_module_items(qualified)?;
        }

        self.non_function_mir_context_stack
            .push(module_path.clone());
        let compile_result = (|| -> Result<()> {
            for (idx, qualified) in qualified_items.iter().enumerate() {
                let future_names = self
                    .future_reference_use_names_for_remaining_items(&qualified_items[idx + 1..]);
                self.push_future_reference_use_names(future_names);
                let compile_result = self.compile_item_with_context(qualified, false);
                self.pop_future_reference_use_names();
                compile_result?;
                self.release_unused_module_reference_borrows_for_remaining_items(
                    &qualified_items[idx + 1..],
                );
            }
            Ok(())
        })();
        self.non_function_mir_context_stack.pop();
        compile_result?;

        let exports = self.collect_module_runtime_exports(&module_items, &module_path);
        let entries: Vec<ObjectEntry> = exports
            .into_iter()
            .map(|(name, value_ident)| ObjectEntry::Field {
                key: name,
                value: Expr::Identifier(value_ident, span),
                type_annotation: None,
            })
            .collect();
        let module_object = Expr::Object(entries, span);
        self.compile_expr(&module_object)?;

        let binding_idx = self.get_or_create_module_binding(&module_path);
        self.emit(Instruction::new(
            OpCode::StoreModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));
        // U4-4: the module-namespace object is a non-numeric heap value; its
        // tracker type comes from `last_expr_type_info` (no numeric expr).
        self.propagate_initializer_type_to_slot(binding_idx, false, false, Some(&module_object));

        if self.module_scope_stack.len() == 1 {
            self.module_namespace_bindings
                .insert(module_def.name.clone());
        }

        self.emit_annotation_lifecycle_calls_for_module(
            &module_path,
            &module_def.annotations,
            Some(binding_idx),
        )?;

        self.pop_module_reference_scope();
        self.module_scope_stack.pop();
        Ok(())
    }

    /// Compile a query (Backtest, Alert, or With/CTE).
    ///
    /// For CTE (WITH) queries:
    /// 1. Compile each CTE subquery and store the result in a named module_binding variable.
    /// 2. Compile the main query (which can reference CTEs by name as variables).
    ///
    /// For Backtest and Alert queries, emit a stub for now.
    fn compile_query(&mut self, query: &Query) -> Result<()> {
        match query {
            Query::With(with_query) => {
                // Compile each CTE: evaluate its subquery and store as a named variable
                for cte in &with_query.ctes {
                    // Recursively compile the CTE's subquery
                    self.compile_query(&cte.query)?;

                    // Store the result in a module_binding variable with the CTE's name
                    let binding_idx = self.get_or_create_module_binding(&cte.name);
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }

                // Compile the main query
                self.compile_query(&with_query.query)?;
            }
            Query::Backtest(_backtest) => {
                // Backtest queries require runtime context to evaluate.
                // Push null as placeholder — the runtime executor handles backtest
                // execution when given a full ExecutionContext.
                self.emit(Instruction::simple(OpCode::PushNull));
            }
            Query::Alert(alert) => {
                // Compile alert condition
                self.compile_expr(&alert.condition)?;
                // Push null as placeholder (alert evaluation requires runtime context)
                self.emit(Instruction::simple(OpCode::Pop));
                self.emit(Instruction::simple(OpCode::PushNull));
            }
        }
        Ok(())
    }

    pub(super) fn propagate_initializer_type_to_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        _is_mutable: bool,
        // U4-4: the initializer expression — its resolved Type drives the
        // numeric slot hint (`numeric_type_of`), replacing the deleted
        // `last_expr_numeric_type` register. `None` where no single init expr
        // is in hand (falls back to `last_expr_type_info`).
        init_expr: Option<&shape_ast::ast::Expr>,
    ) {
        self.propagate_assignment_type_to_slot(slot, is_local, true, init_expr);
    }

    pub(super) fn typed_set_ctor_for_annotation(
        &self,
        type_ann: &TypeAnnotation,
    ) -> Option<crate::bytecode::BuiltinFunction> {
        let ct =
            crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                self, type_ann,
            )?;
        self.typed_set_ctor_for_concrete_type(&ct)
    }

    pub(super) fn typed_set_ctor_for_concrete_type(
        &self,
        ct: &shape_value::v2::ConcreteType,
    ) -> Option<crate::bytecode::BuiltinFunction> {
        let shape_value::v2::ConcreteType::HashSet(elem) = ct else {
            return None;
        };
        match elem.as_ref() {
            shape_value::v2::ConcreteType::String => {
                Some(crate::bytecode::BuiltinFunction::SetCtorString)
            }
            shape_value::v2::ConcreteType::I64 => {
                Some(crate::bytecode::BuiltinFunction::SetCtorI64)
            }
            _ => None,
        }
    }

    pub(super) fn typed_set_ctor_for_call_span(
        &self,
        span: Span,
    ) -> Option<crate::bytecode::BuiltinFunction> {
        if !span.is_dummy()
            && let Some(ct) = self
                .resolved_expr_types
                .get(&span)
                .or_else(|| self.inference_facts.expression_type(span))
                .and_then(|ty| {
                    crate::compiler::monomorphization::type_resolution::concrete_type_from_inference_fact(
                        self, ty,
                    )
                })
            && let Some(typed_ctor) = self.typed_set_ctor_for_concrete_type(&ct)
        {
            return Some(typed_ctor);
        }

        self.pending_expected_call_return_type
            .as_ref()
            .and_then(|ann| self.typed_set_ctor_for_annotation(ann))
    }

    pub(super) fn patch_static_set_ctor_from_annotation(
        &mut self,
        init_expr: Option<&Expr>,
        type_ann: Option<&TypeAnnotation>,
    ) {
        let Some(Expr::FunctionCall {
            name,
            args,
            named_args,
            ..
        }) = init_expr
        else {
            return;
        };
        if name != "Set" || !args.is_empty() || !named_args.is_empty() {
            return;
        }
        let Some(type_ann) = type_ann else {
            return;
        };
        let Some(typed_ctor) = self.typed_set_ctor_for_annotation(type_ann) else {
            return;
        };
        if let Some(last) = self.program.instructions.last_mut()
            && last.opcode == OpCode::BuiltinCall
            && matches!(
                last.operand,
                Some(Operand::Builtin(crate::bytecode::BuiltinFunction::SetCtor))
            )
        {
            last.operand = Some(Operand::Builtin(typed_ctor));
        }
    }

    /// Compile a statement
    pub(super) fn compile_statement(&mut self, stmt: &Statement) -> Result<()> {
        match stmt {
            Statement::Return(expr_opt, _span) => {
                if let Some(expr) = expr_opt {
                    // ADR-006 §2.7.30 (FlipLive): the `return &local` floor
                    // promotion is admitted ONLY under a `&T` return contract.
                    // An UNANNOTATED `return &local` (no `-> &T`, and not a
                    // safe param-reborrow which sets the return-reference
                    // summary) does NOT build a sound PromotedCell carrier on
                    // the return path — the raw ref bits would escape without
                    // an owning carrier (dangling ref / UAF). Keep rejecting it
                    // with B0003. The annotated `-> &T` form (carrier built)
                    // and the param-reborrow form (summary set) both fall
                    // through and compile.
                    if matches!(expr, shape_ast::ast::Expr::Reference { .. })
                        && self.current_function_return_reference_summary.is_none()
                        && !self.current_function_returns_borrow
                    {
                        return Err(ShapeError::SemanticError {
                            message:
                                "[B0003] cannot return or store a reference that outlives its owner"
                                    .to_string(),
                            location: Some(self.span_to_source_location(expr.span())),
                        });
                    }
                    self.plan_flexible_binding_escape_from_expr(expr);
                    // Phase F: when the returned expression is a closure
                    // literal, the closure escapes by definition (it is
                    // about to cross the return boundary). Flag the next
                    // closure emission to use the heap ABI opcode so the
                    // JIT and future Phase H cleanup can rely on a stable
                    // signal. Matches the escape vector in
                    // `docs/v2-closure-specialization.md` §2.1 row 1.
                    if matches!(expr, Expr::FunctionExpr { .. }) {
                        self.emit_make_closure_heap_next = true;
                    }
                    // Numeric-conversion §4 literal adoption (explicit-return
                    // widening, THE RULE user 2026-06-01): a bare int literal
                    // `return`ed into a `number` return type IS the number
                    // literal (`fn g() -> number { return 5 }` ⇒ `5.0`). Re-lower
                    // it so the return slot is Float64-kinded, not an Int64
                    // constant bit-reinterpreted as f64 at the call site.
                    let return_widened =
                        self.current_function_return_type.as_ref().and_then(|ann| {
                            crate::compiler::literal_widen::widen_int_literal_for_annotation(
                                expr, ann,
                            )
                        });
                    let return_expr: &Expr = return_widened.as_ref().unwrap_or(expr);
                    let saved_pending_callable_hint_name = self.pending_callable_hint_name.clone();
                    self.pending_callable_hint_name =
                        self.callable_return_hint_name_for_expr(return_expr);
                    let return_type_annotation = self.current_function_return_type.clone();
                    let saved_expected_call_return_type =
                        self.pending_expected_call_return_type.clone();
                    self.pending_expected_call_return_type = return_type_annotation.clone();
                    let compile_result = if self.current_function_return_reference_summary.is_some()
                    {
                        self.compile_expr_preserving_refs(return_expr)
                    } else {
                        self.compile_expr(return_expr)
                    };
                    self.pending_expected_call_return_type = saved_expected_call_return_type;
                    self.pending_callable_hint_name = saved_pending_callable_hint_name;
                    compile_result?;
                    self.patch_static_set_ctor_from_annotation(
                        Some(return_expr),
                        return_type_annotation.as_ref(),
                    );
                } else {
                    self.emit(Instruction::simple(OpCode::PushNull));
                }
                // ADR-006 §2.7.30 (escape-Drop-deferral): when the returned
                // expression is a bare identifier naming a Drop-bearing
                // local, that local's value is MOVED to the caller. Its
                // `Drop` must defer to the caller's lifetime — emitting a
                // `DropCall` for it here would run the user `Drop::drop` body
                // a second time (the caller drops it again at its binding's
                // scope exit) — the bind-then-return double-drop. The
                // `LoadLocal` clone (above) + the frame-pop `truncate_stack`
                // slot-release already balance the refcount; only the
                // spurious `DropCall` needs suppressing. We scope the skip to
                // exactly this return's drop-emission.
                if let Some(Expr::Identifier(name, _)) = expr_opt {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.local_drop_kind(local_idx).is_some() {
                            self.return_escape_drop_skip_local = Some(local_idx);
                        }
                    }
                }
                // Emit drops for all active drop scopes before returning
                let total_scopes = self.drop_locals.len();
                if total_scopes > 0 {
                    self.emit_drops_for_early_exit(total_scopes)?;
                }
                self.return_escape_drop_skip_local = None;
                self.emit_return_value_with_ownership(expr_opt.as_ref())?;
            }

            Statement::Break(_) => {
                let in_loop = !self.loop_stack.is_empty();
                if in_loop {
                    // Emit drops for drop scopes inside the loop before breaking
                    let scopes_to_exit = self
                        .loop_stack
                        .last()
                        .map(|ctx| self.drop_locals.len().saturating_sub(ctx.drop_scope_depth))
                        .unwrap_or(0);
                    if scopes_to_exit > 0 {
                        self.emit_drops_for_early_exit(scopes_to_exit)?;
                    }
                    let jump_idx = self.emit_jump(OpCode::Jump, 0);
                    if let Some(loop_ctx) = self.loop_stack.last_mut() {
                        loop_ctx.break_jumps.push(jump_idx);
                    }
                } else {
                    return Err(ShapeError::RuntimeError {
                        message: "break statement outside of loop".to_string(),
                        location: None,
                    });
                }
            }

            Statement::Continue(_) => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    // Copy values we need before mutable borrow
                    let scopes_to_exit = self
                        .drop_locals
                        .len()
                        .saturating_sub(loop_ctx.drop_scope_depth);
                    let continue_target = loop_ctx.continue_target;
                    // Emit drops for drop scopes inside the loop before continuing
                    if scopes_to_exit > 0 {
                        self.emit_drops_for_early_exit(scopes_to_exit)?;
                    }
                    if continue_target == usize::MAX {
                        // Deferred continue: emit placeholder forward jump
                        let jump_idx = self.emit_jump(OpCode::Jump, 0);
                        if let Some(loop_ctx) = self.loop_stack.last_mut() {
                            loop_ctx.continue_jumps.push(jump_idx);
                        }
                    } else {
                        let offset =
                            continue_target as i32 - self.program.current_offset() as i32 - 1;
                        self.emit(Instruction::new(
                            OpCode::Jump,
                            Some(Operand::Offset(offset)),
                        ));
                    }
                } else {
                    return Err(ShapeError::RuntimeError {
                        message: "continue statement outside of loop".to_string(),
                        location: None,
                    });
                }
            }

            Statement::VariableDecl(var_decl, _) => {
                // Numeric-conversion §4 literal adoption (let-annotation widening,
                // THE RULE user 2026-06-01): when the binding carries an explicit
                // `number`/`f64` annotation and the initializer is a bare int
                // literal, the literal IS the number literal (`let n: number = 5`
                // ⇒ `5.0`). Rewrite the init node to a `Number` literal BEFORE any
                // sub-path reads `var_decl.value`, so every downstream emission
                // pushes a `Constant::Number` (Float64-kinded slot) instead of an
                // Int64 constant laid into a Float64-stamped slot (the
                // bit-reinterpret hole: `n / 2` int-dividing → `2`). Compile-time
                // literal re-typing, NOT a runtime coercion opcode (no W4-δ Convert
                // defection). A non-literal `int` value is NOT rewritten — the
                // p-var `int`-is-not-`number` rejection stays a compile error.
                let widened_decl;
                let var_decl = match (&var_decl.type_annotation, &var_decl.value) {
                    (Some(ann), Some(value))
                        if crate::compiler::literal_widen::widen_int_literal_for_annotation(
                            value, ann,
                        )
                        .is_some() =>
                    {
                        let mut clone = var_decl.clone();
                        clone.value =
                            crate::compiler::literal_widen::widen_int_literal_for_annotation(
                                value, ann,
                            );
                        widened_decl = clone;
                        &widened_decl
                    }
                    _ => var_decl,
                };
                // Set pending variable name for hoisting integration.
                // compile_typed_object_literal uses self to include hoisted fields in the schema.
                self.pending_variable_name =
                    var_decl.pattern.as_identifier().map(|s| s.to_string());
                self.pending_variable_span = var_decl.pattern.as_identifier_span();
                // v2 Phase 3.1 (Agent 3): when the binding has an explicit
                // `Array<T>` annotation whose element type maps to a
                // typed-array kind, signal it to `compile_expr_array` so
                // the literal lowers to the v2 typed-array path.
                //
                // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element
                // (2026-05-18): route through the compiler-aware
                // `resolve_typed_array_kind_from_annotation` so `Array<B>`
                // for a registered user struct B also maps to
                // `TypedArrayKind::TypedObject` per audit §2.1 + §3.A row 1.
                self.pending_variable_typed_array_kind = var_decl
                    .type_annotation
                    .as_ref()
                    .and_then(|ann| self.resolve_typed_array_kind_and_record_trait(ann));
                // U3 (SB-9 deletion): no typed-map carrier selection. Every
                // `HashMap<K, V>` binding uses the single honest `HashMapData`
                // carrier; there is no `pending_variable_typed_map_kind` to set.

                // Compile-time range check: if the type annotation is a width type
                // (i8, u8, i16, etc.) and the initializer is a constant expression,
                // verify the value fits in the declared width.
                if let (Some(type_ann), Some(init_expr)) =
                    (&var_decl.type_annotation, &var_decl.value)
                {
                    if let shape_ast::ast::TypeAnnotation::Basic(type_name) = type_ann {
                        if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                            // Const-fold path is dormant per ADR-006 §2.4
                            // — `ConstFoldValue` is an uninhabited
                            // placeholder until the phase-2c carrier
                            // shape lands. `eval_const_expr_to_nanboxed`
                            // returns `Option<ConstFoldValue>` whose
                            // `Some` arm is statically unreachable;
                            // matching on the empty enum here keeps the
                            // caller compile-clean while preserving the
                            // surface for the phase-2c rebuild.
                            let _ = (w, type_name);
                            if let Some(const_val) =
                                crate::compiler::expressions::function_calls::eval_const_expr_to_nanboxed(init_expr)
                            {
                                match const_val {}
                            }
                        }
                    }
                }

                // Compile initializer — register the variable even if the initializer fails,
                // to prevent cascading "Undefined variable" errors on later references.
                let mut ref_borrow = None;
                let init_err = if let Some(init_expr) = &var_decl.value {
                    // Special handling: Table row literal syntax
                    // `let t: Table<T> = [a, b], [c, d]` → compile as table construction
                    if let Expr::TableRows(rows, tr_span) = init_expr {
                        match self.compile_table_rows(rows, &var_decl.type_annotation, *tr_span) {
                            Ok(()) => None,
                            Err(e) => {
                                self.emit(Instruction::simple(OpCode::PushNull));
                                Some(e)
                            }
                        }
                    } else if let Expr::Array(items, arr_span) = init_expr {
                        // Single-row table literal: `let t: Table<T> = [a, b, c]`
                        // When the annotation is Table<T>, treat the array as a single row.
                        let is_table_annotated = matches!(
                            &var_decl.type_annotation,
                            Some(shape_ast::ast::TypeAnnotation::Generic { name, args })
                                if name == "Table" && args.len() == 1
                        );
                        if is_table_annotated {
                            let single_row = vec![items.clone()];
                            match self.compile_table_rows(
                                &single_row,
                                &var_decl.type_annotation,
                                *arr_span,
                            ) {
                                Ok(()) => None,
                                Err(e) => {
                                    self.emit(Instruction::simple(OpCode::PushNull));
                                    Some(e)
                                }
                            }
                        } else {
                            match self.compile_expr_for_reference_binding_with_expected_return(
                                init_expr,
                                var_decl.type_annotation.as_ref(),
                            ) {
                                Ok(tracked_borrow) => {
                                    ref_borrow = tracked_borrow;
                                    None
                                }
                                Err(e) => {
                                    self.emit(Instruction::simple(OpCode::PushNull));
                                    Some(e)
                                }
                            }
                        }
                    } else {
                        match self.compile_expr_for_reference_binding_with_expected_return(
                            init_expr,
                            var_decl.type_annotation.as_ref(),
                        ) {
                            Ok(tracked_borrow) => {
                                ref_borrow = tracked_borrow;
                                None
                            }
                            Err(e) => {
                                self.emit(Instruction::simple(OpCode::PushNull));
                                Some(e)
                            }
                        }
                    }
                } else {
                    self.emit(Instruction::simple(OpCode::PushNull));
                    None
                };
                if init_err.is_none() {
                    self.patch_static_set_ctor_from_annotation(
                        var_decl.value.as_ref(),
                        var_decl.type_annotation.as_ref(),
                    );
                }

                // Capture (then clear) pending variable name and v2 typed
                // array kind after the init expression is compiled. The
                // captured kind is recorded against the binding slot below
                // so subsequent typed Get/Set/Push opcode emission can
                // verify the receiver is actually a v2 typed array.
                self.pending_variable_name = None;
                self.pending_variable_span = None;
                let mut captured_typed_array_kind = self.pending_variable_typed_array_kind;
                self.pending_variable_typed_array_kind = None;
                // Kind-changing-map carrier reconciliation (2026-06-15):
                // `pending_variable_typed_array_kind` may have leaked from
                // compiling a SUB-expression of the initializer (the receiver
                // array literal of `<arr>.map(closure)`), stamping the INPUT
                // element kind onto a binding whose RESULT carrier is the
                // closure-return kind. Re-derive the binding's authoritative
                // carrier kind from its PROVEN element type. `Some(Some(k))`
                // overrides with the proven scalar carrier; `Some(None)`
                // suppresses the stale stamp (heap/uncarriered element → the
                // carrier-reading GetProp path); `None` leaves the capture
                // untouched (non-array binding). Per ADR-006 §2.7.5 the
                // inference engine's element type is the proof — never a
                // bit-reinterpret of the input carrier.
                // Numeric-conversion §4 literal adoption (typed-array binding
                // kind, THE RULE user 2026-06-01): an EXPLICIT `Array<number>`
                // annotation is AUTHORITATIVE for the element carrier kind. The
                // `reconcile_binding_typed_array_kind` re-inference walks the
                // literal `[1, 2, 3]` and the inference engine reports its
                // natural element type `int` (it does not see the annotation
                // context), which would override the annotation's `F64` carrier
                // with `I64` — and then `a[0]` emits `TypedArrayGetI64` against an
                // array whose elements were stored as f64 (the elements adopted
                // `number` per the per-element widen at
                // `collections.rs:1849`), reinterpreting the f64 bits as i64
                // (`1.0` → `4607182418800017408`). When the annotation already
                // proved a carrier kind, trust it; only consult the literal
                // re-inference for the UNANNOTATED binding (`let a = [1, 2, 3]`).
                let annotation_proved_array_kind = var_decl
                    .type_annotation
                    .as_ref()
                    .and_then(|ann| self.resolve_typed_array_kind_from_annotation(ann))
                    .is_some();
                if !annotation_proved_array_kind {
                    if let Some(init_expr) = var_decl.value.as_ref() {
                        if let Some(reconciled) = self.reconcile_binding_typed_array_kind(init_expr)
                        {
                            captured_typed_array_kind = reconciled;
                        }
                    }
                }
                // Phase 4b Round 6 WS-1b W16.2-C residual: capture the bare
                // empty-array-accumulator placeholder index alongside the
                // other initializer-derived signals.
                let captured_empty_array_alloc_idx = self.pending_empty_array_alloc_idx.take();

                // ADR-006 §2.7.24 Q25.C: coerce-to-dyn emission. When the
                // binding's annotation is `TypeAnnotation::Dyn(traits)`,
                // emit `OpCode::BoxTraitObject` after the RHS has been
                // pushed onto the stack. The opcode pops the concrete
                // value, looks up `(concrete_type, trait_name)` in the
                // program's `trait_vtables` registry, allocates a
                // `TraitObjectStorage`, and pushes back a kinded
                // `Ptr(HeapKind::TraitObject)` slot.
                //
                // The operand is the trait name as a `Operand::Name(StringId)`
                // — the executor resolves the trait name → vtable lookup
                // via the receiver's concrete type. Multi-trait dyn
                // (`dyn A + B + C`) uses the FIRST trait as the boxing
                // discriminator per §Q25.C.5 `trait_names` field;
                // wider dispatch through additional traits is a future
                // amendment.
                let is_dyn_coerce = init_err.is_none()
                    && var_decl
                        .type_annotation
                        .as_ref()
                        .map(|ann| {
                            crate::compiler::trait_object_emission::trait_name_from_annotation(ann)
                                .is_some()
                        })
                        .unwrap_or(false);
                if is_dyn_coerce {
                    if let Some(trait_name) = var_decl.type_annotation.as_ref().and_then(
                        crate::compiler::trait_object_emission::trait_name_from_annotation,
                    ) {
                        let sid = self.program.add_string(trait_name.to_string());
                        self.emit(Instruction::new(
                            OpCode::BoxTraitObject,
                            Some(Operand::Name(shape_value::StringId(sid as u32))),
                        ));
                    }
                }

                // Emit BindSchema for Table<T> annotations (runtime safety net)
                if let Some(ref type_ann) = var_decl.type_annotation {
                    if let Some(schema_id) = self.get_table_schema_id(type_ann) {
                        self.emit(Instruction::new(
                            OpCode::BindSchema,
                            Some(Operand::Count(schema_id)),
                        ));
                    }
                }

                // At top-level (no current function), create module_bindings; otherwise create locals
                if self.current_function.is_none() {
                    // Top-level: create module_binding variable
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        // v0.3.3 c6 (Wave 1): re-add narrow B0003 guard for
                        // module-scope `let r = &x` at the `Statement::VariableDecl`
                        // path (Item::VariableDecl is handled at the matching
                        // earlier site). Module-level top-level statements
                        // are NOT lowered to MIR — the MIR solver runs per
                        // function only — so without this guard a top-level
                        // `let r = &x` silently runs (SEGFAULTs on use in
                        // some shapes). Per audit
                        // `docs/cluster-audits/v0.3.3/06-borrow-check-bypass.md`
                        // §5(a). Defense-in-depth alongside the MIR
                        // solver's new `LoanSinkKind::ModuleBindingStore`
                        // (which catches in-function `module_g = &local`).
                        // The R8 W9 B9 deletion (commit 8bbd2f99) removed
                        // this on the basis that "MIR is the sole
                        // authority" — but MIR never sees module-scope
                        // statements.
                        //
                        // ADR-006 §2.7.30 (FlipLive): flip EXACTLY the
                        // `ModuleBindingStore` floor sink — a top-level
                        // `let r = &x` where `x` is a program-lifetime module
                        // binding. Same scoping predicate as the
                        // `Item::VariableDecl` sites; non-floor escapes
                        // (referent rooted at a local) still reject.
                        let referent_is_module_floor = var_decl
                            .value
                            .as_ref()
                            .is_some_and(|expr| self.reference_root_is_module_binding(expr));
                        if ref_borrow.is_some() && !referent_is_module_floor {
                            return Err(ShapeError::SemanticError {
                                message:
                                    "[B0003] cannot return or store a reference that outlives its owner"
                                        .to_string(),
                                location: var_decl.value.as_ref().map(|expr| {
                                    self.span_to_source_location(expr.span())
                                }),
                            });
                        }
                        let binding_idx = self.get_or_create_module_binding(name);
                        if let Some(span) = var_decl.pattern.as_identifier_span()
                            && !span.is_dummy()
                        {
                            self.module_binding_spans.insert(binding_idx, span);
                        }

                        // U4-6a: the former `record_binding_object_element_fields`
                        // call is deleted with the side-table; `for {x,y} in
                        // points` now resolves the element object's field
                        // annotations via the inference engine span-table
                        // (`infer_expr_type` in `anonymous_object_element_fields`).

                        // Emit StoreModuleBindingTyped for width-typed bindings,
                        // otherwise emit regular StoreModuleBinding.
                        let used_typed_store = if let Some(TypeAnnotation::Basic(type_name)) =
                            var_decl.type_annotation.as_ref()
                        {
                            if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                                self.emit(Instruction::new(
                                    OpCode::StoreModuleBindingTyped,
                                    Some(Operand::TypedModuleBinding(
                                        binding_idx,
                                        crate::bytecode::NumericWidth::from_int_width(w),
                                    )),
                                ));
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        };
                        if !used_typed_store {
                            self.emit(Instruction::new(
                                OpCode::StoreModuleBinding,
                                Some(Operand::ModuleBinding(binding_idx)),
                            ));
                        }

                        // v2 Phase 3.1 (Agent 3): record v2 typed array kind for this binding
                        if let Some(kind) = captured_typed_array_kind {
                            self.v2_typed_array_module_bindings
                                .insert(binding_idx, kind);
                        }
                        // Phase 4b Round 6 WS-1b W16.2-C residual: re-key a
                        // bare empty-array-accumulator placeholder against
                        // this module binding (top-level `Statement::VarDecl`).
                        if let Some(name) = var_decl.pattern.as_identifier() {
                            self.register_empty_array_accumulator(
                                crate::compiler::EmptyArrayAccumulatorKey::ModuleBinding(
                                    binding_idx,
                                ),
                                var_decl.value.as_ref(),
                                captured_empty_array_alloc_idx,
                                name,
                                var_decl.value.as_ref().map(|v| v.span()),
                            );
                        }
                        // ADR-006 §2.7.27 / Item 4 ruling: transfer the
                        // pending container-kind signal to the module
                        // binding for write-back-aware method dispatch.
                        if let Some(ckind) = self.pending_variable_container_kind.take() {
                            self.mut_self_container_bindings.insert(binding_idx, ckind);
                        }
                        // ADR-006 §2.7.24 Q25.C: record dyn-typed module
                        // binding so subsequent `a.method()` calls emit
                        // `OpCode::DynMethodCall` instead of the standard
                        // `OpCode::CallMethod` path.
                        if let Some(trait_name) = var_decl.type_annotation.as_ref().and_then(
                            crate::compiler::trait_object_emission::trait_name_from_annotation,
                        ) {
                            self.dyn_module_bindings
                                .insert(binding_idx, trait_name.to_string());
                        }

                        // Track type annotation if present (for type checker)
                        if let Some(ref type_ann) = var_decl.type_annotation {
                            // strict-flip (map/collect OUTPUT element-type
                            // stamp): reject `let r: Array<number> =
                            // [1,2,3].map(|x| x*2)` — the map output element is
                            // the closure RETURN type (`int`), so the result is
                            // `Array<int>`, which must NOT coerce to
                            // `Array<number>`. Runs BEFORE the annotation drives
                            // the slot stamp below, so a stale `Float64` stamp
                            // never reaches an `Int64` carrier.
                            if let Some(init_expr) = var_decl.value.as_ref() {
                                self.check_let_annotation_element_type_strict(type_ann, init_expr)?;
                                self.check_let_annotation_scalar_unknown_strict(
                                    type_ann, init_expr,
                                )?;
                            }
                            if let Some(type_name) =
                                Self::tracked_type_name_from_annotation(type_ann)
                            {
                                self.set_module_binding_type_info(binding_idx, &type_name);
                            }
                            // v0.3 WS-6: record the binding's concrete type
                            // from its explicit annotation so a later generic
                            // call site `id(n)` can resolve the argument's
                            // type. The type-tracker only retains a lossy
                            // head-name string (e.g. "option" with no inner
                            // type); the explicit binding fact carries the
                            // full ConcreteType.
                            if let Some(ct) = crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(self, type_ann) {
                                crate::compiler::monomorphization::type_resolution::record_binding_concrete_fact(
                                    self,
                                    crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::ModuleBinding(binding_idx),
                                    ct,
                                    crate::compiler::BindingConcreteFactSource::DeclaredAnnotation,
                                );
                            }
                            // Handle Table<T> generic annotation
                            self.try_track_datatable_type(type_ann, binding_idx, false)?;
                        } else {
                            let is_mutable = var_decl.kind == shape_ast::ast::VarKind::Var;
                            self.propagate_initializer_type_to_slot(
                                binding_idx,
                                false,
                                is_mutable,
                                var_decl.value.as_ref(),
                            );
                            // v0.3 WS-6b GAP A: an *inferred-type* `let p =
                            // <expr>` carries no annotation, so the WS-6
                            // annotated-only recording above never runs. Resolve
                            // the binding's ConcreteType structurally from the
                            // initializer expression (struct literal, enum
                            // constructor, `Some`/`Ok`/`Err`, …) so a later
                            // generic call site `id(p)` can bind its type
                            // argument. `concrete_type_for_expr` returns `None`
                            // for a genuinely type-ambiguous initializer (e.g.
                            // `let n = None`), in which case nothing is recorded
                            // and `id(n)` stays a clean compile error.
                            if let Some(init_expr) = var_decl.value.as_ref() {
                                if let Some(ct) = crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(self, init_expr) {
                                    // ROOT-1 (strict-flip, 2026-06-18): a derived-read
                                    // element type must also reach the type_tracker NAME,
                                    // not only the `*_concrete_types` side-table. Field
                                    // access (`p.age`) and `iter_element_type_name`
                                    // (`for p in ps`) read the tracker NAME, not the
                                    // ConcreteType table — so an inferred
                                    // `let ps = [R{..}]` (struct/array-of-struct, no
                                    // annotation) previously left `ps` named `unknown`
                                    // and `p.age + 10` failed to infer. Stamp the
                                    // tracker name from the proven ConcreteType (the
                                    // ConcreteType IS the proof, ADR-006 §2.7.5 — no
                                    // fabrication; a shape with no stable tracker name
                                    // records nothing). Mirror of the annotated path's
                                    // `set_module_binding_type_info`.
                                    if let Some(tn) = crate::compiler::patterns::binding::concrete_type_tracker_name(&ct) {
                                        // Do not downgrade an already-stamped
                                        // monomorphized schema (`Box<int>`) to the
                                        // base name (`Box`) — see
                                        // `ws6b_name_would_downgrade` (ADR-006 §2.7.5).
                                        let existing = self
                                            .type_tracker
                                            .get_binding_type(binding_idx);
                                        if !Self::ws6b_name_would_downgrade(existing, &tn) {
                                            self.set_module_binding_type_info(binding_idx, &tn);
                                        }
                                    }
                                    crate::compiler::monomorphization::type_resolution::record_binding_concrete_fact(
                                        self,
                                        crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::ModuleBinding(binding_idx),
                                        ct,
                                        crate::compiler::BindingConcreteFactSource::StructuralInitializer,
                                    );
                                }
                            }
                        }

                        // U4-6 post-monomorphization call-site return fact:
                        // after initializer propagation, stamp a module binding
                        // from the specialized method-call return recorded at
                        // `(init_span, current_function)`.
                        if let Some(init_expr) = var_decl.value.as_ref() {
                            crate::compiler::monomorphization::type_resolution::stamp_binding_initializer_monomorphized_call_return(
                                self,
                                crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::ModuleBinding(binding_idx),
                                init_expr,
                            );
                        }

                        // Track for auto-drop at program exit
                        let binding_type_name = self
                            .type_tracker
                            .get_binding_type(binding_idx)
                            .and_then(|info| info.type_name.clone());
                        let drop_kind = binding_type_name
                            .as_ref()
                            .and_then(|tn| self.drop_type_info.get(tn).copied())
                            .or_else(|| {
                                var_decl
                                    .type_annotation
                                    .as_ref()
                                    .and_then(|ann| self.annotation_drop_kind(ann))
                            });
                        if drop_kind.is_some() {
                            let is_async = match drop_kind {
                                Some(DropKind::AsyncOnly) => true,
                                Some(DropKind::Both) => false,
                                Some(DropKind::SyncOnly) | None => false,
                            };
                            self.track_drop_module_binding(binding_idx, is_async);
                        }
                        if let Some(value) = &var_decl.value {
                            self.finish_reference_binding_from_expr(
                                binding_idx,
                                false,
                                name,
                                value,
                                ref_borrow,
                            );
                            self.update_callable_binding_from_expr(binding_idx, false, value);
                        } else {
                            self.clear_reference_binding(binding_idx, false);
                            self.clear_callable_binding(binding_idx, false);
                        }
                    } else {
                        self.compile_destructure_pattern_global(&var_decl.pattern)?;
                    }

                    for (binding_name, _) in var_decl.pattern.get_bindings() {
                        let scoped_name = self
                            .resolve_scoped_module_binding_name(&binding_name)
                            .unwrap_or(binding_name);
                        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                            if var_decl.kind == VarKind::Const {
                                self.const_module_bindings.insert(binding_idx);
                            }
                            if var_decl.kind == VarKind::Let && !var_decl.is_mut {
                                self.immutable_module_bindings.insert(binding_idx);
                            }
                        }
                    }
                    self.apply_binding_semantics_to_pattern_bindings(
                        &var_decl.pattern,
                        false,
                        Self::binding_semantics_for_var_decl(var_decl),
                    );
                    self.plan_flexible_binding_storage_for_pattern_initializer(
                        &var_decl.pattern,
                        false,
                        var_decl.value.as_ref(),
                    );
                } else {
                    // Inside function: create local variable
                    self.compile_destructure_pattern(&var_decl.pattern)?;

                    // Patch StoreLocal → StoreLocalTyped for width-typed simple bindings.
                    // compile_destructure_pattern emits StoreLocal(idx) for Identifier patterns;
                    // we upgrade it here when the type annotation is a width type.
                    if let (Some(name), Some(TypeAnnotation::Basic(type_name))) = (
                        var_decl.pattern.as_identifier(),
                        var_decl.type_annotation.as_ref(),
                    ) {
                        if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                            if let Some(local_idx) = self.resolve_local(name) {
                                if let Some(last) = self.program.instructions.last_mut() {
                                    if last.opcode == OpCode::StoreLocal {
                                        last.opcode = OpCode::StoreLocalTyped;
                                        last.operand = Some(Operand::TypedLocal(
                                            local_idx,
                                            crate::bytecode::NumericWidth::from_int_width(w),
                                        ));
                                    }
                                }
                            }
                        }
                    }

                    // Phase 3/4: Emit PromoteToOwned before StoreLocal for uniquely-owned
                    // bindings. This converts freshly-allocated Arc<HeapValue> (refcount 1)
                    // to Box<HeapValue>, eliminating atomic refcount overhead for the lifetime
                    // of the binding. Applies to:
                    //   - `let` (immutable) with Direct storage
                    //   - `let mut` (owned mutable) with Direct storage
                    //   - `const` with Direct storage
                    // Does NOT apply to `var` bindings, which stay Arc for shared mutability.
                    //
                    // Phase V1.3 (flag `SHAPE_V2_BOX_BY_DEFAULT`, default on): the
                    // predicate is extended to also cover `UniqueHeap` storage — the
                    // class `storage_planning.rs` rule 2 assigns when a `let`/`const`
                    // is mutably captured by a closure. Pre-V1.3 those bindings
                    // allocated as `Arc<HeapValue>` despite being uniquely owned by
                    // construction; the V1.2D `PromoteToShared` emission covers the
                    // escape vectors (Site A = escape into closure, Site B = SharedCow
                    // var write), so the non-escape case can safely switch to Box.
                    // When the flag is off the predicate reverts to Direct-only and
                    // emission is byte-identical to pre-V1.3.
                    //
                    // Phase 5.C: When the initializer was a call to a function whose
                    // return-ownership mode is `NewlyOwned`, the callee already emitted
                    // `ReturnOwned` and handed us a Box-backed value on the stack. In
                    // that case we skip the caller-side `PromoteToOwned` — inserting it
                    // would be a harmless no-op (the owned bit is already set), but
                    // emitting one extra opcode per binding across an entire pipeline
                    // is measurable, and the hint is free to consult.
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        let is_owned_binding =
                            var_decl.kind == VarKind::Let || var_decl.kind == VarKind::Const;
                        if is_owned_binding {
                            if let Some(local_idx) = self.resolve_local(name) {
                                let box_by_default = super::helpers::box_by_default_enabled();
                                let should_promote = self
                                    .mir_storage_class_for_slot(local_idx)
                                    .map_or(false, |sc| {
                                        matches!(
                                            sc,
                                            crate::type_tracking::BindingStorageClass::Direct
                                        ) || (box_by_default
                                            && matches!(
                                                sc,
                                                crate::type_tracking::BindingStorageClass::UniqueHeap
                                            ))
                                    });
                                // Compute the Phase 5.B hint directly from the
                                // initializer AST — the binding semantics aren't
                                // populated yet at this point in the let-statement
                                // pipeline (that happens a few lines below via
                                // `apply_binding_semantics_to_pattern_bindings`),
                                // so we can't read `return_ownership_hint` off the
                                // slot's semantics here.
                                let callee_already_owned = var_decl
                                    .value
                                    .as_ref()
                                    .and_then(|init| {
                                        self.return_ownership_hint_for_initializer(init)
                                    })
                                    .map_or(false, |hint| {
                                        hint == crate::mir::ReturnOwnershipMode::NewlyOwned
                                    });
                                if should_promote && !callee_already_owned {
                                    // The last instruction should be StoreLocal(local_idx).
                                    // Insert PromoteToOwned just before it.
                                    let instr_count = self.program.instructions.len();
                                    if instr_count > 0 {
                                        let last = self.program.instructions[instr_count - 1];
                                        if last.opcode == OpCode::StoreLocal {
                                            // Remove the StoreLocal, emit PromoteToOwned, re-emit StoreLocal.
                                            self.program.instructions.pop();
                                            self.emit(Instruction::simple(OpCode::PromoteToOwned));
                                            self.emit(last);
                                        }
                                    }
                                }

                                // ADR-006 §2.7.30 (R3): def-site cell allocation
                                // for a referent promoted by R2 because a
                                // reference escapes it via a flipped FLOOR sink
                                // (`return &x` / module-binding `let r = &x`).
                                // The borrow planner assigned `SharedCow` AND
                                // flagged the slot in the sink-discriminated
                                // promotion set; promote the just-stored value
                                // into an RC'd `SharedCell` here so `op_make_ref`
                                // builds an OWNING `PromotedCell` carrier that
                                // outlives this frame. Reuses the closures.rs
                                // `LoadLocal + AllocSharedLocal` sequence. The
                                // `shared_locals` membership makes subsequent
                                // reads of this binding route through the cell;
                                // the `shared_drop_locals` registration releases
                                // the def-site share at scope exit (R3 keep-alive
                                // is the ref's owning share, NOT this one).
                                if matches!(
                                    self.mir_storage_class_for_slot(local_idx),
                                    Some(crate::type_tracking::BindingStorageClass::SharedCow)
                                ) && self.slot_is_reference_escape_promotion(local_idx)
                                    && !self.shared_locals.contains(name)
                                {
                                    self.emit(Instruction::new(
                                        OpCode::LoadLocal,
                                        Some(Operand::Local(local_idx)),
                                    ));
                                    self.emit(Instruction::new(
                                        OpCode::AllocSharedLocal,
                                        Some(Operand::Local(local_idx)),
                                    ));
                                    self.shared_locals.insert(name.to_string());
                                    if let Some(scope) = self.shared_drop_locals.last_mut() {
                                        scope.push(local_idx);
                                    }
                                }
                            }
                        }
                    }

                    for (binding_name, _) in var_decl.pattern.get_bindings() {
                        if let Some(local_idx) = self.resolve_local(&binding_name) {
                            if var_decl.kind == VarKind::Const {
                                self.const_locals.insert(local_idx);
                            }
                            if var_decl.kind == VarKind::Let && !var_decl.is_mut {
                                self.immutable_locals.insert(local_idx);
                            }
                            // Track A.1C.3: record `let mut` locals so
                            // later closure capture classification has
                            // a persistent witness when the type-
                            // tracker local semantics get wiped by a
                            // sibling closure's `compile_function`.
                            if var_decl.kind == VarKind::Let && var_decl.is_mut {
                                self.owned_mutable_locals.insert(binding_name.clone());
                            }
                        }
                    }
                    self.apply_binding_semantics_to_pattern_bindings(
                        &var_decl.pattern,
                        true,
                        Self::binding_semantics_for_var_decl(var_decl),
                    );
                    // Phase 5.B: If the initializer is a call to a function whose
                    // return-ownership mode is known, record the hint on each
                    // pattern binding so Phase 5.C codegen can skip the Arc→Box
                    // PromoteToOwned round-trip.
                    if let Some(init) = var_decl.value.as_ref() {
                        if let Some(hint) = self.return_ownership_hint_for_initializer(init) {
                            self.apply_return_ownership_hint_to_pattern_bindings(
                                &var_decl.pattern,
                                true,
                                hint,
                            );
                        }
                    }
                    self.plan_flexible_binding_storage_for_pattern_initializer(
                        &var_decl.pattern,
                        true,
                        var_decl.value.as_ref(),
                    );

                    // Track type annotation first (so drop tracking can resolve the type)
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        if let Some(ref type_ann) = var_decl.type_annotation {
                            // strict-flip (map/collect OUTPUT element-type
                            // stamp): mirror of the module-binding path — reject
                            // `let r: Array<number> = [1,2,3].map(|x| x*2)`
                            // (and the `.iter().map(...).collect()` form). The
                            // map output element is the closure RETURN type, so
                            // an `Array<int>` result must NOT coerce to
                            // `Array<number>`. Runs before the annotation stamps
                            // the local slot kind.
                            if let Some(init_expr) = var_decl.value.as_ref() {
                                self.check_let_annotation_element_type_strict(type_ann, init_expr)?;
                                self.check_let_annotation_scalar_unknown_strict(
                                    type_ann, init_expr,
                                )?;
                            }
                            if let Some(type_name) =
                                Self::tracked_type_name_from_annotation(type_ann)
                            {
                                // Get the local index for self variable
                                if let Some(local_idx) = self.resolve_local(name) {
                                    self.set_local_type_info(local_idx, &type_name);
                                }
                            }
                            // v0.3 WS-6: record the local's concrete type from
                            // its explicit annotation so a later generic call
                            // site `id(n)` can resolve the argument's type.
                            // See the mirror module-binding path above.
                            if let Some(local_idx) = self.resolve_local(name) {
                                if let Some(ct) = crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(self, type_ann) {
                                    crate::compiler::monomorphization::type_resolution::record_binding_concrete_fact(
                                        self,
                                        crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::Local(local_idx),
                                        ct,
                                        crate::compiler::BindingConcreteFactSource::DeclaredAnnotation,
                                    );
                                }
                            }
                            // Handle Table<T> generic annotation
                            if let Some(local_idx) = self.resolve_local(name) {
                                self.try_track_datatable_type(type_ann, local_idx, true)?;
                            }
                            // ADR-006 §2.7.24 Q25.C: record dyn-typed
                            // local so subsequent `a.method()` calls
                            // route through `OpCode::DynMethodCall`.
                            if let Some(trait_name) =
                                crate::compiler::trait_object_emission::trait_name_from_annotation(
                                    type_ann,
                                )
                            {
                                if let Some(local_idx) = self.resolve_local(name) {
                                    self.dyn_locals.insert(local_idx, trait_name.to_string());
                                }
                            }
                        } else if let Some(local_idx) = self.resolve_local(name) {
                            let is_mutable = var_decl.kind == shape_ast::ast::VarKind::Var;
                            self.propagate_initializer_type_to_slot(
                                local_idx,
                                true,
                                is_mutable,
                                var_decl.value.as_ref(),
                            );
                            // v0.3 WS-6b GAP A: mirror of the inferred-type
                            // module-binding path above. An inferred `let p =
                            // <expr>` local carries no annotation, so the WS-6
                            // annotated-only binding fact recording never
                            // fires. Resolve the local's
                            // ConcreteType structurally from the initializer so a
                            // later generic call site `id(p)` (inside the same
                            // function) can bind its type argument. `None` for a
                            // type-ambiguous initializer records nothing — the
                            // clean-compile-error contract is preserved.
                            if let Some(init_expr) = var_decl.value.as_ref() {
                                if let Some(ct) = crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(self, init_expr) {
                                    // ROOT-1 (strict-flip, 2026-06-18): mirror of the
                                    // module-binding path above — stamp the type_tracker
                                    // NAME from the proven ConcreteType so a derived-read
                                    // struct/array-of-struct local (`let p = ps[0]`,
                                    // `let ps = [R{..}]`) is field-accessible and
                                    // for-iterable. The tracker NAME (not the ConcreteType
                                    // side-table) is what `p.age` / `iter_element_type_name`
                                    // consult. ConcreteType IS the proof (ADR-006 §2.7.5).
                                    if let Some(tn) = crate::compiler::patterns::binding::concrete_type_tracker_name(&ct) {
                                        // Do not downgrade an already-stamped
                                        // monomorphized schema (`Box<int>`) to the
                                        // base name (`Box`) — see
                                        // `ws6b_name_would_downgrade` (ADR-006 §2.7.5).
                                        let existing = self.type_tracker.get_local_type(local_idx);
                                        if !Self::ws6b_name_would_downgrade(existing, &tn) {
                                            self.set_local_type_info(local_idx, &tn);
                                        }
                                    }
                                    crate::compiler::monomorphization::type_resolution::record_binding_concrete_fact(
                                        self,
                                        crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::Local(local_idx),
                                        ct,
                                        crate::compiler::BindingConcreteFactSource::StructuralInitializer,
                                    );
                                }
                            }
                        }
                    }

                    // v2 Phase 3.1 (Agent 3): record v2 typed array kind for the local
                    if let Some(kind) = captured_typed_array_kind {
                        if let Some(name) = var_decl.pattern.as_identifier() {
                            if let Some(local_idx) = self.resolve_local(name) {
                                self.v2_typed_array_locals.insert(local_idx, kind);
                            }
                        }
                    }
                    // Phase 4b Round 6 WS-1b W16.2-C residual: re-key a bare
                    // empty-array-accumulator placeholder against this local
                    // slot so the first downstream `.push()` resolves its
                    // element kind and patches the allocator.
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        if let Some(local_idx) = self.resolve_local(name) {
                            self.register_empty_array_accumulator(
                                crate::compiler::EmptyArrayAccumulatorKey::Local(local_idx),
                                var_decl.value.as_ref(),
                                captured_empty_array_alloc_idx,
                                name,
                                var_decl.value.as_ref().map(|v| v.span()),
                            );
                        }
                        // U4-6a: the former `record_binding_object_element_fields`
                        // call (local `let pts = [{x:1,y:2}]`) is deleted with
                        // the side-table; `for {x,y} in pts` resolves the element
                        // object's field annotations via the inference engine
                        // span-table (`infer_expr_type`).
                    }
                    // U4-6 post-monomorphization call-site return fact: mirror
                    // the module-binding path for local let-bound method-chain
                    // intermediates.
                    if let Some(init_expr) = var_decl.value.as_ref() {
                        if let Some(name) = var_decl.pattern.as_identifier() {
                            if let Some(local_idx) = self.resolve_local(name) {
                                crate::compiler::monomorphization::type_resolution::stamp_binding_initializer_monomorphized_call_return(
                                    self,
                                    crate::compiler::monomorphization::type_resolution::BindingInitializerTarget::Local(local_idx),
                                    init_expr,
                                );
                            }
                        }
                    }

                    // ADR-006 §2.7.27 / Item 4 ruling: transfer the
                    // pending container-kind signal from the
                    // initializer ctor (`Set()` / `HashMap()` /
                    // `Deque()` / `PriorityQueue()`) onto the target
                    // local-slot so method-call dispatch can decide
                    // whether to emit `Dup; StoreLocal` write-back. The
                    // signal is consumed (taken) here so a later
                    // statement doesn't accidentally inherit it.
                    let captured_container_kind = self.pending_variable_container_kind.take();
                    if let Some(kind) = captured_container_kind {
                        if let Some(name) = var_decl.pattern.as_identifier() {
                            if let Some(local_idx) = self.resolve_local(name) {
                                self.mut_self_container_locals.insert(local_idx, kind);
                            }
                        }
                    }

                    // Track for auto-drop at scope exit (DropCall silently skips non-Drop types).
                    // Select sync vs async opcode based on the type's DropKind.
                    if let Some(name) = var_decl.pattern.as_identifier() {
                        if let Some(local_idx) = self.resolve_local(name) {
                            let drop_kind = self.local_drop_kind(local_idx).or_else(|| {
                                var_decl
                                    .type_annotation
                                    .as_ref()
                                    .and_then(|ann| self.annotation_drop_kind(ann))
                            });

                            let is_async = match drop_kind {
                                Some(DropKind::AsyncOnly) => {
                                    if !self.current_function_is_async {
                                        let tn = self
                                            .type_tracker
                                            .get_local_type(local_idx)
                                            .and_then(|info| info.type_name.clone())
                                            .unwrap_or_else(|| name.to_string());
                                        return Err(ShapeError::SemanticError {
                                            message: format!(
                                                "type '{}' has only an async drop() and cannot be used in a sync context; \
                                                 add a sync method drop(self) or use it inside an async function",
                                                tn
                                            ),
                                            location: None,
                                        });
                                    }
                                    true
                                }
                                Some(DropKind::Both) => self.current_function_is_async,
                                Some(DropKind::SyncOnly) | None => false,
                            };
                            self.track_drop_local(local_idx, is_async);
                            // Phase V1.1C: also track the slot for an
                            // ownership-aware `DropLocal` at scope exit
                            // when the binding is heap-backed (so releasing
                            // the Arc/Box actually does work). The
                            // `pop_drop_scope` emission is gated on
                            // `SHAPE_V2_OWNERSHIP_MOVES`, so this tracking
                            // is a no-op at the bytecode level when the
                            // flag is off.
                            //
                            // Heap-backed means one of:
                            //   * `UniqueHeap` storage class — owned Box
                            //     allocation per the Phase 4 spec;
                            //   * `Direct` storage class *for a let/const
                            //     of a heap type* — the `PromoteToOwned`
                            //     emission a few lines above boxed a heap
                            //     value into the slot (inline scalars
                            //     round-trip as a no-op and don't need
                            //     drops).
                            //
                            // We skip `SharedCow` (its own refcount path
                            // handles release), `Reference` (borrow, no
                            // ownership), and `LocalMutablePtr`
                            // (stack-resident).
                            if self.binding_slot_needs_ownership_drop(local_idx, var_decl.kind) {
                                self.track_ownership_drop_local(local_idx);
                            }
                            if let Some(value) = &var_decl.value {
                                self.finish_reference_binding_from_expr(
                                    local_idx, true, name, value, ref_borrow,
                                );
                                self.update_callable_binding_from_expr(local_idx, true, value);
                            } else {
                                self.clear_reference_binding(local_idx, true);
                                self.clear_callable_binding(local_idx, true);
                            }
                        }
                    }
                }

                if let Some(e) = init_err {
                    return Err(e);
                }
            }

            Statement::Assignment(assign, _) => 'assign: {
                // Check for const reassignment
                if let Some(name) = assign.pattern.as_identifier() {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if !self.current_binding_uses_mir_write_authority(true)
                            && self.const_locals.contains(&local_idx)
                        {
                            return Err(ShapeError::SemanticError {
                                message: format!("Cannot reassign const variable '{}'", name),
                                location: None,
                            });
                        }
                        // Check for immutable `let` reassignment
                        if !self.current_binding_uses_mir_write_authority(true)
                            && self.immutable_locals.contains(&local_idx)
                        {
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                                    name
                                ),
                                location: None,
                            });
                        }
                        self.check_write_allowed_in_current_context(
                            Self::borrow_key_for_local(local_idx),
                            None,
                        )
                        .map_err(|e| match e {
                            ShapeError::SemanticError { message, location } => {
                                let user_msg = message.replace(
                                    &format!("(slot {})", local_idx),
                                    &format!("'{}'", name),
                                );
                                ShapeError::SemanticError {
                                    message: user_msg,
                                    location,
                                }
                            }
                            other => other,
                        })?;
                    } else {
                        let scoped_name = self
                            .resolve_scoped_module_binding_name(name)
                            .unwrap_or_else(|| name.to_string());
                        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                            if !self.current_binding_uses_mir_write_authority(false)
                                && self.const_module_bindings.contains(&binding_idx)
                            {
                                return Err(ShapeError::SemanticError {
                                    message: format!("Cannot reassign const variable '{}'", name),
                                    location: None,
                                });
                            }
                            // Check for immutable `let` reassignment at module level
                            if !self.current_binding_uses_mir_write_authority(false)
                                && self.immutable_module_bindings.contains(&binding_idx)
                            {
                                return Err(ShapeError::SemanticError {
                                    message: format!(
                                        "Cannot reassign immutable variable '{}'. Use `let mut` or `var` for mutable bindings",
                                        name
                                    ),
                                    location: None,
                                });
                            }
                            self.check_write_allowed_in_current_context(
                                Self::borrow_key_for_module_binding(binding_idx),
                                None,
                            )
                            .map_err(|e| match e {
                                ShapeError::SemanticError { message, location } => {
                                    let user_msg = message.replace(
                                        &format!(
                                            "(slot {})",
                                            Self::borrow_key_for_module_binding(binding_idx)
                                        ),
                                        &format!("'{}'", name),
                                    );
                                    ShapeError::SemanticError {
                                        message: user_msg,
                                        location,
                                    }
                                }
                                other => other,
                            })?;
                        }
                    }
                }

                // Optimization: x = x.push(val) → ArrayPushLocal (O(1) in-place mutation)
                if let Some(name) = assign.pattern.as_identifier() {
                    if let Expr::MethodCall {
                        receiver,
                        method,
                        args,
                        ..
                    } = &assign.value
                    {
                        if method == "push" && args.len() == 1 {
                            if let Expr::Identifier(recv_name, _) = receiver.as_ref() {
                                if recv_name == name {
                                    // R1 empty-array-push let-gen (2026-06-14):
                                    // `a = a.push(x)` where `a` is a bare empty-
                                    // array accumulator (`let mut a = []`, no
                                    // annotation, placeholder `NewArray(0)`
                                    // allocator). The v1 `ArrayPushLocal` path
                                    // below assumes a materialized array carrier
                                    // in the slot; the placeholder accumulator is
                                    // not yet a typed array, so pushing into it
                                    // (especially at MODULE scope, where the slot
                                    // read None) SIGSEGV'd. Route the first such
                                    // self-push through the accumulator finalizer:
                                    // it resolves the element kind from `x`'s
                                    // producer-side proof, PATCHES the placeholder
                                    // allocator to the typed `NewTypedArray*`
                                    // opcode (so the slot holds a real typed
                                    // array), emits the typed push, and leaves the
                                    // array on the stack — which the assignment
                                    // then stores back into the same slot. The
                                    // allocator patch fires AFTER the element type
                                    // resolves, so the module-binding slot is
                                    // constructed with the right kind (no None
                                    // read, no heap corruption).
                                    let source_loc =
                                        self.span_to_source_location(receiver.as_ref().span());
                                    if self.compile_first_push_to_empty_accumulator(
                                        recv_name,
                                        &args[0],
                                        Some(source_loc),
                                    )? {
                                        // The typed push left the (now-typed)
                                        // array on the stack; store it back into
                                        // the binding slot.
                                        if let Some(local_idx) = self.resolve_local(name) {
                                            self.emit(Instruction::new(
                                                OpCode::StoreLocal,
                                                Some(Operand::Local(local_idx)),
                                            ));
                                        } else {
                                            let binding_idx =
                                                self.get_or_create_module_binding(name);
                                            self.emit(Instruction::new(
                                                OpCode::StoreModuleBinding,
                                                Some(Operand::ModuleBinding(binding_idx)),
                                            ));
                                        }
                                        break 'assign;
                                    }
                                    if let Some(local_idx) = self.resolve_local(name) {
                                        self.compile_expr(&args[0])?;
                                        // U4-4: pushed element kind from the one resolved Type.
                                        let pushed_numeric = self.numeric_type_of(&args[0]);
                                        self.emit(Instruction::new(
                                            OpCode::ArrayPushLocal,
                                            Some(Operand::Local(local_idx)),
                                        ));
                                        if let Some(numeric_type) = pushed_numeric {
                                            self.mark_slot_as_numeric_array(
                                                local_idx,
                                                true,
                                                numeric_type,
                                            );
                                        }
                                        // T1 sub-case (a): see the first-push arm.
                                        self.record_pushed_element_concrete_type(name, &args[0]);
                                        self.plan_flexible_binding_storage_from_expr(
                                            local_idx,
                                            true,
                                            &assign.value,
                                        );
                                        break 'assign;
                                    } else {
                                        let binding_idx = self.get_or_create_module_binding(name);
                                        self.compile_expr(&args[0])?;
                                        // U4-4: pushed element kind from the one resolved Type.
                                        let pushed_numeric = self.numeric_type_of(&args[0]);
                                        self.emit(Instruction::new(
                                            OpCode::ArrayPushLocal,
                                            Some(Operand::ModuleBinding(binding_idx)),
                                        ));
                                        if let Some(numeric_type) = pushed_numeric {
                                            self.mark_slot_as_numeric_array(
                                                binding_idx,
                                                false,
                                                numeric_type,
                                            );
                                        }
                                        // T1 sub-case (a): see the first-push arm.
                                        self.record_pushed_element_concrete_type(name, &args[0]);
                                        self.plan_flexible_binding_storage_from_expr(
                                            binding_idx,
                                            false,
                                            &assign.value,
                                        );
                                        break 'assign;
                                    }
                                }
                            }
                        }
                    }
                }

                // Compile value
                let saved_pending_variable_name = self.pending_variable_name.clone();
                let saved_pending_variable_span = self.pending_variable_span;
                self.pending_variable_name =
                    assign.pattern.as_identifier().map(|name| name.to_string());
                self.pending_variable_span = assign.pattern.as_identifier_span();
                // V3-S5 empty-array reassign (STAGE T4, 2026-06-20): an empty
                // array literal RHS (`a = []`) carries no element type of its
                // own — the var-decl path proves it from the `Array<T>`
                // annotation and hands it to `compile_expr_array` via
                // `pending_variable_typed_array_kind` (statements.rs:967),
                // which makes the empty literal lower to the typed
                // `NewTypedArray*` allocator (count 0). A reassignment has no
                // annotation, so without the symmetric hand-off the empty
                // literal fell through to the generic `NewArray(0)` and
                // SURFACEd `op_new_array(0)` at runtime mid-program. Recover
                // the LHS binding's PROVEN element type from the type tracker
                // (the binding's declared `Array<T>` — ADR-006 §2.7.5
                // producer-side proof, no runtime inspection) and re-key it
                // through the same `pending_variable_typed_array_kind`
                // hand-off. NO TypedArrayData: the typed allocator the kind
                // selects is the existing per-T v2-raw `TypedArray<T>`
                // monomorphization which already handles the count-0 case.
                let saved_pending_typed_array_kind = self.pending_variable_typed_array_kind;
                if matches!(&assign.value, Expr::Array(elements, _) if elements.is_empty()) {
                    if let Some(name) = assign.pattern.as_identifier() {
                        if let Some(shape_value::v2::ConcreteType::Array(elem)) =
                            crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                                self,
                                &Expr::Identifier(name.to_string(), Span::DUMMY),
                            )
                        {
                            self.pending_variable_typed_array_kind =
                                crate::compiler::v2_typed_emission::should_use_typed_array(&elem);
                        }
                    }
                }
                let compile_result = self.compile_expr_for_reference_binding(&assign.value);
                self.pending_variable_typed_array_kind = saved_pending_typed_array_kind;
                self.pending_variable_name = saved_pending_variable_name;
                self.pending_variable_span = saved_pending_variable_span;
                let ref_borrow = compile_result?;
                let assigned_ident = assign.pattern.as_identifier().map(str::to_string);

                // Store in variable
                self.compile_destructure_assignment(&assign.pattern)?;
                if let Some(name) = assigned_ident.as_deref() {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if !self.local_binding_is_reference_value(local_idx) {
                            self.finish_reference_binding_from_expr(
                                local_idx,
                                true,
                                name,
                                &assign.value,
                                ref_borrow,
                            );
                            self.update_callable_binding_from_expr(local_idx, true, &assign.value);
                        }
                        self.plan_flexible_binding_storage_from_expr(
                            local_idx,
                            true,
                            &assign.value,
                        );
                    } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name)
                    {
                        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
                            self.finish_reference_binding_from_expr(
                                binding_idx,
                                false,
                                name,
                                &assign.value,
                                ref_borrow,
                            );
                            self.update_callable_binding_from_expr(
                                binding_idx,
                                false,
                                &assign.value,
                            );
                            self.plan_flexible_binding_storage_from_expr(
                                binding_idx,
                                false,
                                &assign.value,
                            );
                        }
                    }
                    self.propagate_assignment_type_to_identifier(name, Some(&assign.value));
                }
            }

            Statement::Expression(expr, _) => {
                // Fast path: arr.push(val) as standalone statement → in-place mutation
                // (avoids the LoadLocal+Pop overhead from the expression-level optimization)
                //
                // ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback):
                // gate the fast path so it does NOT fire when the receiver
                // is a non-Array container (Deque / PriorityQueue / HashMap
                // / HashSet). Those containers have their own `push`
                // handlers in `method_registry`; `ArrayPushLocal` would
                // error on a non-Array slot kind. Falls through to the
                // generic compile_expr path, which dispatches via
                // `CallMethod` and emits the writeback per
                // `resolve_mut_self_writeback_target`.
                if let Expr::MethodCall {
                    receiver,
                    method,
                    args,
                    ..
                } = expr
                {
                    let bespoke_push_blocked = if method == "push"
                        && args.len() == 1
                        && let Expr::Identifier(recv_name, _) = receiver.as_ref()
                    {
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
                        if let Expr::Identifier(recv_name, _) = receiver.as_ref() {
                            let source_loc = self.span_to_source_location(receiver.as_ref().span());
                            // Phase 4b Round 6 WS-1b W16.2-C residual
                            // (2026-05-21): a bare empty-array accumulator's
                            // FIRST `.push()` resolves its element kind,
                            // patches the placeholder allocator, and promotes
                            // the binding. The method leaves the array on the
                            // stack — pop it (statement context discards the
                            // result).
                            if self.compile_first_push_to_empty_accumulator(
                                recv_name,
                                &args[0],
                                Some(source_loc.clone()),
                            )? {
                                self.emit(Instruction::simple(OpCode::Pop));
                                return Ok(());
                            }
                            // Resolve the receiver's `TypedArrayKind`. A
                            // receiver that is ALREADY a v2 typed array
                            // (annotated `Array<T>`, inferred typed literal,
                            // a promoted accumulator from an earlier push)
                            // must emit the typed `TypedArrayPush*` opcode.
                            // The legacy `ArrayPushLocal` below is the v1
                            // NaN-boxed carrier path: emitting it against a
                            // v2-raw `TypedArray<T>` receiver is a
                            // kind-mismatch (the V3-S5 `op_array_push`
                            // strict-kind check).
                            let typed_kind =
                                self.resolve_receiver_typed_array_kind(receiver.as_ref());
                            if let Some(local_idx) = self.resolve_local(recv_name) {
                                if !self.ref_locals.contains(&local_idx) {
                                    self.check_named_binding_write_allowed(
                                        recv_name,
                                        Some(source_loc.clone()),
                                    )?;
                                }
                                if let Some(kind) = typed_kind {
                                    // v2 typed array push: `TypedArrayPush*`
                                    // pops (arr_ptr, value).
                                    self.emit(Instruction::new(
                                        OpCode::LoadLocal,
                                        Some(Operand::Local(local_idx)),
                                    ));
                                    self.compile_typed_array_element_value(kind, &args[0])?;
                                    self.emit(Instruction::simple(kind.push_opcode()));
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
                                return Ok(());
                            } else if !self
                                .mutable_closure_captures
                                .contains_key(recv_name.as_str())
                            {
                                self.check_named_binding_write_allowed(
                                    recv_name,
                                    Some(source_loc),
                                )?;
                                let binding_idx = self.get_or_create_module_binding(recv_name);
                                if let Some(kind) = typed_kind {
                                    self.emit(Instruction::new(
                                        OpCode::LoadModuleBinding,
                                        Some(Operand::ModuleBinding(binding_idx)),
                                    ));
                                    self.compile_typed_array_element_value(kind, &args[0])?;
                                    self.emit(Instruction::simple(kind.push_opcode()));
                                    return Ok(());
                                }
                                self.compile_expr(&args[0])?;
                                self.emit(Instruction::new(
                                    OpCode::ArrayPushLocal,
                                    Some(Operand::ModuleBinding(binding_idx)),
                                ));
                                return Ok(());
                            }
                        }
                    }
                }
                self.compile_expr(expr)?;
                self.emit(Instruction::simple(OpCode::Pop));
            }

            Statement::For(for_loop, _) => {
                self.compile_for_loop(for_loop)?;
            }

            Statement::While(while_loop, _) => {
                self.compile_while_loop(while_loop)?;
            }

            Statement::If(if_stmt, _) => {
                self.compile_if_statement(if_stmt)?;
            }
            Statement::Extend(extend, span) => {
                self.require_comptime_mode("extend", *span)?;
                self.emit_comptime_extend_directive(extend, *span)?;
            }
            Statement::RemoveTarget(span) => {
                self.require_comptime_mode("remove target", *span)?;
                self.emit_comptime_remove_directive(*span)?;
            }
            Statement::SetParamType {
                param_name,
                type_annotation,
                span,
            } => {
                self.require_comptime_mode("set param", *span)?;
                self.emit_comptime_set_param_type_directive(param_name, type_annotation, *span)?;
            }
            Statement::SetParamValue {
                param_name,
                expression,
                span,
            } => {
                self.require_comptime_mode("set param", *span)?;
                self.emit_comptime_set_param_value_directive(param_name, expression, *span)?;
            }
            Statement::SetReturnType {
                type_annotation,
                span,
            } => {
                self.require_comptime_mode("set return", *span)?;
                self.emit_comptime_set_return_type_directive(type_annotation, *span)?;
            }
            Statement::SetReturnExpr { expression, span } => {
                self.require_comptime_mode("set return", *span)?;
                self.emit_comptime_set_return_expr_directive(expression, *span)?;
            }
            Statement::ReplaceBody { body, span } => {
                self.require_comptime_mode("replace body", *span)?;
                self.emit_comptime_replace_body_directive(body, *span)?;
            }
            Statement::ReplaceBodyExpr { expression, span } => {
                self.require_comptime_mode("replace body", *span)?;
                self.emit_comptime_replace_body_expr_directive(expression, *span)?;
            }
            Statement::ReplaceModuleExpr { expression, span } => {
                self.require_comptime_mode("replace module", *span)?;
                self.emit_comptime_replace_module_expr_directive(expression, *span)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::BytecodeCompiler;
    use shape_ast::ast::{Item, Span, Statement, TypeAnnotation};
    use shape_ast::parser::parse_program;

    // The four `test_module_*` / `test_module_inline_comptime_*` tests
    // below assert against the `vm.execute(None)` return shape's deleted
    // `as_number_coerce()` helper. Both the synthesis surface
    // (`synthesize_value_word_from_raw`, playbook §1) and the carrier
    // (`ValueWord` / `ValueWordExt::as_number_coerce`, CLAUDE.md "Renames
    // to refuse on sight") are deleted in the strict-typing bulldozer.
    // Each test is gated `#[cfg(any())]` (always-false) to keep the
    // assertion shape as documentation while preventing the deleted
    // accessor from re-entering compile. Re-enable when the kinded
    // `vm.execute_raw -> (bits, kind)` boundary lands and the tests can
    // read `f64::from_bits(bits)` directly. Phase-2c rebuild surface —
    // see ADR-006 §2.4.
    #[test]
    fn module_qualification_preserves_builtin_set_generic() {
        let mut type_params = std::collections::HashSet::new();
        type_params.insert("T".to_string());

        let set_ann = TypeAnnotation::Generic {
            name: "Set".into(),
            args: vec![TypeAnnotation::Basic("T".to_string())],
        };
        let qualified = BytecodeCompiler::qualify_module_type_annotation(
            &set_ann,
            "std::core::set",
            &type_params,
        );

        assert_eq!(qualified, set_ann);

        let local_ann = TypeAnnotation::Generic {
            name: "LocalBox".into(),
            args: vec![TypeAnnotation::Basic("T".to_string())],
        };
        let qualified_local = BytecodeCompiler::qualify_module_type_annotation(
            &local_ann,
            "std::core::set",
            &type_params,
        );
        assert!(matches!(
            qualified_local,
            TypeAnnotation::Generic { ref name, .. } if name.as_str() == "std::core::set::LocalBox"
        ));
    }

    #[cfg(any())]
    #[test]
    fn test_module_decl_function_resolves_module_const() {
        let code = r#"
            mod math {
                const BASE = 21
                fn twice() {
                    BASE * 2
                }
            }
            math::twice()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        let result = vm.execute(None).expect("Failed to execute");
        assert_eq!(
            result
                .as_number_coerce()
                .expect("module call should return number"),
            42.0
        );
    }

    #[cfg(any())]
    #[test]
    fn test_module_annotation_can_replace_module_items() {
        let code = r#"
            annotation synth_module() {
                targets: [module]
                comptime post(target, ctx) {
                    replace module ("const ANSWER = 40; fn plus_two() { ANSWER + 2 }")
                }
            }

            @synth_module()
            mod demo {}

            demo::plus_two()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        let result = vm.execute(None).expect("Failed to execute");
        assert_eq!(
            result
                .as_number_coerce()
                .expect("module call should return number"),
            42.0
        );
    }

    #[cfg(any())]
    #[test]
    fn test_module_inline_comptime_can_replace_module_items() {
        let code = r#"
            mod demo {
                comptime {
                    replace module ("const ANSWER = 40; fn plus_two() { ANSWER + 2 }")
                }
            }

            demo::plus_two()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        let result = vm.execute(None).expect("Failed to execute");
        assert_eq!(
            result
                .as_number_coerce()
                .expect("module call should return number"),
            42.0
        );
    }

    #[cfg(any())]
    #[test]
    fn test_module_inline_comptime_can_use_module_local_comptime_helper() {
        let code = r#"
            mod demo {
                comptime fn synth() {
                    "const ANSWER = 40; fn plus_two() { ANSWER + 2 }"
                }

                comptime {
                    replace module (synth())
                }
            }

            demo::plus_two()
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.populate_module_objects();
        let result = vm.execute(None).expect("Failed to execute");
        assert_eq!(
            result
                .as_number_coerce()
                .expect("module call should return number"),
            42.0
        );
    }

    #[test]
    fn test_type_annotated_variable_no_wrapping() {
        // BUG-1/BUG-2 fix: variable declarations must NOT emit WrapTypeAnnotation
        // (the wrapper broke arithmetic and comparisons)
        let code = r#"
            type Currency = Number
            let x: Currency = 123
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // WrapTypeAnnotation should NOT be emitted for variable declarations
        let has_wrap_instruction = bytecode
            .instructions
            .iter()
            .any(|instr| instr.opcode == crate::bytecode::OpCode::WrapTypeAnnotation);
        assert!(
            !has_wrap_instruction,
            "Should NOT emit WrapTypeAnnotation for type-annotated variable"
        );
    }

    #[test]
    fn test_untyped_variable_no_wrapping() {
        // Variables without type annotations should NOT emit WrapTypeAnnotation
        let code = r#"
            let x = 123
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // Check that WrapTypeAnnotation instruction was NOT emitted
        let has_wrap_instruction = bytecode
            .instructions
            .iter()
            .any(|instr| instr.opcode == crate::bytecode::OpCode::WrapTypeAnnotation);
        assert!(
            !has_wrap_instruction,
            "Should NOT emit WrapTypeAnnotation for untyped variable"
        );
    }

    // ===== Phase 2: Extend Block Compilation Tests =====

    #[test]
    fn test_extend_block_compiles() {
        let code = r#"
            extend Number {
                method double() {
                    return self * 2
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse extend block");
        let bytecode = BytecodeCompiler::new().compile(&program);
        assert!(
            bytecode.is_ok(),
            "Extend block should compile: {:?}",
            bytecode.err()
        );

        // Verify a function named "Number.double" was generated (qualified extend name).
        let bytecode = bytecode.unwrap();
        let has_double = bytecode.functions.iter().any(|f| f.name == "Number.double");
        assert!(
            has_double,
            "Should generate 'Number.double' function from extend block"
        );
    }

    #[test]
    fn test_extend_method_has_self_param() {
        let code = r#"
            extend Number {
                method add(n) {
                    return self + n
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let func = bytecode.functions.iter().find(|f| f.name == "Number.add");
        assert!(func.is_some(), "Should have 'Number.add' function");
        // The function should have 2 params: self + n
        assert_eq!(
            func.unwrap().arity,
            2,
            "add() should have arity 2 (self + n)"
        );
    }

    #[test]
    fn test_extend_method_rejects_explicit_self_param() {
        let code = r#"
            extend Number {
                method add(self, n) {
                    return self + n
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("Compiler should reject explicit self receiver param in methods");
        let msg = format!("{err}");
        assert!(
            msg.contains("explicit `self` parameter"),
            "Expected explicit self error, got: {msg}"
        );
    }

    // ===== Phase 3: Annotation Handler Compilation Tests =====

    #[test]
    fn test_annotation_def_compiles_handlers() {
        let code = r#"
            annotation warmup(period) {
                before(args, ctx) {
                    args
                }
                after(args, result, ctx) {
                    result
                }
            }
            function test() { return 42; }
        "#;
        let program = parse_program(code).expect("Failed to parse annotation def");
        let bytecode = BytecodeCompiler::new().compile(&program);
        assert!(
            bytecode.is_ok(),
            "Annotation def should compile: {:?}",
            bytecode.err()
        );

        let bytecode = bytecode.unwrap();
        // Verify CompiledAnnotation was registered
        assert!(
            bytecode.compiled_annotations.contains_key("warmup"),
            "Should have compiled 'warmup' annotation"
        );

        let compiled = bytecode.compiled_annotations.get("warmup").unwrap();
        assert!(
            compiled.before_handler.is_some(),
            "Should have before handler"
        );
        assert!(
            compiled.after_handler.is_some(),
            "Should have after handler"
        );
    }

    #[test]
    fn test_exported_annotation_def_compiles_handlers() {
        let code = r#"
            pub annotation warmup(period) {
                before(args, ctx) {
                    args
                }
            }

            @warmup(5)
            fn test() { 42 }
        "#;
        let program = parse_program(code).expect("Failed to parse exported annotation def");
        let bytecode = BytecodeCompiler::new().compile(&program);
        assert!(
            bytecode.is_ok(),
            "Exported annotation def should compile: {:?}",
            bytecode.err()
        );

        let bytecode = bytecode.unwrap();
        assert!(
            bytecode.compiled_annotations.contains_key("warmup"),
            "Should have compiled exported 'warmup' annotation"
        );
    }

    #[test]
    fn test_annotation_handler_function_names() {
        let code = r#"
            annotation my_ann(x) {
                before(args, ctx) {
                    args
                }
            }
            function test() { return 1; }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // Handler should be compiled as an internal function
        let compiled = bytecode.compiled_annotations.get("my_ann").unwrap();
        let handler_id = compiled.before_handler.unwrap() as usize;
        assert!(
            handler_id < bytecode.functions.len(),
            "Handler function ID should be valid"
        );

        let handler_fn = &bytecode.functions[handler_id];
        assert_eq!(
            handler_fn.name, "my_ann___before",
            "Handler function should be named my_ann___before"
        );
    }

    // ===== Phase 4: Compile-Time Function Wrapping Tests =====

    #[test]
    fn test_annotated_function_generates_wrapper() {
        let code = r#"
            annotation tracked(label) {
                before(args, ctx) {
                    args
                }
            }
            @tracked("my_func")
            function compute(x) {
                return x * 2
            }
            function test() { return 1; }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program);
        assert!(
            bytecode.is_ok(),
            "Annotated function should compile: {:?}",
            bytecode.err()
        );

        let bytecode = bytecode.unwrap();
        // Should have the original function (wrapper) and the impl
        let has_impl = bytecode
            .functions
            .iter()
            .any(|f| f.name == "compute___impl");
        assert!(has_impl, "Should generate compute___impl function");

        let has_wrapper = bytecode.functions.iter().any(|f| f.name == "compute");
        assert!(has_wrapper, "Should keep compute as wrapper");
    }

    #[test]
    fn test_unannotated_function_no_wrapper() {
        let code = r#"
            function plain(x) {
                return x + 1
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // Should NOT have an ___impl function
        let has_impl = bytecode
            .functions
            .iter()
            .any(|f| f.name.ends_with("___impl"));
        assert!(
            !has_impl,
            "Non-annotated function should not generate ___impl"
        );
    }

    // ===== Sprint 10: Annotation chaining and target validation =====

    #[test]
    fn test_annotation_chaining_generates_chain() {
        // Two annotations on the same function should generate chained wrappers
        let code = r#"
            annotation first() {
                before(args, ctx) {
                    return args
                }
            }

            annotation second() {
                before(args, ctx) {
                    return args
                }
            }

            @first
            @second
            function compute(x) {
                return x * 2
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program);
        assert!(
            bytecode.is_ok(),
            "Chained annotations should compile: {:?}",
            bytecode.err()
        );
        let bytecode = bytecode.unwrap();

        // Should have: compute (outermost wrapper), compute___impl (body), compute___second (intermediate)
        let has_impl = bytecode
            .functions
            .iter()
            .any(|f| f.name == "compute___impl");
        assert!(has_impl, "Should generate compute___impl function");
        let has_wrapper = bytecode.functions.iter().any(|f| f.name == "compute");
        assert!(has_wrapper, "Should keep compute as outermost wrapper");
        let has_intermediate = bytecode
            .functions
            .iter()
            .any(|f| f.name == "compute___second");
        assert!(
            has_intermediate,
            "Should generate compute___second intermediate wrapper"
        );
    }

    #[test]
    fn test_annotation_allowed_targets_inferred() {
        // An annotation with before/after should have allowed_targets = [Function]
        let code = r#"
            annotation traced() {
                before(args, ctx) {
                    return args
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program).expect("compile");
        let ann = bytecode
            .compiled_annotations
            .get("traced")
            .expect("traced annotation");
        assert!(
            !ann.allowed_targets.is_empty(),
            "before handler should restrict targets"
        );
        assert!(
            ann.allowed_targets
                .contains(&shape_ast::ast::functions::AnnotationTargetKind::Function),
            "before handler should allow Function target"
        );
    }

    #[test]
    fn test_annotation_allowed_targets_explicit_override() {
        // Explicit `targets: [...]` should override inferred defaults.
        let code = r#"
            annotation traced() {
                targets: [type]
                before(args, ctx) {
                    return args
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program).expect("compile");
        let ann = bytecode
            .compiled_annotations
            .get("traced")
            .expect("traced annotation");
        assert_eq!(
            ann.allowed_targets,
            vec![shape_ast::ast::functions::AnnotationTargetKind::Type]
        );
    }

    #[test]
    fn test_metadata_only_annotation_defaults_to_definition_targets() {
        // An annotation with only metadata handler should default to definition targets.
        let code = r#"
            annotation info() {
                metadata() {
                    return { version: 1 }
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new().compile(&program).expect("compile");
        let ann = bytecode
            .compiled_annotations
            .get("info")
            .expect("info annotation");
        assert_eq!(
            ann.allowed_targets,
            vec![
                shape_ast::ast::functions::AnnotationTargetKind::Function,
                shape_ast::ast::functions::AnnotationTargetKind::Type,
                shape_ast::ast::functions::AnnotationTargetKind::Module
            ],
            "metadata-only annotation should default to definition targets"
        );
    }

    #[test]
    fn test_definition_lifecycle_targets_reject_expression_target() {
        let code = r#"
            annotation info() {
                targets: [expression]
                metadata(target, ctx) {
                    target.name
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("metadata hooks on expression targets should fail");
        let msg = format!("{}", err);
        assert!(
            msg.contains("not a definition target"),
            "expected definition-target restriction error, got: {}",
            msg
        );
    }

    #[test]
    fn test_annotation_target_validation_on_struct_type() {
        // Function-only annotation applied to a type should fail.
        let code = r#"
            annotation traced() {
                before(args, ctx) { return args }
            }

            @traced()
            type Point { x: int }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("function-only annotation on type should fail");
        let msg = format!("{}", err);
        assert!(
            msg.contains("cannot be applied to a type"),
            "expected type target validation error, got: {}",
            msg
        );
    }

    #[test]
    fn test_type_c_emits_native_layout_metadata() {
        let bytecode = compiles_to(
            r#"
            type C Pair32 {
                left: i32,
                right: i32,
            }
            "#,
        );

        assert_eq!(bytecode.native_struct_layouts.len(), 1);
        let layout = &bytecode.native_struct_layouts[0];
        assert_eq!(layout.name, "Pair32");
        assert_eq!(layout.abi, "C");
        assert_eq!(layout.size, 8);
        assert_eq!(layout.align, 4);
        assert_eq!(layout.fields.len(), 2);
        assert_eq!(layout.fields[0].name, "left");
        assert_eq!(layout.fields[0].offset, 0);
        assert_eq!(layout.fields[0].size, 4);
        assert_eq!(layout.fields[1].name, "right");
        assert_eq!(layout.fields[1].offset, 4);
        assert_eq!(layout.fields[1].size, 4);
    }

    #[test]
    fn test_type_c_auto_generates_into_from_traits() {
        let bytecode = compiles_to(
            r#"
            type C QuoteC {
                bid: i64,
                ask: i64,
            }

            type Quote {
                bid: i64,
                ask: i64,
            }
            "#,
        );

        let c_to_shape =
            bytecode.lookup_trait_method_symbol("Into", "QuoteC", Some("Quote"), "into");
        let shape_to_c =
            bytecode.lookup_trait_method_symbol("Into", "Quote", Some("QuoteC"), "into");
        let from_c = bytecode.lookup_trait_method_symbol("From", "Quote", Some("QuoteC"), "from");
        let from_shape =
            bytecode.lookup_trait_method_symbol("From", "QuoteC", Some("Quote"), "from");

        assert!(c_to_shape.is_some(), "expected Into<Quote> for QuoteC");
        assert!(shape_to_c.is_some(), "expected Into<QuoteC> for Quote");
        assert!(from_c.is_some(), "expected From<QuoteC> for Quote");
        assert!(from_shape.is_some(), "expected From<Quote> for QuoteC");
    }

    #[test]
    fn test_type_c_auto_conversion_function_compiles() {
        let _ = compiles_to(
            r#"
            type Quote {
                bid: i64,
                ask: i64,
            }

            type C QuoteC {
                bid: i64,
                ask: i64,
            }

            fn spread(q: QuoteC) -> i64 {
                let q_shape = __auto_native_from_QuoteC_to_Quote(q);
                q_shape.ask - q_shape.bid
            }

            spread(QuoteC { bid: 10, ask: 13 })
            "#,
        );
    }

    #[test]
    fn test_type_c_auto_conversion_rejects_incompatible_fields() {
        let program = parse_program(
            r#"
            type Price {
                value: i64,
            }

            type C PriceC {
                value: u64,
            }
            "#,
        )
        .expect("parse failed");
        let err = BytecodeCompiler::new()
            .compile(&program)
            .expect_err("incompatible type C conversion pair should fail");
        let msg = format!("{}", err);
        assert!(
            msg.contains("field type mismatch for auto conversion"),
            "expected type mismatch error, got: {}",
            msg
        );
    }

    // ===== Task 1: Meta on traits =====

    // ===== Drop Track: Sprint 2 Tests =====

    fn compiles_to(code: &str) -> crate::bytecode::BytecodeProgram {
        let program = parse_program(code).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        compiler.compile(&program).expect("compile failed")
    }

    // --- Permission checking tests ---

    #[test]
    fn test_permission_check_allows_pure_module_imports() {
        // json is a pure module — should compile even with empty permissions
        let code = "from std::core::json use { parse }";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_permission_set(Some(shape_abi_v1::PermissionSet::pure()));
        // Should not fail — json requires no permissions
        let _result = compiler.compile(&program);
    }

    #[test]
    fn test_permission_check_blocks_file_import_under_pure() {
        let code = "from std::core::file use { read_text }";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_permission_set(Some(shape_abi_v1::PermissionSet::pure()));
        let result = compiler.compile(&program);
        assert!(
            result.is_err(),
            "Expected permission error for file::read_text under pure"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Permission denied"),
            "Error should mention permission denied: {err_msg}"
        );
        assert!(
            err_msg.contains("fs.read"),
            "Error should mention fs.read: {err_msg}"
        );
    }

    #[test]
    fn test_permission_check_allows_file_import_with_fs_read() {
        let code = "from std::core::file use { read_text }";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        let pset = shape_abi_v1::PermissionSet::from_iter([shape_abi_v1::Permission::FsRead]);
        compiler.set_permission_set(Some(pset));
        // Should not fail
        let _result = compiler.compile(&program);
    }

    #[test]
    fn test_permission_check_no_permission_set_allows_everything() {
        // When permission_set is None (default), no checking is done
        let code = "from std::core::file use { read_text }";
        let program = parse_program(code).expect("parse failed");
        let compiler = BytecodeCompiler::new();
        // permission_set is None by default — should compile fine
        let _result = compiler.compile(&program);
    }

    #[test]
    fn test_permission_check_namespace_import_blocked() {
        let code = "use std::core::http";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_permission_set(Some(shape_abi_v1::PermissionSet::pure()));
        let result = compiler.compile(&program);
        assert!(
            result.is_err(),
            "Expected permission error for `use std::core::http` under pure"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("Permission denied"),
            "Error should mention permission denied: {err_msg}"
        );
    }

    #[test]
    fn test_permission_check_namespace_import_allowed() {
        let code = "use std::core::http";
        let program = parse_program(code).expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_permission_set(Some(shape_abi_v1::PermissionSet::full()));
        // Should not fail
        let _result = compiler.compile(&program);
    }

    fn test_decl(kind: shape_ast::ast::VarKind, is_mut: bool) -> shape_ast::ast::VariableDecl {
        shape_ast::ast::VariableDecl {
            kind,
            is_mut,
            pattern: shape_ast::ast::DestructurePattern::Identifier(
                "x".to_string(),
                shape_ast::ast::Span::DUMMY,
            ),
            type_annotation: None,
            value: None,
            ownership: Default::default(),
        }
    }

    #[test]
    fn test_binding_semantics_for_decl_maps_let_var_classes() {
        let let_semantics = BytecodeCompiler::binding_semantics_for_var_decl(&test_decl(
            shape_ast::ast::VarKind::Let,
            false,
        ));
        assert_eq!(
            let_semantics.ownership_class,
            crate::type_tracking::BindingOwnershipClass::OwnedImmutable
        );
        assert_eq!(
            let_semantics.storage_class,
            crate::type_tracking::BindingStorageClass::Direct
        );

        let let_mut_semantics = BytecodeCompiler::binding_semantics_for_var_decl(&test_decl(
            shape_ast::ast::VarKind::Let,
            true,
        ));
        assert_eq!(
            let_mut_semantics.ownership_class,
            crate::type_tracking::BindingOwnershipClass::OwnedMutable
        );
        assert_eq!(
            let_mut_semantics.storage_class,
            crate::type_tracking::BindingStorageClass::Direct
        );

        let var_semantics = BytecodeCompiler::binding_semantics_for_var_decl(&test_decl(
            shape_ast::ast::VarKind::Var,
            false,
        ));
        assert_eq!(
            var_semantics.ownership_class,
            crate::type_tracking::BindingOwnershipClass::Flexible
        );
        assert_eq!(
            var_semantics.storage_class,
            crate::type_tracking::BindingStorageClass::Deferred
        );
    }

    #[test]
    fn test_destructured_module_bindings_get_binding_semantics() {
        let mut compiler = BytecodeCompiler::new();
        let pattern = shape_ast::ast::DestructurePattern::Array(vec![
            shape_ast::ast::DestructurePattern::Identifier(
                "left".to_string(),
                shape_ast::ast::Span::DUMMY,
            ),
            shape_ast::ast::DestructurePattern::Identifier(
                "right".to_string(),
                shape_ast::ast::Span::DUMMY,
            ),
        ]);
        compiler
            .compile_destructure_pattern_global(&pattern)
            .expect("destructure should compile");
        compiler.apply_binding_semantics_to_pattern_bindings(
            &pattern,
            false,
            BytecodeCompiler::binding_semantics_for_var_decl(&test_decl(
                shape_ast::ast::VarKind::Let,
                false,
            )),
        );

        let left_idx = *compiler
            .module_bindings
            .get("left")
            .expect("left binding should exist");
        let right_idx = *compiler
            .module_bindings
            .get("right")
            .expect("right binding should exist");

        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(left_idx)
                .map(|semantics| semantics.ownership_class),
            Some(crate::type_tracking::BindingOwnershipClass::OwnedImmutable)
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(left_idx)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::Direct)
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(right_idx)
                .map(|semantics| semantics.ownership_class),
            Some(crate::type_tracking::BindingOwnershipClass::OwnedImmutable)
        );
    }

    #[test]
    fn test_flexible_binding_alias_initializer_marks_shared_storage() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let source = compiler.declare_local("source").expect("declare source");
        let dest = compiler.declare_local("dest").expect("declare dest");
        let var_semantics = BytecodeCompiler::binding_semantics_for_var_decl(&test_decl(
            shape_ast::ast::VarKind::Var,
            false,
        ));
        compiler
            .type_tracker
            .set_local_binding_semantics(source, var_semantics);
        compiler
            .type_tracker
            .set_local_binding_semantics(dest, var_semantics);

        compiler.plan_flexible_binding_storage_from_expr(
            dest,
            true,
            &shape_ast::ast::Expr::Identifier("source".to_string(), shape_ast::ast::Span::DUMMY),
        );

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(source)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::SharedCow)
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(dest)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::SharedCow)
        );
    }

    #[test]
    fn test_flexible_destructure_bindings_finalize_to_direct_storage() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let left = compiler.declare_local("left").expect("declare left");
        let right = compiler.declare_local("right").expect("declare right");
        let var_semantics = BytecodeCompiler::binding_semantics_for_var_decl(&test_decl(
            shape_ast::ast::VarKind::Var,
            false,
        ));
        compiler
            .type_tracker
            .set_local_binding_semantics(left, var_semantics);
        compiler
            .type_tracker
            .set_local_binding_semantics(right, var_semantics);

        let pattern = shape_ast::ast::DestructurePattern::Array(vec![
            shape_ast::ast::DestructurePattern::Identifier(
                "left".to_string(),
                shape_ast::ast::Span::DUMMY,
            ),
            shape_ast::ast::DestructurePattern::Identifier(
                "right".to_string(),
                shape_ast::ast::Span::DUMMY,
            ),
        ]);
        compiler.plan_flexible_binding_storage_for_pattern_initializer(
            &pattern,
            true,
            Some(&shape_ast::ast::Expr::Identifier(
                "source".to_string(),
                shape_ast::ast::Span::DUMMY,
            )),
        );

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(left)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::Direct)
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(right)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::Direct)
        );
    }

    #[test]
    fn test_module_var_alias_decl_marks_shared_storage() {
        let program = parse_program(
            r#"
                var source = [1]
                var alias = source
            "#,
        )
        .expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        let first_decl = match &program.items[0] {
            Item::VariableDecl(var_decl, _) => {
                Statement::VariableDecl(var_decl.clone(), Span::DUMMY)
            }
            Item::Statement(stmt, _) => stmt.clone(),
            _ => panic!("expected first variable declaration"),
        };
        let second_decl = match &program.items[1] {
            Item::VariableDecl(var_decl, _) => {
                Statement::VariableDecl(var_decl.clone(), Span::DUMMY)
            }
            Item::Statement(stmt, _) => stmt.clone(),
            _ => panic!("expected second variable declaration"),
        };
        compiler
            .compile_statement(&first_decl)
            .expect("first decl should compile");
        compiler
            .compile_statement(&second_decl)
            .expect("second decl should compile");

        let source_idx = *compiler
            .module_bindings
            .get("source")
            .expect("source binding should exist");
        let alias_idx = *compiler
            .module_bindings
            .get("alias")
            .expect("alias binding should exist");

        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(source_idx)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::SharedCow)
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(alias_idx)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::SharedCow)
        );
    }

    #[test]
    fn test_module_var_fresh_decl_marks_direct_storage() {
        let program = parse_program("var values = [1, 2, 3]").expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        let decl = match &program.items[0] {
            Item::VariableDecl(var_decl, _) => {
                Statement::VariableDecl(var_decl.clone(), Span::DUMMY)
            }
            Item::Statement(stmt, _) => stmt.clone(),
            _ => panic!("expected variable declaration"),
        };
        compiler
            .compile_statement(&decl)
            .expect("decl should compile");

        let values_idx = *compiler
            .module_bindings
            .get("values")
            .expect("values binding should exist");

        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(values_idx)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::Direct)
        );
    }

    #[test]
    fn test_module_var_collection_escape_marks_source_unique_heap() {
        let program = parse_program(
            r#"
                var source = [1]
                var wrapped = [source]
            "#,
        )
        .expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        for item in &program.items {
            let stmt = match item {
                Item::VariableDecl(var_decl, _) => {
                    Statement::VariableDecl(var_decl.clone(), Span::DUMMY)
                }
                Item::Statement(stmt, _) => stmt.clone(),
                _ => continue,
            };
            compiler
                .compile_statement(&stmt)
                .expect("item should compile");
        }

        let source_idx = *compiler
            .module_bindings
            .get("source")
            .expect("source binding should exist");
        let wrapped_idx = *compiler
            .module_bindings
            .get("wrapped")
            .expect("wrapped binding should exist");

        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(source_idx)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::UniqueHeap)
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(wrapped_idx)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::Direct)
        );
    }

    #[test]
    fn test_module_var_assignment_alias_marks_shared_storage() {
        let program = parse_program(
            r#"
                var source = [1]
                var alias = []
                alias = source
            "#,
        )
        .expect("parse failed");
        let mut compiler = BytecodeCompiler::new();
        for item in &program.items {
            let stmt = match item {
                Item::VariableDecl(var_decl, _) => {
                    Statement::VariableDecl(var_decl.clone(), Span::DUMMY)
                }
                Item::Assignment(assign, _) => Statement::Assignment(assign.clone(), Span::DUMMY),
                Item::Statement(stmt, _) => stmt.clone(),
                _ => continue,
            };
            compiler
                .compile_statement(&stmt)
                .expect("item should compile");
        }

        let source_idx = *compiler
            .module_bindings
            .get("source")
            .expect("source binding should exist");
        let alias_idx = *compiler
            .module_bindings
            .get("alias")
            .expect("alias binding should exist");

        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(source_idx)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::SharedCow)
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_binding_semantics(alias_idx)
                .map(|semantics| semantics.storage_class),
            Some(crate::type_tracking::BindingStorageClass::SharedCow)
        );
    }

    // ─── Phase 3 cluster-0 Round 13 T1' gap 3 source-side fix ──────────
    //
    // `desugar_impl_method` now substitutes the trait's declared return
    // type into the synthesized `FunctionDef.return_type` when the impl
    // method omits its own. The 6A `function_return_concrete_types`
    // populator at `compile_post_assembly` reads from
    // `expanded_function_defs` which mirrors `function_defs` — populated
    // by `register_function` which clones the synthesized `FunctionDef`
    // verbatim. So the substituted return-type annotation flows through
    // unchanged, and the trait's declared `ConcreteType` lands in the
    // 6A side-table for the impl-method's function entry.

    #[test]
    fn desugar_impl_method_backfills_return_type_from_trait_declaration() {
        // Smoke 3 minimal shape: trait T { method name() -> string }
        // type X {} impl T for X { method name() { "x" } }
        //
        // The impl method body lacks the return-type annotation. Before
        // T1' gap 3 closure, `FunctionDef.return_type` is `None` for the
        // synthesized `X::name`, and `function_return_concrete_types[X::name]
        // = ConcreteType::Void`. After T1' gap 3 closure, the
        // synthesized return_type is `Some(TypeAnnotation::Basic("string"))`
        // backfilled from the trait's `Required(Method { return_type:
        // Basic("string"), .. })` declaration.
        let code = r#"
            trait T { method name() -> string }
            type X {}
            impl T for X {
                method name() { "x" }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // The impl method desugars to scoped function name `X::name`
        // (default-impl naming per desugar_impl_method line ~1678 —
        // `format!("{}::{}", type_name, method_name)`).
        let func_def = bytecode
            .expanded_function_defs
            .get("X::name")
            .expect("X::name function def should be registered");

        assert!(
            func_def.return_type.is_some(),
            "T1' gap 3: impl method `X::name` return_type should be \
             backfilled from trait declaration `T::method name() -> string`, \
             got None"
        );

        // Extract the substituted annotation and verify it is the trait's
        // declared `string` (not a fabricated default, not the void
        // sentinel).
        let return_ann = func_def
            .return_type
            .as_ref()
            .expect("return_type Some after backfill");
        match return_ann {
            shape_ast::ast::TypeAnnotation::Basic(name) => {
                assert_eq!(
                    name, "string",
                    "T1' gap 3: backfilled return_type must be the \
                     trait's declared `string`, got Basic(`{}`)",
                    name
                );
            }
            other => panic!(
                "T1' gap 3: expected Basic(\"string\") from trait \
                 declaration, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn desugar_impl_method_preserves_explicit_impl_return_type() {
        // If the impl explicitly declares a return-type, the backfill must
        // not override it (the impl's annotation is the authoritative
        // shape — the trait's declared shape is structurally compatible
        // but the impl may be more specific). Verify the substitution is
        // strictly a fallback for None.
        //
        // Smoke 3 -- but with explicit `-> string` repeated on the impl
        // method (impl methods use the `->` return-type syntax per
        // `shape.pest::return_type = { "->" ~ type_annotation }`).
        let code = r#"
            trait T { method name() -> string }
            type X {}
            impl T for X {
                method name() -> string { "x" }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let func_def = bytecode
            .expanded_function_defs
            .get("X::name")
            .expect("X::name function def should be registered");

        let return_ann = func_def
            .return_type
            .as_ref()
            .expect("return_type Some — impl declared explicitly");
        match return_ann {
            shape_ast::ast::TypeAnnotation::Basic(name) => {
                assert_eq!(
                    name, "string",
                    "T1' gap 3: explicit impl return_type must be \
                     preserved verbatim"
                );
            }
            other => panic!(
                "T1' gap 3: expected explicit Basic(\"string\") from \
                 impl source, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn desugar_impl_method_leaves_none_when_trait_method_has_no_return_type() {
        // A trait method that itself has no return type (e.g., a void
        // method) provides no return type to backfill. The substitution
        // path returns the impl's `None` unchanged — no fabricated
        // default per §2.7.7 #9.
        //
        // (`Required` trait members always have a return_type per the
        // AST — `TraitMemberSignature::Method { return_type: TypeAnnotation,
        // .. }` is non-optional. Default trait members carry
        // `Option<TypeAnnotation>`, which can be None for `method foo()
        // {}` default bodies that elide it. This test verifies the
        // default-trait-member arm preserves None when the trait
        // default itself has no return_type annotation.)
        let code = r#"
            trait T {
                method greet() { print("hi") }
            }
            type X {}
            impl T for X {}
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        // The default trait method's impl-side desugar is `X::greet`
        // (default-method path through `desugar_impl_method` invocation
        // at line ~426). When the trait default has no return_type, the
        // backfill returns None — preserving the impl's None.
        if let Some(func_def) = bytecode.expanded_function_defs.get("X::greet") {
            assert!(
                func_def.return_type.is_none(),
                "T1' gap 3: trait default method without return_type \
                 must not fabricate a default annotation, got: {:?}",
                func_def.return_type
            );
        }
        // The fn may not be registered if default-method inlining doesn't
        // emit a synthesized FunctionDef when the impl block is empty;
        // that's fine — the test asserts the negative space (no
        // fabricated annotation).
    }

    #[test]
    fn u4_6_post_mono_module_binding_method_chain_intermediate_compiles() {
        use crate::test_utils::compile_with_prelude;

        let res = compile_with_prelude(
            "let xs = [1, 2, 3]\n\
             let doubled = xs.map(|x| x * 2)\n\
             let trebled = doubled.map(|y| y + 1)\n\
             print(trebled.len())",
        );
        assert!(
            res.is_ok(),
            "module binding method-chain intermediate should compile; got: {:?}",
            res.err()
        );
    }

    #[test]
    fn u4_6_post_mono_local_binding_method_chain_intermediate_compiles() {
        use crate::test_utils::compile_with_prelude;

        let res = compile_with_prelude(
            "fn run() {\n\
             let xs = [1, 2, 3]\n\
             let doubled = xs.map(|x| x * 2)\n\
             let trebled = doubled.map(|y| y + 1)\n\
             print(trebled.len())\n\
             }\n\
             run()",
        );
        assert!(
            res.is_ok(),
            "local binding method-chain intermediate should compile; got: {:?}",
            res.err()
        );
    }

    // ── strict-flip S1: let-annotation Unknown-accept guard (STRUCTURAL) ──
    //
    // The laundering holes that previously ran rc=0 WRONG (VM==JIT) now reject
    // CLEANLY at compile time, plus the no-FP cases that must keep compiling.
    //   (A) angle-A REVERTED (2026-06-22): the prior
    //       `init_rests_on_unprovable_unannotated_fn` reject over-rejected the
    //       idiomatic `let x: int = f(5)` class (un-return-typed fn genuinely
    //       returning int) and is removed. Replaced (S1 body-return inference,
    //       2026-06-22) by a REAL return-type proof: `hof_unannotated_call_
    //       return_concrete_type` resolves an un-annotated fn's return from its
    //       body tail (params seeded to call-site arg types, callable-valued
    //       params resolved through their own bodies — `apply2(id, ret_num,
    //       3.0)` ⇒ `number`). HOLE-3 now REJECTS cleanly into an `int` binding
    //       (number != int) and BINDS cleanly into a `number` binding — no
    //       over-reject, no bit-leak. `binop_operand_disagreeing_primitive`
    //       (HOLE-2) is KEPT — it only fires on a structurally-PROVEN
    //       disagreeing operand (no over-reject).
    //   (B) array-destructure binding facts from runtime inference, now
    //       RECURSING into nested `[[a,b],[c,d]]` patterns (peels one
    //       `Array<…>` layer per level).

    fn compile_fails(source: &str) -> bool {
        let Ok(program) = parse_program(source) else {
            return true;
        };
        BytecodeCompiler::new().compile(&program).is_err()
    }

    #[test]
    fn strict_flip_s1_hole1_array_destructure_then_int_annotation_rejected() {
        // HOLE-1: `let [a, b] = pair` over `Array<number>` binds a,b to number
        // (angle B); `let bad: int = a` then mismatches (number != int).
        assert!(
            compile_fails("let pair = [3.0, 4.0]\nlet [a, b] = pair\nlet bad: int = a\nbad"),
            "HOLE-1: number array element accepted into int binding"
        );
    }

    #[test]
    fn strict_flip_s1_hole2_inline_hof_arith_into_int_rejected() {
        // HOLE-2: `apply(ret_num, 3.0) % 4` = `number % int`; the number-typed
        // left operand disagrees with the `int` annotation.
        assert!(
            compile_fails(
                "fn apply(f, x) { f(x) }\nfn ret_num(x) { x * 2.0 }\n\
                 let bad: int = apply(ret_num, 3.0) % 4\nbad"
            ),
            "HOLE-2: number-operand arithmetic accepted into int binding"
        );
    }

    #[test]
    fn strict_flip_s1_nofp_unannotated_fn_returning_int_into_int_compiles() {
        // angle-A REVERT no-over-reject: `let r: int = f(5)` where `f` is an
        // un-return-typed user fn that GENUINELY returns int from its body must
        // COMPILE and run (a matching annotation must never reject a working
        // program). This was the over-rejected idiomatic class.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(eval_typed_i64("fn f(x) { x + 1 }\nlet r: int = f(5)\nr"), 6,);
    }

    #[test]
    fn strict_flip_s1_nofp_unannotated_fn_returning_number_into_number_compiles() {
        // angle-A REVERT no-over-reject (number twin).
        use crate::test_utils::eval_typed_f64;
        assert_eq!(
            eval_typed_f64("fn g(x) { x * 2.0 }\nlet r: number = g(3.0)\nr"),
            6.0,
        );
    }

    #[test]
    fn strict_flip_s1_nofp_unannotated_fn_chain_into_int_compiles() {
        // angle-A REVERT no-over-reject: a chain of un-annotated-fn bindings.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64(
                "fn a(x) { x + 1 }\nfn b(x) { x * 2 }\n\
                 let p: int = a(5)\nlet q: int = b(p)\nlet r: int = a(q)\nr"
            ),
            13,
        );
    }

    #[test]
    fn strict_flip_s1_nested_destructure_int_arith_compiles() {
        // angle-B nested extension no-over-reject: `let [[a,b],[c,d]] =
        // [[3,4],[5,6]]` stamps a,b,c,d to int; `let s: int = a + b` => 7
        // (AddInt — no "no method add on Int64" runtime crash).
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64("let [[a, b], [c, d]] = [[3, 4], [5, 6]]\nlet s: int = a + b\ns"),
            7,
        );
    }

    #[test]
    fn strict_flip_s1_nested_destructure_number_into_int_rejected() {
        // angle-B nested extension HOLE close: `let [[a,b],[c,d]] =
        // [[3.0,4.0],[5.0,6.0]]` stamps a,b,c,d to number; `let bad: int = a`
        // then mismatches (number != int).
        assert!(
            compile_fails("let [[a, b], [c, d]] = [[3.0, 4.0], [5.0, 6.0]]\nlet bad: int = a\nbad"),
            "nested-destructure: number element accepted into int binding"
        );
    }

    #[test]
    fn strict_flip_s1_nested_destructure_int_into_number_rejected() {
        // angle-B nested extension HOLE close (other direction): int element
        // into a number binding mismatches.
        assert!(
            compile_fails("let [[a, b], [c, d]] = [[3, 4], [5, 6]]\nlet bad: number = a\nbad"),
            "nested-destructure: int element accepted into number binding"
        );
    }

    #[test]
    fn strict_flip_s1_valid_int_array_destructure_compiles() {
        // No-FP (angle B): a homogeneous int array destructure binds a,b to
        // `int`, so `let s: int = a + b` type-checks and runs to 7.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64("let [a, b] = [3, 4]\nlet s: int = a + b\ns"),
            7,
        );
    }

    #[test]
    fn strict_flip_s1_nofp_closure_hof_result_binds_int() {
        // No-FP (angle A): a closure-literal HOF arg genuinely proves `int`
        // (NOT an annotation echo) — must keep compiling and run to 42.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64(
                "fn apply2(f, x, y) { f(x, y) }\nlet r: int = apply2(|a, b| a * b, 6, 7)\nr"
            ),
            42,
        );
    }

    #[test]
    fn strict_flip_s1_nofp_hof_number_return_binds_number() {
        // No-FP (angle A): a resolvable-HOF result (`apply(ret_num, 3.0)` ->
        // number via the HOF resolver) binds cleanly into a `number`.
        use crate::test_utils::eval_typed_f64;
        assert_eq!(
            eval_typed_f64(
                "fn apply(f, x) { f(x) }\nfn ret_num(x) { x * 2.0 }\n\
                 let r: number = apply(ret_num, 3.0)\nr"
            ),
            6.0,
        );
    }

    #[test]
    fn strict_flip_s1_nofp_number_var_mod_int_literal_compiles() {
        // No-FP (angle A): `number % <int literal>` into a `number` binding —
        // the int literal adopts the number context losslessly; must compile.
        use crate::test_utils::eval_typed_f64;
        assert_eq!(
            eval_typed_f64("let x: number = 3.0\nlet r: number = x % 4\nr"),
            3.0,
        );
    }

    #[test]
    fn strict_flip_s1_nofp_dispatch_result_into_int_compiles() {
        // No-FP (angle A): a concrete method-dispatch result (`[1,2,3].sum()`
        // -> int via the method registry) is NOT an un-provable HOF and must
        // bind cleanly into `int` — the guard must not over-fire on it.
        use crate::test_utils::compile_with_prelude;
        assert!(
            compile_with_prelude("let s: int = [1, 2, 3].sum()\ns").is_ok(),
            "no-FP: concrete dispatch result `.sum()` rejected into int binding"
        );
    }

    // ── strict-flip S1 body-return inference (2026-06-22) ──
    // The un-annotated fn's return type is RESOLVED from its body tail (the HM
    // let-gen the user ruled), including the nested HOF `apply2(g, f, x){g(f(x))}`
    // shape. Three properties: resolved-accept, resolved-mismatch-reject, and
    // HOF-indirection-no-leak (the mismatch is a CLEAN compile error, NEVER a
    // raw-bits reinterpret of `6.0`'s f64 into an i64).

    #[test]
    fn strict_flip_s1_hof_nested_number_resolved_accepts_into_number() {
        // Resolved-accept: `apply2(id, ret_num, 3.0)` resolves to `number`
        // (g=id passthrough, f=ret_num : number->number, x=3.0). A matching
        // `number` annotation binds cleanly and runs to 6.0.
        use crate::test_utils::eval_typed_f64;
        assert_eq!(
            eval_typed_f64(
                "fn id(x) { x }\nfn ret_num(x) { x * 2.0 }\n\
                 fn apply2(g, f, x) { g(f(x)) }\n\
                 let r: number = apply2(id, ret_num, 3.0)\nr"
            ),
            6.0,
        );
    }

    #[test]
    fn strict_flip_s1_hof_nested_int_resolved_accepts_into_int() {
        // Resolved-accept (int twin): `apply2(id, ret_int, 5)` resolves to
        // `int` (f=ret_int : int->int). A matching `int` annotation binds
        // cleanly and runs to 6.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64(
                "fn id(x) { x }\nfn ret_int(x) { x + 1 }\n\
                 fn apply2(g, f, x) { g(f(x)) }\n\
                 let r: int = apply2(id, ret_int, 5)\nr"
            ),
            6,
        );
    }

    #[test]
    fn strict_flip_s1_hof_nested_number_into_int_rejected_no_leak() {
        // Resolved-mismatch-reject + HOF-indirection-no-leak: the SAME
        // `apply2(id, ret_num, 3.0)` resolved to `number` must REJECT cleanly
        // into an `int` binding (number != int) — NEVER reinterpret `6.0`'s
        // f64 bits as the i64 `4618441417868443649`.
        assert!(
            compile_fails(
                "fn id(x) { x }\nfn ret_num(x) { x * 2.0 }\n\
                 fn apply2(g, f, x) { g(f(x)) }\n\
                 let bad: int = apply2(id, ret_num, 3.0)\nbad"
            ),
            "HOF-indirection: number-returning nested HOF accepted into int binding (bit-leak)"
        );
    }

    #[test]
    fn strict_flip_s1_hof_onelevel_number_into_int_rejected() {
        // Resolved-mismatch-reject (1-level): `apply(ret_num, 3.0)` resolves to
        // `number`; an `int` binding rejects cleanly.
        assert!(
            compile_fails(
                "fn apply(f, x) { f(x) }\nfn ret_num(x) { x * 2.0 }\n\
                 let bad: int = apply(ret_num, 3.0)\nbad"
            ),
            "1-level HOF: number-returning HOF accepted into int binding"
        );
    }

    #[test]
    fn strict_flip_s1_hof_closure_callable_unresolvable_rejects() {
        // Genuinely-unresolvable (closure-literal callable): `apply(|y| y*2.0,
        // 3.0)` into `int` cannot be statically resolved through a named
        // callable; it must still REJECT (via the constraint solver / FIX B),
        // NEVER leak. The point is no acceptance of an unproven HOF into int.
        assert!(
            compile_fails("fn apply(f, x) { f(x) }\nlet bad: int = apply(|y| y * 2.0, 3.0)\nbad"),
            "closure-callable HOF: unresolved number result accepted into int binding"
        );
    }
}
