# Phase 2d Wave 1 Supervisor Prompt

**Audience:** Claude Code, fresh session, started in `/home/dev/dev/shape-lang/shape/`.
**Generated:** 2026-05-11.
**Predecessor session:** completed the Phase 2d planning + dispatch-readiness work (ADR-006 §2.7.24, inventory, playbook, verify-merge.sh, AGENTS.md updates). Branch parent `bulldozer-strictly-typed` at `45bd827` or later.

---

## Your role this session

You are the **Phase 2d Wave 1 supervisor**. Your job is to:

1. Dispatch two parallel sub-agents — **T1-host-tier-marshal-rebuild** and **W17-make-closure** — via the `Agent` tool, in a single message with two parallel tool calls.
2. Monitor their progress, verifying that each obeys the §0 shared discipline.
3. When each agent reports completion, run their close-gate (verify-merge.sh in their worktree, plus the playbook §0 checklist).
4. Merge each successful branch into `bulldozer-strictly-typed`.
5. Report status to the user at the cadence described below.

**You do NOT write SURFACE bodies yourself.** The sub-agents do. Your job is orchestration + verification + merge.

---

## First action — read these docs IN ORDER before anything else

1. **`docs/cluster-audits/phase-2d-handover.md`** — §0 in full (the rules).
2. **`CLAUDE.md`** — sections "Forbidden Patterns" + "Renames to refuse on sight" + "Single-discriminator discipline (ADR-005)" + "Value & memory model (ADR-006)" + "Mechanical enforcement" + "Phase 2d entry points". In their entirety.
3. **`docs/cluster-audits/phase-2d-playbook.md`** — §0 (shared discipline) in full, then §1 (T1 + W17-make-closure prompts).
4. **`docs/adr/006-value-and-memory-model.md`** §2.7.24 — skim. Full read only if a sub-agent surfaces an §2.7.24-related issue.
5. **`docs/cluster-audits/phase-2d-stub-inventory.md`** — §0 (audit corrections) + §6 (resolved decisions + HeapKind ordinal table).

**Do not skip this reading.** The discipline encoded in those docs is hard-won from W7→W16 sessions where individual agents (including supervisors) drifted. If you skip, you will drift.

After reading, post the user a one-line confirmation: *"Read all 5 mandatory docs; Wave 1 dispatch ready. Starting verification of branch state."*

---

## Verify branch state

```bash
git rev-parse HEAD
git branch --show-current
bash scripts/verify-merge.sh --fast
```

Expected:
- HEAD = `45bd827` or later
- Branch = `bulldozer-strictly-typed`
- `verify-merge.sh --fast` exits 0

If any of these fail, **stop and ask the user**.

---

## Dispatch protocol

For each Wave 1 sub-agent:

### Option A — `Agent` tool with `isolation: "worktree"` (recommended)

```text
Agent({
  description: "Phase 2d Wave 1: <sub-cluster slug>",
  subagent_type: "general-purpose",
  isolation: "worktree",
  prompt: "<the full prompt from §1 below>",
})
```

The `isolation: "worktree"` flag creates a temporary git worktree branched from the current HEAD. After the agent finishes, the returned result includes the worktree path + branch.

**Dispatch both agents in a single message** with two parallel `Agent` tool calls so they run concurrently. The user expects roughly parallel completion (~6-8h elapsed if they actually were elapsed time, but since these are sub-agent invocations, expect ~minutes to a few hours of wall time depending on how complex the cargo cycles get).

### Option B — Manual worktree creation (if you need branch names to match playbook)

If you want branch names matching the playbook convention (`bulldozer-strictly-typed-t1-host-marshal` / `bulldozer-strictly-typed-w17-make-closure`), create the worktrees yourself first:

```bash
git worktree add ../shape-t1-host-marshal -b bulldozer-strictly-typed-t1-host-marshal
git worktree add ../shape-w17-make-closure -b bulldozer-strictly-typed-w17-make-closure
```

Then dispatch each agent with its prompt **and** an instruction to `cd` into its assigned worktree as the very first Bash action. (Without `isolation: "worktree"`, the agent inherits your CWD — it must cd before any edits.)

Pick whichever works; Option A is simpler.

---

## Sub-agent prompts (paste verbatim)

### Agent 1 — `T1-host-tier-marshal-rebuild`

```text
You are a Phase 2d Wave 1 sub-agent. Your sub-cluster is
**T1-host-tier-marshal-rebuild**.

Your job is to rebuild the test-tier eval/marshal API helpers
(`eval()`, `eval_int()`, `eval_float()`, `eval_string()`, `eval_bool()`
and the `Constant::Value(...)` carrier they depended on) against the
post-strict-typing KindedSlot API. This unblocks ~290 test bodies that
are currently `todo!("phase-2c — see ADR-006 §2.7.4 (host-tier
eval/marshal API rebuild — deleted ValueWord/Constant::Value(ValueWord)
carrier)")` across 16 test files in `crates/shape-vm/src/executor/tests/`.

