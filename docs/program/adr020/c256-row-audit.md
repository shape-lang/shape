# #256 — audit of the 33 pre-existing `check-no-dynamic` baseline rows

**Date:** 2026-08-03 · **Base:** `origin/main` @ `1054ddf3` · **Branch:** `c256-row-audit`

Discharges #256 acceptance item 3 ("the 45-row audit is recorded, even where the
answer is 'name-based is sufficient for this row'") against the corrected
denominator from the owner ruling of 2026-08-02: **33 pattern rows**, not 45
lines. The eight rows added at `a144c0d9` for #239/#256 were proven to bite in
that commit and are not the subject here; they were sanity-read and all eight
still measure exactly at their limits.

Item 2 ("the baseline header states the symbol-versus-shape distinction") was
still unowned at `1054ddf3` and is discharged here too, in the header of
`docs/check-no-dynamic-baseline.txt`.

## Method

Every count in this document was re-measured at HEAD with **the gate's own
instrument** — the `count_one()` invocation from `scripts/check-no-dynamic.sh`,
not a hand-rolled variant:

```
rg --no-heading -c -P "$pattern" -g '!*.md' crates bin tools extensions | awk -F: '{s+=$2} END {print s+0}'
```

Note `-c` counts **lines containing a match**, not matches. Two occurrences on
one line count once — which is why `memory.rs:191` contributes 1 to the
bit-pattern row despite carrying two literals.

## Headline findings

**1. Nothing was measurably slack, and that was misleading.** All 33 rows
measured exactly at their limits, so the gate reported no progress and looked
fully ratcheted. It was not. The slack was hiding one level down: **35 of the
41 residual occurrences across the six nonzero rows were comment prose or
markdown, not code.** A row sitting at 12 whose code count is 1 is an 11-unit
hiding place — delete a comment mention, add a live call, count unchanged, gate
green. Six rows were re-seeded against their code counts.

**2. The largest row in the file was measuring nothing.** `NanBox residuals`
sat at 17. Rust occurrences: **zero**. `crates/shape-jit/src/nan_boxing.rs` does
not exist. All 17 were prose in four V2-era audit `.md` files that were checked
in *under* `crates/` — `crates/shape-vm/src/V2_STAGE6_GATE.md`,
`V2_NANBOX_AUDIT.md`, `V2_VALUEWORD_MIGRATION.md`,
`executor/V2_METHOD_DISPATCH_AUDIT.md` — plus one in
`crates/shape-jit/src/compiler/V2_MIGRATION_STATUS.md`. The scope is
directory-based and the script's own header says "source trees only", but
markdown under a source directory was being counted as if it were code. The row
had 17 units of pure-fiction slack guarding a family with no live spelling.

**3. Three rows transcribe one member of a family CLAUDE.md forbids by
wildcard.** §Forbidden Patterns says "`exec_*_dynamic_fallback` handlers.
Deleted." and "`Convert<X>To<Y>` opcodes"; the baseline carried
`exec_arithmetic_dynamic_fallback`, `exec_comparison_dynamic_fallback` and
`ConvertBoolToString`. `exec_string_dynamic_fallback` and `ConvertIntToString`
passed the gate. This is the `emit_index_to_i64` lesson in its cheapest form —
not even a respelling from primitives, just a different suffix.

**4. A normative encoding constant has an uncovered private duplicate.**
`crates/shape-jit/src/ffi/value_ffi.rs:64` defines
`pub const CANONICAL_NAN: u64 = 0x7FF8_0000_0000_0000` — a second copy of
`CANON_NAN_BITS` from `crates/shape-value/src/encoding.rs:112`, which ADR-020 §6
makes the sole source for encodings. It is **dead**: defined once, read nowhere.
The existing bit-pattern row could not see it — its character class starts
`0x[fF]{3}`, so the entire positive-quiet-NaN half of the family (`0x7FF8…`) was
invisible. `mir_compiler/places.rs:1833` likewise re-spells `NULL_NUMBER_BITS`
inline (in a test).

