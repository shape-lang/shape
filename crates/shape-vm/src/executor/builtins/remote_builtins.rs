//! Native `remote` module for executing Shape code on remote `shape serve` instances.
//!
//! Provides a high-level abstraction over the wire protocol so users can
//! execute code or call functions on a remote Shape server directly from
//! Shape code, without manually encoding wire messages.
//!
//! Exports:
//! - remote.execute(addr, code) -> Result<{ value, stdout, error }, string>
//! - remote.ping(addr) -> Result<{ shape_version: string, wire_protocol: int }, string>
//! - remote.__call(addr, fn_ref, args) -> Result<_, string>

use shape_runtime::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_runtime::wire_conversion::wire_to_nb;
use shape_value::{ValueWord, ValueWordExt};
use shape_wire::transport::Transport;
use shape_wire::transport::factory::TransportKind;
use std::cell::RefCell;
use std::sync::Arc;

use super::transport_provider;

// ---------------------------------------------------------------------------
// Thread-local program reference for remote.__call()
// ---------------------------------------------------------------------------

thread_local! {
    /// The current BytecodeProgram, set by the VM before dispatching module
    /// functions. Used by `remote.__call()` to build RemoteCallRequests.
    static CURRENT_PROGRAM: RefCell<Option<crate::bytecode::BytecodeProgram>> = const { RefCell::new(None) };
}

/// Set the thread-local program reference. Called by the VM before module dispatch.
pub fn set_current_program(program: &crate::bytecode::BytecodeProgram) {
    CURRENT_PROGRAM.with(|p| {
        *p.borrow_mut() = Some(program.clone());
    });
}

/// Clear the thread-local program reference. Called by the VM after module dispatch.
pub fn clear_current_program() {
    CURRENT_PROGRAM.with(|p| {
        *p.borrow_mut() = None;
    });
}

/// Build a Shape HashMap from string key-value pairs.
fn make_object(fields: Vec<(&str, ValueWord)>) -> ValueWord {
    let keys: Vec<ValueWord> = fields
        .iter()
        .map(|(k, _)| ValueWord::from_string(Arc::new(k.to_string())))
        .collect();
    let values: Vec<ValueWord> = fields.into_iter().map(|(_, v)| v).collect();
    ValueWord::from_hashmap_pairs(keys, values)
}

