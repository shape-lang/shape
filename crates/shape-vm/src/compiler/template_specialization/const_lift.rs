//! ADR-009 C3 #14 (slice 2) — the ConstLift capture seam: how declared capture
//! VALUES cross from a hook-template construction site into a specialized
//! handler.
//!
//! # The S2 domain is SCALARS ONLY — and S3 REPLACES this module's domain
//!
//! This slice lifts exactly the four scalar literals: `int`, `number`, `bool`,
//! `string` ([`LiftedConst`]). Everything else is a NAMED rejection pointing at
//! the S3 domain. S3 REPLACES this module's domain at this fn/type boundary —
//! the [`lift_capture_value`] signature and the [`LiftedConst`] type are the
//! seam S3's validation slots into:
//!
//! - the COMPOSITIONAL liftable domain (C3-G5: primitives + tuples/arrays/
//!   `Option` of liftables, recursively),
//! - the NEVER-LIFTABLE named rejections per C3-G5 / Dec 95 (references,
//!   resources, capabilities, functions, provider grants, compiler
//!   descriptors, secrets, runtime handles — declaration-site rejection
//!   listing the domain),
//! - the heap-constant `Baked` variant (heap-constant baking at
//!   specialization), and
//! - the Dec-95 rule-6 STRUCTURAL spec-hash (lifted-constant identity →
//!   specialization hash, structural equality for composite config values).
//!
//! `unit` is also S3's: no unit LITERAL `Expr` exists at HEAD
//! (`shape_ast::ast::Literal` has no `Unit` variant), so a unit capture value
//! is named-rejected into the S3 domain sentence rather than half-supported.
//!
//! # Why S2 passes CALL-SITE LITERALS (never const-generic reroute, never a
//! # spec-hash)
//!
//! [`CaptureBindingPlan::CallSiteArgs`] delivers capture values as TYPED
//! LITERAL arguments at the wrapper's handler call sites. For scalars this is
//! semantically identical to baking, and it is chosen deliberately:
//!
//! - it kills the legacy per-invocation-config-eval disease and its W39
//!   `LoadModuleBinding` JIT poison (slice-0 report §4 / §8 item 11);
//! - decisively, it leaves the specialization CACHE KEY untouched — handlers
//!   stay value-generic and are SHARED across installs whose capture values
//!   differ, so S2 builds ZERO of S3's Dec-95 rule-6 spec-hash and cannot
//!   resurrect the verify-1 injectivity bug class (`c3-slice1-report.md`
//!   §Verify round 1, finding 1);
//! - S1's monomorphization plan-guard (b) (`cache.rs`, the CONST-PARAM GUARD)
//!   already fences the const-generic reroute to S3: a template plan reaching
//!   the const-generic path is a named internal error until S3 owns it.
//!
//! # Naming (c3-decisions.md §Naming — binding)
//!
//! The Rust type is module-scoped `const_lift::LiftedConst` — never a bare
//! `ConstValue` (collision with the monomorphization `call_site_consts`
//! machinery), and never built on the dead `comptime_concrete::ConstantValue`
//! (its `Opaque` variant is ValueWord-shaped — a Forbidden-Patterns
//! defection).

use shape_ast::ast::{Expr, Literal, Span, TypeAnnotation};
use shape_ast::error::ShapeError;
use shape_value::KindedSlot;

use super::SpecializationTarget;
use crate::compiler::comptime_builtins::BoundTemplate;

/// A capture value lifted into the S2 scalar constant domain. S3 replaces
/// this enum's domain (compositional liftables + the heap-constant `Baked`
/// variant) at this boundary — see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::compiler) enum LiftedConst {
    /// An `int` capture value.
    Int(i64),
    /// A finite `number` capture value.
    Number(f64),
    /// A `bool` capture value.
    Bool(bool),
    /// A `string` capture value (owned copy — the thread-local store outlives
    /// the mini-VM slot that carried it).
    String(String),
}

impl LiftedConst {
    /// The Shape type name this constant inhabits — the exact spelling a
    /// trailing capture parameter must be annotated with.
    pub(in crate::compiler) fn shape_type_name(&self) -> &'static str {
        match self {
            LiftedConst::Int(_) => "int",
            LiftedConst::Number(_) => "number",
            LiftedConst::Bool(_) => "bool",
            LiftedConst::String(_) => "string",
        }
    }

    /// Whether a declared capture-parameter annotation matches this
    /// constant's type. Exact-name matching over the scalar spellings —
    /// `Basic("int")` or a single-segment `Reference` naming `int` (the same
    /// single-segment rule `checked_template::is_bare_type_param` uses); a
    /// type alias spelled differently is a mismatch surfaced by the named
    /// rejection (strictness over guessing).
    pub(in crate::compiler) fn matches_annotation(&self, annotation: &TypeAnnotation) -> bool {
        let name = match annotation {
            TypeAnnotation::Basic(name) => name.as_str(),
            TypeAnnotation::Reference(path) if !path.is_qualified() => path.name(),
            _ => return false,
        };
        name == self.shape_type_name()
    }

    /// The typed literal `Expr` the weave passes at a handler call site
    /// ([`CaptureBindingPlan::CallSiteArgs`]). AST-level literal — no source
    /// text ever exists.
    pub(in crate::compiler) fn to_literal_expr(&self) -> Expr {
        let literal = match self {
            LiftedConst::Int(value) => Literal::Int(*value),
            LiftedConst::Number(value) => Literal::Number(*value),
            LiftedConst::Bool(value) => Literal::Bool(*value),
            LiftedConst::String(value) => Literal::String(value.clone()),
        };
        Expr::Literal(literal, Span::default())
    }
}

