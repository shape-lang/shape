//! Binary operation expression compilation

use crate::bytecode::{Instruction, NumericWidth, OpCode, Operand};
use crate::type_tracking::{NumericType, VariableTypeInfo};
use shape_ast::ast::operators::{FuzzyOp, FuzzyTolerance};
use shape_ast::ast::{BinaryOp, Expr, Literal, Span, Spanned, UnaryOp};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_schema::{FieldType, SchemaId};

use super::super::BytecodeCompiler;
use super::numeric_ops::{
    CoercionPlan, apply_coercion, inferred_type_to_numeric, is_function_type,
    is_ordered_comparison, is_strict_arithmetic, is_strict_bitwise, is_type_numeric, plan_coercion,
    type_display_name, typed_opcode_for,
};

/// Map a BinaryOp to its operator trait name, if one exists.
///
/// R5.2B: `Add` is included so `try_emit_trait_dispatch` covers the Add
/// branch's `CoercedNeedsGeneric | NoPlan` fallback uniformly with the
/// strict-arithmetic path. The other three strict-arithmetic callers
/// (L1092, L1192, L1229) are gated by `is_strict_arithmetic(op)` which
/// excludes Add, so they remain unaffected.
fn operator_trait_for_op(op: &BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::Add => Some("Add"),
        BinaryOp::Sub => Some("Sub"),
        BinaryOp::Mul => Some("Mul"),
        BinaryOp::Div => Some("Div"),
        BinaryOp::Mod => Some("Mod"),
        BinaryOp::BitAnd => Some("BitAnd"),
        BinaryOp::BitOr => Some("BitOr"),
        BinaryOp::BitXor => Some("BitXor"),
        BinaryOp::BitShl => Some("Shl"),
        BinaryOp::BitShr => Some("Shr"),
        BinaryOp::Greater | BinaryOp::Less | BinaryOp::GreaterEq | BinaryOp::LessEq => Some("Ord"),
        // W1.7: Eq/Neq dispatch for user-defined types. Built-in
        // scalar types take typed `EqInt`/`EqString`/... before this
        // mapping is consulted (`compile_typed_equality` resolves
        // operand types via `resolve_eq_type` and emits typed opcodes
        // first; only when both operands lack a recognised primitive
        // shape does the user-type Eq dispatch fire).
        BinaryOp::Equal | BinaryOp::NotEqual => Some("Eq"),
        _ => None, // Pow has no operator trait
    }
}

/// Map a binary op to the user-facing trait method name (lowercase).
/// Used by Phase 2.5 to emit `CallMethod("add"/"sub"/...)` for operator
/// overloading on user-defined types. The runtime dispatches via
/// `function_name_index["{Type}::{method}"]` (see `op_call_method` →
/// `handle_typed_object_method`).
fn operator_trait_method_for_op(op: &BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::Add => Some("add"),
        BinaryOp::Sub => Some("sub"),
        BinaryOp::Mul => Some("mul"),
        BinaryOp::Div => Some("div"),
        BinaryOp::Mod => Some("mod"),
        BinaryOp::BitAnd => Some("bitand"),
        BinaryOp::BitOr => Some("bitor"),
        BinaryOp::BitXor => Some("bitxor"),
        BinaryOp::BitShl => Some("shl"),
        BinaryOp::BitShr => Some("shr"),
        BinaryOp::Greater | BinaryOp::Less | BinaryOp::GreaterEq | BinaryOp::LessEq => Some("cmp"),
        // W1.7: `Eq::eq(self, other) -> bool`. Both `==` and `!=` map
        // to the same method; the negation for `!=` is emitted by the
        // caller (`compile_typed_equality`) after the dispatch.
        BinaryOp::Equal | BinaryOp::NotEqual => Some("eq"),
        _ => None,
    }
}

fn emit_cmp_result_comparison(compiler: &mut BytecodeCompiler, op: &BinaryOp) {
    use crate::bytecode::Constant;
    let zero_idx = compiler.program.add_constant(Constant::Int(0));
    compiler.emit(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(zero_idx)),
    ));
    let cmp_op = match op {
        BinaryOp::Greater => OpCode::GtInt,
        BinaryOp::Less => OpCode::LtInt,
        BinaryOp::GreaterEq => OpCode::GteInt,
        BinaryOp::LessEq => OpCode::LteInt,
        _ => unreachable!(),
    };
    compiler.emit(Instruction::simple(cmp_op));
}

/// v0.3.3 c6-binop-ref-operand-segfault helper. Return the span of a
/// direct `Expr::Reference` operand if either side of a binary operation
/// is a reference expression. Used by `compile_expr_binary_op` to refuse
/// `&x + y` / `f(&a) + &a` / `&x == y` and friends at semantic-check time
/// — the immediate operand is the only shape that reproduces the JIT
/// `jit_call_method::read_heap_kind` misaligned-pointer SEGFAULT (a
/// `Reference` nested inside, e.g., `(1 + &a)` is caught by the inner
/// binop's own check on recursive descent). Returns the reference's
/// span (preferred error-marker position) so the error diagnostic
/// underlines the borrow rather than the operator.
fn reference_operand_span(left: &Expr, right: &Expr) -> Option<Span> {
    if let Expr::Reference { span, .. } = left {
        return Some(*span);
    }
    if let Expr::Reference { span, .. } = right {
        return Some(*span);
    }
    None
}

fn try_emit_trait_dispatch(
    compiler: &mut BytecodeCompiler,
    op: &BinaryOp,
    left_schema: Option<SchemaId>,
    left_expr: &Expr,
    op_span: Span,
) -> bool {
    let trait_name = match operator_trait_for_op(op) {
        Some(t) => t,
        None => return false,
    };
    let method_name = match operator_trait_method_for_op(op) {
        Some(m) => m,
        None => return false,
    };
    let has_trait_via_schema = left_schema
        .and_then(|sid| compiler.type_tracker.schema_registry().get_by_id(sid))
        .is_some_and(|schema| {
            compiler
                .type_inference
                .env
                .type_implements_trait(&schema.name, trait_name)
        });
    let has_trait = has_trait_via_schema
        || compiler.infer_expr_type(left_expr).ok().is_some_and(|ty| {
            let name = type_display_name(&ty);
            compiler
                .type_inference
                .env
                .type_implements_trait(&name, trait_name)
        });
    if !has_trait {
        return false;
    }
    emit_operator_trait_call(compiler, method_name, op_span);
    if is_ordered_comparison(op) {
        emit_cmp_result_comparison(compiler, op);
    } else if let Some(left_id) = left_schema {
        // Arithmetic operator trait result is `Self` — restore the left schema
        // so chained / assigned uses resolve. (operators slice —
        // compound-assign fix)
        compiler.restore_operator_trait_result_schema(left_id);
    }
    true
}

