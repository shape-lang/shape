# C3 #14 slice-5 report — [C0926] ambient totality + the rejection matrix

Landing on branch `adr009/c3` atop `ae735292` (the S4-close merge + main
merge). Authority: `c3-decisions.md` (G4 no-ambient/config-as-declared-
captures makes [C0926] total; G8+G11; G12; G13; G7),
`c3-slice0-report.md` §4 (a1–a6) + §6 (census), the S2/S3/S4 slice
reports, CLAUDE.md Forbidden Patterns + ADR-006 at maximum binding. This
report grows per stage (S5a → S5b → S5c); each stage's section is
append-only after its gate.

## S5a — the [C0926] gate + Dec-65 census ([C0931]) + the matrix core

### Census re-confirm (mint-time, at `ae735292`)

`rg -o 'C09\d\d' -uu` workspace-wide, per-occurrence classification:
**C0926 comment/doc-only (free)** — occurrences only in AGENTS.md,
battery.rs stale comment, sugar_matrix_tests doc-comment, design docs;
**C0931 doc-only (TRUE NEXT-FREE)** — occurrences only in design docs +
"S5 owns minting from C0931+" comments; **C0932+ free** (single census-
table row). S5a mints EXACTLY [C0926] (one producer:
`pseudo_tuple.rs::AmbientScopeCtx::ambient_rejection`) and [C0931] (one
producer: `functions_annotations.rs::
reject_runtime_module_binding_config_args`). [C0930] reuse was NOT taken
for the capture-bijection sentences — E1-D4 binds C0930 to the single
producer `resolve_param_id` (directive param-miss class); the template
capture-bijection sentences stay G13-uncoded per the S2 posture.

### The hole, measured first (throwaway probes; ALL reverted byte-clean —
### `git status --porcelain` empty before implementation began)

Probe module appended to `weave.rs` tests, run via the lane, outputs
verbatim:

- **P1 (a1 shape, NEW path, concrete API body)** — template body reads a
  script top-level `let ambient = 7`:
  `P1 MEASURED: compiles+runs, value=Some(110), handler module-binding
  loads=[("hook", 1)]` — SILENT ambient capture, `LoadModuleBinding` in
  the specialized handler (the S0 W39 poison shape, alive on the new
  path).
- **P2 (a6 API shape)** — annotation + `pub const secret` + API body fn
  all in `mod defs`, target-module `let secret = 99`:
  `P2 MEASURED: compile error: Semantic error: Undefined variable:
  'secret'` — LOUD today (a mod-block fn cannot see its sibling const at
  definition-compile).
- **P2b (control)** — a PLAIN mod fn reading a sibling `pub const`
  (no annotations at all): `P2b MEASURED: compile error: Semantic error:
  Undefined variable: 'secret'` — the P2 failure is a PRE-EXISTING
  mod-const resolution gap, not template machinery.
