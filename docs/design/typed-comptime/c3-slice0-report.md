# C3 #14 slice-0 report — spikes + baselines of record (S0, no product code)

Synthesis of the six S0 probes (census, per-specialization feasibility,
baselines, SPIKE-JIT, SPIKE-GENERIC+SPIKE-AMBIENT, SPIKE-VMRED) per the
c3-decisions.md slice plan. All measurements at worktree
`/home/dev/dev/shape-lang/shape-adr009-a3`, branch `adr009/c3`, HEAD
`d138a4e4`, via the lane `systemd-run` prefix, foreground, `-j1`
(`--test-threads=1`), one cargo at a time. Every probe restored its throwaway
patches; the tree was proven clean (`git status --porcelain` empty) after
each probe and before this report. S0 mints no product code.

## VERDICTS UP FRONT

1. **C3-G6 depth verdict: SMALL** — C3 can go full native, with one precise
   qualification: full native is reached by building the wrapper as a
   GENERATED ORDINARY TYPED AST FUNCTION compiled through the ordinary
   pipeline (exactly the G2/G3/G4 shape), NOT by un-suppressing `mir_data`
   on the legacy weave — that was measured and produces silent VM≠JIT
   divergence (forbidden). No new MirToIR lowering capability is required
   (§2).
2. **Per-specialization checking: FEASIBLE** — the "check body fn against
   bound Sig per specialization" machine ALREADY EXISTS as the
   monomorphization pipeline; S1 composes it rather than building a checker.
   Genuinely new pieces are small (application-site error attribution,
   sig→tuple glue, concrete-case comparison). One ruling needed
   (tier authority, §7.3) and one carrier gap discovered (§7.2, item 1 of §8).
