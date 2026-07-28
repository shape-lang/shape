//! ADR-009 C3 #14 (slice 2, S2b) — the hook-template INSTALL registry + the
//! pass-2 apply seam.
//!
//! # The apply seam (`BytecodeCompiler::apply_install_hook_template`)
//!
//! The `ComptimeDirective::InstallHookTemplate` consumer for the authoritative
//! pass-2 function-target phase (`process_comptime_directives_for_function`).
//! Handle resolution is a BATCH SNAPSHOT taken at the TOP of that directive
//! loop, before ANY directive applies
//! ([`BytecodeCompiler::snapshot_install_hook_template_handles`] — fix-round-1):
//! applying an earlier directive can trigger a NESTED annotation-handler run
//! (a polymorphic `specialize_template` rides the monomorphization pipeline
//! into the full `compile_function`, which re-enters
//! `execute_comptime_handlers` because annotations survive substitution; an
//! `ExtendItems` compile does the same), and the nested run CLEARS +
//! REPOPULATES the per-run execute-populated stores — so a LATER install's
//! lazily resolved index would read across store generations (a miss
//! misdiagnosed as the internal lifecycle error, or — worse — a repopulated
//! store resolving the stale index to the NESTED run's template and silently
//! installing the WRONG one). The snapshot extends `take_comptime_directives`'
//! value-snapshot discipline to the handles the directives carry. Per install,
//! IN ORDER: the C3-G8 generic-target named rejection →
//! [`super::SpecializationTarget`] glue → `specialize_template`
//! (S3b: the capture VALUES flow in — rule-6 identity + the const_lift BAKE;
//! no separate delivery-plan step exists) → stage the install on the
//! caller's per-target accumulator + write one journaled registry row.
//!
//! # Transaction composition (E1-D6b — why the precondition holds)
//!
//! `specialize_template`'s first statement REQUIRES the already-open C2
//! `InstallTransaction` (`template_specialization/mod.rs:221-229`). At every
//! production reach of this seam the journal is open BY CONSTRUCTION:
//! `compile_in_place` (`compiler_impl_reference_model.rs:1985-1996`) runs
//! `begin_checked_body_install()` at `:1986` BEFORE `compile_in_place_inner`
//! at `:1987`, and every annotation handler (and therefore every directive
//! consumer) executes inside that inner driver — never a second transaction.
//!
//! # The registry (the S8 hover/query substrate)
//!
//! `BytecodeCompiler::hook_install_registry` is a compiler-owned table (the
//! C1 slice-4 `generated_symbol_query` precedent: tooling reads compiler
//! query state, never a text scan). One row per applied install: annotation
//! name, target name, hook kind, the template's declared-Sig rendering, the
//! specialized symbol + function index, the capture names with their
//! `LiftedConst` renderings, and the `@application` span. Rows are written at
//! apply time and JOURNALED through the open `InstallTransaction`
//! (displaced-entry undo per the `checked_body/journal.rs` precedent —
//! `journal_record_hook_install_row` records the pre-write length; rollback
//! truncates back), so a failing compile leaves NO row: the registry rows are
//! transaction-scoped exactly like every other install publication.
//!
//! # Staging, not weaving (S2b stops here)
//!
//! [`StagedHookInstall`]s ACCUMULATE per target across its annotations on a
//! parameter-threaded accumulator (the `pending_original_body_shadow`
//! pattern — parameter, never ambient state). The weave materializes ONCE
//! after the last handler, wrapping the final (possibly replace-body-edited)
//! def in application order — that materialization is S2c; this stage stops
//! at staged installs + registry rows.

use shape_ast::ast::{FunctionDef, Span};
use shape_ast::error::{Result, ShapeError};

use super::{
    SpecializedHandler, TemplateBodyOrigin, render_required_specialization_signature,
    render_template_declared_signature,
};
use crate::compiler::BytecodeCompiler;
use crate::compiler::comptime_builtins::expansion_provenance::ExpansionSite;
use crate::compiler::comptime_builtins::{
    BoundTemplate, ComptimeDirective, comptime_hook_template_at,
};
use crate::compiler::comptime_fragments::checked_template::TemplateHookKind;
use crate::compiler::comptime_target::type_annotation_to_string;

/// One applied hook-template install, staged for the S2c weave. Accumulated
/// per target across its annotations in APPLICATION ORDER (the order the
/// weave wraps in), threaded as a parameter like
/// `pending_original_body_shadow` — never ambient compiler state.
#[derive(Debug)]
pub(in crate::compiler) struct StagedHookInstall {
    /// The specialized handler (function index + mutation carrier). S3b:
    /// capture values are BAKED into the specialized handler
    /// (`const_lift::bake_captures_into_def`) — no capture delivery plan
    /// exists; handler arity == Sig arity everywhere.
    pub(in crate::compiler) handler: SpecializedHandler,
    /// Before or After — which side of the target the weave attaches to.
    pub(in crate::compiler) hook_kind: TemplateHookKind,
    /// The installing annotation's name (origin, parameter-threaded).
    pub(in crate::compiler) annotation_name: String,
    /// The `@application` anchor (origin, parameter-threaded).
    pub(in crate::compiler) application_span: Span,
    /// The handler application's `ExpansionSite` (origin, parameter-threaded)
    /// — the S2c weave derives the shadow reservation's `GeneratedOrigin` +
    /// anchors from the FIRST staged install's site (the `CheckedReplaceBody`
    /// shadow-construction precedent).
    pub(in crate::compiler) site: ExpansionSite,
}

/// One row of the hook-install registry (module docs above): the S8
/// hover/query substrate and the sugar-matrix hover row. Renderings are
/// captured at apply time so the query surface never re-derives them.
#[derive(Debug, Clone)]
pub(in crate::compiler) struct HookInstallRecord {
    /// The installing annotation's name.
    pub(in crate::compiler) annotation_name: String,
    /// The target function's user-spelled name.
    pub(in crate::compiler) target_name: String,
    /// Before or After.
    pub(in crate::compiler) hook_kind: TemplateHookKind,
    /// The template body fn's registered name — IDENTITY, never display:
    /// for sugar-minted bodies this is the `\u{1}`-prefixed hygienic mint.
    /// The S8c projection derives display through the ONE origin producer
    /// (`BytecodeCompiler::template_body_origin`).
    pub(in crate::compiler) body_fn: String,
    /// The template's declared-Sig rendering
    /// (`render_template_declared_signature`, prefixed by the body fn name).
    pub(in crate::compiler) template_sig: String,
    /// The APPLICATION-view specialization signature (the C3-G10
    /// two-signature attribution renderer,
    /// `render_required_specialization_signature` — user spellings, the
    /// delimited types the r8 sugar-matrix row pins). Captured at apply
    /// time like every other rendering.
    pub(in crate::compiler) specialized_sig: String,
    /// The specialized handler's registered symbol (the definition-compiled
    /// body fn for concrete templates; the suffixed unspellable
    /// specialization symbol for polymorphic ones).
    pub(in crate::compiler) specialized_symbol: String,
    /// The specialized handler's function index.
    pub(in crate::compiler) function_index: u16,
    /// Capture names with their `LiftedConst` renderings, in the body fn's
    /// trailing-parameter (delivery) order.
    pub(in crate::compiler) captures: Vec<(String, String)>,
    /// The `@application` span the install anchors at.
    pub(in crate::compiler) application_span: Span,
}

