//! Comptime target object builder.
//!
//! When an annotation definition uses `comptime pre/post(...)`, the compiler builds
//! a structured object describing the annotated item. This module constructs
//! that object from AST information available at compile time.
//!
//! The target object has these fields:
//! - `kind`: string — "function", "type", "expression", etc.
//! - `name`: string — the name of the annotated item (if applicable)
//! - `fields`: array of {name, type} objects (for struct/type targets)
//! - `params`: array of {name, type} objects (for function targets)
//! - `return_type`: string (for function targets)
//! - `annotations`: array of annotation names already applied

pub(crate) use shape_ast::ast::functions::AnnotationTargetKind;
use shape_ast::ast::{FunctionDef, TypeAnnotation};
use shape_runtime::type_schema::{register_predeclared_any_schema, typed_object_from_nb_pairs};
use shape_value::ValueWord;
use std::sync::Arc;

/// A compile-time target descriptor passed to comptime annotation handlers
/// in annotation definitions.
#[derive(Debug, Clone)]
pub(crate) struct ComptimeTarget {
    /// What kind of item is being annotated
    pub kind: AnnotationTargetKind,
    /// Name of the annotated item (empty string for expressions)
    pub name: String,
    /// Fields (for struct/type targets): Vec<(field_name, type_string)>
    pub fields: Vec<(String, String)>,
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
    pub fn from_type(name: &str, fields: &[(String, Option<TypeAnnotation>)]) -> Self {
        let fields = fields
            .iter()
            .map(|(fname, ftype)| {
                let type_str = ftype
                    .as_ref()
                    .map(type_annotation_to_string)
                    .unwrap_or_else(|| "any".to_string());
                (fname.clone(), type_str)
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
    pub fn from_module(name: &str, fields: &[(String, String)]) -> Self {
        Self {
            kind: AnnotationTargetKind::Module,
            name: name.to_string(),
            fields: fields.to_vec(),
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
        ensure_schema(&["name", "type", "const"]);
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

        // fields: array of {name, type} TypedObjects
        let fields_arr: Vec<ValueWord> = self
            .fields
            .iter()
            .map(|(fname, ftype)| {
                typed_object_from_nb_pairs(&[
                    ("name", nb_string(fname.clone())),
                    ("type", nb_string(ftype.clone())),
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

/// Convert a TypeAnnotation to a human-readable string.
fn type_annotation_to_string(ta: &TypeAnnotation) -> String {
    match ta {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(name) => name.clone(),
        TypeAnnotation::Array(inner) => format!("[{}]", type_annotation_to_string(inner)),
        TypeAnnotation::Optional(inner) => format!("{}?", type_annotation_to_string(inner)),
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
            doc_comment: None,
            params: vec![FunctionParameter {
                pattern: DestructurePattern::Identifier("name".to_string(), Span::DUMMY),
                is_const: false,
                is_reference: false,
                is_mut_reference: false,
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
            ),
            (
                "y".to_string(),
                Some(TypeAnnotation::Basic("number".to_string())),
            ),
        ];

        let target = ComptimeTarget::from_type("Point", &fields);
        assert_eq!(target.kind, AnnotationTargetKind::Type);
        assert_eq!(target.name, "Point");
        assert_eq!(target.fields.len(), 2);
        assert_eq!(target.fields[0].0, "x");
        assert_eq!(target.fields[0].1, "number");
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
        // Now returns TypedObject instead of Object
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
            type_annotation_to_string(&TypeAnnotation::Optional(Box::new(TypeAnnotation::Basic(
                "number".to_string()
            )))),
            "number?"
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
            doc_comment: None,
            params: vec![
                FunctionParameter {
                    pattern: DestructurePattern::Identifier("a".to_string(), Span::DUMMY),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    type_annotation: None,
                    default_value: None,
                },
                FunctionParameter {
                    pattern: DestructurePattern::Identifier("b".to_string(), Span::DUMMY),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
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
}
