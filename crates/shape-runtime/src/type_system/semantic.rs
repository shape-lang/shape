//! Semantic Types
//!
//! Defines the types that users see and work with in Shape code.
//! These are separate from storage types (how data is physically represented).
//!
//! Key design:
//! - `Option<f64>` is a semantic type - user sees it as nullable
//! - Storage may use NaN sentinel instead of tagged union
//! - `Result<T>` defaults to universal Error (no explicit E required)

use shape_wire::metadata::{FieldInfo, TypeInfo, TypeKind};
use std::fmt;
use std::hash::Hash;

/// Semantic type identifier for type variables during inference
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeVarId(pub u32);

/// Semantic types - what the user sees in type annotations
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SemanticType {
    // === Primitives ===
    /// Floating-point number (f64)
    Number,
    /// Integer (i64)
    Integer,
    /// Boolean
    Bool,
    /// String
    String,

    // === Generic Containers ===
    /// Optional value: Option<T>
    /// - For numeric T: Uses NaN sentinel in storage
    /// - For other T: Uses discriminated union
    Option(Box<SemanticType>),

    /// Result type: Result<T> or Result<T, E>
    /// - err_type = None means universal Error type
    Result {
        ok_type: Box<SemanticType>,
        err_type: Option<Box<SemanticType>>,
    },

    /// Array of values: Vec<T>
    Array(Box<SemanticType>),

    // === User-Defined Types ===
    /// Struct type with name and fields
    Struct {
        name: String,
        fields: Vec<(String, SemanticType)>,
    },

    /// Enum type with variants
    Enum {
        name: String,
        variants: Vec<EnumVariant>,
        type_params: Vec<String>,
    },

    /// Interface/trait type
    Interface {
        name: String,
        methods: Vec<(String, FunctionSignature)>,
    },

    // === Type System Internals ===
    /// Type variable for inference (α, β, γ)
    TypeVar(TypeVarId),

    /// Named type reference (before resolution)
    Named(String),

    /// Generic type instantiation: MyType<A, B>
    Generic {
        name: String,
        args: Vec<SemanticType>,
    },

    // === Special Types ===
    /// Bottom type - computation that never returns (e.g., panic, infinite loop)
    Never,

    /// Top type - escape hatch (discouraged)
    Any,

    /// Void - no value
    Void,

    /// Function type
    Function(Box<FunctionSignature>),
}

/// Function signature
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FunctionSignature {
    pub params: Vec<FunctionParam>,
    pub return_type: SemanticType,
    pub is_fallible: bool, // True if function uses ? operator
}

/// Function parameter
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FunctionParam {
    pub name: Option<String>,
    pub param_type: SemanticType,
    pub optional: bool,
}

/// Enum variant
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EnumVariant {
    pub name: String,
    pub payload: Option<SemanticType>,
}

impl SemanticType {
    // === Constructors ===

    /// Create Option<T> type
    pub fn option(inner: SemanticType) -> Self {
        SemanticType::Option(Box::new(inner))
    }

    /// Create Result<T> type with universal Error
    pub fn result(ok_type: SemanticType) -> Self {
        SemanticType::Result {
            ok_type: Box::new(ok_type),
            err_type: None,
        }
    }

    /// Create Result<T, E> type with specific error
    pub fn result_with_error(ok_type: SemanticType, err_type: SemanticType) -> Self {
        SemanticType::Result {
            ok_type: Box::new(ok_type),
            err_type: Some(Box::new(err_type)),
        }
    }

    /// Create Vec<T> type
    pub fn array(element: SemanticType) -> Self {
        SemanticType::Array(Box::new(element))
    }

    /// Create function type
    pub fn function(params: Vec<SemanticType>, return_type: SemanticType) -> Self {
        SemanticType::Function(Box::new(FunctionSignature {
            params: params
                .into_iter()
                .map(|t| FunctionParam {
                    name: None,
                    param_type: t,
                    optional: false,
                })
                .collect(),
            return_type,
            is_fallible: false,
        }))
    }

    // === Type Queries ===

    /// Check if type is numeric (for propagating operators)
    pub fn is_numeric(&self) -> bool {
        self.is_number_family() || self.is_integer_family()
    }

    /// Check if this semantic type is in the integer family.
    pub fn is_integer_family(&self) -> bool {
        match self {
            SemanticType::Integer => true,
            SemanticType::Named(name) => matches!(
                name.as_str(),
                "i8" | "u8" | "i16" | "u16" | "i32" | "i64" | "u32" | "u64" | "isize" | "usize"
            ),
            _ => false,
        }
    }

    /// Check if this semantic type is in the floating-point family.
    pub fn is_number_family(&self) -> bool {
        match self {
            SemanticType::Number => true,
            SemanticType::Named(name) => matches!(name.as_str(), "f32" | "f64"),
            _ => false,
        }
    }

