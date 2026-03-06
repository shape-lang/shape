use super::*;
use crate::bytecode::{OpCode, SourceMap};
use shape_abi_v1::PermissionSet;

/// Helper: build a minimal FunctionBlob.
fn make_blob(
    name: &str,
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    strings: Vec<String>,
    dependencies: Vec<FunctionHash>,
) -> FunctionBlob {
    let mut blob = FunctionBlob {
        content_hash: FunctionHash::ZERO,
        name: name.to_string(),
        arity: 0,
        param_names: vec![],
        locals_count: 0,
        is_closure: false,
        captures_count: 0,
        is_async: false,
        ref_params: vec![],
        ref_mutates: vec![],
        mutable_captures: vec![],
        instructions,
        constants,
        strings,
        dependencies,
        callee_names: vec![],
        type_schemas: vec![],
        source_map: vec![],
        foreign_dependencies: vec![],
        required_permissions: PermissionSet::pure(),
    };
    blob.finalize();
    blob
}

/// Build a minimal Program from blobs with a specified entry hash.
fn make_program(blobs: Vec<FunctionBlob>, entry: FunctionHash) -> Program {
    let mut store = HashMap::new();
    for b in blobs {
        store.insert(b.content_hash, b);
    }
    Program {
        entry,
        function_store: store,
        top_level_locals_count: 0,
        top_level_local_storage_hints: vec![],
        module_binding_names: vec![],
        module_binding_storage_hints: vec![],
        function_local_storage_hints: vec![],
        top_level_frame: None,
        data_schema: None,
        type_schema_registry: Default::default(),
        trait_method_symbols: HashMap::new(),
        foreign_functions: vec![],
        native_struct_layouts: vec![],
        debug_info: DebugInfo {
            source_map: SourceMap {
                files: vec!["test.shape".into()],
                source_texts: vec![],
            },
            line_numbers: vec![],
            variable_names: vec![],
            source_text: String::new(),
        },
    }
}

#[test]
fn test_link_single_function() {
    let blob = make_blob(
        "main",
        vec![
            Instruction {
                opcode: OpCode::PushConst,
                operand: Some(Operand::Const(0)),
            },
            Instruction {
                opcode: OpCode::Return,
                operand: None,
            },
        ],
        vec![Constant::Number(42.0)],
        vec!["hello".into()],
        vec![],
    );
    let entry = blob.content_hash;
    let program = make_program(vec![blob], entry);

    let linked = link(&program).expect("link should succeed");

    assert_eq!(linked.functions.len(), 1);
    assert_eq!(linked.functions[0].entry_point, 0);
    assert_eq!(linked.instructions.len(), 2);
    assert_eq!(linked.constants.len(), 1);
    assert_eq!(linked.strings.len(), 1);
    assert_eq!(linked.strings[0], "hello");
    assert_eq!(linked.entry, entry);
    assert_eq!(linked.hash_to_id[&entry], 0);
}

#[test]
fn test_link_two_functions_with_dependency() {
    // `leaf` has no dependencies.
    let leaf = make_blob(
        "leaf",
        vec![Instruction {
            opcode: OpCode::PushConst,
            operand: Some(Operand::Const(0)),
        }],
        vec![Constant::Number(1.0)],
        vec!["a".into()],
        vec![],
    );
    let leaf_hash = leaf.content_hash;

    // `main` depends on `leaf` (dep index 0).
    let main = make_blob(
        "main",
        vec![
            Instruction {
                opcode: OpCode::PushConst,
                operand: Some(Operand::Const(0)),
            },
            Instruction {
                opcode: OpCode::Call,
                operand: Some(Operand::Function(FunctionId(0))),
            },
        ],
        vec![Constant::Number(2.0)],
        vec!["b".into()],
        vec![leaf_hash],
    );
    let main_hash = main.content_hash;

    let program = make_program(vec![leaf, main], main_hash);
    let linked = link(&program).expect("link should succeed");

    assert_eq!(linked.functions.len(), 2);
    // Leaf should come first (dependency-first order).
    assert_eq!(linked.functions[0].name, "leaf");
    assert_eq!(linked.functions[1].name, "main");
    assert_eq!(linked.entry, main_hash);

    // main's const operand should be remapped from 0 to 1
    // (leaf has 1 constant, so const_base for main = 1).
    let main_load_const = &linked.instructions[linked.functions[1].entry_point];
    assert_eq!(main_load_const.operand, Some(Operand::Const(1)));

    // main's function operand should point to leaf's linked ID (0).
    let main_call = &linked.instructions[linked.functions[1].entry_point + 1];
    assert_eq!(main_call.operand, Some(Operand::Function(FunctionId(0))));
}

