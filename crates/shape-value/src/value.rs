//! Surviving typed value types after the strict-typing bulldozer.
//!
//! Most of this module's content (VMArray, Upvalue, HostCallable, PrintResult,
//! PrintSpan) was deleted along with the v1 ValueWord representation. What
//! remains are the pure-data filter / vtable types that don't reference any
//! dynamic-word machinery.

use smallvec::SmallVec;
use std::collections::HashMap;

/// Comparison operator for filter expressions.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
}

/// A literal value in a filter expression (for SQL generation).
#[derive(Debug, Clone, PartialEq)]
pub enum FilterLiteral {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
}

/// Filter expression tree for SQL pushdown.
///
/// Built from comparisons and logical operations. Represents typed
/// column-vs-literal predicates suitable for pushdown to a SQL backend.
#[derive(Debug, Clone, PartialEq)]
pub enum FilterNode {
    /// Column compared to a literal value.
    Compare {
        column: String,
        op: FilterOp,
        value: FilterLiteral,
    },
    /// Logical AND of two filter nodes.
    And(Box<FilterNode>, Box<FilterNode>),
    /// Logical OR of two filter nodes.
    Or(Box<FilterNode>, Box<FilterNode>),
    /// Logical NOT of a filter node.
    Not(Box<FilterNode>),
}

/// Virtual method table for trait objects.
///
/// Maps method names to entries describing how each method dispatches
/// through a `dyn Trait` receiver. Built at vtable-construction time
/// (compile-time per `(impl Trait for Type)` pair) and shared via
/// `Arc<VTable>` from the `TraitObjectStorage` fat-pointer carrier.
///
/// ADR-006 §2.7.24 Q25.C.5 — extended shape (W17-trait-object-storage,
/// 2026-05-11): the legacy 2-variant `VTableEntry { FunctionId, Closure }`
/// is widened to 6 variants (`Direct` / `Closure` / `BoxedReturn` /
/// `SelfArg` / `Generic` / `Compound`) to encode the per-method
/// rewriting that `Erase_T` performs on the method signature when the
/// trait is used as `dyn T`. Per-(impl, method) thunks are emitted at
/// vtable-construction time; the `thunk_id` fields below name them.
#[derive(Debug, Clone)]
pub struct VTable {
    /// Trait names this vtable implements. Supports multi-trait
    /// inheritance — when an impl spans `T: A + B + C`, all three trait
    /// names appear here in order so the §Q25.C.2 vtable-identity check
    /// can compare across the inheritance chain.
    pub trait_names: Vec<String>,
    /// Concrete-type discriminator for the underlying boxed value.
    /// Enables the §Q25.C.2 `Self`-arg runtime check (`Arc::ptr_eq` on
    /// vtables is a tighter equality, but `concrete_type_id` is needed
    /// for the cross-vtable comparison error message and IC-stabilization
    /// key in §Q25.C.6).
    pub concrete_type_id: u32,
    /// Map from method name to dispatch entry.
    pub methods: HashMap<String, VTableEntry>,
}

/// How a single trait method dispatches through `dyn T`.
///
/// Six variants cover the cross-product of (no rewriting / Self-in-return /
/// Self-in-arg / method-generic) per ADR-006 §2.7.24 Q25.C.5. The plain
/// (function_id) and (function_id + type_id) entries preserve the pre-§2.7.24
/// vtable shapes so existing emit-tier wiring continues to compile.
///
/// **Thunks vs function ids**: variants other than `Direct` / `Closure`
/// carry a `thunk_id` rather than a raw function id. The compiler-side
/// vtable-construction tier generates one thunk per `(impl, method)` pair
/// whose Erase_T-rewritten signature differs from the underlying impl
/// method; the thunk does the auto-boxing on return / vtable-identity
/// check / TypeInfo dispatch / etc., then tail-calls the impl method.
#[derive(Debug, Clone)]
pub enum VTableEntry {
    /// Direct call — no `Erase_T` rewriting needed (no `Self` in
    /// non-receiver position, no method-generic parameters). Dispatch
    /// is a simple function-id call. Preserves the pre-§2.7.24
    /// `VTableEntry::FunctionId(u16)` shape (renamed `function_id`
    /// field for forward consistency).
    Direct { function_id: u16 },

