//! ValueWord: 8-byte NaN-boxed value representation for the VM stack.
//!
//! Uses IEEE 754 quiet NaN space to pack type tags and payloads into 8 bytes.
//! Simple types (f64, i48, bool, None, Unit, Function) are stored inline.
//! Complex types are heap-allocated as `Arc<HeapValue>` with the raw pointer in the payload.
//!
//! ## NaN-boxing scheme
//!
//! All tagged values use sign bit = 1 with a quiet NaN exponent, giving us 51 bits
//! for tag + payload. Normal f64 values (including NaN, which is canonicalized to a
//! positive quiet NaN) are stored directly and never collide with our tagged range.
//!
//! ```text
//! Tagged: 0xFFF[C-F]_XXXX_XXXX_XXXX
//!   Bit 63    = 1 (sign, marks as tagged)
//!   Bits 62-52 = 0x7FF (NaN exponent)
//!   Bit 51    = 1 (quiet NaN bit)
//!   Bits 50-48 = tag (3 bits)
//!   Bits 47-0  = payload (48 bits)
//! ```
//!
//! | Tag   | Meaning                                      |
//! |-------|----------------------------------------------|
//! | 0b000 | Heap pointer to `Arc<HeapValue>` (48-bit ptr) |
//! | 0b001 | i48 (48-bit signed integer, sign-extended)   |
//! | 0b010 | Bool (payload bit 0)                         |
//! | 0b011 | None                                         |
//! | 0b100 | Unit                                         |
//! | 0b101 | Function(u16) (payload = function_id)        |
//! | 0b110 | ModuleFunction(u32) (payload = index)        |
//! | 0b111 | Ref (absolute stack slot index, 48 bits)     |

use crate::content::ContentNode;
use crate::datatable::DataTable;
use crate::enums::EnumValue;
use crate::heap_value::{
    ChannelData, DequeData, HashMapData, HeapValue, NativeScalar, NativeTypeLayout, NativeViewData,
    PriorityQueueData, ProjectedRefData, RefProjection, SetData,
};
use crate::slot::ValueSlot;
use crate::value::{FilterNode, HostCallable, PrintResult, VMArray, VTable};
use chrono::{DateTime, FixedOffset, Utc};
use shape_ast::ast::{DataDateTimeRef, DateTimeExpr, Duration, TimeReference, TypeAnnotation};
use shape_ast::data::Timeframe;
use std::collections::HashMap;
use std::sync::Arc;

const REF_TARGET_MODULE_FLAG: u64 = 1 << 47;
const REF_TARGET_INDEX_MASK: u64 = REF_TARGET_MODULE_FLAG - 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefTarget {
    Stack(usize),
    ModuleBinding(usize),
    Projected(ProjectedRefData),
}

// --- Bit layout constants (imported from shared tags module) ---
use crate::tags::{
    CANONICAL_NAN, I48_MAX, I48_MIN, PAYLOAD_MASK, TAG_BOOL, TAG_FUNCTION, TAG_HEAP, TAG_INT,
    TAG_MODULE_FN, TAG_NONE, TAG_REF, TAG_UNIT, get_payload, get_tag, is_tagged, make_tagged,
    sign_extend_i48,
};

/// Single source of truth for tag variants and their inline type dispatch.
///
/// Generates:
/// - `nan_tag_type_name(tag)` — type name string for an inline (non-F64, non-Heap) tag
/// - `nan_tag_is_truthy(tag, payload)` — truthiness for an inline (non-F64, non-Heap) tag
///
/// F64 is handled before the tag match (via `!is_tagged()`), and Heap delegates
/// to HeapValue. Both are kept out of the inline dispatch.
/// Map a raw tag constant to its type name string.
#[inline]
pub fn nan_tag_type_name(tag: u64) -> &'static str {
    match tag {
        TAG_INT => "int",
        TAG_BOOL => "bool",
        TAG_NONE => "option",
        TAG_UNIT => "unit",
        TAG_FUNCTION => "function",
        TAG_MODULE_FN => "module_function",
        TAG_REF => "reference",
        _ => "unknown",
    }
}


/// Evaluate truthiness for an inline tag value.
#[inline]
pub fn nan_tag_is_truthy(tag: u64, payload: u64) -> bool {
    match tag {
        TAG_INT => sign_extend_i48(payload) != 0,
        TAG_BOOL => payload != 0,
        TAG_NONE => false,
        TAG_UNIT => false,
        TAG_FUNCTION | TAG_MODULE_FN | TAG_REF => true,
        _ => true,
    }
}

// ArrayView and ArrayViewMut are in crate::array_view
pub use crate::array_view::{ArrayView, ArrayViewMut};

/// An 8-byte value word for the VM stack (NaN-boxed encoding).
/// Type alias for u64.
pub type ValueWord = u64;

/// Wrapper for Display/Debug formatting of ValueWord values.
pub struct ValueWordDisplay(pub u64);

/// Heap-box a HeapValue (non-GC).
#[inline] #[cfg(not(feature = "gc"))]
pub(crate) fn vw_heap_box(v: HeapValue) -> ValueWord {
    let arc = Arc::new(v);
    let ptr = Arc::into_raw(arc) as u64;
    debug_assert!(ptr & !PAYLOAD_MASK == 0, "pointer exceeds 48 bits");
    make_tagged(TAG_HEAP, ptr & PAYLOAD_MASK)
}
#[inline] #[cfg(feature = "gc")]
pub(crate) fn vw_heap_box(v: HeapValue) -> ValueWord {
    let heap = shape_gc::thread_gc_heap();
    let ptr = heap.alloc(v) as u64;
    debug_assert!(ptr & !PAYLOAD_MASK == 0, "GC pointer exceeds 48 bits");
    make_tagged(TAG_HEAP, ptr & PAYLOAD_MASK)
}

/// Extension trait providing methods on ValueWord (u64).
pub trait ValueWordExt {
    fn from_f64(v: f64) -> ValueWord;
    fn from_i64(v: i64) -> ValueWord;
    fn from_native_scalar(value: NativeScalar) -> ValueWord;
    fn from_native_i8(v: i8) -> ValueWord;
    fn from_native_u8(v: u8) -> ValueWord;
    fn from_native_i16(v: i16) -> ValueWord;
    fn from_native_u16(v: u16) -> ValueWord;
    fn from_native_i32(v: i32) -> ValueWord;
    fn from_native_u32(v: u32) -> ValueWord;
    fn from_native_u64(v: u64) -> ValueWord;
    fn from_native_isize(v: isize) -> ValueWord;
    fn from_native_usize(v: usize) -> ValueWord;
    fn from_native_ptr(v: usize) -> ValueWord;
    fn from_native_f32(v: f32) -> ValueWord;
    fn from_c_view(ptr: usize, layout: Arc<NativeTypeLayout>) -> ValueWord;
    fn from_c_mut(ptr: usize, layout: Arc<NativeTypeLayout>) -> ValueWord;
    fn from_bool(v: bool) -> ValueWord;
    fn none() -> ValueWord;
    fn unit() -> ValueWord;
    fn from_function(id: u16) -> ValueWord;
    fn from_module_function(index: u32) -> ValueWord;
    fn from_ref(absolute_slot: usize) -> ValueWord;
    fn from_module_binding_ref(binding_idx: usize) -> ValueWord;
    fn from_projected_ref(base: ValueWord, projection: RefProjection) -> ValueWord;
    fn heap_box(v: HeapValue) -> ValueWord;
    fn from_string(s: Arc<String>) -> ValueWord;
    fn from_char(c: char) -> ValueWord;
    fn as_char(&self) -> Option<char>;
    fn from_array(a: crate::value::VMArray) -> ValueWord;
    fn from_decimal(d: rust_decimal::Decimal) -> ValueWord;
    fn from_heap_value(v: HeapValue) -> ValueWord;
    fn from_datatable(dt: Arc<DataTable>) -> ValueWord;
    fn from_typed_table(schema_id: u64, table: Arc<DataTable>) -> ValueWord;
    fn from_row_view(schema_id: u64, table: Arc<DataTable>, row_idx: usize) -> ValueWord;
    fn from_column_ref(schema_id: u64, table: Arc<DataTable>, col_id: u32) -> ValueWord;
    fn from_indexed_table(schema_id: u64, table: Arc<DataTable>, index_col: u32) -> ValueWord;
    fn from_range(start: Option<ValueWord>, end: Option<ValueWord>, inclusive: bool) -> ValueWord;
    fn from_enum(e: EnumValue) -> ValueWord;
    fn from_some(inner: ValueWord) -> ValueWord;
    fn from_ok(inner: ValueWord) -> ValueWord;
    fn from_err(inner: ValueWord) -> ValueWord;
    fn from_hashmap(keys: Vec<ValueWord>, values: Vec<ValueWord>, index: HashMap<u64, Vec<usize>>) -> ValueWord;
    fn empty_hashmap() -> ValueWord;
    fn from_hashmap_pairs(keys: Vec<ValueWord>, values: Vec<ValueWord>) -> ValueWord;
    fn from_set(items: Vec<ValueWord>) -> ValueWord;
    fn empty_set() -> ValueWord;
    fn from_deque(items: Vec<ValueWord>) -> ValueWord;
    fn empty_deque() -> ValueWord;
    fn from_priority_queue(items: Vec<ValueWord>) -> ValueWord;
    fn empty_priority_queue() -> ValueWord;
    fn from_content(node: ContentNode) -> ValueWord;
    fn from_int_array(a: Arc<crate::typed_buffer::TypedBuffer<i64>>) -> ValueWord;
    fn from_float_array(a: Arc<crate::typed_buffer::AlignedTypedBuffer>) -> ValueWord;
    fn from_bool_array(a: Arc<crate::typed_buffer::TypedBuffer<u8>>) -> ValueWord;
    fn from_i8_array(a: Arc<crate::typed_buffer::TypedBuffer<i8>>) -> ValueWord;
    fn from_i16_array(a: Arc<crate::typed_buffer::TypedBuffer<i16>>) -> ValueWord;
    fn from_i32_array(a: Arc<crate::typed_buffer::TypedBuffer<i32>>) -> ValueWord;
    fn from_u8_array(a: Arc<crate::typed_buffer::TypedBuffer<u8>>) -> ValueWord;
    fn from_u16_array(a: Arc<crate::typed_buffer::TypedBuffer<u16>>) -> ValueWord;
    fn from_u32_array(a: Arc<crate::typed_buffer::TypedBuffer<u32>>) -> ValueWord;
    fn from_u64_array(a: Arc<crate::typed_buffer::TypedBuffer<u64>>) -> ValueWord;
    fn from_f32_array(a: Arc<crate::typed_buffer::TypedBuffer<f32>>) -> ValueWord;
    fn from_matrix(m: Arc<crate::heap_value::MatrixData>) -> ValueWord;
    fn from_float_array_slice(parent: Arc<crate::heap_value::MatrixData>, offset: u32, len: u32) -> ValueWord;
    fn from_iterator(state: Box<crate::heap_value::IteratorState>) -> ValueWord;
    fn from_generator(state: Box<crate::heap_value::GeneratorState>) -> ValueWord;
    fn from_future(id: u64) -> ValueWord;
    fn from_task_group(kind: u8, task_ids: Vec<u64>) -> ValueWord;
    fn from_mutex(value: ValueWord) -> ValueWord;
    fn from_atomic(value: i64) -> ValueWord;
    fn from_lazy(initializer: ValueWord) -> ValueWord;
    fn from_channel(data: ChannelData) -> ValueWord;
    fn from_trait_object(value: ValueWord, vtable: Arc<VTable>) -> ValueWord;
    fn from_expr_proxy(col_name: Arc<String>) -> ValueWord;
    fn from_filter_expr(node: Arc<FilterNode>) -> ValueWord;
    fn from_instant(t: std::time::Instant) -> ValueWord;
    fn from_io_handle(data: crate::heap_value::IoHandleData) -> ValueWord;
    fn from_time(t: DateTime<FixedOffset>) -> ValueWord;
    fn from_time_utc(t: DateTime<Utc>) -> ValueWord;
    fn from_duration(d: Duration) -> ValueWord;
    fn from_timespan(ts: chrono::Duration) -> ValueWord;
    fn from_timeframe(tf: Timeframe) -> ValueWord;
    fn from_host_closure(nc: HostCallable) -> ValueWord;
    fn from_print_result(pr: PrintResult) -> ValueWord;
    fn from_simulation_call(name: String, params: HashMap<String, ValueWord>) -> ValueWord;
    fn from_function_ref(name: String, closure: Option<ValueWord>) -> ValueWord;
    fn from_data_reference(datetime: DateTime<FixedOffset>, id: String, timeframe: Timeframe) -> ValueWord;
    fn from_time_reference(tr: TimeReference) -> ValueWord;
    fn from_datetime_expr(de: DateTimeExpr) -> ValueWord;
    fn from_data_datetime_ref(dr: DataDateTimeRef) -> ValueWord;
    fn from_type_annotation(ta: TypeAnnotation) -> ValueWord;
    fn from_type_annotated_value(type_name: String, value: ValueWord) -> ValueWord;
    unsafe fn clone_from_bits(bits: u64) -> ValueWord;
    fn is_f64(&self) -> bool;
    fn is_i64(&self) -> bool;
    fn is_bool(&self) -> bool;
    fn is_none(&self) -> bool;
    fn is_unit(&self) -> bool;
    fn is_function(&self) -> bool;
    fn is_heap(&self) -> bool;
    fn is_ref(&self) -> bool;
    fn as_ref_target(&self) -> Option<RefTarget>;
    fn as_ref_slot(&self) -> Option<usize>;
    fn as_f64(&self) -> Option<f64>;
    fn as_i64(&self) -> Option<i64>;
    fn as_u64_value(&self) -> Option<u64>;
    fn as_i128_exact(&self) -> Option<i128>;
    fn as_number_strict(&self) -> Option<f64>;
    fn as_bool(&self) -> Option<bool>;
    fn as_function_id(&self) -> Option<u16>;
    unsafe fn as_f64_unchecked(&self) -> f64;
    unsafe fn as_i64_unchecked(&self) -> i64;
    unsafe fn as_bool_unchecked(&self) -> bool;
    unsafe fn as_function_unchecked(&self) -> u16;
    fn as_heap_ref(&self) -> Option<&HeapValue>;
    fn as_heap_mut(&mut self) -> Option<&mut HeapValue>;
    fn is_truthy(&self) -> bool;
    fn as_number_coerce(&self) -> Option<f64>;
    fn is_module_function(&self) -> bool;
    fn as_module_function(&self) -> Option<usize>;
    fn heap_kind(&self) -> Option<crate::heap_value::HeapKind>;
    fn as_str(&self) -> Option<&str>;
    fn as_decimal(&self) -> Option<rust_decimal::Decimal>;
    unsafe fn as_decimal_unchecked(&self) -> rust_decimal::Decimal;
    fn as_array(&self) -> Option<&VMArray>;
    fn as_any_array(&self) -> Option<ArrayView<'_>>;
    fn as_any_array_mut(&mut self) -> Option<ArrayViewMut<'_>>;
    fn as_datatable(&self) -> Option<&Arc<DataTable>>;
    fn as_typed_table(&self) -> Option<(u64, &Arc<DataTable>)>;
    fn as_row_view(&self) -> Option<(u64, &Arc<DataTable>, usize)>;
    fn as_column_ref(&self) -> Option<(u64, &Arc<DataTable>, u32)>;
    fn as_indexed_table(&self) -> Option<(u64, &Arc<DataTable>, u32)>;
    fn as_typed_object(&self) -> Option<(u64, &[ValueSlot], u64)>;
    fn as_closure(&self) -> Option<(u16, &[crate::value::Upvalue])>;
    fn as_some_inner(&self) -> Option<&ValueWord>;
    fn as_ok_inner(&self) -> Option<&ValueWord>;
    fn as_err_inner(&self) -> Option<&ValueWord>;
    fn as_err_payload(&self) -> Option<ValueWord>;
    fn as_future(&self) -> Option<u64>;
    fn as_trait_object(&self) -> Option<(&ValueWord, &Arc<VTable>)>;
    fn as_expr_proxy(&self) -> Option<&Arc<String>>;
    fn as_filter_expr(&self) -> Option<&Arc<FilterNode>>;
    fn as_host_closure(&self) -> Option<&HostCallable>;
    fn as_duration(&self) -> Option<&shape_ast::ast::Duration>;
    fn as_range(&self) -> Option<(Option<&ValueWord>, Option<&ValueWord>, bool)>;
    fn as_timespan(&self) -> Option<chrono::Duration>;
    fn as_timeframe(&self) -> Option<&Timeframe>;
    fn as_hashmap(&self) -> Option<(&Vec<ValueWord>, &Vec<ValueWord>, &HashMap<u64, Vec<usize>>)>;
    fn as_hashmap_data(&self) -> Option<&HashMapData>;
    fn as_hashmap_mut(&mut self) -> Option<&mut HashMapData>;
    fn as_set_mut(&mut self) -> Option<&mut SetData>;
    fn as_deque_mut(&mut self) -> Option<&mut DequeData>;
    fn as_priority_queue_mut(&mut self) -> Option<&mut PriorityQueueData>;
    fn as_content(&self) -> Option<&ContentNode>;
    fn as_time(&self) -> Option<DateTime<FixedOffset>>;
    fn as_instant(&self) -> Option<&std::time::Instant>;
    fn as_io_handle(&self) -> Option<&crate::heap_value::IoHandleData>;
    fn as_native_scalar(&self) -> Option<NativeScalar>;
    fn as_native_view(&self) -> Option<&NativeViewData>;
    fn as_datetime(&self) -> Option<&DateTime<FixedOffset>>;
    fn as_arc_string(&self) -> Option<&Arc<String>>;
    fn as_int_array(&self) -> Option<&Arc<crate::typed_buffer::TypedBuffer<i64>>>;
    fn as_float_array(&self) -> Option<&Arc<crate::typed_buffer::AlignedTypedBuffer>>;
    fn as_bool_array(&self) -> Option<&Arc<crate::typed_buffer::TypedBuffer<u8>>>;
    fn as_matrix(&self) -> Option<&crate::heap_value::MatrixData>;
    fn as_iterator(&self) -> Option<&crate::heap_value::IteratorState>;
    fn as_generator(&self) -> Option<&crate::heap_value::GeneratorState>;
    fn typed_array_len(&self) -> Option<usize>;
    fn coerce_to_float_array(&self) -> Option<Arc<crate::typed_buffer::AlignedTypedBuffer>>;
    fn to_generic_array(&self) -> Option<crate::value::VMArray>;
    fn to_array_arc(&self) -> Option<crate::value::VMArray>;
    fn vw_equals(&self, other: &ValueWord) -> bool;
    fn vw_hash(&self) -> u64;
    unsafe fn add_i64(a: &ValueWord, b: &ValueWord) -> ValueWord;
    unsafe fn sub_i64(a: &ValueWord, b: &ValueWord) -> ValueWord;
    unsafe fn mul_i64(a: &ValueWord, b: &ValueWord) -> ValueWord;
    fn binary_int_preserving(a: &ValueWord, b: &ValueWord, a_num: f64, b_num: f64, int_op: impl FnOnce(i64, i64) -> Option<i64>, float_op: impl FnOnce(f64, f64) -> f64) -> ValueWord;
    fn raw_bits(&self) -> u64;
    fn from_raw_bits(bits: u64) -> ValueWord;
    fn into_raw_bits(self) -> u64;
    fn type_name(&self) -> &'static str;
    fn to_number(&self) -> Option<f64>;
    fn to_bool(&self) -> Option<bool>;
    fn as_usize(&self) -> Option<usize>;
    fn to_json_value(&self) -> serde_json::Value;
}

