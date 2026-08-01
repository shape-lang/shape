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

Revised scope: **~9 functions converted, ~62 deleted, 31 bare-bails treated**
(not 108 converted / 118 treated). This is materially smaller than the ticket's
estimate and lands comfortably as one slice.

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
Reproduced live (`s.length()` inside a function, `--mode jit`):

> MirToIR: scalar-returning string method `.length(...)` on a proven
> `NativeKind::String` receiver has no sound JIT codegen — the `jit_call_method`
> VM trampoline boxes the scalar result via `box_number(.. as f64)` (a NaN-boxed
> f64) or a `TAG_BOOL_*` sentinel, NEITHER of which is the raw native scalar the
> proven destination slot expects. `write_place` stores the NaN-box bits verbatim
> into the (e.g. `Int64`) slot → garbage … STAGE-StringJIT.

That is this ticket's thesis, already written down as a permanent deopt. The
conversion's payoff is that this deopt (and the Route A / ObjectStore
surface-and-stops in the same family) can be **deleted**, restoring native
execution to whole classes of programs. That is the argument for doing the
work, and it is stronger than "delete some constants".

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
| **(c) unreachable** | 69 | ~163 | delete outright |

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
- `call_string_method` (16 sites) — statically reachable from `jit_call_method`,
  dynamically unreachable because the compiler refuses to emit the call
  (STAGE-StringJIT, §2). 0 corpus hits, 3 falsifiers all deopt.
- `call_object_method` (11 sites), `call_duration_method` (6),
  `call_matrix_method` (4), `call_time_method` (3) — same shape, 0 hits.
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

## 4. The monomorphization rule

The ticket poses a dichotomy: per-kind monomorphized helpers where the emit
site knows the kind, *or* an explicit kind parameter where genuinely
polymorphic.

**The second bucket is empty. Every converted entry point is monomorphized.**

The rule, and why it is total:

> A JIT emit site may only emit a value-producing FFI call when it has a
> proven `NativeKind` for the destination slot. It therefore always knows which
> monomorph to call. A site that cannot prove the destination kind does not get
> a polymorphic fallback — it surface-and-stops, which is what it already does
> today.

