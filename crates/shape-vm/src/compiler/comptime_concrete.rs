//! Comptime ↔ ConcreteType bridge for v2 Phase 5.
//!
//! This module provides a typed view over comptime evaluation results so that
//! values produced inside `comptime { }` blocks can flow through the rest of
//! the v2 monomorphization pipeline as `ConcreteType`s rather than opaque
//! NaN-boxed `ValueWord`s.
//!
//! ## Design
//!
//! Until Phase 4 lands, the comptime mini-VM still uses `ValueWord` as its
//! universal value representation. We don't want to boil the ocean — Phase 4
//! is the boundary where `ValueWord` is finally deleted, and Phase 5 needs to
//! coexist with the still-NaN-boxed runtime.
//!
//! [`ComptimeValue`] therefore wraps a `ValueWord` and exposes:
//! - the underlying NaN-boxed value (so existing comptime code keeps working),
//! - an *optional* [`ConcreteType`] tag describing what the value really is.
//!
//! When the comptime evaluator can prove the type of a result (because it
//! came from a typed builtin like `build_config()`, was unrolled from a
//! typed iterator, or was constructed by a comptime helper that returned a
//! known typed value), it attaches the `ConcreteType`. When it can't, the
//! tag stays `None` and we fall back to NaN-box-driven introspection — Phase
//! 4 will eliminate this fallback once `ValueWord` is gone.
//!
//! ## Bridge functions
//!
//! - [`comptime_value_to_concrete_type`] — given a [`ComptimeValue`], extract
//!   the `ConcreteType` (using both the explicit tag and NaN-box inspection).
//! - [`concrete_type_to_comptime_value`] — given a `ConcreteType`, build a
//!   [`ComptimeValue`] whose payload is a stable string representation of the
//!   type. This is the foundation for typed `type_info()`-style results.
//!
//! ## What stays NaN-boxed (and why)
//!
//! Several comptime constructs intentionally retain raw `ValueWord` payloads:
//!
//! 1. **Comptime annotation targets**. `ComptimeTarget::to_nanboxed()` builds
//!    a `TypedObject` describing functions/types/modules. Until Phase 4
//!    rewrites the annotation handler interface, these targets stay NaN-boxed
//!    so handlers can call into the existing `typed_object_to_hashmap_nb`
//!    machinery. We *can* tag them with `ConcreteType::Struct(_)` via
//!    [`ComptimeValue::with_concrete`], but the payload itself remains a
//!    `ValueWord`.
//!
//! 2. **`comptime for` element values**. When the iterator is unrolled, each
//!    element is currently spliced back into the host program as an AST
//!    literal via [`super::comptime::nb_to_literal`]. Walking through a
//!    `ComptimeValue` lets the unroller stamp a `ConcreteType` on the loop
//!    variable, but the literal it splices is still derived from the
//!    NaN-boxed payload.
//!
//! 3. **Extension function results**. Extension functions registered via
//!    `ModuleExports` return `ValueWord`. We can wrap their result in a
//!    `ComptimeValue` after the call, but the function signatures themselves
//!    remain NaN-boxed until the v2 ABI rewrite.
//!
//! 4. **Comptime directives**. Directives like `Extend`, `SetParamType`,
//!    `ReplaceBody` are AST-level — they don't carry runtime values, so they
//!    don't go through this bridge.
//!
//! Phase 4 will collapse all of these onto raw typed pointers, at which
//! point the `Option<ConcreteType>` tag becomes mandatory and the
//! `ValueWord` field disappears.

use shape_ast::ast::TypeAnnotation;
use shape_runtime::type_system::annotation_to_concrete;
use shape_value::ValueWord;
use shape_value::v2::ConcreteType;
use std::sync::Arc;

