//! Type → ConcreteType conversion for the v2 monomorphization pipeline.
//!
//! After type inference completes, every variable, parameter, field, and
//! expression must have a fully resolved [`ConcreteType`] — no unresolved
//! type variables, no generics. This module is the bridge between the
//! inference engine's [`Type`] / [`TypeAnnotation`] representation and the
//! [`ConcreteType`] used by the bytecode compiler, VM, and JIT.
//!
//! See `docs/v2-monomorphization-design.md` Phase 1.1 for the rationale.
//!
//! ## Entry points
//!
//! - [`type_to_concrete`] — convert a [`Type`] using a [`Unifier`] to resolve
//!   any [`Type::Variable`] via the substitution map. Returns an error if a
//!   type variable cannot be resolved (no escape hatch).
//! - [`annotation_to_concrete`] — convert a [`TypeAnnotation`] directly. Used
//!   by call sites that already have an annotation rather than a `Type`.
//!
//! Both functions return `Result<ConcreteType, ShapeError>` so callers can
//! propagate descriptive errors when conversion fails.

use shape_ast::ast::TypeAnnotation;
use shape_ast::error::ShapeError;
use shape_value::v2::{ConcreteType, StructLayoutId};
use shape_value::ValueWordExt;

use super::types::Type;
use super::unification::Unifier;

/// Convert an inference [`Type`] to a fully resolved [`ConcreteType`].
///
/// `unifier` provides the substitution map produced by type inference. Any
/// [`Type::Variable`] is resolved by walking the substitution chain. If a
/// variable remains unresolved (i.e. inference failed to determine a concrete
/// type), an error is returned — there is no `Any` escape hatch in v2.
pub fn type_to_concrete(ty: &Type, unifier: &Unifier) -> Result<ConcreteType, ShapeError> {
    // Always apply substitutions first so we never see a stale `Type::Variable`
    // that the unifier has already bound to a concrete shape.
    let resolved = unifier.apply_substitutions(ty);
    type_to_concrete_inner(&resolved, unifier)
}

fn type_to_concrete_inner(ty: &Type, unifier: &Unifier) -> Result<ConcreteType, ShapeError> {
    match ty {
        Type::Concrete(ann) => annotation_to_concrete(ann),

        Type::Variable(var) => {
            // After apply_substitutions, the variable was not bound to anything.
            Err(ShapeError::TypeError(format!(
                "[B0015] cannot convert unresolved type variable `{}` to ConcreteType: \
                 inference must produce a concrete type for every local in v2",
                var.0
            )))
        }

        Type::Generic { base, args } => generic_to_concrete(base, args, unifier),

        Type::Constrained { var, .. } => {
            // Constrained vars are still type variables — they must be resolved.
            Err(ShapeError::TypeError(format!(
                "[B0015] cannot convert constrained type variable `{}` to ConcreteType: \
                 inference must resolve trait-bounded variables before lowering",
                var.0
            )))
        }

        Type::Function { .. } => {
            // For now we use a placeholder FunctionTypeId(0). Agent 3 / monomorphization
            // will register concrete signatures and assign stable IDs.
            Ok(ConcreteType::Function(
                shape_value::v2::FunctionTypeId(0),
            ))
        }
    }
}