impl BytecodeCompiler {
    /// Fix-round-1: the BATCH handle snapshot — resolve EVERY
    /// `InstallHookTemplate` directive's per-run store index to its
    /// [`BoundTemplate`] up front, before any directive of the run applies
    /// (module docs: nested handler runs triggered by directive application
    /// clear + repopulate the execute-populated stores, so lazy per-directive
    /// resolution can cross store generations — missing handles misdiagnosed
    /// as the lifecycle internal error, or a stale index SILENTLY resolving
    /// to the nested run's template). Returns the resolved templates in
    /// DIRECTIVE ORDER; the consumer pops one per `InstallHookTemplate` arm.
    /// A miss here is still a broken lifecycle invariant —
    /// internal-error-shaped, never a user rejection (a handle can only be
    /// minted by `before_hook`/`after_hook` with the just-pushed index).
    pub(in crate::compiler) fn snapshot_install_hook_template_handles(
        &self,
        directives: &[ComptimeDirective],
        site: &ExpansionSite,
    ) -> Result<std::collections::VecDeque<BoundTemplate>> {
        let mut resolved = std::collections::VecDeque::new();
        for directive in directives {
            let ComptimeDirective::InstallHookTemplate { template_index } = directive else {
                continue;
            };
            let Some(bound) = comptime_hook_template_at(*template_index) else {
                return Err(ShapeError::RuntimeError {
                    message: format!(
                        "internal error: install directive template index {template_index} is \
                         not live in this comptime execution (the hook-template store is \
                         per-run and handles are snapshot-resolved before any directive \
                         applies — see the COMPTIME_HOOK_TEMPLATES lifecycle doc)"
                    ),
                    location: Some(self.span_to_source_location(site.application_span())),
                });
            };
            resolved.push_back(bound);
        }
        Ok(resolved)
    }

    /// The pass-2 apply seam for one `InstallHookTemplate` directive (module
    /// docs above for the ordered sequence + the transaction-composition
    /// chain). `bound` is the SNAPSHOT-resolved template
    /// ([`Self::snapshot_install_hook_template_handles`] — never a live store
    /// read here, fix-round-1); `annotation_name` + `site` are the
    /// parameter-threaded origin; `staged` is the caller's per-target
    /// accumulator.
    pub(in crate::compiler) fn apply_install_hook_template(
        &mut self,
        bound: &BoundTemplate,
        annotation_name: &str,
        func_def: &FunctionDef,
        site: &ExpansionSite,
        staged: &mut Vec<StagedHookInstall>,
    ) -> Result<()> {
        let application_span = site.application_span();

        // 2. C3-G8: installing on a GENERIC target is a named rejection at
        //    the `@application` site — a deliberate capability withdrawal
        //    (C3-G11), re-armed when #59 (monomorphization-origin re-arm)
        //    lands. The defections.md withdrawal entry lands with S5's
        //    rejection matrix (C3-G11 obligation).
        //
        //    MEASURED reach (disclosed): a generic def's pass-2 body compile
        //    is SKIPPED (`functions.rs` compile_function_with_generated_origin
        //    — "Skip compiling bodies of generic extend methods"), so the
        //    PRIMARY G8 firing site is the analysis pre-pass consumer
        //    (`apply_signature_directives_to_analysis_function`), the only
        //    consumer that observes the generic def's real `type_params`.
        //    This seam keeps two defensive twins of the same rejection:
        //    (a) the direct `type_params` check, should a generic def ever
        //    reach pass-2; (b) the specialization-origin check below — a
        //    monomorphized specialization carries the ORIGINAL's annotations
        //    with `type_params` cleared and a `{original}::{mono_key}` name
        //    (substitution.rs::substitute_function_def), so installing onto
        //    it IS installing onto the generic target per specialization,
        //    exactly what C3-G8 withdraws until #59.
        if func_def
            .type_params
            .as_ref()
            .is_some_and(|params| !params.is_empty())
        {
            return Err(ShapeError::SemanticError {
                message: generic_target_install_rejection_message(
                    bound.template.body_fn(),
                    annotation_name,
                    func_def,
                ),
                location: Some(self.span_to_source_location(application_span)),
            });
        }
        if let Some(original) = self.generic_origin_of_specialized_name(&func_def.name) {
            let original = original.clone();
            return Err(ShapeError::SemanticError {
                message: generic_target_install_rejection_message(
                    bound.template.body_fn(),
                    annotation_name,
                    &original,
                ),
                location: Some(self.span_to_source_location(application_span)),
            });
        }

        // (The former step 3 — the C3-G7 transitional mixed-legacy rejection,
        // one weave owner per target — was deleted WITH the legacy machinery
        // at the S6 capstone: the state it rejected is unconstructible now
        // that the typed weave is the only weave.)

        // 3./4. Target glue + specialization through the ONE open transaction
        //    (module docs: the journal is open by construction — E1-D6b). The
        //    descriptor slot is `None`: the structural comparison path is
        //    complete; the frozen-identity fast path stays available to
        //    callers that already hold a `CallableDescriptor` (identity-only,
        //    slice-0 §7.4).
        let target = self.specialization_target_from_def(func_def, None, application_span)?;
        // 4./5. Specialization + capture delivery are ONE step at S3b: the
        //    capture values flow into the specialization plan (rule-6
        //    identity + the bake) — the S2 bind-a-call-site-plan step is
        //    deleted with `CaptureBindingPlan`.
        let handler = self.specialize_template(&bound.template, &bound.capture_values, &target)?;

        // 7. One journaled registry row (displaced-entry undo: the journal
        //    records the pre-write length; rollback truncates back, so a
        //    failing compile leaves NO row).
        self.journal_record_hook_install_row();
        let specialized_symbol = self
            .program
            .functions
            .get(handler.function_index() as usize)
            .map(|function| function.name.clone())
            .unwrap_or_else(|| bound.template.body_fn().to_string());
        self.hook_install_registry.push(HookInstallRecord {
            annotation_name: annotation_name.to_string(),
            target_name: func_def.name.clone(),
            hook_kind: bound.template.hook_kind(),
            body_fn: bound.template.body_fn().to_string(),
            template_sig: format!(
                "{} {}",
                bound.template.body_fn(),
                render_template_declared_signature(&bound.template)
            ),
            specialized_sig: render_required_specialization_signature(
                bound.template.hook_kind(),
                &target,
            ),
            specialized_symbol,
            function_index: handler.function_index(),
            captures: rendered_captures(bound),
            application_span,
        });

        // 8. Stage for the S2c weave (application order = accumulator order).
        staged.push(StagedHookInstall {
            handler,
            hook_kind: bound.template.hook_kind(),
            annotation_name: annotation_name.to_string(),
            application_span,
            site: site.clone(),
        });
        Ok(())
    }

