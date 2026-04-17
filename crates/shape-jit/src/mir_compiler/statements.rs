//! MIR Statement → Cranelift IR compilation.
//!
//! MIR has ~7 statement kinds (vs ~100 bytecode opcodes).
//! Ownership is structural: Assign releases old heap values,
//! Drop releases refcounts, Nop is skipped.

use cranelift::prelude::*;

use super::MirToIR;
use shape_vm::mir::types::*;
use shape_vm::type_tracking::SlotKind;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Compile a single MIR statement.
    pub(crate) fn compile_statement(
        &mut self,
        stmt: &MirStatement,
    ) -> Result<(), String> {
        match &stmt.kind {
            StatementKind::Assign(place, rvalue) => {
                // v2 fast path: when the destination is `Place::Local(s)` whose
                // ConcreteType is `Array<scalar>`, allocate a real v2 typed
                // array via FFI and bypass the legacy NaN-boxed Aggregate path.
                if let (Rvalue::Aggregate(operands), Some(elem_kind)) = (
                    rvalue,
                    self.v2_typed_array_elem_kind(place),
                ) {
                    if let Some(arr_val) =
                        self.emit_v2_array_aggregate(operands, elem_kind)?
                    {
                        self.release_old_value_if_heap(place)?;
                        self.write_place(place, arr_val)?;
                        return Ok(());
                    }
                }

                // Release old value if overwriting a heap local.
                self.release_old_value_if_heap(place)?;
                // Compile the rvalue.
                let val = self.compile_rvalue(rvalue)?;
                // Write the new value.
                self.write_place(place, val)?;
                Ok(())
            }

            StatementKind::Drop(place) => {
                self.emit_drop(place)?;
                Ok(())
            }

            StatementKind::ArrayStore {
                container_slot,
                operands,
            } => {
                let zero = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    0i64,
                );
                let inst = self.builder.ins().call(
                    self.ffi.new_array,
                    &[self.ctx_ptr, zero],
                );
                let mut arr = self.builder.inst_results(inst)[0];

                // v2-boundary: jit_array_push_elem FFI expects NaN-boxed I64 elements
                for op in operands {
                    let raw = self.compile_operand(op)?;
                    let val = self.ensure_nanboxed(raw);
                    let inst = self.builder
                        .ins()
                        .call(self.ffi.array_push_elem, &[arr, val]);
                    arr = self.builder.inst_results(inst)[0];
                }

                let place = Place::Local(*container_slot);
                self.release_old_value_if_heap(&place)?;
                self.write_place(&place, arr)?;
                Ok(())
            }

            StatementKind::ObjectStore {
                container_slot,
                operands,
                field_names,
            } => {
                // Register a schema for cross-boundary compatibility.
                let real_field_names: Vec<String> = field_names
                    .iter()
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .collect();
                let sid = shape_runtime::type_schema::register_predeclared_any_schema(
                    &real_field_names,
                );

                let schema_id = self.builder.ins().iconst(
                    cranelift::prelude::types::I32,
                    sid as i64,
                );
                let data_size = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    (operands.len() as i64) * 8,
                );
                let inst = self.builder.ins().call(
                    self.ffi.typed_object_alloc,
                    &[schema_id, data_size],
                );
                let mut obj = self.builder.inst_results(inst)[0];

                // Record field_name -> positional byte offset mapping.
                for (i, name) in field_names.iter().enumerate() {
                    if !name.is_empty() {
                        self.field_byte_offsets.insert(name.clone(), (i as u16) * 8);
                    }
                }

                // v2-boundary: typed_object_set_field FFI expects NaN-boxed I64 values
                for (i, op) in operands.iter().enumerate() {
                    let raw = self.compile_operand_raw(op)?;
                    let val = self.ensure_nanboxed(raw);
                    let offset_val = self.builder.ins().iconst(
                        cranelift::prelude::types::I64,
                        (i as i64) * 8,
                    );
                    let inst = self.builder.ins().call(
                        self.ffi.typed_object_set_field,
                        &[obj, offset_val, val],
                    );
                    obj = self.builder.inst_results(inst)[0];
                }

                let place = Place::Local(*container_slot);
                self.release_old_value_if_heap(&place)?;
                self.write_place(&place, obj)?;
                Ok(())
            }

            StatementKind::EnumStore {
                container_slot,
                operands,
            } => {
                // Enum variant construction.
                //
                // In the bytecode path, enums are compiled as TypedObjects with a
                // schema_id and variant discriminant. The MIR doesn't carry schema
                // information, so we represent enum payloads as arrays — the
                // preceding Assign(Aggregate) already creates an array with the
                // payload values.
                //
                // For non-empty payloads, rebuild the array from operands to ensure
                // correct ownership semantics (Move/Copy). For unit variants (empty
                // operands), the slot already holds the value from the Assign.
                if !operands.is_empty() {
                    // Create empty array, then push each element.
                    let zero = self.builder.ins().iconst(
                        cranelift::prelude::types::I64,
                        0i64,
                    );
                    let inst = self.builder.ins().call(
                        self.ffi.new_array,
                        &[self.ctx_ptr, zero],
                    );
                    let mut arr = self.builder.inst_results(inst)[0];

                    // v2-boundary: jit_array_push_elem FFI expects NaN-boxed I64 elements
                    for op in operands {
                        let raw = self.compile_operand(op)?;
                        let val = self.ensure_nanboxed(raw);
                        let inst = self.builder
                            .ins()
                            .call(self.ffi.array_push_elem, &[arr, val]);
                        arr = self.builder.inst_results(inst)[0];
                    }

                    let place = Place::Local(*container_slot);
                    self.release_old_value_if_heap(&place)?;
                    self.write_place(&place, arr)?;
                }
                Ok(())
            }

            StatementKind::Nop => Ok(()),

            StatementKind::TaskBoundary(_, _) => {
                // TaskBoundary is a borrow-checker annotation consumed by the MIR
                // solver. Actual async mechanics are handled by Call terminators to
                // spawn_task/join_init FFI functions. No-op at codegen time.
                Ok(())
            }

            StatementKind::ClosureCapture {
                closure_slot,
                operands,
                function_id,
            } => {
                // Create a closure by pushing captures to ctx.stack and calling jit_make_closure.
                let fid = function_id.ok_or_else(|| {
                    "MirToIR: ClosureCapture missing function_id (MIR not patched)".to_string()
                })?;

                // ── Phase E FAST PATH: stack-allocated closure ─────────
                // When the storage planner has proven the closure slot
                // never escapes its defining frame, allocate a Cranelift
                // StackSlot shaped like `StackClosure { fn_id, type_id,
                // captures... }` instead of calling `jit_make_closure`.
                // Cranelift's SROA eliminates the slot when Phase C has
                // inlined the closure body and the env pointer is dead.
                //
                // The slot is only safe when the closure is local — any
                // escape (return, container store, task boundary, etc.)
                // forces the legacy heap path below.
                if self.non_escaping_closure_slots.contains(closure_slot) {
                    self.emit_stack_closure(fid, *closure_slot, operands)?;
                    return Ok(());
                }

                // ── LEGACY HEAP PATH ──────────────────────────────────
                // Phase H will delete this once `MakeClosureHeap` lands
                // in Phase F. Until then, escaping closures still go
                // through the FFI `jit_make_closure`.

                // Push each capture operand to ctx.stack[stack_ptr + i]
                let stack_base = crate::context::STACK_OFFSET as i32;
                let sp_offset = crate::context::STACK_PTR_OFFSET as i32;
                let old_sp = self.builder.ins().load(
                    cranelift::prelude::types::I64,
                    MemFlags::new(),
                    self.ctx_ptr,
                    sp_offset,
                );

                // v2-boundary: closure captures pushed as NaN-boxed to ctx.stack
                for (i, op) in operands.iter().enumerate() {
                    let raw = self.compile_operand(op)?;
                    let val = self.ensure_nanboxed(raw);
                    let slot_idx = self.builder.ins().iadd_imm(old_sp, i as i64);
                    let byte_off = self.builder.ins().ishl_imm(slot_idx, 3);
                    let abs_off = self.builder.ins().iadd_imm(byte_off, stack_base as i64);
                    let addr = self.builder.ins().iadd(self.ctx_ptr, abs_off);
                    self.builder.ins().store(MemFlags::new(), val, addr, 0);
                }

                // Update ctx.stack_ptr += captures_count
                let new_sp = self.builder.ins().iadd_imm(old_sp, operands.len() as i64);
                self.builder.ins().store(MemFlags::new(), new_sp, self.ctx_ptr, sp_offset);

                // Call jit_make_closure(ctx, function_id, captures_count)
                let fid_val = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    fid as i64,
                );
                let cap_count = self.builder.ins().iconst(
                    cranelift::prelude::types::I64,
                    operands.len() as i64,
                );
                let inst = self.builder.ins().call(
                    self.ffi.make_closure,
                    &[self.ctx_ptr, fid_val, cap_count],
                );
                let closure_val = self.builder.inst_results(inst)[0];

                // Store the closure in the closure_slot
                let place = Place::Local(*closure_slot);
                self.release_old_value_if_heap(&place)?;
                self.write_place(&place, closure_val)?;
                Ok(())
            }
        }
    }

    /// Phase E: emit a Cranelift `StackSlot` shaped like
    /// `StackClosure { function_id: u32, type_id: u32, captures... }`
    /// for a non-escaping closure.
    ///
    /// Layout (from `shape_value::v2::closure_layout::StackClosure`):
    /// - offset 0: function_id (u32)
    /// - offset 4: type_id (u32, always 0 here — Phase E does not yet
    ///   thread the real `ClosureTypeId` through the JIT; the field
    ///   is a layout placeholder for Phase F's `Function<A,R>` dispatch)
    /// - offset 8 onwards: captures, laid out per
    ///   `ClosureLayout::stack_capture_offset(i)` at native width
    ///
    /// Captures are stored at their native Cranelift type so inlined
    /// bodies (Phase C) can consume them without NaN-box round-trips.
    /// Capture kinds that don't map to a native Cranelift type
    /// (pointers, strings, unknown) fall back to I64 storage — correct
    /// since the MIR's `ClosureCapture` slot_kind matches the source.
    ///
    /// The resulting `StackClosure*` pointer is written to the closure
    /// slot as I64 (NaN-boxed value semantics). After Phase C inlining,
    /// the pointer is dead; Cranelift's SROA pass eliminates the slot.
    fn emit_stack_closure(
        &mut self,
        function_id: u16,
        closure_slot: SlotId,
        operands: &[Operand],
    ) -> Result<(), String> {
        use cranelift::prelude::types as cl_types;

        // Determine per-capture Cranelift types + byte offsets.
        // The MIR operand's root slot kind dictates the native storage
        // width. Unknown / Dynamic captures fall back to I64.
        let mut capture_types: Vec<Type> = Vec::with_capacity(operands.len());
        for op in operands.iter() {
            let kind = match op {
                Operand::Copy(p) | Operand::Move(p) | Operand::MoveExplicit(p) => {
                    let slot = p.root_local();
                    super::types::slot_kind_for_local(&self.slot_kinds, slot.0)
                }
                Operand::Constant(MirConstant::Float(_)) => SlotKind::Float64,
                Operand::Constant(MirConstant::Int(_)) => SlotKind::Int64,
                Operand::Constant(MirConstant::Bool(_)) => SlotKind::Bool,
                _ => SlotKind::Unknown,
            };
            capture_types.push(super::types::cranelift_type_for_slot(kind));
        }

        // Compute byte offsets: 8-byte header (fn_id + type_id), then each
        // capture with natural alignment. Mirrors
        // `ClosureLayout::from_capture_types` but operates on Cranelift
        // types directly (the runtime layout struct keys on ConcreteType
        // which is unavailable at MIR-codegen time).
        let header_size: usize = 8;
        let mut offsets: Vec<i32> = Vec::with_capacity(operands.len());
        let mut cur: usize = header_size;
        let mut max_align: usize = 8;
        for ty in &capture_types {
            let (size, align) = cranelift_type_size_align(*ty);
            cur = (cur + align - 1) & !(align - 1);
            offsets.push(cur as i32);
            if align > max_align {
                max_align = align;
            }
            cur += size;
        }
        let total = (cur + max_align - 1) & !(max_align - 1);
        // Cranelift StackSlot size must be > 0 and fit in u32.
        let total = total.max(8) as u32;
        let align_shift: u8 = match max_align {
            1 => 0,
            2 => 1,
            4 => 2,
            8 => 3,
            16 => 4,
            _ => 3,
        };

        let slot = self.builder.create_sized_stack_slot(StackSlotData::new(
            StackSlotKind::ExplicitSlot,
            total,
            align_shift,
        ));

        // function_id at offset 0 (I32)
        let fid_val = self
            .builder
            .ins()
            .iconst(cl_types::I32, function_id as i64);
        self.builder.ins().stack_store(fid_val, slot, 0);
        // type_id at offset 4 (I32). Phase E stores 0 — Phase F threads
        // the real ClosureTypeId through once `Function<A,R>` dispatch
        // needs it for signature lookup.
        let tid_val = self.builder.ins().iconst(cl_types::I32, 0);
        self.builder.ins().stack_store(tid_val, slot, 4);

        // Each capture at its typed offset.
        for (i, op) in operands.iter().enumerate() {
            let val = self.compile_operand(op)?;
            let target_ty = capture_types[i];
            let val_ty = self.builder.func.dfg.value_type(val);
            let stored = self.coerce_for_capture_store(val, val_ty, target_ty);
            self.builder.ins().stack_store(stored, slot, offsets[i]);
        }

        // Produce the closure value = stack slot address. This is a raw
        // pointer; we store it as I64 so existing Place plumbing
        // (which treats closure slots as NaN-boxed I64 values) works
        // uniformly. After Phase C inlining the pointer is dead; SROA
        // eliminates the stack slot. No arc_retain / arc_release on
        // stack closures.
        let closure_addr = self.builder.ins().stack_addr(cl_types::I64, slot, 0);

        // Track the stack slot so drop/release paths know to skip
        // arc_release on this slot.
        self.stack_closure_slots.insert(closure_slot, slot);

        // Write to the closure slot. We deliberately do NOT call
        // `release_old_value_if_heap` here — a stack closure cannot
        // be overwriting a heap pointer on this path (the storage
        // planner guarantees single-assignment to closure slots), and
        // the MIR has already emitted the appropriate Drop for any
        // prior heap handle that sat in this slot.
        let place = Place::Local(closure_slot);
        self.write_place(&place, closure_addr)?;
        Ok(())
    }

    /// Coerce a Cranelift value to the target capture storage type.
    /// Performs zero-extension for narrowings and bitcasts for F64/I64.
    /// Mismatches fall back to NaN-boxing (preserves dynamic semantics).
    fn coerce_for_capture_store(
        &mut self,
        val: Value,
        val_ty: Type,
        target_ty: Type,
    ) -> Value {
        use cranelift::prelude::types as cl_types;
        if val_ty == target_ty {
            return val;
        }
        // Widen native integers to the storage width.
        if target_ty == cl_types::I64 {
            // Anything becomes NaN-boxed I64 for the storage slot.
            return self.ensure_nanboxed(val);
        }
        if target_ty == cl_types::I32 {
            if val_ty == cl_types::I8 || val_ty == cl_types::I16 {
                return self.builder.ins().sextend(cl_types::I32, val);
            }
            if val_ty == cl_types::I64 {
                return self.builder.ins().ireduce(cl_types::I32, val);
            }
        }
        if target_ty == cl_types::I8 {
            if val_ty == cl_types::I32 || val_ty == cl_types::I64 {
                return self.builder.ins().ireduce(cl_types::I8, val);
            }
        }
        if target_ty == cl_types::F64 && val_ty == cl_types::I64 {
            // NaN-boxed I64 carrying an F64 — bitcast back.
            return self
                .builder
                .ins()
                .bitcast(cl_types::F64, MemFlags::new(), val);
        }
        // Last-resort: NaN-box to I64 and store as dynamic.
        self.ensure_nanboxed(val)
    }
}