    /// Pre-existing closure entry (W7 closure trait impls).
    ///
    /// VTable closure entries carry `(function_id, type_id)`; dispatch
    /// allocates a fresh `OwnedClosureBlock` per call via the program's
    /// `closure_function_layouts` registry so the call convention sees
    /// the same raw `TypedClosureHeader` shape that `op_make_closure`
    /// emits.
    Closure { function_id: u32, type_id: u32 },

    /// `Self` (or `Self::A`) appears in return position. The thunk
    /// wraps the impl's concrete return value back into a `dyn T`
    /// carrier at each `wrap_targets` path before returning.
    BoxedReturn {
        thunk_id: u16,
        /// One entry per place the impl's signature names `Self` /
        /// `Self::A` inside its (possibly structural) return type.
        /// E.g. for `fn try_clone(&self) -> Result<Self, Error>`,
        /// `wrap_targets = [WrapTarget { path: [0], wrap_as_trait_id }]`.
        wrap_targets: SmallVec<[WrapTarget; 2]>,
    },

    /// `Self` appears in argument position. The thunk checks
    /// vtable-identity (per §Q25.C.2) between `self`'s vtable and each
    /// `Self`-typed argument's vtable before forwarding to the impl
    /// method.
    SelfArg {
        thunk_id: u16,
        /// Argument indices (0-based, excluding the receiver) at which
        /// the impl's signature names `Self` directly. Each gets one
        /// `Arc::ptr_eq` check on its vtable before dispatch.
        self_arg_positions: SmallVec<[u8; 4]>,
    },

    /// Method has type parameters (`fn method<G: Bound>(&self, g: G)`).
    /// The thunk consumes `type_param_count` `&TypeInfo` parameters
    /// alongside the regular arguments per §Q25.C.3 and dispatches on
    /// `concrete_type_id` for each.
    Generic {
        thunk_id: u16,
        type_param_count: u8,
    },

    /// Combination of `BoxedReturn` / `SelfArg` / `Generic`. The thunk
    /// dispatches per `flags` bit set.
    Compound {
        thunk_id: u16,
        flags: VTableEntryFlags,
        wrap_targets: SmallVec<[WrapTarget; 2]>,
        self_arg_positions: SmallVec<[u8; 4]>,
        type_param_count: u8,
    },
}

/// One auto-boxing site inside an `Erase_T`-rewritten return type.
///
/// `path` walks the generic-argument tree from the outer return type:
/// - `Self` in return position → `path = []` (the whole return)
/// - `Result<Self, Error>` → `path = [0]` (the Ok arm)
/// - `(Self, Self)` (tuple) → two entries, `path = [0]` and `path = [1]`
/// - `HashMap<int, Self>` → `path = [1]` (the value type)
/// - `Option<Result<Self, Error>>` → `path = [0, 0]` (the inner Ok arm)
///
/// `wrap_as_trait_id` is the trait the boxed value should advertise
/// itself under — usually the receiver trait's id, but for `Self::A`
/// (associated-type return with bound `Bound`) the bound's trait id
/// per §Q25.C.1 row 4.
#[derive(Debug, Clone)]
pub struct WrapTarget {
    /// Argument-index path into the structural return type.
    pub path: SmallVec<[u8; 4]>,
    /// Trait the wrapped value's vtable should be registered against.
    pub wrap_as_trait_id: u32,
}

/// Bitfield for `VTableEntry::Compound` — which of the three rewriting
/// shapes apply to this method.
///
/// Plain `u8` bitflags (not the `bitflags` crate) to keep the dep tree
/// small. Helpers: `is_boxed_return()`, `is_self_arg()`, `is_generic()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct VTableEntryFlags(pub u8);

impl VTableEntryFlags {
    pub const BOXED_RETURN: u8 = 0b0000_0001;
    pub const SELF_ARG: u8 = 0b0000_0010;
    pub const GENERIC: u8 = 0b0000_0100;

