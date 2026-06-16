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
use shape_ast::error::ShapeError;
use shape_value::KindedSlot;
use shape_value::heap_value::TypedObjectStorage;
// V3-S5 ckpt-5 (2026-05-15): `TypedArrayData` + `typed_buffer::TypedBuffer`
// imports removed in lockstep with the `nb_string_array` / `nb_object_array`
// surface-and-stop builders below. Both wrappers were deleted at ckpt-1..
// ckpt-4 per W12 audit §3.5 + §B.
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
    /// Phase-2c rebuild (C2-comptime-rebuild): the kinded replacement uses
    /// `typed_object_from_pairs(&[(&str, KindedSlot)])` per ADR-006 §2.4
    /// (the per-FieldType constructor surface). Sub-arrays of string-typed
    /// elements use `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)`.
    /// Sub-arrays whose elements are themselves TypedObjects (the per-field
    /// `{name,type,annotations,optional}` rows, per-param rows, and
    /// per-annotation `{name,args}` rows) use
    /// `the-deleted-heterogeneous-element-carrier(Arc<TypedBuffer<Arc<HeapValue>>>)` —
    /// noting that this is the unmonomorphized carrier shape; the
    /// W17-typed-carrier-monomorphization sub-cluster is parallel-
    /// dispatched and will delete `the-deleted-heterogeneous-element-carrier` per
    /// ADR-006 §2.7.24 Q25.A. If that cluster lands first, this site
    /// rewires onto the specialized `TypedArrayData::TypedObject` arm; the
    /// territory is flagged in the C2 close report so the supervisor can
    /// verify lockstep.
    /// Build a comptime-target descriptor as a kinded slot for handler dispatch.
    ///
    /// R8 W9 G.2 Step 2 Bucket 7 (supervisor 2026-05-25): converted from
    /// `panic!`-on-unbounded-shape to `Err(ShapeError::RuntimeError)`. The
    /// inner `nb_string_array` / `nb_object_array` closures previously
    /// panicked unconditionally because their producer carriers
    /// (`Arc<TypedArrayData::{String, TypedObject}>` wrapped in the deleted
    /// `HeapValue::TypedArray` outer arm) were retired at V3-S5 ckpt-1..
    /// ckpt-4 per `docs/cluster-audits/w12-typed-array-data-deletion-audit.md`
    /// §3.5 + §B + ADR-006 §2.7.24 Q25.A SUPERSEDED — and the v2-raw
    /// `TypedArray<*const StringObj>` / `TypedArray<TypedObjectPtr>` rebuild
    /// lands at ckpt-6 STRICT close (not yet on main). Surface-and-stop via
    /// `Err` propagation through `execute_struct_comptime_handlers` /
    /// `execute_function_comptime_handler` / `execute_module_comptime_handlers`
    /// / `run_comptime_annotation_handlers_for_target`, mirroring the R8 W6
    /// G.2 mechanical pattern (panic\u{2192}Err conversion preserves
    /// CLAUDE.md "Forbidden rationalizations" — no kind-blind Bool default,
    /// no NaN-box fallback, no "documented out-of-scope" rationalization).
    ///
    /// Returns `Err(ShapeError::RuntimeError)` with a structured SURFACE
    /// message tracking the v0.4 / planned rebuild at v2-raw `TypedArray<T>`
    /// direct-access; root-cause fix lands at V3-S5 ckpt-6 STRICT close.
    pub fn to_nanboxed(&self) -> Result<KindedSlot, ShapeError> {
        use shape_runtime::type_schema::{
            register_predeclared_any_schema, typed_object_from_pairs,
        };

        let nb_str = |s: &str| KindedSlot::from_string_arc(Arc::new(s.to_string()));
        let nb_string = |s: String| KindedSlot::from_string_arc(Arc::new(s));
        let ensure_schema = |names: &[&str]| {
            let field_names: Vec<String> = names.iter().map(|name| (*name).to_string()).collect();
            let _ = register_predeclared_any_schema(&field_names);
        };

        // Pre-register the field schemas used below so
        // `typed_object_from_pairs` can resolve them.
        ensure_schema(&["name", "args"]);
        ensure_schema(&["name", "type", "const"]);
        ensure_schema(&["name", "type", "annotations", "optional"]);
        ensure_schema(&[
            "kind",
            "name",
            "fields",
            "params",
            "return_type",
            "annotations",
            "captures",
        ]);

        let kind_str = match self.kind {
            AnnotationTargetKind::Function => "function",
            AnnotationTargetKind::Type => "type",
            AnnotationTargetKind::Module => "module",
            AnnotationTargetKind::Expression => "expression",
            AnnotationTargetKind::Block => "block",
            AnnotationTargetKind::AwaitExpr => "await_expr",
            AnnotationTargetKind::Binding => "binding",
        };

        // V3-S5 ckpt-5 (2026-05-15) — R8 W9 G.2 Step 2 Bucket 7 mechanical
        // panic\u{2192}Err conversion (supervisor 2026-05-25). The original
        // `nb_string_array` + `nb_object_array` closures panicked
        // unconditionally because their producer carriers
        // (`Arc<TypedArrayData::{String, TypedObject}>` wrapped in the
        // deleted `HeapValue::TypedArray` outer arm) were retired at
        // V3-S5 ckpt-1..ckpt-4 per W12 audit §3.5 + §B + ADR-006 §2.7.24
        // Q25.A SUPERSEDED. The v2-raw `TypedArray<*const StringObj>` /
        // `TypedArray<TypedObjectPtr>` rebuild lands at ckpt-6 STRICT close
        // (not yet on main). Refusal #1 binding preserved; the conversion
        // surfaces a structured `Err(ShapeError::RuntimeError)` instead of
        // a runtime panic so caller-side recovery / diagnostics can route
        // the SURFACE message through the normal error reporting chain
        // rather than `catch_unwind` boundaries. Mirrors the R8 W6 G.2
        // mechanical pattern.
        let nb_string_array = |_strings: Vec<String>| -> Result<KindedSlot, ShapeError> {
            Err(ShapeError::RuntimeError {
                message: "comptime_target::nb_string_array: V3-S5 ckpt-5 \
                          consumer-cascade tier 3 SURFACE. \
                          the deleted typed-array-data String \
                          `Arc<Buf<Arc<String>>>` result carrier DELETED \
                          at ckpt-1..ckpt-4 per W12 audit \u{a7}3.5 + \u{a7}B \
                          + ADR-006 \u{a7}2.7.24 Q25.A SUPERSEDED. Rebuild \
                          lands at ckpt-6 STRICT close per v2-raw \
                          `TypedArray<*const StringObj>` direct-access. \
                          REFUSED ON SIGHT (Refusal #1). Feature impl \
                          pending (v0.4 / planned per \
                          `docs/v0.3-close-summary.md` \u{a7}5.16 \
                          JIT-lowering followup workstream)."
                    .to_string(),
                location: None,
            })
        };

        let nb_object_array = |objs: Vec<KindedSlot>| -> Result<KindedSlot, ShapeError> {
            // Drain the popped slots' shares before surfacing — preserves
            // refcount discipline through the Err return path (the inner
            // KindedSlot::Drop retires each share via §2.7.6 kind-aware
            // drop dispatch).
            for obj in objs {
                drop(obj);
            }
            // Suppress unused-import warning.
            let _ = TypedObjectStorage::_new;
            Err(ShapeError::RuntimeError {
                message: "comptime_target::nb_object_array: V3-S5 ckpt-5 \
                          consumer-cascade tier 3 SURFACE. \
                          the deleted typed-array-data TypedObject \
                          `Arc<Buf<TypedObjectPtr>>` result carrier DELETED \
                          at ckpt-1..ckpt-4 per W12 audit \u{a7}3.5 + \u{a7}B \
                          + ADR-006 \u{a7}2.7.24 Q25.A SUPERSEDED. Rebuild \
                          lands at ckpt-6 STRICT close per v2-raw \
                          `TypedArray<TypedObjectPtr>` direct-access. \
                          REFUSED ON SIGHT (Refusal #1). Feature impl \
                          pending (v0.4 / planned per \
                          `docs/v0.3-close-summary.md` \u{a7}5.16 \
                          JIT-lowering followup workstream)."
                    .to_string(),
                location: None,
            })
        };

        // fields: array of {name, type, annotations, optional} TypedObjects
        let mut field_objs: Vec<KindedSlot> = Vec::with_capacity(self.fields.len());
        for (fname, ftype, fanns) in &self.fields {
            // Each annotation becomes {name, args} where args is an
            // array of stringified arg values.
            let mut ann_objs: Vec<KindedSlot> = Vec::with_capacity(fanns.len());
            for (aname, aargs) in fanns {
                let args_arr = nb_string_array(aargs.clone())?;
                ann_objs.push(typed_object_from_pairs(&[
                    ("name", nb_string(aname.clone())),
                    ("args", args_arr),
                ]));
            }
            let anns_arr = nb_object_array(ann_objs)?;
            let is_optional = is_option_type(ftype);
            let effective_type = if is_optional {
                unwrap_option_type(ftype)
            } else {
                ftype.clone()
            };
            field_objs.push(typed_object_from_pairs(&[
                ("name", nb_string(fname.clone())),
                ("type", nb_string(effective_type)),
                ("annotations", anns_arr),
                ("optional", KindedSlot::from_bool(is_optional)),
            ]));
        }
        let fields_arr = nb_object_array(field_objs)?;

        // params: array of {name, type, const} TypedObjects
        let param_objs: Vec<KindedSlot> = self
            .params
            .iter()
            .map(|(pname, ptype, is_const)| {
                typed_object_from_pairs(&[
                    ("name", nb_string(pname.clone())),
                    ("type", nb_string(ptype.clone())),
                    ("const", KindedSlot::from_bool(*is_const)),
                ])
            })
            .collect();
        let params_arr = nb_object_array(param_objs)?;

        // return_type: optional string
        let ret = self
            .return_type
            .as_ref()
            .map(|r| nb_string(r.clone()))
            .unwrap_or_else(KindedSlot::none);

        // annotations: array of strings (names only)
        let ann_arr = nb_string_array(self.annotations.clone())?;

        // captures: array of strings (captured names)
        let captures_arr = nb_string_array(self.captures.clone())?;

        Ok(typed_object_from_pairs(&[
            ("kind", nb_str(kind_str)),
            ("name", nb_string(self.name.clone())),
            ("fields", fields_arr),
            ("params", params_arr),
            ("return_type", ret),
            ("annotations", ann_arr),
            ("captures", captures_arr),
        ]))
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
        TypeAnnotation::Borrow { mutable, inner } => {
            if *mutable {
                format!("&mut {}", type_annotation_to_string(inner))
            } else {
                format!("&{}", type_annotation_to_string(inner))
            }
        }
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
