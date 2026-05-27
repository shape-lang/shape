# v0.3.3 fix-dispatch cluster #13 — SCOPE-RECLAIM 761-entry root-cause partition

**HEAD at audit:** `82f049dd` (post-v0.3.2 tag).
**Audit closed:** 2026-05-27.
**Type:** META audit — consolidates the 761 SCOPE-RECLAIM entries from the
per-binary `v0.3-classification/` docs into a v0.3.3 fix-dispatch partition.
Audit-only. No source changes. No commits. No `git stash`.

**Standing user disposition (verbatim 2026-05-27 (B)):** *"everything needs
to work, v0.3.3 is the target. we are talking about a programming language.
correctness is key."* All 761 SCOPE-RECLAIM entries are release-blocking.
No family below may be re-dispositioned to v0.4 without a NEW dated user
authorization naming the family. Defection-attractor framings refused on
sight per CLAUDE.md §Forbidden-rationalizations.

## Source-of-truth roll-up

Inputs consulted: `TAXONOMY.md` (dated pull-ins), `SCOPE-RECLAIM.md` (high-
level partition), `TRUTH-SET.md` §"SCOPE-RECLAIM root-cause families", and
the per-binary docs `arrays_vectors.md`, `iterators.md`, `hashmap.md`,
`stdlib_json.md`, `stdlib_regex.md`, `stdlib_modules.md`, `comptime.md`,
`structs_types.md`, `pattern_matching.md`, `objects.md`, `objects_arrays.md`,
`strings_formatting.md`, `lsp.md`, `variables_bindings.md`,
`annotations_runtime.md`, `annotations_comptime.md`, `annotation_targets.md`,
`closures_hof.md`, `complex_integration.md`, `drop_raii.md`,
`borrow_refs.md`. Close-summary §0.A (criteria A/B/C/E/J) anchors the
architectural targets.

---

## Family 1 — V3-S5 ckpt-5/ckpt-6 op_new_array construction-cascade

**Approx test count:** ~340 (per TRUTH-SET; the dominant family).

**Per-binary breakdown (largest contributors):**

| Binary | Count | Local SURFACE shape |
|---|---:|---|
| iterators | ~95 (of 120) | Shape-A empty-`[]` + Shape-B `op_new_array(N)` + Shape-D `joinStr` per-V2ElemType |
| arrays_vectors | 39 (Shape-B) + 38 (Shape-A) of 101 | `op_new_array(N)` / empty `[]` un-resolvable |
| structs_types | ~40 | `op_new_array` + `range` + `MakeFieldRef base must reference TypedObject` cascading through TypedObject construction |
| closures_hof | ~25 (of 37) | `op_new_array` in HOF return-builder territory |
| annotations_runtime | 23 | `op_new_array` + annotation-registration cascade |
| annotations_comptime | 23 | `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3 SURFACE` |
| annotation_targets | 16 | `op_new_array` + `nb_object_array` |
| comptime | ~13 (Group SR-2) | `op_new_array(N)` annotation tests |
| pattern_matching | 3 (Group B) | rest-destructure → `op_new_array` |
| stdlib_crypto | 4 | `op_new_array` through `keypair_generate` return Array<u8> |
| borrow_refs | 20 | `SetIndexRef` ckpt-5 |
| drop_raii | 3 | `range()` builtin cascade |
| jit | 3 | tier-2 TypedArrayData rebuild |
| snapshots_resume | 1 | `range()` cascade |
| ranges | 1 | `range()` builtin |
| security_permissions | 1 | (V3-S5) |
| trait_system | ~6 | V3-S5 |
| objects | 1 | object_bracket_access dynamic-key |
| objects_arrays | ~7 | `op_new_array(N)` + nested-array fixtures |
| variables_bindings | ~3 | V3-S5 |
| complex_integration | ~14 | closure-param infer-loss from cascade |
| list_comprehension | 1 | comprehension carrier element-kind |

**Verbatim SURFACE shape (representative):**

```
Runtime error: Not implemented: op_new_array(N): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. The deleted typed-array-data enum +
`Buf<T>` / aligned-typed-buf wrapper layer + outer
`HeapValue::TypedArray(Arc<_>)` arm + `HeapKind::TypedArray=8` ordinal
DELETED across V3-S5 ckpt-1..ckpt-4 per W12-typed-array-data-deletion
audit §3.5 + §3.6 + ADR-006 §2.7.24 Q25.A SUPERSEDED. Construction-site
rebuild lands at ckpt-6 STRICT close after ckpt-5-prime. REFUSED ON
SIGHT: TypedArrayData resurrection under any rename (Refusal #1).
```

