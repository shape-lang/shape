# REP-KIND-ABI (#239) — design

**Status**: design phase, awaiting owner review. No product code on this branch.
**Branch**: `rep-kind-abi`, worktree `shape-wave1-spine`, pinned at main `dac5fe7e`.
**Authority**: ADR-020 §1/§2/§3/§5/§6 (+ all 2026-07-29 amendment blocks);
CLAUDE.md §Forbidden Patterns and §Greenfield; #234 rulings; #225(e)/#226/#227
findings.

---

## 0. Summary

The ticket asks to thread static kinds through ~50 FFI signatures covering
"108 kind-blind `-> u64` entry points". Measurement at HEAD says the live
surface is **9 functions**, that the argument channel is **already kinded**,
that the error channel is **already kind-agnostic and correct**, and that the
only genuinely kind-blind channel is the **single `-> u64` return value**.

The rest of the dialect — 62 of 83 dialect-touching `-> u64` functions, and 77
of the 118 bare-bail returns — is not reachable from Cranelift-emitted code and
is **deleted, not converted**.

Revised scope: **~10 functions converted, ~62 deleted, 31 bare-bails treated**
(not 108 converted / 118 treated). This is materially smaller than the ticket's
estimate and lands comfortably as one slice.

Two additions the FFI-surface framing initially missed, both in the **emit
layer** (§3.5): the `Eq`/`Ne` fallthrough in `compile_binop_dynamic_cmp`, which
has generated three separate silent-wrong-answer defects and should be deleted
rather than point-fixed a fourth time; and `emit_index_to_i64`, which
reconstructs the deleted `is_tagged` dispatch as an inline `iconst` and would
therefore survive both the FFI deletion and a name-only ratchet. The second
changes the ratchet design: **it must match raw bit patterns, not just symbol
names.**

Grill round 1 produced three further corrections, all folded in: a fourth
partition bucket for functions that are unreachable *only* because an emit-site
deopt guards them, which the conversion deletes (§3.0.1 — `call_string_method`
moves out of "delete" into "convert"); an **ownership channel** the signature
must carry alongside the kind channel (§4.2 — heap returns get `*mut
HeapHeader`, scalars get `i64`, same Cranelift class, different Rust type); and
the restatement of §5's zero-safety claim as a **precondition with 25 open
sites** rather than a discharged property (§5.1).

**Grill round 2 refuted §4's central premise (#257), and the correction adds
scope.** The claim that unproven emit sites already surface-and-stop is false:
the guard exists at one site (`terminators.rs:2045`) and not at its siblings,
`slot_kind_of` is itself a fabricating `unwrap_or(Int64)` never called on an
FFI-return path, and seven `Array` method chains silently return `TAG_NULL` as
an `i64` today (one of them aborts). Closing that gap and deleting seven
`unwrap_or` defaults are **in-scope prerequisites landing before the
monomorphization** — monomorphizing an unguarded site changes the shape of the
wrong answer rather than removing it. Scope has now moved twice: **down** on the
partition (§3) and **up** here (§4.0).

---

## 1. Method — what was measured, and how

Everything below is reproduced at `dac5fe7e`, not inherited. Three independent
layers, because each one falsifies over-counting in the layer above:

1. **Registration** — `extern "C" fn` definitions in `crates/shape-jit/src`.
   423 non-test.
2. **Declaration into Cranelift IR** — the `r!()` keys in
   `compiler/ffi_builder.rs::build_ffi_refs`. **186** (the 187th previously
   reported was a comment-line false positive: the only `"jit_print"`
   occurrence is the deleted-marker comment at `ffi_builder.rs:86`).
3. **Emission into a call instruction** — `builder.ins().call(self.ffi.<field>, …)`
   anywhere in `mir_compiler/` + `compiler/`.
4. **Execution** — a throwaway first-hit stderr probe injected into 42
   candidate functions, run over all 481 corpus programs under
   `--mode jit`, plus 14 hand-written targeted falsifiers. Probe reverted;
   the branch is clean.

Commands (from the worktree root):

```bash
# layer 2
grep -n 'r!("[a-z0-9_]*")' crates/shape-jit/src/compiler/ffi_builder.rs \
  | grep -v '^[0-9]*: *//' | grep -o 'r!("[a-z0-9_]*")' | sed 's/r!("//; s/")//' | sort -u
# layer 3
grep -rhno 'ffi\.[a-z0-9_]*' crates/shape-jit/src/mir_compiler/ crates/shape-jit/src/compiler/ --include=*.rs
# layer 4
direnv exec /home/dev/dev/shape-lang cargo build --release --bin shape --jobs 4
for f in tools/vmjit-diff/corpus/*.shape; do target/release/shape run --mode jit "$f"; done
```

Static call-graph closure (comment/string-stripped bodies, `name(` edges, roots
= the 186 `r!()` keys + `WITNESS_ENTRY_SYMBOL` + the four macro-generated
`jit_v2_array_new_*`): `/tmp/…/scratchpad/{reach,partition,partition2,bails}.py`.

### 1.1 Reachability premise — CONFIRMED, and tightened

`compiler/ffi_builder.rs::build_ffi_refs` (line 65) is the sole `FFIFuncRefs`
constructor. The only other `declare_func_in_func` sites take user-function ids
(`compiler/program.rs:201`, `compiler/strategy.rs:288`) or the witness symbol
(`compiler/witness_emit.rs:51`). `jit_typeof`, `jit_to_string`, `jit_to_number`,
`jit_type_check`, `jit_iter_done`, `jit_pattern_check_constructor` have **zero
call sites in the entire workspace** — their only references are the `use`
import into the registration module and the `.expect("Failed to declare …")`
line. Declaration is not callability, as #226 found.

**New**: declaration into IR is not callability either. Two `FFIFuncRefs`
fields are populated by `r!()` but never appear in any `builder.ins().call`:

| field | key | dialect sites | corpus hits |
|---|---|---|---|
| `get_prop` | `jit_get_prop` | 15 | 0 / 481 |
| `alloc_owned_mut_cell` | `jit_alloc_owned_mut_cell` | 0 | 0 / 481 |

`jit_get_prop` is the **#2 entry on the ticket's own work list** and it is dead
at the Cranelift level. Any scoping that trusts the `r!()` set alone
over-counts.

#### The three liveness tiers — reusable test for any FFI-surface enumeration

State this explicitly, because #226 conflated tiers 1 and 2 and this lane's
first draft nearly conflated 2 and 3. For a JIT FFI symbol there are **four**
distinct questions, and only the last means "live":

| tier | question | how to test | count at HEAD |
|---|---|---|---|
| 1. registered | is there an `extern "C" fn` and a `declare(...)` call? | `grep 'extern "C" fn'` | 423 |
| 2. declared into IR | does `build_ffi_refs` populate a `FuncRef` for it? | the `r!()` keys | 186 |
| 3. **referenced** | is the `FFIFuncRefs` field **named anywhere** in `crates/shape-jit/src`? | `grep -rhno 'ffi\.[a-z0-9_]*' mir_compiler/ compiler/` | 184 |
| 4. executed | does a real program under `--mode jit` reach it? | first-hit probe over the corpus + targeted falsifiers | 12 of 42 probed |

**Tier 3 is "never named", NOT "never in a `builder.ins().call`" — and the
distinction is load-bearing.** The first draft of this table asked the narrow
question while running the broad test; the test was sound and the description
was not, which is §10.4.1's pattern inside the table that documents it. The
narrow form is **unsound**, because a `FuncRef` can be selected into a local and
called indirectly:

```rust
let retain_func = self.retain_func_for_place(place)?;   // returns self.ffi.arc_closure_retain
self.builder.ins().call(retain_func, &[val]);           // the field never appears at the call
```

The safety-perms lane measured the narrow formulation on the seed tree: **142
symbols flagged, including retain/release entries proven live by execution.**
The "never referenced at all" form is sound because no indirection can read a
field that is never named. That reasoning is now mechanized as `verify-merge`
CHECK 22 rule R3 and recorded in its baseline's `not_a_rule` field so nobody
"completes" the check later by tightening it.

Tier 1 → 2 is #226's finding (a registered symbol need not be callable).
**Tier 2 → 3 is this lane's addition**: a symbol can be declared into every
compiled function's IR and still have no call site, because `build_ffi_refs`
populates the struct unconditionally while the emit sites are conditional. A
`FuncRef` in the IR costs a relocation entry, not a call. Tier 3 → 4 is the
dynamic filter, and it is the one that needs the §1.2 denominator caveat
attached whenever it is cited.

An enumeration that stops at tier 1 over-counts by ~2.3x; one that stops at
tier 2 still admits `jit_get_prop`. **Any future lane enumerating an FFI
surface should run all four.**

### 1.1.1 THREAT TO THIS SECTION'S VALIDITY — #260

Every "N hits across 481 programs" number in this document assumes a corpus run
is **reproducible**. #260 records a nondeterministic V2 bytecode verifier —
identical input yielding 0, 2 and 8 violations across five runs, confirmed by the
supervisor. The stronger half of the claim, that it can disable the JIT run to
run, was **not** reproducible (fourteen witness runs came back fully native), so
the two halves are filed apart.

**If the stronger half ever holds, it invalidates every dynamic measurement
here** — mine and the refuters' alike — because a program that silently ran
interpreted contributes a zero indistinguishable from a genuine absence. That is
the same instrument failure §7.1 admits to in a different guise.

This is a further reason the dynamic numbers in §3 are presented as a filter over
a static partition rather than as the partition itself, and why every
unreachability claim in §3 is additionally backed by a static impossibility or a
targeted falsifier. **Re-run the corpus twice and compare failure NAME SETS, not
counts, before trusting any number in this document.**

### 1.2 Denominator caveat, stated up front

Only **121 of 481** corpus programs execute any native code (287 bail
whole-program; 188 have no program-level fallback but only 121 record ≥1
native dispatch; 6 produce no witness). So "0 corpus hits" is evidence, not
proof — which is why every "unreachable" claim below is backed by either a
static impossibility (no emit site) or a targeted falsifier that reaches the
construct and observes what happens.

---

## 2. The channel, not the substitution

The ticket's framing is correct and re-verified at HEAD.

**There is no i64 channel and no bool channel.** `jit_load_col_i64`
(`ffi/data.rs:323`) ends at line 368:

```rust
// Read as f64 (JIT stores all numerics as f64), truncate to integer
let value = *col_ptr.add(row_idx);
box_number(value.trunc())
```

