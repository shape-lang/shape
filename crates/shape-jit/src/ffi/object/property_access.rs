// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 7 sites
//     jit_box(HK_STRING, ...) — string char index, data_ref symbol/timezone
//     jit_box(HK_TIME, ...) — data_ref datetime
//     jit_box(HK_TIMEFRAME, ...) — data_ref timeframe
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Property Access Operations
//!
//! Functions for accessing properties on objects, arrays, strings, series,
//! time values, data references, and other types.

use std::collections::HashMap;

use super::super::super::context::{JITDataReference, JITDuration};
// super::super::super::jit_array::JitArray removed — see jit_array.rs
// SURFACE comment. The HK_ARRAY arms below now route to surface-and-stop
// per ADR-006 §2.7.4 / W10 jit-playbook §5.
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;

#[cold]
#[track_caller]
fn unsupported_legacy_heap_kind(func_name: &str, kind: Option<u16>) -> ! {
    match kind {
        Some(kind) => panic!(
            "SURFACE: {func_name} received unsupported legacy JIT heap kind {kind}; \
             ADR-006 section 2.7.5/2.7.10 requires a kinded property entry or an explicit HK arm."
        ),
        None => panic!(
            "SURFACE: {func_name} received non-heap bits where a legacy JIT property carrier \
             was required; ADR-006 section 2.7.5 requires the caller to pass a NativeKind \
             companion instead of probing raw bits."
        ),
    }
}

// ============================================================================
// Property Access (multi-type)
// ============================================================================

