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
//! - `type_ref` siblings: typed descriptors beside each string type field
//! - `annotations`: array of annotation names already applied

// ADR-009 E1 #17 (slice 5): the producer-stamped semantic identity carried by
// each `__ComptimeTypeRef`. Re-exported by `comptime_builtins`; the type_ref's
// consumer resolves the SAME identity via `reconstruct_type_annotation`.
use crate::compiler::comptime_builtins::{FreezeOverlay, FrozenTypeIdentity};
use shape_ast::ast::functions::Annotation;
pub(crate) use shape_ast::ast::functions::AnnotationTargetKind;
use shape_ast::ast::literals::Literal;
use shape_ast::ast::{Expr, FunctionDef, TypeAnnotation};
use shape_ast::error::ShapeError;
use shape_value::heap_value::TypedObjectStorage;
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::{
    ELEM_TYPE_STRING, ELEM_TYPE_TYPED_OBJECT, TypedArray, stamp_elem_type,
};
use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
// V3-S5: old `TypedArrayData` / `typed_buffer::TypedBuffer` carriers stay
// deleted. The comptime target arrays below use stamped v2-raw typed arrays:
// `TypedArray<*const StringObj>` and `TypedArray<*const TypedObjectStorage>`.
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

/// Build a string-kinded slot from an owned `String`.
fn nb_string(s: String) -> KindedSlot {
    KindedSlot::from_string_arc(Arc::new(s))
}

fn type_ref_kind_from_source(source: &str) -> &'static str {
    let source = source.trim();
    if is_option_type(source) {
        return "Option";
    }
    if source.starts_with('[') || source.starts_with("Array<") || source.starts_with("Vec<") {
        return "Array";
    }
    if source.starts_with("HashMap<") {
        return "HashMap";
    }
    if source.starts_with("Result<") {
        return "Result";
    }
    if source.contains("=>") {
        return "Function";
    }
    if source.starts_with("dyn ") {
        return "TraitObject";
    }
    match source {
        "int" | "i64" | "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" => "Int",
        "number" | "f64" | "f32" | "float" => "Number",
        "bool" => "Bool",
        "string" | "str" => "String",
        "decimal" => "Decimal",
        "bigint" => "BigInt",
        "()" | "unit" | "void" => "Unit",
        _ if source.chars().next().is_some_and(|ch| ch.is_uppercase()) => "TypedObject",
        _ => "Unresolved",
    }
}

fn type_ref_name_from_source(source: &str) -> String {
    let source = source.trim();
    if source.starts_with('[') || source.starts_with("Array<") || source.starts_with("Vec<") {
        return "Array".to_string();
    }
    if source.starts_with("HashMap<") {
        return "HashMap".to_string();
    }
    if source.starts_with("Result<") {
        return "Result".to_string();
    }
    if is_option_type(source) {
        return "Option".to_string();
    }
    source.to_string()
}

/// Build the `__ComptimeTypeRef` carrier for a type.
///
/// ADR-009 E5 CKPT-5: the `.source` reparse-fallback FIELD is DELETED. This
/// descriptor carries the producer-stamped `FrozenTypeIdentity`
/// (`identity_high`/`identity_low`) plus the `name`/`kind` spell/reflect-only
/// fields. `spelling` is the type's display spelling used SOLELY to derive the
/// surviving `name`/`kind` reflection fields (the U02 corpus reads
/// `type_ref.kind`, e.g. serde `derive.shape`); it is NOT stored for reparse —
/// there is no `.source` field, and the consumer resolves a stamped ref
/// identity-only via `reconstruct_type_annotation`.
///
/// An unstamped ref carries INVALID ({-1,-1}); the consumer
/// (`type_annotation_from_string_or_type_ref_slot`) rules an INVALID identity a
/// NAMED surface-and-stop — never a silent reparse (the fallback arm no longer
/// exists, so the stamped->reparse walk-back is structurally impossible). The int
/// halves are stamped exactly as `build_frozen_type_ref_heap_value`
/// (type_reflection.rs) does for the sibling schema.
pub(crate) fn build_type_ref_descriptor(
    spelling: &str,
    kind: Option<&str>,
    identity: Option<FrozenTypeIdentity>,
) -> KindedSlot {
    use shape_runtime::type_schema::typed_object_for_named_schema;

    let spelling = spelling.trim();
    let kind = kind.unwrap_or_else(|| type_ref_kind_from_source(spelling));
    let id = identity.unwrap_or(FrozenTypeIdentity::INVALID);
    typed_object_for_named_schema(
        "__ComptimeTypeRef",
        &[
            ("name", nb_string(type_ref_name_from_source(spelling))),
            ("kind", nb_string(kind.to_string())),
            ("identity_high", KindedSlot::from_int(id.high)),
            ("identity_low", KindedSlot::from_int(id.low)),
        ],
    )
}

