# Vertical Deep-Dive Audit 02: Type System & Inference

**Auditor:** 02 of 19 (ultra-deep-dive, commissioned by project owner)
**Date:** 2026-07-11
**Territory:** `crates/shape-runtime/src/type_system/` (inference, environment, types, checking, unification), `crates/shape-runtime/src/type_schema/`, `crates/shape-types/` (documented empty skeleton), inference call sites in the bytecode compiler (`crates/shape-vm/src/compiler/`)
**State audited:** working tree (dirty), branch `main`, HEAD `ce332ca2`
**Binary used for empirical tests:** `/home/dev/dev/shape-lang/shape/target/debug/shape` (prebuilt from this working tree)

---

## 0. Executive summary

### Verdict

The type system's **core strictness spine is real and enforced**: the past-catastrophic
`TypeDiagnosticMode::ReliableOnly` suppression is gone (default is `Strict`, the suppressing
variant was deleted — `crates/shape-vm/src/compiler/compiler_impl_initialization.rs:151`),
`string -> int` is a compile error, `int`/`number` never unify, truthiness coercion is rejected in
`if`/`while`/`&&`/`!`, the lossless-implicit numeric lattice works exactly as ruled (literal
adoption yes, variable widening no, `i8 -> int` implicit, `u64 -> int` rejected, `number -> int`
requires `as`), HM let-generalization with a value restriction works, trait bounds and supertrait
obligations are enforced, and user-enum match exhaustiveness is checked. That is a lot of
load-bearing correctness, verified empirically in ~90 test programs (section 2/9 transcripts).

However, the audit found **one P0 unsoundness hole** (generic-method value arguments are only
*soft*-checked — `HashMap<string,int>.set("a","oops")` compiles, runs, and a later typed read
reinterprets the heap pointer as `int`, printing `106644639314833` — the exact pointer-reinterpretation
class the strict-typing flip was supposed to eliminate), and a **cluster of documented features
that are dead end-to-end**: flow-sensitive narrowing (both the documented `!= null` spelling,
which cannot parse, and the real `!= None` spelling, which the compiler's second type-checking
layer rejects), `instanceof` union narrowing, Option/Result match exhaustiveness, tuples, and
`bigint` (no literal, no cast, no constructor — the runtime's own overflow message recommends
`as bigint`, which does not compile). The root architectural risk is a **three-layer type-checking
split-brain**: the runtime `TypeInferenceEngine` (has narrowing), the bytecode compiler's own
emission-time type tracking (does not), and runtime kind guards (partial). Features implemented
in layer 1 die in layer 2.

### Top-10 findings

| # | Severity | Finding | Evidence |
|---|----------|---------|----------|
| 1 | **P0** | Generic-method value-args soft-checked only: `m.set("a","oops")` on `HashMap<string,int>` compiles; `get + ?? 0` then does pointer arithmetic on a string heap pointer (`v + 1` prints `106644639314833`) | t69/t70 transcripts §9.1; root cause `inference/bidirectional.rs:483` (`synth_with_hint` "soft ... doesn't force") + `inference/expressions.rs:~1555` ("Expected param types that are NOT a bare variable are LEFT ALONE") |
| 2 | **P1** | Flow-sensitive narrowing dead end-to-end: `null` is a reserved word with no grammar production (`shape.pest:1570`), and `x != None` narrowing (implemented at `inference/statements.rs:820-905`) is discarded by the compiler's own binary-op typing (`shape-vm/src/compiler/expressions/binary_ops.rs:240`) | t10/t19/t20/t23/t25–t29 §9.2 |
| 3 | **P1** | Match on `Option`/`Result` is never exhaustiveness-checked → runtime crash "No match arm matched the value"; book says "Exhaustiveness is statically checked" | t43/t49/t50 §9.3; `exhaustiveness.rs:72-81` only handles `SemanticType::Enum` |
| 4 | **P1** | `bigint` unusable: `123...890n` literal fails parse, `5 as bigint` rejected ("Cannot assert type 'int' as 'bigint'"), no stdlib constructor; runtime overflow error recommends the impossible `as bigint` | t61/t91/t92 §9.4 |
| 5 | **P1** | `instanceof` union narrowing (book operators.mdx:159) does not compile — same layer-2 split-brain as #2 | t86 §9.5 |
| 6 | **P1** | Non-bool closure returns accepted where signature requires `-> bool`: `arr.filter(\|x\| x + 1)` compiles and silently filter-keeps everything via runtime truthiness | t77/t83 §9.6 |
| 7 | **P1** | Tuples don't parse at all (`let t = (1, "one")` = parse error) though CLAUDE.md lists tuples and `TypeAnnotation::Tuple` machinery exists throughout the territory | t71–t73 §9.7 |
| 8 | **P1** | Bare unparameterized generic names accepted in unconstrained positions (`fn f(x: Option)` uncalled, `type T { m: HashMap }`) — user ruling was "not a valid type anywhere"; enforcement is a constraint-solving side effect, not resolution-layer validation | t07/t16 vs t13–t15 §9.8 |
| 9 | **P2** | Internal `Debug` representation leaked into user diagnostics: `Generic { base: Concrete(Reference(TypePath ...)), args: [Variable(TypeVar("T62"))] } is not compatible with Array` | t08/t13/t57/t86; `errors.rs:196`, `constraints.rs:~1225` |
| 10 | **P2** | `Array.concat` wrong-element-type only caught at runtime; `includes("x")` on `Array<int>` silently `false` — same soft-check family as #1 with lower blast radius | t82/t84 §9.9 |

### Scores

- **Feature completeness: 68/100.** The strict core (numeric lattice, no-truthiness, let-gen,
  traits, generics, struct literals, user-enum exhaustiveness, bidirectional closure inference)
  works end-to-end and is well-tested; but narrowing, tuples, bigint, `instanceof`, Option/Result
  exhaustiveness, and hard generic-method arg checking — all *claimed* features — are dead or
  half-wired.
- **Code quality: 72/100.** Modern, heavily-documented Rust with disciplined ADR-marker comments,
  0 ignored tests among 480 in-territory tests, only 5 `unsafe` blocks (all in one Arrow-reading
  file, though behind non-`unsafe` pub fns); dragged down by string-keyed `TypeVar`s, the
  `"\u{1}tyvar:"` stringly-typed annotation encoding, Debug-leaking diagnostics, dead
  type-evolution machinery, and 4,900-line files.

### Biggest risk

The biggest risk is not any single bug — it is that **the same type system is implemented one and
a half times** and the halves disagree. The runtime `type_system/` engine (this vertical) infers,
narrows, and solves constraints; the bytecode compiler in `shape-vm` then re-derives types
per-expression for opcode emission and rejects anything its simpler tracker can't prove. Every
feature that exists only in layer 1 (null-narrowing, instanceof-narrowing) is silently dead, and
every laxity that exists only in layer 1 (soft `Synth` hints on method args) becomes a soundness
hole because layer 2 trusts the receiver annotation it was given. Until either the layers share
one fact store (the `InferenceFacts` plumbing is a start) or layer 1's verdicts are binding,
"fix it in the type checker" changes will keep failing to reach users, and audit finding #1's
pattern (soft-accepted ill-typed value + typed read downstream = pointer reinterpretation) will
recur in every new generic container API.

---

## 1. Architecture & code structure map

### 1.1 Module inventory and LOC

Measured with `find ... -name '*.rs' | xargs wc -l` on the working tree (2026-07-11).
Territory total: **45,064 LOC** across `type_system/` + `type_schema/`.

| Module | LOC | Responsibility |
|--------|-----|----------------|
| `type_system/inference/inference_tests.rs` | 5,409 | End-to-end inference unit tests (144 `#[test]`s) |
| `type_system/inference/items.rs` | 4,903 | Top-level item inference: fn schemes, let-generalization (value restriction at :857-:978, :2327-:2358), impls, type params |
| `type_system/inference/expressions.rs` | 4,495 | Expression inference: literals, calls, closures, match, method-call arg checking (the P0 site, :1440-:1680) |
| `type_system/inference/mod.rs` | 4,286 | `TypeInferenceEngine` struct, `InferenceFacts` (per-binding fact export), program walk |
| `type_system/constraints.rs` | 2,614 | `ConstraintSolver`: 3-phase solve (eager unify -> fixed-point retry -> bound application), `NumericDomain` lossless lattice (:38-:71), `probe_equal` single equivalence relation (:249) |
| `type_system/environment/mod.rs` | 2,422 | `TypeEnvironment`: scoped schemes, builtin fn signatures (`define_builtin_functions` :1073), trait registry facade, enum registry, evolution facade |
| `type_system/inference/access.rs` | 2,329 | Property/index access, field resolution, auto-deref (ADR-006 §2.7.30 marker at :1490) |
| `type_system/checking/method_table.rs` | 2,150 | `MethodTable` + `GenericMethodSignature`: PHF-style compile-time method signatures for Vec/String/HashMap/Option/... (`HashMap` block :829-:900) |
| `type_system/inference/operators.rs` | 1,524 | Binary/unary operator typing rules (borrow-transparent reads, ADR-006 marker :730) |
| `type_schema/registry.rs` | 1,364 | `TypeSchemaRegistry`: per-runtime schema registry, content-addressed dedup (`intern_content`, WF-3A / ADR-006 §2.7.31, :23-:76) |
| `type_system/inference/bidirectional.rs` | 1,253 | `CheckMode::{Infer,Check,Synth}`; `synth_with_hint` soft probe (:483) |
| `type_system/environment/registry.rs` | 1,250 | Type aliases, traits, enum defs, record schemas, blanket impls |
| `type_schema/field_types.rs` | 1,057 | `FieldType` + `to_native_kind()` projections (ADR-006 §2.7.5/§2.7.7 markers :72-:122) |
| `type_system/checker.rs` | 984 | `TypeChecker` facade + `analyze_program*` entry points, `TypeAnalysisMode::{FailFast,RecoverAll}` |
| `type_system/inference/statements.rs` | 972 | Statement inference; flow-narrowing extraction (`extract_narrowings` :822, `try_null_narrowing` :876) |
| `type_schema/schema.rs` | 762 | `TypeSchema`, `SchemaContentId` (structural content hash) |
| `type_system/types/core.rs` | 628 | `Type`, `TypeVar`, `TypeScheme`, `TypeVarGen`, `canonicalize()` (U1, :297), `to_annotation()` (:362), tyvar-marker encoding (:73-:89) |
| `type_system/semantic.rs` | 586 | `SemanticType` — the user-facing type vocabulary |
| `type_system/exhaustiveness.rs` | 558 | Match exhaustiveness (user enums + closed unions ONLY, :72-:81) |
| `type_system/inference/extend_methods.rs` | 497 | `extend` block method registration |
| `type_schema/mod.rs` | 460 | `TypedFieldValue` carrier ABI (ADR-005 markers :27-:60), `SchemaError` |
| `type_schema/builtin_schemas.rs` | 460 | Builtin type schemas (field_kinds track, ADR-006 §2.7.26 marker :284) |
| `type_system/types/annotations.rs` | 368 | `TypeAnnotation <-> SemanticType` conversion |
| `type_system/errors.rs` | 360 | `TypeError` enum (thiserror), diagnostic rendering (`format_type` :181 — the Debug-leak site :196) |
| `type_system/storage.rs` | 341 | `StorageType` (NaN-sentinel Option layouts) — consumed by `shape-vm/src/type_tracking.rs:90` |
| `type_system/unification/unifier.rs` | 340 | Substitution store, `bind` with full occurs check (:54), annotation-embedded tyvar resolution |
| `type_system/universal_error.rs` | 318 | `UniversalError` shape for LSDS diagnostics |
| `type_system/environment/evolution.rs` | 315 | "Monotonic type growth" tracking — **write-only/dead**, see §3.4 |
| `type_schema/physical_binding.rs` | 279 | Arrow columnar buffer readers — all 5 `unsafe` blocks in territory (:59-:142) |
| `type_schema/current.rs` | 275 | Task-local/thread-local/process-default ambient registry (B1.7) |
| `type_schema/intersection.rs` | 233 | `A + B` schema merging with field-collision detection |
| `type_system/suggestions.rs` | 230 | Levenshtein "did you mean" |
| `type_system/types/builtins.rs` | 199 | `BuiltinTypes` constructors (number/int/bool/string/array/function) |
| `type_system/unification/structural_equality.rs` | 182 | Structural equality helpers |
| `type_schema/enum_support.rs` | 164 | `EnumVariantInfo` for schema-level enums |
| `type_system/mod.rs` | 145 | Re-exports + (duplicated) tests |
| `type_system/error_bridge.rs` | 114 | `TypeError -> ShapeError` conversion |
| smaller files | ~300 | `hoisting.rs` (138), `types/constraints.rs` (48), `types/mod.rs` (24), `unification/mod.rs` (17), `checking/mod.rs` (11) |