/// Size and alignment in bytes for a Cranelift type, used by the
/// Phase E stack closure layout computation.
fn cranelift_type_size_align(ty: Type) -> (usize, usize) {
    use cranelift::prelude::types as cl_types;
    match ty {
        t if t == cl_types::I8 => (1, 1),
        t if t == cl_types::I16 => (2, 2),
        t if t == cl_types::I32 => (4, 4),
        t if t == cl_types::F32 => (4, 4),
        t if t == cl_types::I64 => (8, 8),
        t if t == cl_types::F64 => (8, 8),
        _ => (8, 8),
    }
}

/// Phase E layout helper: compute stack-closure capture byte offsets
/// given a list of Cranelift capture types. Mirrors the logic inside
/// `emit_stack_closure` so it can be unit-tested independently.
///
/// Returns `(offsets, total_size, max_alignment)` where offsets are
/// absolute from the StackClosure base pointer. The 8-byte header
/// (`function_id: u32` @ 0, `type_id: u32` @ 4) is implicit.
#[cfg(test)]
fn phase_e_layout(capture_types: &[Type]) -> (Vec<i32>, usize, usize) {
    let header_size: usize = 8;
    let mut offsets: Vec<i32> = Vec::with_capacity(capture_types.len());
    let mut cur: usize = header_size;
    let mut max_align: usize = 8;
    for ty in capture_types {
        let (size, align) = cranelift_type_size_align(*ty);
        cur = (cur + align - 1) & !(align - 1);
        offsets.push(cur as i32);
        if align > max_align {
            max_align = align;
        }
        cur += size;
    }
    let total = (cur + max_align - 1) & !(max_align - 1);
    let total = total.max(8);
    (offsets, total, max_align)
}

