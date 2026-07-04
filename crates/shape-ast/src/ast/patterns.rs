//! Pattern matching and destructuring types for Shape AST

use serde::{Deserialize, Serialize};

use super::literals::Literal;
use super::span::Span;
use super::types::TypeAnnotation;

/// Pattern for pattern matching
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Pattern {
    /// Match a specific identifier and bind it
    Identifier { name: String, span: Span },
    /// Match by type and bind identifier when the runtime value conforms
    Typed {
        name: String,
        name_span: Span,
        type_annotation: TypeAnnotation,
    },
    /// Match a literal value
    Literal(Literal),
    /// Match an array pattern
    Array(Vec<Pattern>),
    /// Match an object pattern
    Object(Vec<(String, Pattern)>),
    /// Match anything (underscore)
    Wildcard,
    /// Match a constructor pattern
    Constructor {
        enum_name: Option<super::type_path::TypePath>,
        variant: String,
        fields: PatternConstructorFields,
    },
}

/// Fields bound in a constructor pattern
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternConstructorFields {
    Unit,
    Tuple(Vec<Pattern>),
    Struct(Vec<(String, Pattern)>),
}

impl std::fmt::Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pattern::Identifier { name, .. } => write!(f, "{}", name),
            Pattern::Typed {
                name,
                type_annotation,
                ..
            } => write!(f, "{}: {:?}", name, type_annotation),
            Pattern::Wildcard => write!(f, "_"),
            Pattern::Literal(lit) => write!(f, "{}", lit),
            Pattern::Array(pats) => {
                write!(f, "[")?;
                for (i, p) in pats.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, "]")
            }
            Pattern::Object(fields) => {
                write!(f, "{{ ")?;
                for (i, (key, pat)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", key, pat)?;
                }
                write!(f, " }}")
            }
            Pattern::Constructor {
                enum_name,
                variant,
                fields,
            } => {
                if let Some(e) = enum_name {
                    write!(f, "{}::{}", e, variant)?;
                } else {
                    write!(f, "{}", variant)?;
                }
                match fields {
                    PatternConstructorFields::Unit => Ok(()),
                    PatternConstructorFields::Tuple(pats) => {
                        write!(f, "(")?;
                        for (i, p) in pats.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{}", p)?;
                        }
                        write!(f, ")")
                    }
                    PatternConstructorFields::Struct(fields) => {
                        write!(f, " {{ ")?;
                        for (i, (key, pat)) in fields.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{}: {}", key, pat)?;
                        }
                        write!(f, " }}")
                    }
                }
            }
        }
    }
}

impl Pattern {
    /// Create an identifier pattern with a source span for the binder.
    pub fn identifier(name: impl Into<String>, span: Span) -> Self {
        Pattern::Identifier {
            name: name.into(),
            span,
        }
    }

    /// Create an identifier pattern for synthesized AST nodes.
    pub fn synthetic_identifier(name: impl Into<String>) -> Self {
        Pattern::identifier(name, Span::DUMMY)
    }