/// Emit a `CallMethod` instruction targeting an operator trait method
/// (e.g. `Vec2::add`). Both operands must already be on the stack: receiver
/// first, then the right-hand-side argument.
///
/// `op_span` is the source span of the parent `Expr::BinaryOp` /
/// `Expr::UnaryOp` node. W10 jit-call-method-user-trait-fix (2026-05-17):
/// records the dispatch in `BytecodeProgram.operator_trait_dispatch_sites`
/// so the JIT MIR consumer at `crates/shape-jit/src/mir_compiler/rvalues.
/// rs::compile_rvalue` can re-emit the same dispatch at the matching
/// `Rvalue::BinaryOp` / `Rvalue::UnaryOp` site (keyed by the same span
/// the MIR lowering at `crates/shape-vm/src/mir/lowering/expr.rs::
/// lower_expr_to_temp` stamps on the statement via `expr.span()`).
fn emit_operator_trait_call(
    compiler: &mut BytecodeCompiler,
    method_name: &'static str,
    op_span: Span,
) {
    let method_id = shape_value::MethodId::from_name(method_name);
    let string_id = compiler.program.add_string(method_name.to_string());
    compiler.emit(Instruction::new(
        OpCode::CallMethod,
        Some(Operand::TypedMethodCall {
            method_id: method_id.0,
            arg_count: 1,
            string_id,
            receiver_type_tag: 0xFF,
        }),
    ));
    // ADR-006 §2.7.5 W10 conduit: persist the bytecode-time trait-dispatch
    // decision so the JIT MIR consumer can lift `Rvalue::BinaryOp` at the
    // same source span to a method-call equivalent. arg_count = 1 (binary
    // ops dispatch a single explicit RHS argument; receiver is implicit).
    compiler
        .program
        .operator_trait_dispatch_sites
        .insert(op_span, (method_name.to_string(), 1));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

fn combined_span(left: &Expr, right: &Expr) -> Span {
    let ls = left.span();
    let rs = right.span();
    Span::new(ls.start.min(rs.start), ls.end.max(rs.end))
}

/// Strict-typing sweep (Phase 1): produce a `ShapeError::SemanticError`
/// for a binary operation whose operand types could not be proven at
/// compile time. Replaces the former `*Dynamic`-emission shim
/// (`emit_generic_via_helper` and direct `emit_binary_op(... Unknown,
/// Unknown)` calls).
///
/// The error includes the operator symbol, both operand types (as
/// inferred — falling back to `"unknown"` when inference declines), and a
/// span covering both operands so editors can underline the offending
/// expression.
fn strict_typing_binop_error(
    compiler: &mut BytecodeCompiler,
    op: &BinaryOp,
    left: &Expr,
    right: &Expr,
) -> ShapeError {
    let lhs_type = compiler
        .infer_expr_type(left)
        .map(|t| type_display_name(&t))
        .unwrap_or_else(|_| "unknown".to_string());
    let rhs_type = compiler
        .infer_expr_type(right)
        .map(|t| type_display_name(&t))
        .unwrap_or_else(|_| "unknown".to_string());
    ShapeError::SemanticError {
        message: format!(
            "Cannot infer types for binary operation `{:?}`: operand types are `{}` and `{}`. \
             Strict typing requires both operands to have a known concrete type at compile time. \
             Add a type annotation to disambiguate.",
            op, lhs_type, rhs_type
        ),
        location: Some(compiler.span_to_source_location(combined_span(left, right))),
    }
}

/// Strict no-coercion ruling (user 2026-06-14): `string + non-string` is a
/// compile error. Under strict typing there is no implicit auto-stringify of
/// the non-string operand — the `op_string_concat_int/number/bool`
/// auto-stringify handlers are no longer reachable from `+` for well-typed
/// code. The fix is to use f-string interpolation (`f"{a}{b}"`) or an explicit
/// string conversion.
///
/// Fires when exactly one operand of `+` is a string (or char) and the other
/// is a non-string concrete type. Both string operands take the
/// `StringConcatTyped` path before this; both non-string operands never reach
/// it.
fn string_plus_nonstring_error(
    compiler: &mut BytecodeCompiler,
    left: &Expr,
    right: &Expr,
    lhs_type: &str,
    rhs_type: &str,
) -> ShapeError {
    ShapeError::SemanticError {
        message: format!(
            "Cannot apply `+` to a `string` and a `{}`. Strict typing does not \
             implicitly convert `{}` to a string for concatenation. Use f-string \
             interpolation, e.g. `f\"{{...}}\"`, or convert the value to a string \
             explicitly before concatenating.",
            if lhs_type == "string" || lhs_type == "char" {
                rhs_type
            } else {
                lhs_type
            },
            if lhs_type == "string" || lhs_type == "char" {
                rhs_type
            } else {
                lhs_type
            },
        ),
        location: Some(compiler.span_to_source_location(combined_span(left, right))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericEmitResult {
    EmittedTyped,
    CoercedNeedsGeneric,
    NoPlan,
}

/// Simplified type category for equality dispatch.
/// Collapses int-width variants to `Int` and char to `String` (EqString
/// handles both heap-boxed string and char values via `as_str()`).
///
/// WS-8 (2026-05-22): `Bool` added — `bool == bool` lowers to `EqInt`
/// (bools are 0/1 bits, so bitwise comparison is correct). Pre-WS-8 the
/// missing bool arm fell through to `Eq`-trait dispatch which surfaced
/// `no method 'eq' on receiver kind Bool` for both direct `a == b` and
/// for `vec.shape`'s generic `.includes`/`.indexOf` that compare bool
/// elements element-by-element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EqOperandType {
    Int,
    Number,
    Decimal,
    String,
    Bool,
}

impl BytecodeCompiler {
    /// After an operator-trait dispatch (`a + b` → `a.add(b)` etc.), restore
    /// the expr-type tracking to the RESULT type so a chained or assigned use
    /// sees a concrete type. For the arithmetic operator traits (`Add`/`Sub`/
    /// `Mul`/`Div`/`BitAnd`/`BitOr`/`BitXor`) the result type is `Self` — the
    /// LEFT operand's type — so `result_schema_id` is the left operand's schema
    /// id. `emit_operator_trait_call` clears `last_expr_schema` /
    /// `last_expr_type_info`; without this restore, `acc = acc + x` (and the
    /// `acc += x` it desugars from) dropped `acc`'s schema, so the NEXT
    /// `acc + y` could not resolve the operator trait and failed with
    /// "operand types are unknown". (operators slice — compound-assign fix)
    fn restore_operator_trait_result_schema(&mut self, result_schema_id: SchemaId) {
        if let Some(schema) = self
            .type_tracker
            .schema_registry()
            .get_by_id(result_schema_id)
        {
            let name = schema.name.clone();
            self.last_expr_schema = Some(result_schema_id);
            self.last_expr_type_info =
                Some(crate::type_tracking::VariableTypeInfo::known(result_schema_id, name));
        }
    }

    /// ε-1 PART 1 — emit-side soundness guard.
    ///
    /// Returns a `ProofGap`-derived compile error when a typed numeric opcode
    /// is about to be emitted for `operand` (its `numeric` hint is `Some`, so
    /// a typed opcode WOULD fire) but the operand's actual compile-time type
    /// is still an unresolved `Type::Variable`/`Type::Constrained`.
    ///
    /// That combination means the `NumericType` claim is *fabricated* — no
    /// signal proved the kind, so emitting `MulInt`/`MulNumber`/... would
    /// stamp a default kind on a value of unknown type (the exact silent-wrong
    /// path that reinterpreted the integer `40` as the denormal `2e-321`).
    ///
    /// Restricted to identifiers bound to an *untyped function parameter slot*
    /// with no tracker type info: those are the only operands whose numeric
    /// hint can be set without a proving signal (literals, typed locals and
    /// for-loop variables all carry a real proven kind). This keeps the guard
    /// from false-positiving on ordinary well-typed code.
    fn numeric_operand_proof_gap(
        &mut self,
        op: &BinaryOp,
        operand: &Expr,
        numeric: Option<NumericType>,
    ) -> Option<ShapeError> {
        // No typed opcode would fire for this operand → nothing to prove.
        numeric?;

        let Expr::Identifier(name, _) = operand else {
            return None;
        };
        let local_idx = self.resolve_local(name)?;
        // Only untyped function parameters can carry an unproven numeric hint.
        if !self.param_locals.contains(&local_idx) {
            return None;
        }
        // A param with concrete tracker type info has a proven kind.
        if let Some(info) = self.type_tracker.get_local_type(local_idx) {
            if info.type_name.is_some() || info.storage_hint.is_some() {
                return None;
            }
        }

        // The decisive check: ask the inference engine for the operand's
        // resolved type. A bare unresolved variable (or still-bounded
        // constrained variable) is an unprovable kind.
        let inferred = self.infer_expr_type(operand).ok()?;
        if !matches!(
            inferred,
            shape_runtime::type_system::Type::Variable(_)
                | shape_runtime::type_system::Type::Constrained { .. }
        ) {
            return None;
        }

        let gap = crate::type_tracking::proof_gap_unresolved_operand(
            "emit_typed_arithmetic",
            format!(
                "operand `{}` of `{:?}` has an unresolved type — no signal \
                 proves its NativeKind, so a typed numeric opcode cannot be \
                 emitted. Add a type annotation to the parameter.",
                name, op
            ),
        );
        Some(ShapeError::SemanticError {
            message: gap.to_string(),
            location: Some(self.span_to_source_location(operand.span())),
        })
    }

    /// A-final ROOT-C: deferred-template numeric-binop placeholder.
    ///
    /// Returns `true` (and emits a stack-balancing `Pop`) when this binop is
    /// being compiled inside the body of an *uninstantiated implicit-generic*
    /// function (`fn add(a, b) { a + b }`, never called, params stay
    /// unresolved type variables — see `is_uninstantiated_implicit_generic`).
    /// Such a body is a deferred template whose bytecode is DEAD (re-emitted
    /// with proven kinds per concrete call site), so the polymorphic-numeric
    /// proof-gap (no proven `NativeKind` on the operands) must NOT abort
    /// compilation with a typed-opcode / strict-typing error.
    ///
    /// Both operand values are already on the stack at every binop terminal
    /// that calls this (compiled before the numeric-emit decision), so a single
    /// `Pop` (2 → 1) keeps the dead blob stack-balanced. NO fabricated typed
    /// numeric opcode, no default kind, no int-VALUE->number widening is
    /// emitted — and the blob never runs. STRUCTURAL/schema body checks
    /// (object-spread-without-known-schema, etc.) are unaffected: they
    /// `return Err` from their own emit paths and never reach this numeric-only
    /// deferral. This narrows the prior whole-body skip so a genuine structural
    /// error is no longer suppressed alongside the benign numeric proof-gap.
    fn defer_template_numeric_binop(&mut self) -> bool {
        if !self.deferring_uninstantiated_template_body {
            return false;
        }
        self.emit(Instruction::new(OpCode::Pop, None));
        self.last_expr_numeric_type = None;
        self.last_expr_schema = None;
        true
    }

    fn infer_numeric_pair(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> (Option<NumericType>, Option<NumericType>) {
        let inferred_left = self
            .infer_expr_type(left)
            .ok()
            .and_then(|t| inferred_type_to_numeric(&t));
        let inferred_right = self
            .infer_expr_type(right)
            .ok()
            .and_then(|t| inferred_type_to_numeric(&t));
        (inferred_left, inferred_right)
    }

    fn adopt_missing_numeric_operand_hint(
        &mut self,
        left: &Expr,
        right: &Expr,
        left_numeric: &mut Option<NumericType>,
        right_numeric: &mut Option<NumericType>,
    ) {
        if let (Some(known), None) = (*left_numeric, *right_numeric)
            && matches!(right, Expr::Identifier(..) | Expr::IndexAccess { .. })
            && self.last_expr_schema.is_none()
            // Strict no-coercion ruling (user 2026-06-14): never seed a numeric
            // hint onto an operand the compiler has proven to be a `string`.
            // Doing so misclassifies `s + n` (string + number) as a numeric add
            // and defers the failure to a runtime `TypeError` instead of the
            // clean compile-time `string + non-string` rejection below.
            && !self.expr_is_proven_string(right)
        {
            // Adopt Int only if the identifier has a confirmed Int type.
            // Otherwise promote to Number to avoid misclassifying floats as ints.
            let safe = self.safe_adopt_numeric_hint(right, known);
            // Only adopt if the type didn't change (confirmed match).
            // If safe != known, skip adoption — let the operation fall through
            // to inference or generic opcodes that handle mixed types at runtime.
            if safe == known {
                *right_numeric = Some(safe);
                self.seed_numeric_hint_from_expr(right, safe);
            }
            return;
        }

        if let (None, Some(known)) = (*left_numeric, *right_numeric)
            && matches!(left, Expr::Identifier(..) | Expr::IndexAccess { .. })
        {
            // Do not adopt a numeric hint for identifiers that are typed objects.
            let has_object_schema = if let Expr::Identifier(name, _) = left {
                self.resolve_local(name)
                    .and_then(|idx| self.type_tracker.get_local_type(idx))
                    .and_then(|info| info.schema_id)
                    .is_some()
            } else {
                false
            };
            // Strict no-coercion ruling (user 2026-06-14): same guard as the
            // symmetric branch — a proven-`string` LHS must not adopt the RHS
            // numeric hint, or `s + n` becomes a numeric add that fails at
            // runtime instead of rejecting at compile time.
            if !has_object_schema && !self.expr_is_proven_string(left) {
                let safe = self.safe_adopt_numeric_hint(left, known);
                // Only adopt if the type didn't change (confirmed match).
                if safe == known {
                    *left_numeric = Some(safe);
                    self.seed_numeric_hint_from_expr(left, safe);
                }
            }
        }
    }

    /// Strict no-coercion ruling (user 2026-06-14): true when `expr` is proven
    /// to be a `string` (or `char`) at compile time. Used to keep numeric-hint
    /// adoption from misclassifying a string operand of `+` as numeric, so the
    /// `string + non-string` rejection in the `Add` arm fires at compile time
    /// rather than deferring to a runtime `TypeError`.
    fn expr_is_proven_string(&mut self, expr: &Expr) -> bool {
        if matches!(expr, Expr::Literal(Literal::String(_), _)) {
            return true;
        }
        if matches!(
            self.storage_hint_for_expr(expr),
            Some(crate::type_tracking::NativeKind::String)
        ) {
            return true;
        }
        matches!(
            self.infer_expr_type(expr)
                .ok()
                .map(|t| type_display_name(&t))
                .as_deref(),
            Some("string") | Some("char")
        )
    }

    /// When adopting a numeric hint from one operand to another, check if adopting
    /// Int is safe. If the target identifier has no confirmed Int type from the
    /// type tracker, promote to Number to avoid emitting Int-typed opcodes for
    /// values that may actually be floats at runtime.
    fn safe_adopt_numeric_hint(&self, expr: &Expr, hint: NumericType) -> NumericType {
        if hint != NumericType::Int {
            return hint;
        }
        // Check if the identifier has a confirmed numeric type
        if let Expr::Identifier(name, _) = expr {
            if let Some(local_idx) = self.resolve_local(name) {
                if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                    // Post-§2.7.5.1: `info.storage_hint` is
                    // `Option<StorageHint>`; `Some(Int64)` is the proven-Int
                    // case, anything else (including `None` for
                    // not-yet-proven) falls through to the safe Number path.
                    if info.storage_hint == Some(crate::type_tracking::StorageHint::Int64) {
                        return NumericType::Int;
                    }
                }
            }
        }
        // For unconfirmed types, use Number (safe for both int and float values)
        NumericType::Number
    }

    /// Numeric-conversion §4 literal adoption support: whether `expr` is PROVEN
    /// to carry the `number`/`f64` floating-point family at compile time. Used to
    /// gate the binary-operand int-literal → `number` widening: a bare int
    /// literal whose partner proves float adopts the float family.
    ///
    /// A bare `Int`/`UInt` literal is explicitly NOT counted as proving float
    /// (it has no committed family until a context pins it), so `5 / 2` with no
    /// surrounding number context stays integer division — only a genuine float
    /// operand (a `number`-typed binding, a float literal, a `number`-returning
    /// call, etc.) drives the sibling literal to adopt `number`.
    pub(crate) fn expr_proves_float(&mut self, expr: &Expr) -> bool {
        if matches!(
            expr,
            Expr::Literal(shape_ast::ast::Literal::Int(_), _)
                | Expr::Literal(shape_ast::ast::Literal::UInt(_), _)
        ) {
            return false;
        }
        // A float literal proves float directly.
        if matches!(expr, Expr::Literal(shape_ast::ast::Literal::Number(_), _)) {
            return true;
        }
        matches!(
            self.infer_expr_type(expr)
                .ok()
                .and_then(|t| super::numeric_ops::inferred_type_to_numeric(&t)),
            Some(NumericType::Number)
        )
    }

    /// Returns `true` when the expression is syntactically guaranteed to be numeric.
    /// This does NOT consult the type tracker — it only looks at the AST node itself.
    fn is_expr_confirmed_numeric(expr: &Expr) -> bool {
        match expr {
            Expr::Literal(Literal::Int(_), _)
            | Expr::Literal(Literal::Number(_), _)
            | Expr::Literal(Literal::TypedInt(..), _)
            | Expr::Literal(Literal::UInt(_), _)
            | Expr::Literal(Literal::Decimal(_), _) => true,
            Expr::UnaryOp {
                op: UnaryOp::Neg,
                operand,
                ..
            } => Self::is_expr_confirmed_numeric(operand),
            _ => false,
        }
    }

    /// If `expr` is a bare integer literal (`Literal::Int` / `Literal::UInt`),
    /// return its value (`Literal::UInt` always non-negative). `None` for any
    /// non-literal expression.
    ///
    /// `Literal::TypedInt` is excluded: an explicitly-suffixed literal
    /// (`5i32`, `7u8`) has a declared width that the programmer chose; it
    /// does not adapt.
    fn bare_int_literal_value(expr: &Expr) -> Option<i128> {
        match expr {
            Expr::Literal(Literal::Int(v), _) => Some(*v as i128),
            Expr::Literal(Literal::UInt(v), _) => Some(*v as i128),
            _ => None,
        }
    }

    /// Returns `true` when a bare integer literal of value `v` can soundly
    /// adopt the integer width `w` of a sibling operand. A negative literal
    /// cannot adopt an unsigned width (it has no representation there);
    /// every other case is allowed — sub-range overflow truncates per the
    /// two's-complement wrapping semantics that `let x: i8 = 1000` already
    /// applies (2026-05-20 integer-semantics ruling #3).
    fn int_literal_fits_width(v: i128, w: shape_ast::IntWidth) -> bool {
        !(v < 0 && !w.is_signed())
    }

    /// ADR-006 §2.7.5 stamp-at-compile-time — int-literal width inference.
    ///
    /// A bare integer literal is width-polymorphic: as an operand of a
    /// width-typed binary op it must be inferred and kind-stamped with the
    /// sibling's width, exactly as it would be when bound to a width
    /// annotation (`let x: u64 = 2`, `let y: i8 = 28`). When one operand
    /// carries `NumericType::IntWidth(W)` and the other is a bare integer
    /// literal currently classified as the default `NumericType::Int`,
    /// promote the literal to `IntWidth(W)` so `plan_coercion` keeps the
    /// operation on `W`'s carrier (`AddTyped`/`DivTyped`/... with the
    /// matching `NumericWidth`) instead of widening to the signed default
    /// `i64` `NumericType::Int`.
    ///
    /// Without this, `plan_coercion(IntWidth(W), Int)` returns
    /// `NoCoercion(Int)`: `a / 2` on `a: u64` emits the signed `DivInt`
    /// (`u64::MAX / 2` computes `(-1) / 2 == 0`), and `x + 28` on `x: i8`
    /// emits `AddInt` (`100 + 28 == 128` instead of the wrapped `-128`).
    ///
    /// Only the literal side is promoted; a genuinely width-typed sibling
    /// (`let b: int = 3; a / b`) is left untouched (its `Int` hint stands).
    fn promote_int_literal_to_width_sibling(
        left: &Expr,
        right: &Expr,
        left_numeric: &mut Option<NumericType>,
        right_numeric: &mut Option<NumericType>,
    ) {
        if let (Some(NumericType::IntWidth(w)), Some(NumericType::Int)) =
            (*left_numeric, *right_numeric)
        {
            if let Some(v) = Self::bare_int_literal_value(right) {
                if Self::int_literal_fits_width(v, w) {
                    *right_numeric = Some(NumericType::IntWidth(w));
                }
            }
        } else if let (Some(NumericType::Int), Some(NumericType::IntWidth(w))) =
            (*left_numeric, *right_numeric)
        {
            if let Some(v) = Self::bare_int_literal_value(left) {
                if Self::int_literal_fits_width(v, w) {
                    *left_numeric = Some(NumericType::IntWidth(w));
                }
            }
        }
    }

    /// Get the compile-time StorageHint for an expression, if it can be determined.
    ///
    /// Only returns a hint for identifiers that are immutable (`let` bindings),
    /// since mutable variables (`var`) can be modified through reference parameters
    /// (DerefStore) and their runtime type may diverge from the tracker's static view.
    fn storage_hint_for_expr(&self, expr: &Expr) -> Option<crate::type_tracking::StorageHint> {
        match expr {
            Expr::Identifier(name, _) => {
                let local_idx = self.resolve_local(name)?;
                // Don't trust storage hints for function parameters with no explicit
                // type annotation — their inferred types (from inferred_param_type_hints)
                // can be wrong (e.g., a string param inferred as numeric → B19).
                if self.param_locals.contains(&local_idx) {
                    return None;
                }
                let info = self.type_tracker.get_local_type(local_idx)?;
                // Per ADR-006 §2.7.5.1, `NativeKind::Unknown` was deleted —
                // the in-memory analysis state for "not yet known" is held
                // as `Option<StorageHint>` on `info.storage_hint` itself.
                // Returning that field flat propagates `None` (not yet
                // proven) through this getter's `Option` return type.
                info.storage_hint
            }
            Expr::Literal(Literal::Int(_), _) => Some(crate::type_tracking::StorageHint::Int64),
            Expr::Literal(Literal::Number(_), _) => {
                Some(crate::type_tracking::StorageHint::Float64)
            }
            _ => None,
        }
    }

    #[allow(dead_code)]
    fn emit_numeric_binary_with_coercion(
        &mut self,
        op: &BinaryOp,
        left_numeric: Option<NumericType>,
        right_numeric: Option<NumericType>,
        is_comparison: bool,
    ) -> NumericEmitResult {
        self.emit_numeric_binary_with_coercion_inner(
            op,
            left_numeric,
            right_numeric,
            is_comparison,
            None,
            None,
        )
    }

    fn emit_numeric_binary_with_coercion_trusted(
        &mut self,
        op: &BinaryOp,
        left_numeric: Option<NumericType>,
        right_numeric: Option<NumericType>,
        is_comparison: bool,
        left_expr: &Expr,
        right_expr: &Expr,
    ) -> NumericEmitResult {
        let lhs_hint = self.storage_hint_for_expr(left_expr);
        let rhs_hint = self.storage_hint_for_expr(right_expr);
        self.emit_numeric_binary_with_coercion_inner(
            op,
            left_numeric,
            right_numeric,
            is_comparison,
            lhs_hint,
            rhs_hint,
        )
    }

    fn emit_numeric_binary_with_coercion_inner(
        &mut self,
        op: &BinaryOp,
        left_numeric: Option<NumericType>,
        right_numeric: Option<NumericType>,
        is_comparison: bool,
        _lhs_hint: Option<crate::type_tracking::StorageHint>,
        _rhs_hint: Option<crate::type_tracking::StorageHint>,
    ) -> NumericEmitResult {
        let Some(plan) = plan_coercion(left_numeric, right_numeric) else {
            return NumericEmitResult::NoPlan;
        };

        // u64 + signed is a compile error — must use explicit `as` cast
        if let CoercionPlan::IncompatibleWidths(a, b) = plan {
            self.errors
                .push(shape_ast::error::ShapeError::SemanticError {
                    message: format!(
                        "cannot mix `{}` and `{}` in arithmetic — use an explicit `as` cast",
                        a.type_name(),
                        b.type_name()
                    ),
                    location: None,
                });
            return NumericEmitResult::NoPlan;
        }

        let result_type = apply_coercion(self, plan);
        if let Some(opcode) = typed_opcode_for(op, result_type) {
            // Compact typed opcodes (AddTyped, etc.) need Width operand
            if let NumericType::IntWidth(w) = result_type {
                self.emit(Instruction::new(
                    opcode,
                    Some(Operand::Width(NumericWidth::from_int_width(w))),
                ));
            } else {
                self.emit(Instruction::simple(opcode));
            }
            // After a typed comparison, the result is a bool — record
            // that in `last_expr_type_info` so the implicit-return path
            // (`emit_return_value_with_ownership` →
            // `last_expr_numeric_type_to_storage_hint`) routes to
            // `ReturnValueBool`. Without this, the post-comparison
            // `last_expr_*` state is `None`/`None`, the implicit return
            // emits the legacy untyped `ReturnValue`, and `last_program_
            // return_kind` stays `None` — so the host-boundary synthesizer
            // falls back to passthrough on raw native 0u64/1u64 bits and
            // `as_bool()` returns `None`.
            if is_comparison {
                self.last_expr_type_info =
                    Some(crate::type_tracking::VariableTypeInfo::with_storage(
                        "bool".to_string(),
                        crate::type_tracking::StorageHint::Bool,
                    ));
            } else {
                self.last_expr_type_info = None;
            }
            self.last_expr_numeric_type = if is_comparison {
                None
            } else {
                Some(result_type)
            };
            NumericEmitResult::EmittedTyped
        } else {
            NumericEmitResult::CoercedNeedsGeneric
        }
    }

    /// Phase 2.6.5.3/4: inference-driven typed equality dispatch.
    ///
    /// Architectural shift: resolve operand types from multiple sources
    /// BEFORE compiling them, then pick the typed `Eq*`/`Neq*` opcode.
    ///
    /// Type resolution priority:
    /// 1. Type inference engine (`infer_expr_type`)
    /// 2. AST literal type (for `Literal::Int`, `Literal::String`, etc.)
    /// 3. Asymmetric propagation: if one side is typed and the other is not,
    ///    assume both sides have the same type. This is safe because typed
    ///    comparison opcodes return false for mismatched runtime types.
    ///
    /// Returns `Ok(true)` if a typed opcode was emitted, `Ok(false)` to
    /// fall through to the legacy slot-tracker dispatch.
    fn compile_typed_equality(
        &mut self,
        op: &BinaryOp,
        left: &Expr,
        right: &Expr,
        op_span: Span,
    ) -> Result<bool> {
        if !matches!(op, BinaryOp::Equal | BinaryOp::NotEqual) {
            return Ok(false);
        }

        let is_neq = matches!(op, BinaryOp::NotEqual);

        // Desugar `x == None` / `None == x` to IsNull(x).
        // This covers Option<T> comparisons and any None-literal equality.
        if matches!(right, Expr::Literal(Literal::None, _)) {
            self.compile_expr(left)?;
            self.emit(Instruction::simple(OpCode::IsNull));
            if is_neq {
                self.emit(Instruction::simple(OpCode::Not));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = None;
            return Ok(true);
        }
        if matches!(left, Expr::Literal(Literal::None, _)) {
            self.compile_expr(right)?;
            self.emit(Instruction::simple(OpCode::IsNull));
            if is_neq {
                self.emit(Instruction::simple(OpCode::Not));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.last_expr_numeric_type = None;
            return Ok(true);
        }

        // Resolve operand types from inference + literal fallback.
        let mut lhs_eq = self.resolve_eq_type(left);
        let mut rhs_eq = self.resolve_eq_type(right);

        // Asymmetric propagation: if one side is typed and the other is not,
        // propagate the known type. For `x == 5` where x is an untracked
        // loop counter, the literal 5 tells us to use EqInt.
        if lhs_eq.is_none() && rhs_eq.is_some() {
            lhs_eq = rhs_eq;
        } else if rhs_eq.is_none() && lhs_eq.is_some() {
            rhs_eq = lhs_eq;
        }

        // Pick the typed opcode and whether to negate after.
        // EqString/EqDecimal have no Neq variants → emit Eq + Not for NotEqual.
        // EqInt/EqNumber have NeqInt/NeqNumber variants → use them directly.
        let emission = match (lhs_eq, rhs_eq) {
            (Some(EqOperandType::Int), Some(EqOperandType::Int)) => Some(if is_neq {
                (OpCode::NeqInt, false)
            } else {
                (OpCode::EqInt, false)
            }),
            (Some(EqOperandType::Number), Some(EqOperandType::Number)) => Some(if is_neq {
                (OpCode::NeqNumber, false)
            } else {
                (OpCode::EqNumber, false)
            }),
            (Some(EqOperandType::Decimal), Some(EqOperandType::Decimal)) => {
                Some((OpCode::EqDecimal, is_neq))
            }
            (Some(EqOperandType::String), Some(EqOperandType::String)) => {
                Some((OpCode::EqString, is_neq))
            }
            // WS-8 (2026-05-22): bool == bool lowers to EqInt — bools carry
            // 0/1 bits on the §2.7.7 parallel-kind track and EqInt compares
            // raw 64-bit slot bits, so the bitwise comparison is correct for
            // bool values regardless of which slot bits represent {true,
            // false}. Routes through the existing typed integer opcode
            // without introducing a per-kind PHF entry. Result kind is
            // already Bool downstream.
            (Some(EqOperandType::Bool), Some(EqOperandType::Bool)) => Some(if is_neq {
                (OpCode::NeqInt, false)
            } else {
                (OpCode::EqInt, false)
            }),
            _ => None,
        };

        if let Some((opcode, needs_negate)) = emission {
            self.compile_expr(left)?;
            self.compile_expr(right)?;
            self.emit(Instruction::simple(opcode));
            if needs_negate {
                self.emit(Instruction::simple(OpCode::Not));
            }
            self.last_expr_schema = None;
            // Result is bool — record so the implicit-return path
            // emits `ReturnValueBool` and the host-boundary synthesizer
            // re-tags the raw native bool bits.
            self.last_expr_type_info = Some(crate::type_tracking::VariableTypeInfo::with_storage(
                "bool".to_string(),
                crate::type_tracking::StorageHint::Bool,
            ));
            self.last_expr_numeric_type = None;
            return Ok(true);
        }

        // Strict-typing-sweep: cross-numeric-kind equality where one side
        // is a literal int (e.g. `mean_val == 0` with `mean_val: number`).
        // Symmetric to plan_coercion's CoerceLeft / CoerceRight rules in
        // numeric_ops.rs, just for equality ops. This is the same
        // literal-int-into-number coercion arithmetic already does and
        // doesn't introduce a new fallback path: the resulting opcode is
        // a typed `EqNumber`/`NeqNumber`, not a Dynamic.
        let cross_emission = match (lhs_eq, rhs_eq) {
            (Some(EqOperandType::Number), Some(EqOperandType::Int))
                if matches!(right, Expr::Literal(Literal::Int(_), _)) =>
            {
                Some((OpCode::EqNumber, true /* coerce_right_int_to_number */))
            }
            (Some(EqOperandType::Int), Some(EqOperandType::Number))
                if matches!(left, Expr::Literal(Literal::Int(_), _)) =>
            {
                Some((OpCode::EqNumber, false /* coerce_left_int_to_number */))
            }
            _ => None,
        };
        if let Some((opcode, coerce_right)) = cross_emission {
            self.compile_expr(left)?;
            if !coerce_right {
                self.emit(Instruction::simple(OpCode::IntToNumber));
            }
            self.compile_expr(right)?;
            if coerce_right {
                self.emit(Instruction::simple(OpCode::IntToNumber));
            }
            // EqNumber → NeqNumber via Not when needed.
            let final_op = if is_neq { OpCode::NeqNumber } else { opcode };
            self.emit(Instruction::simple(final_op));
            self.last_expr_schema = None;
            // Result is bool — record so the implicit-return path
            // emits `ReturnValueBool` and the host-boundary synthesizer
            // re-tags the raw native bool bits.
            self.last_expr_type_info = Some(crate::type_tracking::VariableTypeInfo::with_storage(
                "bool".to_string(),
                crate::type_tracking::StorageHint::Bool,
            ));
            self.last_expr_numeric_type = None;
            return Ok(true);
        }

        // W1.7: user-defined `impl Eq for X` dispatch. Mirrors the
        // arithmetic-trait retargets at L1461-1475 / L1493 / L1512.
        // `compile_typed_equality` runs BEFORE either operand has been
        // compiled, so `last_expr_schema` reflects whatever the previous
        // expression left behind — not the left operand's schema. We
        // therefore consult three sources in order of decreasing
        // certainty: the slot tracker's `local_types` schema-id for an
        // identifier, the slot tracker's `binding_types` for a module
        // binding, and finally the inference engine (mirrors the second
        // half of `try_emit_trait_dispatch` at L88).
        //
        // For `!=` an extra `Not` opcode follows so user code only
        // authors `eq`. No separate `Neq` trait — Shape mirrors Rust's
        // single-method shape (`PartialEq::eq` + auto-derived `!=`).
        let trait_name = "Eq";
        let slot_type_name: Option<String> = if let Expr::Identifier(name, _) = left {
            if let Some(slot) = self.resolve_local(name) {
                self.type_tracker
                    .get_local_type(slot)
                    .and_then(|info| info.type_name.clone())
            } else if let Some(slot) = self.module_bindings.get(name).copied() {
                self.type_tracker
                    .get_binding_type(slot)
                    .and_then(|info| info.type_name.clone())
            } else {
                None
            }
        } else {
            None
        };
        let mut has_eq_impl = slot_type_name.as_ref().is_some_and(|name| {
            self.type_inference
                .env
                .type_implements_trait(name, trait_name)
        });
        if !has_eq_impl {
            has_eq_impl = self.infer_expr_type(left).ok().is_some_and(|ty| {
                let name = type_display_name(&ty);
                self.type_inference
                    .env
                    .type_implements_trait(&name, trait_name)
            });
        }
        if has_eq_impl {
            self.compile_expr(left)?;
            self.compile_expr(right)?;
            emit_operator_trait_call(self, "eq", op_span);
            if is_neq {
                self.emit(Instruction::simple(OpCode::Not));
            }
            // Eq::eq returns bool — match the typed-equality path's
            // type_info bookkeeping so the implicit-return path emits
            // `ReturnValueBool` and the host-boundary synthesizer
            // re-tags the raw native bool bits.
            self.last_expr_schema = None;
            self.last_expr_type_info = Some(crate::type_tracking::VariableTypeInfo::with_storage(
                "bool".to_string(),
                crate::type_tracking::StorageHint::Bool,
            ));
            self.last_expr_numeric_type = None;
            return Ok(true);
        }

        // Strict-typing sweep (Phase 1): the typed-equality dispatch above
        // declined for both operands, which historically routed through the
        // `emit_binary_op` shim with `BinOperandKind::Unknown` operands and
        // emitted `EqDynamic` / `NeqDynamic`. That dynamic-fallback path is
        // now a hard compile error.
        let typed_op = if is_neq {
            BinaryOp::NotEqual
        } else {
            BinaryOp::Equal
        };
        Err(strict_typing_binop_error(self, &typed_op, left, right))
    }

    /// Resolve the equality-relevant type of an expression from multiple
    /// sources: inference engine, then AST literal kind.
    fn resolve_eq_type(&mut self, expr: &Expr) -> Option<EqOperandType> {
        // Source 1: type inference engine
        if let Ok(ty) = self.infer_expr_type(expr) {
            if let Some(nt) = inferred_type_to_numeric(&ty) {
                return Some(match nt {
                    NumericType::Int | NumericType::IntWidth(_) => EqOperandType::Int,
                    NumericType::Number => EqOperandType::Number,
                    NumericType::Decimal => EqOperandType::Decimal,
                });
            }
            let name = type_display_name(&ty);
            match name.as_str() {
                "string" | "char" => return Some(EqOperandType::String),
                // WS-8 (2026-05-22): bool inferred type lights up the
                // typed-equality fast path (EqInt over bool bits).
                "bool" => return Some(EqOperandType::Bool),
                _ => {}
            }
        }

        // Source 2: AST literal type
        match expr {
            Expr::Literal(Literal::Int(_) | Literal::UInt(_) | Literal::TypedInt(..), _) => {
                Some(EqOperandType::Int)
            }
            Expr::Literal(Literal::Number(_), _) => Some(EqOperandType::Number),
            Expr::Literal(Literal::Decimal(_), _) => Some(EqOperandType::Decimal),
            Expr::Literal(Literal::String(_), _) => Some(EqOperandType::String),
            // WS-8 (2026-05-22): bool literal type for the AST-fallback
            // source (handles `x == true` where `x` is untyped).
            Expr::Literal(Literal::Bool(_), _) => Some(EqOperandType::Bool),
            _ => None,
        }
    }

    /// R4 c6-widen helper. Return the span of a reference-TYPED binop
    /// operand (one whose inferred type is `&T` / `&mut T`, expressible in
    /// type position after R1), if either side has such a type. Used by
    /// `compile_expr_binary_op` to refuse e.g. `make() + 1` where
    /// `fn make() -> &int`. The syntactic `&x` form is caught separately by
    /// `reference_operand_span`; this covers the typed form that has no
    /// immediate `&` token. Returns the operand's own span so the diagnostic
    /// underlines the offending operand.
    fn reference_typed_operand_span(&mut self, left: &Expr, right: &Expr) -> Option<Span> {
        for operand in [left, right] {
            if self.expr_has_reference_type(operand) {
                return Some(operand.span());
            }
        }
        None
    }

    /// True iff the inferred type of `expr` is a borrow (`&T` / `&mut T`).
    /// The borrow type flows as `Type::Concrete(TypeAnnotation::Borrow{..})`
    /// (R1) — no parallel `Type` enum variant.
    fn expr_has_reference_type(&mut self, expr: &Expr) -> bool {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;
        matches!(
            self.infer_expr_type(expr),
            Ok(Type::Concrete(TypeAnnotation::Borrow { .. }))
        )
    }

    /// True iff `ty` is a concrete `Option<T>` carrier type — i.e. the
    /// runtime value is an `Arc<OptionData>` (`Some(v)` / `None`), as
    /// opposed to a plain nullable `T?` (which is null-coded at runtime).
    /// Used by the `??` lowering to decide whether the JIT must deopt: the
    /// JIT MIR `lower_null_coalesce` `Eq None` path matches the VM for a
    /// nullable but leaks the `Some(v)` wrapper for an Option carrier.
    ///
    /// Note: `T?` desugars to `Option<T>` in the type lattice, so the two
    /// are not distinguishable purely from the unwrapped annotation. We
    /// treat any `Option<T>`-shaped inferred type as a carrier (the
    /// conservative, sound choice — deopt is always correctness-preserving;
    /// the common nullable-index `ctx[...] ?? d` infers to the element type
    /// or a bare nullable scalar, not an `Option<_>` generic, so it is NOT
    /// flagged and keeps its JIT path).
    fn type_is_option_carrier(ty: &shape_runtime::type_system::Type) -> bool {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;
        match ty {
            Type::Generic { base, args } if args.len() == 1 => matches!(
                base.as_ref(),
                Type::Concrete(ann) if ann.as_type_name_str() == Some("Option")
            ),
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Option" && args.len() == 1 =>
            {
                true
            }
            _ => false,
        }
    }

    /// True iff a `TypeAnnotation` is an `Option<T>` / `T?` carrier shape.
    /// `T?` desugars to `TypeAnnotation::Generic { name: "Option", .. }`
    /// (`TypeAnnotation::option`), so a single arm covers both surface forms.
    fn annotation_is_option_carrier(ann: &shape_ast::ast::TypeAnnotation) -> bool {
        ann.is_option()
    }

    /// A-2 `??` JIT-residual detection. Returns `true` when the STATIC type of
    /// a `??` left operand is an `Option<T>` carrier (`Arc<OptionData>` at
    /// runtime — `Some(v)` / `None`), so the JIT must whole-program deopt to
    /// the interpreter (the JIT MIR `lower_null_coalesce` `Eq None` path leaks
    /// the `Some(v)` wrapper that the VM `CoalesceProbe` unwraps).
    ///
    /// The prior detection consulted only `infer_expr_type(left)` →
    /// `type_is_option_carrier`, which catches an inline `Some(..)` constructor
    /// but MISSES a let-bound Option-typed local (`let x: int?`): `int?` is
    /// tracked as the lowercased wrapper name `"option"` (an
    /// `is_integer/number/...`-miss in `infer_expr_type`'s identifier branch),
    /// so the runtime inference engine — which never sees the function-body
    /// `let` — returns `Type::Variable` and the carrier is never recognised.
    ///
    /// This widens the gate to the additional static sources that genuinely
    /// prove an Option carrier WITHOUT weakening the plain-nullable `??`
    /// (non-Option `T?`-as-null-sentinel) JIT path — every source below
    /// requires a declared/recorded `Option<T>` shape, never a bare nullable
    /// scalar:
    ///   1. `infer_expr_type` → `type_is_option_carrier` (inline `Some(..)`,
    ///      Option-returning expressions the engine resolves).
    ///   2. Identifier: the recorded `ConcreteType::Option(_)` from a declared
    ///      `let x: int?` (local or module binding), or the tracker type-name
    ///      `"option"` stamped by `tracked_type_name_from_annotation`.
    ///   3. `FunctionCall`: the callee's declared return annotation is
    ///      `Option<T>` / `T?` (a `T?`-returning fn then `?? d`).
    ///   4. `PropertyAccess`: the receiver-schema field type is
    ///      `FieldType::Option(_)` (a `T?` field then `?? d`).
    ///
    /// Deopt is always correctness-preserving; conservative over-detection of a
    /// carrier only loses the JIT path for that one program, never diverges.
    fn null_coalesce_lhs_is_option_carrier(&mut self, left: &Expr) -> bool {
        // Source 1: inference engine (inline `Some(..)`, resolvable exprs).
        if self
            .infer_expr_type(left)
            .ok()
            .as_ref()
            .is_some_and(Self::type_is_option_carrier)
        {
            return true;
        }

        match left {
            // Source 2: a let-bound Option-typed local / module binding.
            Expr::Identifier(name, _) => {
                use shape_value::v2::ConcreteType;
                // Recorded ConcreteType from the declared annotation
                // (`let x: int?` → `ConcreteType::Option(_)`).
                let recorded_option = self
                    .resolve_local(name)
                    .and_then(|idx| self.current_function_local_concrete_types.get(&idx))
                    .or_else(|| {
                        self.module_bindings
                            .get(name)
                            .and_then(|idx| self.module_binding_concrete_types.get(idx))
                    })
                    .is_some_and(|ct| matches!(ct, ConcreteType::Option(_)));
                if recorded_option {
                    return true;
                }
                // Tracker type-name wrapper: `tracked_type_name_from_annotation`
                // stamps a declared `T?` local as the lowercased `"option"`.
                if self
                    .tracker_type_name_for_identifier(name)
                    .as_deref()
                    == Some("option")
                {
                    return true;
                }
                false
            }
            // Source 3: a `T?`-returning function then `?? d`.
            Expr::FunctionCall { name, .. } => self
                .function_defs
                .get(name)
                .and_then(|def| def.return_type.as_ref())
                .is_some_and(Self::annotation_is_option_carrier),
            // Source 4: a `T?`-typed schema field then `?? d`.
            Expr::PropertyAccess {
                object,
                property,
                optional,
                ..
            } => {
                if *optional {
                    return false;
                }
                use shape_runtime::type_schema::FieldType;
                self.tracker_schema_id_for_expr(object)
                    .and_then(|sid| self.type_tracker.schema_registry().get_by_id(sid))
                    .and_then(|schema| schema.get_field(property))
                    .is_some_and(|field| matches!(field.field_type, FieldType::Option(_)))
            }
            _ => false,
        }
    }

    /// Compile a binary operation expression.
    ///
    /// `op_span` is the source span of the parent `Expr::BinaryOp` node
    /// (W10 jit-call-method-user-trait-fix, 2026-05-17). Threaded into
    /// `emit_operator_trait_call` / `try_emit_trait_dispatch` so the
    /// operator-trait-dispatch side-table keys match the MIR lowering's
    /// statement span (see `crates/shape-vm/src/mir/lowering/expr.rs:1716`).
    pub(super) fn compile_expr_binary_op(
        &mut self,
        left: &Expr,
        op: &BinaryOp,
        right: &Expr,
        op_span: Span,
    ) -> Result<()> {
        // v0.3.3 c6-binop-ref-operand-segfault (Wave 1 Round 2, 2026-05-28).
        // Refuse `&` / `&mut` as a direct binop operand at semantic-check
        // time. The c6 JOINT-FIX (merge `30f36307`) closed the validator-
        // bypass family (silent module-binding loan escape) but `let b =
        // f(&a) + &a` still SEGFAULT'd in JIT mode and produced a runtime
        // error in VM mode — because neither operand has a proven numeric
        // kind, the bytecode compiler emits `CallMethod("add")`, and at
        // JIT-compile time the receiver/arg kinds on the §2.7.7/Q9
        // parallel-kind track are UInt64 (opaque bits) so the JIT
        // `jit_call_method` shell falls into the legacy heap-prefix
        // dispatch which deref-reads `read_heap_kind(receiver_bits)` —
        // SIGSEGV on the raw integer / dangling-ref bits. Reject the shape
        // here, before any code reaches the dispatcher. Mirrors the c2-A
        // semantic-error pattern: structured `SemanticError` pointing at
        // the operator span; the hint suggests `*ref` deref. See audit
        // doc 06 §5(c).
        if let Some(ref_span) = reference_operand_span(left, right) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Cannot apply binary operator `{:?}` to a reference operand. \
                     `&x` produces a borrow (reference), not a value — arithmetic, \
                     comparison, and other binary operators are not defined on \
                     references. Hint: dereference the operand with `*ref` to use \
                     its underlying value, or restructure to keep refs out of \
                     binary expressions.",
                    op
                ),
                location: Some(self.span_to_source_location(ref_span)),
            });
        }
        // R4 c6-widen (v0.3.3 strict-flip): the syntactic check above only
        // catches `&x` operands. Now that `&T` / `&mut T` is expressible in
        // type position (R1), a reference-TYPED operand — e.g. the result of
        // `fn make() -> &int { ... }` used as `make() + 1` — must be refused
        // here too, before any code reaches the JIT `jit_call_method`
        // misaligned-pointer dispatch. Same diagnostic shape as the
        // syntactic case; underline the operator span (the typed operand has
        // no single `&` token to point at).
        if let Some(ref_span) = self.reference_typed_operand_span(left, right) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Cannot apply binary operator `{:?}` to a reference-typed \
                     operand. The operand has a reference (borrow) type `&T` — \
                     arithmetic, comparison, and other binary operators are not \
                     defined on references. Hint: dereference the operand with \
                     `*ref` to use its underlying value, or restructure to keep \
                     refs out of binary expressions.",
                    op
                ),
                location: Some(self.span_to_source_location(ref_span)),
            });
        }
        match op {
            BinaryOp::And => {
                self.compile_expr(left)?;
                let false_jump = self.emit_jump(OpCode::JumpIfFalse, 0);

                self.compile_expr(right)?;
                self.emit(Instruction::simple(OpCode::Not));
                self.emit(Instruction::simple(OpCode::Not));
                let end_jump = self.emit_jump(OpCode::Jump, 0);

                self.patch_jump(false_jump);
                self.emit_bool(false);
                self.patch_jump(end_jump);
                // Boolean result — not a TypedObject or numeric
                self.last_expr_schema = None;
                self.last_expr_numeric_type = None;
            }
            BinaryOp::Or => {
                self.compile_expr(left)?;
                let true_jump = self.emit_jump(OpCode::JumpIfTrue, 0);

                self.compile_expr(right)?;
                self.emit(Instruction::simple(OpCode::Not));
                self.emit(Instruction::simple(OpCode::Not));
                let end_jump = self.emit_jump(OpCode::Jump, 0);

                self.patch_jump(true_jump);
                self.emit_bool(true);
                self.patch_jump(end_jump);
                // Boolean result — not a TypedObject or numeric
                self.last_expr_schema = None;
                self.last_expr_numeric_type = None;
            }
            BinaryOp::NullCoalesce => {
                // Short-circuit null coalescing: a ?? b
                // Only evaluate RHS if LHS is None/absent.
                //
                // Stack discipline (v0.3.3 book-gate fix: `CoalesceProbe`
                // replaces `Dup; IsNull` so that an `Option<T>` carrier
                // `Some(v)` is UNWRAPPED to `v` instead of leaking the
                // wrapper — `Some(5) ?? 99 -> 5`):
                //   1. compile LHS          -> [lhs]
                //   2. CoalesceProbe         -> [present_lhs, is_absent]
                //        present_lhs = unwrapped inner of Some(v) / bare lhs;
                //        a Null placeholder on the absent branch.
                //   3. JumpIfFalse use_lhs   -> [present_lhs]  (lhs present)
                //   4. Pop                   -> []   (discard absent placeholder)
                //   5. compile RHS           -> [rhs]
                //   6. Jump end
                //   use_lhs:                 -> [present_lhs] (already on stack)
                //   end:
                // The VM↔JIT divergence is specific to an `Option<T>`
                // carrier LHS (`Some(v)` → `Arc<OptionData>`): the VM
                // `CoalesceProbe` unwraps it, but the JIT MIR
                // `lower_null_coalesce` models `??` as `Eq` against
                // `MirConstant::None` with no `Arc<OptionData>` unwrap, so
                // it would leak the `Some(v)` wrapper. For a plain nullable
                // (`T?`) LHS the JIT `Eq None` path already matches the VM
                // (the null sentinel IS comparable to None), so NO deopt is
                // needed — gating on the Option-carrier case avoids a
                // blanket JIT regression for every `??` (e.g. the stdlib's
                // `ctx[...] ?? default` nullable-index pattern).
                // A-2 (2026-06-17): the residual flag must fire whenever the
                // `??` LHS STATIC type is an Option carrier — not only the
                // inline `Some(..)` shape the prior `infer_expr_type` check
                // caught, but also a let-bound `x: int?` local, a
                // `T?`-returning function call, and a `T?`-typed field. See
                // `null_coalesce_lhs_is_option_carrier`.
                let lhs_is_option_carrier = self.null_coalesce_lhs_is_option_carrier(left);
                self.compile_expr(left)?;
                self.emit(Instruction::simple(OpCode::CoalesceProbe));
                if lhs_is_option_carrier {
                    // JIT MIR has no Option-unwrap lowering for `??`; deopt
                    // the whole program to the (correct) interpreter so
                    // VM == JIT. Same surface-and-stop shape as the `?`
                    // operator's `has_try_unwrap_residual` flag.
                    self.program.has_null_coalesce_residual = true;
                }
                let use_lhs_jump = self.emit_jump(OpCode::JumpIfFalse, 0);
                // LHS was absent — pop the placeholder, compile RHS
                self.emit(Instruction::simple(OpCode::Pop));
                self.compile_expr(right)?;
                // D4 (S4): `a ?? b` yields the present value, whose numeric kind
                // equals the default `b`'s numeric kind (both branches agree by
                // type — strict typing requires it). Capture the RIGHT operand's
                // numeric type so the result carries a proven numeric kind. The
                // prior unconditional `= None` dropped it, so a `let h = m.get(k)
                // ?? 0` binding lost its `int` kind — outside a loop the slot
                // tracker recovered it via `concrete_type_for_expr`, but inside a
                // loop body `h` read back as `unknown` and a downstream `h + 1`
                // emitted a dynamic `add` method dispatch → runtime "no method
                // 'add' on receiver kind Int64". Propagating the right operand's
                // numeric type is the proof (ADR-006 §2.7.5) — no Bool-default,
                // no fabrication (a non-numeric right operand leaves it `None`).
                let coalesce_numeric = self.last_expr_numeric_type;
                let end_jump = self.emit_jump(OpCode::Jump, 0);
                // LHS was present — the unwrapped value is already on the stack
                self.patch_jump(use_lhs_jump);
                self.patch_jump(end_jump);
                self.last_expr_schema = None;
                self.last_expr_numeric_type = coalesce_numeric;
            }
            BinaryOp::ErrorContext => {
                // WS-3 F3: the `!!` error-context operator. Before this
                // arm existed, `ErrorContext` fell into the generic `_ =>`
                // arithmetic arm, whose strict-operand-type gate rejected
                // the `Result<…>` left operand — so a core operator could
                // not be compiled at all.
                //
                // `value !! context`: the `op_error_context` handler
                // (`executor/exceptions/mod.rs`) pops `context` (top of
                // stack) then `value`, so we compile `left` (value) then
                // `right` (context). On the success leg the handler
                // unwraps to the inner value (`Ok(v) => v`, `Some(v) => v`,
                // bare `v => v`); on the failure leg it builds an AnyError
                // and throws. The opcode + handler already exist; this arm
                // is the only missing dispatch piece.
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                self.emit(Instruction::simple(OpCode::ErrorContext));
                // `!!` yields the UNWRAPPED success value `T` (same as `?`
                // — both unwrap `Ok(v)`/`Some(v)` to `v` on success).
                // Stamp the tracker with the unwrapped success type so a
                // downstream `let v = expr !! "ctx"` records `v`'s type.
                self.stamp_unwrapped_success_type(left);
            }
            BinaryOp::Pipe => {
                // Pipe operator: a |> f transforms to f(a)
                // a |> f(x) transforms to f(a, x)
                match right {
                    Expr::FunctionCall {
                        name,
                        args,
                        named_args,
                        span,
                    } => {
                        // a |> f(x, y) -> f(a, x, y)
                        let mut new_args = vec![left.clone()];
                        new_args.extend(args.iter().cloned());
                        let new_call = Expr::FunctionCall {
                            name: name.clone(),
                            args: new_args,
                            named_args: named_args.clone(),
                            span: *span,
                        };
                        self.compile_expr(&new_call)?;
                    }
                    Expr::MethodCall {
                        receiver,
                        method,
                        args,
                        named_args,
                        optional,
                        span,
                    } => {
                        // a |> obj.method(x) -> obj.method(a, x)
                        let mut new_args = vec![left.clone()];
                        new_args.extend(args.iter().cloned());
                        let new_call = Expr::MethodCall {
                            receiver: receiver.clone(),
                            method: method.clone(),
                            args: new_args,
                            named_args: named_args.clone(),
                            optional: *optional,
                            span: *span,
                        };
                        self.compile_expr(&new_call)?;
                    }
                    Expr::Identifier(name, span) => {
                        // a |> f -> f(a)
                        let new_call = Expr::FunctionCall {
                            name: name.clone(),
                            args: vec![left.clone()],
                            named_args: vec![],
                            span: *span,
                        };
                        self.compile_expr(&new_call)?;
                    }
                    _ => {
                        return Err(ShapeError::RuntimeError {
                            message:
                                "Pipe operator requires a function or method call on the right side"
                                    .to_string(),
                            location: None,
                        });
                    }
                }
            }
            BinaryOp::Add => {
                // R5.4E: retarget typed element-wise Matrix/Vec arithmetic
                // ahead of any operand compilation. These helpers compile
                // both operands + an arg-count constant + emit `BuiltinCall`
                // for the matching `IntrinsicMat*` / `IntrinsicVec*`; they
                // never fall through to the dynamic-fallback `AddDynamic`.
                // Vector first (covers `Vec<number>+Vec<number>` and
                // `Vec<int>+Vec<int>`), then matrix (`Mat<number>+Mat<number>`).
                // The operand shapes are disjoint so ordering is not
                // observable, but we mirror the pattern used in the generic
                // `_ => {}` arm below.
                if self.try_compile_typed_vec_arithmetic(&BinaryOp::Add, left, right)? {
                    return Ok(());
                }
                if self.try_compile_typed_matrix_arithmetic(&BinaryOp::Add, left, right)? {
                    return Ok(());
                }

                // For Add, check if we can do typed merge optimization
                self.compile_expr(left)?;
                let left_schema = self.last_expr_schema.take();
                let mut left_numeric = self.last_expr_numeric_type;

                self.compile_expr(right)?;
                let right_schema = self.last_expr_schema.take();
                let mut right_numeric = self.last_expr_numeric_type;

                // If one side is numeric and the other is an identifier/index read with no
                // hint yet, adopt the known numeric kind and seed slot hints.
                self.adopt_missing_numeric_operand_hint(
                    left,
                    right,
                    &mut left_numeric,
                    &mut right_numeric,
                );

                // ADR-006 §2.7.5 stamp-at-compile-time — int-literal width
                // inference. A bare integer literal adopts the width of its
                // sibling so `a + 1` (u64) stays on the unsigned carrier and
                // `x + 1` (i8) stays on the truncating narrow carrier.
                Self::promote_int_literal_to_width_sibling(
                    left,
                    right,
                    &mut left_numeric,
                    &mut right_numeric,
                );

                // Priority 1: typed object merge (both operands are TypedObjects)
                // Exception: if the left type implements Add, skip merge and emit
                // generic Add so the executor's operator trait dispatch handles it.
                if let (Some(left_id), Some(right_id)) = (left_schema, right_schema) {
                    let left_has_add = self
                        .type_tracker
                        .schema_registry()
                        .get_by_id(left_id)
                        .is_some_and(|schema| {
                            self.type_inference
                                .env
                                .type_implements_trait(&schema.name, "Add")
                        });
                    if left_has_add {
                        // Phase 2.5: operator trait dispatch via CallMethod.
                        // The left operand (receiver) and right operand (arg)
                        // are already on the stack from compile_expr above.
                        emit_operator_trait_call(self, "add", op_span);
                        // The result of `impl Add for T` is `T` (the receiver
                        // type). `emit_operator_trait_call` clears the
                        // expr-type tracking; restore the LEFT schema so a
                        // chained / assigned use sees the result type. Without
                        // this, `acc = acc + x` (or `acc += x`) lost `acc`'s
                        // schema and the NEXT `acc + y` failed to resolve the
                        // operator trait ("operand types are unknown").
                        // (operators slice — compound-assign fix)
                        self.restore_operator_trait_result_schema(left_id);
                    } else {
                        self.compile_typed_merge(left_id, right_id)?;
                        self.last_expr_numeric_type = None;
                    }
                }
                // Priority 2: typed numeric add (same types or mixed Int/Number with coercion)
                //
                // Add is overloaded (numeric add, string concat, array concat,
                // object merge).  Only emit typed numeric opcodes when we have
                // *direct* evidence that both operands are numeric — i.e. each
                // is either a numeric literal or an immutable local whose
                // storage hint is a numeric family.  Without that evidence the
                // `last_expr_numeric_type` values may come from speculative
                // inference hints (inferred_param_type_hints) which can be wrong
                // when a param is actually a string.
                else {
                    // Priority 1.5: dedicated StringConcat / ArrayConcat for
                    // built-in heap types whose operand kinds the compiler can
                    // prove. These replace the heap-heap arms in `exec_arithmetic`
                    // (Phase 2.3 / 2.4) without going through the generic Add
                    // dispatch.
                    let inferred_lhs = self.infer_expr_type(left).ok();
                    let inferred_rhs = self.infer_expr_type(right).ok();
                    let lhs_name = inferred_lhs.as_ref().map(type_display_name);
                    let rhs_name = inferred_rhs.as_ref().map(type_display_name);

                    // String / Char concat: any combination of string + char,
                    // as long as at least one operand is a string. Char + Char
                    // also produces a string (matches the legacy heap-heap arms).
                    let is_strish =
                        |n: &Option<String>| matches!(n.as_deref(), Some("string") | Some("char"));
                    let either_is_string = matches!(lhs_name.as_deref(), Some("string"))
                        || matches!(rhs_name.as_deref(), Some("string"));
                    if is_strish(&lhs_name) && is_strish(&rhs_name) && either_is_string {
                        // Use the typed string concatenation opcode when both
                        // operands are proven strings/chars.
                        self.emit(Instruction::simple(OpCode::StringConcatTyped));
                        self.last_expr_schema = None;
                        // Phase 3e: result of string concat is a string —
                        // propagate so chained concats and assignment-target
                        // type tracking see the type.
                        self.last_expr_type_info = Some(
                            crate::type_tracking::VariableTypeInfo::named("string".to_string()),
                        );
                        self.last_expr_numeric_type = None;
                        return Ok(());
                    }

                    // Strict no-coercion ruling (user 2026-06-14): `string +
                    // non-string` is a COMPILE ERROR. The former R5.5 typed
                    // string + scalar concat path (which emitted
                    // `StringConcatInt`/`Number`/`Bool` auto-stringify opcodes)
                    // is deleted. Under strict typing the non-string operand is
                    // NOT implicitly stringified; the both-strings case already
                    // returned above via `StringConcatTyped`. Here we detect the
                    // mixed case (exactly one operand a string/char, the other a
                    // known non-string concrete type) and reject with a
                    // diagnostic that names f-string interpolation as the
                    // alternative.
                    //
                    // Resolve string-ness via the same multi-source order the
                    // surrounding arithmetic branch uses: `infer_expr_type`
                    // display name, then the `storage_hint_for_expr`
                    // `NativeKind::String` hint (set by `let x: string = ...`
                    // annotations and by literals).
                    let lhs_is_string = is_strish(&lhs_name)
                        || matches!(
                            self.storage_hint_for_expr(left),
                            Some(crate::type_tracking::NativeKind::String)
                        );
                    let rhs_is_string = is_strish(&rhs_name)
                        || matches!(
                            self.storage_hint_for_expr(right),
                            Some(crate::type_tracking::NativeKind::String)
                        );
                    // Exactly one side is a string. The other operand's type is
                    // resolved (numeric/bool/heap) — there is no valid `+` here.
                    if lhs_is_string != rhs_is_string {
                        let lhs_disp = lhs_name.as_deref().unwrap_or("unknown");
                        let rhs_disp = rhs_name.as_deref().unwrap_or("unknown");
                        let (lhs_disp, rhs_disp) = (
                            if lhs_is_string { "string" } else { lhs_disp },
                            if rhs_is_string { "string" } else { rhs_disp },
                        );
                        return Err(string_plus_nonstring_error(
                            self, left, right, lhs_disp, rhs_disp,
                        ));
                    }

                    // Array concat: both operands proven to be arrays. Fires
                    // for every array element kind — numeric (`int[]` /
                    // `number[]`), string, and struct arrays all concatenate
                    // uniformly (USER RULING 2026-06-17: numeric-array `+` is
                    // CONCATENATION, not element-wise add). The element-wise
                    // SIMD `IntrinsicVec*` path no longer claims `+`; it is
                    // reserved for a future `Vec`-type / method form. Display
                    // name comes from `type_display_name`: a generic `Array<T>`
                    // formats as "Array", and a legacy `T[]` formats as "T[]".
                    let is_arrayish = |n: &Option<String>| match n.as_deref() {
                        Some("Array") => true,
                        Some(s) if s.ends_with("[]") => true,
                        _ => false,
                    };
                    if is_arrayish(&lhs_name) && is_arrayish(&rhs_name) {
                        self.emit(Instruction::simple(OpCode::ArrayConcat));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = None;
                        return Ok(());
                    }

                    // DateTime/Duration addition: at least one side is
                    // DateTime or Duration (TimeSpan). Dispatch via
                    // CallMethod("add") so the executor's PHF-backed
                    // datetime/timespan method registry handles the
                    // type combinations. Replaces the generic Add path.
                    //
                    // Phase 3e: accept both PascalCase and lowercase
                    // forms — the compiler's tracker uses PascalCase
                    // but the runtime inference engine returns lowercase.
                    let is_temporal = |n: &Option<String>| {
                        matches!(
                            n.as_deref(),
                            Some("DateTime")
                                | Some("Duration")
                                | Some("TimeSpan")
                                | Some("datetime")
                                | Some("duration")
                                | Some("timespan")
                        )
                    };
                    if is_temporal(&lhs_name) || is_temporal(&rhs_name) {
                        let method_id = shape_value::MethodId::from_name("add");
                        let string_id = self.program.add_string("add".to_string());
                        self.emit(Instruction::new(
                            OpCode::CallMethod,
                            Some(Operand::TypedMethodCall {
                                method_id: method_id.0,
                                arg_count: 1,
                                string_id,
                                receiver_type_tag: 0xFF,
                            }),
                        ));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.last_expr_numeric_type = None;
                        return Ok(());
                    }

                    // Path 4: if infer_expr_type resolved a numeric type
                    // name, fill in missing NumericType for the coercion
                    // planner.
                    let inferred_numeric = |n: &Option<String>| -> Option<NumericType> {
                        match n.as_deref() {
                            Some("int") => Some(NumericType::Int),
                            Some("number") => Some(NumericType::Number),
                            Some("decimal") => Some(NumericType::Decimal),
                            _ => None,
                        }
                    };
                    let lhs_inferred_num = inferred_numeric(&lhs_name);
                    let rhs_inferred_num = inferred_numeric(&rhs_name);
                    if left_numeric.is_none() && lhs_inferred_num.is_some() {
                        left_numeric = lhs_inferred_num;
                    }
                    if right_numeric.is_none() && rhs_inferred_num.is_some() {
                        right_numeric = rhs_inferred_num;
                    }

                    // Confirm each operand is numeric via one of four paths:
                    // 1. Syntactic: it's a numeric literal
                    // 2. Storage hint: it's a local with a known numeric hint
                    //    (excludes untyped function params — see param_locals)
                    // 3. Type tracker info, excluding only untyped function
                    //    params (param_locals) whose inferred hints can be
                    //    wrong (B19). Non-param identifiers (locals, for-loop
                    //    vars, module bindings) have reliable tracker info.
                    // 4. infer_expr_type resolved a numeric type name
                    let is_untyped_param = |e: &Expr| -> bool {
                        if let Expr::Identifier(name, _) = e {
                            if let Some(idx) = self.resolve_local(name) {
                                return self.param_locals.contains(&idx);
                            }
                        }
                        false
                    };
                    // Path 5: check if identifier resolves to a local whose
                    // type_name in the type tracker is numeric. This covers
                    // locals and for-loop variables that have a known type
                    // name but whose storage_hint is Unknown (not yet
                    // propagated).
                    let local_has_numeric_type_name = |e: &Expr| -> Option<NumericType> {
                        if let Expr::Identifier(name, _) = e {
                            if let Some(idx) = self.resolve_local(name) {
                                if self.param_locals.contains(&idx) {
                                    return None;
                                }
                                if let Some(info) = self.type_tracker.get_local_type(idx) {
                                    if let Some(ref tn) = info.type_name {
                                        return match tn.as_str() {
                                            "int" | "Int" | "Integer" | "i64" => {
                                                Some(NumericType::Int)
                                            }
                                            "number" | "Number" | "Float" | "f64" => {
                                                Some(NumericType::Number)
                                            }
                                            "decimal" | "Decimal" => Some(NumericType::Decimal),
                                            _ => None,
                                        };
                                    }
                                }
                            }
                        }
                        None
                    };
                    let lhs_local_num = local_has_numeric_type_name(left);
                    let rhs_local_num = local_has_numeric_type_name(right);
                    if left_numeric.is_none() && lhs_local_num.is_some() {
                        left_numeric = lhs_local_num;
                    }
                    if right_numeric.is_none() && rhs_local_num.is_some() {
                        right_numeric = rhs_local_num;
                    }
                    let lhs_confirmed = Self::is_expr_confirmed_numeric(left)
                        || self
                            .storage_hint_for_expr(left)
                            .is_some_and(|h| h.is_numeric_family())
                        || (!is_untyped_param(left) && left_numeric.is_some())
                        || lhs_inferred_num.is_some()
                        || lhs_local_num.is_some();
                    let rhs_confirmed = Self::is_expr_confirmed_numeric(right)
                        || self
                            .storage_hint_for_expr(right)
                            .is_some_and(|h| h.is_numeric_family())
                        || (!is_untyped_param(right) && right_numeric.is_some())
                        || rhs_inferred_num.is_some()
                        || rhs_local_num.is_some();

                    let primary = if lhs_confirmed && rhs_confirmed {
                        self.emit_numeric_binary_with_coercion_trusted(
                            &BinaryOp::Add,
                            left_numeric,
                            right_numeric,
                            false,
                            left,
                            right,
                        )
                    } else {
                        NumericEmitResult::NoPlan
                    };
                    match primary {
                        NumericEmitResult::EmittedTyped => {
                            self.last_expr_schema = None;
                        }
                        NumericEmitResult::CoercedNeedsGeneric | NumericEmitResult::NoPlan => {
                            // R5.2B: retarget user-defined `impl Add for T` to
                            // `CallMethod` at compile time, mirroring the
                            // symmetric strict-arithmetic path at L1230-1244 and
                            // the numeric-declined paths at L1260/L1279. Uses
                            // `left_schema` captured at L646. When the helper
                            // emits `CallMethod`, we skip the dynamic fallthrough
                            // so that `exec_arithmetic_dynamic_fallback::
                            // try_binary_operator_trait` is never reached for
                            // user-op Add. No new opcode is required; CallMethod
                            // with `Operand::TypedMethodCall` already dispatches
                            // to user impl methods via the function_name_index
                            // (see `executor/objects/mod.rs::op_call_method`,
                            // L1427-L1458).
                            //
                            // When the helper declines (no schema / no matching
                            // impl), the historical path emitted `AddDynamic`
                            // for non-numeric operand combinations (DateTime,
                            // mixed string, polyglot value). Strict-typing
                            // sweep (Phase 1): that dynamic-fallback emission
                            // is now a hard compile error.
                            if !try_emit_trait_dispatch(
                                self,
                                &BinaryOp::Add,
                                left_schema,
                                left,
                                op_span,
                            ) {
                                // A-final ROOT-C: defer the dead deferred-template
                                // body's unprovable-kind `a + b` (emit Pop, no
                                // typed opcode) instead of the strict-typing error.
                                if self.defer_template_numeric_binop() {
                                    return Ok(());
                                }
                                return Err(strict_typing_binop_error(
                                    self,
                                    &BinaryOp::Add,
                                    left,
                                    right,
                                ));
                            }
                        }
                    }
                }
            }
            BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitShl
            | BinaryOp::BitShr => {
                // Phase R5.1C: emit typed bitwise opcodes (`BitAndInt`,
                // `BitOrInt`, `BitXorInt`, `BitShlInt`, `BitShrInt`) when
                // both operand types are provably `int` at compile time.
                // Mixed-type / unresolved cases fall through to the
                // Dynamic (`BitAnd`/`BitOr`/...) variants emitted by
                // `compile_binary_op`.
                //
                // Semantics match the Dynamic variants exactly: no
                // shift-count masking, i48 payload truncation applies.
                // See R5.1B commit body for the edge-case notes.
                //
                // Gate: `SHAPE_V2_TYPED_BITWISE` (default ON via
                // `typed_bitwise_enabled()`). With the flag off, emission
                // is byte-identical to pre-R5.1C.
                self.compile_expr(left)?;
                let mut left_numeric = self.last_expr_numeric_type;
                let left_schema = self.last_expr_schema;
                self.compile_expr(right)?;
                let mut right_numeric = self.last_expr_numeric_type;

                // W1.9: user-defined `impl BitAnd / BitOr / BitXor for X`
                // dispatch — if the left-operand's TypedObject schema
                // implements the matching operator trait, emit a
                // `CallMethod("bitand"/"bitor"/"bitxor")` and return.
                // Both operands are already on the stack from the
                // compile_expr calls above. Mirrors the Add arm's
                // pattern at L756-790 and Sub/Mul/Div/Mod's trait
                // dispatch at L1462-1475.
                if matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor) {
                    if let (Some(trait_name), Some(method_name)) =
                        (operator_trait_for_op(op), operator_trait_method_for_op(op))
                    {
                        let left_implements = left_schema
                            .and_then(|sid| self.type_tracker.schema_registry().get_by_id(sid))
                            .is_some_and(|schema| {
                                self.type_inference
                                    .env
                                    .type_implements_trait(&schema.name, trait_name)
                            });
                        if left_implements {
                            emit_operator_trait_call(self, method_name, op_span);
                            return Ok(());
                        }
                    }
                }

                // Don't trust inferred numeric types for untyped function
                // parameters (same rationale as the `param_locals` guard
                // in the `_ => {}` arithmetic branch below).
                if let Expr::Identifier(name, _) = left {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.param_locals.contains(&local_idx) {
                            left_numeric = None;
                        }
                    }
                }
                if let Expr::Identifier(name, _) = right {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.param_locals.contains(&local_idx) {
                            right_numeric = None;
                        }
                    }
                }

                // Fall back to the inference engine when slot tracking
                // did not produce a numeric hint. This mirrors the
                // `NoPlan` path in the `_ => {}` arithmetic branch.
                if left_numeric.is_none() {
                    left_numeric = self
                        .infer_expr_type(left)
                        .ok()
                        .and_then(|t| inferred_type_to_numeric(&t));
                }
                if right_numeric.is_none() {
                    right_numeric = self
                        .infer_expr_type(right)
                        .ok()
                        .and_then(|t| inferred_type_to_numeric(&t));
                }

                let both_int = matches!(left_numeric, Some(NumericType::Int))
                    && matches!(right_numeric, Some(NumericType::Int));

                // W1.10 (v0.3 R2): user-type operator trait dispatch for
                // `<<` / `>>`. When the left operand has a TypedObject
                // schema implementing `Shl` / `Shr`, emit a `CallMethod`
                // dispatch via the shared `emit_operator_trait_call`
                // path (mirrors the Phase 2.5 dispatch in the generic
                // `_ =>` arithmetic arm at L1456-1475 + Add's dedicated
                // arm at L775-794). Runs only when typed-int emission
                // is not eligible — the typed bitwise path takes
                // precedence for `int << int` to preserve the existing
                // zero-dispatch behavior. BitAnd/BitOr/BitXor handled
                // by sibling W1.9 dispatch above.
                //
                // c5 Phase B (2026-05-28): moved BEFORE the strict-typing
                // gate below so user-type `impl Shl/Shr` dispatch can
                // fire on non-int receivers (e.g. `vec << 2`). The gate
                // would otherwise reject every non-int left operand.
                if !both_int && matches!(op, BinaryOp::BitShl | BinaryOp::BitShr) {
                    let trait_name = operator_trait_for_op(op);
                    let method_name = operator_trait_method_for_op(op);
                    if let (Some(trait_name), Some(method_name)) = (trait_name, method_name) {
                        let has_trait_via_schema = left_schema
                            .and_then(|sid| self.type_tracker.schema_registry().get_by_id(sid))
                            .is_some_and(|schema| {
                                self.type_inference
                                    .env
                                    .type_implements_trait(&schema.name, trait_name)
                            });
                        let has_trait = has_trait_via_schema
                            || self.infer_expr_type(left).ok().is_some_and(|ty| {
                                let name = type_display_name(&ty);
                                self.type_inference
                                    .env
                                    .type_implements_trait(&name, trait_name)
                            });
                        if has_trait {
                            emit_operator_trait_call(self, method_name, op_span);
                            return Ok(());
                        }
                    }
                }

                // c5 Phase B (v0.3.3, 2026-05-28) — bitwise-strict-typing
                // gate. When both operands aren't provably `int` AND no
                // user operator-trait dispatch fires above, refuse at
                // compile time. Pre-fix, the else-branch at L1555 emitted
                // a `BitAnd`/`BitOr`/… dynamic opcode whose executor
                // (`exec_dyn_bit_binary`) discarded the operand kinds and
                // reinterpreted the slot bits as i64 — silently producing
                // garbage integers for `1.5 | 3`, `"hello" & 3`, etc.
                // (audit doc 05 §3 quote, audit doc 05a §c5 anchor sites).
                //
                // The polarity is producer-side: refuse at the compiler
                // rather than preserving kinds at the consumer (diverges
                // from c5/c7/1b precedent because the wrong-bits-on-stack
                // consequence flows from accepting non-int operands at
                // compile time, not from fabricating kind at runtime). Per
                // CLAUDE.md §Type-System-Rules: "NO runtime coercion".
                //
                // The `SHAPE_V2_TYPED_BITWISE` rollback flag (R5.1C) is
                // not consulted here — the dynamic `BitAnd`/`BitOr`/etc.
                // opcodes that the flag-off path emitted are deleted as
                // part of this Phase B fix. The typed-int path is the
                // only path that survives.
                debug_assert!(
                    is_strict_bitwise(op),
                    "bitwise arm must classify as is_strict_bitwise (gate site)"
                );
                if !both_int {
                    let op_symbol = match op {
                        BinaryOp::BitAnd => "&",
                        BinaryOp::BitOr => "|",
                        BinaryOp::BitXor => "^",
                        BinaryOp::BitShl => "<<",
                        BinaryOp::BitShr => ">>",
                        _ => unreachable!(),
                    };
                    let left_desc = match left_numeric {
                        Some(NumericType::Int) => "int".to_string(),
                        Some(_) => self
                            .infer_expr_type(left)
                            .map(|t| type_display_name(&t))
                            .unwrap_or_else(|_| "unknown".to_string()),
                        None => self
                            .infer_expr_type(left)
                            .map(|t| type_display_name(&t))
                            .unwrap_or_else(|_| "unknown".to_string()),
                    };
                    let right_desc = match right_numeric {
                        Some(NumericType::Int) => "int".to_string(),
                        Some(_) => self
                            .infer_expr_type(right)
                            .map(|t| type_display_name(&t))
                            .unwrap_or_else(|_| "unknown".to_string()),
                        None => self
                            .infer_expr_type(right)
                            .map(|t| type_display_name(&t))
                            .unwrap_or_else(|_| "unknown".to_string()),
                    };
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Cannot apply '{}' to {} and {}. Bitwise operators require both \
                             operands to be `int` at compile time. Use an explicit cast \
                             (e.g. `(x as int) {} (y as int)`) when intentional.",
                            op_symbol, left_desc, right_desc, op_symbol,
                        ),
                        location: Some(self.span_to_source_location(combined_span(left, right))),
                    });
                }

                // c5 Phase B: with the strict-typing gate above refusing
                // every non-int operand at compile time, only the typed-
                // int path remains. Emit the proven-typed opcode.
                let typed_opcode = match op {
                    BinaryOp::BitAnd => OpCode::BitAndInt,
                    BinaryOp::BitOr => OpCode::BitOrInt,
                    BinaryOp::BitXor => OpCode::BitXorInt,
                    BinaryOp::BitShl => OpCode::BitShlInt,
                    BinaryOp::BitShr => OpCode::BitShrInt,
                    _ => unreachable!(),
                };
                self.emit(Instruction::simple(typed_opcode));
                // Typed bitwise op on two ints yields an int — preserve
                // the numeric hint so downstream typed emission keeps
                // working (e.g. (a & b) + c stays on the int path).
                self.last_expr_schema = None;
                self.last_expr_type_info = None;
                self.last_expr_numeric_type = Some(NumericType::Int);
            }
            _ => {
                // Typed matrix kernels: Mat<number> * Vec<number>/Mat<number>.
                // Lower before generic strict-arithmetic checks so typed matrix
                // paths never fall back to scalar arithmetic dispatch.
                if matches!(op, BinaryOp::Mul) && self.try_compile_typed_matrix_mul(left, right)? {
                    return Ok(());
                }

                // R5.4E: retarget typed element-wise vector arithmetic for
                // Sub/Mul/Div on `Vec<number>`. The Add case is handled in
                // the dedicated `BinaryOp::Add` arm above. Running before
                // the strict-arithmetic gate below is intentional: the
                // gate rejects non-numeric operand types, but a typed
                // `Vec<number>` param is non-numeric at the type level and
                // would otherwise produce a misleading "Cannot apply '-'
                // to Vec<number> and Vec<number>" error.
                //
                // Vector before matrix to match the generic fallback ordering
                // (the pre-R5.4E path routes vector ops through
                // `TypedArrayData::F64` SIMD while matrix ops go through the
                // heap-matrix arm). Ordering here is not semantically load-
                // bearing — the two helpers classify disjoint operand shapes
                // — but mirrors the shape of other compile-time retargets
                // in this file.
                if self.try_compile_typed_vec_arithmetic(op, left, right)? {
                    return Ok(());
                }
                if self.try_compile_typed_matrix_arithmetic(op, left, right)? {
                    return Ok(());
                }

                // Phase 2.6.5.3: inference-driven typed Eq/Neq dispatch.
                // Queries the inference engine for both operand types BEFORE
                // compiling them and emits the typed opcode directly. This is
                // the PRIMARY path for Equal/NotEqual; the legacy slot-tracker
                // dispatch below is the secondary fallback for cases inference
                // can't resolve.
                if self.compile_typed_equality(op, left, right, op_span)? {
                    return Ok(());
                }

                // Stage 4.2: typed string ordered comparison.
                // When both operands are proven strings and the op is an
                // ordered comparison (>, <, >=, <=), emit the specialized
                // string comparison opcode for zero-dispatch execution.
                if is_ordered_comparison(op) {
                    if let (Ok(lt), Ok(rt)) =
                        (self.infer_expr_type(left), self.infer_expr_type(right))
                    {
                        let lt_name = type_display_name(&lt);
                        let rt_name = type_display_name(&rt);
                        let is_strish = |n: &str| matches!(n, "string" | "char");
                        if is_strish(&lt_name) && is_strish(&rt_name) {
                            let string_cmp_op = match op {
                                BinaryOp::Greater => OpCode::GtString,
                                BinaryOp::Less => OpCode::LtString,
                                BinaryOp::GreaterEq => OpCode::GteString,
                                BinaryOp::LessEq => OpCode::LteString,
                                _ => unreachable!(),
                            };
                            self.compile_expr(left)?;
                            self.compile_expr(right)?;
                            self.emit(Instruction::simple(string_cmp_op));
                            self.last_expr_schema = None;
                            self.last_expr_type_info = None;
                            self.last_expr_numeric_type = None;
                            return Ok(());
                        }
                    }
                }

                // Stage 4.2: temporal Sub dispatch via CallMethod("sub").
                // When one operand is DateTime or Duration/TimeSpan, emit
                // CallMethod instead of falling through to the strict
                // arithmetic check which would reject non-numeric types.
                if matches!(op, BinaryOp::Sub) {
                    if let (Ok(lt), Ok(rt)) =
                        (self.infer_expr_type(left), self.infer_expr_type(right))
                    {
                        let lt_name = type_display_name(&lt);
                        let rt_name = type_display_name(&rt);
                        // Phase 3e: accept both PascalCase ("DateTime") and
                        // lowercase ("datetime") forms — the compiler's
                        // tracker uses PascalCase but the runtime
                        // inference engine returns lowercase for
                        // Expr::DateTime / Expr::Duration literals.
                        let is_temporal = |n: &str| {
                            matches!(
                                n,
                                "DateTime"
                                    | "Duration"
                                    | "TimeSpan"
                                    | "datetime"
                                    | "duration"
                                    | "timespan"
                            )
                        };
                        if is_temporal(&lt_name) || is_temporal(&rt_name) {
                            self.compile_expr(left)?;
                            self.compile_expr(right)?;
                            let method_id = shape_value::MethodId::from_name("sub");
                            let string_id = self.program.add_string("sub".to_string());
                            self.emit(Instruction::new(
                                OpCode::CallMethod,
                                Some(Operand::TypedMethodCall {
                                    method_id: method_id.0,
                                    arg_count: 1,
                                    string_id,
                                    receiver_type_tag: 0xFF,
                                }),
                            ));
                            self.last_expr_schema = None;
                            self.last_expr_type_info = None;
                            self.last_expr_numeric_type = None;
                            return Ok(());
                        }
                    }
                }

                // ── Compile-time type safety for strict arithmetic ──
                // Sub, Mul, Div, Mod, Pow require numeric operands.
                // If both types are known and either is non-numeric → compile error.
                if is_strict_arithmetic(op) {
                    if let (Ok(lt), Ok(rt)) =
                        (self.infer_expr_type(left), self.infer_expr_type(right))
                    {
                        // `infer_expr_type` runs outside the compiler's local-slot context.
                        // For identifiers that are currently bound in bytecode locals/module_bindings,
                        // an inferred Function type may be a shadowed builtin (e.g. `len`).
                        // In that case, defer to slot-based tracking below to avoid false errors.
                        let left_shadowed_builtin = matches!(left, Expr::Identifier(name, _)
                            if (self.resolve_local(name).is_some() || self.module_bindings.contains_key(name))
                                && is_function_type(&lt));
                        let right_shadowed_builtin = matches!(right, Expr::Identifier(name, _)
                            if (self.resolve_local(name).is_some() || self.module_bindings.contains_key(name))
                                && is_function_type(&rt));

                        if left_shadowed_builtin || right_shadowed_builtin {
                            // Skip this early semantic gate for shadowed identifiers.
                            // The typed/local tracking pass below will still enforce arithmetic safety.
                        } else if !is_type_numeric(&lt) || !is_type_numeric(&rt) {
                            // Check if the left operand's type implements an operator trait
                            // for this operation (e.g. impl Mul for Vec2). If so, allow the
                            // generic opcode through to the executor's trait dispatch.
                            let has_operator_trait = operator_trait_for_op(op)
                                .and_then(|trait_name| {
                                    let type_name = type_display_name(&lt);
                                    if self
                                        .type_inference
                                        .env
                                        .type_implements_trait(&type_name, trait_name)
                                    {
                                        Some(())
                                    } else {
                                        None
                                    }
                                })
                                .is_some();

                            if !has_operator_trait {
                                let op_symbol = match op {
                                    BinaryOp::Sub => "-",
                                    BinaryOp::Mul => "*",
                                    BinaryOp::Div => "/",
                                    BinaryOp::Mod => "%",
                                    BinaryOp::Pow => "**",
                                    _ => "?",
                                };
                                return Err(ShapeError::SemanticError {
                                    message: format!(
                                        "Cannot apply '{}' to {} and {}. Both operands must be numeric (int, number, or decimal).",
                                        op_symbol,
                                        type_display_name(&lt),
                                        type_display_name(&rt),
                                    ),
                                    location: Some(
                                        self.span_to_source_location(combined_span(left, right)),
                                    ),
                                });
                            }
                        }
                    }
                }

                // Numeric-conversion §4 literal adoption (binary-operand
                // widening, THE RULE user 2026-06-01): in an arithmetic /
                // ordered-comparison op, a bare int literal whose PARTNER operand
                // is proven `number`/`f64` IS the number literal (`n / 2` where
                // `n: number` ⇒ `2` is `2.0`). Re-lower the literal to a `Number`
                // constant BEFORE compiling it, so the operand carries Float64
                // bits and the op lowers to the `*Number` opcode with two real
                // f64 operands. Without this, the literal pushes Int64 bits, the
                // VM `DivNumber` handler tolerantly coerces (→ 2.5) but the JIT
                // does not (int-divides → 2) — a VM≠JIT divergence AND, at sites
                // with no VM coercion, a raw-bits reinterpret. Compile-time
                // literal re-typing, NOT a runtime coercion opcode.
                let widen_l;
                let widen_r;
                let (left, right): (&Expr, &Expr) =
                    if is_strict_arithmetic(op) || is_ordered_comparison(op) {
                        let left = if self.expr_proves_float(right) {
                            match crate::compiler::literal_widen::widen_int_literal_to_number(left) {
                                Some(w) => {
                                    widen_l = w;
                                    &widen_l
                                }
                                None => left,
                            }
                        } else {
                            left
                        };
                        let right = if self.expr_proves_float(left) {
                            match crate::compiler::literal_widen::widen_int_literal_to_number(right)
                            {
                                Some(w) => {
                                    widen_r = w;
                                    &widen_r
                                }
                                None => right,
                            }
                        } else {
                            right
                        };
                        (left, right)
                    } else {
                        (left, right)
                    };

                // ── Compile operands, capture numeric types and schemas ──
                self.compile_expr(left)?;
                let mut left_numeric = self.last_expr_numeric_type;
                let left_schema = self.last_expr_schema;
                self.compile_expr(right)?;
                let mut right_numeric = self.last_expr_numeric_type;
                let right_schema = self.last_expr_schema;

                // Don't trust inferred numeric types for untyped function parameters.
                // Their inferred_param_type_hints can be wrong (same rationale as the
                // param_locals guard in storage_hint_for_expr for Add).  Without an
                // explicit type annotation the parameter may receive values of any
                // type at runtime, so fall back to generic opcodes.
                if let Expr::Identifier(name, _) = left {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.param_locals.contains(&local_idx) {
                            left_numeric = None;
                        }
                    }
                }
                if let Expr::Identifier(name, _) = right {
                    if let Some(local_idx) = self.resolve_local(name) {
                        if self.param_locals.contains(&local_idx) {
                            right_numeric = None;
                        }
                    }
                }

                // WS-9 / WS-9b: an access operand (`a[0]` index access, or
                // `a.lo` property access) carries no numeric hint from
                // `compile_expr` when the receiver is an unannotated
                // parameter — the access reads an untyped slot. Recover the
                // operand kind from the proven access type:
                //
                // * Index access — `infer_expr_type` resolves `a[i]` for an
                //   array-tracked receiver via `tracked_array_element_type`.
                //   This REPLACES the former blanket `IndexAccess` → `Number`
                //   default, which stamped `Float64` on a statically-`int`
                //   element and silently turned `7 / 2 = 3` into `3.5`.
                //
                // * Property access (WS-9b) — `infer_expr_type` resolves
                //   `a.lo` via `tracker_schema_id_for_expr`: once inference
                //   widens the unannotated parameter to its callsite struct
                //   type (`Box`), the parameter's local slot carries the
                //   `Box` schema id, and the field type is read from the
                //   proven struct schema. The field kind is PROVEN from the
                //   schema, never fabricated — when it cannot be proven the
                //   hint stays `None` and the strict-arithmetic proof guard /
                //   `NoPlan` path raises a loud compile error (no `Number`
                //   default is introduced for property access).
                if is_strict_arithmetic(op) || is_ordered_comparison(op) {
                    let is_access = |e: &Expr| {
                        matches!(e, Expr::IndexAccess { .. } | Expr::PropertyAccess { .. })
                    };
                    if left_numeric.is_none() && is_access(left) {
                        left_numeric = self
                            .infer_expr_type(left)
                            .ok()
                            .and_then(|t| inferred_type_to_numeric(&t));
                    }
                    if right_numeric.is_none() && is_access(right) {
                        right_numeric = self
                            .infer_expr_type(right)
                            .ok()
                            .and_then(|t| inferred_type_to_numeric(&t));
                    }
                }

                if is_strict_arithmetic(op) || is_ordered_comparison(op) {
                    self.adopt_missing_numeric_operand_hint(
                        left,
                        right,
                        &mut left_numeric,
                        &mut right_numeric,
                    );
                }

                // ADR-006 §2.7.5 stamp-at-compile-time — int-literal width
                // inference. A bare integer literal adopts the width of its
                // sibling so the operation stays on the declared-width
                // carrier (`DivTyped`/`ModTyped` with the matching
                // `NumericWidth`) rather than widening to the signed default
                // `Int` (`DivInt`) — covering both `u64` (unsigned div/mod)
                // and the narrow signed/unsigned widths (truncating arith).
                Self::promote_int_literal_to_width_sibling(
                    left,
                    right,
                    &mut left_numeric,
                    &mut right_numeric,
                );

                // ── Schema-based type safety (catches objects in arithmetic) ──
                // If an operand has a schema (it's a TypedObject) but no numeric type,
                // it's an object being used in arithmetic → compile error.
                // Exception: if the left type implements an operator trait for this op.
                if is_strict_arithmetic(op) {
                    let left_is_object = left_schema.is_some() && left_numeric.is_none();
                    let right_is_object = right_schema.is_some() && right_numeric.is_none();
                    if left_is_object || right_is_object {
                        // Check if the left operand's type implements an operator trait
                        let has_operator_trait = left_schema
                            .and_then(|sid| self.type_tracker.schema_registry().get_by_id(sid))
                            .and_then(|schema| {
                                operator_trait_for_op(op).filter(|trait_name| {
                                    self.type_inference
                                        .env
                                        .type_implements_trait(&schema.name, trait_name)
                                })
                            })
                            .is_some();

                        if !has_operator_trait {
                            let op_symbol = match op {
                                BinaryOp::Sub => "-",
                                BinaryOp::Mul => "*",
                                BinaryOp::Div => "/",
                                BinaryOp::Mod => "%",
                                BinaryOp::Pow => "**",
                                _ => "?",
                            };
                            let left_desc = if left_is_object { "object" } else { "numeric" };
                            let right_desc = if right_is_object { "object" } else { "numeric" };
                            return Err(ShapeError::SemanticError {
                                message: format!(
                                    "Cannot apply '{}' to {} and {}. Both operands must be numeric (int, number, or decimal).",
                                    op_symbol, left_desc, right_desc,
                                ),
                                location: Some(
                                    self.span_to_source_location(combined_span(left, right)),
                                ),
                            });
                        }
                    }
                }

                // ── Phase 2.5: operator trait dispatch via CallMethod ──
                // If the left operand is a typed object whose schema implements
                // the operator trait (Sub/Mul/Div/Ord/...), emit a method call
                // instead of falling through to a generic arithmetic opcode. The
                // receiver and the right-hand-side operand are already on the stack.
                //
                // W1.8 (v0.3 R2): for ordered comparison ops (`<`, `<=`, `>`, `>=`)
                // the trait method is `Ord::cmp(other: Self) -> int`; lower the
                // returned int via `emit_cmp_result_comparison` to produce the
                // per-op boolean result. Mirrors the post-call step in
                // `try_emit_trait_dispatch` at L83.
                if let Some(trait_name) = operator_trait_for_op(op) {
                    let dispatches_via_trait = left_schema
                        .and_then(|sid| self.type_tracker.schema_registry().get_by_id(sid))
                        .is_some_and(|schema| {
                            self.type_inference
                                .env
                                .type_implements_trait(&schema.name, trait_name)
                        });
                    if dispatches_via_trait {
                        if let Some(method_name) = operator_trait_method_for_op(op) {
                            emit_operator_trait_call(self, method_name, op_span);
                            if is_ordered_comparison(op) {
                                emit_cmp_result_comparison(self, op);
                            } else if let Some(left_id) = left_schema {
                                // Arithmetic operator trait (`Sub`/`Mul`/...)
                                // result is `Self` — restore the left schema so
                                // chained / assigned uses resolve. (operators
                                // slice — compound-assign fix)
                                self.restore_operator_trait_result_schema(left_id);
                            }
                            return Ok(());
                        }
                    }
                }

                // ── ε-1 PART 1: emit-side soundness guard ──
                // A typed numeric opcode requires the compiler to PROVE both
                // operand kinds (CLAUDE.md §Mechanical enforcement). If an
                // operand's compile-time type is still an unresolved
                // `Type::Variable` the `NumericType` claim is fabricated —
                // surface a clean `ProofGap` diagnostic instead of stamping a
                // default kind and emitting a typed opcode (the silent-wrong
                // path that produced the `2e-321` denormal).
                if let Some(gap) = self.numeric_operand_proof_gap(op, left, left_numeric) {
                    // A-final ROOT-C: defer dead-template proof-gap (emit Pop).
                    if self.defer_template_numeric_binop() {
                        return Ok(());
                    }
                    return Err(gap);
                }
                if let Some(gap) = self.numeric_operand_proof_gap(op, right, right_numeric) {
                    if self.defer_template_numeric_binop() {
                        return Ok(());
                    }
                    return Err(gap);
                }

                // ── Emit typed opcode (with coercion for mixed Int/Number) ──
                let is_comparison = is_ordered_comparison(op);
                let emit_result = self.emit_numeric_binary_with_coercion_trusted(
                    op,
                    left_numeric,
                    right_numeric,
                    is_comparison,
                    left,
                    right,
                );
                match emit_result {
                    NumericEmitResult::EmittedTyped => {}
                    NumericEmitResult::CoercedNeedsGeneric => {
                        // Op has no typed variant for this type combination.
                        // Strict-typing sweep (Phase 1): the historical
                        // dynamic-opcode fallback is now a hard compile error.
                        if !try_emit_trait_dispatch(self, op, left_schema, left, op_span) {
                            // A-final ROOT-C: defer dead-template numeric binop.
                            if self.defer_template_numeric_binop() {
                                return Ok(());
                            }
                            return Err(strict_typing_binop_error(self, op, left, right));
                        }
                    }
                    NumericEmitResult::NoPlan => {
                        // Types unknown from slot tracking — try inference engine.
                        let (inferred_left, inferred_right) = self.infer_numeric_pair(left, right);
                        match self.emit_numeric_binary_with_coercion_trusted(
                            op,
                            inferred_left,
                            inferred_right,
                            is_comparison,
                            left,
                            right,
                        ) {
                            NumericEmitResult::EmittedTyped => {}
                            _ => {
                                // Strict-typing sweep (Phase 1): the historical
                                // dynamic-opcode fallback is now a hard compile error.
                                if !try_emit_trait_dispatch(self, op, left_schema, left, op_span) {
                                    // A-final ROOT-C: defer dead-template numeric binop.
                                    if self.defer_template_numeric_binop() {
                                        return Ok(());
                                    }
                                    return Err(strict_typing_binop_error(self, op, left, right));
                                }
                            }
                        }
                    }
                }
                self.last_expr_schema = None;
            }
        }
        Ok(())
    }

    /// Compile a fuzzy comparison expression with tolerance.
    /// Desugars to arithmetic operations — no dedicated fuzzy VM opcodes needed.
    pub(super) fn compile_expr_fuzzy_comparison(
        &mut self,
        left: &Expr,
        op: &FuzzyOp,
        right: &Expr,
        tolerance: &FuzzyTolerance,
    ) -> Result<()> {
        use crate::bytecode::{Constant, Operand};

        // Store left and right in temp locals to avoid re-evaluation
        let temp_a = self.declare_temp_local("__fuzzy_a")?;
        let temp_b = self.declare_temp_local("__fuzzy_b")?;

        self.compile_expr(left)?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(temp_a)),
        ));
        self.compile_expr(right)?;
        self.emit(Instruction::new(
            OpCode::StoreLocal,
            Some(Operand::Local(temp_b)),
        ));

        // Helper: emit abs(a - b) → load a, load b, SubNumber, Dup, push 0, LtNumber, JumpIfFalse(skip), NegNumber, skip:
        // This computes abs(top-of-stack) inline.  All fuzzy comparison operands are f64.
        let emit_abs_diff = |compiler: &mut BytecodeCompiler| {
            compiler.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(temp_a)),
            ));
            compiler.emit(Instruction::new(
                OpCode::LoadLocal,
                Some(Operand::Local(temp_b)),
            ));
            compiler.emit(Instruction::simple(OpCode::SubNumber));
            // abs: dup, push 0, LtNumber, JumpIfFalse(skip), NegNumber
            compiler.emit(Instruction::simple(OpCode::Dup));
            let zero_idx = compiler.program.add_constant(Constant::Number(0.0));
            compiler.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(zero_idx)),
            ));
            compiler.emit(Instruction::simple(OpCode::LtNumber));
            let skip = compiler.emit_jump(OpCode::JumpIfFalse, 0);
            compiler.emit(Instruction::simple(OpCode::NegNumber));
            compiler.patch_jump(skip);
        };

        match (op, tolerance) {
            (FuzzyOp::Equal, FuzzyTolerance::Absolute(tol)) => {
                // abs(a - b) <= tol
                emit_abs_diff(self);
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
            }
            (FuzzyOp::Equal, FuzzyTolerance::Percentage(tol)) => {
                // abs(a - b) / ((abs(a) + abs(b)) / 2) <= tol
                // numerator: abs(a - b)
                emit_abs_diff(self);
                // denominator: (abs(a) + abs(b)) / 2
                // abs(a)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let zero_idx2 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(zero_idx2)),
                ));
                self.emit(Instruction::simple(OpCode::LtNumber));
                let skip_a = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
                self.patch_jump(skip_a);
                // abs(b)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let zero_idx3 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(zero_idx3)),
                ));
                self.emit(Instruction::simple(OpCode::LtNumber));
                let skip_b = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
                self.patch_jump(skip_b);
                // (abs(a) + abs(b)) / 2
                self.emit(Instruction::simple(OpCode::AddNumber));
                let two_idx = self.program.add_constant(Constant::Number(2.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(two_idx)),
                ));
                self.emit(Instruction::simple(OpCode::DivNumber));
                // numerator / denominator <= tol
                self.emit(Instruction::simple(OpCode::DivNumber));
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
            }
            (FuzzyOp::Greater, FuzzyTolerance::Absolute(tol)) => {
                // a > b || abs(a - b) <= tol
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::GtNumber));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                emit_abs_diff(self);
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
                let end2 = self.emit_jump(OpCode::Jump, 0);
                self.patch_jump(end);
                self.emit_bool(true);
                self.patch_jump(end2);
            }
            (FuzzyOp::Greater, FuzzyTolerance::Percentage(tol)) => {
                // a > b || (percentage within tolerance)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::GtNumber));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                // Reuse percentage tolerance check
                emit_abs_diff(self);
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let z1 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(z1)),
                ));
                self.emit(Instruction::simple(OpCode::LtNumber));
                let sa = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
                self.patch_jump(sa);
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let z2 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(z2)),
                ));
                self.emit(Instruction::simple(OpCode::LtNumber));
                let sb = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
                self.patch_jump(sb);
                self.emit(Instruction::simple(OpCode::AddNumber));
                let two = self.program.add_constant(Constant::Number(2.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(two)),
                ));
                self.emit(Instruction::simple(OpCode::DivNumber));
                self.emit(Instruction::simple(OpCode::DivNumber));
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
                let end2 = self.emit_jump(OpCode::Jump, 0);
                self.patch_jump(end);
                self.emit_bool(true);
                self.patch_jump(end2);
            }
            (FuzzyOp::Less, FuzzyTolerance::Absolute(tol)) => {
                // a < b || abs(a - b) <= tol
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::LtNumber));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                emit_abs_diff(self);
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
                let end2 = self.emit_jump(OpCode::Jump, 0);
                self.patch_jump(end);
                self.emit_bool(true);
                self.patch_jump(end2);
            }
            (FuzzyOp::Less, FuzzyTolerance::Percentage(tol)) => {
                // a < b || (percentage within tolerance)
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::LtNumber));
                let end = self.emit_jump(OpCode::JumpIfTrue, 0);
                emit_abs_diff(self);
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_a)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let z1 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(z1)),
                ));
                self.emit(Instruction::simple(OpCode::LtNumber));
                let sa = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
                self.patch_jump(sa);
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(temp_b)),
                ));
                self.emit(Instruction::simple(OpCode::Dup));
                let z2 = self.program.add_constant(Constant::Number(0.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(z2)),
                ));
                self.emit(Instruction::simple(OpCode::LtNumber));
                let sb = self.emit_jump(OpCode::JumpIfFalse, 0);
                self.emit(Instruction::simple(OpCode::NegNumber));
                self.patch_jump(sb);
                self.emit(Instruction::simple(OpCode::AddNumber));
                let two = self.program.add_constant(Constant::Number(2.0));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(two)),
                ));
                self.emit(Instruction::simple(OpCode::DivNumber));
                self.emit(Instruction::simple(OpCode::DivNumber));
                let tol_idx = self.program.add_constant(Constant::Number(*tol));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(tol_idx)),
                ));
                self.emit(Instruction::simple(OpCode::LteNumber));
                let end2 = self.emit_jump(OpCode::Jump, 0);
                self.patch_jump(end);
                self.emit_bool(true);
                self.patch_jump(end2);
            }
        }

        Ok(())
    }

    /// Compile a typed object merge (a + b where both are TypedObjects).
    ///
    /// Object-literal merge OVERRIDES on duplicate keys (right wins): the
    /// merged field set is `left-fields-not-in-right ++ all-right-fields`,
    /// preserving left order then right order. A shared key (`{x,y} + {y,z}`
    /// shares `y`) appears ONCE, carrying the right operand's value and type.
    ///
    /// This registers that deduplicated merged schema at compile time under the
    /// `__intersection_{left}_{right}` name that the runtime
    /// `derive_merged_schema` looks up, then emits `MergeObject` — the
    /// deduplicating runtime merge (`build_named_merged_storage`, layout
    /// `keep_left ++ right`). The former `TypedMergeObject` path naively
    /// CONCATENATED both field lists, so `{x:1,y:2}+{y:20,z:30}` produced a
    /// schema with a DUPLICATE `y` (`{x, y, y, z}`) instead of the correct
    /// `{x:1, y:20, z:30}`. (operators slice — merge override fix)
    fn compile_typed_merge(&mut self, left_id: SchemaId, right_id: SchemaId) -> Result<()> {
        let registry = self.type_tracker.schema_registry();

        let left_schema = registry
            .get_by_id(left_id)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Unknown left schema ID: {}", left_id),
                location: None,
            })?;
        let right_schema =
            registry
                .get_by_id(right_id)
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!("Unknown right schema ID: {}", right_id),
                    location: None,
                })?;

        // Deduplicated merge layout (right wins on shared keys): match the
        // runtime `op_merge_object` / `build_named_merged_storage` order —
        // left fields whose name is NOT in the right schema (in left order),
        // then ALL right fields (in right order).
        let right_names: std::collections::HashSet<&str> =
            right_schema.fields.iter().map(|f| f.name.as_str()).collect();
        let mut merged_fields: Vec<(String, FieldType)> = Vec::new();
        for f in &left_schema.fields {
            if !right_names.contains(f.name.as_str()) {
                merged_fields.push((f.name.clone(), f.field_type.clone()));
            }
        }
        for f in &right_schema.fields {
            merged_fields.push((f.name.clone(), f.field_type.clone()));
        }

        // Register the merged schema under the name the runtime's
        // `derive_merged_schema` resolves for `MergeObject`.
        let merged_name = format!("__intersection_{}_{}", left_id, right_id);
        let target_id = self
            .type_tracker
            .schema_registry_mut()
            .register_type(merged_name.clone(), merged_fields);

        // Emit MergeObject (deduplicating runtime merge). It reads both operand
        // schemas at runtime and derives the merged schema by name.
        self.emit(Instruction::new(OpCode::MergeObject, None));

        // Track result schema for chained operations (e.g., a + b + c)
        self.last_expr_schema = Some(target_id);
        self.last_expr_type_info = Some(VariableTypeInfo::known(target_id, merged_name));

        Ok(())
    }
}

