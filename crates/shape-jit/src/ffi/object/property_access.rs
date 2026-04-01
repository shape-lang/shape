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
use super::super::super::jit_array::JitArray;
use super::super::super::nan_boxing::*;

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

        // Detect VM-allocated heap objects vs JIT-allocated (JitAlloc).
        // Both have TAG_HEAP (tag 0). JitAlloc has a valid kind u16 in the
        // known range. VM Arc<HeapValue> has the enum discriminant which can
        // collide with low JIT kinds. Reliable check: use is_unified_heap for
        // unified arrays, then check if heap_kind matches a known JIT type.
        // If the kind is NOT a recognized JIT type, try VM interpretation.
        if shape_value::tags::is_tagged(obj_bits)
            && shape_value::tags::get_tag(obj_bits) == shape_value::tags::TAG_HEAP
        {
            let jit_kind = heap_kind(obj_bits);
            let is_known_jit = matches!(
                jit_kind,
                Some(HK_ARRAY)
                    | Some(HK_STRING)
                    | Some(HK_TYPED_OBJECT)
                    | Some(HK_JIT_OBJECT)
                    | Some(HK_DURATION)
                    | Some(HK_TIME)
                    | Some(HK_DATA_REFERENCE)
                    | Some(HK_CLOSURE)
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
                    | Some(HK_F32_ARRAY)
            );
            if !is_known_jit {
                // Not a recognized JIT type — try VM ValueWord interpretation.
                // SAFETY: clone_from_bits is only safe for actual VM ValueWord bits.
                // If this is a JitAlloc with an unrecognized kind, clone_from_bits
                // would crash. But since we checked is_known_jit first, unrecognized
                // JIT kinds are extremely unlikely.
                let vw = shape_value::ValueWord::clone_from_bits(obj_bits);
                if let Some((schema_id, slots, heap_mask)) = vw.as_typed_object() {
                    if let Some(key) = key_str {
                        if let Some(schema) =
                            shape_runtime::type_schema::lookup_schema_by_id_public(
                                schema_id as u32,
                            )
                        {
                            if let Some(idx) = schema.field_names().position(|n| n == key) {
                                if idx < slots.len() {
                                    let is_heap_field = (heap_mask >> idx) & 1 != 0;
                                    let field_vw = slots[idx].as_value_word(is_heap_field);
                                    return super::conversion::nanboxed_to_jit_bits(&field_vw);
                                }
                            }
                        }
                    }
                    return TAG_NULL;
                }
                // Not a VM TypedObject either — fall through
            }
        }

        // JIT-allocated heap objects (JitAlloc with kind header)
        if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
            eprintln!(
                "[jit-get-prop] obj={:#x} heap_kind={:?} key={:?}",
                obj_bits, heap_kind(obj_bits), key_str
            );
        }
        if let Some(kind) = heap_kind(obj_bits) {
            match kind {
                HK_ARRAY | HK_FLOAT_ARRAY | HK_INT_ARRAY | HK_FLOAT_ARRAY_SLICE
                | HK_BOOL_ARRAY | HK_I8_ARRAY | HK_I16_ARRAY | HK_I32_ARRAY
                | HK_U8_ARRAY | HK_U16_ARRAY | HK_U32_ARRAY | HK_U64_ARRAY
                | HK_F32_ARRAY => {
                    let arr = JitArray::from_heap_bits(obj_bits);

                    // Handle array properties
                    match key_str {
                        Some("length") | Some("len") => box_number(arr.len() as f64),
                        Some("first") => arr.first().copied().unwrap_or(TAG_NULL),
                        Some("last") => arr.last().copied().unwrap_or(TAG_NULL),
                        _ => {
                            // Try numeric index (with negative index support)
                            if is_number(key_bits) {
                                let idx_f64 = unbox_number(key_bits);
                                let idx = if idx_f64 < 0.0 {
                                    let len = arr.len() as i64;
                                    let neg_idx = idx_f64 as i64;
                                    let actual_idx = len + neg_idx;
                                    if actual_idx < 0 {
                                        return TAG_NULL;
                                    }
                                    actual_idx as usize
                                } else {
                                    idx_f64 as usize
                                };
                                arr.get(idx).copied().unwrap_or(TAG_NULL)
                            } else {
                                TAG_NULL
                            }
                        }
                    }
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
                HK_TYPED_OBJECT => {
                    // JIT-allocated TypedObject — resolve field by name via schema.
                    // Check both the global stdlib registry AND the trampoline VM's
                    // bytecode schema registry (for user-defined types).
                    let ptr =
                        unbox_typed_object(obj_bits) as *const super::super::typed_object::TypedObject;
                    if !ptr.is_null() {
                        if let Some(key) = key_str {
                            let schema_id = (*ptr).schema_id;
                            // Try global registry first
                            let mut field_idx = shape_runtime::type_schema::lookup_schema_by_id_public(schema_id)
                                .and_then(|s| s.field_names().position(|n| n == key));
                            // Fall back to trampoline VM's bytecode registry
                            if field_idx.is_none() {
                                field_idx = super::super::control::with_trampoline_vm(|vm| {
                                    vm.program().type_schema_registry
                                        .get_by_id(schema_id)
                                        .and_then(|s| s.field_names().position(|n| n == key))
                                }).flatten();
                            }
                            if let Some(idx) = field_idx {
                                return (*ptr).get_field(idx * 8);
                            }
                        }
                    }
                    TAG_NULL
                }
                _ => TAG_NULL,
            }
        } else {
            TAG_NULL
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
/// # Safety
/// `obj_bits` must be a NaN-boxed value with HeapKind::HashMap.
#[inline(always)]
pub extern "C" fn jit_hashmap_shape_id(obj_bits: u64) -> u32 {
    use super::conversion::jit_bits_to_nanboxed;
    let vw = jit_bits_to_nanboxed(obj_bits);
    if let Some(data) = vw.as_hashmap_data() {
        data.shape_id.map(|s| s.0).unwrap_or(0)
    } else {
        0
    }
}

/// Access a HashMap value by slot index (O(1) indexed access).
///
/// Precondition: the caller has verified via a shape guard that the HashMap
/// has the expected shape and `slot_index` is valid for that shape.
///
/// Returns the NaN-boxed value at `values[slot_index]`, or TAG_NULL if
/// the index is out of bounds (defensive fallback).
///
/// # Safety
/// `obj_bits` must be a NaN-boxed value with HeapKind::HashMap.
#[inline(always)]
pub extern "C" fn jit_hashmap_value_at(obj_bits: u64, slot_index: u64) -> u64 {
    use super::conversion::{jit_bits_to_nanboxed, nanboxed_to_jit_bits};
    let vw = jit_bits_to_nanboxed(obj_bits);
    if let Some(data) = vw.as_hashmap_data() {
        let idx = slot_index as usize;
        if let Some(val) = data.values.get(idx) {
            nanboxed_to_jit_bits(val)
        } else {
            TAG_NULL
        }
    } else {
        TAG_NULL
    }
}

/// Get array/string/object/series length
#[inline(always)]
pub extern "C" fn jit_length(value_bits: u64) -> u64 {
    let len = match heap_kind(value_bits) {
        Some(HK_ARRAY) | Some(HK_FLOAT_ARRAY) | Some(HK_INT_ARRAY)
        | Some(HK_FLOAT_ARRAY_SLICE) | Some(HK_BOOL_ARRAY)
        | Some(HK_I8_ARRAY) | Some(HK_I16_ARRAY) | Some(HK_I32_ARRAY)
        | Some(HK_U8_ARRAY) | Some(HK_U16_ARRAY) | Some(HK_U32_ARRAY)
        | Some(HK_U64_ARRAY) | Some(HK_F32_ARRAY) => {
            let arr = unsafe { JitArray::from_heap_bits(value_bits) };
            arr.len()
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
        _ => {
            // VM-format heap values (HashMap, DataTable, etc.)
            if is_heap(value_bits) && !shape_value::tags::is_unified_heap(value_bits) {
                let vw = unsafe { shape_value::ValueWord::clone_from_bits(value_bits) };
                if let Some(hm) = vw.as_hashmap_data() {
                    return box_number(hm.keys.len() as f64);
                }
                if let Some(s) = vw.as_str() {
                    return box_number(s.chars().count() as f64);
                }
            }
            0
        }
    };
    box_number(len as f64)
}
