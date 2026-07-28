//! Machine-applicable fixes derived from proved type facts (ADR-017 §4).
//!
//! A fix produced here is evidence-backed: it materializes facts the checker
//! already proved (which variants a match is missing, what the scrutinee's
//! enum is called) into exact spans and replacement text. It never guesses,
//! and when the proved facts do not determine a safe edit the producer
//! returns `None` — the diagnostic still carries its advice, but no tool is
//! told it can apply anything.
//!
//! These fixes are the single source consumed by both the CLI
//! (`--diagnostics json`) and the LSP (code actions). Neither re-derives them
//! from rendered message text.

use shape_ast::ast::{Span, TypeAnnotation};
use shape_diagnostics::{StructuredEdit, SuggestedFix};

use super::effects::ClosedEffectRow;
use super::errors::DeclaredRowSite;
use super::types::{Type, annotation_contains_reserved_type_var_carrier};

/// Materialize the effect row a boundary needs (ADR-014 §8.2, ADR-017 §4).
///
/// The row written is `inferred` — the row the checker proved, passed as the
/// row itself. Nothing here parses a rendered message or reconstructs a row
/// from atom spelling, so the edit cannot disagree with the fact that produced
/// it. A pure inferred row materializes `! {}`, which §8.2 requires to be an
/// explicit claim rather than an omission.
///
/// Two cases, per [`DeclaredRowSite`]: a boundary that declared too narrow a
/// row has its clause replaced, one that declared none has a clause inserted.
/// Both spell the row identically, so a boundary reaches the same text however
/// it got there.
pub(crate) fn effect_row_fix(
    source: &str,
    site: &DeclaredRowSite,
    inferred: &ClosedEffectRow,
) -> Option<SuggestedFix> {
    let clause = render_clause(inferred);

    let edit = match site.clause {
        // Replace what the boundary wrote. The clause span covers the `!` and
        // the row, so the replacement is the whole claim.
        Some(span) => {
            let end = span.end.min(source.len());
            if span.start > end || !source.is_char_boundary(span.start) {
                return None;
            }
            StructuredEdit::replacement(span.start as u32, end as u32, clause.clone())
        }
        // Insert one where none was written, with the separating space the
        // surface form needs.
        None => {
            if site.insert_at > source.len() || !source.is_char_boundary(site.insert_at) {
                return None;
            }
            StructuredEdit::insertion(site.insert_at as u32, format!(" {clause}"))
        }
    };

    let label = if inferred.is_pure() {
        "Declare the boundary pure with `! {}`".to_string()
    } else {
        format!("Declare the effect row `{clause}`")
    };

    Some(SuggestedFix::new(label, 0.9).with_edits(source, vec![edit]))
}

/// The surface spelling of a closed row: `! {}`, `! {FsRead}`,
/// `! {FsRead, NetConnect}`.
///
/// `canonical_atom_names` sorts, so two rows built by inserting the same atoms
/// in different orders spell identically — the #205 determinism rule applied to
/// fix output, not just to diagnostics.
fn render_clause(row: &ClosedEffectRow) -> String {
    format!("! {}", row.render())
}

/// One position of a callable's contract that the source leaves undeclared,
/// paired with the type the checker inferred for it.
pub(crate) struct UndeclaredPosition<'a> {
    /// What the position is, for the fix's label.
    pub what: ContractPosition,
    /// Byte offset the annotation belongs at: after a parameter's name, or
    /// after the parameter list's `)` for a result.
    pub insert_at: usize,
    /// The type the checker inferred here.
    pub inferred: &'a Type,
}

/// Which part of a contract an [`UndeclaredPosition`] names.
pub(crate) enum ContractPosition {
    Param(String),
    Result,
}

