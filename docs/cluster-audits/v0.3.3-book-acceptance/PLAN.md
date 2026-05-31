# Book-Driven User-Acceptance Validation — v0.3.3 pre-tag gate

**Status:** RATIFIED (supervisor + user 2026-05-28). GATED — fires only when the
team-lead declares the v0.3.3 fix-set substantively complete (all 1220
release-blocking green or allowlisted: FN-REG-CORRECTNESS = 0 AND SCOPE-RECLAIM
closed). Not before.

**Purpose.** v0.3.0 passed every internal gate and still broke for the first
real user — who followed the book and wrote `filter on Array<User>`. The
shape-test corpus tests *fixtures*; it never tests "can a person who only read
the book build something real that produces correct answers." This gate closes
that hole and is the standing pre-tag acceptance gate from v0.3.3 forward.

## User dispositions (2026-05-28)

1. **Trigger:** after the FULL fix-set (all 1220 green/allowlisted). True
   pre-tag signal — every slice should pass; any failure is a real escape.
2. **Methodology:** book-primary, reference-fallback. Agents follow the book
   first; may consult the MCP / reference docs ONLY when the book is silent,
   and MUST flag every such fallback (a fallback IS a `BOOK-GAP` finding).
3. **Large program:** real-world, **non-interactive**, **machine-proofable**.
   Computes results and asserts them against book-derived expected values.

## Partition — ~51 vertical slices (one author agent each)

Grounded in the book TOC (`shape-web/book/book-site/src/content/docs/`).
See `run-validation.workflow.js` `SLICES[]` for the canonical list + per-slice
chapter paths. Groups:

- **Core language (19):** variables/scope · builtin+integer types · operators ·
  control-flow · functions+closures · strings/interpolation · objects-arrays ·
  enums · traits · generics · pattern-matching · error-handling (`?`/`!!`) ·
  references-borrowing · resource-mgmt/Drop · modules · async · datetime ·
  content-rendering · tables/queryable
- **Advanced (8):** comptime · annotations · jit-compilation · ownership ·
  native-C interop · security/permissions · resumability/snapshots · transport
  (single-process reduced)
- **Stdlib (≈18 grouped):** collections · set · state · log · stats
  (distributions/stochastic/random) · numeric-sim (monte_carlo/ode) · rolling ·
  testing/property_testing · math-core · linalg · optimize · interpolation+rotation ·
  domain {finance, iot, physics, simulation} · serialization
  (json/yaml/toml/xml/csv/msgpack) · filesystem (file/io/env/archive/compress) ·
  http · regex+unicode · crypto · time+parallel
- **Polyglot (2):** python-extension · typescript-extension
- **Excluded (not program-testable):** installation, editor-setup-*, repl,
  mcp-server, faq, troubleshooting, configuration.

## Per-agent protocol (author)

1. Read ONLY the assigned book chapter(s). Reference/MCP fallback allowed when
   book is silent — flag each as a `BOOK-GAP`.
2. Write a **small program** (~20–60 LOC; idiomatic; exercises chapter core).
3. Write a **large program** (~1000 LOC; real-world; non-interactive;
   machine-proofable). It computes results and asserts them via explicit checks,
   ending with a `ALL_CHECKS_PASSED` sentinel on success and `CHECK_FAILED: …`
   lines on mismatch. **Expected values are derived from book semantics and
   written BEFORE the first run — never back-filled from observed output.**
   Deterministic: seed any randomness, fix any clock, use local/fixture inputs
   for I/O. Where a slice is genuinely non-deterministic (live http/time),
   declare `run-only-no-assert` with rationale + use the closest deterministic
   proxy.
4. Run both programs under **VM and JIT** via the canonical (ii) F'
   release-binary harness. VM==JIT byte-divergence = release-blocker.
5. Classify each failure: `FN-REG-CORRECTNESS` / `SCOPE-RECLAIM` / `BOOK-WRONG`
   (book documents behavior the language doesn't have) / `BOOK-GAP` (book
   insufficient to author) / `V0.4-DEFER` / `AUTHOR-ERROR` (own typo — fix and
   re-run, a real user would).
6. Save both programs + a per-slice report to
   `docs/cluster-audits/v0.3.3-book-acceptance/programs/<slice>/`.

## Discipline (what makes the gate honest)

- **No working around language bugs.** Fix your own authoring typos; never
  restructure to dodge a defect. The book is the arbiter: book says it works +
  it doesn't → defect, recorded, not worked around.
- **First-run truth.** Record first-run results. Do not debug-iterate to green.
- **Expected-values-before-run.** Assertions encode book-correct answers, not
  observed output. This is the anti-tautology rule.
- **Blind authorship.** Agents don't see each other's programs.
- **git-stash forbidden** (standing absolute binding). Read/Write own slice dir
  only.

## Orchestration (3 phases — `run-validation.workflow.js`)

1. **Author+Run** (pipeline, ~51 slices): write small+large, run VM+JIT, classify.
2. **Adversarial verify** (per slice, pipelined after its author): a *second*
   agent (a) re-runs both programs for reproducibility; (b) independently
   confirms a sample of the large program's expected values are book-correct
   (catches assert-the-bug); (c) checks the large program exercises the
   chapter's documented breadth, not just happy-path; (d) re-adjudicates each
   failure classification.
3. **Compile** (synthesis agent): master truth-set — per-slice pass/fail,
   VM==JIT divergences, failure taxonomy with counts, BOOK-GAP/BOOK-WRONG
   findings, go/no-go signal. Writes `RESULTS.md` in this dir.

## Gating

A FN-REG-CORRECTNESS, SCOPE-RECLAIM, BOOK-WRONG, or VM!=JIT finding from this
gate is RELEASE-BLOCKING for the v0.3.3 tag. BOOK-GAP findings are
documentation-blocking (book fix required) but routed to the shape-web book
owner, not necessarily language-blocking. AUTHOR-ERROR and clean V0.4-DEFER do
not block.

## How to fire (team-lead, post-full-fix-set)

```
Workflow({ scriptPath: "docs/cluster-audits/v0.3.3-book-acceptance/run-validation.workflow.js" })
```

Review `RESULTS.md` + relay the go/no-go + any release-blocking findings to
supervisor. This gate runs AFTER `just test-fast` + the classified-allowlist
diff gate are green, as the final independent pre-tag acceptance step, before
the LSP manual-editor close-gate and user tag authorization.