#[test]
fn test_link_self_recursive_zero_dependency_maps_to_self() {
    let fib = make_blob(
        "fib",
        vec![
            Instruction {
                opcode: OpCode::Call,
                operand: Some(Operand::Function(FunctionId(0))),
            },
            Instruction {
                opcode: OpCode::Return,
                operand: None,
            },
        ],
        vec![],
        vec![],
        vec![FunctionHash::ZERO],
    );
    let fib_hash = fib.content_hash;
    let program = make_program(vec![fib], fib_hash);

    let linked = link(&program).expect("link should succeed for self-recursive sentinel");

    assert_eq!(linked.functions.len(), 1);
    assert_eq!(linked.functions[0].name, "fib");
    let call = &linked.instructions[linked.functions[0].entry_point];
    assert_eq!(call.operand, Some(Operand::Function(FunctionId(0))));
}

#[test]
fn test_link_circular_dependency_detected() {
    // Two blobs that depend on each other.
    // We can't use finalize normally since the hashes depend on dependencies,
    // so we construct them with known hashes.
    let hash_a = FunctionHash([1u8; 32]);
    let hash_b = FunctionHash([2u8; 32]);

    let blob_a = FunctionBlob {
        content_hash: hash_a,
        name: "a".into(),
        arity: 0,
        param_names: vec![],
        locals_count: 0,
        is_closure: false,
        captures_count: 0,
        is_async: false,
        ref_params: vec![],
        ref_mutates: vec![],
        mutable_captures: vec![],
        instructions: vec![],
        constants: vec![],
        strings: vec![],
        dependencies: vec![hash_b],
        callee_names: vec![],
        type_schemas: vec![],
        source_map: vec![],
        foreign_dependencies: vec![],
        required_permissions: PermissionSet::pure(),
    };
    let blob_b = FunctionBlob {
        content_hash: hash_b,
        name: "b".into(),
        arity: 0,
        param_names: vec![],
        locals_count: 0,
        is_closure: false,
        captures_count: 0,
        is_async: false,
        ref_params: vec![],
        ref_mutates: vec![],
        mutable_captures: vec![],
        instructions: vec![],
        constants: vec![],
        strings: vec![],
        dependencies: vec![hash_a],
        callee_names: vec![],
        type_schemas: vec![],
        source_map: vec![],
        foreign_dependencies: vec![],
        required_permissions: PermissionSet::pure(),
    };

    let program = make_program(vec![blob_a, blob_b], hash_a);
    let result = link(&program);
    assert!(matches!(result, Err(LinkError::CircularDependency)));
}

#[test]
fn test_link_missing_blob() {
    let missing = FunctionHash([99u8; 32]);
    let blob = FunctionBlob {
        content_hash: FunctionHash([1u8; 32]),
        name: "main".into(),
        arity: 0,
        param_names: vec![],
        locals_count: 0,
        is_closure: false,
        captures_count: 0,
        is_async: false,
        ref_params: vec![],
        ref_mutates: vec![],
        mutable_captures: vec![],
        instructions: vec![],
        constants: vec![],
        strings: vec![],
        dependencies: vec![missing],
        callee_names: vec![],
        type_schemas: vec![],
        source_map: vec![],
        foreign_dependencies: vec![],
        required_permissions: PermissionSet::pure(),
    };

    let program = make_program(vec![blob], FunctionHash([1u8; 32]));
    let result = link(&program);
    assert!(matches!(result, Err(LinkError::MissingBlob(_))));
}

#[test]
fn test_linked_to_bytecode_roundtrip() {
    let blob = make_blob(
        "main",
        vec![Instruction {
            opcode: OpCode::PushConst,
            operand: Some(Operand::Const(0)),
        }],
        vec![Constant::Number(42.0)],
        vec!["test".into()],
        vec![],
    );
    let entry = blob.content_hash;
    let program = make_program(vec![blob], entry);

    let linked = link(&program).expect("link");
    let bytecode = linked_to_bytecode_program(&linked);

    assert_eq!(bytecode.functions.len(), 1);
    assert_eq!(bytecode.functions[0].name, "main");
    assert_eq!(bytecode.functions[0].entry_point, 0);
    assert_eq!(bytecode.instructions.len(), 1);
    assert_eq!(bytecode.constants.len(), 1);
    assert_eq!(bytecode.strings, vec!["test".to_string()]);
}

