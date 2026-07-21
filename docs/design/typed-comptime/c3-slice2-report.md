# C3 #14 slice-2 report — the public comptime API for hook templates

Landed on branch `adr009/c3` atop `b34e366b` (the S1-close merge) as four
gated stage commits — `34fe10bd` (S2a capture seam + classifier capture-tail
+ builtins with stores and eager chokepoint construction) → `7b07a69b` (S2b
install directive + pass-2 apply seam + journaled install registry) →
`05a76d65` (S2c the typed-AST weave) → `4c95f9e8` (S2d the C3-G2 sugar
matrix + the API-installed JIT smokes). Authority: `c3-decisions.md`
(C3-G0..G13 + the slice plan), `c3-slice1-report.md` (the S1 seams),
`c3-slice0-report.md` (baselines + behavior matrices), CLAUDE.md Forbidden
Patterns + ADR-006 at maximum binding.

Slice-2 charter delivered: the PUBLIC COMPTIME API for hook templates — a
PRODUCER of the S1 `CheckedTemplate` carrier (never a second carrier),
installing through `specialize_template` + the already-open C2
`InstallTransaction` (never a second transaction), attributing through
`template_application_error` (one producer), with the C3-G2 sugar test
passed as the completeness gate: every capability the S4 declarative block
lowers to is expressible through this API with ZERO private side-channels.
NOT in S2 (by plan): ConstLift validation beyond the scalar seam (S3),
grammar/sugar (S4), the full rejection matrix + C09xx minting (S5), any
deletion (S6). The legacy hook machinery stays byte-unchanged beside the
new path.

## S2a — the API surface (`34fe10bd`)

Four public builtins, registered via `register_typed_function` inside
`create_comptime_builtins_module` beside `item_fn` (the E2 opaque-index
precedent), each with a paired `COMPTIME_BUILTIN_FORWARDERS` row:

- `before_hook(body, captures: Array<__CaptureBinding>) -> __CheckedTemplate`
  — `body` is a BARE MODULE-SCOPE FN IDENTIFIER transported by the
  emit-side rewrite `rewrite_template_hook_body_args` (`comptime.rs`; the
  type_ref identity-literal-transport precedent; the lookup is threaded as
  a PARAMETER into `execute_comptime_with_annotation_handler`, never
  ambient state). Construction is EAGER and full-chokepoint at builtin
  execute time: `CheckedTemplateBuilder::new(kind).body_fn(&def)?
  .captures(clause)?.finish()?` — the API produces the S1 carrier through
  the S1 typestate chokepoint, plus `validate_capture_value_types` on the
  lifted values.
- `after_hook(body, captures)` — same shape, `TemplateHookKind::After`.
- `capture(name: string, value) -> __CaptureBinding` — the C1 value-snapshot
  mode implicitly; borrow modes are structurally unconstructible here, so
  [C0902] stays reachable only as defense. The value rides the KindedSlot
  substrate and lifts EAGERLY through the S2 ConstLift seam (below).
