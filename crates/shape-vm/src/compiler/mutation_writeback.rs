//! Mutation-writeback support for `&mut self` method dispatch (ADR-006
//! §2.7.27 / Item 4 ruling, W17-mutation-writeback, 2026-05-12).
//!
//! ## Background
//!
//! The Item 4 ruling adopts Rust-style `&mut self` opt-in for COW
//! container mutating methods. When the compiler sees `let mut s =
//! HashSet(); s.add("a")`, the `add` handler does `Arc::make_mut(&mut
//! hs)` on the receiver Arc and returns the (possibly-cloned) new Arc.
//! Pre-ruling, that new Arc was pushed onto the stack and immediately
//! discarded (the call statement had no consumer), so the binding
//! slot `s` still held the OLD Arc and the mutation was silently lost.
//!
//! The fix is **write-back at the call site**: after `CallMethod`, the
//! compiler emits `Dup; StoreLocal recv` (or `Dup;
//! StoreModuleBinding recv` for top-level bindings) so the binding
//! slot receives the new Arc identity. The result still sits on top of
//! the stack as the expression value of `s.add("a")`.
//!
//! ## Scope of `is_mut_self`
//!
//! Only COW container handlers opt in. Specifically: HashSet, HashMap,
//! Array, Deque, PriorityQueue, TypedArray. Their handlers all share
//! the shape `let mut arc = clone_receiver(); Arc::make_mut(&mut arc);
//! ...; Ok(KindedSlot::from_*(arc))`.
//!
//! Interior-mutability primitives (`Mutex`, `Atomic`, `Lazy`,
//! `Channel`) do **not** opt in. Their mutating methods (`Mutex.set`,
//! `Atomic.store`, etc.) preserve the receiver Arc's identity — the
//! mutation reaches the shared interior via `Cell` / `AtomicI64` /
//! channel-buffer. No write-back is required, and `let m = Mutex(0);
//! m.set(5)` stays valid (the binding is immutable; the shared
//! interior changes).
//!
//! ## Compile-time vs runtime
//!
//! Write-back is a **compile-time concern**. The runtime dispatch shell
//! `op_call_method` pops the receiver before invoking the handler, so
//! the source-binding identity is lost by the time the handler returns.
//! The compiler is the only side that knows "this `s.add("a")` came
//! from a local-slot binding"; it emits `Dup; StoreLocal` accordingly.
//!
//! ## Receiver-kind narrowing
//!
//! Method names like `add` overlap between mutating and non-mutating
//! types (`HashSet.add` mutates; `DateTime.add` is the operator-trait
//! backing for `+` and is pure). The compiler narrows by tracking
//! per-local container kind: `let s = HashSet()` populates
//! `mut_self_container_locals[s_local_idx] = ContainerKind::HashSet`;
//! only then does `s.add(x)` emit the write-back. Unknown receivers
//! fall through to the old functional path (the dispatch text's "silent
//! drop" decision-call for r-value receivers like `compute_set().add(x)`
//! is the same shape).

use shape_value::{KindedSlot, NativeKind, VMError};

/// Container-kind classifier for `&mut self` opt-in routing at compile
/// time.
///
/// Populated at let-binding time when the initializer is a recognized
/// container constructor (`Set()`, `HashMap()`, `Deque()`,
/// `PriorityQueue()`, `[…]`); consumed by `compile_expr_method_call`'s
/// write-back gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    /// HashSet (via `Set()` ctor).
    HashSet,
    /// HashMap (via `HashMap()` ctor).
    HashMap,
    /// Deque (via `Deque()` ctor).
    Deque,
    /// PriorityQueue (via `PriorityQueue()` ctor).
    PriorityQueue,
    /// Generic Array (via array literal `[…]`, only for non-v2-typed
    /// arrays — the v2 typed-array fast paths have their own
    /// `v2_typed_array_locals` tracking and own write-back emit).
    Array,
}