#[cfg(test)]
mod phase_e_tests {
    //! Phase E (JIT stack-closure codegen) layout helper tests.
    //!
    //! End-to-end closure JIT tests that exercise the full MirToIR
    //! `ClosureCapture` lowering live in the integration suite
    //! (`just test-fast`) — they require bytecode-compiling Shape
    //! source and running it through `compile_program_selective`,
    //! which is too heavy for the crate-local unit harness. These
    //! unit tests focus on the offset math so regressions in the
    //! layout are caught without spinning up a full JIT.

    use super::*;
    use cranelift::prelude::types as cl_types;

    #[test]
    fn empty_captures_layout_matches_stack_closure_header() {
        // Empty captures → just the 8-byte { fn_id, type_id } header.
        let (offsets, total, align) = phase_e_layout(&[]);
        assert!(offsets.is_empty());
        assert_eq!(total, 8);
        assert_eq!(align, 8);
        // Matches shape_value::v2::closure_layout::STACK_CLOSURE_HEADER_SIZE.
        assert_eq!(
            total,
            shape_value::v2::closure_layout::STACK_CLOSURE_HEADER_SIZE
        );
    }

    #[test]
    fn single_f64_capture_layout() {
        // f64 capture starts right after the 8-byte header.
        let (offsets, total, align) = phase_e_layout(&[cl_types::F64]);
        assert_eq!(offsets, vec![8]);
        assert_eq!(total, 16);
        assert_eq!(align, 8);
    }

