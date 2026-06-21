//! Operator type inference
//!
//! Handles type inference for literals, binary operators, and unary operators.
//!
//! ## Option Propagation
//!
//! Arithmetic operators support automatic Option propagation:
//! - `Option<Number> + Number` -> `Option<Number>`
//! - `Number + Option<Number>` -> `Option<Number>`
//! - `Option<Number> + Option<Number>` -> `Option<Number>`
//!
//! This enables ergonomic handling of nullable values without explicit unwrapping.
//! At runtime, NaN sentinel is used for Option<f64>, so propagation is zero-cost.

use super::TypeInferenceEngine;
use crate::type_system::*;
use shape_ast::ast::{BinaryOp, Expr, Literal, Span, TypeAnnotation, UnaryOp};

/// Classification of a temporal operand for the documented DateTime/Duration
/// operator arithmetic (datetime book chapter). Duration is its own type, NOT
/// Numeric — these kinds drive a result-type table separate from the numeric
/// arithmetic path, with no int/number coercion.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TemporalKind {
    DateTime,
    Duration,
}

impl TypeInferenceEngine {
    /// Infer type of a literal
    pub(crate) fn infer_literal(&mut self, lit: &Literal) -> TypeResult<Type> {
        Ok(match lit {
            Literal::Int(_) => Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            Literal::UInt(_) => Type::Concrete(TypeAnnotation::Basic("u64".to_string())),
            Literal::TypedInt(_, w) => {
                Type::Concrete(TypeAnnotation::Basic(w.type_name().to_string()))
            }
            Literal::Number(_) => BuiltinTypes::number(),
            Literal::Decimal(_) => Type::Concrete(TypeAnnotation::Basic("decimal".to_string())),
            Literal::String(_) => BuiltinTypes::string(),
            // Character literals evaluate to their integer code point
            // (operators.mdx "Character Literals" — the interop escape hatch).
            // There is NO distinct `char` type: `'A'` IS the int 65, usable
            // anywhere an `int` is. No char<->string coercion exists.
            Literal::Char(_) => Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            Literal::FormattedString { .. } => BuiltinTypes::string(),
            Literal::Bool(_) => BuiltinTypes::boolean(),
            // `None` is polymorphic: Option<T> for fresh T.
            Literal::None => Self::wrap_in_option(self.fresh_type_var()),
            Literal::Unit => Type::Concrete(TypeAnnotation::Basic("()".to_string())),
            Literal::Timeframe(_) => Type::Concrete(TypeAnnotation::Basic("timeframe".to_string())),
        })
    }

    /// Numeric-conversion LITERAL ADOPTION (numeric-conversion-spec §4).
    ///
    /// An untyped integer literal adopts the numeric type required by its
    /// context IFF the literal value is losslessly representable in that type.
    /// `let n: number = 5` makes `5` the f64 literal `5.0`; `val: number > 10`
    /// makes `10` a number literal; `P { x: 1 }` makes `1` a number. An
    /// out-of-range literal does NOT adopt (it surfaces as a compile error
    /// downstream — the literal type stays its natural `int`/`u64`, which then
    /// fails the §2 lattice against the sized target).
    ///
    /// Returns the adopted (context) type when `expr` is a bare integer literal
    /// (`Int`/`UInt`) whose value losslessly fits the concrete numeric
    /// `context` type; otherwise `None` (the literal keeps its natural type).
    /// Explicitly-typed literals (`42u8`) and float/decimal literals do NOT
    /// adopt — they are already their declared type.
    pub(crate) fn adopt_int_literal_in_context(expr: &Expr, context: &Type) -> Option<Type> {
        let lit = match expr {
            Expr::Literal(lit, _) => lit,
            _ => return None,
        };
        let ctx_name = match context {
            Type::Concrete(TypeAnnotation::Basic(n)) => n.as_str(),
            Type::Concrete(TypeAnnotation::Reference(n)) => n.as_str(),
            _ => return None,
        };
        if !BuiltinTypes::is_numeric_type_name(ctx_name) {
            return None;
        }
        let fits = match lit {
            Literal::Int(v) => Self::int_value_fits_numeric(*v as i128, ctx_name),
            Literal::UInt(v) => Self::int_value_fits_numeric(*v as i128, ctx_name),
            // Float/decimal/typed-int literals are already their own type and
            // do not context-adopt (a `42u8` is a u8; `3.5` is a number).
            _ => return None,
        };
        if fits { Some(context.clone()) } else { None }
    }

    /// A bare integer literal whose comparison/arithmetic partner is still an
    /// UNRESOLVED inference variable adopts that variable's identity rather than
    /// staying its natural `int`. This is the var-side analogue of
    /// `adopt_int_literal_in_context`: there the literal adopts a *concrete*
    /// numeric context (`val:number > 10` → `10:number`); here the context is a
    /// not-yet-resolved var (`fn check(n) -> Result<number> { if n > 5 ...; Ok(n)
    /// }` — `n` is a free var when `n > 5` is inferred, later pinned to `number`
    /// by `Ok(n)`). Without this, the comparison arm's `effective_left ~
    /// effective_right` same-type constraint would pin the var to `int` from the
    /// literal, colliding with the eventual `number` and spuriously rejecting
    /// valid code (R1 comparison-literal-adoption-ordering).
    ///
    /// Mirrors the var-propagation arm of `numeric_result_type` (a `(Variable,
    /// Concrete numeric)` pair yields the variable, not the concrete): the var is
    /// the operand the call graph will resolve, so the literal must defer to it.
    ///
    /// SOUNDNESS: fires ONLY for a bare `Int`/`UInt` literal (delegates the
    /// literal-shape gate to `adopt_int_literal_in_context`, which rejects
    /// float/decimal/typed-int literals and any non-literal value) paired with an
    /// unresolved `Type::Variable`. A non-literal operand never adopts, so no
    /// `int`-VALUE silently becomes a `number` and no `number`-VALUE becomes an
    /// `int` — this is pure literal deferral, identical in spirit to the existing
    /// concrete-context adoption. The literal has no committed family until the
    /// var resolves, so adopting the var's identity introduces no widening.
    pub(crate) fn adopt_int_literal_into_var(expr: &Expr, context: &Type) -> Option<Type> {
        // Only adopt when the partner is a still-unresolved inference variable.
        if !matches!(context, Type::Variable(_)) {
            return None;
        }
        // Reuse the literal-shape + value-fits gate. `decimal` accepts any
        // integer literal, so it is a stable proxy for "this expr is a bare
        // adoptable integer literal" without re-implementing the match.
        let decimal_probe = Type::Concrete(TypeAnnotation::Basic("decimal".to_string()));
        Self::adopt_int_literal_in_context(expr, &decimal_probe)?;
        // The literal adopts the partner var's identity (defers to it).
        Some(context.clone())
    }

    /// The concrete numeric type name of a `Type`, if it is a `Basic`/
    /// `Reference` concrete numeric type. `None` for type vars, compound, or
    /// non-numeric types. Used to gate numeric return-context literal adoption.
    pub(crate) fn concrete_numeric_type_name(ty: &Type) -> Option<String> {
        let name = match ty {
            Type::Concrete(TypeAnnotation::Basic(n)) => n.as_str(),
            Type::Concrete(TypeAnnotation::Reference(n)) => n.as_str(),
            _ => return None,
        };
        if BuiltinTypes::is_numeric_type_name(name) {
            Some(name.to_string())
        } else {
            None
        }
    }

