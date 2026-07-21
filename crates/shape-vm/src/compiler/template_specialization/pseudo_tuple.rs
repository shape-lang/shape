//! ADR-009 C3 #14 (slice 1) — the G9 pseudo-tuple core: ONE traversal, two
//! faces (the construction-time usage VALIDATOR and the specialization-time
//! REWRITER).
//!
//! # The G9 ruling this enforces
//!
//! C3-G9 resolved the Args carrier as a SPECIALIZATION-RESOLVED PSEUDO-TUPLE:
//! `args[i]` / `args.length` are TEMPLATE-LEVEL constructs. No tuple value ever
//! exists at runtime — at specialization ([`resolve_pseudo_tuple`], S1c
//! stage 4), a constant-index `args[i]` resolves to the target's i-th typed
//! parameter slot, `args.length` resolves to a constant, and a mutation-return
//! (`return args` / a final bare `args`) specializes to a COMPILER-INTERNAL
//! per-target aggregate at the weave boundary (never user-visible; never the
//! boxed `NewArray` path).
//!
//! # One walker, four faces (binding invariant — do not fork)
//!
//! This module is the single traversal core. [`Face::Validate`] is the
//! construction face ([`validate_pseudo_tuple_uses`], run from
//! `CheckedTemplateBuilder::finish()`); [`Face::Rewrite`] is the
//! specialization face ([`resolve_pseudo_tuple`], run from the
//! monomorphization ride under a [`TemplateSpecializationPlan`]);
//! [`Face::AmbientScope`] is the S5 [C0926] ambient-totality face
//! ([`scan_template_body_ambient_scope`], run from
//! `specialize_template` BEFORE the per-kind dispatch — see the gate's doc
//! comment in `mod.rs` for the site rationale);
//! [`Face::AfterReturnScan`] is the S6 after-side return-kind collection
//! face ([`guard_after_template_return_kind`], run from
//! `specialize_polymorphic_after` BEFORE the ride — the S2c analog for the
//! `(R) -> R` bound). ALL faces share every shape
//! classifier and every named rejection — there is no second, drifting
//! walker; the validate/ambient/after-return faces take `&mut` for
//! traversal uniformity only and never mutate (every mutation sits behind
//! `Face::Rewrite`). The
//! traversal mirrors the exhaustive Statement/Expr skeleton of
//! `monomorphization/substitution.rs` (the precedent full-AST walk): every
//! `match` is exhaustive with no catch-all arm, so a new AST variant is a
//! compile error here, exactly like the substitution walker.
//!
//! # The G9 aggregate is ADR-006-ordinary (compliance statement)
//!
//! The multi-parameter mutation carrier the rewriter emits is the ORDINARY
//! inline-schema TypedObject the ordinary pipeline emits for a typed object
//! literal — `HeapValue::TypedObject(Arc<TypedObjectStorage>)` per ADR-006
//! §2.3. Its fields are FULLY typed from the target's declared annotations
//! (never `FieldType::Any` — the legacy ctx schema at
//! `functions_annotations.rs:4276-4280` is the counter-example and a C3-G7
//! deletion target). No `Box<HeapValue>`, no new `HeapKind`, no new
//! discriminator, no `KindedSlot` in the typed VM↔JIT slot ABI — the
//! aggregate flows through the ordinary function-return path. It is
//! compiler-internal: the field names `a0..aN-1` are the weave contract,
//! never user-visible (C3-G9; #63 tracks a first-class tuple surface).
//!
//! # Known validation boundaries (named, deliberate)
//!
//! - **Out-of-range constant indices are NOT construction-checkable** — arity
//!   is a property of the frozen TARGET, which construction never sees. The
//!   rewrite face rejects `args[7]` against a 2-parameter target with a named
//!   rejection quoting the index and the target's arity + signature; the
//!   specialization seam wraps it in the two-signature application-site
//!   attribution.
//! - **Interpolated f-string contents are not scanned by the pseudo-tuple
//!   faces.** A `Literal::FormattedString` carries its interpolation as raw
//!   text until emission, so a pseudo-tuple reference inside one
//!   (`f"{args[0]}"`) is NEVER rewritten — after specialization the name no
//!   longer exists, and emission resolves module bindings BEFORE fn tables,
//!   so an unrelated application-module binding spelled `args` would be
//!   silently honored (the a6 shape, inside an interpolation). The S5
//!   [`Face::AmbientScope`] face therefore DOES scan interpolation
//!   interiors: an ambient module-binding name there is a [C0926]
//!   rejection, and the template's own `args`/`Args` spellings there are
//!   rejected with the named F5-boundary sentence
//!   ([`AmbientScopeCtx::check_fstring_template_name`], positive twin:
//!   hoist the value to a local outside the f-string) — neither is ever
//!   silently honored. S6 fixlet round 4: the SPECIALIZATION faces
//!   ([`Face::Rewrite`] / [`Face::AfterReturnScan`]) additionally parse the
//!   interiors and reject the two EXIT constructs (`?` / `return`) found
//!   inside ([`Scan::scan_fstring_interior_exits`]) — an interpolation part
//!   is re-parsed and compiled IN THE ENCLOSING FRAME at emission
//!   (`string_interpolation.rs`), so an exit construct there exits the
//!   SPECIALIZED BODY, bypassing the round-2 F1 arm and both exit gates
//!   (measured pointer-bits leak). That scan is SCAN-ONLY: the re-parsed
//!   temp AST is never rewritten (a rewrite there would be silently
//!   discarded — the raw text is what emission re-parses), so the
//!   no-rewrite F5 boundary stands.
//!
//! # The `__c3_` reserved prefix
//!
//! The rewrite face mints specialization-internal names under the `__c3_`
//! prefix (`__c3_p{i}` parameters, `__c3_arg_{i}` mutable locals). The walker
//! rejects ANY identifier carrying that prefix in a template body so a minted
//! name can never collide with (or capture) user spelling — the same
//! internal-name discipline the deleted legacy wrapper applied to its
//! `__args`/`__result`/`__ctx` locals (S6-capstone deletion territory),
//! but enforced at construction instead of relied on by convention. Minted
//! nodes are inserted AFTER the walk (prologue/params) or as terminal
//! replacements the traversal never revisits, so the reserved-prefix check
//! never fires on the rewriter's own output.

use shape_ast::ast::expr_helpers::{BlockItem, QueryClause};
use shape_ast::ast::expressions::{EnumConstructorPayload, Expr, ObjectEntry};
use shape_ast::ast::functions::{Annotation, FunctionDef, FunctionParameter};
use shape_ast::ast::literals::Literal;
use shape_ast::ast::patterns::{DestructurePattern, Pattern, PatternConstructorFields};
use shape_ast::ast::program::{Assignment, OwnershipModifier, VarKind, VariableDecl};
use shape_ast::ast::span::{Span, Spanned};
use shape_ast::ast::statements::{ForInit, Statement};
use shape_ast::ast::types::{ExtendStatement, MethodDef, ObjectTypeField, TypeAnnotation};
use shape_ast::ast::windows::{WindowExpr, WindowFunction};
use shape_ast::error::{Result, ShapeError};

use super::MutationCarrier;
use super::const_lift::LiftedConst;

/// The reserved specialization-internal identifier prefix (see module docs).
pub(in crate::compiler) const RESERVED_SPECIALIZATION_PREFIX: &str = "__c3_";

/// The minted parameter name for the target's i-th typed slot (`__c3_p{i}`).
fn slot_param_name(index: usize) -> String {
    format!("{RESERVED_SPECIALIZATION_PREFIX}p{index}")
}

/// The minted mutable local backing the i-th slot (`__c3_arg_{i}`).
fn slot_local_name(index: usize) -> String {
    format!("{RESERVED_SPECIALIZATION_PREFIX}arg_{index}")
}

/// Unspellable sentinel names for the [`Face::AmbientScope`] scan: the
/// ambient face classifies FREE identifiers against the module scope and
/// never engages the pseudo-tuple surface (the template's own `args` /
/// `Args` spellings are ordinary in-scope parameters for it), so its `Scan`
/// carries names no Shape identifier can collide with.
const AMBIENT_SENTINEL_ARGS: &str = "\u{1}ambient:no-args";
const AMBIENT_SENTINEL_TYPE: &str = "\u{1}ambient:no-type";

/// ADR-009 C3 #14 (slice 5, S5a) — the [C0926] ambient-totality context: the
/// resolution environment + lexical scope stack for one template-body scan.
///
/// # The G4 boundary rule (verbatim, binding)
///
/// fn-callees at module scope are LEGIT; module-scope VALUE bindings are NOT
/// — that is the a6 hazard. A hook template's body reads only its exact
/// inputs (signature parameters + declared captures, C3-G4); everything that
/// resolves ambiently to a module-scope VALUE binding (`let` / `let mut` /
/// `var` / `const` — consts INCLUDED) is a [C0926] rejection, because the
/// body is specialized into the APPLICATION module and resolves names there.
///
/// # The motivating disaster (slice-0 §4 a6, re-measured on the new path)
///
/// S5a probe P2c (recorded in c3-slice5-report.md): an annotation in
/// `mod defs` with its own `pub const secret: int = 11` and a declarative
/// hook body reading `secret`, applied in a module carrying an unrelated
/// `let secret = 99` — the woven program SILENTLY ran with 99 (measured
/// value 1040 vs the intent 160). The target module's binding wins over the
/// annotation's own const with zero diagnostics. This gate makes the whole
/// class a named rejection.
///
/// # The asymmetry rule (invariant 7)
///
/// A module-scope CONST is comptime-evaluable and therefore LEGAL in the
/// `@application` config-arg position (the [C0931] pre-check exempts it),
/// but ILLEGAL inside a template body — G4 exact-inputs totality covers
/// consts; the a2b positive twin is "declare it as a capture".
pub(in crate::compiler) struct AmbientScopeCtx<'a> {
    compiler: &'a crate::compiler::BytecodeCompiler,
    /// The rendered origin for the [C0926] sentence: `` fn `name` `` for
    /// API bodies, `` the `before|after` hook of annotation `X` `` for
    /// sugar-minted bodies — never the SOH hygienic name.
    origin: String,
    /// The template body's defining module (from the body fn's qualified
    /// name, or the sugar annotation's key) — the a2b resolution trial: a
    /// bare name in the body that resolves under the TEMPLATE's own module
    /// is just as ambient as one resolving in the application module.
    template_module: Option<String>,
    /// The lexical scope stack: frame 0 = the body fn's declared parameters
    /// (signature + trailing captures — the exact inputs); inner frames are
    /// pushed at block/closure/arm boundaries and popped on exit, and
    /// binders register in traversal order (sequential visibility: a use
    /// BEFORE a same-spelled local `let` resolves ambiently and rejects).
    scopes: std::cell::RefCell<Vec<Vec<String>>>,
    /// The REAL pseudo-tuple spellings (`args` / `Args`) for PolymorphicArgs
    /// templates — `None` for every other template kind. The ambient face
    /// runs under unspellable sentinels so the pseudo-tuple surface stays
    /// structurally disengaged; these carry the real spellings ONLY for the
    /// f-string-interior arm ([`Self::check_fstring_template_name`]): the
    /// Rewrite face never enters interpolation interiors, so a template
    /// name there survives specialization as raw text and would resolve
    /// ambiently in the application module (fix round 1, F1).
    template_args_param: Option<String>,
    template_type_param: Option<String>,
    /// > 0 while the ambient face is scanning inside an f-string
    /// interpolation interior (incremented around each interior walk in
    /// `Scan::ambient_scan_fstring`).
    fstring_interior_depth: std::cell::Cell<usize>,
}

impl<'a> AmbientScopeCtx<'a> {
    pub(in crate::compiler) fn new(
        compiler: &'a crate::compiler::BytecodeCompiler,
        origin: String,
        template_module: Option<String>,
        pseudo_tuple_params: Option<(String, String)>,
    ) -> Self {
        let (template_args_param, template_type_param) = match pseudo_tuple_params {
            Some((args_param, type_param)) => (Some(args_param), Some(type_param)),
            None => (None, None),
        };
        Self {
            compiler,
            origin,
            template_module,
            scopes: std::cell::RefCell::new(Vec::new()),
            template_args_param,
            template_type_param,
            fstring_interior_depth: std::cell::Cell::new(0),
        }
    }

    fn push_frame(&self) {
        self.scopes.borrow_mut().push(Vec::new());
    }

    fn pop_frame(&self) {
        self.scopes.borrow_mut().pop();
    }

    fn bind(&self, name: &str) {
        if let Some(frame) = self.scopes.borrow_mut().last_mut() {
            frame.push(name.to_string());
        }
    }

    fn in_scope(&self, name: &str) -> bool {
        self.scopes
            .borrow()
            .iter()
            .any(|frame| frame.iter().any(|bound| bound == name))
    }

    /// The scoped-then-bare module-binding detector, mirroring the emission
    /// path (`expressions/mod.rs:453-459` / `identifiers.rs:350`):
    /// `resolve_scoped_module_binding_name` (bare first, then the current
    /// module-scope stack), THEN the template's own defining module (the a2b
    /// trial), THEN imported `pub const`s (which inline at use sites — a
    /// module-scope const per invariant 7).
    fn resolve_module_value_binding(&self, name: &str) -> Option<String> {
        if let Some(resolved) = self.compiler.resolve_scoped_module_binding_name(name) {
            return Some(resolved);
        }
        if let Some(module) = &self.template_module {
            let qualified = format!("{module}::{name}");
            if self.compiler.module_bindings.contains_key(&qualified) {
                return Some(qualified);
            }
        }
        if self.compiler.imported_consts.contains_key(name) {
            return Some(name.to_string());
        }
        None
    }

    /// Module-scope FN evidence for the call-name tie-break (G3: bodies call
    /// helper fns; fn-as-value is code, not invocation-scope state).
    fn is_module_fn(&self, name: &str) -> bool {
        let compiler = self.compiler;
        if compiler.function_defs.contains_key(name)
            || compiler.find_function(name).is_some()
            || compiler.stdlib_function_names.contains(name)
        {
            return true;
        }
        if let Some(module) = &self.template_module {
            let qualified = format!("{module}::{name}");
            if compiler.function_defs.contains_key(&qualified)
                || compiler.find_function(&qualified).is_some()
            {
                return true;
            }
        }
        false
    }

    /// The F5-boundary arm (fix round 1, F1): inside an f-string
    /// interpolation interior, the template's own pseudo-tuple spellings
    /// (`args` / `Args`) reject. The Validate/Rewrite faces never enter
    /// interpolation interiors (the module-docs named boundary), so a
    /// template name there is NEVER specialized away — it would survive
    /// into the woven module as raw text and resolve with
    /// module-bindings-before-fn-tables precedence: the a6 silent-capture
    /// shape, one interpolation deep. Fires unconditionally on the spelling
    /// (totality does not depend on whether a shadow binding happens to be
    /// registered yet). Uncoded named sentence — no new C09xx mint.
    fn check_fstring_template_name(&self, name: &str) -> Result<()> {
        if self.fstring_interior_depth.get() == 0 {
            return Ok(());
        }
        if self.template_args_param.as_deref() == Some(name)
            || self.template_type_param.as_deref() == Some(name)
        {
            return Err(self.fstring_template_name_rejection(name));
        }
        Ok(())
    }

    /// The named F5-boundary sentence (hoist-to-a-local positive twin) for
    /// [`Self::check_fstring_template_name`]. `{origin}` never renders an
    /// SOH hygienic name.
    fn fstring_template_name_rejection(&self, ident: &str) -> ShapeError {
        let args = self.template_args_param.as_deref().unwrap_or("args");
        reject(format!(
            "hook template body {origin} references the template name `{ident}` inside an \
             f-string interpolation; interpolation interiors are raw text to the pseudo-tuple \
             specialization (the named non-scanned boundary), so `{ident}` is never resolved \
             there and would instead resolve ambiently in the application module — hoist the \
             value to a local outside the f-string (for example `let v = {args}[0]`) and \
             interpolate the local (`f\"{{v}}\"`)",
            origin = self.origin
        ))
    }

    /// THE ONE [C0926] sentence producer (S5a; exact sentence, pinned
    /// verbatim). `{origin}` never renders an SOH hygienic name.
    fn ambient_rejection(&self, ident: &str, resolved: &str) -> ShapeError {
        reject(format!(
            "[C0926] hook template body {origin} references `{ident}`, which resolves to the \
             module-scope value binding `{resolved}`; a hook template's body reads only its \
             exact inputs — its signature parameters and its declared captures (C3-G4); module- \
             and invocation-scope values never enter a template ambiently, because the body is \
             specialized into the application module where an unrelated binding spelled \
             `{ident}` silently takes over the hook's behavior — declare it as an input \
             instead: add a typed config parameter (or capture(\"{ident}\", ...)) and reference \
             the capture parameter",
            origin = self.origin
        ))
    }
}

/// ADR-009 C3 #14 (slice 5, S5a) — the [C0926] ambient-scope scan: walk one
/// template body fn with the AMBIENT face and reject the first free
/// identifier that resolves to a module-scope VALUE binding. Frame 0 seeds
/// the body fn's declared parameters (signature + trailing captures — the
/// C3-G4 exact inputs). The walk never mutates; `&mut` is the walker's
/// traversal-uniformity contract.
pub(in crate::compiler) fn scan_template_body_ambient_scope(
    def: &mut FunctionDef,
    ctx: &AmbientScopeCtx<'_>,
) -> Result<()> {
    ctx.push_frame();
    for param in &def.params {
        for (name, _) in param.pattern.get_bindings() {
            ctx.bind(&name);
        }
    }
    let scan = Scan {
        args_param: AMBIENT_SENTINEL_ARGS,
        type_param: AMBIENT_SENTINEL_TYPE,
        comptime_depth: std::cell::Cell::new(0),
        face: Face::AmbientScope(ctx),
    };
    let result = walk_template_body(&scan, &mut def.body);
    ctx.pop_frame();
    result
}

/// Everything the monomorphization ride needs to specialize one template
/// body against one frozen target (C3-G9/G10 + the S3b ConstLift bake).
/// Built at the specialization seam
/// (`template_specialization::specialize_template`) and consumed by
/// `cache.rs::ensure_monomorphic_template_specialization` — the plan is an
/// explicit parameter end to end; no ambient state.
///
/// ADR-009 C3 #14 (slice 3, S3b — Dec-95 rule 6): capture VALUES are IN the
/// plan and IN the specialization identity. The S2 "NAMES only — values
/// never in the plan or the cache key (invariants resolution 5)" posture is
/// SUPERSEDED by this slice's charter: structurally EQUAL config shares a
/// specialization; structurally DIFFERENT config gets a distinct one (the
/// `::cfg#` segment of `template_specialization_key_suffix`), and the values
/// BAKE into the specialized handler as constants
/// (`const_lift::bake_captures_into_def`) — no call-site delivery exists.
#[derive(Debug)]
pub(in crate::compiler) struct TemplateSpecializationPlan {
    /// The G9 pseudo-tuple resolution input — `Some` for PolymorphicArgs
    /// templates ONLY; `None` for the concrete / observer /
    /// PolymorphicResult with-capture routes (which need the bake + the
    /// rule-6 identity but no pseudo-tuple rewrite).
    pub(in crate::compiler) pseudo_tuple: Option<PseudoTuplePlan>,
    /// The template's capture values `(name, value)` in delivery
    /// (trailing-parameter) order — the ONE ordering shared by the `::cfg#`
    /// identity segment and the bake's prologue
    /// (`const_lift::capture_values_in_delivery_order`).
    pub(in crate::compiler) captures: Vec<(String, LiftedConst)>,
}

