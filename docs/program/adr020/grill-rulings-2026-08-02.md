# ADR-020 program — owner grill rulings, 2026-08-02

Seven questions put to the owner in a grill-with-docs session; every ruling
below is an owner decision made against measured evidence (evidence briefs
were assembled per-item by independent readers; load-bearing facts are
restated here with their citations so this doc stands alone). Enactment was
approved as a package the same day.

---

## R-G1. `never` / divergent type — FULL bottom type in the catalog tier

**Ruling.** Add `ConcreteType::Never` and collapse the three inconsistent
divergence encodings onto the **existing** bottom type. This is not
introducing `never` — the surface language and solver already have it
(grammar `shape.pest:1035`, parser `types.rs:159`, AST `types.rs:97`,
solver bottom rule `constraints.rs:478-479`, union absorption, SemanticType,
comptime reflection, foreign ABI, JIT FFI conversion). The intrinsic
catalog's `ConcreteType` (`typed_module_exports.rs:267-351`) is the one tier
that cannot express it, and that gap produced three divergent-native lies:

1. `error` declares `ConcreteType::Unit` (`comptime_builtins.rs:1661-1665`)
   while its own doc comment says `-> never` and the body has no `Ok` path.
2. `state::resume` declares `ConcreteType::Named("never")`
   (`state_builtins/core.rs:317`) — inert in inference, does not round-trip
   (`Basic("never")` ≠ `TypeAnnotation::Never`), yet the book documents
   `-> never`.
3. `exit` declares void (`environment/mod.rs:283`) over
   `std::process::exit` (`vm_impl/builtins.rs:425`).

**Binding conditions** (each verified as a live trap):

- `forwarder_return_annotation` (`comptime.rs:612-637`) has **no Never
  arm** — a Never declaration is a hard generation error today. Add the arm.
- `concrete_type_from_annotation` (`v2_map_emission.rs:20-89`) has **no
  Never arm** — a naive flip re-opens the exact `FrameReturnWrapper::Unknown`
  gap #240 closed. `Never` answers `FrameReturnArity::Zero`
  (`type_tracking.rs:203-208`).
- Divergence analysis becomes **type-driven**: `expr_diverges` /
  `stmt_diverges` / `body_diverges` (`inference/expressions.rs:240-292`)
  currently recognize only `return`/`break`/`continue` — AST-shape-selected
  semantics, the §Forbidden-Patterns family. The reorder was measured
  mechanical (the syntactic check is computed adjacent to inference at all
  four call sites); `Expr::Return`/`Break`/`Continue` type as `Never`.
- `field_types.rs:520-523`'s `unreachable!()` on bottom-type storage becomes
  a proper diagnostic **before** `error` types as Never — otherwise
  `if c { error("x") } else { 1 }` panics the compiler.
- Two pinned tests re-baseline: the drift test's Unit branch
  (`comptime.rs:6320-6394`) and `test_comptime_error_builtin`
  (`comptime_builtins.rs:4965`).

Adjacent defect noted in passing (fix in the same territory or split):
`exit`'s param is declared `number` (f64) while the runtime reads it as int
(`builtins.rs:421-425`).

Non-blocking; scheduled after the #239 flip. Tie-in: #235's two
uninhabited `bounds` arrays (Class D) get element type `never` once
expressible.

## R-G2. Vacuous corpus — tripwire-bound acceptance

**Ruling.** The measured state (11/488 native-executing, 475/486 MATCHes
vacuous, every native program single-dispatch) stays accepted **while the
#239 flip is the active workstream**. Tripwire: if the flip has not landed
with the native rate restored within ~two working sessions, a dedicated lane
immediately builds an interim positive-control instrument. The tripwire
clock starts when #260's extent fix lands (see R-G6 — the flip's
pre-baseline waits on it). No interim micro-corpus before that: the flip's
own acceptance gate (`just check-jit-native-acceptance`) covers the blind
window, and the corpus becomes non-vacuous again as a consequence of the
flip, not by curation.

## R-G3. #252 async permission gating — owned context across await

**Ruling.** Split `ModuleContext`: the permission-relevant part
(`PermissionSet` + `ScopeConstraints`) becomes an owned, Arc-backed context
passed **explicitly as a parameter** in the async invoke ABI
(`TypedModuleAsyncFunction::invoke`). Async bodies then gate exactly like
the wired sync half (`file.rs:70-76`, `env.rs:40-43`,
`network_ops.rs:87,135`): check immediately above the I/O call, scoped
checks against the concrete host/path argument.

