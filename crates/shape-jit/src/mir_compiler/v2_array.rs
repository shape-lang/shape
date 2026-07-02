//! Inline typed array codegen for the v2 runtime.
//!
//! Emits Cranelift IR for direct-memory-access typed array operations
//! with zero FFI overhead and zero NaN-boxing.
//!
//! ## TypedArrayHeader layout (at the array pointer)
//!
//! ```text
//! offset  0: refcount  (u32)
//! offset  4: kind      (u16)
//! offset  6: elem_type (u8)
//! offset  7: _pad      (u8)
//! offset  8: data      (*mut T)  — pointer to contiguous element buffer
//! offset 16: len       (u32)
//! offset 20: cap       (u32)
//! ```
//!
//! ## Element sizes
//!
//! | NativeKind  | Cranelift type | Size (bytes) |
//! |-----------|---------------|--------------|
//! | Float64   | F64           | 8            |
//! | Int64     | I64           | 8            |
//! | Int32     | I32           | 4            |
//! | Int16     | I16           | 2            |
//! | Int8/Bool | I8            | 1            |

use cranelift::prelude::*;
use shape_value::v2::ConcreteType;
use shape_value::HeapKind;
use shape_vm::mir::types::{Operand, Place, SlotId};
use shape_vm::type_tracking::NativeKind;
use std::collections::HashMap;

use super::types::is_v2_typed_array_slot;
use super::MirToIR;

// ── TypedArrayHeader field offsets ───────────────────────────────────────────

/// Offset of the `data` pointer field (`*mut T`) inside `TypedArrayHeader`.
const DATA_PTR_OFFSET: i32 = 8;

/// Offset of the `len` field (`u32`) inside `TypedArrayHeader`.
const LEN_OFFSET: i32 = 16;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Exhaustive HeapKind sink for pointer-width typed-array element storage.
#[inline]
fn heap_ptr_element_type_info(heap_kind: HeapKind) -> (types::Type, i64) {
    match heap_kind {
        HeapKind::String
        | HeapKind::TypedObject
        | HeapKind::Closure
        | HeapKind::Decimal
        | HeapKind::BigInt
        | HeapKind::DataTable
        | HeapKind::Future
        | HeapKind::TaskGroup
        | HeapKind::TypedArray
        | HeapKind::Temporal
        | HeapKind::TableView
        | HeapKind::Content
        | HeapKind::Instant
        | HeapKind::IoHandle
        | HeapKind::NativeScalar
        | HeapKind::NativeView
        | HeapKind::Char
        | HeapKind::HashMap
        | HeapKind::FilterExpr
        | HeapKind::Reference
        | HeapKind::SharedCell
        | HeapKind::HashSet
        | HeapKind::Iterator
        | HeapKind::Deque
        | HeapKind::Channel
        | HeapKind::PriorityQueue
        | HeapKind::Range
        | HeapKind::Result
        | HeapKind::Option
        | HeapKind::TraitObject
        | HeapKind::Mutex
        | HeapKind::Atomic
        | HeapKind::Lazy
        | HeapKind::ModuleFn
        | HeapKind::Matrix
        | HeapKind::MatrixSlice => (types::I64, 8),
    }
}

/// Return the (Cranelift IR type, element byte size) for a given `NativeKind`.
///
/// Panics on slot kinds that do not map to a scalar element type (e.g.
/// `String`, `Dynamic`, `Unknown`).
fn elem_type_info(kind: NativeKind) -> (types::Type, i64) {
    match kind {
        NativeKind::Float64 | NativeKind::NullableFloat64 => (types::F64, 8),
        NativeKind::Int64
        | NativeKind::NullableInt64
        | NativeKind::UInt64
        | NativeKind::NullableUInt64 => (types::I64, 8),
        NativeKind::IntSize
        | NativeKind::NullableIntSize
        | NativeKind::UIntSize
        | NativeKind::NullableUIntSize => {
            // Pointer-sized — 8 bytes on 64-bit targets.
            (types::I64, 8)
        }
        NativeKind::Int32
        | NativeKind::NullableInt32
        | NativeKind::UInt32
        | NativeKind::NullableUInt32 => (types::I32, 4),
        NativeKind::Int16
        | NativeKind::NullableInt16
        | NativeKind::UInt16
        | NativeKind::NullableUInt16 => (types::I16, 2),
        NativeKind::Int8
        | NativeKind::NullableInt8
        | NativeKind::UInt8
        | NativeKind::NullableUInt8 => (types::I8, 1),
        NativeKind::Bool => (types::I8, 1),
        // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18) —
        // 8-byte raw pointer carrier (`*const TypedObjectStorage`). Same shape as
        // NativeKind::StringV2 / DecimalV2 heap-pointer carriers.
        NativeKind::StringV2 | NativeKind::DecimalV2 => (types::I64, 8),
        NativeKind::Ptr(heap_kind) => heap_ptr_element_type_info(heap_kind),
        other => panic!("v2_array: unsupported element NativeKind: {:?}", other),
    }
}