    #[inline]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    #[inline]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[inline]
    pub const fn is_boxed_return(self) -> bool {
        self.0 & Self::BOXED_RETURN != 0
    }

    #[inline]
    pub const fn is_self_arg(self) -> bool {
        self.0 & Self::SELF_ARG != 0
    }

    #[inline]
    pub const fn is_generic(self) -> bool {
        self.0 & Self::GENERIC != 0
    }

    #[inline]
    pub fn set(&mut self, flag: u8) {
        self.0 |= flag;
    }
}

/// Runtime type information for method-generic parameters (§Q25.C.3).
///
/// Threaded through `VTableEntry::Generic` / `Compound` thunks so a
/// `fn method<G: Bound>(&self, g: G)` invocation through `dyn T`
/// dispatches operations on `g` correctly. `concrete_type_id` is the
/// IC-stabilization key per §Q25.C.6.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    /// Concrete-type discriminator — matches `VTable::concrete_type_id`
    /// when the generic argument is itself a trait-object.
    pub concrete_type_id: u32,
    /// If the generic parameter has a trait bound (`G: Bound`), this is
    /// the bound's vtable for the concrete type. `None` when the
    /// parameter is unbounded.
    pub vtable_for_bound: Option<std::sync::Arc<VTable>>,
    /// Size and alignment of the concrete type, in bytes.
    pub size_align: (u32, u32),
}

// ── Erase_T substitution + thunk-construction descriptors ───────────────────
//
// ADR-006 §2.7.24 Q25.C.1 — universal-dyn auto-boxing rule. `ErasureType`
// is the storage-tier mirror of the compiler's `Type` enum, narrowed to
// the shapes `Erase_T` operates on. Emission-tier code (W17-trait-object-
// emission) constructs `ErasureType` values from its richer Type
// representation, runs `Erase_T::rewrite`, and reads off
// `ThunkSignature` to know what shape of thunk to emit per
// `(impl, method)` pair.

/// Storage-tier projection of the method-signature types `Erase_T`
/// operates on. Mirrors the row table in ADR-006 §2.7.24 Q25.C.1:
///
/// | Input `τ` | `Erase_T(τ)` |
/// |---|---|
/// | `Self` | `dyn T` |
/// | `&Self` / `&mut Self` | `&dyn T` / `&mut dyn T` |
/// | `Self::A` w/ bound | `dyn Bound` |
/// | `Self::A` w/o bound | **ETO-001 compile error** |
/// | `G<τ₁, ...>` w/ erasure-safe G | recurse |
/// | method-generic G | `KindedSlot` + `TypeInfo` |
/// | concrete / builtin | unchanged |
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErasureType {
    /// `Self` — boxes into `dyn T` (the trait being erased).
    SelfType,
    /// `&Self` (immutable) — boxes into `&dyn T`.
    SelfRef,
    /// `&mut Self` — boxes into `&mut dyn T`.
    SelfRefMut,
    /// `Self::A` — projection of a Self-associated type. If `bound_trait_id`
    /// is `Some`, the assoc type's bound trait id (the type erases to
    /// `dyn Bound`); if `None`, this is an `ETO-001` compile error
    /// (associated type without a trait bound cannot be erased).
    SelfAssoc {
        assoc_name: String,
        bound_trait_id: Option<u32>,
    },
    /// An erasure-safe generic constructor (Option/Result/Vec/Box/Arc/
    /// HashMap/HashSet/tuple/user-#[erasure_safe]) with type-argument
    /// list to recurse into. `name` is the constructor's user-visible
    /// name; the emission tier maps this back to its own Type ctor.
    Generic {
        name: String,
        args: Vec<ErasureType>,
    },
    /// `&G<...>` / `&mut G<...>` — reference to a generic. Reference
    /// itself is not auto-boxed; recurses into the payload.
    Reference {
        mutable: bool,
        inner: Box<ErasureType>,
    },
    /// Method-generic parameter (`fn foo<G: Bound>(...)`). At dispatch
    /// time the call site supplies a `KindedSlot` payload + a
    /// `&TypeInfo` per generic — see §Q25.C.3.
    MethodGeneric { name: String },
    /// A concrete or builtin type (int, string, user-defined struct,
    /// closure type, etc.). Carries an opaque token so the emission
    /// tier can map back to its richer representation. `Erase_T`
    /// leaves these unchanged.
    Concrete { type_token: u32 },
}

