//! Shape-value <-> MessagePack marshaling for foreign function calls.
//!
//! ADR-006 §2.7.4 / §2.7.5 / §2.7.6 SURFACE: this module is the
//! Rust-side carrier shape for foreign function (extern C / Python /
//! TypeScript) call args and results, sitting between the byte-level
//! msgpack wire and the runtime-tier `KindedSlot` carrier. Per
//! §2.7.5, the extension contract via `*mut c_void` stays on raw u64
//! (the `RawCallableInvoker.invoke` signature in `module_exports.rs`
//! is the stable-ABI surface); the conversion to/from `KindedSlot`
//! happens **inside shape-vm at this boundary**, not at the
//! extension call frame.
//!
//! W8-EX status: signatures speak the §2.7.6 / Q8 carrier
//! (`&[KindedSlot]` for args, `Result<KindedSlot, VMError>` for
//! results) — same vocabulary the project speaks at every other
//! internal Rust dispatch boundary (§2.7.10 method dispatch,
//! §2.7.11 value-call dispatch, exception payload). Bodies remain
//! Phase-2c per §2.7.4 because they depend on the rmpv ↔
//! `KindedSlot::from_<v>` per-NativeKind dispatch + per-FieldType
//! TypedObject construction landing — both Phase-2c surface areas.
//!
//! The pre-rebuild body (now deleted) handled four input shapes:
//!
//! 1. `marshal_args(&[ValueWord], TypeSchemaRegistry)` —
//!    `ValueWord -> rmpv::Value` per arg, then `rmp_serde::to_vec` the
//!    array.
//! 2. `unmarshal_result(bytes, return_type, schema_id, registry)` —
//!    `rmp_serde::from_slice` then `typed_msgpack_to_nanboxed` per
//!    target type.
//! 3. `nanboxed_to_msgpack_value(&ValueWord, &TypeSchemaRegistry)` —
//!    tag-bit dispatch on the input ValueWord routing to scalar /
//!    string / array / typed-object encoding.
//! 4. `marshal_typed_object(entries, schema_id, registry)` — schema-
//!    driven `TypedObjectStorage` construction with per-FieldType
//!    `ValueSlot::from_*` writes and a heap_mask track.
//!
//! Rebuild plan: the public entry points keep their byte-level
//! signatures (raw msgpack on the wire) but accept / produce
//! `KindedSlot` instead of `ValueWord` on the runtime side; the
//! per-FieldType writers use `KindedSlot::from_<v>` constructors and
//! `slot.as_heap_value()` + `HeapValue::*` match (single discriminator
//! per ADR-005 §1) for the heap-bearing arms.

use shape_runtime::type_schema::TypeSchemaRegistry;
use shape_value::{KindedSlot, VMError};

const PHASE_2C_FFI_REBUILD_SURFACE: &str =
    "phase-2c — extern C / foreign-runtime FFI rebuild (ADR-006 §2.7.4 / §2.7.5)";

/// Phase-2c surface stub. Pre-rebuild body was a `Vec<ValueWord> ->
/// msgpack` encoder; rebuild target is `&[KindedSlot] -> msgpack` with
/// per-`NativeKind` dispatch at the body site (§2.7.6).
pub fn marshal_args(
    _args: &[KindedSlot],
    _schemas: &TypeSchemaRegistry,
) -> Result<Vec<u8>, VMError> {
    Err(VMError::NotImplemented(format!(
        "foreign_marshal::marshal_args: {}",
        PHASE_2C_FFI_REBUILD_SURFACE
    )))
}

/// Phase-2c surface stub. Pre-rebuild body was a `msgpack -> ValueWord`
/// decoder (`typed_msgpack_to_nanboxed`); rebuild target produces a
/// `KindedSlot` whose `kind` is sourced from the declared `return_type`
/// + the wire bytes (per §2.7.6 carrier bound + §2.7.5 wire-format
/// post-proof discipline).
pub fn unmarshal_result(
    _bytes: &[u8],
    _return_type: &str,
    _schema_id: Option<u32>,
    _schemas: &TypeSchemaRegistry,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(format!(
        "foreign_marshal::unmarshal_result: {}",
        PHASE_2C_FFI_REBUILD_SURFACE
    )))
}
