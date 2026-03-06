//! TypedObject operations: typed field access, allocation, merge

use cranelift::prelude::*;

use crate::nan_boxing::*;
use shape_vm::bytecode::{Instruction, Operand};

use crate::translator::types::{BytecodeToIR, CompilationMode};

impl<'a, 'b> BytecodeToIR<'a, 'b> {
    /// Compile GetFieldTyped opcode - get field from typed object using precomputed offset
    ///
    /// Stack: [obj] -> [field_value]
    /// Operand: TypedField { type_id, field_idx, field_type_tag }
    ///
    /// Performance: ~2ns vs ~25ns for GetProp (12x faster)
    /// Uses direct memory access for TypedObjects, FFI fallback for HashMap objects.
    ///
    /// In kernel mode, state is always accessed via kernel_state_ptr (no stack pop needed).
    pub(crate) fn compile_get_field_typed(&mut self, instr: &Instruction) -> Result<(), String> {
        let (type_id, field_idx, field_type_tag) = match &instr.operand {
            Some(Operand::TypedField {
                type_id,
                field_idx,
                field_type_tag,
            }) => (*type_id, *field_idx, *field_type_tag),
            _ => return Err("GetFieldTyped requires TypedField operand".to_string()),
        };

        // Compute byte offset from field index (all slots are 8 bytes)
        let offset = field_idx * 8;

        // Kernel mode: direct access via state_ptr, no tag checking
        if self.mode == CompilationMode::Kernel {
            return self.compile_get_field_typed_kernel(offset, field_idx);
        }

        // Standard mode continues below (type_id used in slow path)
        let _ = type_id; // Used below in slow path
        let obj = self
            .stack_pop()
            .ok_or("GetFieldTyped: missing object on stack")?;

        // Type check: TAG_HEAP + HeapKind == HK_TYPED_OBJECT
        let is_typed = self.emit_is_heap_kind(obj, HK_TYPED_OBJECT);

        // Extract alloc_ptr for use in the fast path below
        let payload_mask = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        let alloc_ptr = self.builder.ins().band(obj, payload_mask);

        // Control flow blocks
        let fast_block = self.builder.create_block();
        let slow_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);
        self.builder
            .ins()
            .brif(is_typed, fast_block, &[], slow_block, &[]);

