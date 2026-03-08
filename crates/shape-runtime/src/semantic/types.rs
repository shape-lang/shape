//! Type system for Shape

use serde::{Deserialize, Serialize};
use std::fmt;

/// Types in the Shape type system
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Type {
    /// Numeric type (float)
    Number,
    /// String type
    String,
    /// Boolean type
    Bool,
    /// Null type
    Null,
    /// Unit type (void/no value)
    Unit,
    /// Bar color type
    Color,
    /// Timestamp type (point in time, epoch milliseconds)
    Timestamp,
    /// Timeframe type
    Timeframe,
    /// Time reference
    TimeRef,
    /// Duration type
    Duration,
    /// Pattern reference
    Pattern,
    /// Object type with field types
    Object(Vec<(std::string::String, Type)>),
    /// Array type
    Array(Box<Type>),
    /// Matrix type
    Matrix(Box<Type>),
    /// Column type (vectorized column operations)
    Column(Box<Type>),
    /// Function type
    Function {
        params: Vec<Type>,
        returns: Box<Type>,
    },
    /// Module type
    Module,
    /// Range type (for iteration)
    Range(Box<Type>),
    /// Result type (fallible operations)
    /// Contains the Ok type; Error type is universal
    Result(Box<Type>),
    /// Unknown type (for type inference)
    Unknown,
    /// Error type
    Error,
}

impl Type {
    /// Check if this type can be coerced to another type
    pub fn can_coerce_to(&self, other: &Type) -> bool {
        match (self, other) {
            // Same types are always compatible
            (a, b) if a == b => true,

            // Unknown can coerce to anything
            (Type::Unknown, _) => true,
            (_, Type::Unknown) => true,

            // Error type handling
            (Type::Error, _) | (_, Type::Error) => true,

            // Numeric coercions
            (Type::Number, Type::String) => true, // Numbers can be stringified

            // Vec coercions
            (Type::Array(a), Type::Array(b)) => a.can_coerce_to(b),
            (Type::Matrix(a), Type::Matrix(b)) => a.can_coerce_to(b),
            // Matrix coercions from/to nested vectors
            (Type::Array(rows), Type::Matrix(elem)) => match rows.as_ref() {
                Type::Array(inner) => inner.can_coerce_to(elem),
                _ => false,
            },
            (Type::Matrix(elem), Type::Array(rows)) => match rows.as_ref() {
                Type::Array(inner) => elem.can_coerce_to(inner),
                _ => false,
            },
            (Type::Column(a), Type::Column(b)) => a.can_coerce_to(b),
            (Type::Range(a), Type::Range(b)) => a.can_coerce_to(b),
            (Type::Result(a), Type::Result(b)) => a.can_coerce_to(b),

            // Structural typing for objects: A coerces to B if A has all fields B requires
            (Type::Object(a_fields), Type::Object(b_fields)) => {
                // Empty object type (like a generic "row") accepts any object
                if b_fields.is_empty() {
                    return true;
                }
                // Check that all required fields in B exist in A with compatible types
                b_fields.iter().all(|(b_name, b_type)| {
                    a_fields
                        .iter()
                        .find(|(a_name, _)| a_name == b_name)
                        .map(|(_, a_type)| a_type.can_coerce_to(b_type))
                        .unwrap_or(false)
                })
            }

            _ => false,
        }
    }

