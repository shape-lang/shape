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

// Range method handlers (W15-range, ADR-006 §2.7.23 / Q24, 2026-05-10).
pub mod range_methods;

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

        // ADR-006 §2.7.24 Q25.C: when the receiver is a trait object,
        // route through the DynMethodCall dispatch shell instead of the
        // standard CallMethod path. This handles the case where the
        // compiler couldn't determine at compile-time that the receiver
        // is a `dyn T` (e.g. `let b = a.clone_me()` where `clone_me`
        // returns `Self` through a `BoxedReturn` thunk — the result is
        // a trait object but the compiler emits the standard CallMethod
        // opcode without a `dyn_locals` entry for `b`). Round-2: this
        // fallback ensures correctness; a future amendment can teach
        // type-inference to propagate `dyn T` through method-call
        // result types and emit `DynMethodCall` at the compile site.
        if self.sp >= arg_count + 1 {
            let receiver_idx_check = self.sp - arg_count - 1;
            let (_, receiver_kind_peek) = self.stack_read_kinded_raw(receiver_idx_check);
            if receiver_kind_peek
                == NativeKind::Ptr(shape_value::HeapKind::TraitObject)
            {
                // Reconstruct the instruction with `arg_count` /
                // `string_id` operands and call into the dyn dispatch
                // path. The TypedMethodCall operand layout matches
                // exactly what `op_dyn_method_call` expects.
                return self.exec_trait_object_ops(
                    &Instruction::new(
                        crate::bytecode::OpCode::DynMethodCall,
                        Some(Operand::TypedMethodCall {
                            method_id: _method_id,
                            arg_count: arg_count as u16,
                            string_id: string_id as u16,
                            receiver_type_tag: _receiver_type_tag,
                        }),
                    ),
                    ctx,
                );
            }
        }

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
        // always in-range for a well-formed program. We clone into an
        // owned `String` to release the immutable borrow on
        // `self.program.strings` before the `dispatch_method_kinded`
        // call below takes a mutable borrow on `self`.
        let method_name: String = self
            .program
            .strings
            .get(string_id)
            .cloned()
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "op_call_method: string_id {} out of bounds (pool size {})",
                    string_id,
                    self.program.strings.len()
                ))
            })?;

        // Classify the receiver, resolve the handler, and dispatch via
        // the shared `dispatch_method_kinded` entry — borrow-only ABI per
        // §2.7.10 / Q11. The handler borrows each KindedSlot; share
        // ownership stays with the carriers in `args`.
        let result = self.dispatch_method_kinded(&args, &method_name, ctx)?;

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

    /// Shared method-dispatch entry: resolve the handler via
    /// [`resolve_method_handler`](Self::resolve_method_handler) and call
    /// it with the kinded carrier slice.
    ///
    /// Two callers consume this entry:
    ///
    /// 1. `op_call_method` (above) — VM-side dispatch shell after popping
    ///    the receiver + args from the §2.7.7 stack parallel-kind track.
    /// 2. `jit_trampoline_call_method` (in
    ///    `crates/shape-vm/src/executor/call_convention.rs`) — the
    ///    §2.7.5 cross-crate stable-FFI consumer that converts the JIT's
    ///    pair-slice form into `&[KindedSlot]` carriers and delegates
    ///    here for the actual dispatch.
    ///
    /// `args[0]` is the receiver, `args[1..]` are the call args. Every
    /// entry's `kind` came from the §2.7.7 parallel-kind track at the
    /// producing site — no fabrication. The handler borrows each
    /// `KindedSlot` (§2.7.10 / Q11 borrow-only ABI); share ownership
    /// stays with the carriers at the caller. The returned `KindedSlot`
    /// owns its result share — the caller pushes it onto the stack or
    /// transfers it across the FFI boundary, then `mem::forget`s the
    /// returned carrier to balance refcounts.
    pub(crate) fn dispatch_method_kinded(
        &mut self,
        args: &[KindedSlot],
        method_name: &str,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        let handler = self.resolve_method_handler(args, method_name)?;
        handler(self, args, ctx)
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
            // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
            // ADR-006 §2.7.5 amendment adds F32 + Char as scalar variants.
            // F32 receivers route to NUMBER_METHODS (same numeric method
            // surface as F64). Char receivers route to CHAR_METHODS — the
            // existing receiver registry already covers char methods
            // (`.to_uppercase()`, `.is_alphabetic()`, etc.) and was wired
            // for the `NativeKind::Ptr(HeapKind::Char)` carrier; the same
            // method surface applies regardless of which Char carrier
            // label flows through (both labels store the same codepoint
            // bits and method bodies read via `as_char` which recognizes
            // both labels per the §2.7.5 amendment).
            NativeKind::Float32 => method_registry::NUMBER_METHODS.get(method_name).copied(),
            NativeKind::Char => method_registry::CHAR_METHODS.get(method_name).copied(),
            // UInt64 may be a v2 typed-array pointer (raw `*mut
            // TypedArray<T>`, no Arc) or a plain unsigned integer.
            // Classify via the stamped element-type byte.
            NativeKind::UInt64 => {
                let bits = receiver.slot.raw();
                if let Some(view) = as_v2_typed_array(bits, kind) {
                    let typed = match view.elem_type {
                        // All integer-family kinds (I64/I32 plus W12 S1 sized
                        // ints I8/U8/I16/U16/U32) share the typed-int method
                        // dispatch — methods sum/min/max/etc. operate on the
                        // integer-bit pattern regardless of width; narrower
                        // widths sign-/zero-extend at read time. U64 omitted
                        // — deferred to S1.5 per S1 reopen.
                        V2ElemType::I64
                        | V2ElemType::I32
                        | V2ElemType::I8
                        | V2ElemType::U8
                        | V2ElemType::I16
                        | V2ElemType::U16
                        | V2ElemType::U32 => {
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
                    //
                    // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13):
                    // Matrix and MatrixSlice exit this sub-classification.
                    // Matrix receivers arrive at `HeapKind::Matrix` →
                    // `MATRIX_METHODS` directly; MatrixSlice receivers at
                    // `HeapKind::MatrixSlice` → `FLOAT_ARRAY_METHODS`.
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
                            TypedArrayData::F64(_) | TypedArrayData::F32(_) => {
                                method_registry::FLOAT_ARRAY_METHODS
                                    .get(method_name)
                                    .copied()
                            }
                            TypedArrayData::Bool(_) => method_registry::BOOL_ARRAY_METHODS
                                .get(method_name)
                                .copied(),
                            TypedArrayData::String(_) => None,
                            // W17-typed-carrier-bundle-A checkpoint 3/4:
                            // Q25.A specialized arms — no per-element-type
                            // PHF table (Decimal/BigInt arrays use the
                            // generic ARRAY_METHODS fallback below). The
                            // None return falls through to the .or_else
                            // ARRAY_METHODS lookup.
                            TypedArrayData::Decimal(_)
                            | TypedArrayData::BigInt(_)
                            | TypedArrayData::Char(_)
                            | TypedArrayData::TypedObject(_) => None,
                        },
                        _ => None,
                    };
                    typed.or_else(|| method_registry::ARRAY_METHODS.get(method_name).copied())
                }
                // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13):
                // Matrix is a first-class HeapKind — receivers route
                // directly to `MATRIX_METHODS` (no inner-TypedArrayData
                // sub-classification two-step). MatrixSlice receivers
                // route to `FLOAT_ARRAY_METHODS` (their methods are
                // numeric-aggregations over a flat f64 region; the same
                // PHF that handles `F64`-typed arrays applies).
                HeapKind::Matrix => method_registry::MATRIX_METHODS.get(method_name).copied(),
                HeapKind::MatrixSlice => method_registry::FLOAT_ARRAY_METHODS
                    .get(method_name)
                    .copied(),
                HeapKind::Temporal => {
                    // C1-temporal-lowering (Phase 2d Wave 2): Temporal
                    // slots are `Arc::into_raw::<TemporalData>` — NOT a
                    // `Box<HeapValue>` allocation. `as_heap_value()` would
                    // be wrong-type recovery (5-arm receiver-recovery
                    // soundness rule, CLAUDE.md / handover §0). Sub-
                    // classify by directly borrowing `&TemporalData` from
                    // the slot's Arc-raw pointer, mirroring
                    // `objects/datetime_methods.rs::recv_temporal`.
                    //
                    // SAFETY: when receiver.kind == Ptr(HeapKind::Temporal),
                    // receiver.slot.raw() is `Arc::into_raw::<TemporalData>`
                    // (set by `op_push_const::Constant::Duration` /
                    // `Constant::DateTimeExpr` arms, by
                    // `temporal_result()` in datetime_methods.rs, and by
                    // the §2.7.7 stack parallel-kind track). The carrier
                    // owns one strong-count share for the dispatch
                    // duration; the &TemporalData borrow's lifetime is
                    // bounded by `args[0]`'s share ownership.
                    let bits = receiver.slot.raw();
                    if bits == 0 {
                        None
                    } else {
                        let td: &TemporalData =
                            unsafe { &*(bits as *const TemporalData) };
                        match td {
                            TemporalData::DateTime(_) => {
                                method_registry::DATETIME_METHODS
                                    .get(method_name)
                                    .copied()
                            }
                            TemporalData::TimeSpan(_) | TemporalData::Duration(_) => {
                                method_registry::TIMESPAN_METHODS
                                    .get(method_name)
                                    .copied()
                            }
                            // Timeframe / TimeReference / DateTimeExpr /
                            // DataDateTimeRef have no method PHF — they
                            // are language-level metadata, not method-
                            // call targets. Fall through to
                            // UnknownMethod.
                            _ => None,
                        }
                    }
                }
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
                // Wave 15 W15-deque / W15-channel / W15-priority-queue
                // closes (ADR-006 §2.7.19/Q20, §2.7.20/Q21, §2.7.18/Q19)
                // — the new HeapKind ordinals 23/24/25 with their
                // `*_METHODS` registries.
                HeapKind::Deque => method_registry::DEQUE_METHODS.get(method_name).copied(),
                HeapKind::Channel => method_registry::CHANNEL_METHODS.get(method_name).copied(),
                HeapKind::PriorityQueue => method_registry::PRIORITY_QUEUE_METHODS
                    .get(method_name)
                    .copied(),
                // W17-concurrency (ADR-006 §2.7.25, 2026-05-11): the
                // new HeapKind ordinals 30/31/32 with their
                // MUTEX_METHODS / ATOMIC_METHODS / LAZY_METHODS
                // registries. Method-receiver classification routes
                // `m.lock()` / `a.fetch_add(...)` / `l.get()` here.
                HeapKind::Mutex => method_registry::MUTEX_METHODS.get(method_name).copied(),
                HeapKind::Atomic => method_registry::ATOMIC_METHODS.get(method_name).copied(),
                HeapKind::Lazy => method_registry::LAZY_METHODS.get(method_name).copied(),
                // W15-range close (ADR-006 §2.7.23/Q24): Range receivers
                // route to the RANGE_METHODS PHF.
                HeapKind::Range => method_registry::RANGE_METHODS.get(method_name).copied(),
                // W14-variant-codegen close (ADR-006 §2.7.17/Q18):
                // Result/Option are typed-Arc carriers; method-call
                // dispatch goes through op_is_ok / op_unwrap_ok / etc.
                // typed opcodes, not through the generic method PHF.
                // No method-PHF arm; falls through to UFCS / unknown.
                HeapKind::Result | HeapKind::Option => None,
                // ADR-006 §2.7.10 explicitly excludes the closure /
                // future / reference / shared-cell / filter-expr
                // discriminators from method-call dispatch — these are
                // not user-callable receivers. Trait-object method
                // calls go through `op_dyn_method_call`, not here —
                // the compiler-emission tier (W17-trait-object-emission)
                // emits `DynMethodCall` opcodes that walk the receiver's
                // `Arc<TraitObjectStorage>::vtable` directly per
                // ADR-006 §2.7.24 / Q25.C.5 `VTableEntry` shape, NOT
                // through this generic method PHF.
                HeapKind::Closure
                | HeapKind::Future
                | HeapKind::Reference
                | HeapKind::SharedCell
                | HeapKind::FilterExpr
                | HeapKind::TraitObject
                | HeapKind::IoHandle
                | HeapKind::TaskGroup
                | HeapKind::NativeView
                | HeapKind::NativeScalar
                // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
                // ModuleFn references are not user-callable receivers
                // via method-call dispatch — they route through
                // op_call_value's `Ptr(HeapKind::ModuleFn)` arm directly
                // (`invoke_module_fn_id_stub`), not through this generic
                // PHF lookup.
                | HeapKind::ModuleFn => None,
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

    /// `MakeRange` opcode body — pop (start, end, inclusive) from the §2.7.7
    /// kinded stack and push a fresh `Arc<RangeData>` slot with kind
    /// `NativeKind::Ptr(HeapKind::Range)` (W15-range, ADR-006 §2.7.23 / Q24).
    ///
    /// Stack layout at entry (from `compiler/expressions/misc.rs:369`):
    ///
    /// ```ignore
    /// [.., start_value, end_value, PushConst<Bool>(inclusive), MakeRange]
    /// ```
    ///
    /// Popping order is reverse-push: `inclusive` first, then `end`, then
    /// `start`. Per the surface syntax, `start_value` and `end_value` are
    /// `int`-typed expressions (`0..10`); the `PushNull` placeholder for
    /// open ranges (`..n` / `n..`) reaches this handler with kind
    /// `NativeKind::Bool` and bits zero (the `PushNull` shape) — open
    /// ranges are surfaced as a SURFACE error pending the iterator-tier
    /// semantic (`for i in 0..` infinite loops are their own ADR
    /// follow-up; matches the pre-strict-typing surface).
    ///
    /// Other-kind bounds (Decimal, BigInt, NativeScalar) similarly
    /// surface — the post-strict-typing `RangeData { start: i64, end: i64,
    /// .. }` shape only models i64 ranges at landing. Cross-kind range
    /// bounds are tracked as a follow-up §2.7.23 amendment (mirror of the
    /// W14 Result/Option payload-cardinality discussion).
    pub(in crate::executor) fn op_make_range(&mut self) -> Result<(), VMError> {
        use shape_value::{KindedSlot, NativeKind, ValueSlot, heap_value::RangeData};

        // Pop in reverse-push order: inclusive flag first, then end, then start.
        // We immediately wrap each pop result in a `KindedSlot` carrier so its
        // `Drop` impl handles refcount release on every error path automatically
        // — no manual `drop_with_kind` bookkeeping needed.
        let incl_kinded = {
            let (bits, kind) = self.pop_kinded()?;
            KindedSlot::new(ValueSlot::from_raw(bits), kind)
        };
        let end_kinded = {
            let (bits, kind) = self.pop_kinded()?;
            KindedSlot::new(ValueSlot::from_raw(bits), kind)
        };
        let start_kinded = {
            let (bits, kind) = self.pop_kinded()?;
            KindedSlot::new(ValueSlot::from_raw(bits), kind)
        };

        // The `inclusive` operand is a `PushConst<Bool>` per
        // `compiler/expressions/misc.rs:362-368`. Kind must be Bool —
        // any other kind is a kind-source bug at the emit site.
        let inclusive = match incl_kinded.kind() {
            NativeKind::Bool => incl_kinded.slot().as_bool(),
            _ => {
                return Err(VMError::RuntimeError(
                    "MakeRange: inclusive flag operand must be Bool (kind-source bug \
                     at compile site — `compiler/expressions/misc.rs` emits a \
                     `PushConst<Bool>` for the inclusive flag)".into(),
                ));
            }
        };

        // Bounds: only i64 supported at landing (ADR-006 §2.7.23). Other
        // kinds — Float64 (`0.0..1.0` would-be syntax), Decimal, BigInt,
        // NativeScalar — surface for the cross-kind Range payload
        // follow-up. Bool with zero bits IS the `PushNull` open-range
        // placeholder (`..n` / `n..` / `..`) emitted by the compiler;
        // surface that distinctly so the diagnostic is precise.
        let to_i64 = |k: &KindedSlot, side: &str| -> Result<i64, VMError> {
            match k.kind() {
                NativeKind::Int64 => Ok(k.slot().as_i64()),
                NativeKind::Bool if k.slot().raw() == 0 => Err(VMError::NotImplemented(format!(
                    "MakeRange: open-range bound on {side} side (PushNull placeholder) — \
                     SURFACE: open ranges (`..n` / `n..` / `..`) need the iterator-tier \
                     infinite-iter semantic per ADR-006 §2.7.23 follow-up. Closed ranges \
                     (`start..end` / `start..=end`) work today.",
                ))),
                other => Err(VMError::NotImplemented(format!(
                    "MakeRange: cross-kind bound on {side} side (got {other:?}) — \
                     SURFACE: post-strict-typing RangeData only models i64 ranges at \
                     landing. Cross-kind bounds (Decimal, BigInt, Float64, NativeScalar) \
                     tracked as ADR-006 §2.7.23 follow-up.",
                ))),
            }
        };

        let start = to_i64(&start_kinded, "start")?;
        let end = to_i64(&end_kinded, "end")?;

        let range = std::sync::Arc::new(RangeData::new(start, end, 1, inclusive));
        self.push_kinded_slot(KindedSlot::from_range(range))?;
        Ok(())
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