    /// The C3-G8 specialization-origin check: a monomorphized specialization
    /// is renamed `{original}::{mono_key}[::{semantic_suffix}...]`
    /// (`substitution.rs::substitute_function_def` + the semantic rename
    /// seam), so walk the `::` boundaries from the LONGEST prefix down and
    /// return the first registered def that is EXPLICITLY generic — that def
    /// is the generic target the install would silently attach to per
    /// specialization. A registered non-generic prefix (a genuinely
    /// module-qualified concrete fn) keeps walking and ultimately returns
    /// `None` — concrete fns never take the specialization rename.
    ///
    /// NAMED EDGE (fix-round-1 F3, surfaced not fixed): the walk cannot
    /// distinguish mono-rename suffixes from module qualification, so a
    /// CONCRETE module-qualified fn `foo::bar` whose module name collides
    /// with a registered TOP-LEVEL GENERIC fn `foo<T>` would false-positive
    /// the G8 rejection (LOUD, wrongly-attributed sentence naming `foo` —
    /// fail-closed, never a silent accept). Whether a module and a generic
    /// fn can share a name in one compilation unit is unverified; if it
    /// can, the fix is teaching the walk that mono-rename segments are
    /// mono-keys, not arbitrary idents. Tracked in the slice-2 fix-round-1
    /// report residuals.
    fn generic_origin_of_specialized_name(&self, specialized_name: &str) -> Option<&FunctionDef> {
        let mut end = specialized_name.len();
        while let Some(pos) = specialized_name[..end].rfind("::") {
            let prefix = &specialized_name[..pos];
            if let Some(original) = self.function_defs.get(prefix) {
                if original
                    .type_params
                    .as_ref()
                    .is_some_and(|params| !params.is_empty())
                {
                    return Some(original);
                }
            }
            end = pos;
        }
        None
    }

    /// Discard the registry rows staged for `target_name` when a
    /// `RemoveTarget` directive removes the function mid-handler-chain: a
    /// removed target installs no weave, so its rows would misreport an
    /// install that never materializes. The journaled length-undo stays
    /// correct (rollback truncation to a shorter prior length is unaffected
    /// by rows removed here; on commit the retained rows are exactly the
    /// live installs).
    pub(in crate::compiler) fn discard_hook_install_rows_for_target(&mut self, target_name: &str) {
        self.hook_install_registry
            .retain(|row| row.target_name != target_name);
    }
}

/// ADR-009 C3 #14 (S8c) — the PUBLIC, display-safe projection of one
/// hook-install registry row: the shared query surface LSP hover reads (the
/// C1 slice-4 `generated_symbol_query` precedent — tooling reads the
/// compiler-owned registry through this projection, never a text scan and
/// never a hand-written parallel table).
///
/// Every string field is DISPLAY-SAFE: the sugar mint's `\u{1}`-hygienic
/// body-fn name and the specialization symbol's unspellable marks never
/// appear (machine-pinned in this module's tests). Sugar-origin rows derive
/// `origin` through the ONE producer
/// (`BytecodeCompiler::template_body_origin`); API-body rows carry their
/// real fn name in both `origin` and `body_fn`.
#[derive(Debug, Clone)]
pub struct HookInstallView {
    /// The installing annotation's name.
    pub annotation_name: String,
    /// The target function's user-spelled name.
    pub target_name: String,
    /// `"before"` or `"after"`.
    pub hook_word: &'static str,
    /// The user-facing origin phrase — `` fn `tmpl` `` for API bodies,
    /// `` the `before` hook of annotation `traced` `` for sugar-minted
    /// bodies.
    pub origin: String,
    /// The DECLARATION view: the template's declared (generic) signature,
    /// e.g. `<Args>(args: Args) -> Args` — no body-fn prefix.
    pub declared_signature: String,
    /// The APPLICATION view: the required specialization signature at this
    /// `@application` in user spellings, e.g.
    /// `(int, number) -> (int, number)` (the r8 delimited types).
    pub specialized_signature: String,
    /// Capture names + rendered `LiftedConst` values, in delivery order.
    pub captures: Vec<(String, String)>,
    /// The `@application` anchor.
    pub application_span: Span,
    /// The body fn's user-spelled name for API-authored template bodies —
    /// the hover match identity for hover-on-the-body-fn. `None` for
    /// sugar-minted bodies (their SOH mint never displays; their hover
    /// surface is the application, matched via `annotation_name` +
    /// `application_span`).
    pub body_fn: Option<String>,
}

impl BytecodeCompiler {
    /// ADR-009 C3 #14 (S8c): the compiler-owned hook-install QUERY surface —
    /// one display-safe [`HookInstallView`] per applied `install(...)` row,
    /// in application order. Query AFTER `compile_in_place` (the
    /// `generated_symbol_query` discipline); a rolled-back compile leaves no
    /// row, so no view either.
    pub fn hook_install_query(&self) -> Vec<HookInstallView> {
        self.hook_install_registry
            .iter()
            .map(|record| {
                let TemplateBodyOrigin { origin, sugar, .. } =
                    self.template_body_origin(&record.body_fn, record.hook_kind);
                let declared_signature =
                    match record.template_sig.strip_prefix(record.body_fn.as_str()) {
                        Some(sig) => display_safe_declared_signature(
                            sig.trim_start().to_string(),
                            record.hook_kind,
                        ),
                        // Unreachable by construction (`template_sig` is
                        // `"{body_fn} {sig}"`, written by the one apply seam);
                        // fail SAFE for display — never risk a raw SOH prefix.
                        None => {
                            debug_assert!(false, "template_sig lost its body-fn prefix");
                            String::new()
                        }
                    };
                let hook_word = match record.hook_kind {
                    TemplateHookKind::Before => "before",
                    TemplateHookKind::After => "after",
                };
                let body_fn = (!sugar && !record.body_fn.starts_with('\u{1}'))
                    .then(|| record.body_fn.clone());
                HookInstallView {
                    annotation_name: record.annotation_name.clone(),
                    target_name: record.target_name.clone(),
                    hook_word,
                    origin,
                    declared_signature,
                    specialized_signature: record.specialized_sig.clone(),
                    captures: record.captures.clone(),
                    application_span: record.application_span,
                    body_fn,
                }
            })
            .collect()
    }
}

