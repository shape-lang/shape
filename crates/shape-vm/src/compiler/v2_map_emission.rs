//! Typed map (HashMap) annotation helpers for the v2 runtime.
//!
//! Whole-binding `ConcreteType::HashMap(k, v)` is the source of truth for typed
//! map key/value information. This module keeps the annotation-to-ConcreteType
//! conversion helper used by statement, reference-model, and type-resolution
//! paths.

use shape_ast::ast::TypeAnnotation;
use shape_value::v2::ConcreteType;

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
            // V3-S6a resolver-extension follow-up: `Vec<T>` aliases
            // `Array<T>` in source spelling — the stdlib uses `Vec<U>` in
            // `method map<U>(...) -> Vec<U>`. Without this arm the
            // post-substitution `return_type = Vec<int>` failed
            // `concrete_type_from_annotation` and the JIT's
            // function_return_concrete_types[map_specialization] stayed
            // Void, propagating SURFACE through downstream call sites.
            "Array" | "Vec" if args.len() == 1 => {
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
        // W15.2-LANG-4 jit-filter-predicate fix (2026-05-18). Function
        // and Closure parameters resolve to the opaque
        // `ConcreteType::Function(FunctionTypeId(0))` shape, mirroring
        // the convention used by `resolve_call_site_type_args*` for
        // closure-shaped argument types
        // (`monomorphization/type_resolution.rs:1758/1788`). The JIT-side
        // `native_kind_from_concrete_type` maps this to
        // `Ptr(HeapKind::Closure)` per ADR-006 §2.7.11 / Q12 closure-
        // callee carrier kind; downstream `jit_call_value`'s closure
        // dispatch arm consumes the raw-Arc
        // `Arc<HeapValue::ClosureRaw>` callee bits without falling
        // through to the UInt64 carrier-kind path.
        TypeAnnotation::Function { .. } => Some(ConcreteType::Function(
            shape_value::v2::concrete_type::FunctionTypeId(0),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::ast::type_path::TypePath;

    fn ann_basic(name: &str) -> TypeAnnotation {
        TypeAnnotation::Basic(name.to_string())
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

    // ── v0.3 WS-6b GAP B — typed-map fast path for non-identifier
    //    receivers + function-local typed-map registration ───────────────
    //
    // Two pre-fix failures, both surfacing as
    // `no method 'set'/'get' on receiver kind UInt64`:
    //
    //   1. A function-LOCAL `let m: HashMap<K,V> = HashMap()` allocated a
    //      `NewTypedMap*` carrier but its slot was never registered in
    //      `v2_typed_map_locals` (only the module-binding mirror existed),
    //      so `m.set` / `m.get` fell through to generic `CallMethod`
    //      dispatch — which sees the typed-map pointer's `NativeKind::UInt64`
    //      carrier tag and routes to `NUMBER_METHODS`.
    //   2. The RESULT of a function call returning `HashMap<K,V>` (e.g.
    //      `id(m)`) is a typed-map pointer, but the typed-map fast path was
    //      gated on identifier receivers only.
    //
    // Fix: register `v2_typed_map_locals` at the local let-binding site, and
    // extend `resolve_receiver_typed_map_kind` to recognise a `FunctionCall`
    // / `MethodCall` receiver whose statically-resolved return type is a
    // typed-map `HashMap<K,V>`.

    #[test]
    fn ws6b_typed_map_local_in_function_set_get() {
        // Function-local HashMap: `m.set` / `m.get` on a local binding.
        // U3 (SB-9 deletion): `get` honestly returns `Option<int>`, so the
        // `-> int` return position unwraps via `?? 0` (the deleted TypedMap
        // path returned a bare value with the `(0,Bool)` None sentinel).
        assert_eq!(
            crate::test_utils::eval_typed_i64(
                "fn run() -> int {\n\
                 let mut m: HashMap<string,int> = HashMap()\n\
                 m.set(\"k\", 1)\n\
                 m.get(\"k\") ?? 0\n\
                 }\n\
                 run()"
            ),
            1
        );
    }

    #[test]
    fn ws6b_typed_map_generic_call_result_get() {
        // GAP B canonical reproducer: `.get` on the result of a generic
        // free-function call returning `HashMap<string,int>`.
        assert_eq!(
            crate::test_utils::eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 let mut m: HashMap<string,int> = HashMap()\n\
                 m.set(\"k\", 1)\n\
                 id(m).get(\"k\")"
            ),
            1
        );
    }

    #[test]
    fn ws6b_typed_map_nongeneric_call_result_get() {
        // The same shape with a non-generic constructor function returning
        // a typed map — the typed-map fast path keys off the statically
        // resolved return type, not on genericity.
        assert_eq!(
            crate::test_utils::eval_typed_i64(
                "fn mk() -> HashMap<string,int> {\n\
                 let mut m: HashMap<string,int> = HashMap()\n\
                 m.set(\"k\", 1)\n\
                 m\n\
                 }\n\
                 mk().get(\"k\")"
            ),
            1
        );
    }

    #[test]
    fn ws6b_typed_map_call_result_fluent_set_chain() {
        // `set` on a call result returns the map for fluent chaining — the
        // non-identifier receiver is spilled into a temp so the call is
        // evaluated exactly once.
        assert_eq!(
            crate::test_utils::eval_typed_i64(
                "fn id<T>(x: T) -> T { x }\n\
                 let mut m: HashMap<string,int> = HashMap()\n\
                 id(m).set(\"a\", 5).get(\"a\")"
            ),
            5
        );
    }

    // ── D3 (S4): empty/typed HashMap `len` / `isEmpty` ────────────────────
    // A v2 typed-map carrier (raw `*const TypedMap*`, `NativeKind::UInt64`)
    // cannot dispatch `len`/`isEmpty` through the generic `CallMethod` path
    // (its kind is `UInt64`, not `Ptr(HashMap)`). The stack-based
    // `TypedMapLenStack` opcode reads the K/V-independent `len` field.

    #[test]
    fn s4_d3_empty_typed_map_len_is_zero() {
        assert_eq!(
            crate::test_utils::eval_typed_i64(
                "fn run() -> int {\n\
                 let m: HashMap<string,int> = HashMap()\n\
                 m.len()\n\
                 }\n\
                 run()"
            ),
            0
        );
    }

    #[test]
    fn s4_d3_empty_typed_map_is_empty_true() {
        assert!(crate::test_utils::eval_typed_bool(
            "fn run() -> bool {\n\
             let m: HashMap<string,int> = HashMap()\n\
             m.isEmpty()\n\
             }\n\
             run()"
        ));
    }

    #[test]
    fn s4_d3_populated_typed_map_len_and_is_empty() {
        assert_eq!(
            crate::test_utils::eval_typed_i64(
                "fn run() -> int {\n\
                 let mut m: HashMap<string,int> = HashMap()\n\
                 m.set(\"a\", 1)\n\
                 m.set(\"b\", 2)\n\
                 m.len()\n\
                 }\n\
                 run()"
            ),
            2
        );
        assert!(!crate::test_utils::eval_typed_bool(
            "fn run() -> bool {\n\
             let mut m: HashMap<string,int> = HashMap()\n\
             m.set(\"a\", 1)\n\
             m.isEmpty()\n\
             }\n\
             run()"
        ));
    }
}
