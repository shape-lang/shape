# v0.3.2 shape-test classification audit — TRUTH-SET

**HEAD:** `82f049dd` (post-v0.3.2 tag).
**Audit closed:** 2026-05-27.
**Trigger:** user 2026-05-26 authorization after observing 1065 failing
tests in `shape-test` shipped silently through v0.3.0 / v0.3.1 / v0.3.2.
**Method:** ~50 parallel slice agents (audit-day pattern) + team-lead
sequential batch for slow-build binaries. One audit doc per test binary
under `docs/cluster-audits/v0.3-classification/<binary>.md`.

## Aggregate (refined post error_handling + regression re-classification)

| Class | Count | % | Disposition |
|---|---:|---:|---|
| **FN-REG-CORRECTNESS** | **459** | **35%** | **RELEASE-BLOCKING** — real correctness regressions, must fix before next tag. |
| **SCOPE-RECLAIM** | **761** | **58%** | **RELEASE-BLOCKING** until user re-dispositions to v0.4. Work pulled into v0.3 by dated user authorization. |
| FN-REG-DIAGNOSTIC | 57 | 4% | Per-test fixture text update. |
| V0.4-DEFER | 40 | 3% | Legitimately v0.4 (§5.16 B2 EnumPayload — 27 in enums + 12 in error_handling + 1 IntrinsicVecAddI64). |
| INFRA-FLAKY | 1 | <1% | Binary-level SIGSEGV under parallel cargo (complex_integration). |
| UNKNOWN | 4 | <1% | error_handling(1) const_complex_expression + iterators(1) forEach + pattern_matching(1) recursive-match + regression(1) should_panic-no-longer-panics. |
| **TOTAL** | **1322** | | (over-count from per-shape grouping vs per-test 1065 raw fails) |

**Release-blocking total: 1220 fails (92% of corpus).**

## Per-binary table

```
binary                      C   D   SR  V4  F  U
─────────────────────────────────────────────────
annotations_comptime        0   0   23   0  0   0
annotations_runtime         0   0   23   0  0   0
annotation_targets          0   0   16   0  0   0
arrays_vectors             10   3  101   0  0   0
async_concurrency           0   0    0   0  0   0  ← all-green
book_doctests               2   0    0   0  0   0
book_policy                 0   0    0   0  0   0  ← all-green
borrow_refs                 6   0   30   0  0   0
closures_hof              123   0   37   0  0   0  ← largest C
complex_integration        12   0   50   0  1   0
comptime                    2   1   70   0  0   0
control_flow                5   5    2   0  0   0
datetime_stdlib             0   0    0   0  0   0  ← gated #[cfg(any())]
drop_raii                   0   0    3   0  0   0
e2e                         1   0    0   0  0   0
e2e_gated                   0   0    0   0  0   0  ← Cargo-feature gated
enums                      44   0   19  27  0   0
error_handling             76   1    0  12  0   1  ← Result `!!`/`?` runtime broken
extend_blocks               0   0    0   0  0   0  ← all-green
features                    2   0    0   0  0   0
functions                   0   0    0   0  0   0  ← all-green (214/214)
generics                    0   0    0   0  0   0  ← all-green
hashmap                     0   0   65   0  0   0
integration                 1   0    0   0  0   0
iterators                   0   0  120   0  0   1
jit                         0   0    3   0  0   0
list_comprehension          0   0    1   0  0   0
literals                    1   1    0   0  0   0
lsp                         8   4   12   0  0   0
module_distribution         0   0    0   0  0   0  ← all-green
modules_visibility          7   1    0   0  0   0
native_interop              0   0    0   0  0   0  ← all-green
objects                     2   0    1   0  0   0
objects_arrays             19   0   10   0  0   0
operators                  14   0    2   0  0   0
package_infrastructure      0   0    0   0  0   0  ← all-green
packages_bundles            0   0    0   0  0   0  ← all-green
pattern_matching            4   0   28   0  0   1
query_language              3   2    2   0  0   0
ranges                      2   0    1   0  0   0
regression                 22  27    5   0  0   1  ← refined per serial-run evidence
security_permissions        1   0    1   0  0   0
smoke_test                  0   0    0   0  0   0  ← all-green
snapshots_resume            0   0    1   0  0   0
stdlib_crypto               1   0    4   0  0   0
stdlib_http                 0   5    0   0  0   0
stdlib_json                 0   0   14   0  0   0
stdlib_math                 2   0    0   0  0   0
stdlib_modules              2   0   22   0  0   0
stdlib_regex                0   0    5   0  0   0
strings_formatting         11   0   12   0  0   0
structs_types               3   0   46   0  0   0
tables_queryable            1   0    1   0  0   0
traits                     34   0    2   0  0   0
trait_system                1   0    6   0  0   0
type_aliases_unions         0   0    0   0  0   0  ← all-green
type_inference             17   0   18   0  0   0
typesystem                  0   0    0   0  0   0  ← 0 tests (orphan subdir)
variables_bindings         20   0    5   0  0   0
window_functions            0   7    0   0  0   0
wire_protocol               0   0    0   0  0   0  ← all-green
─────────────────────────────────────────────────
TOTALS                    459  57  761  40  1   4  = 1322
```