/// Convert a [`Type::Generic`] base + args to a [`ConcreteType`].
fn generic_to_concrete(
    base: &Type,
    args: &[Type],
    unifier: &Unifier,
) -> Result<ConcreteType, ShapeError> {
    // Resolve the base type name from a Concrete(Reference) or Concrete(Basic).
    let base_name = match base {
        Type::Concrete(TypeAnnotation::Reference(path)) => path.to_string(),
        Type::Concrete(TypeAnnotation::Basic(name)) => name.clone(),
        _ => {
            return Err(ShapeError::TypeError(format!(
                "[B0015] cannot convert generic type with non-name base `{:?}` to ConcreteType",
                base
            )));
        }
    };

    match base_name.as_str() {
        "Array" | "Vec" => {
            let elem = args.first().ok_or_else(|| {
                ShapeError::TypeError(
                    "[B0015] Array/Vec generic missing element type argument".to_string(),
                )
            })?;
            let elem_concrete = type_to_concrete(elem, unifier)?;
            Ok(ConcreteType::Array(Box::new(elem_concrete)))
        }
        "HashMap" | "Map" => {
            if args.len() < 2 {
                return Err(ShapeError::TypeError(format!(
                    "[B0015] HashMap generic requires 2 type args, got {}",
                    args.len()
                )));
            }
            let k = type_to_concrete(&args[0], unifier)?;
            let v = type_to_concrete(&args[1], unifier)?;
            Ok(ConcreteType::HashMap(Box::new(k), Box::new(v)))
        }
        "Option" => {
            let inner = args.first().ok_or_else(|| {
                ShapeError::TypeError(
                    "[B0015] Option generic missing inner type argument".to_string(),
                )
            })?;
            let inner_concrete = type_to_concrete(inner, unifier)?;
            Ok(ConcreteType::Option(Box::new(inner_concrete)))
        }
        "Result" => {
            let ok = args.first().ok_or_else(|| {
                ShapeError::TypeError(
                    "[B0015] Result generic missing Ok type argument".to_string(),
                )
            })?;
            let ok_concrete = type_to_concrete(ok, unifier)?;
            // Err defaults to a generic string error if not specified.
            let err_concrete = if args.len() >= 2 {
                type_to_concrete(&args[1], unifier)?
            } else {
                ConcreteType::String
            };
            Ok(ConcreteType::Result(
                Box::new(ok_concrete),
                Box::new(err_concrete),
            ))
        }
        _ => {
            // Unknown generic — treat as a struct reference. Agent 3 will
            // assign a real StructLayoutId during monomorphization.
            Ok(ConcreteType::Struct(StructLayoutId(0)))
        }
    }
}