Distinguishing reasons: the `PermissionSet` is per-run immutable, so an
owned snapshot at spawn is exact (no TOCTOU); a spawn-site-only check
cannot enforce `ScopeConstraints` (needs the concrete argument at the I/O
site); a task-local channel was rejected as implicit out-of-band state.
ABI change is free under §Greenfield. Acceptance unchanged from the ticket:
`http.*` provably refuses without `NetConnect` through the CLI on both
tiers, and the lying `http.rs:7` header comment dies in the same commit.

## R-G4. #235 `FieldType::Any` — DELETE OUTRIGHT

**Ruling.** No bounded survivor, no rename. Architecture statement:

> Unknown is representable only in the inference tier
> (`Type::Variable` / `ProofGap`). The schema tier has no unknown state —
> a schema can only be minted from resolved types.

Post-inference Any therefore becomes **unconstructible**, not merely
checked: day one of the migration privatizes `FieldType::Any` construction
to the migration module (the `ProofGap` pattern), so no new site can be
written mid-migration. E0900 (`post_inference_verify.rs`) and its 33-row
allow-list are **deleted with the variant**, not maintained.

Staging (cheapest-first, each class as measured by the census):

1. **A/C/F first** — Class A inference gaps (~9 sites where the type is
   computed and discarded, e.g. `function_calls.rs:1380-1390`;
   `collections.rs:937`; the MIR back-patch), Class C one-liners (6
   `array_field(_, Any)` sites with typed elements available), Class F
   runtime all-Any schema synthesis (`type_schema/mod.rs:186-213` +
   `registry.rs:712-737`). This closes the measured object-literal JIT
   cliff (anonymous literal → whole-function deopt; identical declared-type
   program runs native) and deletes the forbidden `object_field_contracts`
   parallel table (`type_tracking.rs:825`).
2. **B second** — genuinely heterogeneous stdlib carriers (~11 field
   positions: VM-state introspection, `__Option`/`__Result` payloads).
   Genuine per-element dynamism lives in the **value-tier kind track**
   (ADR-006 §2.7.7 — legitimate), never in a schema claim.
3. **E last, own design ticket** — every enum's `__payload_N`
   (`schema.rs:239`): per-variant typed payload layout, materializing the
   compiler-side per-variant kind track that already exists.

Class D (2 provably-empty `bounds` arrays) resolves via R-G1's `never`
element type. The shrink-only ratchet (CHECK 15, baseline 223) re-baselines
downward at each stage.

## R-G5. #237 — downgrade CONFIRMED, scope broadened to the structural fix

**Ruling.** The downgrade to mechanical is confirmed (all premises
independently verified: no serde, never serialized, sole production
consumer returns `Err` unconditionally, nothing branches on the enum), and
the scope is **broadened** to the structural fix: collapse
callables/results/external_receivers/pending_async into **one
`task_id → TaskState` map carrying the driver inside the variant**, so
status is *read*, never reconstructed — fully discharging the #233
"no runtime inference" ruling at this site.

Binding: variant naming follows the traced semantics — the conflated arm
means "entry exists, Pending, driver checked out (executing now, or
orphaned after a failed driver)", NOT the issue's "NotStarted" or the
comment's "Registered", both refuted by the mutator trace. The misleading
string is user-facing (`render_capture_barrier`, `snapshot.rs:708-727`,
passes Future-family barrier text verbatim) and gets curated language like
every other family. Risk surfaces for the lane: the share-accounting `Drop`
(`task_scheduler.rs:599-624`), and the `TaskState` shape must be
extensible by the W17 resume path (which will be the first real
brancher on these states), not replaced by it.

## R-G6. #260 now, pre-flip; #256 rides the flip close-out

**Ruling.** #260 is assigned immediately, scoped to three parts, and must
land **before the #239 flip's pre-baseline reading**:

1. **Extent fix** at `verifier.rs:221-281`: extents are computed
   `entry_point → next entry_point` and never consult `body_length`, so the
   zero-length permission-carrier functions synthesized by
   `publish_dependency_permission_blob` (`import_permissions.rs:284-349`)
   inherit the whole instruction range of the following real function and
   report its V2 typed opcodes as violations. The entire reported class is
   this false positive (`non_carrier_violations = 0` across all sampled
   runs); the run-to-run count variation is upstream `HashMap` iteration
   order (`import_permissions.rs:44`), not verifier disorder — "make the
   verifier deterministic" targeted the wrong layer, and the rename remedy
   is mooted once the class dissolves. The JIT leg *enforces* this false
   positive into whole-program bails (`executor.rs:387-408`,
   `V2VerifierUnverified`), so the #187 bail-inventory ratchet re-baselines
   **in the same commit**.
2. **`reasonClass` field** in `readNativity()`
   (`tools/vmjit-diff/run-diff.mjs:187-199`), which currently discards the
   fallback reason — the one field that can answer what share of the ~407
   whole-program fallbacks #260 explains.
