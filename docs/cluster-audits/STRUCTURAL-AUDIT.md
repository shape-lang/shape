# Shape Structural Audit — Cross-Layer Split-Brain Catalog + Unification Roadmap

**Scope:** READ-ONLY structural research. Worktree `shape-strict-flip-collection-dispatch`, HEAD `bea569cc`.
**Method:** 6 per-layer first-principles audits (L1 parser/AST → L6 JIT-vs-VM), synthesised into a deduped cross-layer catalog.
**Definition of split-brain:** two-or-more *independent* sources of truth for the *same* information that can disagree/drift.

**Verified-at-HEAD load-bearing claims:**
- `prove_native_kind` (`crates/shape-vm/src/type_tracking.rs:1251-1257`) is a Phase-2 pass-through stub — `Ok(claimed_kind)`, no inspection. The headline mechanical enforcement is a no-op.
- `is_supported_builtin` (`crates/shape-jit/src/compiler/accessors.rs:661`) is `fn(_) -> true`. The parity matrix is optimistic/stale.
- Typed-map None-arms push `(NONE_BITS, NativeKind::Bool)` (`executor/v2_handlers/typed_map.rs:121,139,157,172,185,198`) — the deleted (0,Bool) null sentinel.
- Empty-array literal → `Type::Generic{Array}` (`inference/expressions.rs:475`); non-empty → `BuiltinTypes::array` = `Concrete(Array)` (`:516`). Dual array carrier confirmed.
- Dead emission twins `array_emission.rs`/`map_emission.rs`/`typed_map_emission.rs` exist with **zero** `*_emission::` call-sites; live path uses `v2_*` twins. Dual handler files `executor/v2_handlers/typed_map.rs` (live, `dispatch.rs:997`) vs `executor/typed_handlers/typed_map.rs` (dead).
- Five `has_*_residual` flags present on the bytecode program (`core_types.rs:626,666,709,721,736`).

---

## 1. Complete Split-Brain Catalog (deduped across layers)

Each entry: **concept** → the 2+ sources of truth, where each lives, how they drift.

