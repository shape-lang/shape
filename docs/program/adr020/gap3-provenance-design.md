# #240 — gap 3 of the `FrameReturnWrapper::Unknown` deletion: design

**Status:** design complete; roots A1 and B **implemented and landed**, root C
diagnosed but not started. Branch `gap3-provenance`, based on `main@dac5fe7e`.
See §8 for the landed state, the remaining 11 residuals, and the routing call
that `#255`'s split now requires. Sections 1–7 are the original design and are
preserved as written except where a later measurement corrected them; those
corrections are marked in place rather than edited away.

**Headline verdict:** both roots named on #240 are **refuted**. Neither
generated-node provenance (ADR-009 C1) nor comptime-block return-type pinning is
required to close gap 3. 76 of the 99 measured residual stamps — including all
18 of the "dummy span" class and all 62 of the "comptime wrapper" class — are a
single wiring gap: **the comptime builtin forwarder table drops the declared
return type that the intrinsic catalog already carries.** Four table rows,
measured, close them. The 23 that remain split into two genuinely different
roots, neither of which is provenance either.

---

## 1. Re-measurement

The 107/20/62 split on the ticket is an inherited claim. It was re-derived from
scratch at this branch's HEAD.

**Probe.** `FrameReturnMetadata::unclassified_residual()` is the single site in
the workspace that stamps `FrameReturnWrapper::Unknown`
(`crates/shape-vm/src/compiler/helpers.rs:1392`, consumed at `:4217`). The probe
replaces the `unwrap_or_else(FrameReturnMetadata::unclassified_residual)` at
`helpers.rs:4217` with a `panic!` that prints the unclassified function's
registry name, its body's tail-statement kind, and — via a throwaway helper
beside `infer_closure_body_return_type_with_caller_context`
(`crates/shape-vm/src/compiler/expressions/closures.rs:3011`) — **which of that
helper's three decline paths fired**.

**Command** (run from the worktree; the devenv toolchain is not auto-loaded):

```
direnv exec /home/dev/dev/shape-lang cargo test -p shape-vm --lib --jobs 4
```

**Control.** With no probe applied, `main@dac5fe7e` has **8 pre-existing
failures** in this target (listed in §6.3). The inherited "107" therefore
reproduces exactly: 8 pre-existing + 99 probe panics = 107. This measurement saw
106 (one known-flaky test differed), of which **99 are probe panics** — the
number that matters.

### 1.1 The measured partition

The classification chain's fourth source declines in exactly three places
(`closures.rs:3025-3044`): the body has no terminal expression; the terminal
expression's span is a dummy; or the span is real but absent from the post-solve
span table. Every residual falls into one of the three:

| Decline path | Count | The ticket's claim |
|---|---:|---|
| `SPAN_MISS` — real span, absent from `resolved_expr_types` | 73 | "62, the comptime wrapper" |
| `DUMMY_SPAN` — terminal carries `Span{0,0}` | 18 | "~20, generated-node provenance" |
| `NO_TERMINAL_EXPR` — body has no value-producing tail | 8 | not named |

By generated-function family:

| Function family | n | Decline path | Terminal expression |
|---|---:|---|---|
| `HygienicRole::ComptimeHandlerWrapper` | 62 | `SPAN_MISS` | `FunctionCall` |
| `__w27_implicit_error_string` | 13 | `DUMMY_SPAN` | `QualifiedFunctionCall` |
| `__closure_0` | 8 | 7 `SPAN_MISS`, 1 `NO_TERMINAL` | `BinaryOp`×4, `MethodCall`, `FunctionExpr`, `Comptime` |
| `__w27_implicit_item_fn_string_string_int` | 3 | `DUMMY_SPAN` | `QualifiedFunctionCall` |
| `HygienicRole::AnnotationSugarHookBody` (2 nonces) | 4 | `NO_TERMINAL` | — |
| `Conn::drop_async`, `AsyncRes::drop_async` | 2 | `NO_TERMINAL` | — (empty bodies) |
| `__w27_implicit_warning_string` | 1 | `DUMMY_SPAN` | `QualifiedFunctionCall` |
| `install` | 1 | `DUMMY_SPAN` | `QualifiedFunctionCall` |
| `note::…::c3_before_hook::cfg#1::i:7` | 1 | `NO_TERMINAL` | — |
| `__w24_method_Number_tripled_329_340_number` | 1 | `SPAN_MISS` | `BinaryOp` |
| `__w27_implicit_std__core__distributions__dist_uniform_number_number` | 1 | `SPAN_MISS` | `FunctionCall` |
| `__w27_implicit_schema_for_string` | 1 | `SPAN_MISS` | `QualifiedFunctionCall` |
| `myext::connect` | 1 | `SPAN_MISS` | `QualifiedFunctionCall` |

