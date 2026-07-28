//! Deterministic human-readable spelling issued by the semantic freeze.

use std::collections::{BTreeMap, HashSet};

use shape_ast::ast::TypeAnnotation;

use super::super::{FreezeOverlay, FrozenTypeIdentity, canonicalize_type_annotation};

pub(super) fn canonical_type_presentation(
    annotation: &TypeAnnotation,
    overlay: &FreezeOverlay,
) -> Result<String, String> {
    render(annotation, overlay, &BTreeMap::new())
}

fn render(
    annotation: &TypeAnnotation,
    overlay: &FreezeOverlay,
    witnesses: &BTreeMap<String, String>,
) -> Result<String, String> {
    if let Some(name) = witness_name(annotation, witnesses) {
        return Ok(name);
    }
    let canonical = canonicalize_type_annotation(annotation, overlay)?;
    if let Some(name) = registered_name(canonical.identity, overlay) {
        return Ok(name);
    }

    match annotation {
        TypeAnnotation::Basic(_) | TypeAnnotation::Reference(_) => {
            Ok(format_identity(canonical.identity))
        }
        TypeAnnotation::Array(inner) => {
            Ok(format!("Array<{}>", render(inner, overlay, witnesses)?))
        }
        TypeAnnotation::Tuple(items) => Ok(format!(
            "[{}]",
            render_many(items, overlay, witnesses)?.join(", ")
        )),
        TypeAnnotation::Object(fields) => {
            let mut fields = fields.iter().collect::<Vec<_>>();
            fields.sort_by(|left, right| left.name.cmp(&right.name));
            let rendered = fields
                .into_iter()
                .map(|field| {
                    Ok(format!(
                        "{}{}: {}",
                        field.name,
                        if field.optional { "?" } else { "" },
                        render(&field.type_annotation, overlay, witnesses)?
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(format!("{{{}}}", rendered.join(", ")))
        }
        TypeAnnotation::Function {
            params, returns, ..
        } => {
            let params = params
                .iter()
                .map(|parameter| {
                    Ok(format!(
                        "{}{}",
                        render(&parameter.type_annotation, overlay, witnesses)?,
                        if parameter.optional { "?" } else { "" }
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(format!(
                "fn({}) -> {}",
                params.join(", "),
                render(returns, overlay, witnesses)?
            ))
        }
        TypeAnnotation::Union(items) => render_set(items, " | ", overlay, witnesses),
        TypeAnnotation::Intersection(items) => render_set(items, " & ", overlay, witnesses),
        TypeAnnotation::Generic { name, args } => {
            let head = render(&TypeAnnotation::Reference(name.clone()), overlay, witnesses)?;
            Ok(format!(
                "{head}<{}>",
                render_many(args, overlay, witnesses)?.join(", ")
            ))
        }
        TypeAnnotation::Borrow { mutable, inner } => Ok(format!(
            "&{}{}",
            if *mutable { "mut " } else { "" },
            render(inner, overlay, witnesses)?
        )),
        TypeAnnotation::Void => Ok("void".to_string()),
        TypeAnnotation::Never => Ok("never".to_string()),
        TypeAnnotation::Null => Ok("null".to_string()),
        TypeAnnotation::Undefined => Ok("undefined".to_string()),
        TypeAnnotation::Dyn(bounds) => {
            let mut bounds = bounds.iter().map(ToString::to_string).collect::<Vec<_>>();
            bounds.sort();
            bounds.dedup();
            Ok(format!("dyn {}", bounds.join(" + ")))
        }
        TypeAnnotation::Existential {
            witnesses: declared,
            inner,
        } => {
            let mut scoped = witnesses.clone();
            let mut normalized = Vec::with_capacity(declared.len());
            for (index, witness) in declared.iter().enumerate() {
                let name = format!("${index}");
                scoped.insert(witness.clone(), name.clone());
                normalized.push(name);
            }
            Ok(format!(
                "exists<{}> {}",
                normalized.join(", "),
                render(inner, overlay, &scoped)?
            ))
        }
    }
}

fn render_many(
    annotations: &[TypeAnnotation],
    overlay: &FreezeOverlay,
    witnesses: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    annotations
        .iter()
        .map(|annotation| render(annotation, overlay, witnesses))
        .collect()
}

fn render_set(
    annotations: &[TypeAnnotation],
    separator: &str,
    overlay: &FreezeOverlay,
    witnesses: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut members = Vec::new();
    collect_set_members(annotations, separator, &mut members);
    let mut seen = HashSet::new();
    let mut rendered = Vec::new();
    for member in members {
        let canonical = canonicalize_type_annotation(member, overlay)?;
        if seen.insert((canonical.identity.high, canonical.identity.low)) {
            rendered.push((
                canonical.identity.high,
                canonical.identity.low,
                render(member, overlay, witnesses)?,
            ));
        }
    }
    rendered.sort_by_key(|(high, low, _)| (*high, *low));
    Ok(rendered
        .into_iter()
        .map(|(_, _, presentation)| presentation)
        .collect::<Vec<_>>()
        .join(separator))
}

fn collect_set_members<'a>(
    annotations: &'a [TypeAnnotation],
    separator: &str,
    out: &mut Vec<&'a TypeAnnotation>,
) {
    for annotation in annotations {
        match (separator, annotation) {
            (" | ", TypeAnnotation::Union(nested))
            | (" & ", TypeAnnotation::Intersection(nested)) => {
                collect_set_members(nested, separator, out);
            }
            _ => out.push(annotation),
        }
    }
}

fn witness_name(
    annotation: &TypeAnnotation,
    witnesses: &BTreeMap<String, String>,
) -> Option<String> {
    let source = match annotation {
        TypeAnnotation::Basic(name) => name.as_str(),
        TypeAnnotation::Reference(path) => path.as_str(),
        _ => return None,
    };
    witnesses.get(source).cloned()
}

fn registered_name(identity: FrozenTypeIdentity, overlay: &FreezeOverlay) -> Option<String> {
    let mut names = overlay
        .type_names_for_identity(identity)
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    names.extend(
        overlay
            .lexical_parameters
            .names_for_identity(identity)
            .into_iter()
            .map(str::to_string),
    );
    names.extend(
        overlay
            .witnesses
            .iter()
            .filter(|(_, frozen)| **frozen == identity)
            .map(|(name, _)| name.clone()),
    );
    names.sort_by_key(|name| canonical_name_rank(name));
    names.dedup();
    names.into_iter().next()
}

fn canonical_name_rank(name: &str) -> (usize, usize, String) {
    const PREFERRED: &[&str] = &[
        "void",
        "never",
        "null",
        "undefined",
        "bool",
        "int",
        "number",
        "decimal",
        "string",
        "char",
        "any",
    ];
    let preferred = PREFERRED
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap_or(PREFERRED.len());
    (preferred, name.len(), name.to_string())
}

pub(super) fn format_identity(identity: FrozenTypeIdentity) -> String {
    format!(
        "type#{:016x}{:016x}",
        identity.high as u64, identity.low as u64
    )
}