3. **Release-wording alignment** at `vm_impl/program.rs:104-146`: the VM
   leg prints "failed" in release and "warning" in debug for identical
   print-and-continue behavior; the debug wording is the honest one.

Bounded exposure, for the record: the verifier writes stderr and the
differential's `classify()` reads stdout + exit code, so MATCH/DIVERGED
counts are insulated — only the nativity denominator moves, which is
exactly why this precedes the flip's baseline.

**MEASURED OUTCOME (2026-08-03, appended per the testable-premise
discipline):** the ruling's premise — #260 as a candidate first-order cause
of the 11/488 denominator — was **refuted by the lane's measurement and
confirmed by supervisor re-derivation**: `v2-verifier-unverified` accounts
for 0 of the 407 whole-program fallbacks (earlier preflight bails win
first; `record_program_fallback` is first-wins). The extent fix landed on
its own merits (`b5a66808`): corpus verifier-line emissions 360/488 → 3/488,
the 3 survivors being TRUE `__main__` violations (#240 territory); the
native denominator did not move. The pre-flip ordering was cheap insurance
that bought a clean attribution, not the expected recovery. **The R-G2
tripwire clock started 2026-08-03.** Criterion (a) of the ticket was moot,
not satisfied — the upstream `HashMap` iteration order
(`import_permissions.rs:44`) remains, now invisible through the verifier.

**#256** folds into the flip close-out — the only ordering under which its
"proven to bite" acceptance is satisfiable, because the live inline tag
reconstruction (`places.rs:523-551`, re-spelling `0xFFF8...` so even a
symbol row misses it) must die first for a bit-pattern row to seed at
zero. The flip close-out also adds the **missing `is_tagged` row** (a live
`fn is_tagged` at `value_ffi.rs:104` has no baseline row today) and runs
the audit at its true denominator: 33 pattern rows, not 45 lines.

## R-G7. Parked inventory

- **#236 — widened and deleted pre-flip.** The true census is the 9-site
  union: 7 live JIT fabricating kind-defaults (`conversions.rs:30`,
  `blocks.rs:90`, `statements.rs:820`, `rvalues.rs:330`, `rvalues.rs:619`,
  `osr_compiler.rs:225`, `v2_call_abi.rs:309`) plus 2 same-shape non-JIT
  siblings outside both prior lists (`object_creation.rs:551`,
  `stdlib/json.rs:412`). All die as one #239 prerequisite slice before
  monomorphization (design doc §4.0.1: an unguarded monomorph reshapes the
  defect), surface-and-stop replacing each default. **The disputed UInt64
  row (`rvalues.rs:619`) dies with the rest** — owner verdict: if the kind
  is truly the documented §2.7.5 carrier, the compiler can stamp it; a
  default is either unnecessary or fabricating. The §4.0.3 allowlist
  (`types.rs:1081`) must not grow.
- **#195 — re-parked explicitly.** #193/#194 are discharged, but the two
  real prerequisites (per-builtin-method escape contracts through
  `closure_param_escapes`; the MIR empty-container-literal allocation-site
  gap at `lowering/helpers.rs:112`) and the ADR-010 region-plan pipeline
  are un-chartered. Not dispatchable until chartered.
- **#221 — the four benchmark additions are AUTHORIZED** by the owner under
  §Benchmark Integrity: cast-free closure dispatch, a length-bounded array
  loop, a map pipeline, an `.iter()` chain. They measure the interpreter
  until the flip lands; they exist the day nativity returns.
- **#105–#177 — `ready-for-agent` stripped** from the 63 open old-program
  issues (relabelled `adr009-parked`): stale-by-priority since the
  2026-07-29 ADR-020 flip; the label must not dispatch a lane into
  superseded work. Content is not invalidated — the #116–#132 Book-gate
  cluster partly overlaps machinery that shipped since (CHECK 18–20) and
  needs a re-read before any future dispatch.
- **Float-div IEEE (#225) — veto window CLOSED, recorded as accepted.**
  Shipped (`7c99fd03`/`596c90de`, book patch landed); any future change is
  a new design decision, not a pending veto.
- **#189 — stays parked on #227**; front 1 is the territory the
  zero-capture flip experiment measures, so it un-parks with the flip.

---

## Enactment (approved 2026-08-02)

Records: this doc; new tickets (`never` type; enum-payload encoding design);
ruling comments on #252/#235/#237/#260/#256/#236/#195/#221/#225; the
63-issue label sweep. Lanes in dependency order: #260 first (flip
pre-baseline waits on it); #252, #237, #235-stage-1 in parallel; the flip
lane last, briefed only after the zero-capture experiment handback is
verified and independently re-derived on a fresh build.
