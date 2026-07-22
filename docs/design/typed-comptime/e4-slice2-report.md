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
- **S2b** — review close-out (§S2b), append-only on top of the panel-examined
  pair. `28fb8b34` and `d60bbc0` are NOT amended, rebased or rewritten.
  - `shape` `adr009/e4` :: **`9d50714f`** — the MAJOR-1 remedy rewording, the
    strengthened + 2 new pins, and the §2.1 / §2.6 / §3.1 report corrections.
  - `shape-web` `adr009-c3-annotations` :: **`fef948e`** — the MAJOR-2 book
    prose fix, two files only, dirty count verified 64 → 66 → 64.
  - This §0 line is itself appended by a follow-up commit, since a commit
    cannot contain its own hash.

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

> **SUPERSEDED in S2b — the block below is what S2 shipped at `28fb8b34`, not
> what ships now.** Its remedy was FALSE for `on type` annotations; see §S2b.1
> for the corrected sentence and the executed proof.

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

**Deviation from spec §5.H — disclosed by the implementer, RATIFIED by the
supervisor (S2b).** The spec mandated *exactly three* sites (the producer
doc-comment, the `compile_foreign_function` comment header, the test-module
doc-comment). All three are present. The tag was additionally placed on the
three new out-of-crate test cells (`ffi_syntax.rs`, `showcases.rs`,
`foreign_lsp.rs`) and on the section header inside the test module. **Reason:**
the tag's stated purpose is "returns the full deletion set"; those three cells
ARE in the deletion set, so restricting the tag to three sites would have made
the grep return an *incomplete* set and defeated the mechanism. Over-tagging is
strictly better for the tag's own contract; under-tagging is not. **Supervisor
ruling (S2b): the deviation is ratified** — the tag's contract is "returns the
full deletion set", and the three end-to-end cells plus the test-module header
genuinely belong to it, so three would have returned an incomplete set. This is
a ratified deviation, not an absorbed one.

**The tag sits at SEVEN CODE sites** (eight grep lines in `.rs` files; the
producer's doc-comment accounts for two of them — the tag itself plus the
self-referential `grep -rn` instruction). **Restrict the grep to code**, e.g.
`git grep -n "#74 INTERIM REJECTION" -- '*.rs'`: an unrestricted repo-wide
`git grep` returns 13 lines, because this section and `e4-decisions.md` cite the
tag in prose and so pollute their own grep. A future deleter running the
unrestricted form gets five non-site prose hits. Full enumeration by symbol,
since the #74 issue comment originally listed only six:

| # | File | Symbol / anchor |
|---|------|-----------------|
| 1 | `crates/shape-vm/src/compiler/statements/annotation_declarations/sugar_lowering.rs` | `foreign_target_comptime_handler_rejection` — producer doc-comment (2 grep lines, 1 site) |
| 2 | `crates/shape-vm/src/compiler/functions_foreign.rs` | `BytecodeCompiler::compile_foreign_function` — call-site comment, arm (b) |
| 3 | `crates/shape-vm/src/compiler/functions_foreign.rs` | `mod foreign_target_annotation_rejection_tests` — module `//!` doc-comment, paragraph (b) |
| 4 | `crates/shape-vm/src/compiler/functions_foreign.rs` | `foreign_target_annotation_rejection_tests` — the `(b)` **section-divider comment**  ← *the one the #74 comment omitted* |
| 5 | `tools/shape-test/tests/native_interop/ffi_syntax.rs` | `comptime_annotation_on_extern_c_fn_is_rejected_citing_74` |
| 6 | `tools/shape-test/tests/annotations_comptime/showcases.rs` | `llm_tool_on_extern_c_fn_is_rejected_citing_74` |
| 7 | `tools/shape-test/tests/lsp/foreign_lsp.rs` | `comptime_annotation_on_extern_c_fn_surfaces_lsp_diagnostic_at_the_application_line` |

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
runs above are the evidence.

**Warning count — corrected in S2b (this sentence was false as first written).**
The original text claimed "the only warning in the workspace check is a
pre-existing `unused import: super::*`". That is wrong. The `check-clean` log
records **22 warning emissions / 18 distinct printed warnings**, across three
targets: **shape-vm (lib) 16**, **shape-vm (lib test) 5** (4 of them duplicates
of lib warnings, so 1 new), **shape-test (test "iterators") 1**. Substance is
unaffected — **zero of them land in an S2- or S2b-authored file**, verified by
resolving every warning to its `-->` location:

```
crates/shape-vm/src/compiler/comptime_fragments/checked_body.rs      (6)
crates/shape-vm/src/compiler/comptime_fragments/mod.rs               (5)
crates/shape-vm/src/compiler/comptime_fragments/checked_template.rs  (2)
crates/shape-vm/src/compiler/comptime_builtins.rs                    (1)
crates/shape-vm/src/compiler/helpers.rs                              (1)
crates/shape-vm/src/compiler/mod.rs                                  (1)
crates/shape-vm/src/compiler/template_specialization/install_registry.rs (1)
tools/shape-test/tests/iterators/stress_chaining.rs                  (1)
```

Neither `functions_foreign.rs` nor `sugar_lowering.rs` nor any of the three
shape-test cells appears. **No later slice may adopt "one warning" as a
cleanliness baseline** — the real pre-existing figure is the one above.

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

---

## S2b. Review close-out — six lens findings closed

A three-lens panel reviewed S2 (`28fb8b34` + `shape-web` `d60bbc0`) and returned
PASS_WITH_FINDINGS, zero blockers, **two MAJOR** and four MINOR. All six are
closed here. **Append-only**: `28fb8b34` and `d60bbc0` were examined by the
panel and are NOT amended, rebased or rewritten — S2b is a new commit on top in
each repo.

### S2b.1 MAJOR-1 — the diagnostic's remedy was FALSE for `on type` annotations

**The defect.** The S2 sentence's first and most prominent remedy read *"wrap
the call in an ordinary Shape function and annotate that"*. Stated flat, it is
false for every annotation declared `on type` — and **two SHIPPED stdlib
annotations are exactly that**:

- `@json_schema` — `crates/shape-runtime/stdlib-src/serde/derive.shape:33`
  (`pub annotation json_schema() on type`)
- `@to_json` — `crates/shape-runtime/stdlib-src/serde/serialize.shape:25`
  (`pub annotation to_json() on type`)

Both now fire the #74 rejection on a foreign fn (pre-S2 they were silently
ignored — that is the whole point of S2). Reproduced against the **`28fb8b34`
debug `shape`**, following the shipped remedy literally:

```
$ shape run f1_jsonschema_foreign.shape          # @json_schema() on extern "C" fn labs
error[RUNTIME]: … annotation `@json_schema` on extern "C" fn `labs` is not applied — …
… ); wrap the call in an ordinary Shape function and annotate that, …

$ shape run f2_jsonschema_wrapper.shape          # DOING WHAT THE REMEDY SAYS
error[RUNTIME]: Bytecode compilation failed: Semantic error:
  Annotation 'json_schema' cannot be applied to a function. Allowed targets: type
  --> <input>:5:1
   5 | @json_schema()
```

`@to_json` behaves identically. Control, confirming the remedy was true for the
`on function` case only: `@llm_tool` (`stdlib-src/llm/tools.shape:29`,
`on function`) on a wrapper fn compiles and emits `myabs_tool_def()`.

This is the *exact* disqualifying criterion the supervisor used in ruling Q1 to
EXCLUDE `on_define` / `metadata` from S2's scope — "a rejection whose remedy
reads 'apply it to an ordinary Shape function instead' would be false". The
shipped sentence committed the very fault that ruling forbade.

**The fix — remedy clause only. Zero compiler logic changed.** No new
target-kind validation, no branching on `allowed_targets`; the producer
`foreign_target_comptime_handler_rejection` still takes the same four arguments
and the same call site selects it. The remedy now routes through the
annotation's own `on` clause, and the wrapper advice is *visibly conditional* on
that clause instead of stated flat. Head clause, reason clause, the three ruled-IN
signals (**planned**, **not refused**, **interim**) and the `see issue #74`
citation are all preserved verbatim; house style (one sentence, em-dash reason
clause, semicolon before the remedy, no terminal period) is preserved; the #68
sibling `foreign_target_application_rejection` is **byte-for-byte unchanged**.

**The sentence that ships now** (CLI-rendered, `@marked` fixture):