#[cfg(test)]
mod u64_literal_inference_tests {
    //! ADR-006 §2.7.5 stamp-at-compile-time — int-literal width inference.
    //!
    //! Regression coverage for `r5c-2-bg-b2-u64-literal-inference`: a bare
    //! integer literal that is a sibling operand of a width-typed binary op
    //! must adopt the sibling's width so the operation stays on the
    //! declared-width carrier — emitting `DivTyped`/`ModTyped`/... with the
    //! matching `NumericWidth` instead of the signed-default `DivInt`.
    use super::*;
    use shape_ast::IntWidth;

    // ── promote_int_literal_to_width_sibling unit coverage ──

    fn lit_int(v: i64) -> Expr {
        Expr::Literal(Literal::Int(v), Span::DUMMY)
    }
    fn lit_uint(v: u64) -> Expr {
        Expr::Literal(Literal::UInt(v), Span::DUMMY)
    }
    fn ident() -> Expr {
        Expr::Identifier("x".to_string(), Span::DUMMY)
    }

    #[test]
    fn u64_sibling_promotes_right_int_literal() {
        // `a / 2`: a:u64, 2:Int literal → 2 adopts IntWidth(U64).
        let mut l = Some(NumericType::IntWidth(IntWidth::U64));
        let mut r = Some(NumericType::Int);
        BytecodeCompiler::promote_int_literal_to_width_sibling(
            &ident(),
            &lit_int(2),
            &mut l,
            &mut r,
        );
        assert_eq!(l, Some(NumericType::IntWidth(IntWidth::U64)));
        assert_eq!(r, Some(NumericType::IntWidth(IntWidth::U64)));
    }

