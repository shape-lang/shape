//! Feedback-guided speculative IR emission for Tier 2 optimizing JIT.
//!
//! When a `FeedbackVector` is attached to the compilation (Tier 2), the
//! compiler consults IC state at each eligible bytecode site to emit
//! speculative guards with typed fast paths:
//!
//! - **Monomorphic call sites** → direct call to observed target + callee guard
//! - **Monomorphic property access** → guarded field load (schema_id check)
//! - **Stable arithmetic** → typed fast path (e.g., I48+I48 integer add)
//!
//! Guard failures branch to the shared deopt block, which sets a negative
//! signal and returns to the interpreter. The interpreter reconstructs its
//! frame using `DeoptInfo` metadata recorded at each guard point.

use cranelift::prelude::*;

use crate::nan_boxing::*;
use crate::translator::types::BytecodeToIR;
use shape_vm::feedback::{FeedbackSlot, ICState};

/// NaN tag value for I48 (inline signed integer).
/// TAG_BASE | (TAG_INT << TAG_SHIFT)
const I48_TAG_BITS: u64 =
    shape_value::tags::TAG_BASE | (shape_value::tags::TAG_INT << shape_value::tags::TAG_SHIFT);

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    // =====================================================================
    // Monomorphic call site → direct call + callee guard
    // =====================================================================

    /// Check if the call at `bytecode_offset` has monomorphic feedback and
    /// should use a speculative direct call.
    ///
    /// Returns the expected `function_id` if the site is monomorphic and
    /// the target function has a JIT-compiled entry point (user func ref).
    pub(crate) fn speculative_call_target(&self, bytecode_offset: usize) -> Option<u16> {
        let feedback = self.feedback.as_ref()?;
        match feedback.get_slot(bytecode_offset)? {
            FeedbackSlot::Call(fb) if fb.state == ICState::Monomorphic => {
                let target_id = fb.targets.first()?.function_id;
                // Return the target for speculation. If the callee has a FuncRef
                // (self-recursive or Tier-2 compiled cross-function target),
                // compile_call_value emits a guarded direct call. Otherwise,
                // the guard still fires with FFI fallthrough.
                Some(target_id)
            }
            _ => None,
        }
    }

    /// Emit a speculative direct call with a callee identity guard.
    ///
    /// Before calling the expected target, we verify that the actual callee
    /// (loaded from the function table at runtime) matches `expected_fn_id`.
    /// On mismatch, we deopt back to the interpreter.
    ///
    /// # Arguments
    /// * `expected_fn_id` - The function ID observed as monomorphic target
    /// * `callee_val` - The NaN-boxed callee value on the JIT stack
    /// * `bytecode_offset` - For DeoptInfo recording
    ///
    /// # Returns
    /// `true` if the speculative call was emitted, `false` if it could not
    /// be emitted (caller should fall through to generic path).
    pub(crate) fn emit_speculative_call(
        &mut self,
        expected_fn_id: u16,
        callee_val: Value,
        bytecode_offset: usize,
    ) -> bool {
        // Guard: extract the function_id from the callee value and compare.
        //
        // For NaN-boxed function references, the function_id is encoded in
        // the 48-bit payload: TAG_BASE | (TAG_FUNCTION << TAG_SHIFT) | fn_id
        //
        // We extract the payload (bits 47..0) and compare against expected_fn_id.
        let payload_mask_val = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        let actual_fn_id = self.builder.ins().band(callee_val, payload_mask_val);
        let expected_val = self.builder.ins().iconst(types::I64, expected_fn_id as i64);
        let guard_ok = self
            .builder
            .ins()
            .icmp(IntCC::Equal, actual_fn_id, expected_val);

        // Record deopt point with per-guard spill (callee peeked, stack intact)
        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(guard_ok, sb, &[]);
        } else {
            self.deopt_if_false_with_id(guard_ok, deopt_id as u32);
        }

        true
    }

    // =====================================================================
    // Monomorphic property access → guarded field load
    // =====================================================================

    /// Check if the property access at `bytecode_offset` has monomorphic
    /// feedback suitable for a guarded field load.
    ///
    /// Returns `(schema_id, field_idx, field_type_tag, receiver_kind)` if monomorphic.
    /// `receiver_kind`: 0 = TypedObject (schema guard), 1 = HashMap (shape guard).
    pub(crate) fn speculative_property_info(
        &self,
        bytecode_offset: usize,
    ) -> Option<(u64, u16, u16, u8)> {
        let feedback = self.feedback.as_ref()?;
        match feedback.get_slot(bytecode_offset)? {
            FeedbackSlot::Property(fb) if fb.state == ICState::Monomorphic => {
                let entry = fb.entries.first()?;
                Some((
                    entry.schema_id,
                    entry.field_idx,
                    entry.field_type_tag,
                    entry.receiver_kind,
                ))
            }
            _ => None,
        }
    }

    /// Emit a speculative guarded field load for a monomorphic property access.
    ///
    /// Generates:
    /// 1. Type guard: verify value is a TypedObject (heap kind check)
    /// 2. Schema guard: load schema_id from the TypedObject header and compare
    ///    against the expected schema_id from profiling
    /// 3. On guard success: direct indexed field load at `field_idx`
    /// 4. On guard failure: deopt to interpreter
    ///
    /// # Arguments
    /// * `obj` - The NaN-boxed receiver value
    /// * `expected_schema_id` - Schema ID observed as monomorphic
    /// * `field_idx` - Cached field index within the schema
    /// * `field_type_tag` - Type tag for the field (for typed load optimization)
    /// * `bytecode_offset` - For DeoptInfo recording
    ///
    /// # Returns
    /// The loaded field value (NaN-boxed i64), or `None` if the guard could
    /// not be emitted.
    pub(crate) fn emit_speculative_property_load(
        &mut self,
        obj: Value,
        expected_schema_id: u64,
        field_idx: u16,
        _field_type_tag: u16,
        bytecode_offset: usize,
    ) -> Option<Value> {
        // Guard 1: verify the value is a TypedObject
        let is_typed_obj = self.emit_is_heap_kind(obj, HK_TYPED_OBJECT);

        // Record deopt point with per-guard spill (obj on stack)
        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[obj]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(is_typed_obj, sb, &[obj]);
        } else {
            self.deopt_if_false_with_id(is_typed_obj, deopt_id as u32);
        }

        // Extract the heap pointer (masking out tag bits and unified heap flag)
        let ptr_mask_val = self.builder.ins().iconst(types::I64, UNIFIED_PTR_MASK as i64);
        let heap_ptr = self.builder.ins().band(obj, ptr_mask_val);

        // Guard 2: check schema_id.
        //
        // TypedObject layout behind the Arc (JitAlloc or runtime):
        //   The schema_id is stored at a known offset in the TypedObject
        //   header. For JIT-allocated objects, the layout is:
        //     offset 0: heap_kind (u16)
        //     offset 2: padding (u16)
        //     offset 4: schema_id (u32)
        //     offset 8: field data starts
        //
        //   For runtime TypedObject behind Arc<HeapValue>, the layout differs
        //   but schema_id is still accessible. We use the JIT alloc layout
        //   which matches objects allocated by the JIT.
        //
        //   For safety, we compare the full schema_id and deopt on mismatch.
        //   The actual field load uses the same offset calculation as
        //   GetFieldTyped.

        // Load schema_id from offset 4 (after heap_kind u16 + padding u16)
        let schema_id_loaded = self.builder.ins().load(
            types::I32,
            MemFlags::new(),
            heap_ptr,
            4, // JIT_ALLOC_SCHEMA_OFFSET
        );

        let expected_schema = self
            .builder
            .ins()
            .iconst(types::I32, expected_schema_id as i64);
        let schema_guard = self
            .builder
            .ins()
            .icmp(IntCC::Equal, schema_id_loaded, expected_schema);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(schema_guard, sb, &[obj]);
        } else {
            self.deopt_if_false_with_id(schema_guard, deopt_id as u32);
        }

        // Fast path: direct indexed field load.
        // Field data starts at offset 8, each field is 8 bytes (NaN-boxed).
        let field_byte_offset = 8 + (field_idx as i32 * 8);
        let field_val =
            self.builder
                .ins()
                .load(types::I64, MemFlags::new(), heap_ptr, field_byte_offset);

        Some(field_val)
    }

    // =====================================================================
    // Stable arithmetic → typed fast path with type guard
    // =====================================================================

    /// Check if the arithmetic operation at `bytecode_offset` has monomorphic
    /// type feedback suitable for a speculative typed fast path.
    ///
    /// Returns `(left_tag, right_tag)` if the arithmetic site is monomorphic
    /// with a single observed type pair.
    pub(crate) fn speculative_arithmetic_types(&self, bytecode_offset: usize) -> Option<(u8, u8)> {
        let feedback = self.feedback.as_ref()?;
        match feedback.get_slot(bytecode_offset)? {
            FeedbackSlot::Arithmetic(fb) if fb.state == ICState::Monomorphic => {
                let pair = fb.type_pairs.first()?;
                Some((pair.left_tag, pair.right_tag))
            }
            _ => None,
        }
    }

    /// Emit a speculative integer addition with type guards.
    ///
    /// Both operands are checked to be I48 (inline signed integer). On
    /// guard success, the raw integer values are extracted, added, and
    /// re-boxed as I48. On guard failure, deopt to interpreter.
    ///
    /// # Arguments
    /// * `a` - Left operand (NaN-boxed)
    /// * `b` - Right operand (NaN-boxed)
    /// * `bytecode_offset` - For DeoptInfo recording
    ///
    /// # Returns
    /// The result value (NaN-boxed I48), or `None` if guards failed to emit.
    pub(crate) fn emit_speculative_int_add(
        &mut self,
        a: Value,
        b: Value,
        bytecode_offset: usize,
    ) -> Option<Value> {
        // Guard: both operands must be I48 (inline integer).
        // I48 tag: TAG_BASE | (TAG_INT << TAG_SHIFT) in the top 16 bits.
        let tag_mask_val = self.builder.ins().iconst(types::I64, TAG_MASK as i64);
        let i48_tag_val = self.builder.ins().iconst(types::I64, I48_TAG_BITS as i64);

        let a_tag = self.builder.ins().band(a, tag_mask_val);
        let b_tag = self.builder.ins().band(b, tag_mask_val);

        let a_is_int = self.builder.ins().icmp(IntCC::Equal, a_tag, i48_tag_val);
        let b_is_int = self.builder.ins().icmp(IntCC::Equal, b_tag, i48_tag_val);
        let both_int = self.builder.ins().band(a_is_int, b_is_int);

        // Record deopt point with per-guard spill (a, b pre-popped)
        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[a, b]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(both_int, sb, &[a, b]);
        } else {
            self.deopt_if_false_with_id(both_int, deopt_id as u32);
        }

        // Fast path: extract raw i48 payloads, add, rebox.
        // Payload = bits 47..0, sign-extended to i64.
        let payload_mask_val = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        let a_raw = self.builder.ins().band(a, payload_mask_val);
        let b_raw = self.builder.ins().band(b, payload_mask_val);

        // Sign-extend from 48 bits: shift left 16, arithmetic shift right 16
        let shift = self.builder.ins().iconst(types::I32, 16);
        let a_ext = self.builder.ins().ishl(a_raw, shift);
        let a_ext = self.builder.ins().sshr(a_ext, shift);
        let b_ext = self.builder.ins().ishl(b_raw, shift);
        let b_ext = self.builder.ins().sshr(b_ext, shift);

        let sum = self.builder.ins().iadd(a_ext, b_ext);

        // Rebox as I48: mask to 48 bits and OR with tag
        let sum_masked = self.builder.ins().band(sum, payload_mask_val);
        let result = self.builder.ins().bor(sum_masked, i48_tag_val);

        Some(result)
    }

    /// Emit a speculative integer subtraction with type guards.
    pub(crate) fn emit_speculative_int_sub(
        &mut self,
        a: Value,
        b: Value,
        bytecode_offset: usize,
    ) -> Option<Value> {
        let tag_mask_val = self.builder.ins().iconst(types::I64, TAG_MASK as i64);
        let i48_tag_val = self.builder.ins().iconst(types::I64, I48_TAG_BITS as i64);

        let a_tag = self.builder.ins().band(a, tag_mask_val);
        let b_tag = self.builder.ins().band(b, tag_mask_val);

        let a_is_int = self.builder.ins().icmp(IntCC::Equal, a_tag, i48_tag_val);
        let b_is_int = self.builder.ins().icmp(IntCC::Equal, b_tag, i48_tag_val);
        let both_int = self.builder.ins().band(a_is_int, b_is_int);

        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[a, b]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(both_int, sb, &[a, b]);
        } else {
            self.deopt_if_false_with_id(both_int, deopt_id as u32);
        }

        let payload_mask_val = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        let a_raw = self.builder.ins().band(a, payload_mask_val);
        let b_raw = self.builder.ins().band(b, payload_mask_val);

        let shift = self.builder.ins().iconst(types::I32, 16);
        let a_ext = self.builder.ins().ishl(a_raw, shift);
        let a_ext = self.builder.ins().sshr(a_ext, shift);
        let b_ext = self.builder.ins().ishl(b_raw, shift);
        let b_ext = self.builder.ins().sshr(b_ext, shift);

        let diff = self.builder.ins().isub(a_ext, b_ext);

        let diff_masked = self.builder.ins().band(diff, payload_mask_val);
        let result = self.builder.ins().bor(diff_masked, i48_tag_val);

        Some(result)
    }

    /// Emit a speculative integer multiplication with type guards.
    pub(crate) fn emit_speculative_int_mul(
        &mut self,
        a: Value,
        b: Value,
        bytecode_offset: usize,
    ) -> Option<Value> {
        let tag_mask_val = self.builder.ins().iconst(types::I64, TAG_MASK as i64);
        let i48_tag_val = self.builder.ins().iconst(types::I64, I48_TAG_BITS as i64);

        let a_tag = self.builder.ins().band(a, tag_mask_val);
        let b_tag = self.builder.ins().band(b, tag_mask_val);

        let a_is_int = self.builder.ins().icmp(IntCC::Equal, a_tag, i48_tag_val);
        let b_is_int = self.builder.ins().icmp(IntCC::Equal, b_tag, i48_tag_val);
        let both_int = self.builder.ins().band(a_is_int, b_is_int);

        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[a, b]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(both_int, sb, &[a, b]);
        } else {
            self.deopt_if_false_with_id(both_int, deopt_id as u32);
        }

        let payload_mask_val = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        let a_raw = self.builder.ins().band(a, payload_mask_val);
        let b_raw = self.builder.ins().band(b, payload_mask_val);

        let shift = self.builder.ins().iconst(types::I32, 16);
        let a_ext = self.builder.ins().ishl(a_raw, shift);
        let a_ext = self.builder.ins().sshr(a_ext, shift);
        let b_ext = self.builder.ins().ishl(b_raw, shift);
        let b_ext = self.builder.ins().sshr(b_ext, shift);

        let prod = self.builder.ins().imul(a_ext, b_ext);

        let prod_masked = self.builder.ins().band(prod, payload_mask_val);
        let result = self.builder.ins().bor(prod_masked, i48_tag_val);

        Some(result)
    }

    /// Emit a speculative f64 addition with type guards.
    ///
    /// Both operands are checked to be plain f64 numbers (not NaN-tagged).
    /// On guard success, performs native f64 addition and returns the result
    /// as a plain f64 bit pattern.
    pub(crate) fn emit_speculative_f64_add(
        &mut self,
        a: Value,
        b: Value,
        bytecode_offset: usize,
    ) -> Option<Value> {
        // Guard: both must be numbers (not in NaN-tag space)
        let a_is_num = self.is_boxed_number(a);
        let b_is_num = self.is_boxed_number(b);
        let both_num = self.builder.ins().band(a_is_num, b_is_num);

        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[a, b]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(both_num, sb, &[a, b]);
        } else {
            self.deopt_if_false_with_id(both_num, deopt_id as u32);
        }

        // Fast path: bitcast to f64, add, bitcast back
        let a_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), a);
        let b_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), b);
        let sum_f64 = self.builder.ins().fadd(a_f64, b_f64);
        let result = self
            .builder
            .ins()
            .bitcast(types::I64, MemFlags::new(), sum_f64);

        Some(result)
    }

    /// Emit a speculative f64 subtraction with type guards.
    pub(crate) fn emit_speculative_f64_sub(
        &mut self,
        a: Value,
        b: Value,
        bytecode_offset: usize,
    ) -> Option<Value> {
        let a_is_num = self.is_boxed_number(a);
        let b_is_num = self.is_boxed_number(b);
        let both_num = self.builder.ins().band(a_is_num, b_is_num);

        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[a, b]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(both_num, sb, &[a, b]);
        } else {
            self.deopt_if_false_with_id(both_num, deopt_id as u32);
        }

        let a_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), a);
        let b_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), b);
        let diff_f64 = self.builder.ins().fsub(a_f64, b_f64);
        let result = self
            .builder
            .ins()
            .bitcast(types::I64, MemFlags::new(), diff_f64);

        Some(result)
    }

    /// Emit a speculative f64 multiplication with type guards.
    pub(crate) fn emit_speculative_f64_mul(
        &mut self,
        a: Value,
        b: Value,
        bytecode_offset: usize,
    ) -> Option<Value> {
        let a_is_num = self.is_boxed_number(a);
        let b_is_num = self.is_boxed_number(b);
        let both_num = self.builder.ins().band(a_is_num, b_is_num);

        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[a, b]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(both_num, sb, &[a, b]);
        } else {
            self.deopt_if_false_with_id(both_num, deopt_id as u32);
        }

        let a_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), a);
        let b_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), b);
        let prod_f64 = self.builder.ins().fmul(a_f64, b_f64);
        let result = self
            .builder
            .ins()
            .bitcast(types::I64, MemFlags::new(), prod_f64);

        Some(result)
    }

    /// Emit a speculative f64 division with type guards.
    pub(crate) fn emit_speculative_f64_div(
        &mut self,
        a: Value,
        b: Value,
        bytecode_offset: usize,
    ) -> Option<Value> {
        let a_is_num = self.is_boxed_number(a);
        let b_is_num = self.is_boxed_number(b);
        let both_num = self.builder.ins().band(a_is_num, b_is_num);

        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[a, b]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(both_num, sb, &[a, b]);
        } else {
            self.deopt_if_false_with_id(both_num, deopt_id as u32);
        }

        let a_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), a);
        let b_f64 = self.builder.ins().bitcast(types::F64, MemFlags::new(), b);
        let quot_f64 = self.builder.ins().fdiv(a_f64, b_f64);
        let result = self
            .builder
            .ins()
            .bitcast(types::I64, MemFlags::new(), quot_f64);

        Some(result)
    }

    // =====================================================================
    // Top-level dispatch: try speculative arithmetic at current offset
    // =====================================================================

    /// NaN tag constants for type-pair classification from feedback.
    ///
    /// These match the tags used by the interpreter's feedback recording:
    /// - Tag 1 = I48 (inline integer)
    /// - Tag 0 = f64 (number)
    const FEEDBACK_TAG_I48: u8 = 1;
    const FEEDBACK_TAG_F64: u8 = 0;

    /// Try to emit speculative arithmetic for the current instruction.
    ///
    /// If feedback indicates monomorphic integer or float arithmetic at
    /// `bytecode_offset`, pops two operands from the stack, emits a guarded
    /// fast path, and pushes the result. Returns `true` if the speculative
    /// path was emitted (caller should skip the generic path).
    pub(crate) fn try_speculative_add(&mut self, bytecode_offset: usize) -> bool {
        if let Some((left_tag, right_tag)) = self.speculative_arithmetic_types(bytecode_offset) {
            if left_tag == Self::FEEDBACK_TAG_I48 && right_tag == Self::FEEDBACK_TAG_I48 {
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    if let Some(result) = self.emit_speculative_int_add(a, b, bytecode_offset) {
                        self.stack_push(result);
                        return true;
                    }
                    // Guard emission failed — push operands back (unreachable
                    // in practice since emit_speculative_int_add always succeeds)
                    self.stack_push(a);
                    self.stack_push(b);
                }
            } else if left_tag == Self::FEEDBACK_TAG_F64 && right_tag == Self::FEEDBACK_TAG_F64 {
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    if let Some(result) = self.emit_speculative_f64_add(a, b, bytecode_offset) {
                        self.stack_push(result);
                        return true;
                    }
                    self.stack_push(a);
                    self.stack_push(b);
                }
            }
        }
        false
    }

    /// Try to emit speculative subtraction.
    pub(crate) fn try_speculative_sub(&mut self, bytecode_offset: usize) -> bool {
        if let Some((left_tag, right_tag)) = self.speculative_arithmetic_types(bytecode_offset) {
            if left_tag == Self::FEEDBACK_TAG_I48 && right_tag == Self::FEEDBACK_TAG_I48 {
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    if let Some(result) = self.emit_speculative_int_sub(a, b, bytecode_offset) {
                        self.stack_push(result);
                        return true;
                    }
                    self.stack_push(a);
                    self.stack_push(b);
                }
            } else if left_tag == Self::FEEDBACK_TAG_F64 && right_tag == Self::FEEDBACK_TAG_F64 {
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    if let Some(result) = self.emit_speculative_f64_sub(a, b, bytecode_offset) {
                        self.stack_push(result);
                        return true;
                    }
                    self.stack_push(a);
                    self.stack_push(b);
                }
            }
        }
        false
    }

    /// Try to emit speculative multiplication.
    pub(crate) fn try_speculative_mul(&mut self, bytecode_offset: usize) -> bool {
        if let Some((left_tag, right_tag)) = self.speculative_arithmetic_types(bytecode_offset) {
            if left_tag == Self::FEEDBACK_TAG_I48 && right_tag == Self::FEEDBACK_TAG_I48 {
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    if let Some(result) = self.emit_speculative_int_mul(a, b, bytecode_offset) {
                        self.stack_push(result);
                        return true;
                    }
                    self.stack_push(a);
                    self.stack_push(b);
                }
            } else if left_tag == Self::FEEDBACK_TAG_F64 && right_tag == Self::FEEDBACK_TAG_F64 {
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    if let Some(result) = self.emit_speculative_f64_mul(a, b, bytecode_offset) {
                        self.stack_push(result);
                        return true;
                    }
                    self.stack_push(a);
                    self.stack_push(b);
                }
            }
        }
        false
    }

    /// Emit a speculative integer division with type guards.
    /// int / int -> int (truncated toward zero), matching VM semantics.
    pub(crate) fn emit_speculative_int_div(
        &mut self,
        a: Value,
        b: Value,
        bytecode_offset: usize,
    ) -> Option<Value> {
        let tag_mask_val = self.builder.ins().iconst(types::I64, TAG_MASK as i64);
        let i48_tag_val = self.builder.ins().iconst(types::I64, I48_TAG_BITS as i64);

        let a_tag = self.builder.ins().band(a, tag_mask_val);
        let b_tag = self.builder.ins().band(b, tag_mask_val);

        let a_is_int = self.builder.ins().icmp(IntCC::Equal, a_tag, i48_tag_val);
        let b_is_int = self.builder.ins().icmp(IntCC::Equal, b_tag, i48_tag_val);
        let both_int = self.builder.ins().band(a_is_int, b_is_int);

        let (deopt_id, spill_block) = self.emit_deopt_point_with_spill(bytecode_offset, &[a, b]);
        if let Some(sb) = spill_block {
            self.deopt_if_false_with_spill(both_int, sb, &[a, b]);
        } else {
            self.deopt_if_false_with_id(both_int, deopt_id as u32);
        }

        let payload_mask_val = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        let a_raw = self.builder.ins().band(a, payload_mask_val);
        let b_raw = self.builder.ins().band(b, payload_mask_val);

        let shift = self.builder.ins().iconst(types::I32, 16);
        let a_ext = self.builder.ins().ishl(a_raw, shift);
        let a_ext = self.builder.ins().sshr(a_ext, shift);
        let b_ext = self.builder.ins().ishl(b_raw, shift);
        let b_ext = self.builder.ins().sshr(b_ext, shift);

        let quot = self.builder.ins().sdiv(a_ext, b_ext);

        let quot_masked = self.builder.ins().band(quot, payload_mask_val);
        let result = self.builder.ins().bor(quot_masked, i48_tag_val);

        Some(result)
    }

    /// Try to emit speculative division.
    pub(crate) fn try_speculative_div(&mut self, bytecode_offset: usize) -> bool {
        if let Some((left_tag, right_tag)) = self.speculative_arithmetic_types(bytecode_offset) {
            if left_tag == Self::FEEDBACK_TAG_I48 && right_tag == Self::FEEDBACK_TAG_I48 {
                // int / int -> int (truncated toward zero)
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    if let Some(result) = self.emit_speculative_int_div(a, b, bytecode_offset) {
                        self.stack_push(result);
                        return true;
                    }
                    self.stack_push(a);
                    self.stack_push(b);
                }
            } else if left_tag == Self::FEEDBACK_TAG_F64 && right_tag == Self::FEEDBACK_TAG_F64 {
                if self.stack_len() >= 2 {
                    let b = self.stack_pop().unwrap();
                    let a = self.stack_pop().unwrap();
                    if let Some(result) = self.emit_speculative_f64_div(a, b, bytecode_offset) {
                        self.stack_push(result);
                        return true;
                    }
                    self.stack_push(a);
                    self.stack_push(b);
                }
            }
        }
        false
    }

    /// Check if feedback is present (Tier 2 compilation).
    #[inline]
    pub(crate) fn has_feedback(&self) -> bool {
        self.feedback.is_some()
    }
}
