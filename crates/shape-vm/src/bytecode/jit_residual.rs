//! Per-owner attribution of JIT-unsupported constructs (ADR-018 §2, #187).
//!
//! Before this module the compiler recorded each construct the JIT cannot
//! lower soundly as a single program-wide `bool` on `BytecodeProgram`
//! (`has_try_unwrap_residual`, `has_null_coalesce_residual`, …). The JIT
//! executor read those flags and abandoned native execution of the ENTIRE
//! program, so one `?` in one cold helper cost every hot function its native
//! code.
//!
//! ADR-018 §2 makes deopt granularity per-function: an unsupported construct
//! costs its enclosing function native execution, never the program. This
//! module carries the attribution that makes that possible. Each residual is
//! recorded against its owner at the exact point the bytecode compiler emits
//! the construct, using `BytecodeCompiler::current_function` — the same
//! `Option<usize>` function-index convention `monomorphized_method_call_sites`
//! already uses for its caller key (`None` = top-level/module code).
//!
//! The soundness contract is unchanged and must stay unchanged: a residual
//! bearing owner NEVER runs native. Per-function granularity narrows WHO
//! falls back, not WHETHER the unsound lowering is refused.

use std::collections::{BTreeMap, BTreeSet};

/// How much native execution a residual costs.
///
/// Demoting only the enclosing function is correct when the unsound lowering
/// is emitted INSIDE that function. It is not correct when the unsoundness
/// spans a producer/consumer pair that the attribution covers only one half
/// of, or when the demoted function shares mutable runtime state with native
/// code that the trampoline VM does not see. Those residuals keep the
/// whole-program refusal, and the shrink-only baseline records each one with
/// its reason so the floor is visible and can only go down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResidualScope {
    /// The unsound lowering is emitted inside the owner; demoting the owner
    /// to interpreted execution is sufficient, and siblings keep native code.
    Owner,
    /// The whole program must run interpreted. `reason` says why narrowing
    /// the refusal to the owner would be unsound.
    Program,
}

/// A construct the JIT cannot lower soundly, so its owner runs interpreted.
///
/// Each variant names a specific, diagnosed VM/JIT divergence. The variant
/// list is the whole-program-bail baseline's vocabulary — `stable_id` is the
/// identifier the shrink-only ratchet records, so renaming a variant without
/// regenerating the baseline is a gate failure, not a silent drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum JitResidual {
    /// The `?` operator (`OpCode::TryUnwrap`). MIR lowers it as a transparent
    /// copy, so the JIT stamps the SUCCESS type's `NativeKind` on a slot
    /// holding heap `Arc<ResultData>` bits.
    TryUnwrap,
    /// The `??` operator (`OpCode::CoalesceProbe`). The JIT models it as `Eq`
    /// against `None` with no `Arc<OptionData>` unwrap, leaking the `Some(v)`
    /// wrapper.
    NullCoalesce,
    /// An imported `pub const` inlined at use as `PushConst(<value>)`. The
    /// JIT's direct-identifier-eval lowering fires the print FFI with
    /// zero-init bits.
    ImportedConstInline,
    /// A direct call to an imported stdlib function. The JIT-side
    /// `jit_call_value` ModuleFn arm returns `TAG_NULL` silently instead of
    /// surfacing the W17 marshal-return arms the VM surfaces.
    ModuleFnMarshalReturn,
    /// A reference returned via the escape→RC `PromotedCell` carrier. The JIT
    /// has no PromotedCell deref lowering and reads the reference pointer
    /// instead of the referent.
    ReferenceEscapePromotion,
}

impl JitResidual {
    /// Every residual kind, in declaration order.
    pub const ALL: [JitResidual; 5] = [
        JitResidual::TryUnwrap,
        JitResidual::NullCoalesce,
        JitResidual::ImportedConstInline,
        JitResidual::ModuleFnMarshalReturn,
        JitResidual::ReferenceEscapePromotion,
    ];

