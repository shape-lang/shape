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

/// Single source of truth for NanTag variants and their inline type dispatch.
///
/// Generates:
/// - `NanTag` enum
/// - `nan_tag_type_name(tag)` — type name string for an inline (non-F64, non-Heap) tag
/// - `nan_tag_is_truthy(tag, payload)` — truthiness for an inline (non-F64, non-Heap) tag
///
/// F64 is handled before the tag match (via `!is_tagged()`), and Heap delegates
/// to HeapValue. Both are kept out of the inline dispatch.
macro_rules! define_nan_tag_types {
    () => {
        /// Tag discriminator for ValueWord values.
        ///
        /// Returned by `ValueWord::tag()` for fast type dispatch without materializing HeapValue.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum NanTag {
            /// Inline f64 (not tagged — uses the full 64-bit IEEE 754 representation)
            F64,
            /// Inline i48 (48-bit signed integer, covers [-2^47, 2^47-1])
            I48,
            /// Inline bool
            Bool,
            /// None (null)
            None,
            /// Unit (void return)
            Unit,
            /// Function reference (inline u16 function ID)
            Function,
            /// Module function reference (inline u32 index)
            ModuleFunction,
            /// Heap-allocated HeapValue (String, Array, TypedObject, Closure, etc.)
            Heap,
            /// Reference to a stack slot (absolute index, used for pass-by-reference)
            Ref,
        }

        /// Map a raw tag constant to its type name string.
        ///
        /// Covers inline tags only; F64 and Heap are handled separately by callers.
        #[inline]
        fn nan_tag_type_name(tag: u64) -> &'static str {
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
        ///
        /// Covers inline tags only; F64 and Heap are handled separately by callers.
        #[inline]
        fn nan_tag_is_truthy(tag: u64, payload: u64) -> bool {
            match tag {
                TAG_INT => sign_extend_i48(payload) != 0,
                TAG_BOOL => payload != 0,
                TAG_NONE => false,
                TAG_UNIT => false,
                TAG_FUNCTION | TAG_MODULE_FN | TAG_REF => true,
                _ => true,
            }
        }
    };
}

define_nan_tag_types!();

// ===== ArrayView: unified read-only view over all array variants =====