    /// Get the result type of a binary operation
    pub fn binary_op_result(&self, op: &shape_ast::ast::BinaryOp, rhs: &Type) -> Type {
        use shape_ast::ast::BinaryOp;

        match op {
            // Arithmetic operations
            BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::Pow => {
                match (self, rhs) {
                    (Type::Number, Type::Number) => Type::Number,
                    (Type::String, Type::String) if matches!(op, BinaryOp::Add) => Type::String,
                    // Timestamp arithmetic
                    (Type::Timestamp, Type::Timestamp) if matches!(op, BinaryOp::Sub) => {
                        Type::Duration
                    }
                    (Type::Timestamp, Type::Duration)
                        if matches!(op, BinaryOp::Add | BinaryOp::Sub) =>
                    {
                        Type::Timestamp
                    }
                    (Type::Duration, Type::Timestamp) if matches!(op, BinaryOp::Add) => {
                        Type::Timestamp
                    }
                    // Duration arithmetic
                    (Type::Duration, Type::Duration)
                        if matches!(op, BinaryOp::Add | BinaryOp::Sub) =>
                    {
                        Type::Duration
                    }
                    (Type::Duration, Type::Number)
                        if matches!(op, BinaryOp::Mul | BinaryOp::Div) =>
                    {
                        Type::Duration
                    }
                    (Type::Number, Type::Duration) if matches!(op, BinaryOp::Mul) => Type::Duration,
                    // Column arithmetic operations
                    (Type::Column(a), Type::Column(b)) if a == b => Type::Column(a.clone()),
                    (Type::Column(elem), Type::Number) | (Type::Number, Type::Column(elem))
                        if **elem == Type::Number =>
                    {
                        Type::Column(elem.clone())
                    }
                    // Matrix multiplication
                    (Type::Matrix(elem), Type::Array(vec_elem))
                        if matches!(op, BinaryOp::Mul)
                            && **elem == Type::Number
                            && **vec_elem == Type::Number =>
                    {
                        Type::Array(Box::new(Type::Number))
                    }
                    (Type::Matrix(left_elem), Type::Matrix(right_elem))
                        if matches!(op, BinaryOp::Mul)
                            && **left_elem == Type::Number
                            && **right_elem == Type::Number =>
                    {
                        Type::Matrix(Box::new(Type::Number))
                    }
                    // Allow Unknown types in binary operations (for function parameters)
                    (Type::Unknown, Type::Number) | (Type::Number, Type::Unknown) => Type::Number,
                    (Type::Unknown, Type::Unknown) => Type::Unknown,
                    _ => Type::Error,
                }
            }

            // Comparison operations
            BinaryOp::Greater
            | BinaryOp::Less
            | BinaryOp::GreaterEq
            | BinaryOp::LessEq
            | BinaryOp::Equal
            | BinaryOp::NotEqual => {
                match (self, rhs) {
                    (Type::Number, Type::Number) => Type::Bool,
                    (Type::String, Type::String) => Type::Bool,
                    (Type::Bool, Type::Bool) => Type::Bool,
                    (Type::Color, Type::Color) => Type::Bool,
                    // Timestamp comparisons
                    (Type::Timestamp, Type::Timestamp) => Type::Bool,
                    (Type::Duration, Type::Duration) => Type::Bool,
                    // Column comparison operations return Column<Bool>
                    (Type::Column(a), Type::Column(b)) if a == b => {
                        Type::Column(Box::new(Type::Bool))
                    }
                    (Type::Column(elem), Type::Number) | (Type::Number, Type::Column(elem))
                        if **elem == Type::Number =>
                    {
                        Type::Column(Box::new(Type::Bool))
                    }
                    // Allow Unknown types in comparisons
                    (Type::Unknown, Type::Number) | (Type::Number, Type::Unknown) => Type::Bool,
                    (Type::Unknown, Type::Unknown) => Type::Bool,
                    _ => Type::Error,
                }
            }

            // Fuzzy comparison operations
            BinaryOp::FuzzyEqual | BinaryOp::FuzzyGreater | BinaryOp::FuzzyLess => {
                match (self, rhs) {
                    (Type::Number, Type::Number) => Type::Bool,
                    _ => Type::Error,
                }
            }

            // Bitwise operations
            BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitShl
            | BinaryOp::BitShr => match (self, rhs) {
                (Type::Number, Type::Number) => Type::Number,
                _ => Type::Error,
            },

            // Logical operations
            BinaryOp::And | BinaryOp::Or => match (self, rhs) {
                (Type::Bool, Type::Bool) => Type::Bool,
                _ => Type::Error,
            },

            // Null coalescing
            BinaryOp::NullCoalesce => {
                // The result type is the non-null type between the two
                match (self, rhs) {
                    (Type::Null, right) => right.clone(),
                    (left, _) => left.clone(),
                }
            }

            BinaryOp::ErrorContext => match self {
                Type::Result(inner) => Type::Result(inner.clone()),
                Type::Null => Type::Result(Box::new(Type::Unknown)),
                other => Type::Result(Box::new(other.clone())),
            },

            // Pipe operator - result type depends on the right side (function/method call)
            BinaryOp::Pipe => Type::Unknown,
        }
    }

