# Compilation pipeline index

Scope: parser, AST, type system, MIR, bytecode compiler, comptime/annotations.

## 1. Parser & AST

### Pest grammar

**Path**: `crates/shape-ast/src/shape.pest` (1,560 lines; rules `program`, `item`, `statement`, `expression` start at 15/24/510/915).
**Role**: Single Pest grammar file consumed by the `pest_derive` macro on `ShapeParser` to drive all parsing.
**Key rules / invariants**:
- `parse_program` runs `preprocessor::preprocess_semicolons(input)` before invoking `ShapeParser::parse`, so the grammar sees normalized line endings.
- `comptime_block` is grammar-level: a top-level `comptime { ... }` lowers to `Item::Comptime(stmts, span)` in `parser/mod.rs:203-214`.

**Related**: `pest parsing entry`, `AST root types`, `Span and Location`.

---

### AST root types (Item / Statement / Expression)

**Path**: `crates/shape-ast/src/ast/program.rs:30` (`Item`), `crates/shape-ast/src/ast/statements.rs:11` (`Statement`), `crates/shape-ast/src/ast/expressions.rs:33` (`Expr`).
**Role**: Three top-level sum types that together cover every Shape syntactic form; `Program { items, docs }` (`program.rs:23`) is the parser's output.
**Key rules / invariants**:
- Adding a new variant requires updating ~8 files (desugar, closure analysis, type inference, two visitors, two compilers, LSP) — the exhaustive matches force this (per CLAUDE.md).
- `Item::Function`, `Item::ForeignFunction`, and `Item::BuiltinFunctionDecl` are distinct: only the first carries a body.
- `VariableDecl` carries `is_mut: bool` and `OwnershipModifier` (`Inferred`/`Move`/`Clone`) — the binding form (`let`/`let mut`/`var`) is recovered from `VarKind` plus `is_mut`.

**Related**: `pest parsing entry`, `desugar pass`, `Span and Location`.

---

### Span and Location

**Path**: `crates/shape-ast/src/ast/span.rs:8` (`Span`), `crates/shape-ast/src/error/...` (`SourceLocation`), `crates/shape-ast/src/parser/mod.rs:13-28` (`pair_span` / `pair_location`).
**Role**: `Span` is a lightweight `(start, end)` byte offset on every AST node; `SourceLocation` is the heavy line/column/source-line variant attached to errors.
**Key rules / invariants**:
- `Span::DUMMY` (start=0, end=0) marks compiler-synthesized nodes; `is_dummy()` distinguishes them.
- `Span` is `Copy`; `SourceLocation` is allocation-heavy and built only at error time.

**Related**: `pest parsing entry`, `resilient parser`.

---

### pest parsing entry

**Path**: `crates/shape-ast/src/parser/mod.rs:57` (`parse_program`), `:108` (`parse_item`), `:272` (`parse_expression_str`).
**Role**: Public parser API. Returns a `Program` or a `ShapeError::ParseError` / `ShapeError::StructuredParse`.
**Key rules / invariants**:
- `parse_program` is the only entry that runs the semicolon preprocessor; `parse_expression_str` skips it (used for string-interpolation fragments).
- Failed `Rule::item_recovery` matches produce a structured error rather than silent skip.

**Related**: `Pest grammar`, `resilient parser`, `desugar pass`.

---

### Resilient parser

**Path**: `crates/shape-ast/src/parser/resilient.rs:14` (`PartialProgram`), `:73` (`parse_program_resilient`).
**Role**: LSP-friendly entry that returns a `PartialProgram` containing successfully-parsed items plus accumulated `ParseError`s, instead of bailing on the first error.
**Key rules / invariants**:
- Re-exported from crate root as `parse_program_resilient` / `PartialProgram` / `ParseError` / `ParseErrorKind`.

**Related**: `pest parsing entry`.

---

### Desugar pass

**Path**: `crates/shape-ast/src/transform/desugar.rs:13` (`desugar_program`), also `crates/shape-ast/src/transform/comptime_extends.rs`.
**Role**: AST→AST rewrite invoked at the start of `BytecodeCompiler::compile` (e.g. `FromQuery` → method chain).
**Key rules / invariants**:
- `desugar_program` mutates in place; the compiler clones the program first.
- `augment_program_with_generated_extends` is called separately for analyzer input.

**Related**: `bytecode compiler entry`.

---

### Pretty-printer

Not found — possibly: there is no AST→source unparser in this scope. `Display` impls on individual types like `MirConstant` exist for diagnostics, but no whole-program pretty-printer.

---

## 2. Type system

### Type / SemanticType

