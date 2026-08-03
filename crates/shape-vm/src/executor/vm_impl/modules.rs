use super::super::*;
use crate::executor::result_option_carrier;
// `VMError` is intentionally left out of the executor/mod.rs star-import
// (see executor/mod.rs:126 comment); name it locally for the
// `invoke_module_fn_id_stub` surface.
use shape_value::VMError;

const JSON_VARIANT_NULL: i64 = 0;
const JSON_VARIANT_BOOL: i64 = 1;
const JSON_VARIANT_INT: i64 = 2;
const JSON_VARIANT_NUMBER: i64 = 3;
const JSON_VARIANT_STR: i64 = 4;
const JSON_VARIANT_ARRAY: i64 = 5;
const JSON_VARIANT_OBJECT: i64 = 6;

// JsonValue return projection builds the runtime `Json` enum layout from the
// registered std::core::json_value schema: __variant plus __payload_0.
fn validate_json_schema(schema: &shape_runtime::type_schema::TypeSchema) -> Result<(), VMError> {
    if schema.field_count() != 2
        || schema.field_index("__variant") != Some(0)
        || schema.field_index("__payload_0") != Some(1)
    {
        return Err(VMError::RuntimeError(format!(
            "project_concrete_return: Json schema must be the std::core::json_value \
             enum layout (__variant, __payload_0), got fields {:?}",
            schema.field_names().collect::<Vec<_>>()
        )));
    }

    let enum_info = schema.get_enum_info().ok_or_else(|| {
        VMError::RuntimeError(
            "project_concrete_return: registered `Json` schema is not an enum".to_string(),
        )
    })?;
    for (name, id, payload_fields) in [
        ("Null", JSON_VARIANT_NULL as u16, 0),
        ("Bool", JSON_VARIANT_BOOL as u16, 1),
        ("Int", JSON_VARIANT_INT as u16, 1),
        ("Number", JSON_VARIANT_NUMBER as u16, 1),
        ("Str", JSON_VARIANT_STR as u16, 1),
        ("Array", JSON_VARIANT_ARRAY as u16, 1),
        ("Object", JSON_VARIANT_OBJECT as u16, 1),
    ] {
        let Some(variant) = enum_info.variant_by_name(name) else {
            return Err(VMError::RuntimeError(format!(
                "project_concrete_return: Json schema missing variant `{name}`"
            )));
        };
        if variant.id != id || variant.payload_fields != payload_fields {
            return Err(VMError::RuntimeError(format!(
                "project_concrete_return: Json::{name} schema mismatch \
                 (id={}, payload_fields={}), expected (id={id}, payload_fields={payload_fields})",
                variant.id, variant.payload_fields
            )));
        }
    }

    Ok(())
}

fn project_json_value_return(
    schemas: &shape_runtime::type_schema::TypeSchemaRegistry,
    value: shape_runtime::json_value::JsonValue,
) -> Result<shape_value::KindedSlot, VMError> {
    let json_schema = schemas.get("Json").ok_or_else(|| {
        VMError::RuntimeError(
            "project_concrete_return: ConcreteReturn::JsonValue requires the \
             registered `Json` enum schema from std::core::json_value"
                .to_string(),
        )
    })?;
    validate_json_schema(json_schema)?;
    Ok(project_json_value_to_slot(value, json_schema.id as u64))
}

fn json_payload_field_owns_heap_share(bits: u64, kind: shape_value::NativeKind) -> bool {
    use shape_value::{HeapKind, NativeKind};

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
            | HeapKind::ForeignRef
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

fn build_json_enum_slot(
    json_schema_id: u64,
    variant_id: i64,
    payload: shape_value::KindedSlot,
) -> shape_value::KindedSlot {
    use shape_value::heap_value::TypedObjectStorage;
    use shape_value::{KindedSlot, NativeKind, ValueSlot};
    use std::sync::Arc;

    let payload_slot = payload.slot();
    let payload_kind = payload.kind();
    let payload_bits = payload_slot.raw();
    #[cfg(miri)]
    let payload_provenance = payload.miri_provenance();

    let slots = vec![ValueSlot::from_int(variant_id), payload_slot].into_boxed_slice();
    let heap_mask = if json_payload_field_owns_heap_share(payload_bits, payload_kind) {
        1u64 << 1
    } else {
        0
    };
    let field_kinds: Arc<[NativeKind]> =
        Arc::from(vec![NativeKind::Int64, payload_kind].into_boxed_slice());

    std::mem::forget(payload);
    #[cfg(miri)]
    let ptr = TypedObjectStorage::_new_with_miri_field_provenance(
        json_schema_id,
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
    let ptr = TypedObjectStorage::_new(json_schema_id, slots, heap_mask, field_kinds);
    KindedSlot::from_typed_object_raw(ptr)
}

fn take_typed_object_slot(
    slot: shape_value::KindedSlot,
) -> *const shape_value::heap_value::TypedObjectStorage {
    use shape_value::{HeapKind, NativeKind};

    debug_assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
    #[cfg(miri)]
    let ptr = match slot.miri_provenance() {
        shape_value::heap_value::MiriSlotProvenance::TypedObject(ptr) if !ptr.is_null() => ptr,
        other => panic!(
            "take_typed_object_slot: TypedObject slot missing Miri provenance: {:?}",
            other
        ),
    };
    #[cfg(not(miri))]
    let ptr = slot.raw() as *const shape_value::heap_value::TypedObjectStorage;
    std::mem::forget(slot);
    ptr
}

fn project_json_value_to_slot(
    value: shape_runtime::json_value::JsonValue,
    json_schema_id: u64,
) -> shape_value::KindedSlot {
    use shape_runtime::json_value::JsonValue;
    use shape_value::heap_value::{
        HashMapData, HashMapKindedRef, TypedObjectPtr, TypedObjectStorage,
    };
    use shape_value::v2::typed_array::{ELEM_TYPE_TYPED_OBJECT, TypedArray, stamp_elem_type};
    use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
    use std::sync::Arc;

    if let JsonValue::Bytes(bytes) = value {
        // The user-facing Json enum has no Bytes variant. Preserve the JSON
        // wire convention used by json_value_to_serde_json: bytes are a JSON
        // array of integer byte values.
        let elems = bytes
            .into_iter()
            .map(|b| JsonValue::Int(b as i64))
            .collect();
        return project_json_value_to_slot(JsonValue::Array(elems), json_schema_id);
    }

    match value {
        JsonValue::Null => {
            build_json_enum_slot(json_schema_id, JSON_VARIANT_NULL, KindedSlot::none())
        }
        JsonValue::Bool(b) => {
            build_json_enum_slot(json_schema_id, JSON_VARIANT_BOOL, KindedSlot::from_bool(b))
        }
        JsonValue::Int(i) => {
            build_json_enum_slot(json_schema_id, JSON_VARIANT_INT, KindedSlot::from_int(i))
        }
        JsonValue::Number(n) => build_json_enum_slot(
            json_schema_id,
            JSON_VARIANT_NUMBER,
            KindedSlot::from_number(n),
        ),
        JsonValue::String(s) => build_json_enum_slot(
            json_schema_id,
            JSON_VARIANT_STR,
            KindedSlot::from_string_arc(Arc::new(s)),
        ),
        JsonValue::Bytes(_) => unreachable!("handled before payload construction"),
        JsonValue::Array(values) => {
            let mut element_ptrs: Vec<*const TypedObjectStorage> = Vec::with_capacity(values.len());
            for value in values {
                let slot = project_json_value_to_slot(value, json_schema_id);
                // Transfer the slot's typed-object share into the array
                // element. The stamped typed-object array releases each
                // element with TypedObjectStorage::release_elem.
                let ptr = take_typed_object_slot(slot);
                element_ptrs.push(ptr);
            }
            let arr = TypedArray::<*const TypedObjectStorage>::from_slice(&element_ptrs);
            unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT) };
            build_json_enum_slot(
                json_schema_id,
                JSON_VARIANT_ARRAY,
                KindedSlot::new(
                    ValueSlot::from_raw(arr as usize as u64),
                    NativeKind::Ptr(HeapKind::TypedArray),
                ),
            )
        }
        JsonValue::Object(pairs) => {
            let mut data: HashMapData<TypedObjectPtr> = HashMapData::new();
            for (key, value) in pairs {
                let slot = project_json_value_to_slot(value, json_schema_id);
                // Transfer the slot share into the HashMap value wrapper;
                // HashMapData owns and later drops the TypedObjectPtr.
                let ptr = take_typed_object_slot(slot);
                unsafe {
                    data.insert(key.as_str(), TypedObjectPtr::new(ptr));
                }
            }
            let kref = HashMapKindedRef::TypedObject(Arc::new(data));
            build_json_enum_slot(
                json_schema_id,
                JSON_VARIANT_OBJECT,
                KindedSlot::from_hashmap(Arc::new(kref)),
            )
        }
    }
}

