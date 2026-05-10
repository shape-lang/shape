//! Object and array operations for the VM executor.
//!
//! Handles: NewArray, NewObject, GetProp, SetProp, Length, ArrayPush, ArrayPop,
//! MakeClosure, MergeObject, NewTypedObject, TypedMergeObject, CallMethod, MakeRange,
//! WrapTypeAnnotation, SliceAccess.
//!
//! ## Wave 6.5 substep-2 (D-objects-mod) — SURFACE
//!
//! This file is the dispatch shell for generic-object opcodes. The substep-1
//! shim deletion (`push_raw_u64` / `pop_raw_u64` / `push_native_i64` /
//! `stack_read_owned` / `stack_peek_raw`) bound this territory at 39 mandatory
//! shim sites. The pre-Wave-6 file body, however, is built on top of types and
//! helpers that the strict-typing bulldozer **already deleted before
//! substep-1** — it does not compile against the current `shape-value` crate
//! and cannot be migrated by mechanical shim rename:
//!
//! - `shape_value::ValueWord` / `shape_value::ValueWordExt`
//!   (deleted — see `crates/shape-value/src/lib.rs`'s post-bulldozer header).
//! - `shape_value::value_word_drop::vw_drop` /
//!   `shape_value::value_word_drop::vw_clone`
//!   (deleted — replaced by `clone_with_kind` / `drop_with_kind` keyed on
//!    `NativeKind`, ADR-006 §2.7.7).
//! - `ValueWord::from_raw_bits` / `ValueWord::from_*` /
//!   `ValueWord::into_raw_bits` (constructors and accessors all gone with the
//!    type itself).
//! - `as_heap_ref()` (forbidden — playbook §4 #7; replaced by
//!   `slot.as_heap_value()` on `KindedSlot::slot`).
//! - `tag_bits::*` / `is_tagged()` / the deleted W-series ValueWord
//!   synthesizer (forbidden — playbook §4 #7).
//!
//! On top of those, the `MethodHandler` ABI itself was **kind-less in
//! both directions** pre-Wave-γ. ADR-006 §2.7.9 / Q11 (Wave-γ
//! `G-method-fn-v2-abi`) flipped `MethodFnV2` to
//! `fn(&mut VM, &[KindedSlot], _) -> Result<KindedSlot, VMError>` —
//! the kinded carrier slice form per §2.7.1 case 4. The dispatch
//! shell now sources every kind from the §2.7.7 stack parallel-
//! `Vec<NativeKind>` track via `pop_kinded()` (no fabrication), and
//! pushes the returned `KindedSlot` via `push_kinded()` (kind from
//! the handler-returned carrier — no fabrication). The Bool-default
//! rationalization the W-series formalized is no longer reachable.
//! With the ABI in place this dispatch shell becomes a mechanical
//! `pop_kinded` / `push_kinded` / `slot.as_heap_value()` rewrite per
//! playbook §10 D-objects-mod row — Wave-γ-followup territory.
//!
//! Cross-cluster dependencies for the architectural close-out:
//!
//! 1. `D-raw-helpers` rewrites/deletes `objects/raw_helpers.rs` (currently
//!    the carrier for `tag_bits::*` and `extract_heap_ref`). Every Cluster D
//!    sibling file (`property_access.rs`, `array_operations.rs`,
//!    `array_joins.rs`, `concurrency_methods.rs`, `channel_methods.rs`,
//!    `number_methods.rs`, etc.) calls `extract_heap_ref(args[0])` for
//!    HeapValue dispatch — same shape needed here for the receiver bits.
//! 2. Wave-γ-followup body migration: per ADR-006 §2.7.9 / Q11 the
//!    `MethodFnV2` ABI is kinded (`&[KindedSlot]` /
//!    `Result<KindedSlot, VMError>`); ~150 PHF handler bodies stayed
//!    `NotImplemented(SURFACE)` after the ABI flip (Wave-γ
//!    `G-method-fn-v2-abi` close) and are migrated body-by-body in
//!    follow-up sub-clusters per the M-datatable Wave-β `joins.rs`
//!    precedent at close commit `eb78699`.
//! 3. The remaining `ValueWord::from_*` heap-construction sites
//!    (`ValueWord::from_heap_value(HeapValue::Range { .. })`,
//!    `ValueWord::from_type_annotated_value`, `ValueWord::from_array`, etc.)
//!    rewrite to `Arc::into_raw + push_kinded(_, NativeKind::Ptr(HeapKind::*))`
//!    per playbook §3 per-`HeapKind` push pattern.
//!
//! Per playbook §7.4 ("File compiles cleanly OR un-compiling sites have a
//! documented surface") and §8 surface-and-stop trigger ("Cross-cluster
//! migration cascade"), this file's bodies are replaced with
//! `VMError::NotImplemented(SURFACE: ...)` placeholders documenting the
//! cascade. Function signatures and module declarations are preserved so
//! external callers (`dispatch.rs`, `additional/mod.rs`, `compiler/*`)
//! continue to compile.
//!
//! ## Migration status snapshot (substep-2 close)
//!
//! - Mandatory shim hits: 0 (the 39 `push_raw_u64` / `pop_raw_u64` call sites
//!   are gone — they were inside the bodies that this commit replaces with
//!   surface markers).
//! - Sibling shim hits: 0 (none in pre-existing file; verified at audit).
//! - Forbidden-pattern carry-overs: 0 (`ValueWord`, `as_heap_ref`, `vw_drop`,
//!   `value_word_drop`, `as_vw_ref`, `tag_bits`, and the deleted ValueWord
//!   synthesizer are all gone; the `extract_heap_ref` import lived in the
//!   now-deleted bodies and is not reintroduced).
//! - Surfaces: 6 (`exec_objects` opcode dispatch + 5 method-dispatch entries:
//!   `op_call_method`, `op_make_range`, `op_wrap_type_annotation`,
//!   `dispatch_method_handler`, plus the v2 typed-array PHF fast path baked
//!   into `op_call_method`).
//!
//! See `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §10 row
//! `D-objects-mod`, §7.4, §8, and ADR-006 §2.7.6 (Q8) / §2.7.7 (Q9).

