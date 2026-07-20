# C3 #14 slice-4 report — typed config params + the sugar lowering onto the public API

Landed on branch `adr009/c3` atop `f1e76635` (the S3-close merge) as four
gated stage commits — `ae96c4c7` (S4a — #66 item 1: per-call-site capture
value typing) → `25e61831` (S4b — grammar: typed config params, syntactic
classification, declaration-site ConstLift check, typed injection) →
`e3eb42d3` (S4c — the sugar lowering onto the public API + G8/G12 through
the lowered path) → this close commit (S4d — sugar-equivalence proofs,
CLI zero-fallback smoke, classification record, full gates). Authority:
`c3-decisions.md` (C3-G0..G13, esp. G2/G3/G4/G5/G7/G8/G12),
`c3-slice2-report.md` + `c3-slice3-report.md` (the API + ConstLift seams),
GitHub issue #66 item 1 (the S4-opening obligation), `c3-slice0-report.md`
(baselines of record), CLAUDE.md Forbidden Patterns + ADR-006 at maximum
binding.

Slice-4 charter delivered, all four heads:

- **(a) #66 item 1 FIXED** — the public API genuinely supports
  multi-typed config captures (int+string, int+Array<int> in ONE handler,
  end-to-end; §1).
- **(b) Grammar** — `annotation retry(times: int, label: string)` parses;
  the ConstLift domain is checked AT THE DECLARATION (a non-liftable
  config type is a named error before any application; §2).
- **(c) The sugar lowering** — a TypedConfig annotation's declarative
  `before`/`after` block lowers COMPILER-SIDE onto exactly what the
  public API expresses, with ZERO private side-channels — proven
  structurally (E3) and behaviorally (E1/E2); G8 and G12 fire at the
  `@application` site through the lowered path (§3, §5).