    /// Get the result type of a unary operation
    pub fn unary_op_result(&self, op: &shape_ast::ast::UnaryOp) -> Type {
        use shape_ast::ast::UnaryOp;

        match op {
            UnaryOp::Not => match self {
                Type::Bool => Type::Bool,
                _ => Type::Error,
            },
            UnaryOp::Neg => match self {
                Type::Number => Type::Number,
                _ => Type::Error,
            },
            UnaryOp::BitNot => match self {
                Type::Number => Type::Number,
                _ => Type::Error,
            },
        }
    }

    /// Get the type of a property access
    pub fn property_type(&self, property: &str) -> Type {
        match self {
            Type::Object(fields) => {
                // Generic schema-based property access
                if fields.is_empty() {
                    // Empty schema means generic object - allow any property
                    Type::Unknown
                } else {
                    fields
                        .iter()
                        .find(|(name, _)| name == property)
                        .map(|(_, ty)| ty.clone())
                        .unwrap_or(Type::Error)
                }
            }
            _ => Type::Error,
        }
    }

    /// Convert legacy semantic Type to inference Type
    ///
    /// This bridges the legacy type checker's type representation to the
    /// modern inference engine's type system.
    pub fn to_inference_type(&self) -> crate::type_system::Type {
        use crate::type_system::{BuiltinTypes, Type as InferenceType};
        use shape_ast::ast::TypeAnnotation;

        match self {
            Type::Number => BuiltinTypes::number(),
            Type::String => BuiltinTypes::string(),
            Type::Bool => BuiltinTypes::boolean(),
            Type::Null => BuiltinTypes::null(),
            Type::Unit => BuiltinTypes::void(),
            Type::Color => InferenceType::Concrete(TypeAnnotation::Basic("color".to_string())),
            Type::Timestamp => {
                InferenceType::Concrete(TypeAnnotation::Basic("timestamp".to_string()))
            }
            Type::Timeframe => {
                InferenceType::Concrete(TypeAnnotation::Basic("timeframe".to_string()))
            }
            Type::TimeRef => InferenceType::Concrete(TypeAnnotation::Basic("timeref".to_string())),
            Type::Duration => {
                InferenceType::Concrete(TypeAnnotation::Basic("duration".to_string()))
            }
            Type::Pattern => BuiltinTypes::pattern(),
            Type::Module => InferenceType::Concrete(TypeAnnotation::Basic("module".to_string())),
            Type::Unknown => InferenceType::Variable(super::super::type_system::TypeVar::fresh()),
            Type::Error => InferenceType::Concrete(TypeAnnotation::Never),

            Type::Array(elem) => BuiltinTypes::array(elem.to_inference_type()),
            Type::Matrix(elem) => InferenceType::Generic {
                base: Box::new(InferenceType::Concrete(TypeAnnotation::Reference(
                    "Mat".to_string(),
                ))),
                args: vec![elem.to_inference_type()],
            },

            Type::Column(elem) => InferenceType::Generic {
                base: Box::new(InferenceType::Concrete(TypeAnnotation::Reference(
                    "Column".to_string(),
                ))),
                args: vec![elem.to_inference_type()],
            },

            Type::Range(elem) => InferenceType::Generic {
                base: Box::new(InferenceType::Concrete(TypeAnnotation::Reference(
                    "Range".to_string(),
                ))),
                args: vec![elem.to_inference_type()],
            },

            Type::Result(ok_type) => InferenceType::Generic {
                base: Box::new(InferenceType::Concrete(TypeAnnotation::Reference(
                    "Result".to_string(),
                ))),
                args: vec![ok_type.to_inference_type()],
            },

            Type::Object(fields) => {
                let obj_fields: Vec<_> = fields
                    .iter()
                    .map(|(name, ty)| shape_ast::ast::ObjectTypeField {
                        name: name.clone(),
                        optional: false,
                        type_annotation: ty.to_type_annotation(),
                        annotations: vec![],
                    })
                    .collect();
                InferenceType::Concrete(TypeAnnotation::Object(obj_fields))
            }

            Type::Function { params, returns } => {
                let param_annotations: Vec<_> = params
                    .iter()
                    .map(|p| shape_ast::ast::FunctionParam {
                        name: None,
                        optional: false,
                        type_annotation: p.to_type_annotation(),
                    })
                    .collect();
                InferenceType::Concrete(TypeAnnotation::Function {
                    params: param_annotations,
                    returns: Box::new(returns.to_type_annotation()),
                })
            }
        }
    }

