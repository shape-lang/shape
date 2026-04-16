//! Typed array opcode emission helpers.
//!
//! This module is the *gating layer* between the bytecode compiler and the
//! typed array opcode set. Given a `ConcreteType` for an array's element
//! type, [`should_use_typed_array`] reports whether the compiler can safely
//! emit one of the typed `NewTypedArray*`/`TypedArrayPush*`/`TypedArrayGet*`
//! opcodes.
//!
//! Element types this layer recognises:
//!
//! - `f64` (number)
//! - `i64` (int)
//! - `i32`
//! - `bool`
//!
//! Anything else (heap types like `string`, structs, arrays of arrays, sized
//! ints we don't yet have opcodes for, etc.) returns `None`. The compiler is
//! expected to fail soft and emit the legacy NaN-boxed `NewArray` opcode for
//! those cases — the typed-fast-path is intentionally narrow.
//!
//! As more typed array opcodes land (`u8`, `i16`, etc.) this helper will grow
//! more `Some(...)` arms; callers don't need to change.

use shape_value::native::ConcreteType;

use crate::bytecode::OpCode;

/// The kind of typed array the compiler should emit for a known element type.
///
/// Each variant corresponds to a `TypedArray<T>` instantiation that has a
/// matching set of `NewTypedArray*`/`TypedArrayGet*`/`TypedArrayPush*`/
/// `TypedArraySet*` opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedArrayKind {
    /// `TypedArray<f64>` — backing for `Array<number>`.
    F64,
    /// `TypedArray<i64>` — backing for `Array<int>`.
    I64,
    /// `TypedArray<i32>` — backing for `Array<i32>`.
    I32,
    /// `TypedArray<bool>` — backing for `Array<bool>`.
    Bool,
}

impl TypedArrayKind {
    /// The `NewTypedArray*` opcode that allocates this kind of array.
    #[inline]
    pub fn new_opcode(self) -> OpCode {
        match self {
            TypedArrayKind::F64 => OpCode::NewTypedArrayF64,
            TypedArrayKind::I64 => OpCode::NewTypedArrayI64,
            TypedArrayKind::I32 => OpCode::NewTypedArrayI32,
            TypedArrayKind::Bool => OpCode::NewTypedArrayBool,
        }
    }

    /// The `TypedArrayGet*` opcode for this kind.
    #[inline]
    pub fn get_opcode(self) -> OpCode {
        match self {
            TypedArrayKind::F64 => OpCode::TypedArrayGetF64,
            TypedArrayKind::I64 => OpCode::TypedArrayGetI64,
            TypedArrayKind::I32 => OpCode::TypedArrayGetI32,
            TypedArrayKind::Bool => OpCode::TypedArrayGetBool,
        }
    }

    /// The `TypedArrayPush*` opcode for this kind.
    #[inline]
    pub fn push_opcode(self) -> OpCode {
        match self {
            TypedArrayKind::F64 => OpCode::TypedArrayPushF64,
            TypedArrayKind::I64 => OpCode::TypedArrayPushI64,
            TypedArrayKind::I32 => OpCode::TypedArrayPushI32,
            TypedArrayKind::Bool => OpCode::TypedArrayPushBool,
        }
    }

    /// The `TypedArraySet*` opcode for this kind.
    #[inline]
    pub fn set_opcode(self) -> OpCode {
        match self {
            TypedArrayKind::F64 => OpCode::TypedArraySetF64,
            TypedArrayKind::I64 => OpCode::TypedArraySetI64,
            TypedArrayKind::I32 => OpCode::TypedArraySetI32,
            TypedArrayKind::Bool => OpCode::TypedArraySetBool,
        }
    }
}

/// Map a `ConcreteType` element type to a `TypedArrayKind`, if a typed-array
/// fast path exists for that element type.
///
/// Returns `None` for element types that have no typed array opcode yet
/// (heap types like `string`/`struct`/nested arrays, sized integer widths
/// like `i8`/`u16`/etc.). Callers must fall back to the legacy NaN-boxed
/// `NewArray` opcode in that case.
///
/// **Important**: this function is the *single source of truth* for the
/// "do we have a typed-array opcode for this element type?"
#[inline]
pub fn should_use_typed_array(elem_type: &ConcreteType) -> Option<TypedArrayKind> {
    match elem_type {
        ConcreteType::F64 => Some(TypedArrayKind::F64),
        ConcreteType::I64 => Some(TypedArrayKind::I64),
        ConcreteType::I32 => Some(TypedArrayKind::I32),
        ConcreteType::Bool => Some(TypedArrayKind::Bool),
        _ => None,
    }
}