```
annotation `@marked` on extern "C" fn `labs` is not applied — its `comptime post` handler would never run, because foreign function declarations never reach the compile-time annotation-handler pass (running comptime handlers on foreign targets is planned, not refused — see issue #74; this rejection is interim and is deleted when that capability lands); apply @marked to an ordinary Shape declaration its own `on` clause allows (an `on function` annotation goes on a Shape fn wrapping the call; an `on type` annotation goes on a type), move the compile-time work into a `comptime { }` block, or remove it
```

**MANDATORY VERIFICATION — the new remedy is EXECUTED, not merely read.** The
original shipped unexecuted; that is how the fault got in. Against the S2b debug
`shape` (`target/debug/shape`, rebuilt after the edit), for **both** `on`-clause
arms, using the SHIPPED stdlib annotations rather than toy fixtures:

| # | fixture | annotation (`on` clause) | step | result |
|---|---|---|---|---|
| 1 | `f3_llmtool_foreign.shape` | `@llm_tool` (**on function**) | rejection fires | `… apply @llm_tool to an ordinary Shape declaration its own `on` clause allows …` |
| 2 | `f4_llmtool_wrapper.shape` | `@llm_tool` | **follow remedy**: Shape fn wrapping the call | **green** — prints `{"name": "myabs", "description": "absolute value", "parameters": {"type": "object", "properties": {"x": {"type": "integer"}}, "required": ["x"]}}` |
| 3 | `f1_jsonschema_foreign.shape` | `@json_schema` (**on type**) | rejection fires | `… apply @json_schema to an ordinary Shape declaration its own `on` clause allows …` |
| 4 | `f6_jsonschema_ontype.shape` | `@json_schema` | **follow remedy**: annotate a type | **green** — prints `{"type": "object", "title": "AbsRequest", "properties": {"x": {"type": "integer"}}, "required": ["x"]}` |
| 5 | `f5_tojson_foreign.shape` | `@to_json` (**on type**) | rejection fires | `… apply @to_json to an ordinary Shape declaration its own `on` clause allows …` |
| 6 | `f7_tojson_ontype.shape` | `@to_json` | **follow remedy**: annotate a type | **green** — prints `{ "x": 7 }` |

Exact commands (all `shape run <fixture>`, debug binary at
`/home/dev/dev/shape-lang/shape-adr009-a3/target/debug/shape`; fixtures in the
S2b scratch dir). **The remedy is true for both arms.** Rows 2/4/6 are the
literal execution of what the sentence tells the reader to do.

**Pins updated — strengthened, not weakened.** The full-sentence assertion in
`e4s2_comptime_post_annotation_on_extern_c_fn_rejects_citing_74` still asserts
the **whole sentence verbatim** (it was deliberately not shortened to a laxer
substring to reduce churn), and three assertions were **added** to it:

- negative: the flat `"wrap the call in an ordinary Shape function and annotate
  that"` must NOT come back — a direct regression tripwire on MAJOR-1;
- positive: the remedy contains `"its own `on` clause allows"`;
- positive: **both** arms are spelled out.

Two new pins, same module:

- `e4s2b_on_type_comptime_annotation_on_extern_c_fn_gets_a_true_remedy` — an
  `on type` annotation on an `extern "C" fn` gets the type arm, not the flat
  wrapper lie.
- `e4s2b_on_type_comptime_annotation_remedy_compiles` — the **green twin**:
  following that remedy compiles. This is the whole content of the fix, so it is
  pinned in-repo rather than left to the CLI probes above.

The producer's doc comment carries a `REMEDY WORDING IS LOAD-BEARING — do not
flatten it back` block recording the measurement, so a later slice cannot
"simplify" it back without reading why.

**#76 is NOT touched.** The precedence question — `compile_foreign_function`
runs the rejection loop *before* `validate_annotation_target_usage`, so the #74
sentence wins over the sharper target-kind mismatch — is recorded in the
producer's doc comment and in the #74 issue comment as #76's sharpest
motivation. Widening S2b into #76 was explicitly out of bounds. The new wording
is true either way, which is why the precedence bug does not block it.

### S2b.2 MAJOR-2 — the book prose S2 added was FALSE, and regressed a true sentence

**The defect.** Both S2-swept pages asserted a universal dichotomy that is false:

- `advanced/annotations.mdx` — "Annotations that carry **handlers** are rejected
  on **foreign functions**" + "Annotations with **no** handlers (pure markers)
  are accepted".
- `tooling/polyglot.mdx` — "Annotations carrying a **handler** are rejected on
  foreign targets".

`on_define` and `metadata` **are handlers** — in the compiler's own vocabulary
(`CompiledAnnotation::on_define_handler` / `::metadata_handler`,
`crates/shape-vm/src/bytecode/core_types.rs`) and in the book's own
(`advanced/comptime.mdx`, `advanced/comptime-annotations-cookbook.mdx` both call
them lifecycle hooks). They are **accepted and silently no-op** on foreign
targets. Re-verified for S2b against the S2b debug `shape` under `-m vm`:

```
$ shape run -m vm g1_ondefine_foreign.shape     # @marked{on_define} on extern "C" fn
compiled green                                   # ← on_define NEVER fired