**Architectural target.** Per-T v2-raw `TypedArray<T>` flat-struct
monomorphization (W12 audit §A.3 / §3.1 scalar recipe / §2.2 heap-element
recipe). ADR-006 §2.7.24 Q25.A SUPERSEDED. Landing site: **ckpt-6 STRICT
close** per the W12-typed-array-data-deletion audit. Pulled into v0.3 by
the 2026-05-18 user disposition naming this work verbatim
(close-summary §0.A criterion A territory; J.5b/c/d/f already merged
adjacent slices). NO ADR amendment moves this to v0.4.

**Minimal repro.**
```shape
let xs = [1, 2, 3]
print(xs)
```
Fires `op_new_array(3)` at runtime.

**Size estimate:** XL — largest single family by test count (~45% of SCOPE-
RECLAIM); the construction-site rebuild is the architectural keystone the
rest of the cascade depends on. Per close-summary §0.A criterion A "5.5/6
COMPLETE" status, J.5e iterator-protocol audit deferred to v0.4 but ckpt-6
STRICT close itself is in-scope.

**Sequencing.** **First.** Every downstream family that ingests
`Array<T>` (Family 2 consumer-cascade, Family 3 W17.3-4 container-marshal,
Family 4 destructuring of array literals, Family 5 HashMap.keys/values
returning arrays) requires constructed `TypedArray<T>` carriers to exist
before it can be sensibly rebuilt.

**Cross-cluster overlaps.** This family is the upstream cause of much of
cluster #2 (closures HOF infer-loss), cluster #3 (objects_arrays / structs
construction), and partial cluster #4 (string.split → Array<string>).
Merge candidate: combine with Family 2 below into a single ckpt-5 → ckpt-6
super-track.

---

## Family 2 — V3-S5 ckpt-2/ckpt-3 consumer-cascade (filter/map/range/String.iter/String.split)

**Approx test count:** ~180.

**Per-binary breakdown:**

| Binary | Count | Local SURFACE shape |
|---|---:|---|
| iterators | 16 (Shape-C) + 2 (Shape-D) + 1 (Shape-E) | `filter`/`map`/`flatMap`/`except`/`intersect`/`union`/`unique` + `joinStr` + `op_iter_done iter_kind=Bool` |
| arrays_vectors | included in Family-1 totals (cascade source) | `filter`/`map` ckpt-2 |
| closures_hof | ~12 | `map`/`filter`/`flatMap` ckpt-2 in closure positions |
| strings_formatting | 12+ | `String.split: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3` (split returns Array<string>) |
| complex_integration | ~14 | closure-param infer-loss downstream of ckpt-2 |
| trait_system | partial | iter-protocol consumers |
| stdlib_regex | 5 | `match_all` returning Array<object> via ckpt-2 typed-array-data dep |
| pattern_matching | 1 (Group C) | `range()` ckpt-3 |
| structs_types | partial | `range()` in for-loops cascading through TypedObject ctors |

**Verbatim SURFACE shape:**

```
Runtime error: Not implemented: filter: SURFACE — V3-S5 ckpt-2 consumer-
cascade tier 1 surface. `TypedArrayData` enum DELETED at ckpt-1
(2026-05-15) per W12-typed-array-data-deletion audit §3.5 + ADR-006
§2.7.24 Q25.A SUPERSEDED. ... UNREACHABLE until ckpt-6 STRICT close.
```

**Architectural target.** Same Q25.A SUPERSEDED rebuild as Family 1, but
on the consumer side: per-T `TypedArray<T>::filter / map / flatMap / range`
flat-struct kernels. Lands as part of the **ckpt-6 STRICT close**; ckpt-5-
prime is the prerequisite construction primitive.

**Minimal repro.**
```shape
[1, 2, 3].filter(|x| x > 1)
```
Fires `filter: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1`.

**Size estimate:** L — second-largest family by test count.

**Sequencing.** **After Family 1.** Consumers can't be rebuilt against an
absent producer carrier. Within Family 2, range is a leaf and should land
adjacent to ckpt-3; String.split → Array<string> needs string-element
support landed (overlaps 2026-05-21 "Array<string> must work" pull-in).

