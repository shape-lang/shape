//! Type Annotation Conversions
//!
//! Handles conversions between TypeAnnotation (AST) and SemanticType.

use super::builtins::BuiltinTypes;
use crate::type_system::semantic::{EnumVariant, FunctionParam, FunctionSignature, SemanticType};
use shape_ast::ast::TypeAnnotation;

/// Convert a type annotation to canonical source-like text.
pub fn annotation_to_string(ann: &TypeAnnotation) -> String {
    match ann {
        TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => name.clone(),
        TypeAnnotation::Generic { name, args } => {
            if args.is_empty() {
                name.clone()
            } else {
                let rendered: Vec<String> = args.iter().map(annotation_to_string).collect();
                format!("{}<{}>", name, rendered.join(", "))
            }
        }
        TypeAnnotation::Array(inner) => format!("Vec<{}>", annotation_to_string(inner)),
        TypeAnnotation::Tuple(types) => {
            let rendered: Vec<String> = types.iter().map(annotation_to_string).collect();
            format!("[{}]", rendered.join(", "))
        }
        TypeAnnotation::Object(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|f| {
                    let optional = if f.optional { "?" } else { "" };
                    format!(
                        "{}{}: {}",
                        f.name,
                        optional,
                        annotation_to_string(&f.type_annotation)
                    )
                })
                .collect();
            format!("{{ {} }}", rendered.join(", "))
        }
        TypeAnnotation::Function { params, returns } => {
            let rendered_params: Vec<String> = params
                .iter()
                .map(|p| annotation_to_string(&p.type_annotation))
                .collect();
            format!(
                "({}) -> {}",
                rendered_params.join(", "),
                annotation_to_string(returns)
            )
        }
        TypeAnnotation::Union(types) => types
            .iter()
            .map(annotation_to_string)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeAnnotation::Intersection(types) => types
            .iter()
            .map(annotation_to_string)
            .collect::<Vec<_>>()
            .join(" + "),
        TypeAnnotation::Void => "()".to_string(),
        TypeAnnotation::Never => "never".to_string(),
        TypeAnnotation::Null => "None".to_string(),
        TypeAnnotation::Undefined => "undefined".to_string(),
        TypeAnnotation::Dyn(traits) => format!("dyn {}", traits.join(" + ")),
    }
}