impl ErasureType {
    /// Apply the `Erase_T(τ)` substitution per ADR-006 §2.7.24 Q25.C.1.
    /// Returns the rewritten type, plus a `wrap_targets` accumulator
    /// describing every `Self` / `Self::A` site reached (used by the
    /// emission tier to populate `VTableEntry::BoxedReturn::wrap_targets`).
    ///
    /// The `trait_id` argument is the surrounding trait's id — used
    /// for `Self` → `dyn T` (T being the surrounding trait) and as
    /// the fallback `wrap_as_trait_id` for assoc-type erasures.
    ///
    /// Returns `Err` with the ETO error code on unbounded `Self::A`
    /// per §Q25.C.1 row 5.
    pub fn rewrite(&self, trait_id: u32) -> Result<RewriteResult, ErasureError> {
        let mut wrap_targets = SmallVec::new();
        let out = Self::rewrite_inner(self, trait_id, &mut wrap_targets, &mut SmallVec::new())?;
        Ok(RewriteResult {
            erased: out,
            wrap_targets,
        })
    }

    fn rewrite_inner(
        ty: &ErasureType,
        trait_id: u32,
        wrap_targets: &mut SmallVec<[WrapTarget; 2]>,
        path: &mut SmallVec<[u8; 4]>,
    ) -> Result<ErasureType, ErasureError> {
        match ty {
            ErasureType::SelfType => {
                wrap_targets.push(WrapTarget {
                    path: path.clone(),
                    wrap_as_trait_id: trait_id,
                });
                // Self at the outermost position erases to `dyn T`
                // (encoded as a Concrete with the trait-id token —
                // emission tier reads `wrap_targets` for the actual
                // boxing-site information, the type itself becomes a
                // `dyn T` carrier).
                Ok(ErasureType::Concrete { type_token: trait_id })
            }
            ErasureType::SelfRef => {
                wrap_targets.push(WrapTarget {
                    path: path.clone(),
                    wrap_as_trait_id: trait_id,
                });
                Ok(ErasureType::Reference {
                    mutable: false,
                    inner: Box::new(ErasureType::Concrete { type_token: trait_id }),
                })
            }
            ErasureType::SelfRefMut => {
                wrap_targets.push(WrapTarget {
                    path: path.clone(),
                    wrap_as_trait_id: trait_id,
                });
                Ok(ErasureType::Reference {
                    mutable: true,
                    inner: Box::new(ErasureType::Concrete { type_token: trait_id }),
                })
            }
            ErasureType::SelfAssoc { assoc_name, bound_trait_id } => match bound_trait_id {
                Some(bound) => {
                    wrap_targets.push(WrapTarget {
                        path: path.clone(),
                        wrap_as_trait_id: *bound,
                    });
                    Ok(ErasureType::Concrete { type_token: *bound })
                }
                None => Err(ErasureError::Eto001UnboundedAssoc {
                    assoc_name: assoc_name.clone(),
                }),
            },
            ErasureType::Generic { name, args } => {
                let mut new_args = Vec::with_capacity(args.len());
                for (i, arg) in args.iter().enumerate() {
                    path.push(i as u8);
                    new_args.push(Self::rewrite_inner(arg, trait_id, wrap_targets, path)?);
                    path.pop();
                }
                Ok(ErasureType::Generic {
                    name: name.clone(),
                    args: new_args,
                })
            }
            ErasureType::Reference { mutable, inner } => {
                // References themselves are unchanged; recurse into
                // payload without pushing a path step (the reference
                // is a transparent wrapper for `Erase_T` purposes).
                let new_inner =
                    Self::rewrite_inner(inner.as_ref(), trait_id, wrap_targets, path)?;
                Ok(ErasureType::Reference {
                    mutable: *mutable,
                    inner: Box::new(new_inner),
                })
            }
            ErasureType::MethodGeneric { .. } | ErasureType::Concrete { .. } => Ok(ty.clone()),
        }
    }
}

