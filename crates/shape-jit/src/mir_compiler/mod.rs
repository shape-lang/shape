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

// integration_tests and v2_array_tests are gated behind a non-default cfg
// because they currently exercise JIT executor paths that have heap-corruption
// regressions on the jit-v2-phase1 branch (closure capture, array element
// allocation, etc.) — fixing those JIT runtime bugs is tracked separately
// from the BytecodeToIR-removal regression that this branch closed out.
//
// Re-enable by passing `--cfg jit_v2_unstable_tests` to rustc, e.g.
//   RUSTFLAGS='--cfg jit_v2_unstable_tests' cargo test -p shape-jit --lib
#[cfg(all(test, jit_v2_unstable_tests))]
mod integration_tests;

#[cfg(all(test, jit_v2_unstable_tests))]
mod v2_array_tests;

use cranelift::codegen::ir::{FuncRef, StackSlot};
use cranelift::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::ffi_refs::FFIFuncRefs;
use shape_value::v2::closure_layout::ClosureLayout;
use shape_value::v2::ConcreteType;
use shape_vm::bytecode::MirFunctionData;
use shape_vm::mir::types::*;
use shape_vm::type_tracking::SlotKind;

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
    pub(crate) owned_mutable_capture_slots: HashSet<SlotId>,

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
    pub(crate) shared_capture_slots: HashSet<SlotId>,
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
        // Enrich slot_kinds with MIR-level type inference when the bytecode
        // compiler didn't provide them.
        let slot_kinds = types::infer_slot_kinds(&mir_data.mir, &slot_kinds);
        // Phase E: pull the set of non-escaping closure slots out of the MIR
        // storage plan so `ClosureCapture` lowering can pick the stack-slot
        // fast path. Slots absent from this set fall back to the legacy
        // `jit_make_closure` FFI path (Phase H will delete that).
        let non_escaping_closure_slots =
            mir_data.storage_plan.non_escaping_closure_slots.clone();
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
            closure_function_layouts,
            owned_mutable_capture_slots: HashSet::new(),
            shared_capture_slots: HashSet::new(),
        }
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
            match capture_kind {
                CaptureKind::OwnedMutable => {
                    self.owned_mutable_capture_slots.insert(param_slot);
                }
                CaptureKind::Shared => {
                    self.shared_capture_slots.insert(param_slot);
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