## Instrument change

`scripts/check-no-dynamic.sh` `count_one()` gained `-g '!*.md'`.

Measured blast radius across all rows: **exactly two** — NanBox 17→0 and
`TAG_BOOL_*` 52→51. Every other row unchanged.

This is a scope correction, not a loosening. A `.md` file compiles to nothing: a
forbidden construct cannot *exist* there, only be *described* — and CLAUDE.md
§Forbidden Patterns explicitly *requires* describing deleted code by name.
`docs/`, `CLAUDE.md` and plans are already unscanned for exactly that reason;
the directory-based scope simply failed to notice prose that had been filed
under `crates/`.

**Accepted cost, stated plainly:** the eleven rejected-rename rows (14–24) are
prose detectors, and after this change they no longer police those four stale
markdown files. A rename smuggled into one of them would not fail the gate. This
is judged acceptable — those rows exist to keep the rationalization out of the
*code*, and the design prose where such a rename would actually be argued
(`docs/`) was never in scope to begin with. Flagged for the supervisor rather
than buried. The four V2-era `.md` files are stale audit records from a closed
migration and are deletion candidates in their own right; that is a content
decision outside this lane's mandate.

## The 33-row audit

Row numbers are positions in the pre-audit file (`1054ddf3`). "Measured" is the
code-line count at HEAD under the corrected instrument.

