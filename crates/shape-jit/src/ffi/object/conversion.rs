// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 14 sites
//     nanboxed_to_jit_bits: jit_box(HK_STRING/HK_ARRAY/HK_TIME/HK_DURATION/
//       HK_TIMEFRAME/HK_TASK_GROUP/HK_FUTURE), box_ok/box_err/box_some
//   Category B (intermediate/consumed): 3 sites
//     jit_bits_to_nanboxed: Arc::new in ValueWord::from_string/from_array (returned
//       as runtime values, not JIT heap objects)
//   Category C (heap islands): 1 site (nanboxed_to_jit_bits Array conversion)
//!
//! Conversion Between NaN-Boxed Bits and Runtime Values / ValueWord
//!
//! Functions for bidirectional conversion between the JIT's NaN-boxed u64
//! representation and the runtime's value types (ValueWord and Value).

use std::sync::Arc;

use super::super::super::context::JITDuration;
use super::super::super::jit_array::{ArrayElementKind, JitArray};
use super::super::super::nan_boxing::*;

/// JIT-side representation of a TaskGroup for heap boxing.
#[derive(Clone)]
pub struct JitTaskGroup {
    pub kind: u8,
    pub task_ids: Vec<u64>,
}

/// Bridge a width-specific typed array (`Vec<T>`) to a JitArray.
///
/// NaN-boxes each element as f64, sets typed_data to the raw buffer
/// pointer, and tags with the appropriate element kind.
fn typed_array_to_jit<T: Copy + CastToF64>(
    data: &[T],
    hk: u16,
    kind: ArrayElementKind,
) -> u64 {
    let boxed_arr: Vec<u64> = data.iter().map(|&v| box_number(v.cast_f64())).collect();
    let mut jit_arr = JitArray::from_vec(boxed_arr);
    jit_arr.typed_data = data.as_ptr() as *mut u64;
    jit_arr.element_kind = kind.as_byte();
    jit_arr.typed_storage_kind = kind.as_byte();
    jit_arr.kind = hk;
    jit_arr.heap_box()
}

/// Reconstruct a width-specific typed array from a JitArray's NaN-boxed elements.
fn jit_to_typed_array<T, F>(bits: u64, from_fn: F) -> shape_value::ValueWord
where
    T: Default + Copy,
    f64: IntoTyped<T>,
    F: FnOnce(Arc<shape_value::typed_buffer::TypedBuffer<T>>) -> shape_value::ValueWord,
{
    let arr = unsafe { JitArray::from_heap_bits(bits) };
    let data: Vec<T> = arr
        .iter()
        .map(|&b| {
            if is_number(b) {
                <f64 as IntoTyped<T>>::into_typed(unbox_number(b))
            } else {
                T::default()
            }
        })
        .collect();
    let buf = shape_value::typed_buffer::TypedBuffer {
        data,
        validity: None,
    };
    from_fn(Arc::new(buf))
}

/// Helper trait for T → f64 conversion (entry path).
trait CastToF64 {
    fn cast_f64(self) -> f64;
}
impl CastToF64 for i8 { fn cast_f64(self) -> f64 { self as f64 } }
impl CastToF64 for i16 { fn cast_f64(self) -> f64 { self as f64 } }
impl CastToF64 for i32 { fn cast_f64(self) -> f64 { self as f64 } }
impl CastToF64 for i64 { fn cast_f64(self) -> f64 { self as f64 } }
impl CastToF64 for u8 { fn cast_f64(self) -> f64 { self as f64 } }
impl CastToF64 for u16 { fn cast_f64(self) -> f64 { self as f64 } }
impl CastToF64 for u32 { fn cast_f64(self) -> f64 { self as f64 } }
impl CastToF64 for u64 { fn cast_f64(self) -> f64 { self as f64 } }
impl CastToF64 for f32 { fn cast_f64(self) -> f64 { self as f64 } }
impl CastToF64 for f64 { fn cast_f64(self) -> f64 { self } }

