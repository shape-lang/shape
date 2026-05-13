# W12-vm-new-array-untyped-construction — audit

**Branch:** `bulldozer-strictly-typed-w12-vm-new-array-untyped-construction`
**Parent:** `8db19d21` (post-Round-10 merge + Round 11 dispatch metadata).
**Sub-cluster:** Phase 3 cluster-0 Round 11A standalone AUDIT-FIRST.
**Audit dispatch:** Round 11 dispatch metadata in `phase-3-cluster-0-status.md:1750-1768`.

## Surface

Canonical kickoff Smoke 2 (`[1,2,3,4,5].map(|x|x*2).sum()`) fails under
`--mode vm` with:

```
Error: Runtime error: Not implemented: op_new_array: generic untyped-array
construction depends on the kinded the-deleted-heterogeneous-element-carrier
emit path (Phase 2c reentry — see ADR-006 §2.7.4) (line 4)
```

Reproduced on this worktree at `8db19d21`:

```
$ cargo run --bin shape -- run /tmp/smoke2.shape --mode vm
Error: Runtime error: Not implemented: op_new_array: generic untyped-array
construction depends on the kinded the-deleted-heterogeneous-element-carrier
emit path (Phase 2c reentry — see ADR-006 §2.7.4) (line 4)
```

A second VM-side surface in the same Smoke 2 — `IntrinsicSum` — is
documented at `crates/shape-vm/src/executor/vm_impl/builtins.rs:471-520`
(`todo!("phase-1b-vm wave 5d — intrinsic body migration pending")`).
This sub-cluster's territory is **only the `op_new_array` surface**;
`IntrinsicSum` is wave-5d closure-driven-builtins territory, orthogonal.

## §1. Surface site identification

### Error-emission site

**Single source location:**
`crates/shape-vm/src/executor/objects/object_creation.rs:304-324`
(method `VirtualMachine::op_new_array`).

The handler matches `Operand::Count(count)`, pops `count` kinded slots,
retires each via `drop_with_kind`, then surfaces
`VMError::NotImplemented(...)` with the message text quoted verbatim
above (file lines 315-320).

### OpCode definition

`OpCode::NewArray` is defined at
`crates/shape-vm/src/bytecode/opcode_defs.rs` (variable-arity opcode with
`Operand::Count` carrier, called out in the file's variable-arity
section comments at lines 34 + 56).

### Dispatcher

`crates/shape-vm/src/executor/objects/mod.rs:208`:
```rust
NewArray => self.op_new_array(instruction)?,
```
inside the `objects` dispatch table reached from
`executor/dispatch.rs:716` (`NewArray` arm). No intermediate handler.

### Compile-time emit sites

`OpCode::NewArray` is emitted from 9 compile-time sites in
`crates/shape-vm/src/compiler/`:

1. `compiler/expressions/collections.rs:261` —
   `compile_expr_array` legacy path. Fires for **non-empty** heterogeneous
   array literals or any literal that cannot route through the v2 typed
   fast path (spreads, nested-array elements, etc.).
2. `compiler/expressions/collections.rs:259-264` — same function, count=N.
3. `compiler/expressions/mod.rs:425` — annotation-related empty-array
   default (`@ann ctx event_log` allocator).
4. `compiler/expressions/mod.rs:473, 488, 664` — annotation compile path
   empty-array allocators (more annotation context init).
5. `compiler/loops.rs:961` — `compile_list_comprehension` initial empty
   result accumulator (`__comp_result`).
6. `compiler/loops.rs:1126` — `compile_array_with_spread` initial empty
   accumulator (`__array_result`).
7. `compiler/functions_annotations.rs:225, 1412, 1439, 1674` —
   annotation handler args/event-log array allocators.
8. `compiler/patterns/helpers.rs:108-110` — `excluded_keys` array passed
   to `BuiltinFunction::ObjectRest`.
9. `compiler/functions_foreign.rs:601-603` — `out`-param tuple result
   construction for `extern C fn`.

Crucially, the **homogeneous-numeric / bool array literal** path bypasses
`NewArray` entirely: `compile_expr_array` first attempts a v2 typed-array
fast path emitting `NewTypedArrayI64` / `NewTypedArrayF64` /
`NewTypedArrayBool` + `TypedArrayPush*` per element
(`expressions/collections.rs:215-227`). The Smoke 2 literal `[1,2,3,4,5]`
takes that fast path — it does **not** hit `op_new_array` directly. The
v2 typed fast path also covers the `NewTypedArray` legacy fallback at
`collections.rs:255-258` for homogeneous-int/number/bool literals
without the v2 wiring.

