//! MIR-to-Cranelift IR compiler (JIT v2).
//!
//! Compiles directly from Shape's MIR (Mid-level IR) to Cranelift IR,
//! preserving CFG structure, ownership semantics (Move/Copy/Drop),
//! liveness, and storage plans that are lost in the bytecode encoding.
//!
//! # Architecture
//!
//! ```text
//! AST → MIR (existing) → BorrowAnalysis + Liveness + StoragePlan (existing)
//!                      → MirToIR (this module) → Cranelift IR → native code
//! ```
//!
//! # Key differences from BytecodeToIR
//!
//! - **1:1 block mapping**: MIR BasicBlocks map directly to Cranelift blocks
//! - **Ownership-aware**: Move nulls the source, Copy retains, Drop releases
//! - **~7 statement kinds** vs ~100 bytecode opcodes
//! - **Explicit Drop points**: Scope cleanup from MIR, not heuristic

mod blocks;
pub mod bounds_elision;
mod conversions;
mod ownership;
mod places;
mod rvalues;
mod statements;
mod terminators;
pub(crate) mod types;
pub(crate) mod v2_array;
pub(crate) mod v2_field;
pub(crate) mod v2_int;
pub(crate) mod v2_refcount;
pub(crate) mod v2_string;
pub(crate) mod v2_typed_map;

// Heavy execution-path tests — gated behind the `deep-tests` feature.
// Each test calls JITExecutor::execute_program, which JIT-compiles ~118
// stdlib functions via MirToIR. Running them on the default Tier 1 path
// at n-cpu parallelism makes the shape-jit test binary slow enough to
// miss the summary line and racy enough to SIGILL in the JIT code cache.
// See `just test-deep` to run.
#[cfg(all(test, feature = "deep-tests"))]
mod integration_tests;

#[cfg(all(test, feature = "deep-tests"))]
mod v2_array_tests;

// Un-gated: pins the fix-jit-lead arg_count ABI / closure-param typing
// / ClosureRaw decode commits. Keeps the primary regression gate green
// on the default test path (no RUSTFLAGS).
#[cfg(test)]
mod closure_dispatch_regression_tests;

use cranelift::codegen::ir::{FuncRef, StackSlot};
use cranelift::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::ffi_refs::FFIFuncRefs;
use shape_value::v2::closure_layout::ClosureLayout;
use shape_value::v2::struct_layout::FieldKind;
use shape_value::v2::ConcreteType;
use shape_vm::bytecode::MirFunctionData;
use shape_vm::mir::types::*;
use shape_vm::type_tracking::SlotKind;

/// Session 2: side-table entry for a non-escaping stack closure call.
///
/// Carries the function_id, per-capture byte offset, and per-capture
/// Cranelift type recorded at `emit_stack_closure` time. The indirect
/// `Call` terminator consults this when the callee operand resolves to
/// a slot in `MirToIR::stack_closure_call_info` and emits a direct
/// `user_func_refs[function_id]` call with captures loaded from the
/// stack slot instead of routing through the `jit_call_value` FFI.
#[derive(Debug, Clone)]
pub(crate) struct StackClosureCallInfo {
    /// Target function_id (matches the `StackClosure.function_id` field).
    pub(crate) function_id: u16,
    /// Per-capture byte offset inside the `StackSlot`.
    pub(crate) capture_offsets: Vec<i32>,
    /// Per-capture native Cranelift type (F64 / I64 / I32 / I16 / I8 / Bool).
    pub(crate) capture_types: Vec<cranelift::prelude::Type>,
}

/// MIR-to-Cranelift IR compiler.
///
/// Each instance compiles a single MIR function. Reuses the JIT's existing
/// FFI infrastructure (250+ function references) and type mapping.
pub struct MirToIR<'a, 'b> {
    /// Cranelift function builder.
    pub(crate) builder: &'a mut FunctionBuilder<'b>,
    /// JITContext pointer (passed as first function parameter).
    pub(crate) ctx_ptr: Value,
    /// FFI function references (arc_retain, arc_release, print, etc.).
    pub(crate) ffi: FFIFuncRefs,
    /// The caller's entry block (already created, with function params).
    /// MIR bb0 maps to this block instead of creating a new one.
    pub(crate) entry_block: Block,

    // ── Block mapping ──────────────────────────────────────────────
    /// MIR BasicBlockId → Cranelift Block.
    pub(crate) block_map: HashMap<BasicBlockId, Block>,

    // ── Local variables ────────────────────────────────────────────
    /// MIR SlotId → Cranelift Variable.
    pub(crate) locals: HashMap<SlotId, Variable>,
    /// Type info for each local slot (from MIR's LocalTypeInfo).
    pub(crate) local_types: Vec<LocalTypeInfo>,
    /// Frame descriptor slot kinds (from bytecode Function.frame_descriptor),
    /// enriched by MIR-level type inference.
    pub(crate) slot_kinds: Vec<SlotKind>,
    /// v2: Per-slot fully-resolved `ConcreteType` from the bytecode compiler's
    /// `function_local_concrete_types` / `top_level_local_concrete_types`
    /// side-tables. Used by the v2 typed-array codegen path. Empty when the
    /// bytecode compiler did not populate the side-table — callers fall back
    /// to the legacy NaN-boxed path.
    pub(crate) concrete_types: Vec<ConcreteType>,
    /// Next Cranelift variable index.
    pub(crate) next_var: usize,

    // ── MIR data ───────────────────────────────────────────────────
    /// The MIR function being compiled.
    pub(crate) mir: &'a MirFunction,
    /// Borrow analysis (for ownership decisions).
    pub(crate) mir_data: &'a MirFunctionData,
    /// String table for resolving StringId constants.
    pub(crate) strings: &'a [String],
    /// Function name → index mapping for resolving Call terminators.
    pub(crate) function_indices: &'a HashMap<String, u16>,