| # | Pattern (abbrev.) | Old | Measured | Classification | Action |
|---|---|---|---|---|---|
| 1 | `synthesize_value_word_from_raw` | 12 | 1 | 11 doc-comment, 1 surface-and-stop **string literal** (`execution.rs:438`) | **SHRUNK 12→1** + comment exemption |
| 2 | `last_program_return_kind` | 0 | 0 | fully deleted | keep — tombstone (§Forbidden name) |
| 3 | `normalize_persisted_for_slot` | 1 | 0 | 1 doc-comment (`helpers.rs:3175`) | **SHRUNK 1→0** + comment exemption |
| 4 | `exec_arithmetic_dynamic_fallback` | 5 | 0 | 5 doc-comment | **SHRUNK 5→0** + comment exemption |
| 5 | `exec_comparison_dynamic_fallback` | 0 | 0 | fully deleted | keep — tombstone |
| 6 | `ConvertBoolToString` | 0 | 0 | fully deleted | keep — tombstone; **family row added** (see below) |
| 7 | `rebox_native_bits` | 0 | 0 | fully deleted | keep — tombstone |
| 8 | `last_emitted_native_kind` | 8 | 3 | **LIVE production code**: 1 def (`helpers.rs:2708`) + 2 calls (`helpers.rs:4402`, `helpers_binding.rs:701`); 5 doc-comment | **SHRUNK 8→3** + comment exemption |
| 9 | `capture_as_value` | 4 | 0 | 4 doc-comment (`vm_closure_handle.rs`) | **SHRUNK 4→0** + comment exemption |
| 10 | `nan_box\|NanBox\|NanTag` | 17 | 0 | **17 markdown, 0 Rust**; `nan_boxing.rs` deleted | **SHRUNK 17→0** via instrument fix |
| 11 | `SlotKind::(Dynamic\|Unknown)` | 0 | 0 | variants deleted from the enum | keep — tombstone; weak-pattern gap noted below |
| 12–19 | 8 rejected-rename phrases | 0 | 0 | prose detectors, no occurrences | keep — no comment exemption **by design** (the target *is* the prose) |
| 20 | `tag[- ]decode[- ](bridge\|probe\|…)` | 0 | 0 | prose family | keep |
| 21 | `(decoder\|decode)[- ]bridge` | 0 | 0 | prose family; partially overlaps 20 | keep — overlap is harmless, both at 0 |
| 22 | `(synthesis\|tag)[- ]bridge` | 0 | 0 | prose family | keep |
| 23 | value-call defection family | 0 | 0 | prose family (§2.7.11/Q12) | keep |
| 24 | `call_value_(legacy\|raw_u64)` | 0 | 0 | fully deleted | keep — tombstone |
| 25 | `dispatch_value_call_handler_raw` | 0 | 0 | fully deleted | keep — tombstone |
| 26 | `call_value_with_u64_slice` | 0 | 0 | fully deleted | keep — tombstone |
| 27 | `unwrap_or((0, NativeKind::Bool))` | 0 | 0 | **code-shape** row, already | keep |
| 28 | `unwrap_or(NativeKind::Bool)` | 0 | 0 | **code-shape** row, already | keep |
| 29 | Arc ptr-op on `TypedObjectStorage` | 0 | 0 | **code-shape** row (R6 carrier-UB) | keep |
| 30 | Arc ptr-op on `TraitObjectStorage` | 0 | 0 | **code-shape** row (R6 carrier-UB) | keep |
| 31 | `let expected = NativeKind::Bool` | 0 | 0 | **code-shape** row (W17 Stage-0) | keep |
| 32 | `TAG_UNIT` (comment-exempt) | 0 | 0 | fully deleted (#224) | keep — tombstone |
| 33 | `TAG_BOOL_(TRUE\|FALSE)` | 52 | 51 | live kind-blind JIT FFI dialect (#239's target); 52nd was markdown | **SHRUNK 52→51** via instrument fix |

Totals: **6 rows shrunk**, 27 kept, **0 rows deleted**, **0 limits raised**.

## Rows added

All three seed at their measured count and were **proven to bite**.

| Pattern | Seed | Why |
|---|---|---|
| `^(?!\s*(//\|///\|\*)).*\bexec_\w+_dynamic_fallback\b` | 0 | Generalizes rows 4+5 to CLAUDE.md's own wildcard. Residue is 5 comment mentions citing the prohibition |
| `^(?!\s*(//\|///\|\*)).*\bConvert[A-Z]\w*To[A-Z]\w*\b` | 0 | Generalizes row 6 to CLAUDE.md's `Convert<X>To<Y>` class. Residue is 4 comment mentions citing the prohibition |
| `^(?!\s*(//\|///\|\*)).*0x7[fF]{2}[89a-fA-F]_?0{4}_?0{4}_?0{3}[0-9a-fA-F]` | 4 | The positive-quiet-NaN half of the encoding family, which the `0xFFF…` row cannot see. Seeds at 2 normative defs (`encoding.rs:104,112`) + `value_ffi.rs:64` `CANONICAL_NAN` + `places.rs:1833`. SHRINK-ONLY, target 2 |

The first two use the in-file precedent of rows 27–31 (code-shape) and 32–33
(comment exemption); the third mirrors the existing bit-pattern row exactly.

## Bite proofs

A ratchet never observed to fail is an assertion, not a gate. Each changed and
added row was proven by temporary reintroduction into a scratch file under
`crates/`, then removal.

**Round 1 — the six shrunk rows plus the bit-pattern row.** Eight violating
code lines (`synthesize_value_word_from_raw`, `normalize_persisted_for_slot`,
`exec_arithmetic_dynamic_fallback`, `last_emitted_native_kind`,
`capture_as_value`, `NanTag::F64`, `TAG_BOOL_TRUE`, `0x7FF8_0000_0000_0002`).
Result: **exit 1, all eight rows FAIL, each with `regression: +1`** —

```
FAIL  W-series ValueWord synthesizer …            baseline=1   actual=2    (regression: +1)
FAIL  W-series persistence normalizer …           baseline=0   actual=1    (regression: +1)
FAIL  dynamic arithmetic fallback handler …       baseline=0   actual=1    (regression: +1)
FAIL  sparse kind tracker …                       baseline=3   actual=4    (regression: +1)
FAIL  closure capture deletion progress …         baseline=0   actual=1    (regression: +1)
FAIL  NanBox residuals …                          baseline=0   actual=1    (regression: +1)
FAIL  ADR-020 §3 clause 2 (#226) …                baseline=51  actual=52   (regression: +1)
FAIL  ADR-020 §2.1/§6 (#256 row audit) …          baseline=4   actual=5    (regression: +1)
```

Removal → exit 0.

The `NanTag::F64` line is also the **positive control for the markdown
exclusion**: it proves the exclusion did not blind the row to Rust.

**Negative control.** The same text (`NanTag`, `TAG_BOOL_TRUE`,
`synthesize_value_word_from_raw`, `0x7FF8_0000_0000_0002`) placed in a `.md`
file under `crates/shape-vm/src/` → **exit 0**. The exclusion behaves as
designed.

**Round 2 — the two family rows, proven against the gap they close.** The
scratch file used spellings the *named sibling rows cannot match*:
`exec_string_dynamic_fallback` and `OpCode::ConvertIntToString`. Result: **exit
1, exactly two FAILs, both `baseline=0 actual=1 (regression: +1)`** — the two
new family rows — while `exec_arithmetic_dynamic_fallback`,
`exec_comparison_dynamic_fallback` and `ConvertBoolToString` all stayed green.
That is the gap demonstrated, not merely a row shown capable of failing.
Removal → exit 0.

## Weak-pattern gaps found and NOT acted on

Recorded rather than patched, per the lane's propose-don't-add discipline.

**`exec_*_dynamic_fallback` guards a dispatch pattern, not only a name.** A
runtime kind switch inside an arithmetic handler is rebuildable from primitives
under any identifier. No companion row is proposed: the shape is a `match` on
`NativeKind` pairs, and legitimate code matches on `NativeKind` throughout the
VM. Any regex tight enough to avoid swamping the gate would be trivially
sidestepped, and a row that fires constantly is a row that gets ignored. The
honest position is that this class needs a **type-level** guard (the
`prove_native_kind() -> Result<_, ProofGap>` private-constructor mechanism is the
existing example) rather than a grep.

**The CLAUDE.md broader-family rename regex cannot be adopted verbatim.** The
documented family is
`(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)`
— a 9×7 space of which rows 20–23 implement four fragments. Missing combinations
include `kind probe`, `dispatch bridge`, `tag helper`. Measured at HEAD: **23
hits**, all comment prose, and the majority are *negations or citations of the
rule itself* — `// no runtime kind probe`, `// This is NOT a tag/kind bridge`,
`// no bridge/decode-hop` — plus two entirely innocent English phrases
("output capture adapter"). Seeding a row at 23 would create the largest slack
in the file while punishing the documentation CLAUDE.md mandates. Not added.
The generalization is real but unmechanizable at the grep layer.

**`SlotKind::(Dynamic|Unknown)` is rename-escapable.** `SlotKind::Any`,
`::Untyped`, `::Generic`, `::Opaque` would all pass. A speculative alternation
was not added — guessing at the next author's synonym is how a row acquires
false positives without acquiring coverage. The structural guard is that the
enum has no such variant and adding one is a visible diff.

**`IntToNumber` / `NumberToInt` deliberately have no row.** CLAUDE.md forbids
*emitting* them to paper over a type mismatch, but the opcodes are live and
legitimate: 49 code-line occurrences implementing explicit `as` casts, which the
2026-06-01 numeric-conversion ruling requires. A row would seed at 49 and
measure compiler *intent*, which grep cannot see. Deliberately absent.

**Rows 12–19 punish correct documentation, by design.** These match bare
English ("compatibility layer", "decode hop") with no comment exemption, so a
comment quoting CLAUDE.md's refuse-on-sight list fails the gate. Left as-is —
the refusal is meant to bind in prose too — but noted as a live false-positive
hazard for the next author who tries to document the prohibition.

## Open question for the supervisor

The four stale V2-era audit `.md` files under `crates/` are the sole reason the
NanBox row read 17. They are records of a closed migration living in a source
tree. Deleting them would let the instrument change be reverted entirely, at the
cost of losing the history. Out of this lane's mandate; flagged, not acted on.

## Gate state

`bash scripts/check-no-dynamic.sh` → **exit 0**, silent (no FAIL, no progress
line): all 44 rows now sit exactly at their measured counts.
