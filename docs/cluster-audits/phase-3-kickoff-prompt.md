# Phase 3 Cluster-0 — Supervisor Kickoff Prompt

**Audience:** Claude Code, fresh session, started in
`/home/dev/dev/shape-lang/shape/`.
**Predecessor:** `phase-2d-close` tag at `e22bffd2`. VM-path strict-
typing migration complete; JIT path structurally broken pre-Phase-2d.
**Generated:** 2026-05-12.

## Your role this session

Phase 3 cluster-0 supervisor. Dispatch and verify the JIT rebuild
sub-clusters. The pattern is identical to Phase 2d — agent waves,
verify-merge.sh gate, AGENTS.md roster, no Co-Authored-By trailers,
own all code quality.

## First action — read these docs IN ORDER

1. `docs/cluster-audits/phase-2d-close-summary.md` — what just shipped,
   what didn't, why.
2. `docs/cluster-audits/phase-2d-handover.md` §0 — discipline rules
   carry forward unchanged.
3. `CLAUDE.md` "Forbidden Patterns" + "Renames to refuse on sight" +
   "Mechanical enforcement" + "Phase 2d entry points".
4. `docs/adr/006-value-and-memory-model.md` §2.7.14 (Q15 JitArray
   deletion + kinded TypedArray<T> reintroduction). This is the binding
   spec for cluster-0.
5. `docs/cluster-audits/phase-2d-hardening.md` item (i) — the JIT
   blocker described from the Phase 2d Item 3 verification.

Post a 1-line confirmation: "Read all 5 mandatory docs; Phase 3
cluster-0 dispatch ready."

## Branch state verification

    git rev-parse HEAD                       # at or descended from phase-2d-close
    git describe --tags --abbrev=0           # phase-2d-close
    just check-clean                         # exit 0
    bash scripts/verify-merge.sh --fast      # exit 0, 11/11

If any fail, stop and ask the user.

## Cluster-0 sub-cluster sequencing

Cluster-0 has 4 sub-clusters. Suggested dispatch:

**Round 1 (parallel, ~3 agents):**
- W11-jit-new-array (the unblocker — single agent, biggest piece)
- W17-jit-legacy-ordinal-disambiguation (small, ord renumbering work)
- shape-jit lib test runner (small, test-infra conversion)

**Round 2 (after W11-jit-new-array lands):**
- W11-jit-carrier-conversion (depends on the jit_new_array FFI shape
  landing first; routes per-arm encoding through the new carrier
  surface)

After all 4 close: `--mode jit` works end-to-end for the standard
program surface. Cluster-0 closes. Move to cluster-1 (hardening) or
cluster-2 (Wave-3 surfaces) — supervisor's call on order, user
direction welcome.

## W11-jit-new-array — first dispatch (biggest piece)

