//! Native `remote` module for executing Shape code on remote `shape serve` instances.
//!
//! ADR-006 §2.7.28 W17-typed-module-exports 2026-05-23
//!
//! Provides a high-level abstraction over the wire protocol so users can
//! execute code or call functions on a remote Shape server directly from
//! Shape code, without manually encoding wire messages.
//!
//! Exports:
//! - remote.execute(addr, code) -> Result<{ value, stdout, error }, string>
//! - remote.ping(addr) -> Result<{ shape_version: string, wire_protocol: int }, string>
//! - remote.__call(addr, fn_ref, args) -> Result<_, string>
//!
//! ## W17-typed-module-exports rebuild (ADR-006 §2.7.4 + §2.7.4-addendum)
//!
//! Phase-2c R8-W2 (2026-05-23): bodies rebuilt on top of the kind-threaded
//! marshal layer per ADR-006 §2.7.4 + addendum. The variadic
//! `register_typed_function` shape is used here (rather than the
//! per-arity helpers) because the `remote.__call`'s `args: Array<_>`
//! parameter is heterogeneous — it carries closure / typed-object /
//! generic-value payloads that cannot be projected into a single Rust
//! `Vec<T>` at registration time.
//!
//! ### Polymorphic-value boundary (typed-Arc payload protocol)
//!
//! The `remote.execute`'s `value` field and `remote.__call`'s success
//! payload are polymorphic — the server returns whatever Shape value the
//! user code produced. Pre-bulldozer, the path was `WireValue` →
//! `wire_to_nb` (deleted; used the deleted `tag_bits::*` /
//! `ValueWord::from_*` constructors) → `ValueWord`.
//!
//! The W17-typed-module-exports protocol projects polymorphic values
//! through `ConcreteReturn::JsonValue` — the existing strict-typed
//! parsed-data tree (`Null/Bool/Int/Number/String/Bytes/Array/Object`).
//! This is the same projection that `json.parse(text) -> Result<Json>`
//! uses for polymorphic-content parsing (`json.rs:411`). User code
//! receives the value as a `Json` and pattern-matches the variants.
//!
//! Non-JSON-projectable wire values (Table, Range, FunctionRef, Content)
//! surface a `JsonValue::String("<wire-variant-name:phase-2c>")` placeholder
//! consistent with the existing wire-side deferral pattern. Full
//! structured projection for those variants is downstream sub-cluster
//! work (W17-typed-module-exports-followup: typed-payload-projection)
//! per the audit's §6.4 addendum scope.
//!
//! The thread-local `CURRENT_PROGRAM` and its
//! `set_current_program` / `clear_current_program` accessors are
//! retained — they are the protocol contract with
//! `executor/vm_impl/modules.rs` (the VM stamps the active program
//! before each module dispatch).

use shape_runtime::json_value::JsonValue;
use shape_runtime::marshal::{register_typed_fn_1, register_typed_fn_2};
use shape_runtime::module_exports::ModuleExports;
use shape_runtime::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_wire::WireValue;
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

// ---------------------------------------------------------------------------
// Wire protocol helpers
// ---------------------------------------------------------------------------

/// Send a `WireMessage` to a remote server and receive the response.
///
/// One-shot send via a fresh TCP transport. The transport handles
/// length-prefix framing internally; this helper only encodes/decodes
/// MessagePack at the message boundary.
fn wire_roundtrip(
    addr: &str,
    msg: &crate::remote::WireMessage,
) -> Result<crate::remote::WireMessage, String> {
    let transport: Arc<dyn Transport> = transport_provider::transport_provider()
        .create_transport(TransportKind::Tcp)
        .map_err(|e| format!("remote: failed to create transport: {}", e))?;

    let mp = shape_wire::encode_message(msg).map_err(|e| format!("remote: encode error: {}", e))?;

    let response_bytes = transport
        .send(addr, &mp)
        .map_err(|e| format!("remote: transport error: {}", e))?;

    shape_wire::decode_message(&response_bytes).map_err(|e| format!("remote: decode error: {}", e))
}

