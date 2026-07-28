//! Normalized types for published base contracts.
//!
//! A `TypeAnnotation` is surface syntax: `Vec<T>`, `T[]` and `Array<T>` are
//! three spellings of one type. The published contract records the normalized
//! type so that two spellings of the same contract produce the same content
//! identity, and two different contracts never do.
//!
//! Normalization here is syntactic: it canonicalizes spelling, it does not
//! resolve type names to declarations. Name resolution for types is a later
//! slice (this one publishes callable identity, not type identity), so a named
//! type is normalized to its written path and marked as unresolved in the
//! contract's provenance.

use shape_ast::ast::types::TypeAnnotation;

use crate::identity::{CanonicalDigest, DigestWriter};

/// A declared effect row as published in a base contract (ADR-014 §8.3).
///
/// The §8.3 schema-versus-fact distinction lives here. A generic declared
/// contract is a SCHEMA and may publish [`NormalizedEffectRow::Param`]
/// binders, exactly as it publishes type binders. A closed row FACT may not:
/// [`NormalizedEffectRow::is_closed_fact`] is the predicate that separates
/// them, and ADR-010 §13 requires every fact to pass it.
///
/// Atom names are sorted at construction. Nothing here iterates an unordered
/// container, so the rendered row and the digest are stable across runs.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum NormalizedEffectRow {
    /// A closed row: sorted, deduplicated canonical atom names. The empty
    /// vector is the explicit purity claim `! {}`.
    Closed(Vec<String>),
    /// An `effect F` binder. Legal in a published schema, never in a fact.
    Param(String),
    /// The declaration spelled no row. Distinct from `Closed(vec![])`: it is
    /// the absence of a claim, not a claim of purity.
    Undeclared,
}

impl NormalizedEffectRow {
    /// Normalize a surface clause. Atom names are sorted and deduplicated so
    /// `! {NetConnect, FsRead}` and `! {FsRead, NetConnect}` publish the same
    /// contract, as ADR-014 §1's canonical-set requirement demands.
    pub fn from_annotation(annotation: Option<&shape_ast::ast::EffectRowAnnotation>) -> Self {
        use shape_ast::ast::EffectRowAnnotation as Ann;
        match annotation {
            None => NormalizedEffectRow::Undeclared,
            Some(Ann::Atoms { names, .. }) => {
                let mut sorted: Vec<String> = names.clone();
                sorted.sort();
                sorted.dedup();
                NormalizedEffectRow::Closed(sorted)
            }
            Some(Ann::Param { name, .. }) => NormalizedEffectRow::Param(name.clone()),
        }
    }

    /// True iff this row may appear in a persisted closed-row FACT
    /// (ADR-010 §13: open rows close before materialization).
    pub fn is_closed_fact(&self) -> bool {
        matches!(self, NormalizedEffectRow::Closed(_))
    }

    /// The binder this row references, if it is one.
    pub fn unbound_parameter(&self) -> Option<&str> {
        match self {
            NormalizedEffectRow::Param(name) => Some(name),
            _ => None,
        }
    }

    pub fn render(&self) -> String {
        match self {
            NormalizedEffectRow::Closed(atoms) => format!(" ! {{{}}}", atoms.join(", ")),
            NormalizedEffectRow::Param(name) => format!(" ! {name}"),
            NormalizedEffectRow::Undeclared => String::new(),
        }
    }
}

impl CanonicalDigest for NormalizedEffectRow {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        match self {
            NormalizedEffectRow::Closed(atoms) => {
                writer.u8(40);
                writer.seq(atoms);
            }
            NormalizedEffectRow::Param(name) => {
                writer.u8(41);
                writer.str(name);
            }
            NormalizedEffectRow::Undeclared => writer.u8(42),
        }
    }
}

