# E4 #20 — Slice 1 report (the `on`-clause header syntax)

Charter: **E4-D4** (`e4-decisions.md`), issue **#73** + its 2026-07-22
supervisor correction. Slice 1 replaces the annotation body `targets: [...]`
field with a header `on <kind>(, <kind>)*` clause as the **one** accepted
target-restriction spelling, with all seven `AnnotationTargetKind` kinds
header-eligible and G8/G12 target validation firing unchanged.

Gate baseline of record: `e4-slice0-report.md` §1 (FAILED-name sets, not raw
counts). This report is the S1 close hand-off; it does **not** add a
design-index CURRENT row and does **not** touch the book — those are CLOSE
tasks.

## 0. Commit identity

- Branch `adr009/e4`, base `90765ded`, append-only (never amend/rebase/reset).
- **S1a** `1e86713f` — grammar + parser population (intra-slice dual-PARSE
  window, DN3).
- **S1b** `cb735069` — 134-file / 530-transform corpus sweep + body-field
  removal + NAMED tombstone rejection.
- **S1c** (this stage) — LSP re-sync to the header spelling + this report +
  a fixup of the S1b named-negative rejection pin (see §5.1).

## 1. Resolved grammar / parser / AST changes (anchors)

### 1.1 Grammar (`crates/shape-ast/src/shape.pest`)

- `annotation_def` (`:379-381`): the optional `annotation_on_clause?` sits
  between the params group and the body brace —
  `"annotation" ~ ident ~ ("(" ~ annotation_def_params? ~ ")")? ~ annotation_on_clause? ~ "{" ~ annotation_body ~ "}"`.
- `annotation_on_clause` (`:396-398`):
  `"on" ~ annotation_target_kind ~ ("," ~ annotation_target_kind)*` — **reuses
  `annotation_target_kind` verbatim**, no forked vocabulary. `on` is
  contextual (already spelled in `on_clause` / `timeframe_expr` /
  `join_query_clause`); after the name/params group the only continuations are
  `on` or `{`, so no collision.
- `annotation_target_kind` (`:431-439`): all **seven** kinds —
  `function | type | module | expression | block | await_expr | binding`.
- `annotation_legacy_targets_decl` (`:421-429`): the **tombstone** — the old
  body `targets: [...]` form still LEXES (as `annotation_body_item`, `:416-419`)
  solely so the parser can emit a NAMED migration diagnostic. It never yields a
  valid AST.

### 1.2 Parser (`crates/shape-ast/src/parser/extensions.rs`)

- `parse_target_kind` (`:21-31`): the single seven-kind string→`AnnotationTargetKind`
  match, called by **both** the header arm and the (now-rejecting) body arm, so
  the two spellings cannot drift.
- `Rule::annotation_on_clause` arm (`:69-83`): populates `allowed_targets` from
  the header.