    #[test]
    fn u64_sibling_promotes_left_int_literal() {
        // `100 / a`: 100:Int literal, a:u64 → 100 adopts IntWidth(U64).
        let mut l = Some(NumericType::Int);
        let mut r = Some(NumericType::IntWidth(IntWidth::U64));
        BytecodeCompiler::promote_int_literal_to_width_sibling(
            &lit_int(100),
            &ident(),
            &mut l,
            &mut r,
        );
        assert_eq!(l, Some(NumericType::IntWidth(IntWidth::U64)));
        assert_eq!(r, Some(NumericType::IntWidth(IntWidth::U64)));
    }

    #[test]
    fn narrow_sibling_promotes_int_literal() {
        // `x + 28`: x:i8, 28:Int literal → 28 adopts IntWidth(I8).
        for w in [
            IntWidth::I8,
            IntWidth::I16,
            IntWidth::I32,
            IntWidth::U8,
            IntWidth::U16,
            IntWidth::U32,
        ] {
            let mut l = Some(NumericType::IntWidth(w));
            let mut r = Some(NumericType::Int);
            BytecodeCompiler::promote_int_literal_to_width_sibling(
                &ident(),
                &lit_int(28),
                &mut l,
                &mut r,
            );
            assert_eq!(
                r,
                Some(NumericType::IntWidth(w)),
                "literal must adopt {:?}",
                w
            );
        }
    }

