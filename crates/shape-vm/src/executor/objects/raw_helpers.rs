//! Raw u64 extraction helpers for v2 method handlers.
//!
//! These functions extract typed values directly from raw u64 bits WITHOUT
//! constructing ValueWord. They use the NaN-boxing tag layout from shape_value::tags.
//!
//! Safety contract: callers must know the type of the value (via receiver_type_tag
//! from the opcode, or via HeapKind from the dispatch cascade). Passing bits of
//! the wrong type is undefined behavior for heap pointer extraction.

use shape_value::heap_value::{HashMapData, HeapValue};
use shape_value::slot::ValueSlot;
use shape_value::tags::{get_payload, get_tag, is_tagged, sign_extend_i48, TAG_HEAP, TAG_INT};
use shape_value::{ArrayView, Upvalue, VMError, ValueWord, ValueWordExt};

// ─── Inline scalar extraction ─────────────────────────────────────────────

/// Extract f64 from raw bits. Assumes the value is an untagged f64.
#[inline(always)]
pub fn extract_f64(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// Extract i64 from raw NaN-boxed i48 bits. Assumes TAG_INT.
#[inline(always)]
pub fn extract_i48(bits: u64) -> i64 {
    sign_extend_i48(get_payload(bits))
}

/// Extract a number as f64, coercing from int if needed.
/// Returns None if the bits are not a number or int.
#[inline]
pub fn extract_number_coerce(bits: u64) -> Option<f64> {
    if !is_tagged(bits) {
        Some(f64::from_bits(bits))
    } else if get_tag(bits) == TAG_INT {
        Some(sign_extend_i48(get_payload(bits)) as f64)
    } else {
        None
    }
}

/// Extract bool from raw bits. Assumes the value is a tagged bool.
#[inline(always)]
pub fn extract_bool(bits: u64) -> bool {
    get_payload(bits) != 0
}

// ─── Heap pointer extraction ──────────────────────────────────────────────

/// Extract a raw const pointer to the HeapValue from tagged heap bits.
/// Returns None if not heap-tagged.
#[inline(always)]
pub fn extract_heap_ptr(bits: u64) -> Option<*const HeapValue> {
    if is_tagged(bits) && get_tag(bits) == TAG_HEAP {
        let ptr = get_payload(bits) as *const HeapValue;
        if !ptr.is_null() {
            return Some(ptr);
        }
    }
    None
}

/// Extract a &HeapValue reference from heap-tagged bits.
/// SAFETY: The pointer must be valid for the duration of the returned reference.
/// This is safe when called on stack/arg bits that haven't been dropped.
#[inline(always)]
pub unsafe fn extract_heap_ref(bits: u64) -> Option<&'static HeapValue> {
    extract_heap_ptr(bits).map(|ptr| unsafe { &*ptr })
}

/// Extract a &str from heap-tagged string bits.
/// Returns None if not a heap string.
#[inline]
pub fn extract_str(bits: u64) -> Option<&'static str> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::String(s) => Some(s.as_str()),
            _ => None,
        })
    }
}

/// Extract &DateTime<FixedOffset> from heap-tagged DateTime bits.
#[inline]
pub fn extract_datetime(bits: u64) -> Option<&'static chrono::DateTime<chrono::FixedOffset>> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Time(dt) => Some(dt),
            _ => None,
        })
    }
}

/// Extract &std::time::Instant from heap-tagged Instant bits.
#[inline]
pub fn extract_instant(bits: u64) -> Option<&'static std::time::Instant> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Instant(inst) => Some(&**inst),
            _ => None,
        })
    }
}

/// Extract char from heap-tagged Char bits.
/// Returns None if not a heap Char.
#[inline]
pub fn extract_char(bits: u64) -> Option<char> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Char(c) => Some(*c),
            _ => None,
        })
    }
}

// ─── HashMap extraction ──────────────────────────────────────────────────

/// Extract &HashMapData from heap-tagged HashMap bits.
/// Returns None if not a heap HashMap.
#[inline]
pub fn extract_hashmap_data(bits: u64) -> Option<&'static HashMapData> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::HashMap(d) => Some(d.as_ref()),
            _ => None,
        })
    }
}

/// Extract (keys, values, index) tuple from heap-tagged HashMap bits.
/// Returns None if not a heap HashMap.
#[inline]
pub fn extract_hashmap(
    bits: u64,
) -> Option<(
    &'static Vec<ValueWord>,
    &'static Vec<ValueWord>,
    &'static std::collections::HashMap<u64, Vec<usize>>,
)> {
    extract_hashmap_data(bits).map(|d| (&d.keys, &d.values, &d.index))
}