/// ADR-009 C3 #14 (S8c): render a declared-Sig string display-safe. A
/// sugar-minted POLYMORPHIC template renders its HYGIENIC type param
/// (`\u{1}hygienic:<hex>` — the sugar_lowering MINTING/HYGIENE note mints
/// the type-param name alongside the body-fn name) inside the polymorphic
/// `<{type_param}>({param}: {type_param}) -> {type_param}` rendering; the
/// display projection substitutes the canonical sugar-surface spelling
/// (`Args` for a `before` hook, `R` for an `after` — the sugar_lowering doc
/// convention). Mint-free sigs (every API-authored body; concrete/observer
/// sugar bodies, whose `BodySignature` excludes the capture tail) pass
/// through untouched.
fn display_safe_declared_signature(sig: String, hook_kind: TemplateHookKind) -> String {
    if !sig.contains('\u{1}') {
        return sig;
    }
    let display = match hook_kind {
        TemplateHookKind::Before => "Args",
        TemplateHookKind::After => "R",
    };
    // The polymorphic rendering opens with `<{type_param}>` — the minted
    // token is exactly the angle-delimited head; replace every occurrence.
    if let Some(rest) = sig.strip_prefix('<') {
        if let Some((param, _)) = rest.split_once('>') {
            if param.starts_with('\u{1}') {
                let substituted = sig.replace(param, display);
                if !substituted.contains('\u{1}') {
                    return substituted;
                }
            }
        }
    }
    // Defensive only (no known rendering reaches here): scrub any residual
    // mint token so no SOH byte can ever display.
    debug_assert!(
        false,
        "declared-Sig rendering carried a mint outside the type-param head"
    );
    let mut out = String::with_capacity(sig.len());
    let mut rest = sig.as_str();
    while let Some(position) = rest.find('\u{1}') {
        out.push_str(&rest[..position]);
        let tail = &rest[position..];
        let token_end = tail
            .char_indices()
            .skip(1)
            .find(|(_, ch)| !(ch.is_ascii_alphanumeric() || *ch == '_' || *ch == ':'))
            .map(|(index, _)| index)
            .unwrap_or(tail.len());
        out.push_str(display);
        rest = &tail[token_end..];
    }
    out.push_str(rest);
    out
}

/// Render the capture bindings for the registry row: `(name, rendering)` in
/// the body fn's trailing-parameter (delivery) order — the same order the
/// S3b bake prologue binds in (`const_lift::capture_values_in_delivery_order`).
fn rendered_captures(bound: &BoundTemplate) -> Vec<(String, String)> {
    bound
        .template
        .capture_params()
        .iter()
        .filter_map(|(param_name, _)| {
            bound
                .capture_values
                .iter()
                .find(|(name, _)| name == param_name)
                .map(|(name, lifted)| (name.clone(), lifted.render()))
        })
        .collect()
}

/// The ONE C3-G8 sentence producer — shared by the pre-pass firing site
/// (`apply_signature_directives_to_analysis_function`, which observes the
/// generic def's real `type_params`) and the seam's two defensive twins, so
/// the ruled wording (target + generic signature + the #59 re-arm cite + the
/// concrete-target positive twin) never forks.
pub(in crate::compiler) fn generic_target_install_rejection_message(
    template_body_fn: &str,
    annotation_name: &str,
    func_def: &FunctionDef,
) -> String {
    format!(
        "cannot install hook template `{template_body_fn}` (via @{annotation_name}) on `{}`: \
         the target is generic — `{}` — and hook-template installs on generic targets are \
         withdrawn until #59 (the monomorphization-origin re-arm) lands (C3-G8); \
         signature-polymorphic templates stay definable and usable on every concrete target — \
         apply @{annotation_name} to a concrete function",
        func_def.name,
        render_generic_target_signature(func_def),
    )
}