    // ── Direct call support ─────────────────────────────────────────
    /// Function index → Cranelift FuncRef for direct calls (bypasses FFI).
    pub(crate) user_func_refs: HashMap<u16, FuncRef>,
    /// Function index → arity for call validation.
    pub(crate) user_func_arities: HashMap<u16, u16>,

    // ── Borrow support ──────────────────────────────────────────────
    /// MIR SlotId → (Cranelift StackSlot, Cranelift Type) for references
    /// created by `Rvalue::Borrow`. After calls, all referenced locals are
    /// reloaded from their stack slots using the recorded native type.
    ///
    /// R4.2F: the type is tracked so `reload_referenced_locals` can issue a
    /// native-width `stack_load` that matches both the `stack_store` width
    /// and the declared variable type. Non-native slot kinds map to I64 via
    /// `cranelift_type_for_slot`, collapsing to the legacy 8-byte cell.
    pub(crate) ref_stack_slots: HashMap<SlotId, (StackSlot, Type)>,
    /// Mapping from field name to byte offset within a TypedObject.
    pub(crate) field_byte_offsets: HashMap<String, u16>,

    // ── Closure Spec Phase E: stack-allocated closures ──────────────
    /// Slots that hold a non-escaping closure value, per the MIR
    /// storage plan's `non_escaping_closure_slots`. When a
    /// `StatementKind::ClosureCapture` targets a slot in this set,
    /// codegen allocates a Cranelift `StackSlot` shaped like
    /// `StackClosure { function_id: u32, type_id: u32, captures... }`
    /// instead of calling `jit_make_closure`. Cranelift's SROA then
    /// eliminates the slot when Phase C has inlined the closure body
    /// and the env pointer is dead.
    pub(crate) non_escaping_closure_slots: HashSet<SlotId>,
    /// MIR SlotId → Cranelift `StackSlot` backing a non-escaping
    /// closure. Populated on `ClosureCapture`. Used by drop/release
    /// paths to skip `arc_release` on stack-resident closure handles
    /// and by other consumers that need to know the slot is stack-resident.
    pub(crate) stack_closure_slots: HashMap<SlotId, StackSlot>,

    /// Session 2: per-slot stack-closure call metadata captured alongside
    /// `stack_closure_slots`. When an indirect `Call` whose `func` operand
    /// resolves to a slot in this map dispatches the closure, the
    /// terminator can bypass `jit_call_value` entirely — the function_id
    /// and capture byte offsets/Cranelift types are baked into codegen.
    ///
    /// This closes the hole where a stack closure's callee bits are a raw
    /// stack pointer (no NaN-box tag, no `HK_CLOSURE` header) that the
    /// FFI dispatcher can't recognise — the fix is to not dispatch through
    /// the FFI at all when the JIT itself built the closure.
    pub(crate) stack_closure_call_info:
        HashMap<SlotId, StackClosureCallInfo>,

    // ── Closure Spec Phase H1: heap-allocated closure codegen ──────
    /// Map from closure body `function_id` to its `ClosureLayout`.
    /// When present, `emit_heap_closure` uses the layout to emit inline
    /// Cranelift code that allocates a `TypedClosureHeader`-shaped block
    /// and writes captures at their natural-width offsets, replacing the
    /// legacy `jit_make_closure` FFI call. Absent entries fall back to
    /// the FFI path (e.g. when loading a cached program from disk, which
    /// doesn't carry layout metadata).
    pub(crate) closure_function_layouts: HashMap<u16, Arc<ClosureLayout>>,

    // ── Track A.1D.2: OwnedMutable capture side-table ──────────────
    /// Local slots whose Cranelift variable holds the raw `*mut ValueWord`
    /// bits of an `OwnedMutable` capture cell (allocated by
    /// `jit_alloc_owned_mut_cell` in `emit_heap_closure`). For a closure
    /// compiled under this `MirToIR`, the leading `N` entries of
    /// `MirFunction::param_slots` correspond to captures in the same
    /// order as `ClosureLayout::capture_kinds`; each slot whose
    /// `capture_storage_kind(i) == OwnedMutable` is recorded here.
    ///
    /// Effects on the lowering pipeline:
    /// - `read_place(Local(s))` emits `load.i64 [cell_ptr, 0]` (matches
    ///   the interpreter's `op_load_owned_mutable_capture` fresh read).
    /// - `write_place(Local(s), v)` emits `store.i64 v, [cell_ptr, 0]`
    ///   (matches the interpreter's `op_store_owned_mutable_capture`
    ///   fresh write — no old-value release, no retain).
    /// - `null_place` / `release_old_value_if_heap` / `emit_drop` all
    ///   early-return for these slots: the cell pointer bits must
    ///   survive for the entire frame so every read/write finds the
    ///   right box, and the box is reclaimed exactly once by
    ///   `release_typed_closure`'s `Box::from_raw` loop (see
    ///   `ClosureLayout::owned_mutable_capture_mask`, A.1A).
    ///
    /// Empty when the function being compiled is not a closure body,
    /// or has no OwnedMutable captures — non-closure functions then
    /// behave identically to pre-A.1D.2.
    ///
    /// Wave C.2: the value carries the `FieldKind` of the cell's interior
    /// payload (from `ClosureLayout::capture_inner_kind`). The Cranelift
    /// codegen for `read_place` / `write_place` dispatches on this kind
    /// to select the matching per-FieldKind FFI helper
    /// (`jit_read_owned_mut_cell_<kind>` / `jit_write_owned_mut_cell_<kind>`),
    /// so values cross the cell boundary as native Cranelift types
    /// (i64/f64/i32/...) instead of NaN-boxed ValueWord bits.
    pub(crate) owned_mutable_capture_slots: HashMap<SlotId, FieldKind>,