3. **G4's tuple carrier does not exist at HEAD** — no tuple literal, no `.0`
   index, heterogeneous bracket types are a named rejection ("Use a struct
   instead"). This is the largest plan delta: S1 needs either new language
   surface (~8-file fan-out) or a carrier substitution (§8 item 1).
4. **Baselines of record measured; ZERO deviations from priors** — exact
   FAILED-name sets in §1; future slices gate against these sets, not zero.
5. **VMRED root-cause: hygienic-rename lookup miss, NOT typed-prefix
   erasure** — the product invariant the standing vmlib red protects is
   GREEN at HEAD. Disposition confirmed: rewrite at the S5/S6 pin-rewrite
   wave (7-name → 6-name), by role not spelling; do NOT fix early (§5).
6. **[C0926] confirmed unminted-reserved; true next-free = C0931** —
   empirical census in §6. C0910 is permanently burned (absence pins);
   battery.rs's "C3 starts at C0926" comment is stale as a next-free source.
7. **G8 sharpening: today is NOT uniformly broken on generic targets** —
   single-type-param homogeneous generics with type-agnostic hooks WORK
   today (g1/g2/g4/g5). The G8 blanket rejection is a strict improvement
   over the g3/g6 failure modes but a deliberate behavior WITHDRAWAL for the
   working class — defections-log entry required (§3).
8. **[C0926] boundary evidenced** — the a6 shadow probe (target-module
   binding silently wins over the annotation's own module const) is the
   motivating disaster to quote in the rejection rationale (§4). One
   surfaced gap no G-ruling covers: annotations on fn-local nested fns are
   silently dropped (§4, §8 item 10).

---

## 1. Baselines of record

Fresh single-run measurements at d138a4e4 (no N=3 flap census in scope).
Future slices diff FAILED-name sets against EXACTLY these lists. Raw logs
were retained in the session scratchpad (`c3-s0/0{1..7}*.log`, `grep -a`
required — NULs); the scratchpad is session-scoped, so this section carries
the load-bearing content.

| # | Suite | Passed | FAILED | Ignored | Wall | vs prior |
|---|-------|--------|--------|---------|------|----------|
| 1 | shape-test `--test annotations_comptime` | 116 | 10 | 0 | 63.4s | MATCHES exactly |
| 2 | shape-test `--test comptime` | 261 | 3 | 0 | 99.1s | MATCHES |
| 3 | shape-test `--test annotations_runtime` | 24 | 0 | 0 | 12.4s | CLEAN |
| 4 | shape-test `--test annotation_targets` | 24 | 0 | 0 | 10.8s | CLEAN |
| 5 | shape-vm `--lib` | 3168 | 7 | 34 | 69.7s | MATCHES 7-name prior |
| 6a | shape-test `--test lsp` | 502 | 0 | 0 | 45.5s | CLEAN |
| 6b | shape-lsp `--lib` | 882 | 0 | 0 | 37.1s | CLEAN |
| 7 | shape-cli `--test cli_tests` | 47 | 0 | 0 | 255.2s | CLEAN (single-threaded) |

**annotations_comptime FAILED (10):**

- `executed_extend_authority::d7_direct_extend_target_method_materializes_via_executed_prepass`
- `executed_extend_authority::d8_stacked_annotations_both_extend_via_executed_prepass`
- `executed_extend_authority::false_guarded_extend_is_not_materialized_real_method_still_works`
- `executed_extend_authority::function_target_extend_explicit_type_materializes_via_executed_prepass`
- `executed_extend_authority::r6_target_resolves_to_annotated_type_per_application`
- `executed_extend_authority::s4_extend_owner_binds_by_position_not_the_word_target`
- `executed_extend_authority::s4_user_type_named_target_resolves_nominally`
- `executed_extend_authority::u10_target_delivered_by_position_after_hygienic_rename`
- `generated_method_runtime::generated_extend_target_arithmetic_method_behaves_identically_in_vm_and_jit`
- `generated_method_runtime::generated_extend_target_method_behaves_identically_in_vm_and_jit`

**comptime FAILED (3):**

- `annotations::b6_annotation_iterates_callable_parameters_on_vm_and_jit`
- `annotations::b6_annotation_reads_callable_param_modes_on_vm_and_jit`
- `callable::hash_tracer_does_not_disturb_formatted_strings`

**shape-vm --lib FAILED (7):**

- `compiler::expressions::advanced::tests::test_async_let_binding_is_immutable`
- `compiler::expressions::advanced::tests::test_match_arm_empty_array_unprovable_element_is_clean_compile_error`
- `compiler::helpers::frame_return_metadata_tests::runtime_before_hook_impl_stamps_declared_typed_array_parameter_prefix`
- `compiler::monomorphization::cache::route_tests::inlined_closure_keeps_outer_authored_type_ref_in_its_parameter_scope`
- `compiler::monomorphization::cache::route_tests::unavailable_and_missing_callsite_evidence_execute_only_in_legacy_domain`
- `compiler::monomorphization::type_resolution::tests::ws6b_inferred_result_variable_arg`
- `compiler::monomorphization::type_resolution::tests::ws6_generic_id_ok_arg`

**Flapper accounting.** The known vmlib flap pair was GREEN this run and is
NOT in the of-record set: `compiler::comptime_builtins::semantic_freeze::projection::tests::nested_exact_argument_is_closed_before_the_outer_overlay_is_dropped`
and `compiler::monomorphization::cache::route_tests::nested_exact_calls_close_outer_arguments_before_inner_compilation`.
A future failure of either is a KNOWN FLAP (rerun `--exact` before treating
as a regression); a future green is not a fix.

**cli_tests notes.** Module breakdown (non-vacuity verified):
jit_c2_install_native 6, jit_closure_capture_native 9,
jit_fallback_diagnostic_matrix 8, jit_fstring_format 4,
jit_generated_capture_native 9, script_execution 8, tree 3.
`bin/shape-cli/tests/cli/jit_test_support.rs` defines `cli_process_lock()`
(static `Mutex<()>`) serializing every `shape run --mode <m>` subprocess with
a 60s per-subprocess timeout; under load a timeout-induced flap presents as a
`jit_*` failure — rerun before blaming a change. Of-record posture stays
`--test-threads=1` for the whole target.

**Coverage boundary.** Suites not listed (shape-cli execution_tests /
language_tests / stdlib_tests / distributed_*, shape-runtime, shape-jit)
have NO baseline of record from this pass. All runs used the lane cgroup
envelope (MemoryMax=24G, TasksMax=512); reproducing outside it may change
flap behavior (the JIT lock comment ties serialization to low-TasksMax
stability).

**Layout discovery for downstream slices.** shape-test targets are
directory-based (`tests/<name>/main.rs` autodiscovery, no `[[test]]`
stanzas); the lsp target is `tools/shape-test/tests/lsp/` (23 modules incl.
`comptime.rs`, `typed_comptime.rs`, `generated_*.rs`).

---

## 2. C3-G6 — wrapped-MIR lowering depth: SMALL

**Verdict: SMALL.** C3 lowers MIR from the wrapped definition and goes full
native — PROVIDED the wrapper is built as a generated ordinary typed AST
`FunctionDef` compiled through the ordinary pipeline, so bytecode AND MIR
derive from the same wrapped definition. Un-suppressing `mir_data` on the
legacy raw-bytecode weave is NOT a path to SMALL (measured: silent VM≠JIT
divergence). The parts-plus-pinned-seam (Deep) branch of C3-G6 is not
needed.

### Evidence

1. **Suppression re-verified.** `crates/shape-vm/src/compiler/functions.rs:993-1006`
   (WF-1A Item 3 comment + `has_runtime_annotation_hooks`) and `:1046-1047`
   (`mir_data` intentionally `None`); single attach site `:1132` (inside the
   `compile_function` tail only). Mechanism: `analyze_function_body`
   (`functions/body_analysis.rs:13`) lowers MIR from the AST body; the weave
   (`compile_annotation_wrapper`, `functions_annotations.rs:4178`) emits the
   wrapper as RAW BYTECODE via `self.emit()` — no AST body, hence no MIR
   source for the wrapped definition. The cached MIR under the original name
   is the UNWRAPPED body.
2. **Behavior at HEAD: named, never silent, whole-program.** Hook fixtures
   (before args-mutation + after result-mutation, 200-call hot loop): VM and
   JIT exit 0 with identical stdout (40600), and `--mode jit` emits exactly
   one line: `[jit-fallback] function main failed JIT compile: Runtime
   error: JIT compilation failed: MirToIR: function 'calc' has no MIR data
   (bytecode-only functions are no longer supported); running under
   interpreter`. Why whole-program: the wrapper classifies
   `jit_compatible=true` (woven bytecode passes preflight) so `main` emits a
   direct native relocation to it; Phase-4 MirToIR then fails on
   `mir_data=None` (`crates/shape-jit/src/compiler/program.rs:225-227`), and
   demotion is refused at finalize because the failed body is natively
   referenced (program_finalize exactly-once rationale, `program.rs:~745`)
   → whole-program interpreter. Today a hook-bearing fn called from main
   costs ALL nativity but never diverges.
3. **Un-suppression measured (throwaway patch, restored).** Disabling the
   `functions.rs:1046` guard: JIT fully native, ZERO fallback lines, and
   BOTH HOOKS SILENTLY SKIPPED — VM=40600 vs JIT=20500; hook-order fixture
   VM printed before/after lines + 6|8|10, JIT printed 4|5|6. Proves the
   suppression is load-bearing, `:1132` is the single coupling point, and
   the JIT compiles whatever MIR it is handed (unwrapped-body MIR = wrong
   MIR).
4. **Roster facts at HEAD** (hook fixture, 201 functions): the `calc`
   wrapper `jit_compatible=true mir=FALSE`; the hygienic impl slot
   `jit_compatible=true mir=FALSE` (impl bodies compile inside
   `compile_wrapped_function` WITHOUT the mir-attach tail —
   `body_analysis_authority.rs:194-196`, "MIR … maps remain source-keyed");
   the specialized before/after handlers `mir=TRUE` (ordinary
   `compile_function` at `functions_annotations.rs:4129/4133`) and compiled
   natively in the patched run.
5. **C3-shape proxy: full native.** The G4 wrapper hand-written as ordinary
   typed code (hook_before mutates typed arg → direct impl call →
   hook_after mutates typed result, same hot loop): VM==JIT==40600, zero
   fallback. First-class fn value in a local + value-call (the `ctx.target`
   shape): VM==JIT==20500, zero fallback. MirToIR already lowers the entire
   typed-wrapper vocabulary.
6. **What typed AST cannot express is exactly the G7 deletion set:**
   IsArray/IsObject probing of the before-result, `FieldType::Any` args/ctx
   fields (`functions_annotations.rs:4276-4280`), the self-as-f64 magic
   first arg (`Constant::Number(wrapper_func_idx as f64)` at `:4297`).

### Bounded fixes on the SMALL path

(i) Replace the raw-bytecode weave with a generated typed AST wrapper
`FunctionDef` compiled via the ordinary path (E2 item_fn / C2 replace-body
precedent: generated/edited AST fns reach native JIT today). (ii) Attach
`mir_data` for hygienic impl emissions — plumbing only; the MIR is already
computed source-keyed. (iii) Args carrier per §8 item 1. (iv) S7 replicates
the `jit_c2_install_native.rs` zero-fallback + non-vacuity pattern
unchanged.

### Named uncertainties

- ctx object as typed AST unproven end-to-end (fn-value + value-call proven
  separately; object-with-function-field construction untested). If S1
  keeps a runtime ctx, add one smoke before committing ctx-consuming hooks
  to full-native.
- Async hook targets: `await` in main is VM-only (pre-existing
  named-fallback precedent `c2-async-clean-generated-method`) — hook-bearing
  async fns use the named-expected-fallback cell regardless of G6.
- If any S7 cell ever needs a pinned-seam fallback, the sound mechanism is
  classifier-driven trampolining (`jit_compatible=false` so call sites
  trampoline), NOT compile-failure demotion — a natively-referenced demoted
  body refuses the whole program at finalize (measured).
- Patched-run handler nativity is compile-proven, not execution-proven; S7
  zero-fallback cells must EXECUTE the handler path.

---

## 3. SPIKE-GENERIC — generic-target hooks today (sharpens C3-G8)

Fixtures run via `shape -- run` (default mode = JIT-attempt with interpreter
fallback), debug build.

| Fixture | Shape | Result today |
|---|---|---|
| g1 | `fn id<T>(x: T) -> T` + before/after, one concrete type (int) | **WORKS** (`[id] before` / `[id] after` / `42`); whole-program W36 deopt (wrapper call-site `return_kind` unproven) |
| g2 | same, called at TWO types (`id(42)`, `id("hello")`) | **WORKS** at both instantiations; hooks fire per call |
| g3 | generic + concrete second param `fn first_of<T>(x: T, n: int)` | **COMPILE ERROR** — the G7 heterogeneous-args hard error, verbatim: `cannot build annotation args for function 'first_of::f64::semantic_0000_c9e27998240f599227d1ea6a63f3fd2a': parameters have heterogeneous element types. Runtime annotation args require a single statically proven element type.` Fires PER-MONOMORPHIZATION, leaks the mangled internal name, anchors at the param (`n: int`), not the `@application` site |
| g4 | hooks READING `args[0]` and `result` at two types | **WORKS** with correct values at int and string — no silent corruption on the read path |
| g5 | args-MUTATING before (`[args[0] * 2]`), int-only call | **WORKS** (prints 42) |
| g6 | same mutating hook at int AND string | **COMPILE ERROR** — strict-typing array-literal element inference, verbatim: `cannot infer the element type of this array literal…`, anchored INSIDE the hook body (`<input>:3:5`), zero mention of the annotation, the target, or which instantiation (string) caused it |

**G8 sharpening conclusion.** The two failure modes today are (i) any
heterogeneous instantiated signature → mangled-name semantic error at the
wrong site (g3); (ii) type-specific hook + a second instantiation → a
strict-typing error deep in the hook body with no annotation context (g6).
The G8 blanket named rejection strictly improves on those but REGRESSES the
g1/g2/g4/g5 working class. The rejection wording must: (a) name both the
target and its generic signature at the `@application` site; (b) cite the
#59 monomorphization-origin re-arm as the lift condition; (c) carry the
positive twin ("usable on every concrete target"); (d) `docs/defections.md`
must record that pre-C3 partial generic support existed and is deliberately
withdrawn until #59.

**Feasibility signal:** today's machinery already re-checks hook bodies per
instantiation (g3's per-instance mangled name, g6's per-instance failure) —
per-specialization checking is mechanically native to this compiler; only
the diagnostics anchoring is missing.

Untested (named): wider mutation shapes (multi-arg homogeneous, result
mutation on generic targets); g5 covered int-only single-param.

---

## 4. SPIKE-AMBIENT — what fires today on ambient reference (defines [C0926])

| Fixture | Shape | Result today |
|---|---|---|
| a1 | hook references module-scope `let ambient = 7` (same module) | **WORKS** (`[a] ambient=7`) — ambient module-scope capture is a live path; whole-program W39 F1 deopt (`LoadModuleBinding` in hygienic fn, ADR-006 §2.7.14) |
| a2 | `pub let` inside a `mod` block | blocked by an unrelated rule: `module-level variable declarations currently require const` (script top-level `let` IS module-scope-allowed) |
| a2b | annotation + `pub const secret: int = 11` in `mod defs` | **WORKS** (`[a] secret=11`) — declaring-module const visible after splice; W39 deopt. The one today-working case with legitimate intent |
| a3 | annotation in `mod defs`; hook references a binding declared ONLY in the TARGET's module | **WORKS** (`[a] sees=9`) — NO HYGIENE: the hook body is spliced into the application module and resolves names there |
| a6 | shadow probe: `defs::secret = 11` AND target-module `let secret = 99` | prints `[a] secret=99` — the TARGET module SILENTLY WINS over the annotation's own module. **Headline [C0926] evidence**: an unrelated same-spelling user binding silently changes hook behavior |
| a4 / a4c / a4b | application-site local via nested fn (+ no-ambient control + config-from-local) | annotations on fn-local nested `fn` defs are **SILENTLY DROPPED** — no hook, no diagnostic; control (a4c) proves the drop is general, not caused by the ambient reference. Case inexpressible today; the silent no-op is itself a gap (§8 item 10) |
| a4d | config ARG is a runtime module binding (`@amb(chosen)`, `let chosen = 5`) | **WORKS** (`[5] fired`); generated wrapper contains `LoadModuleBinding` → W39 whole-program deopt (per-invocation config eval poisons JIT even for the wrapper) |
| a4e | mutate `chosen` between calls | `[5] fired` … `[6] fired` — **per-invocation config eval empirically confirmed** (the G7 legacy behavior ConstLift deletes; also proves today's config cannot participate in a specialization hash) |
| a5 | legit case: hook references only its config param (literal config) | **WORKS** (`[cfg] legit`); note even this whole-program-deopts (W36) |

**[C0926] boundary from the data.** Under C3-G4 (free identifiers in a hook
body that are not hook params or declared captures are rejected): the
a1/a2b/a3/a6 resolutions all become [C0926] rejections — quote a6 in the
rationale. a2b is the legitimate-intent case: the rejection sentence carries
the positive twin "declare it as a capture" (ConstLift'd CaptureClause
path). a4d/a4e becomes a rejection or const-evaluation under
config-as-ConstLift'd-declared-captures; killing it also removes the
`LoadModuleBinding`-in-wrapper JIT poison. a5 (config param) is the
surviving legit path.

**Cross-observation for G6/S7.** Every hook-bearing program observed deopts
whole-program today via two distinct SURFACE families: W36 (wrapper
call-site return_kind unproven — g1/g2/g4/g5, a4, a5) and W39 F1
(`LoadModuleBinding` in hook/wrapper — a1/a2b/a3/a6, a4d/a4e). The C3 typed
path removes both families for hook-bearing fns.

**Untested (named):** real two-file/project-mode imports (`from defs use
{ @amb }`) could route differently than inline `mod` blocks — the
a2b/a3/a6 hygiene results are script-mode-only evidence. Non-liftable
config values (function/reference as config arg) not probed — the ConstLift
never-liftable rejection wording needs its own probe (S3).

---

## 5. SPIKE-VMRED — the standing vmlib red (helpers.rs:8448)

**Test:** `compiler::helpers::frame_return_metadata_tests::runtime_before_hook_impl_stamps_declared_typed_array_parameter_prefix`
(`crates/shape-vm/src/compiler/helpers.rs:8448`; deterministic failure,
verbatim: `panicked at crates/shape-vm/src/compiler/helpers.rs:8374:32:
missing frame descriptor for compute___impl`).

**Root cause: hygienic-rename lookup miss — NOT typed-prefix erasure.**
Commit 761469cd (ADR009-E3 #19) replaced the user-spellable
`{name}___impl` registry name with an unspellable SOH-prefixed descriptor:
`annotation_hook_impl_name` (`functions_annotations.rs:3362-3367`, "former
`{func_name}___impl`") mints via `mint_hygienic_fn_name_stable`
(`helpers.rs:3584-3590`) rendered as `\u{1}hygienic:{hash}`
(`expansion_provenance.rs:523-528`). The test predates the rename
(introduced d652f664) and looks up the literal string. Probe evidence
(throwaway dump, restored): the compiled program contains
`\u{1}hygienic:36764e81…` with arity 1, `frame_descriptor=true`,
`slots=[Ptr(TypedArray)]` — EXACTLY the asserted value. **The product
invariant (declared typed-array param prefix survives hook-wrapping) is
GREEN at HEAD.** Sibling pin `statements.rs:8251-8256` was already
rewritten for the same rename; this test is the straggler.

**Pre-declared arithmetic (C3-G7).** Disposition confirmed: REWRITE, at the
S5/S6 pin-rewrite wave — not delete, and the red does NOT self-disappear
under legacy deletion (the literal spelling will still resolve to nothing
post-S6). The 7-name vmlib baseline carries this red unchanged through
S1–S4; the FAILED-name set goes 7 → 6 at the slice that rewrites the pin,
via rewrite, not deletion fallout. Do NOT fix now: the fixture's
`before(args, ctx)` homogeneous carrier is itself S6-deleted — a minimal
fix-now would pin doomed syntax.

**Successor-test sketch:** (a) locate the wrapped impl body BY ROLE
(hygienic provenance lookup keyed on `HygienicRole::AnnotationHookImplBody`
or its C3 successor — never by string spelling) and assert the frame-slots
prefix `[Ptr(TypedArray)]`; (b) per G4, assert the before-specialization
bound to `Sig (Array<int>) -> Array<int>` types args element 0 as
`Array<int>`. The stable invariant is "declared typed prefix survives hook
application" — if S1–S4 restructure the wrapper chain, the assertion moves
to whatever cell carries the declared param kinds. Also: split `frame_for`'s
conflated panic (`helpers.rs:8365-8375` merges name-miss and
descriptor-miss) so future genuine descriptor regressions in the module's
other 4 tests aren't masked.

Cross-note consistent with G6: the wrapper `compute` has
`frame_descriptor=true` but `slots=[]` (empty typed prefix) at HEAD.

---

## 6. C09xx census — allocation table + reuse candidates

Method: `rg -o 'C09\d\d' -uu` workspace-wide at d138a4e4, then per-code
classification of every product-code occurrence (mint = a message-string
allocation in non-test code).

| Code | Status | Mint site(s) (product code) | Class |
|---|---|---|---|
| C0901 | MINTED (C1) | `capture_plan/planner.rs:274` | declared capture never used |
| C0902 | MINTED (C1, reused C2) | `capture_plan.rs:231,245`; `comptime_fragments/checked_body.rs:276` | borrow-mode / reference capture rejection |
| C0903 | MINTED (C1) | `capture_plan/surface.rs:59` | capture clause outside comptime-generated code |
| C0904 | MINTED (C1) | `capture_plan.rs:299` | `move` on shared-ownership binding |
| C0905 | MINTED (C1) | `capture_plan.rs:256,325`; `planner.rs:213,243` | capture unresolved / ownership class unknown |
| C0906 | MINTED (C1) | `capture_plan.rs:287` | `move` on module-level binding |
| C0907 | MINTED (C1, reused C2) | `planner.rs:221`; `checked_body.rs:288` | duplicate capture declaration |
| C0908 | MINTED (C1) | `capture_plan.rs:335` | `share` on non-shared binding |
| C0909 | MINTED (C1) | `capture_plan/surface.rs:29` | foreign/fabricated generated-node provenance |
| C0910 | RETIRED — burned | none (deleted with U03 reparse route, C1 s5); absence-pinned at `tools/shape-lsp/src/generated_captures.rs:423`, `tools/shape-test/tests/lsp/generated_captures.rs:193,232` | NOT free — reuse would silently invert three assertions + the e2-close-report refuted-in-part disposition |
| C0911 | MINTED (C1/E2) | `capture_plan/query/aggregation.rs:164,276`; structured constant `capture_plan/query.rs:33` | generated-capture artifact conflict / MissingInferenceFact quarantine |
| C0912 | MINTED (C1) | `reference_flow/diagnostic.rs:31,79` | exact reference-flow conflict |
| C0913–C0921 | RESERVED-UNMINTED (C2's block, untouchable) | none — C0913/C0921 doc-only in `checked_body/battery.rs:94-99`; **C0914–C0920 have ZERO workspace occurrences** | reserved per battery.rs D4-reuse note; c3-decisions.md:98 confirms |
| **C0922–C0925** | MINTED (C2 s3/s4) | `async_drop_context.rs:165`; `statements.rs:7306`; `edit_transaction_guards.rs:73,86` | async-drop / edit-transaction families |
| **C0926** | **CONFIRMED UNMINTED-RESERVED for C3** | none — all 6 occurrences doc/comment (`e2-decisions.md:26`, `c3-decisions.md:50,96,128`, `AGENTS.md:47`, `battery.rs:94`) | free to mint as the C3 headline rejection |
| C0927–C0929 | MINTED (E2) | `comptime_builtins.rs:892`; `statements.rs:731`; `comptime_builtins.rs:1022` | extend_method splice / expr-form replace-body / source-string route removed |
| C0930 | MINTED (E1) | `functions_annotations.rs:121` (producer `resolve_param_id` :102) | directive names undeclared parameter |
| **C0931** | UNMINTED — **TRUE NEXT-FREE** | single occurrence = `c3-decisions.md:98` forward reference | — |
| C0932–C0999 | zero occurrences | — | free |

**Conclusions.** [C0926] confirmed free to mint. True next-free = C0931;
nothing above C0930 is minted or reserved by any track (E1 stopped at
C0930; E2-D5 block = C0927+ with C0926 carved out). `battery.rs:94`'s "C3
starts at C0926" is STALE as a next-free source (E1/E2 subsequently minted
C0927–C0930); only the empirical census is authoritative, and S5 must
re-confirm next-free at mint time (codes minted on unmerged sibling
branches would not appear in this worktree census).

**Reuse candidates — exact current sentences (S5's code+exact-sentence rule
needs these):**

- **[C0902]** (class: a borrow/reference crossing into a generated
  closure/body; `ShapeError::SemanticError`). C2 already set the reuse
  precedent at exactly the construction seam S1 will occupy
  (`checked_body.rs:276`): `[C0902] capture '{} {}' uses a borrow mode; a
  borrow that escapes into a generated body has no lifetime to check and is
  reserved until Shape has a closure-region story`. C1 mints at
  `capture_plan.rs:231` (declared-path borrow mode) and `:245` (value mode
  over a Reference-classified slot) carry the `ReferenceEscapeIntoClosure`
  spelling.
- **[C0907]** (duplicate capture). `planner.rs:221`: `[C0907] duplicate
  capture declaration for '{}'{}; each captured binding may be declared
  exactly once` (suffix names the aliasing prior spelling);
  `checked_body.rs:288`: `… may be declared at most once`. **One-word
  divergence** ("exactly" vs "at most") — a third producer in S1 must pick
  one sentence (candidate one-line alignment inside S1, not a new code).
- **[C0930]** (signature-indexed input miss; E1-D4 resolve-ONCE, single
  producer `resolve_param_id`): `[C0930] comptime `{directive_kind}`{from}
  on `{}` names parameter `{spelling}`, which the frozen signature does not
  declare; its parameters are [{}]` — directly reusable for CheckedTemplate
  signature-indexed input misses.

**Ambient/runtime-boundary sweep (S5 matrix inputs).** The nearest existing
ambient-capture diagnostic is UNCODED: the Wave-46 implicit-capture
rejection, single producer `implicit_capture_message`
(`capture_plan.rs:412`, fired from `surface.rs:74` and `planner.rs:293`):
`generated closure implicitly captures {captures}; generated captures must
be explicit (in generated function '{}', node {})` — no C09xx bracket. C3's
[C0926] is a DIFFERENT class (template inputs, not closure captures), so
minting C0926 does not collide; the S5 matrix should route capture-shaped
ambient cases to the existing capture-family codes (C0902/C0905/C0906) and
reserve C0926 for the template-input seam. Reverse direction
(comptime-only values escaping into runtime): `runtime_lift_rejection`
(`crates/shape-runtime/src/comptime_reflection.rs:807`) — per-schema named
arms, the sentence-style precedent for ConstLift's never-liftable
declaration-site rejections (C3-G5). Forward direction (runtime value
entering comptime): NO named code exists — only the C0003
generated-declaration envelope + the sanitizer fallback (`helpers.rs:1890`)
— C0926 is genuinely new; nothing to reuse for the headline class. Adjacent
reusable for the matrix: C0906 (module-scope ambient), C0909 (provenance
integrity at install), C0911 (evidence-conflict quarantine), C0905
(unresolved capture). The C000x family is a separate namespace — not
candidates.

**Infra caveat C3 inherits when minting.** All C09xx codes are
string-embedded bracket tags (no structured code field on `ShapeError`;
C0927–C0929 are bare `Err(String)` — e2-close-report.md:110-111, follow-up
:257). Only the LSP capture-plan surface has a structured `issue.code()`.
C0931+ minted as SemanticError-with-bracket follows the dominant
convention; routing the family through a coded-diagnostic path is a named
PRE-EXISTING follow-up (supervisor scope decision), not C3's.

---

## 7. Per-specialization-checking feasibility (S1's design input)

**Verdict: FEASIBLE.** The core machine exists as the monomorphization
pipeline; S1 composes, not builds.

### 7.1 What exists

- **C2 battery entry seam.** The battery is a MANIFEST
  (`checked_body/battery.rs:19-46`), not an engine. Rows 2–10 need only a
  `&FunctionDef` — `analyze_function_body` (`body_analysis.rs:13`)
  self-assembles its environment from ambient compiler state; emission
  checks ride `compile_function_inner`. Row 1 (type) is the whole-program
  analyzer and does NOT re-run per-function — the one real design fork
  (§7.3). All brackets inside the C2 InstallTransaction
  (`checked_body/mod.rs:197-239`), which C3 reuses per E1-D6b.
- **Per-instantiation checking already ships.** Definition time skips
  generic defs (`functions.rs:822-828`); instantiation time,
  `ensure_monomorphic_function_for_callsite` (`monomorphization/cache.rs:203`)
  does bounds-check → `substitute_function_def` → register →
  `compile_function` per instantiation = full battery rows 2–10 + strict
  emission-tier checking, with HARD failure (`cache.rs:414-418`), mono-key
  cache dedup, cycle detector, and the const-aware sibling
  `ensure_monomorphic_function_with_consts_for_callsite` (`:461/:475`)
  whose cache key incorporates const VALUES — the Dec-95 rule-6 spec-hash
  precedent S3's ConstLift needs.
- **Tuple binding is supported at both checking tiers.**
  `ConcreteType::Tuple` → `TypeAnnotation::Tuple`
  (`substitution.rs:120-122`); a bare `Args` param annotation substitutes to
  the tuple (`:203-220`). Analyzer: constant-index positional typing
  (`inference/access.rs:676`), destructuring bind (`items.rs:2963`).
  Emission tier: declared tuple annotation records the per-slot
  `ConcreteType::Tuple` (`type_resolution.rs:1429-1435`), `args[k]` at
  constant k resolves the proven per-position type
  (`expressions/mod.rs:2620-2650`), tuple unification at call-site type-arg
  resolution (`:1938-1944`).
- **The existing per-target specialization shape.**
  `compile_specialized_annotation_handler` (`functions_annotations.rs:4029-4136`)
  already synthesizes a fresh `FunctionDef` per (annotation, target) and
  compiles it through the ordinary pipeline — structurally the Sig-binding
  S1 needs; what changes is where types come from (bound Sig instead of the
  homogeneous args array) and error attribution (today anchors at
  handler.span, Decision 68, not the `@application` site).

### 7.2 Smallest composition (generic template body)

At the per-target seam: (i) build `sig_tuple = ConcreteType::Tuple` over the
target's params via `declared_annotation_concrete_type` /
`annotation_param_type_annotation`; (ii) call
`ensure_monomorphic_function_for_callsite(template_fn, &[sig_tuple], …)` —
substitution + registration + per-instantiation battery + hard-error +
cache + cycle guard all come free; (iii) wrap any `Err` in a NEW
application-site attribution error naming both signatures (precedent:
`directive_signature_type_error`, `functions_annotations.rs:3644`).
Concrete body = degenerate case: no body re-check (checked at definition
under its own signature); match-or-error is a signature comparison
(CallableDescriptor equality when both frozen, else
unification/structural equality) + the same two-signature application-site
error. Config captures enter as consts via the `_with_consts` entry point
(spec-hash rule 6 for free).

**Genuinely missing (all small):** (a) the application-site attribution
wrapper; (b) sig→`ConcreteType::Tuple` glue (~20 lines); (c) the
concrete-case comparison + message; (d) runtime pins for heterogeneous
tuples and tuple-in-return-position (see §8 item 1 — the carrier itself is
the real gap); (e) the §7.3 ruling.

### 7.3 Ruling needed for S1: checking-tier authority

Which tier is the "type-checked against the bound Sig" authority?
**Recommended: emission tier + MIR battery** (the monomorphization posture
— per-specialization cost is one `compile_function`, matches G4's "still
compile-time"). The alternative (whole-program analyzer re-run per
specialization) has exactly one precedent,
`recheck_directive_mutated_signature` (`functions_annotations.rs:3581-3660`,
FailFast re-analysis over a signature-patched program clone) and costs a
whole-program pass per application event. Battery row 1 is covered either
way for tuple-indexed bodies (emission-tier proof is strict — unproven
operand = compile error); analyzer-grade message quality inside template
bodies differs.

### 7.4 Sig-source constraint (binding, not optional)

Bind Sig TYPES from the target `FunctionDef`'s param TypeAnnotations +
`annotation_param_type_annotation` fallback — NOT by round-tripping the
freeze: `reconstruct_type_annotation` (`comptime_builtins.rs:521-624`, the
E1-D7 total inverse) NAMED-REJECTS Nominal/Record/Parameter (B4/B5
pending), so a struct-typed target param would fail the freeze round-trip
and generic templates over struct-param targets would reject spuriously.
Keep the frozen `CallableDescriptor` (`type_reflection/payloads.rs:83`,
derived PartialEq over 128-bit FrozenTypeIdentity) for Sig
IDENTITY/equality only. The `inference_facts.function_signature` →
`Type::to_annotation` fallback inherits the documented TypeVar→"unknown"
loss (CLAUDE.md, `core.rs:218`) for unannotated target params — keep the
`annotation_type_is_unknown` guards on the new path.

---

## 8. Changes to the slice plan (deltas the spikes proved vs c3-decisions.md)

1. **[S1, biggest — G4 carrier ruling needed BEFORE S1.]** The G4 "typed
   tuple `args`" carrier does not exist at HEAD: no tuple literal
   expression (`(1,2)` is a parse error), no `.0` index (parse error), and
   bracket tuple types are homogeneous-only by design (`heterogeneous tuple
   [int, string] is not supported … Use a struct instead` — named
   rejection); heterogeneous values would fall through to the legacy
   NewArray boxed path (`collections.rs:460-511`) with zero runtime pins,
   and tuple-in-return-position (`before` returns typed Args) is unpinned.
   Options: (a) grow language surface (tuple literals/index — ~8-file
   exhaustive-match fan-out, S4-adjacent scope growth), or (b) carrier
   substitution: per-param plain args (PROVEN native in SPIKE-JIT) or a
   generated struct. Whichever is chosen, S1 must pin the carrier's runtime
   end-to-end (vm+jit) BEFORE building on it. Emission-tier tuple
   DESTRUCTURING is also unverified (analyzer binds per-position; no
   compiler-side tuple destructure emission found) — G4's
   "indexing/destructuring" may need scoping to indexing first.
2. **[S1 ruling.]** Checking-tier authority per §7.3 (recommended: emission
   tier + MIR battery = monomorphization posture).
3. **[S1 binding constraint.]** Sig types from the AST/inference side;
   frozen CallableDescriptor for identity/equality only (§7.4 freeze
   round-trip gap until B4/B5).
4. **[S1.]** The genuinely-new piece is application-site error attribution
   naming both signatures (precedent `directive_signature_type_error`);
   today's errors anchor at handler.span / mangled mono-key names.
5. **[S1/S2 sequencing.]** Build the new path BESIDE
   `compile_specialized_annotation_handler` (it IS the G7 deletion target —
   self-as-f64, homogeneous args, name-keyed magic params); never extend
   `annotation_arg_array_element_annotation`.
6. **[G6→S7 confirmed SMALL, with the §2 qualification.]** The wrapper must
   be a generated typed AST FunctionDef through the ordinary pipeline;
   un-suppression of the legacy weave is measured-forbidden (silent VM≠JIT
   divergence). Plus: `mir_data` attach for hygienic impl emissions
   (plumbing); S7 cells must EXECUTE the handler path; async targets get the
   named-expected-fallback cell; any future pinned-seam fallback must be
   classifier-driven trampolining, never compile-failure demotion; add a
   ctx-object native smoke if S1 keeps a runtime ctx.
7. **[S5 diagnostics.]** Mint from C0931+ (re-confirm census at mint time);
   C0926 free for the headline; C0910 permanently burned; C0907's
   one-word divergence — pick one sentence when adding a third producer;
   route capture-shaped ambient cases to the existing capture-family codes
   and reserve C0926 for the template-input seam; the uncoded
   implicit-capture sentence needs a pin-verbatim-or-bracket decision
   BEFORE the S5 matrix (flagged so it isn't discovered mid-S5).
8. **[S5/S6 pin arithmetic.]** helpers.rs:8448 rewritten (by role, not
   spelling) at the pin-rewrite wave → vmlib FAILED set 7 → 6 names; do NOT
   fix earlier (would pin doomed homogeneous-args syntax); split
   `frame_for`'s conflated panic. S6's deletion wave should not "correct"
   battery.rs:94's stale historical comment without noting the E-track
   mints.
9. **[G8 wording + defections.]** Rejection fires at the `@application`
   site naming target + generic signature, cites #59 as lift condition,
   carries the positive twin; defections.md records the deliberate
   withdrawal of the pre-C3 partially-working generic class (g1/g2/g4/g5).
10. **[SURFACED — no G-ruling covers it.]** Annotations on fn-local nested
    `fn` definitions are SILENTLY DROPPED today (a4/a4c: no hook, no
    diagnostic; drop site untraced — parser vs compiler). C3 should either
    named-reject or explicitly support; needs a supervisor disposition and
    a drop-site trace before the S5 rejection matrix is finalized.
11. **[S3 input.]** Per-invocation config eval empirically confirmed (a4e)
    — today's config cannot participate in a specialization hash; ConstLift
    deletion of it also removes the W39 `LoadModuleBinding` JIT poison from
    generated wrappers. Non-liftable config values (function/reference as
    config arg) still need their own probe for the never-liftable rejection
    wording; sentence-style precedent = `runtime_lift_rejection` arms.
12. **[Baseline discipline.]** Several of-record reds sit squarely in C3
    territory (executed_extend_authority ×8, generated_method_runtime ×2,
    b6 callable-parameter ×2, frame_return_metadata ×1) — slices touching
    these paths diff against the §1 name-sets, not zero.

## Named uncertainties (aggregate)

- Baselines are single-run; the nested_exact pair is flap-prone (§1).
- SPIKE fixtures are script-mode, debug-build; project-mode imports and
  release-build message text unverified (§3, §4).
- VMRED role identification inferred from arity/slots/doc-comment match,
  not a mechanical hash confirmation (high confidence; a one-line debug
  print of `annotation_hook_impl_name` would make it airtight if S5's
  rewriter wants proof).
- Census covers this worktree only; unmerged sibling branches would not
  appear — S5 re-confirms next-free at mint time.
- Handler nativity under the patched run is compile-proven only; execution
  proof lands in S7.
