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
//!
//! ## Phase-2c deferral (ADR-006 §2.7.4)
//!
//! The function bodies were authored against the pre-bulldozer `ValueWord`
//! shape (`fn(&[ValueWord], &ModuleContext) -> Result<ValueWord, String>`)
//! and used the now-deleted `ValueWord::from_io_handle` / `from_array` /
//! `from_ok` / `from_err` constructors plus `as_io_handle` / `as_str` /
//! `as_any_array` / `as_number_coerce` accessors. The variadic
//! `register_typed_function` body shape is now
//! `Fn(&[KindedSlot], &ModuleContext) -> Result<TypedReturn, String>` and
//! `TypedReturn::ValueWord` is removed (the pass-through escape hatch is
//! unrepresentable post-Phase-2a per `typed_module_exports.rs:179`).
//!
//! The IoHandle-bearing transport bodies (handle materialization at
//! construction, `IoResource::Custom` downcast at use) are not yet
//! expressible as a `TypedReturn` projection: `TypedReturn::Concrete(
//! ConcreteReturn::IoHandle(Arc<IoHandleData>))` exists, but `Result<
//! IoHandle, string>` requires either an `OkIoHandle` wrapper variant or
//! a generic `Ok(ConcreteReturn::IoHandle)` projection lowered through
//! the kind-threaded marshal layer. That work is Phase-2c
//! typed-module-exports rebuild (per ADR-006 §2.7.4 + addendum) — not
//! Wave-β R-transport-remote scope. Bodies are stubbed with
//! `todo!("phase-2c — typed-module-exports rebuild — see ADR-006 §2.7.4
//! + addendum")`.
//!
//! Module schema registration (parameter names, types, descriptions,
//! return-type strings) is preserved verbatim so LSP completion and
//! signature help remain functional. Only the executable bodies are
//! deferred.

use shape_runtime::module_exports::{ModuleContext, ModuleExports, ModuleParam};
use shape_runtime::typed_module_exports::{ConcreteType, TypedReturn, register_typed_function};
use shape_value::KindedSlot;
use shape_wire::transport::{Connection, Transport};
use std::sync::Arc;

/// Type-erased transport handle stored in `IoResource::Custom`.
///
/// Retained as a structural type so future Phase-2c bodies can downcast
/// `IoResource::Custom` payloads without a parallel layout shuffle.
#[allow(dead_code)]
pub(super) struct TransportHandle {
    pub(super) transport: Arc<dyn Transport>,
    pub(super) memoized: Option<Arc<shape_wire::transport::memoized::MemoizedTransport<Arc<dyn Transport>>>>,
}

/// Wrapper for `Box<dyn Connection>` so it can be stored in `IoResource::Custom`
/// (which requires `Any + Send`). The inner `Mutex` allows mutable access through
/// the shared IoHandle reference.
#[allow(dead_code)]
pub(super) struct BoxedConnection(pub(super) std::sync::Mutex<Box<dyn Connection>>);

/// Phase-2c stub body shared by every transport export.
///
/// The variadic registration shape is
/// `Fn(&[KindedSlot], &ModuleContext) -> Result<TypedReturn, String>`. Until
/// the typed-module-exports rebuild lands a `KindedSlot`-shaped IoHandle
/// projection (see module-level comment), every body returns the
/// phase-2c `todo!(...)` macro which surfaces the deferral at the first
/// invocation rather than silently materializing a wrong value.
fn phase_2c_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — typed-module-exports rebuild — see ADR-006 §2.7.4 + addendum")
}

