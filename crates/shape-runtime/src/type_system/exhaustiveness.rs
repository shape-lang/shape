//! Exhaustiveness Checking for Match Expressions
//!
//! Implements compile-time verification that all enum variants are covered
//! in match expressions.
//!
//! Rules:
//! 1. Patterns with `where` guards do NOT contribute to exhaustiveness coverage
//! 2. Unguarded `_` (wildcard) or identifier pattern makes match exhaustive
//! 3. For enums: Uncovered = AllVariants - CoveredVariants

use super::errors::{TypeError, TypeResult};
use super::semantic::SemanticType;
use super::types::Type;

// EnumVariant is used in tests
#[cfg(test)]
use super::semantic::EnumVariant;
use super::types::annotation_to_semantic;
use shape_ast::ast::TypeAnnotation;
use shape_ast::ast::{MatchArm, MatchExpr, Pattern};
use std::collections::HashSet;

/// Result of exhaustiveness checking
#[derive(Debug, Clone, PartialEq)]
pub enum ExhaustivenessResult {
    /// Match is exhaustive (all cases covered)
    Exhaustive,
    /// Match is non-exhaustive (missing variants)
    NonExhaustive {
        enum_name: String,
        missing_variants: Vec<String>,
    },
    /// Match is trivially exhaustive (has wildcard or catch-all pattern)
    TriviallyExhaustive,
    /// Scrutinee is not an enum type (exhaustiveness not applicable)
    NotApplicable,
}

impl ExhaustivenessResult {
    /// Returns true if the match is exhaustive
    pub fn is_exhaustive(&self) -> bool {
        matches!(
            self,
            ExhaustivenessResult::Exhaustive
                | ExhaustivenessResult::TriviallyExhaustive
                | ExhaustivenessResult::NotApplicable
        )
    }

    /// Convert to a TypeError if non-exhaustive
    pub fn to_error(&self) -> Option<TypeError> {
        match self {
            ExhaustivenessResult::NonExhaustive {
                enum_name,
                missing_variants,
            } => Some(TypeError::NonExhaustiveMatch {
                enum_name: enum_name.clone(),
                missing_variants: missing_variants.clone(),
            }),
            _ => None,
        }
    }
}

/// Check exhaustiveness of a match expression
pub fn check_exhaustiveness(
    match_expr: &MatchExpr,
    scrutinee_type: &SemanticType,
) -> ExhaustivenessResult {
    // Only check enums for now - other types are either trivially exhaustive
    // or require more sophisticated pattern analysis
    let (enum_name, variants) = match scrutinee_type {
        SemanticType::Enum { name, variants, .. } => (name.clone(), variants.clone()),
        // For non-enum types, check if there's a wildcard pattern
        _ => {
            if has_unguarded_catch_all(&match_expr.arms) {
                return ExhaustivenessResult::TriviallyExhaustive;
            }
            return ExhaustivenessResult::NotApplicable;
        }
    };

    // Collect all covered variants from unguarded patterns
    let covered = collect_covered_variants(&match_expr.arms, &enum_name);

    // Check for trivial exhaustiveness (wildcard or catch-all)
    if has_unguarded_catch_all(&match_expr.arms) {
        return ExhaustivenessResult::TriviallyExhaustive;
    }

    // Compute missing variants
    let all_variants: HashSet<_> = variants.iter().map(|v| v.name.clone()).collect();
    let missing: Vec<_> = all_variants.difference(&covered).cloned().collect();

    if missing.is_empty() {
        ExhaustivenessResult::Exhaustive
    } else {
        ExhaustivenessResult::NonExhaustive {
            enum_name,
            missing_variants: missing,
        }
    }
}

/// Check exhaustiveness from inference-level type information.
///
/// This supports enum exhaustiveness and closed union exhaustiveness.
pub fn check_exhaustiveness_for_type(
    match_expr: &MatchExpr,
    scrutinee_type: &Type,
) -> ExhaustivenessResult {
    if let Some(TypeAnnotation::Union(variants)) = scrutinee_type.to_annotation() {
        return check_union_exhaustiveness(match_expr, &variants);
    }

    if let Some(semantic_type) = scrutinee_type.to_semantic() {
        return check_exhaustiveness(match_expr, &semantic_type);
    }

    if has_unguarded_catch_all(&match_expr.arms) {
        ExhaustivenessResult::TriviallyExhaustive
    } else {
        // Type inference could not resolve the scrutinee type, so exhaustiveness
        // checking is skipped. This can mask missing match arms at compile time.
        tracing::debug!(
            "exhaustiveness check skipped: scrutinee type {:?} could not be resolved",
            scrutinee_type
        );
        ExhaustivenessResult::NotApplicable
    }
}

