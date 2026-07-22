# ADR-009 Session Handoff — 2026-07-22 (C3 closed, E4 through slice 1)

Supervisor session (Fable) handing off. Everything load-bearing lives in the
repo, the AGENTS.md registry row, and GitHub — this file is a pointer map.

## Board: 19 of 22 conceptually (18 merged + E4 in flight)

Merged: A1-A3, B1-B7, C1, C2, **C3 #14 (`5bc04b08`, 2026-07-21)**, D1/D2, E3,
**E1 #17 (`5cc4a84c`, 2026-07-20)**, E2. Remaining: **E4 #20 (IN FLIGHT)** →
E5 (blocked on B4/B5 via #61) → E6 → F1 → re-run the ADR-009 completion gate
(the 2026-07-16 run FAILED 3/3 refuters; it must genuinely pass) → RTP-00.

## Standing user rulings (binding; do not re-litigate)

- **NO USERS / greenfield** (`memory: no-users-greenfield-surface`).
  Compatibility carries zero weight. Criterion: ergonomics + LSP richness +
  compile-time checkability + architectural soundness. Cut cleanly and
  re-implement rather than staging legacy forward.
- **Named rejections must fail LOUD.** Silent no-ops are the worst state.
- **Model policy (2026-07-22):** workflow fleets run **Opus**
  (`model: 'opus'` on every `agent()` call); the supervisor stays **Fable**
  and makes the large decisions (`memory:
  opus-implementers-fable-supervision`).
- **Metaprogramming-first** (C3-G1): every surface rests on a complete
  comptime API; sugar lowers onto it; stdlib abstracts complexity.

## E4 #20 — EXACTLY where work stopped

Branch `adr009/e4` in `../shape-adr009-a3`, base `bddd2489` (= main post-C3).
**Program of record: `docs/design/typed-comptime/e4-decisions.md`** (E4-D1..D7
+ D-baseline, all USER-RATIFIED 2026-07-22) — read it first, plus the
**2026-07-22 charter-correction comment on issue #20** which BINDS over the
issue body (the body is a pre-C3 snapshot: it describes migrating an @remote
implementation that C3 already deleted).

**Done:**
- **S0 gated** (`70f2b05b`, docs-only): the baseline FAILED-name sets of
  record (`docs/design/typed-comptime/e4-slice0-report.md` §1) — vmlib
  STABLE-6 (+ `nested_exact` as the only permissible 7th, N≥4 `--exact`
  protocol), ann_comptime 10-name, comptime 3-name, modules_visibility
  1-name + 3 dark-window ignores; greens 36/24/506/884/58.
  **Spike verdicts (both in-bounds):** **D7-D-internal-branch** (Result-match
  proven native at last; the wrapper's decision discrimination stays native as
  a compiler-internal typed branch — the user-facing protocol remains the D1
  enum) and **D2-C-delete** (execution-proven zero readers of
  `ctx.state`/`event_log`; the dormant `__annotation_ctx_` whitelist row goes
  with them).
- **S1 gated CLOSED** (`1e86713f` + `cb735069` + `c65c1d8e`; verify 3/3 PASS
  round 1; full gate PASS 10/10): issue **#73** landed — `annotation
  name(config) on <kind>, <kind> { … }`; the body `targets:` field is DELETED
  with a NAMED migration tombstone diagnostic; all seven `AnnotationTargetKind`
  kinds are header-eligible; 134 files / 530 transforms swept; LSP snippet +
  hover re-synced (4 new LSP pins render the on-clause contract).
  **Supervisor-ratified design notes:** DN1 — a missing on-clause means
  INFERENCE (`allowed_targets` stays `Option`; `None` → existing planner
  inference; the header OVERRIDES when present); DN2 — the tombstone
  (named migration diagnostic) is the correct shape, not a silent parse
  error; DN3 — the 3-commit split stands (S1a's dual-PARSE window was an
  intra-slice transient that parses to an ERROR, never a shipped duality).

**Next: S2** (#74 interim loud rejection — flip the frozen scope-fence pin
`s8a_scope_fence_comptime_only_annotation_on_extern_c_fn_stays_unrejected` in
`functions_foreign.rs` to a loud named rejection citing #74) → **S3** (ctx
`FieldType::Any` deletion per D2-C at `functions_annotations.rs:1473-1474` +
`installer.rs:190-192` + the whitelist row) → **S4** (the HookDecision
protocol core — review-mandatory, the deep slice) → **S5** (@remote reborn in
final `on function` syntax) → **S6a-f** (the 21 acceptance tests flipped in
capability waves A→F) → close.

## The S4 hazards (top of the list; from the ratified decisions)

1. **Exit-gate bypass = the walk-back shape.** A `ShortCircuit(R)` before-exit
   carries the RETURN type; `guard_before_template_exit_kinds`
   (`pseudo_tuple.rs:1201`) has no arm for it. All four C3 gates (before-exit,
   after-return, `?`-exit, f-string-interior) must EXTEND — never bypass.
   Bypassing reopens the measured pointer-reinterpretation class the C3
   fixlets closed (four leaks, measured values in the C3 slice reports).
2. **No Any resurrection** one slice after deleting one (the discriminated
   args|R carrier must be typed).
3. **The E4 defection attractor:** "keep an untyped ctx field for @remote's
   convenience" — refused on sight.
4. **JIT non-vacuity:** weave unit tests only assert `mir_data.is_some()` and
   CANNOT catch a demotion — the CLI cell must. Use scalar/typed-object `R`
   for the native cell; a `Result`-typed `R` hooked fn is pinned
   named-expected-fallback citing the §5.16 EnumPayload identity string (that
   payload-bind deopt is pre-existing, universal to all Result code, tracked
   for v0.4 — NOT an E4 deliverable; scope-fenced).

## Operating rules (proven E1→C3→E4; do not relearn)

- Supervisor-only build lane:
  `systemd-run --user --wait --collect --pipe -p MemorySwapMax=0
  -p MemoryMax=24G -p TasksMax=512 --setenv=PATH="$PATH" --setenv=HOME="$HOME"
  --working-directory=<worktree> direnv exec /home/dev/dev/shape-lang
  env CARGO_BUILD_JOBS=2 <cargo …>`. Logs contain NULs: always `grep -a`.
  shape-test suites run `--test-threads=1`.
- One writer per worktree; agents never build outside the lane; workflow
  agents die if they background a build (foreground only).
- Gates judge by **FAILED-NAME SETS** vs the S0 snapshot, never raw counts.
- Append-only after any gated hash; fresh-context capstone for deletions;
  review-mandatory slices get 3 fresh lenses with a fix loop.
- **`just check-clean` does NOT run tests** — a stage gate must include the
  crate whose tests the stage touched (S1 shipped a hard-RED negative pin that
  only a cross-check caught; see the S1 report).
- Ticket filing convention: symbol + test-name + commit anchors (never bare
  line numbers), in-code follow-up markers as primary anchors,
  acceptance-as-tripwire, cite standing USER RULINGs, and **re-sync the next
  program's charter ticket at every program close** (`memory:
  ticket-filing-convention`).

## Open items beyond E4

- **#68** @remote dark window (E4 closes it) · **#69** capture-provenance drop
  (2 ignored pins) · **#70** JIT aggregate proof gap (b) · **#71** template
  serialization × snapshot · **#72** module consts in config position ·
  **#74** foreign comptime handlers (user ruled: they SHOULD run; two-fold
  exploration, E4 ships only the interim rejection).
- **#63 needs USER RE-RATIFICATION** — heterogeneous tuple values would
  reverse the standing 2026-06-17 homogeneous-bracket ruling.
- **#61** E1-D8 residual (E5's `.source` deletion blocked on B4/B5) · **#59**
  D6 monomorphization re-arm · **#60** C092x coded diagnostics · **#64/#65/#66/#67**
  (mono_key injectivity, array kind-leak, mini-VM gaps, fn-type rendering).
- **shape-web branch `adr009-c3-annotations` @ `211fcc3`** (the C3 annotations
  book page) still needs its merge to shape-web main.
- **G3 comptime-local template bodies**: narrowing ACCEPTED as a v1 boundary
  (loud named rejection; the lift rides E-track staging).