/// Extract ArrayView from heap-tagged array bits.
/// Returns None if not a heap array variant.
#[inline]
pub fn extract_any_array(bits: u64) -> Option<ArrayView<'static>> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Array(a) => Some(ArrayView::Generic(a)),
            HeapValue::IntArray(a) => Some(ArrayView::Int(a)),
            HeapValue::FloatArray(a) => Some(ArrayView::Float(a)),
            HeapValue::BoolArray(a) => Some(ArrayView::Bool(a)),
            HeapValue::I8Array(a) => Some(ArrayView::I8(a)),
            HeapValue::I16Array(a) => Some(ArrayView::I16(a)),
            HeapValue::I32Array(a) => Some(ArrayView::I32(a)),
            HeapValue::U8Array(a) => Some(ArrayView::U8(a)),
            HeapValue::U16Array(a) => Some(ArrayView::U16(a)),
            _ => None,
        })
    }
}

// ─── Collection extraction helpers ───────────────────────────────────────

/// Extract &SetData from heap-tagged bits.
/// Returns None if not a heap Set.
#[inline]
pub fn extract_set(bits: u64) -> Option<&'static shape_value::heap_value::SetData> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Set(d) => Some(d.as_ref()),
            _ => None,
        })
    }
}

/// Extract &DequeData from heap-tagged bits.
/// Returns None if not a heap Deque.
#[inline]
pub fn extract_deque(bits: u64) -> Option<&'static shape_value::heap_value::DequeData> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Deque(d) => Some(d.as_ref()),
            _ => None,
        })
    }
}

/// Extract &PriorityQueueData from heap-tagged bits.
/// Returns None if not a heap PriorityQueue.
#[inline]
pub fn extract_priority_queue(
    bits: u64,
) -> Option<&'static shape_value::heap_value::PriorityQueueData> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::PriorityQueue(d) => Some(d.as_ref()),
            _ => None,
        })
    }
}

/// Extract &MatrixData from heap-tagged bits.
/// Returns None if not a heap Matrix.
#[inline]
pub fn extract_matrix(bits: u64) -> Option<&'static shape_value::heap_value::MatrixData> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Matrix(arc) => Some(arc.as_ref()),
            _ => None,
        })
    }
}

/// Extract Arc<MatrixData> clone from heap-tagged bits.
/// Returns None if not a heap Matrix. Used for FloatArraySlice parent.
#[inline]
pub fn extract_matrix_arc(
    bits: u64,
) -> Option<std::sync::Arc<shape_value::heap_value::MatrixData>> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Matrix(arc) => Some(arc.clone()),
            _ => None,
        })
    }
}

/// Extract &ContentNode from heap-tagged bits.
/// Returns None if not a heap Content.
#[inline]
pub fn extract_content(bits: u64) -> Option<&'static shape_value::content::ContentNode> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Content(node) => Some(node.as_ref()),
            _ => None,
        })
    }
}

// ─── SharedCell / Future / TaskGroup extraction ─────────────────────────

/// Extract &Arc<RwLock<ValueWord>> from heap-tagged SharedCell bits.
#[inline]
pub fn extract_shared_cell(
    bits: u64,
) -> Option<&'static std::sync::Arc<std::sync::RwLock<ValueWord>>> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::SharedCell(arc) => Some(arc),
            _ => None,
        })
    }
}

/// Extract future ID from heap-tagged Future bits.
#[inline]
pub fn extract_future_id(bits: u64) -> Option<u64> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Future(id) => Some(*id),
            _ => None,
        })
    }
}

/// Extract (kind, &task_ids) from heap-tagged TaskGroup bits.
#[inline]
pub fn extract_task_group(bits: u64) -> Option<(u8, &'static Vec<u64>)> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::TaskGroup { kind, task_ids } => Some((*kind, task_ids)),
            _ => None,
        })
    }
}

// ─── BigInt / Decimal extraction ────────────────────────────────────────

/// Extract i64 from heap-tagged BigInt bits.
#[inline]
pub fn extract_big_int(bits: u64) -> Option<i64> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::BigInt(v) => Some(*v),
            _ => None,
        })
    }
}

/// Extract Decimal from heap-tagged Decimal bits.
#[inline]
pub fn extract_decimal(bits: u64) -> Option<rust_decimal::Decimal> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Decimal(d) => Some(*d),
            _ => None,
        })
    }
}

