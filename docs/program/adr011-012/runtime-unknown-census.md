# Runtime-`Unknown` census (#233)

**Charter.** OWNER RULING 2026-07-29 on #233: *"if Unknown is a runtime thing, it
should not exist — there should not be any runtime inference."* This document is
the workspace-wide inventory that ruling asked for, and the audit anchor future
work re-derives against.

**Scope.** Every enum variant (or equivalent field/default) under `crates/`,
`bin/`, `tools/`, `extensions/` that encodes an ABSENCE — "we do not know",
"not stamped yet", "could not resolve" — as opposed to a positive fact.
Definition sites are exhaustive; consumer lists are representative where a
symbol has many call sites.

**Classification.**

| Class | Meaning | Verdict |
|-------|---------|---------|
| **(i) RUNTIME-CONSUMED** | a runtime execution path (VM executor, JIT-emitted code / FFI / deopt, runtime dispatch, or deserialization of a shipped artifact) branches on the absence | architectural failure — needs a deletion plan |
| **(ii) COMPILE-TIME-CONSERVATIVE** | a compile-time analysis state whose absence means "not proven; be conservative", consumed only by compiler / MIR / type-inference / verifier / LSP | acceptable; consumers listed so the boundary is explicit |
| **(iii) BUILD-PHASE-ONLY** | a placeholder that never escapes compilation | acceptable; noted where a builder-side `Option` would be the better type |
| **(x) NOT AN ABSENCE** | name pattern-matched the sweep but the variant is a positive fact | excluded, listed so the sweep is reproducible |

**Method.** `rg` over `--type rust` for absence-shaped variant names
(`Unknown|Unspecified|Untyped|Unresolved|Undetermined|Indeterminate|NotProven|Dynamic|Opaque|Missing|Absent|Uninitialized|Unset|Undefined|Any`),
plus `#[serde(default)]` fields whose default is an absence, plus `#[default]`
attributes and `impl Default` on such enums, plus `::unknown()`-style
constructors. Each definition was then read together with its construction and
consumer sites to classify. Line numbers are as of the #233 commit on
`rep-unknown`.

---

## Class (i) — RUNTIME-CONSUMED

### (i-1) `FrameReturnWrapper::Unknown` — `crates/shape-vm/src/type_tracking.rs:172`

The ticket's proven instance. **Runtime inference DELETED by #233; the variant
itself survives as a documented, single-sited residual.**

*Before #233:* `#[default] Unknown` on a `Default`-deriving enum, a
`#[serde(default)]` on `FrameDescriptor::return_wrapper`, and two runtime
methods that RE-DERIVED the wrapper from the ABI carrier kind —
`effective_return_wrapper()` (mapped an unstamped `return_kind =
Ptr(HeapKind::Option/Result)` back to a wrapper) and `abi_return_kind()`
(normalized the same overload the other way). Consumers:
`propagate_none_early_return` (`executor/exceptions/mod.rs`) and
`bytecode_function_returns_option` (`execution.rs`), both RUNTIME.

*After #233:* both normalizations deleted; both consumers ask positive
questions (`== Some(Result)`, `== Option`); the serde default deleted; the
`#[default]` attribute, the `Default` derive, and `impl Default for
FrameDescriptor` deleted; both constructors take the wrapper by value. Nothing
infers a wrapper at runtime any more.

*Residual.* `FrameReturnWrapper::Unknown` is stamped at exactly one site —
`FrameReturnMetadata::unclassified_residual()` in `compiler/helpers.rs`, used by
the descriptor builder when no classification source resolved the function's
return type. Deleting the variant is blocked on four PRODUCER classes whose
static return type never reaches that builder. Measured by making the site a
hard compile error and running `cargo test -p shape-vm --lib` (338 tests failed;
counts are error occurrences):

| # | Producer class | Hits | Root |
|---|----------------|------|------|
| 1 | Generic trait-impl methods (`Into::int::number::into`, prelude) | 130 | `desugar_impl_method` (`compiler/statements.rs`) backfills the trait's declared return type but does NOT substitute the impl's trait ARGUMENTS, so the annotation arrives as the trait's type parameter (`Target`), which no resolver reduces |
| 2 | Closures (`__closure_N`) | 72 | `compile_closure`'s `proto_def` (`compiler/expressions/closures.rs`) is built `return_type: None`; the inferred closure return type is never threaded to the builder |
| 3 | Compiler-generated fns (`\u{1}hygienic:…`, `__w27_implicit_*`, `__w24_method_*`) | ~50 | synthesized headers carry no return annotation |
| 4 | Unannotated trait methods (`Conn::drop`, `X::greet`, `Pair::merge`) | ~25 | resolve through neither the annotation nor the registered-concrete-type source |