/// A canonicalized type as published in a base contract.
#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum NormalizedType {
    Int,
    Number,
    Bool,
    String,
    Decimal,
    BigInt,
    Void,
    Never,
    Null,
    Array(Box<NormalizedType>),
    Optional(Box<NormalizedType>),
    Tuple(Vec<NormalizedType>),
    /// Structural object type; fields are sorted by name for canonical form.
    Object(Vec<(String, NormalizedType)>),
    Union(Vec<NormalizedType>),
    Intersection(Vec<NormalizedType>),
    Function {
        params: Vec<NormalizedType>,
        result: Box<NormalizedType>,
        /// ADR-014 §8.1: the declared effect row is part of the contract, so
        /// it is part of the normalized form and covered by the digest. Two
        /// callables that differ only in row publish different contracts.
        effects: NormalizedEffectRow,
    },
    Reference {
        mutable: bool,
        inner: Box<NormalizedType>,
    },
    Dyn(Vec<String>),
    /// A named type written as `path<args...>`. The path is the source-written
    /// path: this slice publishes callable identity only, so type names are not
    /// yet resolved to definition identities.
    Named {
        path: String,
        args: Vec<NormalizedType>,
    },
    /// The annotation was absent. Published explicitly instead of being
    /// silently inferred: body inference is outside this slice.
    NotDeclared,
    /// A surface form this slice does not normalize. Carries the form's name so
    /// the gap is visible in the published fact instead of being approximated.
    Unsupported(String),
}