**Path**: `crates/shape-runtime/src/type_system/types/core.rs:57` (`Type`), `crates/shape-runtime/src/type_system/semantic.rs:21` (`SemanticType`).
**Role**: Two parallel type representations — `Type` is inference-engine internal (carries `Concrete(TypeAnnotation)`, `Variable`, `Generic`, `Constrained`, `Function` arms), `SemanticType` is the user-visible shape used by metadata/wire layers.
**Key rules / invariants**:
- `Type::Function { params, returns }` exists distinct from `Type::Concrete(TypeAnnotation::Function)` so unresolved variables can flow through call types.
- `Type::to_annotation()` for `Type::Function` lossily turns unresolved vars into `"unknown"` (CLAUDE.md "TypeVar loss" note); a regression test pins the non-lossy `BuiltinTypes::function()` case (`constraints.rs:1193`).
- `any` type was deleted; unannotated positions use `Type::Variable(TypeVar::fresh())`.

**Related**: `Type::Variable + TypeVar`, `BuiltinTypes`, `FieldType`.

---

### Type::Variable + TypeVar

**Path**: `crates/shape-runtime/src/type_system/types/core.rs:22` (`TypeVarGen`), `:47` (`TypeVar`), `:33` (`fresh_var`).
**Role**: Per-inference-engine generator (`TypeVarGen`) producing stable `TypeVar("T0")`, `TypeVar("T1")`, ... names for inference unknowns.
**Key rules / invariants**:
- IDs are scoped to a single `TypeInferenceEngine`, NOT process-global — prevents cross-test ID collisions.
- A `Type::Variable` reaching emission is a compile error: there is no dynamic fallback (CLAUDE.md "Forbidden Patterns").

**Related**: `Type / SemanticType`, `Type inference (bidirectional + flow-sensitive)`, `Unifier`.

---

### FieldType

**Path**: `crates/shape-runtime/src/type_schema/field_types.rs:35` (`FieldType`), `:205` (`FieldAnnotation`), `:212` (`FieldDef`).
**Role**: Storage-layer field discriminator (F64, I64, Bool, String, Timestamp, Array, Object, Decimal, Any, sized int variants) used by `TypeSchema` to compute byte offsets.
**Key rules / invariants**:
- `FieldType::Any` cannot project to a strict-typed `NativeKind` and yields `FieldKindError::AnyTypeNotStrictlyTyped`. The strict-typing plan calls for eliminating `Any`-typed fields.
- `semantic_to_field_type` is the bridge from `SemanticType` to storage layout.

**Related**: `Type / SemanticType`, `NativeKind`, `Schema and SchemaId`.

---

### NativeKind

**Path**: `crates/shape-value/src/native_kind.rs:32`.
**Role**: Single discriminator for typed 8-byte slots at every ABI exit (compile-time proof, marshal layer, wire/snapshot, JIT FFI). Variants for plain/nullable widths plus `Bool`, `String`, and `Ptr(HeapKind)`.
**Key rules / invariants**:
- `NativeKind::Dynamic` and `NativeKind::Unknown` are deleted — compile error if a slot's kind cannot be proven.
- Forbidden to add parametric variants (`Result(...)`, `Option(...)`, `JsonValue`); the strict-typed answer is `HeapKind::TypedObject` + a per-instantiation schema_id.

**Related**: `HeapKind`, `FieldType`, `prove_native_kind (kind tracker)`.

---

### HeapKind

**Path**: `crates/shape-value/src/heap_variants.rs:61` (`HeapKind`), `:100` (`HeapValue`), `:153` (`kind()`).
**Role**: u8-repr discriminant of `HeapValue` arms (String, TypedObject, Closure, Decimal, BigInt, DataTable, Future, TaskGroup, TypedArray, Temporal, TableView, Content, Instant, IoHandle, NativeScalar, NativeView, Char, HashMap).
**Key rules / invariants**:
- `HeapValue` is the single canonical heap discriminator (ADR-005). Layers above must take `Arc<HeapValue>` and dispatch on `HeapValue::kind()`; no parallel sum types.
- Both enums are generated by the `define_heap_types!` macro to keep `kind()` / `is_truthy()` / `type_name()` in lockstep.

**Related**: `NativeKind`, `Schema and SchemaId`.

---

### Schema and SchemaId

**Path**: `crates/shape-runtime/src/type_schema/schema.rs:26` (`TypeSchema`), `crates/shape-value/src/ids.rs:134` (`SchemaId`).
**Role**: Memory-layout description of a declared user type, with computed field offsets, optional enum info, and a content hash; `SchemaId(u32)` is the registry-allocated handle.
**Key rules / invariants**:
- `field_kind(idx)` projects each field to a `NativeKind`; returns `None` for `FieldType::Any`.
- Schema content hash is lazy-cached and skipped during serialization (`#[serde(skip)]`).

