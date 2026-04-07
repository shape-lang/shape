//! v2 typed array opcode emission helpers.
//!
//! This module is the *gating layer* between the bytecode compiler and the
//! v2 typed array opcode set. Given a `ConcreteType` for an array's element
//! type, [`should_use_typed_array`] reports whether the compiler can safely
//! emit one of the typed `NewTypedArray*`/`TypedArrayPush*`/`TypedArrayGet*`
//! opcodes (Phase 3.1).
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
//! those cases — Phase 3.1 is intentionally narrow on the typed-fast-path.
//!
//! As more typed array opcodes land (`u8`, `i16`, etc.) this helper will grow
//! more `Some(...)` arms; callers don't need to change.

use shape_value::v2::ConcreteType;

use crate::bytecode::OpCode;

/// The kind of typed array the compiler should emit for a known element type.
///
/// Each variant corresponds to a `TypedArray<T>` instantiation that has a
/// matching set of `NewTypedArray*`/`TypedArrayGet*`/`TypedArrayPush*`/
/// `TypedArraySet*` opcodes (Phase 3.1).
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
/// "do we have a typed-array opcode for this element type?" question used
/// by `compile_expr_array` and array method dispatch. Adding a new typed
/// element type (say `TypedArrayKind::U8`) is a one-line change here plus
/// new opcodes — call sites pick it up automatically.
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

/// `SlotKind` analogue of [`should_use_typed_array`].
///
/// Provided as a bridge for compiler call sites that haven't yet been
/// converted to use `ConcreteType`. The current Phase 1.2 element-type
/// inference (`v2_array_emission::infer_array_element_type`) returns
/// `SlotKind`, so `compile_expr_array` calls this variant to look up a
/// typed array kind directly.
///
/// The mapping mirrors [`should_use_typed_array`]: only the four element
/// types backed by typed array opcodes today (`Float64`/`Int64`/`Int32`/
/// `Bool`) return `Some`. Anything else (`String`, sized ints other than
/// i32/i64, nullable variants, `NanBoxed`/`Unknown`) falls back to the
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

/// Map a scalar element type name (the inner `T` of `Vec<T>`/`Array<T>`)
/// to a [`TypedArrayKind`], if a typed-array fast path exists for that name.
///
/// Recognises the four scalar element types backed by typed array opcodes
/// today: `number`/`f64` → F64, `int`/`i64` → I64, `i32` → I32, `bool` → Bool.
/// Anything else (including sized ints we don't have opcodes for, and heap
/// types like `string`) returns `None`, which leaves the call site on the
/// legacy NaN-boxed array path.
///
/// This mirrors [`should_use_typed_array`] / [`should_use_typed_array_from_slot_kind`]
/// but operates on the textual element name carried by `VariableTypeInfo::type_name`
/// (`"Vec<int>"` etc). It's the bridge layer used by `compile_expr_method_call`,
/// `compile_expr_index_access`, and `compile_expr_assign` (Phase 3.1 Agent 3) to
/// resolve a tracked array receiver to a typed-array opcode kind.
#[inline]
pub fn typed_array_kind_for_element_name(name: &str) -> Option<TypedArrayKind> {
    match name.trim() {
        "number" | "f64" => Some(TypedArrayKind::F64),
        "int" | "i64" => Some(TypedArrayKind::I64),
        "i32" => Some(TypedArrayKind::I32),
        "bool" => Some(TypedArrayKind::Bool),
        _ => None,
    }
}

/// Map a tracked type name like `"Vec<int>"` / `"Array<number>"` to a
/// [`TypedArrayKind`], if the element type has a typed-array fast path.
///
/// Used by Phase 3.1 method-dispatch / index-access call sites to recognise
/// homogeneous typed arrays whose element type was inferred from a `let` annotation
/// or array literal. Returns `None` for any non-array shape, for element types
/// without a typed opcode kind, and for `Option<...>` wrappers (caller can choose
/// to peel `Option` first if needed).
#[inline]
pub fn typed_array_kind_from_type_name(type_name: &str) -> Option<TypedArrayKind> {
    let trimmed = type_name.trim();
    let inner = trimmed
        .strip_prefix("Vec<")
        .or_else(|| trimmed.strip_prefix("Array<"))?
        .strip_suffix('>')?;
    typed_array_kind_for_element_name(inner)
}

