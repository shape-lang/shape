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
use shape_value::ValueWord;
use shape_wire::transport::factory::TransportKind;
use shape_wire::transport::framing::{decode_framed, encode_framed};
use shape_wire::transport::Transport;
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
    let mut module = ModuleExports::new("remote");
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
    let mp =
        shape_wire::encode_message(msg).map_err(|e| format!("remote: encode error: {}", e))?;

    // Wrap in framing (compression header)
    let framed = encode_framed(&mp);

    // Send via transport (which adds 4-byte length prefix)
    let response_bytes = transport
        .send(addr, &framed)
        .map_err(|e| format!("remote: transport error: {}", e))?;

    // Decode response: decode_framed then decode_message
    let decompressed = decode_framed(&response_bytes)
        .map_err(|e| format!("remote: framing decode error: {}", e))?;

    shape_wire::decode_message(&decompressed)
        .map_err(|e| format!("remote: decode error: {}", e))
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

    let msg = crate::remote::WireMessage::Ping;
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

/// Lightweight ValueWord → SerializableVMValue conversion for remote calls.
///
/// Handles common types without requiring a filesystem-backed SnapshotStore.
/// Falls back to `None` for unsupported/complex heap types.
fn nb_to_serializable(
    nb: &ValueWord,
) -> shape_runtime::snapshot::SerializableVMValue {
    use shape_runtime::snapshot::SerializableVMValue;
    use shape_value::NanTag;

    match nb.tag() {
        NanTag::F64 => SerializableVMValue::Number(nb.as_f64().unwrap()),
        NanTag::I48 => SerializableVMValue::Int(nb.as_i64().unwrap()),
        NanTag::Bool => SerializableVMValue::Bool(nb.as_bool().unwrap()),
        NanTag::None => SerializableVMValue::None,
        NanTag::Unit => SerializableVMValue::Unit,
        NanTag::Function => SerializableVMValue::Function(nb.as_function().unwrap()),
        NanTag::Heap => {
            use shape_value::HeapValue;
            match nb.as_heap_ref() {
                Some(HeapValue::String(s)) => SerializableVMValue::String((**s).clone()),
                Some(HeapValue::Decimal(d)) => SerializableVMValue::Decimal(*d),
                Some(HeapValue::BigInt(i)) => SerializableVMValue::Int(*i),
                Some(HeapValue::Array(arr)) => {
                    let items: Vec<_> = arr.iter().map(|v| nb_to_serializable(v)).collect();
                    SerializableVMValue::Array(items)
                }
                Some(HeapValue::Closure { function_id, upvalues }) => {
                    let ups: Vec<_> = upvalues.iter().map(|u| nb_to_serializable(&u.get())).collect();
                    SerializableVMValue::Closure { function_id: *function_id, upvalues: ups }
                }
                _ => SerializableVMValue::None,
            }
        }
        _ => SerializableVMValue::None,
    }
}

/// Lightweight SerializableVMValue → ValueWord conversion for remote call responses.
fn serializable_to_nb(
    sv: &shape_runtime::snapshot::SerializableVMValue,
) -> ValueWord {
    use shape_runtime::snapshot::SerializableVMValue;

    match sv {
        SerializableVMValue::Number(n) => ValueWord::from_f64(*n),
        SerializableVMValue::Int(i) => ValueWord::from_i64(*i),
        SerializableVMValue::Bool(b) => ValueWord::from_bool(*b),
        SerializableVMValue::None => ValueWord::none(),
        SerializableVMValue::Unit => ValueWord::unit(),
        SerializableVMValue::String(s) => ValueWord::from_string(Arc::new(s.clone())),
        SerializableVMValue::Function(id) => ValueWord::from_function(*id),
        SerializableVMValue::Array(items) => {
            let vals: Vec<_> = items.iter().map(serializable_to_nb).collect();
            ValueWord::from_array(Arc::new(vals))
        }
        SerializableVMValue::Decimal(d) => ValueWord::from_decimal(*d),
        _ => ValueWord::none(),
    }
}

/// remote.__call(addr, fn_ref, args) -> Result<_, string>
///
/// Ships a function call to a remote `shape serve` node. The function is
/// identified by its ID (NanBoxed Function value), and arguments are
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
        .and_then(|a| {
            a.as_function()
                .or_else(|| a.as_f64().map(|n| n as u16))
        })
        .ok_or_else(|| {
            "remote.__call(): second argument must be a function reference".to_string()
        })?;

    // Extract args array — could be an empty array or contain values
    let call_args: Vec<ValueWord> = args
        .get(2)
        .and_then(|a| a.as_any_array().map(|view| (*view.to_generic()).clone()))
        .unwrap_or_default();

    // Get the current program from thread-local
    let program = CURRENT_PROGRAM.with(|p| p.borrow().clone()).ok_or_else(|| {
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
                let value = serializable_to_nb(&serialized_value);
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
        assert_eq!(module.name, "remote");
        assert!(module.exports.contains_key("execute"));
        assert!(module.exports.contains_key("ping"));
        assert!(module.exports.contains_key("__call"));
    }
}