/// Helper trait for f64 → typed element conversion (exit path).
trait IntoTyped<T> {
    fn into_typed(self) -> T;
}
impl IntoTyped<i8> for f64 { fn into_typed(self) -> i8 { self as i8 } }
impl IntoTyped<i16> for f64 { fn into_typed(self) -> i16 { self as i16 } }
impl IntoTyped<i32> for f64 { fn into_typed(self) -> i32 { self as i32 } }
impl IntoTyped<i64> for f64 { fn into_typed(self) -> i64 { self as i64 } }
impl IntoTyped<u8> for f64 { fn into_typed(self) -> u8 { self as u8 } }
impl IntoTyped<u16> for f64 { fn into_typed(self) -> u16 { self as u16 } }
impl IntoTyped<u32> for f64 { fn into_typed(self) -> u32 { self as u32 } }
impl IntoTyped<u64> for f64 { fn into_typed(self) -> u64 { self as u64 } }
impl IntoTyped<f32> for f64 { fn into_typed(self) -> f32 { self as f32 } }
impl IntoTyped<f64> for f64 { fn into_typed(self) -> f64 { self } }

// ============================================================================
// Direct ValueWord <-> JIT Bits Conversion
// ============================================================================

/// Convert JIT NaN-boxed bits directly to ValueWord (no intermediate Value/VMValue).
pub fn jit_bits_to_nanboxed(bits: u64) -> shape_value::ValueWord {
    use shape_value::ValueWord;

    if is_number(bits) {
        return ValueWord::from_f64(unbox_number(bits));
    }
    if bits == TAG_NULL {
        return ValueWord::none();
    }
    if bits == TAG_BOOL_TRUE {
        return ValueWord::from_bool(true);
    }
    if bits == TAG_BOOL_FALSE {
        return ValueWord::from_bool(false);
    }
    if bits == TAG_UNIT {
        return ValueWord::unit();
    }
    if is_inline_function(bits) {
        let func_id = unbox_function_id(bits);
        return ValueWord::from_function_ref(format!("__func_{}", func_id), None);
    }
    if is_ok_tag(bits) {
        let inner_bits = unsafe { unbox_result_inner(bits) };
        let inner = jit_bits_to_nanboxed(inner_bits);
        return ValueWord::from_ok(inner);
    }
    if is_err_tag(bits) {
        let inner_bits = unsafe { unbox_result_inner(bits) };
        let inner = jit_bits_to_nanboxed(inner_bits);
        return ValueWord::from_err(inner);
    }
    if is_some_tag(bits) {
        let inner_bits = unsafe { unbox_some_inner(bits) };
        let inner = jit_bits_to_nanboxed(inner_bits);
        return ValueWord::from_some(inner);
    }

    match heap_kind(bits) {
        Some(HK_STRING) => {
            let s = unsafe { jit_unbox::<String>(bits) };
            ValueWord::from_string(Arc::new(s.clone()))
        }
        Some(HK_ARRAY) => {
            let arr = unsafe { JitArray::from_heap_bits(bits) };
            let values: Vec<ValueWord> = arr.iter().map(|&b| jit_bits_to_nanboxed(b)).collect();
            ValueWord::from_array(Arc::new(values))
        }
        Some(HK_CLOSURE) => {
            let closure = unsafe { jit_unbox::<super::super::super::context::JITClosure>(bits) };
            ValueWord::from_function_ref(format!("__func_{}", closure.function_id), None)
        }
        Some(HK_TASK_GROUP) => {
            let tg = unsafe { jit_unbox::<JitTaskGroup>(bits) };
            ValueWord::from_task_group(tg.kind, tg.task_ids.clone())
        }
        Some(HK_FUTURE) => {
            let id = unsafe { jit_unbox::<u64>(bits) };
            ValueWord::from_future(*id)
        }
        Some(HK_FLOAT_ARRAY) => {
            // Reconstruct FloatArray from JitArray's NaN-boxed element buffer.
            let arr = unsafe { JitArray::from_heap_bits(bits) };
            let floats: Vec<f64> = arr
                .iter()
                .map(|&b| {
                    if is_number(b) {
                        unbox_number(b)
                    } else {
                        0.0
                    }
                })
                .collect();
            let aligned = shape_value::aligned_vec::AlignedVec::from_vec(floats);
            let buf = shape_value::typed_buffer::AlignedTypedBuffer::from_aligned(aligned);
            ValueWord::from_float_array(Arc::new(buf))
        }
        Some(HK_INT_ARRAY) => {
            // Reconstruct IntArray from JitArray's NaN-boxed element buffer.
            let arr = unsafe { JitArray::from_heap_bits(bits) };
            let ints: Vec<i64> = arr
                .iter()
                .map(|&b| {
                    if is_number(b) {
                        unbox_number(b) as i64
                    } else {
                        0
                    }
                })
                .collect();
            let buf = shape_value::typed_buffer::TypedBuffer { data: ints, validity: None };
            ValueWord::from_int_array(Arc::new(buf))
        }
        Some(HK_FLOAT_ARRAY_SLICE) => {
            // Reconstruct FloatArraySlice with original parent Arc linkage.
            let arr = unsafe { JitArray::from_heap_bits(bits) };
            if arr.slice_parent_arc.is_null() {
                // Fallback: parent was lost, materialize as owned FloatArray.
                let floats: Vec<f64> = arr
                    .iter()
                    .map(|&b| if is_number(b) { unbox_number(b) } else { 0.0 })
                    .collect();
                let aligned = shape_value::aligned_vec::AlignedVec::from_vec(floats);
                let buf =
                    shape_value::typed_buffer::AlignedTypedBuffer::from_aligned(aligned);
                ValueWord::from_float_array(Arc::new(buf))
            } else {
                // Reconstitute the Arc without dropping it — the JitArray's Drop
                // will handle the Arc::from_raw when the JitArray is freed.
                let parent = unsafe {
                    Arc::from_raw(
                        arr.slice_parent_arc
                            as *const shape_value::heap_value::MatrixData,
                    )
                };
                // Clone to get our own reference, then leak the original back
                // so the JitArray Drop doesn't double-free.
                let parent_clone = Arc::clone(&parent);
                std::mem::forget(parent);
                ValueWord::from_float_array_slice(
                    parent_clone,
                    arr.slice_offset,
                    arr.slice_len,
                )
            }
        }
        Some(HK_MATRIX) => {
            // Reconstruct Matrix with original Arc<MatrixData>.
            let jm = unsafe {
                jit_unbox::<crate::jit_matrix::JitMatrix>(bits)
            };
            let mat_arc = jm.to_arc();
            ValueWord::from_matrix(mat_arc)
        }
        // Width-specific typed arrays
        Some(HK_BOOL_ARRAY) => jit_to_typed_array::<u8, _>(bits, ValueWord::from_bool_array),
        Some(HK_I8_ARRAY) => jit_to_typed_array::<i8, _>(bits, ValueWord::from_i8_array),
        Some(HK_I16_ARRAY) => jit_to_typed_array::<i16, _>(bits, ValueWord::from_i16_array),
        Some(HK_I32_ARRAY) => jit_to_typed_array::<i32, _>(bits, ValueWord::from_i32_array),
        Some(HK_U8_ARRAY) => jit_to_typed_array::<u8, _>(bits, ValueWord::from_u8_array),
        Some(HK_U16_ARRAY) => jit_to_typed_array::<u16, _>(bits, ValueWord::from_u16_array),
        Some(HK_U32_ARRAY) => jit_to_typed_array::<u32, _>(bits, ValueWord::from_u32_array),
        Some(HK_U64_ARRAY) => jit_to_typed_array::<u64, _>(bits, ValueWord::from_u64_array),
        Some(HK_F32_ARRAY) => jit_to_typed_array::<f32, _>(bits, ValueWord::from_f32_array),
        _ => ValueWord::none(),
    }
}

