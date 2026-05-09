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
use shape_value::KindedSlot;

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

    /// Convert this target to a `KindedSlot` TypedObject describing the
    /// annotated item.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.4.** The previous body
    /// constructed a `HeapValue::TypedObject { schema_id, slots, heap_mask }`
    /// by stamping per-field `ValueWord` constructors (`from_string`,
    /// `from_array`, `from_bool`) into the deleted `ArgVec` /
    /// `vmarray_from_vec` builders. After the strict-typing bulldozer:
    ///
    /// - `HeapValue::TypedObject` no longer carries the inline `{schema_id,
    ///   slots, heap_mask}` shape — it now wraps `Arc<TypedObjectStorage>`
    ///   per ADR-006 §2.3.
    /// - `ValueWord`, `ValueWordExt`, `ArgVec`, and `vmarray_from_vec` are
    ///   deleted from `shape-value`.
    /// - The per-FieldType constructor surface (`ValueSlot::from_string_arc`,
    ///   `from_typed_array`, `from_typed_object`) per ADR-006 §2.4 is the
    ///   replacement for ValueSlot construction, but the comptime-target
    ///   serialization additionally needs a kind-threaded
    ///   `typed_object_from_pairs` that takes `&[(&str, KindedSlot)]` and
    ///   builds `Arc<TypedObjectStorage>` directly. That helper is part of
    ///   the comptime-rebuild surface and is deferred to Phase 2c (per
    ///   playbook §7 #4).
    ///
    /// Until Phase 2c lands, `to_nanboxed` panics rather than emitting a
    /// placeholder TypedObject that would silently drop `optional` flags,
    /// `field_annotations`, or `captures` — comptime annotation handlers
    /// rely on these for AI-first metadata propagation, so partial output
    /// would corrupt downstream `@ai`/`@description`/`@range` resolution.
    pub fn to_nanboxed(&self) -> KindedSlot {
        let _ = (
            &self.kind,
            &self.name,
            &self.fields,
            &self.params,
            &self.return_type,
            &self.annotations,
            &self.captures,
            is_option_type as fn(&str) -> bool,
            unwrap_option_type as fn(&str) -> String,
        );
        todo!("phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4")
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

    // phase-2c: comptime target serialization rebuild — see ADR-006 §2.4.
    // The previous body asserted `value.type_name() == "object"` against a
    // ValueWord TypedObject; both the construction (`to_nanboxed`) and the
    // observation surface (`ValueWord::type_name`) are deleted in the
    // strict-typing bulldozer. Re-enable when the kind-threaded
    // KindedSlot-returning `to_nanboxed` rebuild lands.
    #[test]
    #[ignore = "phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4"]
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

        let _value = target.to_nanboxed();
    }

    #[test]
    #[ignore = "phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4"]
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

        let _value = target.to_nanboxed();
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
    #[ignore = "phase-2c — comptime rebuild against typed-Arc HeapValue layout — see ADR-006 §2.4"]
    fn test_target_captures_vmvalue_included() {
        // Verify captures field appears in the comptime-target serialization.
        let target = ComptimeTarget {
            kind: AnnotationTargetKind::Function,
            name: "closure".to_string(),
            fields: Vec::new(),
            params: Vec::new(),
            return_type: None,
            annotations: Vec::new(),
            captures: vec!["x".to_string(), "y".to_string()],
        };

        let _value = target.to_nanboxed();
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

        // The serialized-form assertion (TypedObject field readback via the
        // deleted `typed_object_to_hashmap_nb` + `as_any_array` ValueWord
        // helpers) is deferred to phase-2c — see ADR-006 §2.4. The struct
        // shape above is the material AST→target invariant; the readback
        // test is the consumer side of the comptime-rebuild surface.
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
