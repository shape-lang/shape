# v0.3.2 + post-tag classification audit — taxonomy + rules

**HEAD at audit start:** `82f049dd` (post-v0.3.2 tag).
**Trigger:** user 2026-05-26 authorization after observing 1065 failing
tests in `shape-test` (18.7% of corpus) shipped silently through
v0.3.0 / v0.3.1 / v0.3.2 tags. Specific user-facing program
(filter on `Array<User>`) crashed at compile time on the playground
post-v0.3.2 deploy.

**Audit scope:** read-only, per-binary, per-test classification of all
~1065 failing tests in the `shape-test` crate. NO source changes. Each
agent writes one audit doc into this directory; team-lead aggregates +
commits at audit close.

---

## Taxonomy (refined by user 2026-05-26)

Each failing test maps to **exactly one** of these classes. The class
drives downstream disposition.

### FN-REG-CORRECTNESS — RELEASE-BLOCKING

Real correctness regression in the language. Source code under test is
plausibly-correct user-facing Shape that PREVIOUSLY ran (or that any
reasonable user would expect to work). The compiler / VM / JIT now
rejects or produces a wrong result.

Examples: filter on `Array<User>` rejected; `result.push()` inside
conditional not propagating element type; silent-wrong-output;
SIGABRT / SEGFAULT; VM ≠ JIT divergence.

**Required in the audit doc per FN-REG-CORRECTNESS entry:**
- Minimal repro (3-10 line Shape program that triggers the failure).
- Bisected regression commit, if findable (`git log --oneline -- <suspect-file>`).
- Affected stdlib symbol or compiler subsystem.

### FN-REG-DIAGNOSTIC — per-test fixture update

Test fixture asserts on specific diagnostic text. Compiler now emits a
different (also-correct) message. The language behavior is correct;
only the test's expected text is stale.

Pattern: assert-equal on error string; new substring missing or
old substring present.

**Required in the audit doc per FN-REG-DIAGNOSTIC entry:**
- Old expected text (from fixture).
- New actual text (from current run).
- One-line note on the language change that drove the new diagnostic.

### SCOPE-RECLAIM — RELEASE-BLOCKING (until user re-dispositions to v0.4)

Test SURFACEs on work that **was pulled into v0.3 by a dated user
disposition** AND **has not been explicitly re-dispositioned to v0.4
by a later dated user authorization**. SURFACE messages that cite
"v0.4 / planned" or "§5.16 follow-up" without the dated re-disposition
are MIS-CITES; the underlying failure routes here, not to V0.4-DEFER.

Dated user pull-ins that gate SCOPE-RECLAIM:

| Date | Authorization | Scope |
|---|---|---|
| 2026-05-18 | V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade. W16.2-A typed-object-element + W16.2-B trait-object-element + W16.2-C empty-literal/spread/comprehension. **The annotation_targets + annotations_comptime cluster IS THIS WORK.** | SURFACE messages citing "V3-S5 ckpt-5 consumer-cascade" or "§5.16 v0.4" are SCOPE-RECLAIM by default unless audit shows otherwise. |
| 2026-05-21 | Array<string> must work. Len trait. Object destructuring must fully work. | Test surfaces on any of these route here. |
| 2026-05-22 | Scope expansion: W16.2-J PHF-retirement + W17.3-4 per-container FieldType + phase-2c host-tier marshal/snapshot rebuild + 6 Known Constraints + doc-truth round. | |
| 2026-05-22 | Comptime trait into v0.3. | |
| 2026-05-22 | KC #2 format_* deletion. | |
| 2026-05-22 | W18 content-rendering rebuild into v0.3 ("regressions not an option"). | |
| 2026-05-26 | LSP-parity-with-rust-analyzer + BindingStorageClass opt-in inlay hints. | |

§5.16 JIT-lowering followup workstream (supervisor 2026-05-25) actual
scope: **aliased-CoW SEGFAULT + imported-const ident-eval + W17-marshal
+ Drop codegen + B2 EnumPayload.** §5.16 does NOT absorb V3-S5
construction-cascade work. SURFACE messages that cite §5.16 for non-
§5.16-scope work are mis-cites; the underlying failures route to
SCOPE-RECLAIM.

**Required in the audit doc per SCOPE-RECLAIM entry:**
- The dated user disposition the underlying work was pulled-in by.
- The exact SURFACE message text (verbatim or short quote).
- The (incorrect) v0.4 anchor cited by the SURFACE.
- Why the cite is incorrect (which row of the table above applies).
- Whether the test asserts on the SURFACE itself (test passes when
  SURFACE fires; test would need updating after fix) OR asserts on
  user-facing semantics (test stays the same after fix).