### Where Smoke 2 hits the surface

The literal `[1,2,3,4,5]` lowers to `NewTypedArrayI64`, not `NewArray`.
The `.map(...)` call dispatches via `OpCode::CallMethod` to the PHF
registry. For a v2 typed-int-array receiver (`NativeKind::UInt64` + v2
view tag), method resolution at `executor/objects/mod.rs:534-546`
consults `TYPED_INT_ARRAY_METHODS` first, falling back to
`ARRAY_METHODS` via `.or_else`. `ARRAY_METHODS["map"]` is
`array_transform::handle_map_v2` (a native `MethodFnV2` handler).

`handle_map_v2` invokes the closure per-element, collects results via
`collect_homogeneous_results` into a typed-array variant. **No
`op_new_array` invocation in this path** at the method-dispatch tier
itself.

However, `xs.map(|x| x * 2)` then dispatches the closure body via
`call_value_immediate_nb`, which executes the user closure's bytecode
through the standard dispatch loop. The closure `|x| x * 2` produces an
`Int64` result — its body emits typed arithmetic + `ReturnValue`, no
`NewArray`.

That leaves three remaining trigger candidates worth verifying with a
bytecode dump (left as a code-fix-side verification step, **not blocking
this audit's deliverable** since the fix-shape is independent of the
exact trigger):

- (a) Some stdlib-source `Vec.map<U>` body at
  `crates/shape-runtime/stdlib-src/core/vec.shape:51-57` reaches the
  bytecode stream as a fallback when PHF doesn't match. `Vec.map`'s body
  begins with `let mut result = []` which compiles to
  `OpCode::NewArray, Count(0)` (empty-array fall-through in
  `compile_expr_array`).
