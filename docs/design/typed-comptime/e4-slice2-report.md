# E4 #20 — Slice 2 report (the `#74` interim foreign-target comptime rejection)

Charter: **E4-D5** (`e4-decisions.md`), issue **#74**. Slice 2 flips the frozen
C3-S8a scope-fence pin into a LOUD named rejection of **comptime-only**
annotations (`comptime pre` / `comptime post`, no declarative hooks) applied to
foreign targets (`extern "C"`, `fn python`, `fn typescript`), citing #74. The
run-capability exploration remains #74's and is NOT an E4 deliverable.

Gate baseline of record: `e4-slice0-report.md` §1 (FAILED-name sets, never raw
counts), plus the `native_interop` row captured by this slice (§3.0) — the
slice-0 table had none.

## 0. Commit identity

- Branch `adr009/e4`, base `75eca793`, append-only (never amend/rebase/reset).
- **S2** — this stage: producer + call site + 13 pins + the two book edits +
  this report.
- Book repo `shape-web`, branch `adr009-c3-annotations`, base `43173a8`,
  commit `d60bbc0` (two files only; the ~64 uncommitted files belonging to
  another agent were left untouched and unstaged — verified before and after).

## 1. Supervisor rulings on the spec's §7 open questions (ratified 2026-07-22)

All eight were ruled before implementation began; none moves user-ratified
scope. Recorded here as the durable home.

| # | question | RULING |
|---|---|---|
| Q1 | widen the rejection to `on_define` / `metadata` on foreign targets? | **EXCLUDE.** D5's ratified word is "comptime-only". Decisive reason: the definition-time lifecycle call is silently DROPPED under `-m jit`, the CLI default, so a rejection whose remedy read "apply it to an ordinary Shape function instead" would be **false**. Ship the 6 cells; keep `e4s2_twin_metadata_only_annotation_on_extern_c_fn_compiles` as the fence so the exclusion is a TEST. File one issue covering BOTH halves. → **#75** |
| Q2 | reject pure markers (zero handlers)? | **NO.** Not a divergence; would break the shipped `@warmup`. Fence pin `e4s2_twin_marker_annotation_without_handlers_on_extern_c_fn_compiles`. |
| Q3 | one producer, or split polyglot vs native `extern C` per #74's two-fold framing? | **ONE**, descriptor-rendered. Identical measured behaviour across all three flavours; two producers for one behaviour is the parallel-implementation shape CLAUDE.md forbids. #74's two-fold framing is about the eventual RUN CAPABILITY, not this rejection. |
| Q4 | message text | **ADOPT §3.1 VERBATIM.** The three ruled-IN signals (*planned* / *not refused* / *interim*) are load-bearing; `planned, not refused` stays pinned by assertion. |
| Q5 | the stacked-annotation behaviour change | **ACCEPTED.** "First rejection-bearing annotation in application order" is the correct generalization. Pinned, and called out in the commit message. |
| Q6 | rename `mod foreign_target_hook_annotation_tests` | **YES** → `foreign_target_annotation_rejection_tests`. 5 green names re-path; bookkeeping in §3.2. |
| Q7 | file the adjacent-holes ticket | **YES**, one issue, ZERO S2 code. → **#76** |
| Q8 | rename `sugar_lowering.rs` | **NOT IN S2.** |

## 2. What shipped

### 2.1 The producer (one, new, separate)

`crates/shape-vm/src/compiler/statements/annotation_declarations/sugar_lowering.rs`
:: **`foreign_target_comptime_handler_rejection(annotation_name, handler_type, target_descriptor, fn_name) -> String`**,
placed immediately after its #68 sibling `foreign_target_application_rejection`,
which is **byte-for-byte unchanged**.

Rendered sentence (verbatim, CLI-confirmed):

```
annotation `@marked` on extern "C" fn `labs` is not applied — its `comptime post` handler would never run, because foreign function declarations never reach the compile-time annotation-handler pass (running comptime handlers on foreign targets is planned, not refused — see issue #74; this rejection is interim and is deleted when that capability lands); wrap the call in an ordinary Shape function and annotate that, move the compile-time work into a `comptime { }` block, or remove it
```

- The phase word comes from the module-private `hook_kind_word()`, i.e. from the
  real `AnnotationHandlerType` — not a hard-coded string. Pinned by
  `e4s2_comptime_pre_annotation_on_extern_c_fn_names_the_pre_handler`.