/// Lift one declared capture VALUE off the `KindedSlot` substrate into the S2
/// scalar constant domain, or a NAMED rejection with a positive twin.
///
/// The kind-dispatched scalar cascade mirrors the established
/// `literal_expr_from_slot` decode (`comptime_builtins.rs`): each accessor is
/// exact on `NativeKind` (ADR-006 §2.7.6 / Q8 — never fabricated from raw
/// bits). Non-finite numbers are rejected like every literal-materialization
/// path.
pub(in crate::compiler) fn lift_capture_value(
    name: &str,
    value: &KindedSlot,
) -> Result<LiftedConst, String> {
    if let Some(s) = value.as_str() {
        return Ok(LiftedConst::String(s.to_string()));
    }
    if let Some(i) = value.as_i64() {
        return Ok(LiftedConst::Int(i));
    }
    if let Some(n) = value.as_f64() {
        if !n.is_finite() {
            return Err(format!(
                "capture `{name}` holds a non-finite number ({n}); capture values must be \
                 finite so they can be delivered as typed literals — pass a finite number"
            ));
        }
        return Ok(LiftedConst::Number(n));
    }
    if let Some(b) = value.as_bool() {
        return Ok(LiftedConst::Bool(b));
    }
    Err(format!(
        "capture `{name}` holds a value outside this slice's ConstLift domain (kind {:?}); \
         pass an int, number, bool, or string capture value — the compositional domain \
         (tuples/arrays/Option of liftables, heap-constant baking, and the never-liftable \
         named rejections) lands with S3 ConstLift",
        value.kind()
    ))
}

/// Validate each lifted capture VALUE against the body fn's declared trailing
/// capture-parameter type (by name — the bijection was already established at
/// `CheckedTemplateBuilder::finish()`). A kind/type mismatch is a NAMED
/// rejection naming BOTH sides with a positive twin.
pub(in crate::compiler) fn validate_capture_value_types(
    body_fn_name: &str,
    capture_params: &[(String, TypeAnnotation)],
    values: &[(String, LiftedConst)],
) -> Result<(), String> {
    for (param_name, annotation) in capture_params {
        let Some((_, lifted)) = values.iter().find(|(name, _)| name == param_name) else {
            // Unreachable after the finish()-time bijection; fail loudly, not
            // silently, if a caller ever skips the chokepoint.
            return Err(format!(
                "internal error: capture parameter `{param_name}` of template body fn \
                 `{body_fn_name}` has no lifted capture value (the construction bijection \
                 was bypassed)"
            ));
        };
        if !lifted.matches_annotation(annotation) {
            return Err(format!(
                "capture `{param_name}` on template body fn `{body_fn_name}` holds a {} value \
                 but the matching trailing capture parameter is annotated `{}`; pass a `{}` \
                 value or annotate the parameter `{param_name}: {}`",
                lifted.shape_type_name(),
                annotation.to_type_string(),
                annotation.to_type_string(),
                lifted.shape_type_name(),
            ));
        }
    }
    Ok(())
}

/// How a specialized handler receives its capture values. S2 has exactly one
/// plan shape; S3's domain replacement may grow it (e.g. a `Baked` heap
/// constant) at this same boundary.
#[derive(Debug, Clone)]
pub(in crate::compiler) enum CaptureBindingPlan {
    /// The weave passes each capture value as a TYPED LITERAL argument at the
    /// wrapper's handler call sites, in the body fn's trailing-parameter
    /// order. Handlers stay value-generic; the specialization cache key is
    /// untouched (see the module docs).
    CallSiteArgs(Vec<Expr>),
}