/// Result of `Erase_T::rewrite` — the erased type plus the wrap-target
/// list for the emission-tier thunk.
#[derive(Debug, Clone)]
pub struct RewriteResult {
    pub erased: ErasureType,
    pub wrap_targets: SmallVec<[WrapTarget; 2]>,
}

/// Errors `Erase_T` can surface per §Q25.C.1 / §Q25.C.4 ETO error codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErasureError {
    /// ETO-001: `Self::A` with no trait bound cannot be erased.
    Eto001UnboundedAssoc { assoc_name: String },
    /// ETO-002: method is `#[static_only]` and cannot be called through
    /// `dyn T` (the emission tier checks this at the call site before
    /// even building a `dyn` coercion).
    Eto002StaticOnly { method_name: String },
}

/// Per-(impl, method) thunk descriptor — the data the emission tier
/// needs to generate a thunk function for one method of one impl.
///
/// The emission tier walks each impl's method list, runs `Erase_T` on
/// each method's signature, and either emits a `VTableEntry::Direct`
/// (no rewriting needed) or a thunk + the corresponding
/// `VTableEntry::{BoxedReturn, SelfArg, Generic, Compound}` per
/// §Q25.C.5.
///
/// **Per-(impl, method) cardinality.** One thunk per pair, NOT one per
/// trait — different impls of the same trait method may need
/// different thunks because the concrete return type differs. (E.g.
/// `impl Animal for Dog { fn clone_me(&self) -> Dog }` vs
/// `impl Animal for Cat { fn clone_me(&self) -> Cat }` — each clones
/// a different concrete type and boxes it under the same trait id.)
#[derive(Debug, Clone)]
pub struct ThunkSignature {
    /// Impl that owns the method (concrete-type id from the impl's
    /// matching `VTable::concrete_type_id`).
    pub impl_type_id: u32,
    /// Trait the method belongs to (`VTable::trait_names` first entry,
    /// or the surrounding trait for multi-impl cases).
    pub trait_id: u32,
    /// Method name as declared in the trait.
    pub method_name: String,
    /// Erasure flags — which of (BoxedReturn / SelfArg / Generic)
    /// apply. Empty → emission tier emits `VTableEntry::Direct` and
    /// no thunk. Single bit → emission emits the corresponding
    /// single-shape `VTableEntry::*` variant. Multiple bits →
    /// `VTableEntry::Compound`.
    pub flags: VTableEntryFlags,
    /// Wrap-targets if `BOXED_RETURN` is set (empty otherwise).
    pub wrap_targets: SmallVec<[WrapTarget; 2]>,
    /// Self-arg positions if `SELF_ARG` is set (empty otherwise).
    pub self_arg_positions: SmallVec<[u8; 4]>,
    /// Number of method-generic parameters if `GENERIC` is set
    /// (zero otherwise).
    pub type_param_count: u8,
}

impl ThunkSignature {
    /// Build a thunk signature from per-arg / per-return erasure
    /// results. The emission tier calls this once per (impl, method)
    /// pair after running `Erase_T` on each component of the
    /// signature.
    ///
    /// - `return_wrap_targets` — `wrap_targets` from running `Erase_T`
    ///   on the method's return type. Non-empty ⇒ `BOXED_RETURN`.
    /// - `self_arg_positions` — indices (0-based, excluding receiver)
    ///   of arguments declared as `Self` in the trait signature.
    ///   Non-empty ⇒ `SELF_ARG`.
    /// - `type_param_count` — number of `<G>` method-generic
    ///   parameters. Non-zero ⇒ `GENERIC`.
    pub fn build(
        impl_type_id: u32,
        trait_id: u32,
        method_name: String,
        return_wrap_targets: SmallVec<[WrapTarget; 2]>,
        self_arg_positions: SmallVec<[u8; 4]>,
        type_param_count: u8,
    ) -> Self {
        let mut flags = VTableEntryFlags::empty();
        if !return_wrap_targets.is_empty() {
            flags.set(VTableEntryFlags::BOXED_RETURN);
        }
        if !self_arg_positions.is_empty() {
            flags.set(VTableEntryFlags::SELF_ARG);
        }
        if type_param_count > 0 {
            flags.set(VTableEntryFlags::GENERIC);
        }
        Self {
            impl_type_id,
            trait_id,
            method_name,
            flags,
            wrap_targets: return_wrap_targets,
            self_arg_positions,
            type_param_count,
        }
    }

