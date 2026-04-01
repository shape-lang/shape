//! Typed struct field access codegen for MirToIR.
//!
//! Generates Cranelift IR for loading/storing typed struct fields at
//! compile-time-known offsets. Each operation is a single Cranelift load
//! or store instruction — no FFI calls, no runtime type dispatch.
//!
//! The complexity of proving types and resolving offsets lives in the
//! compiler (T15/T25); here we just emit the load/store.

use cranelift::prelude::*;

use super::MirToIR;

impl<'a, 'b> MirToIR<'a, 'b> {
    /// Load an f64 field from a struct pointer at a known byte offset.
    pub(crate) fn compile_field_load_f64(&mut self, struct_ptr: Value, offset: i32) -> Value {
        self.builder
            .ins()
            .load(types::F64, MemFlags::new(), struct_ptr, offset)
    }

    /// Load an i64 field from a struct pointer at a known byte offset.
    pub(crate) fn compile_field_load_i64(&mut self, struct_ptr: Value, offset: i32) -> Value {
        self.builder
            .ins()
            .load(types::I64, MemFlags::new(), struct_ptr, offset)
    }

    /// Load an i32 field from a struct pointer at a known byte offset.
    pub(crate) fn compile_field_load_i32(&mut self, struct_ptr: Value, offset: i32) -> Value {
        self.builder
            .ins()
            .load(types::I32, MemFlags::new(), struct_ptr, offset)
    }

    /// Load an i16 field from a struct pointer at a known byte offset.
    pub(crate) fn compile_field_load_i16(&mut self, struct_ptr: Value, offset: i32) -> Value {
        self.builder
            .ins()
            .load(types::I16, MemFlags::new(), struct_ptr, offset)
    }

    /// Load an i8 field from a struct pointer at a known byte offset.
    pub(crate) fn compile_field_load_i8(&mut self, struct_ptr: Value, offset: i32) -> Value {
        self.builder
            .ins()
            .load(types::I8, MemFlags::new(), struct_ptr, offset)
    }

    /// Load a pointer-sized field (Ptr) from a struct at a known byte offset.
    pub(crate) fn compile_field_load_ptr(&mut self, struct_ptr: Value, offset: i32) -> Value {
        self.builder
            .ins()
            .load(types::I64, MemFlags::new(), struct_ptr, offset)
    }

    /// Store an f64 value into a struct field at a known byte offset.
    pub(crate) fn compile_field_store_f64(
        &mut self,
        struct_ptr: Value,
        offset: i32,
        val: Value,
    ) {
        self.builder
            .ins()
            .store(MemFlags::new(), val, struct_ptr, offset);
    }

    /// Store an i64 value into a struct field at a known byte offset.
    pub(crate) fn compile_field_store_i64(
        &mut self,
        struct_ptr: Value,
        offset: i32,
        val: Value,
    ) {
        self.builder
            .ins()
            .store(MemFlags::new(), val, struct_ptr, offset);
    }

    /// Store an i32 value into a struct field at a known byte offset.
    pub(crate) fn compile_field_store_i32(
        &mut self,
        struct_ptr: Value,
        offset: i32,
        val: Value,
    ) {
        self.builder
            .ins()
            .store(MemFlags::new(), val, struct_ptr, offset);
    }

    /// Store an i16 value into a struct field at a known byte offset.
    pub(crate) fn compile_field_store_i16(
        &mut self,
        struct_ptr: Value,
        offset: i32,
        val: Value,
    ) {
        self.builder
            .ins()
            .store(MemFlags::new(), val, struct_ptr, offset);
    }

    /// Store an i8 value into a struct field at a known byte offset.
    pub(crate) fn compile_field_store_i8(
        &mut self,
        struct_ptr: Value,
        offset: i32,
        val: Value,
    ) {
        self.builder
            .ins()
            .store(MemFlags::new(), val, struct_ptr, offset);
    }

    /// Store a pointer-sized value (Ptr) into a struct field at a known byte offset.
    pub(crate) fn compile_field_store_ptr(
        &mut self,
        struct_ptr: Value,
        offset: i32,
        val: Value,
    ) {
        self.builder
            .ins()
            .store(MemFlags::new(), val, struct_ptr, offset);
    }
}

#[cfg(test)]
mod tests {
    use cranelift::prelude::*;
    use cranelift_jit::{JITBuilder, JITModule};
    use cranelift_module::Module;

    /// Helper: build a JIT function that loads an f64 from a struct pointer at
    /// a given byte offset. fn(ptr) -> f64 (as i64 bits).
    fn jit_field_load_f64(ptr: *const u8, offset: i32) -> f64 {
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
        sig.params.push(AbiParam::new(types::I64)); // struct_ptr
        sig.returns.push(AbiParam::new(types::F64)); // field value

        let func_id = module
            .declare_function("load_f64", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut fbuilder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = fbuilder.create_block();
        fbuilder.append_block_params_for_function_params(block);
        fbuilder.switch_to_block(block);
        fbuilder.seal_block(block);

        let struct_ptr = fbuilder.block_params(block)[0];
        // Same pattern as compile_field_load_f64
        let val = fbuilder
            .ins()
            .load(types::F64, MemFlags::new(), struct_ptr, offset);
        fbuilder.ins().return_(&[val]);
        fbuilder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64) -> f64 = unsafe { std::mem::transmute(code_ptr) };
        func(ptr as u64)
    }

