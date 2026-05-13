# W17-vm-call-value-closure-kind-mismatch — Round 13 T5 audit

**Sub-cluster:** `W17-vm-call-value-closure-kind-mismatch`
**Branch:** `bulldozer-strictly-typed-w17-vm-call-value-closure-kind-mismatch`
**Branched from:** `3db6e820` (post-Round-12-merge + Round 13 dispatch metadata).
**Dispatch shape:** AUDIT-FIRST per phase-3-cluster-0-status.md §"Round 13 dispatch plan".
**Audit date:** 2026-05-13.

## §1. Site identification

### Surface (consumer)

`crates/shape-vm/src/executor/call_convention.rs:795-810` — the closure
arm of `call_value_immediate_nb` matches
`callee.slot.as_heap_value()` against `HeapValue::ClosureRaw(block)`.
On the second iteration of `xs.map(|x| x*2)` over `xs = [1,2,3]`, the
match arm rejects the payload and surfaces the debug-assert /
RuntimeError documented in the T5 prompt
(`"HeapKind::Closure label with non-ClosureRaw HeapValue payload: \"string\""`).

In release mode (where `debug_assert!` is inert) the same defect
surfaces a few instructions deeper at
`call_convention.rs:683` — `call_closure_with_nb_args_keepalive`'s
`program.functions.get(func_id as usize).ok_or(VMError::InvalidCall)?`
guard rejects the bogus `func_id` (62449 in one observed run, vs the
correct 1 on the first iteration). This is the surface the CLI reports
as `Runtime error: Invalid function call`.

Both surfaces are correct: they faithfully report that the closure
payload was decoded as garbage. **The consumer-side handling is right.**
The bug is upstream.

### Surface dispatch path (compile-side)

`xs.map(|x| x*2)` on an inferred `Array<int>` is NOT dispatched as a
`CallMethod`. The compiler desugars it to an inline for-loop. The
bytecode trace for the smoke program (`[1,2,3]`, `let doubled = xs.map(|x| x*2)`)
shows the loop sequence:

```
ip=10 CloneLocal Local(0)        ; clone xs onto stack
ip=11 PushConst Const(0)         ; iterator init
ip=12 StoreLocal Local(4)        ; loop counter
ip=13 LoopStart
...
ip=26 CloneLocal Local(1)        ; clone CLOSURE from Local 1 onto stack
ip=27 LoadLocalI64 Local(5)      ; load x (current element)
ip=28 PushConst Const(1)         ; arg_count = 1
ip=29 CallValue                  ; dispatch_call_value_immediate → call_value_immediate_nb
ip=44 ...                        ; closure body executes
ip=49 ReturnOwned
ip=50 ReturnValueI64
ip=30 ArrayPushLocal Local(2)    ; push result into doubled array
ip=33 Jump (-21)                 ; loop back
```

Neither `op_call_method` nor `handle_map_v2` nor `handle_int_map` is
ever invoked for this surface — confirmed by inserting `eprintln!`
probes at each of those entries during audit. The closure-call path
goes through `op_call_value` → `dispatch_call_value_immediate` →
`call_value_immediate_nb`.

### Producer-side classification (§2.7.5)

**The kind label is correct.** On both iterations the closure carrier
on the stack has `kind = NativeKind::Ptr(HeapKind::Closure)` and the
same raw bits pointer. Audit trace from instrumented
`call_value_immediate_nb` entry:

```
[T5 AUDIT] call_value_immediate_nb: callee.kind=Ptr(Closure), raw_bits=0x57e3b066f820, args.len()=1
[T5 AUDIT] call_value_immediate_nb: function_id=1 capture_count=0
[T5 AUDIT] call_closure_with_nb_args_keepalive: func_id=1 functions.len()=116 found=true
... closure body runs ...
[T5 AUDIT] call_value_immediate_nb: callee.kind=Ptr(Closure), raw_bits=0x57e3b066f820, args.len()=1
[T5 AUDIT] call_value_immediate_nb: function_id=62449 capture_count=0    ; <-- GARBAGE
[T5 AUDIT] call_closure_with_nb_args_keepalive: func_id=62449 functions.len()=116 found=false
Error: Runtime error: Invalid function call (line 4)
```