/// Project a single `ConcreteReturn` leaf into a `KindedSlot`.
///
/// **STAGE K1 (2026-06-02).** The leaf projector shared by both
/// `project_typed_return`'s `Concrete` arm and the wrapper arms
/// (`Ok`/`Err`/`Some`) — those build their payload through this function
/// and then wrap the resulting `KindedSlot` in canonical `__Result` /
/// `__Option` TypedObjects.
/// Every arm builds the typed-Arc carrier directly per ADR-005 §1 /
/// ADR-006 §2.7 — no `Box<HeapValue>` wrapping, no value synthesis, no
/// `ValueWord`. Arms that genuinely cannot project typed-Arc-direct at
/// this boundary (`HashMapStringHeapValue` needs the K3 `HashMapData`
/// kinded-value-track amendment) surface clean per §2.7.4.
fn project_concrete_return(
    schemas: &shape_runtime::type_schema::TypeSchemaRegistry,
    c: shape_runtime::typed_module_exports::ConcreteReturn,
) -> Result<shape_value::KindedSlot, VMError> {
    use shape_runtime::typed_module_exports::ConcreteReturn;
    use shape_value::heap_value::{HashMapData, HashMapKindedRef};
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::{
        ELEM_TYPE_F64, ELEM_TYPE_I64, ELEM_TYPE_STRING, TypedArray, stamp_elem_type,
    };
    use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
    use std::sync::Arc;
    match c {
        ConcreteReturn::I64(i) => Ok(KindedSlot::new(
            ValueSlot::from_raw(i as u64),
            NativeKind::Int64,
        )),
        ConcreteReturn::F64(f) => Ok(KindedSlot::new(
            ValueSlot::from_raw(f.to_bits()),
            NativeKind::Float64,
        )),
        ConcreteReturn::Bool(b) => Ok(KindedSlot::new(
            ValueSlot::from_raw(if b { 1 } else { 0 }),
            NativeKind::Bool,
        )),
        ConcreteReturn::Unit => Ok(KindedSlot::new(ValueSlot::from_raw(0), NativeKind::Bool)),
        ConcreteReturn::String(s) => Ok(KindedSlot::from_string_arc(Arc::new(s))),
        // `Instant` rides `Arc<std::time::Instant>` directly (no wrapper
        // struct) per ADR-006 §2.3. `from_instant` moves one
        // `Arc::into_raw` share into the slot with kind
        // `Ptr(HeapKind::Instant)`; the matching Clone/Drop arms in
        // `stack.rs` / `kinded_slot.rs` retire it. Fixes the
        // `Discriminant(5)` K3 SURFACE that `time::now()` /
        // `time::stopwatch()` previously hit — there was no explicit Instant
        // arm, so it fell through to the polymorphic-HashMap K3 error. This
        // is a specific typed-Arc leaf, NOT the polymorphic-value case.
        ConcreteReturn::Instant(inst) => Ok(KindedSlot::from_instant(Arc::new(inst))),
        ConcreteReturn::OpaqueTypedObject(hv) => {
            // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): hv is
            // `Arc<HeapValue::TypedObject(TypedObjectPtr)>`. Clone the
            // wrapper (bumps v2-raw refcount); into_raw moves the share
            // to the slot via `from_typed_object_raw`.
            match &*hv {
                shape_value::heap_value::HeapValue::TypedObject(s) => {
                    Ok(KindedSlot::from_typed_object_raw(s.clone().into_raw()))
                }
                other => Err(VMError::RuntimeError(format!(
                    "project_concrete_return: OpaqueTypedObject expected \
                     HeapValue::TypedObject payload, got {:?}",
                    other.kind()
                ))),
            }
        }
        // R8 W6 G.1 W17-marshal-return-arms close (2026-05-24): explicit
        // arms for ConcreteReturn::IoHandle (disc 16) +
        // ConcreteReturn::DataTable (disc 15). Mirrors the 14 existing
        // typed-Arc constructor precedents in `kinded_slot.rs` per
        // ADR-005 §1 single-discriminator + ADR-006 §2.7.6 / Q8
        // bounded carrier-API.
        ConcreteReturn::IoHandle(h) => Ok(KindedSlot::from_io_handle(h)),
        ConcreteReturn::DataTable(d) => Ok(KindedSlot::from_data_table(d)),

        // ── STAGE K1 (2026-06-02): typed-array leaves ──────────────────
        //
        // `Array<int>` / `Array<number>` carriers are the monomorphic
        // flat-struct `*mut TypedArray<T>` per `docs/runtime-v2-spec.md`;
        // slot bits are the raw pointer, kind `Ptr(HeapKind::TypedArray)`,
        // and the element-type discriminant is stamped at HeapHeader
        // offset 7 so the release path (`release_v2_typed_array`) picks
        // the matching `drop_array::<T>` monomorphization. `Bytes`
        // surfaces to user code as `Array<int>`, so each byte widens to
        // i64 into a `TypedArray<i64>` (ELEM_TYPE_I64). Empty arrays still
        // allocate a stamped zero-length carrier (the round-trip reader
        // maps a null pointer → empty; an owned non-null carrier is the
        // canonical shape for a value pushed onto the stack).
        ConcreteReturn::ArrayI64(v) => {
            let arr = TypedArray::<i64>::from_slice(&v);
            unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        ConcreteReturn::ArrayF64(v) => {
            let arr = TypedArray::<f64>::from_slice(&v);
            unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_F64) };
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        ConcreteReturn::Bytes(bytes) => {
            let widened: Vec<i64> = bytes.iter().map(|&b| b as i64).collect();
            let arr = TypedArray::<i64>::from_slice(&widened);
            unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        ConcreteReturn::ArrayString(v) => {
            // Each element is a fresh `StringObj` (refcount = 1); the
            // array owns each allocation outright. Mirror of the marshal
            // `ToSlot<Vec<Arc<String>>>` producer.
            let arr = TypedArray::<*const StringObj>::with_capacity(v.len() as u32);
            unsafe {
                stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
                for s in &v {
                    let p = StringObj::new(s.as_str()) as *const StringObj;
                    TypedArray::<*const StringObj>::push(arr, p);
                }
            }
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        ConcreteReturn::ArrayStringRows(rows) => {
            // Nested string rows use the marshal-layer v2 nested typed-array
            // producer: outer TypedArray<*const TypedArrayElem>, inner
            // TypedArray<*const StringObj>. This is not an ArrayHeapValue
            // projection; the elements are slot-level typed arrays.
            use shape_runtime::marshal::ToSlot;
            let bits = rows.to_slot();
            Ok(KindedSlot::new(
                ValueSlot::from_raw(bits),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
        ConcreteReturn::ArrayHeapValue(elems) => {
            // STAGE K2 dispatcher: the per-element-T marshal producer
            // (`ToSlot<Vec<Arc<HeapValue>>>` in shape-runtime/marshal.rs)
            // inspects the first element's HeapValue variant, allocates
            // the matching `TypedArray<T>`, stamps the element
            // discriminant, and takes its own share per element. The
            // returned bits are the raw `*mut TypedArray<T>` carrier; the
            // kind is `Ptr(HeapKind::TypedArray)` (the K2 producer's
            // `NATIVE_KIND`). Empty Vec → null carrier (bits = 0).
            use shape_runtime::marshal::ToSlot;
            let bits = elems.to_slot();
            Ok(KindedSlot::new(
                ValueSlot::from_raw(bits),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }

        // ── STAGE K1 (2026-06-02): string→string HashMap leaf ──────────
        //
        // `HashMap<string, string>` builds a `HashMapData<*const
        // StringObj>` (string-value monomorphization), wrapped in the
        // per-V carrier enum `HashMapKindedRef::String`. Each value is a
        // fresh `StringObj` (refcount = 1) whose owned share transfers
        // into the map via `HashMapData::insert`; keys are allocated as
        // fresh `StringObj`s inside `insert`. The `from_hashmap`
        // constructor moves one outer `Arc<HashMapKindedRef>` share into
        // the slot with kind `Ptr(HeapKind::HashMap)`. No heap-value
        // track is needed — the value monomorphization is string, not
        // the polymorphic K3 `Arc<HeapValue>` shape.
        ConcreteReturn::HashMapStringString(pairs) => {
            let mut data: HashMapData<*const StringObj> = HashMapData::new();
            for (k, v) in &pairs {
                let value_ptr = StringObj::new(v.as_str()) as *const StringObj;
                // SAFETY: `value_ptr` is a freshly-allocated StringObj
                // (refcount = 1); `insert` takes ownership of that single
                // share per the HashMapData::insert contract.
                unsafe {
                    data.insert(k.as_str(), value_ptr);
                }
            }
            let kref = Arc::new(HashMapKindedRef::String(Arc::new(data)));
            Ok(KindedSlot::from_hashmap(kref))
        }
        ConcreteReturn::JsonValue(value) => project_json_value_return(schemas, value),

        // ── Genuinely-cannot-go-typed-Arc-direct: SURFACE, do not shim ──
        //
        // `HashMapStringHeapValue` is K3 territory: the polymorphic-value
        // HashMap needs the ADR-006 `HashMapData` kinded-value-track
        // amendment (a parallel `Vec<NativeKind>` over the values) before
        // it can carry `Arc<HeapValue>` payloads without a Bool-default
        // kind. Surface clean rather than shim — a fabricated carrier here
        // is exactly the deleted-pattern class CLAUDE.md §Forbidden refuses.
        other => Err(VMError::NotImplemented(format!(
            "project_concrete_return: ConcreteReturn::{:?} has no \
             typed-Arc-direct projection at the module-return boundary. \
             HashMapStringHeapValue is K3 (pending the ADR-006 HashMapData \
             kinded-value-track amendment). SURFACED per ADR-006 §2.7.4 — \
             no shim. ",
            std::mem::discriminant(&other)
        ))),
    }
}

/// Type alias for a boxed async module-function body future
/// (`Send + 'static` — see `marshal.rs::VariadicTypedAsyncBody`).
type ModuleAsyncFuture = std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<shape_runtime::typed_module_exports::TypedReturn, String>,
            > + Send,
    >,
>;

/// Project a `TypedReturn` value into a `KindedSlot` ready for stack
/// placement.
///
/// **STAGE K1 (2026-06-02).** Lands the container / wrapper arms over the
/// scalar/leaf base established by the W17-snapshot-roundtrip work
/// (Phase 2d Wave 2.6, 2026-05-11). Wrapper arms build their inner
/// payload through [`project_concrete_return`] and wrap the resulting
/// `KindedSlot` in canonical `__Result` / `__Option` TypedObjects;
/// typed-object arms build a `TypedObjectStorage` via
/// [`shape_runtime::type_schema::typed_object_from_pairs`]. Each arm picks
/// its `NativeKind` from the discriminator without value synthesis per
/// ADR-006 §2.7.4. `ArrayObjectPairs` (array of typed objects) builds the
/// same v2-raw `TypedArray<*const TypedObjectStorage>` carrier used by
/// Json arrays and K2 typed-object arrays.
fn project_typed_return(
    schemas: &shape_runtime::type_schema::BuiltinSchemaIds,
    registry: &shape_runtime::type_schema::TypeSchemaRegistry,
    tr: shape_runtime::typed_module_exports::TypedReturn,
) -> Result<shape_value::KindedSlot, VMError> {
    use shape_runtime::type_schema::typed_object_from_pairs;
    use shape_runtime::typed_module_exports::{ConcreteReturn, TypedReturn};
    use shape_value::v2::typed_array::{ELEM_TYPE_TYPED_OBJECT, TypedArray, stamp_elem_type};
    use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};

    // Build a `TypedObjectStorage`-backed KindedSlot from string→leaf
    // pairs. Each leaf projects through `project_concrete_return`; the
    // share is moved into `typed_object_from_pairs`' slot list (which
    // clones-then-forgets per its construction contract).
    fn typed_object_from_concrete_pairs(
        registry: &shape_runtime::type_schema::TypeSchemaRegistry,
        pairs: Vec<(String, ConcreteReturn)>,
    ) -> Result<KindedSlot, VMError> {
        // Project each leaf, holding the owned (name, KindedSlot) pairs so
        // the `&str` borrows the builder needs live across the call.
        let mut owned: Vec<(String, KindedSlot)> = Vec::with_capacity(pairs.len());
        for (name, c) in pairs {
            owned.push((name, project_concrete_return(registry, c)?));
        }
        // `typed_object_from_pairs` borrows each `KindedSlot`, clones it
        // (one refcount bump moved into the slot list), then forgets the
        // clone. Our `owned` originals Drop normally at scope exit, each
        // releasing its single share — net one share owned by the new
        // typed object's slot list.
        let borrowed: Vec<(&str, KindedSlot)> = owned
            .iter()
            .map(|(name, slot)| (name.as_str(), slot.clone()))
            .collect();
        let out = typed_object_from_pairs(&borrowed);
        // `borrowed`'s clones Drop here (−1 each); `owned`'s originals Drop
        // at function return (−1 each). The builder's internal forget keeps
        // exactly one share per field inside the typed object.
        Ok(out)
    }

    match tr {
        TypedReturn::Concrete(c) => project_concrete_return(registry, c),
        TypedReturn::Future(id) => Ok(KindedSlot::new(
            ValueSlot::from_raw(id),
            NativeKind::Ptr(HeapKind::Future),
        )),

        // ── Result / Option wrappers (ADR-006 §2.7.17 / Q18) ───────────
        TypedReturn::Ok(c) => {
            let payload = project_concrete_return(registry, c)?;
            Ok(result_option_carrier::build_ok(schemas, payload))
        }
        TypedReturn::Err(c) => {
            let payload = project_concrete_return(registry, c)?;
            Ok(result_option_carrier::build_err(schemas, payload))
        }
        TypedReturn::Some(c) => {
            let payload = project_concrete_return(registry, c)?;
            Ok(result_option_carrier::build_some(schemas, payload))
        }
        // Canonical Option representation (ffi-rebuild §4.4 stage 3, see
        // `control_flow/foreign_marshal.rs::msgpack_to_kinded_slot` and
        // `patterns/checking.rs`): `None` IS the null sentinel
        // (`KindedSlot::none()`), `Some(v)` is the non-null `build_some`
        // carrier. The pattern-matcher's `None` discriminator emits `IsNull`
        // (null-coding), so a non-null `build_none` carrier here read as
        // `Some` under `IsNull` and then faulted in `UnwrapOption` — user
        // bytecode's `None` (`MirConstant::None`) already null-codes, this
        // arm was the single divergent producer (ADR-009 B2 slice S4, found
        // by the `find_impl` None-arm proof).
        TypedReturn::None => Ok(shape_value::KindedSlot::none()),

        // ── Typed-object wrappers (TypedObjectStorage builder) ─────────
        TypedReturn::ObjectPairs(pairs) | TypedReturn::TypedObject(pairs) => {
            typed_object_from_concrete_pairs(registry, pairs)
        }
        TypedReturn::SomeObjectPairs(pairs) => {
            let payload = typed_object_from_concrete_pairs(registry, pairs)?;
            Ok(result_option_carrier::build_some(schemas, payload))
        }
        TypedReturn::OkObjectPairs(pairs) => {
            let payload = typed_object_from_concrete_pairs(registry, pairs)?;
            Ok(result_option_carrier::build_ok(schemas, payload))
        }
        TypedReturn::ErrObjectPairs(pairs) => {
            let payload = typed_object_from_concrete_pairs(registry, pairs)?;
            Ok(result_option_carrier::build_err(schemas, payload))
        }

        TypedReturn::ArrayObjectPairs(rows) => {
            let mut row_slots: Vec<KindedSlot> = Vec::with_capacity(rows.len());
            for pairs in rows {
                row_slots.push(typed_object_from_concrete_pairs(registry, pairs)?);
            }

            let mut element_ptrs: Vec<*const shape_value::heap_value::TypedObjectStorage> =
                Vec::with_capacity(row_slots.len());
            for slot in row_slots {
                // Transfer the row object's owned v2-raw share into the
                // typed-object array. The stamped array releases each element
                // with `TypedObjectStorage::release_elem`.
                let ptr = take_typed_object_slot(slot);
                element_ptrs.push(ptr);
            }
            let arr = TypedArray::<*const shape_value::heap_value::TypedObjectStorage>::from_slice(
                &element_ptrs,
            );
            unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT) };
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        }
    }
}

impl VirtualMachine {
    /// Spawn an async module-function future onto the shared background
    /// runtime and return a `Future(id)` handle (WF-2D real concurrency).
    ///
    /// The future (`time::sleep`-style, `Send + 'static`, borrows nothing
    /// from the VM) begins running on a worker thread immediately. The
    /// interpreter thread does NOT block here — it receives a `Future(id)`
    /// value and continues, so subsequent async module calls launch
    /// concurrently. `op_await` / `op_join_await` later collect the result
    /// through [`resolve_pending_async_task`] / [`try_resolve_pending_async_task`].
    ///
    /// Completion crosses back over a plain `std::sync::mpsc` channel (not a
    /// tokio primitive) so the interpreter thread's blocking `recv` never
    /// nests inside the ambient current-thread runtime.
    pub(crate) fn spawn_async_module_future(
        &mut self,
        fut: ModuleAsyncFuture,
    ) -> Result<shape_value::KindedSlot, VMError> {
        let task_id = self.next_future_id();
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = crate::executor::async_runtime::shared_runtime().spawn(async move {
            let result = fut.await;
            // Receiver may already be gone (task aborted / VM torn down);
            // ignore the send error.
            let _ = tx.send(result);
        });
        self.task_scheduler.store_pending_async(
            task_id,
            crate::executor::task_scheduler::PendingAsyncTask {
                completion: crate::executor::task_scheduler::AsyncCompletion::Typed(rx),
                abort: Some(handle.abort_handle()),
            },
        );
        Ok(shape_value::KindedSlot::new(
            shape_value::ValueSlot::from_raw(task_id),
            shape_value::NativeKind::Ptr(shape_value::heap_value::HeapKind::Future),
        ))
    }

    /// Block until an in-flight async module task completes, then project its
    /// result and cache it (WF-2D). Used by `op_await` on a `Future(id)` that
    /// names a `pending_async` task.
    ///
    /// The blocking `recv` waits on the worker thread that owns the future —
    /// because the future was spawned when the async call was *evaluated*
    /// (not here), sibling async tasks launched earlier are already making
    /// progress, so a second `await` on an already-elapsed sibling returns
    /// promptly. That is the source of the overlap.
    pub(crate) fn resolve_pending_async_task(
        &mut self,
        task_id: u64,
    ) -> Result<shape_value::KindedSlot, VMError> {
        let task = self
            .task_scheduler
            .take_pending_async(task_id)
            .ok_or_else(|| {
                VMError::RuntimeError(format!("No pending async task registered for {}", task_id))
            })?;
        let body_result = task.completion.recv().map_err(|_| {
            VMError::RuntimeError(format!(
                "async task {} channel closed before completion",
                task_id
            ))
        })?;
        self.project_and_cache_pending_async(task_id, body_result)
    }

    /// Non-blocking poll of an in-flight async module task (WF-2D). Used by
    /// the `race` / `any` first-completion loop in `op_join_await`.
    ///
    /// - `Ok(Some(slot))` — the task completed; result projected + cached.
    /// - `Ok(None)` — still running (nothing to collect yet).
    /// - `Err(_)` — the task's body returned an error (propagated so `race`
    ///   surfaces it and `any` skips to the next branch).
    pub(crate) fn try_resolve_pending_async_task(
        &mut self,
        task_id: u64,
    ) -> Result<Option<shape_value::KindedSlot>, VMError> {
        use std::sync::mpsc::TryRecvError;
        // Peek without removing so a not-ready task stays registered.
        let poll = match self.task_scheduler.peek_pending_async_try_recv(task_id) {
            Some(p) => p,
            None => {
                return Err(VMError::RuntimeError(format!(
                    "No pending async task registered for {}",
                    task_id
                )));
            }
        };
        match poll {
            Ok(body_result) => {
                // Ready — remove the entry and project.
                let _ = self.task_scheduler.take_pending_async(task_id);
                Ok(Some(
                    self.project_and_cache_pending_async(task_id, body_result)?,
                ))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => {
                let _ = self.task_scheduler.take_pending_async(task_id);
                Err(VMError::RuntimeError(format!(
                    "async task {} channel closed before completion",
                    task_id
                )))
            }
        }
    }

    /// Project a completed async body result into a `KindedSlot` and cache it
    /// on the scheduler (so a second `await` / `join` of the same id returns
    /// the cached value). Shared by the blocking and non-blocking resolvers.
    fn project_and_cache_pending_async(
        &mut self,
        task_id: u64,
        outcome: crate::executor::task_scheduler::AsyncTaskOutcome,
    ) -> Result<shape_value::KindedSlot, VMError> {
        use crate::executor::task_scheduler::AsyncTaskOutcome;
        let slot = match outcome {
            AsyncTaskOutcome::Typed(body_result) => {
                let typed_return = body_result.map_err(VMError::RuntimeError)?;
                project_typed_return(
                    &self.builtin_schemas,
                    &self.program.type_schema_registry,
                    typed_return,
                )?
            }
            // ADR-019 §5 / #202. The offloaded invoke returned raw msgpack; the
            // wrap into the user's `Result<T, string>` happens HERE, on the
            // interpreter thread, through the same `wrap_dynamic_result` the
            // synchronous foreign path uses. One unmarshal implementation means
            // a sync and an async foreign call cannot disagree about what the
            // extension returned.
            AsyncTaskOutcome::Foreign {
                foreign_idx,
                result,
            } => self.unmarshal_offloaded_foreign_result(foreign_idx, result)?,
        };
        // Cache a clone so the scheduler entry and the returned slot each own
        // an independent share (same discipline as `resolve_spawned_task`).
        crate::executor::vm_impl::stack::clone_with_kind(slot.raw(), slot.kind());
        self.task_scheduler
            .complete(task_id, slot.raw(), slot.kind());
        Ok(slot)
    }

    /// Register a built-in stdlib module into the VM's module registry.
    /// Delegates to `register_extension` — this is a semantic alias to
    /// distinguish VM-native stdlib modules from user-installed extension plugins.
    pub fn register_stdlib_module(&mut self, module: shape_runtime::module_exports::ModuleExports) {
        self.register_extension(module);
    }

    /// Register an external/user extension module (e.g. loaded from a .so plugin)
    /// into the VM's module registry.
    /// Also merges any method intrinsics for fast Object dispatch.
    ///
    /// Phase-2c surface (ADR-006 §2.7.4 / §2.7.5): the body wraps each
    /// `TypedModuleFunction` into a `ModuleFn` whose signature is
    /// `Fn(&[ValueWord], &ModuleContext) -> Result<ValueWord, String>`.
    /// `ValueWord` was deleted by the strict-typing bulldozer (no type to
    /// import); the kinded rebuild per §2.7.5 makes `ModuleFn`'s argument
    /// slice `&[KindedSlot]` and its return `Result<KindedSlot, String>`.
    /// Extensions stay on the stable raw-bits ABI and convert at the
    /// `RawCallableInvoker` boundary inside shape-runtime.
    ///
    /// The cross-crate `ModuleFn` signature change is shape-runtime
    /// territory (R-shape-runtime sub-cluster) and the corresponding
    /// `TypedReturn::into_value_word()` helper is also deleted; this
    /// caller hand-off lands in the Phase-2c rebuild session.
    pub fn register_extension(&mut self, module: shape_runtime::module_exports::ModuleExports) {
        // Merge method intrinsics — these don't carry ValueWord shapes
        // through the registration path and are safe to keep here.
        for (type_name, methods) in &module.method_intrinsics {
            let entry = self.extension_methods.entry(type_name.clone()).or_default();
            for (method_name, func) in methods {
                entry.insert(method_name.clone(), func.clone());
            }
        }
        // The `module.typed_exports()` rewrap into `ModuleFn` (which
        // marshals `TypedReturn -> ValueWord` at the boundary) is the
        // Phase-2c host-API rebuild (ADR-006 §2.7.4 / §2.7.5):
        // `ModuleFn` becomes `Fn(&[KindedSlot], _) -> Result<KindedSlot, _>`
        // and the marshal step disappears (the typed body's
        // `TypedReturn` is converted directly to `KindedSlot` inside
        // shape-runtime).
        self.module_registry.register(module);
    }

    /// Register a module-function entry in the table and return its ID.
    ///
    /// Phase-2c surface (ADR-006 §2.7.4 / §2.7.5): the
    /// `ValueWord::ModuleFunction` carrier shape this function feeds
    /// depends on the deleted `ValueWord` runtime representation.
    /// Replaced with a kinded `NativeKind::ModuleFunction`-style ID
    /// carrier in the Phase-2c rebuild.
    pub fn register_module_fn_entry(
        &mut self,
        entry: shape_runtime::module_exports::ModuleFnEntry,
    ) -> usize {
        let id = self.module_fn_table.len();
        self.module_fn_table.push(entry);
        id
    }

    /// Invoke a module-function entry by ID.
    ///
    /// **W17-snapshot-roundtrip close (Phase 2d Wave 2.6, 2026-05-11).**
    /// Lands the kinded shape per ADR-006 §2.7.4 / §2.7.5: takes
    /// `&[KindedSlot]` and returns `Result<KindedSlot, VMError>`,
    /// dispatching through the existing [`module_fn_table`] entry
    /// (sum-typed `Typed` / `TypedAsync` per Phase 4c.3). The async
    /// arm runs the future to completion on the ambient tokio
    /// runtime; the sync arm calls the body directly with the slice's
    /// raw `u64` bits and the registered `arg_kinds` table on the
    /// receiver (the body is contract-bound to interpret each slot
    /// per its `arg_kinds[i]`).
    ///
    /// Per `module_exports::ModuleContext`, the body receives a borrow
    /// of the VM's type schema registry plus the optional invoker
    /// hooks needed for callbacks back into the VM. The body is
    /// `Send + Sync` so it can be invoked from worker tasks.
    ///
    /// Returns:
    /// - `Ok(KindedSlot)` — successful invocation; the slot carries
    ///   the projected `TypedReturn` value with the registered return
    ///   type's `NativeKind`.
    /// - `Err(VMError::InvalidCall)` — `fn_id` out of range for the
    ///   current `module_fn_table`.
    /// - `Err(VMError::RuntimeError(msg))` — body returned an error
    ///   string; the message propagates verbatim.
    /// - `Err(VMError::NotImplemented(msg))` — async body called with
    ///   no ambient tokio runtime, or `TypedReturn::*` arm that needs
    ///   the kind-threaded slot projection follow-up. Surface-and-stop
    ///   per ADR-006 §2.7.4 — no Bool-default fallback.
    pub(crate) fn invoke_module_fn_id_stub(
        &mut self,
        fn_id: usize,
        args: &[shape_value::KindedSlot],
    ) -> Result<shape_value::KindedSlot, VMError> {
        let entry = self
            .module_fn_table
            .get(fn_id)
            .ok_or(VMError::InvalidCall)?
            .clone();

        match entry {
            shape_runtime::module_exports::ModuleFnEntry::Typed(typed) => {
                // **W17-state-tier-roundtrip (Phase 2d Wave 3) +
                // WF-2E invoke_callable wiring (2026-07-05).**
                //
                // Capture the read-only `VmStateSnapshot` (so `state.*`
                // bodies introspect the VM via `ctx.vm_state`) and the
                // permission envelope BEFORE `&mut self` is parked in the
                // re-entrancy cell below. The snapshot owns its own
                // KindedSlot shares so the live VM is undisturbed.
                let vm_state_snap = self.capture_vm_state();
                let permissions = self.permission_context();
                let function_hash_raw = self.function_hash_raw.clone();

                // `ctx.schemas` sources the ambient thread-local registry
                // (`current_registry()`) — a cheap `Arc` clone of the same
                // superset `execution.rs` installs at run start (the merged
                // stdlib + user + inline-object-literal schemas). Borrowing
                // this *local* `Arc` instead of `&self.program.type_schema_registry`
                // is what frees `&mut self` for the re-entrant callback: the
                // schema borrow and the `&mut self` re-entry touch disjoint
                // memory, provably to the borrow checker — no unsafe aliasing,
                // no `mem::take` that would blank the registry mid-callback.
                let ambient_registry = shape_runtime::type_schema::current_registry();

                // `&mut self` is parked in a `RefCell` so the `Fn`-typed
                // `invoke_callable` bridge can take a fresh `&mut` per call.
                // This is the designed use of `ModuleContext::invoke_callable`
                // (e.g. `time.benchmark` driving a Shape callable back through
                // the VM): RefCell enforces exclusivity at runtime, so a
                // nested re-entry panics rather than aliasing. Mirrors how
                // VM-side array methods invoke closures via
                // `call_value_immediate_nb` — same ABI, same drive-to-completion
                // loop — just reached from the native-module boundary.
                let vm_cell = std::cell::RefCell::new(self);
                let body_result = {
                    let invoke_cb =
                        |callee: &shape_value::KindedSlot,
                         cb_args: &[shape_value::KindedSlot]|
                         -> Result<shape_value::KindedSlot, String> {
                            vm_cell
                                .borrow_mut()
                                .call_value_immediate_nb(callee, cb_args, None)
                                .map_err(|e| e.to_string())
                        };
                    // Sender-side per-function distributed-transfer seam
                    // (distributed §4.1.1). The dispatcher reads `self.program`
                    // through the SAME `vm_cell` that backs `invoke_cb` — a
                    // short-lived `borrow()` taken inside `call_remote`, NOT a
                    // whole-program clone (the deleted `CURRENT_PROGRAM`
                    // thread-local's per-dispatch clone is distributed R10).
                    // The two never overlap: a module body invokes `remote::call`
                    // OR an `invoke_callable` callback, never both re-entrantly —
                    // `call_remote` performs a wire round-trip and does not
                    // re-enter the VM, so its immutable `borrow()` cannot alias
                    // the callback's `borrow_mut()`. RefCell enforces this at
                    // runtime (WF-2E RefCell park + WF-2C remote-dispatch, both
                    // preserved).
                    let remote_dispatcher =
                        crate::executor::builtins::remote_builtins::ProgramRemoteDispatcher::new(
                            &vm_cell,
                        );
                    let ctx = shape_runtime::module_exports::ModuleContext {
                        schemas: &ambient_registry,
                        invoke_callable: Some(&invoke_cb),
                        raw_invoker: None,
                        function_hashes: Some(function_hash_raw.as_slice()),
                        vm_state: Some(&vm_state_snap),
                        // WF-1D security wiring: thread the VM's installed
                        // permission envelope into the gated-dispatch context.
                        permissions,
                        set_pending_resume: None,
                        set_pending_frame_resume: None,
                        remote_dispatch: Some(&remote_dispatcher),
                    };
                    // The body takes `&[KindedSlot]` carriers directly — the
                    // caller's kinds (VM §2.7.7 parallel kind track) flow
                    // through UNCHANGED. No raw-bits flatten, no re-stamp.
                    (typed.invoke)(args, &ctx).map_err(VMError::RuntimeError)
                    // `invoke_cb`, `remote_dispatcher`, and `ctx` all drop here
                    // — `vm_cell` is no longer borrowed, so `into_inner` below
                    // can reclaim `&mut self`.
                };
                let this = vm_cell.into_inner();
                let typed_return = body_result?;
                project_typed_return(
                    &this.builtin_schemas,
                    &this.program.type_schema_registry,
                    typed_return,
                )
            }
            shape_runtime::module_exports::ModuleFnEntry::TypedAsync(async_entry) => {
                // Async bodies own their arg vec across the await; clone
                // the caller's carriers (retains one share per heap slot)
                // — kinds preserved, no raw-bits flatten.
                let kinded_args: Vec<shape_value::KindedSlot> = args.to_vec();
                // #252 (owner ruling 2026-08-02, §R-G3): the async body owns
                // its permission envelope too. `ModuleContext` borrows the VM
                // and cannot cross an await, which is why async exports used to
                // run ungated; the `Arc<PermissionContext>` snapshot below is
                // `'static`, so the body gates its I/O at the call site exactly
                // like the sync arm above. The envelope is per-run immutable,
                // so this snapshot cannot go stale under the task.
                let permissions = self.permission_context();
                let fut = (async_entry.invoke)(kinded_args, permissions);
                // Install the program's ambient schema registry as a
                // TASK-LOCAL scope wrapping the future, so it is inherited
                // when the future runs on a shared-runtime worker thread (ser).
                // Argument marshalling (`FromSlot::from_kinded`) runs *inside*
                // the future (the `async move` block in `marshal.rs`): an
                // inline object-literal options arg (`Ptr(TypedObject)`) walks
                // its schema through `current_registry()`. The
                // `SyncRegistryScope` installed on the calling thread
                // (`execution.rs`) is thread-local and does NOT cross the spawn
                // boundary; `with_async_scope`'s task-local layer survives
                // tokio task migration. Without it the future sees only the
                // process-default registry and inline object-literal options
                // fail with "unknown TypedObject schema id N". Same
                // `current_registry()` handle the `Typed` arm above borrows.
                // The wrapped future stays `Send + 'static` (both the source
                // `ModuleAsyncFuture` and the `Arc<TypeSchemaRegistry>` are),
                // so it re-boxes into `ModuleAsyncFuture` for the spawn.
                let ambient_registry = shape_runtime::type_schema::current_registry();
                let scoped_fut: ModuleAsyncFuture = Box::pin(
                    shape_runtime::type_schema::with_async_scope(ambient_registry, fut),
                );
                // WF-2D real concurrency (Decision D1): do NOT drive the
                // future to completion inline. Spawn the schema-scoped future
                // onto the shared background runtime and return a `Future(id)`
                // handle immediately, so two independent `time::sleep`s
                // launched back-to-back overlap on the worker pool (~1s wall
                // for two 1s sleeps, not ~2s). `await`/`join` later collect the
                // result. This also keeps bug (b) fixed: nothing blocks the
                // ambient current-thread runtime here. ADR-006 §2.7.4
                // task-scheduler boundary.
                self.spawn_async_module_future(scoped_fut)
            }
        }
    }

    /// Populate extension module objects as module_bindings — W17-comptime-vm-dispatch rebuild.
    ///
    /// **W17-comptime-vm-dispatch (Phase 2d Wave 3, 2026-05-12).**
    /// Per ADR-006 §2.7.26 amendment. Builds a kinded `TypedObject`
    /// per registered extension module, with field slots that store
    /// **module-function-id field references** as `Ptr(HeapKind::ModuleFn)`
    /// inline-scalar payloads. The dispatch chain
    /// `LoadModuleBinding(idx) + GetFieldTyped(...) + CallValue` routes
    /// through:
    ///
    /// 1. `LoadModuleBinding(idx)` reads the kinded module-binding
    ///    slot (TypedObject + `Ptr(HeapKind::TypedObject)` kind) and
    ///    pushes it via `clone_with_kind` retain-on-read (§2.7.7).
    /// 2. `GetFieldTyped { type_id, field_idx, field_type_tag }` pops
    ///    the receiver, recovers the `Arc<TypedObjectStorage>` per
    ///    ADR-005 §1, and reads the field. The compiler emits
    ///    `field_type_tag = FIELD_TAG_ANY` for schema fields of
    ///    `FieldType::Any` (the comptime predeclared schema shape).
    ///    The `op_get_field_typed` body falls through to
    ///    `push_field_value_with_kind`, which sources the kind from
    ///    `storage.field_kinds[field_idx]` (the §2.7.7 parallel-kind
    ///    track) — resolving hardening item (f) — and pushes the
    ///    `module_fn_id as u64` bits with kind
    ///    `Ptr(HeapKind::ModuleFn)`.
    /// 3. `CallValue` pops args + callee, dispatches via
    ///    `call_value_immediate_nb` whose `Ptr(HeapKind::ModuleFn)`
    ///    arm routes to `invoke_module_fn_id_stub(bits as usize, args)` —
    ///    the same path used by `W17-snapshot-roundtrip` for direct
    ///    module-fn invocation.
    ///
    /// Per-module construction:
    ///
    /// - Look up the predeclared `__mod_<name>` schema (registered by
    ///   `compiler/comptime.rs::ensure_module_object_schema` before
    ///   bytecode compilation). The schema field names define the
    ///   storage's field order; missing schemas are skipped (the
    ///   module's exports remain unreachable through this binding).
    /// - For each typed export (sync and async), register a
    ///   `ModuleFnEntry::Typed` / `TypedAsync` into `module_fn_table`
    ///   to obtain a `module_fn_id`.
    /// - For each schema field, look up the matching `module_fn_id`,
    ///   write a `ValueSlot::from_raw(module_fn_id as u64)` with
    ///   `field_kinds[i] = Ptr(HeapKind::ModuleFn)` and `heap_mask`
    ///   bit set. Unmatched fields (a schema field with no
    ///   corresponding export) get the `(0u64, NativeKind::Bool)`
    ///   sentinel pair — same shape as the
    ///   `module_binding_pad_to_kinded` uninitialised-slot convention.
    /// - Construct `Arc<TypedObjectStorage>` via the typed constructor
    ///   per ADR-006 §2.4 and write the `Ptr(HeapKind::TypedObject)`
    ///   slot to the module-binding via
    ///   `module_binding_write_kinded` (§2.7.8 / Q10 lockstep).
    ///
    /// Resolves the upstream `populate_module_objects` no-op blocker
    /// flagged by `W17-snapshot-roundtrip` (commit `fbfbfb6`). The
    /// 4 comptime introspection forms wired by C2-comptime-rebuild
    /// (`a5df165`) — `build_config` / `implements` / `warning` /
    /// `error` — now dispatch end-to-end via VM mode.
    /// Bind every module-scope binding that names a top-level function to that
    /// function's value, WITHOUT running the program's top-level code.
    ///
    /// #202. The isolated task VM (`run_isolated_async_fn`) deliberately does
    /// not run top-level code — that would re-execute the whole program — so the
    /// `StoreModuleBinding` that normally seeds these slots never happens and
    /// they stay at their default. A call that reaches a top-level function
    /// through its module binding (`LoadModuleBinding` + `CallValue`, which is
    /// how a foreign stub is reached — see `remote.rs`'s WF-3E fixAB note) then
    /// dispatches on an uninitialised slot and fails with "callee must be
    /// Closure, ModuleFn or UInt64, got Bool".
    ///
    /// Function bindings are the part of module state that is knowable without
    /// running anything: a top-level `fn`'s value is its id, fixed at compile
    /// time. Bindings initialised by top-level EXPRESSIONS remain uninitialised
    /// — that is the documented WF-2D-fu isolation boundary and this does not
    /// move it.
    pub fn seed_module_function_bindings(&mut self) {
        use shape_value::NativeKind;

        // The binding store grows lazily (`module_binding_pad_to_kinded`), so it
        // is EMPTY until the first write — which, on this path, never happens.
        // Pad first or the loop below has nothing to walk.
        let binding_count = self.program.module_binding_names.len();
        if binding_count == 0 {
            return;
        }
        self.module_binding_pad_to_kinded(binding_count - 1);

        let names = self.program.module_binding_names.clone();
        for (idx, name) in names.iter().enumerate() {
            // Only seed a slot still carrying the never-written sentinel; never
            // overwrite a value something else already put there.
            if self.module_binding_kinds[idx] != NativeKind::Bool
                || self.module_bindings[idx] != Self::NONE_BITS
            {
                continue;
            }
            let Some(func_id) = self.program.functions.iter().position(|f| &f.name == name) else {
                continue;
            };
            // Same encoding `PushConst` gives `Constant::Function` (stack_ops):
            // the id inline, kinded `UInt64`. No heap share to account for.
            self.module_bindings[idx] = func_id as u64;
            self.module_binding_kinds[idx] = NativeKind::UInt64;
        }
    }

    pub fn populate_module_objects(&mut self) {
        use shape_runtime::module_exports::ModuleFnEntry;
        use shape_value::heap_value::TypedObjectStorage;
        use shape_value::{HeapKind, NativeKind, ValueSlot};
        use std::sync::Arc;

        // WF-2G GAP A idempotency guard. `register_module_fn_entry` (the
        // only writer of `module_fn_table`) runs exclusively here, and each
        // VM calls `populate_module_objects` at most once per lifetime EXCEPT
        // on the resume path, where `from_snapshot` calls it before the
        // restore two-pass (so the ModuleFn restore arm can resolve
        // name → id) and `resume_from_snapshot_impl` calls it again. A second
        // full run would re-register every entry (doubling `module_fn_table`
        // and shifting ids), so short-circuit once the table is populated.
        // The thread-local name table installed on the first run stays valid.
        if !self.module_fn_table.is_empty() {
            return;
        }

        // Phase 1 — collect: gather per-module data without taking a
        // mutable borrow on `self` while iterating the registry. The
        // `register_module_fn_entry` call mutates `self.module_fn_table`,
        // which conflicts with an active borrow on `self.module_registry`.
        let module_names: Vec<String> = self
            .module_registry
            .module_names()
            .iter()
            .map(|s| s.to_string())
            .collect();

        // WF-2G GAP A: accumulate `module_fn_id → "module::export"` in
        // registration order (id == index; table starts empty per the guard
        // above) for the snapshot projection / restore name table.
        let mut module_fn_names: Vec<String> = Vec::new();

        for module_name in module_names {
            // Resolve the module's typed exports. The `module_registry.get`
            // borrow is local to this iteration and dropped before we
            // mutate `module_fn_table` below.
            let typed_entries: Vec<(String, ModuleFnEntry)> = {
                let module = match self.module_registry.get(&module_name) {
                    Some(m) => m,
                    None => continue,
                };
                let typed = module.typed_exports();
                let mut entries: Vec<(String, ModuleFnEntry)> =
                    Vec::with_capacity(typed.functions.len() + typed.async_functions.len());
                for (export_name, typed_fn) in &typed.functions {
                    entries.push((export_name.clone(), ModuleFnEntry::Typed(typed_fn.clone())));
                }
                for (export_name, typed_async) in &typed.async_functions {
                    entries.push((
                        export_name.clone(),
                        ModuleFnEntry::TypedAsync(typed_async.clone()),
                    ));
                }
                entries
            };

            // Locate the binding index/indices for this module — prefer the
            // hidden native binding (`__imported_module__::<name>`,
            // injected by the compiler's
            // `ensure_hidden_native_module_binding`), fall back to the
            // plain fully-qualified binding name. The hidden form is used
            // when a Shape artifact module with the same name would
            // otherwise shadow the native object.
            //
            // WF-3E fixAB (D3/D7): a `use std::core::env` namespace import
            // resolves foreign call sites to the SHORT-alias binding (`env`),
            // not the fully-qualified `std::core::env` binding. On a local
            // run the short binding is set by top-level module-init; the
            // per-function remote-dispatch path never runs top-level, so the
            // transferred body reads the short binding as the uninitialised
            // sentinel and `call_value_immediate_nb` refuses the Null/Bool
            // callee. We therefore ALSO populate the last-segment short-alias
            // binding — but only when it is still uninitialised, so a real
            // top-level value (or a value written for another module) is never
            // clobbered (ADR-006 §2.7.8: no fabricated kind; a fresh typed
            // `Ptr(HeapKind::TypedObject)` module object per binding).
            let hidden_name = format!("__imported_module__::{}", module_name);
            let short_name = module_name
                .rsplit("::")
                .next()
                .unwrap_or(module_name.as_str());
            let mut target_binding_idxs: Vec<usize> = Vec::new();
            for (i, n) in self.program.module_binding_names.iter().enumerate() {
                let is_canonical = n == &hidden_name || n == &module_name;
                let is_short_uninit =
                    n.as_str() == short_name && short_name != module_name.as_str() && {
                        // Only claim the short alias if it is still the
                        // uninitialised sentinel — never overwrite a live
                        // value.
                        let (bits, kind) = self.module_binding_read_kinded_raw(i);
                        bits == 0 && matches!(kind, NativeKind::Bool | NativeKind::Null)
                    };
                if is_canonical || is_short_uninit {
                    target_binding_idxs.push(i);
                }
            }
            if target_binding_idxs.is_empty() {
                continue; // No binding name — nothing to populate.
            }

            // Resolve the predeclared module-object schema. The schema
            // is registered before compilation by
            // `compiler/comptime.rs::ensure_module_object_schema`
            // (under canonical name `__mod_<module_name>`).
            // Without it we can't define a stable field order for
            // the typed-object layout, so skip — the binding stays at
            // the no-op-on-drop sentinel and any reference to a field
            // through this binding will surface clean at GetFieldTyped.
            let schema_name = format!("__mod_{}", module_name);
            let schema = match self.lookup_schema_by_name(&schema_name) {
                Some(s) => s.clone(),
                None => continue,
            };

            // Register each typed entry into `module_fn_table` and
            // build a name → module_fn_id lookup.
            let mut fn_id_by_name: std::collections::HashMap<String, u64> =
                std::collections::HashMap::with_capacity(typed_entries.len());
            for (export_name, entry) in typed_entries {
                let fn_id = self.register_module_fn_entry(entry);
                // WF-2G GAP A: record the qualified export name at index
                // `fn_id` (dense, registration order). `module_fn_names.len()`
                // equals the pre-registration table length == fn_id.
                debug_assert_eq!(fn_id, module_fn_names.len());
                module_fn_names.push(format!("{module_name}::{export_name}"));
                fn_id_by_name.insert(export_name, fn_id as u64);
            }

            // Build the typed-object slot list in schema field order.
            // Each field maps to either:
            //   - a known module-fn-id → ValueSlot(fn_id) with kind
            //     Ptr(HeapKind::ModuleFn) and heap_mask bit set, or
            //   - no matching export → (0, NativeKind::Bool) sentinel
            //     (same shape as the module-binding-pad uninitialised
            //     slot convention — no Bool-default fallback for a
            //     known-callable-but-missing field; the compiler
            //     should have surfaced 'module has no export' earlier).
            let field_count = schema.fields.len();
            let mut slots: Vec<ValueSlot> = Vec::with_capacity(field_count);
            let mut field_kinds: Vec<NativeKind> = Vec::with_capacity(field_count);
            let mut heap_mask: u64 = 0;
            for (i, field) in schema.fields.iter().enumerate() {
                match fn_id_by_name.get(&field.name) {
                    Some(&fn_id) => {
                        // ModuleFn inline-scalar slot: bits = fn_id,
                        // kind = Ptr(HeapKind::ModuleFn). Mark the
                        // heap_mask bit so the read path sees the slot
                        // as "kind-bearing" and dispatches through the
                        // FIELD_TAG_ANY / field_kinds resolver in
                        // op_get_field_typed.
                        slots.push(ValueSlot::from_raw(fn_id));
                        field_kinds.push(NativeKind::Ptr(HeapKind::ModuleFn));
                        heap_mask |= 1u64 << i;
                    }
                    None => {
                        // Schema field present but no typed export:
                        // sentinel slot. `clone_with_kind` /
                        // `drop_with_kind` are no-op on (0, Bool).
                        slots.push(ValueSlot::from_raw(0));
                        field_kinds.push(NativeKind::Bool);
                    }
                }
            }

            // Share the immutable per-slot kind track across every target
            // binding's module object (Arc is refcounted, immutable).
            let field_kinds_arc: Arc<[NativeKind]> = Arc::from(field_kinds.into_boxed_slice());

            // Wave 2 Round 4 D4 ckpt-1: migrated to v2-raw `_new` per D1
            // API surface. The raw pointer is directly the carrier bits;
            // `module_binding_write_kinded` consumes a u64 + NativeKind
            // pair so this site cleanly migrates without depending on
            // the `HeapValue::TypedObject` variant signature flip.
            //
            // WF-3E fixAB (D3/D7): allocate a FRESH module object per target
            // binding so each binding owns its own strong count (canonical
            // `std::core::env` binding AND the short `env` alias the
            // transferred body actually loads). The ModuleFn-id slots are
            // inline scalars, so cloning the slot array is a plain copy — no
            // per-slot share accounting.
            for binding_idx in &target_binding_idxs {
                let ptr = TypedObjectStorage::_new(
                    schema.id as u64,
                    slots.clone().into_boxed_slice(),
                    heap_mask,
                    field_kinds_arc.clone(),
                );
                // Hand off one share to the binding slot. The v2-raw pointer
                // bits are the carrier directly (per ADR-006 §2.4 / D1's
                // `from_typed_object_raw` constructor contract). One strong
                // count owned by us is transferred into the binding via
                // `module_binding_write_kinded`.
                let bits = ptr as u64;
                self.module_binding_write_kinded(
                    *binding_idx,
                    bits,
                    NativeKind::Ptr(HeapKind::TypedObject),
                );
            }
        }

        // WF-2G GAP A: install the `module_fn_id → "module::export"` table on
        // this thread so snapshot projection (`slot_to_serializable`) and
        // restore (`serializable_to_slot`) can carry native module-fn values
        // by their sound cross-process identity (the qualified name). Native
        // stdlib fns have no content hash and re-register deterministically,
        // so the id order here matches every node's.
        shape_runtime::snapshot::install_module_fn_name_table(module_fn_names);
    }
}

#[cfg(test)]
mod stage_k1_tests {
    //! STAGE K1 (2026-06-02) round-trip verification for the
    //! `project_typed_return` container/wrapper arms + the
    //! `project_concrete_return` leaf arms. Each test drives a representative
    //! value through a **real module-fn return**: it registers a
    //! `ModuleFnEntry::Typed` whose body returns the `TypedReturn` under test,
    //! invokes it through [`VirtualMachine::invoke_module_fn_id_stub`] (the
    //! same dispatch path `LoadModuleBinding + GetFieldTyped + CallValue`
    //! routes through), and recovers the projected `KindedSlot` to assert the
    //! value survived the projection.
    //!
    //! Recovery reads wrapper carriers through the same `result_option_carrier`
    //! helper as production opcodes, and borrows raw `*const TypedArray<T>` /
    //! `*const TypedObjectStorage` / `*const HashMapKindedRef` for the heap
    //! carriers. The recovered `KindedSlot`'s own `Drop` retires the carrier.

    use super::*;
    use shape_runtime::marshal::FromSlot;
    use shape_runtime::typed_module_exports::{
        ConcreteReturn, ConcreteType, TypedModuleFunction, TypedReturn,
    };
    use shape_value::heap_value::{HashMapKindedRef, TypedObjectStorage};
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::TypedArray;
    use shape_value::{HeapKind, KindedSlot, NativeKind};
    use std::sync::Arc;

    /// Register a typed module-fn whose 0-arg body returns `tr`, then invoke
    /// it through the real `invoke_module_fn_id_stub` dispatch path. Returns
    /// the projected `KindedSlot`.
    fn invoke_typed_return(
        vm: &mut VirtualMachine,
        tr: TypedReturn,
    ) -> Result<KindedSlot, VMError> {
        let tr_cell = std::sync::Mutex::new(Some(tr));
        let tmf = TypedModuleFunction {
            invoke: Arc::new(move |_slots, _ctx| {
                Ok(tr_cell
                    .lock()
                    .unwrap()
                    .take()
                    .expect("module-fn body invoked once"))
            }),
            return_type: ConcreteType::Any,
            arg_types: vec![],
            arg_kinds: vec![],
        };
        let entry = shape_runtime::module_exports::ModuleFnEntry::Typed(tmf);
        let fn_id = vm.register_module_fn_entry(entry);
        vm.invoke_module_fn_id_stub(fn_id, &[])
    }

    fn roundtrip_with_schemas(
        tr: TypedReturn,
    ) -> (KindedSlot, shape_runtime::type_schema::BuiltinSchemaIds) {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let schemas = vm.builtin_schemas.clone();
        let slot = invoke_typed_return(&mut vm, tr).expect("module-fn invocation projects cleanly");
        (slot, schemas)
    }

    fn roundtrip(tr: TypedReturn) -> KindedSlot {
        roundtrip_with_schemas(tr).0
    }

    fn register_json_schema(vm: &mut VirtualMachine) -> u32 {
        use shape_runtime::type_schema::EnumVariantInfo;

        vm.program.type_schema_registry.register_enum_scoped(
            "Json",
            vec![
                EnumVariantInfo::new("Null", JSON_VARIANT_NULL as u16, 0),
                EnumVariantInfo::new("Bool", JSON_VARIANT_BOOL as u16, 1),
                EnumVariantInfo::new("Int", JSON_VARIANT_INT as u16, 1),
                EnumVariantInfo::new("Number", JSON_VARIANT_NUMBER as u16, 1),
                EnumVariantInfo::new("Str", JSON_VARIANT_STR as u16, 1),
                EnumVariantInfo::new("Array", JSON_VARIANT_ARRAY as u16, 1),
                EnumVariantInfo::new("Object", JSON_VARIANT_OBJECT as u16, 1),
            ],
        )
    }

    #[test]
    fn array_i64_roundtrips() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::ArrayI64(vec![
            1, 2, 3, -7,
        ])));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        let arr = slot.raw() as *const TypedArray<i64>;
        let got = unsafe { TypedArray::<i64>::as_slice(arr) };
        assert_eq!(got, &[1, 2, 3, -7]);
    }

    #[test]
    fn array_f64_roundtrips() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::ArrayF64(vec![
            1.5, 2.5, -0.25,
        ])));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        let arr = slot.raw() as *const TypedArray<f64>;
        let got = unsafe { TypedArray::<f64>::as_slice(arr) };
        assert_eq!(got, &[1.5, 2.5, -0.25]);
    }

    #[test]
    fn bytes_roundtrips_as_array_int() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::Bytes(vec![
            0u8, 127, 255,
        ])));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        // Bytes surface as Array<int>: each byte widened to i64.
        let arr = slot.raw() as *const TypedArray<i64>;
        let got = unsafe { TypedArray::<i64>::as_slice(arr) };
        assert_eq!(got, &[0i64, 127, 255]);
    }

    #[test]
    fn array_string_roundtrips() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::ArrayString(vec![
            "alpha".to_string(),
            "beta".to_string(),
        ])));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        let arr = slot.raw() as *const TypedArray<*const StringObj>;
        let ptrs = unsafe { TypedArray::<*const StringObj>::as_slice(arr) };
        let got: Vec<&str> = ptrs
            .iter()
            .map(|&p| unsafe { StringObj::as_str(p) })
            .collect();
        assert_eq!(got, vec!["alpha", "beta"]);
    }

    #[test]
    fn array_string_rows_roundtrips() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::ArrayStringRows(
            vec![
                vec![Arc::new("a".to_string()), Arc::new("b".to_string())],
                vec![Arc::new("1".to_string()), Arc::new("2".to_string())],
            ],
        )));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));

        let rows = <Vec<Vec<Arc<String>>> as FromSlot>::from_slot(slot.raw());
        let got: Vec<Vec<String>> = rows
            .into_iter()
            .map(|row| row.into_iter().map(|cell| (*cell).clone()).collect())
            .collect();
        assert_eq!(
            got,
            vec![
                vec!["a".to_string(), "b".to_string()],
                vec!["1".to_string(), "2".to_string()],
            ]
        );
    }

    #[test]
    fn array_heap_value_typed_object_roundtrips_via_k2() {
        // Build a Vec<Arc<HeapValue::TypedObject>> via the same typed-object
        // builder the wrapper arms use, then project through the K2
        // dispatcher's ArrayHeapValue arm.
        let obj = super::project_concrete_return_for_test_typed_object();
        let elems = vec![obj];
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::ArrayHeapValue(elems)));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
        let arr = slot.raw() as *const TypedArray<*const TypedObjectStorage>;
        let n = unsafe { TypedArray::<*const TypedObjectStorage>::len(arr) };
        assert_eq!(n, 1);
    }

    #[test]
    fn hashmap_string_string_roundtrips() {
        let slot = roundtrip(TypedReturn::Concrete(ConcreteReturn::HashMapStringString(
            vec![
                ("k1".to_string(), "v1".to_string()),
                ("k2".to_string(), "v2".to_string()),
            ],
        )));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::HashMap));
        let kref = slot.raw() as *const HashMapKindedRef;
        match unsafe { &*kref } {
            HashMapKindedRef::String(data) => {
                assert_eq!(data.len(), 2);
                // keys + values are parallel string arrays.
                let keys = unsafe { TypedArray::<*const StringObj>::as_slice(data.keys) };
                let vals = unsafe { TypedArray::<*const StringObj>::as_slice(data.values) };
                let pairs: Vec<(&str, &str)> = keys
                    .iter()
                    .zip(vals.iter())
                    .map(|(&k, &v)| unsafe { (StringObj::as_str(k), StringObj::as_str(v)) })
                    .collect();
                assert!(pairs.contains(&("k1", "v1")));
                assert!(pairs.contains(&("k2", "v2")));
            }
            other => panic!(
                "expected HashMapKindedRef::String, got {:?}",
                std::mem::discriminant(other)
            ),
        }
    }

    #[test]
    fn ok_wrapper_roundtrips() {
        let (slot, schemas) = roundtrip_with_schemas(TypedReturn::Ok(ConcreteReturn::I64(99)));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let view = result_option_carrier::read_result(&schemas, &slot)
            .unwrap()
            .unwrap();
        assert!(view.is_ok());
        let payload = view.clone_payload().unwrap();
        assert_eq!(payload.kind(), NativeKind::Int64);
        assert_eq!(payload.slot().raw() as i64, 99);
        drop(payload);
    }

    #[test]
    fn err_wrapper_roundtrips() {
        let (slot, schemas) =
            roundtrip_with_schemas(TypedReturn::Err(ConcreteReturn::String("boom".to_string())));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let view = result_option_carrier::read_result(&schemas, &slot)
            .unwrap()
            .unwrap();
        assert!(!view.is_ok());
        let payload = view.clone_payload().unwrap();
        assert_eq!(payload.kind(), NativeKind::String);
        assert_eq!(payload.as_str(), Some("boom"));
        drop(payload);
    }

    #[test]
    fn some_wrapper_roundtrips() {
        let (slot, schemas) = roundtrip_with_schemas(TypedReturn::Some(ConcreteReturn::Bool(true)));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let view = result_option_carrier::read_option(&schemas, &slot)
            .unwrap()
            .unwrap();
        assert!(view.is_some());
        let payload = view.clone_payload().unwrap();
        assert_eq!(payload.kind(), NativeKind::Bool);
        assert_eq!(payload.slot().raw(), 1);
        drop(payload);
    }

    /// Canonical Option representation: a module function's `None` return
    /// projects to the NULL SENTINEL, not a non-null `__Option` carrier —
    /// the pattern-matcher's `None` discriminator null-codes (`IsNull`), so
    /// a non-null carrier read as `Some` and faulted in `UnwrapOption`
    /// (ADR-009 B2 slice S4; mirrors
    /// `foreign_marshal.rs::msgpack_to_kinded_slot`'s §4.4 stage-3 rule).
    #[test]
    fn none_wrapper_projects_to_the_null_sentinel() {
        let (slot, _schemas) = roundtrip_with_schemas(TypedReturn::None);
        assert_eq!(slot.kind(), shape_value::KindedSlot::none().kind());
        assert_eq!(slot.raw(), shape_value::KindedSlot::none().raw());
    }

    #[test]
    fn object_pairs_roundtrips_as_typed_object() {
        let slot = roundtrip(TypedReturn::ObjectPairs(vec![
            ("count".to_string(), ConcreteReturn::I64(5)),
            ("ratio".to_string(), ConcreteReturn::F64(0.5)),
        ]));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let obj = slot.raw() as *const TypedObjectStorage;
        let storage = unsafe { &*obj };
        assert_eq!(storage.slots().len(), 2);
    }

    #[test]
    fn object_pairs_string_field_clone_preserves_field_provenance() {
        let slot = roundtrip(TypedReturn::ObjectPairs(vec![(
            "name".to_string(),
            ConcreteReturn::String("Ada".to_string()),
        )]));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let storage = slot.as_typed_object_storage().unwrap();
        assert_eq!(storage.field_kinds[0], NativeKind::String);

        let cloned = storage
            .clone_field_kinded(0)
            .expect("string field should clone from typed object storage");
        assert_eq!(cloned.kind(), NativeKind::String);
        assert_eq!(cloned.as_str(), Some("Ada"));
        drop(cloned);
        drop(slot);
    }

    #[test]
    fn ok_object_pairs_roundtrips() {
        let (slot, schemas) = roundtrip_with_schemas(TypedReturn::OkObjectPairs(vec![(
            "x".to_string(),
            ConcreteReturn::I64(1),
        )]));
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let view = result_option_carrier::read_result(&schemas, &slot)
            .unwrap()
            .unwrap();
        assert!(view.is_ok());
        let payload = view.clone_payload().unwrap();
        assert_eq!(payload.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        drop(payload);
    }

    #[test]
    fn json_value_requires_registered_json_schema() {
        let mut vm = VirtualMachine::new(VMConfig::default());
        let err = invoke_typed_return(
            &mut vm,
            TypedReturn::Ok(ConcreteReturn::JsonValue(
                shape_runtime::json_value::JsonValue::Int(1),
            )),
        )
        .unwrap_err();
        match err {
            shape_value::VMError::RuntimeError(msg) => {
                assert!(msg.contains("registered `Json` enum schema"), "{msg}");
            }
            other => panic!("JsonValue missing-schema must surface RuntimeError, got {other:?}"),
        }
    }

    #[test]
    fn json_value_string_payload_clone_preserves_field_provenance() {
        use shape_runtime::json_value::JsonValue;

        let mut vm = VirtualMachine::new(VMConfig::default());
        let json_schema_id = register_json_schema(&mut vm) as u64;
        let schemas = vm.builtin_schemas.clone();
        let slot = invoke_typed_return(
            &mut vm,
            TypedReturn::Ok(ConcreteReturn::JsonValue(JsonValue::String(
                "Ada".to_string(),
            ))),
        )
        .expect("JsonValue string should project through registered Json schema");

        let result = result_option_carrier::read_result(&schemas, &slot)
            .unwrap()
            .unwrap();
        assert!(result.is_ok());
        let payload = result.clone_payload().unwrap();
        assert_eq!(payload.kind(), NativeKind::Ptr(HeapKind::TypedObject));

        let json_obj = payload.as_typed_object_storage().unwrap();
        assert_eq!(json_obj.schema_id, json_schema_id);
        assert_eq!(json_obj.slots()[0].raw() as i64, JSON_VARIANT_STR);
        assert_eq!(json_obj.field_kinds[1], NativeKind::String);

        let cloned_payload = json_obj
            .clone_field_kinded(1)
            .expect("Json::Str payload clone should preserve field provenance");
        assert_eq!(cloned_payload.kind(), NativeKind::String);
        assert_eq!(cloned_payload.as_str(), Some("Ada"));
        drop(cloned_payload);
        drop(payload);
    }

    #[test]
    fn json_value_roundtrips_as_schema_backed_json_enum() {
        use shape_runtime::json_value::JsonValue;

        let mut vm = VirtualMachine::new(VMConfig::default());
        let json_schema_id = register_json_schema(&mut vm) as u64;
        let schemas = vm.builtin_schemas.clone();
        let slot = invoke_typed_return(
            &mut vm,
            TypedReturn::Ok(ConcreteReturn::JsonValue(JsonValue::Object(vec![
                ("name".to_string(), JsonValue::String("Ada".to_string())),
                (
                    "items".to_string(),
                    JsonValue::Array(vec![JsonValue::Int(7), JsonValue::Bool(true)]),
                ),
            ]))),
        )
        .expect("JsonValue should project through registered Json schema");

        let result = result_option_carrier::read_result(&schemas, &slot)
            .unwrap()
            .unwrap();
        assert!(result.is_ok());
        let payload = result.clone_payload().unwrap();
        assert_eq!(payload.kind(), NativeKind::Ptr(HeapKind::TypedObject));

        let json_obj = payload.as_typed_object_storage().unwrap();
        assert_eq!(json_obj.schema_id, json_schema_id);
        assert_eq!(json_obj.slots()[0].raw() as i64, JSON_VARIANT_OBJECT);
        assert_eq!(json_obj.field_kinds[0], NativeKind::Int64);
        assert_eq!(json_obj.field_kinds[1], NativeKind::Ptr(HeapKind::HashMap));

        let kref = json_obj.slots()[1].raw() as *const HashMapKindedRef;
        let object_data = match unsafe { &*kref } {
            HashMapKindedRef::TypedObject(data) => data,
            other => panic!(
                "expected Json::Object payload HashMapKindedRef::TypedObject, got {:?}",
                std::mem::discriminant(other)
            ),
        };
        assert_eq!(object_data.len(), 2);

        let name_idx = object_data.get_index("name").unwrap();
        let name_ptr = unsafe { (&*(*object_data.values).data.add(name_idx)).as_ptr() };
        let name_storage = unsafe { &*name_ptr };
        assert_eq!(name_storage.schema_id, json_schema_id);
        assert_eq!(name_storage.slots()[0].raw() as i64, JSON_VARIANT_STR);
        assert_eq!(name_storage.field_kinds[1], NativeKind::String);
        let name = unsafe { &*(name_storage.slots()[1].raw() as *const String) };
        assert_eq!(name.as_str(), "Ada");
        let cloned_name_payload = name_storage
            .clone_field_kinded(1)
            .expect("Json::Str payload clone should preserve field provenance");
        assert_eq!(cloned_name_payload.kind(), NativeKind::String);
        assert_eq!(cloned_name_payload.as_str(), Some("Ada"));
        drop(cloned_name_payload);

        let items_idx = object_data.get_index("items").unwrap();
        let items_ptr = unsafe { (&*(*object_data.values).data.add(items_idx)).as_ptr() };
        let items_storage = unsafe { &*items_ptr };
        assert_eq!(items_storage.schema_id, json_schema_id);
        assert_eq!(items_storage.slots()[0].raw() as i64, JSON_VARIANT_ARRAY);
        assert_eq!(
            items_storage.field_kinds[1],
            NativeKind::Ptr(HeapKind::TypedArray)
        );

        let arr = items_storage.slots()[1].raw() as *const TypedArray<*const TypedObjectStorage>;
        let elems = unsafe { TypedArray::<*const TypedObjectStorage>::as_slice(arr) };
        assert_eq!(elems.len(), 2);
        let first = unsafe { &*elems[0] };
        assert_eq!(first.slots()[0].raw() as i64, JSON_VARIANT_INT);
        assert_eq!(first.slots()[1].raw() as i64, 7);
        assert_eq!(first.field_kinds[1], NativeKind::Int64);
        let second = unsafe { &*elems[1] };
        assert_eq!(second.slots()[0].raw() as i64, JSON_VARIANT_BOOL);
        assert_eq!(second.slots()[1].raw(), 1);
        assert_eq!(second.field_kinds[1], NativeKind::Bool);

        drop(payload);
    }

    #[test]
    fn hashmap_string_heap_value_surfaces_clean_k3() {
        // K3 territory — must stay SURFACED (no shim) pending the ADR-006
        // HashMapData kinded-value-track amendment.
        let mut vm = VirtualMachine::new(VMConfig::default());
        let obj = super::project_concrete_return_for_test_typed_object();
        let tr_cell = std::sync::Mutex::new(Some(TypedReturn::Concrete(
            ConcreteReturn::HashMapStringHeapValue(vec![("k".to_string(), obj)]),
        )));
        let tmf = TypedModuleFunction {
            invoke: Arc::new(move |_slots, _ctx| Ok(tr_cell.lock().unwrap().take().unwrap())),
            return_type: ConcreteType::Any,
            arg_types: vec![],
            arg_kinds: vec![],
        };
        let fn_id =
            vm.register_module_fn_entry(shape_runtime::module_exports::ModuleFnEntry::Typed(tmf));
        let err = vm.invoke_module_fn_id_stub(fn_id, &[]).unwrap_err();
        assert!(
            matches!(err, shape_value::VMError::NotImplemented(_)),
            "HashMapStringHeapValue (K3) must surface NotImplemented, got {err:?}"
        );
    }
}