/// Convert TypeAnnotation to SemanticType
pub fn annotation_to_semantic(ann: &TypeAnnotation) -> SemanticType {
    fn scalar_name_to_semantic(name: &str) -> SemanticType {
        if matches!(name, "int" | "Int" | "integer" | "Integer") {
            return SemanticType::Integer;
        }
        if matches!(name, "number" | "Number" | "float" | "Float") {
            return SemanticType::Number;
        }
        if BuiltinTypes::is_number_type_name(name) || BuiltinTypes::is_integer_type_name(name) {
            return SemanticType::Named(
                BuiltinTypes::canonical_numeric_runtime_name(name)
                    .unwrap_or(name)
                    .to_string(),
            );
        }
        if BuiltinTypes::is_bool_type_name(name) {
            return SemanticType::Bool;
        }
        if BuiltinTypes::is_string_type_name(name) {
            return SemanticType::String;
        }
        SemanticType::Named(name.to_string())
    }

    match ann {
        TypeAnnotation::Basic(name) => scalar_name_to_semantic(name),
        TypeAnnotation::Array(elem) => SemanticType::Array(Box::new(annotation_to_semantic(elem))),
        TypeAnnotation::Generic { name, args } => {
            let semantic_args: Vec<_> = args.iter().map(annotation_to_semantic).collect();
            match name.as_str() {
                "Option" if semantic_args.len() == 1 => {
                    SemanticType::Option(Box::new(semantic_args[0].clone()))
                }
                "Result" if !semantic_args.is_empty() => SemanticType::Result {
                    ok_type: Box::new(semantic_args[0].clone()),
                    err_type: semantic_args.get(1).cloned().map(Box::new),
                },
                "Vec" if semantic_args.len() == 1 => {
                    SemanticType::Array(Box::new(semantic_args[0].clone()))
                }
                "Table" if semantic_args.len() == 1 => SemanticType::Generic {
                    name: "Table".to_string(),
                    args: semantic_args,
                },
                "Table" if semantic_args.len() == 2 => SemanticType::Generic {
                    name: "Table".to_string(),
                    args: semantic_args,
                },
                _ => SemanticType::Generic {
                    name: name.clone(),
                    args: semantic_args,
                },
            }
        }
        TypeAnnotation::Reference(name) => scalar_name_to_semantic(name),
        TypeAnnotation::Void => SemanticType::Void,
        TypeAnnotation::Never => SemanticType::Never,
        TypeAnnotation::Function { params, returns } => {
            let param_types: Vec<_> = params
                .iter()
                .map(|p| FunctionParam {
                    name: p.name.clone(),
                    param_type: annotation_to_semantic(&p.type_annotation),
                    optional: p.optional,
                })
                .collect();
            SemanticType::Function(Box::new(FunctionSignature {
                params: param_types,
                return_type: annotation_to_semantic(returns),
                is_fallible: false,
            }))
        }
        TypeAnnotation::Tuple(elems) => {
            // Represent tuple as struct with numeric field names
            let fields: Vec<_> = elems
                .iter()
                .enumerate()
                .map(|(i, e)| (format!("_{}", i), annotation_to_semantic(e)))
                .collect();
            SemanticType::Struct {
                name: "Tuple".to_string(),
                fields,
            }
        }
        TypeAnnotation::Object(fields) => {
            let semantic_fields: Vec<_> = fields
                .iter()
                .map(|f| (f.name.clone(), annotation_to_semantic(&f.type_annotation)))
                .collect();
            SemanticType::Struct {
                name: "Object".to_string(),
                fields: semantic_fields,
            }
        }
        TypeAnnotation::Union(types) => {
            // Convert union to enum-like structure
            let variants: Vec<_> = types
                .iter()
                .enumerate()
                .map(|(i, t)| EnumVariant {
                    name: format!("Variant{}", i),
                    payload: Some(annotation_to_semantic(t)),
                })
                .collect();
            SemanticType::Enum {
                name: "Union".to_string(),
                variants,
                type_params: vec![],
            }
        }
        TypeAnnotation::Intersection(_) => {
            // Intersection types are resolved during type checking
            SemanticType::Void
        }
        TypeAnnotation::Null | TypeAnnotation::Undefined => SemanticType::Void,
        TypeAnnotation::Dyn(traits) => SemanticType::Named(format!("dyn {}", traits.join(" + "))),
    }
}

