//! Type Constraint Solver
//!
//! Solves type constraints generated during type inference to determine
//! concrete types for type variables. The solver operates in three phases:
//!
//! ## Phase 1: Eager unification
//!
//! Each constraint `(T1, T2)` is attempted immediately via `solve_constraint`.
//! Simple bindings (variable-to-concrete, variable-to-variable) succeed here.
//! Constraints that fail (e.g. because a variable is not yet resolved) are
//! deferred to the next phase.
//!
//! ## Phase 2: Fixed-point iteration on deferred constraints
//!
//! Deferred constraints are retried in a loop. Each successful resolution may
//! unlock further deferred constraints by refining substitutions. The loop
//! terminates when a full pass makes no progress. Any constraints still
//! unsolved after the fixed-point are reported as `UnsolvedConstraints`.
//!
//! ## Phase 3: Bound application
//!
//! After all equality constraints are resolved, `apply_bounds` validates
//! type variable bounds (`Numeric`, `Comparable`, `Iterable`, `HasField`,
//! `HasMethod`, `ImplementsTrait`). `HasField` constraints additionally
//! perform backward propagation: when a structural object field is found,
//! the field's result type variable is bound to the actual field type.
//!
//! The solver delegates low-level variable binding and substitution to the
//! `Unifier` (Robinson's algorithm with path compression).

use super::checking::MethodTable;
use super::unification::Unifier;
use super::*;
use shape_ast::ast::{ObjectTypeField, TypeAnnotation};
use std::collections::{HashMap, HashSet};

/// The exactly-representable value domain of a fixed-width numeric type, used
/// by the §2 lossless-implicit lattice (numeric-conversion-spec).
///
/// Integers carry their full `[lo, hi]` value range as `i128` (wide enough for
/// the `u64`/`i64` extremes). Floats (`number`/f64, `f32`) carry their
/// exact-integer range (`[-2^53, 2^53]` / `[-2^24, 2^24]`) and set `is_float`.
///
/// `src` is LOSSLESS-IMPLICIT into `dst` iff `src`'s range is a subset of
/// `dst`'s exactly-representable range AND the source/destination float-ness is
/// compatible (an integer source may widen into a float destination whose
/// exact-integer range contains it; a float source may only flow into another
/// float — never silently into an integer).
#[derive(Clone, Copy)]
struct NumericDomain {
    lo: i128,
    hi: i128,
    is_float: bool,
}

impl NumericDomain {
    /// Whether every value of `self` is exactly representable in `dst`.
    fn is_subset_of(&self, dst: &NumericDomain) -> bool {
        if self.is_float && !dst.is_float {
            // A float source is never silently an integer (number -> int is
            // CAST-required in both the spec and THE RULE).
            return false;
        }
        // Integer -> float (dst.is_float, !self.is_float): legal iff the whole
        // integer range fits the float exact-integer range. Float -> float and
        // integer -> integer: legal iff the range is a subset. The single
        // range-subset test covers all three cases since the float domain's
        // `[lo, hi]` IS its exact-integer range.
        dst.lo <= self.lo && self.hi <= dst.hi
    }
}

/// Check if a Type::Generic base is "Array" or "Vec".
fn is_array_or_vec_base(base: &Type) -> bool {
    match base {
        Type::Concrete(TypeAnnotation::Reference(name)) => name == "Array" || name == "Vec",
        Type::Concrete(TypeAnnotation::Basic(name)) => name == "Array" || name == "Vec",
        _ => false,
    }
}

/// Collapse a degenerate `Union` to a single member when all its members are
/// structurally equal (which includes the single-element `Union([T])` case).
///
/// Match arms whose branches all yield the same type combine into a
/// `Union([T])` / `Union([T, T, ...])` during inference (e.g. `match { Some(v)
/// => v, None => 0 }` where both arms are `int`). Such a union is just `T`, but
/// the trait-bound check would otherwise stringify it (`Union([Basic("int")])`)
/// and fail `Numeric`. A genuinely heterogeneous union (`Union([int, string])`)
/// is left intact and continues to fail single-type trait bounds correctly.
fn collapse_degenerate_union(ann: &TypeAnnotation) -> &TypeAnnotation {
    if let TypeAnnotation::Union(members) = ann {
        if let Some(first) = members.first() {
            if members.iter().all(|m| m == first) {
                return first;
            }
        }
    }
    ann
}

pub struct ConstraintSolver {
    /// Type unifier
    unifier: Unifier,
    /// Deferred constraints that couldn't be solved immediately.
    /// These are handled in solve() via multiple passes.
    _deferred: Vec<(Type, Type)>,
    /// Type variable bounds
    bounds: HashMap<TypeVar, TypeConstraint>,
    /// Method table for HasMethod constraint enforcement
    method_table: Option<MethodTable>,
    /// Trait implementation registry: set of "TraitName::TypeName" keys
    trait_impls: HashSet<String>,
    /// Named-struct field schemas: struct name → its structural object fields.
    /// Lets the solver unify a nominal struct type (`Point`) with the
    /// structural object type that flows in (`{ x: number, y: number }`).
    struct_schemas: HashMap<String, Vec<ObjectTypeField>>,
}

impl Default for ConstraintSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstraintSolver {
    pub fn new() -> Self {
        ConstraintSolver {
            unifier: Unifier::new(),
            _deferred: Vec::new(),
            bounds: HashMap::new(),
            method_table: None,
            trait_impls: HashSet::new(),
            struct_schemas: HashMap::new(),
        }
    }

    /// Attach a method table for HasMethod constraint enforcement.
    /// When set, HasMethod constraints are validated against this table
    /// instead of being accepted unconditionally.
    pub fn set_method_table(&mut self, table: MethodTable) {
        self.method_table = Some(table);
    }

    /// Register trait implementations for ImplementsTrait constraint enforcement.
    /// Each entry is a "TraitName::TypeName" key indicating that TypeName implements TraitName.
    pub fn set_trait_impls(&mut self, impls: HashSet<String>) {
        self.trait_impls = impls;
    }

    /// Register named-struct field schemas so a nominal struct type can unify
    /// with the structural object type that flows into it (and vice versa).
    /// Each entry maps a struct name to its declared fields as
    /// `ObjectTypeField`s — the same shape produced for an object literal.
    pub fn set_struct_schemas(&mut self, schemas: HashMap<String, Vec<ObjectTypeField>>) {
        self.struct_schemas = schemas;
    }

    /// Resolve a named struct's structural fields, if it is a registered
    /// struct schema. Returns `None` for non-struct names (builtins, enums,
    /// type aliases, unknown types) so those keep their existing behaviour.
    fn struct_fields(&self, name: &str) -> Option<&[ObjectTypeField]> {
        self.struct_schemas.get(name).map(|v| v.as_slice())
    }

    /// Unify a structural object's fields against a named struct's declared
    /// fields. Only succeeds when `name` is a *registered struct schema*;
    /// otherwise returns `Ok(false)` so non-struct references keep their
    /// existing (non-unifying) behaviour. Field comparison is exact
    /// (`object_fields_compatible`), so a structurally wrong object still
    /// fails to unify and the program is still correctly rejected.
    fn unify_object_with_named_struct(
        &self,
        obj_fields: &[ObjectTypeField],
        name: &str,
    ) -> TypeResult<bool> {
        match self.struct_fields(name) {
            Some(struct_fields) => {
                // Clone the resolved fields to drop the borrow on `self`
                // before the recursive `&self` call in object_fields_compatible.
                let struct_fields = struct_fields.to_vec();
                self.object_fields_compatible(obj_fields, &struct_fields)
            }
            None => Ok(false),
        }
    }

    /// Solve all type constraints
    pub fn solve(&mut self, constraints: &mut Vec<(Type, Type)>) -> TypeResult<()> {
        // First pass: solve simple unification constraints
        let mut unsolved = Vec::new();

        for (t1, t2) in constraints.drain(..) {
            if self.solve_constraint(t1.clone(), t2.clone()).is_err() {
                // If we can't solve it now, defer it
                unsolved.push((t1, t2));
            }
        }

        // Second pass: try deferred constraints
        let mut made_progress = true;
        while made_progress && !unsolved.is_empty() {
            made_progress = false;
            let mut still_unsolved = Vec::new();

            for (t1, t2) in unsolved.drain(..) {
                if self.solve_constraint(t1.clone(), t2.clone()).is_err() {
                    still_unsolved.push((t1, t2));
                } else {
                    made_progress = true;
                }
            }

            unsolved = still_unsolved;
        }

        // Check if any constraints remain unsolved
        if !unsolved.is_empty() {
            return Err(TypeError::UnsolvedConstraints(unsolved));
        }

        // Apply bounds to type variables
        self.apply_bounds()?;

        Ok(())
    }