/// Test-only helper: build a representative `Arc<HeapValue::TypedObject>`
/// for the K2 / K3 array-and-map-of-heap-value tests above. Lives outside
/// the `#[cfg(test)]` module so it can be a `super::` reference from the
/// nested test module while still being compiled only under `cfg(test)`.
#[cfg(test)]
fn project_concrete_return_for_test_typed_object()
-> std::sync::Arc<shape_value::heap_value::HeapValue> {
    use shape_runtime::type_schema::typed_object_from_pairs;
    use shape_value::KindedSlot;
    use shape_value::heap_value::HeapValue;
    // Build a 1-field typed object via the shared builder, then recover its
    // raw TypedObject pointer into an Arc<HeapValue::TypedObject> carrier
    // (the ArrayHeapValue / HashMapStringHeapValue element shape).
    let slot: KindedSlot = typed_object_from_pairs(&[("id", KindedSlot::from_int(7))]);
    let ptr = slot.raw() as *const shape_value::heap_value::TypedObjectStorage;
    // The slot owns one share; transfer it into the Arc<HeapValue> wrapper
    // via `TypedObjectPtr::new` (which takes over that single share).
    std::mem::forget(slot);
    let tp = shape_value::heap_value::TypedObjectPtr::new(ptr);
    std::sync::Arc::new(HeapValue::TypedObject(tp))
}
