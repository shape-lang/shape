# E4 #20 — Slice 3 report (D2-C: delete the always-empty lifecycle-ctx carrier + loud rejection)

Charter: **E4-D2** (`e4-decisions.md`), issue **#20** (cross-ref **#68**). Slice 3
executes decision **E4-D2 → D2-C "cleanest-C"**: the lifecycle-annotation `ctx`
carrier (`on_define`/`metadata` handlers) was always empty — `{state: {},
event_log: []}` with zero readers on every firing path — so it is **deleted
outright** (no empty-`{}` residual). Because a lingering `ctx` param then
degrades to a silent-null `PushNull`, and per the standing user ruling **a named
rejection must fire LOUD — a silent no-op is the worst possible state**, the
deletion is paired with a **broad, pre-inference, LSP-visible rejection** of any
lifecycle-handler param that is not the `target`/`fn` descriptor.

The typed per-invocation context (`BeforeContext<State>`) returns with the
HookDecision protocol under **#68** — it is NOT this slice.

Gate baseline of record: **re-captured fresh at S3's own base `85785aa7`** (S1+S2
shifted counts — ann_comptime is 117/10 at HEAD, not S0's 116/10). FAILED-name
sets, never raw counts.

## 0. Commit identity

- Branch `adr009/e4`, base **`85785aa7`** (S2 close), append-only — S2 is NOT
  amended/rebased/reset; S3 is one new commit on top.
- **S3 scope (surgical):** 5 deletes + 1 rejection + 6 new pins + 2 incidental
  fixture edits + this report + the `e4-decisions.md` S3-shipped line. The dead
  Rust `AnnotationContext` runtime family reap is DEFERRED (ruling OQ2) → ticket
  **#78**.
- A commit cannot contain its own hash; the S3 hash is recorded in the close
  relay (and may be appended here by a follow-up line, per the S2 pattern).
- `shape-web` is READ-ONLY for S3 (D2-C moves no runnable book fence). The book
  truth-gate was re-run as a regression sweep only; no book file was staged.

## 1. The collision verdict — recorded as RATIFIED: NO COLLISION

Scout-executed and supervisor-ratified (2026-07-23). Dropping the installer
`inferred_handler_parameter_type` `ctx` arm and the `"ctx"` emission arm touches
**only** the runtime-lifecycle (`on_define`/`metadata`) path. The comptime
`pre`/`post` ctx surface is a structurally independent path
(`execute_function_comptime_handler` → positional typing: `idx==1` → comptime ctx
= `{module_path, file}`); it never calls `lifecycle_function` and never consults
`installer.rs`. The two ctx types are disjoint (lifecycle has `state`/`event_log`;
comptime has `module_path`/`file`). Re-proven by execution at S3 (§4).

## 2. Supervisor rulings (BINDING, 2026-07-23)

- **OQ1 — rejection breadth: BROAD.** Lifecycle handlers accept ONLY the
  descriptor param (`target`/`fn` — exactly the names the emission arm +
  `inferred_handler_parameter_type` recognize), as one param or zero. A param
  literally named `ctx` gets the SPECIFIC E4-D2/#68 sub-message; any other
  non-descriptor name gets the generic "unknown lifecycle parameter" message.
  Faithfulness constraint: reject exactly the names that currently resolve to
  `PushNull`; do NOT newly reject a working descriptor. Proven by execution (§4).
- **OQ2 — AnnotationContext reap: DEFER.** Not reaped in the D2-C commit; filed
  as ticket **#78**. `TargetOwner` (`annotation_context.rs:66`, ~13 live prod
  refs incl. E3/E4 extend-owner resolution) is LIVE and untouched by S3.

## 3. Work order executed (5 MUST-DELETE + 1 MUST-ADD)

All "sole caller / sole consumer" claims were grep-confirmed before deletion.