        // Fast path: load TypedObject pointer from JitAlloc.data, then access field
        self.builder.switch_to_block(fast_block);
        self.builder.seal_block(fast_block);
        // alloc_ptr + 8 = JitAlloc.data (which is *const u8 to TypedObject)
        let obj_ptr = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            alloc_ptr,
            JIT_ALLOC_DATA_OFFSET as i32,
        );
        // Field address: obj_ptr + 8 (TypedObject header) + field offset
        let field_addr = self.builder.ins().iadd_imm(obj_ptr, 8 + offset as i64);
        let value = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), field_addr, 0);
        self.builder.ins().jump(merge_block, &[value]);

        // Slow path: FFI fallback
        self.builder.switch_to_block(slow_block);
        self.builder.seal_block(slow_block);
        let field_idx_val = self.builder.ins().iconst(types::I64, field_idx as i64);
        let type_id_val = self.builder.ins().iconst(types::I64, type_id as i64);
        let offset_val = self.builder.ins().iconst(types::I64, offset as i64);
        let inst = self.builder.ins().call(
            self.ffi.get_field_typed,
            &[obj, type_id_val, field_idx_val, offset_val],
        );
        let slow_result = self.builder.inst_results(inst)[0];
        self.builder.ins().jump(merge_block, &[slow_result]);

        // Merge block
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        let result = self.builder.block_params(merge_block)[0];
        self.stack_push(result);
        Ok(())
    }

    /// Compile SetFieldTyped opcode - set field on typed object using precomputed offset
    ///
    /// Stack: [obj, value] -> [obj]
    /// Operand: TypedField { type_id, field_idx, field_type_tag }
    ///
    /// Performance: ~2ns vs ~30ns for SetProp (15x faster)
    /// Uses direct memory store for TypedObjects, FFI fallback for HashMap objects.
    ///
    /// In kernel mode, state is always accessed via kernel_state_ptr.
    pub(crate) fn compile_set_field_typed(&mut self, instr: &Instruction) -> Result<(), String> {
        let (type_id, field_idx, field_type_tag) = match &instr.operand {
            Some(Operand::TypedField {
                type_id,
                field_idx,
                field_type_tag,
            }) => (*type_id, *field_idx, *field_type_tag),
            _ => return Err("SetFieldTyped requires TypedField operand".to_string()),
        };

        // Compute byte offset from field index (all slots are 8 bytes)
        let offset = field_idx * 8;

        // Kernel mode: direct access via state_ptr, no tag checking
        if self.mode == CompilationMode::Kernel {
            return self.compile_set_field_typed_kernel(offset, field_idx);
        }

        // Standard mode continues below (type_id used in slow path)
        let _ = type_id; // Used below in slow path
        let value = self
            .stack_pop()
            .ok_or("SetFieldTyped: missing value on stack")?;
        let obj = self
            .stack_pop()
            .ok_or("SetFieldTyped: missing object on stack")?;

        // Type check: TAG_HEAP + HeapKind == HK_TYPED_OBJECT
        let is_typed = self.emit_is_heap_kind(obj, HK_TYPED_OBJECT);

        // Extract alloc_ptr for use in the fast path below
        let payload_mask = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        let alloc_ptr = self.builder.ins().band(obj, payload_mask);

        // Control flow blocks
        let fast_block = self.builder.create_block();
        let slow_block = self.builder.create_block();
        let merge_block = self.builder.create_block();
        self.builder.append_block_param(merge_block, types::I64);
        self.builder
            .ins()
            .brif(is_typed, fast_block, &[], slow_block, &[]);

        // Fast path: load TypedObject pointer from JitAlloc.data, then store field
        self.builder.switch_to_block(fast_block);
        self.builder.seal_block(fast_block);
        let obj_ptr = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            alloc_ptr,
            JIT_ALLOC_DATA_OFFSET as i32,
        );
        let field_addr = self.builder.ins().iadd_imm(obj_ptr, 8 + offset as i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, field_addr, 0);
        self.builder.ins().jump(merge_block, &[obj]);

        // Slow path: FFI fallback
        self.builder.switch_to_block(slow_block);
        self.builder.seal_block(slow_block);
        let field_idx_val = self.builder.ins().iconst(types::I64, field_idx as i64);
        let type_id_val = self.builder.ins().iconst(types::I64, type_id as i64);
        let offset_val = self.builder.ins().iconst(types::I64, offset as i64);
        let inst = self.builder.ins().call(
            self.ffi.set_field_typed,
            &[obj, value, type_id_val, field_idx_val, offset_val],
        );
        let slow_result = self.builder.inst_results(inst)[0];
        self.builder.ins().jump(merge_block, &[slow_result]);

        // Merge block
        self.builder.switch_to_block(merge_block);
        self.builder.seal_block(merge_block);
        let result = self.builder.block_params(merge_block)[0];
        self.stack_push(result);
        Ok(())
    }

    /// Compile NewTypedObject opcode
    ///
    /// Creates a new TypedObject with fields initialized from the stack.
    ///
    /// Stack: [...field_values] -> [typed_obj]
    /// Operand: TypedObjectAlloc { schema_id, field_count }
    ///
    /// Performance: ~20ns vs ~100ns for NewObject (5x faster)
    pub(crate) fn compile_new_typed_object(&mut self, instr: &Instruction) -> Result<(), String> {
        let (schema_id, field_count) = match &instr.operand {
            Some(Operand::TypedObjectAlloc {
                schema_id,
                field_count,
            }) => (*schema_id, *field_count),
            _ => return Err("NewTypedObject requires TypedObjectAlloc operand".to_string()),
        };

        // Pop field values from virtual stack and store their indices
        let mut field_indices: Vec<Value> = Vec::with_capacity(field_count as usize);
        for _ in 0..field_count {
            let val = self
                .stack_pop()
                .ok_or("NewTypedObject: missing field value on stack")?;
            field_indices.push(val);
        }
        // Reverse since we popped in reverse order (LIFO)
        field_indices.reverse();

        // Calculate data size for allocation
        let data_size = (field_count as i64) * 8;

        // Call jit_typed_object_alloc to get a fresh TypedObject
        let schema_id_val = self.builder.ins().iconst(types::I32, schema_id as i64);
        let data_size_val = self.builder.ins().iconst(types::I64, data_size);
        let alloc_inst = self
            .builder
            .ins()
            .call(self.ffi.typed_object_alloc, &[schema_id_val, data_size_val]);
        let obj = self.builder.inst_results(alloc_inst)[0];

        // Store each field value at the appropriate offset
        // TypedObject layout: 8-byte header + fields at offsets 0, 8, 16, etc.
        let payload_mask = self.builder.ins().iconst(types::I64, PAYLOAD_MASK as i64);
        let alloc_ptr = self.builder.ins().band(obj, payload_mask);
        // Load TypedObject raw pointer from JitAlloc.data
        let typed_ptr = self.builder.ins().load(
            types::I64,
            MemFlags::trusted(),
            alloc_ptr,
            JIT_ALLOC_DATA_OFFSET as i32,
        );

        for (i, field_val) in field_indices.into_iter().enumerate() {
            let field_offset = 8 + (i as i64 * 8); // TypedObject header is 8 bytes
            let field_addr = self.builder.ins().iadd_imm(typed_ptr, field_offset);
            self.builder
                .ins()
                .store(MemFlags::trusted(), field_val, field_addr, 0);
        }

        self.stack_push(obj);
        Ok(())
    }

    /// Compile TypedMergeObject opcode
    ///
    /// Merges two TypedObjects into a new TypedObject using O(1) memcpy.
    /// The target schema is pre-registered at compile time.
    ///
    /// Stack: [left_obj, right_obj] -> [merged_obj]
    /// Operand: TypedMerge { target_schema_id, left_size, right_size }
    ///
    /// Performance: O(1) memcpy-based merge, no HashMap allocation
    pub(crate) fn compile_typed_merge_object(&mut self, instr: &Instruction) -> Result<(), String> {
        let (target_schema_id, left_size, right_size) = match &instr.operand {
            Some(Operand::TypedMerge {
                target_schema_id,
                left_size,
                right_size,
            }) => (*target_schema_id, *left_size, *right_size),
            _ => return Err("TypedMergeObject requires TypedMerge operand".to_string()),
        };

        // Pop operands from stack (LIFO order: right first, then left)
        let right_obj = self
            .stack_pop()
            .ok_or("TypedMergeObject: missing right object on stack")?;
        let left_obj = self
            .stack_pop()
            .ok_or("TypedMergeObject: missing left object on stack")?;

        // Build arguments for FFI call
        let target_id_val = self
            .builder
            .ins()
            .iconst(types::I32, target_schema_id as i64);
        let left_size_val = self.builder.ins().iconst(types::I64, left_size as i64);
        let right_size_val = self.builder.ins().iconst(types::I64, right_size as i64);

        // Call jit_typed_merge_object(target_schema_id, left_size, right_size, left_obj, right_obj)
        let call_inst = self.builder.ins().call(
            self.ffi.typed_merge_object,
            &[
                target_id_val,
                left_size_val,
                right_size_val,
                left_obj,
                right_obj,
            ],
        );
        let result = self.builder.inst_results(call_inst)[0];

        self.stack_push(result);
        Ok(())
    }

    // ========================================================================
    // Kernel Mode Helpers (direct memory access, no FFI)
    // ========================================================================

    /// Kernel mode: GetFieldTyped via state_ptr
    ///
    /// In kernel mode, state is always a TypedObject at kernel_state_ptr.
    /// No type checking needed - just direct memory load.
    fn compile_get_field_typed_kernel(
        &mut self,
        offset: u16,
        _field_idx: u16,
    ) -> Result<(), String> {
        let state_ptr = self
            .kernel_state_ptr
            .ok_or("Kernel mode requires kernel_state_ptr")?;

        // Pop object from stack (ignored in kernel mode - state is always at state_ptr)
        // We still pop to maintain stack consistency with standard mode
        let _ = self.stack_pop();

        // field_addr = state_ptr + 8 (header) + offset
        let field_addr = self.builder.ins().iadd_imm(state_ptr, 8 + offset as i64);
        let value = self
            .builder
            .ins()
            .load(types::I64, MemFlags::trusted(), field_addr, 0);

        self.stack_push(value);
        Ok(())
    }

    /// Kernel mode: SetFieldTyped via state_ptr
    ///
    /// In kernel mode, state is always a TypedObject at kernel_state_ptr.
    /// No type checking needed - just direct memory store.
    fn compile_set_field_typed_kernel(
        &mut self,
        offset: u16,
        _field_idx: u16,
    ) -> Result<(), String> {
        let state_ptr = self
            .kernel_state_ptr
            .ok_or("Kernel mode requires kernel_state_ptr")?;

        // Pop value and object (object ignored - state is always at state_ptr)
        let value = self
            .stack_pop()
            .ok_or("SetFieldTyped: missing value on stack")?;
        let _ = self.stack_pop(); // Pop object (ignored in kernel mode)

        // field_addr = state_ptr + 8 (header) + offset
        let field_addr = self.builder.ins().iadd_imm(state_ptr, 8 + offset as i64);
        self.builder
            .ins()
            .store(MemFlags::trusted(), value, field_addr, 0);

        // Push state_ptr back (for chaining) - represented as the kernel's "state object"
        // In kernel mode, we just push a sentinel that's not used
        self.stack_push(state_ptr);
        Ok(())
    }
}
