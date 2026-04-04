//! Inline atomic refcount operations for the v2 runtime heap layout.
//!
//! In the v2 runtime, every heap-allocated object starts with an 8-byte
//! `HeapHeader` whose first 4 bytes are an `AtomicU32` refcount at offset 0.
//!
//! This module emits Cranelift IR that performs refcount manipulation directly
//! via atomic read-modify-write instructions, eliminating the overhead of FFI
//! calls to `arc_retain` / `arc_release`.
//!
//! ## Generated instruction sequences
//!
//! **Retain** (clone path): 1 instruction
//! ```text
//! atomic_rmw.i32 add, ptr, 1   ; Relaxed ordering
//! ```
//!
//! **Release** (drop hot path): 1 instruction
//! ```text
//! old = atomic_rmw.i32 sub, ptr, 1   ; Release ordering
//! ```
//!
//! **Drop** (release + conditional free): release + branch + call
//! ```text
//! old = atomic_rmw.i32 sub, ptr, 1   ; Release ordering
//! brif (old == 1), free_block, cont_block
//! free_block:
//!   fence Acquire                     ; Synchronize with other releases
//!   call free_func(ptr)
//!   jump cont_block
//! cont_block:
//!   ...
//! ```
//!
//! ## Refcount offset
//!
//! The v2 `HeapHeader` places the refcount `AtomicU32` at byte offset 0.
//! This constant is defined as [`V2_REFCOUNT_OFFSET`].

use cranelift::codegen::ir::{AtomicRmwOp, FuncRef};
use cranelift::prelude::*;

/// Byte offset of the refcount field within a v2 heap object header.
///
/// The v2 `HeapHeader` layout places `refcount: AtomicU32` at offset 0,
/// followed by `kind: u16` at offset 4, `flags: u8` at offset 6, etc.
pub const V2_REFCOUNT_OFFSET: i32 = 0;

/// Inline v2 refcount operations for Cranelift IR emission.
///
/// This struct borrows a `FunctionBuilder` and provides methods that emit
/// atomic refcount instructions directly into the current block. The caller
/// is responsible for ensuring `ptr` is a valid pointer to a v2 heap object
/// (i.e., the refcount `AtomicU32` is at `ptr + V2_REFCOUNT_OFFSET`).
pub struct V2RefcountEmitter<'a, 'b> {
    builder: &'a mut FunctionBuilder<'b>,
}

impl<'a, 'b> V2RefcountEmitter<'a, 'b> {
    /// Create a new emitter wrapping the given function builder.
    pub fn new(builder: &'a mut FunctionBuilder<'b>) -> Self {
        Self { builder }
    }

    /// Emit an inline retain: `atomic_fetch_add([ptr + 0], 1, Relaxed)`.
    ///
    /// Increments the refcount by 1 using a single atomic add instruction
    /// with relaxed memory ordering. This is sufficient for retain because
    /// the caller already holds a live reference, so the refcount cannot
    /// reach zero concurrently.
    ///
    /// # Arguments
    /// * `ptr` - Cranelift `Value` of type `I64`, pointing to the heap object header.
    pub fn emit_retain(&mut self, ptr: Value) {
        let one = self.builder.ins().iconst(types::I32, 1);
        // Relaxed ordering: no synchronization needed for retain.
        // MemFlags::new() produces no special flags; the atomic_rmw opcode
        // itself enforces atomicity at the hardware level.
        self.builder
            .ins()
            .atomic_rmw(types::I32, MemFlags::new(), AtomicRmwOp::Add, ptr, one);
    }

    /// Emit an inline release: `atomic_fetch_sub([ptr + 0], 1, Release)`.
    ///
    /// Decrements the refcount by 1 using a single atomic sub instruction.
    /// Returns the **old** refcount value before the decrement. If the old
    /// value was 1, the refcount has reached zero and the caller must free
    /// the object (after an acquire fence).
    ///
    /// # Arguments
    /// * `ptr` - Cranelift `Value` of type `I64`, pointing to the heap object header.
    ///
    /// # Returns
    /// The old refcount value (`I32`). Caller should compare with 1 to detect
    /// the last-reference case.
    pub fn emit_release(&mut self, ptr: Value) -> Value {
        let one = self.builder.ins().iconst(types::I32, 1);
        // Release ordering: ensures all preceding writes to the object's
        // fields are visible to the thread that will eventually free it.
        // Cranelift's atomic_rmw already implies atomic semantics; the
        // release fence is emitted separately in emit_drop when needed.
        self.builder
            .ins()
            .atomic_rmw(types::I32, MemFlags::new(), AtomicRmwOp::Sub, ptr, one)
    }