    /// Solve a single constraint
    fn solve_constraint(&mut self, t1: Type, t2: Type) -> TypeResult<()> {
        // Apply current substitutions before matching to avoid overwriting
        // existing bindings (e.g., T17=string overwritten by T17=T19 during
        // Function param/return pairwise unification).
        let t1 = self.unifier.apply_substitutions(&t1);
        let t2 = self.unifier.apply_substitutions(&t2);

        match (&t1, &t2) {
            // Variable constraints
            (Type::Variable(v1), Type::Variable(v2)) if v1 == v2 => Ok(()),

            // Constrained type variables — must be matched BEFORE the general
            // Variable arm, otherwise (Variable, Constrained) pairs are caught
            // by the Variable arm and the bound is never recorded.
            (Type::Constrained { var, constraint }, ty)
            | (ty, Type::Constrained { var, constraint }) => {
                // Record the constraint
                self.bounds.insert(var.clone(), *constraint.clone());

                // Unify with the underlying type
                self.solve_constraint(Type::Variable(var.clone()), ty.clone())
            }

            (Type::Variable(var), ty) | (ty, Type::Variable(var)) => {
                // Check occurs check
                if self.occurs_in(var, ty) {
                    return Err(TypeError::InfiniteType(var.clone()));
                }

                self.unifier.bind(var.clone(), ty.clone());
                Ok(())
            }

            // Concrete type constraints
            (Type::Concrete(ann1), Type::Concrete(ann2)) => {
                if self.unify_annotations(ann1, ann2)? {
                    // `unify_annotations` accepts identity plus the directional
                    // §2 lossless-widening lattice (`lossless_implicit(ann1=src,
                    // ann2=dst)`). Every non-subset numeric pair — int<->number
                    // both ways, lossy narrowing, sign reinterpretation,
                    // int(i64)/u64 -> number — falls through to the mismatch.
                    Ok(())
                } else if Self::is_any_error(ann1) || Self::is_any_error(ann2) {
                    // `AnyError` is the top of the error lattice: the default `E`
                    // for bare `Ok(..)`/`?`. It unifies with any concrete error
                    // type (e.g. `Result<int, AnyError> ~ Result<T, string>`).
                    // Bounded to `AnyError` so two distinct concrete named error
                    // types still mismatch.
                    Ok(())
                } else {
                    Err(TypeError::TypeMismatch(
                        format!("{:?}", ann1),
                        format!("{:?}", ann2),
                    ))
                }
            }

            // Generic type constraints
            (Type::Generic { base: b1, args: a1 }, Type::Generic { base: b2, args: a2 }) => {
                self.solve_constraint(*b1.clone(), *b2.clone())?;

                let is_result_base = |base: &Type| match base {
                    Type::Concrete(TypeAnnotation::Reference(name)) => name == "Result",
                    Type::Concrete(TypeAnnotation::Basic(name)) => name == "Result",
                    _ => false,
                };

                if a1.len() != a2.len() {
                    if is_result_base(&b1) && is_result_base(&b2) {
                        match (a1.len(), a2.len()) {
                            // `Result<T>` is error-agnostic shorthand and should unify
                            // with `Result<T, E>` by constraining only the success type.
                            (1, 2) | (2, 1) => {
                                self.solve_constraint(a1[0].clone(), a2[0].clone())?;
                                return Ok(());
                            }
                            _ => return Err(TypeError::ArityMismatch(a1.len(), a2.len())),
                        }
                    } else {
                        return Err(TypeError::ArityMismatch(a1.len(), a2.len()));
                    }
                }

                for (arg1, arg2) in a1.iter().zip(a2.iter()) {
                    self.solve_constraint(arg1.clone(), arg2.clone())?;
                }

                Ok(())
            }

            // Function ~ Function: pairwise unify params + returns
            (
                Type::Function {
                    params: p1,
                    returns: r1,
                },
                Type::Function {
                    params: p2,
                    returns: r2,
                },
            ) => {
                if p1.len() != p2.len() {
                    return Err(TypeError::ArityMismatch(p1.len(), p2.len()));
                }
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    // Parameter compatibility is checked from observed/actual to
                    // declared/expected shape so directional numeric widening
                    // (e.g. int -> number) remains valid in call constraints.
                    self.solve_constraint(param2.clone(), param1.clone())?;
                }
                self.solve_constraint(*r1.clone(), *r2.clone())
            }

            // Cross-compatibility: Type::Function ~ Concrete(TypeAnnotation::Function)
            (
                Type::Function {
                    params: fp,
                    returns: fr,
                },
                Type::Concrete(TypeAnnotation::Function {
                    params: cp,
                    returns: cr,
                }),
            )
            | (
                Type::Concrete(TypeAnnotation::Function {
                    params: cp,
                    returns: cr,
                }),
                Type::Function {
                    params: fp,
                    returns: fr,
                },
            ) => {
                if fp.len() != cp.len() {
                    return Err(TypeError::ArityMismatch(fp.len(), cp.len()));
                }
                for (f_param, c_param) in fp.iter().zip(cp.iter()) {
                    self.solve_constraint(
                        f_param.clone(),
                        Type::Concrete(c_param.type_annotation.clone()),
                    )?;
                }
                self.solve_constraint(*fr.clone(), Type::Concrete(*cr.clone()))
            }

            // Array<T> (Type::Generic with base "Array" or "Vec") ~ Concrete(Array(T))
            (Type::Generic { base, args }, Type::Concrete(TypeAnnotation::Array(elem)))
            | (Type::Concrete(TypeAnnotation::Array(elem)), Type::Generic { base, args })
                if args.len() == 1 && is_array_or_vec_base(base) =>
            {
                self.solve_constraint(args[0].clone(), Type::Concrete((**elem).clone()))
            }

            _ => Err(TypeError::TypeMismatch(
                format!("{:?}", t1),
                format!("{:?}", t2),
            )),
        }
    }

    /// Check if a type variable occurs in a type (occurs check)
    fn occurs_in(&self, var: &TypeVar, ty: &Type) -> bool {
        match ty {
            Type::Variable(v) => v == var,
            Type::Generic { base, args } => {
                self.occurs_in(var, base) || args.iter().any(|arg| self.occurs_in(var, arg))
            }
            Type::Constrained { var: v, .. } => v == var,
            Type::Function { params, returns } => {
                params.iter().any(|p| self.occurs_in(var, p)) || self.occurs_in(var, returns)
            }
            Type::Concrete(_) => false,
        }
    }

    /// The numeric-conversion lossless lattice (numeric-conversion-spec §2).
    ///
    /// An ordered pair `(src, dst)` is **LOSSLESS-IMPLICIT** iff the entire
    /// value range of `src` is a subset of the values exactly representable in
    /// `dst`. This is the *only* implicit numeric conversion THE RULE (user
    /// 2026-06-01) permits: every non-subset pair — `int <-> number` both
    /// directions, lossy width narrowing (`u16 -> u8`, `i64 -> i32`), sign
    /// reinterpretation (`i16 -> u16`, `u8 -> i8`), `int(i64) -> number`,
    /// `u64 -> number`, `number -> int` — is CAST-REQUIRED.
    ///
    /// This replaces the two prior loose relaxations:
    /// - `can_numeric_widen` (accepted *every* integer -> *any* float, so the
    ///   lossy `int(i64) -> number` value-promotion silently passed), and
    /// - `same_canonical_numeric_type` (collapsed all integer widths to a single
    ///   `"int"` alias, so `u16 ~ u8` unified with no cast and silently wrapped
    ///   300 -> 44).
    ///
    /// Identity (`src == dst`, after canonicalization) is trivially lossless.
    ///
    /// `decimal`/`bigint` are arbitrary/exact-precision heap types, NOT part of
    /// the fixed-width lossless lattice: conversions to/from them are always an
    /// explicit `as`-cast and are not accepted here (returns `false`).
    fn lossless_implicit_names(src: &str, dst: &str) -> bool {
        match (Self::numeric_domain(src), Self::numeric_domain(dst)) {
            (Some(s), Some(d)) => s.is_subset_of(&d),
            _ => false,
        }
    }

    /// Directional lossless-implicit check on annotations (src widens to dst).
    fn lossless_implicit(src: &TypeAnnotation, dst: &TypeAnnotation) -> bool {
        match (Self::annotation_name(src), Self::annotation_name(dst)) {
            (Some(s), Some(d)) => Self::lossless_implicit_names(s, d),
            _ => false,
        }
    }

    /// Resolve a numeric type name to its exactly-representable value domain.
    ///
    /// Integers carry their `[lo, hi]` value range (as `i128`, wide enough for
    /// the full `u64`/`i64` range). `number`/`f32` carry the f64/f32
    /// exact-integer range `[-2^53, 2^53]` / `[-2^24, 2^24]` plus a `float`
    /// flag (a float domain is a *strict superset destination* only for integer
    /// sources whose whole range fits — and for float->float widening). Returns
    /// `None` for non-fixed-width-numeric names (`decimal`, named types, ...).
    fn numeric_domain(name: &str) -> Option<NumericDomain> {
        // Width names with a concrete [min, max] range.
        if let Some(w) = shape_ast::IntWidth::from_name(name) {
            let (lo, hi) = if w.is_signed() {
                (w.min_value() as i128, w.max_value() as i128)
            } else {
                (0i128, w.max_unsigned() as i128)
            };
            return Some(NumericDomain {
                lo,
                hi,
                is_float: false,
            });
        }
        // The script primitives + aliases not covered by IntWidth.
        match BuiltinTypes::canonical_numeric_runtime_name(name) {
            // int / i64 — the default 64-bit signed integer.
            Some("i64") => Some(NumericDomain {
                lo: i64::MIN as i128,
                hi: i64::MAX as i128,
                is_float: false,
            }),
            // isize/usize are platform-width; treat as i64/u64 on 64-bit
            // targets (spec §7 OD-4, convention adopted).
            Some("isize") => Some(NumericDomain {
                lo: i64::MIN as i128,
                hi: i64::MAX as i128,
                is_float: false,
            }),
            Some("usize") => Some(NumericDomain {
                lo: 0,
                hi: u64::MAX as i128,
                is_float: false,
            }),
            // number / f64 — exactly represents every integer in [-2^53, 2^53].
            Some("f64") => Some(NumericDomain {
                lo: -(1i128 << 53),
                hi: 1i128 << 53,
                is_float: true,
            }),
            // f32 — exactly represents every integer in [-2^24, 2^24].
            Some("f32") => Some(NumericDomain {
                lo: -(1i128 << 24),
                hi: 1i128 << 24,
                is_float: true,
            }),
            _ => None,
        }
    }

    /// Extract the bare name from a `Basic` / `Reference` annotation (the only
    /// shapes a primitive numeric alias can take). Returns `None` for compound
    /// annotations.
    fn annotation_name(ann: &TypeAnnotation) -> Option<&str> {
        match ann {
            TypeAnnotation::Basic(n) => Some(n.as_str()),
            // `TypePath` derefs to its qualified string; numeric aliases are
            // always single-segment, so this yields e.g. `"i8"`.
            TypeAnnotation::Reference(p) => Some(&**p),
            _ => None,
        }
    }

    /// Whether an annotation is the `AnyError` top of the error lattice.
    ///
    /// `AnyError` is the default error type the compiler stamps for bare
    /// `Ok(..)`/`?` (env/mod.rs and operators.rs) when no concrete error type
    /// is in scope. It is the top of the error sub-lattice: it unifies with any
    /// concrete error type in the error-arg position of `Result<T, E>`. This is
    /// bounded to the `AnyError` name specifically — two *distinct concrete*
    /// named error types still mismatch (no broad suppression).
    fn is_any_error(ann: &TypeAnnotation) -> bool {
        matches!(Self::annotation_name(ann), Some("AnyError"))
    }

    /// Whether `ann1` losslessly widens to `ann2` per the §2 numeric lattice
    /// (directional, `(src, dst)`-ordered). Identity is handled by the caller's
    /// `ann1 == ann2` check; this is purely the proper-widening relation.
    fn annotations_same_numeric(ann1: &TypeAnnotation, ann2: &TypeAnnotation) -> bool {
        Self::lossless_implicit(ann1, ann2)
    }

    /// Unify two type annotations
    fn unify_annotations(&self, ann1: &TypeAnnotation, ann2: &TypeAnnotation) -> TypeResult<bool> {
        // `AnyError` is the top of the error lattice (see `is_any_error`): it
        // unifies with any type in the error-arg position. Bounded to the
        // `AnyError` name so two distinct concrete named error types still fail.
        if Self::is_any_error(ann1) || Self::is_any_error(ann2) {
            return Ok(true);
        }
        match (ann1, ann2) {
            // Basic types. Numeric pairs unify implicitly ONLY when `ann1`
            // (the src/value side) losslessly widens to `ann2` (the dst side)
            // per the §2 lattice — `annotations_same_numeric` is now exactly
            // `lossless_implicit(ann1, ann2)`. Identity stays `ann1 == ann2`.
            (TypeAnnotation::Basic(_), TypeAnnotation::Basic(_)) => {
                Ok(ann1 == ann2 || Self::annotations_same_numeric(ann1, ann2))
            }
            (TypeAnnotation::Reference(n1), TypeAnnotation::Reference(n2)) => {
                Ok(n1 == n2 || Self::annotations_same_numeric(ann1, ann2))
            }
            // Combined (merge r3 enum-nominal + r5 width-int): a `Basic` name
            // and a `Reference` path denote the same nominal type when their
            // names agree (`Color` carried as Basic at decl vs Reference at call
            // site); OR `ann1` losslessly widens to `ann2` per the §2 lattice.
            // Distinct names (`Color` vs `Dir`) still fail.
            (TypeAnnotation::Basic(_), TypeAnnotation::Reference(_))
            | (TypeAnnotation::Reference(_), TypeAnnotation::Basic(_)) => {
                let names_match = matches!(
                    (ann1.as_type_name_str(), ann2.as_type_name_str()),
                    (Some(n1), Some(n2)) if n1 == n2
                );
                Ok(names_match || Self::annotations_same_numeric(ann1, ann2))
            }

            // Borrow types (R1/GAP-2): `&T` / `&mut T`. A distinct constructor:
            // never unwrapped on one side (a bare `int` does NOT unify with
            // `&int`, which keeps the value/reference distinction intact).
            // Mutability must match exactly, and the inner annotation must be
            // structurally equal — references do NOT participate in the §2
            // numeric-widening lattice (`&int` is not a `&number`), so the
            // inner is compared via exact `annotations_equal`, mirroring the
            // unifier's `try_unify` Borrow path.
            (
                TypeAnnotation::Borrow {
                    mutable: m1,
                    inner: i1,
                },
                TypeAnnotation::Borrow {
                    mutable: m2,
                    inner: i2,
                },
            ) => Ok(m1 == m2 && crate::type_system::unification::annotations_equal(i1, i2)),

            // Array types
            (TypeAnnotation::Array(e1), TypeAnnotation::Array(e2)) => {
                self.unify_annotations(e1, e2)
            }

            // Tuple types
            (TypeAnnotation::Tuple(t1), TypeAnnotation::Tuple(t2)) => {
                if t1.len() != t2.len() {
                    return Ok(false);
                }

                for (elem1, elem2) in t1.iter().zip(t2.iter()) {
                    if !self.unify_annotations(elem1, elem2)? {
                        return Ok(false);
                    }
                }

                Ok(true)
            }

            // Structural object types
            (TypeAnnotation::Object(f1), TypeAnnotation::Object(f2)) => {
                self.object_fields_compatible(f1, f2)
            }

            // Function types
            (
                TypeAnnotation::Function {
                    params: p1,
                    returns: r1,
                },
                TypeAnnotation::Function {
                    params: p2,
                    returns: r2,
                },
            ) => {
                if p1.len() != p2.len() {
                    return Ok(false);
                }

                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    if !self.unify_annotations(&param1.type_annotation, &param2.type_annotation)? {
                        return Ok(false);
                    }
                }

                self.unify_annotations(r1, r2)
            }

            // Union types
            // A | B unifies with C | D if each type in one union can unify with at least one type in the other
            (TypeAnnotation::Union(u1), TypeAnnotation::Union(u2)) => {
                // Check that every type in u1 can unify with at least one type in u2
                for t1 in u1 {
                    let mut found_match = false;
                    for t2 in u2 {
                        if self.unify_annotations(t1, t2)? {
                            found_match = true;
                            break;
                        }
                    }
                    if !found_match {
                        return Ok(false);
                    }
                }
                // Check that every type in u2 can unify with at least one type in u1
                for t2 in u2 {
                    let mut found_match = false;
                    for t1 in u1 {
                        if self.unify_annotations(t1, t2)? {
                            found_match = true;
                            break;
                        }
                    }
                    if !found_match {
                        return Ok(false);
                    }
                }
                Ok(true)
            }

            // Union with non-union: A | B unifies with C if either A or B unifies with C
            (TypeAnnotation::Union(union_types), other)
            | (other, TypeAnnotation::Union(union_types)) => {
                for union_type in union_types {
                    if self.unify_annotations(union_type, other)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }

            // Intersection types (order-independent)
            (TypeAnnotation::Intersection(i1), TypeAnnotation::Intersection(i2)) => {
                self.unify_annotation_sets(i1, i2)
            }

            // Void, Null, Undefined
            (TypeAnnotation::Void, TypeAnnotation::Void) => Ok(true),
            (TypeAnnotation::Null, TypeAnnotation::Null) => Ok(true),
            (TypeAnnotation::Undefined, TypeAnnotation::Undefined) => Ok(true),

            // Trait object types: dyn Trait1 + Trait2
            // Two trait objects unify if they have the same set of traits
            (TypeAnnotation::Dyn(traits1), TypeAnnotation::Dyn(traits2)) => {
                Ok(traits1.len() == traits2.len() && traits1.iter().all(|t| traits2.contains(t)))
            }

            // Array<T> (Generic) is equivalent to Vec<T> (Array)
            (TypeAnnotation::Generic { name, args }, TypeAnnotation::Array(elem))
            | (TypeAnnotation::Array(elem), TypeAnnotation::Generic { name, args })
                if name == "Array" && args.len() == 1 =>
            {
                self.unify_annotations(&args[0], elem)
            }

            // Nominal struct ~ structural object: a named struct type (`Point`)
            // unifies with the structural object type its instances carry
            // (`{ x: number, y: number }`), in both directions. Resolution is
            // gated on the name being a *registered struct schema* — builtins,
            // enums, and unknown names fall through to `_ => Ok(false)`, so this
            // never broadens unification for non-struct references. Field
            // comparison reuses the exact-match `object_fields_compatible`
            // (equal field set + per-field unification), so a structurally
            // wrong object (missing/extra/mistyped field) still fails to unify.
            (TypeAnnotation::Object(obj_fields), TypeAnnotation::Basic(name))
            | (TypeAnnotation::Basic(name), TypeAnnotation::Object(obj_fields)) => {
                self.unify_object_with_named_struct(obj_fields, name)
            }
            (TypeAnnotation::Object(obj_fields), TypeAnnotation::Reference(path))
            | (TypeAnnotation::Reference(path), TypeAnnotation::Object(obj_fields)) => {
                self.unify_object_with_named_struct(obj_fields, path.as_str())
            }

            // Concrete nominal type coerces into a trait object iff it
            // implements every trait in the dyn set (standard trait-object
            // upcast). STRICT-FLIP (v0.3.3, SMOKE-s5): `let arr: Array<dyn
            // HasX> = [Bar { .. }]` decomposes the element constraint to
            // `Bar ~ dyn HasX`; without these arms it fell through to
            // `_ => Ok(false)` and surfaced an unsolved-constraint error.
            // Sound: succeeds ONLY when the impl is actually registered
            // (`has_trait_impl` over `self.trait_impls`, keyed `"Trait::Type"`);
            // a type that does not implement the trait still correctly rejects.
            // Both `Basic` and `Reference` because a struct/enum name may infer
            // as either (`format_annotation` renders them identically).
            (TypeAnnotation::Basic(name), TypeAnnotation::Dyn(traits))
            | (TypeAnnotation::Dyn(traits), TypeAnnotation::Basic(name)) => {
                Ok(traits.iter().all(|t| self.has_trait_impl(t.as_str(), name)))
            }
            (TypeAnnotation::Reference(path), TypeAnnotation::Dyn(traits))
            | (TypeAnnotation::Dyn(traits), TypeAnnotation::Reference(path)) => Ok(traits
                .iter()
                .all(|t| self.has_trait_impl(t.as_str(), path.as_str()))),

            // Different types don't unify
            _ => Ok(false),
        }
    }

    fn object_fields_compatible(
        &self,
        left: &[ObjectTypeField],
        right: &[ObjectTypeField],
    ) -> TypeResult<bool> {
        for left_field in left {
            let Some(right_field) = right.iter().find(|f| f.name == left_field.name) else {
                return Ok(false);
            };
            if left_field.optional != right_field.optional {
                return Ok(false);
            }
            if !self.unify_annotations(&left_field.type_annotation, &right_field.type_annotation)? {
                return Ok(false);
            }
        }
        if left.len() != right.len() {
            return Ok(false);
        }
        Ok(true)
    }

    fn unify_annotation_sets(
        &self,
        left: &[TypeAnnotation],
        right: &[TypeAnnotation],
    ) -> TypeResult<bool> {
        if left.len() != right.len() {
            return Ok(false);
        }

        let mut matched = vec![false; right.len()];
        for left_ann in left {
            let mut found = false;
            for (idx, right_ann) in right.iter().enumerate() {
                if matched[idx] {
                    continue;
                }
                if self.unify_annotations(left_ann, right_ann)? {
                    matched[idx] = true;
                    found = true;
                    break;
                }
            }
            if !found {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Apply type variable bounds, propagating resolved field types back to type variables.
    ///
    /// When a `HasField` constraint is satisfied and the expected field type was a
    /// type variable, this binds that variable to the actual field type. This enables
    /// backward propagation: `let f = |obj| obj.x; f({x: 42})` resolves `obj.x` to `int`.
    fn apply_bounds(&mut self) -> TypeResult<()> {
        let mut new_bindings: Vec<(TypeVar, Type)> = Vec::new();

        for (var, constraint) in &self.bounds {
            // Use apply_substitutions to follow the full variable chain
            // (lookup only returns the direct binding, not the resolved type).
            let resolved = self
                .unifier
                .apply_substitutions(&Type::Variable(var.clone()));

            if let Type::Variable(_) = &resolved {
                // Still unresolved — skip for now
                continue;
            }

            self.check_constraint(&resolved, constraint)?;

            // Backward propagation: when a HasField constraint is satisfied,
            // bind the result type variable to the actual field type.
            if let TypeConstraint::HasField(field, expected_field_type) = constraint {
                if let Type::Variable(field_var) = expected_field_type.as_ref() {
                    // Also check if the field var is already resolved
                    let field_resolved = self
                        .unifier
                        .apply_substitutions(&Type::Variable(field_var.clone()));
                    if let Type::Variable(_) = &field_resolved {
                        // Field var still unresolved — try to bind it
                        if let Type::Concrete(TypeAnnotation::Object(fields)) = &resolved {
                            if let Some(found_field) = fields.iter().find(|f| f.name == *field) {
                                new_bindings.push((
                                    field_var.clone(),
                                    Type::Concrete(found_field.type_annotation.clone()),
                                ));
                            }
                        }
                    }
                }
            }

            // Backward propagation: when an Indexable constraint is satisfied
            // and the constrained variable resolved to a concrete array type,
            // bind the carried element variable to the actual element type.
            // This is the connective tissue that lets `a[0]` on an
            // unannotated parameter recover its element type once `a`
            // resolves to `Array<int>` (via callsite unification). Without
            // this, the index access returns a disconnected fresh variable
            // and a downstream `a[0] + b[0]` sees `unknown + unknown`.
            if let TypeConstraint::Indexable(expected_elem_type) = constraint {
                if let Type::Variable(elem_var) = expected_elem_type.as_ref() {
                    let elem_resolved = self
                        .unifier
                        .apply_substitutions(&Type::Variable(elem_var.clone()));
                    if let Type::Variable(_) = &elem_resolved {
                        // RefDispatch (v0.3.3): `r[i]` on `r: &Array<T>` carries
                        // an `Indexable` constraint on the `Borrow`-typed
                        // variable. Deref the `Borrow { inner }` to its referent
                        // before extracting the element type, so the carried
                        // element var binds to the referent's element (mirrors
                        // the `infer_index_access` Borrow arm for the eager path,
                        // and the field-access auto-deref). Without this, an
                        // index through a ref inside a function (`r[1] + 5`)
                        // leaves the element `unknown` and strict typing rejects.
                        let resolved = match &resolved {
                            Type::Concrete(TypeAnnotation::Borrow { inner, .. }) => {
                                Type::Concrete((**inner).clone())
                            }
                            other => other.clone(),
                        };
                        let actual_elem: Option<Type> = match &resolved {
                            Type::Concrete(TypeAnnotation::Array(elem)) => {
                                Some(Type::Concrete((**elem).clone()))
                            }
                            Type::Generic { base, args }
                                if args.len() == 1 && is_array_or_vec_base(base) =>
                            {
                                Some(args[0].clone())
                            }
                            // String indexing yields a single-character string.
                            Type::Concrete(TypeAnnotation::Basic(name)) if name == "string" => {
                                Some(BuiltinTypes::string())
                            }
                            _ => None,
                        };
                        if let Some(elem_ty) = actual_elem {
                            if !matches!(elem_ty, Type::Variable(_)) {
                                new_bindings.push((elem_var.clone(), elem_ty));
                            }
                        }
                    }
                }
            }
        }

        // Apply collected bindings
        for (var, ty) in new_bindings {
            self.unifier.bind(var, ty);
        }

        Ok(())
    }

    /// Check if a type satisfies a constraint
    fn check_constraint(&self, ty: &Type, constraint: &TypeConstraint) -> TypeResult<()> {
        match constraint {
            TypeConstraint::Comparable => match ty {
                Type::Concrete(ann) => {
                    // A match-accumulate union whose arms all yield the same
                    // type (`Union([int])`) is just that type — collapse it
                    // before the comparability check (mirrors the
                    // ImplementsTrait arm at constraints.rs:991). A genuinely
                    // heterogeneous union is left intact and still fails below.
                    // A-final ROOT G.
                    let ann = collapse_degenerate_union(ann);
                    match ann {
                        TypeAnnotation::Basic(name)
                            if BuiltinTypes::is_numeric_type_name(name)
                                || name == "string"
                                || name == "bool" =>
                        {
                            Ok(())
                        }
                        _ => Err(TypeError::ConstraintViolation(format!(
                            "{:?} is not comparable",
                            ty
                        ))),
                    }
                }
                _ => Err(TypeError::ConstraintViolation(format!(
                    "{:?} is not comparable",
                    ty
                ))),
            },

            TypeConstraint::Iterable => match ty {
                Type::Concrete(TypeAnnotation::Array(_)) => Ok(()),
                Type::Concrete(TypeAnnotation::Basic(name))
                    if name == "string" || name == "rows" =>
                {
                    Ok(())
                }
                _ => Err(TypeError::ConstraintViolation(format!(
                    "{:?} is not iterable",
                    ty
                ))),
            },

            // `obj[i]` index access. The carried element type is bound by
            // `apply_bounds` backward propagation (mirrors `HasField`); here
            // we only validate that the resolved type supports indexing.
            TypeConstraint::Indexable(elem) => match ty {
                Type::Concrete(TypeAnnotation::Array(_)) => Ok(()),
                Type::Generic { base, args } if args.len() == 1 && is_array_or_vec_base(base) => {
                    Ok(())
                }
                Type::Concrete(TypeAnnotation::Basic(name))
                    if name == "string" || name == "rows" =>
                {
                    Ok(())
                }
                // Index access through a reference (v0.3.3 RefDispatch): a
                // `Borrow { inner }` indexes THROUGH the reference — deref to its
                // referent and re-check (mirrors the field-access auto-deref in
                // `infer_property_access_internal`). Inference normally resolves
                // the element type via `infer_index_access`'s Borrow arm before
                // this constraint check runs; this arm covers the case where a
                // constrained variable only resolves to a `Borrow` at check time.
                Type::Concrete(TypeAnnotation::Borrow { inner, .. }) => self.check_constraint(
                    &Type::Concrete((**inner).clone()),
                    &TypeConstraint::Indexable(elem.clone()),
                ),
                _ => Err(TypeError::ConstraintViolation(format!(
                    "{:?} does not support index access",
                    ty
                ))),
            },

            TypeConstraint::HasField(field, expected_field_type) => {
                match ty {
                    Type::Concrete(TypeAnnotation::Object(fields)) => {
                        match fields.iter().find(|f| f.name == *field) {
                            Some(found_field) => {
                                // Check that field type matches expected type
                                if let Some(expected_ann) = expected_field_type.to_annotation() {
                                    if self.unify_annotations(
                                        &found_field.type_annotation,
                                        &expected_ann,
                                    )? {
                                        Ok(())
                                    } else {
                                        Err(TypeError::ConstraintViolation(format!(
                                            "field '{}' has type {:?}, expected {:?}",
                                            field, found_field.type_annotation, expected_ann
                                        )))
                                    }
                                } else {
                                    // Expected type is a type variable, accept any field type
                                    Ok(())
                                }
                            }
                            None => Err(TypeError::ConstraintViolation(format!(
                                "{:?} does not have field '{}'",
                                ty, field
                            ))),
                        }
                    }
                    Type::Concrete(TypeAnnotation::Basic(_name)) => {
                        // For named types, we assume property access was validated
                        // during inference using the schema registry. If a HasField
                        // constraint reaches here, it means the type wasn't a known
                        // schema type during inference, so we accept it tentatively.
                        // Runtime will do the final validation.
                        //
                        // Note: Previously this hardcoded "row" with OHLCV fields.
                        // Now schema validation happens in TypeInferenceEngine::infer_property_access.
                        Ok(())
                    }
                    // T1 sub-case (a) (strict-flip, 2026-06-20): a field access on
                    // a value whose type resolved to a NAMED struct `Reference`
                    // (e.g. `rs[0].len` where `rs: Vec<Run>` flows through the
                    // `let mut rs = []; rs = rs.push(Run{..})` element-type
                    // back-propagation, leaving the element type as
                    // `Concrete(Reference(Run))` rather than the structural
                    // `Object(..)` form). Resolve the struct's declared fields and
                    // validate the named field — same registry the structural
                    // `Object(..)` arm consults, mirroring the `Basic(name)`
                    // tentative-accept for an unregistered name. A registered
                    // struct missing the field is a real error; an unregistered
                    // reference is accepted tentatively (runtime validates).
                    Type::Concrete(TypeAnnotation::Reference(path)) => {
                        match self.struct_fields(path.name()) {
                            Some(struct_fields) => {
                                match struct_fields.iter().find(|f| f.name == *field) {
                                    Some(found_field) => {
                                        if let Some(expected_ann) =
                                            expected_field_type.to_annotation()
                                        {
                                            if self.unify_annotations(
                                                &found_field.type_annotation,
                                                &expected_ann,
                                            )? {
                                                Ok(())
                                            } else {
                                                Err(TypeError::ConstraintViolation(format!(
                                                    "field '{}' has type {:?}, expected {:?}",
                                                    field,
                                                    found_field.type_annotation,
                                                    expected_ann
                                                )))
                                            }
                                        } else {
                                            // STAGE F1 (strict-flip, 2026-06-20):
                                            // the field RESULT is an unresolved type
                                            // variable. This arm is reached ONLY by a
                                            // field read on a value whose element type
                                            // was back-propagated to a bare named
                                            // struct `Reference` — i.e. the unannotated
                                            // empty-`[]` accumulator grown by `push`
                                            // (`let mut rs = []; rs = rs.push(Run{..})`,
                                            // then `rs[0].field` / `for r in rs { r.field }`).
                                            // The annotated (`let rs: Array<Run> = …`)
                                            // and non-empty-literal (`let rs = [Run{..}]`)
                                            // paths resolve the element STRUCTURALLY
                                            // (`Object(..)`) and validate + carry the
                                            // field type directly in `infer_property_access`,
                                            // never reaching this arm. Accepting "any
                                            // field type" here makes the field result an
                                            // UNCONSTRAINED `any`: an ill-typed program
                                            // (`let x: bool = rs[0].n` where `n: int`, or
                                            // `rs[0].n + y` with `y: number`) is wrongly
                                            // accepted, and `int`/`number` would silently
                                            // unify (CLAUDE.md §Type-System-Rules / no
                                            // `any` type / int != number). Per the
                                            // no-untyped-array rule, an unannotated
                                            // empty-`[]` accumulator has no DECLARED
                                            // element type, so a field read off its
                                            // element is unprovable WITHOUT annotation —
                                            // surface the existing "annotate the array"
                                            // guidance as a CLEAN compile-error rather
                                            // than sink to `any`.
                                            Err(TypeError::ConstraintViolation(format!(
                                                "cannot infer the type of field '{}' read \
                                                 off an element of `{}`: its element type \
                                                 is only known from a `push` into an \
                                                 unannotated empty array (`[]`), which has \
                                                 no declared element type. Strict typing \
                                                 requires a known element type — annotate \
                                                 the array (`let rs: Array<{}> = []`).",
                                                field,
                                                path.name(),
                                                path.name()
                                            )))
                                        }
                                    }
                                    None => Err(TypeError::ConstraintViolation(format!(
                                        "{:?} does not have field '{}'",
                                        ty, field
                                    ))),
                                }
                            }
                            // Not a registered struct (enum / alias / builtin) —
                            // accept tentatively, parity with the Basic(name) arm.
                            None => Ok(()),
                        }
                    }
                    _ => Err(TypeError::ConstraintViolation(format!(
                        "{:?} cannot have fields",
                        ty
                    ))),
                }
            }

            TypeConstraint::Callable {
                params: expected_params,
                returns: expected_returns,
            } => {
                match ty {
                    Type::Concrete(TypeAnnotation::Function {
                        params: actual_params,
                        returns: actual_returns,
                    }) => {
                        // Check parameter count matches
                        if expected_params.len() != actual_params.len() {
                            return Err(TypeError::ConstraintViolation(format!(
                                "function expects {} parameters, got {}",
                                expected_params.len(),
                                actual_params.len()
                            )));
                        }

                        // Check each parameter type (contravariant: expected <: actual)
                        for (expected, actual) in expected_params.iter().zip(actual_params.iter()) {
                            if let Some(expected_ann) = expected.to_annotation() {
                                if !self
                                    .unify_annotations(&expected_ann, &actual.type_annotation)?
                                {
                                    return Err(TypeError::ConstraintViolation(format!(
                                        "parameter type mismatch: expected {:?}, got {:?}",
                                        expected_ann, actual.type_annotation
                                    )));
                                }
                            }
                        }

                        // Check return type (covariant: actual <: expected)
                        if let Some(expected_ret_ann) = expected_returns.to_annotation() {
                            if !self.unify_annotations(actual_returns, &expected_ret_ann)? {
                                return Err(TypeError::ConstraintViolation(format!(
                                    "return type mismatch: expected {:?}, got {:?}",
                                    expected_ret_ann, actual_returns
                                )));
                            }
                        }

                        Ok(())
                    }
                    Type::Function {
                        params: actual_params,
                        returns: actual_returns,
                    } => {
                        if expected_params.len() != actual_params.len() {
                            return Err(TypeError::ConstraintViolation(format!(
                                "function expects {} parameters, got {}",
                                expected_params.len(),
                                actual_params.len()
                            )));
                        }
                        // Type::Function params are Type, not FunctionParam — compare directly
                        for (expected, actual) in expected_params.iter().zip(actual_params.iter()) {
                            if let (Some(e_ann), Some(a_ann)) =
                                (expected.to_annotation(), actual.to_annotation())
                            {
                                if !self.unify_annotations(&e_ann, &a_ann)? {
                                    return Err(TypeError::ConstraintViolation(format!(
                                        "parameter type mismatch: expected {:?}, got {:?}",
                                        e_ann, a_ann
                                    )));
                                }
                            }
                        }
                        if let (Some(e_ret), Some(a_ret)) = (
                            expected_returns.to_annotation(),
                            actual_returns.to_annotation(),
                        ) {
                            if !self.unify_annotations(&a_ret, &e_ret)? {
                                return Err(TypeError::ConstraintViolation(format!(
                                    "return type mismatch: expected {:?}, got {:?}",
                                    e_ret, a_ret
                                )));
                            }
                        }
                        Ok(())
                    }
                    _ => Err(TypeError::ConstraintViolation(format!(
                        "{:?} is not callable",
                        ty
                    ))),
                }
            }

            TypeConstraint::OneOf(options) => {
                for option in options {
                    // If type matches any option, constraint is satisfied
                    if let Type::Concrete(ann) = option {
                        if let Type::Concrete(ty_ann) = ty {
                            if self.unify_annotations(ann, ty_ann).unwrap_or(false) {
                                return Ok(());
                            }
                        }
                    }
                }

                Err(TypeError::ConstraintViolation(format!(
                    "{:?} does not match any of {:?}",
                    ty, options
                )))
            }

            TypeConstraint::Extends(base) => {
                // Implement subtyping check
                self.is_subtype(ty, base)
            }

            TypeConstraint::ImplementsTrait { trait_name } => {
                match ty {
                    Type::Variable(_) => {
                        // Type variable not yet resolved — this is a compile error
                        // (no deferring per Sprint 2 spec)
                        Err(TypeError::TraitBoundViolation {
                            type_name: format!("{:?}", ty),
                            trait_name: trait_name.clone(),
                        })
                    }
                    Type::Concrete(ann) => {
                        // A match-accumulate union whose arms all yield the same
                        // type (`Union([int])`) is just that type — collapse it
                        // before extracting the name so `Numeric`/etc. resolve.
                        let ann = collapse_degenerate_union(ann);
                        let type_name = match ann {
                            TypeAnnotation::Basic(n) => n.clone(),
                            TypeAnnotation::Reference(n) => n.to_string(),
                            _ => format!("{:?}", ann),
                        };
                        if self.has_trait_impl(trait_name, &type_name) {
                            Ok(())
                        } else {
                            Err(TypeError::TraitBoundViolation {
                                type_name,
                                trait_name: trait_name.clone(),
                            })
                        }
                    }
                    Type::Generic { base, .. } => {
                        let type_name = match base.as_ref() {
                            Type::Concrete(TypeAnnotation::Reference(n)) => n.to_string(),
                            Type::Concrete(TypeAnnotation::Basic(n)) => n.clone(),
                            _ => format!("{:?}", base),
                        };
                        if self.has_trait_impl(trait_name, &type_name) {
                            Ok(())
                        } else {
                            Err(TypeError::TraitBoundViolation {
                                type_name,
                                trait_name: trait_name.clone(),
                            })
                        }
                    }
                    _ => Err(TypeError::TraitBoundViolation {
                        type_name: format!("{:?}", ty),
                        trait_name: trait_name.clone(),
                    }),
                }
            }

            TypeConstraint::HasMethod {
                method_name,
                arg_types: _,
                return_type: _,
            } => {
                // If we have a method table, enforce the constraint
                if let Some(method_table) = &self.method_table {
                    match ty {
                        Type::Variable(_) => Ok(()), // Unresolved type var, defer
                        Type::Concrete(ann) => {
                            let type_name = match ann {
                                TypeAnnotation::Basic(n) => n.clone(),
                                TypeAnnotation::Reference(n) => n.to_string(),
                                TypeAnnotation::Array(_) => "Vec".to_string(),
                                _ => return Ok(()), // Complex types: accept
                            };
                            if method_table.lookup(ty, method_name).is_some() {
                                Ok(())
                            } else {
                                Err(TypeError::MethodNotFound {
                                    type_name,
                                    method_name: method_name.clone(),
                                })
                            }
                        }
                        Type::Generic { base, .. } => {
                            if method_table.lookup(ty, method_name).is_some() {
                                Ok(())
                            } else {
                                let type_name =
                                    if let Type::Concrete(TypeAnnotation::Reference(n)) =
                                        base.as_ref()
                                    {
                                        n.to_string()
                                    } else {
                                        format!("{:?}", base)
                                    };
                                Err(TypeError::MethodNotFound {
                                    type_name,
                                    method_name: method_name.clone(),
                                })
                            }
                        }
                        _ => Ok(()), // Function, Constrained: accept
                    }
                } else {
                    // No method table attached — accept all (backward compatible)
                    Ok(())
                }
            }
        }
    }

    /// Check if a type implements a trait, considering aliases and numeric widening.
    ///
    /// Handles three resolution strategies:
    /// 1. Direct lookup: `"Numeric::int"` in the trait_impls set
    /// 2. Canonical alias: `"Float"` → `"f64"`, `"byte"` → `"u8"` via runtime name table
    /// 3. Script alias: `"i64"` → `"int"`, `"f64"` → `"number"` via script alias table
    /// 4. Numeric widening: integer-family names can satisfy number/float/f64 impls
    fn has_trait_impl(&self, trait_name: &str, type_name: &str) -> bool {
        let key = format!("{}::{}", trait_name, type_name);
        if self.trait_impls.contains(&key) {
            return true;
        }
        // Try canonical runtime alias (e.g. "Float" -> "f64", "byte" -> "u8")
        if let Some(canonical) = BuiltinTypes::canonical_numeric_runtime_name(type_name) {
            let canon_key = format!("{}::{}", trait_name, canonical);
            if self.trait_impls.contains(&canon_key) {
                return true;
            }
        }
        // Try script-facing alias (e.g. "i64" -> "int", "f64" -> "number")
        if let Some(script_alias) = BuiltinTypes::canonical_script_alias(type_name) {
            let alias_key = format!("{}::{}", trait_name, script_alias);
            if self.trait_impls.contains(&alias_key) {
                return true;
            }
        }
        // Numeric widening: integer-family aliases can use number/float/f64 impls.
        if BuiltinTypes::is_integer_type_name(type_name) {
            for widen_to in &["number", "float", "f64"] {
                let widen_key = format!("{}::{}", trait_name, widen_to);
                if self.trait_impls.contains(&widen_key) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if ty is a subtype of base (ty <: base)
    /// Subtyping rules:
    /// - Same types are subtypes of each other
    /// - Any is a supertype of everything
    /// - Vec<A> <: Vec<B> if A <: B (covariant)
    /// - Function<P1, R1> <: Function<P2, R2> if P2 <: P1 (contravariant params) and R1 <: R2 (covariant return)
    fn is_subtype(&self, ty: &Type, base: &Type) -> TypeResult<()> {
        match (ty, base) {
            // Same types are subtypes
            (t1, t2) if t1 == t2 => Ok(()),

            // Type variables - if we can unify, it's compatible
            (Type::Variable(_), _) | (_, Type::Variable(_)) => Ok(()),

            // Array subtyping (covariant)
            (
                Type::Concrete(TypeAnnotation::Array(elem1)),
                Type::Concrete(TypeAnnotation::Array(elem2)),
            ) => {
                let t1 = Type::Concrete(*elem1.clone());
                let t2 = Type::Concrete(*elem2.clone());
                self.is_subtype(&t1, &t2)
            }

            // Function subtyping (contravariant params, covariant return)
            (
                Type::Concrete(TypeAnnotation::Function {
                    params: p1,
                    returns: r1,
                }),
                Type::Concrete(TypeAnnotation::Function {
                    params: p2,
                    returns: r2,
                }),
            ) => {
                // Check parameter count
                if p1.len() != p2.len() {
                    return Err(TypeError::ConstraintViolation(format!(
                        "function parameter count mismatch: {} vs {}",
                        p1.len(),
                        p2.len()
                    )));
                }

                // Contravariant: base params must be subtypes of ty params
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    let t1 = Type::Concrete(param2.type_annotation.clone());
                    let t2 = Type::Concrete(param1.type_annotation.clone());
                    self.is_subtype(&t1, &t2)?;
                }

                // Covariant: ty return must be subtype of base return
                let ret1 = Type::Concrete(*r1.clone());
                let ret2 = Type::Concrete(*r2.clone());
                self.is_subtype(&ret1, &ret2)
            }

            // Optional subtyping: T <: Option<T>
            (t, Type::Concrete(TypeAnnotation::Generic { name, args }))
                if name == "Option" && args.len() == 1 =>
            {
                let inner = Type::Concrete(args[0].clone());
                self.is_subtype(t, &inner)
            }

            // Type::Function subtyping (contravariant params, covariant return)
            (
                Type::Function {
                    params: p1,
                    returns: r1,
                },
                Type::Function {
                    params: p2,
                    returns: r2,
                },
            ) => {
                if p1.len() != p2.len() {
                    return Err(TypeError::ConstraintViolation(format!(
                        "function parameter count mismatch: {} vs {}",
                        p1.len(),
                        p2.len()
                    )));
                }
                // Contravariant params
                for (param1, param2) in p1.iter().zip(p2.iter()) {
                    self.is_subtype(param2, param1)?;
                }
                // Covariant return
                self.is_subtype(r1, r2)
            }

            // Basic types - check if they unify
            (Type::Concrete(ann1), Type::Concrete(ann2)) => {
                if self.unify_annotations(ann1, ann2)? {
                    Ok(())
                } else {
                    Err(TypeError::ConstraintViolation(format!(
                        "{:?} is not a subtype of {:?}",
                        ty, base
                    )))
                }
            }

            // Default: not a subtype
            _ => Err(TypeError::ConstraintViolation(format!(
                "{:?} is not a subtype of {:?}",
                ty, base
            ))),
        }
    }

    /// Get the unifier for applying substitutions
    pub fn unifier(&self) -> &Unifier {
        &self.unifier
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_system::TypeVarGen;
    use shape_ast::ast::ObjectTypeField;

    /// Test-local helper: allocate a fresh type variable from a
    /// per-test counter. Each test owns its own `TypeVarGen`, so IDs
    /// (`T0`, `T1`, ...) are deterministic and independent across tests.
    fn fresh_var(tvgen: &mut TypeVarGen) -> TypeVar {
        tvgen.fresh_var()
    }

    fn fresh_type(tvgen: &mut TypeVarGen) -> Type {
        tvgen.fresh_type()
    }

    #[test]
    fn test_hasfield_backward_propagation_binds_field_type() {
        // When a TypeVar has a HasField constraint and is resolved to a concrete
        // object type, the field's result type variable should be bound to the
        // actual field type. This enables backward type propagation.
        let mut solver = ConstraintSolver::new();
        let mut tvgen = TypeVarGen::new();

        let obj_var = fresh_var(&mut tvgen);
        let field_result_var = fresh_var(&mut tvgen);
        let bound_var = fresh_var(&mut tvgen);

        let mut constraints = vec![
            // obj_var ~ Constrained { var: bound_var, HasField("x", field_result_var) }
            // This records bound: bound_var → HasField("x", field_result_var)
            // and solves: bound_var ~ obj_var
            (
                Type::Variable(obj_var.clone()),
                Type::Constrained {
                    var: bound_var,
                    constraint: Box::new(TypeConstraint::HasField(
                        "x".to_string(),
                        Box::new(Type::Variable(field_result_var.clone())),
                    )),
                },
            ),
            // obj_var = {x: int}
            (
                Type::Variable(obj_var),
                Type::Concrete(TypeAnnotation::Object(vec![ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                }])),
            ),
        ];

        solver.solve(&mut constraints).unwrap();

        // field_result_var should now be resolved to int via apply_bounds
        let resolved = solver
            .unifier()
            .apply_substitutions(&Type::Variable(field_result_var));
        match &resolved {
            Type::Concrete(TypeAnnotation::Basic(name)) => {
                assert_eq!(name, "int", "field type should be int");
            }
            _ => panic!(
                "Expected field_result_var to be resolved to int, got {:?}",
                resolved
            ),
        }
    }

    #[test]
    fn test_hasfield_backward_propagation_multiple_fields() {
        // Test that multiple HasField constraints on the same object all propagate
        let mut solver = ConstraintSolver::new();
        let mut tvgen = TypeVarGen::new();

        let obj_var = fresh_var(&mut tvgen);
        let field_x_var = fresh_var(&mut tvgen);
        let field_y_var = fresh_var(&mut tvgen);
        let bound_var_x = fresh_var(&mut tvgen);
        let bound_var_y = fresh_var(&mut tvgen);

        let mut constraints = vec![
            // HasField("x", field_x_var)
            (
                Type::Variable(obj_var.clone()),
                Type::Constrained {
                    var: bound_var_x,
                    constraint: Box::new(TypeConstraint::HasField(
                        "x".to_string(),
                        Box::new(Type::Variable(field_x_var.clone())),
                    )),
                },
            ),
            // HasField("y", field_y_var)
            (
                Type::Variable(obj_var.clone()),
                Type::Constrained {
                    var: bound_var_y,
                    constraint: Box::new(TypeConstraint::HasField(
                        "y".to_string(),
                        Box::new(Type::Variable(field_y_var.clone())),
                    )),
                },
            ),
            // obj_var = {x: int, y: string}
            (
                Type::Variable(obj_var),
                Type::Concrete(TypeAnnotation::Object(vec![
                    ObjectTypeField {
                        name: "x".to_string(),
                        optional: false,
                        type_annotation: TypeAnnotation::Basic("int".to_string()),
                        annotations: vec![],
                    },
                    ObjectTypeField {
                        name: "y".to_string(),
                        optional: false,
                        type_annotation: TypeAnnotation::Basic("string".to_string()),
                        annotations: vec![],
                    },
                ])),
            ),
        ];

        solver.solve(&mut constraints).unwrap();

        let resolved_x = solver
            .unifier()
            .apply_substitutions(&Type::Variable(field_x_var));
        let resolved_y = solver
            .unifier()
            .apply_substitutions(&Type::Variable(field_y_var));

        match &resolved_x {
            Type::Concrete(TypeAnnotation::Basic(name)) => assert_eq!(name, "int"),
            _ => panic!("Expected x to be int, got {:?}", resolved_x),
        }
        match &resolved_y {
            Type::Concrete(TypeAnnotation::Basic(name)) => assert_eq!(name, "string"),
            _ => panic!("Expected y to be string, got {:?}", resolved_y),
        }
    }

    // ===== Fix 1: Numeric type preservation tests =====

    #[test]
    fn test_int_constrained_numeric_succeeds() {
        // Concrete(int) ~ Constrained(ImplementsTrait("Numeric")) should succeed
        let mut solver = ConstraintSolver::new();
        // Inject Numeric trait impls (same as TypeEnvironment registers)
        let trait_impls: std::collections::HashSet<String> = [
            "Numeric::int",
            "Numeric::number",
            "Numeric::decimal",
            "Numeric::i8",
            "Numeric::i16",
            "Numeric::i32",
            "Numeric::i64",
            "Numeric::u8",
            "Numeric::u16",
            "Numeric::u32",
            "Numeric::u64",
            "Numeric::f32",
            "Numeric::f64",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        solver.set_trait_impls(trait_impls);
        let mut tvgen = TypeVarGen::new();
        let bound_var = fresh_var(&mut tvgen);
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Numeric".to_string(),
                }),
            },
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_int_to_number_is_cast_required() {
        // TP-REBASELINE (numeric-conversion GREEN Stage 2, THE RULE user
        // 2026-06-01 / spec §2): `int(i64) -> number` is CAST-REQUIRED, NOT an
        // implicit widening — not every i64 is exactly representable in f64
        // (e.g. 2^53+1). The prior test pinned the now-deleted loose
        // `can_numeric_widen` (every integer -> any float). Value-level
        // `let n: number = int_value` must reject and demand `int_value as
        // number`. (`i32 -> number` stays IMPL because EVERY i32 fits — see
        // `test_numeric_widening_width_aware_integer_to_float_family`.)
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            Type::Concrete(TypeAnnotation::Basic("number".to_string())),
        )];
        assert!(solver.solve(&mut constraints).is_err());
    }

    #[test]
    fn test_numeric_widening_width_aware_integer_to_float_family() {
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("i16".to_string())),
            Type::Concrete(TypeAnnotation::Basic("f32".to_string())),
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_no_widening_number_to_int() {
        // (Concrete(number), Concrete(int)) should fail — lossy
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("number".to_string())),
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
        )];
        assert!(solver.solve(&mut constraints).is_err());
    }

    #[test]
    fn test_width_int_widens_to_int_directionally() {
        // TP-REBASELINE (numeric-conversion GREEN Stage 2, THE RULE / spec §2):
        // the prior "both directions unify" premise is no longer correct — the
        // §2 lattice is DIRECTIONAL `(src, dst)`. A narrower-or-cross-sign-fit
        // integer widens INTO `int(i64)` (IMPL), but `int` does NOT implicitly
        // narrow / sign-reinterpret back (CAST-REQUIRED). The prior test pinned
        // the now-deleted width-collapse (`canonical_script_alias` -> single
        // `"int"`) that let `int -> i8` and `int -> u64` silently pass.
        //
        // Widening INTO int (subset of i64) — IMPL (accept):
        for (src, dst) in [
            ("i8", "int"),
            ("u16", "int"),
            ("i32", "int"),
            ("u32", "int"),
        ] {
            let mut solver = ConstraintSolver::new();
            let mut constraints = vec![(
                Type::Concrete(TypeAnnotation::Basic(src.to_string())),
                Type::Concrete(TypeAnnotation::Basic(dst.to_string())),
            )];
            assert!(
                solver.solve(&mut constraints).is_ok(),
                "{src} should losslessly widen to {dst}"
            );
        }
        // `int` narrowing / sign-reinterpreting back — CAST-REQUIRED (reject):
        for (src, dst) in [
            ("int", "i8"),
            ("int", "u64"),
            ("int", "i32"),
            ("int", "u16"),
        ] {
            let mut solver = ConstraintSolver::new();
            let mut constraints = vec![(
                Type::Concrete(TypeAnnotation::Basic(src.to_string())),
                Type::Concrete(TypeAnnotation::Basic(dst.to_string())),
            )];
            assert!(
                solver.solve(&mut constraints).is_err(),
                "{src} must NOT implicitly convert to {dst} (cast required)"
            );
        }
    }

    #[test]
    fn test_distinct_width_ints_unify() {
        // Different integer widths share the `int` script type, so `i8 + i16`
        // (a mixed-width add) must type-check.
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("i8".to_string())),
            Type::Concrete(TypeAnnotation::Basic("i16".to_string())),
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_width_int_does_not_collapse_into_number_or_bool() {
        // The R5 fix must NOT collapse `int` and `number` (they stay distinct
        // script types), and must never make an integer unify with `bool`.
        // `number → int` (lossy) must still fail; `bool` vs `i8` must fail.
        for (a, b) in [
            ("number", "i8"),
            ("f64", "i32"),
            ("bool", "i8"),
            ("i8", "bool"),
        ] {
            let mut solver = ConstraintSolver::new();
            let mut constraints = vec![(
                Type::Concrete(TypeAnnotation::Basic(a.to_string())),
                Type::Concrete(TypeAnnotation::Basic(b.to_string())),
            )];
            assert!(
                solver.solve(&mut constraints).is_err(),
                "{a} must NOT unify with {b}"
            );
        }
    }

    #[test]
    fn test_decimal_constrained_numeric_succeeds() {
        let mut solver = ConstraintSolver::new();
        let trait_impls: std::collections::HashSet<String> = [
            "Numeric::int",
            "Numeric::number",
            "Numeric::decimal",
            "Numeric::i8",
            "Numeric::i16",
            "Numeric::i32",
            "Numeric::i64",
            "Numeric::u8",
            "Numeric::u16",
            "Numeric::u32",
            "Numeric::u64",
            "Numeric::f32",
            "Numeric::f64",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        solver.set_trait_impls(trait_impls);
        let mut tvgen = TypeVarGen::new();
        let bound_var = fresh_var(&mut tvgen);
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("decimal".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Numeric".to_string(),
                }),
            },
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_comparable_accepts_int() {
        // int should be Comparable
        let mut solver = ConstraintSolver::new();
        let mut tvgen = TypeVarGen::new();
        let bound_var = fresh_var(&mut tvgen);
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("int".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::Comparable),
            },
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    // ===== Fix 2: Type::Function tests =====

    #[test]
    fn test_function_type_preserves_variables() {
        // BuiltinTypes::function with Variable params should be Type::Function
        let mut tvgen = TypeVarGen::new();
        let param = fresh_type(&mut tvgen);
        let ret = fresh_type(&mut tvgen);
        let func = BuiltinTypes::function(vec![param.clone()], ret.clone());
        match func {
            Type::Function { params, returns } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0], param);
                assert_eq!(*returns, ret);
            }
            _ => panic!("Expected Type::Function, got {:?}", func),
        }
    }

    #[test]
    fn test_function_unification_binds_variables() {
        // (T1)->T2 ~ (number)->string should bind T1=number, T2=string
        let mut solver = ConstraintSolver::new();
        let mut tvgen = TypeVarGen::new();
        let t1 = fresh_var(&mut tvgen);
        let t2 = fresh_var(&mut tvgen);

        let mut constraints = vec![(
            Type::Function {
                params: vec![Type::Variable(t1.clone())],
                returns: Box::new(Type::Variable(t2.clone())),
            },
            Type::Function {
                params: vec![BuiltinTypes::number()],
                returns: Box::new(BuiltinTypes::string()),
            },
        )];

        solver.solve(&mut constraints).unwrap();

        let resolved_t1 = solver.unifier().apply_substitutions(&Type::Variable(t1));
        let resolved_t2 = solver.unifier().apply_substitutions(&Type::Variable(t2));
        assert_eq!(resolved_t1, BuiltinTypes::number());
        assert_eq!(resolved_t2, BuiltinTypes::string());
    }

    #[test]
    fn test_function_cross_unification_with_concrete() {
        // Type::Function ~ Concrete(TypeAnnotation::Function) should unify
        let mut solver = ConstraintSolver::new();
        let mut tvgen = TypeVarGen::new();
        let t1 = fresh_var(&mut tvgen);

        let concrete_func = Type::Concrete(TypeAnnotation::Function {
            params: vec![shape_ast::ast::FunctionParam {
                name: None,
                optional: false,
                type_annotation: TypeAnnotation::Basic("number".to_string()),
            }],
            returns: Box::new(TypeAnnotation::Basic("string".to_string())),
        });

        let mut constraints = vec![(
            Type::Function {
                params: vec![Type::Variable(t1.clone())],
                returns: Box::new(BuiltinTypes::string()),
            },
            concrete_func,
        )];

        solver.solve(&mut constraints).unwrap();

        let resolved = solver.unifier().apply_substitutions(&Type::Variable(t1));
        assert_eq!(resolved, BuiltinTypes::number());
    }

    #[test]
    fn test_object_annotations_unify_structurally() {
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Object(vec![
                ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
                ObjectTypeField {
                    name: "y".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
            ])),
            Type::Concrete(TypeAnnotation::Object(vec![
                ObjectTypeField {
                    name: "x".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
                ObjectTypeField {
                    name: "y".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("int".to_string()),
                    annotations: vec![],
                },
            ])),
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    /// R4: a named struct type (`Point`) unifies with the structural object
    /// type its instances carry (`{ x: number, y: number }`), in both
    /// directions — but only when registered as a struct schema, and only
    /// when the fields match exactly. This mirrors the call-site constraint
    /// `({ x: number, y: number }) -> number ~ (Point) -> number`.
    #[test]
    fn test_named_struct_unifies_with_structural_object() {
        let point_fields = vec![
            ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("number".to_string()),
                annotations: vec![],
            },
            ObjectTypeField {
                name: "y".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("number".to_string()),
                annotations: vec![],
            },
        ];
        let mut schemas = HashMap::new();
        schemas.insert("Point".to_string(), point_fields.clone());

        // Positive: structural object ~ nominal Point (both Basic and Reference
        // spellings of the named struct), and the reverse direction.
        for (obj_first, named) in [
            (true, TypeAnnotation::Basic("Point".to_string())),
            (false, TypeAnnotation::Basic("Point".to_string())),
            (
                true,
                TypeAnnotation::Reference(shape_ast::ast::TypePath::simple("Point")),
            ),
            (
                false,
                TypeAnnotation::Reference(shape_ast::ast::TypePath::simple("Point")),
            ),
        ] {
            let mut solver = ConstraintSolver::new();
            solver.set_struct_schemas(schemas.clone());
            let obj = Type::Concrete(TypeAnnotation::Object(point_fields.clone()));
            let nom = Type::Concrete(named);
            let pair = if obj_first { (obj, nom) } else { (nom, obj) };
            let mut constraints = vec![pair];
            assert!(
                solver.solve(&mut constraints).is_ok(),
                "named struct must unify with matching structural object"
            );
        }

        // Negative 1: a structurally-wrong object (extra/mismatched field) must
        // STILL fail — the fix must not blanket-suppress the error.
        let mut solver = ConstraintSolver::new();
        solver.set_struct_schemas(schemas.clone());
        let wrong_obj = Type::Concrete(TypeAnnotation::Object(vec![ObjectTypeField {
            name: "x".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("number".to_string()),
            annotations: vec![],
        }]));
        let mut constraints = vec![(
            wrong_obj,
            Type::Concrete(TypeAnnotation::Basic("Point".to_string())),
        )];
        assert!(
            solver.solve(&mut constraints).is_err(),
            "object with missing field must NOT unify with named struct"
        );

        // Negative 2: a name that is NOT a registered struct schema must keep
        // its existing (non-unifying) behaviour.
        let mut solver = ConstraintSolver::new();
        solver.set_struct_schemas(schemas);
        let obj = Type::Concrete(TypeAnnotation::Object(point_fields));
        let mut constraints = vec![(
            obj,
            Type::Concrete(TypeAnnotation::Basic("NotAStruct".to_string())),
        )];
        assert!(
            solver.solve(&mut constraints).is_err(),
            "unregistered name must not unify with a structural object"
        );
    }

    #[test]
    fn test_intersection_annotations_unify_order_independent() {
        let mut solver = ConstraintSolver::new();
        let obj_xy = TypeAnnotation::Object(vec![
            ObjectTypeField {
                name: "x".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
            ObjectTypeField {
                name: "y".to_string(),
                optional: false,
                type_annotation: TypeAnnotation::Basic("int".to_string()),
                annotations: vec![],
            },
        ]);
        let obj_z = TypeAnnotation::Object(vec![ObjectTypeField {
            name: "z".to_string(),
            optional: false,
            type_annotation: TypeAnnotation::Basic("int".to_string()),
            annotations: vec![],
        }]);

        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Intersection(vec![
                obj_xy.clone(),
                obj_z.clone(),
            ])),
            Type::Concrete(TypeAnnotation::Intersection(vec![obj_z, obj_xy])),
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    // ===== Sprint 2: ImplementsTrait constraint tests =====

    #[test]
    fn test_implements_trait_satisfied() {
        let mut solver = ConstraintSolver::new();
        let mut impls = std::collections::HashSet::new();
        impls.insert("Comparable::number".to_string());
        solver.set_trait_impls(impls);

        let mut tvgen = TypeVarGen::new();
        let bound_var = fresh_var(&mut tvgen);
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("number".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Comparable".to_string(),
                }),
            },
        )];
        assert!(solver.solve(&mut constraints).is_ok());
    }

    #[test]
    fn test_implements_trait_violated() {
        let mut solver = ConstraintSolver::new();
        // No trait impls registered — string doesn't implement Comparable
        let mut tvgen = TypeVarGen::new();
        let bound_var = fresh_var(&mut tvgen);
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Basic("string".to_string())),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Comparable".to_string(),
                }),
            },
        )];
        let result = solver.solve(&mut constraints);
        assert!(result.is_err());
        match result.unwrap_err() {
            TypeError::TraitBoundViolation {
                type_name,
                trait_name,
            } => {
                assert_eq!(type_name, "string");
                assert_eq!(trait_name, "Comparable");
            }
            other => panic!("Expected TraitBoundViolation, got: {:?}", other),
        }
    }

    /// A degenerate `Union([T])` (all arms of a match yield the same type)
    /// collapses to `T` and satisfies the trait `T` implements. Mirrors
    /// `match { Some(v) => v, None => 0 }` inferring `Union([int])`.
    #[test]
    fn test_implements_trait_single_member_union_collapses() {
        let mut solver = ConstraintSolver::new();
        let mut impls = std::collections::HashSet::new();
        impls.insert("Numeric::number".to_string());
        solver.set_trait_impls(impls);

        let mut tvgen = TypeVarGen::new();
        let bound_var = fresh_var(&mut tvgen);
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Union(vec![TypeAnnotation::Basic(
                "number".to_string(),
            )])),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Numeric".to_string(),
                }),
            },
        )];
        assert!(
            solver.solve(&mut constraints).is_ok(),
            "Union([number]) should collapse to number and satisfy Numeric"
        );
    }

    /// NOT-BROAD-SUPPRESSION: a genuinely heterogeneous union is left intact
    /// and must still FAIL a single-type trait bound. The collapse only fires
    /// when every member is structurally equal.
    #[test]
    fn test_implements_trait_heterogeneous_union_still_violates() {
        let mut solver = ConstraintSolver::new();
        let mut impls = std::collections::HashSet::new();
        impls.insert("Numeric::number".to_string());
        solver.set_trait_impls(impls);

        let mut tvgen = TypeVarGen::new();
        let bound_var = fresh_var(&mut tvgen);
        let mut constraints = vec![(
            Type::Concrete(TypeAnnotation::Union(vec![
                TypeAnnotation::Basic("number".to_string()),
                TypeAnnotation::Basic("string".to_string()),
            ])),
            Type::Constrained {
                var: bound_var,
                constraint: Box::new(TypeConstraint::ImplementsTrait {
                    trait_name: "Numeric".to_string(),
                }),
            },
        )];
        assert!(
            solver.solve(&mut constraints).is_err(),
            "Union([number, string]) is heterogeneous and must NOT satisfy Numeric"
        );
    }

    #[test]
    fn test_implements_trait_via_variable_resolution() {
        let mut solver = ConstraintSolver::new();
        let mut impls = std::collections::HashSet::new();
        impls.insert("Sortable::number".to_string());
        solver.set_trait_impls(impls);

        let mut tvgen = TypeVarGen::new();
        let type_var = fresh_var(&mut tvgen);
        let bound_var = fresh_var(&mut tvgen);

        let mut constraints = vec![
            // T: Sortable
            (
                Type::Variable(type_var.clone()),
                Type::Constrained {
                    var: bound_var,
                    constraint: Box::new(TypeConstraint::ImplementsTrait {
                        trait_name: "Sortable".to_string(),
                    }),
                },
            ),
            // T = number
            (
                Type::Variable(type_var),
                Type::Concrete(TypeAnnotation::Basic("number".to_string())),
            ),
        ];
        assert!(
            solver.solve(&mut constraints).is_ok(),
            "T resolved to number which implements Sortable"
        );
    }

    /// Build `Result<ok, err>` as a Generic with a Result base.
    fn make_result(ok: TypeAnnotation, err: TypeAnnotation) -> Type {
        Type::Generic {
            base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                shape_ast::ast::TypePath::simple("Result"),
            ))),
            args: vec![Type::Concrete(ok), Type::Concrete(err)],
        }
    }

    #[test]
    fn any_error_is_top_of_error_lattice() {
        // `Result<int, AnyError> ~ Result<T, string>`: AnyError is the default
        // error type for bare Ok(..)/? and must unify with any concrete error
        // type in the error-arg position.
        let mut solver = ConstraintSolver::new();
        let mut tvgen = TypeVarGen::new();
        let t = fresh_var(&mut tvgen);

        let mut constraints = vec![(
            make_result(
                TypeAnnotation::Basic("int".to_string()),
                TypeAnnotation::Reference(shape_ast::ast::TypePath::simple("AnyError")),
            ),
            Type::Generic {
                base: Box::new(Type::Concrete(TypeAnnotation::Reference(
                    shape_ast::ast::TypePath::simple("Result"),
                ))),
                args: vec![
                    Type::Variable(t),
                    Type::Concrete(TypeAnnotation::Basic("string".to_string())),
                ],
            },
        )];
        assert!(
            solver.solve(&mut constraints).is_ok(),
            "Result<int, AnyError> should unify with Result<T, string>"
        );
    }

    #[test]
    fn any_error_top_is_bounded_distinct_concrete_errors_still_mismatch() {
        // NOT broad suppression: two DISTINCT concrete named error types must
        // still mismatch in the error-arg position. AnyError is the only top.
        let mut solver = ConstraintSolver::new();
        let mut constraints = vec![(
            make_result(
                TypeAnnotation::Basic("int".to_string()),
                TypeAnnotation::Reference(shape_ast::ast::TypePath::simple("FooErr")),
            ),
            make_result(
                TypeAnnotation::Basic("int".to_string()),
                TypeAnnotation::Reference(shape_ast::ast::TypePath::simple("BarErr")),
            ),
        )];
        assert!(
            solver.solve(&mut constraints).is_err(),
            "Result<int, FooErr> must NOT unify with Result<int, BarErr> (distinct concrete error types)"
        );
    }
}