/// Get property from object, array, string, data row, duration, time, or series
#[inline(always)]
pub extern "C" fn jit_get_prop(obj_bits: u64, key_bits: u64) -> u64 {
    unsafe {
        // Get key as string if it's a string
        let key_str: Option<&str> = if is_heap_kind(key_bits, HK_STRING) {
            Some(unbox_string(key_bits))
        } else {
            None
        };

        // Wave-7 jit-typed-pointer-migration Phase B: the migrated producer
        // `jit_typed_object_alloc` emits the canonical v2-raw
        // `*mut TypedObjectStorage` carrier — a BARE heap pointer (sign bit
        // clear), NOT a NaN-boxed `box_typed_object`. Such a carrier is
        // `is_heap() == false`, so the legacy NaN-box `heap_kind` dispatch
        // below never routes it to a TypedObject arm. Resolve the field here
        // off the out-of-line slot buffer via `slots()` — the same shape the
        // phase-1 `jit_typed_object_get_field` FFI migrated to. The carrier is
        // self-describing: its `HeapHeader.kind` (offset 4) reads
        // `HEAP_KIND_V2_TYPED_OBJECT`, which distinguishes it from the legacy
        // `UnifiedValue` carriers (kind prefix at offset 0, so this reads their
        // refcount low bits ≠ 86) and from other v2-raw carriers (TypedArray =
        // 80, etc.). This is the same producer-placed-field read `read_heap_kind`
        // performs for the NaN-box path (ADR-006 §2.7.5: "not tag-bit dispatch —
        // a field-load from a heap-resident struct that the producing call
        // placed there"), read at the v2 header's offset. NO NaN-box unwrap, NO
        // inline-cell offset, NO Bool-default.
        if obj_bits != 0 && obj_bits & 0x8000_0000_0000_0000 == 0 {
            use shape_value::heap_value::TypedObjectStorage;
            use shape_value::v2::heap_header::HEAP_KIND_V2_TYPED_OBJECT;
            let hdr_kind = *((obj_bits as *const u8).add(4) as *const u16);
            if hdr_kind == HEAP_KIND_V2_TYPED_OBJECT {
                let ptr = obj_bits as *const TypedObjectStorage;
                let Some(key) = key_str else { return TAG_NULL };
                let schema_id = (*ptr).schema_id as u32;
                // Two-tier schema resolution: global stdlib registry first,
                // then the trampoline VM's bytecode registry (user types).
                let mut field_idx =
                    shape_runtime::type_schema::lookup_schema_by_id_public(schema_id)
                        .and_then(|s| s.field_names().position(|n| n == key));
                if field_idx.is_none() {
                    field_idx = super::super::control::with_trampoline_vm(|vm| {
                        vm.program()
                            .type_schema_registry
                            .get_by_id(schema_id)
                            .and_then(|s| s.field_names().position(|n| n == key))
                    })
                    .flatten();
                }
                return match field_idx.and_then(|idx| (*ptr).slots().get(idx)) {
                    Some(slot) => slot.raw(),
                    None => TAG_NULL,
                };
            }
        }

        // Per ADR-006 §2.7.5, the JIT-FFI carries raw `u64` plus a parallel
        // `NativeKind` companion stamped at JIT compile time from the call
        // signature. Pre-strict-typing the property-access fast path tried to
        // discriminate VM-format `Arc<HeapValue>` slots from JIT-format
        // `JitAlloc` allocations by reading the deleted `tag_bits::TAG_HEAP`
        // discriminator and falling back through `ValueWord::clone_from_bits`
        // — both removed in the Phase-2 bulldozer. The strict-typed
        // replacement is a kinded property-access entry that takes the
        // receiver's `NativeKind` from the call signature; the JIT lowering
        // for `op_get_prop` must thread the kind through alongside the bits.
        // TODO(phase-2c §2.7.5/§2.7.10): revive the kinded entry once the
        // op_get_prop JIT lowering passes a `NativeKind` companion.

        // JIT-allocated heap objects (JitAlloc with kind header)
        if tracing::enabled!(target: "shape_jit", tracing::Level::DEBUG) {
            tracing::debug!(
                target: "shape_jit",
                obj = obj_bits,
                heap_kind = ?heap_kind(obj_bits),
                key = ?key_str,
                "jit-get-prop",
            );
        }
        let kind = heap_kind(obj_bits);
        match kind {
            Some(kind) => match kind {
                HK_ARRAY | HK_FLOAT_ARRAY | HK_INT_ARRAY | HK_FLOAT_ARRAY_SLICE | HK_BOOL_ARRAY
                | HK_I8_ARRAY | HK_I16_ARRAY | HK_I32_ARRAY | HK_U8_ARRAY | HK_U16_ARRAY
                | HK_U32_ARRAY | HK_U64_ARRAY | HK_F32_ARRAY => {
                    // SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4):
                    // length / first / last / index decoded the
                    // deleted `JitArray` heap layout. Kinded rebuild
                    // reads `Arc<TypedArrayData>` per-element-kind
                    // (§2.7.6/Q8) and dispatches on the JIT-stamped
                    // element kind (§2.7.5). Until then, all array
                    // property reads surface.
                    let _ = key_str;
                    todo!(
                        "phase-2c §2.7.4 / W10 jit-playbook §5: \
                         JitArray rebuild — jit_get_prop array arm. \
                         Receiver decode + length/first/last/index \
                         all block on the kinded TypedArray<T> \
                         rebuild per ADR-006 §2.7.6/Q8."
                    )
                }
                HK_JIT_OBJECT => {
                    let obj = unified_unbox::<HashMap<String, u64>>(obj_bits);
                    match key_str {
                        Some(key) => obj.get(key).copied().unwrap_or(TAG_NULL),
                        None => TAG_NULL,
                    }
                }
                HK_STRING => {
                    let s = unbox_string(obj_bits);

                    // Handle string properties
                    match key_str {
                        Some("length") | Some("len") => box_number(s.chars().count() as f64),
                        _ => {
                            // Try numeric index for char access (with negative index support)
                            if is_number(key_bits) {
                                let idx_f64 = unbox_number(key_bits);
                                let char_count = s.chars().count();
                                let idx = if idx_f64 < 0.0 {
                                    let len = char_count as i64;
                                    let neg_idx = idx_f64 as i64;
                                    let actual_idx = len + neg_idx;
                                    if actual_idx < 0 {
                                        return TAG_NULL;
                                    }
                                    actual_idx as usize
                                } else {
                                    idx_f64 as usize
                                };
                                if let Some(c) = s.chars().nth(idx) {
                                    box_string(c.to_string())
                                } else {
                                    TAG_NULL
                                }
                            } else {
                                TAG_NULL
                            }
                        }
                    }
                }
                HK_DURATION => {
                    let dur = unified_unbox::<JITDuration>(obj_bits);
                    match key_str {
                        Some("value") => box_number(dur.value),
                        Some("unit") => box_number(dur.unit as f64),
                        _ => TAG_NULL,
                    }
                }
                HK_TIME => {
                    // Time is stored as i64 timestamp in JitAlloc
                    use chrono::{Datelike, TimeZone, Timelike, Utc};
                    let timestamp = *unified_unbox::<i64>(obj_bits);

                    match key_str {
                        Some("timestamp") | Some("unix") | Some("ts") => {
                            box_number(timestamp as f64)
                        }
                        Some("ms") | Some("timestamp_millis") => {
                            box_number((timestamp * 1000) as f64)
                        }
                        Some("year") | Some("month") | Some("day") | Some("hour")
                        | Some("minute") | Some("second") | Some("weekday") => {
                            if let chrono::LocalResult::Single(dt) = Utc.timestamp_opt(timestamp, 0)
                            {
                                match key_str {
                                    Some("year") => box_number(dt.year() as f64),
                                    Some("month") => box_number(dt.month() as f64),
                                    Some("day") => box_number(dt.day() as f64),
                                    Some("hour") => box_number(dt.hour() as f64),
                                    Some("minute") => box_number(dt.minute() as f64),
                                    Some("second") => box_number(dt.second() as f64),
                                    Some("weekday") => {
                                        box_number(dt.weekday().num_days_from_monday() as f64)
                                    }
                                    _ => TAG_NULL,
                                }
                            } else {
                                TAG_NULL
                            }
                        }
                        _ => TAG_NULL,
                    }
                }
                HK_DATA_REFERENCE => {
                    let data_ref = unified_unbox::<JITDataReference>(obj_bits);
                    match key_str {
                        Some("datetime") | Some("time") | Some("timestamp") => {
                            unified_box(HK_TIME, data_ref.timestamp)
                        }
                        Some("symbol") => {
                            if !data_ref.symbol.is_null() {
                                let symbol = (*data_ref.symbol).clone();
                                box_string(symbol)
                            } else {
                                TAG_NULL
                            }
                        }
                        Some("timeframe") => {
                            let tf_value = data_ref.timeframe_value;
                            let tf_unit = data_ref.timeframe_unit;

                            let unit = match tf_unit {
                                0 => crate::ast::TimeframeUnit::Second,
                                1 => crate::ast::TimeframeUnit::Minute,
                                2 => crate::ast::TimeframeUnit::Hour,
                                3 => crate::ast::TimeframeUnit::Day,
                                4 => crate::ast::TimeframeUnit::Week,
                                5 => crate::ast::TimeframeUnit::Month,
                                _ => crate::ast::TimeframeUnit::Minute,
                            };
                            let tf = crate::ast::Timeframe::new(tf_value, unit);
                            unified_box(HK_TIMEFRAME, tf)
                        }
                        Some("timezone") => {
                            if data_ref.has_timezone && !data_ref.timezone.is_null() {
                                let tz = (*data_ref.timezone).clone();
                                box_string(tz)
                            } else {
                                box_str("UTC")
                            }
                        }
                        _ => TAG_NULL,
                    }
                }
                // NOTE (Wave-7 jit-typed-pointer-migration Phase B): the
                // `HK_TYPED_OBJECT` NaN-box arm was DELETED. The migrated
                // producer emits the v2-raw `*mut TypedObjectStorage` bare-
                // pointer carrier, resolved at the top of this function; no
                // live JIT path produces a NaN-boxed `box_typed_object`
                // carrier, so this `heap_kind` match is never reached for a
                // TypedObject. Keeping the old arm would have left an inline-
                // cell `(*ptr).get_field(idx * 8)` read on the deleted JIT-
                // internal `TypedObject` layout — a wrong-offset landmine.
                unsupported => unsupported_legacy_heap_kind("jit_get_prop", Some(unsupported)),
            },
            None => unsupported_legacy_heap_kind("jit_get_prop", None),
        }
    }
}