The hygienic function names are 128-bit digests of a role descriptor plus a
nonce (`HygienicSymbol::mint`,
`crates/shape-vm/src/compiler/comptime_builtins/expansion_provenance.rs:499`).
Recomputing the digest for each `HygienicRole` at nonce 0 identifies the 62
bucket as `ComptimeHandlerWrapper` — the **annotation-handler** mini-program
entry wrapper (`comptime.rs:3276`), not `ComptimeBlockWrapper`. This matters:
`ComptimeBlockWrapper`'s digest (`456eea837957f9463b2df34f4ffb262f`) does not
appear anywhere in the residual set. **A `comptime { }` block's entry wrapper
already classifies today.**

### 1.2 The residual population is masked, so every count is a lower bound

The probe panics, so a test reports only its FIRST unclassified function. As
fixes land, previously hidden residuals appear: `NO_TERMINAL_EXPR` went 8 → 12
across the experiments below without any change that could create new
unclassified functions. The close-out sequence in §5 must therefore iterate to a
fixpoint, not run once against a fixed list.

---

## 2. The two roots on the ticket, refuted

### 2.1 Root 1 — "dummy spans on synthesized bodies, ADR-009 C1 provenance"

**Refuted. Measured to zero without touching provenance.**

The 17 `__w27_implicit_*` dummy-span residuals are implicit-generic
specialisations of the **comptime builtin forwarders**. A forwarder is generated
at `comptime.rs:643` (`comptime_builtin_forwarders`) with the body
`Statement::Return(Some(Expr::QualifiedFunctionCall { … span: Span::DUMMY }))`.
The inference engine deliberately refuses to key a fact on a dummy span
(`crates/shape-runtime/src/type_system/inference/expressions.rs:344`: dummy spans
"collide on `(0,0)` and would alias unrelated expressions") — that refusal is
correct and must stay.

But the forwarder does not need a span-keyed fact at all. Its return type is
**declared**: `register_typed_function(&mut module, "warning", …,
ConcreteType::Unit, …)` at `comptime_builtins.rs:1643`, and the same for `error`
(`:1661`), `install` (`:2208`), `item_fn` (`:1818`,
`ConcreteType::OpaqueTypedObject("__CheckedItem")`). The forwarder-generating
table `COMPTIME_BUILTIN_FORWARDERS` (`comptime.rs:147`) carries its own
hand-written `named_return_type` column and has **`None` for exactly these
rows**, so the generated `FunctionDef` is emitted with `return_type: None` and
the declared type is discarded.

**Measured.** Setting `named_return_type` from the intrinsic's declared
`ConcreteType` for `warning`, `error`, `install` and `item_fn` — four table rows,
no other change — takes `DUMMY_SPAN` from **18 to 0**.

ADR-009 C1 generated-node provenance is not required for gap 3 and should not be
pulled into #227's final act.

### 2.2 Root 2 — "comptime mini-program wrapper; inference does not pin a `comptime { }` block's return type"

**Refuted, on three counts.**

1. **It is not the `comptime { }` block wrapper** (§1.1): it is
   `ComptimeHandlerWrapper`, the annotation-handler wrapper.
2. **It is not "`resolved_expr_types` does not cover the wrapper's tail".** With
   `GAP3_FINALIZE_TRACE` instrumentation on `finalize_expr_type_table`
   (`inference/mod.rs:471`), the mini-program compile for
   `compiler::statements::tests::test_annotated_function_generates_wrapper`
   reports `pre=2 post=0` — inference runs, records two expression types, and
   both are dropped as still-free after substitution. The table is not partial;
   it is empty.