- **(d) The G7-compliant transitional classification** — recorded
  prominently below; the 48 legacy pins stayed green and UNREWRITTEN all
  slice (their rewrite is S6's A-phase).

NOT in S4 (by plan): the S5 rejection matrix beyond what the lowering
structurally needs; any deletion (S6); LSP (S8); the aggregate-carrier
JIT proof gaps (S7's named follow-up).

---

## THE TRANSITIONAL CLASSIFICATION RULE (the ONE ratified G7 duality)

Every `annotation` definition is classified by a compile-time SYNTACTIC
rule into exactly one of two surface classes, decided AT THE DECLARATION
(before any `@application` exists):

- **TypedConfig** — the definition declares **>= 1 config parameter
  carrying a type annotation**. All-or-nothing: every config parameter
  must then be typed (mixed typed/untyped = the declaration-site
  rejection R2), and every declared type must lie within the C3-G5
  ConstLift domain (rejection R1, the ONE reused domain producer).
  Declarative `before`/`after` handlers LOWER onto the public comptime
  API (the S4c sugar) and never engage the legacy weave slots.
- **Legacy** — zero config parameters, or all parameters untyped.
  Declarative handlers keep the BYTE-UNCHANGED legacy runtime-hook weave
  until S6 deletes it.

**Zero-param ruling (resolved in S4 design): NO opt-in marker is
minted.** Zero-param definitions classify Legacy until S6; C3-G0 forbids
minting throwaway grammar that S6 must then delete. New-path e2e coverage
uses typed-config definitions (the mixed-type shape S4 exists to prove);
public-API installs inside zero-param definitions are already green
(sugar-matrix rows r2/r3/r9).

**Comptime `pre`/`post` handlers are CLASSIFICATION-INDEPENDENT** — they
run through the mini-VM on both classes; the S4b typed injection simply
gives TypedConfig handler params their declared annotations.

**NAMED S6 CLOSE:** S6 deletes the Legacy arm, the legacy weave fns
(`compile_specialized_annotation_handler`,
`specialize_annotation_runtime_handlers`, `compile_annotation_wrapper`),
and the untyped-param spelling, then rewrites the 48 legacy pins onto the
typed surface — after which every annotation (zero-param included) is
new-path for free.

**Mechanism:** sealed `AnnotationSurfaceClass { TypedConfig(..),
Legacy(..) }` in `statements/annotation_declarations/planner.rs` — the
evidence token is private to that one file (the C1 CaptureKind
"constructible in ONE file" precedent), `classify_annotation_surface` the
single producer. Pin-greenness by construction: the grammar REJECTED
typed config params before this slice, so no pre-existing program or pin
can classify TypedConfig (verified: zero
`annotation \w+\([^)]*:` hits under `tools/shape-test/tests/`).

---

## 1. S4a — #66 item 1: per-call-site capture value typing (`ae96c4c7`)

**The fix.** The `capture` comptime-builtin forwarder generates as the
ONE generic forwarder `capture<T>(name, value: T) -> __CaptureBinding`
(doc-commented named special-case in `comptime_builtin_forwarders()`,
`compiler/comptime.rs`). Capture values type PER CALL SITE via ordinary
generic-call inference + monomorphization — each mono body still calls
the variadic `__comptime__.capture`, so the registration-side KindedSlot
builtin is untouched and the string-name sentence (param 0 unannotated)
stays reachable. Mixed-type captures in one handler — the exact shape the
sugar's typed config params lower to — now compile and execute.

**Pin flips (weave.rs, ordered):** (1) the S3c probe FLIPPED green
(`mixed_capture_value_types_in_one_handler_execute_end_to_end`); (2)
int+string charter pin (140); (3) mixed across two installs (before int +
after Array<int>, 352); (4) rule-6 mixed-config identity (equal
(int,string) config SHARES, differing SPLITS, 33064096); (5) the #66
item 2 empty-array probe pin green UNCHANGED.

**Measured collateral completions** (general machinery,
`expressions/function_calls.rs`, pinned at the w27 tier): (a)
implicit-generic specialization no longer vetoes on unresolvable args at
ANNOTATED positions; (b) the explicit-generic inference hard error gained
the `deferring_uninstantiated_template_body` guard its implicit sibling
already had. One S3-era pin re-targeted (diagnostic-TIER change on the
unresolvable-static-type misuse class — flagged for supervisor review in
the S4a relay; the S3 execute-time domain sentence stays reachable via a
resolvable-typed out-of-domain value).

## 2. S4b — grammar, classification, declaration-site checks, typed injection (`25e61831`)

`shape.pest`: `annotation_def_param = { ident ~ (":" ~ type_annotation)? }`.
KEY DISCOVERY: NO new AST field — `AnnotationDef.params` is already
`Vec<FunctionParameter>`; the parser fills the existing
`type_annotation`, so the fan-out landed as signature threading + a
14-file consumer audit (all read name/handlers/defaults only). The
def-param carrier grew to `(name, Option<TypeAnnotation>)` pairs via ONE
derivation producer (`handler_resolution::annotation_def_params`); the
comptime injection loop stamps declared annotations onto handler params;
legacy defs carry `None` throughout (byte-equivalent).

R1 reuses `const_lift::annotation_within_lift_domain` at the declaration;
R2 fires on mixed params; the installer's S4b surface-and-stop for
TypedConfig weave slots was replaced by the S4c lowering. The G2
first-half proof landed hand-written: `annotation retry(times: int, tag:
string)` + a `comptime post` spelling ONLY public API executes mixed-typed
end-to-end (140240, per-application config, rule-6 split), with the
config-arg-mismatch loud-rejection twin.

## 3. S4c — the sugar lowering onto the public API (`e3eb42d3`)

ONE producer module `statements/annotation_declarations/sugar_lowering.rs`.

**THE BINDING RULES (the typed hook surface):**

| declarative form | minted body fn (C3-G3, module-scope-shaped) |
|---|---|
| `before(args) { body }` | `fn <minted><Args>(args: Args, <config in declared order with declared annotations>) -> Args { body verbatim }` |
| `after(result) { body }` | `fn <minted><R>(result: R, <config>) -> R { body verbatim }` |
| `before()` / `after()` | the F1 concrete OBSERVER `fn <minted>(<config>) { body }` |

- The Sig param keys the pseudo-tuple face on the DECLARED name — the
  user's spelling (`args`/`result`/any identifier) addresses it verbatim.
- The body is carried VERBATIM: no auto-appended return (the S1/S2
  classifier's named rejections fire exactly as for hand-written bodies).
- There is NO `ctx` on the typed surface (S2 F3; the runtime-hook ctx
  family is E4's charter) — `ctx` in a body is an ordinary unresolved
  identifier, loud.
- Hook-body fn names AND the polymorphic type param are HYGIENIC with a
  STABLE nonce (definition name, hook kind, handler index) — user
  shadowing structurally impossible; one identity per hook across handler
  runs, so Dec-95 rule 6 shares baked specializations across equal-config
  applications.

**The synthesized handler:** ONE `comptime post`-shaped handler per
definition, zero declared params (config arrives via the S4b typed
injection), body EXACTLY
`install(before_hook(<minted>, [capture("p1", p1), …]))` per hook in
declaration order. Both handler-resolution provenances append it AFTER
user comptime handlers (coexistence, user-first); minted defs join
`function_defs` (the same fn-AST-table contract hand-written body fns
ride) and resolve FIRST in all three `template_body_fn_lookup` closures.

**G8 through the sugar:** the EXISTING S2b generic-target producer fires
at the `@application` site (pinned e2e, zero registry rows). **G12:** the
parser desugar sites dropping nested-fn annotations were traced;
`Expr::FunctionExpr` grew `annotations: Option<Box<Vec<Annotation>>>`
(~15-file construction fan-out), and a TypedConfig annotation on a
fn-local nested fn is now the LOUD exact-sentence rejection (Legacy stays
byte-identically silent until S5 — pinned control).

## 4. S4d — the sugar-equivalence proofs (this commit)

The C3-G2 formal close: where the lowering calls internal seams for
compile-time efficiency, the equivalent public-API program produces
IDENTICAL behavior. Pins in `template_specialization/sugar_matrix_tests.rs`:

- **E1 BEHAVIORAL**
  (`s4d_e1_sugar_and_handwritten_api_twin_agree_behaviorally`): one
  program carries `retry_sugar` (sugar-declared) and `retry_api`
  (hand-written `comptime post` + module-scope `api_body`) with IDENTICAL
  hook body code and identical (int, string) config params, applied with
  equal config (3, "ab") to identical twin targets. Identical executed
  values (both 140; program total 140_140_220) AND registry-row
  agreement: hook kind (Before), capture names + LiftedConst renderings
  (`[("times","3"),("tag","\"ab\"")]`), the `::cfg#2` arity head, and the
  implicit application target per row.
- **E2 IDENTITY-SUFFIX**
  (`s4d_e2_cfg_identity_suffix_is_byte_identical_across_both_paths`):
  full symbol equality is IMPOSSIBLE (minted hygienic body-fn names
  differ), so the pin splits both specialized symbols at `::cfg#` and
  asserts the rule-6 identity tail (netstring segments from the ONE
  producer `template_specialization_key_suffix`) is BYTE-IDENTICAL for
  equal config, begins with the `2::` arity head, and DIFFERS for the
  (5, "xy") split control; the heads differ (documented impossibility).
- **E3 STRUCTURAL ZERO-SIDE-CHANNEL**
  (`s4d_e3_synthesized_handler_ast_is_public_api_only`): a unit pin
  calling the ONE lowering producer directly and whitelist-walking the
  synthesized handler AST — zero-param `comptime post` shell; every
  statement an expression; every call one of
  install/before_hook/after_hook/capture with empty const/named args;
  the hook's template a bare identifier naming THIS hook's minted body
  fn; every capture value argument a BARE config-param identifier equal
  to its name literal, in declared order. Any other node kind panics —
  the machine check that the lowering's output is literally a public-API
  program.

**CLI zero-fallback smoke** (`bin/shape-cli/tests/cli/jit_c3_carrier_native.rs`):

- **Cell 9** (`c3-sugar-typed-config-single.shape`,
  `c3_sugar_typed_config_single_runs_natively_both_tiers`): a
  sugar-declared MIXED (int, string) config annotation installs a before
  hook on a 1-ary SINGLE-carrier target (off the S2d aggregate-carrier
  gap), 200-call hot loop crossing T1@100. MEASURED ZERO-FALLBACK both
  modes, exact stdout 601000 (value-distinguishing: skip 199000,
  string-branch false 597000, times misread 203000), parse-based vacuity
  guard. The whole S4 stack — per-call-site capture typing → typed
  injection → lowering → ConstLift bake of BOTH the int and the string
  constant — is native. MEASURED FINDING: the string config must be
  consumed via an equality branch (the S1a cell-3 proven spelling); a
  scalar-returning string method (`tag.length()`) hits the PRE-EXISTING
  named STAGE-StringJIT whole-program deopt
  (`jit-string-scalar-method-deopt` — loud surface-and-stop, unrelated to
  the sugar path; recorded in the fixture header).
- **Cell 10** (`c3-sugar-config-eval-once.shape`,
  `c3_sugar_config_eval_once_warns_once_per_application` — the OPTIONAL
  second cell, ADDED): evaluate-once through the sugar via the S3c
  marker-count pattern on a COEXISTENCE definition (user `comptime post`
  carrying `warning("sugar-cfg-eval")` beside a declarative typed-config
  before hook, TWO equal-config applications). Marker count EXACTLY 2 in
  BOTH modes — per-application, never once-globally (equal config rule-6
  SHARES one baked specialization), never scaling with the 200-call loop;
  stdout 1194000; measured zero-fallback (nativity pinned by cell 9).

## 5. Rejection-sentence inventory (S4 producers; G13 string-tag, no C09xx minted)

Every rejection has an exact sentence and a positive twin (pin anchors in
parentheses).

- **R1 — declaration-site ConstLift domain** (planner.rs, reusing the ONE
  S3 producer): `` annotation `{name}` declares config parameter
  `{param}: {type}`, whose type is outside the ConstLift domain
  ({reason}); {CONST_LIFT_DOMAIN_SENTENCE} — declare the config parameter
  with a liftable type `` — `{reason}` per the S3 class arms (functions /
  references / "not a liftable type"). Twins: int / string / Array<int> /
  [int, int] / Option<int> compile (surface_class.rs). DISCLOSED
  RESIDUAL: fn-typed params render `{type}` as `any` (the shared
  `to_type_string()` catch-all — the same rendering the S3 finish()-time
  sentence ships; the class parenthetical carries the diagnosis).
- **R2 — mixed typed/untyped config params** (planner.rs): `` annotation
  `{name}` mixes typed and untyped config parameters; a typed-config
  annotation declares a type on every config parameter — annotate
  `{first_untyped}` ``. Twin: all-typed classifies TypedConfig.
- **R3 — typed-surface hook shape** (sugar_lowering.rs, fired by the
  planner): `` annotation `{name}` declares typed config parameters,
  which selects the typed hook surface, but its `{kind}` handler declares
  ({params}); typed-surface hooks are before(args) / after(result) /
  zero-param observers before() / after() — or remove the parameter types
  to stay on the legacy surface until it is deleted (C3-G7/S6) ``. Fires
  on >1 param, the magic `fn`/`ctx` single param, and variadic params.
  Twins: `before(args)` / observers lower and execute.
- **R3-family — lifecycle hooks on TypedConfig** (sugar_lowering.rs):
  `` annotation `{name}` declares typed config parameters, which selects
  the typed hook surface, but its `{on_define|metadata}` handler is a
  runtime-lifecycle hook with no typed-surface form yet (the runtime-hook
  context family is E4's charter; the legacy lifecycle surface is deleted
  at C3-S6) — remove the parameter types to stay on the legacy surface
  until it is deleted (C3-G7/S6) ``.
- **G8 through the sugar** (the EXISTING S2b producer, no new producer):
  fires at the `@application` site on generic targets, naming
  `(via @{ann}) on `{target}``, "withdrawn until #59 (the
  monomorphization-origin re-arm)", "apply @{ann} to a concrete
  function"; zero registry rows. Twin: the concrete-target charter pins.
- **G12 — nested-fn application** (closure-compile dispatch, TypedConfig
  only): `` annotation `@{ann}` on fn-local nested function `{fn}` is not
  applied — hook-template annotations on nested functions are not
  supported yet (#62); apply @{ann} to a module-scope function ``. Twin:
  module-scope target weaves; control: Legacy stays silently dropped
  (S5 owns the class).
- **Config-arg type mismatch** (contains-level per charter; S5 owns exact
  attribution): `@retry("x", "y")` against `times: int` rejects loudly
  naming the type mismatch. Twin: the matched-config charter pins.

## 6. Gates at the S4d close (the six-suite run, lane, `-j1`/`--test-threads=1`)

- `cargo check --workspace --all-targets`: zero errors (lane).
- shape-vm `--lib --test-threads=1`: **FAILED set == the S0 7-name
  baseline PLUS the KNOWN-FLAP member**
  `compiler::monomorphization::cache::route_tests::nested_exact_calls_close_outer_arguments_before_inner_compilation`
  (3417 passed; 34 ignored; twice). FLAP DISPOSITION (measured, surfaced,
  NOT normalized): the member is one of the S0 §1 named flap pair
  ("a future failure of either is a KNOWN FLAP"); per protocol it was
  rerun `--exact` — outcomes were NONDETERMINISTIC across identical
  isolated invocations of the same binary (1 green / 4 red observed;
  failure sentence `inner exact evidence must stay closed after leaving
  the outer frame: … call result type 'int' is not compatible with proven
  return type 'string'`, in 0.01s when red — runtime hash-order
  nondeterminism, not load). The S4d working diff is TEST-ONLY plus a
  `#[derive(Debug)]` and cannot interact; the single green isolated run
  with the diff in place confirms. The OBSERVED RED RATE IS HIGHER than
  the S4a record (green-twice there) — surfaced to the supervisor for
  disposition (rate shift vs. S0/S4a; possibly seed-sensitive to
  unrelated map growth). The other pair member stayed green.
- shape-test: `annotations_comptime` FAILED == the 10-name set (116
  passed); `comptime` FAILED == the 3-name set (261 passed);
  `annotations_runtime` 24/24; `annotation_targets` 24/24 (the 48 legacy
  pins green and UNREWRITTEN; `tools/shape-test/` ZERO-line diff across
  the whole `f1e76635..` slice range); `lsp` 502/502.
- shape-lsp `--lib`: 882/882.
- shape-cli `cli_tests --test-threads=1`: **57/57 = 55 prior + 2 new S4d
  cells**; per-module non-vacuity counts verified (jit_c2_install_native
  6, jit_c3_carrier_native 10, jit_closure_capture_native 9,
  jit_fallback_diagnostic_matrix 8, jit_fstring_format 4,
  jit_generated_capture_native 9, script_execution 8, tree 3).
- `just check-clean` exit 0; `just check-no-dynamic` exit 0.
- Refused-regex grep (both the space and the `[ _-]` widened spelling)
  CLEAN over the full `f1e76635..` slice diff.
- `functions_annotations.rs` hunk review over the full slice range: the
  legacy weave fns (`compile_specialized_annotation_handler`,
  `specialize_annotation_runtime_handlers`,
  `compile_annotation_wrapper`) appear in ZERO hunks; every hunk is the
  disclosed carrier threading (`def_param_names` → `def_params`), the
  minted-first `template_body_fn_lookup` precedence, the user-first sugar
  post-handler append, and the `function_defs` `or_insert` registration.
  The 48 pins + name-set equality are the behavioral halves of the claim.

Standing invariants held: ONE carrier / transaction / attribution
producer / identity-suffix producer / bake producer / ConstLift domain
producer / classification chokepoint; no C09xx minted; no serde on
template types; no new HeapKind; no `FieldType::Any` on the new path;
`KindedSlot` never in the typed VM↔JIT slot ABI; `LiftedConst` stays
compiler-tier data (ADR-005 §1).

## 7. Residuals (named, with owners)

1. **#66 item 2** (comptime empty typed-array literal) — OPEN; its loud
   probe pin (weave.rs) is green and unchanged.
2. **Legacy nested-fn silent drop** — byte-identical until S5's matrix
   owns the class (pinned control).
3. **tree-sitter-shape grammar** — typed annotation config params; named
   follow-up OUTSIDE this worktree.
4. **Aggregate-carrier JIT gaps (a)+(b)** — unchanged; S7's named
   follow-up with the S2d measurement.
5. **LSP typed-param hover** — S8 (the registry row already carries the
   substrate).
6. **R1 fn-type rendering** (`any` via the shared `to_type_string()`
   catch-all) — display follow-up with wide blast radius; supervisor
   disposition pending (S4b relay).
7. **Type/module/expression-target sugar residual** — the pass-2
   type/module/expression handler sites do not run the sugar post
   handler; an exotic explicit `targets:[type]` + declarative-hooks
   spelling would be a silent no-op on its type targets (S4c relay; same
   class as the S2b uncalled-generic residual; S5's matrix).
8. **STAGE-StringJIT scalar string-method deopt** — pre-existing, loud,
   named; blocks `.length()` spellings from zero-fallback cells (S4d
   measurement; unrelated to the sugar path).
9. **nested_exact flap rate** — surfaced in §6 for supervisor
   disposition.
10. **S4a diagnostic-tier change** on the unresolvable-static-type
    capture-value misuse class — flagged for supervisor review (S4a
    relay item 2).
