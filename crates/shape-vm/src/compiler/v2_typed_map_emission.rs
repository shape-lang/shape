//! v2 typed map opcode emission helpers (Phase 3.2).
//!
//! This module is the *gating layer* between the bytecode compiler and the
//! v2 typed map opcode set. Given a key/value [`ConcreteType`] pair for a
//! HashMap value, [`should_use_typed_map`] reports whether the compiler can
//! safely emit one of the typed `NewTypedMap*`/`TypedMap*Get`/`TypedMap*Set`/
//! `TypedMap*Has`/`TypedMap*Delete` opcodes (Phase 3.2).
//!
//! Key/value combinations recognised by this layer:
//!
//! - `HashMap<string, number>`  → `TypedMapStringF64`
//! - `HashMap<string, int>`     → `TypedMapStringI64`
//! - `HashMap<string, T>`       → `TypedMapStringPtr` (T = string, struct, …)
//! - `HashMap<int, number>`     → `TypedMapI64F64`
//! - `HashMap<int, int>`        → `TypedMapI64I64`
//! - `HashMap<int, T>`          → `TypedMapI64Ptr`
//!
//! Anything else (e.g. `HashMap<bool, …>`, `HashMap<i32, …>`, `HashMap<f64, …>`,
//! sized integer keys/values) returns `None`. The compiler is expected to
//! fail soft and emit the legacy NaN-boxed `BuiltinCall(HashMapCtor)` /
//! `CallMethod` opcodes for those cases — Phase 3.2 is intentionally narrow
//! on the typed-fast-path.
//!
//! As more typed map opcodes land this helper will grow more `Some(...)` arms;
//! call sites don't need to change.

use shape_value::v2::ConcreteType;

use crate::bytecode::OpCode;

/// The kind of typed map the compiler should emit for a known key/value pair.
///
/// Each variant corresponds to a `TypedMap<K, V>` instantiation that has a
/// matching set of `NewTypedMap*`/`TypedMap*Get`/`TypedMap*Set`/`TypedMap*Has`/
/// `TypedMap*Delete` opcodes (Phase 3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedMapKind {
    /// `TypedMap<*const StringObj, f64>` — backing for `HashMap<string, number>`.
    StringF64,
    /// `TypedMap<*const StringObj, i64>` — backing for `HashMap<string, int>`.
    StringI64,
    /// `TypedMap<*const StringObj, *const u8>` — backing for
    /// `HashMap<string, T>` for any heap-pointer-shaped V.
    StringPtr,
    /// `TypedMap<i64, f64>` — backing for `HashMap<int, number>`.
    I64F64,
    /// `TypedMap<i64, i64>` — backing for `HashMap<int, int>`.
    I64I64,
    /// `TypedMap<i64, *const u8>` — backing for `HashMap<int, T>` for any
    /// heap-pointer-shaped V.
    I64Ptr,
}

impl TypedMapKind {
    /// The `NewTypedMap*` opcode that allocates this kind of map.
    #[inline]
    pub fn new_opcode(self) -> OpCode {
        match self {
            TypedMapKind::StringF64 => OpCode::NewTypedMapStringF64,
            TypedMapKind::StringI64 => OpCode::NewTypedMapStringI64,
            TypedMapKind::StringPtr => OpCode::NewTypedMapStringPtr,
            TypedMapKind::I64F64 => OpCode::NewTypedMapI64F64,
            TypedMapKind::I64I64 => OpCode::NewTypedMapI64I64,
            TypedMapKind::I64Ptr => OpCode::NewTypedMapI64Ptr,
        }
    }

    /// The `TypedMap*Get` opcode for this kind.
    #[inline]
    pub fn get_opcode(self) -> OpCode {
        match self {
            TypedMapKind::StringF64 => OpCode::TypedMapStringF64Get,
            TypedMapKind::StringI64 => OpCode::TypedMapStringI64Get,
            TypedMapKind::StringPtr => OpCode::TypedMapStringPtrGet,
            TypedMapKind::I64F64 => OpCode::TypedMapI64F64Get,
            TypedMapKind::I64I64 => OpCode::TypedMapI64I64Get,
            TypedMapKind::I64Ptr => OpCode::TypedMapI64PtrGet,
        }
    }