- `Rule::annotation_legacy_targets_decl` arm (`:142-160`): returns
  `ShapeError::ParseError` with the named message
  *"annotation targets moved to the header `on` clause — write
  `annotation NAME(...) on function` (issue #73); the body `targets: [...]`
  field was removed"*. Never populates `allowed_targets`.

### 1.3 AST (`crates/shape-ast/src/ast/functions.rs`)

- `AnnotationDef.allowed_targets: Option<Vec<AnnotationTargetKind>>` (`:236`)
  — **unchanged shape**; only its population SOURCE flipped (body → header).
  No new AST field, no new variant, **no exhaustive-match cascade**.
- `AnnotationTargetKind` (`:264-280`): the seven kinds
  (Function/Type/Module/Expression/Block/AwaitExpr/Binding).

## 2. Design notes for supervisor ratification

- **DN1 — missing clause = inference (RECOMMENDED, implemented).** An absent
  `on`-clause leaves `allowed_targets = None`, which the planner's handler-kind
  inference resolves (`planner.rs:287` `unwrap_or_else`). The header **overrides**
  inference when present and is a no-op when absent. Recommended for G0
  ergonomics + checkability. Ratify.
- **DN2 — tombstone vs hard-delete (RECOMMENDED tombstone, implemented).** The
  body form is a NAMED migration recognizer (`annotation_legacy_targets_decl`),
  not a bare pest error and not a literal grammar hard-delete. Still exactly one
  accepted spelling — it parses to an ERROR, adds no runtime path, and is
  categorically distinct from the forbidden ValueWord-family transitional
  renames. Ratify.
- **DN3 — 3-commit staging + squash lever.** S1a is the single gated
  intra-slice dual-PARSE commit; S1b contains the deletion + tombstone; S1c is
  LSP + report. If the supervisor reads E4-D4 "deleted in the same change" as
  per-commit binding, S1a+S1b may be squashed (S1b already carries the
  deletion). The split is recommended for the 134-file sweep's bisectability.
- **DN5 — hover/completion on-clause rendering scope (DONE, not deferred).**
  `AnnotationInfo` cheaply threads `allowed_targets`, so the on-clause is
  rendered into the hover + completion signature now (§4) rather than filed as a
  follow-up. Application-site **@-completion FILTERING** by target kind remains a
  NAMED E4 follow-up (larger feature), not S1 scope.

## 3. Sweep manifest (S1b)

- **134 files / 530 mechanical transforms** rewrote every body
  `targets: [KINDS]` to a header `on KINDS` clause; a scripted multiline
  transform handled all physical encodings (real-newline, single-line,
  escaped-`\n`, continuation, `writeln!` pairs, doubled-brace `format!`
  strings, placeholder-carrying inline-Shape).
- **51 `.shape` files** (corrected count — **not** the pre-correction "19"
  undercount) carry header on-clauses at HEAD; **zero** `targets: [` residuals
  remain in any `.shape` source. This includes the 3 build-critical stdlib-src
  files (serde/derive, serde/serialize, llm/tools) — the workspace gate proves
  they compile under the header spelling.
- Plus hand-edited doc/grammar files excluded from the mechanical pass
  (`extensions.rs`, `ast/functions.rs`, `sugar_lowering.rs:208`, and the
  `shape.pest` header prose) — the 4 mandated stale-`targets:`-spelling doc
  comments corrected.

## 4. LSP surface re-sync (S1c) — anchors

Parser/AST/LSP/fixtures only; **zero** value/slot-layer reach.

- **Snippet** (`completion/snippets.rs`, `annotation-def`, `:126-140`):
  re-synced to the header form —
  `annotation ${1:name}(${2:param}: ${3:int}) on ${4:function} { … }` with the
  trailing tab-stop renumbered (`${4}`→`${5}`). This was the only LSP surface
  that would otherwise ship guidance toward a rejected spelling. Snippet count
  unchanged at **25** (`:210-212`).
- **`AnnotationInfo.targets: Option<Vec<AnnotationTargetKind>>`**
  (`annotation_discovery.rs:23-40`), populated from `ann_def.allowed_targets`
  at both construction sites (`:55`, `:127`).
- **`render_on_clause` / `annotation_signature` / `target_kind_word`**
  (`annotation_discovery.rs`): `target_kind_word` is an **exhaustive** seven-arm
  match (the LSP-tier anti-undercount guard); `render_on_clause` returns `None`
  for absent/empty targets so no `on`-clause is over-claimed under DN1.
- **Hover** (`hover.rs:614-620`): `get_annotation_hover` renders
  `**Annotation**: `@name(params) on <kinds>`` — "the full contract in one
  line" (issue #73). Absent clause → plain `@name` signature.
- **Completion detail** (`completion/annotations.rs:31-46`): the `@`-completion
  detail string mirrors the hover contract (display only — not filtering).

## 5. Rejection-pin identities

- **Named-negative** (`parser/tests/advanced.rs`,
  `test_legacy_body_targets_field_is_rejected_with_named_migration_diagnostic`):
  the body `targets: [...]` form is rejected with the migration message —
  asserts the message text ("annotation targets moved to the header `on`
  clause" + "issue #73"), **not** a bare `is_err` (non-vacuous).
- **Positive twin**
  (`test_legacy_body_targets_positive_twin_header_form_parses`): the same target
  set in header form parses and populates `allowed_targets == Some([Function])`.
- **Header pins** (`test_annotation_header_on_clause_single_kind` /
  `_multi_kind` / `_all_seven_kinds` / `test_annotation_absent_on_clause_leaves_targets_none`):
  the all-seven pin asserts each of the seven variants individually (guards the
  4-of-7 undercount the issue-73 correction fixed).
- **Migrated explicit-targets tests**
  (`test_annotation_def_with_explicit_targets_and_handler`, etc.): green in
  header form.
- **LSP non-vacuity pins** (S1c): `signature_renders_declared_on_clause`,
  `signature_omits_on_clause_when_targets_inferred`,
  `render_on_clause_covers_all_seven_target_kinds` (annotation_discovery.rs
  unit tests) + `test_hover_annotation_usage_renders_on_clause_contract`
  (hover_tests.rs).

### 5.1 S1c fixup of the named-negative pin (DISCLOSED DEVIATION)

The S1b commit shipped the named-negative fixture in the **header** form
(`annotation legacy_form() on function { … }`) by copy-paste from the positive
twin. That form parses cleanly, so `.expect_err(...)` panicked — the pin was
**RED at S1b HEAD `cb735069`**, not vacuously green. S1c restores the fixture to
the body `targets: [function]` form the tombstone actually rejects. This is a
fixture-only, append-only correction squarely in S1c's "pins" + "close S1
honestly" remit. `shape-ast --lib` is now **596/0/0** (was 595 pass / 1 fail).
The migration message and issue-#73 citation assertions now genuinely execute.

## 6. G8/G12 application-site validation — UNCHANGED

Application-site target validation and the planner declaration-tier rejections
read the **resolved** `allowed_targets` Vec and are source-agnostic — they fire
identically whether that Vec came from the header `on`-clause or (absent clause)
from handler-kind inference:

- planner declaration-tier: `planner.rs:287` (inference `unwrap_or_else`) +
  `:305-344` (empty = no restriction; non-Function-declared function
  application rejection; invalid-kind rejection).
- No validation logic was edited in S1; only stale `targets:`-spelling
  doc-comments were corrected.

## 7. Gate results (FAILED-name-set diff vs slice-0 §1)

All via the lane, `-- --test-threads=1`.

| Suite | Slice-0 | S1c | FAILED-name delta |
|---|---|---|---|
| shape-lsp --lib | 884 / 0 / 0 | **888 / 0 / 0** | none (4 new S1c pins, all green) |
| shape-test lsp | 506 / 0 / 0 | **506 / 0 / 0** | none |
| shape-ast --lib | (595 pass, 1 fail latent¹) | **596 / 0 / 0** | −1 fail (§5.1 fixup) |
| `just check-clean` | exit 0 | **exit 0** | — |

¹ The S1b named-negative red (§5.1) was latent because `just check-clean` only
`cargo check`s (it does not RUN tests) and S1c's gate set did not re-run
`shape-ast --lib` until this stage surfaced the contradiction. No slice-0
FAILED name flipped; no new FAILED name appeared. The only pre-existing warning
in the workspace gate (`unused import: super::*`,
`shape-vm/src/compiler/comptime_builtins.rs:3481`) is untouched by S1.

## 8. Close-out list (NOT S1 scope — recorded for the S1 CLOSE step)

- **Book-gate fixtures** (already swept in S1b, no chase-green here):
  `docs/cluster-audits/v0.3.3-book-acceptance/programs/annotations/*.shape`
  (probe1–8, small). **`large.shape` stays RED** on the orthogonal `ctx.state`
  grounds (slice-0 Risk #6) — NOT an S1 deliverable.
- **shape-web `annotations.mdx`** (SEPARATE repo) — CLOSE coordinates it.
- **~16 in-repo `docs/**/*.md` design docs** still showing the body spelling —
  swept at CLOSE for consistency: `CONTEXT.md`,
  `docs/design/comptime-excellence.md`, `docs/design/typed-comptime.md`,
  `docs/design/typed-comptime/annotations-and-hooks.md`,
  `docs/design/distributed-function-transfer.md`,
  `docs/design/typed-comptime/nominals-and-members.md`,
  `docs/vision/distributed-comptime-async-vision.md`,
  `docs/vision/rfc-comptime-transform-api-v1.md`,
  `docs/cluster-audits/wave40-argument-pack-design-hybrid.md`,
  `docs/cluster-audits/wave41-comptime-current-surface.md`,
  `docs/cluster-audits/v0.3-book-slice-C-divergence.md`,
  `docs/audits/2026-07-11-vertical-deep-dive/10-comptime-annotations.md`, plus
  the frozen prior-slice reports (e2/c3 slice reports — historical, may be left
  as-is).
- **CLOSE also owns** design-index CURRENT row + `defections.md` +
  book-truth-gate coordination.

## 9. Arch-invariant compliance (all 8 held)

1. One accepted spelling at slice end (S1a dual-PARSE was the single transient;
   DN2 tombstone parses-to-error, no runtime path). 2. No new AST variant / no
   exhaustive-match cascade (`allowed_targets` stayed `Option<Vec<…>>`). 3. All
   seven kinds header-eligible (asserted individually). 4. G8/G12 unchanged
   (§6). 5. Missing clause = inference (DN1). 6. Per-commit green by FAILED-name
   diff (§7). 7. Forbidden patterns at max binding — parser/AST/LSP/fixtures
   only, zero value/slot-carrier reach, no ValueWord-shaped anything. 8.
   Worktree `shape-adr009-a3` only; append-only; AGENTS.md untouched.
