//! Function and closure call convention, execution wrappers, and async resolution.
//!
//! # Wave 7 — value-call ABI rebuild (foundation sub-cluster: W7-frame-setup)
//!
//! ADR-006 §2.7.11 / Q12 lifts the parallel-kind invariant of §2.7.7 (stack)
//! and §2.7.8 (cells) across the call-frame boundary: every dispatch
//! entry-point in this module carries kinds on `KindedSlot` carriers
//! (callee + args + return). The W7 playbook
//! (`docs/cluster-audits/wave-7-cc1-playbook.md`) carves the migration
//! into 6 sub-clusters — see playbook §3 / §5 for the ordering.
//!
//! W7-frame-setup (this sub-cluster, Round 1) owns the three internal
//! frame-setup helpers:
//!
//! 1. [`call_function_with_nb_args`] — non-closure frame setup from a
//!    `&[KindedSlot]` arg slice. Each arg flows into the new frame's
//!    locals via `stack_write_kinded` per playbook 6.5 §3 (caller owns
//!    shares; the dispatch shell `mem::forget`s its arg vec after this
//!    function returns to transfer the share).
//! 2. [`call_closure_with_nb_args_keepalive`] — closure frame setup,
//!    threading capture kinds via `OwnedClosureBlock::read_capture_kinded`
//!    (§2.7.8 / Q10) and the B9 lockstep companion fields
//!    `closure_heap_bits` + `closure_heap_kind` on `CallFrame`. The
//!    pre-§2.7.8 `_upvalue_bits: Vec<u64>` parameter is replaced by
//!    `closure_block: &OwnedClosureBlock` — capture data flows from the
//!    cell-storage parallel-kind track, not from a side-channel
//!    raw-bits payload.
//! 3. [`call_function_from_stack`] — fast-path frame setup where the
//!    args are already on the value stack from the producing
//!    `Push…`/`LoadLocal…` opcodes. Pops `arg_count` slots via
//!    `pop_kinded` and writes each into its new local slot via
//!    `stack_write_kinded`. Sentinel-fills omitted-arg locals with
//!    `(0u64, NativeKind::Bool)` per playbook 6.5 §2 Null/Unit row.
//!
//! The remaining entry-points in this module — `execute_function_by_name`
//! / `_by_id` / `execute_closure` / `execute_function_fast` /
//! `execute_function_with_named_args` / `resume` / `execute_with_async` /
//! `resolve_spawned_task` / `call_value_immediate_nb` /
//! `jit_trampoline_call_closure` — stay `todo!()` until their respective
//! sub-clusters (W7-cv-static, W7-cv-async, W7-cv-method, W7-op-call-value)
//! land in Rounds 2 / 3.
//!
//! # `_raw` pair-slice family — deleted (W7-cv-polymorphic, Round 3)
//!
//! `call_value_immediate_raw`, `call_function_with_raw_args`, and
//! `call_closure_with_raw_args` carried the `&[(u64, NativeKind)]`
//! pair-slice form pre-§2.7.11. ADR-006 §2.7.11 migration-scope
//! refinement (post-W7 audit, 2026-05-09) rejected this shape on §2.7.6
//! / Q8 carrier-API-bound grounds at the runtime tier, and
//! W7-cv-polymorphic (Round 3) deleted all three entry-points — their
//! callers route through `call_value_immediate_nb` /
//! `call_function_with_nb_args` / `call_closure_with_nb_args_keepalive`
//! over `&[KindedSlot]` instead. `jit_trampoline_call_closure` is the
//! only `_raw` survivor — it is the §2.7.5 cross-crate stable FFI
//! consumer where the parallel-pair shape is canonical (consumers
//! translate `&[KindedSlot]` → raw u64 at the FFI boundary, single
//! direction).
//!
//! # Forbidden patterns (W7 playbook §6 — refused on sight)
//!
//! - `Vec<KindedSlot>` by-move parameter (#12 — caller owns shares;
//!   by-move desynchronizes drop accounting). Borrow-only `&[..]`.
//! - `&[(u64, NativeKind)]` pair-slice as a runtime-tier dispatch ABI
//!   (#13 — §2.7.6 / Q8 carrier-API-bound; pair-slice rejected at
//!   runtime tier, allowed only at the §2.7.5 stable-FFI boundary —
//!   `jit_trampoline_call_closure` is the sole survivor).
//! - Bool-default fallback for unresolved-kind capture at frame setup
//!   (#16 — §2.7.8 #4; correct response is surface-and-stop, panic
//!   from `read_capture_kinded` is diagnostic, not fallback).
//! - Re-introducing `_upvalue_bits: Vec<u64>` parameter — the deleted
//!   pre-§2.7.8 ABI shape; replacement is `&OwnedClosureBlock`.
//! - Renaming the deleted kind-blind value-call ABI by hypothetical
//!   role per CLAUDE.md "Renames to refuse on sight" (#18) — describe
//!   deleted code by name (the pre-§2.7.11 raw-u64 entry-points) or
//!   by deletion-fate (the kind-blind value-call ABI), never via the
//!   bridge/probe/helper/hop/translator/adapter/shim framing the
//!   2026-05-09 broadening enumerates.
//!
//! The B9 lockstep invariant
//! (`closure_heap_bits.is_some() == closure_heap_kind.is_some()`) is
//! enforced via `debug_assert_eq!` at every frame-construction site.

use shape_value::v2::closure_raw::{OwnedClosureBlock, typed_closure_function_id};
use shape_value::{HeapKind, HeapValue, KindedSlot, NativeKind, ValueSlot, VMError};

use super::task_scheduler::TaskStatus;
use super::vm_impl::stack::clone_with_kind;

use super::{CallFrame, VirtualMachine};

impl VirtualMachine {
    /// Execute a named function with arguments, returning its result.
    ///
    /// **W7-cv-method (Round 3 close).** Resolves `name` to `func_id` via
    /// the program function table and routes to
    /// [`execute_function_by_id`] per W7 playbook §4.
    pub fn execute_function_by_name(
        &mut self,
        name: &str,
        args: Vec<KindedSlot>,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        let func_id = self
            .program
            .functions
            .iter()
            .position(|f| f.name == name)
            .ok_or_else(|| VMError::RuntimeError(format!("Function '{}' not found", name)))?
            as u16;
        self.execute_function_by_id(func_id, args, ctx)
    }