    /// Whether this method needs no thunk at all (Direct dispatch).
    pub fn is_direct(&self) -> bool {
        self.flags.bits() == 0
    }

    /// Build the corresponding `VTableEntry` once the emission tier
    /// has assigned `function_id` (for Direct) / `thunk_id`
    /// (otherwise). `Direct` entries get `function_id`; Compound /
    /// shape entries get the thunk id and the emission-tier-
    /// allocated thunk function id is stored elsewhere.
    pub fn to_vtable_entry(&self, function_or_thunk_id: u16) -> VTableEntry {
        let bits = self.flags.bits();
        if bits == 0 {
            return VTableEntry::Direct {
                function_id: function_or_thunk_id,
            };
        }
        let one_bit_set = bits.count_ones() == 1;
        if one_bit_set {
            if self.flags.is_boxed_return() {
                return VTableEntry::BoxedReturn {
                    thunk_id: function_or_thunk_id,
                    wrap_targets: self.wrap_targets.clone(),
                };
            }
            if self.flags.is_self_arg() {
                return VTableEntry::SelfArg {
                    thunk_id: function_or_thunk_id,
                    self_arg_positions: self.self_arg_positions.clone(),
                };
            }
            if self.flags.is_generic() {
                return VTableEntry::Generic {
                    thunk_id: function_or_thunk_id,
                    type_param_count: self.type_param_count,
                };
            }
        }
        VTableEntry::Compound {
            thunk_id: function_or_thunk_id,
            flags: self.flags,
            wrap_targets: self.wrap_targets.clone(),
            self_arg_positions: self.self_arg_positions.clone(),
            type_param_count: self.type_param_count,
        }
    }
}

#[cfg(test)]
mod erase_t_tests {
    //! W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C, 2026-05-11):
    //! pin the `Erase_T` substitution + `ThunkSignature::build` shape
    //! contracts. These are storage-tier compiler-side rewriting
    //! tests; the emission tier consumes these to drive thunk
    //! generation.
    use super::*;

    fn concrete(token: u32) -> ErasureType {
        ErasureType::Concrete { type_token: token }
    }

    #[test]
    fn erase_self_at_top_level_pushes_wrap_target_with_empty_path() {
        // `fn clone(&self) -> Self` → return type `Self`
        // Erase_T → `dyn T`, wrap_targets = [{ path: [], wrap_as: T }]
        let r = ErasureType::SelfType.rewrite(42).unwrap();
        assert_eq!(r.wrap_targets.len(), 1);
        assert_eq!(r.wrap_targets[0].path.as_slice(), &[] as &[u8]);
        assert_eq!(r.wrap_targets[0].wrap_as_trait_id, 42);
    }

    #[test]
    fn erase_result_of_self_pushes_path_zero() {
        // `fn try_clone(&self) -> Result<Self, Error>`
        // → wrap_targets = [{ path: [0], wrap_as: T }]
        let ty = ErasureType::Generic {
            name: "Result".to_string(),
            args: vec![ErasureType::SelfType, concrete(99)],
        };
        let r = ty.rewrite(7).unwrap();
        assert_eq!(r.wrap_targets.len(), 1);
        assert_eq!(r.wrap_targets[0].path.as_slice(), &[0u8] as &[u8]);
        assert_eq!(r.wrap_targets[0].wrap_as_trait_id, 7);
    }

    #[test]
    fn erase_tuple_of_self_self_pushes_two_targets() {
        // `fn split(self) -> (Self, Self)`
        // → wrap_targets = [{ path: [0] }, { path: [1] }]
        let ty = ErasureType::Generic {
            name: "tuple".to_string(),
            args: vec![ErasureType::SelfType, ErasureType::SelfType],
        };
        let r = ty.rewrite(11).unwrap();
        assert_eq!(r.wrap_targets.len(), 2);
        assert_eq!(r.wrap_targets[0].path.as_slice(), &[0u8] as &[u8]);
        assert_eq!(r.wrap_targets[1].path.as_slice(), &[1u8] as &[u8]);
    }