/// Create the `remote` module with remote execution functions.
pub fn create_remote_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::remote");
    module.description = "Remote execution on Shape serve instances".to_string();

    // remote.execute(addr, code) -> Result<{ value, stdout, error }, string>
    module.add_function_with_schema(
        "execute",
        remote_execute,
        ModuleFunction {
            description: "Execute Shape code on a remote server".to_string(),
            params: vec![
                ModuleParam {
                    name: "addr".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Remote server address as host:port".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "code".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Shape source code to execute remotely".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some(
                "Result<{ value, stdout: string?, error: string? }, string>".to_string(),
            ),
        },
    );

    // remote.ping(addr) -> Result<{ shape_version: string, wire_protocol: int }, string>
    module.add_function_with_schema(
        "ping",
        remote_ping,
        ModuleFunction {
            description: "Ping a remote Shape server and get server info".to_string(),
            params: vec![ModuleParam {
                name: "addr".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Remote server address as host:port".to_string(),
                ..Default::default()
            }],
            return_type: Some(
                "Result<{ shape_version: string, wire_protocol: int }, string>".to_string(),
            ),
        },
    );

    // remote.__call(addr, fn_ref, args) -> Result<_, string>
    module.add_function_with_schema(
        "__call",
        remote_call,
        ModuleFunction {
            description: "Call a function on a remote Shape server".to_string(),
            params: vec![
                ModuleParam {
                    name: "addr".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Remote server address as host:port".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "fn_ref".to_string(),
                    type_name: "Function".to_string(),
                    required: true,
                    description: "Function reference to call remotely".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "args".to_string(),
                    type_name: "Array<_>".to_string(),
                    required: true,
                    description: "Arguments to pass to the remote function".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Result<_, string>".to_string()),
        },
    );

    module
}

// ---------------------------------------------------------------------------
// Wire protocol helpers
// ---------------------------------------------------------------------------

/// Send a WireMessage to a remote server and receive the response.
fn wire_roundtrip(
    addr: &str,
    msg: &crate::remote::WireMessage,
) -> Result<crate::remote::WireMessage, String> {
    let transport = transport_provider::transport_provider()
        .create_transport(TransportKind::Tcp)
        .map_err(|e| format!("remote: failed to create transport: {}", e))?;

    // Encode WireMessage to MessagePack
    let mp = shape_wire::encode_message(msg).map_err(|e| format!("remote: encode error: {}", e))?;

    // Send via transport (handles framing + length prefix internally)
    let response_bytes = transport
        .send(addr, &mp)
        .map_err(|e| format!("remote: transport error: {}", e))?;

    // Response is already deframed by transport.send()
    shape_wire::decode_message(&response_bytes).map_err(|e| format!("remote: decode error: {}", e))
}

// ---------------------------------------------------------------------------
// Module functions
// ---------------------------------------------------------------------------

/// remote.execute(addr, code) -> Result<{ value, stdout, error }, string>
///
/// Sends Shape source code to a remote `shape serve` instance for execution.
/// Returns a result object with the structured return value, captured stdout,
/// and any error message.
fn remote_execute(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;

    let addr = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "remote.execute(): first argument must be an address string".to_string())?;

    let code = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| "remote.execute(): second argument must be a code string".to_string())?;

    let msg = crate::remote::WireMessage::Execute(crate::remote::ExecuteRequest {
        code: code.to_string(),
        request_id: 1,
    });

    let response = wire_roundtrip(&addr, &msg)?;

    match response {
        crate::remote::WireMessage::ExecuteResponse(r) => {
            if r.success {
                // Convert WireValue to native Shape value
                let value = wire_to_nb(&r.value);

                let stdout = match r.stdout {
                    Some(s) => ValueWord::from_string(Arc::new(s)),
                    None => ValueWord::none(),
                };

                let obj = make_object(vec![
                    ("value", value),
                    ("stdout", stdout),
                    ("error", ValueWord::none()),
                ]);

                Ok(ValueWord::from_ok(obj))
            } else {
                let error_msg = r.error.unwrap_or_else(|| "unknown error".to_string());
                Ok(ValueWord::from_err(ValueWord::from_string(Arc::new(
                    error_msg,
                ))))
            }
        }
        other => Err(format!(
            "remote.execute(): unexpected response type: {:?}",
            std::mem::discriminant(&other)
        )),
    }
}

/// remote.ping(addr) -> Result<{ shape_version: string, wire_protocol: int }, string>
///
/// Pings a remote Shape server to check if it's alive and get server info.
fn remote_ping(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;

    let addr = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "remote.ping(): argument must be an address string".to_string())?;

    let msg = crate::remote::WireMessage::Ping(crate::remote::PingRequest {});
    let response = wire_roundtrip(&addr, &msg)?;

    match response {
        crate::remote::WireMessage::Pong(info) => {
            let obj = make_object(vec![
                (
                    "shape_version",
                    ValueWord::from_string(Arc::new(info.shape_version)),
                ),
                (
                    "wire_protocol",
                    ValueWord::from_i64(info.wire_protocol as i64),
                ),
            ]);
            Ok(ValueWord::from_ok(obj))
        }
        other => Err(format!(
            "remote.ping(): unexpected response type: {:?}",
            std::mem::discriminant(&other)
        )),
    }
}

