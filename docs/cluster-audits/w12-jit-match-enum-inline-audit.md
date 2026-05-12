# W12-jit-match-enum-inline — audit

**Sub-cluster:** Phase 3 cluster-0 Round 6B (audit-first).
**Branch:** `bulldozer-strictly-typed-w12-jit-match-enum-inline` (parent `0484db12`).
**Audited:** 2026-05-12.
**Cross-cluster siblings:** Round 6A (`W12-jit-call-return-kind-track`) and 6C (`W12-collection-constructor-mir-lowering`).

This audit closes Round 5B's surfaced item (iii) #2 (JIT match-on-enum inline codegen) — one of the three architectural needs the W12-jit-aggregate-non-array close (`d3ea6546`) surfaced as ADR-amendment-level co-design.

---

## §1. Reproduction (current state at parent `0484db12`)

Smoke 1.5 (from dispatch):

```shape
fn divide(a: int, b: int) -> Result<int, string> {
  if b == 0 { return Err("div by zero") }
  return Ok(a / b)
}
let r = divide(10, 2)
match r {
  Ok(v) => print(v),
  Err(e) => print(e),
}
```

```
$ shape run /tmp/smoke_1_5.shape --mode vm    → 5
$ shape run /tmp/smoke_1_5.shape --mode jit   → Error: Runtime error: JIT execution error (code: -1)
```

The `code: -1` is the W12-jit-linker-resolve stub-fallback deopt signal. Under
`SHAPE_JIT_DEBUG=1` the compile failure trail surfaces:

```
[jit-mir] compile failed for 'divide': EnumStore: SURFACE — variant 'Err'
   (operands.len()=1) requires a co-designed Call-return kind track + pattern-
   match codegen + NaN-box↔Arc conversion that exceeds this sub-cluster's
   scope. Conduit extension (option (ii)) is landed; this consumer is option
   (iii) territory per W12-jit-aggregate-non-array audit
   `docs/cluster-audits/w12-jit-aggregate-non-array-audit.md` §4.
   ADR-006 §2.7.14 / §2.7.5.
```

So `divide` itself fails to JIT-compile. Its stub returns -1. The match on `r`
is never reached.

Smoke 2 fails identically — `first_positive`'s body surfaces `EnumStore` for
`Some(x)` and `None`'s `MirConstant::None` path falls through.

---

## §2. The MIR shape produced by `match` on `Result<T,E>` / `Option<T>`

For `match r { Ok(v) => print(v), Err(e) => print(e) }`, the MIR lowering
(`crates/shape-vm/src/mir/lowering/expr.rs::lower_match_expr`,
`expr.rs::lower_match_pattern_condition_operand`) emits:

```text
scrutinee_slot = Use(Copy(r))          # the Result<i64,string> value
arm 0 (Ok(v)):
  push_scope
  # Bindings BEFORE the test:
  v_slot = Use(Copy(Place::Index(scrutinee_place, 0)))
  finish_block: SwitchBool {
    operand: Copy(scrutinee_slot),     # <-- the scrutinee itself
    true_bb: body_block,
    false_bb: next_block,
  }
  body_block:
    # uses v_slot in print(v)
    ...
  pop_scope
arm 1 (Err(e)):
  # same structural shape
```

Two structural problems for JIT consumption:

1. **`SwitchBool { operand: Copy(scrutinee) }`** — the operand is the scrutinee
   itself, NOT a Bool. The JIT's `SwitchBool` handler in
   `crates/shape-jit/src/mir_compiler/terminators.rs:26-95` falls through to
   the I64 truthy-check branch (lines 56-89): NaN-box `not_null && not_none
   && not_false && not_zero`. For a real `Arc<ResultData>` pointer this is
   always true (the pointer is non-zero, not TAG_NULL, not TAG_NONE,
   not TAG_FALSE). Every arm takes the first branch.

2. **`Place::Index(scrutinee, 0)` binding** — the binding `v` for `Ok(v) =>`
   reads the scrutinee at index 0. The JIT compiles `Place::Index` as
   `inline_array_get(base, index)` (`crates/shape-jit/src/mir_compiler/
   places.rs:909-936`), which dereferences the scrutinee as a v2 typed-array
   header. The scrutinee is `Arc<ResultData>`, not an array. The read
   returns garbage (whatever byte 0 of `ResultData` is — the `is_ok` byte
   widened to I64).