impl super::BytecodeCompiler {
    /// Resolve an array receiver expression (`Identifier(name)`) to a
    /// [`TypedArrayKind`], if the receiver is a tracked array whose element
    /// type has a typed-array fast path.
    ///
    /// Walks the receiver name through:
    ///   1. Local slot type-tracker entry (`Vec<int>` etc).
    ///   2. Module binding type-tracker entry.
    ///
    /// Returns `None` for non-identifier receivers, for unresolved names, for
    /// receivers tracked as something other than a homogeneous typed array,
    /// and for element types that have no typed opcode kind today (`string`,
    /// sized ints other than `i32`/`i64`, etc). The caller is expected to
    /// fall back to the legacy NaN-boxed path in those cases.
    ///
    /// Phase 3.1 Agent 3 entry point — used by `compile_expr_method_call`,
    /// `compile_expr_index_access`, and `compile_expr_assign` to gate typed
    /// array opcode emission for `arr.push(x)`, `arr.pop()`, `arr.length`,
    /// `arr[i]`, and `arr[i] = x`.
    pub(crate) fn resolve_receiver_typed_array_kind(
        &self,
        receiver: &shape_ast::ast::Expr,
    ) -> Option<TypedArrayKind> {
        let name = match receiver {
            shape_ast::ast::Expr::Identifier(name, _) => name,
            _ => return None,
        };

        // Local slot first — ONLY if the slot was actually allocated as a
        // v2 typed array via `compile_expr_array`'s typed path. We CANNOT
        // simply trust the type-tracker name here because legacy untyped
        // literals (`let mut a = [1, 2, 3]`) get a `Vec<int>` type-tracker
        // entry too, but the runtime value is a NaN-boxed VMArray, not a
        // `*const TypedArray<i64>`. Emitting a typed get/set for that
        // would corrupt memory.
        if let Some(local_idx) = self.resolve_local(name) {
            if let Some(&kind) = self.v2_typed_array_locals.get(&local_idx) {
                return Some(kind);
            }
            return None;
        }

        // Module binding fallback (same restriction).
        let scoped_name = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped_name) {
            if let Some(&kind) = self.v2_typed_array_module_bindings.get(&binding_idx) {
                return Some(kind);
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::v2::concrete_type::{EnumLayoutId, StructLayoutId};

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
    fn test_slot_kind_nan_boxed_falls_back() {
        use crate::type_tracking::SlotKind;
        assert_eq!(
            should_use_typed_array_from_slot_kind(SlotKind::NanBoxed),
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
}

// ──────────────────────────────────────────────────────────────────────
// Compile integration tests — verify `compile_expr_array` emits the
// correct opcode (`NewTypedArray*` vs legacy `NewArray`/`NewTypedArray`)
// for the array literal shapes called out in the Phase 3.1 deliverables.
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod compile_integration_tests {
    use super::*;
    use crate::bytecode::{BytecodeProgram, OpCode};
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

    #[test]
    fn test_number_literal_emits_new_typed_array_f64() {
        // Annotated: `let arr: Array<number> = [1.0, 2.0, 3.0]` -> NewTypedArrayF64
        let prog = compile(
            r#"
            let arr: Array<number> = [1.0, 2.0, 3.0]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayF64),
            "expected NewTypedArrayF64 in instruction stream"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushF64),
            "expected TypedArrayPushF64 in instruction stream"
        );
    }

    #[test]
    fn test_int_literal_emits_new_typed_array_i64() {
        // Annotated: `let arr: Array<int> = [1, 2, 3]` -> NewTypedArrayI64
        // Bare literals (`[1, 2, 3]` with no annotation) deliberately
        // stay on the legacy `NewTypedArray` path because runtime tests
        // depend on the v1 NaN-boxed shape.
        let prog = compile(
            r#"
            let arr: Array<int> = [1, 2, 3]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI64),
            "expected NewTypedArrayI64 in instruction stream"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushI64),
            "expected TypedArrayPushI64 in instruction stream"
        );
    }