/// ValueWord → SerializableVMValue conversion for remote calls.
///
/// Handles all common types without requiring a filesystem-backed SnapshotStore.
/// Falls back to `None` only for truly unsupported types (BlobRef-backed, IoHandle, etc.).
fn nb_to_serializable(nb: &ValueWord) -> shape_runtime::snapshot::SerializableVMValue {
    use shape_runtime::snapshot::SerializableVMValue;
    use shape_value::tag_bits::{is_tagged, get_tag, TAG_INT, TAG_BOOL, TAG_NONE, TAG_UNIT, TAG_FUNCTION, TAG_HEAP};

    let bits = nb.raw_bits();
    if !is_tagged(bits) {
        return SerializableVMValue::Number(nb.as_f64().unwrap());
    }
    match get_tag(bits) {
        TAG_INT => SerializableVMValue::Number(nb.as_f64().unwrap()),
        TAG_INT => SerializableVMValue::Int(nb.as_i64().unwrap()),
        TAG_BOOL => SerializableVMValue::Bool(nb.as_bool().unwrap()),
        TAG_NONE => SerializableVMValue::None,
        TAG_UNIT => SerializableVMValue::Unit,
        TAG_FUNCTION => SerializableVMValue::Function(nb.as_function_id().unwrap()),
        TAG_HEAP => {
            use shape_value::HeapValue;
            // cold-path: as_heap_ref retained — serialization multi-variant match
            match nb.as_heap_ref() { // cold-path
                Some(HeapValue::String(s)) => SerializableVMValue::String((**s).clone()),
                Some(HeapValue::Decimal(d)) => SerializableVMValue::Decimal(*d),
                Some(HeapValue::BigInt(i)) => SerializableVMValue::Int(*i),
                Some(HeapValue::Array(arr)) => {
                    let items: Vec<_> = arr.iter().map(|v| nb_to_serializable(v)).collect();
                    SerializableVMValue::Array(items)
                }
                _ if nb.as_closure_handle().is_some() => {
                    // Track A.2A: emit the widened closure schema
                    // (`function_id: u32` + `type_id: u32`) for cross-
                    // node transport. Receivers consult the remote
                    // program's `closure_function_layouts_by_name` /
                    // `closure_function_layouts` side-table to rebuild
                    // the raw block (A.4 finalises the reader).
                    let handle = nb.as_closure_handle().unwrap();
                    let function_id = handle.function_id();
                    let type_id = handle.type_id();
                    let ups: Vec<_> = handle
                        .captures_as_values()
                        .iter()
                        .map(nb_to_serializable)
                        .collect();
                    SerializableVMValue::Closure {
                        function_id,
                        type_id,
                        upvalues: ups,
                    }
                }
                Some(HeapValue::Some(inner)) => {
                    SerializableVMValue::Some(Box::new(nb_to_serializable(inner)))
                }
                Some(HeapValue::Ok(inner)) => {
                    SerializableVMValue::Ok(Box::new(nb_to_serializable(inner)))
                }
                Some(HeapValue::Err(inner)) => {
                    SerializableVMValue::Err(Box::new(nb_to_serializable(inner)))
                }
                Some(HeapValue::HashMap(map)) => {
                    let keys: Vec<_> = map.keys.iter().map(|k| nb_to_serializable(k)).collect();
                    let values: Vec<_> = map.values.iter().map(|v| nb_to_serializable(v)).collect();
                    SerializableVMValue::HashMap { keys, values }
                }
                Some(HeapValue::Range {
                    start,
                    end,
                    inclusive,
                }) => SerializableVMValue::Range {
                    start: start.as_ref().map(|s| Box::new(nb_to_serializable(s))),
                    end: end.as_ref().map(|e| Box::new(nb_to_serializable(e))),
                    inclusive: *inclusive,
                },
                Some(HeapValue::TypedArray(shape_value::TypedArrayData::I64(buf))) => {
                    let items: Vec<_> = buf.iter().map(|&v| SerializableVMValue::Int(v)).collect();
                    SerializableVMValue::Array(items)
                }
                Some(HeapValue::TypedArray(shape_value::TypedArrayData::F64(buf))) => {
                    let items: Vec<_> = buf
                        .as_slice()
                        .iter()
                        .map(|&v| SerializableVMValue::Number(v))
                        .collect();
                    SerializableVMValue::Array(items)
                }
                Some(HeapValue::TypedArray(shape_value::TypedArrayData::FloatSlice { parent, offset, len })) => {
                    let off = *offset as usize;
                    let slice_len = *len as usize;
                    let items: Vec<_> = parent.data[off..off + slice_len]
                        .iter()
                        .map(|&v| SerializableVMValue::Number(v))
                        .collect();
                    SerializableVMValue::Array(items)
                }
                Some(HeapValue::TypedArray(shape_value::TypedArrayData::Bool(buf))) => {
                    let items: Vec<_> = buf
                        .iter()
                        .map(|&v| SerializableVMValue::Bool(v != 0))
                        .collect();
                    SerializableVMValue::Array(items)
                }
                Some(HeapValue::TypedObject {
                    schema_id,
                    slots,
                    heap_mask,
                }) => {
                    let slot_data: Vec<_> = slots
                        .iter()
                        .enumerate()
                        .map(|(i, slot)| {
                            if *heap_mask & (1u64 << i) != 0 {
                                let vw = slot.as_value_word(true);
                                nb_to_serializable(&vw)
                            } else {
                                SerializableVMValue::Number(slot.as_f64())
                            }
                        })
                        .collect();
                    SerializableVMValue::TypedObject {
                        schema_id: *schema_id,
                        slot_data,
                        heap_mask: *heap_mask,
                    }
                }
                _ => SerializableVMValue::None,
            }
        }
        _ => SerializableVMValue::None,
    }
}