Each is a WIRING gap, not a genuine unknowable: the static type exists in the
source (trait signature + impl arguments, inferred closure type, generator
intent) and simply does not reach `FrameDescriptor` construction. Note class 1
is why the head spelling cannot be read as `Plain` — a `Target` bound to
`Option<T>` in some impl would be misclassified. **Deletion plan:** close the
four producer classes (#227 REP-FN return-metadata rework is the natural home),
then delete the variant and make the builder site a `ProofGap` surface-and-stop.

### (i-2) `FieldType::Any` — `crates/shape-runtime/src/type_schema/field_types.rs:56`

*Doc:* "Any/dynamic type (uses HashMap access)". A TypedObject SCHEMA field type
that selects a different runtime access path from every typed sibling.

*Constructed:* shipped builtin schemas — `builtin_schemas.rs:516`, `:868`,
`:885`, `:899`, `:917`, `:918` (`array_field("bounds"|"annotations"|"args"|…,
FieldType::Any)`); inline-object schema synthesis at `type_tracking.rs:1220-1226`
(every field of an `__inline_obj_N` schema is stamped `Any`).

*Consumers:* `crates/shape-jit/src/mir_compiler/mod.rs:1328`, `:1365`, `:1376`
(field-layout classification, JIT compile path feeding emitted code);
`executor/objects/array_basic.rs:356` (`predeclared_any_schema` columns, VM
runtime). `builtin_schemas.rs:388`/`:623`/`:713`/`:794` document a
"post-inference whitelist" that exists precisely to permit specific `Any`
fields — i.e. the codebase already tracks this as debt with an allow-list.

*Verdict:* class (i). A schema field whose type is "unknown, use the dynamic
path" is the dynamic-fallback shape at the schema tier. Not in #233's scope;
needs its own ticket sized against the whitelist.

### (i-3) `FutureSnapshotStatus::Unknown` — `crates/shape-vm/src/executor/task_scheduler.rs:160`

*Doc:* "No scheduler entry exists for this id."

*Constructed:* `task_scheduler.rs:283` (`Some(TaskStatus::Pending) =>
Unknown` — note a PENDING task also reports `Unknown`, so the variant conflates
"no entry" with "entry exists but not started"), `:286` (`None => Unknown`).
*Consumer:* `task_scheduler.rs:810`.

*Verdict:* class (i), narrow. Runtime snapshot-facing status. AMBIGUITY STATED:
the `Pending => Unknown` arm means this is not purely "no entry", and I did not
trace whether any resume path branches on the distinction; the single consumer
at `:810` is within the same module. Needs a snapshot/resume owner to rule.

### (i-4) Absence DEFAULTS on JIT paths (not enum variants — flagged because the sweep's charter is absence STATES)

- `crates/shape-jit/src/osr_compiler.rs` — a live local's `NativeKind` missing
  from the frame slots falls back to `NativeKind::Int64`
  (`unwrap_or(NativeKind::Int64)`), documented as "legacy I64-NaN-box ABI
  width". Runtime OSR entry.
- `crates/shape-jit/src/mir_compiler/v2_call_abi.rs` — `let legacy_default =
  NativeKind::Int64;` pads missing param kinds AND supplies the return type when
  `return_kind` is `None`, in `resolve_function_signature`.

Both are "kind not known → assume Int64" on a code-generation path. They are the
same defect class as (i-1) one tier down, and each is a silent
reinterpretation of bits rather than a refusal. Own ticket.

---

## Class (ii) — COMPILE-TIME-CONSERVATIVE (acceptable)

| Definition | Meaning | Consumers (all compile-time) |
|---|---|---|
| `LocalTypeInfo::Unknown` — `crates/shape-vm/src/mir/types.rs:923` | "Unknown type (will be resolved during analysis)" | MIR Copy/Clone inference; named as acceptable in the #233 charter |
| `ReturnOwnershipMode::Unknown` — `crates/shape-vm/src/mir/analysis.rs:403` | "Could not infer — fall back to current Arc behavior"; `meet()` degrades any mismatch to `Unknown` | MIR ownership analysis. A lattice bottom, and the conservative direction is the SAFE one (Arc) |
| `EscapeFact::NotProven(NotProvenReason)` ×2 — `crates/shape-vm/src/mir/escape.rs:153`, `:193` | carries a REASON — the good shape for this class | MIR escape/borrow solver |
| `IcState::Uninitialized` / `TypeFeedback::Uninitialized` — `crates/shape-vm/src/feedback.rs:16`, `:28` | inline-cache state machine start state | Feedback is WRITTEN at runtime and READ by the JIT tier-up decision. It is a profiling observation, not a type claim: a wrong guess is guarded and deoptimizes. Listed here deliberately — the boundary is "speculation behind a guard", not "inference used as truth" |
| `StorageType::Dynamic` — `crates/shape-runtime/src/type_system/storage.rs:87` | "Dynamically typed value (escape hatch)"; produced for `TypeVar`/`Generic` (`:152`) and `Never`/`Void` (`:167`) | `native_kind_from_storage_type` (`type_tracking.rs:108`) maps it to `None` — i.e. it already dead-ends at the kind projection and cannot reach a typed opcode. Display at `:239`. Acceptable, but the name is an attractor; a rename to `NotProjectable` would be honest |
| `TraitResolution::Unknown` — `crates/shape-vm/src/compiler/comptime_builtins/semantic_freeze.rs:357` | impl trait-name resolution failed | comptime freeze, compile-time |
| `ResolutionOutcome::Unresolved` — `crates/shape-semantic-db/src/facts.rs:230` | "No declaration is in scope under that name" — arguably a positive fact | SemanticDb queries, compile-time |
| `NamedExportResolution::Missing` — `crates/shape-runtime/src/module_loader/loading.rs:333` | export lookup miss | `resolve_named_export`, compile-time |
| `TokenKind::Unknown` — `crates/shape-ast/src/error/parse_error/tokens.rs:45` | unrecognized token in a parse error | diagnostics |
| `DocTag::Unknown(String)` — `crates/shape-ast/src/ast/docs.rs:19` | unrecognized doc tag, carries its name | doc tooling |
| `TypeAnnotation::Undefined` — `crates/shape-ast/src/ast/types.rs:102` | surface `undefined` type spelling | AST/parser |
| `IntegrityIssue::Missing { path }` — `tools/xtask/src/perf_suite/integrity.rs:36` | perf-suite file missing | xtask tooling |

---

## Class (iii) — BUILD-PHASE-ONLY (acceptable; typing note)

| Definition | Note |
|---|---|
| `ConstantValue::Opaque(ConcreteType, [u8; 8])` — `crates/shape-vm/src/compiler/comptime_concrete.rs:113` | Self-documented debt: *"Bridge variant for producer paths not yet migrated to typed variants (notably extension-function returns) … deliberately the same size as a `ValueWord` so existing NaN-box round-trips remain feasible. New code should NOT introduce `Opaque` uses."* Consumers are all in-module (`:138` type projection, `:178` `is_typed()` predicate, `:280` test), so it does not escape compilation. **It is nonetheless a ValueWord-shaped carrier by its own admission** and belongs on the strict-typing deletion list even though it is not runtime-consumed |
| `FrameDescriptor::return_kind: Option<NativeKind>` (`#[serde(default)]`) — `crates/shape-vm/src/type_tracking.rs` | Deliberately overloaded "no return value" / "kind not stamped", documented by #224; consumers surface-and-stop at the JIT call site. The `Option` IS the builder-side shape this class wants. Left as-is by #233 (it is #224/#227 territory), recorded here so the overload is not forgotten |
| `BytecodeProgram.function_return_concrete_types[f] == ConcreteType::Void` — `crates/shape-vm/src/bytecode/core_types.rs:448` | `Void` per entry means "no annotation OR annotation didn't reduce" — an absence encoded in a positive variant, `#[serde(skip)]` so it never ships. Same family as (i-1) class 1; fixing the trait-arg substitution fixes both |
| `NominalDescriptor::Opaque { owner }` — `crates/shape-vm/src/compiler/comptime_builtins/type_reflection/payloads.rs:134` | Positive: "semantically non-decomposable nominal". Listed only because it pattern-matched |

---

## Class (x) — NOT AN ABSENCE (swept, excluded)

`Payload::Absent` (`crates/shape-value/src/encoding.rs:256`) — "No payload. The
kind alone carries the signal" — a positive encoding fact (#225 territory).
`ErrorModel::Dynamic` (`crates/shape-abi-v1/src/lib.rs:716`) — a language's
declared error model. `JoinKind::Any` (`crates/shape-ast/src/ast/expr_helpers.rs:174`)
— `join any` semantics. `NominalShape::Opaque`
(`crates/shape-runtime/src/comptime_reflection.rs:336`) and `FrozenType…Undefined`
(`:526`) — sealed catalog entries. `ForeignInvokeMode::Any`
(`crates/shape-jit/src/ffi/control/mod.rs:1058`) — "either invoke mode is
allowed". `typed_module_exports::ConcreteType::Any`
(`crates/shape-runtime/src/typed_module_exports.rs:346`) — the declared `any`
return of `msgpack.decode`; a surface-type spelling, not an inference state,
though it is the LSP-facing edge of the same `FieldType::Any` problem in (i-2).
`TaskStatus` (`task_scheduler.rs:147`) has no absence variant. Test-fixture
`Unknown` enum variants in `tools/shape-test/tests/pattern_matching/advanced.rs:139`,
`:162` are Shape-source test data, not compiler states.

---

## Summary

- **Class (i): 4 rows.** (i-1) is #233's target — its runtime inference is
  deleted; the residual variant is blocked on four compiler wiring gaps.
  (i-2) `FieldType::Any`, (i-3) `FutureSnapshotStatus::Unknown` and (i-4) the two
  `NativeKind::Int64` JIT defaults need their own tickets.
- **Class (ii): 12 rows.** All consumed only by compiler/MIR/tooling.
  `StorageType::Dynamic` and the feedback `Uninitialized` states are the two
  worth re-reading whenever the boundary is questioned.
- **Class (iii): 4 rows.** `ConstantValue::Opaque` is self-declared ValueWord-shaped
  debt and should join the strict-typing deletion list.
- **Class (x): 9 swept-and-excluded.**

No silent bucket: every symbol the sweep matched appears in exactly one class,
and the two rows where I could not fully resolve the semantics — (i-3)'s
`Pending => Unknown` conflation — carry the ambiguity inline.