3. **Bindings emitted BEFORE the test fires** — even if (1) and (2) worked,
   the binding is emitted in the test-block prior to `SwitchBool`. The
   `Err(e) => print(e)` arm's `e` binding is emitted with the SAME scrutinee
   projection — there's no MIR-level "this binding only fires in the
   matching arm". The current shape relies on the next-arm test re-emitting
   the bindings within its own block (`lower_match_expr` does this — each
   arm gets its own scope and own `lower_pattern_bindings_from_place` call
   — so the bindings ARE per-arm in MIR position; the issue is the
   semantics of `Place::Index(scrutinee, 0)` for both arms).

---

## §3. The VM-side reference path (bytecode, not MIR)

The VM does NOT execute MIR. Its bytecode compiler emits a different shape
at `crates/shape-vm/src/compiler/expressions/advanced.rs::compile_expr_match`
that calls `compile_pattern_check` (`compiler/patterns/checking.rs:71-336`).
For `Ok(v)` / `Err(e)`:

```
LoadLocal scrutinee
IsOk                         # OpCode — reads is_ok from Arc<ResultData>
JumpIfFalse next_arm
LoadLocal scrutinee
UnwrapOk                     # OpCode — extracts payload bits + kind
StoreLocal inner_slot
... (recurse with inner_slot)
```

For `Some(v)` / `None`:

```
LoadLocal scrutinee
IsNull                       # OpCode — recognises None-as-null sentinel
                             #   AND Some(_) recognised via "not null"
                             # (note: legacy null-coded Option still flows
                             # through this path per
                             # `is_null_sentinel` in `exceptions/mod.rs:961`)
JumpIfTrue (or JumpIfFalse)
```

Both reference paths (`OpCode::IsOk`, `OpCode::UnwrapOk`, `OpCode::IsNull`)
operate at the runtime layer on the strict-typed `Arc<ResultData>` /
`Arc<OptionData>` representation per ADR-006 §2.7.17 / Q18 (Wave 14
W14-variant-codegen). The JIT must produce equivalent code reading the
same in-memory shape.

---

## §4. Producer-side shape mismatch (cross-cluster)

