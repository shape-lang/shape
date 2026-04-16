//! Typed map (HashMap) emission helpers for the native runtime.
//!
//! This module provides inference functions that determine the key and value
//! `ConcreteType` for a HashMap value at compile time. The compiler uses these
//! to populate the `map_key_value_types` side-table on `BytecodeCompiler`,
//! which downstream Phase 2/3 work consults to emit typed map opcodes.
//!
//! These are pure query functions — they do NOT modify compilation state.
//! Integration into the actual opcode emission paths happens elsewhere.

// Several public helpers in this module are wired in by Phase 2/3 work but
// are not yet consumed by any current production path. The unit tests
// exercise them. Silence the unused-code warnings for the lib build.
#![allow(dead_code)]

use shape_ast::ast::{Expr, Literal, ObjectEntry, Span, TypeAnnotation};
use shape_value::native::ConcreteType;
use shape_value::ValueWordExt;

use super::BytecodeCompiler;

impl BytecodeCompiler {
    /// Record a HashMap key/value `ConcreteType` pair for an AST node, keyed by
    /// span. Idempotent — overwrites any existing entry for the same span.
    pub(crate) fn record_map_key_value_for_node(
        &mut self,
        span: Span,
        key: ConcreteType,
        value: ConcreteType,
    ) {
        self.map_key_value_types.insert(span, (key, value));
    }

    /// Record a HashMap key/value `ConcreteType` pair for a local slot.
    pub(crate) fn record_map_key_value_for_local(
        &mut self,
        slot: u16,
        key: ConcreteType,
        value: ConcreteType,
    ) {
        self.local_map_key_value_types.insert(slot, (key, value));
    }

    /// Record a HashMap key/value `ConcreteType` pair for a module binding.
    pub(crate) fn record_map_key_value_for_module_binding(
        &mut self,
        binding_idx: u16,
        key: ConcreteType,
        value: ConcreteType,
    ) {
        self.module_binding_map_key_value_types
            .insert(binding_idx, (key, value));
    }

    /// Look up the HashMap key/value pair for a local slot, if recorded.
    pub(crate) fn map_key_value_for_local(
        &self,
        slot: u16,
    ) -> Option<&(ConcreteType, ConcreteType)> {
        self.local_map_key_value_types.get(&slot)
    }

    /// Look up the HashMap key/value pair for a module binding, if recorded.
    pub(crate) fn map_key_value_for_module_binding(
        &self,
        binding_idx: u16,
    ) -> Option<&(ConcreteType, ConcreteType)> {
        self.module_binding_map_key_value_types.get(&binding_idx)
    }

    /// Look up the HashMap key/value pair for an AST node span, if recorded.
    pub(crate) fn map_key_value_for_node(
        &self,
        span: Span,
    ) -> Option<&(ConcreteType, ConcreteType)> {
        self.map_key_value_types.get(&span)
    }

    /// Try to record a HashMap binding from a `let` annotation. Returns `true`
    /// if the annotation specified a HashMap with resolvable key/value types
    /// and `false` otherwise.
    ///
    /// `slot` is the local slot or module binding index, depending on `is_local`.
    pub(crate) fn try_track_hashmap_binding_from_annotation(
        &mut self,
        annotation: &TypeAnnotation,
        slot: u16,
        is_local: bool,
        init_span: Option<Span>,
    ) -> bool {
        if let Some((k, v)) = map_key_value_from_annotation(annotation) {
            if is_local {
                self.record_map_key_value_for_local(slot, k.clone(), v.clone());
            } else {
                self.record_map_key_value_for_module_binding(slot, k.clone(), v.clone());
            }
            if let Some(span) = init_span {
                self.record_map_key_value_for_node(span, k, v);
            }
            true
        } else {
            false
        }
    }