/// Convert an AST [`TypeAnnotation`] to a [`ConcreteType`].
///
/// This is the simpler entry point used when the bytecode compiler already
/// has a syntactic annotation (e.g. from `let x: int = 0`) and does not need
/// to consult the inference unifier.
pub fn annotation_to_concrete(ann: &TypeAnnotation) -> Result<ConcreteType, ShapeError> {
    match ann {
        TypeAnnotation::Basic(name) => basic_name_to_concrete(name),

        TypeAnnotation::Reference(path) => {
            let name = path.to_string();
            // A reference may name a primitive (legacy paths sometimes route
            // primitive names through Reference) or a struct/enum/etc.
            if let Ok(ct) = basic_name_to_concrete(&name) {
                return Ok(ct);
            }
            // Special-case the well-known wrapper-shaped types that may appear
            // as bare references with no type args.
            match name.as_str() {
                "Decimal" | "decimal" => Ok(ConcreteType::Decimal),
                "BigInt" | "bigint" => Ok(ConcreteType::BigInt),
                "DateTime" | "datetime" => Ok(ConcreteType::DateTime),
                "String" | "string" => Ok(ConcreteType::String),
                _ => {
                    // Unknown named reference — treat as struct. Agent 3 will
                    // assign a real StructLayoutId during monomorphization.
                    Ok(ConcreteType::Struct(StructLayoutId(0)))
                }
            }
        }

        TypeAnnotation::Array(elem) => {
            let elem_ct = annotation_to_concrete(elem)?;
            Ok(ConcreteType::Array(Box::new(elem_ct)))
        }

        TypeAnnotation::Tuple(elems) => {
            let mut out = Vec::with_capacity(elems.len());
            for e in elems {
                out.push(annotation_to_concrete(e)?);
            }
            Ok(ConcreteType::Tuple(out))
        }

        TypeAnnotation::Generic { name, args } => {
            let name_str = name.to_string();
            match name_str.as_str() {
                "Array" | "Vec" => {
                    let elem = args.first().ok_or_else(|| {
                        ShapeError::TypeError(
                            "[B0015] Array/Vec annotation missing element type".to_string(),
                        )
                    })?;
                    Ok(ConcreteType::Array(Box::new(annotation_to_concrete(elem)?)))
                }
                "HashMap" | "Map" => {
                    if args.len() < 2 {
                        return Err(ShapeError::TypeError(format!(
                            "[B0015] HashMap annotation requires 2 type args, got {}",
                            args.len()
                        )));
                    }
                    Ok(ConcreteType::HashMap(
                        Box::new(annotation_to_concrete(&args[0])?),
                        Box::new(annotation_to_concrete(&args[1])?),
                    ))
                }
                "Option" => {
                    let inner = args.first().ok_or_else(|| {
                        ShapeError::TypeError(
                            "[B0015] Option annotation missing inner type".to_string(),
                        )
                    })?;
                    Ok(ConcreteType::Option(Box::new(annotation_to_concrete(inner)?)))
                }
                "Result" => {
                    let ok = args.first().ok_or_else(|| {
                        ShapeError::TypeError(
                            "[B0015] Result annotation missing Ok type".to_string(),
                        )
                    })?;
                    let ok_ct = annotation_to_concrete(ok)?;
                    let err_ct = if args.len() >= 2 {
                        annotation_to_concrete(&args[1])?
                    } else {
                        ConcreteType::String
                    };
                    Ok(ConcreteType::Result(Box::new(ok_ct), Box::new(err_ct)))
                }
                _ => {
                    // Unknown named generic — treat as struct.
                    Ok(ConcreteType::Struct(StructLayoutId(0)))
                }
            }
        }

        TypeAnnotation::Function { .. } => Ok(ConcreteType::Function(
            shape_value::v2::FunctionTypeId(0),
        )),

        TypeAnnotation::Object(_) => {
            // Anonymous object literals lower to a struct layout. Agent 3
            // will compute and assign the actual StructLayoutId.
            Ok(ConcreteType::Struct(StructLayoutId(0)))
        }

        TypeAnnotation::Void => Ok(ConcreteType::Void),
        TypeAnnotation::Never => Ok(ConcreteType::Void),
        TypeAnnotation::Null => Ok(ConcreteType::Option(Box::new(ConcreteType::Void))),
        TypeAnnotation::Undefined => Ok(ConcreteType::Option(Box::new(ConcreteType::Void))),

        TypeAnnotation::Union(_) | TypeAnnotation::Intersection(_) | TypeAnnotation::Dyn(_) => {
            Err(ShapeError::TypeError(format!(
                "[B0015] cannot convert {:?} to a single ConcreteType — \
                 monomorphization requires a single concrete type per slot",
                ann
            )))
        }
    }
}

