//! Typed Value
//!
//! A value paired with its semantic type information, used at boundaries
//! where type information must be preserved.
//!
//! ## When to Use TypedValue
//!
//! - Return values from external data sources and extension modules
//! - Generic function boundaries
//! - REPL display (show type alongside value)
//! - Serialization/deserialization boundaries
//!
//! ## When NOT to Use TypedValue
//!
//! - Inner loop variables (JIT uses raw f64)
//! - Column element access (direct memory)
//! - Hot paths where type is statically known

use super::semantic::SemanticType;
use super::storage::StorageType;
use shape_value::{HeapValue, ValueWord, ValueWordExt};
use std::fmt;
use std::sync::Arc;

/// A value with its associated semantic type
///
/// Used at function boundaries where type information must be preserved.
/// This is NOT used in hot inner loops where JIT uses raw types.
#[derive(Clone, Debug)]
pub struct TypedValue {
    /// The runtime value
    pub value: ValueWord,
    /// The semantic type (what the user sees)
    pub semantic_type: SemanticType,
}

impl TypedValue {
    /// Create a new typed value
    pub fn new(value: ValueWord, semantic_type: SemanticType) -> Self {
        TypedValue {
            value,
            semantic_type,
        }
    }

    /// Create a new typed value from ValueWord (avoids ValueWord construction at call site)
    pub fn new_nb(value: ValueWord, semantic_type: SemanticType) -> Self {
        TypedValue {
            value: value.clone(),
            semantic_type,
        }
    }

    /// Create a typed number
    pub fn number(n: f64) -> Self {
        TypedValue {
            value: ValueWord::from_f64(n),
            semantic_type: SemanticType::Number,
        }
    }

    /// Create a typed integer
    pub fn integer(n: i64) -> Self {
        TypedValue {
            value: ValueWord::from_i64(n),
            semantic_type: SemanticType::Integer,
        }
    }

    /// Create a typed string
    pub fn string(s: impl Into<String>) -> Self {
        TypedValue {
            value: ValueWord::from_string(Arc::new(s.into())),
            semantic_type: SemanticType::String,
        }
    }

    /// Create a typed boolean
    pub fn boolean(b: bool) -> Self {
        TypedValue {
            value: ValueWord::from_bool(b),
            semantic_type: SemanticType::Bool,
        }
    }

    /// Create a typed None (Option::None)
    pub fn none(inner_type: SemanticType) -> Self {
        TypedValue {
            value: ValueWord::none(),
            semantic_type: SemanticType::Option(Box::new(inner_type)),
        }
    }

    /// Create a typed Some (Option::Some)
    pub fn some(inner: TypedValue) -> Self {
        let inner_type = inner.semantic_type.clone();
        TypedValue {
            value: inner.value,
            semantic_type: SemanticType::Option(Box::new(inner_type)),
        }
    }

    /// Create a typed Ok (Result::Ok)
    pub fn ok(inner: TypedValue) -> Self {
        let inner_type = inner.semantic_type.clone();
        TypedValue {
            value: inner.value,
            semantic_type: SemanticType::Result {
                ok_type: Box::new(inner_type),
                err_type: None,
            },
        }
    }

    /// Create a typed Err (Result::Err)
    pub fn err(message: impl Into<String>, ok_type: SemanticType) -> Self {
        let error_obj = crate::type_schema::typed_object_from_pairs(&[
            ("message", ValueWord::from_string(Arc::new(message.into()))),
            (
                "code",
                ValueWord::from_string(Arc::new("ERROR".to_string())),
            ),
        ]);

        TypedValue {
            value: ValueWord::from_err(error_obj),
            semantic_type: SemanticType::Result {
                ok_type: Box::new(ok_type),
                err_type: None,
            },
        }
    }

    /// Create a void typed value
    pub fn void() -> Self {
        TypedValue {
            value: ValueWord::none(),
            semantic_type: SemanticType::Void,
        }
    }

    /// Get the storage type (for JIT optimization hints)
    pub fn storage_type(&self) -> StorageType {
        StorageType::from_semantic(&self.semantic_type)
    }

    /// Check if this is an Option type
    pub fn is_option(&self) -> bool {
        self.semantic_type.is_option()
    }

    /// Check if this is a Result type
    pub fn is_result(&self) -> bool {
        self.semantic_type.is_result()
    }

    /// Unwrap the inner value from Option::Some
    /// Returns None if this is Option::None or not an Option type
    pub fn unwrap_option(self) -> Option<TypedValue> {
        if let SemanticType::Option(inner_type) = self.semantic_type {
            if self.value.is_none() {
                None
            } else {
                Some(TypedValue {
                    value: self.value,
                    semantic_type: *inner_type,
                })
            }
        } else {
            None
        }
    }