impl NormalizedType {
    /// Renders the canonical spelling of a normalized type. Presentation only.
    pub fn render(&self) -> String {
        match self {
            NormalizedType::Int => "int".to_string(),
            NormalizedType::Number => "number".to_string(),
            NormalizedType::Bool => "bool".to_string(),
            NormalizedType::String => "string".to_string(),
            NormalizedType::Decimal => "decimal".to_string(),
            NormalizedType::BigInt => "bigint".to_string(),
            NormalizedType::Void => "void".to_string(),
            NormalizedType::Never => "never".to_string(),
            NormalizedType::Null => "null".to_string(),
            NormalizedType::Array(inner) => format!("Array<{}>", inner.render()),
            NormalizedType::Optional(inner) => format!("Option<{}>", inner.render()),
            NormalizedType::Tuple(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(NormalizedType::render)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            NormalizedType::Object(fields) => format!(
                "{{ {} }}",
                fields
                    .iter()
                    .map(|(name, ty)| format!("{name}: {}", ty.render()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            NormalizedType::Union(items) => items
                .iter()
                .map(NormalizedType::render)
                .collect::<Vec<_>>()
                .join(" | "),
            NormalizedType::Intersection(items) => items
                .iter()
                .map(NormalizedType::render)
                .collect::<Vec<_>>()
                .join(" + "),
            NormalizedType::Function {
                params,
                result,
                effects,
            } => format!(
                "({}) => {}{}",
                params
                    .iter()
                    .map(NormalizedType::render)
                    .collect::<Vec<_>>()
                    .join(", "),
                result.render(),
                effects.render()
            ),
            NormalizedType::Reference { mutable, inner } => {
                if *mutable {
                    format!("&mut {}", inner.render())
                } else {
                    format!("&{}", inner.render())
                }
            }
            NormalizedType::Dyn(traits) => format!("dyn {}", traits.join(" + ")),
            NormalizedType::Named { path, args } if args.is_empty() => path.clone(),
            NormalizedType::Named { path, args } => format!(
                "{path}<{}>",
                args.iter()
                    .map(NormalizedType::render)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            NormalizedType::NotDeclared => "<not declared>".to_string(),
            NormalizedType::Unsupported(form) => format!("<unsupported: {form}>"),
        }
    }

    /// Normalizes a written annotation, or `NotDeclared` when absent.
    pub fn from_annotation(annotation: Option<&TypeAnnotation>) -> NormalizedType {
        match annotation {
            None => NormalizedType::NotDeclared,
            Some(annotation) => normalize(annotation),
        }
    }
}

fn normalize(annotation: &TypeAnnotation) -> NormalizedType {
    match annotation {
        TypeAnnotation::Basic(name) => normalize_name(name, Vec::new()),
        TypeAnnotation::Array(inner) => NormalizedType::Array(Box::new(normalize(inner))),
        TypeAnnotation::Tuple(items) => {
            NormalizedType::Tuple(items.iter().map(normalize).collect())
        }
        TypeAnnotation::Object(fields) => {
            let mut normalized: Vec<(String, NormalizedType)> = fields
                .iter()
                .map(|field| {
                    let ty = normalize(&field.type_annotation);
                    let ty = if field.optional {
                        NormalizedType::Optional(Box::new(ty))
                    } else {
                        ty
                    };
                    (field.name.clone(), ty)
                })
                .collect();
            normalized.sort();
            NormalizedType::Object(normalized)
        }
        TypeAnnotation::Function {
            params,
            returns,
            effects,
        } => NormalizedType::Function {
            params: params
                .iter()
                .map(|param| {
                    let ty = normalize(&param.type_annotation);
                    if param.optional {
                        NormalizedType::Optional(Box::new(ty))
                    } else {
                        ty
                    }
                })
                .collect(),
            result: Box::new(normalize(returns)),
            effects: NormalizedEffectRow::from_annotation(effects.as_deref()),
        },
        TypeAnnotation::Union(items) => {
            let mut normalized: Vec<NormalizedType> = items.iter().map(normalize).collect();
            normalized.sort();
            normalized.dedup();
            if normalized.len() == 1 {
                normalized.remove(0)
            } else {
                NormalizedType::Union(normalized)
            }
        }
        TypeAnnotation::Intersection(items) => {
            let mut normalized: Vec<NormalizedType> = items.iter().map(normalize).collect();
            normalized.sort();
            normalized.dedup();
            NormalizedType::Intersection(normalized)
        }
        TypeAnnotation::Generic { name, args } => {
            normalize_name(name.as_str(), args.iter().map(normalize).collect())
        }
        TypeAnnotation::Reference(path) => normalize_name(path.as_str(), Vec::new()),
        TypeAnnotation::Borrow { mutable, inner } => NormalizedType::Reference {
            mutable: *mutable,
            inner: Box::new(normalize(inner)),
        },
        TypeAnnotation::Void => NormalizedType::Void,
        TypeAnnotation::Never => NormalizedType::Never,
        TypeAnnotation::Null => NormalizedType::Null,
        TypeAnnotation::Undefined => NormalizedType::Unsupported("undefined".to_string()),
        TypeAnnotation::Dyn(paths) => {
            let mut traits: Vec<String> = paths.iter().map(|p| p.as_str().to_string()).collect();
            traits.sort();
            NormalizedType::Dyn(traits)
        }
        TypeAnnotation::Existential { .. } => {
            NormalizedType::Unsupported("existential".to_string())
        }
    }
}

/// Maps a written type name plus normalized arguments onto the canonical form.
///
/// The primitive names are Shape's built-in scalar types; `Array`/`Vec` and
/// `Option` are the two generic spellings with a dedicated canonical shape.
fn normalize_name(name: &str, args: Vec<NormalizedType>) -> NormalizedType {
    match (name, args.len()) {
        ("int", 0) => NormalizedType::Int,
        ("number", 0) => NormalizedType::Number,
        ("bool", 0) => NormalizedType::Bool,
        ("string", 0) => NormalizedType::String,
        ("decimal", 0) => NormalizedType::Decimal,
        ("bigint", 0) => NormalizedType::BigInt,
        ("void", 0) => NormalizedType::Void,
        ("never", 0) => NormalizedType::Never,
        ("null", 0) => NormalizedType::Null,
        ("Array" | "Vec", 1) => NormalizedType::Array(Box::new(args.into_iter().next().unwrap())),
        ("Option", 1) => NormalizedType::Optional(Box::new(args.into_iter().next().unwrap())),
        _ => NormalizedType::Named {
            path: name.to_string(),
            args,
        },
    }
}

impl CanonicalDigest for NormalizedType {
    fn write_canonical(&self, writer: &mut DigestWriter) {
        // Tags are frozen: adding a variant appends a new tag, it never
        // renumbers an existing one.
        match self {
            NormalizedType::Int => writer.u8(1),
            NormalizedType::Number => writer.u8(2),
            NormalizedType::Bool => writer.u8(3),
            NormalizedType::String => writer.u8(4),
            NormalizedType::Decimal => writer.u8(5),
            NormalizedType::BigInt => writer.u8(6),
            NormalizedType::Void => writer.u8(7),
            NormalizedType::Never => writer.u8(8),
            NormalizedType::Null => writer.u8(9),
            NormalizedType::Array(inner) => {
                writer.u8(10);
                writer.nested(inner.as_ref());
            }
            NormalizedType::Optional(inner) => {
                writer.u8(11);
                writer.nested(inner.as_ref());
            }
            NormalizedType::Tuple(items) => {
                writer.u8(12);
                writer.seq(items);
            }
            NormalizedType::Object(fields) => {
                writer.u8(13);
                writer.seq(fields);
            }
            NormalizedType::Union(items) => {
                writer.u8(14);
                writer.seq(items);
            }
            NormalizedType::Intersection(items) => {
                writer.u8(15);
                writer.seq(items);
            }
            NormalizedType::Function {
                params,
                result,
                effects,
            } => {
                writer.u8(16);
                writer.seq(params);
                writer.nested(result.as_ref());
                writer.nested(effects);
            }
            NormalizedType::Reference { mutable, inner } => {
                writer.u8(17);
                writer.bool(*mutable);
                writer.nested(inner.as_ref());
            }
            NormalizedType::Dyn(traits) => {
                writer.u8(18);
                writer.seq(traits);
            }
            NormalizedType::Named { path, args } => {
                writer.u8(19);
                writer.str(path);
                writer.seq(args);
            }
            NormalizedType::NotDeclared => writer.u8(20),
            NormalizedType::Unsupported(form) => {
                writer.u8(21);
                writer.str(form);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::type_path::TypePath;

    fn generic(name: &str, args: Vec<TypeAnnotation>) -> TypeAnnotation {
        TypeAnnotation::Generic {
            name: TypePath::simple(name),
            args,
        }
    }

    #[test]
    fn array_spellings_normalize_to_one_type() {
        let bracketed = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("int".to_string())));
        let generic_vec = generic("Vec", vec![TypeAnnotation::Basic("int".to_string())]);
        let generic_array = generic("Array", vec![TypeAnnotation::Basic("int".to_string())]);
        let expected = NormalizedType::Array(Box::new(NormalizedType::Int));
        assert_eq!(normalize(&bracketed), expected);
        assert_eq!(normalize(&generic_vec), expected);
        assert_eq!(normalize(&generic_array), expected);
    }

    #[test]
    fn int_and_number_stay_distinct() {
        assert_ne!(
            normalize(&TypeAnnotation::Basic("int".to_string())),
            normalize(&TypeAnnotation::Basic("number".to_string()))
        );
    }

    #[test]
    fn unknown_names_keep_their_written_path() {
        assert_eq!(
            normalize(&TypeAnnotation::Reference(TypePath::from_qualified(
                "app::Money"
            ))),
            NormalizedType::Named {
                path: "app::Money".to_string(),
                args: vec![],
            }
        );
    }

    #[test]
    fn missing_annotation_is_published_not_guessed() {
        assert_eq!(
            NormalizedType::from_annotation(None),
            NormalizedType::NotDeclared
        );
    }

    #[test]
    fn distinct_types_have_distinct_digests() {
        let int = NormalizedType::Int.canonical_digest("test");
        let number = NormalizedType::Number.canonical_digest("test");
        let array_of_int =
            NormalizedType::Array(Box::new(NormalizedType::Int)).canonical_digest("test");
        assert_ne!(int, number);
        assert_ne!(int, array_of_int);
    }
}

#[cfg(test)]
mod effect_row_contract_tests {
    //! Tracer case (c) at the layer where it actually matters (issue #178,
    //! ADR-014 §8.3, ADR-010 §13): the PUBLISHED contract.
    //!
    //! The acceptance criterion asks for proof on the persisted
    //! representation, not merely for the absence of errors during checking.
    //! `NormalizedType` is that representation — it is what a base contract
    //! publishes and what the canonical digest covers — so these assertions
    //! read the published row itself.

    use super::*;
    use shape_ast::ast::{EffectRowAnnotation, FunctionParam, TypeAnnotation};

    fn callback(row: Option<EffectRowAnnotation>) -> TypeAnnotation {
        TypeAnnotation::Function {
            params: vec![],
            returns: Box::new(TypeAnnotation::Basic("int".to_string())),
            effects: row.map(Box::new),
        }
    }

    fn atoms(names: &[&str]) -> Option<EffectRowAnnotation> {
        Some(EffectRowAnnotation::Atoms {
            names: names.iter().map(|n| n.to_string()).collect(),
            span: Default::default(),
        })
    }

    fn binder(name: &str) -> Option<EffectRowAnnotation> {
        Some(EffectRowAnnotation::Param {
            name: name.to_string(),
            span: Default::default(),
        })
    }

    fn published_row(annotation: &TypeAnnotation) -> NormalizedEffectRow {
        match normalize(annotation) {
            NormalizedType::Function { effects, .. } => effects,
            other => panic!("expected a normalized function type, got {other:?}"),
        }
    }

    #[test]
    fn a_generic_schema_may_publish_an_unclosed_binder() {
        // `fn apply<T, effect F>(f: fn() -> T ! F)` — the generic
        // DEFINITION's contract is a schema, and §8.3 lets it persist with
        // its binders exactly as it persists type binders.
        let row = published_row(&callback(binder("F")));
        assert_eq!(row.unbound_parameter(), Some("F"));
        assert!(row.is_closed_fact() == false);
    }

    #[test]
    fn no_unbound_effect_parameter_survives_into_a_closed_row_fact() {
        // THE ACCEPTANCE ASSERTION. Every row a FACT may carry must pass
        // `is_closed_fact`, and the binder form is the one that must not.
        let schema = published_row(&callback(binder("F")));
        let instantiated = published_row(&callback(atoms(&["FsRead"])));
        let pure = published_row(&callback(atoms(&[])));

        assert!(
            !schema.is_closed_fact(),
            "an unbound binder must never qualify as a closed row fact"
        );
        assert!(instantiated.is_closed_fact());
        assert!(pure.is_closed_fact());

        // And the closed forms carry no residual parameter at all.
        assert_eq!(instantiated.unbound_parameter(), None);
        assert_eq!(pure.unbound_parameter(), None);
    }

    #[test]
    fn an_undeclared_row_is_not_a_purity_claim() {
        let undeclared = published_row(&callback(None));
        let explicit_pure = published_row(&callback(atoms(&[])));
        assert_ne!(undeclared, explicit_pure);
        assert!(!undeclared.is_closed_fact());
        assert_eq!(explicit_pure, NormalizedEffectRow::Closed(vec![]));
    }

    #[test]
    fn the_row_participates_in_the_published_contract_digest() {
        // ADR-014 §8.1: two callables differing ONLY in row are different
        // types, so they must not publish the same contract identity.
        let fs_read = normalize(&callback(atoms(&["FsRead"])));
        let net = normalize(&callback(atoms(&["NetConnect"])));
        let pure = normalize(&callback(atoms(&[])));
        let undeclared = normalize(&callback(None));

        let d = |t: &NormalizedType| t.canonical_digest("test");
        assert_ne!(d(&fs_read), d(&net));
        assert_ne!(d(&fs_read), d(&pure));
        assert_ne!(d(&pure), d(&undeclared));
    }

    #[test]
    fn atom_order_does_not_change_the_published_contract() {
        // #205: the published row is a canonical SET. Two spellings of the
        // same row must digest and render identically, or contract identity
        // would depend on how the author happened to type it.
        let forward = normalize(&callback(atoms(&["FsRead", "NetConnect"])));
        let backward = normalize(&callback(atoms(&["NetConnect", "FsRead"])));
        assert_eq!(forward, backward);
        assert_eq!(
            forward.canonical_digest("test"),
            backward.canonical_digest("test")
        );
        assert_eq!(forward.render(), "() => int ! {FsRead, NetConnect}");
    }

    #[test]
    fn a_repeated_atom_is_deduplicated_before_publication() {
        let repeated = published_row(&callback(atoms(&["FsRead", "FsRead"])));
        assert_eq!(
            repeated,
            NormalizedEffectRow::Closed(vec!["FsRead".to_string()])
        );
    }
}