C = FN-REG-CORRECTNESS · D = FN-REG-DIAGNOSTIC · SR = SCOPE-RECLAIM · V4 = V0.4-DEFER · F = INFRA-FLAKY · U = UNKNOWN

## All-green binaries (15)

`async_concurrency`, `book_policy`, `datetime_stdlib` (gated), `e2e_gated`
(feature-gated), `extend_blocks`, `functions` (214/214), `generics`,
`module_distribution`, `native_interop`, `package_infrastructure`,
`packages_bundles`, `smoke_test`, `type_aliases_unions`, `typesystem`
(orphan; 0 tests wired), `wire_protocol`.

## Top FN-REG-CORRECTNESS clusters (release-gating fix work)

1. **closures_hof (123)** — closure-param type-inference loss (77), var-
   capture upvalue allocation broken (23), NativeView carrier-mislabel
   (19). **Core ADR-006 closure semantics.**
2. **error_handling (76)** — Result `!!` context-operator broken at
   runtime (26), Result `?` try-operator broken at runtime (15), `?` +
   propagation chain regressions (9), `!!`+`?` combined edge-cases (16),
   TryInto/Into semantic-diagnostic (5), array-bounds returns-None
   contract broken (5).
3. **enums (44)** — enum equality `==/!=` rejected on statically-
   resolved same-enum operands; wire_conversion soundness panic;
   Option print not unwrapping; Result `!!`/`?` broken.
4. **traits (34)** — 30/34 collapse to W1 trait-operator-coverage
   regression (operator unresolved on user types / Display not
   dispatching / trait return-type not threaded). Likely single bisect
   fixes ~30.
4. **variables_bindings (20)** — width-typed locals
   (`i8`/`i16`/`i32`/`u8`/`u16`/`u32`/`u64`) leak tagged wrapper
   `Object {"I8": Number(100)}` instead of projecting to declared
   `-> int`. 19 tests one root cause.
5. **objects_arrays (19)** — filter-on-TypedObject (user 2026-05-27
   repro family), negative-indexing OOB-returns-None regression,
   HashMap immutable-`.set()`-returning-Self rejection.
6. **type_inference (17)** — core strict-typing inference regressions;
   includes silent-wrong-output (`test_infer_nested_function_calls`
   returns ~0 instead of 30); `.type()` cluster mis-cites Wave 6.
7. **operators (14)** — see per-binary doc.
8. **complex_integration (12)** — 2 SIGABRT-class crashes (deep nested
   struct access; 137-TB OOM on nested TypedObjects).
9. **strings_formatting (11)**.
10. **arrays_vectors (10)** — negative-indexing OOB family.

**Memory-unsafety / silent-wrong-output sub-family** (especially severe;
per user 2026-05-20 no-known-incorrectness binding):
- 3 SIGABRT-class OOMs on nested-struct access (complex_integration:2 +
  structs_types:1; ~134TB allocation requests).
- ADR-006 §2.7.13 invariant violation (`DerefStore kind drift`) on
  struct field mutation (structs_types:2).
- wire_conversion hard panic on enum-discriminant mismatch (enums:2).
- pointer-as-float silent-wrong-output (regression:1 — `bug5` returns
  2.08e-322).
- bitwise ops on non-int silently reinterpret memory (small-batch
  agent flagged 8 fixtures — security/type-safety class).
- borrow-check `test_violation_ref_in_let_binding` runs instead of
  erroring (silent borrow-check bypass).

## SCOPE-RECLAIM root-cause families (release-blocking)

See `SCOPE-RECLAIM.md` for the full per-binary breakdown with dated-
pull-in cross-references. Headline distribution by root cause:

| Root cause | Approximate count | Dated pull-in |
|---|---:|---|
| V3-S5 ckpt-5/-6 op_new_array construction-cascade | ~340 | 2026-05-18 |
| V3-S5 ckpt-2/-3 consumer-cascade (filter/map/range/String.iter) | ~180 | 2026-05-18 |
| W17.3-4 per-container FieldType (incl. W17-marshal-return-arms) | ~110 | 2026-05-22 |
| Object destructuring "must fully work" | ~80 | 2026-05-21 |
| HashMap rebuild (per-V monomorphization, W13 mutation contract) | ~65 | 2026-05-21/22 |
| Comptime trait | ~70 | 2026-05-22 |
| W18 content-rendering | ~12 | 2026-05-22 |