- `target_descriptor` renders all three foreign flavours from ONE producer,
  exactly as the #68 sibling does (Q3).
- **No new diagnostic code.** C3-G13 binds: string-tag text now, with the #60
  routing note. `ShapeError::error_code()` narrows only two SemanticError message
  shapes and this sentence matches neither, so it ships as a plain
  `SemanticError`. No `C09xx` minted.
- **Deliberately a second producer, not a parameterization of #68's.** Different
  reason, different issue, **different deletion date**: the #68 producer dies
  when E4 closes #68; this one outlives it, because the #74 run capability is
  explicitly not an E4 deliverable. A shared producer with a `reason`
  discriminant would entangle two independent lifetimes.

### 2.2 The call site

`crates/shape-vm/src/compiler/functions_foreign.rs` ::
`BytecodeCompiler::compile_foreign_function` — **ONE** loop over
`def.annotations` in source order.

Invariants held:

- `let Some((_, compiled)) = self.lookup_compiled_annotation(ann) else { continue };`
  preserved **in shape**. It is the only thing keeping unresolvable names (the
  dark `@remote`, the G12 nested-fn precedent) out of both rejections. Pinned by
  `e4s2_twin_unknown_annotation_name_on_extern_c_fn_keeps_existing_behavior`.
- Gate: `sugar_post_handler.is_some() || comptime_pre_handler.is_some() || comptime_post_handler.is_some()`.
  `on_define_handler` / `metadata_handler` are **NOT** in the gate (Q1).
- `target_descriptor` and the `Span::DUMMY → def.name_span` fallback computed
  **ONCE**, before the reason branch — no duplicated fallback to drift.
- Reason branch: `sugar_post_handler.is_some()` → #68 producer (unchanged text);
  else → #74 producer with
  `comptime_pre_handler.as_ref().or(comptime_post_handler.as_ref())`.
- `return Err(ShapeError::SemanticError { … })` — surface-and-stop at the first
  rejection-bearing annotation in application order.

### 2.3 The BEHAVIOUR CHANGE across stacked annotations (Q5)

The loop generalizes from "first *hook-bearing* annotation" to "first
*rejection-bearing* annotation". For `@marked()` (comptime-only) stacked above
`@traced("second")` (hook-bearing) on an `extern "C" fn`:

- **BEFORE S2:** names `@traced`, anchored at the LATER line — the loop skipped
  the sugar-less `@marked`. A priority inversion: the squiggle jumped past an
  equally-broken earlier annotation.
- **AFTER S2:** names `@marked`, anchored at its own line.

Deliberate, self-healing across the eventual #68 deletion, and no existing test
covered it (the existing stacked pin uses two *hook* annotations, so it stays
green). Pinned by
`e4s2_stacked_comptime_then_hook_rejects_on_the_first_in_application_order` and
called out in the commit message so it is not read as a regression.

### 2.4 Precedence inside ONE annotation

An annotation carrying BOTH a declarative hook and a comptime handler stays on
the **#68** arm. This is not a judgement call — E4-D5 authorizes rejecting
*"comptime-only annotations"*, and such an annotation is not comptime-only.
Self-healing: when E4 closes #68 and the sugar arm is deleted, the comptime arm
takes over automatically and is still correct, because #74 will not have landed.
Pinned by `e4s2_hook_and_comptime_annotation_on_extern_c_fn_reports_the_68_hook_reason`,
which is a deliberate **tripwire** — it goes red at #68 close and forces a
re-decision.

### 2.5 Stale comments rewritten (mandatory, not cosmetic)

Both blocks declared the very scope fence S2 removes; leaving either would have
contradicted the shipped code.

- `compile_foreign_function`'s header comment — rewritten to two reasons, two
  issues, two deletion dates, the ONE-loop rule, the precedence rule, and the
  ordered scope fence.
- `mod foreign_target_annotation_rejection_tests`'s doc-comment (§5.B) —
  likewise, and it records the §1.2 correction **honestly**: `on_define` /
  `metadata` on foreign targets **ARE** genuine foreign divergences (measured
  firing on an ordinary fn under `-m vm`, silent on every foreign flavour). They
  are excluded because D5 does not authorize them and because the family carries
  an orthogonal `-m jit` drop — **not** because "they don't fire on ordinary fns
  either", which was scout A3's refuted claim.

### 2.6 The `#74 INTERIM REJECTION` grep tag