**Cross-cluster overlaps.** Merge-candidate with Family 1 into one
construction-then-consumer dispatch wave.

---

## Family 3 — W17.3-4 per-container FieldType + W17-marshal-return-arms

**Approx test count:** ~110.

**Per-binary breakdown:**

| Binary | Count | Local SURFACE shape |
|---|---:|---|
| stdlib_json | 14 | `project_typed_return: W17-snapshot-roundtrip surface — TypedReturn::Discriminant(3 or 4)` |
| stdlib_modules | 11 (Group B) + 11 (Group A) | `Discriminant(N)` arm + `module namespace 'set' is not typed` |
| stdlib_regex | 5 | `Discriminant(7)/(8)` arms |
| stdlib_crypto | 4 | `Discriminant(1)` arm |
| structs_types | ~6 | W17.3-4 anon-object construction + WrapTypeAnnotation |
| complex_integration | ~14 | W17.3-4 + HashMap mix |
| variables_bindings | ~5 | W17.3-4 destructure |
| borrow_refs | 9 | `SetLocalIndex` W17-typed-carrier-monomorphization |
| snapshots_resume | 1 | phase-2c marshal `range()` cascade |

**Verbatim SURFACE shape:**

```
Runtime error: Not implemented: project_typed_return: W17-snapshot-roundtrip
surface — TypedReturn::Discriminant(N) container arm needs the per-arm
KindedSlot projection path (typed-Arc ResultData/OptionData/TypedObjectStorage
builders). Tracked as W17-marshal-return-arms follow-up. ADR-006 §2.7.4.
```

**Architectural target.** Per-arm `KindedSlot` projection over typed-Arc
`ResultData` / `OptionData` / `TypedObjectStorage` builders in
`project_typed_return`. **ADR-006 §2.7.4** host-tier marshal/snapshot
canonical 5-arm typed-pointer receiver-recovery pattern (per the
`b4d76858` W17-marshal-return-arms merge for resume; this family completes
the parse / stringify / module-export-typed return arms). Close-summary
§0.A criterion C "8/8 + 2 RESIDUALS MERGED" — the residual surfaces here
are the call sites where new container Discriminants were added by
W17.3-4 W16.2-J merges and the projection table wasn't updated.

Also includes Group A `set` module schema typing — per-container FieldType
work for module-namespace exports per the 2026-05-22 pull-in
(close-summary §0.A criterion B closed for Array<int>/Set<string>/
HashMap<string,int> but module-namespace path is the residual).

**Minimal repro.**
```shape
import std::json
let result = json::parse("{\"a\":1}")
```
Fires `project_typed_return: W17-snapshot-roundtrip surface — TypedReturn::Discriminant(3)`.

**Size estimate:** M.

**Sequencing.** **Parallel-with Family 1 / 2** (different code territory:
ADR-006 §2.7.4 marshal-projection table, not TypedArray construction).
Some overlap with Family 5 HashMap (per-V monomorphization Q25.B).
W17-marshal-return-arms for non-snapshot (parse/stringify) call sites can
land without waiting for ckpt-6.

**Cross-cluster overlaps.** Cluster overlap with cluster #4 (silent-wrong-
output marshal mismatches in stdlib_json roundtrip) — share the same
projection table.

---

## Family 4 — Object/array destructuring "must fully work"

**Approx test count:** ~80.

**Per-binary breakdown:**

| Binary | Count | Local SURFACE shape |
|---|---:|---|
| pattern_matching | 21 (Group A) | "Cannot infer types for binary operation" on destructure-bound `unknown` operands |
| variables_bindings | ~5 | array destructure / for-loop destructure / array-rest |
| objects | 1 | object_destructuring_in_function — `unknown` from destructured params |
| objects_arrays | ~3 | object/array destructure in function param |
| list_comprehension | 1 | string-element rejected by comprehension carrier |
| jit | 1 | object-spread schema_id `__merged_44_45` |
| hashmap | ~10 (some) | value-kind incompat on destructured value bindings |

**Verbatim SURFACE shape:**

```
Semantic error: Cannot infer types for binary operation `Add`: operand
types are `unknown` and `unknown`. Strict typing requires both operands
to have a known concrete type at compile time.
```

