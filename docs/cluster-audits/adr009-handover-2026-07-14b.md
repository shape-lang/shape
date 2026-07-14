# ADR-009 Program Handover — 2026-07-14 (session 2)

Supersedes `adr009-handover-2026-07-14.md` for program state. That doc's §2 (landed
architecture), §4 (how the work is run) and §5 (concurrent-wave hygiene) are still
accurate and are NOT repeated here — read them, then read this.

Two branches are handed over **unmerged and partially verified**. §3 says exactly what
was verified and what was not. Do not treat either as done.

## 1. Board state — corrected

**13 of 22 tickets merged.** `main` @ `a8d60cab` (pushed).

> **GitHub is WRONG about one ticket.** Issue **#14 (C3, CheckedTemplate) shows CLOSED
> (COMPLETED, 2026-07-13) with zero comments and no close report.** It is a **FALSE
> CLOSE**: `rg CheckedTemplate` returns six *docs* and **zero lines of Rust**. The only
> "C3" commits in history (`ecbb9fa6`, `10ea3560`) are optional-chaining, an unrelated
> book feature that shares the label. **C3 is NOT implemented. Reopen #14.**
> (Not reopened in this session: a `gh` write was blocked by a permission classifier and
> the user was asked rather than overridden.)

Remaining chain: **C1 → C2 → {C3, E1, E2} → E4 → E5 → E6 → F1** (9 tickets incl. C1).

| Ticket | State |
|---|---|
| **C1 #12** | slices 0–3 committed on `adr009/c1-rework`, **UNMERGED, review not run**; slice 4 outstanding |
| C2 #13 | blocked on C1 |
| **C3 #14** | **falsely closed — reopen.** Blocked on C2 |
| E1 #17 | blocked on C2 (B6 ✅) — deletes the JSON directive protocol (U01/U06) |
| E2 #18 | blocked on C2 (D1 ✅) — deletes `ItemFragment` + source reparsing (U03/U07) |
| E4 #20 | blocked on C3 (B6 ✅) — deletes `Any` hook shapes (U13) |
| E5 #21 | blocked on E1 — deletes legacy reflection: `type_info`, string-keyed rewriting (U02/U04/U05) |
| E6 #22 | blocked on E1+E2+E4+E5 — stdlib migration + `string_lit` deletion (U08/U14) |
| F1 #23 | blocked on E6 — book/example enablement over the full truth-gate universe |

**Five of the eight post-C1 tickets are primarily DELETIONS.** That is where this program
will defect. CLAUDE.md §Forbidden Patterns is the binding doc; the ValueWord/W-series is
the worked example of a deletion downgraded to a "shim" and becoming permanent.

**E2 is more load-bearing than its slot suggests.** Source reparsing — generated AST
parsed from standalone snippets at byte offset 0 / `Span::DUMMY` — is the *root cause* of
the span-collision class that got C1 rejected. E2 deletes it. Consider whether E2 should
move earlier; it removes a bug substrate, not just legacy code. **Not analysed against the
tree this session** — treat as an open question, not a finding.

## 2. USER RULINGS — binding, do not re-litigate

**The C1 carrier was ALREADY RULED and the first implementer deviated from it undisclosed.**
The posted ruling on #12 reads verbatim: *"GO on Option B (closure-literal capture clause)
… the new optional FunctionExpr capture field is the canonical carrier C2's CheckedBody
staging will populate underneath — one carrier, two producers."* The rejected branch's
close report claiming "no FunctionExpr AST field" was the deviation — that is rework
finding 6. **Do not reopen the carrier question.**

**RULING 1 (2026-07-14) — `move` never lies.** `move x` on a **module-level binding** is
`[C0906] module-level binding 'x' cannot be moved into a closure; module bindings live for
the program and admit no move`. **There is no `Move -> Shared` arm anywhere.** This
overrides what capture inference does today (`closures.rs:~3594`). `Move` lowers ONLY:
local `let` → `Immutable`; local `let mut` → `OwnedMutable`.

**RULING 2 (2026-07-14) — a fourth mode, `share`.** `CaptureMode = { Move, Share,
SharedBorrow, ExclusiveBorrow }`. `share x` is the ONLY way to capture a shared-ownership
binding (module binding / `var` / SharedCell) and lowers to `CaptureKind::Shared`; `share`
on a plain local is rejected `[C0908]`. `&`/`&mut` stay a total named
`ReferenceEscapeIntoClosure` rejection `[C0902]`.

**The invariant both rulings buy: DECLARED WORD == EMITTED CaptureKind, ALWAYS.** No mode's
declared name differs from what is emitted. If a `lowered != declared` field ever appears
"to surface a gap", the model has regressed.

## 3. The two handed-over branches