/// `SlotKind` analogue.
///
/// Provided as a bridge for compiler call sites that haven't yet been
/// converted to use `ConcreteType`. The element-type inference may return
/// `SlotKind`, so `compile_expr_array` calls this variant to look up a
/// typed array kind directly.
///
/// The mapping mirrors [`should_use_typed_array`]: only the four element
/// types backed by typed array opcodes today (`Float64`/`Int64`/`Int32`/
/// `Bool`) return `Some`. Anything else (`String`, sized ints other than
/// i32/i64, nullable variants, `Dynamic`/`Unknown`) falls back to the
/// legacy NaN-boxed `NewArray` path.
#[inline]
pub fn should_use_typed_array_from_slot_kind(
    slot: crate::type_tracking::SlotKind,
) -> Option<TypedArrayKind> {
    use crate::type_tracking::SlotKind;
    match slot {
        SlotKind::Float64 => Some(TypedArrayKind::F64),
        SlotKind::Int64 => Some(TypedArrayKind::I64),
        SlotKind::Int32 => Some(TypedArrayKind::I32),
        SlotKind::Bool => Some(TypedArrayKind::Bool),
        _ => None,
    }
}

/// Map a tracked type name like `"Vec<int>"` / `"Array<number>"` to a [`TypedArrayKind`].
#[inline]
#[allow(dead_code)]
pub fn typed_array_kind_from_type_name(type_name: &str) -> Option<TypedArrayKind> {
    let trimmed = type_name.trim();
    let inner = trimmed
        .strip_prefix("Vec<")
        .or_else(|| trimmed.strip_prefix("Array<"))?
        .strip_suffix('>')?;
    match inner.trim() {
        "number" | "f64" => Some(TypedArrayKind::F64),
        "int" | "i64" => Some(TypedArrayKind::I64),
        "i32" => Some(TypedArrayKind::I32),
        "bool" => Some(TypedArrayKind::Bool),
        _ => None,
    }
}

