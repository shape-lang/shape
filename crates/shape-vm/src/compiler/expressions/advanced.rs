//! Advanced expression compilation (list comprehension, match, try operator)

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::type_tracking::VariableTypeInfo;
use shape_ast::ast::Expr;
use shape_ast::error::Result;

use shape_runtime::type_system::Type;

use super::super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Compile a list comprehension expression
    pub(super) fn compile_expr_list_comprehension(
        &mut self,
        comp: &shape_ast::ast::ListComprehension,
    ) -> Result<()> {
        self.compile_list_comprehension(comp)
    }

    /// Compile a try operator expression (? operator for Result/Option unwrapping)
    ///
    /// The ? operator unwraps fallible values:
    /// - If Ok(value): unwraps and continues with value
    /// - If Err(error): returns early from the current function with the error
    /// - If None: returns early with an AnyError-compatible Err value
    /// - If Some(value): unwraps and continues with value
    /// - For nullable Option encoding, bare non-None values pass through as success
    ///
    /// The containing function is inferred as fallible and wrapped to Result<T>
    /// by type inference when needed.
    pub(super) fn compile_expr_try_operator(&mut self, inner: &Expr) -> Result<()> {
        // Compile the inner fallible expression.
        self.compile_expr(inner)?;

        // Resource-management-chapter L12 (v0.3.3): when `?` short-circuits
        // (early-returns the Err / propagates None) it must run the pending
        // `Drop` for in-scope Drop-bearing locals — exactly like an explicit
        // `return` does via `emit_drops_for_early_exit`. The `?` early-return
        // happens INSIDE `op_try_unwrap` (it calls `return_value_inner`), so
        // the pending-Drop sequence is guarded by a non-consuming
        // `IsTryFailure` probe and only runs on the failure branch:
        //
        //     <carrier>
        //     Dup ; IsTryFailure          ; [carrier, would_short_circuit]
        //     JumpIfFalse SUCCESS         ; [carrier]   (skip drops on Ok/Some)
        //       <DropCall for each OTHER in-scope Drop local>
        //     SUCCESS:
        //     TryUnwrap                   ; unwrap (success) | early-return (failure)
        //
        // `Dup` clones the carrier's heap share (`clone_with_kind`) so the
        // probe's popped copy and the `TryUnwrap` consumer each own a share —
        // refcount-balanced. The drop sequence (`LoadLocal; DropCall` pairs)
        // is stack-neutral, leaving `[carrier]` for `TryUnwrap`.
        //
        // ADR-006 §2.7.30 double-drop coordination (mirrors commit 47ced8d7):
        // when `inner` is a bare identifier naming a Drop-bearing local, that
        // local's value is the one being PROPAGATED via `?` — its `Drop`
        // ownership moves to the caller (the returned Err carrier holds it),
        // so we must NOT emit a `DropCall` for it here. `emit_drops_for_early_exit`
        // already honours `return_escape_drop_skip_local`; we set it to that
        // local so only the OTHER in-scope Drop locals are released.
        let skip_local = match inner {
            Expr::Identifier(name, _) => self
                .resolve_local(name)
                .filter(|&idx| self.local_drop_kind(idx).is_some()),
            _ => None,
        };
        if self.has_failure_drop_locals(skip_local) {
            self.emit(Instruction::simple(OpCode::Dup));
            self.emit(Instruction::simple(OpCode::IsTryFailure));
            let jump_success = self.emit_jump(OpCode::JumpIfFalse, 0);
            // Failure branch: release every OTHER in-scope Drop local across
            // ALL active drop scopes (the `?` short-circuit returns from the
            // whole function frame, same as `return`).
            let total_scopes = self.drop_locals.len();
            self.return_escape_drop_skip_local = skip_local;
            self.emit_drops_for_early_exit(total_scopes)?;
            self.return_escape_drop_skip_local = None;
            self.patch_jump(jump_success);
        }

        // Emit TryUnwrap opcode which handles:
        // 1. Result propagation (Ok/Err)
        // 2. Option propagation (Some/None)
        // 3. Nullable Option runtime encoding compatibility for bare non-None values
        self.emit(Instruction::simple(OpCode::TryUnwrap));

        // c4-4B (2026-05-28): mark the program as containing `?` so the JIT
        // executor preflight deopts to the bytecode interpreter. `?` lowers
        // in MIR as a transparent `Expr::TryOperator => copy`
        // (`crates/shape-vm/src/mir/lowering/expr.rs:2594`), which discards
        // the unwrap-or-early-return semantics — the JIT emits a plain
        // call+store sequence whose result slot retains the trampoline's
        // raw heap-Result `u64` while the parallel-kind tracker (driven by
        // `stamp_unwrapped_success_type` below) records the SUCCESS type's
        // `NativeKind`. The mismatch surfaces at
        // `crates/shape-jit/src/mir_compiler/terminators.rs:1801-1813`
        // (Return I64-wide arm) as `RETURN_TAG_I64` stamped onto a heap
        // pointer — silent-wrong-output VM=42, JIT=137_900_062_693_984 per
        // `regression::jit::jit_trampoline_result_callvalue`. Whole-program
        // deopt mirrors the R8 W7 G.5 V2-verifier + R8 W8 imported-const-
        // inline + R8 W9 B1 W17-marshal-return surface-and-stop pattern.
        // Per supervisor 2026-05-28 c4-4B ratification (audit doc
        // `docs/cluster-audits/v0.3.3/04-pointer-as-float-leak.md` §4B
        // Sub-cluster — FN-REG-CORRECTNESS / RELEASE-BLOCKING).
        self.program.has_try_unwrap_residual = true;

        // WS-3 F2a: stamp the compile-time type tracker with the type of the
        // UNWRAPPED success value. The `?` operator yields the inner `T` of a
        // `Result<T, E>` / `Option<T>` (or `T?`); the runtime inference engine
        // already does this (`try_unwrap_inner_type`,
        // `type_system/inference/expressions.rs`), but the bytecode compiler's
        // parallel tracker did not. Without this stamp, a `let v = expr?`
        // binding records slot `v` with no type, and a later `v + 1` fails
        // strict typing as `unknown + int`. We mirror the runtime engine's
        // unwrap onto `last_expr_*` so `propagate_initializer_type_to_slot`
        // records the unwrapped type on the binding's slot.
        self.stamp_unwrapped_success_type(inner);
        Ok(())
    }

    /// WS-3 F2a: infer the `?`-operand's type, unwrap the `Result`/`Option`
    /// wrapper to the success type, and stamp `last_expr_*` so a downstream
    /// `let`-binding records the unwrapped type on its slot.
    ///
    /// Also reused for the `!!` error-context operator (WS-3 F3): both `?`
    /// and `!!` yield the UNWRAPPED success value `T` on the success leg
    /// (`Ok(v) => v`), so both stamp the same unwrapped type.
    pub(super) fn stamp_unwrapped_success_type(&mut self, inner: &Expr) {
        // Clear any stale stamps from `compile_expr(inner)` first — the
        // unwrapped value's type, not the wrapper's, is what flows out.
        self.last_expr_type_info = None;
        self.last_expr_numeric_type = None;
        self.last_expr_schema = None;

        // Primary path: full type inference on the `?` operand. Covers
        // `Ok(literal)?`, `inline_fn()?`, etc. Returns a `Type::Generic`
        // / `Type::Concrete(Generic)` / `Type::Concrete(Basic("Result<...>"))`
        // shape from which the success arm can be peeled.
        let inferred = self.infer_expr_type(inner).ok();

        // PB2-fix #8 (let-binding-on-fn-result-identifier): the runtime
        // inference engine is module-scope only — it does NOT see
        // function-local `let r = inner()` bindings. For
        // `Expr::Identifier(name)` that is a local with a tracker-recorded
        // `Result<T>` / `Option<T>` type-name, fall back to the tracker.
        // Same shape as the `BuiltinTypes::is_integer_type_name` / scalar
        // fallback at `crates/shape-vm/src/compiler/expressions/mod.rs:1372`,
        // extended to the parameterized fallible wrappers.
        let inner_ty = inferred.or_else(|| {
            use shape_ast::ast::TypeAnnotation;
            use shape_runtime::type_system::Type;
            if let Expr::Identifier(name, _) = inner {
                if let Some(type_name) = self.tracker_type_name_for_identifier(name) {
                    let trimmed = type_name.trim();
                    if trimmed.starts_with("Result<") || trimmed.starts_with("Option<") {
                        return Some(Type::Concrete(TypeAnnotation::Basic(type_name)));
                    }
                }
            }
            None
        });

        let Some(inner_ty) = inner_ty else {
            return;
        };

        // Primary success-arm extraction.
        let mut success_name = Self::try_operator_success_type_name(&inner_ty);

        // PB2-fix #6 (`Err("...")?` / `Ok(literal)?` with unresolved
        // success-arm TypeVar): when the inner `?` operand is a `Result<T,E>`
        // / `Option<T>` whose success arm is an unresolved type variable
        // (`Err(string)` has no Ok-arm to constrain T; the engine returns
        // `Result<TypeVar, string>`), fall back to the enclosing function's
        // DECLARED return type. By contract, `?` inside a `fn -> Result<T>`
        // / `fn -> Option<T>` propagates Err/None and unwraps to that T —
        // so the enclosing return type's success arm IS the unwrapped type
        // the let-binding will receive on the success leg.
        if success_name.is_none() && self.success_arm_is_unresolved_typevar(&inner_ty) {
            if let Some(name) = self.enclosing_function_return_success_name() {
                success_name = Some(name);
            }
        }

        if let Some(name) = success_name {
            self.stamp_last_expr_from_type_name(&name);
        }
    }

    /// Returns `true` when `ty` is a `Result<T, E>` / `Option<T>` /
    /// `T?` whose SUCCESS arm is an unresolved `Type::Variable`. Used to
    /// gate the PB2-fix #6 enclosing-fn fallback narrowly — we ONLY
    /// substitute the enclosing return type when the inner expression
    /// genuinely failed to resolve its success arm; resolved-but-mismatched
    /// success types must surface as type errors, not get silently rewritten.
    fn success_arm_is_unresolved_typevar(&self, ty: &shape_runtime::type_system::Type) -> bool {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;

        let is_fallible_name = |n: &str| n == "Result" || n == "Option";

        match ty {
            Type::Generic { base, args } if !args.is_empty() => {
                let base_name = match base.as_ref() {
                    Type::Concrete(ann) => ann.as_type_name_str().map(str::to_string),
                    _ => None,
                };
                match base_name {
                    Some(n) if is_fallible_name(&n) => {
                        matches!(args[0], Type::Variable(_))
                    }
                    _ => false,
                }
            }
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if is_fallible_name(name) && !args.is_empty() =>
            {
                // `args[0]: TypeAnnotation` — the only annotation shape
                // that round-trips through `to_type_string` as `"unknown"`
                // is a fully-unresolved variable.
                args[0].to_type_string() == "unknown"
            }
            Type::Concrete(TypeAnnotation::Basic(s)) => {
                // Baked form: `Result<unknown, string>` etc.
                Self::first_generic_arg_of_baked_name(s, is_fallible_name).as_deref()
                    == Some("unknown")
            }
            _ => false,
        }
    }

    /// PB2-fix #6 helper: peel the SUCCESS arm out of the enclosing
    /// function's declared `return_type` when it is `Result<T>` /
    /// `Result<T, E>` / `Option<T>`. Returns `None` if there is no current
    /// function, the function has no declared return type, or its declared
    /// return type is not a fallible wrapper.
    fn enclosing_function_return_success_name(&self) -> Option<String> {
        use shape_ast::ast::TypeAnnotation;

        let func_idx = self.current_function?;
        let func_name = self
            .program
            .functions
            .get(func_idx)
            .map(|f| f.name.clone())?;
        let func_def = self.function_defs.get(&func_name)?;
        let ann = func_def.return_type.as_ref()?;
        let is_fallible_name = |n: &str| n == "Result" || n == "Option";
        match ann {
            TypeAnnotation::Generic { name, args }
                if is_fallible_name(name) && !args.is_empty() =>
            {
                Some(args[0].to_type_string())
            }
            TypeAnnotation::Basic(s) => Self::first_generic_arg_of_baked_name(s, is_fallible_name),
            _ => None,
        }
    }

    /// Extract the success type-name of a `Result<T, E>` / `Option<T>` /
    /// `T?` type. Handles every shape `infer_expr_type` can produce:
    /// a `Type::Generic` whose base is the `Result`/`Option` name, a
    /// `Type::Concrete(Generic { name, .. })`, and the
    /// `Type::Concrete(Basic("Result<int, string>"))` string-baked form the
    /// function-return-type hint table produces. Returns `None` when the
    /// operand is not a recognised fallible wrapper.
    fn try_operator_success_type_name(ty: &shape_runtime::type_system::Type) -> Option<String> {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;

        let is_fallible_name = |n: &str| n == "Result" || n == "Option";

        match ty {
            Type::Generic { base, args } if !args.is_empty() => {
                let base_name = match base.as_ref() {
                    Type::Concrete(ann) => ann.as_type_name_str()?.to_string(),
                    _ => return None,
                };
                if !is_fallible_name(&base_name) {
                    return None;
                }
                Self::type_to_simple_name(&args[0])
            }
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if is_fallible_name(name) && !args.is_empty() =>
            {
                Some(args[0].to_type_string())
            }
            Type::Concrete(TypeAnnotation::Basic(s)) => {
                // The function-return-type hint table bakes generics into a
                // single `Basic` string, e.g. `Result<int, string>`. Split
                // off the first type argument at angle-bracket depth 0.
                Self::first_generic_arg_of_baked_name(s, is_fallible_name)
            }
            _ => None,
        }
    }

    /// Render a `Type` to a simple type-name string (best effort).
    fn type_to_simple_name(ty: &shape_runtime::type_system::Type) -> Option<String> {
        use shape_runtime::type_system::Type;
        match ty {
            Type::Concrete(ann) => Some(ann.to_type_string()),
            Type::Generic { .. } => Self::inferred_type_to_hint_name(ty),
            _ => None,
        }
    }

    /// Given a baked generic name like `Result<int, string>`, verify the base
    /// name is a fallible wrapper and return its FIRST type argument
    /// (`int`). Bracket-depth-aware so nested generics in later args do not
    /// confuse the split.
    fn first_generic_arg_of_baked_name(
        s: &str,
        is_fallible_name: impl Fn(&str) -> bool,
    ) -> Option<String> {
        let open = s.find('<')?;
        let base = s[..open].trim();
        if !is_fallible_name(base) {
            return None;
        }
        if !s.ends_with('>') {
            return None;
        }
        let inner = &s[open + 1..s.len() - 1];
        let mut depth = 0usize;
        for (i, ch) in inner.char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    return Some(inner[..i].trim().to_string());
                }
                _ => {}
            }
        }
        Some(inner.trim().to_string())
    }

    /// Stamp `last_expr_*` from a resolved success type-name so a
    /// downstream `let`-binding's `propagate_initializer_type_to_slot`
    /// records the type. Mirrors the `tracked_callable_rt` stamping in
    /// `compile_expr_function_call`.
    fn stamp_last_expr_from_type_name(&mut self, name: &str) {
        use crate::type_tracking::{NumericType, VariableTypeInfo};
        use shape_runtime::type_system::BuiltinTypes;

        match name {
            "int" => self.last_expr_numeric_type = Some(NumericType::Int),
            "number" => self.last_expr_numeric_type = Some(NumericType::Number),
            "decimal" => self.last_expr_numeric_type = Some(NumericType::Decimal),
            "string" | "bool" | "char" | "bigint" => {
                self.last_expr_type_info = Some(VariableTypeInfo::named(name.to_string()));
            }
            other if BuiltinTypes::is_integer_type_name(other) => {
                // Width-aware ints (i8/u8/i16/...): the name round-trips via
                // `type_info`; `propagate_assignment_type_to_slot` re-derives
                // the storage hint from the recorded name.
                self.last_expr_type_info = Some(VariableTypeInfo::named(other.to_string()));
            }
            other => {
                // A user struct / enum success type — stamp the schema so the
                // binding inherits it. Strip any generic args before lookup.
                let base = other.split('<').next().unwrap_or(other).trim();
                if let Some(schema) = self.type_tracker.schema_registry().get(base) {
                    self.last_expr_schema = Some(schema.id);
                }
            }
        }
    }

    /// STAGE-P1 (v0.3.3 strict-flip): recover a match scrutinee's `ConcreteType`
    /// from the keystone expr-type-table when the structural
    /// `concrete_type_for_expr` resolver declines.
    ///
    /// The structural resolver does not project a function CALL's declared
    /// return type, so `match g() { Ok(p) => p.x + p.y }` (where `g() ->
    /// Result<Point,string>`) reaches `compile_match_binding` with no scrutinee
    /// ConcreteType, the `Ok(p)` payload binder is never stamped with `Point`,
    /// and `p.x` / `p.y` erase to `unknown` at the binop. The inference engine
    /// already proved the scrutinee's type and recorded it in
    /// `resolved_expr_types` keyed by span (the T1 keystone). Read that proven
    /// type back and convert it to a `ConcreteType` via the same declared-
    /// annotation projection the explicit-annotation path uses
    /// (`declared_annotation_concrete_type`). No fabrication: the table holds
    /// only fully-resolved types (free vars are dropped by
    /// `finalize_expr_type_table`), and a type that does not project to a
    /// `ConcreteType` (or a dummy span) yields `None`, preserving the prior
    /// surface-and-stop behaviour.
    fn keystone_scrutinee_concrete_type(
        &self,
        scrutinee: &Expr,
    ) -> Option<shape_value::v2::ConcreteType> {
        let span = shape_ast::ast::Spanned::span(scrutinee);
        if span.is_dummy() {
            return None;
        }
        let resolved = self.resolved_expr_types.get(&span)?;
        let ann = resolved.to_annotation()?;
        crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
            self, &ann,
        )
    }

    /// Compile a match expression
    pub(super) fn compile_expr_match(
        &mut self,
        match_expr: &shape_ast::ast::MatchExpr,
    ) -> Result<()> {
        // Check exhaustiveness before compiling
        self.check_match_exhaustiveness(match_expr)?;

        self.push_scope();
        // F5 (v0.3.3 strict-flip): capture the scrutinee's proven ConcreteType
        // BEFORE compiling it (compilation may clobber `last_expr_*`). Threaded
        // into `compile_match_binding` so `Ok(v)`/`Some(v)`/`Err(e)` payload
        // unwraps stamp the binder type from `Result(T,E)` / `Option(T)`.
        let scrutinee_ct =
            crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                self,
                &match_expr.scrutinee,
            )
            // STAGE-P1 (v0.3.3 strict-flip): when the structural
            // `concrete_type_for_expr` declines (e.g. the scrutinee is a CALL —
            // `match g() { Ok(p) => p.x + p.y }` — whose declared
            // `Result<Point,string>` return type the structural resolver does
            // not project), fall back to the keystone expr-type-table. The
            // inference engine walked the full program and recorded `g()`'s
            // resolved type (`Result<Point,string>`) keyed by the scrutinee's
            // source span; converting that proven type to a `ConcreteType` lets
            // `compile_match_binding` thread the payload (`Point`) onto the
            // `Ok(p)` binder so `p.x` / `p.y` resolve instead of erasing to
            // `unknown` at the binop. This reads inference's own output — no
            // fabrication: a miss (free var dropped by `finalize_expr_type_table`)
            // leaves `scrutinee_ct` None and the prior surface-and-stop behaviour.
            .or_else(|| self.keystone_scrutinee_concrete_type(&match_expr.scrutinee));
        self.compile_expr(&match_expr.scrutinee)?;
        let scrutinee_local = self.declare_local("__match_scrutinee")?;
        if let Some(schema_id) = self.last_expr_schema {
            self.type_tracker.set_local_type(
                scrutinee_local,
                VariableTypeInfo::known(schema_id, format!("__typed_obj_{}", schema_id)),
            );
        }
        // Propagate full type info (numeric type, storage hint) from the
        // scrutinee expression so that match bindings inherit it.
        self.propagate_initializer_type_to_slot(scrutinee_local, true, false);
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(scrutinee_local)),
        ));

        let mut end_jumps = Vec::new();
        let mut arm_reference_results = Vec::new();

        // Capture scrutinee type info for restoring before each arm's binding.
        // This includes schema_id, numeric type, and full type_info so that
        // match binding variables inherit the scrutinee's compile-time type.
        let scrutinee_schema = self
            .type_tracker
            .get_local_type(scrutinee_local)
            .and_then(|info| info.schema_id);
        // Post-§2.7.5.1: `info.storage_hint` is `Option<StorageHint>`,
        // so `.and_then` collapses both Option layers. `None` propagates
        // "kind not yet proven" — no numeric type recorded.
        let scrutinee_numeric_type = self
            .type_tracker
            .get_local_type(scrutinee_local)
            .and_then(|info| info.storage_hint)
            .and_then(Self::storage_hint_to_numeric_type);
        let scrutinee_type_info = self.type_tracker.get_local_type(scrutinee_local).cloned();

        for arm in &match_expr.arms {
            // Resolve the pattern-identifier-vs-unit-variant ambiguity ONCE,
            // up front, so every downstream pass (check, binding, and the
            // binding-semantics walks below) sees the same refutable
            // `Constructor` pattern instead of a catch-all `Identifier`
            // binder. A bare capitalized `Red` that names a known unit enum
            // variant becomes `Enum::Red` here; everything else is unchanged.
            let normalized = self.normalize_unit_variant_pattern(&arm.pattern);
            let arm_pattern: &shape_ast::ast::Pattern =
                normalized.as_ref().unwrap_or(&arm.pattern);

            // Pattern check — restore scrutinee schema before checking
            self.last_expr_schema = scrutinee_schema;
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(scrutinee_local)),
            ));
            self.compile_pattern_check(arm_pattern, arm.pattern_span)?;
            let next_arm_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

            // Guard (if present) evaluated with bindings
            let mut guard_fail_jump = None;
            if let Some(guard) = &arm.guard {
                self.push_scope();
                self.last_expr_schema = scrutinee_schema;
                self.last_expr_numeric_type = scrutinee_numeric_type;
                self.last_expr_type_info = scrutinee_type_info.clone();
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(scrutinee_local)),
                ));
                self.compile_match_binding(arm_pattern, scrutinee_ct.as_ref())?;
                self.compile_expr(guard)?;
                guard_fail_jump = Some(self.emit_jump(OpCode::JumpIfFalse, 0));
                self.pop_scope();
            }

            // Arm body with bindings
            self.push_scope();
            self.last_expr_schema = scrutinee_schema;
            self.last_expr_numeric_type = scrutinee_numeric_type;
            self.last_expr_type_info = scrutinee_type_info.clone();
            self.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(scrutinee_local)),
            ));
            self.compile_match_binding(arm_pattern, scrutinee_ct.as_ref())?;
            if self.current_expr_result_mode() == crate::compiler::ExprResultMode::PreserveRef {
                self.compile_expr_preserving_refs(&arm.body)?;
            } else {
                self.compile_expr(&arm.body)?;
            }
            arm_reference_results.push(self.capture_last_expr_reference_result());
            self.pop_scope();

            let end_jump = self.emit_jump(OpCode::Jump, 0);
            end_jumps.push(end_jump);

            // Patch failure jumps to the next arm
            self.patch_jump(next_arm_jump);
            if let Some(jump) = guard_fail_jump {
                self.patch_jump(jump);
            }
        }

        // No match - raise runtime error
        let msg = self.program.add_constant(Constant::String(
            "No match arm matched the value".to_string(),
        ));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(msg)),
        ));
        self.emit(Instruction::simple(OpCode::Throw));

        for jump in end_jumps {
            self.patch_jump(jump);
        }
        self.restore_last_expr_reference_result(Self::merge_reference_results(
            &arm_reference_results,
        ));
        self.pop_scope();
        Ok(())
    }

    /// Compile `async let name = expr`
    ///
    /// Semantics: spawn the RHS expression as a concurrent task and bind
    /// the resulting Future to a local variable.  The future is later
    /// consumed with `await name`.
    ///
    /// Bytecode:
    ///   compile(expr)   -- push the value / closure onto the stack
    ///   SpawnTask       -- pop value, push Future(task_id)
    ///   StoreLocal(slot)-- bind the future to `name`
    ///   LoadLocal(slot) -- push it back so `async let` is an expression
    pub(super) fn compile_async_let(
        &mut self,
        async_let: &shape_ast::ast::AsyncLetExpr,
    ) -> Result<()> {
        if !self.current_function_is_async {
            return Err(shape_ast::error::ShapeError::SemanticError {
                message: "'async let' can only be used inside an async function".to_string(),
                location: None,
            });
        }

        // ── Three concurrency rules at task boundary ──
        // 1. Owned values (move/clone): always allowed
        // 2. &T (shared ref): allowed in structured child tasks
        // 3. &mut T (exclusive ref): FORBIDDEN — would create aliased mutation
        //
        // Walk the RHS expression to detect exclusive references crossing the boundary.
        self.check_task_boundary_safety(&async_let.expr, async_let.span)?;
        self.plan_flexible_binding_escape_from_expr(&async_let.expr);

        // Closure spec Phase G §5.5: a closure literal crossing a
        // detached task boundary (`async let c = || ...`) must use the
        // heap-ABI opcode — the Cranelift stack slot a non-escaping
        // closure would live in cannot outlive the spawning frame. Mark
        // the next `MakeClosure` emission to use `MakeClosureHeap` so
        // the MIR storage planner + JIT codegen see the escape signal.
        // Phase B's escape analysis flags `TaskBoundary` operands as
        // escaping (storage_planning.rs rows 5-6) so non-literal
        // closure operands already fall back to the heap ABI; this
        // hook covers the literal case where MIR never sees a
        // `TaskBoundary` for the closure slot (the literal is inlined
        // into the spawn expression).
        if matches!(&*async_let.expr, Expr::FunctionExpr { .. }) {
            self.emit_make_closure_heap_next = true;
        }

        // Compile the RHS expression
        self.compile_expr(&async_let.expr)?;

        // Spawn it as an async task — replaces top-of-stack value with Future(id)
        self.emit(Instruction::simple(OpCode::SpawnTask));

        // Declare a local variable for the future and store it
        let local_idx = self.declare_local(&async_let.name)?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(local_idx)),
        ));
        self.immutable_locals.insert(local_idx);
        self.type_tracker
            .set_local_binding_semantics(local_idx, Self::owned_immutable_binding_semantics());

        // Propagate the RHS expression's type to the local slot. Without a
        // surface `Future<T>` type in the tracker lattice, the binding's
        // type unifies with the RHS expression's type — the runtime
        // sync-resolution path at `async_ops/mod.rs::op_spawn_task`'s
        // non-callable arm preserves the inner value's kind end-to-end, so
        // `let va = await a` later resolves `va` to the same kind as the
        // original RHS. Without this propagation, multi-binding patterns
        // like `let va = await a; let vb = await b; print(va + vb)` fail
        // strict typing as `unknown + unknown` because the post-`compile_expr`
        // type info is unread by `compile_async_let`.
        //
        // `propagate_assignment_type_to_slot` reads `last_expr_type_info` /
        // `last_expr_numeric_type` / `last_expr_schema` set by
        // `compile_expr(&async_let.expr)` above and stamps the local
        // accordingly — same shape as `var_decl.value`-less local
        // declarations in `compile_statement::Let`.
        self.propagate_initializer_type_to_slot(local_idx, true, false);

        // `async let` is an expression — push the future back onto the stack
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(local_idx)),
        ));

        Ok(())
    }

    /// Compile `async scope { body }`
    ///
    /// Semantics: open a structured concurrency boundary, execute the body,
    /// then cancel all tasks spawned inside the scope that are still pending
    /// (LIFO order).  Nested scopes are independent.
    ///
    /// Bytecode:
    ///   AsyncScopeEnter   -- push scope marker onto async scope stack
    ///   compile(body)     -- body may spawn tasks via `async let`
    ///   AsyncScopeExit    -- cancel pending tasks in LIFO order, leave result
    pub(super) fn compile_async_scope(&mut self, inner: &Expr) -> Result<()> {
        if !self.current_function_is_async {
            return Err(shape_ast::error::ShapeError::SemanticError {
                message: "'async scope' can only be used inside an async function".to_string(),
                location: None,
            });
        }

        // Enter structured concurrency scope
        self.emit(Instruction::simple(OpCode::AsyncScopeEnter));

        // Closure spec Phase G §5.5: conservative v1 for structured
        // boundaries — if the scope's result expression is a closure
        // literal (possibly wrapped in a single-item block), force
        // heap allocation. Future work per §9.6 is to prove that the
        // child future's lifetime is bounded by the parent frame and
        // allow stack closures across structured boundaries.
        if Self::scope_result_is_closure_literal(inner) {
            self.emit_make_closure_heap_next = true;
        }

        // Compile the body — any `async let` inside will spawn tasks tracked by this scope
        self.compile_expr(inner)?;

        // Exit scope — cancels all pending tasks spawned within
        self.emit(Instruction::simple(OpCode::AsyncScopeExit));

        Ok(())
    }

    /// Determine whether the final value produced by an `async scope`
    /// body is a closure literal. Handles the common shapes:
    ///   - `async scope { || 1 }` → `Expr::Block` wrapping one
    ///     `Expr::FunctionExpr`
    ///   - `async scope(|| 1)` → bare `Expr::FunctionExpr`
    fn scope_result_is_closure_literal(expr: &Expr) -> bool {
        match expr {
            Expr::FunctionExpr { .. } => true,
            Expr::Block(block, _) => {
                // The scope's value is the last block item (if it is an
                // expression). A trailing statement produces unit, so no
                // closure can flow out.
                block.items.last().is_some_and(|item| match item {
                    shape_ast::ast::BlockItem::Expression(e) => {
                        Self::scope_result_is_closure_literal(e)
                    }
                    _ => false,
                })
            }
            _ => false,
        }
    }

    /// Check that an expression being spawned as a concurrent task doesn't
    /// capture exclusive (`&mut`) references from the enclosing scope.
    ///
    /// Three concurrency rules:
    /// - Owned values (move/clone): always allowed across task boundary
    /// - `&T` (shared ref): allowed in structured child tasks (truly immutable)
    /// - `&mut T` (exclusive ref): FORBIDDEN (would create aliased mutation)
    fn check_task_boundary_safety(&self, expr: &Expr, span: shape_ast::ast::Span) -> Result<()> {
        // Check for explicit &mut references in the expression
        self.walk_expr_for_exclusive_refs(expr, span)
    }

    /// Walk an expression tree looking for exclusive references that would
    /// cross a task boundary. Reports the first one found.
    fn walk_expr_for_exclusive_refs(
        &self,
        expr: &Expr,
        boundary_span: shape_ast::ast::Span,
    ) -> Result<()> {
        use shape_ast::error::ShapeError;

        match expr {
            // Direct &mut reference — forbidden across task boundary
            Expr::Reference {
                is_mutable: true,
                span,
                ..
            } => {
                return Err(ShapeError::SemanticError {
                    message: "cannot share exclusive reference (&mut) across task boundary — \
                        exclusive references cannot cross into spawned tasks because they would \
                        create aliased mutation. Use an owned value (clone) or a shared reference (&) instead"
                        .to_string(),
                    location: Some(self.span_to_source_location(*span)),
                });
            }

            // Shared refs are OK — recurse into sub-expr for any nested &mut
            Expr::Reference {
                expr: inner,
                is_mutable: false,
                ..
            } => {
                self.walk_expr_for_exclusive_refs(inner, boundary_span)?;
            }

            // Identifier that resolves to an exclusive ref local
            Expr::Identifier(name, id_span) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    if self.exclusive_ref_locals.contains(&local_idx) {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "cannot share exclusive reference '{}' across task boundary — \
                                exclusive references cannot cross into spawned tasks because they \
                                would create aliased mutation. Use an owned value (clone) or a \
                                shared reference (&) instead",
                                name
                            ),
                            location: Some(self.span_to_source_location(*id_span)),
                        });
                    }
                }
            }

            // Recurse into sub-expressions
            Expr::FunctionCall { args, .. } => {
                for arg in args {
                    self.walk_expr_for_exclusive_refs(arg, boundary_span)?;
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.walk_expr_for_exclusive_refs(left, boundary_span)?;
                self.walk_expr_for_exclusive_refs(right, boundary_span)?;
            }
            Expr::UnaryOp { operand, .. } => {
                self.walk_expr_for_exclusive_refs(operand, boundary_span)?;
            }
            Expr::FunctionExpr { .. } => {
                // Function expressions create a new scope — captures are checked at call site
            }
            Expr::Block(block_expr, _) => {
                for item in &block_expr.items {
                    match item {
                        shape_ast::ast::BlockItem::Expression(e) => {
                            self.walk_expr_for_exclusive_refs(e, boundary_span)?;
                        }
                        shape_ast::ast::BlockItem::Statement(
                            shape_ast::ast::Statement::Expression(e, _),
                        ) => {
                            self.walk_expr_for_exclusive_refs(e, boundary_span)?;
                        }
                        _ => {}
                    }
                }
            }
            Expr::PropertyAccess { object, .. } => {
                self.walk_expr_for_exclusive_refs(object, boundary_span)?;
            }
            Expr::MethodCall { receiver, args, .. } => {
                self.walk_expr_for_exclusive_refs(receiver, boundary_span)?;
                for arg in args {
                    self.walk_expr_for_exclusive_refs(arg, boundary_span)?;
                }
            }
            Expr::IndexAccess { object, index, .. } => {
                self.walk_expr_for_exclusive_refs(object, boundary_span)?;
                self.walk_expr_for_exclusive_refs(index, boundary_span)?;
            }
            Expr::If(if_expr, _) => {
                self.walk_expr_for_exclusive_refs(&if_expr.condition, boundary_span)?;
                self.walk_expr_for_exclusive_refs(&if_expr.then_branch, boundary_span)?;
                if let Some(eb) = &if_expr.else_branch {
                    self.walk_expr_for_exclusive_refs(eb, boundary_span)?;
                }
            }
            Expr::Array(elems, _) => {
                for elem in elems {
                    self.walk_expr_for_exclusive_refs(elem, boundary_span)?;
                }
            }
            Expr::Await(inner, _) => {
                self.walk_expr_for_exclusive_refs(inner, boundary_span)?;
            }
            // Leaf expressions (literals, etc.) — no refs to check
            _ => {}
        }
        Ok(())
    }

    /// Check exhaustiveness of a match expression
    ///
    /// Uses the type inference engine to determine the scrutinee type and checks
    /// if all enum variants are covered. Returns a compile error for non-exhaustive matches.
    ///
    /// Note: This requires the type inference engine to have full program context.
    /// If type inference fails (e.g., undefined variable), we skip the check gracefully
    /// since the type inference engine needs full integration to track all program state.
    fn check_match_exhaustiveness(&mut self, match_expr: &shape_ast::ast::MatchExpr) -> Result<()> {
        use shape_runtime::type_system::exhaustiveness;

        // Try to infer scrutinee type
        // If this fails (e.g., undefined variable), fall back to parameter type annotations
        let scrutinee_type = match self.infer_expr_type(&match_expr.scrutinee) {
            Ok(t) => t,
            Err(_) => {
                // Fallback: if scrutinee is a parameter with a type annotation, use it
                if let shape_ast::ast::Expr::Identifier(name, _) = &*match_expr.scrutinee {
                    if let Some(ty) = self.lookup_param_type_annotation(name) {
                        ty
                    } else {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            }
        };

        // Check exhaustiveness for closed types (enums, unions).
        // Union scrutinees must use typed-pattern coverage against their concrete variants.
        let result = if matches!(
            scrutinee_type.to_annotation(),
            Some(shape_ast::ast::TypeAnnotation::Union(_))
        ) {
            exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type)
        } else if let Some(semantic_type) = scrutinee_type.to_semantic() {
            let resolved_type = self.type_inference.resolve_named_to_enum(&semantic_type);
            match resolved_type {
                shape_runtime::type_system::semantic::SemanticType::Enum { .. } => {
                    exhaustiveness::check_exhaustiveness(match_expr, &resolved_type)
                }
                _ => exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type),
            }
        } else {
            exhaustiveness::check_exhaustiveness_for_type(match_expr, &scrutinee_type)
        };

        // Non-exhaustive matches are ERRORS (not warnings)
        // Without exhaustiveness, match type cannot be determined (no null, no auto-Option<T>)
        match result {
            exhaustiveness::ExhaustivenessResult::NonExhaustive {
                enum_name,
                missing_variants,
            } => Err(shape_ast::error::ShapeError::SemanticError {
                message: format!(
                    "Non-exhaustive match on '{}': missing variants: {}",
                    enum_name,
                    missing_variants.join(", ")
                ),
                location: None,
            }),
            _ => Ok(()), // Exhaustive or not applicable
        }
    }

    /// Look up a parameter's type annotation from the current function's parameter list.
    fn lookup_param_type_annotation(&self, name: &str) -> Option<Type> {
        for param in &self.current_function_params {
            if param.pattern.as_identifier() == Some(name) {
                if let Some(ann) = &param.type_annotation {
                    return Some(Type::Concrete(ann.clone()));
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::bytecode::{OpCode, Operand};
    use crate::compiler::BytecodeCompiler;
    use shape_ast::parser::parse_program;

    #[test]
    fn test_match_expression_compiles() {
        // Basic match expression should compile
        let code = r#"
            enum Color { Red, Green, Blue }

            let result = match Color::Red {
                Color::Red => 1,
                Color::Green => 2,
                Color::Blue => 3
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);

        assert!(
            result.is_ok(),
            "Match expression should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_match_with_wildcard() {
        // Match with wildcard pattern should compile
        let code = r#"
            enum Color { Red, Green, Blue }

            let result = match Color::Red {
                Color::Red => 1,
                _ => 2
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);

        assert!(
            result.is_ok(),
            "Match with wildcard should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_typed_union_match_without_wildcard_compiles() {
        let code = r#"
            let result = match (1 as int | string) {
                n: int => n,
                s: string => 0
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "Typed union match should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_typed_union_match_missing_variant_fails_compile() {
        let code = r#"
            let result = match (1 as int | string) {
                n: int => n
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "Missing typed union arm should fail compilation"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("Non-exhaustive match"),
            "Expected non-exhaustive diagnostic, got: {}",
            msg
        );
    }

    #[test]
    fn test_match_binding_is_immutable() {
        let code = r#"
            function test() {
                let source = Some(1)
                return match source {
                    Some(x) => {
                        x = 2
                        x
                    }
                    None => 0
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(result.is_err(), "match binding reassignment should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("cannot assign to immutable binding 'x'"),
            "unexpected error: {}",
            err_msg
        );
    }

    #[test]
    fn test_exhaustiveness_checker_integrated() {
        // Verify that check_match_exhaustiveness method exists and is called
        // This is a smoke test to ensure the integration is in place
        let code = r#"
            enum Status { Active, Inactive }

            let result = match Status::Active {
                Status::Active => 1,
                Status::Inactive => 2
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);

        // Should compile successfully
        assert!(
            result.is_ok(),
            "Exhaustive match should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_type_inference_engine_initialized() {
        // Verify that BytecodeCompiler has type_inference field
        let compiler = BytecodeCompiler::new();

        // Access the type_inference field to ensure it's initialized
        // This is a compile-time check that the field exists
        let _type_inference = &compiler.type_inference;
    }

    /// Test that exhaustiveness checking infrastructure is in place
    ///
    /// Note: Full exhaustiveness checking requires program-wide type inference
    /// to track variable types. The infrastructure is in place (type_inference engine,
    /// check_match_exhaustiveness method, exhaustiveness::check_exhaustiveness call),
    /// but comprehensive testing requires completing the type inference integration.
    ///
    /// Current status:
    /// - ✅ Type inference engine added to compiler
    /// - ✅ check_match_exhaustiveness method implemented
    /// - ✅ Integration into compile_expr_match
    /// - ⏳ Full type inference pass integration (needed for variable type tracking)
    #[test]
    fn test_exhaustiveness_infrastructure_present() {
        // This test verifies the code structure is in place
        // Actual exhaustiveness checking will be tested once full type inference
        // is integrated (which requires tracking enum types and variable types
        // throughout the program compilation)

        let code = r#"
            enum SimpleEnum { A, B }
            match SimpleEnum::A {
                SimpleEnum::A => 1,
                SimpleEnum::B => 2
            }
        "#;

        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);

        // Should compile (infrastructure is in place)
        assert!(
            result.is_ok(),
            "Infrastructure test should compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_unknown_enum_error_highlights_pattern_not_body() {
        // When an unknown enum is used in a match pattern, the error should
        // point to the pattern (Snapshot::Hash), not the arm body.
        let code =
            "match 42 {\n  Snapshot::Hash(id) => print(\"saved\"),\n  _ => print(\"other\"),\n}\n";
        let program = parse_program(code).expect("Failed to parse");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(code);
        let result = compiler.compile(&program);
        assert!(result.is_err(), "Should fail for unknown enum");
        let err = result.unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("Unknown enum type"),
            "Error should mention unknown enum type, got: {}",
            msg
        );
        // Verify the error location points to the pattern, not the body
        if let shape_ast::error::ShapeError::SemanticError {
            location: Some(loc),
            ..
        } = &err
        {
            // Pattern "Snapshot::Hash(id)" is on line 2, starting at column 3
            // Body "print(\"saved\")" is further right on the same line
            // The error should point at the pattern (column <= ~20), not at the body (column ~25+)
            assert_eq!(
                loc.line, 2,
                "Error should be on line 2, got line {}",
                loc.line
            );
            assert!(
                loc.column <= 20,
                "Error column should point to pattern start, not body. Got column {}",
                loc.column
            );
        } else {
            panic!("Expected SemanticError with location, got: {:?}", err);
        }
    }

    // ===== Sprint 5: Async Join Compiler Tests =====

    #[test]
    fn test_join_outside_async_is_error() {
        // Using `await join` outside an async function should produce a semantic error
        let code = r#"
            function not_async() {
                await join all {
                    1,
                    2,
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "await join outside async should produce an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("async"),
            "Error should mention async, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_join_all_compiles_in_async() {
        // `await join all { ... }` inside an async function should compile
        // Use simple literal expressions to avoid "undefined function" errors
        let code = r#"
            async function fetch_all() {
                await join all {
                    1 + 2,
                    3 + 4,
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "await join all in async function should compile: {:?}",
            result.err()
        );

        // Verify opcode sequence contains SpawnTask, JoinInit, JoinAwait
        let bytecode = result.unwrap();
        let instructions = &bytecode.instructions;
        let opcodes: Vec<_> = instructions.iter().map(|i| i.opcode).collect();

        assert!(
            opcodes.contains(&OpCode::SpawnTask),
            "Should contain SpawnTask opcode, got: {:?}",
            opcodes
        );
        assert!(
            opcodes.contains(&OpCode::JoinInit),
            "Should contain JoinInit opcode, got: {:?}",
            opcodes
        );
        assert!(
            opcodes.contains(&OpCode::JoinAwait),
            "Should contain JoinAwait opcode, got: {:?}",
            opcodes
        );

        // Count SpawnTask opcodes — should be 2 (one per branch)
        let spawn_count = opcodes
            .iter()
            .filter(|&&op| op == OpCode::SpawnTask)
            .count();
        assert_eq!(
            spawn_count, 2,
            "Should have 2 SpawnTask opcodes (one per branch)"
        );
    }

    #[test]
    fn test_join_init_operand_encoding() {
        // Verify the packed operand encoding for JoinInit
        let code = r#"
            async function test_join() {
                await join race {
                    10,
                    20,
                    30,
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(result.is_ok(), "Should compile: {:?}", result.err());

        let bytecode = result.unwrap();
        // Find JoinInit instruction and verify operand
        let join_init = bytecode
            .instructions
            .iter()
            .find(|i| i.opcode == OpCode::JoinInit)
            .expect("Should have JoinInit instruction");

        match &join_init.operand {
            Some(Operand::Count(packed)) => {
                let kind = (packed >> 14) & 0x03;
                let arity = packed & 0x3FFF;
                assert_eq!(kind, 1, "Kind should be 1 (Race)");
                assert_eq!(arity, 3, "Arity should be 3");
            }
            other => panic!("Expected Count operand, got: {:?}", other),
        }
    }

    #[test]
    fn test_annotated_expression_compiles() {
        // @annotation expr should compile (annotation is metadata, target is compiled)
        // Use simple expression to avoid undefined function errors
        let code = r#"
            annotation timeout(duration) {}
            async function with_anno() {
                await @timeout(5s) 42
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "Annotated await should compile: {:?}",
            result.err()
        );
    }

    // ===== Sprint 7: Structured Concurrency + Async Trait Methods =====

    #[test]
    fn test_async_let_compiles_in_async_function() {
        let code = r#"
            async function fetch_data() {
                async let x = 1 + 2
                await x
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "async let in async function should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        // Should have SpawnTask (for async let) and StoreLocal (for binding)
        assert!(
            opcodes.contains(&OpCode::SpawnTask),
            "async let should emit SpawnTask opcode"
        );
    }

    #[test]
    fn test_async_let_outside_async_is_error() {
        let code = r#"
            function sync_func() {
                async let x = 1 + 2
                x
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "async let outside async should be an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("async"),
            "Error should mention async, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_async_let_binding_is_immutable() {
        let code = r#"
            async function fetch_data() {
                async let x = 1 + 2
                x = 3
                await x
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(result.is_err(), "async let reassignment should fail");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("immutable") && err_msg.contains("'x'"),
            "unexpected error: {}",
            err_msg
        );
    }

    #[test]
    fn test_async_scope_compiles_in_async_function() {
        let code = r#"
            async function process() {
                async scope {
                    42
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "async scope in async function should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        assert!(
            opcodes.contains(&OpCode::AsyncScopeEnter),
            "async scope should emit AsyncScopeEnter opcode"
        );
        assert!(
            opcodes.contains(&OpCode::AsyncScopeExit),
            "async scope should emit AsyncScopeExit opcode"
        );
    }

    #[test]
    fn test_async_scope_outside_async_is_error() {
        let code = r#"
            function sync_func() {
                async scope {
                    42
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "async scope outside async should be an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("async"),
            "Error should mention async, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_async_scope_with_async_let_inside() {
        // Use a single async let inside an async scope to verify they interact correctly
        let code = r#"
            async function structured() {
                async scope {
                    async let a = 10
                    await a
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "async scope with async let should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        // Should have AsyncScopeEnter, SpawnTask, Await, AsyncScopeExit
        assert!(
            opcodes.contains(&OpCode::AsyncScopeEnter),
            "Should contain AsyncScopeEnter"
        );
        assert!(
            opcodes.contains(&OpCode::AsyncScopeExit),
            "Should contain AsyncScopeExit"
        );
        assert!(
            opcodes.contains(&OpCode::SpawnTask),
            "Should contain SpawnTask (from async let)"
        );
        assert!(
            opcodes.contains(&OpCode::Await),
            "Should contain Await (from await a)"
        );
    }

    #[test]
    fn test_for_await_compiles_in_async_function() {
        let code = r#"
            async function consume_stream() {
                let items = [1, 2, 3]
                for await item in items {
                    item
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "for await in async function should compile: {:?}",
            result.err()
        );

        let bytecode = result.unwrap();
        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        // Should have Await opcode in the loop
        assert!(
            opcodes.contains(&OpCode::Await),
            "for await should emit Await opcode"
        );
    }

    #[test]
    fn test_for_await_outside_async_is_error() {
        let code = r#"
            function sync_func() {
                let items = [1, 2, 3]
                for await item in items {
                    item
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "for await outside async should be an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("async"),
            "Error should mention async, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_async_scope_opcode_ordering() {
        // Verify the opcode sequence: AsyncScopeEnter → body → AsyncScopeExit
        let code = r#"
            async function test() {
                async scope {
                    99
                }
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let bytecode = BytecodeCompiler::new()
            .compile(&program)
            .expect("Failed to compile");

        let opcodes: Vec<_> = bytecode.instructions.iter().map(|i| i.opcode).collect();

        let enter_pos = opcodes
            .iter()
            .position(|&op| op == OpCode::AsyncScopeEnter)
            .expect("Should have AsyncScopeEnter");
        let exit_pos = opcodes
            .iter()
            .position(|&op| op == OpCode::AsyncScopeExit)
            .expect("Should have AsyncScopeExit");

        assert!(
            enter_pos < exit_pos,
            "AsyncScopeEnter (pos {}) should come before AsyncScopeExit (pos {})",
            enter_pos,
            exit_pos
        );
    }

    // ── WS-3 F2a: `?` operator unwrapped-type stamp ──────────────────────

    #[test]
    fn ws3_f2a_try_operator_unwrapped_type_propagates_to_binding() {
        // The `?` operator unwraps `Result<int, string>` to `int`; a
        // later `v + 1` must type-check (`int + int`). Before the F2a
        // fix, `compile_expr_try_operator` did not stamp the tracker, so
        // `v` was recorded with no type and `v + 1` failed strict typing
        // as `unknown + int`.
        let code = r#"
            fn p(s: string) -> Result<int, string> { Ok(42) }
            fn main() -> Result<int, string> {
                let v = p("x")?
                let w = v + 1
                print(w)
                Ok(0)
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "`?`-unwrapped value must keep its type for a downstream binop: {:?}",
            result.err()
        );
    }

    #[test]
    fn ws3_f2a_try_operator_option_unwrapped_type_propagates() {
        // Same as above but for `Option<int>` — `?` unwraps to `int`.
        let code = r#"
            fn p() -> Option<int> { Some(7) }
            fn main() -> Option<int> {
                let v = p()?
                let w = v + 1
                print(w)
                Some(0)
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "`?`-unwrapped Option value must keep its type: {:?}",
            result.err()
        );
    }
}