- (b) An annotation handler context-init array (sites #3/#4/#7 above)
  fires during a stdlib trace path. Empty-array Count(0) variant of
  `op_new_array` surfaces identically — same NotImplemented body.
- (c) Some other empty-array initializer hit at runtime via a stdlib
  function the trace exercises before the `.map` body returns.

In **all three cases the fix is the same** — `op_new_array` must produce
a kinded `TypedArrayData::*` variant at construction time, including a
stable default for the `Count(0)` empty-array shape. The audit's
fix-shape proposal in §4 handles both.

## §2. §-cite verification

### The error message cites §2.7.4

`crates/shape-vm/src/executor/objects/object_creation.rs:317-318`:

```rust
"op_new_array: generic untyped-array construction depends \
 on the kinded the-deleted-heterogeneous-element-carrier emit path \
 (Phase 2c reentry — see ADR-006 §2.7.4)"
```

### §2.7.4 actual content

`docs/adr/006-value-and-memory-model.md:392-449` titled "API rebuild
scope clarification" — Phase 1.B/Phase 2c snapshot serializer + stdlib
registration scope clarifications. Body content:

- Snapshot serialization (`nanboxed_to_serializable` deferral).
- Stdlib registration (`register_typed_function` re-introduction).
- Output adapter `PrintResult` move.
- Display / utility helpers (`ValueWordDisplay`, `vmarray_from_vec`).
- Audit accuracy clarification (recipe instances vs site catalogs).

**§2.7.4 has nothing to do with `TypedArrayData` element-kind
construction or polymorphic-element-buffer deletion.** The closest hit
on the cite text is the bare mention of `vmarray_from_vec` in §2.7.4's
"Display / utility helpers" sub-bullet at line 434 — but that bullet
**directs** to the post-`KindedSlot` shape (`direct
TypedArrayData::from_*` constructors), the opposite direction from a
deferral.

This is a **stray cite of the same class** documented at
`crates/shape-jit/src/mir_compiler/statements.rs:236` (Round 5B fix
§2.7.4 → §2.7.14) and `docs/cluster-audits/w12-enum-constructor-audit.md:215`
(audit-tracked stray-cite list). Phase 2c reentry framing was a
broad-strokes placeholder used during the Phase 1.B → Phase 2c handoff
that survived into a comment.

### Correct cite

`docs/adr/006-value-and-memory-model.md:4465-4769` §2.7.24 Q25.A —
**typed-carrier monomorphization bundle**. Specifically:

- §2.7.24 Q25.A line 4476-4516 deletes
  `TypedArrayData::HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)` and
  introduces 9 new monomorphic specializations (`Decimal`, `BigInt`,
  `DateTime`, `Timespan`, `Duration`, `Instant`, `Char`, `TypedObject`,
  `TraitObject`).
- §2.7.24 Q25.A line 4504-4510 explicitly: *"Per-element kind is uniform
  per variant. … No parallel `Vec<NativeKind>` track. The variant tag IS
  the kind."*

Secondary cites bearing on the rebuild path:

- §2.7.5 (cross-crate ABI policy) at `006-value-and-memory-model.md:450-491`
  — the "kind stamped at compile time from the call signature, not on
  the heap object" principle. The `op_new_array` emit-site's kind source
  IS the §2.7.5 producing-call classification: the bytecode compiler
  knows the element kinds it's pushing.
- §2.7.14 Q15 at `006-value-and-memory-model.md:2273-2602` —
  `JitArray` deletion + kinded `TypedArray<T>` re-introduction. The
  Q15 Route A decision (monomorphized `Arc<TypedArrayData>` per element
  kind) shaped the JIT side. The VM-side `op_new_array` rebuild adopts
  the same Route-A discipline at the bytecode-handler layer.

**The audit's §-cite fix is `§2.7.4` → `§2.7.24 Q25.A`** at all five
hits in `object_creation.rs`:

- Line 25 (module docstring referring to the deleted `HeapValue`-carrier
  shape via `op_new_array` cross-reference).
- Line 31-39 (`op_new_object` + `op_new_array` Phase-2c surface
  docstring — `op_new_object` cites the correct §2.7.4 since
  `create_typed_object_from_pairs` is named in §2.7.4 directly; only the
  `op_new_array` paragraph at lines 33-39 carries the wrong cite).
- Line 289 (`op_new_array` per-function docstring).
- Line 317-318 (the runtime error message itself).
- Line 332-333, 388-389, 489-490 (heterogeneous-array fallback
  references in `op_new_typed_array` / `op_new_set` / sibling
  handlers — all cite §2.7.4 incorrectly when they mean §2.7.24).

These five §-cite fixes are mechanical and disposition-independent;
they should land regardless of whether this sub-cluster proceeds to
code or surfaces-and-stops at audit. The fix is included in the §4
fix-shape proposal below as part of the migrating-close path.

## §3. Deleted-carrier identification

### What the message names

The error string contains the deletion-fate name
`the-deleted-heterogeneous-element-carrier` — an awkward hyphenated
descriptor. Searching the source tree finds 41 hits across 18 files —
all of them comment / docstring / error-message text describing the
deleted carrier by deletion-fate. No code references (no struct
definition, no variant, no constructor) bind to the hyphenated name.
This is the canonical deletion-fate naming pattern per CLAUDE.md
"Renames to refuse on sight" — describing deleted code by deletion-fate
rather than hypothetical role.

### Concrete deletion

`docs/adr/006-value-and-memory-model.md:2912-2917` (the `TypedArrayData`
enum definition's tomb-comment):

```rust
// The polymorphic `HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)` catch-all
// was DELETED in checkpoint 4 of W17-typed-carrier-bundle-A per ADR-006
// §2.7.24 Q25.A. Every construction site migrated to a specialized
// variant in checkpoint 2; every reader was filled with real per-arm
// bodies in checkpoint 3. Do not reintroduce under any rename — see
// §Q25.E #1 forbidden pattern list.
```

So the deleted carrier is named **`TypedArrayData::HeapValue` with
payload `Arc<TypedBuffer<Arc<HeapValue>>>`**. Deletion happened during
Phase 2d W17-typed-carrier-bundle-A "checkpoint 4/4". The replacement
plan (already landed for read sites + most construction sites) is the
specialized-variant grid plus a `TypedObject` catch-all for user-defined
types and a `TraitObject` catch-all for `dyn`-erased element types.

### The construction-helper that bridges (bits, kind) → specialized variant

`crates/shape-value/src/heap_value.rs:2937-3023`:
`TypedArrayData::build_specialized_from_heap_arcs(elems: Vec<Arc<HeapValue>>)
-> Result<TypedArrayData, String>`. Empty input defaults to
`TypedArrayData::TypedObject(Arc<TypedBuffer<Arc<TypedObjectStorage>>>)`
with an empty buffer. Non-empty input dispatches on the first element's
`HeapValue` arm (String / Decimal / BigInt / TypedObject / Char) and
requires every subsequent element to match — heterogeneous-arm input
surfaces a structured `Err`.

### The (bits, kind) → Arc<HeapValue> projection helper

`crates/shape-vm/src/executor/builtins/array_ops.rs:49-86`:
`slot_to_heap_arc(slot: &KindedSlot) -> Result<Arc<HeapValue>, VMError>`.
Mirror at `crates/shape-vm/src/executor/objects/hashmap_methods.rs:748-792`
(`result_slot_to_heap_value_arc`). Both already handle the per-kind
projection for Int64 (→ `HeapValue::BigInt(Arc<i64>)`), String,
heap-kinded `Ptr(_)`. Float64 and Bool surface as errors per ADR-006
§2.3 — there is no `HeapValue::Number` arm and no `HeapValue::Bool`
arm. Inline Float64 / Bool elements thus belong in the matching
`TypedArrayData::F64` / `TypedArrayData::Bool` specialized variant
**directly**, not via the heap-arc-wrapper path.

### Construction-site recovery shape

The recovery shape for `op_new_array` is **two-step**:

1. Pop `count` kinded slots from the stack via `pop_kinded`.
2. Dispatch on element-kind homogeneity:
   - All elements inline-Int64 → `TypedArrayData::I64(Arc<TypedBuffer<i64>>)`.
   - All elements inline-Float64 → `TypedArrayData::F64(Arc<AlignedTypedBuffer>)`.
   - All elements inline-Bool → `TypedArrayData::Bool(Arc<TypedBuffer<u8>>)`.
   - All elements inline-String / `Ptr(HeapKind::String)` →
     `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)`.
   - All elements `Ptr(HeapKind::TypedObject)` →
     `TypedArrayData::TypedObject(Arc<TypedBuffer<Arc<TypedObjectStorage>>>)`.
   - All elements `Ptr(HeapKind::Decimal)` →
     `TypedArrayData::Decimal(...)`.
   - … (parallel arms for `BigInt`, `Char`, `TraitObject`, etc.).
   - Heterogeneous: project each element to `Arc<HeapValue>` via
     `slot_to_heap_arc`, then call
     `TypedArrayData::build_specialized_from_heap_arcs(elems)`. If
     `build_specialized_from_heap_arcs` returns `Err`, surface that as
     `VMError::RuntimeError` — heterogeneous-arm arrays were rejected
     at the value-tier per Q25.A; the right disposition is a user-facing
     RuntimeError, not `NotImplemented(SURFACE)`.

This is the same shape `op_new_typed_array` at
`object_creation.rs:339-625` already uses for the homogeneous case; the
audit's fix-shape extends that handler's logic to `op_new_array`'s
declared-untyped semantics by adding the `slot_to_heap_arc` +
`build_specialized_from_heap_arcs` fallback for cross-kind input.

### Empty-array shape

`op_new_array(Count(0))` is currently emitted by 8 of the 9 compile
sites (every site except `compile_expr_array`'s non-empty literal path).
The user-visible contract for an empty `[]` is "an array whose element
type will be determined by subsequent push / element-store / map
result". `op_new_typed_array` already documents at `object_creation.rs:336-338`
that "Empty arrays default to `TypedArrayData::I64` (an arbitrary but
stable choice; the compiler is responsible for emitting the
kind-specific `NewTypedArray{I64,F64,Bool}` opcodes when the element
type is known at compile time)".

The same "arbitrary but stable" default applies to `op_new_array`. The
choice should mirror `op_new_typed_array`'s I64 default rather than
`build_specialized_from_heap_arcs`'s TypedObject default — because the
empty-array consumers in question (`Vec.map`'s `let mut result = []`,
list comprehensions' `__comp_result`, spread accumulators'
`__array_result`, annotation context's `event_log`) overwhelmingly
push **inline-scalar** elements first. Defaulting empty to
`TypedArrayData::I64` matches the stable-default discipline `__comp_result`
/ `__array_result` already document via `op_new_typed_array`'s
sibling-handler default.

**Caveat:** the empty default is the WEAKEST point in the fix-shape.
Today's `builtin_push` at `executor/builtins/array_ops.rs:184-262`
rejects type-mismatched pushes — push int into a Bool-arm array errors
out. So an empty `[]` defaulting to `I64` and then receiving a
`.push("a")` would surface "push() value kind must match array element
kind (int)". This is acceptable behavior per CLAUDE.md "No runtime
coercion" — but it changes the user-visible failure mode for the
"empty literal then push different type" pattern. The fix-shape's
§4.B sub-section discusses two alternatives and recommends the I64
default at landing, with the alternative left as a follow-up amendment
if user code surfaces the regression.

### Awkward-hyphenated descriptor

The error wording itself —
`"the-deleted-heterogeneous-element-carrier"` — is awkward enough to be
worth a callout for the supervisor. It reads as a workaround for a
broader-family deletion-attractor regex (`(decode|tag|kind|dispatch|...)
(bridge|probe|helper|hop|translator|adapter|shim)` per CLAUDE.md). The
hyphenated form avoids those words but still describes deleted code by
hypothetical-role-with-dashes, which is the same defection-attractor
class.

The audit recommends rewriting all 41 hits across 18 files (comments
+ error messages) to use the **value-tier name** for clarity:
`TypedArrayData::HeapValue (deleted by §2.7.24 Q25.A)`. That matches
the canonical naming at `crates/shape-value/src/executor/objects/property_access.rs:682`
("`the-deleted-heterogeneous-element-carrier (deleted by §2.7.24 Q25.A)`")
where the deletion-fate already names the section.

This rewrite is **textual mechanical cleanup** independent of the
runtime fix; should it grow into its own scope it can land as a
separate commit. Recommendation: bundle into the §4 fix-commit since
the `op_new_array` body's docstring and error message are being
rewritten anyway.

## §4. Fix-shape proposal

### §4.A `op_new_array` body rewrite

Rewrite `crates/shape-vm/src/executor/objects/object_creation.rs:287-324`
to consume the popped kinded slots and dispatch them to a
`TypedArrayData::*` variant:

```rust
pub(in crate::executor) fn op_new_array(
    &mut self,
    instruction: &Instruction,
) -> Result<(), VMError> {
    let count = match instruction.operand {
        Some(Operand::Count(c)) => c as usize,
        _ => return Err(VMError::InvalidOperand),
    };

    // Pop in reverse-push order, then reverse to recover source order.
    let mut popped: Vec<(u64, NativeKind)> = Vec::with_capacity(count);
    for _ in 0..count {
        match self.pop_kinded() {
            Ok(pair) => popped.push(pair),
            Err(e) => {
                for (b, k) in popped.drain(..) { drop_with_kind(b, k); }
                return Err(e);
            }
        }
    }
    popped.reverse();

    // Empty-array default. ADR-006 §2.7.24 Q25.A: per-variant uniform
    // element kind. Empty has no element kind; pick I64 to match the
    // existing op_new_typed_array stable default (object_creation.rs:339-373).
    if popped.is_empty() {
        let buf = TypedBuffer::<i64>::from_vec(Vec::new());
        let arr = Arc::new(TypedArrayData::I64(Arc::new(buf)));
        let bits = Arc::into_raw(arr) as u64;
        return self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray));
    }

    // Homogeneous-kind fast paths. Each arm constructs the matching
    // TypedArrayData specialization directly without an Arc<HeapValue>
    // round-trip.
    let first_kind = popped[0].1;
    let all_match = popped.iter().all(|(_, k)| *k == first_kind);

    if all_match {
        // Reuse op_new_typed_array's homogeneous arms for Int64 / Float64 /
        // Bool / String. The body is identical from this point — extract
        // the per-kind switch into a shared helper
        // `build_typed_array_from_homogeneous(popped, kind)` in
        // crates/shape-vm/src/executor/objects/array_ctor_helpers.rs (new
        // file or inline next to op_new_typed_array). Or call into
        // op_new_typed_array directly via instruction-rewrite — but the
        // shared helper is cleaner because callers handle drop_with_kind
        // sequencing differently on error paths.
        //
        // Heap-kinded homogeneous arms (Decimal / BigInt / TypedObject /
        // Char / DateTime / Timespan / Duration / Instant / TraitObject)
        // mirror Array.filled's heap-element handling at
        // executor/builtins/array_ops.rs:430-491 — each arm extracts the
        // typed Arc share via slot.as_heap_value() + HeapValue::*
        // pattern-match and clones into a TypedBuffer<Arc<T>>.
        let arr = build_typed_array_from_homogeneous(&popped, first_kind)?;
        let bits = Arc::into_raw(Arc::new(arr)) as u64;
        return self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray));
    }

    // Heterogeneous-kind path. ADR-006 §2.7.24 Q25.A explicitly rejects
    // polymorphic catch-all element buffers, but ALLOWS heterogeneous-
    // arm inputs through the Arc<HeapValue> projection helper —
    // build_specialized_from_heap_arcs requires uniform HeapValue arms
    // and surfaces Err on cross-arm input. The user-visible disposition
    // for that Err is RuntimeError (per Q25.A "Arrays do not [admit
    // heterogeneous slots]"), NOT NotImplemented(SURFACE).
    let mut elems: Vec<Arc<HeapValue>> = Vec::with_capacity(popped.len());
    for (bits, kind) in popped.iter() {
        let slot = KindedSlot::new(ValueSlot::from_raw(*bits), *kind);
        let arc = slot_to_heap_arc(&slot)?; // crates/shape-vm/src/executor/builtins/array_ops.rs:49
        std::mem::forget(slot); // ownership transfers into arc via per-kind clone semantics
        elems.push(arc);
    }
    // Release each popped slot's share — slot_to_heap_arc cloned the
    // underlying Arc shares it needed; the popped bits' shares are
    // independent and must be retired.
    for (bits, kind) in popped.drain(..) {
        drop_with_kind(bits, kind);
    }
    let arr = TypedArrayData::build_specialized_from_heap_arcs(elems)
        .map_err(VMError::RuntimeError)?;
    let bits = Arc::into_raw(Arc::new(arr)) as u64;
    self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedArray))
}
```

### §4.B Empty-default alternatives

Two empty-default options were considered:

- **Option α (recommended): empty → `TypedArrayData::I64`.** Matches
  `op_new_typed_array`'s existing stable default (`object_creation.rs:366-373`).
  Documented as "an arbitrary but stable choice; the compiler is
  responsible for emitting the kind-specific `NewTypedArray*` opcodes
  when the element type is known at compile time". Consumers like
  `Vec.map`'s `let mut result = []; result.push(int)` work end-to-end
  (push int matches I64 arm). Consumers like
  `let mut result = []; result.push("a")` would surface the existing
  `builtin_push` "kind mismatch" RuntimeError at the first push — same
  behavior as today's typed empty default.
- **Option β: empty → `TypedArrayData::TypedObject` with empty buffer.**
  Matches `build_specialized_from_heap_arcs`'s empty default
  (`heap_value.rs:2941-2944`). Would accept any heap-kinded first push
  without the I64-default kind-mismatch surface — but would surface
  `push int into Vec<object>` as a kind-mismatch on inline-Int64 first
  push. **Same number of broken cases, just shifted from inline-scalar
  to heap-kinded.** Net wash; α wins on consistency with `op_new_typed_array`.

The fix-shape adopts Option α at landing. If a future user-code regression
surfaces the I64-default kind-mismatch as a real failure mode, that
ADR-amendment territory (kind-polymorphic empty array via lazy
specialization-on-first-push) is its own follow-up.

### §4.C Knock-on sites with the same forbidden-pattern descriptor

The `the-deleted-heterogeneous-element-carrier` deletion-fate descriptor
appears at 41 sites across 18 files. Most are comment / docstring
references that are mechanical to rewrite. Five are runtime error
messages:

- `executor/objects/object_creation.rs:316-320` (`op_new_array` — fixed
  by §4.A).
- `executor/objects/object_creation.rs:386-391` (`op_new_typed_array`'s
  heterogeneous arm — should remain a SURFACE but with the correct §-cite
  and rewritten descriptor).
- `executor/objects/property_access.rs:695, 725` (property-access path
  with heterogeneous-element receiver — out-of-scope SURFACE, fix §-cite
  + descriptor only).
- `executor/objects/array_aggregation.rs:310-360` (`sum`/`min`/`max` on
  Decimal/BigInt arrays — out-of-scope SURFACE, fix §-cite + descriptor
  only).

The fix-commit bundles the §4.A `op_new_array` body rewrite with the
mechanical §-cite + descriptor cleanup across all 41 sites (line-count
~120, no behavior change at the SURFACE sites — they remain SURFACE,
just cite §2.7.24 / Q25.A correctly).

### §4.D Scope estimate

- **`op_new_array` body rewrite:** ~80 LoC inside
  `executor/objects/object_creation.rs`, plus ~50 LoC helper
  (`build_typed_array_from_homogeneous`) extracted from the existing
  `op_new_typed_array` body.
- **§-cite + descriptor mechanical cleanup:** ~40 hits in comments
  (file + line below), ~5 hits in runtime error strings, ~1 hit in the
  AGENTS.md row's "deleted-carrier" reference (cosmetic). ~120 LoC
  total, ~18 files touched.
- **New unit tests** in `object_creation.rs#tests`: 4-6 cases —
  homogeneous Int64 / Float64 / String / heterogeneous (heap-kinded
  TypedObject) / empty / kind-mismatch (heterogeneous inline arms).
- **Total scope:** ~250 LoC across ~3-5 files (object_creation.rs +
  the §-cite descriptor pass + tests). Comfortably within a single-round
  budget. **NO ADR amendment territory** — every cite already exists in
  §2.7.24 Q25.A + §2.7.5; the helper functions already exist
  (`build_specialized_from_heap_arcs`, `slot_to_heap_arc`).

## §5. Phase 2d hardening tracking

Verified no existing hardening-stack entry for `op_new_array` in
`docs/cluster-audits/phase-2d-hardening.md` (items (a)–(i)). The Round
11A surface is NEW — first reported in the post-Round-9 kickoff-smokes
verification at `docs/cluster-audits/phase-3-cluster-0-status.md:1497-1561`
(2026-05-13 entry "Kickoff Smoke 2 + 3 verification (post-Round-9
merge)"). The status doc surfaces it for supervisor disposition:

> **Smoke 2 VM-side blocker** (`op_new_array` Phase 2c reentry, ADR-006
> §2.7.4) — surfaced for supervisor disposition. … Either (a) cluster-0
> absorbs the VM-side fix, or (b) cluster-0 closes with VM-side blocker
> documented as out-of-scope.

Supervisor's Round-11 ruling (status doc lines 1551-1553 + the dispatch
text for this sub-cluster) ratifies option (a): cluster-0's "VM == JIT
for all 4 kickoff smokes" close criterion holds; the VM-side block
folds into Round 11A. This audit's deliverable matches that scope.

## §6. Coordination with Round 11D + trinity

- **11D (W17-mir-mutation-writeback)** territory:
  `crates/shape-vm/src/mir/lowering/expr.rs::Expr::MethodCall`. The
  audit verifies zero overlap — `op_new_array`'s body is in
  `executor/objects/object_creation.rs`, the MIR builder lives in a
  different module tree.
- **Trinity (W12-jit-producing-site-conduit-completeness)** territory:
  `crates/shape-value/src/v2/concrete_type.rs` +
  `crates/shape-jit/src/mir_compiler/{types,rvalues,terminators}.rs`.
  Zero overlap — Round 11A is VM-side bytecode-handler territory; the
  trinity is JIT-side conduit territory.
- **Crate boundary check:** §4.A's body uses
  `TypedArrayData::build_specialized_from_heap_arcs` (in
  `crates/shape-value/`) and `slot_to_heap_arc` (in
  `crates/shape-vm/src/executor/builtins/array_ops.rs`). The
  `slot_to_heap_arc` symbol is currently `fn` (file-local). The fix
  proposes promoting it to `pub(in crate::executor)` so the new
  `op_new_array` body can call it without duplicating the helper. **No
  cross-crate API additions; no public-API surface changes.** Same crate
  boundary discipline `result_slot_to_heap_value_arc` already preserves
  for `hashmap_methods.rs`.

## §7. Forbidden frames refused on sight

Per CLAUDE.md + Round 11A dispatch:

- **"preserve deleted-carrier emit path under documented disposition"**
  — refused. The `TypedArrayData::HeapValue` arm is permanently deleted
  per ADR-006 §2.7.24 Q25.A + §Q25.E #1. No revival under any rename.
- **Bool-default element kind for unknown-kind array** — refused. The
  Smoke 2 receiver path proves kind at construction; there is no
  Bool-default needed. The empty-array default is `TypedArrayData::I64`,
  not Bool.
- **"just one edge case" / "soft-fail counter for now"** — refused. The
  fix-shape covers homogeneous + heterogeneous + empty cases via the
  existing helpers; no soft-fail surface remains.
- **"this is Phase 2c-residual, document as out-of-scope"** — refused
  per supervisor's Round-11 ratification (status doc:1551-1553). The
  cluster-0 "VM == JIT for all 4 kickoff smokes" close criterion means
  VM-side Smoke 2 stalls cluster-0 close regardless of architectural
  layer.
- **Add a transitional `TypedArrayData::HeapValueShim` / `Untyped` /
  `Mixed` / `Generic` variant** — refused. §2.7.24 Q25.A Q25.E lists
  these renames in the forbidden-pattern grid; the value tier has zero
  such variants and the fix must not introduce one.
- **"Defer to a new ADR amendment introducing dynamic-typed empty
  arrays"** — refused at the empty-array shape level. §4.A's I64
  default matches `op_new_typed_array`'s precedent. A future amendment
  introducing flow-sensitive empty-array kind inference would be its
  own scope and is not required for Smoke 2 close.

## §8. ADR amendment surface

**No ADR amendment is required.** Every architectural decision the
fix-shape relies on already lives in §2.7.24 Q25.A + §2.7.5 +
§2.7.10/Q11. The runtime helpers already exist
(`build_specialized_from_heap_arcs`, `slot_to_heap_arc`,
`result_slot_to_heap_value_arc`); the `TypedArrayData` variant grid
already covers every kind the bytecode emit path can produce. The fix
is **bounded mechanical work** within the audit-doc-defined scope.

If during code-fix-side implementation a new shape surfaces
(e.g. a heap-kinded element whose `HeapValue` arm has no matching
`TypedArrayData::*` variant), surface-and-stop and return to supervisor.
No such gap is visible from the audit's static analysis — the Q25.A
variant grid (String / Decimal / BigInt / DateTime / Timespan /
Duration / Instant / Char / TypedObject / TraitObject) covers every
`HeapValue` arm a user can construct from inline / heap kinds at the
`op_new_array` emit site.

## Disposition

**Audit-then-migrate workflow proceeds to migrate.** §4's fix-shape is
bounded mechanical work (~250 LoC) within a single-round budget. No
ADR amendment surface, no sub-cluster split surface. Audit-only close
not required.

Next steps after this audit doc commits:
1. Implement §4.A `op_new_array` body rewrite.
2. Implement §4.C §-cite + descriptor mechanical cleanup.
3. Add §4.D unit tests.
4. Run kickoff Smoke 2 VM-side (with VM-side `IntrinsicSum` still
   surfacing — Smoke 2 end-to-end VM-side `30` output remains blocked
   on the `IntrinsicSum` wave-5d gap, **NOT this sub-cluster's
   territory**). Surface the `IntrinsicSum` block as a follow-up
   workstream for supervisor disposition.
5. Close gates: `cargo check --workspace --lib --tests`,
   `cargo test -p shape-vm --lib` (no regressions from baseline),
   `cargo test -p shape-jit --lib` (no regressions from baseline 361),
   `bash scripts/verify-merge.sh` 12/12, `bash scripts/check-no-dynamic.sh`
   EXIT=0.

## Key file references

- **Surface site:** `crates/shape-vm/src/executor/objects/object_creation.rs:304-324`
  (`op_new_array` body); `:315-320` (error message text).
- **Dispatcher:** `crates/shape-vm/src/executor/objects/mod.rs:208`
  (`NewArray => self.op_new_array(...)`).
- **OpCode def:** `crates/shape-vm/src/bytecode/opcode_defs.rs:34,56`
  (variable-arity opcode definition).
- **Compile emit sites:** 9 sites in `crates/shape-vm/src/compiler/`
  (full list in §1 above).
- **Deleted-carrier tomb:** `crates/shape-value/src/heap_value.rs:2912-2917`
  (TypedArrayData enum's deletion-fate comment).
- **Construction helper:** `crates/shape-value/src/heap_value.rs:2937-3023`
  (`build_specialized_from_heap_arcs`).
- **Projection helper:** `crates/shape-vm/src/executor/builtins/array_ops.rs:49-86`
  (`slot_to_heap_arc`).
- **Sibling handler:** `crates/shape-vm/src/executor/objects/object_creation.rs:339-625`
  (`op_new_typed_array` — homogeneous-kind dispatch template).
- **ADR §2.7.24 Q25.A:** `docs/adr/006-value-and-memory-model.md:4465-4769`
  (typed-carrier monomorphization bundle).
- **ADR §2.7.5:** `docs/adr/006-value-and-memory-model.md:450-491`
  (cross-crate ABI policy — producing-site classification).
- **Round 11A dispatch context:** `docs/cluster-audits/phase-3-cluster-0-status.md:1497-1768`
  (kickoff-smokes verification + Round 11A scope ratification).