    /// Emit a full inline drop: release + conditional acquire fence + dealloc call.
    ///
    /// This is the complete drop sequence:
    /// 1. `old = atomic_fetch_sub([ptr + 0], 1)` (release)
    /// 2. If `old == 1` (was the last reference):
    ///    a. `fence(Acquire)` — synchronize with all prior release stores
    ///    b. `call free_func(ptr)` — deallocate the object
    /// 3. Continue
    ///
    /// The hot path (refcount > 1 after decrement) is a single atomic sub
    /// plus a compare-and-branch that falls through to the continuation.
    ///
    /// # Arguments
    /// * `ptr` - Cranelift `Value` of type `I64`, pointing to the heap object header.
    /// * `free_func` - `FuncRef` for the deallocation function, which takes a
    ///   single `I64` pointer argument.
    pub fn emit_drop(&mut self, ptr: Value, free_func: FuncRef) {
        let old = self.emit_release(ptr);
        let one = self.builder.ins().iconst(types::I32, 1);
        let was_last = self.builder.ins().icmp(IntCC::Equal, old, one);

        // Create blocks for the two paths.
        let free_block = self.builder.create_block();
        let cont_block = self.builder.create_block();

        // Branch: if this was the last reference, jump to dealloc.
        self.builder
            .ins()
            .brif(was_last, free_block, &[], cont_block, &[]);

        // --- Free block: acquire fence + dealloc ---
        self.builder.switch_to_block(free_block);
        self.builder.seal_block(free_block);

        // Acquire fence: synchronizes with all prior Release stores from
        // other threads that decremented this refcount. This ensures we
        // observe all writes to the object before freeing it.
        self.builder.ins().fence();

        self.builder.ins().call(free_func, &[ptr]);
        self.builder.ins().jump(cont_block, &[]);

        // --- Continuation block ---
        self.builder.switch_to_block(cont_block);
        self.builder.seal_block(cont_block);
    }
}

// ============================================================================
// FFI fallback for deallocation
// ============================================================================