**Related**: `FieldType`, `StringId / FunctionId / SchemaId / StackSlotIdx`.

---

### Generic type parameters

**Path**: `crates/shape-ast/src/ast/types.rs:201` (`TypeParam`).
**Role**: AST-level enum carrying the two parameter shapes: `Type { name, trait_bounds, default_type }` and `Const { name, ty, default }` (const generics).
**Key rules / invariants**:
- Const-generic monomorphization is staged: B.3/B.4 not yet implemented; most passes treat `Const` as a stub.
- `Queryable<T>` generic impl headers parse but are erased to simple names downstream (CLAUDE.md "Known Constraints").

**Related**: `Trait system`, `Monomorphization`.

---

### Trait system

**Path**: `crates/shape-runtime/src/type_system/environment/registry.rs:74` (`TraitImplEntry`), `:94` (`BlanketImplEntry`), `:105` (`TypeRegistry`), `:217` (`register_trait_impl`).
**Role**: `TypeRegistry` holds `trait_impls: HashMap<String, TraitImplEntry>` plus blanket impls and aliases, queried by inference and the method table.
**Key rules / invariants**:
- Trait registration deduplicates: `register_trait_impl_with_assoc_types_named` checks an existing entry at line 329 before insertion.

**Related**: `Generic type parameters`, `Type inference (bidirectional + flow-sensitive)`.

---

### Type inference (bidirectional + flow-sensitive)

**Path**: `crates/shape-runtime/src/type_system/inference/mod.rs:56` (`TypeInferenceEngine`), `bidirectional.rs:34` (`CheckMode`), `:62` (`check_expr`), `inference/statements.rs:137-160` (flow narrowing).
**Role**: Hindley-Milner-style engine with three modes: `Infer`, `Check(T)` (hard), `Synth(T)` (soft hint, used to push closure param types).
**Key rules / invariants**:
- Closure parameter types in method calls (`arr.map(|x| ...)`) are inferred bidirectionally: the engine looks up the receiver's `GenericMethodSignature`, extracts the expected closure params, and passes them as `Synth` hints.
- `if x != null { ... }` narrows `T?` to `T` in the then-branch via `extract_narrowings`/`extract_inverse_narrowings`; the narrowed scheme is pushed for the branch only.
- `int` and `number` do NOT unify (CLAUDE.md "Type System Rules").

**Related**: `Unifier`, `ConstraintSolver`, `Type / SemanticType`, `Type::Variable + TypeVar`.

---

### Unifier

**Path**: `crates/shape-runtime/src/type_system/unification/unifier.rs:11`.
**Role**: Substitution-based unifier holding a `HashMap<TypeVar, Type>`; `bind` applies existing substitutions before adding the new one and rejects self-binds.
**Key rules / invariants**:
- Solver and unifier are separate components: `ConstraintSolver` (`type_system/constraints.rs:46`) collects constraints; the unifier resolves them.

**Related**: `Type inference (bidirectional + flow-sensitive)`, `Type::Variable + TypeVar`.

---

### BuiltinTypes

**Path**: `crates/shape-runtime/src/type_system/types/builtins.rs:11`.
**Role**: Constructor module returning canonical `Type` values for primitives (`number`, `int`, `string`, `bool`, `void`, `null`, `array`).
**Key rules / invariants**:
- `canonical_numeric_runtime_name` defines the alias mapping: `number/float/f64 → f64`, `int/integer/i64 → i64`, `byte → u8`, `char → i8`.
- `BuiltinTypes::any()` returns a fresh `Type::Variable` (no `Any` type).

**Related**: `Type / SemanticType`, `FieldType`.

---

## 3. MIR / lifetime / ownership

### MIR root types

**Path**: `crates/shape-vm/src/mir/types.rs` — `Place:72`, `Operand:175`, `MirConstant:199`, `BorrowKind:238`, `Rvalue:256`, `BinOp:274`, `MirStatement:312`, `Terminator:367`, `BasicBlock:401`, `MirFunction:411`.
**Role**: MIR is a CFG-based IR between AST and bytecode, used by the borrow solver, liveness, and the repair engine.
**Key rules / invariants**:
- `Place` distinguishes `Local`, `Field`, `Index` (with operand for diagnostics; conservatively treated as conflicting), `Deref`.
- `Operand::MoveExplicit` (source-level `move x`) must NOT be rewritten to a clone by the move/clone inference pass.
- `MirConstant::Str` carries the literal value through MIR; `StringId` is a legacy carrier.