- `install(template: __CheckedTemplate)` — pushes
  `ComptimeDirective::InstallHookTemplate{template_index}`; the target is
  IMPLICIT (the annotation's target), matching every existing directive.

Handles are the E2 opaque-index shape verbatim:
`typed_object_for_named_schema("__CheckedTemplate"/"__CaptureBinding",
[("index", …)])`; the reserved index-handle schemas live in
`builtin_schemas.rs` (disclosed S2a fold-in). An EMPTY captures array
literal is unprovable under strict typing at a call-arg position, so the
rewrite lowers `before_hook(f, [])` to UNSPELLABLE arity-1 nocapture
forwarders (producer-side only; disclosed S2a deviation 3). Builtin-layer
errors are G13 string-tag message text with the #60 routing note; S2 mints
NO C09xx codes.

### Store lifecycles (the two proven classes, mirrored exactly)

- `COMPTIME_TEMPLATE_BODY_FNS` (`Vec<FunctionDef>`, COMPILE-populated by
  the rewrite) clears at `execute_comptime_with_annotation_handler` ENTRY
  beside `clear_comptime_replace_bodies` — a pre-execute clear would wipe
  the compile-time stash.
- `COMPTIME_HOOK_TEMPLATES` (`Vec<BoundTemplate>`) and
  `COMPTIME_CAPTURE_BINDINGS` (EXECUTE-populated) clear at the pre-execute
  point beside `clear_comptime_checked_items`, so indices are fresh per
  run and the store survives until the next handler run on the thread —
  which is what lets the pass-2 driver resolve `template_index` AFTER
  `vm.execute` returns.

Pushes return the just-pushed index; reads clone.

### The `const_lift` S3 seam contract

`template_specialization/const_lift.rs` (module-scoped naming per
c3-decisions.md; never bare `ConstValue`, never
`comptime_concrete::ConstantValue`):

- `LiftedConst` — SCALAR variants only (Int/Number/Bool/String; no unit
  variant — `shape_ast::ast::Literal` has no Unit literal, so unit is
  named-rejected into the S3 domain sentence per the invariant's own
  conditional).
- `lift_capture_value(name, &KindedSlot) -> Result<LiftedConst, String>` —
  non-scalar values reject naming the S3 compositional domain with the
  positive twin ("pass an int, number, bool, or string capture value — the
  compositional domain … lands with S3 ConstLift"); non-finite numbers
  reject.
- `validate_capture_value_types` — value kind vs declared trailing-param
  type, naming both sides.
- `CaptureBindingPlan::CallSiteArgs(Vec<Expr>)` +
  `bind_captures_for_install` — the weave passes capture values as TYPED
  LITERALS at the wrapper's handler call sites, in trailing-parameter
  order. The specialization CACHE KEY is untouched: handlers stay
  value-generic and shared across installs with different capture values
  (pinned r1/S2c), so S2 builds ZERO of S3's Dec-95 rule-6 spec-hash and
  cannot resurrect the verify-1 injectivity bug class.

**The seam contract (stated verbatim in the module docs): S3 REPLACES this
module's domain** — compositional liftables (tuples/arrays/Option,
recursively), the G5 never-liftable named rejections, the heap-constant
`Baked` variant, and the Dec-95 rule-6 structural spec-hash — at this
fn/type boundary. Never a TODO comment. The S1 plan-guard (b) const-generic
fence stays in place.

### The signature contract (classifier capture-tail extension)

Captures are the TRAILING parameters of the body fn, matched to `capture()`
bindings by NAME as a bijection (delivery in parameter order). Sig arity =
`params.len() - |clause|`. `classify_template_sig` extended:
PolymorphicArgs = one plain type param + `params[0]` bare-`T` + bare-`T`
return + CONCRETELY-annotated trailing capture params (a bare-`T` capture
param is a named rejection); Concrete match-or-error compares
`params[0..sig_arity]` only. Concrete-with-captures bodies keep the S1
definition-compiled fast path. All S1 sentences byte-preserved for
zero-capture inputs.

## S2b — install + the pass-2 apply seam (`7b07a69b`)

`template_specialization/install_registry.rs::apply_install_hook_template`,
the `InstallHookTemplate` consumer at the authoritative pass-2
function-target phase. Per install, in order: resolve the per-run handle
(stale index = internal-error-shaped) → the C3-G8 generic-target rejection
(ONE shared sentence producer, three firing sites — see the disclosed
narrowing below) → the mixed-legacy rejection (one weave owner per target
until S6) → `specialization_target_from_def` → `specialize_template`
(first-statement open-transaction precondition holds by construction:
`compile_in_place` opens the journal before the inner driver) →
`bind_captures_for_install` → stage on the parameter-threaded per-target
accumulator + ONE JOURNALED registry row.

`hook_install_registry` (compiler-owned; the S8 hover/query substrate and
the r8 matrix row): annotation name, target name, hook kind, the template's
declared-Sig rendering, specialized symbol + function index, capture
names with `LiftedConst` renderings in delivery order, the `@application`
span. Rows are journaled through the open `InstallTransaction`
(`journal_record_hook_install_row`, pre-write-length undo) — a failing
compile leaves NO row. All other exhaustive-match directive sites got
additive arms: pre-pass no-op APPLY arms (install applies at pass-2 only,
never double-install) and the non-function-consumer named rejection.

## S2c — the typed-AST weave (`05a76d65`)

`template_specialization/weave.rs::materialize_hook_template_weave`, called
ONCE per target at the end of `execute_comptime_handlers` (after
`finalize_pending_original_body_shadow`, so it wraps the FINAL — possibly
replace-body-edited — definition): the target's final body moves under a
journaled unspellable hygienic shadow (`HygienicRole::TemplateWeaveImplBody`)
compiled through the ordinary NESTED `compile_function` (own mir-attach —
the slice-0 "(ii)" plumbing); the generated ordinary typed-AST wrapper
swaps into `func_def` under the target's name: `before` chain per
`MutationCarrier` (Single rebinds the one typed arg; Aggregate binds the
inline-schema aggregate to an Object-annotated local, then reads
`m.a0..m.aN-1` as the next call's typed args), direct impl call (awaited
for async targets), `after` chain threading the typed result — both chains
in application order, capture values appended as typed literals per
`CaptureBindingPlan::CallSiteArgs`. Bytecode AND MIR derive from the same
wrapped def (C3-G6 SMALL); the legacy weave + its suppression stay
byte-unchanged, with a defensive internal error refusing a legacy-classified
new-path target.

The S2c aggregate-kind guard (pseudo_tuple's rewrite face collects every
carrier write and proves each RHS's ConcreteType against the declared
parameter type; proven-divergent AND unprovable writes are named rejections
through `template_application_error`) CLOSES the S1 pending observation on
the new path — see the relay section below.

## S2d — the C3-G2 sugar-test completeness matrix (`4c95f9e8`)

The API-completeness gate. Each row is at least one EXECUTED fixture
written as an annotation handler using ONLY public spellings
(`template_specialization/sugar_matrix_tests.rs`, 8 pins, all
value-distinguishing). **Result: ZERO holes — no private side-channel
exists, and no API growth was needed.**

| row | capability the S4 sugar lowers to | fixture | public API calls |
|-----|-----------------------------------|---------|------------------|
| r1 | annotation config params → template inputs | `r1_config_param_enters_the_template_only_as_a_capture` | annotation-config binding (`annotation scaled(factor)` + `@scaled(3)`/`@scaled(5)`) + `capture("factor", factor)` + `before_hook` + `install`; 31051 executed; ONE shared value-generic handler; rows record `3`/`5` |
| r2 | `before` body | `r2_before_body_is_a_module_scope_typed_fn` | `install(before_hook(add_one, []))`; 50 executed (skip ⇒ 40) |
| r3 | `after` body | `r3_after_body_is_a_module_scope_typed_fn` | `install(after_hook(double, []))`; 80 executed |
| r4 | application to a target (target implicit) | `r4_application_covers_before_only_after_only_and_both` | three annotations × three targets: before-only 50 / after-only 80 / both 100 → 508100; 4 registry rows, all on the annotations' own targets |
| r5 | stacked annotations | `r5_stacked_annotations_compose_in_application_order` | two stacked annotations each installing before+after: before (1+10)*2 → impl 220, after (220+10)*2 = 460 (either order flip is value-distinguishing) |
| r6 | config-conditional hook selection | `r6_config_conditional_install_is_ordinary_control_flow` | `if enabled { install(...) }` — ordinary handler control flow; `@maybe_hook(false)` leaves its target UNWOVEN (5040; 1 registry row) |
| r7 | target introspection for composition | `r7_target_introspection_selects_the_template` | the EXISTING frozen descriptor, REUSED: `if target.params[0].type == "int" { install(before_hook(bump_int, [])) } else { install(before_hook(bump_num, [])) }`; 1050 executed; rows carry the per-target selected template. S2 adds NO new inspection builtin (the ruling) |
| r8 | hover data | `r8_registry_row_carries_declaration_and_application_views` | the S2b registry row: `template_sig == "tmpl <Args>(args: Args) -> Args"` (generic view at declaration), `specialized_symbol` contains the delimited `(int, number)` Sig (specialized types at application), captures + real `@application` span, symbol==index-name — sugar gets hover for free because it lowers to `install()` |

## The JIT smoke — MEASURED, split disposition (S2d; supervisor relay)

The S0-mandate smoke for the COMPILER-GENERATED path (the S1a cells were
disclosed hand-written proxies). Both fixtures: a real annotation handler
installs a polymorphic before (scalar capture) + concrete after through the
public API; 200-call hot loop crossing T1@100; parse-based vacuity guard.

- **SINGLE carrier (1-ary target) — ZERO-FALLBACK NATIVE.**
  `tests/smokes-jit-closure/c3-api-installed-hooks-single.shape` + CLI cell
  `c3_api_installed_hooks_single_runs_natively_both_tiers`:
  VM==JIT==402600, exit 0 both, zero `[jit-fallback]` lines both modes.
  The generated weave itself — wrapper + hygienic shadow + suffixed
  polymorphic specialized handler + capture-literal delivery — reaches
  native JIT. This discharges the S1a proxy caveat for the 1-ary
  installed-hook shape.
- **AGGREGATE carrier (the mandated heterogeneous 2-ary target) — DEOPTS;
  pinned as the C3-G6 Deep-contingency EXPLICIT NAMED-EXPECTED-FALLBACK
  with loud-flip semantics** (`c3-api-installed-hooks.shape` + CLI cell
  `c3_api_installed_hooks_aggregate_is_a_named_expected_fallback`):
  VM==JIT==400600 (whole-program interpreter, LOUD, never silent), VM zero
  fallback lines, JIT EXACTLY ONE line pinned to the named gap. Zero lines
  FAILS the cell and forces the flip to the zero-fallback form; a different
  fallback identity also fails. Never vacuous in either direction.

The stage prescribed STOP-and-surface on a wrapper deopt. Measured lines,
verbatim:

1. At HEAD (the pinned expectation):
   `[jit-fallback] function main failed JIT compile: Runtime error: JIT
   compilation failed: Route A surface-and-stop: SURFACE — direct call to
   `boost::tuple_i64_f64::c3_before_hook::a2::(int, number)` resolved to
   function index 199 but has no compile-time-proven
   FrameDescriptor.return_kind. W36 named-function callgraph requires a
   static return-kind proof before lowering the call-site destination; no
   runtime inference or Null fallback. ADR-006 §2.7.5.; running under
   interpreter`
2. Behind it (throwaway-probe-measured, probe REVERTED byte-clean before
   commit): with a sound `TypeAnnotation::Object` return-kind arm patched
   into `classify_type_annotation_metadata`, the next stop is
   `[jit-fallback] function main failed JIT compile: Runtime error: JIT
   compilation failed: MirToIR: unresolved direct field read `.a0` (field
   idx 0) lacks a statically proven typed-object byte offset and/or
   projected NativeKind. …; running under interpreter`

**Root-cause attribution (exact):** the G9 compiler-internal mutation
aggregate uses an INLINE `TypeAnnotation::Object` annotation (handler
return + wrapper local). Two proof gaps in the pre-existing kind/layout
provers: (a) `declared_annotation_concrete_type` has no Object arm →
`classify_type_annotation_metadata` stamps no `FrameDescriptor.return_kind`
on the aggregate-returning specialized handler → W36 fires at the wrapper's
call site; (b) MirToIR has no statically proven field layout for
`Place::Field` reads off an inline-Object-annotated local (the
USER-DECLARED `type` path is proven — the S1a aggregate proxy runs native —
only the inline-schema spelling lacks the proof chain). Neither fix is one
of the three forbidden classes (legacy un-suppression, compile-failure
demotion, classifier lies) — (a) is a sound one-arm kind-tracker
completion, (b) is real MIR-depth work — but per the stage's
deopt-contingency instruction NO product code was changed; both gaps are
S7's named follow-up carrying this measurement (the C3-G6 Deep-contingency
obligation). The slice-0 §4 cross-observation ("the C3 typed path removes
both families [W36/W39] for hook-bearing fns") is therefore TRUE for the
Single carrier and PENDING (a)+(b) for the aggregate carrier — a
supervisor disposition point: fold (a)+(b) into S7's charter, or dispatch
a dedicated proof-chain stage before S4 stacks sugar on the aggregate
path.

## Rejection inventory (S2 producers; exact sentences)

Builtin layer (G13 string-tag, #60 routing note):

- Body arg not a bare fn identifier: `` `before_hook` expects a bare
  module-scope fn identifier as its body argument, got {a string literal /
  a literal value / a closure / a call result / a property access / an
  expression}; code is code (C3-G3) — declare `fn my_hook(...)` at module
  scope and pass `my_hook` ``.
- Identifier resolving to no module-scope fn (also the comptime-local/
  nested narrowing sentence): `` `before_hook` body fn `{ident}` does not
  resolve to a module-scope fn in this compilation; declare `fn
  {ident}(...)` at module scope and pass `{ident}` (module-scope and
  previously generated module fns are supported; comptime-local/nested fns
  are not a template body in this slice) ``.
- Capture name not a string: `capture expects a string capture name (got
  kind {:?}); spell the binding capture("name", value)`.
- Capture value outside the S2 scalar domain (the S3-seam sentence):
  `` capture `{name}` holds a value outside this slice's ConstLift domain
  (kind {:?}); pass an int, number, bool, or string capture value — the
  compositional domain (tuples/arrays/Option of liftables, heap-constant
  baking, and the never-liftable named rejections) lands with S3
  ConstLift ``; non-finite numbers: `` capture `{name}` holds a non-finite
  number ({n}); … pass a finite number ``.
- Capture-name/trailing-param bijection ([C0930]-model sentence shapes,
  uncoded): binding matching no trailing param (`` capture `{}` on template
  body fn `{}` matches none of its {} trailing capture parameters … ``),
  trailing param with no binding, name matching a non-trailing param
  (`(beyond its {n} trailing capture parameter(s))` suffix), zero-param
  body (`declares no parameters`), non-identifier / unannotated / bare-`T`
  trailing capture params (`capture parameters are CONCRETE — annotate …`).
- Capture value kind vs declared param type: names both sides
  (`validate_capture_value_types`).
- Duplicate capture → `validate_capture_clause` [C0907] (reused verbatim,
  no third sentence producer).
- `install` non-handle arg: `install expects a __CheckedTemplate handle …
  construct one with before_hook(body_fn, captures) or after_hook(body_fn,
  captures)`.
- All S1 classification + pseudo-tuple sentences are now REACHABLE through
  the public builtins (13 S2a full-compile reachability pins).

Driver layer (`SemanticError` through `template_application_error` /
named producers, anchored at the `@application` span):

- C3-G8 generic target (ONE producer,
  `generic_target_install_rejection_message`): `` cannot install hook
  template `{body_fn}` (via @{ann}) on `{target}`: the target is generic —
  `fn {target}<T…>(…) -> …` — and hook-template installs on generic targets
  are withdrawn until #59 (the monomorphization-origin re-arm) lands
  (C3-G8); signature-polymorphic templates stay definable and usable on
  every concrete target — apply @{ann} to a concrete function ``.
- Non-function target: `` `install` directives are only valid when
  compiling function targets … apply the installing annotation to a
  function ``.
- Mixed legacy+new weave: `` …annotation `@{}` on the same target engages
  the legacy before/after runtime-hook weave, and a target has exactly one
  weave owner until the legacy machinery is deleted (C3-G7 / S6); move all
  of `{}`'s hooks onto the typed hook-template surface ``.
- Stale/out-of-range handle: internal-error-shaped.
- Destructuring-pattern target parameter (S2c named rejection beyond the
  invariant list): `` …parameter {} is a destructuring pattern, and the
  generated hook wrapper forwards parameters by name; bind the parameter
  to a plain name (destructure inside the body instead) ``.
- Aggregate-kind guard: proven-divergent write (`…proves type `string` …
  declares `int` … assign a `int` value…`) and unprovable write (`cannot
  prove the type of the value assigned to `args[0]` … provable at
  specialization`), both wrapped with BOTH signatures.
- Type-less target param + concrete match-or-error: the S1 sentences,
  unchanged.

## Supervisor relays (aggregated for disposition)

1. **Aggregate-kind divergence (the S1 pending observation): CLOSED on the
   new path** by the S2c guard (write-site proof, strictly stronger than
   the prescribed post-compile comparison — no post-compile record of
   proven per-field kinds exists; measured). The LEGACY path remains
   unguarded (S6-deleted) — residual acknowledged. The guard's
   unprovable-write class is surface-and-stop, a deliberate narrowing of
   template mutation RHSes to the provable domain.
2. **Comptime-local narrowing (disclosed):** C3-G3's "module-scope or
   comptime-local" wording is narrowed in S2 to module-scope +
   previously-generated module fns; comptime-local/nested body fns reject
   via the unresolvable-identifier sentence naming the module-scope twin.
   No sugar row needs comptime-local bodies; the E-track quote/splice
   surface is the named later producer.
3. **The aggregate-carrier JIT gap** (the smoke section above): two named
   proof gaps, no product code changed, named-expected-fallback pinned
   with loud-flip; needs a disposition (S7 fold-in vs a dedicated
   proof-chain stage BEFORE S4, since the sugar will make aggregate-carrier
   hooks a one-line spelling).
4. **C3-G8 firing-site narrowing + residual gap (S2b, standing):** an
   UNCALLED generic target in a single-module unit is a silent no-op
   (the handler never completes anywhere today); S4's static lowering or
   S5's rejection matrix can close it statically.
5. **S2a residuals (standing):** the nested-array-literal
   `pending_variable_typed_array_kind` leak (pre-existing, recorded in the
   S2a test doc-comment); the nested_exact vmlib flap member measured
   --exact-failing at the b34e366b base (phase flip, not S2-caused — green
   again in this run's full suite, consistent with flap behavior).

## Test inventory at `4c95f9e8`

S2 in-module unit tests: `const_lift.rs` 10; `checked_template.rs` +11
capture-tail pins (S1's 15 updated in-territory to the full-chain shape);
`comptime.rs` rewrite pins 6; store-lifecycle pins 3; S2a full-compile
PUBLIC-API reachability pins 13; `install_registry.rs` 9 (identity +
direct-execution proofs, verify-1 refuter through the API, G8, rollback);
`weave.rs` 15 (all EXECUTED end-to-end, value-distinguishing);
`sugar_matrix_tests.rs` 8 (the C3-G2 gate). CLI: 5 cells in
`jit_c3_carrier_native.rs` (3 S1a proxies + the S2d Single zero-fallback
cell + the S2d aggregate named-expected-fallback cell). Fixtures:
`c3-api-installed-hooks-single.shape`, `c3-api-installed-hooks.shape`.

## Gates at `4c95f9e8` (the S2d full six-suite re-run)

- shape-vm `--lib` `-j1`: FAILED set == the S0 7-name baseline EXACTLY
  (3306 passed; the nested_exact flap pair green this run).
- shape-test `annotations_comptime`: FAILED == the 10-name set (116
  passed). `comptime`: FAILED == the 3-name set (261 passed).
  `annotations_runtime` 24/24. `annotation_targets` 24/24. `lsp` 502/502.
- shape-lsp `--lib` 882/882.
- shape-cli `cli_tests` `--test-threads=1`: 52/52 = 50 prior + 2 new S2d
  cells; per-module non-vacuity counts verified (jit_c2_install_native 6,
  jit_c3_carrier_native 5, jit_closure_capture_native 9,
  jit_fallback_diagnostic_matrix 8, jit_fstring_format 4,
  jit_generated_capture_native 9, script_execution 8, tree 3).
- `just check-clean` exit 0; `just check-no-dynamic` exit 0;
  `cargo check -p shape-vm --all-targets` zero errors (lane).

Standing S2 invariants, held across all four commits: one carrier
(`CheckedTemplate`), one transaction (the open `InstallTransaction`), one
attribution producer (`template_application_error`); the legacy weave
byte-unchanged; the 48 green annotation pins untouched; no grammar
changes; no serde on template types; no C09xx minted; no new HeapKind; no
`FieldType::Any` on the new path; `KindedSlot` never in the typed VM↔JIT
slot ABI; forbidden-pattern and refused-regex discipline clean.

## Fix round 1 — findings and fixes (append-only atop `4c95f9e8`/`2263f269`)

Two review lenses ran over `34fe10bd..2263f269` (the G2-gate sugar-
completeness lens, verdict FAIL; the seam/lifecycle lens). Fixes land
append-only; the gated stage hashes are never amended.

### F1 (BLOCKER, G2 hole) — the OBSERVER template form

The round-1 G2 gate refuted S2d's "ZERO holes": observer hooks on
zero-param targets and `after` hooks on void targets had NO public-API
spelling (`specialize_polymorphic_before` empty-param rejection; concrete
before arity-0 rejection; after void rejection), while the surviving
declarative surface green-pins exactly those shapes (>= 6 of the 48 pins,
list below), and the S1 report had EXPLICITLY deferred the void-return
revisit TO S2 ("surface-and-stop; S2 revisits") — undischarged. Fix per
C3-G2's own rule ("if desugaring ever needs a capability the API does not
expose, the API must GROW"): the **OBSERVER form** — a CONCRETE template
body with ZERO signature parameters and no/void return (trailing capture
parameters still allowed; `fn note() { ... }`, `fn tagged(tag: int) { ... }`
+ `capture("tag", …)`). Offered to `before_hook`/`after_hook` it
specializes on ANY target (target-uniform BY DESIGN — one lowering of an
args/result-independent declarative block serves every target, no
per-target branching in S4's sugar): the weave calls it at its chain
position with ONLY its capture values; the target's args/result thread
through UNTOUCHED. Zero-param targets thereby gain a `before` spelling and
void targets an `after` spelling — the named holes close. The observer
predicate is disjoint from every mutation form (a mutating before declares
>= 1 signature param; a result-threading after declares exactly one + a
return), so no template classifies both ways; a zero-param VALUE-returning
body is NOT an observer (return would be silently dropped) and keeps the
existing rejections. All four zero-param/void rejection sentences now carry
the observer positive twin. DESIGN DISCLOSURE: the reviewer's sketch was
"arity-0 PolymorphicArgs + After-void unit-observer"; the landed shape
covers both holes with ONE concrete form and no monomorphization/
pseudo-tuple/empty-aggregate machinery — polymorphic-before on 0-ary
targets stays a rejection (its twin names the observer). Fix-agent design
choice under the G2 mandate, flagged for supervisor/user ratification.
Sugar matrix grows row r9 (`r9_observers_cover_zero_param_and_void_
targets`); observer EXECUTION is proven by error-injection (an erroring
observer body fails the woven program; the green control is the
non-vacuity twin) since observers have no data-flow observable.

### F2 (HIGH) — after-chain order: REVERSE application (wrapping/onion)

S2c threaded BOTH chains in application order; the surviving declarative
surface's stacked after hooks run INSIDE-OUT (wrapping.rs
`stacked_after_hooks_transform_result_in_order`: the annotation nearest
the fn is innermost — before chain outer→inner, after chain inner→outer),
and no C3 ruling ordered a semantics change, so S4's lowering would have
silently changed stacked-hook results (12 → 11 on the legacy pin's
fixture). Fix: the weave threads the after chain in REVERSE application
order — one iteration-order change (`weave.rs`), giving the coherent onion
model (first-applied annotation = outermost wrapper). Updated pins: weave
`stacked_after_installs_thread_the_result_in_reverse_application_order`
(40 → 30), sugar r5 renamed `r5_stacked_annotations_compose_as_wrapping`
(460 → 450). A dated user ruling preferring application order flips this
single iteration order back (S6 would then rewrite the legacy pin's
value instead).

### F3 (MEDIUM) — `ctx` disposition row (NAMED OPEN DISPOSITION)

The typed hook-template surface has NO `ctx` parameter (deliberate: G5
makes function values never-liftable; the impl shadow is unspellably
hygienic, so no capture-based spelling can exist), but the LEGACY runtime
hook bodies receive `ctx` and the green pins use it: injection.rs:142-163
(`before_hook_passes_ctx_info`, prints ctx) and before_after.rs:229-255
(`ctx_target_calls_original_impl_from_after_hook` — the §4.1.5
`ctx.target` original-impl call "WF-2C's @remote hard-depends on").
c3-decisions.md fences ctx.state/HookDecision to E4 and says "`ctx.target`
stays per the E3-S4 ruling". DISPOSITION ROW (default derived from the E4
fence, needs supervisor/user ratification): **E4 owns the runtime-hook-body
context family** (HookDecision, failure/retry, ctx.state — ctx.target is
the same runtime-context family for hook BODIES); therefore the two
ctx-dependent pins CANNOT be rewritten onto the typed surface by S6 —
S6's deletion arithmetic must carry them as E4-blocked items, and if E4
has not landed a typed replacement at S6 entry, S6 must surface for a
user ruling BEFORE deleting the machinery implementing ctx (never a
silent capability loss).

### F4 (MEDIUM-LOW) — type-changing after hooks: RULED withdrawal, now named

`After = (R) -> R` per C3-G4. The green pin wrapping.rs:32-49
(`after_hook_wraps_result_in_string`, int result → string) is therefore
INEXPRESSIBLE on the typed surface — a RULED withdrawal (G4), previously
unnamed. Named here for the rejection inventory + the S6 pin-rewrite
arithmetic: that pin is rewritten-with-capability-loss or dropped by dated
disposition at S6.

### F5 (LOW) — whole-args observability, named for S6

`print(f"{args}")` (function_target.rs `annotation_on_multi_param_
function` uses the pattern; the pin does not assert that line) has no
new-path spelling: bare `args` in value position is a named rejection and
f-string interpolation is the documented non-scanned boundary. Per-element
reads cover the uses; named for S6's rewrite arithmetic.

### Lens-2 F1 (MEDIUM) — install-handle SNAPSHOT resolution (stale-handle-across-runs)

CONFIRMED and fixed. The pass-2 directive loop resolved each
`InstallHookTemplate` index LAZILY per directive, while applying an
EARLIER directive in the same list can trigger a NESTED handler run (a
polymorphic `specialize_template` rides the mono pipeline into the full
`compile_function`, which re-enters `execute_comptime_handlers` because
annotations survive substitution; an `ExtendItems` compile does the same)
that CLEARS + REPOPULATES the execute-populated stores — the later
install's stale index then missed (misdiagnosed internal error) or, worse,
resolved to the NESTED run's template and installed the WRONG one
SILENTLY. Fix: `snapshot_install_hook_template_handles` — every install
handle batch-resolves to its `BoundTemplate` at directive-loop ENTRY,
before ANY directive applies (the `take_comptime_directives` value-snapshot
discipline extended to the handles the directives carry);
`apply_install_hook_template` now takes the resolved `&BoundTemplate`,
never a live store read. PROVEN-BITING REFUTER
(`nested_handler_run_during_processing_does_not_shift_install_handles`):
an annotated polymorphic template body fn whose @noise handler pushes TWO
templates during the nested specialization compile; with a throwaway
lazy-resolution probe (reverted byte-clean before commit) the refuter
fails exactly as derived — value 1090 (the nested run's Sig-compatible
`h_noise2` silently installed) vs the correct 180 — proving both the
nested-run mechanism and the silent-wrong-install failure mode. Store
lifecycle docs updated (the "intact until the next per-run clear" sentence
now names the mid-processing clear).

### Lens-2 F2 (LOW) — module-target install rejection arm now fires

`module_target_install_rejects_with_the_function_twin` — the
`process_comptime_directives_for_module` arm's producer + positive twin
fired end-to-end (`@hookann() mod demo { … }` with `targets: [module]`),
the sibling of the type-target test.

### Lens-2 F3 (LOW) — G8 prefix-walk edge, SURFACED only

`generic_origin_of_specialized_name` walks `::` prefixes and cannot
distinguish mono-rename suffixes from module qualification: a CONCRETE
module-qualified `foo::bar` whose module name collides with a registered
top-level generic `foo<T>` would false-positive the G8 rejection — LOUD
and fail-closed (wrongly-attributed sentence naming `foo`), never a silent
accept. Whether a module and a generic fn can share a name in one unit is
unverified; if so, the walk needs mono-key-aware segmentation. Doc-comment
at the fn names the edge; residual, not fixed.

### Lens-2 F4 (NIT) — handler-local shadowing of the body fn identifier

`let my_hook = 3; before_hook(my_hook, [])` previously bound the MODULE fn
silently, inverting ordinary shadowing. Fix: the emit-side rewrite tracks
handler-local binding names (`let` patterns + `for` binders) and rejects a
shadowed body-fn reference with a named sentence + positive twin (rename
either side). DISCLOSED NARROWING: the tracked set is a CONSERVATIVE flat
set — names accumulate lexically and never pop at scope exit (a local in
one branch flags a later use; over-approximation is only ever the LOUD
rejection), and match-arm/closure-param bindings are outside the set
(closure interiors are not recursed by this rewrite at all). Pinned unit +
end-to-end.

### S6 pin-rewrite arithmetic (the F1/F2/F4/F5 ledger)

Now-expressible via observers (rewrite normally at S6):
before_after.rs:174-198 (`before_hook_with_empty_params`),
wrapping.rs:137-163 (`annotation_wrapping_void_function`),
function_target.rs:11-31 (`annotation_on_simple_function`),
other_targets.rs:74-93 (`targets_declaration_function_on_function_works`),
other_targets.rs:166-187 (`annotation_on_module_item` — a fn-target pin
despite the name). E4-blocked (F3): injection.rs:142-163,
before_after.rs:229-255. Ruled-withdrawal (F4): wrapping.rs:32-49.
Boundary-named (F5): function_target.rs multi-param `{args}` uses.
Stacked-after value pins keep their legacy values under F2 (wrapping.rs:
52-81 stays 12).

### Fix-round-1 gates

`cargo check -p shape-vm --all-targets` clean (one PRE-EXISTING
unused-import warning at comptime_builtins.rs test scope, present at
HEAD). template_specialization + checked_template + rewrite + builtin
filters: 148/148. Full shape-vm `--lib` `--test-threads=1`: FAILED set ==
the S0 7-name baseline + the DOCUMENTED `nested_exact` vmlib flapper
(S2d residual 5) — flap re-confirmed NONDETERMINISTIC on the fix tree's
single binary (`--exact` run thrice: FAILED, FAILED, ok), so not
fix-caused. shape-test `annotations_runtime` 24/24 and
`annotation_targets` 24/24 (the 48 legacy pins untouched and green);
`annotations_comptime` FAILED == the 10-name set (116 passed); `comptime`
FAILED == the 3-name set (261 passed). shape-cli `cli_tests` `jit_c3`
filter 5/5 (both S2d cells + the 3 S1a proxies; the aggregate cell still
pins EXACTLY ONE named-expected-fallback line). `just check-clean` exit 0;
`just check-no-dynamic` exit 0; refused-regex grep over the full diff
clean. Suites NOT re-run (disclosed): lsp / shape-lsp (surface untouched),
the remaining cli_tests modules (fixtures untouched; compiled by
check --all-targets) — per the blast-radius verification discipline.
