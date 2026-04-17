//! Native `transport` module for distributed Shape communication.
//!
//! Thin wrapper around `shape_wire::transport` that exposes the transport
//! abstraction to Shape code via the module/builtin system. The actual
//! TCP framing logic lives in `shape_wire::transport::tcp`.
//!
//! Exports:
//! - transport.tcp() -> Transport (marker IoHandle)
//! - transport.memoized(max_entries?) -> MemoTransport (memoized TCP transport)
//! - transport.send(transport, destination, payload) -> Result<Array<int>, string>
//! - transport.connect(transport, destination) -> Result<Connection, string>
//! - transport.connection_send(conn, payload) -> Result<(), string>
//! - transport.connection_recv(conn, timeout?) -> Result<Array<int>, string>
//! - transport.connection_close(conn) -> Result<(), string>
//! - transport.memo_stats(handle) -> { cache_hits, cache_misses, evictions, total_requests }
//! - transport.memo_invalidate(handle) -> ()

use shape_runtime::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_value::{ValueWord, ValueWordExt};
use shape_value::heap_value::{IoHandleData, IoHandleKind, IoResource};
use shape_wire::transport::factory::TransportKind;
use shape_wire::transport::memoized::{MemoConfig, MemoizedTransport};
use shape_wire::transport::{Connection, Transport};
use std::sync::Arc;
use std::time::Duration;

use super::transport_provider;

/// Type-erased transport handle stored in `IoResource::Custom`.
struct TransportHandle {
    transport: Arc<dyn Transport>,
    memoized: Option<Arc<MemoizedTransport<Arc<dyn Transport>>>>,
}

/// Wrapper for `Box<dyn Connection>` so it can be stored in `IoResource::Custom`
/// (which requires `Any + Send`). The inner `Mutex` allows mutable access through
/// the shared IoHandle reference.
struct BoxedConnection(std::sync::Mutex<Box<dyn Connection>>);

