# Team-lead handover — Shape v0.3 close-approach

**Refreshed:** 2026-05-25 at main HEAD `70507224` (post-R8-W9 close-doc
refresh). v0.3 §0.A criteria A–J all substantively met; supervisor
close-ratified at HEAD `70507224`. **v0.3.0 tag HELD** per user
2026-05-25 directive: shape-lsp at HEAD shows 22 test failures vs 14
baseline (+8 new) and the LSP's empirical functional state in a real
editor has not been verified. **NEW v0.3 workstream:** LSP-parity-with-
rust-analyzer ("inline hints top notch + all other features en-par or
better than rust-analyzer"). Tag does NOT land until LSP-parity closes
and user re-authorizes.

## Role

Team-lead for the Shape v0.3 close-approach. Runs the program **autonomously**
— dispatch / partition / batch / merge / close-gate are self-authorized.
Supervisor handles architectural calls; user authorizes tags + language
semantics. User relays between team-lead and supervisor.

## Current state

| | |
|---|---|
| Main HEAD | `70507224` (post-R8 W9 close-doc handover refresh; substance HEAD = `64a2d8e1`) |
| Smoke matrix s1–s5 | **5/5 VM == JIT** (canonical (ii) F' release-binary harness; re-verified at HEAD `70507224` at LSP-parity session entry) |
| verify-merge / check-no-dynamic / check-clean | 13/13 / exit 0 / exit 0 |
| git-stash pre-commit hook | DEPLOYED (R8 W5+W6+W7 ZERO violations cumulative) |
| conflict-marker pre-commit hook | DEPLOYED R8 W7 close (supervisor 2026-05-24 operational suggestion after the 5669a8ff incident); tested empirically (catches `+<<<<<<<` / `+=======` / `+>>>>>>>` in staged diff, exits 1 with recovery instructions) |
| Co-Authored-By trailers (cumulative) | 0 |
| Bad-code merges (cumulative) | 0 (1 conflict-marker miss in R8 W7 was caught + cleaned in follow-up commit `675dcf1b`) |
| v0.3.0 tag | NOT landed — gated on remaining items + user authorization |

## R8 W6 dispatch — Supervisor 16-item disposition (binding 2026-05-24)

Apply user 2026-05-20 no-known-incorrectness binding: memory-unsafety /
VM-JIT divergence / silent-wrong-output / spurious-reject of valid code
ship as v0.3-gating. Incomplete-but-CLEAN ships as v0.4-OK with surface-
and-stop messaging.

| Group | Disposition | Items |
|---|---|---|
| **1** | v0.3-gating MUST FIX (memory-unsafety / divergence; unconditional) | B JIT TypedArrayPushI64 FrameDescriptor SEGFAULT (audit-first); E b-4 transport::tcp() VM/JIT divergence (audit-first) |
| **2** | v0.3-gating panic→structured-error conversion per ADR-006 §2.7.14 SURFACE (feature impl → v0.4) | E b-1 Wave 5d intrinsic ~40+ todo!()'s; io path utilities (if panic); as-? operator (if panic); string-keyed object literals (if panic); transport::quic NYI (if panic) |
| **3** | v0.3 a-class doc fix | http options-arg required (8 fns); other a-class surfaced during G.2 |
| **4** | v0.4 deferral (already errors cleanly OR pure feature-add) | clone in call-arg (verify clean), cview/cmut C-ABI, E b-5 module schema gap, E b-6 transport::quic NYI, F domain legacy type syntax, A G1-B-FQ7-V2VERIFIER stderr noise, J.5e iterator-protocol |
| **5** | Investigate-then-classify (per-binding routing) | C Match enum-payload inference; E b-2 pure-Shape stdlib inference family (~9-10 files); E b-3 HashMap key-kind discriminator gap + W17.3-4.3 alignment |

## R8 W6 outcome (CLOSED 2026-05-24)

4 merges + 5 audit docs + ADR drafts file landed; all close-gates green
at every checkpoint; smoke 5/5 VM == JIT preserved throughout.

| Merge | Commit | Substance |
|---|---|---|
| G.2 bulk panic→SURFACE | (in d8d79daf history) `d61171f4` → merge | 13 dispatch arms in vm_impl/builtins.rs converted Wave-5c/5d/5e |
| G.1 W17 IoHandle/DataTable | `5b134204` → `6bdf09fc` | 2 KindedSlot ctors + 2 project_typed_return arms; eliminates transport::tcp VM/JIT divergence |
| G.1 JIT FrameDescriptor | `1ef4ca9d` → `9639d8c4` | annotation-handler local-storage-hint capture; verifier complaints eliminated |
| G.3 http options-arg | `94dc8fa9` → `d8d79daf` | doc-fix path: 8 fns in http.shape + http.mdx (shape-web) |
| R8 W6 audit-round close | `8570e228` | 5 audit docs + ADR drafts + handover refresh |

## R8 W7 outcome (CLOSED 2026-05-24)

5 merges + ADR §2.7.28/§2.7.29 verbatim text apply + aliased-CoW
audit-doc landed per supervisor 2026-05-24 ratify (ADR 4-decisions + match
enum-payload v0.3-gating override + aliased-CoW v0.3 scope confirmation).

| Merge / Commit | Substance |
|---|---|
| G.5 HashMap divergence-elimination → `83e6e86a` | JIT V2-verifier refusal routes through existing `[jit-fallback]` interpreter path (Option B per audit §5; Option A reverted after empirical smoke-s2 evidence of regression). Eliminates `set::from_array([1,2,3])` JIT garbage-return divergence; smoke 5/5 preserved. |
| Match enum-payload → `5669a8ff` (+ cleanup `675dcf1b`) | TWO sites fixed (audit suggested one): `bind_pattern_vars_typed` Tuple arm in inference engine + `compile_typed_enum_binding` Tuple arm in bytecode compiler via new symmetric `enum_tuple_variant_fields` cache. Sanity test revealed silent-wrong-output on Struct payload that the fix also resolves. |
| ADR §2.7.28/§2.7.29 apply + aliased-CoW audit → `cfd613d8` | §2.7.28 (W17-typed-module-exports) + §2.7.29 (W17-foreign-ffi) verbatim text inserted at ADR-006 line 6587 (+422 LoC). 5 marker comments added at named source locations. Post-apply grep checks all clean. Plus aliased-CoW SEGFAULT audit at `v0.3-r8w7-jit-aliased-cow-segfault-audit.md` (root cause at mir_compiler/v2_array.rs:591-606; M-scope refcount-aware codegen fix recipe). |
| G.5 Cluster C empty-array annotations → `97911e5b` | property_testing.shape + monte_carlo.shape: `Array<int>` / `Array<number>` element-type annotations + necessary Cluster-B-style unblock annotations on param/intermediate types. Anonymous-typed-object element annotation empirically rejected; refused to fabricate workaround. |
| G.5 Cluster B type annotations → `ebb3717c` | testing.shape + math::{optimize, linalg, rotation, interpolation}: function-signature type annotations + 3 documented architectural workarounds: (1) nested `Array<Array<T>>` empty-array workaround via seed-then-overwrite; (2) cross-module generic-fn-via-namespace gap → import-only smoke; (3) generic type-arg inference gap → monomorphize `clamp<T>`/`lerp<T>` to `clamp_int`/`lerp_num` (private helpers only; global `clamp`/`lerp` in math.shape untouched). |

**Cluster A NOT dispatched** — language-semantics decision needed:
log.shape requires module-level mutable state (`let mut current_level`);
fix path is either (a) implement module-level const + refactor log.shape
to function-state OR (b) implement module-level mutable bindings OR (c)
v0.4 with stdlib refactor. Surface to user for ruling.

## Supervisor surfaces (R8 W7 close)

1. **ADR §2.7.28/§2.7.29 LANDED** at `docs/adr/006-value-and-memory-model.md`
   lines 6587-7008 area (5 marker comments + 8 post-apply grep checks clean).
   Drafts-file mechanism confirmed working — same pattern usable for any
   future multi-section ADR text surfaces.
2. **Match enum-payload v0.3-gating fix LANDED** per supervisor's
   doc-contract-leg ruling. Two-site fix; sanity test surfaced
   silent-wrong-output on Struct payload that the same fix resolves.
3. **Aliased-CoW SEGFAULT** audit-doc landed; M-scope fix queued for
   R8 W8+. Fix territory: `crates/shape-jit/src/mir_compiler/v2_array.rs:591-606`
   (refcount-aware codegen with `jit_clone_array_if_shared` FFI wrapper).
4. **G.5 Cluster A** language-semantics decision needed (module-level
   const-only vs module-level mutable state vs stdlib refactor + v0.4).

## Supervisor surfaces (for relay)

1. **ADR §2.7.4 addendum drafts file** at `docs/cluster-audits/v0.3-adr-2-7-4-addendum-drafts.md` — supervisor reads from disk per the new mechanism (bypasses relay-chain text-loss). 4 `[SUPERVISOR-DECISION: ...]` markers: (a) NEW top-level §2.7.28/.29 vs amendment-subsection under §2.7.4; (b) insertion position (numeric vs file-position dominant); (c) retro-add `// §2.7.28` / `// §2.7.29` marker comments; (d) Q-number allocation (next available Q26/Q27).
2. **Match enum-payload inference (G5)** — recommended v0.4 in audit; surface if supervisor reads doc-contract leg of no-known-incorrectness binding stronger than clean-compile-error leg. See `docs/cluster-audits/v0.3-r8w6-match-enum-payload-inference-audit.md` §2.

## R8 W8 outcome (CLOSED 2026-05-24)

1 merge + 1 audit-correction landed.

| Merge / Commit | Substance |
|---|---|
| JIT aliased-CoW SEGFAULT → `ec184dc9` (merge in `ff978be4` lineage) | Surface-and-stop in `mir_compiler/v2_array.rs::try_emit_v2_array_method` push arm. New `mir_has_prior_move_of_slot(slot)` helper scans MIR statements + terminators for prior `Operand::Move/MoveExplicit(Place::Local(slot))`; on match returns Err with structured SURFACE message to trigger W12 fall-through to interpreter (preserves VM in-place mutation semantics). repro1 post-fix: VM ec=0 + JIT ec=0 (deopt to interpreter), both print `[1,2,3,4]` twice. |
| Audit empirical-correction → `ff978be4` | §7 added to `v0.3-r8w7-jit-aliased-cow-segfault-audit.md`: gdb investigation falsified the §3 refcount-aliasing hypothesis (actual cause: `Operand::Move` nulls source slot during `let alias = data` lowering at `mir/lowering/stmt.rs:269-273`). The audit's §4 CoW recipe would have CREATED a NEW VM/JIT divergence (CoW-cloned JIT vs in-place VM). Surface-and-stop is the correct binding-compliant path. Future SEGFAULT-audit lesson preserved (use print() probes to localize slot-loss before hypothesizing refcount issues). |

## R8 W8 extended close additions (CLOSED 2026-05-25)

3 additional merges + 2 docs commits on top of the original R8 W8 (aliased-CoW):

| Merge / Commit | Substance |
|---|---|
| Conflict-marker pre-commit hook | `.git/hooks/pre-commit` extended with `^\+(<<<<<<<\|=======\|>>>>>>>)` staged-diff guard per supervisor 2026-05-25 operational suggestion. Tested empirically: catches markers + rejects + provides recovery instructions. Same mechanical-enforcement pattern as the git-stash hook. R8 W7 5669a8ff incident shape now blocked at commit time. |
| close-summary §5.15 + §5.16 (commits `86de03be` + `a927e607`) | §5.15 "Module-level mutable bindings + concurrency design pass v0.4" bundles module-level mutable state + thread-safety in async-scope + Send/Sync + Mutex/Atomic/Lazy interaction as a coherent v0.4 design pass per user 2026-05-25 framing. §5.16 "JIT-lowering followup workstream v0.4" bundles the 2 R8 W8 v0.3-gating JIT surface-and-stops (aliased-CoW + imported-const ident-eval) for a coherent v0.4 root-cause-fix workstream per supervisor 2026-05-25 bundling. |
| Cluster A module-level const + log.shape Logger refactor → `55fc8531` | Per user 2026-05-25 Option (a) ruling. New `const NAME: Type = expr` at module scope (parser + type-checker + comptime evaluator + bytecode emitter wired through existing Constant-pool mechanism; ADR-006 §2.7.5 stamp-at-compile-time invariant preserved). distributions_advanced.shape `let PI/E/SQRT_2PI` → `const PI/E/SQRT_2PI: number`. log.shape rewritten to explicit `Logger` struct + `pub const LEVEL_*` + free-fn API; module-level mutable `current_level` removed per v0.4 concurrency-design-pass deferral. |
| Cluster A JIT imported-const surface-and-stop → `5ac2613e` (merge in `326f41bd`) | The Cluster A landing introduced a new VM=2/JIT=0 silent-wrong-output divergence on `print(IMPORTED_CONST)` bare + sibling shapes. Per supervisor 2026-05-25 path (i) ruling: NEW `has_imported_const_inline: bool` flag on `BytecodeProgram`/`Program`/`LinkedProgram` set at the Cluster A intercept; JIT preflight refuses + triggers W12 `[jit-fallback]` whole-program deopt to interpreter. Convergence achieved on 5 divergence repros. Root-cause fix in JIT identifier-eval lowering → v0.4 per §5.16. Mirrors aliased-CoW precedent. |

The aliased-CoW SEGFAULT + JIT imported-const ident-eval are the FIRST
TWO members of the §5.16 v0.4 JIT-lowering followup workstream — a
named bundle, not piecemeal items.

## R8 W9 outcome (G.2 Step 2 CLOSED 2026-05-25)

236 USER-followable programs across 6 parallel slice agents; 24 (a) +
28 (b) + 7 (c). All 6 supervisor-dispositioned buckets landed:

| Merge | Commit | Substance |
|---|---|---|
| B2+B7 batch | `dbf25a5a` → `8f3da917` | EnumPayload MIR preflight + comptime_target panic→Err |
| B1 W17-marshal | `77546a3b` → `5a1bddb0` | has_w17_marshal_residual flag + JIT preflight; state::serialize divergence eliminated (re-dispatch after pre-merge catch of "already convergent" wrong-narrowing) |
| B3 Drop runtime | `50910757` → `cb5683bb` | VM MakeRef sentinel + DropLocal-guard + JIT Drop-trait preflight (interpreter Drop dispatch sound per audit §4) |
| B5+B9 bundle | `8bbd2f99` → `fa516bab` | B9 categorical ref_borrow ban deletion + B5 distributions_advanced/ode annotations |
| Audit batch | `64a2d8e1` | 8 audit docs: 6 G.2 Step 2 slices + 2 R8 W9 audit-day (borrow-b0003 + stdlib-inference-residuals) |
| shape-web (a) batch | `228f6eb` (in shape-web) | 4 .mdx files: functions/log/set/collections doc-fixes |

**Reclassified (c) v0.4** (already in close-summary §5.14/§5.16 inventory):
- B4 Wave-5d intrinsics (already R8 W6 G.2 panic→SURFACE; agent (b) classification incorrect)
- B6 HashMap/Set key-kind (already §5.16 v0.4 epic; (a) Set chapter caution added in shape-web batch)
- B8 extern C libm (already §5.14 W17-foreign-ffi-followup)

**§5.16 v0.4 JIT-lowering followup workstream** now has 4 named members
(aliased-CoW + imported-const ident-eval + W17-marshal + Drop codegen)
+ B2 EnumPayload preflight surface-and-stop's root-cause (§2.7.17
receiver-recovery extension) listed.

**3-for-3 catch-pre-merge** preserved (R8 W7 aliased-CoW + R8 W8
Cluster-A-JIT + R8 W9 B1-narrowing). Slice-agent (c)-verify self-
correction layer matured (4 (c)→(b) self-corrections at audit time
during R8 W9).

## Remaining v0.3 scope — NEW LSP-PARITY WORKSTREAM (2026-05-25)

User 2026-05-25 directive (binding): **inline hints "top notch"; all
other LSP features "en-par or better than rust-analyzer"; functional in
a real editor (not just unit-test-green).** v0.3 §0.A criteria A–J
substance-met but the tag is held pending this addition.

Honest sizing: **~5–10 sessions** to a defensible "en-par" position +
tag. Audit-day refines.

### Phase structure (proposal — LSP-A audit refines)

- **PHASE LSP-A — AUDIT-DAY.** Single agent, audit-only (no source
  changes). Output: `docs/cluster-audits/v0.3-lsp-parity-audit.md`.
  Sections §A–G:
  - §A current Shape-LSP feature surface (per-feature grep + behavior)
  - §B 22 failing-test per-test characterization (fixture-drift vs
    functional regression; no bulk fixture updates)
  - §C rust-analyzer feature-set survey + per-feature gap
    (PARITY / GAP-MINOR / GAP-MEDIUM / GAP-MAJOR / NOT-APPLICABLE)
  - §D empirical functional state in a real editor (VS Code OR neovim
    adapter; hover / inlay / completion / diagnostics / signature-help
    / go-to / refs / symbols / code-actions / semantic-tokens / codelens
    / rename); transcripts/screenshots
  - §E sub-cluster partition into closure-waves
  - §F v0.3-vs-v0.4 deferral line per feature (user-decision items
    surfaced)
  - §G 22-test routing per-test → fix vs update-fixture-in-lockstep
- **PHASES LSP-B+ — SUB-CLUSTER DISPATCH WAVES** per audit §E
  partition. Parallel where territory non-overlap; serial where not.
- **PHASE LSP-CLOSE — EMPIRICAL VERIFICATION + close-relay.**
  Re-run audit §D in-editor against post-fix LSP across the test
  matrix (VS Code + per user direction).

### LSP-specific discipline (additive to standing bindings)

- **Test fixtures NEVER bulk-updated** to turn red green. Each red is
  investigated per-test.
- **Functional verification in a real editor is not optional.** Unit-
  test-green ≠ LSP works.
- **Rust-analyzer parity is directional**, not literal — audit
  identifies what matters; user ratifies per-feature.

### LSP-specific surface triggers (additive to standing 4)

- LSP-A audit close → surface for supervisor + user ratify of per-
  feature v0.3-vs-v0.4 line.
- Empirical-in-editor finding that's a functional regression →
  v0.3-gating fix dispatch.

### Parked items (NOT for this phase)

- v0.3.0 tag landing (deferred until LSP-CLOSE + supervisor re-ratify
  + user re-authorize).
- Multi-repo coordination: shape-web tag (currently `228f6eb`
  post-G.2-batch); shape-app playground rebuild + redeploy; shape-mcp
  / shape-registry / shape-infra version tags; tag-push CI deploy
  mechanism. Surface as a coordination relay at the actual tag-land
  step (after LSP-CLOSE), not before.

## ADR §2.7.4 addendum text-ratify — STRUCTURAL FIX

Relay-chain forwarding failed THREE times. Substance + direction ratified
2026-05-24 (commits `33f165cd` typed-module-exports + `e9f73b57` foreign-ffi);
verbatim ADR-doc-insertion text owed. NEW MECHANISM: agent drafts verbatim
text into `docs/cluster-audits/v0.3-adr-2-7-4-addendum-drafts.md`. Supervisor
reads file directly from disk, bypasses relay. Same Q3 pre-flight +
post-apply-grep discipline as §2.7.13 ratify.

## Trajectory

**~5–10 sessions to v0.3.0 tag** (LSP-parity scope addition 2026-05-25).
A–J substance was effectively-0 at HEAD `70507224`; the LSP-parity
binding extends the cycle. LSP-A audit-day produces a firmer estimate
+ partition; re-project at LSP-A close.

## Dispatch hygiene (binding — R7 W3 + R8 W4 hardening)

Agent `isolation: "worktree"` is unreliable. Pre-create a sibling worktree
(`git worktree add /home/dev/dev/shape-lang/shape-<branch> -b <branch>
<main-HEAD>`), dispatch WITHOUT `isolation`, pin the agent to that
absolute path as its first `cd`, forbid `cd`-ing to
`/home/dev/dev/shape-lang/shape`. Each fix branch is built + reproducers
re-verified on main post-merge before the next dispatch.

**`git stash` ABSOLUTE BINDING (supervisor 2026-05-23 + 2026-05-24
pre-commit-hook enforcement):**

> Parallel-dispatch agents are FORBIDDEN from `git stash` in any form.
> State-recovery uses targeted `git checkout -- <file>`, `git reset
> HEAD <file>`, or explicit commits in the agent's own pinned worktree.
> The shared `.git/refs/stash` stack is off-limits. Every dispatch
> prompt for a parallel-worktree agent MUST include this verbatim.

Mechanical enforcement: pre-commit hook at `.git/hooks/pre-commit` rejects
commits while `git stash list` is non-empty (shared across all worktrees).
**R8 W5 ZERO violations** — hook is working.

**Audit-day exception:** read-only audits (no source changes) run in the
main repo without worktrees provided each writes only its own audit doc and
stops without committing. Team-lead commits the audit docs together at
audit-round close. R8 W6 has 5 read-only audits + investigations active.

## Canonical (ii) F' smoke harness

Release binary; NOT pipe-to-tail.
```bash
out=$(timeout 30 ./target/release/shape run --mode $m $f 2>&1)
ec=$?; last=$(echo "$out" | tail -1)
```
Fixtures `tests/smokes/s{1..5}.shape`; expected s1 `4950` / s2 `30` / s3 `x`
/ s4 `2` / s5 `x`, all ec=0, VM == JIT.

## Pending the v0.3 tag

1. A–F land + smoke 5/5 preserved at every checkpoint.
2. G.1 Step 1 (a)/(b)/(c) groups close (R8 W6 in flight).
3. G.2 Step 2 closes; any new (b)-class findings folded.
4. ADR §2.7.4 addendum text ratifies.
5. Relay close-evidence to supervisor; supervisor ratifies.
6. **User** authorizes the `v0.3.0` tag.
7. Team-lead lands the tag at the authorized commit.

## Bindings — refuse on sight

Full detail: CLAUDE.md + ADR-006 §2.7.x. Operative:

- **No-known-incorrectness-ships-in-v0.3** (user 2026-05-20): known-incorrect
  (crash / VM-JIT divergence / wrong result / memory-unsafety / silent-wrong-
  output / spurious-reject of valid code) → v0.3-gating; incomplete-but-CLEAN
  → v0.4-OK with surface-and-stop messaging.
- **Regressions are not an option** (user 2026-05-22).
- All CLAUDE.md Forbidden Patterns + Renames-to-refuse-on-sight + ADR-005 §1
  + ADR-006 §2.7.x + 4-table HeapKind lockstep + 5-arm receiver-recovery.
- **Run-verify binding** (supervisor 2026-05-22): every repro in a surfacing
  MUST run-verify at HEAD before relay.
- **Q3 ground-truth before disposition.**
- No Co-Authored-By trailer; own all code quality.
- **Group 2 conversion discipline:** structured errors per ADR-006 §2.7.14
  SURFACE pattern (feature-name + clear "v0.4 / planned" annotation). NOT
  silent no-ops; NOT Bool-default; NOT panic.

## Cadence

Autonomous. Surface only on: (1) defection-attractor framing; (2) ADR
amendment text drafted (use the designated drafts file); (3) novel
architectural gap needing scope decision; (4) user-decision item. Relays
≤ ~80 lines; plain code fences; HEAD-cite + facts + one specific ask.

Multi-session rotation expected. Refresh this doc at every rotation.

## Close gates (every checkpoint + tag commit)

`just check-clean` exit 0 · `verify-merge.sh` 13/13 · `check-no-dynamic.sh`
exit 0 · smoke s1–s5 5/5 VM == JIT (canonical (ii) F' release-binary
harness) · git-stash pre-commit hook ZERO violations preserved · AGENTS.md
row appended; no Co-Authored-By trailer.

---

*LSP-PARITY-SCOPE ENTRY 2026-05-25 at HEAD `70507224`: v0.3 §0.A
substance A–J ratified by supervisor; v0.3.0 tag HELD per user
2026-05-25 directive on LSP "en-par with rust-analyzer + inline hints
top notch + functional in a real editor". Trajectory re-projected
~5–10 sessions to tag. PHASE LSP-A audit-day dispatched as first
action; all other dispatch held until audit lands.*