/// Lightweight SerializableVMValue → ValueWord conversion for remote call
/// responses.
///
/// Track A.4: closure payloads in a response require the caller program's
/// `closure_function_layouts` side-table to rebuild the raw
/// `TypedClosureHeader`. The `layouts` slice is passed through verbatim; a
/// missing entry for the payload's `function_id` surfaces a hard error
/// through the returned `Result` — no legacy `HeapValue::Closure`
/// fallback exists.
fn serializable_to_nb(
    sv: &shape_runtime::snapshot::SerializableVMValue,
    layouts: &[Option<Arc<shape_value::v2::closure_layout::ClosureLayout>>],
) -> Result<ValueWord, String> {
    use shape_runtime::snapshot::SerializableVMValue;

    Ok(match sv {
        SerializableVMValue::Number(n) => ValueWord::from_f64(*n),
        SerializableVMValue::Int(i) => ValueWord::from_i64(*i),
        SerializableVMValue::Bool(b) => ValueWord::from_bool(*b),
        SerializableVMValue::None => ValueWord::none(),
        SerializableVMValue::Unit => ValueWord::unit(),
        SerializableVMValue::String(s) => ValueWord::from_string(Arc::new(s.clone())),
        SerializableVMValue::Function(id) => ValueWord::from_function(*id),
        SerializableVMValue::Array(items) => {
            let mut vals = Vec::with_capacity(items.len());
            for it in items.iter() {
                vals.push(serializable_to_nb(it, layouts)?);
            }
            ValueWord::from_array(shape_value::vmarray_from_vec(vals))
        }
        SerializableVMValue::Decimal(d) => ValueWord::from_decimal(*d),
        SerializableVMValue::Some(inner) => {
            ValueWord::from_some(serializable_to_nb(inner, layouts)?)
        }
        SerializableVMValue::Ok(inner) => ValueWord::from_ok(serializable_to_nb(inner, layouts)?),
        SerializableVMValue::Err(inner) => {
            ValueWord::from_err(serializable_to_nb(inner, layouts)?)
        }
        SerializableVMValue::HashMap { keys, values } => {
            let mut k = Vec::with_capacity(keys.len());
            for key in keys.iter() {
                k.push(serializable_to_nb(key, layouts)?);
            }
            let mut v = Vec::with_capacity(values.len());
            for val in values.iter() {
                v.push(serializable_to_nb(val, layouts)?);
            }
            ValueWord::from_hashmap_pairs(k, v)
        }
        SerializableVMValue::Range {
            start,
            end,
            inclusive,
        } => ValueWord::from_range(
            match start {
                Some(s) => Some(serializable_to_nb(s, layouts)?),
                None => None,
            },
            match end {
                Some(e) => Some(serializable_to_nb(e, layouts)?),
                None => None,
            },
            *inclusive,
        ),
        SerializableVMValue::Closure { .. } => {
            // Track A.4: defer to the snapshot crate's
            // `serializable_to_nanboxed_with_layouts` so the replay
            // logic has a single home. `SnapshotStore` is only
            // consulted for sidecar blobs (not present in the remote
            // response payloads that reach this path), so a temp
            // store with a throwaway directory is sufficient.
            let tmp = std::env::temp_dir().join("shape_remote_builtins_closure_stage");
            let store = shape_runtime::snapshot::SnapshotStore::new(&tmp).map_err(|e| {
                format!("remote_builtins: failed to create staging store: {}", e)
            })?;
            shape_runtime::snapshot::serializable_to_nanboxed_with_layouts(sv, &store, layouts)
                .map_err(|e| {
                    format!(
                        "remote_builtins: cross-node closure replay failed: {}",
                        e
                    )
                })?
        }
        SerializableVMValue::TypedObject {
            schema_id,
            slot_data,
            heap_mask,
        } => {
            let mut slots = Vec::with_capacity(slot_data.len());
            for (i, sv) in slot_data.iter().enumerate() {
                if *heap_mask & (1u64 << i) != 0 {
                    let vw = serializable_to_nb(sv, layouts)?;
                    let (slot, _) = shape_value::ValueSlot::from_value_word(&vw);
                    slots.push(slot);
                } else {
                    slots.push(match sv {
                        SerializableVMValue::Number(n) => shape_value::ValueSlot::from_number(*n),
                        _ => shape_value::ValueSlot::from_raw(0),
                    });
                }
            }
            ValueWord::from_heap_value(shape_value::HeapValue::TypedObject {
                schema_id: *schema_id,
                slots: slots.into_boxed_slice(),
                heap_mask: *heap_mask,
            })
        }
        _ => ValueWord::none(),
    })
}