    /// Stable identifier recorded in the whole-program-bail baseline.
    pub fn stable_id(&self) -> &'static str {
        match self {
            JitResidual::TryUnwrap => "try-unwrap",
            JitResidual::NullCoalesce => "null-coalesce",
            JitResidual::ImportedConstInline => "imported-const-inline",
            JitResidual::ModuleFnMarshalReturn => "module-fn-marshal-return",
            JitResidual::ReferenceEscapePromotion => "reference-escape-promotion",
        }
    }

    /// How much native execution this residual costs.
    ///
    /// The two `Program`-scoped entries are the recorded floor of the #187
    /// shrink-only ratchet. Converting one to `Owner` requires removing the
    /// hazard named in `program_scope_reason`, not just changing this arm.
    pub fn scope(&self) -> ResidualScope {
        match self {
            JitResidual::TryUnwrap
            | JitResidual::NullCoalesce
            | JitResidual::ImportedConstInline => ResidualScope::Owner,
            JitResidual::ModuleFnMarshalReturn | JitResidual::ReferenceEscapePromotion => {
                ResidualScope::Program
            }
        }
    }

    /// Why a `Program`-scoped residual cannot be narrowed to its owner.
    ///
    /// `None` for `Owner`-scoped residuals.
    pub fn program_scope_reason(&self) -> Option<&'static str> {
        match self {
            JitResidual::ModuleFnMarshalReturn => Some(
                "the call resolves its callee through a hidden native module \
                 binding, so demoting only the owner would interleave native \
                 top-level code with an interpreted function reading the \
                 module-binding array — the same desynchronization the W39 F1 \
                 function-body module-binding refusal exists to prevent",
            ),
            JitResidual::ReferenceEscapePromotion => Some(
                "the residual is recorded against the CONSUMER of the returned \
                 reference, but the `PromotedCell` carrier is created by the \
                 PRODUCER (`fn f() -> &T`), which is not recorded. Demoting only \
                 the consumer would leave a native producer returning a raw \
                 stack address for an interpreted consumer to deref through the \
                 PromotedCell arm",
            ),
            JitResidual::TryUnwrap
            | JitResidual::NullCoalesce
            | JitResidual::ImportedConstInline => None,
        }
    }

    /// One-line reason for the `[jit-fallback]` diagnostic.
    pub fn reason(&self) -> &'static str {
        match self {
            JitResidual::TryUnwrap => {
                "the `?` operator: MIR lowers it as a transparent copy, so the \
                 JIT would stamp the success type's NativeKind onto heap \
                 Result/Option bits"
            }
            JitResidual::NullCoalesce => {
                "the `??` operator: the JIT models it as `Eq` against `None` \
                 with no `Arc<OptionData>` unwrap and would leak the `Some(v)` \
                 wrapper"
            }
            JitResidual::ImportedConstInline => {
                "an imported `pub const` inlined at use: the JIT \
                 direct-identifier-eval lowering fires the print FFI with \
                 zero-init bits"
            }
            JitResidual::ModuleFnMarshalReturn => {
                "a direct call to an imported stdlib function: the JIT \
                 `jit_call_value` ModuleFn arm returns TAG_NULL silently \
                 instead of surfacing the marshal-return arms the VM surfaces"
            }
            JitResidual::ReferenceEscapePromotion => {
                "a reference returned via the escape→RC `PromotedCell` carrier: \
                 the JIT has no PromotedCell deref lowering and would read the \
                 reference pointer instead of the referent"
            }
        }
    }
}

/// Which owner a residual belongs to: a function index, or top-level code.
///
/// `None` is top-level (module) code, matching the bytecode compiler's
/// `current_function` convention.
pub type ResidualOwner = Option<usize>;

/// Per-owner residual attribution for one compiled program.
///
/// Not serialised: like the sibling `has_*_residual` flags it mirrors, this is
/// compile-time state recomputed by the cached-program reload path, and the
/// function indices are opaque per-program table positions rather than a
/// stable wire shape.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JitResidualMap {
    entries: BTreeMap<ResidualOwner, BTreeSet<JitResidual>>,
}

impl JitResidualMap {
    /// Attribute `residual` to `owner`.
    pub fn record(&mut self, owner: ResidualOwner, residual: JitResidual) {
        self.entries.entry(owner).or_default().insert(residual);
    }

