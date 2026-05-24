//! Native `transport` module for distributed Shape communication.
//!
//! ADR-006 §2.7.28 W17-typed-module-exports 2026-05-23
//!
//! Thin wrapper around `shape_wire::transport` that exposes the transport
//! abstraction to Shape code via the module/builtin system. The actual
//! TCP framing logic lives in `shape_wire::transport::tcp`.
//!
//! Exports:
//! - transport.tcp() -> IoHandle  (marker Transport handle)
//! - transport.memoized(max_entries?) -> IoHandle (memoized TCP transport)
//! - transport.send(transport, destination, payload) -> Result<Array<int>, string>
//! - transport.connect(transport, destination) -> Result<IoHandle, string>
//! - transport.connection_send(conn, payload) -> Result<(), string>
//! - transport.connection_recv(conn, timeout?) -> Result<Array<int>, string>
//! - transport.connection_close(conn) -> Result<(), string>
//! - transport.memo_stats(handle) -> { cache_hits, cache_misses, evictions, total_requests }
//! - transport.memo_invalidate(handle) -> ()
//!
//! ## W17-typed-module-exports rebuild (ADR-006 §2.7.4 + §2.7.4-addendum)
//!
//! Phase-2c R8-W2 (2026-05-23): bodies rebuilt on top of the kind-threaded
//! marshal layer per ADR-006 §2.7.4 + addendum. Each body uses the typed
//! `register_typed_fn_N{_full}` helpers — args arrive already-decoded as
//! their Rust types (`Arc<IoHandleData>`, `Arc<String>`, `Vec<i64>`, etc.)
//! per the registered `FromSlot` impls, and the return projects through
//! `TypedReturn::Ok(ConcreteReturn::*)` / `TypedReturn::Err(...)` /
//! `TypedReturn::OkObjectPairs(...)` — the marshal-layer registry-side
//! lowers each variant directly into a typed VM slot via the function's
//! declared `ConcreteType` return descriptor. No `ValueWord` re-introduction,
//! no `wrap_legacy` shim — the kind contract is enforced by the Rust type
//! system at registration time.
//!
//! Surface schema (parameter names, types, descriptions, return-type
//! strings) is preserved verbatim from the pre-bulldozer shape so LSP
//! completion and signature help remain stable.

use shape_runtime::marshal::{
    register_typed_fn_0, register_typed_fn_1, register_typed_fn_1_full, register_typed_fn_2,
    register_typed_fn_2_full, register_typed_fn_3,
};
use shape_runtime::module_exports::{ModuleExports, ModuleParam};
use shape_runtime::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::{IoHandleData, IoResource};
use shape_wire::transport::factory::TransportKind;
use shape_wire::transport::memoized::{MemoConfig, MemoizedTransport};
use shape_wire::transport::{Connection, Transport};
use std::sync::Arc;
use std::time::Duration;

use super::transport_provider;

/// Type-erased transport handle stored in `IoResource::Custom`.
///
/// Carries both the raw `Arc<dyn Transport>` and an optional memoized
/// wrapper. `transport.send` checks the memoized side first and falls
/// through to the raw transport when caching is disabled.
pub(super) struct TransportHandle {
    pub(super) transport: Arc<dyn Transport>,
    pub(super) memoized:
        Option<Arc<MemoizedTransport<Arc<dyn Transport>>>>,
}

/// Wrapper for `Box<dyn Connection>` so it can be stored in
/// `IoResource::Custom` (which requires `Any + Send`). The inner `Mutex`
/// allows mutable access through the shared IoHandle reference.
pub(super) struct BoxedConnection(pub(super) std::sync::Mutex<Box<dyn Connection>>);

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Borrow a `TransportHandle` payload from an IoHandle argument.
///
/// W17-typed-module-exports protocol: the slot bits arrived as
/// `Arc<IoHandleData>` per the registered `FromSlot` impl. The body
/// downcasts the inner `IoResource::Custom` payload to `TransportHandle`
/// — typed-Arc payload through the module symbol-table boundary
/// (ADR-006 §2.7.4 addendum).
fn extract_transport(
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
        if let Some(h) = any.downcast_ref::<TransportHandle>() {
            return Ok(h.transport.clone());
        }
    }
    Err(format!(
        "transport.{}(): first argument must be a Transport handle \
         (got an IoHandle with non-Transport resource)",
        fn_name,
    ))
}