// ============================================================================
// Shape-Guarded HashMap Access
// ============================================================================

/// Extract the shape_id from a NaN-boxed HashMap value.
///
/// Returns the shape_id as a u32 (0 if the HashMap has no shape / dictionary mode).
/// Called by JIT shape guards to compare against an expected shape.
///
/// # Phase-2c surface
///
/// Per ADR-006 §2.7.5 the kinded JIT-FFI entry takes the receiver's
/// `NativeKind` companion from the JIT call signature; pre-strict-typing
/// this routed through `ValueWord::as_hashmap_data()` (deleted with the
/// dynamic word). Re-fill is gated on `HashMapData` surfacing through the
/// kinded carrier per ADR-006 §2.7.10 dispatch shell — see
/// `docs/cluster-audits/wave-10-jit-playbook.md` §5.
///
/// # Safety
/// `obj_bits` must be a NaN-boxed value with HeapKind::HashMap; the kind
/// companion is statically known from the JIT call signature.
#[inline(always)]
pub extern "C" fn jit_hashmap_shape_id(_obj_bits: u64) -> u32 {
    // Phase-2c §2.7.5/§2.7.10: kinded HashMap-shape lookup via the
    // typed-Arc HashMapData reachable from the §2.7.6/Q8 KindedSlot
    // carrier. Returning 0 falls back to "no shape" — JIT shape guards
    // then take the cold dictionary path, which itself surfaces in its
    // own kinded re-fill wave.
    0
}

