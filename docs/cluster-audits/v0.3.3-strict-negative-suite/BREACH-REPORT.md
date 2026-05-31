# v0.3.3 Strict-Typing Negative Acceptance Suite -- Breach Report

**Date:** 2026-05-28
**Binary:** `./target/release/shape` (release)
**Default diagnostic mode:** `TypeDiagnosticMode::ReliableOnly`
(`crates/shape-vm/src/compiler/compiler_impl_initialization.rs:125`)
**Harness:** every program run under BOTH `--mode vm` and `--mode jit`,
HOF/debug spam filtered (`^HOF:`, `outer_source_vars`, `[jit-fallback]`).

## What this suite is

A NEGATIVE type-safety corpus: every program is code that a strict type system
is REQUIRED to reject ("there is only strict" -- CLAUDE.md Type System Rules +
Forbidden Patterns). Today's shipped default (`ReliableOnly`) SUPPRESSES most
type errors -- it runs `should_emit_type_diagnostic` as a filter over analyzer
errors and only surfaces a "reliable" subset
(`compiler_impl_reference_model.rs:1428`). The point of this corpus is to
MEASURE that breach: how many of these required-reject programs WRONGLY run or
crash today.

Classification (CURRENT behavior, not the target):

- **REJECTS_CLEAN** -- `ec != 0`, a clean type/semantic error message, no value
  printed, no panic/crash. (Already strict -- good.)
- **LEAKS_RUN** -- `ec == 0`, the program executed/printed (often garbage like a
  reinterpreted pointer). THE BREACH.
- **CRASHES** -- panic / SIGABRT / SIGSEGV (`ec ~ 134/139`) instead of a clean
  compile-reject. ALSO a breach (must be a clean error, not a crash).

## Per-category table

| category | #programs | REJECTS_CLEAN | LEAKS_RUN | CRASHES | VM!=JIT | verify |
|---|---|---|---|---|---|---|
| not-callable-not-indexable | 15 | 13 | 1 | 1 | 1 | SOUND |
| closure-typed-hof-mismatch | 8 | 0 | 7 | 1 | 1 | SOUND |
| **TOTAL** | **23** | **13** | **8** | **2** | **2** | **SOUND** |

## Headline breach

**10 of 23 programs (43.48%)** that a strict type system is REQUIRED to reject
FAIL to compile-reject under `ReliableOnly` today: they either run to completion
(8 LEAKS_RUN) or crash the process (2 CRASHES) instead of emitting a clean
compile-reject.

`breach_pct = (total_leak_run + total_crash) / total_programs * 100`
` = (8 + 2) / 23 * 100 = 43.48%`.

- **worst_categories** (highest leak/crash): `closure-typed-hof-mismatch`
  -- 8/8 = 100% breach (0 of its 8 programs reject cleanly today).
- **already_strict_categories** (fully REJECTS_CLEAN today): NONE. Even
  `not-callable-not-indexable`, the stronger of the two (13/15 reject), still
  leaks once and crashes once.

## Category 1: not-callable-not-indexable (15 programs)

Programs that call a non-callable value (`int`/`number`/`bool`/`string`) or
index/access a member on a non-indexable value, or index with a wrong-typed key.
Strict typing must reject all 15. 13 already reject cleanly; 1 leaks; 1 crashes.

| program | error mode | VM | JIT | note |
|---|---|---|---|---|
| `r01_call_int.shape` | REJECTS_CLEAN | ec=1 | ec=1 | `call_value_immediate_nb: callee must be ... got Int64` |
| `r02_call_float.shape` | REJECTS_CLEAN | ec=1 | ec=1 | callee got Float64 |
| `r03_call_bool.shape` | REJECTS_CLEAN | ec=1 | ec=1 | callee got Bool |
| `r04_call_string.shape` | REJECTS_CLEAN | ec=1 | ec=1 | callee got String |
| `r05_index_int.shape` | REJECTS_CLEAN | ec=1 | ec=1 | `TypeError: expected ... heap value, got scalar` |
| `r06_index_float.shape` | REJECTS_CLEAN | ec=1 | ec=1 | got scalar |
| `r07_index_map_with_int.shape` | REJECTS_CLEAN | ec=1 | ec=1 | `expected string property name, got non-string key` |
| `r08_method_map_on_int.shape` | REJECTS_CLEAN | ec=1 | ec=1 | `no method 'map' on receiver kind Int64` |
| `r09_length_on_float.shape` | REJECTS_CLEAN | ec=1 | ec=1 | `expected array, object, or string, got scalar` |
| `r10_field_on_bool.shape` | REJECTS_CLEAN | ec=1 | ec=1 | got scalar |
| `r11_string_index_int.shape` | REJECTS_CLEAN | ec=1 | ec=1 | `SURFACE: GetProp on String not yet kinded` (NotImplemented) |
| `r12_string_index_string.shape` | REJECTS_CLEAN | ec=1 | ec=1 | same SURFACE |
| `r13_array_index_string.shape` | REJECTS_CLEAN | ec=1 | ec=1 | `Index <garbage> out of bounds` -- string ptr reinterpreted as int, bounds-caught |
| `leak01_array_index_bool.shape` | **LEAKS_RUN** | ec=0 prints `20` | ec=0 prints `20` | `arr[true]` -- `true` reinterpreted as index 1 |
| `crash01_array_index_array.shape` | **CRASHES** | ec=1 (garbage idx) | **ec=139 SIGSEGV** | `let idx = arr; arr[idx]` -- heap ptr reinterpreted as raw 64-bit index, JIT load deref unmapped mem |

