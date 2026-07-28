//! Type System Errors
//!
//! Defines error types for type inference and type checking,
//! with detailed error messages for better developer experience.

use super::effects::{ClosedEffectRow, EffectRow};
use super::{Type, TypeVar};
use shape_ast::ast::{Span, TypeAnnotation};

pub type TypeResult<T> = Result<T, TypeError>;

/// Where a boundary's declared effect row is written in source, so a
/// materialization fix (ADR-017 §4) knows what to edit.
///
/// Two shapes, because ADR-014 §8.2 draws exactly two cases: the boundary
/// wrote a row that turned out too narrow (replace the clause), or it wrote
/// none at all (insert one). The insertion point is recorded either way, so a
/// site is usable whichever case the checking moment turns out to be.
///
/// This is provenance the *checking site* supplies — the row algebra in
/// [`super::effects`] and `ConstraintSolver::check_declared_boundary` stay
/// span-free, and a boundary check that has no AST in hand simply attaches no
/// site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredRowSite {
    /// Span of the written `! {...}` clause, including the `!`. `None` when
    /// the boundary declared no row.
    pub clause: Option<Span>,
    /// Byte offset at which a clause belongs when none is written — after the
    /// return type of the signature that owns the boundary.
    pub insert_at: usize,
}

impl DeclaredRowSite {
    /// A boundary that wrote a row: the fix replaces `clause`.
    pub fn written(clause: Span) -> Self {
        DeclaredRowSite {
            insert_at: clause.start,
            clause: Some(clause),
        }
    }

    /// A boundary that wrote none: the fix inserts at `insert_at`.
    pub fn omitted(insert_at: usize) -> Self {
        DeclaredRowSite {
            clause: None,
            insert_at,
        }
    }
}

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
    #[error("Cannot construct infinite type for '{}'", .0.presentation_name())]
    InfiniteType(TypeVar),

    /// Unsolved type constraints
    #[error("{}", format_unsolved_constraints(.0))]
    UnsolvedConstraints(Vec<(Type, Type)>),

    /// Type constraint violation
    #[error("Type constraint violation: {0}")]
    ConstraintViolation(String),

    /// ADR-014 §8.1: a callable's effect row exceeds the row its boundary
    /// declares. `excess` is the sorted list of atoms the boundary does not
    /// permit — the payload #180's materialization fix consumes.
    ///
    /// `inferred` and `declared` are the rows themselves, not renderings of
    /// them. The materialization fix (ADR-017 §4) writes source from
    /// `inferred`, and its test asserts against this value; a fix built from a
    /// rendered string would be a second authority on what the row is. The
    /// message renders on demand.
    #[error(
        "effect row {} exceeds the declared row {}; \
         the boundary does not permit {}",
        .inferred.render(),
        .declared.render(),
        .excess.join(", ")
    )]
    EffectRowExceedsBoundary {
        inferred: ClosedEffectRow,
        declared: ClosedEffectRow,
        excess: Vec<String>,
        /// Where the rejected boundary writes (or omits) its row. `None` when
        /// the checking site did not record one — the diagnostic still reports,
        /// it just carries no machine-applicable edit.
        site: Option<DeclaredRowSite>,
    },

    /// ADR-010 §13: an effect parameter reached a checking, freezing, or
    /// persistence point without substituting to a closed row.
    #[error(
        "effect parameter `{parameter}` is unbound at {site}; it must \
         substitute to a closed row before checking, freezing, or persistence"
    )]
    UnboundEffectParameter { parameter: String, site: String },

    /// Two rows could not be compared: different stage or catalog version.
    #[error("effect rows are not comparable: {reason}")]
    IncomparableEffectRows { reason: String },

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
    #[error(
        "trait bound not satisfied: Type '{type_name}' does not implement trait '{trait_name}'"
    )]
    TraitBoundViolation {
        type_name: String,
        trait_name: String,
    },

    /// J-CT.1: a `comptime trait` method was called outside a comptime context.
    /// Comptime-trait methods are compile-time-only — runtime call sites must
    /// be rejected by the type-checker before bytecode emission.
    #[error(
        "Cannot call comptime-trait method '{method_name}' on type '{type_name}' \
         outside a `comptime {{ ... }}` block — the method is compile-time-only"
    )]
    ComptimeMethodCallOutsideComptime {
        type_name: String,
        method_name: String,
    },

    /// J-CT.1: `comptime trait` / `comptime impl` alignment mismatch.
    /// A non-comptime `impl` cannot implement a `comptime trait`, and a
    /// `comptime impl` cannot implement a non-comptime trait.
    #[error(
        "comptime alignment mismatch: trait '{trait_name}' is_comptime={trait_is_comptime}, \
         impl for '{type_name}' is_comptime={impl_is_comptime} — both must agree"
    )]
    ComptimeImplTraitMismatch {
        trait_name: String,
        type_name: String,
        trait_is_comptime: bool,
        impl_is_comptime: bool,
    },
}