**Related**: `MIR lowering pass`, `Place / Rvalue / Operand`, `BorrowKind`.

---

### MIR lowering pass

**Path**: `crates/shape-vm/src/mir/lowering/mod.rs:521` (`lower_function_detailed`), `:594` (`lower_function`); helpers in `lowering/expr.rs`, `lowering/stmt.rs`, `lowering/helpers.rs`.
**Role**: Translates AST function bodies into `MirFunction` (basic blocks with statements + terminators).
**Key rules / invariants**:
- `lower_function_detailed` returns rich result (mir + mutability errors); `lower_function` is the thin wrapper for callers that just want the MIR.

**Related**: `MIR root types`, `Borrow solver`.

---

### Borrow solver

**Path**: `crates/shape-vm/src/mir/solver.rs:65` (`BorrowFacts`), `:114` (`extract_facts`), `:944` (`solve`), `:1281` (`SolverResult`), `:1560` (`analyze`).
**Role**: Datafrog-based NLL borrow checker. Produces `BorrowAnalysis` (the single source of truth consumed by compiler, LSP, and diagnostics).
**Key rules / invariants**:
- Datafrog rules only add tuples (monotone fixed point); termination is guaranteed.
- Post-solve relaxation: `solve()` skips `ReferenceStoredIn*` errors when the container slot's `EscapeStatus` is `Local`.
- Interprocedural sendability: closures with mutable captures crossing detached task boundaries trigger `B0014`.

**Related**: `B0013 / B0014 errors`, `BindingStorageClass + BindingSemantics`, `EscapeStatus`.

---

### BindingStorageClass + BindingSemantics

**Path**: `crates/shape-vm/src/type_tracking.rs:286` (`BindingStorageClass`), `:299` (`BindingSemantics`).
**Role**: Per-slot ownership/storage metadata: `BindingStorageClass` (Deferred / Direct / UniqueHeap / SharedCow / Reference / LocalMutablePtr) plus the four-axis `BindingSemantics` (ownership_class, storage_class, aliasability, mutation_capability, escape_status).
**Key rules / invariants**:
- `LocalMutablePtr` marks a stack slot whose typed `*mut T` was handed to a non-escaping closure env (Closure Spec Phase D); the borrow checker has verified no outer code races it.
- ADR-006 plans extension by two more variants (`SharedAtomic`, `SharedAtomicMut`) for cross-task sharing — explicitly NOT a new modal-types subsystem.

**Related**: `Storage planning pass`, `EscapeStatus`, `Borrow solver`.

---

### Storage planning pass

**Path**: `crates/shape-vm/src/mir/storage_planning.rs:72` (`StoragePlan`), `:282` (`plan_storage`), `:140` (`collect_closure_captures`), `:1002` (`detect_escape_status`).
**Role**: After borrow analysis, decides each local slot's `BindingStorageClass` and records optimisation hints (`inline_array_sizes`, `non_escaping_closure_slots`).
**Key rules / invariants**:
- `INLINE_ARRAY_MAX_ELEMENTS = 8` (one cache line) — arrays larger than this are never tagged for inline storage even if non-escaping.
- The plan is read by `closures.rs:215` and `identifiers.rs:110` but `UniqueHeap` and `SharedCow` currently collapse to identical shared-cell boxing (per memory: "perf boundary, not soundness").

**Related**: `BindingStorageClass + BindingSemantics`, `EscapeStatus`.

---

### Liveness analysis

**Path**: `crates/shape-vm/src/mir/liveness.rs:14` (`LivenessResult`), `:61` (`compute_liveness`).
**Role**: Per-point live-set computation feeding move/clone inference (a value at its last use can be moved; otherwise must be cloned).
**Key rules / invariants**:
- Embedded in `BorrowAnalysis.liveness` — no consumer recomputes liveness independently.

**Related**: `Borrow solver`, `Place / Rvalue / Operand`.

---

### Field analysis

