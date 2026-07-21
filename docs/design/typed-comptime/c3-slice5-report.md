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