/// Convert JIT NaN-boxed bits to ValueWord with JITContext for function name lookup.
pub fn jit_bits_to_nanboxed_with_ctx(
    bits: u64,
    ctx: *const super::super::super::context::JITContext,
) -> shape_value::ValueWord {
    use shape_value::ValueWord;

    if is_number(bits) {
        return ValueWord::from_f64(unbox_number(bits));
    }

    // Handle inline function refs and closures with name lookup
    if is_inline_function(bits) {
        let func_id = unbox_function_id(bits);
        let name = lookup_function_name(ctx, func_id);
        return ValueWord::from_function_ref(name, None);
    }

    if is_heap_kind(bits, HK_CLOSURE) {
        let closure = unsafe { jit_unbox::<super::super::super::context::JITClosure>(bits) };
        let name = lookup_function_name(ctx, closure.function_id);
        return ValueWord::from_function_ref(name, None);
    }

    if is_heap_kind(bits, HK_ARRAY) {
        let arr = unsafe { JitArray::from_heap_bits(bits) };
        let values: Vec<ValueWord> = arr
            .iter()
            .map(|&b| jit_bits_to_nanboxed_with_ctx(b, ctx))
            .collect();
        return ValueWord::from_array(Arc::new(values));
    }

    // For other types, delegate to the basic converter
    jit_bits_to_nanboxed(bits)
}

