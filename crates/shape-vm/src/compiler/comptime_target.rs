//! Comptime target object builder.
//!
//! When an annotation definition uses `comptime pre/post(...)`, the compiler builds
//! a structured object describing the annotated item. This module constructs
//! that object from AST information available at compile time.
//!
//! The target object has these fields:
//! - `kind`: string — "function", "type", "expression", etc.
//! - `name`: string — the name of the annotated item (if applicable)
//! - `fields`: array of {name, type, annotations} objects (for struct/type targets)
//! - `params`: array of {name, type} objects (for function targets)
//! - `return_type`: string (for function targets)
//! - `annotations`: array of annotation names already applied

use shape_ast::ast::functions::Annotation;
pub(crate) use shape_ast::ast::functions::AnnotationTargetKind;
use shape_ast::ast::literals::Literal;
use shape_ast::ast::{Expr, FunctionDef, TypeAnnotation};
use shape_runtime::type_schema::{register_predeclared_any_schema, typed_object_from_nb_pairs};
use shape_value::ValueWord;
use std::sync::Arc;

/// Check if a type string looks like `Option<T>` or `T?`.
fn is_option_type(type_str: &str) -> bool {
    type_str.starts_with("Option<") || type_str.ends_with('?')
}

/// Unwrap `Option<T>` -> `T` or `T?` -> `T` in a type string.
fn unwrap_option_type(type_str: &str) -> String {
    if type_str.starts_with("Option<") && type_str.ends_with('>') {
        type_str[7..type_str.len() - 1].to_string()
    } else if type_str.ends_with('?') {
        type_str[..type_str.len() - 1].to_string()
    } else {
        type_str.to_string()
    }
}

/// Per-field annotation: (annotation_name, Vec<stringified_args>).
pub(crate) type FieldAnnotation = (String, Vec<String>);

/// A compile-time target descriptor passed to comptime annotation handlers
/// in annotation definitions.
#[derive(Debug, Clone)]
pub(crate) struct ComptimeTarget {
    /// What kind of item is being annotated
    pub kind: AnnotationTargetKind,
    /// Name of the annotated item (empty string for expressions)
    pub name: String,
    /// Fields (for struct/type targets): Vec<(field_name, type_string, field_annotations)>
    pub fields: Vec<(String, String, Vec<FieldAnnotation>)>,
    /// Parameters (for function targets): Vec<(param_name, type_string, is_const)>
    pub params: Vec<(String, String, bool)>,
    /// Return type (for function targets)
    pub return_type: Option<String>,
    /// Annotations already applied to this target
    pub annotations: Vec<String>,
    /// Captured variables (for closures): variable names from outer scope
    pub captures: Vec<String>,
}

impl ComptimeTarget {
    /// Create a target descriptor for a function definition.
    pub fn from_function(func: &FunctionDef) -> Self {
        let params: Vec<(String, String, bool)> = func
            .params
            .iter()
            .map(|p| {
                let name = p.simple_name().unwrap_or("<destructured>").to_string();
                let type_str = p
                    .type_annotation
                    .as_ref()
                    .map(type_annotation_to_string)
                    .unwrap_or_else(|| "any".to_string());
                (name, type_str, p.is_const)
            })
            .collect();

        let return_type = func.return_type.as_ref().map(type_annotation_to_string);

        let annotations = func.annotations.iter().map(|a| a.name.clone()).collect();

        // Analyze captures: detect which outer-scope variables the function references.
        // For top-level functions this is empty; for closures it shows captured vars.
        let captures = shape_runtime::closure::EnvironmentAnalyzer::analyze_function(func, &[]);

        Self {
            kind: AnnotationTargetKind::Function,
            name: func.name.clone(),
            fields: Vec::new(),
            params,
            return_type,
            annotations,
            captures,
        }
    }

