//! Snapshot of live VM state for read-only introspection by module functions.
//!
//! `VmStateSnapshot` captures the call stack, locals, args, and module bindings
//! at a point during execution and implements `VmStateAccessor` so that
//! extension modules (e.g., `std::state`) can inspect the VM without holding a
//! mutable borrow on it.
//!
//! # W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12)
//!
//! Built on top of W17-snapshot-roundtrip's kind-threaded serializer API
//! (`slot_to_serializable` / `serializable_to_slot` at
//! `shape-runtime::snapshot`). The pre-bulldozer implementation collected raw
//! bit patterns from the live VM via the deleted Wave-6.5-substep-1 shims and
//! the deleted hand-rolled retain-on-read discipline. The post-§2.7.7
//! replacement threads `NativeKind` from the parallel kind track at every read
//! site and exposes `KindedSlot` carriers through `FrameInfo`.
//!
//! The snapshot is filled lazily — captured via [`VirtualMachine::capture_vm_state`]
//! before each module-function dispatch by [`super::vm_impl::modules`]. Bodies in
//! `state_builtins/*` receive the snapshot via `ModuleContext.vm_state` and
//! project its `KindedSlot` carriers through `slot_to_serializable` to build
//! their `TypedReturn` payloads.

use shape_runtime::module_exports::{FrameInfo, VmStateAccessor};
use shape_value::{KindedSlot, NativeKind, ValueSlot};

use super::VirtualMachine;

/// Snapshot of VM state captured at a point during execution.
///
/// The snapshot owns kinded copies of the relevant VM state — `KindedSlot`'s
/// `Clone` / `Drop` impls dispatch on `NativeKind` to retire / bump heap
/// refcounts, so by the time the snapshot is constructed every slot owns its
/// own share. The live VM keeps its own shares; teardown is independent.
pub(crate) struct VmStateSnapshot {
    /// All call frames from oldest (bottom) to newest (top).
    /// The "current frame" is the last entry; `caller` is the second-to-last.
    pub(crate) frames: Vec<FrameInfo>,

    /// Captured args for the currently-executing function (or empty if at
    /// top-level / no frames).
    pub(crate) current_args: Vec<KindedSlot>,

    /// Locals for the currently-executing function (name + KindedSlot).
    /// Names come from `Function::param_names` when available; otherwise
    /// they're "local_<idx>".
    pub(crate) current_locals: Vec<(String, KindedSlot)>,

    /// Module bindings (binding name + KindedSlot).
    pub(crate) module_bindings: Vec<(String, KindedSlot)>,

    /// Total instructions executed up to capture point.
    pub(crate) instruction_count: usize,
}

impl VirtualMachine {
    /// Capture a read-only snapshot of the current VM state.
    ///
    /// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** Threads
    /// `NativeKind` from the parallel stack/module-binding kind tracks
    /// (§2.7.7 / §2.7.8) so every captured slot owns a typed Arc share
    /// (where heap-bearing). The snapshot lives in `ModuleContext.vm_state`
    /// for the duration of one module-function dispatch.
    pub(crate) fn capture_vm_state(&self) -> VmStateSnapshot {
        let frames = self.snapshot_frames_for_accessor();
        let (current_args, current_locals) = self.snapshot_current_args_locals();
        let module_bindings = self.snapshot_module_bindings_for_accessor();
        VmStateSnapshot {
            frames,
            current_args,
            current_locals,
            module_bindings,
            instruction_count: self.snapshot_instruction_count(),
        }
    }

    /// Build the FrameInfo vector from the live `call_stack`.
    ///
    /// Per-frame upvalues are recovered from `closure_heap_bits` /
    /// `closure_heap_kind` via `OwnedClosureBlock::read_capture_kinded`
    /// (§2.7.8 / Q10 — captures carry their parallel kind track on the
    /// ClosureLayout side-table, not on the CallFrame raw u64 vec).
    fn snapshot_frames_for_accessor(&self) -> Vec<FrameInfo> {
        let mut out = Vec::with_capacity(self.call_stack.len());
        for frame in self.call_stack.iter() {
            let function_name = frame
                .function_id
                .and_then(|fid| self.program.functions.get(fid as usize))
                .map(|f| f.name.clone())
                .unwrap_or_default();

            // Locals window: stack[base_pointer .. base_pointer+locals_count]
            // with parallel kinds. The kinds track is lockstep with the
            // stack data track per §2.7.7 invariant.
            let base = frame.base_pointer;
            let end = base.saturating_add(frame.locals_count).min(self.stack.len());
            let mut locals: Vec<KindedSlot> = Vec::with_capacity(end - base);
            for i in base..end {
                let bits = self.stack[i];
                let kind = self
                    .kinds
                    .get(i)
                    .copied()
                    .unwrap_or(NativeKind::Bool);
                // Clone-on-read via clone_with_kind discipline: snapshot
                // owns its own share.
                let cloned = clone_slot_kinded(bits, kind);
                locals.push(cloned);
            }

            // Upvalues: dispatch through OwnedClosureBlock when the frame
            // is a closure call.
            let upvalues = self.snapshot_frame_upvalues(frame);

            out.push(FrameInfo {
                function_id: frame.function_id,
                function_name,
                blob_hash: frame.blob_hash.map(|h| h.0),
                local_ip: 0, // Per-frame local IP recovery is the
                             // W17-snapshot-callstack-localip follow-up.
                locals,
                upvalues,
                args: Vec::new(), // The per-frame args are at the lower stack
                                  // window (base_pointer - arity..base_pointer);
                                  // recovery requires the per-call arity which
                                  // is not stored on CallFrame today —
                                  // W17-snapshot-frame-args follow-up.
            });
        }
        out
    }

