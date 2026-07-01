//! Type operation expression compilation

use crate::bytecode::{Constant, Instruction, NumericWidth, OpCode, Operand};
use shape_ast::ast::{Expr, Spanned, TypeAnnotation};
use shape_ast::error::{Result, ShapeError};
use shape_runtime::type_system::unification::annotations_equal;
use shape_runtime::type_system::{BuiltinTypes, Type, annotation_to_string};
use std::collections::HashSet;

use super::super::BytecodeCompiler;

const INTO_DISPATCH_TAG: &str = "__IntoDispatch";
const TRY_INTO_DISPATCH_TAG: &str = "__TryIntoDispatch";

/// How an `as`/`as?` cast should be compiled after validation.
#[derive(Debug)]
enum CastLiftKind {
    /// Source type has a direct Into/TryInto impl for the target.
    Direct,
    /// Source is `Option<T>` — needs null-check lifting.
    LiftOption,
    /// Source is `Result<T, E>` — needs Ok/Err lifting.
    LiftResult,
}

fn type_name_to_annotation(name: &str) -> TypeAnnotation {
    match name {
        "number" | "int" | "decimal" | "string" | "bool" | "row" | "pattern" | "function"
        | "module_function" | "duration" | "datetime" | "time" | "timeframe" | "table"
        | "array" | "object" | "option" | "result" | "Type" | "type" | "i8" | "u8" | "i16"
        | "u16" | "i32" | "u32" | "i64" | "u64" | "isize" | "usize" | "byte" | "char" => {
            TypeAnnotation::Basic(name.to_string())
        }
        "()" | "unit" => TypeAnnotation::Void,
        "None" => TypeAnnotation::Null,
        _ => TypeAnnotation::Reference(name.into()),
    }
}

impl BytecodeCompiler {
    fn current_function_type_params(&self) -> HashSet<String> {
        let Some(func_idx) = self.current_function else {
            return HashSet::new();
        };
        let Some(func) = self.program.functions.get(func_idx) else {
            return HashSet::new();
        };
        let Some(func_def) = self.function_defs.get(&func.name) else {
            return HashSet::new();
        };

        func_def
            .type_params
            .as_ref()
            .map(|params| params.iter().map(|p| p.name().to_string()).collect())
            .unwrap_or_default()
    }

    fn annotation_contains_type_param(ann: &TypeAnnotation, type_params: &HashSet<String>) -> bool {
        match ann {
            TypeAnnotation::Basic(name) => type_params.contains(name),
            TypeAnnotation::Reference(name) => type_params.contains(name.as_str()),
            TypeAnnotation::Borrow { inner, .. } => {
                Self::annotation_contains_type_param(inner, type_params)
            }
            TypeAnnotation::Array(inner) => {
                Self::annotation_contains_type_param(inner, type_params)
            }
            TypeAnnotation::Tuple(items)
            | TypeAnnotation::Union(items)
            | TypeAnnotation::Intersection(items) => items
                .iter()
                .any(|item| Self::annotation_contains_type_param(item, type_params)),
            TypeAnnotation::Object(fields) => fields.iter().any(|field| {
                Self::annotation_contains_type_param(&field.type_annotation, type_params)
            }),
            TypeAnnotation::Function { params, returns } => {
                params
                    .iter()
                    .any(|p| Self::annotation_contains_type_param(&p.type_annotation, type_params))
                    || Self::annotation_contains_type_param(returns, type_params)
            }
            TypeAnnotation::Generic { name, args } => {
                type_params.contains(name.as_str())
                    || args
                        .iter()
                        .any(|arg| Self::annotation_contains_type_param(arg, type_params))
            }
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined
            | TypeAnnotation::Dyn(_) => false,
        }
    }

    pub(super) fn should_runtime_type_query(&self, ann: &TypeAnnotation) -> bool {
        let type_params = self.current_function_type_params();
        if type_params.is_empty() {
            return false;
        }
        Self::annotation_contains_type_param(ann, &type_params)
    }

    pub(super) fn is_type_symbol_name(&self, name: &str) -> bool {
        self.struct_types.contains_key(name)
            || self.type_aliases.contains_key(name)
            || self.type_inference.env.lookup_type_alias(name).is_some()
            || self.type_inference.env.get_enum(name).is_some()
            || self.type_inference.env.lookup_trait(name).is_some()
    }

    fn canonical_try_into_name(name: &str) -> String {
        match name {
            "boolean" | "Boolean" | "Bool" => "bool".to_string(),
            "String" => "string".to_string(),
            "Number" => "number".to_string(),
            "Int" => "int".to_string(),
            "Decimal" => "decimal".to_string(),
            _ => name.to_string(),
        }
    }

    fn try_into_name_from_annotation(annotation: &TypeAnnotation) -> Option<String> {
        match annotation {
            TypeAnnotation::Basic(name) => Some(Self::canonical_try_into_name(name)),
            TypeAnnotation::Reference(name) => Some(Self::canonical_try_into_name(name)),
            TypeAnnotation::Generic { name, .. } => Some(Self::canonical_try_into_name(name)),
            _ => None,
        }
    }