/// Typed view over a comptime evaluation result.
///
/// In v2 Phase 5 we want comptime results to flow as [`ConcreteType`]-tagged
/// values rather than opaque [`ValueWord`]s. Because the comptime mini-VM is
/// still NaN-boxed (Phase 4 will fix that), `ComptimeValue` keeps the
/// `ValueWord` payload alongside an optional `ConcreteType` tag.
///
/// - **Tagged values** (`concrete = Some(_)`) have a known compile-time type
///   the rest of the pipeline can monomorphize against.
/// - **Untagged values** (`concrete = None`) fall back to NaN-box
///   introspection. This is the pre-Phase-4 escape hatch.
#[derive(Debug, Clone)]
pub struct ComptimeValue {
    /// NaN-boxed payload — the wire format the comptime mini-VM still speaks.
    pub value: ValueWord,
    /// Optional resolved type. `None` means "we don't know the concrete type
    /// at this site; please introspect the `ValueWord`".
    pub concrete: Option<ConcreteType>,
}

impl ComptimeValue {
    /// Wrap a `ValueWord` with no known concrete type. The bridge functions
    /// will fall back to NaN-box introspection on this value.
    pub fn from_value_word(value: ValueWord) -> Self {
        Self {
            value,
            concrete: None,
        }
    }

    /// Wrap a `ValueWord` together with an explicit `ConcreteType` tag.
    pub fn with_concrete(value: ValueWord, concrete: ConcreteType) -> Self {
        Self {
            value,
            concrete: Some(concrete),
        }
    }

    /// Construct a typed string `ComptimeValue` from a Rust `&str`.
    /// This is the canonical way to encode a type-name result.
    pub fn from_string(s: &str) -> Self {
        Self::with_concrete(
            ValueWord::from_string(Arc::new(s.to_string())),
            ConcreteType::String,
        )
    }

    /// Construct a typed integer `ComptimeValue`.
    pub fn from_i64(v: i64) -> Self {
        Self::with_concrete(ValueWord::from_i64(v), ConcreteType::I64)
    }

    /// Construct a typed number (f64) `ComptimeValue`.
    pub fn from_f64(v: f64) -> Self {
        Self::with_concrete(ValueWord::from_f64(v), ConcreteType::F64)
    }

    /// Construct a typed boolean `ComptimeValue`.
    pub fn from_bool(v: bool) -> Self {
        Self::with_concrete(ValueWord::from_bool(v), ConcreteType::Bool)
    }

    /// Whether this value has a known concrete type. Useful for callers that
    /// want to fail fast on un-tagged values once Phase 4 lands.
    pub fn is_typed(&self) -> bool {
        self.concrete.is_some()
    }
}

/// Bridge: extract a `ConcreteType` from a `ComptimeValue`.
///
/// This function is the v2 Phase 5 single source of truth for "what type
/// does this comptime value have". Resolution order:
///
/// 1. If the value carries an explicit `ConcreteType` tag, return it.
/// 2. Otherwise, introspect the underlying `ValueWord` and infer a tag from
///    its NaN-box discriminant. This handles legacy values that still flow
///    through the comptime pipeline untagged.
/// 3. If neither path can decide, return `None` (the caller must keep the
///    NaN-boxed fallback path active until Phase 4).
pub fn comptime_value_to_concrete_type(val: &ComptimeValue) -> Option<ConcreteType> {
    if let Some(ct) = &val.concrete {
        return Some(ct.clone());
    }
    nb_to_concrete_type(&val.value)
}

