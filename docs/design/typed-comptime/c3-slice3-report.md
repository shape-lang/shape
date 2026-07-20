# C3 #14 slice-3 report — ConstLift: compositional domain, heap-constant bake, spec-hash rule 6

Landed on branch `adr009/c3` atop `8f8b3594` (the S2-close merge) as three
gated stage commits — `cf62997b` (S3a compositional domain core + LIVE
declaration-site domain check, dark-wired value path) → `4d9c9e28` (S3b
rule-6 structural spec-hash + heap-constant bake — the atomic delivery
flip) → `732ac770` (S3c evaluate-once + W39/LoadModuleBinding proofs +
the composite-config zero-fallback CLI cell). Authority: `c3-decisions.md`
(C3-G0..G13, esp. C3-G5 + the §Naming rule), `c3-slice2-report.md` (the
const_lift S3 seam contract), `c3-slice1-report.md` (§Verify-round-1
finding 1 — the injectivity bug class), `c3-slice0-report.md` (§1
baselines, §4 a4d/a4e, §8 item 11), CLAUDE.md Forbidden Patterns +
ADR-006 at maximum binding.

Slice-3 charter delivered, all five heads:

- **Domain (C3-G5):** liftable = `int`/`number`/`bool`/`string` + arrays/
  tuples/`Option` of liftables RECURSIVELY; unit named-rejected into the
  domain sentence; the Dec-95 never-liftable classes reject with named
  class arms at the `capture()` API AND at declaration-site
  (`CheckedTemplateBuilder::finish()`), every producer carrying the closed
  domain + a positive twin (G13 string-tag, no C09xx minted).
- **Bake (evaluate-once + heap constants):** capture values are
  comptime-evaluated ONCE at `capture()` and baked by the ONE producer
  `const_lift::bake_captures_into_def` as prologue constants inside the
  specialized handler, riding the ESTABLISHED per-function constant pool +
  array/Option literal emission — no second constant store; the compiler
  never emits the host-injection-only `Constant::Value(KindedConstant)`.
- **Rule 6 (spec-hash):** lifted-constant identity enters the
  specialization key AND symbol with STRUCTURAL equality
  (`structural_key_segment`, netstring discipline) under a `::cfg#{count}`
  arity head — the S1/S2 injective identity EXTENDED, #64-aware; equal
  config SHARES a specialization, different config splits, zero-capture
  identity BYTE-IDENTICAL to S1/S2.
- **Evaluate-once proven (S3c):** the observable comptime side effect
  (`warning("cfg-eval")`) counts exactly once per @application in the
  subprocess run — never once-globally, never scaling with a 200-call hot
  loop; the S0 §4 a4e legacy per-invocation contrast stays byte-unchanged
  beside the new path until S6.
- **W39 killed on the new path, pinned (S3c):** the specialized handler
  and the generated wrapper contain ZERO module-binding loads (the exact
  S0-named `LoadModuleBinding` 0x52 plus the emitter-dead typed
  0x182..=0x18C family), with a genuine-module-read non-vacuity control;
  the composite-config CLI cell is ZERO-FALLBACK native both tiers.

