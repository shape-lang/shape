//! Loop control operations for the VM executor
//!
//! Handles: LoopStart, LoopEnd, Break, Continue, IterNext, IterDone
//!
//! ADR-006 §2.7.6 / §2.7.7 / §2.7.16 / Wave-β `B10-loops-heap` (playbook
//! §10) + W17-iterator-reference-rebuild (2026-05-12):
//!
//! Heap-side dispatch in `op_iter_next` / `op_iter_done` goes through
//! **per-kind typed-Arc reconstruction** — `Arc::from_raw::<T>(bits)` +
//! borrowed read + `Arc::into_raw(arc)` to restore the slot's share.
//! This is the canonical 5-arm receiver-recovery soundness pattern
//! documented in handover §0 and exemplified by
//! `iterator_methods::clone_typed_array_arc` /
//! `range_methods::clone_range_arc`.
//!
//! The previous implementation cast `*const T` bits to `*const HeapValue`
//! via `ValueSlot::as_heap_value()`. That worked coincidentally when the
//! T enum's discriminator at offset 0 happened to alias a `HeapValue::*`
//! variant of the right arm — but for `Arc<TypedArrayData::TypedObject>`
//! the discriminator (ordinal 20 inside `TypedArrayData`) aliases
//! `HeapValue::Reference` (ordinal 20 inside `HeapValue`), surfacing
//! "iter_kind=Ptr(TypedArray) but heap arm is Reference" at every
//! `for entry in map.entries()` over the post-bundle-A
//! `TypedArrayData::TypedObject(Arc<TypedBuffer<Arc<TypedObjectStorage>>>)`
//! carrier. The correct dispatch reads the inner `T` directly per
//! ADR-006 §2.7.13 / §2.7.16 typed-Arc dispatch label discipline — see
//! also `set_methods::v2_size` (commit `3ac2f11`) for the same shape on
//! a HashSet receiver.
//!
//! Element kind for the pushed value is sourced from the matched
//! `TypedArrayData::*` variant per playbook §2 ("loop iteration value
//! kind from iterator's element FieldType — capture per element").
//!
//! `HeapValue` no longer carries the deleted `Array` / `Range` /
//! `Iterator` variants (post-§2.7.6 heap layout). Iteration over those
//! shapes surfaces as `VMError::NotImplemented` per playbook §7 #4 +
//! §2.7.4 — never papered over with a Bool-default fallback (the
//! W-series rationalization §2.7.7 #9 names verbatim).
//!
//! `RowView` / `TypedTable` row materialization at `IterNext` requires
//! the deleted `ValueWord::from_row_view` packed-tag carrier; the
//! kinded redesign of the row-view payload is Phase 2c work
//! (ADR-006 §2.7.4). Same disposition as the existing
//! `executor/tests/table_iteration.rs` `todo!("phase-2c …")` markers.

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::{LoopContext, VirtualMachine},
};
use crate::executor::vm_impl::stack::drop_with_kind;
use shape_value::datatable::DataTable;
// V3-S5 ckpt-4 (2026-05-15): `TypedArrayData` import deleted — the enum
// was retired at ckpt-1 per W12-typed-array-data-deletion-audit §3.5 +
// ADR-006 §2.7.24 Q25.A SUPERSEDED. Per-arm dispatch in op_iter_done /
// op_iter_next replaced with structured surface-and-stop via the shared
// `ckpt4_surface` builder below (ckpt-3 precedent: `ckpt3_surface` at
// array_ops.rs / typed_array_methods.rs / iterator_methods.rs et al.).
use shape_value::heap_value::{HashMapKindedRef, HeapKind, TableViewData};
use shape_value::{NativeKind, VMError};
use std::sync::Arc;

/// Shared surface-and-stop builder for the V3-S5 ckpt-4 wholesale rewrite.
#[cold]
fn ckpt4_surface(op: &str, detail: &str) -> VMError {
    VMError::NotImplemented(format!(
        "{op} SURFACE (V3-S5 ckpt-4 wholesale rewrite, 2026-05-15): \
         {detail}. The `TypedArrayData` enum + `TypedBuffer<T>` / \
         `AlignedTypedBuffer` wrapper layer were retired wholesale at \
         V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-deletion-audit \
         §3.5 + §B + ADR-006 §2.7.24 Q25.A SUPERSEDED; the per-arm \
         dispatch graph has no discriminator until the v2-raw \
         `TypedArray<T>` per-element-kind rebuild lands in a downstream \
         wave. Refusal #1 binding."
    ))
}

