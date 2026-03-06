//! Type System Errors
//!
//! Defines error types for type inference and type checking,
//! with detailed error messages for better developer experience.

use super::{Type, TypeVar};
use shape_ast::ast::TypeAnnotation;

pub type TypeResult<T> = Result<T, TypeError>;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TypeError {
    /// Type mismatch between expected and actual types
    #[error("Type mismatch: expected '{0}', found '{1}'")]
    TypeMismatch(String, String),

    /// Undefined variable
    #[error("Undefined variable: '{0}'")]
    UndefinedVariable(String),

    /// Undefined function
    #[error("Undefined function: '{0}'")]
    UndefinedFunction(String),

    /// Undefined type
    #[error("Undefined type: '{0}'")]
    UndefinedType(String),

    /// Unknown property on a type
    #[error("Property '{1}' does not exist on type '{0}'")]
    UnknownProperty(String, String),

    /// Arity mismatch in function call or generic type
    #[error("Wrong number of arguments: expected {0}, found {1}")]
    ArityMismatch(usize, usize),

    /// Infinite type (occurs check failure)
    #[error("Cannot construct infinite type for '{}'", .0.0)]
    InfiniteType(TypeVar),

    /// Unsolved type constraints
    #[error("{}", format_unsolved_constraints(.0))]
    UnsolvedConstraints(Vec<(Type, Type)>),

    /// Type constraint violation
    #[error("Type constraint violation: {0}")]
    ConstraintViolation(String),

    /// Const variable without explicit type
    #[error("Const variable '{0}' must have an explicit type annotation or initializer")]
    ConstWithoutType(String),

    /// Invalid type assertion
    #[error("Cannot assert type '{0}' as '{1}'")]
    InvalidAssertion(String, String),

    /// Cyclic type alias
    #[error("Cyclic type alias detected: '{0}'")]
    CyclicTypeAlias(String),

    /// Invalid return type
    #[error("Function return type mismatch: expected '{0}', found '{1}'")]
    InvalidReturnType(String, String),

    /// Missing return statement
    #[error("Function '{0}' must return a value")]
    MissingReturn(String),

    /// Invalid pattern type
    #[error("Invalid pattern type: {0}")]
    InvalidPatternType(String),

    /// Generic type parameter error
    #[error("Generic type error: {message}")]
    GenericTypeError {
        message: String,
        /// Symbol name associated with the error (if known), e.g. function name.
        symbol: Option<String>,
    },

    /// Interface implementation error
    #[error("Interface '{0}' error: {1}")]
    InterfaceError(String, String),

    /// Union type error
    #[error("Union type error: {0}")]
    UnionTypeError(String),

    /// Type annotation parse error
    #[error("Type annotation parse error: {0}")]
    AnnotationParseError(String),

    /// Non-exhaustive match expression
    #[error("Non-exhaustive match on '{enum_name}': missing variants {}", missing_variants.join(", "))]
    NonExhaustiveMatch {
        enum_name: String,
        missing_variants: Vec<String>,
    },

    /// Type mutation error (cannot change variable's fundamental type)
    #[error("Cannot change type of '{variable}' from '{original_type}' to '{attempted_type}'")]
    TypeMutation {
        variable: String,
        original_type: String,
        attempted_type: String,
    },

    /// Trait impl arity mismatch: impl method has different parameter count than trait method
    #[error(
        "impl {trait_name} method '{method_name}' has {got} parameters, but trait requires {expected}"
    )]
    TraitImplArityMismatch {
        trait_name: String,
        method_name: String,
        expected: usize,
        got: usize,
    },

    /// Trait impl validation error (e.g., missing required method)
    #[error("Trait impl error: {0}")]
    TraitImplValidation(String),

    /// Method not found on type
    #[error("Method '{method_name}' not found on type '{type_name}'")]
    MethodNotFound {
        type_name: String,
        method_name: String,
    },

    /// Trait bound violation: type does not implement required trait
    #[error("Type '{type_name}' does not implement trait '{trait_name}'")]
    TraitBoundViolation {
        type_name: String,
        trait_name: String,
    },
}