Same bits, second invocation. `function_id = typed_closure_function_id(block.as_ptr())`
reads the header at the closure-block pointer. The pointer is dangling
on iteration 2 because the underlying `Arc<HeapValue>` has been freed
and its memory reused (by a String allocation in one observed run —
matching the T5 prompt's exact wording).

### Root cause (defect site)

`crates/shape-vm/src/executor/call_convention.rs:835-841` in the
closure arm of `call_value_immediate_nb`:

```rust
self.call_closure_with_nb_args_keepalive(
    function_id,
    block,
    args,
    Some(callee.slot.raw()),    // <-- closure_heap_bits transfer share
    Some(callee.kind),
)?;
```

This passes `callee.slot.raw()` and `callee.kind` to the frame's
`closure_heap_bits` / `closure_heap_kind` companion fields (B9
Wave-α lockstep). On `op_return_value` (or `op_return`) the frame
teardown path at `control_flow/mod.rs:712-726` / `:774-788` releases
that pair via `drop_with_kind(closure_heap_bits, closure_heap_kind)` —
**one `Arc::decrement_strong_count::<HeapValue>` share retired** per
the §2.7.7 Closure arm in `vm_impl/stack.rs:552-554`.

Meanwhile, the `callee` carrier in
`dispatch_call_value_immediate` (`control_flow/mod.rs:408-409`) was
constructed from a `pop_kinded()` that transferred ONE share from the
stack. After `call_value_immediate_nb` returns, the carrier drops at
end of scope and `KindedSlot::Drop` (`kinded_slot.rs:489-554`)
releases **another `Arc::decrement_strong_count::<HeapValue>` share**.

Net accounting per call: 1 share entered the dispatch (via `pop_kinded`);
2 shares retired (frame teardown + carrier `Drop`).

On the first iteration this happens to "work" because the closure
binding at Local 1 also holds one share (installed by the `CloneLocal
Local(1)` at `ip=26` via `clone_with_kind`). After the call:
Local 1 still has its share, but the actual Arc refcount is now 0
(2 retains during clone + pop minus 2 drops during frame + carrier).
The Arc payload is freed; Local 1's bits become dangling.

On the second iteration:
1. `CloneLocal Local(1)` at `ip=26` reads the dangling bits and calls
   `clone_with_kind` — `Arc::increment_strong_count` on freed memory
   (UB; in practice the allocator may have refilled the slot with a
   different `Arc<T>` such as the per-iteration String).
2. `pop_kinded` and the carrier construction propagate the dangling
   bits as `Ptr(HeapKind::Closure)`.
3. `as_heap_value()` returns whatever HeapValue arm now lives at that
   address — observed as `"string"` in the T5 prompt's surface and as
   a garbage TypedClosureHeader (`function_id = 62449`) in this audit
   trace.

The kind track is honest. The carrier ABI is honest. The frame
teardown is honest. **The defect is a single-bit asymmetry between
"how many shares we acquired" and "how many shares we promised to
release" at the value-call dispatch site.**

### Producer-site (§2.7.5) for the share imbalance

The site that decided to install `closure_heap_bits = Some(callee.slot.raw())`
is `call_convention.rs:835-841` (W7-cv-static Round 2 close
`06cdfce`, 2026-05-09). The accompanying docstring at line 828-834
says:

> Frame setup. The B9 lockstep companion fields carry the closure-self
> share so `op_return` / `op_return_value` can release it via
> `drop_with_kind(bits, kind)` on frame teardown.

The docstring assumes the share is **donated** to the frame — but the
producer never compensated by either:

- (a) **`mem::forget`ing the callee carrier** to suppress the carrier-side
  release (transferring the share into the frame), OR
- (b) **`clone_with_kind(callee.slot.raw(), callee.kind)`** before the
  call to install an additional share for the frame (caller keeps its
  carrier-share, frame gets its own share).

Both options match §2.7.11/Q12 share-accounting discipline; the
current code does neither, surfacing as the audited double-free.

## §2. W7/W8 overlap check

Reviewed:

- **Round 7B audit doc** (`7753d52b` 2026-05-12, audit-only close):
  W12-jit-collection-typed-arc-ffi. Scope = JIT-side typed-Arc
  allocation FFI for 8 collection HeapKinds. Did NOT touch
  `call_value_immediate_nb` or the closure carrier share accounting.
- **Round 8B audit doc** (`ba09636b` 2026-05-13, audit-only close):
  W12-jit-collection-method-dispatch-abi. Scope = JIT-side method
  dispatch shell delegating to VM-side kinded dispatch. Did NOT touch
  `call_value_immediate_nb` either.
- **Round 9 close** (`81acb62e` 2026-05-13): typed-Arc collection ctors
  + per-HeapKind retain/release. Touched
  `vm_impl/stack.rs::clone_with_kind` / `drop_with_kind` for 8 collection
  arms — Closure arm was untouched (it was already wired in W7-closure-
  retain Round 2.5, commit `5fa4b19`).
- **Round 10 close** (`2c2ecdf1` 2026-05-13): `jit_call_method` shell
  rebuild + trampoline. Touched `objects/mod.rs::op_call_method` and
  `objects/method_registry.rs`. Did NOT touch `call_value_immediate_nb`
  or the value-call frame setup.
- **W7-cv-static Round 2 close** (`06cdfce` 2026-05-09): the producer
  site itself. This is the commit that introduced the
  `Some(callee.slot.raw()), Some(callee.kind)` shape at lines 835-841.
- **W7-closure-retain Round 2.5 close** (`5fa4b19` 2026-05-09): wired
  `clone_with_kind` / `drop_with_kind` Closure arms to
  `Arc::increment/decrement_strong_count::<HeapValue>`. The two arms are
  symmetric (one increment per `clone_with_kind`, one decrement per
  `drop_with_kind`) — the audited bug is upstream of these arms.

**Disposition:** NEW finding. Not absorbed, not superseded, not
orphaned. The defect entered the tree at `06cdfce` (W7-cv-static, the
`call_value_immediate_nb` body fill) and has been latent until kickoff
Smoke 2's `.map(|x| x*2)` exercised the inline-loop CallValue path
across multiple iterations.

## §3. Cluster-0 disposition

**Confirmed cluster-0 sub-cluster.** Reproducer is the kickoff Smoke 2
full VM program:

```shape
let xs = [1, 2, 3, 4, 5]
let doubled = xs.map(|x| x * 2)
print(doubled.sum())
```

Built worktree binary inside devenv (debug + release; both reproduce).
Failure mode under VM mode:

```
Error: Runtime error: Invalid function call (line 4)
```

In release the error string is `VMError::InvalidCall`; with the
debug-assert active the surface is the T5 prompt's expected
`call_convention.rs:798` debug-assert message
(`"HeapKind::Closure label with non-ClosureRaw HeapValue payload"`).
Both surfaces are downstream symptoms of the same producer-side
share-accounting defect at lines 835-841.

A minimal reproducer (without `.sum()`) confirms `.map(|x| x*2)` alone
suffices to fire the bug:

```shape
let xs = [1, 2, 3]
let doubled = xs.map(|x| x * 2)
print(doubled)
```

Both `xs.map` (any iteration count ≥ 2 triggers) and a direct
`f(5)` closure call (single invocation, no use-after-free reached)
sit in the same `op_call_value` dispatch tree; the latter happens to
escape the bug because the carrier-and-frame double release runs
exactly once on program shutdown, by which point nothing observes the
dangling pointer.

Cluster-0 verdict: the defect blocks kickoff Smoke 2 full VM.
**Round 13 T5 ownership confirmed.**

## §4. Fix shape

The fix is **producer-side share accounting at `call_value_immediate_nb`**,
not consumer-side. The kind label is correct, the kind track is correct,
the consumer's classification is correct.

### Option A (preferred): `mem::forget` the callee carrier after frame setup

The callee's share transfers cleanly into the frame's
`closure_heap_bits` companion. The carrier becomes "donor" and must
not release on `Drop`.

The shape mirrors `execute_closure` at `call_convention.rs:172-173`:

```rust
self.call_closure_with_nb_args_keepalive(function_id, closure_block, &args, None, None)?;
std::mem::forget(args);
```

— there `args` is `Vec<KindedSlot>` and the carriers transfer their
shares into the frame's locals; `mem::forget` suppresses the carrier
`Drop`s that would otherwise double-release.

For `call_value_immediate_nb` the analogous shape is to either:

- **A1.** Have `dispatch_call_value_immediate` `mem::forget` the
  callee carrier when the call routes through the Closure arm. This
  is awkward: the carrier is constructed in
  `dispatch_call_value_immediate` (control_flow/mod.rs:408-409) but
  the Closure-vs-UInt64 decision lives inside
  `call_value_immediate_nb`. The caller doesn't know which arm fired.
- **A2.** Change `call_value_immediate_nb`'s signature to take
  `callee: KindedSlot` by-move (not `&KindedSlot`). The Closure arm
  installs the share into the frame and `mem::forget(callee)`s the
  carrier internally; the UInt64 arm `mem::forget`s as well
  (drop_with_kind on UInt64 is a no-op anyway). The W7-cv-polymorphic
  by-borrow-only ABI is for share-non-transfer call shapes (method
  dispatch over `&[KindedSlot]`); the value-call ABI naturally
  transfers ownership and the carrier-by-move shape aligns with that.

  CAVEAT: ADR-006 §2.7.11/Q12 explicitly says `(callee: KindedSlot, args: &[KindedSlot])`
  — so by-move callee is actually the documented ABI. The current
  `&KindedSlot` shape may itself be a drift from the ADR.

### Option B: `clone_with_kind` before the call (caller keeps carrier-share, frame gets new share)

```rust
NativeKind::Ptr(shape_value::HeapKind::Closure) => {
    // ...
    super::vm_impl::stack::clone_with_kind(callee.slot.raw(), callee.kind);
    self.call_closure_with_nb_args_keepalive(
        function_id,
        block,
        args,
        Some(callee.slot.raw()),
        Some(callee.kind),
    )?;
    // ...
}
```

This is symmetric with `execute_function_with_named_args` at
`call_convention.rs:246-250`, which `clone_with_kind`s before
re-homing a named-arg into the positional slot.

Trade-off: every closure call bumps the Arc refcount twice (once for
the carrier-via-pop_kinded, once for the explicit clone) and releases
twice (frame + carrier). One more atomic pair per call than necessary.

Option A2 is the principled fix because it matches the ADR-documented
ABI shape and avoids the extra atomic pair.

### Discipline check

The fix MUST come from §2.7.5 producing-site classification at the
producer (this is producer-side share accounting at the same producer
that installs the kind label). The fix:

- Does NOT Bool-default any kind. The Closure kind is preserved.
- Does NOT add a tag-bits decode. No tag inspection anywhere.
- Does NOT introduce a bridge/probe/helper/hop/translator/adapter/shim.
  It is a `mem::forget` (Rust std primitive) or a `clone_with_kind`
  (already-existing kinded primitive per §2.7.7 / Q9).
- Does NOT preserve mis-labeling for later. The mis-labeling is
  fictitious — the label is correct on day one; the share accounting
  is the bug.

The full migration shape is bounded: ~5-10 LoC in
`call_convention.rs::call_value_immediate_nb` plus possibly the same
treatment for `resolve_spawned_task` at lines 421-475 (same defect
class — the prompt cites this as the second site).

For the surface in `resolve_spawned_task`: the callable share comes
from `take_callable` (which transfers ownership out of the scheduler
map). After installing it as `closure_heap_bits`, the frame teardown
releases. No carrier is held; the local `callable_bits` / `callable_kind`
are raw u64 + NativeKind, not a `KindedSlot` with `Drop`. So the
double-release shape doesn't apply identically. Verify by following the
share path through `take_callable` and frame teardown — likely OK as-is,
but the audit fix-shape should cite this confirmation.

## §5. Recommendation

**Proceed to code edit within this round budget** if Option A2 is
acceptable (by-move callee). The ADR amendment is unnecessary because
§2.7.11/Q12 already documents the by-move shape; the current
`&KindedSlot` is the drift.

The fix is ~10 LoC. The risk surface is the existing call-sites of
`call_value_immediate_nb` — auditing for any that pass `callee` they
need to keep after the call. Quick survey:

- `dispatch_call_value_immediate` (`control_flow/mod.rs:415`) — the
  primary caller; the carrier is dropped at end of scope anyway, so
  moving in is fine.
- `handle_map_v2` / `handle_filter_v2` / `handle_sort_v2` / similar in
  `objects/array_transform.rs` — pass `&args[1]` to
  `call_value_immediate_nb`. These need a different shape: the
  receiver/closure carriers in `args` are borrowed by the method
  dispatch shell (`&[KindedSlot]`); the caller keeps the share for the
  duration. For these the **right** shape is to `clone_with_kind` the
  closure bits inside the by-move callee path (the caller hands over a
  fresh share each iteration, keeps its own carrier for the next
  iteration).

So a sensible signature might be:

```rust
pub fn call_value_immediate_nb(
    &mut self,
    callee: KindedSlot,  // by-move (consumes one share)
    args: &[KindedSlot],
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<KindedSlot, VMError>
```

And callers that need to preserve their own share clone first:

```rust
super::vm_impl::stack::clone_with_kind(closure.slot.raw(), closure.kind);
let result = vm.call_value_immediate_nb(
    KindedSlot::new(ValueSlot::from_raw(closure.slot.raw()), closure.kind),
    &arg,
    ctx.as_deref_mut(),
)?;
```

This is the ADR-documented shape per §2.7.11/Q12 verbatim.

## §6. Audit-then-proceed verdict

§3 confirms cluster-0 territory. §4 fix shape fits within a
single-round budget (estimated ~20-50 LoC across call_convention.rs +
2-3 method handler callers, no ADR amendment).

**Verdict: proceed to code edit per Option A2 in a follow-up
commit.** This audit doc lands first per the audit-first dispatch
shape; the fix commit follows on the same branch.

If the supervisor prefers Option B (clone-first, no signature change)
to minimize blast radius — that's a single-line `clone_with_kind`
insert at the Closure arm in `call_value_immediate_nb`. Trade-off is
one extra atomic pair per closure call. The supervisor decides.
