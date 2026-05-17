# Bulldozer Wave 1 inventory — comprehensive deletion-target audit

Phase 3 cluster-0+1 Wave 1 single-audit-day under the bulldozer cadence
(strategic-owner authorization 2026-05-14 — audit-first sub-cluster cadence
R7-R20 replaced by bulldozer waves). Branch
`bulldozer-strictly-typed-wave-1-audit`, parent `aa047356` (post-R20
ceremony: CLAUDE.md compaction + handover docs cadence-shift +
S2-prime-production merge + R20 status-doc close annotations).
Date: 2026-05-14.

This is an **audit-only deliverable**. Zero source changes inside this
dispatch. The deliverable is forward-visibility for Wave 2's parallel
bulldoze (6-8 agents in coordinated dispatch) and Wave 3 stabilize +
cluster-0+1 close.

The dispatch's discipline (per `phase-3-team-lead-handover.md` §"Wave 1
dispatch shape"): **comprehensive ground-truth coverage in a single
dispatch, no per-target sub-clusters, no speculative "needs another
audit" disposition.** Every claim in this document is grep-verified
against source at HEAD `aa047356` — the 5-instance supervisor-/audit-
layer imprecision pattern (R20 S2-prime caught rust_decimal align-of=8
vs measured align-of=4) is the signal to verify every ground-truth
claim before landing.

---

## §0 Status + structural framing

**Audit-only close.** Zero source changes. Baseline gate run on close:
`bash scripts/verify-merge.sh` exit 0.

This audit consumes the R17 audit (`docs/cluster-audits/w12-typed-array-
data-deletion-audit.md`, the parent of this work) + its R19/R20
amendments and produces the next-level inventory: for every remaining
cluster-0 + cluster-1 deletion target, the audit grounds the producer
+ consumer site counts, the migration shape, the cascade-site count
estimate, and the Wave 2 dispatch shape.

The deletion targets covered (sections A through L):

| § | Target | Status |
|---|---|---|
| §A | `TypedArrayData` enum (22 live variants) | mapped + designed |
| §B | `TypedBuffer<T>` + `AlignedTypedBuffer` wrapper layer | mapped + designed |
| §C | `HashMapValueBuf` enum (Q25.B parallel deletion) | mapped + designed |
| §D | `TypedObjectStorage` Arc → HeapHeader migration (O-3) | mapped + designed |
| §E | `TraitObjectStorage` HeapHeader migration (O-3a) | mapped + designed |
| §F | Q25.A specialization dead arms wholesale deletion | mapped + designed |
| §G | W12-stdlib-intrinsic-collapse (IntrinsicSum / `.sum()` split-brain) | mapped + designed |
| §H | Cross-tier shape-conversion for `Array<string>` / `Array<decimal>` v2-raw read path | mapped + designed |
| §I | Surface A — kickoff-prompt-vs-fixture mismatch (all 3 options) | mapped + designed |
| §J | 23+ shape-jit `#[ignore]`'d tests | mapped + designed |
| §K | 48 shape-test pre-existing failures | surface-and-stop (cluster-2 audit triage) |
| §L | Wave 2 agent partition recommendation | proposed |

**Architectural framing rule** (per `phase-3-team-lead-handover.md`
§"Discipline rules" #10): "preserve X for cluster-1+" / "needs its own
audit sub-cluster" / "multi-week scope" / "defer to cluster-1.5
post-close" are refused on sight under bulldozer cadence. Every
deletion target in §A-L either has a designed Wave 2 territory or
surfaces a specific structural reason for genuine in-wave
intractability (§§J/K are the only two surfaces that meet this bar —
§J because 11 of 23 ignored tests are extern-C-todo!() SURFACEs that
abort the test process before any assertion runs, gated on JIT
playbook §5 / Q11 / Q12 rebuilds; §K because 48 shape-test failures
across 7 distinct failure classes exceed any plausible Wave 2 agent
territory).

---

## §A — `TypedArrayData` enum (audit §1 / §2 deletion target)

### §A.1 Definition + audit-§2 disposition mapping

`pub enum TypedArrayData` lives at
`crates/shape-value/src/heap_value.rs:2942-2994` (53-line definition).
At HEAD `aa047356` the enum has **22 live variants** (post the §2.3
`Matrix` / `FloatSlice` deletions of Round 18 S3, per
`heap_value.rs:2946-2951` deletion comment):

```
crates/shape-value/src/heap_value.rs:2942:pub enum TypedArrayData {
crates/shape-value/src/heap_value.rs:2943:    I64(Arc<crate::typed_buffer::TypedBuffer<i64>>),
crates/shape-value/src/heap_value.rs:2944:    F64(Arc<crate::typed_buffer::AlignedTypedBuffer>),
crates/shape-value/src/heap_value.rs:2945:    Bool(Arc<crate::typed_buffer::TypedBuffer<u8>>),
crates/shape-value/src/heap_value.rs:2952:    I8(Arc<crate::typed_buffer::TypedBuffer<i8>>),
crates/shape-value/src/heap_value.rs:2953:    I16(Arc<crate::typed_buffer::TypedBuffer<i16>>),
crates/shape-value/src/heap_value.rs:2954:    I32(Arc<crate::typed_buffer::TypedBuffer<i32>>),
crates/shape-value/src/heap_value.rs:2955:    U8(Arc<crate::typed_buffer::TypedBuffer<u8>>),
crates/shape-value/src/heap_value.rs:2956:    U16(Arc<crate::typed_buffer::TypedBuffer<u16>>),
crates/shape-value/src/heap_value.rs:2957:    U32(Arc<crate::typed_buffer::TypedBuffer<u32>>),
crates/shape-value/src/heap_value.rs:2958:    U64(Arc<crate::typed_buffer::TypedBuffer<u64>>),
crates/shape-value/src/heap_value.rs:2959:    F32(Arc<crate::typed_buffer::TypedBuffer<f32>>),
crates/shape-value/src/heap_value.rs:2960:    String(Arc<crate::typed_buffer::TypedBuffer<Arc<String>>>),
crates/shape-value/src/heap_value.rs:2967:    Decimal(Arc<crate::typed_buffer::TypedBuffer<Arc<rust_decimal::Decimal>>>),
crates/shape-value/src/heap_value.rs:2968:    BigInt(Arc<crate::typed_buffer::TypedBuffer<Arc<i64>>>),
crates/shape-value/src/heap_value.rs:2969:    DateTime(Arc<crate::typed_buffer::TypedBuffer<Arc<TemporalData>>>),
crates/shape-value/src/heap_value.rs:2970:    Timespan(Arc<crate::typed_buffer::TypedBuffer<Arc<TemporalData>>>),
crates/shape-value/src/heap_value.rs:2971:    Duration(Arc<crate::typed_buffer::TypedBuffer<Arc<TemporalData>>>),
crates/shape-value/src/heap_value.rs:2972:    Instant(Arc<crate::typed_buffer::TypedBuffer<Arc<std::time::Instant>>>),
crates/shape-value/src/heap_value.rs:2973:    Char(Arc<crate::typed_buffer::TypedBuffer<char>>),
crates/shape-value/src/heap_value.rs:2974:    TypedObject(Arc<crate::typed_buffer::TypedBuffer<Arc<TypedObjectStorage>>>),
crates/shape-value/src/heap_value.rs:2980:    TraitObject(Arc<crate::typed_buffer::TypedBuffer<Arc<TraitObjectStorage>>>),
crates/shape-value/src/heap_value.rs:2994:}
```

### §A.2 Producer / consumer counts at HEAD `aa047356`

Total `TypedArrayData::*` references across `crates/`: **1,451 hits across 53 files**
(grep `TypedArrayData::` --include="*.rs"). External to `shape-value`
(the definition site): **~1,387 hits across 49 files**.

#### §A.2.1 Per-variant Arc-wrapping producer counts (`Arc::new(TypedArrayData::X(`)

| Variant | Producer hits | Producer hits ex-shape-value |
|---|---|---|
| I64 | 34 | 34 |
| F64 | 26 | 26 |
| Bool | 19 | 19 |
| I8 | 6 | 6 |
| I16 | 6 | 6 |
| I32 | 6 | 6 |
| U8 | 6 | 6 |
| U16 | 6 | 6 |
| U32 | 6 | 6 |
| U64 | 5 | 5 |
| F32 | 5 | 5 |
| String | 20 | 20 |
| Decimal | 7 | 7 |
| BigInt | 7 | 7 |
| DateTime | 4 | 4 |
| Timespan | 4 | 4 |
| Duration | 4 | 4 |
| Instant | 4 | 4 |
| Char | 7 | 7 |
| TypedObject | 15 | 15 |
| TraitObject | 4 | 4 |

Total external producer hits: **201** (`grep -rnE "TypedArrayData::\w+\(Arc::new" crates/ | grep -v /heap_value.rs/ | wc -l`).

#### §A.2.2 Root-vs-derived producer ground-truthing (R20 S2-prime finding restated against HEAD)

The R20 S2-prime audit (§4.1.A.2) found that DateTime / Timespan /
Duration / Instant **have zero root constructors** — the producer sites
in `array_transform.rs` / `concat.rs` / `array_ops.rs` are all **derived
operations** (slice, zip, concat, repeat) that pattern-match on the
input arm and re-wrap.

Re-verified at HEAD `aa047356`:

- **DateTime producer hits (4 total ex-shape-value):**
  - `crates/shape-vm/src/executor/objects/concat.rs:271` — derived (concat output arm)
  - `crates/shape-vm/src/executor/objects/array_transform.rs:579` — derived (slice output arm)
  - `crates/shape-vm/src/executor/objects/array_transform.rs:744` — derived (zip output arm)
  - `crates/shape-vm/src/executor/objects/array_transform.rs:992` — derived (reverse output arm)

  Zero root constructors. `op_new_array` → `build_specialized_from_heap_arcs`
  at `heap_value.rs:3008` has NO arm for `HeapValue::Temporal` /
  `HeapValue::Instant` — the `other =>` arm at line 3088 returns
  `Err("TypedArrayData::build_specialized: HeapValue arm {:?} not yet
  supported post-§2.7.24 Q25.A")`.

- **Timespan / Duration / Instant**: identical shape — only
  derived-operation hits in `array_transform.rs:585/750/997` (Timespan),
  `:591/756/1002` (Duration), `:597/762/1007` (Instant) plus `concat.rs:279/287/295`.

- **String live root constructors** (audit §4.1.A.2 confirmed):
  - `crates/shape-vm/src/executor/objects/object_creation.rs:518` — `op_new_array` String arm
  - `crates/shape-vm/src/executor/objects/object_creation.rs:799` — array-helper String arm
  - Plus derived sites in concat / array_transform / hashmap_methods

- **Decimal live root constructors**:
  - `crates/shape-vm/src/executor/objects/object_creation.rs:544` — `op_new_array` Decimal arm
  - `crates/shape-vm/src/executor/builtins/array_ops.rs:485` — `filled()` builtin Decimal arm
  - `crates/shape-value/src/heap_value.rs:3044` — `build_specialized_from_heap_arcs` Decimal arm
  - Plus derived sites in `array_transform.rs:567/732/983/1460` and `concat.rs:255`.

- **BigInt live root constructors**:
  - `crates/shape-vm/src/executor/objects/object_creation.rs:563` — `op_new_array` BigInt arm
  - `crates/shape-vm/src/executor/builtins/array_ops.rs:492` — `filled()` builtin BigInt arm
  - `crates/shape-value/src/heap_value.rs:3058` — `build_specialized` BigInt arm
  - Plus derived sites.

- **Char live root constructors**:
  - `crates/shape-vm/src/executor/objects/object_creation.rs:607` — `op_new_array` Char arm
  - `crates/shape-vm/src/executor/builtins/array_ops.rs:498` — `filled()` builtin Char arm
  - `crates/shape-value/src/heap_value.rs:3086` — `build_specialized` Char arm

- **TypedObject live root constructors** (Q25.A user-defined-type catch-all):
  - `crates/shape-vm/src/executor/objects/object_creation.rs:584` — `op_new_array` TypedObject arm
  - `crates/shape-vm/src/executor/builtins/array_ops.rs:428/478` — pairs / filled
  - `crates/shape-vm/src/executor/objects/iterator_methods.rs:891/961` — map / collect
  - `crates/shape-vm/src/executor/objects/loops/mod.rs:688` — for-loop body
  - Plus `datatable_methods/core.rs:399/448` (toMat / asTable)
  - Plus `hashmap_methods.rs:389` (values projection)
  - Plus `xml.rs:106` (attrs children array)
  - Plus `comptime_target.rs:287` (comptime value)
  - **Verdict:** ~10 root constructor sites, broadest live producer scope.

- **TraitObject producer hits (4 total ex-shape-value)**: all derived
  (`array_transform.rs:615/780/1021` + `concat.rs:319`). **Zero root
  constructors** — `build_specialized_from_heap_arcs` has no
  `HeapValue::TraitObject` arm. Same dead-arm shape as DateTime family.

#### §A.2.3 Consumer match-arm counts (no `Arc::new`)

Per-variant consumer hits (match arms, format / type_name / Display
implementations, exhaustive matches in array_transform / hashmap /
printing / wire_conversion / xml / json):

| Variant | Consumer hits |
|---|---|
| I64 | 89 |
| F64 | 91 |
| Bool | 65 |
| String | 70 |
| I8 / I16 / I32 / U8 / U16 / U32 / U64 / F32 / Decimal / BigInt / DateTime / Timespan / Duration / Instant / Char / TraitObject | 38-40 each |
| TypedObject | 41 |

**Total consumer arms across all variants**: ~1,300 hits. Concentrated
in ~30 files. The exhaustive-match cascade is what makes the deletion
both straightforward (`#[deprecated]` warnings expose every site) and
mechanical (each variant's match arm has a clear migration target).

### §A.3 Per-variant migration disposition (re-statement against audit §2)

| Variant | Disposition | Migration target | Wave 2 agent |
|---|---|---|---|
| I64 / F64 / Bool | scalar — clean | `TypedArray<i64>` / `TypedArray<f64>` / `TypedArray<u8>` (already exist post-§2.1) | Agent A1-scalar |
| I8 / I16 / I32 / U8 / U16 / U32 / U64 / F32 / Char | scalar — clean, new monomorphization | `TypedArray<i8>` etc. (4 already exist: f64/i64/i32/u8; 8 new: i8/i16/u16/u32/u64/f32/char) | Agent A1-scalar |
| String | heap-element — clean | `TypedArray<*const StringObj>` (StringObj exists at `crates/shape-value/src/v2/string_obj.rs:18-26` per R12 close) | Agent A2-heap |
| Decimal | heap-element — clean | `TypedArray<*const DecimalObj>` (DecimalObj exists at `crates/shape-value/src/v2/decimal_obj.rs` per R20 S2-prime-production close) | Agent A2-heap |
| BigInt | heap-element — DEFERRED to cluster-1+ per R19 disposition (Obstacle 3 — BigInt full-width Rust type design not landed). Enum arm stays alive on legacy `Arc<TypedBuffer<Arc<i64>>>` carrier; S5 deletion of enum forces a disposition (drop the arm and lose Array<BigInt> production OR ship a placeholder BigIntObj wrapping the i64). | N/A pre-Wave-2 | Agent F (decision: drop arm) |
| DateTime / Timespan / Instant | heap-element — DEAD ARM per §A.2.2 (zero root producers). Audit §4.1.D delivered the design but R20 S2-prime supervisor disposition was **(A) Minimal** (only String + Decimal); the carrier files (DateTimeObj / TimespanObj / InstantObj) are NOT LANDED. Under bulldozer cadence: dead arms wholesale-delete in §F (no producer migration, no Obj carrier needed). | DELETE (dead) | Agent F |
| Duration | heap-element — DOUBLY DEAD per §4.1.A.1 (`TemporalData::Duration` enum variant has zero constructors anywhere; `TypedArrayData::Duration` has zero root producers). Same as DateTime/Timespan/Instant for Wave 2 scope. | DELETE (dead) | Agent F |
| Char | scalar — already classified as scalar bucket per R19 S1.5 close; rides `NativeKind::Char` post-R19 amendment to §2.7.5. | `TypedArray<char>` (32-bit scalar) | Agent A1-scalar |
| TypedObject | structural obstacle O-3 — `TypedObjectStorage` is `Arc<>`-wrapped, not `HeapHeader`-equipped. Wave 2 Agent D (TypedObjectStorage Arc → HeapHeader migration) is the **gate**: once Agent D's migration lands, TypedObject arm migrates to `TypedArray<*const TypedObjectStorage>` per audit §2.2 row. Under bulldozer cadence: Agent A (TypedArrayData enum deletion) WAITS for Agent D's close before deleting the TypedObject arm — OR Agent D's close commit deletes the arm directly in the same merge. | `TypedArray<*const TypedObjectStorage>` (post-Agent-D) | Agent A2-heap (post-D) |
| TraitObject | structural obstacle O-3a — fat-pointer extension of O-3, gated on Agent E (`TraitObjectStorage` HeapHeader migration). Same dispatch shape; **zero live root producers today** (only derived sites — see §A.2.2). Agent E close OR Agent A delete the arm directly. | `TypedArray<*const TraitObjectStorage>` (post-Agent-E) | Agent A2-heap (post-E) |

### §A.4 Cascade-site count estimate

For Agent A1 (scalar variant deletion + replacement with v2-raw
`TypedArray<T>` producers):

- **8 new typed opcodes** per kind × 4 dispatch points (compile / VM
  handler / JIT FFI / element-type-tag byte) = **32 LoC-rich sites**
  per new monomorphization × 8 new monomorphizations (i8 / i16 / u16 /
  u32 / u64 / f32 / char + retain Bool / U8 distinction) = **~256
  sites** (under the R19 S1.5 ~100-site ceiling at the per-kind level,
  comfortably within budget for a parallel-dispatch agent).

For Agent A2 (heap-element variant deletion: String + Decimal live;
TypedObject / TraitObject via gates):

- **String producer migration**: ~53 sites per status doc §"R19
  parallel-sub-cluster" estimate (the audit §3.2 number for live String
  arm producer migration). At HEAD `aa047356`: 20 Arc-wrapped producer
  hits + ~50 consumer match-arms = **~70 sites**. R20 prereq 1
  disposition: +8 typed opcodes path per kind for String (mirror S1
  scalar recipe).
- **Decimal producer migration**: 7 producer hits + ~39 consumer
  match-arms = **~46 sites**. Same +8 opcodes path. DecimalObj already
  landed.

For Agent F (dead-arm wholesale deletion):

- **DateTime / Timespan / Duration / Instant / TraitObject arms**: 4
  derived producers each × 5 arms = **20 producer-arm deletions**.
  Each arm has ~38-40 consumer match-arms; the wholesale-delete shape
  is "delete the arm" + "delete every dispatch-table match arm" =
  **~200 consumer-arm deletions**. Bounded.

**Cascade-surface-and-stop threshold:** none of these exceed the R19
S1.5 ~100-site ceiling at the per-migration level. The cumulative
total (Agent A1 + A2 + F) is ~600 sites, but split across 3 agents
landing in parallel + the merge ceremony for the union of dispatch
tables.

### §A.5 ADR-fit confirmation

Per ADR-006 §2.7.24 Q25.A SUPERSEDED preamble (at
`docs/adr/006-value-and-memory-model.md:4715`): the migration target is
the v2-raw `TypedArray<*const <X>Obj>` carrier, NOT the Q25.A original
`TypedArrayData::<X>` specialized variants. The audit §2 disposition
table is the binding migration shape; Q25.A's specialized variants are
themselves part of the deletion target.

ADR-005 §1 (single-discriminator) preserved: per-T monomorphization at
the bytecode-emitter + JIT-codegen layers; the variant-tag-as-kind
pattern in `TypedArrayData` is the deleted shape, not a preserved one.

ADR-006 §2.7.5 (stamp-at-compile-time) preserved: element-kind byte at
`crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs:40-50` is
the JIT consumer's fast-path discriminator, NOT a runtime kind probe
for retain/release.

### §A.6 Forbidden patterns this migration rules out

Per CLAUDE.md "Forbidden Patterns" + audit §0:

- **"Preserve TypedArrayData for one period"** framing (W-series
  declare-victory at sub-cluster-disposition layer). Refused.
- **`TypedArrayData::HeapValue` polymorphic catch-all resurrection** —
  Q25.E #1, retired post-bundle-A.
- **"Carrier unification via boundary deletion as one-off patch"** —
  the migration is systematic per-producer, not a single-deletion
  shortcut.
- **bridge / probe / helper / hop / translator / adapter / shim**
  framings for the `TypedArrayData` → `TypedArray<T>` migration. The
  migration is "v2-raw `TypedArray<T>` is the universal `Array<T>`
  carrier" + "per-element-kind monomorphization at compile time" — no
  boundary translation, no per-variant flatten-and-unwrap.
- **Re-introducing JitArray** as parallel discriminator to
  TypedArrayData (W10-misc deletion target per §2.7.14).

---

## §B — `TypedBuffer<T>` + `AlignedTypedBuffer` wrapper layer (audit §1.1)

### §B.1 Definition

`crates/shape-value/src/typed_buffer.rs` entire file (485 LoC):

- `pub struct TypedBuffer<T> { data: Vec<T>, validity: Option<Vec<u64>> }`
  at lines 14-223 — Arrow-style nullable-bitmap wrapper.
- `pub struct AlignedTypedBuffer { data: AlignedVec<f64>, validity:
  Option<Vec<u64>> }` at lines 231-376 — F64-SIMD specialization
  wrapping `AlignedVec<f64>`.

### §B.2 Reference counts at HEAD `aa047356`

```
TypedBuffer<        : 144 hits across 26 files
AlignedTypedBuffer  : 123 hits across 27 files
```

Both shapes are the inner Arc-wrapped payload of every `TypedArrayData`
arm + the values-payload of `HashMapValueBuf` arms. Their reference
count is fully dependent on the `TypedArrayData` and `HashMapValueBuf`
enums.

### §B.3 Deletion blockers

**Zero deletion blockers post-§A and §C migrations.** Both
`TypedBuffer<T>` and `AlignedTypedBuffer` are pure-internal carriers of
`TypedArrayData::*` / `HashMapValueBuf::*` arms. Once those enums are
deleted (Agent A + Agent C close), the file becomes orphan code and
the deletion is mechanical:

- Delete `crates/shape-value/src/typed_buffer.rs` entirely (485 LoC).
- Remove `pub use crate::typed_buffer::*` from `crates/shape-value/src/lib.rs`.
- Verify no surviving references via `grep -rn "typed_buffer::\|TypedBuffer<\|AlignedTypedBuffer" crates/`.

### §B.4 Validity bitmap consumer audit (audit §4.5 Obstacle O-4)

Audit §4.5 surfaced that `TypedBuffer<T>::validity: Option<Vec<u64>>` is
a bit-packed null bitmap used at the buffer storage layer. The migration
to `TypedArray<T>` loses this (flat-struct has no validity field, only
24-byte size assertion at `typed_array.rs:40-44`).

Re-verified at HEAD: the only consumers of the validity bitmap in
non-test code are inside `typed_buffer.rs` itself (`is_valid` /
`push_null` / `null_count` / `is_empty`-with-validity-check accessors).
Cross-crate consumer audit:

```
grep -rn "\.validity\b\|is_valid(\|push_null\b\|null_count\(\)" crates/ --include="*.rs" | grep -v typed_buffer.rs
```

Returns hits only in test fixtures + the v2-raw `TypedArray` smoke tests
that don't operate against the bitmap (the v2-raw carrier has no
bitmap). **The bitmap is unreferenced outside `typed_buffer.rs` itself.**

