//! Function definition and parameter types for Shape AST

use super::DocComment;
use super::expressions::Expr;
use super::span::Span;
use super::statements::Statement;
use super::types::TypeAnnotation;
use serde::{Deserialize, Serialize};
// Re-export TypeParam from types to avoid duplication
pub use super::types::TypeParam;

/// ADR-019 §2 (#199). Defined in the ABI crate, not here: the compiler, the
/// marshal layer and the extension stub renderers all decide on it, and
/// `shape-abi-v1` is the only crate all three can see. Re-exported so the AST
/// reads as if it owned it.
pub use shape_abi_v1::foreign_types::BufferShare;

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

/// A `[C0935]` rejection: a `shared` / `shared mut` parameter spelling that
/// cannot mean what it says (ADR-019 §2 / #199).
///
/// Produced by [`ForeignFunctionDef::invalid_buffer_shares`] and
/// [`FunctionDef::misplaced_buffer_shares`], and consumed by both the compiler
/// and the LSP, so the editor and `shape run` cannot disagree about whether a
/// signature may share.
#[derive(Debug, Clone, PartialEq)]
pub struct BufferShareRejection {
    /// Full diagnostic sentence, `[C0935]`-tagged.
    pub message: String,
    /// The parameter's own span.
    pub span: Span,
    /// The remedy, rendered as a diagnostic hint.
    pub fix_hint: String,
}

/// The reason one `shared` parameter is refused. Kept separate from the message
/// so the compiler and the LSP can classify without parsing prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferShareRefusal {
    /// `shared` on an ordinary Shape function, closure, or method — there is no
    /// foreign boundary for a view to cross.
    NotAForeignDeclaration,
    /// `shared` on an `extern "C"` declaration. Native calls marshal through
    /// libffi against C types, where a buffer is spelled `ptr` and the
    /// lifetime rules are the C ones.
    NativeAbi,
    /// The parameter's declared type is not an array at all.
    NotAnArray,
    /// An array whose element type has no shareable buffer projection.
    ElementNotShareable,
    /// A parameter with no type annotation — nothing to classify.
    Unannotated,
    /// `shared` combined with `&` / `&mut` / `out`.
    ConflictingPassMode,
    /// More than [`shape_abi_v1::MAX_SHARED_VIEWS`] shared parameters on one
    /// declaration.
    TooManyViews,
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
    /// stay writable, because there the row is a component of the type: it
    /// participates in type identity and unification, and the subset judgment
    /// that decides it exists and is exercised
    /// (`ConstraintSolver::check_declared_boundary`).
    ///
    /// What a type-position row is NOT, at this revision, is *enforced*: no
    /// production checking path calls that seam, so passing a `! {FsRead}`
    /// function value where `! {}` is declared still compiles. Measured
    /// 2026-07-28 under #180 — an earlier revision of this text claimed those
    /// rows "are really checked", which was true of the judgment and false of
    /// the program. Production wiring of the boundary check lands with #143,
    /// alongside the body walk.
    ///
    /// The declaration position is refused and the type position is not,
    /// because the two gaps differ in kind. A declaration-position row is a
    /// claim about a body that nothing derives, and nothing ever will until
    /// the body walk exists. A type-position row is a constraint on callers
    /// that the existing seam already decides correctly — what is missing is
    /// the call to it.
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
                 parameters, on higher-order signatures, and as `effect F` binders — where the \
                 row is carried in the type and decided by the subset judgment at the boundary \
                 seam. Two things do not exist yet: the body walk that would derive what \
                 `{name}` actually does, and the production call to that seam (ADR-014 §8.2). \
                 Until they do, a declared row is a promise nothing verifies: `! {{}}` on a \
                 function that reads a file would compile and run. Rather than ship a contract \
                 that can lie, the declaration is refused — the same call the program made for \
                 `async` foreign declarations in issue #201. Enforcement is planned, not \
                 refused: both halves land with issue #143 (EFFECT-CONTRACT). This rejection is \
                 transitional and is deleted when that lands.",
                clause = row.render(),
                name = self.name,
            ),
            span: *row.span(),
            fix_hint: format!(
                "delete the `{clause}` clause from `fn {name}`. That preserves today's semantics \
                 exactly — nothing was checking the row, so removing it removes no guarantee. \
                 A row in TYPE position — a callback parameter typed `f: fn() -> T ! {{FsRead}}` \
                 — is accepted and is carried in the type, so writing one documents the \
                 boundary and will start checking the moment #143 wires the seam. It is not \
                 checked against the value you pass yet either; nothing in Shape enforces an \
                 effect row at this revision.",
                clause = row.render(),
                name = self.name,
            ),
        })
    }
}