**Notes / breach symptoms.** `r05`/`r06`/`r13` reject cleanly but only because a
reinterpreted-pointer index happens to fall out of bounds -- the garbage index
value (`Index 554185952 ...`) is itself evidence of the missing static reject.
`leak01` is the clean breach: a `bool` index silently becomes `int 1`.
`crash01` is the worst: indexing an array by an array (a heap pointer) under JIT
skips the bounds check and dereferences unmapped memory.

**VM!=JIT:** `crash01` only -- VM emits a clean out-of-bounds error (ec=1), JIT
SIGSEGVs (ec=139). The JIT-compiled GetIndex for a lexical-alias index binding
mis-assumes the index kind is `int` and emits a raw load.

## Category 2: closure-typed-hof-mismatch (8 programs)

Closures passed to higher-order array methods (`map`/`filter`/`forEach`/`find`/
`some`) whose closure type contract is violated -- the closure returns the wrong
type for the HOF's element/predicate signature. Strict typing must reject all 8.
NONE reject today: 7 leak, 1 crashes.

| program | error mode | VM | JIT | breach |
|---|---|---|---|---|
| `leak01_map_int_to_number.shape` | **LEAKS_RUN** | `[2.0, 3.0, 4.0]` | same | `Array<int>.map(\|x\| x + 1.0)` -- int element + number, no unify |
| `leak02_filter_pred_returns_int.shape` | **LEAKS_RUN** | `[1, 2, 3, 4]` | same | filter predicate returns `int` (`x+1`), not `bool` -- all kept |
| `leak03_foreach_returns_value.shape` | **LEAKS_RUN** | `done` | same | forEach closure returns `int` (`x*2`), should be Unit -- dropped |
| `leak04_map_number_to_int.shape` | **LEAKS_RUN** | `[7, 7, 7]` | same | `Array<number>.map(\|x\| 7)` -- number element + int, no unify |
| `leak05_filter_pred_returns_number.shape` | **LEAKS_RUN** | `[1, 2, 3, 4]` | same | filter predicate returns `number` (`x*1.0`), not `bool` |
| `leak06_find_pred_returns_int.shape` | **LEAKS_RUN** | `6` | same | find predicate returns `int` (`x-5`), not `bool` |
| `leak07_some_pred_returns_number.shape` | **LEAKS_RUN** | `true` | same | some predicate returns `number` (`x+0.0`), not `bool` |
| `crash01_array_value_as_index.shape` | **CRASHES** | ec=1 (garbage idx) | **ec=139 SIGSEGV** | element-vs-index type confusion: `Array<int>` value used as an `int` index, JIT raw-load deref |

**Notes.** The HOF builders cleanly validate scalar element kinds at push time
(string-into-`f64`-array, etc. all reject), but the closure RETURN type contract
itself is not enforced under `ReliableOnly` -- a predicate returning `int`/
`number` (truthy nonzero) or a `map`/`forEach` body returning the wrong scalar
runs to completion. The crash specimen exercises the same lexical-alias-as-index
JIT mis-specialization as Category 1's crash: an `Array<int>` value (the exact
type a `filter`/`map` predicate parameter carries) used where an `int` index is
required. The em-dash-in-comment compiler panic at
`compiler_impl_initialization.rs:655` (a non-char-boundary slice on multibyte
UTF-8) was encountered and avoided -- all suite programs are ASCII-only so each
exhibits exactly ONE error mode (the type error under test), not the lexer crash.

**VM!=JIT:** `crash01` only -- VM ec=1, JIT ec=139 SIGSEGV (same root cause as
Category 1's crash).

## Target (post-strict-flip acceptance gate)

After the strict flip (`TypeDiagnosticMode::Strict` by default -- the analyzer's
errors surfaced unfiltered, no `should_emit_type_diagnostic` gate), **EVERY one
of these 23 programs MUST be REJECTS_CLEAN under BOTH `--mode vm` AND
`--mode jit`**: a clean compile-reject, no value printed, no crash, identical
VM/JIT outcome. This corpus is the red->green acceptance gate:

- Today: 13/23 reject, 8 leak, 2 crash, 2 VM!=JIT divergences -> RED.
- Target: 23/23 reject, 0 leak, 0 crash, 0 VM!=JIT divergence -> GREEN.

The 8 LEAKS_RUN must become compile-rejects (no runtime coercion, no dynamic
fallback -- CLAUDE.md). The 2 CRASHES must become compile-rejects (a strict
index-type check at compile time eliminates the JIT raw-load entirely). The 13
current REJECTS_CLEAN must STAY rejecting -- but ideally move from runtime errors
to compile-time rejects (e.g. `r05`/`r13`'s garbage-index runtime errors should
become static "value of type T is not indexable / index must be int" diagnostics
with no pointer reinterpretation ever occurring).