fn check_union_exhaustiveness(
    match_expr: &MatchExpr,
    union_variants: &[TypeAnnotation],
) -> ExhaustivenessResult {
    if has_unguarded_catch_all(&match_expr.arms) {
        return ExhaustivenessResult::TriviallyExhaustive;
    }

    let covered_types = collect_covered_union_types(&match_expr.arms);
    let missing: Vec<TypeAnnotation> = union_variants
        .iter()
        .filter(|variant| {
            !covered_types
                .iter()
                .any(|covered| types_match(covered, variant))
        })
        .cloned()
        .collect();

    if missing.is_empty() {
        ExhaustivenessResult::Exhaustive
    } else {
        ExhaustivenessResult::NonExhaustive {
            enum_name: format_union_type_name(union_variants),
            missing_variants: missing.iter().map(format_type_annotation).collect(),
        }
    }
}

fn collect_covered_union_types(arms: &[MatchArm]) -> Vec<TypeAnnotation> {
    let mut covered = Vec::new();

    for arm in arms {
        // Guarded arms do not contribute to exhaustiveness
        if arm.guard.is_some() {
            continue;
        }

        if let Pattern::Typed {
            type_annotation, ..
        } = &arm.pattern
        {
            for ty in flatten_union_annotation(type_annotation) {
                if !covered.iter().any(|existing| types_match(existing, ty)) {
                    covered.push(ty.clone());
                }
            }
        }
    }

    covered
}

fn flatten_union_annotation(ann: &TypeAnnotation) -> Vec<&TypeAnnotation> {
    match ann {
        TypeAnnotation::Union(types) => {
            let mut out = Vec::new();
            for ty in types {
                out.extend(flatten_union_annotation(ty));
            }
            out
        }
        _ => vec![ann],
    }
}

fn types_match(a: &TypeAnnotation, b: &TypeAnnotation) -> bool {
    annotation_to_semantic(a) == annotation_to_semantic(b)
}