// ─── TypedObject / TypeAnnotatedValue extraction ────────────────────────

/// Extract (schema_id, &slots, heap_mask) from heap-tagged TypedObject bits.
#[inline]
pub fn extract_typed_object(bits: u64) -> Option<(u64, &'static [ValueSlot], u64)> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::TypedObject {
                schema_id,
                slots,
                heap_mask,
            } => Some((*schema_id, slots.as_ref(), *heap_mask)),
            _ => None,
        })
    }
}

/// Unwrap TypeAnnotatedValue to its inner bits. Returns the input unchanged
/// if it's not a TypeAnnotatedValue.
#[inline]
pub fn unwrap_annotated_bits(bits: u64) -> u64 {
    unsafe {
        if let Some(HeapValue::TypeAnnotatedValue { value, .. }) = extract_heap_ref(bits) {
            value.into_raw_bits()
        } else {
            bits
        }
    }
}

// ─── Option / Result extraction ─────────────────────────────────────────

/// Extract inner &ValueWord from heap-tagged Some bits.
#[inline]
pub fn extract_some_inner(bits: u64) -> Option<&'static ValueWord> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Some(inner) => Some(inner.as_ref()),
            _ => None,
        })
    }
}

/// Extract inner &ValueWord from heap-tagged Ok bits.
#[inline]
pub fn extract_ok_inner(bits: u64) -> Option<&'static ValueWord> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Ok(inner) => Some(inner.as_ref()),
            _ => None,
        })
    }
}

/// Extract inner &ValueWord from heap-tagged Err bits.
#[inline]
pub fn extract_err_inner(bits: u64) -> Option<&'static ValueWord> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Err(inner) => Some(inner.as_ref()),
            _ => None,
        })
    }
}

// ─── Enum extraction ────────────────────────────────────────────────────

/// Extract &EnumValue from heap-tagged Enum bits.
#[inline]
pub fn extract_enum(bits: u64) -> Option<&'static shape_value::enums::EnumValue> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Enum(e) => Some(e.as_ref()),
            _ => None,
        })
    }
}

// ─── Range extraction ───────────────────────────────────────────────────

/// Extract (start, end, inclusive) from heap-tagged Range bits.
#[inline]
pub fn extract_range(
    bits: u64,
) -> Option<(
    Option<&'static ValueWord>,
    Option<&'static ValueWord>,
    bool,
)> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Range {
                start,
                end,
                inclusive,
            } => Some((
                start.as_ref().map(|b| b.as_ref()),
                end.as_ref().map(|b| b.as_ref()),
                *inclusive,
            )),
            _ => None,
        })
    }
}

// ─── FilterExpr / TraitObject extraction ────────────────────────────────

/// Extract &Arc<FilterNode> from heap-tagged FilterExpr bits.
#[inline]
pub fn extract_filter_expr(
    bits: u64,
) -> Option<&'static std::sync::Arc<shape_value::value::FilterNode>> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::FilterExpr(f) => Some(f),
            _ => None,
        })
    }
}

/// Extract (&ValueWord, &Arc<VTable>) from heap-tagged TraitObject bits.
#[inline]
pub fn extract_trait_object(
    bits: u64,
) -> Option<(
    &'static ValueWord,
    &'static std::sync::Arc<shape_value::value::VTable>,
)> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::TraitObject { value, vtable } => Some((value.as_ref(), vtable)),
            _ => None,
        })
    }
}

// ─── IoHandle extraction ────────────────────────────────────────────────

/// Extract &IoHandleData from heap-tagged IoHandle bits.
#[inline]
pub fn extract_io_handle(bits: u64) -> Option<&'static shape_value::heap_value::IoHandleData> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::IoHandle(h) => Some(h.as_ref()),
            _ => None,
        })
    }
}

// ─── DataTable extraction ───────────────────────────────────────────────

/// Extract &Arc<DataTable> from heap-tagged DataTable bits.
#[inline]
pub fn extract_datatable(
    bits: u64,
) -> Option<&'static std::sync::Arc<shape_value::datatable::DataTable>> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::DataTable(dt) => Some(dt),
            _ => None,
        })
    }
}

// ─── Error helpers ────────────────────────────────────────────────────────