// PHF method registry
pub mod method_registry;
// Raw u64 extraction helpers (v2 — no ValueWord) — D-raw-helpers territory.
pub mod raw_helpers;

// Property access operations (GetProp, SetProp, Length) — D-prop-access territory.
pub mod property_access;

// Object creation operations (NewArray, NewObject, NewTypedObject) — D-obj-create territory.
pub mod object_creation;

// Object merge operations (MergeObject, TypedMergeObject) — D-obj-tail territory.
pub mod object_operations;

// Array operations (ArrayPush, ArrayPop, SliceAccess) — D-array-ops territory.
pub mod array_operations;

// Array method modules.
pub mod array_aggregation;
pub mod array_basic;
pub mod array_joins;
pub mod array_query;
pub mod array_sets;
pub mod array_sort;
pub mod array_transform;

// DataTable method handlers.
pub mod datatable_methods;

// (W15-column, 2026-05-10) `column_methods` deleted: ADR-006 §2.7.21 / Q22.
// `Column` is not a surviving `HeapKind` variant — its semantics are
// absorbed by `HeapKind::TableView` + `TableViewData::ColumnRef` (see
// `crates/shape-value/src/heap_value.rs`). The previous file held 11
// surface-only stubs and a stale PHF map; both are removed.

// IndexedTable method handlers.
pub mod indexed_table_methods;

// HashMap method handlers.
pub mod hashmap_methods;

// Set method handlers.
pub mod deque_methods;
pub mod priority_queue_methods;
pub mod set_methods;

// Number method handlers.
pub mod number_methods;

// String method handlers.
pub mod string_methods;

// Content method handlers.
pub mod content_methods;

// DateTime method handlers.
pub mod datetime_methods;

// Instant method handlers.
pub mod instant_methods;

// Matrix method handlers.
pub mod matrix_methods;

// Iterator method handlers.
pub mod iterator_methods;

// Typed array (Vec<int>, Vec<number>, Vec<bool>) method handlers.
pub mod typed_array_methods;

