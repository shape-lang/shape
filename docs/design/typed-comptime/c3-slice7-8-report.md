# C3 #14 — Slices 7+8 report: JIT residuals, extern-C rejection, W9 re-target, LSP hover, book gate, close-out docs

Authority: `c3-decisions.md` (C3-G6 B+ / the S0 JIT soundness fence block /
C3-G13 / C3-G14 A′) + the S0 §2 named uncertainties + the S2 split
disposition (both fallback lines verbatim) + the S6 report §6 dark-window
ledger (incl. §6.4) + issue #14 DoD §7 + CLAUDE.md Forbidden Patterns and
the book-gate rule. Four workflow stages, one writer each, append-only.

**NOT in these slices (explicitly):** the final 3-lens panel, verify-merge,
and the merge — all the supervisor's, after this workflow.

## 1. The commit chain (append-only on `adr009/c3`, base `03f8b9a1`)

| Commit | Stage | Content |
|---|---|---|
| `d508bad0` | S7-1 | the async-hook-target named-expected-fallback cell (S0 fence) |
| `26780771` | S7-1 | gap-(a) inline-Object return-kind arm; cell 5 re-pinned to gap (b); #70 filed |
| `68d09186` | S8a+b | foreign-target hook-annotation loud rejection + the W9 `@json_schema` import re-target |
| `4a8a14b2` | S8c | LSP hover via the shared hook-install query surface — no parallel table |
| (this commit) | S8d/e/f | design-index CURRENT row, defections completeness, this report |

Book (separate repo, content-only): `shape-web` branch
**`adr009-c3-annotations`**, commit **`211fcc3`** — `annotations.mdx`
rewritten onto the typed surface. **Supervisor obligation surfaced: the
shape ↔ shape-web merge pairing** (the a3 merge and the book branch must
land together; the book's gate evidence is against the a3 binary). The
shape-web repo's other dirty files (`book-site/scripts/*.mjs`, other
`.mdx` pages) were NOT touched and remain uncommitted on the branch.

## 2. S7 — JIT residuals

### 2.1 Cell inventory (banked vs missing, measured at `03f8b9a1`)

