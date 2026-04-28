//! Typed comptime constant value (sweep phase 4d).
//!
//! Phase 4d eliminates `ValueWord` from the comptime constant carrier. Where
//! the previous shape stored a NaN-boxed `ValueWord` plus an *optional*
//! `ConcreteType` tag, [`ConstantValue`] is a typed sum: each variant carries
//! both the runtime data and its concrete type by construction.
//!
//! ## Design
//!
//! ```text
//! pub enum ConstantValue {
//!     I64(i64),
//!     F64(f64),
//!     Bool(bool),
//!     String(Arc<str>),
//!     Array(ConcreteType, Vec<ConstantValue>),
//!     Unit,
//!     None,
//!     Opaque(ConcreteType, [u8; 8]),
//! }
//! ```
//!
//! - **Scalar variants** (I64, F64, Bool, String) carry their value directly.
//!   The corresponding [`ConcreteType`] is implied by the variant.
//! - **Array(ct, elems)** carries an explicit element-type tag (so empty
//!   arrays still type-check) plus the constant elements.
//! - **Unit / None** are the two distinguished "no value" cases. `Unit`
//!   corresponds to `void`; `None` corresponds to `Option(Void)` / a
//!   `null`-shaped sentinel.
//! - **Opaque(ct, bytes)** is a temporary bridge for extension-function
//!   returns and other producer paths that haven't yet been migrated to one
//!   of the typed variants. The 8-byte payload deliberately matches the size
//!   of a `ValueWord`, so callers that still need to round-trip through
//!   NaN-boxing can do so without enlarging the representation. Future phases
//!   should narrow `Opaque` use until it can be deleted.
//!
//! ## Why this exists
//!
//! Comptime evaluation produces values that need to flow into v2 typed
//! monomorphization (`shape_value::v2::ConcreteType`). With the old
//! `ComptimeValue { value: ValueWord, concrete: Option<ConcreteType> }`
//! shape, every consumer had to either trust the optional tag or fall back
//! to NaN-box introspection. With [`ConstantValue`] the type is
//! discriminant-encoded — there is no "untyped" state.
//!
//! ## Bridge functions
//!
//! - [`ConstantValue::concrete_type`] — return the value's type. Total.
//! - [`type_name_constant`] — given a `ConcreteType`, build a
//!   `ConstantValue::String` carrying the canonical type name (used by
//!   `type_info()`-style typed comptime queries).
//! - [`type_annotation_to_constant_value`] — resolve a `TypeAnnotation` to a
//!   typed-name `ConstantValue::String`. Returns `None` for annotations that
//!   cannot be reduced to a `ConcreteType` (unions, intersections, dyn).
//!
//! ## ConcreteType namespace
//!
//! `ConstantValue` references `shape_value::v2::ConcreteType` (the
//! comprehensive runtime-visible enum), not the smaller
//! `shape_runtime::typed_module_exports::ConcreteType` used by extension
//! return-type metadata. Unifying those two enums is a separate
//! cross-cutting refactor and is deliberately out of phase 4d scope.
//!
//! ## What stays out of scope
//!
//! The comptime mini-VM in `compiler/comptime.rs` still uses raw `ValueWord`
//! internally — its `ComptimeExecutionResult.value` and `SetParamValue`
//! directives are NaN-boxed. Migrating that pipeline is a deeper rewrite
//! than fits in phase 4d; once it lands, `Opaque` becomes the bridge
//! variant that disappears, and the typed variants here become the sole
//! constant carrier.
//!
//! Until that wiring lands, the items below are exercised solely by the
//! test module — `#[allow(dead_code)]` is applied at module scope to keep
//! the strict-typing-sweep build clean.

#![allow(dead_code)]

use crate::compiler::v2_map_emission::concrete_type_from_annotation;
use shape_ast::ast::TypeAnnotation;
use shape_value::v2::ConcreteType;
use std::sync::Arc;

/// A typed comptime constant value.
///
/// Each variant carries both the runtime data and its [`ConcreteType`] by
/// construction. There is no "untyped" state — calling
/// [`ConstantValue::concrete_type`] is total.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstantValue {
    /// Signed 64-bit integer. Concrete type: [`ConcreteType::I64`].
    I64(i64),
    /// 64-bit float (default `number`). Concrete type: [`ConcreteType::F64`].
    F64(f64),
    /// Boolean. Concrete type: [`ConcreteType::Bool`].
    Bool(bool),
    /// Interned string. Concrete type: [`ConcreteType::String`].
    String(Arc<str>),
    /// Homogeneous array of constant values. The element type is carried
    /// explicitly so empty arrays still type-check.
    Array(ConcreteType, Vec<ConstantValue>),
    /// Unit (void) — produced by statements without a value.
    Unit,
    /// Distinguished `None` / null — produces `Option<Void>` typing.
    None,
    /// Bridge variant for producer paths not yet migrated to typed variants
    /// (notably extension-function returns). Carries an explicit type tag
    /// alongside an 8-byte payload — deliberately the same size as a
    /// `ValueWord` so existing NaN-box round-trips remain feasible.
    ///
    /// New code should NOT introduce `Opaque` uses; future phases will
    /// narrow this variant until it can be deleted.
    Opaque(ConcreteType, [u8; 8]),
}

