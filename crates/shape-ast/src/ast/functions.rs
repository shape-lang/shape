//! Function definition and parameter types for Shape AST

use super::DocComment;
use super::expressions::Expr;
use super::span::Span;
use super::statements::Statement;
use super::types::TypeAnnotation;
use serde::{Deserialize, Serialize};
// Re-export TypeParam from types to avoid duplication
pub use super::types::TypeParam;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDef {
    pub name: String,
    pub name_span: Span,
    /// Declaring module path for compiler/runtime provenance checks.
    ///
    /// This is injected by the module loader for loaded modules and is not part
    /// of user-authored source syntax.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaring_module_path: Option<String>,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_params: Option<Vec<TypeParam>>,
    pub params: Vec<FunctionParameter>,
    pub return_type: Option<TypeAnnotation>,
    pub where_clause: Option<Vec<super::types::WherePredicate>>,
    pub body: Vec<Statement>,
    pub annotations: Vec<Annotation>,
    pub is_async: bool,
    /// Whether this function is compile-time-only (`comptime fn`).
    ///
    /// Comptime-only functions can only be called from comptime contexts.
    #[serde(default)]
    pub is_comptime: bool,
}

/// A foreign function definition: `fn <language> name(params) -> type { foreign_body }`
///
/// The body is raw source text in the foreign language, not parsed as Shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForeignFunctionDef {
    /// The language identifier (e.g., "python", "julia", "sql")
    pub language: String,
    pub language_span: Span,
    pub name: String,
    pub name_span: Span,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    pub type_params: Option<Vec<TypeParam>>,
    pub params: Vec<FunctionParameter>,
    pub return_type: Option<TypeAnnotation>,
    /// The raw dedented source text of the foreign function body.
    pub body_text: String,
    /// Span of the body text in the original Shape source file.
    pub body_span: Span,
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub is_async: bool,
    /// Native ABI metadata for `extern "C"` declarations.
    ///
    /// When present, this foreign function is not compiled/invoked through a
    /// language runtime extension. The VM links and invokes it via the native
    /// C ABI path.
    #[serde(default)]
    pub native_abi: Option<NativeAbiBinding>,
}

/// Native ABI link metadata attached to a foreign function declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeAbiBinding {
    /// ABI name (currently `"C"`).
    pub abi: String,
    /// Library path or logical dependency key.
    pub library: String,
    /// Symbol name to resolve in the library.
    pub symbol: String,
    /// Declaring package identity for package-scoped native resolution.
    ///
    /// This is compiler/runtime metadata, not source syntax.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_key: Option<String>,
}

impl ForeignFunctionDef {
    /// Whether the declared return type is `Result<T>`.
    pub fn returns_result(&self) -> bool {
        matches!(
            &self.return_type,
            Some(TypeAnnotation::Generic { name, .. }) if name == "Result"
        )
    }

    /// Whether this function uses native ABI binding (e.g. `extern "C"`).
    pub fn is_native_abi(&self) -> bool {
        self.native_abi.is_some()
    }