    // ── Track A.1E: Shared capture side-table ─────────────────────
    /// Local slots whose Cranelift variable holds the raw
    /// `*const SharedCell` bits of a `Shared` capture cell (retained via
    /// `jit_arc_shared_retain` in `emit_heap_closure`). Structurally
    /// parallel to `owned_mutable_capture_slots`: the leading `N`
    /// entries of `MirFunction::param_slots` are captures, and each slot
    /// whose `capture_storage_kind(i) == Shared` is recorded here.
    ///
    /// Effects on the lowering pipeline:
    /// - `read_place(Local(s))` emits the inline lock fast path (CAS
    ///   state byte 0→1 with `Acquire` ordering; on failure, call
    ///   `jit_shared_lock_contended`), then `load.i64 [cell_ptr,
    ///   SHARED_CELL_VALUE_OFFSET]`, then inline unlock fast path
    ///   (CAS 1→0 with `Release` ordering; on failure, call
    ///   `jit_shared_unlock_contended`). Matches the interpreter's
    ///   `op_load_shared_capture` handler semantics (take mutex, clone
    ///   inner bits, drop guard).
    /// - `write_place(Local(s), v)` emits the same lock fast path,
    ///   then `store.i64 v, [cell_ptr, SHARED_CELL_VALUE_OFFSET]`,
    ///   then the unlock fast path. Matches
    ///   `op_store_shared_capture` (take mutex, write, drop guard).
    /// - `null_place` / `release_old_value_if_heap` / `emit_drop` all
    ///   early-return for these slots: the Arc pointer bits must
    ///   survive for the entire frame so every read/write finds the
    ///   right cell, and the share is reclaimed exactly once by
    ///   `release_typed_closure`'s `Arc::from_raw` loop (see
    ///   `ClosureLayout::shared_capture_mask`, A.1A).
    ///
    /// Mutually exclusive with `owned_mutable_capture_slots` per the
    /// `ClosureLayout` invariant (the three capture-kind masks are
    /// disjoint). Empty when the function being compiled is not a
    /// closure body, or has no Shared captures.
    ///
    /// Wave C.2: like `owned_mutable_capture_slots`, the value carries
    /// the inner `FieldKind` so the Cranelift load/store after the
    /// inline `emit_shared_lock` dispatches to the correct native
    /// width at `[cell_ptr + SHARED_CELL_VALUE_OFFSET]`. We keep the
    /// inline lock/unlock — only the per-kind direct load/store is
    /// per-FieldKind — to avoid double-locking through the
    /// `read_shared_cell_<kind>` FFI on the JIT hot path.
    pub(crate) shared_capture_slots: HashMap<SlotId, FieldKind>,

    // ── Session 1 Commit 3: outer-scope Shared-cell slot side-table ─
    /// Local slots whose `BindingStorageClass` is `SharedCow` — i.e.
    /// outer-scope `var` bindings that escape into a closure and hence
    /// get promoted to `Arc<SharedCell>` storage by the bytecode
    /// compiler (`AllocSharedLocal` on promotion;
    /// `Load/StoreSharedLocal` on every subsequent access;
    /// `DropSharedLocal` at scope exit — see
    /// `shape-vm/src/executor/variables/mod.rs`).
    ///
    /// MIR doesn't reflect that promotion directly — it emits plain
    /// `Assign(Local(s), ...)` and `Drop(Local(s))` on the slot — so
    /// the JIT must recognise SharedCow slots via this side-table and
    /// dispatch read/write/drop to the lock-gated + Arc-lifecycle
    /// lowering path.
    ///
    /// Effects on the lowering pipeline:
    /// - `initialize_shared_local_slots` (called once at the start of
    ///   `compile`) allocates a fresh `Arc<SharedCell>` per slot via
    ///   `jit_alloc_shared_cell(NONE_BITS)` and stores the pointer
    ///   bits into the slot's Cranelift variable.
    /// - `read_place(Local(s))` emits the inline lock-gated
    ///   `load.i64 [cell_ptr + SHARED_CELL_VALUE_OFFSET]` (same lowering
    ///   as `shared_capture_slots` — see
    ///   `emit_shared_lock`/`emit_shared_unlock`).
    /// - `write_place(Local(s), v)` emits the matching lock-gated
    ///   store.
    /// - `emit_drop(Local(s))` calls `jit_arc_shared_release` to
    ///   consume the slot's strong share.
    /// - `compile_operand_for_shared_capture` (new) emits a raw
    ///   pointer read — bypassing the lock — so `ClosureCapture`
    ///   operands install the outer cell pointer into the closure's
    ///   Shared capture slot without locking.
    ///
    /// Disjoint from `owned_mutable_capture_slots` and
    /// `shared_capture_slots` — those are leading-capture param slots
    /// of a closure BODY; `shared_local_slots` is a declaring-scope
    /// slot in the outer function.
    pub(crate) shared_local_slots: HashSet<SlotId>,