fn format_union_type_name(types: &[TypeAnnotation]) -> String {
    types
        .iter()
        .map(format_type_annotation)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn format_type_annotation(ann: &TypeAnnotation) -> String {
    match ann {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(name) => name.to_string(),
        TypeAnnotation::Array(inner) => format!("Vec<{}>", format_type_annotation(inner)),
        TypeAnnotation::Tuple(elems) => format!(
            "[{}]",
            elems
                .iter()
                .map(format_type_annotation)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeAnnotation::Object(_) => "object".to_string(),
        TypeAnnotation::Function { .. } => "function".to_string(),
        TypeAnnotation::Union(types) => types
            .iter()
            .map(format_type_annotation)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeAnnotation::Intersection(types) => types
            .iter()
            .map(format_type_annotation)
            .collect::<Vec<_>>()
            .join(" + "),
        TypeAnnotation::Generic { name, args } => {
            if args.is_empty() {
                name.to_string()
            } else {
                format!(
                    "{}<{}>",
                    name,
                    args.iter()
                        .map(format_type_annotation)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        TypeAnnotation::Void => "void".to_string(),
        TypeAnnotation::Never => "never".to_string(),
        TypeAnnotation::Null => "None".to_string(),
        TypeAnnotation::Undefined => "undefined".to_string(),
        TypeAnnotation::Dyn(traits) => format!("dyn {}", traits.join(" + ")),
    }
}

/// Check if the match has an unguarded catch-all pattern
fn has_unguarded_catch_all(arms: &[MatchArm]) -> bool {
    arms.iter().any(|arm| {
        // Only unguarded patterns count
        if arm.guard.is_some() {
            return false;
        }
        is_catch_all_pattern(&arm.pattern)
    })
}

/// Check if a pattern is a catch-all (matches everything)
fn is_catch_all_pattern(pattern: &Pattern) -> bool {
    match pattern {
        // Wildcard matches everything
        Pattern::Wildcard => true,
        // Identifier without guard matches everything
        Pattern::Identifier(_) => true,
        // Other patterns are not catch-all
        _ => false,
    }
}

/// Collect all variant names covered by unguarded constructor patterns
fn collect_covered_variants(arms: &[MatchArm], enum_name: &str) -> HashSet<String> {
    let mut covered = HashSet::new();

    for arm in arms {
        // Patterns with guards do NOT contribute to exhaustiveness
        if arm.guard.is_some() {
            continue;
        }

        if let Some(variant_name) = extract_variant_name(&arm.pattern, enum_name) {
            covered.insert(variant_name);
        }
    }

    covered
}

/// Extract the variant name from a constructor pattern
fn extract_variant_name(pattern: &Pattern, expected_enum: &str) -> Option<String> {
    match pattern {
        Pattern::Constructor {
            enum_name, variant, ..
        } => {
            // Check if this matches the expected enum
            match enum_name {
                Some(name) if name == expected_enum => Some(variant.clone()),
                None => Some(variant.clone()), // Allow unqualified variant names
                _ => None,
            }
        }
        _ => None,
    }
}

/// Check a match expression and return an error if non-exhaustive
pub fn require_exhaustive(match_expr: &MatchExpr, scrutinee_type: &SemanticType) -> TypeResult<()> {
    let result = check_exhaustiveness(match_expr, scrutinee_type);
    match result.to_error() {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::{Expr, Literal, Span};

    fn make_span() -> Span {
        Span { start: 0, end: 0 }
    }

    fn make_enum_type(name: &str, variants: &[&str]) -> SemanticType {
        SemanticType::Enum {
            name: name.to_string(),
            variants: variants
                .iter()
                .map(|v| EnumVariant {
                    name: v.to_string(),
                    payload: None,
                })
                .collect(),
            type_params: vec![],
        }
    }

    fn make_match_arm(pattern: Pattern, guard: Option<Expr>, body: Expr) -> MatchArm {
        MatchArm {
            pattern,
            guard: guard.map(Box::new),
            body: Box::new(body),
            pattern_span: None,
        }
    }

    fn make_constructor_pattern(enum_name: Option<&str>, variant: &str) -> Pattern {
        Pattern::Constructor {
            enum_name: enum_name.map(|s| s.into()),
            variant: variant.to_string(),
            fields: shape_ast::ast::PatternConstructorFields::Unit,
        }
    }

    fn make_string_expr(s: &str) -> Expr {
        Expr::Literal(Literal::String(s.to_string()), make_span())
    }

    #[test]
    fn test_exhaustive_match_all_variants() {
        let status_type = make_enum_type("Status", &["Active", "Inactive"]);
        let match_expr = MatchExpr {
            scrutinee: Box::new(Expr::Identifier("status".to_string(), make_span())),
            arms: vec![
                make_match_arm(
                    make_constructor_pattern(Some("Status"), "Active"),
                    None,
                    make_string_expr("yes"),
                ),
                make_match_arm(
                    make_constructor_pattern(Some("Status"), "Inactive"),
                    None,
                    make_string_expr("no"),
                ),
            ],
        };

        let result = check_exhaustiveness(&match_expr, &status_type);
        assert_eq!(result, ExhaustivenessResult::Exhaustive);
    }

    #[test]
    fn test_non_exhaustive_missing_variant() {
        let status_type = make_enum_type("Status", &["Active", "Inactive"]);
        let match_expr = MatchExpr {
            scrutinee: Box::new(Expr::Identifier("status".to_string(), make_span())),
            arms: vec![make_match_arm(
                make_constructor_pattern(Some("Status"), "Active"),
                None,
                make_string_expr("yes"),
            )],
        };

        let result = check_exhaustiveness(&match_expr, &status_type);
        match result {
            ExhaustivenessResult::NonExhaustive {
                enum_name,
                missing_variants,
            } => {
                assert_eq!(enum_name, "Status");
                assert_eq!(missing_variants, vec!["Inactive"]);
            }
            _ => panic!("Expected NonExhaustive"),
        }
    }

    #[test]
    fn test_exhaustive_with_wildcard() {
        let status_type = make_enum_type("Status", &["Active", "Inactive", "Pending"]);
        let match_expr = MatchExpr {
            scrutinee: Box::new(Expr::Identifier("status".to_string(), make_span())),
            arms: vec![
                make_match_arm(
                    make_constructor_pattern(Some("Status"), "Active"),
                    None,
                    make_string_expr("yes"),
                ),
                make_match_arm(Pattern::Wildcard, None, make_string_expr("no")),
            ],
        };

        let result = check_exhaustiveness(&match_expr, &status_type);
        assert_eq!(result, ExhaustivenessResult::TriviallyExhaustive);
    }

    #[test]
    fn test_guarded_pattern_does_not_count() {
        let status_type = make_enum_type("Status", &["Active", "Inactive"]);
        // Pattern with guard should not contribute to exhaustiveness
        let match_expr = MatchExpr {
            scrutinee: Box::new(Expr::Identifier("status".to_string(), make_span())),
            arms: vec![
                make_match_arm(
                    make_constructor_pattern(Some("Status"), "Active"),
                    Some(Expr::Literal(Literal::Bool(true), make_span())),
                    make_string_expr("yes"),
                ),
                make_match_arm(
                    make_constructor_pattern(Some("Status"), "Inactive"),
                    None,
                    make_string_expr("no"),
                ),
            ],
        };

        let result = check_exhaustiveness(&match_expr, &status_type);
        match result {
            ExhaustivenessResult::NonExhaustive {
                missing_variants, ..
            } => {
                assert!(missing_variants.contains(&"Active".to_string()));
            }
            _ => panic!("Expected NonExhaustive because guarded Active doesn't count"),
        }
    }

    #[test]
    fn test_non_enum_with_wildcard_is_exhaustive() {
        let number_type = SemanticType::Number;
        let match_expr = MatchExpr {
            scrutinee: Box::new(Expr::Identifier("x".to_string(), make_span())),
            arms: vec![
                make_match_arm(
                    Pattern::Literal(Literal::Number(1.0)),
                    None,
                    make_string_expr("one"),
                ),
                make_match_arm(Pattern::Wildcard, None, make_string_expr("other")),
            ],
        };

        let result = check_exhaustiveness(&match_expr, &number_type);
        assert_eq!(result, ExhaustivenessResult::TriviallyExhaustive);
    }

    #[test]
    fn test_union_typed_patterns_are_exhaustive() {
        let union_type = Type::Concrete(TypeAnnotation::Union(vec![
            TypeAnnotation::Basic("int".to_string()),
            TypeAnnotation::Basic("string".to_string()),
        ]));
        let match_expr = MatchExpr {
            scrutinee: Box::new(Expr::Identifier("x".to_string(), make_span())),
            arms: vec![
                make_match_arm(
                    Pattern::Typed {
                        name: "n".to_string(),
                        type_annotation: TypeAnnotation::Basic("int".to_string()),
                    },
                    None,
                    make_string_expr("int"),
                ),
                make_match_arm(
                    Pattern::Typed {
                        name: "s".to_string(),
                        type_annotation: TypeAnnotation::Basic("string".to_string()),
                    },
                    None,
                    make_string_expr("string"),
                ),
            ],
        };

        let result = check_exhaustiveness_for_type(&match_expr, &union_type);
        assert_eq!(result, ExhaustivenessResult::Exhaustive);
    }

    #[test]
    fn test_union_typed_patterns_missing_variant_reports_non_exhaustive() {
        let union_type = Type::Concrete(TypeAnnotation::Union(vec![
            TypeAnnotation::Basic("int".to_string()),
            TypeAnnotation::Basic("string".to_string()),
        ]));
        let match_expr = MatchExpr {
            scrutinee: Box::new(Expr::Identifier("x".to_string(), make_span())),
            arms: vec![make_match_arm(
                Pattern::Typed {
                    name: "n".to_string(),
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                },
                None,
                make_string_expr("int"),
            )],
        };

        let result = check_exhaustiveness_for_type(&match_expr, &union_type);
        match result {
            ExhaustivenessResult::NonExhaustive {
                enum_name,
                missing_variants,
            } => {
                assert_eq!(enum_name, "int | string");
                assert_eq!(missing_variants, vec!["string"]);
            }
            other => panic!("Expected NonExhaustive, got {:?}", other),
        }
    }
}