`jit_load_col_f64` likewise ends in `box_number`. `jit_load_col_bool` no longer
exists (deleted by #226; the marker comment sits at `ffi/data.rs:371`). A
per-type monomorphized family that funnels every type into `f64::to_bits` is
not a typed channel — it is one untyped channel with three names.

**The compiler already says so, in a shipped error message.** A `string`
method returning a scalar whole-function-deopts rather than emit the call.
Source: `mir_compiler/terminators.rs:602-620`; quoted verbatim from a live
`--mode jit` run of `s.length()` inside a function:

> MirToIR: scalar-returning string method `.length(...)` on a proven
> `NativeKind::String` receiver has no sound JIT codegen — the `jit_call_method`
> VM trampoline boxes the scalar result via `box_number(.. as f64)` (a NaN-boxed
> f64) or a `TAG_BOOL_*` sentinel, NEITHER of which is the raw native scalar the
> proven destination slot expects. `write_place` stores the NaN-box bits verbatim
> into the (e.g. `Int64`) slot → garbage … STAGE-StringJIT.

That is this ticket's thesis, already written down as a permanent deopt.

**And it is not the only one. There are two measured whole-program deopt classes,
both caused by the untyped channel:**

| class | trigger | measured effect |
|---|---|---|
| **STAGE-StringJIT** (`terminators.rs:601`) | any scalar-returning method on a proven `string` receiver | whole-function deopt |
| **STAGE-F3** (`terminators.rs:689`) | any method on a `DateTime`/`Temporal`/`Instant`/`Decimal`/`BigInt`/`DataTable`/`TableView`/`Content` receiver | **whole-PROGRAM deopt** — `--native-witness` reports `scope: "program"`, `reason_class: "jit-compile-error"`, **0 native dispatches** |
| **StringV2 equality** (`rvalues.rs:986::both_string`, new on `main` `4b773a0d`) | `==`/`!=` where either operand is `NativeKind::StringV2` | deopt — the `both_string` gate admits only `String`, so `StringV2` surfaces |
| **Route A / ObjectStore** | unprojected object schemas, `Rvalue::Aggregate` | surface-and-stop |

The second is the sharper number: **any program that calls a method on a
DateTime, Decimal, BigInt, DataTable, TableView or Content value runs entirely
interpreted.** Not the enclosing function — the entire program. Verified on
`fn f(d: DateTime) -> int { return d.unix_timestamp() + 1 }`, which is correct on
both tiers precisely because the JIT never runs.

**So the conversion's payoff is not "delete some constants".** It is restoring
native execution across **four** measured deopt classes. The `StringV2` row is
the newest and the most explicit: the merged fix's own comment
(`rvalues.rs:983-985`) defers it to this ticket by name — *"Widening this to one
carrier-blind string kind is representation-program territory (ADR-020 / #239),
not a local patch."* That is a scope handoff recorded in the tree, and it is the
concrete form of the `String`/`StringV2` carrier duality discussed in §12.5.

Worth noting for §4.2: `jit_string_eq`'s correctness depends on a **strong-count
contract** over its operands, and `StringV2` is excluded precisely because its
slots have no retain arm in `ownership.rs::retain_func_for_place`. The merged
fix's soundness therefore rests on exactly the share-accounting invariants O1/O2
specify — independent corroboration that the ownership channel is load-bearing
and not a theoretical nicety.

### 2.1 Precisely which channel is kind-blind

Not the arguments. `jit_call_value(ctx: *mut JITContext) -> u64` and
`jit_call_method(ctx, count) -> u64` pass callee + args + arg-count **through
the JITContext stack**, and the emit site already writes the §2.7.7 parallel
kind track for every slot it pushes (`terminators.rs:2719`,
`emit_kind_track_write(argc_slot_idx, NativeKind::UInt64)`). Arguments are
kinded today.

Not the errors either. #234's channel is already built and already correct:
`ctx.pending_call_error = 1` plus a return of
`shape_value::encoding::ERROR_PLACEHOLDER_BITS`, which is **`0`**
(`crates/shape-value/src/encoding.rs:164`). Zero is memory-safe in *every*
`NativeKind` — null pointer, `0i64`, `false`, `0.0`. It needs no kind by
construction. The emit side already deopts on the flag before `write_place`
(`terminators.rs::emit_pending_call_error_deopt`, called at
`terminators.rs:1981` and `:2217`). 31 sites already do this.

**What is kind-blind is exactly one thing: the `-> u64` return value.** The
emit site takes `builder.inst_results(inst)[0]` and hands it straight to
`write_place(destination, result)` (`terminators.rs:2225`; six such sites at
`:446`, `:879`, `:1792`, `:1983`, `:2225`, `:2638`), which stores the bits
verbatim into a slot whose kind the storage planner already fixed.

So the conversion is: **give the return value the destination's kind.**

---

## 3. The three-way partition and the revised scope

83 `-> u64` functions in shape-jit touch the tag dialect
(`TAG_NULL`/`TAG_NONE`/`make_tagged`/`is_tagged`/`get_tag`/`TAG_BOOL_*`/
`box_function`/`is_inline_function`/`TAG_FUNCTION_BITS`/`unified_box`):
64 `extern "C"`, 19 internal helpers.

| bucket | fns | dialect sites | disposition |
|---|---|---|---|
| **(a) reachable AND executed** | 12 | ~72 | convert — kind-threaded returns |
| **(b) reachable, producer-less after #227** | 2 | 4 | delete with the carrier flip |
| **(c) unreachable, and STAYS unreachable after the conversion** | 68 | ~147 | delete outright |
| **(d) unreachable ONLY because an emit-site deopt guards it** | 1 | 16 | **convert** — the conversion deletes the guard |

### 3.0.1 Bucket (d) — the bucket the first draft was missing

A grill pass found an internal contradiction in the first draft, and it was
real. §2 argues the conversion's payoff is that the STAGE-StringJIT deopt "can
be deleted, restoring native execution", and §10.1 makes that deletion a
required acceptance fixture — yet the first draft put `call_string_method` in
bucket (c) *because* that deopt makes it unreachable. Both cannot hold: if the
deopt dies, the function it was gating becomes live.

The resolution is that "unreachable" was hiding two different states, and only
one of them is a delete:

- **(c) unreachable for a reason the conversion does not change** — no emit
  site (`jit_get_prop`), no source-level producer (`jit_typeof`: `typeof` is
  not a Shape builtin), not registered (`jit_series_*`), or **reachable only
  through a carrier the conversion itself deletes**.
- **(d) unreachable only because an emit-site guard refuses to emit the call,
  where that guard exists *because* the channel is untyped.** Deleting the
  guard is the point of the ticket. These must be converted, in this slice,
  or the acceptance fixture cannot pass.

Measured membership. There are exactly **three** emit-site deopt guards, all in
`mir_compiler/terminators.rs`:

| guard | line | condition | what it gates |
|---|---|---|---|
| STAGE-M1 | `:543` | proven `String` receiver + string-returning method | `call_string_method` |
| STAGE-StringJIT | `:601` | proven `String` receiver (scalar-returning) | `call_string_method` |
| STAGE-F3 | `:689` | receiver `Ptr(Temporal\|Instant\|Decimal\|BigInt\|DataTable\|TableView\|Content)` | the `_ => TAG_NULL` cascade arm |

**`call_string_method` → bucket (d), +16 sites.** It has two inbound routes
inside `jit_call_method`: the `NativeKind::String` arm
(`ffi/call_method/mod.rs:1165`) and the legacy `HK_STRING` cascade arm
(`:1310`). The first is gated *only* by STAGE-M1/StringJIT, so it goes live the
moment those die. Its 16 dialect sites are converted, not deleted.

**The four sibling dispatchers stay in bucket (c)** — and the reason is
measured, not assumed. `call_object_method` (11 sites), `call_duration_method`
(6), `call_matrix_method` (4), `call_time_method` (3) and `matrix_transpose` (1)
are reachable **only** through the legacy JIT-format cascade at
`ffi/call_method/mod.rs:1310-1317` (`HK_JIT_OBJECT` / `HK_DURATION` /
`HK_MATRIX` / `HK_TIME`), which runs only under the `UInt64`
opaque-bits carrier guard — i.e. the `unified_box` NaN-box carrier. That carrier
is on §8's deletion list. **Their only route dies with the dialect**, so unlike
`call_string_method` they do not come back, and they are correctly bucket (c).

**STAGE-F3 is a routing obligation, not a dispatcher to convert.** Its 7
VM-only typed-Arc receiver kinds currently fall to the cascade's silent
`_ => TAG_NULL` arm (`:1318`) — which its own error text documents as producing
`fn f(d: DateTime) -> int { d.unix_timestamp() + 1 }` → **rc=139 SIGSEGV**.
Deleting the guard therefore requires those receivers to route to
`dispatch_call_via_trampoline_vm` (already bucket (a), live, 8 programs)
instead of the `TAG_NULL` arm. That is a routing edit in the same slice, and it
retires a second live SIGSEGV class alongside #254.

**Scope effect: +1 function and +16 sites converted, −16 from bucket (c).** The
headline (~9 functions converted) becomes **~10**, and the deletion bulk is
essentially unchanged. The slice does not change shape.

Bucket (a), with measured corpus execution counts (number of the 481 programs
that executed the function at least once, `--mode jit`):

| fn | file:line | programs | note |
|---|---|---|---|
| `box_string` | `ffi/value_ffi.rs:536` | 150 | `unified_box` string carrier |
| `jit_call_value` | `ffi/control/mod.rs:532` | 20 | **the value-call channel** |
| `jit_typed_object_set_field` | `ffi/typed_object/field_access.rs:68` | 9 | |
| `jit_typed_object_get_field` | `ffi/typed_object/field_access.rs:21` | 9 | ticket's worked example |
| `jit_typed_object_alloc` | `ffi/typed_object/allocation.rs:57` | 9 | |
| `dispatch_call_via_trampoline_vm` | `ffi/control/mod.rs:133` | 8 | |
| `jit_call_method` | `ffi/call_method/mod.rs:595` | 4 | |
| `jit_schema_option_some` | `ffi/typed_object/option.rs:105` | 3 | |
| `dispatch_borrowed_closure_via_trampoline_vm` | `ffi/control/mod.rs:258` | 3 | |
| `build_option_object` | `ffi/typed_object/option.rs:79` | 3 | |
| `box_str` / `matrix_transpose` | `ffi/value_ffi.rs:542` / `call_method/matrix.rs:43` | 0 | reachable, trivially deleted with `unified_box` |

Bucket (b): `box_function` (29 programs) and `is_inline_function` (22
programs) — **live on the production path**, confirming #227's counter-datum
and refuting any reading of #226 that would put them in (c).

Bucket (c) highlights — reachable-looking but provably not callable:

- `jit_get_prop` (15 sites) — no emit site at all (§1.1).
- `jit_set_prop` (1 site) — has two emit sites (`mir_compiler/places.rs:1343,1348`)
  but they are the unprojected-object-schema path; a hand-written dynamic-object
  falsifier surface-and-stops at `ObjectStore: SURFACE — schema id 91
  (__inline_obj_54) field a has …` before reaching them. 0 corpus hits.
- `call_object_method` (11 sites), `call_duration_method` (6),
  `call_matrix_method` (4), `call_time_method` (3) — reachable only via the
  legacy `unified_box` `UInt64` cascade this ticket deletes (§3.0.1), 0 hits.
  **Note `call_string_method` is NOT in this list** — it looks identical from
  here (statically reachable, 0 corpus hits, all falsifiers deopt) but it is
  **bucket (d)**, because its `NativeKind::String` route is gated only by the
  emit-side deopts this conversion removes. See §3.0.1; the difference is
  whether the route survives the conversion, and it is not visible from the hit
  count.
- `jit_iter_next` (8 sites) / `jit_iter_done` (4) — a `for v in arr` loop
  executes 301 native dispatches and never touches them; array iteration is
  fully native.
- `jit_typeof` (4 sites) — `typeof` is not a Shape builtin at all
  ("Undefined function: 'typeof'"). It has no source-level producer.
- the 21-function `jit_series_*` / `jit_time_*` / `jit_eval_*` family (~40
  sites) — none registered as callable, none executed.

**Correction to #226's reachable list.** #226 named `jit_call_method`,
`jit_call_value`, `jit_get_prop`, `jit_set_prop`, `jit_print`,
`jit_string_concat`. That list is wrong in both directions: `jit_get_prop` and
`jit_set_prop` are dead; `jit_print`/`jit_string_concat` carry no dialect
sites (already kind-split into `jit_print_i64`/`_u64`/`_f64`/… by W11/W12);
and it omits the live `jit_typed_object_*` trio, `jit_schema_option_some`,
`build_option_object`, and both trampoline dispatchers. The corrected list is
the bucket-(a) table above.

---

## 3.5 The second category: dialect in the EMIT layer

Everything above enumerates the dialect as it exists in **FFI function bodies**.
That framing has a blind spot, and the supervisor's `jit_string_eq` note is what
exposed it: the JIT also emits tag logic **directly into Cranelift IR**, where
it is not an `extern "C" fn`, has no `r!()` key, and appears in no FFI-surface
enumeration.

Measured split of dialect sites (non-comment, non-prose):

| layer | sites | nature |
|---|---|---|
| `ffi/` | 394 | FFI function bodies — §3's partition |
| `ffi_symbols/` | 110 | registration + intrinsic bodies — mostly bucket (c) |
| `mir_compiler/` | 10 | **emit layer** — 8 are prose inside error-message string literals; 2 are real |
| `compiler/` | 1 | prose inside an error message |

The emit layer is nearly clean, which is good news for scope — but the
residue is disproportionately dangerous, because emit-layer code is
**unconditionally live** (it is the compiler, not the runtime) and because two
of the three instances are invisible to a symbol-name ratchet.

### 3.5.1 `compile_binop_dynamic_cmp` — one kind-blind arm, three incidents

Answering the supervisor's direct question: `compile_binop_dynamic_cmp`
(`mir_compiler/rvalues.rs:2001`) has exactly **two** arms and only **one** is
kind-blind.

- `Eq`/`Ne` → `builder.ins().icmp(cc, lhs, rhs)` on `to_i64_bits`-widened
  operands, justified in-comment as "kind-mismatched bits are unequal by
  construction".
- `Lt`/`Le`/`Gt`/`Ge` → `Err(...)`, surface-and-stop.

So there is no further ordered-comparison inventory. But the Eq/Ne arm's *fix
history* is the finding, and it is worth more than the site count. Three
separate defects have been traced to this one arm, each resolved by adding a
kind proof **upstream** so that one more case stops falling through, while the
fallthrough itself survived every time:

1. **Narrow ints** — `let c: i8 = -56; c == -56` gave VM `true`, JIT `false`,
   because `to_i64_bits` zero-extends an `I8` while the literal slot held a
   sign-extended `I64`. Positive values coincided, so it hid. Fixed by adding
   `narrow_int_cmp_kind` (`rvalues.rs:864`).
2. **`arr.length`** — an unproven kind on the length projection sent
   `i < arr.length` down this path and **deopted every length-bounded loop**,
   making every shape `bounds_elision` can prove invisible to the native tier.
   Fixed by adding a `Place::Field` kind arm (`rvalues.rs:719`).
   **This is the mechanism behind a standing puzzle in the repo**: the measured
   result that JIT bounds-check elision gives no speedup even when it fires
   (0.999x on a check-dominated kernel), and that the 5.3x once credited to BCE
   was really a `.length` kind-projection nativity fix. Of course BCE measured
   ~0 — until this arm was fixed, every length-bounded loop was deopting
   wholesale, so there was no native loop for BCE to optimize. A puzzling
   benchmark note becomes an understood one, and the deletion candidate status
   of BCE should be re-evaluated *after* this conversion, on a tier that
   actually runs the loops natively.
3. **Heap strings** — `Eq`/`Ne` on heap strings emitted a raw `icmp` on two
   POINTERS. Invisible because interned string literals share a pointer, so
   literal-vs-literal passed by luck. Fixed by the safety lane's `jit_string_eq`
   (#232, merged `4b773a0d`).

**Incident 4 landed while this document was being written, and followed the
predicted shape exactly.** Verified at `origin/main` (`4b773a0d`): the fix adds a
`both_string` guard **upstream** at `rvalues.rs:81`, and **the raw-`icmp`
fallthrough is byte-for-byte unchanged** at `rvalues.rs:2116-2124`. That is a
fourth kind proof added ahead of a fallthrough that has now survived four
incidents.

To be unambiguous, because this is a prediction coming true and not a criticism:
**the safety lane did the right thing.** Fixing a live silent-wrong-output bug by
proving the kind upstream is the correct immediate action, and deleting the
fallthrough was never their charter — their own code comment says so, deferring
the general case to this ticket by name (`rvalues.rs:983-985`): *"Widening this
to one carrier-blind string kind is representation-program territory (ADR-020 /
#239), not a local patch."*

The point is what happens **next**. Four incidents, four upstream proofs, one
surviving fallthrough. If #239 does not delete it, incident 5 is already paid
for. §4.0.3 finds the identical shape in a second subsystem with five more point
fixes; that is nine rescues across two mechanisms, which is why the ruling is
delete rather than prove-once-more.

That is the ticket's thesis stated three times by three unrelated incidents: a
kind-blind fallthrough is correct for the kinds its author had in mind and
silently wrong for the rest, and each rescue narrows the hole without closing
it. **Design ruling (RATIFIED, grill round 1): this conversion DELETES the
`Eq`/`Ne` arm rather than adding a fourth kind proof.** The supervisor asked
that the reasoning be recorded at **ADR level, not only here** — three
silent-wrong-answer defects traced to one kind-blind fallthrough, each fixed by
adding a proof upstream while the fallthrough survived, is the §Forbidden
walk-back documented in miniature with three iterations already on the board. A
fourth rescue is the pattern, not a fix. See §4.0.3 for the same shape found
independently in a second subsystem (five more point fixes), which is what
turns this from an anecdote into a rule. Once every operand carries a proven kind, the
ordered-comparison treatment (surface-and-stop) is correct for equality too,
and `compile_binop_dynamic_cmp` ceases to exist rather than shrinking again. A
fourth point fix is the walk-back shape CLAUDE.md §Forbidden names.

### 3.5.2 A tag test that no symbol ratchet can see

`mir_compiler/places.rs:523::emit_index_to_i64` emits a **runtime tag test into
machine code**:

```rust
let tag_base = self.builder.ins()
    .iconst(types::I64, 0xFFF8_0000_0000_0000u64 as i64);
let is_tagged = self.builder.ins()
    .icmp(IntCC::UnsignedGreaterThanOrEqual, index_bits, tag_base);
// … float path: bitcast+fcvt;  int path: ishl 16 / sshr 16 (i48 payload)
self.builder.ins().select(is_tagged, from_int, from_float)
```

This is the deleted `is_tagged` dispatch, reconstructed inline. It does not call
`is_tagged`, does not name `TAG_BASE`, and **spells the constant as a literal**
— so deleting the FFI dialect and ratcheting every symbol in §8 would leave it
untouched and passing. `0xFFF8` appears as a literal at only three
non-comment sites in shape-jit (`places.rs:532`, two in a test at `:1838-1839`,
plus the `TAG_BASE` definition itself), so the residue is small — but the
lesson generalizes.

**Consequence for §8: the ratchet must include the raw bit patterns, not only
the symbol names.** A ratchet that only knows names is defeated by an `iconst`.

**And the general principle, which outlives #239 — write it where the next gate
author reads it:**

> **A forbidden dispatch can be spelled without naming any forbidden symbol.**
> Symbol rows are therefore insufficient *in principle*, not merely incomplete.
> A gate over a semantic prohibition must match the operation's *shape* — its
> constants, its instruction sequence — not only its vocabulary.

Note the trap in this specific instance: the local at `places.rs:533` happens to
be *named* `is_tagged`, so a symbol row for that name would catch this one site.
**That is luck, not design.** Rename the local and the site is invisible again,
while the actual dispatch — the constant, the `icmp`, the `select` — is
unchanged. A gate that catches this by name has not caught the pattern; it has
caught one author's choice of identifier.

Reachability of this specific site is **open and I did not settle it**. Its
caller chain (`index_to_i64` → `inline_array_get` / siblings at
`places.rs:600,653,696,711`) is documented throughout as the **legacy v1 array
layout** (`data@+0 / len@+8`, the JitArray/UnifiedArray shape that Route A
deleted), and my three array-indexing falsifiers (int index, negative index,
number array) all produced VM==JIT correct results, i.e. they took the typed
`v2_array` path. Note the arm is only reached when the Cranelift value type is
`I64` and not `F64`/`I32`/`I8`. I flag it as **inventory the implementing lane
must resolve**: either prove the legacy array path unreachable and delete the
chain (my expectation, consistent with §3's bucket (c)), or convert it. Do not
leave it as-is on the strength of "the tests pass" — by construction they would.

### 3.5.3 Inbound FFI-surface changes — status verified at `origin/main` (`4b773a0d`)

The safety lane's #232 fix is merged. I inspected `origin/main` directly rather
than working from the coordination note, and **three details differ from how the
change was described to me**. Recording them because the implementing lane will
otherwise inherit a stale picture.

**`jit_string_eq` — the kind codes were RULED unnecessary, and I was right to
push.** I argued the explicit `a_kind_code`/`b_kind_code` parameters are the
shape §4 forbids: a runtime discriminator where a static proof exists. The
supervisor initially ruled "bounded exception with carrier unification as the
creditor", then **reversed** on the evidence — the fix gates the arm on
`both_string` (`rvalues.rs:81`, `:986`), meaning **both operands are proven
`NativeKind::String` at the emit site**, so the kind codes are compile-time
constants and mixed `String` × `StringV2` is unreachable *by construction of the
fix itself*. There is no exception to bound because there is no polymorphism at
the call site.

**Status at `main`: the parameters are still present** —
`jit_string_eq(a_bits: u64, a_kind_code: u8, b_bits: u64, b_kind_code: u8) -> u8`
at `ffi/conversion.rs:1162`. The removal is a follow-up commit that has not
landed. **This design's monomorph set does NOT carry them**, and §12.5 needs no
retirement row.

**`jit_v2_string_eq` — NOT deleted at `main`.** It is still defined at
`ffi/v2_string_ffi.rs:197` with its seven direct-call unit tests intact
(`:374-413`). It therefore **remains in this ticket's bucket (c)** rather than
leaving the inventory, and it stays a textbook tier-1-only symbol (§1.1):
registered-looking, never callable, tests green throughout because they call it
as a Rust function.

**The `both_string` arm is a fourth upstream kind proof, and the raw-`icmp`
fallthrough is unchanged** (`rvalues.rs:2116-2124`, byte-identical). See §3.5.1 —
this is the predicted pattern occurring on schedule, and it is the reason the
deletion ruling matters rather than a reason to revisit it.

**Net effect on the §3 partition: none.** `jit_string_eq` is a worked example of
the target ABI on the correct side of the line; `jit_v2_string_eq` stays in
bucket (c); the `StringV2` deopt (§2) is new payoff scope this ticket already
owns by the merged code's own deferral.

## 4. The monomorphization rule

The ticket poses a dichotomy: per-kind monomorphized helpers where the emit
site knows the kind, *or* an explicit kind parameter where genuinely
polymorphic.

**The rule (no runtime kind parameter) stands. The premise the first two drafts
gave for it was FALSE, and the correction adds scope.** Filed as **#257**.

### 4.0 What was refuted, and what survives

The first draft asserted: *"every emit site must already prove its destination
kind to `write_place`, and sites that cannot surface-and-stop today."* The
second half is false, and I verified the refutation independently rather than
accepting it.

**Code evidence, confirmed at HEAD:**

- `mir_compiler/conversions.rs:29::slot_kind_of` — the function I cited as the
  proof gate — is itself
  `slot_kind_for_local(...).unwrap_or(NativeKind::Int64)`. **It fabricates.**
  Its own docstring says so: *"Codegen sites that specifically need a 'kind was
  proven by inference' answer should call `slot_kind_for_local` directly and
  surface-and-stop on `None`."* I cited the function that documents itself as
  not being the gate.
- It is called **once** in non-test code (`places.rs:1463`, inside
  `null_place`) and **never on an FFI-return path**. The mechanism I cited is
  not the mechanism that runs.
- `write_place`'s `Place::Local` arm fabricates the same default
  (`places.rs:1299-1300`).
- The guard does exist — at **one** emit site. The direct-call path checks
  `slot_kind_for_local(&self.slot_kinds, dst.0).is_none()` and surfaces
  (`terminators.rs:2045`). The generic method-call path
  (`terminators.rs:822-879`) takes `inst_results(inst)[0]` and hands it to
  `write_place` with **no such check**. It was written once and never applied
  to its siblings.

**Empirical evidence, reproduced independently on a clean release binary.**
Seven `Array` methods emit silent wrong answers, natively dispatched, `rc=0`,
no bail — the value is `TAG_NULL` read as `i64`:

| expression | VM | JIT | rc |
|---|---|---|---|
| `a.slice(0,2).len()` | `2` | `-1407374883553280` | 0 |
| `a.sort().len()` | `3` | `-1407374883553280` | 0 |
| `a.concat([9]).len()` | `4` | `-1407374883553280` | 0 |
| `a.zip([4,5,6]).len()` | `3` | `-1407374883553280` | 0 |
| `a.take(2).len()` | `2` | `-1407374883553280` | 0 |
| `a.skip(1).len()` | `2` | `-1407374883553280` | 0 |
| **`a.unique().len()`** | `3` | *(no output)* | **134 — SIGABRT** (this lane) / **1** with `Array.includes: receiver bits failed v2 TypedArray detection (kind Ptr(TypedArray))` (supervisor) |

Two controls isolate the variable exactly: `let s: Array<int> = a.slice(0,2);
s.len()` matches (the annotation seeds the slot kind), and `a.first().len()`
matches (`first` is in the parametric table at `types.rs:1319`). Same shape;
the only difference is whether a stamp exists.

**The `unique` row has two different observations recorded deliberately** — this
lane saw rc=134 (SIGABRT), the supervisor saw rc=1 with a
`v2 TypedArray detection` diagnostic. Both are recorded rather than one being
picked, because the disagreement is itself a datum: it means at least one input
to that path is **nondeterministic**, which bears directly on #260 below. Either
way it is not a silent wrong answer, so it does not change the seven-shape
count's character.

**One datum beyond the refutation as filed:** `a.unique().len()` does not
merely return a wrong number, it **aborts (rc=134)**. That is a third live
memory-safety signal in this ticket's territory, alongside #254 and the
STAGE-F3 `DateTime` SIGSEGV (§3.0.1).

**What survives, and it is the load-bearing half.** The *rule* is unaffected:
a runtime kind parameter is still forbidden, because it would reintroduce a
runtime type discriminator where the compiler is *supposed* to have a static
proof — CLAUDE.md §Forbidden ("Runtime `tag_bits` dispatch",
"`SlotKind::Dynamic`") wearing a parameter instead of a tag word. The
refutation is not "you need polymorphism"; it is "you asserted the fail-closed
behaviour already exists, and it does not".

### 4.0.1 The rule, correctly stated

> **Every value-producing FFI emit site must surface-and-stop when the
> destination slot kind is unproven.** Where the kind is proven, the site calls
> the corresponding monomorph. There is no polymorphic fallback and no runtime
> kind parameter. **Today at least three emit sites violate this, and the
> `unwrap_or` defaults that let them do so are in-scope prerequisites of this
> conversion — they land before the monomorphization, not after.**

**Why this ordering is not optional.** An implementer following the first
draft writes `match slot_kind_of(dest)` to pick the monomorph, receives a
fabricated `Int64` for an unproven slot, and calls `jit_call_method_i64` for a
value that is actually `Ptr(HeapKind::TypedArray)`. Once `Float64` splits into
its own monomorph (§4.1), an unproven slot holding an `f64` routes to the i64
monomorph and receives raw bits — **exactly the `box_number` corruption §4.1
claims the f64 monomorph eliminates.** Monomorphizing on an unguarded site does
not fix the silent wrong answer; it changes its shape. The guard extension is a
prerequisite, not a companion.

### 4.0.2 The seven fabricating defaults (in-scope inventory)

All verified at HEAD. #236 documents two of the seven.

| site | default |
|---|---|
| `mir_compiler/places.rs:1300` | `unwrap_or(NativeKind::Int64)` — inside `write_place` |
| `mir_compiler/conversions.rs:30` | `unwrap_or(NativeKind::Int64)` — inside `slot_kind_of` |
| `mir_compiler/blocks.rs:90` | `unwrap_or(NativeKind::Int64)` |
| `mir_compiler/rvalues.rs:301` | `unwrap_or(NativeKind::Int64)` |
| `mir_compiler/statements.rs:872` | `unwrap_or(NativeKind::Int64)` |
| `osr_compiler.rs:225` | `unwrap_or(NativeKind::Int64)` |
| `mir_compiler/rvalues.rs:590` | `unwrap_or(NativeKind::UInt64)` |

Deleting them turns `Option<NativeKind>` back into the proof it already is —
the same mechanical enforcement `prove_native_kind() -> Result<_, ProofGap>`
provides on the VM side (CLAUDE.md §Mechanical enforcement). The type system
already carries the information; seven `unwrap_or`s throw it away.

### 4.0.3 Prior art: the same walk-back, five more times

`mir_compiler/types.rs` records this exact class being point-fixed **at least
five times** by adding one more name arm to a keyed allowlist — HashMap `has`,
HashSet `add`, Deque `pushBack`, `PriorityQueue::push`, and the iterator
adapters. `well_known_method_return_kind` (`types.rs:1081`) is that allowlist,
and its own comment promises "no fabricated default" while `write_place`
fabricates one two files away.

That is the **same three-strikes shape** §3.5.1 identifies for
`compile_binop_dynamic_cmp`, found independently in a second subsystem: a
kind-blind default, correct for the kinds someone had in mind, silently wrong
for the rest, and rescued one name at a time. Two independent instances of one
pattern is the argument for deleting the mechanism rather than extending the
allowlist a sixth and a fourth time respectively.

### 4.1 Shape of the converted signatures

Cranelift return types are per-signature, so the monomorph set is driven by
Cranelift ABI classes, not by all 27 `NativeKind` variants. Three classes:

| class | Rust return | Cranelift return | members |
|---|---|---|---|
| scalar | `i64` | `types::I64` | `Int64`, `UInt64`, `Int32`… (widened in-slot), `Bool`, `Char` |
| pointer | `*mut HeapHeader` | `types::I64` | all `Ptr(HeapKind)`, `String`, `StringV2` |
| float | `f64` | `types::F64` | `Float64` |
| void | *(none)* | *(no results)* | `Null` (ADR-020 §3.3 — already partly landed; `terminators.rs:2219` has the void-call arm) |

So `jit_call_value` becomes `jit_call_value_i64` / `jit_call_value_ptr` /
`jit_call_value_f64` / `jit_call_value_void`, and likewise for the other
converted entry points.

**The scalar and pointer classes are one Cranelift class but two Rust
signatures, deliberately** (see §4.2). They lower identically — `*mut
HeapHeader` and `i64` are both `types::I64`, so this costs nothing at the ABI —
but at the Rust level they are different types, which is what lets rustc
distinguish "this return transfers an owned share" from "this return is a
number". Collapsing them into a single `i64` would erase the only remaining
carrier of that distinction.

`Float64` must be its own monomorph, and this is the load-bearing part: it is
the only way `box_number` dies. Today every numeric result is `f64::to_bits`
into an `i64` return. Splitting the f64 monomorph out means the value travels
in an FP register as an `f64` and never becomes bits at all.

`NullableFloat64` returns `f64` and carries §3.1's canonical NaN sentinel;
nullable narrow scalars return `i64` with §3.1's widened out-of-range niche.
Nullable 64-bit integers do **not** appear: per ADR-020 §5's third sequencing
ruling their presence-pair machinery lands with #229, and at HEAD those slot
kinds have no producer.

### 4.2 The ownership channel — the second thing the signature must carry

A grill pass caught that §4.1's original three-class table specified the *kind*
channel and silently dropped the **ownership** channel. Collapsing `Int64`,
`Bool`, `Char` and every `Ptr(HeapKind)` into one `i64` return means the
signature cannot say whether the caller received an owned share, a borrowed
pointer, or a plain scalar — and those demand different caller behaviour. The
W17 close is the cautionary precedent: a `KindedSlot::new(...)` + `clone()` that
claimed ownership without bumping the refcount produced a ghost share and a
use-after-free at snapshot-drop time.

**The rule at HEAD, measured.** The emit site does:

```rust
self.release_old_value_if_heap(destination)?;   // terminators.rs:2224
self.write_place(destination, result)?;         // terminators.rs:2225
```

`write_place` (`places.rs:1204`) contains **no retain** on any arm. By contrast
the *constant* producers retain explicitly at the producer site
(`arc_string_retain`, `ownership.rs:951` / `:991`), because the constant pool
keeps its own permanent share. The asymmetry is the convention:

> **Invariant O1.** An FFI call's heap return **transfers exactly one owned
> share** to the caller. The emit site releases the destination's old value and
> stores the new one without retaining. A constant producer, which does not
> transfer, retains at the producer site instead.

This is currently a convention held together by comments. The conversion makes
it a type-level obligation:

> **Invariant O2.** A converted entry point returns `*mut HeapHeader` **iff** it
> transfers a share; it returns `i64` / `f64` iff it transfers nothing. There is
> no third case — a borrowed heap pointer is not a legal return, because the
> callee cannot bound the borrow's lifetime across the FFI edge. An entry point
> that today returns a borrowed pointer must retain before returning.

Two consequences worth stating so they are not rediscovered:

1. **The `_ptr` monomorph is the retain audit.** Converting an entry point to
   `-> *mut HeapHeader` forces its author to answer "does this path transfer?"
   on **every** return path, including the bail paths in §5. Today those bails
   return `TAG_NULL` — a non-share — into a slot the emit site may treat as
   heap; that mismatch is one of the two mechanisms behind #254.
2. **O1/O2 join the §10.2 kind-agreement assertion.** The emit-time check
   becomes: the chosen monomorph's class matches the destination's **proven**
   kind — `slot_kind_for_local(...)`, **never** `slot_kind_of`, which is itself
   a fabricating `unwrap_or(Int64)` (§4.0) — **and**, when that kind is heap,
   the monomorph is the `_ptr` one. A scalar
   monomorph writing into a `Ptr(_)`-stamped slot is exactly the
   `ClosurePlaceholder` defect (§6.1) in general form, and this assertion is
   what would have caught it at the emit site.

This is the class of defect §10.2 argues unit tests cannot see, so O1/O2 need
the emit-time assertion and the corpus fixtures, not a test that calls the FFI
function directly with a hand-built context.

---

## 5. The bare-bail returns — 31, not 118

Cross-referencing #225's inherited 118-site classification (`c1c2.json`)
against this lane's reachability:

| | c1 null-ptr | c2 bounds | c2 lookup-miss | unclassified | total |
|---|---|---|---|---|---|
| **live (needs treatment)** | 10 | 1 | 7 | 13 | **31** |
| static-reach only | 0 | 1 | 0 | 9 | 10 |
| unreachable (dies with its fn) | 35 | 12 | 0 | 30 | **77** |

The 31 live sites are confined to seven functions: `jit_call_value` (10),
`jit_call_method` (9), `jit_typed_object_{get,set}_field` (5),
`jit_typed_object_alloc` (2), `build_option_object` (1),
`jit_schema_option_some` (1), `dispatch_borrowed_closure_via_trampoline_vm` (1).

Treatment per the #234 rulings, refined by what already exists:

- **c1 (corrupted-state guards) — no kind needed, but NOT already discharged.**
  The pattern is `set_jit_runtime_error(msg); ctx.pending_call_error = 1; return
  ERROR_PLACEHOLDER_BITS`. In the void monomorph the return disappears entirely
  and only the flag remains. **This refutes the implicit premise that c1 needs
  the kinds in the signatures first** — it does not; it is independent and could
  even land before the conversion, and §5.1 argues it should.

### 5.1 The zero-safety claim is a PRECONDITION, not a property (grill Finding 3)

The first draft asserted that `ERROR_PLACEHOLDER_BITS == 0` "is memory-safe
under every `NativeKind`" and moved on. That is true of **the value in
isolation** and false of the system. A `0` that lands in a slot stamped
`Ptr(HeapKind::Closure)` and later meets `drop_with_kind` is an `Arc` decrement
on a null pointer. The real safety argument rests on the emit side branching to
the deopt on `pending_call_error` **before** `write_place` runs
(`emit_pending_call_error_deopt`, `terminators.rs:2217`) — a property of the
**emit sites**, not of the value. #234 names this the bits==0 guard-bypass
hazard, and the first draft read as though it had been discharged.

**It has not been. Measured at HEAD: of the 31 live bare-bail sites, only 6
acquire `pending_call_error`. 25 do not**, and at least 12 of those
demonstrably `return TAG_NULL`, so a tag word reaches `write_place` today with
no deopt:

| function | live sites lacking the flag |
|---|---|
| `jit_call_value` (`ffi/control/mod.rs`) | `:538`, `:552`, `:601`, `:674`, `:725`, `:778`, `:835`, `:849`, `:895` |
| `jit_call_method` (`ffi/call_method/mod.rs`) | `:607`, `:639`, `:657`, `:668`, `:686`, `:715` |
| `jit_typed_object_get_field` / `_set_field` (`ffi/typed_object/field_access.rs`) | `:27`, `:34`, `:74`, `:81`, `:87` |
| `jit_typed_object_alloc` (`ffi/typed_object/allocation.rs`) | `:70`, `:98` |
| `build_option_object` / `jit_schema_option_some` (`ffi/typed_object/option.rs`) | `:85`, `:112` |
| `dispatch_borrowed_closure_via_trampoline_vm` (`ffi/control/mod.rs`) | `:360` |

**That table is the mechanism of #254 repro B, enumerated.** The nine
`jit_call_value` rows are precisely the paths by which `TAG_NULL` reaches a
proven-`Int64` destination and gets `iadd`-ed. §10.1.1's "impossible by
construction" argument depends on every one of them acquiring the flag; until
they do, the argument describes the design, not the tree.

**Requirement**: all 25 acquire `pending_call_error` in the same slice, and the
§10.2 emit-time assertion additionally checks that no converted entry point has
a return path that neither transfers a valid value (O2) nor sets the flag.

**Recommendation — land c1 first, as its own commit.** It is separable (needs no
kinds), it closes a live memory-safety hazard on today's tree, and it makes the
conversion's diff smaller and easier to review. The rest of the slice stays
indivisible per §9; this one piece genuinely is not, and there is no reason to
hold a hazard closure behind a design review.
- **c2 (semantic nulls) — needs the kind, and only now becomes enumerable.**
  Each site returns its destination kind's §3.1 encoding: null pointer for
  heap `T?`, canonical sentinel NaN for `number?`, widened niche for narrow
  scalars. With the monomorphs in place the destination kind is the monomorph's
  identity, so each site's correct encoding is decided at compile time rather
  than chosen at runtime.

---

## 6. The closure-carrier producer flip

### 6.1 The standing metadata lie, located — and it is a two-stamp duality

`box_function` has **two producers**, both in `mir_compiler/ownership.rs`, and
they emit bit-identical values under **two different kind stamps**:

| producer | emit | kind stamp | honest? |
|---|---|---|---|
| `MirConstant::Function(name)` | `ownership.rs:960` | `NativeKind::UInt64` (`rvalues.rs:634`, `types.rs:2175`) | yes — "opaque raw bits, no classification" |
| `MirConstant::ClosurePlaceholder` | `ownership.rs:1012` | `NativeKind::Ptr(HeapKind::Closure)` (`rvalues.rs:647`, `types.rs:2177`) | **no** |

`box_function` is `make_tagged(TAG_FUNCTION_BITS, fn_id)` — a NaN-boxed tag
word, `0xfffd_0000_0000_00<fid>`. The `UInt64` stamp is merely untyped: because
`UInt64` is not a heap kind, refcount machinery correctly leaves it alone. The
`Ptr(HeapKind::Closure)` stamp is the dangerous one — it tells every consumer
that trusts the kind (`release_old_value_if_heap`, `emit_drop`, capture retain)
that the slot holds a pointer to a refcounted heap object, so those consumers
will do `Arc` pointer arithmetic on a tag word. The `rvalues.rs:647` comment
even asserts the slot "carries `Arc<HeapValue::ClosureRaw>` bits per
§2.7.11/Q12", which is precisely what it does not carry.

The `is_inline_function` guard papers over the consumers that remember to
check; it cannot protect the ones that trust the kind. **This is an unnamed
carrier duality of exactly the shape ADR-020 §3.1.1 requires to be named with a
classification rule** — except here the two carriers are bit-identical and the
"rule" selecting between them is which `MirConstant` variant the MIR happened
to use, which is source-shape-selected semantics (CLAUDE.md §Forbidden). Both
stamps die with the flip: one carrier, `Ptr(HeapKind::Closure)`, actually
pointing at a closure record.

**This is not latent.** Three lines of module-scope Shape:

```shape
let g = |x| x + 1
let h = |y| g(y) + 1
print(h(1))
```

VM `3`; JIT **SIGSEGV** (rc=139, gdb shows the fault inside unsymbolized
Cranelift-emitted code). Filed as **#254**. **This shape is not in the corpus**
(no corpus program segfaults: `SYN__first-class-closure-{dispatch,return}`,
`ACC__functions__segfault-repro`, `ACC__jit-compilation__large` all rc=0;
`SYN__closure-infn-tagnull` rc=1) and it is distinct from #219, which requires a
closure declared inside a function and passed as an argument.

#### #254 is TWO defects — the discriminator is ESCAPE (my "scope" framing was one step short)

The supervisor asked me to run the annotation control against repro B
specifically, on the hypothesis that A is the carrier defect and B is a §5.1 c1
bail (a `TAG_NULL` returned without the flag, then `+ 1`). I ran that control
**and** a scope control. The result splits the defects on a **different axis
than either of us proposed**:

| variant | module scope | inside `fn main()` | annotated |
|---|---|---|---|
| **A** `g=\|x\| x+1; h=\|y\| g(y)+1` | **rc=139 SIGSEGV** | **correct, rc=0** | still rc=139 |
| **C** `g=\|x\| x+1; c=5; h=\|y\| g(y)+c` (capture in the *caller*) | **rc=139 SIGSEGV** | **correct, rc=0** | — |
| **B** `c=5; g=\|x\| x+c; h=\|y\| g(y)+1` (capture in the *callee*) | `TAG_NULL+1`, rc=0 | **`TAG_NULL+1`, rc=0** | still `TAG_NULL+1` |

Two conclusions, both measured:

1. **The two SIGSEGV variants (A and C) require an ESCAPING closure slot.**
   Module scope was the visible correlate and both the supervisor and I stopped
   there; the measured condition is escape. The identical shape inside
   `fn main()` is safe because fn-local closure slots take the **stack** path
   (`statements.rs:652` → `emit_stack_closure`, no retain/release). Module-scope
   bindings escape by definition, which is why every module-scope repro faults —
   but so would any other escaping form.
2. **Repro B is NOT module-scope-conditioned.** It reproduces identically inside
   `fn main()`. So the narrowing that holds for A and C does **not** hold for B,
   and B is a genuinely separate defect with a wider blast radius — every
   closure that captures and is then called from another closure, at any scope.

**Neither variant is #257's class.** Adding a type annotation rescues neither,
whereas the annotation control is exactly what rescues the seven §4.0 `Array`
shapes. So both are **present-and-wrong** stamps, not absent ones.

**But my next inference did not follow, and I am withdrawing it.** I wrote:
annotation rescues neither, therefore both are wrong stamps, therefore *c1 will
not close B*. The first two steps hold; the third does not follow from them. The
annotation control distinguishes an **absent** stamp (#257) from a **wrong** one
(#254). It says nothing about whether a **consumer-side stop** suppresses the
symptom — and that is exactly what c1 is. c1 does not fix a stamp; it makes the
consumer refuse to hand back a usable value.

**The cheap resolution the supervisor proposed, carried out:** the site that
fires for B is `ffi/control/mod.rs:872` (established under gdb), and **`:872` is
one of the eight paths named in #259** (`control/mod.rs:614, 651, 762, 798, 815,
872, 886, 932`) that return `TAG_NULL` **without** setting `pending_call_error` —
confirmed at HEAD: no `pending_call_error` assignment appears anywhere in that
arm. So c1, which makes exactly those paths set the flag, would cause the emit
side to deopt at `emit_pending_call_error_deopt` **before** `write_place`, and no
`TAG_NULL` would reach the destination.

**Therefore the likely outcome is that c1 closes B's SYMPTOM while leaving its
CAUSE** — B becomes correct-via-deopt, and the carrier lie waits for the §6.2
flip. Stated as the expected outcome rather than a settled fact, because only
observing B after c1 lands is decisive. Two consequences for the implementing
lane:

- **The c1 commit's acceptance must not claim it fixed #254.** If B goes green
  after c1, that is a deopt, not a repair, and the §10.1 fixture for B must
  additionally assert native execution (via `--native-witness`) so a
  correct-but-interpreted result cannot be mistaken for a converted channel.
- **Do not be surprised when B goes green early.** A and C will not.

What distinguishes B from A is *which* closure holds the capture (callee vs
caller) — see the measured mechanism below.

**Consequence for §6.2's consumer inventory — I claimed a missing consumer and
then WITHDREW it.** I reasoned from the module-scope conditioning that the
module-binding read path must be an uncovered consumer of the mis-stamped
carrier. That inference rested on the module-scope framing, which the measured
mechanism below refutes: the condition is **escape**, not module binding. There
is no missing module-binding consumer; the module-scope programs fault for the
same reason any escaping closure would.

The consumer inventory that survives is the producer-side one in §6.2, plus the
mask-driven retain loop at `mir_compiler/statements.rs:1214/1233/1243` — which is
**not** a kind-stamp consumer at all (it reads neither stamp) and so was
correctly absent from a list of stamp consumers. It belongs to the inventory as a
**layout** consumer, which is a different and previously unnamed category.

#### The mechanism — MEASURED, and my hypothesis was WRONG

I hypothesised that the capture path retains the captured slot *according to its
stamped kind* `Ptr(Closure)`. **Refuted under gdb.** The retain loop is driven
**solely** by `ClosureLayout::heap_capture_mask`
(`mir_compiler/statements.rs:1214/1233/1243`, faulting `atomic_rmw` at `:1255`).
It never calls `operand_native_kind` and reads **neither** kind stamp. My
"consumes the `Ptr(Closure)` stamp" chain does not exist.

**The real mechanism, and it is a better argument for the flip than mine was:**

- A **non-capturing** closure is `box_function(fid)` — a NaN-boxed tag word.
- `ClosureLayout` is **shared with the VM**, where a closure is always
  `Arc<ClosureRaw>`. It therefore classifies a captured closure as
  `FieldKind::Ptr` and sets its heap-capture-mask bit.
- So **capturing a non-capturing closure retains a tag word.** The layout says
  "refcounted pointer" because on the VM side it always is; the JIT put a tag
  word there instead.

Confirmed by a falsifiable prediction: inserting an extra closure ahead shifts
the `fid`, and the fault address moved `…00c2` → `…00c3` exactly as predicted.

This **strengthens §6.2 rather than weakening it.** The defect is not a stamp
being misread by a consumer — it is **two carriers that no single `ClosureLayout`
can describe**, which is precisely why ADR-020 §3.4 rules that the one carrier is
the VM's `Arc<ClosureRaw>`. A layout shared across two tiers is only coherent if
the tiers agree on the representation. They do not, and that is the thing this
ticket fixes.

**The minimal repro is smaller than the one in §6.1 above: two `let`s and no call
at all.**

```shape
let g = |x| x + 1
let h = |y| g(y) + 1
```

This segfaults with nothing invoked — the fault is in the *allocation* of `h`,
not in any dispatch. Use this form in the fixture; it removes `print`, the call,
and the arithmetic as suspects.

**Correction to §6.1's framing, which I inherited and should not have:** the
condition is **not** module scope. The in-function version is safe because
fn-local closure slots take the **stack** path (`statements.rs:652` →
`emit_stack_closure`, documented at `:940` as doing no retain/release). That is
**escape analysis**, not a module-binding carrier. A module-scope binding escapes
by definition, which is why every module-scope repro faults; but any escaping
closure slot reaches the same code. The supervisor's module-scope narrowing and
my §6.2 "the module-binding read path is a missing consumer" inference are both
**withdrawn** — the missing-consumer claim was reasoning from a false premise.

**Variant B is the mirror image, not a different family.** Where A carries a tag
word under a layout that says `Ptr`, B carries **genuine heap-closure bits under
the `UInt64` stamp** (`rvalues.rs:634`, the `MirConstant::Function` arm) into
`jit_call_value`, whose `UInt64` handler recognises only `TAG_FUNCTION` or
`unified_box(HK_CLOSURE)` — not the VM's raw `Arc`. It falls to the surface arm
at `ffi/control/mod.rs:872` and returns `TAG_NULL`. Two directions of the same
duality: **one has a guard and returns a wrong value, the other has no guard and
faults.**

#### REFUSED SHORTCUT — record it before someone finds it

Hardening the mask-driven retain at `statements.rs:1243` to skip tag-shaped words
**suppresses the segfault and must not be taken.** It leaves a closure that the
layout says is refcounted but which is never retained and never released — the
refcount lie behind a repaired guard. It converts variant A (a crash, which is
loud) into variant B (a silent wrong answer, which is not). That is a strict
worsening dressed as a fix, and it is the §Forbidden walk-back shape: the
dynamic-path defect kept alive under a guard that makes it invisible.

### 6.2 The flip

Per ADR-020 §3.4 as amended, the one carrier is the VM's existing
`Arc<HeapValue::ClosureRaw>` / `Ptr(HeapKind::Closure)`; the JIT adopts it.
The inherited 3-edit recipe from #227 slice 2 (verified-buildable, reverted
only because consumers could not execute the record) is:

1. An `arc_closure_constant` `OnceLock` pool mirroring
   `crate::ffi::string::arc_string_constant` (used at `ownership.rs:933,949,989`)
   — a leaked permanent share per §3.4's immortality ruling: no header flag, no
   RC hot-path branch, the count simply never reaches zero.
2. Both `ownership.rs` emit arms — `MirConstant::Function` (line 960) and
   `MirConstant::ClosurePlaceholder` (line 1012) — plus the latter's
   exhausted-side-table fallback (line 1022, currently `iconst 0`, which is a
   bare null in a slot stamped `Ptr(Closure)`).
3. The two kind stamps (`rvalues.rs:634` `UInt64` and `rvalues.rs:647`
   `Ptr(Closure)`; mirrored at `types.rs:2175,2177`) collapse to the single
   `Ptr(HeapKind::Closure)` — which then becomes *true* rather than a lie.

**Why it failed in #227 and why it succeeds here.** The revert's stated cause
was that `dispatch_borrowed_closure_via_trampoline_vm` cannot execute a
zero-capture closure record, and that closure-callee refcount discipline
differs from string constants. Both are consumer-side facts. #227 could not
change the consumers because the consumers are reached through the kind-blind
`-> u64` value-call return — which is exactly what this ticket converts. The
producer flip and the channel conversion are **one edit or neither**; splitting
them is what produced the malloc corruption. This is the concrete mechanism
behind the ticket's "do not split" instruction, and I agree with it.

Consumer inventory to migrate in the same slice: `jit_call_value`
(`ffi/control/mod.rs:532`, the `is_inline_function` arm), both trampoline
dispatchers (`ffi/control/mod.rs:133,258`), and `unbox_function_id`
(`ffi/value_ffi.rs:310`). After the flip `rustc` enumerates any straggler when
`box_function` / `is_inline_function` / `TAG_FUNCTION_BITS` /
`unbox_function_id` are deleted — that is the intended enumeration mechanism
and the reason the deletion belongs in this slice rather than a follow-up.

---

### 6.3 Implementation path for the flip — feasibility established, construction recorded

Established at `af64e8be` before starting the edit, so the next actor does not
re-derive it.

**1. The layout is available at emit time.** The flip needs a `ClosureLayout` for
a `fid` at JIT *compile* time, because `ownership.rs` emits an `iconst` of the
carrier value. `closure_function_layouts: HashMap<u16, Arc<ClosureLayout>>` is a
field on `MirToIR` (`mir_compiler/mod.rs:447`, used at `statements.rs:648,670`),
so the emit site can consult it directly.

**2. A zero-capture layout is constructible for named functions.**
`MirConstant::Function(name)` names a *function*, not a closure body, so it has
no registered layout — this is the gap the pool must fill.
`ClosureLayout::from_capture_types(&[], &[])`
(`shape-value/src/v2/closure_layout.rs:999`) is well-formed on empty slices: the
length-equality and ≤64 preconditions hold, there is no `ConcreteType::Void` to
panic on, and all three masks come out 0.

**3. The record construction mirrors the VM's**, at
`shape-vm/src/executor/call_convention.rs:1327-1332`:

```rust
let ptr = alloc_typed_closure(fid, /*type_id*/ 0, &layout_arc);
// zero captures: no write_capture_raw_u64 calls
let block = OwnedClosureBlock::from_raw(ptr, layout_arc);
// -> HeapValue::ClosureRaw(block) -> Arc::new -> leak ONE permanent share
```

Immortality per ADR-020 §3.4 is one leaked share — no header flag, no branch on
the RC hot path — mirroring `arc_string_constant`
(`ffi/string.rs:213`), which holds a pool share and hands out an incremented
one.

**4. The #227 slice-2 blocker looks NARROWER than recorded, and this needs
confirming before it is trusted.** The revert reason was recorded as
"`dispatch_borrowed_closure_via_trampoline_vm` cannot execute a zero-capture
record". Reading that consumer (`ffi/control/mod.rs:259-365`), **nothing rejects
zero captures**: `no_cell_captures` is trivially true with all masks 0,
`total_args` reduces to `arg_pairs.len()`, and the native path is taken whenever
the function table has a non-null entry for the fid. The capture loop
`for i in 0..layout.capture_count()` simply does not execute.

So the blocker is more likely to be the **function-table entry for a named
function's fid**, or a record minted without a valid `TypedClosureHeader`, than a
structural inability to dispatch zero captures. **Do not treat this paragraph as
a refutation** — it is a reading, not a measurement, and the #227 lane had the
failing artifact in hand while this is inference from source. Confirm by
constructing the record and calling it before building anything on top; if the
consumer does reject it, the reason will be visible immediately and is the thing
to fix first.

## 7. The third carrier

`jit_make_closure` (`crates/shape-jit/src/ffi/object/closure.rs:40`) produces
`unified_box(HK_CLOSURE)` — a third function-value carrier beside the VM's
`Arc<ClosureRaw>` and the JIT's `box_function`. It is emitted from the legacy
ARM-3 fallback at `crates/shape-jit/src/mir_compiler/statements.rs:821` (fully
qualified deliberately: several files in this tree are named `statements.rs`,
and a bare cite is a deletion hazard).

**RATIFIED: delete the fallback and `jit_make_closure` outright; do not migrate
its error returns.** But the warrant is the structural argument below, **not**
my corpus sweep — that distinction is load-bearing and the doc previously had it
backwards.

### 7.1 Why my original evidence does not support the conclusion

I wrote "0 hits across 481 corpus programs and 0 across four falsifiers" and
concluded no producer exists. **That measurement is close to powerless as
stated**, and I should have caught it: most closure-bearing corpus programs never
reach `ClosureCapture` lowering at all, for reasons unrelated to closures (§1.2 —
only 121/481 execute any native code, and the whole-program bail rate dominates).
A sweep with no per-program positive control reports zero executions whether or
not the arm is reachable. It is an absence claim measured with an instrument that
cannot distinguish absence from non-arrival.

The refuting investigation did it properly: gdb breakpoints on the symbol with
`jit_finalize_heap_closure` as a **live positive control**, observing ~10,001
escaping closure allocations at **100% ARM 2, 0% ARM 3** — including module
scope, which none of my falsifiers covered.

### 7.2 The structural argument — the actual warrant

ARM 3 fires only when `closure_function_layouts.get(&fid) == None` **and** the
slot escapes. Five single-writer links make a missing layout impossible for a
real `fid`.

The critical part is that **the documented escape hatch does not exist.**
`crates/shape-vm/src/bytecode/core_types.rs:677` claims:

> *"serialized — programs loaded from disk fall back to the FFI path."*

That is false. The same serde boundary that drops the layouts also drops the
MIR: `mir_data` and `top_level_mir` are both `#[serde(skip)]`, and
`linker.rs:738` nulls `mir_data`. Every JIT path errors without MIR. **No MIR, no
`ClosureCapture` lowering, no ARM 3.** The one scenario the codebase advertises
as reaching the fallback cannot reach the JIT at all.

### 7.3 Required with the deletion

1. **Delete the two stale artifacts in the same commit.** Both are verified
   present:
   - the false comment at `core_types.rs:677` (quoted above);
   - the `#[deprecated]` note at `ffi/object/closure.rs:36-40`, which states the
     function "exists only to service the legacy non-layout fallback" and that "a
     follow-up phase can delete this FFI once all closure functions are
     guaranteed to have a registered `ClosureLayout`."

   Leaving either re-seeds the belief that a non-layout program can reach the
   JIT. The `#[deprecated]` note is the more dangerous of the two because it
   reads as a live TODO with a precondition that has **already been met**.
2. **Record the untested surfaces rather than implying totality.** Neither the
   corpus sweep nor the gdb investigation covered: the REPL, `wire-serve` /
   `@remote`, `shape build`, snapshot resume, annotation handler bodies, async
   closures, or multi-file imports. The structural argument is what covers them —
   it is a property of the lowering, not of a sample — but the deletion record
   should say which surfaces were reasoned about rather than observed.

If the implementing lane finds a producer, that is a genuine refutation and
should come back as one rather than be resolved by keeping the carrier.

## 8. The ratchet, in the same slice

The mechanism already exists and already has a dialect row. `just
check-no-dynamic` (`scripts/check-no-dynamic.sh`, 111 lines) is a per-symbol
**monotonic-non-increasing** check against the frozen baseline at
`docs/check-no-dynamic-baseline.txt` (format: `<limit>\t<ripgrep PCRE>\t<note>`;
scope `crates bin tools extensions`; docs trees deliberately unscanned so this
design file may name the symbols freely). #226 already seeded:

```
52	^(?!\s*(//|///|\*)).*\bTAG_BOOL_(TRUE|FALSE)\b	… (#239 drives it to 0). SHRINK-ONLY
0	^(?!\s*//).*\bTAG_UNIT\b	ADR-020 §3.3/§6 (#224)
```

The gate passes at HEAD (`bash scripts/check-no-dynamic.sh`, exit 0). Note the
row counts matching **lines** with comment-prefixed lines excluded — a raw
occurrence grep gives 56 for `TAG_BOOL_*` where the gate sees ≤52. Use the gate,
not a grep, when setting new limits.

This slice adds rows at their current counts and drives them, plus the existing
`TAG_BOOL_*` row, to 0 in the **same commit** as the deletion:

`TAG_NULL`, `TAG_NONE`, `TAG_HEAP_BITS`, `TAG_INT_BITS`, `TAG_BOOL_BITS`,
`TAG_NONE_BITS`, `TAG_FUNCTION_BITS`, `TAG_BASE`, `TAG_MASK`, `TAG_SHIFT`,
`TAG_NUMBER`, `TAG_DATA_ROW`, `PAYLOAD_MASK`, `make_tagged`, `is_tagged`,
`get_tag`, `box_function`, `is_inline_function`, `unbox_function_id`,
`box_number`, `unbox_number`, `is_number`, `unified_box`, `is_heap`.

Raw occurrence counts at HEAD in `crates/shape-jit/src` (non-comment lines,
for scale only — set the real limits from the gate): `TAG_NULL` 229,
`box_number` 140, `is_number` 80, `unified_box` 25, `is_inline_function` 12,
`box_function` 8, `is_tagged` 7, `TAG_NONE` 5, `make_tagged` 5, `get_tag` 4,
`TAG_FUNCTION_BITS` 3.

**The ratchet must also cover raw bit patterns, not only symbol names**
(§3.5.2). `emit_index_to_i64` reconstructs the deleted `is_tagged` dispatch from
an `iconst` of `0xFFF8_0000_0000_0000` without naming a single ratcheted symbol;
every row above would pass while that code stands. Add at minimum a row
matching the NaN-box tag bit-pattern literal, e.g.

```
<n>	0x[fF]{3}[89a-fA-F]_?0{4}_?0{4}_?0{4}	ADR-020 §6 — NaN-box tag bit pattern as a literal; the dialect reconstructed as an iconst
```

with the limit set from the gate's own count (three non-comment sites at HEAD,
one of them live), then driven to 0. The name rows and the bit-pattern row are
complementary: the first stops the dialect returning by name, the second stops
it returning by value. **This is the single most important addition to the
ratchet design**, because it is the only row that would have caught the one
emit-layer instance that survives the entire FFI deletion.

The ratchet must **bite before it is trusted**: the slice includes a proof run
showing that re-adding one deleted symbol fails the gate. A ratchet that has
never been observed to fail is an assertion, not a gate — and this one has
never failed, because until now nothing could be ratcheted while the dialect
was load-bearing.

---

## 9. Slicing — one slice, with a named precondition

I concur with the ticket: this does not split. The measured reason is §6.2 —
the producer flip and the return-channel conversion are the same edit, and
#227 slice 2 is the recorded experiment showing that doing one without the
other produces heap corruption rather than a clean intermediate state.

The scope that makes one slice viable is the partition: ~9 functions converted
and ~62 deleted is not a 108-function conversion. Deletion is the bulk of the
diff and it is mechanical (rustc enumerates).

**Named precondition, and the one thing I would not proceed without.** The
`--mode jit` corpus exercises native code in only 121 of 481 programs. A
conversion of the value-call channel validated against that denominator is
validated against a quarter of the corpus. Before the conversion lands, the
slice must add corpus fixtures for the shapes that are currently invisible —
at minimum the three closure shapes in §6.1, which today produce a segfault, a
silent `TAG_NULL` arithmetic result, and a second segfault, and which **no
existing corpus program covers**. Landing the conversion first and the
coverage after would repeat the #227 slice-2 sequence.

---

## 10. Acceptance and gate design

### 10.1 The L106 one-liner is not sufficient

`let inc = |x| x + 1; print(inc(10))` **passes at HEAD** (prints 11, both
tiers). It was the right acceptance test for #227 slice 2, where the producer
flip broke it; it cannot detect the defects this ticket fixes. Proposed
acceptance set, all as corpus fixtures with VM/JIT differential:

| fixture | source | **form — binding** | HEAD behaviour | post-fix |
|---|---|---|---|---|
| `SYN__closure-calls-closure.shape` | §6.1 A (use the two-`let` no-call form) | **ESCAPING slot — required** | **SIGSEGV** | VM==JIT `3` |
| `SYN__closure-calls-closure-outer-capture.shape` | §6.1 C | **ESCAPING slot — required** | **SIGSEGV** | VM==JIT `7` |
| `SYN__closure-calls-closure-capture.shape` | §6.1 B | either (reproduces at both) | silent `TAG_NULL+1` | VM==JIT `7` |
| `SYN__datetime-method-native.shape` | §2 STAGE-F3 | any | whole-**program** deopt, 0 native dispatches | native, VM==JIT |
| `SYN__string-scalar-method.shape` | `s.length()` in a fn | any | whole-fn deopt | native, VM==JIT |
| `SYN__closure-l106.shape` | L106 | module scope | green | stays green (regression) |
| `SYN__string-eq-content.shape` | **already exists** on `main` (#232) | — | green | **reuse, do not duplicate** — extend with a `StringV2` operand, which deopts today (§2) and must go native |

**Fixture-form warning — do not "simplify" the first two into a function.** The
two SIGSEGV variants require the closure slot to **escape**; wrapped in
`fn main()` the slots take the stack path (`statements.rs:652` →
`emit_stack_closure`, no retain/release) and are correct on both tiers at HEAD.
A fixture written the natural way — inside a function — would pass today and
prove nothing, and the next person to tidy the corpus is exactly the person who
would rewrite it that way. Module scope is the *simplest* way to force escape,
not the condition itself (§6.1); any escaping form works. Repro B is the
exception: it reproduces in both forms, so its fixture may take either — but see
§6.1 on why B's fixture must additionally assert native execution.

### 10.1.0 EVERY row asserts native execution — the differential can no longer see the defect

**This requirement supersedes the "post-fix" column above for every row, not
just the positive ones.**

After #257, the native-dispatch rate is 11/482 corpus programs. With the native
tier inert, `--mode jit` and `--mode vm` run **the same interpreter**, so VM==JIT
agreement is nearly free and proves only that the program ran — not that it ran
natively and got the right answer.

**Measured proof that this is not hypothetical.** #257 flipped five known-red
entries to MATCH, and this lane initially reported that as five defects fixed.
All five were then checked with `--native-witness`: every one is
`program_fallback: jit-compile-error` with **zero native dispatches**. They match
because the JIT never runs. **That is the defect being masked, not fixed** — and
retiring those pins would silently re-arm five defects (two release-blocking:
#232's permission divergence and #219) the moment the native rate recovers.

This is the same shape as #224's original finding — *"whole program previously
bailed via unit-returning `fn main`, so VM==JIT trivially"* — which is what
unmasked #231 and #232 in the first place. The wheel has turned once around.

**Requirement.** Every acceptance fixture asserts `sum(native_dispatches) > 0`
under `--mode jit --native-witness`, enforced by
`scripts/check-jit-native-acceptance.sh` (`just check-jit-native-acceptance`).

**The two metrics are NOT interchangeable.** `program_fallback == null` and
`sum(native_dispatches) > 0` differ by an order of magnitude on the same sample
(16.7% vs 0%): a per-function bail leaves `program_fallback` null while nothing
runs natively. **The gate requires non-zero DISPATCHES.** Asserting the absence
of a program-level bail would pass while the tier is inert.

Until the conversion lands this gate fails for every row, by design — it is the
conversion's acceptance criterion, not a merge blocker for the earlier steps.

#### The re-derivation command for the headline claim

"The conversion restored the native rate" is the positive claim step 4 will want
to make, and per §10.4.1 a positive claim about an artifact ships with the
invocation that reproduces it, not a summary of the result. **Do not report this
number from the implementing lane's run alone.**

```bash
# 1. Build from the commit under test. A stale binary lies in the MASKING
#    direction — it "shows" defects already fixed and native rates already
#    restored. This is not hypothetical: a 7-minute-stale worktree binary was
#    caught misreporting during step 2.
direnv exec /home/dev/dev/shape-lang cargo build --release --bin shape --jobs 4

# 2. Per-program native dispatch across the whole corpus.
cd tools/vmjit-diff/corpus
for f in *.shape; do
  timeout 25 ../../../target/release/shape run --mode jit --native-witness /tmp/w.json "$f" >/dev/null 2>&1
  python3 -c "
import json
try:
    d = json.load(open('/tmp/w.json'))
    print(sum(x.get('native_dispatches', 0) for x in d.get('functions', [])))
except Exception:
    print(-1)"
done | awk '$1+0>0 {n++} END {print n\" programs with >=1 native dispatch\"}'

# 3. The acceptance set specifically.
just check-jit-native-acceptance
```

Reference points, both measured on this branch: **121/481 before #257**,
**11/482 after**. Step 4 must move the second number, and the mover must be
verified by someone who did not produce it.

Two load-bearing *positive* fixtures, which is the pair that proves the
conversion delivered a typed channel rather than renaming the untyped one:
`SYN__string-scalar-method` (STAGE-StringJIT gone) and
`SYN__datetime-method-native` (STAGE-F3 gone — and this one is the stronger
signal, since the program currently records **0 native dispatches**, so its
witness going non-zero is unambiguous). Without both, a conversion that keeps
every deopt in place would pass the whole suite.

### 10.1.1 Why #254 repro B becomes impossible by construction

This is the claim the design has to be able to make, so state it precisely.

**Today.** `let c = 5; let g = |x| x + c; let h = |y| g(y) + 1; print(h(1))`
prints `-1407374883553279`. The chain: `g(y)` is a value call; `jit_call_value`
returns `u64`; on its failure path that `u64` is `TAG_NULL`
(`0xfffb000000000000`); the emit site stores the return **verbatim** into `h`'s
destination slot via `write_place` (`terminators.rs:2225`); the slot's proven
kind is `Int64`; the following `+ 1` is therefore emitted as a native `iadd` on
raw `i64`; and `0xfffb000000000000 + 1` is a perfectly well-formed integer. The
program exits 0 with a wrong number. **Nothing in that chain is a bug in any
single component** — each step is locally correct given its inputs. The defect
is that the channel between them carries no type, so a value that means "no
value" is indistinguishable from a value that means `-1407374883553280`.

**After the conversion.** The destination kind is `Int64`, so the emit site
calls the `i64` monomorph, whose Rust signature returns a raw `i64` and which
has no representation for "null" — `TAG_NULL` is not a value it can produce,
because `TAG_NULL` no longer exists (§8 ratchets it to 0). The failure path is
the §2.1 error channel instead: `pending_call_error = 1` plus
`ERROR_PLACEHOLDER_BITS` (`0`), and the emit site's already-present
`emit_pending_call_error_deopt` (`terminators.rs:2217`) branches to the deopt
**before** `write_place` runs. The wrong value is never stored, so the `iadd`
never sees it.

Three independent properties each block the defect, which is what "by
construction" has to mean:

1. **No universal null word exists.** ADR-020 §3.1 replaces it with per-type
   niches; an `int` destination has no null encoding at all (the presence pair
   is #229's, and until then `int` is simply not nullable). There is no bit
   pattern for the failure path to smuggle in.
2. **The failure path is out-of-band.** It is a flag plus a deopt branch, not a
   return value, so it cannot be mistaken for data no matter what bits accompany
   it — and `ERROR_PLACEHOLDER_BITS == 0` is memory-safe under every kind if it
   ever is stored.
3. **The return type is the destination type.** A monomorph returning `i64`
   cannot return a float's bits or a pointer's bits; the mismatch that produced
   #254 is not expressible in the signature.

Repro A (SIGSEGV) is blocked by the same change at a different point: the
`Ptr(HeapKind::Closure)` stamp becomes true (§6.2), so the capture-retain that
currently does `Arc` arithmetic on a tag word operates on a real closure record.

**This is why the fixtures must be in the corpus, not just asserted here.** The
argument above is only as good as the property that no other path can
reintroduce a universal sentinel — and the §8 ratchet, not this paragraph, is
what enforces that going forward.

### 10.1.2 The performance consequence — MEASURED, and worse than this section first said

**Original text said the native rate "falls before it rises". That is true and
materially understates it. Corrected with the measurement.**

#257 landed (deleting the fabricated `Int64` destination default). Measured on
the full corpus, branch vs the `dac5fe7e` control, both binaries built from
source:

| | control | after #257 |
|---|---|---|
| programs with ≥1 native dispatch | **121 / 481** | **11 / 482** |

**A 91% reduction. The native tier is effectively inert in the intermediate
state** — not "reduced". Even L106 (`let inc = |x| x + 1; print(inc(10))`), the
simplest closure call in the language, now executes zero native dispatches. A
60-program sample taken independently by the supervisor found **0 of 60**
executing any native code.

This is correct — an honest bail beats a silent wrong answer, and CLAUDE.md's
"surface-and-stop, not force" says so. The owner accepted it. But it must be
written with its true magnitude, because:

1. **Any JIT-shaped benchmark run in this window measures the interpreter.**
2. **The corpus differential is degraded as a gate** for exactly as long as this
   lasts (§10.1.0, §10.3) — it can no longer distinguish "correct because the
   channel works" from "correct because nothing ran".

**The recovery is NOT a natural rebound**, and describing it as "rises" invited
that reading. It is two pieces of deliberate work: this ticket's conversion, and
stamping the kinds that are missing — `well_known_method_return_kind` coverage,
#240 territory (generated-node provenance, comptime return pinning) and #262.
Nothing recovers on its own.

**Do not close the guard gap by widening the allowlist** — that is §4.0.3's
sixth point fix, and it would restore the native rate by restoring the defect.

### 10.2 Carrier-level unit coverage that would actually bite

The inherited warning is correct — **and the mechanism is worse than "the tests
call the machinery"**. A grill refuter established that the FFI wiring is
**structurally absent** from every test-built JIT module: FFI symbols reach
emitted code only via `register_ffi_symbols` (`compiler/setup.rs:65`), which is
called only from `JITCompiler::new`, while every test `JITBuilder` is built with
`default_libcall_names()` and never sees it, and every test `declare_function`
uses `Linkage::Local`. **Test-emitted code cannot link an FFI function at all.**
Zero of 834 shape-jit unit tests compile-and-run a Shape program in the default
gate. A green `--lib` suite is not weak evidence about the value-call carrier;
it is *no* evidence, because the carrier is not present in the binary under
test.

**Demonstrated by a natural experiment, not just argued.** The #232 fix (merged
`4b773a0d`) reports **503 passing** in shape-jit `--lib` — and a string equality
that compared *pointers instead of content* had shipped and stayed green through
all 503. The fix had to add its own production-path coverage because none
existed. Those 503 tests did not merely fail to catch the defect; they were
structurally incapable of observing it. Cite alongside the `jit_v2_string_eq`
evidence (§3.5.3), where seven unit tests pass by calling a function that no
emitted code can reach.

Three coverage requirements:

1. **Producer/consumer kind-agreement assertion.** For every converted entry
   point, an emit-time assertion in `MirToIR` that the chosen monomorph matches
   the destination slot's **proven** kind — `slot_kind_for_local(...)`, never
   `slot_kind_of` (§4.0) — and, per §4.2 O2, that a heap-kinded destination gets
   the `_ptr` monomorph. This is the check the `ClosurePlaceholder` stamp/emit
   pair would have failed since it was written, and the check that would have
   caught all seven §4.0 Array shapes.
2. **Un-gate `closure_dispatch_regression_tests` — a cheaper bite than I
   assumed existed.** That module (34 tests, `mir_compiler/`) executes real
   programs and asserts on returned values; its header records the pre-fix
   symptom as returning `Null` instead of `Integer(6)` — the exact class this
   ticket owns. It is `deep-tests`-gated on a **stale rationale**:
   `mir_compiler/mod.rs:53-61` justifies the gate by saying
   `ffi/control/mod.rs:171::jit_call_value` is `todo!(...)` and to "re-enable
   when the kinded value-call ABI lands". **It landed** — `jit_call_value` has a
   real body at `ffi/control/mod.rs:532` at HEAD. Re-run and un-gate it in this
   slice; it is materially cheaper than the corpus differential and it bites at
   the right layer.
3. **End-to-end differential through the CLI** — the §10.1 corpus fixtures. A
   carrier defect that only manifests when Cranelift-emitted code retains a
   value is unobservable from any in-process unit test that builds its own
   context.

**Gate arithmetic that has to change, and it is the reason this class ships.**
Verified in the justfile: `just test` (Tier 2, line 64) passes
`--features shape-jit/deep-tests`; **`just test-all` (line 88), the merge
blocker, does not** — it passes only the `shape-vm`/`shape-runtime`/`shape-ast`
deep features. So every test capable of observing this class is **absent from
the merge gate**. Un-gating per (2) is necessary but not sufficient; `test-all`
must also carry `shape-jit/deep-tests`, or the tests exist and the gate still
cannot see them.

State in the slice record that the `--lib` suite passing is *not* evidence, so a
future reviewer does not read a green suite as coverage.

### 10.3 Standing gates

**READ THIS FIRST: the corpus differential is DEGRADED as a gate until the
conversion restores the native rate (§10.1.2).** With 11/482 programs executing
native code, `--mode jit` and `--mode vm` run the same interpreter for almost
every program, so a MATCH is nearly free. Five known-red entries flipped to MATCH
under #257 and all five were verified bail-masked — zero native dispatches
(§10.1.0). **A green corpus in this window is not evidence about the value
channel.** Pair every differential claim with
`just check-jit-native-acceptance`, and treat a MATCH on a program with zero
native dispatches as "not yet tested", not "passing".


Full 481-program corpus differential, 0 unexpected. `known-red.json` has already
moved under this design: the `jit-vm-permission-check-divergence` class was
retired by #232, so the baseline this slice diffs against is **not** the one §1
was measured on — re-baseline before believing any delta. #219's
`closure-infn-stack-pointer-tagnull` entry should close if this conversion fixes
it, but per §6.1 that is now two defects with different scope conditions, so
verify each rather than assuming one entry covers both; `just verify-merge`;
`just check-clean`; `just check-no-dynamic` with the new names and a proven
bite. Rebuild `cargo build --release --bin shape` before any `run-diff.mjs`
invocation — `--fresh` does not rebuild, and stale binaries fail in the masking
direction.

---

## 10.4 Two process findings this document earned the hard way

Both belong in the ADR lift alongside §3.5.1's nine-rescues arithmetic. That
arithmetic tells you **which pattern to refuse**; these two tell you **how the
refusal gets circumvented in good faith**, which is the part that actually
happens.

### 10.4.1 Verifying a property of the artifact you HAVE, and reporting it as a property of the artifact you are DESCRIBING

Three instances in a single day, by three different people, none of them
careless:

| who | what was verified | what was reported |
|---|---|---|
| supervisor | `verify-merge` passed on the merged tree | that a commit message's description of that tree was accurate — it named a deletion that had not happened |
| this lane (§4.0) | a proof gate named `slot_kind_of` **exists** | that unproven emit sites **stop** — the function is a fabricating `unwrap_or(Int64)` and is never called on an FFI-return path |
| this lane (§7.1) | 0 executions across 481 corpus programs | that no producer **exists** — with an instrument that cannot distinguish absence from non-arrival |
| this lane (§10.1.0) | five known-red programs now produce **identical VM and JIT output** | that five defects were **fixed** — all five have zero native dispatches; they agree because the JIT never runs |
| supervisor | `verify-merge` reported **22/22 PASS**, on **three separate occasions in one day** | that a merge had **completed** — the tree still had unmerged files each time. The gate cannot detect an in-progress conflicted merge; only the absence of `UU` entries proves one finished. **This is the sharpest instance in the table, because unlike the others the gate IS the check** — it is the artifact whose entire purpose is to answer the question it was read as answering |
| this lane (§1.1) | a grep for *"is this field named anywhere"* returned 184 | that 184 symbols appear in a `builder.ins().call(...)` — the narrow claim is unsound, since a `FuncRef` can be selected into a local and called indirectly (142 false positives when measured) |

The general form: **a check was run, it passed, and its result was reported as
a different proposition than the one it tested.** Each check was real. None was
evidence for the claim it was offered for.

**Six instances, three people, under two days — four of them this lane's.** The
rate is the finding, not any individual case. Every one was made by someone
competent, unhurried enough to be doing careful work, and **holding the right
tool at the time**: this lane had the `--native-witness` tooling and checked tier
agreement without checking whether anything ran; the supervisor had
`verify-merge` and read its pass as proof that a commit message described the
tree.

#### The countermeasure — and it is NOT "be more careful"

State this plainly, because "be more careful" is the conclusion a reader draws
by default and it is the wrong one. Care is not the variable. All six were
careful.

> **The countermeasure that has actually worked, every time, is a second party
> going to the artifact instead of to the report.**

The evidence is in the table itself. Instance 4 — five known-red entries
reported as fixed when they were bail-masked — was caught only because the
supervisor **re-derived a number this lane had already reported**, on a freshly
built binary, rather than accepting it. That single re-derivation is the only
reason five pins were not retired and five defects, two of them
release-blocking, were not silently re-armed the moment the native rate
recovered. No amount of additional care by the reporting lane would have caught
it, because the lane's check *passed* — it was simply a check of a different
proposition.

Three consequences, all of which are structural rather than behavioural:

1. **Independent re-derivation is not distrust; it is the mechanism.** A fleet
   where the supervisor re-runs lanes' measurements is not expressing doubt
   about competence — it is the only control that has caught this class. Framing
   it as a trust question would remove the control on the grounds of politeness.
2. **A positive claim about an artifact must ship with the command that
   re-derives it.** Not a summary of the result: the invocation, so the second
   party can reproduce rather than re-implement. This is why §1 lists exact
   commands, §1.2 states the denominator, and §1.1.1 instructs the reader to
   distrust this document's own numbers under a named condition.
3. **State which artifact was examined and what would have made the check
   fail.** "The tests pass" is not a claim; "this binary, built from this
   commit, produced this output, and would have produced X if the defect were
   present" is. Where a check cannot fail in the presence of the defect —
   `--lib` for the value-call carrier (§10.2), a stdout hash for a SIGSEGV
   (§10.1), a corpus MATCH while the tier is inert (§10.1.0) — say so instead of
   reporting the pass.

This generalises past #239 and belongs in the ADR lift alongside §3.5.1's
nine-rescues arithmetic. The two are complementary and neither is sufficient
alone: **nine-rescues tells you which pattern to refuse; this tells you how the
refusal gets circumvented in good faith by people who are trying.**

### 10.4.2 A correction applied where it was noticed, not everywhere it applies

A self-audit of this document across its revisions found **two** places where a
superseded claim survived in a location the original correction did not reach:

- §3's bucket-(c) list still named `call_string_method` after §3.0.1 moved it to
  bucket (d) — Finding 1's exact contradiction, surviving in a second spot.
- §4.2's ownership assertion still specified `slot_kind_of(destination)` as the
  emit-time check — the fabricating function #257 refuted — **inside the section
  whose purpose is to prevent that class of error.**

Both were written by an author who had, in the same document, just finished
arguing against exactly this. That is the finding, and it is why §3.5.1's ruling
is *delete the mechanism* rather than *prove one more kind*: a point fix lands
where attention was, and attention is the scarce thing. The nine rescues in
§3.5.1/§4.0.3 were not carelessness either — each was a correct local fix.

**Operational consequence for this slice:** after every correction, grep the
whole document (and the whole tree) for the superseded claim rather than editing
the site that prompted it. Two of two corrections in this document needed that
and did not get it the first time.

## 11. Premises refuted

For the record, since each of these would have mis-scoped the work:

1. **"108 kind-blind entry points must be kind-threaded."** 83 dialect-touching
   `-> u64` functions exist; 21 are statically reachable; 12 execute. The
   conversion surface is ~9 functions and the rest is deletion.
2. **"The `r!()` keys are the complete callable FFI surface."** True as an upper
   bound, false as the working set: `jit_get_prop` and `jit_alloc_owned_mut_cell`
   are declared into every JIT function's IR and never called.
3. **"#226's reachable list is the work list."** Wrong in both directions (§3).
4. **"The 118 bare-bail returns split per #234."** 77 die with unreachable
   functions; 31 are live; and the c1 half needs no kinds at all because
   `ERROR_PLACEHOLDER_BITS == 0` is kind-agnostic by construction.
5. **"Per-kind monomorphization *or* an explicit kind parameter."** The second
   bucket is empty; a kind parameter would be a forbidden runtime discriminator.
6. **"L106 is THE acceptance test."** It passes at HEAD and cannot detect this
   ticket's defects.
7. **"The dialect is an FFI-function problem."** My own first framing, and it
   has a blind spot: the JIT emits tag logic directly into Cranelift IR, where
   it has no symbol and no `r!()` key (§3.5). The emit layer is nearly clean —
   10 sites, 8 of them prose in error strings — but one of the two real ones
   (`emit_index_to_i64`) survives both the FFI deletion and a name-only ratchet.
8. **A stale comment that would mislead the implementing lane.** The docstring
   on `compile_binop` (`rvalues.rs:1855-1868`) describes "an inline NaN-box
   dispatch — `Both-Number` … or `Both-Int` … i48 math" for dynamic arithmetic
   from CallValue-returned slots. **That code no longer exists**:
   `compile_binop_dynamic_arith` (`rvalues.rs:1974`) is now a bare `Err(...)`
   surface-and-stop. Anyone scoping from the comment would budget for an inline
   tag-dispatch conversion that is already done. Delete the stale paragraph in
   this slice.

## 12. Wanted from review

1. ~~**Ratification that the polymorphic bucket is empty**~~ — **REFUTED**
   (#257). The *rule* stands (no runtime kind parameter) and I still want it
   ADR-recorded; the *premise* that unproven sites already surface-and-stop was
   false. Rewritten as §4.0/§4.0.1: unproven destinations **must** surface-and-
   stop, at least three emit sites do not, and closing that plus deleting the
   seven `unwrap_or` defaults are **in-scope prerequisites** landing before the
   monomorphization. What I now want ratified is that ordering.
2. ~~**A ruling on the §9 precondition**~~ — **RATIFIED** (grill round 1,
   without reservation): corpus fixtures for the three closure shapes land
   before the conversion, in the same slice, and this is inside the mandate.
3. **Confirmation that `jit_make_closure` deletion needs no producer search
   beyond what §7 records** — I looked and found none, but absence of a
   producer is the kind of claim a second pair of eyes should try to break.
4. **Disposition of #254 (the §6.1 SIGSEGV)**, now filed and routed elsewhere.
   My reading is that it dies with the §6.2 carrier flip and cannot be fixed
   independently without re-creating the #227 slice-2 failure — a point fix that
   keeps `box_function` under a `Ptr(Closure)` stamp is patching the guard, not
   the lie. If another lane lands a fix first, this design's §6 needs re-basing;
   worth deciding which ticket owns the carrier before both move.
5. ~~Whether `box_string` / `unified_box` belongs here or in #228~~ —
   **RULED (grill round 1): SPLIT AT THE CHANNEL.** The return-channel
   conversion and the dialect deletion for `box_string` are **#239**. The
   `UnifiedValue`/`JitAlloc` HeapHeader merge and any object-layout change are
   **#228**. The boundary, stated here so #228's lane does not find it
   half-done: **#239 changes how the value is RETURNED; #228 changes how the
   object is LAID OUT.** Concretely — #239 converts `box_string`'s call sites
   so a string return travels as `*mut HeapHeader` in a `_ptr` monomorph
   (§4.1/§4.2) and deletes `unified_box`; #239 does **not** touch the header
   bytes, the refcount field, or the GC colour/buffered bits, which stay whole
   for #228. A `box_string` call site that needs a layout change to convert is
   out of scope and should be surfaced, not absorbed.
6. ~~**Ratification of the §3.5.1 ruling to DELETE the `Eq`/`Ne` fallthrough**~~
   — **RATIFIED**, with the reasoning to be lifted to ADR level. Since
   ratification a **fourth** incident has been point-fixed upstream of the
   fallthrough (#232's `both_string` guard, merged `4b773a0d`) while the
   fallthrough itself remains byte-identical at `rvalues.rs:2116-2124`. The
   prediction held on schedule. Four rescues here, five more in the §4.0.3
   allowlist: nine across two mechanisms.
7. ~~**Whether `jit_string_eq` should keep explicit kind-code parameters**~~ —
   **RESOLVED IN FAVOUR OF THE LANE POSITION (ruling reversed).** The fix gates
   on `both_string`, so both operands are proven `String` at the emit site and
   the codes are compile-time constants — exactly §4's "runtime discriminator
   where a static proof exists". No bounded exception, no creditor, no
   retirement row. The parameters are removed in a follow-up; **they are still
   present at `main`** (`ffi/conversion.rs:1162`), so the implementing lane
   should confirm the removal landed before assuming the signature.
8. **Disposition of the legacy v1-array emit chain** (`inline_array_get` and
   siblings, §3.5.2). I did not settle its reachability and I am not guessing.
   Either it is dead (my expectation — delete with bucket (c)) or it hides a
   live inline tag test on every array index.
9. ~~**the STAGE-F3 routing obligation as a live SIGSEGV**~~ — **MY OVER-CLAIM,
   CORRECTED. Do not file.** I read the rc=139 out of the guard's error text and
   reported it as a live defect. It is not. Verified with a valid program
   (my first probe was malformed — `now()` is not a Shape function):

   ```shape
   from std::core::intrinsics use { DateTime }
   fn f(d: DateTime) -> int { return d.unix_timestamp() + 1 }
   ```
   **VM `1718461846`, JIT `1718461846`, rc=0 on both tiers.** The guard fires and
   the program deopts; `--native-witness` reports
   `program_fallback = {scope: "program", reason_class: "jit-compile-error"}` and
   **0 native dispatches**. The rc=139 in the message describes what *would*
   happen if the guard were deleted without routing — a consequence of this
   ticket's own work, not a defect on today's tree.

   Correct status: **a hard precondition on deleting STAGE-F3.** The 7 VM-only
   typed-Arc receiver kinds must route to `dispatch_call_via_trampoline_vm` in
   the same edit that removes the guard, with a corpus fixture proving it, or the
   deletion is not landable. The distinction matters: a latent hazard conditioned
   on a future edit is a design precondition; a live hazard is a ticket. Filing
   this would have put a crash ticket in the tracker for a crash nobody can
   currently trigger.
10. ~~**should c1 land first as a separate commit**~~ — **RATIFIED (grill round
    2): land c1 as its own commit before the conversion.** §9's indivisibility
    covers the producer flip and the channel, not the error-flag adoption. Note
    for the implementing lane: c1 is **not** the fix for #254 repro B — the
    annotation control refutes that (§6.1) — so do not expect the c1 commit to
    close any of the three closure fixtures. It closes the hazard that 25 of 31
    live bail sites can put a bail placeholder into a typed slot with no deopt.