/// Decode the loop iterator-protocol index from its kinded slot.
///
/// The compiler emits `IterNext` / `IterDone` with the idx top-of-stack
/// as one of:
///   - `NativeKind::Int64` (and the rest of the `is_integer_family()`
///     kinds) — typed loop counters from `LoadLocalI64`, `AddInt`, etc.
///   - `NativeKind::Float64` — test harnesses still using
///     `Constant::Number(N.0)` for the idx (legacy entry-point shape).
///
/// All other kinds are a wrong-shape opcode emit and surface as a
/// `TypeError`.
#[inline(always)]
fn decode_iter_idx(bits: u64, kind: NativeKind) -> Result<i64, VMError> {
    if kind.is_integer_family() {
        // Sign-extend per the integer family — `as i64` on the raw bits
        // is the canonical reinterpretation for `NativeKind::Int*`.
        return Ok(bits as i64);
    }
    match kind {
        NativeKind::Float64 | NativeKind::NullableFloat64 => {
            Ok(f64::from_bits(bits) as i64)
        }
        NativeKind::Bool => Ok(bits as i64),
        _ => Err(VMError::TypeError {
            expected: "number",
            got: "non-numeric idx",
        }),
    }
}

// V3-S5 ckpt-4 (2026-05-15): `typed_array_data_len` 22-arm helper DELETED
// in lockstep with the `TypedArrayData` enum + `TypedBuffer<T>` wrapper
// layer. W12-typed-array-data-deletion-audit §3.5/§B + ADR-006 §2.7.24
// Q25.A SUPERSEDED. The op_iter_done `Ptr(HeapKind::TypedArray)` arm
// surface-and-stops via `ckpt4_surface` (helper defined above near the
// `use` block) pending the downstream-wave v2-raw `TypedArray<T>`
// per-element-kind rebuild.

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_loops(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        use OpCode::*;
        match instruction.opcode {
            LoopStart => self.op_loop_start(instruction)?,
            LoopEnd => self.op_loop_end()?,
            Break => self.op_break()?,
            Continue => self.op_continue()?,
            IterNext => self.op_iter_next()?,
            IterDone => self.op_iter_done()?,
            _ => unreachable!(
                "exec_loops called with non-loop opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    // ===== Loop Control Operations =====

    pub(in crate::executor) fn op_loop_start(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        if let Some(Operand::Offset(end_offset)) = instruction.operand {
            let end = (self.ip as i32 + end_offset) as usize;
            self.loop_stack.push(LoopContext {
                start: self.ip,
                end,
            });

            // OSR: record back-edge and attempt entry into JIT-compiled loop.
            // The LoopStart IP (self.ip - 1 since we already advanced past it)
            // is the canonical loop header IP used as the key.
            #[cfg(feature = "jit")]
            {
                let loop_ip = self.ip.saturating_sub(1);
                if let Some(func_id) = self.current_function_id() {
                    // Record the back-edge iteration; request compilation if hot
                    self.check_osr_back_edge(func_id, loop_ip);

                    // Attempt OSR entry if compiled code is available
                    if self.try_osr_entry(func_id, loop_ip)? {
                        // OSR succeeded: IP is set to loop exit, pop the loop
                        // context we just pushed (the JIT executed the loop).
                        self.loop_stack.pop();
                        return Ok(());
                    }
                }
            }
        }
        // Without operand, LoopStart is a JIT-only marker (no-op for interpreter).
        // Break/continue are compiled as Jump instructions, not Break/Continue opcodes.
        Ok(())
    }

    pub(in crate::executor) fn op_loop_end(&mut self) -> Result<(), VMError> {
        self.loop_stack.pop();
        Ok(())
    }

    pub(in crate::executor) fn op_break(&mut self) -> Result<(), VMError> {
        if let Some(loop_ctx) = self.loop_stack.last() {
            self.ip = loop_ctx.end;
            Ok(())
        } else {
            Err(VMError::RuntimeError("break outside of loop".to_string()))
        }
    }

    pub(in crate::executor) fn op_continue(&mut self) -> Result<(), VMError> {
        if let Some(loop_ctx) = self.loop_stack.last() {
            self.ip = loop_ctx.start;
            Ok(())
        } else {
            Err(VMError::RuntimeError(
                "continue outside of loop".to_string(),
            ))
        }
    }

    /// Iterator-protocol "done" check.
    ///
    /// Stack on entry: `[..., iter, idx]`.
    /// Stack on exit: `[..., done: bool]` with `NativeKind::Bool`.
    ///
    /// Both `iter` and `idx` shares are popped via the kinded API and
    /// retired with `drop_with_kind` once the bool is decided. Heap-side
    /// dispatch on `iter_kind` reconstructs the inner typed `Arc<T>` from
    /// raw bits per ADR-006 §2.7.13 / §2.7.16 typed-Arc dispatch label
    /// discipline — never `slot.as_heap_value()` (wrong-type cast under
    /// the 5-arm receiver-recovery soundness rule).
    pub(in crate::executor) fn op_iter_done(&mut self) -> Result<(), VMError> {
        let (idx_bits, idx_kind) = self.pop_kinded()?;
        let (iter_bits, iter_kind) = self.pop_kinded()?;
        let idx = decode_iter_idx(idx_bits, idx_kind)?;

        // v2 typed array fast path: `as_v2_typed_array(bits, kind)` returns
        // Some only when `kind == UInt64` and the header pad byte stamps a
        // v2 typed array. No Arc share involved on this path.
        if let Some(view) =
            crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(iter_bits, iter_kind)
        {
            let done = idx < 0 || idx as u32 >= view.len;
            // Idx is plain numeric; iter is a raw `*mut TypedArray<_>`
            // pointer in `UInt64` (no refcount). `drop_with_kind` is a
            // no-op for both kinds, but call it for symmetry/future-proof.
            drop_with_kind(idx_bits, idx_kind);
            drop_with_kind(iter_bits, iter_kind);
            return self.push_kinded(done as u64, NativeKind::Bool);
        }

        // Heap-side dispatch — per-kind typed-Arc projection. Each arm
        // borrows the inner `Arc<T>` via `Arc::from_raw::<T>(bits)` +
        // `Arc::into_raw(arc)` to restore the slot's original share
        // without bumping the refcount, then reads `len()` from the
        // concrete payload. The slot's share retires via the
        // `drop_with_kind` at the bottom of the function.
        let done_result: Result<bool, VMError> = match iter_kind {
            // V3-S5 ckpt-4 (2026-05-15): `Ptr(HeapKind::TypedArray)` arm
            // body replaced with surface-and-stop via `ckpt4_surface`. The
            // `Arc::<TypedArrayData>::from_raw` reconstruction is no
            // longer constructible (the enum + `typed_array_data_len`
            // 22-arm helper were retired at ckpt-1 + ckpt-4 wholesale
            // deletion). Refcount discipline: the slot's share retires
            // via `drop_with_kind` at the bottom of the function. The
            // v2-raw `TypedArray<T>` per-element-kind rebuild reinstates
            // typed-array iteration via parallel `Ptr(HeapKind::
            // TypedArrayF64)` / `Ptr(HeapKind::TypedArrayI64)` arms
            // (downstream wave).
            NativeKind::Ptr(HeapKind::TypedArray) => {
                Err(ckpt4_surface(
                    "op_iter_done",
                    "iteration over legacy Arc<TypedArrayData> carrier",
                ))
            }
            NativeKind::Ptr(HeapKind::String) | NativeKind::String => {
                // SAFETY: per `KindedSlot::from_string_arc` / `String`
                // construction contract, slot bits are
                // `Arc::into_raw(Arc<String>)`. Reconstruct, read len, restore.
                let arc = unsafe { Arc::<String>::from_raw(iter_bits as *const String) };
                let len = arc.len();
                let _ = Arc::into_raw(arc);
                Ok(idx < 0 || idx as usize >= len)
            }
            NativeKind::Ptr(HeapKind::DataTable) => {
                // SAFETY: per `ValueSlot::from_data_table` contract, slot
                // bits are `Arc::into_raw(Arc<DataTable>)`.
                let arc = unsafe { Arc::<DataTable>::from_raw(iter_bits as *const DataTable) };
                let len = arc.row_count();
                let _ = Arc::into_raw(arc);
                Ok(idx < 0 || idx as usize >= len)
            }
            NativeKind::Ptr(HeapKind::TableView) => {
                // SAFETY: per `ValueSlot::from_table_view`-style contract,
                // slot bits are `Arc::into_raw(Arc<TableViewData>)`.
                let arc = unsafe {
                    Arc::<TableViewData>::from_raw(iter_bits as *const TableViewData)
                };
                let result = match arc.as_ref() {
                    TableViewData::TypedTable { table, .. } => {
                        Ok(idx < 0 || idx as usize >= table.row_count())
                    }
                    // Phase-2c surface: RowView / ColumnRef / IndexedTable
                    // iteration semantics weren't part of the kinded
                    // for-loop redesign. ADR-006 §2.7.4.
                    _ => Err(VMError::NotImplemented(
                        "op_iter_done SURFACE: TableViewData::{RowView,ColumnRef,\
                         IndexedTable} iteration — phase-2c, see ADR-006 §2.7.4"
                            .to_string(),
                    )),
                };
                let _ = Arc::into_raw(arc);
                result
            }
            NativeKind::Ptr(HeapKind::HashMap) => {
                // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): bits are
                // `Arc::into_raw(Arc<HashMapKindedRef>)` per ADR-006 §2.7.24
                // Q25.B SUPERSEDED. Recover the outer Arc, read len() via
                // the kinded ref's per-V len() accessor, restore.
                let arc = unsafe {
                    Arc::<HashMapKindedRef>::from_raw(iter_bits as *const HashMapKindedRef)
                };
                let len = arc.len();
                let _ = Arc::into_raw(arc);
                Ok(idx < 0 || idx as usize >= len)
            }
            // Per playbook §7 #4: legacy `HeapValue::Array` / `Range` /
            // `Iterator` variants were deleted from the heap layout
            // (post-§2.7.6 typed-Arc payload migration). Iteration over
            // those shapes is Phase 2c work — the kinded carrier for
            // ranges + the boxed-Vec `Array` payload both need redesign.
            // Surface, do not invent a Bool-default fallback (§2.7.7 #9).
            _ => Err(VMError::NotImplemented(format!(
                "op_iter_done SURFACE: iter_kind={:?} not supported as iterator \
                 in the kinded API (legacy Array/Range/Iterator HeapValue \
                 variants deleted) — phase-2c, see ADR-006 §2.7.4",
                iter_kind
            ))),
        };

        // Retire both popped shares. Idx kind has no heap payload; iter
        // kind may have an `Arc<T>` share that `drop_with_kind` decrements
        // (per the §2.7.13 / §2.7.16 typed-Arc dispatch arm in
        // `vm_impl::stack::drop_with_kind`).
        drop_with_kind(idx_bits, idx_kind);
        drop_with_kind(iter_bits, iter_kind);
        let done = done_result?;
        self.push_kinded(done as u64, NativeKind::Bool)
    }

    /// Iterator-protocol "next element" fetch.
    ///
    /// Stack on entry: `[..., iter, idx]`.
    /// Stack on exit: `[..., element]` with the element's `NativeKind`
    /// sourced from the matched heap arm (§2 / playbook "loop iteration
    /// value kind from iterator's element FieldType").
    ///
    /// The element-kind decision per heap arm:
    ///   - `TypedArrayData::I64` / `I8..I32` / `U8..U64`  → `Int64`
    ///     (sign- or zero-extended to i64 per the source family).
    ///   - `TypedArrayData::F64` / `F32` / `FloatSlice`   → `Float64`.
    ///   - `TypedArrayData::Bool`                          → `Bool`.
    ///   - `TypedArrayData::String`                        → `String`
    ///     (an `Arc<String>` share is bumped via `Arc::increment_strong_count`
    ///     before being handed to `push_kinded`).
    ///   - `HeapValue::String`                             → `Char` (per
    ///     codepoint), `NativeKind::Ptr(HeapKind::Char)`.
    ///   - `the-deleted-heterogeneous-element-carrier`                     → SURFACE
    ///     (polymorphic `Arc<HeapValue>` carriers don't have a single
    ///     element kind; phase-2c work — see ADR-006 §2.7.4).
    ///
    /// Both popped shares (`iter`, `idx`) are retired with `drop_with_kind`
    /// before return; the pushed element owns its own retained share.
    pub(in crate::executor) fn op_iter_next(&mut self) -> Result<(), VMError> {
        let (idx_bits, idx_kind) = self.pop_kinded()?;
        let (iter_bits, iter_kind) = self.pop_kinded()?;
        let idx = decode_iter_idx(idx_bits, idx_kind)?;

        // v2 typed array fast path. Element kind from `view.elem_type`
        // per playbook §2 (capture per element from the iterator's
        // element FieldType — here the v2 element-type byte stamped on
        // the heap header).
        if let Some(view) =
            crate::executor::v2_handlers::v2_array_detect::as_v2_typed_array(iter_bits, iter_kind)
        {
            use crate::executor::v2_handlers::v2_array_detect::V2ElemType;
            use shape_value::v2::typed_array::TypedArray;
            // Out-of-range push the §2 sentinel (zero bits + Bool kind).
            if idx < 0 || idx as u32 >= view.len {
                drop_with_kind(idx_bits, idx_kind);
                drop_with_kind(iter_bits, iter_kind);
                return self.push_kinded(Self::NONE_BITS, NativeKind::Bool);
            }
            let i = idx as u32;
            let push_result = match view.elem_type {
                V2ElemType::I64 => {
                    let arr = view.ptr as *const TypedArray<i64>;
                    let v = unsafe { TypedArray::<i64>::get_unchecked(arr, i) };
                    self.push_kinded(v as u64, NativeKind::Int64)
                }
                V2ElemType::I32 => {
                    let arr = view.ptr as *const TypedArray<i32>;
                    let v = unsafe { TypedArray::<i32>::get_unchecked(arr, i) };
                    // Sign-extend i32 → i64 before reinterpret to u64.
                    self.push_kinded(v as i64 as u64, NativeKind::Int64)
                }
                V2ElemType::F64 => {
                    let arr = view.ptr as *const TypedArray<f64>;
                    let v = unsafe { TypedArray::<f64>::get_unchecked(arr, i) };
                    self.push_kinded(v.to_bits(), NativeKind::Float64)
                }
                V2ElemType::Bool => {
                    let arr = view.ptr as *const TypedArray<u8>;
                    let v = unsafe { TypedArray::<u8>::get_unchecked(arr, i) };
                    self.push_kinded((v != 0) as u64, NativeKind::Bool)
                }
                // W12 S1 (2026-05-13) — sized-integer iter reads.
                V2ElemType::I8 => {
                    let arr = view.ptr as *const TypedArray<i8>;
                    let v = unsafe { TypedArray::<i8>::get_unchecked(arr, i) };
                    self.push_kinded(v as i64 as u64, NativeKind::Int8)
                }
                V2ElemType::U8 => {
                    let arr = view.ptr as *const TypedArray<u8>;
                    let v = unsafe { TypedArray::<u8>::get_unchecked(arr, i) };
                    self.push_kinded(v as u64, NativeKind::UInt8)
                }
                V2ElemType::I16 => {
                    let arr = view.ptr as *const TypedArray<i16>;
                    let v = unsafe { TypedArray::<i16>::get_unchecked(arr, i) };
                    self.push_kinded(v as i64 as u64, NativeKind::Int16)
                }
                V2ElemType::U16 => {
                    let arr = view.ptr as *const TypedArray<u16>;
                    let v = unsafe { TypedArray::<u16>::get_unchecked(arr, i) };
                    self.push_kinded(v as u64, NativeKind::UInt16)
                }
                V2ElemType::U32 => {
                    let arr = view.ptr as *const TypedArray<u32>;
                    let v = unsafe { TypedArray::<u32>::get_unchecked(arr, i) };
                    self.push_kinded(v as u64, NativeKind::UInt32)
                }
                // V2ElemType::U64 omitted — deferred to S1.5 per S1 reopen.
                // Wave 2 Agent A1 (2026-05-14) — F32 + Char iter reads.
                V2ElemType::F32 => {
                    let arr = view.ptr as *const TypedArray<f32>;
                    let v = unsafe { TypedArray::<f32>::get_unchecked(arr, i) };
                    self.push_kinded(v.to_bits() as u64, NativeKind::Float32)
                }
                V2ElemType::Char => {
                    let arr = view.ptr as *const TypedArray<char>;
                    let v = unsafe { TypedArray::<char>::get_unchecked(arr, i) };
                    self.push_kinded(v as u32 as u64, NativeKind::Char)
                }
                // Wave 2 Agent A2 (2026-05-14) — String + Decimal iter reads.
                // Per audit §4.1.B.4: retain the per-element header before
                // pushing the slot bits as NativeKind::StringV2 / DecimalV2.
                V2ElemType::String => {
                    use shape_value::v2::refcount::v2_retain;
                    use shape_value::v2::string_obj::StringObj;
                    let arr = view.ptr as *const TypedArray<*const StringObj>;
                    let elem_ptr = unsafe { TypedArray::<*const StringObj>::get_unchecked(arr, i) };
                    unsafe { v2_retain(&(*elem_ptr).header) };
                    self.push_kinded(elem_ptr as u64, NativeKind::StringV2)
                }
                V2ElemType::Decimal => {
                    use shape_value::v2::decimal_obj::DecimalObj;
                    use shape_value::v2::refcount::v2_retain;
                    let arr = view.ptr as *const TypedArray<*const DecimalObj>;
                    let elem_ptr = unsafe { TypedArray::<*const DecimalObj>::get_unchecked(arr, i) };
                    unsafe { v2_retain(&(*elem_ptr).header) };
                    self.push_kinded(elem_ptr as u64, NativeKind::DecimalV2)
                }
            };
            drop_with_kind(idx_bits, idx_kind);
            drop_with_kind(iter_bits, iter_kind);
            return push_result;
        }

        // Heap-side dispatch — per-kind typed-Arc projection (same shape
        // as `op_iter_done`). Each arm borrows the inner `Arc<T>` via
        // `Arc::from_raw::<T>(bits)` + `Arc::into_raw(arc)` restore.
        let push_result: Result<(), VMError> = match iter_kind {
            // V3-S5 ckpt-4 (2026-05-15): `Ptr(HeapKind::TypedArray)` arm
            // body replaced with surface-and-stop. The previous
            // `push_typed_array_element` 22-arm helper (deleted below in
            // this same hunk) read the inner `Arc<TypedArrayData>` and
            // dispatched per-element-kind. With the enum + wrapper layer
            // retired wholesale, that dispatch graph has no discriminator.
            NativeKind::Ptr(HeapKind::TypedArray) => {
                let _ = idx; // kept for symmetry with surfaced arms
                Err(ckpt4_surface(
                    "op_iter_next",
                    "iteration over legacy Arc<TypedArrayData> carrier",
                ))
            }
            NativeKind::Ptr(HeapKind::String) | NativeKind::String => {
                // SAFETY: slot bits are `Arc::into_raw(Arc<String>)`.
                let arc = unsafe { Arc::<String>::from_raw(iter_bits as *const String) };
                let result = if idx < 0 {
                    self.push_kinded(Self::NONE_BITS, NativeKind::Bool)
                } else {
                    match arc.chars().nth(idx as usize) {
                        Some(c) => self.push_kinded(c as u64, NativeKind::Ptr(HeapKind::Char)),
                        None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool),
                    }
                };
                let _ = Arc::into_raw(arc);
                result
            }
            NativeKind::Ptr(HeapKind::DataTable) => {
                // Phase-2c surface: DataTable row materialization at
                // `IterNext` used the deleted `ValueWord::from_row_view`
                // packed-tag carrier; the kinded redesign of the row-view
                // payload is pending. Same disposition as
                // `executor/tests/table_iteration.rs` `todo!("phase-2c …")`
                // markers.
                Err(VMError::NotImplemented(
                    "op_iter_next SURFACE: DataTable row materialization \
                     — phase-2c, see ADR-006 §2.7.4 \
                     (RowView carrier pending kinded redesign)"
                        .to_string(),
                ))
            }
            NativeKind::Ptr(HeapKind::TableView) => {
                // Phase-2c surface: TableView row materialization at
                // `IterNext` used the deleted `ValueWord::from_row_view`
                // packed-tag carrier. Same disposition as DataTable.
                Err(VMError::NotImplemented(
                    "op_iter_next SURFACE: TableView row materialization \
                     — phase-2c, see ADR-006 §2.7.4 \
                     (RowView carrier pending kinded redesign)"
                        .to_string(),
                ))
            }
            NativeKind::Ptr(HeapKind::HashMap) => {
                // Phase-2c surface: HashMap iteration yields a
                // `[key, value]` pair which used the deleted
                // `vmarray_from_vec` polymorphic-array constructor —
                // post-bundle-A the user-facing pattern is
                // `for entry in m.entries()` (Entry TypedObject with
                // `{key, value}` fields), iterating the
                // `TypedArrayData::TypedObject` carrier above, not the
                // HashMap itself. Direct HashMap iteration is pending
                // typed-buffer-of-heap construction helpers.
                Err(VMError::NotImplemented(
                    "op_iter_next SURFACE: direct HashMap iteration is not \
                     supported — use `for entry in m.entries()` instead. \
                     Phase-2c, see ADR-006 §2.7.4"
                        .to_string(),
                ))
            }
            // Same playbook §7 #4 disposition as `op_iter_done`: legacy
            // Array / Range / Iterator variants were deleted; iteration
            // over those shapes is phase-2c work.
            _ => Err(VMError::NotImplemented(format!(
                "op_iter_next SURFACE: iter_kind={:?} not supported as iterator \
                 in the kinded API (legacy Array/Range/Iterator HeapValue \
                 variants deleted) — phase-2c, see ADR-006 §2.7.4",
                iter_kind
            ))),
        };

        drop_with_kind(idx_bits, idx_kind);
        drop_with_kind(iter_bits, iter_kind);
        push_result
    }

    // V3-S5 ckpt-4 (2026-05-15): `push_typed_array_element` 22-arm helper
    // (deleted in this hunk) was the per-element-kind dispatch shell that
    // op_iter_next called for `Ptr(HeapKind::TypedArray)` iter_kind. With
    // the `TypedArrayData` enum + `TypedBuffer<T>` wrapper layer retired
    // wholesale at V3-S5 ckpt-1..ckpt-4 (W12-typed-array-data-deletion-
    // audit §3.5/§B + ADR-006 §2.7.24 Q25.A SUPERSEDED), the per-arm
    // discriminator is gone. The 22-arm body (I64/F64/Bool/I8/I16/I32/U8/
    // U16/U32/U64/F32/String/Decimal/BigInt/Char/TypedObject) is deleted
    // wholesale here; reinstatement comes through the downstream-wave
    // v2-raw `TypedArray<T>` per-element-kind rebuild via parallel
    // `Ptr(HeapKind::TypedArrayF64)` / `Ptr(HeapKind::TypedArrayI64)` /
    // etc. arms. Refusal #1 binding.

    /// Sentinel for "no element" in the iterator-protocol element-push
    /// path. Bool-kind 0. Preserved for downstream-wave reuse.
    #[allow(dead_code)]
    const NONE_BITS: u64 = 0;
}

// V3-S5 ckpt-4 (2026-05-15): unit tests over the deleted
// `build_typed_object_array` fixture + `TypedArrayData::TypedObject`
// per-arm iter_done/iter_next pinning DELETED in lockstep with the
// `TypedArrayData` enum + `TypedBuffer<T>` wrapper layer. The
// W17-iterator-reference-rebuild regression target (the "heap arm is
// Reference" surface when ordinal 20 of TypedArrayData aliased ordinal
// 20 of HeapValue::Reference) is preserved in spirit at the
// downstream-wave v2-raw per-element-kind rebuild's smoke-test layer;
// pre-rebuild, no fixture is materializable (the wholesale-deleted
// `Arc<TypedBuffer<TypedObjectPtr>>` payload was the only way to
// construct the killer-case shape pre-V3-S5). W12 audit §3.5/§B +
// ADR-006 §2.7.24 Q25.A SUPERSEDED + Refusal #1 binding.