/// Helper: look up function name from JITContext
fn lookup_function_name(
    ctx: *const super::super::super::context::JITContext,
    func_id: u16,
) -> String {
    if !ctx.is_null() {
        unsafe {
            let ctx_ref = &*ctx;
            if !ctx_ref.function_names_ptr.is_null()
                && (func_id as usize) < ctx_ref.function_names_len
            {
                return (*ctx_ref.function_names_ptr.add(func_id as usize)).clone();
            }
        }
    }
    format!("__func_{}", func_id)
}

// ============================================================================
// TypedScalar <-> JIT Bits Conversion
// ============================================================================

/// Convert JIT NaN-boxed bits to a TypedScalar with an optional type hint.
///
/// When `hint` is provided (e.g., from a FrameDescriptor's last slot), integer-hinted
/// numbers are decoded as `ScalarKind::I64` instead of `ScalarKind::F64`, preserving
/// type identity across the boundary.
pub fn jit_bits_to_typed_scalar(
    bits: u64,
    hint: Option<shape_vm::SlotKind>,
) -> shape_value::TypedScalar {
    use shape_value::TypedScalar;
    use shape_vm::SlotKind;

    if is_number(bits) {
        let f = unbox_number(bits);
        // Check if the hint says this should be an integer
        if let Some(h) = hint {
            match h {
                SlotKind::Int8 | SlotKind::NullableInt8 => {
                    return TypedScalar::i8(f as i8);
                }
                SlotKind::UInt8 | SlotKind::NullableUInt8 => {
                    return TypedScalar::u8(f as u8);
                }
                SlotKind::Int16 | SlotKind::NullableInt16 => {
                    return TypedScalar::i16(f as i16);
                }
                SlotKind::UInt16 | SlotKind::NullableUInt16 => {
                    return TypedScalar::u16(f as u16);
                }
                SlotKind::Int32 | SlotKind::NullableInt32 => {
                    return TypedScalar::i32(f as i32);
                }
                SlotKind::UInt32 | SlotKind::NullableUInt32 => {
                    return TypedScalar::u32(f as u32);
                }
                SlotKind::Int64 | SlotKind::NullableInt64 => {
                    return TypedScalar::i64(f as i64);
                }
                SlotKind::UInt64 | SlotKind::NullableUInt64 => {
                    return TypedScalar::u64(f as u64);
                }
                SlotKind::Float64 | SlotKind::NullableFloat64 => {
                    return TypedScalar::f64_from_bits(bits);
                }
                _ => {
                    // Bool, String, Boxed, etc. — no special numeric treatment
                }
            }
        }
        // Default: treat as f64
        return TypedScalar::f64_from_bits(bits);
    }

    if bits == TAG_BOOL_TRUE {
        return TypedScalar::bool(true);
    }
    if bits == TAG_BOOL_FALSE {
        return TypedScalar::bool(false);
    }
    if bits == TAG_NULL || bits == TAG_NONE {
        return TypedScalar::none();
    }
    if bits == TAG_UNIT {
        return TypedScalar::unit();
    }

    // Non-scalar (heap pointer, function, etc.) — return None sentinel
    TypedScalar::none()
}