3. **The free variable has a single, named cause.** All 62 wrappers have the
   identical tail: `callee=install nargs=1`, with `install` present in
   `function_defs` carrying `return_type: None`. `install` is a comptime
   forwarder — the same table, the same dropped column, the same root as §2.1.

**Measured.** Wiring `install`'s declared `ConcreteType::Unit` through the
forwarder table takes the 62-bucket to **0**, and takes the whole probe from
**106 → 34** failures. Adding `item_fn` reaches **31**.

**The premise the brief asked me to test — "an eliminated wrapper is a better
outcome than a wrapper with a special inference rule" — is moot.** No special
inference rule is needed and no wrapper elimination is needed: with its callee's
declared return present, the wrapper classifies through the ordinary annotation
path. Eliminating the wrapper (binding handler params as module bindings and
placing `handler_body` directly as the trailing `Item::Expression`) is possible,
but it would be a large behavioural change to the comptime mini-program's scoping
model bought for a problem that no longer exists. **Recommend not doing it.**

### 2.3 The refused shortcut was never reachable

The explicitly-refused shortcut (propagating the original function's inferred
return onto a `__w27_implicit_*` specialisation) never came into play.
`implicit_specialization_return_concrete_type`
(`function_calls.rs:7718`) is already the substitution-aware version: it binds
each param name to the call site's `ConcreteType` and walks the tail under that
substitution. It declines on the forwarders for a different and correct reason —
`implicit_specialization_expr_type` has no `Expr::QualifiedFunctionCall` arm, so
a forwarder body is unreachable to it. Once the forwarder declares its return,
the function short-circuits at its first line (`if let Some(return_type) =
func_def.return_type`) and never walks the tail. The shortcut stays refused and
is not needed.

---

## 3. The real root inventory

After the four forwarder rows, **23 probe panics remain** in three roots. Roots A
and C are the same architectural shape at two different bridges.

### Root A — a declared contract in a side table is filtered on its way into the AST tier

This is the dominant root (76 of 99 measured, plus part of the remainder).

Two instances, same shape:

**A1. Comptime intrinsic catalog → forwarder `FunctionDef`.** `ModuleExports`
entries registered by `register_typed_function` carry a declared return
`ConcreteType` and declared `ModuleParam` types. `COMPTIME_BUILTIN_FORWARDERS`
(`comptime.rs:147`) re-declares that contract by hand in three columns
(`return_fields`, `named_return_type`, `param_annotations`) and has drifted. Full
audit of the 24 rows against the catalog:

| Row | Catalog declares | Table declared | Verdict |
|---|---|---|---|
| `warning` | `Unit` | *(none)* | drift — closed by A1 |
| `error` | `Unit` | *(none)* | drift — closed by A1 |
| `install` | `Unit` | *(none)* | drift — closed by A1 |
| `item_fn` | `OpaqueTypedObject("__CheckedItem")` | *(none)* | drift — closed by A1 |
| `extend_method` | `OpaqueTypedObject` | *(none)* | drift — closed by A1, not exercised by a probe failure |
| `extend_method_literal` | `OpaqueTypedObject` | *(none)* | drift — closed by A1, not exercised |
| one string-literal producer row | `String` | *(none)* | drift — closed by A1 |
| other 17 rows | various | present | agree |

**Correction to an earlier claim in this document.** The string-literal
producer row was first reported here as having "no discoverable registered
intrinsic". That was a regex artifact of the audit script, not a fact: a probe
that enumerated all 24 rows against the built `ModuleExports` reported
`GAP3_CAT_MISSING []` — **every** forwarder target has a catalog entry, and that
row's is `ConcreteType::String`. No row needs a hand-declared return, and open
question 2 below is answered.

ADR-011 §Resolved intrinsic identity makes the catalog the authority: intrinsic
behaviour is selected by a resolved `IntrinsicId` "with its exact
type/effect/ownership/stage contract validated". A hand-maintained parallel
declaration of that contract is precisely the thing that ruling forbids, and it
has drifted in 7 of 24 rows.