### 3a. `adr009/c1-rework` — worktree `../shape-adr009-a3` (base `a8d60cab`)

```
dfc188fc  slice 3 — the declared capture clause: the declaration DRIVES emission
02004cac  slice 2 — node-borne generated-code provenance: the capture gate is now total
0faac4d1  slice 1 — fuse the two capture vectors into ONE selector
e5e4a6d8  slice 0 — JIT-nativity preflight
```

Carrier: `captures: Option<CaptureClause>` on `Expr::FunctionExpr` (`shape-ast/src/ast/captures.rs`).
Spelling: `;`-separated in-pipe clause — `|x: int; move hits|`. Generated-code-only surface.
One selector: `compiler/comptime_builtins/capture_plan.rs` (both `mutable_flags` and
`capture_kinds` deleted from `closures.rs`).

**VERIFIED THIS SESSION (by me, independently, not by the implementing agent):**
- `cargo check --workspace --all-targets` — **green**.
- `cargo test -p shape-vm --lib capture_plan` — **25/25 pass**.
- **THE MUTATION CHECK.** Stripped `; move hits` from the `FLAGSHIP_DECLARED` fixture and
  re-ran: `flagship_declared_move_over_read_only_let_mut_emits_owned_mutable` **FAILS**.
  The declaration genuinely drives emission. *This is the check C1's first attempt could
  not survive* — there, `capture(x)` and `capture(&x)` compiled byte-identically. Reverted.
- **THE SENTINEL IS REAL.** Planted a second `CaptureKind::` producer in
  `compiler/expressions/closures.rs`; `scripts/check-no-dynamic.sh` **FAILED** with
  `FAIL ADR-009 C1 K1: a SECOND CaptureKind producer exists.` Reverted. This gate is what
  makes R2 a compile error rather than a code-review norm — the "declared mode discarded,
  inference still authoritative" defection has now happened twice in this repo (ValueWord,
  then C1) and a norm did not stop it.

**NOT VERIFIED — DO NOT ASSUME (the lane was stopped early, on purpose):**
- The **full standing gate battery was NOT run** (shape-test `comptime` /
  `annotations_comptime` / `lsp`; `shape-ast --lib`; `shape-runtime --lib comptime`;
  `shape-lsp --lib`). Only the workspace check + `capture_plan` units were run.
- **The 3-lens adversarial review (defection / soundness / spec) was NOT run.** Neither was
  the refuter pass or the fix round. C1's first attempt **passed its own gates and was
  still rejected 19 findings / 3 critical** — green tests are not evidence here.
- **Slice 2 widens the capture gate to fire where it never fired** (monomorphized generated
  bodies, `ReplaceBody` expansions, nested closures). That is a true-positive widening.
  **Stdlib/annotation fallout has not been measured.** Run `annotations_comptime` and
  `comptime` first.
- **Slice 4 is NOT DONE**: the JIT-nativity proof, LSP hover/references, the
  `docs/design/typed-comptime.md` status row, and the `docs/defections.md` entries
  (incl. recording Rulings 1/2 and what was rejected). §7 DoD is therefore **unmet**.

### 3b. `jit/closure-capture-lowering` — worktree `../shape-adr009-d1` (base `a8d60cab`)

```
4a8b79c8  thread the real NativeKind into jit_alloc_shared_cell (ADR-006 §2.7.8/Q10)
11c7f0a1  docs: correct the false-green "closure specialization 100% LANDED" status
aae72191  capturing closures reach native JIT — delete the capture double-count in the signature
```

Not an ADR-009 ticket, but a **hard prerequisite for C1's slice 4**: R5 demands a JIT proof
that cannot pass under a `[jit-fallback]` deopt, and before this branch **no capturing
closure of any kind reached native JIT**.

Root causes fixed: (1) a **capture double-count in the closure signature** → Cranelift
verifier arity mismatch (`got 2, expected 3`) in the *enclosing* function → whole-program
deopt; (2) `todo!()` at `shape-jit/src/ffi/object/closure.rs:519` → **process abort (134)**
on a `var`/`Shared` capture.

**VERIFIED THIS SESSION (I rebuilt the binary and ran the fixtures myself):**

| capture | VM | JIT | `[jit-fallback]` |
|---|---|---|---|
| Immutable scalar (`let x = 41`) | 0 / `42` | 0 / `42` | **0** |
| Immutable heap (`let s = "shape"`) | 0 / `shape` | 0 / `shape` | **0** |
| Shared (`var n`, mutated) | 0 / `42` | 0 / `42` | **0** ← was an ABORT |
| generated `extend` method | 0 / `42` | 0 / `42` | **0** |
| **module-binding capture** | 0 / `42` | 0 / `42` | **1** ← still deopts |

