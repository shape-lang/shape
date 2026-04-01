//! Typed array access codegen for MirToIR.
//!
//! Generates Cranelift IR for `TypedArray<T>` element access — bounds-checked
//! loads and stores that compile to a single memory access on the hot path.
//!
//! ## TypedArray layout (24 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       8   header (HeapHeader)
//!   8       8   data (*mut T)
//!  16       4   len (u32)
//!  20       4   cap (u32)
//! ```
//!
//! Element access: `data[index]` = `load T [data_ptr + index * sizeof(T)]`

use cranelift::prelude::*;

use super::MirToIR;

/// Byte offset of the `data` pointer within TypedArray.
const TYPED_ARRAY_DATA_OFFSET: i32 = 8;
/// Byte offset of the `len` field within TypedArray.
const TYPED_ARRAY_LEN_OFFSET: i32 = 16;

impl<'a, 'b> MirToIR<'a, 'b> {
    // ── Bounds check helper ─────────────────────────────────────────

    /// Emit bounds check: trap if `index >= len`.
    /// `index` is i64, `len` is loaded as u32 from the array.
    fn emit_array_bounds_check(&mut self, arr: Value, index: Value) {
        // Load len as u32
        let len = self
            .builder
            .ins()
            .load(types::I32, MemFlags::new(), arr, TYPED_ARRAY_LEN_OFFSET);
        // Zero-extend len to i64 for comparison
        let len_i64 = self.builder.ins().uextend(types::I64, len);
        // Check: index (i64, unsigned) < len (i64, unsigned)
        let in_bounds = self
            .builder
            .ins()
            .icmp(IntCC::UnsignedLessThan, index, len_i64);
        self.builder.ins().trapz(in_bounds, TrapCode::User(0));
    }

    /// Load the data pointer from a TypedArray.
    fn emit_array_data_ptr(&mut self, arr: Value) -> Value {
        self.builder
            .ins()
            .load(types::I64, MemFlags::new(), arr, TYPED_ARRAY_DATA_OFFSET)
    }

    // ── f64 element access ──────────────────────────────────────────

    /// Emit: `val = typed_array_f64[index]` (bounds-checked).
    ///
    /// `arr` is a pointer to `TypedArray<f64>`, `index` is i64.
    /// Traps on out-of-bounds access. Returns the f64 value.
    pub(crate) fn compile_typed_array_get_f64(
        &mut self,
        arr: Value,
        index: Value,
    ) -> Value {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        // Element address: data_ptr + index * 8
        let byte_offset = self.builder.ins().imul_imm(index, 8);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder
            .ins()
            .load(types::F64, MemFlags::new(), elem_addr, 0)
    }

    /// Emit: `typed_array_f64[index] = val` (bounds-checked).
    pub(crate) fn compile_typed_array_set_f64(
        &mut self,
        arr: Value,
        index: Value,
        val: Value,
    ) {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        let byte_offset = self.builder.ins().imul_imm(index, 8);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder.ins().store(MemFlags::new(), val, elem_addr, 0);
    }

    // ── i64 element access ──────────────────────────────────────────

    /// Emit: `val = typed_array_i64[index]` (bounds-checked).
    pub(crate) fn compile_typed_array_get_i64(
        &mut self,
        arr: Value,
        index: Value,
    ) -> Value {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        let byte_offset = self.builder.ins().imul_imm(index, 8);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder
            .ins()
            .load(types::I64, MemFlags::new(), elem_addr, 0)
    }

    /// Emit: `typed_array_i64[index] = val` (bounds-checked).
    pub(crate) fn compile_typed_array_set_i64(
        &mut self,
        arr: Value,
        index: Value,
        val: Value,
    ) {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        let byte_offset = self.builder.ins().imul_imm(index, 8);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder.ins().store(MemFlags::new(), val, elem_addr, 0);
    }

    // ── i32 element access ──────────────────────────────────────────

    /// Emit: `val = typed_array_i32[index]` (bounds-checked).
    ///
    /// Returns the value sign-extended to i64 (for NaN-boxed slot storage).
    pub(crate) fn compile_typed_array_get_i32(
        &mut self,
        arr: Value,
        index: Value,
    ) -> Value {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        // Element address: data_ptr + index * 4
        let byte_offset = self.builder.ins().imul_imm(index, 4);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let val_i32 = self
            .builder
            .ins()
            .load(types::I32, MemFlags::new(), elem_addr, 0);
        // Sign-extend to i64 for storage
        self.builder.ins().sextend(types::I64, val_i32)
    }