    /// The `TypedMap*Set` opcode for this kind.
    #[inline]
    pub fn set_opcode(self) -> OpCode {
        match self {
            TypedMapKind::StringF64 => OpCode::TypedMapStringF64Set,
            TypedMapKind::StringI64 => OpCode::TypedMapStringI64Set,
            TypedMapKind::StringPtr => OpCode::TypedMapStringPtrSet,
            TypedMapKind::I64F64 => OpCode::TypedMapI64F64Set,
            TypedMapKind::I64I64 => OpCode::TypedMapI64I64Set,
            TypedMapKind::I64Ptr => OpCode::TypedMapI64PtrSet,
        }
    }

    /// The `TypedMap*Has` opcode for this kind.
    #[inline]
    pub fn has_opcode(self) -> OpCode {
        match self {
            TypedMapKind::StringF64 => OpCode::TypedMapStringF64Has,
            TypedMapKind::StringI64 => OpCode::TypedMapStringI64Has,
            TypedMapKind::StringPtr => OpCode::TypedMapStringPtrHas,
            TypedMapKind::I64F64 => OpCode::TypedMapI64F64Has,
            TypedMapKind::I64I64 => OpCode::TypedMapI64I64Has,
            TypedMapKind::I64Ptr => OpCode::TypedMapI64PtrHas,
        }
    }

    /// The `TypedMap*Delete` opcode for this kind.
    #[inline]
    pub fn delete_opcode(self) -> OpCode {
        match self {
            TypedMapKind::StringF64 => OpCode::TypedMapStringF64Delete,
            TypedMapKind::StringI64 => OpCode::TypedMapStringI64Delete,
            TypedMapKind::StringPtr => OpCode::TypedMapStringPtrDelete,
            TypedMapKind::I64F64 => OpCode::TypedMapI64F64Delete,
            TypedMapKind::I64I64 => OpCode::TypedMapI64I64Delete,
            TypedMapKind::I64Ptr => OpCode::TypedMapI64PtrDelete,
        }
    }
}

/// Whether a `ConcreteType` value can fit through the `*const u8` (pointer)
/// value slot of a `TypedMap*Ptr` opcode. Only heap-pointer-shaped types map
/// safely; primitive scalars must take a typed value-shaped opcode.
#[inline]
fn value_fits_ptr_slot(v: &ConcreteType) -> bool {
    matches!(
        v,
        ConcreteType::String
            | ConcreteType::Struct(_)
            | ConcreteType::Array(_)
            | ConcreteType::HashMap(_, _)
            | ConcreteType::Enum(_)
            | ConcreteType::Closure(_)
            | ConcreteType::Pointer(_)
            | ConcreteType::BigInt
            | ConcreteType::Decimal
            | ConcreteType::DateTime
    )
}