#[test]
fn test_string_operand_remapping() {
    let leaf = make_blob(
        "leaf",
        vec![Instruction {
            opcode: OpCode::GetProp,
            operand: Some(Operand::Property(0)),
        }],
        vec![],
        vec!["x".into()],
        vec![],
    );
    let leaf_hash = leaf.content_hash;

    let main = make_blob(
        "main",
        vec![
            Instruction {
                opcode: OpCode::GetProp,
                operand: Some(Operand::Property(0)),
            },
            Instruction {
                opcode: OpCode::GetProp,
                operand: Some(Operand::Name(StringId(0))),
            },
        ],
        vec![],
        vec!["y".into()],
        vec![leaf_hash],
    );
    let main_hash = main.content_hash;

    let program = make_program(vec![leaf, main], main_hash);
    let linked = link(&program).expect("link");

    // leaf's Property(0) stays 0 (string_base = 0)
    assert_eq!(linked.instructions[0].operand, Some(Operand::Property(0)));

    // main's Property(0) becomes Property(1) (string_base = 1)
    let main_entry = linked.functions[1].entry_point;
    assert_eq!(
        linked.instructions[main_entry].operand,
        Some(Operand::Property(1))
    );

    // main's Name(StringId(0)) becomes Name(StringId(1))
    assert_eq!(
        linked.instructions[main_entry + 1].operand,
        Some(Operand::Name(StringId(1)))
    );
}

#[test]
fn test_source_map_merging() {
    // Build blobs with source maps manually (make_blob doesn't support source_map).
    let leaf_hash;
    let leaf = {
        let mut b = FunctionBlob {
            content_hash: FunctionHash::ZERO,
            name: "leaf".into(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            instructions: vec![
                Instruction {
                    opcode: OpCode::Return,
                    operand: None,
                },
                Instruction {
                    opcode: OpCode::Return,
                    operand: None,
                },
            ],
            constants: vec![],
            strings: vec![],
            dependencies: vec![],
            callee_names: vec![],
            type_schemas: vec![],
            source_map: vec![(0, 0, 10), (1, 0, 11)],
            foreign_dependencies: vec![],
            required_permissions: PermissionSet::pure(),
        };
        b.finalize();
        leaf_hash = b.content_hash;
        b
    };

    let main = {
        let mut b = FunctionBlob {
            content_hash: FunctionHash::ZERO,
            name: "main".into(),
            arity: 0,
            param_names: vec![],
            locals_count: 0,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            instructions: vec![Instruction {
                opcode: OpCode::Return,
                operand: None,
            }],
            constants: vec![],
            strings: vec![],
            dependencies: vec![leaf_hash],
            callee_names: vec![],
            type_schemas: vec![],
            source_map: vec![(0, 0, 20)],
            foreign_dependencies: vec![],
            required_permissions: PermissionSet::pure(),
        };
        b.finalize();
        b
    };
    let main_hash = main.content_hash;

    let program = make_program(vec![leaf, main], main_hash);
    let linked = link(&program).expect("link");

    // leaf occupies instructions [0, 1], main occupies [2].
    // Merged line numbers should be:
    //   (0, 0, 10), (1, 0, 11) from leaf
    //   (2, 0, 20) from main (entry_point=2, local_offset=0)
    assert_eq!(linked.debug_info.line_numbers.len(), 3);
    assert_eq!(linked.debug_info.line_numbers[0], (0, 0, 10));
    assert_eq!(linked.debug_info.line_numbers[1], (1, 0, 11));
    assert_eq!(linked.debug_info.line_numbers[2], (2, 0, 20));
}

#[test]
fn test_vm_starts_linked_program_at_entry_function() {
    let leaf = make_blob(
        "leaf",
        vec![
            Instruction {
                opcode: OpCode::PushConst,
                operand: Some(Operand::Const(0)),
            },
            Instruction {
                opcode: OpCode::Halt,
                operand: None,
            },
        ],
        vec![Constant::Number(1.0)],
        vec![],
        vec![],
    );
    let leaf_hash = leaf.content_hash;

    let main = make_blob(
        "main",
        vec![
            Instruction {
                opcode: OpCode::PushConst,
                operand: Some(Operand::Const(0)),
            },
            Instruction {
                opcode: OpCode::Halt,
                operand: None,
            },
        ],
        vec![Constant::Number(42.0)],
        vec![],
        vec![leaf_hash],
    );
    let main_hash = main.content_hash;

    let program = make_program(vec![leaf, main], main_hash);
    let linked = link(&program).expect("link should succeed");

    // Dependency order places `leaf` first, but execution must start at `main`.
    assert_eq!(linked.functions[0].name, "leaf");
    assert_eq!(linked.functions[1].name, "main");
    assert_eq!(linked.entry, main_hash);

    let mut vm = crate::executor::VirtualMachine::new(crate::executor::VMConfig::default());
    vm.load_linked_program(linked);
    let result = vm.execute(None).expect("execution should succeed");
    assert_eq!(result.as_number_coerce(), Some(42.0));
}