impl ValueWordExt for u64 {
    // ===== Constructors =====

    /// Create a ValueWord from an f64 value.
    ///
    /// Normal f64 values are stored directly. NaN values are canonicalized to a single
    /// canonical NaN to avoid collisions with our tagged range.
    #[inline]
    fn from_f64(v: f64) -> ValueWord {
        let bits = v.to_bits();
        if v.is_nan() {
            // Canonicalize all NaN variants to one known NaN outside our tagged range.
            CANONICAL_NAN
        } else if is_tagged(bits) {
            // Extremely rare: a valid non-NaN f64 whose bits happen to fall in our tagged range.
            // This cannot actually happen because all values in 0x7FFC..0x7FFF range are NaN
            // (exponent all 1s with non-zero mantissa). So this branch is dead code, but kept
            // for safety.
            CANONICAL_NAN
        } else {
            bits
        }
    }

    /// Create a ValueWord from an i64 value.
    ///
    /// Values in the range [-2^47, 2^47-1] are stored inline as i48.
    /// Values outside that range are heap-boxed as `HeapValue::BigInt`.
    #[inline]
    fn from_i64(v: i64) -> ValueWord {
        if v >= I48_MIN && v <= I48_MAX {
            // Fits in 48 bits. Store as sign-extended i48.
            // Truncate to 48 bits by masking with PAYLOAD_MASK.
            let payload = (v as u64) & PAYLOAD_MASK;
            make_tagged(TAG_INT, payload)
        } else {
            // Too large for inline. Heap-box as BigInt.
            ValueWord::heap_box(HeapValue::BigInt(v))
        }
    }

    /// Create a ValueWord from a width-aware native scalar.
    #[inline]
    fn from_native_scalar(value: NativeScalar) -> ValueWord {
        ValueWord::heap_box(HeapValue::NativeScalar(value))
    }

