//! JIT integration tests for TypedObject
//!
//! These tests verify the JIT-compiled GetFieldTyped/SetFieldTyped opcodes
//! use direct memory access for TypedObjects (fast path) and fall back to
//! FFI for HashMap objects (slow path).

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::compiler::JITCompiler;
    use crate::context::{JITConfig, JITContext, JittedStrategyFn};
    use crate::ffi::data::jit_get_field_typed;
    use crate::nan_boxing::*;
    use shape_runtime::type_schema::{FieldType, TypeSchema};
    use shape_vm::bytecode::{BytecodeProgram, Instruction, OpCode, Operand};
    use std::alloc::{Layout, dealloc};

    const TYPED_OBJECT_ALIGNMENT: usize = 64;

    /// Test GetFieldTyped with TypedObject (fast path - direct memory load)
    ///
    /// IGNORED (v2 BytecodeToIR removal): This test feeds a hand-built
    /// `BytecodeProgram` to `JITCompiler::compile_strategy`. After Phase 4 deleted
    /// `BytecodeToIR`, MirToIR is the only JIT path and requires `top_level_mir`
    /// to be populated by the bytecode compiler from AST. Equivalent coverage
    /// exists in `mir_compiler::integration_tests` and the `jit_get_field_typed`
    /// FFI function is exercised directly in `test_jit_field_access_both_paths`.
    #[test]
    #[ignore = "v2: tests deleted BytecodeToIR path; FFI exercised by test_jit_field_access_both_paths"]
    fn test_jit_get_field_typed_fast_path() {
        // Note: LoadModuleBinding/StoreModuleBinding read from ctx.locals[] in memory
        // LoadLocal/StoreLocal use Cranelift's SSA variables (not ctx.locals[])

        // First, verify LoadModuleBinding/StoreModuleBinding work correctly
        let simple_program = BytecodeProgram {
            instructions: vec![
                // Load from ctx.locals[0]
                Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),
                // Store to ctx.locals[1]
                Instruction::new(OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(1))),
                // Return
                Instruction::simple(OpCode::Return),
            ],
            constants: vec![],
            ..Default::default()
        };

        let mut jit = JITCompiler::new(JITConfig::default()).unwrap();
        let simple_func: JittedStrategyFn =
            jit.compile_strategy("test_copy", &simple_program).unwrap();

        let mut ctx = JITContext::default();
        ctx.locals[0] = box_number(12345.0);

        let _result = unsafe { simple_func(&mut ctx) };

        assert_eq!(
            ctx.locals[1], ctx.locals[0],
            "LoadModuleBinding/StoreModuleBinding should copy value"
        );
        assert_eq!(
            unbox_number(ctx.locals[1]),
            12345.0,
            "Value should be 12345.0"
        );

        // Now test GetFieldTyped
        let schema = TypeSchema::new(
            "TestPoint",
            vec![
                ("x".to_string(), FieldType::F64),
                ("y".to_string(), FieldType::F64),
                ("z".to_string(), FieldType::F64),
            ],
        );
        let schema_id = schema.id;

        let ptr = TypedObject::alloc(&schema);
        assert!(!ptr.is_null());

        unsafe {
            let obj = &mut *ptr;
            obj.set_field(0, box_number(100.0)); // x at offset 0
            obj.set_field(8, box_number(200.0)); // y at offset 8
            obj.set_field(16, box_number(300.0)); // z at offset 16
        }

        let typed_obj_bits = box_typed_object(ptr as *const u8);
        assert!(is_typed_object(typed_obj_bits));

        // Debug: verify the FFI function works directly
        let direct_result = jit_get_field_typed(typed_obj_bits, schema_id as u64, 1, 8);
        assert!(is_number(direct_result), "Direct FFI should return number");
        assert_eq!(
            unbox_number(direct_result),
            200.0,
            "Direct FFI should return 200.0"
        );

        // Create a new JIT compiler for the GetFieldTyped test
        let mut jit2 = JITCompiler::new(JITConfig::default()).unwrap();
        let program = BytecodeProgram {
            instructions: vec![
                // Use LoadModuleBinding to read from ctx.locals[0]
                Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),
                Instruction::new(
                    OpCode::GetFieldTyped,
                    Some(Operand::TypedField {
                        type_id: schema_id as u16,
                        field_idx: 1,
                        field_type_tag: 0,
                    }),
                ),
                // Use StoreModuleBinding to write to ctx.locals[1]
                Instruction::new(OpCode::StoreModuleBinding, Some(Operand::ModuleBinding(1))),
                Instruction::simple(OpCode::Return),
            ],
            constants: vec![],
            ..Default::default()
        };

        let func: JittedStrategyFn = jit2.compile_strategy("test_get_typed", &program).unwrap();

        let mut ctx2 = JITContext::default();
        ctx2.locals[0] = typed_obj_bits;

        let _result = unsafe { func(&mut ctx2) };

        let result_bits = ctx2.locals[1];
        assert!(
            is_number(result_bits),
            "Result should be a number, got 0x{:016X} (schema_id={})",
            result_bits,
            schema_id
        );
        assert_eq!(unbox_number(result_bits), 200.0, "Expected y=200.0");

        super::super::allocation::jit_typed_object_dec_ref(typed_obj_bits, schema.data_size as u64);
    }

    /// Test SetFieldTyped with TypedObject (fast path - direct memory store)
    ///
    /// IGNORED: see `test_jit_get_field_typed_fast_path` above.
    #[test]
    #[ignore = "v2: tests deleted BytecodeToIR path; FFI exercised by test_jit_field_access_both_paths"]
    fn test_jit_set_field_typed_fast_path() {
        // Create a TypedObject with initial values
        let schema = TypeSchema::new(
            "MutablePoint",
            vec![
                ("x".to_string(), FieldType::F64),
                ("y".to_string(), FieldType::F64),
            ],
        );
        let schema_id = schema.id;

        let ptr = TypedObject::alloc(&schema);
        assert!(!ptr.is_null());

        unsafe {
            let obj = &mut *ptr;
            obj.set_field(0, box_number(10.0)); // x at offset 0
            obj.set_field(8, box_number(20.0)); // y at offset 8
        }

        let typed_obj_bits = box_typed_object(ptr as *const u8);

        // Create bytecode: push object, push value, set field
        // Use LoadModuleBinding (reads from ctx.locals[]) instead of LoadLocal (SSA variables)
        let program = BytecodeProgram {
            instructions: vec![
                // Load the TypedObject from ctx.locals[0]
                Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(0))),
                // Load the new value from ctx.locals[1]
                Instruction::new(OpCode::LoadModuleBinding, Some(Operand::ModuleBinding(1))),
                // Set field at offset 8 (y field)
                Instruction::new(
                    OpCode::SetFieldTyped,
                    Some(Operand::TypedField {
                        type_id: schema_id as u16,
                        field_idx: 1,
                        field_type_tag: 0,
                    }),
                ),
                // Return
                Instruction::simple(OpCode::Return),
            ],
            constants: vec![],
            ..Default::default()
        };

        let mut jit = JITCompiler::new(JITConfig::default()).unwrap();
        let func: JittedStrategyFn = jit.compile_strategy("test_set_typed", &program).unwrap();

        let mut ctx = JITContext::default();
        ctx.locals[0] = typed_obj_bits;
        ctx.locals[1] = box_number(999.0); // New value for y

        let _result = unsafe { func(&mut ctx) };

        // Verify the field was updated
        unsafe {
            let obj = &*ptr;
            let y_value = unbox_number(obj.get_field(8));
            assert_eq!(y_value, 999.0, "y field should be updated to 999.0");
        }

        // Clean up
        super::super::allocation::jit_typed_object_dec_ref(typed_obj_bits, schema.data_size as u64);
    }

    /// Test that GetFieldTyped works with TypedObjects
    #[test]
    fn test_jit_field_access_both_paths() {
        // Create two TypedObjects with different schemas
        let schema1 = TypeSchema::new("Data", vec![("value".to_string(), FieldType::F64)]);

        let typed_ptr1 = TypedObject::alloc(&schema1);
        unsafe {
            (*typed_ptr1).set_field(0, box_number(42.0));
        }
        let typed_bits1 = box_typed_object(typed_ptr1 as *const u8);

        // Test TypedObject access
        let result1 = jit_get_field_typed(typed_bits1, 0, 0, 0);
        assert!(is_number(result1));
        assert_eq!(unbox_number(result1), 42.0);

        // Create another TypedObject with a different value
        let schema2 = TypeSchema::new("Data2", vec![("value".to_string(), FieldType::F64)]);

        let typed_ptr2 = TypedObject::alloc(&schema2);
        unsafe {
            (*typed_ptr2).set_field(0, box_number(99.0));
        }
        let typed_bits2 = box_typed_object(typed_ptr2 as *const u8);

        // Test second TypedObject access
        let result2 = jit_get_field_typed(typed_bits2, 0, 0, 0);
        assert!(is_number(result2));
        assert_eq!(unbox_number(result2), 99.0);

        // Clean up
        super::super::allocation::jit_typed_object_dec_ref(typed_bits1, schema1.data_size as u64);
        super::super::allocation::jit_typed_object_dec_ref(typed_bits2, schema2.data_size as u64);
    }

    /// Performance comparison: TypedObject vs HashMap field access
    #[test]
    fn test_typed_vs_hashmap_performance() {
        use std::collections::HashMap;
        use std::time::Instant;

        const ITERATIONS: usize = 1_000_000;
        const FIELD_COUNT: usize = 6; // Generic 6-field record

        // Setup: Create a TypedObject with generic field names
        let schema = TypeSchema::new(
            "Record",
            vec![
                ("field0".to_string(), FieldType::I64),
                ("field1".to_string(), FieldType::F64),
                ("field2".to_string(), FieldType::F64),
                ("field3".to_string(), FieldType::F64),
                ("field4".to_string(), FieldType::F64),
                ("field5".to_string(), FieldType::F64),
            ],
        );

        let typed_ptr = TypedObject::alloc(&schema);
        assert!(!typed_ptr.is_null());

        // Initialize TypedObject fields
        unsafe {
            let obj = &mut *typed_ptr;
            obj.set_field(0, box_number(1705000000000.0)); // field0
            obj.set_field(8, box_number(100.0)); // field1
            obj.set_field(16, box_number(105.0)); // field2
            obj.set_field(24, box_number(98.0)); // field3
            obj.set_field(32, box_number(103.0)); // field4
            obj.set_field(40, box_number(1000000.0)); // field5
        }

        // Setup: Create a HashMap-based object
        let mut hashmap: HashMap<String, u64> = HashMap::new();
        hashmap.insert("field0".to_string(), box_number(1705000000000.0));
        hashmap.insert("field1".to_string(), box_number(100.0));
        hashmap.insert("field2".to_string(), box_number(105.0));
        hashmap.insert("field3".to_string(), box_number(98.0));
        hashmap.insert("field4".to_string(), box_number(103.0));
        hashmap.insert("field5".to_string(), box_number(1000000.0));

        // Benchmark TypedObject access
        let typed_start = Instant::now();
        let mut typed_sum = 0.0f64;
        unsafe {
            let obj = &*typed_ptr;
            for _ in 0..ITERATIONS {
                // Access all 6 fields
                typed_sum += unbox_number(obj.get_field(0));
                typed_sum += unbox_number(obj.get_field(8));
                typed_sum += unbox_number(obj.get_field(16));
                typed_sum += unbox_number(obj.get_field(24));
                typed_sum += unbox_number(obj.get_field(32));
                typed_sum += unbox_number(obj.get_field(40));
            }
        }
        let typed_duration = typed_start.elapsed();

        // Benchmark HashMap access
        let hashmap_start = Instant::now();
        let mut hashmap_sum = 0.0f64;
        for _ in 0..ITERATIONS {
            // Access all 6 fields
            hashmap_sum += unbox_number(*hashmap.get("field0").unwrap());
            hashmap_sum += unbox_number(*hashmap.get("field1").unwrap());
            hashmap_sum += unbox_number(*hashmap.get("field2").unwrap());
            hashmap_sum += unbox_number(*hashmap.get("field3").unwrap());
            hashmap_sum += unbox_number(*hashmap.get("field4").unwrap());
            hashmap_sum += unbox_number(*hashmap.get("field5").unwrap());
        }
        let hashmap_duration = hashmap_start.elapsed();

        // Verify correctness (sums should be equal)
        assert!((typed_sum - hashmap_sum).abs() < 0.001, "Sums don't match!");

        // Calculate performance metrics
        let typed_ns_per_access =
            typed_duration.as_nanos() as f64 / (ITERATIONS * FIELD_COUNT) as f64;
        let hashmap_ns_per_access =
            hashmap_duration.as_nanos() as f64 / (ITERATIONS * FIELD_COUNT) as f64;
        let speedup = hashmap_ns_per_access / typed_ns_per_access;

        eprintln!("\n=== TypedObject vs HashMap Performance ===");
        eprintln!(
            "Iterations: {} x {} fields = {} accesses",
            ITERATIONS,
            FIELD_COUNT,
            ITERATIONS * FIELD_COUNT
        );
        eprintln!("TypedObject: {:.2}ns per field access", typed_ns_per_access);
        eprintln!(
            "HashMap:     {:.2}ns per field access",
            hashmap_ns_per_access
        );
        eprintln!("Speedup:     {:.1}x faster", speedup);
        eprintln!("============================================\n");

        // Assert minimum expected speedup (should be at least 3x faster)
        assert!(
            speedup > 3.0,
            "Expected at least 3x speedup, got {:.1}x",
            speedup
        );

        // Clean up
        unsafe {
            let total_size = TYPED_OBJECT_HEADER_SIZE + schema.data_size;
            let layout = Layout::from_size_align(total_size, TYPED_OBJECT_ALIGNMENT).unwrap();
            dealloc(typed_ptr as *mut u8, layout);
        }
    }
}