    /// Execute a function by its ID with positional arguments.
    ///
    /// **W7-cv-method (Round 3 close).** Captures `saved_depth` before
    /// frame setup, routes through [`call_function_with_nb_args`], drives
    /// the callee to completion via
    /// [`execute_until_call_depth`](Self::execute_until_call_depth), and
    /// pops the result via the kinded API (W7 playbook §4 + §2.7.10 / Q11
    /// dispatch shape).
    ///
    /// **Ownership.** Each `KindedSlot` in `args` holds a strong-count
    /// share. `call_function_with_nb_args` transfers shares into local
    /// slots via `stack_write_kinded`; we then `mem::forget` the source
    /// vec to release the per-slot carriers without dropping the shares
    /// — same pattern as the §2.7.10 `op_call_method` dispatch shell.
    pub fn execute_function_by_id(
        &mut self,
        func_id: u16,
        args: Vec<KindedSlot>,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        let saved_call_depth = self.call_stack.len();
        self.call_function_with_nb_args(func_id, &args)?;
        // Shares transferred into the new frame's locals; release the
        // source-side carriers without dropping the shares.
        std::mem::forget(args);
        self.execute_until_call_depth(saved_call_depth, ctx)?;
        let (bits, kind) = self.pop_kinded()?;
        Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind))
    }

    /// Execute a closure with its captured upvalues and arguments.
    ///
    /// **W7-cv-method (Round 3 close).** The pre-§2.7.8 `_upvalue_bits:
    /// Vec<u64>` parameter — the deleted-ABI raw-bits shape — is replaced
    /// by `closure_block: &OwnedClosureBlock`. Captures flow from the
    /// block's parallel-kind track via `read_capture_kinded` inside
    /// [`call_closure_with_nb_args_keepalive`], not from a side-channel
    /// payload (W7 playbook §4 + ADR-006 §2.7.8 / Q10).
    ///
    /// The keep-alive companion fields carry the closure-self share so
    /// `op_return` / `op_return_value` release it via `drop_with_kind`
    /// on frame teardown — same B9 lockstep pattern as
    /// `call_value_immediate_nb`'s closure arm.
    pub fn execute_closure(
        &mut self,
        closure_block: &OwnedClosureBlock,
        args: Vec<KindedSlot>,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        // SAFETY: `closure_block` is a live borrow into a TypedClosureHeader
        // block allocated by `alloc_typed_closure`; its `as_ptr()` points
        // to a valid header per the construction invariant.
        let function_id = unsafe { typed_closure_function_id(closure_block.as_ptr()) };

        let saved_call_depth = self.call_stack.len();
        // No keep-alive carrier — the synthetic dispatch path: the block
        // lifetime is guaranteed by the borrow held across this call,
        // so `closure_heap_bits` / `closure_heap_kind` are both `None`
        // (B9 lockstep `Some(..)` ↔ `Some(..)`).
        self.call_closure_with_nb_args_keepalive(function_id, closure_block, &args, None, None)?;
        std::mem::forget(args);
        self.execute_until_call_depth(saved_call_depth, ctx)?;
        let (bits, kind) = self.pop_kinded()?;
        Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind))
    }

    /// Fast function execution for hot loops (backtesting).
    ///
    /// **W7-cv-method (Round 3 close).** Pre-computed `func_id`, no name
    /// lookup, no args (callers that need args route through
    /// `execute_function_by_id`). Same `saved_depth` pattern as the
    /// other public entry-points.
    pub fn execute_function_fast(
        &mut self,
        func_id: u16,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        let saved_call_depth = self.call_stack.len();
        self.call_function_with_nb_args(func_id, &[])?;
        self.execute_until_call_depth(saved_call_depth, ctx)?;
        let (bits, kind) = self.pop_kinded()?;
        Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind))
    }

    /// Execute a function with named arguments.
    ///
    /// **W7-cv-method (Round 3 close).** Maps `&[(String, KindedSlot)]`
    /// to a positional `Vec<KindedSlot>` via `descriptor.param_names`
    /// lookup, then routes through [`execute_function_by_id`] per W7
    /// playbook §4. Missing positional slots are sentinel-filled with
    /// `(NONE_BITS, NativeKind::Bool)` per W6.5 §2 Null/Unit row —
    /// Drop/Clone-no-op so the pre-population is leak-free.
    ///
    /// **Ownership.** The caller's named-args carry one share per slot;
    /// `clone_with_kind` is NOT used (we re-home the slot's bits by
    /// reading the slot directly). After mapping, the positional vec
    /// owns the same shares; they transfer into the new frame via the
    /// `execute_function_by_id` `mem::forget` discipline.
    pub fn execute_function_with_named_args(
        &mut self,
        func_id: u16,
        named_args: &[(String, KindedSlot)],
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        let (arity, param_names) = {
            let function = self
                .program
                .functions
                .get(func_id as usize)
                .ok_or(VMError::InvalidCall)?;
            (function.arity as usize, function.param_names.clone())
        };

        // Sentinel-fill positional slots with Null/Unit row (W6.5 §2):
        // NONE_BITS + NativeKind::Bool is Drop/Clone-no-op, so the
        // pre-fill is leak-free until the named-arg loop overwrites
        // each present slot below.
        let mut args: Vec<KindedSlot> = (0..arity)
            .map(|_| KindedSlot::new(ValueSlot::none(), NativeKind::Bool))
            .collect();

        for (name, value) in named_args {
            if let Some(idx) = param_names.iter().position(|p| p == name) {
                if idx < args.len() {
                    // The caller owns the named-args slice's shares (they
                    // pass `&[(String, KindedSlot)]` by borrow). Bump the
                    // refcount once via `clone_with_kind` so the
                    // positional vec owns an independent share that
                    // transfers cleanly into the new frame; the caller's
                    // outer slot stays live for them to drop. Sentinel
                    // pair released by `KindedSlot::Drop` on the
                    // `args[idx] = ...` write below — Drop-no-op for
                    // (NONE_BITS, Bool).
                    super::vm_impl::stack::clone_with_kind(value.slot.raw(), value.kind);
                    args[idx] = KindedSlot::new(
                        ValueSlot::from_raw(value.slot.raw()),
                        value.kind,
                    );
                }
            }
        }

        self.execute_function_by_id(func_id, args, ctx)
    }

    /// Resume execution after a suspension.
    ///
    /// **§2.7.4 Phase-2c — stays `todo!()`.** The suspension shape
    /// requires snapshot-tier work: the resume body's pre-§2.7.7 form
    /// pushed `value` onto the stack and re-entered the suspendable
    /// dispatch loop, but the snapshot/restore family
    /// (`apply_pending_resume` / `apply_pending_frame_resume` in
    /// `executor/resume.rs`) is itself §2.7.4 deferred — its bodies
    /// return `VMError::NotImplemented(PHASE_2C_SNAPSHOT_SURFACE)`. Until
    /// the snapshot rebuild lands a kind-threaded
    /// `slot_to_serializable` / `serializable_to_slot` pair plus the
    /// §2.7.8 cell-storage parallel-kind tracks for `module_bindings`
    /// and frame-resume payloads, this entry-point cannot be wired —
    /// surface-and-stop trigger per W7 playbook §8 (snapshot-tier
    /// resume).
    pub fn resume(
        &mut self,
        _value: KindedSlot,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<super::ExecutionResult, VMError> {
        todo!(
            "phase-2c — see ADR-006 §2.7.4 / §2.7.11 out-of-scope: \
             resume() depends on snapshot-tier rebuild (executor/resume.rs \
             apply_pending_resume / apply_pending_frame_resume)"
        )
    }

    /// Execute with automatic async task resolution.
    ///
    /// **Filled by W7-cv-async (Round 3 close).** Per W7 playbook §4
    /// W7-cv-async row, sync-resolution only — suspension state crossing
    /// a `call_value_immediate_*` boundary is OUT OF SCOPE per ADR-006
    /// §2.7.11 out-of-scope clause (Phase-2c snapshot tier; same
    /// out-of-scope clause as §2.7.10).
    ///
    /// Drives the program forward via `execute_fast(ctx)` (the standard
    /// run-to-halt loop that pops the top-of-stack result on completion).
    /// Inline task resolution at `op_await` / `op_join_await` sites in
    /// `executor/async_ops/mod.rs` is the integration point with
    /// [`resolve_spawned_task`] (below) — once the §2.7.4 task-scheduler
    /// kinded-ABI re-light closes those `todo!()` arms, the await-site
    /// handler invokes `resolve_spawned_task(task_id)` directly inside
    /// the dispatch loop, and this driver re-enters `execute_fast` to
    /// continue the program after the suspended `op_await` opcode
    /// returns.
    ///
    /// The pre-bulldozer `execute_with_async` shape — drive a `loop`
    /// over `task_scheduler.iter_pending()` calling `resolve_spawned_task`
    /// per ready task — depends on a public iterator over
    /// `TaskScheduler.callables` that does not exist in the current
    /// scheduler API surface (W7-cv-async owns only `call_convention.rs`
    /// per W7 playbook §10 forbidden zones — `task_scheduler.rs` is
    /// out-of-territory). When a future cluster lands the iteration
    /// API, the loop body in this function is the natural extension
    /// point: while there is a `Pending`-with-callable task, call
    /// `resolve_spawned_task(id)` and discard the per-task result; the
    /// program-level result still comes from `execute_fast` at the end.
    pub fn execute_with_async(
        &mut self,
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        // Sync-resolution only. The §2.7.11 out-of-scope clause is
        // explicit: suspension state crossing a `call_value_immediate_*`
        // boundary is Phase-2c snapshot-tier work and stays outside
        // Wave 7. The bytecode loop drives the program; per-await-site
        // inline resolution is the `resolve_spawned_task` integration
        // point handed to the async_ops dispatch arms when the §2.7.4
        // scheduler kinded-ABI re-light lands.
        self.execute_fast(ctx)
    }

    /// Resolve a spawned task by executing its callable synchronously.
    ///
    /// **Filled by W7-cv-async (Round 3 close).** Per W7 playbook §4
    /// W7-cv-async row body shape: look up the task's callable from the
    /// scheduler, route through `call_closure_with_nb_args_keepalive`
    /// for closure callables (or `call_function_with_nb_args` for raw
    /// function-id callables), drive the callee to completion via
    /// `execute_until_call_depth(saved_depth, None)`, pop the result
    /// via `pop_kinded`, cache it, and return.
    ///
    /// The scheduler stores callables as `(u64, NativeKind)` pairs per
    /// the §2.7.7 carrier shape (Wave 6.5 R-async-time / E-async close
    /// already migrated `task_scheduler.rs` off `ValueWord`). The two
    /// expected callable kinds match the `call_value_immediate_nb`
    /// dispatch shape from W7-cv-static (Round 2 close):
    ///
    /// - `Ptr(HeapKind::Closure)` — recover `OwnedClosureBlock` via
    ///   `slot.as_heap_value()` + `HeapValue::ClosureRaw(block)` per
    ///   ADR-005 §1 single-discriminator. Function-id reads from the
    ///   `TypedClosureHeader` via the unsafe `typed_closure_function_id`
    ///   helper (the canonical accessor; `OwnedClosureBlock` has no
    ///   safe public accessor for `function_id`). Frame setup carries
    ///   the closure-self share through `closure_heap_bits` /
    ///   `closure_heap_kind` per the B9 lockstep companion fields, so
    ///   `op_return` releases it via `drop_with_kind` at frame teardown.
    ///   Spawned tasks have no caller-supplied args — the closure runs
    ///   with `&[]` for the arg slice.
    /// - `UInt64` — function-id callable. The bits encode the function
    ///   id as a raw `u64` payload (`UInt64` is the §2.7.11 callee-
    ///   classification kind for function references; same convention
    ///   as `call_value_immediate_nb`'s `UInt64` arm). No Arc share
    ///   to drop; `drop_with_kind(_, NativeKind::UInt64)` is a no-op.
    ///   Routes through `call_function_with_nb_args(func_id, &[])` —
    ///   the non-closure entry-point in this module's frame-setup
    ///   family (W7-frame-setup, Round 1 close).
    ///
    /// **Cached fast-path.** If `task_scheduler.get_result(task_id)`
    /// returns `TaskStatus::Completed((bits, kind))`, the cached share
    /// is cloned via `clone_with_kind` and returned directly — same
    /// pattern as `TaskScheduler::resolve_task` (cached entry retains
    /// its share; caller gets a fresh share). Cancelled tasks surface
    /// as `RuntimeError`.
    ///
    /// **Suspension out of scope.** If the callee's body suspends mid-
    /// execution (an `op_await` / `op_suspend` inside the spawned
    /// closure), the suspension shape crossing this `call_value_immediate_*`
    /// frame boundary is §2.7.4 Phase-2c snapshot-tier (W7 playbook
    /// §9 risk row, ADR-006 §2.7.11 out-of-scope clause). The current
    /// body drives `execute_until_call_depth` to a definite return; a
    /// `VMError::Suspended` mid-call is propagated upward and the
    /// task's cached entry remains `Pending` until a future Phase-2c
    /// rebuild lands the snapshot-tier resumption.
    pub(in crate::executor) fn resolve_spawned_task(
        &mut self,
        task_id: u64,
    ) -> Result<KindedSlot, VMError> {
        // Cached fast-path — the scheduler already holds a Completed
        // share for this task. Hand out a fresh share via
        // `clone_with_kind` so the cached entry retains its own.
        match self.task_scheduler.get_result(task_id) {
            Some(TaskStatus::Completed((bits, kind))) => {
                let bits = *bits;
                let kind = *kind;
                clone_with_kind(bits, kind);
                return Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind));
            }
            Some(TaskStatus::Cancelled) => {
                return Err(VMError::RuntimeError(format!(
                    "Task {} was cancelled",
                    task_id
                )));
            }
            // Pending or unknown — fall through to the take-callable path.
            Some(TaskStatus::Pending) | None => {}
        }

        // Take ownership of the callable share — `take_callable`
        // transfers the strong-count from the scheduler map to us.
        let (callable_bits, callable_kind) =
            self.task_scheduler.take_callable(task_id).ok_or_else(|| {
                VMError::RuntimeError(format!("No callable registered for task {}", task_id))
            })?;

        // Capture call-stack depth BEFORE frame setup pushes a new
        // frame. The callee's `op_return` / `op_return_value` pops its
        // frame, returning the call-stack depth to this saved value;
        // `execute_until_call_depth(saved_depth, None)` is the canonical
        // "drive callee to completion" loop. Same pattern as
        // `call_value_immediate_nb` (W7-cv-static, Round 2 close).
        let saved_call_depth = self.call_stack.len();

        match callable_kind {
            NativeKind::Ptr(HeapKind::Closure) => {
                // Recover `OwnedClosureBlock` via the §2.7.6 / Q8 heap
                // dispatch path: construct a `ValueSlot` from the raw
                // bits, call `as_heap_value()`, pattern-match the
                // `HeapValue::ClosureRaw(block)` arm per ADR-005 §1
                // single-discriminator. The pattern mirrors
                // `call_value_immediate_nb`'s closure arm verbatim —
                // diverging would re-introduce a forbidden parallel
                // dispatch surface.
                let callable_slot = ValueSlot::from_raw(callable_bits);
                let block: &OwnedClosureBlock = match callable_slot.as_heap_value() {
                    HeapValue::ClosureRaw(b) => b,
                    other => {
                        // Drop the callable share before surfacing the
                        // error so refcount discipline holds (playbook
                        // §3 drop discipline). `drop_with_kind` on
                        // `Ptr(HeapKind::Closure)` releases the
                        // `Arc<HeapValue>` share per W7-closure-retain
                        // (Round 2.5 close).
                        let type_name = other.type_name();
                        super::vm_impl::stack::drop_with_kind(callable_bits, callable_kind);
                        debug_assert!(
                            false,
                            "resolve_spawned_task: HeapKind::Closure label with \
                             non-ClosureRaw HeapValue payload: {:?}",
                            type_name
                        );
                        return Err(VMError::RuntimeError(format!(
                            "resolve_spawned_task: HeapKind::Closure label with \
                             non-ClosureRaw payload: {}",
                            type_name
                        )));
                    }
                };
                // SAFETY: `block` is a live `OwnedClosureBlock` borrowed
                // through the live `&HeapValue` returned by
                // `as_heap_value()`; its `as_ptr()` points to a
                // `TypedClosureHeader` block allocated by
                // `alloc_typed_closure` per the construction invariant.
                let function_id = unsafe { typed_closure_function_id(block.as_ptr()) };

                // Frame setup. The B9 lockstep companion fields carry
                // the closure-self share so `op_return` /
                // `op_return_value` can release it via
                // `drop_with_kind(bits, kind)` on frame teardown — the
                // share transfers from `take_callable` into the
                // `CallFrame.closure_heap_bits` field. Spawned tasks
                // have no caller args, so the arg slice is empty.
                self.call_closure_with_nb_args_keepalive(
                    function_id,
                    block,
                    &[],
                    Some(callable_bits),
                    Some(callable_kind),
                )?;
            }
            NativeKind::UInt64 => {
                // Function-id callable: bits encode the function id as
                // a raw `u64` payload. `UInt64` is the §2.7.11 callee-
                // classification kind for function references — same
                // convention as `call_value_immediate_nb`'s `UInt64`
                // arm (W7-cv-static, Round 2 close). No Arc share to
                // drop; `drop_with_kind(_, NativeKind::UInt64)` is a
                // no-op.
                //
                // Truncate to `u16` since `BytecodeProgram::functions`
                // is indexed by `u16` and `call_function_with_nb_args`
                // takes `func_id: u16`. A bits value that doesn't index
                // into the function table surfaces as
                // `VMError::InvalidCall` from `call_function_with_nb_args`
                // itself per its existing
                // `program.functions.get(func_id as usize).ok_or(VMError::InvalidCall)?`
                // guard.
                let function_id = callable_bits as u16;
                self.call_function_with_nb_args(function_id, &[])?;
            }
            other => {
                // Unsupported callable kind — release the share before
                // surfacing (playbook §3). The kind classification list
                // for spawned-task callables matches
                // `call_value_immediate_nb` (W7-cv-static): closure or
                // function-id; trait-object closure dispatch (W9 TR
                // territory) routes through this RuntimeError until
                // that wave lands.
                super::vm_impl::stack::drop_with_kind(callable_bits, callable_kind);
                return Err(VMError::RuntimeError(format!(
                    "resolve_spawned_task: callable must be \
                     NativeKind::Ptr(HeapKind::Closure) or \
                     NativeKind::UInt64, got {:?}",
                    other
                )));
            }
        }

        // Drive the callee to completion. `execute_until_call_depth`
        // returns when `self.call_stack.len() == saved_call_depth`
        // (the callee's frame has been popped by `op_return`). The
        // return value is left on the value stack by `op_return_value`;
        // `pop_kinded` transfers the share cleanly into the result
        // `KindedSlot`.
        //
        // §2.7.4 Phase-2c — suspension state crossing this frame
        // boundary stays out of scope per ADR-006 §2.7.11 out-of-scope
        // clause. A `VMError::Suspended` propagates upward; the
        // task's cached entry remains `Pending`.
        self.execute_until_call_depth(saved_call_depth, None)?;
        let (result_bits, result_kind) = self.pop_kinded()?;

        // Cache the result — clone the share so the scheduler entry
        // and the returned `KindedSlot` each own one independent
        // strong-count. Same pattern as `TaskScheduler::resolve_task`
        // and `try_resolve_external` cached-completion paths.
        clone_with_kind(result_bits, result_kind);
        self.task_scheduler.complete(task_id, result_bits, result_kind);
        Ok(KindedSlot::new(ValueSlot::from_raw(result_bits), result_kind))
    }

    /// Non-closure frame setup from a `&[KindedSlot]` arg slice
    /// (ADR-006 §2.7.10 / Q11 caller-side carrier; W7 playbook §4).
    ///
    /// Pushes a fresh `CallFrame` for `func_id` and threads each arg's
    /// `(bits, kind)` into the new frame's locals via
    /// `stack_write_kinded`. The B9 lockstep companion fields
    /// `closure_heap_bits` / `closure_heap_kind` are both `None` —
    /// non-closure calls own no closure-self share.
    ///
    /// **Ownership.** The caller (the `op_call_value` dispatch shell or
    /// a public entry-point such as `execute_function_by_id`) owns one
    /// strong-count share per arg slot. This function transfers each
    /// share into the new frame's local slot via `stack_write_kinded`
    /// (which drops the prior occupant — a sentinel after the
    /// `resize_with` below — and installs the new bits). The dispatch
    /// shell calls `mem::forget` on its arg vec after this function
    /// returns to release the source-side carriers without dropping
    /// the shares. Same pattern as the §2.7.10 `op_call_method`
    /// dispatch shell.
    pub(crate) fn call_function_with_nb_args(
        &mut self,
        func_id: u16,
        args: &[KindedSlot],
    ) -> Result<(), VMError> {
        let (locals_count, entry_point) = {
            let func = self
                .program
                .functions
                .get(func_id as usize)
                .ok_or(VMError::InvalidCall)?;
            (func.locals_count as usize, func.entry_point)
        };
        let blob_hash = self.blob_hash_for_function(func_id);

        let base_pointer = self.sp;
        let needed = base_pointer + locals_count;
        if needed > self.stack.len() {
            // ADR-006 §2.7.7 / §2.7.8 lockstep growth: data + parallel
            // kind track grow together. Sentinel pair `(NONE_BITS,
            // NativeKind::Bool)` is Drop/Clone-no-op so the freshly
            // resized window is leak-free until each slot is written
            // by the arg-thread loop / left as the omitted-arg
            // sentinel (W6.5 §2 Null/Unit row).
            self.stack.resize_with(needed * 2 + 1, || Self::NONE_BITS);
            self.kinds.resize(needed * 2 + 1, NativeKind::Bool);
        }

        let return_ip = self.ip;
        self.call_stack.push(CallFrame {
            return_ip,
            base_pointer,
            locals_count,
            function_id: Some(func_id),
            upvalues: None,
            blob_hash,
            closure_heap_bits: None,
            // ADR-006 §2.7.8 / Q10: lockstep companion to
            // `closure_heap_bits`. Non-closure call → both `None`.
            closure_heap_kind: None,
        });

        // Walk args and thread each into the new frame's local at
        // `base_pointer + i`. Per W7 playbook §4 / W6.5 §3, the
        // `stack_write_kinded` write transfers the share into the
        // local slot (drops the sentinel from the resize above —
        // a no-op).
        for (i, slot) in args.iter().enumerate() {
            self.stack_write_kinded(base_pointer + i, slot.slot.raw(), slot.kind);
        }

        self.sp = base_pointer + locals_count;
        self.ip = entry_point;
        Ok(())
    }

    /// Closure frame setup with no closure-self keep-alive (synthetic
    /// dispatch where the block lifetime is guaranteed externally — e.g.
    /// `execute_closure` from the public VM entry-point family). Thin
    /// forwarder over [`call_closure_with_nb_args_keepalive`] with
    /// `(None, None)` for the B9 lockstep companion fields.
    ///
    /// The `_upvalue_bits: Vec<u64>` parameter is the deleted pre-§2.7.8
    /// ABI shape — the kinded replacement takes a borrowed
    /// `OwnedClosureBlock` per ADR-006 §2.7.8 / Q10 (the cell-storage
    /// parallel-kind track is the canonical capture-kind source).
    pub(crate) fn call_closure_with_nb_args(
        &mut self,
        func_id: u16,
        closure_block: &OwnedClosureBlock,
        args: &[KindedSlot],
    ) -> Result<(), VMError> {
        self.call_closure_with_nb_args_keepalive(func_id, closure_block, args, None, None)
    }

    /// Closure frame setup from a borrowed `OwnedClosureBlock` plus an
    /// `&[KindedSlot]` arg slice (ADR-006 §2.7.8 / Q10 cell-storage
    /// parallel-kind invariant; §2.7.10 / Q11 dispatch-slice carrier;
    /// §2.7.11 / Q12 value-call ABI; W7 playbook §4).
    ///
    /// Captures flow via `OwnedClosureBlock::read_capture_kinded(idx)`
    /// — the kind comes directly from the closure layout's
    /// `capture_native_kinds` track, threaded into the new frame's
    /// reserved capture-locals via `stack_write_kinded`. Args follow
    /// the captures, occupying `[base_pointer + capture_count ..
    /// base_pointer + capture_count + args.len()]`.
    ///
    /// The B9 lockstep companion fields `closure_heap_bits` /
    /// `closure_heap_kind` carry the closure-self share (`Some` for
    /// closure dispatch through `op_call_value` / `op_call_closure`;
    /// `None` for synthetic / trampoline-style construction where the
    /// block lifetime is guaranteed externally). The
    /// `debug_assert_eq!` below enforces both fields are `Some`
    /// together or `None` together at every observable boundary.
    ///
    /// **Ownership.** The caller owns one strong-count share per arg
    /// slot and one share for `closure_heap_bits` (when `Some`); both
    /// transfer into the new frame via `stack_write_kinded` and the
    /// `CallFrame.closure_heap_bits` field respectively. Capture reads
    /// via `read_capture_kinded` are raw-bit reads — the shares stay
    /// owned by the `OwnedClosureBlock` (which the caller passes by
    /// borrow); the closure_heap_bits keep-alive ensures the block
    /// outlives the callee's pointer dereferences. Cell-storage
    /// captures (`OwnedMutable` / `Shared`) load through the new
    /// frame's `LoadOwnedClosureSelf` opcode using the kind from
    /// `closure_heap_kind` — see §2.7.8 / Q10 for the cell read flow.
    pub(crate) fn call_closure_with_nb_args_keepalive(
        &mut self,
        func_id: u16,
        closure_block: &OwnedClosureBlock,
        args: &[KindedSlot],
        closure_heap_bits: Option<u64>,
        closure_heap_kind: Option<NativeKind>,
    ) -> Result<(), VMError> {
        debug_assert_eq!(
            closure_heap_bits.is_some(),
            closure_heap_kind.is_some(),
            "ADR-006 §2.7.8 / Q10: closure_heap_bits and closure_heap_kind \
             must be Some together or None together"
        );

        let (locals_count, entry_point) = {
            let func = self
                .program
                .functions
                .get(func_id as usize)
                .ok_or(VMError::InvalidCall)?;
            (func.locals_count as usize, func.entry_point)
        };
        let blob_hash = self.blob_hash_for_function(func_id);

        let layout = closure_block.layout();
        let capture_count = layout.capture_count();

        let base_pointer = self.sp;
        let needed = base_pointer + locals_count;
        if needed > self.stack.len() {
            // ADR-006 §2.7.7 / §2.7.8 lockstep growth — see
            // `call_function_with_nb_args` for the sentinel-pair
            // rationale.
            self.stack.resize_with(needed * 2 + 1, || Self::NONE_BITS);
            self.kinds.resize(needed * 2 + 1, NativeKind::Bool);
        }

        let return_ip = self.ip;
        self.call_stack.push(CallFrame {
            return_ip,
            base_pointer,
            locals_count,
            function_id: Some(func_id),
            upvalues: None,
            blob_hash,
            closure_heap_bits,
            // ADR-006 §2.7.8 / Q10: lockstep companion to
            // `closure_heap_bits`. The `debug_assert_eq!` above
            // guarantees `Some(..)` ↔ `Some(..)`.
            closure_heap_kind,
        });

        // Walk captures from the closure layout's parallel-kind track
        // (ADR-006 §2.7.8 / Q10). `read_capture_kinded(idx)` returns
        // `(bits, kind)` directly — the kind comes from
        // `layout.capture_native_kinds[idx]`, set at closure
        // construction by the producing `MakeClosure` opcode. No
        // fabrication, no Bool-default fallback (§2.7.8 #4 forbidden);
        // a misalignment between layout and stored bits is a
        // construction-side bug that surfaces as a panic from
        // `read_capture_kinded` itself (W7 playbook §8 surface-and-stop).
        for capture_idx in 0..capture_count {
            // SAFETY: the block was constructed by the producing
            // `MakeClosure` opcode with `capture_count` initialised
            // capture slots; the borrow from the dispatch shell holds
            // the block live for the duration of this call.
            let (bits, kind) = unsafe { closure_block.read_capture_kinded(capture_idx) };
            self.stack_write_kinded(base_pointer + capture_idx, bits, kind);
        }

        // Walk args and thread each into the local slot following the
        // captures.
        let arg_base = base_pointer + capture_count;
        for (i, slot) in args.iter().enumerate() {
            self.stack_write_kinded(arg_base + i, slot.slot.raw(), slot.kind);
        }

        self.sp = base_pointer + locals_count;
        self.ip = entry_point;
        Ok(())
    }

    /// `call_value_immediate` (kinded carrier form): dispatches on the
    /// callee's `KindedSlot.kind`. ADR-006 §2.7.11 / Q12 caller-side
    /// shape — both callee and args travel as `KindedSlot`.
    ///
    /// **Filled by W7-cv-static (Round 2 close).** Per W7 playbook §4:
    /// matches on `callee.kind` and routes — `Ptr(HeapKind::Closure)`
    /// recovers the `OwnedClosureBlock` via `slot.as_heap_value()` +
    /// `HeapValue::ClosureRaw` (single discriminator per ADR-005 §1)
    /// and routes to `call_closure_with_nb_args_keepalive`; `UInt64`
    /// callee bits are the function-id and route to
    /// `call_function_with_nb_args`. Both arms drive the callee to
    /// completion via `execute_until_call_depth(saved_depth, ctx)`
    /// (the call-stack-bounded run loop in `dispatch.rs`) and pop the
    /// result from the value stack via `pop_kinded`. Other kinds fall
    /// through to a `RuntimeError` (`VMError::TypeError` is
    /// `&'static str`-bound and incompatible with the format!-style
    /// dynamic-kind error message; the convention used by the existing
    /// `op_call_value` surfaces is `RuntimeError(format!(...))`). The
    /// `HeapValue::HostClosure` variant referenced in pre-Wave-7 docs
    /// has been deleted; only `ClosureRaw` survives in the
    /// closure-dispatch path.
    pub fn call_value_immediate_nb(
        &mut self,
        callee: &KindedSlot,
        args: &[KindedSlot],
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<KindedSlot, VMError> {
        // Capture the call-stack depth BEFORE frame setup pushes a new
        // frame. After the callee's `op_return` / `op_return_value`
        // pops its frame, the call-stack depth returns to this saved
        // value — `execute_until_call_depth(saved_depth, ctx)` is the
        // canonical "drive callee to completion" loop (the playbook's
        // notional `run_until_return` lives here under that name; see
        // `dispatch.rs::execute_until_call_depth`).
        let saved_call_depth = self.call_stack.len();

        match callee.kind {
            NativeKind::Ptr(shape_value::HeapKind::Closure) => {
                // Recover `OwnedClosureBlock` via the §2.7.6 / Q8 heap
                // dispatch path: `slot.as_heap_value()` returns
                // `&HeapValue`, pattern-match the
                // `HeapValue::ClosureRaw(block)` arm per ADR-005 §1
                // single-discriminator. A `HeapKind::Closure` label
                // with any other `HeapValue` payload is a
                // construction-side bug at the producing
                // `op_make_closure`; debug_assert in dev, surface as a
                // RuntimeError in release (the post-§2.7.11 dispatch
                // shell must not silently fabricate a kind — playbook
                // §6 #6 polymorphic-fallthrough forbidden).
                let block: &OwnedClosureBlock = match callee.slot.as_heap_value() {
                    HeapValue::ClosureRaw(block) => block,
                    other => {
                        debug_assert!(
                            false,
                            "call_value_immediate_nb: HeapKind::Closure label with \
                             non-ClosureRaw HeapValue payload: {:?}",
                            other.type_name()
                        );
                        return Err(VMError::RuntimeError(format!(
                            "call_value_immediate_nb: HeapKind::Closure label with \
                             non-ClosureRaw payload: {}",
                            other.type_name()
                        )));
                    }
                };
                // Recover the function-id from the typed closure
                // header. `OwnedClosureBlock` has no safe public
                // accessor for `function_id`; the canonical path is
                // the unsafe `typed_closure_function_id(block.as_ptr())`
                // helper used by the block's own `Debug` impl
                // (`closure_raw.rs:215`). The block's borrow keeps
                // the underlying header live for the duration of this
                // read.
                //
                // SAFETY: `block` is a live `OwnedClosureBlock`
                // (borrowed through the live `&HeapValue` returned by
                // `as_heap_value()`); its `as_ptr()` points to a
                // `TypedClosureHeader` block allocated by
                // `alloc_typed_closure` per the construction
                // invariant.
                let function_id = unsafe { typed_closure_function_id(block.as_ptr()) };

                // Frame setup. The B9 lockstep companion fields carry
                // the closure-self share so `op_return` /
                // `op_return_value` can release it via
                // `drop_with_kind(bits, kind)` on frame teardown.
                // `closure_heap_bits` is the raw slot bits (`Box<HeapValue>`
                // pointer) and `closure_heap_kind` is the matching
                // `NativeKind::Ptr(HeapKind::Closure)`.
                //
                // Round 13 T5 share-accounting fix (W17-vm-call-value-
                // closure-kind-mismatch, audit doc
                // `docs/cluster-audits/w17-vm-call-value-closure-kind-mismatch-audit.md`
                // §4 Option B). The `callee` carrier owns one
                // `Arc<HeapValue>` strong-count share — transferred
                // from the stack via `pop_kinded` in the
                // `dispatch_call_value_immediate` shell
                // (`control_flow/mod.rs:408-409`). The carrier `Drop`
                // releases that share at end of dispatch via
                // `drop_with_kind`. The frame's
                // `closure_heap_bits` companion ALSO releases via
                // `drop_with_kind` at `op_return` / `op_return_value`
                // teardown (`control_flow/mod.rs:712-726` / `:774-788`).
                // Without an explicit `clone_with_kind` here the two
                // releases retire one share more than was acquired —
                // the closure `Arc<HeapValue>` reaches refcount 0
                // before the closure-self binding's
                // `Arc::decrement_strong_count` runs, freeing the
                // header that `op_make_closure`'s producer share at
                // Local 1 still references. On the next iteration
                // `CloneLocal Local(1)` reads the dangling bits and
                // races the allocator — surfacing as
                // `HeapKind::Closure label with non-ClosureRaw payload`
                // in debug or `Invalid function call` in release (the
                // bogus `function_id` read from the freed header fails
                // the `program.functions.get(func_id)` bounds check at
                // `call_closure_with_nb_args_keepalive`).
                //
                // The §2.7.7 / Q9 retain-on-read primitive is the
                // canonical kind-aware refcount bump — no tag decode,
                // no `is_heap()` probe, no Bool-default fallback. Same
                // share-balance pattern as
                // `execute_function_with_named_args` (lines 246-250)
                // which clones each named-arg into the positional vec.
                super::vm_impl::stack::clone_with_kind(callee.slot.raw(), callee.kind);
                self.call_closure_with_nb_args_keepalive(
                    function_id,
                    block,
                    args,
                    Some(callee.slot.raw()),
                    Some(callee.kind),
                )?;

                // Drive the callee to completion. `execute_until_call_depth`
                // returns when `self.call_stack.len() == saved_call_depth`
                // (i.e. the callee's frame has been popped by `op_return`).
                // The return value is left on the value stack by
                // `op_return_value`; pop it via the kinded API so the
                // share transfers cleanly into the result `KindedSlot`.
                self.execute_until_call_depth(saved_call_depth, ctx)?;
                let (bits, kind) = self.pop_kinded()?;
                Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind))
            }
            NativeKind::UInt64 => {
                // Function-id callee: `callee.slot.raw()` is the
                // function-id encoded as raw `u64` bits (§2.7.11 / Q12
                // — `UInt64` is the §2.7.11 callee-classification kind
                // for function references). Truncate to `u16` since
                // `BytecodeProgram::functions` is indexed by `u16` and
                // both Round 1 frame-setup helpers (`call_function_with_nb_args`,
                // `call_closure_with_nb_args_keepalive`) take `func_id: u16`.
                // A bits value that doesn't index into the function
                // table surfaces as `VMError::InvalidCall` from
                // `call_function_with_nb_args` itself (per its
                // existing `program.functions.get(func_id as usize)
                // .ok_or(VMError::InvalidCall)?` guard) — the playbook
                // §8 surface-and-stop trigger ("UInt64 callee bits don't
                // match a real function-id") routes through that path.
                let function_id = callee.slot.raw() as u16;

                self.call_function_with_nb_args(function_id, args)?;

                // Drive callee to completion and pop the result; same
                // pattern as the closure arm above.
                self.execute_until_call_depth(saved_call_depth, ctx)?;
                let (bits, kind) = self.pop_kinded()?;
                Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind))
            }
            // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
            // ModuleFn callee — the slot bits are a `module_fn_id`
            // (cast to u64), indexing the VM's `module_fn_table` per
            // the `populate_module_objects` construction-side contract.
            // Dispatch goes directly through `invoke_module_fn_id_stub`
            // (sync `Typed` or async `TypedAsync` per the §2.7.4
            // task-scheduler boundary in `vm_impl/modules.rs`). The
            // dispatcher converts the `&[KindedSlot]` args at the
            // boundary to the body's `&[u64]` slice + `ModuleContext`,
            // then projects the `TypedReturn` back to a `KindedSlot`
            // via `project_typed_return`. Pure-discriminator inline-
            // scalar dispatch (no Arc bookkeeping on the callee
            // bits — `clone_with_kind` / `drop_with_kind` arms are
            // no-op for `HeapKind::ModuleFn`).
            NativeKind::Ptr(shape_value::HeapKind::ModuleFn) => {
                let module_fn_id = callee.slot.raw() as usize;
                // `invoke_module_fn_id_stub` returns a fresh KindedSlot
                // whose share was minted by `project_typed_return`. We
                // return it directly; the dispatch shell at
                // `dispatch_call_value_immediate` transfers the share
                // into the caller's stack slot via `push_kinded` +
                // `mem::forget` on the result carrier.
                self.invoke_module_fn_id_stub(module_fn_id, args)
            }
            // Match is exhaustive: Closure, UInt64, ModuleFn,
            // all-others-error. No polymorphic fall-through that
            // fabricates kinds (W7 playbook §6 #6 forbidden). Per §8
            // surface-and-stop: trait-object closure dispatch
            // (`Ptr(HeapKind::TypedObject)` carrying a `dyn Trait`
            // vtable) is W9 TR territory and routes through this
            // RuntimeError until that wave lands.
            other => Err(VMError::RuntimeError(format!(
                "call_value_immediate_nb: callee must be \
                 NativeKind::Ptr(HeapKind::Closure), \
                 NativeKind::Ptr(HeapKind::ModuleFn), or NativeKind::UInt64, \
                 got {:?}",
                other
            ))),
        }
    }

    /// Trampoline entry: call a closure by `func_id` with pre-extracted
    /// raw upvalue bits and raw args, returning the result as raw `u64`
    /// bits.
    ///
    /// **W7-cv-method (Round 3 close).** This is the **only** `_raw`
    /// survivor in `call_convention.rs` per ADR-006 §2.7.11
    /// migration-scope refinement: it is the §2.7.5 cross-crate
    /// stable-FFI consumer where the parallel-pair shape (raw `u64`
    /// data + `NativeKind`) is canonical. Consumers translate
    /// `&[KindedSlot]` → raw `u64` at the FFI boundary; this function
    /// is the inverse hop on the runtime side.
    ///
    /// Body wraps `args` as a transient `&[KindedSlot]` slice (no Arc
    /// bump — the JIT pre-incremented each share before crossing the
    /// boundary), constructs a fresh `OwnedClosureBlock` from
    /// `upvalue_bits` per the existing closure-construction convention
    /// (allocate → write each capture's bits at its layout offset →
    /// `OwnedClosureBlock::from_raw`), routes through
    /// [`call_closure_with_nb_args_keepalive`], drives the callee, pops
    /// the result via `pop_kinded`, and returns the bits as raw `u64`
    /// (the kind is discarded — the JIT caller knows the static return
    /// kind from the callee signature).
    ///
    /// **Ownership.** Each `(bits, kind)` in `upvalue_bits` carries a
    /// pre-incremented share. We transfer those shares into the new
    /// closure block via `write_capture_typed` (which stores the bit
    /// pattern without bumping the refcount). The `OwnedClosureBlock`
    /// then owns the captures' shares — its `Drop` walks the layout's
    /// capture masks and releases them. Same for `args`: the JIT
    /// pre-incremented; we hand each transient `KindedSlot` over by
    /// move (no clone), and `call_closure_with_nb_args_keepalive`
    /// transfers the shares into the new frame via `stack_write_kinded`.
    /// We `mem::forget` the transient args vec so its `Drop` does not
    /// double-free.
    pub fn jit_trampoline_call_closure(
        &mut self,
        func_id: u16,
        upvalue_bits: &[(u64, NativeKind)],
        args: &[(u64, NativeKind)],
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<u64, VMError> {
        use shape_value::v2::closure_raw::{alloc_typed_closure, write_capture_raw_u64};
        use std::sync::Arc;

        // Source the closure layout from the program's per-function
        // side-table (`closure_function_layouts[func_id]`). A `None`
        // entry means the function is not a closure — the JIT-side
        // `dispatch_call_via_trampoline_vm` should have routed bare
        // function callees through `call_value_immediate_nb` instead;
        // landing here with `None` is a JIT codegen bug. Surface as a
        // RuntimeError per W7 playbook §8.
        let layout_arc: Arc<shape_value::v2::closure_layout::ClosureLayout> = self
            .program
            .closure_function_layouts
            .get(func_id as usize)
            .and_then(|opt| opt.clone())
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "jit_trampoline_call_closure: no ClosureLayout for func_id {} \
                     (program.closure_function_layouts entry is None — JIT codegen bug)",
                    func_id
                ))
            })?;

        debug_assert_eq!(
            upvalue_bits.len(),
            layout_arc.capture_count(),
            "jit_trampoline_call_closure: upvalue_bits.len() {} != layout.capture_count() {}",
            upvalue_bits.len(),
            layout_arc.capture_count()
        );

        // Allocate a fresh closure block and write each capture's bits
        // at its layout offset. The JIT pre-incremented each heap-typed
        // share before crossing the FFI boundary — `write_capture_raw_u64`
        // stores the bit pattern without bumping the refcount, so the
        // shares transfer cleanly into the new block. The block's Drop
        // (via `release_typed_closure` triggered by `OwnedClosureBlock`)
        // releases each capture's share via the layout's heap/owned/
        // shared capture masks.
        //
        // SAFETY: `alloc_typed_closure` returns a freshly-zeroed block
        // sized for `layout_arc.total_heap_size()`; refcount is 1.
        // `write_capture_raw_u64` writes the 8-byte capture slot at
        // `layout.heap_capture_offset(i)` which is in-bounds for every
        // `i < capture_count()`. The `type_id = 0` placeholder matches
        // what the trampoline-side construction had pre-§2.7.11 (the
        // typed-closure machinery does not key dispatch on `type_id`
        // for trampoline-bound closures).
        let block = unsafe {
            let ptr = alloc_typed_closure(func_id, 0, &layout_arc);
            for (i, (bits, _kind)) in upvalue_bits.iter().enumerate() {
                write_capture_raw_u64(ptr, &layout_arc, i, *bits);
            }
            OwnedClosureBlock::from_raw(ptr, layout_arc)
        };

        // Wrap args as a transient `&[KindedSlot]` slice. No Arc bump:
        // the JIT pre-incremented each share before crossing the FFI
        // boundary; the transient KindedSlots own those shares now,
        // and `call_closure_with_nb_args_keepalive` transfers them into
        // the new frame's locals via `stack_write_kinded`. We
        // `mem::forget` the vec at the end so its `Drop` does not
        // double-free.
        let kinded_args: Vec<KindedSlot> = args
            .iter()
            .map(|(bits, kind)| KindedSlot::new(ValueSlot::from_raw(*bits), *kind))
            .collect();

        let saved_call_depth = self.call_stack.len();
        // No keep-alive carrier: the closure block lives for the
        // duration of this call via the local `block` binding (its
        // `Drop` at end-of-function releases it — but only after
        // `execute_until_call_depth` has returned, by which point the
        // callee's frame has been popped). B9 lockstep: both `None`.
        self.call_closure_with_nb_args_keepalive(func_id, &block, &kinded_args, None, None)?;
        std::mem::forget(kinded_args);

        self.execute_until_call_depth(saved_call_depth, ctx)?;
        let (bits, _kind) = self.pop_kinded()?;
        // Return raw bits. The kind is discarded — the JIT caller
        // knows the static return kind from the callee signature.
        Ok(bits)
    }

    /// Trampoline entry: dispatch a method call on a kinded receiver +
    /// kinded args, returning the result as raw `u64` bits.
    ///
    /// **W12-jit-call-method-shell-rebuild (Phase 3 cluster-0 Round 10 /
    /// 8B.2 close).** Sibling to [`jit_trampoline_call_closure`]; same
    /// §2.7.5 cross-crate stable-FFI consumer shape. The JIT-side
    /// `jit_call_method` shell pops `(bits, kind)` pairs from the JIT's
    /// `ctx.stack` + `ctx.stack_kinds` parallel-kind track per §2.7.7 / Q9,
    /// passes them across the FFI boundary as `&[(u64, NativeKind)]`
    /// pair-slices, and this function converts them to the kinded
    /// carrier form before delegating to
    /// [`dispatch_method_kinded`](Self::dispatch_method_kinded) — the
    /// §2.7.10 / Q11 kinded method-dispatch entry shared with
    /// `op_call_method`.
    ///
    /// **Pair-slice → KindedSlot conversion is single-direction** per
    /// the module-level docstring's "sole `_raw` survivor" rule. The
    /// pair-slice is the canonical §2.7.5 boundary shape; internally
    /// only `&[KindedSlot]` flows. Forbidden alternatives (per ADR-006
    /// §2.7.6 / Q8 + §2.7.10 / Q11):
    /// - parallel `&[NativeKind]` second-slice parameter (carrier-API-
    ///   bound rejection — kind goes on the carrier struct, not a
    ///   side-channel);
    /// - decoding receiver kind from `receiver.0` raw bits via tag-bit
    ///   probe (the deleted §2.7.7 #4 / #7 dispatch);
    /// - Bool-default kinded carrier for unknown receiver kind
    ///   (§2.7.7 #9 — the surface-and-stop discipline forbids this).
    ///
    /// **Ownership.** Each `(bits, kind)` pair carries a pre-incremented
    /// share installed by the JIT producer (per §2.7.7 retain-on-read
    /// semantics on the JIT-side stack). The transient `KindedSlot`
    /// carriers adopt those shares for the call duration. PHF handlers
    /// borrow-only (`&[KindedSlot]` per §2.7.10 / Q11), so the carriers
    /// retain ownership of the JIT-pre-incremented shares throughout
    /// dispatch. When the carriers `Drop` at end of scope, each kind's
    /// `drop_with_kind` releases its share — balancing the JIT-side
    /// retain-before-crossing pattern. The returned `KindedSlot`'s share
    /// is transferred back to the JIT caller as raw u64 bits via
    /// `mem::forget`; the kind is discarded — the JIT caller knows the
    /// static return kind from the callee method signature at the
    /// §2.7.5 stamp-at-compile-time producing site.
    ///
    /// **Lifetime accounting contrast vs. `jit_trampoline_call_closure`.**
    /// The closure trampoline's `mem::forget(kinded_args)` (line 1035)
    /// is because the args were transferred into the callee's frame
    /// locals via `stack_write_kinded` — the shares moved into the
    /// frame, so the transient carriers must NOT release them. Method
    /// dispatch's PHF handlers do not transfer the shares anywhere —
    /// they only borrow — so the transient carriers DO release at end
    /// of scope. Both patterns preserve §2.7.7 retain-on-read +
    /// drop-on-write discipline; the difference is which slot owns the
    /// share at the call's exit boundary.
    pub fn jit_trampoline_call_method(
        &mut self,
        method_name: &str,
        receiver: (u64, NativeKind),
        args: &[(u64, NativeKind)],
        ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<u64, VMError> {
        // Wrap receiver + args as transient `&[KindedSlot]` per the
        // §2.7.10 / Q11 dispatch-slice form (`args[0]` is the receiver,
        // `args[1..]` are the call args). No Arc bump: the JIT
        // pre-incremented each share before crossing the FFI boundary;
        // the transient KindedSlots adopt those shares, dispatch
        // borrow-only, and release on scope exit via `KindedSlot::Drop`.
        let mut kinded_args: Vec<KindedSlot> = Vec::with_capacity(args.len() + 1);
        let (rbits, rkind) = receiver;
        kinded_args.push(KindedSlot::new(ValueSlot::from_raw(rbits), rkind));
        for (bits, kind) in args.iter().copied() {
            kinded_args.push(KindedSlot::new(ValueSlot::from_raw(bits), kind));
        }

        let result = self.dispatch_method_kinded(&kinded_args, method_name, ctx)?;

        // Transfer the result share back to the JIT caller as raw bits.
        // The kind is discarded — the JIT caller knows the static return
        // kind from the callee method signature at the §2.7.5 stamp-at-
        // compile-time producing site.
        let bits = result.slot.raw();
        std::mem::forget(result);

        // `kinded_args` drops here. `KindedSlot::Drop` dispatches on
        // each entry's kind and retires the JIT-pre-incremented share
        // via `drop_with_kind` — no bare `vw_drop`, no Bool-default
        // (forbidden §2.7.7 #9), no decode (forbidden §2.7.7 #4 / #7).
        Ok(bits)
    }

    /// Fast-path frame setup: args are already on the value stack at
    /// `[self.sp - arg_count .. self.sp]` from the producing push
    /// opcodes (e.g. `LoadLocal*`, `PushConst`). The new frame's
    /// `base_pointer` is exactly `self.sp - arg_count`, so those
    /// slots — already carrying the right `(bits, kind)` pairs on the
    /// parallel-kind track — become the new frame's locals 0..arg_count
    /// in place, with no per-slot pop/write copy round-trip.
    ///
    /// Per W7 playbook §4 / W6.5 §3, the share lives once: each arg's
    /// strong-count share was installed into the slot by its producing
    /// opcode and stays in the slot across the frame transition. No
    /// `clone_with_kind`, no `drop_with_kind` — the slot is the share's
    /// home throughout.
    ///
    /// Omitted-arg locals (when `arg_count < locals_count`) are
    /// sentinel-filled with `(NONE_BITS, NativeKind::Bool)` per W6.5
    /// §2 Null/Unit row — Drop/Clone are no-ops on this pair so the
    /// pre-population is leak-free.
    ///
    /// The B9 lockstep companion fields `closure_heap_bits` /
    /// `closure_heap_kind` are both `None` — non-closure call.
    pub(crate) fn call_function_from_stack(
        &mut self,
        func_id: u16,
        arg_count: usize,
    ) -> Result<(), VMError> {
        let func = self
            .program
            .functions
            .get(func_id as usize)
            .ok_or(VMError::InvalidCall)?;
        let locals_count = func.locals_count as usize;
        let blob_hash = self.blob_hash_for_function(func_id);
        let entry_point = func.entry_point;

        if self.sp < arg_count {
            return Err(VMError::StackUnderflow);
        }

        let base_pointer = self.sp - arg_count;
        let needed = base_pointer + locals_count;
        if needed > self.stack.len() {
            // ADR-006 §2.7.7 / §2.7.8 lockstep growth: data + parallel
            // kind track grow together. Sentinel pair is `(NONE_BITS,
            // NativeKind::Bool)` — Drop/Clone are no-ops on this pair
            // so the pre-population window is leak-free.
            self.stack.resize_with(needed * 2 + 1, || Self::NONE_BITS);
            self.kinds.resize(needed * 2 + 1, NativeKind::Bool);
        }

        let return_ip = self.ip;
        self.call_stack.push(CallFrame {
            return_ip,
            base_pointer,
            locals_count,
            function_id: Some(func_id),
            upvalues: None,
            blob_hash,
            closure_heap_bits: None,
            // ADR-006 §2.7.8 / Q10: lockstep with `closure_heap_bits`.
            // Non-closure fast path → both `None`.
            closure_heap_kind: None,
        });

        // Sentinel-fill omitted-arg locals (W6.5 §2 Null/Unit row).
        // Slots `[base_pointer .. base_pointer + arg_count]` already
        // hold the pushed args; slots `[base_pointer + arg_count ..
        // base_pointer + locals_count]` may carry stale shares from
        // a prior frame's teardown. `stack_write_kinded` releases the
        // prior occupant via `drop_with_kind` before installing the
        // sentinel.
        for i in arg_count..locals_count {
            self.stack_write_kinded(base_pointer + i, Self::NONE_BITS, NativeKind::Bool);
        }

        self.sp = base_pointer + locals_count;
        self.ip = entry_point;
        Ok(())
    }
}