    /// Get the simple identifier name if this is a simple pattern
    pub fn as_simple_name(&self) -> Option<&str> {
        match self {
            Pattern::Identifier { name, .. } => Some(name),
            Pattern::Typed { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Get the source span for the binder name in simple binding patterns.
    pub fn binder_span(&self) -> Option<Span> {
        match self {
            Pattern::Identifier { span, .. } => Some(*span),
            Pattern::Typed { name_span, .. } => Some(*name_span),
            _ => None,
        }
    }

    /// Get all binding names and binder spans recursively from this pattern.
    pub fn get_bindings(&self) -> Vec<(String, Span)> {
        match self {
            Pattern::Identifier { name, span } => vec![(name.clone(), *span)],
            Pattern::Typed {
                name, name_span, ..
            } => vec![(name.clone(), *name_span)],
            Pattern::Array(patterns) => patterns.iter().flat_map(Pattern::get_bindings).collect(),
            Pattern::Object(fields) => fields
                .iter()
                .flat_map(|(_, pattern)| pattern.get_bindings())
                .collect(),
            Pattern::Constructor { fields, .. } => match fields {
                PatternConstructorFields::Unit => Vec::new(),
                PatternConstructorFields::Tuple(patterns) => {
                    patterns.iter().flat_map(Pattern::get_bindings).collect()
                }
                PatternConstructorFields::Struct(fields) => fields
                    .iter()
                    .flat_map(|(_, pattern)| pattern.get_bindings())
                    .collect(),
            },
            Pattern::Wildcard | Pattern::Literal(_) => Vec::new(),
        }
    }
}

/// Pattern for destructuring assignments
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DestructurePattern {
    /// Simple identifier pattern: let x = ...
    Identifier(String, Span),

    /// Array destructuring: let [a, b, c] = ...
    Array(Vec<DestructurePattern>),

    /// Object destructuring: let {x, y} = ...
    Object(Vec<ObjectPatternField>),

    /// Rest pattern: let [a, ...rest] = ... or let {x, ...rest} = ...
    Rest(Box<DestructurePattern>),

    /// Decomposition pattern: let a: A, b: B = intersection_value
    /// Extracts component types from an intersection type (A + B).
    /// Each binding specifies a name and type annotation for the component.
    Decomposition(Vec<DecompositionBinding>),
}

/// A single binding in a decomposition pattern
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecompositionBinding {
    /// The variable name to bind
    pub name: String,
    /// The type annotation (component type to extract)
    pub type_annotation: TypeAnnotation,
    /// Source span for error reporting
    pub span: Span,
}

/// Field in object destructuring pattern
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectPatternField {
    pub key: String,
    pub pattern: DestructurePattern, // For {x: y} where y is the local name
}

impl DestructurePattern {
    /// Get the identifier name if this is a simple identifier pattern
    pub fn as_identifier(&self) -> Option<&str> {
        match self {
            DestructurePattern::Identifier(name, _) => Some(name),
            _ => None,
        }
    }

    /// Get the identifier span if this is a simple identifier pattern
    pub fn as_identifier_span(&self) -> Option<Span> {
        match self {
            DestructurePattern::Identifier(_, span) => Some(*span),
            _ => None,
        }
    }

    /// Get all identifier names in this pattern
    pub fn get_identifiers(&self) -> Vec<String> {
        self.get_bindings()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    /// Get all identifier names and their source spans in this pattern.
    ///
    /// This is the canonical way to extract bound variables from any pattern
    /// shape. Both the compiler and LSP should use this method to avoid
    /// divergence in how pattern bindings are discovered.
    pub fn get_bindings(&self) -> Vec<(String, Span)> {
        match self {
            DestructurePattern::Identifier(name, span) => vec![(name.clone(), *span)],
            DestructurePattern::Array(patterns) => {
                patterns.iter().flat_map(|p| p.get_bindings()).collect()
            }
            DestructurePattern::Object(fields) => fields
                .iter()
                .flat_map(|f| f.pattern.get_bindings())
                .collect(),
            DestructurePattern::Rest(pattern) => pattern.get_bindings(),
            DestructurePattern::Decomposition(bindings) => {
                bindings.iter().map(|b| (b.name.clone(), b.span)).collect()
            }
        }
    }
}

// REMOVED: PatternDef and related types for pattern block syntax
// Use annotated functions instead

/// Sweep parameter for optimization
/// Used in backtest config to specify parameter ranges for optimization
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SweepParam {
    /// Range sweep: [min..max] or [min..max, step: step]
    Range {
        min: Box<super::expressions::Expr>,
        max: Box<super::expressions::Expr>,
        step: Option<Box<super::expressions::Expr>>,
    },
    /// Discrete values: [v1, v2, v3, ...]
    Discrete(Vec<super::expressions::Expr>),
}
