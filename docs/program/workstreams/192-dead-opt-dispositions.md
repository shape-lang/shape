# PERF-DEAD-OPT (#192) — disposition of every `shape-jit/src/optimizer/` component

**Authority:** ADR-018 §6 ("Dead analyses are wired or deleted; there is no
third state"), §4 (prior-art rule), R24. Ticket: shape-lang/shape#192.
**Outcome: all thirteen components DELETED.** Zero were wired. No component is
marked "later" — ADR-018 §6 names "keep it for later" as precisely the
forbidden third state.

Total removed: **5,358 lines** across 13 files, **34 tests**, plus one
consumer-side test (`jit_cache::test_tier2_cache_key_stored_in_entry`) and two
dead landing-pad fields. Deleted at `crates/shape-jit/src/optimizer/`.

## The wiring proof, once, for all thirteen

The module was declared exactly once, privately, and under a blanket
`#[allow(dead_code)]`:

```rust
// crates/shape-jit/src/lib.rs:51-55 @ 7b983a15
#[allow(dead_code)]
mod optimizer;
```

`mod optimizer` was **private to the crate**, so no `pub` inside it was
externally reachable and the `pub mod escape_analysis` / `pub mod licm` /
`pub(crate) mod vectorization` visibilities were inert. Exactly **one** symbol
crossed the module boundary in the whole workspace:

```
$ git grep -n 'optimizer' 7b983a15 -- 'crates/shape-jit/src/*.rs' ':!crates/shape-jit/src/optimizer/*'
crates/shape-jit/src/jit_cache.rs:15:use crate::optimizer::Tier2CacheKey;
```

(The other three `optimizer` hits at base are prose in doc comments:
`loop_analysis.rs:42`, `executor.rs:170`, `mir_compiler/field_ref_regression_tests.rs:18`.)

Every analysis entry point had exactly **one** caller — `build_function_plan`
in `optimizer/mod.rs:78` — and `build_function_plan` itself had **zero**:

```
$ git grep -n 'build_function_plan' 7b983a15 -- '*.rs'
7b983a15:crates/shape-jit/src/optimizer/bounds.rs:4:         (doc comment)
7b983a15:crates/shape-jit/src/optimizer/mod.rs:78:          (the definition)
7b983a15:crates/shape-jit/src/optimizer/numeric_arrays.rs:7: (doc comment)
```

So "dead" here is not "dead on the hot path" or "dead pending wiring": the
plan builder was never invoked from any code path, test or otherwise. Nothing
in the crate ever constructed a `FunctionOptimizationPlan` from a real
program. The per-component tests all called the individual `analyze_*`
functions directly against hand-built `BytecodeProgram` fixtures.

**Confirming evidence (the trial cfg-out the ticket allows):** deleting all
thirteen files plus the `mod optimizer;` line leaves
`cargo check -p shape-jit --all-targets` and `just check-clean` green with no
new warnings. The only compile fallout in the entire workspace was the single
`Tier2CacheKey` import above.

## The disposition table

| # | Component | Lines | Tests | Wiring status at `7b983a15` | Disposition |
|---|-----------|------:|------:|-----------------------------|-------------|
| 1 | `bounds.rs` | 1034 | 4 | `analyze_bounds` ← `build_function_plan` only | **DELETED** |
| 2 | `licm.rs` | 597 | 6 | `analyze_licm` ← `build_function_plan` only | **DELETED** |
| 3 | `vectorization.rs` | 653 | 4 | `analyze_vectorization`, `analyze_simd` ← `build_function_plan` only | **DELETED** |
| 4 | `loop_lowering.rs` | 288 | 2 | `plan_loops` ← `build_function_plan` only | **DELETED** |
| 5 | `escape_analysis.rs` | 743 | 7 | `analyze_escape` ← `build_function_plan` only | **DELETED** |
| 6 | `call_path.rs` | 186 | 2 | `analyze_call_path` ← `build_function_plan` only | **DELETED** |
| 7 | `correctness.rs` | 264 | 0 | `validate_plan` ← `build_function_plan` only | **DELETED** |
| 8 | `cross_function.rs` | 92 | 1 | `Tier2CacheKey` ← `jit_cache::CacheEntry.tier2_key`, a field that is `None` on the sole live construction site and read only in a test | **DELETED** |
| 9 | `hof_inline.rs` | 136 | 0 | `analyze_hof_inline` ← `build_function_plan` only | **DELETED** |
| 10 | `numeric_arrays.rs` | 918 | 8 | `analyze_numeric_arrays` ← `build_function_plan` only | **DELETED** |
| 11 | `table_queryable.rs` | 55 | 0 | `analyze_table_queryable` ← `build_function_plan` only | **DELETED** |
| 12 | `typed_mir.rs` | 266 | 0 | `build_typed_mir` ← `build_function_plan` only | **DELETED** |
| 13 | shared plan/cache types (`mod.rs`: `FunctionOptimizationPlan`, `build_function_plan`) | 126 | 0 | zero callers | **DELETED** |

No row measures a win, because no row could: a component with zero callers has
no before/after to report. The charter rule ("no measurement, no close")
applies to *wiring* a component; the measurement standard for a *deletion* is
the wiring proof above plus the VM/JIT differential below.

## Re-derivation evidence each deletion owes

Deletion must not be silent capability loss. For each component, what a future
implementer would have to produce to justify re-deriving it — and, where the
component was wrong in a way worth remembering, why re-deriving it *as written*
would be a mistake.

**1. `bounds.rs`.** Verdict inherited from #191, which evaluated it as the
candidate engine for the ADR-018 §5 widened matcher and rejected it on four
independent grounds: wrong IR (bytecode peephole, not the live MIR path);
loop-keyed plan shape unsound for per-site trust; a runtime-guard mechanism
ADR-018 §5 explicitly rejects ("Speculative guards with runtime deoptimization
are rejected"); zero non-test callers. **Re-derivation must produce:** the
affine `arr[i*n+k]` index shape (`expr_is_affine_square_index` /
`AffineSquareGuard`), which is the one genuinely novel idea here and is *not*
covered by #191's shipped analyzer — re-derived MIR-side, as a static proof,
never as a loop-entry runtime guard. Loop versioning (the
`linear_bound_guards_by_loop` / `non_negative_iv_guards_by_loop` mechanism)
cannot exist at all without an ADR-018 §5 amendment first.

**2. `licm.rs`.** Loop-invariant hoisting of pure calls. **The mechanism must
not be re-derived as written:** purity was selected by *method-name string*
(`is_pure_method_name` matching `"row" | "col" | "transpose" | "shape" |
"len"` out of the program string pool), which is exactly the
spelling-selected-semantics that §Forbidden Patterns refuses and that #191
refused on the same grounds for `len()`-spelled bounds. **Re-derivation must
produce:** a purity fact from the effect-row / resolved-intrinsic-identity
machinery (ADR-011 `IntrinsicId`, ADR-014 effect rows), not a name match; plus
a measured win, since ADR-018 §6 names LICM an expected deletion "unless [its]
wiring ticket can demonstrate a measured win at acceptable risk". The
`BuiltinFunction` half of the whitelist (`Sin`/`Cos`/`Sqrt`/… by enum
discriminant) was identity-based and is sound to re-derive as-is; the
invariance test (`is_invariant_value_producer` over `LoopInfo.invariant_locals`
/ `invariant_module_bindings`) is re-derivable in a few lines from the live
`loop_analysis.rs`, which still computes both sets.

**3. `vectorization.rs`.** Two passes. `analyze_vectorization` picked a
strip-mining width per loop header; `analyze_simd` matched an F64X2 body.
**Re-derivation must produce:** the SIMD eligibility conjunction, which is the
substantive content — canonical IV with step 1, nesting depth 0,
`!body_can_allocate`, body ≤ 80 instructions, every body opcode SIMD-safe, and
*exactly* two invariant-array `GetProp` reads + one f64 arith + one indexed
write to an invariant destination, with source/destination distinct from the
IV and bound slots. Also the scalar-remainder obligation (lengths not divisible
by the lane count). None of this was ever validated against a real program —
its four tests assert against hand-built `BytecodeProgram` fixtures, the same
synthetic-only shape that #191 found had let the shipped bounds matcher admit
zero real programs. ADR-018 §6 names vectorization an expected deletion; a
re-derivation needs a measured win on the committed charter suite, not a
matcher that fires on fixtures.

**4. `loop_lowering.rs`.** Mostly *not* novel: `plan_loops` reads
`header_idx`, `end_idx`, canonical IV, `bound_slot` and `step_value` straight
off `LoopInfo`, which the live `loop_analysis.rs` still computes. Note also
that its `_typed_mir` parameter was unused — the Phase-1 typed MIR never fed
the loop plan. **Re-derivation must produce** only the two genuine deltas: the
`unroll_factor` heuristic (`estimate_unroll_factor` over the body's
numeric-op vs memory-op counts, capped at 2 for nested loops) and the
`register_carried_locals` / `register_carried_module_bindings` sets
(locals both written *and* read in the body — the loop-carried-dependency
set a register allocator would want).

**5. `escape_analysis.rs`.** Disposition already recorded on #193's close
under the ADR-018 §4 prior-art rule, and it stands: "the dead
optimizer/escape_analysis.rs criteria survive HERE; its mechanism
(containment=escape, single-block confinement, scalar-replacement artifacts)
has no remaining consumer". `crates/shape-vm/src/mir/escape.rs` is strictly
stronger on every axis — MIR-level rather than bytecode-level, whole-function
rather than single-basic-block, a nine-vector escape table with transitive
containment rather than a fixed "is it passed to a call" checklist,
three-valued facts (`FrameConfined` is a proof, everything undecidable is
`NotProven`) rather than a boolean `escaped` flag, plus an inbound-reference
product that had no prior art here at all. **Re-derivation must produce** the
one artifact the MIR version deliberately does not: the *scalar-replacement
transform plan* — `ScalarArrayEntry`'s per-element `get_sites` / `set_sites`
maps (instruction index → element index) for arrays of ≤ 8 elements with
constant indices. `mir/escape.rs` answers "does this allocation die with the
frame"; it does not enumerate the per-element accesses an SROA transform needs
to rewrite. That enumeration, re-derived over MIR against
`region_exemption_candidates()`, is the whole of what is lost here. Sequencing
caveat from #193: charter-workload yield is currently zero (0 of 4 allocation
sites frame-confined), so an SROA consumer cannot be planned on today's
numbers.

**6. `call_path.rs`.** Direct-call-vs-inline site selection plus
`restore_param_slots_by_call_site` — the list of parameter local slots that
must be restored after a direct call writes its arguments into
`ctx.locals[0..argc)`. That restore obligation is an artifact of one specific
calling convention that the live `mir_compiler` does not use. **Re-derivation
must produce:** its own calling-convention analysis against whatever ABI is
then live; nothing here transfers. The `inline_depth_limit: 4` default is the
only reusable number, and it is a guess, not a measurement.

**7. `correctness.rs`.** `debug_assert`-only plan-invariant validation
(trusted indices point at real `GetProp`/set instructions, guard array sources
resolve). It validates a data structure that no longer exists. **Re-derivation
must produce:** the same discipline applied to whatever plan replaces it —
that a "trusted"/elided site is asserted to point at the opcode it claims,
in-tree and in debug builds, rather than trusted from the analysis that
produced it. The idea is worth keeping; the code is not.

**8. `cross_function.rs`.** `Tier2CacheKey` — a Tier-2 JIT cache key over
(root blob hash, sorted inlined-callee hashes, compiler version, schema
version, feedback epoch) with a SHA-256 `combined_hash`. It had a consumer in
name only: `jit_cache::CacheEntry.tier2_key` is `None` at the sole live
construction site (`jit_cache.rs:101` @ base) and is read only inside
`test_tier2_cache_key_stored_in_entry`. What actually drives Tier-2
invalidation today is `CacheEntry.dependencies`, which *is* populated on the
live insert path and which `invalidate_by_dependency` walks — that path is
untouched and `test_invalidate_with_tier2_entries` still covers it.
**Re-derivation must produce:** the invalidation argument, which is the real
content — inlining changes the emitted code, so a Tier-2 entry's identity must
include its inlined callees (order-independently, hence the sort), and
schema-version / feedback-epoch must participate because compiled code embeds
shape guards and speculation assumptions that a bump invalidates. Re-derive
that as a key only when something actually keys a cache on it.

**9. `hof_inline.rs`.** Statically resolving the callback `function_id` behind
`map`/`filter`/`reduce`/`find`/`some`/`every`/`forEach`/`findIndex` so the JIT
could emit an inline Cranelift loop instead of an FFI round-trip. Dispatch was
keyed on `MethodId` constants, not spelling — sound, and the idea is live
program work: it is the same target as ADR-018 §7 / PERF-CLOSURE-NATIVE (#188),
whose acceptance bar is `.map`/`.filter` chains executing natively.
**Re-derivation must produce:** the callback-resolution step against the
current closure carrier (a bytecode back-scan for the `function_id` producer
does not survive the carrier unification), and it belongs in #188's lane, not
in a resurrected plan module. This is the one component whose *goal* is
unambiguously still wanted; its 136 lines are not the way to get there.

**10. `numeric_arrays.rs`.** Classified `GetProp` / indexed-write sites by the
kind their *consumer* demands (int / float / bool), gated on a proven index
from `bounds.rs`. This is use-site kind inference over bytecode — in other
words, a second, weaker answer to the question `prove_native_kind()` in
`crates/shape-vm/src/compiler/type_tracking.rs` already answers with a
`ProofGap` whose constructor is private so emit code cannot fabricate a proof.
**Re-derivation is presumptively refused:** a parallel kind-inference path
whose failure mode is silent (no `ProofGap`, just a site absent from a
`HashSet`) is the shape §Forbidden Patterns exists to prevent. Anything wanted
here should extend the compiler's kind tracker instead — the same ruling
CLAUDE.md records for the W4-δ `ConvertBoolToString` opcode. The only detail
worth carrying forward is the asymmetry its author noted: bool *set* lowering
has checked / non-negative / trusted variants and so does not need a proven
index, while numeric set lowering does.

**11. `table_queryable.rs`.** 55 lines collecting instruction-index sets for
`LoadColF64`/`I64`/`Bool`/`Str` and for `filter`/`map`/`count`/`limit`/
`orderBy` `MethodId`s. No analysis — a single pass recording where opcodes
occur. **Re-derivation cost is minutes**; there is no idea to preserve beyond
the observation that typed column loads and the query-DSL method set are the
sites a Table/Queryable lowering would care about.

**12. `typed_mir.rs`.** A bytecode-level shadow stack producing a per-value
`ScalarType` of `I64 | F64 | Bool | Boxed | Unknown`. **Re-derivation is
refused on discipline grounds:** `Boxed`/`Unknown` are a dynamic-fallback
lattice in compiled-code analysis, the same shape as the deleted
`SlotKind::Dynamic` / `SlotKind::Unknown`. Under strict typing a kind is either
proven (`NativeKind`, stamped at compile time) or a compile error; there is no
`Unknown` tier. The live typed-MIR path is `crates/shape-jit/src/mir_compiler/`
consuming shape-vm's MIR, which carries real `NativeKind`s. Nothing here
transfers. (It is also the component that most clearly dates the module:
`plan_loops` took `_typed_mir` and ignored it.)

**13. Shared plan/cache types (`mod.rs`).** `FunctionOptimizationPlan` —
eighteen fields of per-loop and per-instruction-index side tables keyed by
bytecode offset — and `build_function_plan`, the eleven-pass pipeline that
filled it. **Re-derivation must produce** a different keying scheme: bytecode
instruction indices as the join key between analysis and codegen is what made
every component in this module bytecode-bound and therefore unusable by the
live MIR path (#191's first rejection ground for `bounds.rs`). Facts should be
published on the MIR structures their consumers already hold — the pattern
`mir/escape.rs` follows by publishing onto `StoragePlan`.

## Consumer-side cleanups (strictly necessary, disclosed)

Deleting the module required three edits outside `optimizer/`, each removing
something that existed only to serve it:

1. `crates/shape-jit/src/lib.rs` — the `mod optimizer;` declaration and its
   `#[allow(dead_code)]`, replaced by a comment pointing here.
2. `crates/shape-jit/src/jit_cache.rs` — the `Tier2CacheKey` import, the
   `CacheEntry.tier2_key` field (always `None` on the live path, read only in
   its own test), and `test_tier2_cache_key_stored_in_entry`.
   `test_invalidate_with_tier2_entries` is kept and still passes: it exercises
   `dependencies`-driven invalidation, which is the live mechanism.
3. `crates/shape-jit/src/loop_analysis.rs` — `LoopInfo.hoistable_calls`, the
   LICM landing pad. Its doc claimed "The translator consults this to emit
   hoisted calls in the loop pre-header"; nothing read it, and the only writes
   were `Vec::new()`.

A stale prose reference in
`crates/shape-jit/src/mir_compiler/field_ref_regression_tests.rs:18` was
re-pointed at the deletion.

## Scope boundary

`crates/shape-jit/src/mir_compiler/bounds_elision.rs` — #191's analyzer, which
IS wired and tested — is **not** in this ticket's scope and was not touched.
It is not in `optimizer/`, it is not dead, and its survival on measured
grounds (0.999x / 0.978x with an elision-off control) is a user disposition
recorded on #192. Its trap controls in
`crates/shape-jit/src/bounds_elision_traps.rs` also guard the `.length` kind
projection, which is load-bearing (the 5.30x class), and must not be removed
without replacing their nativity assertions.

## Verification

- **Wiring:** `git grep` for every exported entry point across the workspace
  returns hits only inside `optimizer/` at base; post-deletion the workspace
  grep for those names is empty. `just check-clean` green.
- **Differential (`tools/vmjit-diff`):** 467 of the 472 manifest programs run
  VM-vs-JIT, twice (once mid-work, once against the release binary built from
  the final commit). **Zero unexpected non-MATCH; zero new DIVERGED**, both
  times. Every non-MATCH is a pinned known-red:
  `SYN__closure-infn-tagnull.shape` (#219),
  `ACC__functions__finding_s1_unknown_hof_return_kind_confusion.shape`
  (`hof-return-kind-raw-bits`, WF-3A), and — sampled once each way across the
  two runs, exactly as its pin predicts it will — `ACC__comptime__pb3.shape`
  (`comptime-typedstring-verify-nondeterministic`). Hence 465 MATCH / 2
  non-MATCH on the first run and 464 / 3 on the second, that flake being the
  entire difference. The 5 unrun programs are #218's defect (manifest entries
  whose files were deleted by `85fdfce5`, all in `ACC__comptime__`), which
  makes a plain `just diff-vmjit` abort; run per group per the #187 workaround.
- **Tests:** `cargo test -p shape-jit --lib` — 489 passed, 0 failed. Proven no
  regression by name-set diff rather than by count: the listed test set shrinks
  544 → 509, and `comm` against the base listing shows the removed 35 are
  exactly the 34 `optimizer::*` tests plus
  `jit_cache::tests::test_tier2_cache_key_stored_in_entry`, with zero other
  removals and zero additions.
- **Ratchets:** `bash scripts/verify-merge.sh` 22/22. The ADR-011–016 census
  ratchets (CHECK 15/16) are shrink-only, and this change only shrinks.
- **Book gate:** not applicable. Deleting code with zero callers changes no
  public behavior, so the standing ADR-016 gate has nothing to cover.