impl super::BytecodeCompiler {
    /// Resolve an array receiver expression (`Identifier(name)`) to a
    /// [`TypedArrayKind`], if the receiver is a tracked array whose element
    /// type has a typed-array fast path.
    ///
    /// Walks the receiver name through:
    ///   1. Local slot typed-array-locals entry.
    ///   2. Module binding typed-array-module-bindings entry.
    ///
    /// Returns `None` for non-identifier receivers, for unresolved names, for
    /// receivers tracked as something other than a homogeneous typed array,
    /// and for element types that have no typed opcode kind today (`string`,
    /// sized ints other than `i32`/`i64`, etc). The caller is expected to
    /// fall back to the legacy NaN-boxed path in those cases.
    ///
    /// Used by `compile_expr_method_call`, `compile_expr_index_access`, and
    /// `compile_expr_assign` to gate typed array opcode emission for
    /// `arr.push(x)`, `arr.pop()`, `arr.length`, `arr[i]`, and `arr[i] = x`.
    pub(crate) fn resolve_receiver_typed_array_kind(
        &self,
        receiver: &shape_ast::ast::Expr,
    ) -> Option<TypedArrayKind> {
        let name = match receiver {
            shape_ast::ast::Expr::Identifier(name, _) => name,
            _ => return None,
        };

        // Local slot first — ONLY if the slot was actually allocated as a
        // native typed array via `compile_expr_array`'s typed path. We CANNOT
        // simply trust the type-tracker name here because legacy untyped
        // literals (`let mut a = [1, 2, 3]`) get a `Vec<int>` type-tracker
        // entry too, but the runtime value is a NaN-boxed VMArray, not a
        // `*const TypedArray<i64>`. Emitting a typed get/set for that
        // would corrupt memory.
        if let Some(local_idx) = self.resolve_local(name) {
            if let Some(&kind) = self.typed_array_locals.get(&local_idx) {
                return Some(kind);
            }
            return None;
        }

        // Module binding fallback (same restriction).
        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
            if let Some(&kind) = self.typed_array_module_bindings.get(&binding_idx) {
                return Some(kind);
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::native::concrete_type::{EnumLayoutId, StructLayoutId};

    #[test]
    fn test_f64_maps_to_typed_array_f64() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::F64),
            Some(TypedArrayKind::F64)
        );
    }

    #[test]
    fn test_i64_maps_to_typed_array_i64() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::I64),
            Some(TypedArrayKind::I64)
        );
    }

    #[test]
    fn test_i32_maps_to_typed_array_i32() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::I32),
            Some(TypedArrayKind::I32)
        );
    }

    #[test]
    fn test_bool_maps_to_typed_array_bool() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::Bool),
            Some(TypedArrayKind::Bool)
        );
    }

    #[test]
    fn test_string_falls_back_to_legacy() {
        assert_eq!(should_use_typed_array(&ConcreteType::String), None);
    }

    #[test]
    fn test_struct_falls_back_to_legacy() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::Struct(StructLayoutId(0))),
            None
        );
    }

    #[test]
    fn test_enum_falls_back_to_legacy() {
        assert_eq!(
            should_use_typed_array(&ConcreteType::Enum(EnumLayoutId(0))),
            None
        );
    }

    #[test]
    fn test_nested_array_falls_back_to_legacy() {
        // Array<Array<int>> — element type is Array<int>, not yet handled
        // by typed opcodes (would need TypedArray<*const TypedArray<i64>>).
        let nested = ConcreteType::Array(Box::new(ConcreteType::I64));
        assert_eq!(should_use_typed_array(&nested), None);
    }

    #[test]
    fn test_u8_falls_back_to_legacy() {
        // Sized ints other than i32/i64 don't yet have typed opcodes.
        assert_eq!(should_use_typed_array(&ConcreteType::U8), None);
    }

    #[test]
    fn test_option_falls_back_to_legacy() {
        let opt = ConcreteType::Option(Box::new(ConcreteType::I64));
        assert_eq!(should_use_typed_array(&opt), None);
    }

    #[test]
    fn test_opcode_lookup_round_trip() {
        // Sanity check that all four kinds expose all four opcodes.
        for kind in [
            TypedArrayKind::F64,
            TypedArrayKind::I64,
            TypedArrayKind::I32,
            TypedArrayKind::Bool,
        ] {
            let _ = kind.new_opcode();
            let _ = kind.get_opcode();
            let _ = kind.push_opcode();
            let _ = kind.set_opcode();
        }
    }

    // ---- SlotKind variant ----

    #[test]
    fn test_slot_kind_float64_maps_to_f64() {
        use crate::type_tracking::SlotKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(SlotKind::Float64),
            Some(TypedArrayKind::F64)
        );
    }

    #[test]
    fn test_slot_kind_int64_maps_to_i64() {
        use crate::type_tracking::SlotKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(SlotKind::Int64),
            Some(TypedArrayKind::I64)
        );
    }

    #[test]
    fn test_slot_kind_int32_maps_to_i32() {
        use crate::type_tracking::SlotKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(SlotKind::Int32),
            Some(TypedArrayKind::I32)
        );
    }

    #[test]
    fn test_slot_kind_bool_maps_to_bool() {
        use crate::type_tracking::SlotKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(SlotKind::Bool),
            Some(TypedArrayKind::Bool)
        );
    }

    #[test]
    fn test_slot_kind_string_falls_back() {
        use crate::type_tracking::SlotKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(SlotKind::String),
            None
        );
    }

    #[test]
    fn test_slot_kind_unknown_falls_back() {
        use crate::type_tracking::SlotKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(SlotKind::Unknown),
            None
        );
    }

    #[test]
    fn test_slot_kind_dynamic_falls_back() {
        use crate::type_tracking::SlotKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(SlotKind::Dynamic),
            None
        );
    }

    #[test]
    fn test_slot_kind_int8_falls_back() {
        // Sized ints we don't have typed opcodes for fall back to legacy.
        use crate::type_tracking::SlotKind;
        assert_eq!(should_use_typed_array_from_slot_kind(SlotKind::Int8), None);
    }

    // ---- typed_array_kind_from_type_name ----

    #[test]
    fn test_type_name_vec_int_maps_to_i64() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<int>"),
            Some(TypedArrayKind::I64)
        );
    }

    #[test]
    fn test_type_name_vec_number_maps_to_f64() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<number>"),
            Some(TypedArrayKind::F64)
        );
    }

    #[test]
    fn test_type_name_vec_bool_maps_to_bool() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<bool>"),
            Some(TypedArrayKind::Bool)
        );
    }

    #[test]
    fn test_type_name_vec_i32_maps_to_i32() {
        assert_eq!(
            typed_array_kind_from_type_name("Vec<i32>"),
            Some(TypedArrayKind::I32)
        );
    }

    #[test]
    fn test_type_name_array_int_maps_to_i64() {
        assert_eq!(
            typed_array_kind_from_type_name("Array<int>"),
            Some(TypedArrayKind::I64)
        );
    }

    #[test]
    fn test_type_name_vec_string_falls_back() {
        assert_eq!(typed_array_kind_from_type_name("Vec<string>"), None);
    }

    #[test]
    fn test_type_name_non_array_falls_back() {
        assert_eq!(typed_array_kind_from_type_name("HashMap<int, int>"), None);
        assert_eq!(typed_array_kind_from_type_name("int"), None);
    }

    // ---- Compiler integration: typed opcode emission for array literals ----

    /// Helper: compile source and return the list of opcodes in the program.
    fn compiled_opcodes(source: &str) -> Vec<crate::bytecode::OpCode> {
        let program = shape_ast::parser::parse_program(source).expect("parse failed");
        let compiler = crate::compiler::BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile failed");
        bytecode
            .instructions
            .iter()
            .map(|i| i.opcode)
            .collect()
    }

    #[test]
    fn test_float_array_emits_typed_opcodes() {
        let ops = compiled_opcodes("[1.0, 2.0, 3.0]");
        assert!(
            ops.contains(&crate::bytecode::OpCode::NewTypedArrayF64),
            "expected NewTypedArrayF64, got opcodes: {:?}",
            ops
        );
        let push_count = ops
            .iter()
            .filter(|&&op| op == crate::bytecode::OpCode::TypedArrayPushF64)
            .count();
        assert_eq!(
            push_count, 3,
            "expected 3 TypedArrayPushF64, got {}",
            push_count
        );
        assert!(
            !ops.contains(&crate::bytecode::OpCode::NewArray),
            "should NOT fall back to NewArray"
        );
    }

    #[test]
    fn test_int_array_emits_typed_opcodes() {
        let ops = compiled_opcodes("[1, 2, 3]");
        assert!(
            ops.contains(&crate::bytecode::OpCode::NewTypedArrayI64),
            "expected NewTypedArrayI64, got opcodes: {:?}",
            ops
        );
        let push_count = ops
            .iter()
            .filter(|&&op| op == crate::bytecode::OpCode::TypedArrayPushI64)
            .count();
        assert_eq!(
            push_count, 3,
            "expected 3 TypedArrayPushI64, got {}",
            push_count
        );
    }

    #[test]
    fn test_bool_array_emits_typed_opcodes() {
        let ops = compiled_opcodes("[true, false, true]");
        assert!(
            ops.contains(&crate::bytecode::OpCode::NewTypedArrayBool),
            "expected NewTypedArrayBool, got opcodes: {:?}",
            ops
        );
        let push_count = ops
            .iter()
            .filter(|&&op| op == crate::bytecode::OpCode::TypedArrayPushBool)
            .count();
        assert_eq!(
            push_count, 3,
            "expected 3 TypedArrayPushBool, got {}",
            push_count
        );
    }

    #[test]
    fn test_mixed_array_falls_back_to_generic() {
        // Mixed int/string array should use the generic NewArray path
        let ops = compiled_opcodes(r#"[1, "hello"]"#);
        assert!(
            ops.contains(&crate::bytecode::OpCode::NewArray),
            "expected NewArray for mixed array, got opcodes: {:?}",
            ops
        );
        assert!(
            !ops.contains(&crate::bytecode::OpCode::NewTypedArrayI64),
            "should NOT emit NewTypedArrayI64 for mixed array"
        );
    }

    #[test]
    fn test_empty_array_uses_generic_path() {
        let ops = compiled_opcodes("[]");
        assert!(
            ops.contains(&crate::bytecode::OpCode::NewArray),
            "expected NewArray for empty array, got opcodes: {:?}",
            ops
        );
    }

    #[test]
    fn test_float_array_executes_correctly() {
        use crate::test_utils::eval;
        use shape_value::value_word::vw_as_f64;
        // End-to-end: [1.0, 2.0, 3.0].sum() should produce 6.0
        let result = eval("[1.0, 2.0, 3.0].sum()");
        assert_eq!(
            vw_as_f64(result),
            Some(6.0),
            "float array sum should be 6.0"
        );
    }

    #[test]
    fn test_int_array_executes_correctly() {
        use crate::test_utils::eval;
        use shape_value::value_word::vw_as_i64;
        // End-to-end: [10, 20, 30].sum() should produce 60
        let result = eval("[10, 20, 30].sum()");
        let val = vw_as_i64(result).or_else(|| {
            shape_value::value_word::vw_as_f64(result).map(|f| f as i64)
        });
        assert_eq!(val, Some(60), "int array sum should be 60");
    }

    #[test]
    fn test_bool_array_len_executes_correctly() {
        use crate::test_utils::eval;
        use shape_value::value_word::vw_as_i64;
        let result = eval("[true, false, true].len()");
        assert_eq!(
            vw_as_i64(result),
            Some(3),
            "bool array len should be 3"
        );
    }
}