    #[inline]
    fn from_native_i8(v: i8) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::I8(v))
    }

    #[inline]
    fn from_native_u8(v: u8) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::U8(v))
    }

    #[inline]
    fn from_native_i16(v: i16) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::I16(v))
    }

    #[inline]
    fn from_native_u16(v: u16) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::U16(v))
    }

    #[inline]
    fn from_native_i32(v: i32) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::I32(v))
    }

    #[inline]
    fn from_native_u32(v: u32) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::U32(v))
    }

    #[inline]
    fn from_native_u64(v: u64) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::U64(v))
    }

    #[inline]
    fn from_native_isize(v: isize) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::Isize(v))
    }

    #[inline]
    fn from_native_usize(v: usize) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::Usize(v))
    }

    #[inline]
    fn from_native_ptr(v: usize) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::Ptr(v))
    }

    #[inline]
    fn from_native_f32(v: f32) -> ValueWord {
        ValueWord::from_native_scalar(NativeScalar::F32(v))
    }

    /// Create a pointer-backed C view.
    #[inline]
    fn from_c_view(ptr: usize, layout: Arc<NativeTypeLayout>) -> ValueWord {
        ValueWord::heap_box(HeapValue::NativeView(Box::new(NativeViewData {
            ptr,
            layout,
            mutable: false,
        })))
    }

    /// Create a pointer-backed mutable C view.
    #[inline]
    fn from_c_mut(ptr: usize, layout: Arc<NativeTypeLayout>) -> ValueWord {
        ValueWord::heap_box(HeapValue::NativeView(Box::new(NativeViewData {
            ptr,
            layout,
            mutable: true,
        })))
    }

    /// Create a ValueWord from a bool.
    #[inline]
    fn from_bool(v: bool) -> ValueWord {
        make_tagged(TAG_BOOL, v as u64)
    }

    /// Create a ValueWord representing None.
    #[inline]
    fn none() -> ValueWord {
        make_tagged(TAG_NONE, 0)
    }

    /// Create a ValueWord representing Unit.
    #[inline]
    fn unit() -> ValueWord {
        make_tagged(TAG_UNIT, 0)
    }

    /// Create a ValueWord from a function ID.
    #[inline]
    fn from_function(id: u16) -> ValueWord {
        make_tagged(TAG_FUNCTION, id as u64)
    }

    /// Create a ValueWord from a module function index.
    #[inline]
    fn from_module_function(index: u32) -> ValueWord {
        make_tagged(TAG_MODULE_FN, index as u64)
    }

    /// Create a ValueWord reference to an absolute stack slot.
    #[inline]
    fn from_ref(absolute_slot: usize) -> ValueWord {
        make_tagged(TAG_REF, absolute_slot as u64)
    }

    /// Create a ValueWord reference to a module binding slot.
    #[inline]
    fn from_module_binding_ref(binding_idx: usize) -> ValueWord {
        make_tagged(
            TAG_REF,
            REF_TARGET_MODULE_FLAG | (binding_idx as u64 & REF_TARGET_INDEX_MASK),
        )
    }

    /// Create a projected reference backed by heap metadata.
    #[inline]
    fn from_projected_ref(base: ValueWord, projection: RefProjection) -> ValueWord {
        ValueWord::heap_box(HeapValue::ProjectedRef(Box::new(ProjectedRefData {
            base,
            projection,
        })))
    }

    /// Heap-box a HeapValue. Delegates to free function.
    #[inline]
    fn heap_box(v: HeapValue) -> ValueWord {
        vw_heap_box(v)
    }

    // ===== Typed constructors =====

    /// Create a ValueWord from an Arc<String>.
    #[inline]
    fn from_string(s: Arc<String>) -> ValueWord {
        ValueWord::heap_box(HeapValue::String(s))
    }

    /// Create a ValueWord from a char.
    #[inline]
    fn from_char(c: char) -> ValueWord {
        ValueWord::heap_box(HeapValue::Char(c))
    }

    /// Extract a char if this is a HeapValue::Char.
    #[inline]
    fn as_char(&self) -> Option<char> {
        if let Some(HeapValue::Char(c)) = self.as_heap_ref() {
            Some(*c)
        } else {
            std::option::Option::None
        }
    }

    /// Create a ValueWord from a VMArray directly (no intermediate conversion).
    #[inline]
    fn from_array(a: crate::value::VMArray) -> ValueWord {
        ValueWord::heap_box(HeapValue::Array(a))
    }

    /// Create a ValueWord from Decimal directly (no intermediate conversion).
    #[inline]
    fn from_decimal(d: rust_decimal::Decimal) -> ValueWord {
        ValueWord::heap_box(HeapValue::Decimal(d))
    }

    /// Create a ValueWord from any HeapValue directly (no intermediate conversion).
    ///
    /// BigInt that fits i48 is unwrapped to its native ValueWord inline tag instead
    /// of being heap-allocated. All other variants are heap-boxed.
    #[inline]
    fn from_heap_value(v: HeapValue) -> ValueWord {
        match v {
            HeapValue::BigInt(i) => ValueWord::from_i64(i),
            other => ValueWord::heap_box(other),
        }
    }

    // ===== DataTable family constructors =====

    /// Create a ValueWord from a DataTable directly.
    #[inline]
    fn from_datatable(dt: Arc<DataTable>) -> ValueWord {
        ValueWord::heap_box(HeapValue::DataTable(dt))
    }

    /// Create a ValueWord TypedTable directly.
    #[inline]
    fn from_typed_table(schema_id: u64, table: Arc<DataTable>) -> ValueWord {
        ValueWord::heap_box(HeapValue::TypedTable { schema_id, table })
    }

    /// Create a ValueWord RowView directly.
    #[inline]
    fn from_row_view(schema_id: u64, table: Arc<DataTable>, row_idx: usize) -> ValueWord {
        ValueWord::heap_box(HeapValue::RowView {
            schema_id,
            table,
            row_idx,
        })
    }

    /// Create a ValueWord ColumnRef directly.
    #[inline]
    fn from_column_ref(schema_id: u64, table: Arc<DataTable>, col_id: u32) -> ValueWord {
        ValueWord::heap_box(HeapValue::ColumnRef {
            schema_id,
            table,
            col_id,
        })
    }

    /// Create a ValueWord IndexedTable directly.
    #[inline]
    fn from_indexed_table(schema_id: u64, table: Arc<DataTable>, index_col: u32) -> ValueWord {
        ValueWord::heap_box(HeapValue::IndexedTable {
            schema_id,
            table,
            index_col,
        })
    }

    // ===== Container / wrapper constructors =====

    /// Create a ValueWord Range directly.
    #[inline]
    fn from_range(start: Option<ValueWord>, end: Option<ValueWord>, inclusive: bool) -> ValueWord {
        ValueWord::heap_box(HeapValue::Range {
            start: start.map(Box::new),
            end: end.map(Box::new),
            inclusive,
        })
    }

    /// Create a ValueWord Enum directly.
    #[inline]
    fn from_enum(e: EnumValue) -> ValueWord {
        ValueWord::heap_box(HeapValue::Enum(Box::new(e)))
    }

    /// Create a ValueWord Some directly.
    #[inline]
    fn from_some(inner: ValueWord) -> ValueWord {
        ValueWord::heap_box(HeapValue::Some(Box::new(inner)))
    }

    /// Create a ValueWord Ok directly.
    #[inline]
    fn from_ok(inner: ValueWord) -> ValueWord {
        ValueWord::heap_box(HeapValue::Ok(Box::new(inner)))
    }

    /// Create a ValueWord Err directly.
    #[inline]
    fn from_err(inner: ValueWord) -> ValueWord {
        ValueWord::heap_box(HeapValue::Err(Box::new(inner)))
    }

    // ===== HashMap constructors =====

    /// Create a ValueWord HashMap from keys, values, and index.
    #[inline]
    fn from_hashmap(
        keys: Vec<ValueWord>,
        values: Vec<ValueWord>,
        index: HashMap<u64, Vec<usize>>,
    ) -> ValueWord {
        ValueWord::heap_box(HeapValue::HashMap(Box::new(HashMapData {
            keys,
            values,
            index,
            shape_id: None,
        })))
    }

    /// Create an empty ValueWord HashMap.
    #[inline]
    fn empty_hashmap() -> ValueWord {
        ValueWord::heap_box(HeapValue::HashMap(Box::new(HashMapData {
            keys: Vec::new(),
            values: Vec::new(),
            index: HashMap::new(),
            shape_id: None,
        })))
    }

    /// Create a ValueWord HashMap from keys and values, auto-building the bucket
    /// index and computing a shape for O(1) property access when all keys are strings.
    #[inline]
    fn from_hashmap_pairs(keys: Vec<ValueWord>, values: Vec<ValueWord>) -> ValueWord {
        let index = HashMapData::rebuild_index(&keys);
        let shape_id = HashMapData::compute_shape(&keys);
        ValueWord::heap_box(HeapValue::HashMap(Box::new(HashMapData {
            keys,
            values,
            index,
            shape_id,
        })))
    }

    // ===== Set constructors =====

    /// Create a ValueWord Set from items (deduplicating).
    #[inline]
    fn from_set(items: Vec<ValueWord>) -> ValueWord {
        ValueWord::heap_box(HeapValue::Set(Box::new(SetData::from_items(items))))
    }

    /// Create an empty ValueWord Set.
    #[inline]
    fn empty_set() -> ValueWord {
        ValueWord::heap_box(HeapValue::Set(Box::new(SetData {
            items: Vec::new(),
            index: HashMap::new(),
        })))
    }

    // ===== Deque constructors =====

    /// Create a ValueWord Deque from items.
    #[inline]
    fn from_deque(items: Vec<ValueWord>) -> ValueWord {
        ValueWord::heap_box(HeapValue::Deque(Box::new(DequeData::from_items(items))))
    }

    /// Create an empty ValueWord Deque.
    #[inline]
    fn empty_deque() -> ValueWord {
        ValueWord::heap_box(HeapValue::Deque(Box::new(DequeData::new())))
    }

    // ===== PriorityQueue constructors =====

    /// Create a ValueWord PriorityQueue from items (heapified).
    #[inline]
    fn from_priority_queue(items: Vec<ValueWord>) -> ValueWord {
        ValueWord::heap_box(HeapValue::PriorityQueue(Box::new(
            PriorityQueueData::from_items(items),
        )))
    }

    /// Create an empty ValueWord PriorityQueue.
    #[inline]
    fn empty_priority_queue() -> ValueWord {
        ValueWord::heap_box(HeapValue::PriorityQueue(Box::new(PriorityQueueData::new())))
    }

    // ===== Content constructors =====

    /// Create a ValueWord from a ContentNode directly.
    #[inline]
    fn from_content(node: ContentNode) -> ValueWord {
        ValueWord::heap_box(HeapValue::Content(Box::new(node)))
    }

    // ===== Typed collection constructors =====

    /// Create a ValueWord IntArray from an Arc<TypedBuffer<i64>>.
    #[inline]
    fn from_int_array(a: Arc<crate::typed_buffer::TypedBuffer<i64>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::IntArray(a))
    }

    /// Create a ValueWord FloatArray from an Arc<AlignedTypedBuffer>.
    #[inline]
    fn from_float_array(a: Arc<crate::typed_buffer::AlignedTypedBuffer>) -> ValueWord {
        ValueWord::heap_box(HeapValue::FloatArray(a))
    }

    /// Create a ValueWord BoolArray from an Arc<TypedBuffer<u8>>.
    #[inline]
    fn from_bool_array(a: Arc<crate::typed_buffer::TypedBuffer<u8>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::BoolArray(a))
    }

    /// Create a ValueWord I8Array.
    #[inline]
    fn from_i8_array(a: Arc<crate::typed_buffer::TypedBuffer<i8>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::I8Array(a))
    }

    /// Create a ValueWord I16Array.
    #[inline]
    fn from_i16_array(a: Arc<crate::typed_buffer::TypedBuffer<i16>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::I16Array(a))
    }

    /// Create a ValueWord I32Array.
    #[inline]
    fn from_i32_array(a: Arc<crate::typed_buffer::TypedBuffer<i32>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::I32Array(a))
    }

    /// Create a ValueWord U8Array.
    #[inline]
    fn from_u8_array(a: Arc<crate::typed_buffer::TypedBuffer<u8>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::U8Array(a))
    }

    /// Create a ValueWord U16Array.
    #[inline]
    fn from_u16_array(a: Arc<crate::typed_buffer::TypedBuffer<u16>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::U16Array(a))
    }

    /// Create a ValueWord U32Array.
    #[inline]
    fn from_u32_array(a: Arc<crate::typed_buffer::TypedBuffer<u32>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::U32Array(a))
    }

    /// Create a ValueWord U64Array.
    #[inline]
    fn from_u64_array(a: Arc<crate::typed_buffer::TypedBuffer<u64>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::U64Array(a))
    }

    /// Create a ValueWord F32Array.
    #[inline]
    fn from_f32_array(a: Arc<crate::typed_buffer::TypedBuffer<f32>>) -> ValueWord {
        ValueWord::heap_box(HeapValue::F32Array(a))
    }

    /// Create a ValueWord Matrix from MatrixData.
    #[inline]
    fn from_matrix(m: Arc<crate::heap_value::MatrixData>) -> ValueWord {
        ValueWord::heap_box(HeapValue::Matrix(m))
    }

    /// Create a ValueWord FloatArraySlice — a zero-copy view into a parent matrix.
    #[inline]
    fn from_float_array_slice(
        parent: Arc<crate::heap_value::MatrixData>,
        offset: u32,
        len: u32,
    ) -> ValueWord {
        ValueWord::heap_box(HeapValue::FloatArraySlice {
            parent,
            offset,
            len,
        })
    }

    /// Create a ValueWord Iterator from IteratorState.
    #[inline]
    fn from_iterator(state: Box<crate::heap_value::IteratorState>) -> ValueWord {
        ValueWord::heap_box(HeapValue::Iterator(state))
    }

    /// Create a ValueWord Generator from GeneratorState.
    #[inline]
    fn from_generator(state: Box<crate::heap_value::GeneratorState>) -> ValueWord {
        ValueWord::heap_box(HeapValue::Generator(state))
    }

    // ===== Async / concurrency constructors =====

    /// Create a ValueWord Future directly.
    #[inline]
    fn from_future(id: u64) -> ValueWord {
        ValueWord::heap_box(HeapValue::Future(id))
    }

    /// Create a ValueWord TaskGroup directly.
    #[inline]
    fn from_task_group(kind: u8, task_ids: Vec<u64>) -> ValueWord {
        ValueWord::heap_box(HeapValue::TaskGroup { kind, task_ids })
    }

    /// Create a ValueWord Mutex wrapping a value.
    #[inline]
    fn from_mutex(value: ValueWord) -> ValueWord {
        ValueWord::heap_box(HeapValue::Mutex(Box::new(
            crate::heap_value::MutexData::new(value),
        )))
    }

    /// Create a ValueWord Atomic with an initial integer value.
    #[inline]
    fn from_atomic(value: i64) -> ValueWord {
        ValueWord::heap_box(HeapValue::Atomic(Box::new(
            crate::heap_value::AtomicData::new(value),
        )))
    }

    /// Create a ValueWord Lazy with an initializer closure.
    #[inline]
    fn from_lazy(initializer: ValueWord) -> ValueWord {
        ValueWord::heap_box(HeapValue::Lazy(Box::new(crate::heap_value::LazyData::new(
            initializer,
        ))))
    }

    /// Create a ValueWord Channel endpoint.
    #[inline]
    fn from_channel(data: ChannelData) -> ValueWord {
        ValueWord::heap_box(HeapValue::Channel(Box::new(data)))
    }

    // ===== Trait dispatch constructors =====

    /// Create a ValueWord TraitObject directly.
    #[inline]
    fn from_trait_object(value: ValueWord, vtable: Arc<VTable>) -> ValueWord {
        ValueWord::heap_box(HeapValue::TraitObject {
            value: Box::new(value),
            vtable,
        })
    }

    // ===== SQL pushdown constructors =====

    /// Create a ValueWord ExprProxy directly.
    #[inline]
    fn from_expr_proxy(col_name: Arc<String>) -> ValueWord {
        ValueWord::heap_box(HeapValue::ExprProxy(col_name))
    }

    /// Create a ValueWord FilterExpr directly.
    #[inline]
    fn from_filter_expr(node: Arc<FilterNode>) -> ValueWord {
        ValueWord::heap_box(HeapValue::FilterExpr(node))
    }

    // ===== Instant constructors =====

    /// Create a ValueWord Instant directly.
    #[inline]
    fn from_instant(t: std::time::Instant) -> ValueWord {
        ValueWord::heap_box(HeapValue::Instant(Box::new(t)))
    }

    // ===== IoHandle constructors =====

    /// Create a ValueWord IoHandle.
    #[inline]
    fn from_io_handle(data: crate::heap_value::IoHandleData) -> ValueWord {
        ValueWord::heap_box(HeapValue::IoHandle(Box::new(data)))
    }

    // ===== Time constructors =====

    /// Create a ValueWord Time directly from a DateTime<FixedOffset>.
    #[inline]
    fn from_time(t: DateTime<FixedOffset>) -> ValueWord {
        ValueWord::heap_box(HeapValue::Time(t))
    }

    /// Create a ValueWord Time from a DateTime<Utc> (converts to FixedOffset).
    #[inline]
    fn from_time_utc(t: DateTime<Utc>) -> ValueWord {
        ValueWord::heap_box(HeapValue::Time(t.fixed_offset()))
    }

    /// Create a ValueWord Duration directly.
    #[inline]
    fn from_duration(d: Duration) -> ValueWord {
        ValueWord::heap_box(HeapValue::Duration(d))
    }

    /// Create a ValueWord TimeSpan directly.
    #[inline]
    fn from_timespan(ts: chrono::Duration) -> ValueWord {
        ValueWord::heap_box(HeapValue::TimeSpan(ts))
    }

    /// Create a ValueWord Timeframe directly.
    #[inline]
    fn from_timeframe(tf: Timeframe) -> ValueWord {
        ValueWord::heap_box(HeapValue::Timeframe(tf))
    }

    // ===== Other constructors =====

    /// Create a ValueWord HostClosure directly.
    #[inline]
    fn from_host_closure(nc: HostCallable) -> ValueWord {
        ValueWord::heap_box(HeapValue::HostClosure(nc))
    }

    /// Create a ValueWord PrintResult directly.
    #[inline]
    fn from_print_result(pr: PrintResult) -> ValueWord {
        ValueWord::heap_box(HeapValue::PrintResult(Box::new(pr)))
    }

    /// Create a ValueWord SimulationCall directly.
    #[inline]
    fn from_simulation_call(name: String, params: HashMap<String, ValueWord>) -> ValueWord {
        ValueWord::heap_box(HeapValue::SimulationCall(Box::new(
            crate::heap_value::SimulationCallData { name, params },
        )))
    }

    /// Create a ValueWord FunctionRef directly.
    #[inline]
    fn from_function_ref(name: String, closure: Option<ValueWord>) -> ValueWord {
        ValueWord::heap_box(HeapValue::FunctionRef {
            name,
            closure: closure.map(Box::new),
        })
    }

    /// Create a ValueWord DataReference directly.
    #[inline]
    fn from_data_reference(
        datetime: DateTime<FixedOffset>,
        id: String,
        timeframe: Timeframe,
    ) -> ValueWord {
        ValueWord::heap_box(HeapValue::DataReference(Box::new(
            crate::heap_value::DataReferenceData {
                datetime,
                id,
                timeframe,
            },
        )))
    }

    /// Create a ValueWord TimeReference directly.
    #[inline]
    fn from_time_reference(tr: TimeReference) -> ValueWord {
        ValueWord::heap_box(HeapValue::TimeReference(Box::new(tr)))
    }

    /// Create a ValueWord DateTimeExpr directly.
    #[inline]
    fn from_datetime_expr(de: DateTimeExpr) -> ValueWord {
        ValueWord::heap_box(HeapValue::DateTimeExpr(Box::new(de)))
    }

    /// Create a ValueWord DataDateTimeRef directly.
    #[inline]
    fn from_data_datetime_ref(dr: DataDateTimeRef) -> ValueWord {
        ValueWord::heap_box(HeapValue::DataDateTimeRef(Box::new(dr)))
    }

    /// Create a ValueWord TypeAnnotation directly.
    #[inline]
    fn from_type_annotation(ta: TypeAnnotation) -> ValueWord {
        ValueWord::heap_box(HeapValue::TypeAnnotation(Box::new(ta)))
    }

    /// Create a ValueWord TypeAnnotatedValue directly.
    #[inline]
    fn from_type_annotated_value(type_name: String, value: ValueWord) -> ValueWord {
        ValueWord::heap_box(HeapValue::TypeAnnotatedValue {
            type_name,
            value: Box::new(value),
        })
    }

    /// Create a ValueWord by "cloning" from raw bits.
    /// Bumps the Arc refcount for heap-tagged values.
    #[inline(always)]
    #[cfg(not(feature = "gc"))]
    unsafe fn clone_from_bits(bits: u64) -> ValueWord {
        if is_tagged(bits) && get_tag(bits) == TAG_HEAP {
            let ptr = get_payload(bits) as *const HeapValue;
            Arc::increment_strong_count(ptr);
        }
        bits
    }
    /// Create a ValueWord by "cloning" from raw bits (GC variant — no refcount bump needed).
    #[inline(always)]
    #[cfg(feature = "gc")]
    unsafe fn clone_from_bits(bits: u64) -> ValueWord {
        bits
    }

    // ===== Type checks =====

    /// Returns true if this value is an inline f64 (not a tagged value).
    #[inline(always)]
    fn is_f64(&self) -> bool {
        !is_tagged(*self)
    }

    /// Returns true if this value is an inline i48 integer.
    #[inline(always)]
    fn is_i64(&self) -> bool {
        is_tagged(*self) && get_tag(*self) == TAG_INT
    }

    /// Returns true if this value is a bool.
    #[inline(always)]
    fn is_bool(&self) -> bool {
        is_tagged(*self) && get_tag(*self) == TAG_BOOL
    }

    /// Returns true if this value is None.
    #[inline(always)]
    fn is_none(&self) -> bool {
        is_tagged(*self) && get_tag(*self) == TAG_NONE
    }

    /// Returns true if this value is Unit.
    #[inline(always)]
    fn is_unit(&self) -> bool {
        is_tagged(*self) && get_tag(*self) == TAG_UNIT
    }

    /// Returns true if this value is a function reference.
    #[inline(always)]
    fn is_function(&self) -> bool {
        is_tagged(*self) && get_tag(*self) == TAG_FUNCTION
    }

    /// Returns true if this value is a heap-boxed HeapValue.
    #[inline(always)]
    fn is_heap(&self) -> bool {
        is_tagged(*self) && get_tag(*self) == TAG_HEAP
    }

    /// Returns true if this value is a stack reference.
    #[inline(always)]
    fn is_ref(&self) -> bool {
        if is_tagged(*self) {
            return get_tag(*self) == TAG_REF;
        }
        matches!(self.as_heap_ref(), Some(HeapValue::ProjectedRef(_)))
    }

    /// Extract the reference target.
    #[inline]
    fn as_ref_target(&self) -> Option<RefTarget> {
        if is_tagged(*self) && get_tag(*self) == TAG_REF {
            let payload = get_payload(*self);
            let idx = (payload & REF_TARGET_INDEX_MASK) as usize;
            if payload & REF_TARGET_MODULE_FLAG != 0 {
                return Some(RefTarget::ModuleBinding(idx));
            }
            return Some(RefTarget::Stack(idx));
        }
        if let Some(HeapValue::ProjectedRef(data)) = self.as_heap_ref() {
            return Some(RefTarget::Projected((**data).clone()));
        }
        None
    }

    /// Extract the absolute stack slot index from a stack reference.
    #[inline]
    fn as_ref_slot(&self) -> Option<usize> {
        match self.as_ref_target() {
            Some(RefTarget::Stack(slot)) => Some(slot),
            _ => None,
        }
    }

    // ===== Checked extractors =====

    /// Extract as f64, returning None if this is not an inline f64.
    #[inline]
    fn as_f64(&self) -> Option<f64> {
        if self.is_f64() {
            Some(f64::from_bits(*self))
        } else {
            None
        }
    }

    /// Extract as i64, returning None if this is not an exact signed integer.
    ///
    /// Accepts inline i48 values, heap BigInt, and signed-compatible native scalars.
    #[inline]
    fn as_i64(&self) -> Option<i64> {
        if self.is_i64() {
            Some(sign_extend_i48(get_payload(*self)))
        } else if let Some(HeapValue::BigInt(v)) = self.as_heap_ref() {
            Some(*v)
        } else if let Some(HeapValue::NativeScalar(v)) = self.as_heap_ref() {
            v.as_i64()
        } else {
            None
        }
    }

    /// Extract as u64 when the value is an exact non-negative integer.
    #[inline]
    fn as_u64_value(&self) -> Option<u64> {
        if self.is_i64() {
            let v = sign_extend_i48(get_payload(*self));
            return u64::try_from(v).ok();
        }

        if let Some(hv) = self.as_heap_ref() {
            return match hv {
                HeapValue::BigInt(v) => u64::try_from(*v).ok(),
                HeapValue::NativeScalar(v) => v.as_u64(),
                _ => None,
            };
        }

        None
    }

    /// Extract exact integer domain as i128 (used for width-aware arithmetic/comparison).
    #[inline]
    fn as_i128_exact(&self) -> Option<i128> {
        if self.is_i64() {
            return Some(sign_extend_i48(get_payload(*self)) as i128);
        }

        if let Some(hv) = self.as_heap_ref() {
            return match hv {
                HeapValue::BigInt(v) => Some(*v as i128),
                HeapValue::NativeScalar(v) => v.as_i128(),
                _ => None,
            };
        }

        None
    }

    /// Extract numeric values as f64, including lossless i48→f64 coercion.
    #[inline]
    fn as_number_strict(&self) -> Option<f64> {
        if self.is_f64() {
            return Some(f64::from_bits(*self));
        }
        // Inline i48 integers — lossless conversion to f64.
        if is_tagged(*self) && get_tag(*self) == TAG_INT {
            return Some(sign_extend_i48(get_payload(*self)) as f64);
        }
        if let Some(hv) = self.as_heap_ref() {
            return match hv {
                HeapValue::NativeScalar(NativeScalar::F32(v)) => Some(*v as f64),
                _ => None,
            };
        }
        None
    }

    /// Extract as bool, returning None if this is not a bool.
    #[inline]
    fn as_bool(&self) -> Option<bool> {
        if self.is_bool() {
            Some(get_payload(*self) != 0)
        } else {
            None
        }
    }

    /// Extract as function ID, returning None if this is not a function.
    #[inline]
    fn as_function_id(&self) -> Option<u16> {
        if self.is_function() {
            Some(get_payload(*self) as u16)
        } else {
            None
        }
    }

    // ===== Unchecked extractors (for hot paths where the compiler guarantees type) =====

    /// Extract f64 without type checking.
    /// Safely handles inline i48 ints via lossless coercion.
    ///
    /// # Safety
    /// Caller must ensure the value is numeric (f64 or i48).
    #[inline(always)]
    unsafe fn as_f64_unchecked(&self) -> f64 {
        if self.is_f64() {
            f64::from_bits(*self)
        } else if is_tagged(*self) && get_tag(*self) == TAG_INT {
            sign_extend_i48(get_payload(*self)) as f64
        } else {
            debug_assert!(false, "as_f64_unchecked on non-numeric ValueWord");
            0.0
        }
    }

    /// Extract i64 without type checking.
    /// Safely handles f64 values via truncation.
    ///
    /// # Safety
    /// Caller must ensure the value is numeric (i48 or f64).
    #[inline(always)]
    unsafe fn as_i64_unchecked(&self) -> i64 {
        if is_tagged(*self) && get_tag(*self) == TAG_INT {
            sign_extend_i48(get_payload(*self))
        } else if self.is_f64() {
            f64::from_bits(*self) as i64
        } else {
            debug_assert!(false, "as_i64_unchecked on non-numeric ValueWord");
            0
        }
    }

    /// Extract bool without type checking.
    ///
    /// # Safety
    /// Caller must ensure `self.is_bool()` is true.
    #[inline(always)]
    unsafe fn as_bool_unchecked(&self) -> bool {
        debug_assert!(self.is_bool(), "as_bool_unchecked on non-bool ValueWord");
        get_payload(*self) != 0
    }

    /// Extract function ID without type checking.
    ///
    /// # Safety
    /// Caller must ensure `self.is_function()` is true.
    #[inline(always)]
    unsafe fn as_function_unchecked(&self) -> u16 {
        debug_assert!(
            self.is_function(),
            "as_function_unchecked on non-function ValueWord"
        );
        get_payload(*self) as u16
    }

    // ===== ValueWord inspection API =====

    /// Get a reference to the heap-boxed HeapValue without cloning.
    /// Returns None if this is not a heap value.
    #[inline]
    fn as_heap_ref(&self) -> Option<&HeapValue> {
        if is_tagged(*self) && get_tag(*self) == TAG_HEAP {
            let ptr = get_payload(*self) as *const HeapValue;
            Some(unsafe { &*ptr })
        } else {
            None
        }
    }

    /// Get a mutable reference to the heap-boxed HeapValue, cloning if shared.
    #[inline]
    #[cfg(not(feature = "gc"))]
    fn as_heap_mut(&mut self) -> Option<&mut HeapValue> {
        if is_tagged(*self) && get_tag(*self) == TAG_HEAP {
            let ptr = get_payload(*self) as *const HeapValue;
            let mut arc = unsafe { Arc::from_raw(ptr) };
            Arc::make_mut(&mut arc);
            let new_ptr = Arc::into_raw(arc) as u64;
            *self = make_tagged(TAG_HEAP, new_ptr & PAYLOAD_MASK);
            let final_ptr = get_payload(*self) as *mut HeapValue;
            Some(unsafe { &mut *final_ptr })
        } else {
            None
        }
    }
    /// Get a mutable reference to the heap-boxed HeapValue (GC variant).
    #[inline]
    #[cfg(feature = "gc")]
    fn as_heap_mut(&mut self) -> Option<&mut HeapValue> {
        if is_tagged(*self) && get_tag(*self) == TAG_HEAP {
            let ptr = get_payload(*self) as *mut HeapValue;
            Some(unsafe { &mut *ptr })
        } else {
            None
        }
    }

    /// Check truthiness without materializing HeapValue.
    #[inline]
    fn is_truthy(&self) -> bool {
        if !is_tagged(*self) {
            // f64: truthy if non-zero and not NaN
            let f = f64::from_bits(*self);
            return f != 0.0 && !f.is_nan();
        }
        let tag = get_tag(*self);
        if tag == TAG_HEAP {
            let ptr = get_payload(*self) as *const HeapValue;
            return unsafe { (*ptr).is_truthy() };
        }
        nan_tag_is_truthy(tag, get_payload(*self))
    }

    /// Extract as f64 using safe coercion.
    ///
    /// This accepts:
    /// - explicit floating-point values (`number`, `f32`)
    /// - inline `int` values (`i48`)
    ///
    /// It intentionally does not coerce `BigInt`, `i64`, or `u64`
    /// native scalars into `f64` to avoid lossy conversion.
    #[inline]
    fn as_number_coerce(&self) -> Option<f64> {
        if let Some(n) = self.as_number_strict() {
            Some(n)
        } else if is_tagged(*self) && get_tag(*self) == TAG_INT {
            Some(sign_extend_i48(get_payload(*self)) as f64)
        } else {
            None
        }
    }

    /// Check if this ValueWord is a module function tag.
    #[inline(always)]
    fn is_module_function(&self) -> bool {
        is_tagged(*self) && get_tag(*self) == TAG_MODULE_FN
    }

    /// Extract module function index.
    #[inline]
    fn as_module_function(&self) -> Option<usize> {
        if self.is_module_function() {
            Some(get_payload(*self) as usize)
        } else {
            None
        }
    }

    // ===== HeapValue inspection =====

    /// Get the HeapKind discriminator without cloning.
    /// Returns None if this is not a heap value.
    #[inline]
    fn heap_kind(&self) -> Option<crate::heap_value::HeapKind> {
        if let Some(hv) = self.as_heap_ref() {
            Some(hv.kind())
        } else {
            std::option::Option::None
        }
    }

    // ===== Phase 2A: Extended inspection API =====

    /// Get a reference to a heap String without cloning.
    /// Returns None if this is not a heap-boxed String.
    #[inline]
    fn as_str(&self) -> Option<&str> {
        if let Some(hv) = self.as_heap_ref() {
            match hv {
                HeapValue::String(s) => Some(s.as_str()),
                _ => std::option::Option::None,
            }
        } else {
            std::option::Option::None
        }
    }

    /// Extract a Decimal from a heap-boxed Decimal value.
    /// Returns None if this is not a heap-boxed Decimal.
    #[inline]
    fn as_decimal(&self) -> Option<rust_decimal::Decimal> {
        if let Some(hv) = self.as_heap_ref() {
            match hv {
                HeapValue::Decimal(d) => Some(*d),
                _ => std::option::Option::None,
            }
        } else {
            std::option::Option::None
        }
    }

    /// Extract a Decimal without type checking.
    ///
    /// # Safety
    /// Caller must ensure this is a heap-boxed Decimal value.
    #[inline(always)]
    unsafe fn as_decimal_unchecked(&self) -> rust_decimal::Decimal {
        debug_assert!(matches!(self.as_heap_ref(), Some(HeapValue::Decimal(_))));
        match unsafe { self.as_heap_ref().unwrap_unchecked() } {
            HeapValue::Decimal(d) => *d,
            _ => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    /// Get a reference to a heap Array without cloning.
    /// Returns None if this is not a heap-boxed Array.
    #[inline]
    fn as_array(&self) -> Option<&VMArray> {
        if let Some(hv) = self.as_heap_ref() {
            match hv {
                HeapValue::Array(arr) => Some(arr),
                _ => std::option::Option::None,
            }
        } else {
            std::option::Option::None
        }
    }

    /// Get a unified read-only view over any array variant (Generic, Int, Float, Bool, width-specific).
    #[inline]
    fn as_any_array(&self) -> Option<ArrayView<'_>> {
        match self.as_heap_ref()? {
            HeapValue::Array(a) => Some(ArrayView::Generic(a)),
            HeapValue::IntArray(a) => Some(ArrayView::Int(a)),
            HeapValue::FloatArray(a) => Some(ArrayView::Float(a)),
            HeapValue::BoolArray(a) => Some(ArrayView::Bool(a)),
            HeapValue::I8Array(a) => Some(ArrayView::I8(a)),
            HeapValue::I16Array(a) => Some(ArrayView::I16(a)),
            HeapValue::I32Array(a) => Some(ArrayView::I32(a)),
            HeapValue::U8Array(a) => Some(ArrayView::U8(a)),
            HeapValue::U16Array(a) => Some(ArrayView::U16(a)),
            HeapValue::U32Array(a) => Some(ArrayView::U32(a)),
            HeapValue::U64Array(a) => Some(ArrayView::U64(a)),
            HeapValue::F32Array(a) => Some(ArrayView::F32(a)),
            _ => None,
        }
    }

    /// Get a unified mutable view over any array variant. Uses Arc::make_mut for COW.
    #[inline]
    fn as_any_array_mut(&mut self) -> Option<ArrayViewMut<'_>> {
        match self.as_heap_mut()? {
            HeapValue::Array(a) => Some(ArrayViewMut::Generic(a)),
            HeapValue::IntArray(a) => Some(ArrayViewMut::Int(a)),
            HeapValue::FloatArray(a) => Some(ArrayViewMut::Float(a)),
            HeapValue::BoolArray(a) => Some(ArrayViewMut::Bool(a)),
            _ => None,
        }
    }

    // ===== HeapValue extractors for new variants =====

    /// Extract a reference to a DataTable.
    #[inline]
    fn as_datatable(&self) -> Option<&Arc<DataTable>> {
        match self.as_heap_ref()? {
            HeapValue::DataTable(dt) => Some(dt),
            _ => std::option::Option::None,
        }
    }

    /// Extract TypedTable fields.
    #[inline]
    fn as_typed_table(&self) -> Option<(u64, &Arc<DataTable>)> {
        match self.as_heap_ref()? {
            HeapValue::TypedTable { schema_id, table } => Some((*schema_id, table)),
            _ => std::option::Option::None,
        }
    }

    /// Extract RowView fields.
    #[inline]
    fn as_row_view(&self) -> Option<(u64, &Arc<DataTable>, usize)> {
        match self.as_heap_ref()? {
            HeapValue::RowView {
                schema_id,
                table,
                row_idx,
            } => Some((*schema_id, table, *row_idx)),
            _ => std::option::Option::None,
        }
    }

    /// Extract ColumnRef fields.
    #[inline]
    fn as_column_ref(&self) -> Option<(u64, &Arc<DataTable>, u32)> {
        match self.as_heap_ref()? {
            HeapValue::ColumnRef {
                schema_id,
                table,
                col_id,
            } => Some((*schema_id, table, *col_id)),
            _ => std::option::Option::None,
        }
    }

    /// Extract IndexedTable fields.
    #[inline]
    fn as_indexed_table(&self) -> Option<(u64, &Arc<DataTable>, u32)> {
        match self.as_heap_ref()? {
            HeapValue::IndexedTable {
                schema_id,
                table,
                index_col,
            } => Some((*schema_id, table, *index_col)),
            _ => std::option::Option::None,
        }
    }

    /// Extract TypedObject fields (schema_id, slots, heap_mask).
    #[inline]
    fn as_typed_object(&self) -> Option<(u64, &[ValueSlot], u64)> {
        match self.as_heap_ref()? {
            HeapValue::TypedObject {
                schema_id,
                slots,
                heap_mask,
            } => Some((*schema_id, slots, *heap_mask)),
            _ => std::option::Option::None,
        }
    }

    /// Extract Closure fields (function_id, upvalues).
    #[inline]
    fn as_closure(&self) -> Option<(u16, &[crate::value::Upvalue])> {
        match self.as_heap_ref()? {
            HeapValue::Closure {
                function_id,
                upvalues,
            } => Some((*function_id, upvalues)),
            _ => std::option::Option::None,
        }
    }

    /// Extract the inner value from a Some variant.
    #[inline]
    fn as_some_inner(&self) -> Option<&ValueWord> {
        match self.as_heap_ref()? {
            HeapValue::Some(inner) => Some(inner),
            _ => std::option::Option::None,
        }
    }

    /// Extract the inner value from an Ok variant.
    #[inline]
    fn as_ok_inner(&self) -> Option<&ValueWord> {
        match self.as_heap_ref()? {
            HeapValue::Ok(inner) => Some(inner),
            _ => std::option::Option::None,
        }
    }

    /// Extract the inner value from an Err variant.
    #[inline]
    fn as_err_inner(&self) -> Option<&ValueWord> {
        match self.as_heap_ref()? {
            HeapValue::Err(inner) => Some(inner),
            _ => std::option::Option::None,
        }
    }

    /// Extract the original payload from an Err variant, unwrapping AnyError
    /// normalization if present.
    ///
    /// When `Err(x)` is constructed at runtime, `x` is wrapped in an AnyError
    /// TypedObject (slot layout: [category, payload, cause, trace, message, code]).
    /// This method detects that wrapper and returns the original payload from
    /// slot 1 rather than the full AnyError struct.
    fn as_err_payload(&self) -> Option<ValueWord> {
        let inner = match self.as_heap_ref()? {
            HeapValue::Err(inner) => inner.as_ref(),
            _ => return std::option::Option::None,
        };
        // Check if inner is an AnyError TypedObject: slot 0 is "AnyError" string
        if let Some(HeapValue::TypedObject {
            slots, heap_mask, ..
        }) = inner.as_heap_ref()
        {
            const PAYLOAD_SLOT: usize = 1;
            if slots.len() >= 6 && *heap_mask & 1 != 0 {
                // Verify slot 0 is the "AnyError" category string
                let cat = slots[0].as_heap_value();
                if let HeapValue::String(s) = cat {
                    if s.as_str() == "AnyError" && PAYLOAD_SLOT < slots.len() {
                        let is_heap = *heap_mask & (1u64 << PAYLOAD_SLOT) != 0;
                        return Some(slots[PAYLOAD_SLOT].as_value_word(is_heap));
                    }
                }
            }
        }
        // Not an AnyError wrapper — return inner directly
        Some(inner.clone())
    }

    /// Extract a Future ID.
    #[inline]
    fn as_future(&self) -> Option<u64> {
        match self.as_heap_ref()? {
            HeapValue::Future(id) => Some(*id),
            _ => std::option::Option::None,
        }
    }

    /// Extract TraitObject fields.
    #[inline]
    fn as_trait_object(&self) -> Option<(&ValueWord, &Arc<VTable>)> {
        match self.as_heap_ref()? {
            HeapValue::TraitObject { value, vtable } => Some((value, vtable)),
            _ => std::option::Option::None,
        }
    }

    /// Extract ExprProxy column name.
    #[inline]
    fn as_expr_proxy(&self) -> Option<&Arc<String>> {
        match self.as_heap_ref()? {
            HeapValue::ExprProxy(name) => Some(name),
            _ => std::option::Option::None,
        }
    }

    /// Extract FilterExpr node.
    #[inline]
    fn as_filter_expr(&self) -> Option<&Arc<FilterNode>> {
        match self.as_heap_ref()? {
            HeapValue::FilterExpr(node) => Some(node),
            _ => std::option::Option::None,
        }
    }

    /// Extract a HostClosure reference.
    #[inline]
    fn as_host_closure(&self) -> Option<&HostCallable> {
        match self.as_heap_ref()? {
            HeapValue::HostClosure(nc) => Some(nc),
            _ => std::option::Option::None,
        }
    }

    /// Extract a Duration reference.
    #[inline]
    fn as_duration(&self) -> Option<&shape_ast::ast::Duration> {
        match self.as_heap_ref()? {
            HeapValue::Duration(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Extract a Range (start, end, inclusive).
    #[inline]
    fn as_range(&self) -> Option<(Option<&ValueWord>, Option<&ValueWord>, bool)> {
        match self.as_heap_ref()? {
            HeapValue::Range {
                start,
                end,
                inclusive,
            } => Some((
                start.as_ref().map(|b| b.as_ref()),
                end.as_ref().map(|b| b.as_ref()),
                *inclusive,
            )),
            _ => std::option::Option::None,
        }
    }

    /// Extract a TimeSpan (chrono::Duration).
    #[inline]
    fn as_timespan(&self) -> Option<chrono::Duration> {
        match self.as_heap_ref()? {
            HeapValue::TimeSpan(ts) => Some(*ts),
            _ => std::option::Option::None,
        }
    }

    /// Extract a Timeframe reference.
    #[inline]
    fn as_timeframe(&self) -> Option<&Timeframe> {
        match self.as_heap_ref()? {
            HeapValue::Timeframe(tf) => Some(tf),
            _ => std::option::Option::None,
        }
    }

    /// Get the HashMap contents if this is a HashMap.
    #[inline]
    fn as_hashmap(
        &self,
    ) -> Option<(&Vec<ValueWord>, &Vec<ValueWord>, &HashMap<u64, Vec<usize>>)> {
        match self.as_heap_ref()? {
            HeapValue::HashMap(d) => Some((&d.keys, &d.values, &d.index)),
            _ => std::option::Option::None,
        }
    }

    /// Get read-only access to the full HashMapData (includes shape_id).
    #[inline]
    fn as_hashmap_data(&self) -> Option<&HashMapData> {
        match self.as_heap_ref()? {
            HeapValue::HashMap(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get mutable access to the HashMapData.
    /// Uses copy-on-write via `as_heap_mut()` (clones if Arc refcount > 1).
    #[inline]
    fn as_hashmap_mut(&mut self) -> Option<&mut HashMapData> {
        match self.as_heap_mut()? {
            HeapValue::HashMap(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get mutable access to the SetData.
    #[inline]
    fn as_set_mut(&mut self) -> Option<&mut SetData> {
        match self.as_heap_mut()? {
            HeapValue::Set(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get mutable access to the DequeData.
    #[inline]
    fn as_deque_mut(&mut self) -> Option<&mut DequeData> {
        match self.as_heap_mut()? {
            HeapValue::Deque(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get mutable access to the PriorityQueueData.
    #[inline]
    fn as_priority_queue_mut(&mut self) -> Option<&mut PriorityQueueData> {
        match self.as_heap_mut()? {
            HeapValue::PriorityQueue(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Extract a ContentNode reference.
    #[inline]
    fn as_content(&self) -> Option<&ContentNode> {
        match self.as_heap_ref()? {
            HeapValue::Content(node) => Some(node),
            _ => std::option::Option::None,
        }
    }

    /// Extract a DateTime<FixedOffset>.
    #[inline]
    fn as_time(&self) -> Option<DateTime<FixedOffset>> {
        match self.as_heap_ref()? {
            HeapValue::Time(t) => Some(*t),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to the Instant.
    #[inline]
    fn as_instant(&self) -> Option<&std::time::Instant> {
        match self.as_heap_ref()? {
            HeapValue::Instant(t) => Some(t.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to the IoHandleData.
    #[inline]
    fn as_io_handle(&self) -> Option<&crate::heap_value::IoHandleData> {
        match self.as_heap_ref()? {
            HeapValue::IoHandle(data) => Some(data.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Extract a width-aware native scalar value.
    #[inline]
    fn as_native_scalar(&self) -> Option<NativeScalar> {
        match self.as_heap_ref()? {
            HeapValue::NativeScalar(v) => Some(*v),
            _ => None,
        }
    }

    /// Extract a pointer-backed native view.
    #[inline]
    fn as_native_view(&self) -> Option<&NativeViewData> {
        match self.as_heap_ref()? {
            HeapValue::NativeView(view) => Some(view.as_ref()),
            _ => None,
        }
    }

    /// Extract a reference to the DateTime<FixedOffset>.
    #[inline]
    fn as_datetime(&self) -> Option<&DateTime<FixedOffset>> {
        match self.as_heap_ref()? {
            HeapValue::Time(t) => Some(t),
            _ => std::option::Option::None,
        }
    }

    /// Extract an Arc<String> from a heap String.
    #[inline]
    fn as_arc_string(&self) -> Option<&Arc<String>> {
        match self.as_heap_ref()? {
            HeapValue::String(s) => Some(s),
            _ => std::option::Option::None,
        }
    }

    // ===== Typed collection accessors =====

    /// Extract a reference to an IntArray.
    #[inline]
    fn as_int_array(&self) -> Option<&Arc<crate::typed_buffer::TypedBuffer<i64>>> {
        match self.as_heap_ref()? {
            HeapValue::IntArray(a) => Some(a),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to a FloatArray.
    #[inline]
    fn as_float_array(&self) -> Option<&Arc<crate::typed_buffer::AlignedTypedBuffer>> {
        match self.as_heap_ref()? {
            HeapValue::FloatArray(a) => Some(a),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to a BoolArray.
    #[inline]
    fn as_bool_array(&self) -> Option<&Arc<crate::typed_buffer::TypedBuffer<u8>>> {
        match self.as_heap_ref()? {
            HeapValue::BoolArray(a) => Some(a),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to MatrixData.
    #[inline]
    fn as_matrix(&self) -> Option<&crate::heap_value::MatrixData> {
        match self.as_heap_ref()? {
            HeapValue::Matrix(m) => Some(m.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to IteratorState.
    #[inline]
    fn as_iterator(&self) -> Option<&crate::heap_value::IteratorState> {
        match self.as_heap_ref()? {
            HeapValue::Iterator(it) => Some(it.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to GeneratorState.
    #[inline]
    fn as_generator(&self) -> Option<&crate::heap_value::GeneratorState> {
        match self.as_heap_ref()? {
            HeapValue::Generator(g) => Some(g.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Get the length of a typed array (IntArray, FloatArray, BoolArray, width-specific).
    /// Returns None for non-typed-array values.
    #[inline]
    fn typed_array_len(&self) -> Option<usize> {
        match self.as_heap_ref()? {
            HeapValue::IntArray(a) => Some(a.len()),
            HeapValue::FloatArray(a) => Some(a.len()),
            HeapValue::BoolArray(a) => Some(a.len()),
            HeapValue::I8Array(a) => Some(a.len()),
            HeapValue::I16Array(a) => Some(a.len()),
            HeapValue::I32Array(a) => Some(a.len()),
            HeapValue::U8Array(a) => Some(a.len()),
            HeapValue::U16Array(a) => Some(a.len()),
            HeapValue::U32Array(a) => Some(a.len()),
            HeapValue::U64Array(a) => Some(a.len()),
            HeapValue::F32Array(a) => Some(a.len()),
            _ => std::option::Option::None,
        }
    }

    /// Coerce a typed array to a FloatArray (zero-copy for FloatArray, convert for IntArray).
    fn coerce_to_float_array(&self) -> Option<Arc<crate::typed_buffer::AlignedTypedBuffer>> {
        match self.as_heap_ref()? {
            HeapValue::FloatArray(a) => Some(Arc::clone(a)),
            HeapValue::IntArray(a) => {
                let mut buf = crate::typed_buffer::AlignedTypedBuffer::with_capacity(a.len());
                for &v in a.iter() {
                    buf.push(v as f64);
                }
                Some(Arc::new(buf))
            }
            _ => std::option::Option::None,
        }
    }

    /// Convert a typed array to a generic Array of ValueWord values.
    fn to_generic_array(&self) -> Option<crate::value::VMArray> {
        // Generic Array path: hand back the existing Arc<Vec<ValueWord>>.
        if let Some(HeapValue::Array(arc)) = self.as_heap_ref() {
            return Some(arc.clone());
        }
        match self.as_heap_ref()? {
            HeapValue::IntArray(a) => Some(Arc::new(
                a.iter().map(|&v| ValueWord::from_i64(v)).collect(),
            )),
            HeapValue::FloatArray(a) => Some(Arc::new(
                a.iter().map(|&v| ValueWord::from_f64(v)).collect(),
            )),
            HeapValue::BoolArray(a) => Some(Arc::new(
                a.iter().map(|&v| ValueWord::from_bool(v != 0)).collect(),
            )),
            _ => std::option::Option::None,
        }
    }

    /// Backwards-compat shim for tests that previously used `to_array_arc`.
    /// Forwards to [`to_generic_array`] which now handles all array variants.
    #[inline]
    fn to_array_arc(&self) -> Option<crate::value::VMArray> {
        self.to_generic_array()
    }

    /// Fast equality comparison without materializing HeapValue.
    /// For inline types (f64, i48, bool, none, unit, function), compares bits directly.
    /// For heap types, falls back to HeapValue equality.
    #[inline]
    fn vw_equals(&self, other: &ValueWord) -> bool {
        // Fast path: identical bits means identical value (except NaN)
        if *self == *other {
            // Special case: f64 NaN != NaN
            if !is_tagged(*self) {
                let f = f64::from_bits(*self);
                return !f.is_nan();
            }
            // For heap values, same bits = same pointer = definitely equal
            return true;
        }
        // Different bits — for inline types, they're definitely not equal
        if !is_tagged(*self) || !is_tagged(*other) {
            // At least one is f64 — if they're both f64 with different bits, not equal
            // (we already handled the case where both are identical)
            // Cross-type: f64 == i48 coercion
            if let (Some(a), Some(b)) = (self.as_number_coerce(), other.as_number_coerce()) {
                return a == b;
            }
            return false;
        }
        let tag_a = get_tag(*self);
        let tag_b = get_tag(*other);
        if tag_a != tag_b {
            // Different tags — check numeric coercion (f64 vs i48)
            if (tag_a == TAG_INT || !is_tagged(*self)) && (tag_b == TAG_INT || !is_tagged(*other))
            {
                if let (Some(a), Some(b)) = (self.as_number_coerce(), other.as_number_coerce()) {
                    return a == b;
                }
            }
            return false;
        }
        // Same tag, different bits — for heap values, compare HeapValue
        if tag_a == TAG_HEAP {
            let ptr_a = get_payload(*self) as *const HeapValue;
            let ptr_b = get_payload(*other) as *const HeapValue;
            return unsafe { (*ptr_a).equals(&*ptr_b) };
        }
        // For other same-tag inline values with different bits, not equal
        false
    }

    /// Compute a hash for a ValueWord value, suitable for HashMap key usage.
    /// Uses the existing tag dispatch for O(1) inline types.
    fn vw_hash(&self) -> u64 {
        use ahash::AHasher;
        use std::hash::{Hash, Hasher};

        let bits = *self;
        if !is_tagged(bits) {
            let f = unsafe { self.as_f64_unchecked() };
            let fb = if f == 0.0 { 0u64 } else { f.to_bits() };
            let mut hasher = AHasher::default();
            fb.hash(&mut hasher);
            return hasher.finish();
        }
        let tag = get_tag(bits);
        if tag == TAG_INT {
            let i = unsafe { self.as_i64_unchecked() };
            let mut hasher = AHasher::default();
            i.hash(&mut hasher);
            return hasher.finish();
        }
        if tag == TAG_BOOL {
            return if unsafe { self.as_bool_unchecked() } { 1 } else { 0 };
        }
        if tag == TAG_NONE {
            return 0x_DEAD_0000;
        }
        if tag == TAG_UNIT {
            return 0x_DEAD_0001;
        }
        if tag == TAG_HEAP {
                if let Some(hv) = self.as_heap_ref() {
                    match hv {
                        HeapValue::String(s) => {
                            let mut hasher = AHasher::default();
                            s.hash(&mut hasher);
                            hasher.finish()
                        }
                        HeapValue::BigInt(i) => {
                            let mut hasher = AHasher::default();
                            i.hash(&mut hasher);
                            hasher.finish()
                        }
                        HeapValue::Decimal(d) => {
                            let mut hasher = AHasher::default();
                            d.mantissa().hash(&mut hasher);
                            d.scale().hash(&mut hasher);
                            hasher.finish()
                        }
                        HeapValue::NativeScalar(v) => {
                            let mut hasher = AHasher::default();
                            v.type_name().hash(&mut hasher);
                            v.to_string().hash(&mut hasher);
                            hasher.finish()
                        }
                        HeapValue::NativeView(v) => {
                            let mut hasher = AHasher::default();
                            v.ptr.hash(&mut hasher);
                            v.layout.name.hash(&mut hasher);
                            v.mutable.hash(&mut hasher);
                            hasher.finish()
                        }
                        _ => {
                            let mut hasher = AHasher::default();
                            (*self).hash(&mut hasher);
                            hasher.finish()
                        }
                    }
                } else {
                    let mut hasher = AHasher::default();
                    (*self).hash(&mut hasher);
                    hasher.finish()
                }
            } else {
                let mut hasher = AHasher::default();
                (*self).hash(&mut hasher);
                hasher.finish()
            }
    }

    // ===== Arithmetic helpers (operate directly on bits, no conversion) =====

    /// Add two inline i48 values with overflow promotion to f64.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline i48 values (`is_i64()` is true).
    #[inline(always)]
    unsafe fn add_i64(a: &Self, b: &Self) -> ValueWord {
        debug_assert!(a.is_i64() && b.is_i64());
        let lhs = unsafe { a.as_i64_unchecked() };
        let rhs = unsafe { b.as_i64_unchecked() };
        match lhs.checked_add(rhs) {
            Some(result) if result >= I48_MIN && result <= I48_MAX => ValueWord::from_i64(result),
            _ => ValueWord::from_f64(lhs as f64 + rhs as f64),
        }
    }

    /// Subtract two inline i48 values with overflow promotion to f64.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline i48 values.
    #[inline(always)]
    unsafe fn sub_i64(a: &Self, b: &Self) -> ValueWord {
        debug_assert!(a.is_i64() && b.is_i64());
        let lhs = unsafe { a.as_i64_unchecked() };
        let rhs = unsafe { b.as_i64_unchecked() };
        match lhs.checked_sub(rhs) {
            Some(result) if result >= I48_MIN && result <= I48_MAX => ValueWord::from_i64(result),
            _ => ValueWord::from_f64(lhs as f64 - rhs as f64),
        }
    }

    /// Multiply two inline i48 values with overflow promotion to f64.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline i48 values.
    #[inline(always)]
    unsafe fn mul_i64(a: &Self, b: &Self) -> ValueWord {
        debug_assert!(a.is_i64() && b.is_i64());
        let lhs = unsafe { a.as_i64_unchecked() };
        let rhs = unsafe { b.as_i64_unchecked() };
        match lhs.checked_mul(rhs) {
            Some(result) if result >= I48_MIN && result <= I48_MAX => ValueWord::from_i64(result),
            _ => ValueWord::from_f64(lhs as f64 * rhs as f64),
        }
    }

    /// Binary arithmetic with integer-preserving semantics and overflow promotion.
    ///
    /// If both operands are inline I48, applies `int_op` (checked) to the i64 values.
    /// On overflow (None), falls back to `float_op` with the f64 coercions.
    /// If either operand is f64, applies `float_op` directly.
    /// Callers must ensure `a_num` and `b_num` are the `as_number_coerce()` results
    /// from the same `a`/`b` operands.
    #[inline(always)]
    fn binary_int_preserving(
        a: &Self,
        b: &Self,
        a_num: f64,
        b_num: f64,
        int_op: impl FnOnce(i64, i64) -> Option<i64>,
        float_op: impl FnOnce(f64, f64) -> f64,
    ) -> ValueWord {
        if a.is_i64() && b.is_i64() {
            match int_op(unsafe { a.as_i64_unchecked() }, unsafe {
                b.as_i64_unchecked()
            }) {
                Some(result) => ValueWord::from_i64(result),
                None => ValueWord::from_f64(float_op(a_num, b_num)),
            }
        } else {
            ValueWord::from_f64(float_op(a_num, b_num))
        }
    }

    /// Returns the raw u64 bits (for debugging/testing).
    #[inline(always)]
    fn raw_bits(&self) -> u64 {
        *self
    }

    /// Create a ValueWord from raw u64 bits without any NaN-boxing or tagging.
    ///
    /// This is the inverse of `raw_bits()`. The caller is responsible for
    /// ensuring the bits are interpreted correctly (e.g. via `push_raw_f64` /
    /// `pop_raw_f64`). No Drop/heap semantics are attached to the resulting
    /// value — it is treated as an opaque 8-byte slot.
    #[inline(always)]
    fn from_raw_bits(bits: u64) -> ValueWord {
        bits
    }

    /// Return the raw u64 bits. Identity since ValueWord is u64.
    #[inline(always)]
    fn into_raw_bits(self) -> u64 {
        self
    }

    /// Get the type name of this value.
    #[inline]
    fn type_name(&self) -> &'static str {
        if !is_tagged(*self) {
            return "number";
        }
        let tag = get_tag(*self);
        if tag == TAG_HEAP {
            let ptr = get_payload(*self) as *const HeapValue;
            return unsafe { (*ptr).type_name() };
        }
        nan_tag_type_name(tag)
    }

    // ===== Convenience aliases for common extraction patterns =====

    /// Extract as f64, coercing i48 to f64 if needed.
    /// Alias for `as_number_coerce()` — convenience method.
    #[inline]
    fn to_number(&self) -> Option<f64> {
        self.as_number_coerce()
    }

    /// Extract as bool.
    /// Alias for `as_bool()` — convenience method.
    #[inline]
    fn to_bool(&self) -> Option<bool> {
        self.as_bool()
    }

    /// Convert Int or Number to usize (for indexing operations).
    fn as_usize(&self) -> Option<usize> {
        if let Some(i) = self.as_i64() {
            if i >= 0 {
                return Some(i as usize);
            }
        } else if let Some(n) = self.as_f64() {
            if n >= 0.0 && n.is_finite() {
                return Some(n as usize);
            }
        } else if let Some(d) = self.as_decimal() {
            use rust_decimal::prelude::ToPrimitive;
            if let Some(n) = d.to_f64() {
                if n >= 0.0 {
                    return Some(n as usize);
                }
            }
        } else if let Some(view) = self.as_native_view() {
            return Some(view.ptr);
        }
        None
    }

    /// Convert this value to a JSON value for serialization.
    fn to_json_value(&self) -> serde_json::Value {
        use crate::heap_value::HeapValue;
        if let Some(n) = self.as_f64() {
            return serde_json::json!(n);
        }
        if let Some(i) = self.as_i64() {
            return serde_json::json!(i);
        }
        if let Some(b) = self.as_bool() {
            return serde_json::json!(b);
        }
        if self.is_none() || self.is_unit() {
            return serde_json::Value::Null;
        }
        if self.is_function() || self.is_module_function() {
            return serde_json::json!(format!("<{}>", self.type_name()));
        }
        match self.as_heap_ref() {
            Some(HeapValue::String(s)) => serde_json::json!(s.as_str()),
            Some(HeapValue::Decimal(d)) => {
                use rust_decimal::prelude::ToPrimitive;
                if let Some(f) = d.to_f64() {
                    serde_json::json!(f)
                } else {
                    serde_json::json!(d.to_string())
                }
            }
            Some(HeapValue::Array(arr)) => {
                let values: Vec<serde_json::Value> =
                    arr.iter().map(|v| v.to_json_value()).collect();
                serde_json::json!(values)
            }
            Some(HeapValue::Some(v)) => v.to_json_value(),
            Some(HeapValue::Ok(v)) => serde_json::json!({
                "status": "ok",
                "value": v.to_json_value()
            }),
            Some(HeapValue::Err(v)) => serde_json::json!({
                "status": "error",
                "value": v.to_json_value()
            }),
            Some(HeapValue::DataTable(dt)) => serde_json::json!({
                "type": "datatable",
                "rows": dt.row_count(),
                "columns": dt.column_names(),
            }),
            Some(HeapValue::TypedTable { table, schema_id }) => serde_json::json!({
                "type": "typed_table",
                "schema_id": schema_id,
                "rows": table.row_count(),
                "columns": table.column_names(),
            }),
            Some(HeapValue::RowView {
                schema_id, row_idx, ..
            }) => serde_json::json!({
                "type": "row",
                "schema_id": schema_id,
                "row_idx": row_idx,
            }),
            Some(HeapValue::ColumnRef {
                schema_id, col_id, ..
            }) => serde_json::json!({
                "type": "column_ref",
                "schema_id": schema_id,
                "col_id": col_id,
            }),
            Some(HeapValue::IndexedTable {
                schema_id,
                table,
                index_col,
            }) => serde_json::json!({
                "type": "indexed_table",
                "schema_id": schema_id,
                "rows": table.row_count(),
                "columns": table.column_names(),
                "index_col": index_col,
            }),
            Some(HeapValue::HashMap(d)) => {
                let mut map = serde_json::Map::new();
                for (k, v) in d.keys.iter().zip(d.values.iter()) {
                    map.insert(format!("{}", ValueWordDisplay(*k)), v.to_json_value());
                }
                serde_json::Value::Object(map)
            }
            Some(HeapValue::Set(d)) => {
                serde_json::Value::Array(d.items.iter().map(|v| v.to_json_value()).collect())
            }
            Some(HeapValue::Deque(d)) => {
                serde_json::Value::Array(d.items.iter().map(|v| v.to_json_value()).collect())
            }
            Some(HeapValue::PriorityQueue(d)) => {
                serde_json::Value::Array(d.items.iter().map(|v| v.to_json_value()).collect())
            }
            Some(HeapValue::NativeScalar(v)) => match v {
                NativeScalar::I8(n) => serde_json::json!({ "type": "i8", "value": n }),
                NativeScalar::U8(n) => serde_json::json!({ "type": "u8", "value": n }),
                NativeScalar::I16(n) => serde_json::json!({ "type": "i16", "value": n }),
                NativeScalar::U16(n) => serde_json::json!({ "type": "u16", "value": n }),
                NativeScalar::I32(n) => serde_json::json!({ "type": "i32", "value": n }),
                NativeScalar::U32(n) => serde_json::json!({ "type": "u32", "value": n }),
                NativeScalar::I64(n) => {
                    serde_json::json!({ "type": "i64", "value": n.to_string() })
                }
                NativeScalar::U64(n) => {
                    serde_json::json!({ "type": "u64", "value": n.to_string() })
                }
                NativeScalar::Isize(n) => {
                    serde_json::json!({ "type": "isize", "value": n.to_string() })
                }
                NativeScalar::Usize(n) => {
                    serde_json::json!({ "type": "usize", "value": n.to_string() })
                }
                NativeScalar::Ptr(n) => {
                    serde_json::json!({ "type": "ptr", "value": format!("0x{n:x}") })
                }
                NativeScalar::F32(n) => serde_json::json!({ "type": "f32", "value": n }),
            },
            Some(HeapValue::NativeView(v)) => serde_json::json!({
                "type": if v.mutable { "cmut" } else { "cview" },
                "layout": v.layout.name,
                "ptr": v.ptr,
            }),
            // Typed arrays — serialize as JSON arrays of their element values
            Some(HeapValue::IntArray(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            Some(HeapValue::FloatArray(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            Some(HeapValue::BoolArray(buf)) => serde_json::Value::Array(
                buf.data
                    .iter()
                    .map(|&v| serde_json::json!(v != 0))
                    .collect(),
            ),
            Some(HeapValue::I8Array(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            Some(HeapValue::I16Array(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            Some(HeapValue::I32Array(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            Some(HeapValue::U8Array(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            Some(HeapValue::U16Array(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            Some(HeapValue::U32Array(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            Some(HeapValue::U64Array(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            Some(HeapValue::F32Array(buf)) => {
                serde_json::Value::Array(buf.data.iter().map(|&v| serde_json::json!(v)).collect())
            }
            _ => serde_json::json!(format!("<{}>", self.type_name())),
        }
    }
}








impl std::fmt::Display for ValueWordDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_f64() {
            let n = unsafe { self.0.as_f64_unchecked() };
            if n == n.trunc() && n.abs() < 1e15 {
                write!(f, "{}.0", n as i64)
            } else {
                write!(f, "{}", n)
            }
        } else if self.0.is_i64() {
            write!(f, "{}", unsafe { self.0.as_i64_unchecked() })
        } else if self.0.is_bool() {
            write!(f, "{}", unsafe { self.0.as_bool_unchecked() })
        } else if self.0.is_none() {
            write!(f, "none")
        } else if self.0.is_unit() {
            write!(f, "()")
        } else if self.0.is_function() {
            write!(f, "<function:{}>", unsafe { self.0.as_function_unchecked() })
        } else if self.0.is_module_function() {
            write!(f, "<module_function>")
        } else if let Some(target) = self.0.as_ref_target() {
            match target {
                RefTarget::Stack(slot) => write!(f, "&slot_{}", slot),
                RefTarget::ModuleBinding(slot) => write!(f, "&module_{}", slot),
                RefTarget::Projected(_) => write!(f, "&ref"),
            }
        } else if let Some(hv) = self.0.as_heap_ref() {
            // Delegate to HeapValue's Display impl
            write!(f, "{}", hv)
        } else {
            write!(f, "<unknown>")
        }
    }
}

impl std::fmt::Debug for ValueWordDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_f64() {
            write!(f, "ValueWord(f64: {})", unsafe { self.0.as_f64_unchecked() })
        } else if self.0.is_i64() {
            write!(f, "ValueWord(i64: {})", unsafe { self.0.as_i64_unchecked() })
        } else if self.0.is_bool() {
            write!(f, "ValueWord(bool: {})", unsafe {
                self.0.as_bool_unchecked()
            })
        } else if self.0.is_none() {
            write!(f, "ValueWord(None)")
        } else if self.0.is_unit() {
            write!(f, "ValueWord(Unit)")
        } else if self.0.is_function() {
            write!(f, "ValueWord(Function({}))", unsafe {
                self.0.as_function_unchecked()
            })
        } else if let Some(target) = self.0.as_ref_target() {
            write!(f, "ValueWord(Ref({:?}))", target)
        } else if self.0.is_heap() {
            let ptr = get_payload(self.0) as *const HeapValue;
            let hv = unsafe { &*ptr };
            write!(f, "ValueWord(heap: {:?})", hv)
        } else {
            write!(f, "ValueWord(0x{:016x})", self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ===== f64 round-trips =====

    #[test]
    fn test_f64_roundtrip_positive() {
        let v = ValueWord::from_f64(3.14);
        assert!(v.is_f64());
        assert_eq!(v.as_f64(), Some(3.14));
        unsafe { assert_eq!(v.as_f64_unchecked(), 3.14) };
    }

    #[test]
    fn test_f64_roundtrip_negative() {
        let v = ValueWord::from_f64(-123.456);
        assert!(v.is_f64());
        assert_eq!(v.as_f64(), Some(-123.456));
    }

    #[test]
    fn test_f64_zero() {
        let v = ValueWord::from_f64(0.0);
        assert!(v.is_f64());
        assert_eq!(v.as_f64(), Some(0.0));
    }

    #[test]
    fn test_f64_negative_zero() {
        let v = ValueWord::from_f64(-0.0);
        assert!(v.is_f64());
        let extracted = v.as_f64().unwrap();
        // -0.0 and 0.0 are equal per IEEE 754
        assert_eq!(extracted, 0.0);
        // But the sign bit should be preserved
        assert!(extracted.is_sign_negative());
    }

    #[test]
    fn test_f64_infinity() {
        let pos = ValueWord::from_f64(f64::INFINITY);
        assert!(pos.is_f64());
        assert_eq!(pos.as_f64(), Some(f64::INFINITY));

        let neg = ValueWord::from_f64(f64::NEG_INFINITY);
        assert!(neg.is_f64());
        assert_eq!(neg.as_f64(), Some(f64::NEG_INFINITY));
    }

    #[test]
    fn test_f64_nan_canonicalized() {
        let v = ValueWord::from_f64(f64::NAN);
        // NaN is stored as a canonical NaN, which is still a valid f64 NaN
        assert!(v.is_f64());
        let extracted = v.as_f64().unwrap();
        assert!(extracted.is_nan());
    }

    #[test]
    fn test_f64_subnormal() {
        let tiny = f64::MIN_POSITIVE / 2.0; // subnormal
        let v = ValueWord::from_f64(tiny);
        assert!(v.is_f64());
        assert_eq!(v.as_f64(), Some(tiny));
    }

    #[test]
    fn test_f64_max_min() {
        let max = ValueWord::from_f64(f64::MAX);
        assert!(max.is_f64());
        assert_eq!(max.as_f64(), Some(f64::MAX));

        let min = ValueWord::from_f64(f64::MIN);
        assert!(min.is_f64());
        assert_eq!(min.as_f64(), Some(f64::MIN));
    }

    // ===== i64 round-trips =====

    #[test]
    fn test_i64_small_positive() {
        let v = ValueWord::from_i64(42);
        assert!(v.is_i64());
        assert_eq!(v.as_i64(), Some(42));
        unsafe { assert_eq!(v.as_i64_unchecked(), 42) };
    }

    #[test]
    fn test_i64_small_negative() {
        let v = ValueWord::from_i64(-42);
        assert!(v.is_i64());
        assert_eq!(v.as_i64(), Some(-42));
    }

    #[test]
    fn test_i64_zero() {
        let v = ValueWord::from_i64(0);
        assert!(v.is_i64());
        assert_eq!(v.as_i64(), Some(0));
    }

    #[test]
    fn test_i64_i48_max() {
        let max = I48_MAX;
        let v = ValueWord::from_i64(max);
        assert!(v.is_i64());
        assert_eq!(v.as_i64(), Some(max));
    }

    #[test]
    fn test_i64_i48_min() {
        let min = I48_MIN;
        let v = ValueWord::from_i64(min);
        assert!(v.is_i64());
        assert_eq!(v.as_i64(), Some(min));
    }

    #[test]
    fn test_i64_large_needs_heap() {
        // i64::MAX exceeds 48 bits, should heap-box as BigInt
        let v = ValueWord::from_i64(i64::MAX);
        assert!(v.is_heap());
        assert_eq!(v.as_i64(), Some(i64::MAX));
    }

    #[test]
    fn test_i64_min_needs_heap() {
        let v = ValueWord::from_i64(i64::MIN);
        assert!(v.is_heap());
        assert_eq!(v.as_i64(), Some(i64::MIN));
    }

    #[test]
    fn test_i64_just_outside_i48_positive() {
        let val = I48_MAX + 1;
        let v = ValueWord::from_i64(val);
        assert!(v.is_heap());
        assert_eq!(v.as_i64(), Some(val));
    }

    #[test]
    fn test_i64_just_outside_i48_negative() {
        let val = I48_MIN - 1;
        let v = ValueWord::from_i64(val);
        assert!(v.is_heap());
        assert_eq!(v.as_i64(), Some(val));
    }

    // ===== bool round-trips =====

    #[test]
    fn test_bool_true() {
        let v = ValueWord::from_bool(true);
        assert!(v.is_bool());
        assert_eq!(v.as_bool(), Some(true));
        unsafe { assert_eq!(v.as_bool_unchecked(), true) };
    }

    #[test]
    fn test_bool_false() {
        let v = ValueWord::from_bool(false);
        assert!(v.is_bool());
        assert_eq!(v.as_bool(), Some(false));
        unsafe { assert_eq!(v.as_bool_unchecked(), false) };
    }

    // ===== None / Unit =====

    #[test]
    fn test_none() {
        let v = ValueWord::none();
        assert!(v.is_none());
        assert!(!v.is_f64());
        assert!(!v.is_i64());
        assert!(!v.is_bool());
    }

    #[test]
    fn test_unit() {
        let v = ValueWord::unit();
        assert!(v.is_unit());
    }

    // ===== Function =====

    #[test]
    fn test_function() {
        let v = ValueWord::from_function(42);
        assert!(v.is_function());
        assert_eq!(v.as_function_id(), Some(42));
        unsafe { assert_eq!(v.as_function_unchecked(), 42) };
    }

    #[test]
    fn test_function_max_id() {
        let v = ValueWord::from_function(u16::MAX);
        assert!(v.is_function());
        assert_eq!(v.as_function_id(), Some(u16::MAX));
    }

    // ===== Module function =====

    #[test]
    fn test_module_function() {
        let v = ValueWord::from_module_function(99);
        assert!(is_tagged(v));
        assert_eq!(get_tag(v), TAG_MODULE_FN);
        assert_eq!(v.as_module_function(), Some(99));
    }

    // ===== Heap round-trips =====

    #[test]
    fn test_heap_string_roundtrip() {
        let v = ValueWord::from_string(Arc::new("hello world".to_string()));
        assert!(v.is_heap());
        assert_eq!(v.as_arc_string().map(|s| s.as_str()), Some("hello world"));
    }

    #[test]
    fn test_heap_array_roundtrip() {
        let arr = Arc::new(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
        ]);
        let v = ValueWord::from_array(arr.clone());
        assert!(v.is_heap());
        let extracted = v.as_array().expect("should be array");
        assert_eq!(extracted.len(), 3);
    }

    #[test]
    fn test_heap_clone() {
        let v = ValueWord::from_string(Arc::new("clone me".to_string()));
        let cloned = v.clone();

        // Both should extract the same string
        assert_eq!(v.as_arc_string().map(|s| s.as_str()), Some("clone me"));
        assert_eq!(cloned.as_arc_string().map(|s| s.as_str()), Some("clone me"));

        // Clones share the same Arc allocation (refcount bump, not deep copy)
        assert_eq!(
            get_payload(v),
            get_payload(cloned),
            "cloned heap pointers should be identical (Arc shared)"
        );
    }

    // ===== Type discrimination =====

    #[test]
    fn test_type_checks_exclusive() {
        let f = ValueWord::from_f64(1.0);
        assert!(f.is_f64());
        assert!(!f.is_i64());
        assert!(!f.is_bool());
        assert!(!f.is_none());
        assert!(!f.is_unit());
        assert!(!f.is_function());
        assert!(!f.is_heap());

        let i = ValueWord::from_i64(1);
        assert!(!i.is_f64());
        assert!(i.is_i64());
        assert!(!i.is_bool());
        assert!(!i.is_none());
        assert!(!i.is_unit());
        assert!(!i.is_function());
        assert!(!i.is_heap());

        let b = ValueWord::from_bool(true);
        assert!(!b.is_f64());
        assert!(!b.is_i64());
        assert!(b.is_bool());
        assert!(!b.is_none());

        let n = ValueWord::none();
        assert!(!n.is_f64());
        assert!(!n.is_i64());
        assert!(!n.is_bool());
        assert!(n.is_none());

        let u = ValueWord::unit();
        assert!(u.is_unit());
        assert!(!u.is_none());

        let func = ValueWord::from_function(0);
        assert!(func.is_function());
        assert!(!func.is_f64());
    }

    // ===== Size check =====

    #[test]
    fn test_size_is_8_bytes() {
        assert_eq!(std::mem::size_of::<ValueWord>(), 8);
    }

    #[test]
    fn test_heap_value_size() {
        use crate::heap_value::HeapValue;
        let hv_size = std::mem::size_of::<HeapValue>();
        // Largest payload is TypedObject (32 bytes) or FunctionRef (String 24 + Option<Box> 8 = 32),
        // plus discriminant → ~40 bytes. Allow up to 48 for alignment padding.
        assert!(
            hv_size <= 48,
            "HeapValue grew beyond expected 48 bytes: {} bytes",
            hv_size
        );
    }

    // ===== Debug output =====

    #[test]
    fn test_debug_format() {
        let v = ValueWord::from_f64(3.14);
        let dbg = format!("{:?}", ValueWordDisplay(v));
        assert!(dbg.contains("f64"));
        assert!(dbg.contains("3.14"));

        let v = ValueWord::from_i64(42);
        let dbg = format!("{:?}", ValueWordDisplay(v));
        assert!(dbg.contains("i64"));
        assert!(dbg.contains("42"));

        let v = ValueWord::none();
        let dbg = format!("{:?}", ValueWordDisplay(v));
        assert!(dbg.contains("None"));
    }

    // ===== Edge: sign extension correctness =====

    #[test]
    fn test_sign_extension_negative_one() {
        let v = ValueWord::from_i64(-1);
        assert!(v.is_i64());
        assert_eq!(v.as_i64(), Some(-1));
    }

    #[test]
    fn test_sign_extension_boundary() {
        // -1 in 48-bit two's complement is 0x0000_FFFF_FFFF_FFFF (all 48 bits set)
        let v = ValueWord::from_i64(-1);
        let payload = get_payload(v);
        assert_eq!(payload, 0x0000_FFFF_FFFF_FFFF);
        assert_eq!(sign_extend_i48(payload), -1);

        // Most negative i48: -2^47
        let v = ValueWord::from_i64(I48_MIN);
        let payload = get_payload(v);
        // Bit 47 should be set, bits 46-0 should be 0
        assert_eq!(payload, 0x0000_8000_0000_0000);
        assert_eq!(sign_extend_i48(payload), I48_MIN);
    }

    // ===== Drop safety =====

    #[test]
    fn test_drop_non_heap_is_noop() {
        // These should drop without issue (no heap allocation).
        let _ = ValueWord::from_f64(1.0);
        let _ = ValueWord::from_i64(1);
        let _ = ValueWord::from_bool(true);
        let _ = ValueWord::none();
        let _ = ValueWord::unit();
        let _ = ValueWord::from_function(0);
    }

    #[test]
    fn test_drop_heap_frees_memory() {
        // Create a heap value and let it drop — should not leak or crash.
        let _v = ValueWord::from_string(Arc::new("drop test".to_string()));
        // Dropped here — if Drop is wrong, ASAN/MSAN would catch it.
    }

    #[test]
    fn test_multiple_clones_and_drops() {
        let v1 = ValueWord::from_string(Arc::new("multi clone".to_string()));
        let v2 = v1.clone();
        let v3 = v2.clone();

        assert_eq!(v1.as_arc_string().map(|s| s.as_str()), Some("multi clone"));
        assert_eq!(v2.as_arc_string().map(|s| s.as_str()), Some("multi clone"));
        assert_eq!(v3.as_arc_string().map(|s| s.as_str()), Some("multi clone"));

        // All three drop independently without double-free.
        drop(v2);
        assert_eq!(v1.as_arc_string().map(|s| s.as_str()), Some("multi clone"));
        assert_eq!(v3.as_arc_string().map(|s| s.as_str()), Some("multi clone"));
    }

    // ===== DateTime<FixedOffset> round-trips =====

    #[test]
    fn test_datetime_fixed_offset_roundtrip() {
        use chrono::TimeZone;
        let offset = chrono::FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap(); // +05:30
        let dt = offset.with_ymd_and_hms(2024, 6, 15, 14, 30, 0).unwrap();
        let v = ValueWord::from_time(dt);
        let extracted = v.as_time().unwrap();
        assert_eq!(extracted, dt);
        assert_eq!(extracted.offset().local_minus_utc(), 5 * 3600 + 30 * 60);
    }

    #[test]
    fn test_datetime_utc_convenience() {
        use chrono::TimeZone;
        let utc_dt = chrono::Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();
        let v = ValueWord::from_time_utc(utc_dt);
        let extracted = v.as_time().unwrap();
        assert_eq!(extracted.offset().local_minus_utc(), 0);
        assert_eq!(extracted.timestamp(), utc_dt.timestamp());
    }

    #[test]
    fn test_as_datetime_returns_ref() {
        use chrono::TimeZone;
        let offset = chrono::FixedOffset::west_opt(4 * 3600).unwrap(); // -04:00
        let dt = offset.with_ymd_and_hms(2024, 12, 25, 8, 0, 0).unwrap();
        let v = ValueWord::from_time(dt);
        let dt_ref = v.as_datetime().unwrap();
        assert_eq!(*dt_ref, dt);
        assert_eq!(dt_ref.offset().local_minus_utc(), -4 * 3600);
    }

    #[test]
    fn test_datetime_display() {
        use chrono::TimeZone;
        let utc_dt = chrono::Utc.with_ymd_and_hms(2024, 3, 1, 12, 0, 0).unwrap();
        let v = ValueWord::from_time_utc(utc_dt);
        let display = format!("{}", ValueWordDisplay(v));
        assert!(display.contains("2024-03-01"));
    }

    #[test]
    fn test_as_number_coerce_rejects_native_i64_u64() {
        let i64_nb = ValueWord::from_native_scalar(NativeScalar::I64(42));
        let u64_nb = ValueWord::from_native_u64(u64::MAX);
        assert_eq!(i64_nb.as_number_coerce(), None);
        assert_eq!(u64_nb.as_number_coerce(), None);
    }

    #[test]
    fn test_as_number_coerce_accepts_native_f32() {
        let v = ValueWord::from_native_f32(12.5);
        assert_eq!(v.as_number_coerce(), Some(12.5));
    }

    #[test]
    fn test_exact_integer_extractors_cover_u64() {
        let v = ValueWord::from_native_u64(u64::MAX);
        assert_eq!(v.as_u64_value(), Some(u64::MAX));
        assert_eq!(v.as_i128_exact(), Some(u64::MAX as i128));
    }

    // ===== Typed array (IntArray/FloatArray/BoolArray) tests =====

    #[test]
    fn test_int_array_construction_and_roundtrip() {
        let data = vec![1i64, 2, 3, -100, 0];
        let nb = ValueWord::from_int_array(Arc::new(data.clone().into()));
        assert!(nb.is_heap());
        let arr = nb.as_int_array().unwrap();
        assert_eq!(arr.as_slice(), &data);
    }

    #[test]
    fn test_float_array_construction_and_roundtrip() {
        use crate::aligned_vec::AlignedVec;
        let mut aligned = AlignedVec::with_capacity(4);
        aligned.push(1.0);
        aligned.push(2.5);
        aligned.push(-3.14);
        aligned.push(0.0);
        let nb = ValueWord::from_float_array(Arc::new(aligned.into()));
        assert!(nb.is_heap());
        let arr = nb.as_float_array().unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0], 1.0);
        assert_eq!(arr[2], -3.14);
    }

    #[test]
    fn test_bool_array_construction_and_roundtrip() {
        let data = vec![1u8, 0, 1, 1, 0];
        let nb = ValueWord::from_bool_array(Arc::new(data.clone().into()));
        assert!(nb.is_heap());
        let arr = nb.as_bool_array().unwrap();
        assert_eq!(arr.as_slice(), &data);
    }

    #[test]
    fn test_int_array_type_name() {
        let nb = ValueWord::from_int_array(Arc::new(vec![1i64, 2, 3].into()));
        assert_eq!(nb.type_name(), "Vec<int>");
    }

    #[test]
    fn test_float_array_type_name() {
        use crate::aligned_vec::AlignedVec;
        let mut aligned = AlignedVec::with_capacity(1);
        aligned.push(1.0);
        let nb = ValueWord::from_float_array(Arc::new(aligned.into()));
        assert_eq!(nb.type_name(), "Vec<number>");
    }

    #[test]
    fn test_bool_array_type_name() {
        let nb = ValueWord::from_bool_array(Arc::new(vec![0u8, 1].into()));
        assert_eq!(nb.type_name(), "Vec<bool>");
    }

    #[test]
    fn test_int_array_is_truthy_nonempty() {
        let nb = ValueWord::from_int_array(Arc::new(vec![42i64].into()));
        assert!(nb.is_truthy());
    }

    #[test]
    fn test_int_array_is_truthy_empty() {
        let nb = ValueWord::from_int_array(Arc::new(Vec::<i64>::new().into()));
        assert!(!nb.is_truthy());
    }

    #[test]
    fn test_float_array_is_truthy_nonempty() {
        use crate::aligned_vec::AlignedVec;
        let mut aligned = AlignedVec::with_capacity(1);
        aligned.push(0.0);
        let nb = ValueWord::from_float_array(Arc::new(aligned.into()));
        assert!(nb.is_truthy());
    }

    #[test]
    fn test_float_array_is_truthy_empty() {
        use crate::aligned_vec::AlignedVec;
        let nb = ValueWord::from_float_array(Arc::new(AlignedVec::new().into()));
        assert!(!nb.is_truthy());
    }

    #[test]
    fn test_bool_array_is_truthy_nonempty() {
        let nb = ValueWord::from_bool_array(Arc::new(vec![0u8].into()));
        assert!(nb.is_truthy());
    }

    #[test]
    fn test_bool_array_is_truthy_empty() {
        let nb = ValueWord::from_bool_array(Arc::new(Vec::<u8>::new().into()));
        assert!(!nb.is_truthy());
    }

    #[test]
    fn test_int_array_clone_arc_refcount() {
        let data: Arc<crate::typed_buffer::TypedBuffer<i64>> = Arc::new(vec![10i64, 20, 30].into());
        let nb1 = ValueWord::from_int_array(data.clone());
        let nb2 = nb1.clone();
        let arr1 = nb1.as_int_array().unwrap();
        let arr2 = nb2.as_int_array().unwrap();
        assert_eq!(arr1.as_ref(), arr2.as_ref());
        assert!(Arc::ptr_eq(arr1, arr2));
    }

    #[test]
    fn test_float_array_clone_arc_refcount() {
        use crate::aligned_vec::AlignedVec;
        let mut aligned = AlignedVec::with_capacity(2);
        aligned.push(1.0);
        aligned.push(2.0);
        let data: Arc<crate::typed_buffer::AlignedTypedBuffer> = Arc::new(aligned.into());
        let nb1 = ValueWord::from_float_array(data.clone());
        let nb2 = nb1.clone();
        let arr1 = nb1.as_float_array().unwrap();
        let arr2 = nb2.as_float_array().unwrap();
        assert!(Arc::ptr_eq(arr1, arr2));
    }

    #[test]
    fn test_typed_array_len() {
        let int_nb = ValueWord::from_int_array(Arc::new(vec![1i64, 2, 3].into()));
        assert_eq!(int_nb.typed_array_len(), Some(3));

        use crate::aligned_vec::AlignedVec;
        let mut aligned = AlignedVec::with_capacity(2);
        aligned.push(1.0);
        aligned.push(2.0);
        let float_nb = ValueWord::from_float_array(Arc::new(aligned.into()));
        assert_eq!(float_nb.typed_array_len(), Some(2));

        let bool_nb = ValueWord::from_bool_array(Arc::new(vec![0u8, 1, 1, 0].into()));
        assert_eq!(bool_nb.typed_array_len(), Some(4));

        let number_nb = ValueWord::from_f64(42.0);
        assert_eq!(number_nb.typed_array_len(), None);
    }

    #[test]
    fn test_coerce_to_float_array_from_float() {
        use crate::aligned_vec::AlignedVec;
        let mut aligned = AlignedVec::with_capacity(3);
        aligned.push(1.0);
        aligned.push(2.0);
        aligned.push(3.0);
        let nb = ValueWord::from_float_array(Arc::new(aligned.into()));
        let result = nb.coerce_to_float_array().unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], 1.0);
    }

    #[test]
    fn test_coerce_to_float_array_from_int() {
        let nb = ValueWord::from_int_array(Arc::new(vec![10i64, 20, 30].into()));
        let result = nb.coerce_to_float_array().unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], 10.0);
        assert_eq!(result[1], 20.0);
        assert_eq!(result[2], 30.0);
    }

    #[test]
    fn test_to_generic_array_int() {
        let nb = ValueWord::from_int_array(Arc::new(vec![5i64, 10].into()));
        let generic = nb.to_generic_array().unwrap();
        assert_eq!(generic.len(), 2);
        assert_eq!(generic[0].as_i64(), Some(5));
        assert_eq!(generic[1].as_i64(), Some(10));
    }

    #[test]
    fn test_to_generic_array_float() {
        use crate::aligned_vec::AlignedVec;
        let mut aligned = AlignedVec::with_capacity(2);
        aligned.push(1.5);
        aligned.push(2.5);
        let nb = ValueWord::from_float_array(Arc::new(aligned.into()));
        let generic = nb.to_generic_array().unwrap();
        assert_eq!(generic.len(), 2);
        assert_eq!(generic[0].as_f64(), Some(1.5));
        assert_eq!(generic[1].as_f64(), Some(2.5));
    }

    #[test]
    fn test_to_generic_array_bool() {
        let nb = ValueWord::from_bool_array(Arc::new(vec![1u8, 0, 1].into()));
        let generic = nb.to_generic_array().unwrap();
        assert_eq!(generic.len(), 3);
        assert_eq!(generic[0].as_bool(), Some(true));
        assert_eq!(generic[1].as_bool(), Some(false));
        assert_eq!(generic[2].as_bool(), Some(true));
    }

    #[test]
    fn test_int_array_nb_equals() {
        let a = ValueWord::from_int_array(Arc::new(vec![1i64, 2, 3].into()));
        let b = ValueWord::from_int_array(Arc::new(vec![1i64, 2, 3].into()));
        let c = ValueWord::from_int_array(Arc::new(vec![1i64, 2, 4].into()));
        assert!(a.vw_equals(&b));
        assert!(!a.vw_equals(&c));
    }

    #[test]
    fn test_float_array_nb_equals() {
        use crate::aligned_vec::AlignedVec;
        let mut a_data = AlignedVec::with_capacity(2);
        a_data.push(1.0);
        a_data.push(2.0);
        let mut b_data = AlignedVec::with_capacity(2);
        b_data.push(1.0);
        b_data.push(2.0);
        let a = ValueWord::from_float_array(Arc::new(a_data.into()));
        let b = ValueWord::from_float_array(Arc::new(b_data.into()));
        assert!(a.vw_equals(&b));
    }

    #[test]
    fn test_cross_type_accessor_returns_none() {
        let int_nb = ValueWord::from_int_array(Arc::new(vec![1i64, 2].into()));
        assert!(int_nb.as_float_array().is_none());
        assert!(int_nb.as_bool_array().is_none());
        assert!(int_nb.as_array().is_none());

        use crate::aligned_vec::AlignedVec;
        let float_nb = ValueWord::from_float_array(Arc::new(AlignedVec::new().into()));
        assert!(float_nb.as_int_array().is_none());

        let bool_nb = ValueWord::from_bool_array(Arc::new(Vec::<u8>::new().into()));
        assert!(bool_nb.as_int_array().is_none());
        assert!(bool_nb.as_float_array().is_none());
    }

    // ===== to_json_value for typed arrays =====

    #[test]
    fn test_to_json_value_int_array() {
        let buf = crate::typed_buffer::TypedBuffer {
            data: vec![1i64, 2, 3],
            validity: None,
        };
        let v = ValueWord::from_int_array(Arc::new(buf));
        let json = v.to_json_value();
        assert_eq!(json, serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn test_to_json_value_float_array() {
        use crate::aligned_vec::AlignedVec;
        let mut av = AlignedVec::new();
        av.push(1.5);
        av.push(2.5);
        let buf = crate::typed_buffer::AlignedTypedBuffer {
            data: av,
            validity: None,
        };
        let v = ValueWord::from_float_array(Arc::new(buf));
        let json = v.to_json_value();
        assert_eq!(json, serde_json::json!([1.5, 2.5]));
    }

    #[test]
    fn test_to_json_value_bool_array() {
        let buf = crate::typed_buffer::TypedBuffer {
            data: vec![1u8, 0, 1],
            validity: None,
        };
        let v = ValueWord::from_bool_array(Arc::new(buf));
        let json = v.to_json_value();
        assert_eq!(json, serde_json::json!([true, false, true]));
    }

    #[test]
    fn test_to_json_value_empty_int_array() {
        let buf = crate::typed_buffer::TypedBuffer::<i64> {
            data: vec![],
            validity: None,
        };
        let v = ValueWord::from_int_array(Arc::new(buf));
        let json = v.to_json_value();
        assert_eq!(json, serde_json::json!([]));
    }

    #[test]
    fn test_to_json_value_i32_array() {
        let buf = crate::typed_buffer::TypedBuffer {
            data: vec![10i32, 20, 30],
            validity: None,
        };
        let v = ValueWord::heap_box(HeapValue::I32Array(Arc::new(buf)));
        let json = v.to_json_value();
        assert_eq!(json, serde_json::json!([10, 20, 30]));
    }

    #[test]
    fn test_to_json_value_u64_array() {
        let buf = crate::typed_buffer::TypedBuffer {
            data: vec![100u64, 200],
            validity: None,
        };
        let v = ValueWord::heap_box(HeapValue::U64Array(Arc::new(buf)));
        let json = v.to_json_value();
        assert_eq!(json, serde_json::json!([100, 200]));
    }

    #[test]
    fn test_to_json_value_f32_array() {
        let buf = crate::typed_buffer::TypedBuffer {
            data: vec![1.0f32, 2.0],
            validity: None,
        };
        let v = ValueWord::heap_box(HeapValue::F32Array(Arc::new(buf)));
        let json = v.to_json_value();
        assert_eq!(json, serde_json::json!([1.0, 2.0]));
    }
}
