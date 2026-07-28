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
    /// ADR-014 §8.2: the row this boundary declares, if the source spelled a
    /// `!` clause. `None` is "no declared row", not "pure".
    #[serde(default)]
    pub effect_row: Option<super::types::EffectRowAnnotation>,
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

/// The `[C0932]` foreign-async rejection produced by
/// [`ForeignFunctionDef::unsupported_async_rejection`].
///
/// TRANSITIONAL — deleted with its producer when issue #202
/// (POLY-ASYNC-OFFLOAD) lands.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignAsyncRejection {
    /// Full diagnostic sentence, `[C0932]`-tagged.
    pub message: String,
    /// Declaration-name span the rejection anchors at.
    pub span: Span,
    /// The semantics-preserving fix-it, rendered as a diagnostic hint.
    pub fix_hint: String,
}

/// The `[C0934]` unenforced-effect-row rejection produced by
/// [`FunctionDef::unenforced_effect_row_rejection`].
///
/// TRANSITIONAL — deleted with its producer when declaration-vs-body effect
/// enforcement lands (issue #143, EFFECT-CONTRACT).
#[derive(Debug, Clone, PartialEq)]
pub struct UnenforcedEffectRowRejection {
    /// Full diagnostic sentence, `[C0934]`-tagged.
    pub message: String,
    /// The clause's own span, so the squiggle sits under the `!` clause and
    /// not under the function name.
    pub span: Span,
    /// The semantics-preserving fix-it, rendered as a diagnostic hint.
    pub fix_hint: String,
}

/// The remedy for a `[C0933]` rejection, per the reason the type is unmapped.
///
/// Each hint names a concrete rewrite that keeps the call — none of them say
/// "use a different type" without saying which.
fn unmapped_fix_hint(
    unmapped: &shape_abi_v1::foreign_types::UnmappedForeignType,
    direction: shape_abi_v1::foreign_types::ForeignDirection,
) -> String {
    use shape_abi_v1::foreign_types::{ForeignDirection, UnmappedReason};

    let position = match direction {
        ForeignDirection::Argument => "parameter",
        ForeignDirection::Return => "return type",
    };
    match unmapped.reason {
        UnmappedReason::NoWireProjection => format!(
            "convert on the Shape side of the call: pass `{spelling}` as a `string` (or as \
             `int` / `number` if it is numeric) and rebuild it in the foreign body. Shape \
             types with no wire form — DataTable, DateTime, decimal, bigint, char, and the \
             collection types beyond Array and HashMap — have no MessagePack projection.",
            spelling = unmapped.spelling,
        ),
        UnmappedReason::NonScalarElement => format!(
            "only `Array<int>`, `Array<number>`, `Array<bool>` and `Array<string>` cross. \
             Send `{spelling}`'s fields as parallel scalar arrays, or declare an object \
             type and pass one value per call.",
            spelling = unmapped.spelling,
        ),
        UnmappedReason::NonStringMapKey => {
            "declare the map as `HashMap<string, V>` — MessagePack map keys cross as strings."
                .to_string()
        }
        UnmappedReason::UnsupportedConstructor => format!(
            "`{spelling}` has no foreign form. Tuples, function types, unions, references \
             and trait objects do not cross; declare an object type (`{{ a: int, b: string }}` \
             or a named `type`) with the fields you need.",
            spelling = unmapped.spelling,
        ),
        UnmappedReason::WrongDirection => format!(
            "a foreign map crosses inward only. Declare the {position} as an object type — \
             `{{ a: int }}` or a named `type` — which has a projection in both directions.",
        ),
        UnmappedReason::BadArity => format!(
            "check the type arguments on `{spelling}`.",
            spelling = unmapped.spelling,
        ),
    }
}

/// A `[C0933]` rejection: a declared parameter or return type that cannot cross
/// the foreign boundary.
///
/// Produced by [`ForeignFunctionDef::unmapped_foreign_types`] and consumed by
/// both the compiler and the LSP, so the editor and `shape run` cannot disagree
/// about whether a signature is expressible.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignTypeRejection {
    /// Full diagnostic sentence, `[C0933]`-tagged.
    pub message: String,
    /// The declaration-site span: the parameter for a parameter, the function
    /// name for the return type.
    pub span: Span,
    /// The remedy, rendered as a diagnostic hint.
    pub fix_hint: String,
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

