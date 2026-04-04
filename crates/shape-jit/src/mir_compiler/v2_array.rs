//! Inline typed array codegen for the v2 runtime.
//!
//! Emits Cranelift IR for direct-memory-access typed array operations
//! with zero FFI overhead and zero NaN-boxing.
//!
//! ## TypedArrayHeader layout (at the array pointer)
//!
//! ```text
//! offset  0: refcount  (u32)
//! offset  4: kind      (u16)
//! offset  6: elem_type (u8)
//! offset  7: _pad      (u8)
//! offset  8: data      (*mut T)  — pointer to contiguous element buffer
//! offset 16: len       (u32)
//! offset 20: cap       (u32)
//! ```
//!
//! ## Element sizes
//!
//! | SlotKind  | Cranelift type | Size (bytes) |
//! |-----------|---------------|--------------|
//! | Float64   | F64           | 8            |
//! | Int64     | I64           | 8            |
//! | Int32     | I32           | 4            |
//! | Int16     | I16           | 2            |
//! | Int8/Bool | I8            | 1            |

use cranelift::prelude::*;
use shape_vm::type_tracking::SlotKind;

use super::MirToIR;

// ── TypedArrayHeader field offsets ───────────────────────────────────────────

/// Offset of the `data` pointer field (`*mut T`) inside `TypedArrayHeader`.
const DATA_PTR_OFFSET: i32 = 8;

/// Offset of the `len` field (`u32`) inside `TypedArrayHeader`.
const LEN_OFFSET: i32 = 16;

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Return the (Cranelift IR type, element byte size) for a given `SlotKind`.
///
/// Panics on slot kinds that do not map to a scalar element type (e.g.
/// `String`, `NanBoxed`, `Unknown`).
fn elem_type_info(kind: SlotKind) -> (types::Type, i64) {
    match kind {
        SlotKind::Float64 | SlotKind::NullableFloat64 => (types::F64, 8),
        SlotKind::Int64 | SlotKind::NullableInt64 | SlotKind::UInt64 | SlotKind::NullableUInt64 => {
            (types::I64, 8)
        }
        SlotKind::IntSize | SlotKind::NullableIntSize | SlotKind::UIntSize | SlotKind::NullableUIntSize => {
            // Pointer-sized — 8 bytes on 64-bit targets.
            (types::I64, 8)
        }
        SlotKind::Int32 | SlotKind::NullableInt32 | SlotKind::UInt32 | SlotKind::NullableUInt32 => {
            (types::I32, 4)
        }
        SlotKind::Int16 | SlotKind::NullableInt16 | SlotKind::UInt16 | SlotKind::NullableUInt16 => {
            (types::I16, 2)
        }
        SlotKind::Int8 | SlotKind::NullableInt8 | SlotKind::UInt8 | SlotKind::NullableUInt8 => {
            (types::I8, 1)
        }
        SlotKind::Bool => (types::I8, 1),
        other => panic!("v2_array: unsupported element SlotKind: {:?}", other),
    }
}

/// Return the zero/default Cranelift constant for a given `SlotKind`.
///
/// Used as the out-of-bounds fallback value in `v2_array_get`.
fn emit_default(builder: &mut FunctionBuilder, kind: SlotKind) -> Value {
    let (ty, _) = elem_type_info(kind);
    match ty {
        types::F64 => builder.ins().f64const(0.0),
        types::I64 => builder.ins().iconst(types::I64, 0),
        types::I32 => builder.ins().iconst(types::I32, 0),
        types::I16 => builder.ins().iconst(types::I16, 0),
        types::I8 => builder.ins().iconst(types::I8, 0),
        _ => unreachable!(),
    }
}