impl ContainerKind {
    /// Returns the method-name set that mutates this container kind.
    /// Method dispatch consults this to decide whether to emit a
    /// `Dup; StoreLocal` write-back after `CallMethod`.
    ///
    /// The named sets live in
    /// `crate::executor::objects::method_registry` so the registration-
    /// side and the compile-side share one source of truth.
    pub fn is_mut_self_method(self, method: &str) -> bool {
        use crate::executor::objects::method_registry as mr;
        match self {
            ContainerKind::HashSet => mr::MUT_SELF_HASHSET_METHODS.contains(method),
            ContainerKind::HashMap => mr::MUT_SELF_HASHMAP_METHODS.contains(method),
            ContainerKind::Deque => mr::MUT_SELF_DEQUE_METHODS.contains(method),
            ContainerKind::PriorityQueue => {
                mr::MUT_SELF_PRIORITY_QUEUE_METHODS.contains(method)
            }
            ContainerKind::Array => mr::MUT_SELF_ARRAY_METHODS.contains(method),
        }
    }

    /// Classifier for ctor names that produce a known container kind.
    /// Returns `Some(kind)` when the named builtin / type constructor
    /// produces a COW container the write-back layer covers.
    pub fn from_ctor_name(name: &str) -> Option<Self> {
        match name {
            "Set" | "HashSet" => Some(ContainerKind::HashSet),
            "HashMap" => Some(ContainerKind::HashMap),
            "Deque" => Some(ContainerKind::Deque),
            "PriorityQueue" => Some(ContainerKind::PriorityQueue),
            _ => None,
        }
    }

    /// Classifier from a `KindedSlot.kind` at runtime — used by the
    /// dispatch-shell when (in a future hardening) write-back wants to
    /// be runtime-driven. Currently only the compile-time path uses
    /// this enum, but the mapping is here for symmetry.
    #[allow(dead_code)]
    pub fn from_kinded_slot(slot: &KindedSlot) -> Option<Self> {
        match slot.kind {
            NativeKind::Ptr(shape_value::HeapKind::HashSet) => Some(ContainerKind::HashSet),
            NativeKind::Ptr(shape_value::HeapKind::HashMap) => Some(ContainerKind::HashMap),
            NativeKind::Ptr(shape_value::HeapKind::Deque) => Some(ContainerKind::Deque),
            NativeKind::Ptr(shape_value::HeapKind::PriorityQueue) => {
                Some(ContainerKind::PriorityQueue)
            }
            NativeKind::Ptr(shape_value::HeapKind::TypedArray) => Some(ContainerKind::Array),
            _ => None,
        }
    }
}

/// Reserved future surface: the place a writeback target can name. Used
/// by the runtime dispatch path when a future hardening pass moves the
/// write-back into `op_call_method` itself.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum WriteBackTarget {
    Local(u16),
    ModuleBinding(u16),
}

/// Compile-time write-back target resolved at the method-call site by
/// `resolve_mut_self_writeback_target`. Consumed by the post-CallMethod
/// emit step to lay down `Dup; StoreLocal` (or `Dup;
/// StoreModuleBinding`) so the binding slot receives the new (possibly
/// Arc-cloned) receiver Arc.
#[derive(Debug, Clone, Copy)]
pub enum MutSelfWriteBackTarget {
    Local(u16),
    ModuleBinding(u16),
}

/// Reserved future surface — runtime helper for write-back through a
/// `WriteBackTarget` after the handler returns. Currently unused; the
/// compile-time emission path covers Commit 1. Kept as a stub so the
/// follow-up hardening can wire it in without churning the module
/// shape.
#[allow(dead_code)]
pub fn writeback_result(
    _vm: &mut crate::executor::VirtualMachine,
    _target: WriteBackTarget,
    _result: &KindedSlot,
) -> Result<(), VMError> {
    Err(VMError::NotImplemented(
        "mutation_writeback::writeback_result: reserved for a future runtime-driven \
         hardening pass; the W17-mutation-writeback close uses compile-time \
         `Dup; StoreLocal` emission. ADR-006 §2.7.27."
            .to_string(),
    ))
}