    /// Unwrap the Ok value from Result::Ok
    /// Returns Err with the error if this is Result::Err
    pub fn unwrap_result(self) -> Result<TypedValue, ValueWord> {
        if let SemanticType::Result { ok_type, .. } = self.semantic_type {
            if let Some(inner) = self.value.as_err_inner() {
                Err(inner.clone())
            } else if let Some(inner) = self.value.as_ok_inner() {
                Ok(TypedValue {
                    value: inner.clone(),
                    semantic_type: *ok_type,
                })
            } else {
                Ok(TypedValue {
                    value: self.value,
                    semantic_type: *ok_type,
                })
            }
        } else {
            // Not a result, just return as-is
            Ok(self)
        }
    }

    /// Format for display (includes type annotation)
    pub fn display_with_type(&self) -> String {
        format!("{:?}: {}", self.value, self.semantic_type)
    }

    /// Convert to wire protocol TypeInfo
    ///
    /// This allows TypedValue to provide proper type information
    /// to the REPL and external tools.
    pub fn to_type_info(&self) -> shape_wire::metadata::TypeInfo {
        self.semantic_type.to_type_info()
    }
}

impl fmt::Display for TypedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.value)
    }
}

/// Convert ValueWord to TypedValue with inferred type
///
/// Note: This performs basic type inference from the runtime value.
/// For precise typing, use explicit TypedValue constructors.
impl From<ValueWord> for TypedValue {
    fn from(nb: ValueWord) -> Self {
        let semantic_type = infer_semantic_type_nb(&nb);
        TypedValue {
            value: nb,
            semantic_type,
        }
    }
}

/// Infer semantic type from a ValueWord value without materializing ValueWord.
///
/// Dispatches on tag bits for inline types and HeapValue for heap-allocated types.
/// This avoids the allocation overhead of `nb.clone()` for type inference.
fn infer_semantic_type_nb(nb: &ValueWord) -> SemanticType {
    use shape_value::tags::{is_tagged, get_tag, TAG_INT, TAG_BOOL, TAG_NONE, TAG_UNIT, TAG_FUNCTION, TAG_MODULE_FN, TAG_HEAP, TAG_REF};
    let bits = nb.raw_bits();
    if !is_tagged(bits) {
        return SemanticType::Number;
    }
    match get_tag(bits) {
        TAG_INT => SemanticType::Integer,
        TAG_BOOL => SemanticType::Bool,
        TAG_NONE => SemanticType::Option(Box::new(SemanticType::Named("Unknown".to_string()))),
        TAG_UNIT => SemanticType::Void,
        TAG_FUNCTION | TAG_MODULE_FN => {
            SemanticType::Function(Box::new(super::semantic::FunctionSignature {
                params: vec![],
                return_type: SemanticType::Named("Unknown".to_string()),
                is_fallible: false,
            }))
        }
        TAG_HEAP => {
            // cold-path: as_heap_ref retained — multi-variant type inference dispatch
            match nb.as_heap_ref() { // cold-path
                Some(hv) => infer_semantic_type_heap(hv),
                // Should never happen: Heap tag but no heap ref
                std::option::Option::None => SemanticType::Named("Unknown".to_string()),
            }
        }
        TAG_REF => SemanticType::Named("Unknown".to_string()), // References are transparent at the type level
        _ => SemanticType::Named("Unknown".to_string()),
    }
}

