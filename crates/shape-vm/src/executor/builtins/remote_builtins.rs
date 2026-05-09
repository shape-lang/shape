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
//!
//! ## Phase-2c deferral (ADR-006 §2.7.4)
//!
//! The pre-bulldozer bodies hand-marshalled `WireValue` results into
//! `ValueWord` via `from_string` / `from_i64` / `from_ok` / `from_err`
//! plus a bespoke `nb_to_serializable` walker that decoded values via the
//! deleted `tag_bits::*` dispatch (`is_tagged` / `get_tag` / `TAG_*`).
//! Every constructor in that path is deleted (CLAUDE.md "Forbidden
//! code") and `tag_bits::*` is forbidden by ADR-006 §2.7.7 #4 + #7.
//!
//! `register_typed_function`'s body shape is now
//! `Fn(&[KindedSlot], &ModuleContext) -> Result<TypedReturn, String>` and
//! `TypedReturn::ValueWord` is deleted (the pass-through escape hatch is
//! unrepresentable post-Phase-2a per `typed_module_exports.rs:179`). The
//! `remote.execute` / `remote.ping` / `remote.__call` projections need a
//! kind-threaded `WireValue → KindedSlot` marshal that lives in the
//! Phase-2c typed-module-exports rebuild — `WireValue::Object` / `Array`
//! members do not have a 1:1 `ConcreteReturn` projection today.
//!
//! Bodies are stubbed with
//! `todo!("phase-2c — typed-module-exports rebuild — see ADR-006 §2.7.4
//! + addendum")`. Module schema registration (parameter names, types,
//! descriptions, return-type strings) is preserved verbatim so LSP
//! completion and signature help remain functional.
//!
//! The thread-local `CURRENT_PROGRAM` and its `set_current_program` /
//! `clear_current_program` accessors are retained — they are the
//! protocol contract with `executor/vm_impl/modules.rs` (the VM stamps
//! the active program before each module dispatch). The Phase-2c body
//! rebuild will read through this same channel.

use shape_runtime::module_exports::{ModuleContext, ModuleExports, ModuleParam};
use shape_runtime::typed_module_exports::{ConcreteType, TypedReturn, register_typed_function};
use shape_value::KindedSlot;
use std::cell::RefCell;

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

/// Phase-2c stub body shared by every remote export.
///
/// The variadic registration shape is
/// `Fn(&[KindedSlot], &ModuleContext) -> Result<TypedReturn, String>`. Until
/// the typed-module-exports rebuild lands a `KindedSlot`-shaped
/// `WireValue`/`SerializableVMValue` projection (see module-level
/// comment), every body returns the phase-2c `todo!(...)` macro which
/// surfaces the deferral at the first invocation rather than silently
/// materializing a wrong value.
fn phase_2c_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — typed-module-exports rebuild — see ADR-006 §2.7.4 + addendum")
}

/// Create the `remote` module with remote execution functions.
pub fn create_remote_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::remote");
    module.description = "Remote execution on Shape serve instances".to_string();

    // remote.execute(addr, code) -> Result<{ value, stdout, error }, string>
    // The original LSP surface is a complex inline-record + Result<…, string>;
    // ConcreteType::Named preserves the literal string for LSP/schema fidelity.
    register_typed_function(
        &mut module,
        "execute",
        "Execute Shape code on a remote server",
        vec![
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
        ConcreteType::Named(
            "Result<{ value, stdout: string?, error: string? }, string>".to_string(),
        ),
        phase_2c_stub,
    );

    // remote.ping(addr) -> Result<{ shape_version: string, wire_protocol: int }, string>
    register_typed_function(
        &mut module,
        "ping",
        "Ping a remote Shape server and get server info",
        vec![ModuleParam {
            name: "addr".to_string(),
            type_name: "string".to_string(),
            required: true,
            description: "Remote server address as host:port".to_string(),
            ..Default::default()
        }],
        ConcreteType::Named(
            "Result<{ shape_version: string, wire_protocol: int }, string>".to_string(),
        ),
        phase_2c_stub,
    );

    // remote.__call(addr, fn_ref, args) -> Result<_, string>
    register_typed_function(
        &mut module,
        "__call",
        "Call a function on a remote Shape server",
        vec![
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
        ConcreteType::Named("Result<_, string>".to_string()),
        phase_2c_stub,
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

    // Track A.4 cross-node closure decode tests (`test_a4_cross_node_closure_decode_with_layout`,
    // `test_a4_nested_closure_in_array_decodes_with_layout`) drove
    // `serializable_to_nb` (deleted — used `ValueWord::from_*`,
    // `as_closure_handle`, `from_heap_value`, and `serializable_to_nanboxed_with_layouts`,
    // all forbidden post-bulldozer). Re-author with the kind-threaded
    // marshal layer when the Phase-2c typed-module-exports rebuild lands
    // (ADR-006 §2.7.4 + addendum).
}