    #[test]
    fn negative_literal_does_not_adopt_unsigned_width() {
        // `a + (-5)`: a:u64, -5:Int literal → -5 must NOT silently adopt u64.
        let mut l = Some(NumericType::IntWidth(IntWidth::U64));
        let mut r = Some(NumericType::Int);
        BytecodeCompiler::promote_int_literal_to_width_sibling(
            &ident(),
            &lit_int(-5),
            &mut l,
            &mut r,
        );
        assert_eq!(
            r,
            Some(NumericType::Int),
            "negative literal stays Int for u64 sibling"
        );
    }

    #[test]
    fn negative_literal_adopts_signed_width() {
        // `x + (-5)`: x:i8, -5:Int literal → -5 adopts i8 (signed, fits).
        let mut l = Some(NumericType::IntWidth(IntWidth::I8));
        let mut r = Some(NumericType::Int);
        BytecodeCompiler::promote_int_literal_to_width_sibling(
            &ident(),
            &lit_int(-5),
            &mut l,
            &mut r,
        );
        assert_eq!(r, Some(NumericType::IntWidth(IntWidth::I8)));
    }

    #[test]
    fn signed_typed_sibling_is_not_promoted() {
        // `a / b`: a:u64, b:Int (a genuine variable, not a literal) →
        // the `Int` operand is NOT a literal so it stays untouched.
        let mut l = Some(NumericType::IntWidth(IntWidth::U64));
        let mut r = Some(NumericType::Int);
        BytecodeCompiler::promote_int_literal_to_width_sibling(
            &ident(),
            &ident(), // RHS is an identifier, not a literal
            &mut l,
            &mut r,
        );
        assert_eq!(
            r,
            Some(NumericType::Int),
            "non-literal Int sibling stays Int"
        );
    }

