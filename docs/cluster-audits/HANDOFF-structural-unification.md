# HANDOFF — Shape v0.3.3 Structural Unification (L3 type-layer collapse + L5 carrier soundness)

**Status date:** 2026-06-24
**Branch:** `strict-flip-collection-dispatch` (merges to `main` at the v0.3.3 tag; "fix-then-flip")
**HEAD:** `31f1f867`
**Gates at HEAD:** `just check-clean` = 0 · `just check-no-dynamic` = 0 · `scripts/verify-merge.sh` = 13/13 PASS · engine span-table tests `u40 u42 u43pre` = 13/13
**Companion docs (read these too):**
- `docs/cluster-audits/STRUCTURAL-AUDIT.md` — the full split-brain catalog (SB-1..SB-26), the 6 roots, the U1–U7 roadmap.
- `docs/cluster-audits/U4-ROADMAP.md` — the L3-collapse deep-dive (the fallback engine, the 4 type representations, the side-table census, the wave decomposition, the regression corpus, the open questions).

---

## 0. One-paragraph orientation

This branch was stuck: ~30 "patch each gate finding" waves made the book-acceptance pass-count **oscillate** (4→9→14→9) instead of converge. The user re-scoped the work from *patching symptoms* to *finding and deleting the structural split-brains* — places where two or more independent sources of truth for the same fact drift apart. A layer-by-layer audit found **26 split-brains rooting in 6 causes**, and a **deletion-shaped** roadmap (U1–U7) to collapse each to a single source of truth. As of HEAD: the **type-system half is largely done** — U1 (one canonical `Type` + one equality), U3 (one HashMap carrier), and U4-0…U4-6a (the L3 collapse: the fallback re-derivation engine deleted, 3.5 of 4 parallel static-type representations deleted, the span-table is now the single L3 inference authority). A **Miri-driven L5 soundness pass** (R1–R6 below) also closed a carrier-convention Undefined-Behaviour class and installed a regression guard. **Net new test failures across every wave: 0** (each wave verified the FAILED-set byte-identical to its baseline). What remains is lower-value/harder: a few side-tables that need engine extensions, a VM/JIT kind-map decision (supervisor), and the JIT (deferred).

---

## 1. How the split-brains were identified (methodology — the part to internalize)

### 1.1 Definition
A **split-brain** = two or more *independent* sources of truth for the same piece of information, maintained by separate code paths, that can **drift** (disagree) about the same input. The codebase's core disease: *no single source of truth was ever established for a concept, so parallel ones were built and patched pairwise.* This is why patching oscillated — fixing one projection re-broke another.

### 1.2 The 6-layer model + isolation testing
The type pipeline was modeled as 6 layers and each was **exercised in isolation** — feed it an input, check whether its output agrees with what the adjacent layers assume. Where a layer carried its *own* answer that could disagree with another layer's answer for the same expression/binding/value, that's a split-brain.

| Layer | What it is | Crate / location |
|---|---|---|
| **L1** | Parser / AST | `shape-ast` (`shape.pest` → AST) |
| **L2** | Inference engine (the reference-model HM-style solver) | `shape-runtime/src/type_system/inference/` |
| **L3** | Compiler type tracker (what the bytecode emitter consults) | `shape-vm/src/compiler/` (`infer_expr_type`, the trackers, the side-tables) |
| **L4** | Static-Type → runtime-carrier selection | `prove_native_kind`, `ConcreteType`→`NativeKind`, carrier choice |
| **L5** | Carriers / VM | `shape-value` (`HeapValue`, `TypedObjectStorage`, `HashMapData`, `_new` vs `Arc` conventions) + `shape-vm` executor |
| **L6** | JIT | `shape-jit` (an independent second lowering engine) |

**All 6 layers failed isolation** — each held at least one internal duplicate source of truth.

### 1.3 Result: 26 split-brains → 6 roots
The audit cataloged 26 concrete split-brains (`SB-1`..`SB-26` in `STRUCTURAL-AUDIT.md`) and collapsed them to **6 root causes**. The crucial move was mapping each *recurring user-visible bug class* to a root, which proved the bugs weren't independent — they were symptoms of the same few roots:

| Root | The split-brain | Example SBs | Recurring bug class it causes | Fix |
|---|---|---|---|---|
| **R1** | No canonical `Type` encoding + no single equality relation (3 unify/equality procedures over ~5 `Array` encodings; type-vars hidden in 4 places) | SB-3, SB-5, SB-6 | identical-types-not-unifying, spurious union widening, empty/heterogeneous-array rejection, collection-dispatch FP | **U1** ✅ |
| **R2** | Type computation re-implemented across crates: a fallback re-derivation engine + 4 parallel static-type representations + ~16 hint side-tables | SB-1, SB-7, SB-8 | int/number-through-closures, field-read erasure (`rs[0].n`, `self.field`), inline-vs-let asymmetry, string-method/closure-call return loss | **U4** ◑ (core done) |
| **R3** | Static `Type` vs runtime `NativeKind` never unified; the `prove_native_kind` proof-gate was a no-op stub | SB-8b, SB-15 | compile-stamp-`Ptr` vs runtime-scalar (`closures_hof` SIGSEGV class), silent kind corruption | **U2** ◑ (check made real; **unwired** — separable) |
| **R4** | Dual HashMap runtime carrier, *producer-selected by inference luck* (annotate/not changed which runtime structure you got) | SB-9..SB-13, SB-18 | `HashMap<int,int>` failures, None/bool sentinel collision, element leaks, snapshot empties, O(n²) build loops | **U3** ✅ |
| **R5** | JIT is a second from-scratch engine that re-derives kinds; maintained but off | SB-19..SB-24 | VM≠JIT divergence, "every program falls through to interpreter" | **U5** ⏸ (deferred — JIT off, not blocking) |
| **R6** | No grammar↔AST↔diagnostics contract (grammar ⊊ AST; 3 diagnostic renderers; double-parse) | SB-25, SB-26 | parse-class FPs misread as type bugs (tuple destructure, `...rest`, turbofish) | **U6** ⏳ (not started) |

> **The single most important meta-finding:** the fix is *deletion*, not addition. Every root is "two parallel things exist; pick one and delete the other." The historical failure mode (documented at length in `CLAUDE.md §Forbidden Patterns`, the "W-series") is keeping the parallel path "as a fallback for one edge case," which converts a one-time deletion into permanent split-brain maintenance. **If the deletions don't actually land, the work is worthless.**

---

## 2. The unification roadmap (U1–U7) and status

Dependency order: **U1 → U2 → (U3, U4 parallel) → U6 independent → U5 last/optional.** U1 is the keystone — U2/U3/U4 all assume a trustworthy canonical `Type`.

| Step | Goal (all *deletion*-shaped) | Status |
|---|---|---|
| **U1** | ONE canonical `Type` encoding (`Type::Generic{base,args}`) + ONE equivalence relation (probe-mode `solve_constraint`); delete `try_unify` / standalone `types_equal` / the second substitution store | ✅ done (`9fb34e9a`, `7d0c44a7`) |
| **U2** | Make `prove_native_kind` a real static-`Type`↔`NativeKind` check | ◑ check is **real** (`abcc769f`) but has **zero production callers** — wiring it is separable, depends on U4-7 (below) |
| **U3** | ONE HashMap carrier (`HashMapData`); delete the `TypedMap` carrier + the `should_use_typed_map` selection switch; O(1) in-place set | ✅ done (`ffea6ade`, `ffb2481e`, `43dfaade`) |
| **U4** | Collapse L3 to ONE source of truth = the engine span-table; delete the fallback engine + the 4 type reps + the side-tables | ◑ **core done** (U4-0..U4-6a) — see §4; residual in §5 |
| **U5** | Make the JIT a derivation of the VM, or retire its dead half | ⏸ deferred (JIT off ⇒ not a correctness blocker today) |
| **U6** | Grammar↔AST↔diagnostics equivalence contract | ⏳ not started |

---

## 3. The destination architecture (what "done" looks like for L3/L4/L5)

**One inference answer per expression span. Everything else is derived or deleted.**

- **L3 single authority:** the engine's span-keyed table.
  - Engine side: `expr_type_table: HashMap<Span, Type>` (`shape-runtime/src/type_system/inference/mod.rs:268`), finalized by `finalize_expr_type_table` (`mod.rs:361` — drops any entry still carrying a free `Type::Variable`; **no `unknown`-default**), handed to the compiler via `take_expr_type_table` (`mod.rs:352`).
  - Compiler side: `resolved_expr_types: HashMap<Span, Type>` (`shape-vm/src/compiler/mod.rs:891`).
  - **Invariant (now enforced):** a span-table **MISS is a surface-and-stop compile error**, never a re-derivation. (`shape-vm/src/compiler/expressions/mod.rs` terminal arm of `infer_expr_type`.)
