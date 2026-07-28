//! Schema-backed `__Option` constructors for MIR JIT.
//!
//! These are the active JIT producer ABI for `Some` / `None`. They return the
//! same v2-raw `TypedObjectStorage` carrier built by the VM's
//! `result_option_carrier`, not the retired `Arc<OptionData>` shape.

use crate::ffi::value_ffi::TAG_NULL;
use shape_runtime::type_schema::builtin_schemas::{
    OPTION_PAYLOAD, OPTION_VARIANT_NONE, OPTION_VARIANT_SOME,
};
use shape_value::NativeKind;
use shape_value::heap_value::{HeapKind, TypedObjectStorage};
use shape_value::slot::ValueSlot;
use std::sync::Arc;

fn resolve_option_schema_id() -> Option<u64> {
    crate::ffi::control::with_trampoline_vm(|vm| {
        shape_runtime::type_schema::builtin_schemas::resolve_builtin_schema_ids(
            &vm.program().type_schema_registry,
        )
        .map(|ids| ids.option as u64)
    })
    .flatten()
    .or_else(|| {
        let (_registry, ids) =
            shape_runtime::type_schema::TypeSchemaRegistry::with_stdlib_types_and_builtin_ids();
        Some(ids.option as u64)
    })
}

fn payload_field_owns_heap_share(bits: u64, kind: NativeKind) -> bool {
    if bits == 0 {
        return false;
    }
    match kind {
        NativeKind::String | NativeKind::StringV2 | NativeKind::DecimalV2 => true,
        NativeKind::Ptr(hk) => match hk {
            HeapKind::Future | HeapKind::ModuleFn | HeapKind::Char | HeapKind::NativeScalar => {
                false
            }
            HeapKind::String
            | HeapKind::TypedObject
            | HeapKind::Closure
            | HeapKind::Decimal
            | HeapKind::BigInt
            | HeapKind::DataTable
            | HeapKind::TaskGroup
            | HeapKind::TypedArray
            | HeapKind::Temporal
            | HeapKind::TableView
            | HeapKind::Content
            | HeapKind::Instant
            | HeapKind::IoHandle
            | HeapKind::NativeView
            | HeapKind::HashMap
            | HeapKind::FilterExpr
            | HeapKind::Reference
            | HeapKind::SharedCell
            | HeapKind::HashSet
            | HeapKind::Iterator
            | HeapKind::Deque
            | HeapKind::Channel
            | HeapKind::PriorityQueue
            | HeapKind::Range
            | HeapKind::Result
            | HeapKind::Option
            | HeapKind::TraitObject
            | HeapKind::Mutex
            | HeapKind::Atomic
            | HeapKind::Lazy
            | HeapKind::Matrix
            | HeapKind::MatrixSlice => true,
        },
        _ => false,
    }
}

fn build_option_object(variant: i64, payload_bits: u64, payload_kind: NativeKind) -> u64 {
    let Some(schema_id) = resolve_option_schema_id() else {
        tracing::debug!(
            target: "shape_jit",
            "jit_schema_option: SURFACE — builtin __Option schema not registered",
        );
        return TAG_NULL;
    };

    let slots = vec![
        ValueSlot::from_int(variant),
        ValueSlot::from_raw(payload_bits),
    ]
    .into_boxed_slice();
    let heap_mask = if payload_field_owns_heap_share(payload_bits, payload_kind) {
        1u64 << OPTION_PAYLOAD
    } else {
        0
    };
    let field_kinds: Arc<[NativeKind]> =
        Arc::from(vec![NativeKind::Int64, payload_kind].into_boxed_slice());
    let ptr = TypedObjectStorage::_new(schema_id, slots, heap_mask, field_kinds);
    ptr as u64
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_schema_option_some(payload_bits: u64, payload_kind_code: u8) -> u64 {
    let Some(payload_kind) = crate::ffi::stack_kind_code::decode(payload_kind_code) else {
        tracing::debug!(
            target: "shape_jit",
            payload_kind_code,
            "jit_schema_option_some: SURFACE — payload kind code not proven",
        );
        return TAG_NULL;
    };
    build_option_object(OPTION_VARIANT_SOME, payload_bits, payload_kind)
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_schema_option_none() -> u64 {
    build_option_object(OPTION_VARIANT_NONE, 0, NativeKind::Null)
}