    #[test]
    fn uint_literal_adopts_u64_sibling() {
        // `a / 18446744073709551615u64`-shaped literal classified as UInt.
        let mut l = Some(NumericType::IntWidth(IntWidth::U64));
        let mut r = Some(NumericType::Int);
        BytecodeCompiler::promote_int_literal_to_width_sibling(
            &ident(),
            &lit_uint(9_000_000_000_000_000_000),
            &mut l,
            &mut r,
        );
        assert_eq!(r, Some(NumericType::IntWidth(IntWidth::U64)));
    }

    // ── end-to-end opcode-emission coverage ──

    /// Compile a top-level program and return its instructions.
    fn compile_top_level(code: &str) -> Vec<Instruction> {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let compiler = super::super::super::BytecodeCompiler::new();
        let bc = compiler.compile(&program).expect("compile failed");
        bc.instructions
    }

    /// Returns `true` when the instruction stream contains `opcode` carrying
    /// `Operand::Width(width)`.
    fn has_width_typed(instrs: &[Instruction], opcode: OpCode, width: NumericWidth) -> bool {
        instrs.iter().any(|i| {
            i.opcode == opcode && matches!(i.operand, Some(Operand::Width(w)) if w == width)
        })
    }

    fn has_opcode(instrs: &[Instruction], opcode: OpCode) -> bool {
        instrs.iter().any(|i| i.opcode == opcode)
    }