fn format_unsolved_constraints(constraints: &[(Type, Type)]) -> String {
    if constraints.is_empty() {
        "Could not solve type constraints".to_string()
    } else {
        let rendered = constraints
            .iter()
            .map(|(left, right)| {
                format!(
                    "  {} is not compatible with {}",
                    format_type(left),
                    format_type(right)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("Could not solve type constraints:\n{}", rendered)
    }
}

fn format_type(ty: &Type) -> String {
    match ty {
        Type::Variable(_) => "unknown".to_string(),
        Type::Constrained { .. } => "constrained".to_string(),
        Type::Function { params, returns } => {
            let rendered_params = params
                .iter()
                .map(format_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({}) -> {}", rendered_params, format_type(returns))
        }
        _ => ty
            .to_annotation()
            .map(|ann| format_annotation(&ann))
            .unwrap_or_else(|| format!("{:?}", ty)),
    }
}

fn format_annotation(ann: &TypeAnnotation) -> String {
    match ann {
        TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => name.clone(),
        TypeAnnotation::Array(inner) => format!("Vec<{}>", format_annotation(inner)),
        TypeAnnotation::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(format_annotation)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeAnnotation::Object(fields) => {
            let rendered = fields
                .iter()
                .map(|field| {
                    let optional = if field.optional { "?" } else { "" };
                    format!(
                        "{}{}: {}",
                        field.name,
                        optional,
                        format_annotation(&field.type_annotation)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{ {} }}", rendered)
        }
        TypeAnnotation::Function { params, returns } => format!(
            "({}) -> {}",
            params
                .iter()
                .map(|param| format_annotation(&param.type_annotation))
                .collect::<Vec<_>>()
                .join(", "),
            format_annotation(returns)
        ),
        TypeAnnotation::Union(types) => types
            .iter()
            .map(format_annotation)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeAnnotation::Intersection(types) => types
            .iter()
            .map(format_annotation)
            .collect::<Vec<_>>()
            .join(" + "),
        TypeAnnotation::Optional(inner) => format!("{}?", format_annotation(inner)),
        TypeAnnotation::Generic { name, args } => format!(
            "{}<{}>",
            name,
            args.iter()
                .map(format_annotation)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeAnnotation::Void => "void".to_string(),
        TypeAnnotation::Any => "any".to_string(),
        TypeAnnotation::Never => "never".to_string(),
        TypeAnnotation::Null => "None".to_string(),
        TypeAnnotation::Undefined => "undefined".to_string(),
        TypeAnnotation::Dyn(traits) => format!("dyn {}", traits.join(" + ")),
    }
}

/// Type error with source location information
#[derive(Debug, Clone)]
pub struct TypeErrorWithLocation {
    pub error: TypeError,
    pub file: Option<String>,
    pub line: usize,
    pub column: usize,
    pub source_line: Option<String>,
}

impl TypeErrorWithLocation {
    pub fn new(error: TypeError, line: usize, column: usize) -> Self {
        TypeErrorWithLocation {
            error,
            file: None,
            line,
            column,
            source_line: None,
        }
    }

    pub fn with_file(mut self, file: String) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_source_line(mut self, source: String) -> Self {
        self.source_line = Some(source);
        self
    }

    /// Format the error with source location
    pub fn format_with_source(&self) -> String {
        let mut output = String::new();

        // Error location
        if let Some(file) = &self.file {
            output.push_str(&format!("{}:{}:{}: ", file, self.line, self.column));
        } else {
            output.push_str(&format!("{}:{}: ", self.line, self.column));
        }

        // Error message
        output.push_str(&format!("error: {}\n", self.error));

        // Source line with caret
        if let Some(source) = &self.source_line {
            output.push_str(&format!("  {}\n", source));
            output.push_str(&format!(
                "  {}^\n",
                " ".repeat(self.column.saturating_sub(1))
            ));
        }

        output
    }
}

/// Helper for creating common type errors with better messages
pub struct TypeErrorBuilder;

impl TypeErrorBuilder {
    pub fn numeric_expected(actual: &str) -> TypeError {
        TypeError::TypeMismatch("number".to_string(), actual.to_string())
    }

    pub fn boolean_expected(actual: &str) -> TypeError {
        TypeError::TypeMismatch("boolean".to_string(), actual.to_string())
    }

    pub fn string_expected(actual: &str) -> TypeError {
        TypeError::TypeMismatch("string".to_string(), actual.to_string())
    }

    pub fn array_expected(actual: &str) -> TypeError {
        TypeError::TypeMismatch("array".to_string(), actual.to_string())
    }

    pub fn function_expected(actual: &str) -> TypeError {
        TypeError::TypeMismatch("function".to_string(), actual.to_string())
    }

    pub fn pattern_expected(actual: &str) -> TypeError {
        TypeError::TypeMismatch("pattern".to_string(), actual.to_string())
    }

    pub fn row_expected(actual: &str) -> TypeError {
        TypeError::TypeMismatch("row".to_string(), actual.to_string())
    }
}