impl FunctionDef {
    /// ADR-014 §8.2 / R21 (issue #178) — an effect clause on a function
    /// DECLARATION is a compile error until declaration-vs-body enforcement
    /// lands.
    ///
    /// MEASURED before this guard, at the CLI:
    ///
    /// ```shape
    /// use std::core::file
    /// fn sneaky(path: string) -> string ! {} {
    ///     return file::read_text(path)
    /// }
    /// ```
    ///
    /// compiled, ran, and printed the file. The row-in-type slice landed the
    /// type component, the subset judgment and the boundary check, but not the
    /// body walk that derives a callable's ACTUAL row from its call edges
    /// (§8.2's monotone least-fixpoint). With nothing computing the actual
    /// row, a declared row is compared against nothing — so `! {}` was a
    /// purity claim the compiler never checked and the runtime never enforced.
    ///
    /// The program's precedent for exactly this shape is the grill Q3 ruling
    /// (POLY-ASYNC-TRUTH, issue #201): reject the unenforceable spelling until
    /// the enforcement is real, rather than ship a contract that lies.
    ///
    /// Scope is deliberately the DECLARATION position only. Rows on function
    /// TYPES — parameters, higher-order signatures, and `effect F` binders —
    /// stay live, because those the solver genuinely checks through the
    /// subset judgment whenever a function value meets a declared parameter.
    /// A type-position row is checked evidence; a declaration-position row was
    /// an unchecked claim. Only the claim is refused.
    ///
    /// Shared by the compiler and the editor — the LSP surfaces this through
    /// the compile path it already runs rather than through a second
    /// hand-written validator, so the two texts cannot drift.
    ///
    /// TRANSITIONAL — this producer is deleted when #143 lands.
    /// `grep -rn "C0934"` returns the full deletion set.
    pub fn unenforced_effect_row_rejection(&self) -> Option<UnenforcedEffectRowRejection> {
        let row = self.effect_row.as_ref()?;
        Some(UnenforcedEffectRowRejection {
            message: format!(
                "[C0934] the effect clause `{clause}` on `fn {name}` is not enforced yet, so it \
                 cannot be declared here. Effect rows are live as a TYPE component — on \
                 parameters, on higher-order signatures, and as `effect F` binders — and those \
                 are really checked by subset subsumption. What does not exist yet is the body \
                 walk that derives what `{name}` actually does and compares it against this \
                 clause (ADR-014 §8.2). Until it does, a declared row is a promise nothing \
                 verifies: `! {{}}` on a function that reads a file would compile and run. \
                 Rather than ship a contract that can lie, the declaration is refused — the same \
                 call the program made for `async` foreign declarations in issue #201. \
                 Declaration-vs-body enforcement is planned, not refused: see issue #143 \
                 (EFFECT-CONTRACT). This rejection is transitional and is deleted when that \
                 lands.",
                clause = row.render(),
                name = self.name,
            ),
            span: *row.span(),
            fix_hint: format!(
                "delete the `{clause}` clause from `fn {name}`. That preserves today's semantics \
                 exactly — nothing was checking the row, so removing it removes no guarantee. \
                 Effect rows you want checked NOW go in type position instead: a callback \
                 parameter typed `f: fn() -> T ! {{FsRead}}` is enforced against the closure you \
                 pass it.",
                clause = row.render(),
                name = self.name,
            ),
        })
    }
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

    /// ADR-019 §5 step 1 (POLY-ASYNC-TRUTH, issue #201) — the `[C0932]`
    /// rejection for an `async` declaration on a language-runtime foreign
    /// function.
    ///
    /// MEASURED before this rejection existed: nothing on the SHAPE side
    /// consumed the `async`. `predeclare_foreign_function` resolved the
    /// declared return annotation verbatim with no `Future<T>` wrapping, and
    /// the executor's `CallForeign` arm invoked the extension SYNCHRONOUSLY on
    /// the VM thread.
    ///
    /// The extensions DO act on `is_async`, and the remedy wording below is
    /// load-bearing because of it: the Python extension wraps the body in
    /// `async def` + `asyncio.run(...)`
    /// (`extensions/python/src/runtime.rs:203`) and the TypeScript extension
    /// in `async function` driven by a cached tokio runtime
    /// (`extensions/typescript/src/runtime.rs:90`). So an async foreign body
    /// may legitimately contain `await` today, and its awaits really do run —
    /// they just run to completion while the VM thread blocks. Deleting the
    /// keyword is therefore semantics-preserving for a body that does not
    /// `await`, and NOT sufficient on its own for one that does. Do not
    /// flatten the hint back to a bare "remove `async`".
    ///
    /// **TRANSITIONAL.** ADR-019 §5 rules real foreign async IN — the invoke
    /// offloaded off-thread and resolved at `await`, issue #202
    /// (POLY-ASYNC-OFFLOAD). This producer is deleted when that lands; the
    /// compiler's negative tests are its flip-to-green control.
    ///
    /// Native-ABI declarations (`extern "C"`) are OUT of scope: they are not
    /// language runtimes, #202 does not make them async, and they already carry
    /// their own older rejection at the compile site. The two must not collapse.
    ///
    /// Shared by the compiler and the LSP — one text, so the editor and
    /// `shape run` cannot disagree about whether the surface exists (the same
    /// seam as [`Self::validate_type_annotations`]).
    pub fn unsupported_async_rejection(&self) -> Option<ForeignAsyncRejection> {
        if !self.is_async || self.is_native_abi() {
            return None;
        }
        Some(ForeignAsyncRejection {
            message: format!(
                "[C0932] `async fn {language} {name}` is not supported — a foreign call runs \
                 synchronously on the VM thread and its declared return type is not a future. The \
                 body's own `await`s do run to completion inside the extension, but the caller \
                 cannot overlap anything with them, so `async` here promises a concurrency the \
                 runtime does not provide (ADR-019 §5 forbids this untruthful contract). Real \
                 foreign async — the invoke offloaded off-thread and resolved at `await` — is \
                 planned, not refused: see issue #202 (POLY-ASYNC-OFFLOAD). This rejection is \
                 transitional and is deleted when that lands.",
                language = self.language,
                name = self.name,
            ),
            span: self.name_span,
            fix_hint: format!(
                "remove the `async` keyword: the call already runs synchronously, so `fn \
                 {language} {name}` keeps exactly the semantics it has today if its body does not \
                 `await`. A body that DOES `await` must drive its own completion inside the body \
                 until #202 lands (python: wrap it in `asyncio.run(...)`; typescript: resolve the \
                 promise before returning).",
                language = self.language,
                name = self.name,
            ),
        })
    }

