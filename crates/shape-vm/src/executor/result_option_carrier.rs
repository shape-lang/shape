//! Canonical Result/Option runtime carriers.
//!
//! L5 owns the runtime carrier shape: `Result<T, E>` and `Option<T>` are
//! fixed-layout TypedObjects, not separate `HeapKind` variants. This module is
//! the single VM helper for constructing and reading those objects.

use shape_runtime::type_schema::builtin_schemas::{
    BuiltinSchemaIds, OPTION_PAYLOAD, OPTION_VARIANT, OPTION_VARIANT_NONE, OPTION_VARIANT_SOME,
    RESULT_PAYLOAD, RESULT_VARIANT, RESULT_VARIANT_ERR, RESULT_VARIANT_OK,
};
use shape_value::{HeapKind, KindedSlot, NativeKind, TypedObjectStorage, VMError, ValueSlot};
use std::sync::Arc;

pub(crate) struct ResultCarrier<'a> {
    storage: &'a TypedObjectStorage,
}

impl ResultCarrier<'_> {
    #[inline]
    pub(crate) fn is_ok(&self) -> bool {
        read_variant_tag(self.storage, RESULT_VARIANT).expect("validated Result variant")
            == RESULT_VARIANT_OK
    }

    #[inline]
    pub(crate) fn clone_payload(&self) -> Result<KindedSlot, VMError> {
        clone_payload(self.storage, RESULT_PAYLOAD, "__Result")
    }
}

pub(crate) struct OptionCarrier<'a> {
    storage: &'a TypedObjectStorage,
}

impl OptionCarrier<'_> {
    #[inline]
    pub(crate) fn is_some(&self) -> bool {
        read_variant_tag(self.storage, OPTION_VARIANT).expect("validated Option variant")
            == OPTION_VARIANT_SOME
    }

    #[inline]
    pub(crate) fn clone_payload(&self) -> Result<KindedSlot, VMError> {
        clone_payload(self.storage, OPTION_PAYLOAD, "__Option")
    }
}

#[inline]
pub(crate) fn build_ok(schemas: &BuiltinSchemaIds, payload: KindedSlot) -> KindedSlot {
    build_variant_object(schemas.result as u64, RESULT_VARIANT_OK, payload)
}

#[inline]
pub(crate) fn build_err(schemas: &BuiltinSchemaIds, payload: KindedSlot) -> KindedSlot {
    build_variant_object(schemas.result as u64, RESULT_VARIANT_ERR, payload)
}

#[inline]
pub(crate) fn build_some(schemas: &BuiltinSchemaIds, payload: KindedSlot) -> KindedSlot {
    build_variant_object(schemas.option as u64, OPTION_VARIANT_SOME, payload)
}

#[inline]
pub(crate) fn build_none(schemas: &BuiltinSchemaIds) -> KindedSlot {
    build_variant_object(
        schemas.option as u64,
        OPTION_VARIANT_NONE,
        KindedSlot::none(),
    )
}

pub(crate) fn read_result<'a>(
    schemas: &BuiltinSchemaIds,
    slot: &'a KindedSlot,
) -> Result<Option<ResultCarrier<'a>>, VMError> {
    let Some(storage) = read_variant_storage(schemas.result as u64, slot, "__Result")? else {
        return Ok(None);
    };
    validate_variant_tag(
        read_variant_tag(storage, RESULT_VARIANT)?,
        RESULT_VARIANT_OK,
        RESULT_VARIANT_ERR,
        "__Result",
    )?;
    Ok(Some(ResultCarrier { storage }))
}

pub(crate) fn read_option<'a>(
    schemas: &BuiltinSchemaIds,
    slot: &'a KindedSlot,
) -> Result<Option<OptionCarrier<'a>>, VMError> {
    let Some(storage) = read_variant_storage(schemas.option as u64, slot, "__Option")? else {
        return Ok(None);
    };
    validate_variant_tag(
        read_variant_tag(storage, OPTION_VARIANT)?,
        OPTION_VARIANT_SOME,
        OPTION_VARIANT_NONE,
        "__Option",
    )?;
    Ok(Some(OptionCarrier { storage }))
}

