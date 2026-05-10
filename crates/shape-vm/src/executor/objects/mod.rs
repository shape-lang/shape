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
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
};
use shape_value::{HeapKind, HeapValue, KindedSlot, NativeKind, TemporalData, TypedArrayData, ValueSlot, VMError};

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

    /// CallMethod dispatch shell (W16-op-call-method close).
    ///
    /// ADR-006 §2.7.10 / Q11 dispatch shell — pops the receiver +
    /// arg-count call args from the §2.7.7 kinded stack, classifies
    /// the receiver kind to pick the matching PHF method registry,
    /// dispatches through `MethodFnV2`, and pushes the kinded result.
    ///
    /// Body shape per the W7-op-call-value precedent (close commit
    /// `27812cf`, `executor/control_flow/mod.rs:dispatch_call_value_immediate`):
    ///
    /// 1. Pop `arg_count + 1` slots via `pop_kinded()` (receiver
    ///    included). Each pop transfers one share (heap-bearing kinds)
    ///    into the returned `(bits, kind)` pair (WB2.4 retain-on-read,
    ///    §2.7.7); the `KindedSlot::new` carrier takes ownership of
    ///    that share. Pop order is reverse of push order, so reverse
    ///    the vec back to position-aligned order with `args[0]` =
    ///    receiver.
    /// 2. Decode `arg_count` + method name from
    ///    `Operand::TypedMethodCall { arg_count, string_id, .. }`
    ///    (`bytecode/opcode_defs.rs:2023`). The method name string is
    ///    indexed via `string_id` into `self.program.strings`.
    /// 3. Classify `args[0].kind` to pick a PHF registry per the
    ///    §2.7.6 / Q8 heterogeneous-kind body pattern. Numeric / Bool
    ///    / String scalars route to the matching scalar registry;
    ///    `Ptr(HeapKind::*)` heap kinds route to the per-heap-kind
    ///    registry, with `HeapKind::TypedArray` sub-classified on the
    ///    inner `TypedArrayData::{I64, F64, Bool, ...}` variant via
    ///    `slot.as_heap_value()` and `HeapKind::Temporal`
    ///    sub-classified on the inner `TemporalData::{DateTime,
    ///    TimeSpan, ...}` variant. The v2 typed-array fast path
    ///    (`UInt64`-tagged raw `*mut TypedArray<T>` pointer) routes
    ///    through `as_v2_typed_array` to `TYPED_INT_ARRAY_METHODS` /
    ///    `TYPED_NUMBER_ARRAY_METHODS` per playbook §10
    ///    `D-v2-array-detect`.
    /// 4. PHF lookup keyed on `&str` method name returns the
    ///    `MethodFnV2` handler. A miss surfaces a `RuntimeError`
    ///    citing the receiver kind + method name; user-defined
    ///    methods on `HeapValue::TypedObject` fall through to a UFCS
    ///    function-name lookup (`function_name_index`) before the
    ///    final `Unknown method` error. Closure / Future / Reference
    ///    / SharedCell / FilterExpr receivers reject — they are not
    ///    method-call targets.
    /// 5. Dispatch: `handler(self, &args, ctx)` returns
    ///    `Result<KindedSlot, VMError>`. The `&[KindedSlot]` borrow
    ///    leaves the shares with the carriers in this stack frame —
    ///    handlers borrow each entry per §2.7.10 / Q11 borrow-only
    ///    ABI.
    /// 6. Push the result via `push_kinded(result.raw(), result.kind())`
    ///    and `std::mem::forget(result)` so the result share transfers
    ///    cleanly to the stack (no double-drop). The `args` carriers
    ///    drop at end of scope; `KindedSlot::Drop` dispatches on kind
    ///    and releases each share via `drop_with_kind` (no bare
    ///    `vw_drop`, no Bool-default fallback).
    ///
    /// Forbidden surfaces (per CLAUDE.md "Renames to refuse on sight"
    /// + ADR-006 §2.7.10 / Q11): `Vec<KindedSlot>` by-move into a
    /// dispatch helper; `args: &mut [KindedSlot]`; tag-bits decode on
    /// receiver bits; `is_heap()` probe on raw bits; Bool-default
    /// fallback for unknown kind; defection-attractor framing on
    /// the method-dispatch ABI (`MethodFn` / `MethodFnLegacy` /
    /// `dispatch_method_handler_raw` / `call_handler_with_u64_slice`).
    ///
    /// Surfaces remaining (out of W16 territory):
    /// - **IC fast-path recording / hit**: `method_ic_check` /
    ///   `method_ic_record` already accept the kinded `MethodFnV2`
    ///   transmute (`ic_fast_paths.rs:42-44`) — wiring the IC
    ///   recording at the dispatch shell is a downstream JIT-IC
    ///   follow-up, not a correctness gate. The dispatch shell stays
    ///   correct without IC; the IC adds speed only.
    /// - **`HeapKind::Closure` receivers** (e.g. closure-as-trait-
    ///   object dispatch). Trait-object dispatch goes through
    ///   `op_dyn_method_call`, not `op_call_method`; the closure arm
    ///   here rejects with a clear error.
    pub fn op_call_method(
        &mut self,
        instruction: &Instruction,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        // ADR-006 §2.7.10 / Q11: arg_count + method name from operand
        // (typed dispatch is the only emit shape per
        // `compiler/expressions/function_calls.rs:2014` / `binary_ops.rs`
        // / `unary_ops.rs`). Legacy stack-arg-count dispatch is gone.
        let (arg_count, string_id, _method_id, _receiver_type_tag) = match instruction.operand {
            Some(Operand::TypedMethodCall {
                method_id,
                arg_count,
                string_id,
                receiver_type_tag,
            }) => (
                arg_count as usize,
                string_id as usize,
                method_id,
                receiver_type_tag,
            ),
            _ => return Err(VMError::InvalidOperand),
        };

        // Pop receiver + arg_count call args. Each pop_kinded transfers
        // one share into the returned (bits, kind); the KindedSlot
        // carrier takes ownership and releases via drop_with_kind on
        // scope exit. ADR-006 §2.7.7 WB2.4 retain-on-read.
        let total = arg_count + 1;
        let mut args: Vec<KindedSlot> = Vec::with_capacity(total);
        for _ in 0..total {
            let (bits, kind) = self.pop_kinded()?;
            args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
        }
        // Pop is reverse of push order; flip so args[0] is the receiver.
        args.reverse();

        // Resolve method name. The string pool index was offset-fixed
        // at link time (`executor/mod.rs:883`), so direct indexing is
        // always in-range for a well-formed program.
        let method_name: &str = self
            .program
            .strings
            .get(string_id)
            .map(|s| s.as_str())
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "op_call_method: string_id {} out of bounds (pool size {})",
                    string_id,
                    self.program.strings.len()
                ))
            })?;

        // Classify the receiver and resolve the handler. UFCS / unknown
        // is signalled by `Ok(None)` from the resolver so the helper can
        // fall back to a user-defined function lookup before raising
        // `RuntimeError`.
        let handler = self.resolve_method_handler(&args, method_name)?;

        // Dispatch — borrow-only ABI per §2.7.10 / Q11. The handler
        // borrows each KindedSlot; share ownership stays with the
        // carriers in `args`.
        let result = handler(self, &args, ctx)?;

        // Transfer the result share onto the kinded stack. The result
        // carrier is forgotten so its Drop does not double-release.
        self.push_kinded(result.raw(), result.kind())?;
        std::mem::forget(result);

        // `args` carriers drop here. `KindedSlot::Drop` dispatches on
        // each entry's kind and retires its share via the matching
        // `Arc::decrement_strong_count::<T>` arm — no bare vw_drop
        // (forbidden), no Bool-default fallback (forbidden §2.7.7 #9).
        Ok(())
    }

    /// Resolve a method handler from `(receiver_kind, method_name)`.
    ///
    /// Receiver classification per ADR-006 §2.7.6 / Q8 heterogeneous-
    /// kind body pattern: scalar kinds map directly to scalar PHF
    /// registries; `Ptr(HeapKind::*)` heap kinds map to the matching
    /// per-heap-kind registry, with `TypedArray` and `Temporal`
    /// sub-classified through `slot.as_heap_value()` matching to pick
    /// the element-typed sub-registry. The `UInt64`-tagged v2 typed-
    /// array fast path (`*mut TypedArray<T>` pointer with stamped
    /// element-type byte) routes through `v2_array_detect`.
    ///
    /// Returns `Err(RuntimeError)` for unknown method on a known
    /// receiver kind, or unsupported receiver kind. Falls through to
    /// `function_name_index` UFCS for `HeapKind::TypedObject`
    /// receivers when the method is not in `DATATABLE_METHODS` (the
    /// dispatch table covering generic table-shaped methods is the
    /// closest fit; user-defined methods land via UFCS).
    fn resolve_method_handler(
        &self,
        args: &[KindedSlot],
        method_name: &str,
    ) -> Result<method_registry::MethodHandler, VMError> {
        use crate::executor::v2_handlers::v2_array_detect::{V2ElemType, as_v2_typed_array};

        let receiver = &args[0];
        let kind = receiver.kind;

        // Pure-scalar receivers — kind alone selects the registry.
        let scalar_handler: Option<method_registry::MethodHandler> = match kind {
            NativeKind::Float64
            | NativeKind::NullableFloat64
            | NativeKind::Int8
            | NativeKind::NullableInt8
            | NativeKind::UInt8
            | NativeKind::NullableUInt8
            | NativeKind::Int16
            | NativeKind::NullableInt16
            | NativeKind::UInt16
            | NativeKind::NullableUInt16
            | NativeKind::Int32
            | NativeKind::NullableInt32
            | NativeKind::UInt32
            | NativeKind::NullableUInt32
            | NativeKind::Int64
            | NativeKind::NullableInt64
            | NativeKind::NullableUInt64
            | NativeKind::IntSize
            | NativeKind::NullableIntSize
            | NativeKind::UIntSize
            | NativeKind::NullableUIntSize => method_registry::NUMBER_METHODS.get(method_name).copied(),
            NativeKind::Bool => method_registry::BOOL_METHODS.get(method_name).copied(),
            NativeKind::String => method_registry::STRING_METHODS.get(method_name).copied(),
            // UInt64 may be a v2 typed-array pointer (raw `*mut
            // TypedArray<T>`, no Arc) or a plain unsigned integer.
            // Classify via the stamped element-type byte.
            NativeKind::UInt64 => {
                let bits = receiver.slot.raw();
                if let Some(view) = as_v2_typed_array(bits, kind) {
                    let typed = match view.elem_type {
                        V2ElemType::I64 | V2ElemType::I32 => {
                            method_registry::TYPED_INT_ARRAY_METHODS
                                .get(method_name)
                                .copied()
                        }
                        V2ElemType::F64 => method_registry::TYPED_NUMBER_ARRAY_METHODS
                            .get(method_name)
                            .copied(),
                        V2ElemType::Bool => method_registry::BOOL_ARRAY_METHODS
                            .get(method_name)
                            .copied(),
                    };
                    typed.or_else(|| method_registry::ARRAY_METHODS.get(method_name).copied())
                } else {
                    method_registry::NUMBER_METHODS.get(method_name).copied()
                }
            }
            NativeKind::Ptr(_) => None,
        };
        if let Some(h) = scalar_handler {
            return Ok(h);
        }

        // Heap receivers — dispatch on HeapKind, then sub-classify
        // TypedArray / Temporal via `slot.as_heap_value()`.
        if let NativeKind::Ptr(hk) = kind {
            let heap_handler: Option<method_registry::MethodHandler> = match hk {
                HeapKind::String => method_registry::STRING_METHODS.get(method_name).copied(),
                HeapKind::Char => method_registry::CHAR_METHODS.get(method_name).copied(),
                HeapKind::HashMap => method_registry::HASHMAP_METHODS.get(method_name).copied(),
                HeapKind::HashSet => method_registry::SET_METHODS.get(method_name).copied(),
                HeapKind::DataTable => method_registry::DATATABLE_METHODS
                    .get(method_name)
                    .copied(),
                HeapKind::Iterator => method_registry::ITERATOR_METHODS.get(method_name).copied(),
                HeapKind::Instant => method_registry::INSTANT_METHODS.get(method_name).copied(),
                HeapKind::Content => method_registry::CONTENT_METHODS.get(method_name).copied(),
                HeapKind::Decimal => method_registry::NUMBER_METHODS.get(method_name).copied(),
                HeapKind::BigInt => method_registry::NUMBER_METHODS.get(method_name).copied(),
                HeapKind::TypedArray => {
                    // Sub-classify on the inner TypedArrayData variant
                    // (per playbook §10 D-objects-mod receiver-class).
                    // `as_heap_value()` is sound here — TypedArray is a
                    // full `HeapValue` arm (ADR-005 §1).
                    let typed = match receiver.slot.as_heap_value() {
                        HeapValue::TypedArray(arc) => match arc.as_ref() {
                            TypedArrayData::I64(_)
                            | TypedArrayData::I8(_)
                            | TypedArrayData::I16(_)
                            | TypedArrayData::I32(_)
                            | TypedArrayData::U8(_)
                            | TypedArrayData::U16(_)
                            | TypedArrayData::U32(_)
                            | TypedArrayData::U64(_) => method_registry::INT_ARRAY_METHODS
                                .get(method_name)
                                .copied(),
                            TypedArrayData::F64(_)
                            | TypedArrayData::F32(_)
                            | TypedArrayData::FloatSlice { .. } => {
                                method_registry::FLOAT_ARRAY_METHODS
                                    .get(method_name)
                                    .copied()
                            }
                            TypedArrayData::Bool(_) => method_registry::BOOL_ARRAY_METHODS
                                .get(method_name)
                                .copied(),
                            TypedArrayData::Matrix(_) => {
                                method_registry::MATRIX_METHODS.get(method_name).copied()
                            }
                            TypedArrayData::String(_) | TypedArrayData::HeapValue(_) => None,
                        },
                        _ => None,
                    };
                    typed.or_else(|| method_registry::ARRAY_METHODS.get(method_name).copied())
                }
                HeapKind::Temporal => match receiver.slot.as_heap_value() {
                    HeapValue::Temporal(arc) => match arc.as_ref() {
                        TemporalData::DateTime(_) => {
                            method_registry::DATETIME_METHODS.get(method_name).copied()
                        }
                        TemporalData::TimeSpan(_) | TemporalData::Duration(_) => {
                            method_registry::TIMESPAN_METHODS.get(method_name).copied()
                        }
                        // Timeframe / TimeReference / DateTimeExpr /
                        // DataDateTimeRef have no method PHF — they are
                        // language-level metadata, not method-call
                        // targets. Fall through to UnknownMethod.
                        _ => None,
                    },
                    _ => None,
                },
                HeapKind::TypedObject => {
                    // User-defined object methods land here. The
                    // built-in DataTable PHF covers shared table-shape
                    // methods; UFCS resolution below catches user-
                    // defined `fn TypeName.method(self, ...)` shapes.
                    method_registry::DATATABLE_METHODS
                        .get(method_name)
                        .copied()
                }
                HeapKind::TableView => method_registry::DATATABLE_METHODS
                    .get(method_name)
                    .copied(),
                // `HeapKind::Channel` does not exist yet — W15-channel
                // (playbook §2 row) lands the `HeapKind::Channel = 27`
                // ordinal + `HeapValue::Channel(Arc<ChannelData>)` arm
                // alongside the `CHANNEL_METHODS` registry. Until that
                // wave merges, channel receivers cannot be pushed onto
                // the kinded stack — there is no `NativeKind::Ptr(
                // HeapKind::Channel)` to classify here.
                //
                // ADR-006 §2.7.10 explicitly excludes the closure /
                // future / reference / shared-cell / filter-expr
                // discriminators from method-call dispatch — these are
                // not user-callable receivers. Trait-object method
                // calls go through `op_dyn_method_call`, not here.
                HeapKind::Closure
                | HeapKind::Future
                | HeapKind::Reference
                | HeapKind::SharedCell
                | HeapKind::FilterExpr
                | HeapKind::IoHandle
                | HeapKind::TaskGroup
                | HeapKind::NativeView
                | HeapKind::NativeScalar => None,
            };
            if let Some(h) = heap_handler {
                return Ok(h);
            }
        }

        // UFCS / unknown — surface the receiver kind in the error so
        // call sites can diagnose. Per playbook §3 "surface-and-stop
        // if PHF lookup API doesn't quite match", an unknown method
        // is *not* a SURFACE — it's a real runtime error the program
        // can hit, so we return `RuntimeError`, not `NotImplemented`.
        Err(VMError::RuntimeError(format!(
            "no method '{}' on receiver kind {:?}",
            method_name, kind
        )))
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
