# Vertical Deep-Dive 03: Bytecode Compiler & MIR

Auditor 03 of 19 — ultra-deep-dive audit, 2026-07-11.
Territory: `crates/shape-vm/src/compiler/` (all), `crates/shape-vm/src/mir/` (borrow solver,
storage planning, lowering, repair), `crates/shape-vm/src/type_tracking.rs`
(`prove_native_kind`, `ProofGap`, `BindingStorageClass`), `crates/shape-vm/src/bytecode/`
(opcode definitions, verifier, content-addressed blobs) and their emit sites.
Working tree audited as-is (dirty, post-`ce332ca2`). All runtime transcripts produced with the
prebuilt working-tree binary `target/debug/shape` (extension-load warnings elided).

## 0. Executive summary

**Overall health verdict: STRONG CORE, OVERSTATED ENFORCEMENT STORY, ONE LIVE
WRONG-RESULTS HOLE.** The bytecode compiler is a large (~107 KLOC compiler +
~22 KLOC MIR + ~9 KLOC bytecode/type-tracking), feature-rich, strict-typing
front end that in empirical testing enforces every headline type-system rule at
the shipped binary level: `int`≠`number`, no truthiness, bitwise int-only,
string→int rejected, empty-array element proof required, and the historical
`(2 as number)/(8 as number) == 0` bit-reinterpret hole is closed. The MIR
layer is genuinely sophisticated — a Datafrog-based NLL borrow solver with
non-lexical loan liveness (verified empirically), escape analysis driving an
RC-on-escape storage planner, and a verified borrow-error *repair engine*.
RAII drop ordering and ADR-006 §2.7.30 escape-deferred Drop both behave as
documented.

Against that strong base, three problems dominate. First, a **live silent
wrong-results path**: `for i in 0.9..3.9` compiles without error and iterates
`0,1,2` — the range-loop prologue emits `OpCode::NumberToInt` truncation
(`compiler/loops.rs:74,85`), the exact opcode family CLAUDE.md's type-system
rules say must never be emitted to fix a type mismatch, and a direct
contradiction of the ratified "number→int requires explicit `as`" conversion
rule. Second, the **mechanical-enforcement story is largely aspirational**:
CLAUDE.md claims "every typed-opcode emission site must call
`prove_native_kind()`... the Rust type system enforces this", but the gate has
exactly **1 call site** in the whole workspace outside its own module
(`helpers_binding.rs:585`, inside the `prove_exact_scalar_return_kind` funnel;
the two `patterns/destructure.rs` grep hits are comment mentions, not calls)
against ~1,102 `self.emit(` sites and 390 distinct
opcodes referenced; arithmetic emission is proof-guarded by a *different*,
parallel mechanism (`numeric_type_of` → `typed_opcode_for`), and the deprecated
sparse tracker `last_emitted_native_kind` (pinned at baseline count 8 in
`docs/check-no-dynamic-baseline.txt`) is still live. Third, an emitter-tier
**latent lossy-coercion acceptance**: `plan_coercion` at
`compiler/expressions/numeric_ops.rs:243-257` will emit `IntToNumber` for
`Int + Number` and even `IntWidth(U64) + Number` (lossy) mixes; today the
upstream constraint solver rejects those programs first (verified), so
soundness rests entirely on a *different layer* than the one that owns the
rule.

### Top-10 findings

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 1 | **P0** | Float range endpoints silently truncate: `for i in 0.9..3.9` runs and iterates `0,1,2` — silent wrong results + live `NumberToInt` emission forbidden by CLAUDE.md §Type System Rules | `compiler/loops.rs:69-90`; transcript §9.1 |
| 2 | **P1** | `prove_native_kind` "mechanical enforcement" claim is >99% aspirational: 1 call site vs ~1,102 emit sites; typed arithmetic/comparison emission proof-guarded by a parallel mechanism it was supposed to replace | workspace-wide grep §6.3; `type_tracking.rs:1297`, `helpers_binding.rs:585` (sole caller) |
| 3 | **P1** | `plan_coercion` accepts lossy `Int + Number` and `u64 + number` at the emitter tier, emitting runtime `IntToNumber`; guarded only by the upstream analyzer (defense-in-depth inversion) | `numeric_ops.rs:236-306`; transcripts §9.2 |
| 4 | **P1** | Monomorphized generic functions never get a proven `FrameDescriptor.return_kind`, so ANY program calling a generic function whole-program-deopts from JIT to interpreter | transcript §9.4 (`identity` → "no compile-time-proven FrameDescriptor.return_kind") |
| 5 | **P1** | Tail-expression `&local` in a `-> &int` function yields bogus "Undefined variable: 'local'" while `return &local` compiles and runs — same program, different path, nonsense diagnostic | transcripts §9.5 |
| 6 | **P1** | Returning a reference parameter (`fn first(a: &int, ...) -> &int { a }`, also with explicit `return a`) fails inference: "int is not compatible with &int" — reference-typed returns of params are unusable despite `ReturnReferenceSummary` machinery built for them | transcripts §9.6; `mir/analysis.rs` |
| 7 | **P2** | Storage planning behavior depends on env var `SHAPE_V2_VAR_SHAREDCOW` (process-global semantic toggle, self-described "temporary bisect safety net") | `mir/storage_planning.rs:36-64` |
| 8 | **P2** | Opcode surface: 418 opcodes with redundant encodings for the same semantics (`AddI32` direct family vs `AddTyped`+Width operand family; `ConvertTo*` vs `Convert` trait dispatch vs `CastWidth`) | `bytecode/opcode_defs.rs`; §5.2 |
| 9 | **P2** | Sentinel test `no_dynamic.rs` covers exactly ONE forbidden pattern (Bool-default fabrication) while CLAUDE.md claims it "asserts forbidden symbols are absent" (plural); `verify-phase-5` justfile recipe still says the file hasn't landed | `executor/tests/no_dynamic.rs:61`; `justfile:196-197` |
| 10 | **P2** | `compile_statement` is a 1,736-line function; `compile_expr_method_call` 1,501; `compile_expr_function_call` 1,118; `compile_function_body` 891 — god-functions in god-files (statements.rs 9,663 LOC; helpers.rs 9,058 LOC), plus a 178-field `BytecodeCompiler` struct | §3.3 |

**Feature-completeness score: 82/100.** Everything in the language surface
compiles through this pipeline and my empirical battery (strict typing, borrow
checking, NLL, RAII, generics/monomorphization, casts, closures, break-with-
value) passed except the reference-return-of-param gap and the float-range
hole; the deduction is for those, for 19 ignored comptime-emit tests, and for
the return-kind stamping gap that silently degrades the JIT story.

**Code-quality score: 62/100.** Exceptional documentation density, disciplined
ADR cross-referencing, real tests (1,246 in-territory test fns), and zero
`unsafe` in MIR — but 9.7K-line files, 1.7K-line functions, a 178-field
compiler struct, three parallel numeric-kind mapping tables, comment-level
duplication of history in place of design docs, and an enforcement narrative
that has drifted from the code.