$ shape run -m vm g2_ondefine_ordinary.shape    # same annotation, ordinary fn
on_define fired
42

$ shape run -m vm g3_metadata_foreign.shape     # @marked{metadata} on extern "C" fn
compiled green
```

Pinned in-repo by the passing
`e4s2_twin_metadata_only_annotation_on_extern_c_fn_compiles`.

Two aggravating factors, both recorded rather than smoothed over:

1. **S2 regressed a true sentence.** The pre-S2 text ("Hook annotations on
   foreign functions … are rejected") was correctly scoped and TRUE. S2 replaced
   it with a false universal — a net loss of truth on a page a reader trusts.
2. **The book truth-gate structurally cannot catch this.** An over-broad prose
   claim with no counterexample fence passes trivially: there is nothing to
   execute. Only a reader gets hurt. The gate is not a prose oracle and must not
   be treated as one.

**The fix, in both pages** (`shape-web`, book prose only):

1. **Scoped precisely** to what actually rejects: declarative `before` / `after`
   hooks (#68) and `comptime pre` / `comptime post` (#74). The universal
   "handlers" claim is gone from both pages; `annotations.mdx` now opens "Two
   kinds of handler are rejected on foreign functions … Other handler kinds are
   *not* rejected; see the gap noted below".
2. **The gap is NAMED, not quietly narrowed.** `annotations.mdx` carries a
   `:::caution[Known gap #75: on_define and metadata on foreign functions]`
   aside stating plainly that such an annotation is *accepted on a foreign
   function and then silently does nothing*, that the same declaration on an
   ordinary Shape fn runs its `on_define` normally, why it was left out of #74's
   rejection (the orthogonal JIT-drop half), and that **you get no diagnostic
   either way**. `polyglot.mdx` carries the same statement in short form with the
   #75 citation. Per the standing ruling that a silent no-op is the worst state:
   we are shipping one behind a ratified fence, so the book at minimum makes it
   **discoverable**. Hidden trap → documented knowledge.
3. **Neighbouring sentences re-checked.** The `annotations.mdx` comptime
   paragraph restated the same false remedy MAJOR-1 fixed in the compiler
   ("wrap the call in an ordinary Shape function and annotate that") — corrected
   in lockstep, and it now names `@json_schema` / `@to_json` as the concrete
   `on type` counterexample. The "pure markers" sentence is kept but rewritten so
   it no longer implies a dichotomy ("That is not a gap: a marker does nothing on
   any target"). A book-wide grep for the same over-claim shape
   (`handler` × `foreign|extern|polyglot`) returns only these two pages;
   frontmatter `llm_summary` / key-facts on both pages carry no such claim.

**Repo-hazard discipline observed.** `shape-web` carries 64 uncommitted files of
another agent's work. Staged by **exact path only** — never `git add -A`, never
`git add .`, never `git stash`, never `git checkout -- .`. Dirty-file count
verified 64 → 66 → 64 across the commit.

### S2b.3 MINOR-3 — the #74 issue comment under-enumerated the deletion set

The `#74 INTERIM REJECTION` grep tag sits at **7** sites; the #74 issue comment
described only **6**, omitting the `(b)` section-divider comment inside
`foreign_target_annotation_rejection_tests`. Closed by a new **S2b correction
comment** on #74 enumerating all seven by symbol (table reproduced in §2.6
above), plus superseded-markers on the two stale blocks of the original comment
so no reader copies them.

**The 7-vs-3 deviation from spec §5.H is RATIFIED, not absorbed** (supervisor,
S2b) — recorded as such in §2.6. The tag's contract is "returns the full
deletion set"; the three end-to-end cells plus the test-module header genuinely
belong to it, so the spec's "exactly 3" would have returned an *incomplete* set.

### S2b.4 MINOR-4 — the slice report overstated workspace-check cleanliness

§3.1 claimed "the only warning in the workspace check is a pre-existing
`unused import: super::*`". False. Corrected in place: **22 warning emissions /
18 distinct printed warnings** (shape-vm lib 16, shape-vm lib-test 5, shape-test
"iterators" 1), with every warning resolved to its file so the real claim —
**zero in an S2- or S2b-authored file** — is checkable rather than asserted. §3.1
now states explicitly that no later slice may adopt "one warning" as a
cleanliness baseline.

### S2b.5 MINOR-5 — `#[serde(skip)]` undermines the LOUD guarantee → **#77**

Filed as **#77**. `CompiledAnnotation`'s `comptime_pre_handler`,
`comptime_post_handler` and `sugar_post_handler` are all `#[serde(skip, default)]`
(`crates/shape-vm/src/bytecode/core_types.rs`), so a **deserialized**
`CompiledAnnotation` presents `None` for all three, the gate in
`BytecodeCompiler::compile_foreign_function` `continue`s, and **both** the #68
and #74 rejections silently vanish — the exact silent-no-op class S2 exists to
eliminate, restored by a serialization round-trip. (`allowed_targets` is
serde-skipped too, so the `on`-clause check has the same exposure; that half
overlaps #76.)

**Not an S2 regression** — the pre-existing #68 arm has identical exposure — and
**no reproducer exists**: the common in-process path populates the handlers
correctly (verified with `@llm_tool` via `from std::llm::tools use { @llm_tool }`
on an `extern "C" fn`, which rejects as designed). Filed anyway, honestly
labelled latent, because a LOUD guarantee with a serialization-shaped hole is
worth a named ticket. Anchored on the struct + the three field names + the gate
symbol; names content-addressed bytecode, snapshot/resume and the wire protocol
as the paths most likely to expose it; cross-referenced to **#71** (same
`#[serde(skip)]` family, different consequence).

### S2b.6 MINOR-6 — book-gate harness reproducibility caveat (recorded, not fixed)

> **The 557/573 figure is NOT reproducible from committed state.** It was
> measured against a book-gate harness that is **uncommitted**. Both
> `book/book-site/scripts/run-book-truth-gate.mjs` (**+451 lines** vs committed
> HEAD) and `book/book-site/scripts/extract-shape-snippets.mjs` (**+65 lines**)
> are part of another agent's 64-file working set — 499 insertions across those
> two files alone. The gate's **grading classes** AND the **snippet extractor
> that sets the 573 denominator** are both modified relative to committed HEAD.
> A future slice checking out `adr009-c3-annotations` at `d60bbc0` and running
> the gate will NOT get 573 snippets and may not get 16 reds. **Do not treat
> 557/573 as a committed baseline.** It is a measurement against a working tree
> that no commit describes.

Deliberately **not fixed and not committed** — that harness is not S2/S2b's work
and staging it would violate the repo-hazard discipline. Recorded only.

Mitigation already on record: both `expected-fail` fences on
`advanced/annotations.mdx` (the #68 one and the new #74 one) were verified to
grade correctly and **binary-independently** across debug/release × vm/jit, so
the specific claim S2 shipped does not rest on the harness's modified grading
classes.

### S2b.7 Re-gate (targeted — MAJOR-1 changes a pinned string)

Judged by **FAILED-NAME SET**, never counts. Baselines: `e4-slice0-report.md` §1
plus the S2-established `native_interop` row (18/0/0 at base, 19/0 after S2).

| # | suite | S2 result | S2b result | verdict |
|---|---|---|---|---|
| 1 | `cargo test -p shape-vm --lib foreign_target_annotation_rejection_tests` | 15 / 0 | **17 passed / 0 failed** | PASS — 15 + the 2 new `e4s2b_*` pins, all named in the run output |
| 2 | `cargo test -p shape-vm --lib` | 3518 / 7 fail / 36 ign | **3520 passed / 7 failed / 36 ign** | PASS — FAILED set byte-identical to S2: the 6 stable names + the ONE permitted flap member `…nested_exact_calls_close_outer_arguments_before_inner_compilation`. No other name. +2 passed = the 2 new pins |
| 3 | `cargo test -p shape-test --test native_interop -- --test-threads=1` | 19 / 0 | **19 passed / 0 failed** | PASS — FAILED set still empty |
| 4 | `cargo test -p shape-test --test annotations_comptime -- --test-threads=1` | 117 / 10 fail | **117 passed / 10 failed** | PASS — same 10 names (`executed_extend_authority::*` ×8, `generated_method_runtime::*` ×2) |
| 5 | `cargo test -p shape-test --test lsp -- --test-threads=1` | 507 / 0 | **507 passed / 0 failed** | PASS — FAILED set still empty |
| 6 | `just check-clean` | exit 0 | **exit 0** | PASS (warning inventory corrected — §S2b.4) |
| 7 | book truth-gate, re-run | 557 / 573, 16 red | **557 / 573, 16 red — SAME LIST** | PASS |

The 6 stable vmlib FAILED names, for the record: `test_async_let_binding_is_immutable`,
`test_match_arm_empty_array_unprovable_element_is_clean_compile_error`,
`inlined_closure_keeps_outer_authored_type_ref_in_its_parameter_scope`,
`unavailable_and_missing_callsite_evidence_execute_only_in_legacy_domain`,
`ws6b_inferred_result_variable_arg`, `ws6_generic_id_ok_arg`.

**Book gate — the LIST was verified, not just the count.** A swap holding the
count at 16 would be invisible to a count check. Snippets re-extracted first
(`node scripts/extract-shape-snippets.mjs` → `runnable=true: 573`), then
`SHAPE_BIN=<S2b release binary> node scripts/run-book-truth-gate.mjs`, one
whole-corpus run (3m20s on release — the per-slice split S2 needed was a
debug-binary artefact). The 16 reds by page, identical to the S2 record:

| page | reds |
|---|---|
| `advanced/comptime-annotations-cookbook.mdx` | 6 |
| `advanced/comptime.mdx` | 1 |
| `advanced/content-addressed-bytecode.mdx` | 2 |
| `advanced/polyglot-distributed.mdx` | 2 |
| `fundamentals/modules.mdx` | 2 |
| `stdlib/core/remote.mdx` | 2 |
| `tooling/execution-server.mdx` | 1 |
| **total** | **16** |

All 16 are `both-fail`; `vm-only-fail`, `jit-only-fail`, `output-divergence`,
`expected-divergence`, `expected-fail-succeeded`, `expected-fail-missing` and
`runtime-timeout` are all **0**. Positive checks: all **17**
`advanced/annotations.mdx` snippets pass (including the #68 fence and the #74
fence — the #74 fence's `expected-fail="see issue #74"` substring survives the
MAJOR-1 rewording, which is exactly why the rewording preserved the citation
verbatim), and all **4** `tooling/polyglot.mdx` snippets pass. The new `:::caution`
aside adds no fence, so the denominator stays 573.

**Suites deliberately SKIPPED, with reason.** S2 gate suites 5–7
(`annotations_runtime` 36/0, `annotation_targets` 24/0, `comptime` 260/3) were
not re-run. Reason: S2b changes exactly one string literal in
`foreign_target_comptime_handler_rejection` plus test code and doc comments. That
producer is reachable only from `compile_foreign_function`, which none of those
three suites exercises (`annotations_runtime` and `annotation_targets` cover the
#68 arm and ordinary-target validation; `comptime` covers comptime execution on
ordinary fns). No assertion in any of them quotes the #74 sentence — verified by
grep for `see issue #74` / `planned, not refused` across `tools/shape-test/`,
which returns only the three end-to-end cells in suites 3, 4 and 5, all of which
WERE re-run and all of which pin the `"see issue #74"` substring that the
rewording preserves verbatim. Re-running them could not have discriminated.

### S2b.8 Residuals

- **#76 (precedence) is open and untouched by design.** An `on type` annotation
  on a foreign fn still reports the #74 sentence rather than the sharper
  target-kind mismatch. The wording now handles that case truthfully, which is
  the mitigation, not the fix.
- **#77 has no reproducer.** Filed latent. If a deserialized-annotation path is
  ever opened, that ticket is the tripwire.
- **The book gate remains prose-blind.** MAJOR-2 was invisible to it and a future
  over-claim would be too. Nothing in S2b changes that; it is a standing limit of
  the gate, recorded here so no one mistakes a green gate for a true book.
- **The 557/573 baseline is working-tree-dependent** (§S2b.6).