**Mis-cite pattern:** SURFACE messages routinely cite `§5.16 v0.4`,
`§5.15 v0.4-concurrency`, "Wave 6 follow-up" for work that is in
DATED v0.3 user-pull-in scope. The taxonomy explicitly rejects these
cites — the SURFACE annotation is mis-routed, not the work.

## Genuine V0.4-DEFER (28)

Only one family qualifies: **enums::B2-EnumPayload (27)** — `match Some(x) => x + ...`
where match-arm payload binding has type `unknown`. This is precisely
the §5.16 supervisor-2026-05-25-named v0.4 scope, surface-and-stops
cleanly. Plus `objects_arrays::array_concatenation_with_plus` (1) —
SURFACE explicitly cites `v0.4 / planned` for `IntrinsicVecAddI64`.

## FN-REG-DIAGNOSTIC (65)

Mostly stale text in test assertions. Largest cluster:

| Binary | Count | Pattern |
|---|---:|---|
| regression | 36 | `jit_*` arithmetic expects `WireValue::Number`; new int/number split returns `Integer` |
| window_functions | 7 | `Unknown method 'X'` → `no method 'X' on receiver kind Ptr(TypedArray)` |
| stdlib_http | 5 | http API arity changed in R8 W6 G.3; 8 fns now require options arg |
| control_flow | 5 | Various stale-text |
| lsp | 4 | LSP diagnostic-message text drift |
| query_language | 2 | Stale text |
| arrays_vectors | 3 | `contains` no-method text; `indexOf` "2.0" vs "2" |
| comptime / literals / modules_visibility | 1 each | Various |

Fix path: per-test fixture text updates (NOT bulk-update; per-test per
the binding).

## UNKNOWN (4)

- **error_handling (1)** — `const_types_strings::const_complex_expression`
  (trivial `const X = 3 * 4 + 2; X` fixture; no obvious connection to
  Result/Option/`?`/`!!`/B2 patterns). Needs narrow bisect.
- **iterators (1)** — `stress_reduce_collect::test_array_foreach` —
  closure-capture / borrow-solver path needing bisect.
- **pattern_matching (1)** — `t115_match_recursive_function` — `int | number`
  union shape needing fixture-read to disambiguate generic-instantiation
  vs strict-typing destructuring.
- **regression (1)** — `qa::regression_med_13_mutable_params` —
  `#[should_panic]` that no longer panics; needs re-run with attribute
  removed.

## v0.3.3 release-gating fix set (per user 2026-05-27 disposition (B))

Per user 2026-05-27 verbatim: *"everything needs to work, v0.3.3 is the
target. we are talking about a programming language. correctness is key."*

1. All **FN-REG-CORRECTNESS** (459) — RELEASE-BLOCKING.
2. All **SCOPE-RECLAIM** (761) — RELEASE-BLOCKING (no re-disposition).
3. **FN-REG-DIAGNOSTIC** (57) — per-test fixture updates in LOCKSTEP
   with the language fix that drove the new diagnostic.
4. **V0.4-DEFER** (40) — issue-file for v0.4; allowlisted (B2 EnumPayload
   §5.16 cluster + IntrinsicVecAddI64 bare SURFACE).
5. **INFRA-FLAKY** (1) — complex_integration parallel-cargo OOM;
   isolation defect.
6. **UNKNOWN** (4) — narrow bisects before reclassify.

**Release-blocking total: 1220 fails. Allowlist pinned at 41 (40 V4 + 1 F).**

## Discipline findings

1. **Mis-cite pattern is endemic.** Hundreds of SURFACE messages cite
   v0.4 anchors for work in dated v0.3 user pull-ins. Forbidden-pattern
   text already exists in CLAUDE.md but the SURFACE messages weren't
   audited for it. Recommend a `check-no-mis-cite` gate that grep-
   verifies every NEW SURFACE message against the dated-pull-in list.

2. **"Pre-existing baseline" as exemption category** masked 367
   correctness regressions + 756 in-scope-deferral mis-cites for ~9
   days across v0.3.0 / v0.3.1 / v0.3.2 tags. Per Step 3 of supervisor
   ratify, the category is removed; replaced with per-test classified
   allowlist with NO FN-REG-CORRECTNESS or SCOPE-RECLAIM entries.

3. **Smoke-only pre-tag gate** missed every one of these. The
   shape-release skill now requires `just test-fast` + planned
   `cargo test -p shape-test --no-fail-fast` allowlist-diff gate.

4. **shape-test corpus had no per-binary tracking** in CI dashboards;
   the 1065 fails were aggregate-only. Recommend per-binary fail-count
   tracking in CI to detect regression slips.

## See also

- `TAXONOMY.md` — full classification rules + dated user pull-ins.
- `SCOPE-RECLAIM.md` — per-binary SCOPE-RECLAIM cross-reference + mis-
  cite enumeration (to be written next).
- `ALLOWLIST.md` — pre-tag-gate allowlist proposal (to be written next).
- Per-binary docs in this dir.