Paste verbatim into the Agent tool with subagent_type="general-purpose":

    You are a Phase 3 cluster-0 sub-agent. Your sub-cluster is
    **W11-jit-new-array**.

    Your job is to implement the kinded `Arc<TypedArrayData>` FFI surface
    for the JIT path per ADR-006 §2.7.14 Q15. This unblocks
    `shape run --mode jit` for every program — currently EVERY JIT
    compilation aborts because `jit_new_array` is a stub.

    YOU MUST read these docs in order before touching any code:

    1. /home/dev/dev/shape-lang/shape/docs/cluster-audits/phase-2d-handover.md §0
    2. /home/dev/dev/shape-lang/shape/docs/cluster-audits/phase-2d-close-summary.md
    3. /home/dev/dev/shape-lang/shape/CLAUDE.md sections "Forbidden Patterns"
       + "Renames to refuse on sight"
    4. /home/dev/dev/shape-lang/shape/docs/adr/006-value-and-memory-model.md §2.7.14 (Q15 in full)
    5. crates/shape-jit/src/ffi_symbols/array_symbols.rs (current stub)
    6. crates/shape-jit/src/compiler/ffi_builder.rs:73 (current unconditional ref)

    Your territory:
    - crates/shape-jit/src/ffi_symbols/array_symbols.rs (implement stubs)
    - crates/shape-jit/src/compiler/ffi_builder.rs (FFIFuncRefs slots)
    - crates/shape-jit/src/ffi/array_ffi.rs (if it needs jit-side
      TypedArray<T> definitions, add them per §2.7.14 Q15)
    - crates/shape-jit/src/mir_compiler/ (any kind-keyed switches needing
      arms for new HeapKinds 24-33)

    Smoke targets (must pass --mode jit at both Tier 1 baseline @ 100 calls
    and Tier 2 optimizing @ 10k):

      let mut sum = 0
      for i in 0..100 { sum += i }
      print(sum)                             # 4950

      let xs = [1, 2, 3, 4, 5]
      let doubled = xs.map(|x| x * 2)
      print(doubled.sum())                   # 30

      trait T { fn name(&self) -> String }
      impl T for X { fn name(&self) -> String { "x" } }
      let t: dyn T = box(X{})
      print(t.name())                        # x

      let mut s = HashSet()
      s.add("a")
      s.add("b")
      print(s.size())                        # 2

    Each smoke target must produce IDENTICAL output under --mode vm and
    --mode jit. Divergence = bug.

    Forbidden in this sub-cluster (refuse on sight):
    - Resurrecting ValueWord under any rename (CLAUDE.md
      "Forbidden Patterns" #1).
    - Bool-default fallback for unknown kind at JIT FFI boundary —
      surface-and-stop with ADR-006 §2.7.14 / §2.7.5 cite instead.
    - "Tag-decode bridge at JIT boundary" / "FFI-boundary bridge" /
      any defection-attractor descriptor (CLAUDE.md "Renames to refuse
      on sight").
    - Re-introducing JitArray as a parallel discriminator to
      TypedArrayData (per ADR-005 §1 single-discriminator).

    Close gate:
- cargo check --workspace --lib + --tests exit 0 (verified by EXIT
      CODE, not grep)
    - cargo test -p shape-jit --lib passes for jit-related tests
      (deep-tests gated separately per CLAUDE.md known-constraint)
    - All 4 smoke targets produce identical --mode vm and --mode jit
      output
    - bash scripts/verify-merge.sh exit 0
    - bash scripts/check-no-dynamic.sh exit 0
    - AGENTS.md row updated to idle
    - ADR-006 §2.7.14 status updated (or amended if Q15 details need
      refinement based on implementation findings)

    When you finish, commit your work with a clear message (NO
    Co-Authored-By: Claude trailer; NO "blame pre-existing"). Then
    report back with:
    1. Branch name + close commit hash
    2. Output of bash scripts/verify-merge.sh (last 20 lines)
    3. Smoke-target results (all 4, both --mode vm and --mode jit, with
       actual output)
    4. A list of any decisions you had to make beyond the playbook /
       ADR §2.7.14 Q15
    5. Any sites you discovered that you couldn't fix (with surface-and-
       stop shape)
    6. Whether ADR-006 §2.7.14 needs amendment based on what you
       discovered

    If you hit an architectural gap that requires an ADR amendment or
    supervisor decision, STOP and surface to the supervisor with the
    structured error shape from playbook §0 "Surface-and-stop discipline".
    Do not fabricate a fallback to make the compiler happy.

## W17-jit-legacy-ordinal-disambiguation — dispatch in parallel with W11-jit-new-array

Same prompt structure. Territory: the 6 HK_* u16-prefix collisions
catalogued in Item 3's close report. Either bump legacy ords ≥256 or
migrate JIT-internal allocations to canonical-HeapKind ordinals. Close
gate same as W11-jit-new-array except no smoke target — the ords are
internal, verified by grep + verify-merge.sh.

## shape-jit lib test runner — dispatch in parallel with W11-jit-new-array

Territory: convert `#[should_panic(expected = "phase-2c")]` tests on
`extern "C" todo!()` SURFACEs to `#[ignore]` with §-cite. Close gate:
`cargo test -p shape-jit --lib` doesn't SIGABRT.

## Reporting cadence (same as Phase 2d Wave 1)

| When | Content | Format |
|---|---|---|
| After 5 mandatory docs | "Read all 5; cluster-0 ready" | 1 line |
| Pre-dispatch | Branch state confirm | 3-5 lines |
| Dispatch | "Dispatched W11-jit-new-array, W17-jit-legacy-ordinal-disambiguation, shape-jit-test-runner in parallel" | 1 line |
| Each agent close | Per-agent summary | 10-20 lines |
| Round-1 close | Compact summary + Round-2 readiness | 10 lines |
| Round-2 close | Cluster-0 close report | 30 lines |
| Surface-and-stop | Full surfaced gap + ASK user | as needed |

## Stop-and-ask triggers (carry forward from Phase 2d)

Same as `phase-2d-wave-1-supervisor-prompt.md`. Particularly:
- Any forbidden-pattern detection (CLAUDE.md lists carry forward
  with the Phase 2d additions).
- An agent claims they need to reintroduce ValueWord / JitArray /
  any deleted shape.
- An agent's report mentions "tag-decode bridge at JIT boundary"
  or any of the rename-refusal phrases.
- HeapKind ordinal collision at merge.

## End-of-session continuity

If cluster-0 doesn't fully close in this session, write status to
`docs/cluster-audits/phase-3-cluster-0-status.md` mirroring the
Phase 2d Wave 1 status pattern.

Next session resume reads that file first.