    /// Convert inference Type to legacy semantic Type
    ///
    /// This bridges the modern inference engine's type system back to the
    /// legacy type checker's type representation.
    pub fn from_inference_type(ty: &crate::type_system::Type) -> Self {
        use crate::type_system::Type as InferenceType;
        use shape_ast::ast::TypeAnnotation;

        match ty {
            InferenceType::Concrete(ann) => Self::from_type_annotation(ann),

            InferenceType::Variable(_) => Type::Unknown,

            InferenceType::Generic { base, args } => {
                // Handle known generic types
                if let InferenceType::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() {
                    match name.as_str() {
                        "Vec" if !args.is_empty() => {
                            Type::Array(Box::new(Self::from_inference_type(&args[0])))
                        }
                        "Mat" if !args.is_empty() => {
                            Type::Matrix(Box::new(Self::from_inference_type(&args[0])))
                        }
                        "Column" if !args.is_empty() => {
                            Type::Column(Box::new(Self::from_inference_type(&args[0])))
                        }
                        "Range" if !args.is_empty() => {
                            Type::Range(Box::new(Self::from_inference_type(&args[0])))
                        }
                        "Result" if !args.is_empty() => {
                            Type::Result(Box::new(Self::from_inference_type(&args[0])))
                        }
                        _ => Type::Unknown,
                    }
                } else {
                    Type::Unknown
                }
            }

            InferenceType::Constrained { .. } => Type::Unknown,
            InferenceType::Function { params, returns } => {
                let param_types: Vec<_> = params.iter().map(Self::from_inference_type).collect();
                let return_type = Self::from_inference_type(returns);
                Type::Function {
                    params: param_types,
                    returns: Box::new(return_type),
                }
            }
        }
    }

    /// Convert to TypeAnnotation (helper for to_inference_type)
    fn to_type_annotation(&self) -> shape_ast::ast::TypeAnnotation {
        use shape_ast::ast::TypeAnnotation;

        match self {
            Type::Number => TypeAnnotation::Basic("number".to_string()),
            Type::String => TypeAnnotation::Basic("string".to_string()),
            Type::Bool => TypeAnnotation::Basic("bool".to_string()),
            Type::Null => TypeAnnotation::Null,
            Type::Unit => TypeAnnotation::Void,
            Type::Color => TypeAnnotation::Basic("color".to_string()),
            Type::Timestamp => TypeAnnotation::Basic("timestamp".to_string()),
            Type::Timeframe => TypeAnnotation::Basic("timeframe".to_string()),
            Type::TimeRef => TypeAnnotation::Basic("timeref".to_string()),
            Type::Duration => TypeAnnotation::Basic("duration".to_string()),
            Type::Pattern => TypeAnnotation::Basic("pattern".to_string()),
            Type::Module => TypeAnnotation::Basic("module".to_string()),
            Type::Unknown => TypeAnnotation::Basic("unknown".to_string()),
            Type::Error => TypeAnnotation::Never,

            Type::Array(elem) => TypeAnnotation::Array(Box::new(elem.to_type_annotation())),
            Type::Matrix(elem) => TypeAnnotation::Generic {
                name: "Mat".to_string(),
                args: vec![elem.to_type_annotation()],
            },

            Type::Column(elem) => TypeAnnotation::Generic {
                name: "Column".to_string(),
                args: vec![elem.to_type_annotation()],
            },

            Type::Range(elem) => TypeAnnotation::Generic {
                name: "Range".to_string(),
                args: vec![elem.to_type_annotation()],
            },

            Type::Result(ok_type) => TypeAnnotation::Generic {
                name: "Result".to_string(),
                args: vec![ok_type.to_type_annotation()],
            },

            Type::Object(fields) => {
                let obj_fields: Vec<_> = fields
                    .iter()
                    .map(|(name, ty)| shape_ast::ast::ObjectTypeField {
                        name: name.clone(),
                        optional: false,
                        type_annotation: ty.to_type_annotation(),
                        annotations: vec![],
                    })
                    .collect();
                TypeAnnotation::Object(obj_fields)
            }

            Type::Function { params, returns } => {
                let param_annotations: Vec<_> = params
                    .iter()
                    .map(|p| shape_ast::ast::FunctionParam {
                        name: None,
                        optional: false,
                        type_annotation: p.to_type_annotation(),
                    })
                    .collect();
                TypeAnnotation::Function {
                    params: param_annotations,
                    returns: Box::new(returns.to_type_annotation()),
                }
            }
        }
    }