    // ── JIT-side back-patch for unresolved ClosurePlaceholder ──────
    /// Per-placeholder function_id, populated at construction by scanning the
    /// MIR in block/statement order. Each entry corresponds to a
    /// `ClosurePlaceholder` assign that the bytecode compiler's back-patcher
    /// did NOT replace with `Function(name)` — typically because
    /// `closure_function_ids` got cleared by a monomorphization-triggered
    /// `compile_function` call before the top-level-MIR patching ran.
    ///
    /// When `compile_constant(MirConstant::ClosurePlaceholder)` is invoked,
    /// we pop the head of this queue (via `next_closure_placeholder_idx`)
    /// and NaN-box the corresponding function id so the stack carries a
    /// proper `TAG_FUNCTION` bit pattern instead of literal 0. Without this
    /// the JIT's `jit_call_value` sees `0x0` for a no-capture closure and
    /// bails out with "callee is neither function nor closure", which is
    /// the root cause of the gated `parity_array_map/filter/reduce`
    /// failures.
    ///
    /// Empty for MIRs that have no unresolved placeholders, for closure
    /// bodies themselves (their `ClosureCapture` statements carry the
    /// resolved `function_id` directly), and for functions whose
    /// back-patching already succeeded.
    pub(crate) closure_placeholder_fids: Vec<u16>,
    /// Cursor into `closure_placeholder_fids`. Incremented once per call to
    /// `compile_constant(ClosurePlaceholder)`. Statement visit order during
    /// `compile_body` matches the scan order used by
    /// `scan_closure_placeholder_fids`, so this is a stable pairing.
    pub(crate) next_closure_placeholder_idx: std::cell::Cell<usize>,
    /// Bounds-check elision plan: pairs `(arr_slot, iv_slot)` for which
    /// `Place::Index(Local(arr), Operand::*(Local(iv)))` accesses can skip
    /// the inline bounds check. Populated by callers via
    /// `set_bounds_elision_plan` after running
    /// `bounds_elision::analyze(mir)`. Empty by default — falls back to
    /// the bounds-checked path, preserving the v2_array_tests OOB
    /// zero-default semantics.
    pub(crate) bounds_elision: bounds_elision::BoundsElisionPlan,
}

/// Result of MIR preflight check.
pub struct MirPreflightResult {
    /// Whether this function can be compiled via MirToIR.
    pub can_compile: bool,
    /// Reasons why compilation is not possible (empty if can_compile is true).
    pub blockers: Vec<String>,
}

/// Check if a function's MIR can be compiled by MirToIR.
///
/// Returns detailed preflight results. Functions with unsupported MIR
/// features (async, closures, complex places) fall back to BytecodeToIR.
pub fn preflight(mir_data: &MirFunctionData) -> MirPreflightResult {
    let mut blockers = Vec::new();

    for block in &mir_data.mir.blocks {
        for stmt in &block.statements {
            match &stmt.kind {
                StatementKind::Assign(place, rvalue) => {
                    if !is_simple_place(place) {
                        blockers.push(format!(
                            "complex place in assignment at {:?}",
                            stmt.span
                        ));
                    }
                    match rvalue {
                        // BinaryOp, UnaryOp, Use, Clone, Borrow, Aggregate are supported
                        _ => {}
                    }
                }
                StatementKind::Drop(place) => {
                    if !is_simple_place(place) {
                        blockers.push(format!("complex place in drop at {:?}", stmt.span));
                    }
                }
                StatementKind::TaskBoundary(_, _) => {
                    // TaskBoundary is a borrow-checker annotation — no-op at codegen time.
                }
                StatementKind::ClosureCapture { function_id, .. } => {
                    // ClosureCapture is supported when function_id has been patched
                    if function_id.is_none() {
                        blockers.push("ClosureCapture missing function_id".to_string());
                    }
                }
                _ => {}
            }
        }

        match &block.terminator.kind {
            TerminatorKind::Goto(_)
            | TerminatorKind::SwitchBool { .. }
            | TerminatorKind::Return
            | TerminatorKind::Unreachable => {}
            TerminatorKind::Call { .. } => {
                // Call terminators are now supported via FFI dispatch.
            }
        }
    }

    MirPreflightResult {
        can_compile: blockers.is_empty(),
        blockers,
    }
}