`grep -rn "#74 INTERIM REJECTION"` returns the deletion set for when #74 lands.

**Deviation from spec §5.H, disclosed:** the spec mandated *exactly three* sites
(the producer doc-comment, the `compile_foreign_function` comment header, the
test-module doc-comment). All three are present. The tag was additionally placed
on the three new out-of-crate test cells (`ffi_syntax.rs`, `showcases.rs`,
`foreign_lsp.rs`) and on the section header inside the test module. **Reason:**
the tag's stated purpose is "returns the full deletion set"; those three cells
ARE in the deletion set, so restricting the tag to three sites would have made
the grep return an *incomplete* set and defeated the mechanism. Over-tagging is
strictly better for the tag's own contract; under-tagging is not.

## 3. Gate

Every suite judged by **FAILED-NAME SET** against `e4-slice0-report.md` §1, never
by raw counts.

### 3.0 `native_interop` — the missing baseline, now the row of record

The slice-0 §1 table has no `native_interop` row (grepped: zero hits for
`native_interop` / `ffi_syntax`). Captured at the UNMODIFIED base `75eca793`,
**before any edit**:

> `cargo test -p shape-test --test native_interop -- --test-threads=1`
> → **18 passed / 0 failed / 0 ignored. FAILED set: EMPTY.**

Log: `native_interop-baseline.txt` (scratch). Later slices inherit this row: any
member of its FAILED set is a regression.

### 3.1 Gate table (actual numbers)

| # | invocation | baseline | actual | FAILED-set verdict |
|---|---|---|---|---|
| 1 | `cargo test -p shape-vm --lib foreign_target_annotation_rejection_tests` | n/a (subset) | **15 passed / 0 failed** | PASS — 5 surviving `s8a_*` + 10 `e4s2_*`, all named in the run output. Presence POSITIVELY OBSERVED, not inferred. |
| 2 | `cargo test -p shape-vm --lib` | 3510 / 6 fail / 36 ign | **3518 passed / 7 failed / 36 ign** | PASS — the 6 stable names + the ONE permissible 7th flap member `…nested_exact_calls_close_outer_arguments_before_inner_compilation`. No other name. Arithmetic corroborates: 3516 baseline total + 9 net new = 3525 = 3518 + 7. |
| 3 | `cargo test -p shape-test --test native_interop -- --test-threads=1` | **18 / 0 / 0** (§3.0, captured by this slice) | **19 passed / 0 failed** | PASS — +1 = the new cell G1. FAILED set still empty. |
| 4 | `cargo test -p shape-test --test annotations_comptime -- --test-threads=1` | 116 / 10 fail / 0 ign | **117 passed / 10 failed** | PASS — +1 = the new cell G2. FAILED set byte-identical to the baseline 10 (`executed_extend_authority::*` ×8, `generated_method_runtime::*` ×2). |
| 5 | `cargo test -p shape-test --test annotations_runtime -- --test-threads=1` | 36 / 0 / 0 | **36 passed / 0 failed** | PASS — #68 arm untouched. |
| 6 | `cargo test -p shape-test --test annotation_targets -- --test-threads=1` | 24 / 0 / 0 | **24 passed / 0 failed** | PASS — #68 arm untouched. |
| 7 | `cargo test -p shape-test --test comptime -- --test-threads=1` | 260 / 3 fail / 0 ign | **260 passed / 3 failed** | PASS — same 3 names (`annotations::b6_*` ×2, `callable::hash_tracer_does_not_disturb_formatted_strings`). |
| 8 | `cargo test -p shape-test --test lsp -- --test-threads=1` | 506 / 0 / 0 | **507 passed / 0 failed** | PASS — +1 = the new cell G3. FAILED set still empty. |

`just check-clean` (`cargo check --workspace --all-targets`): **exit 0**. It
executes NO tests, so it is necessary and nowhere near sufficient — the eight
runs above are the evidence. The only warning in the workspace check is a
pre-existing `unused import: super::*` in `compiler/comptime_builtins.rs`,
untouched by S2.

`modules_visibility`, `shape-lsp --lib` and `cli_tests` were skipped per spec
§6.1 — outside the blast radius, untouched by S2.

### 3.2 Module-rename bookkeeping (Q6) — 5 disappear, 5 appear