/// ADR-009 E1 #17 (slice 5, A-FULL) STAMP-GATE (E1-D7(a)+(b)).
///
/// Compute the producer-side [`FrozenTypeIdentity`] for `ast` ONLY when it will
/// reconstruct — the SAME predicate the consumer resolves with
/// (`reconstruct_type_annotation(...).is_ok()`), so producer and consumer share
/// ONE code path (E1-D7(b), no parallel gate logic). Identity comes ONLY from
/// [`FreezeOverlay::canonicalize_type_projection`] (projection.rs), which BOTH
/// computes the identity AND interns the composite payload into the shared
/// `overlay.composites` memo — required so the consumer's `payload_of` sees the
/// same composite evidence off the shared `Arc<FreezeOverlay>`. No second hasher
/// is written (E1-D7(b)).
///
/// A canonicalize error (a non-freezable ref) is SWALLOWED to `None` — NEVER
/// propagated — so the `__ComptimeTypeRef` carries `INVALID`, and the consumer
/// (`type_annotation_from_string_or_type_ref_slot`) rules an INVALID identity a
/// NAMED surface-and-stop (E5 CKPT-4 ruling). Post-E5 CKPT-5 there is NO `.source`
/// reparse arm to fall through to — the fallback FIELD and its reparse arm are
/// DELETED, so an unreconstructable ref rejects LOUD and the stamped->reparse
/// walk-back is structurally impossible.
/// After CKPT-1..3 the stamp-gate ADMITS every reconstructable type — leaves,
/// applied-generic nominals (`Array<int>`/`Option<T>`), tuples, records, and bare
/// user-nominals all stamp a real identity. Only genuinely unreconstructable refs
/// (an unresolved return, a synthetic member, a scoped generic parameter, an
/// un-applied generic head) stay `INVALID` → the consumer's NAMED rejection.
fn stamp_for(
    overlay: Option<&FreezeOverlay>,
    ast: Option<&TypeAnnotation>,
) -> Option<FrozenTypeIdentity> {
    let overlay = overlay?;
    let ast = ast?;
    match overlay.canonicalize_type_projection(ast) {
        Ok(projection)
            if crate::compiler::comptime_builtins::reconstruct_type_annotation(
                overlay,
                projection.identity(),
            )
            .is_ok() =>
        {
            Some(projection.identity())
        }
        _ => None,
    }
}