No `§5.16` / `§5.15` mis-cite here — bare strict-typing error. The
bindings from `let [a, b] = arr` / `let { x } = obj` / match-extracted
payloads / `fn f({ x, y })` are not inheriting the container element/field
type into the destructured-binding kind track.

**Architectural target.** Compiler array/object destructure path
(`crates/shape-vm/src/compiler/`) propagating container `FieldType` /
element kind into the destructured binding's `BindingStorageClass` per
ADR-006 §2.7.7-§2.7.8 (parallel kind track at the binding side). Pulled
into v0.3 by user 2026-05-21 "Object destructuring must fully work" +
analogically array destructuring same date.

**Minimal repro.**
```shape
fn f({ x, y }) {
    print(x + y)
}
f({ x: 3, y: 7 })
```
Fires "Cannot infer types for binary operation `Add`: operand types are
`unknown` and `unknown`".

**Size estimate:** S–M.

**Sequencing.** **Parallel.** Independent compiler-side fix; doesn't
strictly require ckpt-6 (the destructuring source can be a Shape
literal). Some test fixtures may also depend on Family 1 (e.g.
`t125_top_level_object_rest_destructure` triggers `op_new_array` because
the rest-pattern desugars to a fresh `[]`) — split those out and route the
upstream piece to Family 1.

**Cross-cluster overlaps.** Source-overlap with closures_hof
infer-loss cluster (cluster #2): closures whose params would be
inferred-via-destructure share the binding-kind propagation path.

---

## Family 5 — HashMap rebuild + W13 mutation contract

**Approx test count:** ~65.

**Per-binary breakdown:**

| Binary | Count | Local SURFACE shape |
|---|---:|---|
| hashmap | ~22 (Family A KVE_SURFACE) + ~12 (Family C INT_KEY) + ~33 mixed | `HashMap.{keys,values,entries,toArray}: V3-S5 ckpt-5` + `HashMap key must be a string (got kind Int64)` |
| complex_integration | (mixed) | W17.3-4 + HashMap |

**Verbatim SURFACE shapes:**

```
Runtime error: Not implemented: HashMap.{keys,values,entries,toArray}:
SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface.
`Arc<TypedArrayData>` result carrier DELETED at V3-S5 ckpt-1..ckpt-4
... Rebuild lands at ckpt-6 STRICT close per the per-T v2-raw
`TypedArray<T>` carrier shape.
```

and

```
Runtime error: HashMap key must be a string (got kind Int64) (line N)
```

**Architectural target.** Outer carrier `Arc<HashMapKindedRef>` per
Wave-2-R3b C2-joint ckpt-2..ckpt-4 + ADR-006 §2.7.24 Q25.B SUPERSEDED
amendment (per-V monomorphization with key narrowed to `string` was the
recipe but integer-key support is now in-scope by user pull-in). W13-
hashmap-mutation contract for `let m = HashMap()` chain ergonomics is the
companion piece. Pulled in by 2026-05-21 ("Array<string> must work" +
HashMap fixture family) + 2026-05-22 (W17.3-4 per-V monomorphization).

**Minimal repro.**
```shape
let m = HashMap()
m.set("a", 1).set("b", 2)
m.keys()
```
Fires `HashMap.keys: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3`.

Or for int keys:
```shape
let m = HashMap()
m.set(1, "a")
```
Fires `HashMap key must be a string (got kind Int64)`.

**Size estimate:** M.

**Sequencing.** **KVE family AFTER Family 1** (returns `Array<T>` so
needs `TypedArray<T>` constructed). INT_KEY family **parallel** (Q25.B
per-V key kind extension, independent territory). W13 mutation chain
mostly compiler/method-dispatch — parallel.

**Cross-cluster overlaps.** Heavy overlap with cluster #3 (HashMap
mis-method-resolution `let m = HashMap(); m.set(...)` rejected as
"cannot assign to immutable binding" classified FN-REG-CORRECTNESS in
objects_arrays.md). Recommend merging the HashMap immutable-chain
FN-REG-CORRECTNESS cluster into this dispatch wave.

---

## Family 6 — Comptime trait Cluster A `const`-initializer

**Approx test count:** ~70 (44 SR-1 const-init + 6 SR-comptime-fields + ~20 dispatch + remaining cross-binary).

**Per-binary breakdown:**