    #[test]
    fn u64_var_div_literal_emits_div_typed_u64() {
        // `a / 2` on `a: u64` must emit `DivTyped` width U64 — the unsigned
        // carrier — NOT the signed `DivInt`.
        let instrs = compile_top_level("let a: u64 = 100\nlet b: u64 = a / 2\n");
        assert!(
            has_width_typed(&instrs, OpCode::DivTyped, NumericWidth::U64),
            "u64 / literal must emit DivTyped(U64): {:?}",
            instrs.iter().map(|i| i.opcode).collect::<Vec<_>>()
        );
        assert!(
            !has_opcode(&instrs, OpCode::DivInt),
            "u64 / literal must NOT emit signed DivInt"
        );
    }

    #[test]
    fn u64_var_mod_literal_emits_mod_typed_u64() {
        let instrs = compile_top_level("let a: u64 = 100\nlet b: u64 = a % 10\n");
        assert!(
            has_width_typed(&instrs, OpCode::ModTyped, NumericWidth::U64),
            "u64 % literal must emit ModTyped(U64)"
        );
        assert!(!has_opcode(&instrs, OpCode::ModInt));
    }

    #[test]
    fn u64_literal_on_left_emits_div_typed_u64() {
        // `100 / a` — literal on the LEFT.
        let instrs = compile_top_level("let a: u64 = 7\nlet b: u64 = 100 / a\n");
        assert!(
            has_width_typed(&instrs, OpCode::DivTyped, NumericWidth::U64),
            "literal / u64 must emit DivTyped(U64)"
        );
    }

    #[test]
    fn u64_var_add_literal_stays_u64_carrier() {
        // `a + 1` on `a: u64` must emit `AddTyped` width U64.
        let instrs = compile_top_level("let a: u64 = 100\nlet b: u64 = a + 1\n");
        assert!(
            has_width_typed(&instrs, OpCode::AddTyped, NumericWidth::U64),
            "u64 + literal must emit AddTyped(U64)"
        );
        assert!(!has_opcode(&instrs, OpCode::AddInt));
    }

    #[test]
    fn u64_var_sub_mul_literal_stay_u64_carrier() {
        let sub = compile_top_level("let a: u64 = 100\nlet b: u64 = a - 1\n");
        assert!(has_width_typed(&sub, OpCode::SubTyped, NumericWidth::U64));
        let mul = compile_top_level("let a: u64 = 100\nlet b: u64 = a * 2\n");
        assert!(has_width_typed(&mul, OpCode::MulTyped, NumericWidth::U64));
    }

    #[test]
    fn narrow_var_add_literal_stays_narrow_carrier() {
        // `x + 28` on `x: i8` must emit `AddTyped` width I8 — the truncating
        // narrow carrier — NOT the signed-default `AddInt`.
        let instrs = compile_top_level("let x: i8 = 100\nlet y: i8 = x + 28\n");
        assert!(
            has_width_typed(&instrs, OpCode::AddTyped, NumericWidth::I8),
            "i8 + literal must emit AddTyped(I8): {:?}",
            instrs.iter().map(|i| i.opcode).collect::<Vec<_>>()
        );
        assert!(!has_opcode(&instrs, OpCode::AddInt));
    }

    #[test]
    fn narrow_u32_div_literal_stays_narrow_carrier() {
        let instrs = compile_top_level("let x: u32 = 4000000000\nlet y: u32 = x / 4\n");
        assert!(
            has_width_typed(&instrs, OpCode::DivTyped, NumericWidth::U32),
            "u32 / literal must emit DivTyped(U32)"
        );
        assert!(!has_opcode(&instrs, OpCode::DivInt));
    }

    #[test]
    fn plain_int_var_div_literal_still_emits_div_int() {
        // Guard: the default `int` (i64) path is unchanged — `n / 2` on
        // `n: int` still emits the signed `DivInt`, not `DivTyped`.
        let instrs = compile_top_level("let n: int = 100\nlet m: int = n / 2\n");
        assert!(
            has_opcode(&instrs, OpCode::DivInt),
            "int / literal must still emit DivInt"
        );
    }

    // ── end-to-end execution coverage (unsigned semantics) ──

    /// Compile + execute a top-level program and return the raw u64 bits of
    /// the final expression.
    fn run_top_level(code: &str) -> u64 {
        use crate::VMConfig;
        use crate::executor::VirtualMachine;
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        let compiler = super::super::super::BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile failed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        vm.execute_raw(None).expect("execution failed")
    }

    #[test]
    fn u64_max_div_literal_computes_unsigned() {
        // u64::MAX / 2 == 9223372036854775807 (unsigned). A signed reinterpret
        // computes (-1) / 2 == 0.
        let bits = run_top_level("let a: u64 = 18446744073709551615\na / 2\n");
        assert_eq!(bits, 9_223_372_036_854_775_807);
    }

    #[test]
    fn u64_max_mod_literal_computes_unsigned() {
        // u64::MAX % 10 == 5 (unsigned). Signed would give -1.
        let bits = run_top_level("let a: u64 = 18446744073709551615\na % 10\n");
        assert_eq!(bits, 5);
    }

    #[test]
    fn u64_literal_left_div_var_computes_unsigned() {
        // 100 / u64::MAX == 0 (unsigned). A signed reinterpret of u64::MAX as
        // -1 would compute 100 / -1 == -100.
        let bits = run_top_level("let a: u64 = 18446744073709551615\n100 / a\n");
        assert_eq!(bits, 0);
    }

    #[test]
    fn u64_add_sub_mul_literal_wrap_at_2_pow_64() {
        // a + 1 wraps u64::MAX → 0.
        assert_eq!(
            run_top_level("let a: u64 = 18446744073709551615\na + 1\n"),
            0
        );
        // a - 1 on u64::MAX → u64::MAX - 1.
        assert_eq!(
            run_top_level("let a: u64 = 18446744073709551615\na - 1\n"),
            18_446_744_073_709_551_614
        );
        // (2^63) * 2 wraps mod 2^64 → 0.
        assert_eq!(
            run_top_level("let m: u64 = 9223372036854775808\nm * 2\n"),
            0
        );
    }
}