/// Convert a TypedScalar to JIT NaN-boxed bits.
///
/// Integer kinds are stored as `box_number(value as f64)` since the JIT's
/// Cranelift IR uses f64 for all numeric operations internally.
pub fn typed_scalar_to_jit_bits(ts: &shape_value::TypedScalar) -> u64 {
    use shape_value::ScalarKind;

    match ts.kind {
        ScalarKind::I8 | ScalarKind::I16 | ScalarKind::I32 | ScalarKind::I64 => {
            box_number(ts.payload_lo as i64 as f64)
        }
        ScalarKind::U8 | ScalarKind::U16 | ScalarKind::U32 | ScalarKind::U64 => {
            box_number(ts.payload_lo as f64)
        }
        ScalarKind::I128 | ScalarKind::U128 => box_number(ts.payload_lo as i64 as f64),
        ScalarKind::F64 | ScalarKind::F32 => ts.payload_lo, // already f64 bits
        ScalarKind::Bool => {
            if ts.payload_lo != 0 {
                TAG_BOOL_TRUE
            } else {
                TAG_BOOL_FALSE
            }
        }
        ScalarKind::None => TAG_NULL,
        ScalarKind::Unit => TAG_UNIT,
    }
}

/// Convert a ValueWord value directly to JIT NaN-boxed bits (no intermediate VMValue).
pub fn nanboxed_to_jit_bits(nb: &shape_value::ValueWord) -> u64 {
    use shape_value::NanTag;
    use shape_value::heap_value::HeapValue;

    match nb.tag() {
        NanTag::F64 => box_number(unsafe { nb.as_f64_unchecked() }),
        NanTag::I48 => box_number(unsafe { nb.as_i64_unchecked() } as f64),
        NanTag::Bool => {
            if unsafe { nb.as_bool_unchecked() } {
                TAG_BOOL_TRUE
            } else {
                TAG_BOOL_FALSE
            }
        }
        NanTag::None => TAG_NULL,
        NanTag::Unit => TAG_UNIT,
        NanTag::Function => {
            let func_id = unsafe { nb.as_function_unchecked() };
            box_function(func_id)
        }
        NanTag::ModuleFunction => TAG_NULL,
        NanTag::Ref => TAG_NULL,
        NanTag::Heap => match nb.as_heap_ref() {
            Some(HeapValue::String(s)) => jit_box(HK_STRING, s.clone()),
            Some(HeapValue::Array(arr)) => {
                // AUDIT(C4): heap island — each element converted via nanboxed_to_jit_bits
                // may itself call jit_box (for strings, nested arrays, etc.), producing
                // JitAlloc pointers stored as raw u64 in the Vec. These inner allocations
                // escape into the outer JitArray element buffer without GC tracking.
                // When GC feature enabled, route through gc_allocator.
                let boxed_arr: Vec<u64> = arr.iter().map(|v| nanboxed_to_jit_bits(v)).collect();
                JitArray::from_vec(boxed_arr).heap_box()
            }
            Some(HeapValue::Time(dt)) => jit_box(HK_TIME, dt.timestamp()),
            Some(HeapValue::Duration(dur)) => {
                let unit_code = match dur.unit {
                    crate::ast::DurationUnit::Seconds => 0,
                    crate::ast::DurationUnit::Minutes => 1,
                    crate::ast::DurationUnit::Hours => 2,
                    crate::ast::DurationUnit::Days => 3,
                    crate::ast::DurationUnit::Weeks => 4,
                    crate::ast::DurationUnit::Months => 5,
                    crate::ast::DurationUnit::Years => 6,
                    crate::ast::DurationUnit::Samples => 7,
                };
                jit_box(
                    HK_DURATION,
                    JITDuration {
                        value: dur.value,
                        unit: unit_code,
                    },
                )
            }
            Some(HeapValue::Timeframe(tf)) => {
                let internal_tf = crate::ast::data::Timeframe::new(
                    tf.value,
                    match tf.unit {
                        crate::ast::TimeframeUnit::Second => {
                            crate::ast::data::TimeframeUnit::Second
                        }
                        crate::ast::TimeframeUnit::Minute => {
                            crate::ast::data::TimeframeUnit::Minute
                        }
                        crate::ast::TimeframeUnit::Hour => crate::ast::data::TimeframeUnit::Hour,
                        crate::ast::TimeframeUnit::Day => crate::ast::data::TimeframeUnit::Day,
                        crate::ast::TimeframeUnit::Week => crate::ast::data::TimeframeUnit::Week,
                        crate::ast::TimeframeUnit::Month => crate::ast::data::TimeframeUnit::Month,
                        crate::ast::TimeframeUnit::Year => crate::ast::data::TimeframeUnit::Year,
                    },
                );
                jit_box(HK_TIMEFRAME, internal_tf)
            }
            Some(HeapValue::Ok(inner)) => {
                let inner_bits = nanboxed_to_jit_bits(inner);
                box_ok(inner_bits)
            }
            Some(HeapValue::Err(inner)) => {
                let inner_bits = nanboxed_to_jit_bits(inner);
                box_err(inner_bits)
            }
            Some(HeapValue::Some(inner)) => {
                let inner_bits = nanboxed_to_jit_bits(inner);
                box_some(inner_bits)
            }
            // BigInt → f64: precision loss for |i| > 2^53
            Some(HeapValue::BigInt(i)) => box_number(*i as f64),
            Some(HeapValue::TaskGroup { kind, task_ids }) => jit_box(
                HK_TASK_GROUP,
                JitTaskGroup {
                    kind: *kind,
                    task_ids: task_ids.clone(),
                },
            ),
            Some(HeapValue::Future(id)) => jit_box(HK_FUTURE, *id),
            Some(HeapValue::FloatArray(buf)) => {
                // Bridge FloatArray → JitArray with typed_data pointing to
                // the AlignedTypedBuffer's f64 data for direct numeric access.
                let len = buf.data.len();
                let boxed_arr: Vec<u64> = buf
                    .data
                    .as_slice()
                    .iter()
                    .map(|&v| box_number(v))
                    .collect();
                let mut jit_arr = JitArray::from_vec(boxed_arr);
                // Point typed_data at the source AlignedVec's f64 buffer.
                // This is safe because the Arc keeps the buffer alive as long
                // as the HeapValue exists, and the JitArray only lives for the
                // duration of the JIT call.
                jit_arr.typed_data = buf.data.as_slice().as_ptr() as *mut u64;
                jit_arr.element_kind =
                    crate::jit_array::ArrayElementKind::Float64.as_byte();
                jit_arr.typed_storage_kind =
                    crate::jit_array::ArrayElementKind::Float64.as_byte();
                let _ = len; // suppress unused warning
                { jit_arr.kind = HK_FLOAT_ARRAY; jit_arr.heap_box() }
            }
            Some(HeapValue::IntArray(buf)) => {
                // Bridge IntArray → JitArray with typed_data pointing to
                // the TypedBuffer<i64>'s data for direct integer access.
                let boxed_arr: Vec<u64> = buf
                    .data
                    .iter()
                    .map(|&v| box_number(v as f64))
                    .collect();
                let mut jit_arr = JitArray::from_vec(boxed_arr);
                jit_arr.typed_data = buf.data.as_ptr() as *mut u64;
                jit_arr.element_kind =
                    crate::jit_array::ArrayElementKind::Int64.as_byte();
                jit_arr.typed_storage_kind =
                    crate::jit_array::ArrayElementKind::Int64.as_byte();
                { jit_arr.kind = HK_INT_ARRAY; jit_arr.heap_box() }
            }
            Some(HeapValue::FloatArraySlice {
                parent,
                offset,
                len,
            }) => {
                // Bridge FloatArraySlice → JitArray with typed_data pointing
                // to the parent MatrixData's AlignedVec at the given offset.
                // Preserves parent Arc linkage for clean round-trip on deopt.
                let off = *offset as usize;
                let slice_len = *len as usize;
                let parent_slice = parent.data.as_slice();
                let end = (off + slice_len).min(parent_slice.len());
                let actual_slice = &parent_slice[off..end];
                let boxed_arr: Vec<u64> = actual_slice
                    .iter()
                    .map(|&v| box_number(v))
                    .collect();
                let mut jit_arr = JitArray::from_vec(boxed_arr);
                // Point typed_data at the parent's data + offset for zero-copy reads.
                if !parent_slice.is_empty() && off < parent_slice.len() {
                    jit_arr.typed_data =
                        unsafe { parent_slice.as_ptr().add(off) } as *mut u64;
                }
                jit_arr.element_kind =
                    crate::jit_array::ArrayElementKind::Float64.as_byte();
                jit_arr.typed_storage_kind =
                    crate::jit_array::ArrayElementKind::Float64.as_byte();
                // Stash parent Arc for round-trip reconstruction.
                // Arc::into_raw increments the strong count; the JitArray Drop
                // impl calls Arc::from_raw to release it.
                jit_arr.slice_parent_arc =
                    Arc::into_raw(Arc::clone(parent)) as *const ();
                jit_arr.slice_offset = *offset;
                jit_arr.slice_len = *len;
                { jit_arr.kind = HK_FLOAT_ARRAY_SLICE; jit_arr.heap_box() }
            }
            Some(HeapValue::Matrix(mat_arc)) => {
                // Bridge Matrix → JitMatrix with direct f64 data pointer.
                let jm = crate::jit_matrix::JitMatrix::from_arc(mat_arc);
                jit_box(HK_MATRIX, jm)
            }
            // Width-specific typed arrays
            Some(HeapValue::BoolArray(buf)) => typed_array_to_jit(&buf.data, HK_BOOL_ARRAY, ArrayElementKind::Bool),
            Some(HeapValue::I8Array(buf)) => typed_array_to_jit(&buf.data, HK_I8_ARRAY, ArrayElementKind::I8),
            Some(HeapValue::I16Array(buf)) => typed_array_to_jit(&buf.data, HK_I16_ARRAY, ArrayElementKind::I16),
            Some(HeapValue::I32Array(buf)) => typed_array_to_jit(&buf.data, HK_I32_ARRAY, ArrayElementKind::I32),
            Some(HeapValue::U8Array(buf)) => typed_array_to_jit(&buf.data, HK_U8_ARRAY, ArrayElementKind::U8),
            Some(HeapValue::U16Array(buf)) => typed_array_to_jit(&buf.data, HK_U16_ARRAY, ArrayElementKind::U16),
            Some(HeapValue::U32Array(buf)) => typed_array_to_jit(&buf.data, HK_U32_ARRAY, ArrayElementKind::U32),
            Some(HeapValue::U64Array(buf)) => typed_array_to_jit(&buf.data, HK_U64_ARRAY, ArrayElementKind::U64),
            Some(HeapValue::F32Array(buf)) => typed_array_to_jit(&buf.data, HK_F32_ARRAY, ArrayElementKind::F32),
            _ => TAG_NULL,
        },
    }
}