/// remote.__call(addr, fn_ref, args) -> Result<_, string>
///
/// Ships a function call to a remote `shape serve` node. The function is
/// identified by its ID (ValueWord Function value), and arguments are
/// serialized via the wire protocol.
///
/// This is the low-level transport used by the `@remote` annotation.
fn remote_call(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;

    let addr = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "remote.__call(): first argument must be an address string".to_string())?;

    // fn_ref can be a Function (u16 ID) or a number (function index from annotation ctx)
    let func_id = args
        .get(1)
        .and_then(|a| a.as_function_id().or_else(|| a.as_f64().map(|n| n as u16)))
        .ok_or_else(|| {
            "remote.__call(): second argument must be a function reference".to_string()
        })?;

    // Extract args array — could be an empty array or contain values
    let call_args: Vec<ValueWord> = args
        .get(2)
        .and_then(|a| a.as_any_array().map(|view| view.to_generic().to_vec()))
        .unwrap_or_default();

    // Get the current program from thread-local
    let program = CURRENT_PROGRAM
        .with(|p| p.borrow().clone())
        .ok_or_else(|| {
            "remote.__call(): no program context available (internal error)".to_string()
        })?;

    // Look up function name for the request
    let function_name = program
        .functions
        .get(func_id as usize)
        .map(|f| f.name.clone())
        .unwrap_or_default();

    // Serialize arguments
    let serialized_args: Vec<_> = call_args
        .iter()
        .map(|arg| nb_to_serializable(arg))
        .collect();

    // Build the remote call request
    let request = crate::remote::build_call_request(&program, &function_name, serialized_args);
    let msg = crate::remote::WireMessage::Call(request);

    let response = wire_roundtrip(&addr, &msg)?;

    match response {
        crate::remote::WireMessage::CallResponse(r) => match r.result {
            Ok(serialized_value) => {
                // Track A.4: decode closures via the caller's layout
                // registry. Cross-node closure payloads must carry a
                // `function_id` that maps to a live layout in the
                // caller's program — missing layouts hard-error.
                let value =
                    serializable_to_nb(&serialized_value, &program.closure_function_layouts)?;
                Ok(ValueWord::from_ok(value))
            }
            Err(e) => Ok(ValueWord::from_err(ValueWord::from_string(Arc::new(
                e.message,
            )))),
        },
        other => Err(format!(
            "remote.__call(): unexpected response type: {:?}",
            std::mem::discriminant(&other)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_module_creation() {
        let module = create_remote_module();
        assert_eq!(module.name, "std::core::remote");
        assert!(module.exports.contains_key("execute"));
        assert!(module.exports.contains_key("ping"));
        assert!(module.exports.contains_key("__call"));
    }

    /// Track A.4: fabricate a wire payload representing a cross-node
    /// closure response, decode it through `serializable_to_nb` with a
    /// matching layout registry, and verify the rebuilt closure reads
    /// back through the `VmClosureHandle` shim.
    #[test]
    fn test_a4_cross_node_closure_decode_with_layout() {
        use shape_runtime::snapshot::SerializableVMValue;
        use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
        use shape_value::v2::concrete_type::ConcreteType;

        // Synthesize a closure payload as if sent by a remote node.
        let payload = SerializableVMValue::Closure {
            function_id: 9,
            type_id: 3,
            upvalues: vec![SerializableVMValue::Int(123)],
        };

        // Caller-side layout registry — the function at id 9 captures
        // a single I64 (Immutable).
        let layout = Arc::new(ClosureLayout::from_capture_types(
            &[ConcreteType::I64],
            &[CaptureKind::Immutable],
        ));
        let mut layouts: Vec<Option<Arc<ClosureLayout>>> = vec![None; 10];
        layouts[9] = Some(Arc::clone(&layout));

        let nb = serializable_to_nb(&payload, &layouts).expect("A.4 replay must succeed");
        let handle = nb
            .as_closure_handle()
            .expect("decoded value must be a closure");
        assert_eq!(handle.function_id(), 9);
        assert_eq!(handle.type_id(), 3);
        assert_eq!(handle.capture_count(), 1);
        assert_eq!(handle.capture_as_value(0).as_i64(), Some(123));

        // Missing layout must surface a hard error, not a panic, not a
        // Legacy-backed closure.
        let err =
            serializable_to_nb(&payload, &[]).expect_err("missing layout must hard-error");
        assert!(
            err.contains("no ClosureLayout registered"),
            "error must mention missing layout, got: {err}"
        );
    }

    /// Track A.4: a nested closure inside an Array payload must also
    /// propagate the layouts slice through the recursive walk.
    #[test]
    fn test_a4_nested_closure_in_array_decodes_with_layout() {
        use shape_runtime::snapshot::SerializableVMValue;
        use shape_value::v2::closure_layout::{CaptureKind, ClosureLayout};
        use shape_value::v2::concrete_type::ConcreteType;

        let closure_payload = SerializableVMValue::Closure {
            function_id: 2,
            type_id: 0,
            upvalues: vec![SerializableVMValue::Number(1.25)],
        };
        let payload = SerializableVMValue::Array(vec![
            SerializableVMValue::Int(7),
            closure_payload,
            SerializableVMValue::String("tail".to_string()),
        ]);

        let layout = Arc::new(ClosureLayout::from_capture_types(
            &[ConcreteType::F64],
            &[CaptureKind::Immutable],
        ));
        let mut layouts: Vec<Option<Arc<ClosureLayout>>> = vec![None; 3];
        layouts[2] = Some(Arc::clone(&layout));

        let nb = serializable_to_nb(&payload, &layouts).expect("nested replay succeeds");
        let arr = nb
            .as_heap_ref()
            .and_then(|hv| match hv {
                shape_value::HeapValue::Array(a) => Some(a.clone()),
                _ => None,
            })
            .expect("top-level must be an array");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_i64(), Some(7));
        let handle = arr[1]
            .as_closure_handle()
            .expect("nested closure must decode");
        assert_eq!(handle.function_id(), 2);
        assert_eq!(handle.capture_as_value(0).as_f64(), Some(1.25));
    }
}