/// Create the `transport` module with TCP transport functions.
pub fn create_transport_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::transport");
    module.description = "Network transport for distributed Shape".to_string();

    // transport.tcp() -> Transport
    module.add_function_with_schema(
        "tcp",
        transport_tcp,
        ModuleFunction {
            description: "Create a TCP transport handle".to_string(),
            params: vec![],
            return_type: Some("Transport".to_string()),
        },
    );

    // transport.quic() -> Transport  (requires `quic` feature)
    #[cfg(feature = "quic")]
    module.add_function_with_schema(
        "quic",
        transport_quic,
        ModuleFunction {
            description: "Create a QUIC transport handle (multiplexed, encrypted)".to_string(),
            params: vec![],
            return_type: Some("Transport".to_string()),
        },
    );

    // transport.send(transport, destination, payload) -> Result<Array<int>, string>
    module.add_function_with_schema(
        "send",
        transport_send,
        ModuleFunction {
            description: "Send a payload to a destination and wait for a length-prefixed response"
                .to_string(),
            params: vec![
                ModuleParam {
                    name: "transport".to_string(),
                    type_name: "Transport".to_string(),
                    required: true,
                    description: "Transport handle from transport.tcp()".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "destination".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Remote address as host:port".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "payload".to_string(),
                    type_name: "Array<int>".to_string(),
                    required: true,
                    description: "Byte array to send".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Result<Array<int>, string>".to_string()),
        },
    );

    // transport.connect(transport, destination) -> Result<Connection, string>
    module.add_function_with_schema(
        "connect",
        transport_connect,
        ModuleFunction {
            description: "Establish a persistent TCP connection to a remote node".to_string(),
            params: vec![
                ModuleParam {
                    name: "transport".to_string(),
                    type_name: "Transport".to_string(),
                    required: true,
                    description: "Transport handle from transport.tcp()".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "destination".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Remote address as host:port".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Result<Connection, string>".to_string()),
        },
    );

    // transport.connection_send(conn, payload) -> Result<(), string>
    module.add_function_with_schema(
        "connection_send",
        connection_send_fn,
        ModuleFunction {
            description: "Send a length-prefixed payload over an established connection"
                .to_string(),
            params: vec![
                ModuleParam {
                    name: "conn".to_string(),
                    type_name: "Connection".to_string(),
                    required: true,
                    description: "Connection handle from transport.connect()".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "payload".to_string(),
                    type_name: "Array<int>".to_string(),
                    required: true,
                    description: "Byte array to send".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Result<(), string>".to_string()),
        },
    );

    // transport.connection_recv(conn, timeout?) -> Result<Array<int>, string>
    module.add_function_with_schema(
        "connection_recv",
        connection_recv_fn,
        ModuleFunction {
            description: "Receive a length-prefixed payload from an established connection"
                .to_string(),
            params: vec![
                ModuleParam {
                    name: "conn".to_string(),
                    type_name: "Connection".to_string(),
                    required: true,
                    description: "Connection handle from transport.connect()".to_string(),
                    ..Default::default()
                },
                ModuleParam {
                    name: "timeout".to_string(),
                    type_name: "int".to_string(),
                    required: false,
                    description: "Timeout in milliseconds (None = wait indefinitely)".to_string(),
                    ..Default::default()
                },
            ],
            return_type: Some("Result<Array<int>, string>".to_string()),
        },
    );

    // transport.connection_close(conn) -> Result<(), string>
    module.add_function_with_schema(
        "connection_close",
        connection_close_fn,
        ModuleFunction {
            description: "Close an established connection".to_string(),
            params: vec![ModuleParam {
                name: "conn".to_string(),
                type_name: "Connection".to_string(),
                required: true,
                description: "Connection handle from transport.connect()".to_string(),
                ..Default::default()
            }],
            return_type: Some("Result<(), string>".to_string()),
        },
    );

    // transport.memoized(max_entries?) -> MemoTransport
    module.add_function_with_schema(
        "memoized",
        transport_memoized,
        ModuleFunction {
            description: "Create a memoized TCP transport that caches send results".to_string(),
            params: vec![ModuleParam {
                name: "max_entries".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Maximum cache entries (default 1024)".to_string(),
                ..Default::default()
            }],
            return_type: Some("MemoTransport".to_string()),
        },
    );

    // transport.memo_stats(handle) -> { cache_hits, cache_misses, evictions, total_requests }
    module.add_function_with_schema(
        "memo_stats",
        transport_memo_stats,
        ModuleFunction {
            description: "Return cache statistics for a memoized transport".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "MemoTransport".to_string(),
                required: true,
                description: "Memoized transport handle from transport.memoized()".to_string(),
                ..Default::default()
            }],
            return_type: Some(
                "{ cache_hits: int, cache_misses: int, evictions: int, total_requests: int }"
                    .to_string(),
            ),
        },
    );

    // transport.memo_invalidate(handle) -> ()
    module.add_function_with_schema(
        "memo_invalidate",
        transport_memo_invalidate,
        ModuleFunction {
            description: "Clear all cached entries in a memoized transport".to_string(),
            params: vec![ModuleParam {
                name: "handle".to_string(),
                type_name: "MemoTransport".to_string(),
                required: true,
                description: "Memoized transport handle from transport.memoized()".to_string(),
                ..Default::default()
            }],
            return_type: Some("()".to_string()),
        },
    );

    module
}

// ---------------------------------------------------------------------------
// Helpers: ValueWord <-> byte conversions
// ---------------------------------------------------------------------------

/// Convert a ValueWord Array<int> to a Vec<u8>.
fn nanboxed_array_to_bytes(arr: &ValueWord) -> Result<Vec<u8>, String> {
    let view = arr
        .as_any_array()
        .ok_or_else(|| "transport: payload must be an Array<int>".to_string())?;
    let array = view.to_generic();
    let mut bytes = Vec::with_capacity(array.len());
    for nb in array.iter() {
        let val = nb
            .as_i64()
            .or_else(|| nb.as_number_coerce().map(|n| n as i64))
            .ok_or_else(|| "transport: payload array elements must be integers".to_string())?;
        if !(0..=255).contains(&val) {
            return Err(format!(
                "transport: byte value out of range: {} (must be 0-255)",
                val
            ));
        }
        bytes.push(val as u8);
    }
    Ok(bytes)
}

/// Convert a Vec<u8> to a ValueWord Array<int>.
fn bytes_to_nanboxed_array(data: &[u8]) -> ValueWord {
    let elements: Vec<ValueWord> = data
        .iter()
        .map(|&b| ValueWord::from_i64(b as i64))
        .collect();
    ValueWord::from_array(shape_value::vmarray_from_vec(elements))
}

/// Extract a typed `TransportHandle` from an IoHandle argument.
fn extract_transport_handle(
    handle: &IoHandleData,
    fn_name: &str,
) -> Result<Arc<dyn Transport>, String> {
    let guard = handle
        .resource
        .lock()
        .map_err(|_| format!("transport.{}(): lock poisoned", fn_name))?;
    let resource = guard
        .as_ref()
        .ok_or_else(|| format!("transport.{}(): handle is closed", fn_name))?;
    if let IoResource::Custom(any) = resource {
        if let Some(transport_handle) = any.downcast_ref::<TransportHandle>() {
            return Ok(transport_handle.transport.clone());
        }
    }
    Err(format!(
        "transport.{}(): first argument must be a Transport handle",
        fn_name
    ))
}

// ---------------------------------------------------------------------------
// Module functions — delegate to shape_wire::transport
// ---------------------------------------------------------------------------

/// transport.tcp() -> Transport
///
/// Creates a typed transport handle backed by `shape_wire::transport::tcp::TcpTransport`.
fn transport_tcp(_args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let transport =
        transport_provider::transport_provider().create_transport(TransportKind::Tcp)?;
    let handle = IoHandleData::new_custom(
        Box::new(TransportHandle {
            transport,
            memoized: None,
        }),
        "transport:tcp".to_string(),
    );
    Ok(ValueWord::from_io_handle(handle))
}

/// transport.quic() -> Transport
///
/// Creates a QUIC transport handle. Requires the `quic` feature and prior
/// configuration via `shape_vm::configure_quic_transport(...)`.
#[cfg(feature = "quic")]
fn transport_quic(_args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let transport =
        transport_provider::transport_provider().create_transport(TransportKind::Quic)?;
    let handle = IoHandleData::new_custom(
        Box::new(TransportHandle {
            transport,
            memoized: None,
        }),
        "transport:quic".to_string(),
    );
    Ok(ValueWord::from_io_handle(handle))
}

/// transport.send(transport, destination, payload) -> Result<Array<int>, string>
///
/// One-shot send. Supports both plain TCP handles and memoized transport handles.
/// When the handle is a memoized transport, results are served from cache when available.
fn transport_send(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "transport.send(): first argument must be a Transport handle".to_string())?;

    let destination = args.get(1).and_then(|a| a.as_str()).ok_or_else(|| {
        "transport.send(): second argument must be a destination string (host:port)".to_string()
    })?;

    let payload_nb = args
        .get(2)
        .ok_or_else(|| "transport.send(): third argument (payload) is required".to_string())?;
    let payload_bytes = nanboxed_array_to_bytes(payload_nb)?;
    let transport = extract_transport_handle(handle, "send")?;
    match transport.send(destination, &payload_bytes) {
        Ok(response) => Ok(ValueWord::from_ok(bytes_to_nanboxed_array(&response))),
        Err(e) => Ok(ValueWord::from_err(ValueWord::from_string(Arc::new(
            format!("transport.send(): {}", e),
        )))),
    }
}

/// transport.connect(transport, destination) -> Result<Connection, string>
///
/// Establish a persistent TCP connection. Returns an IoHandle wrapping a TcpStream
/// for compatibility with the existing connection_send/recv/close functions.
fn transport_connect(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    // Validate transport handle (arg 0)
    let transport_handle = args.first().and_then(|a| a.as_io_handle()).ok_or_else(|| {
        "transport.connect(): first argument must be a Transport handle".to_string()
    })?;

    let destination = args.get(1).and_then(|a| a.as_str()).ok_or_else(|| {
        "transport.connect(): second argument must be a destination string (host:port)".to_string()
    })?;
    let transport = extract_transport_handle(transport_handle, "connect")?;
    match transport.connect(destination) {
        Ok(conn) => {
            let handle = IoHandleData::new_custom(
                Box::new(BoxedConnection(std::sync::Mutex::new(conn))),
                format!("transport:conn:{}", destination),
            );
            Ok(ValueWord::from_ok(ValueWord::from_io_handle(handle)))
        }
        Err(e) => Ok(ValueWord::from_err(ValueWord::from_string(Arc::new(
            format!("transport.connect(): {}", e),
        )))),
    }
}

/// transport.connection_send(conn, payload) -> Result<(), string>
///
/// Send a length-prefixed payload over an established connection.
/// Delegates framing to the `shape_wire::transport::Connection` implementation.
fn connection_send_fn(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let handle = args
        .first()
        .ok_or_else(|| "transport.connection_send(): missing connection argument".to_string())?
        .as_io_handle()
        .cloned()
        .ok_or_else(|| {
            "transport.connection_send(): expected an IoHandle (Connection)".to_string()
        })?;

    let payload_nb = args.get(1).ok_or_else(|| {
        "transport.connection_send(): second argument (payload) is required".to_string()
    })?;
    let payload_bytes = nanboxed_array_to_bytes(payload_nb)?;

    let mut guard = handle
        .resource
        .lock()
        .map_err(|_| "transport.connection_send(): lock poisoned".to_string())?;
    let resource = guard
        .as_mut()
        .ok_or_else(|| "transport.connection_send(): connection is closed".to_string())?;

    let IoResource::Custom(any) = resource else {
        return Err("transport.connection_send(): handle is not a connection".to_string());
    };
    let Some(boxed_conn) = any.downcast_mut::<BoxedConnection>() else {
        return Err("transport.connection_send(): handle is not a connection".to_string());
    };

    let mut conn = boxed_conn
        .0
        .lock()
        .map_err(|_| "transport.connection_send(): lock poisoned".to_string())?;
    match conn.send(&payload_bytes) {
        Ok(()) => Ok(ValueWord::from_ok(ValueWord::unit())),
        Err(e) => Ok(ValueWord::from_err(ValueWord::from_string(Arc::new(
            format!("transport.connection_send(): {}", e),
        )))),
    }
}

/// transport.connection_recv(conn, timeout?) -> Result<Array<int>, string>
///
/// Receive a length-prefixed payload from an established connection.
/// Delegates framing to the `shape_wire::transport::Connection` implementation.
fn connection_recv_fn(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let handle = args
        .first()
        .ok_or_else(|| "transport.connection_recv(): missing connection argument".to_string())?
        .as_io_handle()
        .cloned()
        .ok_or_else(|| {
            "transport.connection_recv(): expected an IoHandle (Connection)".to_string()
        })?;

    let timeout_ms = args.get(1).and_then(|a| {
        if a.is_none() {
            None
        } else {
            a.as_i64()
                .or_else(|| a.as_number_coerce().map(|n| n as i64))
        }
    });

    let mut guard = handle
        .resource
        .lock()
        .map_err(|_| "transport.connection_recv(): lock poisoned".to_string())?;
    let resource = guard
        .as_mut()
        .ok_or_else(|| "transport.connection_recv(): connection is closed".to_string())?;

    let IoResource::Custom(any) = resource else {
        return Err("transport.connection_recv(): handle is not a connection".to_string());
    };
    let Some(boxed_conn) = any.downcast_mut::<BoxedConnection>() else {
        return Err("transport.connection_recv(): handle is not a connection".to_string());
    };

    let timeout = timeout_ms.map(|ms| Duration::from_millis(ms.max(0) as u64));
    let mut conn = boxed_conn
        .0
        .lock()
        .map_err(|_| "transport.connection_recv(): lock poisoned".to_string())?;
    match conn.recv(timeout) {
        Ok(data) => Ok(ValueWord::from_ok(bytes_to_nanboxed_array(&data))),
        Err(e) => Ok(ValueWord::from_err(ValueWord::from_string(Arc::new(
            format!("transport.connection_recv(): {}", e),
        )))),
    }
}

/// transport.connection_close(conn) -> Result<(), string>
///
/// Close an established connection.
fn connection_close_fn(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let handle = args
        .first()
        .ok_or_else(|| "transport.connection_close(): missing connection argument".to_string())?
        .as_io_handle()
        .cloned()
        .ok_or_else(|| {
            "transport.connection_close(): expected an IoHandle (Connection)".to_string()
        })?;

    // All transport connections are boxed `shape_wire::transport::Connection`
    // handles stored in IoResource::Custom.
    {
        let mut guard = handle
            .resource
            .lock()
            .map_err(|_| "transport.connection_close(): lock poisoned".to_string())?;
        let resource = guard
            .as_mut()
            .ok_or_else(|| "transport.connection_close(): connection is closed".to_string())?;
        let IoResource::Custom(any) = resource else {
            return Err("transport.connection_close(): handle is not a connection".to_string());
        };
        let Some(boxed_conn) = any.downcast_mut::<BoxedConnection>() else {
            return Err("transport.connection_close(): handle is not a connection".to_string());
        };

        let mut conn = boxed_conn
            .0
            .lock()
            .map_err(|_| "transport.connection_close(): lock poisoned".to_string())?;
        if let Err(e) = conn.close() {
            return Ok(ValueWord::from_err(ValueWord::from_string(Arc::new(
                format!("transport.connection_close(): {}", e),
            ))));
        }
    }
    handle.close();
    Ok(ValueWord::from_ok(ValueWord::unit()))
}

// ---------------------------------------------------------------------------
// Memoized transport functions
// ---------------------------------------------------------------------------

/// transport.memoized(max_entries?) -> MemoTransport
///
/// Creates a memoized TCP transport. One-shot `send` calls through this handle
/// are cached by `SHA-256(destination || payload)` and served from cache on repeat.
fn transport_memoized(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;
    let max_entries = args
        .first()
        .and_then(|a| {
            if a.is_none() {
                None
            } else {
                a.as_i64()
                    .or_else(|| a.as_number_coerce().map(|n| n as i64))
            }
        })
        .unwrap_or(1024);

    if max_entries < 1 {
        return Err("transport.memoized(): max_entries must be >= 1".to_string());
    }

    let config = MemoConfig {
        max_entries: max_entries as usize,
        enabled: true,
    };
    let base_transport = transport_provider::transport_provider()
        .create_transport(TransportKind::Tcp)
        .map_err(|e| format!("transport.memoized(): {}", e))?;
    let memo = Arc::new(MemoizedTransport::new(base_transport, config));
    let transport: Arc<dyn Transport> = memo.clone();
    let handle = IoHandleData::new_custom(
        Box::new(TransportHandle {
            transport,
            memoized: Some(memo),
        }),
        "transport:memoized_tcp".to_string(),
    );
    Ok(ValueWord::from_io_handle(handle))
}

/// Extract the memoized transport state from a typed transport handle.
fn extract_memo_transport(
    handle: &IoHandleData,
    fn_name: &str,
) -> Result<Arc<MemoizedTransport<Arc<dyn Transport>>>, String> {
    if handle.kind != IoHandleKind::Custom {
        return Err(format!(
            "transport.{}(): expected a memoized transport handle",
            fn_name
        ));
    }
    let guard = handle
        .resource
        .lock()
        .map_err(|_| format!("transport.{}(): lock poisoned", fn_name))?;
    let resource = guard
        .as_ref()
        .ok_or_else(|| format!("transport.{}(): handle is closed", fn_name))?;
    if let IoResource::Custom(any) = resource {
        if let Some(transport_handle) = any.downcast_ref::<TransportHandle>()
            && let Some(memo) = &transport_handle.memoized
        {
            return Ok(Arc::clone(memo));
        }
    }
    Err(format!(
        "transport.{}(): handle is not a memoized transport",
        fn_name
    ))
}

/// transport.memo_stats(handle) -> { cache_hits, cache_misses, evictions, total_requests }
///
/// Returns a snapshot of cache statistics for a memoized transport.
fn transport_memo_stats(args: &[ValueWord], _ctx: &ModuleContext) -> Result<ValueWord, String> {
    let handle = args.first().and_then(|a| a.as_io_handle()).ok_or_else(|| {
        "transport.memo_stats(): first argument must be a MemoTransport handle".to_string()
    })?;

    let memo = extract_memo_transport(handle, "memo_stats")?;
    let stats = memo.stats();

    // Return stats as an array of key-value pairs: [hits, misses, evictions, total]
    let elements = vec![
        ValueWord::from_i64(stats.cache_hits as i64),
        ValueWord::from_i64(stats.cache_misses as i64),
        ValueWord::from_i64(stats.evictions as i64),
        ValueWord::from_i64(stats.total_requests as i64),
    ];
    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(elements)))
}

/// transport.memo_invalidate(handle) -> ()
///
/// Clears all cached entries in a memoized transport.
fn transport_memo_invalidate(
    args: &[ValueWord],
    _ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let handle = args.first().and_then(|a| a.as_io_handle()).ok_or_else(|| {
        "transport.memo_invalidate(): first argument must be a MemoTransport handle".to_string()
    })?;

    let memo = extract_memo_transport(handle, "memo_invalidate")?;
    memo.invalidate_all();
    Ok(ValueWord::unit())
}

#[cfg(test)]
#[path = "transport_builtins_tests.rs"]
mod tests;