    /// Emit: `typed_array_i32[index] = val` (bounds-checked).
    ///
    /// `val` is i64, narrowed to i32 for storage.
    pub(crate) fn compile_typed_array_set_i32(
        &mut self,
        arr: Value,
        index: Value,
        val: Value,
    ) {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        let byte_offset = self.builder.ins().imul_imm(index, 4);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        let val_i32 = self.builder.ins().ireduce(types::I32, val);
        self.builder
            .ins()
            .store(MemFlags::new(), val_i32, elem_addr, 0);
    }

    // ── u8 (bool) element access ────────────────────────────────────

    /// Emit: `val = typed_array_u8[index]` (bounds-checked).
    ///
    /// Returns the value zero-extended to i64.
    pub(crate) fn compile_typed_array_get_u8(
        &mut self,
        arr: Value,
        index: Value,
    ) -> Value {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        // Element address: data_ptr + index * 1
        let elem_addr = self.builder.ins().iadd(data_ptr, index);
        let val_i8 = self
            .builder
            .ins()
            .load(types::I8, MemFlags::new(), elem_addr, 0);
        // Zero-extend to i64
        self.builder.ins().uextend(types::I64, val_i8)
    }

    /// Emit: `typed_array_u8[index] = val` (bounds-checked).
    ///
    /// `val` is i64, narrowed to i8 for storage.
    pub(crate) fn compile_typed_array_set_u8(
        &mut self,
        arr: Value,
        index: Value,
        val: Value,
    ) {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        let elem_addr = self.builder.ins().iadd(data_ptr, index);
        let val_i8 = self.builder.ins().ireduce(types::I8, val);
        self.builder
            .ins()
            .store(MemFlags::new(), val_i8, elem_addr, 0);
    }

    // ── Pointer element access ──────────────────────────────────────

    /// Emit: `val = typed_array_ptr[index]` (bounds-checked).
    ///
    /// For `Array<SomeStruct>` or `Array<Array<T>>` — elements are 8-byte pointers.
    pub(crate) fn compile_typed_array_get_ptr(
        &mut self,
        arr: Value,
        index: Value,
    ) -> Value {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        let byte_offset = self.builder.ins().imul_imm(index, 8);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder
            .ins()
            .load(types::I64, MemFlags::new(), elem_addr, 0)
    }

    /// Emit: `typed_array_ptr[index] = val` (bounds-checked).
    pub(crate) fn compile_typed_array_set_ptr(
        &mut self,
        arr: Value,
        index: Value,
        val: Value,
    ) {
        self.emit_array_bounds_check(arr, index);
        let data_ptr = self.emit_array_data_ptr(arr);
        let byte_offset = self.builder.ins().imul_imm(index, 8);
        let elem_addr = self.builder.ins().iadd(data_ptr, byte_offset);
        self.builder.ins().store(MemFlags::new(), val, elem_addr, 0);
    }

    // ── Array length ────────────────────────────────────────────────

    /// Emit: `len = typed_array.len` (zero-extended to i64).
    pub(crate) fn compile_typed_array_len(&mut self, arr: Value) -> Value {
        let len = self
            .builder
            .ins()
            .load(types::I32, MemFlags::new(), arr, TYPED_ARRAY_LEN_OFFSET);
        self.builder.ins().uextend(types::I64, len)
    }
}

#[cfg(test)]
mod tests {
    use cranelift::prelude::*;
    use cranelift_jit::{JITBuilder, JITModule};
    use cranelift_module::Module;

    /// TypedArray layout for testing (24 bytes):
    ///   offset 0:  [u8; 8] header
    ///   offset 8:  *mut T  data pointer
    ///   offset 16: u32     len
    ///   offset 20: u32     cap
    #[repr(C)]
    struct FakeTypedArrayF64 {
        _header: [u8; 8],
        data: *mut f64,
        len: u32,
        cap: u32,
    }

    #[repr(C)]
    struct FakeTypedArrayI32 {
        _header: [u8; 8],
        data: *mut i32,
        len: u32,
        cap: u32,
    }