YOU MUST read these docs in order before touching any code:

1. /home/dev/dev/shape-lang/shape/docs/cluster-audits/phase-2d-handover.md §0
2. /home/dev/dev/shape-lang/shape/CLAUDE.md sections "Forbidden Patterns"
   + "Renames to refuse on sight" + "Mechanical enforcement"
3. /home/dev/dev/shape-lang/shape/docs/cluster-audits/phase-2d-playbook.md
   §0 (shared discipline) and §1 (your T1 section in full)
4. /home/dev/dev/shape-lang/shape/docs/adr/006-value-and-memory-model.md
   §2.7.4 + §2.7.5 + §2.7.5.1

Your territory:
- Search with `rg 'pub fn eval' crates/shape-vm/src/` to find current
  test-helper definitions
- Search with `rg 'Constant::Value' crates/shape-vm/src/` for the
  carrier's current shape and call sites
- Migrate helpers + carrier to return/accept KindedSlot, not ValueWord

Smoke target: at least one currently-`todo!()` test body (your choice —
pick from `crates/shape-vm/src/executor/tests/typed_array_ops.rs`)
re-filled with the new helper shape and passing.

Forbidden in this sub-cluster (refuse on sight):
- Re-introducing `Constant::Value(ValueWord)` under any rename
- A generic `eval_to_value()` returning a polymorphic carrier
- Any of the CLAUDE.md "Renames to refuse on sight" framings

Close gate (per playbook §0 checklist):
- `cargo check --workspace --lib` exits 0 (verify by EXIT CODE, never
  `cargo check ... | grep -c '^error\['` — that pattern doesn't reflect
  cargo's exit status)
- `cargo test -p shape-vm --lib` passes for tests in your territory
- `bash scripts/verify-merge.sh` exits 0
- `bash scripts/check-no-dynamic.sh` exits 0
- The one re-filled test body actually runs and passes

When you finish, commit your work with a clear message (NO
`Co-Authored-By: Claude` trailer; NO "blame pre-existing"). Then report
back with:
1. Branch name + close commit hash
2. Output of `bash scripts/verify-merge.sh` (last 20 lines)
3. The test body you re-filled (file:line + brief snippet)
4. A list of any decisions you had to make that weren't in the playbook
5. Any sites you discovered that you couldn't fix (with surface-and-stop
   shape: `NotImplemented(SURFACE: <reason>) — ADR-006 §<X>`)

If you hit an architectural gap that requires an ADR amendment or
supervisor decision, STOP and surface to the supervisor with the
structured error shape from playbook §0 "Surface-and-stop discipline".
Do not fabricate a fallback to make the compiler happy.
```

### Agent 2 — `W17-make-closure`

```text
You are a Phase 2d Wave 1 sub-agent. Your sub-cluster is
**W17-make-closure**.

Your job is to fill the `op_make_closure` SURFACE at
`crates/shape-vm/src/executor/control_flow/mod.rs` (~line 447, currently
returning `NotImplemented(PHASE_2C_CALL_REBUILD_SURFACE)`). The §2.7.8
`ClosureCell` cell layout (in `shape-value/v2/closure_raw.rs`) is the
consumer-side foundation and is already built. Your job is the *make*
path: capture-cell construction at closure-creation time, threading
per-capture `NativeKind` from the §2.7.7 stack parallel-kind track into
the new `ClosureCell.kinds` parallel track.

YOU MUST read these docs in order before touching any code:

1. /home/dev/dev/shape-lang/shape/docs/cluster-audits/phase-2d-handover.md §0
2. /home/dev/dev/shape-lang/shape/CLAUDE.md sections "Forbidden Patterns"
   + "Renames to refuse on sight" (especially the §2.7.11 value-call ABI
   defection-attractor family) + "Mechanical enforcement"
3. /home/dev/dev/shape-lang/shape/docs/cluster-audits/phase-2d-playbook.md
   §0 (shared discipline) and §1 (your W17-make-closure section in full)
4. /home/dev/dev/shape-lang/shape/docs/adr/006-value-and-memory-model.md
   §2.7.8 (Q10 cell-storage kind-awareness, in full) + §2.7.11 (Q12
   value-call ABI — the consumer side, already landed)
5. `git show b7c9770` (W7-cv-method `execute_closure` refinement)
6. `crates/shape-vm/src/executor/call_convention.rs::call_value_immediate_nb`
   body — already kinded; your op_make_closure feeds this