/// Build an `Array<string>` slot carried by a stamped v2-raw
/// `TypedArray<*const StringObj>`.
fn nb_string_array(strings: Vec<String>) -> Result<KindedSlot, ShapeError> {
    let arr = TypedArray::<*const StringObj>::with_capacity(strings.len() as u32);
    unsafe {
        stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
        for s in strings {
            let ptr = StringObj::new(s.as_str()) as *const StringObj;
            TypedArray::<*const StringObj>::push(arr, ptr);
        }
    }
    Ok(KindedSlot::new(
        ValueSlot::from_raw(arr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
}

/// Build an `Array<TypedObject>` slot carried by a stamped v2-raw
/// `TypedArray<*const TypedObjectStorage>`. Each element's refcount share is
/// transferred into the array.
fn nb_object_array(objs: Vec<KindedSlot>) -> Result<KindedSlot, ShapeError> {
    let arr = TypedArray::<*const TypedObjectStorage>::with_capacity(objs.len() as u32);
    unsafe {
        stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT);
    }
    for obj in objs {
        if obj.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
            unsafe {
                TypedArray::<*const TypedObjectStorage>::drop_array_heap(arr);
            }
            return Err(ShapeError::RuntimeError {
                message: format!(
                    "comptime_target::nb_object_array expected TypedObject element, got {:?}",
                    obj.kind()
                ),
                location: None,
            });
        }
        let ptr = obj.raw() as *const TypedObjectStorage;
        unsafe {
            TypedArray::<*const TypedObjectStorage>::push(arr, ptr);
        }
        // Transfer the element's refcount share into the array.
        std::mem::forget(obj);
    }
    Ok(KindedSlot::new(
        ValueSlot::from_raw(arr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
}

/// Build the `Array<FieldDescriptor>` slot for `target.fields` (annotation
/// handlers). Each row is a
/// `__ComptimeFieldDescriptor` TypedObject `{name, type, annotations, optional}`;
/// a top-level `Option<T>` / `T?` field type is unwrapped to `T` with `optional`
/// set true (comptime-excellence §4.1.1). Both introspection surfaces produce
/// identical rows from this one builder.
/// `field_type_asts` is index-parallel to `fields` (E1 slice-5 R2+H1); it is the
/// original per-field `TypeAnnotation` AST, or empty (`&[]`) on reflection
/// surfaces that carry no AST. The producer stamps each field's `type_ref`
/// identity via the shared [`stamp_for`] gate. A `None` overlay (or absent AST)
/// leaves the stamp `INVALID`; post-E5 CKPT-5 the consumer rules an INVALID field
/// type_ref LOUD (no `.source` reparse — the fallback field + arm are deleted).
pub(crate) fn build_field_descriptor_array(
    fields: &[(String, String, Vec<FieldAnnotation>)],
    overlay: Option<&FreezeOverlay>,
    field_type_asts: &[Option<TypeAnnotation>],
) -> Result<KindedSlot, ShapeError> {
    use shape_runtime::type_schema::typed_object_for_named_schema;

    let mut field_objs: Vec<KindedSlot> = Vec::with_capacity(fields.len());
    for (idx, (fname, ftype, fanns)) in fields.iter().enumerate() {
        // Each annotation becomes {name, args} where args is an array of
        // stringified arg values.
        let mut ann_objs: Vec<KindedSlot> = Vec::with_capacity(fanns.len());
        for (aname, aargs) in fanns {
            let args_arr = nb_string_array(aargs.clone())?;
            ann_objs.push(typed_object_for_named_schema(
                "__ComptimeAnnotationDescriptor",
                &[("name", nb_string(aname.clone())), ("args", args_arr)],
            ));
        }
        let anns_arr = nb_object_array(ann_objs)?;
        let is_optional = is_option_type(ftype);
        let effective_type = if is_optional {
            unwrap_option_type(ftype)
        } else {
            ftype.clone()
        };
        // ADR-009 E5 CKPT-4 (design §2 class B — MIGRATE; the pre-CKPT-4
        // `!is_optional → None` gate is DROPPED). The stamp AST must render to the
        // field descriptor's emitted `.type` spelling, which is
        // `unwrap_option_type(ftype)` — the UNWRAPPED inner for an optional field
        // (the Option-ness rides on the separate `optional` flag, not the
        // type_ref). So for an OPTIONAL field the stamp AST is the UNWRAPPED-INNER
        // AST (`option_inner`): its frozen identity reconstructs to exactly the
        // inner type the descriptor names, keeping spelling↔identity consistent AND
        // stamping the field. (Applied `Option<T>` itself reconstructs
        // post-CKPT-1, but this descriptor DESCRIBES its inner `T`.) A non-optional
        // field stamps its full declared AST. A `None` overlay or absent AST leaves
        // the stamp `INVALID` (the consumer rules an unstamped ref LOUD — no
        // reparse, the `.source` fallback is DELETED — ADR-009 E5 CKPT-5).
        let field_ast = field_type_asts.get(idx).and_then(|a| a.as_ref());
        let stamp_ast = if is_optional {
            field_ast.and_then(TypeAnnotation::option_inner)
        } else {
            field_ast
        };
        let field_identity = stamp_for(overlay, stamp_ast);
        field_objs.push(typed_object_for_named_schema(
            "__ComptimeFieldDescriptor",
            &[
                ("name", nb_string(fname.clone())),
                ("type", nb_string(effective_type)),
                ("annotations", anns_arr),
                ("optional", KindedSlot::from_bool(is_optional)),
                (
                    "type_ref",
                    build_type_ref_descriptor(&unwrap_option_type(ftype), None, field_identity),
                ),
            ],
        ));
    }
    nb_object_array(field_objs)
}

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
    // ADR-009 E1 #17 (slice 5, A-FULL) — R2+H1 additive parallel AST carriers
    // for the producer stamp-gate. Each is INDEX-COUPLED to a string tuple vec
    // above: `param_type_asts[i]` ↔ `params[i]`, `field_type_asts[i]` ↔
    // `fields[i]`, `return_type_ast` ↔ `return_type`. INVARIANT: populated in
    // LOCKSTEP inside the constructor bodies ONLY, never mutated elsewhere.
    //
    // ADR-006: these are ADDITIVE parallel vecs, NOT a widening of the existing
    // string tuples. `comptime_target_dependency_descriptors`
    // (functions_annotations.rs) — the D1 ExpansionIdentity reader — destructures
    // ONLY `params`/`fields`/`return_type`, which stay byte-identical, so the
    // pre-pass↔pass-2 expansion identity is unperturbed. Identity/AST never feeds
    // that reader.
    /// AST for each parameter's declared type (index-parallel to `params`).
    pub param_type_asts: Vec<Option<TypeAnnotation>>,
    /// AST for the function return type (parallel to `return_type`).
    pub return_type_ast: Option<TypeAnnotation>,
    /// AST for each field's declared type (index-parallel to `fields`).
    pub field_type_asts: Vec<Option<TypeAnnotation>>,
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
        // E1 slice-5 R2+H1: parallel AST vec, LOCKSTEP with `params` (same
        // `func.params` iteration order/length).
        let param_type_asts: Vec<Option<TypeAnnotation>> = func
            .params
            .iter()
            .map(|p| p.type_annotation.clone())
            .collect();

        let return_type = func.return_type.as_ref().map(type_annotation_to_string);
        let return_type_ast = func.return_type.clone();

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
            param_type_asts,
            return_type_ast,
            field_type_asts: Vec::new(),
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
        // E1 slice-5 R2+H1: parallel AST vec, LOCKSTEP with the `fields` string
        // tuple vec below (same `fields` iteration order/length).
        let field_type_asts: Vec<Option<TypeAnnotation>> =
            fields.iter().map(|(_, ftype, _)| ftype.clone()).collect();
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
            param_type_asts: Vec::new(),
            return_type_ast: None,
            field_type_asts,
        }
    }

    /// Create a target descriptor for a module definition.
    ///
    /// Module fields don't carry annotations, so they get empty annotation lists.
    ///
    /// ADR-009 E5 CKPT-4 (design §2 class A — MIGRATE): each field carries its
    /// declared-type AST (index-parallel to the `(name, type_string)` tuples), so
    /// a typed module member (`let x: int`) STAMPS its `type_ref` identity via the
    /// shared `stamp_for` gate when `to_nanboxed` is called with `Some(overlay)`.
    /// Synthetic members (functions / types / modules / annotations) carry no type
    /// AST (`None`) → `INVALID` stamp → `kind: "Unresolved"` → the consumer rules
    /// them LOUD (no reparse — the `.source` fallback is DELETED, E5 CKPT-5).
    pub fn from_module(name: &str, fields: &[(String, String, Option<TypeAnnotation>)]) -> Self {
        let field_type_asts: Vec<Option<TypeAnnotation>> =
            fields.iter().map(|(_, _, ast)| ast.clone()).collect();
        let fields = fields
            .iter()
            .map(|(n, t, _)| (n.clone(), t.clone(), Vec::new()))
            .collect();
        Self {
            kind: AnnotationTargetKind::Module,
            name: name.to_string(),
            fields,
            params: Vec::new(),
            return_type: None,
            annotations: Vec::new(),
            captures: Vec::new(),
            param_type_asts: Vec::new(),
            return_type_ast: None,
            field_type_asts,
        }
    }

    /// Create a target descriptor for an expression.
    pub fn for_expression() -> Self {
        Self {
            kind: AnnotationTargetKind::Expression,
            name: String::new(),
            fields: Vec::new(),
            params: Vec::new(),
            return_type: None,
            annotations: Vec::new(),
            captures: Vec::new(),
            param_type_asts: Vec::new(),
            return_type_ast: None,
            field_type_asts: Vec::new(),
        }
    }

    /// Convert this target to a `KindedSlot` TypedObject describing the
    /// annotated item.
    ///
    /// Builds a comptime-target descriptor as a kinded slot for handler
    /// dispatch.
    ///
    /// The outer target and nested rows are `TypedObjectStorage` values built
    /// through `typed_object_from_pairs`. String sub-arrays use
    /// `TypedArray<*const StringObj>` stamped as `ELEM_TYPE_STRING`; sub-arrays
    /// whose elements are typed-object rows use
    /// `TypedArray<*const TypedObjectStorage>` stamped as
    /// `ELEM_TYPE_TYPED_OBJECT`.
    ///
    /// Object-array construction requires each element to already carry
    /// `NativeKind::Ptr(HeapKind::TypedObject)`. A mismatched element is a
    /// structural compile-time error, not a runtime kind-inference fallback.
    ///
    /// ADR-009 E1 #17 (slice 5, A-FULL): when `overlay` is `Some`, each
    /// `type_ref` descriptor is producer-stamped with the semantic identity of
    /// its declared-type AST via the [`stamp_for`] gate (identity + shared
    /// composite-memo interning through `overlay.canonicalize_type_projection`).
    /// `None` (module/expression targets, tests) stamps `INVALID` everywhere.
    /// The consumer resolves a stamped ref identity-only via
    /// `reconstruct_type_annotation`; post-E5 CKPT-5 there is NO `.source` field or
    /// reparse arm, so an `INVALID` stamp is a NAMED consumer rejection, never a
    /// silent reparse.
    pub fn to_nanboxed(&self, overlay: Option<&FreezeOverlay>) -> Result<KindedSlot, ShapeError> {
        // S2 (comptime-excellence §4.3): every descriptor object is built
        // through `typed_object_for_named_schema`, which resolves a
        // reserved, concrete, pre-registered schema BY NAME. The previous
        // `register_predeclared_any_schema` + `typed_object_from_pairs`
        // path lazily minted an anonymous `__predecl_*` schema in the
        // compiler's ambient registry, whose late-allocated id collided
        // with a `__mod_*` module-object schema at the same numeric id in
        // the handler VM's bytecode registry (cross-registry schema-id
        // reuse). Named reserved schemas are registered at init in every
        // registry, so the baked-in `schema_id` means the same thing on
        // both sides of the boundary.
        use shape_runtime::type_schema::typed_object_for_named_schema;

        let nb_str = |s: &str| KindedSlot::from_string_arc(Arc::new(s.to_string()));
        let nb_string = |s: String| KindedSlot::from_string_arc(Arc::new(s));

        let kind_str = match self.kind {
            AnnotationTargetKind::Function => "function",
            AnnotationTargetKind::Type => "type",
            AnnotationTargetKind::Module => "module",
            AnnotationTargetKind::Expression => "expression",
            AnnotationTargetKind::Block => "block",
            AnnotationTargetKind::AwaitExpr => "await_expr",
            AnnotationTargetKind::Binding => "binding",
        };

        let nb_string_array = |strings: Vec<String>| -> Result<KindedSlot, ShapeError> {
            let arr = TypedArray::<*const StringObj>::with_capacity(strings.len() as u32);
            unsafe {
                stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
                for s in strings {
                    let ptr = StringObj::new(s.as_str()) as *const StringObj;
                    TypedArray::<*const StringObj>::push(arr, ptr);
                }
            }
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        };

        let nb_object_array = |objs: Vec<KindedSlot>| -> Result<KindedSlot, ShapeError> {
            let arr = TypedArray::<*const TypedObjectStorage>::with_capacity(objs.len() as u32);
            unsafe {
                stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT);
            }
            for obj in objs {
                if obj.kind() != NativeKind::Ptr(HeapKind::TypedObject) {
                    unsafe {
                        TypedArray::<*const TypedObjectStorage>::drop_array_heap(arr);
                    }
                    return Err(ShapeError::RuntimeError {
                        message: format!(
                            "comptime_target::nb_object_array expected TypedObject element, got {:?}",
                            obj.kind()
                        ),
                        location: None,
                    });
                }
                let ptr = obj.raw() as *const TypedObjectStorage;
                unsafe {
                    TypedArray::<*const TypedObjectStorage>::push(arr, ptr);
                }
                // Transfer the element's refcount share into the array.
                std::mem::forget(obj);
            }
            Ok(KindedSlot::new(
                ValueSlot::from_raw(arr as usize as u64),
                NativeKind::Ptr(HeapKind::TypedArray),
            ))
        };

        // fields: array of {name, type, annotations, optional} TypedObjects,
        // built through the shared `build_field_descriptor_array` row builder for
        // `target.fields`. The
        // field ASTs (E1 slice-5 R2+H1) thread through for per-field stamping.
        let fields_arr =
            build_field_descriptor_array(&self.fields, overlay, &self.field_type_asts)?;

        // params: array of {name, type, const} TypedObjects. The `type_ref` of
        // each param is producer-stamped (E1 slice-5) from `param_type_asts[i]`,
        // index-coupled to `params[i]`.
        let param_objs: Vec<KindedSlot> = self
            .params
            .iter()
            .enumerate()
            .map(|(idx, (pname, ptype, is_const))| {
                let identity = stamp_for(
                    overlay,
                    self.param_type_asts.get(idx).and_then(|a| a.as_ref()),
                );
                typed_object_for_named_schema(
                    "__ComptimeParamDescriptor",
                    &[
                        ("name", nb_string(pname.clone())),
                        ("type", nb_string(ptype.clone())),
                        ("const", KindedSlot::from_bool(*is_const)),
                        ("type_ref", build_type_ref_descriptor(ptype, None, identity)),
                    ],
                )
            })
            .collect();
        let params_arr = nb_object_array(param_objs)?;

        // return_type: optional string
        let ret = self
            .return_type
            .as_ref()
            .map(|r| nb_string(r.clone()))
            .unwrap_or_else(KindedSlot::none);
        // The return `type_ref` is producer-stamped from `return_type_ast`; the
        // synthetic "unknown" fallback (no declared return) stays INVALID.
        let ret_identity = stamp_for(overlay, self.return_type_ast.as_ref());
        let ret_ref = self
            .return_type
            .as_deref()
            .map(|r| build_type_ref_descriptor(r, None, ret_identity))
            .unwrap_or_else(|| build_type_ref_descriptor("unknown", Some("Unresolved"), None));

        // annotations: array of strings (names only)
        let ann_arr = nb_string_array(self.annotations.clone())?;

        // captures: array of strings (captured names)
        let captures_arr = nb_string_array(self.captures.clone())?;

        Ok(typed_object_for_named_schema(
            "__ComptimeTarget",
            &[
                ("kind", nb_str(kind_str)),
                ("name", nb_string(self.name.clone())),
                ("fields", fields_arr),
                ("params", params_arr),
                ("return_type", ret),
                ("return_type_ref", ret_ref),
                ("annotations", ann_arr),
                ("captures", captures_arr),
            ],
        ))
    }
}