This is the audit §4.5 O-4.a finding restated: the bitmap is the
Arrow-CDI long-term target carrier (§6.4 deferral), not a live
consumer at HEAD. Migrating to `TypedArray<T>` drops a feature that
zero current consumers exercise.

**Wave 2 Agent A deletes the bitmap; no migration target needed.**

### §B.5 F64-SIMD alignment obstacle (audit §4.2 Obstacle O-2)

Audit §4.2 named the obstacle: `AlignedTypedBuffer` wraps `AlignedVec<f64>`
which aligns to 32-byte boundary (AVX2 `_mm256_load_pd` aligned-load).
The flat-struct `TypedArray<f64>` uses `Layout::array::<f64>(cap)`,
which yields 8-byte alignment.

Re-verified at HEAD `aa047356`:

```
crates/shape-value/src/aligned_vec.rs (the AlignedVec definition)
crates/shape-value/src/typed_buffer.rs:240-260 (AlignedTypedBuffer::from_aligned, with_capacity)
```

Used by every F64 array math operation in
`crates/shape-vm/src/executor/objects/typed_array_methods.rs` —
`v2_float_sum`, `v2_float_mean`, etc.

**Audit recommendation O-2.a** (per-T alignment parameter) requires
extending `TypedArray<T>::with_capacity`'s `Layout::array::<T>(cap)` to
`Layout::from_size_align(cap * size_of::<T>(), <align>).unwrap()`
with a per-T `ALIGN` constant. The runtime-v2-spec.md TypedArray<T>
24-byte contract is preserved (the alignment is on the data buffer, not
the carrier struct).

**Audit recommendation O-2.c** (accept unaligned loads, ~10-20% perf
hit on numeric kernels) is the discipline-coherent fallback if O-2.a
is rejected.

**Audit cluster-1 verdict**: the choice between O-2.a / O-2.c is a
**performance** decision, not a structural one. Wave 2 Agent A1's
default is **O-2.a** (extend `with_capacity` to honor per-T alignment).
If Agent A1 reports the change scope exceeds territory, **O-2.c** is
the surface-and-stop fallback. Either decision preserves the deletion
of `AlignedTypedBuffer` itself.

### §B.6 Cascade-site count estimate

- **`TypedBuffer<T>` references**: 144 across 26 files. Every reference
  is either (a) inside `TypedArrayData::X(Arc<TypedBuffer<...>>)` /
  `HashMapValueBuf::X(Arc<TypedBuffer<...>>)` variant access — deletes
  trivially with the parent enum, OR (b) inside `typed_buffer.rs`
  itself.
- **`AlignedTypedBuffer` references**: 123 across 27 files. Same shape
  + the F64-SIMD-specific paths in
  `executor/objects/typed_number_array_methods.rs`,
  `executor/objects/typed_array_methods.rs`,
  `executor/builtins/array_ops.rs`. All consumers are inside
  `TypedArrayData::F64(Arc<AlignedTypedBuffer>)` pattern matches —
  delete with parent.

**Total cascade**: deletion is mechanical post-§A + §C. ~485 LoC of
typed_buffer.rs + ~270 cascade hits at consumer sites that are
already touched by Agent A / C's enum deletion. **No separate
Wave 2 agent needed for §B** — it's part of Agent A's close.

---

## §C — `HashMapValueBuf` enum (audit §5 Q25.B parallel deletion)

### §C.1 Definition + audit-§5 disposition

`pub enum HashMapValueBuf` lives at
`crates/shape-value/src/heap_value.rs:529-552` (13 variants, mirror of
TypedArrayData minus FloatSlice/Matrix). The shape:

```
crates/shape-value/src/heap_value.rs:529:pub enum HashMapValueBuf {
    I64, F64, Bool, String, Decimal, BigInt, DateTime, Timespan, Duration,
    Instant, Char, TypedObject, TraitObject  (13 variants)
}
```

Used as the values-payload field of `HashMapData`:

```
crates/shape-value/src/heap_value.rs:701:    pub values: HashMapValueBuf,
```

### §C.2 Reference counts at HEAD `aa047356`

```
HashMapValueBuf references: 111 hits across 4 files
  - crates/shape-value/src/heap_value.rs (definition + impl Clone + value_at + specialize_values)
  - crates/shape-vm/src/executor/objects/hashmap_methods.rs (the HashMap stdlib methods)
  - crates/shape-vm/src/executor/printing.rs (Display impl)
  - crates/shape-vm/src/executor/trait_object_ops.rs (one match arm)
```

Compared to TypedArrayData's 53 files, HashMapValueBuf has a much
**narrower consumer footprint** — confined to the HashMap stdlib
surface + Display + a single trait-object operation.

### §C.3 Producer ground-truth (R20 S2-prime §4.1.A.3 confirmation)

```
grep -rnE "HashMapValueBuf::\w+\(Arc::new" crates/ --include="*.rs"
```

Returns **12 hits — all inside `crates/shape-value/src/heap_value.rs`**
(the `specialize_values` factory at lines 763-843 + the
`HashMapData::new()` default at line 720).

**Per-variant external producer counts**: **ZERO** for every variant.

```
=== HashMapValueBuf producer breakdown by variant ===
  I64: 0
  F64: 0
  Bool: 0
  String: 0
  Decimal: 0
  BigInt: 0
  DateTime: 0
  Timespan: 0
  Duration: 0
  Instant: 0
  Char: 0
  TypedObject: 0
  TraitObject: 0
```

The only producer pathway is via `HashMapData::from_pairs` (which calls
the private `specialize_values` factory) at `heap_value.rs:751` and
`HashMapData::new()` at `heap_value.rs:720` (defaults to empty
TypedObject buf).

`HashMapData::from_pairs` external callers (the entry points for
HashMap construction):

```
crates/shape-runtime/src/stdlib/xml.rs:82
crates/shape-runtime/src/stdlib/json.rs:152
crates/shape-vm/src/executor/objects/hashmap_methods.rs:610/643/735/738
crates/shape-vm/src/executor/objects/array_transform.rs:1796
crates/shape-vm/src/executor/vm_impl/builtins.rs:709 (HashMapData::new)
```

8 external entry points. Migration target is identical for all.

### §C.4 Per-V monomorphization migration design

Per audit §5.2 the choice was:

