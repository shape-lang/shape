//! Trait object operations for the VM executor
//!
//! Handles: BoxTraitObject, DynMethodCall

use crate::{
    bytecode::{Instruction, OpCode, Operand},
    executor::VirtualMachine,
};
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, VTable, VTableEntry, ValueWord};
use std::collections::HashMap;
use std::sync::Arc;

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_trait_object_ops(
        &mut self,
        instruction: &Instruction,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::BoxTraitObject => self.op_box_trait_object(instruction)?,
            OpCode::DynMethodCall => self.op_dyn_method_call()?,
            OpCode::DropCall => self.op_drop_call_sync()?,
            OpCode::DropCallAsync => self.op_drop_call_async()?,
            _ => unreachable!(
                "exec_trait_object_ops called with non-trait-object opcode: {:?}",
                instruction.opcode
            ),
        }
        Ok(())
    }

    /// Box a concrete value into a trait object.
    ///
    /// Operand: Property(u16) — constant pool index for the trait name string.
    /// Stack: [concrete_value] -> [trait_object]
    fn op_box_trait_object(&mut self, instruction: &Instruction) -> Result<(), VMError> {
        let trait_name_idx = match &instruction.operand {
            Some(Operand::Property(idx)) => *idx,
            _ => {
                return Err(VMError::RuntimeError(
                    "BoxTraitObject requires a Property operand (trait name index)".to_string(),
                ));
            }
        };

        // Look up the trait name in the string pool (Property operand = string pool index)
        let trait_name = self
            .program
            .strings
            .get(trait_name_idx as usize)
            .cloned()
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "BoxTraitObject: string pool index {} out of bounds",
                    trait_name_idx
                ))
            })?;

        let value_nb = self.pop_vw()?;

        let vtable = VTable {
            trait_names: vec![trait_name],
            methods: HashMap::new(),
        };

        self.push_vw(ValueWord::from_heap_value(
            shape_value::heap_value::HeapValue::TraitObject {
                value: Box::new(value_nb),
                vtable: Arc::new(vtable),
            },
        ))?;

        Ok(())
    }

    /// Call a method on a trait object via vtable dispatch.
    ///
    /// Follows the same stack convention as CallMethod:
    /// Stack: [trait_object, arg1, ..., argN, method_name, arg_count]
    fn op_dyn_method_call(&mut self) -> Result<(), VMError> {
        // Pop arg count and method name from stack (same as CallMethod)
        let arg_count_nb = self.pop_vw()?;
        let arg_count = arg_count_nb.as_number_coerce().ok_or_else(|| {
            VMError::RuntimeError("DynMethodCall: expected number for arg count".to_string())
        })? as usize;

        let method_name_nb = self.pop_vw()?;
        let method_name = match method_name_nb.as_str() {
            Some(s) => s.to_string(),
            None => {
                return Err(VMError::TypeError {
                    expected: "string",
                    got: method_name_nb.type_name(),
                });
            }
        };

        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            args.push(self.pop_vw()?);
        }
        args.reverse();

        let receiver = self.pop_vw()?;

        match receiver.as_heap_ref() {
            Some(HeapValue::TraitObject { value, vtable }) => {
                let concrete_kind = value
                    .heap_kind()
                    .map(|k| k as u8)
                    .unwrap_or(value.tag() as u8);
                let method_hash = {
                    let mut h: u32 = 5381;
                    for b in method_name.bytes() {
                        h = h.wrapping_mul(31).wrapping_add(b as u32);
                    }
                    h
                };

                // IC fast path: if this site is monomorphic, skip vtable HashMap lookup.
                let ic_ip = self.ip;
                if let Some(hit) = crate::executor::ic_fast_paths::dyn_method_ic_check(
                    self,
                    ic_ip,
                    concrete_kind,
                    method_hash,
                ) {
                    let callee = ValueWord::from_function(hit.function_id);
                    let mut all_args = Vec::with_capacity(1 + args.len());
                    all_args.push(value.as_ref().clone());
                    all_args.extend(args);
                    let result = self.call_value_immediate_nb(&callee, &all_args, None)?;
                    self.push_vw(result)?;
                    return Ok(());
                }

                // Full vtable lookup
                if let Some(entry) = vtable.methods.get(&method_name) {
                    let mut all_args = Vec::with_capacity(1 + args.len());
                    all_args.push(value.as_ref().clone());
                    all_args.extend(args);

                    let (callee, resolved_func_id) = match entry {
                        VTableEntry::FunctionId(func_id) => {
                            (ValueWord::from_function(*func_id), *func_id)
                        }
                        VTableEntry::Closure {
                            function_id,
                            upvalues,
                        } => (
                            ValueWord::from_heap_value(HeapValue::Closure {
                                function_id: *function_id,
                                upvalues: upvalues.clone(),
                            }),
                            *function_id,
                        ),
                    };

                    // Record IC with resolved function_id for future fast-path hits.
                    crate::executor::ic_fast_paths::dyn_method_ic_record(
                        self,
                        ic_ip,
                        concrete_kind,
                        method_hash,
                        resolved_func_id,
                    );

                    let result = self.call_value_immediate_nb(&callee, &all_args, None)?;
                    self.push_vw(result)?;
                } else {
                    return Err(VMError::RuntimeError(format!(
                        "method '{}' not found in vtable for dyn {}",
                        method_name,
                        vtable.trait_names.join(" + ")
                    )));
                }
            }
            _ => {
                return Err(VMError::RuntimeError(format!(
                    "DynMethodCall on non-trait-object value: {}",
                    receiver.type_name()
                )));
            }
        }

        Ok(())
    }

    /// Sync drop: look up `TypeName::drop`.
    fn op_drop_call_sync(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        if let Some(HeapValue::IoHandle(handle_data)) = value.as_heap_ref() {
            handle_data.close();
            return Ok(());
        }
        let type_name = value.type_name();
        let drop_fn_name = format!("{}::drop", type_name);
        self.do_drop_call(value, &drop_fn_name)
    }

    /// Async drop: look up `TypeName::drop_async`, falling back to `TypeName::drop`.
    fn op_drop_call_async(&mut self) -> Result<(), VMError> {
        let value = self.pop_vw()?;
        if let Some(HeapValue::IoHandle(handle_data)) = value.as_heap_ref() {
            handle_data.close();
            return Ok(());
        }
        let type_name = value.type_name();
        let async_fn_name = format!("{}::drop_async", type_name);
        // Prefer async variant; fall back to sync if not found
        if self.function_name_index.contains_key(&async_fn_name) {
            self.do_drop_call(value, &async_fn_name)
        } else {
            let sync_fn_name = format!("{}::drop", type_name);
            self.do_drop_call(value, &sync_fn_name)
        }
    }

    /// Shared drop execution: push value, call the named drop function, swallow errors.
    fn do_drop_call(&mut self, value: ValueWord, drop_fn_name: &str) -> Result<(), VMError> {
        if let Some(&func_idx) = self.function_name_index.get(drop_fn_name) {
            self.push_vw(value)?;
            match self.call_function_from_stack(func_idx, 1) {
                Ok(()) => {
                    let _ = self.pop_vw();
                }
                Err(e) => {
                    eprintln!("[drop] Error in {}: {}", drop_fn_name, e);
                }
            }
        }
        // If no drop function exists, silently skip (type doesn't impl Drop)
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::bytecode::{Instruction, OpCode, Operand};
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::{VMError, VTable, VTableEntry, ValueWord};
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn test_vtable_creation() {
        let mut methods = HashMap::new();
        methods.insert("to_string".to_string(), VTableEntry::FunctionId(42));
        methods.insert("draw".to_string(), VTableEntry::FunctionId(7));

        let vtable = VTable {
            trait_names: vec!["Drawable".to_string()],
            methods,
        };

        assert_eq!(vtable.trait_names.len(), 1);
        assert_eq!(vtable.trait_names[0], "Drawable");
        assert!(matches!(
            vtable.methods.get("draw"),
            Some(VTableEntry::FunctionId(7))
        ));
        assert!(matches!(
            vtable.methods.get("to_string"),
            Some(VTableEntry::FunctionId(42))
        ));
        assert!(vtable.methods.get("nonexistent").is_none());
    }

    #[test]
    fn test_trait_object_value_wrapping() {
        let concrete = ValueWord::from_i64(42);
        let vtable = Arc::new(VTable {
            trait_names: vec!["Display".to_string()],
            methods: HashMap::new(),
        });

        let trait_obj = ValueWord::from_trait_object(concrete, vtable);

        assert_eq!(trait_obj.type_name(), "trait_object");

        if let Some((value, vt)) = trait_obj.as_trait_object() {
            assert_eq!(value.as_i64(), Some(42));
            assert_eq!(vt.trait_names, vec!["Display".to_string()]);
        } else {
            panic!("Expected TraitObject");
        }
    }

    #[test]
    fn test_multi_trait_vtable() {
        let mut methods = HashMap::new();
        methods.insert("draw".to_string(), VTableEntry::FunctionId(1));
        methods.insert("serialize".to_string(), VTableEntry::FunctionId(2));

        let vtable = VTable {
            trait_names: vec!["Drawable".to_string(), "Serializable".to_string()],
            methods,
        };

        assert_eq!(vtable.trait_names.len(), 2);
        assert_eq!(vtable.methods.len(), 2);
    }

    #[test]
    fn test_box_trait_object_instruction() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        // Add "Display" to the string pool at index 0
        vm.program.strings.push("Display".to_string());

        // Push a concrete value onto the stack
        vm.push_value(ValueWord::from_i64(42));

        let instr = Instruction {
            opcode: OpCode::BoxTraitObject,
            operand: Some(Operand::Property(0)), // string pool index 0 = "Display"
        };

        vm.exec_trait_object_ops(&instr, None).unwrap();

        // The top of stack should now be a TraitObject
        let result = vm.pop().unwrap();
        if let Some((value, vtable)) = result.as_trait_object() {
            assert_eq!(value.as_i64(), Some(42));
            assert_eq!(vtable.trait_names, vec!["Display".to_string()]);
            assert!(vtable.methods.is_empty());
        } else {
            panic!("Expected TraitObject, got {:?}", result);
        }
    }

    #[test]
    fn test_box_trait_object_wraps_string_value() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.program.strings.push("Serializable".to_string());

        vm.push_value(ValueWord::from_string(Arc::new("hello".to_string())));

        let instr = Instruction {
            opcode: OpCode::BoxTraitObject,
            operand: Some(Operand::Property(0)),
        };

        vm.exec_trait_object_ops(&instr, None).unwrap();

        let result = vm.pop().unwrap();
        if let Some((value, vtable)) = result.as_trait_object() {
            assert_eq!(
                value.as_arc_string().map(|s| s.as_ref().to_string()),
                Some("hello".to_string())
            );
            assert_eq!(vtable.trait_names, vec!["Serializable".to_string()]);
        } else {
            panic!("Expected TraitObject, got {:?}", result);
        }
    }

    #[test]
    fn test_box_trait_object_missing_operand() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.push_value(ValueWord::from_i64(1));

        let instr = Instruction {
            opcode: OpCode::BoxTraitObject,
            operand: None,
        };

        let result = vm.exec_trait_object_ops(&instr, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_box_trait_object_empty_stack() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.program.strings.push("Display".to_string());

        let instr = Instruction {
            opcode: OpCode::BoxTraitObject,
            operand: Some(Operand::Property(0)),
        };

        let result = vm.exec_trait_object_ops(&instr, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_box_trait_object_invalid_string_index() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        // String pool is empty, so index 99 is out of bounds
        vm.push_value(ValueWord::from_i64(1));

        let instr = Instruction {
            opcode: OpCode::BoxTraitObject,
            operand: Some(Operand::Property(99)),
        };

        let result = vm.exec_trait_object_ops(&instr, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_dyn_method_call_missing_method() {
        let mut vm = VirtualMachine::new(VMConfig::default());

        // Create a trait object with no methods in vtable
        let vtable = Arc::new(VTable {
            trait_names: vec!["Display".to_string()],
            methods: HashMap::new(),
        });
        let trait_obj = ValueWord::from_trait_object(ValueWord::from_i64(42), vtable);

        // Stack layout: [trait_object, method_name, arg_count]
        vm.push_value(trait_obj);
        vm.push_value(ValueWord::from_string(Arc::new("to_string".to_string())));
        vm.push_value(ValueWord::from_f64(0.0)); // 0 args

        let instr = Instruction {
            opcode: OpCode::DynMethodCall,
            operand: None,
        };

        let result = vm.exec_trait_object_ops(&instr, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            VMError::RuntimeError(msg) => {
                assert!(
                    msg.contains("to_string"),
                    "Error should mention method name: {}",
                    msg
                );
                assert!(
                    msg.contains("Display"),
                    "Error should mention trait name: {}",
                    msg
                );
            }
            other => panic!("Expected RuntimeError, got {:?}", other),
        }
    }

    #[test]
    fn test_dyn_method_call_on_non_trait_object() {
        let mut vm = VirtualMachine::new(VMConfig::default());

        // Push a plain value (not a trait object)
        vm.push_value(ValueWord::from_i64(42));
        vm.push_value(ValueWord::from_string(Arc::new("method".to_string())));
        vm.push_value(ValueWord::from_f64(0.0)); // 0 args

        let instr = Instruction {
            opcode: OpCode::DynMethodCall,
            operand: None,
        };

        let result = vm.exec_trait_object_ops(&instr, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            VMError::RuntimeError(msg) => {
                assert!(
                    msg.contains("non-trait-object"),
                    "Error should mention non-trait-object: {}",
                    msg
                );
            }
            other => panic!("Expected RuntimeError, got {:?}", other),
        }
    }

    #[test]
    fn test_vtable_closure_entry() {
        let mut methods = HashMap::new();
        methods.insert(
            "render".to_string(),
            VTableEntry::Closure {
                function_id: 5,
                upvalues: vec![],
            },
        );

        let vtable = VTable {
            trait_names: vec!["Renderable".to_string()],
            methods,
        };

        match vtable.methods.get("render") {
            Some(VTableEntry::Closure {
                function_id,
                upvalues,
            }) => {
                assert_eq!(*function_id, 5);
                assert!(upvalues.is_empty());
            }
            other => panic!("Expected Closure entry, got {:?}", other),
        }
    }

    #[test]
    fn test_drop_call_no_impl_silently_skips() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        // Push a value with no Drop impl registered
        vm.push_value(ValueWord::from_i64(42));

        let instr = Instruction {
            opcode: OpCode::DropCall,
            operand: None,
        };

        // Should succeed — no drop impl found, silently skips
        vm.exec_trait_object_ops(&instr, None).unwrap();
    }

    #[test]
    fn test_drop_call_empty_stack_errors() {
        let mut vm = VirtualMachine::new(VMConfig::default());

        let instr = Instruction {
            opcode: OpCode::DropCall,
            operand: None,
        };

        // Should error — stack is empty
        let result = vm.exec_trait_object_ops(&instr, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_drop_call_io_handle_closes_file() {
        use shape_value::heap_value::IoHandleData;

        let mut vm = VirtualMachine::new(VMConfig::default());

        // Create a temp file and open it
        let dir = std::env::temp_dir().join("shape_drop_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("drop_test.txt");
        std::fs::write(&path, "test").unwrap();

        let file = std::fs::File::open(&path).unwrap();
        let handle =
            IoHandleData::new_file(file, path.to_string_lossy().to_string(), "r".to_string());
        assert!(handle.is_open());

        let nb = ValueWord::from_io_handle(handle.clone());

        // Push and drop
        vm.push_value(nb);
        let instr = Instruction {
            opcode: OpCode::DropCall,
            operand: None,
        };
        vm.exec_trait_object_ops(&instr, None).unwrap();

        // After DropCall, the handle should be closed
        assert!(!handle.is_open());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_drop_call_io_handle_closes_tcp_listener() {
        use shape_value::heap_value::IoHandleData;

        let mut vm = VirtualMachine::new(VMConfig::default());

        // Bind a TCP listener
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let handle = IoHandleData::new_tcp_listener(listener, "127.0.0.1:0".to_string());
        assert!(handle.is_open());

        let nb = ValueWord::from_io_handle(handle.clone());

        // Push and drop
        vm.push_value(nb);
        let instr = Instruction {
            opcode: OpCode::DropCall,
            operand: None,
        };
        vm.exec_trait_object_ops(&instr, None).unwrap();

        // After DropCall, the handle should be closed
        assert!(!handle.is_open());
    }

    #[test]
    fn test_drop_call_io_handle_double_drop_safe() {
        use shape_value::heap_value::IoHandleData;

        let mut vm = VirtualMachine::new(VMConfig::default());

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let handle = IoHandleData::new_tcp_listener(listener, "127.0.0.1:0".to_string());
        let nb = ValueWord::from_io_handle(handle.clone());

        // Drop once
        vm.push_value(nb.clone());
        let instr = Instruction {
            opcode: OpCode::DropCall,
            operand: None,
        };
        vm.exec_trait_object_ops(&instr, None).unwrap();
        assert!(!handle.is_open());

        // Drop again — should not error (close returns false, no panic)
        vm.push_value(nb);
        vm.exec_trait_object_ops(&instr, None).unwrap();
    }

    #[test]
    fn test_drop_call_io_handle_closes_udp_socket() {
        use shape_value::heap_value::IoHandleData;

        let mut vm = VirtualMachine::new(VMConfig::default());

        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let handle = IoHandleData::new_udp_socket(socket, "127.0.0.1:0".to_string());
        assert!(handle.is_open());

        let nb = ValueWord::from_io_handle(handle.clone());

        vm.push_value(nb);
        let instr = Instruction {
            opcode: OpCode::DropCall,
            operand: None,
        };
        vm.exec_trait_object_ops(&instr, None).unwrap();

        assert!(!handle.is_open());
    }
}
