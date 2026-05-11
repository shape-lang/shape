//! Loop control operations for the VM executor
//!
//! Handles: LoopStart, LoopEnd, Break, Continue, IterNext, IterDone
//!
//! ADR-006 §2.7.6 / §2.7.7 / Wave-β `B10-loops-heap` (playbook §10):
//! heap-side dispatch in `op_iter_next` / `op_iter_done` goes through
//! `ValueSlot::from_raw(bits).as_heap_value()` + `HeapValue::*` match
//! (Q8 single-discriminator). Iterator + idx are popped via the kinded
//! API; element kind for the pushed value is sourced from the matched
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
use shape_value::heap_value::{HeapKind, HeapValue, TableViewData, TypedArrayData};
use shape_value::{NativeKind, VMError, ValueSlot};

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
    /// dispatch on `iter_kind` matches against `slot.as_heap_value()` per
    /// Q8 (single-discriminator) — no per-heap-variant accessor on the
    /// carrier.
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

        // Heap-side dispatch (Q8): single-discriminator pattern through
        // `ValueSlot::as_heap_value()` + `HeapValue::*` match. The bits
        // are an `Arc::into_raw::<T>` payload only on `Ptr(HeapKind::*)`
        // and `String` kinds; non-heap kinds are wrong-shape iter input
        // and surface as a `TypeError` after dropping both shares.
        let done_result: Result<bool, VMError> = match iter_kind {
            NativeKind::Ptr(HeapKind::TypedArray)
            | NativeKind::Ptr(HeapKind::String)
            | NativeKind::String
            | NativeKind::Ptr(HeapKind::DataTable)
            | NativeKind::Ptr(HeapKind::TableView)
            | NativeKind::Ptr(HeapKind::HashMap) => {
                // SAFETY: per the §2.7.7 ownership contract, when `iter_kind`
                // selects one of these heap arms, `iter_bits` is the result
                // of `Arc::into_raw::<T>` for the matching `T`. Building a
                // borrowing `ValueSlot` view over the bits and calling
                // `as_heap_value()` is the Q8-sanctioned single-discriminator
                // dispatch; the `Arc` share retired by `drop_with_kind`
                // below balances the `pop_kinded` transfer.
                let slot = ValueSlot::from_raw(iter_bits);
                let hv = slot.as_heap_value();
                match hv {
                    HeapValue::TypedArray(arr_arc) => {
                        let len = match arr_arc.as_ref() {
                            TypedArrayData::I64(a) => a.len(),
                            TypedArrayData::F64(a) => a.len(),
                            TypedArrayData::Bool(a) => a.len(),
                            TypedArrayData::I8(a) => a.len(),
                            TypedArrayData::I16(a) => a.len(),
                            TypedArrayData::I32(a) => a.len(),
                            TypedArrayData::U8(a) => a.len(),
                            TypedArrayData::U16(a) => a.len(),
                            TypedArrayData::U32(a) => a.len(),
                            TypedArrayData::U64(a) => a.len(),
                            TypedArrayData::F32(a) => a.len(),
                            TypedArrayData::String(a) => a.len(),
                            TypedArrayData::HeapValue(a) => a.len(),
                            TypedArrayData::FloatSlice { len, .. } => *len as usize,
                            TypedArrayData::Matrix(m) => m.data.len(),
                            // W17-typed-carrier-bundle-A commit 1/4: §2.7.24 Q25.A specialized arms.
                            // No construction sites on this branch — surface-and-stop until commit 3.
                            TypedArrayData::Decimal(_)
                            | TypedArrayData::BigInt(_)
                            | TypedArrayData::DateTime(_)
                            | TypedArrayData::Timespan(_)
                            | TypedArrayData::Duration(_)
                            | TypedArrayData::Instant(_)
                            | TypedArrayData::Char(_)
                            | TypedArrayData::TypedObject(_)
                            | TypedArrayData::TraitObject(_) => unreachable!(
                                "TypedArrayData specialized variant reached in W17-typed-carrier-bundle-A commit 1/4: no construction sites yet (ADR-006 §2.7.24 Q25.A)"
                            ),
                        };
                        Ok(idx < 0 || idx as usize >= len)
                    }
                    HeapValue::String(s) => Ok(idx < 0 || idx as usize >= s.len()),
                    HeapValue::DataTable(dt) => Ok(idx < 0 || idx as usize >= dt.row_count()),
                    HeapValue::TableView(tv) => match tv.as_ref() {
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
                    },
                    HeapValue::HashMap(hm) => Ok(idx < 0 || idx as usize >= hm.keys.len()),
                    // The heap arm disagrees with `iter_kind`. ADR-005 §1
                    // single-discriminator violation — surface, never
                    // paper over.
                    other => Err(VMError::NotImplemented(format!(
                        "op_iter_done SURFACE: iter_kind={:?} but heap arm is {:?} — \
                         ADR-005 §1 single-discriminator violation",
                        iter_kind,
                        other.kind()
                    ))),
                }
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
        // kind may have an `Arc<T>` share that `drop_with_kind` decrements.
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
    ///   - `TypedArrayData::HeapValue`                     → SURFACE
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
            };
            drop_with_kind(idx_bits, idx_kind);
            drop_with_kind(iter_bits, iter_kind);
            return push_result;
        }

        // Heap-side dispatch (Q8 / single-discriminator). Same kind-gate
        // as `op_iter_done`: only the heap-bearing kinds reach the
        // `as_heap_value()` view; everything else surfaces.
        let push_result: Result<(), VMError> = match iter_kind {
            NativeKind::Ptr(HeapKind::TypedArray)
            | NativeKind::Ptr(HeapKind::String)
            | NativeKind::String
            | NativeKind::Ptr(HeapKind::DataTable)
            | NativeKind::Ptr(HeapKind::TableView)
            | NativeKind::Ptr(HeapKind::HashMap) => {
                // SAFETY: same construction-side contract as `op_iter_done`
                // — `iter_bits` is `Arc::into_raw::<T>` for the matching
                // heap variant.
                let slot = ValueSlot::from_raw(iter_bits);
                let hv = slot.as_heap_value();
                match hv {
                    HeapValue::TypedArray(arr_arc) => {
                        Self::push_typed_array_element(self, arr_arc.as_ref(), idx)
                    }
                    HeapValue::String(s) => {
                        // Out-of-range / negative idx → None sentinel.
                        if idx < 0 {
                            self.push_kinded(Self::NONE_BITS, NativeKind::Bool)
                        } else {
                            match s.chars().nth(idx as usize) {
                                Some(c) => self
                                    .push_kinded(c as u64, NativeKind::Ptr(HeapKind::Char)),
                                None => self.push_kinded(Self::NONE_BITS, NativeKind::Bool),
                            }
                        }
                    }
                    // Phase-2c surface: DataTable / TableView row
                    // materialization at `IterNext` used the deleted
                    // `ValueWord::from_row_view` packed-tag carrier; the
                    // kinded redesign of the row-view payload is pending.
                    // Same disposition as `executor/tests/table_iteration.rs`
                    // `todo!("phase-2c …")` markers.
                    HeapValue::DataTable(_) | HeapValue::TableView(_) => {
                        Err(VMError::NotImplemented(
                            "op_iter_next SURFACE: DataTable/TableView row \
                             materialization — phase-2c, see ADR-006 §2.7.4 \
                             (RowView carrier pending kinded redesign)"
                                .to_string(),
                        ))
                    }
                    // Phase-2c surface: HashMap iteration yielded a
                    // `[key, value]` two-element array via the deleted
                    // `vmarray_from_vec` constructor. The kinded
                    // equivalent would push a fresh
                    // `Arc<TypedArrayData::HeapValue>` two-slot buffer —
                    // pending typed-buffer-of-heap construction helpers.
                    HeapValue::HashMap(_) => Err(VMError::NotImplemented(
                        "op_iter_next SURFACE: HashMap iteration yields a \
                         [key, value] pair which used the deleted \
                         `vmarray_from_vec` polymorphic-array constructor \
                         — phase-2c, see ADR-006 §2.7.4"
                            .to_string(),
                    )),
                    other => Err(VMError::NotImplemented(format!(
                        "op_iter_next SURFACE: iter_kind={:?} but heap arm is {:?} \
                         — ADR-005 §1 single-discriminator violation",
                        iter_kind,
                        other.kind()
                    ))),
                }
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

    /// Push the `idx`-th element of a `TypedArrayData` with the matching
    /// element `NativeKind`. Out-of-range → `(0, NativeKind::Bool)` None
    /// sentinel per §2.7 (Drop-safe).
    ///
    /// Element-kind sourcing per the matched arm follows playbook §2:
    /// loop iteration value kind from iterator's element FieldType,
    /// captured per element from the runtime `TypedArrayData::*` arm.
    #[inline]
    fn push_typed_array_element(
        vm: &mut VirtualMachine,
        arr: &TypedArrayData,
        idx: i64,
    ) -> Result<(), VMError> {
        if idx < 0 {
            return vm.push_kinded(Self::NONE_BITS, NativeKind::Bool);
        }
        let u = idx as usize;
        match arr {
            TypedArrayData::I64(a) => match a.get(u) {
                Some(&v) => vm.push_kinded(v as u64, NativeKind::Int64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::I8(a) => match a.get(u) {
                Some(&v) => vm.push_kinded(v as i64 as u64, NativeKind::Int64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::I16(a) => match a.get(u) {
                Some(&v) => vm.push_kinded(v as i64 as u64, NativeKind::Int64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::I32(a) => match a.get(u) {
                Some(&v) => vm.push_kinded(v as i64 as u64, NativeKind::Int64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::U8(a) => match a.get(u) {
                Some(&v) => vm.push_kinded(v as u64, NativeKind::Int64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::U16(a) => match a.get(u) {
                Some(&v) => vm.push_kinded(v as u64, NativeKind::Int64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::U32(a) => match a.get(u) {
                Some(&v) => vm.push_kinded(v as u64, NativeKind::Int64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::U64(a) => match a.get(u) {
                Some(&v) => vm.push_kinded(v, NativeKind::Int64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::F64(a) => match a.get(u) {
                Some(&v) => vm.push_kinded(v.to_bits(), NativeKind::Float64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::F32(a) => match a.get(u) {
                Some(&v) => vm.push_kinded((v as f64).to_bits(), NativeKind::Float64),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::FloatSlice { parent, offset, len } => {
                if u >= *len as usize {
                    vm.push_kinded(Self::NONE_BITS, NativeKind::Bool)
                } else {
                    let off = *offset as usize;
                    let v = parent.data[off + u];
                    vm.push_kinded(v.to_bits(), NativeKind::Float64)
                }
            }
            TypedArrayData::Matrix(m) => {
                if u >= m.data.len() {
                    vm.push_kinded(Self::NONE_BITS, NativeKind::Bool)
                } else {
                    vm.push_kinded(m.data[u].to_bits(), NativeKind::Float64)
                }
            }
            TypedArrayData::Bool(a) => match a.get(u) {
                Some(&v) => vm.push_kinded((v != 0) as u64, NativeKind::Bool),
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            TypedArrayData::String(a) => match a.get(u) {
                Some(arc_str) => {
                    // SAFETY: `a` is alive (we hold `arr_arc` for the duration
                    // of the match; `arc_str` borrows from it). We bump the
                    // inner `Arc<String>` strong-count so the pushed share
                    // outlives the iterator's share that retires on
                    // `drop_with_kind` after this push.
                    unsafe {
                        std::sync::Arc::increment_strong_count(
                            std::sync::Arc::as_ptr(arc_str),
                        );
                    }
                    let bits = std::sync::Arc::as_ptr(arc_str) as u64;
                    vm.push_kinded(bits, NativeKind::String)
                }
                None => vm.push_kinded(Self::NONE_BITS, NativeKind::Bool),
            },
            // Polymorphic Arc<HeapValue> buffer — element kind cannot be
            // determined without inspecting each element's HeapValue arm,
            // and the kinded element redesign is phase-2c. Surface per
            // playbook §7 #4 + §2.7.4.
            TypedArrayData::HeapValue(_) => Err(VMError::NotImplemented(
                "op_iter_next SURFACE: TypedArrayData::HeapValue (Arc<HeapValue> \
                 polymorphic element buffer) — phase-2c, see ADR-006 §2.7.4 \
                 (per-element kinded carrier pending)"
                    .to_string(),
            )),
            // W17-typed-carrier-bundle-A commit 1/4: §2.7.24 Q25.A specialized arms.
            // No construction sites on this branch — surface-and-stop until commit 3.
            TypedArrayData::Decimal(_)
            | TypedArrayData::BigInt(_)
            | TypedArrayData::DateTime(_)
            | TypedArrayData::Timespan(_)
            | TypedArrayData::Duration(_)
            | TypedArrayData::Instant(_)
            | TypedArrayData::Char(_)
            | TypedArrayData::TypedObject(_)
            | TypedArrayData::TraitObject(_) => unreachable!(
                "TypedArrayData specialized variant reached in W17-typed-carrier-bundle-A commit 1/4: no construction sites yet (ADR-006 §2.7.24 Q25.A)"
            ),
        }
    }
}