| # | file | symbol | action |
|---|---|---|---|
| 1a | `compiler/functions_annotations.rs` | `emit_annotation_runtime_ctx` (fn) | deleted whole fn (sole caller was the `"ctx"` arm) |
| 1b | same | `emit_empty_annotation_event_log` (fn) | deleted whole fn (sole caller was 1a) |
| 1c | same | `"ctx" =>` arm in `match param_name` | dropped (ctx now falls to `_ => PushNull`) |
| 2 | `compiler/statements/annotation_declarations/installer.rs` | `inferred_handler_parameter_type` `if name == "ctx"` arm | dropped; `fn`/`target` arm + `object_field` kept |
| 3 | `compiler/post_inference_verify.rs` | `SchemaNamePrefix("__annotation_ctx_")` whitelist row | deleted; `__inline_obj_` row KEPT (the real E0900 clearer) |
| 4 | same | `positive_annotation_handler_ctx_passes` test + doc | deleted (the row's sole consumer) |
| A | `compiler/statements/annotation_declarations/planner.rs` | new rejection in the `OnDefine \| Metadata` validation block | added; fires pre-inference, once per declaration, at the handler span |

**Fire mechanics.** The rejection lives inside the `matches!(&handler.handler_type,
OnDefine | Metadata)` block (the loop over `definition.handlers`), so it never
reaches `ComptimePre`/`ComptimePost` handlers — the comptime-ctx surface is
structurally out of reach. It iterates `handler.params` (an
`AnnotationHandlerParam` list) and rejects the first whose `.name` is not
`"target"` or `"fn"`. Variadic runtime-handler params are already rejected
upstream (planner.rs, `is_runtime_handler` variadic guard), so this arm only sees
fixed params. The in-code follow-up marker `E4-D2 ctx-removal` ties the ctx
sub-message to this decision for grep discoverability.

### The two diagnostic sentences (exact, executed)

ctx-specific (param literally named `ctx`):

> Annotation '{name}': the '{on_define|metadata}' lifecycle handler takes only
> '(target)'. The 'ctx' parameter was removed in E4-D2 — the always-empty
> lifecycle ctx ({state: {}, event_log: []}) had no reader. The typed
> per-invocation context returns with the HookDecision protocol (issue #68).

generic (any other non-descriptor name):

> Annotation '{name}': unknown '{kind}' lifecycle handler parameter '{param}'.
> Lifecycle handlers receive only the 'target' descriptor.

## 4. Behavior differential — the loud-rejection proof (executed, base vs after)

Base = binary at `85785aa7` before edits; After = rebuilt binary with S3 edits.
Same six fixtures.

| fixture | base | after | verdict |
|---|---|---|---|
| `on_define(target, ctx){return 1}` | exit 0 (silent-fire) | **exit 1 LOUD** — ctx-specific msg (E4-D2 + #68) | rejection fires |
| `on_define(target, foo){return 1}` | exit 0 (silent no-op) | **exit 1 LOUD** — generic "unknown ... parameter 'foo'" | broad closes the footgun |
| `on_define(target){return 1}` | exit 0 | **exit 0** | descriptor path preserved |
| `metadata(target, ctx){return 1}` | exit 0 | **exit 1 LOUD** — ctx-specific msg (metadata + #68) | both kinds covered |
| `comptime post(target, ctx){ctx.module_path}` | exit 0 | **exit 0** | TWIN GREEN — no collision |
| `comptime pre(target, ctx){target.name}` | exit 0 | **exit 0** | TWIN GREEN — no collision |

**Over-rejection check: PASS.** The ONLY newly-rejected fixtures carry a
null-degrading param (`ctx`/`foo`); the target-only handler still compiles and
fires (exit 0). No currently-green handler with a working descriptor is newly
rejected. The comptime-ctx twins are untouched.

## 5. Blast radius — the BROAD-ruling collateral (honest addendum to spec §4.A)

The spec's §4.A ("only one in-tree breakage") was written **before** the OQ1
BROAD ruling. BROAD newly rejects any lifecycle handler declaring `ctx` (or any
non-descriptor name). A tree scan found the ctx-declaring lifecycle handlers;
only two live in gate-relevant compiling test code, both with **incidental**
`metadata(target, ctx)` whose bodies use only `target`:

- `crates/shape-vm/src/compiler/statements.rs` ::
  `test_definition_lifecycle_targets_reject_expression_target` — tests
  expression-target rejection; its intended error ("not a definition target")
  fires *after* my per-handler block, so BROAD would have intercepted it.
  **Fix:** dropped the incidental `, ctx` → `metadata(target)`. Same test intent,
  stays green.
- `crates/shape-vm/src/compiler/functions.rs` ::
  `test_expression_annotation_rejects_definition_lifecycle_hooks` — tests the
  application-time "definition-time lifecycle hooks" rejection; the declaration is
  planned (and would hit my block) before the application error.
  **Fix:** dropped the incidental `, ctx` → `metadata(target)`. Stays green.

Everything else that mentions ctx-declaring lifecycle handlers is a doc/comment
(`annotation_context.rs` module doc, `shape.pest`, `functions.rs` AST doc,
`core_types.rs` field docs), a **parse-only** shape-ast test
(`parser/tests/advanced.rs::test_annotation_def_with_on_define` — asserts the AST
param list, never compiles → unaffected), or the LSP's own `validate_annotations`
path (`tools/shape-lsp/src/diagnostics.rs::test_validate_annotations_with_defined`
— a separate lightweight validator that does NOT run the compiler planner →
unaffected). The gate-relevant shape-test suites contain **no** lifecycle
handlers at all (only comptime + before/after), so their FAILED sets are
untouched.

## 6. Gate table (real numbers; judged by FAILED-name SETS)

Every suite re-run at S3 base (baseline) and after edits. Verdict per suite.

| suite | baseline (S3 base `85785aa7`) | after S3 | FAILED-name-set verdict |
|---|---|---|---|
| `shape-vm --lib` | 3520 pass / 7 fail / 36 ign | 3525 pass / 7 fail / 36 ign | **UNCHANGED** failed set = 6 stable + the permitted flap `nested_exact_calls_close_outer_arguments_before_inner_compilation`; +5 pass = 6 new pins − 1 deleted test |
| `post_inference_verify` (module) | had `positive_annotation_handler_ctx_passes` | 20 pass / 0 fail | name-set change EXACTLY {−`positive_annotation_handler_ctx_passes`, +`s3_inline_object_any_still_passes_verify`}; the 4 loud-rejection pins live in the `statements` module |
| `annotations_comptime` | 117 / 10 / 0 | 117 / 10 / 0 | **UNCHANGED** (8 `executed_extend_authority::*` + 2 `generated_method_runtime::*`) |
| `comptime` | 260 / 3 / 0 | 260 / 3 / 0 | **UNCHANGED** (2 `b6_*` + `hash_tracer_does_not_disturb_formatted_strings`) |
| `annotations_runtime` | 36 / 0 / 0 GREEN | 36 / 0 / 0 GREEN | **UNCHANGED** green |
| `annotation_targets` | 24 / 0 / 0 GREEN | 24 / 0 / 0 GREEN | **UNCHANGED** green |
| `just check-clean` | exit 0 | exit 0 | only pre-existing warning `unused import super::*` @ `comptime_builtins.rs:3481` (NOT S3 — that file is untouched) |
| book truth-gate | standing 557/573, 16 reds | see §8 residuals | D2-C moves no runnable fence |

The 6 new pins (all PASS):
`s3_lifecycle_ctx_param_is_loud_rejected`, `s3_metadata_ctx_param_is_loud_rejected`,
`s3_lifecycle_nondescriptor_param_is_loud_rejected`,
`s3_lifecycle_descriptor_only_installs_on_define_and_metadata`,
`s3_comptime_ctx_twin_stays_green_after_lifecycle_ctx_removal` (statements module),
`s3_inline_object_any_still_passes_verify` (post_inference_verify module).

## 7. Positive twins that stayed green (collision-free anchors)

The comptime-ctx surface — `comptime post(target, ctx)` / `comptime pre(target,
ctx)` using target reflection — stayed green in BOTH the binary differential (§4,
exit 0) and the ann_comptime suite (§6, unchanged 10-name set). The whole
collision-free verdict rests on these, and they did not move. A dedicated pin
(`s3_comptime_ctx_twin_stays_green_after_lifecycle_ctx_removal`) locks it in.

## 8. Residuals (honest)

- **Dark book fences (S0 risk #6).** The before/after `ctx.state` fences in
  `advanced/annotations.mdx` "Dark Window" and
  `advanced/comptime-annotations-cookbook.mdx` were already dark/red pre-S3 (the
  before/after weave was deleted at C3-S6); D2-C changes no runnable fence.
  Doc-prose cleanup of those dark fences is out of S3 code scope — flag for close
  coordination, not a gate.
- **Deferred AnnotationContext reap (OQ2 → #78).** The dead Rust runtime-dispatch
  family (`annotation_context.rs` dead symbols + `context/registries.rs` no-op
  stubs + the `AnnotationRegistry` disposition) is a separate surface needing its
  own build + gate; `TargetOwner` must be preserved.
- **`__inline_obj_` row's stale reason string.** Its `reason` still carries a
  `§4.D.10-emission` / `__annotation_ctx_*` clause; trimming is cosmetic (dispatch
  reads only `rule` + `permanent`) and explicitly NOT in D2-C scope. Left
  untouched to keep the commit surgical.
