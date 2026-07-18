# E2 #18 — close report (draft)

Ticket: ADR009-E2 #18 — `CheckedItem`/`CheckedModule` replace ItemFragment
sentinels and body/module source reparsing (legacy classes **U03/U07**).
Branch `adr009/e2`. Base `52fc13f8` (main with C1+C2 merged); first E2 commit
`3a1aa469`; program head `45ebd6bc`. Fable supervisor.

This is the slice-6 close draft: documentation only, no production code. Every
hash, count, and finding below is transcribed from the AGENTS.md `adr009/e2`
roster row and the `52fc13f8..45ebd6bc` git log. One numeric discrepancy is
flagged in the [discrepancy note](#discrepancy-note).

---

## Headline

**The U03/U07 source-string comptime-generation route is DELETED TOTALLY.**
There is no surviving source-reparse arm, no carve-out, no deferral to E6.
Comptime code that generates declarations now flows exclusively through typed
carriers:

- **`CheckedModule`** (`ComptimeDirective::ReplaceModuleChecked`) — `replace
  module (expr)` with a typed item carrier.
- **`CheckedItem`** — `item_fn(name, ret, value)` mints a typed literal-body
  free function.
- **`CheckedReplaceBody`** (block-form `replace body`) — pre-analysis
  materialization through the internal C2 `CheckedBody` validator, which flipped
  the C0911 quarantine tripwire (slice 3).
- Method producers **`extend_method(...)`** (computed template body → native
  f-string) and **`extend_method_literal(...)`** (literal method body).

The old routes are rejected at the boundary with two named diagnostics:
`[C0928]` (expr-form `replace body (expr)` — use the block form) and `[C0929]`
(the source-string `extend`/`replace module` generation route — pass a typed
carrier). Roughly **89 test-fixture sites across 14 files** plus **9 `.shape`
CLI fixtures** were migrated to typed or direct-`extend` forms, preserving their
original runtime assertions. Shipped stdlib is **100% off the source-string
arm** — **E2-Q1-A totality is MET**.

The directive carrier was already typed at program start
(`ComptimeDirective::{ReplaceBody{Vec<Statement>}, ReplaceModule/ExtendItems
{Vec<Item>}}`); the real work was replacing the mini-VM string/JSON **transport**
with typed handles and deleting the `__ComptimeItemFragment` schema and the
source-reparse machinery around it.

---

## Program summary — 7 slices

Slices 0, 1, 2, 3, 4, 4.5, 5. User-ratified **E2-Q1 OPTION A** (stdlib migration
in-scope, deletion TOTAL) and **E2-Q2 PULL-INTO-E2** (the serialize computed-body
producer built inside E2 rather than deferred to the E-track) shaped the slice
list; 4.5 was pulled in mid-program to keep totality intact.

| Slice | Closed at | What it landed |
|---|---|---|
| 0 | `ab7c6d47` (docs `3a1aa469`) | Pre-analysis materialization spike (the E2-D6 STOP-decision slice) + parity denominator. Verdict **FEASIBLE-WITH-EXISTING-MACHINERY, no STOP**: the executed-declaration pre-pass already runs comptime-post handlers pre-analysis; `ReplaceBody` was simply dropped at the directive loop's `_ => continue`; the C0911 fix is directive wiring, not analyzer reordering. Review-mandatory. |
| 1 | `5190d41b` (transport `9ce54d3d`) | Typed replace-module transport (`ReplaceModuleChecked`/`CheckedModule`) via a fragment-slot route beside the byte-unchanged legacy string arm (ruled E2-D8 staging, Option C). Full provenance/hygiene/reserve on the typed route. **Named finding**: the legacy `ReplaceModule` consumer raw-swaps `*module_items = items` with zero provenance (dies whole in slice 5). |
| 2 | `b9189d4c` (`216ec759` shared helper + `ebb7ce04` carrier) | `item_fn` yields a typed `CheckedItem` carrier via an opaque-index `TypedObject` handle into a thread-local, cleared-before-run like the directives channel; D10 both-tiers native proof; E2-D9 closure-free tripwire. |
| 3 | `d9abe4dc` (9 commits: `f34062ba` wiring … `d9abe4dc` zero-fallback flip) | Replace-body via the C2 `CheckedBody` validator — **flips the C0911 replace-body quarantine tripwire**. Review-mandatory; independent Opus review **PASS, zero findings, high confidence**. |
| 4 | `692338b0` (`edd06def` derive + `3c0809cf` tools) | Stdlib migration onto `item_fn`: `serde/derive.shape` + `llm/tools.shape`. `string_lit` now has zero live shipped-stdlib callers. `serde/serialize.shape` untouched (deferred to 4.5 per E2-Q2). |
| 4.5 | `d804cde4` (chain `79b168d8`…`d804cde4`) | The minimal Dec-73 checked-splice `extend_method` producer (Option B: typed template builder → native f-string via the language's universal f-string parse, single formatter authority). `serde/serialize.shape` migrated. Review-mandatory; independent Opus review **PASS, 0 blocking, 4 non-blocking**. **E2-Q1-A totality MET.** |
| 5 | `45ebd6bc` | **TOTAL U03/U07 deletion.** Part A block-form replace-body typed carrier; 5b `extend_method_literal` producer + the ~89-site fixture migration; Part B the one deletion commit (`cba541fb`); fix round `45ebd6bc`. Review-mandatory; independent Opus review **PASS, high confidence, 0 blocking / 2 LOW**. |

The slice-5 narrative (Part A/5b/Part B, the sweep corrections, the
concurrent-writer incident, the capstone handoff) is in the companion
[e2-slice5-report.md](e2-slice5-report.md).

---

## E2-D2 inventory correction (record)

The issue #18 body listed `__type_probe`/`parse_type_annotation_payload` in E2's
deletion inventory. **That is an inventory error against the U-class catalog.**
Per **E2-D2** (`e2-decisions.md`):

- `__body_probe` / `parse_function_body_payload` **is** E2 scope (U03).
- `__type_probe` / `parse_type_annotation_payload` feed `set param` / `set
  return` (**U02**) and belong to **E1/E5**. They were **NOT** deleted in E2.

Recorded here for the #18 close per the E2-D2 obligation.

---

## Findings ledger

Transcribed from the roster row. Items 1–3 and 9 are main-side / behavior-change
debt surfaced by E2; 4–8 are E2-internal lessons and dispositions.

1. **shape-test harness JITExecutor empty-capture hole.** The shape-test harness
   JITExecutor leg captures EMPTY output for annotation-generated programs
   (pre-existing main-side harness debt; the same family as the 3
   `generated_method_runtime` baseline names). Mitigation in E2: replace-body
   runtime pins were reworked VM-only and the JIT proof moved to the CLI harness
   (`jit_c2_install_native`).
2. **Multibyte-char panic in diagnostic rendering.** The diagnostic renderer
   PANICS on multibyte characters when slicing source ("byte index not a char
   boundary" on `→`). Main-side debt, discovered during the batch-3 fixture
   migration.
3. **Imported-annotation handler failures swallowed.** Failures inside an
   imported annotation handler surface only as a downstream method-not-found
   error — a diagnosability hole, main-side.
4. **Empty-array-init unavailable in comptime (`[C0001]`).** An empty typed-array
   initializer `[]` is unavailable in comptime code; `.push()` and non-empty
   literals work. `serialize.shape` was restructured to seed its array from the
   first field's real data.
5. **Pins-in-deep-tests-gated-mod lesson.** Slice-4.5 pins initially landed in a
   `deep-tests`-gated module and therefore never ran (a positive-claim lesson);
   they were relocated to a gate-runnable module.
6. **C0927/C0928/C0929 uncoded string tags.** All comptime builtin errors are
   `Err(String)`; the `[C0927]`/`[C0928]`/`[C0929]` tags are string-embedded, not
   routed through the coded-diagnostic path. **Shared follow-up: coded comptime
   diagnostics** (the C092x family; also C2's L2). A GitHub follow-up issue is
   drafted in [Appendix A](#appendix-a--drafted-follow-up-issue-c092x-uncoded-comptime-diagnostics)
   for the supervisor to file at close (mirroring C2's #59 for the D6 re-arm).
7. **Refuted-in-part reachability verdict ([C0910]).** The reachability analysis
   verdict "[C0910] source-unavailable is DEAD post-deletion" was **refuted in
   part**: it analyzed extend-route producers only, but REPLACE-BODY and CALLABLE
   captures legitimately have `source_map == None`. The final fix removed the
   accumulator-poison and routed `source_map == None` to the LISTED
   full-semantics view (no issue), which the capstone proved is what the base
   already did at `capture_at` (`query.rs:302-306`, supervisor-confirmed at
   `d804cde4`) — net effect: one fewer informational issue.
8. **Reviewer L1 note (slice 5).** The span-drift corruption hint was traded away
   as part of the finding-7 disposition — noted, non-blocking.
9. **0-field `@to_json` behavior change.** A 0-field `@to_json` now **rejects
   loudly** where it previously silently emitted `{  }`. Disclosed and pinned.

Latent escape note (slice 4.5): `extend_method` hardcodes Braces interpolation
mode, where `$`/`#` are literal; a future Dollar/Hash-mode caller would need
sigil escaping.

---

## Capability gaps (named, user-visible)

Both gaps are consistent with the user-ratified Q1-A / E2-Q2 staging; the user
re-scopes the E-track if either is wanted sooner.

- **Free functions with non-literal bodies.** ~10 fixtures generate FREE
  functions with closure bodies. There is **no surviving typed route** post-U03
  (`item_fn` is literal-body-only; `extend_*` producers are methods-only). These
  return with **`quote item`** (E-track). Slice-5's disposition rewrote the
  affected fixtures fn→method (the capture gate is container-agnostic — it
  follows the node stamp, slice-3-verified), preserving assertions.
- **Annotation declarations inside replaced modules.** Not expressible on the
  typed surface; returns with **`quote item` / `quote module`** (E-track).

**Discovery trail — the five sweep-scope corrections** (each widened the
migration denominator; they are the empirical map of where source-string
generation actually lived):

1. f-string `extend (f"…")` — the original slice-0 parity denominator.
2. Plain-string `extend ("…")` — ~48 additional callers across ~14 files
   (supervisor-verified 51 raw / 17 files), SURFACE-dominated.
3. `tools/shape-lsp` — 11 live sites the "code-complete" sweep missed; produced
   the binding **workspace-wide both-spelling grep** protocol.
4. `replace module ("…")` — never matched any `extend`-shaped grep; 5 live tests
   + 3 `cfg(any())` dead (A-part-3, the 4th sweep gap).
5. `.shape` CLI fixtures — the 9 C1/C2 fixtures (5th correction, fix round) +
   an F1 hand-off note for book snippets.

---

## E1 hand-off inventory

E1 (typed `RewritePlan` directives; delete the JSON directive protocol —
U01/U06/U02-carriers) rewrites the same `__emit_*` / `serialize_directive_payload`
/ `ComptimeDirective` surface. **E1 must not start until E2 merges** (shared
territory). Surviving JSON transports handed to E1:

- `__emit_extend` — direct-extend authority (E1's JSON-protocol class; ruled OUT
  of E2 scope).
- `__emit_set_param_value`.
- `serialize_directive_payload` core.
- The `statements.rs` :600–740 emit surface.

`__type_probe` also lives in E1/E5 territory (see the E2-D2 correction above).

### Sweep scope boundary (precise)

The E2 migration sweep — the ~89 fixture sites and the 9 `.shape` CLI fixtures —
covered **this repository only** (`crates/`, `tools/`, `bin/`, and the shipped
`.shape` files), enforced by the binding workspace-wide both-spelling grep. It
did **NOT** cover the **`../shape-web`** repo. Book snippets and any prose code
fences under `../shape-web/book/` were **explicitly out of sweep scope** and are
**F1's inventory** (Stage 7). Nothing in this close report should be read as
claiming book-snippet coverage: a book fence that still spells a source-string
`extend`/`replace module` generation call will hit the new `[C0929]` rejection
and is F1's to migrate. E1 and F1 both inherit this boundary.

---

## Baseline evolution

All movements were dispositioned (retirements / rebaselines), not regressions.

| Suite | Start → end | Reason |
|---|---|---|
| st-annotations | 12 → 10 failed names | `d12_*` rebaselined green onto the `[C0929]` rejection; `generated_snippet_…vm_and_jit` retired (residual VM/JIT-identity coverage carried by direct-route siblings). |
| st-comptime | 3 (baseline of record) | 261/3 byte-identical branch-vs-main; **must run `--test-threads=1`** (the 259/5-rotating-names was parallel-state flapping — supervisor lane lesson). |
| vmlib-full | 7 (+ flapper) | 7 pre-existing main names + the `nested_exact_calls_close_outer…` order-sensitive flapper; zero E2 newcomers throughout. |
| lsp-lib | 886 → 882 | C0910 reparse tests retired (5-retire + 2-keep: `presentation.rs:104` kept-renamed as a byte-duplicate of a surviving sibling; `rename_tests.rs:168` unavailable-context test kept as defensive surface). |
| st-lsp | 503 → 502 | generated-captures reparse test retire (`:211`). |

---

## Review record

- **Slice 3** — independent Opus adversarial review: **PASS, zero findings, high
  confidence** (fact-key equality traced to content-derived `ExpansionIdentity`,
  no ordinal; parallel-carrier defection refuted — same body/rewrite/stamp on
  both paths; rollback lever timing non-vacuous).
- **Slice 4.5** — independent Opus panel: **PASS, 0 blocking, 4 non-blocking**
  (all addressed in the `d804cde4` cleanup or recorded here).
- **Slice 5** — independent Opus panel (7 attack surfaces): **PASS, high
  confidence, 0 blocking / 2 LOW** (L1: the traded-away span-drift hint, the
  refuted-in-part disposition; L2: the C0929 uncoded-tag = the shared C092x
  follow-up). Reviewer's key clarifying find: `capture_at` ALREADY skipped
  source-map-less captures at base, so the poison-removal's net effect is one
  fewer informational issue. Totality regex zero-hit; both C0929 pins verified as
  non-vacuous full-program asserts; survivors correctly assigned.

**Mid-program adversarial verdicts (both moved the ledger):**

- The "capturing closures always deopt" prior was **pre-C1** — a supervisor
  4-way differential + re-gate proved the move-capture closure runs
  **ZERO-FALLBACK NATIVE** post-C1 #12; the memory was updated and the test
  upgraded to `assert_c2_fixture_reaches_native_jit`.
- The reachability "C0910 DEAD" verdict was **refuted in part** (finding 7).

---

## Discrepancy note

The roster row records the deletion commit `cba541fb` as "net −488". The tree
shows `cba541fb` at **16 files changed, 164 insertions(+), 492 deletions(-)** =
net −328 (the `comptime_builtins.rs` core alone is +73/−376 = net −303). The
whole capstone landing set (`04904540`…`0de7b244`, 3 commits) is 262/+ 676/− =
net −414. None equals −488; the closest single figure is the **492 gross
deletions** in the deletion commit, which the roster likely rounded/relabeled.
The structural facts all verify (11 symbols deleted with zero live refs; C0929
minted at `comptime_builtins.rs:757`; C0928 at `statements.rs:734`; C0910 →
accumulator-poison → removed in the fix round). This report cites the verified
tree figures; surfaced to the supervisor for the #18 closing comment.

---

## Appendix A — drafted follow-up issue (C092x uncoded comptime diagnostics)

The supervisor files this at close (do not file it from the E2 lane). Suggested
label: `adr-009`. It is the shared C092x follow-up named in finding 6 and in
C2's L2.

**Title:**

> Comptime builtin diagnostics are uncoded `Err(String)` tags — route C0927/C0928/C0929 (and the C092x family) through the coded-diagnostic path

**Body:**

> Follow-up from ADR009-E2 #18 (and C2's L2 note on #13). Every comptime builtin
> error is an `Err(String)`; the diagnostic code is embedded in the message text
> as a `[C09xx]` prefix rather than carried by the coded-diagnostic path the rest
> of the compiler uses. Known instances landed by E2:
>
> - `[C0927]` — `extend_method`: field splice is not a valid identifier
>   (`crates/shape-vm/src/compiler/comptime_builtins.rs:632`).
> - `[C0928]` — expr-form `replace body (expr)` unsupported; use the block form
>   (`crates/shape-vm/src/compiler/statements.rs:734`).
> - `[C0929]` — the source-string `extend`/`replace module` generation route has
>   been removed; pass a typed carrier
>   (`crates/shape-vm/src/compiler/comptime_builtins.rs:757`).
>
> These read correctly to users but are not machine-addressable: they carry no
> structured code, span, or severity through the diagnostic pipeline, so LSP
> surfacing, `--explain`, and any code-based test assertions must string-match the
> tag. The tests currently assert on `err.contains("[C09xx]")`
> (e.g. `comptime_builtins.rs:2135`), which is the symptom.
>
> **Scope:** route the comptime-builtin error surface through the coded-diagnostic
> path so C0927/C0928/C0929 — and the broader C092x comptime-builtin family — are
> emitted as coded diagnostics with spans, not string-tagged `Err(String)`.
> Preserve the exact user-facing message text (the E2 fixtures rebaselined onto
> these strings). This is a diagnostics-plumbing change, not a behavior change.
>
> **Acceptance:** the three codes above emit as structured diagnostics with spans;
> the string-match test assertions are replaced by code assertions; existing E2
> rejection pins stay green.
>
> Related: #18 (E2), #13 L2 (C2).
