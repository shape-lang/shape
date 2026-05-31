# Adversarial Review — lens: snapshot-identity-correctness

Target: `docs/design/v0.3.3-reference-serialization/DESIGN-DRAFT.md`
Code verified at workspace HEAD (`main`, `67768f17`).

**VERDICT: HOLES-FOUND.** Five breaks, two of them fatal to the headline case.
The draft's own O1 calls the central fork "blocking and unresolved"; the breaks
below show the *lean* the draft recommends (heap-ify the referent's storage via
`SharedCow`, keep `Local` ref a non-owning coordinate, serialize `Local`
symbolically) is internally contradictory and produces wrong-memory reads even
*before* any snapshot is taken.

---

## BREAK 1 (FATAL) — promotion to `SharedCow` makes the live `Local` coordinate read a `SharedCell*` as the projected scalar

Triggering shape (the draft's own headline case, §2.2 row 1):

```shape
fn f() -> &int {
    let local = 42      // referent
    let r = &local      // RefTarget::Local{ frame_index=K, slot_index=S, kind=Int64 }
    return r            // ReturnSlot escape — design says PROMOTE, not B0003
}
```

The draft (§3.1, O1-lean §9) resolves the referent's promotion to
`BindingStorageClass::SharedCow` while keeping the *reference* a non-owning
`RefTarget::Local` coordinate.

In code, `SharedCow` promotion is implemented by `op_alloc_shared_local`
(`crates/shape-vm/src/executor/variables/mod.rs:1459-1535`). It **rewrites the
local slot in place**: line `1530-1534` does
`stack_write_kinded(slot, cell_bits, NativeKind::Ptr(HeapKind::SharedCell))` —
the slot at `bp+idx` no longer holds the `Int64` `42`; it holds
`Arc::into_raw(Arc<SharedCell>) as u64` with kind `Ptr(HeapKind::SharedCell)`.

But the reference resolves through `read_ref_target`
(`variables/mod.rs:2976-2998`): for `RefTarget::Local` it computes
`slot = base_pointer + slot_index` and returns
`(stack_read_kinded_raw(slot).0, *kind)` — i.e. it reads the **raw slot bits**
and stamps them with the **kind captured at MakeRef time (`Int64`)**, ignoring
the slot's *current* stored kind (`_stored_kind` is discarded at line `2997`).

After promotion the slot bits are a `*const SharedCell`, but the ref returns
them **tagged as `Int64`**. `op_deref_load` then pushes a `SharedCell` pointer
onto the stack labeled `Int64` and arithmetic on it reads heap-pointer bits as
an integer. This is silent wrong-memory / type-confusion, produced **at live
execution time, with no snapshot involved at all.**

The draft never reconciles "promote the referent's storage to a heap cell" with
"keep the ref a coordinate that reads the slot directly as the projected kind."
These two decisions are mutually exclusive: either the ref must be rewritten to
go *through* the cell (which makes it heap-owning — the §3.2/§5.4 cycle+double-
drop trap the draft itself warns against), or the slot must keep holding the
scalar (which means promotion did nothing to extend the lifetime). The draft
picks both halves of the contradiction.

`op_alloc_shared_local` cannot even be ordered after `MakeRef`: the ref captured
`kind=Int64` at construction (`op_make_ref`, `:2540-2545`). Whatever order the
emit takes, the coordinate-resolution kind and the post-promotion slot kind
diverge.

---

## BREAK 2 (FATAL) — `frame_index` is an absolute `call_stack` index of a frame that the ReturnSlot escape *pops*; the referent slot is destroyed before snapshot can see it

Triggering shape: same `fn f() -> &int { let local = 42; let r = &local; return r }`,
then snapshot the caller that holds the returned `r`.

`op_make_ref` stamps `frame_index = (self.call_stack.len() - 1) as u32`
(`variables/mod.rs:2522-2526`) — the **absolute index of the callee frame at
MakeRef time**, and `slot_index = local_idx` relative to that frame's
`base_pointer`.

`return_value_inner` (`control_flow/mod.rs:763-812`) executes:
`self.call_stack.pop()` (`:768`) then `self.truncate_stack(bp)` (`:778`) where
`bp = frame.base_pointer`. `truncate_stack` walks the parallel-kind track and
`drop_with_kind`s every slot in `[bp..sp)` and resets `sp` to `bp`. **The
referent slot `stack[bp+slot_index]` is dropped and is now beyond `sp`; the
frame at absolute index K is gone from `call_stack`.**

The reference `r` now lives in the *caller's* frame. Its coordinate
`(frame_index=K, slot_index=S)` is dangling the instant the function returns:
- `call_stack[K]` no longer exists (the callee was popped),
- even if some later callee re-occupies index K, its `base_pointer` and slot
  layout are unrelated, so `base_pointer_newK + S` aliases an arbitrary slot.

`snapshot()` (`executor/snapshot.rs:139-215`) captures only the live stack
`stack[0..sp]` (`:154`) and the live `call_stack` (`:198`). The referent is not
in either — it was dropped at return. So the draft's §4.1 "serialize `Local`
symbolically and re-index into the restored `call_stack`" has **nothing to
re-index into**: at snapshot time the frame and slot the coordinate names do not
exist.

This is the core defect the draft's O1/O6 hand-waves: it says "verify a
`SharedCow`-promoted local slot has a stable `(frame_index, slot_index)` that
survives W17 restore." It does not — the promoted slot lives in a frame that the
*return itself* pops. The ReturnSlot escape (the headline flip) is precisely the
case where the originating frame is destroyed, so a coordinate into it can never
be stable. The whole "symbolic Local" serialization path the draft leans on is
unreachable for the one case that motivates the feature.

(For BREAK 2 to be survivable the referent would have to migrate out of the
popped frame into a heap cell whose identity is independent of frame indices —
i.e. the ref must become heap-owning, which is BREAK 1's contradiction and the
draft's own §3.2/§5.4 trap.)

---

## BREAK 3 — `frame_index` is not even a stable identity *within* one live VM, before any return

Triggering shape:

```shape
fn deep(n: int) -> int {
    let x = n
    let r = &x          // frame_index = current depth
    if n > 0 { deep(n-1) }   // pushes/pops frames at the SAME absolute indices
    return *r
}
```

`frame_index` is `call_stack.len()-1` — a **depth ordinal**, not a frame
identity. Recursion (or any sibling call sequence) reuses the same absolute
indices for different frames. The draft's §4.2 wire format serializes
`Local{frame_index, slot_index}` as bare integers and §4.4 Phase C "re-indexes
into the restored call_stack." If the restored `call_stack` has a *different
frame* at index K (any program where the snapshot is taken at a different call
depth than the ref's origin, or after the origin frame returned and a sibling
re-entered), the coordinate resolves into the wrong frame's slot with no
diagnostic. The draft assumes `(frame_index, slot_index)` is a serializable
identity; it is a transient stack coordinate whose meaning depends on the entire
live call_stack shape at one instant.

The `u32::MAX` top-level sentinel (`op_make_ref:2522`, `read_ref_target:2983`)
adds a second hazard: a top-level ref serializes `frame_index=u32::MAX` and on
restore resolves to `base_pointer=0`. If the restored VM's slot-0 region is laid
out differently (different program, different top-level local count), the
sentinel silently reads the wrong root slot.

---

## BREAK 4 — `TypedField` identity token is a raw heap pointer with no provenance check on restore; the design re-`v2_retain`s `by_token[token]` with the wrong allocation pair

Triggering shape:

```shape
type P { x: int }
fn g() -> &int { let p = P{ x: 1 }; let r = &p.x; return r }   // RefTarget::TypedField
```

`RefTarget::TypedField{ receiver: TypedObjectPtr, field_offset, kind }`
(`reference.rs:84-88`) owns one RC share of the object and resolves by reading
`receiver.slots[field_offset].raw()` (`read_ref_target:3005-3012`). The draft
§4.3 interns `receiver.0` (a raw `*const TypedObjectStorage`) as a `u64` token
and §4.4 Phase C does "one `v2_retain` on `by_token[token]`."

Two problems:

(a) **The token is a process-local raw pointer used as a serialization key.** It
is meaningful only as a dedupe key *within one serialize pass* (fine), but the
draft also stores it in the wire format as `referent_token` and the restore must
map it through `RestoreIdentityMap.by_token`. The draft's §4.3 says
"`intern_typed_object` reserves the token before recursing" — but the *referent
body* itself is serialized through "the existing `TypedObject` arm." The
existing `TypedObject` snapshot arm is value-copy serialization; it does **not**
preserve object identity for the *binding* that also holds that object. So a
program where the same object is reachable both as a `TypedField` ref referent
*and* directly as a stack/module binding will serialize the object **twice**
(once via `heap_referents`, once via the binding's `TypedObject` arm), and on
restore the ref points at the `heap_referents` copy while the binding points at
its own copy. Mutation through the ref is then invisible to the binding —
**aliasing contract silently broken** (the draft's own P3/P5 properties fail).
The draft asserts dedupe "regardless of which slot referenced it" (§4.3) but only
the `heap_referents` table is deduped; the ordinary `TypedObject` arm at every
non-ref slot is not threaded through `IdentityWriter`, and §4.0 confirms no
identity-map exists today to make it so.

(b) **Allocation-provenance mismatch on restore.** `read_ref_target` for
`TypedField` reads `receiver.slots[..]` directly assuming the receiver is a
`_new`-path-allocated `TypedObjectStorage` (HeapHeader at offset 0,
`reference.rs:74-83`). The cluster-1.5 / W5 audit (CLAUDE.md Known Constraints;
`vm_state_snapshot.rs:295`) is explicit that mixing `Arc::new(...)` +
`Arc::into_raw` reconstruction with the v2-raw `_new` `drop_with_kind` dispatch
produces SIGABRT. The draft's Phase A "uses the existing `TypedObject`
reconstruction" without stating which allocator path that reconstruction uses.
If it is the `Arc::new` path (as the legacy snapshot restore does), the
`v2_retain` in Phase C and the eventual `drop_with_kind(Ptr(HeapKind::Reference))`
release operate on `Arc::new`-allocated memory through the `_new` release path —
the exact double-free class the audit flagged. The design must pin the restore
allocator to `_new` and prove it; it does neither.

---

## BREAK 5 — `is_mut` dropped from the wire format silently downgrades exclusivity diagnostics if the resumed program *extends* (not replays) the snapshot

Triggering shape: a snapshot/resume host (the wire server, `shape wire-serve`)
resumes a VM and continues execution with **new** bytecode appended (the documented
"resumable distributed execution" use case — snapshot is not only replay-identical).

The draft §4.5 / O5 recommends dropping mutability from the wire (`is_mut`
reserved, always `false`), justified by "the resumed VM re-executes the same MIR
with the same proof." That justification holds **only if resume is bit-identical
replay**. But B0001 exclusivity
(`solver.rs:1073-1079`, `ConflictExclusiveExclusive`) is a **compile-time,
per-function, intra-MIR** property (verified: `grep RwLock|RefCell|try_borrow`
over the ref runtime path returns nothing — there is zero runtime borrow
tracking). A restored `&mut` reference carries **no runtime exclusivity state**.
If the resumed image runs *any* MIR not present in the original snapshot (a
continuation function, an injected expression, the REPL on a resumed VM), there
is no fact in the restored VM that says "this referent is exclusively borrowed."
A second `&mut` taken by the continuation is solved against the *continuation's*
own MIR, which has no record of the live restored loan — exclusivity is silently
lost.

The draft's whole soundness argument for §5 Q1 ("Coherence SAFE — restore
replays the same bytecode") is load-bearing on resume ≡ replay. The snapshot
feature's stated purpose ("resumable distributed execution", CLAUDE.md) is
explicitly *not* pure replay. Either the draft must (a) restrict resume to
bit-identical replay and prove the runtime enforces it, or (b) carry enough state
to re-establish the loan in the resumed solver. It does neither; it removes the
one bit (`is_mut`) that a future runtime check would need.

---

## Cross-cutting note

BREAK 1 and BREAK 2 together show the draft's O1 "lean" (heap-ify the referent's
*storage* but keep the `Local` ref a non-owning coordinate, serialize `Local`
symbolically) cannot hold: promotion via `op_alloc_shared_local` rewrites the
slot to a `SharedCell` pointer (BREAK 1 — coordinate now reads wrong kind), and
the ReturnSlot escape pops the frame the coordinate names (BREAK 2 — coordinate
dangles). The only escape is to make the ref heap-owning through the cell — which
is the cycle-leak + double-drop trap the draft itself flags (§3.2/§5.4) and which
re-opens the W5 SIGABRT class. The fork O1 declares "blocking" is not merely
unresolved; the recommended resolution is unsound. The feature as drafted should
not proceed to code until O1 is resolved in favor of a heap-owning promoted-cell
carrier *with* a proven single-owner-of-record discipline and a frame-independent
referent identity — i.e. essentially the `RefTarget::SharedCell` variant the
draft lists as the *rejected* alternative.