/// Project a `WireValue` into the strict-typed `JsonValue` tree.
///
/// W17-typed-module-exports typed-Arc payload protocol (ADR-006 §2.7.4
/// addendum): polymorphic wire values cross the module-symbol-table
/// boundary as `JsonValue`. Mirrors the json.rs parse-side projection
/// shape (`stdlib/json.rs::serde_json_to_json_value`) — leaf variants
/// project directly, structural variants recurse at the JsonValue
/// layer.
///
/// Non-JSON-projectable variants (Table, Range, FunctionRef, Content,
/// PrintResult, Duration, Timestamp) surface a placeholder string so
/// the caller sees the variant name. Full structured projection for
/// those is downstream sub-cluster work
/// (W17-typed-module-exports-followup: typed-payload-projection).
fn wire_to_json_value(wire: &WireValue) -> JsonValue {
    match wire {
        WireValue::Null => JsonValue::Null,
        WireValue::Bool(b) => JsonValue::Bool(*b),
        WireValue::Number(n) => JsonValue::Number(*n),
        WireValue::Integer(i) => JsonValue::Int(*i),
        WireValue::I8(v) => JsonValue::Int(*v as i64),
        WireValue::I16(v) => JsonValue::Int(*v as i64),
        WireValue::I32(v) => JsonValue::Int(*v as i64),
        WireValue::I64(v) => JsonValue::Int(*v),
        WireValue::U8(v) => JsonValue::Int(*v as i64),
        WireValue::U16(v) => JsonValue::Int(*v as i64),
        WireValue::U32(v) => JsonValue::Int(*v as i64),
        WireValue::U64(v) => JsonValue::Int(*v as i64),
        WireValue::Isize(v) => JsonValue::Int(*v),
        WireValue::Usize(v) => JsonValue::Int(*v as i64),
        WireValue::Ptr(v) => JsonValue::Int(*v as i64),
        WireValue::F32(v) => JsonValue::Number(*v as f64),
        WireValue::String(s) => JsonValue::String(s.clone()),
        WireValue::Array(items) => JsonValue::Array(items.iter().map(wire_to_json_value).collect()),
        WireValue::Object(map) => JsonValue::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), wire_to_json_value(v)))
                .collect(),
        ),
        WireValue::Result { ok, value } => {
            // Preserve the Result discriminator at the JsonValue layer via
            // a tagged object — caller can pattern-match on the literal
            // field names. Mirrors the shape that wire-side decoders
            // produce when there's no direct Result variant.
            let payload = wire_to_json_value(value);
            let tag = if *ok { "Ok" } else { "Err" };
            JsonValue::Object(vec![(tag.to_string(), payload)])
        }
        WireValue::Timestamp(ms) => JsonValue::Int(*ms),
        WireValue::Duration { value, unit } => JsonValue::Object(vec![
            ("__duration".to_string(), JsonValue::Number(*value)),
            (
                "__unit".to_string(),
                JsonValue::String(format!("{:?}", unit)),
            ),
        ]),
        // Non-JSON-projectable variants surface their wire-variant name
        // as a structured placeholder. The typed-Arc protocol for these
        // is downstream W17-typed-module-exports-followup territory.
        WireValue::Table(_) => JsonValue::String("<wire:Table:phase-2c>".to_string()),
        WireValue::Range { .. } => JsonValue::String("<wire:Range:phase-2c>".to_string()),
        WireValue::FunctionRef { name } => {
            JsonValue::String(format!("<wire:FunctionRef:{}>", name))
        }
        WireValue::PrintResult(_) => JsonValue::String("<wire:PrintResult:phase-2c>".to_string()),
        WireValue::Content(_) => JsonValue::String("<wire:Content:phase-2c>".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Module factory
// ---------------------------------------------------------------------------

/// Create the `remote` module with remote execution functions.
pub fn create_remote_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::remote");
    module.description = "Remote execution on Shape serve instances".to_string();

    // remote.execute(addr, code) -> Result<{ value, stdout, error }, string>
    //
    // Sends Shape source code to a remote `shape serve` instance for
    // execution. The success arm returns a typed object with `value`
    // (the polymorphic execution result projected as `Json`), `stdout`
    // (captured stdout, or `null` when none), and `error` (always
    // `null` in the success arm — present for shape consistency with
    // the failure pattern that pre-bulldozer Shape code expected).
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "execute",
        "Execute Shape code on a remote server",
        [("addr", "string"), ("code", "string")],
        ConcreteType::Named(
            "Result<{ value, stdout: string?, error: string? }, string>".to_string(),
        ),
        |addr, code, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let msg = crate::remote::WireMessage::Execute(crate::remote::ExecuteRequest {
                code: (*code).clone(),
                request_id: 1,
            });
            let response = match wire_roundtrip(addr.as_str(), &msg) {
                Ok(r) => r,
                Err(e) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                        "remote.execute(): {}",
                        e
                    ))));
                }
            };
            match response {
                crate::remote::WireMessage::ExecuteResponse(r) => {
                    if r.success {
                        let value = ConcreteReturn::JsonValue(wire_to_json_value(&r.value));
                        let stdout = match r.stdout {
                            Some(s) => ConcreteReturn::String(s),
                            None => ConcreteReturn::String(String::new()),
                        };
                        // Per W17-typed-module-exports protocol, the
                        // success-arm `error` field projects as the
                        // empty string (matches pre-bulldozer null-
                        // sentinel semantics: empty = no error).
                        let error = ConcreteReturn::String(String::new());
                        Ok(TypedReturn::OkObjectPairs(vec![
                            ("value".to_string(), value),
                            ("stdout".to_string(), stdout),
                            ("error".to_string(), error),
                        ]))
                    } else {
                        let msg = r.error.unwrap_or_else(|| "unknown error".to_string());
                        Ok(TypedReturn::Err(ConcreteReturn::String(msg)))
                    }
                }
                other => Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                    "remote.execute(): unexpected response type: {:?}",
                    std::mem::discriminant(&other),
                )))),
            }
        },
    );

    // remote.ping(addr) -> Result<{ shape_version: string, wire_protocol: int }, string>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "ping",
        "Ping a remote Shape server and get server info",
        "addr",
        "string",
        ConcreteType::Named(
            "Result<{ shape_version: string, wire_protocol: int }, string>".to_string(),
        ),
        |addr, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let msg = crate::remote::WireMessage::Ping(crate::remote::PingRequest {});
            let response = match wire_roundtrip(addr.as_str(), &msg) {
                Ok(r) => r,
                Err(e) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                        "remote.ping(): {}",
                        e
                    ))));
                }
            };
            match response {
                crate::remote::WireMessage::Pong(info) => Ok(TypedReturn::OkObjectPairs(vec![
                    (
                        "shape_version".to_string(),
                        ConcreteReturn::String(info.shape_version),
                    ),
                    (
                        "wire_protocol".to_string(),
                        ConcreteReturn::I64(info.wire_protocol as i64),
                    ),
                ])),
                other => Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                    "remote.ping(): unexpected response type: {:?}",
                    std::mem::discriminant(&other),
                )))),
            }
        },
    );

    // remote.__call(addr, fn_ref, args) -> Result<_, string>
    //
    // SURFACE-and-stop per ADR-006 §2.7.8 / Q10 cell-storage
    // parallel-kind: closure handles (`fn_ref`) carry per-capture kind
    // metadata that lives on the closure header. The pre-bulldozer
    // path materialised that metadata via deleted `as_closure_handle`
    // + `captures_as_values` accessors. The kind-threaded rebuild is
    // bounded to the W17-typed-module-exports-followup sub-cluster
    // (typed-payload-projection territory) — out of W17-typed-module-
    // exports scope per audit §2.2. Body returns a structured error
    // rather than dispatch a possibly-wrong call.
    //
    // The registration is preserved so LSP signature help + completion
    // continues to surface the export.
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "__call",
        "Call a function on a remote Shape server",
        // Signature uses (addr, fn_name) at the marshal-typed boundary
        // for the W17-typed-module-exports landing; the heterogeneous
        // `args: Array<_>` parameter projects through the followup
        // sub-cluster. Body errors at the boundary if reached.
        [("addr", "string"), ("fn_name", "string")],
        ConcreteType::Named("Result<_, string>".to_string()),
        |_addr, _fn_name, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            Ok(TypedReturn::Err(ConcreteReturn::String(
                "remote.__call() requires upvalue kind track \
                 (ADR-006 §2.7.8 / Q10 cell-storage parallel-kind) — \
                 not yet wired through the typed-module-exports boundary; \
                 use remote.execute(addr, code) for source-level dispatch"
                    .to_string(),
            )))
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_module_creation() {
        let module = create_remote_module();
        assert_eq!(module.name, "std::core::remote");
        assert!(module.has_export("execute"));
        assert!(module.has_export("ping"));
        assert!(module.has_export("__call"));
    }

    #[test]
    fn test_wire_to_json_primitives() {
        assert_eq!(wire_to_json_value(&WireValue::Null), JsonValue::Null);
        assert_eq!(
            wire_to_json_value(&WireValue::Bool(true)),
            JsonValue::Bool(true)
        );
        assert_eq!(
            wire_to_json_value(&WireValue::Integer(42)),
            JsonValue::Int(42)
        );
        assert_eq!(
            wire_to_json_value(&WireValue::Number(3.14)),
            JsonValue::Number(3.14)
        );
        assert_eq!(
            wire_to_json_value(&WireValue::String("hello".to_string())),
            JsonValue::String("hello".to_string())
        );
    }

    #[test]
    fn test_wire_to_json_structural() {
        let arr = WireValue::Array(vec![
            WireValue::Integer(1),
            WireValue::Integer(2),
            WireValue::Integer(3),
        ]);
        match wire_to_json_value(&arr) {
            JsonValue::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], JsonValue::Int(1));
                assert_eq!(items[2], JsonValue::Int(3));
            }
            other => panic!("expected JsonValue::Array, got {:?}", other),
        }
    }

    #[test]
    fn test_wire_result_to_json_tagged_object() {
        let ok_wire = WireValue::Result {
            ok: true,
            value: Box::new(WireValue::Integer(42)),
        };
        match wire_to_json_value(&ok_wire) {
            JsonValue::Object(pairs) => {
                assert_eq!(pairs.len(), 1);
                assert_eq!(pairs[0].0, "Ok");
                assert_eq!(pairs[0].1, JsonValue::Int(42));
            }
            other => panic!("expected JsonValue::Object(Ok), got {:?}", other),
        }
    }

    // Track A.4 cross-node closure decode tests
    // (`test_a4_cross_node_closure_decode_with_layout`,
    // `test_a4_nested_closure_in_array_decodes_with_layout`) drove the
    // deleted `serializable_to_nb` path. Their re-author lands at the
    // W17-typed-module-exports-followup sub-cluster (typed-payload-
    // projection) — pairs with the `remote.__call` body rebuild.
}