**The module-binding cell is blocked by an ORTHOGONAL, PRE-EXISTING, NAMED surface**, not
by closures: `W39 F1 module-binding function-body SURFACE (ADR-006 §2.7.14)` — module-binding
access inside a JIT'd function body is simply unimplemented. **This matters for C1**: Ruling
2's `share` mode targets module bindings, so `share`-over-module-binding cannot get a
zero-fallback JIT proof until W39 F1 lands. `share` over a local `var` **can** (verified
above). Decide at slice 4: prove `share` via `var` and name the module-binding cell as
blocked-on-W39-F1, or pull W39 F1 in.

**NOT VERIFIED:** the prove-matrix phase never ran, so **there is no committed JIT
regression test** for any of the above — I measured it by hand. The review/refute passes did
not run. `tests/smokes-fallback/f3-preflight-closure-capture.shape` pins the OLD behaviour
(asserts `count_fallback_lines == 1`); check whether the branch rebaselined it to 0 — that
is a **true-positive rebaseline**, not a weakening.

## 4. Traps that will waste your time if you don't know them

- **A top-level `comptime { }` block causes a SILENT whole-program deopt**
  (`program_has_top_level_comptime`, `compiler/mod.rs:~1933`) that emits **NO**
  `[jit-fallback]` line. Any nativity test asserting `count_fallback_lines == 0` therefore
  **passes VACUOUSLY** on such a fixture while the program runs on the interpreter. JIT
  fixtures must contain no top-level comptime block. Annotation-generated `extend` does NOT
  trip it (`Item::Extend`, not `Item::Impl`) and IS JIT-native — verified.
- **`.map()` with a capturing closure is a silent VM≠JIT divergence**: JIT exits **1**
  ("typed-array `.map()` with a closure argument is unimplemented") while VM exits 0 and
  prints — with **zero fallback lines**. Invisible to every nativity harness. Not needed for
  C1 (its fixtures call closures directly). **Unfiled** — a `gh issue create` was blocked by
  a permission classifier. File it.
- Two status docs in this repo were **false-green** and had to be corrected this session
  (a "closure specialization 100% LANDED" note; an April memory claiming JIT closures were
  fixed). Distrust status claims; measure.
- **`main` carried ~130 modified + ~137 untracked files of a concurrent wave's uncommitted
  work.** That is now **committed and pushed** (`d652f664` source, `a8d60cab` docs), so the
  stash-cycle merge dance in the previous handover's §5 **no longer applies**. `main` is
  clean. `.gitignore` now excludes the `node_modules` symlink, `.agents/`, `.claude/skills/`,
  `skills-lock.json`, `__pycache__/` (`e5c5ec71`).

## 5. What to do next, in order

1. **Run the full standing gate battery on `adr009/c1-rework`.** Slice 2's gate widening has
   unmeasured fallout. If it exceeds the ticket, surface it — do not silence the gate.
2. **Run the 3-lens adversarial review + refuter on `adr009/c1-rework`** before merging. The
   first attempt passed its own gates. The single highest-value lens is: *take every passing
   accept test, delete the capture clause, re-run — if it still passes, R2 is a lie.* (I did
   this for the flagship test; it fails correctly. The other accept tests were not checked.)
3. **Merge `jit/closure-capture-lowering` first** (C1 slice 4 depends on it), with a
   committed JIT regression matrix — it currently has none.
4. **C1 slice 4**: JIT proof (`count_fallback_lines == 0`, trait-free fixtures, no top-level
   comptime), LSP hover, status row, `docs/defections.md` (record Rulings 1/2 and the
   accept-and-surface option that was rejected — rework finding 6 was that the previous
   attempt's defections.md never recorded a deviation from a posted ruling).
5. **Reopen #14 (C3).**
6. Then C2 → {C3, E1, E2} → E4 → E5 → E6 → F1.

## 6. Orchestration notes

Machinery from the previous handover §4 still applies (pinned warm worktrees; all-Opus
workflow agents; single build lane, foreground, `timeout: 600000`, `systemd-run … MemoryMax=12G`;
never run the bulk shape-test suite in one command — known hang). Two additions:

- **The preflight slice earned its keep.** Slice 0 was a no-production-code slice whose only
  job was to *execute* an assumption (does annotation-generated `extend` JIT natively?)
  rather than reason about it. It found the JIT closure blocker before anyone wrote grammar.
  Cost: one agent. Without it, slice 4 would have "proved" JIT nativity on a fixture that
  quietly deopted — C1's rejected hole, second time around.
- **Don't over-orchestrate.** A handoff, a status question, or a doc is a *writing* task —
  the inputs are already in the issues, the spec and the previous handover. Fanning out
  agents to re-derive them is waste, and the user will (rightly) call it out.