    /// Validate that all parameter and return types are explicitly annotated,
    /// and that dynamic-language foreign functions declare `Result<T>` as their
    /// return type.
    ///
    /// Foreign function bodies are opaque — the type system cannot infer types
    /// from them. This returns a list of `(message, span)` for each problem,
    /// shared between the compiler and the LSP.
    ///
    /// `dynamic_language` should be `true` for languages like Python, JS, Ruby
    /// where every call can fail at runtime.  Currently all foreign languages
    /// are treated as dynamic (the ABI declares this via `ErrorModel`).
    pub fn validate_type_annotations(&self, dynamic_language: bool) -> Vec<(String, Span)> {
        let mut errors = Vec::new();

        for param in &self.params {
            if param.type_annotation.is_none() {
                let name = param.simple_name().unwrap_or("_");
                errors.push((
                    format!(
                        "Foreign function '{}': parameter '{}' requires a type annotation \
                         (type inference is not available for foreign function bodies)",
                        self.name, name
                    ),
                    param.span(),
                ));
            }
        }

        if self.return_type.is_none() {
            errors.push((
                format!(
                    "Foreign function '{}' requires an explicit return type annotation \
                     (type inference is not available for foreign function bodies)",
                    self.name
                ),
                self.name_span,
            ));
        } else if dynamic_language && !self.returns_result() {
            let inner_type = self
                .return_type
                .as_ref()
                .map(|t| t.to_type_string())
                .unwrap_or_else(|| "T".to_string());
            errors.push((
                format!(
                    "Foreign function '{}': return type must be Result<{}> \
                     (dynamic language runtimes can fail on every call)",
                    self.name, inner_type
                ),
                self.name_span,
            ));
        }

        errors
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionParameter {
    pub pattern: super::patterns::DestructurePattern,
    #[serde(default)]
    pub is_const: bool,
    #[serde(default)]
    pub is_reference: bool,
    /// Whether this is an exclusive (mutable) reference: `&mut x`
    /// Only meaningful when `is_reference` is true.
    #[serde(default)]
    pub is_mut_reference: bool,
    /// Whether this is an `out` parameter (C out-pointer pattern).
    /// Only valid on `extern C fn` declarations. The compiler auto-generates
    /// cell allocation, C call, value readback, and cell cleanup.
    #[serde(default)]
    pub is_out: bool,
    pub type_annotation: Option<TypeAnnotation>,
    pub default_value: Option<Expr>,
}

impl FunctionParameter {
    /// Get the simple parameter name if this is a simple identifier pattern
    pub fn simple_name(&self) -> Option<&str> {
        self.pattern.as_identifier()
    }

    /// Get all identifiers bound by this parameter (for destructuring patterns)
    pub fn get_identifiers(&self) -> Vec<String> {
        self.pattern.get_identifiers()
    }

    /// Get the span for this parameter
    pub fn span(&self) -> Span {
        match &self.pattern {
            super::patterns::DestructurePattern::Identifier(_, span) => *span,
            super::patterns::DestructurePattern::Array(_) => Span::default(),
            super::patterns::DestructurePattern::Object(_) => Span::default(),
            super::patterns::DestructurePattern::Rest(_) => Span::default(),
            super::patterns::DestructurePattern::Decomposition(_) => Span::default(),
        }
    }
}

// Note: TypeParam is re-exported from types module above

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub name: String,
    pub args: Vec<Expr>,
    pub span: Span,
}

impl Annotation {
    pub fn get<'a>(annotations: &'a [Annotation], name: &str) -> Option<&'a Annotation> {
        annotations.iter().find(|a| a.name == name)
    }
}

/// Annotation definition with lifecycle hooks
///
/// Annotations are Shape's aspect-oriented programming mechanism.
/// They can define handlers for different lifecycle events:
///
/// ```shape
/// annotation pattern() {
///     on_define(fn, ctx) { ctx.registry("patterns").set(fn.name, fn); }
///     metadata() { return { is_pattern: true }; }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotationDef {
    pub name: String,
    pub name_span: Span,
    #[serde(default)]
    pub doc_comment: Option<DocComment>,
    /// Annotation parameters (e.g., `period` in `@warmup(period)`)
    pub params: Vec<FunctionParameter>,
    /// Optional explicit target restrictions from `targets: [...]`.
    /// If None, target applicability is inferred from handler kinds.
    pub allowed_targets: Option<Vec<AnnotationTargetKind>>,
    /// Lifecycle handlers (on_define, before, after, metadata)
    pub handlers: Vec<AnnotationHandler>,
    /// Full span of the annotation definition
    pub span: Span,
}

/// Type of annotation lifecycle handler
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnnotationHandlerType {
    /// Called when function is defined (registration time)
    OnDefine,
    /// Called before each function invocation
    Before,
    /// Called after each function invocation
    After,
    /// Returns static metadata for tooling/optimization
    Metadata,
    /// Compile-time pre-inference handler: `comptime pre(target, ctx) { ... }`
    /// Can emit directives to concretize untyped function parameters.
    ComptimePre,
    /// Compile-time post-inference handler: `comptime post(target, ctx) { ... }`
    /// Can emit directives to synthesize return types and runtime bodies.
    ComptimePost,
}

/// Describes what kind of syntax element an annotation is targeting.
/// Used for compile-time validation of annotation applicability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationTargetKind {
    /// @annotation before a function definition
    Function,
    /// @annotation before a type/struct/enum definition
    Type,
    /// @annotation before a module definition
    Module,
    /// @annotation before an arbitrary expression
    Expression,
    /// @annotation before a block expression
    Block,
    /// @annotation inside an await expression: `await @timeout(5s) expr`
    AwaitExpr,
    /// @annotation before a let/var/const binding
    Binding,
}

/// A lifecycle handler within an annotation definition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotationHandler {
    /// Type of handler (on_define, before, after, metadata)
    pub handler_type: AnnotationHandlerType,
    /// Handler parameters (e.g., `fn, ctx` for on_define)
    pub params: Vec<AnnotationHandlerParam>,
    /// Optional return type annotation
    pub return_type: Option<TypeAnnotation>,
    /// Handler body (a block expression)
    pub body: Expr,
    /// Span for error reporting
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotationHandlerParam {
    pub name: String,
    #[serde(default)]
    pub is_variadic: bool,
}