`mod foreign_target_hook_annotation_tests` → `mod foreign_target_annotation_rejection_tests`
re-paths five names. **All five were green before and are green after**; no
FAILED-set member moved. The 5-disappear / 5-appear diff is a rename, not a
regression:

| disappears (old path) | appears (new path) |
|---|---|
| `…::foreign_target_hook_annotation_tests::s8a_hook_annotation_on_extern_c_fn_rejects_with_the_exact_sentence` | `…::foreign_target_annotation_rejection_tests::s8a_hook_annotation_on_extern_c_fn_rejects_with_the_exact_sentence` |
| `…::foreign_target_hook_annotation_tests::s8a_hook_annotation_on_dynamic_language_foreign_fn_rejects_naming_the_language` | `…::foreign_target_annotation_rejection_tests::s8a_hook_annotation_on_dynamic_language_foreign_fn_rejects_naming_the_language` |
| `…::foreign_target_hook_annotation_tests::s8a_stacked_hook_annotations_on_extern_c_fn_reject_on_the_first` | `…::foreign_target_annotation_rejection_tests::s8a_stacked_hook_annotations_on_extern_c_fn_reject_on_the_first` |
| `…::foreign_target_hook_annotation_tests::s8a_twin_extern_c_fn_without_annotations_compiles` | `…::foreign_target_annotation_rejection_tests::s8a_twin_extern_c_fn_without_annotations_compiles` |
| `…::foreign_target_hook_annotation_tests::s8a_twin_same_hook_annotation_on_ordinary_fn_weaves` | `…::foreign_target_annotation_rejection_tests::s8a_twin_same_hook_annotation_on_ordinary_fn_weaves` |

A sixth name, `s8a_scope_fence_comptime_only_annotation_on_extern_c_fn_stays_unrejected`,
disappears for a different reason: it was **inverted**, not renamed (§4).

### 3.3 Tests added (13)

Ten in `crates/shape-vm/src/compiler/functions_foreign.rs`:

1. `e4s2_comptime_post_annotation_on_extern_c_fn_rejects_citing_74` — the
   INVERTED frozen pin. Fixture verbatim from the C3-S8a scope-fence control;
   asserts the full sentence, `see issue #74`, `planned, not refused`,
   `!contains("#68")`, and the `@marked(` anchor.
2. `e4s2_comptime_pre_annotation_on_extern_c_fn_names_the_pre_handler`
3. `e4s2_both_comptime_phases_on_extern_c_fn_names_the_pre_handler`
4. `e4s2_comptime_annotation_on_dynamic_language_foreign_fn_names_the_language`
5. `e4s2_hook_and_comptime_annotation_on_extern_c_fn_reports_the_68_hook_reason` — the precedence pin / #68-close tripwire
6. `e4s2_stacked_comptime_then_hook_rejects_on_the_first_in_application_order` — pins the §2.3 behaviour change
7. `e4s2_twin_same_comptime_annotation_on_ordinary_fn_still_runs_its_handler` — proves the handler still RUNS (its body calls `error()`), not merely that the file compiles
8. `e4s2_twin_marker_annotation_without_handlers_on_extern_c_fn_compiles` — Q2 fence
9. `e4s2_twin_metadata_only_annotation_on_extern_c_fn_compiles` — Q1 fence, tripwire for #75
10. `e4s2_twin_unknown_annotation_name_on_extern_c_fn_keeps_existing_behavior` — pins the `else { continue }` shape, tripwire for #76

Three outside it:

- `tools/shape-test/tests/native_interop/ffi_syntax.rs` ::
  `comptime_annotation_on_extern_c_fn_is_rejected_citing_74` — end-to-end
  through the shipped pipeline, in the FFI suite.
- `tools/shape-test/tests/annotations_comptime/showcases.rs` ::
  `llm_tool_on_extern_c_fn_is_rejected_citing_74` — the ergonomics cell. This
  exact program compiled, ran, printed, exited 0 and generated **nothing** at
  `75eca793`. Sits beside the `llm_tool_derives_schema_via_stdlib_import_*`
  positives.
- `tools/shape-test/tests/lsp/foreign_lsp.rs` ::
  `comptime_annotation_on_extern_c_fn_surfaces_lsp_diagnostic_at_the_application_line`
  — converts the spec's §5.3 code-read into a measurement. It **passes**: the
  `SemanticError` does reach the editor via `analysis.rs` → `compile_in_place`
  (`RecoverAll`) → `error_to_diagnostic_with_uri`, anchored at the
  `@application` line. No `shape-lsp` code change was required.