/// Convert SemanticType to TypeAnnotation
pub fn semantic_to_annotation(ty: &SemanticType) -> TypeAnnotation {
    match ty {
        SemanticType::Number => TypeAnnotation::Basic("number".to_string()),
        SemanticType::Integer => TypeAnnotation::Basic("int".to_string()),
        SemanticType::Bool => TypeAnnotation::Basic("bool".to_string()),
        SemanticType::String => TypeAnnotation::Basic("string".to_string()),
        SemanticType::Option(inner) => TypeAnnotation::Generic {
            name: "Option".to_string(),
            args: vec![semantic_to_annotation(inner)],
        },
        SemanticType::Result { ok_type, err_type } => {
            let mut args = vec![semantic_to_annotation(ok_type)];
            if let Some(err) = err_type {
                args.push(semantic_to_annotation(err));
            }
            TypeAnnotation::Generic {
                name: "Result".to_string(),
                args,
            }
        }
        SemanticType::Array(elem) => TypeAnnotation::Array(Box::new(semantic_to_annotation(elem))),
        SemanticType::Generic { name, args } => TypeAnnotation::Generic {
            name: name.clone(),
            args: args.iter().map(semantic_to_annotation).collect(),
        },
        SemanticType::Named(name) => {
            if BuiltinTypes::is_number_type_name(name)
                || BuiltinTypes::is_integer_type_name(name)
                || BuiltinTypes::is_bool_type_name(name)
                || BuiltinTypes::is_string_type_name(name)
            {
                TypeAnnotation::Basic(name.clone())
            } else {
                TypeAnnotation::Reference(name.clone())
            }
        }
        SemanticType::TypeVar(id) => TypeAnnotation::Reference(format!("T{}", id.0)),
        SemanticType::Void => TypeAnnotation::Void,
        SemanticType::Never => TypeAnnotation::Never,
        SemanticType::Function(sig) => {
            let params: Vec<_> = sig
                .params
                .iter()
                .map(|p| shape_ast::ast::FunctionParam {
                    name: p.name.clone(),
                    optional: p.optional,
                    type_annotation: semantic_to_annotation(&p.param_type),
                })
                .collect();
            TypeAnnotation::Function {
                params,
                returns: Box::new(semantic_to_annotation(&sig.return_type)),
            }
        }
        SemanticType::Struct { name, fields } => {
            if name == "Object" || name == "Tuple" {
                TypeAnnotation::Object(
                    fields
                        .iter()
                        .map(|(n, t)| shape_ast::ast::ObjectTypeField {
                            name: n.clone(),
                            optional: false,
                            type_annotation: semantic_to_annotation(t),
                            annotations: vec![],
                        })
                        .collect(),
                )
            } else {
                TypeAnnotation::Reference(name.clone())
            }
        }
        SemanticType::Enum { name, .. } | SemanticType::Interface { name, .. } => {
            TypeAnnotation::Reference(name.clone())
        }
        SemanticType::Ref(inner) => {
            // Map &T to the annotation for T — references don't have a distinct
            // TypeAnnotation variant yet; the compiler tracks ref-ness separately.
            semantic_to_annotation(inner)
        }
        SemanticType::RefMut(inner) => semantic_to_annotation(inner),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_one_arg() {
        // Table<Number> -> SemanticType::Generic
        let ann = TypeAnnotation::Generic {
            name: "Table".to_string(),
            args: vec![TypeAnnotation::Basic("Number".to_string())],
        };
        let semantic = annotation_to_semantic(&ann);

        match semantic {
            SemanticType::Generic { name, args } => {
                assert_eq!(name, "Table");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], SemanticType::Number);
            }
            _ => panic!("Expected Generic Table, got {:?}", semantic),
        }
    }

    #[test]
    fn test_table_annotation_maps_to_table() {
        let ann = TypeAnnotation::Generic {
            name: "Table".to_string(),
            args: vec![TypeAnnotation::Basic("Number".to_string())],
        };
        let semantic = annotation_to_semantic(&ann);

        match semantic {
            SemanticType::Generic { name, .. } => {
                assert_eq!(name, "Table");
            }
            _ => panic!("Expected Generic Table, got {:?}", semantic),
        }
    }

    #[test]
    fn test_width_aware_int_aliases_map_to_integer_semantic() {
        let int_ann = TypeAnnotation::Basic("int".to_string());
        let number_ann = TypeAnnotation::Basic("number".to_string());
        let byte_ann = TypeAnnotation::Basic("byte".to_string());
        let char_ann = TypeAnnotation::Basic("char".to_string());
        let i16_ann = TypeAnnotation::Basic("i16".to_string());
        let u64_ann = TypeAnnotation::Basic("u64".to_string());
        let f32_ann = TypeAnnotation::Basic("f32".to_string());
        let f64_ann = TypeAnnotation::Basic("f64".to_string());

        assert_eq!(annotation_to_semantic(&int_ann), SemanticType::Integer);
        assert_eq!(annotation_to_semantic(&number_ann), SemanticType::Number);

        assert_eq!(
            annotation_to_semantic(&byte_ann),
            SemanticType::Named("u8".to_string())
        );
        assert_eq!(
            annotation_to_semantic(&char_ann),
            SemanticType::Named("i8".to_string())
        );
        assert_eq!(
            annotation_to_semantic(&i16_ann),
            SemanticType::Named("i16".to_string())
        );
        assert_eq!(
            annotation_to_semantic(&u64_ann),
            SemanticType::Named("u64".to_string())
        );
        assert_eq!(
            annotation_to_semantic(&f32_ann),
            SemanticType::Named("f32".to_string())
        );
        assert_eq!(
            annotation_to_semantic(&f64_ann),
            SemanticType::Named("f64".to_string())
        );
    }
}