    fn try_into_name_from_type(ty: &Type) -> Option<String> {
        match ty {
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                Some(Self::canonical_try_into_name(name))
            }
            Type::Concrete(TypeAnnotation::Reference(name)) => {
                Some(Self::canonical_try_into_name(name))
            }
            Type::Concrete(TypeAnnotation::Generic { name, .. }) => {
                Some(Self::canonical_try_into_name(name))
            }
            Type::Generic { base, .. } => match base.as_ref() {
                Type::Concrete(TypeAnnotation::Basic(name)) => {
                    Some(Self::canonical_try_into_name(name))
                }
                Type::Concrete(TypeAnnotation::Reference(name)) => {
                    Some(Self::canonical_try_into_name(name))
                }
                Type::Concrete(TypeAnnotation::Generic { name, .. }) => {
                    Some(Self::canonical_try_into_name(name))
                }
                _ => None,
            },
            _ => None,
        }
    }

    fn conversion_dispatch_annotation(
        tag: &str,
        source_name: &str,
        target_selector: &str,
    ) -> TypeAnnotation {
        TypeAnnotation::Generic {
            name: tag.into(),
            args: vec![
                TypeAnnotation::Reference(source_name.into()),
                TypeAnnotation::Reference(target_selector.into()),
            ],
        }
    }

    fn annotation_matches_union_member(source: &TypeAnnotation, member: &TypeAnnotation) -> bool {
        if annotations_equal(source, member) {
            return true;
        }

        match (source, member) {
            (TypeAnnotation::Basic(source), TypeAnnotation::Reference(member))
            | (TypeAnnotation::Reference(member), TypeAnnotation::Basic(source)) => {
                Self::canonical_try_into_name(source)
                    == Self::canonical_try_into_name(member.as_str())
            }
            (
                TypeAnnotation::Generic {
                    name: source_name,
                    args: source_args,
                },
                TypeAnnotation::Generic {
                    name: member_name,
                    args: member_args,
                },
            ) => {
                Self::canonical_try_into_name(source_name)
                    == Self::canonical_try_into_name(member_name)
                    && source_args.len() == member_args.len()
                    && source_args
                        .iter()
                        .zip(member_args.iter())
                        .all(|(source_arg, member_arg)| {
                            Self::annotation_matches_union_member(source_arg, member_arg)
                        })
            }
            (TypeAnnotation::Array(source_inner), TypeAnnotation::Array(member_inner)) => {
                Self::annotation_matches_union_member(source_inner, member_inner)
            }
            _ => false,
        }
    }

    fn source_annotation_fits_union(
        source: &TypeAnnotation,
        union_members: &[TypeAnnotation],
    ) -> bool {
        match source {
            TypeAnnotation::Union(source_members) => source_members.iter().all(|source_member| {
                union_members.iter().any(|union_member| {
                    Self::annotation_matches_union_member(source_member, union_member)
                })
            }),
            _ => union_members
                .iter()
                .any(|member| Self::annotation_matches_union_member(source, member)),
        }
    }

    /// Extract the inner type `T` from an `Option<T>` annotation.
    fn unwrap_option_inner(ann: &TypeAnnotation) -> Option<&TypeAnnotation> {
        match ann {
            TypeAnnotation::Generic { name, args } if name == "Option" && args.len() == 1 => {
                Some(&args[0])
            }
            _ => None,
        }
    }

    /// Extract the inner type `T` from a `Result<T, E>` annotation.
    fn unwrap_result_inner(ann: &TypeAnnotation) -> Option<&TypeAnnotation> {
        match ann {
            TypeAnnotation::Generic { name, args } if name == "Result" && !args.is_empty() => {
                Some(&args[0])
            }
            _ => None,
        }
    }

    /// Try to resolve a full type annotation for the source expression.
    ///
    /// Unlike `try_into_name_from_annotation` (which only extracts the base
    /// name), this preserves the full generic structure so we can detect
    /// `Option<T>` and `Result<T, E>` wrappers.
    fn resolve_source_annotation(&mut self, expr: &Expr) -> Option<TypeAnnotation> {
        // Try static annotation first
        if let Ok(ann) = self.static_type_annotation_for_expr(expr) {
            // The type tracker may return a bare `Basic("option")` /
            // `Basic("result")` that has lost its generic args.  For wrapper
            // types we need the full generic structure, so try type inference.
            let is_bare_wrapper = matches!(
                &ann,
                TypeAnnotation::Basic(n) if n == "option" || n == "result"
            ) || matches!(
                &ann,
                TypeAnnotation::Reference(n) if n.as_str() == "Option" || n.as_str() == "Result"
            );
            if !is_bare_wrapper {
                return Some(ann);
            }
            // Try inference for the full generic structure
            if let Ok(ty) = self.infer_expr_type(expr) {
                if let Some(inferred_ann) = ty.to_annotation() {
                    return Some(inferred_ann);
                }
            }
            // Inference failed — return the bare wrapper annotation so lifting
            // can still be detected (inner type validated at runtime)
            return Some(ann);
        }
        // Fall back to inferred type → annotation (preserves generics)
        if let Ok(ty) = self.infer_expr_type(expr) {
            return ty.to_annotation();
        }
        None
    }

    /// Check whether any Into/TryInto impls are registered in the trait registry.
    fn has_any_conversion_impls(&self) -> bool {
        self.type_inference.env.has_any_into_impls()
    }

    /// Check whether `source` has `Into<target>` impl.
    fn has_into_impl(&self, source: &str, target: &str) -> bool {
        self.type_inference
            .env
            .lookup_trait_impl_named("Into", source, target)
            .is_some()
            || self
                .type_inference
                .env
                .lookup_trait_impl("Into", source)
                .is_some()
    }

    /// Check whether `source` has `TryInto<target>` impl.
    fn has_try_into_impl(&self, source: &str, target: &str) -> bool {
        self.type_inference
            .env
            .lookup_trait_impl_named("TryInto", source, target)
            .is_some()
            || self
                .type_inference
                .env
                .lookup_trait_impl("TryInto", source)
                .is_some()
    }

    /// PB4 sub-root #6: route `expr as Target` / `expr as Target?` to a
    /// user-registered `Into`/`TryInto` impl method when one is in scope.
    ///
    /// Returns the function index of the impl's `into`/`tryInto` method
    /// when dispatch should route through it, or `None` to fall back to
    /// the primitive `ConvertTo*` / `TryConvertTo*` opcode path.
    ///
    /// Guards:
    /// - Identity casts (`source == target`) stay on the primitive path
    ///   (no user method to call; primitive is a typed no-op cast).
    /// - **Recursion guard:** when `current_function` is the very impl
    ///   method being compiled, dispatch falls through to the primitive
    ///   opcode. This is how stdlib impl bodies such as
    ///   `impl TryInto<int> for string as int { method tryInto() { self as int? } }`
    ///   (`crates/shape-runtime/stdlib-src/core/try_into.shape:63-65`)
    ///   reach the primitive `TryConvertToInt` opcode without infinite-
    ///   recursing through `lookup_trait_method_symbol`.
    ///
    /// The trait registry's `register_trait_method_symbol` overwrites on
    /// duplicate keys, so a user fixture that redeclares the stdlib
    /// `TryInto<int> for string as int` impl shadows the stdlib body —
    /// the user's `tryInto` function index is what this helper returns,
    /// and the user's `Ok(7)` body fires at the cast site.
    fn user_impl_method_for_cast(
        &self,
        source_name: &str,
        target_selector: &str,
        fallible: bool,
    ) -> Option<usize> {
        if source_name == target_selector {
            return None;
        }
        let (trait_name, method_name) = if fallible {
            ("TryInto", "tryInto")
        } else {
            ("Into", "into")
        };
        let func_name = self
            .program
            .lookup_trait_method_symbol(trait_name, source_name, Some(target_selector), method_name)
            .map(|s| s.to_string())?;
        let func_idx = self.find_function(&func_name)?;
        if self.current_function == Some(func_idx) {
            return None;
        }
        Some(func_idx)
    }

    /// Emit a single-arg `Call` to a user-impl `into`/`tryInto` method.
    ///
    /// The source expression (`self`) must already be on the stack.
    /// Mirrors `emit_call_ok`'s push-arg-count-then-Call pattern.
    fn emit_user_impl_cast_call(&mut self, func_idx: usize) {
        let arg_count = self.program.add_constant(Constant::Int(1));
        self.emit(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(arg_count)),
        ));
        self.emit(Instruction::new(
            OpCode::Call,
            Some(Operand::Function(shape_value::FunctionId(func_idx as u16))),
        ));
    }

    /// Resolve the source-expression's type-name for user-impl lookup.
    /// Returns `None` when the source type cannot be reduced to a single
    /// canonical type-name (e.g. unresolved type variable).
    fn cast_source_name(&mut self, expr: &Expr) -> Option<String> {
        self.static_type_annotation_for_expr(expr)
            .ok()
            .and_then(|ann| Self::try_into_name_from_annotation(&ann))
            .or_else(|| {
                self.infer_expr_type(expr)
                    .ok()
                    .and_then(|ty| Self::try_into_name_from_type(&ty))
            })
    }

    /// Validate an infallible `as Type` cast and determine the compilation
    /// strategy: direct conversion, Option lifting, or Result lifting.
    ///
    /// Returns `Err` for invalid casts (no Into impl). Returns `Ok(None)` when
    /// validation is skipped (test mode, unresolvable source). Returns
    /// `Ok(Some(strategy))` on success.
    fn validate_infallible_cast(
        &mut self,
        expr: &Expr,
        target_name: &str,
    ) -> Result<Option<CastLiftKind>> {
        // Guard: skip validation when no Into impls are registered (test mode)
        if !self.has_any_conversion_impls() {
            return Ok(None);
        }

        let source_ann = match self.resolve_source_annotation(expr) {
            Some(ann) => ann,
            None => return Ok(None), // can't resolve → conservative skip
        };

        let source_name = match Self::try_into_name_from_annotation(&source_ann) {
            Some(n) => n,
            None => return Ok(None),
        };

        // Identity cast: always valid
        if source_name == target_name {
            return Ok(Some(CastLiftKind::Direct));
        }

        // D1 (numeric-conversion GREEN Stage 1) — primitive numeric cast.
        // A cast between two primitive numeric types (`int`/`i64`, the width
        // names, `number`/`f32`, `decimal`) is a BUILT-IN cast governed by
        // the numeric lattice (spec §3 / §8), lowering to the already-existing
        // typed `ConvertToInt`/`ConvertToNumber`/`ConvertToDecimal` opcodes via
        // `convert_opcode_for_primitive` at the caller. It must NOT require a
        // user `Into<Target>` impl: the prelude declares no `Into` for
        // width-typed sources (`i32 as number`) nor for the lossy
        // `number as int` direction. This mirrors the inference-side gate
        // (`type_system/inference/expressions.rs` `Expr::TypeAssertion`) and
        // the `CastWidth` width-cast bypass above. Direct (non-lift) so the
        // result kind is re-stamped via `record_cast_result_kind`. No runtime
        // coercion is introduced — the implicit-conversion paths are governed
        // separately; this only accepts the EXPLICIT `as` cast.
        if BuiltinTypes::is_numeric_type_name(&source_name)
            && BuiltinTypes::is_numeric_type_name(target_name)
        {
            return Ok(Some(CastLiftKind::Direct));
        }

        // Direct impl check
        if self.has_into_impl(&source_name, target_name) {
            return Ok(Some(CastLiftKind::Direct));
        }

        // Option<T> as M → lift if T has Into<M>
        if let Some(inner_ann) = Self::unwrap_option_inner(&source_ann) {
            if let Some(inner_name) = Self::try_into_name_from_annotation(inner_ann) {
                if inner_name == target_name
                    || Self::is_builtin_primitive_numeric_cast(&inner_name, target_name)
                    || self.has_into_impl(&inner_name, target_name)
                {
                    return Ok(Some(CastLiftKind::LiftOption));
                }
            }
            // Inner type has no Into impl → compile error
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Cannot convert `Option<{}>` to `{}`: inner type has no `Into<{}>` implementation",
                    annotation_to_string(inner_ann),
                    target_name,
                    target_name,
                ),
                location: Some(self.span_to_source_location(expr.span())),
            });
        }

        // Result<T, E> as M → lift if T has Into<M>
        if let Some(inner_ann) = Self::unwrap_result_inner(&source_ann) {
            if let Some(inner_name) = Self::try_into_name_from_annotation(inner_ann) {
                if inner_name == target_name
                    || Self::is_builtin_primitive_numeric_cast(&inner_name, target_name)
                    || self.has_into_impl(&inner_name, target_name)
                {
                    return Ok(Some(CastLiftKind::LiftResult));
                }
            }
            // Inner type has no Into impl → compile error
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Cannot convert `Result<{}, _>` to `{}`: inner type has no `Into<{}>` implementation",
                    annotation_to_string(inner_ann),
                    target_name,
                    target_name,
                ),
                location: Some(self.span_to_source_location(expr.span())),
            });
        }

        // Bare wrapper name without generic args (type tracker lost generics):
        // emit lifting code and let the inner conversion be validated at runtime.
        if source_name == "option" || source_name == "Option" {
            return Ok(Some(CastLiftKind::LiftOption));
        }
        if source_name == "result" || source_name == "Result" {
            return Ok(Some(CastLiftKind::LiftResult));
        }

        // No valid conversion path → compile error
        Err(ShapeError::SemanticError {
            message: format!(
                "Cannot convert `{}` to `{}`: no `Into<{}>` implementation found",
                source_name, target_name, target_name,
            ),
            location: Some(self.span_to_source_location(expr.span())),
        })
    }

    /// Validate a fallible `as Type?` cast.
    fn validate_fallible_cast(
        &mut self,
        expr: &Expr,
        target_name: &str,
    ) -> Result<Option<CastLiftKind>> {
        if !self.has_any_conversion_impls() {
            return Ok(None);
        }

        let source_ann = match self.resolve_source_annotation(expr) {
            Some(ann) => ann,
            None => return Ok(None),
        };

        let source_name = match Self::try_into_name_from_annotation(&source_ann) {
            Some(n) => n,
            None => return Ok(None),
        };

        if source_name == target_name {
            return Ok(Some(CastLiftKind::Direct));
        }

        if self.has_try_into_impl(&source_name, target_name)
            || self.has_into_impl(&source_name, target_name)
        {
            return Ok(Some(CastLiftKind::Direct));
        }

        // Option<T> as M? → lift if T has TryInto<M>
        if let Some(inner_ann) = Self::unwrap_option_inner(&source_ann) {
            if let Some(inner_name) = Self::try_into_name_from_annotation(inner_ann) {
                if inner_name == target_name
                    || self.has_try_into_impl(&inner_name, target_name)
                    || self.has_into_impl(&inner_name, target_name)
                {
                    return Ok(Some(CastLiftKind::LiftOption));
                }
            }
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Cannot convert `Option<{}>` to `{}?`: inner type has no `TryInto<{}>` implementation",
                    annotation_to_string(inner_ann),
                    target_name,
                    target_name,
                ),
                location: Some(self.span_to_source_location(expr.span())),
            });
        }

        // Result<T, E> as M? → lift if T has TryInto<M>
        if let Some(inner_ann) = Self::unwrap_result_inner(&source_ann) {
            if let Some(inner_name) = Self::try_into_name_from_annotation(inner_ann) {
                if inner_name == target_name
                    || self.has_try_into_impl(&inner_name, target_name)
                    || self.has_into_impl(&inner_name, target_name)
                {
                    return Ok(Some(CastLiftKind::LiftResult));
                }
            }
            return Err(ShapeError::SemanticError {
                message: format!(
                    "Cannot convert `Result<{}, _>` to `{}?`: inner type has no `TryInto<{}>` implementation",
                    annotation_to_string(inner_ann),
                    target_name,
                    target_name,
                ),
                location: Some(self.span_to_source_location(expr.span())),
            });
        }

        // Bare wrapper name without generic args (type tracker lost generics):
        // emit lifting code and let the inner conversion be validated at runtime.
        if source_name == "option" || source_name == "Option" {
            return Ok(Some(CastLiftKind::LiftOption));
        }
        if source_name == "result" || source_name == "Result" {
            return Ok(Some(CastLiftKind::LiftResult));
        }

        Err(ShapeError::SemanticError {
            message: format!(
                "Cannot convert `{}` to `{}?`: no `TryInto<{}>` implementation found",
                source_name, target_name, target_name,
            ),
            location: Some(self.span_to_source_location(expr.span())),
        })
    }

    /// Emit bytecode for Option<T> lifting with an infallible conversion.
    ///
    /// Stack effect: replaces top-of-stack Option<T> value with the
    /// lifted result. `None` passes through unchanged (stays the null
    /// sentinel); `Some(t)` is **unwrapped** to its payload `t` and then
    /// converted. This mirrors `emit_result_lift_infallible`'s
    /// `IsOk; UnwrapOk; convert` shape — the `UnwrapOption` step is
    /// mandatory: the `ConvertTo*` opcode bodies (`executor/builtins/
    /// type_ops.rs`) accept only proven scalar/heap-numeric source
    /// kinds, never `Ptr(HeapKind::Option)`. Without the unwrap the
    /// `Some` arm would feed the whole Option carrier into the convert
    /// and surface `"cannot convert kind Ptr(Option) to <T>"`, while the
    /// `None` arm (null sentinel, skipped by `IsNull`) succeeds — the
    /// asymmetric cast WS-12 fixes.
    fn emit_option_lift_infallible(&mut self, convert_opcode: OpCode) {
        // Stack: [option_val]
        self.emit(Instruction::simple(OpCode::Dup));
        // Stack: [option_val, option_val]
        // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
        self.emit(Instruction::simple(OpCode::IsNull));
        // Stack: [option_val, is_none]
        let jump_skip = self.emit_jump(OpCode::JumpIfTrue, 0);
        // Stack: [option_val] — not None: unwrap the Some payload, convert it.
        self.emit(Instruction::simple(OpCode::UnwrapOption));
        // Stack: [inner_val]
        self.emit(Instruction::new(convert_opcode, None));
        // Stack: [converted_val]
        let jump_end = self.emit_jump(OpCode::Jump, 0);
        // skip: value is None, leave the null sentinel on stack
        self.patch_jump(jump_skip);
        // Stack: [null]
        // end:
        self.patch_jump(jump_end);
    }

    /// Emit bytecode for Option<T> lifting with a fallible conversion.
    ///
    /// Stack effect: replaces top-of-stack Option<T> value with the
    /// lifted result. `None` passes through; `Some(t)` is unwrapped to
    /// its payload `t` and then try-converted. Same `UnwrapOption`
    /// requirement as `emit_option_lift_infallible` — see that method's
    /// doc comment.
    fn emit_option_lift_fallible(&mut self, try_convert_opcode: OpCode) {
        // `Option<T> as M?` lift. The Some arm unwraps T and runs the
        // try-convert, which now produces a `Result<M, AnyError>`
        // carrier directly (book §Fallible) — no re-wrap needed. The
        // None arm leaves the null-coded None on the stack; for the `?`
        // consumer that is the early-out None, identical to pre-fix.
        // (The bare-None `match` edge — source statically Option — has
        // no test coverage; the direct-primitive `as Type?` contract the
        // book documents is the covered path and produces a real
        // Result.)
        // Stage 2.6.5.2: typed IsNull replaces `PushNull; Eq`.
        self.emit(Instruction::simple(OpCode::Dup));
        self.emit(Instruction::simple(OpCode::IsNull));
        let jump_skip = self.emit_jump(OpCode::JumpIfTrue, 0);
        // Not None: unwrap the Some payload before the try-convert.
        self.emit(Instruction::simple(OpCode::UnwrapOption));
        self.emit(Instruction::new(try_convert_opcode, None));
        let jump_end = self.emit_jump(OpCode::Jump, 0);
        self.patch_jump(jump_skip);
        self.patch_jump(jump_end);
    }

    /// Emit bytecode for Result<T, E> lifting with an infallible conversion.
    ///
    /// Stack effect: replaces top-of-stack Result<T, E> with Result<M, E>.
    /// Err values pass through; Ok(t) values are unwrapped, converted, and re-wrapped.
    fn emit_result_lift_infallible(&mut self, convert_opcode: OpCode) -> Result<()> {
        // Stack: [result_val]
        self.emit(Instruction::simple(OpCode::Dup));
        // Stack: [result_val, result_val]
        self.emit(Instruction::simple(OpCode::IsOk));
        // Stack: [result_val, is_ok]
        let jump_skip = self.emit_jump(OpCode::JumpIfFalse, 0);
        // Stack: [result_val] — it's Ok, unwrap and convert
        self.emit(Instruction::simple(OpCode::UnwrapOk));
        // Stack: [inner_val]
        self.emit(Instruction::new(convert_opcode, None));
        // Stack: [converted_val]
        // Re-wrap in Ok() by calling the Ok builtin
        self.emit_call_ok()?;
        // Stack: [Ok(converted_val)]
        let jump_end = self.emit_jump(OpCode::Jump, 0);
        // skip: value is Err, leave it on stack
        self.patch_jump(jump_skip);
        // Stack: [err_result]
        // end:
        self.patch_jump(jump_end);
        Ok(())
    }

    /// Emit bytecode for Result<T, E> lifting with a fallible conversion.
    fn emit_result_lift_fallible(&mut self, try_convert_opcode: OpCode) -> Result<()> {
        self.emit(Instruction::simple(OpCode::Dup));
        self.emit(Instruction::simple(OpCode::IsOk));
        let jump_skip = self.emit_jump(OpCode::JumpIfFalse, 0);
        self.emit(Instruction::simple(OpCode::UnwrapOk));
        // `TryConvertTo*` now produces a `Result<M, AnyError>` carrier
        // directly (book §Fallible), so the Ok payload is converted AND
        // re-wrapped by the opcode itself — NO `emit_call_ok` here, else
        // we'd double-wrap to `Ok(Ok(v))`. The Err arm (jump_skip) keeps
        // the original Err carrier as the early-out value.
        self.emit(Instruction::new(try_convert_opcode, None));
        let jump_end = self.emit_jump(OpCode::Jump, 0);
        self.patch_jump(jump_skip);
        self.patch_jump(jump_end);
        Ok(())
    }

    /// Emit a call to the `Ok()` builtin to re-wrap a value in Ok.
    fn emit_call_ok(&mut self) -> Result<()> {
        if let Some(func_idx) = self.find_function("Ok") {
            let arg_count = self.program.add_constant(Constant::Int(1));
            self.emit(Instruction::new(
                OpCode::PushConst,
                Some(Operand::Const(arg_count)),
            ));
            self.emit(Instruction::new(
                OpCode::Call,
                Some(Operand::Function(shape_value::FunctionId(func_idx as u16))),
            ));
        }
        // Ok() not found — in stripped test mode, skip the re-wrap
        Ok(())
    }

    pub(super) fn expr_is_type_symbol(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Identifier(name, _) if self.is_type_symbol_name(name))
    }

    /// Resolve a static type annotation for an expression.
    ///
    /// This powers static `.type()` queries when the receiver type is fully
    /// known at compile time.
    pub(super) fn static_type_annotation_for_expr(
        &mut self,
        expr: &Expr,
    ) -> Result<TypeAnnotation> {
        // Support type symbols directly (e.g. Point.type()).
        if let Expr::Identifier(name, _) = expr {
            if self.is_type_symbol_name(name) {
                return Ok(TypeAnnotation::Reference(name.as_str().into()));
            }

            // Prefer compiler-tracked local/module_binding types for identifiers.
            if let Some(local_idx) = self.resolve_local(name) {
                if let Some(info) = self.type_tracker.get_local_type(local_idx) {
                    if let Some(type_name) = &info.type_name {
                        return Ok(type_name_to_annotation(type_name));
                    }
                }
            }
            if let Some(binding_idx) = self.module_bindings.get(name).copied() {
                if let Some(info) = self.type_tracker.get_binding_type(binding_idx) {
                    if let Some(type_name) = &info.type_name {
                        return Ok(type_name_to_annotation(type_name));
                    }
                }
            }
        }

        let inferred = self.infer_expr_type(expr)?;
        if let Some(annotation) = inferred.to_annotation() {
            return Ok(annotation);
        }

        Err(ShapeError::SemanticError {
            message: format!(
                "Could not resolve a concrete static type for expression in type query: {:?}",
                inferred
            ),
            location: Some(self.span_to_source_location(expr.span())),
        })
    }

    /// True when a cast from `source_name` to `target_name` is a BUILT-IN
    /// primitive numeric conversion (both endpoints are primitive numeric
    /// types per the numeric lattice — `int`/`i64`, the width names,
    /// `number`/`f32`, `decimal`).
    ///
    /// Such a cast lowers to the typed `ConvertToInt` / `ConvertToNumber` /
    /// `ConvertToDecimal` opcode — which BOTH converts the value (`2` → `2.0`,
    /// NOT a bit reinterpret) AND re-stamps the result slot's `NativeKind` /
    /// `last_expr_numeric_type` via `record_cast_result_kind`. It must NOT be
    /// intercepted by the user-`Into`-impl routing below: a stdlib
    /// `Into<number> for int` (or sibling) would emit a `Call` that leaves the
    /// compiler's last-expr numeric tracker on the SOURCE type (`int`), so a
    /// downstream `a / b` selects `DivInt` and divides the f64 BIT PATTERNS as
    /// i64 — the `(2 as number) / (8 as number) == 0` soundness hole. The
    /// built-in primitive path is the only correct lowering for these pairs.
    fn is_builtin_primitive_numeric_cast(source_name: &str, target_name: &str) -> bool {
        BuiltinTypes::is_numeric_type_name(source_name)
            && BuiltinTypes::is_numeric_type_name(target_name)
    }

    /// Return the typed ConvertTo* opcode for a primitive target type name,
    /// or None for non-primitive types (which fall through to Convert + trait dispatch).
    fn convert_opcode_for_primitive(target: &str) -> Option<OpCode> {
        match target {
            "int" => Some(OpCode::ConvertToInt),
            "number" => Some(OpCode::ConvertToNumber),
            "string" => Some(OpCode::ConvertToString),
            "bool" => Some(OpCode::ConvertToBool),
            "decimal" => Some(OpCode::ConvertToDecimal),
            "char" => Some(OpCode::ConvertToChar),
            _ => None,
        }
    }

    /// Record the last-expr kind for a completed primitive `as`-cast so
    /// downstream consumers (top-level return-kind synthesis, `let`
    /// binding type tracking, binary-op typed dispatch) observe the
    /// CAST TARGET kind rather than the source kind.
    ///
    /// WS-12: applies only to the direct (non-lift) primitive cast —
    /// the Option/Result lift result is a null-coded `Option<U>` /
    /// `Result<U, _>` whose top-of-stack value is either the null
    /// sentinel or the scalar, so a bare scalar kind must NOT be
    /// stamped for the lift paths.
    fn record_cast_result_kind(&mut self, target_selector: &str) {
        // U4-4: the deleted `last_expr_numeric_type` register stamp for the
        // cast TARGET kind is replaced by a `last_expr_type_info` name stamp.
        // A downstream `(x as number) / (y as number)` derives `Number` for
        // both operands from the cast expressions' one resolved Type
        // (`numeric_type_of` → `infer_expr_type`), so `DivNumber` is selected
        // — closing the `(2 as number) / (8 as number) == 0` soundness hole
        // without a competing register.
        match target_selector {
            "int" | "number" | "decimal" | "bool" | "string" | "char" => {
                self.last_expr_type_info = Some(crate::type_tracking::VariableTypeInfo::named(
                    target_selector.to_string(),
                ));
            }
            // Non-primitive targets do not reach this helper (the caller
            // gates on `convert_opcode_for_primitive` being `Some`).
            _ => {}
        }
    }

    /// Return the typed TryConvertTo* opcode for a primitive target type name,
    /// or None for non-primitive types (which fall through to Convert + trait dispatch).
    fn try_convert_opcode_for_primitive(target: &str) -> Option<OpCode> {
        match target {
            "int" => Some(OpCode::TryConvertToInt),
            "number" => Some(OpCode::TryConvertToNumber),
            "string" => Some(OpCode::TryConvertToString),
            "bool" => Some(OpCode::TryConvertToBool),
            "decimal" => Some(OpCode::TryConvertToDecimal),
            "char" => Some(OpCode::TryConvertToChar),
            _ => None,
        }
    }

    /// Compile a type assertion expression (expr as Type)
    ///
    /// This wraps the value with a TypeAnnotatedValue so that meta formatting
    /// can be applied when the value is printed.
    pub(super) fn compile_expr_type_assertion(
        &mut self,
        expr: &Expr,
        type_annotation: &shape_ast::ast::TypeAnnotation,
    ) -> Result<()> {
        // ── Fallible path: `as Type?` (parsed as `as Option<Type>`) ──
        if let TypeAnnotation::Generic { name, args } = type_annotation
            && name == "Option"
            && args.len() == 1
        {
            let inner_type = &args[0];
            let target_selector =
                Self::try_into_name_from_annotation(inner_type).ok_or_else(|| {
                    ShapeError::SemanticError {
                        message: format!(
                            "`as Type?` target must be a named type selector, found '{}'",
                            annotation_to_string(inner_type)
                        ),
                        location: Some(self.span_to_source_location(expr.span())),
                    }
                })?;

            // Validate the fallible cast (checks source type + lifting)
            let cast_kind = self.validate_fallible_cast(expr, &target_selector)?;

            // PB4 sub-root #6: when a user `TryInto<Target>` impl is in
            // scope for the source type (and we are not currently
            // compiling that impl's body — the stdlib `self as int?`
            // recursion guard), dispatch through the user method
            // instead of the primitive `TryConvertTo*` opcode. The
            // user's body (e.g. `Ok(7)` in
            // `tools/shape-test/tests/error_handling/try_operator.rs:189`)
            // would otherwise be unreachable for primitive targets.
            // Bounded to `CastLiftKind::Direct` (non-wrapped source);
            // Option/Result lift paths preserve current behaviour.
            if matches!(cast_kind, Some(CastLiftKind::Direct)) {
                if let Some(source_name) = self.cast_source_name(expr) {
                    // Built-in primitive numeric cast → primitive
                    // `TryConvertTo*` path below, never a user `Into`-impl
                    // `Call`. See `is_builtin_primitive_numeric_cast`.
                    if !Self::is_builtin_primitive_numeric_cast(&source_name, &target_selector) {
                        if let Some(func_idx) =
                            self.user_impl_method_for_cast(&source_name, &target_selector, true)
                        {
                            self.compile_expr(expr)?;
                            self.emit_user_impl_cast_call(func_idx);
                            return Ok(());
                        }
                    }
                }
            }

            if let Some(try_convert_opcode) =
                Self::try_convert_opcode_for_primitive(&target_selector)
            {
                match cast_kind {
                    Some(CastLiftKind::LiftOption) => {
                        // Option<T> as M? → null-check lifting with try-convert
                        self.compile_expr(expr)?;
                        self.emit_option_lift_fallible(try_convert_opcode);
                        return Ok(());
                    }
                    Some(CastLiftKind::LiftResult) => {
                        // Result<T, E> as M? → Ok/Err lifting with try-convert
                        self.compile_expr(expr)?;
                        self.emit_result_lift_fallible(try_convert_opcode)?;
                        return Ok(());
                    }
                    _ => {
                        // Direct conversion
                        self.compile_expr(expr)?;
                        self.emit(Instruction::new(try_convert_opcode, None));
                        return Ok(());
                    }
                }
            }

            let source_name = self
                .static_type_annotation_for_expr(expr)
                .ok()
                .and_then(|ann| Self::try_into_name_from_annotation(&ann))
                .or_else(|| {
                    self.infer_expr_type(expr)
                        .ok()
                        .and_then(|ty| Self::try_into_name_from_type(&ty))
                })
                .ok_or_else(|| ShapeError::SemanticError {
                    message: format!(
                        "`as Type?` requires a concrete source type for TryInto dispatch, found expression '{}'",
                        annotation_to_string(type_annotation)
                    ),
                    location: Some(self.span_to_source_location(expr.span())),
                })?;

            // `as Type?` compiles to trait-dispatch metadata consumed by Convert.
            self.compile_expr(expr)?;
            let dispatch = Self::conversion_dispatch_annotation(
                TRY_INTO_DISPATCH_TAG,
                &source_name,
                &target_selector,
            );
            let target = self
                .program
                .add_constant(Constant::TypeAnnotation(dispatch));
            self.emit(Instruction::new(
                OpCode::Convert,
                Some(Operand::Const(target)),
            ));
            return Ok(());
        }

        // ── Static union widening: `expr as A | B` ──
        //
        // Union annotations are compile-time metadata. The runtime carrier stays
        // the source expression's kinded slot, and downstream typed patterns
        // (`n: int`, `s: string`) use TypeCheck against that slot kind. Emitting
        // WrapTypeAnnotation here would re-enter the deleted ValueWord wrapper
        // path; emitting Convert would require trait dispatch. Accept only when
        // the compiler can prove the source annotation is a member/subset of the
        // target union.
        if let TypeAnnotation::Union(union_members) = type_annotation {
            let source_annotation = self.resolve_source_annotation(expr).ok_or_else(|| {
                ShapeError::SemanticError {
                    message: format!(
                        "Cannot widen expression to union `{}`: source type is not statically known",
                        annotation_to_string(type_annotation)
                    ),
                    location: Some(self.span_to_source_location(expr.span())),
                }
            })?;

            if !Self::source_annotation_fits_union(&source_annotation, union_members) {
                return Err(ShapeError::SemanticError {
                    message: format!(
                        "Cannot widen `{}` to union `{}`: source type is not a union member",
                        annotation_to_string(&source_annotation),
                        annotation_to_string(type_annotation)
                    ),
                    location: Some(self.span_to_source_location(expr.span())),
                });
            }

            self.compile_expr(expr)?;
            self.last_expr_type_info = None;
            return Ok(());
        }

        // ── Width integer cast: `expr as i8`, `expr as u16`, etc. ──
        // Emits CastWidth which does bit-truncation (Rust-style). Not Into-based.
        if let TypeAnnotation::Basic(name) = type_annotation {
            if let Some(w) = shape_ast::IntWidth::from_name(name) {
                self.compile_expr(expr)?;
                self.emit(Instruction::new(
                    OpCode::CastWidth,
                    Some(Operand::Width(NumericWidth::from_int_width(w))),
                ));
                return Ok(());
            }
        }

        // ── Infallible path: `as Type` ──
        if let Some(target_selector) = Self::try_into_name_from_annotation(type_annotation) {
            // Validate the infallible cast (checks source type + lifting)
            let cast_kind = self.validate_infallible_cast(expr, &target_selector)?;

            // PB4 sub-root #6: same user-impl routing as the fallible
            // path — see the `Some(CastLiftKind::Direct)` block in
            // `validate_fallible_cast` above for the recursion-guard
            // and stdlib-redeclare semantics.
            if matches!(cast_kind, Some(CastLiftKind::Direct)) {
                if let Some(source_name) = self.cast_source_name(expr) {
                    // A built-in primitive numeric cast (`int as number`,
                    // `number as int`, width↔number, …) MUST take the
                    // primitive `ConvertTo*` path below — never a user
                    // `Into`-impl `Call` — so the value is converted AND the
                    // result kind is re-stamped. See
                    // `is_builtin_primitive_numeric_cast`.
                    if !Self::is_builtin_primitive_numeric_cast(&source_name, &target_selector) {
                        if let Some(func_idx) =
                            self.user_impl_method_for_cast(&source_name, &target_selector, false)
                        {
                            self.compile_expr(expr)?;
                            self.emit_user_impl_cast_call(func_idx);
                            return Ok(());
                        }
                    }
                }
            }

            if let Some(convert_opcode) = Self::convert_opcode_for_primitive(&target_selector) {
                match cast_kind {
                    Some(CastLiftKind::LiftOption) => {
                        // Option<T> as M → null-check lifting with convert
                        self.compile_expr(expr)?;
                        self.emit_option_lift_infallible(convert_opcode);
                        return Ok(());
                    }
                    Some(CastLiftKind::LiftResult) => {
                        // Result<T, E> as M → Ok/Err lifting with convert
                        self.compile_expr(expr)?;
                        self.emit_result_lift_infallible(convert_opcode)?;
                        return Ok(());
                    }
                    _ => {
                        // Direct conversion (primitive fast path)
                        self.compile_expr(expr)?;
                        self.emit(Instruction::new(convert_opcode, None));
                        // WS-12: the `ConvertTo*` opcode body re-stamps
                        // the result slot's `NativeKind` to the cast
                        // target (`op_convert_to_*` in `executor/
                        // builtins/type_ops.rs`). The compiler's
                        // last-expr kind trackers must follow suit or a
                        // downstream consumer — top-level return-kind
                        // synthesis, a `let` binding's type, a binary
                        // op's typed-opcode dispatch — keeps the SOURCE
                        // type and mis-formats the converted value
                        // (e.g. `let y = x as number` then `y + 0.5`
                        // printing the f64 bits as an integer). The
                        // `CastWidth` width-cast arm already does this;
                        // the primitive `as`-cast arm must match.
                        self.record_cast_result_kind(&target_selector);
                        return Ok(());
                    }
                }
            }

            // `as Type` compiles to Into<Target> dispatch through Convert.
            self.compile_expr(expr)?;
            let dispatch = self
                .static_type_annotation_for_expr(expr)
                .ok()
                .and_then(|ann| Self::try_into_name_from_annotation(&ann))
                .or_else(|| {
                    self.infer_expr_type(expr)
                        .ok()
                        .and_then(|ty| Self::try_into_name_from_type(&ty))
                })
                .map(|source_name| {
                    Self::conversion_dispatch_annotation(
                        INTO_DISPATCH_TAG,
                        &source_name,
                        &target_selector,
                    )
                })
                .unwrap_or_else(|| type_annotation.clone());
            let target = self
                .program
                .add_constant(Constant::TypeAnnotation(dispatch));
            self.emit(Instruction::new(
                OpCode::Convert,
                Some(Operand::Const(target)),
            ));
            return Ok(());
        }

        // Compile the expression
        self.compile_expr(expr)?;

        // Get the type name for wrapping
        let type_name = annotation_to_string(type_annotation);

        // Add type name to string pool
        let type_name_idx = self.program.add_string(type_name);

        // Emit WrapTypeAnnotation to wrap the value
        self.emit(Instruction::new(
            OpCode::WrapTypeAnnotation,
            Some(Operand::Property(type_name_idx as u16)),
        ));

        Ok(())
    }

    /// Compile an explicit impl selector expression: `expr using ImplName`.
    ///
    /// We encode the selector as a TypeAnnotatedValue marker so formatting
    /// builtins can resolve named Display implementations at runtime.
    pub(super) fn compile_expr_using_impl(&mut self, expr: &Expr, impl_name: &str) -> Result<()> {
        self.compile_expr(expr)?;

        let marker = format!("__impl__:{}", impl_name);
        let marker_idx = self.program.add_string(marker);
        self.emit(Instruction::new(
            OpCode::WrapTypeAnnotation,
            Some(Operand::Property(marker_idx as u16)),
        ));
        Ok(())
    }

    /// Compile an instanceof expression
    pub(super) fn compile_expr_instanceof(
        &mut self,
        expr: &Expr,
        type_annotation: &shape_ast::ast::TypeAnnotation,
    ) -> Result<()> {
        self.compile_expr(expr)?;
        let type_const = self
            .program
            .add_constant(Constant::TypeAnnotation(type_annotation.clone()));
        self.emit(Instruction::new(
            OpCode::TypeCheck,
            Some(Operand::Const(type_const)),
        ));
        Ok(())
    }
}
