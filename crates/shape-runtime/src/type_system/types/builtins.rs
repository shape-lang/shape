//! Built-in Types
//!
//! Defines constructors for common built-in types in Shape.

use super::core::Type;
use shape_ast::ast::TypeAnnotation;

/// Built-in types for Shape
pub struct BuiltinTypes;

impl BuiltinTypes {
    pub fn number() -> Type {
        Type::Concrete(TypeAnnotation::Basic("number".to_string()))
    }

    pub fn integer() -> Type {
        Type::Concrete(TypeAnnotation::Basic("int".to_string()))
    }

    pub fn string() -> Type {
        Type::Concrete(TypeAnnotation::Basic("string".to_string()))
    }

    pub fn boolean() -> Type {
        Type::Concrete(TypeAnnotation::Basic("bool".to_string()))
    }

    pub fn row() -> Type {
        Type::Concrete(TypeAnnotation::Basic("row".to_string()))
    }

    pub fn pattern() -> Type {
        Type::Concrete(TypeAnnotation::Basic("pattern".to_string()))
    }

    pub fn void() -> Type {
        Type::Concrete(TypeAnnotation::Void)
    }

    pub fn null() -> Type {
        Type::Concrete(TypeAnnotation::Null)
    }

    pub fn array(element_type: Type) -> Type {
        Type::Concrete(TypeAnnotation::Array(Box::new(
            element_type.to_annotation().unwrap_or_else(|| TypeAnnotation::Basic("unknown".to_string())),
        )))
    }

    pub fn any() -> Type {
        Type::Variable(super::core::TypeVar::fresh())
    }

    /// Canonical runtime numeric type for aliases and width-aware native names.
    ///
    /// This preserves scripting ergonomics (`int`, `number`) while making
    /// concrete width mapping explicit:
    /// - `int`/`integer` -> `i64`
    /// - `number`/`float` -> `f64`
    /// - `byte` -> `u8`
    /// - `char` -> `i8`
    pub fn canonical_numeric_runtime_name(name: &str) -> Option<&'static str> {
        match name {
            "number" | "Number" | "float" | "Float" | "f64" => Some("f64"),
            "f32" => Some("f32"),
            "int" | "Int" | "integer" | "Integer" | "i64" => Some("i64"),
            "i32" => Some("i32"),
            "i16" => Some("i16"),
            "i8" | "char" => Some("i8"),
            "u64" => Some("u64"),
            "u32" => Some("u32"),
            "u16" => Some("u16"),
            "u8" | "byte" => Some("u8"),
            "isize" => Some("isize"),
            "usize" => Some("usize"),
            _ => None,
        }
    }

    /// Whether the name belongs to the integer family (including width-aware aliases).
    pub fn is_integer_type_name(name: &str) -> bool {
        matches!(
            Self::canonical_numeric_runtime_name(name),
            Some("i8" | "u8" | "i16" | "u16" | "i32" | "i64" | "u32" | "u64" | "isize" | "usize")
        )
    }

    /// Whether the name belongs to the floating-point family.
    pub fn is_number_type_name(name: &str) -> bool {
        matches!(
            Self::canonical_numeric_runtime_name(name),
            Some("f32" | "f64")
        )
    }

    pub fn is_bool_type_name(name: &str) -> bool {
        matches!(name, "bool" | "Bool" | "boolean" | "Boolean")
    }

    pub fn is_string_type_name(name: &str) -> bool {
        matches!(name, "string" | "String")
    }

    /// Canonical script-facing alias used in conversion trait selectors.
    pub fn canonical_script_alias(name: &str) -> Option<&'static str> {
        if Self::is_bool_type_name(name) {
            return Some("bool");
        }
        if Self::is_string_type_name(name) {
            return Some("string");
        }
        if Self::is_integer_type_name(name) {
            return Some("int");
        }
        if Self::is_number_type_name(name) {
            return Some("number");
        }
        match name {
            "Decimal" | "decimal" => Some("decimal"),
            _ => None,
        }
    }

    /// Check if a type name represents a numeric type
    pub fn is_numeric_type_name(name: &str) -> bool {
        Self::canonical_numeric_runtime_name(name).is_some()
            || matches!(name, "decimal" | "Decimal")
    }

    pub fn function(params: Vec<Type>, returns: Type) -> Type {
        Type::Function {
            params,
            returns: Box::new(returns),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BuiltinTypes;

    #[test]
    fn canonical_numeric_runtime_aliases_are_width_explicit() {
        assert_eq!(
            BuiltinTypes::canonical_numeric_runtime_name("int"),
            Some("i64")
        );
        assert_eq!(
            BuiltinTypes::canonical_numeric_runtime_name("number"),
            Some("f64")
        );
        assert_eq!(
            BuiltinTypes::canonical_numeric_runtime_name("byte"),
            Some("u8")
        );
        assert_eq!(
            BuiltinTypes::canonical_numeric_runtime_name("char"),
            Some("i8")
        );
    }

    #[test]
    fn canonical_script_alias_keeps_int_and_number_ergonomics() {
        assert_eq!(BuiltinTypes::canonical_script_alias("i16"), Some("int"));
        assert_eq!(BuiltinTypes::canonical_script_alias("u64"), Some("int"));
        assert_eq!(BuiltinTypes::canonical_script_alias("f32"), Some("number"));
        assert_eq!(BuiltinTypes::canonical_script_alias("bool"), Some("bool"));
        assert_eq!(
            BuiltinTypes::canonical_script_alias("string"),
            Some("string")
        );
    }
}
