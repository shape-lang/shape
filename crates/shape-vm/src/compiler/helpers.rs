//! Helper methods for bytecode compilation

use super::BorrowMode;
use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::type_tracking::{NumericType, StorageHint, TypeTracker, VariableTypeInfo};
use shape_ast::ast::{Spanned, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};
use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use super::{
    BuiltinNameResolution, BytecodeCompiler, DropKind, ParamPassMode, ResolutionScope,
};

/// Phase V1.1D: default-on ownership-aware local opcodes.
///
/// When `true`, the compiler emits the ownership-aware `MoveLocal` /
/// `CloneLocal` / `DropLocal` opcodes (V1.1A/B) for heap-ref (`UniqueHeap`
/// / Direct-owned) bindings. When `false`, emission is byte-identical to
/// pre-V1.1C: `LoadLocal` / `LoadLocalMove` / `LoadLocalClone` continue
/// unchanged; no `MoveLocal` / `CloneLocal` / `DropLocal` are produced.
///
/// V1.1C landed with the default `false` (opt-in via
/// `SHAPE_V2_OWNERSHIP_MOVES=1`). V1.1D flips the default to `true` after
/// fixing three emission bugs the opt-in soak surfaced:
///
///   * Function compilation did not save/restore `ownership_drop_locals`
///     around the callee scope, leaking per-function `DropLocal` entries
///     into the caller's main-level drop pass. This corrupted the result
///     of any heap-returning callee (DateTime arithmetic, comptime body
///     replacement). Fixed in `compiler/functions.rs`.
///   * `CloneLocal` was emitted for `let mut` slots that had been
///     `BoxLocal`-wrapped into a `SharedCell` by a subsequent closure
///     capture. `clone_raw_bits` bumps the cell's Arc without unwrapping,
///     so arithmetic then saw `shared_cell` instead of the inner scalar
///     ("Cannot apply '+' to int and shared_cell"). Gated by
///     `slot_is_boxed` in `emit_load_local_owned`.
///   * Symmetric bug for `DropLocal`: a boxed slot received both the
///     V1.1C `DropLocal` *and* the legacy `DropCall` pass, poisoning the
///     SharedCell before the legacy unwrap could read it.
///     `binding_slot_needs_ownership_drop` and the drop-scope emission
///     sites now skip boxed slots so the Arc-refcount release path owns
///     the release alone.
///
/// Polarity inversion: the env var is now an opt-OUT. Values
/// `0` / `false` / `off` / `no` (case-insensitive, trimmed) disable the
/// flag; unset, empty, or any other value keeps the V1.1D default of
/// `true`. This mirrors the V0.a `SHAPE_V2_VAR_SHAREDCOW` pattern (see
/// `crates/shape-vm/src/mir/storage_planning.rs:50`).
///
/// Rollback: `SHAPE_V2_OWNERSHIP_MOVES=0` restores the pre-V1.1D
/// byte-identical emission. A single-commit revert is also sufficient.
///
/// For unit-test determinism — the `OnceLock` cache freezes whichever env
/// state the test binary starts with, and multiple tests racing
/// `std::env::set_var` would poison the cache — a `#[cfg(test)]`
/// thread-local override (`with_ownership_moves_flag`) lets a single
/// test temporarily force the flag on/off without touching the env.
pub(super) fn ownership_moves_enabled() -> bool {
    #[cfg(test)]
    {
        if let Some(v) = TEST_OWNERSHIP_MOVES_OVERRIDE.with(|cell| cell.get()) {
            return v;
        }
    }
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| match std::env::var("SHAPE_V2_OWNERSHIP_MOVES") {
        Ok(v) => !matches!(
            v.trim(),
            "0" | "false" | "FALSE" | "False"
                | "off" | "OFF" | "Off"
                | "no" | "NO" | "No"
        ),
        Err(_) => true,
    })
}

#[cfg(test)]
thread_local! {
    /// Phase V1.1C test hook: per-thread override for
    /// `ownership_moves_enabled()`. When `Some(b)`, the flag reads as `b`
    /// regardless of env-var state. The override is scoped to a single
    /// closure via `with_ownership_moves_flag` and is cleared on drop —
    /// tests running on the same thread cannot leak flag state into each
    /// other. Thread-local rather than a global mutex so concurrent
    /// `cargo test` workers stay independent.
    pub(super) static TEST_OWNERSHIP_MOVES_OVERRIDE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// Phase V1.1C test helper: run `f` with the ownership-moves flag
/// forced to `enabled`. Restores the previous override on return (even
/// on panic). This is the compile-time-gated test path; production code
/// reads the env var exclusively via `ownership_moves_enabled()`.
#[cfg(test)]
pub(crate) fn with_ownership_moves_flag<R>(enabled: bool, f: impl FnOnce() -> R) -> R {
    struct Guard(Option<bool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            TEST_OWNERSHIP_MOVES_OVERRIDE.with(|cell| cell.set(self.0));
        }
    }
    let prev = TEST_OWNERSHIP_MOVES_OVERRIDE.with(|cell| cell.replace(Some(enabled)));
    let _guard = Guard(prev);
    f()
}

/// Phase V1.2C/D: default-on compiler emission of `PromoteToShared`.
///
/// When `true`, the compiler emits `PromoteToShared` (V1.2A/B) at escape
/// points — sites where a uniquely-owned (Box-backed) value transitions to
/// shared (Arc-backed) ownership:
///
///   * Site A: a `let` / `const` binding classified as `UniqueHeap` is
///     captured by an *escaping* closure (`emit_make_closure_heap_next`
///     was set by the caller, e.g. a return-of-closure or
///     store-into-collection pattern). The value is pushed for the
///     closure env; `PromoteToShared` converts it to Arc so the closure
///     can outlive the owning scope safely.
///
///   * Site B: a `var`-like assignment target with `SharedCow` storage
///     is written from an rhs that was just produced by `PromoteToOwned`
///     (Box-backed). The target's Arc-shared representation needs the
///     value in Arc form; `PromoteToShared` performs the Box→Arc
///     transfer without a refcount bump.
///
/// Site C (passing owned to an Arc-expecting parameter) is deferred to
/// V1.3: it requires `FunctionBorrowSummary.param_ownership_hints` which
/// does not exist at V1.2 time.
///
/// V1.2C ships the emission gated on this flag; V1.2D flips the default
/// to `true`. Polarity matches the V1.1D convention: the env var is an
/// opt-OUT. Values `0` / `false` / `off` / `no` (case-insensitive,
/// trimmed) disable the flag; unset, empty, or any other value keeps the
/// V1.2D default of `true`. Mirrors the V0.a `SHAPE_V2_VAR_SHAREDCOW`
/// pattern (`crates/shape-vm/src/mir/storage_planning.rs:50`) and the
/// V1.1D `SHAPE_V2_OWNERSHIP_MOVES` pattern above.
///
/// Rollback: `SHAPE_V2_PROMOTE_TO_SHARED=0` restores the pre-V1.2C
/// byte-identical emission at both sites.
///
/// For unit-test determinism, a `#[cfg(test)]` thread-local override
/// (`with_promote_to_shared_flag`) lets a single test force the flag
/// on/off without touching the env-var cache.
pub(super) fn promote_to_shared_enabled() -> bool {
    #[cfg(test)]
    {
        if let Some(v) = TEST_PROMOTE_TO_SHARED_OVERRIDE.with(|cell| cell.get()) {
            return v;
        }
    }
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| match std::env::var("SHAPE_V2_PROMOTE_TO_SHARED") {
        Ok(v) => !matches!(
            v.trim(),
            "0" | "false" | "FALSE" | "False"
                | "off" | "OFF" | "Off"
                | "no" | "NO" | "No"
        ),
        Err(_) => true,
    })
}

#[cfg(test)]
thread_local! {
    /// Phase V1.2C test hook: per-thread override for
    /// `promote_to_shared_enabled()`. Thread-local so concurrent
    /// `cargo test` workers remain independent.
    pub(super) static TEST_PROMOTE_TO_SHARED_OVERRIDE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// Phase V1.2C test helper: run `f` with the PromoteToShared flag forced
/// to `enabled`. Restores the previous override on return (even on
/// panic). Production code reads the env var exclusively via
/// `promote_to_shared_enabled()`.
#[cfg(test)]
pub(crate) fn with_promote_to_shared_flag<R>(enabled: bool, f: impl FnOnce() -> R) -> R {
    struct Guard(Option<bool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            TEST_PROMOTE_TO_SHARED_OVERRIDE.with(|cell| cell.set(self.0));
        }
    }
    let prev = TEST_PROMOTE_TO_SHARED_OVERRIDE.with(|cell| cell.replace(Some(enabled)));
    let _guard = Guard(prev);
    f()
}

/// Phase V1.3: default-on Box-by-default allocation for `UniqueHeap` locals.
///
/// When `true`, the compiler extends the Phase 3/4 `PromoteToOwned` emission
/// (which pre-V1.3 only fired for `BindingStorageClass::Direct` heap-typed
/// `let`/`const`) to also fire for `BindingStorageClass::UniqueHeap` slots.
/// Under the V1.2D baseline `UniqueHeap` bindings (a `let`/`const` value
/// captured by a mutating closure; see `storage_planning.rs:935` rule 2)
/// landed in their slot as freshly-allocated `Arc<HeapValue>` with refcount
/// 1 — a wasted atomic per allocation since the binding is uniquely owned
/// by construction. V1.3 converts these to `Box<HeapValue>` via
/// `PromoteToOwned` so the non-escape case pays zero atomic ops.
///
/// Escape safety: V1.2's `PromoteToShared` emission (default on) already
/// covers the two escape vectors — Site A (capture into an escaping
/// closure) and Site B (assignment into a SharedCow-backed `var`). The
/// V1.3 switch therefore operates behind the escape boundary: values start
/// as Box and promote to Arc at the escape point if needed.
///
/// Mechanism: V1.3 does not add any new opcodes. It extends the existing
/// `PromoteToOwned` (0x107) emission in statements.rs by broadening the
/// storage-class predicate. When the flag is off, the predicate keeps its
/// V1.2D shape (`Direct` only) and bytecode is byte-identical to pre-V1.3.
///
/// Polarity matches V1.1D / V1.2D: the env var is an opt-OUT. Values
/// `0` / `false` / `off` / `no` (case-insensitive, trimmed) disable the
/// flag; unset, empty, or any other value keeps the V1.3 default of
/// `true`. Mirrors `SHAPE_V2_VAR_SHAREDCOW` (V0.a),
/// `SHAPE_V2_OWNERSHIP_MOVES` (V1.1D), `SHAPE_V2_PROMOTE_TO_SHARED`
/// (V1.2D).
///
/// Rollback: `SHAPE_V2_BOX_BY_DEFAULT=0` restores the pre-V1.3
/// byte-identical emission. A single-commit revert also suffices.
///
/// For unit-test determinism, a `#[cfg(test)]` thread-local override
/// (`with_box_by_default_flag`) lets a single test force the flag on/off
/// without touching the env-var cache.
pub(super) fn box_by_default_enabled() -> bool {
    #[cfg(test)]
    {
        if let Some(v) = TEST_BOX_BY_DEFAULT_OVERRIDE.with(|cell| cell.get()) {
            return v;
        }
    }
    static CACHED: OnceLock<bool> = OnceLock::new();
    *CACHED.get_or_init(|| match std::env::var("SHAPE_V2_BOX_BY_DEFAULT") {
        Ok(v) => !matches!(
            v.trim(),
            "0" | "false" | "FALSE" | "False"
                | "off" | "OFF" | "Off"
                | "no" | "NO" | "No"
        ),
        Err(_) => true,
    })
}

#[cfg(test)]
thread_local! {
    /// Phase V1.3 test hook: per-thread override for
    /// `box_by_default_enabled()`. Thread-local so concurrent
    /// `cargo test` workers remain independent.
    pub(super) static TEST_BOX_BY_DEFAULT_OVERRIDE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// Phase V1.3 test helper: run `f` with the Box-by-default flag forced to
/// `enabled`. Restores the previous override on return (even on panic).
/// Production code reads the env var exclusively via
/// `box_by_default_enabled()`.
#[cfg(test)]
pub(crate) fn with_box_by_default_flag<R>(enabled: bool, f: impl FnOnce() -> R) -> R {
    struct Guard(Option<bool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            TEST_BOX_BY_DEFAULT_OVERRIDE.with(|cell| cell.set(self.0));
        }
    }
    let prev = TEST_BOX_BY_DEFAULT_OVERRIDE.with(|cell| cell.replace(Some(enabled)));
    let _guard = Guard(prev);
    f()
}