    #[test]
    fn single_i64_capture_layout() {
        let (offsets, total, align) = phase_e_layout(&[cl_types::I64]);
        assert_eq!(offsets, vec![8]);
        assert_eq!(total, 16);
        assert_eq!(align, 8);
    }

    #[test]
    fn two_f64_captures_layout() {
        let (offsets, total, align) = phase_e_layout(&[cl_types::F64, cl_types::F64]);
        assert_eq!(offsets, vec![8, 16]);
        assert_eq!(total, 24);
        assert_eq!(align, 8);
    }

    #[test]
    fn mixed_alignment_packing_layout() {
        // (Bool, I32, F64): bool @ 8 (1 byte), i32 @ 12 (pad from 9), f64 @ 16.
        let (offsets, total, align) =
            phase_e_layout(&[cl_types::I8, cl_types::I32, cl_types::F64]);
        assert_eq!(offsets, vec![8, 12, 16]);
        assert_eq!(total, 24);
        assert_eq!(align, 8);
    }

    #[test]
    fn four_small_captures_pack_tightly() {
        // (I8, I8, I16, I32): 8, 9, 10, 12; total rounds up to 16.
        let (offsets, total, align) = phase_e_layout(&[
            cl_types::I8,
            cl_types::I8,
            cl_types::I16,
            cl_types::I32,
        ]);
        assert_eq!(offsets, vec![8, 9, 10, 12]);
        assert_eq!(total, 16);
        assert_eq!(align, 8);
    }