    /// Check if type is optional
    pub fn is_option(&self) -> bool {
        matches!(self, SemanticType::Option(_))
    }

    /// Check if type is a result
    pub fn is_result(&self) -> bool {
        matches!(self, SemanticType::Result { .. })
    }

    /// Get inner type of Option<T>
    pub fn option_inner(&self) -> Option<&SemanticType> {
        match self {
            SemanticType::Option(inner) => Some(inner),
            _ => None,
        }
    }

    /// Get ok type of Result<T, E>
    pub fn result_ok_type(&self) -> Option<&SemanticType> {
        match self {
            SemanticType::Result { ok_type, .. } => Some(ok_type),
            _ => None,
        }
    }

    /// Check if type contains unresolved type variables
    pub fn has_type_vars(&self) -> bool {
        match self {
            SemanticType::TypeVar(_) => true,
            SemanticType::Option(inner) => inner.has_type_vars(),
            SemanticType::Result { ok_type, err_type } => {
                ok_type.has_type_vars() || err_type.as_ref().is_some_and(|e| e.has_type_vars())
            }
            SemanticType::Array(elem) => elem.has_type_vars(),
            SemanticType::Struct { fields, .. } => fields.iter().any(|(_, t)| t.has_type_vars()),
            SemanticType::Enum { variants, .. } => variants
                .iter()
                .any(|v| v.payload.as_ref().is_some_and(|t| t.has_type_vars())),
            SemanticType::Function(sig) => {
                sig.params.iter().any(|p| p.param_type.has_type_vars())
                    || sig.return_type.has_type_vars()
            }
            SemanticType::Generic { args, .. } => args.iter().any(|a| a.has_type_vars()),
            _ => false,
        }
    }

    // === Wire Protocol Conversion ===

    /// Convert semantic type to wire protocol TypeInfo
    ///
    /// This bridges the compile-time type system with the wire format
    /// used for REPL display and external tool integration.
    pub fn to_type_info(&self) -> TypeInfo {
        match self {
            SemanticType::Number => TypeInfo::number(),
            SemanticType::Integer => TypeInfo::integer(),
            SemanticType::Bool => TypeInfo::bool(),
            SemanticType::String => TypeInfo::string(),

            SemanticType::Option(inner) => {
                let inner_info = inner.to_type_info();
                TypeInfo {
                    name: format!("Option<{}>", inner_info.name),
                    kind: TypeKind::Optional,
                    fields: None,
                    generic_params: Some(vec![inner_info]),
                    variants: None,
                    description: None,
                    metadata: None,
                }
            }

            SemanticType::Result { ok_type, err_type } => {
                let ok_info = ok_type.to_type_info();
                let name = match err_type {
                    Some(e) => format!("Result<{}, {}>", ok_info.name, e.to_type_info().name),
                    None => format!("Result<{}>", ok_info.name),
                };
                let mut params = vec![ok_info];
                if let Some(e) = err_type {
                    params.push(e.to_type_info());
                }
                TypeInfo {
                    name,
                    kind: TypeKind::Result,
                    fields: None,
                    generic_params: Some(params),
                    variants: None,
                    description: None,
                    metadata: None,
                }
            }

            SemanticType::Array(elem) => TypeInfo::array(elem.to_type_info()),

            SemanticType::Struct { name, fields } => {
                let field_infos: Vec<FieldInfo> = fields
                    .iter()
                    .map(|(fname, ftype)| FieldInfo::required(fname, ftype.to_type_info()))
                    .collect();
                TypeInfo::object(name, field_infos)
            }

            SemanticType::Enum { name, variants, .. } => {
                let variant_names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
                TypeInfo {
                    name: name.clone(),
                    kind: TypeKind::Enum,
                    fields: None,
                    generic_params: None,
                    variants: Some(variant_names),
                    description: None,
                    metadata: None,
                }
            }

            SemanticType::Interface { name, .. } => TypeInfo::primitive(name),

            SemanticType::TypeVar(id) => TypeInfo::primitive(format!("T{}", id.0)),

            SemanticType::Named(name) => TypeInfo::primitive(name),

            SemanticType::Generic { name, args } => {
                let arg_infos: Vec<TypeInfo> = args.iter().map(|a| a.to_type_info()).collect();
                let arg_names: Vec<String> = arg_infos.iter().map(|a| a.name.clone()).collect();
                TypeInfo {
                    name: format!("{}<{}>", name, arg_names.join(", ")),
                    kind: TypeKind::Object, // Generic types are object-like
                    fields: None,
                    generic_params: Some(arg_infos),
                    variants: None,
                    description: None,
                    metadata: None,
                }
            }

            SemanticType::Never => TypeInfo::primitive("Never"),
            SemanticType::Any => TypeInfo::primitive("Any"),
            SemanticType::Void => TypeInfo::null(),

            SemanticType::Function(sig) => {
                let param_types: Vec<String> = sig
                    .params
                    .iter()
                    .map(|p| p.param_type.to_type_info().name)
                    .collect();
                let ret_type = sig.return_type.to_type_info().name;
                TypeInfo {
                    name: format!("({}) -> {}", param_types.join(", "), ret_type),
                    kind: TypeKind::Function,
                    fields: None,
                    generic_params: None,
                    variants: None,
                    description: None,
                    metadata: None,
                }
            }
        }
    }
}