    /// Convert from TypeAnnotation (helper for from_inference_type)
    fn from_type_annotation(ann: &shape_ast::ast::TypeAnnotation) -> Self {
        use shape_ast::ast::TypeAnnotation;

        match ann {
            TypeAnnotation::Basic(name) => match name.as_str() {
                "number" | "Number" | "f64" | "float" => Type::Number,
                "string" | "String" => Type::String,
                "bool" | "boolean" | "Boolean" => Type::Bool,
                "null" | "Null" => Type::Null,
                "color" | "Color" => Type::Color,
                "timestamp" | "Timestamp" => Type::Timestamp,
                "timeframe" | "Timeframe" => Type::Timeframe,
                "timeref" | "TimeRef" => Type::TimeRef,
                "duration" | "Duration" => Type::Duration,
                "pattern" | "Pattern" => Type::Pattern,
                "module" | "Module" => Type::Module,
                "object" => Type::Object(vec![]),
                _ => Type::Unknown,
            },

            TypeAnnotation::Array(elem) => Type::Array(Box::new(Self::from_type_annotation(elem))),

            TypeAnnotation::Generic { name, args } => match name.as_str() {
                "Column" if !args.is_empty() => {
                    Type::Column(Box::new(Self::from_type_annotation(&args[0])))
                }
                "Vec" if !args.is_empty() => {
                    Type::Array(Box::new(Self::from_type_annotation(&args[0])))
                }
                "Mat" if !args.is_empty() => {
                    Type::Matrix(Box::new(Self::from_type_annotation(&args[0])))
                }
                "Range" if !args.is_empty() => {
                    Type::Range(Box::new(Self::from_type_annotation(&args[0])))
                }
                "Result" if !args.is_empty() => {
                    Type::Result(Box::new(Self::from_type_annotation(&args[0])))
                }
                _ => Type::Unknown,
            },

            TypeAnnotation::Reference(name) => match name.as_str() {
                "number" | "Number" => Type::Number,
                "string" | "String" => Type::String,
                "bool" | "Bool" => Type::Bool,
                _ => Type::Unknown,
            },

            TypeAnnotation::Object(fields) => {
                let type_fields: Vec<_> = fields
                    .iter()
                    .map(|f| {
                        (
                            f.name.clone(),
                            Self::from_type_annotation(&f.type_annotation),
                        )
                    })
                    .collect();
                Type::Object(type_fields)
            }

            TypeAnnotation::Function { params, returns } => {
                let param_types: Vec<_> = params
                    .iter()
                    .map(|p| Self::from_type_annotation(&p.type_annotation))
                    .collect();
                Type::Function {
                    params: param_types,
                    returns: Box::new(Self::from_type_annotation(returns)),
                }
            }

            TypeAnnotation::Void => Type::Unit,
            TypeAnnotation::Never => Type::Error,
            TypeAnnotation::Null | TypeAnnotation::Undefined => Type::Null,

            TypeAnnotation::Tuple(_)
            | TypeAnnotation::Union(_)
            | TypeAnnotation::Intersection(_)
            | TypeAnnotation::Dyn(_) => Type::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_inference_type_primitives() {
        use crate::type_system::BuiltinTypes;

        assert_eq!(Type::Number.to_inference_type(), BuiltinTypes::number());
        assert_eq!(Type::String.to_inference_type(), BuiltinTypes::string());
        assert_eq!(Type::Bool.to_inference_type(), BuiltinTypes::boolean());
    }

    #[test]
    fn test_to_inference_type_array() {
        let arr = Type::Array(Box::new(Type::Number));
        let inference_type = arr.to_inference_type();

        // Convert back and check
        let back = Type::from_inference_type(&inference_type);
        assert_eq!(back, Type::Array(Box::new(Type::Number)));
    }

    #[test]
    fn test_to_inference_type_matrix() {
        let mat = Type::Matrix(Box::new(Type::Number));
        let inference_type = mat.to_inference_type();

        // Convert back and check
        let back = Type::from_inference_type(&inference_type);
        assert_eq!(back, Type::Matrix(Box::new(Type::Number)));
    }

    #[test]
    fn test_to_inference_type_column() {
        let column = Type::Column(Box::new(Type::Number));
        let inference_type = column.to_inference_type();

        // Convert back and check
        let back = Type::from_inference_type(&inference_type);
        assert_eq!(back, Type::Column(Box::new(Type::Number)));
    }

    #[test]
    fn test_binary_op_result_matrix_mul() {
        use shape_ast::ast::BinaryOp;

        let mat = Type::Matrix(Box::new(Type::Number));
        let vec = Type::Array(Box::new(Type::Number));
        let mat_rhs = Type::Matrix(Box::new(Type::Number));

        assert_eq!(
            mat.binary_op_result(&BinaryOp::Mul, &vec),
            Type::Array(Box::new(Type::Number))
        );
        assert_eq!(
            mat.binary_op_result(&BinaryOp::Mul, &mat_rhs),
            Type::Matrix(Box::new(Type::Number))
        );
    }

    #[test]
    fn test_from_inference_type_primitives() {
        use crate::type_system::BuiltinTypes;

        assert_eq!(
            Type::from_inference_type(&BuiltinTypes::number()),
            Type::Number
        );
        assert_eq!(
            Type::from_inference_type(&BuiltinTypes::string()),
            Type::String
        );
        assert_eq!(
            Type::from_inference_type(&BuiltinTypes::boolean()),
            Type::Bool
        );
    }

    #[test]
    fn test_roundtrip_object_type() {
        let obj = Type::Object(vec![
            ("x".to_string(), Type::Number),
            ("y".to_string(), Type::String),
        ]);
        let inference = obj.to_inference_type();
        let back = Type::from_inference_type(&inference);

        assert_eq!(back, obj);
    }

    #[test]
    fn test_roundtrip_function_type() {
        let func = Type::Function {
            params: vec![Type::Number, Type::String],
            returns: Box::new(Type::Bool),
        };
        let inference = func.to_inference_type();
        let back = Type::from_inference_type(&inference);

        assert_eq!(back, func);
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Type::Number => write!(f, "Number"),
            Type::String => write!(f, "String"),
            Type::Bool => write!(f, "Bool"),
            Type::Null => write!(f, "Null"),
            Type::Unit => write!(f, "Unit"),
            Type::Color => write!(f, "Color"),
            Type::Timestamp => write!(f, "Timestamp"),
            Type::Timeframe => write!(f, "Timeframe"),
            Type::TimeRef => write!(f, "TimeRef"),
            Type::Duration => write!(f, "Duration"),
            Type::Pattern => write!(f, "Pattern"),
            Type::Object(fields) => {
                write!(f, "{{")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, ty)?;
                }
                write!(f, "}}")
            }
            Type::Array(elem) => write!(f, "Vec<{}>", elem),
            Type::Matrix(elem) => write!(f, "Mat<{}>", elem),
            Type::Column(elem) => write!(f, "Column<{}>", elem),
            Type::Function { params, returns } => {
                write!(f, "(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", param)?;
                }
                write!(f, ") -> {}", returns)
            }
            Type::Module => write!(f, "Module"),
            Type::Range(elem) => write!(f, "Range<{}>", elem),
            Type::Result(ok_type) => write!(f, "Result<{}>", ok_type),
            Type::Unknown => write!(f, "?"),
            Type::Error => write!(f, "Error"),
        }
    }
}