impl ConstantValue {
    /// Build a `ConstantValue::String` from a Rust `&str`.
    pub fn from_str(s: &str) -> Self {
        ConstantValue::String(Arc::<str>::from(s))
    }

    /// Build a `ConstantValue::String` from an owned `String`.
    pub fn from_string(s: String) -> Self {
        ConstantValue::String(Arc::<str>::from(s.as_str()))
    }

    /// Return the value's [`ConcreteType`]. Total — every variant has a
    /// well-defined type.
    pub fn concrete_type(&self) -> ConcreteType {
        match self {
            ConstantValue::I64(_) => ConcreteType::I64,
            ConstantValue::F64(_) => ConcreteType::F64,
            ConstantValue::Bool(_) => ConcreteType::Bool,
            ConstantValue::String(_) => ConcreteType::String,
            ConstantValue::Array(elem, _) => ConcreteType::Array(Box::new(elem.clone())),
            ConstantValue::Unit => ConcreteType::Void,
            ConstantValue::None => ConcreteType::Option(Box::new(ConcreteType::Void)),
            ConstantValue::Opaque(ct, _) => ct.clone(),
        }
    }

    /// Extract the i64 payload, if this is an `I64`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ConstantValue::I64(v) => Some(*v),
            _ => None,
        }
    }

    /// Extract the f64 payload, if this is an `F64`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ConstantValue::F64(v) => Some(*v),
            _ => None,
        }
    }

    /// Extract the bool payload, if this is a `Bool`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConstantValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// Extract the string payload, if this is a `String`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ConstantValue::String(s) => Some(s.as_ref()),
            _ => None,
        }
    }

    /// Whether this value is a typed scalar / array (i.e. not `Opaque`).
    /// Useful for callers that want to fail fast once a producer stops
    /// emitting `Opaque`.
    pub fn is_typed(&self) -> bool {
        !matches!(self, ConstantValue::Opaque(_, _))
    }
}

/// Build a `ConstantValue::String` whose payload is the canonical type name
/// for `ct` (e.g. `"int"`, `"number"`, `"Array<int>"`, `"int?"`).
///
/// This is the typed analogue of the v2 Phase 5 `type_info()` query: instead
/// of returning a NaN-boxed string, it returns a typed `ConstantValue`
/// whose own concrete type is `String`.
pub fn type_name_constant(ct: &ConcreteType) -> ConstantValue {
    ConstantValue::from_string(ct.to_string())
}

