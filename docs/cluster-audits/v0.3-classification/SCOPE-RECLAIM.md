# SCOPE-RECLAIM audit — user-pull-in vs SURFACE-cite contradictions

**HEAD:** `82f049dd`.
**Audit closed:** 2026-05-27.
**Scope:** all 756 SCOPE-RECLAIM entries from the v0.3.2 classification
audit. This doc enumerates the SURFACE-cite-vs-dated-pull-in
contradictions sorted by user-pull-in date so the supervisor + user can
re-disposition systematically.

## Why this doc exists

A SCOPE-RECLAIM entry is a test that fails on work the user **explicitly
pulled into v0.3 scope** by a dated authorization, where the SURFACE
message routes the failure to v0.4 (often via `§5.16 / §5.15 / Wave 6
follow-up` cite). The cite is mis-routed: it claims v0.4 deferral
authority that no dated re-disposition granted. Per the taxonomy rule:

> SURFACE messages that cite "v0.4 / planned" or "§5.16 follow-up"
> without the dated re-disposition are MIS-CITES; the underlying
> failure routes here, not to V0.4-DEFER.

The 756 SCOPE-RECLAIM entries are RELEASE-BLOCKING **until the user
explicitly re-dispositions specific entries to v0.4**.

## §5.16 actual scope (supervisor 2026-05-25)

Reference for the bulk of mis-cite cases. The supervisor 2026-05-25
named the §5.16 JIT-lowering followup workstream's ACTUAL scope as:

- aliased-CoW SEGFAULT
- imported-const ident-eval
- W17-marshal
- Drop codegen
- B2 EnumPayload

**Only these 5 are §5.16-legitimate v0.4.** Any SURFACE citing `§5.16`
for OTHER work (V3-S5 construction-cascade, W17.3-4 per-container
FieldType, W18 content-rendering, comptime trait, W16.2-J PHF-
retirement, etc.) is a mis-cite that routes to SCOPE-RECLAIM.

## Per-pull-in roll-up

### 2026-05-18 — V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade + W16.2-A/B/C

**Pull-in language:** "annotation_targets + annotations_comptime cluster
IS THIS WORK." Plus W16.2-A typed-object-element + W16.2-B trait-object-
element + W16.2-C empty-literal/spread/comprehension.

**Approximate SCOPE-RECLAIM count:** ~520 (combines ckpt-5/-6
construction-cascade + ckpt-2/-3 consumer-cascade).

Per-binary breakdown of major contributors:

| Binary | Count | SURFACE shape |
|---|---:|---|
| iterators | 120 | `Array.iter` / `op_new_array(0)` / `String.iter` / `range` / `filter` / `op_new_typed_array` |
| arrays_vectors | ~100 | empty-`[]` un-resolvable + `op_new_array` + `op_new_typed_array` + V3-S5 ckpt-2 consumer |
| hashmap | ~22 | `HashMap.keys/values/entries/toArray` ckpt-5 SURFACE |
| structs_types | ~40 | `op_new_array` + `range` cascading through TypedObject construction |
| complex_integration | ~36 | closure-param infer-loss from ckpt-2/-5 cascade |
| pattern_matching | ~5 | `op_new_array` + `range` |
| annotations_comptime | 23 | `comptime_target::nb_object_array` mis-cite to §5.16 |
| annotations_runtime | 23 | `op_new_array` + annotation registration cascade |
| annotation_targets | 16 | `op_new_array` + `nb_object_array` |
| comptime | ~13 | `op_new_array` annotation tests |
| closures_hof | ~37 | `op_new_array` + V3-S5 ckpt-2 `map`/`filter`/`flatMap` |
| drop_raii | 3 | `range()` builtin cascade |
| ranges | 1 | `range()` builtin cascade |
| jit | 3 | V3-S5 ckpt-3 tier-2 TypedArrayData |
| snapshots_resume | 1 | `range()` cascade |
| stdlib_regex | 5 | (some of) Array<T> return marshal |
| borrow_refs | 20 | `SetIndexRef` ckpt-5 |
| trait_system | ~6 | V3-S5 |
| list_comprehension | 1 | comprehension carrier element-kind |
| variables_bindings | 5 | destructuring + V3-S5 |
| objects | 1 | object_bracket_access dynamic-key |
| security_permissions | 1 | (V3-S5) |

**Mis-cite pattern:** Many SURFACEs in this family cite "§5.16 follow-up"
or "v0.4 / planned per §5.16" for V3-S5 construction-cascade work. Per
the supervisor 2026-05-25 §5.16 actual scope, V3-S5 is NOT in §5.16.
Mis-cite count: **~280 SURFACEs cite §5.16 incorrectly.**

### 2026-05-21 — Array<string> must work + Len trait + object destructuring must fully work

**Approximate SCOPE-RECLAIM count:** ~80.

Per-binary:

| Binary | Count | Shape |
|---|---:|---|
| pattern_matching | 21 | destructure-bound `unknown` operands |
| objects | 1 | object_destructuring_in_function — `unknown` from destructured params |
| variables_bindings | 5 | array destructure / for-loop destructure / array-rest |
| hashmap | ~10 | (some) value-kind incompat on string-keyed maps |
| jit | 1 | object-spread schema_id `__merged_44_45` |
| list_comprehension | 1 | string-element rejected by comprehension carrier |

### 2026-05-22 — W16.2-J PHF + W17.3-4 per-container FieldType + phase-2c marshal/snapshot + 6 KCs + W18 + comptime trait + KC#2

**Approximate SCOPE-RECLAIM count:** ~140.

Per-binary:

| Binary | Count | Shape |
|---|---:|---|
| stdlib_regex | 5 | W17-marshal-return-arms `TypedReturn::Discriminant(7/8)` |
| stdlib_json | 14 | W17-snapshot-roundtrip `project_typed_return` Discriminant(N) |
| stdlib_modules | 22 | 11 W17-marshal + 11 `set` module-namespace schema missing |
| hashmap | ~33 | W13-hashmap-mutation + W17.3-4 per-V monomorphization |
| structs_types | ~6 | W17.3-4 anon-object construction + WrapTypeAnnotation |
| snapshots_resume | 1 | phase-2c marshal `range()` cascade |
| complex_integration | ~14 | W17.3-4 + HashMap |
| comptime | ~50 | R8 W8 Cluster A `const`-initializer §5.15 mis-cite (44) + comptime-fields (6) |
| variables_bindings | ~5 | W17.3-4 destructure |
| borrow_refs | 9 | `SetLocalIndex` W17-typed-carrier-monomorphization |

**§5.15 mis-cite cluster (44 in comptime):** SURFACE cites
"§5.15 v0.4-concurrency-design-pass" for R8 W8 Cluster A `const`-
initializer comptime-evaluation. But comptime trait was explicitly
pulled into v0.3 on 2026-05-22 (J-CT.0/.1/.2). §5.15 is module-level-
mutable-bindings, NOT comptime-trait support. **44 mis-cites here.**

### 2026-05-26 — LSP-parity + BindingStorageClass opt-in inlay hints

**Approximate SCOPE-RECLAIM count:** ~12.

| Binary | Count | Shape |
|---|---:|---|
| lsp | 12 | trait hover/completion/code-lens/goto-def gaps post-Wave-3 |

## Top mis-cite patterns to refuse on sight

These cite phrases were found ROUTING TO v0.4 in SURFACE messages but
the underlying work is in DATED v0.3 user-pull-in scope:

| Cited anchor | Mis-cite count | Actual disposition |
|---|---:|---|
| `§5.16 JIT-lowering followup` | ~280 | §5.16's actual scope is 5 items only (supervisor 2026-05-25). V3-S5 NOT in §5.16. |
| `§5.15 v0.4-concurrency-design-pass` | ~44 | §5.15 is module-mutable-bindings only; comptime trait pulled in 2026-05-22. |
| `"Wave 6 follow-up"` | ~9 | type_inference `.type()` cluster — Wave 6 is not a v0.4 anchor and no dated re-disposition. |
| `"v0.4 / planned"` (bare) | ~30 | various; bare-cite without a specific anchor — no authority to defer. |
| `"W17-marshal-return-arms follow-up"` | ~30 | W17.3-4 pulled in 2026-05-22; follow-up is in-scope completion, not v0.4. |

## Recommended supervisor + user re-disposition

Three families warrant supervisor + user review for potential v0.4
re-disposition:

1. **comptime / W18 content-rendering complex interactions** (~12) — if
   the W18 supervisor-routing kept some narrow surfaces deferred, those
   may be eligible.
2. **Cross-workspace call-hierarchy** (mentioned in LSP-K close) —
   single-file only landed in v0.3; cross-file may be v0.4. Not in
   SCOPE-RECLAIM today (no failing tests cite it) but flag.
3. **§5.15 module-mutable-bindings** is correctly v0.4. Anything that
   genuinely needs module-level `let mut` (not const) and that the test
   was asserting on module-mutable behavior is V0.4-DEFER, not SCOPE-
   RECLAIM. Audit didn't surface any such tests, but worth confirming.

**All other SCOPE-RECLAIM entries are release-blocking by default.**

## Action items

1. Supervisor + user review this doc and either:
   a. Confirm full SCOPE-RECLAIM bucket (756) is release-blocking, OR
   b. Re-disposition specific named families to v0.4 with dated
      authorization (must be a new dated authorization explicitly
      naming the family).
2. After (1), the next-release-gating set = FN-REG-CORRECTNESS (367) +
   surviving SCOPE-RECLAIM.
3. Add a `check-no-mis-cite` gate (grep verify that no SURFACE message
   cites v0.4 / §5.16 / §5.15 / Wave-6 anchors without a corresponding
   dated re-disposition in the pull-in table).