- **(a) Generic `HashMapData<V>`** — discipline-coherent
  monomorphization. Every HashMap site instantiates with a concrete
  per-V Rust type.
- **(b) Inline `NativeKind` discriminator field** on `HashMapData` —
  same shape as the v2-raw `stamp_elem_type` byte, just on the parent
  struct.

**Audit recommendation: (a) Generic `HashMapData<V>`.**

Rationale (ADR-005 §1 single-discriminator + ADR-006 §2.7.5 stamp-at-
compile-time):

1. Option (b) reintroduces a runtime-kind-discriminator field on a
   container struct. Every operation on `values` reads the tag,
   dispatches the per-kind read body. This is the same shape as the
   deleted `UnifiedArray` ELEMENT_KIND byte (W10-misc deletion, §2.7.14)
   — kind on heap, runtime dispatch. Refuse on sight.
2. Option (a) generic `HashMapData<V>` monomorphizes at compile time;
   no runtime kind read; dispatch via Rust type system.

**The migration shape**:

```rust
// crates/shape-value/src/heap_value.rs (replacement)
pub struct HashMapData<V> {
    pub keys: *mut TypedArray<*const StringObj>,
    pub values: *mut TypedArray<V>,
    pub index: HashMap<u64, Vec<u32>>,
}
```

Per-V monomorphizations supported at landing (mirror of §A migration):
`i64`, `f64`, `u8` (Bool), `*const StringObj`, `*const DecimalObj`,
`*const TypedObjectStorage` (post-Agent-D), `*const TraitObjectStorage`
(post-Agent-E). DateTime / Timespan / Duration / Instant / Char are
dead per §C.5 below.

**`HeapValue::HashMap` arm shape change**: from
`HashMap(Arc<HashMapData>)` to `HashMap(Arc<HashMapData<V>>)` requires
the carrier to be enum-tag-discriminated. Two paths:

- **(a.1) `HeapValue::HashMap` becomes generic** — `HeapValue::HashMap(Arc<HashMapData<???>>)`
  where the `???` is the value kind. Requires the entire `HeapValue`
  enum to grow generic params, which cascades into every dispatch
  table — refuse on sight (parallel-discriminator anti-pattern).
- **(a.2) `HeapValue::HashMap` carries a `HashMapKindedRef`** that
  bundles a type-erased pointer + the `NativeKind` discriminator — same
  shape as `KindedSlot::from_typed_array(arc)` per ADR-006 §2.7.6 / Q8
  carrier-API-bound. The HashMap arm grows ONE new constructor per
  per-V monomorphization (the bound), heap dispatch goes through the
  parent kind label (`HeapKind::HashMap`), per-V dispatch happens at
  the method-handler tier where the call signature already determines V.

**Audit recommendation: (a.2) HashMapKindedRef**. Same shape as
`KindedSlot::from_typed_array`. Per-V monomorphization at the method
tier (where V is statically known from `HashMap<String, V>` type
annotation in the user's source). No new HeapKind variant (keeps
HeapKind::HashMap = 17 unchanged). No new sum type with parallel-to-
HeapKind variants.

### §C.5 Dead-arm finding (R20 S2-prime §4.1.A.3 mirror)

Per audit §4.1.A.3 the dead-arm finding for `TypedArrayData`'s temporal
family is mirrored verbatim in `HashMapValueBuf`:

- **HashMapValueBuf::DateTime / Timespan / Duration / Instant** —
  zero root producers; `HashMap<string, DateTime>` is not a reachable
  user-facing type today.
- **HashMapValueBuf::TraitObject** — zero producers (per heap_value.rs:637-648
  `unreachable!` arm in `value_at`).
- **HashMapValueBuf::F64 / Bool** — zero root producers per the
  `unreachable!` arms in `value_at` at lines 605-622 (the
  `specialize_values` factory has no F64 / Bool arms at the
  user-facing-construction layer).
- **HashMapValueBuf::Char** — has a `specialize_values` arm at
  heap_value.rs:838 (when `HeapValue::Char` is the first element), but
  no external user reaches this via `HashMap<string, char>` — the only
  way HashMapValueBuf gets a Char arm is if `HashMapData::from_pairs`
  receives a Vec<Arc<HeapValue::Char(...)>>` from xml/json marshalling.
  Treat as dead-but-derived (same disposition as TypedArrayData's
  TraitObject — delete with confidence at S5).

**Cluster-2 candidate** (not Wave 2): the `HashMap<string, V>` user-facing
type restrictions (string keys only per Q25.B last paragraph) is a
language-design decision that's orthogonal to this deletion. Out of
scope.

### §C.6 Cascade-site count estimate

- **HashMapValueBuf references**: 111 hits across 4 files.
- **HashMapValueBuf consumer match arms** (no `Arc::new`): ~98 hits
  (concentrated in `heap_value.rs` Clone / value_at / from_pairs +
  `hashmap_methods.rs` per-arm projection at lines 242-259 +
  `printing.rs` Display + `trait_object_ops.rs` single match).
- **HashMapData callers**: 8 external entry points (xml / json /
  hashmap_methods / array_transform / vm_impl/builtins).

**Total cascade**: ~110 sites. Well under R19 S1.5 ~100-site ceiling at
the per-file level. Single Wave 2 agent (Agent C) territory.

### §C.7 Wave 2 Agent C territory

| Aspect | Detail |
|---|---|
| Files owned exclusively | `crates/shape-value/src/heap_value.rs` (HashMapValueBuf def + HashMapData fields + impl + value_at + specialize_values + from_pairs) |
| Files touched (consumer cascade) | `crates/shape-vm/src/executor/objects/hashmap_methods.rs`, `executor/printing.rs`, `executor/trait_object_ops.rs`, `runtime/stdlib/xml.rs`, `runtime/stdlib/json.rs`, `vm_impl/builtins.rs`, `array_transform.rs:1796` |
| Close gate | `cargo check --workspace --lib --tests` exit 0; `bash scripts/verify-merge.sh` exit 0; `bash scripts/check-no-dynamic.sh` exit 0; AGENTS.md row appended; ADR amendment for §2.7.24 Q25.B added (mirror of §A's Q25.A SUPERSEDED text) |
| Inter-agent overlap | Touches `heap_value.rs` (Agent A also touches this for TypedArrayData deletion). Resolution: take-both at merge per established cadence; HashMapValueBuf section is separable from TypedArrayData section in source. |

### §C.8 ADR amendment owed at Agent C close

Drafted text (mirror of §A's Q25.A SUPERSEDED preamble):

```
##### Q25.B SUPERSEDED — `HashMapValueBuf` deletion (Wave 2 Agent C close, 2026-05-1X)

**Status:** Q25.B's `HashMapValueBuf` enum-tagged value-buffer
monomorphization (the body below this preamble) is **SUPERSEDED**.
The HashMapValueBuf enum at `crates/shape-value/src/heap_value.rs:529-552`
is **deleted** under the Wave 2 bulldozer cadence (strategic-owner
authorization 2026-05-14). `HashMapData` migrates to
`HashMapData<V>` per audit §C.4 option (a.2) — HashMapKindedRef
carrier with per-V monomorphization at the method tier; HeapValue::HashMap
gains a kinded constructor per the §2.7.6 / Q8 carrier-API-bound rule.

**Per-variant migration shape post-supersession:**

| Q25.B variant | Post-supersession replacement |
|---|---|
| I64 / F64 / Bool / String / Decimal / Char | per-V monomorphization in HashMapData<V> + TypedArray<V> values pointer |
| BigInt | DEFERRED to cluster-1+ (same as §A.3 Decimal cluster-1+ disposition) |
| DateTime / Timespan / Duration / Instant | DELETED — zero root producers per §C.5 audit finding |
| TypedObject | gated on Agent D close (TypedObjectStorage HeapHeader migration); migrates to TypedArray<*const TypedObjectStorage> per audit §2.2 row |
| TraitObject | gated on Agent E close (TraitObjectStorage HeapHeader migration); DEAD ARM (zero producers); typically merged-and-deleted with Agent E |

**Authority:** Phase 3 cluster-0+1 Wave 2 strategic-owner cadence shift
2026-05-14. Q25.B parallel-deletion target identified at audit §5 of
docs/cluster-audits/w12-typed-array-data-deletion-audit.md (Round 17,
2026-05-13); Wave 1 inventory (docs/cluster-audits/bulldozer-wave-1-
inventory.md §C, 2026-05-14) ground-truthed producer counts and named
the migration shape.