/// Best-effort `ValueWord` → `ConcreteType` mapping.
///
/// This is the fallback used when a [`ComptimeValue`] arrives untagged. It
/// only considers shapes the v2 typed runtime can already represent — things
/// like raw closures, host closures, type annotations and tables stay
/// `None`, deferring to the NaN-box path.
pub fn nb_to_concrete_type(nb: &ValueWord) -> Option<ConcreteType> {
    use shape_value::heap_value::HeapValue;

    if nb.is_none() {
        // Bare null collapses to `void` for typing purposes; callers that
        // care about Option-ness should keep the explicit tag instead.
        return Some(ConcreteType::Option(Box::new(ConcreteType::Void)));
    }
    if nb.is_unit() {
        return Some(ConcreteType::Void);
    }
    if nb.as_bool().is_some() {
        return Some(ConcreteType::Bool);
    }
    if nb.as_i64().is_some() {
        return Some(ConcreteType::I64);
    }
    if nb.as_f64().is_some() {
        return Some(ConcreteType::F64);
    }
    if nb.as_str().is_some() {
        return Some(ConcreteType::String);
    }
    if nb.as_decimal().is_some() {
        return Some(ConcreteType::Decimal);
    }

    if let Some(view) = nb.as_any_array() {
        // Probe the first element. Empty arrays cannot be typed precisely
        // here — Phase 4 will require a stored element type.
        if let Some(first) = view.get_nb(0) {
            if let Some(elem_ct) = nb_to_concrete_type(&first) {
                return Some(ConcreteType::Array(Box::new(elem_ct)));
            }
        }
        return Some(ConcreteType::Array(Box::new(ConcreteType::Void)));
    }

    if let Some(heap) = nb.as_heap_ref() {
        return match heap {
            HeapValue::TypedObject { .. } => {
                // We don't have enough info to recover the StructLayoutId
                // from a comptime TypedObject without consulting the bytecode
                // schema registry. Phase 4 will plumb the schema id through;
                // until then, return a placeholder Struct id so callers know
                // it's a struct shape.
                Some(ConcreteType::Struct(shape_value::v2::StructLayoutId(0)))
            }
            HeapValue::String(_) => Some(ConcreteType::String),
            HeapValue::TypeAnnotation(ann) => annotation_to_concrete(ann).ok(),
            _ => None,
        };
    }

    None
}

/// Bridge: build a `ComptimeValue` from a `ConcreteType`.
///
/// This is what `type_info()`-style typed comptime queries call. The
/// resulting `ComptimeValue`:
/// - has its `concrete` field set to `ConcreteType::String` (the kind
///   identifier that callers like `.name` would consume),
/// - has its `value` field set to a `ValueWord` carrying the canonical
///   `Display` form of the input type (`"int"`, `"number"`, `"Array<int>"`,
///   …).
///
/// This matches the typed-result shape the v2 design wants: a string-typed
/// `ConcreteType::String` with a stable payload that downstream code can
/// pattern-match on.
pub fn concrete_type_to_comptime_value(ct: &ConcreteType) -> ComptimeValue {
    ComptimeValue::from_string(&ct.to_string())
}