    /// ADR-019 §1 / R25 (POLY-STUB-CHANNEL, issue #196) — the `[C0933]`
    /// rejections for declared types that cannot cross the foreign boundary.
    ///
    /// Every parameter and the return type are classified against the canonical
    /// marshaling table (`shape_abi_v1::foreign_types`). A type outside the
    /// table used to compile fine and fail on the first call, deep inside
    /// `foreign_marshal`, with a `NotImplemented` naming an internal stage
    /// number; the author saw it only at runtime and only on the path that hit
    /// it. The table is knowable at the declaration, so the diagnostic belongs
    /// there.
    ///
    /// Direction matters: the table is not symmetric. `HashMap<string, V>` has
    /// an outbound projection and no inbound one, so it is legal as a parameter
    /// and refused as a return type.
    ///
    /// Native-ABI declarations (`extern "C"`) are OUT of scope — they marshal
    /// through libffi against C types (`ptr`, `i32`, `cslice`, …), a different
    /// table with its own checks in `build_native_c_signature`.
    ///
    /// Shared by the compiler and the LSP — one text, so the editor and
    /// `shape run` cannot disagree (the same seam as
    /// [`Self::validate_type_annotations`] and
    /// [`Self::unsupported_async_rejection`]).
    pub fn unmapped_foreign_types(&self) -> Vec<ForeignTypeRejection> {
        use shape_abi_v1::foreign_types::{ForeignDirection, ForeignType};

        if self.is_native_abi() {
            return Vec::new();
        }

        let mut rejections = Vec::new();

        for param in &self.params {
            let Some(annotation) = &param.type_annotation else {
                // Missing annotations are `validate_type_annotations`' error;
                // reporting both at one site would be noise.
                continue;
            };
            let declared = annotation.to_type_string();
            if let Err(unmapped) = ForeignType::classify(&declared, ForeignDirection::Argument) {
                let param_name = param.simple_name().unwrap_or("_");
                rejections.push(ForeignTypeRejection {
                    message: format!(
                        "[C0933] `fn {language} {name}`: parameter '{param_name}' is declared \
                         `{declared}`, which cannot cross the foreign boundary — `{spelling}` \
                         is unusable because {why}. Values cross as MessagePack, so the \
                         declared type must be one the marshaling table projects (ADR-019 §1).",
                        language = self.language,
                        name = self.name,
                        spelling = unmapped.spelling,
                        why = unmapped.reason.explain(),
                    ),
                    span: param.span(),
                    fix_hint: unmapped_fix_hint(&unmapped, ForeignDirection::Argument),
                });
            }
        }

        if let Some(return_annotation) = &self.return_type {
            let declared = return_annotation.to_type_string();
            if let Err(unmapped) = ForeignType::classify(&declared, ForeignDirection::Return) {
                rejections.push(ForeignTypeRejection {
                    message: format!(
                        "[C0933] `fn {language} {name}`: the return type is declared \
                         `{declared}`, which cannot cross the foreign boundary — `{spelling}` \
                         is unusable because {why}. Values cross as MessagePack, so the \
                         declared type must be one the marshaling table projects (ADR-019 §1).",
                        language = self.language,
                        name = self.name,
                        spelling = unmapped.spelling,
                        why = unmapped.reason.explain(),
                    ),
                    span: self.name_span,
                    fix_hint: unmapped_fix_hint(&unmapped, ForeignDirection::Return),
                });
            }
        }

        rejections
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
    /// Optional explicit target restrictions from the header `on`-clause
    /// (`annotation name(...) on function, type`). If None, target
    /// applicability is inferred from handler kinds.
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