impl fmt::Display for SemanticType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SemanticType::Number => write!(f, "Number"),
            SemanticType::Integer => write!(f, "Integer"),
            SemanticType::Bool => write!(f, "Bool"),
            SemanticType::String => write!(f, "String"),
            SemanticType::Option(inner) => write!(f, "Option<{}>", inner),
            SemanticType::Result { ok_type, err_type } => match err_type {
                Some(e) => write!(f, "Result<{}, {}>", ok_type, e),
                None => write!(f, "Result<{}>", ok_type),
            },
            SemanticType::Array(elem) => write!(f, "Vec<{}>", elem),
            SemanticType::Struct { name, .. } => write!(f, "{}", name),
            SemanticType::Enum { name, .. } => write!(f, "{}", name),
            SemanticType::Interface { name, .. } => write!(f, "{}", name),
            SemanticType::TypeVar(id) => write!(f, "T{}", id.0),
            SemanticType::Named(name) => write!(f, "{}", name),
            SemanticType::Generic { name, args } => {
                write!(f, "{}<", name)?;
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", arg)?;
                }
                write!(f, ">")
            }
            SemanticType::Never => write!(f, "Never"),
            SemanticType::Any => write!(f, "Any"),
            SemanticType::Void => write!(f, "Void"),
            SemanticType::Function(sig) => {
                write!(f, "(")?;
                for (i, param) in sig.params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param.param_type)?;
                }
                write!(f, ") -> {}", sig.return_type)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_option_creation() {
        let opt = SemanticType::option(SemanticType::Number);
        assert!(opt.is_option());
        assert_eq!(opt.option_inner(), Some(&SemanticType::Number));
    }

    #[test]
    fn test_result_creation() {
        let res = SemanticType::result(SemanticType::Number);
        assert!(res.is_result());
        match &res {
            SemanticType::Result { ok_type, err_type } => {
                assert_eq!(**ok_type, SemanticType::Number);
                assert!(err_type.is_none());
            }
            _ => panic!("Expected Result"),
        }
    }

    #[test]
    fn test_display() {
        assert_eq!(format!("{}", SemanticType::Number), "Number");
        assert_eq!(
            format!("{}", SemanticType::option(SemanticType::Number)),
            "Option<Number>"
        );
        assert_eq!(
            format!("{}", SemanticType::result(SemanticType::String)),
            "Result<String>"
        );
    }

    #[test]
    fn test_to_type_info_primitives() {
        let num = SemanticType::Number.to_type_info();
        assert_eq!(num.name, "Number");
        assert_eq!(num.kind, TypeKind::Primitive);

        let int = SemanticType::Integer.to_type_info();
        assert_eq!(int.name, "Integer");

        let bool_t = SemanticType::Bool.to_type_info();
        assert_eq!(bool_t.name, "Bool");

        let string_t = SemanticType::String.to_type_info();
        assert_eq!(string_t.name, "String");
    }

    #[test]
    fn test_to_type_info_option() {
        let opt = SemanticType::option(SemanticType::Number).to_type_info();
        assert_eq!(opt.name, "Option<Number>");
        assert_eq!(opt.kind, TypeKind::Optional);
        assert!(opt.generic_params.is_some());
        assert_eq!(opt.generic_params.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_to_type_info_result() {
        let res = SemanticType::result(SemanticType::String).to_type_info();
        assert_eq!(res.name, "Result<String>");
        assert_eq!(res.kind, TypeKind::Result);

        let res_with_err =
            SemanticType::result_with_error(SemanticType::Number, SemanticType::String)
                .to_type_info();
        assert_eq!(res_with_err.name, "Result<Number, String>");
    }

    #[test]
    fn test_to_type_info_array() {
        let arr = SemanticType::array(SemanticType::Bool).to_type_info();
        assert_eq!(arr.name, "Array<Bool>");
        assert_eq!(arr.kind, TypeKind::Array);
    }

    #[test]
    fn test_named_width_numeric_families() {
        assert!(SemanticType::Named("i16".to_string()).is_numeric());
        assert!(SemanticType::Named("u64".to_string()).is_integer_family());
        assert!(SemanticType::Named("f32".to_string()).is_number_family());
        assert!(SemanticType::Named("f64".to_string()).is_numeric());
        assert!(!SemanticType::Named("Candle".to_string()).is_numeric());
    }
}