/// Infer semantic type from a HeapValue reference.
fn infer_semantic_type_heap(hv: &HeapValue) -> SemanticType {
    match hv {
        HeapValue::String(_) => SemanticType::String,
        HeapValue::Array(arr) => {
            let elem_type = arr
                .first()
                .map(|nb| infer_semantic_type_nb(nb))
                .unwrap_or(SemanticType::Named("Unknown".to_string()));
            SemanticType::Array(Box::new(elem_type))
        }
        HeapValue::TypedObject { .. } => SemanticType::Struct {
            name: "Object".to_string(),
            fields: vec![],
        },
        HeapValue::Closure { .. } | HeapValue::FunctionRef { .. } => {
            SemanticType::Function(Box::new(super::semantic::FunctionSignature {
                params: vec![],
                return_type: SemanticType::Named("Unknown".to_string()),
                is_fallible: false,
            }))
        }
        HeapValue::ProjectedRef(_) => SemanticType::Named("Unknown".to_string()),
        HeapValue::Decimal(_) => SemanticType::Named("Decimal".to_string()),
        HeapValue::BigInt(_) => SemanticType::Integer,
        HeapValue::HostClosure(_) => SemanticType::Named("HostClosure".to_string()),
        HeapValue::DataTable(_) => SemanticType::Named("DataTable".to_string()),
        HeapValue::TableView(shape_value::heap_value::TableViewData::TypedTable { .. }) => SemanticType::Named("TypedTable".to_string()),
        HeapValue::TableView(shape_value::heap_value::TableViewData::RowView { .. }) => SemanticType::Named("Row".to_string()),
        HeapValue::TableView(shape_value::heap_value::TableViewData::ColumnRef { .. }) => SemanticType::Named("ColumnRef".to_string()),
        HeapValue::TableView(shape_value::heap_value::TableViewData::IndexedTable { .. }) => SemanticType::Named("IndexedTable".to_string()),
        HeapValue::Range { .. } => SemanticType::Named("Range".to_string()),
        HeapValue::Enum(e) => SemanticType::Named(e.enum_name.clone()),
        HeapValue::Some(inner) => SemanticType::Option(Box::new(infer_semantic_type_nb(inner))),
        HeapValue::Ok(inner) => SemanticType::Result {
            ok_type: Box::new(infer_semantic_type_nb(inner)),
            err_type: None,
        },
        HeapValue::Err(_) => SemanticType::Result {
            ok_type: Box::new(SemanticType::Named("Unknown".to_string())),
            err_type: None,
        },
        HeapValue::Future(_) => SemanticType::Named("Future".to_string()),
        HeapValue::TaskGroup { .. } => SemanticType::Named("TaskGroup".to_string()),
        HeapValue::TraitObject { value, .. } => infer_semantic_type_nb(value),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::ExprProxy(_)) => SemanticType::Named("ExprProxy".to_string()),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::FilterExpr(_)) => SemanticType::Named("FilterExpr".to_string()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::DateTime(_)) => SemanticType::Named("Time".to_string()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::Duration(_)) => SemanticType::Named("Duration".to_string()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::TimeSpan(_)) => SemanticType::Named("TimeSpan".to_string()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::Timeframe(_)) => SemanticType::Named("Timeframe".to_string()),
        HeapValue::NativeScalar(v) => match v {
            shape_value::heap_value::NativeScalar::I8(_) => SemanticType::Named("i8".to_string()),
            shape_value::heap_value::NativeScalar::U8(_) => SemanticType::Named("u8".to_string()),
            shape_value::heap_value::NativeScalar::I16(_) => SemanticType::Named("i16".to_string()),
            shape_value::heap_value::NativeScalar::U16(_) => SemanticType::Named("u16".to_string()),
            shape_value::heap_value::NativeScalar::I32(_) => SemanticType::Named("i32".to_string()),
            shape_value::heap_value::NativeScalar::I64(_) => SemanticType::Named("i64".to_string()),
            shape_value::heap_value::NativeScalar::U32(_) => SemanticType::Named("u32".to_string()),
            shape_value::heap_value::NativeScalar::U64(_) => SemanticType::Named("u64".to_string()),
            shape_value::heap_value::NativeScalar::Isize(_) => {
                SemanticType::Named("isize".to_string())
            }
            shape_value::heap_value::NativeScalar::Usize(_) => {
                SemanticType::Named("usize".to_string())
            }
            shape_value::heap_value::NativeScalar::Ptr(_) => SemanticType::Named("ptr".to_string()),
            shape_value::heap_value::NativeScalar::F32(_) => SemanticType::Named("f32".to_string()),
        },
        HeapValue::NativeView(view) => SemanticType::Named(format!(
            "{}<{}>",
            if view.mutable { "CMut" } else { "CView" },
            view.layout.name
        )),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::TimeReference(_)) => SemanticType::Named("TimeReference".to_string()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::DateTimeExpr(_)) => SemanticType::Named("DateTimeExpr".to_string()),
        HeapValue::Temporal(shape_value::heap_value::TemporalData::DataDateTimeRef(_)) => SemanticType::Named("DataDateTimeRef".to_string()),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::TypeAnnotation(_)) => SemanticType::Named("TypeAnnotation".to_string()),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::TypeAnnotatedValue { value, .. }) => infer_semantic_type_nb(value),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::PrintResult(_)) => SemanticType::Named("PrintResult".to_string()),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::SimulationCall(_)) => SemanticType::Named("SimulationCall".to_string()),
        HeapValue::Rare(shape_value::heap_value::RareHeapData::DataReference(_)) => SemanticType::Named("DataReference".to_string()),
        HeapValue::HashMap(_) => SemanticType::Named("HashMap".to_string()),
        HeapValue::Set(_) => SemanticType::Named("Set".to_string()),
        HeapValue::Deque(_) => SemanticType::Named("Deque".to_string()),
        HeapValue::PriorityQueue(_) => SemanticType::Named("PriorityQueue".to_string()),
        HeapValue::Content(_) => SemanticType::Named("Content".to_string()),
        HeapValue::Instant(_) => SemanticType::Named("Instant".to_string()),
        HeapValue::IoHandle(_) => SemanticType::Named("IoHandle".to_string()),
        HeapValue::SharedCell(arc) => infer_semantic_type_nb(&arc.read().unwrap()),
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I64(_)) => SemanticType::Array(Box::new(SemanticType::Integer)),
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::F64(_)) => SemanticType::Array(Box::new(SemanticType::Number)),
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::FloatSlice { .. }) => SemanticType::Array(Box::new(SemanticType::Number)),
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::Bool(_)) => SemanticType::Array(Box::new(SemanticType::Bool)),
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I8(_)) => {
            SemanticType::Array(Box::new(SemanticType::Named("i8".to_string())))
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I16(_)) => {
            SemanticType::Array(Box::new(SemanticType::Named("i16".to_string())))
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::I32(_)) => {
            SemanticType::Array(Box::new(SemanticType::Named("i32".to_string())))
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U8(_)) => {
            SemanticType::Array(Box::new(SemanticType::Named("u8".to_string())))
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U16(_)) => {
            SemanticType::Array(Box::new(SemanticType::Named("u16".to_string())))
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U32(_)) => {
            SemanticType::Array(Box::new(SemanticType::Named("u32".to_string())))
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::U64(_)) => {
            SemanticType::Array(Box::new(SemanticType::Named("u64".to_string())))
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::F32(_)) => {
            SemanticType::Array(Box::new(SemanticType::Named("f32".to_string())))
        }
        HeapValue::TypedArray(shape_value::heap_value::TypedArrayData::Matrix(_)) => SemanticType::Named("Mat<number>".to_string()),
        HeapValue::Iterator(_) => SemanticType::Named("Iterator".to_string()),
        HeapValue::Generator(_) => SemanticType::Named("Generator".to_string()),
        HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Mutex(_)) => SemanticType::Named("Mutex".to_string()),
        HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Atomic(_)) => SemanticType::Named("Atomic".to_string()),
        HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Lazy(_)) => SemanticType::Named("Lazy".to_string()),
        HeapValue::Concurrency(shape_value::heap_value::ConcurrencyData::Channel(_)) => SemanticType::Named("Channel".to_string()),
        HeapValue::Char(_) => SemanticType::Named("char".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typed_number() {
        let tv = TypedValue::number(42.0);
        assert_eq!(tv.semantic_type, SemanticType::Number);
        assert_eq!(tv.value.as_f64(), Some(42.0));
    }

    #[test]
    fn test_typed_string() {
        let tv = TypedValue::string("hello");
        assert_eq!(tv.semantic_type, SemanticType::String);
    }

    #[test]
    fn test_typed_option_some() {
        let inner = TypedValue::number(42.0);
        let some = TypedValue::some(inner);
        assert!(some.is_option());
        assert_eq!(
            some.semantic_type,
            SemanticType::Option(Box::new(SemanticType::Number))
        );
    }

    #[test]
    fn test_typed_option_none() {
        let none = TypedValue::none(SemanticType::Number);
        assert!(none.is_option());
        assert!(none.value.is_none());
    }

    #[test]
    fn test_typed_result_ok() {
        let inner = TypedValue::string("success");
        let ok = TypedValue::ok(inner);
        assert!(ok.is_result());
    }

    #[test]
    fn test_unwrap_option() {
        let some = TypedValue::some(TypedValue::number(42.0));
        let unwrapped = some.unwrap_option();
        assert!(unwrapped.is_some());
        let inner = unwrapped.unwrap();
        assert_eq!(inner.semantic_type, SemanticType::Number);
    }

    #[test]
    fn test_storage_type_derivation() {
        let option_num = TypedValue::some(TypedValue::number(42.0));
        let storage = option_num.storage_type();
        assert_eq!(storage, StorageType::NullableFloat64);
    }

    #[test]
    fn test_display_with_type() {
        let tv = TypedValue::number(42.0);
        let display = tv.display_with_type();
        assert!(display.contains("42"));
        assert!(display.contains("Number"));
    }

    #[test]
    fn test_native_scalar_width_is_preserved_in_semantic_type() {
        let tv: TypedValue = ValueWord::from_native_i16(7).into();
        assert_eq!(tv.semantic_type, SemanticType::Named("i16".to_string()));

        let tv_u8: TypedValue = ValueWord::from_native_u8(255).into();
        assert_eq!(tv_u8.semantic_type, SemanticType::Named("u8".to_string()));
    }

    #[test]
    fn test_native_f32_scalar_is_preserved_in_semantic_type() {
        let tv: TypedValue = ValueWord::from_native_f32(3.5).into();
        assert_eq!(tv.semantic_type, SemanticType::Named("f32".to_string()));
    }
}
