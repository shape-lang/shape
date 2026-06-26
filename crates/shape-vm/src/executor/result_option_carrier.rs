//! Canonical Result/Option runtime carriers.
//!
//! L5 owns the runtime carrier shape: `Result<T, E>` and `Option<T>` are
//! fixed-layout TypedObjects, not separate `HeapKind` variants. This module is
//! the single VM helper for constructing and reading those objects.

use crate::executor::vm_impl::stack::clone_with_kind;
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
    build_variant_object(schemas.option as u64, OPTION_VARIANT_NONE, KindedSlot::none())
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

fn build_variant_object(schema_id: u64, variant: i64, payload: KindedSlot) -> KindedSlot {
    let payload_slot = payload.slot();
    let payload_kind = payload.kind();
    let payload_bits = payload_slot.raw();

    let slots = vec![ValueSlot::from_int(variant), payload_slot].into_boxed_slice();
    let heap_mask = if payload_field_owns_heap_share(payload_bits, payload_kind) {
        1u64 << OPTION_PAYLOAD
    } else {
        0
    };
    let field_kinds: Arc<[NativeKind]> =
        Arc::from(vec![NativeKind::Int64, payload_kind].into_boxed_slice());

    std::mem::forget(payload);
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

    // SAFETY: kind == Ptr(TypedObject); the slot owns a live `_new`
    // TypedObjectStorage share. This is a read-only borrow bounded by `slot`.
    let storage: &TypedObjectStorage = unsafe { &*(bits as *const TypedObjectStorage) };
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
    let slot = storage.slots()[idx];
    let kind = storage.field_kinds[idx];
    clone_with_kind(slot.raw(), kind);
    Ok(KindedSlot::new(slot, kind))
}

fn payload_field_owns_heap_share(bits: u64, kind: NativeKind) -> bool {
    if bits == 0 {
        return false;
    }
    match kind {
        NativeKind::String | NativeKind::StringV2 | NativeKind::DecimalV2 => true,
        NativeKind::Ptr(HeapKind::Future | HeapKind::ModuleFn | HeapKind::Char) => false,
        NativeKind::Ptr(HeapKind::NativeScalar) => false,
        NativeKind::Ptr(_) => true,
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
        // SAFETY: test helper only receives `from_typed_object_raw` carriers
        // built by this module and borrows while `slot` owns the share.
        unsafe { &*(bits as *const TypedObjectStorage) }
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

    #[test]
    fn none_carrier_has_no_payload_heap_mask() {
        let schemas = schemas();
        let option = build_none(&schemas);
        let storage = storage(&option);

        assert_eq!(storage.schema_id, schemas.option as u64);
        assert_eq!(storage.slots()[OPTION_VARIANT].as_i64(), OPTION_VARIANT_NONE);
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