**Biggest risk.** The strict-typing guarantee is now split across two layers
that do not share a proof artifact: the constraint solver
(`analyze_program_full`) rejects ill-typed programs, while the emitter keeps
permissive legacy paths (`plan_coercion`'s lossy arms, `NumberToInt` loop
prologue, `last_emitted_native_kind`) that assume the solver already said yes.
Any compilation entry point that skips or weakens full analysis — REPL
fragments, `eval_with_loaders` (documented as "bypasses standard analysis"),
comptime sub-compiles, future incremental compilation — inherits the emitter's
permissiveness with no gate behind it. The float-range hole (#1) is exactly
this class of bug already shipped: a specialized emission path (range-loop
prologue) that never consults the solver's verdict on `number`-typed endpoints
because it *made its own decision* to truncate. The fix pattern (route ALL
kind decisions through one sealed proof API) exists in the codebase but is
deployed at exactly one call site.

## 1. Architecture & code structure map

### 1.1 Module inventory and LOC (measured via `find … | xargs wc -l`, working tree 2026-07-11)

**`crates/shape-vm/src/compiler/` — 64 files, 106,566 LOC total.** Top files:

| File | LOC | Responsibility |
|------|-----|----------------|
| `statements.rs` | 9,663 | Statement + Item compilation dispatch (`compile_statement` is one 1,736-line fn) |
| `helpers.rs` | 9,058 | Grab-bag: builtin classification, kind walk-back (`last_emitted_native_kind`), typed return-value opcode selection, storage-hint capture |
| `expressions/function_calls.rs` | 9,052 | Call/method-call compilation (`compile_expr_method_call` 1,501 lines) |
| `functions.rs` | 6,579 | Function registration + body compile + MIR integration (`compile_function_body` 891 lines) |
| `monomorphization/type_resolution.rs` | 5,442 | Concrete-type resolution for generic instantiation |
| `expressions/closures.rs` | 4,856 | Closure literal compile, capture analysis, callsite param hints |
| `expressions/binary_ops.rs` | 4,659 | Binary op typed-opcode selection, operator-trait dispatch |
| `monomorphization/substitution.rs` | 4,170 | Type-param substitution into ASTs |
| `comptime.rs` | 3,599 | Compile-time evaluation (runs a VM inside the compiler) |
| `compiler_impl_reference_model.rs` | 3,435 | The `compile()` driver + reference-model inference |
| `functions_annotations.rs` | 3,310 | `@annotation` handling (before/after/comptime) |
| `expressions/collections.rs` | 3,083 | Array/object/map literals |
| `expressions/mod.rs` | 2,902 | `compile_expr` dispatch |
| `v2_typed_emission.rs` | 2,803 | v2 typed struct/array opcode emission |
| `loops.rs` | 2,436 | Loop specialization incl. the range-counter fast path (finding #1 lives here) |
| `monomorphization/cache.rs` | 1,754 | mono_key specialization cache (budget: 64 closure specializations/module, `cache.rs:40`) |
| `helpers_reference.rs` | 1,722 | Reference/borrow codegen helpers |
| `functions_foreign.rs` | 1,523 | Polyglot fn (python/typescript/extern C) compilation |
| `post_inference_verify.rs` | 1,214 | `FieldType::Any` boundary enforcement (E0900) |
| `comptime_builtins.rs` | 1,491 | `type_info`, `implements`, `warning`, `error`, `build_config` |
| `patterns/destructure.rs` | 1,439 | Destructuring — cites `prove_native_kind` in comments only (`:300` doc comment, `:1364` `//` comment), no call |
| `helpers_binding.rs` | 1,382 | Binding semantics + the SOLE `prove_native_kind` call site in the workspace (`:585`) |

Sub-directory totals: `expressions/` 33,663 LOC (19 files); `monomorphization/`
12,292 LOC (5 files); `patterns/` ~3,300 LOC (5 files); `comptime*` ~9,900 LOC
(6 files).

The long tail below the table (all counted inside the aggregates above) is
worth naming because several files carry their own ADR touchpoints:
`v2_typed_emission.rs`'s siblings `v2_array_emission.rs` (636 LOC — inference
deciding whether an array literal/annotated binding can use typed-array
opcodes) and `v2_map_emission.rs` (301 — `ConcreteType::HashMap(k,v)`
annotation-conversion helper shared by statement/reference-model/
type-resolution paths); `trait_object_emission.rs` (435 — compiler-side
`Arc<VTable>` construction per `(impl Trait for Type)` pair plus dyn-coerce
and dyn-method-call detection, ADR-006 §2.7.24 Q25.C); `mutation_writeback.rs`
(233 — `&mut self` COW mutation-writeback for method dispatch, §2.7.27);
`string_interpolation.rs` (723 — f-string compilation; the interpolation
*parser* deliberately lives in shape-ast so compiler/inference/LSP share it);
`mir_schema_threading.rs` (130 — threads schema ids into MIR
`StatementKind::ObjectStore`, §2.7.5 stamp-at-compile-time);
`literal_widen.rs` (86 — the 2026-06-01 lossless literal-adoption AST rewrite
already noted in §1.2's pipeline); `comptime_concrete.rs` (393 — the
post-ValueWord typed comptime constant carrier `ConstantValue`, each variant
carrying data + concrete type by construction) and `comptime_diagnostics.rs`
(260 — LSDS-routed comptime error presentation); and the two same-named
control-flow files — `control_flow.rs` (69 — if/try-catch statement
compilation) vs `expressions/control_flow.rs` (122 — break/continue/return
expression compilation) — a naming collision worth resolving.

**`crates/shape-vm/src/mir/` — 14 files, 21,726 LOC:**

| File | LOC | Responsibility |
|------|-----|----------------|
| `storage_planning.rs` | 3,854 | Per-function `StoragePlan`: `BindingStorageClass` assignment, escape status, inline-array hints |
| `solver.rs` | 3,796 | Datafrog NLL borrow solver (`loan_live_at` fixpoint, conflict/escape detection) |
| `lowering/mod.rs` | 3,753 | AST→MIR lowering driver |
| `lowering/expr.rs` | 3,064 | Expression lowering to MIR statements |
| `return_ownership.rs` | 1,413 | Phase 5.B/5.C return-ownership inference (skip `PromoteToOwned` at callsites) |
| `lowering/stmt.rs` | 1,251 | Statement lowering |
| `field_analysis.rs` | 1,124 | Flow-sensitive field definite-init + dead-field liveness |
| `types.rs` | 957 | MIR core types (`MirFunction`, `Place`, `LoanSinkKind`, `ProjectionStep`) |
| `analysis.rs` | 628 | `BorrowAnalysis` result type, interprocedural summaries |
| `lowering/helpers.rs` | 595 | Lowering utilities |
| `repair.rs` | 594 | Borrow-error repair engine (REORDER/SCOPE/CLONE/DOWNGRADE/EXTRACT, solver-verified) |
| `liveness.rs` | 365 | Loan liveness computation |
| `cfg.rs` | 303 | Control-flow graph over MIR points |
| `mod.rs` | 29 | Re-exports |

**`crates/shape-vm/src/bytecode/` + `type_tracking.rs` — 8,989 LOC:**
`opcode_defs.rs` 3,342 (418 opcodes via `define_opcodes!` macro with per-opcode
category/pops/pushes metadata); `type_tracking.rs` 2,021 (TypeTracker,
`FrameDescriptor`, `prove_native_kind`/`ProofGap`); `core_types.rs` 1,394
(Instruction/Operand/Constant/Function); `verifier.rs` 830 (trusted-opcode and
v2-typed-opcode invariant checks); `content_addressed.rs` 653 (FunctionBlob,
SHA-256 content hash); `closure_layout_fallback.rs` 404; `program_impl.rs` 345.

### 1.2 Data flow

```
AST (shape-ast, pest)
  │  desugar_program → widen_numeric_literals → rebind_named_args   (AST rewrites,
  │  compiler_impl_reference_model.rs:1985-2005 — literal adoption is an AST
  │  RE-TYPING, deliberately not a runtime coercion opcode)
  ▼
analyze_program_full (shape-runtime constraint solver; FailFast, Strict default —
  compiler_impl_initialization.rs:151: "the suppressing ReliableOnly variant was
  deleted (WF-0A)")
  │  + infer_reference_model_with_comptime_context → InferenceFacts
  │    (resolved_expr_types table consulted by infer_expr_type; mod.rs:2184)
  ▼
BytecodeCompiler::compile (compiler_impl_reference_model.rs:1982)
  │  pass 1: register_item_functions (statements.rs — signatures, "Pre-declare
  │          only — full registration happens in second pass", statements.rs:5907)
  │  pass 2: compile_item_with_context per item
  │     per function: compile_function_body (functions.rs:875)
  │        ├─ mir::lowering::lower_function_detailed_with_returns_and_variants
  │        ├─ mir::solver::solve (Datafrog NLL) + BorrowAnalysisOptions
  │        │    { allow_return_slot_local_escape_promotion } (§2.7.30 contract)
  │        ├─ mir::field_analysis::analyze_fields (dead fields, cond-init)
  │        ├─ storage_planning → StoragePlan → BindingStorageClass per slot
  │        └─ bytecode emission (typed opcodes; FunctionBlobBuilder snapshot
  │           strategy, mod.rs:247 — global→blob-local index remap)
  ▼
post_inference_verify (FieldType::Any boundary, E0900)
verifier::verify_trusted_opcodes / verify_v2_typed_opcodes (bytecode/verifier.rs)
  ▼
BytecodeProgram { instructions, constants, functions (+FrameDescriptor),
                  content-addressed FunctionBlobs (SHA-256 + permissions) }
```

Notable: the compiler runs the **borrow solver and storage planner per
function during pass 2**, i.e. MIR is not a separate pipeline stage but a
per-function analysis subroutine of bytecode compilation. The JIT consumes MIR
separately (shape-jit `mir_compiler/` — out of territory) which makes
`mir/lowering/` a shared front-end for two consumers.

### 1.3 Key types

- `BytecodeCompiler` (`compiler/mod.rs:650-1845`): **178 field declarations**
  (measured `awk` over the struct span). Holds program, type tracker, inference
  facts, monomorphization cache, comptime state, module/import tables, loop
  contexts, annotation state, borrow summaries, closure hints, etc. This is the
  god-object of the pipeline (see §3.3).
- `TypeTracker` (`type_tracking.rs:688`): schema registry + per-slot
  `VariableTypeInfo` + `BindingSemantics` with scope push/pop and
  snapshot/restore for nested function compiles (`snapshot_local_types`,
  `type_tracking.rs:1052` — with the scope-stack co-snapshot fix documented in
  its comment).
- `FrameDescriptor` (`type_tracking.rs:168`): `slots: Vec<NativeKind>`
  (post-proof, no Option wrap per ADR-006 §2.7.5.1), `return_kind:
  Option<NativeKind>`, `return_wrapper: FrameReturnWrapper` — serialized into
  `FunctionBlob` (wire format).
- `ProofGap` (`type_tracking.rs:1235`): sealed via private `ProofGapSeal(())` —
  only `prove_native_kind` / `proof_gap_unresolved_operand` can mint one.
- `BindingStorageClass` (`type_tracking.rs:359`): `Deferred | Direct |
  UniqueHeap | SharedCow | Reference | LocalMutablePtr` (the Phase D
  stack-slot-with-closure-ptr class).
- `BorrowFacts`/`BorrowAnalysis` (`mir/solver.rs:84`, `mir/analysis.rs`):
  Datafrog input relations (`loan_issued_at`, `cfg_edge`, `invalidates`,
  `use_of_loan`) + rich sink taxonomy (task-boundary, closure-capture,
  array/object/enum-store loans).
- `StoragePlan` (`mir/storage_planning.rs:74`): slot→class map +
  `inline_array_sizes` optimization hints (currently consumer-less, see §11).
- `FunctionBlobBuilder` (`compiler/mod.rs:247`): snapshot-of-global-pools
  strategy for content-addressed blob extraction.

### 1.4 Entry points

- `BytecodeCompiler::new()` / `::compile(program)` —
  `compiler_impl_reference_model.rs:1982` (consumes self; whole-program).
- Per-function: `compile_function_body` (`functions.rs:875` region) — where
  MIR lowering/solving/planning are invoked.
- Public contract field: `pub stdlib_function_names: HashSet<String>`
  (`compiler/mod.rs:1711`) — must be set by anyone calling
  `prepend_prelude_items()` (CLAUDE.md testing convention; see §2.9).
- Test harness: `crates/shape-vm/src/test_utils.rs` `eval_*` helpers.

## 2. Feature completeness

Legend: **WORKS-E2E** = verified with a run transcript against the working-tree
binary; **CODE-EXISTS** = implementation read but not (fully) exercised;
**PARTIAL** = works with verified gaps; **STUBBED/IGNORED** = present but
disabled.

### 2.1 Strict typing enforcement — WORKS-E2E

All six probes rejected/accepted correctly (full transcripts in §9.2):

- `let a = 1; let b = 2.0; print(a + b)` → compile error "int is not
  compatible with number" ✓
- `let n: int = s` (s: string) → compile error ✓ — the catastrophic
  `ReliableOnly` diagnostic-suppression bypass recorded in the 2026-05-29 audit
  memory is **fixed in the shipped compiler**: `TypeDiagnosticMode` is now only
  `{Strict, RecoverAll}` and the default is `Strict`
  (`compiler/mod.rs:555-558`, `compiler_impl_initialization.rs:151` — comment:
  "the suppressing ReliableOnly variant was deleted (WF-0A)").
- `1.5 & 2.0` → compile error (bitwise int-only gate, `numeric_ops.rs:19-47`) ✓
- `if x { }` with `x: int` → compile error (no truthiness) ✓
- `let a = []` unused/unannotated → excellent targeted error ("empty array `a`
  has an un-resolvable element type… add an annotation") ✓
- `(2 as number) / (8 as number)` → `0.25` ✓ — the historical
  divide-bit-patterns-as-i64 soundness hole is closed; the fix mechanism is
  documented at `expressions/type_ops.rs:780-798` (`is_builtin_primitive_numeric_cast`
  must win over user-`Into` routing) and `type_ops.rs:824-842`
  (`record_cast_result_kind` stamps the cast TARGET type).

### 2.2 Numeric width system — PARTIAL (one wrong-results hole, one latent hole)

- `let a: i32 = 100; let b: int = a` → `100` — lossless widening ✓
- `u8 + number` → `202.5` (lossless promotion via emitted `IntToNumber`;
  `numeric_ops.rs:251-257`) ✓ per the ratified lossless-only conversion rule.
- `u8 + int` → `1200` (widen to i64) ✓; `u32 + number` → `4294967295.5` ✓.
- `u64 + i8` → clean compile error "cannot mix `u64` and `i8` in arithmetic —
  use an explicit `as` cast" (`plan_coercion` `IncompatibleWidths`,
  `numeric_ops.rs:270-281`) ✓
- `u64 + number` → rejected by the constraint solver ("u64 is not compatible
  with number") — BUT `plan_coercion` itself would classify it `CoerceLeft`
  and emit a lossy `IntToNumber` if reached (`numeric_ops.rs:251-252` matches
  ALL `IntWidth(_)`, including `U64`). Latent hole, §9.2.
- **Float range endpoints: WRONG RESULTS** — `for i in 0.9..3.9` iterates
  `0,1,2` (silent `NumberToInt` truncation, `loops.rs:69-90`). §9.1. This is
  the only place in my battery where strict typing silently produces wrong
  behavior instead of an error.

### 2.3 Borrow checking (MIR solver) — WORKS-E2E with diagnostics gaps

- Double `&mut` → `[B0001] cannot mutably borrow this value because it is
  already borrowed` with origin/still-needed notes ✓
- Write-while-borrowed → `[B0002]` ✓
- **Non-lexical**: borrow used then owner reassigned afterwards → compiles and
  runs (`b4_ok_borrow`: prints `5` then `10`) ✓ — genuine NLL, not lexical
  scoping.
- Local-reference escape from an unannotated function → `[B0003] cannot return
  or store a reference that outlives its owner` ✓
- Escape WITH declared `-> &int` + explicit `return &local` → compiles, prints
  42 (ADR-006 §2.7.30 escape-RC-promotion via
  `BorrowAnalysisOptions::allow_return_slot_local_escape_promotion`,
  `mir/solver.rs:66-79`) ✓
- Gap 1: the same function with a **tail expression** `&local` instead of
  `return &local` fails with bogus "Undefined variable: 'local'" (§9.5).
- Gap 2: returning a reference **parameter** fails inference entirely (§9.6),
  making the interprocedural `ReturnReferenceSummary` machinery
  (`mir/analysis.rs`, threaded into `FunctionReturnReferenceSummary` at
  `compiler/mod.rs:223-240`) unreachable for its primary use case.

### 2.4 RAII / escape analysis — WORKS-E2E

- Scope drop in reverse declaration order (`dropping b` before `dropping a`) ✓
- Escaping value (`fn make() -> Res`) has Drop deferred to program end —
  printed after `still alive` ✓ (ADR-006 §2.7.30 observable ordering).
- Storage planning implements RC-on-escape: `Direct` default, `UniqueHeap` for
  mutably-captured escapes, `SharedCow` for `var` (`storage_planning.rs:1-23`).
  Note the `var`→SharedCow decision is currently unconditional under the
  `SHAPE_V2_VAR_SHAREDCOW` env flag default (see §11.2).

### 2.5 Generics & monomorphization — WORKS-E2E in VM; JIT-blocking metadata gap

- `fn identity<T>(x: T) -> T` called with int and string → `42`, `hello` ✓.
  Lazy per-type-tuple specialization with stable `mono_key` cache
  (`monomorphization/cache.rs:1-40`), closure-specialization budget 64/module
  (`cache.rs:40`).
- BUT: every call to a generic function makes the whole program deopt from the
  JIT — "direct call to `identity` resolved to function index 195 but has no
  compile-time-proven FrameDescriptor.return_kind" (§9.4). The compiler
  monomorphizes with full concrete types in hand yet does not stamp
  `FrameDescriptor.return_kind` on the specialization. Compiler-side metadata
  gap, not a JIT limitation.
- CLAUDE.md's "pre-existing failure cluster (a) generic-fn instantiation
  returning Null" did NOT reproduce for the basic cases I ran; the cluster is
  evidently narrower than the doc implies (stale doc risk, §8).

### 2.6 Pattern matching — PARTIAL (no compile-time exhaustiveness)

- Enum match with unit/tuple/struct payloads and field destructuring →
  correct values ✓ (transcript §9.7).
- **Non-exhaustive enum match compiles without error** and throws `Uncaught
  exception: Error: No match arm matched the value` at runtime (exit 1) when
  the missing arm is hit — despite the book's front-matter claim "match is an
  expression with exhaustiveness checking"
  (`shape-web/book/book-site/src/content/docs/fundamentals/control-flow.mdx:6`).
  For a `-> int` function this also means the static return type is satisfied
  by a runtime abort path. §9.3.

### 2.7 Casts & conversion — WORKS-E2E

- `x as int` / `as number` / etc. lower to `ConvertTo*` typed opcodes
  (`type_ops.rs:802-812`); `as Type?` lowers to `TryConvertTo*` or
  trait-dispatch `Convert` with dispatch metadata (`type_ops.rs:914-970`);
  user `Into`/`TryInto` impls are routed around the primitive-numeric pairs to
  preserve kind-stamping soundness (`type_ops.rs:795-798`). Union widening `as
  A | B` is compile-time-only metadata with membership proof
  (`type_ops.rs:973-999`).
- Note: `ConvertToNumber` is **VM-only** — the JIT preflights it as
  unsupported and whole-program-deopts (transcript §9.2, t2). Cast-heavy code
  silently runs interpreted.

### 2.8 Comptime — WORKS-E2E for blocks/exprs; ignored-test cluster on emit directives

- `comptime { print(...) }` executed at compile time; `let x = comptime { 6*7 }`
  → 42 ✓ (§9.7).
- 14 of the 19 in-territory `#[ignore]` tests are the comptime emit-directive
  family (`functions.rs:2913-3153`: set-return/set-param/replace-body/extend
  "still depends on deleted host argument conversion"; `comptime_target.rs`
  ×3 + `comptime.rs` ×1 "phase-2c comptime rebuild against typed-Arc HeapValue
  layout"). Given project memory says the comptime core landed in WF-1B,
  these ignore reasons deserve a re-audit — either the tests can be revived or
  the features are still dead (§7.3).

### 2.9 `stdlib_function_names` contract — CODE-EXISTS, contract is convention-only

`pub stdlib_function_names: HashSet<String>` (`compiler/mod.rs:1711`) is a bare
public field. Nothing in the type system forces a caller of
`prepend_prelude_items()` to set it — the CLAUDE.md testing convention is
enforced by review only. A builder-style API or making
`prepend_prelude_items()` set the field itself would close this trap; today it
persists as a known footgun.

### 2.10 Content-addressed compilation — CODE-EXISTS (verified structurally)

`FunctionBlobBuilder` (`compiler/mod.rs:247-490`) implements the
snapshot-and-remap strategy: record pool sizes at fn start, extract delta,
remap global constant/string indices to blob-local ordinals. Permissions and
dependency hashes feed the SHA-256 content hash (`bytecode/content_addressed.rs`).
Not exercised E2E here (distributed-execution auditor's territory).

### 2.11 Bytecode verification — PARTIAL by design

`verify_trusted_opcodes` checks only operand *shape* for the two surviving
trusted opcodes; the stale `MissingFrameDescriptor` rule was deliberately
removed (WS-10b, `verifier.rs:14-44`) because it fired 16 false positives per
run and "enforced nothing — `load_program` only `eprintln!`'d".
`verify_v2_typed_opcodes` checks FrameDescriptor presence + FieldOffset sanity
for v2 typed ops (`verifier.rs:221`). There is no stack-effect/type-flow
verification of general bytecode — the `stack_pops`/`stack_pushes` metadata in
`define_opcodes!` exists but no verifier consumes it for a full abstract
interpretation pass.

## 3. Code quality

### 3.1 Idiom & error handling

- Error handling is uniformly `Result<T, ShapeError>` with structured
  `SemanticError { message, location }`; user-facing errors carry source
  context and error codes (`B0001`-`B0014`, `E0900`,
  `E_TYPED_OPCODE_WITHOUT_PROOF`). Diagnostic *quality* is mostly excellent
  (the empty-array message is exemplary) with localized failures (§9.5's
  "Undefined variable", §9.8's constraint-solver span pointing at `let x = 5`
  instead of the offending `if x`).
- Documentation density is the highest I have seen in this codebase: nearly
  every non-trivial function carries a doc comment naming the wave/cluster
  that introduced it and the ADR section binding it. The flip side: comments
  routinely embed multi-paragraph project history (e.g.
  `type_tracking.rs:1069-1105`'s deprecation note citing audit §4.D.5 +
  supervisor-ratify dates), which ages badly (§8 lists concrete stale ones).
- Naming is consistent (`compile_expr_*`, `emit_*`, `prove_*`, `infer_*`),
  though the `helpers.rs` / `helpers_binding.rs` / `helpers_reference.rs`
  split is by size, not by concept.

### 3.2 Unsafe usage — 20 occurrences, concentrated and mostly justified

Census (`grep -rc "unsafe "`): compiler 19 (comptime_builtins.rs 4,
comptime_target.rs 9, comptime.rs 4, v2_typed_emission.rs 1 [doc-comment
mention only], expressions/temporal.rs 1), mir 0, bytecode 1
(`core_types.rs:848 pub unsafe fn from_raw(bits, kind)` — properly marked).
The comptime cluster derefs raw `TypedObjectStorage` pointers
(`comptime_builtins.rs:306: let storage = unsafe { &*(bits as *const
TypedObjectStorage) }`) because comptime evaluation runs a VM inside the
compiler and reads its v2-raw slot carriers. These are the standard v2-raw
access pattern; the risk is they replicate the allocator-pair-mismatch class
that bit the W5 residuals (test-fixture `Arc::new` vs `_new`-allocated
carriers — see CLAUDE.md v2-raw-heap-audit note). **Zero unsafe in the borrow
solver / storage planner** is a genuine strength.

### 3.3 Complexity hotspots (measured, `awk` fn-length scan)

| Function | LOC | File |
|----------|-----|------|
| `compile_statement` | 1,736 | statements.rs |
| `compile_expr_method_call` | 1,501 | expressions/function_calls.rs |
| `compile_expr_function_call` | 1,118 | expressions/function_calls.rs |
| `compile_function_body` | 891 | functions.rs |
| `compile_item_with_context` | 597 | statements.rs |
| `compile_function_inner` | 427 | functions.rs |
| `register_item_functions` | 361 | statements.rs |

Plus the 178-field `BytecodeCompiler` struct (`mod.rs:650-1845`, 1,195 lines
of field declarations + docs). These numbers mean every new AST variant or
call-shape lands inside kilo-line match arms; the CLAUDE.md "Exhaustive Match
Rule" (~8+ files per AST variant) is partly a *consequence* of this shape.
The `compile()` driver itself (`compiler_impl_reference_model.rs:1982-…`)
interleaves ~15 pre-passes inline (desugar, literal widening, named-arg
rebind, comptime extends, imported-analysis items, analyzer, reference model,
field hoisting…) rather than as a declared pipeline.

### 3.4 Dead code in-territory

- 46 `#[allow(dead_code)]` sites (grep census) — many are audit-trail metadata
  fields (`WhitelistEntry.section/reason`, `post_inference_verify.rs:44-50`,
  documented as intentional), but `ImportedSymbol` (`mod.rs:107`),
  `ResolutionScope` (`mod.rs:140`), and `FunctionBlobBuilder.const_start/
  string_start` (`mod.rs:253-257`) carry allows on genuinely unused data.
- `StoragePlan.inline_array_sizes`: computed every compile, "Today no consumer
  acts on the hint" (`storage_planning.rs:88-92`) — honest, but it is paid-for
  analysis with zero payoff since Phase D.
- `module_registry.rs` (executor-side but compiler-referenced): "retained for
  API compatibility but contains no active functionality" with a stale
  ValueWord-era doc comment (§8.3).
- The `Convert`/`TypeCheck` opcodes at 0x91/0x92 have exactly 2 compiler emit
  sites each (`type_ops.rs:967,1140`) — alive, but the "generic" naming is a
  trap for future forbidden-pattern grep audits (they are trait-dispatch cast
  machinery, not dynamic fallback).

## 4. Duplication & DRY violations

### 4.1 The numeric-kind mapping tables — 3 parallel maps in one file, ~8 across the crate seam

`binary_ops.rs` contains three hand-written partial maps into `NumericType`:

- `StorageHint → NumericType` (`binary_ops.rs:505-513`)
- `FieldKind → NumericType` (`binary_ops.rs:532-540`)
- `FieldType → NumericType` (`binary_ops.rs:565-574`)

plus, in the same universe: `native_kind_from_storage_type`
(`type_tracking.rs:90-110`), `VariableTypeInfo::infer_storage_hint` (name →
StorageHint, `type_tracking.rs:607-638`),
`storage_hint_for_runtime_numeric` (`type_tracking.rs:646-662`),
`inferred_type_to_numeric` (Type → NumericType, `numeric_ops.rs:161-182`), and
`literal_numeric_type` (AST literal → NumericType, `numeric_ops.rs:145-158`).
Each map hard-codes the same i8/u8/i16/u16/i32/u32/i64/u64/f64/decimal rows.
**Divergence is dangerous here**: adding a numeric width (or changing decimal's
carrier) requires touching ≥6 places in two crates with no compiler assistance
beyond non-exhaustive-match luck (several of these match on `&str` names, which
the compiler cannot check at all). The canonical projection
`native_kind_from_concrete_type` (shape-value) that `prove_native_kind` uses is
the *ninth* member of this family.

### 4.2 Dead duplicate schema-registration APIs — mint-path divergence

`TypeTracker` exposes three sibling registration functions
(`type_tracking.rs:1123-1184`): `register_inline_object_schema_typed` (routes
through `TypeSchemaRegistry::register_type_scoped`, the WF-3A content-intern
"single mint point", `registry.rs:487-493`), and
`register_named_object_schema` + `register_typed_object_schema` (both
construct `TypeSchema::new(...)` then `register(schema)`). Grep shows the
latter two have **zero callers outside type_tracking.rs** — dead duplicates.
`TypeSchema::new` interns via the *ambient* `current_registry()`
(`schema.rs:97-105`) rather than the tracker's own registry; the comment says
registration "re-derives the handle from THAT registry's intern table", but
keeping two mint idioms alive around the exact root cause of the recurring
schema-id-collision family (project memory: `next_id` counter collisions,
routed to WF-3A) invites regression. Delete the dead pair.

### 4.3 Range-endpoint truncation prologue — copy-pasted, bug included

The `NumberToInt`-if-float endpoint prologue appears verbatim twice:
`loops.rs:69-90` (range counter loop) and `loops.rs:1684-1704` (spread range
inside array literal, `__spread_counter_{idx}`). Both carry the identical
"U4-4: endpoint numeric kind derived from the one resolved Type" comment and
the identical silent-truncation bug (finding #1). A fix applied to one and not
the other would make `for i in 0.9..3.1` and `[...0.9..3.1]` disagree.

### 4.4 Proof-then-swallow twins

`helpers_binding.rs:580-673` defines `prove_exact_scalar_return_kind` (hard
error on ProofGap) and then two wrappers `top_level_metadata_return_kind_from_proof`
/ `exact_top_level_metadata_return_kind_for_expr` that call the same proof and
**swallow the error into `None`** (documented as metadata-only). The pattern is
sound for its stated purpose, but it means the phrase "calls prove_native_kind"
does not imply "enforces proof" — a reviewer must check which of the twin
idioms a site uses. One `#[must_use]` proof-token type would collapse this.

### 4.5 Hand-rolled AST walkers

`module_local_calls.rs` (412) + `module_local_expr_calls.rs` (364) +
`module_local_expr_helpers.rs` (392) + `module_local_expr_scopes.rs` (276) —
~1,450 LOC of manual recursive `match`-on-`Expr`/`Statement` purely to qualify
module-local call names, duplicating the traversal skeleton that
`shape_runtime::visitor` already provides (and which
`closures.rs::collect_closure_callsite_param_hints` separately re-implements
again for its own pre-pass). Every new `Expr` variant must be added to each
walker by hand; a missed arm silently skips qualification (no exhaustiveness
error, since several arms end in `_ => {}`).

## 5. Split-brain analysis

### 5.1 Bytecode compiler vs MIR lowering — two AST lowerings of one semantics

The compiler lowers AST→bytecode (`compiler/expressions/*`, ~34 KLOC) and
*separately* lowers AST→MIR (`mir/lowering/*`, ~8.7 KLOC) for borrow
checking/storage planning — and shape-jit consumes MIR for codegen. Same
language, two lowering brains. Divergence is observable **today** at the cast
seam: MIR models casts as a kind-restamping instruction that "VM lowers … to
the kind-restamping `OpCode::ConvertTo*` family" (`mir/types.rs:485-501`),
but the JIT preflights `ConvertToNumber` as `vm_only_opcodes` and deopts the
whole program (transcript §9.2). Every construct that only one lowering
supports becomes a silent whole-program deopt (best case) or a semantics fork
(worst case). The JIT-fallback banners I collected in one afternoon
(`ConvertToNumber`, generic-call return_kind, stdlib ModuleFn dispatch,
Int32 print) are the drift ledger in real time.

### 5.2 Redundant opcode encodings for identical semantics

- `AddI32/SubI32/MulI32/DivI32/ModI32/EqI32/NeqI32/GtI32/...` (direct, no
  operand) vs `AddTyped/SubTyped/MulTyped/DivTyped/ModTyped/CmpTyped` +
  `Operand::Width` (I8..F64) — `typed_opcode_for` special-cases I32 into the
  direct family and routes i8/i16/u8/u16/u32/u64 into the width family
  (`numeric_ops.rs:365-404`). Two executor paths, two JIT lowering paths, one
  semantics. Additionally `CmpTyped` returns an ordering {-1,0,1} while every
  other comparison returns bool, so width comparisons secretly reuse the
  `GtInt` bool family instead (comment at `numeric_ops.rs:392-395`) — meaning
  `CmpTyped` is a third encoding with (as far as the compiler is concerned)
  no emitter for ordered comparisons except decimal/special paths.
- Cast machinery exists in three tiers: `ConvertTo*`/`TryConvertTo*` (primitive,
  0x76-0x81), `Convert` + TypeAnnotation dispatch metadata (trait-based user
  impls, 0x92), and `CastWidth` (bit-truncation, 0xF7). Defensible layering,
  but each tier has separate VM and JIT support matrices.

### 5.3 Doc-vs-code drift (enforcement narrative)

- CLAUDE.md §Mechanical enforcement: "Every typed-opcode emission site must
  call `prove_native_kind` … The Rust type system enforces this." Reality:
  exactly 1 call site (`helpers_binding.rs:585`; the two additional grep hits
  in `patterns/destructure.rs` at `:300` and `:1364` are a doc comment and a
  `//` comment — the destructuring code *cites* the gate as the fix without
  calling it) vs ~1,102 `self.emit(` sites; typed arithmetic is guarded by
  `numeric_type_of`/`plan_coercion` instead, and the transitional tracker
  `last_emitted_native_kind` (baseline-pinned at 8, described in
  `docs/check-no-dynamic-baseline.txt` as "sparse kind tracker (replace with
  prove_native_kind)") is still the *claimed-kind source* feeding the one real
  proof site. The seal prevents fabricating a `ProofGap`, but nothing prevents
  *not calling the gate at all* — the failure mode that actually occurs.
- CLAUDE.md: "Sentinel test `no_dynamic.rs` asserts forbidden symbols are
  absent" — the test asserts exactly ONE pattern (Bool-default slot
  fabrication, `no_dynamic.rs:61-76`); the multi-symbol check lives only in
  the shell script.
- `justfile:196-197`: `verify-phase-5` still prints "TODO: invoke sentinel
  test when … no_dynamic.rs lands" — the file landed (99 lines, Jul 4).
- `type_tracking.rs:20`: "Generic `GetProp`/`SetProp` are reserved for non-dot
  operations" — module-header doc that predates several refactors; worth a
  refresh sweep together with §8.3's stale comments.

### 5.4 Forbidden-symbol baseline vs CLAUDE.md forbidden list

`docs/check-no-dynamic-baseline.txt` pins 30+ patterns, but **neither
`IntToNumber` nor `NumberToInt` is a row**, despite CLAUDE.md §Type System
Rules naming exactly those opcodes as never-emit. The two guard layers
(narrative rule vs mechanical baseline) have diverged: the mechanically
enforced set is a subset of the documented set, and the live emissions
(`numeric_ops.rs:294,299`, `binary_ops.rs:1087,1091`, `loops.rs:74,85,1688,1699`)
sit precisely in the unenforced gap.

### 5.5 Opcode stack-effect metadata vs executors

`define_opcodes!` records `pops`/`pushes` per opcode (`opcode_defs.rs:36-72`)
but no verifier or executor consults them (variable-arity opcodes use 0/0 by
convention). This is a classic parallel table that will silently rot; either
wire it into a bytecode stack-effect checker or delete it.

### 5.6 `FrameDescriptor.return_kind` producer vs JIT consumer

The JIT hard-requires a proven `return_kind` for direct-call lowering (W36,
per the runtime fallback message in §9.4); the compiler stamps it via the
walk-back machinery for ordinary functions but not for monomorphized generic
specializations. Producer and consumer of the same field live in different
crates with no shared test asserting "every function the compiler registers
carries a stamped return_kind or is documented VM-only".

## 6. ADR & spec conformance

Marker density in territory: 214 `// ADR-005`/`// ADR-006` marker comments
across compiler/mir/bytecode/type_tracking (grep census).

### 6.1 ADR-005 (single discriminator, typed slots)

| Rule | Verdict | Evidence |
|------|---------|----------|
| §1 `HeapValue` is the canonical heap discriminator; no parallel 1:1 sum types | **CONFORMS (in territory)** | No new `HeapKind`-projecting enums found in compiler/mir; compile-tier enums (`NumericType`, `VariableKind`) discriminate *static types*, not heap values |
| §2 single `TypedFieldValue::String` exception | N/A here (shape-runtime) | — |
| §4 uniform slot ABI, no `Box<HeapValue>` in new code | **CONFORMS** | Only 3 comment-mentions of `Box<HeapValue>` in territory, all describing avoidance (`expressions/temporal.rs:106`, `helpers.rs:211`, `statements.rs:7019`) |

### 6.2 ADR-006 value & memory model

| Rule | Verdict | Evidence |
|------|---------|----------|
| §2.7.5.1 `FrameDescriptor.slots: Vec<NativeKind>` post-proof, no Option wrap, no Unknown | **CONFORMS structurally** | `type_tracking.rs:168-187`; `Option<NativeKind>` used only for the single-slot `return_kind` field, explicitly justified against §2.7.8/Q10 in the doc comment (`type_tracking.rs:162-166`). Caveat: "every slot is proven by FunctionBlob construction" is asserted in comments, and the monomorphized-generic `return_kind=None` gap (§9.4) shows *stamping completeness* is not enforced |
| §2.7.5 stamp-at-compile-time; no fabrication from raw bits | **CONFORMS in compiler** | `native_kind_from_storage_type` returns `None` for complex types (`type_tracking.rs:101-108`); `infer_storage_hint` returns `None` for unknown names (`:607-638`); no `NativeKind::Unknown` exists |
| §2.7.7 no Bool-default fabrication | **CONFORMS + mechanically guarded** | baseline rows `unwrap_or((0, NativeKind::Bool))` and `unwrap_or(NativeKind::Bool)` pinned at 0; `no_dynamic.rs` test enforces the first at build time; `bash scripts/check-no-dynamic.sh` exits 0 on the working tree (run transcript §9.9) |
| §2.7 KindedSlot must not leak into the typed VM↔JIT slot ABI | **CONFORMS at compile tier** | 181 `KindedSlot` mentions in compiler/ are comptime-evaluation carriers (comptime runs a VM in-process — a GENERIC_CARRIER site per §2.7), not slot-ABI leakage |
| §2.7.30 escape-RC-promote for returned refs; Drop deferral | **CONFORMS behaviorally** | Transcripts §9.5/§9.10: `return &local` with `-> &int` contract runs; escaping `impl Drop` value drops at program end; `BorrowAnalysisOptions::allow_return_slot_local_escape_promotion` gates on declared contract (`mir/solver.rs:66-79`). Gap: tail-expression form broken (finding #5) |
| §Forbidden: generic opcodes (Add/Sub/Lt without kind) | **CONFORMS** | `opcode_defs.rs:87-107` documents the deletions (AddDynamic 0x10-0x16, GtDynamic 0x20-0x25, dynamic bitwise 0x17-0x1C) with byte-gap preservation; no bare `Add` variant exists (macro-extraction census, 418 opcodes, all arithmetic kind-suffixed or width-parameterised). `And`/`Or`/`Not` (0x30-0x32) are bool-only logical ops gated by the no-truthiness rule (verified: `if x` on int rejects) |
| §Forbidden: `Convert<X>To<Y>` papering over kind-tracker gaps | **CONFORMS with a caveat** | No `ConvertBoolToString`-class opcode exists (baseline row pinned 0). `ConvertTo*` (0x76-0x7B) are *explicit-cast* lowerings reachable from `as` syntax (`type_ops.rs:802-812`) — sanctioned per CLAUDE.md `__into_*` note. Caveat: `IntToNumber`/`NumberToInt` (0xF3/0xF4) ARE kind-mismatch-fixing conversion opcodes with live emit sites (findings #1/#3) |
| §Forbidden: `SlotKind::Dynamic/Unknown`, `exec_*_dynamic_fallback`, ValueWord | **CONFORMS** | All greps return only comments describing deletions by name (permitted per CLAUDE.md scope rule); baseline rows at 0 or comment-only counts; script exit 0 |
| Defection-attractor rename regex | **CONFORMS** | No `(decode\|tag\|kind\|dispatch\|value.call\|...)( bridge\|probe\|helper\|hop\|translator\|adapter\|shim)` matches in territory source |
| §2.7.24 typed-carrier monomorphization (Q25) | **CODE-EXISTS, partially verified** | Monomorphization produces concrete-typed specializations E2E (§2.5); per-specialization return-kind stamping incomplete (finding #4) |

### 6.3 CLAUDE.md §Type System Rules — the two failures

1. "**Never emit `IntToNumber`/`NumberToInt` coercion opcodes to 'fix' type
   mismatches**" — VIOLATED live at `loops.rs:74,85,1688,1699` (float range
   endpoints, wrong results demonstrated §9.1) and latently at
   `numeric_ops.rs:289-306` + `binary_ops.rs:1087,1091` (mixed-numeric binop
   coercion; live-reachable today only for the lossless IntWidth+Number pairs
   because the constraint solver rejects the lossy ones first — §9.2). If the
   lossless-implicit-conversion ruling (2026-06-01) is the operative spec,
   then CLAUDE.md's blanket sentence is stale and should be rewritten to
   "lossless-only, and only via the solver-approved pairs"; either way doc and
   code currently disagree and the range-loop case violates BOTH versions.
2. "Typed opcodes require compile-time proof via `prove_native_kind()`" —
   see §5.3: the predicate is real (SB-8b tests prove it rejects int/number
   unification, width narrowing, UInt64-for-Ptr, Bool-for-Option:
   `type_tracking.rs:1364-1522`), but it guards a single call site
   (`helpers_binding.rs:585`). The de facto proof
   mechanism for arithmetic is the solver + `numeric_type_of`, which is
   *plausible* but is not the sealed API the docs describe.

### 6.4 Runtime v2 spec (`docs/runtime-v2-spec.md`)

The compiler-facing requirements — typed opcodes only when proven, per-slot
NativeKind stamping, `FrameDescriptor` as the JIT ABI contract — are
implemented as described above. Not re-verified against the executor (auditor
02's territory); the one seam checked (JIT preflight consuming compiler
`vm_only_opcodes` classification) behaves as designed, though the *size* of
the VM-only set is a completeness problem (§5.1).

## 7. Test coverage in-territory

### 7.1 Counts (grep census, working tree)

| Area | `#[test]` fns | Notes |
|------|---------------|-------|
| `compiler/` | 937 | in-file `#[cfg(test)]` modules per CLAUDE.md convention |
| `mir/` | 227 | lowering 74, storage_planning 50, return_ownership 34, solver 29, repair 4 |
| `bytecode/` | 51 | verifier, content_addressed, core_types |
| `type_tracking.rs` | 31 | incl. the 8-test SB-8b `prove_native_kind` adversarial suite |
| **Total** | **1,246** | 19 `#[ignore]`, 49 test modules |

Top test files: `functions.rs` 113, `monomorphization/type_resolution.rs` 80,
`v2_typed_emission.rs` 76, `mir/lowering/mod.rs` 74, `binary_ops.rs` 72,
`statements.rs` 62.

### 7.2 Assertion quality — genuinely good where it counts

- The `prove_native_kind` suite (`type_tracking.rs:1364-1522`) is adversarial:
  it asserts REJECTION of the historically-observed lies (SB-10 UInt64-for-
  HashMap, SB-12 Bool-for-Option-None, old JIT `Ptr(HeapKind::Option/Result)`
  carriers, int/number unification both directions, width narrowing both
  directions). This is exactly the right shape for a soundness gate — the
  problem is deployment breadth (§5.3), not test quality.
- MIR solver tests build hand-constructed `MirFunction`s and assert on solver
  verdicts (e.g. `test_conflicting_shared_and_exclusive_error`,
  `solver.rs:2339-…`) — unit-level, deterministic, no string matching.
- `FrameDescriptor` wire-compat is tested including deserialization of the
  legacy overloaded shape (`frame_descriptor_deserializes_old_shape_with_unknown_wrapper`,
  `type_tracking.rs:1490-1502`).
- `numeric_ops` coercion tests assert the width lattice precisely (u64+i8
  incompatible, u32+i8 promotes to i64, `numeric_ops.rs:482-531`) — note these
  tests *bless* the CoercionPlan design that CLAUDE.md's blanket rule forbids,
  i.e. the test suite and the doc encode different specs (§6.3).

### 7.3 Ignored tests — 19, three clusters

1. **Comptime emit directives (10)** — `functions.rs:2913-3153`: set-return /
   set-param / replace-body / extend "still depends on deleted host argument
   conversion". These are feature-level holes, not flaky tests.
2. **Phase-2c comptime carrier rebuilds (4+1 placeholder)** —
   `comptime_target.rs:705,721,886`, `comptime.rs:1785` ("comptime rebuild
   against typed-Arc HeapValue layout — see ADR-006 §2.4"). Given comptime
   blocks/exprs work E2E (§2.8) and project memory says the comptime
   correctness core landed (WF-1B), at least the *reasons* are stale even if
   the tests still fail — nobody has re-triaged them.
3. **Diagnostic-only (1)** — `v2_typed_emission.rs:2782` (opcode-trace
   helper, legitimately ignored).

The remaining ~3 are scattered singletons. None of the 19 is a
"pass-but-ignored" case as far as the stated reasons go, but cluster 2's
reasons cite a phase that other documents call complete — re-run them.

### 7.4 Gaps

- **No end-to-end emission-discipline test**: nothing compiles a corpus and
  asserts "no `IntToNumber` in emitted bytecode except behind solver-approved
  lossless pairs". The float-range bug would have been caught by a 5-line test
  asserting float endpoints are a compile error.
- **No exhaustiveness tests for the numeric mapping-table family** (§4.1):
  adding a width silently misses tables.
- **repair.rs has only 4 tests** for a 5-strategy engine with solver
  re-verification — the least-tested sophisticated component in the territory.
- **No test that every registered function gets `return_kind` stamped**
  (finding #4 class).
- Match exhaustiveness has no negative test (because the feature doesn't
  exist — §9.3), yet the book claims it.

## 8. Book/docs vs reality

### 8.1 Book claims checked (shape-web/book/book-site/src/content/docs)

| Claim | Reality | Verdict |
|-------|---------|---------|
| "match is an expression with exhaustiveness checking" (`fundamentals/control-flow.mdx:6`) | Non-exhaustive enum match compiles; runtime "No match arm matched" exception (§9.3) | **FALSE** |
| "`for i in 0..n` (exclusive), `0..=n` (inclusive)" (ibid.) | Correct for int endpoints; book silent on float endpoints, which the compiler accepts and truncates (§9.1) | TRUE but hides a trap |
| "`let mut` is single-owner with zero overhead; `var` is shared with reference counting" (`fundamentals/variables.mdx:12`) | Matches `BindingStorageClass` design (`Direct` vs `SharedCow`; `storage_planning.rs:1-23`); `var`→SharedCow currently unconditional under the default env flag rather than aliasing-conditional (`storage_planning.rs:36-64`) | TRUE (semantics), implementation coarser than doc implies |
| "Mutations use copy-on-write" for `var` (`variables.mdx:94`) | SharedCow class exists and is planned per-slot; not stress-verified here | PLAUSIBLE (untested by me) |
| References & Borrowing chapter exists; B-codes | B0001/B0002/B0003 all fire with correct semantics; NLL verified (§9.5-9.6 transcripts) | TRUE with the two P1 gaps (tail-ref, ref-param return) |

### 8.2 CLAUDE.md claims checked

| Claim | Verdict |
|-------|---------|
| "prove_native_kind … Rust type system enforces this" | **OVERSTATED** — 1 call site; parallel proof mechanisms elsewhere (§5.3) |
| "Sentinel test asserts forbidden symbols are absent" | **OVERSTATED** — 1 pattern of ~30 (§5.3) |
| "`just check-no-dynamic` greps forbidden symbols … build fails on hit" | TRUE — script + baseline verified, exit 0 on working tree (§9.9) |
| "Pre-existing shape-test failure cluster (a) generic-fn instantiation returning Null" | Did not reproduce on basic generics (§2.5) — cluster description likely stale/narrower |
| "Two-pass compilation (register then compile)" | TRUE — `register_item_functions` pre-pass then `compile_item_with_context` (statements.rs:5907 "Pre-declare only — full registration happens in second pass") |
| "Tier thresholds, IC state machine" table rows | Out of territory; not checked |
| `BindingStorageClass` at "type_tracking.rs:286" | Drifted: now at `type_tracking.rs:359` (file grew) — minor index rot, same file |

### 8.3 Stale comments in code (should be swept)

- `test_utils.rs:37`: "See `dispatch.rs::synthesize_value_word_from_raw` for
  the canonical raw-bits → tagged-ValueWord encoder" — function deleted;
  comment survives as instruction to use deleted code.
- `executor/module_registry.rs:4`: "All module function dispatch now uses the
  ValueWord-based `module_fn_table`" — ValueWord is deleted; the file is a
  no-op husk with a misleading header.
- `justfile:196-197` verify-phase-5 TODO (§5.3).
- `codebase-index` / CLAUDE.md line numbers for `BindingStorageClass` (286 →
  359).

## 9. Bugs & correctness risks found

### 9.1 [P0] Float range endpoints silently truncate — wrong results, forbidden opcode emission

`compiler/loops.rs:69-90` (range counter loop) and `loops.rs:1684-1704`
(spread ranges): if `numeric_type_of(endpoint)` is `Number`, the compiler
emits `OpCode::NumberToInt` and proceeds with an int counter. No error, no
warning, book documents ranges as int-only.

```
$ cat r2_float_var_range.shape
let a = 0.9
let b = 3.9
for i in a..b {
    print(i)
}
$ shape run r2_float_var_range.shape
0
1
2
```

`0.9..3.9` becomes `0..3`. Under any reasonable semantics this is wrong
(float iteration, or a compile error per the strict-typing rules and the
ratified "number→int requires explicit `as`" conversion rule). It violates
CLAUDE.md's never-emit rule for `NumberToInt` in both letter and spirit —
this is a kind-mismatch being "fixed" at runtime. Literal form `0.9..3.1`
reproduces identically. Severity P0 per the rubric (silent wrong results).
Fix: make `Number`-typed endpoints a compile error (mirroring the
empty-array diagnostic quality), delete both truncation arms.

### 9.2 [P1] Emitter-tier lossy coercion acceptance (latent) + IntToNumber emission (live, lossless subset)

`plan_coercion` (`numeric_ops.rs:236-286`) classifies `Int + Number` →
`CoerceLeft(Number)` and `IntWidth(_) + Number` → coerce — where `IntWidth(_)`
includes `U64` (lossy above 2^53). `apply_coercion` (`:289-306`) emits
`Swap; IntToNumber; Swap` / `IntToNumber`. Verified reachability today:

```
let a: u8 = 200      # u8 + number: COMPILES, prints 202.5 (lossless — OK per
let b = 2.5          # 2026-06-01 ruling, but emitted via runtime IntToNumber)
print(a + b)

let a = 1            # int + number: REJECTED upstream by constraint solver
let b = 2.0          # "int is not compatible with number"
print(a + b)

let a: u64 = 18446744073709551615   # u64 + number: REJECTED upstream
let b = 0.5                          # "u64 is not compatible with number"
```

So the lossy arms are dead **only because a different layer rejects first**.
The emitter itself encodes pre-strict-typing semantics. Any analysis-bypassing
entry path (CLAUDE.md documents `eval_with_loaders()` as exactly that;
comptime sub-compiles and REPL fragments are candidates) executes
`plan_coercion` without the guard. Fix: delete the `Int+Number` arms and gate
`IntWidth+Number` on a lossless-width predicate inside `plan_coercion` itself,
so the emitter enforces the ruling locally.

### 9.3 [P1] No compile-time match exhaustiveness

```
enum E { A, B }
fn f(e: E) -> int { match e { E::A => 1 } }
print(f(E::B))
→ Uncaught exception: Error: No match arm matched the value   (exit 1)
```

Compiles clean; `-> int` is satisfied by an abort path. Book claims
exhaustiveness checking (§8.1). For a language whose whole identity is
"if it can't be proven, it's a compile error", this is the largest
front-end-semantics gap I found.

### 9.4 [P1] Monomorphized generic functions lack proven `FrameDescriptor.return_kind` → whole-program JIT deopt

```
fn identity<T>(x: T) -> T { x }
print(identity(42))
→ [jit-fallback] … direct call to `identity` resolved to function index 195
  but has no compile-time-proven FrameDescriptor.return_kind. W36 …; running
  under interpreter
42
```

The specialization was compiled from a fully concrete signature
(`identity::i64`), so the compiler had the proof in hand and dropped it. Every
program touching ANY generic function silently loses the JIT tier. The
surface-and-stop behavior is correct per ADR-006 (no fabrication) — the bug is
producer-side stamping completeness in `monomorphization/` + the return-kind
walk-back not running for specializations.

### 9.5 [P1] Tail-expression `&local` under a `-> &int` contract: bogus "Undefined variable"

```
fn make_ref() -> &int {
    let local = 42
    &local            # tail expr  → error: Undefined variable: 'local' (:3:6)
}
# but with `return &local` instead → compiles, prints 42 (§2.3, §2.4 promotion)
```

Same program modulo the `return` keyword: one path applies §2.7.30 escape
promotion, the other fails name resolution with a nonsense diagnostic. Points
at a tail-expression lowering path that processes the reference before the
binding is registered (or in a different scope snapshot). Severity P1: the
error message actively misleads (variable IS defined two lines up), and the
working form is the non-idiomatic one.

### 9.6 [P1] Reference-typed parameters cannot be returned

```
fn first(a: &int, b: &int) -> &int { return a }
→ error: Could not solve type constraints: int is not compatible with &int (:1:4)
```

Both tail-expr and explicit-return forms fail. Reference params auto-deref to
`int` inside the body, and nothing re-references them for the `-> &int`
return. The interprocedural machinery built for this
(`ReturnReferenceSummary` → `FunctionReturnReferenceSummary`,
`compiler/mod.rs:223-240`; callee summaries in `mir/solver.rs::CalleeSummaries`)
is unreachable for its motivating case. Reference-returning APIs are limited
to owned-local promotion (§9.5's working form).

### 9.7 [verified-good] Feature battery transcripts

```
enum/struct-payload match → 12.56636 / 12.0            (f1_match)
comptime block+expr       → "at compile time" / 42      (f3_comptime)
pipe + null-coalesce      → 42 / 99                     (f4_pipe_coalesce)
Result + `?`              → Ok(11) / Err("neg")         (f5_result_try)
closure capture mutation  → 2                           (t9, VM; JIT deopts)
break with value          → 42                          (t10, JIT-compiled OK)
NLL borrow-release        → 5 / 10                      (b4_ok_borrow)
RAII order + escape defer → b,a then program-end        (raii2/raii3)
```

### 9.8 [P2] Constraint-solver diagnostics anchor to the binding, not the use

`if x { }` with `x = 5` reports the error span at `let x = 5` (line 1), not at
the `if` condition (§2.1 transcript t5). Correct rejection, misleading anchor.
Same class as the B0003 span in §9.5 pointing all three notes at `:3:12`.

### 9.9 [verified-good] Forbidden-pattern guard run

```
$ bash scripts/check-no-dynamic.sh; echo EXIT=$?
EXIT=0
```

Silence = every row exactly at baseline. All non-zero-baseline symbols
(`synthesize_value_word_from_raw` 12, `exec_arithmetic_dynamic_fallback` 5,
`capture_as_value` 12, nan-box family 17…) were grep-verified to be comments
describing deletions — with ONE live-code exception: `last_emitted_native_kind`
(`helpers.rs:2442`, consumed at `helpers_binding.rs:606` and `helpers.rs:4052`),
which is the pinned tech-debt tracker the baseline itself says to replace with
`prove_native_kind`.

### 9.10 [P2] `BorrowAnalysisOptions` default is permissive-false but flag-flipped per call site

`allow_return_slot_local_escape_promotion` defaults `false`
(`mir/solver.rs:74-79`) and is set from the *declared* return annotation at
the `compile_function_body` call site (`functions.rs:885` region). Correct
today, but the promotion decision is annotation-string-driven at a distance
from the solver; a second caller forgetting the option regresses to
B0003-rejecting valid programs (or worse, passing `true` unconditionally).
Candidate for deriving inside the solver from the MIR function's own
return-type field.

## 10. What is done well

1. **The strict-typing flip is real and shipped.** The default compile path is
   `TypeDiagnosticMode::Strict` with `FailFast` analysis; the suppressing
   `ReliableOnly` mode — recorded in the 2026-05-29 audit as the top
   release-blocker ("string→int compiles+runs+reinterprets heap pointer") —
   was actually deleted, not renamed (`compiler/mod.rs:555-558`,
   `compiler_impl_initialization.rs:151`), and every probe I threw at the
   binary was rejected with a good message. This is the single most important
   claim in the vertical and it holds.

2. **Datafrog NLL borrow solver** (`mir/solver.rs`). Choosing a Datalog
   fixpoint engine over hand-rolled dataflow gives monotone-convergence
   correctness by construction, a declarative rule set that reads like the
   Polonius literature, and a single `BorrowAnalysis` artifact consumed by
   compiler, LSP, and diagnostics ("no consumer re-derives results",
   solver.rs:7-8). Empirically it exhibits true non-lexical behavior (§9.7)
   — most young languages ship lexical borrows and NLL is genuinely hard.

3. **The borrow-repair engine** (`mir/repair.rs`): candidate fixes
   (REORDER/SCOPE/CLONE/DOWNGRADE/EXTRACT) are each *re-verified by re-running
   the solver on the modified MIR* before being suggested. Verified-repair
   suggestions are a state-of-the-art diagnostics feature.

4. **AST rewrites instead of runtime coercion for literal adoption.**
   `widen_numeric_literals` re-types adopting int literals *before* both
   bytecode and MIR lowering consume the AST
   (`compiler_impl_reference_model.rs:1987-1995`), closing the
   `takes_num(5) → 2.5e-323` bit-reinterpret hole at the only layer where the
   fix is zero-cost and shared by both consumers. Same pattern for named-arg
   rebinding (single rewrite, all downstream passes see positional form).

5. **`ProofGap` sealed-constructor design** (`type_tracking.rs:1234-1268`,
   `ProofGapSeal(())`): the API shape is exactly right — the projection is
   total, exact-equality only, with adversarial tests encoding each
   historically-observed lie (SB-10/SB-12/int≠number/width). The critique in
   this report is deployment breadth, not design.

6. **Deleted opcode byte-gaps preserved with tombstone comments**
   (`opcode_defs.rs:87-107`): dynamic arithmetic/comparison/bitwise opcode
   bytes are documented and never reused — binary compatibility with old
   bytecode is protected and the deletion history is auditable in-place.

7. **The `define_opcodes!` macro** gives every opcode a category and stack
   effect in one declaration and generates exhaustive `category()` — adding an
   opcode without metadata is a compile error.

8. **Escape-status-driven storage planning** implements the ADR-006
   "refcount on escape, not mutability" rule as an explicit, testable lattice
   (`Direct`/`UniqueHeap`/`SharedCow`/`LocalMutablePtr` with per-slot
   `EscapeStatus`), with 50 tests on the planner alone. The `LocalMutablePtr`
   class (stack slot + typed `*mut T` into a non-escaping closure env,
   `type_tracking.rs:349-368`) is a sophisticated optimization most VMs skip.

9. **Monotonic forbidden-symbol baseline** (`check-no-dynamic-baseline.txt` +
   script): per-symbol counts may only decrease; once 0, forever 0. This is a
   well-designed ratchet — the gaps are in its coverage (§5.4), not mechanism.

10. **`FunctionBlobBuilder` snapshot strategy** for content-addressed blobs:
    compiling into global pools then delta-extracting with index remapping
    (`mod.rs:242-490`) avoids a second compilation pass while keeping blobs
    self-contained for hashing.

11. **Comment discipline on invariants.** ADR markers are grep-able (214 in
    territory), and safety-critical decisions (e.g. why builtin primitive
    casts must not route through user `Into` impls — `type_ops.rs:780-798`)
    are written down at the exact code point where a future refactor would
    otherwise reintroduce the bug.

## 11. What is done poorly / tech debt

1. **God-struct + god-functions** (§3.3). A 178-field compiler struct where
   any helper can read/write any phase's state means no phase boundary is
   enforceable; the 1,736-line `compile_statement` and two >1,100-line call
   compilers are where new bugs will concentrate. The `compile()` driver
   inlines ~15 sequential pre-passes with ad-hoc local state instead of a
   pipeline abstraction.

2. **Split proof mechanisms** (§5.3, §6.3). Four coexisting "what kind is this
   expression" oracles: `resolved_expr_types` (solver table),
   `numeric_type_of`/`literal_numeric_type`, `last_expr_type_info` +
   `last_emitted_native_kind` (emission walk-back), and
   `prove_native_kind` (sealed gate, 1 site). The first three can disagree;
   the fourth is the one documented as authoritative and is the least used.
   The U4-4/U4-5b comments show migration *direction* is right (deleted
   `last_expr_numeric_type` register, deleted stringly return-type map), but
   the endpoint — one oracle — is far away.

3. **Emitter permissiveness surviving behind the solver** (§9.2): the
   forbidden lossy arms of `plan_coercion`, `IntToNumber` emission helpers,
   and the loop-prologue truncation are all still in the binary, guarded (or
   not, §9.1) by a layer that doesn't know it is the guard.

4. **Env-var semantic toggle in storage planning.**
   `SHAPE_V2_VAR_SHAREDCOW` (`storage_planning.rs:36-64`) changes
   `BindingStorageClass` assignment process-globally, is self-described as a
   "temporary bisect safety net; … can be deleted", and is still present.
   Compiled-program semantics (aliasing behavior of `var`) should never fork
   on an environment variable; and a OnceLock-cached env read makes it
   untestable within one process.

5. **Consumer-less analysis output**: `inline_array_sizes` hints computed
   every compile since Phase D with zero consumers
   (`storage_planning.rs:88-92`). Either wire the SROA/SmallVec consumer or
   stop paying for the analysis.

6. **The numeric mapping-table archipelago** (§4.1) and **dead schema-mint
   duplicates** (§4.2) — both classic drift generators sitting next to
   historically-bitten code (schema-id collision family).

7. **Comment/history bloat as documentation**: multi-paragraph wave/cluster
   histories inline at every touchpoint (readable, but several are already
   stale — §8.3 — and the sheer volume trains readers to skip comments,
   which defeats the invariant-comment discipline praised in §10.11).

8. **Diagnostics anchoring** (§9.8): solver errors point at bindings instead
   of uses; the B0003 family prints the same span for origin/still-needed/
   escape notes. The repair engine's quality makes the contrast stark.

9. **19 ignored tests whose stated reasons cite completed phases** (§7.3) —
   nobody owns re-triage; ignore-rot is how features quietly stay broken.

10. **`ConvertToNumber` (and friends) VM-only** — the compiler emits opcodes
    the JIT declares unsupported, so a single `as number` in `main` interprets
    the whole program (transcript §9.2 t2). The typed-opcode surface grew
    faster than the JIT's coverage, and nothing in CI measures the
    VM-only-opcode set size.

## 12. Prioritized recommendations

### P0 — correctness now

1. **Reject `Number`-typed range endpoints** at both loop sites
   (`loops.rs:69-90`, `:1684-1704`); delete the four `NumberToInt` emissions.
   Effort: small (compile error + 2 tests + book note). This closes the only
   demonstrated wrong-results hole in the vertical.
2. **Harden `plan_coercion` locally**: remove `Int+Number` arms; gate
   `IntWidth+Number` on lossless widths (u8/u16/u32/i8/i16/i32 → f64 only;
   reject U64). Effort: small. Add a corpus test asserting emitted bytecode
   contains no `IntToNumber` outside the whitelisted lossless pairs.
3. **Implement enum-match exhaustiveness checking** (or, minimally, retract
   the book claim and downgrade to a documented runtime error). The compiler
   already has variant tables (monomorphization resolves enum layouts);
   checking unit/tuple/struct variant coverage plus wildcard is a bounded
   feature. Effort: medium.

### P1 — soundness architecture & broken features

4. **Stamp `return_kind` on monomorphized specializations** in
   `monomorphization/` (the concrete return `ConcreteType` is available at
   specialization time — project it via `prove_native_kind`, which is exactly
   the intended use). Effort: small-medium; unlocks JIT for all
   generic-touching programs. Add the missing invariant test: every registered
   function has `return_kind` stamped or a documented VM-only reason.
5. **Fix tail-expression `&local` under `-> &T`** to route through the same
   escape-promotion path as `return &local` (§9.5), and make the failure
   message never claim an in-scope variable is undefined. Effort: medium.
6. **Make reference-param returns work or reject them with a real message**
   (§9.6). If auto-deref semantics forbid returning `&` params, say so in the
   diagnostic and the book; if not, wire `ReturnReferenceSummary` into
   inference. Effort: medium.
7. **Converge on one kind oracle**: migrate `last_emitted_native_kind`
   consumers to solver-fact + `prove_native_kind` (the baseline file already
   declares this intent), then ratchet its baseline 8→0. Make emission of
   kind-suffixed opcodes require a `ProvenKind` token type (newtype returned
   only by `prove_native_kind`) so the "type system enforces it" sentence in
   CLAUDE.md becomes true. Effort: large but incremental per opcode family.

### P2 — hygiene & drift control

8. Add `\bIntToNumber\b` / `\bNumberToInt\b` emission-site rows to
   `check-no-dynamic-baseline.txt` (start at current counts, ratchet down).
   Effort: trivial.
9. Delete dead `register_typed_object_schema`/`register_named_object_schema`
   (§4.2); delete or wire `inline_array_sizes`; remove `SHAPE_V2_VAR_SHAREDCOW`
   once ownership Phase 1 lands as its own comment demands. Effort: small.
10. Extend `no_dynamic.rs` to iterate the baseline file's rows (single source
    of truth) instead of hard-coding one pattern; fix the `verify-phase-5`
    TODO. Effort: small.
11. Stale-comment sweep: `test_utils.rs:37`, `module_registry.rs:1-6`,
    CLAUDE.md `type_tracking.rs:286` line ref, ignored-test reason re-triage
    (§7.3). Effort: small.
12. Deduplicate the numeric mapping tables behind one
    `WidthLattice`/projection module with an exhaustive-by-construction test
    (§4.1). Effort: medium.
13. Track the JIT `vm_only_opcodes` set size in CI; alert on growth (§11.10).
    Effort: small.
14. Split `compile_statement`/`compile_expr_method_call` along their existing
    match arms into per-construct functions; freeze `BytecodeCompiler` field
    growth (any new field needs a sub-struct). Effort: large, mechanical,
    high-payoff for the agent-driven workflow this repo uses.

---

### Appendix A — reproduction inventory

All scratch programs under
`/tmp/claude-1000/-home-dev-dev-shape-lang-shape/64326cfd-c702-4fc9-8d52-24f3e6c2ff09/scratchpad/verticals/bytecode-compiler/`:
`t1_mixed_arith` … `t10_break_value`, `b1_two_mut` … `b4_ok_borrow`,
`b2b`–`b2f`, `raii1`–`raii3`, `c1_u8_plus_number` … `c5_i64max_check`,
`r1_float_range`, `r2_float_var_range`, `f1_match` … `f5_result_try`,
`f2b_hit_missing_arm`. Binary: `target/debug/shape` (working tree,
post-ce332ca2, dirty). Guard run: `bash scripts/check-no-dynamic.sh` → exit 0.

### Appendix B — quantitative summary

| Metric | Value |
|--------|-------|
| Territory LOC | ~137,300 (compiler 106,566 + mir 21,726 + bytecode/type_tracking 8,989) |
| Opcodes defined | 418 (`define_opcodes!`, `opcode_defs.rs`) |
| Distinct opcodes referenced by compiler | 390 |
| `self.emit(` call sites | 1,102 |
| `prove_native_kind` call sites (excl. its own module) | 1 (`helpers_binding.rs:585`; the 2 further grep hits in `patterns/destructure.rs:300,:1364` are comments, not calls) |
| `#[test]` fns in territory | 1,246 (19 ignored) |
| `unsafe` occurrences | 20 (19 compiler [comptime-heavy], 1 bytecode, 0 mir) |
| `#[allow(dead_code)]` | 46 |
| ADR marker comments | 214 |
| `BytecodeCompiler` fields | 178 |
| Longest function | 1,736 LOC (`compile_statement`) |
| Empirical probes run | 31 programs; 6 findings, 8 verified-good clusters |