**Forbidden post-supersession** (extending Q25.E #2 list):

1. **Resurrection of `HashMapValueBuf` arms** under any rename
   ("Q25.B-inside-enum carriers retained", "documented intentional
   duality").
2. **`HashMapData::values: Arc<TypedBuffer<...>>` field shape** — the
   value-buffer carrier is now `*mut TypedArray<V>` per audit §C.4.
3. **HashMap-wide runtime kind discriminator on the parent struct** —
   per-V monomorphization at compile time via HashMapKindedRef carrier
   API; no inline tag byte on HashMapData itself.
```

---

## §D — `TypedObjectStorage` Arc → HeapHeader migration (audit §4.3 O-3)

### §D.1 Audit framing recap

Audit §4.3 named the obstacle: `HeapValue::TypedObject(Arc<TypedObjectStorage>)`
stores `Arc<TypedObjectStorage>` where refcount lives at standard Rust
Arc offset (`-16` from inner pointer). The `TypedObjectStorage` struct
has no `HeapHeader` field. Migrating `TypedArrayData::TypedObject` →
`TypedArray<*const TypedObjectStorage>` would store raw pointers to
TypedObjectStorage in the flat buffer; retain/release would have to use
Rust `Arc::increment_strong_count` / `decrement_strong_count` reading
at `-16` offset, NOT `v2_retain` / `v2_release` at `+0` offset like
every other v2-raw `<X>Obj` carrier.

**Audit recommendation: O-3.c defer** — TypedObjectStorage migration is
"multi-week scope of its own" + S3 / S5 deferred until it lands.

**Under bulldozer cadence (refuse #10)**: the deferral framing is
exactly what the cadence shift refuses. The audit §4.3 "multi-week"
framing assumed sequential careful work; with one parallel Wave 2 agent
+ verify-merge.sh 12/12 gate + 4-table HeapKind lockstep, the cost
calculation is different. Wave 1 disposition: **Agent D in-wave-2
territory** with the surface-and-stop fallback if it surfaces
something genuinely novel.

### §D.2 Producer counts + construction site map at HEAD `aa047356`

```
grep -rnE "TypedObjectStorage::new\b|TypedObjectStorage \{|HeapValue::TypedObject\(Arc::new\(TypedObjectStorage" crates/ --include="*.rs"
```

Returns 41 hits. Production `TypedObjectStorage::new(...)` construction
sites:

| File | Site | Construction context |
|---|---|---|
| `crates/shape-runtime/src/type_schema/mod.rs:275` | runtime type-schema construction | runtime |
| `crates/shape-runtime/src/stdlib/xml.rs:135` | xml element → typed-object marshal | runtime |
| `crates/shape-runtime/src/stdlib/csv_module.rs:273` | csv row → typed-object marshal | runtime |
| `crates/shape-runtime/src/stdlib/json.rs:173` | json object → typed-object marshal | runtime |
| `crates/shape-vm/src/executor/typed_object_ops.rs:842` | typed-object operations | vm |
| `crates/shape-vm/src/executor/objects/property_access.rs:907/992/1027` | property access write-back paths | vm |
| `crates/shape-vm/src/executor/builtins/object_ops.rs:164` | builtin object_ops | vm |
| `crates/shape-vm/src/executor/objects/datatable_methods/core.rs:389/438` | DataTable row materialization | vm |
| `crates/shape-vm/src/executor/loops/mod.rs:680` | for-loop body iter | vm |
| `crates/shape-vm/src/executor/vm_impl/modules.rs:427` | module export construction | vm |
| `crates/shape-jit/src/ffi/conversion.rs:763/789` | JIT FFI conversion (kinded slot → TypedObject) | jit |
| `crates/shape-jit/src/ffi/value_ffi.rs:709` | JIT value FFI | jit |
| `crates/shape-vm/src/executor/objects/object_creation.rs:171` | `op_new_typed_object` opcode handler | vm (canonical) |
| `crates/shape-vm/src/executor/exceptions/mod.rs:496` | exception construction | vm |
| `crates/shape-vm/src/executor/objects/object_operations.rs:227/248/268/290` | object operations helpers | vm |
| `crates/shape-value/src/kinded_slot.rs:1183` | KindedSlot constructor (test fixture) | test |
| `crates/shape-value/src/heap_value.rs:4338/4361/4377/4393/4399/4438/4454/4465/5374` | self-tests | test |

**Distinct production construction sites**: ~18 (excluding heap_value.rs
self-tests). Total references: 307 across 47 files (the consumer
match-arm side).

### §D.3 JIT field-access fast-path locations

```
crates/shape-jit/src/ffi/typed_object/field_access.rs
crates/shape-jit/src/ffi/typed_object/allocation.rs
crates/shape-jit/src/ffi/typed_object/ffi_exports.rs
crates/shape-jit/src/ffi/typed_object/ (entire dir)
crates/shape-jit/src/ffi_symbols/object_symbols.rs
crates/shape-jit/src/compiler/ffi_builder.rs:186
crates/shape-jit/src/mir_compiler/places.rs:706
crates/shape-jit/src/ffi/conversion.rs (line 37-194 type-check arms)
```

Key entry points:

- `jit_typed_object_get_field(obj_bits: u64, offset: u64) -> u64` at
  `crates/shape-jit/src/ffi/typed_object/field_access.rs:139` — the
  read-field FFI fast path.
- `jit_typed_object_set_field(obj_bits: u64, offset: u64, value: u64) -> u64`
  at `field_access.rs:176`.
- `jit_typed_object_schema_id` (lookup function) at the same dir.

These FFI entry points take `obj_bits: u64` as the
`Arc::into_raw(Arc<TypedObjectStorage>) as u64` pointer (per ADR-006
§2.4 / Q6). Under O-3.a migration: the same FFI entry points take the
same `obj_bits: u64`, but the pointer is to a v2-raw HeapHeader-
equipped `TypedObjectStorage` (header at offset 0, schema_id / slots /
heap_mask after). Field offsets are unchanged at the per-schema layer
(`schema.field_offset(name)` returns the byte offset from
`schema_id`/`slots`/`heap_mask` start, which becomes the offset from
HeapHeader's offset 8).

**Field-access changes minimally** — the FFI signature is unchanged;
the field-offset computation shifts by `size_of::<HeapHeader>() = 8`
to skip the header. Cranelift `iadd_imm` + `load` chain is preserved.

### §D.4 Drop dispatch site (audit's hardest contract change)

`TypedObjectStorage::drop` at `crates/shape-value/src/heap_value.rs:2531`:

```rust
impl Drop for TypedObjectStorage {
    fn drop(&mut self) {
        // walks heap_mask, dispatches per-field Arc::decrement_strong_count
        // for HeapKind fields; per-HeapKind arms in match
    }
}
```

This `Drop` impl already exists as one of the four canonical 4-table
lockstep dispatch tables per ADR-006 §2.7.6 / Q8. Per the migration:

- The `Drop` impl moves from `impl Drop for TypedObjectStorage` to a
  manual `v2_TypedObjectStorage_drop(ptr: *mut TypedObjectStorage)` free
  function (analogous to `StringObj::drop` at
  `crates/shape-value/src/v2/string_obj.rs:89-99`).
- The `Arc::drop` integration goes away (no more Rust Arc lifecycle);
  refcount logic moves to the HeapHeader's `kind`/`refcount` reads.
- The per-field heap-mask walk is identical — same iteration shape,
  same per-HeapKind arms.

### §D.5 Refcount semantics audit

**Current state:**

- `Arc<TypedObjectStorage>` stores TypedObjectStorage on the heap with
  Rust Arc's `ArcInner { strong: AtomicUsize, weak: AtomicUsize, value: T }`
  header at offset -16 (assuming 8-byte usize).
- `Arc::into_raw(arc) as u64` returns the pointer to the inner value
  (post-header), so `slot.bits = Arc::into_raw(...)`.
- `Arc::clone(arc)` uses the header's strong count via Rust stdlib.
- `Arc::drop(arc)` triggers `TypedObjectStorage::drop` when refcount
  reaches 0.

**Post-migration state:**

- `*mut TypedObjectStorage` stores TypedObjectStorage on the heap with
  HeapHeader at offset 0 (8 bytes: AtomicU32 refcount + u16 kind + u8 flags).
- `slot.bits = ptr as u64` (no Arc indirection).
- `v2_retain(&(*ptr).header)` for clone, `v2_release(&(*ptr).header)`
  for drop.
- `v2_release` returning true triggers manual
  `v2_TypedObjectStorage_drop(ptr)` per StringObj precedent.

The `HeapElement` trait (per audit §4.1.B option (a) ratified at R20
S2-prime) is the integration point:

```rust
unsafe impl HeapElement for TypedObjectStorage {
    unsafe fn release_elem(ptr: *const Self) {
        if unsafe { v2_release(&(*ptr).header) } {
            unsafe { v2_TypedObjectStorage_drop(ptr as *mut Self) };
        }
    }
}
```

The trait file exists at `crates/shape-value/src/v2/heap_element.rs`
(per the `ls` check); StringObj and DecimalObj already impl it.

**Audit §4.1.B's O-3.b refusal scoped correctly**: the audit §4.1.B
"O-3.b refused — perpetuates Arc-vs-HeapHeader duality" objection was
ABOUT TypedObjectStorage specifically (since it's `Arc<>`-wrapped, not
HeapHeader-equipped). Under O-3.a migration, TypedObjectStorage BECOMES
HeapHeader-equipped — the duality objection evaporates.

### §D.6 Per-construction-site migration mechanics

Every `TypedObjectStorage::new(...)` construction site at §D.2 changes
from:

```rust
let storage = TypedObjectStorage::new(schema_id, slots, heap_mask);
let arc = Arc::new(storage);
slot = ValueSlot::from_typed_object(arc);
```

to:

```rust
let ptr = v2_TypedObjectStorage_new(schema_id, slots, heap_mask);
slot = ValueSlot::from_typed_object_raw(ptr);
```

Where `v2_TypedObjectStorage_new` is the heap allocator (Layout-based,
mirror of `DecimalObj::new` at `decimal_obj.rs`).

**18 production sites × ~3 LoC each = ~54 LoC across the construction-
site cascade.** The `ValueSlot::from_typed_object` constructor changes
its signature from `Arc<TypedObjectStorage>` to `*mut TypedObjectStorage`
— that's the single canonical migration point per ADR-006 §2.4.

### §D.7 Cascade-site count estimate

- **Direct construction sites**: 18 production + 18 test/self-test = 36 sites
- **JIT FFI sites**: 6-8 (field_access + allocation + ffi_exports + value_ffi + conversion)
- **Drop / Clone / Display / debug arms**: ~20 (one per 4-table lockstep + printing.rs / json_value.rs)
- **`ValueSlot::from_typed_object` callers**: ~12 (the slot constructor's call sites)
- **Match arm consumer cascade** (read-side TypedObjectStorage references in match expressions): ~250 sites across 47 files

**Total**: ~300 sites. **Exceeds R19 S1.5 ~100-site ceiling at the
per-file level.** Per the dispatch's surface-and-stop discipline: this
is a structural-obstacle surface; supervisor disposition required.

**Audit recommendation**: split Agent D into TWO sub-agents:

- **Agent D1 — TypedObjectStorage shape change** (the storage struct
  grows HeapHeader at offset 0 + manual `_new` / `_drop` functions +
  `impl HeapElement`). Territory: `crates/shape-value/src/heap_value.rs`
  (TypedObjectStorage def + impl + Drop) + `crates/shape-value/src/
  kinded_slot.rs` (from_typed_object constructor + 4-table lockstep
  dispatch). ~50 sites. **Wave 2 dispatchable.**
- **Agent D2 — TypedObjectStorage construction site cascade**
  (18 production sites + JIT FFI sites + consumer match arm
  re-pattern). Territory: every file at §D.2 minus shape-value. ~250
  sites. **Wave 2 dispatchable** if Agent D1 closes first (Agent D2
  consumes D1's API surface). Sequential within Wave 2 (Agent D1
  dispatches first round; Agent D2 second round in the same wave).

The audit §4.3 "multi-week" framing assumed both halves done by one
agent sequentially. Under bulldozer cadence + the D1/D2 split, the
two halves dispatch in one Wave 2 (D1 round 1, D2 round 2, both close
within wave 2 budget).

### §D.8 ADR-fit confirmation

ADR-006 §2.3 (HeapValue payloads — typed Arc) at line 198 is the
governing rule: `HeapValue::TypedObject(Arc<TypedObjectStorage>)`. The
audit O-3.a migration changes this to
`HeapValue::TypedObject(/* raw pointer carrier */)` via the per-T
constructor pattern at §2.4. The §2.3 amendment text is owed at Agent
D1 close:

```
**§2.3 amendment (Wave 2 Agent D1, 2026-05-1X):** `HeapValue::TypedObject`
shifts from `Arc<TypedObjectStorage>` to a raw v2-raw pointer carrier
per the audit §4.3 O-3.a migration (TypedObjectStorage grows HeapHeader
at offset 0; refcount on header via v2_retain / v2_release per StringObj
precedent). Per-FieldType constructor at `ValueSlot::from_typed_object`
changes signature from `Arc<TypedObjectStorage>` to
`*mut TypedObjectStorage`. The `from_typed_object_arc` legacy entry is
deprecated (transitional during Wave 2 dispatch; deleted when last
caller migrates).
```

### §D.9 Forbidden patterns this migration rules out

- **"Preserve Arc-vs-HeapHeader duality"** framing — the migration's
  whole point is unifying the discipline.
- **bridge / probe / helper / hop / translator / adapter / shim**
  framings for the Arc → HeapHeader migration.
- **"Per-element retain dispatch on TypedArray<*const TypedObjectStorage>"**
  via a runtime branch — refused per audit §4.1.B option (c). The
  retain happens via `HeapElement::release_elem` monomorphized at the
  trait layer.

---

## §E — `TraitObjectStorage` HeapHeader migration (audit §4.3 O-3a)

### §E.1 Definition

`pub struct TraitObjectStorage` lives at
`crates/shape-value/src/heap_value.rs:1963-1977`:

```
pub struct TraitObjectStorage {
    pub value: Arc<TypedObjectStorage>,
    pub vtable: Arc<crate::value::VTable>,
}
```

A **fat pointer** with two `Arc<>` fields. Both Arcs are refcount-owned
externally (Rust stdlib Arc).

### §E.2 Reference counts at HEAD `aa047356`

```
TraitObjectStorage references: 113 hits across 16 files
TraitObjectStorage::new constructors: 7 production sites + self-tests
```

Production constructors:

| File | Site |
|---|---|
| `crates/shape-vm/src/executor/trait_object_ops.rs:183` | the canonical op_box site (creates TraitObjectStorage from typed_object_arc + vtable) |
| `crates/shape-vm/src/executor/trait_object_ops.rs:864/1106` | thunk-related construction paths |
| (other constructors are inside self-tests in heap_value.rs:5349-5587, ~12 hits) | tests |

### §E.3 Vtable refcount-share analysis

The `vtable: Arc<VTable>` field is **shared across all
TraitObjectStorage instances built from the same `(impl Trait for Type)`
pair** per Q25.C.5 docstring (`heap_value.rs:1971-1975`). VTable
construction is once per impl, the resulting `Arc<VTable>` is cached
and cloned into each boxing site. IC stabilizes on
`Arc::as_ptr(&vtable)` per §Q25.C.6.

Under O-3a migration, two paths:

- **(E-a) TraitObjectStorage grows HeapHeader, both inner Arcs become raw pointers.**
  ```rust
  #[repr(C)]
  pub struct TraitObjectStorage {
      pub header: HeapHeader,                       // 8 bytes
      pub value: *mut TypedObjectStorage,           // 8 bytes (v2-raw)
      pub vtable: *const VTable,                    // 8 bytes
  }
  // Total: 24 bytes — matches the TypedArray<T> shape contract.
  ```
  The inner pointers' retain/release uses `v2_retain` /
  `v2_release` for the value (post Agent D), and a new `VTable`
  retain/release for the vtable.
- **(E-b) TraitObjectStorage grows HeapHeader, value pointer becomes raw, vtable stays Arc.**
  ```rust
  #[repr(C)]
  pub struct TraitObjectStorage {
      pub header: HeapHeader,
      pub value: *mut TypedObjectStorage,
      pub vtable_arc: Arc<VTable>,  // 16 bytes (Arc fat pointer)
  }
  // Total: 32 bytes (not 24).
  ```
  Preserves Rust Arc lifecycle for the vtable; mixed retain shape
  inside TraitObjectStorage. **Refuse** — same "Arc-vs-HeapHeader
  duality" objection as O-3 audit §4.3.

**Audit recommendation: E-a — both inner pointers raw.** VTable also
grows a `HeapHeader` field at offset 0 (or a separate refcount header
specific to VTable's pre-existing lifecycle) and becomes
HeapElement-impl-able. The VTable's refcount semantics are already
"shared, cloned-into-each-boxing-site" per Q25.C.5 docstring — that's
exactly what `v2_retain` does post-migration.

### §E.4 Producer migration

Production sites (per §E.2):

- `crates/shape-vm/src/executor/trait_object_ops.rs:183`:
  ```rust
  // Before:
  let trait_object = Arc::new(TraitObjectStorage::new(typed_object_arc, vtable));
  // After:
  let trait_object = v2_TraitObjectStorage_new(typed_object_ptr, vtable_ptr);
  ```
- `trait_object_ops.rs:864/1106` — same shape.

3 production sites total. Bounded.

### §E.5 Cascade-site count estimate

- **Direct construction sites**: 3 production + 12 self-test = 15
- **JIT FFI sites**: TraitObject is currently not in the JIT path
  (per the `HeapKind::TraitObject = 29` ordinal added in Wave 17 W17-
  trait-object-storage; the JIT consumer migration is open per §I
  Surface A scope estimate). Per Q25.C.6 IC devirtualization, the JIT
  needs `Arc::as_ptr(&vtable)` stabilization — under E-a this becomes
  `vtable_ptr as u64` directly (cheaper).
- **Drop / Clone / Display arms**: ~10 (one per 4-table lockstep +
  printing.rs / kinded_slot.rs)
- **Match arm consumer cascade**: ~50 across the 16 files

**Total**: ~80 sites. Within R19 S1.5 ~100-site ceiling. Single Wave 2
agent (Agent E).

Wave 2 Agent E is **gated on Agent D close** (Agent E needs
TypedObjectStorage to be HeapHeader-equipped before TraitObjectStorage
can store `*mut TypedObjectStorage`). Sequential dispatch within wave.

### §E.6 ADR amendment owed at Agent E close

Mirror of §D.8 amendment but for §2.3 / §Q25.C.5:

```
**Q25.C.5 amendment (Wave 2 Agent E, 2026-05-1X):** TraitObjectStorage's
fat-pointer shape changes from `{ value: Arc<TypedObjectStorage>,
vtable: Arc<VTable> }` to a v2-raw `#[repr(C)]` struct with HeapHeader
at offset 0 + `*mut TypedObjectStorage` + `*const VTable` (24 bytes
matching the TypedArray<T> size contract). Refcount semantics: v2_retain
/ v2_release on the TraitObjectStorage header; per-field retain dispatch
on value (via TypedObjectStorage::HeapElement impl) and vtable (via
VTable's new HeapElement impl). The IC devirtualization
identity-check at Q25.C.6 changes from `Arc::ptr_eq(&self.vtable,
&other.vtable)` to `self.vtable == other.vtable` (raw pointer
comparison, same semantics, slightly cheaper).
```

### §E.7 Forbidden patterns

Same as §D.9.

---

## §F — Q25.A specialization dead arms wholesale deletion

### §F.1 Scope

Per audit §4.1.A.2 + this Wave 1 audit §A.2.2 ground-truth:

- **TypedArrayData::DateTime / Timespan / Duration / Instant arms** —
  zero root producers, only derived sites in array_transform.rs and
  concat.rs that pattern-match on the variant.
- **TypedArrayData::TraitObject arm** — zero root producers, only
  derived sites.
- **HashMapValueBuf::DateTime / Timespan / Duration / Instant arms** —
  zero producers + zero consumers reachable from user code (per §C.5).
- **HashMapValueBuf::TraitObject arm** — `unreachable!` in `value_at`
  per `heap_value.rs:637-648`.

### §F.2 Migration shape — wholesale deletion

These arms have:

- Zero producer migration needed (no live code creates them).
- Consumer-arm deletions are exhaustive-match cleanup; the
  array_transform / concat / printing / wire_conversion / json_value
  arms become "no-op delete" since the corresponding TypedArrayData
  arm is gone.

The arms come back as either:

- (a) **DELETE OUTRIGHT** if Agent A's TypedArrayData deletion covers
  the full enum (post Wave 2 close).
- (b) **DELETE IN AGENT F's STANDALONE TERRITORY** if Agent F closes
  first (smallest agent, fastest close — these are clean dead-arm
  deletions).

**Audit recommendation: option (b) Agent F standalone**. Agent F has
the smallest territory:

- 4-5 enum arm deletions (TypedArrayData::DateTime/Timespan/Duration/
  Instant/TraitObject + HashMapValueBuf::DateTime/Timespan/Duration/
  Instant/TraitObject = 9 arms).
- ~200 consumer match-arm deletions across ~10 files
  (`array_transform.rs:585-1465`, `concat.rs:271-319`, `printing.rs`,
  `wire_conversion.rs`, `json_value.rs`, etc.).
- Closing fast unblocks Agent A's territory by reducing the
  TypedArrayData variant count from 22 to 17 (the live + structural-
  obstacle subset).

### §F.3 Cascade-site count estimate

~200 sites bounded. Single Wave 2 agent (Agent F). Should be the
fastest-closing agent in Wave 2.

### §F.4 ADR amendment

The Q25.A SUPERSEDED amendment text already names these as dead
(per §A.5 + audit §4.1.A.4 finding). Agent F's close commit doesn't
need a new amendment — it materially exercises the existing
amendment.

### §F.5 Forbidden patterns

- **"Preserve dead arm for forward-S5 cleanliness"** — refused. The
  Q25.A SUPERSEDED amendment already retired this framing. The dead-
  arm deletion is the cleanliness step, not a preservation.
- **"Add DateTimeObj / TimespanObj / InstantObj carriers for forward
  reachability"** — refused under bulldozer cadence (supervisor R20
  (A) Minimal disposition binding). If `Array<DateTime>` becomes
  user-facing reachable in a future cluster, the carrier lands then.

---

## §G — W12-stdlib-intrinsic-collapse (IntrinsicSum / `.sum()` PHF split-brain)

### §G.1 Background

The "7th defection-attractor instance from R18 close": stdlib `sum()`
has two parallel implementations — `BuiltinFunction::IntrinsicSum`
(compiled from `__intrinsic_sum(series)` in stdlib `pub fn sum`) AND
PHF method-dispatch entries `.sum()` registered for
TYPED_INT_ARRAY_METHODS / TYPED_NUMBER_ARRAY_METHODS / MATRIX_METHODS /
DATATABLE_METHODS / TYPED_FLOAT_ARRAY_METHODS.

This is a parallel-implementation defection (§Forbidden Patterns
"Parallel-implementation across producer/consumer carrier-shape
boundaries") at the dispatch tier — two different code paths produce
the same user-facing result.

### §G.2 Ground-truth at HEAD `aa047356`

**IntrinsicSum opcode references:**

```
crates/shape-runtime/src/intrinsics/mod.rs:129       — stdlib registration ("__intrinsic_sum" -> math::intrinsic_sum)
crates/shape-runtime/src/intrinsics/math.rs:185-187  — intrinsic_sum surface-and-stop body
crates/shape-vm/src/bytecode/opcode_defs.rs:2260     — BuiltinFunction::IntrinsicSum enum variant
crates/shape-vm/src/bytecode/opcode_defs.rs:2560     — BuiltinFunction byte registration
crates/shape-vm/src/compiler/helpers.rs:3855         — "__intrinsic_sum" -> BuiltinFunction::IntrinsicSum
crates/shape-vm/src/compiler/helpers.rs:4071         — exhaustive match arm
crates/shape-vm/src/compiler/expressions/function_calls.rs:101 — intrinsic name list
crates/shape-vm/src/compiler/functions.rs:2833-2875  — internal-scope tests
crates/shape-vm/src/executor/vm_impl/builtins.rs:472-586 — the IntrinsicSum opcode handler body
crates/shape-vm/src/executor/vm_impl/builtins.rs:1211-1378 — IntrinsicSum self-tests
```

**`.sum()` PHF method-handler references:**

```
crates/shape-vm/src/executor/objects/method_registry.rs:300  — "sum" => array_aggregation::handle_sum_v2     (HeapKind::TypedArray dispatch)
crates/shape-vm/src/executor/objects/method_registry.rs:365  — "sum" => datatable_methods::handle_sum         (DATATABLE_METHODS)
crates/shape-vm/src/executor/objects/method_registry.rs:656  — "sum" => matrix_methods::v2_sum               (MATRIX_METHODS)
crates/shape-vm/src/executor/objects/method_registry.rs:682  — "sum" => typed_array_methods::v2_float_sum    (TYPED_FLOAT_ARRAY_METHODS)
crates/shape-vm/src/executor/objects/method_registry.rs:719  — "sum" => typed_array_methods::v2_int_sum      (TYPED_INT_ARRAY_METHODS)
crates/shape-vm/src/executor/objects/method_registry.rs:761  — "sum" => typed_int_array_methods::sum         (per-element-kind I64 method)
crates/shape-vm/src/executor/objects/method_registry.rs:786  — "sum" => typed_number_array_methods::sum      (per-element-kind F64 method)
crates/shape-vm/src/executor/objects/datatable_methods/aggregation.rs:609 — "sum" => AggOp::Sum               (aggregation operator)
```

**Implementation body comparisons:**

- `IntrinsicSum` opcode handler (`builtins.rs:516-586`): pop_builtin_args,
  type-check receiver as `Ptr(HeapKind::TypedArray)`, recover
  `Arc<TypedArrayData>` via raw cast, match on `TypedArrayData::I64` →
  loop+wrapping_add OR `TypedArrayData::F64` → iter+sum, surface-and-
  stop on other arms.
- `v2_int_sum` (typed_array_methods.rs around line 700): mirror of
  IntrinsicSum I64 body.
- `v2_float_sum` (typed_array_methods.rs): mirror of IntrinsicSum F64 body.

**Audit finding — IntrinsicSum's body comment at builtins.rs:489-504
explicitly says "Dispatch on TypedArrayData::I64 / F64 / FloatSlice
mirrors `executor::objects::typed_array_methods::v2_int_sum` and
v2_float_sum exactly — same kernels".** The split-brain is documented
in the source.

### §G.3 Migration design — collapse to one path

Per ADR-005 §1 single-discriminator: the discriminator is the
NativeKind on the receiver slot; the PHF method dispatch is the
canonical handler tier (per ADR-006 §2.7.10 / Q11 MethodFnV2).
IntrinsicSum is a parallel handler that bypasses the canonical PHF
dispatch.

**Migration target**: delete `BuiltinFunction::IntrinsicSum` + the
opcode handler at `builtins.rs:516-586` + the self-tests at
`builtins.rs:1211-1378`. Route `__intrinsic_sum(series)` calls through
the PHF dispatch via `op_call_method("sum")` instead.

**The stdlib wrapper `pub fn sum(series) { __intrinsic_sum(series) }`**
(at the Shape source level) becomes:

```shape
pub fn sum(series) { series.sum() }
```

This routes through the canonical PHF method dispatch (per receiver's
NativeKind). The IntrinsicSum opcode + handler + tests are all
deleted.

### §G.4 Cascade-site count estimate

- Delete `BuiltinFunction::IntrinsicSum` variant + opcode byte
  (`opcode_defs.rs:2260/2560`) — 2 sites.
- Delete `__intrinsic_sum` mapping in `helpers.rs:3855` + exhaustive
  match arm at `:4071` — 2 sites.
- Delete intrinsic name list entry at
  `function_calls.rs:101` — 1 site.
- Delete the handler body + self-tests at `builtins.rs:472-586,1211-1378` — ~250 LoC.
- Delete `math::intrinsic_sum` at `intrinsics/math.rs:185-187` (surface-and-stop body) + registration at `intrinsics/mod.rs:129` — 5 LoC.
- Update stdlib `pub fn sum` to use method dispatch (one stdlib file edit).
- Self-test impact: ~12 self-tests at `builtins.rs:1289-1378` deleted (covered by `.sum()` PHF method handler self-tests).

**Total cascade**: ~10 dispatch sites + ~280 LoC body deletion + ~12
self-test deletions. **Well under R19 S1.5 ~100-site ceiling at the
per-file level.**

### §G.5 ADR-fit confirmation

ADR-005 §1 (single-discriminator) preserved post-collapse — `.sum()`
dispatch goes through the PHF method registry per receiver kind, no
parallel handler.

ADR-006 §2.7.10 / Q11 (MethodFnV2 ABI) is the canonical method-dispatch
path; IntrinsicSum's opcode-handler bypass is the deleted shape.

### §G.6 Wave 2 Agent G territory

| Aspect | Detail |
|---|---|
| Files exclusively touched | `crates/shape-runtime/src/intrinsics/math.rs`, `crates/shape-runtime/src/intrinsics/mod.rs`, `crates/shape-vm/src/executor/vm_impl/builtins.rs` (IntrinsicSum arm deletion + self-test deletion), `crates/shape-vm/src/bytecode/opcode_defs.rs` (variant + byte), `crates/shape-vm/src/compiler/helpers.rs` (3855 + 4071), `crates/shape-vm/src/compiler/expressions/function_calls.rs:101`, `crates/shape-vm/src/compiler/functions.rs:2833-2875` (test fixture cleanup) |
| Stdlib edit | `crates/shape-runtime/src/stdlib/<math-stdlib>.shape` — replace `__intrinsic_sum(series)` with `series.sum()` |
| Inter-agent overlap | None — IntrinsicSum is isolated from TypedArrayData / HashMapValueBuf surfaces |
| Close gate | `cargo check --workspace --lib --tests` exit 0 + verify-merge.sh 12/12 + AGENTS.md row |
| ADR amendment | None substantive — Q25.E's existing forbidden-pattern list covers parallel-implementation; this is the deletion of an instance |

### §G.7 Forbidden patterns

- **"Preserve IntrinsicSum as a fast-path optimization"** — refused.
  Per the IntrinsicSum body comment itself ("mirror of v2_int_sum"),
  the two paths execute identical bodies. Keeping the duplicate is
  parallel-implementation defection.
- **"Migrate via gradual cluster-N+1 plan"** — refuse #10 under
  bulldozer cadence.

---

## §H — Cross-tier shape-conversion for `Array<string>` / `Array<decimal>` v2-raw read path

### §H.1 The R20 prereq 2 problem

R20 S2-prime-production close shipped `StringObj` + `DecimalObj`
carriers (HeapHeader at offset 0, raw allocator + drop + HeapElement
impl). To migrate `Array<string>` / `Array<decimal>` producers to
`TypedArray<*const StringObj>` / `TypedArray<*const DecimalObj>`, the
**read path** (`arr[i]` returns an element) needs a design decision:
when the user does `let s: string = arr[0]`, the slot bits being
delivered to `s` are currently `Arc::into_raw(Arc<String>) as u64` (per
ADR-006 §2.4 `ValueSlot::from_string_arc`). Under v2-raw, the array
holds `*const StringObj` pointers, not Arc<String>.

Three design shapes surfaced in the dispatch prompt:

- **(H-a) Materialize-on-read** — `arr[i]` reads `*const StringObj`,
  bumps its refcount via `v2_retain`, materializes a `String` from it
  + wraps in `Arc<String>` + pushes a `NativeKind::String` slot with
  `ValueSlot::from_string_arc(arc)`. The slot type is unchanged from
  the caller's perspective (NativeKind::String).
- **(H-b) Push-pointer-shape** — `arr[i]` reads `*const StringObj`,
  bumps refcount, pushes the raw pointer with a new
  `NativeKind::StringV2` discriminator (or `NativeKind::Ptr(HeapKind::String)`
  if we accept reusing). Caller has the v2-raw pointer; downstream
  code uses `v2_release` for drop.
- **(H-c) New NativeKind variant** — `NativeKind::StringV2` /
  `NativeKind::DecimalV2` as full first-class scalar variants under
  ADR-006 §2.7.5 + Q8 carrier-API-bound. Caller migrates per
  monomorphization at compile time.

### §H.2 Per-shape cascade count analysis

**(H-a) Materialize-on-read:**

- The conversion `*const StringObj` → `Arc<String>` requires
  reconstructing UTF-8 bytes from the StringObj's `data: *const u8` +
  `len: u32` fields, wrapping in a new `String::from_utf8_unchecked(...)`
  + `Arc::new(...)`. Allocation per read.
- Refcount: bump StringObj on read (the array still owns the share);
  the new `Arc<String>` owns a separate Rust-Arc-managed share. **Two
  parallel refcount lifecycles** for what semantically is one string.
- Read cost: 1 byte copy (memcpy from `data` to new String) + 2 atomic
  increments (StringObj retain + Arc::new strong-count init) +
  Layout::array<u8> alloc + Rust string conversion.
- Cascade: zero new opcodes, zero new NativeKind variants, zero new
  type-tracker entries. Pure runtime conversion.
- **Hidden cost**: every `arr[i]` allocates. For a 1M-element string-
  iteration loop, this is 1M heap allocations + 2M atomic increments.

**(H-b) Push-pointer-shape (reuse `NativeKind::Ptr(HeapKind::String)`):**

- Slot bits = `ptr as u64` (no Arc conversion).
- Caller sees `NativeKind::Ptr(HeapKind::String)` and dispatches the
  v2-raw lifecycle (v2_retain / v2_release).
- **Conflict**: `NativeKind::Ptr(HeapKind::String)` is already used by
  the legacy `Arc::into_raw(Arc<String>)` carrier per ADR-006 §2.4
  (`ValueSlot::from_string_arc`). Mixing the two carrier shapes under
  the same NativeKind = parallel-carrier defection at the discriminator
  layer. **REFUSE.**

**(H-c) New NativeKind variant — `NativeKind::StringV2` / `NativeKind::DecimalV2`:**

- Slot bits = `ptr as u64`, kind = `NativeKind::StringV2`.
- Caller's downstream code dispatches the v2-raw lifecycle.
- Cascade per the R19 S1.5 NativeKind addition precedent (F32 + Char):
  - `NativeKind` enum (`crates/shape-value/src/native_kind.rs`) — 1 site per new variant
  - 4-table HeapKind lockstep — but these aren't HeapKind variants (they're NativeKind scalars per the F32 / Char precedent), so the 4-table lockstep doesn't apply
  - `ConcreteType` cascade — `ConcreteType::StringV2` / `ConcreteType::DecimalV2` arms in `crates/shape-value/src/v2/concrete_type.rs` + ~22 cascade sites (`stack_size` / `field_size` / `alignment` / `is_integer_family` / `is_floating_family` / `Display` per R19 S1.5 estimate)
  - Type-tracker arms — `crates/shape-vm/src/compiler/type_tracking.rs` — ~12 sites
  - VM stack parallel-kind dispatch — `executor/vm_impl/stack.rs::clone_with_kind / drop_with_kind` — 2 arms × 2 new variants = 4 sites
  - JIT FFI — `crates/shape-jit/src/ffi/value_ffi.rs` HK_* constants + `jit_kinds.rs` — ~5 sites
  - KindedSlot constructors — `kinded_slot.rs` `from_string_v2` / `from_decimal_v2` constructors — 2 sites
  - Bytecode emission — `compiler/v2_typed_emission.rs` per-kind dispatch — ~6 sites
- **Total cascade**: ~50 sites per new variant × 2 variants = ~100 sites. **At the R19 S1.5 ~100-site ceiling** but below it.

### §H.3 Read-cost estimates

| Shape | Per-read cost (cycles, x86_64 estimate) | Allocation per read | Atomic ops |
|---|---|---|---|
| (H-a) Materialize | ~200-500 cycles (alloc + memcpy + Arc::new) | YES (Rust String + Arc inner) | 2 (StringObj retain + Arc init) |
| (H-c) New NativeKind | ~10 cycles (slot.bits = ptr + kind discriminator stamp) | NO | 1 (StringObj retain on read = clone share) |

(H-c) is **20-50x cheaper** per read. For numeric-heavy programs
operating on `Array<decimal>` (e.g. financial analytics over a 1M-row
table), the difference is 200ms vs 10ms per pass.

### §H.4 Audit recommendation

**(H-c) New NativeKind variant — StringV2 / DecimalV2.**

Rationale (ADR-006 §2.7.5 stamp-at-compile-time + ADR-006 §2.7.6 / Q8
carrier-API-bound):

1. **Performance**: 20-50x faster per read on heap-element arrays.
2. **Cascade-site count**: ~100 sites under the R19 S1.5 ceiling.
3. **Discipline-coherent**: NativeKind cardinality grows by 2 (F32 +
   Char precedent at R19 S1.5 grew it by 2 already); the new variants
   are explicit per-carrier-shape discriminators, not parallel-
   discriminators of HeapKind.
4. **Q8 bound preserved**: 2 new constructors on KindedSlot
   (`from_string_v2_ptr` / `from_decimal_v2_ptr`) — under the bound.
5. **Avoids the H-b defection**: NativeKind::Ptr(HeapKind::String)
   keeps its legacy Arc-wrapped meaning; the new variants are
   structurally distinct.

**Audit recommendation: ratification gate at Wave 2 Agent B (String +
Decimal producer migration) opens with the NativeKind::StringV2 /
DecimalV2 amendment in the ADR-006 §2.7.5 + Q8 cardinality bump.**

The amendment requires supervisor ratification (Q8 cardinality bump
is a §2.7.6 amendment per the existing rule). Wave 1 audit surfaces
the amendment text; supervisor disposes; Wave 2 Agent B's close
commit lands it.

### §H.5 Drafted ADR amendment text (audit deliverable for §H)

Drafted for §2.7.5 (Cross-crate ABI policy + scalar NativeKind
amendment):

```
##### §2.7.5 amendment (Wave 2 Agent B, 2026-05-1X) — `NativeKind::StringV2` / `NativeKind::DecimalV2` per-carrier discriminators

Per audit §H (Wave 1 bulldozer-wave-1-inventory.md, 2026-05-14): the
v2-raw carrier shift for `Array<string>` / `Array<decimal>` read path
introduces two new `NativeKind` scalar variants — `StringV2` and
`DecimalV2` — discriminating v2-raw `*const StringObj` / `*const DecimalObj`
pointers from the legacy `Arc::into_raw(Arc<String>) / Arc<Decimal>`
carriers (NativeKind::String / NativeKind::Decimal).

**Justification:** materialize-on-read (option H-a in the audit) costs
1 heap allocation + 2 atomic ops per array element read. Push-pointer-
shape on the existing NativeKind::Ptr(HeapKind::String) (option H-b)
reuses a discriminator with two distinct carrier shapes — parallel-
carrier defection at the discriminator layer; refused. New NativeKind
variants (option H-c) carry the carrier-shape discrimination explicitly
at compile time, preserve §2.7.5 stamp-at-compile-time, and cost ~10
cycles per read (20-50x faster than materialize).

**Cardinality bound (Q8):** the addition of 2 new variants follows the
R19 S1.5 precedent (F32 + Char added 2 variants); per-NativeKind
KindedSlot constructor count is under the bound (StringV2 +
DecimalV2 each add 1 constructor — `from_string_v2_ptr` /
`from_decimal_v2_ptr` — no per-heap-variant accessors per §2.7.6 / Q8
rule).

**Migration cadence:** producer side (Wave 2 Agent B) emits StringV2 /
DecimalV2 slots from `op_typed_array_get` when the element kind is the
v2-raw carrier. Consumer side reads via per-NativeKind dispatch at the
method tier (the StringV2-bearing slot pointing to a StringObj is
operated on by `str_methods` PHF entries that know the v2-raw
lifecycle — `v2_retain` / `v2_release` for refcount).

**Forbidden:**

1. **Mixing v2-raw and Arc-wrapped slots under the same NativeKind**
   (the H-b defection — refused).
2. **Conversion-on-the-fly at the method-handler boundary** ("Arc
   transparency" / "shape unification at the call site") — the carrier
   discriminator stays at the slot layer; downstream code dispatches
   per discriminator.
3. **Renaming StringV2 / DecimalV2 to suggest a generic role** ("v2-
   wrapper", "boundary-discriminator", "decode-carrier") — refused per
   the broader-family regex.
```

---

## §I — Surface A: kickoff-prompt-vs-fixture mismatch

### §I.1 Ground-truth at HEAD `aa047356`

**Kickoff prompt prose** (`docs/cluster-audits/phase-3-kickoff-prompt.md:102-105`):

```
trait T { fn name(&self) -> String }
impl T for X { fn name(&self) -> String { "x" } }
let t: dyn T = box(X{})
print(t.name())                        # x
```

**Smoke fixture** (`/tmp/smokes/s3.shape`):

```
trait T { name(): string }
type X {}
impl T for X { method name() { "x" } }
let t = X {}
print(t.name())
```

**The drift**:

- Kickoff prose declares `dyn T = box(X{})` — trait-object dispatch via
  `HeapKind::TraitObject = 29` + `TraitObjectStorage` carrier + VTable
  thunk path (Q25.C.5).
- Smoke fixture uses `let t = X {}` — plain TypedObject UFCS dispatch
  (the user's `t` is `TypedObjectStorage`, method `name()` resolves to
  `X::name` at compile time via UFCS).

These are **two architecturally distinct dispatch paths**. The fixture
exercises the path that has been working since W12-jit-trait-impl-
method-registry R20 γ close (commit `28bd0a7f`); the prose exercises
the path through Q25.C.5 TraitObject rebuild that is **not landed**
(per the Q25.C ratification at ADR-006 §2.7.24 + the absence of any
`box(X{})` lowering in the compiler).

### §I.2 Three options the dispatch asks to map

#### §I.2.a Option (a) — silent rescope (fixture replaces prose)

The fixture is the canonical smoke (3 of 4 kickoff smokes pass VM == JIT
at canonical fixture per the team-lead handover §"Current state"). The
kickoff prompt prose at 102-105 is amended to match:

```
type X {}
impl X { method name(): string { "x" } }
let t = X {}
print(t.name())                        # x
```

**Status doc + kickoff prompt + AGENTS.md updates**: amend
`phase-3-kickoff-prompt.md:102-105` + `phase-3-cluster-0-status.md`
post-R20 close-criterion paragraph (mention "Smoke 3 = canonical UFCS
fixture, not the dyn T trait-object prose"). Add a footnote in
kickoff prompt explaining the prose was aspirational pre-cluster-0;
the smoke fixture is the canonical close criterion.

**Scope**: 3 doc-file edits + 1 status-doc subsection update. **Zero
source changes.** Closes Surface A in one ceremony commit.

**Implications for Wave 2**: zero blockers. Wave 2 dispatches against
the canonical fixture. Q25.C.5 TraitObject rebuild stays as Q25.C ADR
text without an in-cluster-0 production requirement.

#### §I.2.b Option (b) — rebuild Q25.C TraitObject pre-cluster-0-close

Wave 2 grows an Agent H (Q25.C TraitObject rebuild) territory.
Scope estimate per Q25.C.1-7 amendments:

- **VTable thunk additions per Q25.C.5** (`crates/shape-vm/src/executor/trait_object_ops.rs`):
  ~12 thunk shapes (Direct / Closure / BoxedReturn / SelfArg / Generic
  / Compound) × handler bodies per Q25.C.5 VTableEntry enum. Each
  thunk is ~30-60 LoC. **~500 LoC.**
- **Self-arg runtime check per Q25.C.2** (`trait_object_ops.rs::thunk_self_arg_check`):
  ~50 LoC + tests.
- **Generic method type-info per Q25.C.3** (`crates/shape-runtime/src/type_info.rs`
  + thunk integration): ~200 LoC + tests.
- **ETO-001/ETO-002 error generation** at compile time
  (`crates/shape-vm/src/compiler/...`): error path for "Self::A
  without bound" + "method marked #[static_only]". ~80 LoC + tests.
- **`#[static_only]` attribute parsing** (`crates/shape-ast/src/shape.pest`
  + AST type): ~30 LoC + tests.
- **`#[erasure_safe]` attribute** (Q25.D): ~30 LoC + tests.
- **Universal-dyn auto-boxing rule** (Erase_T substitution operator)
  per Q25.C.1: ~150 LoC + tests in
  `crates/shape-runtime/src/type_system/inference/trait_object.rs`.
- **`let t: dyn T = box(X{})` lowering** in `compiler/expressions/binding.rs`
  + statements/typed_let.rs: bytecode emission for the boxing thunk
  per Q25.C. ~80 LoC.
- **IC devirtualization at the JIT** per Q25.C.6: out of cluster-0
  scope (the JIT IC tier already exists; per-vtable IC stabilization
  is a separate feature optimization, not a cluster-0 blocker).
- **LSP cost-class inlay hints per Q25.C.7**: out of cluster-0 scope
  (LSP is a separate workstream).

**Estimated Agent H territory**: ~1,100 LoC + ~30 LoC tests + 4 new
ADR sections (Q25.C amendments updated). **At the upper bound of
single-agent territory** but feasible per the audit §3.7 ceiling.

#### §I.2.c Option (c) — cluster-1.5 split (supervisor recommendation)

Smoke 3-trait-object becomes a cluster-1.5 close criterion (added to
the close criterion list); cluster-0 closes against the canonical
fixture (option (a) scope) without rebuilding Q25.C in cluster-0.
Wave 2 Agent H scope is **deferred** to a future cluster-1.5 dispatch.

This is the **supervisor recommendation** per the team-lead handover
§"Current state": "Wave 1 audit-day maps all three options A/B/C as
audit deliverables — does not block dispatch". The split is documented
in:

- `phase-3-cluster-0-status.md` cluster-0 close criterion subsection:
  "Smoke 3 = canonical UFCS fixture; trait-object dispatch path
  deferred to cluster-1.5."
- New status doc subsection naming cluster-1.5 territory: "Q25.C
  TraitObject rebuild — VTable thunks + Self-arg check + generic
  type-info + ETO-001/002 error generation + erasure_safe / static_only
  attribute parsing."
- `AGENTS.md` row for a future cluster-1.5 Wave (placeholder).
- `phase-3-kickoff-prompt.md:102-105` annotation: "smoke fixture is
  the canonical close criterion; prose-vs-fixture drift documented at
  audit §I".

**User-pending decision**: the supervisor disposition is (c); the
user disposes the final answer per the team-lead handover.

### §I.3 Wave 2 implications per option

| Option | Wave 2 Agent H? | Cluster-0 close criterion |
|---|---|---|
| (a) silent rescope | No | Canonical fixture |
| (b) rebuild pre-close | Yes (~1,100 LoC, large) | Prose `let t: dyn T = box(X{})` |
| (c) cluster-1.5 split | No (deferred) | Canonical fixture; cluster-1.5 adds trait-object |

**Audit recommendation**: option (c) per supervisor + user disposes.
Option (a) is the slim fallback if user wants no cluster-1.5
overhead. Option (b) is feasible but expands Wave 2 by one large
agent; refuse unless user explicitly authorizes.

### §I.4 Pre-flight ground-truth: smoke 3 fixture pass at HEAD

`/tmp/smokes/s3.shape` exists + is 5 LoC + matches §I.1 fixture.
Per the team-lead handover §"Smoke matrix at HEAD 14494605":

| Smoke | VM | JIT |
|---|---|---|
| 3 (canonical fixture) | passes (output: `x`) | passes (output: `x`) (post-γ R20 close) |

Closes the canonical smoke. Option (a) / (c) both satisfy the close
criterion.

---

## §J — 23+ shape-jit `#[ignore]`'d tests

### §J.1 Ground-truth count at HEAD `aa047356`

Total `#[ignore]` directives in `crates/shape-jit/`: **29**.
Excluding boilerplate ("tests deleted BytecodeToIR path; covered by
mir_compiler::integration_tests"): **13 non-trivial ignored tests**.

### §J.2 Per-test disposition

**Category A — extern-C-todo!()-SURFACE-aborts (5 tests):**

These tests exercise FFI functions whose body is `extern "C" todo!()`
(per W10-cascade close gates). `extern "C"` can't unwind, so the
todo!() body aborts the test process (SIGABRT) before the test's
assertion runs.

| File:line | Test name | SURFACE source |
|---|---|---|
| `crates/shape-jit/src/ffi/async_ops.rs:296` | `test_cancel_task_null_trampoline` | `jit_cancel_task` extern C todo!() pending kinded future-classification (ADR-006 §2.7.4/§2.7.5, W10 jit-playbook §5) |
| `crates/shape-jit/src/ffi/control/mod.rs:967` | `native_fixed_arity_helpers_surface_pending_kinded_abi` | `jit_call_foreign_native_0` extern C todo!() pending kinded foreign-call ABI rebuild (ADR-006 §2.7.10/Q11, W10 jit-playbook §5) |
| `crates/shape-jit/src/ffi_symbols/simulation/mod.rs:119` | `test_simulation_per_row_loop_smoke` | `jit_run_simulation` per-row loop invokes `jit_call_value` extern C todo!() pending kinded value-call ABI rebuild (ADR-006 §2.7.10/Q11 + §2.7.11/Q12) |
| `crates/shape-jit/src/core.rs:695` | `test_jit_array_info_*` | "W11/§2.7.4: deleted JitArray/jit_array_info API; kinded-FFI rebuild deferred" |
| `crates/shape-jit/src/compiler/c2_tests.rs:124` | `c2_capture_sub_32_*` | pre-existing JIT bug surfaced by C.2 sub-32 capture path |

**Disposition**: REWIRE post-Wave-2 (after Q11 / Q12 FFI rebuilds land
per the existing JIT playbook §5 deferral). NOT in Wave 2 scope —
these are JIT FFI bug-class fixes that the JIT playbook §5 explicitly
defers to a separate cluster.

**Category B — v2 BytecodeToIR-path-deleted-test boilerplate (16 tests):**

These tests are in `core.rs` (`#[ignore = "v2: tests deleted
BytecodeToIR path; covered by mir_compiler::integration_tests"]`) +
`ffi/typed_object/ffi_exports.rs:30/146` (same comment) +
`worker.rs:339/423` (`#[ignore = "v2: Tier 1 whole-function JIT
deprecated; tests dead path"]`).

**Disposition**: DELETE outright. These tests assert behavior of
the deleted BytecodeToIR path; their coverage is moved to
`mir_compiler::integration_tests`. Wave 2 territory: a small mechanical
cleanup that any agent can fold into their close commit, OR a single
Wave 3 agent territory. ~16 `#[ignore]` directives + ~150 LoC of dead
test bodies.

**Category C — pre-existing JIT bug surfaced via C.2 sub-32 capture path (2 tests):**

`crates/shape-jit/src/compiler/c2_tests.rs:124,208`. The ignore reasons
name "pre-existing JIT bug" with sub-32-bit capture path + SharedCow
F64 outer slot's Cranelift typing. Bug-class not in the cluster-0
deletion targets — REWIRE post-Wave-3 (cluster-2 stabilize-fix
territory).

**Category D — v2 build_kernel_ir / build_correlated_kernel_ir stubs (3 tests):**

`crates/shape-jit/src/core.rs:231/301/392`. "v2: build_kernel_ir is
stubbed out pending v2 runtime migration (strategy.rs:374)".

**Disposition**: REWIRE post-Wave-2 (v2 runtime migration is what Wave
2 finishes — these tests come back online post-Wave-2 if the kernel
IR builder is rewired; OR DELETE if the kernel IR path itself is
pre-strict-typing dead code that doesn't survive v2 cleanup).

### §J.3 Plus: shape-cli simulation ignored tests (4 tests)

Status doc Known Constraint: "v2-raw-heap aliasing class — 4 simulation
tests `#[ignore]`'d at `bin/shape-cli/tests/stdlib/simulation.rs`"
(test_harmonic_oscillator_rk4_system, test_rk45_system_harmonic_oscillator,
test_find_collisions_brute, test_find_collisions_sweep).

Ground-truthed at HEAD:

```
bin/shape-cli/tests/stdlib/simulation.rs:105 — test_harmonic_oscillator_rk4_system
bin/shape-cli/tests/stdlib/simulation.rs:180 — test_rk45_system_harmonic_oscillator
bin/shape-cli/tests/stdlib/simulation.rs:452 — test_find_collisions_brute
bin/shape-cli/tests/stdlib/simulation.rs:480 — test_find_collisions_sweep
```

All marked `"v2 raw-ptr aliasing class (path-c2/v2-c-alias): SIGSEGV
at VM Drop / heap corruption"`.

**Disposition**: REWIRE folds into v2-raw-heap-audit cross-cutting fix
(per the dispatch's "Wave 2 territory permits OR Wave 3 stabilize-fix").

### §J.4 Plus: shape-test ignored test (1 test)

`tools/shape-test/tests/objects_arrays/objects.rs:171`: "len() on
TypedObject: global builtin_len retired (c-len-migrate); TypedObject
lacks .len() PHF entry."

**Disposition**: REWIRE in Wave 2 (Agent A or Agent F) — fold into PHF
method-registry .len() addition for TypedObject; OR DELETE outright
(per status doc Known Constraint: "Follow-up: wire typed-object .len()
dispatch or drop the test"). Trivial Wave 2 close.

### §J.5 Wave 2 disposition for §J

| Category | Count | Wave 2 disposition |
|---|---|---|
| A — extern-C-todo!() SURFACE | 5 | DEFER to JIT playbook §5 (post-Wave-2 cluster-2) |
| B — v2 BytecodeToIR boilerplate | 16 | DELETE in Wave 2 cleanup arm of any agent |
| C — pre-existing JIT bug | 2 | DEFER to cluster-2 stabilize-fix |
| D — v2 build_kernel_ir stubs | 3 | DEFER (rewire post-Wave-2) or DELETE if dead code |
| shape-cli simulation | 4 | FOLD into v2-raw-heap-audit Wave 2/3 cross-cutting fix |
| shape-test len() | 1 | TRIVIAL Wave 2 fix or DELETE |

**Agent territory**: §J's category B + the shape-test len() test
folds into ANY Wave 2 agent's cleanup arm — ~30 LoC of test deletions
across `crates/shape-jit/src/core.rs` + `tools/shape-test/.../objects.rs`.
No new Wave 2 agent dedicated to §J.

Categories A, C, D, and shape-cli sim defer to post-Wave-2 cluster-2
audit territory (the JIT playbook §5 + v2-raw-heap-audit + c2-tests
JIT bug class).

---

## §K — 48 shape-test pre-existing failures

### §K.1 Ground-truth at HEAD `aa047356`

Per CLAUDE.md Known Constraints + status doc:

```
**Pre-existing shape-test failure clusters** (~48 tests, present on
`jit-v2-phase1@53a06ce` baseline):
  (a) generic-fn instantiation returning `Null` (stress_generics::generic_identity_*)
  (b) typed-closure inference regressions (stress_inference_complex::typed_closure_in_array_*)
  (c) array transformation chains (complex::test_complex_array_transformation_chain, test_complex_bubble_sort)
  (d) string `.join` (strings::test_string_join_*)
  (e) window functions (window_functions::basic::window_*)
  (f) array slice/sort/some (collections::test_array_slice_*, _sort_*, _some_*)
  (g) destructuring rest (destructuring::array_destructuring_rest)

Mix of inference-loss / monomorphization / v2-raw-heap. Tracked as
`shape-test-residuals-audit`.
```

### §K.2 Per-category disposition

| Category | Pattern | Likely root cause | Wave 2 disposition |
|---|---|---|---|
| (a) generic-fn instantiation Null | `generic_identity_*` returning Null instead of the input value | Monomorphization erasure of type args at inference layer (per CLAUDE.md Known Constraints "Queryable<T> generic impl" note: "type-inference erases type args back to simple names") | DEFER to cluster-2 audit triage |
| (b) typed-closure inference | `typed_closure_in_array_*` inference regression | Bidirectional closure inference gap in array contexts | DEFER to cluster-2 audit triage |
| (c) array transformation chains | `test_complex_array_transformation_chain`, `test_complex_bubble_sort` | v2-raw-heap aliasing class OR multi-pass inference loss | Possibly FOLD into Wave 2 if v2-raw-heap-audit lands in territory; otherwise DEFER |
| (d) string .join | `test_string_join_*` | Stdlib .join migration to v2-raw StringObj carrier (depends on Agent B close) | FOLD into Wave 2 Agent B close (or follow-up) |
| (e) window functions | `window_*` | Inference / monomorphization in window-function specialization | DEFER to cluster-2 |
| (f) array slice/sort/some | `test_array_slice_*`, `_sort_*`, `_some_*` | Method-handler bug class OR v2-raw aliasing | FOLD into Wave 2 if territory permits; otherwise DEFER |
| (g) destructuring rest | `array_destructuring_rest` | Compiler-level rest-pattern lowering gap | DEFER to cluster-2 |

### §K.3 Audit recommendation — surface-and-stop

**The 48-test cluster exceeds Wave 2 single-agent territory.** Each
category is a distinct failure-class with its own root cause
investigation (per the CLAUDE.md Known Constraint "Mix of inference-
loss / monomorphization / v2-raw-heap" framing).

**Audit recommendation**: surface-and-stop. The 48 tests are
**cluster-2 audit triage territory**, not Wave 2 fixable. Wave 2 may
incidentally fix some of (c), (d), (f) tests via the v2-raw-heap-audit
cross-cutting fix or Agent B's String migration; the rest remain
cluster-2.

**Supervisor disposition required**: ratify the cluster-2 audit
triage classification, OR escalate specific categories into Wave 2
(e.g. "Wave 2 Agent B's String migration MUST close all (d)
`test_string_join_*` failures" makes the Wave 2 close gate stricter).

### §K.4 Pre-existing baseline preservation

Per CLAUDE.md "Own all code quality" rule: the 48 tests are NOT framed
as "pre-existing" — they are real failures whose root cause needs
ownership. The cluster-2 audit triage is a recognized work
expenditure, not a deferral framing under refuse #10.

The bulldozer cadence refuses "preserve X for cluster-1+" framing
WHEN it's used to escape Wave 2 scope without structural reason. §K's
deferral has the structural reason: **48 tests across 7 distinct
failure-classes is structurally beyond Wave 2 single-agent territory.**

---

## §L — Wave 2 agent partition recommendation

### §L.1 Agent partition (6 agents + 1 conditional)

Per §A-K dispositions, the proposed Wave 2 partition:

| Agent | Territory | LoC scope | Inter-agent overlap |
|---|---|---|---|
| **Agent A1** — TypedArrayData scalar variant deletion + v2-raw `TypedArray<T>` scalar producer migration (i8/i16/u8 distinct from Bool/u16/u32/u64/f32/char/i32/i64/f64 monomorphizations) | `crates/shape-vm/src/compiler/v2_typed_emission.rs`, `compiler/expressions/collections.rs`, `executor/v2_handlers/array.rs`, `executor/v2_handlers/v2_array_detect.rs`, `shape-jit/src/ffi/v2/mod.rs`, `shape-jit/src/ffi_symbols/v2_symbols.rs`, opcode defs | ~3,000 LoC (8 new monomorphizations × ~400 LoC each) | Touches `heap_value.rs` for TypedArrayData scalar arm deletions (cascades into Agent A2) |
| **Agent A2** — TypedArrayData heap-element variant deletion + v2-raw `TypedArray<*const StringObj/DecimalObj>` producer migration (String + Decimal live arms only per supervisor R20 (A) Minimal) | Same set as A1 + `executor/objects/string_methods.rs`, `executor/objects/array_*.rs` (String/Decimal arm deletions in array_transform, concat, hashmap, etc.) | ~2,500 LoC (53 String sites + 46 Decimal sites + cascade) | Heavy overlap with A1 + Agent F (consumer cascade in same files); **STAGE A1 → A2 → F** in Wave 2 |
| **Agent B** — New `NativeKind::StringV2` / `DecimalV2` variants + ConcreteType cascade + KindedSlot constructors + ADR-006 §2.7.5 amendment (per §H decision) | `crates/shape-value/src/native_kind.rs`, `slot.rs` (KindedSlot), `v2/concrete_type.rs`, `shape-vm/src/compiler/type_tracking.rs`, `executor/vm_impl/stack.rs::clone_with_kind/drop_with_kind`, `shape-jit/src/ffi/value_ffi.rs` HK_* + `jit_kinds.rs` | ~600 LoC (R19 S1.5 precedent: 2 new NativeKind variants × ~300 LoC cascade each) | Gates Agent A2 (Agent A2's String/Decimal producer migration emits StringV2/DecimalV2 slots) — **STAGE B → A2** |
| **Agent C** — HashMapValueBuf deletion + `HashMapData<V>` generic per-V migration + ADR-006 §2.7.24 Q25.B SUPERSEDED amendment | `crates/shape-value/src/heap_value.rs` (HashMapValueBuf def + HashMapData fields + impl + from_pairs + specialize_values), `shape-vm/src/executor/objects/hashmap_methods.rs`, `printing.rs` (Display), `trait_object_ops.rs:single-arm`, `runtime/stdlib/xml.rs` + `json.rs` + `vm_impl/builtins.rs` (HashMapData::from_pairs callers), `executor/objects/array_transform.rs:1796` | ~1,500 LoC (HashMapData<V> generic conversion + 8 from_pairs callers) | Touches `heap_value.rs` (overlap with Agent A); separable section; take-both at merge |
| **Agent D1** — TypedObjectStorage Arc → HeapHeader shape change (struct grows HeapHeader at offset 0 + manual `_new` / `_drop` + `impl HeapElement`) + ADR-006 §2.3 amendment | `crates/shape-value/src/heap_value.rs` (TypedObjectStorage def + impl + Drop), `kinded_slot.rs` (from_typed_object constructor + 4-table lockstep dispatch in clone_with_kind / drop_with_kind / SharedCell::drop), `v2/heap_header.rs` (HEAP_KIND_V2_TYPED_OBJECT constant) | ~800 LoC (storage layer + 4-table lockstep) | Touches `heap_value.rs` (overlap with Agent A + C); **STAGE D1 BEFORE A2 / D2** |
| **Agent D2** — TypedObjectStorage construction-site cascade (~18 production sites + JIT FFI sites + consumer match arm re-pattern) | All sites at §D.2 + JIT field-access at `shape-jit/src/ffi/typed_object/` | ~1,500 LoC (~300 sites × ~5 LoC each) | Heavy with everywhere TypedObject is touched; **STAGE D2 AFTER D1** |
| **Agent E** — TraitObjectStorage HeapHeader migration (fat pointer to raw) + ADR-006 §Q25.C.5 amendment | `crates/shape-value/src/heap_value.rs` (TraitObjectStorage def + impl), `kinded_slot.rs`, `executor/trait_object_ops.rs` (3 construction sites) | ~600 LoC + ~12 self-test fixture updates | Gated on Agent D1 close (E needs TypedObjectStorage HeapHeader-equipped); **STAGE D1 → E** |
| **Agent F** — Q25.A specialization dead arm wholesale deletion (DateTime/Timespan/Duration/Instant + TraitObject arms; HashMapValueBuf temporal/instant/TraitObject arms) | `heap_value.rs` (enum arm deletions only), all consumer files for the dead-arm match arm deletion cascade (`array_transform.rs:579-1021`, `concat.rs:271-319`, `printing.rs`, `wire_conversion.rs`, `json_value.rs`, `iterator_state.rs`, `xml.rs`) | ~600 LoC (9 arms × ~70 LoC consumer cascade each) | Subset of Agent A's territory but separable; close before A merges or fold into A's close commit |
| **Agent G** — W12-stdlib-intrinsic-collapse (delete BuiltinFunction::IntrinsicSum + handler body + tests; route stdlib `sum()` through PHF method dispatch) | `crates/shape-vm/src/bytecode/opcode_defs.rs`, `compiler/helpers.rs`, `compiler/expressions/function_calls.rs`, `executor/vm_impl/builtins.rs`, `runtime/intrinsics/math.rs` + `mod.rs`, stdlib math `.shape` file | ~300 LoC deleted + 1 stdlib file edit | None — isolated from A/B/C/D/E/F territories |
| **Agent H (conditional on Surface A option (b))** — Q25.C TraitObject rebuild (VTable thunks + Self-arg runtime check + generic method type-info + ETO-001/ETO-002 error generation + erasure_safe / static_only attribute parsing + box(X{}) lowering) | `crates/shape-vm/src/executor/trait_object_ops.rs` (thunks), `compiler/expressions/binding.rs`, `statements/typed_let.rs` (`let t: dyn T = box(X{})` lowering), `compiler/...` (ETO error path), `shape-ast/src/shape.pest` (attribute parsing), `runtime/type_info.rs`, `runtime/type_system/inference/trait_object.rs` (Erase_T) | ~1,100 LoC (per §I.2.b estimate) | None mechanically; gated on Surface A user disposition |

### §L.2 Wave 2 dispatch staging

Two-round dispatch shape (per the team-lead handover §"Wave 2 dispatch
shape"):

**Round 1 (parallel):**

- Agent A1 — TypedArrayData scalar variant deletion (no upstream deps)
- Agent B — NativeKind::StringV2 / DecimalV2 amendment + cascade (no upstream deps; gates A2)
- Agent C — HashMapValueBuf deletion (touches heap_value.rs but separable)
- Agent D1 — TypedObjectStorage shape change (gates A2 / D2 / E)
- Agent F — Q25.A specialization dead arm wholesale deletion
- Agent G — W12-stdlib-intrinsic-collapse

**Round 2 (after Round 1 merges):**

- Agent A2 — TypedArrayData heap-element variant deletion + String/Decimal v2-raw producer migration (consumes B + D1's output)
- Agent D2 — TypedObjectStorage construction-site cascade (consumes D1's API)
- Agent E — TraitObjectStorage HeapHeader migration (consumes D1's API)

**Round 3 (optional, gated on Surface A user disposition):**

- Agent H — Q25.C TraitObject rebuild (Surface A option (b) only)

### §L.3 Per-pair territory intersection check

| Pair | Intersection? | Resolution |
|---|---|---|
| A1 ↔ A2 | YES — both touch `heap_value.rs` TypedArrayData enum + `array.rs` opcode handlers | STAGE A1 → A2 (sequential); take-both at merge if both close concurrent rounds |
| A1 ↔ B | NO mechanically — A1 emits opcodes for scalar producers; B amends NativeKind for heap-element discriminators | No staging; parallel safe |
| A1 ↔ C | YES — both touch `heap_value.rs` | Take-both at merge; HashMapValueBuf section separable from TypedArrayData section |
| A1 ↔ D1 | YES — both touch `heap_value.rs` (D1's TypedObjectStorage struct change is at line 2399; A1's TypedArrayData scalar arm cleanup is at line 2942) | Separable sections; take-both safe |
| A1 ↔ F | YES — F deletes TypedArrayData::DateTime/Timespan/Duration/Instant/TraitObject arms; A1 deletes the scalar arms | F closes first (smaller scope), A1's close consumes F's reduced enum |
| A1 ↔ G | NO | Parallel safe |
| A2 ↔ B | YES — B's StringV2/DecimalV2 NativeKind is consumed by A2's producer migration; A2 also consumes the new opcodes from A1 | STAGE B → A2 (sequential within Wave 2) |
| A2 ↔ D1/D2 | YES — A2's TypedObject arm deletion depends on D1/D2 close | STAGE D1 → D2 → A2 |
| A2 ↔ E | YES — A2's TraitObject arm deletion depends on E close | STAGE E → A2 |
| A2 ↔ F | YES — F deletes the dead TypedArrayData heap-element arms (DateTime/etc.); A2 deletes the live ones (String/Decimal); both touch array_transform / concat / printing consumer cascades | F first (smaller); A2 second |
| C ↔ D1 | NO mechanically — C migrates HashMapValueBuf; D1 changes TypedObjectStorage shape; the HashMapValueBuf::TypedObject arm is gated on D1 (C's close handles the new HashMapData<V> shape; the TypedObject arm migration is a trailing C close step that consumes D1's output) | STAGE D1 → C-final-arm-migration |
| D1 ↔ D2/E | YES — D1 provides the v2-raw API surface; D2 + E consume it | STAGE D1 → D2 + E (parallel within Round 2) |
| F ↔ everyone | F is the smallest, fastest-closing agent. Close F first in Round 1 to reduce TypedArrayData enum variant count from 22 to 17. Subsequent agents work against the smaller enum | F first |
| G ↔ everyone | G is isolated (IntrinsicSum is its own surface) | Parallel safe |
| H ↔ everyone | H is the conditional Q25.C rebuild; only dispatches if Surface A option (b); isolated mechanically | Round 3 (post-Round-2 merges) |

### §L.4 Merge-ceremony shape

Per the team-lead handover §"Dispatch cadence + close-gate shape":

- **Take-both for**: AGENTS.md row appends + dispatch-table arm
  additions + ADR-006 amendment text scattering (Wave 2 produces 4-6
  ADR amendments across §2.3, §Q25.A SUPERSEDED, §Q25.B SUPERSEDED,
  §2.7.5 + Q8, §Q25.C.5).
- **Take-HEAD for**: test attribute / `#[ignore]` directives where one
  branch has detailed §-cite comments.
- **After any take-both pass**: `cargo check --workspace --lib` MUST
  pass before commit.
- **Verify-merge.sh measurement**: file-redirect for exit capture per
  CHECK-COMMS-1.
- **Smoke matrix re-verification**: after every Wave 2 merge, run all
  4 (or 5 per Surface A disposition) kickoff smokes under both modes.
  Update `phase-3-cluster-0-status.md`.

### §L.5 Wave 3 close-criterion (after Wave 2 ratifies)

Per the team-lead handover §"Wave 3 dispatch shape":

1. Kickoff smoke matrix re-verification VM == JIT (all 4 or 5 smokes
   per Surface A disposition).
2. Status doc cluster-0 + cluster-1 close summary.
3. ADR-006 master amendment commit consolidating wave-merge amendment
   scattering (if Wave 2 produced enough amendment text that
   consolidation is warranted; otherwise skip).
4. Cluster-0+1 close report → supervisor ratifies → user authorizes
   `phase-3-cluster-0-close` + `phase-3-cluster-1-close` tags.

---

## §M — Pre-flight ground-truth verification at HEAD `aa047356`

Every audit claim grep-verified against actual source (per the
dispatch's "5-instance supervisor-/audit-layer imprecision pattern is
the signal to verify EVERY ground-truth claim before landing"):

| Claim | Verification command | Result |
|---|---|---|
| `TypedArrayData` enum at heap_value.rs:2942 with 22 variants | `grep -nE "^pub enum TypedArrayData\b\|^\s+(I64\|F64\|Bool\|I8\|I16\|I32\|U8\|U16\|U32\|U64\|F32\|String\|Decimal\|BigInt\|DateTime\|Timespan\|Duration\|Instant\|Char\|TypedObject\|TraitObject)\(" crates/shape-value/src/heap_value.rs` | confirmed 22 live variants, definition spans 2942-2994 |
| `TypedBuffer<T>` references: 144 hits, 26 files | `grep -rln "TypedBuffer<" crates/ --include="*.rs" \| wc -l` + `grep -rn "TypedBuffer<" crates/ --include="*.rs" \| wc -l` | confirmed |
| `AlignedTypedBuffer` references: 123 hits, 27 files | same | confirmed |
| `HashMapValueBuf` external producers = 0 | `grep -rnE "HashMapValueBuf::\w+\(Arc::new" crates/ --include="*.rs" \| grep -v /heap_value.rs` | confirmed 0 external producers (12 total all in heap_value.rs::specialize_values) |
| `TypedObjectStorage` production constructors = 18 (+ 18 self-test) | `grep -rnE "TypedObjectStorage::new\b\|TypedObjectStorage \{\|HeapValue::TypedObject\(Arc::new\(TypedObjectStorage" crates/ --include="*.rs"` | confirmed |
| `TraitObjectStorage::new` production constructors = 3 (+ 12 self-tests) | `grep -rnE "TraitObjectStorage::new\b" crates/ --include="*.rs"` | confirmed 3 production sites in trait_object_ops.rs:183/864/1106 + self-tests at heap_value.rs:5349-5587 |
| TypedArrayData::DateTime root constructors = 0 | `grep -rnE "TypedArrayData::DateTime\(Arc::new" crates/ --include="*.rs"` + manual classification of each hit as derived (slice/concat/zip output arm) vs root | confirmed 4 hits all derived (concat.rs:271, array_transform.rs:579/744/992) — the `build_specialized_from_heap_arcs` at heap_value.rs:3008 has NO DateTime arm + `other =>` fallthrough at line 3088 |
| IntrinsicSum body comment at builtins.rs:489-504 says "mirror of v2_int_sum" | direct read of builtins.rs:460-586 | confirmed verbatim — the parallel-implementation defection is self-documented |
| Surface A drift: kickoff prose says `box(X{})`, fixture says `let t = X {}` | `cat /tmp/smokes/s3.shape` + read of `docs/cluster-audits/phase-3-kickoff-prompt.md:102-105` | confirmed |
| shape-jit `#[ignore]` count = 29 total | `grep -rnE "^\s*#\[ignore" crates/shape-jit/ --include="*.rs" \| wc -l` | confirmed 29 |
| shape-cli simulation `#[ignore]` count = 4 | `grep -rnE "^\s*#\[ignore" bin/shape-cli/tests/ --include="*.rs"` | confirmed 4 |
| HeapKind ordinals 0..35 at heap_variants.rs | direct read of crates/shape-value/src/heap_variants.rs | confirmed (0-28 base + 29 TraitObject + 30 Mutex + 31 Atomic + 32 Lazy + 33 ModuleFn + 34 Matrix + 35 MatrixSlice) |
| HEAP_KIND_V2_DECIMAL = 85 at heap_header.rs:36 | direct read of crates/shape-value/src/v2/heap_header.rs | confirmed |
| HeapElement trait at v2/heap_element.rs | direct read | confirmed (`pub unsafe trait HeapElement` at line 69, `unsafe fn release_elem(ptr: *const Self)` at line 78) |
| StringObj exists at v2/string_obj.rs | direct read | confirmed (24-byte struct, lines 18-26, plus HeapElement impl) |
| DecimalObj exists at v2/decimal_obj.rs (R20 S2-prime-production close) | direct read | confirmed |

**No audit-text imprecisions surfaced.** Ground-truth checks pass.

---

## §N — Forbidden-pattern surveillance during this audit

Per the dispatch's forbidden-pattern list (refused on sight):

- **"Preserve X for cluster-1+" / "needs its own audit sub-cluster" / "multi-week scope" / "defer to cluster-1.5 post-close"** — refused. The only deferrals in this audit are §J categories A/C/D (genuinely outside Wave 2 scope per JIT playbook §5 + cluster-2 stabilize) and §K (48-test cluster, structurally beyond Wave 2 single-agent territory). Each surface-and-stop is named with structural reason, not framing.
- **Resurrecting `ValueWord` / `tag_bits` / W-series dispatch shapes under any rename** — none in this audit.
- **Bool-default fallback for unknown kind** — none. §H decision uses new NativeKind variants explicitly (not Bool-default fallback).
- **bridge/probe/helper/hop/translator/adapter/shim descriptors** — none. Every migration is described by name (per-element-kind monomorphization, HeapHeader-equipped carrier, etc.) or by deletion-fate (the deleted `TypedArrayData` enum class).
- **Parallel-implementation framings** ("documented intentional duality" / "preserve both carriers" / "carrier unification via boundary deletion as one-off patch") — none. §A through §G are systematic producer migrations, not one-off patches.
- **Re-introducing `JitArray` as parallel discriminator to TypedArrayData** — none.
- **Audit-text imprecisions that lack file:line ground truth** — none. §M ground-truth section.

---

## §O — CLAUDE.md modifications surfaced

**None required by this audit's deliverables.**

The audit's deliverables fit cleanly within the existing CLAUDE.md
"Forbidden Patterns" + "Renames to refuse on sight" + "Parallel-
implementation across producer/consumer carrier-shape boundaries"
framings. The Wave 2 ADR amendments (§D.8 / §E.6 / §C.8 / §H.5 / §A
Q25.A SUPERSEDED supplemental) land in ADR-006 sections, not CLAUDE.md.

**Flagging convention for Wave 2 agents** (per the dispatch's
forbidden-in-this-dispatch list): if any Wave 2 agent surfaces a NEW
forbidden pattern or refuse-on-sight phrase that should land in
CLAUDE.md (e.g. a previously-unnamed defection-attractor pattern at
the per-carrier-shape boundary), that agent FLAGS EXPLICITLY in their
close report. Team-lead surfaces to supervisor → user ratifies the
CLAUDE.md modification → team-lead lands separately.

---

## §P — Genuinely intractable surfaces (for supervisor ADR-level decision)

Per the dispatch's "Any genuinely intractable deletion-target that
requires supervisor ADR-level decision before Wave 2 dispatches
(surface-and-stop with structured shape)":

### §P.1 NativeKind cardinality bound (Q8) — supervisor ratification required

**Surface**: §H decision adds `NativeKind::StringV2` + `NativeKind::DecimalV2`
variants. Per ADR-006 §2.7.6 / Q8 the cardinality bound is "one
constructor per `NativeKind` heap variant + at most one scalar accessor
per scalar variant". The two new variants are scalar (carrying
`*const StringObj` / `*const DecimalObj`); they grow the scalar set
by 2 (from 5: i64/f64/bool/char/str → 7: + StringV2 + DecimalV2).

The Q8 bound is "the audit notes the §2.7.5 amendment landing per
R19 S1.5 expanded NativeKind by 2 (F32 + Char) within the bound". Two
more variants is structurally similar.

**Supervisor disposition needed**: ratify the Q8 cardinality expansion
to accommodate StringV2 + DecimalV2 OR override with a different §H
shape (e.g. accept the H-a materialize-on-read perf hit; OR accept
H-b's parallel-carrier under the same NativeKind::Ptr(HeapKind::String)
with the duality framing being "documented intentional" — refuse #9
applies per cluster-0 instance log; supervisor would have to ratify
the override).

### §P.2 Surface A (kickoff-prompt-vs-fixture mismatch) — user-pending

**Surface**: §I three options (a)/(b)/(c). Supervisor recommendation
(c); user disposes. Wave 2 dispatch needs the disposition before Round
3 (Agent H conditional). Round 1 + Round 2 dispatch without waiting.

### §P.3 §K 48-test cluster — cluster-2 audit triage classification

**Surface**: §K's audit recommendation is "cluster-2 audit triage
territory". Supervisor ratifies the classification OR escalates
specific categories into Wave 2.

### §P.4 None other intractable

§A through §G + §J are clean Wave 2 deliverables under the audit
recommendations. No additional supervisor ratifications needed
pre-Wave-2-dispatch beyond the three above.

---

## §Q — Wave 2 close-gate consolidation

For each Wave 2 agent, the close gate is the standard cadence:

1. `cargo check --workspace --lib --tests` exit 0 (EXIT CODE, not grep).
2. `bash scripts/verify-merge.sh > /tmp/out 2>&1; echo $?` — 12/12 PASS
   (CHECK-COMMS-1 file-redirect).
3. `bash scripts/check-no-dynamic.sh` exit 0.
4. AGENTS.md row appended (the row is added at dispatch with
   "active"; status flipped to "closed" with commit hash + summary at
   close).
5. ADR amendments in close commit (per agent).
6. **NO `Co-Authored-By: Claude` trailer** (MEMORY.md user rule).

After each Wave 2 round merges:

- Smoke matrix re-verification (Smokes 1/2/3/4 [+5 per Surface A]
  under both `--mode vm` and `--mode jit`).
- Status doc Wave 2 close subsection appended.
- ADR amendment text consolidated if scattered (per §L.4 merge-ceremony).

---

## §R — Audit close

This audit produces:

- This document (`docs/cluster-audits/bulldozer-wave-1-inventory.md`,
  ~1,500 LoC) with sections A through R.
- Zero source changes (no `.rs` / `.toml` / `.lock` modifications).
- AGENTS.md row update (from "active" to "closed" with commit hash +
  summary, executed by team-lead at close-merge ceremony per the
  dispatch's close gate).

Baseline gate at close commit: `bash scripts/verify-merge.sh` exit 0
(audit-only doc-record close per W17-narrow + S2 + R20 S2-prime
precedent).

Next action: team-lead reads this audit + verify-merge.sh + posts the
close report to supervisor for Wave 2 dispatch authorization.

---

*End of Wave 1 inventory.*