/// Build a single-payload enum variant TypedObject (`__variant` at field 0,
/// payload at field 1). Shared by the Result/Option carriers here and by the
/// snapshot-marker construction (design §4.1.3) — any enum whose variant
/// carries at most one payload uses this exact layout.
pub(crate) fn build_variant_object(schema_id: u64, variant: i64, payload: KindedSlot) -> KindedSlot {
    let payload_slot = payload.slot();
    let payload_kind = payload.kind();
    let payload_bits = payload_slot.raw();
    #[cfg(miri)]
    let payload_provenance = payload.miri_provenance();

    let slots = vec![ValueSlot::from_int(variant), payload_slot].into_boxed_slice();
    let heap_mask = if payload_field_owns_heap_share(payload_bits, payload_kind) {
        1u64 << OPTION_PAYLOAD
    } else {
        0
    };
    let field_kinds: Arc<[NativeKind]> =
        Arc::from(vec![NativeKind::Int64, payload_kind].into_boxed_slice());

    std::mem::forget(payload);
    #[cfg(miri)]
    let ptr = TypedObjectStorage::_new_with_miri_field_provenance(
        schema_id,
        slots,
        heap_mask,
        field_kinds,
        vec![
            shape_value::heap_value::MiriSlotProvenance::None,
            payload_provenance,
        ]
        .into_boxed_slice(),
    );
    #[cfg(not(miri))]
    let ptr = TypedObjectStorage::_new(schema_id, slots, heap_mask, field_kinds);
    KindedSlot::from_typed_object_raw(ptr)
}

fn read_variant_storage<'a>(
    schema_id: u64,
    slot: &'a KindedSlot,
    name: &'static str,
) -> Result<Option<&'a TypedObjectStorage>, VMError> {
    if !matches!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject)) {
        return Ok(None);
    }

    let bits = slot.slot().raw();
    if bits == 0 {
        return Err(VMError::RuntimeError(format!(
            "{name}: null TypedObject carrier"
        )));
    }

    let storage = slot.as_typed_object_storage().ok_or_else(|| {
        VMError::RuntimeError(format!("{name}: missing TypedObject pointer provenance"))
    })?;
    if storage.schema_id != schema_id {
        return Ok(None);
    }
    validate_layout(storage, name)?;
    Ok(Some(storage))
}

fn validate_layout(storage: &TypedObjectStorage, name: &'static str) -> Result<(), VMError> {
    if storage.slots().len() != 2 || storage.field_kinds.len() != 2 {
        return Err(VMError::RuntimeError(format!(
            "{name}: corrupt storage layout (slots={}, field_kinds={})",
            storage.slots().len(),
            storage.field_kinds.len()
        )));
    }
    if storage.field_kinds[0] != NativeKind::Int64 {
        return Err(VMError::RuntimeError(format!(
            "{name}: corrupt variant kind {:?}",
            storage.field_kinds[0]
        )));
    }
    Ok(())
}

fn read_variant_tag(storage: &TypedObjectStorage, idx: usize) -> Result<i64, VMError> {
    let value = storage.slots()[idx].as_i64();
    Ok(value)
}

fn validate_variant_tag(
    tag: i64,
    first: i64,
    second: i64,
    name: &'static str,
) -> Result<(), VMError> {
    if tag == first || tag == second {
        Ok(())
    } else {
        Err(VMError::RuntimeError(format!(
            "{name}: invalid variant discriminant {tag}"
        )))
    }
}