/// Access a HashMap value by slot index (O(1) indexed access).
///
/// Precondition: the caller has verified via a shape guard that the HashMap
/// has the expected shape and `slot_index` is valid for that shape.
///
/// Returns the NaN-boxed value at `values[slot_index]`, or TAG_NULL if
/// the index is out of bounds.
///
/// # Phase-2c surface
///
/// Same Phase-2c surface as `jit_hashmap_shape_id`; the re-encoding of
/// per-slot values back to JIT-bits depends on `nanboxed_to_jit_bits`'s
/// kinded re-fill (`conversion.rs` Phase-2c surface).
///
/// # Safety
/// `obj_bits` must be a NaN-boxed value with HeapKind::HashMap.
#[inline(always)]
pub extern "C" fn jit_hashmap_value_at(_obj_bits: u64, _slot_index: u64) -> u64 {
    // Phase-2c §2.7.5/§2.7.10: kinded slot read via the typed-Arc
    // HashMapData reachable from the §2.7.6/Q8 KindedSlot carrier. The
    // shape-guard precondition becomes a kind-dispatch precondition at
    // the kinded entry-point (HashMap kind companion stamped at JIT
    // compile time).
    TAG_NULL
}

/// Get array/string/object/series length
#[inline(always)]
pub extern "C" fn jit_length(value_bits: u64) -> u64 {
    let kind = heap_kind(value_bits);
    let len = match kind {
        Some(HK_ARRAY)
        | Some(HK_FLOAT_ARRAY)
        | Some(HK_INT_ARRAY)
        | Some(HK_FLOAT_ARRAY_SLICE)
        | Some(HK_BOOL_ARRAY)
        | Some(HK_I8_ARRAY)
        | Some(HK_I16_ARRAY)
        | Some(HK_I32_ARRAY)
        | Some(HK_U8_ARRAY)
        | Some(HK_U16_ARRAY)
        | Some(HK_U32_ARRAY)
        | Some(HK_U64_ARRAY)
        | Some(HK_F32_ARRAY) => {
            // SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): array
            // length read decoded the deleted JitArray layout. Kinded
            // rebuild reads `Arc<TypedArrayData>::len` per §2.7.6/Q8.
            todo!(
                "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray \
                 rebuild — jit_length array arm."
            )
        }
        Some(HK_STRING) => {
            let s = unsafe { unbox_string(value_bits) };
            s.chars().count()
        }
        Some(HK_JIT_OBJECT) => {
            let obj = unsafe { unified_unbox::<HashMap<String, u64>>(value_bits) };
            obj.len()
        }
        Some(HK_COLUMN_REF) => 0,
        other => unsupported_legacy_heap_kind("jit_length", other),
    };
    box_number(len as f64)
}