`divide`'s `Ok(a/b)` lowers to `EnumStore { variant_name: Some("Ok"),
operands: [a/b] }`. The JIT consumer at
`crates/shape-jit/src/mir_compiler/statements.rs:203-257` surfaces-and-stops
because:

1. **No producer for `Arc<ResultData>` at JIT compile time.** The existing
   `jit_make_ok` / `_err` / `_some` FFI in `ffi/result.rs:21-137` returns
   the JIT-internal NaN-box `unified_box(HK_OK, Box::into_raw(UnifiedValue<u64>))`
   shape — NOT `Arc::into_raw(Arc<ResultData>)`. Wiring the EnumStore consumer
   to these would put a NaN-box bit-pattern into a slot that downstream
   consumers (top-level `match r`, VM trampoline boundary) interpret as
   `Arc<ResultData>`. The two shapes are not interconvertible without an
   FFI-boundary converter — exactly the W12-jit-aggregate-non-array §4.4
   item 3 round-trip audit gap.

2. **The VM-side `BuiltinFunction::OkCtor` produces `Arc::new(ResultData::ok(payload))`**
   in `executor/vm_impl/builtins.rs:551-568` and stores via
   `KindedSlot::from_result` (kind = `Ptr(HeapKind::Result)`, bits =
   `Arc::into_raw(Arc<ResultData>) as u64`). The JIT must emit a producer
   that returns the SAME shape — a new `jit_v2_make_result_ok` /
   `jit_v2_make_result_err` / `jit_v2_make_option_some` FFI that allocates
   `Arc<ResultData>` / `Arc<OptionData>` and returns the raw `Arc::into_raw`
   bits.

The audit doc for W12-jit-aggregate-non-array (`§4.4`) identified all three
needs (Call-return kind, match codegen, ABI audit). 6A territory closes the
first; this audit (6B) addresses the second. The third — a `jit_v2_make_result_*`
family that allocates `Arc<ResultData>` instead of NaN-box `UnifiedValue<u64>`
— remains. Without it, even with my fix landed, `divide` cannot produce a
JIT-compatible Result value.

---

## §5. Audit decision

The audit reveals that the close criterion ("Smoke 1.5 + Smoke 2 pass under
both VM and JIT identically") requires **all three of the W12-jit-aggregate-non-array
§4.4 co-design items** to land in lockstep:

| # | Item | Territory | Status |
|---|---|---|---|
| 1 | Call-terminator return-kind track (so `let r = divide(...)` knows `r`'s kind = `Ptr(HeapKind::Result)`) | 6A `W12-jit-call-return-kind-track` | dispatching in parallel |
| 2 | JIT match-on-enum inline codegen (this sub-cluster's territory) | 6B (this audit) | option (iii) — depends on #3 |
| 3 | `jit_v2_make_result_*` family producing `Arc<ResultData>`-shape bits at the EnumStore site | Round 5B §4.4 item 3 (NaN-box↔Arc audit) | NOT in any current cluster |

My match-on-enum codegen depends on #3 because: my codegen reads from
`*const ResultData` (per §3's reference path), but the EnumStore producer
currently emits NaN-box bits. The two MUST agree on the slot's in-memory
shape OR the consumer must classify by kind and dispatch (which requires
both producer paths to be present, plus a runtime-tier converter at the
boundary).

The honest assessment: **#2 cannot land green without #3.** Landing my
codegen against `Arc<ResultData>` while the producer emits NaN-box bits
will segfault the match path; landing my codegen against the NaN-box shape
while the VM trampoline returns `Arc<ResultData>` bits will fail the top-
level match path. Either way, end-to-end smokes don't pass.

Per CLAUDE.md surface-and-stop discipline, this audit surfaces the gap to
the supervisor for cluster-1 (or a Round-7 dispatch) to co-design items
#2 and #3 together. **The audit's landed deliverable is the MIR-shape and
JIT-codegen blueprint below, not a forbidden-pattern stopgap.**

### §5.1 Why not land "just option (iii) #2" against the NaN-box producer

The NaN-box `unified_box(HK_OK, ...)` shape is the deleted ValueWord-shape
path — its consumers (`is_ok_tag`, `is_err_tag`, `is_some_tag`) gate on
`is_tagged(bits) -> is_heap(bits)`, which returns `false` for raw Arc
pointers (the W12-vw-tests-rewrite close `63d00c98` documents 9 deleted
tests asserting this exact roundtrip). Wiring my match codegen against
the NaN-box shape would:

- Re-enable the deleted tag-bit dispatch surface for Result/Option pattern
  matching (CLAUDE.md "Forbidden code" — runtime `tag_bits` dispatch deleted
  with the W-series).
- Force the VM↔JIT boundary to convert between `Arc<ResultData>` and
  `unified_box(HK_OK, …)` at every trampoline call — a "tag-decode bridge"
  / "Arc-to-NaN-box translator" defection-attractor (CLAUDE.md broader-family
  regex on `(decode|tag|kind|dispatch|...) (bridge|probe|helper|hop|
  translator|adapter|shim)` extends to the Arc↔NaN-box conversion).

Refused on sight per §0 forbidden-pattern discipline.

### §5.2 Why not land "just option (iii) #2" against an Arc<ResultData>
producer that doesn't yet exist

The audit could add a producer-side `jit_v2_make_result_*` family without
the supervisor authorizing the broader §4.4 co-design — i.e. take territory
that 6B doesn't formally cover. But:

- The dispatch's "territory overlap warning" between 6A and 6B already
  flags coordination as load-bearing.
- The NaN-box↔Arc round-trip audit (#3) is a separate ADR-level item; the
  W12-jit-aggregate-non-array audit explicitly classified it as "untested
  end-to-end". Doing the audit without the supervisor's authorization is
  scope creep — the same antipattern the W11 walk-back exhibited (see
  phase-3-cluster-0-status §"Round 1 process notes").
- Per Round 5B audit §4.5, "future agent picking up the (iii) work should
  start there" — implying co-design dispatch, not 6B unilaterally expanding.

So 6B surfaces the dependency and lands the MIR/JIT blueprint as the
authorisation-ready spec for whichever sub-cluster co-designs #2 + #3.

---

## §6. The MIR-shape change required (blueprint for #2)

To unblock the match-on-enum codegen, the MIR needs to carry two new
operand shapes — both are bounded additions to existing `Rvalue` shapes, no
new statement kinds, no new HeapKind, no ADR amendment beyond §2.7.5 producing-
site classification:

### §6.1 `Rvalue::EnumTest { operand: Operand, variant: VariantTag }`

```rust
/// Test whether a Result/Option scrutinee matches a specific variant.
/// Result type: Bool.
EnumTest {
    operand: Operand,
    variant: VariantTag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariantTag {
    Ok,
    Err,
    Some_,
    None_,
}
```

Producer site: `lower_match_pattern_condition_operand` arm for
`Pattern::Constructor { variant: "Ok" | "Err" | "Some" | "None", ... }`
(see `crates/shape-vm/src/mir/lowering/expr.rs:1242-1244` — currently emits
`Operand::Copy(scrutinee_slot)`). Replace with:

```rust
let bool_slot = builder.alloc_temp(LocalTypeInfo::Copy);
builder.push_stmt(
    StatementKind::Assign(
        Place::Local(bool_slot),
        Rvalue::EnumTest {
            operand: Operand::Copy(Place::Local(scrutinee_slot)),
            variant: VariantTag::Ok, // per pattern.variant
        },
    ),
    pattern_span,
);
Some(Operand::Copy(Place::Local(bool_slot)))
```

JIT consumer (`crates/shape-jit/src/mir_compiler/rvalues.rs`):

```rust
Rvalue::EnumTest { operand, variant } => {
    let bits = self.compile_operand_raw(operand)?;
    let func_ref = match variant {
        VariantTag::Ok    => self.ffi.arc_result_is_ok,
        VariantTag::Err   => self.ffi.arc_result_is_err,
        VariantTag::Some_ => self.ffi.arc_option_is_some,
        VariantTag::None_ => self.ffi.arc_option_is_none,
    };
    let inst = self.builder.ins().call(func_ref, &[bits]);
    Ok(self.builder.inst_results(inst)[0])  // returns I8 bool
}
```

### §6.2 `Rvalue::EnumPayload { operand: Operand, variant: VariantTag }`

```rust
/// Extract the payload from a Result/Option scrutinee. Caller must have
/// already proven the variant via `EnumTest`. Result type: U64 (raw bits;
/// kind is `payload.kind` which 6A's call-return-kind track threads
/// separately).
EnumPayload {
    operand: Operand,
    variant: VariantTag,
}
```

Producer site: `lower_constructor_bindings_from_place_opt`
(`crates/shape-vm/src/mir/lowering/stmt.rs:490-526`). Replace the
`projected_index_place` call with:

```rust
let payload_slot = builder.alloc_temp(...);
builder.push_stmt(
    StatementKind::Assign(
        Place::Local(payload_slot),
        Rvalue::EnumPayload {
            operand: Operand::Copy(scrutinee_place.clone()),
            variant: ...,
        },
    ),
    span,
);
// then bind inner pattern to `payload_slot`
lower_pattern_bindings_from_place_opt(builder, inner_pattern, Some(&Place::Local(payload_slot)), ...);
```

JIT consumer:

```rust
Rvalue::EnumPayload { operand, variant } => {
    let bits = self.compile_operand_raw(operand)?;
    let func_ref = match variant {
        VariantTag::Ok | VariantTag::Err   => self.ffi.arc_result_payload,
        VariantTag::Some_                   => self.ffi.arc_option_payload,
        VariantTag::None_                   => {
            return Err("EnumPayload on None: no payload to extract")
        }
    };
    let inst = self.builder.ins().call(func_ref, &[bits]);
    Ok(self.builder.inst_results(inst)[0])
}
```

### §6.3 FFI entry points (`crates/shape-jit/src/ffi/result.rs` additions)

```rust
/// Read `is_ok` from an Arc<ResultData> pointer.
/// SAFETY: `bits` must be `Arc::into_raw(Arc<ResultData>) as u64` from the
/// VM-side `KindedSlot::from_result` producer (or 6B's jit_v2_make_result_*).
/// The caller's slot owns one strong-count share; this read borrows.
pub extern "C" fn jit_arc_result_is_ok(bits: u64) -> u8 {
    if bits == 0 { return 0; }
    let r: &shape_value::ResultData = unsafe { &*(bits as *const _) };
    if r.is_ok { 1 } else { 0 }
}

pub extern "C" fn jit_arc_result_is_err(bits: u64) -> u8 {
    if bits == 0 { return 0; }
    let r: &shape_value::ResultData = unsafe { &*(bits as *const _) };
    if !r.is_ok { 1 } else { 0 }
}

/// Read the payload's raw bits from an Arc<ResultData> pointer.
/// CALLER MUST retain the payload's strong-count share (clone via
/// `clone_with_kind`) before using it as a JIT-owned value — the borrow
/// here doesn't bump the inner share.
pub extern "C" fn jit_arc_result_payload(bits: u64) -> u64 {
    if bits == 0 { return 0; }
    let r: &shape_value::ResultData = unsafe { &*(bits as *const _) };
    // clone the payload share so the returned bits are an owned slot.
    let payload = r.payload.clone();
    let raw = payload.slot.raw();
    std::mem::forget(payload);  // transfer share to caller
    raw
}

pub extern "C" fn jit_arc_option_is_some(bits: u64) -> u8 { ... }
pub extern "C" fn jit_arc_option_is_none(bits: u64) -> u8 { ... }
pub extern "C" fn jit_arc_option_payload(bits: u64) -> u64 { ... }
```

The kind companion for the payload (per ADR-006 §2.7.5 §2.7.7 / Q9 parallel-
kind track) flows out of band — the JIT-side caller must stamp the
payload's kind into the parallel-kind track at the destination slot. This
is where 6A's Call-return-kind work intersects: the kind of `payload` was
classified at the producer site (`Ok(a/b)` — kind of `a/b` is `Int64`)
and threaded through to the slot during EnumStore consumption. The MIR
inference pass would need a tweak so `Place::Local(payload_slot)` after
`EnumPayload` inherits the payload kind from the producer's EnumStore
classification.

### §6.4 Exhaustive match site updates

Adding `Rvalue::EnumTest` and `Rvalue::EnumPayload` requires updating
~10 exhaustive match sites on `Rvalue`:

```
crates/shape-vm/src/mir/return_ownership.rs:141, 293
crates/shape-vm/src/mir/field_analysis.rs:164
crates/shape-vm/src/mir/liveness.rs:162
crates/shape-vm/src/mir/solver.rs:596, 674, 694, 711, 733, 750, 772, 1505, 1540
crates/shape-jit/src/mir_compiler/rvalues.rs (the new arms)
```

For most consumers (solver, liveness, field-analysis), the new variants
treat the operand the same as `Rvalue::Use(Operand::Copy(...))` — borrow-
check it the same way, no fields read, no new aliasing. Return-ownership-
mode is `Owned` (the payload's share transfers).

### §6.5 Pre-existing MIR shape problem (binding emission order)

Per §2 #3, the current MIR-lowering emits bindings into the body block
correctly — `lower_match_expr` per-arm scopes per-arm bindings. The
audit's initial reading of "bindings BEFORE the test" was wrong — they're
in the arm-body block, AFTER the SwitchBool true_bb arrives there.
However, the bindings ARE emitted even when the arm doesn't match — both
arms' bindings reference the same `Place::Index(scrutinee, 0)` projection,
and reading byte 0 of `ResultData` returns the `is_ok` byte regardless of
which variant — so the Err arm gets `e = 0 or 1` (the is_ok byte
interpreted as int).

The proposed `EnumPayload` rvalue addresses this because each arm's
binding emits its own `EnumPayload { variant: ... }` rvalue — the JIT
codegen for `EnumPayload { variant: Ok }` only reads when `is_ok = true`,
and the surrounding MIR control flow already guarantees the arm is only
entered after the matching `EnumTest` returned true. So the variant tag
on `EnumPayload` is a producer-side §2.7.5 classification, not a runtime
check.

---

## §7. Sites surfaced for cite-tracked follow-up

| # | Item | §-cite | Disposition |
|---|---|---|---|
| 1 | NaN-box↔Arc round-trip audit (W12-jit-aggregate-non-array §4.4 item 3) | §2.7.5 / §2.7.14 | **load-bearing dependency** — must land before #2 (this audit) can be wired green |
| 2 | EnumStore-consumer `jit_v2_make_result_ok/err`, `jit_v2_make_option_some` producers (return `Arc::into_raw(Arc<ResultData>) as u64`) | §2.7.5 / §2.7.17 | dependency of #1 |
| 3 | 6A Call-return kind-track (so `r`'s kind = `Ptr(HeapKind::Result)` after `divide(10,2)`) | §2.7.5 | parallel sub-cluster; load-bearing for `EnumTest` operand kind dispatch |
| 4 | This audit's `Rvalue::EnumTest` / `Rvalue::EnumPayload` MIR additions | §2.7.5 producer-site classification | blueprint landed; implementation deferred until #1+#2+#3 are co-designed |
| 5 | `compile_expr_match`-style schema-based `Pattern::Constructor` for user-defined enums in MIR lowering | §2.7.5 | out of scope; current MIR has no dispatch path for `enum Shape { Circle, Square }`-style user enums via JIT either. Same blueprint shape (`Rvalue::EnumTest` extended with `VariantTag::User(EnumLayoutId, variant_id)`) covers it when dispatched |
| 6 | Print of `Some(3)` requires heap-arm `jit_print` classification (Round 5B surfaced item #3 / phase-3-cluster-0-status surfaced items #3) | §2.7.5 | not 6B's territory; cluster-2 candidate per surfaced items table |

---

## §8. Close gates (audit-only delivery)

Per dispatch's audit-first discipline (if your audit reveals (iii), STOP
after the audit commit and SURFACE):

- `cargo check --workspace --lib --tests` EXIT=0 (no code change)
- `cargo test -p shape-jit --lib` baseline preserved (322/0/26)
- `bash scripts/verify-merge.sh` 12/12
- `bash scripts/check-no-dynamic.sh` EXIT=0
- Smoke 1.5: VM `5`, JIT `JIT execution error (code: -1)` (unchanged — the
  EnumStore producer surface is the blocker; this audit identifies what
  is needed)
- Smoke 2: VM `Some(3)`, JIT `JIT execution error (code: -1)` (same)

The audit lands the structural blueprint (`Rvalue::EnumTest` / `EnumPayload`
shapes, FFI signatures, JIT consumer skeleton, exhaustive-match-site
impact). No code change beyond the audit doc.

---

## §9. Forbidden patterns this audit does NOT propose

- **No "Arc-to-NaN-box translator" / "Result-shape decode bridge" / "match
  dispatch bridge" / "variant-tag translator" / "match-codegen helper"** —
  the proposed `Rvalue::EnumTest` / `EnumPayload` are MIR producer-site
  classifications per §2.7.5, not runtime dispatch translators (CLAUDE.md
  broader-family regex on `(decode|tag|kind|dispatch|...) (bridge|probe|
  helper|hop|translator|adapter|shim)` — refused on sight).
- **No Bool-default fallback for variant tag** — the variant is a
  producer-site classification at MIR-emission time (§2.7.7 #9); the JIT
  consumer dispatches on the MIR's `VariantTag` enum, never decodes from
  bits (§2.7.7 #4 / #7).
- **No deleted tag-bit decode resurrection** — the proposed FFI helpers
  read `Arc<ResultData>::is_ok` / `Arc<OptionData>::is_some` via direct
  `*const T` borrow, NOT via NaN-box tag bits or `is_heap_kind(bits, HK_OK)`
  (the deleted W-series predicate family).
- **No generic SwitchBool fallthrough for enum variants** — the producer-
  side `lower_match_pattern_condition_operand` rewrites the SwitchBool
  operand to a proper Bool slot via `EnumTest`, eliminating the kind-blind
  fallthrough path the current MIR shape exhibits.
- **No new HeapKind variant** — `HeapKind::Result` / `HeapKind::Option`
  exist (Wave 14 W14-variant-codegen, ordinals 27 / 28); the audit reuses
  them.

---

## §10. Disposition

**Audit lands. Implementation surfaced as cross-cluster dependency.**

The supervisor (Round 7 dispatch, or cluster-1 hardening) needs to either:

1. **Co-design a single sub-cluster** that lands #1 + #2 + #3 together
   (the §4.4 trinity from W12-jit-aggregate-non-array audit) — typically a
   2-day workstream per the wave 17 typed-carrier-monomorphization-bundle
   precedent (`docs/cluster-audits/phase-2d-handover.md` §"Audit before
   rebuild"). This audit is the blueprint.
2. **Or accept that Smokes 1.5 / 2 stay JIT-error-blocked through cluster-0
   close** and defer the §4.4 trinity to cluster-2 or beyond, treating the
   bytecode-VM path as the canonical Result/Option runtime and the JIT
   path as a "skip Result/Option-bearing functions" fallback for cluster-0.

Per supervisor's call. This sub-cluster (6B) closes at the audit-doc
landing.