impl TypeError {
    /// Record where the rejected boundary writes its row.
    ///
    /// The row algebra decides *whether* a boundary is violated; only the
    /// checking site knows *where* the violated boundary is written. Splitting
    /// it this way is what lets `ConstraintSolver::check_declared_boundary`
    /// stay span-free while the fix funnel still receives an editable site.
    ///
    /// A no-op on any other variant, so a checking site may attach a site
    /// unconditionally.
    pub fn with_declared_row_site(mut self, at: DeclaredRowSite) -> Self {
        if let TypeError::EffectRowExceedsBoundary { site, .. } = &mut self {
            *site = Some(at);
        }
        self
    }
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
        Type::Function {
            params,
            returns,
            effects,
        } => {
            let rendered_params = params
                .iter()
                .map(format_type)
                .collect::<Vec<_>>()
                .join(", ");
            // An underived row renders as the bare arrow type: showing
            // `! <underived>` in a user-facing message would advertise an
            // internal proof gap as if it were a declared row.
            let rendered_row = match effects {
                EffectRow::Unproven => String::new(),
                row => format!(" ! {}", row.render()),
            };
            format!(
                "({}) -> {}{}",
                rendered_params,
                format_type(returns),
                rendered_row
            )
        }
        _ => ty
            .to_annotation()
            .map(|ann| format_annotation(&ann))
            .unwrap_or_else(|| format!("{:?}", ty)),
    }
}

fn format_annotation(ann: &TypeAnnotation) -> String {
    match ann {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(name) => name.to_string(),
        TypeAnnotation::Borrow { mutable, inner } => {
            if *mutable {
                format!("&mut {}", format_annotation(inner))
            } else {
                format!("&{}", format_annotation(inner))
            }
        }
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
        TypeAnnotation::Function {
            params, returns, ..
        } => format!(
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
        TypeAnnotation::Generic { name, args } => format!(
            "{}<{}>",
            name,
            args.iter()
                .map(format_annotation)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeAnnotation::Void => "void".to_string(),
        TypeAnnotation::Never => "never".to_string(),
        TypeAnnotation::Null => "None".to_string(),
        TypeAnnotation::Undefined => "undefined".to_string(),
        TypeAnnotation::Dyn(traits) => format!("dyn {}", traits.join(" + ")),
        // ADR-009 B3 (S1): existential descriptor package type.
        TypeAnnotation::Existential { witnesses, inner } => {
            format!(
                "exists<{}> {}",
                witnesses.join(", "),
                format_annotation(inner)
            )
        }
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
    /// Machine-applicable fixes proved alongside the error (ADR-017 §4).
    /// Empty when the checker proved none.
    pub fixes: Vec<shape_diagnostics::SuggestedFix>,
}

impl TypeErrorWithLocation {
    pub fn new(error: TypeError, line: usize, column: usize) -> Self {
        TypeErrorWithLocation {
            error,
            file: None,
            line,
            column,
            source_line: None,
            fixes: Vec::new(),
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

    /// Attach machine-applicable fixes (ADR-017 §4).
    pub fn with_fixes(mut self, fixes: Vec<shape_diagnostics::SuggestedFix>) -> Self {
        self.fixes = fixes;
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

#[cfg(test)]
mod tests {
    use super::TypeError;
    use crate::type_system::{TypeVar, TypeVarGen};

    #[test]
    fn infinite_type_uses_the_declared_parameter_presentation() {
        let mut variables = TypeVarGen::new();
        let parameter = TypeVar::declared(variables.fresh_declared_owner(), 0, "Element");

        let rendered = TypeError::InfiniteType(parameter).to_string();

        assert_eq!(rendered, "Cannot construct infinite type for 'Element'");
        assert!(!rendered.contains('\u{1}'));
    }
}