**A2. Extension `ModuleExports` → inference.**
`build_module_qualified_scalar_returns` (`function_calls.rs:6259`) is the
existing bridge that hands module-export return types to the analyzer. It reads
`module.get_schema(export).return_type` and then **drops every return that is not
a canonical scalar alias** (`:6302` — `canonical_script_alias(...) else
continue`). `myext::__connect` and `myext::__connect_codegen` return a
table/TypedObject schema, so their declared returns are filtered out and the
calls stay free variables. Same shape as A1: a real declaration, a lossy bridge.

**Fix (both instances).** Delete the hand-maintained authority and derive from
the catalog.

- A1: generate each forwarder's `FunctionDef.return_type` (and param
  annotations) from the registered intrinsic's declared contract. The mapping is
  total and small: `Unit`/`Void` → `TypeAnnotation::Basic("void")` (accepted by
  `concrete_type_from_annotation`, `v2_map_emission.rs:31`);
  `OpaqueTypedObject(name)` → `Basic(name)`; `Object` → the declared field
  annotation the `return_fields` column already builds; scalars → their names.
  The three columns then have no remaining authority and are deleted.
- A2: widen the bridge from "scalar aliases only" to the full declared return
  contract, or state in the ADR why a non-scalar module export deliberately has
  no inference-visible return type. This one may be larger than it looks and
  should be sliced separately from A1.

**Mechanical enforcement.** A unit test asserting that every
`COMPTIME_BUILTIN_FORWARDERS` row generates a `FunctionDef` whose `return_type`
is `Some(_)` and agrees with the registered intrinsic's declared `ConcreteType`.
Without it the table drifts again.

**One catalog question this surfaces.** `error` is registered
`ConcreteType::Unit` but its implementation returns `Err(...)` and aborts
comptime execution — it diverges. Wiring from the catalog faithfully reproduces
`Unit`. If the catalog is wrong, that is a real but separate defect; it does not
block gap 3.

### Root B — "falls off the end ⇒ returns nothing" is never concluded (12 measured)

Members: 8 `AnnotationSugarHookBody` observer bodies, the 2 empty-bodied
`*::drop_async` trait methods, 1 `__closure_0`, 1 `c3_before_hook` body.

Measured body shapes: 2 have **zero statements**; 9 end in a `VariableDecl` with
no value-returning `return` anywhere; 1 is `Return(Some(Block))` whose block ends
in a `VariableDecl`. Every one of them produces no value. Nothing needs to be
inferred — the fact is structural.

The mint site says so out loud: `sugar_lowering.rs:452` builds the observer form
as "`fn <minted>(<config>) { body }` — concrete, zero signature params, **no
return** (F1; target-uniform by design)" and emits `return_type: None`. Under
strict typing "returns nothing" and "return type not stated" are different
facts, and the generator knows which one it means.

**Fix, in two parts.**

1. **At the generators.** Where a synthesized function is known to return
   nothing, spell it: `return_type: Some(TypeAnnotation::Basic("void"))`, not
   `None`. `sugar_lowering.rs:452` (observer form) is the measured instance. This
   is the ADR-011-correct fix — the generator holds the contract, so the
   generator states it.
2. **At the classifier, for user-authored bodies** (`Conn::drop_async` and
   friends are ordinary source functions with no annotation). Make the terminal
   finder total: `closure_body_terminal_expr` should return a classification —
   *this expression is the value* or *this body produces no value* — instead of
   `Option<&Expr>`, where `None` currently conflates "no value" with "cannot
   tell".

**The naive rule is unsound and must not be used.** "Terminal finder returned
`None` ⇒ `Void`" also fires for a body whose last statement is an `if/else` in
which both branches `return v`, and for a `loop` with `break v` — neither
returns unit. `stmt_terminal` (`closures.rs:3067`) returns `None` for both. The
sound predicate is *the body does not diverge and has no value-producing tail*,
and the machinery already exists in the inference tier: `expr_diverges` /
`stmt_diverges` / `body_diverges` at
`crates/shape-runtime/src/type_system/inference/expressions.rs:240-287`. Route
the rule through those, not through the terminal finder's `None`.

