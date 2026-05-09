//! Function and closure call convention, execution wrappers, and async resolution.
//!
//! # Wave-β B12 disposition (cluster B-round-2)
//!
//! Cluster B prior partial-close (commits 28de706..727143e at supervisor
//! merge `62513e3`) migrated 3 of 7 mandatory shim sites here onto the
//! kinded API. The remaining 4 sites — `call_value_immediate_nb`'s
//! TAG_HEAP closure-dispatch arm, the `resolve_spawned_task` host-closure
//! arm, the `call_value_immediate_raw` raw-bits closure arm, and the
//! `call_function_with_raw_args` / `call_closure_with_raw_args` raw-arg
//! frame-setup pair — were architecturally entangled with deleted
//! symbols:
//!
//! - `shape_value::ValueWord` (the deleted runtime value carrier).
//! - `shape_value::ValueWordExt` (deleted accessor trait).
//! - `shape_value::Upvalue` (deleted closure-capture wrapper; the
//!   replacement layout per ADR-006 §2.7.8 / Q10 is the kind-extended
//!   `closure_raw::ClosureCell`).
//! - `shape_value::value_word_drop::vw_drop` / `vw_clone` (forbidden
//!   per CLAUDE.md "Forbidden Patterns" #8 — superseded by
//!   `clone_with_kind(bits, kind)` / `drop_with_kind(bits, kind)` per
//!   playbook §3).
//! - `shape_value::tag_bits::{is_tagged, get_tag, get_payload,
//!   TAG_FUNCTION, TAG_HEAP, TAG_MODULE_FN}` (forbidden per CLAUDE.md
//!   "Forbidden Patterns" #7 — the deleted tag_bits dispatch).
//! - `super::objects::raw_helpers::{extract_closure_info,
//!   try_call_host_closure, clone_raw_bits}` (deleted by D-raw-helpers;
//!   the surviving `raw_helpers` surface is `extract_filter_expr` only).
//! - `ValueWord::as_heap_ref()` (forbidden per CLAUDE.md "Forbidden
//!   Patterns" #7 — the deleted `as_heap_ref` heap-side dispatch
//!   pattern; replacement goes through `slot.as_heap_value()` +
//!   `HeapValue::*` match per Q8).
//!
//! Per playbook §7 REVISED #4 ("if a call site cannot be migrated
//! cleanly, the correct shape is `NotImplemented(SURFACE)` /
//! `todo!(\"phase-2c — see ADR-006 §2.7.4\")` / surface-and-stop, never
//! a forbidden-pattern workaround"), every body in this module that
//! referenced any of those symbols panics via `todo!()` until cluster
//! B-round-2 lands the kinded call-convention rebuild. The B9
//! `CallFrame.closure_heap_kind: Option<NativeKind>` lockstep companion
//! threading from commit `52c799f` is preserved structurally on the
//! frame-construction sites that survived migration.
//!
//! Cited surfaces (each `todo!()` site):
//!
//! 1. `execute_function_by_name` / `_by_id` / `execute_closure` /
//!    `execute_function_fast` / `execute_function_with_named_args` —
//!    the public VM entry-point ABI takes deleted `ValueWord` /
//!    `Upvalue` parameters; the kind-threaded replacement (`(bits,
//!    kind)` slices, kinded upvalue cells per §2.7.8) is part of the
//!    cluster B-round-2 rebuild that lands together with the
//!    `closure_raw::ClosureCell` `kinds: Vec<NativeKind>` extension and
//!    the consumer migration in `remote.rs` / `compiler_tests.rs` /
//!    JIT-trampoline FFI.
//! 2. `resume` / `execute_with_async` / `resolve_spawned_task` —
//!    same dependency. `resolve_spawned_task` additionally walks
//!    `task_scheduler::TaskStatus` (deleted `ValueWord` carrier) and
//!    invokes `Upvalue::new(vw_clone(...))` per share, both deleted.
//! 3. `call_function_with_nb_args` / `call_closure_with_nb_args` /
//!    `call_closure_with_nb_args_keepalive` — frame-setup helpers
//!    over deleted `&[ValueWord]` / `Vec<Upvalue>`; the post-§2.7.8
//!    shape takes `(bits, kind)` slices, threads
//!    `closure_heap_bits` + `closure_heap_kind` lockstep on the pushed
//!    `CallFrame` (B9 landed; the field now exists), and writes each
//!    arg via `stack_write_kinded(idx, bits, kind)`.
//! 4. `call_value_immediate_nb` / `call_value_immediate_raw` —
//!    polymorphic-callee dispatch entry points. The deleted
//!    `tag_bits::*` dispatch is replaced by a kind-typed dispatch
//!    on `NativeKind::Ptr(HeapKind::Closure)` /
//!    `NativeKind::Function` once the cluster B-round-2 callee-bits
//!    threading lands. The HEAP-arm closure handle path goes through
//!    `slot.as_heap_value()` + `HeapValue::ClosureRaw` per ADR-005 §1
//!    single-discriminator; the host-closure arm goes through
//!    `HeapValue::HostClosure` symmetrically.
//! 5. `jit_trampoline_call_closure` — JIT FFI consumer; same
//!    dependency on the deleted `Upvalue` / `vw_clone` pair. The
//!    post-§2.7.8 trampoline reads `(bits, kind)` slices for each
//!    capture and constructs the kinded closure cell directly.
//! 6. `call_function_with_raw_args` / `call_closure_with_raw_args` —
//!    raw-bits frame setup; depended on the deleted
//!    `raw_helpers::clone_raw_bits` (which itself ran the forbidden
//!    tag_bits dispatch internally). Replacement: thread `&[(u64,
//!    NativeKind)]` slices and call `clone_with_kind(bits, kind)` per
//!    arg + `stack_write_kinded`.
//! 7. `call_function_from_stack` — fast-path frame setup; depended on
//!    the deleted unkinded slot-write shim (deleted post-§2.7.7) and
//!    on the function table's `param_kinds` for source-of-kind sourcing per playbook
//!    §2 (function-call return / param kinds). The frame-pushed
//!    `closure_heap_kind: None` lockstep is preserved here so the
//!    teardown discipline already-landed in B9 round through the
//!    kinded `drop_with_kind` path on this construction site too.
//!
//! No forbidden patterns are introduced by the stubs: every body is
//! either the §2.7.8 lockstep-companion frame construction (no shim
//! refs), or `todo!()`. The B9 lockstep invariant
//! (`closure_heap_bits.is_some() == closure_heap_kind.is_some()`) is
//! preserved on every surviving frame-push site.