/// The G9 pseudo-tuple face of a [`TemplateSpecializationPlan`]: everything
/// the rewrite face needs to resolve one polymorphic BEFORE template body
/// against one frozen target.
#[derive(Debug)]
pub(in crate::compiler) struct PseudoTuplePlan {
    /// The template's pseudo-tuple parameter spelling (`args`).
    pub(in crate::compiler) args_param: String,
    /// The template's type parameter spelling (`Args`).
    pub(in crate::compiler) type_param: String,
    /// The frozen target's parameters: display name + AST-side declared
    /// annotation, in signature order (slice-0 §7.4 — AST side, never the
    /// freeze round-trip).
    pub(in crate::compiler) target_params: Vec<(String, TypeAnnotation)>,
    /// How the mutated pack flows back (C3-G9): `Single` for a 1-ary target,
    /// the compiler-internal `Aggregate` otherwise.
    pub(in crate::compiler) carrier: MutationCarrier,
}

impl PseudoTuplePlan {
    fn arity(&self) -> usize {
        self.target_params.len()
    }

    /// The expression a mutation-return specializes to: the bare typed local
    /// for `Single`, the inline-schema typed object literal for `Aggregate`
    /// (see the module-docs ADR-006 compliance statement).
    fn carrier_expr(&self, span: Span) -> Expr {
        match &self.carrier {
            MutationCarrier::Single { .. } => Expr::Identifier(slot_local_name(0), span),
            MutationCarrier::Aggregate { fields } => Expr::Object(
                fields
                    .iter()
                    .enumerate()
                    .map(|(index, (name, _))| ObjectEntry::Field {
                        key: name.clone(),
                        value: Expr::Identifier(slot_local_name(index), span),
                        type_annotation: None,
                    })
                    .collect(),
                span,
            ),
        }
    }

    /// The specialized return annotation: the target's one parameter type for
    /// `Single`, the fully-typed inline object schema for `Aggregate`. The
    /// transient post-substitution `Tuple` annotation is REPLACED here — it
    /// never reaches checking or emission (C3-G9).
    fn carrier_return_annotation(&self) -> TypeAnnotation {
        match &self.carrier {
            MutationCarrier::Single { annotation } => annotation.clone(),
            MutationCarrier::Aggregate { fields } => TypeAnnotation::Object(
                fields
                    .iter()
                    .map(|(name, annotation)| ObjectTypeField {
                        name: name.clone(),
                        optional: false,
                        type_annotation: annotation.clone(),
                        annotations: Vec::new(),
                    })
                    .collect(),
            ),
        }
    }

