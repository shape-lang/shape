//! ValueWord <-> MessagePack marshaling for foreign function calls.
//!
//! ADR-006 §2.7.4 / §2.7.5 SURFACE: this module crosses the cross-crate
//! ABI boundary — extension contracts stay on raw u64 per §2.7.5, but
//! every internal Rust dispatch path (rmpv encoding/decoding, msgpack
//! schema population, typed-object field write) consumed the deleted
//! `ValueWord` / `tag_bits` / `as_heap_ref` / `vmarray_from_vec` /
//! `ArgVec` surfaces. Per the B11-control-flow-heap dispatch (ADR-006
//! §2.7.4 cross-crate ABI consumer-side migration policy), the body is
//! stubbed pending phase-2c rebuild: thread `&[KindedSlot]` through the
//! marshal sites with raw u64 retained only at the §2.7.5 FFI extension
//! contract surface.
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