10 cells banked in `bin/shape-cli/tests/cli/jit_c3_carrier_native.rs`
(S1a carrier trio; S2d API-installed single ZERO-FALLBACK + aggregate
named-expected-fallback; S3c composite-config + the two evaluate-once
pins; S4d sugar typed-config + sugar evaluate-once). Missing per the
G6/B+ obligations: the ASYNC cell (the S0 fence rule "async hooks =
named-expected-fallback") and the aggregate proof-gap family disposition.
The ctx-consuming cells the S2d parenthetical once listed are MOOT
post-S6/C3-G14 (no ctx on the typed surface — E4 #20/#68), recorded in
the module header. Result: 11 cells, `jit_c3_carrier_native` 10→11.

### 2.2 The async cell (`d508bad0`, cell 11) — measurements verbatim

Fixture `tests/smokes-jit-closure/c3-async-hook-target.shape`:
sugar-declared typed-config annotation (`before` + `after`) on an
`async fn` target awaited from `main`. VM: exit 0, stdout `600000` —
value-distinguishing per skipped hook (before-skip 200000, after-skip
599000, both-skip 199000 = the measured no-hook control, config-misread
202000; refuters derived in the fixture header). JIT: whole-program
fall-through with EXACTLY ONE loud line, measured verbatim at `03f8b9a1`:

```
[jit-fallback] function main failed JIT compile: Runtime error: JIT compilation failed: Main code contains unsupported constructs: JitPreflightReport { vm_only_opcodes: [Await], unsupported_builtins: [] }; running under interpreter
```

VM==JIT by fall-through; loud-flip semantics (zero lines = async lowering
kinded = the cell FAILS and flips to `assert_c3_fixture_reaches_native_jit`).
Fixture constraint (measured, hook-INDEPENDENT, recorded in the header):
`for i in 0..200` INSIDE an async fn stops with the pre-existing
`op_iter_done SURFACE: iter_kind=Ptr(Range) …` (phase-2c, ADR-006 §2.7.4)
with or without hooks — the driver uses the `while` spelling. This is a
pre-existing async-lowering surface outside S7's charter, surfaced here
for the supervisor (no issue filed by the stage).

### 2.3 The aggregate proof-gap family (`26780771`) — (a) fixed, (b) ticketed

The S2d split measured two gaps behind cell 5
(`c3-api-installed-hooks.shape`, VM==JIT==400600). Gap (a) — no
`TypeAnnotation::Object` return-kind arm — was pinned at the W36 identity:

```
[jit-fallback] function main failed JIT compile: Runtime error: JIT compilation failed: Route A surface-and-stop: SURFACE — direct call to `boost::tuple_i64_f64::c3_before_hook::a2::(int, number)` resolved to function index 199 but has no compile-time-proven FrameDescriptor.return_kind. W36 named-function callgraph requires a static return-kind proof before lowering the call-site destination; no runtime inference or Null fallback. ADR-006 §2.7.5.; running under interpreter
```

S7 closed (a) as a bounded kind-tracker completion
(`classify_type_annotation_metadata` top-level Object arm stamping
`Ptr(TypedObject)`/`Plain`; unit pin
`inline_object_return_stamps_typed_object_abi_and_plain_wrapper`; no
anonymous-object `ConcreteType` minted — B4/B5 fence respected). The
load-bearing proof was the execution differential: the pinned W36
identity stopped firing the moment the arm landed (loud-flip caught it).
The deopt then advanced to EXACTLY the S2d-probe-predicted gap-(b) line,
measured verbatim post-(a):

```
[jit-fallback] function main failed JIT compile: Runtime error: JIT compilation failed: MirToIR: unresolved direct field read `.a0` (field idx 0) lacks a statically proven typed-object byte offset and/or projected NativeKind. The JIT no longer falls through to the legacy `get_prop` property getter for `Place::Field` reads because that path reinterprets raw v2 typed-object carriers and can crash or diverge for object/trait snippets. Surface-and-stop: deopt to the bytecode interpreter until the field layout and field kind are proven at compile time.; running under interpreter
```

Gap (b) is real MIR-depth work — NOT attempted in-slice per the charter;
cell 5 RE-PINNED to the measured gap-(b) identity (still exit 0 both,
VM 0 / JIT exactly 1 line, VM==JIT==400600); **issue #70** filed carrying
both measurements verbatim + the flip-to-zero-fallback close condition.
Attribution boundary stands: the user-declared `type` spelling of the
same aggregate is zero-fallback native (cells 2/3) — only the
inline-schema spelling lacks the proof chain.

## 3. S8a — the extern-C rejection (`68d09186`)

Probes at `26780771` (pre-fix, the S6 §6.4 close-out measured first):
(A) `extern "C"` + `before(args)` compiled + ran with the hook a SILENT
no-op (printed `42` only); (B) `fn python` + `after(result)` likewise;
(C, scope fence) a COMPTIME-ONLY annotation's `error()` in `comptime
post` ALSO never fired on an extern target. Landed: a LOUD named
surface-and-stop rejection at the head of `compile_foreign_function`,
anchored at the `@application` span, firing on the first
application-order annotation whose compiled definition carries
declarative hooks (`sugar_post_handler.is_some()`). ONE producer
`sugar_lowering::foreign_target_application_rejection`; C3-G13 uncoded
message text + #60 routing note. The sentence, verbatim (with
`{target_descriptor}` = `extern "C"` / `foreign python` etc.):

```
annotation `@{ann}` on {target_descriptor} fn `{name}` is not applied — runtime hook templates weave ordinary Shape function bodies, and foreign-function targets have no typed hook surface yet (E4 re-implements hooks on foreign targets — see issue #68); apply @{ann} to an ordinary Shape function or remove it
```

Pins: 3 must-reject (exact sentence + anchored line), 2 positive twins,
1 scope-fence CONTROL. **OPEN DISPOSITION for the supervisor (the probe-C
finding, obligation carried from stage 2):** comptime-only annotations
(the `@json_schema` class) are ALSO silent no-ops on foreign targets —
deliberately NOT folded into the ordered rejection (undisclosed scope-move
refused; full text in the `docs/defections.md` 2026-07-21 S8a entry).
Options: (i) extend the same named rejection to ALL resolvable annotations
on foreign targets; (ii) run comptime handlers for foreign targets as an
E4-adjacent workstream; (iii) leave dark with a #68 note. The CONTROL pin
freezes the current boundary either way.

## 4. S8b — the W9 re-target (`68d09186`)

`scoped_contract_named_stdlib_annotation_import_enables_bare_json_schema`:
`from std::serde::derive use { @json_schema }` + application + generated
`User_json_schema()` output pinned exactly. The bare-import spelling
worked first try (no namespace fallback needed). The three @remote
`#[ignore]` rows (#68, E4's acceptance suite) and the 1 pre-existing red
untouched. `modules_visibility` 132→133 green.

## 5. S8c — LSP hover via the shared query surface (`4a8a14b2`)

Compiler: `HookInstallRecord` + `body_fn` (identity, never display) +
`specialized_sig` (captured at apply time via the C3-G10 attribution
renderer — r8 delimited, user spellings, never a mono-key); PUBLIC
`BytecodeCompiler::hook_install_query() -> Vec<HookInstallView>`
(display-safe projection — the C1 slice-4 `generated_symbol_query`
precedent); the S5a origin phrase extracted VERBATIM into the ONE shared
`template_body_origin` (the [C0926] gate consumes the same producer — the
wording can never fork). MEASURED red-first: sugar mints the polymorphic
TYPE PARAM hygienically too, so the raw declared-Sig rendering carries
`\u{1}` beyond the body-fn prefix — `display_safe_declared_signature`
substitutes the canonical sugar-surface spellings (`Args`/`R`). LSP:
hover routes through the `GeneratedQuerySession` precedent; application
hover appends the Installed-hooks section (template signature + capture
set — the #14 hover proof); body-fn hover renders the generic declaration
view (rows matched ONLY by the projection's `body_fn` identity). Pins:
shape-vm +2 (display-safety MACHINE PIN with built-in planted needle +
API-row twin), shape-lsp +2 (884/0, full-markdown no-SOH pins,
planted-needle exercised mechanically), st-lsp +4 (506/0: application
sig+captures, body-fn generic view, sugar origin phrase, negative
control). No hand-written parallel LSP table anywhere.

## 6. S8d — the book (shape-web `adr009-c3-annotations` @ `211fcc3`)

`book/book-site/src/content/docs/advanced/annotations.mdx` rewritten onto
the ONE typed surface (the prior page spelled the DELETED legacy surface —
untyped config + `before(args, ctx)` + the ctx/before-return-contract
machinery, all compile errors at a3 HEAD). Documented: typed config
params; declarative `before(args)`/`after(result)` + zero-param
observers; the typed `args` pseudo-tuple + per-specialization checking;
the sugar lowering onto `install`/`before_hook`/`after_hook`/`capture`
(G2); ConstLift domain + evaluate-once-per-application; async targets
(the loud named Await fall-through documented in-page); stacked order;
type-target `extend` + module-target typed `item_fn` carrier; the
rejection matrix as `expected-fail` fences (untyped config, generic #59,
nested #62, foreign #68, ambient [C0926], hook-bearing non-function
targets); imports re-targeted onto `@json_schema` (named import = bare
`@name`; namespace import = qualified `@derive::json_schema` —
MEASURED: bare-via-namespace does NOT resolve for stdlib modules at
HEAD); the dark-window note (ctx/HookDecision/@remote = E4 #68).

**The gate-runnable example (the book-gate rule):** the flagship fence —
`annotation audited(bump: int, tag: string)` with BOTH hooks on a typed
fn — `runnable=true expected="[math] before\n[math] after\n25\n"`,
zero-fallback native both tiers (probed directly + gated).

**Page gate: 16/16 PASS** (8 `expected=` runnable examples + 6
`expected-fail` rejection fences + 2 import rows) against the a3 release
binary (`cargo build --release` in the a3 worktree via the lane), via
`extract-shape-snippets.mjs` + `run-book-truth-gate.mjs --shape-bin`.
Extraction note: a CommonMark rule (no backticks in a backtick-fence
info string) silently dropped two `expected-fail` fences on the first
extract — fixed by removing backticks from the meta value; fence count
verified 16/16 in the manifest afterwards.

**FULL truth gate with the same binary: 572 runnable, 555 pass, 17 fail —
ALL 17 CLASSIFIED, zero new unattributed reds, zero divergence buckets:**

| Ledger | Count | Fences |
|---|---|---|
| #68 (@remote dark window, E4) | 7 | `fundamentals/modules.mdx` L50, L61; `stdlib/core/remote.mdx` L41, L77; `advanced/polyglot-distributed.mdx` L74, L213; `tooling/execution-server.mdx` L130 |
| #23 (F1 full-universe enablement — pages spelling the deleted untyped-config surface) | 10 | `advanced/comptime-annotations-cookbook.mdx` L206, L224, L245, L267, L285, L369; `advanced/comptime.mdx` L130; `advanced/content-addressed-bytecode.mdx` L347, L371; `fundamentals/modules.mdx` L97 |

Every red's stderr names either the C3 typed-surface rejection sentence
verbatim ("declares config parameter … without a type … C3-G7/S6") or
`Unknown annotation '@remote'` / a serve-fixture @remote application —
attribution was never unclear, so no main-binary comparison run was
needed. Per the charter, ONLY annotations.mdx was rewritten; the 10
untyped-config pages are #23's charter, the 7 @remote fences are #68's.

## 7. S8e — design-index + defections

- `docs/design/typed-comptime.md` `CheckedTemplate<Sig, Captures>` row →
  CURRENT / VM+JIT via ADR009-C3 in the B5/B6/B7 house style (carrier +
  per-spec checking S1/G10; API + sugar S2/S4 G2; ConstLift + rule-6 S3;
  rejection matrix + [C0926] S5/S8a; S6 one-implementation deletion +
  absence sentinel; JIT cells incl. the named-expected-fallback pins;
  LSP hover via the shared query; the book line). Named TARGET remainder:
  Dec-95 staging spellings (`hook.emit {}`, `body(captures){}`, `#ident`)
  = E-track second producer (dated user disposition 2026-07-20);
  ctx/HookDecision/@remote/foreign-target hooks = E4 #20/#68; #59; #62;
  template serialization follow-up. The `HookPlan` row stays as-is.
- `docs/defections.md`: three entries appended (S6 capstone/G14 A′ —
  @remote+@indicator cut, the 21-ignore disposition, ctx-pin
  RETIRE-vs-ignore with the symmetry alternative, both delete-direction
  judgment calls, W9 coverage-zero + re-target; S7 — async honesty +
  the (a)/(b) split with rejected alternatives; S8c+d — hover SOH via
  the one producer, the diagnostic-tier SOH residual, book scope bound
  to annotations.mdx with the #23/#68 ledger). Cross-checked line-by-line
  against the S6 report §§6–7; the S5 and S6-fixlet-round entries and the
  S8a scope-fence entry were already present.

## 8. Suite arithmetic (stage-1 baselines at `03f8b9a1` → this commit; lane, `-j1`/`--test-threads=1`)

Pre-declared additions across the slices: shape-vm +9 (1 gap-(a) unit pin
+ 6 S8a pins + 2 S8c pins), st-lsp +4, shape-lsp +2, cli +1 (the async
cell), modules_visibility +1 green. All landed; FAILED name-sets
byte-identical to baseline in every suite (final sweep, this stage):

| Suite | Baseline (`03f8b9a1`) | Final (this commit) |
|---|---|---|
| shape-vm `--lib` | 3501 / 6 FAILED (6-name) / 36 ignored | **3510 / 6 / 36 — FAILED set byte-identical** |
| annotations_runtime | 36/36 | **36/36** |
| annotation_targets | 24/24 | **24/24** |
| annotations_comptime | 116 / 10-name FAILED | **116 / 10-name identical** |
| comptime | 260 / 3-name FAILED | **260 / 3-name identical** |
| lsp (shape-test) | 502/502 | **506/506** |
| shape-lsp `--lib` | 882/882 | **884/884** |
| modules_visibility | 132 / 1 / 3 ignored | **133 / 1 / 3 — same red, same ignores** |
| cli_tests | 57/57 | **58/58** (316s; jit_c3_carrier_native 11, jit_generated_capture_native 9, jit_closure_capture_native 9, script_execution 8, jit_fallback_diagnostic_matrix 8, jit_c2_install_native 6, jit_fstring_format 4, tree 3) |

`just check-clean` exit 0 and `just check-no-dynamic` exit 0 at every
commit; refused-regex (space + `[ _-]` widened spellings) CLEAN over
every per-stage diff and over the full slice range (verified at this
commit). The `nested_exact` flap fired once in stage 2 (protocol run 3/1
over 4 isolated `--exact`) and once in stage 3 (isolated N=5 1/5 ok WITH
the diff, 2/5 ok at pristine `68d09186` via a working-tree stash
control — pre-existing nondeterminism, recorded, not of record); it did
NOT fire in this stage's final sweep.

## 9. New measured findings from this stage (surfaced, not fixed)

1. **Zero-param observer JIT deopt (proof-gap family sibling).** A
   sugar-declared zero-param `before()` observer runs green both modes
   (VM==JIT stdout, exit 0) but `--mode jit` deopts whole-program with
   exactly one loud W36 line — the minted hook-body fn has no
   compile-time-proven `FrameDescriptor.return_kind` (measured during
   S8d probing, release binary):
   `… direct call to `hygienic:0fb4…` resolved to function index 195 but
   has no compile-time-proven FrameDescriptor.return_kind. W36 … ADR-006
   §2.7.5.; running under interpreter`. Candidate: the observer's minted
   body carries no return annotation, so
   `classify_type_annotation_metadata` has nothing to stamp — the same
   family as the closed gap (a) (a Void/Null stamp is the analogous
   bounded completion). Needs an owner/disposition; the book's observer
   fence gates green regardless (stdout equality, loud stderr).
2. **Diagnostic-tier SOH leak.** The specialization-rejection family
   renders raw `hygienic:<hash>` mints in the template-identity clause
   and the declared-signature clause (measured on the zero-param-target
   rejection, the [C0926] sentence, and the G8 generic-target sentence).
   The S8c display-safe rendering covers the LSP hover tier; the
   compile-diagnostic producers still render mints. Disposition needed
   (extend the S8c display-safe projection to the diagnostic renderers);
   deliberately not fixed in a docs stage (defections entry).
3. **Namespace-import bare-name resolution (measured, documented
   as-is).** `use std::serde::derive` does NOT bind bare `@json_schema`
   at HEAD (`Unknown annotation`); the qualified `@derive::json_schema`
   resolves. The book documents the measured truth (named import = bare;
   namespace = qualified). Whether bare-via-namespace SHOULD bind for
   stdlib modules (the old page claimed it via graph re-exports; the W9
   namespace rows are @remote-ignored) is open for the supervisor.

## 10. Residuals with named owners

| Residual | Owner |
|---|---|
| Generic-target hook installs (deliberate withdrawal; re-arm per specialization origin) | #59 |
| Annotations on fn-local nested fns | #62 |
| ctx/HookDecision/@remote/foreign-target hooks + the 21-test `#[ignore]` acceptance suite + the 3 @remote W9 import rows | #68 / E4 #20 |
| Book full-universe enablement (10 untyped-config fences on 4 pages + everything else outside annotations.mdx) | #23 (F1) |
| Aggregate inline-Object MirToIR field-layout proof (cell 5 loud-flip pin) | #70 |
| Zero-param observer W36 return-kind deopt (§9.1) | issue #69/disposition |
| Diagnostic-tier SOH mint rendering (§9.2) | needs disposition |
| Comptime-only annotations silent on foreign targets (probe C) | supervisor disposition (§3) |
| C0902 provenance drop (2 `#[ignore]`'d reference-provenance pins) | issue #69 (S6 report §6.6) |
| `scoped_contract_snapshot_requires_explicit_import` red | pre-existing main-merge (stash-differential-proven) |
| String scalar-method whole-program deopt | STAGE-StringJIT (pre-existing, untouched) |
| Async range-iterator `op_iter_done` SURFACE inside async fns | pre-existing async-lowering surface (§2.2) |
| `nested_exact` flap | documented nondeterminism (protocol runs recorded) |

## 11. Forbidden-patterns audit of these slices

No fallback kept, nothing renamed, no feature flag, no vacuous pin in
either direction, no compile-failure demotion, no legacy un-suppression
(moot — the weave is deleted; the typed path is the only path and is
MIR-attached). Refused-regex (both spellings) CLEAN over the full range
`03f8b9a1..HEAD`. The probe-C widening, the observer-deopt cosmetic
dodge, and the diagnostic-renderer scope-creep were each considered and
refused with defections entries.