**Path**: `crates/shape-vm/src/mir/field_analysis.rs:26` (`FieldAnalysis`), `:45` (`FieldAnalysisInput`), `:51` (`analyze_fields`).
**Role**: Tracks granular field-level access and mutation, feeding disjoint-borrow conflict detection (`x.a` and `x.b` don't conflict).
**Key rules / invariants**:
- Field analysis does NOT widen index borrows (`x[i]` and `x[j]` always conflict — `Place::overlaps` line 132).

**Related**: `Borrow solver`, `Place / Rvalue / Operand`.

---

### Return ownership analysis

**Path**: `crates/shape-vm/src/mir/return_ownership.rs:71` (`infer_return_ownership_mode`); `ReturnOwnershipMode` enum re-exported via `crates/shape-vm/src/mir/analysis.rs`.
**Role**: Phase 5.B/5.C analysis: classifies each function's return as borrowed-from-param vs. uniquely-owned. Hint stored on `BindingSemantics::return_ownership_hint` so the call-site can skip an `Arc → Box` `PromoteToOwned`.
**Key rules / invariants**:
- Inconsistent return reference (different paths return different params/projections) → `BorrowErrorKind::InconsistentReferenceReturn` (B0007).

**Related**: `Borrow solver`, `Storage planning pass`.

---

### Ref escape analysis

**Path**: `crates/shape-vm/src/mir/storage_planning.rs:1002` (`detect_escape_status`); enforced in solver via `analysis.rs:95` and `helpers.rs:2082`.
**Role**: Computes `EscapeStatus::{Local, Captured, Escaped}` per slot and rejects refs flowing into closure environments or returns (contract-based for returns).
**Key rules / invariants**:
- Refs CAN sit in local containers (`solver.rs:1188`) but cannot escape into closures (memory-pinned semantic boundary).

**Related**: `EscapeStatus`, `Storage planning pass`, `B0013 / B0014 errors`.

---

### B0013 / B0014 errors

**Path**: `crates/shape-vm/src/mir/analysis.rs:151-156` (`BorrowErrorKind::CallSiteAliasConflict` / `NonSendableAcrossTaskBoundary`), `:184-187` (`BorrowErrorCode::B0013` / `B0014`).
**Role**: Documented borrow-error codes 0001-0014 spanning lexical and MIR-based checkers; B0013 = call-site alias conflict (one mutated, one read of same var), B0014 = non-sendable closure across detached task.
**Key rules / invariants**:
- Both lexical and MIR checkers share the same code space — diagnostics look identical regardless of source.

**Related**: `Borrow solver`, `Ref escape analysis`.

---

### Place / Rvalue / Operand

**Path**: `crates/shape-vm/src/mir/types.rs:72` / `:256` / `:175`.
**Role**: Place = something borrowable/assignable; Operand = how a value is supplied (Copy / Move / MoveExplicit / Constant); Rvalue = the right-hand side of an assignment (Use / Borrow / BinaryOp / UnaryOp / Aggregate / Clone).
**Key rules / invariants**:
- `Place::conflicts_with` returns true only when both share a root local AND projections overlap.

**Related**: `MIR root types`, `BorrowKind (Shared / Exclusive)`.

---

### BorrowKind (Shared / Exclusive)

**Path**: `crates/shape-vm/src/mir/types.rs:238`.
**Role**: Two-variant enum (`Shared`, `Exclusive`) attached to every loan; drives the conflict matrix in the solver.
**Key rules / invariants**:
- "Mutable" in source language = `Exclusive` in MIR; the AST `&mut x` lowers to `Rvalue::Borrow(Exclusive, place)`.

**Related**: `Borrow solver`, `Place / Rvalue / Operand`.

---

### Aliasability / MutationCapability / EscapeStatus

**Path**: `crates/shape-vm/src/type_tracking.rs:242` / `:252` / `:262`.
**Role**: Three small enums composing the binding-semantics triple. Aliasability ∈ {Unique, SharedImmutable, SharedMutable}; MutationCapability ∈ {Immutable, LocalMutable, SharedMutable}; EscapeStatus ∈ {Local, Captured, Escaped}.
**Key rules / invariants**:
- Initial state for `let mut x = 0` is `Unique` + `LocalMutable` + `Local` — a stack scalar, NOT `Arc<Mutex<int>>` (ADR-006 "refcount on escape, not on mutability").

**Related**: `BindingStorageClass + BindingSemantics`.

---

## 4. Bytecode compiler & blob format

### OpCode enum

**Path**: `crates/shape-vm/src/bytecode/opcode_defs.rs:40` (in `define_opcodes!` macro), categories at `:10`, instruction at `:1895`, builtin enum at `:2002`.
**Role**: `#[repr(u16)]` stack-machine opcode set, generated by the `define_opcodes!` macro that also derives `category()`, `stack_pops()`, `stack_pushes()`.
**Key rules / invariants**:
- Generic opcodes (`Add`, `Sub`, `Lt` without kind suffix) are deleted — only typed variants exist (`AddInt`, `MulNumber`, `EqInt`, ...).
- Variable-arity opcodes (`Call`, `CallMethod`, `NewArray`) declare 0/0 in the macro and rely on runtime arity.

**Related**: `Operand encoding`, `FunctionBlob and content_hash`, `Bytecode compiler entry`.

---

### Operand encoding

**Path**: `crates/shape-vm/src/bytecode/opcode_defs.rs:1902`.
**Role**: Single-operand-per-instruction enum carrying `Const(u16)`, `Local(u16)`, `ModuleBinding(u16)`, `Offset(i32)`, `Function(FunctionId)`, `Builtin(BuiltinFunction)`, `Count`, `Property`, etc.
**Key rules / invariants**:
- Each `Instruction` has at most one `Operand` (`opcode_defs.rs:1895`); multi-arg semantics are encoded via stack push/pop counts plus the single operand.

**Related**: `OpCode enum`, `String pool / StringId`, `StringId / FunctionId / SchemaId / StackSlotIdx`.

---

### FunctionBlob and content_hash

**Path**: `crates/shape-vm/src/bytecode/content_addressed.rs:33` (`FunctionBlob`), `:5` (`FunctionHash`), `:122` (`compute_hash`), `:166` (`Program`).
**Role**: Self-contained, content-addressed bytecode unit. Each blob owns its instructions, constants, strings, dependency hashes, foreign-fn deps, source map, and `required_permissions`.
**Key rules / invariants**:
- `compute_hash` serializes via MessagePack (`rmp_serde`) and SHA-256s the result; permissions are sorted by name first for deterministic hashing.
- Two functions with identical code but different permissions get DIFFERENT content hashes — permissions are baked into identity.

**Related**: `Linker`, `OpCode enum`, `Operand encoding`.

---

### Instruction stream

**Path**: `crates/shape-vm/src/bytecode/core_types.rs:240` (`BytecodeProgram`), `:395` (`Function`), `:621` (`Instruction` impl).
**Role**: After linking, `BytecodeProgram` holds a flat instruction array indexed by `Function`-recorded ranges; constants and strings live in shared pools.
**Key rules / invariants**:
- Pre-linking storage uses `FunctionBlob` (per-blob pools); post-linking uses `BytecodeProgram` (shared pools with remapped operand indices).

**Related**: `FunctionBlob and content_hash`, `Linker`.

---

### String pool / StringId

**Path**: `crates/shape-value/src/ids.rs:82` (`StringId`).
**Role**: `StringId(u32)` newtype indexing into `BytecodeProgram::strings`; opcodes carry `StringId` instead of heap strings.
**Key rules / invariants**:
- The reverse `String → u32` lookup is via `HashMap<String, u32>` for O(1) interning.
- Re-exported as `bytecode::StringId` (`opcode_defs.rs:6`).

**Related**: `Operand encoding`, `StringId / FunctionId / SchemaId / StackSlotIdx`.

---

### StringId / FunctionId / SchemaId / StackSlotIdx

**Path**: `crates/shape-value/src/ids.rs:11` (`FunctionId(u16)`), `:82` (`StringId(u32)`), `:134` (`SchemaId(u32)`), `:186` (`StackSlotIdx(usize)`).
**Role**: Strongly-typed newtype wrappers preventing accidental cross-domain misuse of raw integers.
**Key rules / invariants**:
- All four are `#[repr(transparent)]` (zero-cost) and provide `new`, `raw`, `index` const methods plus `From` impls.

**Related**: `String pool / StringId`, `FunctionBlob and content_hash`, `Schema and SchemaId`.

---

### Bytecode compiler entry (two-pass)

**Path**: `crates/shape-vm/src/compiler/mod.rs:501` (`BytecodeCompiler`), `crates/shape-vm/src/compiler/compiler_impl_reference_model.rs:1063` (`compile`), `:1556` (`compile_with_source`), `:1571` (`compile_with_graph`), `:1584` (`compile_with_graph_and_prelude`).
**Role**: Two-pass compiler: (1) collects all function definitions and module bindings, (2) compiles each item.
**Key rules / invariants**:
- `compile` clones the program, runs `desugar_program`, then `augment_program_with_generated_extends` for the analyzer.
- `stdlib_function_names: HashSet<String>` (line 1138) MUST be set whenever `prepend_prelude_items` is called — failing to set it breaks resolution.
- The compiler holds `closure_registry`, `closure_type_ids`, `closure_capture_kinds`, `function_type_registry` for v2 closure specialization (Phases A–F).

**Related**: `Linker (transitive permission union)`, `Monomorphization`.

---

### Monomorphization

**Path**: `crates/shape-vm/src/compiler/monomorphization/mod.rs` (overview), `type_resolution.rs`, `substitution.rs`, `cache.rs`.
**Role**: Generic-function specialization engine — `fn map<T, U>(...)` is compiled once per concrete instantiation. The `mono_key` cache (`"map::i64_string"`) maps to a function index in `BytecodeProgram::functions`.
**Key rules / invariants**:
- Looked up by `BytecodeCompiler::ensure_monomorphic_function` on every generic call site.
- Each specialization sees a fully `ConcreteType`-resolved AST so typed opcodes can be emitted throughout.

**Related**: `Bytecode compiler entry (two-pass)`, `Generic type parameters`.

---

### Linker (transitive permission union)

**Path**: `crates/shape-vm/src/linker.rs:281` (`link`), `:619` (`linked_to_bytecode_program`), error type at `:23`.
**Role**: Topologically sorts blobs by dependency edges, flattens per-blob pools (constants/strings/instructions) into the merged `LinkedProgram`, remapping operand indices.
**Key rules / invariants**:
- Computes the transitive union of all blobs' `required_permissions` at link time — final permissions = ⋃ per-blob perms.
- Constant pool overflow at >65535 entries is a `LinkError::ConstantPoolOverflow`.

**Related**: `FunctionBlob and content_hash`, `Bytecode compiler entry (two-pass)`.

---

### Bytecode verifier

**Path**: `crates/shape-vm/src/bytecode/verifier.rs:17` (`VerifyError`).
**Role**: Validates trusted/v2 typed opcode invariants — every trusted op must sit inside a function with a `FrameDescriptor`, slots referenced by typed ops must not be `NativeKind::Unknown`, field offsets must be < 4096.
**Key rules / invariants**:
- Verifier runs as a static check; failures are programmer errors, not user errors.

**Related**: `OpCode enum`, `NativeKind`.

---

## 5. Comptime / annotations

### comptime { } blocks

**Path**: AST item at `crates/shape-ast/src/ast/program.rs:77` (`Item::Comptime(Vec<Statement>, Span)`); parser at `crates/shape-ast/src/parser/mod.rs:203-214`.
**Role**: Top-level `comptime { stmts }` — executed during compilation; result discarded (side effects only).
**Key rules / invariants**:
- The grammar distinguishes top-level comptime blocks from comptime-functions and comptime annotation handlers.

**Related**: `Comptime evaluator`, `Comptime builtins`.

---

### Comptime evaluator

**Path**: `crates/shape-vm/src/compiler/comptime.rs:239` (`execute_comptime`), `:304` (`compile_and_execute_comptime_program`), `:413` (`execute_in_runtime`).
**Role**: A mini-VM that compiles and runs a synthetic `Program` at compile time, used for annotation comptime handlers and for evaluating comptime statement bodies.
**Key rules / invariants**:
- Returns a `ComptimeExecutionResult { value: ValueWord, directives }` — directives are queued via thread-local for the surrounding compilation pass to apply.
- `rebind_typed_object_bindings_to_bytecode_schemas` handles SchemaId rewriting between the comptime VM and the host compiler's registry.

**Related**: `comptime { } blocks`, `Comptime builtins`, `ComptimeTarget`.

---

### Comptime builtins

**Path**: `crates/shape-vm/src/compiler/comptime_builtins.rs` — `ComptimeDirective:23` and registered builtins (`implements`, `warning`, `error`, `build_config`).
**Role**: Functions only callable inside `comptime { }`. Builtins emit `ComptimeDirective`s through `push_comptime_directive` (thread-local) to be drained by the surrounding pass.
**Key rules / invariants**:
- `ComptimeDirective` variants include `SetParamType`, `SetParamValue`, `SetReturnType`, `ReplaceBody`, `ReplaceModule`, `Extend`, `RemoveTarget` — covering AST mutation, type concretization, and module rewriting.
- Builtins use `register_typed_function` to register their signatures.

**Related**: `Comptime evaluator`, `ComptimeTarget`.

---

### ComptimeTarget

**Path**: `crates/shape-vm/src/compiler/comptime_target.rs:45`.
**Role**: Compile-time target descriptor passed to `comptime pre/post` annotation handlers describing the annotated item (kind, name, fields, params, return_type, applied annotations, captures).
**Key rules / invariants**:
- `fields` is `Vec<(String, String, Vec<FieldAnnotation>)>` (3-tuple) — per-field annotations are surfaced via `to_nanboxed()` as `{name, args}` objects.
- `FieldAnnotation = (String, Vec<String>)` (name + stringified args) at `:40`.

**Related**: `Annotation registry`, `Comptime evaluator`.

---

### ConstantValue (typed comptime carrier)

**Path**: `crates/shape-vm/src/compiler/comptime_concrete.rs:90`.
**Role**: Phase 4d typed sum replacing the old `ComptimeValue { value: ValueWord, concrete: Option<ConcreteType> }` shape — every variant carries its `ConcreteType` by construction.
**Key rules / invariants**:
- `Opaque(ConcreteType, [u8; 8])` is a deliberate bridge variant for not-yet-migrated producers; its 8-byte payload matches `ValueWord`'s size for round-trip compatibility. New code must NOT introduce `Opaque` uses.
- The 4d migration is incomplete: `comptime.rs` itself still uses raw `ValueWord` internally; module is `#[allow(dead_code)]` until the wiring lands.

**Related**: `Comptime evaluator`, `ComptimeTarget`.

---

### Annotation registry

**Path**: `crates/shape-runtime/src/annotation_context.rs:50`.
**Role**: `AnnotationRegistry` stores `annotation X { ... }` definitions by name; `register` and `get` are the only public mutators.
**Key rules / invariants**:
- Annotations are NOT modeled as named exports/imports (CLAUDE.md "Known Constraints"). Only namespace import (`use std::core::remote`) inlines whole modules; bare `from pkg use { @ann }` is not in the grammar.

**Related**: `AnnotationContext`, `AnnotationDef AST`.

---

### AnnotationContext

**Path**: `crates/shape-runtime/src/annotation_context.rs:89` (`AnnotationContext`), `:182` (`AnnotationCache`), `:244` (`AnnotationState`), `:287` (`NamedRegistry`), `:343` (`EmittedEvent`), `:358` (`DataRangeState`).
**Role**: Domain-agnostic runtime primitives passed to annotation lifecycle handlers as `ctx`: cache, persistent state, named registries, emitted events, data-range manipulation.
**Key rules / invariants**:
- All annotation behaviour is defined in Shape stdlib, not Rust. The Rust side only provides primitives.
- Lifecycle hooks: `on_define`, `before`, `after`, `metadata`, `comptime pre`, `comptime post`.

**Related**: `Annotation registry`, `Annotation lifecycle emission`.

---

### AnnotationDef AST + handler types

**Path**: `crates/shape-ast/src/ast/functions.rs:226` (`AnnotationDef`), `:244` (`AnnotationHandlerType`), `:264` (`AnnotationTargetKind`), `:283` (`AnnotationHandler`), `:202` (`Annotation` use site).
**Role**: AST representation of `annotation X(params) { on_define(...) {...} ... }`.
**Key rules / invariants**:
- `AnnotationHandlerType::ComptimePre` and `ComptimePost` are distinct from runtime hooks — they emit directives to mutate the AST before/after type inference.
- `AnnotationTargetKind` covers `Function`, `Type`, `Module`, `Expression`, `Block`, `AwaitExpr`, `Binding`.

**Related**: `Annotation registry`, `Annotation lifecycle emission`.

---

### Annotation lifecycle emission

**Path**: `crates/shape-vm/src/compiler/functions_annotations.rs:17` (`emit_annotation_lifecycle_calls`), `:43` (for type), `:59` (for module), `:111` (handler call), `:303` (`execute_comptime_handlers`).
**Role**: Walks each annotated item and emits the bytecode that registers/runs each lifecycle handler at the right point (`on_define` at registration, `before/after` around invocation, `comptime pre/post` during compilation).
**Key rules / invariants**:
- `run_comptime_annotation_handlers_for_target` (`expressions/mod.rs:157`) is the comptime entry from expression contexts; statement contexts route through `statements.rs:845`/`:930`/`:3973`.

**Related**: `AnnotationDef AST + handler types`, `Comptime evaluator`.

---

### @ai annotation + LLM prompt expansion

**Path**: Defined in stdlib — no dedicated Rust handler in this scope. Per CLAUDE.md "AI-First Implementation": the `packages/ai/` directory is the intended home for the Shape-side definitions; the Rust side exposes only the generic annotation-lifecycle infrastructure (`AnnotationContext`, `AnnotationRegistry`, `ComptimeTarget`).
**Role**: An `@ai` annotation rewrites the function body to a typed LLM call (return type → JSON Schema for structured output).
**Key rules / invariants**:
- The annotation behaviour lives in stdlib, NOT in Rust — same architectural rule as all other annotations.
- `packages/ai/` directory is empty in the working tree at the time of indexing (only `packages/xgboost/` exists). The implementation may live elsewhere or not be checked in.

**Related**: `Annotation registry`, `AnnotationContext`, `ComptimeTarget`.

---
