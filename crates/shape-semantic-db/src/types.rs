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
            NormalizedType::Function { params, result } => format!(
                "({}) => {}",
                params
                    .iter()
                    .map(NormalizedType::render)
                    .collect::<Vec<_>>()
                    .join(", "),
                result.render()
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
        TypeAnnotation::Tuple(items) => NormalizedType::Tuple(items.iter().map(normalize).collect()),
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
        TypeAnnotation::Function { params, returns } => NormalizedType::Function {
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
            NormalizedType::Function { params, result } => {
                writer.u8(16);
                writer.seq(params);
                writer.nested(result.as_ref());
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
        let array_of_int = NormalizedType::Array(Box::new(NormalizedType::Int))
            .canonical_digest("test");
        assert_ne!(int, number);
        assert_ne!(int, array_of_int);
    }
}