    /// Create a target descriptor for a named type with fields.
    ///
    /// Each field carries its type annotation and any annotations applied to it
    /// (e.g. `@description`, `@range`). The annotations are converted to
    /// `(name, stringified_args)` pairs so comptime handlers can inspect them.
    pub fn from_type(
        name: &str,
        fields: &[(String, Option<TypeAnnotation>, Vec<Annotation>)],
    ) -> Self {
        let fields = fields
            .iter()
            .map(|(fname, ftype, anns)| {
                let type_str = ftype
                    .as_ref()
                    .map(type_annotation_to_string)
                    .unwrap_or_else(|| "any".to_string());
                let field_anns: Vec<FieldAnnotation> = anns
                    .iter()
                    .map(|a| {
                        let args: Vec<String> = a.args.iter().map(expr_to_string_lossy).collect();
                        (a.name.clone(), args)
                    })
                    .collect();
                (fname.clone(), type_str, field_anns)
            })
            .collect();

        Self {
            kind: AnnotationTargetKind::Type,
            name: name.to_string(),
            fields,
            params: Vec::new(),
            return_type: None,
            annotations: Vec::new(),
            captures: Vec::new(),
        }
    }

    /// Create a target descriptor for a module definition.
    ///
    /// Module fields don't carry annotations, so they get empty annotation lists.
    pub fn from_module(name: &str, fields: &[(String, String)]) -> Self {
        let fields = fields
            .iter()
            .map(|(n, t)| (n.clone(), t.clone(), Vec::new()))
            .collect();
        Self {
            kind: AnnotationTargetKind::Module,
            name: name.to_string(),
            fields,
            params: Vec::new(),
            return_type: None,
            annotations: Vec::new(),
            captures: Vec::new(),
        }
    }

    /// Create a target descriptor for an expression.
    #[allow(dead_code)]
    pub fn for_expression() -> Self {
        Self {
            kind: AnnotationTargetKind::Expression,
            name: String::new(),
            fields: Vec::new(),
            params: Vec::new(),
            return_type: None,
            annotations: Vec::new(),
            captures: Vec::new(),
        }
    }

    /// Convert this target to a ValueWord TypedObject.
    ///
    /// This is the primary constructor — `to_vmvalue()` delegates here.
    pub fn to_nanboxed(&self) -> ValueWord {
        let nb_str = |s: &str| ValueWord::from_string(Arc::new(s.to_string()));
        let nb_string = |s: String| ValueWord::from_string(Arc::new(s));
        let ensure_schema = |names: &[&str]| {
            let field_names: Vec<String> = names.iter().map(|name| (*name).to_string()).collect();
            let _ = register_predeclared_any_schema(&field_names);
        };

        ensure_schema(&["name", "type"]);
        ensure_schema(&["name", "type", "annotations"]);
        ensure_schema(&["name", "type", "annotations", "optional"]);
        ensure_schema(&["name", "type", "const"]);
        ensure_schema(&["name", "args"]);
        ensure_schema(&[
            "kind",
            "name",
            "fields",
            "params",
            "return_type",
            "annotations",
            "captures",
        ]);

        // kind
        let kind_str = match self.kind {
            AnnotationTargetKind::Function => "function",
            AnnotationTargetKind::Type => "type",
            AnnotationTargetKind::Module => "module",
            AnnotationTargetKind::Expression => "expression",
            AnnotationTargetKind::Block => "block",
            AnnotationTargetKind::AwaitExpr => "await_expr",
            AnnotationTargetKind::Binding => "binding",
        };

        // fields: array of {name, type, annotations, optional} TypedObjects
        let fields_arr: Vec<ValueWord> = self
            .fields
            .iter()
            .map(|(fname, ftype, fanns)| {
                // Each annotation becomes {name, args} where args is an array of strings
                let anns_arr: Vec<ValueWord> = fanns
                    .iter()
                    .map(|(aname, aargs)| {
                        let args_arr: Vec<ValueWord> =
                            aargs.iter().map(|a| nb_string(a.clone())).collect();
                        typed_object_from_nb_pairs(&[
                            ("name", nb_string(aname.clone())),
                            ("args", ValueWord::from_array(Arc::new(args_arr))),
                        ])
                    })
                    .collect();
                // Detect Option<T> types and expose an `optional` flag + unwrapped inner type
                let is_optional = is_option_type(ftype);
                let effective_type = if is_optional {
                    unwrap_option_type(ftype)
                } else {
                    ftype.clone()
                };
                typed_object_from_nb_pairs(&[
                    ("name", nb_string(fname.clone())),
                    ("type", nb_string(effective_type)),
                    ("annotations", ValueWord::from_array(Arc::new(anns_arr))),
                    ("optional", ValueWord::from_bool(is_optional)),
                ])
            })
            .collect();

        // params: array of {name, type} TypedObjects
        let params_arr: Vec<ValueWord> = self
            .params
            .iter()
            .map(|(pname, ptype, is_const)| {
                typed_object_from_nb_pairs(&[
                    ("name", nb_string(pname.clone())),
                    ("type", nb_string(ptype.clone())),
                    ("const", ValueWord::from_bool(*is_const)),
                ])
            })
            .collect();

        // return_type
        let ret = self
            .return_type
            .as_ref()
            .map(|r| nb_string(r.clone()))
            .unwrap_or_else(ValueWord::none);

        // annotations
        let ann_arr: Vec<ValueWord> = self
            .annotations
            .iter()
            .map(|a| nb_string(a.clone()))
            .collect();

        // captures: array of captured variable names
        let captures_arr: Vec<ValueWord> =
            self.captures.iter().map(|c| nb_string(c.clone())).collect();

        typed_object_from_nb_pairs(&[
            ("kind", nb_str(kind_str)),
            ("name", nb_string(self.name.clone())),
            ("fields", ValueWord::from_array(Arc::new(fields_arr))),
            ("params", ValueWord::from_array(Arc::new(params_arr))),
            ("return_type", ret),
            ("annotations", ValueWord::from_array(Arc::new(ann_arr))),
            ("captures", ValueWord::from_array(Arc::new(captures_arr))),
        ])
    }
}