`crates/shape-types/` confirmed to be the documented empty skeleton: no `src/`, no `Cargo.toml`,
only `data/ES.1m.mktd` (16,831 bytes of market data — a stray fixture; see §3.4).

### 1.2 The three-layer checking pipeline (data flow)

Empirically and from call sites, a Shape program is type-checked by **three distinct layers**:

1. **Semantic analysis pass** (this vertical): `shape-vm`'s `BytecodeCompiler` calls
   `analyze_program_full(...)` (`crates/shape-vm/src/compiler/compiler_impl_reference_model.rs:2149`,
   `statements.rs:5716` for directive re-analysis, `functions_annotations.rs:2291` for comptime
   context). This constructs a `TypeChecker` -> `TypeInferenceEngine` -> `ConstraintSolver`
   pipeline and is used as an **error gate**: `Err(errors)` aborts compilation. When
   `TypeDiagnosticMode::Strict` (the default, `compiler_impl_initialization.rs:151`), analysis
   failures are fatal; `RecoverAll` exists for LSP-style recovery
   (`compiler_impl_reference_model.rs:2128-2162`).
2. **Emission-time type tracking** (shape-vm, out of territory but the inference *call-site*
   boundary): the compiler holds its own `type_inference: TypeInferenceEngine` field
   (`compiler/mod.rs:1018`, initialized `compiler_impl_initialization.rs:91`, used from 11 files —
   17 uses in `statements.rs` alone) *and* an independent per-expression typing pass for typed
   opcode emission (`compiler/expressions/binary_ops.rs:240` produces "Cannot infer types for
   binary operation ..."). `prove_native_kind()` (`type_tracking.rs`) is the kind-proof gate.
3. **Runtime kind guards**: e.g. "HashMap key must be a string (got kind Int64)" (t74/t78) and
   "Array.concat: element type mismatch" (t82) — partial, value-position gaps exist (t70).

The critical property: **layer 1's narrowing facts do not flow to layer 2.** The engine narrows
`x: Option<int>` to `int` inside `if x != None { ... }` (`inference/statements.rs:457-476`
applies `extract_narrowings` to a pushed scope), but layer 2 re-derives `x`'s type from the
binding annotation and rejects `x + 1` with its own error (t25/t26). `InferenceFacts`
(`inference/mod.rs:~58`) exists precisely to export per-binding facts to the compiler, but does
not carry flow-sensitive (span-scoped) facts.

### 1.3 Key types

- **`Type`** (`types/core.rs:93-112`): `Concrete(TypeAnnotation) | Variable(TypeVar) | Generic{base,args} | Constrained{var,constraint} | Function{params,returns}`. The inference-level representation.
- **`TypeVar(pub String)`** (`core.rs:47`): string-keyed type variables, `"T{n}"` from a per-engine `TypeVarGen` (:22-:43).
- **`TypeScheme`** (`core.rs:116-125`): quantified vars + trait bounds + default types; `instantiate_with_bounds` emits `ImplementsTrait` constraints per bound (:183-:228).
- **`SemanticType`** (`semantic.rs:21+`): user-facing vocabulary (Number/Integer/Bool/String/Option/Result/Array/Struct/Enum/Function/Ref/RefMut/TypeVar/Named/Generic/Void/Never).
- **`TypeConstraint`**: bounds — Numeric, Comparable, Iterable, HasField, HasMethod, ImplementsTrait, Callable (solved in `constraints.rs` phase 3).
- **`MethodTable` / `GenericMethodSignature`** (`checking/method_table.rs`): compile-time method signature registry with `TypeParamExpr` (`E::ReceiverParam(i)`, `E::MethodParam(i)`, `E::SelfType`) resolved per receiver.
- **`TypeSchemaRegistry`** (`type_schema/registry.rs:33`): per-runtime registry, content-addressed handle interning (`intern_content`), predeclared-schema caches — the WF-3A answer to the counter-collision family.
- **`StorageType`** (`storage.rs`): physical layouts (NaN-sentinel `Option<f64>`, validity-bitmap `Option<i64>`, 0/1/2 `Option<bool>`); projected to `NativeKind` in `shape-vm/src/type_tracking.rs:90`.

### 1.4 Entry points

- `analyze_program(program)` / `analyze_program_with_mode(...)` / `analyze_program_full(...)` (`checker.rs:642-:729`) — the compiler-facing gates.
- `TypeChecker::check_program` (`checker.rs:97`) — builder-configured (`with_source`, `with_known_bindings`, `with_analysis_mode`, `with_root_comptime_context`).
- `TypeInferenceEngine::new()` + `infer_*` family — used directly by the bytecode compiler via its `type_inference` field for expression-level queries.
- `quick_check(source)` (`checker.rs:777`) — test/tooling convenience.

---

## 2. Feature completeness

Every row below was **empirically tested** against the working-tree binary
(`target/debug/shape run`, scratch programs under the audit scratchpad, test IDs t01–t92).
"WORKS" means compiled and produced the expected output/error end-to-end; "CODE EXISTS" means the
machinery is present in-territory but does not reach users.

### 2.1 Strict-typing enforcement truth (the past-catastrophic lead) — VERIFIED FIXED

| Test | Program | Result |
|------|---------|--------|
| t01 | `let x: int = "hello"` | **Compile error** — "string is not compatible with int" |
| t12 | `let x: any = 5` | **Compile error** — no `any` escape hatch ("int is not compatible with any"; `any` treated as an unknown nominal, see §9.10) |
| t11 | `let x = []` | **Compile error** with an excellent message: "empty array `x` has an un-resolvable element type ... add an annotation (`let x: Array<T> = []`) or remove the unused binding" |
| t34/t35/t36 | `while n {}` / `if a && true {}` / `!a` with `a: int` | **Compile error** each — "int is not compatible with bool" (no-truthiness ruling enforced at compile time) |
| t06 | `if 1 { }` | **Compile error** |
| t32 | `a == b` with `a: int`, `b: number` | **Compile error** — int/number don't unify even under `==` |

`TypeDiagnosticMode` today: `Strict` and `RecoverAll` only (`shape-vm/src/compiler/mod.rs:555-559`);
the suppressing `ReliableOnly` variant is **deleted** with an in-code tombstone comment
(`compiler_impl_initialization.rs:151`), and a unit test asserts the default
(`compiler_impl_initialization.rs:1169`). The catastrophic finding from project memory is fixed at
the enum level (the variant cannot be re-selected).

### 2.2 Numeric conversion rules (user ruling: implicit ONLY if lossless) — WORKS

| Test | Program | Result | Verdict |
|------|---------|--------|---------|
| t03 | `let x: number = 2` | prints `2.0` | literal adopts context when lossless ✓ |
| t02 | `let i: int = 5; let x: number = i` | compile error | int variable does not widen (i64 ⊄ f64-exact) ✓ |
| t04 | `let n: number = 2.5; let i: int = n` | compile error | number→int needs cast ✓ |
| t05 | `n as int` on 2.9 | prints `2` | D2 truncation semantics ✓ (note: `ConvertToInt` is a VM-only opcode; the JIT preflight deopts whole-program — out-of-territory JIT gap) |
| t37 | `i as number` | prints `5.0` | explicit widening ✓ |
| t38 | `"42" as int` | compile error "Cannot assert type 'string' as 'int'" ✓ |
| t39 | `9223372036854775807 + 1` | **runtime** error "integer overflow ... widen explicitly with `as number` or `as bigint`" | D3 overflow→error ✓, but the hint recommends `as bigint` which does not compile (§9.4) |
| t88 | `let x: i8 = 300` | compile error | out-of-range literal rejected ✓ |
| t89 | `let x: i8 = 100; let y: int = x` | prints `100` | width-widening implicit (lossless) ✓ |
| t90 | `let x: u64 = 5u64; let y: int = x` | compile error "u64 is not compatible with int" | u64⊄i64 ✓ |

The implementation is the `NumericDomain` exact-representability lattice
(`constraints.rs:38-71`): subset-of-exactly-representable-range + float-direction rules. This is
principled and matches the ruled spec (`project_numeric_conversion_rule` memory) precisely.

### 2.3 HM let-generalization (user ruling) — WORKS

- t09: `fn get_none() { None }` then `let x: Option<int> = get_none()` + match — prints `none`. ✓
- t40: `fn id(x) { x }; let a: int = id(5); let s: string = id("hi")` — prints `5 hi` (two instantiations of one implicit scheme). ✓
- Value restriction implemented: `items.rs:857-:867` ("for an unannotated fn whose body is NON-EXPANSIVE w.r.t. its to-be-quantified [vars] ... EXPANSIVE bodies keep the [monomorphic scheme] — that is the int+string-through-one-slot" soundness case), expansiveness scan at `items.rs:2327-:2358`.

### 2.4 Flow-sensitive narrowing — DEAD END-TO-END (code exists)

| Test | Program | Result |
|------|---------|--------|
| t10/t19/t20/t23 | `if x != null { ... }` (CLAUDE.md's documented spelling) | **parse error** — `null` is reserved (`shape.pest:1570`) but has no literal production (only `none_literal = "None"`, `shape.pest:1424`) |
| t25 | `let x: Option<int> = Some(41); if x != None { print(f"{x + 1}") }` | **compile error** "operand types are `Option` and `int`" |
| t26 | same in fn with `return x + 1` | **compile error** at the narrowed use |
| t29 | `if x != None { let y: int = x }` | **compile error** "Option<int> is not compatible with int" |

The narrowing implementation exists and is unit-tested (`inference/statements.rs:820-905`:
`extract_narrowings` for `x != None`, `extract_inverse_narrowings` for else-branch of `x == None`,
`unwrap_optional_type` handling both `Option<T>` encodings) — but the compiler's emission layer
does not consume it. **Working paths for Option consumption**: `match` (t09), `??` (t30, prints
`42`), optional chaining `cfg?.server?.port ?? 8080` (t87, prints `8080`). `.unwrap()` does not
exist (t31: "Method 'unwrap' not found on type 'Option'").

Also note t21: `let x: int? = 41` is a compile error ("int is not compatible with Option<int>") —
`T?` is strict sugar for `Option<T>` with no auto-wrapping; assignment requires `Some(41)`.

### 2.5 Bare unparameterized generic names (user ruling: invalid everywhere) — PARTIAL

| Test | Position | Result |
|------|----------|--------|
| t13 | `fn f(x: Option) -> int` + call `f(Some(1))` | rejected (via constraint failure, Debug-leaking message) |
| t14 | `fn g() -> Option { None }` | rejected |
| t15 | `let x: Option = None` | rejected |
| t08 | `let x: Array = []` | rejected |
| t07 | `fn f(x: Option) -> int { 1 }` **never called** | **ACCEPTED** — compiles & runs |
| t16 | `type T { m: HashMap }` (field, unused) | **ACCEPTED** — compiles & runs |

Enforcement is a *side effect of unification* (`Generic{Option, [T62]}` vs bare `Option` fails to
solve), not a resolution-layer validity rule. Any position whose annotation never meets a
constraint accepts the bare name — directly violating the ruling's "annotation/param/return/field ...
anywhere" scope (§9.8).

### 2.6 Generics, traits, enums, structs — LARGELY WORKS

| Test | Feature | Result |
|------|---------|--------|
| t44 | trait with `fn greet(self)` (CLAUDE.md's documented syntax) | compile error with a good message: "method receivers are implicit. Use `method greet(...)` without `self`" — CLAUDE.md syntax is stale (§8) |
| t51 | trait + impl with `method` syntax + generic bound `fn hello<T: Greet>` | **works** ("woof rex") |
| t45 | call with unbound type | **works** — "trait bound not satisfied: Type 'Cat' does not implement trait 'Greet'" |
| t52/t53 | `trait Pet extends Animal` (CLAUDE.md syntax) | **parse error** — real syntax is `trait Pet: Animal` (`shape.pest:198` `supertrait_list = { ":" ~ ... }`) |
| t54 | `trait Pet: Animal` + both impls | **works** ("rex owned by ann") |
| t55 | `impl Pet` without `impl Animal` | **works** — supertrait obligation enforced ("Type 'Cat' does not implement trait 'Animal'") |
| t65 | `let g: dyn Greet = Dog{...}; g.greet()` | **works** ("woof rex") |
| t46 | `type Box<T> { value: T }` + inference from literal | **works** (`43`) |
| t47 | struct literal wrong field type | **works** — rich diagnostic with `note:` pointing at field decl |
| t48 | struct literal missing field | **works** — with `help: add \`y\`` |
| t41/t42 | user-enum match exhaustiveness | **works** — "missing variants Blue" |
| t64 | union param `x: int \| string` | compiles & calls |
| t86 | `instanceof` narrowing on union | **compile error** (dead, §9.5) |

### 2.7 Collections & bidirectional inference

| Test | Feature | Result |
|------|---------|--------|
| t56 | `arr.filter(\|x\| x % 2 == 0)` / `arr.map(\|x\| x * 2)` on `Array<int>` | **works** — closure params inferred from receiver element type |
| t63 | `let x: Array<int> = ["a"]` | rejected ✓ |
| t75/t76 | `a.push("x")` on `Array<int>` | rejected at compile time with a model diagnostic ✓ (special-cased hard check, `inference/expressions.rs:1587-1637`) |
| t66 | `HashMap()` + `.set("a",1)` + `.get("a") ?? 0` | **works** (`1`) |
| t79/t80/t81 | `get` result used at wrong type | rejected ✓ (method-table return types enforced) |
| t69/t70 | `.set("a","oops")` on `HashMap<string,int>` | **ACCEPTED — P0 unsound**, §9.1 |
| t74/t78 | `.set(1,2)` / `.get(42)` wrong key | compiles; **runtime** error only |
| t82 | `a.concat(["x"])` on `Array<int>` | compiles; runtime error only |
| t84 | `a.includes("x")` on `Array<int>` | compiles; silently `false` |
| t57/t58 | `.insert(...)` (not a real method) | rejected, but via Debug-leak "cannot have fields" + "Method 'insert' not found" |

### 2.8 Primitive type zoo

| Type | Status | Evidence |
|------|--------|----------|
| `int`, `number`, `bool`, `string` | WORKS | throughout |
| `i8/u8/i16/u16/i32/u32/u64` + literal suffixes | WORKS (typing level) | t88/t89/t90; book integer-types.mdx |
| `decimal` | WORKS with `D` suffix | t67: `1.5D + 2.5D` prints `4.0D`; **lowercase `1.5d` parses as Duration (days)** producing the baffling "Duration is not compatible with decimal" (t60, §9.11) |
| `bigint` | **UNUSABLE** | t61 (`...n` literal: parse error), t91/t92 (`as bigint`: "Cannot assert"); no stdlib constructor (grep: only `stdlib/json.rs` mentions it) — §9.4 |
| tuples | **MISSING (parse level)** | t71/t72/t73 all parse-error on `(1, "one")`; `TypeAnnotation::Tuple` handling exists across the territory (errors.rs:212, unifier.rs:138, occurs check :246) — carrier code with no producer |
| `DateTime`/`Duration` | typed (Duration surfaced in t60) | not deeply audited (other vertical) |

### 2.9 Exhaustiveness

- User enums: **checked** (t41). Wildcard/identifier catch-all recognized; `where`-guarded arms
  correctly don't count (`exhaustiveness.rs:7-9`).
- Closed unions: code path exists (`check_exhaustiveness_for_type` -> `check_union_exhaustiveness`, :106-:115).
- **`Option`/`Result`: NOT checked** — `check_exhaustiveness` matches only `SemanticType::Enum`
  (:72-:81); `SemanticType::Option`/`Result` fall to `NotApplicable`, whose `is_exhaustive()` is
  `true` (:41-:48). Empirical: t43 (missing `Err` arm compiles+runs), t49/t50 (**runtime crash**
  "No match arm matched the value"). §9.3.

---

## 3. Code quality

### 3.1 Idiom & naming — good, with two structural sins

The territory is modern Rust: `thiserror` for `TypeError` (`errors.rs:11`), builder-pattern
`TypeChecker` (`checker.rs:42-95`), RAII scope guards for the ambient registry
(`type_schema/current.rs`, `SyncRegistryScope`), per-engine `TypeVarGen` instead of process
statics (`core.rs:15-24` — explicitly motivated by cross-test ID collision), and unusually strong
comment discipline: most non-obvious blocks carry a dated provenance tag (e.g. "STAGE F1
(strict-flip, 2026-06-20)" at `constraints.rs:1167`, "R3-subcase struct-array HOF (strict-flip,
2026-06-14)" at `bidirectional.rs:484`).

Two structural sins:

1. **String-keyed type variables.** `TypeVar(pub String)` (`core.rs:47`) means every fresh var
   allocates, every substitution-map op hashes a string, and identity is convention ("T{n}").
   Worse, `Type::to_semantic()` recovers a numeric `TypeVarId` by
   `var.0.trim_start_matches('T').parse::<u32>().unwrap_or(0)` (`core.rs:424-426`, again
   `:458-459`): any var not named `T<digits>` — including user type params `T`, `U`, `K`, `V`
   which the engine stores by their source names (`items.rs:87` "quantifies over `TypeVar("T")`") —
   collapses to `TypeVarId(0)`. Two distinct user params in one signature can alias in the
   `SemanticType` projection (§9.12).
2. **The `"\u{1}tyvar:"` marker encoding** (`core.rs:55-89`): because `TypeAnnotation` has no
   variable variant, unresolved vars inside object-literal field types are smuggled through
   `TypeAnnotation::Basic("\u{1}tyvar:T7")` strings. It is carefully done (SOH byte can't collide
   with identifiers; occurs check decodes markers, `unifier.rs:213-268`; WF-6 stack-overflow
   regression test at `unifier.rs:300-327`) — but it means the substitution machinery must
   string-parse inside what claims to be a *concrete* type, and every new `TypeAnnotation`
   consumer must know about it. This is a stringly-typed parallel channel through the AST type.

### 3.2 Error handling — sound structure, leaky rendering

`TypeResult<T> = Result<T, TypeError>` throughout; no panics on user input observed in ~90
adversarial programs (worst case is a parse error with a slightly wrong span). But diagnostic
rendering leaks internals:

- `errors.rs:196`: `format_type` falls back to `format!("{:?}", ty)` whenever `to_annotation()`
  returns `None` — which is exactly the interesting case (a `Generic` containing any
  `Type::Variable`). Users see
  `Generic { base: Concrete(Reference(TypePath { segments: ["Option"], qualified: "Option" })), args: [Variable(TypeVar("T62"))] }` (t13/t14/t15).
- `constraints.rs:~1225` (`"{:?} cannot have fields"`) and `:~1214` (`"{:?} does not have field"`)
  — same leak class, observed in t57.
- The shape-vm binary-op message leaks too (t86: `Concrete(Union([Basic("int"), Basic("string")]))`),
  showing the leak pattern was copied across layers.
- `errors.rs:211`: `TypeAnnotation::Array` renders as `Vec<...>` while the language surface says
  `Array<...>` — user-visible vocabulary drift.

### 3.3 Unsafe usage — 5 blocks, all in one file, API-misdesigned

All 5 `unsafe` blocks in the territory live in `type_schema/physical_binding.rs`
(:59, :86, :109, :127, :142) — Arrow columnar buffer readers (`read_f64`, `read_i64`,
`read_bool`, `read_str`, `is_null`). Problems:

- Every reader documents "# Safety — Caller must ensure `row_idx < table.row_count()`" **but is a
  safe `pub fn`** — a textbook unsoundness-by-API: safe code can trigger UB by passing a large
  index. They should be `unsafe fn` or bounds-checked.
- `read_str` (:127) uses `std::str::from_utf8_unchecked` on buffer bytes.
- Wrong-datatype reads silently return `f64::NAN` / `0` (:73, :96) — soft-fail contrary to the
  repo's surface-and-stop doctrine.
- Mitigating: the type is only re-exported (`type_schema/mod.rs:82`); no caller outside the module
  was found in the workspace (grep §pre-report), so this is **dormant risk + dead-ish code**, not
  an active hole.

### 3.4 Dead code in-territory

- **`environment/evolution.rs` (315 LOC)**: "monotonic type growth where variables can have
  fields added through assignment" — a pre-strict-typing concept that contradicts the current
  doctrine. Data is *recorded* (`inference/expressions.rs:2633` `record_field_assignment`) but the
  read APIs `get_evolution`/`all_evolutions` (`environment/mod.rs:1905/:1935`) have **zero
  non-test callers workspace-wide** (grep verified). Write-only machinery.
- **`physical_binding.rs`**: no consumer outside its own module/tests (see §3.3).
- **`checker.rs:771` `type_of_expr(expr, _env)`**: takes an ignored `_env` parameter — vestigial
  API.
- **Tuple machinery**: `TypeAnnotation::Tuple` arms in `errors.rs:212`, `unifier.rs:138-143`,
  `annotation_occurs` (:246) are unreachable from source (tuples don't parse, t71-73).
- `crates/shape-types/data/ES.1m.mktd` — a 16 KB market-data fixture in an otherwise empty crate
  skeleton; nothing references it.

### 3.5 Complexity hotspots

- `inference/items.rs` (4,903) and `inference/expressions.rs` (4,495) are the two monsters; the
  method-call inference match arm in expressions.rs runs ~400 lines (:1440-:1840) with five
  layered special cases (Table.select fast-path, method_vars minting, K/V eager binding,
  push hard-check, map/flatMap return-position binding) — this is where the P0 lives, and its
  density is why the soft/hard checking gap wasn't noticed.
- `environment/mod.rs:1073` `define_builtin_functions` is a single ~360-line function
  hand-registering builtin signatures — a parallel source of truth to the actual stdlib registry
  (split-brain risk, §5.4).
- `constraints.rs::solve_constraint` match spans several hundred lines with deep nesting
  (:260-:1250 region) including the struct-schema structural unification.

### 3.6 Test hygiene

480 `#[test]`s in territory, **0 `#[ignore]`** — a notably clean ratio for this repo (contrast
the ~23 ignored shape-jit tests). Tests are co-located per CLAUDE.md convention. Assertion quality
is generally high (e.g. `unifier.rs:309-327` asserts both refusal-to-store AND termination;
`inference_tests.rs` asserts full scheme shapes, not just is-ok).

---

## 4. Duplication & DRY violations

1. **Duplicated test bodies**: `type_system/mod.rs:67-116` (`test_type_to_semantic_primitives`,
   `_option`, `_result`) are near-verbatim copies of `types/core.rs:577-619`. Harmless but pure
   waste; a drift in one silently stops covering the other's intent.
2. **Two narrowing extractors**: `extract_narrowings` (statements.rs:822) and
   `extract_inverse_narrowings` (:847) duplicate the binary-op destructuring with only the
   operator flipped — a `fn narrowings_for(op)` would halve it. Low risk (both dead in practice,
   §2.4).
3. **Three type-rendering paths**: `errors.rs::format_type`/`format_annotation` (:181-:269),
   `annotation_to_string` (types/annotations.rs re-export), and the engine's
   `render_type_for_diag` (used at `expressions.rs:1630`). They already disagree: `format_annotation`
   prints `Vec<...>` for arrays (:211) while push-diagnostics print `Array`-style names via
   `render_type_for_diag` (t75 says "element type is `int`" with `Array` phrasing). Divergence is
   user-visible vocabulary inconsistency, dangerous only to UX.
4. **Vec/Array alias round-trip asymmetry**: `canonicalize_collection_base` folds `Vec` -> `Array`
   (`core.rs:317-324`) as U1's canonical spelling, but `SemanticType::Array::to_inference_type`
   *produces* base `"Vec"` (`core.rs:511-514`). Every semantic->inference round trip re-mints the
   non-canonical alias that canonicalization must fold again. Works today because
   `is_array_or_vec_base` (`constraints.rs:74-81`) accepts both — i.e. the alias-tolerance is
   duplicated in a *third* place. One missed `Vec`-acceptance site = latent unification failure.
5. **HashMap runtime key-guard vs compile-time signature**: the key type is enforced twice
   (method table `E::ReceiverParam(0)` at `method_table.rs:833` + runtime "HashMap key must be a
   string" guard) while the value type is enforced **zero** times (§9.1) — the duplication
   pattern inverted into a gap.
6. **`format_unsolved_constraints` duplicated concept**: `inference/mod.rs:1276` and `:1346` carry
   the same "call result type '{}' is not compatible with proven return type '{}'" message built
   twice at two call sites.

---

## 5. Split-brain analysis

### 5.1 Engine-vs-compiler type checking (the big one)

Same concept — "what is the type of this expression" — implemented twice:

- **Runtime engine** (this territory): full HM with constraints, narrowing, bidirectional modes.
- **Bytecode compiler** (shape-vm): per-expression type derivation for typed-opcode emission
  (`binary_ops.rs:240` error text; `prove_native_kind` in `type_tracking.rs`), plus its own
  `TypeInferenceEngine` *instance* consulted ad-hoc from 11 files.

Drift evidence (all empirical): narrowing works in layer 1's unit tests
(`statements.rs:908-970+`) but layer 2 rejects the narrowed use (t25/t26); `instanceof`
narrowing documented and presumably layer-1-visible, layer 2 rejects (t86). Risk: any future
type-system feature (e.g. the ruled generic-args requirement) must be implemented twice or it
only half-exists. The `InferenceFacts` export (`inference/mod.rs`) is the intended bridge but
carries only per-binding facts, not flow-scoped facts.

### 5.2 Type vocabulary triple representation

`TypeAnnotation` (AST) vs `Type` (inference) vs `SemanticType` (user-facing) with 6 conversion
functions (`annotation_to_semantic`, `semantic_to_annotation`, `to_annotation`, `to_semantic`,
`to_inference_type`, `canonicalize_annotation`). Each conversion is lossy in a different way:
`to_annotation` loses vars (-> `"unknown"`, `core.rs:401/:406` — the CLAUDE.md "known constraint",
whose cite `core.rs:218` is now stale; the code moved to :362-:413), `to_semantic` loses var
identity (`unwrap_or(0)`, §9.12), `annotation->semantic->annotation` loses `Vec`/`Array` spelling.
U1 canonicalization (`core.rs:297-359`) is the documented fix for the *worst* historical
split-brain (three encodings of `Array<T>`, STRUCTURAL-AUDIT SB-4) and is real progress — but it
normalizes at *use sites* (equality/solver entry) rather than at construction, so non-canonical
values still circulate.

### 5.3 Exhaustiveness: two entry points

`check_exhaustiveness(match, SemanticType)` (:66) and `check_exhaustiveness_for_type(match, Type)`
(:106) — the latter adds union handling the former lacks; a caller choosing the former silently
loses union exhaustiveness. Both share the Option/Result blind spot (§9.3).

### 5.4 Builtin signatures: hand-registered vs actual stdlib

`environment/mod.rs:1073` `define_builtin_functions` hand-declares signatures ("print:
<T>(T) -> void") in parallel with the real builtin registry in shape-runtime and the PHF method
registry in shape-vm (`method_registry.rs`). Similarly, `checking/method_table.rs` re-declares
every Vec/String/HashMap method signature that the VM's PHF dispatch table declares again for
execution. Drift evidence: `insert` exists in neither (fine), but `remove` had to be retrofitted
("Previously absent from the inference table", `method_table.rs:851-856`) — proof the two tables
have historically diverged.

### 5.5 Doc-vs-code split-brains (details in §8)

- CLAUDE.md narrowing spelling `!= null` vs grammar `None`-only.
- CLAUDE.md trait syntax (`fn method(self)`, `extends`) vs enforced `method` keyword + `:` supertraits.
- Book "Exhaustiveness is statically checked" vs Option/Result runtime crash.
- Book `instanceof` narrowing vs compile error.
- Book/CLAUDE.md `bigint` vs no construction path; runtime overflow hint recommends non-compiling `as bigint`.
- `constraints.rs:29` claims "Robinson's algorithm with path compression" — `Unifier` does chain-walking
  (`apply_substitutions` recurses per lookup, `unifier.rs:67-105`) with **no** compression.
- `bidirectional.rs:25-27` module doc says Synth "emits the constraint" — `synth_with_hint` (:483)
  explicitly does NOT ("if it fails, just return inferred ... doesn't force"). The doc describes
  the sound behavior; the code implements the unsound one. This exact drift is the P0.

---

## 6. ADR & spec conformance

Rule-by-rule for the ADR clauses that bind this territory (markers grepped: 22 `// ADR-005` /
`// ADR-006` sites in-territory).

### 6.1 ADR-005 (typed-slot construction / single discriminator)

| Rule | Status | Evidence |
|------|--------|----------|
| §1 Single discriminator: layers above HeapValue dispatch on `HeapValue::kind()`, no parallel sum types projecting 1:1 to HeapKind | **CONFORMS** | `TypedFieldValue` (`type_schema/mod.rs:41-63`) has scalar variants + `String` + a single `Heap(Arc<HeapValue>)` catch-all; the header comment (:27-:40) explicitly forbids per-HeapKind variants and demands ADR-level justification for additions. The scalar variants (F64..U64, Bool) are native scalars, not HeapKind projections — conformant. |
| §2 The one String exception, named and bounded | **CONFORMS** | `String(Arc<String>)` at `mod.rs:53-58` cites the ADR and the measurement rationale verbatim. No second exception found. |
| §Forbidden: no `from_heap_arc(Arc<HeapValue>)` catch-all (Q6) | **CONFORMS** | grep for `from_heap_arc` and `Box<HeapValue>` in territory: zero hits. |
| Typed pointers, no `Box<HeapValue>` wrapping | **CONFORMS in-territory** | zero `Box<HeapValue>` hits in `type_system/` + `type_schema/`. |

### 6.2 ADR-006 (value & memory model)

| Rule | Status | Evidence |
|------|--------|----------|
| §2.7.5 stamp-at-compile-time (kinds are compile-time projections, never fabricated from bits) | **CONFORMS** | `FieldType::to_native_kind()` *refuses* static projection for `Any`/`Option`/`Array`/`HashMap`/`Set` rather than guessing (`field_types.rs:90-125`); `FieldKindError::AnyTypeNotStrictlyTyped` (:14-:27) makes "Any reached NativeKind" a hard error. Closure return stamping cites the clause (`inference/expressions.rs:1365`, `:1656`). |
| §2.7.7 / §2.7.26 carrier-authoritative field_kinds track | **CONFORMS** | `builtin_schemas.rs:284` registers the parallel `field_kinds` track with the clause cite. |
| §2.7.30 escape-RC-promote / value-position auto-deref | **CONFORMS (in-territory share)** | `inference/access.rs:1490` (GapA value-position auto-deref for `-> &T` callees), `operators.rs:730` (borrow-transparent operand reads). |
| §2.7.31 structural schema identity (WF-3A) | **CONFORMS** | `TypeSchemaRegistry` interns by `SchemaContentId` (`registry.rs:44-56`): "no code path assigns a handle except `intern_content`"; `next_id` demoted to a dense intern-index allocator, tests assert structural dedup (`registry.rs:1123-1233`). The `project_schema_id_collision_family` memory item's root fix has **landed**. Residual: the header still documents a "B1 migration window" where `TypeSchema::new` bumps a global static (`registry.rs:29-32`) — migration incomplete but honestly documented. |
| §Forbidden (CLAUDE.md): no `SlotKind::Dynamic`/`Unknown`, no generic opcodes, no ValueWord, no tag_bits | **CONFORMS** | greps for the forbidden symbol families in-territory: zero live hits. The only "dynamic" in-territory is `FieldType::Any`, which is (a) not a runtime dispatch mechanism, (b) hard-erroring at NativeKind projection, and (c) documented as a shrink-to-zero target (`field_types.rs:10-18`). NOT a P0 forbidden-pattern hit, but see caveat below. |
| No runtime coercion / no dynamic fallback / inference failure = compile error | **CONFORMS at the rule level, one hole** | t01–t12 battery; the empty-array diagnostic (t11) is the exemplar. The hole is the soft-`Synth` method-arg path (§9.1) — not a *dynamic fallback* (no fallback opcode is emitted; the wrong value is emitted *as if* well-typed), but it breaks the same "compile-time proof" contract the forbidden patterns protect. |

**Caveat worth surfacing:** `FieldType::Any` has 228 references workspace-wide and a live
producer fallback in the adjacent compiler (`shape-vm/src/compiler/helpers.rs:5682`
`TypeAnnotation::Borrow{..} => FieldType::Any`). In-territory it is contained (projection
refuses); the *producer* side is the thing to keep shrinking. This matches the documented
narrowed-exception list (W17.2-C §4.D.7) rather than a defection, but it is exactly the shape
that historically re-grew.

### 6.3 Known-constraints register accuracy (CLAUDE.md §Known Constraints)

- "`Type::to_annotation()` TypeVar loss at core.rs:218" — **behavior confirmed, cite stale**: the
  `"unknown"` freezing now lives at `core.rs:401` and `:406` (function param/return positions);
  `to_annotation` starts at `:362`. `core.rs:218` is now inside `instantiate_with_bounds`. The
  compensating claim ("`BuiltinTypes::function()` preserves them, regression test
  constraints.rs:1193") also drifted: `constraints.rs:1193` is now inside the HasField
  struct-schema solver arm, not a test.
- "Queryable<T> generic impl parses but type-inference erases type args" (`statements.rs:788`,
  `items.rs:514/:677` cites) — not re-verified line-exactly here, but the cited files' line
  numbers have demonstrably drifted elsewhere; the register needs a refresh pass.

---

## 7. Test coverage in-territory

### 7.1 Counts

- **480 `#[test]` functions** across `type_system/` + `type_schema/`; **0 `#[ignore]`**.
- Distribution (top): `inference/inference_tests.rs` 144, `inference/items.rs` 42,
  `type_schema/field_types.rs` 30, `constraints.rs` 27, `environment/registry.rs` 22,
  `checking/method_table.rs` 22, `environment/mod.rs` 17, `inference/expressions.rs` 16,
  `semantic.rs` 15, `inference/operators.rs` 14, `type_schema/{schema,registry}.rs` 13 each.
- The territory participates in the workspace `deep-tests` tiering only indirectly (no
  deep-gated modules in-territory).

### 7.2 Assertion quality — spot checks

- `unification/unifier.rs:286-339`: occurs-check tests assert positive AND negative cases plus
  the WF-6 stack-overflow regression (refusal to store + termination + acyclic still stored).
  Model quality.
- `inference/statements.rs:910+`: narrowing unit tests build the exact AST (`x != None`) and
  assert extracted narrowings — **good tests of dead code**: they pin layer-1 behavior that no
  user can reach (§9.2). Nothing in-territory or in shape-test exercises narrowing end-to-end
  through the compiler, which is why the split-brain is invisible to CI.
- `constraints.rs` numeric-domain tests cover the ruled 104-case conversion matrix's core
  (subset checks per width pair).
- `checker.rs` has only 7 tests for 984 lines including the 5 public entry points — the
  gate-behavior (FailFast vs RecoverAll interplay with the compiler's TypeDiagnosticMode) is
  under-covered in-territory (it is covered indirectly by shape-vm compiler tests).

### 7.3 Gaps (each maps to a §9 bug that tests would have caught)

1. **No negative-path test for generic-method value args** (`m.set(wrong_type)`): 22 tests in
   `method_table.rs` all test signature *resolution*, none test *rejection*. The P0 (§9.1) lives
   exactly in this gap.
2. **No end-to-end narrowing test** compiled through the bytecode compiler (only engine-level
   unit tests).
3. **No exhaustiveness tests for Option/Result scrutinees** (`exhaustiveness.rs` tests use
   `EnumVariant` fixtures only — `#[cfg(test)] use super::semantic::EnumVariant` at :17).
4. **No test that `bigint`/tuple annotations are constructible** — carrier arms exist untested
   against any producer.
5. **physical_binding.rs**: 7 tests but none adversarial (no out-of-bounds, no wrong-datatype
   assertions on the silent-NaN paths).

---

## 8. Book/docs vs reality for this vertical

Book root: `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/`.

| Claim | Source | Reality (measured) |
|-------|--------|--------------------|
| "Exhaustiveness is statically checked" | `fundamentals/pattern-matching.mdx:6` (llm_summary) | True for user enums (t41); **false** for the built-in enums the same book advertises ("Built-in enums: Option<T> ... Result<T,E>", `enums.mdx:6`): missing `Err`/`None` arms compile and crash at runtime (t43/t49/t50) |
| "Match on enums must be exhaustive — cover all variants or use `_`" | `enums.mdx:9` | Same as above — holds only for user enums |
| `instanceof` "narrows a union type to one of its members. In an `if instanceof` branch the variable is narrowed" | `operators.mdx:159-160` | **Does not compile** (t86): "Cannot infer types for binary operation `Add`: operand types are `Concrete(Union([...]))` and `int`" |
| "`expr?.prop` lowers to `match ... `, reusing Shape's Option pattern matching and flow narrowing" | `operators.mdx:474` | `?.` itself works (t87); the "flow narrowing" it namechecks is dead (§9.2) |
| "For arbitrary-precision integers, use `bigint`" (×5 mentions) | `integer-types.mdx:30,37,71,142,146` | **No way to construct one**: no literal suffix (grammar `int_width_suffix` = i8..u64 only, `shape.pest:1612`), `as bigint` rejected (t91/t92) |
| "HashMap is functional/immutable — .set() returns a NEW map" | `builtin-types.mdx:6` | API shape confirmed (t66); the set-value type hole (§9.1) undermines the typed-container claim |
| CLAUDE.md: "Flow-sensitive narrowing: `if x != null { ... }` narrows `T?` to `T`" | CLAUDE.md §Type System Rules | **Both halves false**: `null` cannot parse (reserved, no production — `shape.pest:1570` vs `:1405-1424`); the `None` spelling doesn't narrow through compilation (t25-29) |
| CLAUDE.md: "Traits: `trait Name { fn method(self) -> ReturnType; }` with `extends` for supertraits" | CLAUDE.md §Language Features | Both stale: explicit `self` is a **compile error** ("Use `method greet(...)` without `self`", t44); `extends` is a **parse error**, real syntax `trait Pet: Animal` (t52/t54, `shape.pest:197-198`) |
| CLAUDE.md: "tuples" in the types list | CLAUDE.md §Language Features | Tuple literals/annotations **do not parse** (t71-73) |
| CLAUDE.md: known-constraint cite `core.rs:218` | CLAUDE.md §Known Constraints | Stale — behavior at `core.rs:401/:406` now (§6.3) |
| "Bidirectional closure inference ... `arr.filter(\|x\| ...)` infers x's type" | CLAUDE.md | **True** (t56) — and the mechanism (`Synth` hints from `GenericMethodSignature`) is exactly as documented |
| "Flow-sensitive narrowing" for `if x != None` in stdlib patterns | `stdlib` docs (not exhaustively audited) | risk of copy-pasted dead patterns in examples |

Pattern: the book's *fundamentals* chapters are accurate about what works (numeric rules,
integer widths, HashMap immutability, `?.`/`??`), but every claim in the
narrowing/exhaustiveness/bigint cluster is aspirational. CLAUDE.md's language-features section
has at least four stale syntax claims — dangerous because agents write test programs from it
(this audit lost ~6 programs to `null`, `extends`, `fn(self)`, tuple syntax before consulting
the grammar).

---

## 9. Bugs & correctness risks found

All transcripts are from `target/debug/shape run` on the working tree, 2026-07-11 (extension-load
warnings elided). Scratch sources under
`/tmp/claude-1000/-home-dev-dev-shape-lang-shape/.../scratchpad/verticals/type-system/`.

### 9.1 [P0] Generic-method value arguments are soft-checked → heap-pointer reinterpretation

**Repro (t69, t70):**

```shape
let m: HashMap<string, int> = HashMap()
let m2 = m.set("a", "oops")        // V is int; "oops" is string — ACCEPTED
let v: int = m2.get("a") ?? 0
print(f"{v + 1}")
```

```text
$ shape run t70_hashmap_unsound.shape
106644639314833
```

A `string` heap pointer is stored where the schema says `int`, read back through the
`int`-typed path, and arithmetic is performed on the pointer bits. This is the exact
"string→int compiles+runs+reinterprets heap pointer as i64" class that
`project_reliableonly_strict_bypass` (user's 2026-05-29 top release-blocker) was about —
resurfacing through a different door.

**Root cause chain (all in-territory):**

1. `inference/expressions.rs:1508-1520` — method args whose *expected* type is concrete are
   checked with `CheckMode::Synth(expected)`:
   `Some(ty) => self.check_expr(arg, CheckMode::Synth(ty.clone()))`.
2. `inference/bidirectional.rs:483-529` — `synth_with_hint`: *"Try to unify with hint - if it
   fails, just return inferred. This is a 'soft' constraint that helps but doesn't force."*
   `probe_equal(string, int)` is false → returns `string` **without error**.
3. `inference/expressions.rs:~1550-1560` — the hard-constraint block is *"bounded TIGHTLY to
   expected param types that are a bare `Type::Variable`"*; concrete expecteds are documented as
   *"LEFT ALONE ... force-constraining those would reject valid calls like `[1,2,3].concat([])`,
   `[1,2,3].zip(["a"])`, or `[1,2,3].includes(None)`"*. So no constraint is ever emitted for
   `set`'s value arg once V is concrete.
4. No runtime guard exists for HashMap *values* (keys have one — t74/t78 error with "HashMap key
   must be a string (got kind Int64)").

**Interlocking evidence:** with an *unannotated* receiver (`let m = HashMap()`), K/V are fresh
vars → the hard-bind path fires → `.set("a", 1)` correctly pins V=int and a later wrong-typed
read errors (t81). The hole opens exactly when the user adds MORE type information (the
annotation), which resolves `ReceiverParam(1)` to concrete `int` and routes the arg to the soft
path. Worse-is-better inversion: annotating your map makes it unsafe.

**Severity: P0** — wrong results + memory-content disclosure (pointer values observable), fully
strict-mode, no unsafe/FFI involved.

### 9.2 [P1] Flow-sensitive narrowing dead in both spellings

```text
$ cat t23_null_compare.shape
let x: int? = 41
if x != null { print("not null") }
$ shape run t23_null_compare.shape
error[E0001]: unexpected `}`, expected something else   # null: reserved word, no production
```

```text
$ cat t26_narrow_fn.shape
fn f(x: Option<int>) -> int {
    if x != None { return x + 1 }
    return 0
}
$ shape run t26_narrow_fn.shape
error[RUNTIME]: ... Cannot infer types for binary operation `Add`: operand types are `Option`
and `int`. Strict typing requires both operands to have a known concrete type at compile time.
```

- Grammar: `null` is in the reserved-word list (`shape.pest:1570`) but the only absence literal is
  `none_literal = @{ "None" ... }` (`shape.pest:1424`). Any `x != null` is a guaranteed parse error.
- Engine narrowing exists and is correct for `x != None` / inverse `x == None`
  (`inference/statements.rs:820-905`) with unit tests — but the error above is produced by the
  *compiler's* independent binary-op typing (`shape-vm/src/compiler/expressions/binary_ops.rs:240`),
  which never sees the narrowed environment. Also unreachable for assignment (t29:
  `let y: int = x` inside the guard → "Option<int> is not compatible with int" — this one from
  the engine itself, meaning even layer 1 doesn't apply its own narrowing to `let` RHS uses).
- Note t25's error span points at line 1 (`let x ...`) instead of the offending `x + 1` — span
  quality bug riding along.

**Severity: P1** — a documented core feature (CLAUDE.md + book) with zero working path; the
workaround (`match`/`??`) exists, which keeps it out of P0.

### 9.3 [P1] Option/Result match exhaustiveness unchecked → guaranteed runtime crash

```text
$ cat t49_result_err_unmatched.shape
fn f() -> Result<int, string> { Err("boom") }
let r = f()
match r { Ok(v) => print(f"{v}") }
print("after match")
$ shape run t49_result_err_unmatched.shape
Uncaught exception:
Error: No match arm matched the value
```

Same for `Option` (t50). Cause: `check_exhaustiveness` handles only `SemanticType::Enum`
(`exhaustiveness.rs:72-81` — comment admits "Only check enums for now"); `Option`/`Result`
project to dedicated `SemanticType` variants (`semantic.rs:36/:40`) and fall through to
`NotApplicable`, which `is_exhaustive()` treats as fine (:41-:48). The compiler emits a match with
no fallback arm → runtime abort. Book explicitly promises static checking (§8).

**Severity: P1-high** — sound (it crashes rather than corrupts) but violates the language's
central static-safety promise on its two most-used enums.

### 9.4 [P1] `bigint` unusable; runtime error message recommends a non-compiling fix

```text
$ shape run t61_bigint.shape       # let b: bigint = 123456789012345678901234567890n
Error: Parse error: Invalid integer: number too large to fit in target type

$ shape run t91_bigint_cast.shape  # let b: bigint = 5 as bigint
error[RUNTIME]: ... Cannot assert type 'int' as 'bigint'

$ shape run t39_overflow.shape     # 9223372036854775807 + 1
Error: Runtime error: integer overflow: result exceeds the int (i64) range;
widen explicitly with `as number` or `as bigint`     # <-- does not compile
```

No literal (grammar suffixes are i8..u64 only, `shape.pest:1607-1612`), no cast, no stdlib
constructor (grep across `stdlib/`: only `json.rs` mentions bigint). Wire/marshal/snapshot layers
all carry BigInt variants — a value type with serialization support but no birth path.

### 9.5 [P1] `instanceof` union narrowing does not compile (t86)

```text
fn describe(x: int | string) -> string {
    if x instanceof int { return f"int {x + 1}" }
    return "string"
}
→ error: Cannot infer types for binary operation `Add`: operand types are
  `Concrete(Union([Basic("int"), Basic("string")]))` and `int`.
```

Book documents this exact pattern (`operators.mdx:159-160`). Same split-brain root as §9.2, plus
a Debug-repr leak in the message.

### 9.6 [P1] Closure return types not checked against method signatures → silent wrong results

```text
$ cat t83_filter_nonbool_rt.shape
let arr: Array<int> = [1, 2, 3]
let evens = arr.filter(|x| x + 1)    # predicate returns int, signature wants bool
print(f"{evens.length()}")
$ shape run t83_filter_nonbool_rt.shape
3                                     # everything kept — int treated as truthy at runtime
```

`filter`'s table signature is `fn(T) -> bool` but `check_function_expr_against`
(`bidirectional.rs:531+`) unifies params and *body* without hard-failing on the return mismatch
(the closure's `int` return flows into a soft path). The runtime then evaluates the int predicate
as truthy — i.e. the **no-truthiness rule is violated dynamically** after being enforced
statically everywhere else (t34-36). Wrong results, silent.

### 9.7 [P1] Tuples don't parse (t71-73)

```text
$ shape run t71_tuple_infer.shape    # let t = (1, "one")
Error: Parse error: Syntax error near: = (1, "one")
```

Both value and annotation forms fail. In-territory, `TypeAnnotation::Tuple` is handled by the
unifier, occurs check, and error rendering — consumer code for a producer that doesn't exist.
Parser territory owns the fix; type-system territory owns the dead arms.

### 9.8 [P1] Bare generic names accepted wherever no constraint touches them (t07/t16)

```text
$ shape run t07_bare_option.shape    # fn f(x: Option) -> int { 1 }   (never called)
ok
$ shape run t16_bare_hashmap_field.shape   # type T { m: HashMap }
ok
```

The 2026-05-31 ruling (`project_generic_types_require_args`) demands rejection "anywhere
(annotation/param/return/field)" at the type-resolution layer. Current enforcement is emergent:
the annotation only dies if unification compares it against an instantiated generic (t13-15).
Unconstrained positions pass. Fix belongs in annotation resolution (single choke point), not in
the solver.

### 9.9 [P2] Runtime-only enforcement for wrong-typed collection args (t74/t78/t82/t84)

`m.set(1, 2)` on `HashMap<string,int>` → runtime "HashMap key must be a string";
`a.concat(["x"])` on `Array<int>` → runtime "element type mismatch"; `a.includes("x")` →
silently `false`. All should be compile errors under the compile-time-proof doctrine; all are
consequences of the same soft-`Synth` design as §9.1 but with runtime guards or benign semantics
catching them.

### 9.10 [P2] `any` rejected only by accident of nominal lookup (t12/t17/t18)

`let x: any = 5` errors with "int is not compatible with any" — `any` is treated as an unknown
*nominal type reference*, not lexically banned. Consequently `fn f(x: Flurble) -> int {1}`
(unknown type, never called) **compiles** (t18), same as bare generics (§9.8). Undefined type
names in unconstrained annotation positions are not validated — `TypeError::UndefinedType`
(`errors.rs:26-27`) exists but this path never fires for them.

### 9.11 [P2] `1.5d` parses as Duration (days) → baffling diagnostic (t60)

```text
let d: decimal = 1.5d
→ error: Duration is not compatible with decimal    # decimal wants uppercase 1.5D
```

Grammar-documented (`shape.pest:1419` "Uses capital D to differentiate from 'd' (days)"), but the
diagnostic doesn't hint at the near-miss. One-line suggestion fix (suggestions.rs machinery
already exists for identifiers).

### 9.12 [P2] `Type::to_semantic()` collapses non-numeric TypeVars to `TypeVarId(0)` (core.rs:424-426, :458-459)

`var.0.trim_start_matches('T').parse::<u32>().unwrap_or(0)`: user-named params (`T` → `""` → 0,
`U`/`K`/`V` → parse fail → 0) all alias to `TypeVarId(0)` in the `SemanticType` projection. Any
consumer distinguishing two generic params via `SemanticType::TypeVar` sees them merged. Latent
until a SemanticType consumer relies on var identity across a multi-param signature.

### 9.13 [P2] Debug representation in user-facing diagnostics

Sites: `errors.rs:196` (unsolved-constraint rendering, t08/t13/t14/t15), `constraints.rs`
`"{:?} cannot have fields"`/`"{:?} does not have field"` arms (t57), and the compiler's
binary-op message (t86). Fix is a recursive `format_type` for `Generic`/args and reusing it in
the constraint-violation arms.

### 9.14 [P2] Occurs-check failure is silent (unifier.rs:54-56)

`bind` on a cyclic binding silently returns; the program later fails with a generic
"unknown"-typed diagnostic instead of "cannot construct infinite type" —
`TypeError::InfiniteType` (`errors.rs:38-39`) exists but this path cannot produce it. Honest
outcome, poor attribution.

---

## 10. What is done well

1. **The `NumericDomain` lossless-implicit lattice** (`constraints.rs:38-71`). One 30-line
   value-range abstraction implements the entire ruled conversion matrix (integer widths, float
   exact-integer ranges, direction rules) and matched every empirical probe (t02/03/88/89/90).
   This is how type rules should be built: one principled predicate, not a case table.
2. **The empty-array diagnostic** (t11) and the accumulator machinery behind it
   (`EmptyArrayAccumulatorKey`, push-driven element resolution): the error states what the
   compiler knows, why it can't proceed, and both remediations. Best diagnostic in the language.
3. **U1 canonicalization** (`core.rs:281-359`): the historical three-encodings-of-`Array<T>`
   split-brain (SB-4) was killed by a single canonical form + a single equivalence relation
   (`probe_equal`, `constraints.rs:249` — with an explicit doc distinguishing *equivalence* from
   *unifiability* and why `try_unify` was deleted for over-merging). Textbook cleanup of exactly
   the class of drift this repo fights.
4. **Value-restricted let-generalization** (`items.rs:857-978, :2327-2358`): HM generalization
   gated on a non-expansiveness scan, with the unsound counter-example
   ("int+string-through-one-slot") named in the comment. Verified working end-to-end (t09/t40).
5. **Occurs check through annotation-embedded markers** (`unifier.rs:213-268` + WF-6 regression
   test): the tyvar-marker hack (§3.1) at least carries its full safety obligation — the check
   descends into `Concrete` annotations, which a naive implementation would treat as var-free.
6. **Schema content-identity** (`registry.rs`): the recurring next_id-collision family
   (project memory) got its root fix — handles minted only via `intern_content`, structural
   dedup tested, serialization rebuilds the derived index. The dense-counter is demoted to an
   intern index rather than deleted-and-renamed — honest migration.
7. **Struct-literal diagnostics** (t47/t48): field-type mismatch carries a `note:` pointing at
   the declaration; missing field carries `help:`. Rich-diagnostic plumbing
   (`TypeErrorWithLocation`, error_bridge) pays off where it's wired.
8. **Per-engine `TypeVarGen`** (`core.rs:15-43`): type-var scoping moved off process statics with
   the rationale documented (cross-test ID collisions) — the same discipline later applied to the
   schema registry (task-local/thread-local/default layering in `type_schema/current.rs`).
9. **Trait machinery**: supertrait obligations enforced transitively
   (`get_transitive_supertrait_names`, `environment/mod.rs:1624`), `dyn Trait` works, comptime
   trait/impl alignment has dedicated error variants (`errors.rs:135-159`). The
   "method receivers are implicit" error (t44) is a model actionable diagnostic.
10. **0 ignored tests across 480** in-territory — no parked failures, unlike adjacent verticals.

---

## 11. What is done poorly / tech debt

1. **Soft-`Synth` as the default for method-arg checking** (`bidirectional.rs:483`,
   `expressions.rs:1508-1560`): "helps but doesn't force" is the opposite of a type *checker*.
   The legitimate flexibility cases named in the comment (`concat([])`, `includes(None)`) should
   be handled by variance/subtyping rules, not by disabling checking for every concrete expected
   type. This single design choice produced §9.1 (P0), §9.6, and §9.9.
2. **The three-layer pipeline with no shared fact store** (§5.1): narrowing and instanceof died
   here; the P0 partially lives here (layer 2 trusts annotations layer 1 didn't enforce). Every
   cross-layer feature costs double implementation.
3. **String-keyed `TypeVar` + `"\u{1}tyvar:"` marker encoding** (§3.1): pervasive allocation and
   string-parsing in the hottest inference paths, plus the `TypeVarId(0)` collapse (§9.12). A
   `u32`-keyed var with an interned display table would delete both.
4. **Dead subsystems kept warm**: evolution tracking (315 LOC, write-only, §3.4) *contradicts*
   the strict-typing doctrine in its own doc-comment ("variables can have fields added through
   assignment"); physical_binding (279 LOC, no consumers, unsound-API `unsafe` readers); tuple
   arms with no producer.
5. **Diagnostic rendering debt**: Debug leaks (§9.13), `Vec<>`-vs-`Array<>` vocabulary drift
   (errors.rs:211), wrong spans on constraint errors (t25 points at line 1), silent occurs-check
   (§9.14). The *content* of errors is often good; the rendering layer lags.
6. **`environment/mod.rs` monolith** (2,422 LOC): scope stack + builtin signatures + trait
   registry facade + enum registry + evolution + hoisted fields in one type. The
   `define_builtin_functions` hand-table (:1073) is a fourth parallel signature source (engine
   env, method_table, VM PHF registry, stdlib itself).
7. **Comment-to-code drift in load-bearing safety docs**: `bidirectional.rs` module doc claims
   Synth "emits the constraint" (it doesn't); `constraints.rs:29` claims path compression (none
   exists). When the doc describes the sound design and the code implements the unsound one,
   reviewers reading docs approve the wrong thing.
8. **CLAUDE.md/known-constraints staleness** (§6.3, §8): four wrong syntax claims and drifted
   line-cites in the canonical agent-facing doc actively mislead automation (measured: 6 wasted
   probe programs in this audit).

---

## 12. Prioritized recommendations

### P0 — do first

1. **Close the soft-`Synth` hole for value-position method args** (§9.1).
   Minimal safe fix: in the `expressions.rs` arg loop, emit a *hard* constraint for concrete
   expected types that are value positions (non-function, non-SelfType, non-collection-literal),
   keeping the documented exceptions as explicit carve-outs (`Option`-accepting `includes`,
   empty-literal `concat`/`zip`) — or better, express those as proper
   `T | subtype` signatures in the method table. Add the missing negative tests
   (`set` wrong V, wrong K; `get` wrong K) at both engine and shape-test level.
   Effort: 1-2 days + blast-radius diff run (per `feedback_subcluster_regression_scope`).
   Also add a runtime kind guard on HashMap *value* writes as defense-in-depth (the key guard
   already exists), so the class can't recur silently. Effort: hours.

### P1 — release-blocking coherence

2. **Option/Result exhaustiveness** (§9.3): extend `check_exhaustiveness` to
   `SemanticType::Option`/`Result` (two-variant closed enums — trivial variant sets). Effort:
   half a day incl. tests; message already exists (`NonExhaustiveMatch`).
3. **Decide narrowing's fate** (§9.2): either (a) plumb flow-scoped facts into the compiler
   (extend `InferenceFacts` with span-keyed narrowings and consume them in binary_ops/type
   tracking), or (b) delete the engine narrowing + fix CLAUDE.md/book to say "use `match`/`??`".
   Half-existing is the worst state: it pins dead behavior with green tests. (a) is ~1-2 weeks
   given the layer boundary; (b) is a day. Same decision gates `instanceof` (§9.5).
4. **Bare-generic and undefined-name validation at annotation resolution** (§9.8, §9.10): one
   choke point where every `TypeAnnotation` is resolved should reject bare `Option/Array/HashMap/
   Result/Set` and unknown type names, firing `UndefinedType`/GenericTypeError regardless of
   whether a constraint ever touches the position. Effort: 1-2 days + FP sweep over stdlib.
5. **Closure-return hard check** (§9.6): in `check_function_expr_against`, `Check` (not soft)
   the inferred body return against a concrete expected return (`bool` for filter/every/some).
   Effort: ~1 day; watch the documented comparator carve-outs (sort's `-> number`).
6. **bigint: pick a story** (§9.4): either wire literal suffix + `as bigint` (grammar +
   `NumericDomain` row + Into registration) or remove it from the book/CLAUDE.md and fix the
   overflow hint text. The hint text fix is minutes and should not wait for the feature.

### P2 — hygiene

7. Fix Debug-leak rendering (`format_type` recursive Generic arm + constraint-violation arms) —
   hours; kills the ugliest diagnostics in the language (t08/t13/t57/t86).
8. Delete or revive dead subsystems: evolution tracking (delete; write-only, doctrine-conflicting),
   physical_binding (delete or mark `unsafe fn` + find its owner), duplicated tests in
   `type_system/mod.rs`. Effort: hours each.
9. Replace `TypeVar(String)` with `TypeVar(u32)` + display table; fixes §9.12 mechanically.
   Effort: 2-3 days (wide but mechanical; the `"\u{1}tyvar:"` encoding survives unchanged or
   becomes a typed `TypeAnnotation::Var` variant while at it).
10. Refresh CLAUDE.md (§Language Features trait syntax, narrowing spelling, tuples claim) and
    §Known Constraints line-cites; refresh book pattern-matching/operators/integer-types claims
    (§8). Effort: an hour; prevents every future agent from re-burning the same probes.
11. Add a `1.5d`→`1.5D` near-miss hint (§9.11) and surface occurs-check failures as
    `InfiniteType` (§9.14). Effort: hours.

---

## Appendix A. Complete empirical test matrix

Every probe program run for this audit, in execution order. "compile-err" = rejected before
execution with a type/semantic error; "parse-err" = grammar rejection; "runtime-err" = compiled
then failed; "RUNS" = executed to completion. The ✓/✗ column judges the outcome against the
language's own documented rules (CLAUDE.md type-system rules + user rulings in project memory +
book claims).

| ID | Program essence | Outcome | Judged |
|----|-----------------|---------|--------|
| t01 | `let x: int = "hello"` | compile-err "string is not compatible with int" | ✓ strict |
| t02 | `let i: int = 5; let x: number = i` | compile-err | ✓ no implicit int→number for variables |
| t03 | `let x: number = 2` | RUNS, prints `2.0` | ✓ lossless literal adoption |
| t04 | `let n: number = 2.5; let i: int = n` | compile-err | ✓ number→int needs `as` |
| t05 | `2.9 as int` | RUNS, prints `2` (JIT deopts: `ConvertToInt` VM-only) | ✓ D2 truncation |
| t06 | `if 1 { }` | compile-err "int is not compatible with bool" | ✓ no truthiness |
| t07 | `fn f(x: Option) -> int {1}` (uncalled) | RUNS | ✗ bare generic accepted (§9.8) |
| t08 | `let x: Array = []` | compile-err, Debug-leak message | ✓/✗ rejected but leaks internals |
| t09 | `fn get_none() { None }` + `let x: Option<int> = get_none()` + match | RUNS "none" (JIT deopts: EnumPayload) | ✓ let-generalization |
| t10 | `if x != null` in fn with `int?` param | parse-err | ✗ documented spelling unparseable (§9.2) |
| t11 | `let x = []` | compile-err, model diagnostic | ✓ inference failure = error |
| t12 | `let x: any = 5` | compile-err | ✓ no any-type (but see t18) |
| t13 | t07 + actually calling `f(Some(1))` | compile-err, Debug-leak | ✓ rejected once constrained |
| t14 | `fn g() -> Option { None }` | compile-err | ✓ |
| t15 | `let x: Option = None` | compile-err | ✓ |
| t16 | `type T { m: HashMap }` (unused) | RUNS | ✗ bare generic field accepted (§9.8) |
| t17 | `let x: Flurble = 5` | compile-err "int is not compatible with Flurble" | ✓-ish (wrong reason: unknown name not flagged) |
| t18 | `fn f(x: Flurble) -> int {1}` (uncalled) | RUNS | ✗ unknown type accepted (§9.10) |
| t19 | t10 + else branch | parse-err | ✗ (§9.2) |
| t20 | `let x: int? = 41; if x != null` | parse-err | ✗ (§9.2) |
| t21 | `let x: int? = 41` | compile-err "int is not compatible with Option<int>" | ✓ by design (no auto-wrap); doc silence noted |
| t22 | f-string `{a + 1}` control | RUNS `2` | ✓ (isolates t20's failure to `null`) |
| t23 | `x: int?` + `x != null` | parse-err | ✗ (§9.2) |
| t24 | `Option<int>` + `x != None` compare only | RUNS "not none" | ✓ comparison itself types |
| t25 | t24 + use `x + 1` inside guard | compile-err "Option and int" | ✗ narrowing dead (§9.2) |
| t26 | same in fn + return | compile-err at use site | ✗ (§9.2) |
| t27 | `int?` + `Some(41)` + guard + use | compile-err | ✗ (§9.2) |
| t28 | no-guard `x + 1` on Option control | compile-err | ✓ control (guard adds nothing) |
| t29 | guard + `let y: int = x` | compile-err | ✗ narrowing dead for assignment too |
| t30 | `x ?? 0` | RUNS `42` | ✓ working Option-consumption path |
| t31 | `x.unwrap()` | compile-err "Method 'unwrap' not found" | ✓ (API doesn't exist — fine) |
| t32 | `int == number` | compile-err | ✓ |
| t33 | match arms `Color.Red` (dot syntax) | parse-err | — wrong syntax probe (real: `::`) |
| t34 | `while n` with `n: int` | compile-err | ✓ no truthiness |
| t35 | `if a && true` with `a: int` | compile-err | ✓ |
| t36 | `!a` with `a: int` | compile-err | ✓ |
| t37 | `i as number` | RUNS `5.0` | ✓ |
| t38 | `"42" as int` | compile-err "Cannot assert" | ✓ |
| t39 | `i64::MAX + 1` | runtime-err overflow, hint recommends `as bigint` | ✓ D3 / ✗ hint non-compiling (§9.4) |
| t40 | `fn id(x){x}` at int AND string | RUNS `5 hi` | ✓ implicit generics |
| t41 | user enum match missing variant | compile-err "missing variants Blue" | ✓ exhaustiveness |
| t42 | exhaustive user enum match | RUNS "blue" | ✓ |
| t43 | `match r { Ok(v) => ... }` no Err arm | RUNS `1` | ✗ Result exhaustiveness unchecked (§9.3) |
| t44 | trait `fn greet(self)` (CLAUDE.md syntax) | compile-err "receivers are implicit" | ✗ doc stale / ✓ good message |
| t45 | generic call, bound unsatisfied | compile-err "does not implement trait" | ✓ bounds enforced |
| t46 | `type Box<T>` + literal inference | RUNS `43` | ✓ generic types |
| t47 | struct literal wrong field type | compile-err + note | ✓ |
| t48 | struct literal missing field | compile-err + help | ✓ |
| t49 | t43 with actual `Err` value | **runtime crash** "No match arm matched" | ✗ (§9.3) |
| t50 | Option match, no None arm, x=None | **runtime crash** | ✗ (§9.3) |
| t51 | trait via `method` keyword + bound | RUNS "woof rex" | ✓ |
| t52 | `trait Pet extends Animal` | parse-err | ✗ CLAUDE.md syntax wrong |
| t53 | t52 negative control | parse-err | — |
| t54 | `trait Pet: Animal` both impls | RUNS | ✓ supertraits work (colon syntax) |
| t55 | `impl Pet` w/o `impl Animal` | compile-err "does not implement trait 'Animal'" | ✓ supertrait obligation |
| t56 | `filter`/`map` closures on `Array<int>` | RUNS `1 2` | ✓ bidirectional closure inference |
| t57 | `HashMap()` + `.insert` | compile-err "cannot have fields" Debug-leak | ✗ message quality |
| t58 | `.insert` direct | compile-err "Method 'insert' not found" | ✓ (method is `set`) |
| t59 | tuple in f-string | parse-err | ✗ tuples (§9.7) |
| t60 | `1.5d` as decimal | compile-err "Duration is not compatible with decimal" | ✗ near-miss hint absent (§9.11) |
| t61 | bigint `...n` literal | parse-err "number too large" | ✗ (§9.4) |
| t62 | decimal + number mix | compile-err | ✓ no cross-type unify (message confused by t60's `d`) |
| t63 | `Array<int> = ["a"]` | compile-err | ✓ |
| t64 | union param `int \| string` | RUNS | ✓ unions exist |
| t65 | `dyn Greet` binding + call | RUNS "woof rex" | ✓ trait objects |
| t66 | HashMap `set`/`get`/`??` happy path | RUNS `1` | ✓ |
| t67 | decimal `1.5D + 2.5D` | RUNS `4.0D` | ✓ |
| t68 | tuple `t.0` direct | parse-err | ✗ (§9.7) |
| t69 | `set("a","oops")` on `HashMap<string,int>` | **RUNS** | ✗✗ P0 (§9.1) |
| t70 | t69 + `get ?? 0` + `v + 1` | **RUNS, prints `106644639314833`** | ✗✗ P0 pointer reinterpretation |
| t71 | `let t = (1, "one")` | parse-err | ✗ (§9.7) |
| t72 | tuple type annotation | parse-err | ✗ (§9.7) |
| t73 | tuple + `.0` access | parse-err | ✗ (§9.7) |
| t74 | `set(1, 2)` wrong key | runtime-err key guard | ✗ should be compile-err (§9.9) |
| t75 | `push("x")` on `Array<int>` | compile-err, model diagnostic | ✓ (hard-coded push check) |
| t76 | t75 + read | compile-err | ✓ |
| t77 | `filter(\|x\| x + 1)` non-bool | **RUNS** | ✗ closure return unchecked (§9.6) |
| t78 | `get(42)` wrong key type | runtime-err | ✗ should be compile-err (§9.9) |
| t79 | correct `set`, `get` at wrong type | compile-err | ✓ return types enforced |
| t80 | direct `get` wrong annotation | compile-err | ✓ |
| t81 | unannotated map: `set` pins K/V, wrong read | compile-err | ✓ fresh-var path IS sound (contrast t69) |
| t82 | `concat(["x"])` on `Array<int>` | runtime-err guard | ✗ should be compile-err (§9.9) |
| t83 | t77 + `.length()` | RUNS `3` (all kept) | ✗ runtime truthiness (§9.6) |
| t84 | `includes("x")` on `Array<int>` | RUNS `false` | ✗ soft-accepted (§9.9) |
| t85 | `zip` heterogeneous | RUNS `2` | ✓ (legitimately generic) |
| t86 | `instanceof` narrowing on union | compile-err, Debug-leak | ✗ book feature dead (§9.5) |
| t87 | `cfg?.server?.port ?? 8080` | RUNS `8080` | ✓ optional chaining |
| t88 | `let x: i8 = 300` | compile-err | ✓ range-checked literal |
| t89 | `i8` → `int` implicit | RUNS `100` | ✓ lossless widening |
| t90 | `u64` → `int` implicit | compile-err | ✓ u64 ⊄ i64 |
| t91 | `5 as bigint` | compile-err "Cannot assert" | ✗ (§9.4) |
| t92 | `(big as bigint) + (1 as bigint)` | compile-err | ✗ (§9.4) |

Tally: 92 probes — 49 behaved per spec, 10 were syntax-discovery probes/controls, and 33 exposed
the defects catalogued in §9 (many probes per defect; 14 distinct findings).

### Appendix B. Grep-verified negative assertions

For auditability, the claims below rest on empty grep results over the working tree (in-territory
scope: `crates/shape-runtime/src/type_system`, `crates/shape-runtime/src/type_schema` unless
noted):

- No `#[ignore]` in territory (480 tests).
- No `from_heap_arc`, no `Box<HeapValue>` in territory (ADR-005/006 conformance, §6).
- No `ValueWord` / `tag_bits` / `SlotKind::Dynamic` / `exec_*_dynamic_fallback` symbol-family
  hits in territory (CLAUDE.md §Forbidden).
- `get_evolution` / `all_evolutions` / `EvolutionRegistry`: zero non-test callers workspace-wide
  outside `environment/` (dead-code claim, §3.4).
- `PhysicalSchemaBinding`: zero callers outside `type_schema/` (its own `mod.rs` re-export only)
  (§3.3).
- `unsafe`: exactly 5 hits in territory, all `type_schema/physical_binding.rs` (§3.3).
- `bigint` in `stdlib/`: only `stdlib/json.rs` (no constructor) (§9.4).
- `TypeDiagnosticMode`: only `Strict` and `RecoverAll` variants exist
  (`shape-vm/src/compiler/mod.rs:555-559`); default `Strict` asserted by unit test
  (`compiler_impl_initialization.rs:1169`).

---

*End of report — auditor 02 (type-system), 2026-07-11.*