This is not a new rule; it is the existing contract, stated. `write_place`'s
destination is a MIR `Place` whose slot kind is fixed by the storage planner
and readable at the emit site via
`mir_compiler/conversions.rs:29::slot_kind_of(SlotId) -> NativeKind`. Sites
without a proof already fail closed — the two live messages are the W36
named-call bail ("direct call to `mk` resolved to function index 194 but has no
compile-time-proven `FrameDescriptor.return_kind` … no runtime inference or
Null fallback. ADR-006 §2.7.5") and the Route A `Rvalue::Aggregate` surface.

Adding a runtime kind parameter would therefore be strictly a **regression**:
it would reintroduce a runtime type discriminator on a path where the compiler
already has a static proof, which is CLAUDE.md §Forbidden ("Runtime `tag_bits`
dispatch", "`SlotKind::Dynamic`") wearing a parameter instead of a tag word.
I recommend the ADR record the emptiness of the second bucket explicitly so a
later agent cannot reopen it as "the polymorphic case the design allowed for".

### 4.1 Shape of the converted signatures

Cranelift return types are per-signature, so the monomorph set is driven by
Cranelift ABI classes, not by all 27 `NativeKind` variants. Three classes:

| class | Cranelift return | members |
|---|---|---|
| `i64` | `types::I64` | `Int64`, `UInt64`, `Int32`… (widened in-slot), `Bool`, `Char`, all `Ptr(HeapKind)`, `String`, `StringV2` |
| `f64` | `types::F64` | `Float64` |
| void | *(no results)* | `Null` (ADR-020 §3.3 — already partly landed; `terminators.rs:2219` has the void-call arm) |

So `jit_call_value` becomes `jit_call_value_i64` / `jit_call_value_f64` /
`jit_call_value_void`, and likewise for the other converted entry points. The
integer class stays one monomorph because a raw `i64` slot and a raw pointer
slot are bit-identical at the ABI — the *kind* distinction lives in the
emit-site metadata and the caller's `write_place`, which is precisely ADR-020
§1's "type lives exclusively in static metadata".

`Float64` must be its own monomorph, and this is the load-bearing part: it is
the only way `box_number` dies. Today every numeric result is `f64::to_bits`
into an `i64` return. Splitting the f64 monomorph out means the value travels
in an FP register as an `f64` and never becomes bits at all.

`NullableFloat64` returns `f64` and carries §3.1's canonical NaN sentinel;
nullable narrow scalars return `i64` with §3.1's widened out-of-range niche.
Nullable 64-bit integers do **not** appear: per ADR-020 §5's third sequencing
ruling their presence-pair machinery lands with #229, and at HEAD those slot
kinds have no producer.

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

- **c1 (corrupted-state guards) — no kind needed, and mostly already done.**
  The pattern is `set_jit_runtime_error(msg); ctx.pending_call_error = 1; return
  ERROR_PLACEHOLDER_BITS`, and `ERROR_PLACEHOLDER_BITS == 0` is memory-safe under
  every kind. 31 sites in shape-jit already carry it. Remaining c1 bare-bails
  just adopt it. In the void monomorph the return disappears entirely and only
  the flag remains. **This refutes the implicit premise that c1 needs the kinds
  in the signatures first** — it does not; it is independent and could even
  precede the conversion.
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
Cranelift-emitted code). With a capture added, it becomes a silent wrong answer
of exactly `TAG_NULL + 1` = `-1407374883553279` (`0xfffb000000000000 + 1`),
rc=0. Filed as **#254**. **This shape is not in the corpus** (no corpus program segfaults:
`SYN__first-class-closure-{dispatch,return}`, `ACC__functions__segfault-repro`,
`ACC__jit-compilation__large` all rc=0; `SYN__closure-infn-tagnull` rc=1) and
it is distinct from #219, which requires a closure declared inside a function
and passed as an argument.

Working hypothesis for the fault (**not yet proven**, offered so the
implementing lane can confirm cheaply): `h` captures `g`; the capture path
retains the captured slot according to its stamped kind `Ptr(Closure)`;
`Arc::increment_strong_count` on the tag word writes to a wild address. If
confirmed, the crash is a direct consequence of the metadata lie and dies with
this conversion — which is the strongest available argument that the flip is
the fix, not a refactor.

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

## 7. The third carrier

`jit_make_closure` (`ffi/object/closure.rs:40`) produces
`unified_box(HK_CLOSURE)` — a third function-value carrier beside the VM's
`Arc<ClosureRaw>` and the JIT's `box_function`. It is emitted from the legacy
fallback at `mir_compiler/statements.rs:821`.

**Measured: it never executes.** 0 hits across 481 corpus programs, and 0 hits
across four hand-written falsifiers built specifically to force an escaping
capturing closure (local capture + escape through a call, capture pushed into
an array, nested capture, capture returned). The escaping paths that do run
reach `emit_heap_closure` or `dispatch_borrowed_closure_via_trampoline_vm`
instead; the array variant surface-and-stops at Route A
(`Rvalue::Aggregate reached the kind …`).

I could not construct a producer. **Ruling tested and upheld: delete the
`statements.rs:821` fallback and `jit_make_closure` outright; do not migrate its
error returns.** ADR-020 §6 forbids a second function-value carrier, so a third
one that provably has no producer needs no migration path, and §Greenfield
forbids keeping it "just in case".

If the implementing lane finds a producer I could not, that is a genuine
refutation and should come back as one rather than be resolved by keeping the
carrier.

---

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