Your territory:
- Primary: `crates/shape-vm/src/executor/control_flow/mod.rs::op_make_closure`
- Read-only consumer: `crates/shape-value/src/v2/closure_raw.rs`
  (ClosureCell, alloc_typed_closure, write_capture_kinded per §2.7.8 Q10)
- Search compiler-side emit sites with
  `rg 'OpCode::MakeClosure' crates/shape-vm/src/compiler/`

Smoke targets (both must run on the VM and produce the expected output):

```shape
let f = |x| x + 1
print(f(5))                                          # 6

let xs = [1, 2, 3]
let doubled = xs.map(|x| x * 2)
print(doubled)                                       # [2, 4, 6]
```

Forbidden in this sub-cluster (refuse on sight):
- Bool-default fallback for capture kinds (§2.7.8 #4 — surface-and-stop
  if a kind-source gap appears)
- Any "frame-setup probe" / "callee-kind helper" / "capture-injection
  adapter" framing (CLAUDE.md "Renames to refuse on sight")
- Restoring `_upvalue_bits: Vec<u64>` or any kind-blind capture vector
- Any of the deleted value-call ABI shape names from CLAUDE.md §2.7.11
  forbidden list

Close gate (per playbook §0 checklist):
- `cargo check --workspace --lib` exits 0 (verify by EXIT CODE, not grep)
- `cargo test -p shape-vm --lib` passes
- Both smoke targets above run on VM and produce expected output
- `bash scripts/verify-merge.sh` exits 0
- `bash scripts/check-no-dynamic.sh` exits 0

When you finish, commit your work with a clear message (NO
`Co-Authored-By: Claude` trailer; NO "blame pre-existing"). Then report
back with:
1. Branch name + close commit hash
2. Output of `bash scripts/verify-merge.sh` (last 20 lines)
3. Smoke-target results (both programs, actual output)
4. A list of any decisions you had to make that weren't in the playbook
5. Any sites you discovered that you couldn't fix (with surface-and-stop
   shape)
6. Whether anything in `closure_raw.rs` needed adjustment beyond
   read-only consumption (this would be a signal of an §2.7.8 amendment)

If you hit an architectural gap, STOP and surface to the supervisor.
Do not fabricate a fallback.
```

---

## Reporting cadence to the user

The user wants concise, structured updates at meaningful gates. Not constant chatter.

### Required reports

| When | Content | Format |
|---|---|---|
| After reading 5 mandatory docs | "Read all 5 mandatory docs; Wave 1 dispatch ready. Starting branch state verification." | 1 line |
| Pre-dispatch | Branch state (HEAD, verify-merge --fast result). If unhealthy, ASK the user before proceeding. | 3-5 lines |
| At dispatch | "Dispatched T1 ({branch}) and W17-make-closure ({branch}) in parallel. Awaiting completion." | 1 line |
| Each agent completion | Per-agent summary: close commit hash, verify-merge result, smoke results, any decisions made beyond the playbook, any surfaced gaps. | 10-20 lines per agent |
| At merge time | "Merging {branch} into bulldozer-strictly-typed. verify-merge.sh: {pass/fail}." If fail, ASK the user. | 2-3 lines per merge |
| At Wave 1 close | Full summary table (see "Wave 1 close report" below) + recommendation for Wave 2 dispatch. | ~30 lines |
| On surface-and-stop (any agent) | Full surfaced gap as the agent reported it + your analysis of which ADR § applies + recommended path forward + ASK the user. | ~20 lines |
| On any forbidden-pattern detection | STOP IMMEDIATELY. Quote the forbidden pattern + where it appeared. ASK the user. | as needed |

### Do NOT do

- Constant heartbeat updates ("agent still running" every 5 min). The Agent tool's completion notification is the heartbeat.
- Speculative status ("I think the agent will finish soon"). Wait for the actual completion.
- Inline reproduction of large output. Quote the relevant lines + cite file:line for the rest.

---

## Stop-and-ask triggers

Pause everything and ask the user when:

1. **Any forbidden-pattern detection** (refuse on sight; CLAUDE.md "Forbidden Patterns" + "Renames to refuse on sight" + §2.7.24 Q25.E new entries). Do NOT proceed even if it looks innocuous.
2. **HeapKind ordinal collision at merge.** Even with the bump-on-collision rule, the supervisor must approve renumbering.
3. **`verify-merge.sh` failure that isn't trivially fixable** (e.g., 4-table lockstep miss requires the agent to add arms — that's a re-dispatch, not a supervisor action). If the failure indicates an architectural gap, surface.
4. **Test count regression** vs the pre-dispatch baseline. Even one previously-passing test now failing is grounds to stop.
5. **An agent discovers the work requires a new ADR amendment** (e.g., §2.7.25 for concurrency primitives was anticipated, but W17-make-closure shouldn't need a new amendment — if it claims to, scrutinize).
6. **An agent takes longer than ~2h elapsed wall time** without progress — likely stuck.
7. **Any agent's report mentions** "decoder bridge", "FFI-boundary bridge", "boundary translation", "host-boundary normalization", "tag-decode helper", "MethodFn translator", "polymorphic fallback", "catch-all element buffer", or any of the rename-refusal phrases. These are not engineering — they are walk-back attempts. Refuse and surface.
8. **`branch_state_unhealthy`** — pre-dispatch verification fails. Don't dispatch on a broken base.

For each stop-and-ask, structure the user message as:

```
SURFACE: <one-line summary>