    #[test]
    fn erase_nested_option_result_self_pushes_deep_path() {
        // `fn deep(&self) -> Option<Result<Self, E>>`
        // → wrap_targets = [{ path: [0, 0] }]
        let ty = ErasureType::Generic {
            name: "Option".to_string(),
            args: vec![ErasureType::Generic {
                name: "Result".to_string(),
                args: vec![ErasureType::SelfType, concrete(50)],
            }],
        };
        let r = ty.rewrite(33).unwrap();
        assert_eq!(r.wrap_targets.len(), 1);
        assert_eq!(r.wrap_targets[0].path.as_slice(), &[0u8, 0u8] as &[u8]);
    }

    #[test]
    fn erase_unbounded_assoc_returns_eto_001() {
        let ty = ErasureType::SelfAssoc {
            assoc_name: "Item".to_string(),
            bound_trait_id: None,
        };
        let err = ty.rewrite(1).unwrap_err();
        assert!(matches!(err, ErasureError::Eto001UnboundedAssoc { .. }));
    }

    #[test]
    fn erase_bounded_assoc_erases_to_bound_trait() {
        let ty = ErasureType::SelfAssoc {
            assoc_name: "Iter".to_string(),
            bound_trait_id: Some(77),
        };
        let r = ty.rewrite(1).unwrap();
        assert_eq!(r.wrap_targets.len(), 1);
        assert_eq!(r.wrap_targets[0].wrap_as_trait_id, 77);
    }

    #[test]
    fn erase_concrete_type_is_identity_no_wrap_targets() {
        let r = concrete(42).rewrite(1).unwrap();
        assert_eq!(r.wrap_targets.len(), 0);
        assert_eq!(r.erased, concrete(42));
    }

    #[test]
    fn thunk_signature_direct_when_no_rewriting() {
        let sig = ThunkSignature::build(
            1,
            2,
            "name".to_string(),
            SmallVec::new(),
            SmallVec::new(),
            0,
        );
        assert!(sig.is_direct());
        match sig.to_vtable_entry(7) {
            VTableEntry::Direct { function_id } => assert_eq!(function_id, 7),
            _ => panic!("expected Direct"),
        }
    }

    #[test]
    fn thunk_signature_boxed_return_only() {
        let mut wts: SmallVec<[WrapTarget; 2]> = SmallVec::new();
        wts.push(WrapTarget {
            path: SmallVec::new(),
            wrap_as_trait_id: 1,
        });
        let sig = ThunkSignature::build(
            1,
            2,
            "clone".to_string(),
            wts,
            SmallVec::new(),
            0,
        );
        assert!(!sig.is_direct());
        match sig.to_vtable_entry(9) {
            VTableEntry::BoxedReturn { thunk_id, wrap_targets } => {
                assert_eq!(thunk_id, 9);
                assert_eq!(wrap_targets.len(), 1);
            }
            _ => panic!("expected BoxedReturn"),
        }
    }

    #[test]
    fn thunk_signature_compound_when_two_flags() {
        // Self in return AND a method generic — Compound.
        let mut wts: SmallVec<[WrapTarget; 2]> = SmallVec::new();
        wts.push(WrapTarget {
            path: SmallVec::new(),
            wrap_as_trait_id: 5,
        });
        let sig = ThunkSignature::build(
            1,
            5,
            "compound".to_string(),
            wts,
            SmallVec::new(),
            1,
        );
        match sig.to_vtable_entry(11) {
            VTableEntry::Compound { thunk_id, flags, type_param_count, .. } => {
                assert_eq!(thunk_id, 11);
                assert!(flags.is_boxed_return());
                assert!(flags.is_generic());
                assert!(!flags.is_self_arg());
                assert_eq!(type_param_count, 1);
            }
            _ => panic!("expected Compound"),
        }
    }
}