| fixture | source | HEAD behaviour | post-fix |
|---|---|---|---|
| `SYN__closure-calls-closure.shape` | §6.1 m3 | **SIGSEGV** | VM==JIT `3` |
| `SYN__closure-calls-closure-capture.shape` | §6.1 m4 | silent `TAG_NULL+1` | VM==JIT `7` |
| `SYN__closure-calls-closure-outer-capture.shape` | §6.1 m5 | **SIGSEGV** | VM==JIT `7` |
| `SYN__string-scalar-method.shape` | `s.length()` in a fn | whole-fn deopt | native, VM==JIT |
| `SYN__closure-l106.shape` | L106 | green | stays green (regression) |

The string fixture is the load-bearing *positive* one: it proves the
STAGE-StringJIT deopt is gone, i.e. that the conversion delivered a typed
channel rather than merely renaming the untyped one. Without it, a conversion
that keeps every deopt in place would pass.

### 10.2 Carrier-level unit coverage that would actually bite

The inherited warning is correct and I can now state its mechanism: the
shape-jit `--lib` suite stays green through heap corruption because its tests
exercise FFI functions **as Rust functions**, with hand-built arguments, never
through Cranelift-emitted code. `jit_call_value(ctx)` called from a Rust test
with a hand-built `JITContext` never exercises the emit site's kind stamping,
which is where the lie lives. This is the §machinery-vs-wiring failure in its
purest form: the test calls the machinery, the bug is in the wiring.

Two coverage requirements, neither satisfiable by the existing suite shape:

1. **Producer/consumer kind-agreement assertion.** For every converted entry
   point, a test that asserts the emit site's `slot_kind_of(destination)` and
   the monomorph actually called agree. Mechanically: an emit-time debug
   assertion in `MirToIR` that the chosen monomorph matches the destination
   kind, exercised by compiling real MIR. This is the check that the
   `ClosurePlaceholder` stamp/emit pair would have failed since it was written.
2. **End-to-end differential through the CLI**, not the seam — the corpus
   fixtures above. A carrier defect that only manifests when Cranelift-emitted
   code retains a value is unobservable from any in-process unit test that
   builds its own context.

I recommend requirement 1 be stated in the slice record as the reason the
`--lib` suite passing is *not* evidence, so a future reviewer does not read a
green suite as coverage.

### 10.3 Standing gates

Full 481-program corpus differential, 0 unexpected (with `known-red.json`
updated: #219's `closure-infn-stack-pointer-tagnull` entry should close if this
conversion fixes it — verify rather than assume); `just verify-merge`;
`just check-clean`; `just check-no-dynamic` with the new names and a proven
bite. Rebuild `cargo build --release --bin shape` before any `run-diff.mjs`
invocation — `--fresh` does not rebuild, and stale binaries fail in the masking
direction.

---

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

## 12. Wanted from review

1. **Ratification that the polymorphic bucket is empty** (§4) and that the ADR
   should say so, closing it against reopening.
2. **A ruling on the §9 precondition**: corpus fixtures for the three closure
   shapes land *before* the conversion, in the same slice. I read this as
   inside the ticket's mandate; it is also the only way the gate is real.
3. **Confirmation that `jit_make_closure` deletion needs no producer search
   beyond what §7 records** — I looked and found none, but absence of a
   producer is the kind of claim a second pair of eyes should try to break.
4. **Disposition of #254 (the §6.1 SIGSEGV)**, now filed and routed elsewhere.
   My reading is that it dies with the §6.2 carrier flip and cannot be fixed
   independently without re-creating the #227 slice-2 failure — a point fix that
   keeps `box_function` under a `Ptr(Closure)` stamp is patching the guard, not
   the lie. If another lane lands a fix first, this design's §6 needs re-basing;
   worth deciding which ticket owns the carrier before both move.
5. Whether `box_string` / `unified_box` (150 programs — by far the most-executed
   dialect function) should be converted in this slice or whether the string
   heap carrier is #228 territory. I have scoped it in, but it is the one
   boundary I am least sure of.