- **L4 single derivation:** `engine Type → ConcreteType → native_kind_from_concrete_type() → NativeKind`.
  - The one `ConcreteType→NativeKind` map: `shape-value/src/v2/closure_layout.rs:944`.
  - Numeric opcode index derived (not stored): `numeric_type_of` (`shape-vm/src/compiler/binary_ops.rs:386`) → `inferred_type_to_numeric` (`numeric_ops.rs:78`).
- **L5 single carrier discipline:** `HeapValue` is the canonical discriminator (ADR-005/006). The two `_new`/HeapHeader carriers (`TypedObjectStorage`, `TraitObjectStorage`) are addressed by raw `*mut`/HeapHeader refcount; everything else is `Arc<T>`. The carrier-convention is now **guarded** against regression (see §6).

---

## 4. What's been done (commit-by-commit)

### 4a. Type-system unification (U1–U3)
- `9fb34e9a` **U1** — one canonical `Type::Generic` + one probe-mode equivalence; deleted `Unifier::try_unify`, standalone `types_equal`, the engine-level second `unifier` field.
- `7d0c44a7` **U1b** — finished the deletions a skeptic caught (second unifier, Function-fold).
- `abcc769f` **U2** — `prove_native_kind` (`shape-vm/src/compiler/type_tracking.rs:1261`) became a real exact-equality check via the sealed `ProofGap` constructor. (Still unwired — see §5.)
- `28a4aa4a`,`ffea6ade` **U3** — DELETED the `TypedMap` carrier; routed all HashMap to `HashMapData` (HeapKind 17); deleted the `should_use_typed_map` selection switch (the split-brain *was* that switch).
- `7add28c1`→`1bc2a94a` (reverted)→`ffb2481e`→`43dfaade` **U3-perf** — HashMap.set made **O(1) in-place** by mirroring `Array.push`'s raw-pointer-view mechanism; `HashMapData.index` boxed so all three fields are raw-provenance. (The reverted `7add28c1` forged a `&mut` to Arc-shared data = UB; the redo is Miri-clean.)