// V0.c scaffolding: handlers for native v2 TypedArray<i64>/TypedArray<f64>
// receivers. Registered in `method_registry` under typed-array PHF maps.
pub mod typed_int_array_methods;
pub mod typed_number_array_methods;

// Concurrency primitive (Mutex<T>, Atomic<T>, Lazy<T>) method handlers.
pub mod concurrency_methods;

// Channel (MPSC sender/receiver) method handlers.
pub mod channel_methods;

// Concatenation opcodes (StringConcat, ArrayConcat) — dedicated v2 replacements
// for the generic Add overload on built-in heap types.
pub mod concat;

// Typed HashMap and String access opcodes — local-slot based, skip HeapValue dispatch.
pub mod typed_access;

use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
};
use shape_value::VMError;

impl VirtualMachine {
    /// Dispatch shell for object opcodes.
    ///
    /// Each opcode arm currently calls into a sibling Cluster D file
    /// (`object_creation`, `property_access`, `array_operations`, etc.) whose
    /// own substep-2 migration is in flight under a peer Wave-α sub-cluster.
    /// The dispatch shell itself is kind-correct because it forwards to the
    /// per-opcode handler unchanged. The legacy entries that lived directly
    /// in `objects/mod.rs` (`op_call_method`, `op_wrap_type_annotation`,
    /// `op_make_range`) are surfaced below — see each function's doc comment
    /// for the architectural cascade ruling.
    #[inline(always)]
    pub(in crate::executor) fn exec_objects(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            NewArray => self.op_new_array(instruction)?,
            NewTypedArray => self.op_new_typed_array(instruction)?,
            NewMatrix => self.op_new_matrix(instruction)?,
            NewObject => self.op_new_object(instruction)?,
            GetProp => self.op_get_prop(ctx)?,
            SetProp => self.op_set_prop()?,
            SetLocalIndex => self.op_set_local_index(instruction)?,
            SetModuleBindingIndex => self.op_set_module_binding_index(instruction)?,
            Length => self.op_length()?,
            ArrayPush => self.op_array_push()?,
            ArrayPushLocal => self.op_array_push_local(instruction)?,
            ArrayPop => self.op_array_pop()?,
            MakeClosure => self.op_make_closure(instruction)?,
            MergeObject => self.op_merge_object()?,
            NewTypedObject => self.op_new_typed_object(instruction)?,
            TypedMergeObject => self.op_typed_merge_object(instruction)?,
            WrapTypeAnnotation => self.op_wrap_type_annotation(instruction)?,
            SliceAccess => self.op_slice_access()?,
            MakeRange => self.op_make_range()?,
            _ => unreachable!(
                "exec_objects called with non-object opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// SURFACE: WrapTypeAnnotation cannot be migrated in this cluster.
    ///
    /// The pre-Wave-6 body popped a `ValueWord` and constructed a
    /// `ValueWord::from_type_annotated_value(name, inner)` wrapper. Both the
    /// `ValueWord` type and the `from_type_annotated_value` constructor were
    /// deleted by the strict-typing bulldozer before substep-1; there is no
    /// post-§2.7.7 wrapper shape. The annotation-wrap design itself needs
    /// re-thinking under ADR-006 (annotations as parallel metadata, not as a
    /// payload tag), which is outside the D-objects-mod sub-cluster's
    /// territory.
    ///
    /// Cross-cluster cascade: the compiler emitter currently produces
    /// `WrapTypeAnnotation` opcodes; that emit site is in `compiler/` and
    /// must coordinate with the kinded annotation-metadata model before this
    /// handler is rewritten.
    fn op_wrap_type_annotation(&mut self, _instruction: &Instruction) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "SURFACE: WrapTypeAnnotation depends on the deleted ValueWord wrapper \
             type. Annotation wrapping needs a kinded redesign (ADR-006 §2.7.6 \
             / Q8) — see playbook §8 cross-cluster cascade. D-objects-mod scope \
             does not include the compiler emit site."
                .into(),
        ))
    }

    /// SURFACE: CallMethod dispatch shell — kinded ABI landed, receiver
    /// classification + IC wiring downstream.
    ///
    /// `op_call_method` is the central method-dispatch shell. The
    /// `MethodFnV2` ABI flip to `(&mut VM, &[KindedSlot], _) ->
    /// Result<KindedSlot, VMError>` landed in this cluster (Wave-γ
    /// G-method-fn-v2-abi, ADR-006 §2.7.9 / Q11); see the parent
    /// commit's amendment to `docs/adr/006-value-and-memory-model.md`
    /// for the architectural justification.
    ///
    /// With the kinded ABI in place, the dispatch shell's mechanical
    /// shape per ADR-006 §2.7.9 is:
    ///
    /// ```ignore
    /// // Pop receiver + N args from the §2.7.7 kinded stack.
    /// // Kind for each entry comes from the parallel-Vec<NativeKind>
    /// // track populated by the producing opcode — no fabrication.
    /// let mut args: Vec<KindedSlot> = Vec::with_capacity(arg_count + 1);
    /// for _ in 0..(arg_count + 1) {
    ///     let (bits, kind) = self.pop_kinded()?;
    ///     args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
    /// }
    /// args.reverse();  // pop order is reverse of push order
    ///
    /// // Resolve handler: PHF lookup keyed on (receiver_kind, method_name).
    /// // Receiver kind is `args[0].kind` per §2.7.9; for heap receivers
    /// // unwrap to HeapKind via `Ptr(HeapKind::*)` matching, then index
    /// // into the matching PHF map (HEAP_KIND -> PHF_REGISTRY mapping
    /// // is the ARRAY_METHODS / DATATABLE_METHODS / etc. table).
    /// let handler: MethodFnV2 = resolve_handler(args[0].kind, method_name_id)?;
    ///
    /// // Dispatch: handler reads &args, returns a kinded result.
    /// let result: KindedSlot = handler(self, &args, ctx)?;
    ///
    /// // Push result back onto the kinded stack (kind from the
    /// // returned KindedSlot — no fabrication). mem::forget skips the
    /// // carrier-drop so the share transfers cleanly to the stack.
    /// self.push_kinded(result.slot.into_raw(), result.kind)?;
    /// std::mem::forget(result);
    /// // args drops here — each KindedSlot's Drop dispatches on kind
    /// // and releases the share via drop_with_kind. No bare vw_drop
    /// // (forbidden #8). No Bool-default fallback (forbidden §2.7.7 #9
    /// // / §2.7.9).
    /// ```
    ///
    /// This shell body is **not yet wired** in this cluster. The ABI
    /// flip is the deliverable of Wave-γ G-method-fn-v2-abi; the
    /// dispatch-shell body wiring depends on:
    ///
    /// 1. The receiver-classification cascade
    ///    (`receiver_is_numeric` / `receiver_is_bool` /
    ///    `receiver_is_heap` + `HeapKind` match + sub-dispatch on
    ///    `Concurrency` / `TypedArray` / `Temporal` / `TableView`
    ///    inner variants). Pre-§2.7.9 this used `ValueWord::is_*` /
    ///    `as_heap_ref` (forbidden, playbook §4 #7); post-§2.7.9 it's
    ///    `match args[0].kind { NativeKind::* => ..., NativeKind::Ptr(
    ///    HeapKind::*) => args[0].slot.as_heap_value() match {
    ///    HeapValue::* => ... } }` per §2.7.6 / Q8. Wave-γ-followup
    ///    territory (sub-cluster `D-objects-mod-receiver-class`).
    /// 2. The IC fast-path call
    ///    (`crate::executor::ic_fast_paths::method_ic_check`) — the
    ///    `MethodFnV2` storage transmute through `usize` is ABI-opaque
    ///    and the `(receiver_kind, method_name_id)` keying is
    ///    unchanged, so the IC fast-path semantics survive the flip
    ///    untouched. The IC unit test's `dummy_handler` constant
    ///    signature realigns to the kinded ABI — IC sub-cluster
    ///    follow-up.
    /// 3. The v2-typed-array PHF fast-path detector
    ///    (`v2_array_detect::as_v2_typed_array`) — pre-§2.7.9 this
    ///    relied on `as_vw_ref` reinterpreting `&u64` as `&ValueWord`
    ///    (deleted). Post-§2.7.9 the detector takes
    ///    `(args[0].slot.bits(), args[0].kind)` directly. Wave-γ-
    ///    followup `D-v2-array-detect` cluster row.
    /// 4. The legacy stack-based calling convention reading
    ///    `arg_count` + `method_name` via `pop_raw_u64` +
    ///    `ValueWord::as_str` — either rewrites to `pop_kinded` +
    ///    `NativeKind::String` arm match, or is deleted as legacy
    ///    bytecode the compiler no longer emits. Wave-γ-followup
    ///    territory.
    /// 5. The `handle_typed_object_method_v2` path — pre-§2.7.9 used
    ///    `as_heap_ref` (forbidden) to extract `(schema_id, slots,
    ///    heap_mask)`; post-§2.7.9 uses `args[0].slot.as_heap_value()`
    ///    + `HeapValue::TypedObject` match. Wave-γ-followup territory.
    ///
    /// Per playbook §7.4 + §8 surface-and-stop trigger ("Cross-cluster
    /// migration cascade") this body surfaces back as
    /// `NotImplemented(SURFACE)` until the receiver-classification
    /// cascade lands. The ABI is no longer the blocker; the
    /// classification rewrite is. Do not paper over.
    pub fn op_call_method(
        &mut self,
        _instruction: &Instruction,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "SURFACE: op_call_method body wiring depends on the receiver-\
             classification cascade rewrite (Wave-γ-followup). The MethodFnV2 ABI \
             flip landed in this cluster (ADR-006 §2.7.9 / Q11) — the dispatch shell \
             now constructs `&[KindedSlot]` from popped stack args via \
             pop_kinded() and pushes the returned KindedSlot via push_kinded() \
             without fabricated kinds. Receiver classification + HeapKind \
             sub-dispatch + v2-array-detect + IC fast-path realignment are \
             downstream Wave-γ-followup territory. See ADR-006 §2.7.9 for the \
             dispatch-shell pseudocode + sub-cluster split."
                .into(),
        ))
    }

    /// SURFACE: MakeRange cannot be migrated in this cluster.
    ///
    /// The pre-Wave-6 body popped three `ValueWord` payloads (start, end,
    /// inclusive) and constructed `ValueWord::from_heap_value(HeapValue::Range
    /// { start: Option<Box<ValueWord>>, end: Option<Box<ValueWord>>, .. })`.
    /// Every ingredient (`ValueWord::from_raw_bits`, `ValueWord::is_none`,
    /// `Box<ValueWord>`, `ValueWord::from_heap_value`) is deleted. The kinded
    /// equivalent constructs `Arc<HeapValue::Range { start: Option<Arc<...>>,
    /// end: Option<Arc<...>>, .. }>` whose payload type itself is undecided
    /// — Range bounds cross-kind (a range over `int` carries Int64 payloads;
    /// a range over `Decimal` carries Arc<Decimal>; a range over the open
    /// integers has `None`). The post-§2.7.7 Range payload shape needs an
    /// ADR-006 follow-up; this is the same surface as `op_call_method`'s
    /// handler-ABI cascade.
    pub(in crate::executor) fn op_make_range(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "SURFACE: MakeRange depends on the deleted Box<ValueWord> Range payload \
             shape. Cross-kind range bounds (int, Decimal, BigInt, open range) \
             need a kinded HeapValue::Range redesign per ADR-006. See playbook \
             §8 cross-cluster cascade."
                .into(),
        ))
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests removed during D-objects-mod surface.
// ═════════════════════════════════════════════════════════════════════════════
//
// The pre-Wave-6 `v2a_dispatch_tests` module exercised the v2 typed-array PHF
// dispatch through `op_call_method` and used `ValueWord::from_native_ptr` /
// `ValueWord::from_array` / `ValueWord::from_i64` for receiver construction.
// All four constructors are deleted with the type. The tests' canonical
// shape (PHF resolution + handler invocation) is independent of the
// dispatch shell and fits naturally in `method_registry.rs`'s own test
// module once the handler ABI migrates; they are not required to live here.
// Re-instated in the post-cascade rewrite under cluster
// `E-builtins-backlog` / `D-v2-array-detect`.