    /// Resolve a receiver expression's HashMap key/value types, if known.
    /// Walks identifier receivers back to the local/module binding side-table.
    pub(crate) fn resolve_receiver_map_key_value(
        &self,
        receiver: &Expr,
    ) -> Option<(ConcreteType, ConcreteType)> {
        match receiver {
            Expr::Identifier(name, _) => {
                if let Some(slot) = self.resolve_local(name) {
                    if let Some(kv) = self.map_key_value_for_local(slot).cloned() {
                        return Some(kv);
                    }
                }
                if let Some(&binding_idx) = self.module_bindings.get(name) {
                    if let Some(kv) = self.map_key_value_for_module_binding(binding_idx).cloned()
                    {
                        return Some(kv);
                    }
                }
                // Also try to derive a HashMap<K,V> from the type tracker's
                // tracked type name for this identifier, e.g. "HashMap<string,
                // int>" — picks up bindings whose annotation went through the
                // generic type-tracker path but not the native side-table.
                if let Some(slot) = self.resolve_local(name) {
                    if let Some(info) = self.type_tracker.get_local_type(slot) {
                        if let Some(name) = info.type_name.as_deref() {
                            if let Some(kv) = parse_hashmap_kv_from_tracked_name(name) {
                                return Some(kv);
                            }
                        }
                    }
                }
                if let Some(&binding_idx) = self.module_bindings.get(name) {
                    if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                        if let Some(name) = info.type_name.as_deref() {
                            if let Some(kv) = parse_hashmap_kv_from_tracked_name(name) {
                                return Some(kv);
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Record the element `ConcreteType` for an array-shaped expression at
    /// the given AST span. Used by Phase 1.2 / 2.2 / 3.2 tracking — array
    /// literal compilation, array method dispatch, etc.
    pub(crate) fn record_array_element_type(&mut self, span: Span, element: ConcreteType) {
        self.array_element_types.insert(span, element);
    }

    /// Look up the recorded array element `ConcreteType` for a span, if any.
    pub(crate) fn get_array_element_type(&self, span: Span) -> Option<&ConcreteType> {
        self.array_element_types.get(&span)
    }

    /// Whether a receiver expression resolves to a tracked native typed map.
    /// Used by [`compile_expr_method_call`] to gate the typed-map fast path.
    ///
    /// Walks the receiver name through the native typed-map locals/module bindings
    /// side-table only — does NOT consult the type tracker fallback. The
    /// stricter check ensures we never emit typed map opcodes for receivers
    /// allocated as legacy NaN-boxed `HashMapData`.
    pub(crate) fn is_typed_map_receiver(&self, receiver: &Expr) -> bool {
        let name = match receiver {
            Expr::Identifier(name, _) => name,
            _ => return false,
        };
        if let Some(local_idx) = self.resolve_local(name) {
            if self.v2_typed_map_locals.contains_key(&local_idx) {
                return true;
            }
        }
        if let Some(&binding_idx) = self.module_bindings.get(name) {
            if self.v2_typed_map_module_bindings.contains_key(&binding_idx) {
                return true;
            }
        }
        false
    }

    /// Resolve a typed-map receiver expression to its
    /// [`crate::compiler::typed_map_emission::TypedMapKind`]. Returns
    /// `None` for non-identifier receivers and for receivers that aren't
    /// tracked as native typed maps.
    pub(crate) fn resolve_receiver_typed_map_kind(
        &self,
        receiver: &Expr,
    ) -> Option<crate::compiler::typed_map_emission::TypedMapKind> {
        let name = match receiver {
            Expr::Identifier(name, _) => name,
            _ => return None,
        };
        if let Some(local_idx) = self.resolve_local(name) {
            if let Some(&kind) = self.v2_typed_map_locals.get(&local_idx) {
                return Some(kind);
            }
        }
        if let Some(&binding_idx) = self.module_bindings.get(name) {
            if let Some(&kind) = self.v2_typed_map_module_bindings.get(&binding_idx) {
                return Some(kind);
            }
        }
        None
    }
}

/// Bridge between the BytecodeCompiler's type tracker and the native typed-map
/// side-table. For a call expression that may be a `HashMap()` constructor,
/// inspect the surrounding context (the call's recorded span side-table, the
/// pending variable's type annotation captured before init compilation) and
/// return a `(K, V)` `ConcreteType` pair when one can be derived.
///
/// Returns `None` for any non-resolvable shape — callers MUST fall back to
/// the legacy `BuiltinCall(HashMapCtor)` path in that case.
///
/// Phase 3.2: This is the single point where the compiler asks "do we have
/// enough type information to lower this `HashMap()` constructor (or method
/// dispatch) to a typed-map opcode?". When type inference resolves both K
/// and V, this returns Some.
pub(crate) fn infer_hashmap_kv_from_context(
    compiler: &BytecodeCompiler,
    expr: &Expr,
) -> Option<(ConcreteType, ConcreteType)> {
    // 1. Side-table by AST span (populated either by annotation tracking or
    //    by inference helpers earlier in the compilation pass).
    let span = shape_ast::ast::Spanned::span(expr);
    if let Some(kv) = compiler.map_key_value_for_node(span).cloned() {
        return Some(kv);
    }
    // 2. Pending variable typed-map kind, set by the enclosing
    //    `let m: HashMap<K, V> = ...` annotation BEFORE init compilation.
    if let Some(kind) = compiler.pending_variable_typed_map_kind {
        return Some(typed_map_kind_to_concrete_kv(kind));
    }
    None
}

/// Convert a [`crate::compiler::typed_map_emission::TypedMapKind`] back
/// into a `(K, V)` `ConcreteType` pair. Used by
/// [`infer_hashmap_kv_from_context`] when only the typed-map kind has been
/// captured (no full annotation/side-table entry exists).
fn typed_map_kind_to_concrete_kv(
    kind: crate::compiler::typed_map_emission::TypedMapKind,
) -> (ConcreteType, ConcreteType) {
    use crate::compiler::typed_map_emission::TypedMapKind;
    match kind {
        TypedMapKind::StringF64 => (ConcreteType::String, ConcreteType::F64),
        TypedMapKind::StringI64 => (ConcreteType::String, ConcreteType::I64),
        // StringPtr collapses many V types onto the ptr-shaped slot. Use
        // String as the canonical V; this is enough for downstream
        // verification (callers only check `should_use_typed_map`).
        TypedMapKind::StringPtr => (ConcreteType::String, ConcreteType::String),
        TypedMapKind::I64F64 => (ConcreteType::I64, ConcreteType::F64),
        TypedMapKind::I64I64 => (ConcreteType::I64, ConcreteType::I64),
        TypedMapKind::I64Ptr => (ConcreteType::I64, ConcreteType::String),
    }
}

/// Parse a tracked type-tracker name like `"HashMap<string, int>"` into a
/// `(K, V)` `ConcreteType` pair. Returns `None` for non-HashMap shapes or
/// inner types that don't map cleanly onto `ConcreteType`.
///
/// Used by [`BytecodeCompiler::resolve_receiver_map_key_value`] as a fallback
/// when the native side-tables don't carry an entry but the type tracker does.
pub(crate) fn parse_hashmap_kv_from_tracked_name(name: &str) -> Option<(ConcreteType, ConcreteType)> {
    let trimmed = name.trim();
    let inner = trimmed
        .strip_prefix("HashMap<")
        .or_else(|| trimmed.strip_prefix("Map<"))?
        .strip_suffix('>')?;
    // Split on the top-level comma. Generic params are scalar in the cases
    // we care about (string/int/number/etc.) so a single split is fine.
    let mut depth = 0i32;
    let mut split_idx = None;
    for (i, c) in inner.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => {
                split_idx = Some(i);
                break;
            }
            _ => {}
        }
    }
    let idx = split_idx?;
    let k_name = inner[..idx].trim();
    let v_name = inner[idx + 1..].trim();
    let k = scalar_name_to_concrete(k_name)?;
    let v = scalar_name_to_concrete(v_name)?;
    Some((k, v))
}

/// Map a scalar type name to a `ConcreteType`. Mirrors the small basic-type
/// table in [`concrete_type_from_annotation`] for the names commonly seen in
/// type-tracker `type_name` strings.
fn scalar_name_to_concrete(name: &str) -> Option<ConcreteType> {
    match name {
        "number" | "float" | "f64" | "Number" => Some(ConcreteType::F64),
        "int" | "i64" | "integer" | "Int" => Some(ConcreteType::I64),
        "i32" => Some(ConcreteType::I32),
        "i16" => Some(ConcreteType::I16),
        "i8" => Some(ConcreteType::I8),
        "u64" => Some(ConcreteType::U64),
        "u32" => Some(ConcreteType::U32),
        "u16" => Some(ConcreteType::U16),
        "u8" => Some(ConcreteType::U8),
        "bool" | "boolean" | "Bool" => Some(ConcreteType::Bool),
        "string" | "str" | "String" => Some(ConcreteType::String),
        "decimal" | "Decimal" => Some(ConcreteType::Decimal),
        "bigint" | "BigInt" => Some(ConcreteType::BigInt),
        "DateTime" | "datetime" | "Time" => Some(ConcreteType::DateTime),
        _ => None,
    }
}

/// Recognise an annotation that names a HashMap and extract its key/value
/// `ConcreteType`s.
///
/// Recognised forms:
/// - `HashMap<K, V>`
/// - `Map<K, V>` (alias)
/// - `Option<HashMap<K, V>>` / `HashMap<K, V>?`
///
/// Returns `None` for unsupported shapes (non-HashMap generics, missing args,
/// inner types that don't map to a `ConcreteType`, etc).
pub fn map_key_value_from_annotation(
    annotation: &TypeAnnotation,
) -> Option<(ConcreteType, ConcreteType)> {
    match annotation {
        TypeAnnotation::Generic { name, args }
            if (name == "HashMap" || name == "Map") && args.len() == 2 =>
        {
            let k = concrete_type_from_annotation(&args[0])?;
            let v = concrete_type_from_annotation(&args[1])?;
            Some((k, v))
        }
        // `HashMap<K, V>?` desugars to Generic { name: "Option", args: [HashMap<K,V>] }
        TypeAnnotation::Generic { name, args } if name == "Option" && args.len() == 1 => {
            map_key_value_from_annotation(&args[0])
        }
        _ => None,
    }
}

/// Map a type annotation to a `ConcreteType`. Used for both HashMap key and
/// value type extraction. Returns `None` for type shapes that don't map cleanly
/// (unresolved generics, custom struct names that aren't yet registered, etc).
pub fn concrete_type_from_annotation(annotation: &TypeAnnotation) -> Option<ConcreteType> {
    match annotation {
        TypeAnnotation::Basic(name) => match name.as_str() {
            "number" | "float" | "f64" => Some(ConcreteType::F64),
            "int" | "i64" | "integer" => Some(ConcreteType::I64),
            "i32" => Some(ConcreteType::I32),
            "i16" => Some(ConcreteType::I16),
            "i8" => Some(ConcreteType::I8),
            "u64" => Some(ConcreteType::U64),
            "u32" => Some(ConcreteType::U32),
            "u16" => Some(ConcreteType::U16),
            "u8" => Some(ConcreteType::U8),
            "bool" | "boolean" => Some(ConcreteType::Bool),
            "string" | "str" => Some(ConcreteType::String),
            "decimal" => Some(ConcreteType::Decimal),
            "bigint" => Some(ConcreteType::BigInt),
            "DateTime" | "datetime" | "Time" => Some(ConcreteType::DateTime),
            "void" | "unit" => Some(ConcreteType::Void),
            // Unknown name — could be a user struct, but we don't have the
            // StructLayoutId registry wired here yet. Phase 1.1 Agent 3
            // will fill this in. For now, signal "not resolvable".
            _ => None,
        },
        TypeAnnotation::Reference(path) => {
            // Treat as Basic-style reference; same fallback semantics.
            concrete_type_from_annotation(&TypeAnnotation::Basic(path.to_string()))
        }
        TypeAnnotation::Array(inner) => {
            let elem = concrete_type_from_annotation(inner)?;
            Some(ConcreteType::Array(Box::new(elem)))
        }
        TypeAnnotation::Generic { name, args } => match name.as_str() {
            "Array" if args.len() == 1 => {
                let elem = concrete_type_from_annotation(&args[0])?;
                Some(ConcreteType::Array(Box::new(elem)))
            }
            "HashMap" | "Map" if args.len() == 2 => {
                let k = concrete_type_from_annotation(&args[0])?;
                let v = concrete_type_from_annotation(&args[1])?;
                Some(ConcreteType::HashMap(Box::new(k), Box::new(v)))
            }
            "Option" if args.len() == 1 => {
                let inner = concrete_type_from_annotation(&args[0])?;
                Some(ConcreteType::Option(Box::new(inner)))
            }
            "Result" if args.len() == 2 => {
                let ok = concrete_type_from_annotation(&args[0])?;
                let err = concrete_type_from_annotation(&args[1])?;
                Some(ConcreteType::Result(Box::new(ok), Box::new(err)))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Infer key/value types from an object literal that is being used as a
/// HashMap (e.g. `let m: HashMap<string, int> = { "a": 1, "b": 2 }`).
///
/// All keys are statically `string` (object literals only allow string keys
/// in Shape source). The value type is inferred from the homogeneous literal
/// values; mixed value types return `None`.
///
/// Returns `None` for empty object literals (no values to infer from), spread
/// objects (dynamic shape), and heterogeneous value types.
pub fn map_key_value_from_object_literal(
    entries: &[ObjectEntry],
) -> Option<(ConcreteType, ConcreteType)> {
    if entries.is_empty() {
        return None;
    }
    let mut value_kind: Option<ConcreteType> = None;

    for entry in entries {
        let value_expr = match entry {
            ObjectEntry::Field { value, .. } => value,
            // Spread objects can't be analyzed statically — bail.
            ObjectEntry::Spread(_) => return None,
        };

        let kind = literal_concrete_type(value_expr)?;
        match &value_kind {
            Some(prev) if *prev != kind => return None,
            Some(_) => {}
            None => value_kind = Some(kind),
        }
    }

    let value_kind = value_kind?;
    Some((ConcreteType::String, value_kind))
}

/// Map a literal-only expression to its `ConcreteType`. Returns `None` for
/// any non-literal expression — the inference is purely syntactic.
fn literal_concrete_type(expr: &Expr) -> Option<ConcreteType> {
    match expr {
        Expr::Literal(Literal::Number(_), _) => Some(ConcreteType::F64),
        Expr::Literal(Literal::Int(_), _) => Some(ConcreteType::I64),
        Expr::Literal(Literal::Bool(_), _) => Some(ConcreteType::Bool),
        Expr::Literal(Literal::String(_), _) => Some(ConcreteType::String),
        Expr::Literal(Literal::Decimal(_), _) => Some(ConcreteType::Decimal),
        Expr::Literal(Literal::TypedInt(_, w), _) => Some(typed_int_width_to_concrete(*w)),
        _ => None,
    }
}

/// Convert an `IntWidth` to the corresponding scalar `ConcreteType`.
fn typed_int_width_to_concrete(w: shape_ast::IntWidth) -> ConcreteType {
    use shape_ast::IntWidth;
    match w {
        IntWidth::I8 => ConcreteType::I8,
        IntWidth::U8 => ConcreteType::U8,
        IntWidth::I16 => ConcreteType::I16,
        IntWidth::U16 => ConcreteType::U16,
        IntWidth::I32 => ConcreteType::I32,
        IntWidth::U32 => ConcreteType::U32,
        IntWidth::U64 => ConcreteType::U64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::Span;
    use shape_ast::ast::type_path::TypePath;

    fn span() -> Span {
        Span::default()
    }

    fn ann_basic(name: &str) -> TypeAnnotation {
        TypeAnnotation::Basic(name.to_string())
    }

    fn hashmap_ann(k: TypeAnnotation, v: TypeAnnotation) -> TypeAnnotation {
        TypeAnnotation::Generic {
            name: TypePath::simple("HashMap"),
            args: vec![k, v],
        }
    }

    fn obj_field(key: &str, value: Expr) -> ObjectEntry {
        ObjectEntry::Field {
            key: key.to_string(),
            value,
            type_annotation: None,
        }
    }

    // ----------------------------------------------------------------
    // map_key_value_from_annotation
    // ----------------------------------------------------------------

    #[test]
    fn test_hashmap_string_int() {
        let ann = hashmap_ann(ann_basic("string"), ann_basic("int"));
        let (k, v) = map_key_value_from_annotation(&ann).expect("HashMap<string, int>");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(v, ConcreteType::I64);
    }

    #[test]
    fn test_hashmap_string_number() {
        let ann = hashmap_ann(ann_basic("string"), ann_basic("number"));
        let (k, v) = map_key_value_from_annotation(&ann).expect("HashMap<string, number>");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(v, ConcreteType::F64);
    }

    #[test]
    fn test_hashmap_int_bool() {
        let ann = hashmap_ann(ann_basic("int"), ann_basic("bool"));
        let (k, v) = map_key_value_from_annotation(&ann).expect("HashMap<int, bool>");
        assert_eq!(k, ConcreteType::I64);
        assert_eq!(v, ConcreteType::Bool);
    }

    #[test]
    fn test_hashmap_alias_map() {
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Map"),
            args: vec![ann_basic("string"), ann_basic("number")],
        };
        let (k, v) = map_key_value_from_annotation(&ann).expect("Map<string, number>");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(v, ConcreteType::F64);
    }

    #[test]
    fn test_hashmap_nested_array_value() {
        // HashMap<string, Array<int>>
        let inner = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![ann_basic("int")],
        };
        let ann = hashmap_ann(ann_basic("string"), inner);
        let (k, v) = map_key_value_from_annotation(&ann).expect("HashMap<string, Array<int>>");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(v, ConcreteType::Array(Box::new(ConcreteType::I64)));
    }

    #[test]
    fn test_hashmap_nested_hashmap_value() {
        // HashMap<string, HashMap<string, int>>
        let inner = hashmap_ann(ann_basic("string"), ann_basic("int"));
        let ann = hashmap_ann(ann_basic("string"), inner);
        let (k, v) =
            map_key_value_from_annotation(&ann).expect("HashMap<string, HashMap<string, int>>");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(
            v,
            ConcreteType::HashMap(Box::new(ConcreteType::String), Box::new(ConcreteType::I64),)
        );
    }

    #[test]
    fn test_non_hashmap_returns_none() {
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![ann_basic("int")],
        };
        assert_eq!(map_key_value_from_annotation(&ann), None);
    }

    #[test]
    fn test_hashmap_wrong_arity_returns_none() {
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("HashMap"),
            args: vec![ann_basic("string")],
        };
        assert_eq!(map_key_value_from_annotation(&ann), None);
    }

    #[test]
    fn test_basic_string_returns_none() {
        assert_eq!(map_key_value_from_annotation(&ann_basic("string")), None);
    }

    #[test]
    fn test_unresolved_user_type_returns_none() {
        // HashMap<string, MyStruct> — MyStruct is unknown
        let ann = hashmap_ann(ann_basic("string"), ann_basic("MyStruct"));
        assert_eq!(map_key_value_from_annotation(&ann), None);
    }

    #[test]
    fn test_optional_hashmap_unwraps() {
        // Option<HashMap<string, int>>
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Option"),
            args: vec![hashmap_ann(ann_basic("string"), ann_basic("int"))],
        };
        let (k, v) = map_key_value_from_annotation(&ann).expect("Option<HashMap<string, int>>");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(v, ConcreteType::I64);
    }

    // ----------------------------------------------------------------
    // map_key_value_from_object_literal
    // ----------------------------------------------------------------

    #[test]
    fn test_object_literal_string_int_values() {
        let entries = vec![
            obj_field("a", Expr::Literal(Literal::Int(1), span())),
            obj_field("b", Expr::Literal(Literal::Int(2), span())),
        ];
        let (k, v) = map_key_value_from_object_literal(&entries).expect("homogeneous int values");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(v, ConcreteType::I64);
    }

    #[test]
    fn test_object_literal_string_number_values() {
        let entries = vec![
            obj_field("x", Expr::Literal(Literal::Number(1.5), span())),
            obj_field("y", Expr::Literal(Literal::Number(2.5), span())),
        ];
        let (k, v) =
            map_key_value_from_object_literal(&entries).expect("homogeneous number values");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(v, ConcreteType::F64);
    }

    #[test]
    fn test_object_literal_string_string_values() {
        let entries = vec![
            obj_field("a", Expr::Literal(Literal::String("foo".into()), span())),
            obj_field("b", Expr::Literal(Literal::String("bar".into()), span())),
        ];
        let (k, v) =
            map_key_value_from_object_literal(&entries).expect("homogeneous string values");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(v, ConcreteType::String);
    }

    #[test]
    fn test_object_literal_string_bool_values() {
        let entries = vec![
            obj_field("a", Expr::Literal(Literal::Bool(true), span())),
            obj_field("b", Expr::Literal(Literal::Bool(false), span())),
        ];
        let (k, v) = map_key_value_from_object_literal(&entries).expect("homogeneous bool values");
        assert_eq!(k, ConcreteType::String);
        assert_eq!(v, ConcreteType::Bool);
    }

    #[test]
    fn test_object_literal_heterogeneous_values_returns_none() {
        let entries = vec![
            obj_field("a", Expr::Literal(Literal::Int(1), span())),
            obj_field("b", Expr::Literal(Literal::String("two".into()), span())),
        ];
        assert_eq!(map_key_value_from_object_literal(&entries), None);
    }

    #[test]
    fn test_object_literal_int_and_number_returns_none() {
        // int and number do not unify in Shape's type system.
        let entries = vec![
            obj_field("a", Expr::Literal(Literal::Int(1), span())),
            obj_field("b", Expr::Literal(Literal::Number(2.0), span())),
        ];
        assert_eq!(map_key_value_from_object_literal(&entries), None);
    }

    #[test]
    fn test_object_literal_empty_returns_none() {
        assert_eq!(map_key_value_from_object_literal(&[]), None);
    }

    #[test]
    fn test_object_literal_with_non_literal_returns_none() {
        let entries = vec![
            obj_field("a", Expr::Literal(Literal::Int(1), span())),
            obj_field("b", Expr::Identifier("some_var".to_string(), span())),
        ];
        assert_eq!(map_key_value_from_object_literal(&entries), None);
    }

    #[test]
    fn test_object_literal_with_spread_returns_none() {
        let entries = vec![
            obj_field("a", Expr::Literal(Literal::Int(1), span())),
            ObjectEntry::Spread(Expr::Identifier("other".to_string(), span())),
        ];
        assert_eq!(map_key_value_from_object_literal(&entries), None);
    }

    // ----------------------------------------------------------------
    // concrete_type_from_annotation
    // ----------------------------------------------------------------

    #[test]
    fn test_concrete_type_primitives() {
        assert_eq!(
            concrete_type_from_annotation(&ann_basic("int")),
            Some(ConcreteType::I64)
        );
        assert_eq!(
            concrete_type_from_annotation(&ann_basic("number")),
            Some(ConcreteType::F64)
        );
        assert_eq!(
            concrete_type_from_annotation(&ann_basic("bool")),
            Some(ConcreteType::Bool)
        );
        assert_eq!(
            concrete_type_from_annotation(&ann_basic("string")),
            Some(ConcreteType::String)
        );
        assert_eq!(
            concrete_type_from_annotation(&ann_basic("decimal")),
            Some(ConcreteType::Decimal)
        );
        assert_eq!(
            concrete_type_from_annotation(&ann_basic("u8")),
            Some(ConcreteType::U8)
        );
    }

    #[test]
    fn test_concrete_type_array_of_int() {
        let ann = TypeAnnotation::Array(Box::new(ann_basic("int")));
        assert_eq!(
            concrete_type_from_annotation(&ann),
            Some(ConcreteType::Array(Box::new(ConcreteType::I64)))
        );
    }

    #[test]
    fn test_concrete_type_generic_array_of_number() {
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Array"),
            args: vec![ann_basic("number")],
        };
        assert_eq!(
            concrete_type_from_annotation(&ann),
            Some(ConcreteType::Array(Box::new(ConcreteType::F64)))
        );
    }

    #[test]
    fn test_concrete_type_unknown_returns_none() {
        assert_eq!(concrete_type_from_annotation(&ann_basic("MyStruct")), None);
    }

    // ----------------------------------------------------------------
    // BytecodeCompiler integration: side-table read/write
    // ----------------------------------------------------------------

    fn fresh_compiler() -> BytecodeCompiler {
        BytecodeCompiler::new()
    }

    #[test]
    fn test_record_and_lookup_local_map_kv() {
        let mut compiler = fresh_compiler();
        compiler.record_map_key_value_for_local(7, ConcreteType::String, ConcreteType::I64);
        let (k, v) = compiler.map_key_value_for_local(7).expect("recorded entry");
        assert_eq!(*k, ConcreteType::String);
        assert_eq!(*v, ConcreteType::I64);
        assert_eq!(compiler.map_key_value_for_local(8), None);
    }

    #[test]
    fn test_record_and_lookup_module_binding_map_kv() {
        let mut compiler = fresh_compiler();
        compiler.record_map_key_value_for_module_binding(3, ConcreteType::I64, ConcreteType::F64);
        let (k, v) = compiler
            .map_key_value_for_module_binding(3)
            .expect("recorded entry");
        assert_eq!(*k, ConcreteType::I64);
        assert_eq!(*v, ConcreteType::F64);
    }

    #[test]
    fn test_record_and_lookup_node_map_kv() {
        let mut compiler = fresh_compiler();
        let s = Span::new(10, 20);
        compiler.record_map_key_value_for_node(s, ConcreteType::String, ConcreteType::Bool);
        let (k, v) = compiler.map_key_value_for_node(s).expect("recorded entry");
        assert_eq!(*k, ConcreteType::String);
        assert_eq!(*v, ConcreteType::Bool);
    }

    #[test]
    fn test_try_track_hashmap_binding_from_annotation_local() {
        let mut compiler = fresh_compiler();
        let ann = hashmap_ann(ann_basic("string"), ann_basic("int"));
        let init_span = Span::new(100, 110);
        let tracked =
            compiler.try_track_hashmap_binding_from_annotation(&ann, 5, true, Some(init_span));
        assert!(tracked, "should have tracked HashMap binding");
        let (k, v) = compiler.map_key_value_for_local(5).expect("local recorded");
        assert_eq!(*k, ConcreteType::String);
        assert_eq!(*v, ConcreteType::I64);
        let (k2, v2) = compiler
            .map_key_value_for_node(init_span)
            .expect("node recorded");
        assert_eq!(*k2, ConcreteType::String);
        assert_eq!(*v2, ConcreteType::I64);
    }

    #[test]
    fn test_try_track_hashmap_binding_from_annotation_module() {
        let mut compiler = fresh_compiler();
        let ann = hashmap_ann(ann_basic("int"), ann_basic("number"));
        let tracked = compiler.try_track_hashmap_binding_from_annotation(&ann, 2, false, None);
        assert!(tracked);
        let (k, v) = compiler
            .map_key_value_for_module_binding(2)
            .expect("module binding recorded");
        assert_eq!(*k, ConcreteType::I64);
        assert_eq!(*v, ConcreteType::F64);
    }

    #[test]
    fn test_try_track_hashmap_binding_from_non_hashmap_annotation_returns_false() {
        let mut compiler = fresh_compiler();
        let ann = ann_basic("int");
        let tracked = compiler.try_track_hashmap_binding_from_annotation(&ann, 1, true, None);
        assert!(!tracked);
        assert_eq!(compiler.map_key_value_for_local(1), None);
    }

    #[test]
    fn test_resolve_receiver_map_key_value_from_non_identifier_returns_none() {
        let compiler = fresh_compiler();
        let receiver = Expr::Literal(Literal::Int(0), span());
        assert_eq!(compiler.resolve_receiver_map_key_value(&receiver), None);
    }

    // ----------------------------------------------------------------
    // Phase 2.2 — HashMap method result-type tracking
    //
    // Side-table-only unit tests that mirror what `compile_expr_method_call`
    // does when it sees `m.keys()`, `m.values()`, `m.entries()`, `m.get(k)`,
    // `m.set(k, v)`, etc. for a HashMap with a known K/V pair. We exercise
    // the same `record_*` helpers the dispatch path calls so the side-table
    // shapes are testable without standing up the full compile pipeline.
    // ----------------------------------------------------------------

    #[test]
    fn test_phase22_keys_records_array_of_k() {
        let mut compiler = fresh_compiler();
        let call_span = Span::new(100, 110);
        let k = ConcreteType::String;
        let v = ConcreteType::I64;
        // Mimics compile_expr_method_call's `keys` arm.
        compiler.record_array_element_type(call_span, k.clone());
        compiler.record_map_key_value_for_node(call_span, k.clone(), v);
        let elem = compiler
            .get_array_element_type(call_span)
            .expect("array element type recorded");
        assert_eq!(*elem, ConcreteType::String);
    }

    #[test]
    fn test_phase22_values_records_array_of_v_int() {
        let mut compiler = fresh_compiler();
        let call_span = Span::new(200, 210);
        // HashMap<string, int>: values() -> Array<int>
        let k = ConcreteType::String;
        let v = ConcreteType::I64;
        compiler.record_array_element_type(call_span, v.clone());
        compiler.record_map_key_value_for_node(call_span, k, v);
        let elem = compiler
            .get_array_element_type(call_span)
            .expect("array element type recorded");
        assert_eq!(*elem, ConcreteType::I64);
    }

    #[test]
    fn test_phase22_get_records_kv_metadata_no_array() {
        let mut compiler = fresh_compiler();
        let call_span = Span::new(300, 310);
        // HashMap<string, number>: get("foo") -> Option<number>
        // Per the dispatch arm we still record kv metadata, but no array
        // element type (the get arm is intentionally a no-op for the array
        // side-table — Option<V> isn't an Array<V>).
        let k = ConcreteType::String;
        let v = ConcreteType::F64;
        compiler.record_map_key_value_for_node(call_span, k.clone(), v.clone());
        // No call to record_array_element_type — get() is not array-shaped.
        assert_eq!(compiler.get_array_element_type(call_span), None);
        let (rk, rv) = compiler
            .map_key_value_for_node(call_span)
            .expect("kv metadata recorded");
        assert_eq!(*rk, ConcreteType::String);
        assert_eq!(*rv, ConcreteType::F64);
    }

    #[test]
    fn test_phase22_entries_records_array_of_tuple_kv() {
        let mut compiler = fresh_compiler();
        let call_span = Span::new(400, 410);
        let k = ConcreteType::String;
        let v = ConcreteType::I64;
        let pair = ConcreteType::Tuple(vec![k.clone(), v.clone()]);
        compiler.record_array_element_type(call_span, pair.clone());
        compiler.record_map_key_value_for_node(call_span, k, v);
        let elem = compiler
            .get_array_element_type(call_span)
            .expect("array element type recorded");
        assert_eq!(
            *elem,
            ConcreteType::Tuple(vec![ConcreteType::String, ConcreteType::I64])
        );
    }

    #[test]
    fn test_phase22_set_preserves_kv_metadata() {
        let mut compiler = fresh_compiler();
        let call_span = Span::new(500, 510);
        // set(k, v) -> HashMap<K, V> — type-preserving.
        let k = ConcreteType::I64;
        let v = ConcreteType::String;
        compiler.record_map_key_value_for_node(call_span, k.clone(), v.clone());
        let (rk, rv) = compiler
            .map_key_value_for_node(call_span)
            .expect("kv metadata recorded after set");
        assert_eq!(*rk, ConcreteType::I64);
        assert_eq!(*rv, ConcreteType::String);
        // set() does not produce an array, so the array side-table is empty.
        assert_eq!(compiler.get_array_element_type(call_span), None);
    }

    // ----------------------------------------------------------------
    // Phase 2.2 — String split → Array<String> tracking
    // ----------------------------------------------------------------

    #[test]
    fn test_phase22_split_records_array_of_string() {
        let mut compiler = fresh_compiler();
        let call_span = Span::new(600, 610);
        // Mimics the split arm: receiver is a string, record element=String
        compiler.record_array_element_type(call_span, ConcreteType::String);
        let elem = compiler
            .get_array_element_type(call_span)
            .expect("array element type recorded");
        assert_eq!(*elem, ConcreteType::String);
    }
}