#[cfg(test)]
mod ws3_f3_error_context_tests {
    //! WS-3 F3: the `!!` error-context operator compiles.
    //!
    //! Before this fix `compile_expr_binary_op` had no
    //! `BinaryOp::ErrorContext` arm, so `!!` fell into the generic
    //! arithmetic arm whose strict-operand-type gate rejected the
    //! `Result<…>` left operand — a core operator could not be compiled
    //! at all. The opcode (`OpCode::ErrorContext`) and the runtime
    //! handler (`op_error_context`) already existed; only the
    //! compiler-dispatch arm was missing.
    use crate::compiler::BytecodeCompiler;
    use shape_ast::parser::parse_program;

    #[test]
    fn ws3_f3_error_context_operator_compiles() {
        let code = r#"
            fn boom() -> Result<int, string> { Err("bad") }
            fn main() -> Result<int, string> {
                let v = boom() !! "ctx"
                print(v)
                Ok(0)
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "`!!` error-context operator must compile: {:?}",
            result.err()
        );
    }

    #[test]
    fn ws3_f3_error_context_unwrapped_type_propagates_to_binding() {
        // `!!` yields the UNWRAPPED success value `T` (`Ok(v) => v`), so
        // a downstream `v + 1` must type-check as `int + int`.
        let code = r#"
            fn good() -> Result<int, string> { Ok(99) }
            fn main() -> Result<int, string> {
                let v = good() !! "ctx"
                let w = v + 1
                print(w)
                Ok(0)
            }
        "#;
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "`!!`-unwrapped value must keep its type for a downstream binop: {:?}",
            result.err()
        );
    }
}

#[cfg(test)]
mod ws9c_anonymous_object_factory_tests {
    //! WS-9c: an unannotated function returning an object literal built from
    //! its parameters — `fn aabb(lo, hi) { {min: lo, max: hi} }` — is an
    //! anonymous-object factory. Before this fix the object literal froze its
    //! field types to `unknown` (no `TypeAnnotation` variable variant), so a
    //! later `a.min + b.max` over the factory result was spuriously rejected
    //! as `unknown + unknown`. The fix keeps the field-value parameters as
    //! `tyvar` markers through inference, publishes `apply_callsite_unions`'
    //! `resolved` fixpoint into the unifier, and registers an inline schema
    //! for the inferred return so the compiler resolves `.field` access.
    use crate::compiler::BytecodeCompiler;
    use shape_ast::parser::parse_program;

    fn compiles(code: &str) -> bool {
        let program = parse_program(code).expect("Failed to parse");
        BytecodeCompiler::new().compile(&program).is_ok()
    }

    #[test]
    fn ws9c_factory_result_field_binop_compiles() {
        // The headline repro: a binop over two factory results' fields.
        assert!(compiles(
            r#"
            fn aabb(lo, hi) { {min: lo, max: hi} }
            let a = aabb(1, 5)
            let b = aabb(2, 6)
            print(a.min + b.max)
            "#
        ));
    }

    #[test]
    fn ws9c_factory_result_through_unannotated_param_compiles() {
        // The factory result threaded into a second unannotated function.
        assert!(compiles(
            r#"
            fn aabb(lo, hi) { {min: lo, max: hi} }
            fn area(box) { box.max - box.min }
            print(area(aabb(1, 5)))
            "#
        ));
    }

    #[test]
    fn ws9c_factory_result_direct_field_access_compiles() {
        // `f(...).field` directly, with no intervening `let` binding.
        assert!(compiles(
            r#"
            fn aabb(lo, hi) { {min: lo, max: hi} }
            print(aabb(1, 5).min + 1)
            "#
        ));
    }

    #[test]
    fn ws9c_factory_result_array_literal_compiles() {
        // An array literal of factory results must compile — the element
        // type resolves to the factory's anonymous-object return type rather
        // than cascading an `unknown` element into `op_new_array`.
        assert!(compiles(
            r#"
            fn aabb(lo, hi) { {min: lo, max: hi} }
            let xs = [aabb(1, 5), aabb(2, 6)]
            print(xs)
            "#
        ));
    }
}

#[cfg(test)]
mod r1_r4_reference_type_tests {
    //! R1 grammar (`&T` / `&mut T` in type position) + R4 reference-operand
    //! auto-deref.
    //!
    //! R1: `fn f(x: &int) -> int` and `-> &int` must PARSE (were E0001
    //! parse errors before the `reference_type` grammar alternative +
    //! `TypeAnnotation::Borrow` cascade landed).
    //!
    //! R4 (reconciled F4, 2026-06-18): a reference-TYPED binop operand
    //! (`fn use_ref(x: &int) -> int { x + 1 }`) AUTO-DEREFS — it reads the
    //! referent value THROUGH the reference and computes correctly, mirroring
    //! the already-auto-derefing first-class reference-BOUND path
    //! (`let r = &n; r + 1`, see `borrow_refs/operator_deref.rs`) and method
    //! dispatch (`r.len()`). This is the behavior the book documents:
    //! `fundamentals/references-borrowing.mdx` "Returning a value through a
    //! reference" — `fn read_val(&x) { return x }` "returns the dereferenced
    //! value, not the reference"; `advanced/ownership-deep-dive.mdx`
    //! "First-Class References" shows `let val = r + 1` "reads through r via
    //! DerefLoad", never `*r + 1` (explicit `*r` does not even parse).
    //!
    //! The earlier stage of this test asserted the OPPOSITE (a Rust-shaped
    //! compile-REJECT requiring explicit `*x`). That assertion was already
    //! FAILING on baseline (its guard never fired) and contradicted the
    //! shipped auto-deref behavior + the book; it is rebaselined here to the
    //! shipped behavior. The hard binder is *no SIGSEGV / no corruption* —
    //! satisfied because the operand resolves to the referent scalar type and
    //! emits the typed numeric opcode (VM==JIT==6), not a raw-pointer read.
    use crate::compiler::BytecodeCompiler;
    use shape_ast::parser::parse_program;

    #[test]
    fn r1_reference_param_and_return_type_parse() {
        // Was E0001 (parse error). After R1 the `&int` param and `&int`
        // return both parse — `parse_program` returns `Ok`.
        let code = r#"
            fn f(x: &int) -> int { 5 }
            fn g() -> &int { let a = 3; &a }
            fn main() { print(0) }
        "#;
        let parsed = parse_program(code);
        assert!(
            parsed.is_ok(),
            "`&int` in param and return position must PARSE (R1): {:?}",
            parsed.err()
        );
    }

    #[test]
    fn r1_reference_mut_type_parses() {
        let code = r#"
            fn h(x: &mut int) -> int { 7 }
            fn main() { print(0) }
        "#;
        let parsed = parse_program(code);
        assert!(
            parsed.is_ok(),
            "`&mut int` in param position must PARSE (R1): {:?}",
            parsed.err()
        );
    }

    #[test]
    fn r4_reference_typed_operand_auto_derefs() {
        // `x` has reference type `&int`; `x + 1` AUTO-DEREFS — the operand
        // resolves to the referent scalar `int`, emits the typed numeric
        // opcode, and COMPILES cleanly. This mirrors the reference-BOUND path
        // (`let r = &n; r + 1`) and method dispatch (`r.len()`), and matches
        // the book (`fn read_val(&x) { return x }` returns the dereferenced
        // value; `let val = r + 1` "reads through r via DerefLoad"). No
        // explicit `*x` is required (and `*x` does not parse). The earlier
        // Rust-shaped compile-REJECT assertion was stale (already failing on
        // baseline) and is rebaselined to the shipped auto-deref behavior.
        let code = r#"
            fn use_ref(x: &int) -> int { x + 1 }
            fn main() { print(0) }
        "#;
        let program = parse_program(code).expect("R1: `&int` param must parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_ok(),
            "a reference-typed binop operand must auto-deref and compile (R4): {:?}",
            result.err()
        );
    }
}

#[cfg(test)]
mod s4_type_erasure_dispatch_tests {
    //! S4: the type-erasure dispatch tail surfacing in collection / trait /
    //! null-coalesce contexts.
    //!
    //! - D1: `Array<int>.sum()/.min()/.max()` return `int` (element type),
    //!   `Array<number>` returns `number`; mixed int/number programs no longer
    //!   collide via the shared `_oob` placeholder var.
    //! - traits: an `extend`/`impl Trait` method's DECLARED return type
    //!   propagates to an un-annotated call-site binding (no silent
    //!   `int → number` float corruption).
    //! - D4: `a ?? b` preserves the right operand's numeric kind, so a
    //!   `let h = m.get(k) ?? 0` binding stays `int` inside a loop body (no
    //!   runtime "no method 'add' on receiver kind Int64").
    //! - functions: a free-function call with named arguments is a clean
    //!   compile error (was silent-discard → wrong result).
    use crate::compiler::BytecodeCompiler;
    use crate::test_utils::{eval_typed_bool, eval_typed_i64};
    use shape_ast::parser::parse_program;

    #[test]
    fn s4_d1_array_int_sum_min_max_return_int() {
        assert_eq!(eval_typed_i64("let s: int = [1, 2, 3].sum()\ns"), 6);
        assert_eq!(eval_typed_i64("let v = [4, 1, 9]\nlet m: int = v.min()\nm"), 1);
        assert_eq!(eval_typed_i64("let v = [4, 1, 9]\nlet m: int = v.max()\nm"), 9);
    }

    #[test]
    fn s4_d1_mixed_int_and_number_sum_do_not_collide() {
        // Both call sites hit the receiver-element projection; the `_oob`
        // placeholder must be freshened per call so `int` and `number`
        // results don't unify.
        let code = "let s: int = [1, 2, 3].sum()\nlet fs: number = [1.0, 2.0].sum()\ns";
        assert_eq!(eval_typed_i64(code), 6);
    }

    #[test]
    fn s4_traits_extend_method_int_return_propagates_to_binding() {
        // `p.tot()` declared `-> int`; an un-annotated `let a = p.tot()`
        // must track `int` so `a + a` emits `AddInt` → 28 (not 28.0).
        let code = "type P { x: int, y: int }\n\
                    extend P { method tot() -> int { self.x + self.y } }\n\
                    let p = P { x: 6, y: 8 }\n\
                    let a = p.tot()\n\
                    a + a";
        assert_eq!(eval_typed_i64(code), 28);
    }

    #[test]
    fn s4_d4_null_coalesce_preserves_int_kind_in_loop() {
        // `m.get(k) ?? 0` must keep `int` across loop iterations.
        let code = "let mut m: HashMap<string,int> = HashMap()\n\
                    m.set(\"x\", 3)\n\
                    let mut acc = 0\n\
                    for i in [0, 1] {\n\
                      let h = m.get(\"x\") ?? 0\n\
                      acc = acc + h\n\
                    }\n\
                    acc";
        assert_eq!(eval_typed_i64(code), 6);
    }

    #[test]
    fn s4_functions_named_args_on_free_fn_are_compile_error() {
        let code = "fn bv(w: int = 1, h: int = 1, d: int = 1) -> int { return w * h * d }\n\
                    print(bv(w: 2, h: 3, d: 4))";
        let program = parse_program(code).expect("Failed to parse");
        let result = BytecodeCompiler::new().compile(&program);
        assert!(
            result.is_err(),
            "named call arguments on a free function must be a clean compile error"
        );
    }

    #[test]
    fn s4_functions_positional_args_still_work() {
        let code = "fn bv(w: int = 1, h: int = 1, d: int = 1) -> int { return w * h * d }\nbv(2, 3, 4)";
        assert_eq!(eval_typed_i64(code), 24);
    }
}

#[cfg(test)]
mod operator_trait_dispatch_completeness_tests {
    //! Operators slice — operator-trait dispatch COMPLETENESS (4 findings):
    //!  1. MERGE-HIJACK: an untyped object-literal merge must NOT be hijacked
    //!     by a structurally-matching in-scope `impl Add`; the merge builtin
    //!     wins for untyped object literals and OVERRIDES on shared keys.
    //!  2. operator-trait dispatch works for `+=` / compound-assign on a `mut`
    //!     user-type receiver, and for chained `acc = acc + x`.
    //!  3. an inline struct-LITERAL as the LEFT operand of `Sub`/`Mul` resolves
    //!     the user impl (like the variable form, and like Add).
    //!  4. operator-trait resolution is declaration-ORDER INDEPENDENT (an impl
    //!     declared AFTER a fn that uses it still resolves).
    use crate::compiler::BytecodeCompiler;
    use crate::test_utils::eval_typed_i64;
    use shape_ast::parser::parse_program;

    fn compiles(code: &str) -> bool {
        let program = parse_program(code).expect("Failed to parse");
        BytecodeCompiler::new().compile(&program).is_ok()
    }

    const MONEY_ADD: &str = "type Money { cents: int }\n\
        impl Add for Money {\n\
          method add(other: Money) -> Money { Money { cents: self.cents + other.cents } }\n\
        }\n";

    // ── Finding 1: object-literal merge is NOT hijacked by impl Add ──

    #[test]
    fn merge_not_hijacked_by_structural_impl_add_overrides_shared_key() {
        // `{x:1,y:2} + {y:20,z:30}` must FIELD-MERGE (right wins on `y`, `z`
        // kept) — NOT be hijacked into a structurally-matching `impl Add for
        // Vec2 {x,y}` (which would positionally add and drop `z`).
        let code = "type Vec2 { x: int, y: int }\n\
            impl Add for Vec2 {\n\
              method add(other: Vec2) -> Vec2 { Vec2 { x: self.x + other.x, y: self.y + other.y } }\n\
            }\n\
            let m = {x:1, y:2} + {y:20, z:30}\n\
            m.x + m.y * 100 + m.z * 10000";
        // x=1, y=20, z=30  → 1 + 2000 + 300000 = 302001
        assert_eq!(eval_typed_i64(code), 302001);
    }

    #[test]
    fn plain_object_merge_overrides_shared_key_right_wins() {
        // No impl in scope: the merge builtin still overrides (no duplicate
        // key). `{a:1,b:2} + {b:9,c:3}` → b=9.
        let code = "let m = {a:1, b:2} + {b:9, c:3}\n\
                    m.a + m.b * 10 + m.c * 100";
        // a=1, b=9, c=3 → 1 + 90 + 300 = 391
        assert_eq!(eval_typed_i64(code), 391);
    }

    // ── Finding 2: compound-assign + mut receiver on a user operator type ──

    #[test]
    fn compound_assign_on_mut_user_type_compiles() {
        let code = format!(
            "{MONEY_ADD}\
             let a = Money {{ cents: 5 }}\n\
             let mut acc = Money {{ cents: 0 }}\n\
             acc += a\n\
             acc.cents"
        );
        assert_eq!(eval_typed_i64(&code), 5);
    }

    #[test]
    fn chained_mut_assign_then_add_keeps_user_type() {
        // `acc += a` (desugars to `acc = acc + a`) must restore `acc`'s schema
        // so the NEXT `acc = acc + b` still resolves the operator trait.
        let code = format!(
            "{MONEY_ADD}\
             let a = Money {{ cents: 5 }}\n\
             let b = Money {{ cents: 7 }}\n\
             let mut acc = Money {{ cents: 0 }}\n\
             acc += a\n\
             acc = acc + b\n\
             acc.cents"
        );
        assert_eq!(eval_typed_i64(&code), 12);
    }

    #[test]
    fn immutable_let_user_add_still_works() {
        let code = format!(
            "{MONEY_ADD}\
             let a = Money {{ cents: 5 }}\n\
             let b = Money {{ cents: 7 }}\n\
             let c = a + b\n\
             c.cents"
        );
        assert_eq!(eval_typed_i64(&code), 12);
    }

    // ── Finding 3: inline struct-literal LEFT operand of Sub / Mul ──

    #[test]
    fn inline_struct_literal_left_of_sub_resolves_impl() {
        let code = "type Money { cents: int }\n\
            impl Sub for Money {\n\
              method sub(other: Money) -> Money { Money { cents: self.cents - other.cents } }\n\
            }\n\
            let v = Money { cents: 3 }\n\
            let r = Money { cents: 10 } - v\n\
            r.cents";
        assert_eq!(eval_typed_i64(code), 7);
    }

    #[test]
    fn inline_struct_literal_both_operands_of_sub_resolves_impl() {
        let code = "type Money { cents: int }\n\
            impl Sub for Money {\n\
              method sub(other: Money) -> Money { Money { cents: self.cents - other.cents } }\n\
            }\n\
            let r = Money { cents: 10 } - Money { cents: 3 }\n\
            r.cents";
        assert_eq!(eval_typed_i64(code), 7);
    }

    #[test]
    fn inline_struct_literal_left_of_mul_resolves_impl() {
        let code = "type Money { cents: int }\n\
            impl Mul for Money {\n\
              method mul(other: Money) -> Money { Money { cents: self.cents * other.cents } }\n\
            }\n\
            let v = Money { cents: 3 }\n\
            let r = Money { cents: 10 } * v\n\
            r.cents";
        assert_eq!(eval_typed_i64(code), 30);
    }

    // ── Finding 4: declaration-order independence ──

    #[test]
    fn operator_trait_impl_declared_after_use_site_resolves() {
        // `fn double` uses `m + m` and is declared BEFORE `impl Add for Money`.
        let code = "type Money { cents: int }\n\
            fn double(m: Money) -> Money { m + m }\n\
            impl Add for Money {\n\
              method add(other: Money) -> Money { Money { cents: self.cents + other.cents } }\n\
            }\n\
            let r = double(Money { cents: 4 })\n\
            r.cents";
        assert_eq!(eval_typed_i64(code), 8);
    }

    // ── Negative: a struct WITHOUT an Add impl in `+` is still rejected ──

    #[test]
    fn struct_without_add_impl_in_plus_is_compile_error() {
        let code = "type P { a: int }\n\
            let x = P { a: 1 }\n\
            let y = P { a: 2 }\n\
            let z = x + y\n\
            z.a";
        assert!(
            !compiles(code),
            "`+` on a struct with no `impl Add` must be a compile error"
        );
    }
}