/// Map a key/value `ConcreteType` pair to a `TypedMapKind`, if a typed-map
/// fast path exists for that combination.
///
/// Returns `None` for combinations that have no typed map opcode yet
/// (bool keys, sized integer keys/values like i32/u8, mixed scalar values).
/// Callers must fall back to the legacy NaN-boxed `BuiltinCall(HashMapCtor)`
/// allocation and `CallMethod` dispatch in that case.
///
/// **Important**: this function is the *single source of truth* for the
/// "do we have a typed-map opcode for this K/V pair?" question used by
/// `compile_expr_function_call` (HashMap constructor) and HashMap method
/// dispatch. Adding a new typed K/V pair (say `TypedMapKind::I32F64`) is a
/// one-line change here plus new opcodes — call sites pick it up automatically.
#[inline]
pub fn should_use_typed_map(k: &ConcreteType, v: &ConcreteType) -> Option<TypedMapKind> {
    match (k, v) {
        // String-keyed combos.
        (ConcreteType::String, ConcreteType::F64) => Some(TypedMapKind::StringF64),
        (ConcreteType::String, ConcreteType::I64) => Some(TypedMapKind::StringI64),
        (ConcreteType::String, v) if value_fits_ptr_slot(v) => Some(TypedMapKind::StringPtr),

        // i64-keyed combos.
        (ConcreteType::I64, ConcreteType::F64) => Some(TypedMapKind::I64F64),
        (ConcreteType::I64, ConcreteType::I64) => Some(TypedMapKind::I64I64),
        (ConcreteType::I64, v) if value_fits_ptr_slot(v) => Some(TypedMapKind::I64Ptr),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::v2::concrete_type::{EnumLayoutId, StructLayoutId};

    #[test]
    fn test_string_f64_maps_to_string_f64() {
        assert_eq!(
            should_use_typed_map(&ConcreteType::String, &ConcreteType::F64),
            Some(TypedMapKind::StringF64)
        );
    }

    #[test]
    fn test_string_i64_maps_to_string_i64() {
        assert_eq!(
            should_use_typed_map(&ConcreteType::String, &ConcreteType::I64),
            Some(TypedMapKind::StringI64)
        );
    }

    #[test]
    fn test_string_string_maps_to_string_ptr() {
        assert_eq!(
            should_use_typed_map(&ConcreteType::String, &ConcreteType::String),
            Some(TypedMapKind::StringPtr)
        );
    }

    #[test]
    fn test_string_struct_maps_to_string_ptr() {
        assert_eq!(
            should_use_typed_map(
                &ConcreteType::String,
                &ConcreteType::Struct(StructLayoutId(0))
            ),
            Some(TypedMapKind::StringPtr)
        );
    }

    #[test]
    fn test_int_int_maps_to_i64_i64() {
        assert_eq!(
            should_use_typed_map(&ConcreteType::I64, &ConcreteType::I64),
            Some(TypedMapKind::I64I64)
        );
    }

    #[test]
    fn test_int_number_maps_to_i64_f64() {
        assert_eq!(
            should_use_typed_map(&ConcreteType::I64, &ConcreteType::F64),
            Some(TypedMapKind::I64F64)
        );
    }

    #[test]
    fn test_int_string_maps_to_i64_ptr() {
        assert_eq!(
            should_use_typed_map(&ConcreteType::I64, &ConcreteType::String),
            Some(TypedMapKind::I64Ptr)
        );
    }

    #[test]
    fn test_int_enum_maps_to_i64_ptr() {
        assert_eq!(
            should_use_typed_map(&ConcreteType::I64, &ConcreteType::Enum(EnumLayoutId(0))),
            Some(TypedMapKind::I64Ptr)
        );
    }

    #[test]
    fn test_int_array_maps_to_i64_ptr() {
        assert_eq!(
            should_use_typed_map(
                &ConcreteType::I64,
                &ConcreteType::Array(Box::new(ConcreteType::F64))
            ),
            Some(TypedMapKind::I64Ptr)
        );
    }

    #[test]
    fn test_string_bool_falls_back() {
        // bool is a scalar but we don't have a typed map opcode for it.
        assert_eq!(
            should_use_typed_map(&ConcreteType::String, &ConcreteType::Bool),
            None
        );
    }

    #[test]
    fn test_bool_keyed_falls_back() {
        // bool keys are not supported.
        assert_eq!(
            should_use_typed_map(&ConcreteType::Bool, &ConcreteType::I64),
            None
        );
    }

    #[test]
    fn test_i32_keyed_falls_back() {
        // i32 keys would need their own opcode family.
        assert_eq!(
            should_use_typed_map(&ConcreteType::I32, &ConcreteType::F64),
            None
        );
    }

    #[test]
    fn test_string_i32_falls_back() {
        // sized-int values are not yet wired up to typed map opcodes.
        assert_eq!(
            should_use_typed_map(&ConcreteType::String, &ConcreteType::I32),
            None
        );
    }

    #[test]
    fn test_string_u8_falls_back() {
        assert_eq!(
            should_use_typed_map(&ConcreteType::String, &ConcreteType::U8),
            None
        );
    }

    #[test]
    fn test_opcode_lookup_round_trip() {
        // Sanity check that all kinds expose all five opcodes.
        for kind in [
            TypedMapKind::StringF64,
            TypedMapKind::StringI64,
            TypedMapKind::StringPtr,
            TypedMapKind::I64F64,
            TypedMapKind::I64I64,
            TypedMapKind::I64Ptr,
        ] {
            let _ = kind.new_opcode();
            let _ = kind.get_opcode();
            let _ = kind.set_opcode();
            let _ = kind.has_opcode();
            let _ = kind.delete_opcode();
        }
    }
}

// ──────────────────────────────────────────────────────────────────────
// End-to-end compiler integration tests (Phase 3.2)
//
// These verify that the compiler emits the expected typed map opcodes
// when compiling Shape programs that use HashMap with known K/V types,
// and falls back to the legacy `BuiltinCall(HashMapCtor)` /
// `CallMethod` path when types can't be resolved.
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod compile_integration_tests {
    use super::*;
    use crate::bytecode::{BytecodeProgram, OpCode, Operand};
    use crate::compiler::BytecodeCompiler;

    fn compile(src: &str) -> BytecodeProgram {
        let program = shape_ast::parser::parse_program(src).expect("parse should succeed");
        BytecodeCompiler::new()
            .compile_with_source(&program, src)
            .expect("compile should succeed")
    }

    fn has_opcode(prog: &BytecodeProgram, op: OpCode) -> bool {
        prog.instructions.iter().any(|i| i.opcode == op)
    }

    fn count_opcode(prog: &BytecodeProgram, op: OpCode) -> usize {
        prog.instructions.iter().filter(|i| i.opcode == op).count()
    }

    #[test]
    fn test_let_hashmap_string_number_emits_typed_new() {
        // `let m: HashMap<string, number> = HashMap()` → NewTypedMapStringF64
        let prog = compile(
            r#"
            let m: HashMap<string, number> = HashMap()
            m
        "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedMapStringF64),
            "expected NewTypedMapStringF64 in instruction stream"
        );
        // Legacy ctor should NOT be emitted on the typed-map fast path.
        // (Validate by checking that the BuiltinCall(HashMapCtor) is absent.)
        let legacy_ctor = prog.instructions.iter().any(|i| {
            matches!(
                (i.opcode, i.operand),
                (
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(
                        crate::bytecode::BuiltinFunction::HashMapCtor
                    ))
                )
            )
        });
        assert!(
            !legacy_ctor,
            "should not emit legacy BuiltinCall(HashMapCtor) when typed-map path applies"
        );
    }

    #[test]
    fn test_let_hashmap_string_int_emits_typed_new() {
        let prog = compile(
            r#"
            let m: HashMap<string, int> = HashMap()
            m
        "#,
        );
        assert!(has_opcode(&prog, OpCode::NewTypedMapStringI64));
    }

    #[test]
    fn test_let_hashmap_int_string_emits_i64_ptr_new() {
        let prog = compile(
            r#"
            let m: HashMap<int, string> = HashMap()
            m
        "#,
        );
        assert!(has_opcode(&prog, OpCode::NewTypedMapI64Ptr));
    }

    #[test]
    fn test_hashmap_set_get_emits_typed_ops() {
        // Verify both NewTypedMap*, TypedMap*Set and TypedMap*Get are emitted.
        let prog = compile(
            r#"
            let m: HashMap<string, number> = HashMap()
            m.set("foo", 1.0)
            m.get("foo")
        "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedMapStringF64),
            "expected NewTypedMapStringF64"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedMapStringF64Set),
            "expected TypedMapStringF64Set"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedMapStringF64Get),
            "expected TypedMapStringF64Get"
        );
    }

    #[test]
    fn test_hashmap_int_string_set_emits_i64_ptr_set() {
        let prog = compile(
            r#"
            let m: HashMap<int, string> = HashMap()
            m.set(1, "x")
            m
        "#,
        );
        assert!(has_opcode(&prog, OpCode::NewTypedMapI64Ptr));
        assert!(has_opcode(&prog, OpCode::TypedMapI64PtrSet));
    }

    #[test]
    fn test_hashmap_has_emits_typed_has() {
        let prog = compile(
            r#"
            let m: HashMap<string, int> = HashMap()
            m.set("a", 1)
            m.has("a")
        "#,
        );
        assert!(has_opcode(&prog, OpCode::TypedMapStringI64Has));
    }

    #[test]
    fn test_hashmap_delete_emits_typed_delete() {
        let prog = compile(
            r#"
            let m: HashMap<string, int> = HashMap()
            m.set("a", 1)
            m.delete("a")
            m
        "#,
        );
        assert!(has_opcode(&prog, OpCode::TypedMapStringI64Delete));
    }

    #[test]
    fn test_unannotated_hashmap_uses_legacy_ctor() {
        // No annotation → no inference path → legacy BuiltinCall(HashMapCtor).
        // Note: the linter may auto-add annotations, so we keep this test
        // intentionally minimal.
        let prog = compile(
            r#"
            let m = HashMap()
            m
        "#,
        );
        // Legacy ctor should be present.
        let legacy_ctor = prog.instructions.iter().any(|i| {
            matches!(
                (i.opcode, i.operand),
                (
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(
                        crate::bytecode::BuiltinFunction::HashMapCtor
                    ))
                )
            )
        });
        assert!(
            legacy_ctor,
            "untyped HashMap() should emit legacy BuiltinCall(HashMapCtor)"
        );
        // No typed-map opcode should be present.
        assert!(!has_opcode(&prog, OpCode::NewTypedMapStringF64));
        assert!(!has_opcode(&prog, OpCode::NewTypedMapStringI64));
        assert!(!has_opcode(&prog, OpCode::NewTypedMapI64F64));
        assert!(!has_opcode(&prog, OpCode::NewTypedMapI64I64));
        assert!(!has_opcode(&prog, OpCode::NewTypedMapI64Ptr));
    }

    #[test]
    fn test_hashmap_string_bool_falls_back_to_legacy() {
        // bool values aren't on the typed-map fast path, so we expect the
        // legacy ctor + CallMethod dispatch.
        let prog = compile(
            r#"
            let m: HashMap<string, bool> = HashMap()
            m.set("a", true)
            m
        "#,
        );
        // No typed-map ctor for string→bool.
        assert!(!has_opcode(&prog, OpCode::NewTypedMapStringF64));
        assert!(!has_opcode(&prog, OpCode::NewTypedMapStringI64));
        // Legacy ctor + CallMethod path used instead.
        let legacy_ctor = prog.instructions.iter().any(|i| {
            matches!(
                (i.opcode, i.operand),
                (
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(
                        crate::bytecode::BuiltinFunction::HashMapCtor
                    ))
                )
            )
        });
        assert!(
            legacy_ctor,
            "string→bool HashMap should fall back to legacy ctor"
        );
    }

    #[test]
    fn test_hashmap_int_int_emits_i64_i64_ops() {
        let prog = compile(
            r#"
            let m: HashMap<int, int> = HashMap()
            m.set(42, 99)
            m.get(42)
        "#,
        );
        assert!(has_opcode(&prog, OpCode::NewTypedMapI64I64));
        assert!(has_opcode(&prog, OpCode::TypedMapI64I64Set));
        assert!(has_opcode(&prog, OpCode::TypedMapI64I64Get));
    }

    #[test]
    fn test_hashmap_int_number_emits_i64_f64_ops() {
        let prog = compile(
            r#"
            let m: HashMap<int, number> = HashMap()
            m.set(1, 3.14)
            m
        "#,
        );
        assert!(has_opcode(&prog, OpCode::NewTypedMapI64F64));
        assert!(has_opcode(&prog, OpCode::TypedMapI64F64Set));
    }

    #[test]
    fn test_typed_new_count_matches() {
        // Single binding → single typed allocation, no duplicate.
        let prog = compile(
            r#"
            let m: HashMap<string, int> = HashMap()
            m
        "#,
        );
        assert_eq!(count_opcode(&prog, OpCode::NewTypedMapStringI64), 1);
    }
}
