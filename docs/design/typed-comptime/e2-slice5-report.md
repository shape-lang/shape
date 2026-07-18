# E2 #18 slice-5 report — TOTAL U03/U07 deletion

Review-mandatory slice. Closed on branch `adr009/e2` at head `45ebd6bc`;
independent Opus review panel **PASS, high confidence, 0 blocking / 2 LOW**. This
is the slice narrative: Part A (the block-form replace-body carrier), 5b (the
`extend_method_literal` producer + the ~89-site fixture migration), Part B (the
companions, the one deletion commit, and the fix round), followed by process
notes (the concurrent-writer incident and the capstone handoff).

The slice removed the WHOLE closed-under-callers U03/U07 inventory in one
deletion commit, only after parity + review — E2-D8 discipline. It landed against
a green battery, not deletion-first (refused on gate hygiene).

---

## Part A — block-form replace-body typed carrier

Closed at `3072de5e` (`3c701ee7` carrier + `3072de5e` pins). Parity gate GREEN
steps 1–6 (check-all 0, builtins 251/0, e2 pins 5/5, st-annotations 111/12 exact
baseline, lsp 886/0, cli 6/6).

The block-form `replace body` typed carrier stashes into `COMPTIME_REPLACE_BODIES`
at handler-compile time and clears at `execute_comptime_with_annotation_handler`
ENTRY (the new clear point that Fork B's obstacle analysis had identified as
missing); a new `__emit_replace_body_checked(index)` builtin reads it. The legacy
`__emit_replace_body` dies whole once caller-free (Part B).

**Scope checkpoint rulings (2026-07-18):**

- **Q1 CORRECTED.** `statements.rs:718` `ReplaceBodyExpr` is a WORKING
  source-string transport today, not dead. The variant **STAYS**; its emit is
  rerouted to a clean `[C0928]` rejection (per E2-D7). No pest change, **zero AST
  variants deleted** — the phase-1 ~19-file exhaustive-match fan-out was an
  overestimate.
- **Q2 OUT.** `__emit_extend` is the direct-extend authority (E1's JSON-protocol
  class). Recorded in the E1 hand-off inventory.

---

## 5b — the `extend_method_literal` producer + the fixture migration

### Why 5b existed

When Part B's closure map was surfaced, the STOP-and-surface disclosed a live
surface the earlier denominator had missed: fixtures written against the
source-string `extend` arm that would break on its deletion — including
int-returning literal-body methods that `extend_method` (template-body) cannot
express. Supervisor ruled **option (a) as slice 5B before Part B**: build a
literal-body method producer, migrate every fixture preserving its original
runtime assertions, then delete against green.

Rejected alternatives (named walk-backs, refused):
rejection-rebaseline of surviving-machinery coverage (would delete real
coverage) and a keep-arm (the canonical walk-back).

### 5b-1 — the producer

Committed `0a58fd47` (+ `0813895d`, an `E0433` `Item::Extend` qualify fix). A
separate builtin `extend_method_literal(type, method, ret, value)` was RATIFIED
over an overload (strict-typed params; the `item_fn` precedent). The literal
decode reuses `literal_expr_from_slot` VERBATIM; the assembly shares
`build_extend_item_with_method_body` with the template producer (single
authority). Both registration surfaces are wired, including the
`COMPTIME_BUILTIN_FORWARDERS` row (the two-surface lesson from slice 4.5).

### The migration denominator — five sweep corrections

The migration list of record was reached through five successive scope
corrections, each surfaced by the implementer, each verified by the supervisor.
They are the empirical map of where source-string generation actually lived:

1. **f-string `extend (f"…")`** — the slice-0 parity denominator; 53 raw call
   sites across 16 files (classifier: 41 live, 12 were comments).
2. **plain-string `extend ("…")`** — ~48 additional callers across ~14 files
   (supervisor-verified 51 raw / 17 files); the entire 5b analysis had grepped
   only `extend (f"`. SURFACE-dominated (closure/RAII/borrow/generics bodies).
3. **`tools/shape-lsp`** — 11 live sites the "code-complete" sweep missed;
   produced the binding **workspace-wide both-spelling grep** protocol
   (crates/tools/bin, implementer-pasted + supervisor-rerun before deletion GO).
4. **`replace module ("…")`** — never matched any `extend`-shaped grep; 5 live
   tests + 3 `cfg(any())` dead (the A-part-3 collateral).
