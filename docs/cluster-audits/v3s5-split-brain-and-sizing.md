# V3-S5 TypedArray migration — split-brain check + rebuild sizing

**Date:** 2026-05-31
**Question (user):** Is the V3-S5 TypedArray migration a *controlled* migration
(one carrier deleted, one carrier rebuilt, gaps bridged by clean errors) or a
*split-brain* (old + partial-new carriers coexisting, producing divergent
results)? Plus: size the rebuild (the dominant v0.3.3 estimate variable).

**Verdict (one line):** CONTROLLED single-carrier migration. NOT split-brain.
All gaps are clean structured errors. Rebuild size **L, ~3–5 sessions** (the
largest SCOPE-RECLAIM body, ~520 of 761 tests) — and the structural foundation
plus most of the consumer hot path has already landed.

All claims below were re-verified against code at HEAD (binary rebuilt
2026-05-29) before writing.

---

## 1. Split-brain verdict: CONTROLLED, single carrier

The old carrier is genuinely **deleted**, not coexisting:

- `TypedArrayData` enum / `struct` — **zero definitions**. `rg "enum
  TypedArrayData|struct TypedArrayData"` over `crates/` returns nothing (exit 1).
  Every `TypedArrayData` textual hit (~200+ across the tree) is a comment, a
  doc-comment, a tombstone, a `VMError::NotImplemented` error string, or a
  "REFUSED ON SIGHT (Refusal #1)" anti-resurrection marker. No type usage, no
  construction, no match-arm.
- `TypedBuffer<T>` / `AlignedTypedBuffer` — no definitions; the file
  `crates/shape-value/src/typed_buffer.rs` does **not exist** (deleted). The
  `pub mod typed_buffer;` line in `lib.rs:35` is a tombstone comment.
- `HeapValue::TypedArray(Arc<TypedArrayData>)` enum arm — deleted at ckpt-4.
  All `HeapValue::TypedArray` hits are comment/tombstone lines
  (`heap_variants.rs:486-501`, `heap_value.rs:4071,4237,4715,4824`).

There is exactly **one** live runtime Array carrier:

- `TypedArray<T>` — flat `#[repr(C)]` struct, 24 bytes (compile-time asserted),
  `crates/shape-value/src/v2/typed_array.rs:28`. Per-T monomorphized (i64 / f64 /
  i32 / u8 / f32 / char scalar + heap-element pointer variants).