- **P2c (a6 SUGAR shape — THE HEADLINE)** — `mod defs { pub const
  secret: int = 11; annotation hookann(times: int) { before(args) {
  args[0] = args[0] + secret + times ... } } }`, application module
  `let secret = 99`, `@defs::hookann(1)`:
  `P2c MEASURED: compiles+runs, value=Some(1040)` — the
  application-module binding SILENTLY WINS over the annotation's own
  const (1040 = shadow 99; the annotation's intent 11 would give 160).
  The slice-0 a6 disaster reproduced ON THE NEW PATH via the
  verbatim-carried sugar body.
- **P2e (a3 API shape)** — binding only at top level, API body in
  `mod defs`: `P2e MEASURED: compile error: Semantic error: Undefined
  variable: 'sees'` — loud today (the mod-block body fn compiles at
  definition, before/without the top-level binding); the a3 SUGAR shape
  is the silent variant (same mechanism as P2c).
- **P3a (a4d analog, TypedConfig)** — `let chosen = 5` + `@retry(chosen)`:
  `P3a MEASURED: compile error: Semantic error: [C0001] Undefined
  variable: chosen. Variable names resolve from local scope and module
  scope.` — loud but bland; the [C0931] upgrade target.
- **P3b (a4d analog, Legacy comptime)** — untyped-param annotation with
  a comptime post handler, `@amb(chosen)`: same bland
  `[C0001] Undefined variable: chosen...` text.
- **P4 (Dec-65 (i))** — `capture("x", args)` in handler scope:
  `[C0001] Undefined variable: 'args'` — loud pre-evaluation.
- **P5 (Dec-65 (ii))** — `comptime { let y = x }` inside a template body
  (concrete sig param `x`): `[C0001] Undefined variable: 'x'` — loud
  pre-evaluation (`execute_comptime_with_context` receives helpers only).
- **P6 (const-config twin probe)** — top-level `const n: int = 3` +
  `@retry(n)`: `P6 MEASURED: compile error: Semantic error: [C0001]
  Undefined variable: n. ...` — **the const-config positive twin CANNOT
  RUN at HEAD**: the handler mini-VM has no const-injection route for
  module consts (see deviation 1).

### The [C0926] gate (landed)

- **Site**: `specialize_template` (`template_specialization/mod.rs`),
  IMMEDIATELY after the E1-D6b transaction check, BEFORE the per-kind
  dispatch — unconditional (cache-hit or not), all four template kinds.
  Site rationale (in the code comment): construction-time checking is
  ORDER-SENSITIVE (a binding registered between construction and handler
  compile slips through; the pre-pass fn-lookup-miss defers to pass-2 by
  design); every install passes `specialize_template` — totality by
  construction. Unit pins cover concrete-before / concrete-after /
  polymorphic-before / observer + the ordering pin (fires before the
  zero-param per-kind rejection) + the unreferenced-binding negative
  control.
- **Mechanism**: a THIRD face (`Face::AmbientScope`) on the ONE
  pseudo-tuple traversal core — never a second walker. The face runs
  with unspellable sentinel template names (the pseudo-tuple surface is
  structurally disengaged; the template's own `args`/captures are
  ordinary frame-0 parameters), maintains a lexical scope stack
  (frame 0 = sig + capture params; frames at block/closure/arm
  boundaries; binders register in traversal order → sequential
  visibility), and classifies every free identifier:
  - resolves (scoped-then-bare via
    `resolve_scoped_module_binding_name`, mirroring the emission path
    `expressions/mod.rs:453-459` / `identifiers.rs:350`; PLUS the
    template-module trial, deviation 3; PLUS `imported_consts`,
    deviation 4) to a module-scope VALUE binding → **[C0926]**;
  - call-name (FunctionCall / joined QualifiedFunctionCall) resolving to
    a module-scope FN (`function_defs` / `find_function` /
    `stdlib_function_names`, bare + scoped) → LEGIT (G3);
  - sig/capture params + locals → LEGIT; unresolvable → left to ordinary
    downstream resolution (unchanged sentences).
  - MUST-SCAN positions the pseudo-tuple faces skip, all covered +
    pinned: f-string interpolation interiors (re-parsed exactly as the
    emitter does — the F5 boundary does NOT apply to this face),
    assignment TARGETS (`StoreModuleBinding`), closure interiors,
    module-QUALIFIED value references (`defs::secret` — the Unit-payload
    enum-constructor parse).
  - Value-position precedence is the EMISSION precedence (module
    bindings before fn tables — `identifiers.rs:350` vs `:437`), so the
    gate rejects exactly what the emitted code would silently load.
- **Sentence** (ONE producer, pinned byte-exact in
  `a1_toplevel_let_in_template_body_rejects_with_the_exact_sentence`):
  `[C0926] hook template body {origin} references `{ident}`, which
  resolves to the module-scope value binding `{resolved}`; a hook
  template's body reads only its exact inputs — its signature parameters
  and its declared captures (C3-G4); module- and invocation-scope values
  never enter a template ambiently, because the body is specialized into
  the application module where an unrelated binding spelled `{ident}`
  silently takes over the hook's behavior — declare it as an input
  instead: add a typed config parameter (or capture("{ident}", ...)) and
  reference the capture parameter` — anchored at the `@application` span
  through `template_application_error`. `{origin}` = `` fn `name` `` for
  API bodies / `` the `before|after` hook of annotation `X` `` for
  sugar-minted bodies (compiled-annotation reverse lookup; the SOH
  hygienic name is never rendered in the sentence).
- **The G4 boundary rule + a6 quote + invariant-7 asymmetry rule** are
  stated verbatim in the `AmbientScopeCtx` doc-comment (the a6 P2c
  measurement is quoted as the motivating disaster).

### The a1–a6 disposition table (each row pinned in
### `weave.rs::tests::s5a_ambient_totality` / the S4 pins)

| row | shape | verdict | pin |
|-----|-------|---------|-----|
| a1 | script top-level `let` read in template body | **[C0926]** | `a1_...rejects_with_the_exact_sentence` (byte-exact sentence + @application line) |
| a2 | `pub let` in a mod block | unchanged pre-existing ``module-level variable declarations currently require `const` `` | `a2_...control` (asserts NOT [C0926]) |
| a2b | annotation's own module `pub const` | **[C0926]**, resolved `defs::secret` via the template-module trial; capture positive twin | `a2b_...capture_twin` |
| a3 | binding only in the TARGET's module (sugar body) | **[C0926]** | `a3_target_module_binding_rejects_c0926` |
| a4/a4c | nested-fn application | C3-G12 loud rejection — TypedConfig landed S4 (`reject_typed_config_annotations_on_nested_fn`, pinned there); Legacy extension is S5b | S4 pins + control |
| a4d/a4e | runtime module binding as config arg | **[C0931]** | `c0931_typed_config_...` (byte-exact + line) / `c0931_legacy_comptime_...` |
| a5 | config-param-only body | **LEGIT** — executed value-distinguishing (13) + the ambient belt (handler + wrapper zero module-binding loads) | `a5_config_only_body_weaves_runs_and_stays_module_binding_free` |
| a6 | defs const + same-spelled target binding | **[C0926]** naming the APPLICATION-module binding (the silent winner) | `a6_shadow_probe_...` |

`ctx` stays an ordinary unresolved identifier (E4 family — no arm).
Scope-stack behavior pins: local-shadow negative control (runs with the
LOCAL value, 60 vs ambient 110), use-before-let (rejects), disjoint-
branch shadow (frame pops → rejects), module-fn-callee control (runs).

### The proven-biting refuters

- **Pre-fix hole**: P1/P2c above (silent 110 / silent 1040).
- **Neuter probe** (throwaway `return Ok(())` at the gate entry,
  REVERTED byte-clean): with the face neutered, the committed a1 and a6
  pins FAIL with `fixture must be rejected: ()` — the fixtures COMPILE
  (and P1/P2c measured what they then run: the silent ambient values).
  10 of the 20 matrix pins bite under neuter (all rejection pins); the
  10 that stay green are the positive twins / controls / Dec-65 loud
  pins, as designed. Bonus observation recorded: under neuter the
  qualified-reference fixture (`defs::secret` in a sugar body) fails
  MISATTRIBUTED through the S2c aggregate-kind guard ("cannot prove the
  type of the value assigned to `args[0]`") — pre-gate that class was
  loud-but-wrong; post-gate it is the named [C0926].
- **Ambient belt (weave tier)**: the a5 pin extends the S3c
  `module_binding_loads` scanner claim — post-gate, the specialized
  handler AND the generated wrapper carry ZERO module-binding loads; the
  genuine-module-read non-vacuity control is unchanged beside it.

### Dec-65 — [C0931] + the PINNED-UNCONSTRUCTIBLE note (E2-D9 precedent)

**The ONE constructible Dec-65-family shape** is the config-arg position:
a RUNTIME module binding in `@ann(<arg>)` (the a4d analog). Minted as a
pre-check at ALL THREE seams where the mini-program call args are built —
the signature-directive pre-pass (`functions_annotations.rs` ~:462), the
authoritative pass-2 seam (`execute_comptime_annotation_handler`, which
also covers the type/module/expression-target call sites in
`statements.rs` / `expressions/mod.rs`), and the declaration-discovery
speculative pre-pass (~:2481) — each returning `Err` BEFORE execution so
the pre-pass error-swallows cannot eat it. Sentence (ONE producer, pinned
byte-exact): `[C0931] config argument `{ident}` for `@{ann}` references a
runtime module binding; annotation config is evaluated once at compile
time (Dec 65 — runtime values never enter a comptime evaluation position)
— pass a literal or a comptime const; a value that varies at runtime
cannot configure a compile-time specialization` — anchored at the
`@application` (`ann.span`). Exemptions (the invariant-7 asymmetry rule,
stated in the producer doc): `const_module_bindings` members, injected
specialization `const_bindings` (by name), `imported_consts`. Fires for
BOTH classes (TypedConfig + Legacy-with-comptime-handlers — the disclosed
legacy diagnostic upgrade; legacy RUNTIME-hook config is per-invocation
and untouched until S6). The detector's collector is conservative
(value-position identifiers incl. f-string interiors; call names
skipped); an uncollected shape falls to the pre-existing loud mini-VM
unresolved error — never silent.

**Pinned-unconstructible (no dead product arm)**: every OTHER
hook-input→comptime shape dies loud PRE-evaluation, texts locked by probe
pins: (i) `capture("x", args)` / hook-input names in handler scope →
`[C0001] Undefined variable: 'args'`
(`dec65_hook_input_in_handler_scope_dies_loud_preevaluation`);
(ii) `comptime {}` reading a concrete sig param → `[C0001] Undefined
variable: 'x'` (`dec65_comptime_block_reading_concrete_sig_param_...`);
(iii) capture of the target descriptor → the S3 never-liftable
compiler-descriptor arm (already pinned in `const_lift.rs`). PLUS the
stage-optional walker arm, TAKEN: pseudo-tuple/type-param names inside
`Expr::Comptime` in a template body now reject with the named uncoded
sentence ("a `comptime` block inside a template body cannot read
`{name}`: comptime code evaluates at compile time, before the hook's
runtime inputs exist (Dec 65 ...)") instead of leaking a minted
`__c3_arg_{i}` unresolved error
(`dec65_comptime_block_reading_pseudo_tuple_names_the_boundary` also
asserts no `__c3_` leak). Mechanism: `comptime_depth` on the ONE walker —
the pseudo-tuple interceptors are gated to `depth == 0`.

### C0907 one-word alignment (optional item — TAKEN)

`checked_body.rs` `validate_capture_clause` now says "exactly once",
aligned with the planner producer. The pre-declared "divergent pin flip"
turned out to be a NO-OP: no pin asserted the divergent "at most once"
word; the existing `duplicate_capture_name_is_rejected_c0907` pin was
STRENGTHENED to assert the aligned phrasing instead.

### Deviations / disclosures (S5a)

1. **The const-config positive twin cannot RUN at HEAD** (probe P6): the
   handler mini-VM has no visibility of module consts, so an exempted
   const config arg still fails with the PRE-EXISTING loud `[C0001]
   Undefined variable`. The stage's "const-config [C0931] exemption
   runs" is therefore delivered as: the exemption NEVER MIS-FIRES (a
   const is never called a runtime module binding — pinned) + the loud
   pre-existing failure locked as a control
   (`c0931_const_config_arg_is_exempt_and_keeps_the_preexisting_loud_error`).
   Making module consts visible in the config position is a NAMED
   FOLLOW-UP needing supervisor disposition — product growth beyond the
   "diagnostic upgrade only" charter. The twin that RUNS today is the
   literal config arg (a5 + the r1-family pins).
2. **`template_application_error` grew a provenance note** ("hook
   template installed on `{target}` from this application site"). Twice
   load-bearing: LSDS provenance + it satisfies the D1 preserve
   predicate of `preserve_or_wrap_directive_failure`, so the
   application-site anchor now SURVIVES the directive-processing wrap
   end-to-end (previously EVERY template rejection was flattened into a
   handler-span RuntimeError e2e — the S0 g3 mis-anchoring class
   C3-G10 exists to fix; the S1 anchoring pins only covered the direct
   `specialize_template` call). Display renders only the message, so
   every existing sentence pin is byte-unaffected; no test anywhere
   asserts the dropped "directive processing failed" prefix (grepped).
3. **The a2b template-module resolution trial**: the detector also tries
   `{template_module}::{ident}` (the body fn's defining module, from its
   qualified name or the sugar annotation's key). Beyond the strict
   `:454-459` mirror, REQUIRED for the mandated a2b verdict (at
   specialize time the compiler's module-scope stack is the APPLICATION
   module's, so the annotation's own-module const would otherwise miss)
   — and G4-faithful: a reference resolving to ANY module-scope value
   binding is ambient.
4. **`imported_consts` included in the detector** (invariant-7-faithful:
   an imported `pub const` inlines at use sites — a module-scope const;
   silently honored pre-gate). Script-mode fixtures cannot pin it (S0's
   two-file caveat stands); noted for S8/book coverage.
5. **Traversal-order flips in the shared walker** (value/iterable BEFORE
   binder in variable_decl / for / let-expr / async-let / query-let /
   comprehension clauses): load-bearing ONLY for the ambient face's
   sequential visibility; order-independent for the validate/rewrite
   faces (their checks accumulate no cross-subtree state; the
   carrier-writes collection sits in the assignment arm, untouched).
6. **Weave test harness fix** (test-tier): `compile_source` now sets
   `compiler.source_text`, so span→line mapping in location pins is real
   (previously every location degraded to 1:1 in this harness).
7. **check_reserved is skipped by the ambient face** — concrete bodies
   were never construction-walked, so the ambient face must not tighten
   the `__c3_` reserved-prefix surface beyond its [C0926] charter.
8. **Closure capture-clause entries** (C1 generated closures inside
   template bodies) register as closure-frame binders without an
   outer-scope ambient check — a `captures(x)` naming a module binding
   would slip the gate's closure arm; the body USE of such a capture is
   in-scope by construction. Narrowing disclosed; the C1 capture-plan
   machinery has its own module-binding rules.
9. **The attribution FRAME still renders the SOH minted symbol** for
   sugar bodies (``annotation template `\u{1}hygienic:...` (declared
   ...)``). Pre-existing S2b/S4 behavior; the S5 obligation ("never
   render the SOH name") binds the sentence's `{origin}`, which never
   does. Display follow-up candidate, flagged.
10. **P2b finding**: plain mod fns cannot read sibling mod consts
    (loud `Undefined variable`) — a pre-existing language gap the a2b
    API shape sits on; recorded for whoever owns mod-scope resolution.

### Throwaway probes — reverted-clean confirmation

Both probe waves (the P1–P6 fixture module in `weave.rs`; the gate
neuter in `mod.rs`) were reverted byte-clean before commit
(`git status --porcelain` empty of probe files / `git diff` shows only
the implementation + pins). No probe text ships.

### Gates at the S5a close (lane, `-j1` / `--test-threads=1`)

- New-pin filters: `s5a_ambient_totality` 20/20; `c0926_gate*` unit pins
  6/6; template_specialization + pseudo_tuple + checked_template +
  functions_annotations + sugar filters 328/328.
- Full shape-vm `--lib --test-threads=1`: FAILED set == the S0 7-name
  baseline EXACTLY, plus ONE red of the DOCUMENTED nested_exact flap
  member — flap protocol run: `--exact` twice on the same binary →
  FAILED then ok (nondeterministic; the S4d-recorded behavior; NOT
  S5a-caused).
- shape-test: `annotations_runtime` 24/24; `annotation_targets` 24/24
  (the 48 legacy pins green); `annotations_comptime` FAILED == the
  10-name S0 set (116 passed); `comptime` FAILED == the 3-name S0 set
  (261 passed). lsp / shape-lsp / cli_tests deferred to the S5c close
  per the blast-radius discipline (no LSP/CLI surface touched in S5a).
- `cargo check -p shape-vm --all-targets`: zero errors (one PRE-EXISTING
  unused-import warning at comptime_builtins.rs test scope, on record
  since S2).
- `just check-clean` exit 0. `just check-no-dynamic` exit 0 (plus its
  informational "closure capture deletion progress baseline=12 actual=4"
  note — pre-existing counter, not S5a's).
- Refused-regex grep (the CLAUDE.md broader-family regex, space/`[ _-]`
  widened) over the full `ae735292..` working diff: zero hits.
- Legacy weave byte-check: `git diff ae735292 --
  functions_annotations.rs` hunks contain ZERO occurrences of
  `compile_specialized_annotation_handler` /
  `specialize_annotation_runtime_handlers` /
  `compile_annotation_wrapper` (the [C0931] seams are the comptime
  handler-execution sites only).
- Census post-mint: [C0926] product-code mint = the one pseudo_tuple.rs
  producer; [C0931] = the one functions_annotations.rs producer; no
  other code minted or touched (C0913–C0921 untouched).

## S5b — the static G8 arm + G12 Legacy extension + non-function-target rejections + lifecycle pins + the G11 defections entry

### The holes, measured first (throwaway probes; BOTH waves reverted
### byte-clean — `git status --porcelain` empty before implementation)

Probe modules appended to `sugar_matrix_tests.rs`, run via the lane,
outputs verbatim:

- **P-G8a (sugar TypedConfig-with-hooks, UNCALLED generic)** — `compile
  error: … cannot install hook template `hygienic:888e6770b1fa80de62edd6ea
  99e86301` (via @retry_g) on `id`: the target is generic — `fn id<T>(x:
  T) -> T` — …` — the sugar shape was ALREADY LOUD pre-S5b through the
  DYNAMIC pre-pass directive arm (the minted body fns resolve through
  `sugar_body_fns` with no function-registration dependence, so the
  pre-pass handler run succeeds and the directive arm fires). The charter's
  expectation that this shape was silent did not reproduce; the genuinely
  silent hole was the API path.
- **P-G8b (API-path direct install, UNCALLED generic — THE HEADLINE)** —
  `compiles, run=Ok(Some(7)), registry_rows=0` — SILENT no-op (the S2b
  residual, alive at HEAD).
- **P-G8c (helper-mediated install, uncalled generic)** — `compile error:
  Semantic error: Undefined function: 'before_hook'` — loud-but-bland
  PRE-EXISTING failure at the HELPER's own pass-2 compile (module fns
  cannot spell the comptime forwarders); same text on the concrete
  control (**P-G8c-ctl**). The helper-mediated class is therefore
  unconstructible-silent today; the static arm upgrades the GENERIC case
  to the named G8 sentence at the `@application` site (disclosed
  diagnostic-tier change below).
- **P-G8d (value-position `let f = before_hook; install(f(…))`, uncalled
  generic)** — `compiles, run=Ok(Some(7)), registry_rows=0` — SILENT.
  Concrete control (**P-G8d-ctl**): loud pre-existing `[C0001] Undefined
  variable: 'my_before'` (handler execution reaches the unresolvable
  fn-as-value first).
- **P-G8e (sugar, CALLED generic — anchor)** — same G8 sentence as P-G8a.
- **P-NFT (`targets: [type]` TypedConfig-with-hooks applied to a type)** —
  `compiles, run=Ok(Some(7)), registry_rows=0` — SILENT (S4 residual 7
  confirmed). **P-NFT-mixed** (`targets: [function, type]`): fn
  application weaves (12, one row); type application silent.
  **P-NFT-mod** / **P-NFT-expr**: module and expression siblings both
  silent (`registry_rows=0`).
- **P-LEGGEN (legacy declarative weave on CALLED generic)** — `compiles,
  run=Ok(Some(10))` — the g1/g4 accidental-working class confirmed at
  HEAD. **P-LEGGEN-unc** (uncalled): silent 7 (legacy class untouched
  until S6, per G11). **P-CONC-unc** (API install on an uncalled CONCRETE
  fn): `registry_rows=1` — installs regardless of calledness.
  **P-LC-legacy / P-LC-legacy-od** (zero-param metadata / on_define
  applied): green through execution.

### 1. The static G8 arm (the S2 supervisor obligation — CLOSED)

- **Site**: the TOP of the signature-directive pre-pass per-annotation
  loop (`apply_function_comptime_signature_directives_to_function`) —
  BEFORE the phases loop, before any handler execution; the pre-pass
  walks ALL items incl. Export fns and nested modules, so every
  `@application` is visited regardless of calledness. Resolution
  (`resolve_comptime_annotation_handlers`) is a pure map lookup.
- **Rule**: `func_def.type_params` non-empty AND the entry is
  TEMPLATE-ENGAGING ⇒ `Err` with the EXISTING ONE producer
  `generic_target_install_rejection_message` (sentence byte-unchanged)
  at the `@application` span. Concrete targets: zero cost beyond the
  `type_params` check.
- **Template-engaging classification (static, conservative — the Lens-2
  F4 precedent)**: (a) sugar path — `entry.sugar_body_fns` non-empty
  (hint = first minted body fn, the same name the dynamic arm renders);
  (b) API path — a syntactic scan of the entry's comptime handler bodies
  AND their transitive helpers for the **`install` name** in call-name OR
  value position (bare, qualified-last-segment, or the marked value
  form). ENGAGEMENT keys on `install` ONLY — the sole installer; the
  constructor names (`before_hook`/`after_hook`/`*_nocapture`) feed the
  body-fn HINT but do not engage. MEASURED NARROWING of the charter's
  five-name set: with all five engaging, the fix-round-1 F5
  store-lifecycle refuter — a MEASURED-GREEN baseline pin
  (`nested_handler_run_during_processing_does_not_shift_install_handles`)
  whose `@noise` annotation on a polymorphic template BODY fn
  constructs two handles and installs nothing — was rejected
  (`cannot install hook template `h_noise` … on `tmpl``). C3-G8
  withdraws INSTALLS on generic targets; construction is not
  installation and the construct-only class is load-bearing (the batch
  snapshot machinery exists for exactly its nested-run shape). Locked by
  the `s5b_static_g8_construct_only_handler_on_generic_stays_green`
  control. Dodge-resistance is unweakened: `install` is the ONLY route
  to an install directive, and every spelling of it (call, qualified,
  method, value, helper-transitive) engages.
- **`collect_authorized_comptime_helpers` transitivity VERIFIED** (the
  charter's check): it worklist-closes helpers-calling-helpers over
  `self.function_defs`; reused verbatim for the scan. DISCLOSED
  ADDITION: in a single-module unit both pre-passes run BEFORE function
  registration (the S2b measured reach), so the registered table is
  empty exactly where the uncalled-generic hole lives — the scan
  therefore also closes over `collect_pre_pass_ast_function_defs`, a
  SCAN-ONLY syntactic fn table from the analysis program's own items
  (bare + module-qualified names; threaded as a parameter, built once
  per pre-pass run). Never an execution surface.
- **Value-position coverage without a second walker**: the shared
  scoped-name collector (`collect_scoped_names_in_expr` — the ONE
  established walker over handler bodies) grew ONE guarded arm: an
  `Expr::Identifier` naming an install-family member (all five, for the
  mark; engagement then filters to `install`) is recorded under the
  unspellable SOH `INSTALL_FAMILY_VALUE_MARK` prefix. The mark can
  never resolve in any fn table, so helper collection is byte-equivalent
  to pre-S5b (lookup miss → skip); every other identifier stays
  uncollected. No walker fork.
- **The body-fn hint** is best-effort (`hook_constructor_hint_in_expr` —
  first `before_hook`/`after_hook` call with a bare-identifier first
  arg; realistic spellings only), falling back to the established
  `"<template>"` placeholder. Engagement NEVER depends on the hint. The
  hint keeps the pre-existing S2b pin
  (`generic_target_install_rejects_with_the_g8_sentence`, CALLED generic
  — previously fired at the pass-2 mono seam) byte-identical: the static
  arm now fires first with the same `my_before` rendering.
- **The three dynamic firing sites remain** (layered, not deleted): the
  pre-pass directive arm + the two `apply_install_hook_template` twins.
- **Pins** (install_registry.rs + sugar_matrix_tests.rs): (i) sugar
  uncalled (`s5b_static_g8_sugar_on_uncalled_generic_rejects`); (ii) API
  uncalled with @application-line span assert
  (`…api_install_on_uncalled_generic_rejects_at_the_application_site`);
  (iii) helper-mediated (`…helper_mediated_install_…`) + concrete
  control locking the pre-existing `Undefined function: 'before_hook'`;
  (iv) value-position (`…value_position_reference_…`) + the
  mark-arm-isolation pin (`…value_position_reference_alone_engages_…`,
  handler body = `let f = install` only) + the construct-only
  engagement-key control (`…construct_only_handler_on_generic_stays_
  green`); (v) uncalled-concrete twin installs (row 1); (vi)
  legacy-weave-on-generic control executes 10.
- **Proven-biting neuter refuter** (throwaway `return Ok(())` at the arm
  entry, run against the FINAL narrowed arm, REVERTED byte-clean):
  exactly the 4 API-path pins FAIL — api_install (`fixture must reject:
  ()`, the fixture compiles and runs silently: the P-G8b hole verbatim),
  value_position + value_position_alone (silent, P-G8d), and
  helper_mediated (falls back to the bland pre-existing `Undefined
  function: 'before_hook'`, failing the G8-fragment assert) — while all
  5 controls/twins stay green (construct-only, helper-concrete,
  legacy-on-generic, uncalled-concrete, and the sugar uncalled pin —
  the dynamic directive arm covers the sugar path, consistent with
  P-G8a).

### 2. G12 Legacy extension (`reject_typed_config_annotations_on_nested_fn`)

Same producer, class-conditional wording via `classify_annotation_params`:
TypedConfig keeps the S4 sentence BYTE-UNCHANGED (`{class_phrase}` =
"hook-template annotations"); Legacy now rejects loudly with
"annotations on nested functions are not supported yet (#62)".
Unresolvable annotation names keep today's `continue` (out of matrix
scope); a mixed classification is unconstructible (R2 fires at the
declaration) — defensive skip. THE PRE-DECLARED PIN FLIP executed: the
S4c silent-drop control (`s4c_g12_legacy_annotation_on_nested_fn_stays_
silently_dropped`, asserted silent 4) is now
`s5b_g12_legacy_annotation_on_nested_fn_rejects_loudly` (asserts the
Legacy sentence verbatim, zero rows). The S4 TypedConfig pin + twin are
untouched and green.

### 3. Non-function-target TypedConfig-with-hooks (S4 residual 7 — REJECT, two tiers)

No legitimate semantic found: hooks attach to a FUNCTION's call seam;
the type/module/expression consumer seams run ONLY
`comptime_pre_handler`/`comptime_post_handler`, never the sugar post
handler. NAMED-NOT-BUILT alternative: a "wrap every method of the type"
semantic is conceivable but would need its own ruling — recorded here,
not improvised.

- **(a) Declaration tier** (planner.rs, after `allowed_targets` is
  computed): `sugar.is_some()` AND non-empty targets excluding
  `function` ⇒ the named sentence (ONE producer
  `non_function_targets_declaration_rejection` in sugar_lowering.rs).
  Empty `allowed_targets` = "no restriction" — function applications
  stay legal, so no rejection. Pins: `s5b_nonfn_type_only_targets_…`
  (byte-exact), `…multi_nonfn_targets_render_the_full_list`
  (`[type, module]`), and three declaration twins (mixed-with-function /
  hook-free TypedConfig on `[type]` / Legacy on `[type]`).
- **(b) Application tier** at the three seams (type:
  `execute_struct_comptime_handlers`; module:
  `execute_module_comptime_handlers`; expression:
  `run_comptime_annotation_handlers_for_target`):
  `compiled.sugar_post_handler.is_some()` ⇒ the named sentence (ONE
  producer `non_function_target_application_rejection`), anchored at
  `ann.span`. Reachable only through MIXED `targets: [function, …]`
  definitions. Pins: mixed fn-application WEAVES (12, one row — the
  twin) / type application rejects byte-exact / module + expression
  siblings ({kind} = "module"/"expression"; an expression target
  renders the established `target` placeholder).

### 4. Lifecycle verify+pin (charter item 4)

- **(i)** `s5b_r3_family_mixed_def_rejects_in_both_handler_orders`: a
  TypedConfig def with BOTH a declarative hook and `on_define` rejects
  with the R3-family sentence in BOTH handler orders (the lowering loop
  rejects OnDefine/Metadata before its empty-hooks early return).
- **(ii)** `s5b_legacy_lifecycle_twin_zero_param_def_executes_green`:
  zero-param def + `on_define` + `metadata` applied to a fn — compiles
  AND executes green (7), zero registry rows.
- **RECORDED**: TypedConfig lifecycle handlers can NEVER receive typed
  config params — the R3-family declaration rejection is TOTAL
  (`plan_definition` runs the lowering for EVERY TypedConfig def; the
  handler loop hits the OnDefine/Metadata arm before the
  `body_fns.is_empty()` early return, so a lifecycle handler in a
  TypedConfig def always rejects at the declaration, hooks or no hooks).

### 5. The G11 defections.md entry

Landed as the dated append-only entry "2026-07-21 — ADR009-C3 (#14) S5:
C3-G8/G11 deliberate capability withdrawal — generic-target hook
installs" in `docs/defections.md`: names the withdrawn g1/g2/g4/g5
working class + the homogeneous-args accident; the g3/g6 failure modes
the rejection strictly improves; the #59 re-arm lift condition (installs
re-arm per specialization origin); the S5b static arm closing the
uncalled silent no-op; the positive twin; and FOUR
considered-and-rejected compromises (handler-run-dependent G8 only;
construction-time-only [C0926]; exact-name-only scan;
registered-table-only helper closure).

### Deviations / disclosures (S5b)

1. **P-G8a did not reproduce the charter's "sugar uncalled = silent"
   expectation** — the sugar path was already loud through the dynamic
   directive arm (measured; the minted `sugar_body_fns` resolve without
   function registration). The static arm still fires FIRST (execution-
   independent totality); the proven-biting refuter binds to the API
   path, where the silence was real.
2. **Helper-mediated GENERIC case: diagnostic-tier change** — pre-S5b it
   failed loud-but-bland at the helper's pass-2 compile (`Undefined
   function: 'before_hook'`); the static arm's G8 sentence now preempts
   it at the `@application` site. The CONCRETE sibling keeps the
   pre-existing text byte-unchanged (control pin). Nothing that ran
   keeps running differently.
3. **The scan-only AST fn table** (`collect_pre_pass_ast_function_defs`)
   — a disclosed addition beyond "reuse collect_authorized_comptime_
   helpers", required because the registered table is empty at the
   pre-pass in single-module units (P-G8c would otherwise stay
   uncovered at the static site). Scan-only; never resolution/execution.
4. **The shared collector's marked value arm** — `collect_scoped_names_
   in_expr` now records install-family VALUE-position identifiers under
   an unspellable SOH mark. Helper collection is byte-equivalent (the
   mark can never resolve); no second walker was forked. All other
   identifiers stay uncollected.
5. **The hint walker is best-effort by design** (realistic spellings:
   blocks, let-initializers, nested call args, if/else) — exotic shapes
   render the established `"<template>"` placeholder; engagement never
   depends on the hint.
5b. **The charter's five-name engagement set was narrowed to `install`**
   (measured collateral: the five-name key rejected the F5
   store-lifecycle baseline pin — see the classification bullet in §1).
   The four constructor names still feed the value-position MARK and the
   HINT; engagement (the thing that fires G8) keys on the one name that
   can actually install. Plan-invariant-5 reading: the F5 pin is a
   baseline member with no pre-declared flip, and the construct-only
   class is load-bearing product behavior — narrowing preserved both
   without weakening dodge-resistance (`install` is the only installer).
6. **`apply_function_comptime_signature_directives_to_items` /
   `_to_function` grew one threaded parameter** (`ast_fn_defs`) — the
   pre-pass entry builds the table once per run. Internal signatures;
   no public surface.
7. **The `type_target_install_rejects_with_the_function_twin` pin
   (S2b)** keeps firing its pass-2 `install`-directive sentence: its
   fixture is a ZERO-PARAM (Legacy-classified) annotation whose comptime
   handler installs on a `targets: [type]` def — no sugar, so the new
   declaration/application tiers do not engage; the directive-level
   rejection remains that class's owner.
8. **Expression-seam kind rendering** reuses the established
   `format!("{:?}").to_lowercase()` spelling (`expression`, `block`,
   `binding`, `awaitexpr`) — the pinned sibling uses `expression`; the
   exotic kinds ride the same producer unpinned.

### Throwaway probes — reverted-clean confirmation

Three throwaway edits shipped nothing: probe wave 1 (`s5b_probes`,
sugar_matrix_tests.rs), probe wave 2 (`s5b_probes2`, same file), and the
static-arm neuter (`return Ok(())` in functions_annotations.rs) were
each reverted byte-clean before commit (`git status --porcelain` clean /
diff-inspected). No probe text ships.

### Gates at the S5b close (lane, `-j1` / `--test-threads=1`)

- Stage filter (`install_registry surface_class sugar_matrix
  template_specialization closures`): **257/257** (0 failed; includes
  the 7 new static-G8 pins + construct-only control, the G12 flip, the
  6 non-function-target pins, the 2 lifecycle pins, the sugar-uncalled
  pin, and every pre-existing pin in those modules).
- Full shape-vm `--lib --test-threads=1`: **FAILED set == the S0 7-name
  baseline EXACTLY** (3462 passed; 34 ignored; the nested_exact flap
  pair GREEN this run).
- shape-test: `annotations_runtime` **24/24**; `annotation_targets`
  **24/24** (the 48 legacy pins untouched this stage);
  `annotations_comptime` FAILED == the 10-name S0 set (116 passed);
  `comptime` FAILED == the 3-name S0 set (261 passed). lsp / shape-lsp
  / cli_tests deferred to the S5c close per the blast-radius discipline
  (no LSP/CLI surface touched in S5b).
- `cargo check -p shape-vm --all-targets`: zero errors; the dead-code
  warning set was diffed against base `314ffada` via stash (12 at base,
  12 with the S5b diff — all pre-existing).
- `just check-clean` exit 0. `just check-no-dynamic` exit 0 (plus its
  pre-existing informational closure-capture counter note).
- Refused-regex grep (the CLAUDE.md broader-family regex, space/`[ _-]`
  widened) over the full `314ffada..` working diff: zero hits.
- Legacy weave byte-check: the `functions_annotations.rs` hunks contain
  ZERO occurrences of `compile_specialized_annotation_handler` /
  `specialize_annotation_runtime_handlers` /
  `compile_annotation_wrapper`.
- C09xx discipline: S5b mints NO code (the minted set stays exactly
  S5a's [C0926] + [C0931]); the new S5b sentences are G13
  string-tag-uncoded with the #60 routing posture.