#[cfg(test)]
mod v2_typed_object_prop_tests {
    //! Wave-7 jit-typed-pointer-migration Phase B: `jit_get_prop`'s TypedObject
    //! consumer now reads the canonical v2-raw `*mut TypedObjectStorage` bare-
    //! pointer carrier (identified by `HeapHeader.kind == HEAP_KIND_V2_TYPED_
    //! OBJECT` at offset 4, resolved via `slots()`), and the old NaN-boxed
    //! `HK_TYPED_OBJECT` inline-cell arm is deleted. This test proves the
    //! migrated consumer is SAFE on a real v2 carrier: the v2 bare-pointer branch
    //! is taken (so the carrier is NOT misread as the deleted inline-cell layout,
    //! and does NOT fall through to the legacy `heap_kind` → `None` surface-and-
    //! stop panic).

    use crate::ffi::object::property_access::jit_get_prop;
    use crate::ffi::typed_object::jit_typed_object_alloc;
    use crate::ffi::value_ffi::{TAG_NULL, box_string};
    use shape_runtime::type_schema::{FieldType, SyncRegistryScope, TypeSchemaRegistry};
    use shape_value::v2::heap_header::HEAP_KIND_V2_TYPED_OBJECT;
    use std::sync::Arc;

    #[test]
    fn jit_get_prop_on_v2_typed_object_is_safe() {
        let mut reg = TypeSchemaRegistry::new_with_stdlib();
        let schema_id = reg.register_type("PhaseBProp", vec![("x".to_string(), FieldType::F64)]);
        let _scope = SyncRegistryScope::enter(Arc::new(reg));

        let bits = jit_typed_object_alloc(schema_id as u32, 8);
        assert_ne!(bits, TAG_NULL, "producer resolved schema + allocated");

        // The carrier self-describes as a v2 TypedObject at header offset 4 —
        // the exact discriminator `jit_get_prop`'s v2 branch keys on.
        unsafe {
            let hdr_kind = *((bits as *const u8).add(4) as *const u16);
            assert_eq!(hdr_kind, HEAP_KIND_V2_TYPED_OBJECT);
        }

        // Dynamic property read on the v2 carrier. `jit_get_prop`'s v2 branch is
        // taken (proven by the absence of the legacy `heap_kind`→`None` panic on
        // this bare pointer). It returns TAG_NULL here because the receiver-
        // agnostic key-classification (`is_heap_kind` on the raw `box_string`
        // key) is a SEPARATE upstream legacy surface that these strict-typed
        // raw carriers do not satisfy — out of Phase B scope. The load-bearing
        // guarantee is that the migrated consumer neither misreads the v2 carrier
        // as an inline-cell TypedObject nor surface-panics on it.
        let key = box_string("x".to_string());
        let got = jit_get_prop(bits, key);
        assert_eq!(got, TAG_NULL);

        crate::ffi::v2::jit_v2_typed_object_release(bits as *const u8);
    }
}