5. **`.shape` CLI fixtures** — the 9 C1/C2 fixtures (surfaced in the fix round),
   plus an F1 hand-off note for book snippets under `../shape-web`.

Denominator of record: **~89 sites / 14 files migrated, 14 held by design** for
Part B. The supervisor's workspace closure sweep MATCHED the implementer's
(anything-else bucket EMPTY).

### 5b-2 — classification and the five batches

The read-only classifier bucketed each live site: **`item_fn`** (literal-body
free functions), **`extend_method_literal`** (literal-body methods),
**`extend_method`**-template (computed bodies), and **SURFACE** — multi-statement
/ closure / async / param / generic / multi-method bodies (the C1/C2 battery
rich-body fixtures), which migrate to the DIRECT `extend target { method … }`
STATEMENT form (the d7 route: arbitrary parsed-AST bodies, positional owner
binding, survives slice 5 as `__emit_extend`/E1 territory).

Migration ran in five gated batches, both spellings per file:

- **Batch 1** (f-string): `nominal 130b33b9`, `typed_comptime 2891d979`,
  `flagship e459068f`, `frozen_reflection 9f0f1537` (10 sites). Interim gate FULL
  GREEN.
- **Batch 2**: `generated_rename 4d6b643c` (EML×3), `lineage d8565979`,
  `teardown/slice4 209717cf`, `denial 88cca6a4`, `gate_totality fc695b94`
  (Option-C×3, generic-method parse verified).
- **Batch 3**: `annotation_import_pipeline a9c4b3d9`, `declared_tests 4eacb0e8`,
  `generated_capture f351bc5e`, `slice4 f32c86b0`, `reference_rejections
  6b60eddd` + the isolated `fa75c73c`.
- **Batch 4**: the c2 batteries `caf5017a` + `8bab01b5`, then `teardown
  09c41bd0` + `declared_capture b2df1f11`.
- **Batch 5** (`tools/shape-lsp`, pervasively transport-subject): `:881` clean at
  `faa1e3af` + a 6-site isolated ARBITRATION commit `bdc4209d` (the computed-name
  group stays `GeneratorControlled`; the application-binder pair classifies
  identically). **5b-2 COMPLETE at `bdc4209d`.**

**Constraint-1 cleared empirically.** The 3 module-binding trigger migrations
(`:133`/`:166`/`:206`, fn→method, assertions byte-unchanged) went GREEN in vmlib
3141/7 exact baseline, proving the `C0906`/`C0902`/`C0912` capture arms are
container-agnostic.

