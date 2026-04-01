//! Inline refcounting for MirToIR — replaces FFI calls with inline atomics.
//!
//! Hot path for retain: 1 atomic add instruction, no FFI call.
//! Hot path for release (refcount > 1): 1 atomic sub + 1 branch, no FFI call.
//! Cold path for release (last ref): fence + FFI call for deallocation.

use cranelift::codegen::ir::AtomicRmwOp;
use cranelift::prelude::*;

impl<'a, 'b> super::MirToIR<'a, 'b> {
    /// Inline retain: atomic_fetch_add([ptr + 0], 1, Relaxed)
    /// HeapHeader.refcount is AtomicU32 at offset 0.
    /// Hot path: 1 atomic instruction, no FFI call.
    pub(crate) fn compile_inline_retain(&mut self, ptr: Value) {
        let one = self.builder.ins().iconst(types::I32, 1);
        // Cranelift atomic_rmw: atomically add 1 to refcount at ptr+0
        self.builder.ins().atomic_rmw(
            types::I32,
            MemFlags::new(),
            AtomicRmwOp::Add,
            ptr,
            one,
        );
        // No return value needed — we don't care about the old count
    }

    /// Inline release: atomic_fetch_sub([ptr + 0], 1, Release)
    /// If old count was 1, this was the last reference → call FFI dealloc.
    /// Hot path (not last ref): 1 atomic + 1 branch.
    /// Cold path (last ref): fence(Acquire) + FFI call for dealloc.
    pub(crate) fn compile_inline_release(&mut self, ptr: Value) {
        let one = self.builder.ins().iconst(types::I32, 1);

        // atomic_rmw returns the OLD value before subtraction
        let old_count = self.builder.ins().atomic_rmw(
            types::I32,
            MemFlags::new(),
            AtomicRmwOp::Sub,
            ptr,
            one,
        );

        // If old count was 1, this was the last reference
        let was_one = self.builder.ins().iconst(types::I32, 1);
        let is_last = self.builder.ins().icmp(IntCC::Equal, old_count, was_one);

        // Branch: last ref → dealloc block, else → continue
        let dealloc_block = self.builder.create_block();
        let continue_block = self.builder.create_block();
        self.builder
            .ins()
            .brif(is_last, dealloc_block, &[], continue_block, &[]);

        // Dealloc block (cold path)
        self.builder.switch_to_block(dealloc_block);
        self.builder.seal_block(dealloc_block);
        // Acquire fence before dealloc for memory ordering
        self.builder.ins().fence();
        // Call the existing arc_release FFI for actual deallocation.
        // It handles the full Drop logic including HeapValue cleanup
        // (freeing strings, arrays, closures, etc.).
        self.builder.ins().call(self.ffi.arc_release, &[ptr]);
        self.builder.ins().jump(continue_block, &[]);

        // Continue block (hot path resumes here)
        self.builder.switch_to_block(continue_block);
        self.builder.seal_block(continue_block);
    }
}

#[cfg(test)]
mod tests {
    use cranelift::codegen::ir::AtomicRmwOp;
    use cranelift::prelude::*;
    use cranelift_jit::{JITBuilder, JITModule};
    use cranelift_module::Module;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Build a JIT function that atomically increments a u32 at ptr+0 (inline retain).
    /// fn(ptr) -> ()
    fn jit_inline_retain(ptr: u64) {
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
        sig.params.push(AbiParam::new(types::I64)); // ptr

        let func_id = module
            .declare_function("retain", cranelift_module::Linkage::Local, &sig)
            .unwrap();
        ctx.func.signature = sig;

        let mut fn_builder_ctx = FunctionBuilderContext::new();
        let mut fbuilder = FunctionBuilder::new(&mut ctx.func, &mut fn_builder_ctx);
        let block = fbuilder.create_block();
        fbuilder.append_block_params_for_function_params(block);
        fbuilder.switch_to_block(block);
        fbuilder.seal_block(block);

        let obj_ptr = fbuilder.block_params(block)[0];

        // Same pattern as compile_inline_retain
        let one = fbuilder.ins().iconst(types::I32, 1);
        fbuilder.ins().atomic_rmw(
            types::I32,
            MemFlags::new(),
            AtomicRmwOp::Add,
            obj_ptr,
            one,
        );

        fbuilder.ins().return_(&[]);
        fbuilder.finalize();

        module.define_function(func_id, &mut ctx).unwrap();
        module.clear_context(&mut ctx);
        module.finalize_definitions().unwrap();

        let code_ptr = module.get_finalized_function(func_id);
        let func: fn(u64) = unsafe { std::mem::transmute(code_ptr) };
        func(ptr)
    }

    #[test]
    fn test_inline_retain_increments_refcount() {
        // Simulate a HeapHeader with refcount at offset 0
        #[repr(C)]
        struct FakeHeapHeader {
            refcount: AtomicU32,
            _rest: [u8; 4],
        }
        let header = FakeHeapHeader {
            refcount: AtomicU32::new(1),
            _rest: [0; 4],
        };
        let ptr = &header as *const _ as u64;

        // Retain should increment refcount from 1 to 2
        jit_inline_retain(ptr);
        assert_eq!(header.refcount.load(Ordering::SeqCst), 2);

        // Retain again: 2 -> 3
        jit_inline_retain(ptr);
        assert_eq!(header.refcount.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn test_inline_retain_multiple_calls() {
        #[repr(C)]
        struct FakeHeapHeader {
            refcount: AtomicU32,
            _rest: [u8; 4],
        }
        let header = FakeHeapHeader {
            refcount: AtomicU32::new(1),
            _rest: [0; 4],
        };
        let ptr = &header as *const _ as u64;

        for _ in 0..10 {
            jit_inline_retain(ptr);
        }
        assert_eq!(header.refcount.load(Ordering::SeqCst), 11);
    }
}