### V0.4-DEFER — legitimately v0.4 territory

Both conditions must hold:
- (a) **Never in v0.3 user-pulled-in scope** — work was never named
  by any dated user disposition above as v0.3-gating.
- (b) **Surface-and-stops cleanly** per ADR-006 §2.7.14 (no panic, no
  silent-wrong-output, no SEGFAULT; emits a structured error with
  feature-name + "v0.4 / planned" annotation).

**Required in the audit doc per V0.4-DEFER entry:**
- Brief reason the work is genuinely v0.4 (one sentence).
- Confirmation that surface-and-stop is clean (cite the SURFACE text).
- Recommended v0.4 issue ID (or `TBD-v0.4-<short-slug>`).

### INFRA-FLAKY — environment / parallelism / timing

Test fails intermittently OR depends on environment state (network,
disk, system clock) that's flaky. Re-running may pass.

**Required in the audit doc per INFRA-FLAKY entry:**
- Evidence of intermittency (re-run shows pass) OR environment dep.
- Whether to gate on retry, isolate to its own binary, or `#[ignore]`.

### UNKNOWN — requires per-test investigation

Cannot classify confidently from the failure output alone. May need
empirical bisect, deeper code-read, or supervisor input.

**Required in the audit doc per UNKNOWN entry:**
- What specifically blocks classification (what's missing).
- Recommended next-step (further investigation owner / depth).

---

## Per-binary close-doc format

Each agent writes `docs/cluster-audits/v0.3-classification/<binary>.md`
with this structure. **No source changes. No commits. Doc is the
only artifact.**

```markdown
# <binary-name> classification

**HEAD:** 82f049dd
**Total tests in binary:** N
**Passed:** P / Failed: F / Ignored: I
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test <binary> --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | x |
| FN-REG-DIAGNOSTIC  | x |
| SCOPE-RECLAIM      | x |
| V0.4-DEFER         | x |
| INFRA-FLAKY        | x |
| UNKNOWN            | x |

## Per-test classification

### <test_name_1>

Class: **<CLASS>**

<failure excerpt — 3-10 lines>

<class-specific required fields per the taxonomy above>

### <test_name_2>
...
```

If the binary has zero failures, write a single-line doc:
```markdown
# <binary> — all-green at HEAD 82f049dd. No classification needed.
```

## Hard discipline (per CLAUDE.md + handover)

- **Audit-only.** No source / fixture changes. The audit doc is the
  only output.
- **No commits during audit.** Team-lead commits all per-binary docs
  together at audit close.
- **`git stash` is FORBIDDEN.** Use `git commit -m WIP` to a private
  branch if you need state recovery. Per-worktree git alias is no-op
  for builtins (supervisor 2026-05-26 retraction); load-bearing is
  your self-discipline.
- **Run-verify binding:** every classification decision MUST be backed
  by an actual test-output excerpt from a `cargo test` invocation at
  HEAD. No paraphrasing.
- **Per-test discipline:** no bulk-classify. Each test gets its own
  row + evidence.
- **Cross-file `code_review` discipline:** don't expand scope by
  reading unrelated code. Limit reads to the failing test's fixture +
  the immediate stdlib / compiler symbol it exercises.
- **CLAUDE.md Forbidden Patterns** + Renames-to-refuse-on-sight apply
  to any defection-attractor framings encountered (e.g., "rationalize
  by deferring to v0.4" without dated re-disposition).

## What aggregates after audit close

Team-lead produces, after all per-binary docs land:

1. **Master truth-set:** `TRUTH-SET.md` rolling up all per-binary
   classifications into one taxonomy-wide table.
2. **SCOPE-RECLAIM audit:** `SCOPE-RECLAIM.md` enumerating the
   user-pull-in vs SURFACE-cite contradictions, sorted by user-
   pull-in date.
3. **Allowlist proposal:** `ALLOWLIST.md` listing the FN-REG-DIAGNOSTIC
   + V0.4-DEFER + INFRA-FLAKY entries as the new pre-tag-gate
   allowlist (with per-entry issue link or in-doc justification).
   ZERO FN-REG-CORRECTNESS + ZERO SCOPE-RECLAIM in the allowlist by
   design.
4. **Skill revision:** update `~/.claude/skills/shape-release/SKILL.md`
   with the new `cargo test -p shape-test --no-fail-fast` allowlist-
   diff gate + new combination-shape smoke fixtures + removed
   "pre-existing baseline" exemption.