/// Check if a Place is supported by MirToIR.
/// Supports arbitrary nesting of Local, Field, and Index.
/// Only Deref (references) is unsupported.
fn is_simple_place(place: &Place) -> bool {
    match place {
        Place::Local(_) => true,
        Place::Field(inner, _) | Place::Index(inner, _) => is_simple_place(inner),
        Place::Deref(inner) => is_simple_place(inner),
    }
}

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Create a new MIR-to-IR compiler.
    ///
    /// `entry_block` is the Cranelift block already created by the caller
    /// (with function parameters appended). MIR bb0 maps to this block.
    pub fn new(
        builder: &'a mut FunctionBuilder<'b>,
        ctx_ptr: Value,
        ffi: FFIFuncRefs,
        mir_data: &'a MirFunctionData,
        slot_kinds: Vec<SlotKind>,
        strings: &'a [String],
        entry_block: Block,
        function_indices: &'a HashMap<String, u16>,
        user_func_refs: HashMap<u16, FuncRef>,
        user_func_arities: HashMap<u16, u16>,
    ) -> Self {
        Self::new_with_concrete_types(
            builder,
            ctx_ptr,
            ffi,
            mir_data,
            slot_kinds,
            Vec::new(),
            strings,
            entry_block,
            function_indices,
            user_func_refs,
            user_func_arities,
        )
    }

    /// Same as `new` but also accepts a per-slot `ConcreteType` vector for
    /// the v2 typed-array fast path. Empty vec → legacy NaN-boxed behaviour.
    pub fn new_with_concrete_types(
        builder: &'a mut FunctionBuilder<'b>,
        ctx_ptr: Value,
        ffi: FFIFuncRefs,
        mir_data: &'a MirFunctionData,
        slot_kinds: Vec<SlotKind>,
        concrete_types: Vec<ConcreteType>,
        strings: &'a [String],
        entry_block: Block,
        function_indices: &'a HashMap<String, u16>,
        user_func_refs: HashMap<u16, FuncRef>,
        user_func_arities: HashMap<u16, u16>,
    ) -> Self {
        Self::new_with_closure_layouts(
            builder,
            ctx_ptr,
            ffi,
            mir_data,
            slot_kinds,
            concrete_types,
            strings,
            entry_block,
            function_indices,
            user_func_refs,
            user_func_arities,
            HashMap::new(),
        )
    }

    /// Closure-spec Phase H1 constructor: also accepts a
    /// `function_id → ClosureLayout` map so `emit_heap_closure` can lay out
    /// captures for escaping closures without going through the
    /// `jit_make_closure` FFI. Passing an empty map degrades gracefully to
    /// the legacy FFI path (same behaviour as `new_with_concrete_types`).
    pub fn new_with_closure_layouts(
        builder: &'a mut FunctionBuilder<'b>,
        ctx_ptr: Value,
        ffi: FFIFuncRefs,
        mir_data: &'a MirFunctionData,
        slot_kinds: Vec<SlotKind>,
        concrete_types: Vec<ConcreteType>,
        strings: &'a [String],
        entry_block: Block,
        function_indices: &'a HashMap<String, u16>,
        user_func_refs: HashMap<u16, FuncRef>,
        user_func_arities: HashMap<u16, u16>,
        closure_function_layouts: HashMap<u16, Arc<ClosureLayout>>,
    ) -> Self {
        let local_types = mir_data.mir.local_types.clone();
        // Slot-numbering correction: the bytecode compiler's
        // `FrameDescriptor.slots` and the MIR's local slots use different
        // numbering. MIR reserves `SlotId(0)` for the implicit return
        // value (`__mir_return`) and numbers parameters starting at 1;
        // the bytecode compiler puts the first parameter at slot 0 with
        // no implicit return slot. Seeding MirToIR with bytecode
        // frame_descriptor kinds thus misaligns every slot by +1. In the
        // worst case this declares MIR's return slot with the bytecode
        // param's `SlotKind`, so a `return 7.0` write gets narrowed
        // (e.g. `F64 → Bool` via `ireduce`) and corrupts the return value.
        // Regression case: `fn get_val(flag: bool) -> number? { if flag
        // { return 7.0 } return None }` declared MIR slot 0 as `Bool`
        // because the bytecode put `flag` at index 0; writing the `7.0`
        // F64 through `ensure_kind(_, Bool)` truncated to 0 and
        // `None ?? 42.0` then evaluated to 42.0 for every branch.
        //
        // Until the two tables share a slot-numbering convention, drop
        // the bytecode seed and rely on MIR-level inference only.
        let _ = slot_kinds;
        let slot_kinds = types::infer_slot_kinds(&mir_data.mir, &[]);
        // Phase E: pull the set of non-escaping closure slots out of the MIR
        // storage plan so `ClosureCapture` lowering can pick the stack-slot
        // fast path. Slots absent from this set fall back to the legacy
        // `jit_make_closure` FFI path (Phase H will delete that).
        let non_escaping_closure_slots =
            mir_data.storage_plan.non_escaping_closure_slots.clone();

        // Session 1 Commit 3: scan `storage_plan` for outer-scope
        // local slots that actually get promoted to
        // `Arc<SharedCell>` storage at runtime. The bytecode
        // compiler emits `AllocSharedLocal` ONLY when a slot is
        // captured by a closure AND gets the Shared capture kind —
        // not for every SharedCow slot. The `SHAPE_V2_VAR_SHAREDCOW`
        // default classifies every `var` binding as SharedCow even
        // when it never escapes, so we cannot use the storage class
        // alone.
        //
        // The authoritative signal is `slot_semantics[slot]
        // .escape_status == Captured` AND
        // `slot_classes[slot] == SharedCow`. Captured-by-closure +
        // SharedCow is the exact condition under which the bytecode
        // compiler emits `AllocSharedLocal` (see
        // `expressions/closures.rs`'s `is_shared_local_slot` arm).
        //
        // Param slots (captures) are further excluded because they
        // are governed by the capture-side-tables
        // `owned_mutable_capture_slots` / `shared_capture_slots`.
        //
        // cell-identity #1: the storage-plan scan alone is NOT
        // sufficient. The MIR's storage planner classifies a slot's
        // ownership from `binding_semantics`, and on some pipelines
        // a `var` binding arrives at the planner as
        // `BindingOwnershipClass::OwnedImmutable` rather than
        // `Flexible` — so Rule 1b (`SHAPE_V2_VAR_SHAREDCOW` +
        // Flexible → SharedCow) does not fire and the slot lands as
        // `Direct` / `LocalMutablePtr` even though the bytecode
        // emits the `AllocSharedLocal` lifecycle against it. The
        // second scan below covers the gap by picking up every slot
        // that is an operand of a `ClosureCapture` whose layout
        // declares a `CaptureKind::Shared` capture at that position.
        use shape_vm::type_tracking::{BindingStorageClass, EscapeStatus};
        let param_slot_set: HashSet<SlotId> =
            mir_data.mir.param_slots.iter().copied().collect();
        let mut shared_local_slots: HashSet<SlotId> = HashSet::new();
        for (slot, class) in &mir_data.storage_plan.slot_classes {
            if !matches!(class, BindingStorageClass::SharedCow) {
                continue;
            }
            if param_slot_set.contains(slot) {
                continue;
            }
            // Only slots captured by a closure get the cell
            // promotion at the bytecode level. A `var` that never
            // escapes into a closure stays plain-valued in the
            // interpreter — the JIT must match that semantics or
            // diverge from the interpreter's view of the same slot.
            let is_captured = mir_data
                .storage_plan
                .slot_semantics
                .get(slot)
                .map(|sem| matches!(sem.escape_status, EscapeStatus::Captured))
                .unwrap_or(false);
            if !is_captured {
                continue;
            }
            shared_local_slots.insert(*slot);
        }

        // cell-identity #1: augment `shared_local_slots` by scanning
        // `ClosureCapture` statements whose `function_id` resolves to a
        // `ClosureLayout` with `CaptureKind::Shared` captures. The MIR
        // storage planner sometimes classifies `var` bindings as
        // `LocalMutablePtr` (not `SharedCow`) when the ownership class
        // for the slot is stored as `OwnedImmutable` in the MIR's
        // `binding_semantics` table, so the storage-plan scan above
        // misses them. The bytecode compiler still emits `AllocSharedLocal`
        // / `LoadSharedLocal` / `StoreSharedLocal` / `DropSharedLocal`
        // for those slots — and the closure body's JIT compilation
        // treats its capture param slot as `shared_capture_slots`
        // (it expects a `*const SharedCell` pointer). If the declaring
        // frame's JIT doesn't allocate an `Arc<SharedCell>` and doesn't
        // lock-gated route reads/writes through it, the closure gets a
        // plain scalar bit pattern as its "cell pointer" — and the
        // closure's first `jit_arc_shared_retain` on that value
        // segfaults. Driving the side-table off the layout's
        // `CaptureKind::Shared` mask closes the gap: any slot that is
        // an operand of a Shared capture in a call to a layout-carrying
        // function is promoted to the Arc<SharedCell> lowering path.
        use shape_value::v2::closure_layout::CaptureKind;
        use shape_vm::mir::types::{Operand as MirOperand, Place as MirPlace, StatementKind};
        for block in &mir_data.mir.blocks {
            for stmt in &block.statements {
                let StatementKind::ClosureCapture {
                    operands,
                    function_id,
                    ..
                } = &stmt.kind
                else {
                    continue;
                };
                let Some(fid) = *function_id else {
                    continue;
                };
                let Some(layout) = closure_function_layouts.get(&fid) else {
                    continue;
                };
                for (i, op) in operands.iter().enumerate() {
                    if i >= layout.capture_count() {
                        break;
                    }
                    if !matches!(layout.capture_storage_kind(i), CaptureKind::Shared) {
                        continue;
                    }
                    let root = match op {
                        MirOperand::Copy(p)
                        | MirOperand::Move(p)
                        | MirOperand::MoveExplicit(p) => match p {
                            MirPlace::Local(s) => Some(*s),
                            _ => None,
                        },
                        MirOperand::Constant(_) => None,
                    };
                    if let Some(slot) = root {
                        if param_slot_set.contains(&slot) {
                            // Capture-side slot: handled by the
                            // `shared_capture_slots` side-table via
                            // `register_owned_mutable_capture_slots`.
                            continue;
                        }
                        shared_local_slots.insert(slot);
                    }
                }
            }
        }

        // JIT-side fallback for unresolved `ClosurePlaceholder` constants.
        // See the `closure_placeholder_fids` doc-comment on `MirToIR` for
        // why this is needed; in short, monomorphization's `compile_function`
        // clears `closure_function_ids` in the bytecode compiler before the
        // top-level MIR back-patching runs, so some placeholders leak into
        // the MIR we receive. This scan produces the same pairing the
        // bytecode's back-patcher would have, keyed on MIR traversal order.
        let closure_placeholder_fids =
            scan_closure_placeholder_fids(&mir_data.mir, function_indices);

        Self {
            builder,
            ctx_ptr,
            ffi,
            entry_block,
            block_map: HashMap::new(),
            locals: HashMap::new(),
            local_types,
            slot_kinds,
            concrete_types,
            next_var: 0,
            mir: &mir_data.mir,
            mir_data,
            strings,
            function_indices,
            user_func_refs,
            user_func_arities,
            ref_stack_slots: HashMap::new(),
            field_byte_offsets: HashMap::new(),
            non_escaping_closure_slots,
            stack_closure_slots: HashMap::new(),
            stack_closure_call_info: HashMap::new(),
            closure_function_layouts,
            owned_mutable_capture_slots: HashMap::new(),
            shared_capture_slots: HashMap::new(),
            shared_local_slots,
            closure_placeholder_fids,
            next_closure_placeholder_idx: std::cell::Cell::new(0),
            bounds_elision: bounds_elision::BoundsElisionPlan::default(),
        }
    }

    /// Install a precomputed bounds-elision plan so `Place::Index` codegen
    /// can skip the inline bounds check on trusted access pairs.
    ///
    /// Callers normally invoke `bounds_elision::analyze(&mir_data.mir)` and
    /// pass the result here. Leaving the plan empty (the default) is
    /// always sound — every access falls back to the bounds-checked path,
    /// matching pre-elision behaviour and preserving the v2_array_tests
    /// OOB zero-default semantics.
    pub fn set_bounds_elision_plan(&mut self, plan: bounds_elision::BoundsElisionPlan) {
        self.bounds_elision = plan;
    }

    /// Track A.1D.2: register the leading capture param slots that back
    /// an `OwnedMutable` capture cell for the closure body currently being
    /// compiled.
    ///
    /// `captures_count` is the number of leading entries in
    /// `MirFunction::param_slots` that correspond to closure captures
    /// (the caller ABI stores captures before user params: `[ctx_ptr,
    /// capture_0..N, user_param_0..M]`). `layout` is the
    /// `ClosureLayout` for this function's `function_id`, so
    /// `layout.capture_storage_kind(i)` reports the per-capture
    /// `CaptureKind`. Slots whose kind is `OwnedMutable` are flagged —
    /// `read_place` and `write_place` then emit a pointer-deref load /
    /// store through the raw `*mut ValueWord` bits, matching the A.1B
    /// interpreter handlers.
    ///
    /// Also patches `self.slot_kinds` for each capture param slot using
    /// the layout's `capture_types[i]`. Closure params are untyped at
    /// the bytecode compiler level (see `compile_expr_closure` in
    /// `expressions/closures.rs` — capture params are synthesised with
    /// `type_annotation: None`), so MIR-level inference leaves them
    /// `Unknown`. Without per-capture kinds the `Rvalue::BinaryOp`
    /// lowering falls through to the dynamic-binop path, which
    /// unconditionally errors out (see `compile_binop` at
    /// `rvalues.rs::~411`). Patching the slot kind here lets the
    /// typed binop pickers (`compile_binop_int64`, `compile_binop_f64`,
    /// etc.) engage for `x + 1`-style closure-body arithmetic. For
    /// OwnedMutable slots, `read_place` always emits `load.i64` through
    /// the cell — the kind informs the binop picker about the inner
    /// value's representation (NaN-boxed int, NaN-boxed float, etc.),
    /// not the width of the slot itself.
    ///
    /// No-op for non-closure functions (`captures_count == 0`) and for
    /// closures whose layout marks every capture as `Immutable`. A.1E
    /// extends this registration to populate `shared_capture_slots`
    /// alongside `owned_mutable_capture_slots`; both side-tables are
    /// parallel in structure but drive different lowering paths (see
    /// their doc-comments on `MirToIR`).
    pub fn register_owned_mutable_capture_slots(
        &mut self,
        captures_count: u16,
        layout: &ClosureLayout,
    ) {
        use shape_value::v2::closure_layout::CaptureKind;
        let captures_count = captures_count as usize;
        if captures_count == 0 {
            return;
        }
        // Defensive: the layout must have a capture_kinds entry per
        // declared capture. A mismatch indicates a compiler bug upstream
        // (e.g. the layout was minted against a different signature); we
        // clamp to the smaller of the two so no out-of-bounds panics
        // slip into release builds.
        let len = captures_count.min(layout.capture_kinds.len());
        for (i, &param_slot) in self
            .mir
            .param_slots
            .iter()
            .take(len)
            .enumerate()
        {
            let capture_kind = layout.capture_storage_kind(i);
            let is_cell_capture = matches!(
                capture_kind,
                CaptureKind::OwnedMutable | CaptureKind::Shared
            );
            if !is_cell_capture {
                continue;
            }
            // Wave C.2: capture the cell's interior FieldKind alongside
            // the slot id so `read_place`/`write_place` can pick the
            // matching per-kind FFI helper instead of NaN-boxing through
            // the legacy ValueWord-bits path.
            let inner_kind = layout.capture_inner_kind(i);
            match capture_kind {
                CaptureKind::OwnedMutable => {
                    self.owned_mutable_capture_slots
                        .insert(param_slot, inner_kind);
                }
                CaptureKind::Shared => {
                    self.shared_capture_slots.insert(param_slot, inner_kind);
                }
                CaptureKind::Immutable => unreachable!(),
            }
            // Propagate the layout's known concrete type onto the
            // slot kind vector so `Rvalue::BinaryOp` lowering can
            // pick the typed arithmetic path. Only patch when the
            // slot was previously `Unknown`; a non-Unknown kind
            // from the bytecode frame descriptor wins. Same
            // treatment applies to OwnedMutable and Shared — in
            // both cases `read_place` returns an I64 ValueWord-
            // shaped value and the downstream binop picker keys on
            // the kind, not the cell-pointer width itself.
            if let Some(concrete) = layout.capture_types.get(i) {
                if let Some(kind) = types::elem_slot_kind_for_concrete(concrete) {
                    let idx = param_slot.0 as usize;
                    if idx < self.slot_kinds.len() {
                        if self.slot_kinds[idx]
                            == shape_vm::type_tracking::SlotKind::Unknown
                        {
                            self.slot_kinds[idx] = kind;
                        }
                    }
                }
            }
        }
    }

    /// Compile the MIR function to Cranelift IR.
    ///
    /// Returns Ok(()) on success. The actual return instructions are emitted
    /// by compile_terminator for TerminatorKind::Return blocks.
    /// Full compilation: create blocks, declare locals, initialize, compile body.
    /// Used when the caller hasn't set up blocks/locals externally.
    pub fn compile(&mut self) -> Result<(), String> {
        self.create_blocks();
        self.declare_locals();
        // Session 1 Commit 3: eagerly materialise Arc<SharedCell>s for
        // every SharedCow local slot. No-op when the set is empty.
        self.initialize_shared_local_slots();
        self.compile_body()
    }

    /// Compile the MIR function body (blocks already created, locals already declared).
    /// Called after the caller has optionally stored function params to local variables.
    /// `param_count` indicates how many leading slots are function params (skip init).
    pub fn compile_body(&mut self) -> Result<(), String> {
        if std::env::var_os("SHAPE_JIT_MIR_TRACE").is_some() {
            for (bi, block) in self.mir.blocks.iter().enumerate() {
                eprintln!("[mir-trace] bb{}: {} stmts, term={:?}",
                    bi, block.statements.len(), block.terminator.kind);
                for (si, stmt) in block.statements.iter().enumerate() {
                    eprintln!("[mir-trace]   s[{}]: {:?}", si, stmt.kind);
                }
            }
        }
        for block_idx in 0..self.mir.blocks.len() {
            let block = &self.mir.blocks[block_idx];
            let cl_block = self.block_map[&block.id];

            // bb0 is the caller's entry block — already switched to and sealed.
            // For other blocks, switch to the new block.
            if block_idx != 0 {
                self.builder.switch_to_block(cl_block);
            }

            // DON'T initialize locals here — the caller has already:
            // 1. Called declare_locals() (all vars declared)
            // 2. Stored function params to their slots (params have real values)
            // Cranelift's SSA handles undefined variables as 0/default.

            // Compile statements.
            for stmt in &block.statements {
                self.compile_statement(stmt)?;
            }

            // Compile terminator.
            self.compile_terminator(&block.terminator)?;
        }

        // Seal all blocks after all code is emitted. Sealing before all
        // predecessors are known causes Cranelift assertion failures.
        self.builder.seal_all_blocks();

        Ok(())
    }

    /// Reload all locals that have been borrowed via Rvalue::Borrow.
    ///
    /// After a function call, the callee may have mutated values through
    /// shared references. We conservatively reload all referenced locals
    /// from their StackSlots to keep Cranelift variables in sync.
    ///
    /// R4.2F: stack cells are now native-sized/aligned (matching the root
    /// local's Cranelift type), so `stack_load` directly produces a value
    /// of the declared variable's type — no NaN-box unboxing needed.
    pub(crate) fn reload_referenced_locals(&mut self) {
        let refs: Vec<_> = self
            .ref_stack_slots
            .iter()
            .map(|(&slot_id, &(stack_slot, cl_ty))| (slot_id, stack_slot, cl_ty))
            .collect();
        for (slot_id, stack_slot, cl_ty) in refs {
            let reloaded = self.builder.ins().stack_load(cl_ty, stack_slot, 0);
            if let Some(&var) = self.locals.get(&slot_id) {
                self.builder.def_var(var, reloaded);
            }
        }
    }
}