    /// Recover per-frame upvalues as `Vec<KindedSlot>` from
    /// `closure_heap_bits` / `closure_heap_kind`.
    ///
    /// Returns `None` for non-closure frames (`closure_heap_bits == None`).
    /// For closure frames, walks the `OwnedClosureBlock` via
    /// `read_capture_kinded(idx)` per ADR-006 §2.7.8 / Q10 — the
    /// ClosureLayout side-table carries the parallel kind track.
    fn snapshot_frame_upvalues(&self, frame: &super::CallFrame) -> Option<Vec<KindedSlot>> {
        let bits = frame.closure_heap_bits?;
        let _kind = frame.closure_heap_kind?;
        if bits == 0 {
            return None;
        }
        // SAFETY: per the §2.7.8 / Q10 construction contract, when
        // closure_heap_bits is Some(bits), the bits are
        // `OwnedClosureBlock::into_raw(...)` (an Arc-like share). The
        // OwnedClosureBlock recovery is the same 5-arm receiver-recovery
        // pattern as elsewhere — recover, clone, restore the original
        // share. `read_capture_kinded` then walks the ClosureLayout
        // side-table to source per-capture NativeKind.
        let ptr = bits as *mut u8;
        // The block layout exposes capture-count via the layout Arc which
        // we can't access from here without re-deriving it. The closure
        // block bytes themselves carry their `type_id`; we recover the
        // layout via the executor's closure-layout cache.
        let block = match self.try_borrow_closure_block(ptr) {
            Some(b) => b,
            None => return None,
        };
        let count = block.layout().capture_count();
        let mut out: Vec<KindedSlot> = Vec::with_capacity(count);
        for idx in 0..count {
            // SAFETY: idx < capture_count; the block is borrowed live.
            let (bits, kind) = unsafe { block.read_capture_kinded(idx) };
            let cloned = clone_slot_kinded(bits, kind);
            out.push(cloned);
        }
        Some(out)
    }

    /// Args + locals for the currently-executing (topmost) frame.
    /// For a top-level VM with no frames, returns (empty, empty).
    fn snapshot_current_args_locals(&self) -> (Vec<KindedSlot>, Vec<(String, KindedSlot)>) {
        let Some(top) = self.call_stack.last() else {
            return (Vec::new(), Vec::new());
        };

        // Locals window: same as frames-side recovery.
        let base = top.base_pointer;
        let end = base.saturating_add(top.locals_count).min(self.stack.len());
        let func_opt = top
            .function_id
            .and_then(|fid| self.program.functions.get(fid as usize));
        let mut locals: Vec<(String, KindedSlot)> = Vec::with_capacity(end - base);
        for i in base..end {
            let local_idx = i - base;
            let name = func_opt
                .and_then(|f| f.param_names.get(local_idx).cloned())
                .unwrap_or_else(|| format!("local_{local_idx}"));
            let bits = self.stack[i];
            let kind = self.kinds.get(i).copied().unwrap_or(NativeKind::Bool);
            let cloned = clone_slot_kinded(bits, kind);
            locals.push((name, cloned));
        }

        // Args: the call-frame ABI stores args contiguously in the
        // locals window for typed call shape — the first `arity` slots
        // are args, the rest are body locals. Surface them as a separate
        // vector for `state.args()` consumers.
        let args: Vec<KindedSlot> = if let Some(func) = func_opt {
            let arity = (func.arity as usize).min(top.locals_count);
            let mut out = Vec::with_capacity(arity);
            for i in 0..arity {
                let slot_idx = base + i;
                if slot_idx >= self.stack.len() {
                    break;
                }
                let bits = self.stack[slot_idx];
                let kind = self.kinds.get(slot_idx).copied().unwrap_or(NativeKind::Bool);
                out.push(clone_slot_kinded(bits, kind));
            }
            out
        } else {
            Vec::new()
        };

        (args, locals)
    }