    /// Render the target's parameter list for the out-of-range rejection
    /// (`a: int, b: number`).
    fn render_target_params(&self) -> String {
        self.target_params
            .iter()
            .map(|(name, annotation)| format!("{name}: {}", annotation.to_type_string()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Validate every use of the pseudo-tuple parameter (`args_param`) and the
/// template type parameter (`type_param`) in a polymorphic BEFORE template
/// body.
///
/// Legal uses of `args_param`, exactly:
///
/// - `args[<int literal>]` in read position (`IndexAccess` with a
///   `Literal::Int` index and no slice end),
/// - the same shape as an assignment target (`args[<int literal>] = expr`),
/// - `args.length` (plain, non-optional property access),
/// - `return args` (statement form, and the parser's expression-position
///   `return` twin — `Expr::Return` — which is the same authored spelling
///   built by the block-item return path in the parser),
/// - a FINAL bare `args` expression statement at the top level of the body
///   (the implicit-return tail).
///
/// Everything else involving `args_param` or `type_param` is a NAMED
/// rejection with a positive twin (see the `reject_*` constructors); the
/// walker also rejects rebinding either name (a shadowed pseudo-tuple could
/// silently change what the rewrite face targets) and any identifier with the
/// reserved `__c3_` prefix.
///
/// Called from `CheckedTemplateBuilder::finish()` for
/// `TemplateSig::PolymorphicArgs` only — concrete bodies and polymorphic
/// AFTER bodies (`result`) carry ordinary values with no pseudo-tuple
/// surface. The `&mut` is traversal-uniformity with the rewrite face only;
/// the validate face never mutates.
pub(in crate::compiler) fn validate_pseudo_tuple_uses(
    body: &mut [Statement],
    args_param: &str,
    type_param: &str,
) -> Result<()> {
    let scan = Scan {
        args_param,
        type_param,
        comptime_depth: std::cell::Cell::new(0),
        face: Face::Validate,
    };
    walk_template_body(&scan, body)
}

/// The rewrite face (C3-G9 resolution): resolve every pseudo-tuple construct
/// in an already-substituted specialized template def against the plan's
/// frozen target. Transform, in order:
///
/// 1. Walk the body with [`Face::Rewrite`]: constant-index `args[i]` (read
///    and assignment-target) → the minted local `__c3_arg_{i}` (a constant
///    index outside `[0, N)` is a NAMED rejection quoting the index and the
///    target's arity + signature); `args.length` → the constant `N`;
///    `return args` / the final bare-`args` tail → the carrier expression.
///    Any residual `args`/`type_param` use named-rejects via the SAME shared
///    core the construction face runs. After the walk, TWO specialization
///    guards run over the collected events: the S2c aggregate-kind guard
///    ([`guard_carrier_write_kinds`] — every per-slot write proves its
///    declared parameter type) and the S6-round-3 F3 before-side exit gate
///    ([`guard_before_template_exit_kinds`] — every value-producing exit
///    proves the bound carrier type; value-less completion rejects).
/// 2. Replace the single `args` parameter with N parameters `__c3_p{i}`
///    typed with the target's AST-side annotations.
/// 3. Prepend the prologue `let mut __c3_arg_{i} = __c3_p{i}` per parameter
///    (uniform mutability without assuming parameter assignability).
/// 4. Replace the (substituted-to-`Tuple`) return annotation with the
///    carrier annotation — the transient `Tuple` never reaches checking or
///    emission.
///
/// The walk runs BEFORE the prologue/parameter minting so the reserved-prefix
/// check never sees the rewriter's own output.
pub(in crate::compiler) fn resolve_pseudo_tuple(
    def: &mut FunctionDef,
    plan: &TemplateSpecializationPlan,
    compiler: &crate::compiler::BytecodeCompiler,
) -> Result<()> {
    let internal = |message: String| ShapeError::RuntimeError {
        message,
        location: None,
    };
    // S3b: the pseudo-tuple face is `Some` for PolymorphicArgs plans only;
    // the monomorphization ride guards the call on that — a `None` here is
    // an internal invariant breach, never a route.
    let Some(pt) = &plan.pseudo_tuple else {
        return Err(internal(format!(
            "internal error: resolve_pseudo_tuple for `{}` received a plan without a \
             pseudo-tuple face; the monomorphization ride calls the resolution only for \
             PolymorphicArgs plans",
            def.name
        )));
    };
    let arity = pt.arity();
    if arity == 0 {
        return Err(internal(format!(
            "internal error: resolve_pseudo_tuple for `{}` received a zero-parameter target \
             plan; the specialization seam rejects zero-parameter before-targets before \
             building a plan",
            def.name
        )));
    }
    let carrier_consistent = match &pt.carrier {
        MutationCarrier::Single { .. } => arity == 1,
        MutationCarrier::Aggregate { fields } => arity > 1 && fields.len() == arity,
    };
    if !carrier_consistent {
        return Err(internal(format!(
            "internal error: resolve_pseudo_tuple for `{}` received an inconsistent plan \
             (arity {arity} vs carrier {:?}); the specialization seam derives the carrier \
             from the same target parameter list",
            def.name, pt.carrier
        )));
    }
    // The substituted def's parameter shape is chokepoint-guaranteed:
    // `[args_param, <capture params>...]` — the pseudo-tuple parameter first,
    // then the concretely-annotated trailing capture parameters in the
    // plan's (delivery) order (zero captures = the original
    // exactly-one-parameter shape). The tail is PRESERVED here; the S3b bake
    // (`const_lift::bake_captures_into_def`, run by the ride immediately
    // after this resolution) strips it and bakes the values as prologue
    // constants.
    let expected_params = 1 + plan.captures.len();
    if def.params.len() != expected_params
        || def.params[0].simple_name() != Some(pt.args_param.as_str())
    {
        return Err(internal(format!(
            "internal error: resolve_pseudo_tuple expected `{}` to carry the pseudo-tuple \
             parameter `{}` plus {} trailing capture parameter(s); a PolymorphicArgs template \
             classifies with exactly that shape at construction",
            def.name,
            pt.args_param,
            plan.captures.len(),
        )));
    }
    for (offset, (expected_name, _)) in plan.captures.iter().enumerate() {
        if def.params[1 + offset].simple_name() != Some(expected_name.as_str()) {
            return Err(internal(format!(
                "internal error: resolve_pseudo_tuple expected `{}`'s trailing capture \
                 parameter at position {} to be `{expected_name}`; the construction-time \
                 capture bijection guarantees the tail order",
                def.name,
                offset + 2,
            )));
        }
    }
    let capture_tail: Vec<FunctionParameter> = def.params.split_off(1);

    // (1) The rewrite walk — same core, second face. The walk COLLECTS every
    // per-slot mutation's rewritten RHS for the aggregate-kind guard below,
    // the S6 provable-locals events (binding-position names with their
    // post-rewrite initializers, and every whole-name assignment target),
    // and — S6 round 3, F3 — every body-level non-conforming return value
    // for the before-side exit gate (the conforming `return args` spellings
    // are intercepted and rewritten before collection).
    let carrier_writes = std::cell::RefCell::new(Vec::new());
    let local_bindings = std::cell::RefCell::new(Vec::new());
    let assigned_locals = std::cell::RefCell::new(std::collections::HashSet::new());
    let body_returns = std::cell::RefCell::new(Vec::new());
    let scan = Scan {
        args_param: &pt.args_param,
        type_param: &pt.type_param,
        comptime_depth: std::cell::Cell::new(0),
        face: Face::Rewrite {
            plan: pt,
            carrier_writes: &carrier_writes,
            local_bindings: &local_bindings,
            assigned_locals: &assigned_locals,
            returns: &body_returns,
        },
    };
    walk_template_body(&scan, &mut def.body)?;
    let carrier_writes = carrier_writes.into_inner();
    let local_bindings = local_bindings.into_inner();
    let assigned_locals = assigned_locals.into_inner();
    let body_returns = body_returns.into_inner();

    // (1b) ADR-009 C3 #14 (slice 2, S2c) — the aggregate-kind guard, closing
    // the S1 pending observation (c3-slice1-report.md §Observations: a
    // carrier-local re-assignment re-stamps the slot kind at emission, so an
    // aggregate field's runtime kind could silently diverge from its declared
    // carrier annotation — and the weave's typed `.a{i}` read would then
    // misinterpret raw bits). Every collected `args[i] = expr` write must
    // PROVE the declared parameter type; a proven-divergent or unprovable
    // write is a NAMED rejection here (surface-and-stop, CLAUDE.md "if the
    // type can't be proven, it is a compile error"), which the
    // monomorphization ride classifies Hard and the seam wraps through
    // `template_application_error` with both signatures at the
    // `@application` site. Runs at specialization BEFORE the handler
    // compiles: there is no post-compile record of proven per-field kinds
    // (return `ConcreteType`s register from DECLARED annotations only), so
    // the guard proves the writes at the only point where the declared
    // types and the write expressions are both in hand.
    guard_carrier_write_kinds(
        compiler,
        pt,
        &capture_tail,
        &carrier_writes,
        &local_bindings,
        &assigned_locals,
    )?;

    // (1c) ADR-009 C3 #14 (S6 fixlet round 3, F3) — the BEFORE-side exit
    // gate: every value-producing exit of the (post-rewrite) body must PROVE
    // the bound before-protocol type — the carrier type the weave's typed
    // read consumes (see the gate's docs for the weave-consumption quote).
    // Runs AFTER the write guard so the established per-slot write
    // rejections keep sentence precedence, and BEFORE the parameter/prologue
    // minting so the gated body is exactly the walked one. The
    // monomorphization ride classifies a rejection Hard and the seam wraps
    // it through `template_application_error` at the `@application` site,
    // exactly like the write guard.
    guard_before_template_exit_kinds(
        compiler,
        pt,
        &capture_tail,
        &def.body,
        &body_returns,
        &local_bindings,
        &assigned_locals,
    )?;

    // (2) Per-slot minted parameters, typed from the target's AST side; the
    // trailing capture parameters (already concrete — never substituted)
    // are PRESERVED verbatim after them for the S3b bake, which strips the
    // tail and bakes the values as prologue constants immediately after
    // this resolution (the ride's bake seam).
    def.params = pt
        .target_params
        .iter()
        .enumerate()
        .map(|(index, (_, annotation))| FunctionParameter {
            pattern: DestructurePattern::Identifier(slot_param_name(index), Span::default()),
            is_const: false,
            is_reference: false,
            is_mut_reference: false,
            is_out: false,
            type_annotation: Some(annotation.clone()),
            default_value: None,
        })
        .collect();
    def.params.extend(capture_tail);

    // (3) The mutable-local prologue.
    let mut body = Vec::with_capacity(arity + def.body.len());
    for index in 0..arity {
        body.push(Statement::VariableDecl(
            VariableDecl {
                kind: VarKind::Let,
                is_mut: true,
                pattern: DestructurePattern::Identifier(slot_local_name(index), Span::default()),
                type_annotation: None,
                value: Some(Expr::Identifier(slot_param_name(index), Span::default())),
                ownership: OwnershipModifier::Inferred,
            },
            Span::default(),
        ));
    }
    body.append(&mut def.body);
    def.body = body;

    // (4) The carrier return annotation replaces the transient Tuple. The
    // production caller hands in a substitution output whose `type_params`
    // are already cleared; clear defensively so the resolved def is concrete
    // either way (the polymorphism has resolved away).
    def.return_type = Some(pt.carrier_return_annotation());
    def.type_params = None;
    Ok(())
}

/// The shared proof environment of the two specialization guards (the S2c
/// write guard and the S6-round-3 F3 exit gate): the declared per-slot
/// `ConcreteType`s in target-parameter order, plus the declared trailing
/// captures (name → concrete; an unresolvable capture annotation simply
/// stays out of the env — the affected expression then proves `None` and
/// the consuming guard's named rejection stands, never a guess).
///
/// The specialization seam (`specialize_polymorphic_before`) already proved
/// every target parameter resolves concretely before building the plan, so
/// a slot miss here is internal-error-shaped, never a user rejection.
fn resolve_carrier_proof_env(
    compiler: &crate::compiler::BytecodeCompiler,
    plan: &PseudoTuplePlan,
    capture_tail: &[FunctionParameter],
) -> Result<(
    Vec<shape_value::v2::ConcreteType>,
    Vec<(String, shape_value::v2::ConcreteType)>,
)> {
    use crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type;
    use shape_value::v2::ConcreteType;

    let mut declared_slots: Vec<ConcreteType> = Vec::with_capacity(plan.target_params.len());
    for (param_name, annotation) in &plan.target_params {
        let Some(concrete) = declared_annotation_concrete_type(compiler, annotation) else {
            return Err(reject(format!(
                "internal error: the specialization guards could not resolve the declared type \
                 `{}` of target parameter `{param_name}`; the specialization seam proves every \
                 parameter resolves concretely before building the plan",
                annotation.to_type_string()
            )));
        };
        declared_slots.push(concrete);
    }
    let captures: Vec<(String, ConcreteType)> = capture_tail
        .iter()
        .filter_map(|param| {
            let name = param.simple_name()?.to_string();
            let concrete = param
                .type_annotation
                .as_ref()
                .and_then(|annotation| declared_annotation_concrete_type(compiler, annotation))?;
            Some((name, concrete))
        })
        .collect();
    Ok((declared_slots, captures))
}

/// ADR-009 C3 #14 (slice 2, S2c) — the aggregate-kind guard (see the call
/// site in [`resolve_pseudo_tuple`] for the full rationale). Every collected
/// per-slot write must PROVE the declared parameter type via
/// [`carrier_write_concrete_type`]; a proven-divergent write and an
/// unprovable write are both NAMED rejections with positive twins
/// (surface-and-stop — never a silently kind-divergent typed read in the
/// weave). Comparison is `ConcreteType` equality (never spelling equality —
/// the S1 same-Sig cache-hit carrier-spelling observation), so alias
/// spellings of one concrete type (`Array<int>` / `Vec<int>`) agree.
fn guard_carrier_write_kinds(
    compiler: &crate::compiler::BytecodeCompiler,
    plan: &PseudoTuplePlan,
    capture_tail: &[FunctionParameter],
    writes: &[(usize, Expr)],
    local_bindings: &[(String, Option<Expr>)],
    assigned_locals: &std::collections::HashSet<String>,
) -> Result<()> {
    if writes.is_empty() {
        return Ok(());
    }
    let (declared_slots, captures) = resolve_carrier_proof_env(compiler, plan, capture_tail)?;

    // S6 (supervisor-ordered guard growth): resolve the provable-initializer
    // locals BEFORE the write loop, so a hoisted local (`let t = args[0]`)
    // joins the provable write-RHS set — transitively, at its binding.
    let locals = resolve_provable_locals(
        compiler,
        &declared_slots,
        &captures,
        local_bindings,
        assigned_locals,
    );

    for (slot, expr) in writes {
        let (param_name, annotation) = &plan.target_params[*slot];
        let required = &declared_slots[*slot];
        match carrier_write_concrete_type(compiler, &declared_slots, &captures, &locals, expr) {
            Some(proven) if &proven == required => {}
            Some(proven) => {
                return Err(reject(format!(
                    "the value assigned to `{args}[{slot}]` proves type `{proven}` but the \
                     target's parameter `{param_name}` declares `{ann}` — a kind-divergent \
                     write would make the typed mutation carrier's field a{slot} lie about \
                     its runtime kind at the weave's typed read; assign a `{ann}` value (or \
                     change the target parameter's declared type)",
                    args = plan.args_param,
                    ann = annotation.to_type_string(),
                )));
            }
            None => {
                return Err(reject(format!(
                    "cannot prove the type of the value assigned to `{args}[{slot}]`: the \
                     typed mutation carrier requires every per-slot write to prove the \
                     declared parameter type `{ann}` (parameter `{param_name}`) — assign an \
                     expression whose type is provable at specialization (a literal, another \
                     `{args}[i]` slot, a declared capture, a local bound to a provable \
                     initializer, a typed fn's result, or arithmetic over those)",
                    args = plan.args_param,
                    ann = annotation.to_type_string(),
                )));
            }
        }
    }
    Ok(())
}

/// ADR-009 C3 #14 (S6 soundness fixlet, supervisor-ordered) — the
/// PROVABLE-INITIALIZER LOCALS resolution: a local whose initializer
/// expression is provable under the existing guard rules joins the provable
/// set at its binding, transitively (`let t = args[0]; let u = t + 1`).
///
/// The map is FLAT (name → proof) with STICKY POISONING, which is what makes
/// final-state evaluation sound against scoping and loop re-execution:
///
/// - a name EVER assigned after binding (`assigned_locals`) is poisoned
///   outright — a textual walk cannot bound re-execution order under loops,
///   so a mutated local never joins the set;
/// - a name bound more than once stays provable ONLY if every binding proves
///   the SAME type (a conflicting or unprovable rebind poisons the name for
///   the WHOLE body — earlier-collected writes included, since the write
///   loop evaluates against this final map);
/// - a name colliding with a declared capture parameter is poisoned (the
///   flat map cannot tell a pre-shadow capture read from a post-shadow local
///   read, so neither meaning is blessed);
/// - every non-`let name = expr` binding construct arrives as an
///   unprovable-by-construction `None` event (see
///   [`Scan::note_binding_event`]).
///
/// Bindings are processed in textual walk order, so an initializer sees
/// exactly the names bound before it (locals never hoist); `Some(T)` in the
/// FINAL map therefore means every runtime value any same-spelled binding
/// ever holds has type `T` — the invariant the write loop consumes.
/// Poisoned names fall back to the PRE-S6 resolution for an unknown
/// identifier (per-occurrence span-keyed inference facts, then the module
/// resolvers — see the evaluator's Identifier arm), never to the capture
/// env: the provable set only ever GROWS relative to the pre-S6 guard, and
/// an unprovable name still rejects with the existing named sentence.
fn resolve_provable_locals(
    compiler: &crate::compiler::BytecodeCompiler,
    declared_slots: &[shape_value::v2::ConcreteType],
    captures: &[(String, shape_value::v2::ConcreteType)],
    local_bindings: &[(String, Option<Expr>)],
    assigned_locals: &std::collections::HashSet<String>,
) -> std::collections::HashMap<String, Option<shape_value::v2::ConcreteType>> {
    let mut locals: std::collections::HashMap<String, Option<shape_value::v2::ConcreteType>> =
        std::collections::HashMap::new();
    for (name, init) in local_bindings {
        let proof = if assigned_locals.contains(name)
            || captures.iter().any(|(capture, _)| capture == name)
        {
            None
        } else {
            init.as_ref().and_then(|expr| {
                carrier_write_concrete_type(compiler, declared_slots, captures, &locals, expr)
            })
        };
        match locals.entry(name.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(proof);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let keep = matches!((entry.get(), &proof), (Some(existing), Some(new)) if existing == new);
                if !keep {
                    entry.insert(None);
                }
            }
        }
    }
    locals
}

/// One value-producing exit of a polymorphic `after` template body: either a
/// concrete expression the guard must prove, or a provably value-less
/// completion path (a bare `return`, a missing else branch or non-value
/// final statement in tail position).
enum ResultLeaf {
    Value(Expr),
    NoValue,
}

/// Decompose one return-position expression into its VALUE LEAVES: `if` /
/// `match` / block tails recurse into their branch results (so a
/// conditional body like `if c { capture_val } else { result }` proves per
/// branch); everything else IS a leaf. A missing else branch in a
/// value-producing position is a [`ResultLeaf::NoValue`] completion. A
/// nested `return` expression contributes nothing here — the walker
/// collected its value as its own return-position event.
fn collect_result_value_leaves(expr: &Expr, leaves: &mut Vec<ResultLeaf>) {
    match expr {
        Expr::If(if_expr, _) => {
            collect_result_value_leaves(&if_expr.then_branch, leaves);
            match &if_expr.else_branch {
                Some(else_branch) => collect_result_value_leaves(else_branch, leaves),
                None => leaves.push(ResultLeaf::NoValue),
            }
        }
        // The parser's second conditional spelling (`if c { a } else { b }`
        // in expression position parses as `Expr::Conditional`).
        Expr::Conditional {
            then_expr,
            else_expr,
            ..
        } => {
            collect_result_value_leaves(then_expr, leaves);
            match else_expr {
                Some(else_expr) => collect_result_value_leaves(else_expr, leaves),
                None => leaves.push(ResultLeaf::NoValue),
            }
        }
        Expr::Match(match_expr, _) => {
            for arm in &match_expr.arms {
                collect_result_value_leaves(&arm.body, leaves);
            }
        }
        Expr::Block(block, _) => match block.items.last() {
            Some(BlockItem::Expression(inner)) => collect_result_value_leaves(inner, leaves),
            Some(BlockItem::Statement(Statement::Return(_, _))) => {}
            Some(BlockItem::Statement(stmt)) => {
                collect_tail_leaves_of_statements(std::slice::from_ref(stmt), leaves)
            }
            Some(BlockItem::VariableDecl(_)) | Some(BlockItem::Assignment(_)) | None => {
                leaves.push(ResultLeaf::NoValue)
            }
        },
        Expr::Return(_, _) => {}
        other => leaves.push(ResultLeaf::Value(other.clone())),
    }
}

/// The statement-tail twin of [`collect_result_value_leaves`]: the FINAL
/// statement of a statement list is the implicit-return position. An
/// expression statement recurses into its value leaves; a `return` statement
/// contributes nothing (collected by the walker); a tail `if` STATEMENT
/// recurses per branch (a missing else = a value-less fall-through);
/// anything else (loops, declarations, assignments, an empty body) is a
/// value-less completion.
fn collect_tail_leaves_of_statements(body: &[Statement], leaves: &mut Vec<ResultLeaf>) {
    match body.last() {
        Some(Statement::Expression(expr, _)) => collect_result_value_leaves(expr, leaves),
        Some(Statement::Return(_, _)) => {}
        Some(Statement::If(if_stmt, _)) => {
            collect_tail_leaves_of_statements(&if_stmt.then_body, leaves);
            match &if_stmt.else_body {
                Some(else_body) => collect_tail_leaves_of_statements(else_body, leaves),
                None => leaves.push(ResultLeaf::NoValue),
            }
        }
        _ => leaves.push(ResultLeaf::NoValue),
    }
}

/// ADR-009 C3 #14 (S6 soundness fixlet, supervisor-ordered) — the AFTER-side
/// return-kind gate: the analog of the S2c aggregate-kind guard for
/// polymorphic `after` templates. The G4 bound is `(R) -> R`, but before
/// this gate nothing ever checked the body's ACTUAL return type against the
/// bound `R` — a type-changing body (`after(result) { f"{prefix}: {result}"
/// }` on an int target) specialized, wove, and ran, printing the string's
/// HEAP POINTER as the int result (the measured S6 A-phase leak; the exact
/// heap-pointer-reinterpretation class the strict-typing program exists to
/// kill).
///
/// Fires at the specialization seam (`specialize_polymorphic_after`, BEFORE
/// the monomorphization ride) for both the zero-capture and with-capture
/// routes, sugar and API paths alike. Every value-producing exit of the body
/// — the implicit-return tail plus every `return` (statement and expression
/// forms), with `if`/`match`/block tails decomposed per branch — must PROVE
/// the bound result type under the same bounded evaluator the S2c guard
/// runs ([`carrier_write_concrete_type`]: the `result` parameter at `R`, the
/// declared captures, the S6 provable-initializer locals, literals,
/// arithmetic, typed calls). Proven-divergent and unprovable exits are both
/// NAMED rejections (surface-and-stop — CLAUDE.md "if the type can't be
/// proven, it is a compile error"), which the seam wraps through
/// `template_application_error` with both signatures at the `@application`
/// site — the established two-signature attribution.
///
/// S6 fixlet round 2: the walk itself carries the F1 `?`-exit rejection
/// (`?` early-returns the propagated failure carrier from the specialized
/// body — never provable against the bound `R`; see
/// [`Scan::reject_try_operator_exit`] for the measured leak), and the
/// return collection is BODY-LEVEL only (closure-interior / comptime-block
/// returns never reach the specialized body's exit — the F2 filter at
/// [`Scan::note_return_value`]).
///
/// KNOWN BOUNDARY (named, deliberate): the gate proves the type of every
/// value-producing exit it can see; path-completeness (a body that can fall
/// through with no value — bare `return`, missing else in tail position, a
/// non-value final statement) rejects via [`ResultLeaf::NoValue`], but
/// exhaustive control-flow reachability (e.g. `loop` breaks) stays with the
/// downstream compile battery, exactly like the before-side guard.
pub(in crate::compiler) fn guard_after_template_return_kind(
    compiler: &crate::compiler::BytecodeCompiler,
    def: &FunctionDef,
    result_param: &str,
    required: &shape_value::v2::ConcreteType,
    required_spelling: &str,
    capture_params: &[(String, TypeAnnotation)],
) -> Result<()> {
    use crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type;

    // The proof environment: the result parameter at the bound `R`, plus the
    // declared trailing captures (concretely annotated at construction; an
    // unresolvable annotation simply stays out of the env — the affected
    // exit then rejects as unprovable, never a guess).
    let mut captures: Vec<(String, shape_value::v2::ConcreteType)> =
        vec![(result_param.to_string(), required.clone())];
    for (name, annotation) in capture_params {
        if let Some(concrete) = declared_annotation_concrete_type(compiler, annotation) {
            captures.push((name.clone(), concrete));
        }
    }

    // Collection walk — the ONE walker, fourth face, under the unspellable
    // sentinel names (an `after` body has no pseudo-tuple surface; the walk
    // never mutates and never rejects on its own).
    let local_bindings = std::cell::RefCell::new(Vec::new());
    let assigned_locals = std::cell::RefCell::new(std::collections::HashSet::new());
    let returns = std::cell::RefCell::new(Vec::new());
    let mut body = def.body.clone();
    let scan = Scan {
        args_param: AMBIENT_SENTINEL_ARGS,
        type_param: AMBIENT_SENTINEL_TYPE,
        comptime_depth: std::cell::Cell::new(0),
        face: Face::AfterReturnScan {
            local_bindings: &local_bindings,
            assigned_locals: &assigned_locals,
            returns: &returns,
        },
    };
    scan.statements(&mut body, ScanMode::TemplateBody)?;

    let locals = resolve_provable_locals(
        compiler,
        &[],
        &captures,
        &local_bindings.into_inner(),
        &assigned_locals.into_inner(),
    );

    let mut leaves = Vec::new();
    collect_tail_leaves_of_statements(&body, &mut leaves);
    for return_value in returns.into_inner() {
        match return_value {
            Some(expr) => collect_result_value_leaves(&expr, &mut leaves),
            None => leaves.push(ResultLeaf::NoValue),
        }
    }

    for leaf in leaves {
        match leaf {
            ResultLeaf::NoValue => {
                return Err(reject(format!(
                    "the template body can complete without returning a value, but the \
                     required specialization returns `{required_spelling}` (an `after` \
                     template returns the typed result); end the body with the result value, \
                     or `return` one on every path"
                )));
            }
            ResultLeaf::Value(expr) => {
                match carrier_write_concrete_type(compiler, &[], &captures, &locals, &expr) {
                    Some(proven) if &proven == required => {}
                    Some(proven) => {
                        return Err(reject(format!(
                            "the template body returns `{proven}` but the required \
                             specialization returns `{required_spelling}` (the target's \
                             declared result type) — a kind-divergent result would make the \
                             woven wrapper reinterpret the hook's raw return bits as \
                             `{required_spelling}` at its typed read; return a \
                             `{required_spelling}` value (or change the target's declared \
                             return type)"
                        )));
                    }
                    None => {
                        return Err(reject(format!(
                            "cannot prove the type of a value the template body returns: the \
                             typed result requires every return-position expression to prove \
                             the target's declared result type `{required_spelling}` — return \
                             an expression whose type is provable at specialization (the \
                             typed `{result_param}` parameter, a literal, a declared capture, \
                             a local bound to a provable initializer, a typed fn's result, or \
                             arithmetic over those)"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// ADR-009 C3 #14 (S6 fixlet round 3, F3 — supervisor-ordered) — the
/// BEFORE-side exit gate: the analog of the after-side return-kind gate for
/// polymorphic `before` templates, at the same specialization seam
/// (`resolve_pseudo_tuple`, both the sugar and API routes — every
/// PolymorphicArgs specialization passes here).
///
/// THE BOUND BEFORE-PROTOCOL TYPE (determined from the weave's actual
/// carrier consumption — `weave.rs`, the before chain): the weave binds a
/// non-observer `before` handler's call result to a local ANNOTATED with the
/// carrier type and reads it back TYPED —
/// [`MutationCarrier::Single`]: `let __c3_w{n}: <the target's one parameter
/// type> = handler(args...)`, rebound as the next call's one argument;
/// [`MutationCarrier::Aggregate`]: `let __c3_w{n}: { a0: T0, ..., aN-1:
/// TN-1 } = handler(args...)` (the CURRENT target's AST-side parameter
/// spellings), then per-field `m.a0 .. m.aN-1` typed reads feed the next
/// call. The bound type at EVERY value-producing exit is therefore exactly
/// the carrier's return annotation
/// ([`PseudoTuplePlan::carrier_return_annotation`]): the one declared
/// parameter type for `Single`; the compiler-internal per-target aggregate
/// for `Aggregate` (C3-G4's typed args-mutation contract, G9-resolved).
/// Before this gate NOTHING checked the body's actual exits against that
/// bound — the S2c guard covers per-slot `args[i] = expr` writes only, the
/// sugar carries bodies verbatim with no auto-appended return, and a stray
/// value / value-less completion reached the woven local's typed read
/// unchecked (the round-1-lens F3 finding; the same reinterpretation class
/// as the measured after-side leak).
///
/// WHAT PROVES: both conforming mutation-return spellings (`return args`,
/// the final bare-`args` tail) are rewritten to the carrier expression at
/// the walk and are conforming BY CONSTRUCTION (minted slot locals typed
/// by the prologue + the S2c write guard) — but only the `return args`
/// spelling is intercepted BEFORE the return collection and yields zero
/// leaves; the rewritten bare-`args` TAIL IS collected as an ordinary tail
/// leaf and PROVES here (the minted carrier expression passes the
/// per-field/`Single` arms below). Every OTHER value-producing exit
/// (a stray value tail, a non-`args` `return`) must PROVE the carrier type
/// under the same bounded evaluator the S2c guard runs
/// ([`carrier_write_concrete_type`]: minted slot locals, declared captures,
/// provable-initializer locals, literals, arithmetic, typed calls) — for
/// `Single`, `ConcreteType` equality with the one declared parameter type;
/// for `Aggregate` (which has NO `ConcreteType` — inline object annotations
/// are deliberately outside `declared_annotation_concrete_type`), a
/// positionally exact `{ a0: ..., aN-1: ... }` object literal proving every
/// field's declared parameter type, minted or user-spelled alike. Proven-
/// divergent, unprovable, and pack-less (value-less completion / bare
/// `return`) exits are all NAMED rejections (surface-and-stop — CLAUDE.md
/// "if the type can't be proven, it is a compile error"); the ride
/// classifies them Hard and the seam wraps them through
/// `template_application_error` with both signatures at the `@application`
/// site. The `?`-exit is rejected during the walk itself (the round-2 F1
/// arm on `Face::Rewrite` — [`Scan::reject_try_operator_exit`]); exit
/// constructs hidden inside f-string interpolation interiors — invisible to
/// this gate's collections because the interior is raw text to the walker —
/// are rejected by the round-4 interior scan
/// ([`Scan::scan_fstring_interior_exits`]).
///
/// POSITIVE TWINS (gated to exactly what the weave accepts): the canonical
/// mutation-return spellings; any exit proving the carrier type (e.g.
/// `return args[0] * 2` on a 1-ary int target — type-sound and previously
/// working, the gate proves types, never polices spellings); and for
/// pure-read observation WITHOUT delivering the pack, the zero-parameter
/// void OBSERVER form (`before()` sugar / a zero-param void body fn) — the
/// observer route carries no pseudo-tuple plan and no carrier, the weave
/// calls it with zero arguments and threads the target's arguments
/// untouched, so it never enters this gate.
///
/// KNOWN BOUNDARY (named, deliberate — same as the after gate): the gate
/// proves the type of every visible value-producing exit; exhaustive
/// control-flow reachability (e.g. `loop`-break value paths — a loop in
/// tail position rejects conservatively as a value-less completion) stays
/// with the downstream compile battery.
fn guard_before_template_exit_kinds(
    compiler: &crate::compiler::BytecodeCompiler,
    plan: &PseudoTuplePlan,
    capture_tail: &[FunctionParameter],
    body: &[Statement],
    body_returns: &[Option<Expr>],
    local_bindings: &[(String, Option<Expr>)],
    assigned_locals: &std::collections::HashSet<String>,
) -> Result<()> {
    use shape_value::v2::ConcreteType;

    let (declared_slots, captures) = resolve_carrier_proof_env(compiler, plan, capture_tail)?;
    let locals = resolve_provable_locals(
        compiler,
        &declared_slots,
        &captures,
        local_bindings,
        assigned_locals,
    );

    let mut leaves = Vec::new();
    collect_tail_leaves_of_statements(body, &mut leaves);
    for return_value in body_returns {
        match return_value {
            Some(expr) => collect_result_value_leaves(expr, &mut leaves),
            None => leaves.push(ResultLeaf::NoValue),
        }
    }

    let args = &plan.args_param;
    let carrier_desc = match &plan.carrier {
        MutationCarrier::Single { annotation } => format!(
            "`{}` (the target's one parameter type — the typed args-mutation carrier)",
            annotation.to_type_string()
        ),
        MutationCarrier::Aggregate { .. } => format!(
            "the compiler-internal argument aggregate over ({}) (the typed args-mutation \
             carrier the weave reads back per-field at its typed read)",
            plan.render_target_params()
        ),
    };
    let value_less = || {
        reject(format!(
            "the template body can complete without delivering the mutated argument pack, \
             but the required specialization returns {carrier_desc} — a `before` template \
             delivers the typed `{args}` pack to the woven call on every exit; end the body \
             with `{args}` (or `return {args}`) on every path, or declare the template as a \
             zero-parameter void observer (`fn t() {{ ... }}`) to observe the call without \
             touching its arguments"
        ))
    };
    let divergent = |proven: &ConcreteType| {
        reject(format!(
            "the template body delivers `{proven}` at an exit but the required \
             specialization returns {carrier_desc} — a kind-divergent exit would make the \
             woven wrapper reinterpret the hook's raw return bits at its typed read; return \
             the whole mutated pack with `return {args}` (or a final bare `{args}`)"
        ))
    };
    let unprovable = || {
        reject(format!(
            "cannot prove the type of a value the template body delivers at an exit: the \
             typed args-mutation carrier requires every value-producing exit to prove \
             {carrier_desc} — return the whole mutated pack with `return {args}` (or a \
             final bare `{args}`), or deliver an expression whose type is provable at \
             specialization (a literal, an `{args}[i]` slot, a declared capture, a local \
             bound to a provable initializer, a typed fn's result, or arithmetic over those)"
        ))
    };

    for leaf in leaves {
        let expr = match leaf {
            ResultLeaf::NoValue => return Err(value_less()),
            ResultLeaf::Value(expr) => expr,
        };
        // The Aggregate carrier's conforming shape: a positionally exact
        // `{ a0: ..., aN-1: ... }` object literal (the minted carrier
        // expression, or an equivalent user spelling), every field proving
        // its declared parameter type.
        if let (MutationCarrier::Aggregate { fields }, Expr::Object(entries, _)) =
            (&plan.carrier, &expr)
        {
            let positionally_exact = entries.len() == fields.len()
                && entries.iter().zip(fields.iter()).all(|(entry, (name, _))| {
                    matches!(entry, ObjectEntry::Field { key, .. } if key == name)
                });
            if positionally_exact {
                for (index, entry) in entries.iter().enumerate() {
                    let ObjectEntry::Field { value, .. } = entry else {
                        unreachable!("guarded by positionally_exact");
                    };
                    let (param_name, annotation) = &plan.target_params[index];
                    let required = &declared_slots[index];
                    match carrier_write_concrete_type(
                        compiler,
                        &declared_slots,
                        &captures,
                        &locals,
                        value,
                    ) {
                        Some(proven) if &proven == required => {}
                        Some(proven) => {
                            return Err(reject(format!(
                                "the pack value delivered for field a{index} proves type \
                                 `{proven}` but the target's parameter `{param_name}` \
                                 declares `{ann}` — a kind-divergent field would make the \
                                 weave's typed a{index} read lie about its runtime kind; \
                                 deliver a `{ann}` value (or return the whole mutated pack \
                                 with `return {args}`)",
                                ann = annotation.to_type_string(),
                            )));
                        }
                        None => {
                            return Err(reject(format!(
                                "cannot prove the type of the pack value delivered for \
                                 field a{index}: the typed args-mutation carrier requires \
                                 it to prove the declared parameter type `{ann}` \
                                 (parameter `{param_name}`) — deliver a provable `{ann}` \
                                 value, or return the whole mutated pack with \
                                 `return {args}`",
                                ann = annotation.to_type_string(),
                            )));
                        }
                    }
                }
                continue;
            }
        }
        let required_single = match &plan.carrier {
            MutationCarrier::Single { .. } => Some(&declared_slots[0]),
            MutationCarrier::Aggregate { .. } => None,
        };
        match carrier_write_concrete_type(compiler, &declared_slots, &captures, &locals, &expr) {
            Some(proven) if required_single == Some(&proven) => {}
            Some(ConcreteType::Void) => return Err(value_less()),
            Some(proven) => return Err(divergent(&proven)),
            None => return Err(unprovable()),
        }
    }
    Ok(())
}

/// The guard's bounded, template-owned type evaluator: resolve the
/// `ConcreteType` an expression PROVES inside a rewritten specialized
/// template body. Knows exactly the template's own vocabulary — the minted
/// carrier locals (declared slot types), the declared trailing captures, the
/// S6 provable-initializer locals ([`resolve_provable_locals`]), and
/// structural arithmetic/comparison composition over those — and delegates
/// every other leaf to the compiler's existing
/// [`concrete_type_for_expr`](crate::compiler::monomorphization::type_resolution::concrete_type_for_expr)
/// resolver (literals, module constants, typed fn calls, ...). `None` =
/// unprovable — the guard rejects, never guesses (no fabricated kind).
fn carrier_write_concrete_type(
    compiler: &crate::compiler::BytecodeCompiler,
    declared_slots: &[shape_value::v2::ConcreteType],
    captures: &[(String, shape_value::v2::ConcreteType)],
    locals: &std::collections::HashMap<String, Option<shape_value::v2::ConcreteType>>,
    expr: &Expr,
) -> Option<shape_value::v2::ConcreteType> {
    use crate::compiler::monomorphization::type_resolution::concrete_type_for_expr;
    use shape_ast::ast::operators::{BinaryOp, UnaryOp};
    use shape_value::v2::ConcreteType;

    fn is_numeric(concrete: &ConcreteType) -> bool {
        matches!(
            concrete,
            ConcreteType::I64
                | ConcreteType::F64
                | ConcreteType::F32
                | ConcreteType::I32
                | ConcreteType::I16
                | ConcreteType::I8
                | ConcreteType::U64
                | ConcreteType::U32
                | ConcreteType::U16
                | ConcreteType::U8
        )
    }

    match expr {
        Expr::Identifier(name, _) => {
            for (index, concrete) in declared_slots.iter().enumerate() {
                if name.as_str() == slot_local_name(index) {
                    return Some(concrete.clone());
                }
            }
            // S6: the provable-initializer locals — consulted BEFORE the
            // capture env (a template-body `let` shadows a same-spelled
            // parameter). A provable entry IS the proof. A poisoned entry
            // (`Some(None)` — mutated, conflictingly rebound, or bound to
            // an initializer this evaluator cannot prove) SKIPS the capture
            // env (the flat map cannot tell a pre-shadow capture read from
            // a post-shadow local read) and falls through to
            // `concrete_type_for_expr` — exactly the pre-S6 resolution for
            // an unknown identifier: its span-keyed inference facts are
            // per-occurrence (shadow-correct for analyzer-visited API
            // bodies), and a sugar-minted body has no facts, so the name
            // stays unprovable and the guard's named rejection stands.
            match locals.get(name.as_str()) {
                Some(Some(proof)) => return Some(proof.clone()),
                Some(None) => return concrete_type_for_expr(compiler, expr),
                None => {}
            }
            if let Some((_, concrete)) = captures.iter().find(|(n, _)| n == name) {
                return Some(concrete.clone());
            }
            concrete_type_for_expr(compiler, expr)
        }
        // ADR-009 C3 #14 (slice 4, S4c): a method call whose RECEIVER is only
        // provable through THIS guard's own slot/capture env. The established
        // chain (`concrete_type_for_expr` — specialized returns, span-keyed
        // inference facts, receiver-derived projection) is tried FIRST; it
        // covers hand-written bodies, whose spans the analyzer visited. A
        // sugar-lowered template body's spans point inside the annotation
        // block (never analyzer-visited — no facts), so `tag.length()` on a
        // capture param needs the receiver proven from the guard env, then
        // the ONE registered-signature projection
        // (`method_return_for_receiver_concrete_type`) supplies the return.
        // Unproven receiver / unregistered method still yields `None` → the
        // loud named unprovable-write rejection stands (no fabrication).
        Expr::MethodCall {
            receiver, method, ..
        } => concrete_type_for_expr(compiler, expr).or_else(|| {
            let receiver_ct =
                carrier_write_concrete_type(compiler, declared_slots, captures, locals, receiver)?;
            crate::compiler::monomorphization::type_resolution::
                method_return_for_receiver_concrete_type(compiler, &receiver_ct, method)
        }),
        // Same S4c completion for a SINGLE-index read off a guard-env-proven
        // array (`cfg[0]` on an `Array<int>` capture param): the element type
        // IS the proof. Slices (`end_index` present) and non-array receivers
        // fall through to `None` — the loud rejection stands.
        Expr::IndexAccess {
            object,
            end_index: None,
            ..
        } => concrete_type_for_expr(compiler, expr).or_else(|| {
            match carrier_write_concrete_type(compiler, declared_slots, captures, locals, object)? {
                ConcreteType::Array(element) => Some(*element),
                _ => None,
            }
        }),
        Expr::BinaryOp {
            left, op, right, ..
        } => {
            let lt = carrier_write_concrete_type(compiler, declared_slots, captures, locals, left)?;
            let rt =
                carrier_write_concrete_type(compiler, declared_slots, captures, locals, right)?;
            match op {
                BinaryOp::Add => {
                    (lt == rt && (is_numeric(&lt) || lt == ConcreteType::String))
                        .then(|| lt.clone())
                }
                BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Pow => {
                    (lt == rt && is_numeric(&lt)).then(|| lt.clone())
                }
                BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitShl
                | BinaryOp::BitShr => {
                    (lt == rt && lt == ConcreteType::I64).then_some(ConcreteType::I64)
                }
                BinaryOp::Greater
                | BinaryOp::Less
                | BinaryOp::GreaterEq
                | BinaryOp::LessEq
                | BinaryOp::Equal
                | BinaryOp::NotEqual
                | BinaryOp::FuzzyEqual
                | BinaryOp::FuzzyGreater
                | BinaryOp::FuzzyLess => (lt == rt).then_some(ConcreteType::Bool),
                BinaryOp::And | BinaryOp::Or => (lt == ConcreteType::Bool
                    && rt == ConcreteType::Bool)
                    .then_some(ConcreteType::Bool),
                // Null-handling / pipeline composition is outside the
                // guard's structural domain — unprovable here, named
                // rejection at the guard (never a guess).
                BinaryOp::NullCoalesce | BinaryOp::ErrorContext | BinaryOp::Pipe => None,
            }
        }
        Expr::UnaryOp { op, operand, .. } => {
            let inner =
                carrier_write_concrete_type(compiler, declared_slots, captures, locals, operand)?;
            match op {
                UnaryOp::Neg => is_numeric(&inner).then(|| inner.clone()),
                UnaryOp::Not => (inner == ConcreteType::Bool).then_some(ConcreteType::Bool),
                UnaryOp::BitNot => (inner == ConcreteType::I64).then_some(ConcreteType::I64),
            }
        }
        _ => concrete_type_for_expr(compiler, expr),
    }
}

/// The shared top-level driver both faces run: per-statement traversal with
/// the implicit-return tail interception (a FINAL bare `args` expression
/// statement at the TOP LEVEL of the body is the mutation-return spelling).
fn walk_template_body(scan: &Scan<'_>, body: &mut [Statement]) -> Result<()> {
    let last = body.len().checked_sub(1);
    for (i, stmt) in body.iter_mut().enumerate() {
        if Some(i) == last {
            let is_tail_bare_args = matches!(
                stmt,
                Statement::Expression(Expr::Identifier(name, _), _)
                    if name.as_str() == scan.args_param
            );
            if is_tail_bare_args {
                if let Face::Rewrite { plan, .. } = scan.face {
                    let Statement::Expression(expr, _) = stmt else {
                        unreachable!("guarded by is_tail_bare_args");
                    };
                    let span = expr.span();
                    *expr = plan.carrier_expr(span);
                }
                continue;
            }
        }
        scan.statement(stmt, ScanMode::TemplateBody)?;
    }
    Ok(())
}

/// Where the walker currently is relative to the closure boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanMode {
    /// Directly inside the template body — the pseudo-tuple surface is live.
    TemplateBody,
    /// Inside a closure sub-expression — the pseudo-tuple does not cross
    /// closure boundaries in S1, so ANY occurrence of either name rejects.
    ClosureInterior,
}

/// Which face of the single walker is running (see the module docs).
#[derive(Clone, Copy)]
enum Face<'a> {
    /// Construction-time usage validation — classifies and rejects, never
    /// mutates.
    Validate,
    /// Specialization-time resolution — the same classification plus the G9
    /// node replacements. `carrier_writes` collects every per-slot mutation
    /// (`args[i] = expr`, post-rewrite RHS) for the S2c aggregate-kind guard
    /// (`guard_carrier_write_kinds`) — the walk only COLLECTS; the guard's
    /// comparison runs once after the walk. `local_bindings` /
    /// `assigned_locals` are the S6 provable-initializer-locals events (the
    /// supervisor-ordered S2c guard growth): every binding-position name is
    /// recorded in textual order (with its post-rewrite initializer for the
    /// simple `let name = expr` shape, `None` for every other binding
    /// construct), and every whole-name assignment target is recorded so a
    /// mutated local can be excluded from the provable set
    /// ([`resolve_provable_locals`] — evaluation happens once at guard time,
    /// never mid-walk, so loop-carried re-execution cannot outrun the proof).
    Rewrite {
        plan: &'a PseudoTuplePlan,
        carrier_writes: &'a std::cell::RefCell<Vec<(usize, Expr)>>,
        local_bindings: &'a std::cell::RefCell<Vec<(String, Option<Expr>)>>,
        assigned_locals: &'a std::cell::RefCell<std::collections::HashSet<String>>,
        /// ADR-009 C3 #14 (S6 fixlet round 3, F3): every BODY-LEVEL
        /// `return`-position value expression, POST-rewrite (`None` = a bare
        /// `return` — a pack-less exit the BEFORE-side exit gate rejects).
        /// The conforming mutation-return spellings (`return args`, and the
        /// final bare-`args` tail) are intercepted BEFORE collection and
        /// never enter this list — they are conforming by construction
        /// (rewritten to the carrier expression). Closure-interior and
        /// comptime-block returns are filtered exactly as on the after face
        /// (the F2 body-level-only rule at [`Scan::note_return_value`]).
        returns: &'a std::cell::RefCell<Vec<Option<Expr>>>,
    },
    /// ADR-009 C3 #14 (slice 5, S5a) — the [C0926] ambient-totality face:
    /// classifies every FREE identifier against the lexical scope stack and
    /// the module scope ([`AmbientScopeCtx`]); never mutates. Runs with the
    /// unspellable sentinel template names, so the pseudo-tuple surface is
    /// structurally disengaged (the template's own `args`/`Args` are
    /// ordinary in-scope parameters for this face).
    AmbientScope(&'a AmbientScopeCtx<'a>),
    /// ADR-009 C3 #14 (S6 soundness fixlet) — the AFTER-side return-kind
    /// collection face ([`guard_after_template_return_kind`]): walks a
    /// polymorphic `after` template body under the unspellable sentinel
    /// names (the pseudo-tuple surface is structurally disengaged — an
    /// `after` body has none) and COLLECTS the same provable-locals events
    /// as the rewrite face plus every BODY-LEVEL `return`-position value
    /// expression (closure-interior and comptime-block returns are not body
    /// exits and are filtered — S6 fixlet round 2, F2); never mutates.
    /// Round-2 F1 amendment: the walk carries ONE named rejection of its
    /// own — a body-level `?` exit ([`Scan::reject_try_operator_exit`]),
    /// whose propagated failure carrier is never provable at the guard;
    /// every other named rejection still runs once after the walk, at the
    /// guard.
    AfterReturnScan {
        local_bindings: &'a std::cell::RefCell<Vec<(String, Option<Expr>)>>,
        assigned_locals: &'a std::cell::RefCell<std::collections::HashSet<String>>,
        /// Every `return` statement/expression's value (`None` = a bare
        /// `return` — a no-value exit the guard rejects against `(R) -> R`).
        returns: &'a std::cell::RefCell<Vec<Option<Expr>>>,
    },
}

/// What the template-body interception decided for one expression node.
enum Intercept {
    /// Not a pseudo-tuple form — continue the generic traversal.
    No,
    /// A legal pseudo-tuple form; the validate face keeps the node.
    LegalKeep,
    /// A legal pseudo-tuple form; the rewrite face replaces the node.
    Replace(Expr),
}

/// ADR-009 C3 #14 (S6 fixlet round 4): which EXIT construct the
/// interpolation-interior scan found (see
/// [`Scan::scan_fstring_interior_exits`] /
/// [`Scan::reject_fstring_interior_exit`]).
#[derive(Clone, Copy)]
enum FstringInteriorExit {
    /// `?` — an unconditional early return of the propagated failure
    /// carrier (the round-2 F1 mechanism, hidden inside the interior).
    TryOperator,
    /// `return` (expression or statement form) — exits the specialized body
    /// from inside the string build.
    Return,
}

struct Scan<'a> {
    args_param: &'a str,
    type_param: &'a str,
    /// S5a Dec-65 walker arm: > 0 while inside an `Expr::Comptime` block.
    /// The pseudo-tuple surface is NOT live there (comptime code evaluates
    /// before the hook's runtime inputs exist), so `args`/`Args` uses inside
    /// a comptime block reject with the named Dec-65 sentence instead of
    /// being intercepted as legal reads (which would leak a minted
    /// `__c3_arg_{i}` name into the mini-VM's unresolved-identifier error).
    comptime_depth: std::cell::Cell<usize>,
    face: Face<'a>,
}

fn reject(message: String) -> ShapeError {
    ShapeError::SemanticError {
        message,
        location: None,
    }
}

impl<'a> Scan<'a> {
    // ---------------------------------------------------------------------
    // Named rejections (uncoded sentences + positive twins; S5 owns C09xx
    // minting from C0931+ — no code brackets here).
    // ---------------------------------------------------------------------

    fn reject_non_constant_index(&self) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple requires a compile-time-constant index: write \
             `{args}[<int literal>]` (for example `{args}[0]`), which resolves to the target's \
             parameter slot at specialization; a non-constant index has no parameter slot to \
             resolve to",
            args = self.args_param
        ))
    }

    /// Rewrite face only: a constant index with no parameter slot on THIS
    /// target. Quotes the index and the target's arity + signature (the
    /// specialization seam adds the two-signature application-site frame).
    fn reject_index_out_of_range(
        &self,
        index: i64,
        plan: &PseudoTuplePlan,
    ) -> ShapeError {
        let arity = plan.arity();
        reject(format!(
            "the `{args}` pseudo-tuple index {index} is out of range for this target: the \
             target declares {arity} parameter{plural} ({params}), so constant indices resolve \
             only in 0..{arity}",
            args = self.args_param,
            plural = if arity == 1 { "" } else { "s" },
            params = plan.render_target_params(),
        ))
    }

    fn reject_slice(&self) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple cannot be sliced: no tuple value exists at runtime; use \
             `{args}[<int literal>]` for one typed parameter slot or `{args}.length` for the \
             parameter count",
            args = self.args_param
        ))
    }

    fn reject_other_property(&self, property: &str) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple has no property `{property}`: its only property is \
             `{args}.length` (a specialization-time constant); use `{args}[<int literal>]` for \
             the typed parameter slots",
            args = self.args_param
        ))
    }