fn clone_payload(
    storage: &TypedObjectStorage,
    idx: usize,
    name: &'static str,
) -> Result<KindedSlot, VMError> {
    if idx >= storage.slots().len() || idx >= storage.field_kinds.len() {
        return Err(VMError::RuntimeError(format!(
            "{name}: payload index {idx} outside storage layout"
        )));
    }
    storage
        .clone_field_kinded(idx)
        .ok_or_else(|| VMError::RuntimeError(format!("{name}: payload index {idx} missing")))
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

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::builtin_schemas::register_builtin_schemas;
    use shape_runtime::type_schema::registry::TypeSchemaRegistry;

    fn schemas() -> BuiltinSchemaIds {
        let mut registry = TypeSchemaRegistry::new();
        register_builtin_schemas(&mut registry)
    }

    fn storage(slot: &KindedSlot) -> &TypedObjectStorage {
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let bits = slot.slot().raw();
        assert_ne!(bits, 0);
        slot.as_typed_object_storage()
            .expect("from_typed_object_raw carrier has provenance")
    }

    #[test]
    fn result_carrier_uses_typed_object_schema_and_exact_payload_kind() {
        let schemas = schemas();
        let result = build_ok(&schemas, KindedSlot::from_int(42));

        let storage = storage(&result);
        assert_eq!(storage.schema_id, schemas.result as u64);
        assert_eq!(storage.field_kinds[RESULT_VARIANT], NativeKind::Int64);
        assert_eq!(storage.field_kinds[RESULT_PAYLOAD], NativeKind::Int64);
        assert_eq!(storage.heap_mask, 0);

        let view = read_result(&schemas, &result).unwrap().unwrap();
        assert!(view.is_ok());
        let payload = view.clone_payload().unwrap();
        assert_eq!(payload.as_i64(), Some(42));
        drop(payload);
        drop(result);
    }

    #[test]
    fn option_carrier_retains_and_releases_heap_payload_by_field_kind() {
        let schemas = schemas();
        let text = Arc::new("hello".to_string());
        let before = Arc::strong_count(&text);
        let option = build_some(&schemas, KindedSlot::from_string_arc(Arc::clone(&text)));
        assert_eq!(Arc::strong_count(&text), before + 1);

        let storage = storage(&option);
        assert_eq!(storage.schema_id, schemas.option as u64);
        assert_eq!(storage.field_kinds[OPTION_PAYLOAD], NativeKind::String);
        assert_eq!((storage.heap_mask >> OPTION_PAYLOAD) & 1, 1);

        let view = read_option(&schemas, &option).unwrap().unwrap();
        assert!(view.is_some());
        let payload = view.clone_payload().unwrap();
        assert_eq!(Arc::strong_count(&text), before + 2);
        assert_eq!(payload.as_str(), Some("hello"));

        drop(payload);
        assert_eq!(Arc::strong_count(&text), before + 1);
        drop(option);
        assert_eq!(Arc::strong_count(&text), before);
    }

    #[cfg(miri)]
    #[test]
    fn miri_result_option_typed_object_payload_clone_and_drop() {
        use shape_value::v2::heap_element::HeapElement;
        use shape_value::v2::refcount::{v2_get_refcount, v2_retain};

        let schemas = schemas();

        unsafe {
            let payload_kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
            let payload_ptr = TypedObjectStorage::_new(
                704,
                vec![ValueSlot::from_int(99)].into_boxed_slice(),
                0,
                payload_kinds,
            );
            v2_retain(&(*payload_ptr).header);
            assert_eq!(
                v2_get_refcount(&(*payload_ptr).header),
                2,
                "payload has one carrier-owned share plus one witness share"
            );

            let option = build_some(&schemas, KindedSlot::from_typed_object_raw(payload_ptr));
            {
                let option_storage = storage(&option);
                assert_eq!(option_storage.schema_id, schemas.option as u64);
                assert_eq!(
                    option_storage.field_kinds[OPTION_PAYLOAD],
                    NativeKind::Ptr(HeapKind::TypedObject)
                );
                assert_eq!((option_storage.heap_mask >> OPTION_PAYLOAD) & 1, 1);
            }

            let payload = {
                let option_view = read_option(&schemas, &option).unwrap().unwrap();
                assert!(option_view.is_some());
                option_view.clone_payload().unwrap()
            };
            assert_eq!(payload.kind(), NativeKind::Ptr(HeapKind::TypedObject));
            assert_eq!(
                payload
                    .as_typed_object_storage()
                    .expect("payload clone keeps typed-object provenance")
                    .schema_id,
                704
            );
            assert_eq!(
                v2_get_refcount(&(*payload_ptr).header),
                3,
                "clone_payload retained the typed-object payload share"
            );

            drop(payload);
            assert_eq!(
                v2_get_refcount(&(*payload_ptr).header),
                2,
                "dropping the cloned payload releases its retained share"
            );

            drop(option);
            assert_eq!(
                v2_get_refcount(&(*payload_ptr).header),
                1,
                "dropping the Option carrier releases the original payload share"
            );

            TypedObjectStorage::release_elem(payload_ptr);
        }
    }

    #[test]
    fn none_carrier_has_no_payload_heap_mask() {
        let schemas = schemas();
        let option = build_none(&schemas);
        let storage = storage(&option);

        assert_eq!(storage.schema_id, schemas.option as u64);
        assert_eq!(
            storage.slots()[OPTION_VARIANT].as_i64(),
            OPTION_VARIANT_NONE
        );
        assert_eq!(storage.field_kinds[OPTION_PAYLOAD], NativeKind::Null);
        assert_eq!((storage.heap_mask >> OPTION_PAYLOAD) & 1, 0);

        let view = read_option(&schemas, &option).unwrap().unwrap();
        assert!(!view.is_some());
        let payload = view.clone_payload().unwrap();
        assert_eq!(payload.kind(), NativeKind::Null);
        drop(payload);
        drop(option);
    }

    #[test]
    fn result_reader_rejects_foreign_typed_object_schema() {
        let schemas = schemas();
        let option = build_some(&schemas, KindedSlot::from_int(7));
        assert!(read_result(&schemas, &option).unwrap().is_none());
        drop(option);
    }
}