    /// Module bindings captured at snapshot time.
    ///
    /// Binding names come from the program's `module_binding_names` when
    /// available; otherwise indexed names are used.
    fn snapshot_module_bindings_for_accessor(&self) -> Vec<(String, KindedSlot)> {
        let n = self
            .module_bindings
            .len()
            .min(self.module_binding_kinds.len());
        let mut out: Vec<(String, KindedSlot)> = Vec::with_capacity(n);
        for i in 0..n {
            let name = self
                .program
                .module_binding_names
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("binding_{i}"));
            let bits = self.module_bindings[i];
            let kind = self.module_binding_kinds[i];
            let cloned = clone_slot_kinded(bits, kind);
            out.push((name, cloned));
        }
        out
    }

    fn snapshot_instruction_count(&self) -> usize {
        self.instruction_count
    }

    /// Try to borrow the `OwnedClosureBlock` whose `ptr` matches a
    /// `closure_heap_bits` payload. Returns `None` if no closure
    /// layout is registered for the block's type_id.
    ///
    /// This routes through the program's `closure_function_layouts`
    /// table populated at load time per the §2.7.8 / Q10 layout-side-
    /// table protocol.
    fn try_borrow_closure_block(
        &self,
        ptr: *mut u8,
    ) -> Option<shape_value::v2::closure_raw::OwnedClosureBlock> {
        use shape_value::v2::closure_raw::{
            OwnedClosureBlock, retain_typed_closure, typed_closure_function_id,
        };
        if ptr.is_null() {
            return None;
        }
        // SAFETY: the block's `function_id` is stored in the
        // TypedClosureHeader at the top of the block. The closure-layout
        // side-table is keyed by `func_id` (Vec<Option<Arc<ClosureLayout>>>
        // per program.closure_function_layouts), so we index that table
        // with the recovered function_id to recover the layout.
        let fn_id = unsafe { typed_closure_function_id(ptr) };
        let layout = self
            .program
            .closure_function_layouts
            .get(fn_id as usize)
            .and_then(|opt| opt.clone())?;
        // SAFETY: ptr is a live OwnedClosureBlock allocation (per the
        // §2.7.8 / Q10 construction contract on `closure_heap_bits`).
        // We bump the strong-count share via `retain_typed_closure` so the
        // resulting `OwnedClosureBlock`'s `Drop` doesn't free the share
        // the live frame still owns. `from_raw` then takes a new share.
        unsafe {
            retain_typed_closure(ptr);
        }
        let block = unsafe { OwnedClosureBlock::from_raw(ptr as *const u8, layout) };
        Some(block)
    }
}

/// Clone a `(bits, kind)` pair through the §2.7.7 `clone_with_kind`
/// dispatch — bumping the strong-count share for heap-bearing kinds.
/// Wraps the result in a `KindedSlot` carrier.
fn clone_slot_kinded(bits: u64, kind: NativeKind) -> KindedSlot {
    // W5 v0.3 fix (2026-05-17): bump the underlying refcount via
    // `clone_with_kind` BEFORE wrapping in a `KindedSlot` carrier.
    //
    // The previous shape (`KindedSlot::new(...)` then `carrier.clone()`)
    // was a share-accounting double-release: `KindedSlot::new` claims
    // ownership without bumping the refcount, so the carrier and the
    // live VM both claim the same share. The `carrier.clone()` bump and
    // `carrier`'s Drop release exactly cancel each other, leaving the
    // refcount unchanged — but the returned `cloned` KindedSlot owns a
    // share that doesn't exist. When the snapshot drops later, the
    // cloned's Drop decrements the refcount to 0 while the live VM
    // binding/stack still holds an owning reference. The next access
    // through the VM-side reference is a use-after-free.
    //
    // The correct shape mirrors `module_binding_read_owned_kinded` in
    // `executor/mod.rs:792-796` and `OwnedClosureBlock::read_capture_kinded`'s
    // call site discipline: explicit `clone_with_kind` retain followed
    // by `KindedSlot::new` claim of the freshly minted share. The live
    // VM keeps its own share unchanged.
    //
    // Empirically isolated via `eprintln` instrumentation in
    // `docs/cluster-audits/v0.3-w5-v2-raw-residuals.md` §1; root cause
    // identical to the cluster-1.5 share-accounting double-release class
    // (Round 13 T5 closure-self / cluster-1.5 args/captures), surfaced
    // at the snapshot-clone boundary rather than the closure-call
    // boundary.
    crate::executor::vm_impl::stack::clone_with_kind(bits, kind);
    KindedSlot::new(ValueSlot::from_raw(bits), kind)
}

impl VmStateAccessor for VmStateSnapshot {
    fn current_frame(&self) -> Option<FrameInfo> {
        self.frames.last().cloned()
    }

    fn all_frames(&self) -> Vec<FrameInfo> {
        self.frames.clone()
    }

    fn caller_frame(&self) -> Option<FrameInfo> {
        // Caller is the second-to-last frame (the last is the currently-
        // executing frame).
        let len = self.frames.len();
        if len < 2 {
            None
        } else {
            Some(self.frames[len - 2].clone())
        }
    }

    fn current_args(&self) -> Vec<KindedSlot> {
        self.current_args.clone()
    }

    fn current_locals(&self) -> Vec<(String, KindedSlot)> {
        self.current_locals.clone()
    }

    fn module_bindings(&self) -> Vec<(String, KindedSlot)> {
        self.module_bindings.clone()
    }

    fn instruction_count(&self) -> usize {
        self.instruction_count
    }
}
