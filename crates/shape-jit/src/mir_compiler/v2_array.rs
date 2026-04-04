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