// ── Implementation ──────────────────────────────────────────────────────────

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Look up the `ConcreteType` (if any) the bytecode compiler recorded for
    /// a local slot.
    #[allow(dead_code)]
    pub(crate) fn concrete_type_for_slot(&self, slot: SlotId) -> Option<&ConcreteType> {
        let ct = self.concrete_types.get(slot.0 as usize)?;
        if matches!(ct, ConcreteType::Void) {
            None
        } else {
            Some(ct)
        }
    }

    /// `true` when the indexing base is a `string` receiver (`s[i]`).
    ///
    /// String indexing is NOT an array element load: the VM lowers `s[i]`
    /// through the `GetProp` String arm (`dispatch_get_prop`), which
    /// allocates a real 1-char `NativeKind::String` (`Arc<String>`) —
    /// byte-identical to `op_string_char_at` (typed_access.rs). The JIT's
    /// `Place::Index` arms, in contrast, only model ARRAY element access
    /// (`inline_array_get` / the v2 typed-array fast path), so a string
    /// base falls through to `inline_array_get`, which reinterprets the
    /// `Arc<String>` heap pointer as a v1 array layout (data@+0/len@+8),
    /// reads a wild "element pointer", and then a downstream retain
    /// (`Rvalue::Clone` / Copy disposition) dereferences it → SIGSEGV
    /// (`jit_arc_retain` on garbage bits). The defended-against repro is
    /// `s[i] == "x"` (book `fundamentals/strings.mdx`: "Index chars via
    /// `s[i]`"). There is no JIT string-char producer wired today, so the
    /// principled response per CLAUDE.md "surface-and-stop, not force" is
    /// to fail JIT compilation here and fall through to the interpreter,
    /// whose String-arm path is correct (verified: `--mode vm` returns the
    /// right result). Returns `true` only when the base is statically stamped
    /// as a string, either through the per-slot `NativeKind` track or the
    /// older concrete-type side table.
    pub(crate) fn index_base_is_string(&self, place: &Place) -> bool {
        if matches!(self.place_native_kind(place), Some(NativeKind::String)) {
            return true;
        }
        match place {
            Place::Local(s) => matches!(
                self.concrete_types.get(s.0 as usize),
                Some(ConcreteType::String)
            ),
            _ => false,
        }
    }

    /// If the place is known to hold a v2 `Array<T>` whose element type
    /// is a scalar primitive, return the matching element `NativeKind`.
    /// Returns `None` for non-array places, arrays of non-scalar
    /// elements, or unresolved types — caller falls back to legacy path.
    ///
    /// Two base shapes are recognised:
    ///
    /// - `Place::Local(slot)` — the slot's `ConcreteType` (threaded from
    ///   `BytecodeProgram.top_level_local_concrete_types` per ADR-006
    ///   §2.7.5, W12-top-level-concrete-types-conduit close 2026-05-12)
    ///   is inspected via `is_v2_typed_array_slot`.
    ///
    /// - `Place::Field(_, field_idx)` — γ-CP5 7a (jit-typedarray-ptr):
    ///   a struct field declared `Array<T>` carries a v2 `TypedArray<T>`
    ///   pointer in its 8-byte slot. The element kind comes from the
    ///   schema-derived `field_array_elem_kinds` map (stamped at
    ///   `populate_field_byte_offsets_from_schemas` time). Without this
    ///   arm `b.items[i]` (field-projected array base) fell through to
    ///   the legacy `inline_array_get` which uses the v1 array layout
    ///   (data@+0/len@+8) and read the wrong element offset for the v2
    ///   `TypedArray<T>` (data@8/len@16) actually stored in the field.
    pub(crate) fn v2_typed_array_elem_kind(&self, place: &Place) -> Option<NativeKind> {
        match place {
            Place::Local(s) => is_v2_typed_array_slot(&self.concrete_types, s.0),
            Place::Field(_, field_idx) => {
                let name = self.mir.field_name_table.get(field_idx)?;
                self.field_array_elem_kinds.get(name).copied()
            }
            _ => None,
        }
    }

    /// R8 W8 jit-aliased-cow-push v0.3 surface-and-stop helper.
    ///
    /// Scan ALL statements in ALL blocks of the current MIR function for
    /// any `Operand::Move(Place::Local(slot))` or
    /// `Operand::MoveExplicit(Place::Local(slot))` occurrence. Returns
    /// true on first hit.
    ///
    /// Used by the typed-array `.push()` inline codegen to detect the
    /// aliased-CoW SEGFAULT shape (see audit
    /// `docs/cluster-audits/v0.3-r8w7-jit-aliased-cow-segfault-audit.md`
    /// §3 / §6): when the receiver slot has been previously moved out of
    /// (e.g. via `let alias = data` MIR-lowering at
    /// `crates/shape-vm/src/mir/lowering/stmt.rs:269-273`), reading the
    /// nulled slot at push time dereferences NULL → SIGSEGV in
    /// `jit_v2_array_push`. The detector triggers a structured `Err` on
    /// match, which the W12 fall-through (`shape-jit/src/executor.rs:
    /// 170-194`) routes to the bytecode interpreter — VM == JIT.
    ///
    /// **Conservatism.** A function-wide scan is over-conservative: a
    /// `Move(slot)` in an unreachable arm or strictly AFTER the push site
    /// in execution order would still trigger the deopt. The
    /// conservatism is binding-compliant for v0.3 — over-deopt costs JIT
    /// throughput, never correctness. A precise data-flow check is v0.4
    /// territory (alongside the deeper MIR-lowering fix that would not
    /// emit `Move` from still-live `let`-source bindings at all).
    ///
    /// **Operand coverage.** Scans `Rvalue::Use` / `Rvalue::Clone`
    /// operands, `Rvalue::BinaryOp` / `Rvalue::UnaryOp` operands,
    /// `Rvalue::Aggregate` operands, `StatementKind::ArrayStore` /
    /// `ObjectStore` / `EnumStore` / `ClosureCapture` / `TaskBoundary`
    /// operands, and `TerminatorKind::Call` `func` + `args` operands +
    /// `TerminatorKind::SwitchBool` `operand`. Other terminators
    /// (`Goto` / `Return` / `Unreachable`) carry no operands.
    pub(crate) fn mir_has_prior_move_of_slot(&self, slot: SlotId) -> bool {
        use shape_vm::mir::types::{Operand, Rvalue, StatementKind, TerminatorKind};
        let matches_slot = |op: &Operand| -> bool {
            matches!(
                op,
                Operand::Move(Place::Local(s)) | Operand::MoveExplicit(Place::Local(s))
                    if *s == slot
            )
        };
        let rvalue_has_move = |rv: &Rvalue| -> bool {
            match rv {
                Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => matches_slot(op),
                Rvalue::BinaryOp(_, lhs, rhs) => matches_slot(lhs) || matches_slot(rhs),
                Rvalue::FuzzyComparison { lhs, rhs, .. } => matches_slot(lhs) || matches_slot(rhs),
                Rvalue::Aggregate(ops) => ops.iter().any(&matches_slot),
                Rvalue::Borrow(_, _) => false,
                Rvalue::EnumTest { operand, .. }
                | Rvalue::EnumPayload { operand, .. }
                | Rvalue::TypePatternTest { operand, .. }
                | Rvalue::EnumDiscriminantTest { operand, .. }
                | Rvalue::PrimitiveCast { operand, .. } => matches_slot(operand),
            }
        };
        for block in &self.mir.blocks {
            for stmt in &block.statements {
                match &stmt.kind {
                    StatementKind::Assign(_, rv) => {
                        if rvalue_has_move(rv) {
                            return true;
                        }
                    }
                    StatementKind::ArrayStore { operands, .. }
                    | StatementKind::ObjectStore { operands, .. }
                    | StatementKind::EnumStore { operands, .. }
                    | StatementKind::ClosureCapture { operands, .. }
                    | StatementKind::ModuleBindingStore { operands, .. }
                    | StatementKind::TaskBoundary(operands, _) => {
                        if operands.iter().any(&matches_slot) {
                            return true;
                        }
                    }
                    StatementKind::Drop(_) | StatementKind::Nop => {}
                }
            }
            match &block.terminator.kind {
                TerminatorKind::Call { func, args, .. } => {
                    if matches_slot(func) || args.iter().any(&matches_slot) {
                        return true;
                    }
                }
                TerminatorKind::SwitchBool { operand, .. } => {
                    if matches_slot(operand) {
                        return true;
                    }
                }
                TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
            }
        }
        false
    }

    /// v0.3.3 move-semantics JIT-divergence surface-and-stop detector.
    ///
    /// Root cause: `compile_operand` (`ownership.rs:225`) lowers every
    /// `Operand::Move` / `Operand::MoveExplicit` by reading the value and
    /// then NULLing the source slot (`null_place`) to prevent double-drop.
    /// The MIR lowering of `let b = a` / `a = i` emits `Use(Move(src))`
    /// unconditionally (`lowering/stmt.rs:261-270`), but the VM does NOT
    /// honour that as a destructive move: `compute_ownership_decisions`
    /// (`mir/solver.rs:1736`) downgrades the move to `Copy` (Copy types) or
    /// `Clone` (still-live non-Copy) and keeps the source slot's value.
    ///
    /// Consequence — VM != JIT, and JIT is the default mode:
    ///   * `let a = 42; let b = a; print(a)` — VM prints 42, JIT reads the
    ///     nulled slot and prints 0 (silent-wrong-output).
    ///   * `a = i` inside a `while` loop — the JIT nulls the loop counter
    ///     `i` on every iteration's copy, so the condition re-reads 0 and
    ///     the loop never terminates (JIT hangs -> timeout).
    ///
    /// Per CLAUDE.md "a JIT path that cannot match the VM MUST surface-and-
    /// stop (deopt to the interpreter)". Replicating the VM's per-point
    /// Copy/Clone/Move liveness decision inside the JIT operand lowering is
    /// a v0.4 root-cause workstream; for v0.3.3 the binding-compliant fix is
    /// a whole-function deopt whenever the divergence SHAPE is present —
    /// i.e. a slot is `Move`/`MoveExplicit`-sourced and that same slot is
    /// read again at a DIFFERENT program point with no guaranteed intervening
    /// reinitialisation. Returns `true` to request the deopt.
    ///
    /// **Soundness over throughput.** The analysis is intentionally
    /// conservative: a read in any other block, or a later read in the same
    /// block not preceded by a reinitialising `Assign(slot, ..)`, both
    /// trigger the deopt. Over-deopt costs JIT speed, never correctness; the
    /// bytecode interpreter (which honours the VM ownership model) runs the
    /// program and VM == JIT is preserved. Mirrors `mir_has_prior_move_of_slot`.
    pub(crate) fn mir_has_move_then_read_divergence(&self) -> bool {
        use shape_vm::mir::types::{Operand, Place, Rvalue, StatementKind, TerminatorKind};

        // (block_idx, stmt_idx) of every Move/MoveExplicit source occurrence,
        // keyed by the moved slot. stmt_idx == usize::MAX marks a move that
        // occurs in the block's terminator operands.
        let mut moves: HashMap<SlotId, Vec<(usize, usize)>> = HashMap::new();
        // Same keying for every READ of a slot (Copy operand, borrow, or any
        // operand position the JIT lowers as a value read). Move sources are
        // ALSO reads (they read the value before nulling), but a move site is
        // not a "later read" of itself — we exclude the exact move location.
        let mut reads: HashMap<SlotId, Vec<(usize, usize)>> = HashMap::new();
        // (block_idx, stmt_idx) of every Assign whose destination is a bare
        // `Place::Local(slot)` — a reinitialisation point that clears the
        // moved state for that slot.
        let mut reinits: HashMap<SlotId, Vec<usize>> = HashMap::new();

        let record_operand =
            |op: &Operand,
             bi: usize,
             si: usize,
             moves: &mut HashMap<SlotId, Vec<(usize, usize)>>,
             reads: &mut HashMap<SlotId, Vec<(usize, usize)>>| {
                // v0.3.3 move-then-read divergence: a `let q = p` whole-value
                // bind lowers as `Use(Move(Local(p)))`, which `compile_operand`
                // nulls. A SUBSEQUENT read of `p` — including a field/element
                // projection such as `print(p.x)` lowered as
                // `Copy(Field(Local(p), 0))` — then reads the nulled slot and
                // (for a struct) dereferences a null/corrupted pointer → SIGSEGV
                // under JIT (VM keeps `p` live and prints correctly). The read
                // tracking MUST therefore key on the place's ROOT local, not
                // only on a bare `Place::Local`, or a projected later read is
                // missed and the whole-function deopt never fires. (Pre-fix this
                // arm only matched `Place::Local`, so `let q = p; print(p.x)`
                // segfaulted instead of deopting.)
                match op {
                    Operand::Move(place) | Operand::MoveExplicit(place) => {
                        if let Place::Local(s) = place {
                            moves.entry(*s).or_default().push((bi, si));
                        }
                        reads.entry(place.root_local()).or_default().push((bi, si));
                    }
                    Operand::Copy(place) => {
                        reads.entry(place.root_local()).or_default().push((bi, si));
                    }
                    _ => {}
                }
            };

        let record_rvalue_reads =
            |rv: &Rvalue,
             bi: usize,
             si: usize,
             moves: &mut HashMap<SlotId, Vec<(usize, usize)>>,
             reads: &mut HashMap<SlotId, Vec<(usize, usize)>>| {
                match rv {
                    Rvalue::Use(op) | Rvalue::Clone(op) | Rvalue::UnaryOp(_, op) => {
                        record_operand(op, bi, si, moves, reads);
                    }
                    Rvalue::BinaryOp(_, lhs, rhs) => {
                        record_operand(lhs, bi, si, moves, reads);
                        record_operand(rhs, bi, si, moves, reads);
                    }
                    Rvalue::FuzzyComparison { lhs, rhs, .. } => {
                        record_operand(lhs, bi, si, moves, reads);
                        record_operand(rhs, bi, si, moves, reads);
                    }
                    Rvalue::Aggregate(ops) => {
                        for op in ops {
                            record_operand(op, bi, si, moves, reads);
                        }
                    }
                    // A borrow reads the slot's value (the JIT loads it). Key on
                    // the root local so a projected borrow (`&p.x`) still counts
                    // as a later read of `p`.
                    Rvalue::Borrow(_, place) => {
                        reads.entry(place.root_local()).or_default().push((bi, si));
                    }
                    Rvalue::EnumTest { operand, .. }
                    | Rvalue::EnumPayload { operand, .. }
                    | Rvalue::TypePatternTest { operand, .. }
                    | Rvalue::EnumDiscriminantTest { operand, .. }
                    | Rvalue::PrimitiveCast { operand, .. } => {
                        record_operand(operand, bi, si, moves, reads);
                    }
                }
            };

        for (bi, block) in self.mir.blocks.iter().enumerate() {
            for (si, stmt) in block.statements.iter().enumerate() {
                match &stmt.kind {
                    StatementKind::Assign(dest, rv) => {
                        if let Place::Local(d) = dest {
                            reinits.entry(*d).or_default().push(si);
                        }
                        record_rvalue_reads(rv, bi, si, &mut moves, &mut reads);
                    }
                    StatementKind::ArrayStore { operands, .. }
                    | StatementKind::ObjectStore { operands, .. }
                    | StatementKind::EnumStore { operands, .. }
                    | StatementKind::ClosureCapture { operands, .. }
                    | StatementKind::ModuleBindingStore { operands, .. }
                    | StatementKind::TaskBoundary(operands, _) => {
                        for op in operands {
                            record_operand(op, bi, si, &mut moves, &mut reads);
                        }
                    }
                    StatementKind::Drop(_) | StatementKind::Nop => {}
                }
            }
            // Terminator operands — stmt_idx sentinel usize::MAX sorts after
            // every real statement in the same block.
            match &block.terminator.kind {
                TerminatorKind::Call { func, args, .. } => {
                    record_operand(func, bi, usize::MAX, &mut moves, &mut reads);
                    for op in args {
                        record_operand(op, bi, usize::MAX, &mut moves, &mut reads);
                    }
                }
                TerminatorKind::SwitchBool { operand, .. } => {
                    record_operand(operand, bi, usize::MAX, &mut moves, &mut reads);
                }
                TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
            }
        }

        // For each moved slot, deopt if it is read at any program point that
        // is not exactly one of its move sites, unless that read is in the
        // SAME block strictly after a reinitialising assign that itself
        // follows the move (straight-line reinit clears the moved state).
        for (slot, move_sites) in &moves {
            let Some(slot_reads) = reads.get(slot) else {
                continue;
            };
            let empty = Vec::new();
            let slot_reinits = reinits.get(slot).unwrap_or(&empty);
            for &(rb, rs) in slot_reads {
                // Is there a move that this read can observe the null of?
                // Conservative: any move in a DIFFERENT block, or an earlier
                // move in the SAME block, is a divergence unless a same-block
                // reinit sits strictly between the move and the read.
                for &(mb, ms) in move_sites {
                    // A move site reads the value before nulling — it is not a
                    // "later read" of ITSELF. Exclude only the move at this
                    // exact location; a move at a DIFFERENT location is still a
                    // later read that can observe a prior move's null. (Pre-fix
                    // this skipped the read against ALL moves when the read sat
                    // on any move site, so `let q = p; let r = p` — two
                    // consecutive whole-value moves of `p` — was missed and the
                    // second move read the JIT-nulled `p` → SIGSEGV.)
                    if (mb, ms) == (rb, rs) {
                        continue;
                    }
                    let observable = if mb != rb {
                        // Cross-block: the read may execute after the move on
                        // some CFG path (including loop back-edges). Deopt.
                        true
                    } else {
                        // Same block: only a read strictly after the move.
                        ms < rs && {
                            // Cleared if a reinit lies in (ms, rs].
                            !slot_reinits.iter().any(|&ri| ri > ms && ri <= rs)
                        }
                    };
                    if observable {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// True when the place's root local is known to hold a TypedObject
    /// (`ConcreteType::Struct(_)` / `ConcreteType::Enum(_)` /
    /// `ConcreteType::Option(_)` / `ConcreteType::Result(_, _)` /
    /// `ConcreteType::Tuple(_)`). These all share the `HeapKind::TypedObject`
    /// carrier and are materialised by the subsequent
    /// `StatementKind::ObjectStore` / `EnumStore`.
    ///
    /// Used by the `Assign(Aggregate)` short-circuit in `statements.rs`:
    /// when the bytecode compiler proved the destination slot is a
    /// TypedObject, the preceding `Rvalue::Aggregate` is a MIR scratch step
    /// — the real allocation happens in the following `ObjectStore`.
    /// Skipping the Aggregate avoids the `Route A surface-and-stop`
    /// previously hit at compile time for `Point { x, y }`-style literals.
    ///
    /// Source: the per-MirToIR `concrete_types` vector, threaded from
    /// `BytecodeProgram.top_level_local_concrete_types` per ADR-006
    /// §2.7.5 (W12-top-level-concrete-types-conduit close, 2026-05-12).
    pub(crate) fn is_typed_object_slot(&self, place: &Place) -> bool {
        let slot = match place {
            Place::Local(s) => *s,
            _ => return false,
        };
        let Some(ct) = self.concrete_types.get(slot.0 as usize) else {
            return false;
        };
        matches!(
            ct,
            ConcreteType::Struct(_)
                | ConcreteType::Enum(_)
                | ConcreteType::Option(_)
                | ConcreteType::Result(_, _)
                | ConcreteType::Tuple(_)
        )
    }

    /// Return the FFI `FuncRef` for `jit_v2_array_new_<elem>`.
    ///
    /// ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
    /// extended with `StringV2` / `DecimalV2` arms routing to
    /// `jit_new_typed_array_string` / `jit_new_typed_array_decimal`. These
    /// allocate `TypedArray<*const StringObj>` / `TypedArray<*const
    /// DecimalObj>` carriers per ADR-006 §2.7.5 + §2.7.24 Q25.A SUPERSEDED +
    /// audit deliverable (b) §4.1.B. Per-element pointer payload is the
    /// v2-raw heap-element shape produced by VM-side `NewStringV2` /
    /// `NewDecimalV2` opcodes at
    /// `crates/shape-vm/src/executor/v2_handlers/array.rs:803-858`.
    pub(crate) fn v2_array_new_func(
        &self,
        elem: NativeKind,
    ) -> Option<cranelift::codegen::ir::FuncRef> {
        match elem {
            NativeKind::Float64 => Some(self.ffi.v2_array_new_f64),
            NativeKind::Int64 | NativeKind::UInt64 => Some(self.ffi.v2_array_new_i64),
            NativeKind::Int32 | NativeKind::UInt32 => Some(self.ffi.v2_array_new_i32),
            NativeKind::Bool | NativeKind::Int8 | NativeKind::UInt8 => {
                Some(self.ffi.v2_array_new_bool)
            }
            NativeKind::StringV2 => Some(self.ffi.v2_array_new_string),
            NativeKind::DecimalV2 => Some(self.ffi.v2_array_new_decimal),
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18) —
            // v2-raw `TypedArray<*const TypedObjectStorage>` allocator per ADR-006
            // §2.7.5 + audit `v0.3-w16-v3s5-ckpt56-strict-close-audit.md` §2.1.
            // Per-element payload is an 8-byte `*const TypedObjectStorage` raw
            // pointer; pushed via the generic `jit_v2_array_push` I64-shaped
            // dispatcher (size=8 below). Mirrors the String/Decimal carriers.
            NativeKind::Ptr(shape_value::HeapKind::TypedObject) => {
                Some(self.ffi.v2_array_new_typed_object)
            }
            // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05) —
            // v2-raw `TypedArray<*const TraitObjectStorage>` allocator per
            // ADR-006 §2.7.5 + §2.7.24 Q25.C. 8-byte `*const TraitObjectStorage`
            // raw pointer payload; mirrors the TypedObject carrier.
            NativeKind::Ptr(shape_value::HeapKind::TraitObject) => {
                Some(self.ffi.v2_array_new_trait_object)
            }
            _ => None,
        }
    }

    /// Return the element byte size for `NativeKind`s backed by the generic
    /// `jit_v2_array_push` dispatcher, or `None` for unsupported kinds. The
    /// caller uses the returned size as the `elem_size` I8 immediate passed
    /// to the dispatcher.
    ///
    /// ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
    /// `StringV2` / `DecimalV2` are 8-byte pointer carriers — the element
    /// payload is a `*const StringObj` / `*const DecimalObj` raw pointer,
    /// pushed via the generic `jit_v2_array_push` I64-shaped dispatcher.
    pub(crate) fn v2_array_push_elem_size(&self, elem: NativeKind) -> Option<i64> {
        match elem {
            NativeKind::Float64 => Some(8),
            NativeKind::Int64 | NativeKind::UInt64 => Some(8),
            NativeKind::Int32 | NativeKind::UInt32 => Some(4),
            NativeKind::Bool | NativeKind::Int8 | NativeKind::UInt8 => Some(1),
            NativeKind::StringV2 | NativeKind::DecimalV2 => Some(8),
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18) —
            // 8-byte raw pointer carrier (`*const TypedObjectStorage`).
            NativeKind::Ptr(shape_value::HeapKind::TypedObject) => Some(8),
            // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05) —
            // 8-byte raw pointer carrier (`*const TraitObjectStorage`).
            NativeKind::Ptr(shape_value::HeapKind::TraitObject) => Some(8),
            _ => None,
        }
    }

    /// Emit a call to the generic `jit_v2_array_push` FFI dispatcher. `val`
    /// is the element value Cranelift SSA value already coerced to the
    /// native Cranelift type for `elem` (via `coerce_to_v2_elem`). This
    /// helper zero/sign-extends or bitcasts the value to I64 and passes
    /// `elem_size` as an I8 immediate.
    pub(crate) fn emit_v2_array_push_call(
        &mut self,
        arr_ptr: Value,
        val: Value,
        elem: NativeKind,
    ) -> Result<(), String> {
        let elem_size = match self.v2_array_push_elem_size(elem) {
            Some(s) => s,
            None => return Err(format!("v2_array_push: unsupported elem kind {:?}", elem)),
        };
        let bits = self.widen_to_i64_bits(val);
        let size_val = self.builder.ins().iconst(types::I8, elem_size);
        self.builder
            .ins()
            .call(self.ffi.v2_array_push, &[arr_ptr, bits, size_val]);
        Ok(())
    }

    /// Widen/bitcast an arbitrary Cranelift element value into an I64 bit
    /// pattern suitable for the generic `jit_v2_array_push` dispatcher.
    fn widen_to_i64_bits(&mut self, val: Value) -> Value {
        let val_type = self.builder.func.dfg.value_type(val);
        if val_type == types::F64 {
            self.builder.ins().bitcast(types::I64, MemFlags::new(), val)
        } else if val_type == types::I64 {
            val
        } else if val_type == types::I32 || val_type == types::I16 || val_type == types::I8 {
            // Zero-extend: the dispatcher uses only the low `elem_size` bytes,
            // so sign bits above that are ignored.
            self.builder.ins().uextend(types::I64, val)
        } else {
            val
        }
    }

    /// Convert a Cranelift value into the native type expected by the v2
    /// element store/push helpers for `elem`.
    pub(crate) fn coerce_to_v2_elem(&mut self, val: Value, elem: NativeKind) -> Value {
        let val_type = self.builder.func.dfg.value_type(val);
        match elem {
            NativeKind::Float64 => {
                if val_type == types::F64 {
                    val
                } else if val_type == types::I64 {
                    self.builder.ins().bitcast(types::F64, MemFlags::new(), val)
                } else {
                    let i64_val = if val_type == types::I32 {
                        self.builder.ins().sextend(types::I64, val)
                    } else if val_type == types::I8 {
                        self.builder.ins().uextend(types::I64, val)
                    } else {
                        val
                    };
                    self.builder.ins().fcvt_from_sint(types::F64, i64_val)
                }
            }
            NativeKind::Int64 | NativeKind::UInt64 => {
                if val_type == types::I64 {
                    let shifted = self.builder.ins().ishl_imm(val, 16);
                    self.builder.ins().sshr_imm(shifted, 16)
                } else if val_type == types::I32 {
                    self.builder.ins().sextend(types::I64, val)
                } else if val_type == types::I8 {
                    self.builder.ins().uextend(types::I64, val)
                } else {
                    val
                }
            }
            NativeKind::Int32 | NativeKind::UInt32 => {
                if val_type == types::I32 {
                    val
                } else if val_type == types::I64 {
                    let shifted = self.builder.ins().ishl_imm(val, 16);
                    let i64_val = self.builder.ins().sshr_imm(shifted, 16);
                    self.builder.ins().ireduce(types::I32, i64_val)
                } else if val_type == types::I8 {
                    self.builder.ins().uextend(types::I32, val)
                } else {
                    val
                }
            }
            NativeKind::Bool | NativeKind::Int8 | NativeKind::UInt8 => {
                if val_type == types::I8 {
                    val
                } else if val_type == types::I64 {
                    self.builder.ins().ireduce(types::I8, val)
                } else if val_type == types::I32 {
                    self.builder.ins().ireduce(types::I8, val)
                } else {
                    val
                }
            }
            // ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
            // StringV2 / DecimalV2 elements are 8-byte raw pointers — the
            // operand value is already an I64-shaped `*const StringObj` /
            // `*const DecimalObj` produced by the per-element constant
            // materializer in `emit_v2_array_aggregate`'s StringV2/DecimalV2
            // arm. No coercion needed.
            NativeKind::StringV2 | NativeKind::DecimalV2 => val,
            // Phase 4b Round 4 W16.2-A op_new_array-typed-object-element (2026-05-18) —
            // 8-byte raw pointer carrier, no coercion.
            NativeKind::Ptr(shape_value::HeapKind::TypedObject) => val,
            // Phase 4b W16.2-B op_new_array-trait-object-element (2026-06-05) —
            // 8-byte raw pointer carrier, no coercion.
            NativeKind::Ptr(shape_value::HeapKind::TraitObject) => val,
            _ => val,
        }
    }

    /// Coerce an arbitrary index Cranelift value into an `i32`.
    pub(crate) fn coerce_index_to_i32(&mut self, index_val: Value) -> Value {
        let idx_type = self.builder.func.dfg.value_type(index_val);
        if idx_type == types::I32 {
            index_val
        } else if idx_type == types::F64 {
            let i64_val = self.builder.ins().fcvt_to_sint_sat(types::I64, index_val);
            self.builder.ins().ireduce(types::I32, i64_val)
        } else if idx_type == types::I8 {
            self.builder.ins().uextend(types::I32, index_val)
        } else {
            let shifted = self.builder.ins().ishl_imm(index_val, 16);
            let payload = self.builder.ins().sshr_imm(shifted, 16);
            self.builder.ins().ireduce(types::I32, payload)
        }
    }

    /// Allocate a v2 typed array of the given element kind via FFI, then push
    /// each operand value into it. Returns the raw `*mut TypedArray<T>` as an
    /// `i64` Cranelift value, or `None` when no v2 helper exists.
    ///
    /// ckpt-6-prime Group X JIT FFI String/Decimal BUILD (2026-05-15):
    /// `StringV2` element kind takes a kind-specific per-element path —
    /// each `MirConstant::Str` / `MirConstant::StringId` operand is
    /// materialized at JIT-compile time as a `*const StringObj` constant
    /// via `crate::ffi::v2::string_obj_constant` (refcount-boosted permanent
    /// share, mirroring `crate::ffi::string::arc_string_constant` for the
    /// legacy `Arc<String>` carrier). The constant pointer is embedded as
    /// an `iconst I64` and pushed via the generic `jit_v2_array_push`
    /// dispatcher with elem_size=8. This is the JIT-side equivalent of the
    /// VM's `NewStringV2` opcode + `TypedArrayPushString` per-element
    /// transfer at `crates/shape-vm/src/executor/v2_handlers/array.rs:803`.
    ///
    /// `DecimalV2` element kind currently surfaces-and-stops at the MIR
    /// producer site — `MirConstant` has no `Decimal` variant, so Array
    /// <decimal> literals can't currently flow through MIR. Wiring the
    /// per-element NewDecimalV2 equivalent requires MIR-side producer
    /// support (`MirConstant::Decimal` variant or equivalent constant-pool
    /// reference), which is downstream territory beyond Group X's JIT FFI
    /// build scope.
    pub(crate) fn emit_v2_array_aggregate(
        &mut self,
        operands: &[Operand],
        elem: NativeKind,
    ) -> Result<Option<Value>, String> {
        let alloc_func = match self.v2_array_new_func(elem) {
            Some(f) => f,
            None => return Ok(None),
        };
        if self.v2_array_push_elem_size(elem).is_none() {
            return Ok(None);
        }

        let cap = self.builder.ins().iconst(types::I32, operands.len() as i64);
        let inst = self.builder.ins().call(alloc_func, &[cap]);
        let arr_ptr = self.builder.inst_results(inst)[0];

        match elem {
            // ckpt-6-prime Group X JIT FFI String/Decimal BUILD: per-element
            // NewStringV2 equivalent at the JIT mir_compiler dispatch site.
            // Each operand must be a `MirConstant::Str` / `MirConstant::
            // StringId` — the only producer sites for `NativeKind::StringV2`
            // Array<string> literals per ADR-006 §2.7.5 + audit deliverable
            // (b) §4.1.B. Other operand shapes structurally cannot produce
            // a StringV2-kind value and surface-and-stop here (no Bool-
            // default per §2.7.7 #9 / CLAUDE.md "Forbidden rationalizations").
            NativeKind::StringV2 => {
                use shape_vm::mir::types::MirConstant;
                for op in operands {
                    let s: String = match op {
                        Operand::Constant(MirConstant::Str(s)) => s.clone(),
                        Operand::Constant(MirConstant::StringId(id)) => {
                            let idx = *id as usize;
                            if idx >= self.strings.len() {
                                return Err(format!(
                                    "emit_v2_array_aggregate: StringV2 elem StringId({}) \
                                     out of bounds (pool len = {}) — string-pool conduit \
                                     mismatch at JIT compile time. ADR-006 §2.7.5 / Group X \
                                     JIT FFI String/Decimal BUILD.",
                                    id,
                                    self.strings.len()
                                ));
                            }
                            self.strings[idx].clone()
                        }
                        other => {
                            return Err(format!(
                                "emit_v2_array_aggregate: SURFACE — StringV2 elem kind \
                                 requires `MirConstant::Str` / `MirConstant::StringId` \
                                 operand per Group X NewStringV2-equivalent dispatch \
                                 (ADR-006 §2.7.5 + §2.7.24 Q25.A SUPERSEDED + audit \
                                 deliverable (b) §4.1.B). Got: {:?}. No Bool-default \
                                 fallback per §2.7.7 #9 / CLAUDE.md Forbidden \
                                 rationalizations.",
                                other
                            ));
                        }
                    };
                    // Compile-time materialize a `*const StringObj` permanent-
                    // share constant (refcount=2; one share is the active
                    // share transferred to the array, the other is the
                    // constant's permanent share that survives JIT-function
                    // Drop chains).
                    let string_obj_ptr = crate::ffi::v2::string_obj_constant(&s);
                    let val = self
                        .builder
                        .ins()
                        .iconst(types::I64, string_obj_ptr as usize as i64);
                    self.emit_v2_array_push_call(arr_ptr, val, elem)?;
                }
            }
            // ckpt-6-prime Group X JIT FFI String/Decimal BUILD: per-element
            // NewDecimalV2 equivalent surface-and-stop. `MirConstant` has no
            // `Decimal` variant so Array<decimal> literals can't currently
            // flow through MIR — the FFI allocator + carrier-routing is
            // wired (above) but the per-element producer requires MIR-side
            // support that's beyond Group X's JIT FFI build scope.
            NativeKind::DecimalV2 => {
                return Err(format!(
                    "emit_v2_array_aggregate: SURFACE — DecimalV2 elem-kind \
                     per-element materialization requires MIR-side producer \
                     support (`MirConstant::Decimal` variant or equivalent \
                     constant-pool reference) which is not yet wired. Group X \
                     scope covers the JIT FFI allocator + carrier-routing \
                     (jit_new_typed_array_decimal + v2_array_new_func \
                     DecimalV2 arm); per-element materializer awaits the MIR \
                     producer's wiring. ADR-006 §2.7.5 + §2.7.24 Q25.A \
                     SUPERSEDED + audit deliverable (b) §4.1.B. {} operands \
                     received; no Bool-default per §2.7.7 #9.",
                    operands.len()
                ));
            }
            _ => {
                // Scalar element kinds (Float64/Int64/Int32/Bool/etc.) —
                // existing inline path. compile_operand_raw produces a
                // Cranelift SSA value already in the native element type;
                // coerce_to_v2_elem normalizes and emit_v2_array_push_call
                // routes through the generic dispatcher.
                //
                // γ-CP3 jit-array-builder (v0.3 NO-KNOWN-INCORRECTNESS).
                // A scalar-element typed array can only be built from
                // scalar-element operands. An array-builder construct that
                // the JIT does NOT model — an array-spread element
                // (`[...a, 4, 5]`), or the slice-shape `Aggregate([source,
                // start])` the MIR producer emits for an open-range index
                // (`xs[2..]`) and for a destructure-rest binding
                // (`let [a, ...rest] = ...`) — carries a heap-pointer
                // operand (the source `*mut TypedArray<T>`) where a scalar
                // element value is required.
                //
                // Pre-γ-CP3 this loop blindly pushed the raw heap-pointer
                // bits as an `Int64` element: the destination array then
                // held the pointer integer instead of the spread/slice
                // contents, and a downstream `.sum()` / `.len()` read
                // uninitialized/garbage memory. The VM correctly surfaces
                // these same constructs (`op_new_array` SURFACE, the V3-S5
                // ckpt-5 consumer-cascade) — the JIT must match.
                //
                // Per ADR-006 §2.7.14 forbidden list ("Bool-default
                // fallback for unknown element kinds") + §2.7.7 #9, the
                // honest response is surface-and-stop: a structured `Err`
                // that the W12 fall-through routes to the bytecode
                // interpreter, which produces the VM's clean error.
                // Real array-builder/slice JIT codegen is a follow-up,
                // gated on the V3-S5 `op_new_array` construction rebuild
                // landing VM-side first (implementing it before that
                // would create a NEW VM/JIT divergence).
                //
                // Operands whose kind is genuinely unproven (`None`) flow
                // through the existing scalar path unchanged — the
                // detector fires ONLY on a proven heap-pointer kind, so
                // ordinary scalar array literals (`[1, 2, 3]`) are not
                // over-broadly bailed.
                for op in operands {
                    if let Some(NativeKind::Ptr(heap_kind)) = self.operand_slot_kind(op) {
                        return Err(format!(
                            "emit_v2_array_aggregate: SURFACE — scalar \
                             element kind {:?} array has an operand with \
                             heap-pointer kind Ptr({:?}). This is an \
                             array-builder/slice construct the MIR-JIT \
                             does not model (array-spread element, \
                             open-range slice `xs[2..]`, or destructure-\
                             rest `let [a, ...rest] = ...`). The JIT \
                             surfaces-and-stops so the W12 fall-through \
                             routes the program to the bytecode \
                             interpreter, which surfaces the VM's clean \
                             `op_new_array` / `SliceAccess` error — VM == \
                             JIT, neither produces garbage. Real \
                             array-builder codegen is a γ-CP3 follow-up, \
                             gated on the V3-S5 op_new_array construction \
                             rebuild. ADR-006 §2.7.14 / §2.7.7 #9.",
                            elem, heap_kind
                        ));
                    }
                    let raw = self.compile_operand_raw(op)?;
                    let val = self.coerce_to_v2_elem(raw, elem);
                    self.emit_v2_array_push_call(arr_ptr, val, elem)?;
                }
            }
        }

        Ok(Some(arr_ptr))
    }

    /// Try to emit an inline v2 typed-array method call.
    pub(crate) fn try_emit_v2_array_method(
        &mut self,
        method_name: &str,
        receiver: &Place,
        rest_args: &[Operand],
        destination: &Place,
        elem: NativeKind,
    ) -> Result<Option<()>, String> {
        match method_name {
            // γ-CP9 jit-groupby-surface (v0.3 NO-KNOWN-INCORRECTNESS item
            // 9). `count` / `group` / `groupBy` on a typed array take a
            // `|x| ...` closure predicate. The MIR-JIT has no inline
            // codegen for these here, and the fall-through path
            // (`jit_call_method` → VM trampoline) cannot carry the JIT-
            // format NaN-boxed inline-closure carrier across the FFI
            // boundary to the VM's v2-raw `Ptr(Closure)` ABI — the
            // carrier-shape mismatch made the transient `kinded_args`
            // drop SIGSEGV on the closure arg (array `groupBy(|x| ...)`
            // crashed ec=139). A real inline JIT path would have to model
            // the closure-callback ABI for typed arrays — W10 jit-
            // playbook §5 / §2.7.4 territory — and would only re-create a
            // VM/JIT divergence while the VM-side `handle_group_by_v2` /
            // `handle_count_v2` still SURFACE.
            //
            // The honest fix is a compile-stage surface-and-stop (the
            // γ-CP3 array-builder pattern): return a structured `Err`.
            // The W12 fall-through (`docs/cluster-audits/v0.3-w12-jit-
            // mode-semantics-close.md`) routes the whole program to the
            // bytecode interpreter, which runs the method call with its
            // own carrier-correct closure handling and produces the VM's
            // behaviour verbatim. Net result: VM == JIT — both run the
            // identical interpreter path, neither produces garbage or
            // SIGSEGVs.
            "count" | "group" | "groupBy" => {
                let _ = (receiver, rest_args, destination, elem);
                Err(format!(
                    "γ-CP9 SURFACE: typed-array `.{}()` JIT codegen is \
                     unimplemented (closure-callback ABI for typed-array \
                     higher-order methods is W10 jit-playbook §5 / ADR-006 \
                     §2.7.4 territory) — JIT compilation bails so the W12 \
                     fall-through runs the interpreter",
                    method_name,
                ))
            }
            "length" | "len" => {
                let arr_ptr = self.read_place(receiver)?;
                let len_i32 = self.v2_array_len(arr_ptr);
                let len_i64 = self.builder.ins().sextend(types::I64, len_i32);
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, len_i64)?;
                Ok(Some(()))
            }
            "push" => {
                if rest_args.len() != 1 {
                    return Ok(None);
                }
                if self.v2_array_push_elem_size(elem).is_none() {
                    return Ok(None);
                }
                // R8 W8 jit-aliased-cow-push v0.3 surface-and-stop
                // (audit `docs/cluster-audits/v0.3-r8w7-jit-aliased-cow-
                // segfault-audit.md` §4 fallback / §6, supervisor ratify
                // 2026-05-24, memory-unsafety unconditional v0.3-gating).
                //
                // Reproducer:
                //   var data: Array<int> = [1, 2, 3]
                //   let alias = data        // MIR: Assign(alias, Use(Move(data)))
                //   data.push(4)            // SEGFAULT under JIT
                //
                // Root cause: MIR lowering at `crates/shape-vm/src/mir/
                // lowering/stmt.rs:269-273` emits `Operand::Move` for `let`
                // bindings whose ownership is `Inferred` (per the
                // `OwnershipModifier::Inferred` doc-comment in
                // `shape-ast/src/ast/program.rs:107` — "For `let`: always
                // move"). The JIT's `compile_operand` Move arm at
                // `mir_compiler/ownership.rs:225-230` nulls the source slot
                // after reading via `null_place`. The bytecode VM's
                // `data.push(4)` compile path does NOT go through MIR — it
                // emits CloneLocal-equivalent opcodes that keep the source
                // live, which is why `--mode vm` correctly shows both
                // `data` and `alias` as `[1, 2, 3, 4]` (the array is shared
                // through the refcount-bumped aliasing).
                //
                // The post-MIR-Move JIT then reads NULL from `data`'s slot
                // (verified empirically via gdb: `rbx = 0x0` at
                // `jit_v2_array_push+39`) and the subsequent `mov
                // 0x10(%rbx),%eax` dereferences NULL → SIGSEGV.
                //
                // The v0.3-binding-compliant fix is the audit §6 fallback:
                // detect the at-risk shape statically (the receiver slot
                // has been previously moved out of via `Operand::Move` /
                // `MoveExplicit` in the same function) and surface-and-stop
                // by returning `Err` — the existing W12 fall-through at
                // `executor.rs:170-194` routes the whole program to the
                // bytecode interpreter, which produces the VM's clean
                // semantics. Eliminates the SEGFAULT (memory-unsafety, the
                // v0.3 gating condition); the deeper MIR-lowering fix to
                // not Move from a still-live `let`-source binding is v0.4
                // territory (touches the documented `OwnershipModifier::
                // Inferred` semantics).
                //
                // Refused defection-attractor framings per CLAUDE.md
                // §Forbidden-Patterns: no Bool-default refcount probe, no
                // decode kind from bits, no "preserve unverified path with
                // a fallback flag", no CoW codegen (would diverge from VM
                // semantics — VM does NOT clone-on-write, both aliases
                // observe the in-place mutation).
                if let Place::Local(recv_slot) = receiver {
                    if self.mir_has_prior_move_of_slot(*recv_slot) {
                        return Err(format!(
                            "R8 W8 jit-aliased-cow-push SURFACE: typed-array \
                             `.push()` receiver slot {} has a prior \
                             `Operand::Move` / `MoveExplicit` in the same \
                             function (e.g. `let alias = data` MIR-lowering \
                             at `mir/lowering/stmt.rs:269-273` nulls `data`'s \
                             slot post-Move). The JIT inline push would \
                             dereference a NULL receiver pointer → SIGSEGV \
                             (audit `v0.3-r8w7-jit-aliased-cow-segfault-\
                             audit.md`). Surface-and-stop deopts the whole \
                             program to the bytecode interpreter (W12 \
                             fall-through at `shape-jit/src/executor.rs:\
                             170-194`), which uses its own MIR-independent \
                             compile that does NOT null the source slot — \
                             VM == JIT semantics restored. Memory-unsafety \
                             unconditional v0.3-gating per supervisor \
                             2026-05-24 ruling.",
                            recv_slot.0,
                        ));
                    }
                }
                let arr_ptr = self.read_place(receiver)?;
                let raw_arg = self.compile_operand_raw(&rest_args[0])?;
                let val = self.coerce_to_v2_elem(raw_arg, elem);
                self.emit_v2_array_push_call(arr_ptr, val, elem)?;
                let none_val = self.builder.ins().iconst(types::I64, 0i64);
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, none_val)?;
                Ok(Some(()))
            }
            // Phase 4b Round 4 W15 LANG-9-spin-3-first JIT fix
            // (2026-05-18). ADR-006 §2.7.5 producer-side stamp: inline the
            // element-0 / element-(len-1) read via `v2_array_get` for the
            // chained-receiver shape (`[..].map(..).first()` —
            // F3b reproducer) where the receiver lacks a Place::Local
            // binding to register `v2_typed_array_locals` against. The
            // result kind matches the VM PHF (`typed_int_array_methods::
            // first` returns Int64 for I64-element arrays) — see the
            // sibling `parametric_method_return_kind_from_receiver`
            // ("first"|"last"|"pop", Array(elem)) arm in
            // `mir_compiler/types.rs` which stamps the same element kind
            // into the JIT slot_kinds track.
            //
            // Bypasses the `jit_call_method` trampoline — the typed-
            // element read is structurally cheap (load data+0 or
            // data+(len-1)*size) and removes the FFI hop. Pre-fix the
            // F3b reproducer fell through to `jit_call_method` →
            // `typed_int_array_methods::first` returning the bare
            // element bits with kind Int64; the JIT downstream
            // `operand_slot_kind` previously returned `Ptr(Option)` via
            // the pre-fix arm above and treated the bare element bits as
            // an Option<T> pointer carrier → "None" rendered. The
            // post-fix slot kind is Int64 (sibling §2.7.5 arm) but the
            // chained-receiver shape's slot-bits trip a different
            // downstream surface (the result kind+bits flow through
            // unbinded chain slots without the let-binding's full
            // §2.7.5 conduit). This arm closes that surface by
            // structurally emitting the element read inline.
            //
            // Empty-array contract: returns the element default (0 for
            // integers, 0.0 for floats, false for bools) via
            // `v2_array_get`'s out-of-bounds branch in
            // `mir_compiler/v2_array.rs::v2_array_get`. This mirrors
            // the VM PHF's empty-array `KindedSlot::none()` Bool/0
            // sentinel for the integer/float/bool element families.
            // String / Decimal / Char element receivers fall through
            // (Ok(None)) since the heap-element variants need carrier
            // retain on read and are tracked separately under the
            // V3-S5 ckpt-6 STRICT close.
            "first" => {
                if !rest_args.is_empty() {
                    return Ok(None);
                }
                if !matches!(
                    elem,
                    NativeKind::Int64
                        | NativeKind::UInt64
                        | NativeKind::Int32
                        | NativeKind::UInt32
                        | NativeKind::Int16
                        | NativeKind::UInt16
                        | NativeKind::Int8
                        | NativeKind::UInt8
                        | NativeKind::Float64
                        | NativeKind::Float32
                        | NativeKind::Bool
                ) {
                    return Ok(None);
                }
                let arr_ptr = self.read_place(receiver)?;
                let zero_idx = self.builder.ins().iconst(types::I32, 0);
                let elem_val = self.v2_array_get(arr_ptr, zero_idx, elem);
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, elem_val)?;
                Ok(Some(()))
            }
            "last" => {
                if !rest_args.is_empty() {
                    return Ok(None);
                }
                if !matches!(
                    elem,
                    NativeKind::Int64
                        | NativeKind::UInt64
                        | NativeKind::Int32
                        | NativeKind::UInt32
                        | NativeKind::Int16
                        | NativeKind::UInt16
                        | NativeKind::Int8
                        | NativeKind::UInt8
                        | NativeKind::Float64
                        | NativeKind::Float32
                        | NativeKind::Bool
                ) {
                    return Ok(None);
                }
                let arr_ptr = self.read_place(receiver)?;
                let len_i32 = self.v2_array_len(arr_ptr);
                let one = self.builder.ins().iconst(types::I32, 1);
                let last_idx = self.builder.ins().isub(len_i32, one);
                // `v2_array_get` performs an unsigned bounds check
                // (`index < len`); when len==0 the resulting last_idx
                // wraps to a large positive u32 that fails the bounds
                // check and the OOB path returns the element default —
                // mirrors the VM PHF's empty-array `KindedSlot::none()`
                // sentinel for integer/float/bool element families.
                let elem_val = self.v2_array_get(arr_ptr, last_idx, elem);
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, elem_val)?;
                Ok(Some(()))
            }
            "sum" => {
                // Phase C.3: Bypass method dispatch entirely — call the SIMD
                // reduction FFI (`jit_v2_array_sum_f64` / `jit_v2_array_sum_i64`)
                // in one shot. The FFI uses `wide::f64x4`/`wide::i64x4` lanes
                // so AVX2/NEON-capable CPUs get a ~4x throughput over the
                // scalar loop.
                if !rest_args.is_empty() {
                    return Ok(None);
                }
                let sum_func = match elem {
                    NativeKind::Float64 => self.ffi.v2_array_sum_f64,
                    NativeKind::Int64 | NativeKind::UInt64 => self.ffi.v2_array_sum_i64,
                    _ => return Ok(None),
                };
                let arr_ptr = self.read_place(receiver)?;
                let inst = self.builder.ins().call(sum_func, &[arr_ptr]);
                let result = self.builder.inst_results(inst)[0];
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, result)?;
                Ok(Some(()))
            }
            // f64-only SIMD reductions. Dispatched only for Array<number>.
            "min" | "max" | "mean" | "avg" | "sumSquares" | "sum_squares" => {
                if !rest_args.is_empty() {
                    return Ok(None);
                }
                if !matches!(elem, NativeKind::Float64) {
                    return Ok(None);
                }
                let func = match method_name {
                    "min" => self.ffi.v2_array_min_f64,
                    "max" => self.ffi.v2_array_max_f64,
                    "mean" | "avg" => self.ffi.v2_array_mean_f64,
                    "sumSquares" | "sum_squares" => self.ffi.v2_array_sum_squares_f64,
                    _ => unreachable!(),
                };
                let arr_ptr = self.read_place(receiver)?;
                let inst = self.builder.ins().call(func, &[arr_ptr]);
                let result = self.builder.inst_results(inst)[0];
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, result)?;
                Ok(Some(()))
            }
            // f64 scalar broadcast — returns a new Array<number>.
            "scale" | "addScalar" | "add_scalar" => {
                if rest_args.len() != 1 {
                    return Ok(None);
                }
                if !matches!(elem, NativeKind::Float64) {
                    return Ok(None);
                }
                let func = match method_name {
                    "scale" => self.ffi.v2_array_scale_f64,
                    "addScalar" | "add_scalar" => self.ffi.v2_array_add_scalar_f64,
                    _ => unreachable!(),
                };
                let arr_ptr = self.read_place(receiver)?;
                let raw = self.compile_operand_raw(&rest_args[0])?;
                let scalar = self.coerce_to_v2_elem(raw, NativeKind::Float64);
                let inst = self.builder.ins().call(func, &[arr_ptr, scalar]);
                let new_arr = self.builder.inst_results(inst)[0];
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, new_arr)?;
                Ok(Some(()))
            }
            // f64 element-wise binary ops — both operands are Array<number>,
            // returns a new Array<number>.
            "addArray" | "add_array" | "mulArray" | "mul_array" => {
                if rest_args.len() != 1 {
                    return Ok(None);
                }
                if !matches!(elem, NativeKind::Float64) {
                    return Ok(None);
                }
                let func = match method_name {
                    "addArray" | "add_array" => self.ffi.v2_array_add_f64,
                    "mulArray" | "mul_array" => self.ffi.v2_array_mul_f64,
                    _ => unreachable!(),
                };
                let arr_ptr = self.read_place(receiver)?;
                let other = self.compile_operand_raw(&rest_args[0])?;
                // The other argument is an Array<number> (pointer); no coercion
                // needed, but make sure the value type is i64 before handoff.
                let other_i64 = {
                    let ty = self.builder.func.dfg.value_type(other);
                    if ty == types::I64 {
                        other
                    } else {
                        // Fall back to generic dispatch if we couldn't resolve
                        // the other operand to a plain pointer-sized value.
                        return Ok(None);
                    }
                };
                let inst = self.builder.ins().call(func, &[arr_ptr, other_i64]);
                let new_arr = self.builder.inst_results(inst)[0];
                self.release_old_value_if_heap(destination)?;
                self.write_place(destination, new_arr)?;
                Ok(Some(()))
            }
            _ => Ok(None),
        }
    }

    /// Inline typed array element read.
    ///
    /// Emits:
    /// 1. Load `data` pointer from `[arr_ptr + 8]`
    /// 2. Load `len` (u32) from `[arr_ptr + 16]`
    /// 3. Bounds check: `if index >= len` raise an out-of-bounds error
    ///    (early `return_` of `JIT_SIGNAL_INDEX_OUT_OF_BOUNDS` — VM/JIT parity)
    /// 4. Compute element address: `data + index * elem_size`
    /// 5. Load element with the correct Cranelift type
    ///
    /// `arr_ptr` is a Cranelift `i64` value pointing to a `TypedArrayHeader`.
    /// `index` is a Cranelift `i32` value (unsigned index).
    /// Returns the loaded element value (type depends on `elem_type`).
    pub fn v2_array_get(&mut self, arr_ptr: Value, index: Value, elem_type: NativeKind) -> Value {
        let (cl_type, elem_size) = elem_type_info(elem_type);

        // 1. Load data pointer (i64) from arr_ptr + DATA_PTR_OFFSET
        let data_ptr =
            self.builder
                .ins()
                .load(types::I64, MemFlags::trusted(), arr_ptr, DATA_PTR_OFFSET);

        // 2. Load length (u32) from arr_ptr + LEN_OFFSET
        let len = self
            .builder
            .ins()
            .load(types::I32, MemFlags::trusted(), arr_ptr, LEN_OFFSET);

        // 3. Bounds check: if index >= len, branch to out-of-bounds block
        let in_bounds_block = self.builder.create_block();
        let oob_block = self.builder.create_block();
        let merge_block = self.builder.create_block();

        // The merge block receives the result as a block parameter.
        self.builder.append_block_param(merge_block, cl_type);

        let cmp = self.builder.ins().icmp(IntCC::UnsignedLessThan, index, len);
        self.builder
            .ins()
            .brif(cmp, in_bounds_block, &[], oob_block, &[]);

        // ── Out-of-bounds path: raise an out-of-bounds error ────────────
        //
        // WS-3 F1: the prior codegen fabricated the element-type zero here
        // (a default-constant `jump merge`). That silently produced a value
        // for a memory-unsafe access the VM correctly rejects with
        // `VMError::IndexOutOfBounds` — a VM/JIT divergence. The MirToIR
        // function returns `i32` (the `JittedStrategyFn` ABI), so an early
        // `return_` of the `JIT_SIGNAL_INDEX_OUT_OF_BOUNDS` signal is
        // type-correct. The executor maps that signal back to the VM's
        // `Index out of bounds` diagnostic. Mirrors the
        // `compile_int_divmod_guarded` clean-error fall-through shape.
        self.builder.switch_to_block(oob_block);
        self.builder.seal_block(oob_block);
        let oob_signal = self.narrow_iconst(
            types::I32,
            crate::context::JIT_SIGNAL_INDEX_OUT_OF_BOUNDS as i64,
        );
        self.builder.ins().return_(&[oob_signal]);

        // ── In-bounds path: compute address and load element ────────────
        self.builder.switch_to_block(in_bounds_block);
        self.builder.seal_block(in_bounds_block);

        // 4. Compute byte offset: index (u32) -> i64, then * elem_size
        let index_i64 = self.builder.ins().uextend(types::I64, index);
        let byte_offset = if (elem_size as u64).is_power_of_two() {
            let shift = (elem_size as u64).trailing_zeros() as i64;
            self.builder.ins().ishl_imm(index_i64, shift)
        } else {
            let size_val = self.builder.ins().iconst(types::I64, elem_size);
            self.builder.ins().imul(index_i64, size_val)
        };
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);

        // 5. Load element with trusted flags (bounds already checked)
        let loaded = self
            .builder
            .ins()
            .load(cl_type, MemFlags::trusted(), elem_addr, 0);

        self.builder.ins().jump(merge_block, &[loaded]);

        // ── Merge ───────────────────────────────────────────────────────
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);

        self.builder.block_params(merge_block)[0]
    }

    /// Inline typed array length.
    ///
    /// Emits a single `load i32 [arr_ptr + 16]`.
    pub fn v2_array_len(&mut self, arr_ptr: Value) -> Value {
        self.builder
            .ins()
            .load(types::I32, MemFlags::trusted(), arr_ptr, LEN_OFFSET)
    }

    /// Inline typed array element write.
    ///
    /// Emits:
    /// 1. Load `data` pointer from `[arr_ptr + 8]`
    /// 2. Load `len` (u32) from `[arr_ptr + 16]`
    /// 3. Bounds check: `if index >= len` raise an out-of-bounds error
    ///    (early `return_` of `JIT_SIGNAL_INDEX_OUT_OF_BOUNDS` — VM/JIT parity)
    /// 4. Compute element address: `data + index * elem_size`
    /// 5. Store element with the correct Cranelift type
    ///
    /// `val` must be a Cranelift value whose type matches `elem_type`.
    pub fn v2_array_set(
        &mut self,
        arr_ptr: Value,
        index: Value,
        val: Value,
        elem_type: NativeKind,
    ) {
        let (_cl_type, elem_size) = elem_type_info(elem_type);

        // 1. Load data pointer
        let data_ptr =
            self.builder
                .ins()
                .load(types::I64, MemFlags::trusted(), arr_ptr, DATA_PTR_OFFSET);

        // 2. Load length
        let len = self
            .builder
            .ins()
            .load(types::I32, MemFlags::trusted(), arr_ptr, LEN_OFFSET);

        // 3. Bounds check
        let in_bounds_block = self.builder.create_block();
        let oob_block = self.builder.create_block();
        let continue_block = self.builder.create_block();

        let cmp = self.builder.ins().icmp(IntCC::UnsignedLessThan, index, len);
        self.builder
            .ins()
            .brif(cmp, in_bounds_block, &[], oob_block, &[]);

        // ── Out-of-bounds path: raise an out-of-bounds error ────────────
        //
        // WS-3 F1: the prior codegen silently skipped the store on OOB
        // (`brif` fell through to `continue_block`). That diverges from the
        // VM, which rejects the access with `VMError::IndexOutOfBounds`. The
        // MirToIR function returns `i32`, so an early `return_` of the
        // `JIT_SIGNAL_INDEX_OUT_OF_BOUNDS` signal is type-correct; the
        // executor maps it to the VM's `Index out of bounds` diagnostic.
        self.builder.switch_to_block(oob_block);
        self.builder.seal_block(oob_block);
        let oob_signal = self.narrow_iconst(
            types::I32,
            crate::context::JIT_SIGNAL_INDEX_OUT_OF_BOUNDS as i64,
        );
        self.builder.ins().return_(&[oob_signal]);

        // ── In-bounds path: store element ───────────────────────────────
        self.builder.switch_to_block(in_bounds_block);
        self.builder.seal_block(in_bounds_block);

        let index_i64 = self.builder.ins().uextend(types::I64, index);
        let byte_offset = if (elem_size as u64).is_power_of_two() {
            let shift = (elem_size as u64).trailing_zeros() as i64;
            self.builder.ins().ishl_imm(index_i64, shift)
        } else {
            let size_val = self.builder.ins().iconst(types::I64, elem_size);
            self.builder.ins().imul(index_i64, size_val)
        };
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);

        self.builder
            .ins()
            .store(MemFlags::trusted(), val, elem_addr, 0);

        self.builder.ins().jump(continue_block, &[]);

        // ── Continue ────────────────────────────────────────────────────
        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════
