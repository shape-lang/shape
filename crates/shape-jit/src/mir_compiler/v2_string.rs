//! String codegen for MirToIR.
//!
//! Generates Cranelift IR for `StringObj` field access — inline memory loads
//! at compile-time-known offsets. No FFI calls needed for simple field reads.
//!
//! ## StringObj layout (24 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       8   header (HeapHeader)
//!   8       8   data (*const u8, UTF-8 bytes)
//!  16       4   len (u32, byte count)
//!  20       4   _pad (u32)
//! ```

use cranelift::prelude::*;

use super::MirToIR;

/// Byte offset of the `data` pointer within StringObj.
const STRING_OBJ_DATA_OFFSET: i32 = 8;
/// Byte offset of the `len` field within StringObj.
const STRING_OBJ_LEN_OFFSET: i32 = 16;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Inline load of `StringObj.len` (u32, zero-extended to i64).
    ///
    /// `str_ptr` is a Cranelift value holding a pointer to a `StringObj`.
    /// Returns the byte length as an i64 value.
    pub(crate) fn compile_string_len(&mut self, str_ptr: Value) -> Value {
        let len = self
            .builder
            .ins()
            .load(types::I32, MemFlags::new(), str_ptr, STRING_OBJ_LEN_OFFSET);
        self.builder.ins().uextend(types::I64, len)
    }

    /// Inline load of `StringObj.data` (pointer to UTF-8 bytes).
    ///
    /// `str_ptr` is a Cranelift value holding a pointer to a `StringObj`.
    /// Returns the data pointer as an i64 value.
    pub(crate) fn compile_string_data_ptr(&mut self, str_ptr: Value) -> Value {
        self.builder
            .ins()
            .load(types::I64, MemFlags::new(), str_ptr, STRING_OBJ_DATA_OFFSET)
    }
}

#[cfg(test)]
mod tests {
    use cranelift::prelude::*;
    use cranelift_jit::{JITBuilder, JITModule};
    use cranelift_module::Module;

    /// Simulate StringObj layout for testing (24 bytes):
    ///   offset 0:  [u8; 8] header
    ///   offset 8:  *const u8 data pointer
    ///   offset 16: u32      len
    ///   offset 20: u32      _pad
    #[repr(C)]
    struct FakeStringObj {
        _header: [u8; 8],
        data: *const u8,
        len: u32,
        _pad: u32,
    }

    /// Build a JIT function: fn(str_ptr: i64) -> i64 (length, zero-extended)
    /// Same pattern as compile_string_len.
    fn jit_string_len(ptr_val: u64) -> i64 {
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
            .declare_function("string_len", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut fbuilder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = fbuilder.create_block();
        fbuilder.append_block_params_for_function_params(block);
        fbuilder.switch_to_block(block);
        fbuilder.seal_block(block);

        let str_ptr = fbuilder.block_params(block)[0];
        // Same pattern as compile_string_len
        let len = fbuilder
            .ins()
            .load(types::I32, MemFlags::new(), str_ptr, 16); // STRING_OBJ_LEN_OFFSET
        let len_i64 = fbuilder.ins().uextend(types::I64, len);
        fbuilder.ins().return_(&[len_i64]);
        fbuilder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64) -> i64 = unsafe { std::mem::transmute(code_ptr) };
        func(ptr_val)
    }

    /// Build a JIT function: fn(str_ptr: i64) -> i64 (data pointer)
    /// Same pattern as compile_string_data_ptr.
    fn jit_string_data_ptr(ptr_val: u64) -> i64 {
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
            .declare_function("string_data_ptr", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut fbuilder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = fbuilder.create_block();
        fbuilder.append_block_params_for_function_params(block);
        fbuilder.switch_to_block(block);
        fbuilder.seal_block(block);

        let str_ptr = fbuilder.block_params(block)[0];
        // Same pattern as compile_string_data_ptr
        let data = fbuilder
            .ins()
            .load(types::I64, MemFlags::new(), str_ptr, 8); // STRING_OBJ_DATA_OFFSET
        fbuilder.ins().return_(&[data]);
        fbuilder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64) -> i64 = unsafe { std::mem::transmute(code_ptr) };
        func(ptr_val)
    }

    #[test]
    fn test_string_len_codegen() {
        let data = b"hello";
        let s = FakeStringObj {
            _header: [0; 8],
            data: data.as_ptr(),
            len: 5,
            _pad: 0,
        };
        let ptr = &s as *const _ as u64;
        assert_eq!(jit_string_len(ptr), 5);
    }

    #[test]
    fn test_string_len_zero() {
        let s = FakeStringObj {
            _header: [0; 8],
            data: std::ptr::null(),
            len: 0,
            _pad: 0,
        };
        let ptr = &s as *const _ as u64;
        assert_eq!(jit_string_len(ptr), 0);
    }

    #[test]
    fn test_string_data_ptr_codegen() {
        let data = b"test";
        let s = FakeStringObj {
            _header: [0; 8],
            data: data.as_ptr(),
            len: 4,
            _pad: 0,
        };
        let ptr = &s as *const _ as u64;
        let result = jit_string_data_ptr(ptr);
        assert_eq!(result as u64, data.as_ptr() as u64);
    }

    #[test]
    fn test_string_len_large() {
        let s = FakeStringObj {
            _header: [0; 8],
            data: std::ptr::null(),
            len: 1_000_000,
            _pad: 0,
        };
        let ptr = &s as *const _ as u64;
        assert_eq!(jit_string_len(ptr), 1_000_000);
    }
}