/// Render a generic target's signature for the C3-G8 rejection —
/// `fn name<T, U>(a: T, b: int) -> T`, from the AST side (user spellings,
/// never a mangled mono-key — the S0 g3 rule).
fn render_generic_target_signature(func_def: &FunctionDef) -> String {
    let type_params = func_def
        .type_params
        .as_ref()
        .map(|params| {
            params
                .iter()
                .map(|param| param.name().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let params = func_def
        .params
        .iter()
        .map(|param| {
            let name = param.simple_name().unwrap_or("<pattern>");
            match &param.type_annotation {
                Some(annotation) => format!("{name}: {}", type_annotation_to_string(annotation)),
                None => name.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    match &func_def.return_type {
        Some(annotation) => format!(
            "fn {}<{type_params}>({params}) -> {}",
            func_def.name,
            type_annotation_to_string(annotation)
        ),
        None => format!("fn {}<{type_params}>({params})", func_def.name),
    }
}

// ADR-009 C3 #14 (slice 2, S2b): end-to-end apply-seam pins through the
// PUBLIC comptime API over the full compile path (parse → pre-pass →
// analyzer → pass-2 handler → `install` directive → this module's apply
// seam), asserting COMPILER STATE (the journaled registry) and EXECUTING
// specialized handlers (the S1c stage-4 pattern — compile-proof alone is
// banned per the S0 §2 named uncertainty).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::comptime_builtins::expansion_provenance::{
        ApplicationId, CanonicalHash, ComptimeStage, ExpansionIdentity, GeneratorRef,
        TargetIdentity,
    };
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::{KindedSlot, NativeKind};

    /// Whole-program fixture (the S2a `hook_template_builtin_tests` shape):
    /// module-scope body fns + one installing annotation + caller-supplied
    /// annotated targets (and optional top-level calls).
    fn hook_source(body_fns: &str, handler_stmts: &str, targets_and_calls: &str) -> String {
        format!(
            r#"
{body_fns}

annotation hookann() on function {{
  comptime post(target, ctx) {{
    {handler_stmts}
  }}
}}

{targets_and_calls}
"#
        )
    }

    /// Full production compile; returns BOTH the result and the compiler so
    /// pins can assert the post-compile (or post-rollback) registry state.
    fn compile_source(
        src: &str,
    ) -> (
        shape_ast::error::Result<()>,
        crate::compiler::BytecodeCompiler,
    ) {
        let program = shape_ast::parse_program(src).expect("fixture parses");
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        let result = compiler.compile_in_place(&program);
        (result, compiler)
    }

    fn compiled_ok(src: &str) -> crate::compiler::BytecodeCompiler {
        let (result, compiler) = compile_source(src);
        result.expect("fixture must compile");
        compiler
    }

    fn expect_compile_reject(src: &str, needles: &[&str]) -> crate::compiler::BytecodeCompiler {
        let (result, compiler) = compile_source(src);
        let text = result.expect_err("fixture must reject").to_string();
        for needle in needles {
            assert!(
                text.contains(needle),
                "expected rejection containing {needle:?}, got: {text}"
            );
        }
        compiler
    }

    // IDENTITY PROOF (the S1c pattern) + the pre-pass single-apply pin:
    // a concrete install resolves the DEFINITION-COMPILED body fn's index,
    // and the pre-pass + pass-2 double handler run yields EXACTLY ONE
    // registry row (the pre-pass consumer arm is a documented no-op).
    #[test]
    fn concrete_install_records_the_definition_compiled_body_index_once() {
        let compiler = compiled_ok(&hook_source(
            "fn my_before(x: int) -> int { return x + 1 }",
            "install(before_hook(my_before, []))",
            "@hookann()\nfn victim(a: int) -> int { return a }\n\nvictim(1)",
        ));
        assert_eq!(
            compiler.hook_install_registry.len(),
            1,
            "exactly one row across the pre-pass + pass-2 double handler run"
        );
        let row = &compiler.hook_install_registry[0];
        assert_eq!(row.annotation_name, "hookann");
        assert_eq!(row.target_name, "victim");
        assert_eq!(row.hook_kind, TemplateHookKind::Before);
        let body_index = compiler
            .find_function("my_before")
            .expect("the template body fn is registered");
        assert_eq!(
            row.function_index as usize, body_index,
            "a concrete install resolves the definition-compiled body index"
        );
        assert_eq!(row.specialized_symbol, "my_before");
        assert!(row.captures.is_empty());
        assert!(
            row.template_sig.contains("my_before") && row.template_sig.contains("(int) -> int"),
            "the row renders the template's declared Sig: {}",
            row.template_sig
        );
    }

    // EXECUTION PROOF (the S1c stage-4 pattern; S3b PIN RE-TARGET, ordered
    // by the slice-3 charter): a polymorphic install with a scalar capture
    // specializes through the monomorphization ride with the capture BAKED —
    // the specialized handler's arity == Sig arity (the capture tail is
    // STRIPPED; the value is a prologue constant), it executes with ONLY the
    // target slots, mutates slot 0 via the BAKED capture and the
    // `args.length` constant, and the capture literal is recorded on the
    // registry row. The S2 twin executed with [target slots..., capture
    // value] — that delivery mechanism is deleted.
    #[test]
    fn polymorphic_install_with_capture_executes_and_records_the_literal() {
        let compiler = compiled_ok(&hook_source(
            "fn tmpl<Args>(args: Args, factor: int) -> Args {\n\
             \x20   args[0] = args[0] * factor + args.length\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, [capture(\"factor\", 3)]))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\nvictim(1, 2.0)",
        ));
        assert_eq!(compiler.hook_install_registry.len(), 1);
        let row = &compiler.hook_install_registry[0];
        assert_eq!(row.hook_kind, TemplateHookKind::Before);
        assert_eq!(
            row.captures,
            vec![("factor".to_string(), "3".to_string())],
            "the capture literal is recorded in delivery order"
        );
        assert!(
            row.specialized_symbol.contains("tmpl"),
            "the polymorphic install records the suffixed specialization symbol: {}",
            row.specialized_symbol
        );
        assert_ne!(
            row.specialized_symbol, "tmpl",
            "a polymorphic handler is a specialization, never the generic def itself"
        );
        assert!(
            row.specialized_symbol.contains("::cfg#1::i:3"),
            "S3b: the symbol carries the rule-6 config segment: {}",
            row.specialized_symbol
        );

        // Direct-call execution proof with ONLY the Sig args (S3b: the
        // capture is BAKED, not passed): [a=5, b=4.5] →
        // a0 = 5*factor(baked 3) + args.length(=2) = 17, a1 = 4.5 untouched.
        //
        // S2c UPDATE (execute by NAME, not by compiler-side index): the
        // row's `function_index` is a COMPILER-side program index, and
        // `VirtualMachine::load_program` links/relocates the shipped
        // program — its function ids need not match. At S2b the two
        // orderings happened to coincide; the S2c weave's extra generated
        // functions (hygienic shadow + woven wrapper) shifted the linked
        // order, so executing the raw index silently ran the WOVEN TARGET
        // instead of the handler. The row's `specialized_symbol` names the
        // handler unambiguously in both tables.
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(compiler.program.clone());
        let result = vm
            .execute_function_by_name(
                &row.specialized_symbol,
                vec![KindedSlot::from_int(5), KindedSlot::from_number(4.5)],
                None,
            )
            .expect("the specialized handler executes");
        let storage = result
            .as_typed_object_storage()
            .expect("the 2-ary mutation aggregate is the inline-schema TypedObject");
        assert_eq!(storage.field_kinds[0], NativeKind::Int64);
        assert_eq!(
            storage.slots()[0].raw() as i64,
            17,
            "slot 0 mutated via capture + length"
        );
        assert_eq!(storage.field_kinds[1], NativeKind::Float64);
        assert_eq!(
            f64::from_bits(storage.slots()[1].raw()),
            4.5,
            "slot 1 flows through"
        );
    }

    // THE VERIFY-1 REFUTER SHAPE THROUGH THE API: two targets whose FLAT
    // tuple renderings collide (the S1 injectivity fix's exact pair) must
    // specialize SEPARATELY when installed through the public builtins.
    #[test]
    fn colliding_render_sigs_installed_through_the_api_stay_separate() {
        let compiler = compiled_ok(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\x20   return args\n}",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn target_a(x: [int, int, int], y: number) -> int { return 1 }\n\n\
             @hookann()\nfn target_b(x: [int, int], y: int, z: number) -> int { return y }",
        ));
        assert_eq!(
            compiler.hook_install_registry.len(),
            2,
            "one row per annotated target"
        );
        let (first, second) = (
            &compiler.hook_install_registry[0],
            &compiler.hook_install_registry[1],
        );
        assert_eq!(first.target_name, "target_a");
        assert_eq!(second.target_name, "target_b");
        assert_ne!(
            first.function_index, second.function_index,
            "distinct Sigs must never share one checked specialization (verify-1)"
        );
    }

    // C3-G8: install on a GENERIC target is a named rejection — target name
    // + generic signature + the #59 re-arm cite + the concrete-target
    // positive twin — and no row lands. REACH UPDATE (S5b): the STATIC
    // application-site arm (`reject_template_engaging_annotation_on_
    // generic_target`, functions_annotations.rs) now fires FIRST at the
    // signature-directive pre-pass — per `@application`, before any handler
    // execution — closing the S2b residual (an UNCALLED generic target in a
    // single-module unit was a SILENT no-op; probe P-G8b, slice-5 report).
    // The dynamic sites (the pre-pass directive arm + this seam's two
    // twins) remain as layered backstops.
    #[test]
    fn generic_target_install_rejects_with_the_g8_sentence() {
        let compiler = expect_compile_reject(
            &hook_source(
                "fn my_before(x: int) -> int { return x + 1 }",
                "install(before_hook(my_before, []))",
                "@hookann()\nfn victim<T>(x: T) -> T { return x }\n\nvictim(1)",
            ),
            &[
                "cannot install hook template `my_before` (via @hookann) on `victim`",
                "fn victim<T>(x: T) -> T",
                "withdrawn until #59 (the monomorphization-origin re-arm)",
                "apply @hookann to a concrete function",
            ],
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "a rejected install leaves no registry row"
        );
    }

    // ═══ ADR-009 C3 #14 (slice 5, S5b) — THE STATIC C3-G8 ARM ═══
    //
    // The S2b supervisor obligation closed: a @application of a
    // template-engaging annotation on a generic target rejects STATICALLY at
    // the application site, with NO reliance on the target being called and
    // NO handler-run dependence — through both the sugar path (the
    // sugar_matrix pin) and the API path (here). Pre-fix measurements
    // (probe wave, slice-5 report §S5b): P-G8b (API direct, uncalled
    // generic) compiled and ran SILENTLY with zero registry rows; P-G8d
    // (value-position `let f = before_hook`) likewise. The pins below are
    // the closed holes; the neuter refuter is recorded in the report.

    /// Span-asserting compile (the weave.rs S5a harness shape: source_text
    /// gives real span→line mapping).
    fn expect_semantic_error_with_line(
        src: &str,
    ) -> (
        String,
        Option<shape_ast::error::SourceLocation>,
        crate::compiler::BytecodeCompiler,
    ) {
        let program = shape_ast::parse_program(src).expect("fixture parses");
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        compiler.source_text = Some(src.to_string());
        let result = compiler.compile_in_place(&program);
        match result.expect_err("fixture must reject") {
            ShapeError::SemanticError { message, location } => (message, location, compiler),
            other => panic!("expected a SemanticError, got: {other}"),
        }
    }

    // THE HEADLINE (pin ii): an API-path installing annotation on an
    // UNCALLED generic target in a single-module unit — measured SILENT
    // pre-fix (P-G8b: `compiles, run=Ok(Some(7)), registry_rows=0`) — now
    // rejects with the byte-unchanged G8 sentence (the ONE producer),
    // anchored at the `@application` line, zero registry rows.
    #[test]
    fn s5b_static_g8_api_install_on_uncalled_generic_rejects_at_the_application_site() {
        let src = r#"
fn my_before(x: int) -> int { return x + 1 }

annotation hookann() on function {
  comptime post(target, ctx) {
    install(before_hook(my_before, []))
  }
}

@hookann()
fn victim<T>(x: T) -> T { return x }

7
"#;
        let (message, location, compiler) = expect_semantic_error_with_line(src);
        for needle in [
            "cannot install hook template `my_before` (via @hookann) on `victim`",
            "fn victim<T>(x: T) -> T",
            "withdrawn until #59 (the monomorphization-origin re-arm)",
            "apply @hookann to a concrete function",
        ] {
            assert!(
                message.contains(needle),
                "the G8 sentence must fire statically on the UNCALLED generic; \
                 missing {needle:?}: {message}"
            );
        }
        let location = location.expect("the rejection carries the @application span");
        let application_line = src
            .lines()
            .position(|line| line.starts_with("@hookann"))
            .expect("fixture has the application line")
            + 1;
        assert_eq!(
            location.line, application_line,
            "the static arm anchors at the @application site"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "a rejected install leaves no registry row"
        );
    }

    // Pin (iii): helper-mediated install — the handler calls a module helper
    // whose body installs; the static scan's transitive closure (authorized
    // helpers + the AST-side syntactic table) sees through the indirection.
    // DISCLOSED behavior change on the GENERIC target only: pre-fix this
    // fixture failed LOUD-BUT-BLAND at the helper's own pass-2 compile
    // (P-G8c: `Undefined function: 'before_hook'` — module fns cannot spell
    // the comptime forwarders); the named G8 sentence now preempts it at the
    // @application site.
    #[test]
    fn s5b_static_g8_helper_mediated_install_on_uncalled_generic_rejects() {
        let compiler = expect_compile_reject(
            r#"
fn my_before(x: int) -> int { return x + 1 }

fn do_install() {
  install(before_hook(my_before, []))
}

annotation hookann() on function {
  comptime post(target, ctx) {
    do_install()
  }
}

@hookann()
fn victim<T>(x: T) -> T { return x }

7
"#,
            &[
                "cannot install hook template `my_before` (via @hookann) on `victim`",
                "fn victim<T>(x: T) -> T",
                "withdrawn until #59 (the monomorphization-origin re-arm)",
            ],
        );
        assert!(compiler.hook_install_registry.is_empty());
    }

    // Pin (iii) CONTROL: the same helper-mediated shape on a CONCRETE
    // target keeps the PRE-EXISTING loud failure byte-unchanged (the static
    // arm keys on `type_params` and never touches concrete targets).
    #[test]
    fn s5b_static_g8_helper_mediated_concrete_control_keeps_the_preexisting_loud_error() {
        expect_compile_reject(
            r#"
fn my_before(x: int) -> int { return x + 1 }

fn do_install() {
  install(before_hook(my_before, []))
}

annotation hookann() on function {
  comptime post(target, ctx) {
    do_install()
  }
}

@hookann()
fn victim(x: int) -> int { return x }

victim(7)
"#,
            &["Undefined function: 'before_hook'"],
        );
    }

    // Pin (iv): a VALUE-position install-family reference cannot dodge the
    // scan — `let f = before_hook; install(f(...))` on an uncalled generic
    // was SILENT pre-fix (P-G8d). The hint falls back to the established
    // `<template>` placeholder (no hook-constructor call-shape to name).
    #[test]
    fn s5b_static_g8_value_position_reference_on_uncalled_generic_rejects() {
        let compiler = expect_compile_reject(
            r#"
fn my_before(x: int) -> int { return x + 1 }

annotation hookann() on function {
  comptime post(target, ctx) {
    let f = before_hook
    install(f(my_before, []))
  }
}

@hookann()
fn victim<T>(x: T) -> T { return x }

7
"#,
            &[
                "cannot install hook template `<template>` (via @hookann) on `victim`",
                "withdrawn until #59 (the monomorphization-origin re-arm)",
            ],
        );
        assert!(compiler.hook_install_registry.is_empty());
    }

    // Pin (iv) isolation: the VALUE-position arm ALONE engages — a handler
    // whose ONLY `install` reference is `let f = install` (no call
    // anywhere) still classifies template-engaging. Without the S5b marked
    // value arm in the shared collector this fixture compiles silently.
    #[test]
    fn s5b_static_g8_value_position_reference_alone_engages_the_scan() {
        let compiler = expect_compile_reject(
            r#"
annotation hookann() on function {
  comptime post(target, ctx) {
    let f = install
  }
}

@hookann()
fn victim<T>(x: T) -> T { return x }

7
"#,
            &[
                "cannot install hook template `<template>` (via @hookann) on `victim`",
                "apply @hookann to a concrete function",
            ],
        );
        assert!(compiler.hook_install_registry.is_empty());
    }

    // ENGAGEMENT-KEY CONTROL: a CONSTRUCT-only handler (`before_hook`
    // handles built, nothing installed) on a generic target is NOT
    // template-engaging — C3-G8 withdraws INSTALLS; construction is legal
    // load-bearing machinery (the F5 store-lifecycle refuter class below,
    // `nested_handler_run_during_processing_does_not_shift_install_handles`,
    // annotates a polymorphic template body fn exactly so).
    #[test]
    fn s5b_static_g8_construct_only_handler_on_generic_stays_green() {
        let compiler = compiled_ok(
            r#"
fn h_noise(x: int) -> int { return x + 40 }

annotation noise() on function {
  comptime post(target, ctx) {
    let a = before_hook(h_noise, [])
  }
}

@noise()
fn tmpl<Args>(args: Args) -> Args { return args }

7
"#,
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "a construct-only handler installs nothing"
        );
    }

    // Pin (v): the concrete-target twin — the SAME API-path annotation on an
    // UNCALLED CONCRETE fn compiles and installs (P-CONC-unc measured one
    // registry row; concrete pass-2 body compile runs regardless of
    // calledness). The static arm's `type_params` key never brushes it.
    #[test]
    fn s5b_static_g8_uncalled_concrete_twin_still_installs() {
        let compiler = compiled_ok(&hook_source(
            "fn my_before(x: int) -> int { return x + 1 }",
            "install(before_hook(my_before, []))",
            "@hookann()\nfn victim(x: int) -> int { return x }\n\n7",
        ));
        assert_eq!(
            compiler.hook_install_registry.len(),
            1,
            "the uncalled CONCRETE target still installs"
        );
        assert_eq!(compiler.hook_install_registry[0].target_name, "victim");
    }

    // Pin (vi) — S6 FLIP of the S5b legacy-weave control: the C3-G11
    // DELIBERATE CAPABILITY WITHDRAWAL lands with the collapse. A zero-param
    // hook definition now routes the typed weave, so a hook on a GENERIC
    // target is the G8 named rejection at the application site (the S0
    // g1/g4 accidental-working class worked only by accident of the deleted
    // homogeneous-args representation; the defections.md C3-G11 entry names
    // the withdrawal and the #59 re-arm).
    #[test]
    fn s6_static_g8_zero_param_hook_on_generic_target_now_rejects() {
        let (result, compiler) = compile_source(
            r#"
annotation dbl() on function {
  before(args) {
    args[0] = args[0] * 2
    return args
  }
}

@dbl()
fn id<T>(x: T) -> T { return x }

id(5)
"#,
        );
        let message = result
            .expect_err("a zero-param hook on a generic target must reject (C3-G8/G11)")
            .to_string();
        assert!(
            message.contains("withdrawn until #59"),
            "the G8 sentence must fire on the generic target, got: {message}"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "a rejected install leaves no registry row"
        );
    }

    // Non-function target: the pass-2 type-target consumer names the
    // rejection with its positive twin.
    #[test]
    fn type_target_install_rejects_with_the_function_twin() {
        expect_compile_reject(
            &format!(
                r#"
fn my_before(x: int) -> int {{ return x + 1 }}

annotation hookann() on type {{
  comptime post(target, ctx) {{
    install(before_hook(my_before, []))
  }}
}}

@hookann()
type Widget {{
  w: int
}}
"#
            ),
            &[
                "`install` directives are only valid when compiling function targets",
                "apply the installing annotation to a function",
            ],
        );
    }

    // ADR-009 C3-S6 completion: `mixed_legacy_weave_target_rejects_the_
    // install` RETIRED — the mixed-legacy state is UNCONSTRUCTIBLE after the
    // classification collapse (its `legacy_hook()` + `before(args, ctx)`
    // fixture now rejects at the declaration, and no definition can populate
    // handler slots that no longer exist). The dead one-weave-owner rejection
    // block was deleted with the rest of the legacy machinery at the S6
    // capstone.

    // The install builtin's own misuse rejection: a non-handle argument is
    // named with the producer twin.
    #[test]
    fn non_handle_install_argument_rejects_naming_the_producers() {
        expect_compile_reject(
            &hook_source(
                "fn my_before(x: int) -> int { return x + 1 }",
                "install(5)",
                "@hookann()\nfn victim(a: int) -> int { return a }\n\nvictim(1)",
            ),
            &[
                "install expects a __CheckedTemplate handle",
                "before_hook(body_fn, captures) or after_hook(body_fn, captures)",
            ],
        );
    }

    // ROLLBACK PROBE (extends the S1c pin over the REGISTRY): the FIRST
    // target's install applies (row + cache entry), the SECOND target's
    // install fails at pass-2 (zero-param before), the whole compile rolls
    // back — NO registry row survives and the monomorphization cache is
    // evicted (checked_body/mod.rs fold-in). Non-vacuity: the error names
    // the SECOND target, proving the first had already applied in program
    // order before the rollback.
    #[test]
    fn failing_install_rolls_back_registry_rows_and_the_cache() {
        let (result, compiler) = compile_source(&hook_source(
            "fn tmpl<Args>(args: Args) -> Args {\n\x20   return args\n}",
            "install(before_hook(tmpl, []))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\n\
             @hookann()\nfn zero() -> int { return 7 }",
        ));
        let text = result
            .expect_err("the zero-param before target must fail the compile")
            .to_string();
        assert!(
            text.contains("zero") && text.contains("declares no parameters"),
            "the failure comes from the SECOND target's install (non-vacuity): {text}"
        );
        assert!(
            compiler.hook_install_registry.is_empty(),
            "rollback removes the first target's already-written registry row"
        );
        assert_eq!(
            compiler.monomorphization_cache.legacy_len(),
            0,
            "rollback evicts the first install's at/above-watermark cache entry"
        );
    }

    // Stale/out-of-range handle at the SNAPSHOT resolver is
    // INTERNAL-ERROR-shaped (a handle can only be minted with a live
    // just-pushed index; user code cannot spell one). Fix-round-1: the
    // resolution moved from the per-directive apply seam to the batch
    // snapshot at directive-loop entry — this pins the moved producer.
    #[test]
    fn stale_handle_is_an_internal_error_at_the_snapshot() {
        crate::compiler::comptime_builtins::clear_comptime_hook_templates();
        let compiler = crate::compiler::BytecodeCompiler::new();
        let site = ExpansionSite::new(
            ExpansionIdentity::new(
                GeneratorRef::from_canonical_descriptor("annotation:hookann:post"),
                ApplicationId::from_canonical_descriptor("application:test:0:0"),
                TargetIdentity::from_canonical_descriptor("function:victim"),
                ComptimeStage::AnnotationHandler,
                CanonicalHash::from_canonical_argument_descriptors(&[]),
                CanonicalHash::from_canonical_dependency_descriptors(&[]),
            ),
            0,
            Span::default(),
            Span::default(),
        );
        let directives = vec![ComptimeDirective::InstallHookTemplate { template_index: 7 }];
        let err = compiler
            .snapshot_install_hook_template_handles(&directives, &site)
            .expect_err("a dead index must be internal-error-shaped");
        assert!(
            err.to_string().contains("internal error"),
            "internal-error-shaped: {err}"
        );
        assert!(
            err.to_string()
                .contains("snapshot-resolved before any directive applies"),
            "names the snapshot discipline: {err}"
        );
        assert!(compiler.hook_install_registry.is_empty());
    }

    // FIX-ROUND-1 REGRESSION (the stale-handle-across-runs class): applying
    // an earlier install in the SAME directive list triggers a NESTED
    // handler run (the polymorphic template's body fn carries an annotation;
    // annotations survive substitution, so the specialization's nested
    // `compile_function` re-enters `execute_comptime_handlers`), which
    // clears + REPOPULATES the per-run template store — the nested handler
    // deliberately pushes TWO templates so index 1 exists again. Pre-fix,
    // the LATER install's lazy resolution of index 1 picked up the nested
    // run's `h_noise2` ((int) -> int, Sig-compatible with the target) and
    // installed the WRONG template SILENTLY: victim(4) = (4+5+100)*10 =
    // 1090. The batch snapshot resolves both handles before any apply, so
    // the second install is `h2`: (4+5)*2 = 18 → impl 180.
    #[test]
    fn nested_handler_run_during_processing_does_not_shift_install_handles() {
        let src = r#"
fn h_noise(x: int) -> int { return x + 40 }
fn h_noise2(x: int) -> int { return x + 100 }
fn h2(x: int) -> int { return x * 2 }

annotation noise() on function {
  comptime post(target, ctx) {
    let a = before_hook(h_noise, [])
    let b = before_hook(h_noise2, [])
  }
}

@noise()
fn tmpl<Args>(args: Args) -> Args {
    args[0] = args[0] + 5
    return args
}

annotation hookann() on function {
  comptime post(target, ctx) {
    install(before_hook(tmpl, []))
    install(before_hook(h2, []))
  }
}

@hookann()
fn victim(a: int) -> int { return a * 10 }

victim(4)
"#;
        let compiler = compiled_ok(src);
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(compiler.program.clone());
        let value = vm
            .execute(None)
            .expect("program executes")
            .as_i64()
            .expect("top-level result is an int");
        assert_eq!(
            value, 180,
            "the SECOND install must be h2 ((4+5)*2 → impl 180); the nested run's \
             repopulated store must never leak in (stale index ⇒ h_noise2 ⇒ 1090)"
        );
        assert_eq!(compiler.hook_install_registry.len(), 2);
        assert!(
            compiler.hook_install_registry[1]
                .template_sig
                .starts_with("h2 "),
            "row 2 records the SNAPSHOT-resolved template, not the nested run's: {}",
            compiler.hook_install_registry[1].template_sig
        );
    }

    // FIX-ROUND-1: a handler-local binding shadowing the body fn's name is a
    // named rejection end-to-end through the full compile (the rewrite's
    // shadow check — comptime.rs; without it the module fn would bind
    // silently, inverting ordinary shadowing).
    #[test]
    fn handler_local_shadowing_the_body_fn_rejects_end_to_end() {
        expect_compile_reject(
            &hook_source(
                "fn my_hook(x: int) -> int { return x + 1 }",
                "let my_hook = 3\n    install(before_hook(my_hook, []))",
                "@hookann()\nfn victim(a: int) -> int { return a }\n\nvictim(1)",
            ),
            &[
                "is shadowed by a handler-local binding",
                "rename the local binding or the body fn",
            ],
        );
    }

    // ═══ ADR-009 C3 #14 (S8c) — the hover QUERY projection ═══
    //
    // `hook_install_query()` is the shared query surface the LSP hover
    // reads (the C1 slice-4 `generated_symbol_query` precedent). The first
    // pin is the compiler-tier display-safety MACHINE PIN (charter: no
    // display field of any projected row contains '\u{1}') with its
    // built-in planted needle — the sugar row's RAW registry identity IS
    // the SOH mint, so the projection is provably what strips it. The
    // second is the API-row identity twin (body-fn identity + the r8
    // delimited application view).

    #[test]
    fn s8c_sugar_row_projection_is_display_safe_and_carries_both_views() {
        let compiler = compiled_ok(
            r#"
annotation traced(factor: int) on function {
  before(args) {
    args[0] = args[0] * factor
    return args
  }
}

@traced(3)
fn victim(a: int) -> int { return a }

victim(1)
"#,
        );
        // The planted needle (vacuity guard): the RAW row's identity fields
        // DO carry the SOH hygienic mint...
        assert_eq!(compiler.hook_install_registry.len(), 1);
        let record = &compiler.hook_install_registry[0];
        assert!(
            record.body_fn.starts_with('\u{1}'),
            "control: the sugar row's raw body-fn identity is the SOH mint"
        );
        assert!(
            record.template_sig.contains('\u{1}'),
            "control: the raw declared-Sig rendering carries the mint prefix"
        );

        // ...and the projection strips every one of them.
        let views = compiler.hook_install_query();
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert_eq!(view.annotation_name, "traced");
        assert_eq!(view.target_name, "victim");
        assert_eq!(view.hook_word, "before");
        assert_eq!(
            view.origin, "the `before` hook of annotation `traced`",
            "sugar origin renders through the ONE producer, never the SOH name"
        );
        assert_eq!(view.declared_signature, "<Args>(args: Args) -> Args");
        assert_eq!(view.specialized_signature, "(int) -> int");
        assert_eq!(view.captures, vec![("factor".to_string(), "3".to_string())]);
        assert_eq!(
            view.body_fn, None,
            "a sugar mint is never a display identity"
        );
        assert_ne!(view.application_span, Span::default());
        for field in [
            view.annotation_name.as_str(),
            view.target_name.as_str(),
            view.hook_word,
            view.origin.as_str(),
            view.declared_signature.as_str(),
            view.specialized_signature.as_str(),
        ] {
            assert!(
                !field.contains('\u{1}'),
                "SOH leaked into a display field: {field:?}"
            );
        }
        for (name, value) in &view.captures {
            assert!(!name.contains('\u{1}') && !value.contains('\u{1}'));
        }
    }

    #[test]
    fn s8c_api_row_projection_carries_the_body_fn_identity_and_application_view() {
        let compiler = compiled_ok(&hook_source(
            "fn tmpl<Args>(args: Args, factor: int) -> Args {\n\
             \x20   args[0] = args[0] * factor\n\
             \x20   return args\n\
             }",
            "install(before_hook(tmpl, [capture(\"factor\", 3)]))",
            "@hookann()\nfn victim(a: int, b: number) -> int { return a }\n\nvictim(1, 2.0)",
        ));
        let views = compiler.hook_install_query();
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert_eq!(
            view.body_fn.as_deref(),
            Some("tmpl"),
            "an API body projects its real fn name as the hover match identity"
        );
        assert_eq!(view.origin, "fn `tmpl`");
        assert_eq!(view.declared_signature, "<Args>(args: Args) -> Args");
        assert_eq!(
            view.specialized_signature, "(int, number) -> (int, number)",
            "the application view renders the r8 delimited types"
        );
        assert_eq!(view.captures, vec![("factor".to_string(), "3".to_string())]);
    }

    // FIX-ROUND-1 (lens item 6): the MODULE-target consumer's install
    // rejection arm fires with the function positive twin — the sibling of
    // `type_target_install_rejects_with_the_function_twin`
    // (`process_comptime_directives_for_module`, statements.rs).
    #[test]
    fn module_target_install_rejects_with_the_function_twin() {
        expect_compile_reject(
            &format!(
                r#"
fn my_before(x: int) -> int {{ return x + 1 }}

annotation hookann() on module {{
  comptime post(target, ctx) {{
    install(before_hook(my_before, []))
  }}
}}

@hookann()
mod demo {{
  fn answer() -> int {{ return 0 }}
}}
"#
            ),
            &[
                "`install` directives are only valid when compiling function targets",
                "apply the installing annotation to a function",
            ],
        );
    }
}