What happened:
  <agent report excerpt or your observation>

ADR § / playbook reference:
  <citation>

Options I see:
  A) <option> — <implication>
  B) <option> — <implication>

Which path?
```

---

## Wave 1 close report (post when both agents merge)

```
## Phase 2d Wave 1 — close report

### T1-host-tier-marshal-rebuild
- Branch: <name>
- Close commit: <hash>
- verify-merge.sh: PASS
- check-no-dynamic.sh: PASS
- Smoke: <re-filled test body details>
- New test passes: <count>
- Sites resolved: <details>
- Decisions made beyond playbook: <list or "none">
- Merged at: <hash>

### W17-make-closure
- Branch: <name>
- Close commit: <hash>
- verify-merge.sh: PASS
- check-no-dynamic.sh: PASS
- Smoke targets:
  - `let f = |x| x + 1; print(f(5))` → 6 (PASS)
  - `[1,2,3].map(|x| x * 2)` → [2,4,6] (PASS)
- Sites resolved: 1 SURFACE at control_flow/mod.rs:~447
- Decisions made beyond playbook: <list or "none">
- Merged at: <hash>

### Post-Wave-1 state
- HEAD: <hash>
- `cargo check --workspace --lib`: EXIT 0
- check-no-dynamic baseline: <delta>
- Inventory site count delta: <pre> → <post>
- AGENTS.md: T1 row idle (close <hash>), W17-make-closure row idle (close <hash>)

### Recommendation for Wave 2

Wave 2 sub-clusters now ready to dispatch in parallel (up to 8 agents):
- W17-array-typed-receiver (3-5h, low risk, independent)
- W17-iterator-tableview (4-6h, medium risk, independent)
- W17-references-mutation (4-6h, gates W17-typed-object-mutation)
- W17-builtin-coercions (3-5h, low risk, independent)
- W17-foreign-ffi (6-8h, medium risk, independent)
- W17-typed-module-exports (6-8h, medium risk, independent)
- W17-array-closure-callback (4-6h, blocked by W17-make-closure — NOW UNBLOCKED)
- W17-native-scalar-carrier (2-4h, low risk, verify-scope first)
- W17-method-bodies-misc (2-3h, low risk, absorbs deque-ctor cleanup)
- C1-temporal-lowering (4-6h, blocked by T1 — NOW UNBLOCKED)

Suggested 4-6 parallel dispatch: <your picks>.

Awaiting user greenlight for Wave 2.
```

---

## End-of-session continuity

If you do not complete Wave 1 in this session (agents still running, or merge pending), write a brief status note to:

```
docs/cluster-audits/phase-2d-wave-1-status.md
```

Format:

```markdown
# Phase 2d Wave 1 — in-flight status

Session: <date>
Last action: <description>
Pending: <list>
Resume action: <what the next supervisor session should do first>

Agent T1 state: <dispatched / completed / pending merge / surface-and-stopped>
Agent W17-make-closure state: <dispatched / completed / pending merge / surface-and-stopped>
```

Then post that location + a one-line summary to the user.

---

## Final reminders

- You are operating under all the discipline of playbook §0. Every refuse-on-sight rule applies to YOUR reasoning too, not just the sub-agents'.
- `cargo check ... | grep -c '^error\['` does not reflect cargo's exit code. Use `cargo check ... && echo CLEAN || echo FAILED` or `bash scripts/verify-merge.sh`.
- No `Co-Authored-By: Claude` in any commits, supervisor or sub-agent.
- If you are about to write a doc, edit, or commit that uses a CLAUDE.md "Renames to refuse on sight" phrase, stop. Even if it would make a sentence easier to write.
- You ARE allowed to take risky-looking actions (worktree creation, merging, agent dispatch) as part of Wave 1 dispatch — that's your assigned role. You are NOT allowed to take destructive actions without user authorization (force-push, branch deletion, hard-reset).

Start by reading the 5 mandatory docs. Post the 1-line confirmation. Then verify branch state.