NOT in S3 (by plan): grammar/typed config params on the annotation block
(S4), the rejection matrix beyond lift-domain rejections (S5), any
deletion (S6), the aggregate-carrier JIT gap (S7's named follow-up). The
legacy weave (`compile_specialized_annotation_handler`,
`specialize_annotation_runtime_handlers`, `compile_annotation_wrapper`,
the `functions_annotations.rs` hook regions) is BYTE-unchanged across the
whole slice range (`git diff 8f8b3594 -- …/functions_annotations.rs` is
empty; `tools/shape-test/` diff empty — the 48 annotation pins untouched
and green).

## S3a — the compositional domain core (`cf62997b`)

`template_specialization/const_lift.rs` became the S3 domain core, at the
exact fn/type seam S2 left:

- `LiftedConst` grew `Array(Vec<LiftedConst>)` / `Some(Box<LiftedConst>)`
  / `None` with STRUCTURAL `PartialEq`/`Eq` — `Number` equality is
  f64 BIT-PATTERN equality (`-0.0` distinct from `0.0`: sound
  over-distinction, disclosed; non-finite values reject at lift, at every
  depth, so NaN never reaches equality).
- `lift_value` is the recursive C3-G5 lift: exact-kind scalar cascade →
  kind-witnessed composite arms (`Null`→`None`; `Ptr(Option)` payload
  recursion by borrow, zero retains; `Ptr(TypedArray)` via the elem-type
  stamp walk on the `typed_array_descriptor_lift_rejection` precedent —
  I64/F64/BOOL/STRING/nested-TYPED_ARRAY arms; the S3b-added
  `lift_option_typed_object` arm for the L5 `__Option` runtime carrier) →
  never-liftable class arms (functions, references, runtime handles incl.
  Future/Atomic, compiler descriptors incl. the `__CheckedTemplate` /
  `__CaptureBinding` / `__CheckedItem` handles) → the fail-closed
  out-of-domain arm naming the kind.
- `annotation_within_lift_domain` LIVE at `finish()` through the single
  `classify_template_sig` chokepoint: a trailing capture parameter with a
  fn-type names "functions", a `Borrow` names "references",
  nominal/other types reject as "not a liftable type"; bare `&T` capture
  params are caught by the pre-existing S2 plain-by-value arm (pinned;
  the Borrow arm is reached via nested positions like `Array<&int>`).
- `matches_annotation` went compositional (empty array matches any
  `Array<T>`; `Tuple` length + positional; `Some`/`None` vs `Option<T>`;
  scalar-vs-Option strictness = MISMATCH, decided-and-pinned);
  `to_expr` projects arrays/`None`/`Some(...)` onto established literal
  emission; `render()` grew display-only composite forms.
- `structural_key_segment` landed unit-proven (dark until S3b) with the
  in-doc injectivity argument and all six charter refuter classes, each
  with a flat-join non-vacuity control.

## S3b — the atomic delivery flip (`4d9c9e28`)

One append-only commit by design (composite acceptance + identity + bake
+ delivery re-plumb cannot split per-commit-green):

- `lift_capture_value` delegates to the recursive `lift_value`; the S2
  scalar-domain landing-point sentences are deleted with their tests.
- `TemplateSpecializationPlan` restructured to `{ pseudo_tuple:
  Option<PseudoTuplePlan>, captures: Vec<(String, LiftedConst)> }` — one
  walker, no traversal fork.
- **Identity:** `template_specialization_key_suffix(type_args, captures)`
  appends `::cfg#{count}` + one netstring segment per value in delivery
  order, identity-locked on mono key AND specialized symbol (`cache.rs`);
  zero captures append NOTHING — zero-capture keys/symbols BYTE-IDENTICAL
  to S1/S2 (all S1 identity pins + the `colliding_flat_tuple` refuter
  pass untouched; byte-stability unit pin added).
- **Bake:** `bake_captures_into_def` (ONE producer) strips trailing
  capture params and prepends `let mut {name}: {annotation} =
  {value.to_expr()}` prologue lets in delivery order, called in
  `ensure_monomorphic_function_impl` after `resolve_pseudo_tuple`, before
  register+compile. Mutability probe: Shape by-value params ARE
  assignable (`functions.rs:1941-1949`), hence `let mut` — a capture that
  the body assigns keeps parameter semantics (shadowing pinned).
- **Routes:** PolymorphicArgs carries values on the plan;
  PolymorphicResult-with-captures departs the shared entry at `[R]`
  (pinned; zero-capture path byte-unchanged); concrete/observer-with-
  captures ride with `type_args=[]` behind the `template_plan`-gated
  declares-no-type-params seam (`ensure_baked_concrete_specialization`);
  rollback covered by the S1c evict fold-in (rollback-rerun pin).
- **Re-plumb:** `specialize_template` takes capture values;
  `CaptureBindingPlan` + `bind_captures_for_install` +
  `StagedHookInstall.capture_plan` DELETED — the weave passes only Sig
  args, observers zero args, handler arity == Sig arity; registry rows
  render composites.
- Rule-6 pins landed end-to-end EXECUTION-proven: scalar 3/5 distinct +
  equal-share; `[1,2]`/`[1,2,3]`/`[2,1]` three-way distinct;
  `("ab","c")`/`("a","bc")` with the flat-join control; nested
  `[[1,2],[3]]`/`[[1],[2,3]]`; `Some(5)`/`None`; suffix-level refuters
  with non-vacuity controls (`Some(5)`-vs-`5`, count-head arity).

## S3c — the proof stage (`732ac770`)

No product code — pins only.

### Evaluate-once (charter (d))

**Warning-channel probe result (disclosed):** annotation-handler
`warning()` diagnostics DO reach the `shape run` subprocess — on STDERR,
as `warning[C0002]: <msg>` lines (LSDS terminal render;
`surface_comptime_warnings` fired at the `functions_annotations.rs:1434`
handler seam, anchored `<synthetic>:{line}:{col}`). The charter's PRIMARY
CLI route therefore applies; the in-process `execution.warnings` fallback
was not needed. (Stderr also carries pre-existing noise — extension-load
lines and "V2 bytecode verification warning" blocks from the comptime
synthetic program — the marker count filters on
`warning[`-prefixed lines containing the marker, insensitive to that
furniture.)

Fixtures `c3-config-eval-once.shape` (one application, marker count
EXACTLY 1, stdout 223000) and `c3-config-eval-once-two-apps.shape` (two
applications, count EXACTLY 2, stdout 2453000), both #65-fenced (hoisted
`let cfg = [10, 20]`), both 200-call hot loops crossing T1@100, both
modes: CLI cells 7/8 in `jit_c3_carrier_native.rs`
(`c3_config_eval_once_warning_fires_once_per_application`,
`c3_config_eval_once_two_applications_warn_exactly_twice`). The 2-app
pair carries structurally EQUAL config, so rule 6 SHARES one baked
specialization — yet the count is 2: evaluate-once-per-BINDING follows
APPLICATIONS (handler runs), never once-globally; and it never scales
with the 200 invocations (a per-invocation implementation would count
400+; the S0 a4e legacy contrast — `[5] fired` … `[6] fired` after a
module-binding mutation between calls — is quoted in the fixture
headers). Both fixtures measured zero-fallback in JIT mode; the
evaluate-once cells deliberately do NOT assert fallback counts so their
failure always means an evaluate-once regression (nativity is cell 6's
pin) — disclosed design choice.