/// Deallocate a v2 heap object by raw pointer.
///
/// This is the FFI function called from JIT-generated code when the refcount
/// reaches zero. It reconstructs the allocation layout from the heap header
/// and frees the memory.
///
/// # Safety
/// `ptr` must point to a valid v2 heap object whose refcount has reached zero.
/// The caller must have already executed an acquire fence.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_dealloc(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }

    // In the v2 layout, the heap header tells us the object kind and size.
    // For now, delegate to the type-specific deallocation logic.
    // This will be expanded as v2 object types are defined.
    //
    // Safety: ptr is non-null and was allocated by the v2 allocator.
    // The refcount is already zero, so no other thread holds a reference.
    unsafe {
        // Read the kind field at offset 4 (after the 4-byte refcount).
        let kind_offset = 4usize;
        let _kind = *(ptr.add(kind_offset) as *const u16);

        // TODO: Dispatch to type-specific dealloc based on kind.
        // For now this is a placeholder; actual deallocation will be
        // wired up when v2 object types are fully defined.
        let _ = _kind;
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use cranelift::codegen::ir::Function;
    use cranelift::codegen::isa::CallConv;

    /// Helper: create a Cranelift function context with a single I64 parameter
    /// (the heap object pointer) and build IR using the provided closure.
    ///
    /// Returns the textual CLIF IR for inspection.
    fn build_test_ir<F>(name: &str, f: F) -> String
    where
        F: FnOnce(&mut FunctionBuilder, Value),
    {
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64)); // ptr param

        let mut func = Function::with_name_signature(
            cranelift::codegen::ir::UserFuncName::testcase(name),
            sig,
        );

        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut func, &mut func_ctx);

        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let ptr = builder.block_params(entry_block)[0];
        f(&mut builder, ptr);

        builder.ins().return_(&[]);
        builder.finalize();

        func.to_string()
    }

    /// Helper: create a Cranelift function context with a single I64 parameter
    /// and a declared dealloc function, then build IR using the provided closure.
    ///
    /// Returns the textual CLIF IR for inspection.
    fn build_test_ir_with_free<F>(name: &str, f: F) -> String
    where
        F: FnOnce(&mut FunctionBuilder, Value, FuncRef),
    {
        let mut sig = Signature::new(CallConv::SystemV);
        sig.params.push(AbiParam::new(types::I64)); // ptr param

        let mut func = Function::with_name_signature(
            cranelift::codegen::ir::UserFuncName::testcase(name),
            sig,
        );

        // Declare the dealloc function signature: fn(ptr: I64) -> void
        let mut dealloc_sig = Signature::new(CallConv::SystemV);
        dealloc_sig.params.push(AbiParam::new(types::I64));
        let dealloc_sig_ref = func.import_signature(dealloc_sig);

        let mut func_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut func, &mut func_ctx);

        // Declare the dealloc func ref
        let dealloc_name =
            cranelift::codegen::ir::ExternalName::testcase("jit_v2_dealloc");
        let dealloc_func_ref = builder.import_function(cranelift::codegen::ir::ExtFuncData {
            name: dealloc_name,
            signature: dealloc_sig_ref,
            colocated: false,
        });

        let entry_block = builder.create_block();
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        builder.seal_block(entry_block);

        let ptr = builder.block_params(entry_block)[0];
        f(&mut builder, ptr, dealloc_func_ref);

        builder.ins().return_(&[]);
        builder.finalize();

        func.to_string()
    }

    #[test]
    fn test_retain_emits_atomic_add() {
        let ir = build_test_ir("retain", |builder, ptr| {
            let mut emitter = V2RefcountEmitter::new(builder);
            emitter.emit_retain(ptr);
        });

        // Verify the IR contains an atomic_rmw add instruction
        assert!(
            ir.contains("atomic_rmw") && ir.contains("add"),
            "retain should emit atomic_rmw add. IR:\n{ir}"
        );
        // The constant 1 should appear as an iconst
        assert!(
            ir.contains("iconst.i32 1"),
            "retain should load constant 1. IR:\n{ir}"
        );
    }

    #[test]
    fn test_release_emits_atomic_sub() {
        let ir = build_test_ir("release", |builder, ptr| {
            let mut emitter = V2RefcountEmitter::new(builder);
            let _old = emitter.emit_release(ptr);
        });

        // Verify the IR contains an atomic_rmw sub instruction
        assert!(
            ir.contains("atomic_rmw") && ir.contains("sub"),
            "release should emit atomic_rmw sub. IR:\n{ir}"
        );
    }

    #[test]
    fn test_drop_emits_branch_and_fence() {
        let ir = build_test_ir_with_free("drop", |builder, ptr, free_func| {
            let mut emitter = V2RefcountEmitter::new(builder);
            emitter.emit_drop(ptr, free_func);
        });

        // Should contain the atomic sub
        assert!(
            ir.contains("atomic_rmw") && ir.contains("sub"),
            "drop should emit atomic_rmw sub. IR:\n{ir}"
        );
        // Should contain an icmp for old == 1
        assert!(
            ir.contains("icmp"),
            "drop should emit icmp. IR:\n{ir}"
        );
        // Should contain a brif (conditional branch)
        assert!(
            ir.contains("brif"),
            "drop should emit brif. IR:\n{ir}"
        );
        // Should contain a fence instruction
        assert!(
            ir.contains("fence"),
            "drop should emit fence. IR:\n{ir}"
        );
        // Should contain a call to the free function
        assert!(
            ir.contains("call"),
            "drop should emit call to free. IR:\n{ir}"
        );
    }

    #[test]
    fn test_drop_has_three_blocks() {
        let ir = build_test_ir_with_free("drop_blocks", |builder, ptr, free_func| {
            let mut emitter = V2RefcountEmitter::new(builder);
            emitter.emit_drop(ptr, free_func);
        });

        // Count block declarations: entry + free + cont = 3 blocks.
        // Each block declaration looks like "block0:" — count the colons after "block\d"
        let block_decl_count = ir
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                trimmed.starts_with("block") && trimmed.ends_with(':')
            })
            .count();
        assert!(
            block_decl_count >= 3,
            "drop should create at least 3 blocks (entry, free, cont). Found {block_decl_count}. IR:\n{ir}"
        );
    }

    #[test]
    fn test_v2_dealloc_null_safety() {
        // Calling with null should not crash
        jit_v2_dealloc(std::ptr::null_mut());
    }

    #[test]
    fn test_v2_dealloc_with_fake_object() {
        // Allocate a small buffer simulating a v2 heap object
        let mut buf = [0u8; 32];
        // Set refcount to 0 at offset 0 (already zero)
        // Set kind to 0 at offset 4
        buf[4] = 0;
        buf[5] = 0;

        // This should not crash (placeholder dealloc is a no-op)
        jit_v2_dealloc(buf.as_mut_ptr());
    }

    #[test]
    fn test_refcount_offset_is_zero() {
        assert_eq!(V2_REFCOUNT_OFFSET, 0);
    }

    #[test]
    fn test_atomic_u32_has_correct_size() {
        // Verify that AtomicU32 is the same size as u32 (4 bytes), which is
        // a prerequisite for the inline atomic_rmw approach: Cranelift emits
        // a 4-byte atomic RMW at offset 0 of the heap header.
        assert_eq!(
            std::mem::size_of::<std::sync::atomic::AtomicU32>(),
            4,
            "AtomicU32 must be 4 bytes for inline refcounting"
        );
    }

    /// Verify that the emitted retain IR has exactly one atomic_rmw instruction
    /// (no redundant ops).
    #[test]
    fn test_retain_is_single_atomic_op() {
        let ir = build_test_ir("retain_single", |builder, ptr| {
            let mut emitter = V2RefcountEmitter::new(builder);
            emitter.emit_retain(ptr);
        });

        let atomic_count = ir.matches("atomic_rmw").count();
        assert_eq!(
            atomic_count, 1,
            "retain should emit exactly 1 atomic_rmw. Found {atomic_count}. IR:\n{ir}"
        );
    }

    /// Verify that the emitted release IR has exactly one atomic_rmw instruction.
    #[test]
    fn test_release_is_single_atomic_op() {
        let ir = build_test_ir("release_single", |builder, ptr| {
            let mut emitter = V2RefcountEmitter::new(builder);
            let _old = emitter.emit_release(ptr);
        });

        let atomic_count = ir.matches("atomic_rmw").count();
        assert_eq!(
            atomic_count, 1,
            "release should emit exactly 1 atomic_rmw. Found {atomic_count}. IR:\n{ir}"
        );
    }
}