/// Resolve a `TypeAnnotation` to its canonical-name `ConstantValue::String`.
///
/// Returns `None` if the annotation cannot be reduced to a `ConcreteType`
/// (e.g. unions, intersections, dyn). Callers may keep an annotation-level
/// fallback in those cases.
pub fn type_annotation_to_constant_value(ann: &TypeAnnotation) -> Option<ConstantValue> {
    let ct = concrete_type_from_annotation(ann)?;
    Some(type_name_constant(&ct))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::TypeAnnotation;
    use shape_ast::ast::type_path::TypePath;
    use shape_value::v2::ConcreteType;

    // ------------------------------------------------------------------
    // ConstantValue constructors and concrete_type
    // ------------------------------------------------------------------

    #[test]
    fn ctor_string_concrete_type_is_string() {
        let v = ConstantValue::from_str("int");
        assert_eq!(v.concrete_type(), ConcreteType::String);
        assert_eq!(v.as_str(), Some("int"));
        assert!(v.is_typed());
    }

    #[test]
    fn ctor_i64_concrete_type_is_i64() {
        let v = ConstantValue::I64(42);
        assert_eq!(v.concrete_type(), ConcreteType::I64);
        assert_eq!(v.as_i64(), Some(42));
        assert!(v.is_typed());
    }

    #[test]
    fn ctor_f64_concrete_type_is_f64() {
        let v = ConstantValue::F64(3.14);
        assert_eq!(v.concrete_type(), ConcreteType::F64);
        assert_eq!(v.as_f64(), Some(3.14));
    }

    #[test]
    fn ctor_bool_concrete_type_is_bool() {
        let v = ConstantValue::Bool(true);
        assert_eq!(v.concrete_type(), ConcreteType::Bool);
        assert_eq!(v.as_bool(), Some(true));
    }

    #[test]
    fn array_concrete_type_carries_element_type() {
        let v = ConstantValue::Array(
            ConcreteType::I64,
            vec![ConstantValue::I64(1), ConstantValue::I64(2)],
        );
        assert_eq!(
            v.concrete_type(),
            ConcreteType::Array(Box::new(ConcreteType::I64))
        );
    }

    #[test]
    fn empty_array_still_types() {
        // Empty arrays type-check via the element-type tag — there's no
        // need to introspect a (nonexistent) element.
        let v = ConstantValue::Array(ConcreteType::F64, vec![]);
        assert_eq!(
            v.concrete_type(),
            ConcreteType::Array(Box::new(ConcreteType::F64))
        );
    }

    #[test]
    fn unit_and_none_have_distinct_concrete_types() {
        assert_eq!(ConstantValue::Unit.concrete_type(), ConcreteType::Void);
        assert_eq!(
            ConstantValue::None.concrete_type(),
            ConcreteType::Option(Box::new(ConcreteType::Void))
        );
    }

    #[test]
    fn opaque_carries_explicit_type_tag() {
        // Opaque is the bridge for not-yet-migrated producer paths. The
        // explicit type tag is required and `is_typed()` reports false so
        // callers can detect remaining bridge sites.
        let v = ConstantValue::Opaque(ConcreteType::U8, [0; 8]);
        assert_eq!(v.concrete_type(), ConcreteType::U8);
        assert!(!v.is_typed());
    }

    // ------------------------------------------------------------------
    // type_name_constant — ConcreteType → typed String
    // ------------------------------------------------------------------

    #[test]
    fn type_name_int_returns_typed_string() {
        let v = type_name_constant(&ConcreteType::I64);
        assert_eq!(v.concrete_type(), ConcreteType::String);
        assert_eq!(v.as_str(), Some("int"));
    }

    #[test]
    fn type_name_number_returns_typed_string() {
        let v = type_name_constant(&ConcreteType::F64);
        assert_eq!(v.concrete_type(), ConcreteType::String);
        assert_eq!(v.as_str(), Some("number"));
    }

    #[test]
    fn type_name_array_of_number() {
        let v = type_name_constant(&ConcreteType::Array(Box::new(ConcreteType::F64)));
        assert_eq!(v.as_str(), Some("Array<number>"));
    }

    #[test]
    fn type_name_option_int() {
        let v = type_name_constant(&ConcreteType::Option(Box::new(ConcreteType::I64)));
        assert_eq!(v.as_str(), Some("int?"));
    }

    // ------------------------------------------------------------------
    // type_annotation_to_constant_value — annotation end-to-end
    // ------------------------------------------------------------------

    #[test]
    fn annotation_int_resolves_to_typed_string() {
        let ann = TypeAnnotation::Basic("int".into());
        let v = type_annotation_to_constant_value(&ann)
            .expect("int annotation should resolve");
        assert_eq!(v.concrete_type(), ConcreteType::String);
        assert_eq!(v.as_str(), Some("int"));
    }

    #[test]
    fn annotation_array_f64_resolves_to_array_number_string() {
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![TypeAnnotation::Basic("f64".into())],
        };
        let whole = type_annotation_to_constant_value(&ann).expect("Array<f64> resolves");
        assert_eq!(whole.as_str(), Some("Array<number>"));

        // And via concrete-type extraction directly:
        let resolved =
            concrete_type_from_annotation(&ann).expect("concrete_type_from_annotation works");
        match resolved {
            ConcreteType::Array(elem) => {
                assert_eq!(*elem, ConcreteType::F64);
                let elem_value = type_name_constant(&elem);
                assert_eq!(elem_value.as_str(), Some("number"));
                assert_eq!(elem_value.concrete_type(), ConcreteType::String);
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn comptime_for_field_iteration_yields_typed_field_info() {
        // Stand-in for `comptime for field in MyStruct.fields { field.type }`.
        // Each field-type annotation becomes a typed `ConstantValue::String`.
        let fields = [
            ("x", TypeAnnotation::Basic("number".into())),
            ("y", TypeAnnotation::Basic("number".into())),
            ("name", TypeAnnotation::Basic("string".into())),
        ];

        let typed_fields: Vec<(&str, ConstantValue)> = fields
            .iter()
            .map(|(n, ann)| {
                let cv = type_annotation_to_constant_value(ann)
                    .expect("field type annotation must resolve");
                (*n, cv)
            })
            .collect();

        assert_eq!(typed_fields.len(), 3);
        for (name, cv) in &typed_fields {
            assert_eq!(
                cv.concrete_type(),
                ConcreteType::String,
                "field '{}' should be typed String",
                name
            );
        }
        assert_eq!(typed_fields[0].1.as_str(), Some("number"));
        assert_eq!(typed_fields[1].1.as_str(), Some("number"));
        assert_eq!(typed_fields[2].1.as_str(), Some("string"));
    }

    #[test]
    fn annotation_to_constant_value_returns_none_for_unions() {
        // Unions can't be reduced to a single ConcreteType, so the bridge
        // has to fail soft.
        let ann = TypeAnnotation::Union(vec![
            TypeAnnotation::Basic("int".into()),
            TypeAnnotation::Basic("string".into()),
        ]);
        assert!(type_annotation_to_constant_value(&ann).is_none());
    }
}