    /// Residuals recorded against top-level (module) code.
    pub fn top_level(&self) -> impl Iterator<Item = JitResidual> + '_ {
        self.owned_by(None)
    }

    /// Residuals recorded against the function at `index`.
    pub fn for_function(&self, index: usize) -> impl Iterator<Item = JitResidual> + '_ {
        self.owned_by(Some(index))
    }

    /// Whether the function at `index` carries any residual.
    pub fn function_is_residual_bearing(&self, index: usize) -> bool {
        self.entries
            .get(&Some(index))
            .is_some_and(|s| !s.is_empty())
    }

    /// Whether top-level code carries any residual — top-level IS the program
    /// entry the JIT compiles as `main`, so there is no smaller unit to
    /// demote and the program runs interpreted.
    pub fn top_level_is_residual_bearing(&self) -> bool {
        self.entries.get(&None).is_some_and(|s| !s.is_empty())
    }

    /// Residuals anywhere in the program whose scope is `Program` — they cost
    /// the whole program its native execution regardless of which owner holds
    /// them. This is the quantity the #187 shrink-only ratchet bounds.
    pub fn program_scoped(&self) -> impl Iterator<Item = JitResidual> + '_ {
        let mut seen = BTreeSet::new();
        self.entries
            .values()
            .flat_map(|set| set.iter().copied())
            .filter(|r| r.scope() == ResidualScope::Program)
            .filter(move |r| seen.insert(*r))
    }

    /// Whether any `Program`-scoped residual is present.
    pub fn has_program_scoped(&self) -> bool {
        self.entries
            .values()
            .flat_map(|set| set.iter())
            .any(|r| r.scope() == ResidualScope::Program)
    }

    /// Whether nothing at all was recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.values().all(|s| s.is_empty())
    }

    /// Every owner carrying at least one residual, top-level first.
    pub fn owners(&self) -> impl Iterator<Item = ResidualOwner> + '_ {
        self.entries
            .iter()
            .filter(|(_, set)| !set.is_empty())
            .map(|(owner, _)| *owner)
    }

    fn owned_by(&self, owner: ResidualOwner) -> impl Iterator<Item = JitResidual> + '_ {
        self.entries
            .get(&owner)
            .into_iter()
            .flat_map(|set| set.iter().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residual_recorded_against_a_function_does_not_reach_top_level() {
        let mut map = JitResidualMap::default();
        map.record(Some(3), JitResidual::TryUnwrap);

        assert!(map.function_is_residual_bearing(3));
        assert!(!map.function_is_residual_bearing(0));
        assert!(!map.top_level_is_residual_bearing());
        assert!(!map.is_empty());
    }

    #[test]
    fn top_level_residual_is_distinguishable_from_a_function_residual() {
        let mut map = JitResidualMap::default();
        map.record(None, JitResidual::NullCoalesce);

        assert!(map.top_level_is_residual_bearing());
        assert!(!map.function_is_residual_bearing(0));
        assert_eq!(
            map.top_level().collect::<Vec<_>>(),
            vec![JitResidual::NullCoalesce]
        );
    }

    #[test]
    fn recording_the_same_residual_twice_keeps_one_entry() {
        let mut map = JitResidualMap::default();
        map.record(Some(1), JitResidual::TryUnwrap);
        map.record(Some(1), JitResidual::TryUnwrap);
        map.record(Some(1), JitResidual::NullCoalesce);

        assert_eq!(map.for_function(1).count(), 2);
        assert_eq!(map.owners().collect::<Vec<_>>(), vec![Some(1)]);
    }

    #[test]
    fn stable_ids_are_unique_across_every_residual_kind() {
        let ids: BTreeSet<&str> = JitResidual::ALL.iter().map(|r| r.stable_id()).collect();
        assert_eq!(ids.len(), JitResidual::ALL.len());
    }

    #[test]
    fn every_program_scoped_residual_records_why_it_cannot_be_narrowed() {
        for residual in JitResidual::ALL {
            match residual.scope() {
                ResidualScope::Program => assert!(
                    residual.program_scope_reason().is_some(),
                    "{} is Program-scoped and must record why narrowing it to \
                     its owner would be unsound",
                    residual.stable_id()
                ),
                ResidualScope::Owner => assert!(
                    residual.program_scope_reason().is_none(),
                    "{} is Owner-scoped and must not carry a whole-program \
                     refusal reason",
                    residual.stable_id()
                ),
            }
        }
    }

    #[test]
    fn program_scoped_residuals_are_collected_across_owners_without_duplicates() {
        let mut map = JitResidualMap::default();
        map.record(Some(0), JitResidual::TryUnwrap);
        map.record(Some(1), JitResidual::ReferenceEscapePromotion);
        map.record(Some(2), JitResidual::ReferenceEscapePromotion);

        assert!(map.has_program_scoped());
        assert_eq!(
            map.program_scoped().collect::<Vec<_>>(),
            vec![JitResidual::ReferenceEscapePromotion]
        );
    }

    #[test]
    fn an_owner_scoped_residual_alone_leaves_the_program_natively_compilable() {
        let mut map = JitResidualMap::default();
        map.record(Some(4), JitResidual::TryUnwrap);
        map.record(Some(4), JitResidual::NullCoalesce);

        assert!(!map.has_program_scoped());
        assert!(map.function_is_residual_bearing(4));
        assert!(!map.top_level_is_residual_bearing());
    }
}