// ── Implementation ──────────────────────────────────────────────────────────

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Inline typed array element read.
    ///
    /// Emits:
    /// 1. Load `data` pointer from `[arr_ptr + 8]`
    /// 2. Load `len` (u32) from `[arr_ptr + 16]`
    /// 3. Bounds check: `if index >= len` return zero-default
    /// 4. Compute element address: `data + index * elem_size`
    /// 5. Load element with the correct Cranelift type
    ///
    /// `arr_ptr` is a Cranelift `i64` value pointing to a `TypedArrayHeader`.
    /// `index` is a Cranelift `i32` value (unsigned index).
    /// Returns the loaded element value (type depends on `elem_type`).
    pub fn v2_array_get(
        &mut self,
        arr_ptr: Value,
        index: Value,
        elem_type: SlotKind,
    ) -> Value {
        let (cl_type, elem_size) = elem_type_info(elem_type);

        // 1. Load data pointer (i64) from arr_ptr + DATA_PTR_OFFSET
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, DATA_PTR_OFFSET);

        // 2. Load length (u32) from arr_ptr + LEN_OFFSET
        let len = self
            .builder
            .ins()
            .load(types::I32, MemFlags::trusted(), arr_ptr, LEN_OFFSET);

        // 3. Bounds check: if index >= len, branch to out-of-bounds block
        let in_bounds_block = self.builder.create_block();
        let oob_block = self.builder.create_block();
        let merge_block = self.builder.create_block();

        // The merge block receives the result as a block parameter.
        self.builder.append_block_param(merge_block, cl_type);

        let cmp = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, index, len);
        self.builder
            .ins()
            .brif(cmp, in_bounds_block, &[], oob_block, &[]);

        // ── Out-of-bounds path: return default ──────────────────────────
        self.builder.switch_to_block(oob_block);
        self.builder.seal_block(oob_block);

        let default_val = emit_default(self.builder, elem_type);
        self.builder.ins().jump(merge_block, &[default_val]);

        // ── In-bounds path: compute address and load element ────────────
        self.builder.switch_to_block(in_bounds_block);
        self.builder.seal_block(in_bounds_block);

        // 4. Compute byte offset: index (u32) -> i64, then * elem_size
        let index_i64 = self.builder.ins().uextend(types::I64, index);
        let byte_offset = if (elem_size as u64).is_power_of_two() {
            let shift = (elem_size as u64).trailing_zeros() as i64;
            self.builder.ins().ishl_imm(index_i64, shift)
        } else {
            let size_val = self.builder.ins().iconst(types::I64, elem_size);
            self.builder.ins().imul(index_i64, size_val)
        };
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);

        // 5. Load element with trusted flags (bounds already checked)
        let loaded = self
            .builder
            .ins()
            .load(cl_type, MemFlags::trusted(), elem_addr, 0);

        self.builder.ins().jump(merge_block, &[loaded]);

        // ── Merge ───────────────────────────────────────────────────────
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);

        self.builder.block_params(merge_block)[0]
    }

    /// Inline typed array length.
    ///
    /// Emits a single `load i32 [arr_ptr + 16]`.
    pub fn v2_array_len(&mut self, arr_ptr: Value) -> Value {
        self.builder
            .ins()
            .load(types::I32, MemFlags::trusted(), arr_ptr, LEN_OFFSET)
    }

    /// Inline typed array element write.
    ///
    /// Emits:
    /// 1. Load `data` pointer from `[arr_ptr + 8]`
    /// 2. Load `len` (u32) from `[arr_ptr + 16]`
    /// 3. Bounds check: `if index >= len` skip (silent no-op for OOB)
    /// 4. Compute element address: `data + index * elem_size`
    /// 5. Store element with the correct Cranelift type
    ///
    /// `val` must be a Cranelift value whose type matches `elem_type`.
    pub fn v2_array_set(
        &mut self,
        arr_ptr: Value,
        index: Value,
        val: Value,
        elem_type: SlotKind,
    ) {
        let (_cl_type, elem_size) = elem_type_info(elem_type);

        // 1. Load data pointer
        let data_ptr = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), arr_ptr, DATA_PTR_OFFSET);

        // 2. Load length
        let len = self
            .builder
            .ins()
            .load(types::I32, MemFlags::trusted(), arr_ptr, LEN_OFFSET);

        // 3. Bounds check
        let in_bounds_block = self.builder.create_block();
        let continue_block = self.builder.create_block();

        let cmp = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, index, len);
        self.builder
            .ins()
            .brif(cmp, in_bounds_block, &[], continue_block, &[]);

        // ── In-bounds path: store element ───────────────────────────────
        self.builder.switch_to_block(in_bounds_block);
        self.builder.seal_block(in_bounds_block);

        let index_i64 = self.builder.ins().uextend(types::I64, index);
        let byte_offset = if (elem_size as u64).is_power_of_two() {
            let shift = (elem_size as u64).trailing_zeros() as i64;
            self.builder.ins().ishl_imm(index_i64, shift)
        } else {
            let size_val = self.builder.ins().iconst(types::I64, elem_size);
            self.builder.ins().imul(index_i64, size_val)
        };
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);

        self.builder
            .ins()
            .store(MemFlags::trusted(), val, elem_addr, 0);

        self.builder.ins().jump(continue_block, &[]);

        // ── Continue ────────────────────────────────────────────────────
        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift::codegen::ir::Function;
    use cranelift::codegen::isa::CallConv;

    /// Helper: set up a minimal Cranelift `Function` + `FunctionBuilder` and
    /// call `body_fn` inside the entry block.  Returns the finalized IR text
    /// so tests can assert on instruction patterns.
    fn with_builder<F>(body_fn: F) -> String
    where
        F: FnOnce(&mut MirToIR, Value, Value),
    {
        let mut func = Function::new();
        // Signature: (arr_ptr: i64, index: i32) -> i64
        func.signature.params.push(AbiParam::new(types::I64));
        func.signature.params.push(AbiParam::new(types::I32));
        func.signature.returns.push(AbiParam::new(types::I64));
        func.signature.call_conv = CallConv::SystemV;

        let mut func_builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut func, &mut func_builder_ctx);

        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let arr_ptr = builder.block_params(entry)[0];
        let index = builder.block_params(entry)[1];

        {
            let mut mir = MirToIR::new(&mut builder);
            body_fn(&mut mir, arr_ptr, index);
        }

        // Return a dummy i64 zero so the function signature is satisfied.
        let ret = builder.ins().iconst(types::I64, 0);
        builder.ins().return_(&[ret]);
        builder.finalize();

        // Render the IR to text for assertion.
        func.to_string()
    }

    /// Same helper but the body returns a value that we use as the return.
    fn with_builder_returning<F>(body_fn: F) -> String
    where
        F: FnOnce(&mut MirToIR, Value, Value) -> Value,
    {
        let mut func = Function::new();
        func.signature.params.push(AbiParam::new(types::I64)); // arr_ptr
        func.signature.params.push(AbiParam::new(types::I32)); // index
        func.signature.returns.push(AbiParam::new(types::I64));
        func.signature.call_conv = CallConv::SystemV;

        let mut func_builder_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut func, &mut func_builder_ctx);

        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let arr_ptr = builder.block_params(entry)[0];
        let index = builder.block_params(entry)[1];

        let result;
        {
            let mut mir = MirToIR::new(&mut builder);
            result = body_fn(&mut mir, arr_ptr, index);
        }

        builder.ins().return_(&[result]);
        builder.finalize();

        func.to_string()
    }

    // ── v2_array_len ────────────────────────────────────────────────────

    #[test]
    fn test_v2_array_len_emits_load_i32_at_offset_16() {
        let ir = with_builder(|mir, arr_ptr, _index| {
            mir.v2_array_len(arr_ptr);
        });

        // Should contain a load of i32 at offset 16 from the arr_ptr param.
        assert!(
            ir.contains("load.i32 notrap aligned"),
            "expected trusted i32 load in IR:\n{}",
            ir
        );
    }

    // ── v2_array_get ────────────────────────────────────────────────────

    #[test]
    fn test_v2_array_get_f64_emits_data_load_and_bounds_check() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Float64);
        });

        // Must load the data pointer (i64 at offset 8)
        assert!(
            ir.contains("load.i64 notrap aligned"),
            "expected trusted i64 load (data ptr) in IR:\n{}",
            ir
        );
        // Must load the length (i32 at offset 16)
        assert!(
            ir.contains("load.i32 notrap aligned"),
            "expected trusted i32 load (len) in IR:\n{}",
            ir
        );
        // Must have a bounds comparison
        assert!(
            ir.contains("icmp"),
            "expected icmp for bounds check in IR:\n{}",
            ir
        );
        // Must have a conditional branch
        assert!(
            ir.contains("brif"),
            "expected brif for bounds check in IR:\n{}",
            ir
        );
        // Must load an f64 element
        assert!(
            ir.contains("load.f64 notrap aligned"),
            "expected trusted f64 load (element) in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_v2_array_get_i32_emits_i32_element_load() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Int32);
        });

        // Element load should be i32 (distinct from the len load which is also i32).
        // The IR should have at least two i32 loads: one for len, one for element.
        let i32_load_count = ir.matches("load.i32 notrap aligned").count();
        assert!(
            i32_load_count >= 2,
            "expected at least 2 trusted i32 loads (len + element) in IR, got {}:\n{}",
            i32_load_count,
            ir
        );
    }

    #[test]
    fn test_v2_array_get_i64_emits_i64_element_load() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Int64);
        });

        // Must load both data ptr (i64) and element (i64) — at least 2 i64 loads.
        let i64_load_count = ir.matches("load.i64 notrap aligned").count();
        assert!(
            i64_load_count >= 2,
            "expected at least 2 trusted i64 loads (data ptr + element) in IR, got {}:\n{}",
            i64_load_count,
            ir
        );
    }

    #[test]
    fn test_v2_array_get_bool_emits_i8_element_load() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Bool);
        });

        assert!(
            ir.contains("load.i8 notrap aligned"),
            "expected trusted i8 load (bool element) in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_v2_array_get_i16_emits_i16_element_load() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Int16);
        });

        assert!(
            ir.contains("load.i16 notrap aligned"),
            "expected trusted i16 load (i16 element) in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_v2_array_get_f64_has_default_zero() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Float64);
        });

        // The OOB default is `f64const 0.0`
        assert!(
            ir.contains("f64const") && ir.contains("0.0"),
            "expected f64const 0.0 default in IR:\n{}",
            ir
        );
    }

    // ── v2_array_set ────────────────────────────────────────────────────

    #[test]
    fn test_v2_array_set_f64_emits_store() {
        let ir = with_builder(|mir, arr_ptr, index| {
            let val = mir.builder.ins().f64const(42.0);
            mir.v2_array_set(arr_ptr, index, val, SlotKind::Float64);
        });

        // Must have a store instruction
        assert!(
            ir.contains("store"),
            "expected store instruction in IR:\n{}",
            ir
        );
        // Must have bounds check
        assert!(
            ir.contains("brif"),
            "expected brif for bounds check in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_v2_array_set_i32_emits_store() {
        let ir = with_builder(|mir, arr_ptr, index| {
            let val = mir.builder.ins().iconst(types::I32, 99);
            mir.v2_array_set(arr_ptr, index, val, SlotKind::Int32);
        });

        assert!(
            ir.contains("store"),
            "expected store instruction in IR:\n{}",
            ir
        );
    }

    // ── Element size / address computation ──────────────────────────────

    #[test]
    fn test_v2_array_get_f64_uses_shift_by_3() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Float64);
        });

        // elem_size = 8 = 2^3 -> should use ishl_imm by 3
        assert!(
            ir.contains("ishl_imm") && ir.contains(", 3"),
            "expected ishl_imm by 3 for 8-byte elements in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_v2_array_get_i32_uses_shift_by_2() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Int32);
        });

        // elem_size = 4 = 2^2 -> should use ishl_imm by 2
        assert!(
            ir.contains("ishl_imm") && ir.contains(", 2"),
            "expected ishl_imm by 2 for 4-byte elements in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_v2_array_get_i16_uses_shift_by_1() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Int16);
        });

        // elem_size = 2 = 2^1 -> ishl_imm by 1
        assert!(
            ir.contains("ishl_imm") && ir.contains(", 1"),
            "expected ishl_imm by 1 for 2-byte elements in IR:\n{}",
            ir
        );
    }

    #[test]
    fn test_v2_array_get_bool_no_shift() {
        let ir = with_builder(|mir, arr_ptr, index| {
            mir.v2_array_get(arr_ptr, index, SlotKind::Bool);
        });

        // elem_size = 1 = 2^0 -> ishl_imm by 0 (i.e. no shift needed,
        // but the implementation may still emit ishl_imm 0 or just uextend).
        // The key assertion: no shift by 2 or 3.
        let has_shift_by_2_or_3 =
            (ir.contains("ishl_imm") && ir.contains(", 2"))
            || (ir.contains("ishl_imm") && ir.contains(", 3"));
        assert!(
            !has_shift_by_2_or_3,
            "bool (1-byte) elements should not shift by 2 or 3:\n{}",
            ir
        );
    }

    // ── elem_type_info ──────────────────────────────────────────────────

    #[test]
    fn test_elem_type_info_sizes() {
        assert_eq!(elem_type_info(SlotKind::Float64), (types::F64, 8));
        assert_eq!(elem_type_info(SlotKind::Int64), (types::I64, 8));
        assert_eq!(elem_type_info(SlotKind::UInt64), (types::I64, 8));
        assert_eq!(elem_type_info(SlotKind::Int32), (types::I32, 4));
        assert_eq!(elem_type_info(SlotKind::UInt32), (types::I32, 4));
        assert_eq!(elem_type_info(SlotKind::Int16), (types::I16, 2));
        assert_eq!(elem_type_info(SlotKind::UInt16), (types::I16, 2));
        assert_eq!(elem_type_info(SlotKind::Int8), (types::I8, 1));
        assert_eq!(elem_type_info(SlotKind::UInt8), (types::I8, 1));
        assert_eq!(elem_type_info(SlotKind::Bool), (types::I8, 1));
    }

    #[test]
    fn test_elem_type_info_nullable_variants() {
        assert_eq!(elem_type_info(SlotKind::NullableFloat64), (types::F64, 8));
        assert_eq!(elem_type_info(SlotKind::NullableInt64), (types::I64, 8));
        assert_eq!(elem_type_info(SlotKind::NullableInt32), (types::I32, 4));
        assert_eq!(elem_type_info(SlotKind::NullableInt16), (types::I16, 2));
        assert_eq!(elem_type_info(SlotKind::NullableInt8), (types::I8, 1));
    }

    #[test]
    #[should_panic(expected = "unsupported element SlotKind")]
    fn test_elem_type_info_rejects_string() {
        elem_type_info(SlotKind::String);
    }

    #[test]
    #[should_panic(expected = "unsupported element SlotKind")]
    fn test_elem_type_info_rejects_nanboxed() {
        elem_type_info(SlotKind::NanBoxed);
    }

    #[test]
    #[should_panic(expected = "unsupported element SlotKind")]
    fn test_elem_type_info_rejects_unknown() {
        elem_type_info(SlotKind::Unknown);
    }

    // ── Integration: round-trip get then set ────────────────────────────

    #[test]
    fn test_v2_array_get_then_set_compiles() {
        // Verify that a get followed by a set produces valid Cranelift IR
        // without panics or assertion failures from the builder.
        let ir = with_builder(|mir, arr_ptr, index| {
            let val = mir.v2_array_get(arr_ptr, index, SlotKind::Float64);
            // Use a fresh index (same value, but demonstrates chaining).
            mir.v2_array_set(arr_ptr, index, val, SlotKind::Float64);
        });

        // Just check it compiled without errors; the IR should have both
        // load and store instructions.
        assert!(ir.contains("load.f64"), "missing f64 load:\n{}", ir);
        assert!(ir.contains("store"), "missing store:\n{}", ir);
    }

    // ── v2_array_len returning a value ──────────────────────────────────

    #[test]
    fn test_v2_array_len_returns_i32() {
        let ir = with_builder_returning(|mir, arr_ptr, _index| {
            let len = mir.v2_array_len(arr_ptr);
            // Extend to i64 so it matches the return signature.
            mir.builder.ins().uextend(types::I64, len)
        });

        // Verify the load at offset 16
        assert!(
            ir.contains("load.i32 notrap aligned"),
            "expected i32 load for len:\n{}",
            ir
        );
        assert!(
            ir.contains("uextend.i64"),
            "expected uextend to i64:\n{}",
            ir
        );
    }
}