### 4b. L5 carrier-convention soundness pass (Miri-driven; roots R3/R4 residue)
Miri (Rust's UB interpreter) was run over `shape-value` + the non-JIT `shape-vm` executor. It surfaced a **carrier-convention split-brain**: heap values are carried two ways — `Arc<T>` vs the v2-raw `_new`/HeapHeader convention — and several sites recovered a `_new` object with an `Arc` operation (`Arc::from_raw` / `Arc::increment_strong_count`), which does a `byte_sub(16)` into a non-existent `ArcInner` header = UB. Six roots fixed, all Miri-verified (Stacked + Tree Borrows, with negative controls):

- `32e9800a`+`38c018d9` **R1** — TypedObject field write went through `&TypedObjectStorage` which *froze* the allocation; rewrote the write path to raw `*mut` end-to-end. (Swept in R3 too: `op_set_prop`.)
- `35a6ca6f` **R2** — closure-capture read did an unconditional 8-byte read that overran a 4-byte tail slot; now reads the slot's kind-width and zero/sign-extends.
- `0425b998` **R4** — `op_length` `Arc::from_raw` on a `_new` TypedObject → transient raw `*const` read.
- `0a1c9072` **R5** — `op_get_prop` + exceptions `Arc::from_raw` → transient raw read; **carrier table established** (only `TypedObjectStorage` + `TraitObjectStorage` are `_new`; all others are genuine `Arc`).
- `23c1b1bf`+`18991310` **R6** — `Arc::increment_strong_count` on `_new` carriers → `v2_retain(HeapHeader)`; **mechanical regression guard** added to `check-no-dynamic` (fails the build if any `Arc::{from_raw,increment,decrement}` is applied to the two `_new` carriers — guard regex broadened to match any expression, not just the literal `bits`).

### 4c. L3 collapse (U4) — the deletion half of an already-built span-table
**Reframe:** the span-keyed `expr_type_table` *already existed and was consulted first* — but it had been bolted on top of the old split-brain "as a fallback." U4 is finishing the deletion.

- `76277dc0` **U4 roadmap** — `docs/cluster-audits/U4-ROADMAP.md`.
- `af8dac93` **U4-0** (feasibility gate) — made the engine **export** closure-body field-reads (root: a closure param `Emp` resolves to `Type::Concrete(Basic("Emp"))`, but field projection only handled the `Reference` carrier; `Basic` fell to a `HasField` arm that *tentatively accepted without binding* the field-result var → it stayed free → `finalize` dropped it). Fix: normalize struct-named `Basic(name)`→`Reference(name)` gated by the real struct/alias registry. Also projects `Borrow`→referent at record time. **Proved the deletion is feasible** (bounded inference fix, not a redesign).
- `b93ad146` **U4-1** — removed the `PropertyAccess` consult-exclusion + ladder arms #13/#14; field reads now served by the span-table; STAGE-F1 (`inference/access.rs`) is the sole field-read strictness gate.
- `cea0542c` **U4-2** — DELETED the closure-body mini-inferencer (`closures.rs`, ~411 lines — a *fourth* stringly type-engine whose missing `PropertyAccess` arm was the live `unknown`-typed-closure-field bug). Closure returns now come from the span-table.
- `6c1e84f4`→`87558c2d`→`3523953e` **U4-3 (keystone)** — *measured* the fallback's live MISS set first (found it masked ~88 programs via a recording-abort bug), *closed* that with **U4-3pre** resilient span-table recording (`87558c2d` — record trivially-typed children even when a sibling aborts; soundness-preserving for exhaustiveness/arm-join), then **DELETED the fallback re-derivation engine** (`3523953e`). A table MISS is now a loud compile error. The deletion *un-masked* a previously-hidden inference bug (`probe3`).
- `4cdf8e89` **U4-4** — DELETED the standalone `NumericType` register (a 2nd source for "is this int or number"); derived from the one `Type` at opcode-selection. Golden opcode parity proven **byte-identical** (independently captured).
- `ea0ad992`+`374bd26a` **U4-5/5b** — closed the `type_name: Option<String>` array string **round-trip** (`Array<T>`→`"Vec<T>"`→strip→reparse — the lossy path that caused element-type loss), DELETED `function_return_types: HashMap<String,String>` (→ structural `function_return_concrete_types: HashMap<String,ConcreteType>`), collapsed a duplicated reference-referent carrier to one structural carrier.
- `31f1f867` **U4-6a** — DELETED 2 genuinely-redundant per-span side-tables (`array_element_types`, `binding_object_element_fields`); re-pointed readers to the engine span-table / structural recursion.

**Verification discipline applied to every wave** (so the dev can trust the "0 regressions" claim): build the wave's baseline commit and HEAD, run `cargo test -p shape-vm --lib` + `-p shape-runtime --lib` on both, and **diff the FAILED-test-name sets** — any test that passed at baseline and fails at HEAD is a real regression (FP). Across U4-0…U4-6a these diffs were byte-identical (the strict-flip baseline reds stay red; nothing new broke). Each wave also ran a small program corpus through *both* binaries for runtime/opcode parity, and the soundness/Miri waves added negative controls.

---

## 5. STANDING ISSUES (pick-up list for the next dev)

### 5.1 U4-6 remaining side-tables — need *engine extensions*, not clean deletes
U4-6a's re-enumeration found the side-tables split into **two classes**:
- **Per-span projections** (= `resolved_expr_types`): redundant, deletable. (2 done.)
- **Per-*signature* / per-*capture* projections**: carry info the per-*span* engine table does **not** replicate. **5 of these remain:**
  - `inferred_param_concrete_types`, `inferred_param_object_fields`, `inferred_param_fn_param_types`, `inferred_return_object_fields` — project `inferred_types: HashMap<String, Type::Function>` (per-function *signature* inference, consumed-and-dropped). Neutering them regresses real behavior: the JIT typed-array param fast-path on `fn get(xs,i){xs[i]}`, and `box.field` on an unannotated object param. **To delete:** extend the engine to export per-signature param/return info structurally (so readers can query it), or formally re-disposition them as legitimate non-span derived caches (they are single-reader and have **not** been shown to drift).
  - `binding_collection_carrier_kinds` — carries the bare-ctor collection capture kind (`let mut m = HashMap()` captured in a closure). The engine currently returns `Err` here: `infer_expr_type(Identifier("m"))` trips the `HasField` constraint (it re-runs from an empty env → `UndefinedVariable`). **This is the "capture-carrier SIGSEGV class."** **To delete:** the carrier only needs the *heap discriminator* (HashMap/HashSet/Deque/PQ), so a `Type→ConcreteType→NativeKind` on the binding's ctor should suffice *without* field resolution — but the engine must expose the bare-ctor binding's resolved Type at capture-resolution time without tripping `HasField`. **Verify that claim before deleting.**

### 5.2 U4-6 Tier 2/3 (not started)
- Tier 2: `local_/module_binding_array_element_types`, `map_key_value_types` family, `function_return_schema_ids`, array-callable tables. (Delete the lockstep-upgrade hack in `helpers.rs`.)
- Tier 3 (load-bearing, delete LAST): `current_function_local_concrete_types` / `module_binding_concrete_types` + their **snapshot/restore plumbing** in `monomorphization/cache.rs` (highest blast radius); `inferred_param_type_hints`.

### 5.3 U4-7 — the two `ConcreteType→NativeKind` maps disagree (SUPERVISOR DECISION)
`shape-value/src/v2/closure_layout.rs:944` (VM, total) and `shape-vm/src/mir_compiler/types.rs:151` (JIT) **disagree on four arms**:

| ConcreteType | VM (closure_layout) | JIT (mir) |
|---|---|---|
| `Option(_)` | `Ptr(TypedObject)` | `Ptr(Option)` |
| `Result(_,_)` | `Ptr(TypedObject)` | `Ptr(Result)` |
| `Pointer(_)` | `Ptr(NativeView)` | `UInt64` |
| `Void` | **panics** | `None` |

Latent today (JIT is off), but a genuine VM/JIT correctness drift and the prerequisite for wiring `prove_native_kind` (U2). **Needs a canonical-answer ruling from whoever owns JIT lowering.** Not blocking U4-6.

### 5.4 `type_name` field residual (core split-brain closed; this is benign cleanup)
The *lossy* array round-trip is gone. `type_name: Option<String>` survives (~59–84 reads) in **string-keyed** roles that don't lose structure: a scalar-name→ConcreteType total map (`monomorphization/type_resolution.rs:2706/2744`), `is_array_type_name`/`is_temporal_type_name` predicates (`helpers.rs:4021/4029`), the matrix `Vec`/`Mat` kernel selector (`matrix_ops.rs` — governed by a separate **user ruling 2026-06-17**), and the schema-registry lookup key (`SchemaRegistry::get(&str)`). **Full field deletion is blocked on re-keying the string-keyed schema registry to a structural `NamedTypeId`/`SchemaId`, which depends on a not-yet-landed schema-aware layout registry.** Also `advanced.rs:314 first_generic_arg_of_baked_name` reparses `Result<…>`/`Option<…>` generic-arg strings (pre-existing, a different family). Lower priority.

### 5.5 `prove_native_kind` is real but unwired (U2 tail)
`type_tracking.rs:1261` is a correct exact-equality check; **all 8 callers are `#[cfg(test)]`**. The one production proof-gate that fires (`numeric_operand_proof_gap`) was deleted in U4-4; strictness now lives in the U4-3 table-miss surface-and-stop. Wiring `prove_native_kind` meaningfully requires U4-7's single `ConcreteType→NativeKind` map first. (Note: a doc comment at `helpers_binding.rs` still *claims* the return kind comes from `prove_native_kind` — it does not; fix when wiring.)

### 5.6 Other tracked follow-ups (lower priority)
- **HashSet insert + HashMap remove/merge still COW** (`Arc::make_mut`) → likely O(n²); apply the same raw-ptr in-place treatment U3 used for `set`.
- **`Option<HashMap>.get()`-chaining** blocked by a strict-flip type-checker limit ("Option<HashMap> cannot have fields") — a type-checker gap, not a memory bug.
- **fn-arg-move enforcement** (per the user's **value/move semantics** ruling): passing a collection to a fn currently *silently shares* it; under value/move it should *move* (post-call use = B0005, like `let a = m`). Uniform move-analysis fix for Array + HashMap. (See `project_v033_tag_readiness.md` for the full ruling.)
- **CLI vs engine non-exhaustiveness**: the `shape check` CLI path lets a non-exhaustive enum match pass that the *engine* detects (a diagnostic-surfacing gap; pre-existing).
- **`ws3_f3_error_context_unwrapped_type_propagates_to_binding`** red (a `!!`-unwrap span-table miss, pre-existing/U4-3-era).

### 5.7 The pre-existing strict-flip baseline failures (context, NOT caused by this work)
The branch carries a WIP strict-typing-flip failure set: **~250 `shape-vm --lib`** failures (channel_ops/decimal_ops/Json.keys parallel-exec + stdlib-caching cluster, plus the phase-2c ADR-006 deleted-`ValueWord` test stubs) and **5 `shape-runtime --lib`** failures (numeric-body-constraint / ok-union inference tests). These are the strict-flip FP-regression blast-radius the broader v0.3.3 effort is working through — **every structural wave above verified its FAILED set byte-identical to these**, i.e. introduced none of them. (Also: a benign `Json.keys NewTypedArrayString has no FrameDescriptor` verifier stderr line appears on trivial programs — pre-existing cosmetic noise, SB-22.)

---

## 6. Invariants & disciplines (so you don't reintroduce a split-brain)

1. **DELETION discipline.** When unifying, the parallel path must be *deleted*, not kept "as a fallback for one edge case." That rename is the documented historical failure (`CLAUDE.md §Forbidden Patterns`, the W-series). If a deletion surfaces a gap, fix it **at the surviving source** (the engine), or **surface it** — never retain the parallel path. Several waves here did exactly this (U4-1 imported-fields, U4-2 `Array.sum`, U4-3pre resilient recording).
2. **Mechanical guards** (run on every CI + pre-commit):
   - `scripts/check-no-dynamic.sh` + `docs/check-no-dynamic-baseline.txt` — per-symbol monotonic-non-increasing baseline. Includes the **R6 carrier-UB guard** (no `Arc::{from_raw,increment,decrement}` on `TypedObjectStorage`/`TraitObjectStorage`).
   - `scripts/verify-merge.sh` (`just verify-merge`) — 13 exit-code checks; required pre-merge.
3. **The no-FP-sweep** is the regression gate: build baseline + HEAD, diff the FAILED-test-name sets, FPs (passed→failed) must be 0. Don't trust a green-looking suite — the branch baseline is noisy; diff it.
4. **Adversarial verification.** Every wave here was implemented + then checked by ≥2 independent skeptics with a "try to break it / default to unsound" stance. It caught a forged `&mut` (UB), two *fabricated* Miri claims, and an incomplete fix. For soundness work: **actually install and run Miri** (`rustup run nightly cargo miri test -p shape-value --lib …`, both default Stacked Borrows and `-Zmiri-tree-borrows`); don't trust an asserted Miri pass.
5. **Machine safety** (this box has OOM'd): every Shape program run is memory+time capped (`ulimit -v 12582912; timeout 20 …`); justfile recipes carry `ulimit -v 50331648` (48 GiB). **Never run the full 613-test bulk** (the OOM trigger). Miri target dirs fill `/tmp` (9.5 GiB tmpfs) — use a per-run `CARGO_TARGET_DIR` and `rm -rf` it after.
6. **`int` ≠ `number`** — they never unify. No silent coercion; no `unknown`-the-type consumed as a real type (an `unknown` sentinel must behave as a table MISS → surface-and-stop).

---

## 7. Where to look / how to continue

- **The methodology + full SB catalog:** `docs/cluster-audits/STRUCTURAL-AUDIT.md`.
- **The L3-collapse plan + regression corpus + open questions:** `docs/cluster-audits/U4-ROADMAP.md` (§4 wave decomposition, §6 the `f8`/`h1`/`h2`/`h4b` red→green + stay-green/stay-error corpus, §7 risks).
- **The running state log** (decisions, rulings, per-wave outcomes): the team-lead memory at `~/.claude/projects/-home-dev-dev-shape-lang-shape/memory/project_v033_tag_readiness.md`.
- **Engine:** `shape-runtime/src/type_system/inference/` (`mod.rs` span-table + finalize; `expressions.rs` recording incl. the U4-3pre `infer_child_resilient`; `access.rs` field-access + STAGE-F1).
- **Compiler / L3:** `shape-vm/src/compiler/` (`mod.rs:891` `resolved_expr_types`; `expressions/mod.rs` `infer_expr_type` + the now-surface-and-stop terminal arm; `binary_ops.rs:386` `numeric_type_of`; the surviving side-tables on the tracker struct).
- **Carriers / L5:** `shape-value/src/heap_value.rs` (`TypedObjectStorage`, `HashMapData`), `v2/closure_layout.rs:944` (the `ConcreteType→NativeKind` map).
- **To continue U4-6:** start by re-enumerating the *current* side-table fields on the compiler tracker struct (the roadmap's lists are stale — U4-4/U4-5b deleted several), classify per-span vs per-signature/capture, and for each per-signature/capture table decide engine-extension vs re-disposition (§5.1).

---

*Prepared by the rotated-in team-lead for the v0.3.3 structural-unification effort. Every claim here is grounded in a committed wave with adversarial verification; cross-check against the commit messages (`git log` on `strict-flip-collection-dispatch`) and the two companion docs.*