    /// Whether integer value `v` is losslessly representable in the numeric
    /// type named `name` (numeric-conversion-spec §4 literal-adoption range
    /// check). Width names use their concrete `[min, max]`; `int`/`i64` use the
    /// i64 range; `u64`/`usize` the u64 range; `number`/`f64` the exact-integer
    /// range `[-2^53, 2^53]`; `f32` the range `[-2^24, 2^24]`. `decimal` accepts
    /// any integer literal (arbitrary precision).
    fn int_value_fits_numeric(v: i128, name: &str) -> bool {
        if let Some(w) = shape_ast::IntWidth::from_name(name) {
            return if w.is_signed() {
                v >= w.min_value() as i128 && v <= w.max_value() as i128
            } else {
                v >= 0 && v <= w.max_unsigned() as i128
            };
        }
        match BuiltinTypes::canonical_numeric_runtime_name(name) {
            Some("i64") | Some("isize") => v >= i64::MIN as i128 && v <= i64::MAX as i128,
            Some("usize") => v >= 0 && v <= u64::MAX as i128,
            Some("f64") => v >= -(1i128 << 53) && v <= (1i128 << 53),
            Some("f32") => v >= -(1i128 << 24) && v <= (1i128 << 24),
            // decimal: arbitrary precision, any integer literal fits.
            _ => name == "decimal" || name == "Decimal",
        }
    }