/// Build a typed comptime value from a `TypeAnnotation`.
///
/// This is the entry point for `type_info(T)`-style calls in user code:
/// the type annotation is resolved through `annotation_to_concrete` and then
/// re-exposed as a typed `ComptimeValue`.
///
/// Returns `None` if the annotation cannot be resolved to a `ConcreteType`
/// (e.g. unions, intersections, dyn). Callers should fall back to the
/// NaN-boxed value path in that case.
pub fn type_annotation_to_comptime_value(ann: &TypeAnnotation) -> Option<ComptimeValue> {
    let ct = annotation_to_concrete(ann).ok()?;
    Some(concrete_type_to_comptime_value(&ct))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::ast::type_path::TypePath;
    use shape_value::v2::ConcreteType;

    // ------------------------------------------------------------------
    // ComptimeValue constructors
    // ------------------------------------------------------------------

    #[test]
    fn ctor_from_string_is_typed() {
        let v = ComptimeValue::from_string("int");
        assert!(v.is_typed());
        assert_eq!(v.concrete, Some(ConcreteType::String));
        assert_eq!(v.value.as_str(), Some("int"));
    }

    #[test]
    fn ctor_from_i64_is_typed() {
        let v = ComptimeValue::from_i64(42);
        assert_eq!(v.concrete, Some(ConcreteType::I64));
        assert_eq!(v.value.as_i64(), Some(42));
    }

    #[test]
    fn ctor_from_f64_is_typed() {
        let v = ComptimeValue::from_f64(3.14);
        assert_eq!(v.concrete, Some(ConcreteType::F64));
        assert_eq!(v.value.as_f64(), Some(3.14));
    }

    #[test]
    fn ctor_from_bool_is_typed() {
        let v = ComptimeValue::from_bool(true);
        assert_eq!(v.concrete, Some(ConcreteType::Bool));
        assert_eq!(v.value.as_bool(), Some(true));
    }

    #[test]
    fn ctor_from_value_word_is_untyped() {
        let v = ComptimeValue::from_value_word(ValueWord::from_i64(7));
        assert!(!v.is_typed());
    }

    // ------------------------------------------------------------------
    // Bridge: ComptimeValue → ConcreteType
    // ------------------------------------------------------------------

    #[test]
    fn bridge_uses_explicit_tag_first() {
        // The explicit tag wins even if it disagrees with the NaN-box content.
        // This lets callers force a specific shape (e.g. tag a u8 buffer that
        // happens to round-trip through f64).
        let v = ComptimeValue::with_concrete(ValueWord::from_i64(0), ConcreteType::U8);
        assert_eq!(comptime_value_to_concrete_type(&v), Some(ConcreteType::U8));
    }

    #[test]
    fn bridge_falls_back_to_nb_inspection_for_int() {
        let v = ComptimeValue::from_value_word(ValueWord::from_i64(42));
        assert_eq!(comptime_value_to_concrete_type(&v), Some(ConcreteType::I64));
    }

    #[test]
    fn bridge_falls_back_to_nb_inspection_for_string() {
        let v = ComptimeValue::from_value_word(ValueWord::from_string(Arc::new("hi".into())));
        assert_eq!(
            comptime_value_to_concrete_type(&v),
            Some(ConcreteType::String)
        );
    }

    #[test]
    fn bridge_falls_back_for_bool_and_f64() {
        assert_eq!(
            comptime_value_to_concrete_type(&ComptimeValue::from_value_word(ValueWord::from_bool(
                false
            ))),
            Some(ConcreteType::Bool)
        );
        assert_eq!(
            comptime_value_to_concrete_type(&ComptimeValue::from_value_word(ValueWord::from_f64(
                2.5
            ))),
            Some(ConcreteType::F64)
        );
    }

    #[test]
    fn bridge_handles_typed_array_via_nb() {
        let arr = Arc::new(vec![ValueWord::from_i64(1), ValueWord::from_i64(2)]);
        let nb = ValueWord::from_array(arr);
        let v = ComptimeValue::from_value_word(nb);
        // The result should be Array<i64> via element introspection.
        assert_eq!(
            comptime_value_to_concrete_type(&v),
            Some(ConcreteType::Array(Box::new(ConcreteType::I64)))
        );
    }

    #[test]
    fn bridge_handles_unit_and_none() {
        let unit = ComptimeValue::from_value_word(ValueWord::unit());
        assert_eq!(
            comptime_value_to_concrete_type(&unit),
            Some(ConcreteType::Void)
        );

        let none = ComptimeValue::from_value_word(ValueWord::none());
        assert_eq!(
            comptime_value_to_concrete_type(&none),
            Some(ConcreteType::Option(Box::new(ConcreteType::Void)))
        );
    }

    // ------------------------------------------------------------------
    // Bridge: ConcreteType → ComptimeValue
    // ------------------------------------------------------------------

    #[test]
    fn reverse_bridge_int_returns_typed_string() {
        let v = concrete_type_to_comptime_value(&ConcreteType::I64);
        assert_eq!(v.concrete, Some(ConcreteType::String));
        assert_eq!(v.value.as_str(), Some("int"));
    }

    #[test]
    fn reverse_bridge_number_returns_typed_string() {
        let v = concrete_type_to_comptime_value(&ConcreteType::F64);
        assert_eq!(v.concrete, Some(ConcreteType::String));
        assert_eq!(v.value.as_str(), Some("number"));
    }

    #[test]
    fn reverse_bridge_array_of_number_returns_array_number_string() {
        let v = concrete_type_to_comptime_value(&ConcreteType::Array(Box::new(ConcreteType::F64)));
        assert_eq!(v.concrete, Some(ConcreteType::String));
        assert_eq!(v.value.as_str(), Some("Array<number>"));
    }

    #[test]
    fn reverse_bridge_option_int_returns_int_questionmark() {
        let v = concrete_type_to_comptime_value(&ConcreteType::Option(Box::new(ConcreteType::I64)));
        assert_eq!(v.value.as_str(), Some("int?"));
    }

    // ------------------------------------------------------------------
    // type_annotation_to_comptime_value end-to-end
    // ------------------------------------------------------------------

    #[test]
    fn comptime_type_info_int_name_is_int_string() {
        // `comptime { type_info(int).name }` — the `.name` access returns the
        // canonical type name as a `ConcreteType::String`.
        let ann = TypeAnnotation::Basic("int".into());
        let v = type_annotation_to_comptime_value(&ann).expect("int annotation should resolve");
        assert_eq!(v.concrete, Some(ConcreteType::String));
        assert_eq!(v.value.as_str(), Some("int"));
    }

    #[test]
    fn comptime_type_info_array_f64_element_type() {
        // `comptime { type_info(Array<f64>).element_type }` — the
        // element-type query collapses to `number` in canonical form, with
        // `ConcreteType::F64` as the underlying element representation.
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Basic("f64".into())],
        };
        // First: the whole-type comptime value.
        let whole = type_annotation_to_comptime_value(&ann).expect("Array<f64> resolves");
        assert_eq!(whole.value.as_str(), Some("Array<number>"));

        // Then: extract the element type via the bridge directly to verify
        // the ConcreteType representation a typed runtime would consume.
        let resolved = annotation_to_concrete(&ann).expect("annotation_to_concrete works");
        match resolved {
            ConcreteType::Array(elem) => {
                assert_eq!(*elem, ConcreteType::F64);
                let elem_value = concrete_type_to_comptime_value(&elem);
                assert_eq!(elem_value.value.as_str(), Some("number"));
                assert_eq!(elem_value.concrete, Some(ConcreteType::String));
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn comptime_for_field_iteration_yields_typed_field_info() {
        // Stand-in for `comptime for field in MyStruct.fields { field.type }`.
        // We simulate the field descriptors a comptime for-loop would
        // produce, then verify each one becomes a typed ComptimeValue when
        // walked through the bridge.
        let fields = [
            ("x", TypeAnnotation::Basic("number".into())),
            ("y", TypeAnnotation::Basic("number".into())),
            ("name", TypeAnnotation::Basic("string".into())),
        ];

        let typed_fields: Vec<(&str, ComptimeValue)> = fields
            .iter()
            .map(|(n, ann)| {
                let cv = type_annotation_to_comptime_value(ann)
                    .expect("field type annotation must resolve");
                (*n, cv)
            })
            .collect();

        // Every iteration produces a typed (string) ComptimeValue whose
        // payload is the canonical type name. That's exactly what a typed
        // `comptime for field in ...` body would consume.
        assert_eq!(typed_fields.len(), 3);
        for (name, cv) in &typed_fields {
            assert!(cv.is_typed(), "field '{}' should be typed", name);
            assert_eq!(cv.concrete, Some(ConcreteType::String));
        }
        assert_eq!(typed_fields[0].1.value.as_str(), Some("number"));
        assert_eq!(typed_fields[1].1.value.as_str(), Some("number"));
        assert_eq!(typed_fields[2].1.value.as_str(), Some("string"));
    }

    #[test]
    fn type_annotation_to_comptime_value_returns_none_for_unions() {
        // Unions can't be reduced to a single ConcreteType (per
        // `concrete_conv` rules), so the bridge has to fail soft.
        let ann = TypeAnnotation::Union(vec![
            TypeAnnotation::Basic("int".into()),
            TypeAnnotation::Basic("string".into()),
        ]);
        assert!(type_annotation_to_comptime_value(&ann).is_none());
    }
}