/// Map a basic primitive type name (e.g. `"int"`, `"number"`, `"i32"`) to a
/// [`ConcreteType`]. Returns an error for non-primitive names.
fn basic_name_to_concrete(name: &str) -> Result<ConcreteType, ShapeError> {
    // Default scripting aliases first.
    match name {
        "number" | "Number" | "float" | "Float" | "f64" => return Ok(ConcreteType::F64),
        "int" | "Int" | "integer" | "Integer" | "i64" => return Ok(ConcreteType::I64),
        "i32" => return Ok(ConcreteType::I32),
        "i16" => return Ok(ConcreteType::I16),
        "i8" | "char" => return Ok(ConcreteType::I8),
        "u64" => return Ok(ConcreteType::U64),
        "u32" => return Ok(ConcreteType::U32),
        "u16" => return Ok(ConcreteType::U16),
        "u8" | "byte" => return Ok(ConcreteType::U8),
        "bool" | "Bool" | "boolean" | "Boolean" => return Ok(ConcreteType::Bool),
        "string" | "String" => return Ok(ConcreteType::String),
        "decimal" | "Decimal" => return Ok(ConcreteType::Decimal),
        "bigint" | "BigInt" => return Ok(ConcreteType::BigInt),
        "datetime" | "DateTime" => return Ok(ConcreteType::DateTime),
        "void" => return Ok(ConcreteType::Void),
        // f32 has no separate ConcreteType today; treat as F64.
        "f32" => return Ok(ConcreteType::F64),
        // isize/usize collapse to i64/u64 on 64-bit hosts; we pick i64/u64.
        "isize" => return Ok(ConcreteType::I64),
        "usize" => return Ok(ConcreteType::U64),
        _ => {}
    }

    Err(ShapeError::TypeError(format!(
        "[B0015] basic type name `{}` does not map to a primitive ConcreteType",
        name
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_system::{BuiltinTypes, Type, TypeVar};
    use shape_ast::ast::type_path::TypePath;
    use shape_ast::ast::TypeAnnotation;
    use shape_value::v2::ConcreteType;

    fn unifier() -> Unifier {
        Unifier::new()
    }

    // ----- Primitive type → ConcreteType -----

    #[test]
    fn number_maps_to_f64() {
        let ty = BuiltinTypes::number();
        let ct = type_to_concrete(&ty, &unifier()).unwrap();
        assert_eq!(ct, ConcreteType::F64);
    }

    #[test]
    fn integer_maps_to_i64() {
        let ty = BuiltinTypes::integer();
        let ct = type_to_concrete(&ty, &unifier()).unwrap();
        assert_eq!(ct, ConcreteType::I64);
    }

    #[test]
    fn boolean_maps_to_bool() {
        let ty = BuiltinTypes::boolean();
        let ct = type_to_concrete(&ty, &unifier()).unwrap();
        assert_eq!(ct, ConcreteType::Bool);
    }

    #[test]
    fn string_maps_to_string() {
        let ty = BuiltinTypes::string();
        let ct = type_to_concrete(&ty, &unifier()).unwrap();
        assert_eq!(ct, ConcreteType::String);
    }

    #[test]
    fn width_specific_integers() {
        for (name, expected) in [
            ("i8", ConcreteType::I8),
            ("u8", ConcreteType::U8),
            ("i16", ConcreteType::I16),
            ("u16", ConcreteType::U16),
            ("i32", ConcreteType::I32),
            ("u32", ConcreteType::U32),
            ("i64", ConcreteType::I64),
            ("u64", ConcreteType::U64),
        ] {
            let ann = TypeAnnotation::Basic(name.to_string());
            let ct = annotation_to_concrete(&ann).unwrap();
            assert_eq!(ct, expected, "name = {}", name);
        }
    }

    // ----- Composites -----

    #[test]
    fn array_of_number_maps_to_array_f64() {
        let ann = TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("number".into())));
        let ct = annotation_to_concrete(&ann).unwrap();
        assert_eq!(ct, ConcreteType::Array(Box::new(ConcreteType::F64)));
    }

    #[test]
    fn array_of_int_via_type_to_concrete() {
        let ty = Type::Concrete(TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
            "int".into(),
        ))));
        let ct = type_to_concrete(&ty, &unifier()).unwrap();
        assert_eq!(ct, ConcreteType::Array(Box::new(ConcreteType::I64)));
    }

    #[test]
    fn hashmap_string_int_via_annotation() {
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("HashMap"),
            args: vec![
                TypeAnnotation::Basic("string".into()),
                TypeAnnotation::Basic("int".into()),
            ],
        };
        let ct = annotation_to_concrete(&ann).unwrap();
        assert_eq!(
            ct,
            ConcreteType::HashMap(
                Box::new(ConcreteType::String),
                Box::new(ConcreteType::I64)
            )
        );
    }

    #[test]
    fn hashmap_string_int_via_generic_type() {
        // Type::Generic { base: Reference("HashMap"), args: [string, int] }
        let ty = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                TypePath::simple("HashMap"),
            ))),
            args: vec![BuiltinTypes::string(), BuiltinTypes::integer()],
        };
        let ct = type_to_concrete(&ty, &unifier()).unwrap();
        assert_eq!(
            ct,
            ConcreteType::HashMap(
                Box::new(ConcreteType::String),
                Box::new(ConcreteType::I64)
            )
        );
    }

    #[test]
    fn nested_array_of_array_of_int() {
        // Array<Array<int>>
        let ann = TypeAnnotation::Array(Box::new(TypeAnnotation::Array(Box::new(
            TypeAnnotation::Basic("int".into()),
        ))));
        let ct = annotation_to_concrete(&ann).unwrap();
        assert_eq!(
            ct,
            ConcreteType::Array(Box::new(ConcreteType::Array(Box::new(ConcreteType::I64))))
        );
    }

    #[test]
    fn option_int_via_annotation() {
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Option"),
            args: vec![TypeAnnotation::Basic("int".into())],
        };
        let ct = annotation_to_concrete(&ann).unwrap();
        assert_eq!(ct, ConcreteType::Option(Box::new(ConcreteType::I64)));
    }

    #[test]
    fn option_int_via_generic_type() {
        let ty = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                TypePath::simple("Option"),
            ))),
            args: vec![BuiltinTypes::integer()],
        };
        let ct = type_to_concrete(&ty, &unifier()).unwrap();
        assert_eq!(ct, ConcreteType::Option(Box::new(ConcreteType::I64)));
    }

    #[test]
    fn result_int_string_via_annotation() {
        let ann = TypeAnnotation::Generic {
            name: TypePath::simple("Result"),
            args: vec![
                TypeAnnotation::Basic("int".into()),
                TypeAnnotation::Basic("string".into()),
            ],
        };
        let ct = annotation_to_concrete(&ann).unwrap();
        assert_eq!(
            ct,
            ConcreteType::Result(Box::new(ConcreteType::I64), Box::new(ConcreteType::String))
        );
    }

    #[test]
    fn tuple_int_number_bool() {
        let ann = TypeAnnotation::Tuple(vec![
            TypeAnnotation::Basic("int".into()),
            TypeAnnotation::Basic("number".into()),
            TypeAnnotation::Basic("bool".into()),
        ]);
        let ct = annotation_to_concrete(&ann).unwrap();
        assert_eq!(
            ct,
            ConcreteType::Tuple(vec![
                ConcreteType::I64,
                ConcreteType::F64,
                ConcreteType::Bool,
            ])
        );
    }

    // ----- Variable resolution via unifier -----

    #[test]
    fn variable_resolved_via_unifier() {
        let var = TypeVar::new("T_test_0".to_string());
        let mut u = Unifier::new();
        u.bind(var.clone(), BuiltinTypes::number());
        let ty = Type::Variable(var);
        let ct = type_to_concrete(&ty, &u).unwrap();
        assert_eq!(ct, ConcreteType::F64);
    }

    #[test]
    fn unresolved_variable_errors() {
        let var = TypeVar::new("T_unresolved".to_string());
        let ty = Type::Variable(var);
        let err = type_to_concrete(&ty, &unifier()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("T_unresolved"),
            "error should name the variable: {}",
            msg
        );
    }

    #[test]
    fn variable_inside_array_resolved_via_unifier() {
        // Type::Generic { base: Array, args: [Type::Variable] } where the
        // variable is bound to int via the unifier.
        let var = TypeVar::new("T_elem".to_string());
        let mut u = Unifier::new();
        u.bind(var.clone(), BuiltinTypes::integer());
        let ty = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                TypePath::simple("Array"),
            ))),
            args: vec![Type::Variable(var)],
        };
        let ct = type_to_concrete(&ty, &u).unwrap();
        assert_eq!(ct, ConcreteType::Array(Box::new(ConcreteType::I64)));
    }

    // ----- Edge cases -----

    #[test]
    fn void_maps_to_void() {
        let ann = TypeAnnotation::Void;
        let ct = annotation_to_concrete(&ann).unwrap();
        assert_eq!(ct, ConcreteType::Void);
    }

    #[test]
    fn unknown_reference_falls_back_to_struct_placeholder() {
        let ann = TypeAnnotation::Reference(TypePath::simple("MyCustomType"));
        let ct = annotation_to_concrete(&ann).unwrap();
        assert!(matches!(ct, ConcreteType::Struct(_)));
    }

    #[test]
    fn union_type_errors() {
        let ann = TypeAnnotation::Union(vec![
            TypeAnnotation::Basic("int".into()),
            TypeAnnotation::Basic("string".into()),
        ]);
        let err = annotation_to_concrete(&ann).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ConcreteType"), "error: {}", msg);
    }
}