    /// Check if a type is Option<T> and extract the inner type
    fn unwrap_option_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { base, args } if args.len() == 1 => {
                if let Type::Concrete(ann) = base.as_ref() {
                    if ann.as_type_name_str() == Some("Option") {
                        return Some(args[0].clone());
                    }
                }
                None
            }
            // Handle T? desugared to TypeAnnotation::Generic { name: "Option", args }
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if name == "Option" && args.len() == 1 =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            _ => None,
        }
    }

    /// Is this type a bare null sentinel — the `None` literal or an explicit
    /// `null`/`Null` annotation?
    ///
    /// The `None` literal infers as `Option<var>` (an Option whose element is a
    /// still-unresolved type variable, see `infer_literal`). A concrete
    /// `Option<int>` is NOT a sentinel — its element type is known and equality
    /// against it should still type-check normally. Used by the `==`/`!=` arm to
    /// allow `None == x` (null-presence checks) without forcing same-type
    /// unification, while keeping `1 == "x"` a rejection.
    fn is_null_sentinel(ty: &Type) -> bool {
        match ty {
            // Explicit `null`/`None` annotation.
            Type::Concrete(TypeAnnotation::Null) => true,
            // `None` literal → `Option<var>`; only an unresolved element counts.
            Type::Generic { base, args }
                if args.len() == 1
                    && matches!(
                        base.as_ref(),
                        Type::Concrete(ann) if ann.as_type_name_str() == Some("Option")
                    )
                    && matches!(args[0], Type::Variable(_)) =>
            {
                true
            }
            _ => false,
        }
    }

    /// Wrap a type in Option<T>
    fn wrap_in_option(ty: Type) -> Type {
        Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Option".into()))),
            args: vec![ty],
        }
    }

    /// Check if a type is Result<T>/Option<T>/T? and extract the success type.
    fn unwrap_result_or_option_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Generic { base, args } if !args.is_empty() => match base.as_ref() {
                Type::Concrete(ann)
                    if ann
                        .as_type_name_str()
                        .is_some_and(|n| n == "Result" || n == "Option") =>
                {
                    Some(args[0].clone())
                }
                _ => None,
            },
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if (name == "Result" || name == "Option") && !args.is_empty() =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            _ => None,
        }
    }

    /// Wrap a type in Result<T, AnyError>.
    fn wrap_in_result(&self, ty: Type) -> Type {
        self.wrap_result_type(ty)
    }

    /// Compute the result type for numeric arithmetic based on operand types.
    ///
    /// Same concrete numeric type → preserve it (int*int→int, number*number→number).
    /// Mixed concrete numeric → widen to number (int*float→number).
    /// One operand still a TypeVar, the other concrete numeric → propagate the
    /// TypeVar as the result. This keeps the result type *linked* to the
    /// unresolved operand instead of eagerly collapsing to `number`. It is the
    /// load-bearing change for transitive inference: in `fn double(x){x*2}`
    /// the body's type becomes `x`'s own variable, so `double`'s return type
    /// is unified with its parameter. Once a callsite (possibly several calls
    /// deep) resolves the parameter, the return type resolves with it. The
    /// eager `number` collapse severed that link and made `double` infer as
    /// `fn(number)->number` even when every observed argument was an `int`.
    /// Both operands TypeVars → no information yet; default to `number`.
    fn numeric_result_type(left: &Type, right: &Type) -> Type {
        match (left, right) {
            // Same concrete numeric type → preserve it
            (
                Type::Concrete(TypeAnnotation::Basic(l)),
                Type::Concrete(TypeAnnotation::Basic(r)),
            ) if l == r && BuiltinTypes::is_numeric_type_name(l) => left.clone(),
            // Mixed widths within the same script-level numeric family
            // (e.g. `i8 + i16`, `u8 + u32`) → preserve the script family type.
            // Both integer widths canonicalize to `int`; both float widths to
            // `number`. This keeps `i8 + i16` an `int` (not a `number`), so it
            // satisfies a `-> int` return. Cross-family mixes (int + float,
            // int + decimal) fall through to the widen-to-number arm below.
            (
                Type::Concrete(TypeAnnotation::Basic(l)),
                Type::Concrete(TypeAnnotation::Basic(r)),
            ) if BuiltinTypes::is_numeric_type_name(l)
                && BuiltinTypes::is_numeric_type_name(r)
                && BuiltinTypes::canonical_script_alias(l).is_some()
                && BuiltinTypes::canonical_script_alias(l)
                    == BuiltinTypes::canonical_script_alias(r) =>
            {
                let alias =
                    BuiltinTypes::canonical_script_alias(l).expect("guarded by is_some() above");
                Type::Concrete(TypeAnnotation::Basic(alias.to_string()))
            }
            // Mixed concrete numeric across families → widen to number
            (
                Type::Concrete(TypeAnnotation::Basic(l)),
                Type::Concrete(TypeAnnotation::Basic(r)),
            ) if BuiltinTypes::is_numeric_type_name(l) && BuiltinTypes::is_numeric_type_name(r) => {
                BuiltinTypes::number()
            }
            // One operand is an unresolved variable, the other a concrete
            // numeric type → propagate the variable so the result type stays
            // tied to the operand the call graph will eventually resolve.
            (Type::Variable(_), Type::Concrete(TypeAnnotation::Basic(r)))
                if BuiltinTypes::is_numeric_type_name(r) =>
            {
                left.clone()
            }
            (Type::Concrete(TypeAnnotation::Basic(l)), Type::Variable(_))
                if BuiltinTypes::is_numeric_type_name(l) =>
            {
                right.clone()
            }
            // Both operands are still unresolved variables → keep the result
            // LINKED to the left operand variable rather than eagerly collapsing
            // to `number`. The eager collapse severs the call-graph link in the
            // recursive case `n * factorial(n - 1)`: `factorial`'s return is
            // still a fresh variable while its own body is being inferred, so
            // the multiply pairs two variables. Collapsing to `number` makes the
            // if-branch unification `int(then) ~ number(else)` a hard mismatch
            // under the strict §2 numeric lattice (int->number is now
            // cast-required), spuriously rejecting valid all-int recursion.
            // Propagating the variable lets the callsite-union fixpoint resolve
            // it to the concrete argument type (`int` for `factorial(6)`); the
            // deferred `number` default in `refine_numeric_params_post_callsite`
            // still fires when NO call site ever pins the variable, so a
            // never-called `fn triple(x){x*3}` still resolves its param to
            // `number`.
            (Type::Variable(_), Type::Variable(_)) => left.clone(),
            // Any other operand shape (non-basic concrete) → default to number.
            _ => BuiltinTypes::number(),
        }
    }

    /// True when `ty` is still an unresolved inference variable (or a
    /// constraint-bearing var) with no concrete shape yet. Used by the
    /// overloaded `+` arm to defer the Numeric commitment when neither operand
    /// can disambiguate numeric-add vs string-concat. A-final ROOT J3.
    fn is_unresolved_var(ty: &Type) -> bool {
        matches!(ty, Type::Variable(_) | Type::Constrained { .. })
    }

    fn is_string_like(ty: &Type) -> bool {
        match ty {
            Type::Concrete(ann) if ann.as_type_name_str() == Some("string") => true,
            Type::Concrete(TypeAnnotation::Union(types)) => types
                .iter()
                .any(|ann| ann.as_type_name_str() == Some("string")),
            Type::Generic { base, args } if args.len() == 1 => {
                matches!(
                    base.as_ref(),
                    Type::Concrete(ann) if ann.as_type_name_str() == Some("Option")
                ) && matches!(
                    &args[0],
                    Type::Concrete(ann) if ann.as_type_name_str() == Some("string")
                )
            }
            _ => false,
        }
    }

    fn is_vec_number(ty: &Type) -> bool {
        match ty {
            Type::Concrete(TypeAnnotation::Array(inner)) => inner
                .as_type_name_str()
                .is_some_and(|n| BuiltinTypes::is_numeric_type_name(n)),
            Type::Concrete(TypeAnnotation::Generic { name, args }) if name == "Vec" => {
                args.first().is_some_and(|arg| {
                    arg.as_type_name_str()
                        .is_some_and(|n| BuiltinTypes::is_numeric_type_name(n))
                })
            }
            Type::Generic { base, args } if args.len() == 1 => {
                matches!(
                    base.as_ref(),
                    Type::Concrete(ann) if ann.as_type_name_str() == Some("Vec")
                ) && matches!(
                    &args[0],
                    Type::Concrete(ann) if ann.as_type_name_str().is_some_and(|n| BuiltinTypes::is_numeric_type_name(n))
                )
            }
            _ => false,
        }
    }

    fn is_mat_number(ty: &Type) -> bool {
        match ty {
            Type::Concrete(TypeAnnotation::Generic { name, args }) if name == "Mat" => {
                args.first().is_some_and(|arg| {
                    arg.as_type_name_str()
                        .is_some_and(|n| BuiltinTypes::is_numeric_type_name(n))
                })
            }
            Type::Generic { base, args } if args.len() == 1 => {
                matches!(
                    base.as_ref(),
                    Type::Concrete(ann) if ann.as_type_name_str() == Some("Mat")
                ) && matches!(
                    &args[0],
                    Type::Concrete(ann) if ann.as_type_name_str().is_some_and(|n| BuiltinTypes::is_numeric_type_name(n))
                )
            }
            _ => false,
        }
    }

    fn mat_number_type() -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Mat".into(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        })
    }

    fn vec_number_type() -> Type {
        Type::Concrete(TypeAnnotation::Generic {
            name: "Vec".into(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        })
    }

    /// Build intersection type for object-like `+` (structural merge).
    fn infer_object_add_type(left: &Type, right: &Type) -> Option<Type> {
        fn push_members(ty: &Type, out: &mut Vec<TypeAnnotation>) -> bool {
            match ty {
                Type::Concrete(TypeAnnotation::Object(fields)) => {
                    out.push(TypeAnnotation::Object(fields.clone()));
                    true
                }
                // NOTE: a bare `Reference(name)` (a NAMED type like `Money`) is
                // deliberately NOT a merge member. Object-literal merge is for
                // UNTYPED object literals only; a named type that implements
                // `Add` dispatches to its impl (checked in the `Add` arm BEFORE
                // this helper). Without this exclusion a `Money + Money` was
                // hijacked into `Intersection([Money, Money])` — the structural
                // merge of two named structs — instead of the user's
                // `impl Add for Money`. MERGE-HIJACK fix (operators slice).
                Type::Concrete(TypeAnnotation::Intersection(types)) => {
                    out.extend(types.clone());
                    true
                }
                _ => false,
            }
        }

        let mut members = Vec::new();
        if !push_members(left, &mut members) || !push_members(right, &mut members) {
            return None;
        }

        Some(Type::Concrete(TypeAnnotation::Intersection(members)))
    }

    /// Extract the single element `Type` from an array/Vec-shaped `Type`, in any
    /// of the three representations the inference engine produces
    /// (`Concrete(Array)`, `Concrete(Generic{"Array"|"Vec"})`, or
    /// `Generic{base: Array|Vec, args}`). Returns `None` for non-array shapes.
    fn array_element_type(ty: &Type) -> Option<Type> {
        match ty {
            Type::Concrete(TypeAnnotation::Array(inner)) => Some(Type::Concrete((**inner).clone())),
            Type::Concrete(TypeAnnotation::Generic { name, args })
                if (name == "Array" || name == "Vec") && args.len() == 1 =>
            {
                Some(Type::Concrete(args[0].clone()))
            }
            Type::Generic { base, args }
                if args.len() == 1
                    && matches!(
                        base.as_ref(),
                        Type::Concrete(ann)
                            if matches!(ann.as_type_name_str(), Some("Array") | Some("Vec"))
                    ) =>
            {
                Some(args[0].clone())
            }
            _ => None,
        }
    }

    /// `Array<T> + Array<T> -> Array<T>` concatenation (the book idiom
    /// `weekdays = weekdays + [elem]`, datetime.mdx §Date Range Iteration). The
    /// VM already concatenates arrays; the type checker must accept it without
    /// routing into `infer_numeric_arithmetic_op` (which rejects `Array<int>`
    /// as non-Numeric). Strict: both sides must be arrays and their element
    /// types are unified via a same-type constraint — `Array<int> +
    /// Array<number>` is rejected, no silent element coercion. Returns `None`
    /// when either operand is not an array shape, so genuine numeric/string add
    /// is unaffected.
    fn infer_array_add_type(&mut self, left: &Type, right: &Type, span: Span) -> Option<Type> {
        let left_elem = Self::array_element_type(left)?;
        let right_elem = Self::array_element_type(right)?;

        // Element-type resolution. One side may carry an *unresolved* element —
        // either a live type var or the `"unknown"` sentinel that
        // `Type::to_annotation()` lowers a lost TypeVar to (known constraint,
        // core.rs:218). This is exactly the loop-accumulator idiom `nums = nums
        // + [x]`, where the accumulator's element annotation is not yet pinned
        // when the loop body is inferred. Pushing a strict same-type constraint
        // there would assert `unknown == int` and fail. Instead, when one side
        // is unresolved, adopt the other (concrete) side's element as the
        // result and skip the constraint — no coercion is introduced because
        // the concrete side is the only carrier of a real type. When BOTH sides
        // are concrete we keep the strict agreement constraint (`int !=
        // number`, no silent element coercion).
        let left_unresolved = Self::is_unresolved_array_elem(&left_elem);
        let right_unresolved = Self::is_unresolved_array_elem(&right_elem);
        match (left_unresolved, right_unresolved) {
            (false, false) => {
                // Strict element-type agreement; no coercion (int != number).
                self.push_constraint_with_origin(left_elem.clone(), right_elem, span);
                Some(BuiltinTypes::array(left_elem))
            }
            (true, false) => Some(BuiltinTypes::array(right_elem)),
            (false, true) => Some(BuiltinTypes::array(left_elem)),
            (true, true) => Some(BuiltinTypes::array(left_elem)),
        }
    }

    /// An array element type that carries no committed information: a live type
    /// var, a `Constrained` var, or the `"unknown"` sentinel that
    /// `Type::to_annotation()` lowers a lost TypeVar to (core.rs:218).
    fn is_unresolved_array_elem(ty: &Type) -> bool {
        Self::is_unresolved_var(ty)
            || matches!(
                ty,
                Type::Concrete(ann) if ann.as_type_name_str() == Some("unknown")
            )
    }

    /// Shared numeric arithmetic inference for `+`, `-`, `*`, `/`, `%`.
    fn infer_numeric_arithmetic_op(
        &mut self,
        left: &Type,
        right: &Type,
        span: Span,
    ) -> TypeResult<Type> {
        // Check for Option propagation
        let left_inner = Self::unwrap_option_type(left);
        let right_inner = Self::unwrap_option_type(right);

        let (effective_left, effective_right, is_optional) = match (&left_inner, &right_inner) {
            (Some(l), Some(r)) => (l.clone(), r.clone(), true),
            (Some(l), None) => (l.clone(), right.clone(), true),
            (None, Some(r)) => (left.clone(), r.clone(), true),
            (None, None) => (left.clone(), right.clone(), false),
        };

        // Constrain operands to be numeric (int, float, number, decimal)
        // without forcing to `number` — preserves type specificity
        let left_bound = self.fresh_var();
        self.push_constraint_with_origin(
            effective_left.clone(),
            Type::Constrained {
                var: left_bound,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Numeric".to_string(),
                }),
            },
            span,
        );
        let right_bound = self.fresh_var();
        self.push_constraint_with_origin(
            effective_right.clone(),
            Type::Constrained {
                var: right_bound,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Numeric".to_string(),
                }),
            },
            span,
        );

        // Numeric-conversion §5 value-level invariant: a cross-family mix of a
        // concrete INT-family value and a concrete NUMBER-family value
        // (`int_var + number_var`) is a silent int->number promotion and is
        // FORBIDDEN — both operands must already be the same family; mixing
        // requires an explicit cast. (Literal operands were already adopted into
        // the other operand's type at the `Expr::BinaryOp` seam, so `a:number +
        // 3` is `number + number` here and is unaffected.) Push the directional
        // `(int, number)` same-type constraint so the tightened §2 lattice
        // rejects it. Same-family arithmetic (int+int, i8+i16, number+number)
        // and any var-involving op are untouched — only the concrete
        // cross-family case gets the extra constraint.
        if let (Some(lf), Some(rf)) = (
            Self::concrete_numeric_family(&effective_left),
            Self::concrete_numeric_family(&effective_right),
        ) {
            if lf != rf {
                self.push_constraint_with_origin(
                    effective_left.clone(),
                    effective_right.clone(),
                    span,
                );
            }
        }

        // Compute result type based on operand types
        let result = Self::numeric_result_type(&effective_left, &effective_right);

        if is_optional {
            Ok(Self::wrap_in_option(result))
        } else {
            Ok(result)
        }
    }

    /// The script-level numeric family of a CONCRETE numeric type: `"int"` for
    /// any integer width (incl. `int`/`i64`/`u64`/sized), `"number"` for the
    /// float family (`number`/`f64`/`f32`), `"decimal"` for decimal. Returns
    /// `None` for type vars, non-numeric, or compound types. Used by the §5
    /// cross-family arithmetic-rejection check; deliberately reuses
    /// `canonical_script_alias` (which is the *family* collapse — correct here,
    /// where we only want "are these the same family", not "are these the same
    /// width").
    fn concrete_numeric_family(ty: &Type) -> Option<&'static str> {
        let name = match ty {
            Type::Concrete(TypeAnnotation::Basic(n)) => n.as_str(),
            Type::Concrete(TypeAnnotation::Reference(n)) => n.as_str(),
            _ => return None,
        };
        BuiltinTypes::canonical_script_alias(name)
            .filter(|fam| matches!(*fam, "int" | "number" | "decimal"))
    }

    /// Temporal operand classification for the datetime/duration operator
    /// arithmetic documented in the datetime book chapter. Returns the kind of
    /// a CONCRETE DateTime or Duration (TimeSpan) operand, accepting both the
    /// PascalCase (`DateTime`, `Duration`, `TimeSpan`) forms the compiler's
    /// tracker uses and the lowercase (`datetime`, `duration`, `timespan`)
    /// forms the inference engine stamps on the literals. `None` for any other
    /// type (including type vars) — those flow to the existing numeric path.
    fn temporal_operand_kind(ty: &Type) -> Option<TemporalKind> {
        let name = match ty {
            Type::Concrete(TypeAnnotation::Basic(n)) => n.as_str(),
            Type::Concrete(TypeAnnotation::Reference(n)) => n.as_str(),
            _ => return None,
        };
        match name {
            "DateTime" | "datetime" => Some(TemporalKind::DateTime),
            "Duration" | "TimeSpan" | "duration" | "timespan" => Some(TemporalKind::Duration),
            _ => None,
        }
    }

    /// Result type for the documented DateTime/Duration `+`/`-` operator rules:
    ///
    /// * `DateTime + Duration` / `Duration + DateTime` -> `DateTime`
    /// * `DateTime - Duration` -> `DateTime`
    /// * `DateTime - DateTime` -> `Duration`
    /// * `Duration ± Duration` -> `Duration`
    ///
    /// Duration is NOT Numeric — these rules are separate from the numeric
    /// arithmetic path and introduce no int/number coercion. `None` when the
    /// operand combination is not one of the documented temporal forms (e.g.
    /// `DateTime + DateTime`), which then rejects through the normal path.
    fn temporal_arithmetic_result(
        op: &BinaryOp,
        left: &Type,
        right: &Type,
    ) -> Option<Type> {
        let lk = Self::temporal_operand_kind(left)?;
        let rk = Self::temporal_operand_kind(right)?;
        let datetime = || Type::Concrete(TypeAnnotation::Reference("DateTime".into()));
        let duration = || Type::Concrete(TypeAnnotation::Basic("duration".into()));
        match (op, lk, rk) {
            // DateTime + Duration / Duration + DateTime -> DateTime
            (BinaryOp::Add, TemporalKind::DateTime, TemporalKind::Duration)
            | (BinaryOp::Add, TemporalKind::Duration, TemporalKind::DateTime) => {
                Some(datetime())
            }
            // Duration + Duration -> Duration
            (BinaryOp::Add, TemporalKind::Duration, TemporalKind::Duration) => Some(duration()),
            // DateTime - Duration -> DateTime
            (BinaryOp::Sub, TemporalKind::DateTime, TemporalKind::Duration) => Some(datetime()),
            // DateTime - DateTime -> Duration
            (BinaryOp::Sub, TemporalKind::DateTime, TemporalKind::DateTime) => Some(duration()),
            // Duration - Duration -> Duration
            (BinaryOp::Sub, TemporalKind::Duration, TemporalKind::Duration) => Some(duration()),
            // Any other combination (e.g. DateTime + DateTime,
            // Duration - DateTime) is not a documented form.
            _ => None,
        }
    }

    /// Auto-deref a reference operand for operator inference (finding 9,
    /// ADR-006 §2.7.30). A `Borrow { inner }` operand (`&int`) is read THROUGH
    /// the reference: the referent annotation is forwarded verbatim so
    /// `let r = &x; r + 1` typechecks on `int`, mirroring the already
    /// auto-derefing method dispatch (`r.len()`) and the
    /// `advanced/ownership-deep-dive.mdx` "First-Class References" example
    /// (`let val = r + 1` "reads through r via DerefLoad"). No coercion —
    /// `int`/`number` separation untouched. This relaxes the engine's premature
    /// "Borrow does not implement Numeric" pre-rejection so `r + 1` reaches the
    /// bytecode layer, where the binding-vs-typed-param distinction is decided
    /// from the operand EXPRESSION: a reference-BOUND identifier (`let r = &n`)
    /// auto-derefs via its recorded referent type (`compiler/expressions/mod.rs`
    /// `reference_referent_type_name`); a reference-MODE param (`&p`) already had
    /// a correct deref path. The reference-TYPED-operand R4 rule
    /// (`reference_typed_operand_span`, a separate pre-existing surface) is
    /// orthogonal to this engine helper and unchanged by it.
    fn deref_operand_for_operator(operand: &Type) -> Type {
        if let Type::Concrete(TypeAnnotation::Borrow { inner, .. }) = operand {
            return Self::deref_operand_for_operator(&Type::Concrete((**inner).clone()));
        }
        operand.clone()
    }

    /// Infer type of binary operation
    ///
    /// Supports Option propagation: if either operand is Option<T>, the result is Option<T>.
    pub(crate) fn infer_binary_op(
        &mut self,
        left: &Type,
        op: &BinaryOp,
        right: &Type,
        span: Span,
    ) -> TypeResult<Type> {
        let left = &Self::deref_operand_for_operator(left);
        let right = &Self::deref_operand_for_operator(right);
        match op {
            BinaryOp::Add => {
                // Operator-trait dispatch PRECEDES object-literal merge for a
                // NAMED type that implements `Add`. A struct `Money` with
                // `impl Add for Money` must dispatch to its impl, NOT be
                // structurally merged. The object-literal-merge builtin
                // (`infer_object_add_type` below) is reserved for UNTYPED object
                // literals (`{ x: 1 } + { y: 2 }`); it no longer claims named
                // types (the `Reference` arm was removed), so a `Money + Money`
                // that would previously become `Intersection([Money, Money])`
                // (and then fail to unify with a `-> Money` return) now resolves
                // to the impl's `Money`. MERGE-HIJACK fix (operators slice).
                if let Some(result_type) = self.check_operator_trait(left, "Add") {
                    return Ok(result_type);
                }
                if let Some(merged) = Self::infer_object_add_type(left, right) {
                    return Ok(merged);
                }
                // `Array<T> + Array<T>` is concatenation (book idiom
                // `weekdays = weekdays + [elem]`). Must be handled before the
                // numeric fallback below, which would reject `Array<int>` as
                // non-Numeric.
                if let Some(concatenated) = self.infer_array_add_type(left, right, span) {
                    return Ok(concatenated);
                }
                // String concatenation is allowed in Shape and should not force
                // numeric constraints on the opposite operand.
                if Self::is_string_like(left) || Self::is_string_like(right) {
                    return Ok(BuiltinTypes::string());
                }
                // Temporal operator arithmetic (datetime book chapter):
                // `DateTime + Duration` -> `DateTime`, `Duration + Duration` ->
                // `Duration`. Must run before the numeric fallback, which would
                // reject `Duration` as non-Numeric. Duration is NOT Numeric — no
                // int/number coercion is introduced. The bytecode compiler
                // dispatches these via `CallMethod("add")` (binary_ops.rs).
                if let Some(result) = Self::temporal_arithmetic_result(&BinaryOp::Add, left, right) {
                    return Ok(result);
                }
                // `+` is overloaded (numeric add OR string concat). When BOTH
                // operands are still unresolved type variables there is nothing
                // to disambiguate at body time — committing to a Numeric bound
                // here is the J3 over-constraint (it later rejects a string call
                // site). Defer: yield the left operand var and let
                // callsite-union propagation pin the operands. A CONCRETE
                // numeric on either side (e.g. `c + 1`) still flows to
                // infer_numeric_arithmetic_op below and keeps the genuine
                // Numeric requirement; `-`/`*`/`/`/`%` are numeric-only and are
                // not in this arm, so they are unaffected. A-final ROOT J3.
                if Self::is_unresolved_var(left) && Self::is_unresolved_var(right) {
                    return Ok(left.clone());
                }
                self.infer_numeric_arithmetic_op(left, right, span)
            }
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                // Temporal operator arithmetic (datetime book chapter):
                // `DateTime - Duration` -> `DateTime`, `DateTime - DateTime` ->
                // `Duration`, `Duration - Duration` -> `Duration`. Only `Sub`
                // has temporal forms (`*`/`/`/`%` on temporals are not
                // documented). Runs before the numeric fallback which would
                // reject the non-Numeric `Duration`/`DateTime` operands.
                if matches!(op, BinaryOp::Sub) {
                    if let Some(result) =
                        Self::temporal_arithmetic_result(&BinaryOp::Sub, left, right)
                    {
                        return Ok(result);
                    }
                }
                if matches!(op, BinaryOp::Mul) {
                    if Self::is_mat_number(left) && Self::is_vec_number(right) {
                        return Ok(Self::vec_number_type());
                    }
                    if Self::is_mat_number(left) && Self::is_mat_number(right) {
                        return Ok(Self::mat_number_type());
                    }
                }
                // Operator trait fallback
                let trait_name = match op {
                    BinaryOp::Sub => "Sub",
                    BinaryOp::Mul => "Mul",
                    BinaryOp::Div => "Div",
                    _ => "", // Mod has no operator trait
                };
                if !trait_name.is_empty() {
                    if let Some(result_type) = self.check_operator_trait(left, trait_name) {
                        return Ok(result_type);
                    }
                }
                self.infer_numeric_arithmetic_op(left, right, span)
            }

            BinaryOp::Equal | BinaryOp::NotEqual => {
                // `null`/`None` is comparable to a value of any type — `None == 0`,
                // `x == None`, etc. are legitimate null-presence checks that must
                // not force same-type unification. When exactly one operand is a
                // bare null sentinel (the `None` literal infers as `Option<var>`,
                // or an explicit `Null` annotation), skip the `left ~ right`
                // constraint and yield bool directly.
                //
                // This is narrow on purpose: it does NOT relax equality between
                // two distinct non-null concrete types — `1 == "x"` still pushes
                // the same-type constraint and rejects.
                if Self::is_null_sentinel(left) != Self::is_null_sentinel(right) {
                    return Ok(BuiltinTypes::boolean());
                }
                // Equality can work on any types, but they should be the same
                self.push_constraint_with_origin(left.clone(), right.clone(), span);
                Ok(BuiltinTypes::boolean())
            }

            BinaryOp::Less | BinaryOp::Greater | BinaryOp::LessEq | BinaryOp::GreaterEq => {
                // Comparison operations with Option propagation
                let left_inner = Self::unwrap_option_type(left);
                let right_inner = Self::unwrap_option_type(right);

                let (effective_left, effective_right, is_optional) =
                    match (&left_inner, &right_inner) {
                        (Some(l), Some(r)) => (l.clone(), r.clone(), true),
                        (Some(l), None) => (l.clone(), right.clone(), true),
                        (None, Some(r)) => (left.clone(), r.clone(), true),
                        (None, None) => (left.clone(), right.clone(), false),
                    };

                // Operator trait fallback: a user type that `impl Ord` lowers
                // `<`/`<=`/`>`/`>=` to a `cmp(other) -> int` call followed by an
                // integer comparison against 0 (the bytecode compiler emits this
                // via `operator_trait_for_op`/`emit_cmp_result_comparison`). The
                // strict `Comparable` constraint below only admits built-in
                // scalars, so route the user-type case through `Ord` first.
                // `cmp`'s argument is `other: Self`, so the operands must still
                // be the same type — push that constraint, then yield `bool`.
                if self.check_operator_trait(&effective_left, "Ord").is_some() {
                    self.push_constraint_with_origin(
                        effective_left.clone(),
                        effective_right.clone(),
                        span,
                    );
                    return if is_optional {
                        Ok(Self::wrap_in_option(BuiltinTypes::boolean()))
                    } else {
                        Ok(BuiltinTypes::boolean())
                    };
                }

                self.push_constraint_with_origin(effective_left.clone(), effective_right, span);
                // Add constraint that types must be comparable
                let var = self.fresh_var();
                self.push_constraint_with_origin(
                    effective_left,
                    Type::Constrained {
                        var,
                        constraint: Box::new(TypeConstraint::Comparable),
                    },
                    span,
                );

                if is_optional {
                    // Comparison with Option returns Option<Bool>
                    Ok(Self::wrap_in_option(BuiltinTypes::boolean()))
                } else {
                    Ok(BuiltinTypes::boolean())
                }
            }

            BinaryOp::And | BinaryOp::Or => {
                // Logical operations
                self.push_constraint_with_origin(left.clone(), BuiltinTypes::boolean(), span);
                self.push_constraint_with_origin(right.clone(), BuiltinTypes::boolean(), span);
                Ok(BuiltinTypes::boolean())
            }

            BinaryOp::FuzzyEqual | BinaryOp::FuzzyLess | BinaryOp::FuzzyGreater => {
                // Fuzzy comparison for numbers. Numeric-conversion GREEN Stage 2:
                // `Numeric` BOUND rather than a hard `~ number` so an `int`
                // operand is not rejected by the tightened §2 lattice (parallel
                // to `Pow` and array-index — preserves either-family operands).
                self.push_numeric_operand_bound(left, span);
                self.push_numeric_operand_bound(right, span);
                Ok(BuiltinTypes::boolean())
            }

            BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::BitShl
            | BinaryOp::BitShr => {
                // Operator trait fallback: a user type that `impl BitAnd`
                // (`BitOr`/`BitXor`/`Shl`/`Shr`) lowers the corresponding bitwise
                // operator to a `bitand(other) -> Self` (etc.) call (the bytecode
                // compiler emits this via `operator_trait_for_op`). The strict
                // `int` constraint below only admits integer scalars, so route a
                // user-type operand through its bitwise trait first. The method
                // takes `other: Self`, so the operands must be the same type;
                // the result is `Self` (the left type).
                let trait_name = match op {
                    BinaryOp::BitAnd => "BitAnd",
                    BinaryOp::BitOr => "BitOr",
                    BinaryOp::BitXor => "BitXor",
                    BinaryOp::BitShl => "Shl",
                    BinaryOp::BitShr => "Shr",
                    _ => unreachable!(),
                };
                if let Some(result_type) = self.check_operator_trait(left, trait_name) {
                    self.push_constraint_with_origin(left.clone(), right.clone(), span);
                    return Ok(result_type);
                }
                // Bitwise operations require integer operands
                self.push_constraint_with_origin(left.clone(), BuiltinTypes::integer(), span);
                self.push_constraint_with_origin(right.clone(), BuiltinTypes::integer(), span);
                Ok(BuiltinTypes::integer())
            }

            BinaryOp::Pow => {
                // R3 (USER-RULED 2026-06-01): `**` is family-preserving exactly
                // like `*`/`-`/`/`. int ** int -> int, number ** number ->
                // number; a cross-family concrete mix (int ** number) is a
                // silent int->number promotion and is REJECTED — an explicit
                // cast is required (no loose `can_numeric_widen`). This matches
                // the shipped VM `PowInt`/JIT codegen, which already produce an
                // `int` for an all-int base+exponent. Routing through the shared
                // `infer_numeric_arithmetic_op` gives the identical operand
                // Numeric-bound + §5 cross-family rejection + family-preserving
                // `numeric_result_type` used by the other arithmetic ops, so
                // `2 ** 8` is now `256: int` (was `256.0: number`) and
                // `2.0 ** 8.0` stays `number`. Option propagation is handled by
                // `infer_numeric_arithmetic_op` itself.
                self.infer_numeric_arithmetic_op(left, right, span)
            }

            BinaryOp::NullCoalesce => {
                // Null coalescing `a ?? b`: yields the UNWRAPPED element type
                // `T` of the left operand.
                //   - `Option<T>` / `T?` left → result `T`; the default `b`
                //     must also be `T` (so `Some(5) ?? "x"` is a type error).
                //   - bare (non-Option) left → result is that left type; the
                //     default `b` must unify with it.
                //
                // v0.3.3 book-gate fix: previously this returned `right` with
                // NO constraint between the two operands, so `Some(5) ?? 99`
                // typed as `int` while the runtime leaked `Some(5)`, and a
                // mismatched default like `Some(5) ?? "x"` was silently
                // accepted. The runtime now unwraps `Some(v) -> v` via the
                // `CoalesceProbe` opcode; the static type must match.
                let result_ty = Self::unwrap_option_type(left).unwrap_or_else(|| left.clone());
                // The default `b` must produce the same `T`.
                self.push_constraint_with_origin(right.clone(), result_ty.clone(), span);
                Ok(result_ty)
            }

            BinaryOp::ErrorContext => {
                // Context wrapping always returns Result<SuccessType>.
                // - Result<T> !! ctx -> Result<T>
                // - Option<T>/T? !! ctx -> Result<T>
                // - T !! ctx -> Result<T>
                let success = if let Some(inner) = Self::unwrap_result_or_option_type(left) {
                    inner
                } else if Self::is_unresolved_var(left) {
                    // The left operand is still an unresolved inference variable
                    // (e.g. `g() !! ctx` where `g`'s return type has not been
                    // resolved yet). Wrapping the bare var as `Result<Variable>`
                    // and then unwrapping it via a following `?` would yield the
                    // still-unconstrained var, dropping the success type entirely
                    // (finding 5). Instead, link the operand to `Result<T>` by
                    // pushing a constraint `left = Result<T, AnyError>` with a
                    // fresh success var `T`, and thread `T` through. Once `g`'s
                    // return type resolves, `T` resolves with it — so
                    // `(g() !! ctx)?` yields `g`'s success type.
                    let success_var = self.fresh_type_var();
                    let constrained = self.wrap_in_result(success_var.clone());
                    self.push_constraint_with_origin(left.clone(), constrained, span);
                    success_var
                } else {
                    left.clone()
                };
                Ok(self.wrap_in_result(success))
            }

            BinaryOp::Pipe => {
                // Pipe operator - left is piped into right (which should be a function)
                // Result type is determined by the right side's return type
                // For now, return a new type variable that will be resolved later
                Ok(self.fresh_type_var())
            }
        }
    }

    /// Push a `Numeric`-bound constraint on an operator operand. Both `int` and
    /// `number` (and the numeric widths) satisfy `Numeric`, so this accepts an
    /// operand of either family without forcing it to `number` (which the
    /// tightened §2 lattice would reject for an `int` operand, since `int ->
    /// number` is now CAST-required). Numeric-conversion GREEN Stage 2.
    fn push_numeric_operand_bound(&mut self, operand: &Type, span: Span) {
        let bound = self.fresh_var();
        self.push_constraint_with_origin(
            operand.clone(),
            Type::Constrained {
                var: bound,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Numeric".to_string(),
                }),
            },
            span,
        );
    }

    /// Infer type of unary operation
    ///
    /// Supports Option propagation: if operand is Option<T>, result is Option<ResultType>.
    pub(crate) fn infer_unary_op(&mut self, op: &UnaryOp, operand: &Type) -> TypeResult<Type> {
        let operand = &Self::deref_operand_for_operator(operand);
        let inner = Self::unwrap_option_type(operand);
        let (effective_operand, is_optional) = match &inner {
            Some(t) => (t.clone(), true),
            None => (operand.clone(), false),
        };

        match op {
            UnaryOp::Not => {
                // Operator trait fallback (W1.6): if operand type implements
                // Not, return that type (unary `!` on user types is a UFCS
                // call to `Not::not(self) -> Self`). Sibling of the Neg
                // fallback below.
                if let Some(result_type) = self.check_operator_trait(&effective_operand, "Not") {
                    return if is_optional {
                        Ok(Self::wrap_in_option(result_type))
                    } else {
                        Ok(result_type)
                    };
                }
                self.constraints
                    .push((effective_operand, BuiltinTypes::boolean()));
                if is_optional {
                    Ok(Self::wrap_in_option(BuiltinTypes::boolean()))
                } else {
                    Ok(BuiltinTypes::boolean())
                }
            }
            UnaryOp::Neg => {
                // Operator trait fallback: if operand type implements Neg, return that type
                if let Some(result_type) = self.check_operator_trait(&effective_operand, "Neg") {
                    return if is_optional {
                        Ok(Self::wrap_in_option(result_type))
                    } else {
                        Ok(result_type)
                    };
                }
                // Unary analogue of `numeric_result_type` (the binary precision
                // fix). `-x` must preserve the operand's numeric type instead of
                // unconditionally widening to `number`:
                //   - concrete numeric (int/number/i8/decimal/…) → preserve it
                //     (so `-x` with x:int stays `int`, satisfying a `-> int`
                //     return and `int`/`number` separation);
                //   - unresolved Variable/Constrained → propagate the operand
                //     var (stays call-graph-linked, like the `var <op> concrete`
                //     arm of `numeric_result_type`) so a later callsite resolves
                //     the result with the parameter, instead of collapsing to
                //     `number` and severing the link;
                //   - non-numeric concrete (e.g. `-"s"`) → keep the `== number`
                //     constraint so it still rejects.
                let result = match &effective_operand {
                    Type::Concrete(TypeAnnotation::Basic(name))
                        if BuiltinTypes::is_numeric_type_name(name) =>
                    {
                        // Concrete numeric: validate via the Numeric trait bound
                        // (accepts int/number/i8/…, rejects non-numeric) without
                        // forcing `number`, then preserve the operand's type.
                        let bound = self.fresh_var();
                        self.constraints.push((
                            effective_operand.clone(),
                            Type::Constrained {
                                var: bound,
                                constraint: Box::new(TypeConstraint::ImplementsTrait {
                                    trait_name: "Numeric".to_string(),
                                }),
                            },
                        ));
                        effective_operand.clone()
                    }
                    Type::Variable(_) | Type::Constrained { .. } => {
                        // Unresolved: constrain to Numeric (keeps it linked to the
                        // call graph) and propagate the operand var as the result.
                        let bound = self.fresh_var();
                        self.constraints.push((
                            effective_operand.clone(),
                            Type::Constrained {
                                var: bound,
                                constraint: Box::new(TypeConstraint::ImplementsTrait {
                                    trait_name: "Numeric".to_string(),
                                }),
                            },
                        ));
                        effective_operand.clone()
                    }
                    // Non-numeric concrete (or other) → keep the strict `== number`
                    // constraint so genuinely-bad operands like `-"s"` reject.
                    _ => {
                        self.constraints
                            .push((effective_operand.clone(), BuiltinTypes::number()));
                        BuiltinTypes::number()
                    }
                };
                if is_optional {
                    Ok(Self::wrap_in_option(result))
                } else {
                    Ok(result)
                }
            }
            UnaryOp::BitNot => {
                self.constraints
                    .push((effective_operand, BuiltinTypes::integer()));
                if is_optional {
                    Ok(Self::wrap_in_option(BuiltinTypes::integer()))
                } else {
                    Ok(BuiltinTypes::integer())
                }
            }
        }
    }

    /// Check if a type implements an operator trait (Add, Sub, Mul, Div, Neg, Eq, Ord).
    /// If so, returns the result type (the operand type itself for Self-returning traits).
    fn check_operator_trait(&self, operand_type: &Type, trait_name: &str) -> Option<Type> {
        let type_name = match operand_type {
            Type::Concrete(ann) => ann.as_type_name_str()?,
            _ => return None,
        };
        // Skip primitive/numeric types — they use the built-in arithmetic path
        if BuiltinTypes::is_numeric_type_name(type_name)
            || type_name == "string"
            || type_name == "bool"
        {
            return None;
        }
        if self.env.type_implements_trait(type_name, trait_name) {
            Some(operand_type.clone())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic(name: &str) -> Type {
        Type::Concrete(TypeAnnotation::Basic(name.to_string()))
    }

    #[test]
    fn test_mixed_width_int_arithmetic_stays_int() {
        // Strict-flip root R5: `i8 + i16` (mixed integer widths) must produce
        // `int`, not `number`, so it satisfies a `-> int` return. Previously
        // the mismatched-name path widened any mixed concrete pair to `number`.
        for (l, r) in [("i8", "i16"), ("u8", "u32"), ("i32", "i8"), ("i8", "int")] {
            let result = TypeInferenceEngine::numeric_result_type(&basic(l), &basic(r));
            assert_eq!(
                result,
                basic("int"),
                "{l} + {r} should stay `int`, got {result:?}"
            );
        }
    }

    #[test]
    fn test_int_plus_float_widens_to_number() {
        // Cross-family mixes still widen to `number` — int/number stay distinct.
        for (l, r) in [("i8", "f64"), ("int", "number"), ("i16", "f32")] {
            let result = TypeInferenceEngine::numeric_result_type(&basic(l), &basic(r));
            assert_eq!(
                result,
                BuiltinTypes::number(),
                "{l} + {r} should widen to `number`, got {result:?}"
            );
        }
    }

    #[test]
    fn test_unwrap_option_generic() {
        let option_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Option".into()))),
            args: vec![BuiltinTypes::number()],
        };
        let inner = TypeInferenceEngine::unwrap_option_type(&option_num);
        assert!(inner.is_some());
        assert_eq!(inner.unwrap(), BuiltinTypes::number());
    }

    #[test]
    fn test_unwrap_option_annotation() {
        let option_num = Type::Concrete(TypeAnnotation::Generic {
            name: "Option".into(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        });
        let inner = TypeInferenceEngine::unwrap_option_type(&option_num);
        assert!(inner.is_some());
    }

    #[test]
    fn test_unwrap_non_option() {
        let num = BuiltinTypes::number();
        let inner = TypeInferenceEngine::unwrap_option_type(&num);
        assert!(inner.is_none());
    }

    #[test]
    fn test_wrap_in_option() {
        let num = BuiltinTypes::number();
        let wrapped = TypeInferenceEngine::wrap_in_option(num);
        assert!(matches!(wrapped, Type::Generic { .. }));

        // Verify it's Option<number>
        let unwrapped = TypeInferenceEngine::unwrap_option_type(&wrapped);
        assert!(unwrapped.is_some());
    }

    #[test]
    fn test_null_sentinel_eq_skips_same_type_constraint() {
        // `None == 0` / `0 == None`: the `None` literal infers as `Option<var>`,
        // a null sentinel. Equality against it must NOT push a `left ~ right`
        // same-type constraint (null is comparable to any value), and yields bool.
        for left_is_none in [true, false] {
            let mut engine = TypeInferenceEngine::new();
            let none = TypeInferenceEngine::wrap_in_option(engine.fresh_type_var());
            let (l, r) = if left_is_none {
                (none, basic("int"))
            } else {
                (basic("int"), none)
            };
            let before = engine.constraints.len();
            let inferred = engine
                .infer_binary_op(&l, &BinaryOp::Equal, &r, Span::DUMMY)
                .expect("null-sentinel equality should infer");
            assert_eq!(inferred, BuiltinTypes::boolean());
            assert_eq!(
                engine.constraints.len(),
                before,
                "null-sentinel `==` must not push a same-type unification constraint \
                 ({l:?} == {r:?})"
            );
        }
        // Explicit `Null` annotation is also a sentinel.
        let mut engine = TypeInferenceEngine::new();
        let before = engine.constraints.len();
        engine
            .infer_binary_op(
                &Type::Concrete(TypeAnnotation::Null),
                &BinaryOp::NotEqual,
                &basic("string"),
                Span::DUMMY,
            )
            .expect("explicit-Null inequality should infer");
        assert_eq!(engine.constraints.len(), before);
    }

    #[test]
    fn test_distinct_concrete_eq_still_constrains_and_rejects() {
        // NOT broad suppression: two distinct non-null concrete types (`1 == "x"`)
        // must STILL push the same-type unification constraint, which the solver
        // then rejects. `None == None` also keeps constraining (both sentinels →
        // condition is false), so `Option<var> ~ Option<var>` is still pushed.
        {
            let mut engine = TypeInferenceEngine::new();
            let before = engine.constraints.len();
            engine
                .infer_binary_op(
                    &basic("int"),
                    &BinaryOp::Equal,
                    &basic("string"),
                    Span::DUMMY,
                )
                .expect("equality should infer");
            assert_eq!(
                engine.constraints.len(),
                before + 1,
                "`int == string` must still push the same-type constraint"
            );
        }
        {
            let mut engine = TypeInferenceEngine::new();
            let none_l = TypeInferenceEngine::wrap_in_option(engine.fresh_type_var());
            let none_r = TypeInferenceEngine::wrap_in_option(engine.fresh_type_var());
            let before = engine.constraints.len();
            engine
                .infer_binary_op(&none_l, &BinaryOp::Equal, &none_r, Span::DUMMY)
                .expect("None == None should infer");
            assert_eq!(
                engine.constraints.len(),
                before + 1,
                "two-sentinel `None == None` must still push the same-type constraint"
            );
        }

        // And the genuine mismatch `int ~ string` is actually rejected by the solver.
        let mut engine = TypeInferenceEngine::new();
        engine
            .infer_binary_op(
                &basic("int"),
                &BinaryOp::Equal,
                &basic("string"),
                Span::DUMMY,
            )
            .expect("inference itself succeeds; the constraint is what fails");
        assert!(
            engine.solver.solve(&mut engine.constraints).is_err(),
            "`int == string` must still be a type error after the null-eq fix"
        );
    }

    #[test]
    fn test_error_context_promotes_option_to_result() {
        let mut engine = TypeInferenceEngine::new();
        let option_num = Type::Concrete(TypeAnnotation::Generic {
            name: "Option".into(),
            args: vec![TypeAnnotation::Basic("number".to_string())],
        });
        let inferred = engine
            .infer_binary_op(
                &option_num,
                &BinaryOp::ErrorContext,
                &BuiltinTypes::string(),
                Span::DUMMY,
            )
            .expect("option !! context should infer");

        let expected = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Result".into()))),
            args: vec![
                BuiltinTypes::number(),
                Type::Concrete(TypeAnnotation::Reference("AnyError".into())),
            ],
        };
        assert_eq!(inferred, expected);
    }

    #[test]
    fn test_error_context_keeps_result_inner_type() {
        let mut engine = TypeInferenceEngine::new();
        let result_num = Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference("Result".into()))),
            args: vec![
                BuiltinTypes::number(),
                Type::Concrete(TypeAnnotation::Reference("AnyError".into())),
            ],
        };
        let inferred = engine
            .infer_binary_op(
                &result_num,
                &BinaryOp::ErrorContext,
                &BuiltinTypes::string(),
                Span::DUMMY,
            )
            .expect("result !! context should infer");
        assert_eq!(inferred, result_num);
    }

    #[test]
    fn test_infer_literal_formatted_string_is_string() {
        let mut engine = TypeInferenceEngine::new();
        let inferred = engine
            .infer_literal(&Literal::FormattedString {
                value: "x={x}".to_string(),
                mode: shape_ast::ast::InterpolationMode::Braces,
            })
            .expect("formatted string literal should infer");
        assert_eq!(inferred, BuiltinTypes::string());
    }

    #[test]
    fn test_infer_literal_none_is_option_not_null() {
        let mut engine = TypeInferenceEngine::new();
        let inferred = engine
            .infer_literal(&Literal::None)
            .expect("None literal should infer");

        match inferred {
            Type::Generic { base, args } => {
                assert!(
                    matches!(
                        base.as_ref(),
                        Type::Concrete(TypeAnnotation::Reference(name)) if name == "Option"
                    ),
                    "None must infer as Option<T>, got {:?}",
                    base
                );
                assert_eq!(args.len(), 1, "Option must have exactly one type argument");
                assert!(
                    !matches!(&args[0], Type::Concrete(TypeAnnotation::Null)),
                    "None must not infer as null"
                );
            }
            other => panic!("expected Option<T> for None, got {:?}", other),
        }
    }

    #[test]
    fn test_add_object_types_produces_intersection() {
        let mut engine = TypeInferenceEngine::new();
        let left = Type::Concrete(TypeAnnotation::Object(vec![
            shape_ast::ast::ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
        ]));
        let right = Type::Concrete(TypeAnnotation::Object(vec![
            shape_ast::ast::ObjectTypeField {
                name: "z".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
        ]));

        let inferred = engine
            .infer_binary_op(&left, &BinaryOp::Add, &right, Span::DUMMY)
            .expect("object + object should infer");

        assert!(
            matches!(inferred, Type::Concrete(TypeAnnotation::Intersection(_))),
            "expected intersection type, got {:?}",
            inferred
        );
    }
}