/// Create the `transport` module with TCP transport functions.
pub fn create_transport_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::transport");
    module.description = "Network transport for distributed Shape".to_string();

    // transport.tcp() -> Transport
    register_typed_function(
        &mut module,
        "tcp",
        "Create a TCP transport handle",
        vec![],
        ConcreteType::Named("Transport".to_string()),
        phase_2c_stub,
    );

    // transport.quic() -> Transport  (requires `quic` feature)
    #[cfg(feature = "quic")]
    register_typed_function(
        &mut module,
        "quic",
        "Create a QUIC transport handle (multiplexed, encrypted)",
        vec![],
        ConcreteType::Named("Transport".to_string()),
        phase_2c_stub,
    );

    // transport.send(transport, destination, payload) -> Result<Array<int>, string>
    register_typed_function(
        &mut module,
        "send",
        "Send a payload to a destination and wait for a length-prefixed response",
        vec![
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
        ConcreteType::Result2(
            Box::new(ConcreteType::ArrayInt),
            Box::new(ConcreteType::String),
        ),
        phase_2c_stub,
    );

    // transport.connect(transport, destination) -> Result<Connection, string>
    register_typed_function(
        &mut module,
        "connect",
        "Establish a persistent TCP connection to a remote node",
        vec![
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
        ConcreteType::Result2(
            Box::new(ConcreteType::Named("Connection".to_string())),
            Box::new(ConcreteType::String),
        ),
        phase_2c_stub,
    );

    // transport.connection_send(conn, payload) -> Result<(), string>
    register_typed_function(
        &mut module,
        "connection_send",
        "Send a length-prefixed payload over an established connection",
        vec![
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
        // Original LSP surface was "Result<(), string>"; ConcreteType::Result2
        // emits "Result<unit, string>". Use Named to preserve the literal.
        ConcreteType::Named("Result<(), string>".to_string()),
        phase_2c_stub,
    );

    // transport.connection_recv(conn, timeout?) -> Result<Array<int>, string>
    register_typed_function(
        &mut module,
        "connection_recv",
        "Receive a length-prefixed payload from an established connection",
        vec![
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
        ConcreteType::Result2(
            Box::new(ConcreteType::ArrayInt),
            Box::new(ConcreteType::String),
        ),
        phase_2c_stub,
    );

    // transport.connection_close(conn) -> Result<(), string>
    register_typed_function(
        &mut module,
        "connection_close",
        "Close an established connection",
        vec![ModuleParam {
            name: "conn".to_string(),
            type_name: "Connection".to_string(),
            required: true,
            description: "Connection handle from transport.connect()".to_string(),
            ..Default::default()
        }],
        ConcreteType::Named("Result<(), string>".to_string()),
        phase_2c_stub,
    );

    // transport.memoized(max_entries?) -> MemoTransport
    register_typed_function(
        &mut module,
        "memoized",
        "Create a memoized TCP transport that caches send results",
        vec![ModuleParam {
            name: "max_entries".to_string(),
            type_name: "int".to_string(),
            required: false,
            description: "Maximum cache entries (default 1024)".to_string(),
            ..Default::default()
        }],
        ConcreteType::Named("MemoTransport".to_string()),
        phase_2c_stub,
    );

    // transport.memo_stats(handle) -> { cache_hits, cache_misses, evictions, total_requests }
    register_typed_function(
        &mut module,
        "memo_stats",
        "Return cache statistics for a memoized transport",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "MemoTransport".to_string(),
            required: true,
            description: "Memoized transport handle from transport.memoized()".to_string(),
            ..Default::default()
        }],
        ConcreteType::Named(
            "{ cache_hits: int, cache_misses: int, evictions: int, total_requests: int }"
                .to_string(),
        ),
        phase_2c_stub,
    );

    // transport.memo_invalidate(handle) -> ()
    register_typed_function(
        &mut module,
        "memo_invalidate",
        "Clear all cached entries in a memoized transport",
        vec![ModuleParam {
            name: "handle".to_string(),
            type_name: "MemoTransport".to_string(),
            required: true,
            description: "Memoized transport handle from transport.memoized()".to_string(),
            ..Default::default()
        }],
        // Preserve the literal "()" surface as-is.
        ConcreteType::Named("()".to_string()),
        phase_2c_stub,
    );

    module
}

#[cfg(test)]
#[path = "transport_builtins_tests.rs"]
mod tests;