/// Emit a runtime-dispatched addition instruction.
///
/// This is the fallback path for `+` when the compiler cannot prove both
/// operand types at compile time (e.g. untyped locals, mixed
/// string/numeric contexts, DateTime arithmetic). The VM's
/// `exec_arithmetic` handles type dispatch at runtime.
///
/// Typed callers should prefer `AddInt`, `AddNumber`, `AddDecimal`,
/// `StringConcat`, or `ArrayConcat` when the operand types are proven.
pub(super) fn emit_dynamic_add(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::AddDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched equality instruction.
///
/// This is the fallback path for `==` when the compiler cannot prove both
/// operand types at compile time (e.g. untyped function params, generic
/// stdlib code). The VM's `exec_comparison` handles type dispatch at runtime
/// via `vw_equals`.
///
/// Typed callers should prefer `EqInt`, `EqNumber`, `EqDecimal`, or
/// `EqString` when the operand types are proven.
pub(super) fn emit_dynamic_eq(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::EqDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched not-equal instruction.
///
/// This is the fallback path for `!=` when the compiler cannot prove both
/// operand types at compile time. The VM's `exec_comparison` handles type
/// dispatch at runtime via `vw_equals`.
///
/// Typed callers should prefer `NeqInt`, `NeqNumber`, or typed equality +
/// `Not` when the operand types are proven.
pub(super) fn emit_dynamic_neq(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::NeqDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched subtraction instruction.
///
/// Fallback for `-` when operand types are not proven at compile time.
/// The VM's `exec_arithmetic` handles type dispatch at runtime.
///
/// Typed callers should prefer `SubInt`, `SubNumber`, `SubDecimal`
/// when the operand types are proven.
pub(in crate::compiler) fn emit_dynamic_sub(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::SubDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched multiplication instruction.
///
/// Fallback for `*` when operand types are not proven at compile time.
/// The VM's `exec_arithmetic` handles type dispatch at runtime.
///
/// Typed callers should prefer `MulInt`, `MulNumber`, `MulDecimal`
/// when the operand types are proven.
pub(in crate::compiler) fn emit_dynamic_mul(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::MulDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched division instruction.
///
/// Fallback for `/` when operand types are not proven at compile time.
/// The VM's `exec_arithmetic` handles type dispatch at runtime.
///
/// Typed callers should prefer `DivInt`, `DivNumber`, `DivDecimal`
/// when the operand types are proven.
pub(in crate::compiler) fn emit_dynamic_div(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::DivDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched modulo instruction.
///
/// Fallback for `%` when operand types are not proven at compile time.
/// The VM's `exec_arithmetic` handles type dispatch at runtime.
///
/// Typed callers should prefer `ModInt`, `ModNumber`
/// when the operand types are proven.
pub(in crate::compiler) fn emit_dynamic_mod(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::ModDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched power/exponentiation instruction.
///
/// Fallback for `**` when operand types are not proven at compile time.
/// The VM's `exec_arithmetic` handles type dispatch at runtime.
///
/// Typed callers should prefer `PowInt`, `PowNumber`
/// when the operand types are proven.
pub(in crate::compiler) fn emit_dynamic_pow(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::PowDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched greater-than instruction.
///
/// Fallback for `>` when operand types are not proven at compile time.
/// The VM's `exec_comparison` handles type dispatch at runtime.
///
/// Typed callers should prefer `GtInt`, `GtNumber`, `GtDecimal`
/// when the operand types are proven.
pub(in crate::compiler) fn emit_dynamic_gt(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::GtDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched less-than instruction.
///
/// Fallback for `<` when operand types are not proven at compile time.
/// The VM's `exec_comparison` handles type dispatch at runtime.
///
/// Typed callers should prefer `LtInt`, `LtNumber`, `LtDecimal`
/// when the operand types are proven.
pub(in crate::compiler) fn emit_dynamic_lt(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::LtDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched greater-than-or-equal instruction.
///
/// Fallback for `>=` when operand types are not proven at compile time.
/// The VM's `exec_comparison` handles type dispatch at runtime.
///
/// Typed callers should prefer `GteInt`, `GteNumber`, `GteDecimal`
/// when the operand types are proven.
pub(in crate::compiler) fn emit_dynamic_gte(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::GteDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Emit a runtime-dispatched less-than-or-equal instruction.
///
/// Fallback for `<=` when operand types are not proven at compile time.
/// The VM's `exec_comparison` handles type dispatch at runtime.
///
/// Typed callers should prefer `LteInt`, `LteNumber`, `LteDecimal`
/// when the operand types are proven.
pub(in crate::compiler) fn emit_dynamic_lte(compiler: &mut BytecodeCompiler) {
    compiler.emit(Instruction::simple(OpCode::LteDynamic));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
}

/// Phase V3.1: typed-vs-dynamic binary-op dispatch kind.
///
/// Generalizes `NumericType` with a few non-numeric categories the binary-op
/// emission path cares about (string, bool) plus an `Unknown` sentinel so
/// callers can thread through `Option<NumericType>` or richer inference
/// results. V3.1 only wires in the Numeric and String cases today — Bool is
/// reserved for the future when an `EqBool` typed opcode lands (no such
/// opcode exists in V3, so bool equality falls back to `EqDynamic`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler) enum BinOperandKind {
    /// Resolved to a specific numeric type (Int, Number, Decimal, IntWidth).
    Numeric(NumericType),
    /// Resolved to `string` or `char` — triggers `StringConcatTyped` for `+`.
    String,
    /// Resolved to `bool`. V3.1: no typed bool-eq opcode; always falls back
    /// to the Dynamic opcode family for now. Reserved for future typed bool
    /// opcodes without a caller migration.
    Bool,
    /// Type is not known at compile time — emit the Dynamic fallback.
    Unknown,
}

impl BinOperandKind {
    /// Build a `BinOperandKind` from an `Option<NumericType>` — the most
    /// common shape used by existing binary-op emission sites.
    pub(in crate::compiler) fn from_numeric(nt: Option<NumericType>) -> Self {
        match nt {
            Some(n) => BinOperandKind::Numeric(n),
            None => BinOperandKind::Unknown,
        }
    }
}

/// Phase V3.1 shim: pick the typed opcode for a (BinaryOp, NumericType,
/// NumericType) triple when both operands resolve to the same numeric
/// category, else return `None` (caller falls back to the Dynamic opcode).
///
/// Mirrors the numeric-only subset of
/// `compiler::expressions::numeric_ops::typed_opcode_for` so that
/// `emit_binary_op` can live in the helpers module without a module-visibility
/// dance. The two tables stay in lockstep; the expressions-side helper
/// additionally handles `IntWidth` coercions which V3.1 intentionally does
/// NOT generalize (width handling stays in `emit_numeric_binary_with_coercion`
/// until V3.2+ tranches migrate those callers).
fn typed_numeric_opcode(op: shape_ast::ast::BinaryOp, nt: NumericType) -> Option<OpCode> {
    use shape_ast::ast::BinaryOp;
    // V3.1 shim: handle only scalar Int/Number/Decimal paths. IntWidth is
    // left to the existing width-aware emission path in the expressions
    // module — migrating those sites is V3.3+ work.
    let matched = match (op, nt) {
        // Arithmetic — Int
        (BinaryOp::Add, NumericType::Int) => OpCode::AddInt,
        (BinaryOp::Sub, NumericType::Int) => OpCode::SubInt,
        (BinaryOp::Mul, NumericType::Int) => OpCode::MulInt,
        (BinaryOp::Div, NumericType::Int) => OpCode::DivInt,
        (BinaryOp::Mod, NumericType::Int) => OpCode::ModInt,
        (BinaryOp::Pow, NumericType::Int) => OpCode::PowInt,
        // Arithmetic — Number (f64)
        (BinaryOp::Add, NumericType::Number) => OpCode::AddNumber,
        (BinaryOp::Sub, NumericType::Number) => OpCode::SubNumber,
        (BinaryOp::Mul, NumericType::Number) => OpCode::MulNumber,
        (BinaryOp::Div, NumericType::Number) => OpCode::DivNumber,
        (BinaryOp::Mod, NumericType::Number) => OpCode::ModNumber,
        (BinaryOp::Pow, NumericType::Number) => OpCode::PowNumber,
        // Arithmetic — Decimal (no ModDecimal/PowDecimal opcodes)
        (BinaryOp::Add, NumericType::Decimal) => OpCode::AddDecimal,
        (BinaryOp::Sub, NumericType::Decimal) => OpCode::SubDecimal,
        (BinaryOp::Mul, NumericType::Decimal) => OpCode::MulDecimal,
        (BinaryOp::Div, NumericType::Decimal) => OpCode::DivDecimal,
        // Comparison — Int
        (BinaryOp::Greater, NumericType::Int) => OpCode::GtInt,
        (BinaryOp::Less, NumericType::Int) => OpCode::LtInt,
        (BinaryOp::GreaterEq, NumericType::Int) => OpCode::GteInt,
        (BinaryOp::LessEq, NumericType::Int) => OpCode::LteInt,
        (BinaryOp::Equal, NumericType::Int) => OpCode::EqInt,
        (BinaryOp::NotEqual, NumericType::Int) => OpCode::NeqInt,
        // Comparison — Number
        (BinaryOp::Greater, NumericType::Number) => OpCode::GtNumber,
        (BinaryOp::Less, NumericType::Number) => OpCode::LtNumber,
        (BinaryOp::GreaterEq, NumericType::Number) => OpCode::GteNumber,
        (BinaryOp::LessEq, NumericType::Number) => OpCode::LteNumber,
        (BinaryOp::Equal, NumericType::Number) => OpCode::EqNumber,
        (BinaryOp::NotEqual, NumericType::Number) => OpCode::NeqNumber,
        // Comparison — Decimal (no Gt/Lt/Neq typed opcodes; fall back)
        (BinaryOp::Equal, NumericType::Decimal) => OpCode::EqDecimal,
        _ => return None,
    };
    Some(matched)
}

/// Map a `BinaryOp` to its `Dynamic`-family opcode. Returns `None` for ops
/// that are NOT arithmetic/comparison (And/Or/BitOps/NullCoalesce/…) —
/// those follow a separate code path in `compile_expr_binary_op`.
fn dynamic_opcode_for(op: shape_ast::ast::BinaryOp) -> Option<OpCode> {
    use shape_ast::ast::BinaryOp;
    Some(match op {
        BinaryOp::Add => OpCode::AddDynamic,
        BinaryOp::Sub => OpCode::SubDynamic,
        BinaryOp::Mul => OpCode::MulDynamic,
        BinaryOp::Div => OpCode::DivDynamic,
        BinaryOp::Mod => OpCode::ModDynamic,
        BinaryOp::Pow => OpCode::PowDynamic,
        BinaryOp::Greater => OpCode::GtDynamic,
        BinaryOp::Less => OpCode::LtDynamic,
        BinaryOp::GreaterEq => OpCode::GteDynamic,
        BinaryOp::LessEq => OpCode::LteDynamic,
        BinaryOp::Equal => OpCode::EqDynamic,
        BinaryOp::NotEqual => OpCode::NeqDynamic,
        _ => return None,
    })
}

/// Phase V3.1: unified typed-vs-dynamic binary-op emission shim.
///
/// Given a `BinaryOp` plus inferred operand kinds, emits the best-matching
/// bytecode opcode:
///
///   * Both operands `Numeric(nt)` with matching `nt` AND a typed opcode
///     exists for `(op, nt)` → emit the typed opcode (`AddInt`, `MulNumber`,
///     `EqDecimal`, …).
///   * Both operands `String` AND `op == Add` → emit `StringConcatTyped`
///     (matches the existing string-concat short-circuit in `compile_expr_binary_op`).
///   * Otherwise (mismatched kinds, `Unknown`, `Bool`, or no typed opcode) →
///     emit the `Dynamic`-family opcode (`AddDynamic`, `EqDynamic`, …).
///
/// Returns:
///   * `Ok(true)` when an opcode was emitted (either typed or dynamic).
///   * `Ok(false)` when `op` is NOT one this shim handles (And/Or/BitOps/
///     NullCoalesce/ErrorContext/Pipe/Fuzzy*) — the caller is expected to
///     route those ops through their dedicated emission paths.
///
/// This is a V3.1 **shim only** — no callers migrate yet. V3.2 (arithmetic),
/// V3.3 (comparisons), V3.4 (pattern-match `Eq`), and V3.5 (leftover sites)
/// will mechanically switch the ~48 existing emission sites over to this
/// helper. Until then, existing inline dispatch in
/// `compiler/expressions/binary_ops.rs` and friends continues to produce
/// identical bytecode.
///
/// Side effects: on emit (typed or dynamic), resets
/// `last_expr_schema` / `last_expr_type_info` / `last_expr_numeric_type`
/// to match the legacy `emit_dynamic_*` helpers. Typed-numeric emissions
/// additionally restore `last_expr_numeric_type = Some(nt)` for arithmetic
/// so downstream numeric-propagation logic keeps working. Comparisons always
/// clear the numeric hint because the result is a `bool`.
#[allow(dead_code)] // V3.1 helper — consumers land in V3.2+ tranches.
pub(in crate::compiler) fn emit_binary_op(
    compiler: &mut BytecodeCompiler,
    op: shape_ast::ast::BinaryOp,
    lhs: BinOperandKind,
    rhs: BinOperandKind,
) -> Result<bool> {
    use shape_ast::ast::BinaryOp;

    // Non-arithmetic / non-comparison ops are not the shim's responsibility.
    // And/Or short-circuit, bitwise ops emit dedicated opcodes, NullCoalesce
    // has its own jump sequence, etc.
    let Some(dyn_opcode) = dynamic_opcode_for(op) else {
        return Ok(false);
    };

    // Priority 1: both operands are proven same numeric type — emit typed.
    if let (BinOperandKind::Numeric(lnt), BinOperandKind::Numeric(rnt)) = (lhs, rhs) {
        if lnt == rnt {
            if let Some(typed) = typed_numeric_opcode(op, lnt) {
                compiler.emit(Instruction::simple(typed));
                compiler.last_expr_schema = None;
                compiler.last_expr_type_info = None;
                // Arithmetic preserves the numeric kind; comparison collapses to bool.
                compiler.last_expr_numeric_type = match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Pow => Some(lnt),
                    _ => None,
                };
                return Ok(true);
            }
        }
    }

    // Priority 2: both operands are String and op is Add — emit StringConcatTyped.
    // Matches the short-circuit already wired into `compile_expr_binary_op`
    // for the proven-strings case.
    if matches!((lhs, rhs, op), (BinOperandKind::String, BinOperandKind::String, BinaryOp::Add)) {
        compiler.emit(Instruction::simple(OpCode::StringConcatTyped));
        compiler.last_expr_schema = None;
        compiler.last_expr_type_info = None;
        compiler.last_expr_numeric_type = None;
        return Ok(true);
    }

    // Fallback: emit the Dynamic-family opcode. Mirrors the per-op
    // `emit_dynamic_*` helpers so we preserve their state-reset discipline.
    compiler.emit(Instruction::simple(dyn_opcode));
    compiler.last_expr_schema = None;
    compiler.last_expr_type_info = None;
    compiler.last_expr_numeric_type = None;
    Ok(true)
}

/// Extract the core error message from a ShapeError, stripping redundant
/// "Type error:", "Runtime error:", "Compile error:", etc. prefixes that
/// thiserror's Display impl adds.  This prevents nested comptime errors
/// from accumulating multiple prefixes like
/// "Runtime error: Comptime block evaluation failed: Runtime error: …".
pub(crate) fn strip_error_prefix(e: &ShapeError) -> String {
    let msg = e.to_string();
    // Known prefixes added by thiserror Display
    const PREFIXES: &[&str] = &[
        "Runtime error: ",
        "Type error: ",
        "Semantic error: ",
        "Parse error: ",
        "VM error: ",
        "Lexical error: ",
    ];
    let mut s = msg.as_str();
    // Strip at most 3 layers of prefix to handle deep nesting
    for _ in 0..3 {
        let mut stripped = false;
        for prefix in PREFIXES {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest;
                stripped = true;
                break;
            }
        }
        // Also strip the comptime wrapping messages themselves
        const COMPTIME_PREFIXES: &[&str] = &[
            "Comptime block evaluation failed: ",
            "Comptime handler execution failed: ",
            "Comptime block directive processing failed: ",
        ];
        for prefix in COMPTIME_PREFIXES {
            if let Some(rest) = s.strip_prefix(prefix) {
                s = rest;
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    s.to_string()
}



impl BytecodeCompiler {
    /// Resolve a ConcreteType type_tag from compiler state for the receiver.
    /// Returns 0xFF when the type cannot be determined.
    pub(crate) fn resolve_type_tag(
        numeric_type: Option<crate::type_tracking::NumericType>,
        type_info: &Option<crate::type_tracking::VariableTypeInfo>,
    ) -> u8 {
        use crate::type_tracking::NumericType;
        // Priority 1: numeric type (most precise)
        if let Some(nt) = numeric_type {
            return match nt {
                NumericType::Number => 0,   // F64
                NumericType::Int => 1,      // I64
                NumericType::IntWidth(_) => 1, // I64 (treat all int widths as I64 for dispatch)
                NumericType::Decimal => 22, // Decimal
            };
        }
        // Priority 2: type_info type_name
        if let Some(info) = type_info {
            if let Some(ref name) = info.type_name {
                return match name.as_str() {
                    "number" | "Number" => 0,    // F64
                    "int" | "Int" => 1,          // I64
                    "bool" | "Bool" => 9,        // Bool
                    "string" | "String" => 10,   // String
                    "DateTime" => 25,            // DateTime
                    _ => {
                        // Check for collection types
                        if name.starts_with("Array") || name.starts_with("Vec") {
                            12 // Array
                        } else if name.starts_with("HashMap") || name.starts_with("Map") {
                            13 // HashMap
                        } else if name.starts_with("Set") {
                            14 // Set (mapped to Option tag — we'll refine)
                        } else {
                            0xFF
                        }
                    }
                };
            }
            // Priority 3: kind-based inference
            use crate::type_tracking::VariableKind;
            match &info.kind {
                VariableKind::Table { .. } => return 11, // Struct-like (DataTable dispatch)
                _ => {}
            }
        }
        0xFF // Unknown
    }

    fn scalar_type_name_from_numeric(numeric_type: NumericType) -> &'static str {
        match numeric_type {
            NumericType::Int | NumericType::IntWidth(_) => "int",
            NumericType::Number => "number",
            NumericType::Decimal => "decimal",
        }
    }

    fn array_type_name_from_numeric(numeric_type: NumericType) -> &'static str {
        match numeric_type {
            NumericType::Int | NumericType::IntWidth(_) => "Vec<int>",
            NumericType::Number => "Vec<number>",
            NumericType::Decimal => "Vec<decimal>",
        }
    }

    fn is_array_type_name(type_name: Option<&str>) -> bool {
        matches!(type_name, Some(name) if name.starts_with("Vec<") && name.ends_with('>'))
    }

    /// Convert a source annotation to a tracked type name when we have a
    /// canonical runtime representation for it.
    pub(super) fn tracked_type_name_from_annotation(type_ann: &TypeAnnotation) -> Option<String> {
        match type_ann {
            TypeAnnotation::Basic(name) => Some(name.clone()),
            TypeAnnotation::Reference(name) => Some(name.to_string()),
            TypeAnnotation::Array(inner) => Some(format!("Vec<{}>", inner.to_type_string())),
            // Keep the canonical Vec<T> naming even if a Generic slips through.
            TypeAnnotation::Generic { name, args } if name == "Vec" && args.len() == 1 => {
                Some(format!("Vec<{}>", args[0].to_type_string()))
            }
            TypeAnnotation::Generic { name, args } if name == "Mat" && args.len() == 1 => {
                Some(format!("Mat<{}>", args[0].to_type_string()))
            }
            // Track Option/Result wrapper types so conversion lifting can
            // detect them (even though generic args are lost in the tracker).
            TypeAnnotation::Generic { name, .. }
                if name == "Option" || name == "Result" =>
            {
                Some(name.to_lowercase())
            }
            _ => None,
        }
    }

    /// Resolve a type name through the module scope stack and imports.
    ///
    /// If the name is already directly known (in struct_types, type_aliases, etc.),
    /// returns it as-is. Otherwise, tries prefixing with each module scope from
    /// innermost to outermost, then checks imported names to find a match.
    pub(super) fn resolve_type_name(&self, name: &str) -> String {
        // Already qualified or directly found
        if name.contains("::") || self.is_type_known_direct(name) {
            return name.to_string();
        }
        // Try module scope prefixes (innermost to outermost)
        for scope in self.module_scope_stack.iter().rev() {
            let qualified = format!("{}::{}", scope, name);
            if self.is_type_known_direct(&qualified) {
                return qualified;
            }
        }
        // Check imported names (from `from ... use { Name }` imports)
        if let Some(imported) = self.imported_names.get(name) {
            // When module_path is set (graph-compiled dependency), prefer
            // module-qualified name. This prevents accidental binding to an
            // unrelated local/bare type of the same name.
            if !imported.module_path.is_empty() {
                let qualified = format!("{}::{}", imported.module_path, imported.original_name);
                if self.is_type_known_direct(&qualified) {
                    return qualified;
                }
            }
            // Fall back to bare original name (legacy imports without module_path)
            if self.is_type_known_direct(&imported.original_name) {
                return imported.original_name.clone();
            }
        }
        // Try namespace module prefixes (from `use module` imports)
        for ns in &self.module_namespace_bindings {
            let qualified = format!("{}::{}", ns, name);
            if self.is_type_known_direct(&qualified) {
                return qualified;
            }
            // Try canonical path for graph-compiled modules
            if let Some(canonical) = self.graph_namespace_map.get(ns) {
                let cq = format!("{}::{}", canonical, name);
                if self.is_type_known_direct(&cq) {
                    return cq;
                }
            }
        }
        // Return as-is (may be a forward reference or builtin)
        name.to_string()
    }

    /// Direct type lookup without scope resolution
    fn is_type_known_direct(&self, name: &str) -> bool {
        self.struct_types.contains_key(name)
            || self.type_aliases.contains_key(name)
            || self
                .type_inference
                .env
                .lookup_type_alias(name)
                .is_some()
            || self.type_inference.env.get_enum(name).is_some()
            || self
                .type_inference
                .env
                .lookup_interface(name)
                .is_some()
            || self.type_inference.env.lookup_trait(name).is_some()
            || self.type_tracker.schema_registry().get(name).is_some()
    }

    /// Resolve a trait name to its canonical form for definition lookup.
    ///
    /// Returns `(canonical_name, basename)` where `canonical_name` is used for
    /// `trait_defs` lookup and `basename` is used for dispatch registration
    /// (runtime dispatch keys are always bare basenames).
    pub(super) fn resolve_trait_name(&self, name: &str) -> (String, String) {
        let basename = name.rsplit("::").next().unwrap_or(name).to_string();
        // Check trait_defs in priority order
        if self.trait_defs.contains_key(name) {
            return (name.to_string(), basename);
        }
        for scope in self.module_scope_stack.iter().rev() {
            let q = format!("{}::{}", scope, name);
            if self.trait_defs.contains_key(&q) {
                return (q, basename);
            }
        }
        if let Some(imported) = self.imported_names.get(name) {
            if !imported.module_path.is_empty() {
                let q = format!("{}::{}", imported.module_path, imported.original_name);
                if self.trait_defs.contains_key(&q) {
                    return (q, basename);
                }
            }
        }
        for ns in &self.module_namespace_bindings {
            let q = format!("{}::{}", ns, name);
            if self.trait_defs.contains_key(&q) {
                return (q, basename);
            }
            if let Some(canonical) = self.graph_namespace_map.get(ns) {
                let cq = format!("{}::{}", canonical, name);
                if self.trait_defs.contains_key(&cq) {
                    return (cq, basename);
                }
            }
        }
        // Fall back to type_inference.env for built-in traits (Into, From, etc.)
        // registered bare in mod.rs but not in trait_defs.
        if self.type_inference.env.lookup_trait(name).is_some() {
            return (name.to_string(), basename);
        }
        if self.type_inference.env.lookup_trait(&basename).is_some() {
            return (basename.clone(), basename);
        }
        (name.to_string(), basename)
    }

    /// Mark a local/module binding slot as an array with numeric element type.
    ///
    /// Used by `x = x.push(value)` in-place mutation lowering so subsequent
    /// indexed reads can recover numeric hints.
    pub(super) fn mark_slot_as_numeric_array(
        &mut self,
        slot: u16,
        is_local: bool,
        numeric_type: NumericType,
    ) {
        let info =
            VariableTypeInfo::named(Self::array_type_name_from_numeric(numeric_type).to_string());
        if is_local {
            self.type_tracker.set_local_type(slot, info);
        } else {
            self.type_tracker.set_binding_type(slot, info);
        }
    }

    /// Mark a local/module binding slot as a scalar numeric type.
    pub(super) fn mark_slot_as_numeric_scalar(
        &mut self,
        slot: u16,
        is_local: bool,
        numeric_type: NumericType,
    ) {
        let info =
            VariableTypeInfo::named(Self::scalar_type_name_from_numeric(numeric_type).to_string());
        if is_local {
            self.type_tracker.set_local_type(slot, info);
        } else {
            self.type_tracker.set_binding_type(slot, info);
        }
    }

    /// Seed numeric hints from expression usage in arithmetic contexts.
    ///
    /// - `x` in numeric arithmetic becomes scalar numeric (`int`/`number`/`decimal`).
    /// - `arr[i]` implies `arr` is `Vec<numeric>`.
    pub(super) fn seed_numeric_hint_from_expr(
        &mut self,
        expr: &shape_ast::ast::Expr,
        numeric_type: NumericType,
    ) {
        match expr {
            shape_ast::ast::Expr::Identifier(name, _) => {
                if let Some(local_idx) = self.resolve_local(name) {
                    self.mark_slot_as_numeric_scalar(local_idx, true, numeric_type);
                    return;
                }
                let scoped_name = self
                    .resolve_scoped_module_binding_name(name)
                    .unwrap_or_else(|| name.to_string());
                if let Some(binding_idx) = self.module_bindings.get(&scoped_name).copied() {
                    self.mark_slot_as_numeric_scalar(binding_idx, false, numeric_type);
                }
            }
            shape_ast::ast::Expr::IndexAccess {
                object,
                end_index: None,
                ..
            } => {
                if let shape_ast::ast::Expr::Identifier(name, _) = object.as_ref() {
                    if let Some(local_idx) = self.resolve_local(name) {
                        self.mark_slot_as_numeric_array(local_idx, true, numeric_type);
                        return;
                    }
                    let scoped_name = self
                        .resolve_scoped_module_binding_name(name)
                        .unwrap_or_else(|| name.to_string());
                    if let Some(binding_idx) = self.module_bindings.get(&scoped_name).copied() {
                        self.mark_slot_as_numeric_array(binding_idx, false, numeric_type);
                    }
                }
            }
            _ => {}
        }
    }

    fn recover_or_bail_with_null_placeholder(&mut self, err: ShapeError) -> Result<()> {
        if self.should_recover_compile_diagnostics() {
            self.errors.push(err);
            self.emit(Instruction::simple(OpCode::PushNull));
            Ok(())
        } else {
            Err(err)
        }
    }

    pub(super) fn compile_expr_as_value_or_placeholder(
        &mut self,
        expr: &shape_ast::ast::Expr,
    ) -> Result<()> {
        match self.compile_expr(expr) {
            Ok(()) => Ok(()),
            Err(err) => self.recover_or_bail_with_null_placeholder(err),
        }
    }

    /// Emit an instruction and return its index
    /// Also records the current source line and file in debug info
    pub(super) fn emit(&mut self, instruction: Instruction) -> usize {
        let idx = self.program.emit(instruction);
        // Record line number and file for this instruction
        if self.current_line > 0 {
            self.program.debug_info.line_numbers.push((
                idx,
                self.current_file_id,
                self.current_line,
            ));
        }
        idx
    }

    /// Emit a boolean constant
    pub(super) fn emit_bool(&mut self, value: bool) {
        let const_idx = self.program.add_constant(Constant::Bool(value));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
    }

    /// Emit a unit constant
    pub(super) fn emit_unit(&mut self) {
        let const_idx = self.program.add_constant(Constant::Unit);
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(const_idx)),
        ));
    }

    /// Emit a jump instruction with placeholder offset.
    ///
    /// When `opcode` is `JumpIfFalse` and the immediately preceding instruction
    /// is a typed or trusted comparison (produces a known bool), upgrades to
    /// `JumpIfFalseTrusted` which skips `is_truthy()` dispatch.
    pub(super) fn emit_jump(&mut self, mut opcode: OpCode, dummy: i32) -> usize {
        if opcode == OpCode::JumpIfFalse && self.last_instruction_produces_bool() {
            opcode = OpCode::JumpIfFalseTrusted;
        }
        self.emit(Instruction::new(opcode, Some(Operand::Offset(dummy))))
    }

    /// Returns true if the last emitted instruction always produces a boolean result.
    fn last_instruction_produces_bool(&self) -> bool {
        self.program
            .instructions
            .last()
            .map(|instr| {
                matches!(
                    instr.opcode,
                    OpCode::GtInt
                        | OpCode::GtNumber
                        | OpCode::GtDecimal
                        | OpCode::LtInt
                        | OpCode::LtNumber
                        | OpCode::LtDecimal
                        | OpCode::GteInt
                        | OpCode::GteNumber
                        | OpCode::GteDecimal
                        | OpCode::LteInt
                        | OpCode::LteNumber
                        | OpCode::LteDecimal
                        | OpCode::EqInt
                        | OpCode::EqNumber
                        | OpCode::NeqInt
                        | OpCode::NeqNumber
                        | OpCode::EqString
                        | OpCode::EqDecimal
                        | OpCode::IsNull
                        | OpCode::GtDynamic
                        | OpCode::LtDynamic
                        | OpCode::GteDynamic
                        | OpCode::LteDynamic
                        | OpCode::EqDynamic
                        | OpCode::NeqDynamic
                        | OpCode::Not
                )
            })
            .unwrap_or(false)
    }

    /// Patch a jump instruction with the correct offset
    pub(super) fn patch_jump(&mut self, jump_idx: usize) {
        let offset = self.program.current_offset() as i32 - jump_idx as i32 - 1;
        self.program.instructions[jump_idx] = Instruction::new(
            self.program.instructions[jump_idx].opcode,
            Some(Operand::Offset(offset)),
        );
    }

    /// Compile function call arguments, enabling `&` reference expressions.
    ///
    /// Each call's arguments get their own borrow region so that borrows from
    /// `&` references are released after the call returns. This matches Rust's
    /// semantics: temporary borrows from function arguments don't persist beyond
    /// the call. Sequential calls like `inc(&a); inc(&a)` are correctly allowed.
    pub(super) fn compile_call_args(
        &mut self,
        args: &[shape_ast::ast::Expr],
        expected_param_modes: Option<&[ParamPassMode]>,
    ) -> Result<Vec<(u16, u16)>> {
        self.call_arg_module_binding_ref_writebacks.push(Vec::new());

        let mut first_error: Option<ShapeError> = None;
        for (idx, arg) in args.iter().enumerate() {
            let pass_mode = expected_param_modes
                .and_then(|modes| modes.get(idx).copied())
                .unwrap_or(ParamPassMode::ByValue);

            let arg_result = match pass_mode {
                ParamPassMode::ByRefExclusive | ParamPassMode::ByRefShared => {
                    let borrow_mode = if pass_mode.is_exclusive() {
                        BorrowMode::Exclusive
                    } else {
                        BorrowMode::Shared
                    };
                    if let shape_ast::ast::Expr::Reference { expr, span, .. } = arg {
                        self.compile_reference_expr(expr, *span, borrow_mode)
                            .map(|_| ())
                    } else {
                        self.compile_implicit_reference_arg(arg, borrow_mode)
                    }
                }
                ParamPassMode::ByValue => {
                    if let shape_ast::ast::Expr::Reference { span, .. } = arg {
                        let message = if expected_param_modes.is_some() {
                            "[B0004] unexpected `&` argument: target parameter is not a reference parameter".to_string()
                        } else {
                            "[B0004] cannot pass `&` to a callable value without a declared reference contract; \
                             call a named function with known parameter modes or add an explicit callable type"
                                .to_string()
                        };
                        Err(ShapeError::SemanticError {
                            message,
                            location: Some(self.span_to_source_location(*span)),
                        })
                    } else {
                        self.plan_flexible_binding_escape_from_expr(arg);
                        self.compile_expr(arg)
                    }
                }
            };

            if let Err(err) = arg_result {
                if self.should_recover_compile_diagnostics() {
                    self.errors.push(err);
                    // Keep stack arity consistent for downstream call codegen.
                    self.emit(Instruction::simple(OpCode::PushNull));
                    continue;
                }
                first_error = Some(err);
                break;
            }
        }

        let writebacks = self
            .call_arg_module_binding_ref_writebacks
            .pop()
            .unwrap_or_default();
        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(writebacks)
        }
    }

    pub(super) fn compile_implicit_reference_arg(
        &mut self,
        arg: &shape_ast::ast::Expr,
        mode: BorrowMode,
    ) -> Result<()> {
        use shape_ast::ast::Expr;
        match arg {
            Expr::Identifier(name, span) => self
                .compile_reference_identifier(name, *span, mode)
                .map(|_| ()),
            Expr::PropertyAccess {
                object,
                property,
                optional: false,
                span,
            } => self
                .compile_reference_property_access(object, property, *span, mode)
                .map(|_| ()),
            Expr::IndexAccess {
                object,
                index,
                end_index: None,
                span,
            } => self
                .compile_reference_index_access(object, index, *span, mode)
                .map(|_| ()),
            _ => {
                self.compile_expr_preserving_refs(arg)?;
                if let Some(returned_mode) = self.last_expr_reference_mode() {
                    if mode == BorrowMode::Exclusive && returned_mode != BorrowMode::Exclusive {
                        return Err(ShapeError::SemanticError {
                            message:
                                "cannot pass a shared reference result to an exclusive parameter"
                                    .to_string(),
                            location: Some(self.span_to_source_location(arg.span())),
                        });
                    }
                    return Ok(());
                }
                if mode == BorrowMode::Exclusive {
                    return Err(ShapeError::SemanticError {
                        message:
                            "[B0004] mutable reference arguments must be simple variables or existing exclusive references"
                                .to_string(),
                        location: Some(self.span_to_source_location(arg.span())),
                    });
                }
                let temp = self.declare_temp_local("__arg_ref_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(temp)),
                ));
                // MIR analysis is the sole authority for borrow checking.
                self.emit(Instruction::new(
                    OpCode::MakeRef,
                    Some(Operand::Local(temp)),
                ));
                Ok(())
            }
        }
    }

    pub(super) fn compile_reference_identifier(
        &mut self,
        name: &str,
        span: shape_ast::ast::Span,
        mode: BorrowMode,
    ) -> Result<u32> {
        if let Some(local_idx) = self.resolve_local(name) {
            // Reject exclusive borrows of const variables
            if mode == BorrowMode::Exclusive && self.const_locals.contains(&local_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot pass const variable '{}' by exclusive reference",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            if self.ref_locals.contains(&local_idx) {
                // Forward an existing reference parameter by value (TAG_REF).
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(local_idx)),
                ));
                return Ok(u32::MAX);
            }
            if self.reference_value_locals.contains(&local_idx) {
                if mode == BorrowMode::Exclusive
                    && !self.exclusive_reference_value_locals.contains(&local_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Cannot pass shared reference variable '{}' as an exclusive reference",
                            name
                        ),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(local_idx)),
                ));
                return Ok(u32::MAX);
            }
            // MIR analysis is the sole authority for borrow checking.
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::Local(local_idx)),
            ));
            Ok(u32::MAX)
        } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
            let Some(&binding_idx) = self.module_bindings.get(&scoped_name) else {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "[B0004] reference argument must be a local or module_binding variable, got '{}'",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            };
            // Reject exclusive borrows of const module bindings
            if mode == BorrowMode::Exclusive && self.const_module_bindings.contains(&binding_idx) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot pass const variable '{}' by exclusive reference",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }
            if self.reference_value_module_bindings.contains(&binding_idx) {
                if mode == BorrowMode::Exclusive
                    && !self
                        .exclusive_reference_value_module_bindings
                        .contains(&binding_idx)
                {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "Cannot pass shared reference variable '{}' as an exclusive reference",
                            name
                        ),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
                self.emit(Instruction::new(
                    OpCode::LoadModuleBinding,
                    Some(Operand::ModuleBinding(binding_idx)),
                ));
                return Ok(u32::MAX);
            }
            // MIR analysis is the sole authority for borrow checking.
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::ModuleBinding(binding_idx)),
            ));
            Ok(u32::MAX)
        } else if let Some(func_idx) = self.find_function(name) {
            // Function name passed as reference argument: create a temporary local
            // with the function constant and make a reference to it.
            let temp = self.declare_temp_local("__fn_ref_")?;
            let const_idx = self
                .program
                .add_constant(Constant::Function(func_idx as u16));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(const_idx)),
            ));
            self.emit(Instruction::new(
                OpCode::StoreLocal,
                Some(Operand::Local(temp)),
            ));
            // MIR analysis is the sole authority for borrow checking.
            self.emit(Instruction::new(
                OpCode::MakeRef,
                Some(Operand::Local(temp)),
            ));
            Ok(u32::MAX)
        } else {
            Err(ShapeError::SemanticError {
                message: format!(
                    "[B0004] reference argument must be a local or module_binding variable, got '{}'",
                    name
                ),
                location: Some(self.span_to_source_location(span)),
            })
        }
    }

    /// Push a new scope
    pub(super) fn push_scope(&mut self) {
        self.locals.push(HashMap::new());
        self.type_tracker.push_scope();
    }

    /// Pop a scope
    pub(super) fn pop_scope(&mut self) {
        self.locals.pop();
        self.type_tracker.pop_scope();
    }

    /// Declare a local variable
    pub(super) fn declare_local(&mut self, name: &str) -> Result<u16> {
        let idx = self.next_local;
        self.next_local += 1;

        if let Some(scope) = self.locals.last_mut() {
            scope.insert(name.to_string(), idx);
        }

        Ok(idx)
    }

    /// Resolve a local variable
    pub(super) fn resolve_local(&self, name: &str) -> Option<u16> {
        for scope in self.locals.iter().rev() {
            if let Some(&idx) = scope.get(name) {
                return Some(idx);
            }
        }
        None
    }

    /// Reverse lookup: slot index → local name (if any currently in scope).
    /// Used by the Phase V1.1C emission gate to consult `boxed_locals` (keyed
    /// by name) for a given slot. Returns the first match walking the scope
    /// stack innermost→outermost.
    pub(super) fn local_name_for_slot(&self, slot: u16) -> Option<&str> {
        for scope in self.locals.iter().rev() {
            for (name, &idx) in scope.iter() {
                if idx == slot {
                    return Some(name.as_str());
                }
            }
        }
        None
    }

    /// Phase V1.1C: true when the slot has been converted to a SharedCell
    /// wrapper via a prior `BoxLocal` (tracked in `self.boxed_locals` keyed
    /// by binding name). The V1.1C `CloneLocal` opcode does not auto-
    /// unwrap `SharedCell`s, so boxed slots must fall through to the
    /// legacy `LoadLocal` path which handles the unwrap.
    pub(super) fn slot_is_boxed(&self, slot: u16) -> bool {
        self.local_name_for_slot(slot)
            .map(|name| self.boxed_locals.contains(name))
            .unwrap_or(false)
    }

    /// Declare a temporary local variable
    pub(super) fn declare_temp_local(&mut self, prefix: &str) -> Result<u16> {
        let name = format!("{}{}", prefix, self.next_local);
        self.declare_local(&name)
    }

    /// Set type info for an existing local variable
    pub(super) fn set_local_type_info(&mut self, slot: u16, type_name: &str) {
        let info = if let Some(schema) = self.type_tracker.schema_registry().get(type_name) {
            VariableTypeInfo::known(schema.id, type_name.to_string())
        } else {
            VariableTypeInfo::named(type_name.to_string())
        };
        self.type_tracker.set_local_type(slot, info);
    }

    /// Set type info for a module_binding variable
    pub(super) fn set_module_binding_type_info(&mut self, slot: u16, type_name: &str) {
        let info = if let Some(schema) = self.type_tracker.schema_registry().get(type_name) {
            VariableTypeInfo::known(schema.id, type_name.to_string())
        } else {
            VariableTypeInfo::named(type_name.to_string())
        };
        self.type_tracker.set_binding_type(slot, info);
    }

    /// Capture local storage hints for a compiled function.
    ///
    /// Must be called before the function scope is popped so the type tracker still
    /// has local slot metadata. Also populates the function's `FrameDescriptor` so
    /// the verifier and executor can use per-slot type info for trusted opcodes.
    pub(super) fn capture_function_local_storage_hints(&mut self, func_idx: usize) {
        let Some(func) = self.program.functions.get(func_idx) else {
            return;
        };
        let hints: Vec<StorageHint> = (0..func.locals_count)
            .map(|slot| self.type_tracker.get_local_storage_hint(slot))
            .collect();

        // Populate FrameDescriptor on the function for trusted opcode verification.
        let has_any_known = hints.iter().any(|h| *h != StorageHint::Unknown);
        let instr_len = self.program.instructions.len();
        let code_end = if func.body_length > 0 {
            (func.entry_point + func.body_length).min(instr_len)
        } else {
            instr_len
        };
        let has_trusted = if func.entry_point <= code_end && code_end <= instr_len {
            self.program.instructions[func.entry_point..code_end]
                .iter()
                .any(|i| i.opcode.is_trusted())
        } else {
            false
        };
        if has_any_known || has_trusted {
            self.program.functions[func_idx].frame_descriptor = Some(
                crate::type_tracking::FrameDescriptor::from_slots(hints.clone()),
            );
        }

        if self.program.function_local_storage_hints.len() <= func_idx {
            self.program
                .function_local_storage_hints
                .resize(func_idx + 1, Vec::new());
        }
        self.program.function_local_storage_hints[func_idx] = hints;
    }

    /// Populate program-level storage hints for top-level locals and module bindings.
    pub(super) fn populate_program_storage_hints(&mut self) {
        let top_hints: Vec<StorageHint> = (0..self.next_local)
            .map(|slot| self.type_tracker.get_local_storage_hint(slot))
            .collect();
        self.program.top_level_local_storage_hints = top_hints.clone();

        // Build top-level FrameDescriptor so JIT can use per-slot type info
        let has_any_known = top_hints.iter().any(|h| *h != StorageHint::Unknown);
        let has_trusted = self
            .program
            .instructions
            .iter()
            .any(|i| i.opcode.is_trusted());
        if has_any_known || has_trusted {
            self.program.top_level_frame =
                Some(crate::type_tracking::FrameDescriptor::from_slots(top_hints));
        }

        let mut module_binding_hints = vec![StorageHint::Unknown; self.module_bindings.len()];
        for &idx in self.module_bindings.values() {
            if let Some(slot) = module_binding_hints.get_mut(idx as usize) {
                *slot = self.type_tracker.get_module_binding_storage_hint(idx);
            }
        }
        self.program.module_binding_storage_hints = module_binding_hints;

        if self.program.function_local_storage_hints.len() < self.program.functions.len() {
            self.program
                .function_local_storage_hints
                .resize(self.program.functions.len(), Vec::new());
        } else if self.program.function_local_storage_hints.len() > self.program.functions.len() {
            self.program
                .function_local_storage_hints
                .truncate(self.program.functions.len());
        }
    }

    /// Propagate the current expression's inferred type metadata to a target slot.
    ///
    /// Used by assignment sites to keep mutable locals/module_bindings typed when
    /// safe, and to clear stale hints when assigning unknown/dynamic values.
    pub(super) fn propagate_assignment_type_to_slot(
        &mut self,
        slot: u16,
        is_local: bool,
        allow_number_hint: bool,
    ) {
        if let Some(ref info) = self.last_expr_type_info {
            if info.is_indexed()
                || info.is_datatable()
                || info.schema_id.is_some()
                || Self::is_array_type_name(info.type_name.as_deref())
            {
                if is_local {
                    self.type_tracker.set_local_type(slot, info.clone());
                } else {
                    self.type_tracker.set_binding_type(slot, info.clone());
                }
                return;
            }
        }

        if let Some(schema_id) = self.last_expr_schema {
            let schema_name = self
                .type_tracker
                .schema_registry()
                .get_by_id(schema_id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| format!("__anon_{}", schema_id));
            let info = VariableTypeInfo::known(schema_id, schema_name);
            if is_local {
                self.type_tracker.set_local_type(slot, info);
            } else {
                self.type_tracker.set_binding_type(slot, info);
            }
            return;
        }

        if let Some(numeric_type) = self.last_expr_numeric_type {
            let (type_name, hint) = match numeric_type {
                crate::type_tracking::NumericType::Int => ("int", StorageHint::Int64),
                crate::type_tracking::NumericType::IntWidth(w) => {
                    use shape_ast::IntWidth;
                    let hint = match w {
                        IntWidth::I8 => StorageHint::Int8,
                        IntWidth::U8 => StorageHint::UInt8,
                        IntWidth::I16 => StorageHint::Int16,
                        IntWidth::U16 => StorageHint::UInt16,
                        IntWidth::I32 => StorageHint::Int32,
                        IntWidth::U32 => StorageHint::UInt32,
                        IntWidth::U64 => StorageHint::UInt64,
                    };
                    (w.type_name(), hint)
                }
                crate::type_tracking::NumericType::Number => {
                    if !allow_number_hint {
                        if is_local {
                            self.type_tracker
                                .set_local_type(slot, VariableTypeInfo::unknown());
                        } else {
                            self.type_tracker
                                .set_binding_type(slot, VariableTypeInfo::unknown());
                        }
                        return;
                    }
                    ("number", StorageHint::Float64)
                }
                // Decimal typed opcodes are not JIT-compiled yet.
                crate::type_tracking::NumericType::Decimal => {
                    if is_local {
                        self.type_tracker
                            .set_local_type(slot, VariableTypeInfo::unknown());
                    } else {
                        self.type_tracker
                            .set_binding_type(slot, VariableTypeInfo::unknown());
                    }
                    return;
                }
            };
            let info = VariableTypeInfo::with_storage(type_name.to_string(), hint);
            if is_local {
                self.type_tracker.set_local_type(slot, info);
            } else {
                self.type_tracker.set_binding_type(slot, info);
            }
            return;
        }

        // Assignment to an unknown/dynamic expression invalidates prior hints.
        if is_local {
            self.type_tracker
                .set_local_type(slot, VariableTypeInfo::unknown());
        } else {
            self.type_tracker
                .set_binding_type(slot, VariableTypeInfo::unknown());
        }
    }

    /// Propagate current expression type metadata to an identifier target.
    ///
    /// Reference locals are skipped because assignment writes through to a pointee.
    pub(super) fn propagate_assignment_type_to_identifier(&mut self, name: &str) {
        if let Some(local_idx) = self.resolve_local(name) {
            if self.local_binding_is_reference_value(local_idx) {
                return;
            }
            self.propagate_assignment_type_to_slot(local_idx, true, true);
            return;
        }

        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        let binding_idx = self.get_or_create_module_binding(&scoped_name);
        self.propagate_assignment_type_to_slot(binding_idx, false, true);
    }

    /// Get the type tracker (for external configuration)
    /// Resolve a local namespace name to its canonical module path.
    ///
    /// Checks `graph_namespace_map` first (populated by graph-driven compilation),
    /// then falls back to `module_scope_sources` (legacy AST inlining path).
    pub(crate) fn resolve_canonical_module_path(&self, local_name: &str) -> Option<String> {
        self.graph_namespace_map
            .get(local_name)
            .or_else(|| self.module_scope_sources.get(local_name))
            .cloned()
    }

    pub fn type_tracker(&self) -> &TypeTracker {
        &self.type_tracker
    }

    /// Get mutable type tracker (for registering types)
    pub fn type_tracker_mut(&mut self) -> &mut TypeTracker {
        &mut self.type_tracker
    }

    /// Resolve a column name to its index using the data schema.
    /// Returns an error if no schema is provided or the column doesn't exist.
    pub(super) fn resolve_column_index(&self, field: &str) -> Result<u32> {
        self.program
            .data_schema
            .as_ref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "No data schema provided. Cannot resolve field '{}'. \
                     Hint: Use stdlib/finance to load market data with OHLCV schema.",
                    field
                ),
                location: None,
            })?
            .get_index(field)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Unknown column '{}' in data schema. Available columns: {:?}",
                    field,
                    self.program
                        .data_schema
                        .as_ref()
                        .map(|s| &s.column_names)
                        .unwrap_or(&vec![])
                ),
                location: None,
            })
    }

    /// Check if a field name is a known data column in the schema.
    pub(super) fn is_data_column(&self, field: &str) -> bool {
        self.program
            .data_schema
            .as_ref()
            .map(|s| s.get_index(field).is_some())
            .unwrap_or(false)
    }

    /// Collect all outer scope variables
    pub(super) fn collect_outer_scope_vars(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for scope in &self.locals {
            for name in scope.keys() {
                names.insert(name.clone());
            }
        }
        for name in self.module_bindings.keys() {
            names.insert(name.clone());
        }
        names.into_iter().collect()
    }

    /// Get or create a module_binding variable
    pub(super) fn get_or_create_module_binding(&mut self, name: &str) -> u16 {
        if let Some(&idx) = self.module_bindings.get(name) {
            idx
        } else {
            let idx = self.next_global;
            self.next_global += 1;
            self.module_bindings.insert(name.to_string(), idx);
            idx
        }
    }

    pub(super) fn resolve_scoped_module_binding_name(&self, name: &str) -> Option<String> {
        if crate::module_resolution::is_hidden_annotation_import_module_name(name) {
            return None;
        }
        if self.module_bindings.contains_key(name) {
            return Some(name.to_string());
        }
        for module_path in self.module_scope_stack.iter().rev() {
            let candidate = format!("{}::{}", module_path, name);
            if self.module_bindings.contains_key(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    pub(super) fn resolve_scoped_function_name(&self, name: &str) -> Option<String> {
        if self.program.functions.iter().any(|f| f.name == name) {
            return Some(name.to_string());
        }
        for module_path in self.module_scope_stack.iter().rev() {
            let candidate = format!("{}::{}", module_path, name);
            if self.program.functions.iter().any(|f| f.name == candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// Find a function by name
    pub(super) fn find_function(&self, name: &str) -> Option<usize> {
        // Check function aliases first (e.g., __original__ -> shadow function).
        if let Some(actual_name) = self.function_aliases.get(name) {
            if let Some(idx) = self
                .program
                .functions
                .iter()
                .position(|f| f.name == *actual_name)
            {
                return Some(idx);
            }
        }

        // Try direct/scoped resolution
        if let Some(resolved) = self.resolve_scoped_function_name(name) {
            if let Some(idx) = self
                .program
                .functions
                .iter()
                .position(|f| f.name == resolved)
            {
                return Some(idx);
            }
        }

        // If direct lookup failed, check imported_names for alias -> original name mapping.
        // When a function is imported with an alias (e.g., `use { foo as bar } from "module"`),
        // the function is registered under its original (possibly module-qualified) name,
        // but the user refers to it by the alias.
        if let Some(imported) = self.imported_names.get(name) {
            let original = &imported.original_name;
            // Try direct match on the original name
            if let Some(idx) = self
                .program
                .functions
                .iter()
                .position(|f| f.name == *original)
            {
                return Some(idx);
            }
            // Try scoped resolution on the original name
            if let Some(resolved) = self.resolve_scoped_function_name(original) {
                if let Some(idx) = self
                    .program
                    .functions
                    .iter()
                    .position(|f| f.name == resolved)
                {
                    return Some(idx);
                }
            }
            // Try module-qualified name: module_path::original_name
            // This is needed for graph-compiled dependencies where functions
            // are registered with their module-qualified names.
            if !imported.module_path.is_empty() {
                let qualified = format!("{}::{}", imported.module_path, original);
                if let Some(idx) = self
                    .program
                    .functions
                    .iter()
                    .position(|f| f.name == qualified)
                {
                    return Some(idx);
                }
            }
        }

        None
    }

    /// Resolve the receiver's type name for extend method dispatch.
    ///
    /// Determines the Shape type name from all available compiler state:
    /// - `last_expr_type_info.type_name` for TypedObjects (e.g., "Point", "Candle")
    /// - `last_expr_numeric_type` for numeric types → "Int", "Number", "Decimal"
    /// - Receiver expression analysis for arrays, strings, booleans
    ///
    /// Returns the base type name (e.g., "Vec" not "Vec<int>") suitable for
    /// extend method lookup as "Type.method".
    pub(super) fn resolve_receiver_extend_type(
        &self,
        receiver: &shape_ast::ast::Expr,
        receiver_type_info: &Option<crate::type_tracking::VariableTypeInfo>,
        _receiver_schema: Option<u32>,
    ) -> Option<String> {
        // 1. Numeric type from typed opcode tracking — checked first because
        //    the type tracker stores lowercase names ("int", "number") while
        //    extend blocks use capitalized TypeName ("Int", "Number", "Decimal").
        if let Some(numeric) = self.last_expr_numeric_type {
            return Some(
                match numeric {
                    crate::type_tracking::NumericType::Int
                    | crate::type_tracking::NumericType::IntWidth(_) => "Int",
                    crate::type_tracking::NumericType::Number => "Number",
                    crate::type_tracking::NumericType::Decimal => "Decimal",
                }
                .to_string(),
            );
        }

        // 2. TypedObject type name (user-defined types like Point, Candle)
        if let Some(info) = receiver_type_info {
            if let Some(type_name) = &info.type_name {
                // Strip generic params: "Vec<int>" → "Vec"
                let base = type_name.split('<').next().unwrap_or(type_name);
                return Some(base.to_string());
            }
        }

        // 3. Infer from receiver expression shape
        match receiver {
            shape_ast::ast::Expr::Literal(lit, _) => match lit {
                shape_ast::ast::Literal::String(_)
                | shape_ast::ast::Literal::FormattedString { .. }
                | shape_ast::ast::Literal::ContentString { .. } => Some("String".to_string()),
                shape_ast::ast::Literal::Bool(_) => Some("Bool".to_string()),
                _ => None,
            },
            shape_ast::ast::Expr::Array(..) => Some("Vec".to_string()),
            _ => None,
        }
    }

    /// Emit store instruction for an identifier
    pub(super) fn emit_store_identifier(&mut self, name: &str) -> Result<()> {
        // Mutable closure captures: emit StoreClosure (or Phase D's typed
        // StoreCaptureMutPtr<T> when the outer slot is `LocalMutablePtr`).
        if let Some(&upvalue_idx) = self.mutable_closure_captures.get(name) {
            if let Some(&(ptr_idx, kind)) = self.local_mutable_ptr_captures.get(name) {
                use shape_value::v2::struct_layout::FieldKind;
                let op = match kind {
                    FieldKind::F64 => OpCode::StoreCaptureMutPtrF64,
                    FieldKind::I64 => OpCode::StoreCaptureMutPtrI64,
                    FieldKind::I32 => OpCode::StoreCaptureMutPtrI32,
                    FieldKind::Bool => OpCode::StoreCaptureMutPtrBool,
                    _ => OpCode::StoreCaptureMutPtrPtr,
                };
                self.emit(Instruction::new(op, Some(Operand::Local(ptr_idx))));
                return Ok(());
            }
            self.emit(Instruction::new(
                OpCode::StoreClosure,
                Some(Operand::Local(upvalue_idx)),
            ));
            return Ok(());
        }
        if let Some(local_idx) = self.resolve_local(name) {
            if self.local_binding_is_reference_value(local_idx) {
                if !self.local_reference_binding_is_exclusive(local_idx) {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "cannot assign through shared reference variable '{}'",
                            name
                        ),
                        location: None,
                    });
                }
                self.emit(Instruction::new(
                    OpCode::DerefStore,
                    Some(Operand::Local(local_idx)),
                ));
            } else {
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(local_idx)),
                ));
                // Patch StoreLocal → StoreLocalTyped for width-typed locals
                if let Some(type_name) = self
                    .type_tracker
                    .get_local_type(local_idx)
                    .and_then(|info| info.type_name.as_deref())
                {
                    if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
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
        } else {
            let scoped_name = self
                .resolve_scoped_module_binding_name(name)
                .unwrap_or_else(|| name.to_string());
            let binding_idx = self.get_or_create_module_binding(&scoped_name);
            self.emit(Instruction::new(
                OpCode::StoreModuleBinding,
                Some(Operand::ModuleBinding(binding_idx)),
            ));
            // Patch StoreModuleBinding → StoreModuleBindingTyped for width-typed bindings
            if let Some(type_name) = self
                .type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| info.type_name.as_deref())
            {
                if let Some(w) = shape_ast::IntWidth::from_name(type_name) {
                    if let Some(last) = self.program.instructions.last_mut() {
                        if last.opcode == OpCode::StoreModuleBinding {
                            last.opcode = OpCode::StoreModuleBindingTyped;
                            last.operand = Some(Operand::TypedModuleBinding(
                                binding_idx,
                                crate::bytecode::NumericWidth::from_int_width(w),
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn classify_builtin_function(&self, name: &str) -> Option<BuiltinNameResolution> {
        let builtin = match name {
            // Option type constructor
            "Some" => BuiltinFunction::SomeCtor,
            "Ok" => BuiltinFunction::OkCtor,
            "Err" => BuiltinFunction::ErrCtor,
            "HashMap" => BuiltinFunction::HashMapCtor,
            "Set" => BuiltinFunction::SetCtor,
            "Deque" => BuiltinFunction::DequeCtor,
            "PriorityQueue" => BuiltinFunction::PriorityQueueCtor,
            "Mutex" => BuiltinFunction::MutexCtor,
            "Atomic" => BuiltinFunction::AtomicCtor,
            "Lazy" => BuiltinFunction::LazyCtor,
            "Channel" => BuiltinFunction::ChannelCtor,
            // Json navigation helpers
            "__json_object_get" => BuiltinFunction::JsonObjectGet,
            "__json_array_at" => BuiltinFunction::JsonArrayAt,
            "__json_object_keys" => BuiltinFunction::JsonObjectKeys,
            "__json_array_len" => BuiltinFunction::JsonArrayLen,
            "__json_object_len" => BuiltinFunction::JsonObjectLen,
            "__intrinsic_vec_abs" => BuiltinFunction::IntrinsicVecAbs,
            "__intrinsic_vec_sqrt" => BuiltinFunction::IntrinsicVecSqrt,
            "__intrinsic_vec_ln" => BuiltinFunction::IntrinsicVecLn,
            "__intrinsic_vec_exp" => BuiltinFunction::IntrinsicVecExp,
            "__intrinsic_vec_add" => BuiltinFunction::IntrinsicVecAdd,
            "__intrinsic_vec_sub" => BuiltinFunction::IntrinsicVecSub,
            "__intrinsic_vec_mul" => BuiltinFunction::IntrinsicVecMul,
            "__intrinsic_vec_div" => BuiltinFunction::IntrinsicVecDiv,
            "__intrinsic_vec_max" => BuiltinFunction::IntrinsicVecMax,
            "__intrinsic_vec_min" => BuiltinFunction::IntrinsicVecMin,
            "__intrinsic_vec_select" => BuiltinFunction::IntrinsicVecSelect,
            "__intrinsic_matmul_vec" => BuiltinFunction::IntrinsicMatMulVec,
            "__intrinsic_matmul_mat" => BuiltinFunction::IntrinsicMatMulMat,

            // Existing builtins
            "abs" => BuiltinFunction::Abs,
            "min" => BuiltinFunction::Min,
            "max" => BuiltinFunction::Max,
            "sqrt" => BuiltinFunction::Sqrt,
            "ln" => BuiltinFunction::Ln,
            "pow" => BuiltinFunction::Pow,
            "exp" => BuiltinFunction::Exp,
            "log" => BuiltinFunction::Log,
            "floor" => BuiltinFunction::Floor,
            "ceil" => BuiltinFunction::Ceil,
            "round" => BuiltinFunction::Round,
            "sin" => BuiltinFunction::Sin,
            "cos" => BuiltinFunction::Cos,
            "tan" => BuiltinFunction::Tan,
            "asin" => BuiltinFunction::Asin,
            "acos" => BuiltinFunction::Acos,
            "atan" => BuiltinFunction::Atan,
            "stddev" => BuiltinFunction::StdDev,
            "__intrinsic_map" => BuiltinFunction::Map,
            "__intrinsic_filter" => BuiltinFunction::Filter,
            "__intrinsic_reduce" => BuiltinFunction::Reduce,
            "print" => BuiltinFunction::Print,
            "format" => BuiltinFunction::Format,
            "len" | "count" => BuiltinFunction::Len,
            // "throw" removed: Shape uses Result types
            "__intrinsic_snapshot" | "snapshot" => BuiltinFunction::Snapshot,
            "exit" => BuiltinFunction::Exit,
            "range" => BuiltinFunction::Range,
            "is_number" | "isNumber" => BuiltinFunction::IsNumber,
            "is_string" | "isString" => BuiltinFunction::IsString,
            "is_bool" | "isBool" => BuiltinFunction::IsBool,
            "is_array" | "isArray" => BuiltinFunction::IsArray,
            "is_object" | "isObject" => BuiltinFunction::IsObject,
            "is_data_row" | "isDataRow" => BuiltinFunction::IsDataRow,
            "to_string" | "toString" => BuiltinFunction::ToString,
            "to_number" | "toNumber" => BuiltinFunction::ToNumber,
            "to_bool" | "toBool" => BuiltinFunction::ToBool,
            // __into_*/__try_into_* builtins removed — primitive conversions now use
            // typed ConvertTo*/TryConvertTo* opcodes emitted directly by the compiler.
            "__native_ptr_size" => BuiltinFunction::NativePtrSize,
            "__native_ptr_new_cell" => BuiltinFunction::NativePtrNewCell,
            "__native_ptr_free_cell" => BuiltinFunction::NativePtrFreeCell,
            "__native_ptr_read_ptr" => BuiltinFunction::NativePtrReadPtr,
            "__native_ptr_write_ptr" => BuiltinFunction::NativePtrWritePtr,
            "__native_table_from_arrow_c" => BuiltinFunction::NativeTableFromArrowC,
            "__native_table_from_arrow_c_typed" => BuiltinFunction::NativeTableFromArrowCTyped,
            "__native_table_bind_type" => BuiltinFunction::NativeTableBindType,
            "fold" => BuiltinFunction::ControlFold,

            // Math intrinsics
            "__intrinsic_minimize" => BuiltinFunction::IntrinsicMinimize,
            "__intrinsic_bspline2_3d_batch" => BuiltinFunction::IntrinsicBspline2_3dBatch,
            "__intrinsic_sum" => BuiltinFunction::IntrinsicSum,
            "__intrinsic_mean" => BuiltinFunction::IntrinsicMean,
            "__intrinsic_min" => BuiltinFunction::IntrinsicMin,
            "__intrinsic_max" => BuiltinFunction::IntrinsicMax,
            "__intrinsic_std" => BuiltinFunction::IntrinsicStd,
            "__intrinsic_variance" => BuiltinFunction::IntrinsicVariance,

            // Random intrinsics
            "__intrinsic_random" => BuiltinFunction::IntrinsicRandom,
            "__intrinsic_random_int" => BuiltinFunction::IntrinsicRandomInt,
            "__intrinsic_random_seed" => BuiltinFunction::IntrinsicRandomSeed,
            "__intrinsic_random_normal" => BuiltinFunction::IntrinsicRandomNormal,
            "__intrinsic_random_array" => BuiltinFunction::IntrinsicRandomArray,

            // Distribution intrinsics
            "__intrinsic_dist_uniform" => BuiltinFunction::IntrinsicDistUniform,
            "__intrinsic_dist_lognormal" => BuiltinFunction::IntrinsicDistLognormal,
            "__intrinsic_dist_exponential" => BuiltinFunction::IntrinsicDistExponential,
            "__intrinsic_dist_poisson" => BuiltinFunction::IntrinsicDistPoisson,
            "__intrinsic_dist_sample_n" => BuiltinFunction::IntrinsicDistSampleN,

            // Stochastic process intrinsics
            "__intrinsic_brownian_motion" => BuiltinFunction::IntrinsicBrownianMotion,
            "__intrinsic_gbm" => BuiltinFunction::IntrinsicGbm,
            "__intrinsic_ou_process" => BuiltinFunction::IntrinsicOuProcess,
            "__intrinsic_random_walk" => BuiltinFunction::IntrinsicRandomWalk,

            // Rolling intrinsics
            "__intrinsic_rolling_sum" => BuiltinFunction::IntrinsicRollingSum,
            "__intrinsic_rolling_mean" => BuiltinFunction::IntrinsicRollingMean,
            "__intrinsic_rolling_std" => BuiltinFunction::IntrinsicRollingStd,
            "__intrinsic_rolling_min" => BuiltinFunction::IntrinsicRollingMin,
            "__intrinsic_rolling_max" => BuiltinFunction::IntrinsicRollingMax,
            "__intrinsic_ema" => BuiltinFunction::IntrinsicEma,
            "__intrinsic_linear_recurrence" => BuiltinFunction::IntrinsicLinearRecurrence,

            // Series intrinsics
            "__intrinsic_shift" => BuiltinFunction::IntrinsicShift,
            "__intrinsic_diff" => BuiltinFunction::IntrinsicDiff,
            "__intrinsic_pct_change" => BuiltinFunction::IntrinsicPctChange,
            "__intrinsic_fillna" => BuiltinFunction::IntrinsicFillna,
            "__intrinsic_cumsum" => BuiltinFunction::IntrinsicCumsum,
            "__intrinsic_cumprod" => BuiltinFunction::IntrinsicCumprod,
            "__intrinsic_clip" => BuiltinFunction::IntrinsicClip,

            // Trigonometric intrinsics (map __intrinsic_ forms to existing builtins)
            "__intrinsic_sin" => BuiltinFunction::Sin,
            "__intrinsic_cos" => BuiltinFunction::Cos,
            "__intrinsic_tan" => BuiltinFunction::Tan,
            "__intrinsic_asin" => BuiltinFunction::Asin,
            "__intrinsic_acos" => BuiltinFunction::Acos,
            "__intrinsic_atan" => BuiltinFunction::Atan,
            "__intrinsic_atan2" => BuiltinFunction::IntrinsicAtan2,
            "__intrinsic_sinh" => BuiltinFunction::IntrinsicSinh,
            "__intrinsic_cosh" => BuiltinFunction::IntrinsicCosh,
            "__intrinsic_tanh" => BuiltinFunction::IntrinsicTanh,

            // Statistical intrinsics
            "__intrinsic_correlation" => BuiltinFunction::IntrinsicCorrelation,
            "__intrinsic_covariance" => BuiltinFunction::IntrinsicCovariance,
            "__intrinsic_percentile" => BuiltinFunction::IntrinsicPercentile,
            "__intrinsic_median" => BuiltinFunction::IntrinsicMedian,

            // Character code intrinsics
            "__intrinsic_char_code" => BuiltinFunction::IntrinsicCharCode,
            "__intrinsic_from_char_code" => BuiltinFunction::IntrinsicFromCharCode,

            // Series access
            "__intrinsic_series" => BuiltinFunction::IntrinsicSeries,

            // Reflection
            "reflect" => BuiltinFunction::Reflect,

            // Additional math builtins
            "sign" => BuiltinFunction::Sign,
            "gcd" => BuiltinFunction::Gcd,
            "lcm" => BuiltinFunction::Lcm,
            "hypot" => BuiltinFunction::Hypot,
            "clamp" => BuiltinFunction::Clamp,
            "isNaN" | "is_nan" => BuiltinFunction::IsNaN,
            "isFinite" | "is_finite" => BuiltinFunction::IsFinite,
            "mat" => BuiltinFunction::MatFromFlat,
            _ => return None,
        };

        let scope = match name {
            "Some" | "Ok" | "Err" => ResolutionScope::TypeAssociated,
            "print" => ResolutionScope::Prelude,
            _ if Self::is_internal_intrinsic_name(name) => ResolutionScope::InternalIntrinsic,
            _ => ResolutionScope::ModuleBinding,
        };

        Some(match scope {
            ResolutionScope::InternalIntrinsic => {
                BuiltinNameResolution::InternalOnly { builtin, scope }
            }
            _ => BuiltinNameResolution::Surface { builtin, scope },
        })
    }

    pub(super) fn is_internal_intrinsic_name(name: &str) -> bool {
        name.starts_with("__native_")
            || name.starts_with("__intrinsic_")
            || name.starts_with("__json_")
    }

    pub(super) const fn variable_scope_summary() -> &'static str {
        "Variable names resolve from local scope and module scope."
    }

    pub(super) const fn function_scope_summary() -> &'static str {
        "Function names resolve from module scope, explicit imports, type-associated scope, and the implicit prelude."
    }

    pub(super) fn undefined_variable_message(&self, name: &str) -> String {
        format!(
            "Undefined variable: {}. {}",
            name,
            Self::variable_scope_summary()
        )
    }

    pub(super) fn undefined_function_message(&self, name: &str) -> String {
        format!(
            "Undefined function: {}. {}",
            name,
            Self::function_scope_summary()
        )
    }

    pub(super) fn internal_intrinsic_error_message(
        &self,
        name: &str,
        resolution: BuiltinNameResolution,
    ) -> String {
        format!(
            "'{}' resolves to {} and is not available from ordinary user code. Internal intrinsics are reserved for std::* implementations and compiler-generated code.",
            name,
            resolution.scope().label()
        )
    }

    /// Check if a builtin function requires arg count
    pub(super) fn builtin_requires_arg_count(&self, builtin: BuiltinFunction) -> bool {
        matches!(
            builtin,
            BuiltinFunction::Abs
                | BuiltinFunction::Min
                | BuiltinFunction::Max
                | BuiltinFunction::Sqrt
                | BuiltinFunction::Ln
                | BuiltinFunction::Pow
                | BuiltinFunction::Exp
                | BuiltinFunction::Log
                | BuiltinFunction::Floor
                | BuiltinFunction::Ceil
                | BuiltinFunction::Round
                | BuiltinFunction::Sin
                | BuiltinFunction::Cos
                | BuiltinFunction::Tan
                | BuiltinFunction::Asin
                | BuiltinFunction::Acos
                | BuiltinFunction::Atan
                | BuiltinFunction::StdDev
                | BuiltinFunction::Range
                | BuiltinFunction::Slice
                | BuiltinFunction::Push
                | BuiltinFunction::Pop
                | BuiltinFunction::First
                | BuiltinFunction::Last
                | BuiltinFunction::Zip
                | BuiltinFunction::Map
                | BuiltinFunction::Filter
                | BuiltinFunction::Reduce
                | BuiltinFunction::ForEach
                | BuiltinFunction::Find
                | BuiltinFunction::FindIndex
                | BuiltinFunction::Some
                | BuiltinFunction::Every
                | BuiltinFunction::SomeCtor
                | BuiltinFunction::OkCtor
                | BuiltinFunction::ErrCtor
                | BuiltinFunction::HashMapCtor
                | BuiltinFunction::SetCtor
                | BuiltinFunction::DequeCtor
                | BuiltinFunction::PriorityQueueCtor
                | BuiltinFunction::MutexCtor
                | BuiltinFunction::AtomicCtor
                | BuiltinFunction::LazyCtor
                | BuiltinFunction::ChannelCtor
                | BuiltinFunction::Print
                | BuiltinFunction::Format
                | BuiltinFunction::Len
                // BuiltinFunction::Throw removed
                | BuiltinFunction::Snapshot
                | BuiltinFunction::ObjectRest
                | BuiltinFunction::IsNumber
                | BuiltinFunction::IsString
                | BuiltinFunction::IsBool
                | BuiltinFunction::IsArray
                | BuiltinFunction::IsObject
                | BuiltinFunction::IsDataRow
                | BuiltinFunction::ToString
                | BuiltinFunction::ToNumber
                | BuiltinFunction::ToBool
                | BuiltinFunction::NativePtrSize
                | BuiltinFunction::NativePtrNewCell
                | BuiltinFunction::NativePtrFreeCell
                | BuiltinFunction::NativePtrReadPtr
                | BuiltinFunction::NativePtrWritePtr
                | BuiltinFunction::NativeTableFromArrowC
                | BuiltinFunction::NativeTableFromArrowCTyped
                | BuiltinFunction::NativeTableBindType
                | BuiltinFunction::ControlFold
                | BuiltinFunction::IntrinsicMinimize
                | BuiltinFunction::IntrinsicBspline2_3dBatch
                | BuiltinFunction::IntrinsicSum
                | BuiltinFunction::IntrinsicMean
                | BuiltinFunction::IntrinsicMin
                | BuiltinFunction::IntrinsicMax
                | BuiltinFunction::IntrinsicStd
                | BuiltinFunction::IntrinsicVariance
                | BuiltinFunction::IntrinsicRandom
                | BuiltinFunction::IntrinsicRandomInt
                | BuiltinFunction::IntrinsicRandomSeed
                | BuiltinFunction::IntrinsicRandomNormal
                | BuiltinFunction::IntrinsicRandomArray
                | BuiltinFunction::IntrinsicDistUniform
                | BuiltinFunction::IntrinsicDistLognormal
                | BuiltinFunction::IntrinsicDistExponential
                | BuiltinFunction::IntrinsicDistPoisson
                | BuiltinFunction::IntrinsicDistSampleN
                | BuiltinFunction::IntrinsicBrownianMotion
                | BuiltinFunction::IntrinsicGbm
                | BuiltinFunction::IntrinsicOuProcess
                | BuiltinFunction::IntrinsicRandomWalk
                | BuiltinFunction::IntrinsicRollingSum
                | BuiltinFunction::IntrinsicRollingMean
                | BuiltinFunction::IntrinsicRollingStd
                | BuiltinFunction::IntrinsicRollingMin
                | BuiltinFunction::IntrinsicRollingMax
                | BuiltinFunction::IntrinsicEma
                | BuiltinFunction::IntrinsicLinearRecurrence
                | BuiltinFunction::IntrinsicShift
                | BuiltinFunction::IntrinsicDiff
                | BuiltinFunction::IntrinsicPctChange
                | BuiltinFunction::IntrinsicFillna
                | BuiltinFunction::IntrinsicCumsum
                | BuiltinFunction::IntrinsicCumprod
                | BuiltinFunction::IntrinsicClip
                | BuiltinFunction::IntrinsicCorrelation
                | BuiltinFunction::IntrinsicCovariance
                | BuiltinFunction::IntrinsicPercentile
                | BuiltinFunction::IntrinsicMedian
                | BuiltinFunction::IntrinsicAtan2
                | BuiltinFunction::IntrinsicSinh
                | BuiltinFunction::IntrinsicCosh
                | BuiltinFunction::IntrinsicTanh
                | BuiltinFunction::IntrinsicCharCode
                | BuiltinFunction::IntrinsicFromCharCode
                | BuiltinFunction::IntrinsicSeries
                | BuiltinFunction::IntrinsicVecAbs
                | BuiltinFunction::IntrinsicVecSqrt
                | BuiltinFunction::IntrinsicVecLn
                | BuiltinFunction::IntrinsicVecExp
                | BuiltinFunction::IntrinsicVecAdd
                | BuiltinFunction::IntrinsicVecSub
                | BuiltinFunction::IntrinsicVecMul
                | BuiltinFunction::IntrinsicVecDiv
                | BuiltinFunction::IntrinsicVecMax
                | BuiltinFunction::IntrinsicVecMin
                | BuiltinFunction::IntrinsicVecSelect
                | BuiltinFunction::IntrinsicMatMulVec
                | BuiltinFunction::IntrinsicMatMulMat
                | BuiltinFunction::Sign
                | BuiltinFunction::Gcd
                | BuiltinFunction::Lcm
                | BuiltinFunction::Hypot
                | BuiltinFunction::Clamp
                | BuiltinFunction::IsNaN
                | BuiltinFunction::IsFinite
                | BuiltinFunction::MatFromFlat
        )
    }

    /// Check if any compiled function exists whose name indicates a user-defined
    /// override of the given method name (via extend blocks or impl blocks).
    ///
    /// Looks for function names like `Type.method` or `Type::method`.
    pub(super) fn has_any_user_defined_method(&self, method: &str) -> bool {
        let dot_suffix = format!(".{}", method);
        let colon_suffix = format!("::{}", method);
        self.program
            .functions
            .iter()
            .any(|f| f.name.ends_with(&dot_suffix) || f.name.ends_with(&colon_suffix))
    }

    /// Check if a method name is a known built-in method on any VM type.
    /// Used by UFCS to determine if `receiver.method(args)` should be dispatched
    /// as a built-in method call or rewritten to `method(receiver, args)`.
    pub(super) fn is_known_builtin_method(method: &str) -> bool {
        // Array methods (from ARRAY_METHODS PHF map)
        matches!(method,
            "map" | "filter" | "reduce" | "forEach" | "find" | "findIndex"
            | "some" | "every" | "sort" | "groupBy" | "flatMap"
            | "len" | "length" | "first" | "last" | "reverse" | "slice"
            | "concat" | "take" | "drop" | "skip"
            | "indexOf" | "includes"
            | "join" | "flatten" | "unique" | "distinct" | "distinctBy"
            | "sum" | "avg" | "min" | "max" | "count"
            | "where" | "select" | "orderBy" | "thenBy" | "takeWhile"
            | "skipWhile" | "single" | "any" | "all"
            | "innerJoin" | "leftJoin" | "crossJoin"
            | "union" | "intersect" | "except"
        )
        // DataTable methods (from DATATABLE_METHODS PHF map)
        || matches!(method,
            "columns" | "column" | "head" | "tail" | "mean" | "std"
            | "describe" | "aggregate" | "group_by" | "index_by" | "indexBy"
            | "simulate" | "toMat" | "to_mat"
        )
        // Column methods (from COLUMN_METHODS PHF map)
        || matches!(method, "toArray")
        // IndexedTable methods (from INDEXED_TABLE_METHODS PHF map)
        || matches!(method, "resample" | "between")
        // Number methods handled inline in op_call_method
        || matches!(method,
            "toFixed" | "toInt" | "toNumber" | "to_number" | "floor" | "ceil" | "round"
            | "abs" | "sign" | "clamp"
        )
        // String methods handled inline
        || matches!(method,
            "toUpperCase" | "toLowerCase" | "trim" | "contains" | "startsWith"
            | "endsWith" | "split" | "replace" | "substring" | "charAt"
            | "padStart" | "padEnd" | "repeat" | "toString"
        )
        // Object methods handled by handle_object_method
        || matches!(method, "keys" | "values" | "has" | "get" | "set" | "len")
        // DateTime methods (from DATETIME_METHODS PHF map)
        || matches!(method, "format")
        // Universal intrinsic methods
        || matches!(method, "type")
    }

    /// Try to track a `Table<T>` type annotation as a DataTable variable.
    ///
    /// If the annotation is `Generic { name: "Table", args: [Reference(T)] }`,
    /// looks up T's schema and marks the variable as `is_datatable`.
    pub(super) fn try_track_datatable_type(
        &mut self,
        type_ann: &shape_ast::ast::TypeAnnotation,
        slot: u16,
        is_local: bool,
    ) -> shape_ast::error::Result<()> {
        use shape_ast::ast::TypeAnnotation;
        if let TypeAnnotation::Generic { name, args } = type_ann {
            if name == "Table" && args.len() == 1 {
                let inner_name = match &args[0] {
                    TypeAnnotation::Reference(t) => Some(t.as_str()),
                    TypeAnnotation::Basic(t) => Some(t.as_str()),
                    _ => None,
                };
                if let Some(type_name) = inner_name {
                    let schema_id = self
                        .type_tracker
                        .schema_registry()
                        .get(type_name)
                        .map(|s| s.id);
                    if let Some(sid) = schema_id {
                        let info = crate::type_tracking::VariableTypeInfo::datatable(
                            sid,
                            type_name.to_string(),
                        );
                        if is_local {
                            self.type_tracker.set_local_type(slot, info);
                        } else {
                            self.type_tracker.set_binding_type(slot, info);
                        }
                    } else if type_name.len() == 1 && type_name.chars().next().map_or(false, |c| c.is_ascii_uppercase()) {
                        // Generic type parameter (e.g., T) — skip DataTable tracking,
                        // the concrete type will be determined at the call site.
                    } else {
                        return Err(shape_ast::error::ShapeError::SemanticError {
                            message: format!(
                                "Unknown type '{}' in Table<{}> annotation",
                                type_name, type_name
                            ),
                            location: None,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Check if a variable is a RowView (typed row from Arrow DataTable).
    pub(super) fn is_row_view_variable(&self, name: &str) -> bool {
        if let Some(local_idx) = self.resolve_local(name) {
            if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                return info.is_row_view();
            }
        }
        if let Some(&binding_idx) = self.module_bindings.get(name) {
            if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                return info.is_row_view();
            }
        }
        false
    }

    /// Get the available field names for a RowView variable's schema.
    pub(super) fn get_row_view_field_names(&self, name: &str) -> Option<Vec<String>> {
        let type_name = if let Some(local_idx) = self.resolve_local(name) {
            self.type_tracker
                .get_local_type(local_idx)
                .and_then(|info| {
                    if info.is_row_view() {
                        info.type_name.clone()
                    } else {
                        None
                    }
                })
        } else if let Some(&binding_idx) = self.module_bindings.get(name) {
            self.type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| {
                    if info.is_row_view() {
                        info.type_name.clone()
                    } else {
                        None
                    }
                })
        } else {
            None
        };

        if let Some(tn) = type_name {
            if let Some(schema) = self.type_tracker.schema_registry().get(&tn) {
                return Some(schema.field_names().map(|n| n.to_string()).collect());
            }
        }
        None
    }

    /// Try to resolve a property access on a RowView variable to a column ID.
    ///
    /// Returns `Some(col_id)` if the variable is a tracked RowView and the field
    /// exists in its schema. Returns `None` if the variable isn't a RowView or
    /// the field is unknown (caller should emit a compile-time error).
    pub(super) fn try_resolve_row_view_column(
        &self,
        var_name: &str,
        field_name: &str,
    ) -> Option<u32> {
        // Check locals first, then module_bindings
        if let Some(local_idx) = self.resolve_local(var_name) {
            return self
                .type_tracker
                .get_row_view_column_id(local_idx, true, field_name);
        }
        if let Some(&binding_idx) = self.module_bindings.get(var_name) {
            return self
                .type_tracker
                .get_row_view_column_id(binding_idx, false, field_name);
        }
        None
    }

    /// Determine the appropriate LoadCol opcode for a RowView field.
    ///
    /// Looks up the field's FieldType and maps it to the corresponding opcode.
    /// Falls back to LoadColF64 if the type can't be determined.
    pub(super) fn row_view_field_opcode(&self, var_name: &str, field_name: &str) -> OpCode {
        use shape_runtime::type_schema::FieldType;

        let type_name = if let Some(local_idx) = self.resolve_local(var_name) {
            self.type_tracker
                .get_local_type(local_idx)
                .and_then(|info| info.type_name.clone())
        } else if let Some(&binding_idx) = self.module_bindings.get(var_name) {
            self.type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| info.type_name.clone())
        } else {
            None
        };

        if let Some(type_name) = type_name {
            if let Some(schema) = self.type_tracker.schema_registry().get(&type_name) {
                if let Some(field) = schema.get_field(field_name) {
                    return match field.field_type {
                        FieldType::F64 => OpCode::LoadColF64,
                        FieldType::I64 | FieldType::Timestamp => OpCode::LoadColI64,
                        FieldType::Bool => OpCode::LoadColBool,
                        FieldType::String => OpCode::LoadColStr,
                        _ => OpCode::LoadColF64, // default
                    };
                }
            }
        }
        OpCode::LoadColF64 // default
    }

    /// Resolve the NumericType for a RowView field (used for typed opcode emission).
    pub(super) fn resolve_row_view_field_numeric_type(
        &self,
        var_name: &str,
        field_name: &str,
    ) -> Option<crate::type_tracking::NumericType> {
        use crate::type_tracking::NumericType;
        use shape_runtime::type_schema::FieldType;

        let type_name = if let Some(local_idx) = self.resolve_local(var_name) {
            self.type_tracker
                .get_local_type(local_idx)
                .and_then(|info| info.type_name.clone())
        } else if let Some(&binding_idx) = self.module_bindings.get(var_name) {
            self.type_tracker
                .get_binding_type(binding_idx)
                .and_then(|info| info.type_name.clone())
        } else {
            None
        };

        if let Some(type_name) = type_name {
            if let Some(schema) = self.type_tracker.schema_registry().get(&type_name) {
                if let Some(field) = schema.get_field(field_name) {
                    return match field.field_type {
                        FieldType::F64 => Some(NumericType::Number),
                        FieldType::I64 | FieldType::Timestamp => Some(NumericType::Int),
                        FieldType::Decimal => Some(NumericType::Decimal),
                        _ => None,
                    };
                }
            }
        }
        None
    }

    /// Convert a TypeAnnotation to a FieldType for TypeSchema registration
    pub(super) fn type_annotation_to_field_type(
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> shape_runtime::type_schema::FieldType {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_schema::FieldType;
        match ann {
            TypeAnnotation::Basic(s) => match s.as_str() {
                "number" | "float" | "f64" | "f32" => FieldType::F64,
                "i8" => FieldType::I8,
                "u8" => FieldType::U8,
                "i16" => FieldType::I16,
                "u16" => FieldType::U16,
                "i32" => FieldType::I32,
                "u32" => FieldType::U32,
                "u64" => FieldType::U64,
                "int" | "i64" | "integer" | "isize" | "usize" | "byte" | "char" => FieldType::I64,
                "string" | "str" => FieldType::String,
                "decimal" => FieldType::Decimal,
                "bool" | "boolean" => FieldType::Bool,
                "timestamp" => FieldType::Timestamp,
                // Non-primitive type names (e.g. "Server", "Inner") are nested
                // object references.  The parser emits Basic for `ident` matches
                // inside `basic_type`, so treat unknown names as Object references
                // to enable typed field access on nested structs.
                other => FieldType::Object(other.to_string()),
            },
            TypeAnnotation::Reference(s) => FieldType::Object(s.to_string()),
            TypeAnnotation::Array(inner) => {
                FieldType::Array(Box::new(Self::type_annotation_to_field_type(inner)))
            }
            TypeAnnotation::Generic { name, .. } => match name.as_str() {
                // Generic containers that need NaN boxing
                "HashMap" | "Map" | "Result" | "Option" | "Set" => FieldType::Any,
                // User-defined generic structs — preserve the type name
                other => FieldType::Object(other.to_string()),
            },
            _ => FieldType::Any,
        }
    }

    /// Evaluate an annotation argument expression to a string representation.
    /// Only handles compile-time evaluable expressions (literals).
    pub(super) fn eval_annotation_arg(expr: &shape_ast::ast::Expr) -> Option<String> {
        use shape_ast::ast::{Expr, Literal};
        match expr {
            Expr::Literal(Literal::String(s), _) => Some(s.clone()),
            Expr::Literal(Literal::Number(n), _) => Some(n.to_string()),
            Expr::Literal(Literal::Int(i), _) => Some(i.to_string()),
            Expr::Literal(Literal::Bool(b), _) => Some(b.to_string()),
            _ => None,
        }
    }

    /// Get the schema ID for a `Table<T>` type annotation, if applicable.
    ///
    /// Returns `Some(schema_id)` if the annotation is `Table<T>` and `T` is a registered
    /// TypeSchema. Returns `None` otherwise.
    pub(super) fn get_table_schema_id(
        &self,
        type_ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<u16> {
        use shape_ast::ast::TypeAnnotation;
        if let TypeAnnotation::Generic { name, args } = type_ann {
            if name == "Table" && args.len() == 1 {
                let inner_name = match &args[0] {
                    TypeAnnotation::Basic(t) => Some(t.as_str()),
                    TypeAnnotation::Reference(t) => Some(t.as_str()),
                    _ => None,
                };
                if let Some(type_name) = inner_name {
                    return self
                        .type_tracker
                        .schema_registry()
                        .get(type_name)
                        .map(|s| s.id as u16);
                }
            }
        }
        None
    }

    // ===== Drop scope management =====

    /// Push a new drop scope. Must be paired with pop_drop_scope().
    pub(super) fn push_drop_scope(&mut self) {
        self.drop_locals.push(Vec::new());
        // Phase V1.1C: parallel ownership-drop scope (heap-ref locals that
        // need a `DropLocal` opcode at scope exit when the flag is on).
        // Pushed in lockstep regardless of flag state; only the emission in
        // `pop_drop_scope` is gated.
        self.ownership_drop_locals.push(Vec::new());
    }

    /// Pop the current drop scope, emitting DropCall instructions for all
    /// tracked locals in reverse order.
    pub(super) fn pop_drop_scope(&mut self) -> Result<()> {
        // Phase V1.1C: when `SHAPE_V2_OWNERSHIP_MOVES` is on, emit an
        // ownership-aware `DropLocal` for each heap-ref local declared in
        // this scope — in reverse order — *before* the legacy `DropCall`
        // trait-invocation pass. Conservative: with the current
        // always-Clone read policy no local is ever poisoned by a Move, so
        // every tracked `UniqueHeap` local is still live at scope exit.
        // TODO: once MIR last-use information is threaded into read-side
        // emission (so `MoveLocal` can be emitted on terminal reads), the
        // tracker here must be updated to skip drops for moved-out slots.
        let ownership_locals = self.ownership_drop_locals.pop().unwrap_or_default();
        if ownership_moves_enabled() {
            for local_idx in ownership_locals.into_iter().rev() {
                // Phase V1.1C fix: `BoxLocal` may have converted the slot
                // to a `SharedCell` wrapper between declaration time
                // (when `track_ownership_drop_local` captured the slot)
                // and scope exit. `DropLocal` poisons the slot with
                // `0u64`, which breaks the `LoadLocal` + `DropCall` pass
                // that immediately follows (it would no longer see the
                // Arc-backed cell). The pre-existing refcount release
                // path in the `DropCall` pass handles boxed slots
                // correctly, so skip here.
                if self.slot_is_boxed(local_idx) {
                    continue;
                }
                self.emit(Instruction::new(
                    OpCode::DropLocal,
                    Some(Operand::Local(local_idx)),
                ));
            }
        }
        // Emit DropCall for each tracked local in reverse order
        if let Some(locals) = self.drop_locals.pop() {
            for (local_idx, is_async) in locals.into_iter().rev() {
                self.emit_drop_call_for_local(local_idx, is_async);
            }
        }
        Ok(())
    }

    /// Phase V1.1C: record a local slot as needing an ownership-aware
    /// `DropLocal` at the next `pop_drop_scope`. Called from the compiler's
    /// variable-declaration path when the slot's MIR storage class is
    /// `UniqueHeap` (i.e. owned heap allocation that the ownership-moves
    /// runtime would otherwise leak). No-op when no drop scope is active.
    pub(super) fn track_ownership_drop_local(&mut self, local_idx: u16) {
        if let Some(scope) = self.ownership_drop_locals.last_mut() {
            scope.push(local_idx);
        }
    }

    /// Phase V1.1C: true when the slot's storage hint matches an
    /// inline-scalar native type (int / number / bool / sized integers).
    /// Used to separate Box-promoted heap values from zero-cost inline
    /// values both for `DropLocal` emission at scope exit and for
    /// `CloneLocal` emission on reads.
    pub(super) fn slot_has_inline_scalar_hint(&self, local_idx: u16) -> bool {
        let hint = self
            .type_tracker
            .get_local_type(local_idx)
            .map(|info| info.storage_hint)
            .unwrap_or(StorageHint::Unknown);
        hint.is_numeric_family() || matches!(hint, StorageHint::Bool)
    }

    /// Phase V1.1C: true when the slot is backed by an owned heap
    /// allocation per the Phase 4 contract — either `UniqueHeap` storage
    /// class, or `Direct` storage class combined with a non-scalar
    /// storage hint. The latter captures the Box-promoted heap path
    /// (strings, arrays, hashmaps, typed objects) handed to the slot by
    /// `PromoteToOwned`. Inline scalars on Direct slots and every other
    /// storage class (SharedCow, Reference, LocalMutablePtr, Deferred)
    /// are excluded.
    pub(super) fn slot_is_heap_backed_owned(&self, local_idx: u16) -> bool {
        use crate::type_tracking::BindingStorageClass;
        match self.mir_storage_class_for_slot(local_idx) {
            Some(BindingStorageClass::UniqueHeap) => true,
            Some(BindingStorageClass::Direct) => !self.slot_has_inline_scalar_hint(local_idx),
            _ => false,
        }
    }

    /// Phase V1.1C: decide whether the newly-declared local slot should
    /// receive a `DropLocal` opcode at scope exit (when the ownership-moves
    /// flag is on). The slot needs a drop iff it is heap-backed — either
    /// `UniqueHeap` storage class (owned Box allocation), or `Direct`
    /// storage class combined with a `let`/`const` binding of a heap type
    /// (the `PromoteToOwned` emission converts these to Box). Inline
    /// scalars (`int`, `number`, `bool`, etc.) own no heap resource and
    /// are skipped per `docs/ownership-aware-runtime-v2.md` §Phase 1.
    /// `var` bindings of heap type on Direct storage are skipped too —
    /// the Arc refcount path already releases them.
    pub(super) fn binding_slot_needs_ownership_drop(
        &self,
        local_idx: u16,
        var_kind: shape_ast::ast::VarKind,
    ) -> bool {
        use crate::type_tracking::BindingStorageClass;
        // Phase V1.1C fix: if the slot has been SharedCell-wrapped by a prior
        // `BoxLocal` (closure capture path), the legacy Arc-refcount release
        // path in `pop_drop_scope` / `emit_drops_for_early_exit` handles the
        // release. Emitting `DropLocal` here poisons the slot to `0u64`
        // which breaks the auto-unwrap in `LoadLocal` / `LoadClosure` for any
        // subsequent read (e.g. compiler-injected reads like `LoadLocal` +
        // `DropCall` pairs that immediately follow the `DropLocal`).
        if self.slot_is_boxed(local_idx) {
            return false;
        }
        match self.mir_storage_class_for_slot(local_idx) {
            Some(BindingStorageClass::UniqueHeap) => true,
            Some(BindingStorageClass::Direct) => {
                // Only let / const are Box-promoted by the PromoteToOwned
                // rule (see statements.rs §Phase 3/4). var bindings with
                // Direct storage stay Arc-wrapped and release via the
                // existing refcount path.
                matches!(
                    var_kind,
                    shape_ast::ast::VarKind::Let | shape_ast::ast::VarKind::Const
                ) && !self.slot_has_inline_scalar_hint(local_idx)
            }
            _ => false,
        }
    }

    /// Emit a single LoadLocal + DropCall pair for a local variable.
    /// The type name is resolved from the type tracker and encoded as a
    /// Property operand so the executor can look up `TypeName::drop`.
    fn emit_drop_call_for_local(&mut self, local_idx: u16, is_async: bool) {
        let type_name_opt = self
            .type_tracker
            .get_local_type(local_idx)
            .and_then(|info| info.type_name.clone());
        self.emit(Instruction::new(
            OpCode::LoadLocal,
            Some(Operand::Local(local_idx)),
        ));
        let opcode = if is_async {
            OpCode::DropCallAsync
        } else {
            OpCode::DropCall
        };
        if let Some(type_name) = type_name_opt {
            let str_idx = self.program.add_string(type_name);
            self.emit(Instruction::new(opcode, Some(Operand::Property(str_idx))));
        } else {
            self.emit(Instruction::simple(opcode));
        }
    }

    /// Emit a single LoadModuleBinding + DropCall pair for a module binding.
    /// Similar to `emit_drop_call_for_local` but loads from module bindings.
    pub(super) fn emit_drop_call_for_module_binding(&mut self, binding_idx: u16, is_async: bool) {
        let type_name_opt = self
            .type_tracker
            .get_binding_type(binding_idx)
            .and_then(|info| info.type_name.clone());
        self.emit(Instruction::new(
            OpCode::LoadModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));
        let opcode = if is_async {
            OpCode::DropCallAsync
        } else {
            OpCode::DropCall
        };
        if let Some(type_name) = type_name_opt {
            let str_idx = self.program.add_string(type_name);
            self.emit(Instruction::new(opcode, Some(Operand::Property(str_idx))));
        } else {
            self.emit(Instruction::simple(opcode));
        }
    }

    /// Track a local variable as needing Drop at scope exit.
    pub(super) fn track_drop_local(&mut self, local_idx: u16, is_async: bool) {
        if let Some(scope) = self.drop_locals.last_mut() {
            scope.push((local_idx, is_async));
        }
    }

    /// Resolve the DropKind for a local variable's type.
    /// Returns None if the type is unknown or has no Drop impl.
    pub(super) fn local_drop_kind(&self, local_idx: u16) -> Option<DropKind> {
        let type_name = self
            .type_tracker
            .get_local_type(local_idx)
            .and_then(|info| info.type_name.as_ref())?;
        self.drop_type_info.get(type_name).copied()
    }

    /// Resolve DropKind from a type annotation.
    pub(super) fn annotation_drop_kind(&self, type_ann: &TypeAnnotation) -> Option<DropKind> {
        let type_name = Self::tracked_type_name_from_annotation(type_ann)?;
        self.drop_type_info.get(&type_name).copied()
    }

    /// Emit drops for all scopes being exited (used by return/break/continue).
    /// `scopes_to_exit` is the number of drop scopes to emit drops for.
    pub(super) fn emit_drops_for_early_exit(&mut self, scopes_to_exit: usize) -> Result<()> {
        let total = self.drop_locals.len();
        if scopes_to_exit > total {
            return Ok(());
        }
        // Phase V1.1C: when the ownership-moves flag is on, also emit a
        // `DropLocal` for each heap-ref (UniqueHeap) local tracked in the
        // scopes being exited, innermost-first / reverse declaration order.
        // Flag off: no ownership drops — byte-identical to pre-V1.1C.
        // Note: early-exit drops are inserted at the jump/return site; the
        // scope stack itself is popped later by the enclosing block, so we
        // must *not* consume `self.ownership_drop_locals` here. Cloning
        // matches the `self.drop_locals` treatment below.
        if ownership_moves_enabled() {
            let ownership_total = self.ownership_drop_locals.len();
            if scopes_to_exit <= ownership_total {
                let mut ownership_scopes: Vec<Vec<u16>> = Vec::new();
                for i in (ownership_total - scopes_to_exit..ownership_total).rev() {
                    let locals = self
                        .ownership_drop_locals
                        .get(i)
                        .cloned()
                        .unwrap_or_default();
                    ownership_scopes.push(locals);
                }
                for locals in ownership_scopes {
                    for local_idx in locals.into_iter().rev() {
                        // Phase V1.1C fix: skip `DropLocal` emission for
                        // slots that were SharedCell-wrapped by a prior
                        // `BoxLocal`. See the companion comment in
                        // `pop_drop_scope` for the rationale.
                        if self.slot_is_boxed(local_idx) {
                            continue;
                        }
                        self.emit(Instruction::new(
                            OpCode::DropLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    }
                }
            }
        }
        // Collect locals from scopes being exited (innermost first)
        let mut scopes: Vec<Vec<(u16, bool)>> = Vec::new();
        for i in (total - scopes_to_exit..total).rev() {
            let locals = self.drop_locals.get(i).cloned().unwrap_or_default();
            scopes.push(locals);
        }
        // Now emit DropCall instructions
        for locals in scopes {
            for (local_idx, is_async) in locals.into_iter().rev() {
                self.emit_drop_call_for_local(local_idx, is_async);
            }
        }
        Ok(())
    }

    /// Track a module binding as needing Drop at program exit.
    pub(super) fn track_drop_module_binding(&mut self, binding_idx: u16, is_async: bool) {
        self.drop_module_bindings.push((binding_idx, is_async));
    }
}


#[cfg(test)]
mod tests {
    use super::super::BytecodeCompiler;
    use crate::compiler::ParamPassMode;
    use crate::type_tracking::BindingStorageClass;
    use shape_ast::ast::{Expr, Span, TypeAnnotation};
    use shape_runtime::type_schema::FieldType;

    #[test]
    fn test_type_annotation_to_field_type_array_recursive() {
        let ann = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("int".to_string())));
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Array(Box::new(FieldType::I64)));
    }

    #[test]
    fn test_type_annotation_to_field_type_optional() {
        let ann = TypeAnnotation::Generic {
            name: "Option".into(),
            args: vec![TypeAnnotation::Basic("int".to_string())],
        };
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Any);
    }

    #[test]
    fn test_type_annotation_to_field_type_generic_hashmap() {
        let ann = TypeAnnotation::Generic {
            name: "HashMap".into(),
            args: vec![
                TypeAnnotation::Basic("string".to_string()),
                TypeAnnotation::Basic("int".to_string()),
            ],
        };
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Any);
    }

    #[test]
    fn test_type_annotation_to_field_type_generic_user_struct() {
        let ann = TypeAnnotation::Generic {
            name: "MyContainer".into(),
            args: vec![TypeAnnotation::Basic("string".to_string())],
        };
        let ft = BytecodeCompiler::type_annotation_to_field_type(&ann);
        assert_eq!(ft, FieldType::Object("MyContainer".to_string()));
    }

    #[test]
    fn test_flexible_storage_promotion_is_monotonic() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        compiler.promote_flexible_binding_storage_for_slot(
            slot,
            true,
            BindingStorageClass::UniqueHeap,
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );

        compiler.promote_flexible_binding_storage_for_slot(slot, true, BindingStorageClass::Direct);
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );

        compiler.promote_flexible_binding_storage_for_slot(
            slot,
            true,
            BindingStorageClass::SharedCow,
        );
        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::SharedCow)
        );
    }

    #[test]
    fn test_escape_planner_marks_array_element_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        let expr = Expr::Array(
            vec![Expr::Identifier("value".to_string(), Span::DUMMY)],
            Span::DUMMY,
        );
        compiler.plan_flexible_binding_escape_from_expr(&expr);

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_escape_planner_marks_if_branch_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        let expr = Expr::If(
            Box::new(shape_ast::ast::IfExpr {
                condition: Box::new(Expr::Literal(
                    shape_ast::ast::Literal::Bool(true),
                    Span::DUMMY,
                )),
                then_branch: Box::new(Expr::Identifier("value".to_string(), Span::DUMMY)),
                else_branch: None,
            }),
            Span::DUMMY,
        );
        compiler.plan_flexible_binding_escape_from_expr(&expr);

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_escape_planner_marks_async_let_rhs_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        let expr = Expr::AsyncLet(
            Box::new(shape_ast::ast::AsyncLetExpr {
                name: "task".to_string(),
                expr: Box::new(Expr::Identifier("value".to_string(), Span::DUMMY)),
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        );
        compiler.plan_flexible_binding_escape_from_expr(&expr);

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_call_args_mark_by_value_identifier_as_unique_heap() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        compiler
            .compile_call_args(&[Expr::Identifier("value".to_string(), Span::DUMMY)], None)
            .expect("call args should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::UniqueHeap)
        );
    }

    #[test]
    fn test_call_args_leave_by_ref_identifier_storage_unchanged() {
        let mut compiler = BytecodeCompiler::new();
        compiler.push_scope();
        let slot = compiler.declare_local("value").expect("declare local");
        compiler.type_tracker.set_local_binding_semantics(
            slot,
            BytecodeCompiler::binding_semantics_for_ownership_class(
                crate::type_tracking::BindingOwnershipClass::Flexible,
            ),
        );

        compiler
            .compile_call_args(
                &[Expr::Identifier("value".to_string(), Span::DUMMY)],
                Some(&[ParamPassMode::ByRefShared]),
            )
            .expect("reference call args should compile");

        assert_eq!(
            compiler
                .type_tracker
                .get_local_binding_semantics(slot)
                .map(|semantics| semantics.storage_class),
            Some(BindingStorageClass::Deferred)
        );
    }

    // ========================================================================
    // Phase V3.1: emit_binary_op shim tests
    // ========================================================================

    /// Helper: read the opcode of the last instruction emitted to the
    /// compiler's program. Used by the `emit_binary_op` tests below.
    fn last_emitted_opcode(compiler: &BytecodeCompiler) -> crate::bytecode::OpCode {
        compiler
            .program
            .instructions
            .last()
            .expect("compiler program is empty")
            .opcode
    }

    #[test]
    fn emit_binary_op_int_int_add_emits_add_int() {
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use crate::type_tracking::NumericType;
        use shape_ast::ast::BinaryOp;

        let mut compiler = BytecodeCompiler::new();
        let handled = emit_binary_op(
            &mut compiler,
            BinaryOp::Add,
            BinOperandKind::Numeric(NumericType::Int),
            BinOperandKind::Numeric(NumericType::Int),
        )
        .expect("emit_binary_op should succeed");
        assert!(handled, "Add should be a handled op");
        assert_eq!(last_emitted_opcode(&compiler), OpCode::AddInt);
    }

    #[test]
    fn emit_binary_op_number_number_add_emits_add_number() {
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use crate::type_tracking::NumericType;
        use shape_ast::ast::BinaryOp;

        let mut compiler = BytecodeCompiler::new();
        let handled = emit_binary_op(
            &mut compiler,
            BinaryOp::Add,
            BinOperandKind::Numeric(NumericType::Number),
            BinOperandKind::Numeric(NumericType::Number),
        )
        .expect("emit_binary_op should succeed");
        assert!(handled);
        assert_eq!(last_emitted_opcode(&compiler), OpCode::AddNumber);
    }

    #[test]
    fn emit_binary_op_number_number_mul_emits_mul_number() {
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use crate::type_tracking::NumericType;
        use shape_ast::ast::BinaryOp;

        let mut compiler = BytecodeCompiler::new();
        emit_binary_op(
            &mut compiler,
            BinaryOp::Mul,
            BinOperandKind::Numeric(NumericType::Number),
            BinOperandKind::Numeric(NumericType::Number),
        )
        .expect("emit_binary_op should succeed");
        assert_eq!(last_emitted_opcode(&compiler), OpCode::MulNumber);
    }

    #[test]
    fn emit_binary_op_int_int_cmp_emits_typed_cmp_opcodes() {
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use crate::type_tracking::NumericType;
        use shape_ast::ast::BinaryOp;

        let cases = [
            (BinaryOp::Greater, OpCode::GtInt),
            (BinaryOp::Less, OpCode::LtInt),
            (BinaryOp::GreaterEq, OpCode::GteInt),
            (BinaryOp::LessEq, OpCode::LteInt),
            (BinaryOp::Equal, OpCode::EqInt),
            (BinaryOp::NotEqual, OpCode::NeqInt),
        ];
        for (op, expected) in cases {
            let mut compiler = BytecodeCompiler::new();
            emit_binary_op(
                &mut compiler,
                op,
                BinOperandKind::Numeric(NumericType::Int),
                BinOperandKind::Numeric(NumericType::Int),
            )
            .expect("emit_binary_op should succeed");
            assert_eq!(
                last_emitted_opcode(&compiler),
                expected,
                "wrong opcode for Int {:?}",
                op
            );
        }
    }

    #[test]
    fn emit_binary_op_decimal_decimal_emits_typed_decimal_opcodes() {
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use crate::type_tracking::NumericType;
        use shape_ast::ast::BinaryOp;

        let cases = [
            (BinaryOp::Add, OpCode::AddDecimal),
            (BinaryOp::Sub, OpCode::SubDecimal),
            (BinaryOp::Mul, OpCode::MulDecimal),
            (BinaryOp::Div, OpCode::DivDecimal),
            (BinaryOp::Equal, OpCode::EqDecimal),
        ];
        for (op, expected) in cases {
            let mut compiler = BytecodeCompiler::new();
            emit_binary_op(
                &mut compiler,
                op,
                BinOperandKind::Numeric(NumericType::Decimal),
                BinOperandKind::Numeric(NumericType::Decimal),
            )
            .expect("emit_binary_op should succeed");
            assert_eq!(
                last_emitted_opcode(&compiler),
                expected,
                "wrong opcode for Decimal {:?}",
                op
            );
        }
    }

    #[test]
    fn emit_binary_op_lhs_known_rhs_unknown_emits_dynamic() {
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use crate::type_tracking::NumericType;
        use shape_ast::ast::BinaryOp;

        let mut compiler = BytecodeCompiler::new();
        emit_binary_op(
            &mut compiler,
            BinaryOp::Add,
            BinOperandKind::Numeric(NumericType::Int),
            BinOperandKind::Unknown,
        )
        .expect("emit_binary_op should succeed");
        assert_eq!(last_emitted_opcode(&compiler), OpCode::AddDynamic);
    }

    #[test]
    fn emit_binary_op_both_unknown_emits_dynamic() {
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use shape_ast::ast::BinaryOp;

        let mut compiler = BytecodeCompiler::new();
        emit_binary_op(
            &mut compiler,
            BinaryOp::Add,
            BinOperandKind::Unknown,
            BinOperandKind::Unknown,
        )
        .expect("emit_binary_op should succeed");
        assert_eq!(last_emitted_opcode(&compiler), OpCode::AddDynamic);

        let mut compiler = BytecodeCompiler::new();
        emit_binary_op(
            &mut compiler,
            BinaryOp::Equal,
            BinOperandKind::Unknown,
            BinOperandKind::Unknown,
        )
        .expect("emit_binary_op should succeed");
        assert_eq!(last_emitted_opcode(&compiler), OpCode::EqDynamic);
    }

    #[test]
    fn emit_binary_op_mismatched_numeric_types_emits_dynamic() {
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use crate::type_tracking::NumericType;
        use shape_ast::ast::BinaryOp;

        // Int + Number with no coercion available: shim falls back to Dynamic.
        let mut compiler = BytecodeCompiler::new();
        emit_binary_op(
            &mut compiler,
            BinaryOp::Add,
            BinOperandKind::Numeric(NumericType::Int),
            BinOperandKind::Numeric(NumericType::Number),
        )
        .expect("emit_binary_op should succeed");
        assert_eq!(last_emitted_opcode(&compiler), OpCode::AddDynamic);
    }

    #[test]
    fn emit_binary_op_string_string_add_emits_string_concat_typed() {
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use shape_ast::ast::BinaryOp;

        let mut compiler = BytecodeCompiler::new();
        emit_binary_op(
            &mut compiler,
            BinaryOp::Add,
            BinOperandKind::String,
            BinOperandKind::String,
        )
        .expect("emit_binary_op should succeed");
        assert_eq!(last_emitted_opcode(&compiler), OpCode::StringConcatTyped);
    }

    #[test]
    fn emit_binary_op_bool_bool_equal_falls_back_to_dynamic() {
        // V3.1: no typed EqBool opcode exists yet. Bool equality must fall
        // back to EqDynamic. This test pins that contract so a future
        // EqBool addition is a deliberate, reviewable change.
        use crate::bytecode::OpCode;
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use shape_ast::ast::BinaryOp;

        let mut compiler = BytecodeCompiler::new();
        emit_binary_op(
            &mut compiler,
            BinaryOp::Equal,
            BinOperandKind::Bool,
            BinOperandKind::Bool,
        )
        .expect("emit_binary_op should succeed");
        assert_eq!(last_emitted_opcode(&compiler), OpCode::EqDynamic);
    }

    #[test]
    fn emit_binary_op_returns_false_for_unsupported_ops() {
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use shape_ast::ast::BinaryOp;

        // And/Or/BitOps/NullCoalesce/ErrorContext/Pipe/Fuzzy* are NOT the
        // shim's responsibility — they return Ok(false) so the caller routes
        // them through their dedicated paths.
        let unsupported = [
            BinaryOp::And,
            BinaryOp::Or,
            BinaryOp::BitAnd,
            BinaryOp::BitOr,
            BinaryOp::BitXor,
            BinaryOp::BitShl,
            BinaryOp::BitShr,
            BinaryOp::NullCoalesce,
            BinaryOp::ErrorContext,
            BinaryOp::Pipe,
            BinaryOp::FuzzyEqual,
            BinaryOp::FuzzyGreater,
            BinaryOp::FuzzyLess,
        ];
        for op in unsupported {
            let mut compiler = BytecodeCompiler::new();
            let handled = emit_binary_op(
                &mut compiler,
                op,
                BinOperandKind::Unknown,
                BinOperandKind::Unknown,
            )
            .expect("emit_binary_op should succeed");
            assert!(!handled, "{:?} should be unhandled (Ok(false))", op);
            assert!(
                compiler.program.instructions.is_empty(),
                "{:?} must not emit any instruction on refusal",
                op
            );
        }
    }

    #[test]
    fn emit_binary_op_preserves_numeric_hint_for_arithmetic_only() {
        use crate::compiler::helpers::{BinOperandKind, emit_binary_op};
        use crate::type_tracking::NumericType;
        use shape_ast::ast::BinaryOp;

        // Arithmetic: typed-numeric emission should propagate
        // last_expr_numeric_type so downstream numeric hints still flow.
        let mut compiler = BytecodeCompiler::new();
        emit_binary_op(
            &mut compiler,
            BinaryOp::Add,
            BinOperandKind::Numeric(NumericType::Int),
            BinOperandKind::Numeric(NumericType::Int),
        )
        .expect("ok");
        assert_eq!(compiler.last_expr_numeric_type, Some(NumericType::Int));

        // Comparison: result is bool, so numeric hint must be cleared.
        let mut compiler = BytecodeCompiler::new();
        compiler.last_expr_numeric_type = Some(NumericType::Number);
        emit_binary_op(
            &mut compiler,
            BinaryOp::Less,
            BinOperandKind::Numeric(NumericType::Int),
            BinOperandKind::Numeric(NumericType::Int),
        )
        .expect("ok");
        assert_eq!(compiler.last_expr_numeric_type, None);
    }

    #[test]
    fn from_numeric_maps_none_to_unknown() {
        use crate::compiler::helpers::BinOperandKind;
        use crate::type_tracking::NumericType;

        assert_eq!(
            BinOperandKind::from_numeric(None),
            BinOperandKind::Unknown
        );
        assert_eq!(
            BinOperandKind::from_numeric(Some(NumericType::Int)),
            BinOperandKind::Numeric(NumericType::Int)
        );
        assert_eq!(
            BinOperandKind::from_numeric(Some(NumericType::Number)),
            BinOperandKind::Numeric(NumericType::Number)
        );
    }
}