/// Best-effort stringification of an annotation argument expression.
///
/// Annotation args are typically literals (string, number, bool). For anything
/// more complex we fall back to the Debug representation.
fn expr_to_string_lossy(expr: &Expr) -> String {
    match expr {
        // For string literals, return the raw string (no quotes)
        Expr::Literal(Literal::String(s), _) => s.clone(),
        // All other literals have a Display impl
        Expr::Literal(lit, _) => lit.to_string(),
        Expr::Identifier(name, _) => name.clone(),
        _ => format!("{expr:?}"),
    }
}

/// Convert a TypeAnnotation to a human-readable string.
fn type_annotation_to_string(ta: &TypeAnnotation) -> String {
    match ta {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(name) => name.to_string(),
        TypeAnnotation::Array(inner) => format!("[{}]", type_annotation_to_string(inner)),
        TypeAnnotation::Union(types) => types
            .iter()
            .map(type_annotation_to_string)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeAnnotation::Intersection(types) => types
            .iter()
            .map(type_annotation_to_string)
            .collect::<Vec<_>>()
            .join(" & "),
        TypeAnnotation::Function { params, returns } => {
            let params_str = params
                .iter()
                .map(|p| type_annotation_to_string(&p.type_annotation))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({}) => {}", params_str, type_annotation_to_string(returns))
        }
        TypeAnnotation::Generic { name, args } => {
            let args_str = args
                .iter()
                .map(type_annotation_to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}<{}>", name, args_str)
        }
        TypeAnnotation::Object(fields) => {
            let fields_str = fields
                .iter()
                .map(|f| {
                    format!(
                        "{}: {}",
                        f.name,
                        type_annotation_to_string(&f.type_annotation)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{}}}", fields_str)
        }
        TypeAnnotation::Tuple(types) => {
            let types_str = types
                .iter()
                .map(type_annotation_to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({})", types_str)
        }
        TypeAnnotation::Void => "void".to_string(),
        TypeAnnotation::Never => "never".to_string(),
        TypeAnnotation::Null => "null".to_string(),
        TypeAnnotation::Undefined => "undefined".to_string(),
        TypeAnnotation::Dyn(traits) => format!("dyn {}", traits.join(" + ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::{DestructurePattern, FunctionDef, FunctionParameter, Span};

    #[test]
    fn test_target_from_function() {
        let func = FunctionDef {
            name: "greet".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            params: vec![FunctionParameter {
                pattern: DestructurePattern::Identifier("name".to_string(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
                is_out: false,
                type_annotation: Some(TypeAnnotation::Basic("string".to_string())),
                default_value: None,
            }],
            return_type: Some(TypeAnnotation::Basic("string".to_string())),
            body: Vec::new(),
            type_params: None,
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
            where_clause: None,
        };

        let target = ComptimeTarget::from_function(&func);
        assert_eq!(target.kind, AnnotationTargetKind::Function);
        assert_eq!(target.name, "greet");
        assert_eq!(target.params.len(), 1);
        assert_eq!(target.params[0].0, "name");
        assert_eq!(target.params[0].1, "string");
        assert!(!target.params[0].2);
        assert_eq!(target.return_type, Some("string".to_string()));
    }

    #[test]
    fn test_target_from_type() {
        let fields = vec![
            (
                "x".to_string(),
                Some(TypeAnnotation::Basic("number".to_string())),
                Vec::new(),
            ),
            (
                "y".to_string(),
                Some(TypeAnnotation::Basic("number".to_string())),
                Vec::new(),
            ),
        ];

        let target = ComptimeTarget::from_type("Point", &fields);
        assert_eq!(target.kind, AnnotationTargetKind::Type);
        assert_eq!(target.name, "Point");
        assert_eq!(target.fields.len(), 2);
        assert_eq!(target.fields[0].0, "x");
        assert_eq!(target.fields[0].1, "number");
        assert!(target.fields[0].2.is_empty());
    }

    #[test]
    fn test_target_from_type_with_field_annotations() {
        use shape_ast::ast::Expr;
        use shape_ast::ast::literals::Literal;

        let fields = vec![
            (
                "label".to_string(),
                Some(TypeAnnotation::Basic("string".to_string())),
                vec![Annotation {
                    name: "description".to_string(),
                    args: vec![Expr::Literal(
                        Literal::String("A label".to_string()),
                        Span::DUMMY,
                    )],
                    span: Span::DUMMY,
                }],
            ),
            (
                "confidence".to_string(),
                Some(TypeAnnotation::Basic("number".to_string())),
                vec![
                    Annotation {
                        name: "description".to_string(),
                        args: vec![Expr::Literal(
                            Literal::String("0.0 to 1.0".to_string()),
                            Span::DUMMY,
                        )],
                        span: Span::DUMMY,
                    },
                    Annotation {
                        name: "range".to_string(),
                        args: vec![
                            Expr::Literal(Literal::Number(0.0), Span::DUMMY),
                            Expr::Literal(Literal::Number(1.0), Span::DUMMY),
                        ],
                        span: Span::DUMMY,
                    },
                ],
            ),
        ];

        let target = ComptimeTarget::from_type("Sentiment", &fields);
        assert_eq!(target.name, "Sentiment");
        assert_eq!(target.fields.len(), 2);

        // First field: one annotation
        assert_eq!(target.fields[0].2.len(), 1);
        assert_eq!(target.fields[0].2[0].0, "description");
        assert_eq!(target.fields[0].2[0].1, vec!["A label"]);

        // Second field: two annotations
        assert_eq!(target.fields[1].2.len(), 2);
        assert_eq!(target.fields[1].2[0].0, "description");
        assert_eq!(target.fields[1].2[1].0, "range");
        assert_eq!(target.fields[1].2[1].1, vec!["0", "1"]);
    }

    #[test]
    fn test_target_to_vmvalue() {
        let target = ComptimeTarget {
            kind: AnnotationTargetKind::Function,
            name: "test_fn".to_string(),
            fields: Vec::new(),
            params: vec![("x".to_string(), "number".to_string(), false)],
            return_type: Some("bool".to_string()),
            annotations: vec!["cached".to_string()],
            captures: vec!["outer_var".to_string()],
        };

        let value = target.to_nanboxed();
        assert_eq!(value.type_name(), "object");
    }

    #[test]
    fn test_target_to_vmvalue_with_field_annotations() {
        let target = ComptimeTarget {
            kind: AnnotationTargetKind::Type,
            name: "Sentiment".to_string(),
            fields: vec![
                (
                    "label".to_string(),
                    "string".to_string(),
                    vec![("description".to_string(), vec!["A label".to_string()])],
                ),
                (
                    "score".to_string(),
                    "number".to_string(),
                    vec![("range".to_string(), vec!["0".to_string(), "1".to_string()])],
                ),
            ],
            params: Vec::new(),
            return_type: None,
            annotations: Vec::new(),
            captures: Vec::new(),
        };

        let value = target.to_nanboxed();
        assert_eq!(value.type_name(), "object");
    }

    #[test]
    fn test_target_for_expression() {
        let target = ComptimeTarget::for_expression();
        assert_eq!(target.kind, AnnotationTargetKind::Expression);
        assert_eq!(target.name, "");
        assert!(target.fields.is_empty());
        assert!(target.params.is_empty());
    }

    #[test]
    fn test_type_annotation_to_string_variants() {
        assert_eq!(
            type_annotation_to_string(&TypeAnnotation::Basic("number".to_string())),
            "number"
        );
        assert_eq!(
            type_annotation_to_string(&TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
                "string".to_string()
            )))),
            "[string]"
        );
        assert_eq!(
            type_annotation_to_string(&TypeAnnotation::Generic {
                name: "Option".into(),
                args: vec![TypeAnnotation::Basic("number".to_string())],
            }),
            "Option<number>"
        );
        assert_eq!(
            type_annotation_to_string(&TypeAnnotation::Union(vec![
                TypeAnnotation::Basic("string".to_string()),
                TypeAnnotation::Basic("number".to_string()),
            ])),
            "string | number"
        );
        assert_eq!(type_annotation_to_string(&TypeAnnotation::Void), "void");
        assert_eq!(type_annotation_to_string(&TypeAnnotation::Never), "never");
    }

    #[test]
    fn test_target_captures_empty_for_toplevel() {
        // A top-level function with no outer references has empty captures
        let func = FunctionDef {
            name: "add".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            params: vec![
                FunctionParameter {
                    pattern: DestructurePattern::Identifier("a".to_string(), Span::DUMMY),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    type_annotation: None,
                    default_value: None,
                },
                FunctionParameter {
                    pattern: DestructurePattern::Identifier("b".to_string(), Span::DUMMY),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    type_annotation: None,
                    default_value: None,
                },
            ],
            return_type: None,
            body: vec![shape_ast::ast::Statement::Return(
                Some(shape_ast::ast::Expr::BinaryOp {
                    left: Box::new(shape_ast::ast::Expr::Identifier(
                        "a".to_string(),
                        Span::DUMMY,
                    )),
                    op: shape_ast::ast::BinaryOp::Add,
                    right: Box::new(shape_ast::ast::Expr::Identifier(
                        "b".to_string(),
                        Span::DUMMY,
                    )),
                    span: Span::DUMMY,
                }),
                Span::DUMMY,
            )],
            type_params: None,
            annotations: Vec::new(),
            is_async: false,
            is_comptime: false,
            where_clause: None,
        };

        let target = ComptimeTarget::from_function(&func);
        assert!(
            target.captures.is_empty(),
            "Top-level function should have no captures"
        );
    }

    #[test]
    fn test_target_captures_vmvalue_included() {
        // Verify captures field appears in ValueWord output
        let target = ComptimeTarget {
            kind: AnnotationTargetKind::Function,
            name: "closure".to_string(),
            fields: Vec::new(),
            params: Vec::new(),
            return_type: None,
            annotations: Vec::new(),
            captures: vec!["x".to_string(), "y".to_string()],
        };

        let value = target.to_nanboxed();
        // Now returns TypedObject
        assert_eq!(value.type_name(), "object");
    }

    #[test]
    fn test_target_from_type_with_option_fields() {
        // Fields with Option<T> type should have `optional: true` and unwrapped inner type.
        let fields = vec![
            (
                "name".to_string(),
                Some(TypeAnnotation::Basic("string".to_string())),
                Vec::new(),
            ),
            (
                "nickname".to_string(),
                Some(TypeAnnotation::option(TypeAnnotation::Basic(
                    "string".to_string(),
                ))),
                Vec::new(),
            ),
            (
                "age".to_string(),
                Some(TypeAnnotation::option(TypeAnnotation::Basic(
                    "number".to_string(),
                ))),
                Vec::new(),
            ),
        ];

        let target = ComptimeTarget::from_type("Person", &fields);
        assert_eq!(target.name, "Person");
        assert_eq!(target.fields.len(), 3);

        // First field: "name" with type "string" — NOT optional
        assert_eq!(target.fields[0].0, "name");
        assert_eq!(target.fields[0].1, "string");

        // Second field: "nickname" with type "Option<string>" — IS optional
        assert_eq!(target.fields[1].0, "nickname");
        assert_eq!(target.fields[1].1, "Option<string>");

        // Third field: "age" with type "Option<number>" — IS optional
        assert_eq!(target.fields[2].0, "age");
        assert_eq!(target.fields[2].1, "Option<number>");

        // Verify the nanboxed representation includes optional flags
        let value = target.to_nanboxed();
        assert_eq!(value.type_name(), "object");

        // Extract fields array from the target TypedObject
        if let Some(fields_map) = shape_runtime::type_schema::typed_object_to_hashmap_nb(&value) {
            let fields_arr = fields_map.get("fields").expect("should have fields");
            if let Some(view) = fields_arr.as_any_array() {
                let arr = view.to_generic();
                assert_eq!(arr.len(), 3);

                // Check first field is NOT optional
                if let Some(f0) = shape_runtime::type_schema::typed_object_to_hashmap_nb(&arr[0]) {
                    let opt = f0.get("optional").expect("should have optional field");
                    assert_eq!(
                        opt.as_bool(),
                        Some(false),
                        "non-option field should be optional=false"
                    );
                    let type_str = f0.get("type").expect("should have type");
                    assert_eq!(
                        type_str.as_str(),
                        Some("string"),
                        "non-option field type should be 'string'"
                    );
                }

                // Check second field IS optional with unwrapped type
                if let Some(f1) = shape_runtime::type_schema::typed_object_to_hashmap_nb(&arr[1]) {
                    let opt = f1.get("optional").expect("should have optional field");
                    assert_eq!(
                        opt.as_bool(),
                        Some(true),
                        "Option<string> field should be optional=true"
                    );
                    let type_str = f1.get("type").expect("should have type");
                    assert_eq!(
                        type_str.as_str(),
                        Some("string"),
                        "Option<string> field type should be unwrapped to 'string'"
                    );
                }

                // Check third field IS optional with unwrapped type
                if let Some(f2) = shape_runtime::type_schema::typed_object_to_hashmap_nb(&arr[2]) {
                    let opt = f2.get("optional").expect("should have optional field");
                    assert_eq!(
                        opt.as_bool(),
                        Some(true),
                        "Option<number> field should be optional=true"
                    );
                    let type_str = f2.get("type").expect("should have type");
                    assert_eq!(
                        type_str.as_str(),
                        Some("number"),
                        "Option<number> field type should be unwrapped to 'number'"
                    );
                }
            }
        }
    }

    #[test]
    fn test_is_option_type_detection() {
        assert!(is_option_type("Option<string>"));
        assert!(is_option_type("Option<number>"));
        assert!(is_option_type("Option<Array<int>>"));
        assert!(!is_option_type("string"));
        assert!(!is_option_type("number"));
        assert!(!is_option_type("Array<Option<int>>"));
    }

    #[test]
    fn test_unwrap_option_type() {
        assert_eq!(unwrap_option_type("Option<string>"), "string");
        assert_eq!(unwrap_option_type("Option<number>"), "number");
        assert_eq!(unwrap_option_type("string"), "string");
        assert_eq!(unwrap_option_type("number"), "number");
    }
}
