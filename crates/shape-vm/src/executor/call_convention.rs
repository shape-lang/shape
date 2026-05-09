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
    #[allow(dead_code)]
    fn resolve_spawned_task(&mut self, task_id: u64) -> Result<KindedSlot, VMError> {
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
            // Match is exhaustive: Closure, UInt64, all-others-error.
            // No polymorphic fall-through that fabricates kinds (W7
            // playbook §6 #6 forbidden). Per §8 surface-and-stop:
            // trait-object closure dispatch (`Ptr(HeapKind::TypedObject)`
            // carrying a `dyn Trait` vtable) is W9 TR territory and
            // routes through this RuntimeError until that wave lands.
            other => Err(VMError::RuntimeError(format!(
                "call_value_immediate_nb: callee must be \
                 NativeKind::Ptr(HeapKind::Closure) or NativeKind::UInt64, \
                 got {:?}",
                other
            ))),
        }
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