Deleted: **none**. The five §5.E must-keep tests are byte-for-byte unchanged
(they assert the #68 sentence; an edit would have meant the #68 producer moved,
which S2 must not do).

**Spec §5.D6 note, resolved:** the existing `compile_err_with_location` handles a
fired comptime `error()` without panicking — a fired comptime error IS a
`ShapeError::SemanticError`. No variant-agnostic helper was needed. This
converts a "medium confidence, read + CLI-rendered" claim into an executed one.

### 3.4 CLI confirmation (end-to-end, outside the test harness)

The `75eca793 + S2` debug binary on the defect fixture:

```
error[RUNTIME]: Bytecode compilation failed: Semantic error: annotation `@marked` on extern "C" fn `labs` is not applied — its `comptime post` handler would never run, … see issue #74 …
  --> <input>:7:1
     |
 7 | @marked()
     | ^~~~~~~~~
```

Anchored at the `@application` line, sentence verbatim.

## 4. Book debt (shipped in-slice, per the every-feature-in-the-book HARD GATE)

Repo `shape-web`, branch `adr009-c3-annotations`, commit `d60bbc0`, **two files
only**.

- `book/book-site/src/content/docs/advanced/annotations.mdx` (§The Rejection
  Matrix) — the foreign-function paragraph declared ONE reason; it now declares
  TWO, each naming its own issue and deletion date. Adds a gate-runnable
  ` ```shape expected-fail="see issue #74" ` fence beside the existing #68 fence,
  records that #74 is *planned, not refused* and the rejection *interim*, and
  states the accepted case (pure markers still compile on foreign targets). The
  spec flagged that the section's lead ("Hook annotations on foreign functions")
  was now wrong — it is corrected; note it is a paragraph lead, not an `##`
  heading (the enclosing heading is `## The Rejection Matrix`, unchanged).
- `book/book-site/src/content/docs/tooling/polyglot.mdx` (§Annotations) — the
  prose "Polyglot functions support the same annotations as regular Shape
  functions" was **already false at HEAD** (#68 has rejected hook annotations on
  foreign targets since C3-S8a) and doubly false after S2. Replaced with the real
  rule + a link. The fence below it carries no annotation, so nothing broke.

**Repo-hazard discipline observed.** `shape-web` carried ~64 files of another
agent's uncommitted work. Staged by exact path only; never `git add -A/.`, never
`git stash`, never `git checkout -- .`. Dirty-file count verified 64 → 66 → 64
across the commit; `git show --stat` confirms exactly two files.

### 4.1 Book truth-gate — re-run, NOT read from the stale on-disk report

`SHAPE_BIN=<S2 release binary> node scripts/run-book-truth-gate.mjs`, snippets
re-extracted first. Run per slice (see §6 for why).

| slice | total | pass | red |
|---|---|---|---|
| A | 225 | 223 | 2 |
| B | 245 | 243 | 2 |
| C | 24 | 24 | 0 |
| D | 47 | 40 | 7 |
| E | 32 | 27 | 5 |
| **all** | **573** | **557** | **16** |

**Standing of record was 556/572, 16 red. Result: 557/573, the SAME 16 red.**
Exactly the predicted movement — the total rose by the one new `expected-fail`
fence and it passes.

The 16 reds, all pre-existing, all `both-fail`, none in a page S2 touched:
`fundamentals/modules.mdx` ×2; `stdlib/core/remote.mdx` ×2;
`advanced/comptime-annotations-cookbook.mdx` ×6; `advanced/comptime.mdx` ×1;
`advanced/content-addressed-bytecode.mdx` ×2; `advanced/polyglot-distributed.mdx` ×2;
`tooling/execution-server.mdx` ×1.

Positive checks that the real signal was looked for and is absent:

- `expected-fail-succeeded: 0` and `expected-fail-missing: 0` in **every** slice.
  The existing `advanced/annotations.mdx` #68 fence did not move class.
- All **17** `advanced/annotations.mdx` snippets are `pass`, including the
  pre-existing #68 fence and the new #74 fence.
- All 4 gated `tooling/polyglot.mdx` snippets are `pass`.

## 5. Tickets filed