    #[test]
    fn test_bool_literal_emits_new_typed_array_bool() {
        // Annotated: `let arr: Array<bool> = [true, false]` -> NewTypedArrayBool
        let prog = compile(
            r#"
            let arr: Array<bool> = [true, false]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayBool),
            "expected NewTypedArrayBool in instruction stream"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushBool),
            "expected TypedArrayPushBool in instruction stream"
        );
    }

    #[test]
    fn test_typed_int_literal_emits_new_typed_array_i32() {
        // Annotated: `let arr: Array<i32> = [1, 2, 3]` -> NewTypedArrayI32
        let prog = compile(
            r#"
            let arr: Array<i32> = [1, 2, 3]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewTypedArrayI32),
            "expected NewTypedArrayI32 in instruction stream"
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushI32),
            "expected TypedArrayPushI32 in instruction stream"
        );
    }

    #[test]
    fn test_heterogeneous_literal_falls_back_to_legacy_new_array() {
        // `[1, "x", true]` is heterogeneous → no typed array fast path,
        // falls back to legacy `NewArray` (NaN-boxed Vec<ValueWord>).
        let prog = compile("[1, \"x\", true]");
        assert!(
            has_opcode(&prog, OpCode::NewArray),
            "heterogeneous literal must emit legacy NewArray"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayI64),
            "heterogeneous literal must not emit NewTypedArrayI64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayF64),
            "heterogeneous literal must not emit NewTypedArrayF64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayBool),
            "heterogeneous literal must not emit NewTypedArrayBool"
        );
    }

    #[test]
    fn test_struct_array_falls_back_to_legacy_new_array() {
        // `let arr: Array<MyStruct> = [...]` — element type is a heap
        // (struct) type with no typed opcode kind. Must fall back to the
        // legacy NaN-boxed `NewArray` path.
        //
        // Note: `infer_array_element_type` already returns `None` for
        // object literals, so this exercises the fallback purely on the
        // shape of the elements (object literals → no typed kind).
        let prog = compile(
            r#"
            type Point { x: int, y: int }
            let arr = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }]
            arr
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::NewArray),
            "heap-typed array must emit legacy NewArray"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayF64),
            "heap-typed array must not emit NewTypedArrayF64"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayI64),
            "heap-typed array must not emit NewTypedArrayI64"
        );
    }

    #[test]
    fn test_empty_literal_falls_back_to_legacy_new_array() {
        // Empty literal has no element type the compiler can prove —
        // fall back to legacy `NewArray`.
        let prog = compile("[]");
        assert!(
            has_opcode(&prog, OpCode::NewArray),
            "empty array must emit legacy NewArray (no element type)"
        );
        assert!(
            !has_opcode(&prog, OpCode::NewTypedArrayF64),
            "empty array must not emit a typed-array opcode"
        );
    }

    // ──────────────────────────────────────────────────────────────────
    // v2 Phase 3.1 (Agent 3): method dispatch / index access / index
    // assignment / property access fast paths.
    //
    // These tests verify that `arr.push(x)`, `arr[i]`, `arr[i] = x`,
    // and `arr.length` lower to typed array opcodes when the receiver
    // is a tracked array with a homogeneous, typed-opcode-backed element
    // type. They also verify the fail-soft fallback to generic
    // `GetProp`/`SetProp`/`Length`/`ArrayPushLocal` when the element
    // type is unknown.
    // ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_index_access_typed_int_array_emits_typed_get_i64() {
        // `let arr: Array<int> = [1, 2, 3]; arr[0]` -> TypedArrayGetI64
        let prog = compile(
            r#"
            let arr: Array<int> = [1, 2, 3]
            arr[0]
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayGetI64),
            "expected TypedArrayGetI64 in instruction stream, got: {:?}",
            prog.instructions
                .iter()
                .map(|i| i.opcode)
                .collect::<Vec<_>>()
        );
        // Generic GetProp must NOT be present for the index access.
        // (Legacy NewTypedArrayI64 from the literal IS expected.)
    }

    #[test]
    fn test_index_access_typed_number_array_emits_typed_get_f64() {
        let prog = compile(
            r#"
            let arr: Array<number> = [1.0, 2.0, 3.0]
            arr[0]
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayGetF64),
            "expected TypedArrayGetF64 in instruction stream"
        );
    }

    #[test]
    fn test_index_access_typed_bool_array_emits_typed_get_bool() {
        let prog = compile(
            r#"
            let arr: Array<bool> = [true, false]
            arr[0]
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayGetBool),
            "expected TypedArrayGetBool in instruction stream"
        );
    }

    #[test]
    fn test_index_access_untyped_falls_back_to_get_prop() {
        // Empty / heterogeneous array → no typed kind → falls back.
        let prog = compile(
            r#"
            let arr = [1, "x", true]
            arr[0]
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::GetProp),
            "expected legacy GetProp for untyped array index access"
        );
        assert!(
            !has_opcode(&prog, OpCode::TypedArrayGetI64),
            "untyped array must not emit TypedArrayGetI64"
        );
        assert!(
            !has_opcode(&prog, OpCode::TypedArrayGetF64),
            "untyped array must not emit TypedArrayGetF64"
        );
    }

    #[test]
    fn test_push_typed_number_array_emits_typed_push_f64() {
        // `let arr: Array<number> = [1.0]; arr.push(2.0)` -> TypedArrayPushF64
        // Note: the literal `[1.0]` already emits TypedArrayPushF64 for the
        // initial elements, so we additionally check that there's a
        // TypedArrayPushF64 after a LoadLocal — i.e. the explicit push call.
        let prog = compile(
            r#"
            let mut arr: Array<number> = [1.0]
            arr.push(2.0)
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushF64),
            "expected TypedArrayPushF64 for arr.push(2.0)"
        );
        assert!(
            !has_opcode(&prog, OpCode::ArrayPushLocal),
            "should not emit legacy ArrayPushLocal when typed-array path applies"
        );
    }

    #[test]
    fn test_push_typed_int_array_emits_typed_push_i64() {
        let prog = compile(
            r#"
            let mut arr: Array<int> = [1]
            arr.push(2)
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayPushI64),
            "expected TypedArrayPushI64 for arr.push(2)"
        );
        assert!(
            !has_opcode(&prog, OpCode::ArrayPushLocal),
            "should not emit legacy ArrayPushLocal when typed-array path applies"
        );
    }

    #[test]
    fn test_index_assign_typed_int_array_emits_typed_set_i64() {
        // `let mut arr: Array<int> = [1]; arr[0] = 99` -> TypedArraySetI64
        let prog = compile(
            r#"
            let mut arr: Array<int> = [1]
            arr[0] = 99
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArraySetI64),
            "expected TypedArraySetI64 in instruction stream"
        );
        assert!(
            !has_opcode(&prog, OpCode::SetLocalIndex),
            "should not emit legacy SetLocalIndex for typed array"
        );
    }

    #[test]
    fn test_index_assign_typed_number_array_emits_typed_set_f64() {
        let prog = compile(
            r#"
            let mut arr: Array<number> = [1.0]
            arr[0] = 99.0
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArraySetF64),
            "expected TypedArraySetF64 in instruction stream"
        );
    }

    #[test]
    fn test_length_typed_bool_array_emits_typed_array_len() {
        // `let arr: Array<bool> = [true]; arr.length` -> TypedArrayLen
        let prog = compile(
            r#"
            let arr: Array<bool> = [true]
            arr.length
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayLen),
            "expected TypedArrayLen for arr.length"
        );
        // Generic Length opcode should NOT be present.
        assert!(
            !has_opcode(&prog, OpCode::Length),
            "should not emit legacy Length for typed array"
        );
    }

    #[test]
    fn test_length_typed_int_array_emits_typed_array_len() {
        let prog = compile(
            r#"
            let arr: Array<int> = [1, 2, 3]
            arr.length
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::TypedArrayLen),
            "expected TypedArrayLen for arr.length"
        );
    }

    #[test]
    fn test_length_untyped_array_falls_back_to_legacy_length() {
        let prog = compile(
            r#"
            let arr = [1, "x", true]
            arr.length
            "#,
        );
        assert!(
            has_opcode(&prog, OpCode::Length),
            "expected legacy Length for untyped array"
        );
        assert!(
            !has_opcode(&prog, OpCode::TypedArrayLen),
            "untyped array must not emit TypedArrayLen"
        );
    }
}