/// Materialize the declared contract an annotated or exported callable is
/// missing (ADR-011 §2, ADR-014 §8.2, ADR-017 §4).
///
/// Every annotation written comes from an inferred type the checker already
/// holds. The fix is all-or-nothing: if a single position cannot be spelled
/// honestly, no edit is produced at all, because a signature with some
/// positions declared and others not is still an undeclared contract, and one
/// that declares the wrong thing is worse than one that declares nothing.
///
/// # Why this does not go through `Type::to_annotation`
///
/// `Type::to_annotation` substitutes `unknown` for a parameter or return it
/// cannot convert inside a `Type::Function` (`types/core.rs:417,422`). That is
/// tolerable for a rendered hint and fatal here: `fn(unknown) -> int` is not a
/// type, it is a sentence about the compiler's ignorance, and writing it into
/// the user's source would declare a contract that means nothing. [`spell`]
/// therefore does its own conversion and returns `None` at exactly the points
/// `to_annotation` papers over.
pub(crate) fn declared_contract_fix(
    source: &str,
    callable: &str,
    undeclared: &[UndeclaredPosition<'_>],
) -> Option<SuggestedFix> {
    if undeclared.is_empty() {
        return None;
    }

    let mut edits = Vec::with_capacity(undeclared.len());
    for position in undeclared {
        if position.insert_at > source.len() || !source.is_char_boundary(position.insert_at) {
            return None;
        }
        // `spell` refuses rather than approximates; one refusal drops the
        // whole fix.
        let text = spell(position.inferred)?;
        let written = match position.what {
            ContractPosition::Param(_) => format!(": {text}"),
            ContractPosition::Result => format!(" -> {text}"),
        };
        edits.push(StructuredEdit::insertion(
            position.insert_at as u32,
            written,
        ));
    }

    // Deterministic regardless of the order the checker visited positions in
    // (#205). `EditPlan::apply` sorts too; sorting here as well means the
    // *plan* is byte-identical across compiles, not merely its result.
    edits.sort_by_key(|edit| (edit.start(), edit.end()));

    Some(
        SuggestedFix::new(format!("Declare the contract of `{callable}`"), 0.9)
            .with_edits(source, edits),
    )
}

/// The Shape source text for an inferred type, or `None` when the type has no
/// honest spelling.
///
/// Refusal is the point. Every `None` below is a position where the compiler
/// knows less than a declaration would claim, and where a materialized
/// signature would therefore be a lie:
///
/// * an inference hole — a `TypeVar` with no declared provenance;
/// * a constrained type, whose bound is not a type;
/// * a reserved type-var carrier smuggled into an annotation;
/// * any nested occurrence of the above.
///
/// A `TypeVar` that *does* carry declared provenance is an explicit binder the
/// author wrote (`fn identity<T>(x)`), and spells as that binder's own name —
/// this is the `TypeParamRef` preservation the ADR-017 §4 fix set requires.
fn spell(ty: &Type) -> Option<String> {
    match ty {
        Type::Concrete(annotation) => spell_annotation(annotation),
        Type::Variable(var) => var
            .declared_provenance()
            .map(|provenance| provenance.source_name().to_string()),
        Type::Constrained { .. } => None,
        Type::Generic { base, args } => {
            let Type::Concrete(TypeAnnotation::Reference(name)) = base.as_ref() else {
                return None;
            };
            let spelled: Option<Vec<String>> = args.iter().map(spell).collect();
            let spelled = spelled?;
            if spelled.is_empty() {
                // A bare generic name is not a type in Shape; refuse rather
                // than write one.
                return None;
            }
            let name = name.to_string();
            let name = if name == "Vec" {
                "Array".to_string()
            } else {
                name
            };
            Some(format!("{name}<{}>", spelled.join(", ")))
        }
        Type::Function {
            params,
            returns,
            effects,
        } => {
            let spelled: Option<Vec<String>> = params.iter().map(spell).collect();
            Some(format!(
                "fn({}) -> {}{}",
                spelled?.join(", "),
                spell(returns)?,
                match effects.to_annotation() {
                    Some(row) => format!(" {}", row.render()),
                    None => String::new(),
                }
            ))
        }
    }
}

/// The Shape source text for a written annotation, or `None` when it has none.
fn spell_annotation(annotation: &TypeAnnotation) -> Option<String> {
    if annotation_contains_reserved_type_var_carrier(annotation) {
        return None;
    }
    let each = |items: &[TypeAnnotation]| -> Option<Vec<String>> {
        items.iter().map(spell_annotation).collect()
    };
    Some(match annotation {
        // `unknown` is what `Type::to_annotation` writes where it gave up. It
        // is not a Shape type, and a contract that declares it declares
        // nothing.
        TypeAnnotation::Basic(name) if name == "unknown" => return None,
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(path) => {
            let rendered = path.to_string();
            if rendered == "unknown" {
                return None;
            }
            rendered
        }
        TypeAnnotation::Generic { name, args } if !args.is_empty() => {
            let name = name.to_string();
            let name = if name == "Vec" {
                "Array".to_string()
            } else {
                name
            };
            format!("{name}<{}>", each(args)?.join(", "))
        }
        TypeAnnotation::Array(inner) => format!("Array<{}>", spell_annotation(inner)?),
        TypeAnnotation::Tuple(items) => format!("[{}]", each(items)?.join(", ")),
        TypeAnnotation::Union(items) => each(items)?.join(" | "),
        TypeAnnotation::Intersection(items) => each(items)?.join(" + "),
        TypeAnnotation::Object(fields) => {
            let mut rendered = Vec::with_capacity(fields.len());
            for field in fields {
                rendered.push(format!(
                    "{}{}: {}",
                    field.name,
                    if field.optional { "?" } else { "" },
                    spell_annotation(&field.type_annotation)?
                ));
            }
            format!("{{ {} }}", rendered.join(", "))
        }
        TypeAnnotation::Function {
            params,
            returns,
            effects,
        } => {
            let mut spelled = Vec::with_capacity(params.len());
            for param in params {
                spelled.push(spell_annotation(&param.type_annotation)?);
            }
            format!(
                "fn({}) -> {}{}",
                spelled.join(", "),
                spell_annotation(returns)?,
                match effects {
                    Some(row) => format!(" {}", row.render()),
                    None => String::new(),
                }
            )
        }
        TypeAnnotation::Borrow { mutable, inner } => format!(
            "&{}{}",
            if *mutable { "mut " } else { "" },
            spell_annotation(inner)?
        ),
        TypeAnnotation::Dyn(paths) => format!(
            "dyn {}",
            paths
                .iter()
                .map(|path| path.to_string())
                .collect::<Vec<_>>()
                .join(" + ")
        ),
        TypeAnnotation::Void => "void".to_string(),
        TypeAnnotation::Never => "never".to_string(),
        TypeAnnotation::Null => "null".to_string(),
        // No surface spelling this producer is willing to guess at.
        TypeAnnotation::Generic { .. }
        | TypeAnnotation::Existential { .. }
        | TypeAnnotation::Undefined => return None,
    })
}

/// Materialize the arms a non-exhaustive match is missing.
///
/// Emits an edit only for the block form — the match's closing brace alone
/// on its own line — where the insertion point and the arm indentation are
/// both determined by the source. A single-line match (`match c { Red => 1 }`)
/// has no unambiguous place to put an arm without also repairing the
/// preceding arm's punctuation, so it gets no edit.
pub(crate) fn non_exhaustive_match_fix(
    source: &str,
    match_span: Span,
    enum_name: &str,
    missing_variants: &[String],
) -> Option<SuggestedFix> {
    if missing_variants.is_empty() || !is_path_like(enum_name) {
        return None;
    }
    if !missing_variants.iter().all(|v| is_identifier(v)) {
        return None;
    }

    let (insert_at, arm_indent) = block_arm_insert_point(source, match_span)?;

    let mut new_text = String::new();
    for variant in missing_variants {
        new_text.push_str(&format!(
            "{arm_indent}{enum_name}::{variant} => {{\n{arm_indent}}},\n"
        ));
    }

    let label = if missing_variants.len() == 1 {
        format!("Add missing match arm for {enum_name}")
    } else {
        format!("Add missing match arms for {enum_name}")
    };

    Some(
        SuggestedFix::new(label, 0.9)
            .with_edits(source, vec![StructuredEdit::insertion(insert_at, new_text)]),
    )
}

/// Byte offset at which a new arm belongs, plus the indentation an arm at
/// that position carries.
///
/// `None` unless the match's closing brace is the first non-whitespace on its
/// line, which is what makes "insert a whole line before it" well defined.
fn block_arm_insert_point(source: &str, match_span: Span) -> Option<(u32, String)> {
    let end = match_span.end.min(source.len());
    let matched = source.get(match_span.start..end)?;
    // The span ends just past the match's closing brace.
    let brace_rel = matched.rfind('}')?;
    let brace_at = match_span.start + brace_rel;

    let line_start = source[..brace_at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indent = &source[line_start..brace_at];
    if !indent.chars().all(|c| c == ' ' || c == '\t') {
        return None;
    }

    Some((line_start as u32, format!("{indent}  ")))
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// An enum name may be module-qualified (`colors::Color`) but must not be a
/// type expression — a union scrutinee reports its whole type as the "enum
/// name", and `object | int::Foo` is not an arm.
fn is_path_like(name: &str) -> bool {
    !name.is_empty() && name.split("::").all(is_identifier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_diagnostics::EditPlan;

    const PROGRAM: &str = "enum Color { Red, Green, Blue }\n\
                           fn describe(c: Color) -> int {\n\
                           \x20 match c {\n\
                           \x20   Color::Red => 1,\n\
                           \x20 }\n\
                           }\n";

    fn match_span() -> Span {
        let start = PROGRAM.find("match c").expect("match keyword");
        let end = PROGRAM[start..].find("}\n").expect("closing brace") + start + 1;
        Span::new(start, end)
    }

    fn apply(fix: &SuggestedFix) -> String {
        fix.edit_plan
            .as_ref()
            .expect("machine-applicable")
            .apply(PROGRAM)
            .expect("applies")
    }

    #[test]
    fn materializes_one_arm_per_missing_variant() {
        let fix = non_exhaustive_match_fix(
            PROGRAM,
            match_span(),
            "Color",
            &["Green".to_string(), "Blue".to_string()],
        )
        .expect("fix");

        assert_eq!(fix.label, "Add missing match arms for Color");
        assert_eq!(
            apply(&fix),
            "enum Color { Red, Green, Blue }\n\
             fn describe(c: Color) -> int {\n\
             \x20 match c {\n\
             \x20   Color::Red => 1,\n\
             \x20   Color::Green => {\n\
             \x20   },\n\
             \x20   Color::Blue => {\n\
             \x20   },\n\
             \x20 }\n\
             }\n"
        );
    }

    #[test]
    fn arm_indent_follows_the_closing_brace() {
        let fix = non_exhaustive_match_fix(PROGRAM, match_span(), "Color", &["Blue".to_string()])
            .expect("fix");
        let inserted = &fix.edit_plan.as_ref().unwrap().edits[0].new_text;
        assert!(
            inserted.starts_with("    Color::Blue"),
            "arm indents two past the closing brace's two: {inserted:?}"
        );
        assert_eq!(fix.label, "Add missing match arm for Color");
    }

    #[test]
    fn plan_is_bound_to_the_source_it_was_proved_against() {
        let fix = non_exhaustive_match_fix(PROGRAM, match_span(), "Color", &["Blue".to_string()])
            .expect("fix");
        let plan: &EditPlan = fix.edit_plan.as_ref().unwrap();
        assert!(plan.validate(PROGRAM).is_ok());
        assert!(plan.validate(&format!("// edited\n{PROGRAM}")).is_err());
    }

    /// A union scrutinee reports its whole type where an enum name would go.
    /// `object | int::Missing` is not an arm, so no edit is offered.
    #[test]
    fn union_scrutinee_gets_no_edit() {
        assert!(
            non_exhaustive_match_fix(
                PROGRAM,
                match_span(),
                "object | int",
                &["Missing".to_string()],
            )
            .is_none()
        );
    }

    #[test]
    fn single_line_match_gets_no_edit() {
        let source = "fn f(c: Color) -> int { match c { Color::Red => 1 } }\n";
        let start = source.find("match c").unwrap();
        let end = source.rfind("} }").unwrap() + 1;
        assert!(
            non_exhaustive_match_fix(
                source,
                Span::new(start, end),
                "Color",
                &["Blue".to_string()]
            )
            .is_none(),
            "no unambiguous insertion point without repairing arm punctuation"
        );
    }

    #[test]
    fn no_missing_variants_means_no_fix() {
        assert!(non_exhaustive_match_fix(PROGRAM, match_span(), "Color", &[]).is_none());
    }

    #[test]
    fn qualified_enum_name_is_accepted() {
        assert!(
            non_exhaustive_match_fix(
                PROGRAM,
                match_span(),
                "colors::Color",
                &["Blue".to_string()],
            )
            .is_some()
        );
    }
}

#[cfg(test)]
mod effect_row_fix_tests {
    //! ADR-014 §8.2 / ADR-017 §4 — materializing the row a boundary needs.
    //!
    //! Every inferred row here comes out of the real `ConstraintSolver`, not
    //! out of a literal: the fix's input has to be the fact the checker
    //! actually produced, or the two could drift and the tests would not
    //! notice.

    use super::*;
    use crate::type_system::constraints::ConstraintSolver;
    use crate::type_system::effects::{
        EffectAtom, EffectRow, EffectStage, OperationalEffectId, resolve_row_annotation,
    };
    use crate::type_system::errors::TypeError;
    use crate::type_system::types::{BuiltinTypes, Type};
    use shape_ast::ast::{EffectRowAnnotation, TypeAnnotation};

    const FS_READ: EffectAtom = EffectAtom::Operation(OperationalEffectId::FsRead);
    const NET_CONNECT: EffectAtom = EffectAtom::Operation(OperationalEffectId::NetConnect);

    /// The canonical tracer for a type-position boundary: `narrow` accepts a
    /// callback that claims purity, and `use_it` hands it one that reads.
    const TRACER: &str = "fn narrow(cb: fn() -> int ! {}) -> int { return cb() }\n\
                          fn use_it(g: fn() -> int ! {FsRead}) -> int { return narrow(g) }\n";

    fn closed(atoms: &[EffectAtom]) -> EffectRow {
        EffectRow::Closed(
            crate::type_system::effects::ClosedEffectRow::from_atoms(
                EffectStage::Runtime,
                atoms.iter().copied(),
            )
            .unwrap(),
        )
    }

    fn callback(row: EffectRow) -> Type {
        Type::function_with_effects(vec![], BuiltinTypes::number(), row)
    }

    /// Run a real boundary check and return the error it produces.
    fn boundary_error(inferred: &[EffectAtom], declared: &[EffectAtom]) -> TypeError {
        ConstraintSolver::new()
            .check_declared_boundary(&callback(closed(inferred)), &callback(closed(declared)))
            .expect_err("the tracer's boundary must reject")
    }

    /// The span of the declared clause in `TRACER`'s `narrow` — the `! {}` the
    /// fix replaces.
    fn tracer_site() -> DeclaredRowSite {
        let start = TRACER.find("! {}").expect("declared clause");
        DeclaredRowSite::written(Span::new(start, start + "! {}".len()))
    }

    fn fix_for(error: &TypeError, source: &str, site: &DeclaredRowSite) -> SuggestedFix {
        let TypeError::EffectRowExceedsBoundary { inferred, .. } = error else {
            panic!("expected a structured boundary error, got {error:?}");
        };
        effect_row_fix(source, site, inferred).expect("the proved row materializes")
    }

    fn apply(fix: &SuggestedFix, source: &str) -> String {
        fix.edit_plan
            .as_ref()
            .expect("machine-applicable")
            .apply(source)
            .expect("applies")
    }

    /// The row a parsed program declares on `narrow`'s callback parameter,
    /// read back as a semantic value.
    fn declared_row_of(source: &str) -> EffectRow {
        let program = shape_ast::parse_program(source).expect("fixed source parses");
        for item in &program.items {
            let shape_ast::ast::program::Item::Function(function, _) = item else {
                continue;
            };
            if function.name != "narrow" {
                continue;
            }
            let Some(TypeAnnotation::Function { effects, .. }) =
                function.params[0].type_annotation.as_ref()
            else {
                panic!("`narrow`'s parameter is a function type");
            };
            let clause: &EffectRowAnnotation = effects.as_deref().expect("a declared clause");
            return resolve_row_annotation(clause, EffectStage::Runtime).expect("a resolvable row");
        }
        panic!("`narrow` is declared in the tracer");
    }

    // -- Tripwire 2 ---------------------------------------------------------

    /// The materialized row IS the checker's inferred row. Asserted on the
    /// semantic value recovered from the fixed source, not on the text: a
    /// string comparison would pass for a fix that merely happened to render
    /// the same today.
    #[test]
    fn the_materialized_row_equals_the_inferred_row_as_a_semantic_fact() {
        let error = boundary_error(&[FS_READ], &[]);
        let TypeError::EffectRowExceedsBoundary { inferred, .. } = &error else {
            panic!("structured payload");
        };

        let fixed = apply(&fix_for(&error, TRACER, &tracer_site()), TRACER);

        assert_eq!(
            declared_row_of(&fixed),
            EffectRow::Closed(inferred.clone()),
            "the row the fixed source declares must be the row the checker proved"
        );
    }

    /// And the assertion above has teeth: a row that is merely *a* row does
    /// not pass it.
    #[test]
    fn a_different_row_would_fail_the_semantic_assertion() {
        let error = boundary_error(&[FS_READ], &[]);
        let fixed = apply(&fix_for(&error, TRACER, &tracer_site()), TRACER);
        assert_ne!(
            declared_row_of(&fixed),
            closed(&[FS_READ, NET_CONNECT]),
            "a wider row must not compare equal to the proved one"
        );
    }

    // -- Tripwire 1 (the producer's half; the compile-clean half lives in the
    //    CLI's cross-consumer test, which owns a compiler) -------------------

    #[test]
    fn the_fixed_tracer_parses_and_declares_the_row_it_needed() {
        let error = boundary_error(&[FS_READ], &[]);
        let fixed = apply(&fix_for(&error, TRACER, &tracer_site()), TRACER);

        assert_eq!(
            fixed,
            "fn narrow(cb: fn() -> int ! {FsRead}) -> int { return cb() }\n\
             fn use_it(g: fn() -> int ! {FsRead}) -> int { return narrow(g) }\n"
        );
        assert!(shape_ast::parse_program(&fixed).is_ok());

        // And the boundary the fix repaired now holds under the same check
        // that rejected it.
        ConstraintSolver::new()
            .check_declared_boundary(&callback(closed(&[FS_READ])), &callback(closed(&[FS_READ])))
            .expect("the repaired boundary accepts the interior");
    }

    // -- The `! {}` case §8.2 requires to be explicit ------------------------

    #[test]
    fn a_pure_inferred_row_materializes_an_explicit_purity_claim() {
        let pure = crate::type_system::effects::ClosedEffectRow::pure(EffectStage::Runtime);
        let source = "fn narrow(cb: fn() -> int) -> int { return cb() }\n";
        let at = source.find(") -> int {").expect("return type") + ") -> int".len();

        let fix = effect_row_fix(source, &DeclaredRowSite::omitted(at), &pure).expect("fix");
        assert_eq!(fix.label, "Declare the boundary pure with `! {}`");
        assert_eq!(
            apply(&fix, source),
            "fn narrow(cb: fn() -> int) -> int ! {} { return cb() }\n"
        );
        assert!(shape_ast::parse_program(&apply(&fix, source)).is_ok());
    }

    /// An omitted row and `! {}` are different claims (§8.2), so the omitted
    /// case inserts rather than assuming the boundary meant purity all along.
    #[test]
    fn an_omitted_row_is_inserted_not_assumed() {
        let row = crate::type_system::effects::ClosedEffectRow::from_atoms(
            EffectStage::Runtime,
            [FS_READ],
        )
        .unwrap();
        let source = "fn f(cb: fn() -> int) -> int { return cb() }\n";
        let at = source.find(") -> int {").expect("return type") + ") -> int".len();

        let fixed = apply(
            &effect_row_fix(source, &DeclaredRowSite::omitted(at), &row).expect("fix"),
            source,
        );
        assert_eq!(
            fixed,
            "fn f(cb: fn() -> int) -> int ! {FsRead} { return cb() }\n"
        );
    }

    // -- Determinism (#205) --------------------------------------------------

    /// Nothing unordered reaches the fix. Two rows built by inserting the same
    /// atoms in opposite orders produce byte-identical plans, and repeating the
    /// call produces the same bytes again.
    #[test]
    fn fix_output_is_byte_deterministic_across_invocations_and_insertion_order() {
        let forward = boundary_error(&[FS_READ, NET_CONNECT], &[]);
        let backward = boundary_error(&[NET_CONNECT, FS_READ], &[]);

        let one = fix_for(&forward, TRACER, &tracer_site());
        let two = fix_for(&backward, TRACER, &tracer_site());
        let three = fix_for(&forward, TRACER, &tracer_site());

        assert_eq!(one, two, "insertion order must not reach the fix");
        assert_eq!(one, three, "a second compile must produce the same bytes");
        assert_eq!(
            one.edit_plan.as_ref().unwrap().edits[0].new_text,
            "! {FsRead, NetConnect}"
        );
    }

    // -- Refusals ------------------------------------------------------------

    #[test]
    fn a_site_past_the_end_of_the_source_produces_no_fix() {
        let row = crate::type_system::effects::ClosedEffectRow::pure(EffectStage::Runtime);
        assert!(effect_row_fix("fn f() {}", &DeclaredRowSite::omitted(9_999), &row).is_none());
    }

    #[test]
    fn a_site_inside_a_code_point_produces_no_fix() {
        let row = crate::type_system::effects::ClosedEffectRow::pure(EffectStage::Runtime);
        // Byte 1 is the middle of `é`.
        assert!(effect_row_fix("é", &DeclaredRowSite::omitted(1), &row).is_none());
    }

    /// The plan is bound to the revision it was proved against, like every
    /// other fix on this channel.
    #[test]
    fn the_plan_refuses_source_that_moved_under_it() {
        let error = boundary_error(&[FS_READ], &[]);
        let fix = fix_for(&error, TRACER, &tracer_site());
        let plan = fix.edit_plan.as_ref().expect("plan");
        assert!(plan.validate(TRACER).is_ok());
        assert!(plan.validate(&format!("// edited\n{TRACER}")).is_err());
    }
}