| Binary | Count | Local SURFACE shape |
|---|---:|---|
| comptime | 44 (Group SR-1) + ~6 (comptime-fields) + ~20 (Group SR-2 op_new_array — overlaps Family 1) | "Extending the comptime evaluator is v0.4-concurrency-design-pass territory per §5.15" |
| lsp | 1 (`comptime::generated_method_call_from_comptime_extend_executes`) | comptime-extend method generation |

**Verbatim SURFACE shape:**

```
comptime-evaluable (literal, or unary `-`/`!` on a literal). Function
calls and other runtime-dependent expressions are rejected per R8 W8
Cluster A (2026-05-24). Extending the comptime evaluator is
v0.4-concurrency-design-pass territory per
docs/v0.3-close-summary.md §5.15.

Runtime error: Undefined variable: BUILD_TAG.
```

**MIS-CITE.** The SURFACE attributes comptime-evaluator extension to
**§5.15** (v0.4-concurrency-design-pass). §5.15 is module-mutable-bindings
ONLY (close-summary line 678: "Module-level mutable bindings + concurrency
design pass — v0.4 (2026-05-24)"); comptime-trait infrastructure was
explicitly pulled into v0.3 by the 2026-05-22 user disposition (close-
summary §0.A criterion J landed J-CT.0/.1/.2 — 3/4 merged). Shipping
comptime trait without `comptime { }` block binding emission +
`comptime fn` call evaluation + `type_info(T).<field>` + `build_config()`
+ `implements(T, Trait)` evaluation leaves the 2026-05-22 pull-in non-
functional. **44 mis-cites.**

**Architectural target.** Extend the R8 W8 Cluster A `const`-initializer
gate site in `crates/shape-vm/src/compiler/` to accept the full comptime-
evaluable expression grammar (function call, arithmetic, comptime
builtins). The comptime evaluator (`execute_comptime_with_context`) per
J-CT.2 `ae34b01f` is the dispatcher; the gate is the upstream rejection
site.

**Minimal repro.**
```shape
comptime {
    const BUILD_TAG = build_config().target
}
print(BUILD_TAG)
```
Fires "Extending the comptime evaluator is v0.4-concurrency-design-pass
territory per §5.15" mis-cite.

**Size estimate:** M (gate-site extension is bounded; downstream evaluator
already merged).

**Sequencing.** **Parallel.** Compiler gate change is independent of
TypedArray rebuilds; no cross-territory blocker. Some Group SR-2 tests
inside `comptime.md` ALSO fire `op_new_array` — split those to Family 1.

**Cross-cluster overlaps.** No FN-REG-CORRECTNESS cluster overlap. LSP
`generated_method_call_from_comptime_extend_executes` is bridge entry —
include in Family 6 wave.

---

## Family 7 — W18 content-rendering complex interactions + LSP gaps

**Approx test count:** ~24 (12 W18 content + 12 LSP-parity).

**Per-binary breakdown:**

| Binary | Count | Local SURFACE shape |
|---|---:|---|
| lsp | 12 | trait hover/completion/code-lens/goto-def gaps post-Wave-3 |
| strings_formatting | partial (W18-adjacent) | string-formatting interactions with content `f"..."` style specs |
| various | partial | W18 D1 syntax-determined return-type interactions with destructuring / closures |

**Architectural target.** LSP-parity-with-rust-analyzer (close-summary —
no explicit §0.A criterion, named by 2026-05-26 user disposition):
trait hover, trait completion inside `impl { }`, code-lens on trait
definitions, goto-def on impl method/trait names. BindingStorageClass
opt-in inlay hints (per 2026-05-26 disposition; gated on Q25.C.6 IC
devirtualization).

W18 content side: close-summary §0.A criterion E "5/4 COMPLETE (W18.0 +
.2 + .3 + .4 + .5 + .6 all merged; W18.1 v0.4-deferred)." Residual
content surfaces here are complex-interaction edge cases (style-spec
parsing into builder pattern) and the W18.4 spec-types swap-in follow-up.

**Minimal repro.** (LSP)
Open `impl Display for Point { }` in editor, request completion inside
the impl block → empty list (expected: suggest `fmt(self) -> string`).

**Size estimate:** S.

**Sequencing.** **Last.** LSP and W18 residuals don't block other
families and depend on a stable compiler / runtime / type-system surface.
Land after Families 1–6 stabilize.