Once concluded, the classification is `ConcreteType::Void` → `FrameReturnArity::
Zero` via the existing `return_metadata_from_concrete_type` (`helpers.rs:1429`),
which is a positive fact per ADR-020 §3.3 and makes the descriptor useful rather
than merely present.

### Root C — genuine span-table misses on closures and generated methods (11 measured)

7 `__closure_0` (`BinaryOp`×4, `MethodCall`, `FunctionExpr`, `Comptime` tails),
1 `__w24_method_*` (`BinaryOp`), and the 3 A2 cases above.

Sub-shape C1: **three of the four `BinaryOp` cases report `table_len=0`** — the
closure is compiled in a context where no span table is installed at all
(comptime mini-programs, module-scope compiles). No amount of span-keyed lookup
helps there; the type must come from the same place the enclosing compile got
its types.

Sub-shape C2: the rest have a populated table (`table_len` 3/4/9/15/29) and the
terminal's span is genuinely absent — `finalize_expr_type_table` dropped it as
still-free. Each needs its own diagnosis; this is the smallest and least
understood root and should be sized only after A and B land, because the masking
in §1.2 means its true population is not yet known.

---

## 4. What is explicitly NOT the answer

**`FrameDescriptor.return_wrapper: Option<FrameReturnWrapper>`.** It is tempting:
`return_kind: Option<NativeKind>` is already accepted on the same struct with the
meaning "kind not stamped", and the census's class (iii) advice is "should be
typed as a builder-`Option`". It is still refused. The runtime consumers
(`propagate_none_early_return`, `bytecode_function_returns_option`) branch on
this field, so an `Option` there is a runtime-consumed absence — census class
(i), the same fact `Unknown` encodes, under a less suspicious name. That is the
CLAUDE.md §Forbidden "rename to a less suspicious name" shape, and the owner's
2026-07-29 ruling ("if Unknown is a runtime thing, it should not exist") rules it
out directly. Recorded here so it is not rediscovered as a shortcut; it belongs
in `docs/defections.md` if anyone proposes it again.

**Reading an unclassified return as `Plain`.** Already refused on the enum and
still correct: `Plain` is the positive claim "returns no fallible wrapper", and
asserting it for a `Result`-returning frame silently changes `?` semantics.

---

## 5. Close-out sequence for #227's final act

Each step is gated; a step that does not meet its gate stops the sequence rather
than proceeding with a documented residual.

**Step 0 — land the probe as a temporary hard error, not as a commit.** The
sequence is driven by the probe from §1. It is never committed; it is re-applied
at each step to re-measure.

**Step 1 — Root A1: derive forwarder signatures from the intrinsic catalog.**
Delete the `return_fields` / `named_return_type` / `param_annotations` authority
in favour of the registered contract; add the drift test from §3.
*Gate:* probe `DUMMY_SPAN` = 0; probe total ≤ 31 panics; `just verify-merge`
green; failing-test NAME SET vs the §6.3 baseline shows no additions outside the
two known-flaky families.

**Step 2 — Root B: conclude "returns nothing".** Generator-side `-> void` at the
known sites, plus the divergence-backed classification for user-authored bodies.
*Gate:* probe `NO_TERMINAL_EXPR` = 0 **and** the re-measure is repeated after the
change, because §1.2 masking will reveal residuals this step's own fix uncovers.

**Step 3 — Root C + Root A2: closures, generated methods, module exports.**
Size only after steps 1–2, against a fresh measurement.
*Gate:* probe total = 0.

**Step 4 — restore the hard error permanently.** Replace
`FrameReturnMetadata::unclassified_residual()` at `helpers.rs:4217` with a
`ProofGap`-shaped surface-and-stop, matching the `prove_native_kind()` pattern
(`compiler/type_tracking.rs`) so emit code cannot fabricate a classification.
*Gate:* `cargo test -p shape-vm --lib` at the §6.3 baseline failure set exactly.