impl FunctionDef {
    /// ADR-019 §2 / R25 (POLY-ZERO-COPY, issue #199) — the `[C0935]` rejections
    /// for `shared` / `shared mut` on an ordinary Shape function.
    ///
    /// The grammar accepts the word on any parameter deliberately. Refusing it
    /// in the parser would report "expected pattern" under the parameter name
    /// and leave the author to work out that the word they wrote is the problem;
    /// accepting it and refusing it here reports what they actually did.
    ///
    /// Shape-to-Shape calls never had the copy this word removes, so there is
    /// nothing here for it to buy — which is what the diagnostic says.
    pub fn misplaced_buffer_shares(&self) -> Vec<BufferShareRejection> {
        let mut rejections = Vec::new();
        for param in &self.params {
            let share = param.buffer_share;
            if !share.is_shared() {
                continue;
            }
            rejections.push(buffer_share_rejection(
                BufferShareRefusal::NotAForeignDeclaration,
                &format!("fn {}", self.name),
                param.simple_name().unwrap_or("_"),
                share.spelling().unwrap_or("shared"),
                &param
                    .type_annotation
                    .as_ref()
                    .map(|a| a.to_type_string())
                    .unwrap_or_else(|| "?".to_string()),
                param.span(),
            ));
        }
        rejections
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
    /// [`Self::validate_type_annotations`]). The third member of that family,
    /// the `[C0932]` foreign-async rejection, was deleted with #202: `async fn
    /// python` / `async fn typescript` now compile to a real off-thread
    /// offload.
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

    /// ADR-019 §2 / R25 (POLY-ZERO-COPY, issue #199) — the `[C0935]` rejections
    /// for `shared` / `shared mut` parameters that cannot mean what they say.
    ///
    /// Sharing exports a pointer into live host memory to foreign code for the
    /// duration of the call. Whether that is even expressible is decided by the
    /// declaration alone — the language, the pass mode, and the element type are
    /// all right there — so the answer belongs at the declaration and not at the
    /// first call that happens to take the path.
    ///
    /// The element set is `int` and `number`, and the exclusions are soundness
    /// rather than effort; [`shape_abi_v1::foreign_types::ForeignScalar::buffer_elem`]
    /// carries the reasons.
    ///
    /// Whether the LOADED extension can actually honour the mode is a different
    /// question with a different answer per host, and it is checked at the call
    /// against the negotiated capability. This check is the part that is true
    /// everywhere.
    ///
    /// Shared by the compiler and the LSP — one text, so the editor and
    /// `shape run` cannot disagree, the same seam as
    /// [`Self::unmapped_foreign_types`].
    pub fn invalid_buffer_shares(&self) -> Vec<BufferShareRejection> {
        use shape_abi_v1::foreign_types::{ForeignDirection, ForeignType};

        let mut rejections = Vec::new();
        let mut shared_seen = 0usize;

        for param in &self.params {
            let share = param.buffer_share;
            if !share.is_shared() {
                continue;
            }
            let spelling = share.spelling().unwrap_or("shared");
            let param_name = param.simple_name().unwrap_or("_");
            shared_seen += 1;

            let refusal = if self.is_native_abi() {
                BufferShareRefusal::NativeAbi
            } else if param.is_reference || param.is_out {
                BufferShareRefusal::ConflictingPassMode
            } else if shared_seen > shape_abi_v1::MAX_SHARED_VIEWS {
                BufferShareRefusal::TooManyViews
            } else {
                match &param.type_annotation {
                    None => BufferShareRefusal::Unannotated,
                    Some(annotation) => {
                        let declared = annotation.to_type_string();
                        match ForeignType::classify(&declared, ForeignDirection::Argument) {
                            Ok(ForeignType::Array(elem)) if elem.buffer_elem().is_some() => {
                                continue;
                            }
                            Ok(ForeignType::Array(_)) => BufferShareRefusal::ElementNotShareable,
                            // A type that cannot cross at all is already a
                            // `[C0933]`; reporting the share on top of it would
                            // be two diagnostics for one mistake.
                            Err(_) => continue,
                            Ok(_) => BufferShareRefusal::NotAnArray,
                        }
                    }
                }
            };

            let declared = param
                .type_annotation
                .as_ref()
                .map(|a| a.to_type_string())
                .unwrap_or_else(|| "?".to_string());
            rejections.push(buffer_share_rejection(
                refusal,
                &format!("fn {} {}", self.language, self.name),
                param_name,
                spelling,
                &declared,
                param.span(),
            ));
        }

        rejections
    }
}

/// Build one `[C0935]` rejection. Shared by the foreign and the ordinary
/// declaration checks so the two cannot drift into different words for the same
/// refusal.
fn buffer_share_rejection(
    refusal: BufferShareRefusal,
    subject: &str,
    param_name: &str,
    spelling: &str,
    declared: &str,
    span: Span,
) -> BufferShareRejection {
    let (why, fix_hint) = match refusal {
        BufferShareRefusal::NotAForeignDeclaration => (
            "there is no foreign boundary here for a view to cross. `shared` declares that \
             a buffer is exported to foreign code instead of copied onto the MessagePack \
             wire, which is only a question a `fn python` / `fn typescript` declaration \
             asks. Shape-to-Shape calls already pass arrays without copying."
                .to_string(),
            format!(
                "delete `{spelling}` from '{param_name}'. Shape's own calls do not copy an \
                 array to pass it, so removing the word costs nothing and changes nothing. \
                 If you meant to take a reference, that is `&` / `&mut`."
            ),
        ),
        BufferShareRefusal::NativeAbi => (
            "an `extern \"C\"` declaration marshals through libffi against C types, where \
             a buffer is spelled `ptr` and its lifetime is the C contract's business. \
             `shared` is the language-runtime boundary's word (ADR-019 §2) and has no \
             meaning on the native one."
                .to_string(),
            format!(
                "delete `{spelling}` from '{param_name}' and declare the parameter as `ptr` \
                 if the C function takes a buffer, passing the length alongside it as the \
                 C signature requires."
            ),
        ),
        BufferShareRefusal::NotAnArray => (
            format!(
                "'{param_name}' is declared `{declared}`, which has no buffer to share. A \
                 view is a window onto a contiguous native array; a scalar, an object or a \
                 map has no such window."
            ),
            format!(
                "delete `{spelling}` from '{param_name}'. `{declared}` crosses by copy, and \
                 for a value of that size the copy is not what costs you anything."
            ),
        ),
        BufferShareRefusal::ElementNotShareable => (
            format!(
                "'{param_name}' is declared `{declared}`, whose element type has no \
                 shareable buffer. `Array<int>` and `Array<number>` are contiguous `i64` / \
                 `f64` that foreign code can read directly. `Array<bool>` is one byte per \
                 element with only 0 and 1 valid, so a writable view could put a value in \
                 it that is neither `true` nor `false`; `Array<string>` holds host pointers \
                 with host refcounts, so sharing it would export the heap."
            ),
            format!(
                "delete `{spelling}` from '{param_name}', or change the element type to \
                 `int` or `number` if the data is numeric — those are the two that cross \
                 without a copy."
            ),
        ),
        BufferShareRefusal::Unannotated => (
            format!(
                "'{param_name}' has no type annotation, so there is nothing to decide the \
                 buffer layout from."
            ),
            format!(
                "annotate '{param_name}' — `shared {param_name}: Array<number>` or \
                 `Array<int>` — or delete `{spelling}`."
            ),
        ),
        BufferShareRefusal::ConflictingPassMode => (
            format!(
                "'{param_name}' already declares a pass mode. `shared` IS the borrow: \
                 `shared` is the immutable view and `shared mut` the exclusive one, both \
                 scoped to the call, so combining it with `&`, `&mut` or `out` would be \
                 declaring the same thing twice in two vocabularies."
            ),
            format!(
                "keep one: write `{spelling} {param_name}: Array<number>` and drop the \
                 sigil."
            ),
        ),
        BufferShareRefusal::TooManyViews => (
            format!(
                "this declaration shares more than {max} parameters. The host asks the \
                 extension which views were released when the body returned, and that \
                 answer is a {max}-bit mask — a view past the limit could not be accounted \
                 for, and an unaccounted view is exactly what sharing must never leave \
                 behind.",
                max = shape_abi_v1::MAX_SHARED_VIEWS,
            ),
            format!(
                "share at most {max} parameters per declaration; pack the rest into fewer, \
                 longer arrays, or copy them.",
                max = shape_abi_v1::MAX_SHARED_VIEWS,
            ),
        ),
    };

    BufferShareRejection {
        message: format!(
            "[C0935] `{subject}`: parameter '{param_name}' is declared `{spelling}`, but \
             {why}"
        ),
        span,
        fix_hint,
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
    /// How this parameter's buffer crosses a foreign call — the `shared` /
    /// `shared mut` spelling (ADR-019 §2 / #199).
    ///
    /// Legal only on a `fn <language>` declaration, and only over a buffer the
    /// marshaling table can project natively; everywhere else a non-default
    /// value is a `[C0935]` rejection. It lives on the shared parameter type
    /// rather than on a foreign-only one so that "you wrote `shared` where it
    /// means nothing" is a diagnostic rather than a parse error the author has
    /// to decode.
    #[serde(default)]
    pub buffer_share: BufferShare,
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