### The W39/LoadModuleBinding named check (charter (e), bytecode tier)

`weave.rs::baked_config_emits_no_module_binding_loads_in_handler_or_wrapper`:
a woven program carrying a SCALAR capture and an ARRAY capture (two
annotations, one handler each — see the probe result below), executed
(value 64 proves both baked constants), then scanned: every
`hook_install_registry` row's specialized handler (both symbols carry
`::cfg#1`) and the generated wrapper (`victim`'s Function entry) contain
ZERO instructions in the module-binding LOAD family — the S0-named legacy
`LoadModuleBinding` (0x52) PLUS the typed `LoadModuleBinding{I64…Ptr}`
variants (0x182..=0x18C; emitter-dead at HEAD —
`typed_load_module_binding_opcode` is `#[allow(dead_code)]` — scanned
anyway so a future emitter flip cannot reintroduce the poison under a
typed spelling). NON-VACUITY CONTROL
(`module_binding_load_scanner_counts_a_genuine_module_read`): a fn
genuinely reading a top-level script binding counts > 0, and the emitted
opcode IS 0x52 — the exact opcode the S0 a4d wrapper measurement
recorded. The bytecode-level pin holds regardless of JIT status. (The
hygienic impl shadow is deliberately OUT of the claim: it is USER code
and may legitimately read module bindings.)

### The composite-config zero-fallback CLI cell (charter (e), primary route)

`tests/smokes-jit-closure/c3-composite-config-single.shape` — a real
annotation handler installs a before template with an `Array<int>`
capture on a 1-ary SINGLE-carrier target (deliberately off the known
aggregate-carrier gap), 200-call hot loop, value-distinguishing
arithmetic (462000; skip ⇒ 199000, element-swap ⇒ 482000, dropped length
⇒ 458000), #65-fenced. Cell 6
`c3_composite_config_single_runs_natively_both_tiers` on the established
zero-fallback pattern: exact stdout, VM==JIT, zero `[jit-fallback]` lines
BOTH modes, parse-based vacuity guard. **MEASURED GREEN — the baked
composite prologue (heap-constant `Array<int>` construction at handler
entry) reaches native JIT directly; the `NewTypedArray*`-lowering
named-expected-fallback contingency was NOT needed and no follow-up
issue is required for Single-carrier composite config.** The
scalar-config sibling is already covered natively by the S2d single cell
(`capture("bump", 2)`), so no duplicate cell was added (the charter's
check-first rule).

### Nested-composite end-to-end

Fully landed in S3b
(`rule6_nested_composite_config_stays_distinct_end_to_end`:
`[[1,2],[3]]` vs `[[1],[2,3]]` through capture→finish→install→bake→
weave→execution, 122/112 per-target values, distinct function indices,
row renders `[[1, 2], [3]]`) — S3c re-verified green; no addition
needed.

## Resolved design decisions (with disclosed deviations)

1. **Identity rides the template suffix, NOT
   `ensure_monomorphic_function_with_consts_for_callsite`** — the S0 §7.2
   sketch ("config captures enter as consts via the `_with_consts` entry
   point") is SUPERSEDED by the charter's own "extend the S1/S2 injective
   identity" instruction, for three load-bearing reasons: (a)
   `ComptimeConstValue` is the CONST-GENERIC surface and is SCALAR-only —
   composites would need a second composite-constant carrier
   (parallel-implementation defection); (b) `const_value_mono_segment`'s
   string arm is LOSSY (the sanitizer maps `"a b"` and `"a_b"` to the
   same segment — the exact #64/S1-verify-1 injectivity bug class; the
   S3a unit control calls the real producer and proves the collision);
   (c) the S1 plan-guard (b) const-generic fence in `cache.rs` STAYS — a
   template plan reaching the const-generic reroute remains a named
   internal error.
2. **Netstring identity, display separate.** `structural_key_segment` is
   tagged + length/count-prefixed (`i:`/`n:`(bit-pattern hex)/`b:`/
   `s:{len}:`/`a:{len}:[…]`/`o:s:`/`o:n`), injective by the in-doc
   decoding argument; `render()` is display-only (registry rows, S8
   hover) and provably collides where identity must not (the cross-tag
   control: `render` of `1` and `"1"` collide, segments differ).
3. **f64 structural equality is bit-pattern equality** — `-0.0` ≠ `0.0`
   (over-distinction is sound: at worst two specializations where one
   would do); non-finite rejects at lift, including inside composites.
4. **Declaration-site sentence grew `({reason})`** (S3a disclosed
   deviation): the mandated sentence shape is verbatim except an inserted
   parenthetical so the Function/Borrow arms can name
   "functions"/"references" as the charter requires.
5. **NativeKind has NO Unit variant at HEAD** — no unit value can reach
   the capture seam; the out-of-domain test rows use
   `Int32`/`Char`/`Ptr(HashMap)` instead. Related nuance (surfaced):
   `shape_ast::ast::Literal` DOES have a Unit variant and the grammar
   parses `()` (`shape.pest` unit_literal), so the domain sentence's
   "unit has no literal form" parenthetical is stale at the AST tier —
   moot at the VALUE tier where lift operates; kept verbatim as mandated.
6. **The L5 `__Option` TypedObject runtime carrier** (S3b disclosed
   product-code growth, necessary completion): the mini-VM's
   `Some(x)`/annotated `None` produce the fixed-layout `__Option`
   TypedObject, not `Ptr(HeapKind::Option)` — `lift_value` grew the
   schema-witnessed `lift_option_typed_object` arm reusing the ONE
   established owned-share field read (`comptime::read_typed_object_field`,
   visibility widened `pub(in crate::compiler)`); malformed carriers
   reject loudly; the host-side `OptionData` arm stays.
7. **Param-mutability probe:** Shape by-value params ARE assignable
   (`functions.rs:1941-1949`) — the bake mints `let mut` prologue lets so
   assignment semantics survive the param→local move (pinned).
8. **Ordered in-territory pin flips, executed and disclosed** (S3b):
   sugar r1 → `r1_config_params_are_rule6_constlift_identity_two_
   specializations` (two specializations, rows still 3/5, output 31051
   unchanged); weave shared-handler pin → rule6 distinct/equal twins;
   install_registry capture pin re-targeted to the baked prologue;
   comptime_builtins non-scalar pin → the S3 out-of-domain sentence;
   const_lift plan tests → bake tests. No other pin moved.
9. **Evaluate-once cells assert semantics only** (S3c, disclosed above):
   marker counts + stdout + VM==JIT; nativity is pinned by cell 6, so
   each cell's failure has one meaning.
10. **The W39 mixed-kind program spells one config kind per annotation**
    (S3c, disclosed adaptation): the charter's "scalar + Array captures"
    program is delivered as two annotations on one target because of the
    handler-wide unification limitation (relay item 3 below) — the woven
    program still carries both bake spellings through one wrapper.

## Rejection-sentence inventory (S3 producers; exact sentences)

The closed-domain sentence embedded verbatim in every producer
(`CONST_LIFT_DOMAIN_SENTENCE`):

- `the ConstLift domain is int, number, bool, and string values, plus
  arrays, homogeneous tuples, and Option of liftable values, recursively
  (unit has no literal form and is not liftable; a None/null value lifts
  only against an Option-typed capture parameter)`

Value-tier producers (G13 string-tag, #60 routing note, no C09xx):

- Out-of-domain: `` capture `{name}` holds a value outside the ConstLift
  domain (kind {kind_desc}); {DOMAIN_SENTENCE} — pass a liftable capture
  value ``.
- Never-liftable (per-class arms fill `{class}` with `function` /
  `reference` / `runtime handle` / `compiler descriptor` …): `` capture
  `{name}` holds a {class} value (kind {kind_desc}), which is never
  liftable (C3-G5 / Dec-95): references, resources, capabilities,
  functions, provider grants, compiler descriptors, secrets, and runtime
  handles cannot cross the comptime->runtime stage boundary;
  {DOMAIN_SENTENCE} — pass a liftable capture value ``.
- Non-finite (recursion-inherited, fires at every depth): `` capture
  `{name}` holds a non-finite number ({n}); capture values must be finite
  so they can be delivered as typed literals — pass a finite number ``.
- Value-vs-declared-annotation (`validate_capture_value_types`):
  `` capture `{param}` on template body fn `{fn}` holds a {lifted-type}
  value but the matching trailing capture parameter is annotated
  `{annotation}`; pass a `{annotation}` value or annotate the parameter
  `{param}: {lifted-type}` ``.

Declaration-site producer (`finish()`-time, template construction):

- `` template body fn `{fn}` declares trailing capture parameter
  `{name}: {type}`, whose type is outside the ConstLift domain
  ({reason}); {DOMAIN_SENTENCE} — declare the capture parameter with a
  liftable type `` — `{reason}` is `` `{type}` is a function type, and
  functions are never liftable (C3-G5 / Dec-95) `` / `` `{type}` is a
  reference type, and references are never liftable (C3-G5 / Dec-95) `` /
  `` `{type}` is not a liftable type ``.

Pinned pre-existing (NOT S3 producers; loud surface-and-stop locked by
probe pins): the comptime empty-array-literal rejection (`[C0001] this
operation is not available in compile-time code`) and the handler-wide
mixed-capture-type inference rejection (`[C0001] Could not solve type
constraints`).

## Test inventory at `732ac770`

In-module unit tests (current totals): `const_lift.rs` 47 (domain lift
happy paths + every rejection sentence verbatim + `matches_annotation`
matrix + `to_expr` projections + `render` forms + all six
`structural_key_segment` refuter classes with flat-join non-vacuity
controls + bake pins); `checked_template.rs` 27 (incl. the finish()-time
declaration-site pins); `template_specialization/mod.rs` 36 (seam
share/split, bake-route seam pins, empty-array-bakes-at-the-seam,
suffix-level refuters); `weave.rs` 30 (rule-6 end-to-end matrix, the S3c
W39 pair, the mixed-value-types probe pin, the empty-array probe pin);
`install_registry.rs` 12; `sugar_matrix_tests.rs` 9;
`pseudo_tuple.rs` 27. CLI: 8 cells in `jit_c3_carrier_native.rs` (3 S1a
proxies + 2 S2d + S3c cells 6/7/8). Fixtures added in S3c:
`c3-composite-config-single.shape`, `c3-config-eval-once.shape`,
`c3-config-eval-once-two-apps.shape` (all #65-fenced with the doc-comment
naming the leak and the hoisted spelling).

## Gates at `732ac770` (the S3c full six-suite close run, lane, `-j1`)

- shape-vm `--lib` `--test-threads=1`: FAILED set == the S0 7-name
  baseline EXACTLY (3379 passed; 34 ignored; the nested_exact flap pair
  green this run).
- shape-test `annotations_comptime`: FAILED == the 10-name set (116
  passed). `comptime`: FAILED == the 3-name set (261 passed).
  `annotations_runtime` 24/24. `annotation_targets` 24/24 (the 48 legacy
  pins green; `tools/shape-test/` has a ZERO-line diff across the whole
  slice range). `lsp` 502/502.
- shape-lsp `--lib` 882/882.
- shape-cli `cli_tests` `--test-threads=1`: 55/55 = 52 prior + 3 new S3c
  cells; per-module non-vacuity counts verified (jit_c2_install_native 6,
  jit_c3_carrier_native 8, jit_closure_capture_native 9,
  jit_fallback_diagnostic_matrix 8, jit_fstring_format 4,
  jit_generated_capture_native 9, script_execution 8, tree 3).
- `just check-clean` exit 0; `just check-no-dynamic` exit 0;
  `cargo check -p shape-vm --all-targets` zero errors (lane).
- Refused-regex grep clean over the working diff AND the full
  `8f8b3594..HEAD` slice diff; `functions_annotations.rs` 0-line diff
  (legacy weave byte-unchanged).

Standing invariants held across all three commits: one carrier, one
transaction, one attribution producer, ONE identity-suffix producer, ONE
bake producer, one traversal core; no second constant store; no grammar
changes; no serde on template types; no C09xx minted; no new HeapKind;
no `FieldType::Any` on the new path; `KindedSlot` never in the typed
VM↔JIT slot ABI; `LiftedConst` is compiler-tier data, never a runtime
carrier or a parallel HeapKind discriminator (ADR-005 §1).

## Supervisor relays (aggregated for disposition)

1. **Composite-config JIT: MEASURED GREEN (closes the charter (e)
   question for the Single carrier).** The baked `Array<int>` prologue
   reaches native JIT zero-fallback both tiers (cell 6). No contingency
   pin, no follow-up needed for Single-carrier config. The
   AGGREGATE-carrier gaps (a)+(b) from S2d are UNCHANGED and remain S7's
   named follow-up with the S2d measurement.
2. **Warning channel finding (S3c probe):** handler `warning()`
   diagnostics reach the subprocess stderr (`warning[C0002]:` LSDS
   lines) — the charter's primary CLI route worked; recorded here so S7+
   cells can reuse the marker-count pattern.
3. **NEW — handler-wide capture VALUE-type unification (pre-existing
   inference gap, S4-RELEVANT).** All `capture()` value arguments inside
   ONE annotation handler must have the SAME type: mixing int+Array or
   int+string (one captures array OR separate installs) fails the
   comptime mini-VM's solving with the pre-existing loud `[C0001] Could
   not solve type constraints` (the builtin's `unknown` value param is
   one shared inference site). Homogeneous multi-captures work (two
   scalars: the C0907 pin; two arrays: probed + now exercised by the W39
   pin). LOUD surface-and-stop locked by
   `mixed_capture_value_types_in_one_handler_are_a_loud_inference_
   rejection_today`. **Disposition needed BEFORE S4:** the sugar lowers
   annotation config params onto `capture()` inside ONE generated
   handler, so a declarative annotation with MIXED-type config params
   (`annotation retry(times: int, tag: string)`) cannot lower until
   either the builtin's value param gets per-call-site inference or S4
   chooses a different lowering spelling.
4. **Pre-existing comptime empty-typed-array-literal gap** (S3b probe,
   standing): `let cfg: Array<int> = []` in a handler rejects loudly
   (`[C0001] … not available in compile-time code`) BEFORE `capture()`
   is reached; the bake half is seam-proven green. Follow-up issue
   disposition still open.
5. **Unit-literal staleness nuance** (S3a, standing): the mandated domain
   sentence's "unit has no literal form" is stale at the AST tier
   (grammar parses `()`); moot at the value tier. Kept verbatim as
   mandated — flag if the sentence should be re-worded when S5 owns the
   matrix.
6. **Symbol-length residual** (S3b, standing): large string/array configs
   produce long keys/symbols — correctness-first; SHA-256 of the
   rendering is a sanctioned possible follow-up, never silent in this
   slice.
7. **S7 hand-offs:** aggregate-carrier (a)+(b) unchanged (relay 1); S7
   zero-fallback cells must EXECUTE handler paths (standing S0 §2
   obligation — the S3c cells do); the eval-once marker-count pattern
   (relay 2) is available for S7's wider matrix.