**Step 5 — delete `FrameReturnWrapper::Unknown`.** Remove the variant, the
`unclassified_residual` constructor, and the four-producer-class documentation
block on the enum (`type_tracking.rs:120-180`), which this document supersedes.
Update `docs/program/adr011-012/runtime-unknown-census.md` §(i-1) to record the
close.
*Gate:* `just check-no-dynamic`; `just verify-merge` on both trees.

**Step 6 — regenerate the embedded stdlib.** The variant's removal shifts the
`FrameReturnWrapper` discriminant, so every serialized `FrameDescriptor` in the
embedded artifact is invalidated. Under §Greenfield there is no dual reader and
no migration: old artifacts fail to load.
*Gate:* **explicitly verify the artifact was regenerated and is loaded.** #233's
close recorded that the embedded stdlib was stale at main and that a decode
failure **silently falls back to source compilation** (`stdlib.rs:54`) — so a
forgotten regeneration produces a green suite and a wrong artifact. Assert the
artifact decodes rather than inferring it from a passing test run.

**Step 7 — corpus differential.** `cargo build --release --bin shape` first
(`run-diff.mjs --fresh` does not rebuild), then the vm/jit corpus differential;
`git checkout --` the `ACC__native-c__probe2.lock` timestamp afterwards. Never
`just diff-vmjit-corpus`.

---

## 6. Acceptance

### 6.1 The probe at zero

`unclassified_residual` is a hard error and `cargo test -p shape-vm --lib`
returns to the §6.3 baseline failure set. This is the primary acceptance
criterion and it is self-enforcing after step 4: with the site a
surface-and-stop, an unclassified return is a compile error, so the probe cannot
regress silently.

### 6.2 Regression coverage that keeps the roots closed

The probe disappears at step 4, so each root needs coverage that outlives it.

- **Root A1:** the drift test — for every `COMPTIME_BUILTIN_FORWARDERS` row, the
  generated `FunctionDef.return_type` is `Some(_)` and agrees with the intrinsic
  catalog's declared `ConcreteType`. This is the test that would have prevented
  the whole gap, and it is cheap and total.
- **Root A2:** a test that a non-scalar-returning extension export is visible to
  inference at a module-qualified call site (the `myext::connect` shape).
- **Root B:** a test that a function whose body falls off the end is stamped
  `FrameReturnArity::Zero`, plus the **unsoundness guards**: a body whose last
  statement is an `if/else` with both branches returning a value, and a `loop`
  with `break v`, must NOT be stamped `Zero`. Those two are the tests that stop
  the naive rule from being reintroduced.
- **Root C:** per-case, once diagnosed.
- **Cross-cutting:** a sentinel test asserting `FrameReturnWrapper` has exactly
  three variants, in the shape of
  `crates/shape-vm/src/executor/tests/no_dynamic.rs`, so the variant cannot come
  back under a new name.

### 6.3 The baseline failure set

`main@dac5fe7e`, `cargo test -p shape-vm --lib`, no probe: **8 failures.**

```
compiler::expressions::advanced::tests::test_async_let_binding_is_immutable
compiler::expressions::advanced::tests::test_match_arm_empty_array_unprovable_element_is_clean_compile_error
compiler::monomorphization::cache::route_tests::inlined_closure_keeps_outer_authored_type_ref_in_its_parameter_scope
compiler::monomorphization::cache::route_tests::nested_exact_calls_close_outer_arguments_before_inner_compilation
compiler::monomorphization::cache::route_tests::unavailable_and_missing_callsite_evidence_execute_only_in_legacy_domain
compiler::monomorphization::type_resolution::tests::ws6_generic_id_ok_arg
compiler::monomorphization::type_resolution::tests::ws6b_inferred_result_variable_arg
executor::foreign_async::tests::<one name, varies per run>
```

`route_tests` and `foreign_async` are the two known-flaky families; compare
failure NAME SETS, never counts, and treat a differing `foreign_async` name as
the flake rather than a regression.

---

## 7. Open questions for the owner

1. **`error`'s declared return type.** The catalog says `ConcreteType::Unit`; the
   implementation diverges (`Err`, aborting comptime execution). Deriving from
   the catalog is right either way, but if Shape wants a `never`/divergent return
   type this is where it first bites. Not blocking.