/// Get the type name string for error messages, without constructing ValueWord.
#[inline]
pub fn type_name_from_bits(bits: u64) -> &'static str {
    if !is_tagged(bits) {
        return "number";
    }
    let tag = get_tag(bits);
    if tag == TAG_INT {
        return "int";
    }
    if tag == TAG_HEAP {
        if let Some(hv) = unsafe { extract_heap_ref(bits) } {
            return hv.type_name();
        }
    }
    if tag == shape_value::tags::TAG_BOOL {
        return "bool";
    }
    if tag == shape_value::tags::TAG_NONE {
        return "null";
    }
    if tag == shape_value::tags::TAG_UNIT {
        return "unit";
    }
    if tag == shape_value::tags::TAG_FUNCTION || tag == shape_value::tags::TAG_MODULE_FN {
        return "function";
    }
    "unknown"
}

/// Create a TypeError with expected/got from raw bits.
#[inline]
pub fn type_error(expected: &'static str, bits: u64) -> VMError {
    VMError::TypeError {
        expected,
        got: type_name_from_bits(bits),
    }
}

// ─── Callable inspection ────────────────────────────────────────────────

/// Check if raw u64 bits represent a callable value (function, module function,
/// closure, or host closure).
#[inline]
pub fn is_callable_raw(bits: u64) -> bool {
    use shape_value::tags::{TAG_FUNCTION, TAG_MODULE_FN};

    if !is_tagged(bits) {
        return false;
    }

    let tag = get_tag(bits);
    match tag {
        TAG_FUNCTION | TAG_MODULE_FN => true,
        TAG_HEAP => unsafe {
            extract_heap_ref(bits)
                .map(|hv| matches!(hv, HeapValue::Closure { .. } | HeapValue::HostClosure(..)))
                .unwrap_or(false)
        },
        _ => false,
    }
}

/// Check truthiness of raw u64 bits without taking ownership.
#[inline]
pub fn is_truthy_raw(bits: u64) -> bool {
    let vw = std::mem::ManuallyDrop::new(ValueWord::from_raw_bits(bits));
    vw.is_truthy()
}

// ─── Clone / ownership helpers ───────────────────────────────────────────

/// Clone raw u64 bits, bumping Arc refcount for heap-tagged values.
///
/// For inline values the bits are simply copied. For heap-tagged values,
/// the underlying Arc refcount is incremented.
#[inline(always)]
pub fn clone_raw_bits(bits: u64) -> u64 {
    if is_tagged(bits) && get_tag(bits) == TAG_HEAP {
        let ptr = get_payload(bits) as *const HeapValue;
        if !ptr.is_null() {
            unsafe { std::sync::Arc::increment_strong_count(ptr) };
        }
    }
    bits
}

// ─── Closure / callable inspection from raw bits ─────────────────────────

/// Extract closure info (function_id, upvalues) from raw heap-tagged bits.
/// Returns borrowed references — no Vec clone.
#[inline]
pub fn extract_closure_info(bits: u64) -> Option<(u16, &'static [shape_value::Upvalue])> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::Closure {
                function_id,
                upvalues,
            } => Some((*function_id, upvalues.as_slice())),
            _ => None,
        })
    }
}

/// Check if raw bits represent a HostClosure, and if so call it with raw args.
/// Returns None if the bits are not a HostClosure.
#[inline]
pub fn try_call_host_closure(bits: u64, args: &[u64]) -> Option<Result<u64, VMError>> {
    unsafe {
        extract_heap_ref(bits).and_then(|hv| match hv {
            HeapValue::HostClosure(callable) => {
                let args_vec: Vec<ValueWord> = args
                    .iter()
                    .map(|&b| {
                        let tmp = std::mem::ManuallyDrop::new(ValueWord::from_raw_bits(b));
                        (*tmp).clone()
                    })
                    .collect();
                let result = callable
                    .call(&args_vec)
                    .map(|vw| vw.into_raw_bits())
                    .map_err(VMError::RuntimeError);
                Some(result)
            }
            _ => None,
        })
    }
}

/// Get the arity of a callable from raw u64 bits.
#[inline]
pub fn callable_arity_raw(
    program: &crate::bytecode::BytecodeProgram,
    callee_bits: u64,
) -> Option<u16> {
    use shape_value::tags::{TAG_FUNCTION, TAG_MODULE_FN};

    if !is_tagged(callee_bits) {
        return None;
    }

    let tag = get_tag(callee_bits);
    match tag {
        TAG_FUNCTION => {
            let func_id = get_payload(callee_bits) as u16;
            program.functions.get(func_id as usize).map(|f| f.arity)
        }
        TAG_HEAP => {
            extract_closure_info(callee_bits)
                .and_then(|(fid, _)| program.functions.get(fid as usize).map(|f| f.arity))
        }
        TAG_MODULE_FN => None,
        _ => None,
    }
}
