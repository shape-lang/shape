//! Function and method call expression compilation

use crate::bytecode::{BuiltinFunction, Constant, Instruction, OpCode, Operand};
use crate::compiler::comptime_builtins::semantic_freeze::SpecializationTypeOverlay;
use crate::compiler::monomorphization::cache::ClosureDefPeek;
use crate::compiler::monomorphization::call_site_consts;
use crate::compiler::monomorphization::semantic_specialization::SemanticSpecializationRequest;
use crate::compiler::monomorphization::type_resolution::{
    concrete_type_for_expr, extract_arg_concrete_types, resolve_call_site_type_args,
    resolve_call_site_type_args_from_expected_return, resolve_call_site_type_args_with_closures,
};
use crate::compiler::string_interpolation::has_interpolation;
use crate::compiler::v2_typed_emission::{TypedArrayKind, should_use_typed_array};
use crate::executor::typed_object_ops::field_type_to_tag;
use crate::type_tracking::{VariableKind, VariableTypeInfo};
use shape_ast::ast::{Expr, InterpolationMode, Literal, ObjectEntry, Span, Spanned, Statement};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::closure::EnvironmentAnalyzer;
use shape_runtime::type_system::Type;
use shape_runtime::type_system::suggestions::suggest_function;
use shape_value::v2::ConcreteType;
use std::collections::BTreeSet;
use std::collections::HashMap;

use super::super::{BuiltinNameResolution, BytecodeCompiler, ModuleBuiltinFunction};
use super::number_extend_specialization::{
    number_receiver_generic_substitutions, substitute_type_params_in_annotation,
};

fn type_annotation_from_concrete_type(ct: &ConcreteType) -> Option<shape_ast::ast::TypeAnnotation> {
    crate::compiler::expressions::closures::concrete_type_to_type_annotation(ct)
}

fn content_type_info() -> VariableTypeInfo {
    VariableTypeInfo::with_storage(
        "content".to_string(),
        crate::type_tracking::NativeKind::Ptr(shape_value::HeapKind::Content),
    )
}

fn content_preserving_method(method: &str) -> bool {
    matches!(
        method,
        "bold"
            | "italic"
            | "underline"
            | "dim"
            | "fg"
            | "bg"
            | "border"
            | "max_rows"
            | "maxRows"
            | "add"
            | "series"
            | "title"
            | "x_label"
            | "xLabel"
            | "y_label"
            | "yLabel"
            | "width"
            | "height"
            | "headers"
            | "row"
            | "language"
            | "source"
            | "pair"
            | "build"
    )
}

#[cfg(test)]
mod w28_hof_reduce_static_proof_tests {
    use crate::test_utils::eval_typed_i64;

    #[test]
    fn flatmap_result_threads_element_type_to_reduce() {
        let src = r#"
            let nested = [[1, 2], [3, 4], [5]]
            let flat = nested.flatMap(|arr| arr)
            flat.reduce(|acc, x| acc + x, 0)
        "#;

        assert_eq!(eval_typed_i64(src), 15);
    }
}

fn concrete_type_cache_key(ct: &ConcreteType) -> String {
    type_annotation_from_concrete_type(ct)
        .map(|ann| {
            ann.to_type_string()
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                .collect::<String>()
        })
        .unwrap_or_else(|| format!("{:?}", ct).replace(|ch: char| !ch.is_ascii_alphanumeric(), "_"))
}

fn adopt_int_literal_for_implicit_numeric_expr(
    left_ct: &ConcreteType,
    left: &Expr,
    right_ct: &ConcreteType,
    right: &Expr,
) -> Option<ConcreteType> {
    let is_int_lit = |expr: &Expr| matches!(expr, Expr::Literal(Literal::Int(_), _));
    if *left_ct == ConcreteType::F64 && *right_ct == ConcreteType::I64 && is_int_lit(right) {
        return Some(ConcreteType::F64);
    }
    if *right_ct == ConcreteType::F64 && *left_ct == ConcreteType::I64 && is_int_lit(left) {
        return Some(ConcreteType::F64);
    }
    None
}

#[cfg(test)]
mod w27_implicit_generic_tests {
    use crate::bytecode::OpCode;
    use crate::compiler::BytecodeCompiler;
    use crate::executor::{VMConfig, VirtualMachine};
    use crate::test_utils::compile_with_prelude;
    use crate::type_tracking::NativeKind;
    use shape_ast::parser::parse_program;
    use shape_value::{KindedSlot, ValueSlot};

    fn eval_with_source_and_kind(source: &str, expected: NativeKind) -> KindedSlot {
        let program = parse_program(source).expect("source should parse");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(source);
        let bytecode = compiler.compile(&program).expect("compile should succeed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let bits = vm.execute_raw(None).expect("execution should succeed");
        KindedSlot::new(ValueSlot::from_raw(bits), expected)
    }

    fn eval_without_source_and_kind(source: &str, expected: NativeKind) -> KindedSlot {
        let program = parse_program(source).expect("source should parse");
        let compiler = BytecodeCompiler::new();
        let bytecode = compiler.compile(&program).expect("compile should succeed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let bits = vm.execute_raw(None).expect("execution should succeed");
        KindedSlot::new(ValueSlot::from_raw(bits), expected)
    }

    #[test]
    fn complex_math_library_calls_concrete_implicit_specializations() {
        let source = r#"
            fn abs(x) { if x < 0 { 0 - x } else { x } }
            fn max(a, b) { if a > b { a } else { b } }
            fn min(a, b) { if a < b { a } else { b } }
            fn clamp(x, lo, hi) { max(lo, min(x, hi)) }

            print(abs(-7))
            print(max(3, 9))
            print(min(3, 9))
            print(clamp(15, 0, 10))
            print(clamp(-5, 0, 10))
            print(clamp(5, 0, 10))
        "#;

        let bytecode = compile_with_prelude(source).expect("compile should succeed");
        let ca = bytecode
            .content_addressed
            .as_ref()
            .expect("graph/prelude compile should produce content-addressed blobs");

        let blob_named = |needle: &str| {
            ca.function_store
                .values()
                .find(|blob| blob.name.contains(needle))
                .unwrap_or_else(|| {
                    let mut names: Vec<_> = ca
                        .function_store
                        .values()
                        .map(|blob| blob.name.as_str())
                        .collect();
                    names.sort();
                    panic!("missing blob containing {needle}; blobs: {names:?}")
                })
        };

        let max_blob = blob_named("__w27_implicit_max");
        assert!(
            max_blob
                .instructions
                .iter()
                .any(|instruction| instruction.opcode == OpCode::GtInt),
            "max specialization should use a real typed comparison, got {:?}",
            max_blob.instructions
        );

        let min_blob = blob_named("__w27_implicit_min");
        assert!(
            min_blob
                .instructions
                .iter()
                .any(|instruction| instruction.opcode == OpCode::LtInt),
            "min specialization should use a real typed comparison, got {:?}",
            min_blob.instructions
        );

        let clamp_blob = ca
            .function_store
            .values()
            .find(|blob| blob.name == "clamp")
            .expect("missing source clamp blob");
        assert!(
            clamp_blob
                .callee_names
                .iter()
                .any(|name| name.contains("__w27_implicit_max"))
                && clamp_blob
                    .callee_names
                    .iter()
                    .any(|name| name.contains("__w27_implicit_min")),
            "clamp specialization should call concrete max/min specializations, got {:?}",
            clamp_blob.callee_names
        );
    }

    // ADR-009 C3 #14 (slice 4, S4a — #66 item 1 collateral completion),
    // measured via the comptime handler wrapper: a MIXED
    // annotated/unannotated implicit-generic fn whose ANNOTATED-position
    // arg has no compile-time-resolvable type (here: a known-binding
    // identifier — the comptime mini-program's runtime-preset target/ctx
    // shape) must still specialize on its UNANNOTATED positions. Pre-fix,
    // `try_specialize_implicit_generic_free_function_call` required a
    // concrete type for EVERY arg — one unresolvable arg at an annotated
    // position (whose declaration the substitution never touches) vetoed
    // the whole specialization, the call dispatched onto the dead deferred
    // template, and a generic callee on the unannotated param inside that
    // template hard-failed with "cannot infer type argument(s) for generic
    // function 'cap'".
    #[test]
    fn mixed_annotated_implicit_generic_specializes_despite_unresolvable_annotated_arg() {
        let source = r#"
fn cap<T>(name, value: T) -> int { return 7 }
fn handler(t: string, c: string, factor) -> int { return cap("f", factor) }
handler(tgt, ctx, 3)
"#;
        let program = parse_program(source).expect("source should parse");
        let mut compiler = BytecodeCompiler::new();
        // Runtime-preset bindings: known by NAME only — no compile-time type.
        compiler.register_known_bindings(&["tgt".to_string(), "ctx".to_string()]);
        let bytecode = compiler
            .compile(&program)
            .expect("the mixed annotated/unannotated implicit generic must compile");
        // The unannotated position drove a real specialization; the two
        // annotated positions key as declaration-fixed.
        assert!(
            bytecode
                .functions
                .iter()
                .any(|function| function.name.starts_with("__w27_implicit_handler")),
            "expected a __w27_implicit_handler specialization, got: {:?}",
            bytecode
                .functions
                .iter()
                .map(|function| function.name.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unannotated_mutual_recursion_rejects_wrong_kind_call_boundary() {
        let source = r#"
            function is_even(n) {
                let val = n
                if val == 0 { return true }
                return is_odd(val - 1)
            }
            function is_odd(n) {
                let val = n
                if val == 0 { return false }
                return is_even(val - 1)
            }
            is_even(10)
        "#;

        let program = parse_program(source).expect("source should parse");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(source);
        let err = compiler
            .compile(&program)
            .expect_err("unannotated mutual recursion must reject before execution");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("cannot safely pass argument #1")
                && msg.contains("statically proven as")
                && msg.contains("callee parameter slot is"),
            "unexpected diagnostic: {msg}"
        );
    }

    #[test]
    fn unannotated_numeric_mutual_recursion_specializes_param_kind() {
        let source = r#"
            function is_even(n) {
                if n == 0 { return 1 }
                return is_odd(n - 1)
            }
            function is_odd(n) {
                if n == 0 { return 0 }
                return is_even(n - 1)
            }
            is_even(10)
        "#;

        assert_eq!(
            eval_with_source_and_kind(source, NativeKind::Int64).as_i64(),
            Some(1)
        );
    }

    #[test]
    fn unannotated_numeric_function_preserves_float64_callsite() {
        let source = r#"
            function add_one(x) {
                return x + 1
            }
            add_one(1.5)
        "#;

        assert_eq!(
            eval_with_source_and_kind(source, NativeKind::Float64).as_f64(),
            Some(2.5)
        );
    }

    #[test]
    fn pipe_chain_preserves_float64_implicit_generic_callsite() {
        let source = r#"
            function double(x) {
                return x * 2
            }
            function add_one(x) {
                return x + 1
            }
            5.0 |> double |> add_one
        "#;

        assert_eq!(
            eval_with_source_and_kind(source, NativeKind::Float64).as_f64(),
            Some(11.0)
        );
    }

    #[test]
    fn pipe_call_does_not_default_unproven_numeric_specialization() {
        let source = r#"
            function add_one(x) {
                return x + 1
            }
            "oops" |> add_one
        "#;

        let program = parse_program(source).expect("source should parse");
        let mut compiler = BytecodeCompiler::new();
        compiler.set_source(source);
        let err = compiler
            .compile(&program)
            .expect_err("string pipe into numeric implicit generic must reject");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("Cannot apply `+` to a `string` and a `int`")
                && msg.contains("Strict typing does not implicitly convert"),
            "unexpected diagnostic: {msg}"
        );
    }

    #[test]
    fn source_unavailable_mutual_recursion_preserves_float64_callsite() {
        let source = r#"
            function is_even(n) {
                if n == 0 { return 1.0 }
                return is_odd(n - 1)
            }
            function is_odd(n) {
                if n == 0 { return 0.0 }
                return is_even(n - 1)
            }
            is_even(10.0)
        "#;

        assert_eq!(
            eval_without_source_and_kind(source, NativeKind::Float64).as_f64(),
            Some(1.0)
        );
    }
}

/// Strict-typing-sweep (Cluster 3): map a `NativeKind` (the type-tracker's
/// per-slot storage hint) to an AST `TypeAnnotation`. Used by HOF dispatch
/// to type closure user params from a bare `[1, 2, 3]`-literal receiver
/// when no whole-binding `ConcreteType::Array(elem)` entry exists yet.
fn slot_kind_to_type_annotation(
    kind: crate::type_tracking::NativeKind,
) -> Option<shape_ast::ast::TypeAnnotation> {
    use crate::type_tracking::NativeKind;
    use shape_ast::ast::TypeAnnotation;
    Some(match kind {
        NativeKind::Float64 => TypeAnnotation::Basic("number".to_string()),
        NativeKind::Int64 => TypeAnnotation::Basic("int".to_string()),
        NativeKind::Int32 => TypeAnnotation::Basic("i32".to_string()),
        NativeKind::Int16 => TypeAnnotation::Basic("i16".to_string()),
        NativeKind::Int8 => TypeAnnotation::Basic("i8".to_string()),
        NativeKind::UInt64 => TypeAnnotation::Basic("u64".to_string()),
        NativeKind::UInt32 => TypeAnnotation::Basic("u32".to_string()),
        NativeKind::UInt16 => TypeAnnotation::Basic("u16".to_string()),
        NativeKind::UInt8 => TypeAnnotation::Basic("u8".to_string()),
        NativeKind::Bool => TypeAnnotation::Basic("bool".to_string()),
        NativeKind::String => TypeAnnotation::Basic("string".to_string()),
        // Other kinds (Decimal, BigInt, DateTime, nullable variants,
        // pointers, etc.) are not productive for typed binary-op emission;
        // returning None lets the closure body compile with no annotation,
        // which is identical to the pre-fix behaviour.
        _ => return None,
    })
}

fn array_element_annotation_from_inferred_type(
    compiler: &BytecodeCompiler,
    ty: &Type,
) -> Option<shape_ast::ast::TypeAnnotation> {
    use shape_ast::ast::TypeAnnotation;

    let ann = ty.to_annotation()?;
    let elem_ann = match ann {
        TypeAnnotation::Array(inner) => *inner,
        TypeAnnotation::Generic { name, mut args }
            if (name.as_str() == "Array" || name.as_str() == "Vec") && args.len() == 1 =>
        {
            args.remove(0)
        }
        _ => return None,
    };

    crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
        compiler, &elem_ann,
    )?;
    Some(elem_ann)
}

fn type_annotation_from_tracker_name(type_name: &str) -> Option<shape_ast::ast::TypeAnnotation> {
    use shape_ast::ast::{TypeAnnotation, TypePath};

    let type_name = type_name.trim();
    if type_name.is_empty() || type_name == "unknown" {
        return None;
    }

    if let Some(inner) = type_name
        .strip_prefix("Vec<")
        .and_then(|s| s.strip_suffix('>'))
        .or_else(|| {
            type_name
                .strip_prefix("Array<")
                .and_then(|s| s.strip_suffix('>'))
        })
    {
        let inner_ann = type_annotation_from_tracker_name(inner)?;
        return Some(TypeAnnotation::Generic {
            name: TypePath::simple("Vec"),
            args: vec![inner_ann],
        });
    }

    if let Some(inner) = type_name.strip_suffix("[]") {
        let inner_ann = type_annotation_from_tracker_name(inner)?;
        return Some(TypeAnnotation::Array(Box::new(inner_ann)));
    }

    if matches!(
        type_name,
        "int"
            | "i64"
            | "i32"
            | "i16"
            | "i8"
            | "u64"
            | "u32"
            | "u16"
            | "u8"
            | "number"
            | "f64"
            | "float"
            | "bool"
            | "string"
            | "decimal"
            | "bigint"
            | "DateTime"
    ) {
        Some(TypeAnnotation::Basic(type_name.to_string()))
    } else {
        Some(TypeAnnotation::Reference(TypePath::simple(type_name)))
    }
}

fn array_element_annotation_from_tracker_name(
    compiler: &BytecodeCompiler,
    type_name: &str,
) -> Option<shape_ast::ast::TypeAnnotation> {
    use shape_ast::ast::TypeAnnotation;

    let ann = type_annotation_from_tracker_name(type_name)?;
    let elem_ann = match ann {
        TypeAnnotation::Array(inner) => *inner,
        TypeAnnotation::Generic { name, mut args }
            if (name.as_str() == "Array" || name.as_str() == "Vec") && args.len() == 1 =>
        {
            args.remove(0)
        }
        _ => return None,
    };

    crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
        compiler, &elem_ann,
    )?;
    Some(elem_ann)
}

fn tracker_type_info_from_concrete_type(ct: &ConcreteType) -> Option<VariableTypeInfo> {
    crate::compiler::patterns::binding::concrete_type_tracker_name(ct).map(VariableTypeInfo::named)
}

/// Array methods whose Rust PHF handlers are the canonical strict-carrier
/// implementation when the receiver is statically proven as `Array<T>`.
///
/// This gate is deliberately compile-time only: receiver proof comes from the
/// existing `ConcreteType::Array(_)` metadata, and runtime dispatch still uses
/// the kinded `CallMethod` path. `push` stays out because identifier receivers
/// use the bespoke typed push/writeback path and the generic PHF handler has a
/// different return shape.
fn prefer_native_array_phf_method(method: &str) -> bool {
    matches!(
        method,
        "clone"
            | "forEach"
            | "findIndex"
            | "indexOf"
            | "includes"
            | "slice"
            | "take"
            | "drop"
            | "skip"
            | "concat"
            | "flatten"
            | "groupBy"
            | "pop"
    )
}

fn array_element_kind_from_concrete_type(ct: &ConcreteType) -> Option<TypedArrayKind> {
    match ct {
        ConcreteType::Array(elem) => should_use_typed_array(elem.as_ref()),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum CaptureMutSelfWriteBackTarget {
    OwnedMutable { capture_idx: u16, opcode: OpCode },
    Shared { capture_idx: u16, opcode: OpCode },
}

fn container_kind_from_concrete_type(
    ct: &ConcreteType,
) -> Option<crate::compiler::mutation_writeback::ContainerKind> {
    use crate::compiler::mutation_writeback::ContainerKind;
    match ct {
        ConcreteType::HashMap(_, _) => Some(ContainerKind::HashMap),
        ConcreteType::HashSet(_) => Some(ContainerKind::HashSet),
        ConcreteType::Deque(_) => Some(ContainerKind::Deque),
        ConcreteType::PriorityQueue => Some(ContainerKind::PriorityQueue),
        ConcreteType::Array(_) => Some(ContainerKind::Array),
        _ => None,
    }
}

/// Task #108 companion: rewrite a return-type annotation by prefixing
/// any bare `Basic`/`Reference` names with the given namespace. Module-
/// qualified callees (`m::mk` returns `P`) carry their return type in
/// bare form even though the schema is registered as `m::P`; we use this
/// only as a fallback when the bare-name schema lookup misses, so type
/// info propagates through to a downstream `m::mk().x` property access
/// and the GetProp emit site can record its native-kind hint. Returns
/// `None` when the annotation already qualifies (`m::P`) or is shaped
/// such that prefixing wouldn't help (`Object`, `Function`, `Tuple`, …).
fn qualify_type_annotation_with_namespace(
    ann: &shape_ast::ast::TypeAnnotation,
    namespace: &str,
) -> Option<shape_ast::ast::TypeAnnotation> {
    use shape_ast::ast::TypeAnnotation;
    match ann {
        TypeAnnotation::Basic(name) if !name.contains("::") => {
            Some(TypeAnnotation::Basic(format!("{}::{}", namespace, name)))
        }
        TypeAnnotation::Reference(name) if !name.as_str().contains("::") => Some(
            TypeAnnotation::Reference(format!("{}::{}", namespace, name.as_str()).into()),
        ),
        _ => None,
    }
}

/// WS-9c: project a `FieldType` to the `TypeAnnotation` used as an
/// object-field contract. Unlike `field_type_to_annotation` (which refuses
/// `Array`/`Any`/`Option` so the caller falls back to the inference engine),
/// this best-effort projection records a contract for every field that has
/// a representable annotation; `Any` and unrepresentable shapes yield `None`
/// and simply carry no contract (the field stays an honest `unknown`).
pub(crate) fn field_type_contract_annotation(
    ft: &shape_runtime::type_schema::FieldType,
) -> Option<shape_ast::ast::TypeAnnotation> {
    use shape_ast::ast::TypeAnnotation;
    use shape_runtime::type_schema::FieldType;
    let basic = |s: &str| Some(TypeAnnotation::Basic(s.to_string()));
    match ft {
        FieldType::String => basic("string"),
        FieldType::I64 => basic("int"),
        FieldType::F64 => basic("number"),
        FieldType::Bool => basic("bool"),
        FieldType::Decimal => basic("decimal"),
        FieldType::Timestamp => basic("DateTime"),
        FieldType::I8 => basic("i8"),
        FieldType::U8 => basic("u8"),
        FieldType::I16 => basic("i16"),
        FieldType::U16 => basic("u16"),
        FieldType::I32 => basic("i32"),
        FieldType::U32 => basic("u32"),
        FieldType::U64 => basic("u64"),
        FieldType::Object(name) => Some(TypeAnnotation::Reference(name.as_str().into())),
        FieldType::Array(inner) => field_type_contract_annotation(inner)
            .map(|inner_ann| TypeAnnotation::Array(Box::new(inner_ann))),
        FieldType::Option(inner) => {
            field_type_contract_annotation(inner).map(TypeAnnotation::option)
        }
        // W17.3-4.1 — project HashMap<K, V> / Set<T> back to the
        // surface `TypeAnnotation::Generic { name, args }` shape the
        // parser emits. Inner contract projection is best-effort:
        // mirrors the existing Array/Option `?`-style propagation so
        // a container with an unrepresentable inner falls back to
        // `None` (the field stays an honest `unknown`).
        FieldType::HashMap { key, value } => {
            let k = field_type_contract_annotation(key)?;
            let v = field_type_contract_annotation(value)?;
            Some(TypeAnnotation::Generic {
                name: shape_ast::ast::type_path::TypePath::simple("HashMap"),
                args: vec![k, v],
            })
        }
        FieldType::Set(inner) => {
            let elem = field_type_contract_annotation(inner)?;
            Some(TypeAnnotation::Generic {
                name: shape_ast::ast::type_path::TypePath::simple("Set"),
                args: vec![elem],
            })
        }
        FieldType::Any => None,
    }
}

// U4-4: `return_type_to_numeric`, `builtin_return_numeric_type`, and
// `method_return_numeric_type` are DELETED. They computed a `NumericType` for
// the sole purpose of stamping the deleted `last_expr_numeric_type` register
// for call/method results (`floor(3.7) + 1 => 4`, `pow(sin(x),2.0)+...`). The
// inference engine already proves these return types (it recorded them in the
// span-keyed `resolved_expr_types` table), so a downstream binop now derives
// the call result's numeric kind via `numeric_type_of` → `infer_expr_type` —
// one source, no register-feeding tables.

/// Conservative compile-time-constant check for const parameters.
/// Accepts literals and recursively literal-composed containers.
fn is_compile_time_const_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_, _) => true,
        Expr::UnaryOp { operand, .. } => is_compile_time_const_expr(operand),
        Expr::BinaryOp { left, right, .. } => {
            is_compile_time_const_expr(left) && is_compile_time_const_expr(right)
        }
        Expr::Array(items, _) => items.iter().all(is_compile_time_const_expr),
        Expr::Object(entries, _) => entries
            .iter()
            .all(|entry| matches!(entry, shape_ast::ast::ObjectEntry::Field { value, .. } if is_compile_time_const_expr(value))),
        _ => false,
    }
}

fn literal_const_slot(literal: &Literal) -> Option<shape_value::KindedSlot> {
    use shape_value::{KindedSlot, NativeKind, ValueSlot};
    Some(match literal {
        Literal::Int(value) => KindedSlot::from_int(*value),
        Literal::UInt(value) => KindedSlot::new(ValueSlot::from_u64(*value), NativeKind::UInt64),
        Literal::TypedInt(value, _) => KindedSlot::from_int(*value),
        Literal::Number(value) => KindedSlot::from_number(*value),
        Literal::String(value) => KindedSlot::from_string(value),
        Literal::Char(value) => KindedSlot::from_char(*value),
        Literal::Bool(value) => KindedSlot::from_bool(*value),
        Literal::None | Literal::Unit => KindedSlot::none(),
        Literal::Decimal(_) | Literal::FormattedString { .. } | Literal::Timeframe(_) => {
            return None;
        }
    })
}

fn const_expr_literal(expr: &Expr) -> Option<&Literal> {
    match expr {
        Expr::Literal(literal, _) => Some(literal),
        _ => None,
    }
}

fn const_expr_fingerprint(expr: &Expr) -> Option<String> {
    const_expr_literal(expr).map(|literal| format!("{:?}", literal))
}

pub(crate) enum ConstFoldValue {}

pub(crate) fn eval_const_expr_to_nanboxed(expr: &Expr) -> Option<ConstFoldValue> {
    let _ = expr;
    None
}

impl BytecodeCompiler {
    fn reject_const_args_for_non_generic_call(
        &self,
        callee_name: &str,
        const_args: &[Expr],
        span: Span,
    ) -> Result<()> {
        if const_args.is_empty() {
            return Ok(());
        }
        Err(ShapeError::SemanticError {
            message: format!(
                "'{}' does not declare const generic parameters",
                callee_name
            ),
            location: Some(self.span_to_source_location(span)),
        })
    }

    fn field_type_to_static_hof_concrete_type(
        &self,
        ft: &shape_runtime::type_schema::FieldType,
    ) -> Option<ConcreteType> {
        use shape_runtime::type_schema::FieldType;
        use shape_value::v2::concrete_type::{EnumLayoutId, StructLayoutId};

        Some(match ft {
            FieldType::I64 | FieldType::Timestamp => ConcreteType::I64,
            FieldType::F64 => ConcreteType::F64,
            FieldType::Bool => ConcreteType::Bool,
            FieldType::String => ConcreteType::String,
            FieldType::Decimal => ConcreteType::Decimal,
            FieldType::I8 => ConcreteType::I8,
            FieldType::I16 => ConcreteType::I16,
            FieldType::I32 => ConcreteType::I32,
            FieldType::U8 => ConcreteType::U8,
            FieldType::U16 => ConcreteType::U16,
            FieldType::U32 => ConcreteType::U32,
            FieldType::U64 => ConcreteType::U64,
            FieldType::Array(inner) => ConcreteType::Array(Box::new(
                self.field_type_to_static_hof_concrete_type(inner)?,
            )),
            FieldType::Object(name) => {
                let resolved = self.resolve_type_name(name);
                if self
                    .type_tracker
                    .schema_registry()
                    .get(resolved.as_str())
                    .is_some_and(|schema| schema.get_enum_info().is_some())
                {
                    ConcreteType::named_enum(resolved, EnumLayoutId(0))
                } else {
                    ConcreteType::named_struct(resolved, StructLayoutId(0))
                }
            }
            FieldType::Option(_)
            | FieldType::HashMap { .. }
            | FieldType::Set(_)
            | FieldType::Any => return None,
        })
    }

    fn static_hof_array_receiver_concrete_type(
        &mut self,
        receiver: &Expr,
        receiver_ct: Option<&ConcreteType>,
    ) -> Option<ConcreteType> {
        if let Some(ct @ ConcreteType::Array(_)) = receiver_ct {
            return Some(ct.clone());
        }

        if let Expr::Identifier(name, _) = receiver
            && let Some(type_name) = self.tracker_type_name_for_identifier(name)
            && let Some(ann) = type_annotation_from_tracker_name(&type_name)
            && let Some(ct) =
                crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                    self, &ann,
                )
            && matches!(ct, ConcreteType::Array(_))
        {
            return Some(ct);
        }

        if let Expr::MethodCall {
            receiver: inner,
            method,
            args,
            ..
        } = receiver
        {
            let inner_ct = concrete_type_for_expr(self, inner);
            if let Some(ct) =
                self.static_array_hof_result_concrete_type(inner, inner_ct.as_ref(), method, args)
                && matches!(ct, ConcreteType::Array(_))
            {
                return Some(ct);
            }
        }

        let ann = self.infer_expr_type(receiver).ok()?.to_annotation()?;
        let ct =
            crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                self, &ann,
            )?;
        matches!(ct, ConcreteType::Array(_)).then_some(ct)
    }

    fn static_hof_terminal_expr(body: &[Statement]) -> Option<&Expr> {
        body.iter().rev().find_map(|stmt| match stmt {
            Statement::Expression(expr, _) | Statement::Return(Some(expr), _) => Some(expr),
            _ => None,
        })
    }

    fn static_hof_expr_concrete_type(
        &mut self,
        param_types: &HashMap<String, ConcreteType>,
        expr: &Expr,
    ) -> Option<ConcreteType> {
        if let Expr::Identifier(name, _) = expr
            && let Some(ct) = param_types.get(name)
        {
            return Some(ct.clone());
        }

        if let Some(ct) = concrete_type_for_expr(self, expr) {
            return Some(ct);
        }

        match expr {
            Expr::PropertyAccess {
                object, property, ..
            } => {
                let object_ct = self.static_hof_expr_concrete_type(param_types, object)?;
                let type_name = match object_ct {
                    ConcreteType::Struct(named) => named.name_str()?.to_string(),
                    ConcreteType::Enum(named) => named.name_str()?.to_string(),
                    ConcreteType::Array(_) if property == "length" => {
                        return Some(ConcreteType::I64);
                    }
                    _ => return None,
                };
                let resolved = self.resolve_type_name(&type_name);
                let field_type = self
                    .type_tracker
                    .schema_registry()
                    .get(resolved.as_str())
                    .and_then(|schema| schema.get_field(property))
                    .map(|field| field.field_type.clone())?;
                self.field_type_to_static_hof_concrete_type(&field_type)
            }
            Expr::IndexAccess { object, .. } => {
                let object_ct = self.static_hof_expr_concrete_type(param_types, object)?;
                match object_ct {
                    ConcreteType::Array(elem) => Some(*elem),
                    _ => None,
                }
            }
            Expr::BinaryOp {
                left, op, right, ..
            } => {
                use shape_ast::ast::BinaryOp;
                match op {
                    BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::Greater
                    | BinaryOp::Less
                    | BinaryOp::GreaterEq
                    | BinaryOp::LessEq
                    | BinaryOp::FuzzyEqual
                    | BinaryOp::FuzzyGreater
                    | BinaryOp::FuzzyLess
                    | BinaryOp::And
                    | BinaryOp::Or => Some(ConcreteType::Bool),
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Pow
                    | BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::BitShl
                    | BinaryOp::BitShr => {
                        let left_ct = self.static_hof_expr_concrete_type(param_types, left)?;
                        let right_ct = self.static_hof_expr_concrete_type(param_types, right)?;
                        if left_ct == right_ct {
                            Some(left_ct)
                        } else if matches!(
                            (&left_ct, &right_ct),
                            (ConcreteType::F64, ConcreteType::I64)
                                | (ConcreteType::I64, ConcreteType::F64)
                        ) {
                            Some(ConcreteType::F64)
                        } else {
                            None
                        }
                    }
                    BinaryOp::NullCoalesce => self
                        .static_hof_expr_concrete_type(param_types, left)
                        .or_else(|| self.static_hof_expr_concrete_type(param_types, right)),
                    BinaryOp::ErrorContext | BinaryOp::Pipe => None,
                }
            }
            Expr::If(if_expr, _) => {
                let then_ct =
                    self.static_hof_expr_concrete_type(param_types, &if_expr.then_branch)?;
                let else_ct = if_expr.else_branch.as_ref().and_then(|else_branch| {
                    self.static_hof_expr_concrete_type(param_types, else_branch)
                })?;
                (then_ct == else_ct).then_some(then_ct)
            }
            Expr::Conditional {
                then_expr,
                else_expr: Some(else_expr),
                ..
            } => {
                let then_ct = self.static_hof_expr_concrete_type(param_types, then_expr)?;
                let else_ct = self.static_hof_expr_concrete_type(param_types, else_expr)?;
                (then_ct == else_ct).then_some(then_ct)
            }
            Expr::Block(block, _) => block.items.iter().rev().find_map(|item| match item {
                shape_ast::ast::BlockItem::Expression(expr) => {
                    self.static_hof_expr_concrete_type(param_types, expr)
                }
                shape_ast::ast::BlockItem::Statement(Statement::Expression(expr, _))
                | shape_ast::ast::BlockItem::Statement(Statement::Return(Some(expr), _)) => {
                    self.static_hof_expr_concrete_type(param_types, expr)
                }
                _ => None,
            }),
            _ => None,
        }
    }

    fn static_hof_closure_return_concrete_type(
        &mut self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[Statement],
        explicit_return: Option<&shape_ast::ast::TypeAnnotation>,
        arg_types: &[ConcreteType],
    ) -> Option<ConcreteType> {
        if let Some(ann) = explicit_return {
            return crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                self, ann,
            );
        }

        let mut param_types = HashMap::new();
        for (param, ct) in params.iter().zip(arg_types.iter()) {
            if let Some(name) = param.simple_name() {
                param_types.insert(name.to_string(), ct.clone());
            }
        }

        Self::static_hof_terminal_expr(body)
            .and_then(|terminal| self.static_hof_expr_concrete_type(&param_types, terminal))
            .or_else(|| {
                let caller_arg_type_names: Vec<Option<String>> = arg_types
                    .iter()
                    .map(crate::compiler::patterns::binding::concrete_type_tracker_name)
                    .collect();
                let ty = crate::compiler::expressions::closures::infer_closure_body_return_type_with_caller_context(
                    self,
                    params,
                    body,
                    explicit_return,
                    &[],
                    &caller_arg_type_names,
                )?;
                let ann = ty.to_annotation()?;
                crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                    self, &ann,
                )
            })
    }

    fn static_array_hof_result_concrete_type(
        &mut self,
        receiver: &Expr,
        receiver_ct: Option<&ConcreteType>,
        method: &str,
        args: &[Expr],
    ) -> Option<ConcreteType> {
        let receiver_ct = self.static_hof_array_receiver_concrete_type(receiver, receiver_ct)?;
        let ConcreteType::Array(elem) = receiver_ct else {
            return None;
        };
        let elem_ct = (*elem).clone();

        match method {
            "filter" => Some(ConcreteType::Array(Box::new(elem_ct))),
            "map" => {
                let Expr::FunctionExpr {
                    params,
                    body,
                    return_type,
                    ..
                } = args.first()?
                else {
                    return None;
                };
                let arg_types = if params.len() >= 2 {
                    vec![elem_ct, ConcreteType::I64]
                } else {
                    vec![elem_ct]
                };
                let result_ct = self.static_hof_closure_return_concrete_type(
                    params,
                    body,
                    return_type.as_ref(),
                    &arg_types,
                )?;
                Some(ConcreteType::Array(Box::new(result_ct)))
            }
            "flatMap" => {
                let Expr::FunctionExpr {
                    params,
                    body,
                    return_type,
                    ..
                } = args.first()?
                else {
                    return None;
                };
                let result_ct = self.static_hof_closure_return_concrete_type(
                    params,
                    body,
                    return_type.as_ref(),
                    &[elem_ct],
                )?;
                match result_ct {
                    ConcreteType::Array(inner) => Some(ConcreteType::Array(inner)),
                    _ => None,
                }
            }
            "reduce" => {
                let init = args.get(1)?;
                concrete_type_for_expr(self, init)
                    .or_else(|| self.static_hof_expr_concrete_type(&HashMap::new(), init))
            }
            _ => None,
        }
    }

    pub(crate) fn hidden_native_module_binding_name(module_path: &str) -> String {
        format!("__imported_module__::{}", module_path)
    }

    fn stamp_last_expr_from_static_type(&mut self, ty: &Type) -> bool {
        if Self::type_contains_unknown(ty) {
            return false;
        }
        let Some(annotation) = ty.to_annotation() else {
            return false;
        };
        let Some(type_info) = self.type_info_from_annotation(&annotation) else {
            return false;
        };
        self.last_expr_type_info = Some(type_info);
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);
        true
    }

    fn stamp_last_expr_from_static_call_expr(&mut self, name: &str, args: &[Expr], span: Span) {
        if self.last_expr_type_info.is_some() {
            return;
        }
        let call_expr = Expr::FunctionCall {
            name: name.to_string(),
            const_args: Vec::new(),
            args: args.to_vec(),
            named_args: Vec::new(),
            span,
        };
        if let Ok(return_ty) = self.infer_expr_type(&call_expr) {
            self.stamp_last_expr_from_static_type(&return_ty);
        }
    }

    fn current_function_callable_param_return_type(
        &self,
        param_name: &str,
        arg_count: Option<usize>,
    ) -> Option<Type> {
        let function_name = self.current_body_semantic_owner_key()?;
        let Type::Function { params, .. } = self
            .inference_facts
            .function_signature(function_name)?
            .canonicalize()
        else {
            return None;
        };
        let param_idx = self
            .current_function_params
            .iter()
            .position(|param| param.simple_name() == Some(param_name))?;
        let Type::Function {
            params: callable_params,
            returns,
            ..
        } = params.get(param_idx)?.canonicalize()
        else {
            return None;
        };
        if let Some(expected) = arg_count
            && callable_params.len() != expected
        {
            return None;
        }
        let return_ty = *returns;
        if Self::type_contains_unknown(&return_ty) {
            return None;
        }
        Some(return_ty)
    }

    fn ensure_hidden_native_module_binding(&mut self, module_path: &str) -> String {
        let binding_name = Self::hidden_native_module_binding_name(module_path);
        if !self.module_bindings.contains_key(&binding_name) {
            let binding_idx = self.get_or_create_module_binding(&binding_name);
            self.register_extension_module_schema(module_path);
            let module_schema_name = format!("__mod_{}", module_path);
            if self
                .type_tracker
                .schema_registry()
                .get(&module_schema_name)
                .is_some()
            {
                self.set_module_binding_type_info(binding_idx, &module_schema_name);
            }
        }
        binding_name
    }

    fn compile_module_builtin_function_call(
        &mut self,
        builtin_decl: &ModuleBuiltinFunction,
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        if !self
            .is_native_module_export(&builtin_decl.source_module_path, &builtin_decl.export_name)
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "builtin function '{}' has no runtime implementation in module '{}'",
                    builtin_decl.export_name, builtin_decl.source_module_path
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }
        // R8 W9 B1 W17-marshal-return JIT surface-and-stop flag
        // (2026-05-25). `builtin fn` declarations like
        // `from std::core::state use { serialize }` route through this
        // helper which calls `compile_module_namespace_call_on_binding`
        // — emitting a `LoadModuleBinding(idx) + GetFieldTyped(...) +
        // CallValue` sequence whose callee is a `Ptr(HeapKind::ModuleFn)`
        // (see ADR-006 §2.7.26 amendment). At runtime VM-side this
        // routes cleanly through `invoke_module_fn_id_stub` +
        // `project_typed_return`; JIT-side `jit_call_value` ModuleFn
        // arm at `ffi/control/mod.rs:704-715` silently returns TAG_NULL
        // — silent-wrong-output. Set the flag so the JIT preflight
        // refuses and deopts to the bytecode interpreter via the W12
        // `[jit-fallback]` path. Root-cause fix in JIT ModuleFn dispatch
        // (`dispatch_module_fn_call` `todo!()` + the §2.7.10/Q11 kinded
        // handler ABI rebuild) is v0.4 per
        // `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup.
        // Restrict to user-space main compilation. Dep-module bodies
        // execute their internal stdlib calls only when transitively
        // reachable from main; setting the flag during dep-module
        // compilation would poison every program that imports any
        // stdlib (e.g. s1's `let mut sum = 0; for i in 0..100 {...}`
        // pulls in `std::core::remote::__call` during stdlib bootstrap
        // even though main never invokes it).
        if self.module_scope_stack.is_empty() {
            self.program.has_w17_marshal_residual = true;
        }
        let binding_name =
            self.ensure_hidden_native_module_binding(&builtin_decl.source_module_path);
        self.compile_module_namespace_call_on_binding(
            &binding_name,
            &builtin_decl.source_module_path,
            span,
            &builtin_decl.export_name,
            &[],
            args,
        )
    }

    fn resolve_scoped_module_builtin_function(&self, name: &str) -> Option<ModuleBuiltinFunction> {
        if let Some(decl) = self.module_builtin_functions.get(name) {
            return Some(decl.clone());
        }

        for module_path in self.module_scope_stack.iter().rev() {
            let candidate = format!("{}::{}", module_path, name);
            if let Some(decl) = self.module_builtin_functions.get(&candidate) {
                return Some(decl.clone());
            }
            if self.is_native_module_export(module_path, name) {
                return Some(ModuleBuiltinFunction {
                    export_name: name.to_string(),
                    source_module_path: module_path.clone(),
                });
            }
        }
        None
    }

    fn extract_table_schema_from_annotation(
        &mut self,
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<(u32, String)> {
        let shape_ast::ast::TypeAnnotation::Generic { name, args } = ann else {
            return None;
        };
        if name != "Table" || args.len() != 1 {
            return None;
        }

        match &args[0] {
            shape_ast::ast::TypeAnnotation::Basic(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| (schema.id, name.clone())),
            shape_ast::ast::TypeAnnotation::Reference(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| (schema.id, name.to_string())),
            shape_ast::ast::TypeAnnotation::Object(fields) => {
                // Register the inline schema with typed field info so downstream
                // RowView field accesses (`row.open`) can resolve column type
                // and emit typed LoadCol* opcodes / numeric-type hints.
                let typed_fields: Vec<(&str, shape_runtime::type_schema::FieldType)> = fields
                    .iter()
                    .map(|field| {
                        let ft =
                            BytecodeCompiler::type_annotation_to_field_type(&field.type_annotation);
                        (field.name.as_str(), ft)
                    })
                    .collect();
                let schema_id = self
                    .type_tracker
                    .register_inline_object_schema_typed(&typed_fields);
                // Also register field contracts so downstream callable-field
                // unwrapping (e.g. nested `() => Table<{...}>` returns) and
                // any contract-based field lookups see the annotated types.
                let mut contracts = std::collections::HashMap::with_capacity(fields.len());
                for field in fields {
                    contracts.insert(field.name.clone(), field.type_annotation.clone());
                }
                self.type_tracker
                    .register_object_field_contracts(schema_id, contracts);
                let schema_name = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .map(|schema| schema.name.clone())
                    .unwrap_or_else(|| format!("__anon_{}", schema_id));
                Some((schema_id, schema_name))
            }
            _ => None,
        }
    }

    fn extract_object_schema_id_from_annotation(
        &mut self,
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<u32> {
        let shape_ast::ast::TypeAnnotation::Object(fields) = ann else {
            return None;
        };
        // W17.2-C §4.D.5 migration: route through the typed variant
        // with FieldType::Any per field (NOT per-field type lowering
        // via type_annotation_to_field_type — that path changes the
        // schema layout vs the pre-existing Any-typed shape, which
        // breaks downstream consumers that depend on the legacy
        // Any-uniform field layout). The `register_object_field_contracts`
        // call below STILL preserves per-field TypeAnnotation contracts
        // so downstream callable-field unwrapping + JIT lookups see
        // the annotated types. The verification-pass safety net
        // catches via the `__inline_obj_*` transitional row.
        // Per audit §4.D.5 PROPAGATE deferred to v0.4 W17.3+ for the
        // per-field-typed schema layout migration. ADR-006 §2.7.5
        // producer-side stamp preserved at the contract layer
        // (`register_object_field_contracts`).
        let typed_fields: Vec<(&str, shape_runtime::type_schema::FieldType)> = fields
            .iter()
            .map(|field| {
                (
                    field.name.as_str(),
                    shape_runtime::type_schema::FieldType::Any,
                )
            })
            .collect();
        let schema_id = self
            .type_tracker
            .register_inline_object_schema_typed(&typed_fields);
        let mut map = std::collections::HashMap::with_capacity(fields.len());
        for field in fields {
            map.insert(field.name.clone(), field.type_annotation.clone());
        }
        self.type_tracker
            .register_object_field_contracts(schema_id, map);
        Some(schema_id)
    }

    /// WS-9c: derive/register the inline anonymous schema for an unannotated
    /// function whose canonical inferred signature returns a structural object.
    ///
    /// This is intentionally a projection from `InferenceFacts` at the read
    /// site, not a parallel function-name side table. The schema remains
    /// Any-uniform to match object-literal layout, while field contracts carry
    /// the precise inferred field annotations for downstream field lookups.
    pub(crate) fn inferred_return_object_schema_id(&mut self, call_name: &str) -> Option<u32> {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_schema::FieldType;

        let mut candidates = Vec::new();
        let mut push_candidate = |candidate: String| {
            if !candidates.iter().any(|existing| existing == &candidate) {
                candidates.push(candidate);
            }
        };
        push_candidate(call_name.to_string());
        if let Some(scoped) = self.resolve_scoped_module_binding_name(call_name) {
            push_candidate(scoped);
        }
        if let Some((_, tail)) = call_name.rsplit_once("::") {
            push_candidate(tail.to_string());
        }

        let fields: Vec<(String, shape_runtime::type_schema::FieldType)> =
            candidates.iter().find_map(|candidate| {
                let Type::Function { returns, .. } =
                    self.inference_facts.function_signature(candidate)?
                else {
                    return None;
                };
                let Type::Concrete(TypeAnnotation::Object(obj_fields)) = returns.as_ref() else {
                    return None;
                };
                if obj_fields.is_empty() {
                    return None;
                }
                Some(
                    obj_fields
                        .iter()
                        .map(|field| {
                            (
                                field.name.clone(),
                                Self::type_annotation_to_field_type(&field.type_annotation),
                            )
                        })
                        .collect(),
                )
            })?;

        let typed_fields: Vec<(&str, FieldType)> = fields
            .iter()
            .map(|(name, _)| (name.as_str(), FieldType::Any))
            .collect();
        let schema_id = self
            .type_tracker
            .register_inline_object_schema_typed(&typed_fields);
        let mut contracts = HashMap::with_capacity(fields.len());
        for (name, field_ty) in &fields {
            if let Some(ann) = field_type_contract_annotation(field_ty) {
                contracts.insert(name.clone(), ann);
            }
        }
        if !contracts.is_empty() {
            self.type_tracker
                .register_object_field_contracts(schema_id, contracts);
        }
        Some(schema_id)
    }

    /// WS-9c: build a `VariableTypeInfo` for an unannotated function whose
    /// inferred return type is an anonymous structural object.
    fn inline_schema_for_inferred_return(&mut self, call_name: &str) -> Option<VariableTypeInfo> {
        let schema_id = self.inferred_return_object_schema_id(call_name)?;
        let schema_name = self
            .type_tracker
            .schema_registry()
            .get_by_id(schema_id)
            .map(|schema| schema.name.clone())
            .unwrap_or_else(|| format!("__anon_{}", schema_id));
        Some(VariableTypeInfo::known(schema_id, schema_name))
    }

    fn type_info_from_annotation(
        &mut self,
        ann: &shape_ast::ast::TypeAnnotation,
    ) -> Option<VariableTypeInfo> {
        match ann {
            shape_ast::ast::TypeAnnotation::Generic { name, .. } if name == "Table" => self
                .extract_table_schema_from_annotation(ann)
                .map(|(schema_id, type_name)| VariableTypeInfo::datatable(schema_id, type_name)),
            shape_ast::ast::TypeAnnotation::Object(_) => {
                let schema_id = self.extract_object_schema_id_from_annotation(ann)?;
                let schema_name = self
                    .type_tracker
                    .schema_registry()
                    .get_by_id(schema_id)
                    .map(|schema| schema.name.clone())
                    .unwrap_or_else(|| format!("__anon_{}", schema_id));
                Some(VariableTypeInfo::known(schema_id, schema_name))
            }
            shape_ast::ast::TypeAnnotation::Basic(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| VariableTypeInfo::known(schema.id, name.clone()))
                .or_else(|| Self::builtin_scalar_type_info(name)),
            shape_ast::ast::TypeAnnotation::Reference(name) => self
                .type_tracker
                .schema_registry()
                .get(name.as_str())
                .map(|schema| VariableTypeInfo::known(schema.id, name.to_string()))
                .or_else(|| Self::builtin_scalar_type_info(name)),
            // PB2-fix #8 (`let r = inner()` where `inner -> Result<T>`):
            // stamp the binding with the baked wrapper-type-name string so
            // `propagate_assignment_type_to_slot` records it on the slot.
            // `compile_expr_try_operator::stamp_unwrapped_success_type`
            // peels the success arm out of that baked name (already handled
            // by `first_generic_arg_of_baked_name`) and stamps the unwrapped
            // type onto the downstream `let v = r?` binding. Narrow to the
            // two fallible-wrapper names — `Result` and `Option` — so this
            // does not regress other generic returns (`Array<T>` etc. have
            // their own `is_array_type_name` propagation path).
            shape_ast::ast::TypeAnnotation::Generic { name, .. }
                if name == "Result" || name == "Option" =>
            {
                Some(VariableTypeInfo::named(ann.to_type_string()))
            }
            _ => None,
        }
    }

    fn builtin_scalar_type_info(name: &str) -> Option<VariableTypeInfo> {
        shape_runtime::type_system::BuiltinTypes::canonical_script_alias(name)
            .map(|canonical| VariableTypeInfo::named(canonical.to_string()))
    }

    fn type_info_from_inferred_type(&mut self, inferred: &Type) -> Option<VariableTypeInfo> {
        let ann = inferred.to_annotation()?;
        self.type_info_from_annotation(&ann)
    }

    fn table_schema_from_type_info(type_info: &VariableTypeInfo) -> Option<(u32, String)> {
        if type_info.is_datatable() {
            Some((type_info.schema_id?, type_info.type_name.clone()?))
        } else {
            None
        }
    }

    fn table_select_body_expr(body: &[Statement]) -> Option<&Expr> {
        match body {
            [Statement::Expression(expr, _)] => Some(expr),
            [Statement::Return(Some(expr), _)] => Some(expr),
            _ => None,
        }
    }

    fn row_field_projection_column<'a>(expr: &'a Expr, row_param: &str) -> Option<&'a str> {
        let Expr::PropertyAccess {
            object, property, ..
        } = expr
        else {
            return None;
        };
        match object.as_ref() {
            Expr::Identifier(name, _) if name == row_param => Some(property.as_str()),
            _ => None,
        }
    }

    fn static_table_select_columns(
        &self,
        arg: &Expr,
        schema_id: u32,
        type_name: &str,
    ) -> Result<Option<Vec<String>>> {
        let Expr::FunctionExpr { params, body, .. } = arg else {
            return Ok(None);
        };
        if params.len() != 1 {
            return Ok(None);
        }
        let Some(row_param) = params[0].pattern.as_identifier() else {
            return Ok(None);
        };
        let Some(body_expr) = Self::table_select_body_expr(body) else {
            return Ok(None);
        };

        let mut columns = Vec::new();
        match body_expr {
            Expr::Object(entries, _) => {
                if entries.is_empty() {
                    return Ok(None);
                }
                for entry in entries {
                    let ObjectEntry::Field { key, value, .. } = entry else {
                        return Ok(None);
                    };
                    let Some(column) = Self::row_field_projection_column(value, row_param) else {
                        return Ok(None);
                    };
                    if key != column {
                        return Ok(None);
                    }
                    columns.push(column.to_string());
                }
            }
            _ => {
                let Some(column) = Self::row_field_projection_column(body_expr, row_param) else {
                    return Ok(None);
                };
                columns.push(column.to_string());
            }
        }

        let Some(schema) = self.type_tracker.schema_registry().get_by_id(schema_id) else {
            return Err(ShapeError::SemanticError {
                message: format!("Table.select: schema for '{}' is not registered", type_name),
                location: Some(self.span_to_source_location(arg.span())),
            });
        };
        for column in &columns {
            if !schema.has_field(column) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Table.select: type '{}' has no field '{}'",
                        type_name, column
                    ),
                    location: Some(self.span_to_source_location(arg.span())),
                });
            }
        }

        Ok(Some(columns))
    }

    fn value_schema_from_type_info(type_info: &VariableTypeInfo) -> Option<u32> {
        if matches!(type_info.kind, VariableKind::Value) {
            type_info.schema_id
        } else {
            None
        }
    }

    fn extract_table_schema_from_callable_field(
        &mut self,
        receiver_schema_id: u32,
        field_name: &str,
    ) -> Option<(u32, String)> {
        let field_ann = self
            .type_tracker
            .get_object_field_contract(receiver_schema_id, field_name)?
            .clone();
        let shape_ast::ast::TypeAnnotation::Function { params, returns, .. } = field_ann else {
            return None;
        };
        if !params.is_empty() {
            return None;
        }
        self.extract_table_schema_from_annotation(&returns)
    }

    fn is_native_module_export(&self, module_name: &str, export_name: &str) -> bool {
        self.extension_registry
            .as_ref()
            .and_then(|registry| registry.iter().rev().find(|m| m.name == module_name))
            .is_some_and(|module| module.has_export(export_name))
    }

    fn is_native_module_export_available(&self, module_name: &str, export_name: &str) -> bool {
        self.extension_registry
            .as_ref()
            .and_then(|registry| registry.iter().rev().find(|m| m.name == module_name))
            .is_some_and(|module| module.is_export_available(export_name, self.comptime_mode))
    }

    fn ensure_const_specialization(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<Option<(String, usize)>> {
        let Some(const_param_indices) = self.function_const_params.get(name).cloned() else {
            return Ok(None);
        };
        if const_param_indices.is_empty() {
            return Ok(None);
        }

        let original_def =
            self.function_defs
                .get(name)
                .cloned()
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "const specialization requested for unknown function '{}'",
                        name
                    ),
                    location: None,
                })?;

        let mut key_parts = Vec::with_capacity(const_param_indices.len());
        let mut const_bindings = Vec::with_capacity(const_param_indices.len());
        for idx in const_param_indices {
            let Some(arg) = args.get(idx) else {
                continue;
            };
            let Some(fingerprint) = const_expr_fingerprint(arg) else {
                return Ok(None);
            };
            let Some(literal) = const_expr_literal(arg) else {
                return Ok(None);
            };
            let Some(value) = literal_const_slot(literal) else {
                return Ok(None);
            };
            let param_name = original_def
                .params
                .get(idx)
                .and_then(|p| p.simple_name())
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "const specialization for '{}' requires a named const parameter at position {}",
                        name,
                        idx + 1
                    ),
                    location: None,
                })?
                .to_string();
            key_parts.push(format!("{}={}", param_name, fingerprint));
            const_bindings.push((param_name, value));
        }

        if const_bindings.is_empty() {
            return Ok(None);
        }

        let specialization_key = format!("{}::{}", name, key_parts.join("|"));
        if let Some(&existing_idx) = self.const_specializations.get(&specialization_key) {
            let existing_name = self
                .program
                .functions
                .get(existing_idx)
                .map(|f| f.name.clone())
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "const specialization cache for '{}' points at missing function index {}",
                        name, existing_idx
                    ),
                    location: None,
                })?;
            return Ok(Some((existing_name, existing_idx)));
        }

        let mut specialized_def = original_def;
        specialized_def.name = format!("{}__const_{}", name, self.next_const_specialization_id);
        self.next_const_specialization_id = self.next_const_specialization_id.saturating_add(1);
        if let Some(module_path) = specialized_def.declaring_module_path.clone() {
            for ann in &mut specialized_def.annotations {
                if ann.name.contains("::") {
                    continue;
                }
                let qualified = Self::qualify_module_symbol(&module_path, &ann.name);
                if self.program.compiled_annotations.contains_key(&qualified) {
                    ann.name = qualified;
                }
            }
        }
        let specialized_name = specialized_def.name.clone();

        self.specialization_const_bindings
            .insert(specialized_name.clone(), const_bindings);
        self.register_function(&specialized_def)?;
        let specialized_idx =
            self.find_function(&specialized_name)
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "const specialization failed to register function '{}'",
                        specialized_name
                    ),
                    location: None,
                })?;
        self.const_specializations
            .insert(specialization_key.clone(), specialized_idx);

        // This value-const specialization introduces no new semantic type
        // arguments. Push a declaration-only frame so an enclosing exact
        // specialization cannot leak into its body.
        let specialization_overlay =
            Self::declaration_only_specialization_overlay(name, &specialized_def);
        let specialization_overlay_guard = self
            .specialization_type_overlays
            .enter(specialization_overlay);
        let compile_result = self.compile_function(&specialized_def);
        drop(specialization_overlay_guard);

        if let Err(err) = compile_result {
            self.specialization_const_bindings.remove(&specialized_name);
            self.const_specializations.remove(&specialization_key);
            return Err(err);
        }

        Ok(Some((specialized_name, specialized_idx)))
    }

    /// Compile a function call expression
    pub(super) fn compile_expr_function_call(
        &mut self,
        name: &str,
        const_args: &[Expr],
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        use crate::compiler::template_specialization::pseudo_tuple::{
            REMOTE_ARG_PACK_MARKER, REMOTE_CALL_RAISING_FN, REMOTE_IMPL_REF_MARKER,
        };
        // ADR-009 E4 S5 (CP3) — a stray `@remote` weave marker that reached
        // bytecode compilation is a LOUD compile error. The decision-hook weave
        // SUBSTITUTES `__remote_impl_ref()` / `__remote_arg_pack()` at lowering
        // time (`pseudo_tuple::substitute_remote_markers`), so a marker surviving
        // to here is a reference OUTSIDE a `@remote` before-hook weave (an
        // internal error or a misuse) — never a silent misdispatch of arg[0] as
        // the callee, and never a fabricated value (E4-D3 fail-loud).
        if name == REMOTE_IMPL_REF_MARKER || name == REMOTE_ARG_PACK_MARKER {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "`{name}()` is a reserved `@remote` weave marker: it is substituted by the \
                     decision-hook weave at lowering time and is never callable directly. A bare \
                     `{name}()` reference outside the `@remote` before-hook is an internal error \
                     or a misuse — remove it."
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }
        // ADR-009 E4 S5 (CP4, E4-D3) — the `@remote` synthesized short-circuit
        // call the decision weave emits: `__call_raising(addr, <impl-shadow>,
        // <arg-pack>)`, where the callee (arg[1]) is the hygienic impl shadow —
        // an unspellable SOH-prefixed name in `template_weave_shadow_names`, so
        // this interception is unreachable from user code. Elaborate it at the
        // shadow's BARE R (the raising primitive delivers the callee's value at
        // its declared return type and RAISES on failure; Q26) — NEVER the
        // wrapper's return type (no self-recursion). If the impl-ref marker
        // failed to substitute, arg[1] would still be `__remote_impl_ref()` and
        // the stray-marker reject above fires — there is no `?? args[0]` fallback.
        if name == REMOTE_CALL_RAISING_FN
            && args.len() == 3
            && let Expr::Identifier(shadow_name, _) = &args[1]
            && self.template_weave_shadow_names.contains(shadow_name)
        {
            let shadow_name = shadow_name.clone();
            return self.compile_remote_raising_short_circuit(&shadow_name, args, span);
        }
        // Numeric-conversion §4 literal adoption (call-argument widening, THE
        // RULE user 2026-06-01): a bare int literal passed to a `number`(f64)
        // parameter IS the number literal (`f(5)` where `fn f(x: number)` ⇒
        // `5.0`). Re-lower each such argument to a `Number` literal BEFORE any
        // downstream `compile_call_args`, so the argument carries Float64 bits
        // and the callee's number param slot is not fed an Int64 constant that
        // the call site bit-reinterprets as f64 (`5` → `2.5e-323`). Compile-time
        // literal re-typing keyed on the callee's DECLARED param annotations
        // (`self.function_defs`); a non-literal int arg is NOT rewritten — the
        // p-var `int`-is-not-`number` rejection stays a compile error. Only the
        // direct-named-user-function path resolves param annotations here; the
        // imported stdlib-wrapper path resolves through its scoped module
        // binding name; the indirect-callable path keeps the raw args.
        let scoped_name = self.resolve_scoped_module_binding_name(name);
        let param_widening_def = self.function_defs.get(name).or_else(|| {
            scoped_name
                .as_deref()
                .and_then(|scoped| self.function_defs.get(scoped))
        });
        let widened_args: Option<Vec<Expr>> = param_widening_def.and_then(|def| {
            let params = &def.params;
            let mut changed = false;
            let mut out: Vec<Expr> = Vec::with_capacity(args.len());
            for (i, arg) in args.iter().enumerate() {
                let widened = params.get(i).and_then(|p| {
                    p.type_annotation.as_ref().and_then(|ann| {
                        crate::compiler::literal_widen::widen_int_literal_for_annotation(arg, ann)
                    })
                });
                match widened {
                    Some(w) => {
                        changed = true;
                        out.push(w);
                    }
                    None => out.push(arg.clone()),
                }
            }
            if changed { Some(out) } else { None }
        });
        let args: &[Expr] = widened_args.as_deref().unwrap_or(args);

        // Reject comptime-only builtins outside of comptime blocks.
        // These functions are only available inside `comptime { }` blocks.
        if Self::is_comptime_only_builtin(name) && !self.comptime_mode {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "'{}' is a comptime-only builtin and can only be called inside a `comptime {{ }}` block",
                    name
                ),
                location: Some(self.span_to_source_location(span)),
            });
        }

        // WF-3E (D7): a bare call `snapshot()` under `use std::core::snapshot`
        // where the module's last segment collides with a same-named export.
        // The bare name resolves to the module NAMESPACE binding (a
        // predeclared module object that a pure Shape-source stdlib module
        // like `snapshot` never populates on a local run), so the callable
        // path below would emit `LoadModuleBinding + CallValue` and consume an
        // uninitialised Bool sentinel at runtime
        // (`call_value_immediate_nb: ... got Bool`). When `name` is a
        // namespace-import alias (from `module_scope_sources`), is not shadowed
        // by a local/closure-capture, and the imported module exports a
        // function of the SAME name, rewrite to the qualified form
        // `name::name(..)` — the exact path the documented
        // `snapshot::snapshot()` takes (resolves to the real function via the
        // module schema registry / `find_function`). No Bool-default
        // consumption; a genuine resolution. The `{name}::{name}` existence
        // gate keeps unrelated shadowing (`use std::core::math` + a user
        // `fn math()`, where `math` has no `math` export) on the normal path.
        if self.resolve_local(name).is_none()
            && !self.mutable_closure_captures.contains_key(name)
            && self.is_module_namespace_name(name)
        {
            let canonical = self
                .resolve_canonical_module_path(name)
                .unwrap_or_else(|| name.to_string());
            let canonical_scoped = format!("{}::{}", canonical, name);
            let exports_same_named = self.module_member_is_exported(&canonical, name) == Some(true)
                || self.find_function(&canonical_scoped).is_some();
            if exports_same_named {
                return self.compile_module_namespace_call(name, span, name, const_args, args);
            }
        }

        // Check locals FIRST — function parameters (and other local variables holding
        // callable values) must take priority over global function lookup.  Without this,
        // `fn apply(f, x) { f(x) }` would fail because `find_function("f")` returns None
        // and the code falls through to "Undefined function" error.
        if self.resolve_local(name).is_some()
            || self.mutable_closure_captures.contains_key(name)
            || self.resolve_scoped_module_binding_name(name).is_some()
        {
            self.reject_const_args_for_non_generic_call(name, const_args, span)?;
            // R8 W9 B1 W17-marshal-return JIT surface-and-stop flag
            // (2026-05-25). Direct call to an imported stdlib function —
            // the callee resolves via `resolve_scoped_module_binding_name`
            // and loads a `Ptr(HeapKind::ModuleFn)` value. At runtime the
            // `CallValue` opcode dispatches via the VM-side
            // `call_value_immediate_nb` ModuleFn arm, which routes to
            // `invoke_module_fn_id_stub` + `project_typed_return` and
            // surfaces cleanly when the typed-return arm hits the
            // W17-marshal-return-arms catch-all at
            // `crates/shape-vm/src/executor/vm_impl/modules.rs:74`.
            //
            // The JIT-side `jit_call_value` ModuleFn arm at
            // `crates/shape-jit/src/ffi/control/mod.rs:704-715` instead
            // returns `TAG_NULL` (= the `-1407374883553280` NaN-box null
            // pattern) silently with only a `tracing::debug!` line —
            // swallowing the W17-marshal-return surface and producing
            // silent-wrong-output (VM=ec1 SURFACE / JIT=ec0 garbage on
            // `print(serialize([1.0,2.0,3.0]).len())`).
            //
            // Mark the program so `JITExecutor::execute_with_jit` deopts
            // to the bytecode interpreter via the existing W12
            // `[jit-fallback]` path — VM == JIT semantics restored via
            // path-convergence. Mirrors R8 W7 G.5 V2-verifier preflight
            // + R8 W8 imported-const-inline surface-and-stop precedents.
            // Root-cause fix in JIT ModuleFn dispatch
            // (`dispatch_module_fn_call` `todo!()` + the §2.7.10/Q11
            // kinded handler ABI rebuild) is v0.4 per
            // `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup.
            // Restrict to user-space main compilation (see
            // `compile_module_builtin_function_call` below for the
            // dep-module-bootstrap rationale).
            if self.resolve_scoped_module_binding_name(name).is_some()
                && self.module_scope_stack.is_empty()
            {
                self.program.has_w17_marshal_residual = true;
            }
            let expected_param_modes = if let Some(local_idx) = self.resolve_local(name) {
                self.local_callable_pass_modes.get(&local_idx).cloned()
            } else if let Some(scoped_name) = self.resolve_scoped_module_binding_name(name) {
                self.module_bindings
                    .get(&scoped_name)
                    .and_then(|binding_idx| {
                        self.module_binding_callable_pass_modes
                            .get(binding_idx)
                            .cloned()
                    })
            } else {
                None
            };
            let return_reference_summary = self.function_return_reference_summary_for_name(name);
            // Use compile_expr_identifier to correctly load the callee value,
            // handling ref_locals (DerefLoad), mutable closure captures (LoadClosure), etc.
            self.compile_expr_identifier(name, span)?;

            let writebacks = self.compile_call_args(args, expected_param_modes.as_deref())?;

            // Phase F: emit `CallFunctionIndirect` when the callee is a
            // typed callable (`Function<A, R>` parameter or local binding
            // with known callable pass modes) and fits `u16`. The arity
            // travels in the operand so the runtime skips the extra
            // `PushConst` round-trip, and the JIT can pick a
            // `call_indirect` signature from the inferred
            // `FunctionTypeId`. Fallback is the legacy `CallValue` path
            // which reads arity from the stack.
            let prefers_indirect =
                expected_param_modes.is_some() && args.len() <= u16::MAX as usize;
            if prefers_indirect {
                self.emit(Instruction::new(
                    OpCode::CallFunctionIndirect,
                    Some(Operand::Count(args.len() as u16)),
                ));
            } else {
                let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));
                self.emit(Instruction::simple(OpCode::CallValue));
            }
            if !writebacks.is_empty() {
                let result_local = self.declare_temp_local("__call_value_result_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));
                for (shadow_local, binding_idx) in writebacks {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(shadow_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            // U4-6 callable-return deletion: derive the return type of a
            // callable binding structurally from `InferenceFacts` or a retained
            // closure body peek, then render it only for the residual tracker
            // display-name stamp. No slot-indexed return string map remains.
            let tracked_callable_return_type = self
                .callable_binding_return_type(name, Some(args.len()))
                .or_else(|| {
                    self.current_function_callable_param_return_type(name, Some(args.len()))
                });
            let tracked_callable_rt: Option<String> = tracked_callable_return_type
                .as_ref()
                .and_then(Self::inferred_type_to_hint_name);
            if let Some(rt_name) = tracked_callable_rt.as_ref() {
                match rt_name.as_str() {
                    // U4-4: numeric register stamps replaced by `last_expr_type_info`
                    // name stamps (same shape as the width-aware-int arm). The
                    // numeric storage hint is re-derived from this name (or the
                    // resolved Type) downstream.
                    "int" | "number" | "decimal" => {
                        self.last_expr_type_info = Some(
                            crate::type_tracking::VariableTypeInfo::named(rt_name.clone()),
                        );
                    }
                    other
                        if shape_runtime::type_system::BuiltinTypes::is_integer_type_name(
                            other,
                        ) =>
                    {
                        // Width-aware ints — fall through; the i32/i16/etc.
                        // names round-trip via type_info.
                        self.last_expr_type_info = Some(
                            crate::type_tracking::VariableTypeInfo::named(other.to_string()),
                        );
                    }
                    "string" | "bool" | "char" => {
                        self.last_expr_type_info = Some(
                            crate::type_tracking::VariableTypeInfo::named(rt_name.clone()),
                        );
                    }
                    _ => {}
                }
            }
            if let Some(return_reference_summary) = return_reference_summary {
                self.set_last_expr_reference_result(return_reference_summary.mode, true);
            } else if let Some(borrow_mode) = self.function_declares_borrow_return(name) {
                // ADR-006 §2.7.30 (GapA): a `-> &T` callee with no param-reborrow
                // summary (the PromotedCell ReturnSlot floor) returns a reference
                // value; mark it auto-deref so value position reads THROUGH it.
                self.set_last_expr_reference_result(borrow_mode, true);
                // The returned reference rides the §2.7.30 escape-promote
                // `PromotedCell` carrier, which the JIT has no lowering for (it
                // models refs as per-function stack-cell/field addresses only and
                // would read the raw reference pointer). Force whole-program JIT
                // deopt to the interpreter, which resolves the referent soundly.
                self.program.has_reference_escape_promotion = true;
            } else {
                self.clear_last_expr_reference_result();
            }

            // cluster-2-cw-IB-class-b (2026-05-16, supervisor R3 binding-
            // ratified): value-call return-`ConcreteType` classification
            // at the bytecode-emission layer. ADR-006 §2.7.5 stamp-at-
            // compile-time discipline.
            //
            // When the callee resolves to a local closure binding with a
            // retained body peek (populated at let-binding time by
            // `update_callable_binding_from_expr`), re-run the closure-
            // body return-type inference WITH the caller-context arg
            // types injected as typed-array param hints. If the
            // inference yields a recognised scalar/Array return name,
            // convert it to a `ConcreteType` and stamp the side-table
            // `value_call_return_concrete_types[(call_span,
            // current_function)]`. The MIR conduit's value-call
            // destination pass then projects this onto
            // `top_level_local_concrete_types[dst_slot]` /
            // `function_local_concrete_types[fn_idx][dst_slot]`, the
            // JIT-MIR `slot_kinds` projection picks up the matching
            // `NativeKind`, and downstream consumers (`print`,
            // BinaryOp, etc.) reach their kinded dispatch paths.
            //
            // Class B fixture (inventory §B.2): `let xs: Array<int> =
            // [..]; let f = |inner| inner.sum(); print(f(xs))`. Pre-fix:
            // VM=15 / JIT=NotImplemented(SURFACE, print operand NK=None).
            // Post-fix: VM=15 / JIT=15 (VM == JIT load-bearing).
            //
            // No tag-bit decode, no Bool-default fallback, no fabricated
            // default — when:
            //   • The callee is not a local closure binding, OR
            //   • No retained body peek exists (closure was passed in
            //     from elsewhere, e.g. function parameter), OR
            //   • The closure body's terminal expression cannot be
            //     classified against the caller-context-seeded
            //     param_types (the inference returns None), OR
            //   • The classified return name cannot be mapped back to a
            //     ConcreteType,
            // the side-table receives no entry and the destination slot
            // stays `Void` per §2.7.5.1 / §2.7.7 #9 — the JIT then
            // surfaces honestly at the print dispatch site rather than
            // fabricating a kind.
            // Resolve the closure body peek from either the local slot
            // map or the module-binding slot map. Locals take priority
            // (mirrors the `tracked_callable_rt` chain above).
            let closure_peek: Option<crate::compiler::ClosureBodyPeek> =
                if let Some(local_idx) = self.resolve_local(name) {
                    self.local_callable_closure_bodies.get(&local_idx).cloned()
                } else if let Some(scoped) = self.resolve_scoped_module_binding_name(name) {
                    self.module_bindings.get(&scoped).and_then(|idx| {
                        self.module_binding_callable_closure_bodies
                            .get(idx)
                            .cloned()
                    })
                } else {
                    self.module_bindings.get(name).and_then(|idx| {
                        self.module_binding_callable_closure_bodies
                            .get(idx)
                            .cloned()
                    })
                };
            if let Some(peek) = closure_peek {
                {
                    // Resolve the caller-context arg type names per
                    // argument expression. `concrete_type_for_expr` is
                    // the same resolver the rest of the bytecode-
                    // emission layer uses (covers tracker-recorded
                    // primitives + typed-array bindings via whole-binding
                    // `ConcreteType::Array(elem)` records.
                    // U4-5b: keep the caller-arg ConcreteTypes STRUCTURALLY so
                    // the closure-body param seed (below) reads each arg's array
                    // element type directly from `ConcreteType::Array(elem)` —
                    // never by stripping a `"Vec<elem>"` display string. The
                    // string vec is retained ONLY for the closure-body
                    // mini-inferencer ABI (`..._with_caller_context`, a T3-wave
                    // target), not as a type source.
                    let caller_arg_concrete_types: Vec<Option<shape_value::v2::ConcreteType>> = args
                        .iter()
                        .map(|arg_expr| {
                            crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(self, arg_expr)
                        })
                        .collect();
                    let caller_arg_type_names: Vec<Option<String>> = caller_arg_concrete_types
                        .iter()
                        .map(|ct| {
                            ct.as_ref()
                                .and_then(|ct| {
                                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(ct)
                                })
                                .and_then(|ann| {
                                    crate::compiler::BytecodeCompiler::tracked_type_name_from_annotation(&ann)
                                })
                        })
                        .collect();

                    // Run the closure-body return-type inference with
                    // the caller-context arg types. The inference is
                    // cheap (AST walk over the closure body, no
                    // bytecode emission); running unconditionally covers
                    // both:
                    //   (a) The Class B case: the closure param is
                    //       inferred-typed at the call site (no
                    //       annotation, no body-literal pairing), so
                    //       the let-binding-time inference returned
                    //       None and `tracked_callable_rt` is None.
                    //   (b) The let-binding-time-already-resolved case:
                    //       e.g. `let f = || 15` — the body's terminal
                    //       Literal(Int) is enough at let-binding time,
                    //       `tracked_callable_rt = Some("int")`. Even
                    //       here, the side-table must be populated so
                    //       the JIT-MIR conduit's value-call destination
                    //       pass can stamp `concrete_types[dst]` (the
                    //       let-binding-time tracker recorded only the
                    //       bytecode-side `last_expr_*`, which doesn't
                    //       reach the JIT). The two paths converge on
                    //       the same `ConcreteType::I64` answer here.
                    {
                        // Prefer the let-binding-time result when
                        // present (it consulted the closure body
                        // without needing caller-context); fall through
                        // to the caller-context inference when the
                        // let-binding-time inference returned None.
                        let inferred = tracked_callable_rt
                            .as_ref()
                            .cloned()
                            .or_else(|| {
                                crate::compiler::expressions::closures::infer_closure_body_return_type_name_with_caller_context(
                                    self,
                                    &peek.params,
                                    &peek.body,
                                    peek.return_type.as_ref(),
                                    &[],
                                    &caller_arg_type_names,
                                )
                            });
                        if let Some(rt_name) = inferred {
                            // Map the return name to a ConcreteType.
                            // Mirrors the `tracked_type_name_from_
                            // annotation` → ConcreteType chain used by
                            // `concrete_type_for_expr`. Scalars are
                            // handled directly; `Vec<T>` returns are
                            // not supported here (the typed-array-
                            // returning closure case is Class C's
                            // sibling territory).
                            let ct: Option<shape_value::v2::ConcreteType> = match rt_name.as_str() {
                                "int" | "i64" => Some(shape_value::v2::ConcreteType::I64),
                                "i32" => Some(shape_value::v2::ConcreteType::I32),
                                "i16" => Some(shape_value::v2::ConcreteType::I16),
                                "i8" => Some(shape_value::v2::ConcreteType::I8),
                                "u64" => Some(shape_value::v2::ConcreteType::U64),
                                "u32" => Some(shape_value::v2::ConcreteType::U32),
                                "u16" => Some(shape_value::v2::ConcreteType::U16),
                                "u8" => Some(shape_value::v2::ConcreteType::U8),
                                "number" | "f64" => Some(shape_value::v2::ConcreteType::F64),
                                "bool" => Some(shape_value::v2::ConcreteType::Bool),
                                "string" => Some(shape_value::v2::ConcreteType::String),
                                "decimal" => Some(shape_value::v2::ConcreteType::Decimal),
                                "bigint" => Some(shape_value::v2::ConcreteType::BigInt),
                                "DateTime" => Some(shape_value::v2::ConcreteType::DateTime),
                                _ => None,
                            };
                            if let Some(ct) = ct {
                                self.program
                                    .value_call_return_concrete_types
                                    .insert((span, self.current_function), ct);

                                // cluster-2-cw-IB-class-b (closure-body
                                // typed-array param seed): retroactively
                                // populate `mir.local_typed_array_element_types`
                                // for the closure body's MIR slot
                                // corresponding to each typed-array
                                // caller-context arg. The MIR-side
                                // conduit's empty-typed-array-seed
                                // pass at `helpers.rs:623` consumes
                                // this map at
                                // `propagate_concrete_types_through_mir`
                                // time (which runs AFTER bytecode
                                // emission completes) to stamp
                                // `concrete_types[inner_slot] =
                                // Array(elem)` for the closure body.
                                // The JIT-MIR's `slot_kinds`
                                // projection then picks up
                                // `Ptr(TypedArray)` for `inner` and
                                // dispatches `.len()` /
                                // `.sum()` through the kinded fast
                                // path, returning raw scalar bits
                                // (Int64=15 for our fixture) instead
                                // of TAG_NULL.
                                //
                                // Without this, the closure body's
                                // JIT compilation has no type info
                                // for `inner` and the method
                                // dispatch returns TAG_NULL — the
                                // outer print would then read
                                // TAG_NULL bits and print garbage
                                // even with the destination kind
                                // correctly stamped Int64.
                                if let Some(closure_fn_idx) = peek.function_index {
                                    // Wrap in a block to allow early
                                    // exit via `break` for skip cases
                                    // (Arc shared / mir missing).
                                    'seed_block: {
                                        let Some(func) =
                                            self.program.functions.get_mut(closure_fn_idx)
                                        else {
                                            break 'seed_block;
                                        };
                                        let Some(mir_data_arc) = func.mir_data.as_mut() else {
                                            break 'seed_block;
                                        };
                                        // `Arc::get_mut` returns
                                        // `Some(&mut T)` only when
                                        // the strong-count is 1 —
                                        // the bytecode-emission
                                        // stage's invariant for
                                        // closure-body MIR Arcs (no
                                        // other clone exists yet
                                        // since content-addressed
                                        // program build runs later).
                                        // When this invariant is
                                        // broken (e.g. an upstream
                                        // change clones the Arc
                                        // before bytecode emission
                                        // completes), the propagation
                                        // is skipped; the side-table
                                        // stamping above still
                                        // applies, so the print
                                        // dispatch routes to
                                        // `jit_print_i64` — only
                                        // the closure body's typed-
                                        // array param seed is
                                        // missed.
                                        let Some(mir_data) = std::sync::Arc::get_mut(mir_data_arc)
                                        else {
                                            break 'seed_block;
                                        };
                                        // Match closure-body param
                                        // slots to caller-context arg
                                        // types. The MIR's
                                        // `param_slots` align 1:1
                                        // with the closure literal's
                                        // params list (no captures
                                        // interleaved for value-call
                                        // shape; the captures-as-
                                        // leading-args ABI is for the
                                        // trampoline closure-call
                                        // path, which doesn't fire
                                        // here per `vm_captures=false`
                                        // in the FAST PATH).
                                        for (param_idx, slot) in
                                            mir_data.mir.param_slots.clone().iter().enumerate()
                                        {
                                            // U4-5b: read the caller arg's array
                                            // element type STRUCTURALLY from its
                                            // recorded `ConcreteType::Array(elem)`.
                                            // The element ConcreteType IS the
                                            // proof (ADR-006 §2.7.5); no
                                            // `"Vec<elem>"` display-string strip /
                                            // re-parse. A non-array (or
                                            // unresolved) caller arg yields
                                            // nothing to seed (surface-and-stop
                                            // preserved). Only typed-array element
                                            // kinds (scalars with a typed-array
                                            // carrier) reach
                                            // `local_typed_array_element_types`;
                                            // `or_insert` skips a struct/object
                                            // element exactly as the kind-bounded
                                            // string match did.
                                            let Some(Some(shape_value::v2::ConcreteType::Array(
                                                elem,
                                            ))) = caller_arg_concrete_types.get(param_idx)
                                            else {
                                                continue;
                                            };
                                            let elem_ct = (**elem).clone();
                                            if crate::compiler::v2_typed_emission::should_use_typed_array(
                                                &elem_ct,
                                            )
                                            .is_some()
                                            {
                                                mir_data
                                                    .mir
                                                    .local_typed_array_element_types
                                                    .entry(*slot)
                                                    .or_insert(elem_ct);
                                            }
                                        }
                                    }
                                }

                                // U4-4: stamp `last_expr_type_info` from the
                                // tracked callable return-type name so a
                                // downstream `let`-binding / storage-hint sees
                                // it. The deleted `last_expr_numeric_type`
                                // register stamp is gone — a binop on the call
                                // result derives its numeric kind from the one
                                // resolved Type via `numeric_type_of`.
                                match rt_name.as_str() {
                                    "int" | "number" | "decimal" => {
                                        self.last_expr_type_info = Some(
                                            crate::type_tracking::VariableTypeInfo::named(
                                                rt_name.clone(),
                                            ),
                                        );
                                    }
                                    other if shape_runtime::type_system::BuiltinTypes::is_integer_type_name(other) => {
                                        self.last_expr_type_info =
                                            Some(crate::type_tracking::VariableTypeInfo::named(
                                                other.to_string(),
                                            ));
                                    }
                                    "string" | "bool" | "char" => {
                                        self.last_expr_type_info = Some(
                                            crate::type_tracking::VariableTypeInfo::named(
                                                rt_name.clone(),
                                            ),
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }

            return Ok(());
        }

        // Check for user-defined functions (after locals — function parameters take priority)
        if let Some(func_idx) = self.find_function(name) {
            let resolved_name = self.program.functions[func_idx].name.clone();

            // Check if this function was removed by a comptime annotation handler.
            if self.removed_functions.contains(&resolved_name)
                || self.removed_functions.contains(name)
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "function '{}' was removed by a comptime annotation handler and cannot be called",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            let is_comptime_fn = self
                .function_defs
                .get(&resolved_name)
                .or_else(|| self.function_defs.get(name))
                .map(|def| def.is_comptime)
                .unwrap_or(false);
            if is_comptime_fn && !self.comptime_mode {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "'{}' is declared as `comptime fn` and can only be called from comptime contexts",
                        name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            let mut call_name = resolved_name;
            let mut call_func_idx = func_idx;

            // BUG3 — free-function monomorphization wiring.
            //
            // When the callee is a generic function (`fn inner<T>(x: T) { ... }`)
            // and the call-site args resolve to concrete types, produce (or
            // reuse) a `inner::<concrete>` specialization and redirect the
            // call to it. Otherwise the call would land on the empty
            // template body (generic bodies are intentionally skipped in
            // `compile_function`) and run off the end of the bytecode,
            // blowing the VM call stack.
            //
            // The cycle detector in `ensure_monomorphic_function` prevents
            // transitive re-entry on the same `(fn_name, type_args)` pair
            // if a dispatch helper ever tries to resolve the specialization
            // from inside its own body. On a soft failure (unresolved type
            // args, cycle, benign compile error) we fall back to the
            // unspecialized callee — the caller already surfaces a clean
            // diagnostic when the body is empty.
            //
            // Phase 3a: a hard error (trait-bound violation) is propagated
            // up so the user sees a precise diagnostic instead of a
            // recursion / stack-overflow at runtime.
            if let Some(specialized_idx) =
                self.try_monomorphize_free_function_call(&call_name, const_args, args, span)?
            {
                call_func_idx = specialized_idx;
                call_name = self.program.functions[call_func_idx].name.clone();
            } else if let Some(specialized_idx) =
                self.try_specialize_implicit_generic_free_function_call(&call_name, args, span)?
            {
                call_func_idx = specialized_idx;
                call_name = self.program.functions[call_func_idx].name.clone();
            } else if !self.deferring_uninstantiated_template_body
                && self
                    .function_defs
                    .get(&call_name)
                    .and_then(|d| d.type_params.as_ref())
                    .is_some_and(|tps| tps.iter().any(|tp| !tp.is_const()))
            {
                // Soundness: the callee is a generic function and
                // monomorphization could not resolve a concrete specialization
                // from the call-site arguments. Generic function bodies are
                // intentionally skipped in `compile_function` (their AST is
                // kept only as a substitution template), so emitting a `Call`
                // onto this index would dispatch into a zero-instruction body
                // — the VM runs off the end and hangs. A type argument that
                // cannot be inferred is a compile error, not a silent
                // fall-through. A self-recursive generic call resolves to its
                // specialization's index above (`ensure_monomorphic_function`
                // caches before compiling the body), so it never reaches here.
                //
                // ADR-009 C3 #14 (slice 4, S4a): guarded by
                // `deferring_uninstantiated_template_body` — the SAME
                // discipline as the implicit-generic sibling branch below.
                // Inside a deferred implicit-generic TEMPLATE body the blob is
                // DEAD (its AST is re-emitted per concrete call site with
                // proven types, where this error still fires for real gaps);
                // a generic callee whose type args come from the template's
                // own unresolved params must not hard-fail the dead blob.
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "cannot infer type argument(s) for generic function '{}' from the call-site arguments — annotate the arguments or call with values whose types are statically known",
                        call_name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            } else if !self.deferring_uninstantiated_template_body
                && self.function_defs.get(&call_name).is_some_and(|def| {
                    self.is_uninstantiated_implicit_generic(def)
                        && self.implicit_generic_body_requires_concrete_emission(def)
                })
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "cannot infer concrete parameter type(s) for implicit-generic function '{}' from this call site",
                        call_name
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            let total_arity = self.program.functions[call_func_idx].arity as usize;
            let (required_arity, effective_total_arity) = self
                .function_arity_bounds
                .get(&call_name)
                .copied()
                .unwrap_or((total_arity, total_arity));
            let actual_arity = args.len();
            if actual_arity < required_arity || actual_arity > effective_total_arity {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Function '{}' expects between {} and {} arguments, got {}",
                        name, required_arity, effective_total_arity, actual_arity
                    ),
                    location: Some(self.span_to_source_location(span)),
                });
            }

            if let Some(const_param_indices) = self.function_const_params.get(&call_name).cloned() {
                for idx in const_param_indices {
                    if idx >= actual_arity {
                        continue;
                    }
                    let arg = &args[idx];
                    if !is_compile_time_const_expr(arg) {
                        return Err(ShapeError::SemanticError {
                            message: format!(
                                "Function '{}' parameter #{} is declared `const` and requires a compile-time constant argument",
                                name,
                                idx + 1
                            ),
                            location: Some(self.span_to_source_location(arg.span())),
                        });
                    }
                }

                if let Some((specialized_name, specialized_idx)) =
                    self.ensure_const_specialization(&call_name, args)?
                {
                    call_name = specialized_name;
                    call_func_idx = specialized_idx;
                }
            }

            let ref_params = self.program.functions[call_func_idx].ref_params.clone();
            let ref_mutates = self.program.functions[call_func_idx].ref_mutates.clone();
            let pass_modes = Self::pass_modes_from_ref_flags(&ref_params, &ref_mutates);
            let return_reference_summary =
                self.function_return_reference_summary_for_name(&call_name);

            // Sweep phase 3c.x: bidirectional inference for `any`-typed
            // callable params on free user functions. When the callee has
            // an `any`-annotated param at position k AND args[k] is a
            // closure literal AND the other concrete-typed args' types
            // determine the closure's param types, install
            // `pending_closure_param_types` so the closure compile path
            // attaches concrete annotations to its user params (`|x, y|`
            // → `|x: int, y: int|`). See
            // `apply2(|x, y| x + y, 2, 3)` — without this, `x + y` fails
            // strict typing as `unknown + unknown`.
            // Wave 1a PART B: usage-driven closure seeding for UNANNOTATED
            // callable params. When `fn apply2(f, x, y) { f(x, y) }` USES `f`
            // as a callable, whole-program inference resolved `f`'s type to a
            // concrete `fn(int,int)->_`; the call site `apply2(|a,b| a*b, …)`
            // seeds `|a,b|` from that inferred signature. This is the
            // higher-ranked extension of PART A. It supersedes the legacy
            // `any`-annotation heuristic below; the heuristic is only consulted
            // as a fallback when inference produced no concrete signature.
            let seeded_from_inference =
                self.install_pending_closure_param_types_for_inferred_fn_param(&call_name, args);
            if !seeded_from_inference {
                self.install_pending_closure_param_types_for_any_param_hof(&call_name, args);
            }

            // STAGE F4 (strict-flip, 2026-06-20): thread the callee's declared
            // param type-annotations so a bare empty-array argument (`[]`) whose
            // param is `Array<T>` constructs a valid typed empty `TypedArray<T>`
            // rather than SURFACEing `op_new_array(0)`. Looked up by the resolved
            // `call_name` first (covers monomorphized / const-specialized
            // callees), falling back to the surface `name`.
            let param_annotations: Option<Vec<Option<shape_ast::ast::TypeAnnotation>>> = self
                .function_defs
                .get(&call_name)
                .or_else(|| self.function_defs.get(name))
                .map(|def| {
                    def.params
                        .iter()
                        .map(|p| p.type_annotation.clone())
                        .collect()
                });

            let frame_widened_args =
                self.widen_int_literal_args_for_call_frame(args, call_func_idx);
            let args = frame_widened_args.as_deref().unwrap_or(args);

            // FIX B (strict-flip, THE GENERAL ROOT): an argument whose inferred
            // type is `unknown` / an unresolved free variable MUST NOT be
            // accepted into a parameter whose declared type is a PROVEN concrete
            // type (a primitive, a registered struct, or another concrete
            // nominal). This is the keystone's no-any-sink rule for binary-op
            // operands, extended to call arguments — it closes the LAUNDER
            // boundary that a pattern-bound `unknown` (or any other genuinely
            // un-inferable value) would otherwise pass through into a typed slot
            // and be bit-reinterpreted as that slot's NativeKind.
            //
            // No false positives after the T1 keystone: legitimate dispatch
            // results (`.map`/`.get`/match-arm binders/...) now resolve to
            // CONCRETE types via the post-solve expr-type table, so a VALID
            // program never passes `unknown` here. Only a genuinely un-inferable
            // value reaches this gate, and it SHOULD reject.
            //
            // A generic param (`fn f<T>(x: T)`) is NOT a proven concrete type —
            // its annotation names a type parameter, so it is excluded.
            self.reject_unknown_arg_into_typed_param(
                name,
                &call_name,
                args,
                param_annotations.as_deref(),
            )?;
            // Deferred implicit-generic templates are dead bytecode: concrete
            // call sites re-emit a specialization with proven parameter slots.
            // Enforcing a frame-kind guard against the template's provisional
            // descriptor would turn an implementation artifact into a static
            // proof. Keep the guard for every concrete emission path.
            if !self.deferring_uninstantiated_template_body {
                self.reject_mismatched_arg_kind_into_call_frame(&call_name, args, call_func_idx)?;
            }

            let writebacks = self.compile_call_args_with_param_types(
                args,
                Some(&pass_modes),
                param_annotations.as_deref(),
            )?;
            // The closure compile path takes() the hint, but if the closure
            // arg failed early (or there's no closure arg), clear any
            // residual hint to avoid leaking it into a later unrelated call.
            self.pending_closure_param_types = None;

            // Compile default expressions for missing arguments
            if actual_arity < effective_total_arity {
                let func_def = self
                    .function_defs
                    .get(&call_name)
                    .or_else(|| self.function_defs.get(name))
                    .cloned();
                for param_idx in actual_arity..effective_total_arity {
                    let mut emitted_default = false;
                    if let Some(ref fdef) = func_def {
                        if let Some(param) = fdef.params.get(param_idx) {
                            if let Some(ref default_expr) = param.default_value {
                                let is_ref_param =
                                    ref_params.get(param_idx).copied().unwrap_or(false);
                                if is_ref_param {
                                    let borrow_mode =
                                        if ref_mutates.get(param_idx).copied().unwrap_or(false) {
                                            crate::compiler::BorrowMode::Exclusive
                                        } else {
                                            crate::compiler::BorrowMode::Shared
                                        };
                                    self.compile_implicit_reference_arg(default_expr, borrow_mode)?;
                                }
                                if !is_ref_param {
                                    self.compile_expr(default_expr)?;
                                }
                                emitted_default = true;
                            }
                        }
                    }
                    if !emitted_default {
                        self.emit_unit();
                    }
                }
            }
            let arg_count = self
                .program
                .add_constant(Constant::Int(effective_total_arity as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(
                    call_func_idx as u16,
                ))),
            ));
            // Record callee as a blob dependency
            if let Some(ref mut blob) = self.current_blob_builder {
                blob.record_call(&call_name);
            }
            if !writebacks.is_empty() {
                let result_local = self.declare_temp_local("__call_result_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));
                for (shadow_local, binding_idx) in writebacks {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(shadow_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
            }

            let return_type_annotation = self
                .function_defs
                .get(&call_name)
                .and_then(|def| def.return_type.clone())
                .or_else(|| {
                    self.foreign_function_defs
                        .get(&call_name)
                        .and_then(|def| def.return_type.clone())
                });
            // Module-qualified callees (`m::mk` returns `P`) carry their
            // return-type annotation in bare form (`P`) even though the
            // schema is registered as `m::P`. `type_info_from_annotation`
            // looks up the bare name first; on miss, retry with the call
            // name's namespace prefix so the schema lookup succeeds and
            // downstream property access (`m::mk().x`) resolves the
            // typed-field tag at the GetProp emit site (task #108
            // companion to commit 0f15571's executor flip).
            let direct = return_type_annotation
                .as_ref()
                .and_then(|ann| self.type_info_from_annotation(ann));
            self.last_expr_type_info = direct
                .or_else(|| {
                    let ann = return_type_annotation.as_ref()?;
                    let namespace = call_name.rsplit_once("::").map(|(ns, _)| ns)?;
                    let qualified = qualify_type_annotation_with_namespace(ann, namespace)?;
                    self.type_info_from_annotation(&qualified)
                })
                // WS-9c: an unannotated function whose inferred return type
                // is an anonymous object (an object-literal factory) carries
                // no `return_type_annotation`. Register an inline anonymous
                // schema for the projected return fields so the call result
                // — and a `let` bound to it — resolves `.field` access.
                .or_else(|| self.inline_schema_for_inferred_return(&call_name));
            self.last_expr_schema = self
                .last_expr_type_info
                .as_ref()
                .and_then(Self::value_schema_from_type_info);
            self.stamp_last_expr_from_static_call_expr(&call_name, args, span);

            // Propagate return type for typed opcode emission
            if let Some(return_reference_summary) = return_reference_summary {
                self.set_last_expr_reference_result(return_reference_summary.mode, true);
            } else if let Some(borrow_mode) = self.function_declares_borrow_return(&call_name) {
                // ADR-006 §2.7.30 (GapA): `-> &T` callee returns a reference value;
                // value position reads THROUGH it (no param-reborrow summary).
                self.set_last_expr_reference_result(borrow_mode, true);
                // JIT has no §2.7.30 PromotedCell lowering — deopt to interpreter.
                self.program.has_reference_escape_promotion = true;
            } else {
                self.clear_last_expr_reference_result();
            }
            return Ok(());
        }

        self.reject_const_args_for_non_generic_call(name, const_args, span)?;

        if let Some(builtin_decl) = self.resolve_scoped_module_builtin_function(name)
            && !(self.allow_internal_builtins
                && Self::is_internal_intrinsic_name(&builtin_decl.export_name))
        {
            return self.compile_module_builtin_function_call(&builtin_decl, args, span);
        }

        // Builtins take precedence - they're optimized Rust implementations.
        // Phase 1 keeps the current surface behavior, but distinguishes
        // surface names from internal-only intrinsics for diagnostics.
        if let Some(resolution) = self.classify_builtin_function(name) {
            let builtin = match resolution {
                BuiltinNameResolution::Surface { builtin, .. } => builtin,
                BuiltinNameResolution::InternalOnly { builtin, .. }
                    if self.allow_internal_builtins =>
                {
                    builtin
                }
                BuiltinNameResolution::InternalOnly { .. } => {
                    return Err(ShapeError::SemanticError {
                        message: self.internal_intrinsic_error_message(name, resolution),
                        location: Some(self.span_to_source_location(span)),
                    });
                }
            };

            let builtin = if builtin == BuiltinFunction::SetCtor {
                if !args.is_empty() {
                    builtin
                } else {
                    self.typed_set_ctor_for_call_span(span).ok_or_else(|| {
                        ShapeError::SemanticError {
                            message: "cannot construct `Set()` without a statically proven element type; annotate it as `Set<T>` or use it where the checker pins `T`"
                                .to_string(),
                            location: Some(self.span_to_source_location(span)),
                        }
                    })?
                }
            } else {
                builtin
            };

            // Special handling for print with string interpolation
            if builtin == BuiltinFunction::Print {
                return self.compile_print_with_interpolation(args);
            }

            // U3 (SB-9 deletion): the dual-HashMap-carrier split-brain is gone.
            // ALL HashMap construction routes through `BuiltinCall(HashMapCtor)`
            // → the single honest `HashMapData` carrier (HeapKind::HashMap,
            // refcounted via the §2.7.7 kind track, element-releasing Drop,
            // `NativeKind::Null` None-arm, snapshot arm). The compile-time
            // `should_use_typed_map` switch + the `TypedMap<K,V>` carrier that
            // stamped `NativeKind::UInt64` (the carrier-kind lie SB-10/11/12/13)
            // were deleted: adding/removing a type annotation on a HashMap must
            // not change which runtime structure, None-encoding, or refcount
            // discipline is used.

            for arg in args {
                self.compile_expr_as_value_or_placeholder(arg)?;
            }
            if self.builtin_requires_arg_count(builtin) {
                let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));
            }
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(builtin)),
            ));
            // Propagate known return type for builtin functions
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.clear_last_expr_reference_result();
            // ADR-006 §2.7.27 / Item 4 ruling: signal a recognised COW
            // container kind so the surrounding let-binding code path
            // can transfer it to the receiver-local's
            // `mut_self_container_locals` entry. The signal is consumed
            // at `statements.rs` let-binding completion (mirror of
            // `pending_variable_typed_array_kind`).
            if let Some(kind) =
                crate::compiler::mutation_writeback::ContainerKind::from_ctor_name(name)
            {
                self.pending_variable_container_kind = Some(kind);
            }
            return Ok(());
        }

        // Removed global data-loading API:
        // load("provider", { ... }) -> provider.load({ ... }) (module-scoped).
        if name == "load"
            && args.len() == 2
            && matches!(args[0], Expr::Literal(Literal::String(_), _))
        {
            return Err(ShapeError::SemanticError {
                message:
                    "load(provider, params) has been removed. Use module-scoped calls like `provider.load({ ... })`."
                        .to_string(),
                location: Some(self.span_to_source_location(span)),
            });
        }

        // Named import from a native extension module (e.g. `from std::core::file use { read_text }`).
        // Native modules have no AST to inline, so the function won't be in program.functions.
        // Keep a private module binding so the imported symbol can dispatch without
        // implicitly creating a user-visible namespace.
        if let Some(imported) = self.imported_names.get(name).cloned() {
            if self.is_native_module_export(&imported.module_path, &imported.original_name) {
                let binding_name = self.ensure_hidden_native_module_binding(&imported.module_path);
                return self.compile_module_namespace_call_on_binding(
                    &binding_name,
                    &imported.module_path,
                    span,
                    &imported.original_name,
                    &[],
                    args,
                );
            }
        }

        // Build error message with suggestions
        let mut message = self.undefined_function_message(name);

        // Try import suggestion first
        if let Some(module_path) = self.suggest_import(name) {
            message = format!(
                "Unknown function '{}'. Did you mean to import it via '{}'\n\n  from {} use {{ {} }}\n\n{}",
                name,
                module_path,
                module_path,
                name,
                Self::function_scope_summary(),
            );
        } else {
            // Try typo suggestion from available function names
            let available = self.collect_available_function_names();
            if let Some(suggestion) = suggest_function(name, &available) {
                message.push_str(&format!(". {}", suggestion));
            }
        }
        Err(ShapeError::RuntimeError {
            message,
            location: Some(self.span_to_source_location(span)),
        })
    }

    /// FIX B (strict-flip, THE GENERAL ROOT — close the launder boundary):
    /// reject an argument whose inferred type is `unknown` / an unresolved free
    /// variable when the matching parameter's declared type is a PROVEN concrete
    /// type (a primitive, a registered struct, or a registered concrete enum).
    ///
    /// This mirrors the keystone's no-any-sink rule for binary-op operands,
    /// extended to call arguments. Without it a pattern-bound `unknown` (or any
    /// other genuinely un-inferable value) launders through the typed fn-arg
    /// boundary into a concrete slot and is bit-reinterpreted as that slot's
    /// NativeKind — the catastrophic cross-type reinterpret
    /// (`sink(n)` with `n: unknown` → `int` param → raw-ptr arithmetic).
    ///
    /// NO FALSE POSITIVES after the T1 keystone: legitimate dispatch results
    /// (`.map`/`.get`/match-arm binders) resolve to CONCRETE types via the
    /// post-solve expr-type table, so a valid program never passes `unknown`
    /// here. A generic param (`fn f<T>(x: T)`) is excluded — its annotation
    /// names a type parameter, not a proven concrete type.
    fn reject_unknown_arg_into_typed_param(
        &mut self,
        name: &str,
        call_name: &str,
        args: &[Expr],
        param_annotations: Option<&[Option<shape_ast::ast::TypeAnnotation>]>,
    ) -> Result<()> {
        let Some(param_annotations) = param_annotations else {
            return Ok(());
        };

        // The callee's own generic type-parameter names. A param annotated with
        // one of these is NOT a proven concrete type (it is monomorphized from
        // the call-site arg), so it must NOT trigger the reject.
        let type_param_names: std::collections::HashSet<String> = self
            .function_defs
            .get(call_name)
            .or_else(|| self.function_defs.get(name))
            .and_then(|def| def.type_params.as_ref())
            .map(|tps| tps.iter().map(|tp| tp.name().to_string()).collect())
            .unwrap_or_default();

        for (idx, arg) in args.iter().enumerate() {
            let Some(Some(ann)) = param_annotations.get(idx) else {
                continue;
            };
            if !self.param_annotation_is_proven_concrete(ann, &type_param_names) {
                continue;
            }
            // The param slot is a proven concrete type. Reject only when the
            // argument's type is genuinely un-inferable.
            let Ok(arg_ty) = self.infer_expr_type(arg) else {
                continue;
            };
            if Self::type_is_unknown(&arg_ty) {
                let param_disp = Self::annotation_display_name(ann);
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "argument #{} to '{}' has an un-inferable type (`unknown`), \
                         but the parameter has the proven concrete type '{}' — an \
                         `unknown`-typed value cannot be passed into a typed parameter \
                         (this would reinterpret its raw bits as '{}'). Annotate the \
                         value or destructure it through a type-checked pattern.",
                        idx + 1,
                        name,
                        param_disp,
                        param_disp
                    ),
                    location: Some(self.span_to_source_location(arg.span())),
                });
            }
        }
        Ok(())
    }

    fn reject_mismatched_arg_kind_into_call_frame(
        &mut self,
        call_name: &str,
        args: &[Expr],
        call_func_idx: usize,
    ) -> Result<()> {
        let Some(func) = self.program.functions.get(call_func_idx) else {
            return Ok(());
        };
        let Some(frame) = func.frame_descriptor.as_ref() else {
            return Ok(());
        };
        let slot_kinds = frame.slots.clone();

        for (idx, arg) in args.iter().enumerate() {
            let Some(expected_kind) = slot_kinds.get(idx).copied() else {
                continue;
            };
            if !Self::direct_call_scalar_kind_guard_applies(expected_kind) {
                continue;
            }
            let Some(arg_ct) = self.direct_call_arg_concrete_type(arg) else {
                continue;
            };
            let actual_kind =
                shape_value::v2::closure_layout::native_kind_from_concrete_type(&arg_ct);
            if !Self::direct_call_scalar_kind_guard_applies(actual_kind)
                || actual_kind == expected_kind
            {
                continue;
            }

            return Err(ShapeError::SemanticError {
                message: format!(
                    "cannot safely pass argument #{} to '{}': argument is statically proven as {:?}, \
                     but the callee parameter slot is {:?}. Add an explicit annotation or make the \
                     call-site proof match the callee signature.",
                    idx + 1,
                    call_name,
                    actual_kind,
                    expected_kind
                ),
                location: Some(self.span_to_source_location(arg.span())),
            });
        }

        Ok(())
    }

    fn widen_int_literal_args_for_call_frame(
        &self,
        args: &[Expr],
        call_func_idx: usize,
    ) -> Option<Vec<Expr>> {
        let frame = self
            .program
            .functions
            .get(call_func_idx)?
            .frame_descriptor
            .as_ref()?;
        let mut changed = false;
        let mut out = Vec::with_capacity(args.len());
        for (idx, arg) in args.iter().enumerate() {
            let widened = match frame.slots.get(idx).copied() {
                Some(shape_value::NativeKind::Float64) => {
                    crate::compiler::literal_widen::widen_int_literal_to_number(arg)
                }
                _ => None,
            };
            match widened {
                Some(w) => {
                    changed = true;
                    out.push(w);
                }
                None => out.push(arg.clone()),
            }
        }
        if changed { Some(out) } else { None }
    }

    fn direct_call_arg_concrete_type(&mut self, arg: &Expr) -> Option<ConcreteType> {
        concrete_type_for_expr(self, arg).or_else(|| {
            let Type::Concrete(ann) = self.infer_expr_type(arg).ok()? else {
                return None;
            };
            crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                self, &ann,
            )
        })
    }

    fn direct_call_scalar_kind_guard_applies(kind: shape_value::NativeKind) -> bool {
        matches!(
            kind,
            shape_value::NativeKind::Int64
                | shape_value::NativeKind::UInt64
                | shape_value::NativeKind::Int32
                | shape_value::NativeKind::UInt32
                | shape_value::NativeKind::Int16
                | shape_value::NativeKind::UInt16
                | shape_value::NativeKind::Int8
                | shape_value::NativeKind::UInt8
                | shape_value::NativeKind::Float64
                | shape_value::NativeKind::Float32
                | shape_value::NativeKind::Bool
        )
    }

    /// True when a parameter's declared annotation resolves to a PROVEN concrete
    /// type — a known primitive, a registered struct (type alias to an object
    /// shape), or a registered enum — and is NOT one of the callee's generic
    /// type-parameter names. Generic, structural, and unresolved annotations are
    /// NOT proven concrete (conservative: only positive proof triggers FIX B).
    fn param_annotation_is_proven_concrete(
        &self,
        ann: &shape_ast::ast::TypeAnnotation,
        type_param_names: &std::collections::HashSet<String>,
    ) -> bool {
        use shape_ast::ast::TypeAnnotation;
        let name = match ann {
            TypeAnnotation::Basic(n) => n.as_str(),
            TypeAnnotation::Reference(p) => p.as_str(),
            _ => return false,
        };
        // A generic type-parameter name is monomorphized from the arg — never a
        // proven concrete slot.
        if type_param_names.contains(name) {
            return false;
        }
        if Self::is_known_concrete_primitive_name(name) {
            return true;
        }
        // A registered struct (`type Point { ... }` → type alias to Object) or a
        // registered enum is a proven concrete nominal slot.
        self.type_inference.env.lookup_type_alias(name).is_some()
            || self.type_inference.env.get_enum(name).is_some()
    }

    /// Known primitive (non-generic) type names. Mirrors the type-system side
    /// `is_known_primitive_name`; kept compiler-local to avoid a cross-crate
    /// pub surface just for this gate.
    pub(crate) fn is_known_concrete_primitive_name(name: &str) -> bool {
        matches!(
            name,
            "int"
                | "number"
                | "bool"
                | "string"
                | "decimal"
                | "bigint"
                | "char"
                | "byte"
                | "DateTime"
                | "Duration"
        )
    }

    /// True when an inferred argument type is genuinely un-inferable: an
    /// unresolved free type variable, a still-bounded constrained variable, or
    /// the `"unknown"` placeholder a lost type var renders to.
    pub(crate) fn type_is_unknown(ty: &shape_runtime::type_system::Type) -> bool {
        use shape_ast::ast::TypeAnnotation;
        use shape_runtime::type_system::Type;
        match ty {
            Type::Variable(_) | Type::Constrained { .. } => true,
            Type::Concrete(TypeAnnotation::Basic(n)) => n == "unknown",
            Type::Concrete(TypeAnnotation::Reference(p)) => p.as_str() == "unknown",
            _ => false,
        }
    }

    /// Display name for a param annotation in a FIX-B diagnostic.
    fn annotation_display_name(ann: &shape_ast::ast::TypeAnnotation) -> String {
        use shape_ast::ast::TypeAnnotation;
        match ann {
            TypeAnnotation::Basic(n) => n.clone(),
            TypeAnnotation::Reference(p) => p.to_string(),
            other => format!("{:?}", other),
        }
    }

    /// Check if a method name accepts a closure argument with a receiver-typed row parameter.
    ///
    /// Queries the MethodTable for Table and DataTable first; falls back to
    /// the hardcoded heuristic for user-defined types or methods not yet in the table.
    fn is_datatable_closure_method(&self, method: &str) -> bool {
        if self
            .method_table
            .takes_closure_with_receiver_param("Table", method)
            || self
                .method_table
                .takes_closure_with_receiver_param("DataTable", method)
        {
            return true;
        }
        // Fallback: hardcoded heuristic for methods not registered in the MethodTable
        // (e.g., user-defined types, aliases like group_by/index_by)
        Self::is_datatable_closure_method_heuristic(method)
    }

    /// Hardcoded fallback for closure-method detection.
    fn is_datatable_closure_method_heuristic(method: &str) -> bool {
        matches!(
            method,
            "filter"
                | "forEach"
                | "map"
                | "find"
                | "some"
                | "every"
                | "groupBy"
                | "group_by"
                | "orderBy"
                | "index_by"
                | "indexBy"
                | "sum"
                | "mean"
                | "min"
                | "max"
                | "simulate"
        )
    }

    /// Check if a method preserves the Table<T> type (output is same Table<T> as input).
    ///
    /// Queries the MethodTable for Table, DataTable, and Array first; falls back to
    /// the hardcoded heuristic for user-defined types or methods not yet in the table.
    fn is_type_preserving_table_method(&self, method: &str) -> bool {
        if self.method_table.is_self_returning("Table", method)
            || self.method_table.is_self_returning("DataTable", method)
        {
            return true;
        }
        // Fallback: hardcoded heuristic for methods not registered in the MethodTable
        // (e.g., user-defined types, aliases like "where", "slice", "reverse", "concat")
        Self::is_type_preserving_table_method_heuristic(method)
    }

    /// Hardcoded fallback for type-preserving method detection.
    fn is_type_preserving_table_method_heuristic(method: &str) -> bool {
        matches!(
            method,
            "filter"
                | "where"
                | "head"
                | "tail"
                | "slice"
                | "reverse"
                | "concat"
                | "orderBy"
                | "sort"
        )
    }

    pub(super) fn is_module_namespace_name(&self, name: &str) -> bool {
        (name == "__comptime__" && self.allow_internal_comptime_namespace)
            || self.module_namespace_bindings.contains(name)
    }

    fn compile_type_namespace_builtin_call(
        &mut self,
        namespace: &str,
        function: &str,
        const_args: &[Expr],
        args: &[Expr],
        span: Span,
    ) -> Result<bool> {
        let builtin = match (namespace, function) {
            ("DateTime", "now") => Some(BuiltinFunction::DateTimeNow),
            ("DateTime", "utc") => Some(BuiltinFunction::DateTimeUtc),
            ("DateTime", "parse") => Some(BuiltinFunction::DateTimeParse),
            ("DateTime", "from_epoch") => Some(BuiltinFunction::DateTimeFromEpoch),
            ("DateTime", "from_parts") => Some(BuiltinFunction::DateTimeFromParts),
            ("DateTime", "from_unix_secs") => Some(BuiltinFunction::DateTimeFromUnixSecs),
            ("Content", "chart") => Some(BuiltinFunction::ContentChart),
            ("Content", "text") => Some(BuiltinFunction::ContentTextCtor),
            ("Content", "table") => Some(BuiltinFunction::ContentTableCtor),
            ("Content", "code") => Some(BuiltinFunction::ContentCodeCtor),
            ("Content", "kv") => Some(BuiltinFunction::ContentKvCtor),
            ("Content", "fragment") => Some(BuiltinFunction::ContentFragmentCtor),
            // SC1 (R8 — supervisor): `Color.rgb(r, g, b)` is the only
            // call-form style-spec constructor. It returns a string carrier
            // (`rgb(r,g,b)`). Named members `Color.red` / `Border.rounded`
            // / `ChartType.line` are compile-time-constant strings emitted
            // directly by the property-access path, not routed here.
            ("Color", "rgb") => Some(BuiltinFunction::ColorRgbCtor),
            // W18.5 per-type builder constructors (supervisor D4,
            // R8 W3 2026-05-24): `Table::new()` / `Code::new()` /
            // `KeyValue::new()` → empty `ContentNode` of the matching
            // variant. Chained `.headers(...)` / `.row(...)` / `.border(...)`
            // / `.language(...)` / `.source(...)` / `.pair(...)` / `.build()`
            // live in `CONTENT_METHODS` PHF as method-call dispatch on the
            // Content receiver. Both `Foo::new()` (parsed as
            // QualifiedFunctionCall) and `Foo.new()` (parsed as MethodCall
            // on Identifier("Foo")) route here through
            // `compile_expr_qualified_function_call` /
            // `compile_expr_method_call` → `compile_type_namespace_builtin_call`.
            ("Table", "new") => Some(BuiltinFunction::TableBuilderNew),
            ("Code", "new") => Some(BuiltinFunction::CodeBuilderNew),
            ("KeyValue", "new") => Some(BuiltinFunction::KeyValueBuilderNew),
            _ => None,
        };

        let Some(builtin) = builtin else {
            return Ok(false);
        };
        let callee_name = format!("{}::{}", namespace, function);
        self.reject_const_args_for_non_generic_call(&callee_name, const_args, span)?;

        for arg in args {
            self.compile_expr_as_value_or_placeholder(arg)?;
        }
        let count = self.program.add_constant(Constant::Int(args.len() as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(count)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(builtin)),
        ));
        self.last_expr_schema = None;
        // SC1: `Color.rgb(...)` returns a `string` carrier. Content namespace
        // constructors and the per-type content builders return the concrete
        // `Ptr(HeapKind::Content)` carrier so array literals can prove
        // `Array<content>` without falling back to Any.
        self.last_expr_type_info = if matches!(builtin, BuiltinFunction::ColorRgbCtor) {
            Some(crate::type_tracking::VariableTypeInfo::named(
                "string".to_string(),
            ))
        } else if matches!(
            builtin,
            BuiltinFunction::ContentChart
                | BuiltinFunction::ContentTextCtor
                | BuiltinFunction::ContentTableCtor
                | BuiltinFunction::ContentCodeCtor
                | BuiltinFunction::ContentKvCtor
                | BuiltinFunction::ContentFragmentCtor
                | BuiltinFunction::TableBuilderNew
                | BuiltinFunction::CodeBuilderNew
                | BuiltinFunction::KeyValueBuilderNew
        ) {
            Some(content_type_info())
        } else {
            None
        };
        self.clear_last_expr_reference_result();
        let _ = span;
        Ok(true)
    }

    pub(super) fn compile_expr_qualified_function_call(
        &mut self,
        namespace: &str,
        function: &str,
        const_args: &[Expr],
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        let scoped_name = format!("{}::{}", namespace, function);
        if let Some(builtin_decl) = self.module_builtin_functions.get(&scoped_name).cloned() {
            self.reject_const_args_for_non_generic_call(&scoped_name, const_args, span)?;
            return self.compile_module_builtin_function_call(&builtin_decl, args, span);
        }
        if self.find_function(&scoped_name).is_some() {
            return self.compile_expr_function_call(&scoped_name, const_args, args, span);
        }

        if self.is_module_namespace_name(namespace) {
            return self.compile_module_namespace_call(namespace, span, function, const_args, args);
        }

        if self.compile_type_namespace_builtin_call(namespace, function, const_args, args, span)? {
            return Ok(());
        }

        if let Some(schema) = self.type_tracker.schema_registry().get(namespace)
            && let Some(enum_info) = schema.get_enum_info()
            && enum_info.variant_by_name(function).is_some()
        {
            return self.compile_expr_enum_constructor(
                namespace,
                function,
                &shape_ast::ast::EnumConstructorPayload::Tuple(args.to_vec()),
            );
        }

        Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown qualified call '{}::{}'. Module namespace calls require an explicit `use`, and type-associated calls require the type to define that item.",
                namespace, function
            ),
            location: Some(self.span_to_source_location(span)),
        })
    }

    /// Strict-typing-sweep (Cluster 3): for HOF method calls on arrays
    /// (`.map` / `.filter` / `.reduce` / `.forEach` / `.find` / `.findIndex`
    /// / `.some` / `.every` / `.flatMap`), populate
    /// `pending_closure_param_types` so the closure compile path attaches a
    /// concrete annotation to the user param (e.g. `|x|` → `|x: int|`)
    /// which the type-tracker installs and the binary-op compile path then
    /// trusts.
    ///
    /// The receiver was already compiled by the caller, so whole-binding
    /// `ConcreteType::Array(elem)` records are populated; an inline array
    /// literal receiver resolves its element type structurally via
    /// `concrete_type_for_expr` (U4-6a: the per-span `array_element_types`
    /// cache is deleted).
    ///
    /// Argument-order validation: every HOF wired here takes its callback
    /// as positional argument 0 — `map(f)` / `filter(predicate)` /
    /// `reduce(f, init)` / etc. (see `crates/shape-runtime/stdlib-src/core/
    /// vec.shape`). If argument 0 is a *provably* non-callable expression
    /// (a literal, an array literal, or an object literal — none of which
    /// can ever denote a callable), the call is ill-typed. Without this
    /// guard, the wrong-order call `[1,2,3].reduce(0, |acc,x| acc+x)`
    /// (init first, JS/conventional order — but Shape's `reduce` is
    /// `(f, init)`) bound the int `0` to the generic callable param `f`
    /// and degenerated into a re-entrant `main` miscompile (infinite loop)
    /// instead of a clean type error. We surface a compile-time
    /// `SemanticError` here, the earliest point that has both the method
    /// name and the literal arg kinds in hand.
    pub(crate) fn install_pending_closure_param_types_for_hof(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
    ) -> Result<()> {
        // Only the simple "single closure with one user-param-of-element-type"
        // HOFs are wired here. Reduce takes (acc, x) — both are element-type
        // for homogeneous folds, so we hint both. Sort takes a comparator
        // `(T, T) => int` — both params are element-type, like reduce's
        // homogeneous fold but with the array element type for both
        // positions (D-α.1 close, 2026-05-22, per
        // `v0.3-d-alpha-audit.md` §4 trigger KC #6(f)).
        let is_single_arg_hof = matches!(
            method,
            "map"
                | "filter"
                | "forEach"
                | "find"
                | "findIndex"
                | "some"
                | "every"
                | "flatMap"
                | "groupBy"
        );
        let is_reduce = method == "reduce";
        let is_sort = method == "sort";
        if !is_single_arg_hof && !is_reduce && !is_sort {
            return Ok(());
        }
        // Need at least one closure arg.
        if args.is_empty() {
            return Ok(());
        }

        // Argument-order / argument-kind validation. The callback is
        // positional argument 0 for every HOF wired here. A literal,
        // array literal, or object literal at that position can never be
        // a callable — reject it with a clean compile error rather than
        // letting an int bind a generic callable param and miscompile.
        // (Identifiers / function references / property accesses are NOT
        // rejected: they may legitimately resolve to a callable.)
        if let Some(non_callable_kind) = Self::provably_non_callable_kind(&args[0]) {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "`{method}` expects a closure (function) as its first argument, \
                     got {non_callable_kind}. Shape's `{method}` takes the callback \
                     first{}.",
                    if is_reduce {
                        " — the signature is `reduce(f, init)`, not `reduce(init, f)`"
                    } else {
                        ""
                    }
                ),
                location: Some(self.span_to_source_location(args[0].span())),
            });
        }

        let elem_ann_opt: Option<shape_ast::ast::TypeAnnotation> =
            match crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                self, receiver,
            ) {
                Some(shape_value::v2::concrete_type::ConcreteType::Array(inner)) => {
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(&inner)
                }
                _ => None,
            }
            // Fallback: if the receiver is an inline array literal, infer
            // element type from the elements via the existing inference helper.
            // (U4-6a: `concrete_type_for_expr` now resolves array literals
            // structurally from their elements; this `infer_array_element_type`
            // fallback remains for the kinds it does not yet cover.)
            .or_else(|| {
                if let Expr::Array(elements, _) = receiver {
                    let kind = crate::compiler::v2_array_emission::infer_array_element_type(
                        elements,
                        &self.type_tracker,
                    )?;
                    slot_kind_to_type_annotation(kind)
                } else {
                    None
                }
            })
            // Wave 1b iterator-HOF (2026-06-15): when the receiver is an
            // element-type-PRESERVING iterator adapter chain
            // (`[1,2,3].iter()`, `arr.iter().filter(..).take(n)`), the
            // receiver `ConcreteType` is not an `Array<T>` (Iterator has no
            // `ConcreteType` variant), so the array paths above yield `None`.
            // `iter_element_type_name` recovers the element-type NAME from the
            // adapter chain (recursing through `iter`/`filter`/`take`/`skip`,
            // the type-preserving adapters). Map that name to a
            // `TypeAnnotation`, but ONLY when it resolves to a known concrete
            // type — `declared_annotation_concrete_type` is the proof gate, so
            // an un-resolvable element name SURFACEs (the closure param stays
            // unannotated, exactly as for an array receiver whose element type
            // can't be proven). int and number stay distinct: the name carries
            // the exact proven element type, never a numeric default.
            .or_else(|| {
                let elem_name = self.iter_element_type_name(receiver)?;
                let ann = shape_ast::ast::TypeAnnotation::Basic(elem_name);
                crate::compiler::monomorphization::type_resolution::declared_annotation_concrete_type(
                    self, &ann,
                )
                .map(|_| ann)
            })
            // R3-subcase struct-array HOF (strict-flip, 2026-06-15): the
            // `.iter()` adapter chain over a struct array
            // (`users.iter().filter(|u| u.score > 85)`,
            // `users.iter().find(|u| ...)`) — the name-based fallback above
            // yields the lossy `Vec<object>` head-name which
            // `declared_annotation_concrete_type` rejects, so the struct
            // identity was lost and `u` stayed unannotated. `iter_element_concrete_type`
            // recovers the element `ConcreteType::Struct(named)` (recursing the
            // type-preserving `iter`/`filter`/`take`/`skip` adapters), and
            // `concrete_type_to_type_annotation` renders it as `Reference(name)`
            // so the closure param carries the struct type and `u.score`
            // resolves. The name IS the proof (per ADR-006 §2.7.5); an unnamed /
            // non-struct element yields `None` and the param stays unannotated.
            .or_else(|| {
                let elem_ct = self.iter_element_concrete_type(receiver)?;
                crate::compiler::expressions::closures::concrete_type_to_type_annotation(&elem_ct)
            })
            // W28 HOF reduce/static proof: `let flat = xs.flatMap(...)`
            // records a tracker name (`Vec<T>`) even when no whole-binding
            // `ConcreteType` fact exists yet. Parse that already-recorded
            // compile-time fact to type the next reducer's item parameter.
            .or_else(|| {
                if let Expr::Identifier(name, _) = receiver {
                    let type_name = self.tracker_type_name_for_identifier(name)?;
                    array_element_annotation_from_tracker_name(self, &type_name)
                } else {
                    None
                }
            })
            // Inline producer chain: `users.map(|u| u.age).reduce(...)`
            // never passes through a `let`, so recover the receiver result
            // directly from the static HOF shape.
            .or_else(|| {
                if let Expr::MethodCall {
                    receiver: inner,
                    method: recv_method,
                    args: recv_args,
                    ..
                } = receiver
                {
                    let inner_ct = concrete_type_for_expr(self, inner);
                    match self.static_array_hof_result_concrete_type(
                        inner,
                        inner_ct.as_ref(),
                        recv_method,
                        recv_args,
                    )? {
                        ConcreteType::Array(elem) => {
                            crate::compiler::expressions::closures::concrete_type_to_type_annotation(
                                &elem,
                            )
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            })
            // W28 HOF reduce/static proof: a named receiver whose initializer is
            // a type-changing HOF such as `flatMap` may already have an engine
            // span-table/binding fact of `Array<T>` even when the local
            // `ConcreteType` side-table has no entry. Consume that compile-time
            // fact here so a following `.reduce(|acc, x| ..., init)` can type
            // `x` from `T`; the accumulator still comes from `init` below.
            .or_else(|| {
                let ty = self.infer_expr_type(receiver).ok()?;
                array_element_annotation_from_inferred_type(self, &ty)
            });
        let Some(elem_ann) = elem_ann_opt else {
            return Ok(());
        };

        let hints = if is_reduce {
            // reduce(f, init): the callback `f` is positional arg 0 with
            // two user params `(acc, x)`. The accumulator type is proven by
            // the seed expression (`init`, positional arg 1), while the item
            // type is proven by the receiver element type. Do not fall back to
            // the element type for `acc`: a fold like
            // `Array<string>.reduce(|n, s| n + s.length, 0)` is valid only if
            // the seed proves `n: int`, not because strings can be decoded at
            // runtime.
            let acc_ann = args.get(1).and_then(|init| {
                crate::compiler::monomorphization::type_resolution::concrete_type_for_expr(
                    self, init,
                )
                .and_then(|ct| {
                    crate::compiler::expressions::closures::concrete_type_to_type_annotation(&ct)
                })
                .or_else(|| {
                    self.static_hof_expr_concrete_type(&HashMap::new(), init)
                        .and_then(|ct| {
                            crate::compiler::expressions::closures::concrete_type_to_type_annotation(
                                &ct,
                            )
                        })
                })
                .or_else(|| crate::compiler::expressions::closures::infer_callsite_arg_type(init))
            });
            vec![acc_ann, Some(elem_ann)]
        } else if is_sort {
            // sort(cmp): the callback `cmp` is positional arg 0 with two
            // user params `(a, b)`, both elem-type for a homogeneous
            // comparator. The return type (int) is not propagated as a
            // hint — closure body inference recovers it from the literal
            // arithmetic ops on the int-typed params. (D-α.1 close —
            // closes KC #6(f) test_array_sort_ascending /
            // test_array_sort_descending; see audit §4 sort row.)
            vec![Some(elem_ann.clone()), Some(elem_ann)]
        } else if matches!(method, "map" | "filter" | "groupBy") {
            match args.first() {
                Some(Expr::FunctionExpr { params, .. }) if params.len() >= 2 => {
                    vec![
                        Some(elem_ann),
                        Some(shape_ast::ast::TypeAnnotation::Basic("int".to_string())),
                    ]
                }
                _ => vec![Some(elem_ann)],
            }
        } else {
            vec![Some(elem_ann)]
        };
        self.pending_closure_param_types = Some(hints);
        Ok(())
    }

    /// Classify an argument expression that is *provably* not a callable.
    /// Returns a human-readable kind name (for diagnostics) when the
    /// expression can never denote a closure/function, or `None` when it
    /// might (identifiers, function references, calls, property accesses,
    /// conditionals, etc. — anything that could resolve to a callable).
    ///
    /// Only the unambiguous literal forms are rejected: this is a
    /// conservative guard that never false-positives on a legitimate
    /// callable argument such as a named function passed to `.map`.
    fn provably_non_callable_kind(arg: &Expr) -> Option<&'static str> {
        match arg {
            Expr::Literal(lit, _) => Some(match lit {
                Literal::Int(_) | Literal::UInt(_) | Literal::TypedInt(_, _) => "an int",
                Literal::Number(_) => "a number",
                Literal::Decimal(_) => "a decimal",
                Literal::String(_) | Literal::FormattedString { .. } => "a string",
                // A char literal IS an int code point (operators.mdx).
                Literal::Char(_) => "an int",
                Literal::Bool(_) => "a bool",
                // `None`, `Unit`, `Timeframe` — non-callable values.
                _ => "a literal value",
            }),
            Expr::Array(_, _) => Some("an array"),
            Expr::Object(_, _) => Some("an object"),
            _ => None,
        }
    }

    /// Wave 1a PART B: usage-driven closure-argument seeding from an
    /// inference-resolved FUNCTION-TYPED parameter.
    ///
    /// When a free user function has an UNANNOTATED param that its body USES as
    /// a callable (`fn apply2(f, x, y) { f(x, y) }`, `fn map_pair(f, a, b) {
    /// [f(a), f(b)] }`), whole-program inference resolves that param to a
    /// concrete `Type::Function`; `inferred_param_fn_param_types_from_facts`
    /// derives its argument annotations on demand from `inference_facts` plus
    /// the callee body-use shape.
    /// Here, at the call site, if the matching argument is a CLOSURE LITERAL
    /// whose user-param count equals the inferred signature arity, we map the
    /// inferred argument annotations 1:1 onto the closure's params via
    /// `pending_closure_param_types`. The closure compile path then attaches
    /// concrete annotations (`|a, b|` → `|a: int, b: int|`), so a body like
    /// `a * b` type-checks under strict typing instead of failing
    /// `unknown * unknown`.
    ///
    /// Returns `true` iff a hint was installed.
    ///
    /// Soundness (strict-typing core):
    /// * Fires ONLY for params the engine resolved to a fully-concrete fn-type
    ///   (the `None`-bailing producer guarantees no `unknown` leaks in). An
    ///   un-inferable / dead callable param has no entry ⇒ no seeding ⇒ the
    ///   closure keeps its existing rejection. No `any`, no Bool-default, no
    ///   silent pick.
    /// * Requires arity to match EXACTLY (signature arity == closure user-param
    ///   count). A mismatch ⇒ no seeding (the call is independently an
    ///   arity error elsewhere).
    /// * Each closure param is seeded from its OWN signature position, so
    ///   heterogeneous signatures (`fn(int, string)`) map correctly —
    ///   `int`/`number`/`string` stay distinct. A later body conflict with the
    ///   seeded type still surfaces as a strict error.
    pub(crate) fn install_pending_closure_param_types_for_inferred_fn_param(
        &mut self,
        callee_name: &str,
        args: &[Expr],
    ) -> bool {
        let Some(func_def) = self.function_defs.get(callee_name).cloned() else {
            return false;
        };
        // Find the argument positions that are closure literals AND for which
        // the callee has an inferred concrete fn-type. We only seed when there
        // is exactly one such position (a single pending hint slot), matching
        // the closure compile path's single-`take()` consumption.
        let mut seedable: Option<(usize, Vec<shape_ast::ast::TypeAnnotation>)> = None;
        for (idx, arg) in args.iter().enumerate() {
            if !matches!(arg, Expr::FunctionExpr { .. }) {
                continue;
            }
            let Some(sig_anns) =
                self.inferred_param_fn_param_types_from_facts(callee_name, &func_def, idx)
            else {
                continue;
            };
            if seedable.is_some() {
                // More than one seedable closure arg — the single pending-hint
                // slot cannot serve both. Bail rather than mis-seed.
                return false;
            }
            seedable = Some((idx, sig_anns));
        }
        let Some((closure_pos, sig_anns)) = seedable else {
            return false;
        };

        // Match the closure's user-param count to the inferred arity exactly.
        let Expr::FunctionExpr { params, .. } = &args[closure_pos] else {
            return false;
        };
        if params.len() != sig_anns.len() || sig_anns.is_empty() {
            return false;
        }

        let hints: Vec<Option<shape_ast::ast::TypeAnnotation>> =
            sig_anns.iter().cloned().map(Some).collect();
        self.pending_closure_param_types = Some(hints);
        true
    }

    /// Sweep phase 3c.x: bidirectional inference for free user functions
    /// whose callable param is typed `any`. When the call site supplies a
    /// closure literal at the same position, infer the closure's param
    /// types from the OTHER concrete-typed args at the call site.
    ///
    /// Concretely: `apply2(f: any, a: int, b: int) -> int` called as
    /// `apply2(|x, y| x + y, 2, 3)` should map to `|x: int, y: int|`.
    /// We scan args once, find the (single) closure arg position, and use
    /// the remaining args' inferred types to fill closure-param hints.
    /// We require the remaining args' types to be homogeneous and to
    /// match the closure's user-param count exactly.
    pub(crate) fn install_pending_closure_param_types_for_any_param_hof(
        &mut self,
        callee_name: &str,
        args: &[Expr],
    ) {
        // Locate the (single) closure-literal arg.
        let closure_idx = args
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match a {
                Expr::FunctionExpr { .. } => Some(i),
                _ => None,
            })
            .collect::<Vec<_>>();
        if closure_idx.len() != 1 {
            return;
        }
        let closure_pos = closure_idx[0];

        // Look up the closure's user-param count.
        let closure_user_param_count = if let Expr::FunctionExpr { params, .. } = &args[closure_pos]
        {
            params.len()
        } else {
            return;
        };
        if closure_user_param_count == 0 {
            return;
        }

        // The callee must be a known user function whose param at
        // `closure_pos` is annotated `any` (callable-by-erased-type).
        let func_def = match self.function_defs.get(callee_name).cloned() {
            Some(def) => def,
            None => return,
        };
        let callee_param_at_closure_pos = match func_def.params.get(closure_pos) {
            Some(p) => p,
            None => return,
        };
        let is_any_annotated = matches!(
            &callee_param_at_closure_pos.type_annotation,
            Some(shape_ast::ast::TypeAnnotation::Basic(name)) if name == "any"
        );
        if !is_any_annotated {
            return;
        }

        // Collect inferred types for the remaining (non-closure) args.
        let mut remaining_types: Vec<shape_ast::ast::TypeAnnotation> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if i == closure_pos {
                continue;
            }
            let ty = match self.infer_expr_type(arg) {
                Ok(t) => t,
                Err(_) => return, // Unknown type — bail.
            };
            // Require a Concrete(Basic(...)) primitive name.
            let ann = match ty {
                shape_runtime::type_system::Type::Concrete(ann) => ann,
                _ => return,
            };
            remaining_types.push(ann);
        }

        // Require exactly `closure_user_param_count` remaining args (so
        // they zip 1:1 with the closure's user params).
        if remaining_types.len() != closure_user_param_count {
            return;
        }
        // Require all remaining types to be the same primitive scalar name
        // — homogeneous arithmetic is the only safe pattern for a closure
        // body like `x + y`. Heterogeneous args would need stronger
        // analysis to map to specific param positions.
        let first = match remaining_types.first() {
            Some(shape_ast::ast::TypeAnnotation::Basic(n)) => n.clone(),
            _ => return,
        };
        for ann in &remaining_types[1..] {
            match ann {
                shape_ast::ast::TypeAnnotation::Basic(n) if *n == first => {}
                _ => return,
            }
        }
        if !BytecodeCompiler::tracker_type_name_is_primitive(&first) {
            return;
        }

        let elem_ann = shape_ast::ast::TypeAnnotation::Basic(first);
        let hints = vec![Some(elem_ann); closure_user_param_count];
        self.pending_closure_param_types = Some(hints);
    }

    /// ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback,
    /// 2026-05-12): determine whether the method call needs a
    /// post-`CallMethod` write-back to the receiver's binding slot.
    ///
    /// Returns `Some(target)` when ALL of:
    /// - `receiver` is an `Identifier(name, _)` (resolvable to a
    ///   local-slot index OR a module-binding index);
    /// - the receiver binding is tracked as a recognised COW container
    ///   kind in `mut_self_container_locals` /
    ///   `mut_self_container_bindings`;
    /// - `method` is in the kind's `MUT_SELF_*` set per
    ///   `method_registry`.
    ///
    /// Returns `None` otherwise; the standard `CallMethod` path then
    /// runs without write-back (the dispatch text's "silent drop"
    /// decision-call for r-value receivers and for non-container
    /// receivers).
    fn resolve_mut_self_writeback_target(
        &self,
        receiver: &Expr,
        method: &str,
    ) -> Option<crate::compiler::mutation_writeback::MutSelfWriteBackTarget> {
        use crate::compiler::mutation_writeback::MutSelfWriteBackTarget;
        let Expr::Identifier(name, _) = receiver else {
            return None;
        };
        if let Some(local_idx) = self.resolve_local(name) {
            // R2 chained-builder-on-immutable: a `&mut self` builder method
            // on an immutable receiver returns a NEW container Arc; the
            // binding is left unchanged. No write-back is emitted (which
            // would otherwise require — and reject on — an immutable
            // binding). In-place mutation is the opt-in `let mut` feature.
            if self.is_local_immutable(local_idx) || self.is_local_const(local_idx) {
                return None;
            }
            if let Some(&kind) = self.mut_self_container_locals.get(&local_idx) {
                if kind.is_mut_self_method(method) {
                    return Some(MutSelfWriteBackTarget::Local(local_idx));
                }
            }
            return None;
        }
        let scoped = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped) {
            // R2: immutable module binding — same no-write-back rule.
            if self.is_module_binding_immutable(binding_idx)
                || self.is_module_binding_const(binding_idx)
            {
                return None;
            }
            if let Some(&kind) = self.mut_self_container_bindings.get(&binding_idx) {
                if kind.is_mut_self_method(method) {
                    return Some(MutSelfWriteBackTarget::ModuleBinding(binding_idx));
                }
            }
        }
        None
    }

    /// Resolve a self-returning mutating method writeback whose receiver is a
    /// mutable closure capture rather than an ordinary local/module binding.
    ///
    /// This is still fully static: the receiver must be an identifier present
    /// in the closure-capture maps, its container kind must be proven from
    /// `ConcreteType`, and the capture cell's interior `FieldKind` must have
    /// been recorded when the closure layout was compiled. No runtime
    /// receiver classification is introduced here.
    fn resolve_mut_self_capture_writeback_target(
        &self,
        receiver: &Expr,
        method: &str,
    ) -> Result<Option<CaptureMutSelfWriteBackTarget>> {
        let Expr::Identifier(name, span) = receiver else {
            return Ok(None);
        };
        if !self.mutable_closure_captures.contains_key(name.as_str()) {
            return Ok(None);
        }

        let receiver_ct = concrete_type_for_expr(self, receiver).or_else(|| {
            crate::compiler::monomorphization::type_resolution::binding_fact_capture_type(
                self, name,
            )
        });
        let Some(container_kind) = receiver_ct
            .as_ref()
            .and_then(container_kind_from_concrete_type)
        else {
            return Ok(None);
        };
        if !container_kind.is_mut_self_method(method) {
            return Ok(None);
        }

        if let Some(&shared_idx) = self.shared_closure_captures.get(name.as_str()) {
            let kind = self
                .shared_capture_inner_kinds
                .get(name.as_str())
                .copied()
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "cannot write back mutating method `{method}` on captured container \
                         `{name}` without a statically proven shared-capture kind"
                    ),
                    location: Some(self.span_to_source_location(*span)),
                })?;
            return Ok(Some(CaptureMutSelfWriteBackTarget::Shared {
                capture_idx: shared_idx,
                opcode: crate::compiler::helpers::shared_typed_store_opcode(kind),
            }));
        }

        if let Some(&owned_idx) = self.owned_mutable_closure_captures.get(name.as_str()) {
            let kind = self
                .owned_mutable_capture_inner_kinds
                .get(name.as_str())
                .copied()
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "cannot write back mutating method `{method}` on captured container \
                         `{name}` without a statically proven owned-capture kind"
                    ),
                    location: Some(self.span_to_source_location(*span)),
                })?;
            return Ok(Some(CaptureMutSelfWriteBackTarget::OwnedMutable {
                capture_idx: owned_idx,
                opcode: crate::compiler::helpers::owned_mutable_typed_store_opcode(kind),
            }));
        }

        Ok(None)
    }

    /// Tuple-return resolver — ADR-006 §2.7.27 amendment (W17-pop-mutation).
    ///
    /// Returns `Some(target)` when:
    /// - the binding's tracked container kind has `method` in its
    ///   `MUT_SELF_TUPLE_RETURN_*` set;
    /// - the receiver is an `Identifier` resolvable to a local-slot or
    ///   module-binding index.
    ///
    /// Returns `None` for r-value receivers (the caller emits `Swap; Pop`
    /// silent-drop in that case — mirror of the §2.7.27 self-returning
    /// r-value silent-drop rule) and for non-pop method names.
    ///
    /// Separate from `resolve_mut_self_writeback_target` because the
    /// post-CallMethod codegen differs (`Swap; Store*` vs `Dup; Store*`)
    /// and the ABI categories are mutually exclusive at the registry
    /// level — a method is either self-returning OR tuple-return, never
    /// both. Both resolvers share the receiver-rooting machinery
    /// (`mut_self_container_locals` / `mut_self_container_bindings`).
    fn resolve_mut_self_tuple_return_target(
        &self,
        receiver: &Expr,
        method: &str,
    ) -> Option<crate::compiler::mutation_writeback::MutSelfWriteBackTarget> {
        use crate::compiler::mutation_writeback::MutSelfWriteBackTarget;
        let Expr::Identifier(name, _) = receiver else {
            return None;
        };
        if let Some(local_idx) = self.resolve_local(name) {
            // R2: immutable receiver — no write-back. Returning None routes
            // a known tuple-return method through the r-value silent-drop
            // path (`Swap; Pop`), which consumes the side-channel NewSelf
            // and leaves the binding unchanged (sound).
            if self.is_local_immutable(local_idx) || self.is_local_const(local_idx) {
                return None;
            }
            if let Some(&kind) = self.mut_self_container_locals.get(&local_idx) {
                if kind.is_mut_self_tuple_return_method(method) {
                    return Some(MutSelfWriteBackTarget::Local(local_idx));
                }
            }
            return None;
        }
        let scoped = self
            .resolve_scoped_module_binding_name(name)
            .unwrap_or_else(|| name.to_string());
        if let Some(&binding_idx) = self.module_bindings.get(&scoped) {
            // R2: immutable module binding — same no-write-back rule.
            if self.is_module_binding_immutable(binding_idx)
                || self.is_module_binding_const(binding_idx)
            {
                return None;
            }
            if let Some(&kind) = self.mut_self_container_bindings.get(&binding_idx) {
                if kind.is_mut_self_tuple_return_method(method) {
                    return Some(MutSelfWriteBackTarget::ModuleBinding(binding_idx));
                }
            }
        }
        None
    }

    /// Returns `true` if `method` is registered for the tuple-return
    /// ABI under SOME container kind (used to choose between `Swap; Pop`
    /// silent-drop and the standard no-writeback path at r-value
    /// receiver sites). The kind narrowing happens at
    /// `resolve_mut_self_tuple_return_target`; this is just the
    /// method-name lookup.
    fn is_known_tuple_return_method(&self, method: &str) -> bool {
        crate::executor::objects::method_registry::is_mut_self_tuple_return_method_name(method)
    }

    /// Compile missing trailing arguments at a UFCS-style method call site.
    ///
    /// For each position in `actual_arity_with_self..effective_total_arity`,
    /// look up the corresponding `FunctionParameter::default_value` on the
    /// resolved callee's `FunctionDef` and compile that expression in place.
    /// Positions whose param declares no default fall back to a `Unit`
    /// sentinel (the prior, blunt behavior for both UFCS sites).
    ///
    /// Mirrors the regular `Call` path (see `compile_expr_function_call`
    /// lines ~1175-1208) so UFCS method calls participate in default-arg
    /// expansion identically to direct function calls. This is what makes
    /// `arr.slice(start)` reach `Vec.slice(self, start, end: int = -1)` with
    /// `end = -1` rather than `end = Unit` (D-δ array_slice single-arg
    /// close — `v0.3-known-constraints-audit` §6(f) Repro 1).
    ///
    /// `func_name` is the resolved callee name (e.g. `"Vec.slice"`); it keys
    /// both `function_defs` (for the default-expr AST) and the per-param
    /// reference-mode flags read from `program.functions[func_idx]`. The
    /// `func_idx` index addresses the same function so we can read
    /// `ref_params` / `ref_mutates` without re-looking up by name.
    pub(super) fn compile_missing_ufcs_default_args(
        &mut self,
        func_name: &str,
        func_idx: usize,
        actual_arity_with_self: usize,
        effective_total_arity: usize,
    ) -> Result<()> {
        if actual_arity_with_self >= effective_total_arity {
            return Ok(());
        }
        let func_def = self.function_defs.get(func_name).cloned();
        let ref_params = self.program.functions[func_idx].ref_params.clone();
        let ref_mutates = self.program.functions[func_idx].ref_mutates.clone();
        for param_idx in actual_arity_with_self..effective_total_arity {
            let mut emitted_default = false;
            if let Some(ref fdef) = func_def {
                if let Some(param) = fdef.params.get(param_idx) {
                    if let Some(ref default_expr) = param.default_value {
                        let is_ref_param = ref_params.get(param_idx).copied().unwrap_or(false);
                        if is_ref_param {
                            let borrow_mode =
                                if ref_mutates.get(param_idx).copied().unwrap_or(false) {
                                    crate::compiler::BorrowMode::Exclusive
                                } else {
                                    crate::compiler::BorrowMode::Shared
                                };
                            self.compile_implicit_reference_arg(default_expr, borrow_mode)?;
                        } else {
                            self.compile_expr(default_expr)?;
                        }
                        emitted_default = true;
                    }
                }
            }
            if !emitted_default {
                self.emit_unit();
            }
        }
        Ok(())
    }

    /// Compile `expr.type().to_string()` when the `type()` receiver is fully
    /// known at compile time.
    ///
    /// `Constant::TypeAnnotation` is compile-time metadata, not a runtime
    /// `PushConst` carrier in the strict stack ABI. The user-visible result of
    /// the chained call is a string, so lower directly to the already-known
    /// rendered type name and preserve the same receiver side effects that
    /// static `type()` lowering would have preserved before popping the value.
    fn try_compile_static_type_to_string(&mut self, receiver: &Expr) -> Result<bool> {
        let Expr::MethodCall {
            receiver: type_receiver,
            method: type_method,
            args: type_args,
            named_args: type_named_args,
            optional,
            ..
        } = receiver
        else {
            return Ok(false);
        };

        if type_method != "type"
            || *optional
            || !type_args.is_empty()
            || !type_named_args.is_empty()
        {
            return Ok(false);
        }

        let is_type_symbol = self.expr_is_type_symbol(type_receiver);
        match self.static_type_annotation_for_expr(type_receiver) {
            Ok(type_ann) if !self.should_runtime_type_query(&type_ann) => {
                if !is_type_symbol {
                    self.compile_expr(type_receiver)?;
                    self.emit(Instruction::simple(OpCode::Pop));
                }

                let idx = self
                    .program
                    .add_constant(Constant::String(type_ann.to_type_string()));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(idx)),
                ));
                self.last_expr_schema = None;
                self.last_expr_type_info = Some(VariableTypeInfo::named("string".to_string()));
                self.clear_last_expr_reference_result();
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(err) => {
                if is_type_symbol {
                    Err(err)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Compile a method call expression
    pub(super) fn compile_expr_method_call(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
        // ADR-006 §2.7.5 V3-S6b conduit: AST span of the
        // `Expr::MethodCall` site. Threaded through to
        // `try_monomorphize_method_call` / `_with_closures` for the
        // `(Span, current_function) → specialized_idx` side-table key.
        // The conduit producer at
        // `infer_top_level_concrete_types_from_mir_with_resolvers` reads
        // the matching `Terminator.span` (set by `builder.emit_call(...,
        // span)` in `mir/lowering/expr.rs` at the `Expr::MethodCall` arm)
        // to look up the specialized callee.
        call_site_span: Span,
    ) -> Result<()> {
        // Chained function calls: `f(a)(b)` is parsed as MethodCall with method "__call__".
        // Compile as: evaluate receiver (which produces a callable), compile args, CallValue.
        if method == "__call__" {
            let expected_param_modes = self.callable_pass_modes_from_expr(receiver);
            let return_reference_summary =
                self.callable_return_reference_summary_from_expr(receiver);
            if let Expr::FunctionExpr { params, .. } = receiver {
                let saved_pending_closure_param_types = self.pending_closure_param_types.take();
                let hints: Vec<Option<shape_ast::ast::TypeAnnotation>> = args
                    .iter()
                    .map(crate::compiler::expressions::closures::infer_callsite_arg_type)
                    .collect();
                if params.len() == hints.len() && hints.iter().any(Option::is_some) {
                    self.pending_closure_param_types = Some(hints);
                }
                let receiver_result = self.compile_expr(receiver);
                self.pending_closure_param_types = saved_pending_closure_param_types;
                if let Err(err) = receiver_result {
                    return Err(err);
                }
            } else {
                self.compile_expr(receiver)?;
            }
            let writebacks = self.compile_call_args(args, expected_param_modes.as_deref())?;
            let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::simple(OpCode::CallValue));
            if !writebacks.is_empty() {
                let result_local = self.declare_temp_local("__chained_call_result_")?;
                self.emit(Instruction::new(
                    OpCode::StoreLocal,
                    Some(Operand::Local(result_local)),
                ));
                for (shadow_local, binding_idx) in writebacks {
                    self.emit(Instruction::new(
                        OpCode::LoadLocal,
                        Some(Operand::Local(shadow_local)),
                    ));
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
                self.emit(Instruction::new(
                    OpCode::LoadLocal,
                    Some(Operand::Local(result_local)),
                ));
            }
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            // Phase 4b Round 3 Surface-1A LANG-W13-3-iife-closure-capture:
            // IIFE `(|y| body)(args)` parses as
            // `MethodCall { method: "__call__", receiver: FunctionExpr {..} }`
            // (parser site: `crates/shape-ast/src/parser/expressions/primary.rs:167`).
            // The closure's return type is statically inferable via
            // `infer_closure_body_return_type_name`, but until this stamp the
            // post-`CallValue` `last_expr_*` were cleared unconditionally,
            // so `let r = (|y| y + base)(x)` recorded `r` as Unknown and
            // downstream binops failed strict-typing as `unknown + int`. Per
            // ADR-006 §2.7.5 producer-side stamp-at-compile-time: the
            // closure-body inference IS the proof — no runtime decode, no
            // fabricated Bool-default. Mirrors the by-name `let f = |...|`
            // tracker hop above (line 593) and the `update_callable_binding_
            // from_expr` recording at the `let f = <FunctionExpr>` site
            // (`helpers_reference.rs:685`).
            if let Expr::FunctionExpr {
                params,
                body,
                return_type,
                ..
            } = receiver
            {
                // Seed caller-context arg type names from the IIFE's
                // argument expressions. The inference engine uses these
                // to type unannotated closure params at the call site
                // (cluster-2-cw-IB-class-b pattern). Per ADR-006 §2.7.5
                // stamp-at-compile-time: the call-site arg type IS the
                // proof of the closure param's type at this invocation.
                let caller_arg_type_names: Vec<Option<String>> = args
                    .iter()
                    .map(|arg| {
                        self.infer_expr_type(arg).ok().and_then(|ty| {
                            let display = crate::compiler::expressions::closures::type_display_name_for_closure_inference(&ty);
                            if BytecodeCompiler::tracker_type_name_is_primitive(&display) {
                                Some(display)
                            } else {
                                None
                            }
                        })
                    })
                    .collect();
                if let Some(rt_name) =
                    crate::compiler::expressions::closures::infer_closure_body_return_type_name_with_caller_context(
                        self,
                        params,
                        body,
                        return_type.as_ref(),
                        &[],
                        &caller_arg_type_names,
                    )
                {
                    match rt_name.as_str() {
                        // U4-4: numeric register stamps replaced by
                        // `last_expr_type_info` name stamps (same as width arm).
                        "int" | "number" | "decimal" => {
                            self.last_expr_type_info =
                                Some(crate::type_tracking::VariableTypeInfo::named(
                                    rt_name.clone(),
                                ));
                        }
                        other
                            if shape_runtime::type_system::BuiltinTypes::is_integer_type_name(
                                other,
                            ) =>
                        {
                            self.last_expr_type_info =
                                Some(crate::type_tracking::VariableTypeInfo::named(
                                    other.to_string(),
                                ));
                        }
                        "string" | "bool" | "char" => {
                            self.last_expr_type_info = Some(
                                crate::type_tracking::VariableTypeInfo::named(rt_name.clone()),
                            );
                        }
                        _ => {}
                    }
                }
                let _ = call_site_span; // reserved for JIT-conduit extension
            }
            if let Some(return_reference_summary) = return_reference_summary {
                self.set_last_expr_reference_result(return_reference_summary.mode, true);
            } else {
                self.clear_last_expr_reference_result();
            }
            return Ok(());
        }

        // EmptyArray (strict-flip, 2026-06-16): reject a bare empty-array
        // LITERAL used directly as a method receiver (`[].iter()`,
        // `[].map(|x| x)`, `[].count()`) whose element type cannot be proven.
        //
        // A bare `[]` literal carries no element-type proof on its own — the
        // accumulator deferral that rescues `let mut a = []; a.push(x)` only
        // applies when the literal is BOUND to a binding (re-keyed into
        // `empty_array_accumulators`, resolved by the first `.push()`). An
        // inline-receiver `[]` is never bound, so it would otherwise lower to a
        // placeholder `NewArray(0)` that SURFACEs `op_new_array(0)` at runtime.
        // Surface a CLEAN compile error here instead (no runtime garbage).
        //
        // An ANNOTATED / inferable empty array is unaffected: in
        // `let a: Array<int> = []; a.map(...)` the receiver is the IDENTIFIER
        // `a` (which carries the annotation's element type), never the bare
        // `[]` literal. Only a literal-position empty array reaches this guard.
        // SURFACE-scope note: resolving the element type of an inline `[]` from
        // a downstream usage / return-type context (e.g. `_ => []` in a
        // `-> Array<string>` match arm) is the broader empty-array let-gen
        // inference the task defers — out of scope here.
        if let Expr::Array(elements, arr_span) = receiver {
            if elements.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "cannot infer the element type of this empty array (`[]`). \
                         It is used directly as the receiver of `.{method}(...)` with \
                         no `Array<T>` annotation and no element to infer a type from, \
                         so strict typing cannot prove what element type it holds. \
                         Bind it with an annotation first \
                         (`let a: Array<T> = []; a.{method}(...)`) or build a non-empty \
                         array."
                    ),
                    location: Some(self.span_to_source_location(*arr_span)),
                });
            }
        }

        // In-place mutation: arr.push(val) → ArrayPushLocal + LoadLocal
        // This is the primary push path for method calls inside function bodies,
        // loops, and blocks (which are compiled as expressions, not statements).
        //
        // ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback):
        // gate this bespoke path so it does NOT fire when the receiver
        // is a non-Array container (Deque / PriorityQueue / HashMap /
        // HashSet). Those containers have their own `push` handlers in
        // method_registry which the standard `CallMethod` path
        // dispatches to; `ArrayPushLocal` would error on a
        // non-Array slot kind (the runtime explicitly rejects
        // `Ptr(PriorityQueue)` etc. with `NotImplemented`).
        let bespoke_push_blocked = if let Expr::Identifier(recv_name, _) = receiver {
            let local_kind = self
                .resolve_local(recv_name)
                .and_then(|idx| self.mut_self_container_locals.get(&idx).copied());
            let module_kind = if local_kind.is_none() {
                let scoped = self
                    .resolve_scoped_module_binding_name(recv_name)
                    .unwrap_or_else(|| recv_name.to_string());
                self.module_bindings
                    .get(&scoped)
                    .copied()
                    .and_then(|idx| self.mut_self_container_bindings.get(&idx).copied())
            } else {
                None
            };
            local_kind
                .or(module_kind)
                .map(|kind| {
                    !matches!(
                        kind,
                        crate::compiler::mutation_writeback::ContainerKind::Array
                    )
                })
                .unwrap_or(false)
        } else {
            false
        };
        if method == "push" && args.len() == 1 && !bespoke_push_blocked {
            if let Expr::Identifier(recv_name, _) = receiver {
                // Phase 4b Round 6 WS-1b W16.2-C residual (2026-05-21): if
                // the receiver is a bare empty-array accumulator
                // (`let mut out = []`) still awaiting its element kind, this
                // FIRST `.push()` resolves the kind, patches the placeholder
                // allocator, promotes the binding, and emits the typed push
                // — leaving the array on the stack as the expression result.
                // Every subsequent push then takes the typed path below
                // (`resolve_receiver_typed_array_kind` now reports the kind).
                if self.compile_first_push_to_empty_accumulator(
                    recv_name,
                    &args[0],
                    Some(self.span_to_source_location(receiver.span())),
                )? {
                    self.clear_last_expr_reference_result();
                    return Ok(());
                }
                // v2 Phase 3.1 (Agent 3): typed-array fast path for `arr.push(x)`.
                // Resolved BEFORE arg compilation since compile_expr may
                // overwrite tracker state. Falls through to legacy
                // `ArrayPushLocal` for non-typed arrays / unrecognised
                // element types.
                let typed_kind = self.resolve_receiver_typed_array_kind(receiver);
                let source_loc = self.span_to_source_location(receiver.span());
                if let Some(local_idx) = self.resolve_local(recv_name) {
                    if !self.ref_locals.contains(&local_idx) {
                        self.check_named_binding_write_allowed(
                            recv_name,
                            Some(source_loc.clone()),
                        )?;
                    }
                    if let Some(kind) = typed_kind {
                        // v2 typed array push: `TypedArrayPush*` pops
                        // (arr_ptr, value). Push the array, then the value,
                        // then the typed opcode.
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                        // WS-1b: emit the element in the carrier shape the
                        // typed push requires — `NewStringV2` / `NewDecimalV2`
                        // for string / decimal literals so the
                        // `TypedArrayPushString` / `TypedArrayPushDecimal`
                        // strict-kind check accepts it.
                        self.compile_typed_array_element_value(kind, &args[0])?;
                        self.emit(Instruction::simple(kind.push_opcode()));
                        // Push the mutated array as expression result.
                        if self.ref_locals.contains(&local_idx)
                            || self.reference_value_locals.contains(&local_idx)
                        {
                            self.emit(Instruction::new(
                                OpCode::DerefLoad,
                                Some(Operand::Local(local_idx)),
                            ));
                        } else {
                            self.emit(Instruction::new(
                                OpCode::LoadLocal,
                                Some(Operand::Local(local_idx)),
                            ));
                        }
                        self.clear_last_expr_reference_result();
                        return Ok(());
                    }
                    self.compile_expr(&args[0])?;
                    // U4-4: pushed element kind from the one resolved Type.
                    let pushed_numeric = self.numeric_type_of(&args[0]);
                    self.emit(Instruction::new(
                        OpCode::ArrayPushLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                    if let Some(numeric_type) = pushed_numeric {
                        self.mark_slot_as_numeric_array(local_idx, true, numeric_type);
                    }
                    // Push the mutated array as expression result
                    if self.ref_locals.contains(&local_idx)
                        || self.reference_value_locals.contains(&local_idx)
                    {
                        self.emit(Instruction::new(
                            OpCode::DerefLoad,
                            Some(Operand::Local(local_idx)),
                        ));
                    } else {
                        self.emit(Instruction::new(
                            OpCode::LoadLocal,
                            Some(Operand::Local(local_idx)),
                        ));
                    }
                    self.clear_last_expr_reference_result();
                    return Ok(());
                } else if !self
                    .mutable_closure_captures
                    .contains_key(recv_name.as_str())
                {
                    self.check_named_binding_write_allowed(recv_name, Some(source_loc))?;
                    let binding_idx = self.get_or_create_module_binding(recv_name);
                    if let Some(kind) = typed_kind {
                        // v2 typed array push for module bindings.
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                        // WS-1b: carrier-aware element emit (see local-slot
                        // path above).
                        self.compile_typed_array_element_value(kind, &args[0])?;
                        self.emit(Instruction::simple(kind.push_opcode()));
                        self.emit(Instruction::new(
                            OpCode::LoadModuleBinding,
                            Some(Operand::ModuleBinding(binding_idx)),
                        ));
                        self.clear_last_expr_reference_result();
                        return Ok(());
                    }
                    self.compile_expr(&args[0])?;
                    self.emit(Instruction::new(
                        OpCode::ArrayPushLocal,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    // Push the mutated array as expression result
                    self.emit(Instruction::new(
                        OpCode::LoadModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                    self.clear_last_expr_reference_result();
                    return Ok(());
                }
            }
        }

        // U3 (SB-9 deletion): the v2 typed-map method fast path is gone.
        // `m.set/.get/.has/.delete/.len` on a HashMap now always dispatches
        // through the generic `CallMethod` / local-slot HashMapData path
        // (the single honest carrier), never the deleted `TypedMap<K,V>`
        // carrier with its `NativeKind::UInt64` kind lie.

        // Local-slot-based typed method dispatch.
        //
        // When the receiver is an identifier in a local slot with a proven
        // collection or string type, emit the local-slot-based opcodes that
        // read the receiver directly from the slot.
        if let Some(()) = self.try_compile_typed_slot_method(receiver, method, args)? {
            return Ok(());
        }

        // Universal type query: `expr.type()`.
        // Use static type constants when fully resolved; otherwise fall back to
        // runtime `TypeOf` so generic parameters resolve to concrete call-site types.
        if method == "type" {
            if !args.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: "type() does not take any arguments".to_string(),
                    location: Some(self.span_to_source_location(receiver.span())),
                });
            }

            let is_type_symbol = self.expr_is_type_symbol(receiver);

            match self.static_type_annotation_for_expr(receiver) {
                Ok(type_ann) if !self.should_runtime_type_query(&type_ann) => {
                    // Preserve receiver side effects for expression receivers.
                    // For type symbols (e.g. Point.type()), skip value codegen.
                    if !is_type_symbol {
                        self.compile_expr(receiver)?;
                        self.emit(Instruction::simple(OpCode::Pop));
                    }

                    let idx = self
                        .program
                        .add_constant(Constant::TypeAnnotation(type_ann));
                    self.emit(Instruction::new(
                        OpCode::PushConst,
                        Some(Operand::Const(idx)),
                    ));
                }
                Ok(_) => {
                    self.compile_expr(receiver)?;
                    self.emit(Instruction::new(
                        OpCode::BuiltinCall,
                        Some(Operand::Builtin(BuiltinFunction::TypeOf)),
                    ));
                }
                Err(err) => {
                    if is_type_symbol {
                        return Err(err);
                    }
                    self.compile_expr(receiver)?;
                    self.emit(Instruction::new(
                        OpCode::BuiltinCall,
                        Some(Operand::Builtin(BuiltinFunction::TypeOf)),
                    ));
                }
            }

            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // Universal formatting conversion: `expr.to_string()`.
        // Lower directly to FormatValueWithMeta so it shares exactly the same
        // rendering path as interpolation/print.
        //
        // HOWEVER: if the receiver's type has a user-defined `to_string` method
        // (via an extend block or impl), we must NOT short-circuit here — the
        // user method should shadow the builtin.  We check this by looking for
        // any compiled function whose name ends in `.to_string`, `.toString`,
        // `::to_string`, or `::toString`.
        if (method == "to_string" || method == "toString")
            && !self.has_any_user_defined_method(method)
        {
            if !args.is_empty() {
                return Err(ShapeError::SemanticError {
                    message: "to_string() does not take any arguments".to_string(),
                    location: Some(self.span_to_source_location(receiver.span())),
                });
            }

            if self.try_compile_static_type_to_string(receiver)? {
                return Ok(());
            }

            self.compile_expr(receiver)?;

            let count = self.program.add_constant(Constant::Int(1));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(count)),
            ));
            self.emit(Instruction::new(
                OpCode::BuiltinCall,
                Some(Operand::Builtin(BuiltinFunction::FormatValueWithMeta)),
            ));
            self.last_expr_schema = None;
            // D-β string-join receiver-kind fix (v0.3 KC #6(d), 2026-05-22):
            // `.toString()` / `.to_string()` always returns a `string`. The
            // pre-fix code cleared `last_expr_type_info` to None, which made
            // downstream string-Add operations infer the RHS as `unknown` and
            // surface "Cannot infer types for binary operation `Add`: operand
            // types are `string` and `unknown`". The cascade hit
            // monomorphizing `Vec.join`'s body (`result + self[i].toString()`)
            // for any element kind, which raised the compile error inside
            // `ensure_monomorphic_function`. The unrestored
            // `current_blob_builder` (the `?`-early-exit between take and
            // restore in `compile_function_body`) then leaked Vec.join's
            // builder into `build_content_addressed_program`, which finalized
            // it as the `__main__` blob (arity=0 synthetic). The `__main__`
            // blob disappeared, the linker entry pointed to Vec.join's body,
            // execution started inside Vec.join with self/separator slots
            // uninitialized (Bool sentinel) → "no method 'len' on receiver
            // kind Bool". Per ADR-006 §2.7.5 stamp-at-compile-time, the
            // producer-site IS the `toString` builtin — its return kind is
            // statically known. No fabrication, no Bool-default.
            self.last_expr_type_info = Some(crate::type_tracking::VariableTypeInfo::named(
                "string".to_string(),
            ));
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        if let Expr::Identifier(namespace_name, namespace_span) = receiver {
            if self.is_module_namespace_name(namespace_name)
                && self.resolve_local(namespace_name).is_none()
                && !self
                    .mutable_closure_captures
                    .contains_key(namespace_name.as_str())
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Module namespace calls must use `::`. Replace `{}.{}` with `{}::{}(...)`.",
                        namespace_name, method, namespace_name, method
                    ),
                    location: Some(self.span_to_source_location(*namespace_span)),
                });
            }

            // Removed legacy CSV namespace entrypoint.
            // Keep this specific to unresolved namespace-like access so local
            // variables named `csv` can still expose their own `load` method.
            if method == "load"
                && namespace_name == "csv"
                && self.resolve_local(namespace_name).is_none()
                && !self.mutable_closure_captures.contains_key(namespace_name)
            {
                return Err(ShapeError::SemanticError {
                    message: "csv.load(...) has been removed. Use a module-scoped data source API from a configured extension module."
                        .to_string(),
                    location: Some(self.span_to_source_location(*namespace_span)),
                });
            }

            if self.compile_type_namespace_builtin_call(
                namespace_name,
                method,
                &[],
                args,
                *namespace_span,
            )? {
                return Ok(());
            }
        }

        // Comptime mini-programs may include scoped helper functions (`m::f`) without
        // materializing a runtime module object for `m`. Prefer direct scoped dispatch.
        if let Expr::Identifier(namespace, _) = receiver {
            let scoped_name = format!("{}::{}", namespace, method);
            if self.find_function(&scoped_name).is_some() {
                return self.compile_expr_function_call(&scoped_name, &[], args, receiver.span());
            }
        }

        // Compile-time enforcement: resample/between require an Indexed table
        if method == "resample" || method == "between" {
            if let Expr::Identifier(name, span) = receiver {
                let is_indexed = self
                    .resolve_local(name)
                    .and_then(|idx| self.type_tracker.get_local_type(idx))
                    .map(|info| info.is_indexed())
                    .unwrap_or(false);
                let is_table = self
                    .resolve_local(name)
                    .and_then(|idx| self.type_tracker.get_local_type(idx))
                    .map(|info| info.is_datatable())
                    .unwrap_or(false);
                if is_table && !is_indexed {
                    return Err(ShapeError::RuntimeError {
                        message: format!(
                            "{}() requires an indexed table. Use .indexBy(row => row.column) first",
                            method
                        ),
                        location: Some(self.span_to_source_location(*span)),
                    });
                }
            }
        }

        // ADR-006 §2.7.24 Q25.C: detect dyn-typed receiver and emit
        // `OpCode::DynMethodCall` (bypassing the standard CallMethod
        // path). Detection runs BEFORE receiver compilation because
        // `compile_expr` overwrites the compiler-state we'd otherwise
        // need (the `last_expr_*` family), and the dispatch shape is
        // determined by the receiver's compile-time `dyn T` annotation,
        // not the runtime kind.
        //
        // Round-2 scope: only `Identifier`-shaped receivers are dyn-tracked
        // (the locals registered in `dyn_locals` / `dyn_module_bindings`).
        // Wider receiver shapes (`(foo()).method()` where `foo()`
        // returns `dyn T`) need return-type propagation through
        // `last_expr_type_info`; deferred to a follow-up sub-cluster
        // per ADR-006 §2.7.24 Q25.C.6 (IC layer would consume this for
        // devirtualization).
        let dyn_trait_name: Option<String> = if let Expr::Identifier(name, _) = receiver {
            if let Some(local_idx) = self.resolve_local(name) {
                self.dyn_locals.get(&local_idx).cloned()
            } else {
                let scoped = self
                    .resolve_scoped_module_binding_name(name)
                    .unwrap_or_else(|| name.to_string());
                self.module_bindings
                    .get(&scoped)
                    .copied()
                    .and_then(|idx| self.dyn_module_bindings.get(&idx).cloned())
            }
        } else {
            None
        };

        // ADR-006 §2.7.27 / Item 4 ruling (W17-mutation-writeback): detect
        // whether this method call needs a `&mut self` write-back after
        // the standard `CallMethod` dispatch. The decision is made BEFORE
        // compiling the receiver because `compile_expr` overwrites
        // `last_expr_*` state and we need the receiver-shape captured
        // upfront. Three conditions: (1) receiver is an Identifier (so
        // there's a binding location to write back to); (2) the binding
        // is tracked as a recognised COW container kind (HashSet /
        // HashMap / Deque / PriorityQueue / Array); (3) the method name
        // matches the kind's `MUT_SELF_*` set in `method_registry`.
        //
        // Interior-mutability primitives (Mutex / Atomic / Lazy /
        // Channel) deliberately do NOT register a container-kind in
        // `mut_self_container_locals`, so their `set` / `store` / `send`
        // / etc. methods do not trip this gate — the Arc identity is
        // preserved through interior mutability and no writeback is
        // required.
        let mut_self_writeback_target: Option<
            crate::compiler::mutation_writeback::MutSelfWriteBackTarget,
        > = self.resolve_mut_self_writeback_target(receiver, method);
        let mut_self_capture_writeback_target =
            self.resolve_mut_self_capture_writeback_target(receiver, method)?;

        // ADR-006 §2.7.27 amendment (W17-pop-mutation): tuple-return
        // pop-shape detection. Mutually exclusive with the self-return
        // case above (a method is registered in at most one set).
        let mut_self_tuple_return_target: Option<
            crate::compiler::mutation_writeback::MutSelfWriteBackTarget,
        > = if mut_self_writeback_target.is_some() || mut_self_capture_writeback_target.is_some() {
            // A method is never registered as both self-return and
            // tuple-return — the registries are partitioned by ABI.
            None
        } else {
            self.resolve_mut_self_tuple_return_target(receiver, method)
        };

        // R-value receivers calling a known tuple-return method need the
        // dispatch shell's silent-drop emission (Swap; Pop) — the new
        // container Arc is on the stack below the popped element with
        // no owner, so we drop it to balance refcounts. Mirror of the
        // §2.7.27 self-returning r-value silent-drop rule.
        //
        // `is_rvalue_tuple_return` triggers when (a) the method is in
        // the tuple-return registry under SOME container kind, AND (b)
        // the receiver is not identifier-rooted with a tracked
        // container kind. This includes both genuine r-value receivers
        // (e.g. `make_deque().popBack()`) and identifier receivers whose
        // binding wasn't tracked as a container kind (e.g. a function
        // parameter the compiler didn't see constructed) — in both
        // cases the handler still side-channel-publishes NewSelf, so
        // we must consume it.
        let is_rvalue_tuple_return =
            mut_self_tuple_return_target.is_none() && self.is_known_tuple_return_method(method);

        if mut_self_writeback_target.is_some() || mut_self_tuple_return_target.is_some() {
            // Enforce the let-vs-let-mut immutability check at the
            // method-call site: a `&mut self` call on an immutable
            // binding is the cleanest place to surface "method `add`
            // mutates the receiver; bind `s` as `let mut s = ...`".
            // The diagnostic flows through the existing
            // `check_named_binding_write_allowed` which already handles
            // both local-slot and module-binding cases. Applies to both
            // ABI variants — pop-shaped mutating methods on `let`
            // bindings are the same footgun as self-returning ones.
            if let Expr::Identifier(name, span) = receiver {
                let source_loc = self.span_to_source_location(*span);
                self.check_named_binding_write_allowed(name, Some(source_loc))?;
            }
        }

        // Compile receiver (the object/series being called)
        self.compile_expr(receiver)?;
        let receiver_schema = self.last_expr_schema;
        let receiver_type_info = self.last_expr_type_info.clone();
        // Capture receiver's numeric type for extend method return type
        // propagation. U4-4: derived from the receiver's one resolved Type.
        let receiver_numeric_type = self.numeric_type_of(receiver);
        // Capture receiver's extend type before args compilation overwrites compiler state.
        let receiver_extend_type =
            self.resolve_receiver_extend_type(receiver, &receiver_type_info, receiver_schema);
        // Native array PHF methods take precedence when the receiver is
        // statically proven as `Array<T>`. The core `extend Vec<T>` bodies for
        // these names are useful documentation/fallbacks, but several still
        // depend on generic scalar/operator surfaces (`len`, `cmp`, generic
        // recursion) that are not the strict typed-array carrier proof. The
        // proof here is compile-time `ConcreteType::Array(_)`, not runtime
        // value inspection.
        let receiver_concrete_type = concrete_type_for_expr(self, receiver);
        let static_array_callback_arity = match args.first() {
            Some(Expr::FunctionExpr { params, .. }) => Some(params.len()),
            Some(Expr::Identifier(name, _)) => self
                .find_function(name)
                .map(|idx| self.program.functions[idx].arity as usize),
            _ => None,
        };
        let array_hof_with_index_form = matches!(method, "map" | "filter" | "groupBy");
        let receiver_is_array = matches!(
            receiver_concrete_type.as_ref(),
            Some(ConcreteType::Array(_))
        );
        if receiver_is_array
            && array_hof_with_index_form
            && let Some(arity) = static_array_callback_arity
            && !(arity == 1 || arity == 2)
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "`{method}` callback must accept one parameter `(element)` or two \
                     `(element, index)`; statically found arity {arity}"
                ),
                location: args
                    .first()
                    .map(|arg| self.span_to_source_location(arg.span())),
            });
        }
        // WF-1A Item 4 (hashmap-filter-garbage, audit 2026-07-04 §4(d)): the
        // `*Indexed` rewrite below must fire ONLY for a statically-proven Array
        // receiver. Without the `receiver_is_array` gate, an arity-2 closure on
        // a NON-array receiver (e.g. `HashMap.filter(|k, v| ...)`) rewrote the
        // method name to the Array-only `filterIndexed`, which no HashMap/Set
        // registry map registers — the VM cleanly errored "no method
        // filterIndexed on receiver kind Ptr(HashMap)" while the JIT dispatch
        // produced a nondeterministic garbage pointer-int (exit 0). Gating on
        // `receiver_is_array` keeps a HashMap/Set arity-2 `filter`/`map` under
        // its plain name so it dispatches to the collection's own 2-arg handler
        // (`HASHMAP_METHODS["filter"]` -> `v2_filter`, etc.) identically in both
        // modes. (`prefer_native_array_method` below already ANDs
        // `receiver_is_array`; this gate additionally guards the name rewrite.)
        let indexed_native_array_callback = receiver_is_array
            && array_hof_with_index_form
            && static_array_callback_arity == Some(2);
        let prefer_native_array_method = (prefer_native_array_phf_method(method)
            || indexed_native_array_callback)
            && receiver_is_array;
        let emitted_method_name = match (method, indexed_native_array_callback) {
            ("map", true) => "mapIndexed",
            ("filter", true) => "filterIndexed",
            ("groupBy", true) => "groupByIndexed",
            _ => method,
        };
        let native_array_element_kind = receiver_concrete_type
            .as_ref()
            .and_then(array_element_kind_from_concrete_type);
        let static_array_hof_result_type = self.static_array_hof_result_concrete_type(
            receiver,
            receiver_concrete_type.as_ref(),
            method,
            args,
        );

        // Resolve closure-row schema from the receiver contract.
        // `receiver` was compiled immediately above and may carry Table<T> metadata.
        if self.is_datatable_closure_method(method) {
            if let Some(ref info) = receiver_type_info {
                if let Some((schema_id, type_name)) = Self::table_schema_from_type_info(info) {
                    self.closure_row_schema = Some((schema_id, type_name));
                }
            } else if let Some(schema_id) = receiver_schema {
                if let Some((schema_id, type_name)) =
                    self.extract_table_schema_from_callable_field(schema_id, method)
                {
                    self.closure_row_schema = Some((schema_id, type_name));
                }
            }
        }

        // Save the receiver's Table<T> schema BEFORE compiling args.
        // Closure compilation resets expression metadata, so we must save it here.
        let receiver_table_schema = receiver_type_info
            .as_ref()
            .and_then(Self::table_schema_from_type_info);

        if method == "select" && args.len() == 1 {
            if let Some((schema_id, ref type_name)) = receiver_table_schema {
                if matches!(args.first(), Some(Expr::FunctionExpr { .. })) {
                    let Some(columns) =
                        self.static_table_select_columns(&args[0], schema_id, type_name)?
                    else {
                        return Err(ShapeError::SemanticError {
                            message:
                                "Table.select(lambda) requires a statically provable direct row-field projection"
                                    .to_string(),
                            location: Some(self.span_to_source_location(args[0].span())),
                        });
                    };
                    for column in &columns {
                        self.compile_literal(&Literal::String(column.clone()))?;
                    }
                    let method_id = shape_value::MethodId::from_name(method);
                    let string_idx = self.program.add_string(method.to_string());
                    let rtt = Self::resolve_type_tag(receiver_numeric_type, &receiver_type_info);
                    self.emit(Instruction::new(
                        OpCode::CallMethod,
                        Some(Operand::TypedMethodCall {
                            method_id: method_id.0,
                            arg_count: columns.len() as u16,
                            string_id: string_idx,
                            receiver_type_tag: rtt,
                        }),
                    ));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.closure_row_schema = None;
                    self.pending_closure_param_types = None;
                    self.clear_last_expr_reference_result();
                    return Ok(());
                }
            }
        }

        // Typed-object callable field dispatch:
        // `obj.field(args...)` where `field` is a typed property that stores a closure/function.
        // This is required for generated connection objects like `conn.candles()`.
        // Only dispatch this way when the field type could actually hold a callable
        // (Any, Object, Array). Primitive field types (int, number, bool, etc.) are
        // never callable, so `t.value()` with `value: int` must fall through to
        // the CallMethod path for trait method dispatch.
        if let Some(schema_id) = receiver_schema
            && let Some(schema) = self.type_tracker.schema_registry().get_by_id(schema_id)
            && let Some(field) = schema.get_field(method)
            && field.field_type.is_potentially_callable()
        {
            if schema_id > u16::MAX as u32 || field.offset > u16::MAX as usize {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "typed-field metadata exceeds limits for method-style field call '{}'",
                        method
                    ),
                    location: Some(self.span_to_source_location(receiver.span())),
                });
            }

            let operand = Operand::TypedField {
                type_id: schema_id as u16,
                field_idx: field.index as u16,
                field_type_tag: field_type_to_tag(&field.field_type),
            };
            self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));

            // wave7 finance-field-arith-gap (repair): call-argument position.
            self.call_argument_depth += 1;
            for arg in args {
                if let Err(err) = self.compile_expr_as_value_or_placeholder(arg) {
                    self.call_argument_depth -= 1;
                    return Err(err);
                }
            }
            self.call_argument_depth -= 1;

            let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::simple(OpCode::CallValue));

            self.last_expr_type_info = self
                .extract_table_schema_from_callable_field(schema_id, method)
                .map(|(sid, type_name)| VariableTypeInfo::datatable(sid, type_name));
            self.last_expr_schema = self
                .last_expr_type_info
                .as_ref()
                .and_then(Self::value_schema_from_type_info);
            self.closure_row_schema = None;
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // Strict-typing-sweep (Cluster 3): bidirectional closure inference for HOFs.
        // For known HOF method names operating on arrays, resolve the receiver's
        // element type and use it to type the closure arg's user params. The
        // closure-compile path consumes `pending_closure_param_types`.
        self.install_pending_closure_param_types_for_hof(receiver, method, args)?;

        // Compile arguments (closure_row_schema is consumed during closure compilation).
        //
        // W18 arrays/vectors: `Array<T>.concat([])` has a compile-time receiver
        // proof for the empty argument's element type (`T`). Thread that proof
        // through the same `pending_variable_typed_array_kind` hand-off used by
        // annotated empty arrays so the literal lowers to `NewTypedArray*(0)`
        // instead of the deleted untyped `NewArray(0)` placeholder.
        // wave7 finance-field-arith-gap (repair): mark call-argument position so
        // a bare implicit-generic function identifier passed as a HOF argument
        // (`arr.map(double)`) is exempt from the function-as-value capture guard.
        self.call_argument_depth += 1;
        for (idx, arg) in args.iter().enumerate() {
            let contextual_empty_array_kind = if prefer_native_array_method
                && method == "concat"
                && idx == 0
            {
                match arg {
                    Expr::Array(elements, _) if elements.is_empty() => native_array_element_kind,
                    _ => None,
                }
            } else {
                None
            };
            let result = if let Some(kind) = contextual_empty_array_kind {
                let saved = self.pending_variable_typed_array_kind;
                self.pending_variable_typed_array_kind = Some(kind);
                let result = self.compile_expr_as_value_or_placeholder(arg);
                self.pending_variable_typed_array_kind = saved;
                result
            } else {
                self.compile_expr_as_value_or_placeholder(arg)
            };
            if let Err(err) = result {
                self.call_argument_depth -= 1;
                return Err(err);
            }
        }
        self.call_argument_depth -= 1;

        // Clear closure_row_schema after compiling args (in case it wasn't consumed)
        self.closure_row_schema = None;
        // Clear closure-arg type hints in case the closure literal was never reached.
        self.pending_closure_param_types = None;

        // ADR-006 §2.7.24 Q25.C: emit `DynMethodCall` for dyn-typed
        // receivers. Stack at this point is `[receiver, arg1, ...,
        // argN]`. The opcode consumes them plus a string id for the
        // method name and an arg-count, and dispatches through the
        // receiver's vtable per §Q25.C.5 `VTableEntry`.
        if let Some(_trait_name) = dyn_trait_name.as_ref() {
            let string_idx = self.program.add_string(method.to_string());
            self.emit(Instruction::new(
                OpCode::DynMethodCall,
                Some(Operand::TypedMethodCall {
                    method_id: shape_value::MethodId::from_name(method).0,
                    arg_count: args.len() as u16,
                    string_id: string_idx,
                    receiver_type_tag: 0xFF,
                }),
            ));
            self.last_expr_schema = None;
            self.last_expr_type_info = None;
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // UFCS: If a user-defined function exists with this name, prefer it over built-in methods.
        // This allows `extend` blocks to override built-in methods for specific types.
        // Rewrite `receiver.method(args)` → `method(receiver, args)`.
        //
        // Check bare function name first (user-defined free functions), then
        // extend-method qualified name "Type.method" using the captured receiver type.
        // For numeric types, also check parent type: Int → Number (Int is a subtype of
        // Number for method dispatch, so `extend Number` methods apply to Int values).
        let extend_func_idx = if prefer_native_array_method {
            None
        } else {
            receiver_extend_type.as_deref().and_then(|type_name| {
                let qualified = format!("{}.{}", type_name, method);
                self.find_function(&qualified).or_else(|| {
                    // Try parent type for subtypes (Int → Number)
                    let parent = match type_name {
                        "Int" => Some("Number"),
                        _ => None,
                    };
                    parent.and_then(|p| {
                        let parent_qualified = format!("{}.{}", p, method);
                        self.find_function(&parent_qualified)
                    })
                })
            })
        };
        let free_func_idx = if prefer_native_array_method {
            None
        } else {
            self.find_function(method)
        };
        // D-γ window_over_partition_by hang fix (v0.3 KC #6(e), 2026-05-22):
        // a UFCS-resolved generic extend method (e.g. `Vec.map<T,U>`) has
        // its body skipped at compile time (functions.rs:201-207 — generic
        // bodies stay in `function_defs` only, awaiting monomorphization).
        // If monomorphization fails for the concrete receiver/arg types
        // (e.g. `Vec<Struct>.map` where the closure-aware resolver bails on
        // the struct element kind and the type-only resolver returns None
        // for the same reason), the previous code unconditionally emitted
        // `Call(generic_idx)`. The generic blob has no instructions and no
        // entry in `blob_name_to_hash`, so the content-addressed linker's
        // `remap_fid` (linker.rs:105) takes the ZERO-sentinel branch,
        // fails the `name_to_id[callee_name]` lookup, and falls back to
        // `current_function_id` — rewriting the call target to `__main__`
        // itself. The program then recurses through `__main__` until stack
        // overflow / SIGKILL. Fix: when the resolved function is generic
        // and monomorphization fails, skip the UFCS branch and let the
        // standard `CallMethod` runtime dispatch handle it — that path
        // surfaces a clean NotImplemented error from the PHF method
        // registry (e.g. ckpt2_surface for typed-array methods), preserving
        // the surface-and-stop discipline rather than silently hanging.
        let ufcs_candidate_idx = extend_func_idx
            .or(free_func_idx)
            .filter(|&idx| self.current_function != Some(idx));
        let is_generic_unmonomorphizable = if let Some(idx) = ufcs_candidate_idx {
            let func_name = self.program.functions[idx].name.clone();
            let is_generic = self
                .function_defs
                .get(&func_name)
                .and_then(|d| d.type_params.as_ref())
                .is_some_and(|tps| !tps.is_empty());
            if !is_generic {
                None
            } else {
                // Probe monomorphization without compiling default args yet.
                // If it succeeds, the UFCS branch below will re-run it and
                // hit the cache; if it fails, we know to skip the UFCS
                // branch entirely. ADR-009 A3 (S2): a HARD specialized-body
                // compile error propagates out of the probe (`?`) — it is
                // the user's real diagnostic, not a reason to fall back.
                let static_idx = match self.try_specialize_concrete_user_method_call(
                    &func_name,
                    receiver,
                    args,
                    call_site_span,
                ) {
                    Ok(Some(i)) => Some(i),
                    Ok(None) => self.try_monomorphize_method_call(
                        &func_name,
                        receiver,
                        args,
                        call_site_span,
                    )?,
                    // ADR-009 A3 (review round 1) — a HARD specialized-body
                    // compile error from the call-site specialization path is
                    // ALSO the user's real diagnostic; swallowing it here and
                    // falling through to `try_monomorphize_method_call` let
                    // the two resolution paths diverge (one failing, one
                    // succeeding), after which the second resolution attempt
                    // reused the registered-but-never-compiled specialization
                    // (empty body → silent wrong output). Propagate.
                    Err(err) => return Err(err),
                };
                if static_idx.is_none() {
                    Some(idx)
                } else {
                    None
                }
            }
        } else {
            None
        };
        if let Some(func_idx) = extend_func_idx
            .or(free_func_idx)
            .filter(|&idx| self.current_function != Some(idx))
            .filter(|&idx| Some(idx) != is_generic_unmonomorphizable)
        {
            // UFCS rewrite: receiver already compiled (on stack), args already compiled.
            // Stack is: [receiver, arg1, arg2, ...] — receiver is first, which is what we want.
            // For missing args, compile the param's `default_value` expression (if
            // declared); else pad with `Unit` (preserves prior behavior for params
            // without defaults). This mirrors the regular Call path
            // (lines 1175-1208). The default-expression compile site lets stdlib
            // extend methods like `Vec.slice(start: int, end: int = -1)` accept
            // the single-arg form (`arr.slice(start)`) without the caller having
            // to push a sentinel — D-δ array_slice single-arg silent-wrong-output
            // close (v0.3-known-constraints-audit §6(f) Repro 1).
            let func_name = self.program.functions[func_idx].name.clone();
            let total_arity = self.program.functions[func_idx].arity as usize;
            let effective_total_arity = self
                .function_arity_bounds
                .get(&func_name)
                .map(|(_, eff)| *eff)
                .unwrap_or(total_arity);
            let actual_arity_with_self = args.len() + 1;
            self.compile_missing_ufcs_default_args(
                &func_name,
                func_idx,
                actual_arity_with_self,
                effective_total_arity,
            )?;
            let call_arity = actual_arity_with_self.max(effective_total_arity);

            // --- Monomorphization: specialize generic extend methods ---
            //
            // When the resolved function has type parameters (e.g. `Vec<T>.indexOf`
            // where T is generic), try to monomorphize it for the receiver's
            // concrete element type. This produces a specialized function that
            // the v2 pipeline can emit typed opcodes for.
            //
            // Falls back to the generic function index on any failure — but the
            // D-γ guard above ensures we only reach this fallback for
            // non-generic functions (whose generic-empty body is the actual
            // compiled body) or for generic functions where the probe
            // succeeded (so monomorphization here will hit the cache).
            let call_func_idx = match self.try_specialize_concrete_user_method_call(
                &func_name,
                receiver,
                args,
                call_site_span,
            )? {
                Some(idx) => idx,
                None => self
                    .try_monomorphize_method_call(&func_name, receiver, args, call_site_span)?
                    .unwrap_or(func_idx),
            };

            let arg_count = self.program.add_constant(Constant::Int(call_arity as i64));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));

            let call_func_name = self.program.functions[call_func_idx].name.clone();
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(
                    call_func_idx as u16,
                ))),
            ));
            // Record callee as a blob dependency
            if let Some(ref mut blob) = self.current_blob_builder {
                blob.record_call(&call_func_name);
            }
            self.last_expr_schema = None;
            if self.stamp_last_expr_from_function_return_annotation(&call_func_name) {
                self.clear_last_expr_reference_result();
                return Ok(());
            }
            // Propagate return type for UFCS method calls.
            // U4-4: the extend-method receiver-numeric-type propagation (which
            // wrote the deleted `last_expr_numeric_type` register for chaining)
            // is removed — a chained binop derives the result kind from the one
            // resolved Type.
            // UFCS to user function: type-preserving methods still propagate Table<T>
            if self.is_type_preserving_table_method(method) {
                self.last_expr_type_info = receiver_type_info;
            } else {
                self.last_expr_type_info = None;
            }
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // BUG-TR2 fix: Check for trait impl methods BEFORE falling through to builtin dispatch.
        // When the receiver has a known type (e.g., TypedObject with type_name "MyType"),
        // check if a trait impl method "MyType::method" or extend method "MyType.method"
        // exists. If so, dispatch it via direct Call instead of letting the builtin
        // with the same name shadow it.
        {
            // Use receiver_extend_type (covers both TypedObjects and primitives).
            // For subtypes (Int → Number), also try parent type methods.
            let extend_type_names: Vec<&str> = match receiver_extend_type.as_deref() {
                Some("Int") => vec!["Int", "Number"],
                Some(t) => vec![t],
                None => vec![],
            };
            // Check impl methods (Type::method) and extend methods (Type.method)
            let scoped_func_idx = if prefer_native_array_method {
                None
            } else {
                extend_type_names.iter().find_map(|type_name| {
                    let scoped_name = format!("{}::{}", type_name, method);
                    let extend_name = format!("{}.{}", type_name, method);
                    self.find_function(&scoped_name)
                        .or_else(|| self.find_function(&extend_name))
                })
            };
            // Also check trait_method_symbols for named impls
            let trait_func_idx = scoped_func_idx
                .is_none()
                .then(|| {
                    if prefer_native_array_method {
                        None
                    } else {
                        extend_type_names.iter().find_map(|type_name| {
                            self.program
                                .find_default_trait_impl_for_type_method(type_name, method)
                                .map(|s| s.to_string())
                                .and_then(|impl_func_name| self.find_function(&impl_func_name))
                        })
                    }
                })
                .flatten();

            // D-γ window_over_partition_by hang fix (v0.3 KC #6(e), 2026-05-22):
            // parallel guard to the extend-method UFCS site above — see the
            // comment there for the root-cause analysis. When the resolved
            // impl/trait method is a generic-no-body and monomorphization
            // fails, skip this branch so the standard `CallMethod` runtime
            // dispatch handles it (clean NotImplemented error vs. silent
            // hang from the linker's `current_function_id` fallback).
            let scoped_candidate_idx = scoped_func_idx
                .or(trait_func_idx)
                .filter(|&idx| self.current_function != Some(idx));
            let scoped_is_generic_unmonomorphizable = if let Some(idx) = scoped_candidate_idx {
                let func_name = self.program.functions[idx].name.clone();
                let is_generic = self
                    .function_defs
                    .get(&func_name)
                    .and_then(|d| d.type_params.as_ref())
                    .is_some_and(|tps| !tps.is_empty());
                if !is_generic {
                    None
                } else {
                    // ADR-009 A3 (S2): HARD specialized-body compile errors
                    // propagate out of the probe (`?`).
                    let mono_idx = self.try_monomorphize_method_call(
                        &func_name,
                        receiver,
                        args,
                        call_site_span,
                    )?;
                    if mono_idx.is_none() { Some(idx) } else { None }
                }
            } else {
                None
            };
            if let Some(func_idx) = scoped_func_idx
                .or(trait_func_idx)
                .filter(|&idx| self.current_function != Some(idx))
                .filter(|&idx| Some(idx) != scoped_is_generic_unmonomorphizable)
            {
                let func_name = self.program.functions[func_idx].name.clone();
                let total_arity = self.program.functions[func_idx].arity as usize;
                let effective_total_arity = self
                    .function_arity_bounds
                    .get(&func_name)
                    .map(|(_, eff)| *eff)
                    .unwrap_or(total_arity);
                let actual_arity_with_self = args.len() + 1;
                // Compile each missing arg's declared `default_value` (or pad
                // with Unit when none is declared) — same logic as the extend
                // UFCS site above; see that comment for rationale.
                self.compile_missing_ufcs_default_args(
                    &func_name,
                    func_idx,
                    actual_arity_with_self,
                    effective_total_arity,
                )?;
                let call_arity = actual_arity_with_self.max(effective_total_arity);

                // --- Monomorphization: specialize generic impl/trait methods ---
                //
                // When an impl method has synthesized type parameters (e.g.
                // `Array::findIndex` with T from the receiver's element type),
                // try to monomorphize it for the receiver's concrete type.
                // Falls back to the generic function index on any failure —
                // but the D-γ guard above ensures we only reach this fallback
                // for non-generic functions or generic functions where the
                // probe succeeded (cache hit).
                let call_func_idx = match self.try_specialize_concrete_user_method_call(
                    &func_name,
                    receiver,
                    args,
                    call_site_span,
                )? {
                    Some(idx) => idx,
                    None => self
                        .try_monomorphize_method_call(&func_name, receiver, args, call_site_span)?
                        .unwrap_or(func_idx),
                };

                let arg_count = self.program.add_constant(Constant::Int(call_arity as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));

                let call_func_name = self.program.functions[call_func_idx].name.clone();
                self.emit(Instruction::new(
                    OpCode::Call,
                    Some(Operand::Function(shape_value::FunctionId(
                        call_func_idx as u16,
                    ))),
                ));
                if let Some(ref mut blob) = self.current_blob_builder {
                    blob.record_call(&call_func_name);
                }
                self.last_expr_schema = None;
                if self.stamp_last_expr_from_function_return_annotation(&call_func_name) {
                    self.clear_last_expr_reference_result();
                    return Ok(());
                }
                if self.is_type_preserving_table_method(method) {
                    self.last_expr_type_info = receiver_type_info;
                } else {
                    self.last_expr_type_info = None;
                }
                self.clear_last_expr_reference_result();
                return Ok(());
            }
        }

        // Also check built-in intrinsics for UFCS (skip if it's a known built-in method name)
        if !Self::is_known_builtin_method(method) {
            if let Some(resolution) = self.classify_builtin_function(method) {
                let builtin = match resolution {
                    BuiltinNameResolution::Surface { builtin, .. } => builtin,
                    BuiltinNameResolution::InternalOnly { builtin, .. }
                        if self.allow_internal_builtins =>
                    {
                        builtin
                    }
                    BuiltinNameResolution::InternalOnly { .. } => {
                        return Err(ShapeError::SemanticError {
                            message: self.internal_intrinsic_error_message(method, resolution),
                            location: Some(self.span_to_source_location(receiver.span())),
                        });
                    }
                };

                // UFCS to builtin: receiver + args already on stack
                let arg_count = self
                    .program
                    .add_constant(Constant::Int((args.len() + 1) as i64));
                self.emit(Instruction::new(
                    OpCode::PushConst,
                    Some(Operand::Const(arg_count)),
                ));
                self.emit(Instruction::new(
                    OpCode::BuiltinCall,
                    Some(Operand::Builtin(builtin)),
                ));
                self.last_expr_schema = None;
                // Propagate known return type for UFCS builtin method calls
                if self.is_type_preserving_table_method(method) {
                    self.last_expr_type_info = receiver_type_info;
                } else {
                    self.last_expr_type_info = None;
                }
                self.clear_last_expr_reference_result();
                return Ok(());
            }
        }

        // Standard method call dispatch (runtime via CallMethod opcode)
        // Resolve method name to a typed MethodId at compile time
        let method_id = shape_value::MethodId::from_name(method);
        let string_idx = self.program.add_string(emitted_method_name.to_string());

        // Resolve receiver ConcreteType tag for type-tagged dispatch
        let rtt = Self::resolve_type_tag(receiver_numeric_type, &receiver_type_info);

        self.emit(Instruction::new(
            OpCode::CallMethod,
            Some(Operand::TypedMethodCall {
                method_id: method_id.0,
                arg_count: args.len() as u16,
                string_id: string_idx,
                receiver_type_tag: rtt,
            }),
        ));

        // ADR-006 §2.7.27 / Item 4 ruling: post-CallMethod write-back.
        // The handler returned a fresh `Arc<HashSetData>` /
        // `Arc<HashMapData>` / etc. (possibly cloned via
        // `Arc::make_mut`). `Dup` bumps the heap refcount so we have
        // two independent shares of the new Arc; `StoreLocal recv`
        // pops one and writes it back to the receiver's binding slot
        // (the existing `stack_write_kinded` drops the slot's prior
        // share via `drop_with_kind`). The remaining share stays on
        // the stack as the expression value of the method call.
        //
        // For interior-mutability primitives (Mutex / Atomic / Lazy /
        // Channel), `resolve_mut_self_writeback_target` returns None
        // because their container kinds are not registered in
        // `mut_self_container_locals`. The Arc identity is preserved
        // through interior mutability; no writeback is needed.
        if let Some(target) = mut_self_writeback_target {
            use crate::compiler::mutation_writeback::MutSelfWriteBackTarget;
            self.emit(Instruction::simple(OpCode::Dup));
            match target {
                MutSelfWriteBackTarget::Local(local_idx) => {
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                }
                MutSelfWriteBackTarget::ModuleBinding(binding_idx) => {
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
            }
        } else if let Some(target) = mut_self_capture_writeback_target {
            self.emit(Instruction::simple(OpCode::Dup));
            match target {
                CaptureMutSelfWriteBackTarget::OwnedMutable {
                    capture_idx,
                    opcode,
                }
                | CaptureMutSelfWriteBackTarget::Shared {
                    capture_idx,
                    opcode,
                } => {
                    self.emit(Instruction::new(opcode, Some(Operand::Local(capture_idx))));
                }
            }
        } else if let Some(target) = mut_self_tuple_return_target {
            // ADR-006 §2.7.27 amendment (W17-pop-mutation): tuple-return
            // post-call codegen. Stack at this point is
            // `[..., NewContainer, popped_element]` — the handler
            // side-channel-pushed NewContainer via `vm.push_kinded`
            // before returning the popped element, and the dispatch
            // shell then pushed the returned popped element on top.
            //
            // `Swap` flips the top two: `[..., popped_element, NewContainer]`.
            // `Store*(target)` pops NewContainer and writes it to the
            // receiver binding (existing `stack_write_kinded` releases
            // the prior occupant's share via `drop_with_kind`); the
            // popped_element remains on the stack as the call's
            // expression value.
            use crate::compiler::mutation_writeback::MutSelfWriteBackTarget;
            self.emit(Instruction::simple(OpCode::Swap));
            match target {
                MutSelfWriteBackTarget::Local(local_idx) => {
                    self.emit(Instruction::new(
                        OpCode::StoreLocal,
                        Some(Operand::Local(local_idx)),
                    ));
                }
                MutSelfWriteBackTarget::ModuleBinding(binding_idx) => {
                    self.emit(Instruction::new(
                        OpCode::StoreModuleBinding,
                        Some(Operand::ModuleBinding(binding_idx)),
                    ));
                }
            }
        } else if is_rvalue_tuple_return {
            // ADR-006 §2.7.27 amendment (W17-pop-mutation): r-value
            // receiver silent-drop. The handler side-channel-pushed
            // NewContainer before returning the popped element, so the
            // stack is `[..., NewContainer, popped_element]`. With no
            // receiver binding to write back to, `Swap; Pop` flips and
            // drops NewContainer (the `Pop` opcode's drop_with_kind
            // discipline releases the heap share cleanly). Mirror of
            // the §2.7.27 self-returning r-value silent-drop rule.
            self.emit(Instruction::simple(OpCode::Swap));
            self.emit(Instruction::simple(OpCode::Pop));
        }

        // Propagate known return type for standard method calls
        self.last_expr_schema = None;

        if let Some(ct) = static_array_hof_result_type.as_ref()
            && let Some(type_info) = tracker_type_info_from_concrete_type(ct)
        {
            self.last_expr_type_info = Some(type_info);
            self.last_expr_schema = self
                .last_expr_type_info
                .as_ref()
                .and_then(Self::value_schema_from_type_info);
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        if prefer_native_array_method {
            let method_expr = Expr::MethodCall {
                receiver: Box::new(receiver.clone()),
                method: method.to_string(),
                args: args.to_vec(),
                named_args: vec![],
                optional: false,
                span: call_site_span,
            };
            self.last_expr_type_info = self
                .infer_expr_type(&method_expr)
                .ok()
                .as_ref()
                .and_then(|ty| self.type_info_from_inferred_type(ty));
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // REAL-MOVE keep-both (v0.3.3, user 2026-06-21): `clone` returns
        // `Self`. For a struct (`TypedObject`) receiver carrying a known
        // compile-time schema, the deep-clone result has the SAME schema —
        // so a subsequent `q.x = ...` field write can resolve the slot
        // offset (without this, `last_expr_schema = None` makes the binding
        // schema-less and the field-write compiler rejects with "requires
        // compile-time field resolution"). Mirror of the type-preserving
        // table-method propagation below.
        if method == "clone" && receiver_schema.is_some() {
            self.last_expr_schema = receiver_schema;
            self.last_expr_type_info = receiver_type_info.clone();
            self.clear_last_expr_reference_result();
            return Ok(());
        }

        // Propagate Table<T> type through type-preserving methods.
        // After filter/head/tail/etc., the result is still Table<T>.
        if self.is_type_preserving_table_method(method) {
            self.last_expr_type_info = receiver_type_info.clone();
        } else if receiver_type_info
            .as_ref()
            .is_some_and(Self::type_info_is_content)
            && content_preserving_method(method)
        {
            self.last_expr_type_info = Some(content_type_info());
        } else {
            self.last_expr_type_info = None;
        }

        // Track indexBy result: extract field name from closure arg at compile time
        if (method == "indexBy" || method == "index_by") && receiver_table_schema.is_some() {
            if let Some((schema_id, ref type_name)) = receiver_table_schema {
                let index_col = args.first().and_then(Self::extract_closure_field_name);
                if let Some(col_name) = index_col {
                    self.last_expr_type_info = Some(VariableTypeInfo::indexed(
                        schema_id,
                        type_name.clone(),
                        col_name,
                    ));
                }
            }
        }

        self.clear_last_expr_reference_result();
        Ok(())
    }

    /// Try to compile a method call using local-slot-based typed opcodes.
    ///
    /// Returns `Ok(Some(()))` if the method was compiled as a typed opcode,
    /// `Ok(None)` if the method should fall through to the generic path.
    fn try_compile_typed_slot_method(
        &mut self,
        receiver: &Expr,
        method: &str,
        args: &[Expr],
    ) -> Result<Option<()>> {
        let name = match receiver {
            Expr::Identifier(name, _) => name,
            _ => return Ok(None),
        };
        let local_idx = match self.resolve_local(name) {
            Some(idx) => idx,
            None => return Ok(None),
        };

        match method {
            // `.len()` — typed length for arrays, maps, strings
            "len" if args.is_empty() => {
                if self.v2_typed_array_locals.contains_key(&local_idx) {
                    self.emit(Instruction::new(
                        OpCode::ArrayLenTyped,
                        Some(Operand::Local(local_idx)),
                    ));
                    self.last_expr_schema = None;
                    self.last_expr_type_info = None;
                    self.clear_last_expr_reference_result();
                    return Ok(Some(()));
                }
                if !self.param_locals.contains(&local_idx) {
                    let is_string = self
                        .type_tracker
                        .get_local_type(local_idx)
                        .and_then(|info| {
                            info.type_name
                                .as_deref()
                                .map(|n| n == "string" || n == "String")
                        })
                        .unwrap_or(false);
                    if is_string {
                        self.emit(Instruction::new(
                            OpCode::StringLenTyped,
                            Some(Operand::Local(local_idx)),
                        ));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.clear_last_expr_reference_result();
                        return Ok(Some(()));
                    }
                }
            }

            // U3 (SB-9 deletion): the `.get/.has/.set` local-slot fast path
            // was guarded on `v2_typed_map_locals`, which only registered the
            // deleted `TypedMap<K,V>` carrier. With one honest `HashMapData`
            // carrier, HashMap method calls dispatch through the generic
            // `CallMethod` path (MapGetStr*/MapHasStr/MapSetStr* opcodes are
            // unreachable from here and slated for removal).

            // `.push(value)` — typed array push (local-slot-based)
            "push" if args.len() == 1 => {
                if let Some(&kind) = self.v2_typed_array_locals.get(&local_idx) {
                    let opcode = match kind {
                        crate::compiler::v2_typed_emission::TypedArrayKind::I64 => {
                            Some(OpCode::ArrayPushI64)
                        }
                        crate::compiler::v2_typed_emission::TypedArrayKind::F64 => {
                            Some(OpCode::ArrayPushF64)
                        }
                        _ => None,
                    };
                    if let Some(opcode) = opcode {
                        let source_loc = self.span_to_source_location(receiver.span());
                        if !self.ref_locals.contains(&local_idx) {
                            self.check_named_binding_write_allowed(name, Some(source_loc))?;
                        }
                        self.compile_expr(&args[0])?;
                        self.emit(Instruction::new(opcode, Some(Operand::Local(local_idx))));
                        // Push the mutated array as expression result.
                        if self.ref_locals.contains(&local_idx)
                            || self.reference_value_locals.contains(&local_idx)
                        {
                            self.emit(Instruction::new(
                                OpCode::DerefLoad,
                                Some(Operand::Local(local_idx)),
                            ));
                        } else {
                            self.emit(Instruction::new(
                                OpCode::LoadLocal,
                                Some(Operand::Local(local_idx)),
                            ));
                        }
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.clear_last_expr_reference_result();
                        return Ok(Some(()));
                    }
                }
            }

            // `.charAt(index)` — typed string char access
            "charAt" if args.len() == 1 => {
                if !self.param_locals.contains(&local_idx) {
                    let is_string = self
                        .type_tracker
                        .get_local_type(local_idx)
                        .and_then(|info| {
                            info.type_name
                                .as_deref()
                                .map(|n| n == "string" || n == "String")
                        })
                        .unwrap_or(false);
                    if is_string {
                        self.compile_expr(&args[0])?;
                        self.emit(Instruction::new(
                            OpCode::StringCharAt,
                            Some(Operand::Local(local_idx)),
                        ));
                        self.last_expr_schema = None;
                        self.last_expr_type_info = None;
                        self.clear_last_expr_reference_result();
                        return Ok(Some(()));
                    }
                }
            }

            _ => {}
        }

        Ok(None)
    }

    fn compile_module_namespace_call(
        &mut self,
        namespace_name: &str,
        namespace_span: Span,
        method: &str,
        const_args: &[Expr],
        args: &[Expr],
    ) -> Result<()> {
        self.compile_module_namespace_call_on_binding(
            namespace_name,
            namespace_name,
            namespace_span,
            method,
            const_args,
            args,
        )
    }

    fn compile_module_namespace_call_on_binding(
        &mut self,
        binding_name: &str,
        namespace_name: &str,
        namespace_span: Span,
        method: &str,
        const_args: &[Expr],
        args: &[Expr],
    ) -> Result<()> {
        // Detect json.parse(text, TypeName) → rewrite to json.__parse_typed(text, schema_id).
        // When the second arg is a type identifier with a registered schema, we compile
        // a typed deserialization call that uses @alias annotations and field types.
        // Resolve canonical module path: namespace_name may be a local alias ("json")
        // or already canonical ("std::core::json").
        let canonical_module = self
            .resolve_canonical_module_path(namespace_name)
            .unwrap_or_else(|| namespace_name.to_string());
        if canonical_module == "std::core::json" && method == "parse" && args.len() == 2 {
            if let Expr::Identifier(type_name, _) = &args[1] {
                if let Some(target_schema) = self.type_tracker.schema_registry().get(type_name) {
                    let target_schema_id = target_schema.id;
                    // Rewrite: compile as json.__parse_typed(text, schema_id)
                    let schema_id_expr =
                        Expr::Literal(Literal::Number(target_schema_id as f64), args[1].span());
                    let rewritten_args = vec![args[0].clone(), schema_id_expr];
                    return self.compile_module_namespace_call_on_binding(
                        binding_name,
                        namespace_name,
                        namespace_span,
                        "__parse_typed",
                        &[],
                        &rewritten_args,
                    );
                }
            }
        }

        // Q33 / distributed §4.1.1: `remote::call(addr, fn_ref, arg0, arg1, …)`
        // is compiler-recognized — the same special-casing class as
        // `as`-casts → `__into_*`. Resolve `fn_ref` to a concrete function
        // type, positionally type-check the call args against its declared
        // param types (compile error on arity / type mismatch — never a
        // runtime coercion), lower the args to a TypedObject `_0.._n` pack
        // carrier (per-field kinds proven from the compiled args), and dispatch
        // to the internal `__call_raising` sibling with `R` instantiated from
        // `fn_ref`'s declared return type.
        if canonical_module == "std::core::remote" && (method == "call" || method == "call_async") {
            return self.compile_remote_call_elaboration(
                binding_name,
                namespace_name,
                namespace_span,
                args,
                method == "call_async",
            );
        }

        // Shape-source module exports (non-native) compile as regular functions.
        // Route namespace calls to direct function dispatch so const-template
        // specialization/comptime handlers run in the same compiler context.
        //
        // The compiled dep-module functions are qualified under the CANONICAL
        // module path (`calc::numbers::imax`), not the local alias the call site
        // uses (`numbers::imax`). Resolve the local namespace name to its
        // canonical path first, then try the canonical-qualified scoped name,
        // falling back to the literal `namespace::method` form (already-canonical
        // call sites / nested module scopes).
        let canonical_scoped_name = format!("{}::{}", canonical_module, method);
        let local_scoped_name = format!("{}::{}", namespace_name, method);
        // PRIVACY GATE: the compiled function table is name-keyed and holds
        // every dep function — public AND private. Routing on `find_function`
        // alone would expose a non-`pub` function through a namespace call
        // (`util::secret(..)`). Only route when `method` is a public export of
        // the resolved module. `Some(false)` => module is known but `method`
        // is private/absent: do NOT route (fall through to the standard
        // "module namespace not typed / undefined" diagnostic, matching the
        // named-import path which is gated on `Item::Export` membership).
        // `None` => module not in the graph (legacy inlining / native): keep
        // prior behavior.
        let member_exported = self.module_member_is_exported(&canonical_module, method);
        if !self.is_native_module_export(namespace_name, method) && member_exported != Some(false) {
            if self.find_function(&canonical_scoped_name).is_some() {
                return self.compile_expr_function_call(
                    &canonical_scoped_name,
                    const_args,
                    args,
                    namespace_span,
                );
            }
            if self.find_function(&local_scoped_name).is_some() {
                return self.compile_expr_function_call(
                    &local_scoped_name,
                    const_args,
                    args,
                    namespace_span,
                );
            }
        }

        let callee_name = format!("{}::{}", namespace_name, method);
        self.reject_const_args_for_non_generic_call(&callee_name, const_args, namespace_span)?;

        if self.is_native_module_export(namespace_name, method)
            && !self.is_native_module_export_available(namespace_name, method)
        {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module export '{}::{}' is only available in comptime contexts",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        }

        // R8 W9 B1 W17-marshal-return JIT surface-and-stop flag
        // (2026-05-25). Native module namespace calls (e.g.
        // `state::serialize(arr)` or imported `serialize(arr)` via
        // `from std::core::state use { serialize }`) emit
        // `LoadModuleBinding + GetFieldTyped + CallValue` per ADR-006
        // §2.7.26. The callee is a `Ptr(HeapKind::ModuleFn)` value; at
        // runtime VM-side this routes cleanly through
        // `invoke_module_fn_id_stub` + `project_typed_return`; JIT-side
        // `jit_call_value` ModuleFn arm at
        // `crates/shape-jit/src/ffi/control/mod.rs:704-715` silently
        // returns TAG_NULL. Set the flag so the JIT preflight refuses
        // and deopts to the bytecode interpreter via the W12
        // `[jit-fallback]` path. v0.4 root-cause fix per
        // `docs/v0.3-close-summary.md` §5.16 JIT-lowering followup.
        // Restrict to user-space main compilation (see same restriction
        // at `compile_module_builtin_function_call` above for the
        // dep-module-bootstrap rationale).
        if self.is_native_module_export(namespace_name, method)
            && self.module_scope_stack.is_empty()
        {
            self.program.has_w17_marshal_residual = true;
        }

        // D5 (WF-3E over-wire enforcement): derive the CURRENT blob's
        // `required_permissions` from the callee's actual native-stdlib-module
        // call, not from the import statement. `record_blob_permissions` stamps
        // onto `self.current_blob_builder`; while a function body is being
        // compiled that IS the per-function blob, so a transferred per-function
        // blob carries its real derived permissions (e.g. a fn calling
        // `file::write_text` carries `FsWrite`). This covers BOTH namespace
        // imports (`use std::core::http`) and named imports — the permission is
        // derived from the call site, which the import-time recording at
        // `statements.rs:1939` / `check_import_permissions` cannot do because at
        // import time `current_blob_builder` is `__main__`, never the callee's
        // blob. The §4.6 receiver load-refusal
        // (`remote.rs` `load_linked_program_with_permissions` -> linker union)
        // then operates on real hash-baked data, so a strict (granted=[]) node
        // refuses a transferred fs.write fn at LOAD.
        //
        // Unconditional by canonical module path (NOT gated on
        // `is_native_module_export`, whose registry keys the short alias `file`
        // rather than the canonical `std::core::file` the builtin-fn path routes
        // through): `record_blob_permissions` derives via
        // `capability_tags::required_permissions`, which returns `pure()` (empty)
        // for every non-capability module — user Shape modules, math, json — so a
        // spurious record is a no-op (`record_blob_permissions` skips empty
        // sets). Only the real capability modules (`std::core::file` -> FsWrite,
        // `std::core::http` -> NetConnect, `std::core::env` -> Env, …) contribute.
        //
        // Graph dependencies stamp the function that actually issues the call.
        // Only a module whose graph identity carries embedded-stdlib resolver
        // provenance receives the tightly scoped bootstrap exception; neither
        // module nesting nor a user-chosen `std::...` path grants authority.
        self.record_owned_capability_call_permissions(&canonical_module, method);

        // For native module exports, use a hidden binding so that the native
        // module object is not clobbered when a Shape artifact module with the
        // same name is compiled (the module decl overwrites the regular binding).
        let effective_binding_name = if self.is_native_module_export(namespace_name, method) {
            self.ensure_hidden_native_module_binding(namespace_name)
        } else {
            binding_name.to_string()
        };

        let binding_idx = *self
            .module_bindings
            .get(&effective_binding_name)
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "module namespace '{}' is not bound in the current scope",
                    namespace_name
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            })?;
        self.emit(Instruction::new(
            OpCode::LoadModuleBinding,
            Some(Operand::ModuleBinding(binding_idx)),
        ));
        self.last_expr_type_info = self.type_tracker.get_binding_type(binding_idx).cloned();
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);

        let schema_id = self
            .last_expr_schema
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "module namespace '{}' is not typed. Missing module schema for export '{}'",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            })?;

        let Some(schema) = self.type_tracker.schema_registry().get_by_id(schema_id) else {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module namespace '{}' schema id {} is not registered",
                    namespace_name, schema_id
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        };

        let Some(field) = schema.get_field(method) else {
            return Err(ShapeError::SemanticError {
                message: format!("module '{}' has no export '{}'", namespace_name, method),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        };

        if schema_id > u16::MAX as u32 || field.offset > u16::MAX as usize {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "module '{}' export metadata exceeds typed-field limits for '{}'",
                    namespace_name, method
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        }
        let operand = Operand::TypedField {
            type_id: schema_id as u16,
            field_idx: field.index as u16,
            field_type_tag: field_type_to_tag(&field.field_type),
        };
        self.emit(Instruction::new(OpCode::GetFieldTyped, Some(operand)));

        // Empty-in-context element-type inference (issue #14, user-ratified
        // CANONICAL-INSTANTIATE 2026-07-07): a native module export marshals its
        // arguments through the object-graph boundary (`FromSlot`/`to_json_value`),
        // an UNCONSTRAINED monomorphic sink — the declared param is the polymorphic
        // `PolymorphicArg` (`value: _`). A context-free empty array `[]` reaching
        // this sink (a bare `[]` arg, or a `children: []` field of an object-literal
        // arg, e.g. `xml::stringify({ children: [] })`) has a provably-unobserved
        // element type; instantiate it at the canonical unit `int` so it lowers to
        // the monomorphic `TypedArray<int>` empty allocator (marshals to an empty
        // array) instead of the `NewArray(0)` placeholder that SURFACEs at runtime.
        // Scoped (save/restore) to exactly this call's argument subtree.
        let saved_canonical_instantiate = self.pending_empty_array_canonical_instantiate;
        self.pending_empty_array_canonical_instantiate = true;
        let mut arg_err: Option<ShapeError> = None;
        for arg in args {
            if let Err(e) = self.compile_expr_as_value_or_placeholder(arg) {
                arg_err = Some(e);
                break;
            }
        }
        self.pending_empty_array_canonical_instantiate = saved_canonical_instantiate;
        if let Some(e) = arg_err {
            return Err(e);
        }

        let arg_count = self.program.add_constant(Constant::Int(args.len() as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(arg_count)),
        ));
        self.emit(Instruction::simple(OpCode::CallValue));

        // WF-3A-tail (time::millis inference): the inference tier returns a
        // fresh type var for a `module::fn()` call (it holds no module-export
        // signatures — see the QualifiedFunctionCall arm in
        // `type_system/inference/expressions.rs`), so an unannotated
        // `let start = time::millis()` used to erase to `unknown` and reject
        // `now - start` at binop compile time. The native module schema DOES
        // carry each export's declared scalar return type; recover it here so
        // the binding is stamped with the proven scalar type. Scoped to native
        // module scalar returns only — heap/wrapper returns (json::parse ->
        // Result<Json>, etc.) keep the existing infer/schema path so their own
        // navigation semantics are untouched. No type is fabricated: the type
        // is the declared `-> T` from the stdlib source.
        if let Some(ti) = self.native_module_declared_return_type_info(&canonical_module, method) {
            self.last_expr_type_info = Some(ti);
        } else {
            let namespace_call_expr = Expr::QualifiedFunctionCall {
                namespace: namespace_name.to_string(),
                function: method.to_string(),
                const_args: Vec::new(),
                args: args.to_vec(),
                named_args: vec![],
                span: namespace_span,
            };
            let inferred = self.infer_expr_type(&namespace_call_expr).ok();
            self.last_expr_type_info = inferred
                .as_ref()
                .and_then(|ty| self.type_info_from_inferred_type(ty));
        }
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);
        Ok(())
    }

    /// WF-3A-tail: recover a native module export's declared return type from
    /// the module schema registry (`ModuleExports.get_schema(..).return_type`,
    /// a type-name string like `"number"` or `"Result<Json, string>"`). The
    /// inference tier holds no module-export signatures, so an unannotated
    /// `let r = json::parse(..)` used to erase to unknown and force dynamic
    /// method dispatch on the marshalled `Json` enum (which does not resolve
    /// `extend Json` methods) — the json/msgpack navigation bug (#16/#17).
    ///
    /// Returns `Some` for:
    /// - scalar families (`bool` / `string` / `int` / `number` / `decimal`) —
    ///   fixes `time::millis()` operand-position inference;
    /// - fallible/optional wrappers (`Result<..>` / `Option<..>`) — carries the
    ///   Ok/Some payload type name so `match r { Ok(v) => v.method() }` binds
    ///   `v` with a proven type and resolves `extend` methods statically.
    ///
    /// Other heap returns (bare enums / arrays / objects) return `None` so the
    /// caller keeps the existing inference/schema path. The type is the declared
    /// `-> T` from the stdlib source — nothing is fabricated.
    fn native_module_declared_return_type_info(
        &self,
        canonical_module: &str,
        method: &str,
    ) -> Option<VariableTypeInfo> {
        let registry = self.extension_registry.as_ref()?;
        let module = registry.iter().rev().find(|m| m.name == canonical_module)?;
        let schema = module.get_schema(method)?;
        let return_type = schema.return_type.as_ref()?.trim();
        if let Some(scalar) = Self::builtin_scalar_type_info(return_type) {
            return Some(scalar);
        }
        // Fallible/optional wrappers: propagate the baked wrapper-type-name so
        // `propagate_assignment_type_to_slot`'s `Result<`/`Option<` guard
        // records it and the downstream `Ok(v)`/`Some(v)` binding recovers the
        // payload type (mirrors `type_info_from_annotation`'s Generic arm).
        if return_type.starts_with("Result<") || return_type.starts_with("Option<") {
            return Some(VariableTypeInfo::named(return_type.to_string()));
        }
        None
    }

    /// WF-3A-tail (operand-position inference): recover a native module export's
    /// declared SCALAR return type as an inference-tier `Type`. This is the
    /// inference-tier sibling of `native_module_declared_return_type_info`'s
    /// emit-tier let-binding stamp. The runtime inference engine holds no
    /// module-export signatures, so its `QualifiedFunctionCall` arm returns a
    /// fresh type var — which made a BARE `time::millis()` used directly as a
    /// binary operand (`time::millis() - start`) erase to `unknown` and reject
    /// the `Sub`. Consult the native module schema's declared `-> T` and return
    /// the proven scalar `ConcreteType` so the call carries its true declared
    /// type in ANY position (binary operand, argument, return, index), not only
    /// when bound to a `let`.
    ///
    /// Scoped to SCALAR returns (`bool`/`string`/`int`/`number`/`decimal`) via
    /// `canonical_script_alias`, which returns `None` for `Result<..>` /
    /// `Option<..>` / bare enums / arrays / objects — those keep the existing
    /// inference/schema path so their navigation semantics (json::parse ->
    /// Result<Json>, etc.) are untouched. `int` and `number` stay separate; the
    /// type is the declared `-> T` from the stdlib source — nothing is
    /// fabricated.
    pub(super) fn native_module_declared_scalar_return_type(
        &self,
        canonical_module: &str,
        method: &str,
    ) -> Option<Type> {
        use shape_ast::ast::TypeAnnotation;
        let registry = self.extension_registry.as_ref()?;
        let module = registry.iter().rev().find(|m| m.name == canonical_module)?;
        let schema = module.get_schema(method)?;
        let return_type = schema.return_type.as_ref()?.trim();
        let canonical =
            shape_runtime::type_system::BuiltinTypes::canonical_script_alias(return_type)?;
        Some(Type::Concrete(TypeAnnotation::Basic(canonical.to_string())))
    }

    /// WF-3A-tail: build the `"namespace::function" -> canonical-scalar-type`
    /// map handed to the semantic analyzer so a module-qualified builtin call
    /// (`time::millis()`) infers its declared scalar return type in ANY position
    /// (binary operand, call argument, index), not only when bound to a `let`.
    ///
    /// Scoped to native module SCALAR exports (`bool`/`int`/`number`/`string`/
    /// `decimal`) via `canonical_script_alias`. `Result<..>`/`Option<..>`/heap
    /// returns are omitted, so the semantic analyzer keeps its existing fresh-var
    /// path for those and json/msgpack navigation is untouched. Nothing is
    /// fabricated — each value is the declared `-> T` from the module schema;
    /// `int` and `number` stay distinct.
    pub(crate) fn build_module_qualified_scalar_returns(
        &self,
    ) -> std::collections::HashMap<String, String> {
        let mut map = std::collections::HashMap::new();
        let Some(registry) = self.extension_registry.as_ref() else {
            return map;
        };
        // A module-qualified call in source uses a LOCAL namespace name — the
        // trailing segment of an unaliased `use std::core::time` ("time") or an
        // explicit `use .. as t` alias ("t"). The extension registry keys
        // modules by their FULL canonical path ("std::core::time"). Collect,
        // per registry module, every local namespace the inference engine might
        // observe: the full path, its trailing segment, and any alias in the
        // graph/scope namespace maps that resolves to that path.
        let mut aliases_for: std::collections::HashMap<&str, Vec<String>> =
            std::collections::HashMap::new();
        for (local, canonical) in self
            .graph_namespace_map
            .iter()
            .chain(self.module_scope_sources.iter())
        {
            aliases_for
                .entry(canonical.as_str())
                .or_default()
                .push(local.clone());
        }
        for module in registry.iter() {
            let mut namespaces: Vec<String> = vec![module.name.clone()];
            if let Some(last) = module.name.rsplit("::").next() {
                if last != module.name {
                    namespaces.push(last.to_string());
                }
            }
            if let Some(aliases) = aliases_for.get(module.name.as_str()) {
                namespaces.extend(aliases.iter().cloned());
            }
            for export in module.export_names_available(self.comptime_mode) {
                let Some(schema) = module.get_schema(export) else {
                    continue;
                };
                let Some(return_type) = schema.return_type.as_ref() else {
                    continue;
                };
                let Some(scalar) = shape_runtime::type_system::BuiltinTypes::canonical_script_alias(
                    return_type.trim(),
                ) else {
                    continue;
                };
                for ns in &namespaces {
                    map.insert(format!("{}::{}", ns, export), scalar.to_string());
                }
            }
        }
        map
    }

    /// Q33 / distributed §4.1.1: elaborate a direct
    /// `remote::call(addr, fn_ref, args…)` site — the imperative analog of the
    /// `@remote` annotation path, in the same special-casing class as
    /// `as`-casts → `__into_*`.
    ///
    /// 1. Resolve `fn_ref` to a statically-known callable — a named function
    ///    declaration (R1) OR a `let`-bound closure literal (R2). Both surface
    ///    the same `(params, return_type)` signature: a named function via its
    ///    `FunctionDef`, a closure value via its retained `ClosureBodyPeek`
    ///    (`local_callable_closure_bodies` / `module_binding_callable_closure_bodies`,
    ///    the same peek the value-call return-inference path reads). §4.1.1: for
    ///    a closure value "the type is static, the callee identity is not" — the
    ///    signature type-checks at compile time; the runtime `Ptr(HeapKind::Closure)`
    ///    value (with its captures) is resolved by the sender's closure arm
    ///    (`remote_builtins.rs::call_remote`).
    /// 2. Positionally type-check each call arg against the declared param
    ///    type — a proven mismatch (or an un-provable arg against a concrete
    ///    declared param) is a COMPILE error, never a runtime coercion.
    /// 3. Lower the positional call args into a TypedObject `_0.._n` pack
    ///    carrier whose per-field kinds are the compiled arg kinds (the
    ///    supervisor-D1 tuple carrier — no new `HeapKind::Tuple`).
    /// 4. Dispatch to the internal `__call_raising` sibling (§4.1.2), then
    ///    instantiate the call-site type `R` from `fn_ref`'s declared return
    ///    type (scalars / strings project through the typed return carrier
    ///    today; heap-shaped returns are R3).
    fn compile_remote_call_elaboration(
        &mut self,
        binding_name: &str,
        namespace_name: &str,
        namespace_span: Span,
        args: &[Expr],
        async_result: bool,
    ) -> Result<()> {
        use shape_ast::ast::{FunctionParameter, TypeAnnotation};
        let surface_name = if async_result {
            "remote::call_async"
        } else {
            "remote::call"
        };

        if args.len() < 2 {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "{surface_name}(addr, fn, args…) requires at least a server address \
                     and a function reference"
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        }
        let addr_expr = args[0].clone();
        let fn_ref_expr = args[1].clone();
        let call_args = &args[2..];

        // (1) Resolve `fn_ref` to a statically-known callable and recover its
        // declared signature `(params, return_type)`. A bare identifier can name
        // either a top-level function (R1) or a `let`-bound closure value (R2).
        let fn_name = match &fn_ref_expr {
            Expr::Identifier(name, _) => name.clone(),
            _ => {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "{surface_name}: the function reference must name a \
                         statically-known function or closure binding"
                    ),
                    location: Some(self.span_to_source_location(fn_ref_expr.span())),
                });
            }
        };
        // Named-function declaration first (R1); then a retained closure-literal
        // peek for a `let`-bound closure value (R2). Both yield the same
        // `(Vec<FunctionParameter>, Option<TypeAnnotation>)` signature shape.
        // NOTE (supervisor reconciliation at the ADR-009 E3 merge): this
        // in-progress hunk resolved `fn_name` through `self.function_aliases`
        // (the replace-body-scoped `__original__` -> shadow map). E3/U11
        // DELETED that map and the `__original__` spelling itself (rejection
        // row 1: a user-spelled `__original__` must no longer resolve to the
        // shadow); the pre-annotation body is reached only through the typed
        // `ctx.original` capability now. The alias hop can never fire
        // post-E3, so it is dropped here. If this wave needs `remote::call`
        // on the pre-annotation original from inside a `replace body`, route
        // it through `ctx.original` (compiler/original_body_rewrite.rs).
        let (params, return_type): (Vec<FunctionParameter>, Option<TypeAnnotation>) = if let Some(
            func_def,
        ) =
            self.function_defs.get(&fn_name).cloned()
        {
            (func_def.params, func_def.return_type)
        } else if let Some(peek) = self.remote_closure_peek(&fn_name) {
            (peek.params, peek.return_type)
        } else {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "{surface_name}: '{}' is not a statically-known function or closure binding",
                    fn_name
                ),
                location: Some(self.span_to_source_location(fn_ref_expr.span())),
            });
        };

        // (2) Arity check.
        if call_args.len() != params.len() {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "{surface_name}: '{}' expects {} argument(s), but {} were supplied",
                    fn_name,
                    params.len(),
                    call_args.len()
                ),
                location: Some(self.span_to_source_location(namespace_span)),
            });
        }

        // (2 cont.) Positional type-check each arg against the declared param
        // type. A proven mismatch is a COMPILE error (no runtime coercion); an
        // arg whose type cannot be proven against a concretely-declared param
        // is also rejected — strict typing has no `any` escape hatch.
        for (i, arg) in call_args.iter().enumerate() {
            let param = &params[i];
            let Some(param_ann) = param.type_annotation.as_ref() else {
                // Unannotated / wildcard param: nothing concrete to prove against.
                continue;
            };
            let Some(param_ct) =
                crate::compiler::v2_map_emission::concrete_type_from_annotation(param_ann)
            else {
                // Param type not concretely resolvable at this layer (user
                // struct / generic): leave to the receiver-side ABI check.
                continue;
            };
            match concrete_type_for_expr(self, arg) {
                Some(arg_ct) if arg_ct == param_ct => {}
                Some(arg_ct) => {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "{surface_name}: argument #{} to '{}' has type `{}`, but the \
                             declared parameter type is `{}`",
                            i + 1,
                            fn_name,
                            arg_ct,
                            param_ct
                        ),
                        location: Some(self.span_to_source_location(arg.span())),
                    });
                }
                None => {
                    return Err(ShapeError::SemanticError {
                        message: format!(
                            "{surface_name}: cannot statically prove the type of argument #{} \
                             to '{}' (declared parameter type `{}`) — annotate the value",
                            i + 1,
                            fn_name,
                            param_ct
                        ),
                        location: Some(self.span_to_source_location(arg.span())),
                    });
                }
            }
        }

        // (3) Lower the positional args to a TypedObject `_0.._n` pack. Fields
        // are emitted in call order, so the object's slot order is the
        // positional argument order the receiver marshals against.
        let pack_entries: Vec<ObjectEntry> = call_args
            .iter()
            .enumerate()
            .map(|(i, arg)| ObjectEntry::Field {
                key: format!("_{i}"),
                value: arg.clone(),
                type_annotation: None,
            })
            .collect();
        let pack_expr = Expr::Object(pack_entries, namespace_span);

        // Dispatch to the internal `__call_result` sibling (§4.9 / FIX C):
        // the recoverable primitive whose native body returns a real
        // `Result<R, RemoteError>` value (success → `Ok(R)`, transport /
        // protocol / remote failure → `Err(RemoteError::…)`). The raising
        // sibling `__call_raising` is left for the `@remote` before-hook (Q26).
        let rewritten = vec![addr_expr, fn_ref_expr, pack_expr];
        let internal_name = if async_result {
            "__call_async_result"
        } else {
            "__call_result"
        };
        self.compile_module_namespace_call_on_binding(
            binding_name,
            namespace_name,
            namespace_span,
            internal_name,
            &[],
            &rewritten,
        )?;

        // (4) Type the call site at `Result<R, RemoteError>` (NOT the bare `R`
        // of the raising sibling), so the documented
        // `match remote::call(…) { Ok(v) => …, Err(e) => … }` type-checks and
        // the runtime `Result` value it produces matches. `R` is `fn_ref`'s
        // declared return type, except a remote callee returning `Future<T>` is
        // receiver-materialized before serialization and therefore surfaces as
        // payload `T`. An unannotated/unit return becomes `Void`.
        let r_ann = return_type
            .clone()
            .map(Self::remote_result_payload_annotation)
            .unwrap_or(shape_ast::ast::TypeAnnotation::Void);
        let result_ann = shape_ast::ast::TypeAnnotation::Generic {
            name: shape_ast::ast::TypePath::simple("Result"),
            args: vec![
                r_ann,
                shape_ast::ast::TypeAnnotation::Reference(shape_ast::ast::TypePath::simple(
                    "RemoteError",
                )),
            ],
        };
        let final_ann = if async_result {
            shape_ast::ast::TypeAnnotation::Generic {
                name: shape_ast::ast::TypePath::simple("Future"),
                args: vec![result_ann],
            }
        } else {
            result_ann
        };
        self.last_expr_type_info = self.type_info_from_annotation(&final_ann);
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);
        Ok(())
    }

    /// ADR-009 E4 S5 (CP4, E4-D3) — elaborate the `@remote` decision weave's
    /// synthesized short-circuit `__call_raising(addr, <impl-shadow>, <arg-pack>)`
    /// at the shadow's BARE `R`.
    ///
    /// This is the raising-primitive analog of [`Self::compile_remote_call_elaboration`]
    /// (which types the recoverable `remote::call` at `Result<R, RemoteError>`):
    /// `__call_raising` delivers the callee's value at its DECLARED return type
    /// and RAISES an ordinary runtime error on transport / protocol / remote
    /// failure (Q26), so the short-circuit types at the bare `R`. The callee
    /// (`shadow_name`) is the hygienic impl shadow the weave registered before it
    /// lowered the decision helper; its declared return type IS `R` (the shadow
    /// carries the target's own body), so the before-exit gate's `== R` proof
    /// (`pseudo_tuple::is_call_raising_payload`) and this emitted type agree.
    ///
    /// The three arguments are already in final form (the markers were
    /// substituted at weave time): `addr` (a baked config-capture string),
    /// `Identifier(shadow_name)` — which lowers to the shadow's UInt64 function
    /// id, the callee `build_remote_call_request_for_fn_ref` marshals against —
    /// and the `[__c3_p0 .. __c3_pN-1]` positional pack (one `Array<T>`, the
    /// OUTER-TypedArray `serialize_arg_pack` wire arm). No re-lowering to a
    /// `_0.._n` TypedObject pack (that is the `remote::call` carrier).
    fn compile_remote_raising_short_circuit(
        &mut self,
        shadow_name: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<()> {
        // E4-D3: the callee IDENTITY is the impl shadow's fn-ref — NEVER the
        // wrapper (infinite recursion). Read the shadow's declared return `R`.
        let return_annotation = self
            .function_defs
            .get(shadow_name)
            .ok_or_else(|| ShapeError::SemanticError {
                message: format!(
                    "internal error: the `@remote` short-circuit callee `{shadow_name}` is a \
                     registered impl shadow with no recorded AST; the weave registers the shadow \
                     before lowering the decision helper (E4-D3 — no fallback)"
                ),
                location: Some(self.span_to_source_location(span)),
            })?
            .return_type
            .clone();

        // Emit the RAISING sibling `remote::__call_raising` over the final args.
        // `Identifier(shadow_name)` compiles to the shadow's UInt64 fn-ref; the
        // pack is the `Array<T>` positional carrier; `addr` is the baked string.
        self.compile_module_builtin_function_call(
            &ModuleBuiltinFunction {
                export_name: "__call_raising".to_string(),
                source_module_path: "std::core::remote".to_string(),
            },
            args,
            span,
        )?;

        // Type the call site at the shadow's BARE `R` (NOT the builtin's declared
        // `_`, and NOT `Result<R, RemoteError>` — that is the recoverable
        // `remote::call` sibling). An unannotated / unit shadow return is `Void`.
        self.last_expr_type_info = return_annotation
            .as_ref()
            .and_then(|ann| self.type_info_from_annotation(ann));
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);
        self.clear_last_expr_reference_result();
        Ok(())
    }

    fn remote_result_payload_annotation(
        ann: shape_ast::ast::TypeAnnotation,
    ) -> shape_ast::ast::TypeAnnotation {
        use shape_ast::ast::TypeAnnotation;
        match ann {
            TypeAnnotation::Generic { name, mut args }
                if name.name() == "Future" && args.len() == 1 =>
            {
                args.remove(0)
            }
            TypeAnnotation::Basic(name) => {
                let trimmed = name.trim();
                if let Some(inner) = trimmed
                    .strip_prefix("Future<")
                    .and_then(|rest| rest.strip_suffix('>'))
                {
                    TypeAnnotation::Basic(inner.trim().to_string())
                } else {
                    TypeAnnotation::Basic(name)
                }
            }
            other => other,
        }
    }

    /// Resolve a `let`-bound closure value's retained [`ClosureBodyPeek`] by
    /// binding name (distributed §4.1.1 R2). Locals take priority over module
    /// bindings, mirroring the value-call return-inference lookup chain at the
    /// top of `compile_module_function_call`. Returns `None` when the name is
    /// not a closure binding whose literal was retained (e.g. a closure passed
    /// in as a function parameter — its body is not statically reachable, so
    /// `remote::call` cannot type-check it and reports the clean §4.1.1 error).
    fn remote_closure_peek(&self, name: &str) -> Option<crate::compiler::ClosureBodyPeek> {
        if let Some(local_idx) = self.resolve_local(name) {
            self.local_callable_closure_bodies.get(&local_idx).cloned()
        } else if let Some(scoped) = self.resolve_scoped_module_binding_name(name) {
            self.module_bindings.get(&scoped).and_then(|idx| {
                self.module_binding_callable_closure_bodies
                    .get(idx)
                    .cloned()
            })
        } else {
            self.module_bindings.get(name).and_then(|idx| {
                self.module_binding_callable_closure_bodies
                    .get(idx)
                    .cloned()
            })
        }
    }

    /// Extract the field name from a simple closure like `row => row.field`.
    /// Returns Some("field") if the closure is a single property access on the parameter.
    fn extract_closure_field_name(expr: &Expr) -> Option<String> {
        if let Expr::FunctionExpr { params, body, .. } = expr {
            if params.len() != 1 {
                return None;
            }
            let param_name = params[0].simple_name()?;

            // Check body: either [Return(Some(PropertyAccess))] or [Expression(PropertyAccess)]
            if body.len() != 1 {
                return None;
            }
            let inner = match &body[0] {
                shape_ast::ast::Statement::Return(Some(e), _) => e,
                shape_ast::ast::Statement::Expression(e, _) => e,
                _ => return None,
            };

            if let Expr::PropertyAccess {
                object, property, ..
            } = inner
            {
                if let Expr::Identifier(name, _) = object.as_ref() {
                    if name == param_name {
                        return Some(property.clone());
                    }
                }
            }
        }
        None
    }

    /// Compile print call with string interpolation expansion
    ///
    /// For strings with `{expr}`, expands at compile time:
    /// - Literal parts: pushed as string constants
    /// - Expression parts: parsed, compiled, converted to string
    /// - Parts are concatenated with Add
    fn compile_print_with_interpolation(&mut self, args: &[Expr]) -> Result<()> {
        let mut processed_args = 0;

        for arg in args {
            // Check if this is a string literal with interpolation
            if let Expr::Literal(Literal::String(s), _span) = arg {
                if has_interpolation(s) {
                    // Expand the interpolation
                    if let Err(err) =
                        self.compile_interpolated_string_expression(s, InterpolationMode::Braces)
                    {
                        if self.should_recover_compile_diagnostics() {
                            self.errors.push(err);
                            self.emit(Instruction::simple(OpCode::PushNull));
                        } else {
                            return Err(err);
                        }
                    }
                    processed_args += 1;
                    continue;
                }
            }

            // Normal argument - compile as-is
            self.compile_expr_as_value_or_placeholder(arg)?;
            processed_args += 1;
        }

        // Push arg count and call print
        let arg_count = self
            .program
            .add_constant(Constant::Int(processed_args as i64));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(arg_count)),
        ));
        self.emit(Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::Print)),
        ));

        self.last_expr_schema = None;
        self.last_expr_type_info = None;

        Ok(())
    }

    /// Collect all available function names for suggestions
    fn collect_available_function_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        // User-defined functions
        for func in &self.program.functions {
            names.push(func.name.clone());
        }
        // Builtin function names (common ones only, skip intrinsics)
        let builtins = [
            "abs",
            "min",
            "max",
            "sqrt",
            "ln",
            "pow",
            "exp",
            "log",
            "floor",
            "ceil",
            "round",
            "sin",
            "cos",
            "tan",
            "stddev",
            "slice",
            "push",
            "pop",
            "first",
            "last",
            "zip",
            "map",
            "filter",
            "reduce",
            "forEach",
            "find",
            "findIndex",
            "some",
            "every",
            "print",
            "format",
            "range",
            "sum",
            "mean",
            "std",
            "variance",
        ];
        for name in builtins {
            names.push(name.to_string());
        }
        names
    }

    /// Check if a function name is a comptime-only builtin.
    /// These are only callable inside `comptime { }` blocks and are rejected
    /// during normal compilation with a helpful error message.
    fn is_comptime_only_builtin(name: &str) -> bool {
        shape_runtime::builtin_metadata::is_comptime_builtin_function(name)
    }

    /// BUG3 — Attempt to monomorphize a generic free function for the given
    /// call-site argument types. Returns `Some(specialized_func_idx)` on
    /// success, or `None` if monomorphization is not applicable or fails
    /// (non-generic callee, unresolved type args, cycle, compile error).
    ///
    /// Mirrors `try_monomorphize_method_call` but without a receiver — the
    /// callee's params are unified directly against the call-site arg types.
    ///
    /// Returns:
    ///   - `Ok(Some(idx))` — specialized function compiled, dispatch directly
    ///   - `Ok(None)`     — soft fallback: resolution incomplete or cycle.
    ///                      Caller falls back to the generic template.
    ///   - `Err(e)`       — hard error: trait-bound violation (Phase 3a) or
    ///                      a specialized-body compile error (ADR-009 A3 S2,
    ///                      `SpecializationFailure::Hard`). Surfaced so the
    ///                      user sees the precise diagnostic — e.g. a comptime
    ///                      semantic-freeze rejection — instead of a masked
    ///                      "cannot infer type argument(s)" or a stack
    ///                      overflow from a silently-empty generic body.
    pub(crate) fn try_monomorphize_free_function_call(
        &mut self,
        func_name: &str,
        explicit_const_args: &[Expr],
        args: &[Expr],
        call_site_span: Span,
    ) -> Result<Option<usize>> {
        let to_shape_error =
            |err: call_site_consts::CallSiteConstArgError| ShapeError::SemanticError {
                message: err.message,
                location: Some(self.span_to_source_location(err.span)),
            };

        // 1. Generic functions participate when they have type params
        // inferred from value arguments, explicit const-generic call-site
        // args, or both.
        let (type_params, const_param_count): (Vec<String>, usize) = {
            let Some(def) = self.function_defs.get(func_name) else {
                return Ok(None);
            };
            let Some(tps) = def.type_params.as_ref() else {
                if !explicit_const_args.is_empty() {
                    return Err(to_shape_error(call_site_consts::no_const_params_error(
                        func_name,
                        call_site_span,
                    )));
                }
                return Ok(None);
            };
            if tps.is_empty() {
                if !explicit_const_args.is_empty() {
                    return Err(to_shape_error(call_site_consts::no_const_params_error(
                        func_name,
                        call_site_span,
                    )));
                }
                return Ok(None);
            }
            (
                tps.iter()
                    .filter(|tp| !tp.is_const())
                    .map(|tp| tp.name().to_string())
                    .collect(),
                tps.iter().filter(|tp| tp.is_const()).count(),
            )
        };
        if type_params.is_empty() && explicit_const_args.is_empty() {
            return Ok(None);
        }
        let const_args = call_site_consts::resolve_explicit_const_args(
            func_name,
            const_param_count,
            explicit_const_args,
            call_site_span,
        )
        .map_err(to_shape_error)?;

        // 2. Per-arg concrete types (None for anything the resolver can't
        //    identify — calls, member accesses, etc.).
        let arg_types = extract_arg_concrete_types(self, args);

        // 3. Unify call-site arg types against the declared param annotations
        //    to bind each type param to a concrete type.
        let resolution = if type_params.is_empty() {
            crate::compiler::monomorphization::type_resolution::TypeArgResolution::with_consts(
                func_name,
                Vec::new(),
                const_args.clone(),
            )
        } else if let Some(resolution) =
            crate::compiler::monomorphization::type_resolution::resolve_call_site_type_args_from_exprs(
                self,
                func_name,
                args,
                &arg_types,
                &type_params,
            ) {
            resolution
        } else if args.is_empty() {
            let Some(expected_return) = self.pending_expected_call_return_type.as_ref() else {
                return Ok(None);
            };
            let Some(resolution) = resolve_call_site_type_args_from_expected_return(
                self,
                func_name,
                expected_return,
                &type_params,
            ) else {
                return Ok(None);
            };
            resolution
        } else {
            return Ok(None);
        };

        // 4. All type args must be concrete. When resolution yields nothing,
        //    fall back to the unspecialized (empty) template and let the
        //    caller diagnose — it's never correct to emit a specialized
        //    call with missing bindings.
        if !type_params.is_empty() && resolution.type_args.is_empty() {
            return Ok(None);
        }
        if resolution.type_args.len() != type_params.len() {
            return Ok(None);
        }

        // 4.5. Phase 3a — pre-check trait bounds against the resolved type
        //      args. This is intentionally separate from the cache call
        //      below so a bound violation surfaces cleanly even when
        //      `ensure_monomorphic_function` would otherwise tunnel a
        //      different SemanticError through (recursion guards, cycle
        //      detection, etc.). Construct the same `subs` map the cache
        //      builds and run the shared validator.
        if let Some(original_def) = self.function_defs.get(func_name).cloned() {
            let subs: HashMap<String, ConcreteType> = type_params
                .iter()
                .cloned()
                .zip(resolution.type_args.iter().cloned())
                .collect();
            self.check_trait_bounds_at_specialization(func_name, &original_def, &subs)?;
        }

        // 5. Produce / reuse the specialization. On a SOFT failure (cycle,
        //    resolution bookkeeping) fall back to the unspecialized template;
        //    a HARD failure (the specialized body itself failed to compile)
        //    propagates — it carries the user's real diagnostic
        //    (ADR-009 A3 S2, surface-and-stop).
        let caller_function = self.current_function;
        let saved_expected_call_return_type = self.pending_expected_call_return_type.take();
        let semantic_request = self.semantic_specialization_request(func_name, call_site_span);
        let specialization_result = if explicit_const_args.is_empty() {
            self.ensure_monomorphic_function_for_callsite(
                func_name,
                &resolution.type_args,
                semantic_request,
            )
        } else {
            self.ensure_monomorphic_function_with_consts_for_callsite(
                func_name,
                &resolution.type_args,
                &const_args,
                semantic_request,
            )
        };
        self.pending_expected_call_return_type = saved_expected_call_return_type;
        match specialization_result {
            Ok(specialized_idx) => {
                self.program
                    .monomorphized_method_call_sites
                    .insert((call_site_span, caller_function), specialized_idx as usize);
                // A recursive call inside a generic body that re-resolves to
                // the specialization currently being compiled MUST still
                // redirect to that specialization's index — `Call`-ing the
                // generic template index instead would dispatch into a
                // zero-instruction body (generic bodies are skipped in
                // `compile_function`). `ensure_monomorphic_function` caches
                // the specialization index *before* compiling the body, so a
                // self-recursive resolution is a plain cache hit and never
                // re-enters compilation.
                Ok(Some(specialized_idx as usize))
            }
            // ADR-009 A3 (S2): a specialized-body compile error is the
            // user's REAL diagnostic — propagate it instead of falling back
            // to the generic template (which would re-report the unrelated
            // "cannot infer type argument(s)" at the call site).
            Err(crate::compiler::monomorphization::cache::SpecializationFailure::Hard(e)) => Err(e),
            Err(crate::compiler::monomorphization::cache::SpecializationFailure::Soft(_)) => {
                Ok(None)
            }
        }
    }

    fn try_specialize_implicit_generic_free_function_call(
        &mut self,
        func_name: &str,
        args: &[Expr],
        call_site_span: Span,
    ) -> Result<Option<usize>> {
        let Some(original_def) = self.function_defs.get(func_name).cloned() else {
            return Ok(None);
        };
        if original_def
            .type_params
            .as_ref()
            .is_some_and(|tps| !tps.is_empty())
            || !self.is_uninstantiated_implicit_generic(&original_def)
            || original_def.params.len() != args.len()
        {
            return Ok(None);
        }

        // ADR-009 C3 #14 (slice 4, S4a — #66 item 1 collateral completion):
        // only the param positions the substitution loop below can SUBSTITUTE
        // (unannotated, by-value, simple-identifier) need a call-site-resolved
        // concrete type. Positions with a DECLARED annotation (and
        // reference/const/destructuring params) keep their declaration
        // untouched, so requiring THEIR args to resolve here let one
        // unresolvable arg at an annotated position veto the whole
        // specialization — the call then dispatched onto the dead template
        // blob with its unannotated params unresolved (measured: the comptime
        // handler wrapper's runtime-preset target/ctx binding identifiers,
        // which have no compile-time type, blocked specializing the handler
        // for its config param and broke every generic call on that param).
        let mut arg_cts: Vec<Option<ConcreteType>> = Vec::with_capacity(args.len());
        for (param, arg) in original_def.params.iter().zip(args.iter()) {
            let substitutable = param.type_annotation.is_none()
                && !param.is_reference
                && !param.is_mut_reference
                && !param.is_const
                && param.simple_name().is_some();
            if !substitutable {
                arg_cts.push(None);
                continue;
            }
            let Some(ct) = self.concrete_type_for_implicit_specialization_arg(arg)? else {
                return Ok(None);
            };
            arg_cts.push(Some(ct));
        }

        let mut specialized_def = original_def.clone();
        let mut changed = false;
        for (param, ct) in specialized_def.params.iter_mut().zip(arg_cts.iter()) {
            if param.type_annotation.is_some()
                || param.is_reference
                || param.is_mut_reference
                || param.is_const
                || param.simple_name().is_none()
            {
                continue;
            }
            // Substitutable positions always resolved above.
            let Some(ct) = ct.as_ref() else {
                continue;
            };
            let Some(ann) = type_annotation_from_concrete_type(ct) else {
                return Ok(None);
            };
            param.type_annotation = Some(ann);
            changed = true;
        }

        // Return inference needs a full per-position type vector; fill the
        // non-substituted positions from their DECLARED annotations when the
        // annotation converts (an inconvertible annotation just skips return
        // inference — the specialized body infers its own return type).
        let full_cts: Option<Vec<ConcreteType>> = original_def
            .params
            .iter()
            .zip(arg_cts.iter())
            .map(|(param, ct)| {
                ct.clone().or_else(|| {
                    param
                        .type_annotation
                        .as_ref()
                        .and_then(crate::compiler::v2_map_emission::concrete_type_from_annotation)
                })
            })
            .collect();
        if let Some(full_cts) = full_cts
            && let Some(return_ct) =
                self.implicit_specialization_return_concrete_type(&original_def, &full_cts, 0)
            && specialized_def.return_type.is_none()
            && let Some(ann) = type_annotation_from_concrete_type(&return_ct)
        {
            specialized_def.return_type = Some(ann);
            changed = true;
        }

        if !changed {
            return Ok(None);
        }

        let mut name_parts = vec![format!(
            "__w27_implicit_{}",
            func_name
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                .collect::<String>()
        )];
        // Substituted positions key on the resolved type (all-unannotated
        // fns — the whole pre-S4a domain — keep byte-identical keys);
        // non-substituted positions are fixed by the declaration, so one
        // stable marker keeps the key injective over what actually varies.
        name_parts.extend(arg_cts.iter().map(|ct| match ct {
            Some(ct) => concrete_type_cache_key(ct),
            None => "decl".to_string(),
        }));
        specialized_def.name = name_parts.join("_");
        specialized_def.type_params = Some(Vec::new());

        if let Some(idx) = self.find_function(&specialized_def.name) {
            // ADR-009 A3 (review round 1) — never reuse a
            // registered-but-never-compiled specialization (empty body);
            // see the twin guard in `try_specialize_concrete_user_method_call`.
            if self
                .failed_call_site_specializations
                .contains(&specialized_def.name)
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "implicit-generic specialization '{}' failed to compile at an earlier call site; the registered specialization has no body and cannot be dispatched",
                        specialized_def.name
                    ),
                    location: Some(self.span_to_source_location(call_site_span)),
                });
            }
            self.program
                .monomorphized_method_call_sites
                .insert((call_site_span, self.current_function), idx);
            return Ok(Some(idx));
        }

        self.register_function(&specialized_def)?;
        let Some(specialized_idx) = self.find_function(&specialized_def.name) else {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "failed to register implicit-generic specialization '{}'",
                    specialized_def.name
                ),
                location: Some(self.span_to_source_location(call_site_span)),
            });
        };

        let saved_closure_function_ids = std::mem::take(&mut self.closure_function_ids);
        let saved_local_concrete_facts =
            std::mem::take(&mut self.current_function_local_concrete_facts);
        let saved_local_binding_spans = std::mem::take(&mut self.local_binding_spans);
        // Implicit generics have no declared TypeVar capabilities. An empty
        // declaration-only frame masks any enclosing exact specialization.
        let specialization_overlay =
            SpecializationTypeOverlay::declaration_only(func_name, Vec::new());
        let specialization_overlay_guard = self
            .specialization_type_overlays
            .enter(specialization_overlay);
        let compile_result = self.compile_function(&specialized_def);
        drop(specialization_overlay_guard);
        self.closure_function_ids = saved_closure_function_ids;
        self.current_function_local_concrete_facts = saved_local_concrete_facts;
        self.local_binding_spans = saved_local_binding_spans;
        if let Err(err) = compile_result {
            // Registration is positional and cannot be rolled back — remember
            // the failure so the reuse short-circuit above refuses the empty
            // registered body.
            self.failed_call_site_specializations
                .insert(specialized_def.name.clone());
            return Err(err);
        }

        self.program
            .monomorphized_method_call_sites
            .insert((call_site_span, self.current_function), specialized_idx);

        Ok(Some(specialized_idx))
    }

    pub(super) fn implicit_generic_body_requires_concrete_emission(
        &self,
        func_def: &shape_ast::ast::FunctionDef,
    ) -> bool {
        let mut visiting = BTreeSet::new();
        self.implicit_generic_body_requires_concrete_emission_by_name(&func_def.name, &mut visiting)
    }

    fn implicit_generic_body_requires_concrete_emission_by_name(
        &self,
        func_name: &str,
        visiting: &mut BTreeSet<String>,
    ) -> bool {
        if !visiting.insert(func_name.to_string()) {
            return false;
        }
        let Some(func_def) = self.function_defs.get(func_name) else {
            return false;
        };
        let param_names: BTreeSet<String> = func_def
            .params
            .iter()
            .filter_map(|param| param.pattern.as_identifier().map(str::to_string))
            .collect();
        func_def.body.iter().any(|stmt| {
            self.implicit_generic_stmt_requires_concrete_emission(stmt, &param_names, visiting)
        })
    }

    fn implicit_generic_stmt_requires_concrete_emission(
        &self,
        stmt: &Statement,
        param_names: &BTreeSet<String>,
        visiting: &mut BTreeSet<String>,
    ) -> bool {
        match stmt {
            Statement::Return(Some(expr), _)
            | Statement::Expression(expr, _)
            | Statement::SetParamValue {
                expression: expr, ..
            }
            | Statement::SetParamTypeExpr {
                expression: expr, ..
            }
            | Statement::SetReturnExpr {
                expression: expr, ..
            }
            | Statement::ReplaceBodyExpr {
                expression: expr, ..
            }
            | Statement::ReplaceModuleExpr {
                expression: expr, ..
            }
            | Statement::ExtendItemsExpr {
                expression: expr, ..
            } => self.implicit_generic_expr_requires_concrete_emission(expr, param_names, visiting),
            Statement::VariableDecl(decl, _) => decl.value.as_ref().is_some_and(|expr| {
                self.implicit_generic_expr_requires_concrete_emission(expr, param_names, visiting)
            }),
            Statement::Assignment(assign, _) => self
                .implicit_generic_expr_requires_concrete_emission(
                    &assign.value,
                    param_names,
                    visiting,
                ),
            Statement::If(if_stmt, _) => {
                self.implicit_generic_expr_requires_concrete_emission(
                    &if_stmt.condition,
                    param_names,
                    visiting,
                ) || if_stmt.then_body.iter().any(|stmt| {
                    self.implicit_generic_stmt_requires_concrete_emission(
                        stmt,
                        param_names,
                        visiting,
                    )
                }) || if_stmt.else_body.as_ref().is_some_and(|else_body| {
                    else_body.iter().any(|stmt| {
                        self.implicit_generic_stmt_requires_concrete_emission(
                            stmt,
                            param_names,
                            visiting,
                        )
                    })
                })
            }
            Statement::While(while_loop, _) => {
                self.implicit_generic_expr_requires_concrete_emission(
                    &while_loop.condition,
                    param_names,
                    visiting,
                ) || while_loop.body.iter().any(|stmt| {
                    self.implicit_generic_stmt_requires_concrete_emission(
                        stmt,
                        param_names,
                        visiting,
                    )
                })
            }
            Statement::For(for_loop, _) => {
                let init_requires = match &for_loop.init {
                    shape_ast::ast::ForInit::ForIn { iter, .. } => self
                        .implicit_generic_expr_requires_concrete_emission(
                            iter,
                            param_names,
                            visiting,
                        ),
                    shape_ast::ast::ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.implicit_generic_stmt_requires_concrete_emission(
                            init,
                            param_names,
                            visiting,
                        ) || self.implicit_generic_expr_requires_concrete_emission(
                            condition,
                            param_names,
                            visiting,
                        ) || self.implicit_generic_expr_requires_concrete_emission(
                            update,
                            param_names,
                            visiting,
                        )
                    }
                };
                init_requires
                    || for_loop.body.iter().any(|stmt| {
                        self.implicit_generic_stmt_requires_concrete_emission(
                            stmt,
                            param_names,
                            visiting,
                        )
                    })
            }
            Statement::ReplaceBody { body, .. } => body.iter().any(|stmt| {
                self.implicit_generic_stmt_requires_concrete_emission(stmt, param_names, visiting)
            }),
            _ => false,
        }
    }

    fn implicit_generic_expr_requires_concrete_emission(
        &self,
        expr: &Expr,
        param_names: &BTreeSet<String>,
        visiting: &mut BTreeSet<String>,
    ) -> bool {
        match expr {
            Expr::BinaryOp {
                op, left, right, ..
            } => {
                // wave7 finance-field-arith-gap: only an operator that lowers to
                // a typed numeric/bitwise/ordered opcode (and so needs a proven
                // operand kind) makes an unannotated-param body un-emittable. An
                // equality (`row.x != None`) or logical (`and`) binop compiles
                // fine in the deferred template, so it must NOT force concrete
                // emission (else an object-predicate like `is_ohlcv` over an
                // anonymous object is mis-flagged). A flagged op nested inside an
                // excluded one (`(row.a - row.b) != 0`) is still caught by the
                // recursion below.
                if super::numeric_ops::op_requires_proven_operand_kind(op)
                    && (Self::expr_mentions_any_name(left, param_names)
                        || Self::expr_mentions_any_name(right, param_names))
                {
                    return true;
                }
                self.implicit_generic_expr_requires_concrete_emission(left, param_names, visiting)
                    || self.implicit_generic_expr_requires_concrete_emission(
                        right,
                        param_names,
                        visiting,
                    )
            }
            Expr::FunctionCall { name, args, .. } => {
                let args_require = args.iter().any(|arg| {
                    self.implicit_generic_expr_requires_concrete_emission(
                        arg,
                        param_names,
                        visiting,
                    )
                });
                if args_require {
                    return true;
                }
                self.function_defs.get(name).is_some_and(|def| {
                    self.is_uninstantiated_implicit_generic(def)
                        && self.implicit_generic_body_requires_concrete_emission_by_name(
                            name, visiting,
                        )
                })
            }
            Expr::QualifiedFunctionCall { args, .. } => args.iter().any(|arg| {
                self.implicit_generic_expr_requires_concrete_emission(arg, param_names, visiting)
            }),
            Expr::MethodCall { receiver, args, .. } => {
                self.implicit_generic_expr_requires_concrete_emission(
                    receiver,
                    param_names,
                    visiting,
                ) || args.iter().any(|arg| {
                    self.implicit_generic_expr_requires_concrete_emission(
                        arg,
                        param_names,
                        visiting,
                    )
                })
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.implicit_generic_expr_requires_concrete_emission(
                    condition,
                    param_names,
                    visiting,
                ) || self.implicit_generic_expr_requires_concrete_emission(
                    then_expr,
                    param_names,
                    visiting,
                ) || else_expr.as_ref().is_some_and(|expr| {
                    self.implicit_generic_expr_requires_concrete_emission(
                        expr,
                        param_names,
                        visiting,
                    )
                })
            }
            Expr::If(if_expr, _) => {
                self.implicit_generic_expr_requires_concrete_emission(
                    &if_expr.condition,
                    param_names,
                    visiting,
                ) || self.implicit_generic_expr_requires_concrete_emission(
                    &if_expr.then_branch,
                    param_names,
                    visiting,
                ) || if_expr.else_branch.as_ref().is_some_and(|expr| {
                    self.implicit_generic_expr_requires_concrete_emission(
                        expr,
                        param_names,
                        visiting,
                    )
                })
            }
            Expr::Block(block, _) => block.items.iter().any(|item| match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => {
                    decl.value.as_ref().is_some_and(|expr| {
                        self.implicit_generic_expr_requires_concrete_emission(
                            expr,
                            param_names,
                            visiting,
                        )
                    })
                }
                shape_ast::ast::BlockItem::Assignment(assign) => self
                    .implicit_generic_expr_requires_concrete_emission(
                        &assign.value,
                        param_names,
                        visiting,
                    ),
                shape_ast::ast::BlockItem::Statement(stmt) => self
                    .implicit_generic_stmt_requires_concrete_emission(stmt, param_names, visiting),
                shape_ast::ast::BlockItem::Expression(expr) => self
                    .implicit_generic_expr_requires_concrete_emission(expr, param_names, visiting),
            }),
            Expr::Array(elements, _) => elements.iter().any(|expr| {
                self.implicit_generic_expr_requires_concrete_emission(expr, param_names, visiting)
            }),
            Expr::Object(entries, _) => entries.iter().any(|entry| match entry {
                shape_ast::ast::ObjectEntry::Field { value, .. }
                | shape_ast::ast::ObjectEntry::Spread(value) => self
                    .implicit_generic_expr_requires_concrete_emission(value, param_names, visiting),
            }),
            Expr::UnaryOp { operand, .. }
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::AsyncScope(operand, _)
            | Expr::Spread(operand, _) => self.implicit_generic_expr_requires_concrete_emission(
                operand,
                param_names,
                visiting,
            ),
            Expr::Return(Some(expr), _) | Expr::Break(Some(expr), _) => {
                self.implicit_generic_expr_requires_concrete_emission(expr, param_names, visiting)
            }
            Expr::Let(let_expr, _) => {
                let_expr.value.as_ref().is_some_and(|expr| {
                    self.implicit_generic_expr_requires_concrete_emission(
                        expr,
                        param_names,
                        visiting,
                    )
                }) || self.implicit_generic_expr_requires_concrete_emission(
                    &let_expr.body,
                    param_names,
                    visiting,
                )
            }
            Expr::Assign(assign_expr, _) => self.implicit_generic_expr_requires_concrete_emission(
                &assign_expr.value,
                param_names,
                visiting,
            ),
            Expr::FunctionExpr { .. } => false,
            _ => false,
        }
    }

    fn expr_mentions_any_name(expr: &Expr, names: &BTreeSet<String>) -> bool {
        match expr {
            Expr::Identifier(name, _) => names.contains(name),
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                Self::expr_mentions_any_name(left, names)
                    || Self::expr_mentions_any_name(right, names)
            }
            Expr::UnaryOp { operand, .. }
            | Expr::TryOperator(operand, _)
            | Expr::Await(operand, _)
            | Expr::AsyncScope(operand, _)
            | Expr::Spread(operand, _) => Self::expr_mentions_any_name(operand, names),
            // wave7 finance-field-arith-gap: a field / index read on a param
            // (`row.high`, `bars[i]`) MENTIONS that param. Without these arms
            // object-field arithmetic (`row.high - row.low`) evaded
            // `implicit_generic_body_requires_concrete_emission`, so the
            // guards keyed on it did not see that the body needs a proven
            // concrete param type — the root of the untyped-param field-arith
            // strict-typing bypass.
            Expr::PropertyAccess { object, .. } => Self::expr_mentions_any_name(object, names),
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                Self::expr_mentions_any_name(object, names)
                    || Self::expr_mentions_any_name(index, names)
                    || end_index
                        .as_ref()
                        .is_some_and(|e| Self::expr_mentions_any_name(e, names))
            }
            Expr::FunctionCall { args, .. } | Expr::QualifiedFunctionCall { args, .. } => args
                .iter()
                .any(|arg| Self::expr_mentions_any_name(arg, names)),
            Expr::MethodCall { receiver, args, .. } => {
                Self::expr_mentions_any_name(receiver, names)
                    || args
                        .iter()
                        .any(|arg| Self::expr_mentions_any_name(arg, names))
            }
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                Self::expr_mentions_any_name(condition, names)
                    || Self::expr_mentions_any_name(then_expr, names)
                    || else_expr
                        .as_ref()
                        .is_some_and(|expr| Self::expr_mentions_any_name(expr, names))
            }
            Expr::If(if_expr, _) => {
                Self::expr_mentions_any_name(&if_expr.condition, names)
                    || Self::expr_mentions_any_name(&if_expr.then_branch, names)
                    || if_expr
                        .else_branch
                        .as_ref()
                        .is_some_and(|expr| Self::expr_mentions_any_name(expr, names))
            }
            Expr::Block(block, _) => block.items.iter().any(|item| match item {
                shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                    .value
                    .as_ref()
                    .is_some_and(|expr| Self::expr_mentions_any_name(expr, names)),
                shape_ast::ast::BlockItem::Assignment(assign) => {
                    Self::expr_mentions_any_name(&assign.value, names)
                }
                shape_ast::ast::BlockItem::Statement(stmt) => {
                    Self::stmt_mentions_any_name(stmt, names)
                }
                shape_ast::ast::BlockItem::Expression(expr) => {
                    Self::expr_mentions_any_name(expr, names)
                }
            }),
            Expr::Return(Some(expr), _) | Expr::Break(Some(expr), _) => {
                Self::expr_mentions_any_name(expr, names)
            }
            Expr::Array(elements, _) => elements
                .iter()
                .any(|expr| Self::expr_mentions_any_name(expr, names)),
            Expr::Object(entries, _) => entries.iter().any(|entry| match entry {
                shape_ast::ast::ObjectEntry::Field { value, .. }
                | shape_ast::ast::ObjectEntry::Spread(value) => {
                    Self::expr_mentions_any_name(value, names)
                }
            }),
            Expr::Let(let_expr, _) => {
                let_expr
                    .value
                    .as_ref()
                    .is_some_and(|expr| Self::expr_mentions_any_name(expr, names))
                    || Self::expr_mentions_any_name(&let_expr.body, names)
            }
            Expr::Assign(assign_expr, _) => Self::expr_mentions_any_name(&assign_expr.value, names),
            _ => false,
        }
    }

    fn stmt_mentions_any_name(stmt: &Statement, names: &BTreeSet<String>) -> bool {
        match stmt {
            Statement::Return(Some(expr), _) | Statement::Expression(expr, _) => {
                Self::expr_mentions_any_name(expr, names)
            }
            Statement::VariableDecl(decl, _) => decl
                .value
                .as_ref()
                .is_some_and(|expr| Self::expr_mentions_any_name(expr, names)),
            Statement::Assignment(assign, _) => Self::expr_mentions_any_name(&assign.value, names),
            Statement::If(if_stmt, _) => {
                Self::expr_mentions_any_name(&if_stmt.condition, names)
                    || if_stmt
                        .then_body
                        .iter()
                        .any(|stmt| Self::stmt_mentions_any_name(stmt, names))
                    || if_stmt.else_body.as_ref().is_some_and(|else_body| {
                        else_body
                            .iter()
                            .any(|stmt| Self::stmt_mentions_any_name(stmt, names))
                    })
            }
            Statement::While(while_loop, _) => {
                Self::expr_mentions_any_name(&while_loop.condition, names)
                    || while_loop
                        .body
                        .iter()
                        .any(|stmt| Self::stmt_mentions_any_name(stmt, names))
            }
            Statement::For(for_loop, _) => {
                let init_mentions = match &for_loop.init {
                    shape_ast::ast::ForInit::ForIn { iter, .. } => {
                        Self::expr_mentions_any_name(iter, names)
                    }
                    shape_ast::ast::ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        Self::stmt_mentions_any_name(init, names)
                            || Self::expr_mentions_any_name(condition, names)
                            || Self::expr_mentions_any_name(update, names)
                    }
                };
                init_mentions
                    || for_loop
                        .body
                        .iter()
                        .any(|stmt| Self::stmt_mentions_any_name(stmt, names))
            }
            _ => false,
        }
    }

    fn concrete_type_for_implicit_specialization_arg(
        &mut self,
        expr: &Expr,
    ) -> Result<Option<ConcreteType>> {
        if let Some(ct) = concrete_type_for_expr(self, expr) {
            return Ok(Some(ct));
        }
        if let Expr::BinaryOp {
            left, op, right, ..
        } = expr
        {
            if matches!(op, shape_ast::ast::BinaryOp::Pipe) {
                return self.concrete_type_for_implicit_pipe_call(left, right);
            }
        }
        let Expr::FunctionCall { name, args, .. } = expr else {
            return Ok(None);
        };
        let Some(func_def) = self.function_defs.get(name).cloned() else {
            return Ok(None);
        };
        if !self.is_uninstantiated_implicit_generic(&func_def) {
            return Ok(None);
        }
        let mut arg_cts = Vec::with_capacity(args.len());
        for arg in args {
            let Some(ct) = self.concrete_type_for_implicit_specialization_arg(arg)? else {
                return Ok(None);
            };
            arg_cts.push(ct);
        }
        Ok(self.implicit_specialization_return_concrete_type(&func_def, &arg_cts, 0))
    }

    fn concrete_type_for_implicit_pipe_call(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> Result<Option<ConcreteType>> {
        let (name, pipe_args) = match right {
            Expr::Identifier(name, _) => (name.as_str(), &[][..]),
            Expr::FunctionCall { name, args, .. } => (name.as_str(), args.as_slice()),
            _ => return Ok(None),
        };

        let Some(func_def) = self.function_defs.get(name).cloned() else {
            return Ok(None);
        };
        if !self.is_uninstantiated_implicit_generic(&func_def)
            || func_def.params.len() != pipe_args.len() + 1
        {
            return Ok(None);
        }

        let mut arg_cts = Vec::with_capacity(pipe_args.len() + 1);
        let Some(left_ct) = self.concrete_type_for_implicit_specialization_arg(left)? else {
            return Ok(None);
        };
        arg_cts.push(left_ct);
        for arg in pipe_args {
            let Some(ct) = self.concrete_type_for_implicit_specialization_arg(arg)? else {
                return Ok(None);
            };
            arg_cts.push(ct);
        }

        Ok(self.implicit_specialization_return_concrete_type(&func_def, &arg_cts, 0))
    }

    fn implicit_specialization_return_concrete_type(
        &self,
        func_def: &shape_ast::ast::FunctionDef,
        arg_cts: &[ConcreteType],
        depth: usize,
    ) -> Option<ConcreteType> {
        if depth > 8 || func_def.params.len() != arg_cts.len() {
            return None;
        }
        if let Some(return_type) = func_def.return_type.as_ref() {
            return crate::compiler::v2_map_emission::concrete_type_from_annotation(return_type);
        }

        let mut param_cts: HashMap<&str, ConcreteType> = HashMap::new();
        for (param, ct) in func_def.params.iter().zip(arg_cts.iter()) {
            let name = param.pattern.as_identifier()?;
            if let Some(annotation) = param.type_annotation.as_ref() {
                let declared =
                    crate::compiler::v2_map_emission::concrete_type_from_annotation(annotation)?;
                if declared != *ct {
                    return None;
                }
            }
            param_cts.insert(name, ct.clone());
        }

        self.implicit_specialization_stmt_tail_type(&func_def.body, &param_cts, depth)
    }

    fn implicit_specialization_stmt_tail_type(
        &self,
        body: &[Statement],
        param_cts: &HashMap<&str, ConcreteType>,
        depth: usize,
    ) -> Option<ConcreteType> {
        match body.last()? {
            Statement::Return(Some(expr), _) | Statement::Expression(expr, _) => {
                self.implicit_specialization_expr_type(expr, param_cts, depth)
            }
            Statement::If(if_stmt, _) => {
                let then_ct = self.implicit_specialization_stmt_tail_type(
                    &if_stmt.then_body,
                    param_cts,
                    depth,
                )?;
                let else_body = if_stmt.else_body.as_ref()?;
                let else_ct =
                    self.implicit_specialization_stmt_tail_type(else_body, param_cts, depth)?;
                if then_ct == else_ct {
                    Some(then_ct)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn implicit_specialization_expr_type(
        &self,
        expr: &Expr,
        param_cts: &HashMap<&str, ConcreteType>,
        depth: usize,
    ) -> Option<ConcreteType> {
        match expr {
            Expr::Identifier(name, _) => param_cts.get(name.as_str()).cloned(),
            Expr::Literal(_, _) => concrete_type_for_expr(self, expr),
            Expr::If(if_expr, _) => {
                let then_ct =
                    self.implicit_specialization_expr_type(&if_expr.then_branch, param_cts, depth)?;
                let else_ct = if_expr.else_branch.as_ref().and_then(|else_expr| {
                    self.implicit_specialization_expr_type(else_expr, param_cts, depth)
                })?;
                if then_ct == else_ct {
                    Some(then_ct)
                } else {
                    None
                }
            }
            Expr::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                let then_ct =
                    self.implicit_specialization_expr_type(then_expr, param_cts, depth)?;
                let else_ct = else_expr.as_ref().and_then(|else_expr| {
                    self.implicit_specialization_expr_type(else_expr, param_cts, depth)
                })?;
                if then_ct == else_ct {
                    Some(then_ct)
                } else {
                    None
                }
            }
            Expr::Block(block, _) => match block.items.last()? {
                shape_ast::ast::BlockItem::Expression(expr) => {
                    self.implicit_specialization_expr_type(expr, param_cts, depth)
                }
                shape_ast::ast::BlockItem::Statement(stmt) => self
                    .implicit_specialization_stmt_tail_type(
                        std::slice::from_ref(stmt),
                        param_cts,
                        depth,
                    ),
                shape_ast::ast::BlockItem::VariableDecl(decl) => {
                    decl.value.as_ref().and_then(|expr| {
                        self.implicit_specialization_expr_type(expr, param_cts, depth)
                    })
                }
                shape_ast::ast::BlockItem::Assignment(assign) => {
                    self.implicit_specialization_expr_type(&assign.value, param_cts, depth)
                }
            },
            Expr::Return(Some(expr), _) => {
                self.implicit_specialization_expr_type(expr, param_cts, depth)
            }
            Expr::FunctionCall { name, args, .. } => {
                let func_def = self.function_defs.get(name)?;
                let mut arg_cts = Vec::with_capacity(args.len());
                for arg in args {
                    arg_cts.push(
                        self.implicit_specialization_expr_type(arg, param_cts, depth)
                            .or_else(|| concrete_type_for_expr(self, arg))?,
                    );
                }
                self.implicit_specialization_return_concrete_type(func_def, &arg_cts, depth + 1)
            }
            Expr::BinaryOp {
                left, op, right, ..
            } => {
                use shape_ast::ast::BinaryOp;
                match op {
                    BinaryOp::Greater
                    | BinaryOp::Less
                    | BinaryOp::GreaterEq
                    | BinaryOp::LessEq
                    | BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::And
                    | BinaryOp::Or => Some(ConcreteType::Bool),
                    _ => {
                        let left_ct =
                            self.implicit_specialization_expr_type(left, param_cts, depth)?;
                        let right_ct =
                            self.implicit_specialization_expr_type(right, param_cts, depth)?;
                        if left_ct == right_ct {
                            Some(left_ct)
                        } else {
                            adopt_int_literal_for_implicit_numeric_expr(
                                &left_ct, left, &right_ct, right,
                            )
                        }
                    }
                }
            }
            _ => concrete_type_for_expr(self, expr),
        }
    }

    /// Attempt to monomorphize a generic extend method for the receiver's
    /// concrete type. Returns `Some(specialized_func_idx)` on success, or
    /// `None` if monomorphization is not applicable or fails.
    ///
    /// This is the bridge between generic extend methods (e.g. `Vec<T>.indexOf`)
    /// and the monomorphization cache. When the receiver has a concretely known
    /// type (e.g. `Array<int>`), the function's type parameters are resolved
    /// and a specialized version is compiled/cached.
    fn try_specialize_concrete_user_method_call(
        &mut self,
        func_name: &str,
        receiver: &Expr,
        args: &[Expr],
        call_site_span: Span,
    ) -> Result<Option<usize>> {
        if !(func_name.contains('.') || func_name.contains("::")) {
            return Ok(None);
        }

        let Some(original_def) = self.function_defs.get(func_name).cloned() else {
            return Ok(None);
        };
        if original_def.params.len() != args.len() + 1 {
            return Ok(None);
        }

        let Some(receiver_ct) = concrete_type_for_expr(self, receiver) else {
            return Ok(None);
        };
        let generic_substitutions = if original_def
            .type_params
            .as_ref()
            .is_some_and(|tps| !tps.is_empty())
        {
            let Some(substitutions) =
                number_receiver_generic_substitutions(func_name, &original_def, &receiver_ct)
            else {
                return Ok(None);
            };
            substitutions
        } else {
            HashMap::new()
        };
        let mut call_arg_cts = Vec::with_capacity(args.len() + 1);
        call_arg_cts.push(receiver_ct);
        for arg in args {
            let Some(ct) = concrete_type_for_expr(self, arg) else {
                return Ok(None);
            };
            call_arg_cts.push(ct);
        }

        let mut specialized_def = original_def.clone();
        let mut changed = false;
        if !generic_substitutions.is_empty() {
            for param in &mut specialized_def.params {
                if let Some(ann) = param.type_annotation.as_mut() {
                    let substituted =
                        substitute_type_params_in_annotation(ann, &generic_substitutions);
                    if substituted != *ann {
                        *ann = substituted;
                        changed = true;
                    }
                }
            }
            if let Some(return_ann) = specialized_def.return_type.as_mut() {
                let substituted =
                    substitute_type_params_in_annotation(return_ann, &generic_substitutions);
                if substituted != *return_ann {
                    *return_ann = substituted;
                    changed = true;
                }
            }
        }
        for (param, ct) in specialized_def.params.iter_mut().zip(call_arg_cts.iter()) {
            if param.type_annotation.is_none() {
                let Some(ann) = type_annotation_from_concrete_type(ct) else {
                    return Ok(None);
                };
                param.type_annotation = Some(ann);
                changed = true;
            }
        }

        let return_ct = if specialized_def.return_type.is_none() {
            self.concrete_user_method_body_return_type(&specialized_def, &call_arg_cts)
        } else {
            None
        };
        if let Some(ct) = return_ct.as_ref() {
            if let Some(ann) = type_annotation_from_concrete_type(ct) {
                specialized_def.return_type = Some(ann);
                changed = true;
            }
        }

        if !changed {
            return Ok(None);
        }

        let mut name_parts = vec![format!(
            "__w24_method_{}_{}_{}",
            func_name
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                .collect::<String>(),
            call_site_span.start,
            call_site_span.end
        )];
        name_parts.extend(call_arg_cts.iter().map(concrete_type_cache_key));
        let abi_specialization_name = name_parts.join("_");
        let declared_type_params = Self::specialization_type_param_names(&original_def);
        let semantic_request = self.semantic_specialization_request(func_name, call_site_span);
        let semantic = self.prepare_semantic_specialization(
            func_name,
            abi_specialization_name.clone(),
            declared_type_params.len(),
            semantic_request,
        )?;
        specialized_def.name = semantic.specialized_symbol(abi_specialization_name);
        specialized_def.type_params = Some(Vec::new());
        let specialization_overlay = semantic.overlay(func_name, &declared_type_params)?;

        if let Some(idx) = self.find_function(&specialized_def.name) {
            // ADR-009 A3 (review round 1) — a registered specialization whose
            // body compile FAILED must never be reused: its Function entry has
            // zero instructions, so a `Call` would silently dispatch an empty
            // body (or trip the linker's `remap_fid` self-recursion). Re-raise
            // a hard error instead (surface-and-stop).
            if self
                .failed_call_site_specializations
                .contains(&specialized_def.name)
            {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "method call specialization '{}' failed to compile at an earlier call site; the registered specialization has no body and cannot be dispatched",
                        specialized_def.name
                    ),
                    location: Some(self.span_to_source_location(call_site_span)),
                });
            }
            self.program
                .monomorphized_method_call_sites
                .insert((call_site_span, self.current_function), idx);
            return Ok(Some(idx));
        }

        self.register_function(&specialized_def)?;
        let Some(specialized_idx) = self.find_function(&specialized_def.name) else {
            return Err(ShapeError::SemanticError {
                message: format!(
                    "failed to register method call specialization '{}'",
                    specialized_def.name
                ),
                location: Some(self.span_to_source_location(call_site_span)),
            });
        };

        let saved_closure_function_ids = std::mem::take(&mut self.closure_function_ids);
        let saved_local_concrete_facts =
            std::mem::take(&mut self.current_function_local_concrete_facts);
        let saved_local_binding_spans = std::mem::take(&mut self.local_binding_spans);
        // Exact facts install declared-token mappings; legacy facts install
        // declaration-only reflection and cannot become query-visible exact
        // capture evidence.
        let specialization_overlay_guard = self
            .specialization_type_overlays
            .enter(specialization_overlay);
        let compile_result = self.compile_function(&specialized_def);
        drop(specialization_overlay_guard);
        self.closure_function_ids = saved_closure_function_ids;
        self.current_function_local_concrete_facts = saved_local_concrete_facts;
        self.local_binding_spans = saved_local_binding_spans;
        if let Err(err) = compile_result {
            // ADR-009 A3 (review round 1) — the registration above cannot be
            // rolled back (function indices are positional); remember the
            // failure so the reuse short-circuit refuses to dispatch the
            // empty registered body.
            self.failed_call_site_specializations
                .insert(specialized_def.name.clone());
            return Err(err);
        }

        self.program
            .monomorphized_method_call_sites
            .insert((call_site_span, self.current_function), specialized_idx);

        Ok(Some(specialized_idx))
    }

    fn concrete_user_method_body_return_type(
        &self,
        func_def: &shape_ast::ast::FunctionDef,
        call_arg_cts: &[ConcreteType],
    ) -> Option<ConcreteType> {
        let body_expr = match func_def.body.as_slice() {
            [Statement::Expression(expr, _)] => expr,
            [Statement::Return(Some(expr), _)] => expr,
            _ => return None,
        };

        match body_expr {
            Expr::Identifier(name, _) if name == "self" => call_arg_cts.first().cloned(),
            _ => concrete_type_for_expr(self, body_expr),
        }
    }

    fn stamp_last_expr_from_function_return_annotation(&mut self, func_name: &str) -> bool {
        let Some(def) = self.function_defs.get(func_name) else {
            return false;
        };
        let Some(return_type) = def.return_type.clone() else {
            return false;
        };
        let declaring_module_path = def.declaring_module_path.clone();
        let direct = self.type_info_from_annotation(&return_type);
        let namespaced = || {
            let symbol_namespace = if let Some((receiver_type, _)) = func_name.rsplit_once('.') {
                receiver_type.rsplit_once("::").map(|(ns, _)| ns)
            } else {
                func_name
                    .rsplit_once("::")
                    .and_then(|(receiver_type, _)| receiver_type.rsplit_once("::"))
                    .map(|(ns, _)| ns)
            };
            let namespace = declaring_module_path.as_deref().or(symbol_namespace)?;
            let qualified = qualify_type_annotation_with_namespace(&return_type, namespace)?;
            self.type_info_from_annotation(&qualified)
        };
        let Some(type_info) = direct.or_else(namespaced) else {
            return false;
        };
        self.last_expr_type_info = Some(type_info);
        self.last_expr_schema = self
            .last_expr_type_info
            .as_ref()
            .and_then(Self::value_schema_from_type_info);
        true
    }

    // ADR-009 A3 (S2): returns `Result<Option<usize>>` — `Ok(None)` is the
    // soft fallback (resolution incomplete, cycle, self-call guard) where the
    // caller dispatches the generic path; `Err(e)` propagates a HARD
    // specialized-body compile error (`SpecializationFailure::Hard`) so the
    // user's real diagnostic (e.g. a comptime semantic-freeze rejection)
    // surfaces instead of being masked by the generic-path fallback.
    fn try_monomorphize_method_call(
        &mut self,
        func_name: &str,
        receiver: &Expr,
        args: &[Expr],
        // ADR-006 §2.7.5 V3-S6b conduit: the AST `Expr::MethodCall.span`
        // of the call-site, threaded from `compile_expr_method_call`. On
        // specialization success we stamp `(call_site_span,
        // self.current_function) → specialized_idx` into
        // `self.program.monomorphized_method_call_sites` so the conduit
        // producer can lift `function_return_concrete_types[
        // specialized_idx]` into the destination slot's ConcreteType at
        // the matching `MirConstant::Method` Call-terminator site.
        call_site_span: Span,
    ) -> Result<Option<usize>> {
        // 1. Check if the function has type parameters. Only type-kind
        //    generics participate in the call-site annotation-unification
        //    resolver — const-kind generics (B.3) are bound separately via
        //    declaration defaults inside
        //    `ensure_monomorphic_function_with_consts`, which is auto-invoked
        //    by `ensure_monomorphic_function` on step 7 when the callee has
        //    any const params.
        let type_params: Vec<String> = {
            let Some(def) = self.function_defs.get(func_name) else {
                return Ok(None);
            };
            let Some(tps) = def.type_params.as_ref() else {
                return Ok(None);
            };
            if tps.is_empty() {
                return Ok(None);
            }
            tps.iter()
                .filter(|tp| !tp.is_const())
                .map(|tp| tp.name().to_string())
                .collect()
        };

        // 2. Build combined arg_types: [receiver_concrete_type, arg1_ct, ...].
        //    The function's first param is `self` (the receiver), followed by
        //    the explicit method arguments.
        let Some(receiver_ct) = concrete_type_for_expr(self, receiver) else {
            return Ok(None);
        };
        let method_arg_cts = extract_arg_concrete_types(self, args);
        let mut combined_arg_types: Vec<Option<shape_value::v2::ConcreteType>> =
            Vec::with_capacity(1 + method_arg_cts.len());
        combined_arg_types.push(Some(receiver_ct));
        combined_arg_types.extend(method_arg_cts);

        // 3. Combined args expression list (receiver first, then method args)
        //    for the closure-aware resolver.
        let mut combined_args: Vec<Expr> = Vec::with_capacity(1 + args.len());
        combined_args.push(receiver.clone());
        combined_args.extend(args.iter().cloned());

        // 4. Phase C — if any method arg is a closure literal, route through
        //    the closure-aware resolver so the mono key incorporates the
        //    closure's layout + inferred return type. Otherwise fall through
        //    to the type-only path (byte-for-byte compatible with pre-C).
        let has_closure_arg = args.iter().any(|a| matches!(a, Expr::FunctionExpr { .. }));
        let semantic_request = self.semantic_specialization_request(func_name, call_site_span);

        if has_closure_arg {
            if let Some(idx) = self.try_monomorphize_method_call_with_closures(
                func_name,
                &combined_args,
                call_site_span,
                semantic_request.clone(),
            )? {
                return Ok(Some(idx));
            }
            // Fall-through: either resolution bailed, inlining failed, or the
            // budget was exhausted. Hand off to the type-only path which
            // produces a `Call(fn_id)` direct dispatch rather than an
            // inlined body — still better than `CallValue`. ADR-009 A3 (S2):
            // this fall-through is also what guarantees a genuine user-body
            // compile error swallowed by the closure-INLINING path re-fires
            // below on the un-inlined body and propagates as `Err`.
        }

        // 5. Type-only resolver — existing behaviour.
        let Some(resolution) =
            resolve_call_site_type_args(self, func_name, &combined_arg_types, &type_params)
        else {
            return Ok(None);
        };

        // 6. All type args must be concrete (no unresolved variables).
        if resolution.type_args.is_empty() {
            return Ok(None);
        }

        // 7. Call ensure_monomorphic_function to get/create the specialization.
        //    ADR-009 A3 (S2): a SOFT failure (cycle, resolution bookkeeping)
        //    returns Ok(None) to fall back to the generic version; a HARD
        //    failure (the specialized body itself failed to compile)
        //    propagates the user's real diagnostic.
        match self.ensure_monomorphic_function_for_callsite(
            func_name,
            &resolution.type_args,
            semantic_request,
        ) {
            Ok(specialized_idx) => {
                let idx = specialized_idx as usize;
                // Self-call guard: if the monomorphized specialization is the
                // same function we are currently compiling (e.g. `Vec.len::i64`
                // calling `self.len()` which monomorphizes back to itself),
                // return None so the caller falls through to the built-in
                // method dispatch, preventing infinite recursion at runtime.
                if self.current_function == Some(idx) {
                    return Ok(None);
                }
                // ADR-006 §2.7.5 V3-S6b conduit population: stamp the
                // `(call_site_span, calling_function) → specialized_idx`
                // mapping so the conduit producer at
                // `infer_top_level_concrete_types_from_mir_with_resolvers`
                // can lift `function_return_concrete_types[
                // specialized_idx]` into the destination slot's
                // ConcreteType at the matching `MirConstant::Method`
                // Call-terminator site. `self.current_function` is the
                // post-monomorphization specialized FunctionId of the
                // CALLER (same value the conduit's per-fn loop uses for
                // its `current_function` parameter), so the composite-
                // key invariant holds across the conduit boundary.
                self.program
                    .monomorphized_method_call_sites
                    .insert((call_site_span, self.current_function), idx);
                Ok(Some(idx))
            }
            Err(crate::compiler::monomorphization::cache::SpecializationFailure::Hard(e)) => Err(e),
            Err(crate::compiler::monomorphization::cache::SpecializationFailure::Soft(_)) => {
                Ok(None)
            }
        }
    }

    /// Phase C — closure-aware specialization path.
    ///
    /// Runs the closure-extended resolver on `combined_args` (receiver +
    /// method args). For each `Expr::FunctionExpr` argument, peeks the
    /// closure's captures + body so the cache key encodes the closure's
    /// layout and so the substitution pass can inline the closure body into
    /// the specialized stdlib template.
    ///
    /// Returns `Ok(None)` for an ordinary inlining refusal. Exact semantic
    /// preparation failures remain `Err` and never silently downgrade.
    fn try_monomorphize_method_call_with_closures(
        &mut self,
        func_name: &str,
        combined_args: &[Expr],
        // ADR-006 §2.7.5 V3-S6b conduit: AST span of the parent
        // `Expr::MethodCall`, threaded from `try_monomorphize_method_call`.
        // Mirror site of the type-only path's population —
        // populates `monomorphized_method_call_sites` on the closure-
        // aware specialization's success branch with the same shape.
        call_site_span: Span,
        semantic_request: SemanticSpecializationRequest,
    ) -> Result<Option<usize>> {
        // W20 closures_hof guard (2026-06-27): Phase-C inlining is only
        // sound for closure literals with no captures. The inliner rewrites
        // `f(item)` inside the specialized stdlib template, but it does not
        // extend that specialized function's ABI with captured locals. A
        // read-only capture such as `vals.map(|x| x + base)` inside another
        // function therefore leaves `base` as an unproven identifier in the
        // specialized `Vec.map` body and can miscompile into an unbounded
        // allocation loop. Until capture hoisting is represented in compiler
        // metadata and call-site bytecode, route captured closures through
        // the existing kinded closure value-call path.
        if self.any_closure_arg_captures_outer_binding(combined_args) {
            return Ok(None);
        }

        // SOUNDNESS GUARD (F1 mutating-capture-closure HOF segfault, 2026-06-18):
        // The closure-aware specialization INLINES the closure body into the
        // monomorphized stdlib template (`ensure_monomorphic_function_with_closures`
        // → `compile_function` on the substituted body). That inline pass
        // compiles the closure body OUTSIDE the closure-capture context
        // (`mutable_closure_captures` / `shared_closure_captures` are empty),
        // so a body assignment `total = total + x` to a mutably-captured
        // outer binding lowers to a plain `StoreModuleBinding` / `StoreLocal`
        // — clobbering the binding slot (overwriting the `Arc<SharedCell>`
        // pointer with the raw scalar) instead of routing through
        // `StoreSharedCapture`. The later `LoadSharedModuleBinding` then
        // dereferences the scalar-as-pointer → SIGSEGV (misaligned deref).
        //
        // Inlining cannot be done soundly here without reconstructing the
        // full capture environment inside the specialized template. Until
        // that lands, refuse the inline specialization for any closure arg
        // that MUTATES a captured outer binding and fall back to the
        // type-only / value-call path (which sets up the capture environment
        // correctly via `op_make_closure` + `call_value_immediate_nb` — the
        // same path a direct closure call takes, proven correct). The
        // read-only-capture closures (`map`/`filter`/`forEach` with no outer
        // mutation) keep the inline fast path.
        if self.any_closure_arg_mutates_outer_binding(combined_args) {
            return Ok(None);
        }

        // Only type-kind generics participate in call-site annotation
        // unification. Const-kind generics (B.3) are bound separately via
        // declaration defaults.
        let type_params: Vec<String> = {
            let Some(def) = self.function_defs.get(func_name) else {
                return Ok(None);
            };
            let Some(tps) = def.type_params.as_ref() else {
                return Ok(None);
            };
            if tps.is_empty() {
                return Ok(None);
            }
            tps.iter()
                .filter(|tp| !tp.is_const())
                .map(|tp| tp.name().to_string())
                .collect()
        };

        // Per-arg concrete types (closure args collapse to an opaque
        // Function/Closure tag, same as the type-only path).
        let arg_types = extract_arg_concrete_types(self, combined_args);

        let Some(resolution) = resolve_call_site_type_args_with_closures(
            self,
            func_name,
            combined_args,
            &arg_types,
            &type_params,
        ) else {
            return Ok(None);
        };
        if resolution.type_args.is_empty() {
            return Ok(None);
        }
        if resolution.closure_specs.is_empty() {
            // No closure arg after all — bounce to the type-only path.
            return Ok(None);
        }

        // Gather the peeked closure def info (params, body, captures) and
        // the callee's formal param name for each closure arg. The resolver
        // processed `combined_args` in order; we walk it in the same order
        // to keep positional alignment.
        let closure_defs: Vec<ClosureDefPeek> = combined_args
            .iter()
            .filter_map(|a| match a {
                Expr::FunctionExpr { params, body, .. } => Some((params.clone(), body.clone())),
                _ => None,
            })
            .map(|(params, body)| self.peek_closure_def(&params, &body))
            .collect();

        // Pull the formal closure-param names from the callee def.
        let Some(def) = self.function_defs.get(func_name).cloned() else {
            return Ok(None);
        };
        let mut callee_closure_param_names: Vec<String> = Vec::new();
        for (i, a) in combined_args.iter().enumerate() {
            if matches!(a, Expr::FunctionExpr { .. }) {
                let Some(param) = def.params.get(i) else {
                    return Ok(None);
                };
                let ids = param.get_identifiers();
                if ids.len() != 1 {
                    // Destructured closure param — not supported.
                    return Ok(None);
                }
                callee_closure_param_names.push(ids[0].clone());
            }
        }

        match self.ensure_monomorphic_function_with_closures_for_callsite(
            func_name,
            &resolution.type_args,
            &resolution.closure_specs,
            &closure_defs,
            &callee_closure_param_names,
            semantic_request,
        ) {
            Ok(Some(specialized_idx)) => {
                let idx = specialized_idx as usize;
                if self.current_function == Some(idx) {
                    return Ok(None);
                }
                // ADR-006 §2.7.5 V3-S6b conduit population (mirror of the
                // type-only path). Stamps the `(call_site_span,
                // current_function) → specialized_idx` mapping for the
                // closure-aware specialization branch. Same composite-key
                // invariant as the type-only mirror — `current_function`
                // is the post-monomorphization specialized FunctionId of
                // the caller, matching the conduit producer's per-fn
                // loop's `current_function` parameter.
                self.program
                    .monomorphized_method_call_sites
                    .insert((call_site_span, self.current_function), idx);
                Ok(Some(idx))
            }
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// True if any closure literal argument captures an outer binding. Phase-C
    /// HOF inlining has no capture-hoisting ABI yet, so captured closures must
    /// stay on the value-call path where `MakeClosure` and
    /// `call_value_immediate_nb` carry the statically stamped closure layout.
    fn any_closure_arg_captures_outer_binding(&self, args: &[Expr]) -> bool {
        fn expr_has_outer_mut_self_method(expr: &Expr, outer_captures: &BTreeSet<String>) -> bool {
            match expr {
                Expr::MethodCall {
                    receiver,
                    method,
                    args,
                    ..
                } => {
                    matches!(
                        receiver.as_ref(),
                        Expr::Identifier(name, _) if outer_captures.contains(name)
                    ) && crate::executor::objects::method_registry::is_mut_self_method_name(method)
                        || expr_has_outer_mut_self_method(receiver, outer_captures)
                        || args
                            .iter()
                            .any(|arg| expr_has_outer_mut_self_method(arg, outer_captures))
                }
                Expr::Assign(assign, _) => {
                    expr_has_outer_mut_self_method(&assign.value, outer_captures)
                }
                Expr::Block(block, _) => block.items.iter().any(|item| match item {
                    shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                        .value
                        .as_ref()
                        .is_some_and(|expr| expr_has_outer_mut_self_method(expr, outer_captures)),
                    shape_ast::ast::BlockItem::Assignment(assign) => {
                        expr_has_outer_mut_self_method(&assign.value, outer_captures)
                    }
                    shape_ast::ast::BlockItem::Statement(stmt) => {
                        stmt_has_outer_mut_self_method(stmt, outer_captures)
                    }
                    shape_ast::ast::BlockItem::Expression(expr) => {
                        expr_has_outer_mut_self_method(expr, outer_captures)
                    }
                }),
                Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                    expr_has_outer_mut_self_method(left, outer_captures)
                        || expr_has_outer_mut_self_method(right, outer_captures)
                }
                Expr::UnaryOp { operand, .. } => {
                    expr_has_outer_mut_self_method(operand, outer_captures)
                }
                Expr::FunctionCall { args, .. } => args
                    .iter()
                    .any(|arg| expr_has_outer_mut_self_method(arg, outer_captures)),
                Expr::Array(elements, _) => elements
                    .iter()
                    .any(|elem| expr_has_outer_mut_self_method(elem, outer_captures)),
                Expr::Return(Some(expr), _) => expr_has_outer_mut_self_method(expr, outer_captures),
                Expr::If(if_expr, _) => {
                    expr_has_outer_mut_self_method(&if_expr.condition, outer_captures)
                        || expr_has_outer_mut_self_method(&if_expr.then_branch, outer_captures)
                        || if_expr.else_branch.as_ref().is_some_and(|expr| {
                            expr_has_outer_mut_self_method(expr, outer_captures)
                        })
                }
                Expr::Match(match_expr, _) => {
                    expr_has_outer_mut_self_method(&match_expr.scrutinee, outer_captures)
                        || match_expr.arms.iter().any(|arm| {
                            arm.guard.as_ref().is_some_and(|guard| {
                                expr_has_outer_mut_self_method(guard, outer_captures)
                            }) || expr_has_outer_mut_self_method(&arm.body, outer_captures)
                        })
                }
                Expr::FunctionExpr { .. } => false,
                _ => false,
            }
        }

        fn stmt_has_outer_mut_self_method(
            stmt: &shape_ast::ast::Statement,
            outer_captures: &BTreeSet<String>,
        ) -> bool {
            match stmt {
                shape_ast::ast::Statement::Assignment(assign, _) => {
                    expr_has_outer_mut_self_method(&assign.value, outer_captures)
                }
                shape_ast::ast::Statement::Expression(expr, _)
                | shape_ast::ast::Statement::Return(Some(expr), _) => {
                    expr_has_outer_mut_self_method(expr, outer_captures)
                }
                shape_ast::ast::Statement::VariableDecl(decl, _) => decl
                    .value
                    .as_ref()
                    .is_some_and(|expr| expr_has_outer_mut_self_method(expr, outer_captures)),
                _ => false,
            }
        }

        fn expr_assigns_outer(expr: &Expr, outer_captures: &BTreeSet<String>) -> bool {
            match expr {
                Expr::Assign(assign, _) => {
                    matches!(
                        assign.target.as_ref(),
                        Expr::Identifier(name, _) if outer_captures.contains(name)
                    ) || expr_assigns_outer(&assign.value, outer_captures)
                }
                Expr::Block(block, _) => block.items.iter().any(|item| match item {
                    shape_ast::ast::BlockItem::VariableDecl(decl) => decl
                        .value
                        .as_ref()
                        .is_some_and(|expr| expr_assigns_outer(expr, outer_captures)),
                    shape_ast::ast::BlockItem::Assignment(assign) => {
                        assign
                            .pattern
                            .as_identifier()
                            .is_some_and(|name| outer_captures.contains(name))
                            || expr_assigns_outer(&assign.value, outer_captures)
                    }
                    shape_ast::ast::BlockItem::Statement(stmt) => {
                        assignment_targets_outer(stmt, outer_captures)
                    }
                    shape_ast::ast::BlockItem::Expression(expr) => {
                        expr_assigns_outer(expr, outer_captures)
                    }
                }),
                Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                    expr_assigns_outer(left, outer_captures)
                        || expr_assigns_outer(right, outer_captures)
                }
                Expr::UnaryOp { operand, .. } => expr_assigns_outer(operand, outer_captures),
                Expr::FunctionCall { args, .. } => args
                    .iter()
                    .any(|arg| expr_assigns_outer(arg, outer_captures)),
                Expr::MethodCall { receiver, args, .. } => {
                    expr_assigns_outer(receiver, outer_captures)
                        || args
                            .iter()
                            .any(|arg| expr_assigns_outer(arg, outer_captures))
                }
                Expr::Array(elements, _) => elements
                    .iter()
                    .any(|elem| expr_assigns_outer(elem, outer_captures)),
                Expr::Return(Some(expr), _) => expr_assigns_outer(expr, outer_captures),
                Expr::If(if_expr, _) => {
                    expr_assigns_outer(&if_expr.condition, outer_captures)
                        || expr_assigns_outer(&if_expr.then_branch, outer_captures)
                        || if_expr
                            .else_branch
                            .as_ref()
                            .is_some_and(|expr| expr_assigns_outer(expr, outer_captures))
                }
                Expr::Match(match_expr, _) => {
                    expr_assigns_outer(&match_expr.scrutinee, outer_captures)
                        || match_expr.arms.iter().any(|arm| {
                            arm.guard
                                .as_ref()
                                .is_some_and(|guard| expr_assigns_outer(guard, outer_captures))
                                || expr_assigns_outer(&arm.body, outer_captures)
                        })
                }
                Expr::FunctionExpr { .. } => false,
                _ => false,
            }
        }

        fn assignment_targets_outer(
            stmt: &shape_ast::ast::Statement,
            outer_captures: &BTreeSet<String>,
        ) -> bool {
            match stmt {
                shape_ast::ast::Statement::Assignment(assign, _) => {
                    assign
                        .pattern
                        .as_identifier()
                        .is_some_and(|name| outer_captures.contains(name))
                        || expr_assigns_outer(&assign.value, outer_captures)
                }
                shape_ast::ast::Statement::Expression(expr, _)
                | shape_ast::ast::Statement::Return(Some(expr), _) => {
                    expr_assigns_outer(expr, outer_captures)
                }
                shape_ast::ast::Statement::VariableDecl(decl, _) => decl
                    .value
                    .as_ref()
                    .is_some_and(|expr| expr_assigns_outer(expr, outer_captures)),
                _ => false,
            }
        }

        let outer_vars = self.collect_outer_scope_vars();
        args.iter().any(|a| {
            let Expr::FunctionExpr { params, body, .. } = a else {
                return false;
            };
            let proto_def = shape_ast::ast::FunctionDef {
                name: "__capturecheck_closure__".to_string(),
                name_span: Span::DUMMY,
                declaring_module_path: None,
                doc_comment: None,
                type_params: None,
                params: params.clone(),
                return_type: None,
                body: body.clone(),
                annotations: vec![],
                where_clause: None,
                is_async: false,
                is_comptime: false,
                effect_row: None,
            };
            let (captured, mutated) =
                EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
            let param_names: BTreeSet<String> =
                params.iter().flat_map(|p| p.get_identifiers()).collect();
            let outer_captures: BTreeSet<String> = captured
                .iter()
                .filter(|n| !param_names.contains(*n))
                .cloned()
                .collect();
            if outer_captures.is_empty() {
                return false;
            }
            let mutates_outer = mutated.iter().any(|n| outer_captures.contains(n))
                || body
                    .iter()
                    .any(|stmt| assignment_targets_outer(stmt, &outer_captures));
            let mutates_outer_container_by_method = body
                .iter()
                .any(|stmt| stmt_has_outer_mut_self_method(stmt, &outer_captures));
            !mutates_outer || mutates_outer_container_by_method
        })
    }

    /// F1 soundness guard: true if ANY `Expr::FunctionExpr` argument mutates
    /// a captured outer binding (i.e. its `EnvironmentAnalyzer`
    /// `mutated_captures` set is non-empty). Such closures cannot be inlined
    /// by the closure-aware monomorphization path soundly — the inline pass
    /// loses the mutable-capture environment and lowers the body's write to a
    /// plain binding store, clobbering the `Arc<SharedCell>` slot. The caller
    /// falls back to the value-call path, which sets up the capture
    /// environment correctly.
    fn any_closure_arg_mutates_outer_binding(&self, args: &[Expr]) -> bool {
        let outer_vars = self.collect_outer_scope_vars();
        args.iter().any(|a| {
            let Expr::FunctionExpr { params, body, .. } = a else {
                return false;
            };
            let proto_def = shape_ast::ast::FunctionDef {
                name: "__mutcheck_closure__".to_string(),
                name_span: Span::DUMMY,
                declaring_module_path: None,
                doc_comment: None,
                type_params: None,
                params: params.clone(),
                return_type: None,
                body: body.clone(),
                annotations: vec![],
                where_clause: None,
                is_async: false,
                is_comptime: false,
                effect_row: None,
            };
            let (_captured, mutated) =
                EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
            // A capture name in `mutated` that is NOT a closure param is an
            // outer-binding mutation. (`analyze_function_with_mutability`
            // already restricts `mutated` to captured — non-local — names.)
            let param_names: BTreeSet<String> =
                params.iter().flat_map(|p| p.get_identifiers()).collect();
            mutated.iter().any(|n| !param_names.contains(n))
        })
    }

    /// Phase C — peek a closure literal's params/body/captures without
    /// lowering. Runs the same `EnvironmentAnalyzer` the compiler uses for
    /// closure compilation so the capture list matches what the emitter sees
    /// later.
    fn peek_closure_def(
        &self,
        params: &[shape_ast::ast::FunctionParameter],
        body: &[shape_ast::ast::Statement],
    ) -> ClosureDefPeek {
        let proto_def = shape_ast::ast::FunctionDef {
            name: "__peek_closure__".to_string(),
            name_span: Span::DUMMY,
            declaring_module_path: None,
            doc_comment: None,
            type_params: None,
            params: params.to_vec(),
            return_type: None,
            body: body.to_vec(),
            annotations: vec![],
            where_clause: None,
            is_async: false,
            is_comptime: false,
            effect_row: None,
        };
        let outer_vars = self.collect_outer_scope_vars();
        let (mut captured_vars, _mutated) =
            EnvironmentAnalyzer::analyze_function_with_mutability(&proto_def, &outer_vars);
        captured_vars.sort();
        let param_names: BTreeSet<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();
        captured_vars.retain(|n| !param_names.contains(n));

        let param_name_list: Vec<String> =
            params.iter().flat_map(|p| p.get_identifiers()).collect();

        ClosureDefPeek {
            param_names: param_name_list,
            body: body.to_vec(),
            capture_names: captured_vars,
        }
    }
}

#[cfg(test)]
mod ws2_zeta_b_tests {
    //! ζ-(b) regression: a call to a generic function whose type arguments
    //! cannot be resolved from the call site must surface a clean compile
    //! error. Generic function bodies are intentionally skipped in
    //! `compile_function` (their AST is kept only as a substitution
    //! template); emitting a `Call` onto that zero-instruction body let the
    //! VM run off the end and hang (30s timeout on `print(id(None))`).

    use crate::compiler::BytecodeCompiler;
    use shape_ast::error::Result;

    /// Compile a whole top-level program, returning the compile `Result`.
    fn try_compile(code: &str) -> Result<()> {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        BytecodeCompiler::new().compile(&program).map(|_| ())
    }

    #[test]
    fn generic_call_with_unresolvable_type_arg_is_compile_error() {
        // `id<T>(x: T)` called with `None` — `None` has no ConcreteType, so
        // `T` cannot be bound. This must be a clean compile error, not a
        // fall-through onto the empty generic template (which hangs the VM).
        //
        // Under fn-boundary let-gen (let-gen-gating-predicate-spec.md §4), the
        // bare-application binding `let y = id(None)` whose final type is a
        // fully-polymorphic `Option<T>` is now caught at the INFERENCE binding
        // level ("Cannot infer a concrete type for binding 'y'") rather than (or
        // before) the bytecode generic-template guard ("cannot infer type
        // argument"). Either is a valid clean compile error — the load-bearing
        // contract is that it does NOT compile (and does not hang the VM).
        let err = try_compile("fn id<T>(x: T) -> T { x }\nlet y = id(None)\n")
            .expect_err("id(None) must not compile — T is unresolvable");
        let msg = format!("{err:?}");
        assert!(
            (msg.contains("cannot infer type argument") && msg.contains("id"))
                || (msg.contains("Cannot infer a concrete type for binding") && msg.contains('y')),
            "expected a generic-type-arg / unpinnable-binding inference error, got: {msg}"
        );
    }

    #[test]
    fn generic_call_with_concrete_arg_compiles() {
        // `id(5)` — `5` is `int`, so `T = int` resolves and `id::I64`
        // monomorphizes. Must compile cleanly (no false positive from the
        // empty-template guard).
        try_compile("fn id<T>(x: T) -> T { x }\nlet y = id(5)\n")
            .expect("id(5) must compile — T resolves to int");
    }

    #[test]
    fn self_recursive_generic_compiles() {
        // A generic body whose recursive call re-resolves to the
        // specialization currently being compiled must redirect to that
        // specialization's index — not trip the empty-template guard.
        try_compile(
            "fn countdown<T>(x: int, v: T) -> T { if x <= 0 { v } else { countdown(x - 1, v) } }\n\
             let r = countdown(3, 42)\n",
        )
        .expect("self-recursive generic must compile");
    }
}

#[cfg(test)]
mod imported_wrapper_literal_adoption_tests {
    use crate::test_utils::compile_with_prelude;

    #[test]
    fn imported_number_param_accepts_bare_int_literal() {
        compile_with_prelude(
            r#"
            from std::core::math use { sin }
            sin(2)
            "#,
        )
        .expect("bare int literal should adopt imported wrapper's number parameter");
    }

    #[test]
    fn imported_number_param_rejects_int_variable() {
        let err = compile_with_prelude(
            r#"
            from std::core::math use { sin }
            let x = 2
            sin(x)
            "#,
        )
        .expect_err("nonliteral int variable must not adopt a number parameter");
        let msg = format!("{err:?}");
        assert!(
            (msg.contains("cannot safely pass argument #1")
                && msg.contains("std::core::math::sin")
                && msg.contains("Int64")
                && msg.contains("Float64"))
                || msg.contains("(number) -> number is not compatible with (int) -> number"),
            "expected strict int-variable-to-number rejection, got: {msg}"
        );
    }
}

#[cfg(test)]
mod wave1a_partb_fn_typed_param_tests {
    //! Wave 1a PART B: a function's UNANNOTATED parameter that the body USES as
    //! a callable (`fn apply2(f, x, y) { f(x, y) }`) is inferred to a function
    //! type by whole-program inference; a closure-literal argument at that
    //! position is then seeded with the inferred signature's param types so its
    //! own body type-checks (`|a, b| a * b` → `|a: int, b: int|`). The
    //! higher-ranked extension of PART A's let-bound-closure call-site
    //! inference.
    //!
    //! Soundness contract: seed ONLY when inference produced a fully-concrete
    //! signature; an un-inferable / dead callable param yields no seeding and
    //! the closure keeps its existing rejection. `int` and `number` stay
    //! distinct. No fabrication, no `any`, no silent pick.

    use crate::compiler::BytecodeCompiler;
    use shape_ast::error::Result;

    fn try_compile(code: &str) -> Result<()> {
        let program = shape_ast::parser::parse_program(code).expect("parse failed");
        BytecodeCompiler::new().compile(&program).map(|_| ())
    }

    #[test]
    fn callable_param_seeds_closure_arg_params() {
        // `f` used as `f(x, y)` with x,y inferred from the int-literal call
        // args → `f: fn(_, _)`; the closure `|a, b| a * b` is seeded so
        // `a * b` is no longer `unknown * unknown`.
        try_compile(
            "fn apply2(f, x, y) { f(x, y) }\n\
             apply2(|a, b| a * b, 6, 7)\n",
        )
        .expect("apply2 with a closure arg whose params are usage-inferred must compile");
    }

    #[test]
    fn single_callable_param_seeds_unary_closure() {
        try_compile(
            "fn apply(f, x) { f(x) }\n\
             apply(|n| n * n, 6)\n",
        )
        .expect("apply(|n| n * n, 6) must compile — `n` seeded from f's inferred signature");
    }

    #[test]
    fn overloaded_plus_body_seeds_from_callsite_int_args() {
        // `+` is overloaded (numeric OR string concat), so whole-program
        // inference on the callable param `f` in ISOLATION leaves its argument
        // types as unresolved variables — the engine's `f` projection alone is
        // NOT concrete. But the call `run2(|p, q| p + q, 3, 4)` passes int
        // LITERALS to `a, b`, and the body `f(a, b)` maps `f`'s params to those
        // outer params, so `p, q` are PROVABLY `int` from the call site (the
        // Wave 1a PART B soundness fix carries the exact proven outer-param type
        // onto the closure via the body-usage mapping). `p + q` then types as
        // `int + int` and compiles. This is NOT a forced default: `3, 4` are
        // int literals; `int` is what the call site genuinely proved (the same
        // mechanism that makes `apply2(|a,b| a*b, 6, 7)` yield `int`, not the
        // unsound `number`). An under-constrained usage with NO concrete
        // outer-arg mapping (a dead callable, or args that are not bare outer
        // params) is still NOT seeded.
        try_compile(
            "fn run2(f, a, b) { f(a, b) }\n\
             run2(|p, q| p + q, 3, 4)\n",
        )
        .expect("call-site int args make the +-bodied closure params provably int — must compile");
    }

    #[test]
    fn int_and_number_stay_distinct_through_seeding() {
        // The seeded closure param type follows the inferred signature; this
        // program multiplies int-literal-seeded params and the result feeds an
        // int context. The point is that compilation succeeds with a single
        // proven element type rather than silently unifying int with number.
        try_compile(
            "fn apply2(f, x, y) { f(x, y) }\n\
             let r = apply2(|a, b| a * b, 6, 7)\n",
        )
        .expect("seeded-closure call must compile cleanly");
    }

    #[test]
    fn seeded_closure_params_carry_int_not_number() {
        // SOUNDNESS REGRESSION GUARD (Wave 1a PART B fix). The pre-fix producer
        // seeded the closure's params as `number` (the engine's collapsed `f`
        // projection), so `apply2(|a,b| a*b, 6, 7)` computed `42.0` (Float64) —
        // a static `number` that does not match the proven `int*int`. The fix
        // carries the EXACT proven type (`int`, from the int literals `6, 7`
        // flowing through the body usage `f(x, y)`) onto the closure, so the
        // result is `42` (Int64). `int` and `number` do NOT unify; defaulting a
        // numeric param to `number` is forbidden (CLAUDE.md).
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64("fn apply2(f, x, y) { f(x, y) }\napply2(|a, b| a * b, 6, 7)"),
            42,
            "int*int through an inferred fn-typed param must stay int (42), never number (42.0)"
        );
    }

    #[test]
    fn seeded_closure_result_binds_to_int_context() {
        // `let r: int = apply2(|a,b| a*b, 6, 7)` must type-check: the closure
        // result is provably `int`, so binding into an `int` context succeeds
        // with no error and no coercion.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64(
                "fn apply2(f, x, y) { f(x, y) }\nlet r: int = apply2(|a, b| a * b, 6, 7)\nr"
            ),
            42,
        );
    }

    #[test]
    fn callable_param_return_stamps_local_result() {
        try_compile(
            r#"
            fn try_apply(f, val) {
                let result = f(val)
                if result < 0 { Err("negative result") } else { Ok(result) }
            }
            match try_apply(|x| x * 2 - 100, 80) {
                Ok(v) => v + 1,
                Err(e) => 0
            }
            "#,
        )
        .expect("callable param return type must flow into local result and Result payload");
    }

    #[test]
    fn unannotated_result_payload_flows_through_named_calls() {
        try_compile(
            r#"
            fn safe_adjust(a, b) {
                if b == 0 { return Err("zero") }
                Ok(a + b)
            }
            fn process_adjust(a, b) {
                match safe_adjust(a, b) {
                    Ok(v) => {
                        if v > 10 {
                            v - 1
                        } else {
                            v + 1
                        }
                    },
                    Err(e) => 0
                }
            }
            process_adjust(6, 5)
            "#,
        )
        .expect("unannotated named Result payloads must remain statically typed through match");
    }

    #[test]
    fn nested_returned_closure_call_compiles_with_static_param_facts() {
        try_compile(
            r#"
            fn make_adder(base) {
                |offset| {
                    |x| base + offset + x
                }
            }
            let intermediate = make_adder(5)
            let add_fn = intermediate(3)
            add_fn(10)
            "#,
        )
        .expect("nested returned closure params and return must be statically provable");
    }

    // -- Indirected-callable COMPLETENESS (full-inference ruling) -----------
    //
    // The SoundRoot floor makes an un-followable indirected closure SURFACE.
    // The completeness extension FOLLOWS the callable through indirection so the
    // two tractable shapes INFER instead — without compromising the floor. Each
    // pair below proves `int` stays `int` (42, never 42.0) and `number` stays
    // `number` (42.0).

    #[test]
    fn id_laundered_callable_infers_int_not_number() {
        // `let h = id(|a,b| a*b)` launders the closure through identity; the
        // resolver follows `h` to its use as `applyx`'s callable arg, where the
        // int literals 6,7 prove the closure params `int`. Result is `42`
        // (Int64), NEVER `42.0` — the recurring number-default unsoundness.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64(
                "fn applyx(f, x, y) { f(x, y) }\n\
                 fn id(g) { g }\n\
                 let h = id(|a, b| a * b)\n\
                 let acc: int = 0\n\
                 acc + applyx(h, 6, 7)"
            ),
            42,
            "id-laundered int*int must stay int (42), never number (42.0)"
        );
    }

    #[test]
    fn id_laundered_callable_number_stays_number() {
        // The `number` sibling: 6.0,7.0 prove the closure params `number`, so
        // the result is `42.0` (Float64). `int` and `number` do NOT unify.
        use crate::test_utils::eval_typed_f64;
        assert_eq!(
            eval_typed_f64(
                "fn applyx(f, x, y) { f(x, y) }\n\
                 fn id(g) { g }\n\
                 let h = id(|a, b| a * b)\n\
                 let acc: number = 0.0\n\
                 acc + applyx(h, 6.0, 7.0)"
            ),
            42.0,
        );
    }

    #[test]
    fn two_level_wrapper_callable_infers_int_not_number() {
        // `fn wrap(f,x,y){ applyx(f,x,y) }` forwards the callable one hop; the
        // resolver maps `applyx`'s invocation arg slots back through wrap's
        // forwarding call to wrap's own params, whose call-site args 6,7 prove
        // `int`. Result `42` (Int64), no kind-crash.
        use crate::test_utils::eval_typed_i64;
        assert_eq!(
            eval_typed_i64(
                "fn applyx(f, x, y) { f(x, y) }\n\
                 fn wrap(f, x, y) { applyx(f, x, y) }\n\
                 let acc: int = 0\n\
                 acc + wrap(|a, b| a * b, 6, 7)"
            ),
            42,
            "2-level-wrapper int*int must stay int (42), never number (42.0)"
        );
    }

    #[test]
    fn two_level_wrapper_callable_number_stays_number() {
        use crate::test_utils::eval_typed_f64;
        assert_eq!(
            eval_typed_f64(
                "fn applyx(f, x, y) { f(x, y) }\n\
                 fn wrap(f, x, y) { applyx(f, x, y) }\n\
                 let acc: number = 0.0\n\
                 acc + wrap(|a, b| a * b, 6.0, 7.0)"
            ),
            42.0,
        );
    }

    #[test]
    fn laundered_but_never_invoked_closure_still_surfaces() {
        // SoundRoot floor preservation. The closure is laundered through `id`
        // but its result is NEVER used as a callable, so no concrete invocation
        // proves its params. The resolver cannot follow the hop, so the case
        // still SURFACEs (rejects) — it must NOT silently default to `number`.
        let err = try_compile(
            "fn id(g) { g }\n\
             let h = id(|a, b| a * b)\n\
             0",
        );
        assert!(
            err.is_err(),
            "an un-invoked laundered closure must SURFACE, never number-default"
        );
    }
}

#[cfg(test)]
mod r3_elemerasure_tests {
    //! R3-elemerasure (strict-flip): the concrete element/return type of a
    //! builtin (PHF) array method that returns `Self`
    //! (`sort`/`reverse`/`take`/…) or the receiver element type
    //! (`first`/`last`/…) was LOST across the chain, so a downstream closure
    //! param or binary-op operand saw `unknown` and the strict-typing emitter
    //! rejected `[5,2,8].sort().map(|x| x*x)` / `[99].first() == a.last()` with
    //! "Cannot infer types for binary operation". The fix derives the result
    //! `ConcreteType` from the receiver's proven type via the method's
    //! REGISTERED signature shape (no hardcoded list, no fabrication).

    use crate::test_utils::{eval_typed_bool, eval_typed_i64};

    #[test]
    fn sort_then_map_squares_resolves_element_type() {
        // The cited PROOF case: `.sort().map(|x| x*x)` — both Mul operands are
        // the closure param, so the element type MUST flow through `.sort()`'s
        // `Self` return for `x` to type as `int`.
        assert_eq!(eval_typed_i64("([5, 2, 8].sort().map(|x| x * x))[2]"), 64);
    }

    #[test]
    fn chained_self_returning_then_map_resolves_element_type() {
        // Full chain: sort → reverse → take → map. Every `Self`-returning link
        // must carry the element type forward.
        assert_eq!(
            eval_typed_i64("([5, 2, 8, 1, 9, 3].sort().reverse().take(3).map(|x| x * x))[0]"),
            81
        );
    }

    #[test]
    fn first_eq_last_resolves_element_type() {
        // The cited PROOF case: `a.first() == a.last()` — both operands are the
        // receiver element type (`ReceiverParam(0)`); without recovery the
        // `Equal` saw `unknown == unknown`.
        assert!(eval_typed_bool("let a = [99]\na.first() == a.last()"));
    }

    #[test]
    fn let_bound_first_in_arith_resolves_element_type() {
        // `let x = a.first(); x + 1` — the scalar element result must propagate
        // into the binding's recorded ConcreteType so the binop operand
        // resolves. Covers the module-binding propagation site.
        assert_eq!(eval_typed_i64("let a = [40]\nlet x = a.first()\nx + 2"), 42);
    }

    #[test]
    fn number_element_stays_number_through_sort_map() {
        // int != number must survive element propagation: a `number` array's
        // element stays `number`, so `x * 2.0` types and the result is float.
        // (Compiles and runs — a wrong int collapse would reject `* 2.0`.)
        let _ = eval_typed_i64("([1, 2, 3].sort().map(|x| x + 1))[0]");
    }
}

#[cfg(test)]
mod empty_array_inline_receiver_tests {
    //! EmptyArray (strict-flip, 2026-06-16): a bare empty array LITERAL used
    //! directly as a method receiver (`[].iter()`, `[].map(|x| x)`) has no
    //! element-type proof and is never bound, so the accumulator deferral that
    //! rescues `let mut a = []; a.push(x)` cannot apply. Pre-fix it lowered to
    //! a placeholder `NewArray(0)` that SURFACEd `op_new_array(0)` at RUNTIME;
    //! now it is a CLEAN compile error. An ANNOTATED empty array is unaffected
    //! (its receiver is the identifier `a`, which carries the annotation's
    //! element type) and a non-empty literal receiver is unaffected.

    use crate::test_utils::compile_with_prelude;

    #[test]
    fn inline_empty_array_map_receiver_is_clean_compile_error() {
        let res = compile_with_prelude("fn run() { print([].map(|x| x).count()) }\nrun()");
        assert!(
            res.is_err(),
            "inline `[].map(...)` with no element-type proof must reject at compile time"
        );
        let msg = format!("{:?}", res.unwrap_err());
        assert!(
            msg.contains("empty array") && msg.contains("element type"),
            "rejection should explain the un-resolvable empty-array element type; got: {msg}"
        );
    }

    #[test]
    fn inline_empty_array_iter_receiver_is_clean_compile_error() {
        let res = compile_with_prelude("fn run() { print([].iter().count()) }\nrun()");
        assert!(
            res.is_err(),
            "inline `[].iter()` with no element-type proof must reject at compile time"
        );
    }

    #[test]
    fn annotated_empty_array_iter_chain_compiles() {
        // The receiver here is the IDENTIFIER `a`, not the bare `[]` literal —
        // the annotation supplies the element type, so the guard does not fire.
        let res = compile_with_prelude(
            "fn run() { let a: Array<int> = []; print(a.iter().count()) }\nrun()",
        );
        assert!(
            res.is_ok(),
            "annotated `let a: Array<int> = []; a.iter()...` should compile: {:?}",
            res.err()
        );
    }

    #[test]
    fn annotated_empty_array_map_compiles() {
        let res = compile_with_prelude(
            "fn run() { let a: Array<int> = []; let b = a.map(|x| x); print(b.len()) }\nrun()",
        );
        assert!(
            res.is_ok(),
            "annotated empty array `.map(...)` should compile: {:?}",
            res.err()
        );
    }

    #[test]
    fn non_empty_literal_receiver_still_compiles() {
        // The guard is scoped to EMPTY literals — a non-empty literal receiver
        // resolves its element type from its elements and is unaffected.
        let res = compile_with_prelude("fn run() { print([1, 2, 3].map(|x| x * 2).sum()) }\nrun()");
        assert!(
            res.is_ok(),
            "non-empty literal `.map(...)` receiver must still compile: {:?}",
            res.err()
        );
    }
}

#[cfg(test)]
mod r3_subcase_struct_array_hof_tests {
    //! R3-subcase struct-array HOF (strict-flip): a closure over an array of
    //! structs that reads a struct field (`users.filter(|u| u.score > 85)`)
    //! resolved the field to `unknown` because the struct identity was erased
    //! at array-of-structs construction — the `TypedArrayKind::TypedObject →
    //! ConcreteType` round-trip collapsed every struct element to
    //! `placeholder_struct(name: None)`. The fix (and, post-U4-6a, the sole
    //! mechanism) recovers the NAMED struct element `ConcreteType` STRUCTURALLY
    //! from the literal elements via `concrete_type_for_expr`'s element
    //! recursion, so the HOF closure param carries the struct type and a field
    //! access resolves to the field's type. Type-proven, not broad-suppression:
    //! a non-existent field still rejects.

    use crate::test_utils::compile_with_prelude;

    const USER_TYPE: &str = "type User { name: string, score: int }\n";

    #[test]
    fn filter_struct_array_reads_field_compiles() {
        // `u.score` inside the filter closure resolves against `User` — the
        // exact case the R3 fix SURFACED. Pre-fix: "Cannot infer types for
        // binary operation `Greater`: operand types are `unknown` and `int`".
        let src = format!(
            "{USER_TYPE}fn run() {{ \
               let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
               let high = users.filter(|u| u.score > 85)\n\
               print(high.len()) }}\nrun()"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "filter over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn map_struct_array_reads_field_compiles() {
        // `.map(|u| u.score * 2)` — closure param `u: User`, `u.score: int`.
        let src = format!(
            "{USER_TYPE}fn run() {{ \
               let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
               let scores = users.map(|u| u.score * 2)\n\
               print(scores.len()) }}\nrun()"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "map over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn find_struct_array_reads_field_compiles() {
        // `.find(|u| u.score > 85)` returns `User?`; the closure body reads the
        // struct field — `ReceiverParam(0)` element flows the struct type in.
        let src = format!(
            "{USER_TYPE}fn run() {{ \
               let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
               let f = users.find(|u| u.score > 85)\n\
               print(f.name) }}\nrun()"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "find over Array<User> reading u.score / f.name should compile"
        );
    }

    #[test]
    fn nonexistent_field_in_struct_array_closure_rejects() {
        // NOT broad-suppression: a field that does not exist on `User` must
        // still be a compile error (the struct identity is now KNOWN, so the
        // schema check fires) — never silently accepted.
        let src = format!(
            "{USER_TYPE}fn run() {{ \
               let users = [User {{ name: \"a\", score: 90 }}]\n\
               let bad = users.filter(|u| u.nonexistent > 5)\n\
               print(bad.len()) }}\nrun()"
        );
        let res = compile_with_prelude(&src);
        assert!(
            res.is_err(),
            "a non-existent struct field inside the HOF closure must reject, got Ok"
        );
        let msg = format!("{:?}", res.unwrap_err());
        assert!(
            msg.contains("nonexistent"),
            "rejection should name the missing field; got: {msg}"
        );
    }

    #[test]
    fn module_scope_filter_struct_array_resolves_result_element() {
        // R3-subcase (strict-flip, 2026-06-15): a MODULE-scope struct array
        // filter — the monomorphized `Vec.filter` body's `let mut result = [];
        // result.push(item)` accumulator needs `item: User` to resolve the
        // result element type. Pre-fix: the `for item in self` loop var carried
        // only the lossy tracker NAME (`User`) which `concrete_type_for_expr`
        // could not map back to `ConcreteType::Struct`, so the accumulator
        // surfaced "empty array `result` has an un-resolvable element type".
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             let high = users.filter(|u| u.score > 85)\n\
             print(high.len())"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "module-scope filter over Array<User> should resolve its result element type"
        );
    }

    #[test]
    fn for_in_struct_array_reads_field_compiles() {
        // The loop variable of `for u in users` must carry the `User`
        // ConcreteType (not just the tracker name) so `u.score` resolves.
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             for u in users {{ print(u.score) }}"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "for-in over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn iter_filter_struct_array_reads_field_compiles() {
        // `.iter().filter(|u| u.score > 80)` — the type-preserving `iter`/
        // `filter` adapter chain must thread the `User` element identity to the
        // closure param. Pre-fix the name-based `.iter()` fallback yielded the
        // lossy `Vec<object>` head-name which was rejected, so `u` stayed
        // unannotated and `u.score > 80` surfaced "operand types are `unknown`
        // and `int`".
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             let hi = users.iter().filter(|u| u.score > 80).collect()\n\
             print(hi.len())"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            ".iter().filter over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn iter_find_struct_array_reads_field_compiles() {
        // `.iter().find(|u| u.score > 80)` — same iterator-adapter element
        // identity threading as the filter case.
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             let f = users.iter().find(|u| u.score > 80)\n\
             print(f)"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            ".iter().find over Array<User> reading u.score should compile"
        );
    }

    #[test]
    fn filter_then_map_struct_field_chain_compiles() {
        // The full R3 chain: `users.filter(|u| u.score > 85).map(|u| u.score)`
        // — struct identity must survive the filter's result element type AND
        // flow into the second closure's `u.score`. (Int-returning map; the
        // String-returning `.map(|u| u.name)` carrier is the separate, pre-
        // existing `Array.select: closure-return kind String` J.5d limitation.)
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}, User {{ name: \"b\", score: 50 }}]\n\
             let top = users.filter(|u| u.score > 85).map(|u| u.score)\n\
             print(top.len())"
        );
        assert!(
            compile_with_prelude(&src).is_ok(),
            "filter(...).map(...) chain over Array<User> should compile"
        );
    }

    #[test]
    fn nonexistent_field_in_iter_filter_closure_rejects() {
        // Soundness guard for the `.iter()` path: the struct identity now flows
        // through the adapter chain, so a non-existent field still rejects.
        let src = format!(
            "{USER_TYPE}\
             let users = [User {{ name: \"a\", score: 90 }}]\n\
             let bad = users.iter().filter(|u| u.nonexistent > 5).collect()\n\
             print(bad.len())"
        );
        let res = compile_with_prelude(&src);
        assert!(
            res.is_err(),
            "a non-existent struct field inside the .iter().filter closure must reject"
        );
        assert!(
            format!("{:?}", res.unwrap_err()).contains("nonexistent"),
            "rejection should name the missing field"
        );
    }
}