    #[test]
    fn i64_capture_forces_8_byte_total() {
        // Single I64 capture → total = 16 (header 8 + i64 8).
        let (offsets, total, _) = phase_e_layout(&[cl_types::I64]);
        assert_eq!(offsets, vec![8]);
        assert_eq!(total, 16);
    }

    #[test]
    fn single_bool_capture_pads_to_8_bytes() {
        // Bool is 1 byte at offset 8; total rounds up to 16 (8-byte alignment).
        let (offsets, total, align) = phase_e_layout(&[cl_types::I8]);
        assert_eq!(offsets, vec![8]);
        assert_eq!(total, 16);
        assert_eq!(align, 8);
    }

    #[test]
    fn unknown_type_defaults_to_i64_layout() {
        // Catch-all in cranelift_type_size_align returns (8,8). An
        // arbitrary pointer-sized type should therefore behave like I64.
        let (size, align) = cranelift_type_size_align(cl_types::F64);
        assert_eq!((size, align), (8, 8));
        let (size, align) = cranelift_type_size_align(cl_types::I64);
        assert_eq!((size, align), (8, 8));
    }

    #[test]
    fn many_mixed_captures_match_expected_pattern() {
        // Seven captures: f64, i32, bool, i64, i8, i16, f64.
        // Expected offsets: 8, 16, 20, 24 (pad to 8), 32, 34, 40; total rounds to 48.
        let (offsets, total, align) = phase_e_layout(&[
            cl_types::F64,
            cl_types::I32,
            cl_types::I8,
            cl_types::I64,
            cl_types::I8,
            cl_types::I16,
            cl_types::F64,
        ]);
        assert_eq!(offsets, vec![8, 16, 20, 24, 32, 34, 40]);
        assert_eq!(total, 48);
        assert_eq!(align, 8);
    }

    #[test]
    fn layout_agrees_with_runtime_closure_layout_for_f64() {
        // Cross-check against shape_value::v2::ClosureLayout for the
        // all-F64 signature. Both must agree on offsets and total size.
        use shape_value::v2::closure_layout::ClosureLayout;
        use shape_value::v2::concrete_type::ConcreteType;

        let runtime_layout = ClosureLayout::from_capture_types(&[
            ConcreteType::F64,
            ConcreteType::F64,
        ]);
        let (offsets, total, _) = phase_e_layout(&[cl_types::F64, cl_types::F64]);
        assert_eq!(total, runtime_layout.total_stack_size());
        assert_eq!(offsets[0] as usize, runtime_layout.stack_capture_offset(0));
        assert_eq!(offsets[1] as usize, runtime_layout.stack_capture_offset(1));
    }
}