- Flowed on the kinded slot directly as `NativeKind::Ptr(HeapKind::TypedArray)`
  holding the raw `*mut TypedArray<T>` — same no-`HeapValue`-wrapper pattern as
  FilterExpr. The on-header magic is the NEW ordinal
  `HEAP_KIND_V2_TYPED_ARRAY = 80` (`v2/heap_header.rs:21`), distinct from the
  legacy `HeapKind::TypedArray = 8` discriminator slot (ordinal 8 VACATED, "do
  not reassign" — it is the kind-track label, not a second carrier).

Single-discriminator proof (no divergent path):

- `as_v2_typed_array(bits, kind)` (`executor/v2_handlers/v2_array_detect.rs:207`)
  accepts **only** `Ptr(HeapKind::TypedArray)` + an on-header
  `HEAP_KIND_V2_TYPED_ARRAY` check. Doc 178-205 explicitly invokes CLAUDE.md
  §"Parallel-implementation across producer/consumer carrier-shape boundaries"
  and describes the kind-track *as* the discriminator — no value/low-address
  heuristic, no `is_heap()` probe.
- 4-table lockstep (`executor/vm_impl/stack.rs:105`, `kinded_slot.rs:812/1179`,
  `closure_layout.rs`, `heap_value.rs`) routes `HeapKind::TypedArray`
  *exclusively* to `retain_v2_typed_array` / `release_v2_typed_array` (on-header
  refcount). The legacy `Arc<TypedArrayData>` retain/release is gone.

The one split-brain *smell* was found and FIXED, not kept: the prior
`NativeKind::UInt64 | Ptr(HeapKind::TypedArray)` overload (scalar-u64 sharing
the array-pointer carrier → SIGSEGV) was corrected at r5c-2-β-CKPT-C
(`c2825f93`, 2026-05-20) to make the kind track the sole discriminator.
`scripts/check-no-dynamic.sh` exits **0** at HEAD (no ValueWord /
TypedArrayData / dynamic-fallback re-introduction).

CLAUDE.md's parallel-implementation forbidden-section and the cluster-0
instance log are *doing their job*: the one historical producer/consumer JIT
`.map()` mismatch was caught by the cluster-0 tracker and closed via the
non-defection Option B (producer migration), refusing Option A (consumer
fallback) and Option C (keep both carriers).

---

## 2. Are all the V3-S5 SURFACEs clean? YES — all clean, none unsafe

Every gap is a structured one-directional dead-end, never a silent-wrong result
or a memory-unsafety:

- **VM consumer surfaces** — `ckpt2_surface` / `ckpt5_surface` builders
  (`array_transform.rs:219`, `object_creation.rs:104`) return a structured
  `VMError::NotImplemented` reporting the receiver kind. Each is
  `#[cold]`-marked. Closure arity is validated *before* surfacing
  (`handle_map_v2` checks `args[1].kind == Ptr(Closure)` first), so arg-shape
  errors aren't swallowed. ~87 `ckptN_surface` call-sites across the executor.
- **VM construction surfaces** — `op_new_array` / `op_new_typed_array` DRAIN
  every popped `(bits, kind)` share via `drop_with_kind` (preserving
  `data.len() == kinds.len()` + refcount discipline) THEN return the structured
  error. No empty-array fabrication, no leak.
- **JIT surfaces** — return a compile-time `Err(String)` ("Route A
  surface-and-stop" / "R8 W7 G.5 SURFACE", ADR-006 §2.7.14). JIT compilation
  *fails closed* and falls through to the VM interpreter so the VM and JIT
  surfaces agree. No silent wrong-codegen, no Bool-default fallback.

Empirical (release binary, HEAD, debug-trace filtered):

- `[1,2,3,4,5].sum()` → `15` (works)
- `[1,2,3,4,5].slice(1,3)` → `[2, 3]` (works, single carrier)
- `[1,2,3,4,5].map(|x| x*2).filter(|x| x>4)` → **CORRECTION 2026-05-31: this
  claim is FALSE — it SURFACEs on `filter`. See §7. The `[6,8,10]` result is
  actually produced by `filter().map()`, a transposition. `map().filter()` and
  `map().map()` SURFACE (inline-chain map-result inference loss).**

No divergent or silent-wrong result was produced in any test. The JIT
fail-closed message ("NewTypedArrayI64 ... has no FrameDescriptor ... falling
through to bytecode interpreter") is itself a clean surface that *preserves*
correctness — VM and JIT agree.

---

## 3. Material update to the prior facets (done/stubbed boundary moved)

The facets were captured at an earlier point when `.map().filter()` cleanly
*surfaced*. **At HEAD it now returns the correct result.** Root cause: `map` /
`filter` / `reduce` / `flatMap` / `groupBy` are now **pure-Shape `extend
Vec<T>` methods** (`crates/shape-runtime/stdlib-src/core/vec.shape:46-75`):

```shape
method map<U>(f: (T) => U) -> Vec<U> {
    let mut result = []
    for item in self { result.push(f(item)) }
    result
}
```

These compile to a `for`-loop that constructs the v2 carrier via
`NewTypedArrayI64` + `TypedArrayPushI64` (visible in the JIT trace function name
`Vec.map::i64_i64_closure_...`). Method dispatch resolves to the Shape `extend`
method, so the native `handle_map_v2` SURFACE stub in `array_transform.rs` is
**off the hot path** — it survives only as a PHF-signature placeholder. The
structural single-carrier verdict is unchanged and *reinforced*: the Shape
methods build and consume exactly the same `*mut TypedArray<T>` carrier.

Net effect on sizing: the consumer hot path (the per-handler closure-callback
rebuild the facets flagged as the residual) has been substantially absorbed by
the pure-Shape `extend Vec<T>` route. The remaining native SURFACE stubs are
the construction-cascade tail + heap-element / heterogeneous arrays + scattered
set/joins/datatable handlers, not the everyday map/filter/reduce path.

---

## 4. Rebuild size: L, ~3–5 sessions, the dominant SCOPE-RECLAIM body

Three tracking artifacts agree (all present and verified):

- `docs/cluster-audits/w12-typed-array-data-deletion-audit.md` §3.7 —
  **5 sub-clusters, ~4.5 sessions** (floor 4 / ceiling 6; ceiling only if
  O-3/O-3a needs a multi-week storage-tier redesign). NOTE: this estimate
  *predates* the partial landing — S1 scalar pass, String/Decimal/TypedObject/
  TraitObject carriers, and ckpt-1..ckpt-5 (enum deletion + 4-table lockstep +
  per-T mutation APIs + JIT-FFI ckpt-6-prime allocators) have all landed.
- `docs/cluster-audits/v0.3.3-scope-reclaim-sizing.md` —
  **~520 of 761 SCOPE-RECLAIM tests** (the largest cluster), split for
  bookkeeping into v3s5-ckpt56 construction-cascade (~340) + v3s5-ckpt23
  consumer-cascade (~180), but **ONE carrier-migration program**.
  **4 net-new fix-seams** (of 13 total v0.3.3 net-new): construction build-path,
  consumer-handler rewrite, per-T element stringification (`joinStr`), and
  per-element-kind consumer-handler rebuild. **Effort L; ~3–5 sessions.**
- `docs/cluster-audits/v0.3-w16-v3s5-ckpt56-strict-close-audit.md` — the
  v0.3-gating slice is just 3 construction-site SURFACEs (`op_new_array` +
  `op_new_typed_array` + dormant `op_new_object`), closeable via 2–3
  sub-clusters (W16.2-A TypedObject-element, W16.2-B TraitObject-element,
  optional W16.2-C empty-literal). Temporal/bigint/instant element kinds are
  v0.4-deferred (dead arms, zero live producers).

What's LANDED (working through the single carrier): scalar element kinds wired
end-to-end (NewTypedArray I64/F64/I32/Bool + String/Decimal opcodes); the
slice/concat/take/drop/skip/sort handlers migrated to live primitives
(`array_transform.rs` `handle_slice_v2`/`handle_concat_v2`/`handle_take_v2`/
`handle_drop_v2`/`handle_skip_v2`/`handle_sort_v2` all consume via
`extract_view` → confirms the consumer side is real); the pure-Shape
map/filter/reduce/flatMap/groupBy route; ckpt-1..ckpt-5 enum deletion + wire/
json/marshal migration + 4-table lockstep + JIT-FFI allocators.

What's REMAINING (ckpt-6 STRICT close): native `op_new_array` heterogeneous +
heap-element-array construction (Array<TypedObject>, Array<dyn T>, spread,
empty `[]`); the scattered native consumer-method SURFACE stubs
(flatten/flatMap/groupBy/joins/sets + set/hashmap/priority-queue/datatable);
per-T element stringification; JIT heap-element allocators. The hard structural
pieces (carrier design, 4-table lockstep, refcount discipline, JIT-FFI
foundation) are done — the residual is mechanical per-handler rebuild against an
existing carrier, **closer to the audit FLOOR than ceiling.**

---

## 5. Is V3-S5 the only big half-done migration? No — bounded-and-tracked, no surprises

Every mid-migration marker in the tree belongs to one of FOUR overlapping
families, all enumerated in `docs/cluster-audits/phase-2d-stub-inventory.md` +
`AGENTS.md`:

1. **V3-S5 TypedArray** [this] — tracked: W12 audit + ADR-006 §2.7.24.
2. **W17 / phase-2d sub-cluster umbrella** (~28 sub-clusters, all citing
   ADR-006 §2.7.x): snapshot-resume (`PHASE_2C_SNAPSHOT_SURFACE`),
   trait-object-rebuild (BoxedReturn rewrap), hashmap-typed-buffer
   (`HashMapData<V>` / `HashMapKindedRef` monomorphization — arm flipped,
   mutation API still gated), datatable-results, array-*, concurrency,
   foreign-ffi, typed-module-exports. This is the *umbrella*, not separate
   surprises.
3. **ConcreteReturn keystone** (~75%) — its tail is the BoxedReturn rewrap
   surfaces in (2).
4. **W17-jit-stubs** — the JIT-FFI tier = V3-S5 ckpt-6 + W17 marshal in the
   JIT. Explicitly DEFERRED to phase-2d Item 3 (JIT verification).

`docs/cluster-audits/phase-2d-stub-inventory.md` sizes the whole debt: ~104
production `Err(NotImplemented(...SURFACE...))` sites + ~55 production
`todo!("phase-2c…")` stubs + ~300 test placeholders + ~35 compile-tier stubs;
estimate ~4–6 days with 4–6 parallel agents OR ~16–22 days serial; largest
single sub-clusters W17-typed-carrier-monomorphization (24–32h) and
W17-snapshot-resume (8–16h). No marker pointed at any subsystem outside the
value/heap-carrier + marshal/snapshot + JIT-FFI scope already in the v0.3.3
workstreams. `shape-wire/src` is clean (zero SURFACE/todo markers).

---

## 6. Estimate impact (V3-S5 = dominant SCOPE-RECLAIM variable)

V3-S5 dominates the v0.3.3 SCOPE-RECLAIM body (~520 of 761 tests = ~68%), so
its sizing *is* the primary swing on the v0.3.3 close estimate.

- It is its OWN keystone — NOT one of the three previously-counted keystones
  (§2.7.4 host-tier marshal, strict-flip checker-fix, ref-ser), so it is honest
  net-new work that must be carried in the projection. It contributes **4 of the
  13 total net-new fix-seams** for v0.3.3.
- The swing is asymmetric toward the floor: the carrier design, 4-table
  lockstep, refcount discipline, JIT-FFI foundation, scalar element kinds, the
  slice/sort family, and the pure-Shape map/filter/reduce route have ALL landed.
  The remaining work is mechanical per-handler rebuild against an existing
  carrier. That pushes the W12 §3.7 range toward floor-4 rather than ceiling-6.
- Ceiling risk (the O-3/O-3a storage-tier redesign that drove the 6-session
  ceiling) is RESOLVED per the cluster-0 status (TypedObject/TraitObject
  HeapHeader migration landed), so the ceiling pressure is largely retired.

**Bottom line for v0.3.3:** treat V3-S5 as effort **L, ~3–5 sessions** of
net-new work, weighted toward the lower end. It is the single largest line item
but is past its highest-risk structural phase; the residual is the mechanical
construction-cascade + scattered consumer/JIT-FFI tail, all bounded and tracked
in W12 + phase-2d-stub-inventory + the W16 strict-close audit.

---

## 7. CORRECTION (2026-05-31, team-lead run-verify) — the §2/§3 "consumer hot path absorbed" claim is partly FALSE

Run-verified at HEAD against BOTH the release binary (2026-05-29 10:42) and the
debug binary (2026-05-29 19:55 = at-HEAD-for-code; last code commit was the
14:20 PB5 merge, everything after is docs-only). Identical results on both,
VM and JIT:

| Expression | Result |
|---|---|
| `[1,2,3,4,5].filter(\|x\| x>3)` (standalone) | ✓ `[4, 5]` |
| `[1,2,3,4,5].map(\|x\| x*2)` (standalone) | ✓ `[2,4,6,8,10]` |
| `[1,2,3,4,5].reduce(\|a,x\| a+x, 0)` | ✓ `15` |
| `filter().map()` | ✓ `[6, 8, 10]` |
| `filter().filter()` | ✓ `[2, 3, 4]` |
| `map().sum()` / `.reduce()` / `.len()` / `.slice()` | ✓ work |
| **`map().filter()`** (int AND number) | ❌ **SURFACE on `filter`** |
| **`map().map()`** | ❌ **SURFACE on `map`** |
| `let m = arr.map(...); m.filter(...)` (let-bound) | ✓ `[4, 6]` |
| `fn f(v: Vec<int>){v.filter(...)}; f(arr.map(...))` | ❌ **SURFACE** |

**The §2 line-100 empirical was a transposition.** `[6,8,10]` is produced by
`filter().map()`, NOT `map().filter()`. The real `map().filter()` SURFACEs.

**Root cause is NOT a carrier gap — it is inline-chain element-type inference
loss across `map<U>`.** Proof: the SAME `.filter()` on a SAME map-result works
when the map-result is *let-bound* (`let m = arr.map(...); m.filter(...)` → ✓)
but SURFACEs when *inline-chained* (`arr.map(...).filter(...)` → ❌). The
let-binding gives the receiver a resolved `Vec<int>` type so `.filter()`
dispatches to the pure-Shape `Vec.filter` extend method (vec.shape:46); the
inline map-result does NOT carry its element kind `U` to the chained call site,
so dispatch falls through to the **native** `handle_filter` ckpt-2 SURFACE stub.
`.sum()/.reduce()/.len()/.slice()` survive inline because their native handlers
are implemented (slice migrated; sum/reduce/len don't re-dispatch on `Vec<U>`).

**Sizing impact.** §3's "consumer hot path absorbed by the pure-Shape route" is
*directionally* right but leaves a real, common residual: any chain where
`.map()` (or `.map().map()`) feeds another *consumer* method (`filter`/`map`/
other native-stub consumers). This is a HIGH-frequency idiom. BUT the fix is
most likely a **compiler inference fix** (propagate `map<U>`'s declared return
element type to the chained-receiver dispatch site so it resolves to the
pure-Shape method), NOT the big native ckpt-2 consumer-cascade rebuild — the
let-bound success proves the carrier + pure-Shape methods are fully capable.
This item belongs with the inference-loss family (triage against PB2
inference-loss-after-`?` + c8 closure-param-infer), carried SEPARATELY from
the native construction-cascade tail. It does not enlarge the native ckpt-6
rebuild; it adds one inference seam. V3-S5 floor-weighting otherwise stands.

---

## Citations

- `crates/shape-value/src/v2/typed_array.rs:28` — sole `TypedArray<T>` struct (24-byte repr(C))
- `crates/shape-value/src/v2/heap_header.rs:21` — `HEAP_KIND_V2_TYPED_ARRAY = 80`
- `crates/shape-value/src/heap_variants.rs:486-501,70` — deleted `HeapValue::TypedArray` arm; ordinal 8 VACATED
- `crates/shape-value/src/lib.rs:35,74` — `typed_buffer` mod/use tombstone; file absent
- `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs:178-215` — single-carrier `as_v2_typed_array`, u64-disambiguation, CLAUDE.md parallel-impl invocation
- `crates/shape-vm/src/executor/vm_impl/stack.rs:85-130` — 4-table lockstep → `retain_v2_typed_array` (single carrier; UInt64-overload-fix history)
- `crates/shape-value/src/kinded_slot.rs:812,1179` — clone/drop arms → `release_v2_typed_array`/`v2_retain`
- `crates/shape-vm/src/executor/objects/array_transform.rs:219-269` (ckpt2_surface + arity pre-check), :340-490 (migrated slice/concat/take/drop/skip/sort via extract_view), :148-157 (flatten/flatMap still stubbed)
- `crates/shape-vm/src/executor/objects/object_creation.rs:104-124` — drain-then-surface construction stubs
- `crates/shape-runtime/stdlib-src/core/vec.shape:46-75` — pure-Shape map/filter/reduce/flatMap (builds the v2 carrier via push loop)
- `crates/shape-jit/src/mir_compiler/rvalues.rs:337` etc. — JIT compile-time Err (fail-closed to interpreter)
- `docs/cluster-audits/w12-typed-array-data-deletion-audit.md` §3.7 — 5 sub-clusters / ~4.5 sessions / floor 4 / ceiling 6
- `docs/cluster-audits/v0.3.3-scope-reclaim-sizing.md` — ~520/761 tests / 4 net-new seams / L / ~3–5 sessions / 13 total net-new
- `docs/cluster-audits/v0.3-w16-v3s5-ckpt56-strict-close-audit.md` — 3 construction-site v0.3-gating SURFACEs
- `docs/cluster-audits/phase-2d-stub-inventory.md` §1.A/§1.B/§3/§4 — whole-tree migration-debt inventory
- `docs/cluster-audits/phase-3-cluster-0-status.md` — parallel-impl defection tracker (JIT .map() mismatch caught + closed Option B; O-3/O-3a resolved)
- `scripts/check-no-dynamic.sh` — exits 0 at HEAD
- Empirical (release binary, 2026-05-29): `sum()`=15, `slice(1,3)`=[2,3], `map().filter()`=[6,8,10] (correct, single carrier)