    /// Helper: build a JIT function that stores an f64 to a struct at offset,
    /// then loads it back. fn(ptr, val) -> f64.
    fn jit_field_store_load_f64(ptr: *mut u8, offset: i32, val: f64) -> f64 {
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
        sig.params.push(AbiParam::new(types::I64)); // struct_ptr
        sig.params.push(AbiParam::new(types::F64)); // value
        sig.returns.push(AbiParam::new(types::F64)); // read-back

        let func_id = module
            .declare_function("store_load_f64", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut fbuilder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = fbuilder.create_block();
        fbuilder.append_block_params_for_function_params(block);
        fbuilder.switch_to_block(block);
        fbuilder.seal_block(block);

        let struct_ptr = fbuilder.block_params(block)[0];
        let store_val = fbuilder.block_params(block)[1];

        // Same pattern as compile_field_store_f64 then compile_field_load_f64
        fbuilder
            .ins()
            .store(MemFlags::new(), store_val, struct_ptr, offset);
        let loaded = fbuilder
            .ins()
            .load(types::F64, MemFlags::new(), struct_ptr, offset);
        fbuilder.ins().return_(&[loaded]);
        fbuilder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64, f64) -> f64 = unsafe { std::mem::transmute(code_ptr) };
        func(ptr as u64, val)
    }

    /// Helper: build a JIT function that loads an i32 from a struct pointer,
    /// sign-extends to i64. fn(ptr) -> i64.
    fn jit_field_load_i32(ptr: *const u8, offset: i32) -> i64 {
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
        sig.params.push(AbiParam::new(types::I64)); // struct_ptr
        sig.returns.push(AbiParam::new(types::I64)); // field value (sign-extended)

        let func_id = module
            .declare_function("load_i32", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut fbuilder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = fbuilder.create_block();
        fbuilder.append_block_params_for_function_params(block);
        fbuilder.switch_to_block(block);
        fbuilder.seal_block(block);

        let struct_ptr = fbuilder.block_params(block)[0];
        // Same pattern as compile_field_load_i32 + sign-extend to i64
        let val_i32 = fbuilder
            .ins()
            .load(types::I32, MemFlags::new(), struct_ptr, offset);
        let val_i64 = fbuilder.ins().sextend(types::I64, val_i32);
        fbuilder.ins().return_(&[val_i64]);
        fbuilder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64) -> i64 = unsafe { std::mem::transmute(code_ptr) };
        func(ptr as u64)
    }

    #[test]
    fn test_field_load_f64_at_offset_8() {
        // Simulate a struct with HeapHeader (8 bytes) then an f64 field at offset 8
        #[repr(C)]
        struct FakeStruct {
            _header: [u8; 8],
            x: f64,
        }
        let s = FakeStruct {
            _header: [0; 8],
            x: 3.14,
        };
        let result = jit_field_load_f64(&s as *const _ as *const u8, 8);
        assert!((result - 3.14).abs() < 1e-10);
    }

    #[test]
    fn test_field_load_f64_at_offset_16() {
        // Two f64 fields after the header
        #[repr(C)]
        struct FakeStruct {
            _header: [u8; 8],
            _x: f64,
            y: f64,
        }
        let s = FakeStruct {
            _header: [0; 8],
            _x: 1.0,
            y: 4.0,
        };
        let result = jit_field_load_f64(&s as *const _ as *const u8, 16);
        assert!((result - 4.0).abs() < 1e-10);
    }

    #[test]
    fn test_field_store_then_load_f64() {
        #[repr(C)]
        struct FakeStruct {
            _header: [u8; 8],
            x: f64,
        }
        let mut s = FakeStruct {
            _header: [0; 8],
            x: 0.0,
        };
        let result =
            jit_field_store_load_f64(&mut s as *mut _ as *mut u8, 8, 42.5);
        assert!((result - 42.5).abs() < 1e-10);
        // Also verify the memory was actually written
        assert!((s.x - 42.5).abs() < 1e-10);
    }

    #[test]
    fn test_field_load_i32_at_offset() {
        #[repr(C)]
        struct FakeStruct {
            _header: [u8; 8],
            width: i32,
            height: i32,
        }
        let s = FakeStruct {
            _header: [0; 8],
            width: 1920,
            height: 1080,
        };
        let w = jit_field_load_i32(&s as *const _ as *const u8, 8);
        let h = jit_field_load_i32(&s as *const _ as *const u8, 12);
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
    }

    #[test]
    fn test_field_load_i32_negative() {
        #[repr(C)]
        struct FakeStruct {
            _header: [u8; 8],
            value: i32,
        }
        let s = FakeStruct {
            _header: [0; 8],
            value: -42,
        };
        // i32 load should sign-extend to i64, preserving the negative value
        let result = jit_field_load_i32(&s as *const _ as *const u8, 8);
        assert_eq!(result, -42);
    }
}