/// Build the capture delivery plan for one install of `template` onto
/// `target` (the S2b weave consumer; S2a proves the seam). Values are
/// ordered by the body fn's TRAILING PARAMETER order — the weave appends
/// them after the signature arguments at each handler call site.
///
/// The `target` parameter anchors error attribution (the `@application`
/// span) and is the seam S3's target-aware baking slots into; the S2 scalar
/// plan itself is target-independent.
pub(in crate::compiler) fn bind_captures_for_install(
    bound: &BoundTemplate,
    target: &SpecializationTarget,
) -> Result<CaptureBindingPlan, ShapeError> {
    let template = &bound.template;
    let mut args = Vec::with_capacity(template.capture_params().len());
    for (param_name, _annotation) in template.capture_params() {
        let Some((_, lifted)) = bound
            .capture_values
            .iter()
            .find(|(name, _)| name == param_name)
        else {
            // Unreachable after the finish()-time bijection + the builtin's
            // value validation; internal-error-shaped (the specialize_template
            // precedent), anchored for the record at the application site.
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "internal error: bind_captures_for_install found no capture value for \
                     trailing parameter `{param_name}` of template body fn `{}` (install \
                     target `{}`); the construction bijection was bypassed",
                    template.body_fn(),
                    target.name,
                ),
                location: None,
            });
        };
        args.push(lifted.to_literal_expr());
    }
    Ok(CaptureBindingPlan::CallSiteArgs(args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::{NativeKind, ValueSlot};

    // ── lift_capture_value: the S2 scalar domain edges ─────────────────────

    #[test]
    fn lifts_the_four_scalars() {
        assert_eq!(
            lift_capture_value("c", &KindedSlot::from_int(42)).expect("int lifts"),
            LiftedConst::Int(42)
        );
        assert_eq!(
            lift_capture_value("c", &KindedSlot::from_number(2.5)).expect("number lifts"),
            LiftedConst::Number(2.5)
        );
        assert_eq!(
            lift_capture_value("c", &KindedSlot::from_bool(true)).expect("bool lifts"),
            LiftedConst::Bool(true)
        );
        assert_eq!(
            lift_capture_value(
                "c",
                &KindedSlot::from_string_arc(std::sync::Arc::new("hi".to_string()))
            )
            .expect("string lifts"),
            LiftedConst::String("hi".to_string())
        );
    }

    #[test]
    fn non_scalar_value_is_a_named_rejection_pointing_at_the_s3_domain() {
        // A Null-kinded slot is outside the S2 scalar domain (like unit, for
        // which no literal Expr exists — module docs); the rejection names
        // the domain and the positive twin.
        let unit = KindedSlot::new(ValueSlot::from_raw(0), NativeKind::Null);
        let err = lift_capture_value("cfg", &unit).expect_err("null is not an S2 scalar");
        assert!(
            err.contains("outside this slice's ConstLift domain"),
            "names the domain violation: {err}"
        );
        assert!(
            err.contains("pass an int, number, bool, or string"),
            "carries the positive twin: {err}"
        );
        assert!(
            err.contains("lands with S3 ConstLift"),
            "names the S3 compositional domain seam: {err}"
        );
    }

    #[test]
    fn non_finite_number_is_a_named_rejection() {
        let err = lift_capture_value("cfg", &KindedSlot::from_number(f64::NAN))
            .expect_err("non-finite numbers must reject");
        assert!(err.contains("non-finite"), "names finiteness: {err}");
        assert!(
            err.contains("pass a finite number"),
            "carries the positive twin: {err}"
        );
    }

    // ── LiftedConst: annotation matching + literal projection ──────────────

    #[test]
    fn annotation_matching_is_exact_over_scalar_spellings() {
        let int_ann = TypeAnnotation::Basic("int".to_string());
        let string_ann = TypeAnnotation::Basic("string".to_string());
        assert!(LiftedConst::Int(1).matches_annotation(&int_ann));
        assert!(!LiftedConst::Int(1).matches_annotation(&string_ann));
        assert!(LiftedConst::String("x".into()).matches_annotation(&string_ann));
        assert!(
            !LiftedConst::Bool(true)
                .matches_annotation(&TypeAnnotation::Array(Box::new(int_ann))),
            "a composite annotation never matches a scalar constant"
        );
    }

    #[test]
    fn to_literal_expr_projects_typed_literals() {
        match LiftedConst::Int(7).to_literal_expr() {
            Expr::Literal(Literal::Int(7), _) => {}
            other => panic!("expected Int literal, got {other:?}"),
        }
        match LiftedConst::String("s".into()).to_literal_expr() {
            Expr::Literal(Literal::String(s), _) if s == "s" => {}
            other => panic!("expected String literal, got {other:?}"),
        }
        match LiftedConst::Bool(false).to_literal_expr() {
            Expr::Literal(Literal::Bool(false), _) => {}
            other => panic!("expected Bool literal, got {other:?}"),
        }
        match LiftedConst::Number(1.5).to_literal_expr() {
            Expr::Literal(Literal::Number(n), _) if n == 1.5 => {}
            other => panic!("expected Number literal, got {other:?}"),
        }
    }

    // ── validate_capture_value_types ────────────────────────────────────────

    fn int_param(name: &str) -> (String, TypeAnnotation) {
        (name.to_string(), TypeAnnotation::Basic("int".to_string()))
    }

    #[test]
    fn value_type_match_is_green() {
        validate_capture_value_types(
            "t",
            &[int_param("cfg")],
            &[("cfg".to_string(), LiftedConst::Int(5))],
        )
        .expect("int value against int annotation is green");
    }

    #[test]
    fn value_type_mismatch_names_both_sides_with_positive_twin() {
        let err = validate_capture_value_types(
            "t",
            &[(
                "cfg".to_string(),
                TypeAnnotation::Basic("string".to_string()),
            )],
            &[("cfg".to_string(), LiftedConst::Int(5))],
        )
        .expect_err("int value against string annotation must reject");
        assert!(err.contains("holds a int value"), "names the value: {err}");
        assert!(err.contains("annotated `string`"), "names the declared type: {err}");
        assert!(
            err.contains("pass a `string` value"),
            "carries the positive twin: {err}"
        );
    }

    #[test]
    fn missing_value_is_an_internal_error_never_silent() {
        let err = validate_capture_value_types("t", &[int_param("cfg")], &[])
            .expect_err("a bypassed bijection must fail loudly");
        assert!(err.contains("internal error"), "internal-error-shaped: {err}");
    }

    // ── bind_captures_for_install (the S2b weave consumer's seam) ───────────

    use crate::compiler::BytecodeCompiler;
    use crate::compiler::comptime_fragments::checked_template::{
        CheckedTemplateBuilder, TemplateHookKind,
    };
    use shape_ast::ast::{CaptureClause, CaptureEntry, CaptureMode, Item};

    fn def_of(src: &str) -> shape_ast::ast::FunctionDef {
        shape_ast::parse_program(src)
            .expect("fixture parses")
            .items
            .into_iter()
            .find_map(|item| match item {
                Item::Function(func, _) => Some(func),
                _ => None,
            })
            .expect("fixture has one function")
    }

    fn move_entry(name: &str) -> CaptureEntry {
        CaptureEntry {
            mode: CaptureMode::Move,
            name: name.to_string(),
            span: shape_ast::ast::Span::default(),
            name_span: shape_ast::ast::Span::default(),
        }
    }

    fn target_of(src: &str) -> SpecializationTarget {
        let def = def_of(src);
        BytecodeCompiler::new()
            .specialization_target_from_def(&def, None, def.name_span)
            .expect("target glue builds from declared annotations")
    }

    #[test]
    fn call_site_args_deliver_in_param_order_regardless_of_binding_order() {
        // The body fn's trailing captures are (factor: int, tag: string); the
        // capture() bindings arrive in the REVERSE order — delivery follows
        // PARAMETER order (the weave contract), matching by NAME.
        let def = def_of("fn t(x: int, factor: int, tag: string) -> int { return x }");
        let template = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&def)
            .expect("shape passes")
            .captures(CaptureClause {
                entries: vec![move_entry("tag"), move_entry("factor")],
                span: shape_ast::ast::Span::default(),
            })
            .finish()
            .expect("template finishes");
        let bound = BoundTemplate {
            template,
            capture_values: vec![
                ("tag".to_string(), LiftedConst::String("hi".to_string())),
                ("factor".to_string(), LiftedConst::Int(3)),
            ],
        };
        let target = target_of("fn victim(a: int) -> int { return a }");

        let CaptureBindingPlan::CallSiteArgs(args) =
            bind_captures_for_install(&bound, &target).expect("plan builds");
        assert_eq!(args.len(), 2);
        match &args[0] {
            Expr::Literal(Literal::Int(3), _) => {}
            other => panic!("param order puts `factor` first, got {other:?}"),
        }
        match &args[1] {
            Expr::Literal(Literal::String(s), _) if s == "hi" => {}
            other => panic!("param order puts `tag` second, got {other:?}"),
        }
    }

    #[test]
    fn missing_capture_value_is_an_internal_error() {
        let def = def_of("fn t(x: int, factor: int) -> int { return x }");
        let template = CheckedTemplateBuilder::new(TemplateHookKind::Before)
            .body_fn(&def)
            .expect("shape passes")
            .captures(CaptureClause {
                entries: vec![move_entry("factor")],
                span: shape_ast::ast::Span::default(),
            })
            .finish()
            .expect("template finishes");
        let bound = BoundTemplate {
            template,
            capture_values: Vec::new(), // the bypassed-bijection shape
        };
        let target = target_of("fn victim(a: int) -> int { return a }");

        let err = bind_captures_for_install(&bound, &target)
            .expect_err("a bypassed bijection fails loudly");
        assert!(
            err.to_string().contains("internal error"),
            "internal-error-shaped: {err}"
        );
    }
}