/// Borrow the memoized-transport view of a TransportHandle, if present.
fn extract_memoized(
    handle: &IoHandleData,
    fn_name: &str,
) -> Result<Arc<MemoizedTransport<Arc<dyn Transport>>>, String> {
    let guard = handle
        .resource
        .lock()
        .map_err(|_| format!("transport.{}(): lock poisoned", fn_name))?;
    let resource = guard
        .as_ref()
        .ok_or_else(|| format!("transport.{}(): handle is closed", fn_name))?;
    if let IoResource::Custom(any) = resource {
        if let Some(h) = any.downcast_ref::<TransportHandle>() {
            if let Some(memo) = h.memoized.as_ref() {
                return Ok(memo.clone());
            }
            return Err(format!(
                "transport.{}(): handle is a plain Transport (not memoized) — \
                 call transport.memoized() to create a memoized handle",
                fn_name,
            ));
        }
    }
    Err(format!(
        "transport.{}(): argument must be a memoized Transport handle",
        fn_name,
    ))
}

/// Convert a `Vec<i64>` payload (Shape `Array<int>`) to bytes.
///
/// Each element must be 0..=255; out-of-range elements are an error
/// per the pre-bulldozer contract.
fn ints_to_bytes(payload: &[i64], fn_name: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(payload.len());
    for &v in payload {
        if !(0..=255).contains(&v) {
            return Err(format!(
                "transport.{}(): byte value out of range: {} (must be 0..=255)",
                fn_name, v,
            ));
        }
        bytes.push(v as u8);
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Module factory
// ---------------------------------------------------------------------------

/// Create the `transport` module with TCP transport functions.
pub fn create_transport_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::transport");
    module.description = "Network transport for distributed Shape".to_string();

    // transport.tcp() -> IoHandle
    //
    // Creates a fresh TCP transport handle. The `IoHandle` return is a
    // marker — actual TCP work happens in transport.send/connect/etc.
    register_typed_fn_0(
        &mut module,
        "tcp",
        "Create a TCP transport handle",
        ConcreteType::IoHandle,
        |ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let transport = transport_provider::transport_provider()
                .create_transport(TransportKind::Tcp)
                .map_err(|e| format!("transport.tcp(): {}", e))?;
            let handle = IoHandleData::new_custom(
                Box::new(TransportHandle {
                    transport,
                    memoized: None,
                }),
                "transport:tcp".to_string(),
            );
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(handle))))
        },
    );

    // transport.quic() -> IoHandle  (requires `quic` feature)
    #[cfg(feature = "quic")]
    register_typed_fn_0(
        &mut module,
        "quic",
        "Create a QUIC transport handle (multiplexed, encrypted)",
        ConcreteType::IoHandle,
        |ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let transport = transport_provider::transport_provider()
                .create_transport(TransportKind::Quic)
                .map_err(|e| format!("transport.quic(): {}", e))?;
            let handle = IoHandleData::new_custom(
                Box::new(TransportHandle {
                    transport,
                    memoized: None,
                }),
                "transport:quic".to_string(),
            );
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(handle))))
        },
    );

    // transport.send(transport, destination, payload: Array<int>) -> Result<Array<int>, string>
    register_typed_fn_3::<_, Arc<IoHandleData>, Arc<String>, Vec<i64>>(
        &mut module,
        "send",
        "Send a payload to a destination and wait for a length-prefixed response",
        [
            ("transport", "IoHandle"),
            ("destination", "string"),
            ("payload", "Array<int>"),
        ],
        ConcreteType::Result2(
            Box::new(ConcreteType::ArrayInt),
            Box::new(ConcreteType::String),
        ),
        |handle, dest, payload, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let bytes = match ints_to_bytes(&payload, "send") {
                Ok(b) => b,
                Err(e) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(e)));
                }
            };
            let transport = match extract_transport(&handle, "send") {
                Ok(t) => t,
                Err(e) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(e)));
                }
            };
            match transport.send(dest.as_str(), &bytes) {
                Ok(response) => {
                    Ok(TypedReturn::Ok(ConcreteReturn::Bytes(response)))
                }
                Err(e) => Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                    "transport.send(): {}",
                    e
                )))),
            }
        },
    );

    // transport.connect(transport, destination) -> Result<IoHandle, string>
    register_typed_fn_2::<_, Arc<IoHandleData>, Arc<String>>(
        &mut module,
        "connect",
        "Establish a persistent TCP connection to a remote node",
        [("transport", "IoHandle"), ("destination", "string")],
        ConcreteType::Result2(
            Box::new(ConcreteType::IoHandle),
            Box::new(ConcreteType::String),
        ),
        |handle, dest, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let transport = match extract_transport(&handle, "connect") {
                Ok(t) => t,
                Err(e) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(e)));
                }
            };
            match transport.connect(dest.as_str()) {
                Ok(conn) => {
                    let conn_handle = IoHandleData::new_custom(
                        Box::new(BoxedConnection(std::sync::Mutex::new(conn))),
                        format!("transport:conn:{}", dest.as_str()),
                    );
                    Ok(TypedReturn::Ok(ConcreteReturn::IoHandle(Arc::new(
                        conn_handle,
                    ))))
                }
                Err(e) => Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                    "transport.connect(): {}",
                    e
                )))),
            }
        },
    );

    // transport.connection_send(conn, payload: Array<int>) -> Result<(), string>
    register_typed_fn_2::<_, Arc<IoHandleData>, Vec<i64>>(
        &mut module,
        "connection_send",
        "Send a length-prefixed payload over an established connection",
        [("conn", "IoHandle"), ("payload", "Array<int>")],
        // Preserve the literal "Result<(), string>" surface — Result2 would
        // emit "Result<unit, string>".
        ConcreteType::Named("Result<(), string>".to_string()),
        |handle, payload, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let bytes = match ints_to_bytes(&payload, "connection_send") {
                Ok(b) => b,
                Err(e) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(e)));
                }
            };
            let mut guard = match handle.resource.lock() {
                Ok(g) => g,
                Err(_) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(
                        "transport.connection_send(): lock poisoned".to_string(),
                    )));
                }
            };
            let resource = match guard.as_mut() {
                Some(r) => r,
                None => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(
                        "transport.connection_send(): handle is closed".to_string(),
                    )));
                }
            };
            if let IoResource::Custom(any) = resource {
                if let Some(boxed) = any.downcast_mut::<BoxedConnection>() {
                    let mut conn_guard = boxed.0.lock().map_err(|_| {
                        "transport.connection_send(): connection lock poisoned".to_string()
                    })?;
                    return match conn_guard.send(&bytes) {
                        Ok(()) => Ok(TypedReturn::Ok(ConcreteReturn::Unit)),
                        Err(e) => Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                            "transport.connection_send(): {}",
                            e
                        )))),
                    };
                }
            }
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "transport.connection_send(): handle is not a Connection".to_string(),
            )))
        },
    );

    // transport.connection_recv(conn, timeout?: int) -> Result<Array<int>, string>
    register_typed_fn_2_full::<_, Arc<IoHandleData>, i64>(
        &mut module,
        "connection_recv",
        "Receive a length-prefixed payload from an established connection",
        [
            ModuleParam {
                name: "conn".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "Connection handle from transport.connect()".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "timeout".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Timeout in milliseconds (0 = wait indefinitely)".to_string(),
                default_snippet: Some("0".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::Result2(
            Box::new(ConcreteType::ArrayInt),
            Box::new(ConcreteType::String),
        ),
        |handle, timeout_ms, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let timeout = if timeout_ms > 0 {
                Some(Duration::from_millis(timeout_ms as u64))
            } else {
                None
            };
            let mut guard = match handle.resource.lock() {
                Ok(g) => g,
                Err(_) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(
                        "transport.connection_recv(): lock poisoned".to_string(),
                    )));
                }
            };
            let resource = match guard.as_mut() {
                Some(r) => r,
                None => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(
                        "transport.connection_recv(): handle is closed".to_string(),
                    )));
                }
            };
            if let IoResource::Custom(any) = resource {
                if let Some(boxed) = any.downcast_mut::<BoxedConnection>() {
                    let mut conn_guard = boxed.0.lock().map_err(|_| {
                        "transport.connection_recv(): connection lock poisoned"
                            .to_string()
                    })?;
                    return match conn_guard.recv(timeout) {
                        Ok(bytes) => Ok(TypedReturn::Ok(ConcreteReturn::Bytes(bytes))),
                        Err(e) => Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                            "transport.connection_recv(): {}",
                            e
                        )))),
                    };
                }
            }
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "transport.connection_recv(): handle is not a Connection".to_string(),
            )))
        },
    );

    // transport.connection_close(conn) -> Result<(), string>
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        &mut module,
        "connection_close",
        "Close an established connection",
        "conn",
        "IoHandle",
        ConcreteType::Named("Result<(), string>".to_string()),
        |handle, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let mut guard = match handle.resource.lock() {
                Ok(g) => g,
                Err(_) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(
                        "transport.connection_close(): lock poisoned".to_string(),
                    )));
                }
            };
            let resource = match guard.as_mut() {
                Some(r) => r,
                None => {
                    // Already closed — Ok.
                    return Ok(TypedReturn::Ok(ConcreteReturn::Unit));
                }
            };
            if let IoResource::Custom(any) = resource {
                if let Some(boxed) = any.downcast_mut::<BoxedConnection>() {
                    let mut conn_guard = boxed.0.lock().map_err(|_| {
                        "transport.connection_close(): connection lock poisoned"
                            .to_string()
                    })?;
                    let result = match conn_guard.close() {
                        Ok(()) => TypedReturn::Ok(ConcreteReturn::Unit),
                        Err(e) => TypedReturn::Err(ConcreteReturn::String(format!(
                            "transport.connection_close(): {}",
                            e
                        ))),
                    };
                    // Drop the connection guard before clearing the handle.
                    drop(conn_guard);
                    *guard = None;
                    return Ok(result);
                }
            }
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "transport.connection_close(): handle is not a Connection".to_string(),
            )))
        },
    );

    // transport.memoized(max_entries?: int) -> IoHandle
    register_typed_fn_1_full::<_, i64>(
        &mut module,
        "memoized",
        "Create a memoized TCP transport that caches send results",
        [ModuleParam {
            name: "max_entries".to_string(),
            type_name: "int".to_string(),
            required: false,
            description: "Maximum cache entries (0 → default 1024)".to_string(),
            default_snippet: Some("1024".to_string()),
            ..Default::default()
        }],
        ConcreteType::IoHandle,
        |max_entries, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let inner = transport_provider::transport_provider()
                .create_transport(TransportKind::Tcp)
                .map_err(|e| format!("transport.memoized(): {}", e))?;
            let config = MemoConfig {
                max_entries: if max_entries > 0 {
                    max_entries as usize
                } else {
                    1024
                },
                enabled: true,
            };
            let memoized = Arc::new(MemoizedTransport::new(inner.clone(), config));
            // Erase to Arc<dyn Transport> for the field type — the
            // memoized wrapper itself implements Transport via the inner
            // Arc<dyn Transport>.
            let transport_view: Arc<dyn Transport> = memoized.clone();
            let handle = IoHandleData::new_custom(
                Box::new(TransportHandle {
                    transport: transport_view,
                    memoized: Some(memoized),
                }),
                "transport:memoized".to_string(),
            );
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(handle))))
        },
    );

    // transport.memo_stats(handle) -> { cache_hits, cache_misses, evictions, total_requests }
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        &mut module,
        "memo_stats",
        "Return cache statistics for a memoized transport",
        "handle",
        "IoHandle",
        ConcreteType::Named(
            "{ cache_hits: int, cache_misses: int, evictions: int, total_requests: int }"
                .to_string(),
        ),
        |handle, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let memo = extract_memoized(&handle, "memo_stats")?;
            let stats = memo.stats();
            Ok(TypedReturn::ObjectPairs(vec![
                (
                    "cache_hits".to_string(),
                    ConcreteReturn::I64(stats.cache_hits as i64),
                ),
                (
                    "cache_misses".to_string(),
                    ConcreteReturn::I64(stats.cache_misses as i64),
                ),
                (
                    "evictions".to_string(),
                    ConcreteReturn::I64(stats.evictions as i64),
                ),
                (
                    "total_requests".to_string(),
                    ConcreteReturn::I64(stats.total_requests as i64),
                ),
            ]))
        },
    );

    // transport.memo_invalidate(handle) -> ()
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        &mut module,
        "memo_invalidate",
        "Clear all cached entries in a memoized transport",
        "handle",
        "IoHandle",
        // Preserve the literal "()" surface as-is.
        ConcreteType::Named("()".to_string()),
        |handle, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let memo = extract_memoized(&handle, "memo_invalidate")?;
            memo.invalidate_all();
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    module
}

#[cfg(test)]
#[path = "transport_builtins_tests.rs"]
mod tests;