### SB-1 — "What is the type of expression E" computed by THREE engines
- **Source A:** shape-runtime inference engine `TypeInferenceEngine` (`type_system/inference/`), run once at module scope.
- **Source B:** shape-vm bytecode compiler's reference-model engine `compiler_impl_reference_model.rs:814` (`infer_program_best_effort`, whole-program) → produces `resolved_expr_types` + ~13 hint tables.
- **Source C:** the empty fallback engine `self.type_inference` (`compiler/mod.rs:877`, created empty at `compiler_impl_initialization.rs:85`), consulted at `infer_expr_type` tail `expressions/mod.rs:2188`.
- **Plus** the error-reporting checker `analyze_program_with_mode` (`compiler_impl_reference_model.rs:2019`) — a third pass.
- **Drift:** C has no function-body bindings, so it returns `Variable`/unknown for the very locals A/B already solved. Three engines, one question, different answers. (L3 BRAIN-PAIR #1; L6 split #2 extends this with the JIT's own re-derivation.)

### SB-2 — Two unifier instances / two substitution stores
- **Source A:** `TypeInferenceEngine.unifier: Unifier` (`inference/mod.rs:66`).
- **Source B:** `ConstraintSolver.unifier: Unifier`, private (`constraints.rs:104`).
- **Reconciliation:** `self.unifier.merge(self.solver.unifier())` (`mod.rs:1803`), absent-key-only (`unifier.rs:186-192`) — same var bound differently silently keeps the engine's, drops the solver's. `finalize_expr_type_table` grounds through the engine unifier (`mod.rs:368`), not the post-solve solver store.
- **Drift:** post-merge gap grounds the keystone table from the wrong store. (L2 split #1.)

### SB-3 — THREE unify/equality procedures with different arm coverage
- **Hard mutating:** `ConstraintSolver::solve_constraint` (`constraints.rs:229`) — HAS cross-form Array arm (`:387`) + Function arm (`:353`) + Concrete-Generic arm (`:884`).
- **Soft read-only:** `Unifier::try_unify` (`unifier.rs:208`) — has NEITHER cross-form arm.
- **Structural equality:** `types_equal` (`structural_equality.rs:15`), used by match-arm join (`expressions.rs:1574`), union dedup (`mod.rs:1170/1421/1477`), as-cast identity (`expressions.rs:2988/3032`) — NO cross-form arm at all.
- **Drift:** the solver unifies `Generic{Array}` ~ `Concrete(Array)`; `types_equal`/`try_unify` see them as distinct. Whether `A==B` depends on which procedure asks. (L2 splits #2,#3.)

### SB-4 — Multiple encodings of `Array<T>` (the headline collection-dispatch split)
- **Enc 1:** `Type::Generic{base: Reference("Array"), args:[...]}` — var-preserving (empty-array literal, `expressions.rs:475`).
- **Enc 2:** `Type::Concrete(TypeAnnotation::Array(...))` — `BuiltinTypes::array` (`expressions.rs:516`, `builtins.rs:57-63`); routes element through `to_annotation()` collapsing `Type::Variable`→`Basic("unknown")` (the documented TypeVar loss, `builtins.rs:60-61`).
- **Enc 3:** `Type::Concrete(TypeAnnotation::Generic{name:"Array",...})` (`expressions.rs:2397`, `constraints.rs:884`).
- **Enc 4 (downstream):** tracker `type_name: Option<String>` `"int[]"`/`"Array<int>"` (`type_tracking.rs:338`); re-parsed by `.strip_suffix("[]")` (`expressions/mod.rs:1635`) and `Array<`/`Vec<` strip (`:2230`).
- **Enc 5 (runtime):** `ConcreteType::Array(Box<...>)` (shape-value v2).
- **Drift:** the solver patches encodings pairwise (`constraints.rs:353/387/884`); the other procedures never got those arms. (L2 split #4 + L3 BRAIN-PAIR #2.)

### SB-5 — Dual type *vocabulary*: `Type` vs `SemanticType`
- **Source A:** inference `Type` (`types/core.rs:92`), base name `"Array"`.
- **Source B:** user-facing `SemanticType` (`type_system/semantic`), base name `"Vec"`.
- **Bridge:** lossy `to_semantic`/`to_inference_type` (`core.rs:328/398`): `Array`↔`Vec` alias (`core.rs:351/418`); `Type::Variable`↔`SemanticType::TypeVar` by parsing the `Tnn` string (`core.rs:333`); `Type::Variable`→`SemanticType::Void` on function-return path (`core.rs:381`).
- **Drift:** two vocabularies must agree on every primitive name; the Void mapping silently discards unresolved vars. (L2 split #5.)

### SB-6 — Type-variable has FOUR hiding places
- `Type::Variable` (canonical); `Type::Constrained`; the `\u{1}tyvar:` magic string inside `Concrete(Object{...})` (`core.rs:73`, decoded only in `unifier.rs:97`); the parsed `Tnn` string in `SemanticType`.
- **Drift:** the three comparison procedures (SB-3) do not all decode all four; a var hidden in a string is invisible to `types_equal`. (L2 split #6.)

### SB-7 — FOUR static-type representations inside L3
- Engine `Type`/`TypeAnnotation` (structural, shape-runtime) vs tracker `type_name: Option<String>` (stringly, `type_tracking.rs:338`) vs `ConcreteType` (shape-value v2) vs `NumericType` (`type_tracking.rs:52`, per-last-expression mutable register, `mod.rs:726`).
- **Drift:** `infer_expr_type` spends ~26 arms converting between them; `binary_ops` reads `NumericType` for opcode selection while `infer_expr_type` reads the Type table — two sources for "is this operand int or number". (L3 BRAIN-PAIRs #2,#4.)

### SB-8 — ~13 hint side-tables, each a frozen projection of L2
- `current_function_local_concrete_types`/`module_binding_concrete_types` (`mod.rs:1711/1737`), `inferred_param_concrete_types`/`_type_hints`/`inferred_return_type_hints` (`mod.rs:1410/1398`), `local_callable_return_types`/`module_binding_callable_return_types`/`local_array_callable_return_types`/`module_binding_array_callable_return_types` (`mod.rs:771-788`), `array_element_types`/`local_array_element_types`/`module_binding_array_element_types` (`mod.rs:1263-1269`), `inferred_param_object_fields`/`inferred_return_object_fields`.
- **Drift:** each duplicates a slice of the engine solve, populated by separate projection fns, consulted in priority order — two tables can disagree about the same binding. (L3 BRAIN-PAIR #3.)

### SB-8b — `prove_native_kind` proof gate is a no-op stub
- **Concept:** "the claimed NativeKind is consistent with the proven static type."
- **Source of truth:** intended to be `prove_native_kind` (`type_tracking.rs:1251`); actual: pass-through `Ok(claimed_kind)`, ProofGap constructor never invoked.
- **Drift:** there is NO actual static-Type↔NativeKind consistency check anywhere; the mechanism that is supposed to *prevent* drift between L3 type and L4/L5 kind is theatrical. This is the keystone gap that lets all of SB-9..SB-12 ship. (L4 split #1.)

### SB-9 — DUAL HashMap runtime carrier (the most consequential split)
- **Carrier A:** v2 open-addressing `TypedMap<K,V>` (`shape-value/src/v2/typed_map.rs:40`, HeapHeader kind=82).
- **Carrier B:** insertion-ordered `HashMapData<V>` (`heap_value.rs:1199`) in `HashMapKindedRef` (`:1716`), HeapKind::HashMap ordinal 17.
- **Selection:** compile-time `should_use_typed_map(&k,&v)` (`v2_typed_map_emission.rs:155`) — routes (String,F64/I64/Ptr)+(I64,F64/I64/Ptr) to A, everything else (incl. untyped `HashMap()`) to B. Switch lives at `function_calls.rs:1572-1607`.
- **Two emission modules, two handler modules** (`v2_handlers/typed_map.rs` live vs `typed_handlers/typed_map.rs` dead), **two JIT paths**.
- **Drift:** the carrier chosen depends on whether L1–L4 *proved* K/V — the same source program lands in a different runtime structure based on upstream inference completeness. Adding/removing a type annotation changes runtime semantics. (L4 split #2; L5 split #1.)

### SB-10 — TypedMap stamps the WRONG kind on a heap pointer (carrier-kind lie)
- The TypedMap pointer is pushed as `NativeKind::UInt64` (`v2_handlers/typed_map.rs:77-102`), not `Ptr(HeapKind::HashMap)`.
- **Consequence chain:** `is_refcounted(UInt64)==false` (`native_kind.rs:374`) → `clone_with_kind`/`drop_with_kind` (`stack.rs:54,446`) treat it as a plain int (no retain/release). The HeapKind::HashMap arm of `clone_with_kind` (`stack.rs:123`) only understands Carrier B and would CORRUPT a TypedMap pointer if it saw one.
- **Second lifetime mechanism:** TypedMap lifetime owned by compiler-emitted scope-drop opcodes keyed off `v2_typed_map_locals` (`compiler/mod.rs:1205`, `statements.rs:6087`).
- **Drift:** lifetime authority is SPLIT — kind-track refcount for B, compiler-emitted opcodes for A. The §2.7.7 parallel-kind track (supposed single lifetime/serialization authority) is hollowed out for A. (L5 split #2.)

### SB-11 — TypedMap value-type collapse: static type → UInt64
- `should_use_typed_map`/`value_fits_ptr_slot` (`v2_typed_map_emission.rs:125-139`) collapse every heap-pointer V (string, Struct, Array, HashMap, Enum, Closure, BigInt, Decimal, DateTime) into ONE untyped `StringPtr`/`I64Ptr` carrier (`*const u8`, no value-kind field).
- Readback hardcodes `NativeKind::UInt64` (`v2_handlers/typed_map.rs:156,197`) regardless of static V type.
- **Drift:** compiler says `string`/`Array<T>`/`P`; carrier readback says `UInt64`. Irreversible — the static type is thrown away at the carrier. (L4 split #3.)

### SB-12 — None/Null sentinel: two encodings of "absence of value"
- **Carrier encoding:** `(NONE_BITS=0, NativeKind::Bool)` in both map get-paths (`v2_handlers/typed_map.rs:121…`, `typed_handlers/typed_map.rs:109-186`).
- **Stack-track encoding:** `NativeKind::Null` (post-R5b discipline; `native_kind.rs:112-153` documents (0,Bool)⇔false as the exact removed unsound collision).
- **Drift:** present-but-`false` map value is bit-indistinguishable from absent. The map carriers regressed to the forbidden sentinel. (L4 split #7; L5 split #3.)

### SB-13 — TypedMap element refcounts unmanaged (lifetime split inside one carrier)
- `TypedMap::drop_map` (`v2/typed_map.rs:125`) only `dealloc`s buckets — NO release over keys/values. Carrier B's `HashMapData::Drop` DOES release element TypedArrays via `HashMapValueElem`.
- **Drift:** element lifetime faithful in B, leaked in A, for the same `HashMap<string,string>`. (L5 split #4.)

### SB-14 — Callable value: two carriers (named-fn vs closure)
- **Named function value:** bits=fn-id, kind `NativeKind::UInt64` (`PushConst(Constant::Function)`).
- **Closure value:** `Arc::into_raw(Arc<HeapValue::ClosureRaw>)`, kind `Ptr(HeapKind::Closure)`.
- **Reconciliation:** `op_make_closure` (`control_flow/mod.rs:603-637`) materializes a real zero-capture closure when the layout stamps Ptr but the popped kind is UInt64 (the HEAD-`bea569cc` fix).
- **Drift:** two carriers for one "callable" concept reconciled at runtime, not at the producer. (L5 split #5.)

### SB-15 — Compile-time capture-kind stamp vs runtime capture-kind
- Closure layout `capture_native_kind(i)` (compile-time, `resolve_capture_concrete_type`) vs popped runtime kind (§2.7.7 track) — two sources for one capture's type. HEAD guard `control_flow/mod.rs:719-737` surface-and-stops when a transitively-captured unproven param is stamped `Ptr(HeapKind::NativeView)` but arrives as scalar `Int64` (would drop a small int as Arc → SIGSEGV).
- **Drift:** an L4 proof gap surfaced as an L5 runtime refusal. (L5 split #6; this is the subject of the HEAD commit.)

### SB-16 — Two `clone_with_kind`/`drop_with_kind` implementations
- `executor/vm_impl/stack.rs:54/446` (per-HeapKind stack dispatch) vs `shape-value/v2/closure_raw.rs:1522/1556` (closure capture read/write). Each a hand-maintained per-kind table that must stay in lockstep.
- **Drift:** divergence is a latent double-free/leak class. (L5 split #7.)

### SB-17 — FOUR parallel type discriminators at the L4 carrier boundary
- `StorageType` (`type_tracking.rs:88`, `native_kind_from_storage_type` returns None for Array/Object/Result/Function/Struct/Dynamic — 5 families have no kind), `NativeKind` (`native_kind.rs`), `ConcreteType` (`shape-value/v2/concrete_type.rs`), `Type`/`TypeAnnotation`. Four discriminators, four partial Type→kind functions, no canonical map. (L4 split #6.)

### SB-18 — Snapshot carrier coverage split
- `snapshot.rs:705` maps `HashMap` → `Ptr(HeapKind::HashMap)`; reconstruction (`snapshot.rs:1429`) imports HashMapData/HashMapKindedRef only. TypedMap (kind 82, pushed UInt64) has NO snapshot representation — serializes a bare pointer-as-integer; resume dereferences garbage.
- **Drift:** the two carriers disagree on whether they are serializable at all. (L5 split #8; couples to W17.)

### SB-19 — Dual lowering of one AST (VM bytecode vs JIT MIR)
- VM consumes bytecode (`compiler/`); JIT consumes MIR (`mir/lowering/{expr,stmt,helpers}.rs`) → Cranelift (`shape-jit/src/mir_compiler/`). Two independent translations of one source program; `mod.rs:14-19` advertises it as a feature.
- **Drift:** any construct lowered in one but not faithfully in the other diverges. (L6 split #1.)

### SB-20 — JIT re-derives slot kinds independently (extends SB-1 to a 4th brain)
- JIT `infer_slot_kinds`/`infer_slot_kinds_with_concrete`/`infer_top_level_concrete_types_from_mir` (`mir_compiler/types.rs`, refd `mir/lowering/expr.rs:483,581`).
- **Drift:** the c4-4B `?`-operator bug — bytecode tracker stamps Int64 (success type) while runtime bits are a heap Result pointer; JIT return-arm (`terminators.rs:1801-1813`) trusts the stamp. (L6 split #2.)

### SB-21 — Parity matrix vs actual lowering coverage
- `build_full_opcode_parity_matrix`/`build_full_builtin_parity_matrix` (`accessors.rs`) reports 201/201 + 184/184. But `is_supported_builtin` is `-> true` (`:661`), `vm_only_opcode_reason` (`:571`) only flags ops not in `ALL_OPCODES`. Optimistic stale source of truth contradicting real per-construct deopts in `mir_compiler`. (L6 split #3.)

### SB-22 — Two-faced V2 verifier
- `verify_v2_typed_opcodes` (`verifier.rs`) is a WARNING in the VM (`vm_impl/program.rs:88-118`, prints to stderr, never aborts) but a HARD whole-program-deopt GATE in the JIT (`executor.rs:252-270`). The stdlib prelude `Json.keys` emits `NewTypedArrayString` with no FrameDescriptor, so it fails on EVERY program → single-handedly disables the JIT universally. (L6 split #4.)

### SB-23 — Five `has_*_residual` divergence-catalog flags duplicated 3×
- `has_imported_const_inline` (`core_types.rs:626`), `has_w17_marshal_residual` (`:666`), `has_try_unwrap_residual` (`:709`), `has_reference_escape_promotion` (`:721`), `has_null_coalesce_residual` (`:736`); each gates a whole-program JIT deopt (`executor.rs:289,342,407,454,484`); duplicated into the content-addressed blob (`content_addressed.rs:324-615`).
- **Drift:** the divergence catalog itself is a split-brain kept in lockstep across two structs. (L6 split #5.)

### SB-24 — Dead in-VM tiered-JIT dispatch ABI
- §2.7.10/Q11 kinded value-call ABI deleted, never rebuilt. `control_flow/mod.rs:251-256` returns `NotImplemented(PHASE_2C_CALL_REBUILD_SURFACE)` when native code is installed; `tier_manager: None` (`vm_impl/init.rs:76`). Tier machinery (`tier.rs`, `deopt.rs`, feedback) records calls into a dead end. (L6 split #6.)

### SB-25 — Three parse-failure diagnostic renderers + double-parse (L1)
- Path A grammar `item_recovery` flat `"Syntax error near:"` (`parser/mod.rs:86-92`, `statements.rs:499`, `resilient.rs:137`); Path B `pest_converter.rs` rich `error[E0001]`; Path C resilient heuristics. PLUS double-parse: `module_resolution.rs:213` parses for import pre-resolution (`Warning: failed to parse…` `:216`) then the main path re-parses.
- **Drift:** same defect prints different diagnostics + reported twice; recomputed not threaded. (L1 splits #4,#5.)

### SB-26 — Grammar vs AST node-set (dead `Expr::Tuple`)
- AST declares `Expr::Tuple(Vec<Expr>)` (`expressions.rs:28`) + doc `/// Tuple type: [T1,T2,T3]` (`types.rs:15`); NO grammar production builds it; zero construction sites. The `(...)`/`[...]` delimiters overloaded across unit/grouping/decomposition/array/tuple-type with no rule owning "parenthesized comma-list". Dead `logos` lexer dep (`Cargo.toml:20`, zero hits).
- **Drift:** AST over-specified, grammar under-specified, no equivalence contract. (L1 splits #1,#2,#3,#6.)

---

## 2. Layer Health

| Layer | Faithful in isolation? | Divergence boundary |
|-------|------------------------|---------------------|
| **L1 parser/AST** | **FAIL** | Grammar reach ⊊ AST node-set (dead `Expr::Tuple`, no tuple/rest/turbofish productions). Three diagnostic renderers + double-parse. Boundary: L1→L2 — tuple/rest/turbofish programs never reach L2; read as parse noise, misclassified as type/inference bugs. |
| **L2 inference engine** | **FAIL** | Internally inconsistent: 3 equality procedures (SB-3) over 5 Array encodings (SB-4) with 2 substitution stores (SB-2). Faithful only on already-canonical inputs. Boundary: L2→L3 — `finalize_expr_type_table` drops free-var entries (dual-Array carrier prevents pinning); L3 surfaces "cannot infer". |
| **L3 compiler tracker** | **FAIL** | Re-implements L2 in a different crate with 4 representations (SB-7) + 13 side-tables (SB-8) + 3 engine instances (SB-1). The empty fallback engine (`mod.rs:877`) structurally returns unknown for function-body locals. Boundary: L3→L4 — static-type erasure on collection/method/field locals. |
| **L4 type→carrier** | **FAIL** | The proof gate is a stub (SB-8b). Carrier selection is a partial function (SB-9); selected carriers throw the static type away (SB-11). Boundary: L4→L5 — compiler says `string`, carrier readback says `UInt64`. |
| **L5 carriers/VM** | **FAIL** | Two HashMap carriers (SB-9) with incompatible lifetime models forcing a kind lie (SB-10); regressed None sentinel (SB-12); leaked elements (SB-13); two callable carriers (SB-14); two clone/drop impls (SB-16); snapshot gap (SB-18). Boundary: L5→L6 + L5→snapshot. |
| **L6 JIT vs VM** | **FAIL (effectively off)** | A third semantics engine (SB-19/20) that can only run safely with the JIT disabled. `Json.keys` verifier gate (SB-22) fires on 100% of programs → no program reaches native code. Boundary: VM↔JIT result equivalence is unmaintainable by construction. |

**No layer passes in isolation.** Every layer either re-computes a fact the prior layer already owned, or carries it in a second representation that the next layer's expectations don't match.

---

## 3. Root Causes (the few architectural roots under the many findings)

**R1 — No single canonical `Type` + no single equivalence relation (L1–L3).**
The same fact "these two types are the same" is computed by three non-equivalent procedures over a `Type` enum that admits multiple encodings of the same type, with no normalization step. Subsumes SB-2..SB-8. *The collection-dispatch strict-flip root lives here.*

**R2 — Type computation is re-implemented across crate boundaries instead of exported once (L3, L6).**
Because the engine runs once at module scope but emission descends into function bodies, each strict-typing gap was closed by adding a side-table or a fallback arm rather than making the engine authoritative. Three engine instances in shape-vm + the JIT's fourth re-derivation. Subsumes SB-1, SB-7, SB-8, SB-20.

**R3 — Static type vs runtime kind not unified, and the gate that would unify them is a no-op (L4, L5).**
`prove_native_kind` is a stub (SB-8b), so nothing forces the runtime NativeKind/carrier to be a faithful projection of the proven static type. Four parallel discriminators (SB-17), each with a partial Type→kind map. Subsumes SB-8b, SB-10, SB-11, SB-12, SB-17.

**R4 — Dual carriers for one concept, producer-selected by inference luck (L4, L5).**
HashMap (TypedMap vs HashMapData), callable (named-fn vs closure), and the dead non-v2 emission/handler twins. Selection keyed on whether upstream proved the type, so the same program lands in different runtime structures. Subsumes SB-9, SB-13, SB-14, SB-16, SB-18, and the dead twins.

**R5 — The JIT is a second from-scratch semantics engine, not a derivation of the VM (L6).**
Equivalence between two implementations of a 200-opcode/184-builtin surface cannot be maintained by construction, only chased; the codebase retreated to "deopt the whole program on suspicion", and one gate fires universally, so the engine is maintained-but-off. Subsumes SB-19, SB-21, SB-22, SB-23, SB-24.

**R6 — No grammar↔AST↔diagnostics equivalence contract (L1).**
AST and grammar are separately-evolved; parse failures rendered three ways and parsed twice. Subsumes SB-25, SB-26.

The deepest of these is the pair **R1 (no canonical Type/equivalence) + R3 (no enforced Type↔kind projection)**: R1 is why types disagree *within* the static pipeline; R3 is why the static type and the runtime carrier disagree *across* the boundary. R2/R4/R5 are all consequences of "no single source of truth was ever established, so people built parallel ones."

---

## 4. Recurring Gate-Class → Root Split-Brain Mapping

| Recurring gate-class | Root split-brain | Layer |
|----------------------|------------------|-------|
| **int/number through closures** (`f(5)+f(7)`, `arr[0](1)+arr[1](1)`, `d.hour()+1` inline-vs-let) | SB-1 (empty fallback engine returns unknown for body locals) + SB-8 (`local_callable_return_types` side-tables duplicate return types the engine doesn't project) + SB-7 (`NumericType` vs Type table). Root **R2**. | L3 |
| **hashmap-Int64 readback** (`m.get('a')` → raw pointer integer) | SB-11 (StringPtrGet stamps UInt64) + SB-10 (kind lie). Root **R3**/**R4**. | L4/L5 |
| **nested-struct-array SIGABRT / use-after-free** (struct-in-collection) | SB-10 (StringPtrSet stores `*const u8` without retaining refcount, value freed under it, reread as UInt64) + SB-13 (drop_map leaks/mismanages elements). Root **R3**/**R4**. | L5 |
| **V3-S5 ckpt-empties + SIGABRT** (typed-array/typed-map snapshot restore lands empty/dangling) | SB-18 (TypedMap has no snapshot arm) + SB-10 (UInt64 pointer-as-integer serialized). Root **R4**. | L5/snapshot |
| **constraint-solve-edges incl. "identical types not unifying"** (`(Vec<int>)->int != (Vec<int>)->int`) | SB-3 (three procedures, different arms) + SB-4 (multiple Array/Function encodings) + SB-2 (absent-key-only merge). Root **R1**. | L2 |
| **collection-dispatch strict-flip FP class** (annotation flips which branch fires) | SB-4 (empty vs non-empty array carrier) + SB-9 (`should_use_typed_map` switch). Root **R1**+**R4**. | L2/L4 |
| **None/Some readback corruption + bool-collision** (present-false vs absent) | SB-12 (`(0,Bool)` sentinel regression). Root **R3**. | L4/L5 |
| **closures_hof transitive-capture SIGSEGV** (the HEAD commit's subject) | SB-15 (compile-stamp Ptr vs runtime scalar Int64). Root **R3** (L4 proof gap surfaced at L5). | L5 |
| **VM≠JIT divergence** (`?`-returns-pointer-bits, imported-const garbage, as-cast unconverted, comptime-fires-twice, Drop-body-elided) | SB-19/SB-20 (dual lowering + JIT re-derives kinds) + SB-23 (residual flags). Root **R2**+**R5**. | L6 |
| **"every program falls through to interpreter"** | SB-22 (`Json.keys` verifier gate, two-faced). Root **R5**. | L6 |
| **PARSE-class FPs misread as type bugs** (`let (a,b)=pair`, `array_destructuring_rest`, turbofish, `(int,int)` types) | SB-26 (grammar⊊AST, missing productions) + SB-25 (3 renderers + double-parse noise). Root **R6**. | L1 |
| **spurious union widening / over-broad inferred types** | SB-3 (`unique_types` dedup uses `types_equal` which can't see two encodings are equal). Root **R1**. | L2 |
| **field-read erasure** (`rs[0].n`, `self.field`, `f(...).field`) | SB-1 (PropertyAccess excluded from keystone) + SB-7 (`concrete_type_for_expr` re-derivation). Root **R2**. | L3 |

---

## 5. Unification Roadmap (ordered, one source of truth per concern)

Ordered by leverage and dependency. "Tractable-v0.3.3" = scoped, mechanically testable in isolation; "Larger refactor" = cross-crate, multi-wave.

### U1 — ONE canonical `Type` encoding + ONE equivalence relation (root R1)
- **Unify:** pick `Type::Generic{base,args}` for ALL parametric collections; delete the `Concrete(Array)`/`Concrete(Generic)` synthesis paths; add `Type::canonicalize()` run on every type entering a comparison/constraint. Fix `BuiltinTypes::array` to stop routing through `to_annotation()` (which loses `Variable`→unknown, `builtins.rs:60`) so empty and non-empty array literals produce the SAME carrier. Delete `Unifier::try_unify` + standalone `types_equal`; derive a single relation from `solve_constraint` run in non-committing probe mode (clone substitution, attempt, discard). Remove the engine-level `unifier` field; read substitutions only from `self.solver` (kills the absent-key merge, `mod.rs:1803`).
- **Clears:** constraint-solve-edges / identical-types-not-unifying, collection-dispatch FP class, spurious union widening, empty/heterogeneous-array rejections. **~5 recurring classes + the strict-flip branch root.**
- **Blast radius:** shape-runtime `inference/` + `unification/` + `constraints.rs` + `structural_equality.rs` callers (`expressions.rs`, `statements.rs`, `mod.rs`). Contained to one crate.
- **Verdict: TRACTABLE-v0.3.3.** Highest leverage. Regression gate: one unit test asserting `solve_constraint`, probe-`try_unify`, and the equality relation all agree on the 4 Array + 2 Function encodings.

### U2 — Turn `prove_native_kind` from stub into the real check (root R3)
- **Unify:** consult the slot-kind tracker / `ConcreteType` at `site`; reject a claimed kind that doesn't round-trip to the static type. Makes the headline mechanical-enforcement actually enforce.
- **Clears:** by itself clears nothing (it's a gate) but it *prevents* SB-10/SB-11/SB-12/SB-15 from shipping — converts them from silent corruption to compile-time refusals, which is what surfaces them for fixing.
- **Blast radius:** `type_tracking.rs` + every `prove_native_kind` caller (will start failing where they currently lie). Expect a wave of surfaced violations.
- **Verdict: TRACTABLE-v0.3.3 to flip on; the surfaced-violation tail is larger.** Prerequisite for trusting U3/U4.

### U3 — ONE HashMap carrier + kind-track as its single lifetime authority (root R4)
- **Unify:** pick exactly one carrier (HashMapData already integrates with the kind track, has a real element-releasing Drop, and a snapshot arm — or fully promote TypedMap and delete HashMapData). Delete the `should_use_typed_map` switch (`function_calls.rs:1572`) — *that switch IS the split-brain*. Push the survivor with `Ptr(HeapKind::HashMap)` (kill the UInt64 lie, SB-10); retire `v2_typed_map_locals` compiler-emitted drops. Fix both None-arms to `NativeKind::Null` (SB-12). Survivor's Drop releases K/V element shares (SB-13). Delete the dead twins `array_emission.rs`/`map_emission.rs`/`typed_map_emission.rs` + `executor/typed_handlers/typed_map.rs`. TypedMap open-addressing speed (if measured to matter) becomes an internal storage detail behind the SAME HeapKind + SAME slot kind — never a second ordinal (82) or second slot kind (UInt64).
- **Clears:** hashmap-Int64, nested-struct-array SIGABRT, None/bool-collision, TypedMap element leaks, V3-S5 map snapshot empties. **~5 recurring classes.**
- **Blast radius:** shape-value `typed_map.rs`/`heap_value.rs`, shape-vm emission + both handler dirs, snapshot.rs, JIT map path. Multi-file but single-concept.
- **Verdict: TRACTABLE-v0.3.3** (it is mostly *deletion* — pick one, delete the other). **Acceptance test: adding/removing a type annotation on a HashMap must not change which runtime structure, None-encoding, or refcount discipline is used.**

### U4 — Collapse L3 to ONE source of truth = the engine's span-keyed table (root R2)
- **Unify:** DELETE the empty fallback engine `self.type_inference.infer_expr` (`expressions/mod.rs:2188`) — a span-table MISS becomes a loud surface-and-stop, not a re-derivation. Make the reference-model engine export EVERY expression (record PropertyAccess/field-reads; move the STAGE-F1 strictness ruling INTO the engine as a real error). Replace tracker `type_name: Option<String>` with the engine `Type`/`ConcreteType` so the `.strip_suffix("[]")` (`:1635`) and `Array<`/`Vec<` re-parse (`:2230`) disappear. Keep `NumericType`/`NativeKind` only as the emit-time stamp DERIVED from that one Type. Retire the ~13 hint side-tables as the engine table becomes complete. The 26-arm ladder collapses to: span-table → ConcreteType projection → surface-and-stop.
- **Clears:** int/number-through-closures, field-read erasure, inline-vs-let asymmetry, string-method return loss, closure-call return loss. **~6 recurring classes.**
- **Blast radius:** shape-vm `compiler/` (the keystone consumer + every side-table populator). Depends on U1 (the engine must be trustworthy first).
- **Verdict: Larger refactor** but high leverage; deletion-shaped, not addition-shaped. The CLAUDE.md anti-pattern warning applies: the ladder must be *deleted*, not kept "as fallback."

### U5 — Make the JIT a derivation of the VM, or retire its dead half (root R5)
- **Unify:** JIT lowers from the SAME type-stamped bytecode + parallel-kind track the VM executes — not an independent MIR with its own `infer_slot_kinds` (kills SB-19/SB-20 by construction). Delete the five `has_*_residual` flags + the parity matrix's `is_supported_builtin => true` allow-list; regenerate `jit parity` from actual lowering coverage. Fix the prelude `Json.keys` FrameDescriptor so the verifier stops being a universal off-switch; demote the verifier to ONE consequence in both engines. Either rebuild the §2.7.10/Q11 kinded value-call ABI so `tier_manager` can dispatch, OR delete `tier.rs`/`deopt.rs`/feedback-as-JIT-driver wholesale (SB-24).
- **Clears:** all VM≠JIT divergence classes, "every program falls through to interpreter." But the JIT is *already off*, so these are not blocking correctness today.
- **Blast radius:** entire shape-jit crate + the executor's residual-flag deopt gates.
- **Verdict: Larger refactor, NOT v0.3.3.** Lowest urgency (JIT off = correctness via VM only). The one cheap v0.3.3 win here is fixing `Json.keys`'s FrameDescriptor + silencing the universal verifier-warning stderr noise.

### U6 — Grammar↔AST↔diagnostics equivalence contract (root R6)
- **Unify:** ONE `paren_group` production (arity-based: 0→Unit, 1→inner, ≥2→`Expr::Tuple`) filling the dead node + removing the `(...)`/`[...]` overload; tuple TYPE `(T1,T2)` matching value syntax; add `..`/`...rest` to `destructure_array_pattern` + `turbofish` arm. Add a build-time conformance check (grep-sentinel like `no_dynamic.rs`) that every AST variant has ≥1 parser construction site. Route ALL parse failures through `pest_converter` E0001; delete the flat "Syntax error near" previews. Parse ONCE (thread the `Program` from import pre-resolution into main compile, `module_resolution.rs:213`). Drop unused `logos`.
- **Clears:** PARSE-class FPs misread as type bugs (tuple destructuring, array-rest, turbofish), duplicated diagnostic noise.
- **Blast radius:** shape-ast grammar + parser + error rendering; one consumer change in module_resolution. Contained.
- **Verdict: TRACTABLE-v0.3.3** (grammar additions + diagnostic consolidation are low-risk and self-contained); reclassifies several "residual" failures out of the inference bucket.

**Dependency order:** U1 → U2 → (U3, U4 in parallel) → U6 independent → U5 last/optional. U1 is the keystone: U4 depends on a trustworthy engine, and U2's enforcement is only meaningful once U1 makes the static type canonical.

---

## 6. Headline

**Yes — this codebase is fixable by unifying the split-brains, and the work is predominantly DELETION, not addition.** Every recurring gate-class maps to one of six architectural roots, and those roots are all the same disease: *no single source of truth was ever established for a concept, so parallel ones were built and patched pairwise.* That is a tractable problem precisely because the fix is to pick one representation and delete the others — the dangerous direction (adding more reconciliation arms / fallbacks / side-tables) is exactly the W-series walk-back CLAUDE.md already forbids.

**Single highest-leverage structural fix: U1 — establish ONE canonical `Type` encoding and ONE equivalence relation in the inference engine (root R1).** It is the most leveraged because (a) it is the named root of the strict-flip `collection-dispatch` branch this worktree exists to fix; (b) it clears ~5 recurring classes directly (identical-types-not-unifying, union widening, empty/heterogeneous-array, collection-dispatch FP, as-cast drift); (c) it is the *prerequisite* for U4 (L3 can only trust an engine that is internally consistent) and for U2 (the proof gate can only check against a canonical static type); and (d) it is contained to a single crate (shape-runtime) with a crisp, isolated regression gate (one test asserting all three former equality procedures agree on the 4 Array + 2 Function encodings). Fixing U1 first turns the engine into the trustworthy single authority that U2/U3/U4 all assume — without it, every downstream unification is building on sand.

**The one caveat that determines feasibility:** U1 and U4 must be done as *deletions* (remove `Concrete(Array)` synthesis, remove `try_unify`/`types_equal`, remove the fallback engine, remove the side-tables). The historical failure mode in this codebase — documented at length in CLAUDE.md §Forbidden Patterns — is keeping the parallel path "as a fallback for one edge case," which converts a one-time deletion into permanent split-brain maintenance. The roadmap is feasible for v0.3.3 *iff* the deletions actually land.