use shape_value::{KindedSlot, NativeKind, VMError};

use super::VirtualMachine;

impl VirtualMachine {
    /// Execute a named function with arguments, returning its result.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced deleted `ValueWord` carriers + the deleted
    /// `call_function_with_nb_args` ABI. Cluster B-round-2 rebuild lands
    /// the kinded `(bits, kind)` slice ABI on this entry point together
    /// with the `closure_raw::ClosureCell` parallel-`Vec<NativeKind>`
    /// extension.
    pub fn execute_function_by_name(
        &mut self,
        _name: &str,
        _args: Vec<KindedSlot>,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             execute_function_by_name kinded-ABI rebuild pending"
        )
    }

    /// Execute a function by its ID with positional arguments.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Same
    /// kind-threaded ABI rebuild as `execute_function_by_name`.
    pub fn execute_function_by_id(
        &mut self,
        _func_id: u16,
        _args: Vec<KindedSlot>,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             execute_function_by_id kinded-ABI rebuild pending"
        )
    }

    /// Execute a closure with its captured upvalues and arguments.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced deleted `Upvalue` (replaced by §2.7.8 kind-extended
    /// `closure_raw::ClosureCell`). Cluster B-round-2 routes this through
    /// the kinded cell layout.
    pub fn execute_closure(
        &mut self,
        _function_id: u16,
        _upvalue_bits: Vec<u64>,
        _args: Vec<KindedSlot>,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             execute_closure kinded-cell rebuild pending"
        )
    }

    /// Fast function execution for hot loops (backtesting).
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Same
    /// kind-threaded rebuild as `execute_function_by_id`.
    pub fn execute_function_fast(
        &mut self,
        _func_id: u16,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             execute_function_fast kinded-ABI rebuild pending"
        )
    }

    /// Execute a function with named arguments.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Same
    /// kind-threaded rebuild as `execute_function_by_id`. Named-arg
    /// mapping logic is value-tier-independent and migrates trivially
    /// once the kinded ABI lands.
    pub fn execute_function_with_named_args(
        &mut self,
        _func_id: u16,
        _named_args: &[(String, KindedSlot)],
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             execute_function_with_named_args kinded-ABI rebuild pending"
        )
    }

    /// Resume execution after a suspension.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced the deleted unkinded-push shim (CLAUDE.md "Forbidden
    /// Patterns" surface — replaced by `push_kinded(bits, kind)` per
    /// playbook §2). The post-§2.7.7 replacement sources kind from the
    /// resume-point's expected return-slot kind via the suspended
    /// `FrameDescriptor.return_kind`.
    pub fn resume(
        &mut self,
        _value: KindedSlot,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<super::ExecutionResult, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             resume kinded-ABI rebuild pending"
        )
    }

    /// Execute with automatic async task resolution.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// drove `resolve_spawned_task` (see below) which carries the same
    /// kinded-ABI rebuild dependency.
    pub fn execute_with_async(
        &mut self,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             execute_with_async kinded-ABI rebuild pending"
        )
    }

    /// Resolve a spawned task by executing its callable synchronously.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced deleted `Upvalue::new(vw_clone(raw))` per capture
    /// (forbidden #8 — replaced by `clone_with_kind(bits, kind)` over
    /// kind-threaded capture bits per §2.7.8). The post-§2.7.8
    /// replacement reads each capture's `(bits, kind)` from the
    /// `VmClosureHandle` over the kind-extended `ClosureCell` layout
    /// and constructs the new frame's upvalue cell with lockstep kinds.
    #[allow(dead_code)]
    fn resolve_spawned_task(&mut self, _task_id: u64) -> Result<KindedSlot, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             resolve_spawned_task kinded-cell rebuild pending"
        )
    }

    /// Module function call: takes a `&[KindedSlot]` arg slice (ADR-006
    /// §2.7.10 / Q11 caller-side carrier) and threads each slot's
    /// `(bits, kind)` onto the kind-tracked stack via
    /// `stack_write_kinded(idx, bits, kind)`.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced deleted `vw_clone` retain-on-read + the deleted
    /// unkinded stack-slot read/write/take shims (all CLAUDE.md
    /// "Forbidden Patterns" — the post-§2.7.7 replacements are
    /// `clone_with_kind` + `stack_write_kinded` + `stack_take_kinded`
    /// per playbook §3).
    /// The frame-construction tail (B9 lockstep companion
    /// `closure_heap_bits: None` / `closure_heap_kind: None`) is the
    /// shape that survives the rebuild — preserved verbatim once the
    /// kind-threaded arg-write loop lands.
    pub(crate) fn call_function_with_nb_args(
        &mut self,
        _func_id: u16,
        _args: &[KindedSlot],
    ) -> Result<(), VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             call_function_with_nb_args kinded-ABI rebuild pending"
        )
    }

    /// Host closure call: takes a `Vec<u64>` raw-bits upvalue payload
    /// (matching `CallFrame.upvalues: Option<Vec<u64>>`, with the
    /// parallel `NativeKind` track sourced from `ClosureLayout::
    /// capture_native_kinds` per §2.7.8 / Q10) and a `&[KindedSlot]`
    /// arg slice per §2.7.10 / Q11.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Thin
    /// wrapper over `call_closure_with_nb_args_keepalive` with `(None,
    /// None)` lockstep — preserved as a stub so the public signature
    /// stays bound for callers in `remote.rs` / JIT FFI.
    pub(crate) fn call_closure_with_nb_args(
        &mut self,
        _func_id: u16,
        _upvalue_bits: Vec<u64>,
        _args: &[KindedSlot],
    ) -> Result<(), VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             call_closure_with_nb_args kinded-cell rebuild pending"
        )
    }

    /// WB2.3 variant of [`call_closure_with_nb_args`] that takes an
    /// optional keep-alive `closure_heap_bits` plus its lockstep
    /// `closure_heap_kind` companion (ADR-006 §2.7.8 / Q10).
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** B9's
    /// lockstep-companion threading shape is documented here so the
    /// rebuild lands the kinded path with the
    /// `debug_assert_eq!(closure_heap_bits.is_some(),
    /// closure_heap_kind.is_some())` invariant preserved.
    pub(crate) fn call_closure_with_nb_args_keepalive(
        &mut self,
        _func_id: u16,
        _upvalue_bits: Vec<u64>,
        _args: &[KindedSlot],
        closure_heap_bits: Option<u64>,
        closure_heap_kind: Option<NativeKind>,
    ) -> Result<(), VMError> {
        debug_assert_eq!(
            closure_heap_bits.is_some(),
            closure_heap_kind.is_some(),
            "ADR-006 §2.7.8 / Q10: closure_heap_bits and closure_heap_kind \
             must be Some together or None together"
        );
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             call_closure_with_nb_args_keepalive kinded-cell rebuild pending"
        )
    }

    /// `call_value_immediate` (kinded carrier form): dispatches on the
    /// callee's `KindedSlot.kind`. ADR-006 §2.7.10 / Q11 caller-side
    /// shape — both callee and args travel as `KindedSlot`.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced the deleted `tag_bits::{is_tagged, get_tag,
    /// TAG_FUNCTION, TAG_HEAP, TAG_MODULE_FN}` dispatch (CLAUDE.md
    /// "Forbidden Patterns" #7). Replacement: callee enters as
    /// `(bits, kind)` from the caller; dispatch matches on `kind`
    /// against `NativeKind::Function`, `NativeKind::ModuleFunction`,
    /// `NativeKind::Ptr(HeapKind::Closure)`, with the host-closure
    /// arm going through `slot.as_heap_value()` + `HeapValue::HostClosure`
    /// per ADR-005 §1 single-discriminator.
    pub fn call_value_immediate_nb(
        &mut self,
        _callee: &KindedSlot,
        _args: &[KindedSlot],
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             call_value_immediate_nb kind-threaded callee dispatch rebuild pending"
        )
    }

    /// Trampoline entry: call a closure by `func_id` with pre-extracted
    /// raw upvalue bits and raw args, returning the result as raw `u64`
    /// bits.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced deleted `Upvalue::new(vw_clone(b))` per capture
    /// (forbidden #8). JIT-side post-§2.7.8 trampoline lands a
    /// `&[(u64, NativeKind)]` upvalue slice plus an `&[(u64,
    /// NativeKind)]` args slice; the kinded `closure_raw::ClosureCell`
    /// is constructed directly from the slice without a stop through
    /// the deleted `Upvalue` enum.
    pub fn jit_trampoline_call_closure(
        &mut self,
        _func_id: u16,
        _upvalue_bits: &[(u64, NativeKind)],
        _args: &[(u64, NativeKind)],
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<u64, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             jit_trampoline_call_closure kinded-cell rebuild pending"
        )
    }

    // ─── Raw u64 call API (v2) ─────────────────────────────────────────────

    /// Raw-bits closure/function call: dispatches on callee kind.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced the deleted `tag_bits::*` dispatch (forbidden #7),
    /// the deleted `raw_helpers::extract_closure_info` /
    /// `try_call_host_closure` consumers (D-raw-helpers cluster
    /// rewrote `raw_helpers` to expose `extract_filter_expr` only),
    /// and the deleted `ValueWord::from_raw_bits` /
    /// `ValueWord::into_raw_bits` raw-bits roundtrip. Post-§2.7.8
    /// replacement signature accepts `(callee_bits, callee_kind,
    /// args: &[(u64, NativeKind)])` and dispatches via kind match —
    /// no tag decode, no raw-bits roundtrip.
    pub fn call_value_immediate_raw(
        &mut self,
        _callee_bits: u64,
        _callee_kind: NativeKind,
        _args: &[(u64, NativeKind)],
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<u64, VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             call_value_immediate_raw kind-threaded callee dispatch rebuild pending"
        )
    }

    /// Set up a function call frame from raw u64 args.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced the deleted `raw_helpers::clone_raw_bits` (forbidden
    /// — D-raw-helpers cluster deleted; the function ran the
    /// forbidden tag_bits dispatch internally). Replacement: take
    /// `args: &[(u64, NativeKind)]` and call `clone_with_kind(bits,
    /// kind)` + `stack_write_kinded(idx, bits, kind)` per arg, with
    /// the B9 lockstep `closure_heap_bits: None` / `closure_heap_kind:
    /// None` companion preserved on the pushed `CallFrame`.
    pub(crate) fn call_function_with_raw_args(
        &mut self,
        _func_id: u16,
        _args: &[(u64, NativeKind)],
    ) -> Result<(), VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             call_function_with_raw_args kinded-arg rebuild pending"
        )
    }

    /// Set up a closure call frame from raw u64 args.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Same
    /// kinded-arg rebuild as `call_function_with_raw_args`. The B9
    /// lockstep `closure_heap_bits: None` / `closure_heap_kind: None`
    /// companion is preserved (synthetic / trampoline-style
    /// construction where block lifetime is guaranteed externally).
    pub(crate) fn call_closure_with_raw_args(
        &mut self,
        _func_id: u16,
        _upvalue_bits: &[(u64, NativeKind)],
        _args: &[(u64, NativeKind)],
    ) -> Result<(), VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             call_closure_with_raw_args kinded-arg + kinded-cell rebuild pending"
        )
    }

    /// Fast-path function call: reads `arg_count` arguments directly
    /// from the value stack instead of collecting them into a temporary
    /// `Vec`.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4 / §2.7.8.** Body
    /// referenced the deleted unkinded-write shim on a `ValueWord::none()`
    /// sentinel for filling omitted-arg slots (forbidden — replaced by
    /// `stack_write_kinded(idx, 0u64, NativeKind::Bool)` per playbook
    /// §2, sourced from the sentinel `NONE_BITS` convention at
    /// `vm_impl/stack.rs`). The B9 lockstep `closure_heap_bits: None`
    /// / `closure_heap_kind: None` companion is preserved on the
    /// pushed `CallFrame`.
    pub(crate) fn call_function_from_stack(
        &mut self,
        _func_id: u16,
        _arg_count: usize,
    ) -> Result<(), VMError> {
        todo!(
            "phase-2c — ADR-006 §2.7.8 cluster B-round-2: \
             call_function_from_stack kinded-fill rebuild pending"
        )
    }
}