/// Unified read-only view over all array variants (generic, int, float, bool, width-specific).
///
/// Returned by `ValueWord::as_any_array()`. Use typed fast-path methods
/// (`as_f64_slice()`, `as_i64_slice()`) for hot paths, or `to_generic()`
/// / `get_nb()` for cold paths that need ValueWord values.
#[derive(Debug)]
pub enum ArrayView<'a> {
    Generic(&'a Arc<Vec<ValueWord>>),
    Int(&'a Arc<crate::typed_buffer::TypedBuffer<i64>>),
    Float(&'a Arc<crate::typed_buffer::AlignedTypedBuffer>),
    Bool(&'a Arc<crate::typed_buffer::TypedBuffer<u8>>),
    I8(&'a Arc<crate::typed_buffer::TypedBuffer<i8>>),
    I16(&'a Arc<crate::typed_buffer::TypedBuffer<i16>>),
    I32(&'a Arc<crate::typed_buffer::TypedBuffer<i32>>),
    U8(&'a Arc<crate::typed_buffer::TypedBuffer<u8>>),
    U16(&'a Arc<crate::typed_buffer::TypedBuffer<u16>>),
    U32(&'a Arc<crate::typed_buffer::TypedBuffer<u32>>),
    U64(&'a Arc<crate::typed_buffer::TypedBuffer<u64>>),
    F32(&'a Arc<crate::typed_buffer::TypedBuffer<f32>>),
}

impl<'a> ArrayView<'a> {
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            ArrayView::Generic(a) => a.len(),
            ArrayView::Int(a) => a.len(),
            ArrayView::Float(a) => a.len(),
            ArrayView::Bool(a) => a.len(),
            ArrayView::I8(a) => a.len(),
            ArrayView::I16(a) => a.len(),
            ArrayView::I32(a) => a.len(),
            ArrayView::U8(a) => a.len(),
            ArrayView::U16(a) => a.len(),
            ArrayView::U32(a) => a.len(),
            ArrayView::U64(a) => a.len(),
            ArrayView::F32(a) => a.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get element at index as ValueWord (boxes typed elements — use for cold paths).
    #[inline]
    pub fn get_nb(&self, idx: usize) -> Option<ValueWord> {
        match self {
            ArrayView::Generic(a) => a.get(idx).cloned(),
            ArrayView::Int(a) => a.get(idx).map(|&i| ValueWord::from_i64(i)),
            ArrayView::Float(a) => a.get(idx).map(|&f| ValueWord::from_f64(f)),
            ArrayView::Bool(a) => a.get(idx).map(|&b| ValueWord::from_bool(b != 0)),
            ArrayView::I8(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::I16(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::I32(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::U8(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::U16(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::U32(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::U64(a) => a.get(idx).map(|&v| {
                if v <= i64::MAX as u64 {
                    ValueWord::from_i64(v as i64)
                } else {
                    ValueWord::from_native_u64(v)
                }
            }),
            ArrayView::F32(a) => a.get(idx).map(|&v| ValueWord::from_f64(v as f64)),
        }
    }

    #[inline]
    pub fn first_nb(&self) -> Option<ValueWord> {
        self.get_nb(0)
    }

    #[inline]
    pub fn last_nb(&self) -> Option<ValueWord> {
        if self.is_empty() {
            None
        } else {
            self.get_nb(self.len() - 1)
        }
    }

    /// Materialize into a generic ValueWord array. Cheap Arc clone for Generic variant.
    pub fn to_generic(&self) -> Arc<Vec<ValueWord>> {
        match self {
            ArrayView::Generic(a) => (*a).clone(),
            ArrayView::Int(a) => Arc::new(a.iter().map(|&i| ValueWord::from_i64(i)).collect()),
            ArrayView::Float(a) => Arc::new(
                a.as_slice()
                    .iter()
                    .map(|&f| ValueWord::from_f64(f))
                    .collect(),
            ),
            ArrayView::Bool(a) => {
                Arc::new(a.iter().map(|&b| ValueWord::from_bool(b != 0)).collect())
            }
            ArrayView::I8(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::I16(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::I32(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::U8(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::U16(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::U32(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::U64(a) => Arc::new(
                a.iter()
                    .map(|&v| {
                        if v <= i64::MAX as u64 {
                            ValueWord::from_i64(v as i64)
                        } else {
                            ValueWord::from_native_u64(v)
                        }
                    })
                    .collect(),
            ),
            ArrayView::F32(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_f64(v as f64)).collect())
            }
        }
    }

    #[inline]
    pub fn as_i64_slice(&self) -> Option<&[i64]> {
        if let ArrayView::Int(a) = self {
            Some(a.as_slice())
        } else {
            None
        }
    }

    #[inline]
    pub fn as_f64_slice(&self) -> Option<&[f64]> {
        if let ArrayView::Float(a) = self {
            Some(a.as_slice())
        } else {
            None
        }
    }

    #[inline]
    pub fn as_bool_slice(&self) -> Option<&[u8]> {
        if let ArrayView::Bool(a) = self {
            Some(a.as_slice())
        } else {
            None
        }
    }

    #[inline]
    pub fn as_generic(&self) -> Option<&Arc<Vec<ValueWord>>> {
        if let ArrayView::Generic(a) = self {
            Some(a)
        } else {
            None
        }
    }

    #[inline]
    pub fn iter_i64(&self) -> Option<std::slice::Iter<'_, i64>> {
        if let ArrayView::Int(a) = self {
            Some(a.iter())
        } else {
            None
        }
    }

    #[inline]
    pub fn iter_f64(&self) -> Option<std::slice::Iter<'_, f64>> {
        if let ArrayView::Float(a) = self {
            Some(a.as_slice().iter())
        } else {
            None
        }
    }
}

/// Mutable view over all array variants. Uses Arc::make_mut for COW semantics.
pub enum ArrayViewMut<'a> {
    Generic(&'a mut Arc<Vec<ValueWord>>),
    Int(&'a mut Arc<crate::typed_buffer::TypedBuffer<i64>>),
    Float(&'a mut Arc<crate::typed_buffer::AlignedTypedBuffer>),
    Bool(&'a mut Arc<crate::typed_buffer::TypedBuffer<u8>>),
}

impl ArrayViewMut<'_> {
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            ArrayViewMut::Generic(a) => a.len(),
            ArrayViewMut::Int(a) => a.len(),
            ArrayViewMut::Float(a) => a.len(),
            ArrayViewMut::Bool(a) => a.len(),
        }
    }

    pub fn pop_vw(&mut self) -> Option<ValueWord> {
        match self {
            ArrayViewMut::Generic(a) => Arc::make_mut(a).pop(),
            ArrayViewMut::Int(a) => Arc::make_mut(a).data.pop().map(ValueWord::from_i64),
            ArrayViewMut::Float(a) => Arc::make_mut(a).pop().map(ValueWord::from_f64),
            ArrayViewMut::Bool(a) => Arc::make_mut(a)
                .data
                .pop()
                .map(|b| ValueWord::from_bool(b != 0)),
        }
    }

    pub fn push_vw(&mut self, val: ValueWord) -> Result<(), crate::context::VMError> {
        match self {
            ArrayViewMut::Generic(a) => {
                Arc::make_mut(a).push(val);
                Ok(())
            }
            ArrayViewMut::Int(a) => {
                if let Some(i) = val.as_i64() {
                    Arc::make_mut(a).push(i);
                    Ok(())
                } else {
                    Err(crate::context::VMError::TypeError {
                        expected: "int",
                        got: val.type_name(),
                    })
                }
            }
            ArrayViewMut::Float(a) => {
                if let Some(f) = val.as_number_coerce() {
                    Arc::make_mut(a).push(f);
                    Ok(())
                } else {
                    Err(crate::context::VMError::TypeError {
                        expected: "number",
                        got: val.type_name(),
                    })
                }
            }
            ArrayViewMut::Bool(a) => {
                if let Some(b) = val.as_bool() {
                    Arc::make_mut(a).push(if b { 1 } else { 0 });
                    Ok(())
                } else {
                    Err(crate::context::VMError::TypeError {
                        expected: "bool",
                        got: val.type_name(),
                    })
                }
            }
        }
    }
}

/// An 8-byte value word for the VM stack (NaN-boxed encoding).
///
/// This is NOT Copy because heap-tagged values reference an `Arc<HeapValue>`.
/// Clone is implemented manually to bump the Arc refcount (no allocation).
/// Drop is implemented to decrement the Arc refcount.
#[repr(transparent)]
pub struct ValueWord(u64);

impl ValueWord {
    // ===== Constructors =====

    /// Create a ValueWord from an f64 value.
    ///
    /// Normal f64 values are stored directly. NaN values are canonicalized to a single
    /// canonical NaN to avoid collisions with our tagged range.
    #[inline]
    pub fn from_f64(v: f64) -> Self {
        let bits = v.to_bits();
        if v.is_nan() {
            // Canonicalize all NaN variants to one known NaN outside our tagged range.
            Self(CANONICAL_NAN)
        } else if is_tagged(bits) {
            // Extremely rare: a valid non-NaN f64 whose bits happen to fall in our tagged range.
            // This cannot actually happen because all values in 0x7FFC..0x7FFF range are NaN
            // (exponent all 1s with non-zero mantissa). So this branch is dead code, but kept
            // for safety.
            Self(CANONICAL_NAN)
        } else {
            Self(bits)
        }
    }

    /// Create a ValueWord from an i64 value.
    ///
    /// Values in the range [-2^47, 2^47-1] are stored inline as i48.
    /// Values outside that range are heap-boxed as `HeapValue::BigInt`.
    #[inline]
    pub fn from_i64(v: i64) -> Self {
        if v >= I48_MIN && v <= I48_MAX {
            // Fits in 48 bits. Store as sign-extended i48.
            // Truncate to 48 bits by masking with PAYLOAD_MASK.
            let payload = (v as u64) & PAYLOAD_MASK;
            Self(make_tagged(TAG_INT, payload))
        } else {
            // Too large for inline. Heap-box as BigInt.
            Self::heap_box(HeapValue::BigInt(v))
        }
    }

    /// Create a ValueWord from a width-aware native scalar.
    #[inline]
    pub fn from_native_scalar(value: NativeScalar) -> Self {
        Self::heap_box(HeapValue::NativeScalar(value))
    }

    #[inline]
    pub fn from_native_i8(v: i8) -> Self {
        Self::from_native_scalar(NativeScalar::I8(v))
    }

    #[inline]
    pub fn from_native_u8(v: u8) -> Self {
        Self::from_native_scalar(NativeScalar::U8(v))
    }

    #[inline]
    pub fn from_native_i16(v: i16) -> Self {
        Self::from_native_scalar(NativeScalar::I16(v))
    }

    #[inline]
    pub fn from_native_u16(v: u16) -> Self {
        Self::from_native_scalar(NativeScalar::U16(v))
    }

    #[inline]
    pub fn from_native_i32(v: i32) -> Self {
        Self::from_native_scalar(NativeScalar::I32(v))
    }

    #[inline]
    pub fn from_native_u32(v: u32) -> Self {
        Self::from_native_scalar(NativeScalar::U32(v))
    }

    #[inline]
    pub fn from_native_u64(v: u64) -> Self {
        Self::from_native_scalar(NativeScalar::U64(v))
    }

    #[inline]
    pub fn from_native_isize(v: isize) -> Self {
        Self::from_native_scalar(NativeScalar::Isize(v))
    }

    #[inline]
    pub fn from_native_usize(v: usize) -> Self {
        Self::from_native_scalar(NativeScalar::Usize(v))
    }

    #[inline]
    pub fn from_native_ptr(v: usize) -> Self {
        Self::from_native_scalar(NativeScalar::Ptr(v))
    }

    #[inline]
    pub fn from_native_f32(v: f32) -> Self {
        Self::from_native_scalar(NativeScalar::F32(v))
    }

    /// Create a pointer-backed C view.
    #[inline]
    pub fn from_c_view(ptr: usize, layout: Arc<NativeTypeLayout>) -> Self {
        Self::heap_box(HeapValue::NativeView(Box::new(NativeViewData {
            ptr,
            layout,
            mutable: false,
        })))
    }

    /// Create a pointer-backed mutable C view.
    #[inline]
    pub fn from_c_mut(ptr: usize, layout: Arc<NativeTypeLayout>) -> Self {
        Self::heap_box(HeapValue::NativeView(Box::new(NativeViewData {
            ptr,
            layout,
            mutable: true,
        })))
    }

    /// Create a ValueWord from a bool.
    #[inline]
    pub fn from_bool(v: bool) -> Self {
        Self(make_tagged(TAG_BOOL, v as u64))
    }

    /// Create a ValueWord representing None.
    #[inline]
    pub fn none() -> Self {
        Self(make_tagged(TAG_NONE, 0))
    }

    /// Create a ValueWord representing Unit.
    #[inline]
    pub fn unit() -> Self {
        Self(make_tagged(TAG_UNIT, 0))
    }

    /// Construct a ValueWord from raw u64 bits, bumping the Arc refcount for
    /// heap-tagged values. This is equivalent to `Clone::clone` but works from
    /// raw bits (e.g. read via pointer arithmetic) instead of a `&ValueWord`.
    /// Create a ValueWord from a function ID.
    #[inline]
    pub fn from_function(id: u16) -> Self {
        Self(make_tagged(TAG_FUNCTION, id as u64))
    }

    /// Create a ValueWord from a module function index.
    #[inline]
    pub fn from_module_function(index: u32) -> Self {
        Self(make_tagged(TAG_MODULE_FN, index as u64))
    }

    /// Create a ValueWord reference to an absolute stack slot.
    #[inline]
    pub fn from_ref(absolute_slot: usize) -> Self {
        Self(make_tagged(TAG_REF, absolute_slot as u64))
    }

    /// Create a ValueWord reference to a module binding slot.
    #[inline]
    pub fn from_module_binding_ref(binding_idx: usize) -> Self {
        Self(make_tagged(
            TAG_REF,
            REF_TARGET_MODULE_FLAG | (binding_idx as u64 & REF_TARGET_INDEX_MASK),
        ))
    }

    /// Create a projected reference backed by heap metadata.
    #[inline]
    pub fn from_projected_ref(base: ValueWord, projection: RefProjection) -> Self {
        Self::heap_box(HeapValue::ProjectedRef(Box::new(ProjectedRefData {
            base,
            projection,
        })))
    }

    /// Heap-box a HeapValue directly.
    ///
    /// Under the `gc` feature, allocates via the GC heap (bump allocator, no refcount).
    /// Without `gc`, uses `Arc<HeapValue>` with refcount bump on clone.
    #[inline]
    #[cfg(not(feature = "gc"))]
    pub(crate) fn heap_box(v: HeapValue) -> Self {
        let arc = Arc::new(v);
        let ptr = Arc::into_raw(arc) as u64;
        debug_assert!(
            ptr & !PAYLOAD_MASK == 0,
            "pointer exceeds 48 bits — platform not supported"
        );
        Self(make_tagged(TAG_HEAP, ptr & PAYLOAD_MASK))
    }

    /// Heap-box a HeapValue via the GC heap (bump allocation, no refcount).
    ///
    /// Clone is a bitwise copy (no refcount). Drop is a no-op (GC handles deallocation).
    #[inline]
    #[cfg(feature = "gc")]
    pub(crate) fn heap_box(v: HeapValue) -> Self {
        let heap = shape_gc::thread_gc_heap();
        let ptr = heap.alloc(v) as u64;
        debug_assert!(
            ptr & !PAYLOAD_MASK == 0,
            "GC pointer exceeds 48 bits — platform not supported"
        );
        Self(make_tagged(TAG_HEAP, ptr & PAYLOAD_MASK))
    }

    // ===== Typed constructors =====

    /// Create a ValueWord from an Arc<String>.
    #[inline]
    pub fn from_string(s: Arc<String>) -> Self {
        Self::heap_box(HeapValue::String(s))
    }

    /// Create a ValueWord from a char.
    #[inline]
    pub fn from_char(c: char) -> Self {
        Self::heap_box(HeapValue::Char(c))
    }

    /// Extract a char if this is a HeapValue::Char.
    #[inline]
    pub fn as_char(&self) -> Option<char> {
        if let Some(HeapValue::Char(c)) = self.as_heap_ref() {
            Some(*c)
        } else {
            std::option::Option::None
        }
    }

    /// Create a ValueWord from a VMArray directly (no intermediate conversion).
    #[inline]
    pub fn from_array(a: crate::value::VMArray) -> Self {
        Self::heap_box(HeapValue::Array(a))
    }

    /// Create a ValueWord from Decimal directly (no intermediate conversion).
    #[inline]
    pub fn from_decimal(d: rust_decimal::Decimal) -> Self {
        Self::heap_box(HeapValue::Decimal(d))
    }

    /// Create a ValueWord from any HeapValue directly (no intermediate conversion).
    ///
    /// BigInt that fits i48 is unwrapped to its native ValueWord inline tag instead
    /// of being heap-allocated. All other variants are heap-boxed.
    #[inline]
    pub fn from_heap_value(v: HeapValue) -> Self {
        match v {
            HeapValue::BigInt(i) => Self::from_i64(i),
            other => Self::heap_box(other),
        }
    }

    // ===== DataTable family constructors =====

    /// Create a ValueWord from a DataTable directly.
    #[inline]
    pub fn from_datatable(dt: Arc<DataTable>) -> Self {
        Self::heap_box(HeapValue::DataTable(dt))
    }

    /// Create a ValueWord TypedTable directly.
    #[inline]
    pub fn from_typed_table(schema_id: u64, table: Arc<DataTable>) -> Self {
        Self::heap_box(HeapValue::TypedTable { schema_id, table })
    }

    /// Create a ValueWord RowView directly.
    #[inline]
    pub fn from_row_view(schema_id: u64, table: Arc<DataTable>, row_idx: usize) -> Self {
        Self::heap_box(HeapValue::RowView {
            schema_id,
            table,
            row_idx,
        })
    }

    /// Create a ValueWord ColumnRef directly.
    #[inline]
    pub fn from_column_ref(schema_id: u64, table: Arc<DataTable>, col_id: u32) -> Self {
        Self::heap_box(HeapValue::ColumnRef {
            schema_id,
            table,
            col_id,
        })
    }

    /// Create a ValueWord IndexedTable directly.
    #[inline]
    pub fn from_indexed_table(schema_id: u64, table: Arc<DataTable>, index_col: u32) -> Self {
        Self::heap_box(HeapValue::IndexedTable {
            schema_id,
            table,
            index_col,
        })
    }

    // ===== Container / wrapper constructors =====

    /// Create a ValueWord Range directly.
    #[inline]
    pub fn from_range(start: Option<ValueWord>, end: Option<ValueWord>, inclusive: bool) -> Self {
        Self::heap_box(HeapValue::Range {
            start: start.map(Box::new),
            end: end.map(Box::new),
            inclusive,
        })
    }

    /// Create a ValueWord Enum directly.
    #[inline]
    pub fn from_enum(e: EnumValue) -> Self {
        Self::heap_box(HeapValue::Enum(Box::new(e)))
    }

    /// Create a ValueWord Some directly.
    #[inline]
    pub fn from_some(inner: ValueWord) -> Self {
        Self::heap_box(HeapValue::Some(Box::new(inner)))
    }

    /// Create a ValueWord Ok directly.
    #[inline]
    pub fn from_ok(inner: ValueWord) -> Self {
        Self::heap_box(HeapValue::Ok(Box::new(inner)))
    }

    /// Create a ValueWord Err directly.
    #[inline]
    pub fn from_err(inner: ValueWord) -> Self {
        Self::heap_box(HeapValue::Err(Box::new(inner)))
    }

    // ===== HashMap constructors =====

    /// Create a ValueWord HashMap from keys, values, and index.
    #[inline]
    pub fn from_hashmap(
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
    pub fn empty_hashmap() -> ValueWord {
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
    pub fn from_hashmap_pairs(keys: Vec<ValueWord>, values: Vec<ValueWord>) -> ValueWord {
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
    pub fn from_set(items: Vec<ValueWord>) -> ValueWord {
        ValueWord::heap_box(HeapValue::Set(Box::new(SetData::from_items(items))))
    }

    /// Create an empty ValueWord Set.
    #[inline]
    pub fn empty_set() -> ValueWord {
        ValueWord::heap_box(HeapValue::Set(Box::new(SetData {
            items: Vec::new(),
            index: HashMap::new(),
        })))
    }

    // ===== Deque constructors =====

    /// Create a ValueWord Deque from items.
    #[inline]
    pub fn from_deque(items: Vec<ValueWord>) -> ValueWord {
        ValueWord::heap_box(HeapValue::Deque(Box::new(DequeData::from_items(items))))
    }

    /// Create an empty ValueWord Deque.
    #[inline]
    pub fn empty_deque() -> ValueWord {
        ValueWord::heap_box(HeapValue::Deque(Box::new(DequeData::new())))
    }

    // ===== PriorityQueue constructors =====

    /// Create a ValueWord PriorityQueue from items (heapified).
    #[inline]
    pub fn from_priority_queue(items: Vec<ValueWord>) -> ValueWord {
        ValueWord::heap_box(HeapValue::PriorityQueue(Box::new(
            PriorityQueueData::from_items(items),
        )))
    }

    /// Create an empty ValueWord PriorityQueue.
    #[inline]
    pub fn empty_priority_queue() -> ValueWord {
        ValueWord::heap_box(HeapValue::PriorityQueue(Box::new(PriorityQueueData::new())))
    }

    // ===== Content constructors =====

    /// Create a ValueWord from a ContentNode directly.
    #[inline]
    pub fn from_content(node: ContentNode) -> ValueWord {
        ValueWord::heap_box(HeapValue::Content(Box::new(node)))
    }

    // ===== Typed collection constructors =====

    /// Create a ValueWord IntArray from an Arc<TypedBuffer<i64>>.
    #[inline]
    pub fn from_int_array(a: Arc<crate::typed_buffer::TypedBuffer<i64>>) -> Self {
        Self::heap_box(HeapValue::IntArray(a))
    }

    /// Create a ValueWord FloatArray from an Arc<AlignedTypedBuffer>.
    #[inline]
    pub fn from_float_array(a: Arc<crate::typed_buffer::AlignedTypedBuffer>) -> Self {
        Self::heap_box(HeapValue::FloatArray(a))
    }

    /// Create a ValueWord BoolArray from an Arc<TypedBuffer<u8>>.
    #[inline]
    pub fn from_bool_array(a: Arc<crate::typed_buffer::TypedBuffer<u8>>) -> Self {
        Self::heap_box(HeapValue::BoolArray(a))
    }

    /// Create a ValueWord I8Array.
    #[inline]
    pub fn from_i8_array(a: Arc<crate::typed_buffer::TypedBuffer<i8>>) -> Self {
        Self::heap_box(HeapValue::I8Array(a))
    }

    /// Create a ValueWord I16Array.
    #[inline]
    pub fn from_i16_array(a: Arc<crate::typed_buffer::TypedBuffer<i16>>) -> Self {
        Self::heap_box(HeapValue::I16Array(a))
    }

    /// Create a ValueWord I32Array.
    #[inline]
    pub fn from_i32_array(a: Arc<crate::typed_buffer::TypedBuffer<i32>>) -> Self {
        Self::heap_box(HeapValue::I32Array(a))
    }

    /// Create a ValueWord U8Array.
    #[inline]
    pub fn from_u8_array(a: Arc<crate::typed_buffer::TypedBuffer<u8>>) -> Self {
        Self::heap_box(HeapValue::U8Array(a))
    }

    /// Create a ValueWord U16Array.
    #[inline]
    pub fn from_u16_array(a: Arc<crate::typed_buffer::TypedBuffer<u16>>) -> Self {
        Self::heap_box(HeapValue::U16Array(a))
    }

    /// Create a ValueWord U32Array.
    #[inline]
    pub fn from_u32_array(a: Arc<crate::typed_buffer::TypedBuffer<u32>>) -> Self {
        Self::heap_box(HeapValue::U32Array(a))
    }

    /// Create a ValueWord U64Array.
    #[inline]
    pub fn from_u64_array(a: Arc<crate::typed_buffer::TypedBuffer<u64>>) -> Self {
        Self::heap_box(HeapValue::U64Array(a))
    }

    /// Create a ValueWord F32Array.
    #[inline]
    pub fn from_f32_array(a: Arc<crate::typed_buffer::TypedBuffer<f32>>) -> Self {
        Self::heap_box(HeapValue::F32Array(a))
    }

    /// Create a ValueWord Matrix from MatrixData.
    #[inline]
    pub fn from_matrix(m: Arc<crate::heap_value::MatrixData>) -> Self {
        Self::heap_box(HeapValue::Matrix(m))
    }

    /// Create a ValueWord FloatArraySlice — a zero-copy view into a parent matrix.
    #[inline]
    pub fn from_float_array_slice(
        parent: Arc<crate::heap_value::MatrixData>,
        offset: u32,
        len: u32,
    ) -> Self {
        Self::heap_box(HeapValue::FloatArraySlice {
            parent,
            offset,
            len,
        })
    }

    /// Create a ValueWord Iterator from IteratorState.
    #[inline]
    pub fn from_iterator(state: Box<crate::heap_value::IteratorState>) -> Self {
        Self::heap_box(HeapValue::Iterator(state))
    }

    /// Create a ValueWord Generator from GeneratorState.
    #[inline]
    pub fn from_generator(state: Box<crate::heap_value::GeneratorState>) -> Self {
        Self::heap_box(HeapValue::Generator(state))
    }

    // ===== Async / concurrency constructors =====

    /// Create a ValueWord Future directly.
    #[inline]
    pub fn from_future(id: u64) -> Self {
        Self::heap_box(HeapValue::Future(id))
    }

    /// Create a ValueWord TaskGroup directly.
    #[inline]
    pub fn from_task_group(kind: u8, task_ids: Vec<u64>) -> Self {
        Self::heap_box(HeapValue::TaskGroup { kind, task_ids })
    }

    /// Create a ValueWord Mutex wrapping a value.
    #[inline]
    pub fn from_mutex(value: ValueWord) -> Self {
        Self::heap_box(HeapValue::Mutex(Box::new(
            crate::heap_value::MutexData::new(value),
        )))
    }

    /// Create a ValueWord Atomic with an initial integer value.
    #[inline]
    pub fn from_atomic(value: i64) -> Self {
        Self::heap_box(HeapValue::Atomic(Box::new(
            crate::heap_value::AtomicData::new(value),
        )))
    }

    /// Create a ValueWord Lazy with an initializer closure.
    #[inline]
    pub fn from_lazy(initializer: ValueWord) -> Self {
        Self::heap_box(HeapValue::Lazy(Box::new(crate::heap_value::LazyData::new(
            initializer,
        ))))
    }

    /// Create a ValueWord Channel endpoint.
    #[inline]
    pub fn from_channel(data: ChannelData) -> Self {
        Self::heap_box(HeapValue::Channel(Box::new(data)))
    }

    // ===== Trait dispatch constructors =====

    /// Create a ValueWord TraitObject directly.
    #[inline]
    pub fn from_trait_object(value: ValueWord, vtable: Arc<VTable>) -> Self {
        Self::heap_box(HeapValue::TraitObject {
            value: Box::new(value),
            vtable,
        })
    }

    // ===== SQL pushdown constructors =====

    /// Create a ValueWord ExprProxy directly.
    #[inline]
    pub fn from_expr_proxy(col_name: Arc<String>) -> Self {
        Self::heap_box(HeapValue::ExprProxy(col_name))
    }

    /// Create a ValueWord FilterExpr directly.
    #[inline]
    pub fn from_filter_expr(node: Arc<FilterNode>) -> Self {
        Self::heap_box(HeapValue::FilterExpr(node))
    }

    // ===== Instant constructors =====

    /// Create a ValueWord Instant directly.
    #[inline]
    pub fn from_instant(t: std::time::Instant) -> Self {
        Self::heap_box(HeapValue::Instant(Box::new(t)))
    }

    // ===== IoHandle constructors =====

    /// Create a ValueWord IoHandle.
    #[inline]
    pub fn from_io_handle(data: crate::heap_value::IoHandleData) -> Self {
        Self::heap_box(HeapValue::IoHandle(Box::new(data)))
    }

    // ===== Time constructors =====

    /// Create a ValueWord Time directly from a DateTime<FixedOffset>.
    #[inline]
    pub fn from_time(t: DateTime<FixedOffset>) -> Self {
        Self::heap_box(HeapValue::Time(t))
    }

    /// Create a ValueWord Time from a DateTime<Utc> (converts to FixedOffset).
    #[inline]
    pub fn from_time_utc(t: DateTime<Utc>) -> Self {
        Self::heap_box(HeapValue::Time(t.fixed_offset()))
    }

    /// Create a ValueWord Duration directly.
    #[inline]
    pub fn from_duration(d: Duration) -> Self {
        Self::heap_box(HeapValue::Duration(d))
    }

    /// Create a ValueWord TimeSpan directly.
    #[inline]
    pub fn from_timespan(ts: chrono::Duration) -> Self {
        Self::heap_box(HeapValue::TimeSpan(ts))
    }

    /// Create a ValueWord Timeframe directly.
    #[inline]
    pub fn from_timeframe(tf: Timeframe) -> Self {
        Self::heap_box(HeapValue::Timeframe(tf))
    }

    // ===== Other constructors =====

    /// Create a ValueWord HostClosure directly.
    #[inline]
    pub fn from_host_closure(nc: HostCallable) -> Self {
        Self::heap_box(HeapValue::HostClosure(nc))
    }

    /// Create a ValueWord PrintResult directly.
    #[inline]
    pub fn from_print_result(pr: PrintResult) -> Self {
        Self::heap_box(HeapValue::PrintResult(Box::new(pr)))
    }

    /// Create a ValueWord SimulationCall directly.
    #[inline]
    pub fn from_simulation_call(name: String, params: HashMap<String, ValueWord>) -> Self {
        Self::heap_box(HeapValue::SimulationCall(Box::new(
            crate::heap_value::SimulationCallData { name, params },
        )))
    }

    /// Create a ValueWord FunctionRef directly.
    #[inline]
    pub fn from_function_ref(name: String, closure: Option<ValueWord>) -> Self {
        Self::heap_box(HeapValue::FunctionRef {
            name,
            closure: closure.map(Box::new),
        })
    }

    /// Create a ValueWord DataReference directly.
    #[inline]
    pub fn from_data_reference(
        datetime: DateTime<FixedOffset>,
        id: String,
        timeframe: Timeframe,
    ) -> Self {
        Self::heap_box(HeapValue::DataReference(Box::new(
            crate::heap_value::DataReferenceData {
                datetime,
                id,
                timeframe,
            },
        )))
    }

    /// Create a ValueWord TimeReference directly.
    #[inline]
    pub fn from_time_reference(tr: TimeReference) -> Self {
        Self::heap_box(HeapValue::TimeReference(Box::new(tr)))
    }

    /// Create a ValueWord DateTimeExpr directly.
    #[inline]
    pub fn from_datetime_expr(de: DateTimeExpr) -> Self {
        Self::heap_box(HeapValue::DateTimeExpr(Box::new(de)))
    }

    /// Create a ValueWord DataDateTimeRef directly.
    #[inline]
    pub fn from_data_datetime_ref(dr: DataDateTimeRef) -> Self {
        Self::heap_box(HeapValue::DataDateTimeRef(Box::new(dr)))
    }

    /// Create a ValueWord TypeAnnotation directly.
    #[inline]
    pub fn from_type_annotation(ta: TypeAnnotation) -> Self {
        Self::heap_box(HeapValue::TypeAnnotation(Box::new(ta)))
    }

    /// Create a ValueWord TypeAnnotatedValue directly.
    #[inline]
    pub fn from_type_annotated_value(type_name: String, value: ValueWord) -> Self {
        Self::heap_box(HeapValue::TypeAnnotatedValue {
            type_name,
            value: Box::new(value),
        })
    }

    /// Create a ValueWord by "cloning" from raw bits read from a stack slot.
    ///
    /// For inline values (f64, i48, bool, none, unit, function), this simply
    /// wraps the bits. For heap values (without `gc`), it bumps the Arc refcount.
    /// With `gc`, it's a pure bitwise copy (GC handles liveness).
    ///
    /// # Safety
    /// `bits` must be a valid ValueWord representation (either an inline value
    /// or a heap-tagged value with a valid Arc<HeapValue> / GC pointer).
    #[inline(always)]
    #[cfg(not(feature = "gc"))]
    pub unsafe fn clone_from_bits(bits: u64) -> Self {
        if is_tagged(bits) && get_tag(bits) == TAG_HEAP {
            let ptr = get_payload(bits) as *const HeapValue;
            unsafe { Arc::increment_strong_count(ptr) };
        }
        Self(bits)
    }

    /// Create a ValueWord by "cloning" from raw bits (GC path: pure bitwise copy).
    ///
    /// # Safety
    /// `bits` must be a valid ValueWord representation.
    #[inline(always)]
    #[cfg(feature = "gc")]
    pub unsafe fn clone_from_bits(bits: u64) -> Self {
        // GC path: no refcount — just copy the bits.
        Self(bits)
    }

    // ===== Type checks =====

    /// Returns true if this value is an inline f64 (not a tagged value).
    #[inline(always)]
    pub fn is_f64(&self) -> bool {
        !is_tagged(self.0)
    }

    /// Returns true if this value is an inline i48 integer.
    #[inline(always)]
    pub fn is_i64(&self) -> bool {
        is_tagged(self.0) && get_tag(self.0) == TAG_INT
    }

    /// Returns true if this value is a bool.
    #[inline(always)]
    pub fn is_bool(&self) -> bool {
        is_tagged(self.0) && get_tag(self.0) == TAG_BOOL
    }

    /// Returns true if this value is None.
    #[inline(always)]
    pub fn is_none(&self) -> bool {
        is_tagged(self.0) && get_tag(self.0) == TAG_NONE
    }

    /// Returns true if this value is Unit.
    #[inline(always)]
    pub fn is_unit(&self) -> bool {
        is_tagged(self.0) && get_tag(self.0) == TAG_UNIT
    }

    /// Returns true if this value is a function reference.
    #[inline(always)]
    pub fn is_function(&self) -> bool {
        is_tagged(self.0) && get_tag(self.0) == TAG_FUNCTION
    }

    /// Returns true if this value is a heap-boxed HeapValue.
    #[inline(always)]
    pub fn is_heap(&self) -> bool {
        is_tagged(self.0) && get_tag(self.0) == TAG_HEAP
    }

    /// Returns true if this value is a stack reference.
    #[inline(always)]
    pub fn is_ref(&self) -> bool {
        if is_tagged(self.0) {
            return get_tag(self.0) == TAG_REF;
        }
        matches!(self.as_heap_ref(), Some(HeapValue::ProjectedRef(_)))
    }

    /// Extract the reference target.
    #[inline]
    pub fn as_ref_target(&self) -> Option<RefTarget> {
        if is_tagged(self.0) && get_tag(self.0) == TAG_REF {
            let payload = get_payload(self.0);
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
    pub fn as_ref_slot(&self) -> Option<usize> {
        match self.as_ref_target() {
            Some(RefTarget::Stack(slot)) => Some(slot),
            _ => None,
        }
    }

    // ===== Checked extractors =====

    /// Extract as f64, returning None if this is not an inline f64.
    #[inline]
    pub fn as_f64(&self) -> Option<f64> {
        if self.is_f64() {
            Some(f64::from_bits(self.0))
        } else {
            None
        }
    }

    /// Extract as i64, returning None if this is not an exact signed integer.
    ///
    /// Accepts inline i48 values, heap BigInt, and signed-compatible native scalars.
    #[inline]
    pub fn as_i64(&self) -> Option<i64> {
        if self.is_i64() {
            Some(sign_extend_i48(get_payload(self.0)))
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
    pub fn as_u64(&self) -> Option<u64> {
        if self.is_i64() {
            let v = sign_extend_i48(get_payload(self.0));
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
    pub fn as_i128_exact(&self) -> Option<i128> {
        if self.is_i64() {
            return Some(sign_extend_i48(get_payload(self.0)) as i128);
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
    pub fn as_number_strict(&self) -> Option<f64> {
        if self.is_f64() {
            return Some(f64::from_bits(self.0));
        }
        // Inline i48 integers — lossless conversion to f64.
        if is_tagged(self.0) && get_tag(self.0) == TAG_INT {
            return Some(sign_extend_i48(get_payload(self.0)) as f64);
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
    pub fn as_bool(&self) -> Option<bool> {
        if self.is_bool() {
            Some(get_payload(self.0) != 0)
        } else {
            None
        }
    }

    /// Extract as function ID, returning None if this is not a function.
    #[inline]
    pub fn as_function(&self) -> Option<u16> {
        if self.is_function() {
            Some(get_payload(self.0) as u16)
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
    pub unsafe fn as_f64_unchecked(&self) -> f64 {
        if self.is_f64() {
            f64::from_bits(self.0)
        } else if is_tagged(self.0) && get_tag(self.0) == TAG_INT {
            sign_extend_i48(get_payload(self.0)) as f64
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
    pub unsafe fn as_i64_unchecked(&self) -> i64 {
        if is_tagged(self.0) && get_tag(self.0) == TAG_INT {
            sign_extend_i48(get_payload(self.0))
        } else if self.is_f64() {
            f64::from_bits(self.0) as i64
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
    pub unsafe fn as_bool_unchecked(&self) -> bool {
        debug_assert!(self.is_bool(), "as_bool_unchecked on non-bool ValueWord");
        get_payload(self.0) != 0
    }

    /// Extract function ID without type checking.
    ///
    /// # Safety
    /// Caller must ensure `self.is_function()` is true.
    #[inline(always)]
    pub unsafe fn as_function_unchecked(&self) -> u16 {
        debug_assert!(
            self.is_function(),
            "as_function_unchecked on non-function ValueWord"
        );
        get_payload(self.0) as u16
    }

    // ===== ValueWord inspection API =====

    /// Get a reference to the heap-boxed HeapValue without cloning.
    /// Returns None if this is not a heap value.
    #[inline]
    pub fn as_heap_ref(&self) -> Option<&HeapValue> {
        if is_tagged(self.0) && get_tag(self.0) == TAG_HEAP {
            let ptr = get_payload(self.0) as *const HeapValue;
            Some(unsafe { &*ptr })
        } else {
            None
        }
    }

    /// Get a mutable reference to the heap-boxed HeapValue, cloning if shared.
    /// Returns None if this is not a heap value.
    ///
    /// Without `gc`: Uses Arc::make_mut semantics (clones if refcount > 1).
    /// With `gc`: Direct mutable access (GC objects are not refcounted).
    #[inline]
    #[cfg(not(feature = "gc"))]
    pub fn as_heap_mut(&mut self) -> Option<&mut HeapValue> {
        if is_tagged(self.0) && get_tag(self.0) == TAG_HEAP {
            let ptr = get_payload(self.0) as *const HeapValue;
            // Reconstruct the Arc without dropping it (we'll consume it via make_mut).
            let mut arc = unsafe { Arc::from_raw(ptr) };
            // Ensure unique ownership (clones HeapValue if refcount > 1, no-op if == 1).
            Arc::make_mut(&mut arc);
            // Leak the Arc back into a raw pointer and update self.0
            // (make_mut may have reallocated if refcount > 1).
            let new_ptr = Arc::into_raw(arc) as u64;
            self.0 = make_tagged(TAG_HEAP, new_ptr & PAYLOAD_MASK);
            let final_ptr = get_payload(self.0) as *mut HeapValue;
            Some(unsafe { &mut *final_ptr })
        } else {
            None
        }
    }

    /// Get a mutable reference to the heap-boxed HeapValue (GC path).
    ///
    /// With GC, objects are not refcounted so we can always get a direct
    /// mutable reference without copy-on-write.
    #[inline]
    #[cfg(feature = "gc")]
    pub fn as_heap_mut(&mut self) -> Option<&mut HeapValue> {
        if is_tagged(self.0) && get_tag(self.0) == TAG_HEAP {
            let ptr = get_payload(self.0) as *mut HeapValue;
            Some(unsafe { &mut *ptr })
        } else {
            None
        }
    }

    /// Check truthiness without materializing HeapValue.
    #[inline]
    pub fn is_truthy(&self) -> bool {
        if !is_tagged(self.0) {
            // f64: truthy if non-zero and not NaN
            let f = f64::from_bits(self.0);
            return f != 0.0 && !f.is_nan();
        }
        let tag = get_tag(self.0);
        if tag == TAG_HEAP {
            let ptr = get_payload(self.0) as *const HeapValue;
            return unsafe { (*ptr).is_truthy() };
        }
        nan_tag_is_truthy(tag, get_payload(self.0))
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
    pub fn as_number_coerce(&self) -> Option<f64> {
        if let Some(n) = self.as_number_strict() {
            Some(n)
        } else if is_tagged(self.0) && get_tag(self.0) == TAG_INT {
            Some(sign_extend_i48(get_payload(self.0)) as f64)
        } else {
            None
        }
    }

    /// Check if this ValueWord is a module function tag.
    #[inline(always)]
    pub fn is_module_function(&self) -> bool {
        is_tagged(self.0) && get_tag(self.0) == TAG_MODULE_FN
    }

    /// Extract module function index.
    #[inline]
    pub fn as_module_function(&self) -> Option<usize> {
        if self.is_module_function() {
            Some(get_payload(self.0) as usize)
        } else {
            None
        }
    }

    // ===== HeapValue inspection =====

    /// Get the HeapKind discriminator without cloning.
    /// Returns None if this is not a heap value.
    #[inline]
    pub fn heap_kind(&self) -> Option<crate::heap_value::HeapKind> {
        if let Some(hv) = self.as_heap_ref() {
            Some(hv.kind())
        } else {
            std::option::Option::None
        }
    }

    // ===== Phase 2A: Extended inspection API =====

    /// Tag discriminator for ValueWord values.
    /// Used for fast dispatch without materializing HeapValue.
    #[inline]
    pub fn tag(&self) -> NanTag {
        if !is_tagged(self.0) {
            return NanTag::F64;
        }
        match get_tag(self.0) {
            TAG_INT => NanTag::I48,
            TAG_BOOL => NanTag::Bool,
            TAG_NONE => NanTag::None,
            TAG_UNIT => NanTag::Unit,
            TAG_FUNCTION => NanTag::Function,
            TAG_MODULE_FN => NanTag::ModuleFunction,
            TAG_HEAP => NanTag::Heap,
            TAG_REF => NanTag::Ref,
            _ => unreachable!("invalid ValueWord tag"),
        }
    }

    /// Get a reference to a heap String without cloning.
    /// Returns None if this is not a heap-boxed String.
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
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
    pub fn as_decimal(&self) -> Option<rust_decimal::Decimal> {
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
    pub unsafe fn as_decimal_unchecked(&self) -> rust_decimal::Decimal {
        debug_assert!(matches!(self.as_heap_ref(), Some(HeapValue::Decimal(_))));
        match unsafe { self.as_heap_ref().unwrap_unchecked() } {
            HeapValue::Decimal(d) => *d,
            _ => unsafe { std::hint::unreachable_unchecked() },
        }
    }

    /// Get a reference to a heap Array without cloning.
    /// Returns None if this is not a heap-boxed Array.
    #[inline]
    #[deprecated(note = "Use as_any_array() instead for unified typed array dispatch")]
    pub fn as_array(&self) -> Option<&VMArray> {
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
    pub fn as_any_array(&self) -> Option<ArrayView<'_>> {
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
    pub fn as_any_array_mut(&mut self) -> Option<ArrayViewMut<'_>> {
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
    pub fn as_datatable(&self) -> Option<&Arc<DataTable>> {
        match self.as_heap_ref()? {
            HeapValue::DataTable(dt) => Some(dt),
            _ => std::option::Option::None,
        }
    }

    /// Extract TypedTable fields.
    #[inline]
    pub fn as_typed_table(&self) -> Option<(u64, &Arc<DataTable>)> {
        match self.as_heap_ref()? {
            HeapValue::TypedTable { schema_id, table } => Some((*schema_id, table)),
            _ => std::option::Option::None,
        }
    }

    /// Extract RowView fields.
    #[inline]
    pub fn as_row_view(&self) -> Option<(u64, &Arc<DataTable>, usize)> {
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
    pub fn as_column_ref(&self) -> Option<(u64, &Arc<DataTable>, u32)> {
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
    pub fn as_indexed_table(&self) -> Option<(u64, &Arc<DataTable>, u32)> {
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
    pub fn as_typed_object(&self) -> Option<(u64, &[ValueSlot], u64)> {
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
    pub fn as_closure(&self) -> Option<(u16, &[crate::value::Upvalue])> {
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
    pub fn as_some_inner(&self) -> Option<&ValueWord> {
        match self.as_heap_ref()? {
            HeapValue::Some(inner) => Some(inner),
            _ => std::option::Option::None,
        }
    }

    /// Extract the inner value from an Ok variant.
    #[inline]
    pub fn as_ok_inner(&self) -> Option<&ValueWord> {
        match self.as_heap_ref()? {
            HeapValue::Ok(inner) => Some(inner),
            _ => std::option::Option::None,
        }
    }

    /// Extract the inner value from an Err variant.
    #[inline]
    pub fn as_err_inner(&self) -> Option<&ValueWord> {
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
    pub fn as_err_payload(&self) -> Option<ValueWord> {
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
    pub fn as_future(&self) -> Option<u64> {
        match self.as_heap_ref()? {
            HeapValue::Future(id) => Some(*id),
            _ => std::option::Option::None,
        }
    }

    /// Extract TraitObject fields.
    #[inline]
    pub fn as_trait_object(&self) -> Option<(&ValueWord, &Arc<VTable>)> {
        match self.as_heap_ref()? {
            HeapValue::TraitObject { value, vtable } => Some((value, vtable)),
            _ => std::option::Option::None,
        }
    }

    /// Extract ExprProxy column name.
    #[inline]
    pub fn as_expr_proxy(&self) -> Option<&Arc<String>> {
        match self.as_heap_ref()? {
            HeapValue::ExprProxy(name) => Some(name),
            _ => std::option::Option::None,
        }
    }

    /// Extract FilterExpr node.
    #[inline]
    pub fn as_filter_expr(&self) -> Option<&Arc<FilterNode>> {
        match self.as_heap_ref()? {
            HeapValue::FilterExpr(node) => Some(node),
            _ => std::option::Option::None,
        }
    }

    /// Extract a HostClosure reference.
    #[inline]
    pub fn as_host_closure(&self) -> Option<&HostCallable> {
        match self.as_heap_ref()? {
            HeapValue::HostClosure(nc) => Some(nc),
            _ => std::option::Option::None,
        }
    }

    /// Extract a Duration reference.
    #[inline]
    pub fn as_duration(&self) -> Option<&shape_ast::ast::Duration> {
        match self.as_heap_ref()? {
            HeapValue::Duration(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Extract a Range (start, end, inclusive).
    #[inline]
    pub fn as_range(&self) -> Option<(Option<&ValueWord>, Option<&ValueWord>, bool)> {
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
    pub fn as_timespan(&self) -> Option<chrono::Duration> {
        match self.as_heap_ref()? {
            HeapValue::TimeSpan(ts) => Some(*ts),
            _ => std::option::Option::None,
        }
    }

    /// Extract an EnumValue reference.
    #[inline]
    pub fn as_enum(&self) -> Option<&EnumValue> {
        match self.as_heap_ref()? {
            HeapValue::Enum(e) => Some(e.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Extract a Timeframe reference.
    #[inline]
    pub fn as_timeframe(&self) -> Option<&Timeframe> {
        match self.as_heap_ref()? {
            HeapValue::Timeframe(tf) => Some(tf),
            _ => std::option::Option::None,
        }
    }

    /// Get the HashMap contents if this is a HashMap.
    #[inline]
    pub fn as_hashmap(
        &self,
    ) -> Option<(&Vec<ValueWord>, &Vec<ValueWord>, &HashMap<u64, Vec<usize>>)> {
        match self.as_heap_ref()? {
            HeapValue::HashMap(d) => Some((&d.keys, &d.values, &d.index)),
            _ => std::option::Option::None,
        }
    }

    /// Get read-only access to the full HashMapData (includes shape_id).
    #[inline]
    pub fn as_hashmap_data(&self) -> Option<&HashMapData> {
        match self.as_heap_ref()? {
            HeapValue::HashMap(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get mutable access to the HashMapData.
    /// Uses copy-on-write via `as_heap_mut()` (clones if Arc refcount > 1).
    #[inline]
    pub fn as_hashmap_mut(&mut self) -> Option<&mut HashMapData> {
        match self.as_heap_mut()? {
            HeapValue::HashMap(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get the Set data if this is a Set.
    #[inline]
    pub fn as_set(&self) -> Option<&SetData> {
        match self.as_heap_ref()? {
            HeapValue::Set(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get mutable access to the SetData.
    #[inline]
    pub fn as_set_mut(&mut self) -> Option<&mut SetData> {
        match self.as_heap_mut()? {
            HeapValue::Set(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get the Deque data if this is a Deque.
    #[inline]
    pub fn as_deque(&self) -> Option<&DequeData> {
        match self.as_heap_ref()? {
            HeapValue::Deque(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get mutable access to the DequeData.
    #[inline]
    pub fn as_deque_mut(&mut self) -> Option<&mut DequeData> {
        match self.as_heap_mut()? {
            HeapValue::Deque(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get the PriorityQueue data if this is a PriorityQueue.
    #[inline]
    pub fn as_priority_queue(&self) -> Option<&PriorityQueueData> {
        match self.as_heap_ref()? {
            HeapValue::PriorityQueue(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Get mutable access to the PriorityQueueData.
    #[inline]
    pub fn as_priority_queue_mut(&mut self) -> Option<&mut PriorityQueueData> {
        match self.as_heap_mut()? {
            HeapValue::PriorityQueue(d) => Some(d),
            _ => std::option::Option::None,
        }
    }

    /// Extract a ContentNode reference.
    #[inline]
    pub fn as_content(&self) -> Option<&ContentNode> {
        match self.as_heap_ref()? {
            HeapValue::Content(node) => Some(node),
            _ => std::option::Option::None,
        }
    }

    /// Extract a DateTime<FixedOffset>.
    #[inline]
    pub fn as_time(&self) -> Option<DateTime<FixedOffset>> {
        match self.as_heap_ref()? {
            HeapValue::Time(t) => Some(*t),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to the Instant.
    #[inline]
    pub fn as_instant(&self) -> Option<&std::time::Instant> {
        match self.as_heap_ref()? {
            HeapValue::Instant(t) => Some(t.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to the IoHandleData.
    #[inline]
    pub fn as_io_handle(&self) -> Option<&crate::heap_value::IoHandleData> {
        match self.as_heap_ref()? {
            HeapValue::IoHandle(data) => Some(data.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Extract a width-aware native scalar value.
    #[inline]
    pub fn as_native_scalar(&self) -> Option<NativeScalar> {
        match self.as_heap_ref()? {
            HeapValue::NativeScalar(v) => Some(*v),
            _ => None,
        }
    }

    /// Extract a pointer-backed native view.
    #[inline]
    pub fn as_native_view(&self) -> Option<&NativeViewData> {
        match self.as_heap_ref()? {
            HeapValue::NativeView(view) => Some(view.as_ref()),
            _ => None,
        }
    }

    /// Extract a reference to the DateTime<FixedOffset>.
    #[inline]
    pub fn as_datetime(&self) -> Option<&DateTime<FixedOffset>> {
        match self.as_heap_ref()? {
            HeapValue::Time(t) => Some(t),
            _ => std::option::Option::None,
        }
    }

    /// Extract an Arc<String> from a heap String.
    #[inline]
    pub fn as_arc_string(&self) -> Option<&Arc<String>> {
        match self.as_heap_ref()? {
            HeapValue::String(s) => Some(s),
            _ => std::option::Option::None,
        }
    }

    // ===== Typed collection accessors =====

    /// Extract a reference to an IntArray.
    #[inline]
    pub fn as_int_array(&self) -> Option<&Arc<crate::typed_buffer::TypedBuffer<i64>>> {
        match self.as_heap_ref()? {
            HeapValue::IntArray(a) => Some(a),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to a FloatArray.
    #[inline]
    pub fn as_float_array(&self) -> Option<&Arc<crate::typed_buffer::AlignedTypedBuffer>> {
        match self.as_heap_ref()? {
            HeapValue::FloatArray(a) => Some(a),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to a BoolArray.
    #[inline]
    pub fn as_bool_array(&self) -> Option<&Arc<crate::typed_buffer::TypedBuffer<u8>>> {
        match self.as_heap_ref()? {
            HeapValue::BoolArray(a) => Some(a),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to MatrixData.
    #[inline]
    pub fn as_matrix(&self) -> Option<&crate::heap_value::MatrixData> {
        match self.as_heap_ref()? {
            HeapValue::Matrix(m) => Some(m.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to IteratorState.
    #[inline]
    pub fn as_iterator(&self) -> Option<&crate::heap_value::IteratorState> {
        match self.as_heap_ref()? {
            HeapValue::Iterator(it) => Some(it.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Extract a reference to GeneratorState.
    #[inline]
    pub fn as_generator(&self) -> Option<&crate::heap_value::GeneratorState> {
        match self.as_heap_ref()? {
            HeapValue::Generator(g) => Some(g.as_ref()),
            _ => std::option::Option::None,
        }
    }

    /// Get the length of a typed array (IntArray, FloatArray, BoolArray, width-specific).
    /// Returns None for non-typed-array values.
    #[inline]
    pub fn typed_array_len(&self) -> Option<usize> {
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
    pub fn coerce_to_float_array(&self) -> Option<Arc<crate::typed_buffer::AlignedTypedBuffer>> {
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
    pub fn to_generic_array(&self) -> Option<crate::value::VMArray> {
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

    /// Fast equality comparison without materializing HeapValue.
    /// For inline types (f64, i48, bool, none, unit, function), compares bits directly.
    /// For heap types, falls back to HeapValue equality.
    #[inline]
    pub fn vw_equals(&self, other: &ValueWord) -> bool {
        // Fast path: identical bits means identical value (except NaN)
        if self.0 == other.0 {
            // Special case: f64 NaN != NaN
            if !is_tagged(self.0) {
                let f = f64::from_bits(self.0);
                return !f.is_nan();
            }
            // For heap values, same bits = same pointer = definitely equal
            return true;
        }
        // Different bits — for inline types, they're definitely not equal
        if !is_tagged(self.0) || !is_tagged(other.0) {
            // At least one is f64 — if they're both f64 with different bits, not equal
            // (we already handled the case where both are identical)
            // Cross-type: f64 == i48 coercion
            if let (Some(a), Some(b)) = (self.as_number_coerce(), other.as_number_coerce()) {
                return a == b;
            }
            return false;
        }
        let tag_a = get_tag(self.0);
        let tag_b = get_tag(other.0);
        if tag_a != tag_b {
            // Different tags — check numeric coercion (f64 vs i48)
            if (tag_a == TAG_INT || !is_tagged(self.0)) && (tag_b == TAG_INT || !is_tagged(other.0))
            {
                if let (Some(a), Some(b)) = (self.as_number_coerce(), other.as_number_coerce()) {
                    return a == b;
                }
            }
            return false;
        }
        // Same tag, different bits — for heap values, compare HeapValue
        if tag_a == TAG_HEAP {
            let ptr_a = get_payload(self.0) as *const HeapValue;
            let ptr_b = get_payload(other.0) as *const HeapValue;
            return unsafe { (*ptr_a).equals(&*ptr_b) };
        }
        // For other same-tag inline values with different bits, not equal
        false
    }

    /// Compute a hash for a ValueWord value, suitable for HashMap key usage.
    /// Uses the existing tag dispatch for O(1) inline types.
    pub fn vw_hash(&self) -> u64 {
        use ahash::AHasher;
        use std::hash::{Hash, Hasher};

        let tag = self.tag();
        match tag {
            NanTag::F64 => {
                let f = unsafe { self.as_f64_unchecked() };
                let bits = if f == 0.0 { 0u64 } else { f.to_bits() };
                let mut hasher = AHasher::default();
                bits.hash(&mut hasher);
                hasher.finish()
            }
            NanTag::I48 => {
                let i = unsafe { self.as_i64_unchecked() };
                let mut hasher = AHasher::default();
                i.hash(&mut hasher);
                hasher.finish()
            }
            NanTag::Bool => {
                if unsafe { self.as_bool_unchecked() } {
                    1
                } else {
                    0
                }
            }
            NanTag::None => 0x_DEAD_0000,
            NanTag::Unit => 0x_DEAD_0001,
            NanTag::Heap => {
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
                            self.0.hash(&mut hasher);
                            hasher.finish()
                        }
                    }
                } else {
                    let mut hasher = AHasher::default();
                    self.0.hash(&mut hasher);
                    hasher.finish()
                }
            }
            _ => {
                let mut hasher = AHasher::default();
                self.0.hash(&mut hasher);
                hasher.finish()
            }
        }
    }

    // ===== Arithmetic helpers (operate directly on bits, no conversion) =====

    /// Add two inline f64 values.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline f64 values (`is_f64()` is true).
    #[inline(always)]
    pub unsafe fn add_f64(a: &Self, b: &Self) -> Self {
        debug_assert!(a.is_f64() && b.is_f64());
        let lhs = unsafe { a.as_f64_unchecked() };
        let rhs = unsafe { b.as_f64_unchecked() };
        Self::from_f64(lhs + rhs)
    }

    /// Add two inline i48 values with overflow promotion to f64.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline i48 values (`is_i64()` is true).
    #[inline(always)]
    pub unsafe fn add_i64(a: &Self, b: &Self) -> Self {
        debug_assert!(a.is_i64() && b.is_i64());
        let lhs = unsafe { a.as_i64_unchecked() };
        let rhs = unsafe { b.as_i64_unchecked() };
        match lhs.checked_add(rhs) {
            Some(result) if result >= I48_MIN && result <= I48_MAX => Self::from_i64(result),
            _ => Self::from_f64(lhs as f64 + rhs as f64),
        }
    }

    /// Subtract two inline f64 values.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline f64 values.
    #[inline(always)]
    pub unsafe fn sub_f64(a: &Self, b: &Self) -> Self {
        debug_assert!(a.is_f64() && b.is_f64());
        let lhs = unsafe { a.as_f64_unchecked() };
        let rhs = unsafe { b.as_f64_unchecked() };
        Self::from_f64(lhs - rhs)
    }

    /// Subtract two inline i48 values with overflow promotion to f64.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline i48 values.
    #[inline(always)]
    pub unsafe fn sub_i64(a: &Self, b: &Self) -> Self {
        debug_assert!(a.is_i64() && b.is_i64());
        let lhs = unsafe { a.as_i64_unchecked() };
        let rhs = unsafe { b.as_i64_unchecked() };
        match lhs.checked_sub(rhs) {
            Some(result) if result >= I48_MIN && result <= I48_MAX => Self::from_i64(result),
            _ => Self::from_f64(lhs as f64 - rhs as f64),
        }
    }

    /// Multiply two inline f64 values.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline f64 values.
    #[inline(always)]
    pub unsafe fn mul_f64(a: &Self, b: &Self) -> Self {
        debug_assert!(a.is_f64() && b.is_f64());
        let lhs = unsafe { a.as_f64_unchecked() };
        let rhs = unsafe { b.as_f64_unchecked() };
        Self::from_f64(lhs * rhs)
    }

    /// Multiply two inline i48 values with overflow promotion to f64.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline i48 values.
    #[inline(always)]
    pub unsafe fn mul_i64(a: &Self, b: &Self) -> Self {
        debug_assert!(a.is_i64() && b.is_i64());
        let lhs = unsafe { a.as_i64_unchecked() };
        let rhs = unsafe { b.as_i64_unchecked() };
        match lhs.checked_mul(rhs) {
            Some(result) if result >= I48_MIN && result <= I48_MAX => Self::from_i64(result),
            _ => Self::from_f64(lhs as f64 * rhs as f64),
        }
    }

    /// Divide two inline f64 values.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline f64 values.
    #[inline(always)]
    pub unsafe fn div_f64(a: &Self, b: &Self) -> Self {
        debug_assert!(a.is_f64() && b.is_f64());
        let lhs = unsafe { a.as_f64_unchecked() };
        let rhs = unsafe { b.as_f64_unchecked() };
        Self::from_f64(lhs / rhs)
    }

    /// Binary arithmetic with integer-preserving semantics and overflow promotion.
    ///
    /// If both operands are inline I48, applies `int_op` (checked) to the i64 values.
    /// On overflow (None), falls back to `float_op` with the f64 coercions.
    /// If either operand is f64, applies `float_op` directly.
    /// Callers must ensure `a_num` and `b_num` are the `as_number_coerce()` results
    /// from the same `a`/`b` operands.
    #[inline(always)]
    pub fn binary_int_preserving(
        a: &Self,
        b: &Self,
        a_num: f64,
        b_num: f64,
        int_op: impl FnOnce(i64, i64) -> Option<i64>,
        float_op: impl FnOnce(f64, f64) -> f64,
    ) -> Self {
        if matches!(a.tag(), NanTag::I48) && matches!(b.tag(), NanTag::I48) {
            match int_op(unsafe { a.as_i64_unchecked() }, unsafe {
                b.as_i64_unchecked()
            }) {
                Some(result) => Self::from_i64(result),
                None => Self::from_f64(float_op(a_num, b_num)),
            }
        } else {
            Self::from_f64(float_op(a_num, b_num))
        }
    }

    /// Compare two inline i48 values (greater than), returning a ValueWord bool.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline i48 values.
    #[inline(always)]
    pub unsafe fn gt_i64(a: &Self, b: &Self) -> Self {
        debug_assert!(a.is_i64() && b.is_i64());
        let lhs = unsafe { a.as_i64_unchecked() };
        let rhs = unsafe { b.as_i64_unchecked() };
        Self::from_bool(lhs > rhs)
    }

    /// Compare two inline i48 values (less than), returning a ValueWord bool.
    ///
    /// # Safety
    /// Both `a` and `b` must be inline i48 values.
    #[inline(always)]
    pub unsafe fn lt_i64(a: &Self, b: &Self) -> Self {
        debug_assert!(a.is_i64() && b.is_i64());
        let lhs = unsafe { a.as_i64_unchecked() };
        let rhs = unsafe { b.as_i64_unchecked() };
        Self::from_bool(lhs < rhs)
    }

    /// Returns the raw u64 bits (for debugging/testing).
    #[inline(always)]
    pub fn raw_bits(&self) -> u64 {
        self.0
    }

    /// Create a ValueWord from raw u64 bits without any NaN-boxing or tagging.
    ///
    /// This is the inverse of `raw_bits()`. The caller is responsible for
    /// ensuring the bits are interpreted correctly (e.g. via `push_raw_f64` /
    /// `pop_raw_f64`). No Drop/heap semantics are attached to the resulting
    /// value — it is treated as an opaque 8-byte slot.
    #[inline(always)]
    pub fn from_raw_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// Get the type name of this value.
    #[inline]
    pub fn type_name(&self) -> &'static str {
        if !is_tagged(self.0) {
            return "number";
        }
        let tag = get_tag(self.0);
        if tag == TAG_HEAP {
            let ptr = get_payload(self.0) as *const HeapValue;
            return unsafe { (*ptr).type_name() };
        }
        nan_tag_type_name(tag)
    }

    // ===== Convenience aliases for common extraction patterns =====

    /// Extract as f64, coercing i48 to f64 if needed.
    /// Alias for `as_number_coerce()` — convenience method.
    #[inline]
    pub fn to_number(&self) -> Option<f64> {
        self.as_number_coerce()
    }

    /// Extract as bool.
    /// Alias for `as_bool()` — convenience method.
    #[inline]
    pub fn to_bool(&self) -> Option<bool> {
        self.as_bool()
    }

    /// Convert Int or Number to usize (for indexing operations).
    pub fn as_usize(&self) -> Option<usize> {
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
    pub fn to_json_value(&self) -> serde_json::Value {
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
                    map.insert(format!("{}", k), v.to_json_value());
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

// --- PartialEq implementation ---
//
// Compares ValueWord values by semantic equality:
// - Inline types: bit-exact comparison (f64 NaN != NaN, matching IEEE 754)
// - Heap types: delegates to HeapValue::partial_eq

impl PartialEq for ValueWord {
    fn eq(&self, other: &Self) -> bool {
        // Bit-identical covers all inline types and same-Arc heap pointers.
        if self.0 == other.0 {
            return true;
        }
        // If both are heap-tagged, compare the underlying HeapValues structurally.
        if is_tagged(self.0)
            && get_tag(self.0) == TAG_HEAP
            && is_tagged(other.0)
            && get_tag(other.0) == TAG_HEAP
        {
            match (self.as_heap_ref(), other.as_heap_ref()) {
                (Some(a), Some(b)) => a.structural_eq(b),
                _ => false,
            }
        } else {
            false
        }
    }
}

impl Eq for ValueWord {}

// --- Clone implementation ---

#[cfg(not(feature = "gc"))]
impl Clone for ValueWord {
    #[inline]
    fn clone(&self) -> Self {
        if is_tagged(self.0) && get_tag(self.0) == TAG_HEAP {
            // Bump the Arc refcount — no allocation, no deep clone.
            let ptr = get_payload(self.0) as *const HeapValue;
            unsafe { Arc::increment_strong_count(ptr) };
            Self(self.0)
        } else {
            // Non-heap values: just copy the bits.
            Self(self.0)
        }
    }
}

#[cfg(feature = "gc")]
impl Clone for ValueWord {
    #[inline]
    fn clone(&self) -> Self {
        // GC path: bitwise copy. No refcount manipulation needed —
        // the GC traces live pointers to determine liveness.
        Self(self.0)
    }
}

// --- Drop implementation ---

#[cfg(not(feature = "gc"))]
impl Drop for ValueWord {
    fn drop(&mut self) {
        if is_tagged(self.0) && get_tag(self.0) == TAG_HEAP {
            let ptr = get_payload(self.0) as *const HeapValue;
            if !ptr.is_null() {
                unsafe {
                    // Decrement the Arc refcount; drops HeapValue when it reaches zero.
                    Arc::decrement_strong_count(ptr);
                }
            }
        }
    }
}

#[cfg(feature = "gc")]
impl Drop for ValueWord {
    #[inline]
    fn drop(&mut self) {
        // GC path: no-op. The garbage collector handles deallocation
        // by tracing live objects and reclaiming dead ones.
    }
}

impl std::fmt::Display for ValueWord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_f64() {
            let n = unsafe { self.as_f64_unchecked() };
            if n == n.trunc() && n.abs() < 1e15 {
                write!(f, "{}.0", n as i64)
            } else {
                write!(f, "{}", n)
            }
        } else if self.is_i64() {
            write!(f, "{}", unsafe { self.as_i64_unchecked() })
        } else if self.is_bool() {
            write!(f, "{}", unsafe { self.as_bool_unchecked() })
        } else if self.is_none() {
            write!(f, "none")
        } else if self.is_unit() {
            write!(f, "()")
        } else if self.is_function() {
            write!(f, "<function:{}>", unsafe { self.as_function_unchecked() })
        } else if self.is_module_function() {
            write!(f, "<module_function>")
        } else if let Some(target) = self.as_ref_target() {
            match target {
                RefTarget::Stack(slot) => write!(f, "&slot_{}", slot),
                RefTarget::ModuleBinding(slot) => write!(f, "&module_{}", slot),
                RefTarget::Projected(_) => write!(f, "&ref"),
            }
        } else if let Some(hv) = self.as_heap_ref() {
            match hv {
                HeapValue::Char(c) => write!(f, "{}", c),
                HeapValue::String(s) => write!(f, "{}", s),
                HeapValue::Array(arr) => {
                    write!(f, "[")?;
                    for (i, elem) in arr.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", elem)?;
                    }
                    write!(f, "]")
                }
                HeapValue::TypedObject { .. } => write!(f, "{{...}}"),
                HeapValue::Closure { function_id, .. } => write!(f, "<closure:{}>", function_id),
                HeapValue::Decimal(d) => write!(f, "{}", d),
                HeapValue::BigInt(i) => write!(f, "{}", i),
                HeapValue::HostClosure(_) => write!(f, "<host_closure>"),
                HeapValue::DataTable(dt) => {
                    write!(f, "<datatable:{}x{}>", dt.row_count(), dt.column_count())
                }
                HeapValue::TypedTable { table, .. } => write!(
                    f,
                    "<typed_table:{}x{}>",
                    table.row_count(),
                    table.column_count()
                ),
                HeapValue::RowView { row_idx, .. } => write!(f, "<row:{}>", row_idx),
                HeapValue::ColumnRef { col_id, .. } => write!(f, "<column:{}>", col_id),
                HeapValue::IndexedTable { table, .. } => write!(
                    f,
                    "<indexed_table:{}x{}>",
                    table.row_count(),
                    table.column_count()
                ),
                HeapValue::HashMap(d) => {
                    write!(f, "HashMap{{")?;
                    for (i, (k, v)) in d.keys.iter().zip(d.values.iter()).enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}: {}", k, v)?;
                    }
                    write!(f, "}}")
                }
                HeapValue::Set(d) => {
                    write!(f, "Set{{")?;
                    for (i, item) in d.items.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", item)?;
                    }
                    write!(f, "}}")
                }
                HeapValue::Deque(d) => {
                    write!(f, "Deque[")?;
                    for (i, item) in d.items.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", item)?;
                    }
                    write!(f, "]")
                }
                HeapValue::PriorityQueue(d) => {
                    write!(f, "PriorityQueue[")?;
                    for (i, item) in d.items.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", item)?;
                    }
                    write!(f, "]")
                }
                HeapValue::Content(node) => write!(f, "{}", node),
                HeapValue::Instant(t) => write!(f, "<instant:{:?}>", t.elapsed()),
                HeapValue::IoHandle(data) => {
                    let status = if data.is_open() { "open" } else { "closed" };
                    write!(f, "<io_handle:{}:{}>", data.path, status)
                }
                HeapValue::Range {
                    start,
                    end,
                    inclusive,
                } => {
                    if let Some(s) = start {
                        write!(f, "{}", s)?;
                    }
                    write!(f, "{}", if *inclusive { "..=" } else { ".." })?;
                    if let Some(e) = end {
                        write!(f, "{}", e)?;
                    }
                    std::fmt::Result::Ok(())
                }
                HeapValue::Enum(e) => {
                    write!(f, "{}", e.variant)?;
                    match &e.payload {
                        crate::enums::EnumPayload::Unit => Ok(()),
                        crate::enums::EnumPayload::Tuple(values) => {
                            write!(f, "(")?;
                            for (i, v) in values.iter().enumerate() {
                                if i > 0 {
                                    write!(f, ", ")?;
                                }
                                write!(f, "{}", v)?;
                            }
                            write!(f, ")")
                        }
                        crate::enums::EnumPayload::Struct(fields) => {
                            let mut pairs: Vec<_> = fields.iter().collect();
                            pairs.sort_by_key(|(k, _)| (*k).clone());
                            write!(f, " {{ ")?;
                            for (i, (k, v)) in pairs.iter().enumerate() {
                                if i > 0 {
                                    write!(f, ", ")?;
                                }
                                write!(f, "{}: {}", k, v)?;
                            }
                            write!(f, " }}")
                        }
                    }
                }
                HeapValue::Some(v) => write!(f, "some({})", v),
                HeapValue::Ok(v) => write!(f, "ok({})", v),
                HeapValue::Err(v) => write!(f, "err({})", v),
                HeapValue::Future(id) => write!(f, "<future:{}>", id),
                HeapValue::TaskGroup { task_ids, .. } => {
                    write!(f, "<task_group:{}>", task_ids.len())
                }
                HeapValue::TraitObject { value, .. } => write!(f, "{}", value),
                HeapValue::ExprProxy(name) => write!(f, "<expr:{}>", name),
                HeapValue::FilterExpr(_) => write!(f, "<filter_expr>"),
                HeapValue::Time(t) => write!(f, "{}", t),
                HeapValue::Duration(d) => write!(f, "{:?}", d),
                HeapValue::TimeSpan(ts) => write!(f, "{}", ts),
                HeapValue::Timeframe(tf) => write!(f, "{:?}", tf),
                HeapValue::TimeReference(_) => write!(f, "<time_ref>"),
                HeapValue::DateTimeExpr(_) => write!(f, "<datetime_expr>"),
                HeapValue::DataDateTimeRef(_) => write!(f, "<data_datetime_ref>"),
                HeapValue::TypeAnnotation(_) => write!(f, "<type_annotation>"),
                HeapValue::TypeAnnotatedValue { type_name, value } => {
                    write!(f, "{}({})", type_name, value)
                }
                HeapValue::PrintResult(_) => write!(f, "<print_result>"),
                HeapValue::SimulationCall(data) => write!(f, "<simulation:{}>", data.name),
                HeapValue::FunctionRef { name, .. } => write!(f, "<fn:{}>", name),
                HeapValue::ProjectedRef(_) => write!(f, "&ref"),
                HeapValue::DataReference(data) => write!(f, "<data:{}>", data.id),
                HeapValue::NativeScalar(v) => write!(f, "{v}"),
                HeapValue::NativeView(v) => write!(
                    f,
                    "<{}:{}@0x{:x}>",
                    if v.mutable { "cmut" } else { "cview" },
                    v.layout.name,
                    v.ptr
                ),
                HeapValue::SharedCell(arc) => write!(f, "{}", arc.read().unwrap()),
                HeapValue::IntArray(a) => {
                    write!(f, "Vec<int>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                HeapValue::FloatArray(a) => {
                    write!(f, "Vec<number>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        if *v == v.trunc() && v.abs() < 1e15 {
                            write!(f, "{}", *v as i64)?;
                        } else {
                            write!(f, "{}", v)?;
                        }
                    }
                    write!(f, "]")
                }
                HeapValue::BoolArray(a) => {
                    write!(f, "Vec<bool>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", *v != 0)?;
                    }
                    write!(f, "]")
                }
                HeapValue::Matrix(m) => {
                    write!(f, "<Mat<number>:{}x{}>", m.rows, m.cols)
                }
                HeapValue::Iterator(it) => {
                    write!(
                        f,
                        "<iterator:pos={},transforms={}>",
                        it.position,
                        it.transforms.len()
                    )
                }
                HeapValue::Generator(g) => {
                    write!(f, "<generator:state={}>", g.state)
                }
                HeapValue::Mutex(_) => write!(f, "<mutex>"),
                HeapValue::Atomic(a) => {
                    write!(
                        f,
                        "<atomic:{}>",
                        a.inner.load(std::sync::atomic::Ordering::Relaxed)
                    )
                }
                HeapValue::Channel(c) => {
                    if c.is_sender() {
                        write!(f, "<channel:sender>")
                    } else {
                        write!(f, "<channel:receiver>")
                    }
                }
                HeapValue::Lazy(l) => {
                    let initialized = l.value.lock().map(|g| g.is_some()).unwrap_or(false);
                    if initialized {
                        write!(f, "<lazy:initialized>")
                    } else {
                        write!(f, "<lazy:pending>")
                    }
                }
                HeapValue::I8Array(a) => {
                    write!(f, "Vec<i8>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                HeapValue::I16Array(a) => {
                    write!(f, "Vec<i16>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                HeapValue::I32Array(a) => {
                    write!(f, "Vec<i32>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                HeapValue::U8Array(a) => {
                    write!(f, "Vec<u8>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                HeapValue::U16Array(a) => {
                    write!(f, "Vec<u16>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                HeapValue::U32Array(a) => {
                    write!(f, "Vec<u32>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                HeapValue::U64Array(a) => {
                    write!(f, "Vec<u64>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                HeapValue::F32Array(a) => {
                    write!(f, "Vec<f32>[")?;
                    for (i, v) in a.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", v)?;
                    }
                    write!(f, "]")
                }
                HeapValue::FloatArraySlice {
                    parent,
                    offset,
                    len,
                } => {
                    let slice =
                        &parent.data[*offset as usize..(*offset + *len) as usize];
                    write!(f, "Vec<number>[")?;
                    for (i, v) in slice.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        if *v == v.trunc() && v.abs() < 1e15 {
                            write!(f, "{}", *v as i64)?;
                        } else {
                            write!(f, "{}", v)?;
                        }
                    }
                    write!(f, "]")
                }
            }
        } else {
            write!(f, "<unknown>")
        }
    }
}

impl std::fmt::Debug for ValueWord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_f64() {
            write!(f, "ValueWord(f64: {})", unsafe { self.as_f64_unchecked() })
        } else if self.is_i64() {
            write!(f, "ValueWord(i64: {})", unsafe { self.as_i64_unchecked() })
        } else if self.is_bool() {
            write!(f, "ValueWord(bool: {})", unsafe {
                self.as_bool_unchecked()
            })
        } else if self.is_none() {
            write!(f, "ValueWord(None)")
        } else if self.is_unit() {
            write!(f, "ValueWord(Unit)")
        } else if self.is_function() {
            write!(f, "ValueWord(Function({}))", unsafe {
                self.as_function_unchecked()
            })
        } else if let Some(target) = self.as_ref_target() {
            write!(f, "ValueWord(Ref({:?}))", target)
        } else if self.is_heap() {
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
        assert_eq!(v.as_function(), Some(42));
        unsafe { assert_eq!(v.as_function_unchecked(), 42) };
    }

    #[test]
    fn test_function_max_id() {
        let v = ValueWord::from_function(u16::MAX);
        assert!(v.is_function());
        assert_eq!(v.as_function(), Some(u16::MAX));
    }

    // ===== Module function =====

    #[test]
    fn test_module_function() {
        let v = ValueWord::from_module_function(99);
        assert!(is_tagged(v.0));
        assert_eq!(get_tag(v.0), TAG_MODULE_FN);
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
            get_payload(v.0),
            get_payload(cloned.0),
            "cloned heap pointers should be identical (Arc shared)"
        );
    }

    // ===== Arithmetic helpers =====

    #[test]
    fn test_add_f64() {
        let a = ValueWord::from_f64(1.5);
        let b = ValueWord::from_f64(2.5);
        let result = unsafe { ValueWord::add_f64(&a, &b) };
        assert_eq!(result.as_f64(), Some(4.0));
    }

    #[test]
    fn test_add_i64() {
        let a = ValueWord::from_i64(100);
        let b = ValueWord::from_i64(200);
        let result = unsafe { ValueWord::add_i64(&a, &b) };
        assert_eq!(result.as_i64(), Some(300));
    }

    #[test]
    fn test_add_i64_negative() {
        let a = ValueWord::from_i64(-50);
        let b = ValueWord::from_i64(30);
        let result = unsafe { ValueWord::add_i64(&a, &b) };
        assert_eq!(result.as_i64(), Some(-20));
    }

    #[test]
    fn test_sub_f64() {
        let a = ValueWord::from_f64(10.0);
        let b = ValueWord::from_f64(3.0);
        let result = unsafe { ValueWord::sub_f64(&a, &b) };
        assert_eq!(result.as_f64(), Some(7.0));
    }

    #[test]
    fn test_sub_i64() {
        let a = ValueWord::from_i64(50);
        let b = ValueWord::from_i64(80);
        let result = unsafe { ValueWord::sub_i64(&a, &b) };
        assert_eq!(result.as_i64(), Some(-30));
    }

    #[test]
    fn test_mul_f64() {
        let a = ValueWord::from_f64(3.0);
        let b = ValueWord::from_f64(4.0);
        let result = unsafe { ValueWord::mul_f64(&a, &b) };
        assert_eq!(result.as_f64(), Some(12.0));
    }

    #[test]
    fn test_mul_i64() {
        let a = ValueWord::from_i64(7);
        let b = ValueWord::from_i64(-6);
        let result = unsafe { ValueWord::mul_i64(&a, &b) };
        assert_eq!(result.as_i64(), Some(-42));
    }

    #[test]
    fn test_div_f64() {
        let a = ValueWord::from_f64(10.0);
        let b = ValueWord::from_f64(4.0);
        let result = unsafe { ValueWord::div_f64(&a, &b) };
        assert_eq!(result.as_f64(), Some(2.5));
    }

    #[test]
    fn test_gt_i64() {
        let a = ValueWord::from_i64(10);
        let b = ValueWord::from_i64(5);
        let result = unsafe { ValueWord::gt_i64(&a, &b) };
        assert_eq!(result.as_bool(), Some(true));

        let result2 = unsafe { ValueWord::gt_i64(&b, &a) };
        assert_eq!(result2.as_bool(), Some(false));
    }

    #[test]
    fn test_lt_i64() {
        let a = ValueWord::from_i64(3);
        let b = ValueWord::from_i64(7);
        let result = unsafe { ValueWord::lt_i64(&a, &b) };
        assert_eq!(result.as_bool(), Some(true));

        let result2 = unsafe { ValueWord::lt_i64(&b, &a) };
        assert_eq!(result2.as_bool(), Some(false));
    }

    // ===== i64 overflow in arithmetic =====

    #[test]
    fn test_add_i64_overflow_to_heap() {
        // Adding two values near i48 max that overflow to > i48 range
        // promotes to f64 (V8 SMI semantics) instead of heap-boxing as BigInt
        let a = ValueWord::from_i64(I48_MAX);
        let b = ValueWord::from_i64(1);
        let result = unsafe { ValueWord::add_i64(&a, &b) };
        assert!(result.is_f64());
        let expected = (I48_MAX + 1) as f64;
        assert_eq!(result.as_f64(), Some(expected));
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
        let dbg = format!("{:?}", v);
        assert!(dbg.contains("f64"));
        assert!(dbg.contains("3.14"));

        let v = ValueWord::from_i64(42);
        let dbg = format!("{:?}", v);
        assert!(dbg.contains("i64"));
        assert!(dbg.contains("42"));

        let v = ValueWord::none();
        let dbg = format!("{:?}", v);
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
        let payload = get_payload(v.0);
        assert_eq!(payload, 0x0000_FFFF_FFFF_FFFF);
        assert_eq!(sign_extend_i48(payload), -1);

        // Most negative i48: -2^47
        let v = ValueWord::from_i64(I48_MIN);
        let payload = get_payload(v.0);
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
        let display = format!("{}", v);
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
        assert_eq!(v.as_u64(), Some(u64::MAX));
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