**One batch-3 newcomer** (`slice4.rs` mutation-control: a multi-line indented
`.replace` fragment broke on re-indent → the control no-op'd) was fixed at
`3e3b96c9` with an `assert_ne!` loud-fail guard + a sibling `.replace` audit (no
vacuous passes).

**st-comptime lane lesson.** An interim `st-comptime` anomaly resolved to
parallel-state flapping: single-threaded branch-vs-main are both **261/3
byte-identical**; st-comptime MUST run `--test-threads=1`.

**Per-transport-subject rulings** (tests whose SUBJECT was the deleted route):
`d12_*` → rejection-rebaseline onto the new `[C0929]`; `generated_snippet_…
vm_and_jit` → RETIRE (residual carried by direct-route siblings); `u10` →
migrate as EML now (subject is the positional-delivery property, transport-
independent); the `:5198`/`:5223` snippet-span tests → RETIRE conditional on
producer-route span-anchor pins; the lsp `[C0910]` source-unavailable test →
RETIRE under a **reachability obligation**. **Expected-FAILED after Part B: 12 →
10** (d12 out-as-green + generated_snippet retired), disclosed for all downstream
differentials.

---

## Part B — companions, the deletion, the fix round

### Companions

`e0098ebc` (generated_method_runtime retire + fold-in); `21b73bf8`
(functions_annotations span-test → producer-route-pin rewrites + fold-ins);
`94f91a7b` (tip-amend of the ungated `4ebc8090` — the `d12`→C0929 rebaseline +
`u10`→EML, an ACCEPTED deviation from per-commit-green: exactly one designed-red
intermediate test, green at the deletion head); `3f21266b` + the C0910 retire
part-1/2 chain (5-retire + 2-keep: `presentation.rs:104` kept-renamed as a
byte-duplicate of a surviving sibling; `rename_tests.rs:168` unavailable-context
test KEPT as defensive surface); `dbb3a738` (the concurrent-writer's Step-1
edits, verified correct and adopted as history — see process notes);
`04904540` (A-part-3 replace-module collateral).

**Reachability verdicts** (traced; agent + supervisor dispositions in
`8399e5c7`): `[C0910]` DEAD post-deletion — 3 tests + fixtures retire, the
emission block (`query.rs:408-418`) + constant (`:31`) die IN the deletion, and
the None-branch is REPLACED with a loud surface-and-stop invariant error (never
silent, never the dead reparse-flavored text). Snippet-binder classification is
SHARED/REACHABLE — the generator-span scan (`compiler_queries.rs:193`) serves
surviving routes; nothing production dies; the `:1268` fixture REROUTES to
`extend_method_literal`.

### The deletion commit

`cba541fb` — the one deletion commit: 11 symbols removed, `__CheckedItem`-only,
the `[C0929]` message minted verbatim (`comptime_builtins.rs:757`), `[C0910]` →
accumulator-poison. Tree stat: **16 files changed, 164 insertions(+), 492
deletions(-)** (the `comptime_builtins.rs` core alone +73/−376). Verification:
every deleted symbol re-grepped across crates/tools/bin — zero live code refs; no
orphaned imports. (The roster's "net −488" characterization does not match the
tree net of −328; see the close report's discrepancy note.)

`0de7b244` — companion B: the d12 successor pin, the replace-module `[C0929]`
twin, and the `:1268` reroute. **Capstone landed here.**

### The fix round

The definitive battery had 6 suites green at predicted baselines but **two red
clusters**:

1. **lsp-lib 878/4** — the "C0910 DEAD" verdict refuted in part (REPLACE-BODY +
   CALLABLE captures legitimately have `source_map == None`; the new poison
   QUARANTINED them, breaking slice-3's C0911 flip test + 3 callable-capture
   tests). Fixed at `6ab0f9db`: remove the poison — `source_map == None` routes
   to the LISTED full-semantics view, no issue. The capstone proved
   `ValidationFailed` unconstructible post-deletion, and the reviewer confirmed
   `capture_at` ALREADY skipped source-map-less captures at base
   (`query.rs:302-306` at `d804cde4`) — so the net effect is one fewer
   informational issue.
2. **cli natives** — generated/edited methods not materializing end-to-end
   (`Method not found on type 'Job'`). Fixed at `45ebd6bc` via option (d): all 9
   C1/C2 `.shape` CLI fixtures rewritten to the direct `extend target { }` form
   (the 5th sweep correction; the capstone's totality-collision STOP dissolved by
   the 5b-2 SURFACE-migration precedent it had lacked context for).

**Fix gate ALL GREEN** at `45ebd6bc`: lsp-lib 882/0 (4 newcomers cured), cli 6/6
**+ the C1 matrix 9/9 native including the async cell**, st-annotations 110/10
exact, check-all 0, verify-merge success.

---

## Process notes

### Process violation — mid-lane edits (Part A)

During Part A the implementer made mid-lane working-tree Part-B edits on a
crossed message; the `vmlib-full` gate step was contaminated (an `E0425` on
half-deleted source) and invalidated. Binding rule issued: **ZERO edits between a
lane announcement and the supervisor verdict; ask-and-wait on any
crossing-uncertainty.**

### Concurrent-writer incident (contained)

The original implementer never processed a stand-down instruction across three
successive message crossings — it kept executing into the worktree WHILE a fresh
capstone agent examined the same tree. The capstone caught the live mtime churn
and correctly STOPped instead of racing. The original's Step-1 edits landed as
`dbb3a738` (verified correct, adopted as history); it then announced beginning
THE DELETION itself, and the supervisor TERMINATED its process outright
(`TaskStop`). The tree was verified clean at `dbb3a738` post-termination — no
race occurred. **One writer (the capstone) was physically enforced.**

### Capstone handoff

At the capstone boundary the original implementer flagged its own context depth
and STOOD DOWN — a protective judgment the supervisor ruled correct twice. A
fresh Opus capstone agent was dispatched with the complete verbatim spec (the
C0910 retire part-2 → the pure deletion commit with the C0929 mint + the C0910 →
invariant-error + the full 11-symbol list + the E1 hand-off → companion B → the
workspace sweep proof + the recomputed expected-FAILED set). The capstone
executed the deletion, companion B, and the sweep proofs, landing at `0de7b244`;
the fix round followed to `45ebd6bc`.
