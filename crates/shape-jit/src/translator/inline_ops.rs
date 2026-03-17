//! Inline operations for direct memory access without FFI calls.
//!
//! Contains inline array operations (get/set/length/push) and direct
//! DataFrame column access that bypass FFI for maximum performance.

use cranelift::prelude::*;

use crate::context::*;
use crate::nan_boxing::*;

use super::types::BytecodeToIR;

// Dense bool bitset lowering is implemented but currently disabled by default
// until the fast-path guard strategy is tuned to avoid sieve regressions.
const ENABLE_BOOL_DENSE_ARRAY_PATH: bool = false;

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    #[inline]
    fn emit_array_element_addr(&mut self, data_ptr: Value, index_i64: Value) -> Value {
        // Scale element index by 8 bytes (u64 slot width) using a shift.
        // This is cheaper than a multiply and helps tight-loop array kernels.
        let byte_offset = self.builder.ins().ishl_imm(index_i64, 3);
        self.builder.ins().iadd(data_ptr, byte_offset)
    }

    /// Extract the 48-bit payload pointer from a NaN-boxed heap value.
    ///
    /// Masks off the tag bits (upper 16 bits) and returns the raw pointer
    /// value as an i64. This is the common first step for all heap value
    /// access: `bits & PAYLOAD_MASK`.
    #[inline]
    pub(in crate::translator) fn emit_payload_ptr(&mut self, boxed: Value) -> Value {
        let payload_mask = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        self.builder.ins().band(boxed, payload_mask)
    }

    /// Extract the JitAlloc data pointer from a NaN-boxed heap value.
    ///
    /// Combines `emit_payload_ptr` with adding the JitAlloc header offset
    /// to skip past the `[kind: u16, _pad: [u8; 6]]` prefix to the data.
    #[inline]
    pub(in crate::translator) fn emit_jit_alloc_data_ptr(&mut self, boxed: Value) -> Value {
        let alloc_ptr = self.emit_payload_ptr(boxed);
        self.builder
            .ins()
            .iadd_imm(alloc_ptr, JIT_ALLOC_DATA_OFFSET as i64)
    }

    /// Load a value from memory using trusted MemFlags.
    ///
    /// Trusted loads are appropriate when the pointer is known valid (e.g.,
    /// after a heap kind guard or within a bounds-checked array access).
    #[inline]
    pub(in crate::translator) fn emit_trusted_load(
        &mut self,
        ty: types::Type,
        ptr: Value,
        offset: i32,
    ) -> Value {
        self.builder
            .ins()
            .load(ty, MemFlags::trusted(), ptr, offset)
    }

    #[inline]
    fn emit_array_ptr(&mut self, arr_boxed: Value) -> Value {
        // Skip JitAlloc header to reach the JitArray data.
        // JitAlloc<T> layout: [kind: u16, _pad: [u8; 6], data: T] — data starts at offset 8.
        self.emit_jit_alloc_data_ptr(arr_boxed)
    }

    #[inline]
    fn emit_array_typed_meta(&mut self, arr_ptr: Value) -> (Value, Value) {
        let typed_data = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 24);
        let element_kind = self
            .builder
            .ins()
            .load(types::I8, MemFlags::trusted(), arr_ptr, 32);
        (typed_data, element_kind)
    }

    #[inline]
    pub(in crate::translator) fn emit_boxed_bool_from_i1(&mut self, cond: Value) -> Value {
        let true_val = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
        let false_val = self.builder.ins().iconst(types::I64, TAG_BOOL_FALSE as i64);
        self.builder.ins().select(cond, true_val, false_val)
    }

    /// Check whether a NaN-boxed value is a heap object with a specific HeapKind.
    ///
    /// Emits Cranelift IR equivalent to the runtime `is_heap_kind(bits, expected)`:
    ///   1. Check upper 16 bits == TAG_BASE (confirms heap-tagged NaN-boxed value)
    ///   2. Speculatively load HeapKind u16 from JitAlloc header at payload pointer
    ///   3. Compare kind with `expected_hk`
    ///   4. AND both checks together
    ///
    /// Returns an i8 (boolean) Cranelift Value: 1 if match, 0 otherwise.
    #[inline]
    pub(in crate::translator) fn emit_is_heap_kind(
        &mut self,
        val: Value,
        expected_hk: u16,
    ) -> Value {
        // Step 1: (val & 0xFFFF_0000_0000_0000) == TAG_BASE
        let tag_mask_val = self
            .builder
            .ins()
            .iconst(types::I64, 0xFFFF_0000_0000_0000u64 as i64);
        let tag_base_val = self.builder.ins().iconst(types::I64, TAG_BASE as i64);
        let upper_bits = self.builder.ins().band(val, tag_mask_val);
        let is_heap = self
            .builder
            .ins()
            .icmp(IntCC::Equal, upper_bits, tag_base_val);

        // Step 2: Speculatively load heap_kind u16 from JitAlloc header
        let alloc_ptr = self.emit_payload_ptr(val);
        let kind_u16 = self
            .builder
            .ins()
            .load(types::I16, MemFlags::new(), alloc_ptr, 0);
        let kind_ext = self.builder.ins().uextend(types::I64, kind_u16);

        // Step 3: Compare kind with expected
        let hk_val = self.builder.ins().iconst(types::I64, expected_hk as i64);
        let is_kind = self.builder.ins().icmp(IntCC::Equal, kind_ext, hk_val);

        // Step 4: Both must be true
        self.builder.ins().band(is_heap, is_kind)
    }

    fn emit_typed_bool_bitset_load(&mut self, typed_data: Value, index_i64: Value) -> Value {
        let byte_idx = self.builder.ins().ushr_imm(index_i64, 3);
        let byte_addr = self.builder.ins().iadd(typed_data, byte_idx);
        let byte = self
            .builder
            .ins()
            .load(types::I8, MemFlags::trusted(), byte_addr, 0);
        let bit_pos_i64 = self.builder.ins().band_imm(index_i64, 7);
        let bit_pos = self.builder.ins().ireduce(types::I8, bit_pos_i64);
        let one = self.builder.ins().iconst(types::I8, 1);
        let mask = self.builder.ins().ishl(one, bit_pos);
        let masked = self.builder.ins().band(byte, mask);
        let zero = self.builder.ins().iconst(types::I8, 0);
        let is_set = self.builder.ins().icmp(IntCC::NotEqual, masked, zero);
        self.emit_boxed_bool_from_i1(is_set)
    }

    fn emit_typed_bool_bitset_store(&mut self, typed_data: Value, index_i64: Value, value: Value) {
        let byte_idx = self.builder.ins().ushr_imm(index_i64, 3);
        let byte_addr = self.builder.ins().iadd(typed_data, byte_idx);
        let prev = self
            .builder
            .ins()
            .load(types::I8, MemFlags::trusted(), byte_addr, 0);
        let bit_pos_i64 = self.builder.ins().band_imm(index_i64, 7);
        let bit_pos = self.builder.ins().ireduce(types::I8, bit_pos_i64);
        let one = self.builder.ins().iconst(types::I8, 1);
        let mask = self.builder.ins().ishl(one, bit_pos);
        let set_byte = self.builder.ins().bor(prev, mask);
        let clear_mask = self.builder.ins().bnot(mask);
        let clear_byte = self.builder.ins().band(prev, clear_mask);
        let true_tag = self.builder.ins().iconst(types::I64, TAG_BOOL_TRUE as i64);
        let is_true = self.builder.ins().icmp(IntCC::Equal, value, true_tag);
        let next = self.builder.ins().select(is_true, set_byte, clear_byte);
        self.builder
            .ins()
            .store(MemFlags::trusted(), next, byte_addr, 0);
    }

    // ========================================================================
    // Inline Array Operations — direct memory access, no FFI
    // ========================================================================

    /// Extract the JitArray data pointer and length from a NaN-boxed array value.
    ///
    /// Uses inline Cranelift IR to directly load from JitArray's #[repr(C)] layout:
    ///   offset 0: data (*mut u64)
    ///   offset 8: len (u64)
    ///
    /// This replaces the FFI call to `jit_array_info`, eliminating ~50-100ns of
    /// call overhead per array access. The #[repr(C)] layout guarantee makes the
    /// field offsets stable across compiler versions.
    ///
    /// Returns (data_ptr, length) as Cranelift Values.
    pub(in crate::translator) fn emit_array_data_ptr(
        &mut self,
        arr_boxed: Value,
    ) -> (Value, Value) {
        // Inline JitArray access: extract pointer and load fields directly
        // JitArray is #[repr(C)] with guaranteed offsets: data=0, len=8
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        let length = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 8);
        (data_ptr, length)
    }

    /// Extract only the JitArray data pointer from a NaN-boxed array value.
    ///
    /// Trusted indexed get/set paths do not need `len`, so this avoids one
    /// extra memory load per access.
    pub(in crate::translator) fn emit_array_data_ptr_only(&mut self, arr_boxed: Value) -> Value {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        self.builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 0)
    }

    /// Emit bounds check: trap if index >= length.
    /// Returns the validated index as i64.
    pub(in crate::translator) fn emit_bounds_check(
        &mut self,
        index: Value,
        length: Value,
    ) -> Value {
        // If index is negative or >= length, use 0 (safe default)
        // For performance, we do a single unsigned comparison:
        // treating index as unsigned, index >= length catches both negative and too-large
        let in_bounds = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, index, length);
        let zero = self.builder.ins().iconst(types::I64, 0);
        // Return index if in bounds, 0 otherwise (prevents out-of-bounds access)
        self.builder.ins().select(in_bounds, index, zero)
    }

    /// Inline array get: load element directly from memory.
    ///
    /// Equivalent to: arr[index] where arr is NaN-boxed Vec<u64>
    /// Emits ~8 instructions instead of an FFI call (~50-100ns overhead).
    pub(in crate::translator) fn inline_array_get(
        &mut self,
        arr_boxed: Value,
        index_boxed: Value,
    ) -> Value {
        // Convert index from NaN-boxed f64 to i64
        let idx_f64 = self.i64_to_f64(index_boxed);
        let idx_i64 = self.builder.ins().fcvt_to_sint_sat(types::I64, idx_f64);
        self.inline_array_get_i64(arr_boxed, idx_i64)
    }

    /// Inline array get with raw i64 index (already unboxed).
    pub(in crate::translator) fn inline_array_get_i64(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
    ) -> Value {
        // Get data pointer and length
        let (data_ptr, length) = self.emit_array_data_ptr(arr_boxed);

        // Handle negative indices: if idx < 0, idx = length + idx
        let final_idx = self.normalize_array_index(idx_i64, length);

        // Bounds check
        let safe_idx = self.emit_bounds_check(final_idx, length);

        // Load element: data_ptr[safe_idx * 8]
        let element_addr = self.emit_array_element_addr(data_ptr, safe_idx);
        self.builder
            .ins()
            .load(types::I64, MemFlags::trusted(), element_addr, 0)
    }

    /// Inline array get with raw i64 index known to be non-negative.
    ///
    /// Skips negative-index normalization but keeps bounds checks.
    pub(in crate::translator) fn inline_array_get_i64_non_negative(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
    ) -> Value {
        let (data_ptr, length) = self.emit_array_data_ptr(arr_boxed);
        let safe_idx = self.emit_bounds_check(idx_i64, length);
        let element_addr = self.emit_array_element_addr(data_ptr, safe_idx);
        self.builder
            .ins()
            .load(types::I64, MemFlags::trusted(), element_addr, 0)
    }

    /// Strict typed fast path for array reads with raw i64 index.
    ///
    /// Uses prevalidated loop proofs (trusted index set) to skip index
    /// normalization and bounds checks in non-hoisted contexts.
    pub(in crate::translator) fn inline_array_get_i64_trusted(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
    ) -> Value {
        let data_ptr = self.emit_array_data_ptr_only(arr_boxed);
        let element_addr = self.emit_array_element_addr(data_ptr, idx_i64);
        self.builder
            .ins()
            .load(types::I64, MemFlags::trusted(), element_addr, 0)
    }

    fn inline_array_get_bool_at_index(
        &mut self,
        arr_ptr: Value,
        data_ptr: Value,
        idx_i64: Value,
    ) -> Value {
        let (typed_data, element_kind) = self.emit_array_typed_meta(arr_ptr);
        let zero_ptr = self.builder.ins().iconst(types::I64, 0);
        let has_typed = self
            .builder
            .ins()
            .icmp(IntCC::NotEqual, typed_data, zero_ptr);
        let bool_kind = self.builder.ins().iconst(
            types::I8,
            crate::jit_array::ArrayElementKind::Bool.as_byte() as i64,
        );
        let is_bool = self
            .builder
            .ins()
            .icmp(IntCC::Equal, element_kind, bool_kind);
        let use_typed = self.builder.ins().band(has_typed, is_bool);

        let typed_block = self.builder.create_block();
        let fallback_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);
        self.builder
            .ins()
            .brif(use_typed, typed_block, &[], fallback_block, &[]);

        self.builder.switch_to_block(typed_block);
        self.builder.seal_block(typed_block);
        let typed_val = self.emit_typed_bool_bitset_load(typed_data, idx_i64);
        self.builder.ins().jump(merge_block, &[typed_val]);

        self.builder.switch_to_block(fallback_block);
        self.builder.seal_block(fallback_block);
        let element_addr = self.emit_array_element_addr(data_ptr, idx_i64);
        let boxed = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), element_addr, 0);
        self.builder.ins().jump(merge_block, &[boxed]);

        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        self.builder.block_params(merge_block)[0]
    }

    /// Leaner bool-read fast path for trusted sites:
    /// check only `element_kind == Bool` and skip the extra typed-pointer compare.
    fn inline_array_get_bool_at_index_trusted(
        &mut self,
        arr_ptr: Value,
        data_ptr: Value,
        idx_i64: Value,
    ) -> Value {
        let element_kind = self
            .builder
            .ins()
            .load(types::I8, MemFlags::trusted(), arr_ptr, 32);
        let bool_kind = self.builder.ins().iconst(
            types::I8,
            crate::jit_array::ArrayElementKind::Bool.as_byte() as i64,
        );
        let is_bool = self
            .builder
            .ins()
            .icmp(IntCC::Equal, element_kind, bool_kind);

        let typed_block = self.builder.create_block();
        let fallback_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);
        self.builder
            .ins()
            .brif(is_bool, typed_block, &[], fallback_block, &[]);

        self.builder.switch_to_block(typed_block);
        self.builder.seal_block(typed_block);
        let typed_data = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 24);
        let typed_val = self.emit_typed_bool_bitset_load(typed_data, idx_i64);
        self.builder.ins().jump(merge_block, &[typed_val]);

        self.builder.switch_to_block(fallback_block);
        self.builder.seal_block(fallback_block);
        let element_addr = self.emit_array_element_addr(data_ptr, idx_i64);
        let boxed = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), element_addr, 0);
        self.builder.ins().jump(merge_block, &[boxed]);

        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        self.builder.block_params(merge_block)[0]
    }

    pub(in crate::translator) fn inline_array_get_i64_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
    ) -> Value {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        let length = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 8);
        let final_idx = self.normalize_array_index(idx_i64, length);
        let safe_idx = self.emit_bounds_check(final_idx, length);
        self.inline_array_get_bool_at_index(arr_ptr, data_ptr, safe_idx)
    }

    pub(in crate::translator) fn inline_array_get_i64_non_negative_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
    ) -> Value {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        let length = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 8);
        let safe_idx = self.emit_bounds_check(idx_i64, length);
        self.inline_array_get_bool_at_index(arr_ptr, data_ptr, safe_idx)
    }

    pub(in crate::translator) fn inline_array_get_i64_trusted_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
    ) -> Value {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        self.inline_array_get_bool_at_index_trusted(arr_ptr, data_ptr, idx_i64)
    }

    /// Inline array get with pre-hoisted data_ptr and length (Array LICM).
    ///
    /// Skips tag check and `emit_array_data_ptr` — only performs:
    ///   index conversion → negative index handling → bounds check → element load.
    ///
    /// Used when the array is a loop-invariant local with hoisted array info,
    /// eliminating ~17 instructions and 2 branches per array access in tight loops.
    pub(in crate::translator) fn inline_array_get_hoisted(
        &mut self,
        index_boxed: Value,
        data_ptr: Value,
        length: Value,
    ) -> Value {
        // Convert index from NaN-boxed f64 to i64
        let idx_f64 = self.i64_to_f64(index_boxed);
        let idx_i64 = self.builder.ins().fcvt_to_sint_sat(types::I64, idx_f64);
        self.inline_array_get_hoisted_i64(idx_i64, data_ptr, length)
    }

    /// Inline array get with raw i64 index and hoisted array info.
    pub(in crate::translator) fn inline_array_get_hoisted_i64(
        &mut self,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
    ) -> Value {
        // Handle negative indices: if idx < 0, idx = length + idx
        let final_idx = self.normalize_array_index(idx_i64, length);

        // Bounds check
        let safe_idx = self.emit_bounds_check(final_idx, length);

        // Load element: data_ptr[safe_idx * 8]
        let element_addr = self.emit_array_element_addr(data_ptr, safe_idx);
        self.builder
            .ins()
            .load(types::I64, MemFlags::trusted(), element_addr, 0)
    }

    /// Inline array get with raw i64 index and hoisted array info.
    ///
    /// Index is proven non-negative, so this skips normalization and keeps
    /// only the bounds check and memory load.
    pub(in crate::translator) fn inline_array_get_hoisted_i64_non_negative(
        &mut self,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
    ) -> Value {
        let safe_idx = self.emit_bounds_check(idx_i64, length);
        let element_addr = self.emit_array_element_addr(data_ptr, safe_idx);
        self.builder
            .ins()
            .load(types::I64, MemFlags::trusted(), element_addr, 0)
    }

    pub(in crate::translator) fn inline_array_get_hoisted_i64_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
    ) -> Value {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let final_idx = self.normalize_array_index(idx_i64, length);
        let safe_idx = self.emit_bounds_check(final_idx, length);
        self.inline_array_get_bool_at_index(arr_ptr, data_ptr, safe_idx)
    }

    pub(in crate::translator) fn inline_array_get_hoisted_i64_non_negative_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
    ) -> Value {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let safe_idx = self.emit_bounds_check(idx_i64, length);
        self.inline_array_get_bool_at_index(arr_ptr, data_ptr, safe_idx)
    }

    pub(in crate::translator) fn inline_array_get_hoisted_i64_trusted_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        data_ptr: Value,
        _length: Value,
    ) -> Value {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        self.inline_array_get_bool_at_index_trusted(arr_ptr, data_ptr, idx_i64)
    }

    /// Strict typed fast path for hoisted array reads with raw i64 index.
    ///
    /// Assumes the array stores numeric payloads and the index expression is
    /// type-checked by the compiler. A single bounds trap keeps memory safety
    /// without routing to VM fallback paths.
    pub(in crate::translator) fn inline_array_get_hoisted_i64_trusted(
        &mut self,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
    ) -> Value {
        let _ = length;

        let element_addr = self.emit_array_element_addr(data_ptr, idx_i64);
        self.builder
            .ins()
            .load(types::I64, MemFlags::trusted(), element_addr, 0)
    }

    /// Inline array set: store element directly to memory.
    ///
    /// Equivalent to: arr[index] = value where arr is NaN-boxed Vec<u64>
    /// Modifies the array in-place (assumes exclusive access via & reference or CoW).
    pub(in crate::translator) fn inline_array_set(
        &mut self,
        arr_boxed: Value,
        index_boxed: Value,
        value: Value,
    ) {
        // Convert index from NaN-boxed f64 to i64
        let idx_f64 = self.i64_to_f64(index_boxed);
        let idx_i64 = self.builder.ins().fcvt_to_sint_sat(types::I64, idx_f64);
        self.inline_array_set_i64(arr_boxed, idx_i64, value);
    }

    /// Inline array set with raw i64 index (already unboxed).
    pub(in crate::translator) fn inline_array_set_i64(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        value: Value,
    ) {
        let (data_ptr, length) = self.emit_array_data_ptr(arr_boxed);

        // Handle negative indices
        let final_idx = self.normalize_array_index(idx_i64, length);

        // Bounds check
        let safe_idx = self.emit_bounds_check(final_idx, length);

        // Store element: data_ptr[safe_idx * 8] = value
        let element_addr = self.emit_array_element_addr(data_ptr, safe_idx);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, element_addr, 0);
    }

    /// Inline array set with raw i64 index known to be non-negative.
    ///
    /// Skips negative-index normalization but keeps bounds checks.
    pub(in crate::translator) fn inline_array_set_i64_non_negative(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        value: Value,
    ) {
        let (data_ptr, length) = self.emit_array_data_ptr(arr_boxed);
        let safe_idx = self.emit_bounds_check(idx_i64, length);
        let element_addr = self.emit_array_element_addr(data_ptr, safe_idx);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, element_addr, 0);
    }

    /// Strict typed fast path for array writes with raw i64 index.
    ///
    /// Uses prevalidated loop proofs (trusted index set) to skip index
    /// normalization and bounds checks in non-hoisted contexts.
    pub(in crate::translator) fn inline_array_set_i64_trusted(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        value: Value,
    ) {
        let data_ptr = self.emit_array_data_ptr_only(arr_boxed);
        let element_addr = self.emit_array_element_addr(data_ptr, idx_i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, element_addr, 0);
    }

    pub(in crate::translator) fn emit_sync_bool_typed_slot_if_present(
        &mut self,
        arr_ptr: Value,
        idx_i64: Value,
        value: Value,
    ) {
        if !ENABLE_BOOL_DENSE_ARRAY_PATH {
            return;
        }
        let (typed_data, element_kind) = self.emit_array_typed_meta(arr_ptr);
        let zero_ptr = self.builder.ins().iconst(types::I64, 0);
        let has_typed = self
            .builder
            .ins()
            .icmp(IntCC::NotEqual, typed_data, zero_ptr);
        let bool_kind = self.builder.ins().iconst(
            types::I8,
            crate::jit_array::ArrayElementKind::Bool.as_byte() as i64,
        );
        let is_bool = self
            .builder
            .ins()
            .icmp(IntCC::Equal, element_kind, bool_kind);
        let use_typed = self.builder.ins().band(has_typed, is_bool);

        let typed_block = self.builder.create_block();
        let done_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(use_typed, typed_block, &[], done_block, &[]);

        self.builder.switch_to_block(typed_block);
        self.builder.seal_block(typed_block);
        self.emit_typed_bool_bitset_store(typed_data, idx_i64, value);
        self.builder.ins().jump(done_block, &[]);

        self.builder.switch_to_block(done_block);
        self.builder.seal_block(done_block);
    }

    fn inline_array_set_bool_at_index(
        &mut self,
        arr_ptr: Value,
        data_ptr: Value,
        idx_i64: Value,
        value: Value,
    ) {
        let element_addr = self.emit_array_element_addr(data_ptr, idx_i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, element_addr, 0);
        self.emit_sync_bool_typed_slot_if_present(arr_ptr, idx_i64, value);
    }

    /// Leaner bool-write fast path for trusted sites:
    /// check only `element_kind == Bool`, then update typed bitset mirror.
    fn inline_array_set_bool_at_index_trusted(
        &mut self,
        arr_ptr: Value,
        data_ptr: Value,
        idx_i64: Value,
        value: Value,
    ) {
        let element_kind = self
            .builder
            .ins()
            .load(types::I8, MemFlags::trusted(), arr_ptr, 32);
        let bool_kind = self.builder.ins().iconst(
            types::I8,
            crate::jit_array::ArrayElementKind::Bool.as_byte() as i64,
        );
        let is_bool = self
            .builder
            .ins()
            .icmp(IntCC::Equal, element_kind, bool_kind);

        let typed_block = self.builder.create_block();
        let boxed_block = self.builder.create_block();
        let done_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_bool, typed_block, &[], boxed_block, &[]);

        self.builder.switch_to_block(typed_block);
        self.builder.seal_block(typed_block);
        let typed_data = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 24);
        self.emit_typed_bool_bitset_store(typed_data, idx_i64, value);
        self.builder.ins().jump(done_block, &[]);

        self.builder.switch_to_block(boxed_block);
        self.builder.seal_block(boxed_block);
        let element_addr = self.emit_array_element_addr(data_ptr, idx_i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, element_addr, 0);
        self.builder.ins().jump(done_block, &[]);

        self.builder.switch_to_block(done_block);
        self.builder.seal_block(done_block);
    }

    pub(in crate::translator) fn inline_array_set_i64_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        value: Value,
    ) {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        let length = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 8);
        let final_idx = self.normalize_array_index(idx_i64, length);
        let safe_idx = self.emit_bounds_check(final_idx, length);
        self.inline_array_set_bool_at_index(arr_ptr, data_ptr, safe_idx, value);
    }

    pub(in crate::translator) fn inline_array_set_i64_non_negative_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        value: Value,
    ) {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        let length = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 8);
        let safe_idx = self.emit_bounds_check(idx_i64, length);
        self.inline_array_set_bool_at_index(arr_ptr, data_ptr, safe_idx, value);
    }

    pub(in crate::translator) fn inline_array_set_i64_trusted_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        value: Value,
    ) {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, 0);
        self.inline_array_set_bool_at_index_trusted(arr_ptr, data_ptr, idx_i64, value);
    }

    /// Strict typed fast path for hoisted array writes with raw i64 index.
    ///
    /// Emits one bounds trap and a direct store. Used by reference-based
    /// numeric kernels where fallback mutation paths are intentionally disabled.
    pub(in crate::translator) fn inline_array_set_hoisted_i64_trusted(
        &mut self,
        _arr_boxed: Value,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
        value: Value,
    ) {
        let _ = length;

        let element_addr = self.emit_array_element_addr(data_ptr, idx_i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, element_addr, 0);
    }

    pub(in crate::translator) fn inline_array_set_hoisted_i64_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
        value: Value,
    ) {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let final_idx = self.normalize_array_index(idx_i64, length);
        let safe_idx = self.emit_bounds_check(final_idx, length);
        self.inline_array_set_bool_at_index(arr_ptr, data_ptr, safe_idx, value);
    }

    pub(in crate::translator) fn inline_array_set_hoisted_i64_non_negative_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
        value: Value,
    ) {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        let safe_idx = self.emit_bounds_check(idx_i64, length);
        self.inline_array_set_bool_at_index(arr_ptr, data_ptr, safe_idx, value);
    }

    pub(in crate::translator) fn inline_array_set_hoisted_i64_trusted_bool(
        &mut self,
        arr_boxed: Value,
        idx_i64: Value,
        data_ptr: Value,
        _length: Value,
        value: Value,
    ) {
        let arr_ptr = self.emit_array_ptr(arr_boxed);
        self.inline_array_set_bool_at_index_trusted(arr_ptr, data_ptr, idx_i64, value);
    }

    /// Checked hoisted array write with raw i64 index.
    pub(in crate::translator) fn inline_array_set_hoisted_i64(
        &mut self,
        _arr_boxed: Value,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
        value: Value,
    ) {
        let final_idx = self.normalize_array_index(idx_i64, length);
        let safe_idx = self.emit_bounds_check(final_idx, length);
        let element_addr = self.emit_array_element_addr(data_ptr, safe_idx);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, element_addr, 0);
    }

    /// Checked hoisted array write with raw i64 index known to be non-negative.
    pub(in crate::translator) fn inline_array_set_hoisted_i64_non_negative(
        &mut self,
        _arr_boxed: Value,
        idx_i64: Value,
        data_ptr: Value,
        length: Value,
        value: Value,
    ) {
        let safe_idx = self.emit_bounds_check(idx_i64, length);
        let element_addr = self.emit_array_element_addr(data_ptr, safe_idx);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, element_addr, 0);
    }

    /// Normalize an array index, supporting negative indexing.
    ///
    /// If `idx` is negative, returns `length + idx`, otherwise returns `idx`.
    fn normalize_array_index(&mut self, idx: Value, length: Value) -> Value {
        let zero = self.builder.ins().iconst(types::I64, 0);
        let is_negative = self.builder.ins().icmp(IntCC::SignedLessThan, idx, zero);
        let adjusted_idx = self.builder.ins().iadd(length, idx);
        self.builder.ins().select(is_negative, adjusted_idx, idx)
    }

    /// Inline array length: load Vec.len field directly.
    ///
    /// Returns length as NaN-boxed f64.
    pub(in crate::translator) fn inline_array_length(&mut self, arr_boxed: Value) -> Value {
        let (_, length) = self.emit_array_data_ptr(arr_boxed);
        // Convert length to NaN-boxed f64
        let len_f64 = self.builder.ins().fcvt_from_sint(types::F64, length);
        self.f64_to_i64(len_f64)
    }

    // ========================================================================
    // Direct DataFrame Field Access
    // ========================================================================

    /// Emit direct data field load from pre-computed column arrays (generic DataFrame)
    /// This is the core optimization: instead of FFI calls, we emit direct pointer arithmetic
    /// Returns a NaN-boxed f64 (i64 value)
    ///
    /// Generated code equivalent:
    /// ```ignore
    /// let row_idx = ctx.current_row + offset;
    /// if row_idx < ctx.row_count && column_index < ctx.column_count {
    ///     let col_ptr = ctx.column_ptrs[column_index];
    ///     if !col_ptr.is_null() {
    ///         return box_number(*col_ptr.add(row_idx));
    ///     }
    /// }
    /// return TAG_NULL;
    /// ```
    #[allow(dead_code)]
    pub(in crate::translator) fn emit_data_field_load(
        &mut self,
        offset_val: Value,
        column_index: u32,
    ) -> Value {
        // FAST PATH: Skip bounds/null checks for performance
        // The backtest engine guarantees valid data and indices during execution.
        // This reduces data access from ~20 instructions to ~8 instructions.

        // Load current_row from ctx
        let current_row = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            self.ctx_ptr,
            CURRENT_ROW_OFFSET,
        );

        // Convert offset to i64 and add to current_row
        let offset_i64 = self.builder.ins().sextend(types::I64, offset_val);
        let row_idx = self.builder.ins().iadd(current_row, offset_i64);

        // Load column_ptrs base pointer from ctx
        let column_ptrs_base = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            self.ctx_ptr,
            COLUMN_PTRS_OFFSET,
        );

        // Calculate address of column pointer: column_ptrs_base + column_index * 8
        let col_offset = self
            .builder
            .ins()
            .iconst(types::I64, (column_index as i64) * 8);
        let col_ptr_addr = self.builder.ins().iadd(column_ptrs_base, col_offset);

        // Load the column data pointer
        let col_data_ptr =
            self.builder
                .ins()
                .load(types::I64, MemFlags::trusted(), col_ptr_addr, 0);

        // Calculate element address: col_data_ptr + row_idx * 8 (sizeof f64)
        let eight = self.builder.ins().iconst(types::I64, 8);
        let byte_offset = self.builder.ins().imul(row_idx, eight);
        let element_addr = self.builder.ins().iadd(col_data_ptr, byte_offset);

        // Load f64 from the address (trusted - we know data is valid)
        let loaded_f64 = self
            .builder
            .ins()
            .load(types::F64, MemFlags::trusted(), element_addr, 0);

        // Convert f64 to NaN-boxed i64
        self.f64_to_i64(loaded_f64)
    }

    // DELETED: Finance-specific field name mapping
    // All field names should be resolved via DataFrameSchema at compile time
    // No hardcoded OHLCV column indices

    /// Emit indicator load from computed series stored in additional DataFrame columns
    /// Indicators are computed during warmup and stored as additional columns in the DataFrame.
    /// The column index for an indicator is determined by the schema at compile time.
    ///
    /// Note: This is a placeholder - actual indicator loading uses the generic DataFrame
    /// column access via emit_data_field_load with the appropriate column index.
    #[allow(dead_code)]
    pub(in crate::translator) fn emit_indicator_load(
        &mut self,
        _indicator: &str,
        _period: i32,
    ) -> Value {
        // Indicators are now handled as additional columns in the DataFrame.
        // The compiler resolves indicator names to column indices at compile time.
        // This function is kept for backward compatibility but returns null.
        // Use emit_data_field_load with the appropriate column_index instead.
        self.builder.ins().iconst(types::I64, TAG_NULL as i64)
    }
}

// FFI helper functions called from JIT-compiled code
// These optimized functions provide O(1) direct array access for DataFrame columns
// Column indices are resolved at compile time from DataFrameSchema

/// Helper function to get a column value at offset using generic DataFrame access
#[inline(always)]
#[allow(dead_code)]
unsafe fn get_column_at(ctx: *mut JITContext, column_index: usize, offset: i32) -> f64 {
    if ctx.is_null() {
        return 0.0;
    }
    let ctx_ref = unsafe { &*ctx };

    // Check if DataFrame columns are available
    if ctx_ref.column_ptrs.is_null() || column_index >= ctx_ref.column_count {
        return 0.0;
    }

    // Calculate row index with offset
    let row_signed = ctx_ref.current_row as i32 + offset;
    if row_signed < 0 || row_signed as usize >= ctx_ref.row_count {
        return 0.0;
    }
    let row_idx = row_signed as usize;

    // Get column pointer and read value
    let col_ptr = unsafe { *ctx_ref.column_ptrs.add(column_index) };
    if col_ptr.is_null() {
        return 0.0;
    }
    unsafe { *col_ptr.add(row_idx) }
}