**Cross-cluster overlaps.** None major; trait hover/completion overlaps
the `traits` binding (34 FN-REG-CORRECTNESS — cluster #X) trait-operator
coverage cluster but the test territories are disjoint.

---

## Recommended dispatch order

| Wave | Family | Rationale |
|---|---|---|
| **1a (first)** | Family 1 (V3-S5 ckpt-5/6 construction) | Architectural keystone; ckpt-6 STRICT close. Unblocks Families 2, 5-KVE, parts of 4. Largest single block (~340). |
| **1b (parallel with 1a)** | Family 3 (W17.3-4 marshal arms) | Separate code territory (ADR-006 §2.7.4 projection table); doesn't wait on ckpt-6. ~110 tests. |
| **1c (parallel with 1a)** | Family 4 (destructuring) | Compiler-side binding-kind propagation; independent of TypedArray rebuilds. ~80 tests. Split rest-pattern fixtures to Family 1. |
| **1d (parallel with 1a)** | Family 6 (comptime trait gate) | Compiler gate-site extension; J-CT.2 evaluator already merged. ~70 tests. Split SR-2 op_new_array fixtures to Family 1. |
| **2a (after Family 1)** | Family 2 (consumer cascade ckpt-2/3) | Producer→consumer dependency; needs TypedArray<T> carriers. ~180 tests. |
| **2b (after Family 1)** | Family 5 KVE (HashMap.keys/values/entries) | Needs TypedArray<T>. ~22 tests. |
| **2b' (parallel with 2b)** | Family 5 INT_KEY + W13 mutation | Q25.B per-V key kind + mutation contract; independent territory. ~33 + W13 tests. |
| **3 (last)** | Family 7 (LSP + W18 residuals) | Depends on stable compiler/runtime surface. ~24 tests. |

**Rationale summary.** Family 1 is the architectural foundation
(producer carrier rebuild) that Families 2 + 5-KVE depend on directly;
Families 3, 4, 6 are independent parallel tracks that can ship same-wave
as Family 1; Family 7 is best deferred to the end so it can be measured
against a stable surface. The merge candidates noted (1+2 super-track;
HashMap immutable-chain FN-REG-CORRECTNESS into Family 5) reduce
dispatch coordination cost.

## Cross-cluster overlap summary

| User-facing cluster | Overlaps with SCOPE-RECLAIM family | Recommendation |
|---|---|---|
| Cluster #1 SIGABRT (complex_integration / structs_types nested-OOM) | Adjacent to Family 1 (TypedArray construction is the carrier the OOMs trip through) | Investigate jointly; SIGABRT triage may fall out of ckpt-6 close |
| Cluster #2 closures_hof infer-loss | Family 2 (consumer-cascade) + Family 4 (destructure-bound infer) | Land after Family 2 lands |
| Cluster #3 objects_arrays HashMap immutable-chain | Family 5 (HashMap rebuild) | Merge into Family 5 wave |
| Cluster #4 silent-wrong-output stdlib_json roundtrip | Family 3 (W17-marshal-return-arms projection table) | Same projection-table fix lands both |
| Cluster #5 traits W1 operator coverage (~30 collapse) | Light overlap with Family 7 LSP trait surface | Land trait fix first (likely single bisect); Family 7 LSP can verify |

## Mis-cite enforcement (pre-commit guard recommendation)

Per SCOPE-RECLAIM.md §"Action items" and 2026-05-27 (B) standing
disposition: add a `check-no-mis-cite` gate that grep-verifies every NEW
SURFACE message string against the TAXONOMY dated-pull-in table at
commit time. Mis-cite anchors to refuse on sight (counts from this audit):

- `§5.16 JIT-lowering followup` for non-§5.16-scope work (~280 mis-cites)
- `§5.15 v0.4-concurrency-design-pass` for comptime-trait work (~44)
- `"Wave 6 follow-up"` (~9)
- bare `"v0.4 / planned"` without dated anchor (~30)
- `"W17-marshal-return-arms follow-up"` (~30)

Mechanical pre-commit hook preferred; the audit-only failure mode of the
v0.3.0/.1/.2 tags traces directly to this gap.

## Discipline footer

- AUDIT-ONLY: no source / fixture changes made.
- No commits during audit.
- No `git stash` used.
- No re-disposition proposals (2026-05-27 (B) is binding).
- All defection-attractor framings refused on sight per CLAUDE.md
  §Forbidden-rationalizations + §Renames-to-refuse-on-sight.