2. ~~The string-literal producer row's missing catalog entry.~~ **ANSWERED by
   measurement** — see the correction under §3 Root A1. Every forwarder target
   has a catalog entry; the gap was in the audit script, not the code. A1's
   drift test is total over all 24 rows.
3. **Root A2's scope.** ~~May deserve its own ticket.~~ **RULED 2026-08-01:
   split to #255.** Widening `build_module_qualified_scalar_returns` beyond
   scalar aliases touches extension typing generally and interacts with #207's
   named-type schema registration. #227's final act no longer waits on it —
   which means the probe cannot reach zero in this lane; see §8.

Nothing in this design requires an ADR amendment. In particular the ADR-009 C1
generated-node-provenance shape is **not** needed — that is the main thing this
measurement changes about the plan.

---

## 8. Implementation status (2026-08-01)

Authorized to leave design phase after the refutation was verified. A1 and B are
landed and measured; C is diagnosed but not implemented.

| Root | Commit | Probe panics after | Clean-run failures |
|---|---|---|---|
| *(design baseline)* | `82dd9e53` | 99 | 8 |
| **A1** — forwarder returns derived from the catalog | `d4de20b7` | 23 | 8 |
| **B** — a body producing no value classifies as unit | `4b84e2ed` | 11 | 8 |
| **C** — closure / generated-method span misses | not started | — | — |

"Clean-run failures" is `cargo test -p shape-vm --lib` with no probe applied,
and is the same failure NAME SET as the `dac5fe7e` control at every step (§6.3).
Neither root regressed anything.

A1 closed the drift CLASS, not the four rows the experiment needed: the
`named_return_type` column is deleted, all 24 rows derive, and a row that
resolves to no registered intrinsic — or whose declared type has no annotation
spelling — is a hard error at generation time.

### 8.1 The remaining 11, by root

| n | Root | Members |
|---:|---|---|
| 7 | **C** — closures | `__closure_0` in 7 tests; tails `BinaryOp`×4, `MethodCall`, `FunctionExpr`, `Comptime` |
| 1 | **C** — generated method | `__w24_method_Number_tripled_329_340_number`, tail `BinaryOp` |
| 2 | **A2 → #255** | `myext::__connect`, `myext::__connect_codegen` — extension-module exports |
| 1 | **A2-adjacent** | `__intrinsic_dist_uniform` — a registered intrinsic absent from `function_defs`, so a specialisation's tail walk cannot resolve it. Same shape as A2 (declared contract in a side table); needs a routing call between #255 and this ticket |

**Three of the seven closure cases report `table_len=0`** — the closure is
compiled where no span table is installed at all
(`failed_shadow_emission_restores_body_analysis_authority`,
`replacement_mir_uses_only_its_own_distinct_closure_identity`,
`b5_const_in_closure_body_is_substituted`). No span-keyed lookup can help those;
the type has to come from wherever the enclosing compile got its own.

### 8.2 The probe cannot reach zero in this lane

With A2 split to #255, 2 (arguably 3) of the 11 residuals are out of this
ticket's scope, so "probe at fixpoint zero" and "A2 is #255's" cannot both hold
here. The close-out sequence in §5 needs one of:

- **(a)** #255 lands first and #227's final act waits on it after all; or
- **(b)** #227's final act is gated on "probe zero **excluding** the A2 members",
  with those members' descriptors provably unreachable from the stamp site — a
  claim that has to be demonstrated, not assumed; or
- **(c)** the A2 members are re-pulled into this ticket.

This is an owner/supervisor routing call, not a technical one. Until it is made,
step 4 (restoring the hard error) cannot be scheduled — restoring it while any
residual remains turns those tests into hard failures.

### 8.3 Process note

The probe's revert was `git checkout --` on `helpers.rs` and `closures.rs`.
Applied while root B was still uncommitted in `closures.rs`, it silently
discarded that work; it was caught only because the follow-up test run failed to
compile. The probe script now snapshots both files at apply time and restores
from that snapshot. Anyone re-running this measurement mid-change should verify
the same, or commit before probing.
