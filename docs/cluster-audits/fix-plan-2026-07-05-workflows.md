# Fix Program for audit-2026-07-04-claimed-vs-real — Dynamic Workflow Plan

**Input:** `docs/cluster-audits/audit-2026-07-04-claimed-vs-real.md` (29 confirmed critical/high findings, 158 book-gate failures, ~10.5k LOC dead code, 6 split-brains).
**Goal:** fix everything fix-forward (implement, don't retract), then re-run the full book truth-gate green.
**Shape:** 16 workflows in 5 waves + a standing design workflow (WF-D). Findings are grouped by **root cause**, not symptom — e.g. one marshal fix retires ~9 audit rows.

## 0-pre. Priority spine (user ruling 2026-07-05)

The user ruled four verticals of **utmost importance**, all design-first:

1. **Resumability + distributed execution work for real** (WF-2B, WF-2C elevated to the program's core deliverables, not just audit rows).
2. **Comptime is excellent AND ergonomic** — the bar is not "un-broken", it is a polished introspection/derive/annotation story. WF-1B gains an ergonomics acceptance layer (see amendment below).
3. **Polyglot functions work with the modular extension system** (WF-2A) — extension loading/discovery stays modular; the rebuild must preserve the LanguageRuntimeVTable extension contract, not inline the runtimes.
4. **Polyglot × distributed compose** — a foreign-function-bearing program must survive per-function remote transfer and snapshot/resume. This is NEW design surface the audit never probed: foreign bodies inside content-addressed FunctionBlobs (what does the hash cover — source, extension id, extension version?), extensions as declared remote dependencies, foreign frames as suspension barriers, Ffi permission propagation over the wire. Covered by new **WF-2F** + the WF-D integration design doc.

**WF-D `priority-design` (runs immediately, parallel to Wave 0):** a design workflow producing five docs under `docs/design/` — ffi-rebuild, snapshot-resume, distributed-function-transfer, comptime-excellence, polyglot-distributed-integration — each drafted from code recon, adversarially reviewed (ADR/forbidden-pattern lens, feasibility lens, ergonomics lens), revised, and left for **user ratification** before the Wave-2 rebuild workflows implement against them. Open questions are consolidated for the user; nothing in Wave 2's design-sensitive lanes starts building until its doc is ratified.

**WF-1B amendment (comptime excellence):** close gate extends beyond the marshal/schema fixes — `target.fields` contract exactly as documented (`{name, type, annotations, optional}`), first-class diagnostics (`error()` preserves message + span, `warning()` surfaces, no internal jargon), `type_info()` functional, and at least two polished stdlib derive-style showcases (e.g. a schema-derive and an LLM-integration pattern per the CLAUDE.md claim that currently has zero instances) running green in the book gate. Ergonomics criteria come from the WF-D comptime doc.

**WF-2F `polyglot-distributed-integration` (new, Wave 2, after WF-2A+2B+2C):** implements the WF-D integration design: foreign-function blobs transfer + execute remotely (extension-dependency declaration, receiver-side extension resolution, hash coverage of foreign source), snapshot/resume of programs containing foreign functions (suspension barriers at foreign frames with clean refusal or completion semantics), Ffi permission unioned by the linker and enforced by the receiver. Close gate: e2e matrix — {python, typescript, C} × {remote transfer, snapshot→resume, remote+resume combined} — green, plus book chapter for the combined story.

---

## 0. Global execution rules (binding for every workflow)

1. **Toolchain:** every cargo/just invocation via `direnv exec /home/dev/dev/shape-lang <cmd>`.
2. **Worktrees:** one branch + one **pre-created pinned worktree** per workflow (`git worktree add`), created inline before dispatch. Never rely on `isolation: 'worktree'` for code-mutating agents (known-unreliable: stale base + agents cd into main repo).
3. **Merge gate:** `bash scripts/verify-merge.sh` (11 checks) + `just check-clean` + `just test` before any merge to main. Merges serialize; no two workflow branches merge in the same step without a rebase-and-regate.
4. **Regression scope:** narrow fixes verified via **blast-radius module diff** (worktree-vs-main FAILED-name diff of affected test modules), not full single-threaded dual runs.
5. **Forbidden patterns:** all FFI/marshal/snapshot rebuild work is squarely in deleted-ValueWord territory. Any agent proposal matching the CLAUDE.md §Forbidden regex `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)` is refused on sight and surfaced. Rebuilds go through ADR-006 §2.7.4/§2.7.5 typed marshal (KindedSlot / NativeKind carriers), never raw-bits synthesis. Considered-but-rejected compromises logged in `docs/defections.md`.
6. **StructuredOutput schemas ≤6 fields** (larger schemas trap workflow agents in empty-emit retry loops).
7. **Book truth-gate is the checkpoint regression** (hard gate): the full gate re-runs at the end of every wave, not only at the end of the program. A wave is not closed while its gate delta is negative.
8. **Adversarial verify by default:** every "fixed" claim gets an independent refuter agent reproducing the audit's original failing probe end-to-end (vm AND jit modes) before it counts.
9. **Surface-and-stop discipline:** if a fix genuinely seems to need dynamic dispatch or a new Convert opcode, the workflow STOPS that lane and surfaces to the user; it does not improvise.

---

## 1. Wave / dependency graph

```
Wave 0 (parallel, independent, low-risk)
  WF-0A gate-hardening-dead-code
  WF-0B vmjit-differential-harness       ──┐ (harness used by all later waves)
  WF-0C parser-exponential-blowup          │
                                           │
Wave 1 (correctness core, parallel)        ▼
  WF-1A jit-default-correctness         (uses 0B)
  WF-1B comptime-marshal-family         ──► unblocks WF-2E (serialization stdlib), state::hash
  WF-1C drop-escape-boundaries
  WF-1D security-wiring
                                           │
Wave 2 (feature rebuilds)                  ▼
  WF-2A ffi-rebuild (op_call_foreign + native_abi + Ffi perm)   [biggest; depends 1D perm plumbing]
  WF-2B snapshot-resume (W17 completion)                        [independent]
  WF-2C remote-per-function-transfer                            [depends 2B state prims lightly]
  WF-2D async-truthiness                                        [independent]
  WF-2E stdlib-serialization-native (json/msgpack/toml/yaml/xml/http/time/finance) [depends 1B]
                                           │
Wave 3 (edges + polish, parallel)          ▼
  WF-3A type-system-edges (impl-for-builtins, narrowing, bigint, decimal, unwrap, empty-array)
  WF-3B ux-polish (error text, version, LSP, tree-sitter, CLAUDE.md/llms.txt, doc mechanicals)
  WF-3C cycle-leak-mitigation (weak refs + docs)   [ruling required]
                                           │
Wave 4                                     ▼
  WF-4 book-truth-gate-close (full 616-example gate → 100% or explicitly-cautioned)
```

---

## 2. Wave 0 — Foundations

### WF-0A `gate-hardening-dead-code` (audit §5, rec 9)
Deletion targets are fully enumerated by the audit — pure pipeline work.
- **Phases:** `Inventory-confirm` (1 agent re-verifies each target is still dead at HEAD) → `Delete` (pipeline over ~10 deletion units: shape-gc fix-or-remove, 25 orphan .rs files, ReliableOnly variant, IntrinsicsRegistry, orphan-only `jit` feature + cranelift deps, ~25 unused deps, 173 `#[allow(dead_code)]` triage, shape-types/shape-viz-native skeletons, typed_access_bench) → `Gates` (rejoin `--benches` to `just check-clean`; extend verify-merge lockstep to the JIT retain/release tables ownership.rs:264-338 + collection_arc.rs:227-417; generate JIT return-kind tables from method_registry instead of hand-sync) → `Verify` (check-clean + test).
- **Fan-out:** pipeline(units, delete-agent, compile-check-agent). Each unit compiles independently before the barrier.
- **Close gate:** `just check-clean` green **including --benches**; verify-merge extended checks pass; sentinel no_dynamic tests green.

### WF-0B `vmjit-differential-harness` (rec 2 tail)
- **Phases:** `Recover` (locate/reconstruct the 738-program corpus from the audit scratchpad; check it into `tools/` or regenerate from book examples) → `Harness` (a `just diff-vmjit` recipe: run each program under `--mode vm` and `--mode jit`, diff stdout/exit; emit FAILED-name list) → `Baseline` (record current 19-double-exec + divergence set as the known-red baseline file).
- Also: make the 117-case numeric-conversion suite run under **both** modes in CI tiers.
- **Close gate:** recipe runs end-to-end; baseline file committed; suite dual-mode.

### WF-0C `parser-exponential-blowup` (finding: depth-12 nesting >150s, shape.pest:1099)
- **Phases:** `Diagnose` (2 agents independently profile the pest backtracking path) → `Fix` (restructure the ordered choice / add memoization or precedence-climbing for the hot rule) → `Verify` (perf harness: depths 4–20 must stay <1s; full parser test suite; book-gate parse-only sweep to catch grammar regressions).
- Single-lane workflow with adversarial perf verification; not a wide fan-out.
- **Close gate:** depth-20 parse <1s in run and check; zero parse regressions across all `.shape` files in repo + book examples.

---

## 3. Wave 1 — Correctness core

### WF-1A `jit-default-correctness` (audit §4.4, rec 2) — branch `fix/jit-correctness`
Four independent sub-fixes, one shared verification harness (WF-0B).
- **Sites:** (a) signal -1 outer-Err whole-program re-run — executor.rs:886-891: carve out mid-run failures so fallback resumes/aborts but never re-executes side effects; (b) i64 checked add/sub/mul per D3 ruling — rvalues.rs:1559-1563 (delete the stale 2026-05-20 wrapping comment); (c) route annotated functions through wrappers under JIT (JIT currently calls un-wrapped impl); (d) arity-2 HashMap filter/map wrongly rewritten to Array-only filterIndexed/mapIndexed — function_calls.rs:4714.
- **Fan-out:** pipeline over the 4 sub-fixes; each = fix-agent → refuter-agent reproducing the audit probe (BEFORE-prints-twice; add_pair(i64::MAX,1); before-hook short-circuit 99-vs-5000; HashMap.filter garbage int) under both modes → blast-radius diff.
- **Also:** tiered-compilation inertness (enable_tiered_compilation zero callers, promoted-dispatch NotImplemented stub at control_flow/mod.rs:285-300). **Scope ruling applied:** wire it OR delete it is a strategy decision — this workflow only *documents* current inertness and adds a failing-marked test; the wire-vs-delete decision goes to the user (Decision D4 below).
- **Close gate:** `just diff-vmjit` delta strictly positive, zero new divergences; D3 conformance green under jit; 4 audit probes green.

### WF-1B `comptime-marshal-family` (audit §4 rec 5; retires ~9 findings) — branch `fix/comptime-marshal`
Root-cause-first: two fixes fan out into nine symptom validations.
- **Root causes:** (1) Bool-collapse in vec!-declared builtin args (comptime_builtins.rs:466-470, marshal.rs:2295-2298) — destroys `error()` messages, silences `warning()`, breaks `type_info()`, drives `state::hash` constant digest; (2) schema-id collision corrupting every `target.fields` descriptor (garbage keys from json's schema); (3) `set return` bypassing body-vs-signature check (functions_annotations.rs:1492 → SIGSEGV).
- **Phases:** `RootFix` (2 agents: marshal fix; schema-registration fix — sequential on the same branch, they touch the same files) → `TypeCheck` (close the set-return verification hole; the explicit-annotation path already rejects, reuse it) → `SymptomSweep` (parallel refuters: error()/warning()/type_info()/target.fields/field.name/state::hash/param.const/`__original__(args)` semantics/descriptor keys — each reproduces the audit probe) → `Gate`.
- **Note:** `__original__(args)` (compiler injects array-as-int) is its own small compile fix in the same territory; include as a 4th lane.
- **Close gate:** all comptime book examples in adv-comptime chunk pass; segfault probe now a compile error; state::hash produces distinct digests for distinct inputs.

### WF-1C `drop-escape-boundaries` (audit §4.7, rec 6, ADR-006 §2.7.30) — branch `fix/drop-escape`
- **Sites:** (a) returned Drop values never dropped — functions.rs:2335-2348 skips producer DropCall, caller never re-arms; (b) returned-closure captures dropped prematurely (use-after-finalize); (c) drop-error containment (op_drop_call_impl propagates with `?`; book guarantees log-and-continue + remaining drops run + return value preserved).
- **Fan-out:** `Design` (1 agent maps escape-detection extension: bare-identifier tail returns → closure captures, per §2.7.30) → `Implement` (3 lanes) → `Verify` (refuters on the audit repros: returned-value-never-drops, "dropped 9 before closure read", b.drop()-error-aborts; plus the existing non-escaping suite must stay green — reverse order, per-iteration, early return, module scope).
- **Close gate:** resource-management.mdx examples green; no regression in non-escaping Drop tests.

### WF-1D `security-wiring` (audit §4.2, rec 3) — branch `fix/security-wiring`
Everything exists; nothing is connected. Plumbing workflow.
- **Sites:** `let _sandbox = config.sandbox;` (serve_cmd.rs:430); `load_program_with_permissions` zero call sites (program.rs:357,381; remote.rs:743 uses plain load); shape.toml `[permissions]`/`[sandbox]` parsed-not-enforced; `check_permission` None=allow-all with production ModuleContext hardcoding None (modules.rs:699-706); no resource-limit flags on `shape run`.
- **Phases:** `Thread` (pipeline over 5 plumbing sites) → `EscapeTest` (regression test asserting the live sandbox-escape `file::write_text` under `serve --sandbox strict` is REFUSED; fs.read=false refuses /etc/hostname; resource limits actually fire) → `Gate`.
- **Also reserves** (does not implement) the Ffi/Native permission enum slot for WF-2A — one commit adding the variant + tags so content hashes stabilize before FFI lands.
- **Close gate:** escape regression test red-before/green-after committed; all three tiers demonstrably fire via CLI.

---

## 4. Wave 2 — Feature rebuilds

### WF-2A `ffi-rebuild` (audit §4.1, rec 1) — branch `rebuild/ffi` — THE BIG ONE
One stub gates three verticals (Python, TypeScript, C). This is phase-2c territory: the rebuild MUST be the ADR-006 §2.7.4/§2.7.5 typed-marshal design. Hard forbidden-pattern exposure — every agent prompt carries the refusal list.
- **Phases:**
  1. `Design` — 1 agent drafts the marshal design doc against ADR-006 §2.7.4/§2.7.5 (KindedSlot arg/ret carriers per FieldType, no raw-u64 slices), reviewed by 2 adversarial ADR-compliance judges. STOP-and-surface if design needs anything on the forbidden list.
  2. `Interpreter-C` — rebuild `link_native_function`/`invoke_linked_function` (native_abi.rs:72-104) + `op_call_foreign` (control_flow/mod.rs:854-903) for extern C first (simplest marshal surface); make linking **lazy** so declaration alone is never fatal; wire `out`-param stubs; e2e against a gcc-built .so.
  3. `Interpreter-Py/TS` — LanguageRuntimeVTable invocation path for the PyO3 + deno_core extensions (extension sides are already real); fix the unreachable bundled eval namespaces; `Result<T>` error channel.
  4. `Permission` — gate every foreign call behind the Ffi permission (variant landed in WF-1D); scope constraints for library paths.
  5. `JIT` — either implement `jit_call_foreign_impl` (ffi/control/mod.rs:931 `todo!()`) or make foreign calls a *clean, tested* deopt-to-interpreter (no silent divergence). Recommend clean deopt first, native JIT path as follow-up.
  6. `CI` — un-gate/rewrite the Python e2e tests (current ones use compiler-rejected signatures) into a tier that actually runs, so the path can never silently die again. Same for TS and C.
  7. `Statics` — the compile-side contradictions: cview<T>/cmut<T> vs CView/CMut, cstring↔Shape string, ptr↔int inexpressible (book's out-param example must compile), `shape check` frontmatter parsing.
- **Fan-out:** phases 2/3/7 are pipelines over call-shape matrices (arg types × return types × error paths), each cell = implement-agent → differential probe vm+jit.
- **Close gate:** polyglot-c book chunk from 4/10 → 10/10; python/TS book examples green; declaring-without-calling extern C is non-fatal; foreign e2e in CI tier; no forbidden-pattern hits in `just check-no-dynamic`.

### WF-2B `snapshot-resume` (audit §4.3, W17 completion — already release-blocking per 2026-05-29 ruling) — branch `rebuild/snapshot-resume`
- **Sites:** SNAPSHOT_FUTURE_ID sentinel with zero consumers (builtins.rs:306-309); both resume stubs (execution.rs:190/209 — "depends on the deleted ValueWord carrier" is a lie post-§2.7.7, snapshot serialization is parallel Vec<u64>+Vec<NativeKind>); Ctrl+C saves nothing (no producer of ShapeError::Interrupted); `std::core::state` W17 stubs; whole-VM restore lands empty + 8 opaque-stub arms + a Bool-default violation (per project memory).
- **Phases:** `Catch` (host-side sentinel consumer → serialize via §2.7.7 kind-track machinery, which the audit confirms exists at library level) → `Persist/Resume` (both entry points; recompile-and-resume path) → `Interrupt` (Ctrl+C producer) → `StatePrims` (std::core::state set; state::hash correctness arrives from WF-1B) → `RoundTrip` (fan-out: N snapshot/resume programs covering frames, closures, module bindings, &mut references per the reference-serialization ruling, TypedObjects, mid-loop suspension — each program = write-agent + refuter running snapshot→kill→resume→assert-identical-output).
- **Close gate:** snapshot()/--resume/Ctrl+C all work end-to-end; leaked `Suspended on future 18446…` string unreachable; adv-exec book chunk green.

### WF-2C `remote-per-function-transfer` (audit §4.6, rec 8) — branch `fix/remote-transfer`
Small precise fixes over already-working Rust machinery.
- **Sites:** 3-vs-2 `__call` arity (remote.shape:81 vs remote_builtins.rs:318); stub `__call` body (remote_builtins.rs:328-340) → implement over the working Rust Call path; receiver closure/upvalue rejection (remote.rs:731) → thread the kind track; missing-blob panic → construct `RemoteErrorKind::MissingModuleFunction`; `localhost` rejected by transport (hostname resolution or doc fix).
- **Phases:** `Fix` (pipeline over 5 sites) → `E2E` (fan-out: @remote annotation, remote::__call direct, closure-carrying call, missing-blob error path, permission-in-hash + permission-union tests the audit flags as missing) → `Gate`.
- **Close gate:** adv-distributed book chunk 8/15 → 15/15; the two missing permission tests exist and pass.

### WF-2D `async-truthiness` (audit §4.5, rec 7) — branch `rebuild/async` — **Decision D1 applies**
- **Unconditional bug fixes** (do regardless of D1): (a) module-qualified calls inside async fns fail `Unknown qualified call` (function_calls.rs:3185) — source-order registration bug; (b) top-level `await time::sleep` Rust-panics via block_in_place (modules.rs:733).
- **Semantics (recommended: implement real concurrency):** `async let` compiles RHS eagerly before SpawnTask (advanced.rs:745) → closure-thunk deferral; `join race` runs all to completion; `join any` never skips failures; scope cancellation vacuous.
- **Phases:** `Bugfixes` (2 lanes) → `Semantics-design` (1 agent + 2 judges: deferral compilation strategy) → `Implement` (pipeline: async let / race / any / settle / cancellation) → `TimingVerify` (refuters with wall-clock asserts: two 1s tasks < 1.3s; race returns at first completion; cancellation observable) → `Gate`.
- **Close gate:** timing probes pass both modes; async.mdx examples green as written.

### WF-2E `stdlib-serialization-native` (audit §4.12) — branch `fix/stdlib-native` — depends on WF-1B marshal fixes
- **Modules & sites:** http (SIGSEGV in options-arg unmarshalling, `FromSlot for Vec<(Arc<String>,Arc<HeapValue>)>`); xml::stringify SIGSEGV (marshal.rs:938 unsound reinterpretation) + unconditional stub (xml.rs:334); json stringify stub "pending N7" (json.rs:502-511) + 5 navigation methods failing on Ptr(TypedObject) + typed-parse field_kinds=0; msgpack 4/4 stubbed "pending N4/N6" (msgpack_module.rs:52-118); toml/yaml stubbed; time now()/stopwatch() Discriminant(5)/K3 error + benchmark kind-blind raw-bits f64 read (stdlib_time.rs:128 — forbidden-pattern-adjacent, fix properly); std::finance parse failures + backtest::engine compiler stack overflow (likely fixed by WF-0C parser work — verify, else own lane).
- **Fan-out:** `MarshalCore` first (the N4/N6/N7 typed marshal for object-graph args — shared infrastructure, 1-2 agents sequential) → `Modules` pipeline (one lane per module: fix → refuter runs the module's failing book examples vm+jit) → `Gate`.
- **Close gate:** std-native-1 47%→>95%, std-domain 11%→>90% book chunks; zero SIGSEGVs across all stdlib book examples.

---

## 5. Wave 3 — Edges + polish

### WF-3A `type-system-edges` (audit §4.11, §4.8) — branch `fix/type-edges` — **Decision D2 (bigint) applies**
- **Lanes:** impl-Trait-for-builtins runtime dispatch (compiles, `no method 'shout' on receiver kind String`); flow-narrowing (`instanceof` per operators.mdx:159; `null` tokenization); string/HashMap builtin method args compile-time-checked; `fn empty() { [] }` → op_new_array(0) V3-S5 stub (per Seam-1 ruling: untyped-array = compile error unless let-gen resolves T); `Result.unwrap()` dispatch (`Generic{Result<int,string>} cannot have fields`); decimal `round()`/`abs()` + document `D` suffix; **bigint**: recommended real arbitrary-precision heap payload replacing the Arc<i64> placeholder (heap_variants.rs:437-442) + literal/annotation/cast/constructor paths (book recommends it 4×).
- **Fan-out:** pipeline over 7 lanes, each fix-agent → refuter on the audit probe → blast-radius diff.
- **Close gate:** all §4.11 probes flip; integer-types.mdx bigint examples run.

### WF-3B `ux-polish-split-brains` (audit §5 split-brains, rec 10) — branch `fix/ux-polish`
Wide, shallow, highly parallel — ideal pipeline.
- **Lanes (~14):** stop labeling compile errors "Runtime error"; strip internal jargon (ADR refs, ckpt-5, "REFUSED ON SIGHT") from user-facing text; clear unavailability message for anything still stubbed; version reconcile 0.3.2/0.3.3; LSP extern-C false error (diagnostics.rs:1533 validate_type_annotations(true)); LSP completion textEdits; tree-sitter drift (extern, out, f$/f#); CLAUDE.md trait-syntax fix (`fn method(self)`/`extends` — book is right, CLAUDE.md wrong); shape-mcp llms.txt 5/5 wrong facts; char three-way inconsistency (at minimum: consistent compile error + FieldType story); match-guard docs (`where` not `if`) in llm_summary + CLAUDE.md; Array/Vec naming in errors.rs:211; book build docs (mdBook→Astro); FORMAT_VERSION + `shape bundle` + keys-trust flags + operators `// 4` + localhost→IP doc mechanicals.
- **Close gate:** grep-based assertions for jargon strings; LSP integration test on extern C; llms.txt fact-check agent passes.

### WF-3C `cycle-leak-mitigation` (audit §4.9) — **Decision D3 applies**
- Recommended scope: document the leak class in the book (memory model chapter) + introduce weak references for the closure-capture cycle pattern + a heap-growth soak test in the deep tier. Full GC is explicitly out of scope (shape-gc is deleted-or-quarantined by WF-0A).
- Single-lane design + implement + soak-verify (RSS-flat assertion on the audit's 20M-iteration cycle repro with weak refs applied).

---

## 6. Wave 4 — Close

### WF-4 `book-truth-gate-close` (audit §3, rec 4)
The committed ready-to-fire acceptance-gate workflow (~51 slice agents) is the vehicle; this run is the program's exit criterion.
- **Phases:** `FullGate` (all 616 examples, vm+jit) → `Classify` (per feedback ruling: BLOCKED/masked-unknown bucket exists; "pre-existing" may not absorb stub-masked failures; spot-check small-real-bug vs large-benign split) → `FixBothDirections` (pipeline: remaining failing examples → fix code or fix doc; **stale-pessimistic cluster**: remove wrong cautions on spread/comprehensions/NumericVec/trait-bounds/set/random/unicode::graphemes/destructuring — these work today and the cautions steer users away) → `ReGate` → loop until stable.
- **Close gate:** 616/616 pass, or every failure carries an accurate availability caution ratified by the user. Whole-program-deopt rate (currently ~62%) reported as a metric with follow-up filed (not a blocker — correctness first, JIT coverage is v0.4).

---

## 6bis. Wave 0 execution log (2026-07-05)

Findings from the Wave-0 runs that adjust later waves:

- **Audit corrections established during confirm passes:**
  - shape-vm `jit` feature is **live** (gates OSR/tier-dispatch in 7 files) — audit's "exists solely for orphan jit_ffi_integration.rs" was wrong; only the orphan file + 5 cranelift optional deps were deleted, the feature stays.
  - Parser blowup root cause is **not** shape.pest:1099 `primary_expr` (exonerated) — it is shared-prefix re-parsing in three ordered-choice rules (`assignment_expr` ×2, `range_expr` ×3, `array_literal` ×2), fixed by left-factoring.
  - The audit's "616 gate examples" denominator matches no committed manifest (738 fences, 240 runnable); WF-0B defined its own explicit denominator (467-program corpus).
- **Two NEW VM/JIT divergence classes** found by the WF-0B baseline, absent from the audit: `hof-return-kind-raw-bits` and `jit-object-merge-field-add` (3 acceptance programs). → **WF-1A scope grows a triage lane** for these two classes (root-cause + fix or reclassify).
- **Book-tier double-execution caveat:** the audit's 19-program double-exec class does not reproduce inside the 240 runnable-fence subset (every candidate compile-time-deopts before side effects); the class is held live by committed synthetic repros. → Corpus-widening candidate (full 738 fences incl. non-runnable transforms) noted for WF-4.
- **Parser deferred follow-ups** (per WF-0C decision, least-change ruling): (1) left-factor `array_literal`'s shared `[ ~ expression` prefix to kill the residual ~2x/level on nested list-comprehension *heads* (user-visible only ≥~25 nesting levels); (2) consider `pest::pratt_parser` for the expression core long-term. → Filed to WF-3B or a v0.4 parser lane.
- **Cross-workflow coupling:** known-red entry `ACC__pattern-matching__large` is a TIMEOUT artifact of the parser blowup (27s compile) — remove from known-red.json when `wave0/parser-backtracking` merges.
- **Pre-existing failures pinned:** 4 shape-jit `typedarray_ptr_regression_tests::jit_closure_capture_*` deep-test failures reproduce identically at main HEAD (`E_TYPED_OPCODE_WITHOUT_PROOF` at `emit_return_value_with_ownership` — not Wave-0 regressions); 1 known shape-jit parallel-suite flake.
- **WF-0A close (yellow, pre-existing residuals only):** 5 logical commits on `wave0/deadcode-gates` (orphans / ReliableOnly+IntrinsicsRegistry / Cargo mass incl. shape-gc crate deletion / gate hardening / allow triage). `just check-clean` is now `--all-targets` incl. benches; `verify-merge.sh` is now 15 checks incl. CHECK 6b (JIT retain/release lockstep — frozen 24-variant baseline on the legacy fallback: growth blocked, existing surface still to fix).
- **Three REAL JIT return-kind drift families** pinned by the new `registry_cross_check` test (crates/shape-jit/src/mir_compiler/types.rs): `HashMap.get` Option-vs-bare-value, `Array<int>.mean` Int64-vs-Float64, `HashMap.iter` receiver carrier mismatch (**UB-class** — cannot work through kinded dispatch). → **WF-1A triage lane** alongside the two divergence classes above.
- **Watch item:** a spurious uncommitted re-addition of rayon/pest/pest_derive to `crates/shape-runtime/Cargo.toml` appeared mid-run in the WF-0A worktree (origin unknown; discarded after verifying zero usage). If it recurs, suspect a hook or concurrent tooling — same family as the documented `module_resolution.rs` linter hook.

## 6ter. Ratification binders (2026-07-05, into implementation waves)

- **All 54 design defaults ratified** (`docs/design/00-priority-spine-overview.md` RATIFIED 2026-07-05); implementation workflows build against the ratified docs.
- **Q13 override → binding ADR amendment (WF-2A stage 3):** nonconforming foreign returns are class-1 `Err` on the user's `Result` (discriminator prefix `TypeConformanceError:` in the payload), NOT `RuntimeError`. This **contradicts ADR-006 §2.7.29 clause 2** ("wire-vs-declared type mismatch → `VMError::RuntimeError`") as written at HEAD. WF-2A stage 3 MUST amend `docs/adr/006-value-and-memory-model.md` §2.7.29 clause 2 to match, moving ADR + code together (do not pre-amend). Tracked in `ffi-rebuild.md` §3.2. Marshal-arm *gaps* stay the compile-time marshalability error / surface-and-stop backstop — never `Err`; the discriminator keeps them distinguishable from genuine foreign failures.
- **Permission posture (Q15/Q28/Q52):** local `shape run` grants `Ffi` unscoped; `shape serve` defaults `ffi_languages` strict-empty; loopback binds sandboxed+moderate, non-loopback Pure-only. Binds WF-1D (variant + posture) and WF-2A/2C/2F.
- **Q53(b):** foreign-function-ref closure-capture typed carrier green-lit as WF-2F-adjacent (serialized arm carrying entry `content_hash`, rebound via §4.2.0 ordinal↔hash); v1 refusal stays until it lands.
- **One-time hash break batched into WF-2A stage 0** (Q1): `frame_descriptor`+`capture_kinds` into `FunctionBlobHashInput`, A6 foreign_dependencies ordering + `CallForeign` ordinal rewrite + linker remap, A7 declared-alias, A5(ii) `__ffi_h{hex16}_return`, entry-hash += `is_async`/`param_names`. No persisted stores may exist before it lands.

## 6quater. Wave-0 close checkpoint (2026-07-05, main ce49fc36) — book-gate scope correction

Differential harness on merged main: **459 MATCH / 8 known-red / 0 unexpected** (the one Wave-0 baseline delta is `ACC__pattern-matching__large` TIMEOUT→MATCH from the parser fix — 27s→1.26s compile). The tight correctness gate is healthy.

**But the book truth-gate is structurally misleading — this changes the WF-4 exit criterion.** `run-book-truth-gate.mjs` reports **240/240 green, which is only the `runnable=true` fences**. Of 738 total fences, **498 are `runnable=false`** and skipped; a vm-only probe of those shows **110 pass / 388 FAIL**. Real book truth ≈ **350/738 (~47%)**, not 100%. Only 6 of 498 exclusions carry an explicit fails-at-HEAD note — 382 fail silently outside the gate. The `runnable=false` curation is itself part of how the audit's "documents broken features as working" happened. (See memory `project_book_gate_denominator_trap`.)

**Binding corrections:**
- **WF-4 close criterion is measured against the full ~738-fence universe** (minus genuinely-intentional-error examples), NOT 240 runnable=true. A green 240/240 is not an acceptable exit.
- **WF-4 audits the 498 exclusions:** flip the ~110 currently-passing (stale-pessimistic — datetime 15, functions 10, security-permissions 8, comptime-cookbook 7) to `runnable=true`; give every remaining exclusion an accurate fails-at-HEAD annotation tied to the fixing workflow.
- **Failure classes → workflow mapping:** 146 dead-stdlib-namespace (io/state/http/crypto/json/csv/regex/linalg) → WF-2A/2B/2E; 193 other (unknown annotations, typed-array-carrier semantics, state::capture/caller) → WF-1B/2B/3A; 27 parse errors, 20 NotImplemented/SURFACE spread across waves.
- **2 NEW SIGSEGVs** not in the audit → early triage: `A__advanced__ownership-deep-dive__15__L459` (likely Drop/ownership — flag to WF-1C) and `B__fundamentals__content__12__L341`. Added as differential-corpus candidates.

## 6quinquies. Wave-1 routed residuals (from WF-1A close, 2026-07-05)

WF-1A fixed its 4 core bugs + 2 of 3 drift families; the rest are **routed, not dropped**:

- **NEW LANE — `jit-fallback-engine-isolation` (WF-1A-followup, schedule Wave-1.5 / early Wave-2).** Genuine JIT correctness bug WF-1A surfaced: the `[jit-fallback]` interpreter path recompiles on the **same Cranelift engine already mutated** by `compile_program_for_inspection` (executor.rs:149), so `a+b` resolves against a polluted schema/impl registry → the `jit-object-merge-field-add` class (4 corpus ids: `ACC__objects-arrays__small`, `ACC__operators__small`, `ACC__operators__large`, `ACC__collections__large`). Merge codegen is correct in isolation — the fix is engine isolation on fallback. **Now higher-priority** because WF-1A's signal-reexec fix *expands* the set of programs that take the fallback path (entanglement noted). `ACC__collections__large` is a NEW instance unmasked by the item-4 fix. Pinned known-red until fixed.
- **WF-3A gains two lanes:** (1) `hof-return-kind-raw-bits` (`apply(f,x)` unprovable return with untyped param `f`) → needs HM let-generalization / HOF return-kind inference + the D2 number→int compile error, not a JIT stamp; (2) `HashMap.get` non-uniform return (Int64-on-hit / Null-on-miss) → ADR-006 §2.7.17 uniform `Option<V>` handler (registry_cross_check pin only, no corpus program).
- **NEW LANE — deep-test stdlib-JIT-caching root (follow-up).** The 4 `mir_compiler::typedarray_ptr_regression_tests::jit_closure_capture_array_*` deterministic failures (`E_TYPED_OPCODE_WITHOUT_PROOF`) + 1 flaky `jit_err_path_set_add_non_string_key` — all confirmed pre-existing at branch HEAD, the stdlib-JIT-compilation-caching root already flagged in CLAUDE.md known-constraints. Own follow-up lane.
- **`ACC__comptime__pb3` comptime/decimal panic** (V2 `NewTypedArrayString` has no `FrameDescriptor` → verifier bug, + rust_decimal `CapacityError` on f-string / JIT W17 Decimal-GetProp stub) — pre-existing, reproduces on main@562be041 with no workflow changes. **Tripped the diff-vmjit gate in 3 workflows (WF-1C, WF-1A finisher, WF-2D) — a recurring per-workflow tax.** ACTION: pin in `known-red.json` (class `TypedArrayString-framedescriptor / W17-typed-carrier`, cite repro on main@562be041), folded into the jit-fallback branch merge to avoid double-editing the file. Real fix → **WF-3A / W17-typed-carrier lane** (the `NewTypedArrayString` FrameDescriptor emission + the JIT `GetProp on Ptr(Decimal)` kinding).

## 6sexies. Wave-2 batch-1 completions

- **WF-2D async (green/yellow, only pb3):** real concurrency per Decision D1 — two 1s async-lets = 1003ms, race=203ms w/ loser cancelled, any skips failures, scope cancellation observable; both unconditional bugs fixed. New shared multi-thread `async_runtime.rs` (futures spawn + mpsc completion, avoids the block_in_place panic). 2 commits `wave2/async`. Book async.mdx concurrency claims now truthful; join-settle/named-branch-unpacking remain v0.4.
- **WF-1A-followup jit-fallback (green, committed `c5259d49`):** root cause was a `SchemaId`-counter (`TypeSchemaRegistry.next_id`) pollution, not codegen — the `[jit-fallback]` path now runs the already-built inspection bytecode via `execute_compiled` instead of recompiling. Retired the 4-id `jit-object-merge-field-add` class + **pinned `pb3`**; differential baseline **462→466 MATCH**. Branch ready for the Wave-2 merge batch.
- **NEW pinned flaky (from jit-fallback close):** `jit_err_path_set_add_non_string_key_surfaces_clean_error` — nondeterministic error-ordering race in bytecode type analysis on the `compile_program_for_inspection` path (`executor.rs:149`), flaps pass/fail even single-threaded `--exact`; predicate expects "Set<int> cannot have fields" but sometimes gets "Method 'size' not found on type 'Set'". Pre-existing, causally unreachable from the fallback fix. → follow-up known-flaky pin + WF-3A error-ordering determinism.
- **Wave-2 merge plan:** one serialized batch of 6 branches (jit-fallback, wave2/async, wave2/stdlib-serialization, wave2/ffi-rest, wave2/snapshot-resume, wave2/remote-transfer) after 2A/2B/2C land; fold `pb3`+set-key flaky pins in via jit-fallback's known-red edit. Then WF-2F branches from the merged main.
- **WF-2E serialization (green/yellow, committed `wave2/stdlib-serialization` 8 commits):** json/msgpack/toml/yaml/xml/http/time all VM==JIT, SIGSEGVs + `pending N*` stubs gone. **The schema-identity collision family (WF-1B's root cause B) recurred in json/xml enum-node walking** — fixed with collision-robust by-name+structural detection + new `PolymorphicArg` marshal type + exhaustive 36-variant HeapKind dispatch. **http JIT `Undefined property: status` self-resolves when jit-fallback merges** (same dirty-engine bug). Residuals routed:
  - **`std::finance` BLOCKED (2 non-marshal walls) → WF-3A + a compiler-stack-overflow lane:** (1) `backtest::engine` import overflows the compiler stack (a real compilation recursion, NOT the Wave-0 parser blowup — verified); (2) finance stdlib `.shape` source has strict-flip violations (`let sum` mutated w/o `let mut`) — mechanical stdlib cleanup. `@warmup`/`@indicator` registration + type-syntax modernization already landed.
  - **json navigation (`.get/.at/.len/...`) PARTIAL → WF-3A inference:** works on an explicit `let d: Json = ...` receiver, not a bare `json.parse()` result (inference doesn't propagate `Json` to the bare binding).
  - **`variables__large` (W39 module-binding JIT SURFACE)** — another pre-existing differential non-MATCH surfaced; pin + route to a JIT module-binding lane.

- **WF-2C remote transfer (green/yellow, committed `wave2/remote-transfer` 5 commits):** `@remote` per-function transfer works END-TO-END over a real serve node (VM==JIT); receiver-owned permission-over-wire (zero sender trust); `MissingModuleFunction` + retry-once resupply; §2.7.8 capture kind-track; forbidden-pattern-clean (`RemoteDispatcher` trait replaces the deleted `CURRENT_PROGRAM` thread-local). **WF-2F precondition CONFIRMED.** Residuals → **NEW WF-2C-followup lane (runs parallel to WF-2F):**
  - Direct `remote::call(addr, fn, args)` public-surface compiler elaboration (Q33: call-site positional type-check + TypedObject `_0.._n` pack carrier + function-ref FromSlot) — library primitives exist, the compiler-recognized elaboration (same class as `as`→`__into_*`) is the remaining work.
  - Closure-over-wire USER-e2e (`@remote` on a capturing closure) — library-verified (refusal matrix green), not yet driven through a user program.
  - Heap-shaped return typed projection (arrays/objects; scalars/strings wired).
  - Active TLS-on-TCP termination in the serve accept loop (gate honestly refuses non-loopback without cert+token today; active encryption is the follow-up).

## 7. Decisions requiring user ruling (recommended defaults marked)

| # | Decision | Options | Recommendation |
|---|----------|---------|----------------|
| D1 | Async semantics | implement real concurrency vs document serial model | **Implement** (WF-2D) — "fix all" reading; book already promises it |
| D2 | bigint | implement arbitrary precision vs remove type + fix book | **Implement** — book recommends it 4× as the D3-overflow escape hatch |
| D3 | Cycle leaks | weak refs + docs vs real GC vs docs-only | **Weak refs + docs** (WF-3C); GC deferred |
| D4 | Tiered compilation | wire T1/T2 end-to-end vs delete the inert machinery | **Defer decision to post-Wave-1**; WF-1A only documents + marks. Wiring it is v0.4-scale |
| D5 | JIT foreign path | native codegen vs clean tested deopt | **Clean deopt now** (WF-2A phase 5), native path follow-up |

---

## 8. Launch order

0. **Immediately (parallel to Wave 0):** WF-D priority-design — five design docs to user ratification; the long-pole Wave-2 rebuilds implement against them.
1. **Now:** WF-0A + WF-0B + WF-0C in parallel (independent branches).
2. **On 0B done:** WF-1A, 1B, 1C, 1D in parallel (4 branches, 4 pinned worktrees).
3. **Wave-1 merges serialized** through verify-merge; book truth-gate checkpoint #1.
4. **Then:** WF-2B, 2C, 2D immediately; WF-2E when 1B merges; WF-2A when 1D merges (permission variant landed). 2A is the long pole — start its Design phase during Wave 1.
5. **Wave-2 merges + checkpoint #2. Then Wave 3 parallel, checkpoint #3, then WF-4 to close.**

Rough scale: Wave 0 ≈ a day of wall-clock agent time; Wave 1 ≈ 2-4 days; Wave 2 ≈ 1-2 weeks (2A dominates); Waves 3-4 ≈ 3-5 days. Token cost is dominated by WF-2A and WF-4.
