//! Tests for the `transport` native module.
//!
//! ## Phase-2c deferral (ADR-006 §2.7.4)
//!
//! The pre-bulldozer test suite drove every export through its body
//! function (`transport_tcp`, `transport_send`, `transport_connect`,
//! `connection_send_fn`, `connection_recv_fn`, `connection_close_fn`,
//! `transport_memoized`, `transport_memo_stats`,
//! `transport_memo_invalidate`) using `ValueWord::from_io_handle` /
//! `from_string` / `from_i64` arguments and `as_heap_ref()` /
//! `HeapValue::Ok(...)` / `HeapValue::Err(...)` / `to_array_arc()`
//! assertions on the returned `ValueWord`. Every one of those APIs is
//! deleted (see CLAUDE.md "Forbidden code" / "Renames to refuse on
//! sight" / ADR-006 §2.7.7 forbidden #1, #4, #7), and the body
//! functions themselves are now phase-2c stubs (see
//! `transport_builtins.rs` module-level comment).
//!
//! The end-to-end behavior tests (length-prefixed framing roundtrip,
//! connect/send/recv on a live TCP listener, memoized cache hits) belong
//! with the Phase-2c typed-module-exports rebuild — once the bodies
//! return `Result<TypedReturn, String>` and the `Result<IoHandle, _>`
//! / `Result<Array<int>, _>` projections lower through the kind-threaded
//! marshal layer, the tests can be re-authored against `KindedSlot`
//! arguments and `TypedReturn` assertions.
//!
//! Until then, the only assertion the territory can soundly make is
//! that the module factory still publishes the export schema that the
//! LSP / JIT consult for completion and signature help.

#[allow(unused_imports)]
use super::*;

#[test]
fn test_create_transport_module() {
    let module = create_transport_module();
    assert_eq!(module.name, "std::core::transport");
    assert!(module.has_export("tcp"));
    assert!(module.has_export("send"));
    assert!(module.has_export("connect"));
    assert!(module.has_export("connection_send"));
    assert!(module.has_export("connection_recv"));
    assert!(module.has_export("connection_close"));
    assert!(module.has_export("memoized"));
    assert!(module.has_export("memo_stats"));
    assert!(module.has_export("memo_invalidate"));
}

#[test]
fn test_transport_builtins_has_no_tcpstream_fallback() {
    let src = include_str!("transport_builtins.rs");
    assert!(
        !src.contains("IoResource::TcpStream"),
        "transport_builtins must not bypass shape-wire Connection via IoResource::TcpStream"
    );
}