/// Best-effort stringification of an annotation argument expression.
///
/// Annotation args are typically literals (string, number, bool). For anything
/// more complex we fall back to the Debug representation.
///
/// ADR-009 D1: this is also the canonical argument-descriptor rendering for
/// `ExpansionIdentity.arguments_hash` — it is exactly the stringification the
/// comptime handler itself consumes for field-annotation args, so identity
/// hashes what the expansion saw, never re-rendered source text.
pub(crate) fn expr_to_string_lossy(expr: &Expr) -> String {
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
pub(crate) fn type_annotation_to_string(ta: &TypeAnnotation) -> String {
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
        TypeAnnotation::Function {
            params, returns, ..
        } => {
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
        // ADR-009 B3 (S1): existential descriptor package type.
        TypeAnnotation::Existential { witnesses, inner } => {
            format!(
                "exists<{}> {}",
                witnesses.join(", "),
                type_annotation_to_string(inner)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::{DestructurePattern, FunctionDef, FunctionParameter, Span};
    use shape_value::v2::typed_array::read_elem_type;

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
                buffer_share: shape_ast::ast::BufferShare::Copied,
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
            effect_row: None,
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
            param_type_asts: Vec::new(),
            return_type_ast: None,
            field_type_asts: Vec::new(),
        };

        let _value = target.to_nanboxed(None);
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
            param_type_asts: Vec::new(),
            return_type_ast: None,
            field_type_asts: Vec::new(),
        };

        let _value = target.to_nanboxed(None);
    }

    #[test]
    fn target_to_nanboxed_uses_v2_raw_typed_arrays() {
        let target = ComptimeTarget {
            kind: AnnotationTargetKind::Function,
            name: "annotated".to_string(),
            fields: Vec::new(),
            params: vec![("x".to_string(), "int".to_string(), true)],
            return_type: Some("int".to_string()),
            annotations: vec!["memo".to_string(), "trace".to_string()],
            captures: vec!["outer".to_string()],
            param_type_asts: Vec::new(),
            return_type_ast: None,
            field_type_asts: Vec::new(),
        };

        let slot = target.to_nanboxed(None).unwrap();
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedObject));

        let storage = slot
            .as_typed_object_storage()
            .expect("target should be a typed object");
        assert_eq!(storage.slots().len(), 8);

        let params = storage.slots()[3].raw() as *const TypedArray<*const TypedObjectStorage>;
        let annotations = storage.slots()[6].raw() as *const TypedArray<*const StringObj>;
        let captures = storage.slots()[7].raw() as *const TypedArray<*const StringObj>;

        unsafe {
            assert_eq!(read_elem_type(params as *const u8), ELEM_TYPE_TYPED_OBJECT);
            assert_eq!(TypedArray::<*const TypedObjectStorage>::len(params), 1);

            assert_eq!(read_elem_type(annotations as *const u8), ELEM_TYPE_STRING);
            let ann_slice = TypedArray::<*const StringObj>::as_slice(annotations);
            assert_eq!(ann_slice.len(), 2);
            assert_eq!(StringObj::as_str(ann_slice[0]), "memo");
            assert_eq!(StringObj::as_str(ann_slice[1]), "trace");

            assert_eq!(read_elem_type(captures as *const u8), ELEM_TYPE_STRING);
            let captures_slice = TypedArray::<*const StringObj>::as_slice(captures);
            assert_eq!(captures_slice.len(), 1);
            assert_eq!(StringObj::as_str(captures_slice[0]), "outer");
        }
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
                    buffer_share: shape_ast::ast::BufferShare::Copied,
                    type_annotation: None,
                    default_value: None,
                },
                FunctionParameter {
                    pattern: DestructurePattern::Identifier("b".to_string(), Span::DUMMY),
                    is_const: false,
                    is_reference: false,
                    is_mut_reference: false,
                    is_out: false,
                    buffer_share: shape_ast::ast::BufferShare::Copied,
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
            effect_row: None,
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
            param_type_asts: Vec::new(),
            return_type_ast: None,
            field_type_asts: Vec::new(),
        };

        let _value = target.to_nanboxed(None);
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