    fn reject_optional_access(&self) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple is never optional: write `{args}.length` as a plain \
             access; optional chaining (`?.`) has no null case to guard on a \
             specialization-resolved constant",
            args = self.args_param
        ))
    }

    fn reject_bare_value(&self) -> ShapeError {
        reject(format!(
            "the `{args}` pseudo-tuple has no first-class value: address one typed parameter \
             slot as `{args}[<int literal>]`, the parameter count as `{args}.length`, or return \
             the whole mutated pack with `return {args}` (or a final bare `{args}`)",
            args = self.args_param
        ))
    }

    fn reject_closure_occurrence(&self, name: &str) -> ShapeError {
        reject(format!(
            "`{name}` cannot appear inside a closure: the `{args}` pseudo-tuple is \
             specialization-resolved and does not cross closure boundaries in S1; do the \
             pseudo-tuple access in the template body itself and pass the resulting value into \
             the closure",
            args = self.args_param
        ))
    }

    fn reject_type_param_annotation(&self) -> ShapeError {
        reject(format!(
            "the template type parameter `{tp}` cannot appear in a body-internal type \
             annotation: it names the whole bound signature and resolves away at \
             specialization; annotate with a concrete type or let inference type the binding",
            tp = self.type_param
        ))
    }

    /// Verify-1 fix: the template type parameter spelled as a call name
    /// (`Args(1)` / `x.Args(1)` / `Args::f(1)`).
    fn reject_type_param_call(&self) -> ShapeError {
        reject(format!(
            "the template type parameter `{tp}` cannot be called or used as a call name: it \
             names the whole bound signature and resolves away at specialization; call a \
             concrete function, or address the typed parameter slots through \
             `{args}[<int literal>]`",
            tp = self.type_param,
            args = self.args_param
        ))
    }

    fn reject_reserved_prefix(&self, name: &str) -> ShapeError {
        reject(format!(
            "identifier `{name}` uses the reserved prefix `{prefix}` (the compiler-internal \
             namespace for specialization-minted locals); choose a name without the `{prefix}` \
             prefix",
            prefix = RESERVED_SPECIALIZATION_PREFIX
        ))
    }

    fn reject_rebind(&self, name: &str) -> ShapeError {
        reject(format!(
            "`{name}` cannot be rebound inside a template body: the name is part of the \
             pseudo-tuple surface (`{args}` / its type parameter `{tp}`) and rebinding would \
             shadow the specialization-resolved meaning; choose a different binding name",
            args = self.args_param,
            tp = self.type_param
        ))
    }

    /// ADR-009 C3 #14 (S6 fixlet round 2, F1) — a body-level `?` exit. The
    /// `?` operator's failure path is an UNCONDITIONAL early return of the
    /// propagated failure carrier (`expressions/advanced.rs::
    /// compile_expr_try_operator`), bypassing the typed exit contract each
    /// specialization-time gate proves. MEASURED on the round-1 gate
    /// (2026-07-21 round-2 probes): after side — the Err carrier escaped a
    /// `(int) -> int` specialization as `R` and `add(3, 4) + 1` printed the
    /// pointer bits `102997035238305` (the heap-pointer-reinterpretation
    /// class); before side — the Err carrier reached the weave's aggregate
    /// consumption and silently corrupted the woven call (`1` where `8`).
    /// The propagated carrier's type is never provable at the scan, so this
    /// is a NAMED surface-and-stop rejection (CLAUDE.md: "if the type can't
    /// be proven, it is a compile error"), with the explicit-`match`
    /// positive twin.
    fn reject_try_operator_exit(&self) -> ShapeError {
        match self.face {
            Face::AfterReturnScan { .. } => reject(
                "the `?` operator cannot be used in an `after` template body: its failure \
                 path propagates the `Err` value as the specialized body's early return, and \
                 the propagated `Err` carrier can never prove the target's declared result \
                 type (an `after` template returns the typed result on every path) — handle \
                 the `Result` explicitly (`match f() { Ok(v) => ..., Err(e) => ... }`) and \
                 return a result-typed value on every path"
                    .to_string(),
            ),
            _ => reject(format!(
                "the `?` operator cannot be used in a `before` template body: its failure \
                 path propagates the `Err` value as the specialized body's early return, and \
                 the propagated `Err` carrier can never prove the typed argument pack the \
                 weave consumes (a `before` template's exits deliver the typed `{args}` \
                 pack) — handle the `Result` explicitly (`match f() {{ Ok(v) => ..., \
                 Err(e) => ... }}`) and return the `{args}` pack on every path",
                args = self.args_param
            )),
        }
    }

    /// ADR-009 C3 #14 (S6 fixlet round 4) — an EXIT construct (`?` /
    /// `return`) inside an f-string INTERPOLATION INTERIOR. An interpolation
    /// part is re-parsed and compiled IN THE ENCLOSING FRAME at emission
    /// (`string_interpolation.rs:277-295`), so the construct exits the
    /// SPECIALIZED BODY — but the interior is raw text to the walker (the
    /// module-docs F5 boundary), so the round-2 F1 arm and both exit gates'
    /// collections never saw it. MEASURED on the round-3 state (2026-07-21
    /// round-4 probe): an `after` body `let x = f"{fallible(0)?}"` then
    /// `result` on `fn add(a: int, b: int) -> int` RAN and `add(3, 4) + 1`
    /// printed the Err carrier's pointer bits `95158939846801` (the
    /// heap-pointer-reinterpretation class). NAMED surface-and-stop
    /// rejection (CLAUDE.md: "if the type can't be proven, it is a compile
    /// error") with the move-it-out positive twin, per the F1 sentence
    /// conventions.
    fn reject_fstring_interior_exit(&self, construct: FstringInteriorExit) -> ShapeError {
        let (construct_head, construct_mechanism, construct_twin) = match construct {
            FstringInteriorExit::TryOperator => (
                "the `?` operator cannot be used",
                "its failure path propagates the `Err` value as the specialized body's \
                 early return from inside the string build",
                "hoist the fallible call out of the f-string, handle the `Result` \
                 explicitly (`match f() { Ok(v) => ..., Err(e) => ... }`), and \
                 interpolate the handled local (`f\"{v}\"`)",
            ),
            FstringInteriorExit::Return => (
                "a `return` cannot be used",
                "it exits the specialized body from inside the string build",
                "move the `return` out of the f-string and interpolate a plain local \
                 computed before it",
            ),
        };
        match self.face {
            Face::AfterReturnScan { .. } => reject(format!(
                "{construct_head} inside an f-string interpolation in an `after` template \
                 body: the interpolation expression compiles in the specialized body's own \
                 frame, so {construct_mechanism}, bypassing the typed exit contract (an \
                 `after` template returns the typed result on every path) — \
                 {construct_twin}"
            )),
            _ => reject(format!(
                "{construct_head} inside an f-string interpolation in a `before` template \
                 body: the interpolation expression compiles in the specialized body's own \
                 frame, so {construct_mechanism}, bypassing the typed exit contract (a \
                 `before` template's exits deliver the typed `{args}` pack) — \
                 {construct_twin}",
                args = self.args_param
            )),
        }
    }

    /// S5a Dec-65 walker arm (uncoded, G13 string-tag with the #60 routing
    /// note): a template pseudo-tuple / type-param name read inside an
    /// `Expr::Comptime` block. Comptime code evaluates at compile time,
    /// before the hook's runtime inputs exist — without this arm the
    /// rewrite face would resolve `args[i]` to a minted `__c3_arg_{i}`
    /// local and the comptime mini-VM would leak that reserved name in a
    /// bland unresolved-identifier error.
    fn reject_comptime_runtime_input(&self, name: &str) -> ShapeError {
        reject(format!(
            "a `comptime` block inside a template body cannot read `{name}`: comptime code \
             evaluates at compile time, before the hook's runtime inputs exist (Dec 65 — \
             runtime values never enter a comptime evaluation position); compute with \
             comptime-known values instead, or move the runtime work outside the `comptime` \
             block"
        ))
    }

    // ---------------------------------------------------------------------
    // S5a comptime-interior tracking + ambient-face helpers
    // ---------------------------------------------------------------------

    /// True while the pseudo-tuple surface is live: outside any comptime
    /// block. Gates every interception (reads, writes, mutation-returns) so
    /// `args`/`Args` inside `comptime { }` fall through to the named Dec-65
    /// rejection instead of classifying as legal runtime uses.
    fn pseudo_tuple_surface_live(&self) -> bool {
        self.comptime_depth.get() == 0
    }

    /// A template name (`args_param` / `type_param`) observed inside a
    /// comptime block in TemplateBody mode → the Dec-65 rejection.
    fn check_comptime_runtime_input(&self, name: &str, mode: ScanMode) -> Result<()> {
        if mode == ScanMode::TemplateBody
            && !self.pseudo_tuple_surface_live()
            && (name == self.args_param || name == self.type_param)
        {
            return Err(self.reject_comptime_runtime_input(name));
        }
        Ok(())
    }

    fn ambient(&self) -> Option<&'a AmbientScopeCtx<'a>> {
        match self.face {
            Face::AmbientScope(ctx) => Some(ctx),
            _ => None,
        }
    }

    /// Run `f` inside a fresh lexical frame on the ambient face (no-op
    /// otherwise). Binders registered inside pop with the frame — the
    /// disjoint-branch shadow discipline.
    fn ambient_frame<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        if let Some(ctx) = self.ambient() {
            ctx.push_frame();
            let result = f();
            ctx.pop_frame();
            result
        } else {
            f()
        }
    }

    /// Ambient face: register a binder in the current frame.
    fn ambient_bind(&self, name: &str) {
        if let Some(ctx) = self.ambient() {
            ctx.bind(name);
        }
    }

    /// Ambient face, VALUE position: a free identifier resolving to a
    /// module-scope VALUE binding is [C0926]. Module bindings are checked
    /// BEFORE fn tables — the emission path's own precedence
    /// (`identifiers.rs:350` resolves module bindings before
    /// `find_function`), so the gate rejects exactly what the emitted code
    /// would silently load. Unresolvable names are left to ordinary
    /// downstream resolution (unchanged sentences).
    fn ambient_check_value_name(&self, name: &str) -> Result<()> {
        let Some(ctx) = self.ambient() else {
            return Ok(());
        };
        // The F5-boundary arm runs BEFORE the in_scope check: the template's
        // own `args`/`Args` ARE frame-0 in-scope params for this face, but
        // inside an f-string interior that in-scope-ness is a lie — the
        // Rewrite face never resolves interiors (fix round 1, F1).
        ctx.check_fstring_template_name(name)?;
        if ctx.in_scope(name) {
            return Ok(());
        }
        if let Some(resolved) = ctx.resolve_module_value_binding(name) {
            return Err(ctx.ambient_rejection(name, &resolved));
        }
        Ok(())
    }

    /// Ambient face, CALL-NAME position: fn-callees at module scope are
    /// LEGIT (G3); a call name that is NOT a module fn but resolves to a
    /// module-scope VALUE binding (a module `let` holding a closure) is
    /// [C0926].
    fn ambient_check_call_name(&self, name: &str) -> Result<()> {
        let Some(ctx) = self.ambient() else {
            return Ok(());
        };
        // Same F5-boundary arm as the value position (fix round 1, F1): a
        // call spelled with the template name inside an f-string interior
        // can never be specialized either.
        ctx.check_fstring_template_name(name)?;
        if ctx.in_scope(name) || ctx.is_module_fn(name) {
            return Ok(());
        }
        if let Some(resolved) = ctx.resolve_module_value_binding(name) {
            return Err(ctx.ambient_rejection(name, &resolved));
        }
        Ok(())
    }

    /// Ambient face, ASSIGNMENT-target position: a store into a module-scope
    /// binding (`StoreModuleBinding`) is just as ambient as a read.
    fn ambient_check_assign_target(&self, name: &str) -> Result<()> {
        self.ambient_check_value_name(name)
    }

    /// Ambient face: scan f-string interpolation interiors (the MUST-SCAN
    /// position the pseudo-tuple faces skip — see the module docs). Each
    /// interpolation expression re-parses exactly as the emitter does
    /// (`string_interpolation.rs:277`); parse failures are left to the
    /// emitter's own error.
    fn ambient_scan_fstring(
        &self,
        value: &str,
        interp_mode: shape_ast::ast::InterpolationMode,
        mode: ScanMode,
    ) -> Result<()> {
        let Some(ctx) = self.ambient() else {
            return Ok(());
        };
        let Ok(parts) = shape_ast::interpolation::parse_interpolation_with_mode(value, interp_mode)
        else {
            return Ok(());
        };
        for part in parts {
            if let shape_ast::interpolation::InterpolationPart::Expression { expr, .. } = part {
                let Ok(mut parsed) = shape_ast::parser::parse_expression_str(&expr) else {
                    continue;
                };
                // Interior-depth tracking for the F5-boundary arm
                // (`AmbientScopeCtx::check_fstring_template_name`, fix
                // round 1, F1): the template's own `args`/`Args` spellings
                // reject inside interiors — the Rewrite face never resolves
                // them here.
                ctx.fstring_interior_depth
                    .set(ctx.fstring_interior_depth.get() + 1);
                let result = self.expr(&mut parsed, mode);
                ctx.fstring_interior_depth
                    .set(ctx.fstring_interior_depth.get() - 1);
                result?;
            }
        }
        Ok(())
    }

    /// ADR-009 C3 #14 (S6 fixlet round 4) — the specialization faces'
    /// interpolation-interior EXIT scan: parse this f-string's interpolation
    /// parts (the same parse [`Self::ambient_scan_fstring`] runs — parse
    /// failures are left to the emitter's own error) and reject the two
    /// exit constructs found inside (`Expr::TryOperator` and
    /// `Expr::Return` / `Statement::Return` — see
    /// [`Self::reject_fstring_interior_exit`] for the measured leak and the
    /// sentence). Called at the `Literal::FormattedString` leaf on
    /// [`Face::Rewrite`] and [`Face::AfterReturnScan`] in
    /// [`ScanMode::TemplateBody`] with the surface live; an f-string inside
    /// a closure interior compiles its interiors in the CLOSURE frame, so
    /// the mode gate skips it there (the F1 closure-transparency boundary).
    ///
    /// SCAN-ONLY BY DESIGN: the re-parsed temp AST is searched, never
    /// walked by the faces — on the rewrite face an intercept-Replace
    /// mutation into the temp AST would be SILENTLY DISCARDED (the raw text
    /// is what emission re-parses), which is exactly the class of quiet
    /// wrongness this module refuses.
    fn scan_fstring_interior_exits(
        &self,
        value: &str,
        interp_mode: shape_ast::ast::InterpolationMode,
    ) -> Result<()> {
        let Ok(parts) = shape_ast::interpolation::parse_interpolation_with_mode(value, interp_mode)
        else {
            return Ok(());
        };
        for part in parts {
            if let shape_ast::interpolation::InterpolationPart::Expression { expr, .. } = part {
                let Ok(parsed) = shape_ast::parser::parse_expression_str(&expr) else {
                    continue;
                };
                self.scan_parsed_interior_for_exits(&parsed)?;
            }
        }
        Ok(())
    }

    /// The scan-only pass over ONE re-parsed interpolation-interior
    /// expression (see [`Self::scan_fstring_interior_exits`]). Boundaries
    /// mirror the walker's own (F1/F2): closure interiors are pruned (`?` /
    /// `return` there exit the CLOSURE frame, never the specialized body)
    /// and comptime blocks are pruned (compile-time mini-VM evaluation,
    /// Dec 65). A NESTED f-string literal inside the interior compiles its
    /// own interiors in the same enclosing frame, so the scan recurses into
    /// it.
    fn scan_parsed_interior_for_exits(&self, parsed: &Expr) -> Result<()> {
        use shape_runtime::visitor::{Visitor, walk_expr};

        struct ExitScan<'s, 'a> {
            scan: &'s Scan<'a>,
            found: Option<ShapeError>,
        }

        impl ExitScan<'_, '_> {
            fn record(&mut self, err: ShapeError) {
                if self.found.is_none() {
                    self.found = Some(err);
                }
            }
        }

        impl Visitor for ExitScan<'_, '_> {
            fn visit_expr_try_operator(&mut self, _expr: &Expr, _span: Span) -> bool {
                let err = self
                    .scan
                    .reject_fstring_interior_exit(FstringInteriorExit::TryOperator);
                self.record(err);
                false
            }
            fn visit_expr_return(&mut self, _expr: &Expr, _span: Span) -> bool {
                let err = self
                    .scan
                    .reject_fstring_interior_exit(FstringInteriorExit::Return);
                self.record(err);
                false
            }
            // Statement-form `return` (block-bearing interiors).
            fn visit_stmt(&mut self, stmt: &Statement) -> bool {
                if matches!(stmt, Statement::Return(_, _)) {
                    let err = self
                        .scan
                        .reject_fstring_interior_exit(FstringInteriorExit::Return);
                    self.record(err);
                    return false;
                }
                true
            }
            // Closure interiors: exits there leave the CLOSURE frame.
            fn visit_expr_function_expr(&mut self, _expr: &Expr, _span: Span) -> bool {
                false
            }
            // Comptime blocks: compile-time mini-VM evaluation (Dec 65).
            fn visit_expr_comptime(&mut self, _expr: &Expr, _span: Span) -> bool {
                false
            }
            fn visit_expr_comptime_for(&mut self, _expr: &Expr, _span: Span) -> bool {
                false
            }
            // Nested f-string: its interiors compile in the same enclosing
            // frame — recurse.
            fn visit_expr_literal(&mut self, expr: &Expr, _span: Span) -> bool {
                if let Expr::Literal(Literal::FormattedString { value, mode }, _) = expr {
                    if self.found.is_none() {
                        if let Err(err) = self.scan.scan_fstring_interior_exits(value, *mode) {
                            self.record(err);
                        }
                    }
                }
                true
            }
        }

        let mut visitor = ExitScan {
            scan: self,
            found: None,
        };
        walk_expr(&mut visitor, parsed);
        match visitor.found {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    // ---------------------------------------------------------------------
    // Name checks
    // ---------------------------------------------------------------------

    /// Any identifier spelling, in any role: the reserved-prefix check.
    /// The ambient face SKIPS it — that face classifies module-scope
    /// resolution only and must not tighten the (never construction-walked)
    /// concrete-body surface beyond its [C0926] charter. The S6
    /// after-return face skips it for the same reason: `after` bodies are
    /// never construction-walked, and the return-kind gate must not
    /// introduce rejections outside its return-type charter.
    fn check_reserved(&self, name: &str) -> Result<()> {
        if matches!(
            self.face,
            Face::AmbientScope(_) | Face::AfterReturnScan { .. }
        ) {
            return Ok(());
        }
        if name.starts_with(RESERVED_SPECIALIZATION_PREFIX) {
            return Err(self.reject_reserved_prefix(name));
        }
        Ok(())
    }

    /// ADR-009 C3 #14 (S6): record one binding-position name for the
    /// provable-locals resolution ([`resolve_provable_locals`]). `init` is
    /// `Some` ONLY for the simple `let name = expr` shape (the post-rewrite
    /// initializer on the rewrite face); every other binding construct
    /// (destructures, loop/match/query/closure binders, no-initializer
    /// declarations) records `None` — unprovable by construction, so a
    /// same-spelled shadow can never be blessed through the flat map.
    fn note_binding_event(&self, name: &str, init: Option<&Expr>) {
        match self.face {
            Face::Rewrite { local_bindings, .. }
            | Face::AfterReturnScan { local_bindings, .. } => {
                local_bindings
                    .borrow_mut()
                    .push((name.to_string(), init.cloned()));
            }
            Face::Validate | Face::AmbientScope(_) => {}
        }
    }

    /// ADR-009 C3 #14 (S6): record one whole-name assignment target. A name
    /// that is EVER assigned after binding is excluded from the provable
    /// set entirely (conservative — re-execution order under loops cannot be
    /// bounded by a textual walk, so a mutated local never joins the set).
    fn note_assigned_local(&self, name: &str) {
        match self.face {
            Face::Rewrite {
                assigned_locals, ..
            }
            | Face::AfterReturnScan {
                assigned_locals, ..
            } => {
                assigned_locals.borrow_mut().insert(name.to_string());
            }
            Face::Validate | Face::AmbientScope(_) => {}
        }
    }

    /// ADR-009 C3 #14 (S6): record one `return`-position value expression
    /// (`None` = a bare `return`) for the exit gates — the after-side
    /// return-kind guard ([`Face::AfterReturnScan`]) and the S6-round-3 F3
    /// before-side exit gate ([`Face::Rewrite`], consumed by
    /// [`guard_before_template_exit_kinds`]). Callers note AFTER walking the
    /// value, so on the rewrite face the collected expression carries the
    /// POST-rewrite spelling (pseudo-tuple reads already resolved to minted
    /// slot locals — the same ordering rule as the `carrier_writes`
    /// collection); the after face never mutates, so the ordering is
    /// observation-equivalent there.
    ///
    /// S6 fixlet round 2, F2: BODY-LEVEL returns only. A closure's own
    /// `return` exits the CLOSURE frame, and a comptime block's `return`
    /// ends the compile-time mini-VM evaluation (Dec 65) — neither reaches
    /// the specialized body's typed exit, so neither enters the guard
    /// (before this filter a closure helper's non-`R` internal return
    /// inside a type-correct `after` body false-positively rejected the
    /// install — MEASURED round-2 probe). Tail-position closures/comptime
    /// blocks are unaffected: they arrive at the guard as ordinary
    /// [`ResultLeaf::Value`] leaves and prove (or conservatively reject) as
    /// whole expressions.
    fn note_return_value(&self, value: Option<&Expr>, mode: ScanMode) {
        if mode == ScanMode::ClosureInterior || self.comptime_depth.get() > 0 {
            return;
        }
        match self.face {
            Face::AfterReturnScan { returns, .. } | Face::Rewrite { returns, .. } => {
                returns.borrow_mut().push(value.cloned());
            }
            Face::Validate | Face::AmbientScope(_) => {}
        }
    }

    /// A name in BINDING position (let/for/match/query bindings).
    fn check_binding_name(&self, name: &str, mode: ScanMode) -> Result<()> {
        self.check_binding_name_tracked(name, mode, None)
    }

    /// [`Self::check_binding_name`] with the S6 provable-initializer event:
    /// the `variable_decl` simple-identifier path passes its (post-rewrite)
    /// initializer; every other caller records the unprovable default.
    fn check_binding_name_tracked(
        &self,
        name: &str,
        mode: ScanMode,
        init: Option<&Expr>,
    ) -> Result<()> {
        self.check_reserved(name)?;
        self.ambient_bind(name);
        self.note_binding_event(name, init);
        match mode {
            ScanMode::ClosureInterior => {
                if name == self.args_param || name == self.type_param {
                    return Err(self.reject_closure_occurrence(name));
                }
            }
            ScanMode::TemplateBody => {
                if name == self.args_param || name == self.type_param {
                    return Err(self.reject_rebind(name));
                }
            }
        }
        Ok(())
    }

    /// A name in ASSIGNMENT-target position (mutating an existing binding).
    fn check_assign_target_name(&self, name: &str, mode: ScanMode) -> Result<()> {
        self.check_reserved(name)?;
        self.ambient_check_assign_target(name)?;
        self.note_assigned_local(name);
        match mode {
            ScanMode::ClosureInterior => {
                if name == self.args_param || name == self.type_param {
                    return Err(self.reject_closure_occurrence(name));
                }
            }
            ScanMode::TemplateBody => {
                // `args = e` mutates the whole pack as a value — not a legal
                // use (only per-slot `args[i] = e` is).
                if name == self.args_param {
                    return Err(self.reject_bare_value());
                }
            }
        }
        Ok(())
    }

    /// A name in CALL-NAME position (function name, qualified-call
    /// namespace/function, method name). Verify-1 fix: call names previously
    /// checked only the reserved prefix, so `args(1)` passed both faces and
    /// post-rewrite either degraded to a confusing downstream resolution
    /// error or — if a module fn spelled `args` existed — silently resolved
    /// to it (a meaning shift). Call names resolve OUTSIDE the pseudo-tuple
    /// surface, so either template name appearing as one is a NAMED
    /// rejection under both scan modes: the same bare-value /
    /// closure-occurrence sentences for `args_param`, the call-name twin of
    /// the annotation rejection for `type_param`.
    fn check_call_name(&self, name: &str, mode: ScanMode) -> Result<()> {
        self.check_reserved(name)?;
        match mode {
            ScanMode::ClosureInterior => {
                if name == self.args_param || name == self.type_param {
                    return Err(self.reject_closure_occurrence(name));
                }
            }
            ScanMode::TemplateBody => {
                if name == self.args_param {
                    return Err(self.reject_bare_value());
                }
                if name == self.type_param {
                    return Err(self.reject_type_param_call());
                }
            }
        }
        Ok(())
    }

    fn is_args_identifier(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Identifier(name, _) if name == self.args_param)
    }

    /// The minted local for a range-checked constant index (rewrite face).
    fn checked_slot_local(
        &self,
        index: i64,
        plan: &PseudoTuplePlan,
    ) -> Result<String> {
        if index < 0 || (index as usize) >= plan.arity() {
            return Err(self.reject_index_out_of_range(index, plan));
        }
        Ok(slot_local_name(index as usize))
    }

    // ---------------------------------------------------------------------
    // Statements
    // ---------------------------------------------------------------------

    fn statements(&self, stmts: &mut [Statement], mode: ScanMode) -> Result<()> {
        for stmt in stmts.iter_mut() {
            self.statement(stmt, mode)?;
        }
        Ok(())
    }

    fn statement(&self, stmt: &mut Statement, mode: ScanMode) -> Result<()> {
        match stmt {
            Statement::Return(value, _) => {
                // `return args` — the mutation-return spelling (template body
                // only; inside a closure it is an occurrence like any other;
                // inside a comptime block the surface is not live — Dec 65).
                if mode == ScanMode::TemplateBody && self.pseudo_tuple_surface_live() {
                    if let Some(inner) = value {
                        if self.is_args_identifier(inner) {
                            if let Face::Rewrite { plan, .. } = self.face {
                                let span = inner.span();
                                *inner = plan.carrier_expr(span);
                            }
                            return Ok(());
                        }
                    }
                }
                // S6 exit-gate collection: every BODY-LEVEL return-position
                // value (a bare `return` records `None` — a no-value exit;
                // closure-interior/comptime returns are filtered — F2). The
                // value is walked FIRST so the rewrite face collects the
                // POST-rewrite expression (round 3, F3 — the `carrier_writes`
                // ordering rule; the after face never mutates, so the order
                // is observation-equivalent there).
                self.opt_expr(value.as_mut(), mode)?;
                self.note_return_value(value.as_ref(), mode);
                Ok(())
            }
            Statement::Break(_) | Statement::Continue(_) | Statement::RemoveTarget(_) => Ok(()),
            Statement::VariableDecl(decl, _) => self.variable_decl(decl, mode),
            Statement::Assignment(assign, _) => self.assignment(assign, mode),
            Statement::Expression(expr, _) => self.expr(expr, mode),
            Statement::For(for_loop, _) => self.ambient_frame(|| {
                match &mut for_loop.init {
                    ForInit::ForIn { pattern, iter } => {
                        // Iterable BEFORE the pattern binds: the iterable
                        // cannot see the loop binder (order load-bearing for
                        // the ambient face; order-independent for the
                        // pseudo-tuple faces).
                        self.expr(iter, mode)?;
                        self.destructure_pattern_binding(pattern, mode)?;
                    }
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.statement(init, mode)?;
                        self.expr(condition, mode)?;
                        self.expr(update, mode)?;
                    }
                }
                self.statements(&mut for_loop.body, mode)
            }),
            Statement::While(while_loop, _) => {
                self.expr(&mut while_loop.condition, mode)?;
                self.ambient_frame(|| self.statements(&mut while_loop.body, mode))
            }
            Statement::If(if_stmt, _) => {
                self.expr(&mut if_stmt.condition, mode)?;
                self.ambient_frame(|| self.statements(&mut if_stmt.then_body, mode))?;
                if let Some(else_body) = &mut if_stmt.else_body {
                    self.ambient_frame(|| self.statements(else_body, mode))?;
                }
                Ok(())
            }
            Statement::Extend(ext, _) => self.extend(ext, mode),
            Statement::SetParamType {
                type_annotation, ..
            } => self.type_annotation(type_annotation, mode),
            Statement::SetParamTypeExpr { expression, .. } => self.expr(expression, mode),
            Statement::SetParamValue { expression, .. } => self.expr(expression, mode),
            Statement::SetReturnType {
                type_annotation, ..
            } => self.type_annotation(type_annotation, mode),
            Statement::SetReturnExpr { expression, .. } => self.expr(expression, mode),
            Statement::ReplaceBody { body, .. } => self.statements(body, mode),
            Statement::ReplaceBodyExpr { expression, .. } => self.expr(expression, mode),
            Statement::ReplaceModuleExpr { expression, .. } => self.expr(expression, mode),
            Statement::ExtendItemsExpr { expression, .. } => self.expr(expression, mode),
        }
    }

    fn variable_decl(&self, decl: &mut VariableDecl, mode: ScanMode) -> Result<()> {
        // Value/annotation BEFORE the pattern binds: `let x = x + 1`
        // references the OUTER `x` on its right-hand side (sequential
        // visibility — order load-bearing for the ambient face,
        // order-independent for the pseudo-tuple faces).
        if let Some(annotation) = &decl.type_annotation {
            self.type_annotation(annotation, mode)?;
        }
        self.opt_expr(decl.value.as_mut(), mode)?;
        // S6 provable-initializer locals: the simple `let name = expr` shape
        // records its POST-rewrite initializer (the value walk above already
        // resolved any `args[i]` reads to minted slot locals); every other
        // pattern shape records the unprovable default through
        // `check_binding_name`.
        match &decl.pattern {
            DestructurePattern::Identifier(name, _) => {
                self.check_binding_name_tracked(name, mode, decl.value.as_ref())
            }
            other => self.destructure_pattern_binding(other, mode),
        }
    }

    fn assignment(&self, assign: &mut Assignment, mode: ScanMode) -> Result<()> {
        self.destructure_pattern_assign_target(&assign.pattern, mode)?;
        self.expr(&mut assign.value, mode)
    }

    fn extend(&self, ext: &mut ExtendStatement, mode: ScanMode) -> Result<()> {
        for method in &mut ext.methods {
            self.method_def(method, mode)?;
        }
        Ok(())
    }

    fn method_def(&self, method: &mut MethodDef, mode: ScanMode) -> Result<()> {
        self.check_reserved(&method.name)?;
        for annotation in &mut method.annotations {
            self.annotation_args(annotation, mode)?;
        }
        self.ambient_frame(|| {
            for param in &mut method.params {
                self.function_parameter(param, mode)?;
            }
            if let Some(when) = &mut method.when_clause {
                self.expr(when, mode)?;
            }
            if let Some(ret) = &method.return_type {
                self.type_annotation(ret, mode)?;
            }
            self.statements(&mut method.body, mode)
        })
    }

    fn function_parameter(&self, param: &mut FunctionParameter, mode: ScanMode) -> Result<()> {
        self.destructure_pattern_binding(&param.pattern, mode)?;
        if let Some(annotation) = &param.type_annotation {
            self.type_annotation(annotation, mode)?;
        }
        self.opt_expr(param.default_value.as_mut(), mode)
    }

    fn annotation_args(&self, annotation: &mut Annotation, mode: ScanMode) -> Result<()> {
        for arg in &mut annotation.args {
            self.expr(arg, mode)?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Patterns (never rewritten — shared borrows suffice)
    // ---------------------------------------------------------------------

    fn destructure_pattern_binding(&self, pat: &DestructurePattern, mode: ScanMode) -> Result<()> {
        match pat {
            DestructurePattern::Identifier(name, _) => self.check_binding_name(name, mode),
            DestructurePattern::Array(items) => {
                for item in items {
                    self.destructure_pattern_binding(item, mode)?;
                }
                Ok(())
            }
            DestructurePattern::Object(fields) => {
                for field in fields {
                    self.destructure_pattern_binding(&field.pattern, mode)?;
                }
                Ok(())
            }
            DestructurePattern::Rest(inner) => self.destructure_pattern_binding(inner, mode),
            DestructurePattern::Decomposition(bindings) => {
                for binding in bindings {
                    self.check_binding_name(&binding.name, mode)?;
                    self.type_annotation(&binding.type_annotation, mode)?;
                }
                Ok(())
            }
        }
    }

    fn destructure_pattern_assign_target(
        &self,
        pat: &DestructurePattern,
        mode: ScanMode,
    ) -> Result<()> {
        match pat {
            DestructurePattern::Identifier(name, _) => self.check_assign_target_name(name, mode),
            DestructurePattern::Array(items) => {
                for item in items {
                    self.destructure_pattern_assign_target(item, mode)?;
                }
                Ok(())
            }
            DestructurePattern::Object(fields) => {
                for field in fields {
                    self.destructure_pattern_assign_target(&field.pattern, mode)?;
                }
                Ok(())
            }
            DestructurePattern::Rest(inner) => self.destructure_pattern_assign_target(inner, mode),
            DestructurePattern::Decomposition(bindings) => {
                for binding in bindings {
                    self.check_assign_target_name(&binding.name, mode)?;
                    self.type_annotation(&binding.type_annotation, mode)?;
                }
                Ok(())
            }
        }
    }

    fn match_pattern(&self, pat: &Pattern, mode: ScanMode) -> Result<()> {
        match pat {
            Pattern::Identifier { name, .. } => self.check_binding_name(name, mode),
            Pattern::Typed {
                name,
                type_annotation,
                ..
            } => {
                self.check_binding_name(name, mode)?;
                self.type_annotation(type_annotation, mode)
            }
            Pattern::Literal(_) | Pattern::Wildcard => Ok(()),
            Pattern::Array(items) => {
                for item in items {
                    self.match_pattern(item, mode)?;
                }
                Ok(())
            }
            Pattern::Object(fields) => {
                for (_, field_pat) in fields {
                    self.match_pattern(field_pat, mode)?;
                }
                Ok(())
            }
            Pattern::Constructor { fields, .. } => match fields {
                PatternConstructorFields::Unit => Ok(()),
                PatternConstructorFields::Tuple(items) => {
                    for item in items {
                        self.match_pattern(item, mode)?;
                    }
                    Ok(())
                }
                PatternConstructorFields::Struct(entries) => {
                    for (_, field_pat) in entries {
                        self.match_pattern(field_pat, mode)?;
                    }
                    Ok(())
                }
            },
        }
    }

    // ---------------------------------------------------------------------
    // Type annotations (never rewritten — shared borrows suffice)
    // ---------------------------------------------------------------------

    /// A body-internal type annotation must not mention the template type
    /// parameter (it resolves away at specialization; there is no nameable
    /// type behind it).
    fn type_annotation(&self, annotation: &TypeAnnotation, mode: ScanMode) -> Result<()> {
        if self.annotation_mentions_type_param(annotation) {
            return Err(match mode {
                ScanMode::TemplateBody => self.reject_type_param_annotation(),
                ScanMode::ClosureInterior => self.reject_closure_occurrence(self.type_param),
            });
        }
        Ok(())
    }

    fn annotation_mentions_type_param(&self, annotation: &TypeAnnotation) -> bool {
        match annotation {
            TypeAnnotation::Basic(name) => name == self.type_param,
            TypeAnnotation::Array(inner) => self.annotation_mentions_type_param(inner),
            TypeAnnotation::Tuple(items)
            | TypeAnnotation::Union(items)
            | TypeAnnotation::Intersection(items) => items
                .iter()
                .any(|item| self.annotation_mentions_type_param(item)),
            TypeAnnotation::Object(fields) => fields
                .iter()
                .any(|field| self.annotation_mentions_type_param(&field.type_annotation)),
            TypeAnnotation::Function { params, returns } => {
                params
                    .iter()
                    .any(|param| self.annotation_mentions_type_param(&param.type_annotation))
                    || self.annotation_mentions_type_param(returns)
            }
            TypeAnnotation::Generic { name, args } => {
                (!name.is_qualified() && name.name() == self.type_param)
                    || args.iter().any(|arg| self.annotation_mentions_type_param(arg))
            }
            TypeAnnotation::Reference(path) => {
                !path.is_qualified() && path.name() == self.type_param
            }
            TypeAnnotation::Borrow { inner, .. } => self.annotation_mentions_type_param(inner),
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined => false,
            TypeAnnotation::Dyn(paths) => paths
                .iter()
                .any(|path| !path.is_qualified() && path.name() == self.type_param),
            TypeAnnotation::Existential { inner, .. } => {
                self.annotation_mentions_type_param(inner)
            }
        }
    }

    // ---------------------------------------------------------------------
    // Expressions
    // ---------------------------------------------------------------------

    fn opt_expr(&self, expr: Option<&mut Expr>, mode: ScanMode) -> Result<()> {
        match expr {
            Some(inner) => self.expr(inner, mode),
            None => Ok(()),
        }
    }

    fn exprs(&self, exprs: &mut [Expr], mode: ScanMode) -> Result<()> {
        for expr in exprs.iter_mut() {
            self.expr(expr, mode)?;
        }
        Ok(())
    }

    fn named_exprs(&self, entries: &mut [(String, Expr)], mode: ScanMode) -> Result<()> {
        for (_, value) in entries.iter_mut() {
            self.expr(value, mode)?;
        }
        Ok(())
    }

    /// The legal `args[<int literal>]` shape, checked when the receiver is
    /// the pseudo-tuple identifier in `TemplateBody` mode. Returns the
    /// constant index value, or the named rejection for slices and
    /// non-constant indices.
    fn args_index_access(&self, index: &Expr, end_index: Option<&Expr>) -> Result<i64> {
        if end_index.is_some() {
            return Err(self.reject_slice());
        }
        match index {
            Expr::Literal(Literal::Int(value), _) => Ok(*value),
            _ => Err(self.reject_non_constant_index()),
        }
    }

    /// Classify one expression node against the template-body pseudo-tuple
    /// surface. Shared by both faces: legality (and its named rejections) is
    /// decided HERE once; the faces differ only in keep-vs-replace.
    fn intercept_template_body_expr(&self, expr: &Expr) -> Result<Intercept> {
        match expr {
            Expr::PropertyAccess {
                object,
                property,
                optional,
                span,
            } if self.is_args_identifier(object) => {
                if property != "length" {
                    return Err(self.reject_other_property(property));
                }
                if *optional {
                    return Err(self.reject_optional_access());
                }
                Ok(match self.face {
                    Face::Validate => Intercept::LegalKeep,
                    Face::Rewrite { plan, .. } => Intercept::Replace(Expr::Literal(
                        Literal::Int(plan.arity() as i64),
                        *span,
                    )),
                    // Unreachable: the ambient/after-return faces run with
                    // unspellable sentinel template names, so
                    // `is_args_identifier` can never match; `No` keeps the
                    // generic traversal total.
                    Face::AmbientScope(_) | Face::AfterReturnScan { .. } => Intercept::No,
                })
            }
            Expr::IndexAccess {
                object,
                index,
                end_index,
                span,
            } if self.is_args_identifier(object) => {
                let slot_index = self.args_index_access(index, end_index.as_deref())?;
                Ok(match self.face {
                    Face::Validate => Intercept::LegalKeep,
                    Face::Rewrite { plan, .. } => Intercept::Replace(Expr::Identifier(
                        self.checked_slot_local(slot_index, plan)?,
                        *span,
                    )),
                    // Unreachable under the sentinel names (see above).
                    Face::AmbientScope(_) | Face::AfterReturnScan { .. } => Intercept::No,
                })
            }
            _ => Ok(Intercept::No),
        }
    }

    fn expr(&self, expr: &mut Expr, mode: ScanMode) -> Result<()> {
        // S5a: the pseudo-tuple surface is live only OUTSIDE comptime blocks
        // (Dec 65) — inside one, `args[i]` must not classify as a legal
        // runtime read; it falls through to the named Dec-65 rejection at
        // the identifier leaf.
        if mode == ScanMode::TemplateBody && self.pseudo_tuple_surface_live() {
            match self.intercept_template_body_expr(expr)? {
                Intercept::No => {}
                Intercept::LegalKeep => return Ok(()),
                Intercept::Replace(replacement) => {
                    *expr = replacement;
                    return Ok(());
                }
            }
        }
        match expr {
            // Leaves. FormattedString interpolation is not REWRITTEN by the
            // pseudo-tuple faces; the AMBIENT face scans it (the module
            // docs' named boundary + its S5 exception), and — S6 fixlet
            // round 4 — the SPECIALIZATION faces run the scan-only
            // interior-EXIT rejection (`?` / `return` inside an
            // interpolation interior compile in the specialized body's own
            // frame and would bypass the F1 arm and both exit gates; see
            // `scan_fstring_interior_exits` for the measured leak).
            Expr::Literal(lit, _) => {
                if let Literal::FormattedString { value, mode: interp_mode } = &*lit {
                    self.ambient_scan_fstring(value, *interp_mode, mode)?;
                    if mode == ScanMode::TemplateBody
                        && self.pseudo_tuple_surface_live()
                        && matches!(
                            self.face,
                            Face::Rewrite { .. } | Face::AfterReturnScan { .. }
                        )
                    {
                        self.scan_fstring_interior_exits(value, *interp_mode)?;
                    }
                }
                Ok(())
            }
            Expr::DataRef(_, _)
            | Expr::DataDateTimeRef(_, _)
            | Expr::TimeRef(_, _)
            | Expr::DateTime(_, _)
            | Expr::PatternRef(_, _)
            | Expr::Duration(_, _)
            | Expr::Continue(_)
            | Expr::Unit(_) => Ok(()),

            Expr::Identifier(name, _) => {
                self.check_reserved(name)?;
                // S5a Dec-65 arm: a template name read inside a comptime
                // block (checked BEFORE the bare-value rejection so the
                // sentence names the real boundary).
                self.check_comptime_runtime_input(name, mode)?;
                self.ambient_check_value_name(name)?;
                match mode {
                    ScanMode::TemplateBody => {
                        if name.as_str() == self.args_param {
                            // Bare `args` in a value position (the legal
                            // return/tail spellings are handled by the
                            // callers before recursion reaches here).
                            return Err(self.reject_bare_value());
                        }
                        Ok(())
                    }
                    ScanMode::ClosureInterior => {
                        if name.as_str() == self.args_param || name.as_str() == self.type_param {
                            return Err(self.reject_closure_occurrence(name));
                        }
                        Ok(())
                    }
                }
            }

            Expr::TypeSyntax(annotation, _) => self.type_annotation(annotation, mode),

            Expr::DataRelativeAccess { reference, .. } => self.expr(reference, mode),

            // The args-receiver cases were intercepted above (TemplateBody);
            // here only ordinary receivers (and closure-interior occurrences,
            // which reject at the Identifier leaf) remain.
            Expr::PropertyAccess { object, .. } => self.expr(object, mode),

            Expr::IndexAccess {
                object,
                index,
                end_index,
                span: _,
            } => {
                self.expr(object, mode)?;
                self.expr(index, mode)?;
                self.opt_expr(end_index.as_deref_mut(), mode)
            }

            Expr::BinaryOp { left, right, .. } => {
                self.expr(left, mode)?;
                self.expr(right, mode)
            }

            Expr::FuzzyComparison { left, right, .. } => {
                self.expr(left, mode)?;
                self.expr(right, mode)
            }

            Expr::UnaryOp { operand, .. } => self.expr(operand, mode),

            Expr::FunctionCall {
                name,
                const_args,
                args,
                named_args,
                span: _,
            } => {
                self.check_call_name(name, mode)?;
                self.ambient_check_call_name(name)?;
                self.exprs(const_args, mode)?;
                self.exprs(args, mode)?;
                self.named_exprs(named_args, mode)
            }

            Expr::QualifiedFunctionCall {
                namespace,
                function,
                const_args,
                args,
                named_args,
                span: _,
            } => {
                self.check_call_name(namespace, mode)?;
                self.check_call_name(function, mode)?;
                self.ambient_check_call_name(&format!("{namespace}::{function}"))?;
                self.exprs(const_args, mode)?;
                self.exprs(args, mode)?;
                self.named_exprs(named_args, mode)
            }

            Expr::EnumConstructor {
                enum_name,
                variant,
                payload,
                ..
            } => {
                // S5a ambient MUST-SCAN: a module-QUALIFIED value reference
                // (`defs::secret`) parses as a Unit-payload enum constructor;
                // when the joined path names a module-scope VALUE binding it
                // is exactly as ambient as a bare reference. Genuine enum
                // variants / comptime fields never live in `module_bindings`,
                // so they fall through untouched.
                if matches!(payload, EnumConstructorPayload::Unit) {
                    if let Some(ctx) = self.ambient() {
                        let joined = format!("{enum_name}::{variant}");
                        if !ctx.in_scope(&joined)
                            && ctx.compiler.module_bindings.contains_key(&joined)
                        {
                            return Err(ctx.ambient_rejection(&joined, &joined));
                        }
                    }
                }
                match payload {
                    EnumConstructorPayload::Unit => Ok(()),
                    EnumConstructorPayload::Tuple(items) => self.exprs(items, mode),
                    EnumConstructorPayload::Struct(fields) => self.named_exprs(fields, mode),
                }
            }

            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                span: _,
            } => {
                self.expr(condition, mode)?;
                self.expr(then_expr, mode)?;
                self.opt_expr(else_expr.as_deref_mut(), mode)
            }

            Expr::Object(entries, _) => {
                for entry in entries.iter_mut() {
                    match entry {
                        ObjectEntry::Field {
                            value,
                            type_annotation,
                            ..
                        } => {
                            if let Some(annotation) = type_annotation {
                                self.type_annotation(annotation, mode)?;
                            }
                            self.expr(value, mode)?;
                        }
                        ObjectEntry::Spread(inner) => self.expr(inner, mode)?,
                    }
                }
                Ok(())
            }

            Expr::Array(items, _) => self.exprs(items, mode),

            Expr::ListComprehension(comp, _) => self.ambient_frame(|| {
                for clause in &mut comp.clauses {
                    // Iterable BEFORE the clause pattern binds: a clause's
                    // iterable sees earlier clauses' binders but not its own
                    // (order load-bearing for the ambient face only).
                    self.expr(&mut clause.iterable, mode)?;
                    self.destructure_pattern_binding(&clause.pattern, mode)?;
                    if let Some(filter) = &mut clause.filter {
                        self.expr(filter, mode)?;
                    }
                }
                self.expr(&mut comp.element, mode)
            }),

            Expr::Block(block, _) => self.ambient_frame(|| {
                for item in &mut block.items {
                    match item {
                        BlockItem::VariableDecl(decl) => self.variable_decl(decl, mode)?,
                        BlockItem::Assignment(assign) => self.assignment(assign, mode)?,
                        BlockItem::Statement(stmt) => self.statement(stmt, mode)?,
                        BlockItem::Expression(inner) => self.expr(inner, mode)?,
                    }
                }
                Ok(())
            }),

            Expr::TypeAssertion {
                expr: inner,
                type_annotation,
                meta_param_overrides,
                span: _,
            } => {
                self.expr(inner, mode)?;
                self.type_annotation(type_annotation, mode)?;
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values_mut() {
                        self.expr(value, mode)?;
                    }
                }
                Ok(())
            }

            Expr::InstanceOf {
                expr: inner,
                type_annotation,
                span: _,
            } => {
                self.expr(inner, mode)?;
                self.type_annotation(type_annotation, mode)
            }

            // THE closure boundary: everything inside scans in
            // `ClosureInterior` mode — any occurrence of either name rejects.
            // The ambient face pushes a frame (closure params/captures are
            // closure-local binders) and keeps classifying inside — closure
            // interiors are a [C0926] MUST-SCAN position.
            Expr::FunctionExpr {
                params,
                return_type,
                body,
                captures,
                generated_origin: _,
                // C3-G12 nested-fn annotation carrier: annotation args are
                // application-site expressions, not template-body names.
                annotations: _,
                span: _,
            } => self.ambient_frame(|| {
                // Capture-clause entries are syntactic references into the
                // OUTER environment (captures.rs: resolved at the capture
                // gate against the construction scope), so the ambient face
                // classifies them BEFORE the closure's own binders register
                // (fix round 1, F2): an entry naming a module-scope value
                // binding is [C0926] in EVERY mode — `move` is C0906-gated
                // and the borrow spellings are C0902-gated downstream, but
                // `share` would silently wave the module value into a
                // template closure, the a6 hazard through a capture clause.
                if let Some(clause) = captures {
                    for entry in &clause.entries {
                        self.ambient_check_value_name(&entry.name)?;
                    }
                }
                for param in params.iter_mut() {
                    self.function_parameter(param, ScanMode::ClosureInterior)?;
                }
                if let Some(ret) = return_type {
                    self.type_annotation(ret, ScanMode::ClosureInterior)?;
                }
                if let Some(clause) = captures {
                    for entry in &clause.entries {
                        self.check_binding_name(&entry.name, ScanMode::ClosureInterior)?;
                    }
                }
                self.statements(body, ScanMode::ClosureInterior)
            }),

            Expr::Spread(inner, _) => self.expr(inner, mode),

            Expr::If(if_expr, _) => {
                self.expr(&mut if_expr.condition, mode)?;
                self.ambient_frame(|| self.expr(&mut if_expr.then_branch, mode))?;
                self.ambient_frame(|| self.opt_expr(if_expr.else_branch.as_deref_mut(), mode))
            }

            Expr::While(while_expr, _) => {
                self.expr(&mut while_expr.condition, mode)?;
                self.ambient_frame(|| self.expr(&mut while_expr.body, mode))
            }

            Expr::For(for_expr, _) => {
                // Iterable BEFORE the pattern binds (see the statement-form
                // For; order load-bearing for the ambient face only).
                self.expr(&mut for_expr.iterable, mode)?;
                self.ambient_frame(|| {
                    self.match_pattern(&for_expr.pattern, mode)?;
                    self.expr(&mut for_expr.body, mode)
                })
            }

            Expr::Loop(loop_expr, _) => {
                self.ambient_frame(|| self.expr(&mut loop_expr.body, mode))
            }

            Expr::Let(let_expr, _) => {
                if let Some(annotation) = &let_expr.type_annotation {
                    self.type_annotation(annotation, mode)?;
                }
                // Value BEFORE the pattern binds (sequential visibility).
                self.opt_expr(let_expr.value.as_deref_mut(), mode)?;
                self.ambient_frame(|| {
                    self.match_pattern(&let_expr.pattern, mode)?;
                    self.expr(&mut let_expr.body, mode)
                })
            }

            Expr::Assign(assign_expr, _) => {
                // `args[<int literal>] = expr` — the legal per-slot mutation
                // target. The target's own index legality is checked with the
                // same core as the read path; the rewrite face swaps the
                // target for the minted local. Not live inside comptime
                // blocks (Dec 65).
                if mode == ScanMode::TemplateBody && self.pseudo_tuple_surface_live() {
                    let intercepted = match assign_expr.target.as_ref() {
                        Expr::IndexAccess {
                            object,
                            index,
                            end_index,
                            span,
                        } if self.is_args_identifier(object) => {
                            Some((self.args_index_access(index, end_index.as_deref())?, *span))
                        }
                        _ => None,
                    };
                    if let Some((slot_index, span)) = intercepted {
                        if let Face::Rewrite {
                            plan,
                            carrier_writes,
                            ..
                        } = self.face
                        {
                            *assign_expr.target = Expr::Identifier(
                                self.checked_slot_local(slot_index, plan)?,
                                span,
                            );
                            // Rewrite the RHS FIRST so the collected write
                            // carries the post-rewrite expression (nested
                            // `args[j]` reads already resolved to their
                            // minted locals), then record it for the S2c
                            // aggregate-kind guard.
                            self.expr(&mut assign_expr.value, mode)?;
                            carrier_writes
                                .borrow_mut()
                                .push((slot_index as usize, (*assign_expr.value).clone()));
                            return Ok(());
                        }
                        return self.expr(&mut assign_expr.value, mode);
                    }
                }
                // S6: an expression-position whole-name assignment (`name =
                // e` reaching here as `Expr::Assign` rather than the
                // statement form) mutates the binding — record the target so
                // the provable-locals resolution excludes it. Compound
                // targets (index/property writes) do not rebind the local's
                // type and are not recorded.
                if let Expr::Identifier(name, _) = assign_expr.target.as_ref() {
                    self.note_assigned_local(name);
                }
                self.expr(&mut assign_expr.target, mode)?;
                self.expr(&mut assign_expr.value, mode)
            }

            Expr::Break(value, _) => self.opt_expr(value.as_deref_mut(), mode),

            Expr::Return(value, _) => {
                // The parser's expression-position `return` twin: the same
                // authored `return args` spelling (see the fn docs). Not
                // live inside comptime blocks (Dec 65).
                if mode == ScanMode::TemplateBody && self.pseudo_tuple_surface_live() {
                    if let Some(inner) = value.as_deref_mut() {
                        if self.is_args_identifier(inner) {
                            if let Face::Rewrite { plan, .. } = self.face {
                                let span = inner.span();
                                *inner = plan.carrier_expr(span);
                            }
                            return Ok(());
                        }
                    }
                }
                // S6 exit-gate collection: the expression-position `return`
                // twin records exactly like the statement form (body-level
                // only — the F2 filter; value walked first so the rewrite
                // face collects the post-rewrite expression — round 3, F3).
                self.opt_expr(value.as_deref_mut(), mode)?;
                self.note_return_value(value.as_deref(), mode);
                Ok(())
            }

            Expr::MethodCall {
                receiver,
                method,
                args,
                named_args,
                optional: _,
                span: _,
            } => {
                self.check_call_name(method, mode)?;
                self.expr(receiver, mode)?;
                self.exprs(args, mode)?;
                self.named_exprs(named_args, mode)
            }

            Expr::Match(match_expr, _) => {
                self.expr(&mut match_expr.scrutinee, mode)?;
                for arm in &mut match_expr.arms {
                    // One frame per arm: an arm's pattern binders are not
                    // visible in sibling arms (ambient face).
                    self.ambient_frame(|| {
                        self.match_pattern(&arm.pattern, mode)?;
                        if let Some(guard) = &mut arm.guard {
                            self.expr(guard, mode)?;
                        }
                        self.expr(&mut arm.body, mode)
                    })?;
                }
                Ok(())
            }

            Expr::Range { start, end, .. } => {
                self.opt_expr(start.as_deref_mut(), mode)?;
                self.opt_expr(end.as_deref_mut(), mode)
            }

            Expr::TimeframeContext { expr: inner, .. } => self.expr(inner, mode),

            Expr::TryOperator(inner, _) => {
                // ADR-009 C3 #14 (S6 fixlet round 2, F1): `?` compiles to an
                // UNCONDITIONAL early return of the propagated failure
                // carrier (`expressions/advanced.rs::compile_expr_try_operator`
                // — `TryUnwrap` returns from the whole frame on `Err`/`None`),
                // so a body-level `?` exits the specialized body OUTSIDE the
                // typed exit contract the gates prove — named rejection on
                // both specialization-time scans (Rewrite = before,
                // AfterReturnScan = after; see `reject_try_operator_exit`).
                // Closure interiors stay transparent (`?` there returns from
                // the CLOSURE frame) and so do comptime blocks (compile-time
                // evaluation, Dec 65) and the construction/ambient faces
                // (specialization is the total chokepoint — the S2c/after-
                // gate site rationale).
                if mode == ScanMode::TemplateBody
                    && self.pseudo_tuple_surface_live()
                    && matches!(
                        self.face,
                        Face::Rewrite { .. } | Face::AfterReturnScan { .. }
                    )
                {
                    return Err(self.reject_try_operator_exit());
                }
                self.expr(inner, mode)
            }

            Expr::UsingImpl { expr: inner, .. } => self.expr(inner, mode),

            Expr::SimulationCall { params, span: _, .. } => self.named_exprs(params, mode),

            Expr::WindowExpr(window, _) => self.window_expr(window, mode),

            Expr::FromQuery(query, _) => self.ambient_frame(|| {
                // Source BEFORE the query binder (sequential visibility).
                self.expr(&mut query.source, mode)?;
                self.check_binding_name(&query.variable, mode)?;
                for clause in &mut query.clauses {
                    match clause {
                        QueryClause::Where(cond) => self.expr(cond, mode)?,
                        QueryClause::OrderBy(specs) => {
                            for spec in specs.iter_mut() {
                                self.expr(&mut spec.key, mode)?;
                            }
                        }
                        QueryClause::GroupBy {
                            element,
                            key,
                            into_var,
                        } => {
                            self.expr(element, mode)?;
                            self.expr(key, mode)?;
                            if let Some(var) = into_var {
                                self.check_binding_name(var, mode)?;
                            }
                        }
                        QueryClause::Join {
                            variable,
                            source,
                            left_key,
                            right_key,
                            into_var,
                        } => {
                            self.check_binding_name(variable, mode)?;
                            self.expr(source, mode)?;
                            self.expr(left_key, mode)?;
                            self.expr(right_key, mode)?;
                            if let Some(var) = into_var {
                                self.check_binding_name(var, mode)?;
                            }
                        }
                        QueryClause::Let { variable, value } => {
                            // Value BEFORE the binder (sequential
                            // visibility for the ambient face).
                            self.expr(value, mode)?;
                            self.check_binding_name(variable, mode)?;
                        }
                    }
                }
                self.expr(&mut query.select, mode)
            }),

            Expr::StructLiteral { fields, .. } => self.named_exprs(fields, mode),

            Expr::Await(inner, _) => self.expr(inner, mode),

            Expr::Join(join, _) => {
                for branch in &mut join.branches {
                    for annotation in &mut branch.annotations {
                        self.annotation_args(annotation, mode)?;
                    }
                    self.expr(&mut branch.expr, mode)?;
                }
                Ok(())
            }

            Expr::Annotated {
                annotation, target, ..
            } => {
                self.annotation_args(annotation, mode)?;
                self.expr(target, mode)
            }

            Expr::AsyncLet(async_let, _) => {
                // Value BEFORE the binder (sequential visibility).
                self.expr(&mut async_let.expr, mode)?;
                self.check_binding_name(&async_let.name, mode)
            }

            Expr::AsyncScope(inner, _) => self.expr(inner, mode),

            Expr::Comptime(stmts, _) => {
                // S5a Dec-65 boundary: the pseudo-tuple surface is not live
                // inside a comptime block (see `comptime_depth`). The
                // ambient face still classifies inside — a module-binding
                // reference in a comptime block is ambient all the same.
                self.comptime_depth.set(self.comptime_depth.get() + 1);
                let result = self.ambient_frame(|| self.statements(stmts, mode));
                self.comptime_depth.set(self.comptime_depth.get() - 1);
                result
            }

            Expr::ComptimeFor(comp_for, _) => {
                // Iterable BEFORE the binders (sequential visibility).
                self.expr(&mut comp_for.iterable, mode)?;
                self.ambient_frame(|| {
                    for witness in &comp_for.witnesses {
                        self.check_binding_name(witness, mode)?;
                    }
                    self.check_binding_name(&comp_for.variable, mode)?;
                    self.statements(&mut comp_for.body, mode)
                })
            }

            Expr::Reference { expr: inner, .. } => self.expr(inner, mode),

            Expr::TableRows(rows, _) => {
                for row in rows.iter_mut() {
                    self.exprs(row, mode)?;
                }
                Ok(())
            }
        }
    }

    fn window_expr(&self, window: &mut WindowExpr, mode: ScanMode) -> Result<()> {
        match &mut window.function {
            WindowFunction::Lag { expr, default, .. }
            | WindowFunction::Lead { expr, default, .. } => {
                self.expr(expr, mode)?;
                self.opt_expr(default.as_deref_mut(), mode)?;
            }
            WindowFunction::RowNumber
            | WindowFunction::Rank
            | WindowFunction::DenseRank
            | WindowFunction::Ntile(_) => {}
            WindowFunction::FirstValue(expr)
            | WindowFunction::LastValue(expr)
            | WindowFunction::NthValue(expr, _)
            | WindowFunction::Sum(expr)
            | WindowFunction::Avg(expr)
            | WindowFunction::Min(expr)
            | WindowFunction::Max(expr) => self.expr(expr, mode)?,
            WindowFunction::Count(expr) => self.opt_expr(expr.as_deref_mut(), mode)?,
        }
        self.exprs(&mut window.over.partition_by, mode)?;
        if let Some(order_by) = &mut window.over.order_by {
            for (expr, _) in &mut order_by.columns {
                self.expr(expr, mode)?;
            }
        }
        // WindowFrame bounds carry no expressions (usize offsets only).
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_of(src: &str) -> FunctionDef {
        shape_ast::parse_program(src)
            .expect("fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Function(func, _) => Some(func),
                _ => None,
            })
            .expect("fixture has one function")
    }

    fn body_of(src: &str) -> Vec<Statement> {
        def_of(src).body
    }

    fn validate(src: &str) -> Result<()> {
        let mut body = body_of(src);
        validate_pseudo_tuple_uses(&mut body, "args", "Args")
    }

    fn expect_reject(src: &str, needle: &str) {
        let err = validate(src).expect_err("fixture must be rejected");
        assert!(
            err.to_string().contains(needle),
            "expected rejection containing {needle:?}, got: {err}"
        );
    }

    fn int_ann() -> TypeAnnotation {
        TypeAnnotation::Basic("int".into())
    }

    fn number_ann() -> TypeAnnotation {
        TypeAnnotation::Basic("number".into())
    }

    /// A 2-ary `(int, number)` aggregate plan — the mod.rs seam's shape.
    fn aggregate_plan() -> TemplateSpecializationPlan {
        TemplateSpecializationPlan {
            pseudo_tuple: Some(PseudoTuplePlan {
                args_param: "args".into(),
                type_param: "Args".into(),
                target_params: vec![("a".into(), int_ann()), ("b".into(), number_ann())],
                carrier: MutationCarrier::Aggregate {
                    fields: vec![("a0".into(), int_ann()), ("a1".into(), number_ann())],
                },
            }),
            captures: Vec::new(),
        }
    }

    /// A 1-ary `(int)` Single plan.
    fn single_plan() -> TemplateSpecializationPlan {
        TemplateSpecializationPlan {
            pseudo_tuple: Some(PseudoTuplePlan {
                args_param: "args".into(),
                type_param: "Args".into(),
                target_params: vec![("a".into(), int_ann())],
                carrier: MutationCarrier::Single {
                    annotation: int_ann(),
                },
            }),
            captures: Vec::new(),
        }
    }

    fn resolve(src: &str, plan: &TemplateSpecializationPlan) -> Result<FunctionDef> {
        let mut def = def_of(src);
        let compiler = crate::compiler::BytecodeCompiler::new();
        resolve_pseudo_tuple(&mut def, plan, &compiler)?;
        Ok(def)
    }

    fn expect_resolve_reject(src: &str, plan: &TemplateSpecializationPlan, needle: &str) {
        let err = resolve(src, plan).expect_err("fixture must be rejected by the rewrite face");
        assert!(
            err.to_string().contains(needle),
            "expected rewrite-face rejection containing {needle:?}, got: {err}"
        );
    }

    // =====================================================================
    // Validate face (construction-time) — behavior unchanged from S1b.
    // =====================================================================

    // LEGAL: the full pseudo-tuple surface — constant-index read, constant-
    // index mutation, `.length`, and the `return args` mutation-return.
    #[test]
    fn full_legal_surface_validates() {
        validate(
            r#"
fn t<Args>(args: Args) -> Args {
    args[0] = args[0] + 1
    let n = args.length
    if n > 1 {
        args[1] = 2
    }
    return args
}
"#,
        )
        .expect("the legal surface validates");
    }

    // LEGAL: a FINAL bare `args` expression statement is the implicit-return
    // tail spelling.
    #[test]
    fn final_bare_args_tail_is_legal() {
        validate(
            r#"
fn t<Args>(args: Args) -> Args {
    args[0] = 1
    args
}
"#,
        )
        .expect("final bare args tail is the implicit mutation-return");
    }

    // LEGAL: `return args` nested under control flow.
    #[test]
    fn return_args_inside_nested_block_is_legal() {
        validate(
            r#"
fn t<Args>(args: Args) -> Args {
    if args.length > 0 {
        return args
    }
    return args
}
"#,
        )
        .expect("nested return args is legal");
    }

    // LEGAL: constant-index reads compose in ordinary expressions.
    #[test]
    fn constant_index_reads_in_expressions_are_legal() {
        validate(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args[0] + args[1]
    args[0] = x
    return args
}
"#,
        )
        .expect("constant-index reads are legal");
    }

    // NEGATIVE: non-constant index.
    #[test]
    fn non_constant_index_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let i = 0
    args[i] = 1
    return args
}
"#,
            "compile-time-constant index",
        );
    }

    // NEGATIVE: slicing (`args[0..1]` parses to `IndexAccess` with a slice
    // end).
    #[test]
    fn slicing_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args[0..1]
    return args
}
"#,
            "cannot be sliced",
        );
    }

    // NEGATIVE: any property other than `length`.
    #[test]
    fn other_property_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args.first
    return args
}
"#,
            "has no property `first`",
        );
    }

    // NEGATIVE: bare `args` in a value position.
    #[test]
    fn bare_args_value_position_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args
    return args
}
"#,
            "no first-class value",
        );
    }

    // NEGATIVE: bare `args` as a NON-final expression statement is not the
    // tail spelling.
    #[test]
    fn bare_args_mid_body_expression_statement_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    args
    return args
}
"#,
            "no first-class value",
        );
    }

    // NEGATIVE: whole-pack assignment (`args = e`).
    #[test]
    fn whole_pack_assignment_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    args = 1
    return args
}
"#,
            "no first-class value",
        );
    }

    // NEGATIVE: the pseudo-tuple does not cross closure boundaries.
    #[test]
    fn args_inside_closure_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let f = |x| x + args[0]
    return args
}
"#,
            "does not cross closure boundaries",
        );
    }

    // NEGATIVE: the type parameter does not cross closure boundaries either.
    #[test]
    fn type_param_inside_closure_annotation_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let f = |x: Args| x
    return args
}
"#,
            "does not cross closure boundaries",
        );
    }

    // NEGATIVE: the type parameter in a body-internal annotation.
    #[test]
    fn type_param_in_body_annotation_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x: Args = 0
    return args
}
"#,
            "cannot appear in a body-internal type annotation",
        );
    }

    // NEGATIVE: the reserved `__c3_` prefix anywhere in the body.
    #[test]
    fn reserved_prefix_identifier_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let __c3_tmp = 1
    return args
}
"#,
            "reserved prefix `__c3_`",
        );
    }

    // NEGATIVE: rebinding the pseudo-tuple parameter name.
    #[test]
    fn rebinding_args_param_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let args = 1
    return args
}
"#,
            "cannot be rebound",
        );
    }

    // NEGATIVE (verify-1 fix): the pseudo-tuple spelling in FUNCTION-CALL
    // name position (`args(1)`) is not a call — the same bare-value
    // rejection class. Pre-fix this passed both faces and degraded to a
    // confusing downstream error (or a silent meaning shift onto a module
    // fn spelled `args`).
    #[test]
    fn args_as_function_call_name_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args(1)
    return args
}
"#,
            "no first-class value",
        );
    }

    // NEGATIVE (verify-1 fix): the same rejection for the METHOD-name
    // spelling (`y.args(0)`).
    #[test]
    fn args_as_method_name_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let y = [1]
    let x = y.args(0)
    return args
}
"#,
            "no first-class value",
        );
    }

    // NEGATIVE (verify-1 fix): the template type parameter as a call name.
    #[test]
    fn type_param_as_call_name_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = Args(1)
    return args
}
"#,
            "cannot be called",
        );
    }

    // NEGATIVE (verify-1 fix): call-name occurrences inside a closure use
    // the closure-occurrence sentence, same as every other occurrence.
    #[test]
    fn args_as_call_name_inside_closure_is_rejected() {
        expect_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let f = |x| args(x)
    return args
}
"#,
            "does not cross closure boundaries",
        );
    }

    // =====================================================================
    // Rewrite face (specialization-time) — the C3-G9 resolution.
    // =====================================================================

    // The full aggregate transform: params minted + typed, prologue
    // prepended, index read/write and `.length` resolved, the return
    // rewritten to the inline-schema object, and the return annotation
    // replaced (no Tuple survives).
    #[test]
    fn aggregate_resolution_transforms_the_full_surface() {
        let def = resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    args[0] = args[0] + 1
    let n = args.length
    return args
}
"#,
            &aggregate_plan(),
        )
        .expect("the legal surface resolves");

        // (2) minted, typed parameters.
        assert_eq!(def.params.len(), 2);
        assert_eq!(def.params[0].simple_name(), Some("__c3_p0"));
        assert_eq!(def.params[1].simple_name(), Some("__c3_p1"));
        assert_eq!(def.params[0].type_annotation, Some(int_ann()));
        assert_eq!(def.params[1].type_annotation, Some(number_ann()));

        // (3) the prologue: let mut __c3_arg_i = __c3_pi.
        for (index, stmt) in def.body[..2].iter().enumerate() {
            let Statement::VariableDecl(decl, _) = stmt else {
                panic!("expected prologue decl at {index}, got {stmt:?}");
            };
            assert_eq!(decl.kind, VarKind::Let);
            assert!(decl.is_mut, "prologue locals are uniformly mutable");
            assert_eq!(
                decl.pattern,
                DestructurePattern::Identifier(slot_local_name(index), Span::default())
            );
            assert_eq!(
                decl.value,
                Some(Expr::Identifier(slot_param_name(index), Span::default()))
            );
        }

        // (1) the per-slot assignment target and read both resolved.
        let Statement::Expression(Expr::Assign(assign, _), _) = &def.body[2] else {
            panic!("expected the rewritten assignment, got {:?}", def.body[2]);
        };
        assert!(
            matches!(assign.target.as_ref(), Expr::Identifier(name, _) if name == "__c3_arg_0"),
            "assign target must resolve to the minted local: {:?}",
            assign.target
        );
        assert!(
            format!("{:?}", assign.value).contains("__c3_arg_0"),
            "the read side must resolve to the minted local: {:?}",
            assign.value
        );

        // `.length` → the arity constant.
        let Statement::VariableDecl(n_decl, _) = &def.body[3] else {
            panic!("expected the length decl, got {:?}", def.body[3]);
        };
        assert!(
            matches!(n_decl.value, Some(Expr::Literal(Literal::Int(2), _))),
            "args.length must resolve to the constant target arity: {:?}",
            n_decl.value
        );

        // The mutation-return → the aggregate object literal.
        let Statement::Return(Some(Expr::Object(entries, _)), _) = &def.body[4] else {
            panic!("expected the aggregate return, got {:?}", def.body[4]);
        };
        assert_eq!(entries.len(), 2);
        for (index, entry) in entries.iter().enumerate() {
            let ObjectEntry::Field { key, value, .. } = entry else {
                panic!("expected a plain field, got {entry:?}");
            };
            assert_eq!(key, &format!("a{index}"));
            assert!(
                matches!(value, Expr::Identifier(name, _) if name == &slot_local_name(index))
            );
        }

        // (4) the return annotation is the fully-typed inline object schema —
        // the transient Tuple annotation never survives resolution.
        let Some(TypeAnnotation::Object(fields)) = &def.return_type else {
            panic!("expected the object return annotation, got {:?}", def.return_type);
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "a0");
        assert_eq!(fields[0].type_annotation, int_ann());
        assert_eq!(fields[1].name, "a1");
        assert_eq!(fields[1].type_annotation, number_ann());
        assert!(
            !matches!(def.return_type, Some(TypeAnnotation::Tuple(_))),
            "no Tuple annotation may survive resolution"
        );
        assert!(def.type_params.is_none(), "the resolved def is concrete");
    }

    // Single-carrier resolution: the mutation-return (here the bare-args
    // TAIL spelling) becomes the bare minted local; the return annotation is
    // the target's one parameter type.
    #[test]
    fn single_resolution_rewrites_tail_to_the_minted_local() {
        let def = resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    args[0] = args[0] * 3
    args
}
"#,
            &single_plan(),
        )
        .expect("the single-carrier surface resolves");

        assert_eq!(def.params.len(), 1);
        assert_eq!(def.params[0].simple_name(), Some("__c3_p0"));
        let Statement::Expression(tail, _) = def.body.last().expect("body has a tail") else {
            panic!("expected the rewritten tail expression");
        };
        assert!(
            matches!(tail, Expr::Identifier(name, _) if name == "__c3_arg_0"),
            "the bare-args tail must resolve to the minted local: {tail:?}"
        );
        assert_eq!(def.return_type, Some(int_ann()));
    }

    // NEGATIVE (rewrite face): an out-of-range constant READ quotes the
    // index and the target's arity + signature.
    #[test]
    fn out_of_range_read_is_rejected_naming_index_and_arity() {
        let err = resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args[7]
    return args
}
"#,
            &aggregate_plan(),
        )
        .expect_err("an out-of-range index must be rejected at specialization");
        let message = err.to_string();
        assert!(
            message.contains("index 7 is out of range"),
            "must quote the index: {message}"
        );
        assert!(
            message.contains("declares 2 parameters"),
            "must quote the target arity: {message}"
        );
        assert!(
            message.contains("a: int, b: number"),
            "must quote the target signature: {message}"
        );
    }

    // NEGATIVE (rewrite face): the same out-of-range rule on the ASSIGNMENT
    // target.
    #[test]
    fn out_of_range_assignment_target_is_rejected() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    args[2] = 1
    return args
}
"#,
            &aggregate_plan(),
            "index 2 is out of range",
        );
    }

    // FAIL-CLOSED (rewrite face): the shared core fires the SAME named
    // rejections for shapes construction already rejects — a non-constant
    // index …
    #[test]
    fn rewrite_face_rejects_non_constant_index_via_the_shared_core() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let i = 0
    args[i] = 1
    return args
}
"#,
            &aggregate_plan(),
            "compile-time-constant index",
        );
    }

    // … and a closure-interior occurrence.
    #[test]
    fn rewrite_face_rejects_closure_occurrence_via_the_shared_core() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let f = |x| x + args[0]
    return args
}
"#,
            &aggregate_plan(),
            "does not cross closure boundaries",
        );
    }

    // … and the call-name misuse (verify-1 fix): the SAME shared core fires
    // the bare-value rejection on the rewrite face too.
    #[test]
    fn rewrite_face_rejects_args_call_name_via_the_shared_core() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = args(1)
    return args
}
"#,
            &aggregate_plan(),
            "no first-class value",
        );
    }

    // ── S6 fixlet round 2, F1: the body-level `?`-exit arm ────────────────

    // NEGATIVE (rewrite face = the before side): a body-level `?` early-
    // returns the propagated failure carrier past the typed args-pack exit
    // (round-2 probe: the Err carrier consumed as the weave aggregate
    // silently corrupted the woven call — `1` where `8`). Named rejection.
    #[test]
    fn rewrite_face_rejects_body_level_try_operator() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = helper()?
    return args
}
"#,
            &aggregate_plan(),
            "the `?` operator cannot be used in a `before` template body",
        );
    }

    // POSITIVE twin of the arm's closure filter: a `?` INSIDE a closure
    // returns from the closure frame, not the template body — transparent
    // on the rewrite face (the closure itself touches no pseudo-tuple name).
    #[test]
    fn try_operator_inside_a_closure_is_transparent_on_the_rewrite_face() {
        resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    let f = |x: int| { helper(x)? }
    return args
}
"#,
            &aggregate_plan(),
        )
        .expect("a closure-interior `?` is not a template-body exit");
    }

    // DELIBERATE SITE CHOICE (pinned): construction (the validate face)
    // stays permissive for `?` — specialization is the total chokepoint
    // (every install passes the specialization-time scans; the S2c/after-
    // gate site rationale), so the named rejection fires there, with the
    // two-signature application-site attribution.
    #[test]
    fn try_operator_is_construction_transparent() {
        validate(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = helper()?
    return args
}
"#,
        )
        .expect("construction stays permissive; the specialization scans own the rejection");
    }

    // ── S6 fixlet round 4: interpolation-interior exit rejection ──────────

    // MUST-REJECT: a `?` hidden inside an f-string interpolation interior.
    // The interior compiles in the specialized body's own frame
    // (string_interpolation.rs), so the `?` early-returns the Err carrier
    // past the F1 arm and the before-side exit gate — the interior is raw
    // text to the walker (the F5 boundary the round-2 lens traced as the
    // one remaining bypass). Named rejection during the walk.
    #[test]
    fn rewrite_face_rejects_try_operator_inside_fstring_interior() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = f"{helper()?}"
    return args
}
"#,
            &aggregate_plan(),
            "the `?` operator cannot be used inside an f-string interpolation in a `before` \
             template body",
        );
    }

    // MUST-REJECT: a `return` hidden inside an interpolation interior exits
    // the specialized body from inside the string build — same bypass,
    // second exit construct.
    #[test]
    fn rewrite_face_rejects_return_inside_fstring_interior() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = f"{return 1}"
    return args
}
"#,
            &aggregate_plan(),
            "a `return` cannot be used inside an f-string interpolation in a `before` \
             template body",
        );
    }

    // POSITIVE twin of the scan's closure boundary: a `?` inside a CLOSURE
    // inside the interior exits the closure frame, not the specialized body
    // — the scan prunes closure interiors exactly like the F1 arm.
    #[test]
    fn try_operator_in_closure_inside_fstring_interior_is_transparent() {
        resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    let x = f"{run(|y: int| { helper(y)? })}"
    return args
}
"#,
            &aggregate_plan(),
        )
        .expect("a closure-interior `?` inside an interpolation is not a template-body exit");
    }

    // ── S6 fixlet round 3, F3: the BEFORE-side exit gate ──────────────────

    // MUST-REJECT (value-less completion, branch decomposition): a tail `if`
    // whose only exit is a branch-local `return args` falls through with NO
    // pack on the missing-else path — the woven typed local would read
    // garbage. Named rejection with the observer positive twin in the
    // sentence.
    #[test]
    fn before_exit_gate_value_less_missing_else_is_rejected() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    if args.length > 1 {
        return args
    }
}
"#,
            &aggregate_plan(),
            "can complete without delivering the mutated argument pack",
        );
    }

    // MUST-REJECT (value-less completion, declaration tail): the pure-read
    // body with no mutation-return at all — the exact sugar-verbatim shape
    // the order traced (`before(args) { let v = args[0] }`).
    #[test]
    fn before_exit_gate_value_less_decl_tail_is_rejected() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    let v = args[0]
}
"#,
            &single_plan(),
            "can complete without delivering the mutated argument pack",
        );
    }

    // MUST-REJECT (pack-less bare `return`): a value-less `return` is an
    // exit that delivers nothing — same rejection family.
    #[test]
    fn before_exit_gate_bare_return_is_rejected() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    if args.length > 1 {
        return
    }
    return args
}
"#,
            &aggregate_plan(),
            "can complete without delivering the mutated argument pack",
        );
    }

    // MUST-REJECT (stray value tail on the AGGREGATE carrier): a provable
    // non-pack tail (`args[0] + 1` → int) can never be the compiler-internal
    // aggregate the weave reads back per-field.
    #[test]
    fn before_exit_gate_stray_value_tail_rejects_on_the_aggregate_carrier() {
        let err = resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    args[0] = args[0] + 1
    args[0] + 1
}
"#,
            &aggregate_plan(),
        )
        .expect_err("a stray non-pack tail must be rejected by the exit gate");
        let message = err.to_string();
        assert!(
            message.contains("delivers `int` at an exit"),
            "names the proven kind: {message}"
        );
        assert!(
            message.contains("compiler-internal argument aggregate over (a: int, b: number)"),
            "names the bound carrier: {message}"
        );
    }

    // MUST-REJECT (divergent `return` on the SINGLE carrier): a body-level
    // `return "boom"` proves string against the bound int carrier.
    #[test]
    fn before_exit_gate_divergent_return_rejects_on_the_single_carrier() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    if args.length > 0 {
        return "boom"
    }
    return args
}
"#,
            &single_plan(),
            "delivers `string` at an exit",
        );
    }

    // MUST-REJECT (unprovable exit): an exit whose type the bounded
    // evaluator cannot prove is a LOUD rejection, never a guess.
    #[test]
    fn before_exit_gate_unprovable_exit_is_rejected() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    return mystery()
}
"#,
            &single_plan(),
            "cannot prove the type of a value the template body delivers",
        );
    }

    // POSITIVE TWIN (type-proof, not spelling police): a NON-canonical exit
    // that PROVES the Single carrier type is legal — `return args[0] * 2`
    // on a 1-ary int target delivered the doubled arg pre-gate (type-sound,
    // previously working) and still resolves.
    #[test]
    fn before_exit_gate_conforming_value_exit_is_legal_on_the_single_carrier() {
        let def = resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    return args[0] * 2
}
"#,
            &single_plan(),
        )
        .expect("a carrier-proving value exit is legal");
        assert_eq!(def.return_type, Some(int_ann()));
    }

    // MUST-REJECT (per-field proof on a user-spelled aggregate literal): an
    // object literal positionally matching the carrier shape is proven
    // FIELD BY FIELD — `a1` delivering an int slot where the target's `b`
    // declares number is a kind-divergent field, named as such.
    #[test]
    fn before_exit_gate_user_spelled_aggregate_literal_proves_per_field() {
        expect_resolve_reject(
            r#"
fn t<Args>(args: Args) -> Args {
    return { a0: args[0], a1: args[0] }
}
"#,
            &aggregate_plan(),
            "the pack value delivered for field a1",
        );
    }

    // POSITIVE TWIN (canonical spellings yield ZERO leaves): both
    // mutation-return spellings stay green under the gate — pinned by the
    // existing resolve-face battery (`aggregate_resolution_transforms_the_
    // full_surface`, `single_resolution_rewrites_tail_to_the_minted_local`),
    // whose minted carrier exits the gate proves by construction. This twin
    // pins the branch-conditional shape beside the missing-else rejection
    // above: the SAME body WITH the else-side pack delivery resolves.
    #[test]
    fn before_exit_gate_branchy_mutation_returns_stay_green() {
        resolve(
            r#"
fn t<Args>(args: Args) -> Args {
    if args.length > 1 {
        return args
    }
    return args
}
"#,
            &aggregate_plan(),
        )
        .expect("pack delivery on every path resolves");
    }

    // INTERNAL INVARIANT: a def that does not carry exactly the pseudo-tuple
    // parameter is an internal error (the classifier guarantees the shape).
    #[test]
    fn resolve_on_a_mismatched_def_is_an_internal_error() {
        let mut def = def_of("fn t(x: int, y: int) -> int { return x }");
        let compiler = crate::compiler::BytecodeCompiler::new();
        let err = resolve_pseudo_tuple(&mut def, &aggregate_plan(), &compiler)
            .expect_err("a mismatched def must be an internal error");
        assert!(
            err.to_string().contains("internal error"),
            "expected the named internal invariant error: {err}"
        );
    }
}