    /// Build a JIT function: fn(arr_ptr: i64, index: i64) -> f64
    /// Same pattern as compile_typed_array_get_f64:
    ///   bounds check, load data ptr, compute offset, load f64.
    fn jit_typed_array_get_f64(arr_ptr: u64, index: i64) -> f64 {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let isa_builder = cranelift_native::builder().unwrap();
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);
        let mut ctx = module.make_context();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // arr_ptr
        sig.params.push(AbiParam::new(types::I64)); // index
        sig.returns.push(AbiParam::new(types::F64)); // element value

        let func_id = module
            .declare_function("array_get_f64", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut fbuilder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = fbuilder.create_block();
        fbuilder.append_block_params_for_function_params(block);
        fbuilder.switch_to_block(block);
        fbuilder.seal_block(block);

        let arr = fbuilder.block_params(block)[0];
        let idx = fbuilder.block_params(block)[1];

        // Bounds check (same pattern as emit_array_bounds_check)
        let len = fbuilder
            .ins()
            .load(types::I32, MemFlags::new(), arr, 16); // TYPED_ARRAY_LEN_OFFSET
        let len_i64 = fbuilder.ins().uextend(types::I64, len);
        let in_bounds = fbuilder
            .ins()
            .icmp(IntCC::UnsignedLessThan, idx, len_i64);
        fbuilder.ins().trapz(in_bounds, TrapCode::User(0));

        // Load data pointer
        let data_ptr = fbuilder
            .ins()
            .load(types::I64, MemFlags::new(), arr, 8); // TYPED_ARRAY_DATA_OFFSET

        // Element address: data_ptr + index * 8
        let byte_offset = fbuilder.ins().imul_imm(idx, 8);
        let elem_addr = fbuilder.ins().iadd(data_ptr, byte_offset);
        let val = fbuilder
            .ins()
            .load(types::F64, MemFlags::new(), elem_addr, 0);
        fbuilder.ins().return_(&[val]);
        fbuilder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64, i64) -> f64 = unsafe { std::mem::transmute(code_ptr) };
        func(arr_ptr, index)
    }

    /// Build a JIT function: fn(arr_ptr: i64) -> i64 (length)
    fn jit_typed_array_len(arr_ptr: u64) -> i64 {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed_and_size").unwrap();
        let isa_builder = cranelift_native::builder().unwrap();
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .unwrap();
        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let mut module = JITModule::new(builder);
        let mut ctx = module.make_context();

        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));

        let func_id = module
            .declare_function("array_len", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut fbuilder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = fbuilder.create_block();
        fbuilder.append_block_params_for_function_params(block);
        fbuilder.switch_to_block(block);
        fbuilder.seal_block(block);

        let arr = fbuilder.block_params(block)[0];
        let len = fbuilder
            .ins()
            .load(types::I32, MemFlags::new(), arr, 16);
        let len_i64 = fbuilder.ins().uextend(types::I64, len);
        fbuilder.ins().return_(&[len_i64]);
        fbuilder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64) -> i64 = unsafe { std::mem::transmute(code_ptr) };
        func(arr_ptr)
    }

    #[test]
    fn test_typed_array_get_f64_element() {
        let data: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let arr = FakeTypedArrayF64 {
            _header: [0; 8],
            data: data.as_ptr() as *mut f64,
            len: 5,
            cap: 5,
        };
        let arr_ptr = &arr as *const _ as u64;
        assert!((jit_typed_array_get_f64(arr_ptr, 0) - 1.0).abs() < 1e-10);
        assert!((jit_typed_array_get_f64(arr_ptr, 2) - 3.0).abs() < 1e-10);
        assert!((jit_typed_array_get_f64(arr_ptr, 4) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_typed_array_len() {
        let data: Vec<f64> = vec![1.0, 2.0, 3.0];
        let arr = FakeTypedArrayF64 {
            _header: [0; 8],
            data: data.as_ptr() as *mut f64,
            len: 3,
            cap: 8,
        };
        let arr_ptr = &arr as *const _ as u64;
        assert_eq!(jit_typed_array_len(arr_ptr), 3);
    }

    #[test]
    fn test_typed_array_len_zero() {
        let arr = FakeTypedArrayF64 {
            _header: [0; 8],
            data: std::ptr::null_mut(),
            len: 0,
            cap: 0,
        };
        let arr_ptr = &arr as *const _ as u64;
        assert_eq!(jit_typed_array_len(arr_ptr), 0);
    }
}