- **#75** — *Annotation lifecycle handlers (`on_define` / `metadata`) are silent
  no-ops — on foreign targets always, and on ordinary fns under `-m jit`.* Both
  halves in one issue per Q1, with the `o1`–`o4` / `m1` / `m2` reproduction
  matrix, the fix ORDER argument (Half B first, or the remedy sentence is a
  lie), symbol anchors, and
  `e4s2_twin_metadata_only_annotation_on_extern_c_fn_compiles` as the acceptance
  tripwire.
- **#76** — *Foreign-fn annotation path skips target/duplicate validation.* One
  shared root (`compile_foreign_function` calls neither
  `validate_annotation_target_usage` nor `check_duplicate_annotations`), zero S2
  code. Records that the unknown-name case is protected today by the named G12
  nested-fn precedent and so needs a RULING, not a patch; and carries the sharp
  correction that `on`-clause validation does **not** gate comptime handlers.
  Tripwire: `e4s2_twin_unknown_annotation_name_on_extern_c_fn_keeps_existing_behavior`.
- **#74** — commented: the interim rejection shipped, the `#74 INTERIM REJECTION`
  grep tag, and the two tests #74's implementer must flip.
- **#68** — no change. Its sentence and its five pins are untouched.

## 6. Deviations from the spec (disclosed, with reasons)

1. **Grep tag on more than three sites** (§2.6). Reason: the tag's own contract
   is "returns the full deletion set"; the three new test cells are in that set.
   All three mandated sites are present.
2. **Book gate run per slice, with a RELEASE binary** rather than one whole-corpus
   debug run. Reason: a debug-binary whole-corpus run measured ~10 snippets/min
   and would need ~57 minutes; this environment kills any single call at 10
   minutes and backgrounding a build is fatal to a workflow subagent. The gate
   script exposes `--slice` but no offset, and slices A (225) and B (245) each
   exceed the 10-minute wall in debug. A release binary is also strictly SAFER
   for this gate: the only debug/release-sensitive category is `runtime-timeout`,
   and a slow binary manufactures false reds. Result: `runtime-timeout: 0` in
   every slice, and the pass/red split matched the standing record exactly. The
   union of the five slices is the full 573-snippet universe — no snippet was
   skipped.

## 7. Residuals — honest

Carried forward, none introduced-and-hidden:

1. **`completion/annotations.rs`** — application-site `@`-completion does no
   target-kind filtering (already a NAMED E4 follow-up from S1c `c65c1d8e`).
   After S2 the editor still suggests comptime-only annotations at foreign-fn
   sites that the compiler now rejects. This is a **new, small LSP/compiler
   inconsistency created by S2**. Fixing it is the existing follow-up's job, not
   a new ticket.
2. **Annotated foreign fn inside an IMPORTED module** is broken at HEAD,
   annotation or not: `Internal error: foreign function '<mod>::<name>' not
   registered`, reproduced with the annotation deleted. This route is
   BLOCKED/masked-unknown, **not measured**, and S2 **cannot** regression-pin it.
   If someone later fixes registration, S2's behaviour on that route is unproven.
3. **`ExportItem::ForeignFunction`** (`statements.rs`) is not reachable from
   source at HEAD (`export fn …` → `Undefined variable: 'export'`). Source
   reading says it funnels to the same `compile_foreign_function`; **inferred,
   not measured**.
4. **`shape serve` / `wire-serve` / snapshot `--resume` / `shape tui`** were not
   exercised. Structurally covered by the two-call-site argument (both callers of
   `compile_foreign_function` are in `statements.rs`), but unmeasured.
5. **`stdlib/core/remote.mdx`** still claims "`@remote` also works on foreign
   function definitions", backed by a `runnable=false` fence. `@remote` is dark
   (`Unknown annotation '@remote'` at HEAD), so the claim is forward-facing into
   #68/#74 territory and is **E4 S5's** deliverable. Flagged, not fixed here.
6. **`metadata` × python/typescript and `on_define` × typescript** are
   single-source cells (one scout, executed, unreplicated). Nothing S2 ships
   depends on them; if #75's fix widens the rejection, re-measure those three
   first.
7. **The `-m jit` lifecycle drop** is a second silent-no-op family in the same
   neighbourhood. It masked the truth from two of three scouts. Anyone reading
   `on_define` behaviour in future must control for execution mode. Now #75
   Half B.
8. **The vmlib flap** showed its permissible 7th member on this run
   (`nested_exact_calls_close_outer_arguments_before_inner_compilation`). Within
   the baseline's binding flap rule; recorded so the next slice does not read it
   as new.