pub(crate) mod v2_call_abi;

/// Reconstruct the bytecode compiler's back-patch pairing for unresolved
/// `ClosurePlaceholder` constants in a MIR function, keyed on statement
/// traversal order.
///
/// # Why this exists
///
/// The bytecode compiler's `closure_function_ids` vector — the list of
/// `("__closure_N", function_id)` pairs used to patch
/// `MirConstant::ClosurePlaceholder` into `MirConstant::Function(name)` —
/// is cleared by `compile_function` (see
/// `shape-vm/src/compiler/functions.rs:510`). Monomorphization triggered
/// during a top-level call (e.g. `arr.map(|x| x*2)`) goes through
/// `ensure_monomorphic_function` → `compile_function`, clearing the
/// vector even though the top-level MIR's back-patching hasn't yet run.
/// The result: any no-capture closure literal in top-level code leaks
/// into the top-level MIR as a raw `ClosurePlaceholder`.
///
/// The downstream effect is that `compile_constant` lowers the
/// placeholder to a literal `iconst 0`, the callsite pushes `0x0` as the
/// callee, and `jit_call_value` BAILs with "callee is neither function
/// nor closure". The gated `parity_array_map/filter/reduce` tests all
/// hit this exact path.
///
/// # Scan semantics
///
/// Mirrors the bytecode compiler's loop in
/// `functions.rs::compile_function` (and the top-level analogue in
/// `compiler_impl_reference_model.rs`): walk every block's statement
/// list; a `ClosureCapture` with a resolved `function_id` "claims" the
/// immediately-following `ClosurePlaceholder` (which the patcher rewrites
/// to `Nop`), so the placeholder does NOT need independent resolution.
/// An unpaired placeholder consumes the next `__closure_<idx>` name.
///
/// # Name-to-id lookup
///
/// We can't consult `closure_function_ids` from the JIT, but
/// `function_indices` (built from `BytecodeProgram::functions`) has every
/// `__closure_N` entry in the same global ordering the bytecode compiler
/// assigned. Looking up `__closure_<idx>` directly produces the correct
/// function_id for each unpaired placeholder, in the same sequence the
/// unpatched back-patcher would have produced.
///
/// # Graceful degradation
///
/// If the lookup misses (e.g. a malformed program where `__closure_<idx>`
/// is absent), we push `u16::MAX` as a sentinel. `compile_constant`'s
/// fallback path then emits the legacy `iconst 0` so behaviour is no
/// worse than before this scan.
fn scan_closure_placeholder_fids(
    mir: &shape_vm::mir::types::MirFunction,
    function_indices: &std::collections::HashMap<String, u16>,
) -> Vec<u16> {
    use shape_vm::mir::types::{MirConstant, Operand, Rvalue, StatementKind};

    let mut result: Vec<u16> = Vec::new();
    let mut closure_idx: u32 = 0;
    let mut has_capture = false;
    for block in &mir.blocks {
        for stmt in &block.statements {
            let is_placeholder = matches!(
                &stmt.kind,
                StatementKind::Assign(
                    _,
                    Rvalue::Use(Operand::Constant(MirConstant::ClosurePlaceholder))
                )
            );
            if is_placeholder {
                if has_capture {
                    // Paired with a preceding ClosureCapture — the
                    // bytecode back-patcher would have turned this into
                    // a Nop. compile_constant still receives the
                    // placeholder for this slot (the patched MIR would
                    // not), so we record u16::MAX and let the fallback
                    // iconst(0) fire; ClosureCapture's function_id
                    // drives the actual closure allocation in
                    // `emit_heap_closure` / `emit_stack_closure`, and
                    // the subsequent Assign(slot, placeholder) is a
                    // dead store that write_place discards.
                    result.push(u16::MAX);
                    has_capture = false;
                } else {
                    let name = format!("__closure_{}", closure_idx);
                    let fid = function_indices.get(&name).copied().unwrap_or(u16::MAX);
                    result.push(fid);
                    closure_idx = closure_idx.saturating_add(1);
                }
                continue;
            }
            if let StatementKind::ClosureCapture {
                function_id: Some(_),
                ..
            } = &stmt.kind
            {
                // The bytecode patcher consumes one closure_id for the
                // capture itself — advance the counter so the unpaired
                // placeholder counter stays aligned with the compiler's.
                closure_idx = closure_idx.saturating_add(1);
                has_capture = true;
            }
        }
    }
    result
}
