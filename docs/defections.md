# Defections Log

A running record of considered-but-rejected compromises in the strict-typing work (`~/.claude/plans/stop-native-vs-tagged-tax.md`). Future sessions read this to recognize the pattern in real time.

## Why this log exists

The `v2-nanbox-removal-plan.md` Step 6 ("delete `ValueWord`") was originally a one-line deletion. Mid-execution it was renamed to "ValueBits shim retained as FFI-boundary bridge" and became permanent. That single rationalization compounded into ~6 weeks of W-series cleanup, deferred v2-raw-heap aliasing tests, ignored shape-jit tests, and ~48 shape-test failures.

Rationalizations sound reasonable in the moment. They look obvious in hindsight. This log captures them while they're fresh so the next session can spot the same shape faster.

## How to use

When you (agent or human) consider a fallback / shim / bridge / decode hop / "follow-up" disposition for the strict-typed work, **before** implementing it, log the consideration here. Even if you ultimately reject it. Logging takes 60 seconds; the discipline pays back when the next session reads the log on day one.

Cross-reference: `shape/CLAUDE.md` "Forbidden Patterns" section enumerates the patterns. This log records the *attempts* at those patterns.

## Format

```
## YYYY-MM-DD — <one-line summary>

**Considered:** <what you almost did>

**Rationalization:** <why it sounded reasonable in the moment>

**Pattern recognized:** <which forbidden pattern from CLAUDE.md this matches>

**Alternative taken:** <what you did instead>

**Cost saved:** <estimated days/weeks of W-series-style cleanup avoided>
```

## Historical defections (pre-log, reconstructed)

These were not logged at the time. Reconstructed from commit history and plan archaeology so the pattern is on record.

### 2026-04-18 — `v2-nanbox-removal-plan.md` Step 6 quietly downgraded

**Considered:** delete `crates/shape-value/src/value_word.rs`, replace with `pub type ValueWord = u64`, no methods.

**Rationalization:** "comptime, polyglot, and unproven-type sites need a dynamic representation; retain `ValueBits` shim as documented FFI-boundary bridge."

**Pattern recognized:** "Rename to a less suspicious name" (`ValueBits shim`, `FFI-boundary bridge`).

**Alternative taken (at the time):** retained `ValueWord` as ~2,650-line "dynamic fallback". Plan status edited from "delete `ValueWord`" to "ValueBits shim landed; dynamic-fallback residuals tracked".

**Actual cost incurred:** the W-series (W1–W4, α/δ follow-ups, 9 commits over multiple sessions); 4 deferred v2-raw-heap aliasing tests; ~48 shape-test failures in the same bug class; ~23 ignored shape-jit tests. Estimate: 4–6 weeks of cumulative cleanup that this rename made permanent. Resulting plan (`stop-native-vs-tagged-tax.md`) reverses the decision and bulldozes first.

### 2026-05-05 — W4-δ `ConvertBoolToString` opcode

**Considered:** add a dedicated `ConvertBoolToString` opcode to handle `bool as string` casts at runtime.

**Rationalization:** "the existing convert path loses type info; one new opcode is small and surgical (74 LoC, 1 test closed)."

**Pattern recognized:** "Add a new opcode for this specific conversion" — a `Convert<X>To<Y>` opcode added to paper over a compiler kind-tracker gap.

**Alternative taken (at the time):** the new opcode was added (commit `3fa7456`).

**Should have done:** fix the compiler so the convert path doesn't lose type info. The bool source's kind was statically knowable at the convert site; `last_emitted_native_kind` had a propagation gap.

**Cost incurred:** one more opcode in `OpCode` enum; another decode site to delete in Phase 1 of the strict-typing bulldozer.

---

## 2026-05-07 — HashMap-marshal micro-cluster — named on-record (full entry)

This is **not** a defection. On-record promotion from named-in-passing
references (B1 audit-grounded correction at line ~334 + 343, dated
2026-05-07 sub-entry on csv consumer expansion) to a fully-named
cluster entry. Per finding #11 in-place-update discipline, the
prior named-in-passing references in the B1 entry are correct as
written; this entry consolidates the cluster into one place for
future audits.

**Cluster identity.** Three confirmed consumer sites, one
architectural decision:

- **`http.rs`** — body builds object responses
  `{status, headers, body, ok}` via `ValueWord::from_hashmap_pairs`
  (deleted) and parses option args via `as_hashmap` (deleted).
  Currently still uses the legacy `register_typed_function` ABI;
  has 5+ shape-runtime --lib errors blocked on the HashMap-marshal
  decision plus the marshal-extension surface.
- **`csv.parse_records`** — returns `Array<HashMap<string, string>>`
  via the deleted `ValueWord::from_hashmap_pairs(keys, values)`
  helper. Held back from Phase 2d Array's `csv_module` migration
  (commit `9f6b1d3`) explicitly because the HashMap shape was not
  yet decided. Body documented as deferred at
  `crates/shape-runtime/src/stdlib/csv_module.rs` deferred-list
  comment.
- **`csv.stringify_records`** — consumes
  `Array<HashMap<string, string>>` via `as_hashmap` (deleted).
  Same deferral as `csv.parse_records`.

**Plus B1 sub-decision #2 unblock:** the JSON `Json::Object(...)`
payload currently uses `ValueWord::from_hashmap_pairs(keys, values)`
to build a HashMap-shaped value. B1 sub-decision #2's runtime shape
question (per the 2026-05-07 B1 audit-grounded correction) reduces
to "what HashMap-marshal shape does the JSON Object variant use?" —
landing HashMap-marshal answers it. After HashMap-marshal lands,
B1 becomes mechanical (5 parser modules + ~19 errors).

**Storage-shape options (3 surfaced; architectural decision pending):**

- **(P1) `HeapValue::HashMap(HashMapData)` variant.**
  Add a new top-level variant to `HeapValue` (similar to how
  `TypedArray(TypedArrayData)` and `Temporal(TemporalData)` are
  shaped). `HashMapData` carries either:
  - (a) `Vec<(Arc<String>, Arc<HeapValue>)>` for general
    `HashMap<string, *>` keying, or
  - (b) two separate buffers — `Arc<TypedBuffer<Arc<String>>>` keys
    + `Arc<TypedBuffer<Arc<HeapValue>>>` values — reusing Phase 2d
    Array's `TypedArrayData::String` / `TypedArrayData::HeapValue`
    storage shapes for the underlying buffers.

  Pro: a real HashMap variant with O(1) key lookup if backed by a
  hash index (mirrors the legacy `from_hashmap_pairs` shape).
  Update `HeapKind` enum (one new variant); update all
  match-on-HeapKind sites; provide accessor methods.
  Con: adds a `HeapKind`/`HeapValue` variant — incremental scope
  similar to Phase 2d Array's TypedArrayData additions, but
  workspace-wide rather than enum-internal.

- **(P2) Two-slot `TypedObject` projection (`{keys: Array<string>,
  values: Array<HeapValue>}`).**
  Encode `HashMap<string, T>` as a TypedObject with a fixed
  2-slot schema where slot 0 is a `TypedArrayData::String` keys
  array and slot 1 is a `TypedArrayData::HeapValue` values array.
  Reuses Phase 2d Array's already-landed variants entirely; no
  new HeapValue variant needed.

  Pro: zero new HeapKind variants; uses already-landed Phase 2d
  Array shapes; fits the strict-typed model cleanly.
  Con: O(n) key lookup unless a hash index is added separately.
  **Cross-crate interlock** — shape-vm's `__json_object_get` /
  `__json_array_at` / `as_hashmap` accessors currently expect a
  HashMap-shaped runtime value; switching to two-slot TypedObject
  requires updating those accessors in lockstep. The lockstep
  change is bounded but adds shape-vm scope to the HashMap-marshal
  landing.

- **(P3) `from_hashmap_pairs`-equivalent free function returning a
  pre-existing typed shape.**
  E.g., produce a `HeapValue::TypedObject{schema_id: HashMap, slots,
  heap_mask}` where `HashMap` is a registered schema with
  per-instance keys. The downside is that anonymous schemas (one
  per HashMap instance) inflate the schema registry; alternatively
  use a single canonical "HashMap shape" schema as a marker, with
  the actual key/value data in slots. This is option P2 in
  disguise; refused as a separate option once examined.

**Open sub-decisions within the storage-shape choice:**

- If P1: hash index — eager (build at insert) vs lazy
  (build at first lookup) vs none (linear scan).
- If P1 with hash index: index storage — inline in `HashMapData`
  vs separate cache.
- If P2: shape-vm `__json_object_get`/`as_hashmap` accessor
  migration — single combined commit vs split runtime/dispatch
  commits.

**Already-rejected options (refuse on sight):**

- **Use `serde_json::Map`-style ordered map as the storage type
  directly** — third-party type at the FFI boundary; refuses
  cleanly on the same grounds as foreign-type-at-marshal-layer
  rejections elsewhere.
- **Use Rust's `HashMap<String, HeapValue>` directly** — does not
  preserve insertion order (Shape's `from_hashmap_pairs` semantics
  preserve insertion order); behavioral regression vs the legacy
  shape.
- **Single `HashMapStringString`-only path** with a separate
  `HashMapStringJsonValue` etc. for each value type —
  parametric-explosion shape (same family as the rejected
  per-element-kind `TypedArrayData` variants from Phase 2d Array).
  `HashMap<string, string>` is the most-used shape but
  `HashMap<string, *>` (for any heap-typed value) is the general
  consumer surface.

**DAG dependencies:**

- **Phase 2d Array (resolved)** — both P1(b) and P2 reuse Phase 2d
  Array's `TypedArrayData::String` / `TypedArrayData::HeapValue`
  variants. No interlock; supports either path.
- **Cluster #4 Option (resolved)** — independent. `ConcreteReturn::HashMapStringString`
  already exists; `Option<HashMap<*, *>>` would compose via
  `TypedReturn::Some(ConcreteReturn::HashMap*)` — no new architectural
  decision per Cluster #4's β shape.
- **shape-vm cascade** — interlock present for P2 only. P1 is
  shape-runtime-internal; shape-vm consumers continue to use
  `as_hashmap()`-equivalent which gets re-implemented to read
  `HeapValue::HashMap` instead of the deleted ValueWord-HashMap.
- **B4 core-foundation** — independent. HashMap-marshal storage
  shape doesn't depend on closure-captures, module-loader-value,
  plugin-ABI, etc.

**HashMap-marshal disposition:** named cluster on-record. Audit
1+2+3 binding pre-work pending — Stage C of the supervisor's
three-stage plan covers this once Stage B (zero-copy) lands. The
storage-shape decision (P1 vs P2) is the architectural commit
candidate; sign-off relay required before any code change.

**Predicted error-drop on landing:**

- Architectural extension commit (storage-shape variant + FromSlot/
  ToSlot + ConcreteReturn::HashMap* additions): 0 ± 3 (same shape
  as Phase 2d Array + Cluster #4 leaf landings).
- Consumer migration commits:
  - `http.rs` migration: -2 to -4
  - `csv.parse_records` + `csv.stringify_records` activation: ~0
    (currently commented-out / stubbed, like regex.match/find were
    pre-Cluster #4)
  - B1 cascade after this lands: -19 across 5 parser modules
    (separate session, mechanical)
- Total this cluster's landing (Stage C): -2 to -4 errors directly.
  Plus the structural unblock value of B1's mechanical cascade.

**Cost saved by full on-record entry:** prevents the named-in-passing
references from being re-derived in future audits. The 3 storage-
shape options are surfaced with structural reasoning so Stage C's
audit can read once and trust the framing — same model as the
zero-copy entry above.

---

## 2026-05-07 — Arc<TypedBuffer<T>> zero-copy marshal variants — named cluster (trigger fired)

This is **not** a defection. On-record promotion of a previously-
adjacency-deferred follow-up to a fully-named cluster, triggered by
the intrinsics-typed-CC audit surfacing a perf-sensitive consumer.

**Provenance: cluster #3 forward-compatibility door** (defections.md
lines 1075-1091, "Array<T> marshal extension" entry). When option β
(owned-clone) was chosen for the byte/numeric `FromSlot` family, the
entry explicitly anticipated:

> "Option β is forward-compatible: when a perf consumer arrives, the
> additional `Arc<TypedBuffer<T>>` impls land as their own round-trip
> with the consumer driving design choices."

**The trigger has fired.** The intrinsics-typed-CC audit (separate
entry below) identified the perf consumer: SIMD-optimized `vec_abs`,
`vec_sqrt`, `vec_ln`, `vec_exp`, `vec_add`, `vec_sub`, `vec_mul`,
`vec_div`, `vec_max`, `vec_min`, `vec_select`, `vec_add_i64` (and
the matrix / convolution / fft / scan / rolling / recurrence /
distributions / random intrinsic families). These are hot-path
numeric kernels operating on `Vec<f64>` / `Vec<i64>` arrays of
typically 10K-1M elements. Migrating them through option β's
owned-clone marshal layer would impose a ~10-100× regression per
call (one full element-by-element data copy on input, one on
output, dwarfing the SIMD inner loop).

**On-record trigger justification (load-bearing):** the user's
constraint is **perf-non-negotiable** — 10-100× regression is not
acceptable for the strict-typing migration. Deferring zero-copy
again would be the rename-pattern applied at the cluster-priority
layer: "next session, every session forever." Cluster #3's
forward-compat door was a deliberate "land β now, α when needed"
deal. The trigger condition has materialized; promoting to a named
cluster captures that explicitly.

**Anticipated architectural shape: α + ε in parallel with β.**

- **α (`Arc<TypedBuffer<T>>` zero-data-copy `FromSlot`/`ToSlot`)** —
  one `Arc::clone` (single atomic op) per `from_slot` call, **zero
  data clone**. Body declares `arr: Arc<TypedBuffer<f64>>` and
  accesses `&arr.data[..]` via `Arc::deref` — zero indirection in
  hot loops. Returns wrap `Arc<TypedBuffer<f64>>` into the slot via
  `Arc::into_raw(Arc<HeapValue>)` after wrapping in
  `HeapValue::TypedArray(TypedArrayData::F64(arc))` — same shape as
  β's owned-clone returns, just without the per-element copy.

- **ε (per-element-type parallel impls)** — distinct
  `impl FromSlot for Arc<TypedBuffer<i64>>`,
  `impl FromSlot for Arc<TypedBuffer<u8>>`,
  `impl FromSlot for Arc<TypedBuffer<f64>>`, etc., all declaring
  `NATIVE_KIND = NativeKind::Ptr(HeapKind::TypedArray)`. Body's
  declared parameter type selects which impl is monomorphized.
  Element-width discrimination is via the **Rust type system**
  (compile-time), not via a parametric `NativeKind`. This is
  identical to β's existing per-element-type impl shape — same
  precedent (cluster #3 entry, defections.md:1056-1066) directly
  reused, no new pattern invention.

- **β stays in production parallel with α.** Existing β consumers
  (`compress`, `archive`, `byte_utils`, `csv`, etc.) keep
  owned-clone semantics — they aren't perf-critical and don't
  benefit from zero-copy. The two impl families coexist, each
  selected by the body's declared parameter type.

**Already-rejected options (refuse on sight, no re-litigation):**

- **(γ) `FromSlotRef<'a>` trait variant.** Trait-family-proliferation
  risk. Rejected at cluster #3 entry (defections.md ~1060-1063).
  Two parallel traits (`FromSlot` + `FromSlotRef<'a>`) splits the
  marshal-API into a borrow-vs-own dichotomy that every future
  consumer has to navigate. Refused.
- **(δ) Parametric `NativeKind::TypedArrayI64` / `TypedArrayF64`
  variants.** Discriminator-level parametric explosion. Already
  refused on the `native_kind.rs:88-96` watchlist; carries the same
  shape rejection across α as it did across β. Refused.
- **(path 2) Per-element `HeapKind` split** —
  `HeapValue::TypedArrayI64` / `HeapValue::TypedArrayF64` etc. as
  fully-discriminative top-level variants. ~25-variant scope
  explosion. Rejected at cluster #3 entry on scope grounds. Same
  rejection applies for α.

**Open sub-decisions (for the architectural-extension commit when
sign-off relays):**

- Element-type coverage at first landing — `f64` + `i64` + `u8`
  cover the dominant intrinsic surface. `bool`, `i8`/`i16`/`i32` /
  `u16`/`u32`/`u64`/`f32` could land alongside or follow
  consumer-driven. Audit 1 of Stage B determines the minimum
  necessary set.
- Mutable-access shape — does any intrinsic need to *write* into
  a shared typed buffer (vs producing a new buffer on output)? If
  yes, that's a separate architectural sub-question (Arc-shared
  data is read-only without `Arc::get_mut`). Audit 1 of Stage B
  surfaces this.

**Explicit interlock with intrinsics-typed-CC cluster (named below):**
zero-copy must land **before** intrinsics migration. The intrinsics-
typed-CC entry's `(Q1)` sub-decision is fully resolved by zero-copy
landing — once `Arc<TypedBuffer<T>>` impls are in production, intrinsic
bodies migrate one file at a time using the existing marshal-layer
shape (`register_typed_fn_N`), no new architectural decisions per
file. Intrinsics-typed-CC's `(Q2)` calling-convention question
remains, but it becomes a yes-fold-to-marshal-layer answer once the
perf concern is resolved.

**Does not displace `move-semantics-marshal` (defections.md:1242).**
On-record disambiguation, BINDING:

| Workstream | Problem domain | Trigger |
|---|---|---|
| **Arc<TypedBuffer<T>> zero-copy (this entry)** | Shared **read** access to typed buffers without per-call data clone. Single atomic op, zero data copy. | Perf-sensitive consumer reading a typed array (intrinsics SIMD kernels). |
| **move-semantics-marshal (deferred)** | **Owned** data move at FFI boundary using `LoadLocalMove`/`LoadLocalClone` bytecode opcodes — eliminates a clone when the caller's local goes out of scope. | Different — consumer would need OWNED data, not shared read. |

The two workstreams are **complementary, not competitive**. Both
land independently against their own perf-trigger conditions.
Don't conflate.

**Performance characteristic re-confirms `no-dynamic-types` and
`Rust-like let` invariants:** α stays leaf-typed at the body's
parameter type — `Arc<TypedBuffer<f64>>` is a strict-typed Rust
type (no `dyn`, no `Box<dyn Any>`, no `ValueWord`). The marshal-
boundary discriminator is `NativeKind::Ptr(HeapKind::TypedArray)` —
unchanged from β. The element-width contract is at the body-side
Rust type level (per option ε pattern), not in slot bits. At the
FFI boundary specifically, α achieves the perf-equivalent of
move-semantics for shared immutable data (single atomic op + zero
data copy). The `Rust-like let` performance goal is achieved
end-to-end for intrinsic call paths once α lands.

**Disposition:** named cluster on-record. Stage B of the supervisor's
three-stage plan (sign-off granted on plan structure; architectural-
extension commits await per-stage sign-off relay). Audit 1+2+3
binding pre-work pending — see Stage B description above this entry's
landing window.

**Cost saved by promotion:** prevents the rename-at-cluster-priority
defection. Without a named cluster on-record, every subsequent
session's audit would re-encounter the cluster #3 forward-compat
door and re-defer "until a perf consumer arrives" — but the consumer
HAS arrived. Promotion cements the trigger. Estimated avoided
"perf-cost-deferred-indefinitely" rationalization: 4-8 weeks of
intrinsic-by-intrinsic perf regressions disguised as "small
migration cost we can profile later."

### 2026-05-07 — Audit-grounded correction: per-storage-variant body-type map

Audit-grounded correction in-place per finding #11. **The original
entry text above stays.** This dated subsection supersedes the
original framing of α's body type as uniformly `Arc<TypedBuffer<T>>`.
No contradictory new entry. No retroactive rewrite.

**Audit 2 finding (Stage B pre-work, 2026-05-07).** `TypedArrayData::F64`
stores `Arc<AlignedTypedBuffer>` (`crates/shape-value/src/heap_value.rs:482`),
**not `Arc<TypedBuffer<f64>>`**. `AlignedTypedBuffer` wraps
`AlignedVec<f64>` for 32-byte SIMD alignment
(`crates/shape-value/src/typed_buffer.rs:230`); it is not
interchangeable with `TypedBuffer<f64>` (whose `data` is `Vec<f64>`)
without a per-element copy.

The original entry text wrote α with body type `Arc<TypedBuffer<f64>>`
uniformly across all element types (lines 264-271 above). That
framing approximates. **The load-bearing structural property of
α + ε is per-element-type body types pinning storage variants via
the Rust type system — *not* uniform-element-type-storage-shape.**
A1 below is α-refinement, not α-alternative: it specifies α's per-
element-type ε-pattern impls more precisely than the original
entry's text. The α + ε load-bearing property (per-element-type
impls; body type pins storage variant via Rust types; NATIVE_KIND
uniform `Ptr(HeapKind::TypedArray)`; in-body pattern-match
consistency check) is preserved.

**Per-storage-variant body-type map (binding for Stage B's
architectural-extension commit):**

| Storage variant | Body type | First-landing? | Notes |
|---|---|---|---|
| `TypedArrayData::F64` | `Arc<AlignedTypedBuffer>` | ✅ yes | 78 intrinsic consumers (~85%); AlignedVec<f64> SIMD alignment preserved |
| `TypedArrayData::I64` | `Arc<TypedBuffer<i64>>` | ✅ yes | 12 consumers (~13%) — vector / rolling / array_transforms |
| `TypedArrayData::U8` | `Arc<TypedBuffer<u8>>` | ✅ yes | symmetry with existing β `Vec<u8>`; minor incremental scope |
| `TypedArrayData::Bool` | `Arc<TypedBuffer<u8>>` | ❌ deferred | Rust-type-collision with U8 — body type alone cannot disambiguate. Defer until consumer surfaces (likely newtype resolution). Only 2 sites currently use bool-as-f64-0/1 (scan_or, scan_and); deferral cost near-zero. |
| `TypedArrayData::I32` | `Arc<TypedBuffer<i32>>` | ❌ deferred | 1 helper consumer; follow consumer-driven |
| `TypedArrayData::Matrix` | `Arc<MatrixData>` | follow-on within Stage B | non-primitive storage; separate ε-style impl. Land if matmul / mat_add / mat_sub consumer migration needs it within Stage B; otherwise defer. **Don't add speculatively.** |
| Other variants (F32 / I8 / I16 / U16 / U32 / U64 / String / HeapValue / FloatSlice) | per-need | ❌ deferred | no current intrinsic consumer per Audit 1 |

**First-landing element-type set rationale.** Audit 1 enumerated
92 functions across 14 intrinsics files. f64 dominates (~85%);
i64 next (~13%); u8 covered for symmetry with already-landed β
`Vec<u8>` impl. Bool / i32 deferred per consumer count and (for
Bool) the Rust-type-collision-with-U8 disambiguation question.
Matrix as separate ε-style impl is a follow-on; don't add
speculatively.

**Options surfaced by the implementing agent (A1/A2/A3) — A1 chosen:**

- **(A1) Asymmetric body types per storage variant.** Each impl
  pins its storage variant via in-body pattern match. The Rust
  trait system enforces the body-side mapping. **Chosen.**
  Mirrors existing shape-vm precedent —
  `crates/shape-vm/src/executor/objects/typed_array_methods.rs:19`
  has `extract_float_array(args) -> &Arc<AlignedTypedBuffer>` and
  line 26 has `extract_int_array(args) -> &Arc<TypedBuffer<i64>>`
  side-by-side. Asymmetric body types are the **established
  pattern in this codebase**, not an invention of Stage B.
- **(A2) Symmetric `Arc<TypedBuffer<f64>>` body via copy-at-FromSlot.**
  Element-by-element copy of 10K-1M f64s on every call. **Refused
  on watchlist (perf-non-negotiable softening).** Direct
  rationalization shape — re-classifying intrinsics as not-
  really-hot-path is forbidden.
- **(A3) Refactor `TypedArrayData::F64` to hold `Arc<TypedBuffer<f64>>`.**
  Storage refactor; loses `AlignedVec<f64>`'s 32-byte SIMD
  alignment, regressing the existing SIMD inner-loop perf. Cross-
  crate workspace scope (touches shape-vm executor, JIT FFI, viz,
  wire). **Refused on cross-crate scope + AlignedVec SIMD
  alignment regression risk.** Out of Stage B scope; would need
  its own surface-and-decide round-trip.

**Additional alternatives (A4-A7) the supervisor checked; all
rejected on structural grounds:**

- **(A4) Generic dispatch via `Arc<dyn AlignedBuffer>` trait.**
  Re-introduces `dyn` at the marshal boundary. **Refused — no-
  dynamic-types defection** (`shape/CLAUDE.md` Forbidden Patterns).
- **(A5) Refactor `TypedBuffer<T>` to support optional alignment
  via const-generic.** Same cross-crate blast radius as A3 minus
  the storage-variant change. **Refused — probably net negative.**
- **(A6) Body declares via enum `MaybeAligned<f64>`.** Re-
  introduces dynamic dispatch in the body. **Refused — defection-
  shape (W-series rename pattern at body-type layer).**
- **(A7) Newtype wrapper `Arc<TypedBufferF64Aligned>`.** Adds
  indirection without solving the underlying storage-shape
  difference. **Refused — net negative.**

**Disposition:** A1 chosen; first-landing set f64 + i64 + u8;
supervisor sign-off relayed through user. Architectural-extension
commit candidate cleared to land (6 impls in `marshal.rs`:
FromSlot + ToSlot for each of `Arc<AlignedTypedBuffer>`,
`Arc<TypedBuffer<i64>>`, `Arc<TypedBuffer<u8>>`). Predicted
0 ± 3 errors. β stays parallel; no enum modifications; no
dispatch-table changes. **Microbenchmark verification gate
binding** (`vec_abs` / `vec_add` over a 10K-element f64 array
vs pre-bulldozer baseline) before declaring Stage B landing
complete; >10% regression = STOP and surface, do not proceed
to consumer migration.

**Finding #11 symmetry-extension.** This is the first audit-
grounded correction applied to a supervisor-authored on-record
entry. The original zero-copy entry (lines 226-371 above, dated
2026-05-07) was reviewed and signed off by the supervisor at
write-time; Audit 2 caught the calibration over-simplification
by verifying body type vs storage variant against current code
(`heap_value.rs:482`) rather than trusting the entry's framing.
Finding #11 stated "audit-grounded correction is binding for
prior on-record entries" without distinguishing entry-author.
**The rule applies symmetrically — supervisor-authored entries
are not exempt.** Lesson: structural reasoning at sign-off time
is not infallible against ground truth either. Verify against
current code, don't trust prior framing, even framing the
supervisor signed off on.

### 2026-05-07 — Second audit-grounded correction: α ToSlot impls dead at marshal layer

Second finding #11 symmetry-extension applied to this entry. **The
two prior subsections (lines 226-371 original entry + lines 372-484
"per-storage-variant body-type map" correction) stay on-record.**
This subsection captures the ToSlot-dead-at-marshal-layer finding
caught during Commit 5 (Q2 marshal-fold trial) preparation.

**Audit during Commit 5 prep (2026-05-07).** Verifying the
supervisor's sketched Commit 5 body against current code surfaced
that **Commit 2's α ToSlot impls (`Arc<AlignedTypedBuffer>`,
`Arc<TypedBuffer<i64>>`, `Arc<TypedBuffer<u8>>`) exist as trait
impls but are NOT consumed by the dispatcher's `TypedReturn → slot
push` path.** The dispatcher is at
`crates/shape-vm/src/executor/vm_impl/modules.rs:144-210`
(`invoke_typed_module_fn`); body returns are projected via
`typed_result.map(|t| t.into_value_word())` at line 208, which
pattern-matches on `TypedReturn` / `ConcreteReturn` variants
internally rather than dispatching through `ToSlot::to_slot()`.
The existing β `ToSlot` impls for `Vec<Arc<String>>` /
`Vec<Arc<HeapValue>>` (`marshal.rs:485-516`) sit alongside the
trait but are likewise not on the dispatcher's return-projection
call graph.

The original entry text wrote (lines 264-271):

> "Returns wrap `Arc<TypedBuffer<f64>>` into the slot via
> `Arc::into_raw(Arc<HeapValue>)` after wrapping in
> `HeapValue::TypedArray(TypedArrayData::F64(arc))` — same shape as
> β's owned-clone returns, just without the per-element copy."

That described the `ToSlot` trait's *logical operation* but elided
that production wrapping happens via the dispatcher's
`TypedReturn → ValueWord` projection, NOT via direct
`ToSlot::to_slot()` invocation. The α `ToSlot` impls land in
`marshal.rs` as the FromSlot pair-mate, but they don't reach the
production return path until a follow-on architectural extension
adds a `ConcreteReturn::ArrayAlignedF64(Arc<AlignedTypedBuffer>)`-
class variant *plus* a corresponding dispatcher projection arm.

**Implication for Stage B (binding for Commit 5 onwards):** the
Q2 marshal-fold trial (Commit 6 vec_add migration) lands with
**input-side zero-copy** (`FromSlot<Arc<AlignedTypedBuffer>>`
consumed at `register_typed_fn_2:851`) but **output-side
owned-clone** (body returns `ConcreteReturn::ArrayF64(Vec<f64>)`;
dispatcher allocates `AlignedTypedBuffer` from the `Vec<f64>`).
Mirrors β's owned-clone shape on the return side. The C5-B
architectural extension to add `ConcreteReturn::ArrayAlignedF64`
+ dispatcher projection is **deferred as a separate workstream**,
contingent on Q2 = marshal-fold passing the Commit 7 gate. If Q2
passes, C5-B becomes a follow-on optimization activating Commit
2's currently-marshal-dead α `ToSlot` impls and saving one
allocation + memcpy per output-bound intrinsic call.

**Why land Stage B's α `ToSlot` impls now if they're dead at the
marshal layer?** Cluster #3's option β chose owned-clone for input
deliberately; α's perf-non-negotiable trigger fired specifically
for input zero-copy on hot-path SIMD intrinsic *reads* (the
dominant cost for 78 of 92 intrinsic functions per Audit 1). The
Commit 2 architectural-extension commit lands the FromSlot half
that production *does* consume, plus the matched ToSlot impls as
pattern-completion (`FromSlot` and `ToSlot` are paired traits;
omitting one creates an asymmetric API surface). The ToSlot impls
also become live when C5-B lands. Calling them "dead" today is
correct for the marshal-layer projection path; calling them
"premature" would be wrong — they're load-bearing for C5-B's
follow-on, and adding them now keeps the Stage B `marshal.rs`
section internally consistent.

**Disposition for this subsection:** in-place correction logged.
Commit 6's Q2 trial proceeds with C5-A (input-zero-copy +
output-owned-clone). C5-B deferred as separate workstream — its
own architectural surface-and-decide round-trip after Q2 = marshal-
fold validates.

**Lesson re-asserted (finding #11 symmetry-extension, second
instance):** verifying against current code, not trusting prior
framing — the original entry was reviewed and signed off twice
(once at write-time, then implicitly when the per-storage-variant
correction landed). Both reviews missed that "wrap via
`Arc::into_raw(Arc<HeapValue>)`" describes a logical operation that
production routes through a different code path. Audits against
ground truth, not against prior framing — symmetric across all
on-record entries regardless of authorship.

### 2026-05-07 — Third audit-grounded correction: gate infeasibility on bulldozer-strictly-typed

Third finding #11 symmetry-extension applied to this entry. **The
three prior subsections (lines 226-371 original entry + per-storage-
variant correction + α-ToSlot-dead-at-marshal correction) stay
on-record.** This subsection captures the gate-infeasibility
finding caught during Commit 4 (bench harness) preparation.

**Audit during Q2 marshal-fold trial (Commit 6) bench-harness
preparation revealed:** the empirical gate as scoped requires
`shape-vm` benches to compile, which requires `shape-runtime --lib`
to compile, which is the **end state** of the strict-typing
reconstruction (currently 89 errors on `bulldozer-strictly-typed`
by design). Verified independently via `cargo bench -p shape-vm
--no-run`: fails on the 89 shape-runtime errors. Verified
specifically via `crates/shape-runtime/src/intrinsics/vector.rs:10`:
imports the (deleted) `shape_value::{ValueWord, ValueWordExt}`,
making the legacy `intrinsic_vec_add` body itself non-compiling.
All 14 intrinsics files have similar errors; **no single-file
migration unblocks the bench**.

**The gate cannot run pre-migration on this branch.** The original
gate framing implicitly assumed a workspace where legacy paths
still compile in parallel with new α impls. That assumption was
false on `bulldozer-strictly-typed` from day one of strict-typing
reconstruction. Audit-2 of Stage B pre-work caught body-type vs
storage-variant; this audit (during gate-prep) caught the
build-state vs gate-runnability gap that earlier framing missed.

**Correct gate methodology (binding from this subsection):** run
the gate as soon as bench-feasibility achieves — i.e., after
intrinsics-typed-CC cluster migration completes (and any cross-
cluster dependencies that gate `shape-vm` building also land).
Until then, **Q2 disposition stays "tentative-marshal-fold
pending empirical validation" — NOT "Q2 = marshal-fold landed."**

**Migration discipline mitigates the deferred-gate risk:**

- Each intrinsic file lands as its own atomically-revertible
  commit. No bundling. No "while I'm in this file, also
  migrate Y."
- Each commit message references "intrinsics-typed-CC migration;
  Q2 still tentative-marshal-fold pending empirical gate."
- Bench-feasibility check after each commit (`cargo bench -p
  shape-vm --no-run`); when it succeeds, gate runs.
- If the eventual gate fails, all intrinsics commits revert in
  series (each is atomic, makes this feasible) + Q2 lands as
  separate-path with a fresh architectural surface-and-decide.
  Estimated revert cost: 14 small reverts + Q2 redesign +
  intrinsics re-migration to separate-path shape; multi-day
  rework but bounded.

**Comparison methodology (binding for the eventual gate):**
theoretical SIMD-floor + dispatcher-overhead budget, NOT cross-
branch baseline (rustc/LLVM-version noise above signal floor for
a 10-25% gate threshold), NOT same-branch legacy baseline (which
doesn't exist on this branch since legacy intrinsic bodies don't
compile). For `vec_add` over 10K f64 arrays:

- SIMD compute floor: ~3-7us
- Body `Vec<f64>` alloc: ~1-2us
- Output `AlignedTypedBuffer` alloc + memcpy: ~2-3us
- Total expected if marshal-fold viable: ~5-12us

**Gate threshold:** per-call overhead delta over theoretical
floor <2us preferred, <3us acceptable, >3us = fail. (Slightly
relaxed from prior 1us/2us spec because absolute-vs-theoretical
comparison has more inherent uncertainty than same-branch legacy
comparison.)

**Meta-finding (finding #11 symmetry-extension, third instance —
binding-as-baseline for future audits).** Three audit-grounded
corrections in one session to framing the supervisor authored:
(1) per-storage-variant body-type map vs uniform `Arc<TypedBuffer<T>>`,
(2) α `ToSlot` dead at marshal layer vs production path,
(3) gate-runnability vs build-state on `bulldozer-strictly-typed`.
Common pattern: scoping architectural decisions without verifying
they're executable against current build state. **Future audit-1+2+3
pre-work binding addition: "verify against current build state"
alongside "verify against current code."** Specifically: any
empirical gate or measurement-based decision must include a
`cargo build`/`cargo bench --no-run`/equivalent runnability check
at scoping time. Captured here per finding #11 symmetry-extension;
not elevated to a new finding number — the audit-grounded-correction
discipline is the load-bearing rule, this is its scope clarification
for measurement-based decisions specifically.

**Disposition for this subsection:** in-place correction logged.
Q2 disposition explicitly tentative-marshal-fold pending empirical
gate. Consumer migration proceeds with per-file revert discipline.

### 2026-05-07 — Fourth audit-grounded correction: cross-crate dispatcher scope

Fourth finding #11 symmetry-extension applied to this entry.
**The four prior subsections (lines 226-371 original entry +
per-storage-variant correction + α-ToSlot-dead-at-marshal
correction + gate-infeasibility correction) stay on-record.**
This subsection captures the cross-crate-dispatcher-scope finding
caught during vector.rs migration preparation.

**Audit during vector.rs migration preparation revealed:** the
intrinsic-bodies-per-file framing in the gate-infeasibility
subsection ("each intrinsic file lands as its own atomically-
revertible commit") is incomplete. The 14 intrinsic files'
public functions (`vector::intrinsic_vec_add`, etc.) are called
from **shape-vm's dispatcher** at
`crates/shape-vm/src/executor/builtins/vector_intrinsics.rs:25-39`
(for vector intrinsics; analogous dispatcher sites for the other
13 files) via `BuiltinFunction::IntrinsicVec*`-class opcode match
arms, with `&[ValueWord]` argument slices. Migration of any
intrinsic from legacy `IntrinsicFn` to `register_typed_fn_N`
**changes the Rust signature** in a way that breaks shape-vm's
dispatcher call sites — and `ValueWord` is deleted, so the
legacy signature cannot exist on this branch in any form.

**The migration is inherently cross-crate.** Each file's commit
must update both:

- **shape-runtime side** — intrinsic body migration to
  `register_typed_fn_N` with typed Rust args.
- **shape-vm side** — dispatcher arm rerouting in
  `vector_intrinsics.rs` (or analogous file for other intrinsic
  groups) to look up via `module.typed_exports().functions.get(...)`
  and invoke the typed closure.

The two changes ship together as one atomic per-file commit;
splitting them creates a temporarily-broken interim state
(W-series rename pattern at the dispatcher layer).

**The original "shape-runtime-only commit per file" framing
missed this.** Audit-1 of Stage B pre-work enumerated body
shapes inside shape-runtime intrinsic files but did not trace
the cross-crate call graph from each public function to its
shape-vm consumer site. Audit-3 of Stage B caught the cross-
crate `TypedArrayData` consumer surface in shape-vm but didn't
apply the same lens to `BuiltinFunction::IntrinsicVec*` opcodes
— those are a parallel cross-crate scope that the audit missed.

**This fourth audit catch surfaces an additional discipline
addition (binding-as-baseline for future audits — this is the
fourth finding #11 symmetry-extension applied to supervisor-
authored framing in one session, baking the pattern in):**

The audit-1+2+3 binding pre-work for any architectural decision
now includes three specific verification disciplines, all under
finding #11's audit-grounded-correction scope:

1. **"Verify against current code"** (always — original
   finding #11 baseline).
2. **"Verify against current build state"** (for measurement-
   based or empirical-gate decisions — third instance, captured
   in the gate-infeasibility correction subsection above).
3. **"Verify against current cross-crate call graph"** (for
   architectural decisions touching dispatch / calling-conventions
   / public-API / serialization — this instance).

For each new architectural decision, the implementing agent
runs `rg`/`cargo check --workspace`/dispatch-table inspection at
scoping time to enumerate cross-crate consumers of the symbols
being changed. Supervisor sign-off includes verification that
the cross-crate call graph was checked.

These three disciplines aren't separate findings; they are scope
clarifications under finding #11's audit-grounded-correction
discipline. Future on-record entries cite finding #11 as the
load-bearing rule.

**Implication for the per-file migration plan (binding):**
the migration commits stay scoped per-intrinsic-file but expand
to include both crates. Predicted error windows per file:

- shape-runtime side: -3 to -8 errors per file (intrinsic body
  migration + use-statement removal + per-file removed-symbol
  cascades).
- shape-vm side: 0 to -1 error per file (dispatcher arm changes;
  mostly mechanical 1:1 reroute).
- Combined: -3 to -9 errors per file.

If shape-vm side touches >10 lines per dispatcher arm, that's a
sign Q2-marshal-fold-light is bigger than scoped — implementing
agent surfaces and stops. Each dispatcher reroute should be
~5-10 lines (one match arm fetching from `typed_exports` and
invoking).

**Disposition for this subsection:** in-place correction logged.
Q2 = marshal-fold-light per the intrinsics-typed-CC entry's
Q2-C correction subsection (cross-referenced). Migration
proceeds as cross-crate per-file commits with combined
shape-runtime + shape-vm changes.

### 2026-05-07 — Fifth audit catch + supervisor-side discipline addition

Addendum to the fourth-correction subsection above, surfaced
during vector.rs migration preparation. **Both prior framings
in the fourth-correction subsection — "shape-vm: 0 to -1 errors
per file" and "combined cross-crate per-file commits" —
overestimated the coherence of shape-vm's compile state on
this branch.**

**Fifth audit finding (specifics).** Verifying shape-vm against
ground truth (`rg "ValueWord" crates/shape-vm/`, `cargo check
-p shape-vm --lib`) revealed shape-vm's pre-existing ValueWord
breakage extends beyond the dispatcher arms in
`vector_intrinsics.rs:25-39` into the supporting infrastructure:

- `pop_builtin_args` (`crates/shape-vm/src/executor/vm_impl/builtins.rs:5`)
  returns `Vec<ValueWord>` (broken — `ValueWord` is deleted from
  `shape_value`).
- `invoke_typed_module_fn`
  (`crates/shape-vm/src/executor/vm_impl/modules.rs:147`) takes
  `args: &[ValueWord]` (broken — same root cause).
- shape-vm does not compile in isolation. The 89 shape-runtime
  errors cascade into a larger shape-vm error set; only
  shape-runtime's count is currently visible because shape-vm
  fails to compile at the dependency edge.

**Implication: combined cross-crate per-file commits are
infeasible.** The dispatcher routing change Q2-marshal-fold-light
requires would land alongside shape-vm's broader ValueWord
cleanup workstream (B4 core-foundation cluster or similar), not
as a per-file shape-vm change coordinated with each shape-runtime
intrinsic file migration. Per the intrinsics-typed-CC entry's
Q2 lifecycle three-stage transition subsection (cross-referenced;
M-A reframing), per-file migration commits land **shape-runtime
side only**. shape-vm dispatcher arms remain broken (cascade-
state) until shape-vm cleanup workstream runs; the
"shape-vm: 0 to -1 errors per file" estimate from the fourth-
correction subsection is structurally correct in the absolute
(no errors fixed) but the framing was misleading — shape-vm
won't measurably improve until its own broader cleanup converges.

**Supervisor-side calibration finding extension (binding-as-
baseline, fifth finding-#11-symmetry-extension instance in this
session — the discipline pattern is now incontrovertibly
established).** The first four instances added documented
disciplines to defections.md (verify against current code +
build state + cross-crate call graph). The fifth instance
happened anyway. Diagnosis: **documenting disciplines retro-
actively does not prevent the next instance unless the
disciplines are applied prospectively at sign-off time.** The
pattern "add discipline to log → next instance happens anyway"
is five instances in one session. The corrective is **APPLICATION
at sign-off time, not just DOCUMENTATION in the log.**

**Concrete operational change (supervisor-side, binding):**
Before signing off on any multi-crate or cross-crate
architectural decision, the supervisor (acting via the user's
relay) runs:

- `cargo check -p <each-affected-crate> --lib` — confirms
  build state of each affected crate independently.
- `rg "<symbol>" crates/` — enumerates cross-crate consumers of
  any moved/modified function/type/trait.
- Read of affected dispatch tables, public APIs, serialization
  formats — verifies the architectural assumption against
  current ground truth.

The supervisor brief (`~/.claude/plans/strict-typing-supervisor-
brief.md`) gives Bash access; supervisor uses it for verifying
own proposals at sign-off time, not just for verifying agent
reports. **Verify EVERY architectural assumption against current
ground truth before sign-off, not after agent catches a flaw.**

**Not a new finding number** — extension of finding #11
symmetry-extension. The audit-grounded-correction discipline is
the load-bearing rule; the prospective-verification clarification
is its operational application.

**Pattern reckoning (binding for the rest of this session AND
forward sessions):** five instances of the same supervisor-side
omission in one session is a meta-pattern that justifies explicit
calibration. The agent's stop-and-surface discipline has been
working through all five; the failure mode is on the supervisor
side. With the prospective-verification discipline now baseline,
future sign-offs should produce fewer execution-time corrections.
If the pattern recurs at a sixth instance, the next layer of
calibration is examining whether the supervisor brief itself
needs structural revision rather than additional in-place
clarifications.

**Disposition for this addendum:** fifth instance logged.
Migration proceeds per the intrinsics-typed-CC entry's Q2
lifecycle three-stage transition (M-A reframing): shape-runtime-
only per-file commits; shape-vm dispatcher routing deferred to
shape-vm cleanup workstream's natural scope.

---

## 2026-05-07 — intrinsics-typed-CC cluster (renamed from intrinsics-dispatch-table) — named on-record

This is **not** a defection. On-record cluster naming with
**rename**: previously referenced as "intrinsics-dispatch-table" in
prior session handovers; that framing was wrong.

**Audit-grounded rename rationale.** The 2026-05-07 audit (this
session) traced `IntrinsicsRegistry` (`intrinsics/mod.rs:41`) to
zero external consumers. shape-vm bypasses the registry entirely:
`BuiltinFunction::IntrinsicVec*` opcodes dispatch via a direct
match-on-builtin call into `shape_runtime::intrinsics::vector::*`
functions (`crates/shape-vm/src/executor/builtins/vector_intrinsics.rs:25-39`).
The "dispatch table" half of the cluster name was load-bearing in
the original handover framing; the audit confirms it is dead code.
Per finding #11 audit-grounded-correction discipline: name the
cluster by what it actually is, not by the dead-infrastructure
that was assumed to define it.

**The actual cluster surface** is the `IntrinsicFn` calling-
convention signature at `intrinsics/mod.rs:34`:

```rust
pub type IntrinsicFn = fn(&[ValueWord], &mut ExecutionContext)
    -> Result<ValueWord>;
```

`ValueWord` is deleted from shape-runtime; every body using this
signature fails to compile. 18 errors across 14 intrinsics files
(`array_transforms`, `convolution`, `distributions`, `fft`,
`math`, `matrix`, `random`, `recurrence`, `rolling`, `scan`,
`statistical`, `stochastic`, `vector`, plus `mod.rs`). Plus 4
errors in `multi_table/functions.rs` which uses the same
calling-convention shape (originally clustered with intrinsics
by handover-naming; audit-1 confirms the same architectural fate).

**Sub-decisions (open):**

- **(Q1) Owns-clone vs zero-copy for `Vec<f64>` / `Vec<i64>` /
  `Vec<bool>` `FromSlot`.** Cluster #3's option β chose owns-clone
  for byte/intrinsic arrays explicitly because no current consumer
  needed zero-copy. Intrinsics ARE the perf-sensitive consumer;
  per-call clone of 10K-1M-element f64 arrays would regress 10-100×
  vs the SIMD inner loop. **Resolution: blocked on `Arc<TypedBuffer<T>>`
  zero-copy cluster (entry above) landing first.** Zero-copy lands;
  (Q1) becomes "use Arc<TypedBuffer<T>>" mechanically.

- **(Q2) Marshal-layer-folded vs separate-typed-IntrinsicFn-path.**
  After (Q1) resolves via zero-copy, do intrinsics still need a
  separate calling-convention surface, or do they fold into
  `register_typed_fn_N`? Folding gains schema/LSP integration and
  uniform dispatch; a separate path could shave the dispatcher
  round-trip. Audit 2 of Stage C-or-after determines which.

- **(Q3) `IntrinsicsRegistry` deletion.** Confirmed dead code
  (zero external consumers). Mechanical deletion commit; no
  architectural choice. Can land before/during/after the
  calling-convention migration.

**DAG dependencies:**

- **`Arc<TypedBuffer<T>>` zero-copy cluster (above)** — blocking.
  Intrinsics migration cannot start until zero-copy lands.
- **B4 module-exports-core sub-decision** (defections.md:1644-1648)
  — `module_exports.rs` registry types still take `&[ValueWord]`,
  the same calling-convention shape. (Q2)'s resolution should be
  coherent with B4's answer. **Interlock confirmed**, but not
  blocking — (Q2) can land first if zero-copy is the only blocker
  and B4 module-exports-core agrees to follow the same answer.

**Predicted error-drop (Stage C-or-after, post-zero-copy landing):**

- Architectural extension (calling-convention migration of
  `IntrinsicFn`-shaped functions to typed marshal entries):
  0 ± 3 (consumer-fixing follow-on commits drop the actual errors).
- Per-file consumer migration: -1 to -3 per file × ~14 files =
  -14 to -42 total. The wide range reflects per-file size
  variability (matrix.rs has 5 errors, others have 1 each).
- `IntrinsicsRegistry` deletion: 0 errors (dead code; deletion
  is mechanical).

**Watchlist (binding when this lands):**

- Refuse "use β owned-clone for cold-path intrinsics, zero-copy
  for hot ones only" — splits the calling convention into hot/cold
  buckets, defection-attractor. All intrinsics use one shape.
- Refuse "extend `IntrinsicFn` to take typed slot bits without
  threading through marshal layer" — recreates a parallel
  calling-convention surface. The (Q2) decision should fold into
  marshal layer or commit to a separately-named typed-IntrinsicFn
  shape, not "IntrinsicFn but with typed bits."
- Refuse "register `IntrinsicsRegistry` on-load and dispatch via
  it" — dead-code-revival. The audit confirmed shape-vm bypasses
  it entirely; revival would re-attach a code path that nothing
  reads.
- Refuse "split intrinsics into shape-vm internal builtins" —
  cross-crate boundary violation; intrinsics live in shape-runtime
  per the workspace structure (`crates/shape-runtime/src/intrinsics/`).

**Disposition:** named cluster on-record, blocked on zero-copy
cluster landing first. Stage C of the supervisor's three-stage
plan covers this once Stage B (zero-copy) lands.

**Prior-handover-framing correction:** the "intrinsics-dispatch-table"
name appeared in the Phase 2c and Phase 2d handover documents and in
multiple prior defections.md entries as a sixth-named-cluster (e.g.
defections.md ~115 and elsewhere). **Going forward, the correct
name is "intrinsics-typed-CC" cluster.** Existing references in
older entries are not retroactively rewritten (drift-by-amendment-
elsewhere risk per finding #11); future entries use the correct
name.

### 2026-05-07 — Q2 evaluation methodology shift (gate-deferred-to-feasibility)

In-place dated subsection per finding #11 symmetry-extension.
Cross-references the zero-copy entry's third audit-grounded
correction subsection (the gate-infeasibility subsection,
`docs/defections.md` 2026-05-07 dated subsection at the
zero-copy entry).

**Q2 evaluation methodology shift.** Empirical resolution of (Q2)
"Marshal-layer-folded vs separate-typed-IntrinsicFn-path" shifts
from "before bulk migration" (the original sub-decision framing)
to **"as soon as bench is feasible after intrinsics-typed-CC
migration completes."** Trigger: `bulldozer-strictly-typed` is
non-buildable until the strict-typing reconstruction reaches its
end state; the Q2 microbenchmark gate as originally scoped
required a buildable workspace that doesn't exist on this branch.
See the zero-copy entry's third correction subsection for the
build-state gap that surfaced this shift, and the comparison
methodology (theoretical SIMD-floor + dispatcher-overhead budget,
not cross-branch or same-branch-legacy baseline since neither is
viable).

**Q2 disposition (binding until the gate fires):** "tentative-
marshal-fold pending empirical validation." **Not** "Q2 = marshal-
fold landed." This distinction is load-bearing — every commit
during intrinsics-typed-CC migration must reference Q2 as
tentative, never as resolved.

**Per-file revert discipline absorbs the deferred-gate risk:**

- One commit per intrinsics file. **NO BUNDLING.** No "while I'm
  in this file, also migrate Y."
- Each commit message references "intrinsics-typed-CC migration;
  Q2 still tentative-marshal-fold pending empirical gate."
- Bench-feasibility check after each commit (`cargo bench -p
  shape-vm --no-run`); when it succeeds, gate runs.
- Per-commit error-delta measurement is binding.
- If the eventual gate fails, all intrinsics commits revert in
  series. Atomic per-file commits are what enable this.

**Migration ordering (suggested):** vector.rs unary intrinsics
first (vec_abs / vec_sqrt / vec_ln / vec_exp — smallest, simplest,
single-input/single-output element-wise), then math.rs, then
vector.rs binary arithmetic (vec_add / vec_sub / vec_mul / vec_div
/ vec_max / vec_min / vec_select / vec_add_i64), then more
complex (matrix.rs, fft.rs, distributions.rs, random.rs,
rolling.rs, recurrence.rs, scan.rs, statistical.rs, stochastic.rs,
convolution.rs, array_transforms.rs, multi_table/functions.rs).
The intra-vector.rs split is a sub-file commit boundary
(not a "no-bundling" violation — different intrinsic-shape
sub-clusters within the same file land separately for revert
granularity).

**Watchlist (binding throughout migration):** the existing
intrinsics-typed-CC watchlist refusals (above) plus:

- Refuse updating Q2 disposition to "marshal-fold landed"
  before the gate fires. Keep "tentative" attribution explicit
  in every commit message until empirical validation.
- Refuse "the gate would have failed anyway, so skip it" —
  without measurement you don't know.
- Refuse "we're X commits in, reverting is too much work, just
  push through if marginal" — marginal-pass-rationalization.
  Threshold is the threshold.
- Refuse "synthetic bench as sanity check" — same shape as G4
  defection-attractor refused at last sign-off, under softer
  naming. Rename-to-less-suspicious-name pattern.
- Refuse bundling intrinsics-typed-CC migration with B1 parsers
  / B4 cluster work — separate clusters, separate sign-offs.

**Disposition for this subsection:** Q2 evaluation methodology
shift logged in-place. (Q1) stays resolved by zero-copy α + ε
landing (Commit 2 = `9af2882`). (Q2) stays tentative-marshal-fold
pending bench-feasibility gate. (Q3) `IntrinsicsRegistry`
deletion is unchanged — mechanical commit, can land before /
during / after migration with no architectural risk.

### 2026-05-07 — Q2 resolves at first-migration-commit, not at gate (Q2-C correction)

In-place dated subsection per finding #11 symmetry-extension.
**The prior Q2 evaluation-methodology-shift subsection above
stays on-record.** This subsection corrects two structural flaws
in that subsection's framing, surfaced during vector.rs migration
preparation. Cross-references the cross-crate-scope finding on
the zero-copy entry's fourth audit-grounded correction subsection
(forward reference; lands together as a coordinated documentation
update).

**Audit during vector.rs migration preparation revealed two
structural flaws in the prior X2 subsection:**

1. **Q2 cannot stay tentative through migration.** The previous
   subsection wrote "Q2 disposition stays 'tentative-marshal-fold
   pending empirical validation' until the gate fires." That
   framing is wrong. Migration of any single intrinsic file from
   legacy `IntrinsicFn` to `register_typed_fn_N` *requires* a
   coordinated cross-crate change to shape-vm's dispatcher
   (`crates/shape-vm/src/executor/builtins/vector_intrinsics.rs:25-39`
   for vector intrinsics; analogous dispatcher sites for the
   other 13 intrinsic files). The dispatcher routing change *is*
   Q2's resolution — there is no migration commit that can be
   "tentatively-marshal-fold." The choice is committed at the
   first migration commit (vector.rs), and every subsequent
   intrinsic file follows that pattern.

2. **The gate validates the chosen option, it doesn't choose.**
   "Bench-feasibility gate fires when shape-runtime --lib
   compiles" is correct for *measuring* the chosen option's perf
   characteristic. It is not the moment of architectural choice.
   That moment is the first migration commit's dispatcher
   routing.

**Corrected Q2 disposition lifecycle (binding):**

| Phase | Q2 disposition |
|---|---|
| Pre-vector.rs commit | **Tentative** — both options live; sign-off pending |
| At vector.rs commit (Commit 6) | **Committed-to-marshal-fold-light** pending gate validation |
| Through rest-of-intrinsics migration | Same — committed; each file's commit reaffirms |
| At first bench-feasibility (after Phase 2d completes) | Gate fires: validates committed-to choice |
| Pass | **Resolved as marshal-fold-light** — committed choice confirmed |
| Fail | **Revert all intrinsics commits in series** + fresh surface-and-decide for separate-path |

**Q2-A explicitly resolved as marshal-fold-light** (per
2026-05-07 supervisor sign-off relayed through user). Two scope
variants of marshal-fold were enumerated:

- **Q2-marshal-fold-light** (chosen): keep
  `BuiltinFunction::IntrinsicVec*` opcodes in the bytecode;
  reroute the dispatcher's match arms to look up via
  `module.typed_exports().functions.get("__intrinsic_vec_*")`
  and invoke the typed closure. Smaller cross-crate scope; opcode
  discriminants preserved (JIT specialization unaffected); each
  dispatcher arm becomes ~5-10 lines. Achieves the structural
  unification benefit of marshal-fold (single dispatch path
  through `typed_exports`).
- **Q2-marshal-fold-heavy** (deferred follow-on workstream):
  delete `BuiltinFunction::IntrinsicVec*` opcodes entirely;
  compiler emits regular typed-module-call opcode for
  `__intrinsic_vec_*`. Fully unified dispatch. Touches compiler-
  side emission (`crates/shape-vm/src/compiler/helpers.rs:3161-3175`
  + `matrix_ops.rs:294-298` + opcode definitions at
  `bytecode/opcode_defs.rs:2162-2455` + tests at
  `executor/tests/operator_overload.rs:518`). Cleaner end-state;
  bigger cross-crate scope. **Deferred as follow-on cleanup
  workstream after Q2 = marshal-fold-light validates via the
  eventual gate.**

**Structural reasoning (per supervisor brief).** Marshal-fold
preserves the typed-module-export API as the single dispatch
surface — the structural-enforcement principle (forbidden state
unrepresentable in the type system) extends to the dispatch
layer: one dispatch path, not two. Separate-path keeps a parallel
calling-convention which is the "smaller subset of existing
discriminator" defection-pattern at the dispatch layer. Light
variant of marshal-fold avoids compiler-emission scope while
still landing the structural unification.

**Per-file migration shape (binding for all 14 intrinsic files):**
each migration commit covers one intrinsics file's bodies
(shape-runtime side) AND updates shape-vm's dispatcher routing
for those intrinsics (shape-vm side) atomically. Cross-crate
revert is one commit, not many. No splitting shape-runtime +
shape-vm changes per-file.

**Disposition for this subsection:** Q2-A resolved as marshal-
fold-light; Q2-B per-file commits scoped as cross-crate;
Q2-marshal-fold-heavy deferred. Q2 disposition transitions from
"tentative" to "committed-to-marshal-fold-light pending gate
validation" at the vector.rs migration commit (Commit 6 in the
post-X4 ordering).

### 2026-05-07 — Q2 lifecycle three-stage transition (M-A reframing)

In-place dated subsection per finding #11 symmetry-extension.
**The two prior subsections (Q2 evaluation methodology shift +
Q2-C correction) stay on-record.** This subsection corrects the
"combined cross-crate per-file commit" framing to a three-stage
Q2 lifecycle, surfaced during vector.rs migration preparation
(fifth audit-grounded correction in this session).
Cross-references the zero-copy entry's fifth-finding addendum on
its fourth-correction subsection.

**Audit during vector.rs migration preparation revealed:**
shape-vm's pre-existing ValueWord breakage extends beyond the
dispatcher arms in `crates/shape-vm/src/executor/builtins/vector_intrinsics.rs:25-39`
into the supporting infrastructure:
`pop_builtin_args` (`crates/shape-vm/src/executor/vm_impl/builtins.rs:5`)
returns `Vec<ValueWord>`; `invoke_typed_module_fn`
(`crates/shape-vm/src/executor/vm_impl/modules.rs:147`) takes
`&[ValueWord]`. shape-vm does not compile in isolation — the
broken-state cascades from shape-runtime's 89 errors through
shape-vm's ValueWord references. The dispatcher arm changes that
Q2-marshal-fold-light requires would land alongside shape-vm's
broader ValueWord cleanup workstream (B4 core-foundation cluster
or similar), not as a per-file shape-vm change coordinated with
each shape-runtime intrinsic file migration.

**Implication: combined cross-crate per-file commits are
infeasible for intrinsics-typed-CC migration.** Each per-file
migration commit lands the **shape-runtime side only** (intrinsic
body migration to `register_typed_fn_N`). The shape-vm dispatcher
routing change (Q2-marshal-fold-light's dispatcher-side
implementation) is part of shape-vm's broader cleanup workstream.

**Corrected three-stage Q2 lifecycle (binding):**

| Stage | Trigger | Q2 disposition |
|---|---|---|
| Pre-vector.rs commit | — | **Tentative** — both options live; sign-off pending |
| **Stage 1 (architectural disposition)** | At vector.rs commit (Commit 7 in post-X6 ordering) | **Committed-to-marshal-fold-light**; code implementation pending shape-vm cleanup workstream. shape-runtime body migrations land per-file; shape-vm dispatcher arms stay as-is (broken cascade from ValueWord deletion). |
| **Stage 2 (code resolution)** | At shape-vm cleanup workstream's dispatcher routing implementation | **Code resolution lands** — shape-vm dispatcher arms reroute through `module.typed_exports().functions.get(...)` lookup. |
| **Stage 3 (validation)** | At first bench-feasibility (`cargo bench -p shape-vm --no-run` succeeds) | **Validated** by gate measurement, OR **failed** — revert all intrinsics commits + fresh surface-and-decide for Q2-separate-path. |

**Sub-option (a) for shape-runtime body migration (binding):**
DELETE the legacy `pub fn intrinsic_vec_*(args: &[ValueWord], ...)
-> Result<ValueWord>` functions during shape-runtime body
migration. Don't keep transitional stubs. shape-vm dispatcher
arms (in `vector_intrinsics.rs:25-39`) will reference function
names that no longer exist (broken in a different way than
before — but shape-vm was already broken; the error category
just shifts). Cleaner than transitional cruft (which would be
the W-series rename pattern at the legacy-stub layer).

**Per-file migration shape (binding for all 14 intrinsic files,
post-X6):** each migration commit lands shape-runtime side only.
No shape-vm changes per-file. Each commit message references
"intrinsics-typed-CC migration; Q2 architectural disposition
committed-to-marshal-fold-light, code resolution pending shape-vm
cleanup workstream."

**Disposition for this subsection:** M-A scope chosen; three-
stage Q2 lifecycle committed; combined cross-crate framing
superseded. Per-file commits are shape-runtime only. shape-vm
cleanup workstream's natural scope absorbs the dispatcher
routing implementation.

### 2026-05-07 — Predicted error-drop calibration correction (vector.rs empirical)

In-place dated subsection per finding #11 symmetry-extension.
**The original "Predicted error-drop" subsection above stays
on-record.** This subsection corrects the per-file estimate
based on vector.rs's measured outcome and per-file-histogram
inspection. Cross-references the zero-copy entry's fifth-finding
addendum (supervisor-side prospective-verification discipline,
`docs/defections.md` 2026-05-07 zero-copy entry, commit
`1688b91`).

**vector.rs empirical (Commit 7 = `9a0c574`):** measured -1
error. Predicted -3 to -8 (per the original subsection's
"-1 to -3 per file × ~14 files = -14 to -42 total" framing).
Outside the prediction window (>25% miss).

**Calibration mechanism (verified by per-file error histogram
inspection during the post-Commit-7 audit):** E0432
unresolved-import errors **cascade-suppress at file
granularity**, not per-symbol. When a file imports a deleted
symbol (`use shape_value::{ValueWord, ValueWordExt};`), Rust
emits one E0432 root error and treats subsequent references to
those symbols within that file as resolved-via-error-recovery.
Per-symbol cascade does NOT happen for these files. **Most
intrinsic files emit 1 root error from the import line; deleting
the import + bodies that depend on it drops 1 error.**

**Outliers exist:** files with ValueWord references at non-import
sites (function signatures using `&[ValueWord]` outside the
import-cascade scope, struct fields, etc.) emit additional
distinct E0425/E0433 errors that aren't suppressed by the import
cascade.

**Refined per-file expectation (binding for forward calibration):**

| File | Expected error drop | Notes |
|---|---|---|
| `vector.rs` | -1 (LANDED, `9a0c574`) | measured |
| `math.rs` | -1 | single-import-cascade |
| `fft.rs` | -1 | single-import-cascade |
| `scan.rs` | -1 | single-import-cascade |
| `rolling.rs` | -1 | single-import-cascade |
| `recurrence.rs` | -1 | single-import-cascade |
| `convolution.rs` | -1 | single-import-cascade |
| `array_transforms.rs` | -1 | single-import-cascade |
| `statistical.rs` | -1 | single-import-cascade |
| `stochastic.rs` | -1 | single-import-cascade |
| `random.rs` | -1 | single-import-cascade |
| `intrinsics/mod.rs` | -1 | single-import-cascade (after all 14 file migrations land, this becomes mechanical cleanup) |
| `distributions.rs` | -2 | non-import-cascade outlier |
| `matrix.rs` | -5 | non-import-cascade outlier; **architectural-adjacent surface required** at migration time (non-primitive `Arc<MatrixData>` storage, may need `ConcreteReturn::Matrix` extension) |
| `multi_table/functions.rs` | -4 | non-import-cascade outlier; **architectural-adjacent surface required** at migration time (last in migration order; flag any architectural surprise) |
| **Total intrinsics-typed-CC migration** | **~22-25 errors across 14 commits** | NOT the original -14 to -42 framing |

**Original "-14 to -42" estimate diagnosis:** the lower bound
(-14) was correct, derived from "1 per file × 14 files." The
upper bound (-42) was based on assuming per-symbol cascade where
the histogram shows file-level cascade. Without per-file
histogram inspection at scoping time, the upper bound was a
calibration over-estimate.

**Predict-vs-measure tally update (intrinsics-typed-CC
migration):** vector.rs 1/1 with the *refined* per-file estimate
(refined to -1 expected; measured -1 = exact). 0/1 with the
*original* -3 to -8 estimate (measured -1 = outside window).
Forward predictions use the refined per-file expectation.

**Cross-reference with zero-copy entry's fifth-finding addendum
(commit `1688b91`).** That subsection added the supervisor-side
prospective-verification discipline ("verify against current
build state + cross-crate call graph + per-file ground-truth
inspection at sign-off time"). The original predicted-error-drop
estimate above would have been refined at scoping time if the
per-file error histogram had been inspected — `cargo check -p
shape-runtime --lib --message-format=short --keep-going 2>&1 |
awk -F: '{print $1}' | sort | uniq -c` cleanly reveals the file-
granularity cascade pattern. Going forward, scope predictions
include per-file error-histogram inspection up front. This is
the discipline working applied retrospectively to update an
on-record entry, exactly the finding #11 symmetry-extension
shape.

**Disposition for this subsection:** in-place correction logged.
Forward per-file migration commits use the refined expectation.
matrix.rs and multi_table/functions.rs flagged for architectural-
adjacent surfacing BEFORE writing code (non-primitive return
shapes likely require `ConcreteReturn` extensions, an
architectural extension not consumer migration).

### 2026-05-07 — Multi-architectural files: partial-migration pattern + sub-decision queue

In-place dated subsection per finding #11 symmetry-extension.
**The four prior subsections (Q2 evaluation methodology shift +
Q2-C correction + Q2 lifecycle three-stage + predicted error-drop
calibration) stay on-record.** This subsection captures the
partial-migration pattern surfaced during math.rs (Commit 8 =
`0fd7959`) and the on-record sub-decision queue.

**Audit during math.rs migration preparation revealed:** some
intrinsic files are **multi-architectural-surface bodies** —
they contain a mix of intrinsics where some are clean
single-typed-input/single-typed-return migrations and others
have polymorphic shapes (polymorphic return, polymorphic input,
fast-path/slow-path duality) that can't fit single typed-marshal
entries. These files migrate **partially**: clean intrinsics
land via `register_typed_fn_N`; polymorphic ones stay as legacy
`IntrinsicFn` bodies pending follow-on architectural sub-decisions.

**math.rs (Commit 8 = `0fd7959`) is the canonical example.** 14
of 19 intrinsics migrated to typed marshal entries
(`create_math_intrinsics_module`); 5 intrinsics deferred:

| Function | Polymorphism shape | Sub-decision queue entry |
|---|---|---|
| `intrinsic_sum` | Polymorphic return: `i64` (Vec<int> fast path) vs `f64` (Vec<number>) | M1-split (cross-crate) |
| `intrinsic_min` | Polymorphic return + polymorphic input | M1-split (cross-crate) |
| `intrinsic_max` | Polymorphic return + polymorphic input | M1-split (cross-crate) |
| `intrinsic_char_code` | Polymorphic input: `Char` vs `String` | char_code multi-input-type dispatch |
| `intrinsic_bspline2_3d_batch` | Fast-path/slow-path: FloatArray vs generic Array | bspline consumer audit |

**Per-file expectation refinement (binding for forward
calibration):** ~30-50% of intrinsic files may be partial-
migration cases with **0 error drop** in their migration commit
(legacy bodies keep ValueWord import alive). Full migration
follows when the per-file sub-decisions land. **Partial-migration
0-drop commits are NOT failures** — they are the normal outcome
of decision-heavy architectural cluster work (mirroring Phase
2c's 8/8 calibration miss observation that "architectural
cluster work is decision-heavy, not code-heavy").

**Refined intrinsics-typed-CC migration total estimate:** ~15-20
errors total (down from the prior ~22-25 estimate from the
predicted error-drop calibration correction subsection above).
Several files will be in partial-migration state pending follow-
on sub-decisions. Full intrinsics-typed-CC error-drop happens
across multiple sessions: clean migrations now, follow-on
sub-decisions later.

### Follow-on architectural sub-decisions (on-record queue)

Sub-decisions surfaced by partial-migration audits during the
intrinsics-typed-CC migration. Each gets its own surface-and-
decide round-trip with audits; none bundled into consumer
migration commits.

1. **M1-split: per-element-type intrinsics for polymorphic-
   return intrinsics (`sum`, `min`, `max`).** Cross-crate change
   to shape-vm compiler emission (`crates/shape-vm/src/compiler/helpers.rs:3231-3236`),
   new opcode discriminants (`bytecode/opcode_defs.rs:2392-2396`),
   and classification logic analog of `classify_typed_vec_arithmetic`
   at `matrix_ops.rs:243`. Two typed entries per polymorphic
   intrinsic: `__intrinsic_sum_f64(Arc<AlignedTypedBuffer>) -> f64`
   and `__intrinsic_sum_i64(Arc<TypedBuffer<i64>>) -> i64`,
   compiler picks based on input type inference. Architectural
   extension; out of M-A scope.

2. **char_code multi-input-type dispatch (`Char` vs `String`).**
   `HeapValue::Char` is first-class (`heap_value.rs:846`); some
   user code holds Char values from `for c in s.chars()`-style
   iteration. Either two separate intrinsics
   (`__intrinsic_char_code_char(Arc<Char>) -> f64` and
   `__intrinsic_char_code_str(Arc<String>) -> f64`), or a new
   `register_typed_fn` variant that handles type-driven dispatch
   within the marshal layer. Architectural sub-decision.

3. **bspline2_3d_batch generic-array consumer audit.** Trace
   `math.shape:243` wrapper consumer + any user-code consumers;
   determine if FloatArray-only suffices or generic-array slow
   path is needed. If audit shows generic-array consumers exist,
   slow-path requires a `ConcreteReturn` extension or a
   different dispatch shape. Audit-only sub-decision (no code
   yet); architectural call follows audit.

4. **Possible others discovered during subsequent intrinsic file
   migrations.** Forward intrinsic file audits may surface
   additional polymorphism patterns. Append to this queue
   per-instance with the same shape.

These sub-decisions land **AFTER** intrinsics-typed-CC migration
of the clean cases completes (i.e., after all 14 intrinsic files
have had their clean intrinsics migrated and the polymorphic
ones queued here). Each sub-decision is its own surface-and-
decide round-trip; no bundling.

**Disposition for this subsection:** partial-migration pattern
on-record; sub-decision queue established. Forward intrinsic file
migrations follow the same pattern: audit per file, migrate clean
intrinsics, queue polymorphic ones.

### 2026-05-07 — Sub-decision queue extension: M1-split inventory + N1 Option<T> marshal

In-place dated subsection per finding #11 symmetry-extension.
**The five prior subsections (Q2 evaluation methodology shift +
Q2-C correction + Q2 lifecycle three-stage + predicted error-drop
calibration + partial-migration pattern + sub-decision queue) stay
on-record.** This subsection extends the sub-decision queue
inventory and adds **item #5 (N1 Option<T> typed marshal)**,
surfaced during the rolling.rs / scan.rs / recurrence.rs batched-3
audit (Commit X10 = `1f920ca`).

**Audit during rolling.rs / scan.rs / recurrence.rs batched-3
preparation revealed:**

1. **M1-split inventory under-counted.** The original queue item #1
   listed `sum`/`min`/`max` as the polymorphic-return cases. The
   forward audit on `rolling.rs` revealed `intrinsic_rolling_sum`,
   `intrinsic_rolling_min`, `intrinsic_rolling_max` carry the
   identical polymorphic-input shape (`try_extract_i64_slice` fast
   path → `option_i64_vec_to_nb` validity-bitmap return for sum;
   raw `Vec<i64>` for min/max — but min/max use `option_i64_vec_to_nb`
   for window-overrun pad cases too, see `rolling.rs:151,201`).
   **Inventory extended from 5 to 8 functions:** math `sum`/`min`/
   `max` + array_transforms `diff`/`cumsum` + rolling `sum`/`min`/
   `max`. Validity-aware-return-variant sub-question now has two
   confirmed consumers (`array_transforms::diff` and
   `rolling::rolling_sum`); `rolling_min`/`max` need verification at
   M1-split execution time whether their pad-case use of
   `option_i64_vec_to_nb` requires the same validity treatment or
   can use a default-value fallback.

2. **A new architectural sub-decision: N1 Option<T> typed marshal.**
   Three intrinsic bodies (scan.rs's `intrinsic_scan` initial-value
   third arg; recurrence.rs's `intrinsic_linear_recurrence`
   initial_value third arg; potentially future intrinsics with
   nullable-scalar-Shape-side optional args) take an optional value
   that is either a number or null at the Shape-side language level.
   Body code uses `args[i].is_none()` to discriminate. Current
   marshal-API surface has no `FromSlot for Option<T>` impl
   (verified via `rg "impl FromSlot for"
   crates/shape-runtime/src/marshal.rs`: only `i64`, `f64`, `bool`,
   `Arc<String>`, `Arc<DataTable>`, `Arc<IoHandleData>`, `Vec<u8>`,
   `Vec<i64>`, `Vec<Arc<String>>`, `Vec<Arc<HeapValue>>`,
   `Arc<AlignedTypedBuffer>`, `Arc<TypedBuffer<i64>>`,
   `Arc<TypedBuffer<u8>>`). `register_typed_fn_N_full`'s `required:
   false` + `default_snippet` synthesizes a typed default at compiler
   emission time (per `marshal.rs:933-935` "Bodies stay typed — the
   dispatcher always sees N typed args because the compiler
   synthesizes any missing trailing optional before emitting the
   call"); that solves arity-optional but not value-nullable.

**Updated sub-decision queue (binding):**

1. **M1-split: per-element-type intrinsics for polymorphic-input
   intrinsics (now 8 functions).** Math `sum`/`min`/`max` +
   array_transforms `diff`/`cumsum` + rolling `sum`/`min`/`max`.
   Cross-crate change to shape-vm compiler emission +
   classification logic + new opcode discriminants. Validity-aware-
   return-variant sub-question for `diff` + `rolling_sum` (and
   possibly `rolling_min`/`max`). Architectural extension; out of
   M-A scope. (Prior queue item #1, inventory extended.)

2. **char_code multi-input-type dispatch (`Char` vs `String`).**
   Unchanged. (Prior queue item #2.)

3. **bspline2_3d_batch generic-array consumer audit.** Unchanged.
   (Prior queue item #3.)

4. **Possible others discovered during subsequent intrinsic file
   migrations.** (Prior queue item #4.)

5. **N1: `FromSlot for Option<T>` typed marshal.** Marshal-API
   extension to add `Option<T>` `FromSlot` for at-minimum
   `Option<f64>` (and likely `Option<i64>`, `Option<bool>`).
   Architectural shape: `from_slot()` checks `slot.is_none()` at
   decode time and returns `Option::None` if so, `Some(T::from_slot(slot))`
   otherwise. **NATIVE_KIND must remain a single value** —
   discriminator on null-vs-value at the slot decode layer, NOT a
   sentinel within `T`'s value space (NaN-as-null, Infinity-as-null,
   etc. all on watchlist as "Sentinel values for absent optional
   args"). Confirmed consumers: `scan.rs::intrinsic_scan`'s
   third arg (initial value, with additional bool-or-number 3-way
   polymorphism that even N1 alone doesn't resolve), and
   `recurrence.rs::intrinsic_linear_recurrence`'s third arg
   (initial_value, plain `Option<f64>`). scan.rs has zero
   stdlib/package consumers per post-bulldozer rg
   (`__intrinsic_scan` registered at `intrinsics/mod.rs:320` with
   no callers in `stdlib-src/` or `packages/`); deletion-candidate
   for shape-vm cleanup workstream, which may reduce N1's
   urgency. Architectural extension; out of M-A scope; surface-
   and-decide round-trip needed.

**Sub-decision queue items remain on-record-only.** Adding to the
queue ≠ approval to execute. Each item requires its own surface-
and-decide round-trip with audit-1+2+3 binding pre-work +
supervisor sign-off + structural reasoning. The queue is
documentation of known forward sub-decisions, not a work plan.

**rolling.rs commit landed (Commit X10 = `1f920ca`):** 3 of 6
intrinsics migrated to typed marshal (`__intrinsic_rolling_mean`,
`__intrinsic_rolling_std`, `__intrinsic_ema`); 3 deferred
(`__intrinsic_rolling_sum`/`min`/`max`). Predicted error drop:
0 ± 1 (partial-migration pattern; legacy bodies retain ValueWord
import). Measured: 0. **Within window.** Decision-heavy pacing
pattern is binding baseline; per-file 0-drop is normal not anomaly.

**scan.rs and recurrence.rs full-defer:** both blocked on N1
(architectural sub-decision; out of M-A scope). No commits land
for these files until N1 round-trip resolves. scan.rs additionally
has a bool-or-number 3-way polymorphism on its initial-value
third arg that even N1 alone may not resolve; second-order
sub-question to evaluate at N1 sign-off time.

**Disposition for this subsection:** sub-decision queue inventory
extended; N1 added as item #5; rolling.rs partial migration
landed; scan.rs and recurrence.rs full-defer pending N1.

---

## 2026-05-07 — Phase 2d Array cluster post-mortem — predict-vs-measure within window (-7 of -7..-10)

This is **not** a defection. On-record calibration outcome from
the Phase 2d Array cluster landing across 4 commits.

**Predict-before-measure (per finding #12 binding discipline):**

| Commit | Predicted | Measured | Delta vs prediction |
|---|---|---|---|
| 1 (architectural extension) | 0 ± 3 | 96 → 96 (0) | exact |
| 2 (csv_module migration) | -2 to -4 | 96 → 92 (-4) | exact (upper bound) |
| 3 (arrow_module migration) | -3 | 92 → 89 (-3) | exact |
| 4 (process_ops migration) | 0 to -2 | 89 → 89 (0) | exact (lower bound) |
| **Total** | **-7 to -10** | **96 → 89 (-7)** | **within window (lower bound)** |

Predict-vs-measure success rate: **4/4 in window** (Phase 2c was
1/8). The audit-1+2+3 pre-execution discipline introduced in finding
#12 + Phase 2c continues to pay off. This is the second consecutive
session with all sub-cluster predictions in window after audits.

**Calibration sub-finding (small):** the audit-2 surfacing of "17
files have `match TypedArrayData::*` sites — all need new arms"
was a slight over-count. Actual: **only 1 file (heap_value.rs
itself) needed exhaustiveness updates**. The other 16 files use
specific-arm patterns (e.g. `HeapValue::TypedArray(TypedArrayData::I64(arr)) =>
{...}`) inside outer matches with wildcard fallback, which adding
new variants does NOT break. The prediction baseline assumed
"variant addition implies exhaustiveness churn"; reality is
"variant addition only churns _exhaustive_ match sites, not
specific-arm-with-wildcard sites." Same diagnostic shape as
finding #10's stale-import miscount: the audit lookup was at the
wrong granularity.

**Cost saved by leaf-first DAG ordering** (per finding #12):

- Each consumer migration was a clean per-file commit (csv_module
  92→92, arrow_module 92→89, process_ops 89→89). No cross-cutting
  rebases or interim shapes.
- B1 sub-decision #1 is now resolved (the JsonValue::Array runtime
  shape uses the same `TypedArrayData::HeapValue` variant). B1 is
  one step closer to leaf eligibility.
- Process_ops, csv_module, arrow_module are no longer cluster-#2
  / cluster-#3 / cluster-#5 blockers. The main remaining clusters
  (B4 core-foundation, Cluster #4 Option, B1 JsonValue residual,
  intrinsics-dispatch-table) are eligible to land in the order
  finding #12 suggested.

**Watchlist refusals (all sustained, none re-litigated):**

- `ConcreteReturn::Array(Vec<ConcreteReturn>)` recursive: not
  introduced. Maintained leaf-only invariant.
- Per-element-kind `TypedArrayData::DataTable` / `IoHandle` /
  `String`-of-strings variants: not introduced. Maintained
  unparametric-NativeKind constraint.
- `as_typed_array_string()` helpers on HeapValue: not introduced.
  Body-side monomorphization via `FromSlot for Vec<Arc<String>>`
  was the path, mirroring cluster #2's option γ.
- "Rename Vec<Arc<X>> to ValueArray": not entertained. Each
  `FromSlot`/`ToSlot` impl declared its concrete element type at
  the trait-impl level.

**Pacing observation:** Phase 2c's "architectural cluster work is
decision-heavy, not code-heavy" framing held this session — the
surfacing-and-deciding (audits + 3 sub-decisions) consumed about
half the session; the 4 commits (1 architectural + 3 mechanical)
landed in the other half. Total session: ~7 errors dropped, plus
multi-cluster unblock. The unblock value is structural and will
be measured in subsequent sessions when B1 / B4 / Cluster #4
sub-decisions can land cleanly.

---

## 2026-05-07 — Phase 2d Array cluster — TypedArrayData::String + TypedArrayData::HeapValue extension (LANDED)

This is **not** a defection. On-record landing of the Phase 2d Array
cluster leaf decision identified by calibration finding #12 (cluster
DAG ordering). Extends the cluster #3 `Array<T>` marshal (option β,
2026-05-06) with the `String` and `HeapValue` element-storage arms
that the prior session deferred.

**Identified as a leaf cluster** (no open dependencies on B1, B4,
Cluster #4, or intrinsics-dispatch-table) per finding #12. Multi-
cluster unblock: process_ops `Array<string>` input, csv_module
`Array<Array<string>>` rows, arrow_module `Array<DataTable>`,
B1 sub-decision #1 (`JsonValue::Array` projection), Phase 2d
sub-cluster #4 path utilities.

**Pre-execution audits (per Phase 2c/2d binding discipline):**

- **Audit 1 (consumer-call-shape):** consumer count clarified —
  csv_module 4 errors (Array<Array<string>> + Array<HashMap<...>>),
  arrow_module 3 errors (Array<DataTable>), process_ops 0 visible
  (stubbed). The vmarray_from_vec users in `json/yaml/toml/msgpack/xml`
  belong to **B1 JsonValue cluster, not Phase 2d Array** — they build
  recursive JsonValue arrays and are blocked on B1 sub-decision #1
  (which itself was found to depend on Phase 2d Array landing). The
  `intrinsics/{matrix,distributions,...}` and `multi_table/functions`
  vmarray_from_vec users belong to **B4 / intrinsics-dispatch-table
  clusters, not Phase 2d Array**. Eighth confirmed instance of the
  directory-adjacency cluster fallacy avoided.
- **Audit 2 (marshal-API surface):** revealed that
  `ConcreteReturn::ArrayString(Vec<String>)` and
  `ConcreteType::ArrayString` were already present in
  `typed_module_exports.rs:67/149` from prior partial work (5
  production sites in `file_ops`/`regex`/`unicode`/`file`), and
  `wire_conversion.rs:227` was already pre-wired to project
  `TypedArrayData::String(buf)` to `WireValue::Array` of strings.
  The runtime-level `TypedArrayData::String` variant + `FromSlot`
  for `Vec<Arc<String>>` were the missing pieces. For `HeapValue`
  arrays, all of variant + FromSlot + ToSlot + ConcreteReturn
  variant + ConcreteType variant were missing.
- **Audit 3 (cluster DAG):** confirmed leaf status. None of the
  5-7 B4 sub-decisions, B1's 5 sub-decisions, Cluster #4's
  prelude-vs-import question, or the intrinsics-dispatch-table
  IntrinsicFn calling convention has any backwards-edge into
  Phase 2d Array's storage-shape decision.

**Architectural sub-decisions (3 internal — surfaced before
execution; user-decided in one round-trip):**

| # | Sub-decision | Resolution |
|---|---|---|
| **A** | `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)` | Land. Mirrors existing `TypedArrayData::I64`/`F64`/etc. variant shape. Element type `Arc<String>` is the canonical refcounted-string shape. |
| **B** | `TypedArrayData::HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)` | Land. Element-kind discrimination is a body-side type contract (option ε pattern from cluster #3). Per spec: "the kind tells you the arm; HeapValue dispatch is a consistency check, not a probe." |
| **C** | ConcreteReturn variant for heap-element arrays | **(γ)** Single generic `ConcreteReturn::ArrayHeapValue(Vec<Arc<HeapValue>>)`. Matches cluster #3's option β/ε philosophy: don't carry element-kind in the discriminator; body-side Rust types enforce homogeneity. Rejected (δ): per-element-kind variants (`ArrayDataTable` / `ArrayIoHandle` / etc.) — same parametric-explosion shape as path-2 / option-δ rejected at cluster #3 entry. |

**Watchlist refusals (binding, all sustained):**
- `ConcreteReturn::Array(Vec<ConcreteReturn>)` recursive — refused
  (breaks leaf-only invariant; cluster DAG would loop).
- Per-element-kind `TypedArrayData::DataTable` / `IoHandle` /
  `String`-of-strings variants — refused (parametric HeapKind
  explosion, same shape as path 2 rejected at cluster #3).
- `as_typed_array_string()` helpers on `HeapValue` that hide the
  typed correspondence — refused (α-shape rejected at cluster #2).
- "Rename Vec<Arc<X>> to ValueArray" or similar surface rename —
  refused (CLAUDE.md forbidden pattern).

**Code shape (this commit):**

- `crates/shape-value/src/heap_value.rs`:
  - `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)` variant
  - `TypedArrayData::HeapValue(Arc<TypedBuffer<Arc<HeapValue>>>)` variant
  - `type_name()`, `is_truthy()`, `Display::fmt` arms for both
- `crates/shape-runtime/src/marshal.rs`:
  - `FromSlot for Vec<Arc<String>>` / `Vec<Arc<HeapValue>>` —
    follow `Vec<u8>` template, panic-on-mismatch consistency check
  - `ToSlot for Vec<Arc<String>>` / `Vec<Arc<HeapValue>>` — wrap
    into `TypedBuffer` then `Arc<HeapValue>::into_raw` to obtain
    slot bits
- `crates/shape-runtime/src/typed_module_exports.rs`:
  - `ConcreteReturn::ArrayHeapValue(Vec<Arc<HeapValue>>)` variant
  - `ConcreteType::ArrayHeapValue(String)` variant — caller-provided
    user-facing type-name string for LSP
  - `shape_type_name()` arm

**Direct error-drop:** ~0 (consumers not yet migrated; the variants
add capability without reducing existing error count). The drop
comes from commits 2-4 (csv_module / arrow_module / process_ops
migrations). Predicted total -7 to -10 across this round-trip.

**Calibration outlook:** if the architectural extension surfaces
previously-hidden errors (analog of d716482's wrap_legacy-deletion
revealing-errors pattern, or finding #9's correctness vs compile
distinction), the error count may go UP on this commit. That is
the same diagnostic shape as findings #5/#9 — surface and re-audit
if it happens, don't suppress. Predicted error-drop window for
commit 1: 0 ± 3.

**Commit pacing:** per cluster #2 file_ops precedent, per-consumer
migration commits (2-4) follow this leaf-extension commit. One
commit per logical unit, all in-session. Bisect-friendly: commit 1
is the architectural extension; commits 2-4 each migrate a single
stdlib module to the new shape.

**Cost saved:** prevented forcing per-consumer-kind ConcreteReturn
variants (option δ) which would have grown linearly with each new
heap kind that returns through stdlib (DataTable, IoHandle, future
heap kinds). The single `ArrayHeapValue` variant absorbs all of
them via body-side type contract, mirroring how cluster #3's
option β handled the element-width problem at the input side.
Estimated avoided variant-count growth: 5-8 ConcreteReturn variants
over the next year. Plus the avoided "should this be ArrayDataTable
or ArrayHeapValue<DataTable>" follow-up debate if the per-kind path
had been taken.

---

## 2026-05-06 — JsonValue marshal extension — deferred (parser cluster blocker)

This is **not** a defection. On-record deferral of the `JsonValue`-return
marshal extension. The Phase 2b unified marshal entry already established
that monomorphized typed sum-types are the strict-typed answer for parser
returns; what remains is the architectural decision on runtime
representation and the dispatcher projection that wires it through.

**Considered:** add `TypedReturn::JsonValue(JsonValue)` as a new top-level
variant and project it at the marshal boundary into a Shape-side `JsonValue`
enum (per `~/.claude/plans/stop-native-vs-tagged-tax.md` line ~17 and the
2026-05-06 typed-`JsonValue` entry below). Migrate the five parser modules
(json/yaml/toml/msgpack/xml) to return `JsonValue` from their bodies.

**Architectural decision needed (next session):**

1. **Runtime representation.** `JsonValue` is a typed sum-type with 8
   variants, each carrying a payload. The defection-watchlist entry
   (HeapKind trim + NativeKind::Ptr extension) explicitly **rejected**
   parametric NativeKind variants like `NativeKind::JsonValue`. The
   strict-typed answer is `HeapKind::TypedObject` plus a `schema_id`
   per the v2 spec — each `JsonValue` variant gets a monomorphized
   schema (one for `JsonValue::Null`, one for `JsonValue::Bool`, etc.,
   or alternatively a single schema with a discriminant field). The
   schema design choice (per-variant schemas vs single schema with
   discriminant) is the open question.
2. **Shape-side enum wiring.** The Shape-side `JsonValue` enum
   declaration must be in scope at parser registration time so the
   compiler can resolve `match parse_json(s) { JsonValue::Null => ... }`.
   Either prelude-bake it (like `Result<T, E>` / `Option<T>`) or
   require explicit `import { JsonValue } from std::core::json`.
3. **Dispatcher projection.** The marshal dispatcher must walk the
   Rust `JsonValue` recursively and project each variant into the
   matching TypedObject schema. Recursive projection has a
   stack-depth concern for deeply-nested JSON.

**Pattern recognized:** This is **not** runtime tag-decode dispatch; it
is monomorphized typed-sum-type construction, which the strict-typing
plan endorses. The deferral exists because the architectural decisions
(1)/(2)/(3) above shape the marshal-layer signatures, and committing
half-blind would risk signature redo.

**Alternative taken:** defer the `TypedReturn::JsonValue` variant + the
five parser-module migrations to the next session, alongside the
shape-vm cascade. The on-record entry names the architectural decisions
needed so the next session can land them with one round-trip
surface-and-decide each.

**Consumer count blocked:** ~19 errors across `stdlib/json.rs` (4),
`stdlib/yaml.rs` (3), `stdlib/toml_module.rs` (5), `stdlib/msgpack_module.rs`
(3), `stdlib/xml.rs` (4).

**Cost saved:** prevented signature-redo from picking parser-side-blind
runtime representation. Estimate: 1-2 days of "audit Shape-enum wiring
choice and re-touch dispatcher projection" follow-up avoided.
Acknowledged: 19 errors remain in shape-runtime --lib until the
extension lands.

### 2026-05-07 — Audit-grounded correction (Phase 2d sub-cluster B1)

The 2026-05-06 entry above framed JsonValue as a single deferred
extension blocking 5 parser modules. Phase 2d sub-cluster B1
Audit 1 + Audit 2 (per the now-baseline pre-cluster-execution
discipline) revealed the framing was incomplete on two axes.

**(a) File count: was 8, audit revealed 5.** Phase 2d's broader
A/B/C audit lifted 8 files into the JsonValue cluster on the basis
of shared stale-import shape (csv_module, xml, json, msgpack_module,
yaml, toml_module, arrow_module, http). Audit 1 (consumer-call-shape)
on the function bodies revealed three are NOT JsonValue-shaped:

- `csv_module` — body returns `Array<Array<string>>` /
  `Array<HashMap<string, string>>`. Cluster identity: **cluster #3
  Array<T> + Array<string>-marshal sub-cluster** (Phase 2d sub-cluster
  #3 of process_ops). Uses `vmarray_from_vec` + `ArgVec`, not
  recursive JsonValue.
- `arrow_module` — body returns `Result<Array<DataTable>, string>`
  via deleted `TypedReturn::ArrayValueWord`. Cluster identity:
  **cluster #3 Array<T> (Array<DataTable>) plus
  ArrayValueWord-cleanup sub-cluster**.
- `http.rs` — body builds object responses
  `{status, headers, body, ok}` via `from_hashmap_pairs` and parses
  options via `as_hashmap`. Cluster identity: **HashMap-marshal +
  TypedObject-with-recursive-HashMap sub-cluster** (new on-record
  micro-cluster).

The original 2026-05-06 entry's "five parser modules
(json/yaml/toml/msgpack/xml)" framing was correct. The 8-file
expansion was the seventh instance of the directory-adjacency
cluster fallacy — same imports, different cluster identities.

**(a.1) 2026-05-07 — HashMap-marshal micro-cluster gains csv consumers.**
The Phase 2d Array cluster landing migrated csv.parse, csv.stringify,
csv.read_file, csv.is_valid to the typed marshal layer (commits
9f6b1d3 and earlier in this same workstream). The two remaining
csv functions — **csv.parse_records** (returns
`Array<HashMap<string, string>>`) and **csv.stringify_records**
(consumes `Array<HashMap<string, string>>`) — were deferred for
exactly the same reason `http.rs` was: `HeapValue` has no `HashMap`
variant in the strict-typed runtime. The architectural decision is
shared: a single HashMap-marshal landing unblocks both
`http.rs`'s response-object construction *and* csv's record-row
construction. Cluster identity confirmed: **HashMap-marshal
micro-cluster, two consumer sites (`http.rs` + `csv_module.rs`),
one architectural decision.** Do not split into per-consumer
sub-decisions — the storage shape (whether a new
`HeapValue::HashMap` variant, a `TypedObject`-with-keys/values
projection, or some `from_hashmap_pairs`-equivalent) is the same
question for both. See `crates/shape-runtime/src/stdlib/csv_module.rs`
deferred-list comment for the consumer-side hold.

**(b) Sub-decision count: was 3, audit revealed 5 with interlocks.**
The 2026-05-06 entry listed three architectural decisions
(runtime representation, Shape-side enum wiring, dispatcher
projection). Audit 2 (marshal-API surface) revealed B1 actually
decomposes into five, two of which interlock with already-on-record
clusters:

| # | Sub-decision | Interlock |
|---|---|---|
| **1** | `TypedArrayData::String` and/or `TypedArrayData::HeapValue` for storing `JsonValue::Array(Vec<JsonValue>)`. Currently `TypedArrayData` (`crates/shape-value/src/heap_value.rs:470`) has 13 numeric/Matrix variants — no String, no HeapValue. | **Interlocks with Phase 2d sub-cluster #3 (process_ops Array<string>)** + the cluster #3 Array<T> family. Single architectural call unblocks JsonValue::Array projection AND Array<string> consumers AND csv_module + arrow_module rows. |
| **2** | `JsonValue::Object(Vec<(String, JsonValue)>)` runtime shape — two-slot TypedObject `{keys: Array<string>, values: Array<HeapValue>}`? HashMap-shaped `from_hashmap_pairs`? new HeapKind variant? | Net new; depends on (1). Also interlocks with `http.rs`'s response-object micro-cluster. |
| **3** | Per-variant schemas (8 monomorphized schemas, one per JsonValue arm) vs single schema with discriminant slot. Watchlist (native_kind.rs:88-96) **forbids** single+discriminant — that is the W-series shape at the schema layer. **Effectively forced answer: per-variant schemas.** Open: per-variant schema *registration strategy* (compiler-synthesized at enum-decl time vs stdlib-pre-registered at module init). | Mostly settled by watchlist. The registration-strategy sub-question is real but smaller. |
| **4** | Shape-side `JsonValue` enum visibility — prelude-bake or `import { JsonValue } from std::core::json`? | **Interlocks with Cluster #4 Option<T>** which has the same prelude-vs-import question. Resolving Cluster #4 first answers (4) for free. |
| **5** | Recursive marshal-side projection — recursive vs iterative; stack-depth bound for deeply-nested JSON. | Net new. Probably the smallest of the five once (1)+(2) settle. |

**B1 is an INTERIOR node in the cluster DAG, not a leaf.** Two
sub-decisions (#1 and #4) interlock with leaf clusters that are
themselves on-record deferred. Resolving the leaves first is the
correct order — see calibration finding #12.

**Phase 2d B1 disposition:** path β — surface-and-defer. No
marshal-API extension committed this session. The on-record entry
above is updated by this correction. Next-session priority is
re-ordered per the cluster DAG (see finding #12). Predicted error
drop for B1 residual *after* leaf clusters land: ~-19 (the 5 parser
modules' errors clear in one go once sub-decisions #1 and #4 are
resolved upstream).

**Watchlist crystallization (eighth instance):** "audit-grounded
correction is binding for prior on-record entries." When an audit
surfaces a framing flaw in an existing entry, update the entry in
place with a dated correction subsection — don't create a new
entry that contradicts the old one. The log's value is that future
sessions can read each entry once and trust its current state.
Drift-by-amendment-elsewhere defeats that.

### 2026-05-07 — Post-leaves audit: residual reduces 5→1 architectural

After Phase 2d Array (commits `9fc35ac`/`9f6b1d3`/`29d61fa`) and
Cluster #4 Option (`ed18cb8`/`c07d18e`) landed, two of the five
B1 sub-decisions self-resolve, two were never architectural to
begin with, and only one remains as a genuine architectural
question. Audit-grounded correction per finding #11 in-place
discipline:

| # | Sub-decision | 2026-05-07 status |
|---|---|---|
| **1** | TypedArrayData::String / HeapValue for Json::Array | **RESOLVED** by Phase 2d Array (commit `9fc35ac`). `TypedArrayData::String` and `TypedArrayData::HeapValue` variants live; `FromSlot for Vec<Arc<String>>` and `Vec<Arc<HeapValue>>` impls landed. JsonValue::Array projection is mechanical given those variants. |
| **2** | `JsonValue::Object(Vec<(String, JsonValue)>)` runtime shape | **STILL OPEN — interior interlock.** Audit 2 of the 2026-05-07 B1 residual session confirmed: current `json_value_to_enum` body uses `ValueWord::from_hashmap_pairs(keys, values)` (deleted) to build the Object payload, and `HeapValue` has no `HashMap` variant. The architectural decision is the **HashMap-marshal micro-cluster** (entry below at the named-in-passing site, expanded 2026-05-07 to a full on-record entry). Three storage-shape options surfaced there. **B1 #2 is unblocked when HashMap-marshal lands**; alternatively, a runtime-shape change (e.g. 2-slot TypedObject `{keys: Array<string>, values: Array<JsonValue>}`) would interlock with shape-vm `__json_object_get`/`as_hashmap` accessors instead. The HashMap-marshal route keeps shape-vm changes minimal; the runtime-shape route avoids waiting on HashMap-marshal but adds cross-crate scope. |
| **3** | Per-variant schemas vs single+discriminant; registration strategy | **EFFECTIVELY SETTLED.** Audit 2 found the per-variant schemas path is already in production: `stdlib-src/core/json_value.shape` declares the `Json` enum with 7 variant schemas, registered at module load via the registry. The "compiler-synthesized at enum-decl time vs stdlib-pre-registered at module init" sub-question's de-facto answer is **stdlib-pre-registered** — the existing code uses it consistently. No architectural decision pending; the single+discriminant alternative remains forbidden by the watchlist (`native_kind.rs:88-96`). |
| **4** | Shape-side enum visibility (prelude-bake vs import) | **RESOLVED** by Cluster #4 Option (commit `ed18cb8`'s audit). `Option`/`Some`/`None` are already prelude-baked (`builtin type Option;` at `intrinsics.shape:31`). Cluster #4 audit established the precedent for sum-types-in-prelude generally; B1 follows. JsonValue is prelude-baked when it lands, no separate decision needed. |
| **5** | Recursive marshal-side projection (stack-depth bound) | **DEMOTED from architectural to runtime concern.** The "recursive vs iterative" question is an implementation choice — current `json_value_to_enum` is recursive; deeply-nested JSON could stack-overflow. This is addressed at landing time with a simple iteration limit or explicit-stack rewrite — no marshal-API change. Not a pre-landing architectural decision. |

**B1 residual cluster identity (post-2026-05-07):** the cluster
collapses to **one open architectural sub-decision (#2)** plus
mechanical body migration of the 5 parser modules. Sub-decision #2
is itself blocked on **HashMap-marshal landing**.

**B1 disposition:** STILL INTERIOR; depends on HashMap-marshal
cluster landing first. After HashMap-marshal lands, B1 becomes
mechanical: the 5 parser modules (json/yaml/toml/msgpack/xml,
~19 errors total) migrate one-file-per-commit using the new
HashMap shape + already-landed Phase 2d Array variants + Cluster #4
Option variants.

**Predicted error-drop after HashMap-marshal landing:**
~-19 (the original 2026-05-06 prediction holds, now with the
specific blocker named).

**Cost saved by audit reduction:** prevents 4 of the 5 prior
"sub-decisions" from being re-debated each session. The log
captures the once-architectural questions as resolved and the one
remaining question as concretely-blocked.

---

## 2026-05-06 — Option<T> / TypedReturn::SomeObjectPairs marshal extension — deferred

On-record deferral. `TypedReturn::Some(payload)` currently takes a
`ConcreteReturn`, but `ConcreteReturn` is intentionally a leaf-only set
per the Concrete/Wrapper split (see 2026-05-06 entry "TypedReturn
recursive variants"). Returning `Some(typed_object)` is therefore
unrepresentable at the marshal boundary.

**Considered:** extend the marshal layer to support `Some(TypedObject)`
returns. Two shapes are viable:

- (α) Add `ConcreteReturn::TypedObject(Vec<(String, ConcreteReturn)>)`
  — recursive payload but bounded by `ConcreteReturn`'s leaf set. Already
  permits `Ok(TypedObject)` / `Err(TypedObject)` for free, since wrapper
  variants take `ConcreteReturn`.
- (β) Add `TypedReturn::SomeObjectPairs(Vec<(String, ConcreteReturn)>)`
  as a flat variant alongside the existing `TypedReturn::Some(ConcreteReturn)`.
  Avoids recursion but requires per-wrapper expansion (`OkObjectPairs`,
  `ErrObjectPairs`).

**Architectural decision needed (next session):** pick (α) vs (β).
(α) keeps the wrapper variants minimal (recursive payload absorbs all
typed-object cases) but breaks the "ConcreteReturn is leaf-only" invariant.
(β) preserves the leaf-only invariant but doubles the wrapper-variant
count for each typed-object case. The choice has implications for the
JsonValue marshal (above) since `JsonValue::Object` is structurally a
typed-object payload.

**Pattern recognized:** This is **not** the runtime-discipline /
optional-defection-becomes-default pattern; both shapes are structurally
typed and statically enforced. The deferral exists because the choice
between (α) and (β) interlocks with the JsonValue runtime
representation — they should be decided together.

**Alternative taken:** defer the marshal extension. `regex.match` /
`regex.find` (Option<Object> return) skip registration for now;
`arrow_module` / `csv_module` typed-row returns wait alongside.

**Consumer count blocked:** ~5 errors visible (regex.match deferred
already in regex.rs; arrow_module 3, csv_module 4). Plus the parser
cluster's `JsonValue::Object` case once `JsonValue` lands.

**Cost saved:** prevented locking in `ConcreteReturn` shape before
JsonValue's runtime representation is decided. Estimate: 1 day of
"redo Concrete/Wrapper split" follow-up avoided.

### 2026-05-07 — Audit-grounded correction: β chosen, surface reduced 5→1, LANDED

Per finding #11's audit-grounded-correction discipline: the original
2026-05-06 framing above was incomplete on three axes. Audits 1+2+3
ahead of execution (per finding #12 binding pre-work) revealed that
the original "5-sub-decision interlock with JsonValue" framing
collapses to a single architectural decision once current code is
read carefully.

**(a) Architectural decision: β chosen.** Three flat per-wrapper
variants landed in commit `ed18cb8`:
- `TypedReturn::SomeObjectPairs(Vec<(String, ConcreteReturn)>)`
- `TypedReturn::OkObjectPairs(Vec<(String, ConcreteReturn)>)`
- `TypedReturn::ErrObjectPairs(Vec<(String, ConcreteReturn)>)`

**Structural reasoning** (binding per supervisor sign-off): β
preserves the leaf-only invariant of `ConcreteReturn` as
**unrepresentably-violated by Rust's type system**. α would have
made leaf-only-violation compile-permitted (a `ConcreteReturn`
variant carrying nested `ConcreteReturn` payloads). β mirrors the
already-landed `ObjectPairs` (`typed_module_exports.rs:117`) and
`ArrayObjectPairs` (line 135) precedent — **pattern continuation,
not pattern invention**. Scope: 3 new variants is bounded; far
smaller than path-2's ~25 HeapKind variants or option δ's parametric
NativeKind explosion. Three additional options checked at sign-off
review and rejected on structural grounds: (δ) single generic
`TypedReturn::WrappedObjectPairs(WrapperKind, …)` — parallel-
discriminator drift; (ε) defer indefinitely — Option-layer rename
of "documented FFI-boundary helper" pattern; (ζ) name-only descriptor
+ separate value lookup — splits value-from-kind, exactly the
structural-purity gain β provides.

**(b) Audit reduction 5 → 1.** The 2026-05-06 framing implied 5
interlocked sub-decisions. Audit 2 (marshal-API surface) verified
4 of them were **already self-resolved** through prior commits:

| # | Sub-decision | Resolution |
|---|---|---|
| **1** | Prelude-vs-import for Option/Some/None | **Already prelude-baked.** `builtin type Option;` at `crates/shape-runtime/stdlib-src/core/intrinsics.shape:31`. `BuiltinFunction::SomeCtor` already registered (`shape-vm/.../helpers.rs:3144`). `ValueWord::none()` exists. No decision pending. |
| **2** | None encoding at slot bits | **Already settled in NativeKind.** Per-numeric `NullableInt8`/`NullableInt16`/`NullableInt32`/`NullableInt64`/`NullableUInt*`/`NullableFloat64`/`NullableIntSize`/`NullableUIntSize` variants exist (`crates/shape-value/src/native_kind.rs:34-77`). Heap types use `NativeKind::Ptr(HeapKind::*)` with bits=0 = None (per `crates/shape-value/src/v2/typed_option.rs`). Watchlist explicitly forbids parametric `NativeKind::Option(T)` at `native_kind.rs:88-96`. |
| **3** | `Some(leaf-payload)` for I64/F64/Bool/String/etc. | **Already works** via existing `TypedReturn::Some(ConcreteReturn::*)`. No decision pending. |
| **4** | `FromSlot for Option<T>` (input side) | **No current consumers** require Option<T> as a stdlib FFI input; optional inputs use `_full + default_snippet` per the marshal-optional-args entry. Future-extensible without architectural decision now. |
| **5** | `Some(TypedObject)` payload | **The actual gap.** Resolved by commit `ed18cb8` (β + 3 variants). |

The 5-sub-decision framing was correct at 2026-05-06's vantage point;
between then and 2026-05-07 the runtime layer (NativeKind nullable
variants, prelude type, builtin constructors) had already advanced
enough that 4 of the 5 sub-decisions self-resolved. Reading current
code first (Audit 2) caught this.

**(c) Consumer count clarification.** The 2026-05-06 entry's "~5
errors visible (regex.match deferred already in regex.rs; arrow_module
3, csv_module 4)" was an over-count by the same shape as cluster #3's
matrix.rs misclassification. The 2026-05-07 audit-grounded correction
on the B1 entry (lines 312-379 above) reclassified `arrow_module`
and `csv_module` typed-row returns to **Phase 2d Array cluster, not
Cluster #4**. Phase 2d Array landed (commits `9fc35ac`/`9f6b1d3`/
`29d61fa`). The actual Cluster #4 consumers were: regex.match and
regex.find — both **commented-out (deferral notes), not error-emitting**.
Per finding #10, the "~5 errors visible" claim was a stale-import-
style miscount; the correct figure is **0 currently-blocked
shape-runtime --lib errors**.

**(d) B1 sub-decision #4 resolved as side-effect.** B1's
"prelude-vs-import for the JsonValue enum" sub-decision (entry above
at line 355) interlocked with Cluster #4's prelude-vs-import question
for sum-types generally. With Option already prelude-baked (per (b)
sub-decision #1), the precedent for sum-types-in-prelude is
established. **B1 sub-decision #4 falls out for free** — JsonValue
follows the Option precedent and is prelude-baked when its runtime
shape lands. B1 residual sub-decisions reduce from 5 to 3
(remaining: #2 Object runtime shape, #3 schema registration strategy,
#5 recursive projection).

**Disposition:** LANDED. Two-commit session at 2026-05-07:

| Commit | Subject | Errors |
|---|---|---|
| `ed18cb8` | TypedReturn::SomeObjectPairs/OkObjectPairs/ErrObjectPairs variants | 89 → 89 (0) |
| `c07d18e` | regex.match + regex.find activation | 89 → 89 (0) |

Predicted 0 ± 3 on commit 1, 0 on commit 2. **Measured 0 on both.**
Predict-vs-measure 2/2 in window. Third leaf-cluster session at the
"small direct drop, multi-cluster unblock value" profile (Phase 2d
Array architectural commit + B1 audit-and-defer + this).

**Multi-cluster unblock value:**
- regex.match / regex.find activated — feature restoration for
  Shape user code.
- B1 sub-decision #4 self-resolved by Option precedent.
- B1 residual reduces 5 → 3 sub-decisions; B1 closer to leaf
  eligibility.
- Future stdlib `Some(TypedObject)` / `Ok(TypedObject)` /
  `Err(TypedObject)` returns can now land mechanically (e.g. table
  lookup returning `Option<{...}>`, structured error responses,
  HashMap-marshal sub-cluster's typed-object surface).

**Watchlist refusals (binding through implementation, none re-litigated):**
- `NativeKind::Option(T)` parametric variant — confirmed forbidden
  at `native_kind.rs:88-96`.
- Sentinel values for None inline in typed bits — None encoding is
  discriminator-level (NullableInt64) or null-pointer (heap), not
  sentinel-bits-in-Some.
- `as_some()` / `as_none()` helpers on HeapValue — same shape as
  cluster #2 α-rejection.
- Recursive `ConcreteReturn::Option(Box<ConcreteReturn>)` — α applied
  to wrapper itself; same leaf-only break.
- "Smaller subset enum of NativeKind for Option-only" — parallel-
  discriminator drift.
- "None is rare, fallback for now" — softening.
- "Rename Option→Maybe to avoid the parametric question" —
  rename-to-less-suspicious-name.

**Watchlist library additions (FYI for future sessions):**
- (δ) Single generic `TypedReturn::WrappedObjectPairs(WrapperKind, …)`
  — parallel-discriminator drift; refused on sight.
- (ε) Indefinite deferral of architecturally-bounded extensions —
  Option-layer rename of "documented FFI-boundary helper"; cluster
  never closes; refused on sight.
- (ζ) Splitting value-from-kind via name-only descriptor + separate
  lookup — exactly the structural-purity gain β provides; net
  negative; refused on sight.

**Cost saved:** prevented α's leaf-only-invariant break (would have
required either reverting later or accepting a permanently-weaker
ConcreteReturn invariant). Estimated 1-2 weeks of "ConcreteReturn
recursive payloads breeding W-series-shape consumer rationalization"
avoided over the next year.

---

## 2026-05-06 — marshal-optional-args extension — register_typed_fn_N_full

This is **not** a defection (no strict-typing compromise; no dispatch
shape preserved). It is an **on-record marshal-API extension**:
extends `register_typed_fn_N` with `_full` variants taking
`ModuleParam` directly so per-param `required: bool` +
`default_snippet: Option<String>` can flow through to the schema
introspection layer and the compiler-side default-arg insertion
path. Bodies stay typed — the compiler ensures all N typed args
are present at call time before the marshal layer sees the call.

**Discovered:** during cluster #2 (IoHandle marshal) execution. The
existing `register_typed_fn_N` family in `crates/shape-runtime/src/marshal.rs:281-365`
hardcodes `required: true` on every `ModuleParam` it constructs:

```rust
.map(|(name, ty)| crate::module_exports::ModuleParam {
    name: (*name).to_string(),
    type_name: (*ty).to_string(),
    required: true,                     // hardcoded
    ..Default::default()                 // default_snippet: None
})
```

Migrating any stdlib function with optional trailing args to
`register_typed_fn_N` produces a Shape-level signature regression —
e.g., `io.open("/path")` (canonical I/O, mode default `"r"`) becomes
a compile error because the migrated registration declares mode
as required without a default.

**Audit (across all current and pending stdlib clusters):** 17
trailing-optional-arg sites identified workspace-wide:

- stdlib_io (12): `io.open(mode?)`, `io.read(n?)`, `io.read_bytes(n?)`,
  `io.mkdir(recursive?)`, 4× network/UDP `n?` (max bytes/buffer),
  2× process spawn `args?` (Vec<string>), pipe-ops `handle?` (default stdin),
  gzip `level?`.
- stdlib/http.rs (2): optional `object` + `any` typed headers/body.
- stdlib/json.rs (1): optional `bool` (likely pretty-print).
- stdlib/csv_module.rs (2): optional `string` + `Array<string>` typed.

All trailing-position. No mid-position optionals. No optional-of-optional.
No varargs (the Vec<string> args param is a single optional-typed-array,
not varargs).

**Considered (option 1 — register_typed_fn_N_full):**

```rust
pub fn register_typed_fn_2_full<F, P0, P1>(
    module: &mut ModuleExports,
    name: impl Into<String>,
    description: impl Into<String>,
    params: [ModuleParam; 2],
    return_type: ConcreteType,
    body: F,
) where ...
```

Body remains typed (`Fn(P0, P1, &ModuleContext) -> Result<TypedReturn, String>`);
the dispatcher reads `arg_kinds` from FromSlot impls; the
ModuleExports schema records each `ModuleParam` directly, including
its `required` and `default_snippet`. The compiler-side default-arg
insertion path (`crates/shape-vm/src/compiler/functions_foreign.rs:433`,
`crates/shape-vm/src/compiler/statements.rs:540`) reads
`default_value` and synthesizes the missing call-site arg before
the marshal layer sees the call. So the body always receives N
typed args; "optional" is purely a compile-time/schema concern.

**Considered (option 2 — sentinel values inline, REJECTED):**

Migrate optional args as required. Encode "absent" via a sentinel
value (e.g. `-1` for "read all", `""` for "default mode"). Body
checks the sentinel and applies the default at runtime.

**Pattern recognized (option 2 rejection):** This is a textbook
W-series shape applied at the marshal-API level. The "optional vs
present" distinction is dynamic state stored in typed bits; the
body decodes the sentinel at runtime to recover the intent. Same
shape as `Convert<X>To<Y>` opcodes papering over kind-tracker gaps,
just one layer higher. The Shape-level signature LIES about
required-ness (compiler-side reads `required: true`) while runtime
behavior reconstructs the optional via convention. Forbidden:
identical pattern to "rename to a less suspicious name" from the
CLAUDE.md forbidden list, applied to the marshal-API surface.

**Considered (option 3 — defer with user-facing regression, REJECTED):**

Migrate stdlib_io functions to required-only. Accept that
`io.open("/path")` becomes a compile error (mode now required).
Add a follow-up workstream `marshal-optional-args` to address.

**Pattern recognized (option 3 rejection):** `io.open(path, mode?)`
is canonical Shape I/O; its signature is part of Shape's public
API. Breaking it isn't a deferred residual, it's a behavior change
visible to every Shape user. Same precedent as the simulation
deletion: the simulation engine had no live consumer (acceptable
deletion); `io.open`'s optional `mode` has every Shape user as
its consumer (unacceptable regression). Different size of consumer
surface, different disposition.

**Pattern recognized (general):** marshal-API extensions are real
architectural work, not "small plumbing." This is the **seventh
sub-cluster discovery in cluster surface analysis** during this
work — joining: directory-adjacency cluster fallacy (×6 instances),
the cluster #1 sibling re-trace, cluster #3 matrix.rs
mis-classification, and now this. Pattern is structural: the
strict-typing migration interacts with the codebase such that every
cluster surface analysis surfaces a new architectural prerequisite.

**Adopted standard pre-work for any cluster scope estimate:**

- **Audit 1 (consumer-call-shape):** trace each consumer's actual
  call shape, dispatch path, calling convention. Catches
  directory-adjacency miscounts.
- **Audit 2 (marshal-API):** verify marshal layer supports what
  consumer bodies need. Catches missing infrastructure for
  optional args, generic returns, async dispatch, etc.

Predict scope only AFTER both audits. Predictions before audits
have been wrong 7 of 7 times.

**Alternative taken:** option 1 — extend `register_typed_fn_N`
family with `_full` variants taking `[ModuleParam; N]` directly,
allowing `required: false` + `default_snippet: Some("…")` to flow
through to schema introspection and compiler-side default-arg
insertion. Sync arity 0/1/2/3 + async arity 1/2/3 = 7 new variants,
~30 LoC each.

**Adjacent sub-cluster surfaced (additional finding, not blocking):**
some optional args have types with NO FromSlot impl yet —
`Vec<Arc<String>>` for process spawn `args?`, `object`/`any` for
http headers/body, `Array<string>` for csv. These are separate
FromSlot extensions (their own sub-clusters), NOT blocked by the
optional-args extension itself. The optional-args extension
unblocks the ~10-12 cases with known FromSlot types
(int/bool/string/IoHandle); the rest wait on their own FromSlot
impls. Logged here for traceability; no separate entry needed since
each `Vec<T>`-typed FromSlot follows the cluster #3 option β
precedent and the `Arc<HeapValue::*>`-typed FromSlot follows the
cluster #2 option γ precedent.

**Cost saved:** prevented the W-series defection at the marshal-API
level (option 2's sentinel-value pattern). Prevented user-facing
Shape signature regression on canonical I/O (`io.open`/`io.read`/etc).
Acknowledged: ~30 LoC × 7 arities = ~210 LoC of additive marshal-API
extension. Bounded scope; doesn't touch existing migrated callers
(file.rs, regex.rs, crypto.rs, env.rs, unicode.rs, compress.rs,
archive.rs all use the non-`_full` variants and stay unchanged).

### 2026-05-07 — first-position-optional sub-cluster (deferred)

Audit-grounded sub-entry under the marshal-optional-args extension
above. The above entry's audit identified "pipe-ops `handle?` (default
stdin)" among the 17 trailing-optional-arg sites. Phase 2d's
`process_ops` migration revealed that this case is structurally
different from the other optionals and warrants its own sub-cluster
identity.

**Shape difference.** Trailing-optional with N≥1 required params
(e.g. `io.open(path, mode?)`, `io.read(handle, n?)`) is fully
addressed by the `register_typed_fn_N_full` family — the body
always receives N typed args; the missing call-site arg is
synthesized from `default_snippet` by the compile-time default-arg
insertion path. Body sees no "optional" semantics.

**First-position-optional** is the case where the optional param
is the **only** param. Two distinct user-facing surfaces:

1. **`fn()` (arity 0)** — no args, body uses an internal default.
2. **`fn(x)` (arity 1)** — one arg, body uses provided value.

These two surfaces share a function name. The current
`register_typed_fn_N` family is **fixed-arity**: a name registers
to exactly one (arity, arg-kinds) tuple. Supporting both arity-0
and arity-1 forms under the same name requires arity-disambiguation
at the registration layer, which the marshal-API surface does not
currently expose.

**Audit 1 (consumer-call-shape) finding:** `io.read_line` is the
**unique consumer** of this shape in the current stdlib. The 0-arg
fns currently registered (`stdin`, `stdout`, `stderr`,
`ed25519_generate_keypair`, `cwd`, `os`, `arch`) have no 1-arg
overload partner. The pending modules (json/http/yaml/toml/msgpack/
xml) have only trailing-position optionals with required first
params (already handled by `_full` variants). No other current or
near-term consumer.

**Considered (option α — arity-1 with `default_snippet: Some("io.stdin()")`):**
register `io.read_line(handle: IoHandle)` as required-1-arg with
the default snippet being a function call. The compile-time
default-arg insertion path expands the call site `read_line()`
into `read_line(io.stdin())` before marshal sees it. **Open
question:** does `default_snippet` parse arbitrary expressions, or
literals only? If arbitrary expressions: this works with no
marshal-API extension. If literals only: option α requires
extending the default-arg insertion path to handle expression
defaults. Audit 2 (default-snippet expression-vs-literal capability
in the compile-time insertion path) is the gating sub-question for
this shape.

**Considered (option β — arity overloading at the registration
layer):** allow `register_typed_fn_0` *and* `register_typed_fn_1`
to be called with the same name; dispatcher disambiguates by
call-site argument count. Adds a runtime field
(`overload_by_arity: HashMap<usize, Box<TypedInvoke>>`) to
`ModuleFnEntry`. Cleaner if multiple consumers eventually surface;
heavier than necessary for a single consumer.

**Considered (option γ — split into two distinct names):** rename
the arity-1 form to `read_line_from(handle)` and keep `read_line()`
arity-0. User-facing surface change to canonical Shape I/O
(`io.read_line(handle)` is documented as the file/pipe form).
Same precedent as the `marshal-optional-args` entry's option 3
rejection — `io.read_line(handle?)` is part of Shape's public API,
breaking it isn't a deferred residual but a behavior change.

**Watchlist refusals (binding when this lands):**
- Refuse "register two names internally and alias" (rename-to-
  less-suspicious-name shape).
- Refuse "body checks `args.is_empty()` and dispatches" — this is
  the dynamic-fallback shape applied at the marshal-API entry
  point. Forbidden per CLAUDE.md.
- Refuse "make `Arc<IoHandleData>` an `Option<Arc<IoHandleData>>`
  parameter" — interlocks with Cluster #4 Option marshal which is
  itself a deferred decision; would push the first-position-
  optional question into the Option<T> cluster's sub-decisions.
- Refuse "default_snippet parses Shape source at registration time
  and bakes the result into the schema" — moves arbitrary-
  expression evaluation into stdlib registration (sandbox/
  permission boundary violation potential).

**Disposition:** deferred to its own surface-and-decide round-trip.
Currently held as a source-comment TODO in
`crates/shape-runtime/src/stdlib_io/process_ops.rs::read_line`
that says "Callers should use `io.stdin()` then `read_line(handle)`
instead." This is a **behavior workaround**, not a permanent
solution — Shape's documented `io.read_line()` 0-arg form is
broken until this sub-cluster lands.

**Predicted error-drop:** 0 (no current shape-runtime --lib
errors; this is a feature-completeness gap, not a compile-time
break). Surfaces in shape-test as a `read_line()`-no-arg test
failure when the test harness comes back online.

**Cost saved by sub-entry:** prevents the source-comment-TODO
accumulation pattern. Future sessions don't need to re-derive
"why is `read_line()` no-arg deferred" — the architectural shape,
options, and watchlist refusals are on-record.

---

## 2026-05-06 — IoHandle marshal extension — deferred (stdlib_io cluster blocker)

On-record deferral. The `stdlib_io` module (file_ops, network_ops,
path_ops, process_ops, async_file_ops + `mod.rs`) routes 48 functions
through `wrap_legacy()` / `wrap_legacy_async()` adapters that take
`&[ValueWord]` and produce `TypedReturn::ValueWord` (deleted variant).
Migration to typed marshal requires `FromSlot` for `IoHandle`-typed
arguments and `ToSlot` for `IoHandle`-typed returns.

**`HeapKind::IoHandle`** is in the trimmed HeapKind enum (post-2026-05-06
trim, kind index 13). The Phase 2b foundation entry's "DON'T surface"
list explicitly permits "Adding FromSlot/ToSlot impls for types already
in HeapKind." So the surface change is in scope; the architectural
decision is the **unwrap policy**.

**Considered (option α — Arc<HeapValue> + body unwraps):**
```rust
impl FromSlot for Arc<HeapValue> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(HeapKind::IoHandle);
    fn from_slot(bits: u64) -> Self { /* recover Arc<HeapValue> */ }
}
```
Body declares `arg: Arc<HeapValue>` and calls `match arg.as_ref() {
HeapValue::IoHandle(h) => …, _ => unreachable!() }` (the heap-kind
discriminator-already-checked by the dispatcher's NATIVE_KIND
contract).

**Pattern recognized (option α):** the body's `match` arm is naturally
exhaustive given the trimmed HeapKind enum (per the unified Phase 2b
"unreachable arms in match kind blocks" watchlist). But every IoHandle
body re-writes the same single-arm match. Boilerplate, not unsafe.

**Considered (option β — `&IoHandleData` borrowed):**
```rust
impl FromSlot for &IoHandleData {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(HeapKind::IoHandle);
    fn from_slot(bits: u64) -> Self { /* recover borrow */ }
}
```
Body declares `arg: &IoHandleData` and gets the typed payload directly.
Saves the boilerplate match.

**Pattern recognized (option β):** the borrow lifetime needs to outlive
the slot read but not the slot itself; lifetime annotation across the
`FromSlot` trait boundary is non-trivial. Probably needs a custom `FromSlotRef<'a>`
trait variant. Real architectural call.

**Considered (option γ — Arc<IoHandleData> separate-Arc):**
extract the `IoHandleData` payload into its own Arc-counted heap allocation,
parallel to `Arc<DataTable>`. Each IoHandle slot stores both a
`HeapValue::IoHandle(...)` outer Arc AND an inner `Arc<IoHandleData>`
for direct typed access. Mirrors the DataTable pattern.

**Pattern recognized (option γ):** double-Arc allocation per handle is
a real perf regression for I/O-heavy workloads. Probably the right
answer is to make `HeapValue::IoHandle` payload directly `Arc<IoHandleData>`
(single Arc, accessed via `&Arc::clone(&payload)` from the heap value).
That's a `HeapValue` shape change, not just a marshal extension.

**Architectural decision (this session):** pick **option γ —
Arc<IoHandleData> separate-Arc, with `HeapValue::IoHandle` payload
changed from `Box<IoHandleData>` to `Arc<IoHandleData>` (single Arc,
not double-Arc as the original entry framed it).** The "double-Arc
perf regression" framing in the original entry was wrong — it
counted `Box<IoHandleData>` clone cost (alloc + memcpy + atomic
inside the inner Arc<Mutex<...>>) the same as `Arc<IoHandleData>`
clone (one atomic). Switching the variant payload from Box to Arc
is actually a small **perf improvement** for HeapValue cloning,
not a regression.

**Why option γ over α and β:**

- **α (Arc<HeapValue> + body unwrap)** would mirror cluster #3
  option β's pattern-match-on-mismatch shape, but every body declares
  `arc: Arc<HeapValue>` and unwraps via match. The body's parameter
  type is opaque — readers see "this body takes a heap pointer,
  then reaches into HeapValue::IoHandle" rather than "this body
  takes an IoHandle." The α-shape "as_io_handle() helper on
  HeapValue" rationalization that compresses the unwrap boilerplate
  is rejected: it would hide the typed correspondence between the
  body's parameter type and the actual payload. Same pattern as
  cluster #3's rejection of "preserve dead infrastructure under
  typed shape" — different layer, same defection-attractor risk.
- **β (FromSlotRef<'a>)** is a real architectural addition — a
  parallel trait family (`FromSlotRef<'a>`, `register_typed_fn_N_ref`
  parallel helpers) that will proliferate (DataTable wants this
  eventually, then Content, then everything). The lifetime
  plumbing is non-trivial and the trait-family-grows risk is real.
  Avoid.
- **γ (Arc<IoHandleData>)** mirrors the existing `Arc<DataTable>`
  precedent at `marshal.rs:193` exactly. Single source of truth
  at the FromSlot pattern level. Self-documenting body
  (`fn open(handle: Arc<IoHandleData>)` states what the body
  needs). Same consistency-check residual as α and β at the
  `match HeapValue::IoHandle(...) => …, _ => panic!()` layer.

The HeapValue payload shape change (`Box<IoHandleData>` →
`Arc<IoHandleData>`) is a small structural edit at
`heap_variants.rs` macro + `heap_value.rs` Clone impl. Consumer-side
breakage is bounded to pattern-match sites that destructure
`HeapValue::IoHandle(box_or_arc)` — auditable in one grep.

**Alternative taken:** option γ. `Arc<IoHandleData>` FromSlot/ToSlot
impls in `marshal.rs` mirror the `Arc<DataTable>` shape exactly.
Bodies declare `Arc<IoHandleData>` and call methods on it directly
via `Arc::deref` (e.g. `handle.is_open()`, `handle.close()`,
`handle.resource.lock()`).

**Consumer count blocked (post-2026-05-06 cluster identity re-trace,
sixth instance of the directory-adjacency cluster fallacy):** the
original "48 stdlib_io functions" framing conflated cluster #2
(IoHandle marshal) with the path-only mass migration that lives
in the same `stdlib_io/` directory. Tracing each function via grep
`as_io_handle / from_io_handle / IoHandleData`:

| File | Total `pub fn` | IoHandle-touching | Cluster identity |
|------|----------------|-------------------|------------------|
| `file_ops.rs` | 17 | ~8 (open, read, read_to_string, read_bytes, write, close, flush, read_gzip, write_gzip) | cluster #2 |
| `network_ops.rs` | 9 | ~9 (TCP/UDP/handle ops) | cluster #2 |
| `process_ops.rs` | 12 | ~10 (spawn/kill/wait/etc) | cluster #2 |
| `path_ops.rs` | 5 | **0** | NOT cluster #2 — stdlib_io path-mass cluster |
| `async_file_ops.rs` | 5 | **0** | NOT cluster #2 — stdlib_io path-mass cluster (async variant) |
| `mod.rs` | (registration) | (handle wiring) | cluster #2 |

**Real cluster #2 surface: ~27-30 IoHandle-touching functions**, not 48.
The other ~18 stdlib_io functions are path-only (`Arc<String>`
input, no IoHandle) and migrate mechanically with `register_typed_fn_N`
or `register_typed_async_fn_N` — they're a separate cluster
("stdlib_io path-mass") that's already fully unblocked, just sitting
alongside the IoHandle-blocked work in the same crate directory.

**Watchlist crystallization (sixth instance, now confirmed
baseline):** "directory-adjacency cluster fallacy." When a deferral
entry frames a cluster by directory or file name (e.g. "the
stdlib_io cluster", "the parser cluster"), verify each file's
actual call shape (which calling convention, which dispatch path,
which marshal extension it depends on) before assigning to the
cluster. **File / directory adjacency is not cluster identity.**
Six prior instances in this work, all systematic miscounts. Adopted
as a binding pre-cluster-execution check.

**Cost saved:** prevented committing to option α's "as_io_handle()
helper on HeapValue" shape (which would have hidden the typed
correspondence between body parameter and payload — same
defection-attractor pattern as the cluster #3 dead-infrastructure
rejection). Prevented option β's FromSlotRef<'a> trait-family
expansion that would have proliferated across DataTable / Content /
Arc<String> consumers.

---

## 2026-05-06 — Array<T> marshal extension — deferred (byte/intrinsics cluster blocker)

On-record deferral. `Array<int>` / `Array<number>` arguments and returns
are blocked on the choice of FromSlot/ToSlot signature for typed
heap-array slots.

**`HeapKind::TypedArray`** is in the trimmed HeapKind enum (kind 8).
`HeapValue::TypedArray(TypedArrayData::*)` carries `Arc<TypedBuffer<i64>>`
/ `Arc<AlignedTypedBuffer>` / `Arc<TypedBuffer<u8>>` / etc. The marshal
extension permission is on the "DON'T surface" list — the architectural
decision is the canonical Rust input/output type.

**Considered (option α — Arc<TypedBuffer<T>>):**
```rust
impl FromSlot for Arc<TypedBuffer<i64>> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(HeapKind::TypedArray);
    fn from_slot(bits: u64) -> Self { /* recover from HeapValue::TypedArray::I64 */ }
}
impl ToSlot for Arc<TypedBuffer<i64>> { … }
```
Body declares `arr: Arc<TypedBuffer<i64>>` and reads `&arr[..]`. Zero-copy.

**Pattern recognized (option α):** but `HeapKind::TypedArray` covers
ALL element widths (`I64` / `F64` / `Bool` / `I8` / etc.) — the
NATIVE_KIND alone doesn't pin the element width. The dispatcher
needs additional element-width metadata. Either thread it through
the slot (which is back to runtime tag-decode at the boundary) or
parametrize NativeKind by element width (parametric NativeKind
defection, rejected).

**Considered (option β — Vec<T> by-value owned):**
```rust
impl FromSlot for Vec<i64> {
    const NATIVE_KIND: NativeKind = NativeKind::Ptr(HeapKind::TypedArray);
    fn from_slot(bits: u64) -> Self { /* clone HeapValue::TypedArray::I64 into Vec */ }
}
```
Per-call clone. Simpler but loses the zero-copy benefit.

**Considered (option γ — `&[T]` borrowed):** same lifetime concern as
IoHandle option β; needs `FromSlotRef<'a>` trait variant.

**Considered (option δ — element-width parametric `NativeKind`):**
`NativeKind::TypedArrayI64`, `NativeKind::TypedArrayF64`, etc. as
distinct discriminants. Avoids the "TypedArray's element width is
not in the NativeKind" gap.

**Pattern recognized (option δ):** This **is** the parametric-NativeKind
pattern explicitly rejected on the watchlist (HeapKind trim + NativeKind::Ptr
entry). Re-creates "heterogeneous-by-default" at the discriminator level.
**Forbidden.**

**Considered (option ε — NativeKind unparametric, element width via Rust type):**
two parallel `FromSlot` impls per element type, both with
`NATIVE_KIND = NativeKind::Ptr(HeapKind::TypedArray)`. The body's
declared parameter type (`Vec<u8>` vs `Vec<i64>` vs `Arc<TypedBuffer<f64>>`)
selects which impl is monomorphized; the impl pattern-matches
`HeapValue::TypedArray(TypedArrayData::I64 | F64 | …)` and panics
on mismatch. Precedent: the existing `Arc<DataTable>` `FromSlot`
impl in `marshal.rs:193` uses the same shape.

**Pattern recognized (option ε):** the marshal-boundary panic is
**spec-permitted consistency check, not runtime probe**. Per
`docs/runtime-v2-spec.md`: "the kind tells you the arm; HeapValue
dispatch is a consistency check, not a probe." The dispatcher
decision was made at registration via the FromSlot impl's declared
return type; the panic is `debug_assert!`-equivalent, unreachable
in a well-typed system. Distinct from the rejected option δ —
NativeKind itself stays unparametric, so the discriminator level
carries no element-width information; element-width is a body-side
type contract enforced by the Rust type system.

**Considered (path 2 — per-element HeapKind split, on-record rejected):**
split `HeapValue::TypedArray(TypedArrayData)` into per-element
top-level variants — `HeapValue::TypedArrayI64(Arc<TypedBuffer<i64>>)`,
`HeapValue::TypedArrayF64(...)`, etc., with `NativeKind::Ptr(HeapKind::TypedArrayI64)`,
`NativeKind::Ptr(HeapKind::TypedArrayF64)`, etc. as fully-discriminative
discriminators. Same restructuring would apply to `TemporalData`,
`TableViewData`, etc.

**Rationalization (path 2):** "structural-enforcement-pure is worth
the scope. NativeKind alone fully discriminates the element type,
no runtime consistency check at the marshal boundary, the v2 spec's
unparametric-NativeKind constraint stays satisfied without any
panic-on-mismatch arms anywhere in the codebase."

**Pattern recognized (path 2):** scope ~25+ new HeapKind/HeapValue
variants. Same magnitude of cross-cutting refactor as the heap_value
reconstruction (2026-05-06 entry). On-record rejected on scope
grounds, not soundness — path 2 is structurally cleaner than
option ε, but the consistency-check residual is spec-permitted
and the perf cost is at the FFI boundary, not the hot path. See
the separate maximalist-v2-redesign deferral entry below for the
broader architectural cut path 2 anticipates.

**Architectural decision (this session):** pick **option β** —
`FromSlot for Vec<u8>` / `Vec<i64>` etc. with `NATIVE_KIND =
NativeKind::Ptr(HeapKind::TypedArray)`, owned-clone semantics, no
zero-copy `Arc<TypedBuffer<T>>` impls until a perf-sensitive
consumer drives them. Implements the byte-iterator pattern that
all current consumers use; defers the `Arc<TypedBuffer<T>>`
zero-copy variants to a future surface-and-decide round-trip
with concrete consumer profiles in hand.

**Why option β over ε:** YAGNI — no current Array<T> marshal
consumer needs zero-copy. Adding `Arc<TypedBuffer<T>>` impls
speculatively recreates the dead-infrastructure-attractor pattern
(simulation/engine.rs precedent: domain feature with no live
consumer becomes attractor for new code routed through inadequate
shape). Option β is forward-compatible: when a perf consumer
arrives, the additional `Arc<TypedBuffer<T>>` impls land as their
own round-trip with the consumer driving design choices.

**Alternative taken:** option β. The bodies use
`Vec<u8>::from_slot(bits)` to obtain owned-clone byte arrays,
matching the existing byte-iterator code paths in compress /
archive / byte_utils. Returns wrap `Vec<T>` back into
`HeapValue::TypedArray(TypedArrayData::*(Arc::new(TypedBuffer::from_vec(v))))`.

**Consumer count blocked (post-2026-05-06 cluster identity re-trace):**
~7 errors visible in the actual Array<T> marshal cluster:
`compress.rs` (2), `archive.rs` (2), `byte_utils.rs` (3). Plus
`register_typed_function` → `register_typed_fn_N` rename in
compress/archive (mechanical, not Array<T>-specific).

**Files mis-attributed in the original entry, now reclassified:**
- `intrinsics/matrix.rs` (5 errors) — uses `IntrinsicFn = fn(&[ValueWord], &mut ExecutionContext) -> Result<ValueWord>` calling convention from `intrinsics/mod.rs:34`, NOT `register_typed_fn_N`. **Belongs to the intrinsics-dispatch-table cluster** (named in the cluster #1 sibling-list correction above), not Array<T>. The original entry's "perf-sensitive matrix.rs drives FromSlot signature choice" framing was wrong — matrix.rs doesn't use the marshal layer at all.
- `stdlib_io/file_ops.rs` read_bytes/write_bytes (2-3 errors) — Array<T> ∩ **cluster #2 (IoHandle)**. Migrates as part of the IoHandle cluster, not Array<T>.

**Calibration finding (this session):** the original "Consumer count
blocked: ~10-15 errors" claim was both an over-count AND a
mis-classification. The correct count is ~7, with 5 errors
(matrix.rs) belonging to a different cluster entirely. Pattern:
**cluster decomposition produced by tracing imports + "obvious cluster"
intuition is unreliable.** Verifiable principle: trace each
consumer's actual call shape (which calling convention, which
dispatch path) before assigning to a cluster. The "perf-sensitive
consumer drives signature" framing in particular missed that
matrix.rs doesn't even use the marshal layer.

**Cost saved:** prevented committing to zero-copy `Arc<TypedBuffer<T>>`
semantics with no perf consumer driving the choice. Estimate: 1-2
days of "audit which calling convention the consumer actually uses"
follow-up avoided. Acknowledged: ~7 errors land alongside the
option β implementation in this session.

---

## 2026-05-06 — maximalist v2 redesign — dissolve HeapValue sum type at discriminator level (DEFERRED)

This is **not** a defection (no strict-typing compromise; no dispatch
shape preserved). It is an **on-record DEFERRED follow-up workstream**:
the redesign IS the right long-term answer per the v2 spec endpoint;
it's just not the right short-term answer for the current
shape-runtime compile-completion phase. Logged here so future
sessions see the architectural target rather than treating the
current consistency-check residual as the terminal shape.

**Considered:** leave `HeapValue` as the sum type at slot-bits level
forever; accept marshal-boundary runtime consistency checks (the
`match HeapValue::* => …, _ => panic!()` arms inside `FromSlot::from_slot`
impls) as permanent residual. Each slot's bits remain
`Arc::into_raw(Arc<HeapValue>)`; the dispatcher decodes by reading
`HeapValue` and pattern-matching the expected arm.

**Rationalization:** "the consistency checks are spec-permitted
(`runtime-v2-spec.md`: 'consistency check, not probe'), the marshal
layer is the FFI boundary not the hot path, opcode dispatch /
JIT hot loops already have zero runtime kind checks. The check
residual is debug-assert-equivalent in a well-typed system."

**Pattern recognized:** the rationalization is correct as far as
it goes — marshal-boundary checks are spec-permitted and not on
the hot path. But the rationalization understates two future risks:

1. **Non-zero runtime overhead at the marshal boundary itself.**
   Every typed FFI call materializes `Arc<HeapValue>` and walks
   the enum discriminant to verify the arm matches. The walk is
   one branch + one pattern match per FromSlot, but it scales
   linearly with FFI call density. At ~10⁶ stdlib calls/sec in
   I/O-heavy programs the cost is ~3-5% of stdlib runtime —
   measurable but not catastrophic. Hot-path opcode dispatch
   stays zero-check, but stdlib-call-heavy programs (file I/O
   loops, byte-array batches, event-driven analytics) sit at
   the marshal boundary continuously.

2. **Discriminator-level dispatch sites multiply across phases.**
   Snapshot, wire, debug-print, leak-check, and any future
   tooling that walks heap values goes through the same
   `match HeapValue::*` shape. Each site is a candidate for
   the W-series defection attractor: an unspecified author
   adds a "just one more arm" handler that quietly becomes a
   runtime probe instead of a consistency check.

**Maximalist alternative (the redesign):** each typed heap thing
(`TypedArray<u8>`, `TypedArray<f64>`, `DataTable`, `Instant`,
`IoHandle`, etc.) gets its own heap allocation; slot bits become
typed pointers (`Arc::into_raw(Arc<TypedBuffer<u8>>)` directly,
not `Arc::into_raw(Arc<HeapValue>)`); `NativeKind` explodes from
~25 to ~50+ variants — `NativeKind::Ptr(HeapKind::TypedArrayU8)`,
`NativeKind::Ptr(HeapKind::TypedArrayF64)`, etc. as fully-discriminative
discriminators. `HeapValue` as a sum type **dissolves** at the
slot/wire/snapshot/debug levels — it survives only as a
constructor-parameter type for legacy code paths during the
transition. Snapshot/wire/debug paths dispatch directly on
`NativeKind` without `HeapValue` lookup; refcount/drop dispatches
on the typed pointer's `Drop` impl directly.

**Cost of the redesign:** 2-4 week cross-cutting refactor. Touches
every heap-allocation site, refcounter, drop, snapshot/wire path,
plus the entire NativeKind enum + dispatcher table. The path 2
discussion in the cluster #3 deferral entry above is a tractable
subset (TypedArray-only); the full redesign generalizes it across
all currently-`HeapValue::*` shapes.

**Alternative taken (current):** option β with consistency-check
residual. Acceptable because (a) marshal is the FFI boundary, not
the hot path; (b) hot-path opcode dispatch already has zero runtime
kind checks per the v2 spec design; (c) the redesign is out of scope
for the shape-runtime compile-completion phase. Logged as deferred
follow-up rather than rejected because the redesign IS structurally
the correct long-term answer.

**Trigger conditions for revisit (any one is sufficient):**

(a) **Profiling shows marshal-boundary checks >1% runtime in real
    programs.** Specifically: `perf` / `dtrace` / similar shows the
    `from_slot` pattern-match arms hot enough to register on
    sampling profilers under realistic stdlib-heavy workloads.
(b) **Path 2 (per-element TypedArray HeapKind split) becomes
    competitive on scope grounds.** Specifically: the typed-array
    zoo grows past N variants (~15+) where the consistency-check
    code is more boilerplate than the per-element-HeapKind split
    would be.
(c) **v2 spec evolves to require typed pointers at slot level.**
    Specifically: `docs/runtime-v2-spec.md` is updated to forbid
    the `Arc<HeapValue>` indirection at the slot bits, requiring
    direct typed pointers.
(d) **HeapValue accumulates additional dispatch sites that recreate
    W-series-shape risks.** Specifically: a future session is found
    rationalizing a "just one more arm in the marshal-boundary
    pattern match" change, and the defection-pattern review identifies
    the underlying risk as the dispatcher's sum-type shape rather
    than the individual change.

**Adjacent already-on-record entries:** path 2 in the cluster #3
deferral above is the same architectural direction restricted to
the TypedArray subtree. The HeapKind trim (2026-05-06 entry) and
NativeKind::Ptr(HeapKind) extension are intermediate steps in the
same direction. The maximalist redesign generalizes those across
all heap-allocated typed shapes.

**Cost saved:** 2-4 weeks of cross-cutting refactor deferred from
the current phase, where the consistency-check residual is
spec-permitted and not load-bearing. Acknowledged: each FromSlot
impl in the marshal layer carries one arm of `match HeapValue::* =>
…, _ => panic!()` boilerplate as long-term residual, with the
documented spec-citation justifying it as consistency check rather
than runtime probe.

---

## 2026-05-06 — move-semantics-marshal — leverage existing LoadLocalMove/LoadLocalClone bytecode opcodes at the stdlib FFI boundary (DEFERRED)

This is **not** a defection (no strict-typing compromise; no dispatch
shape preserved). It is an **on-record DEFERRED follow-up workstream**:
the bytecode already encodes per-local move-vs-clone ownership; the
marshal layer's always-clone shape is consistent with current FromSlot
abstraction, but it doesn't propagate the existing bytecode-level
ownership signal across the FFI boundary. Logged so future sessions
see the architectural target rather than treating option β's
always-clone as the terminal shape.

**Bytecode-side state (audited 2026-05-06):**

- **Wired (older, in production):** `LoadLocalMove` (0x104) transfers
  ownership and zeros the source slot; `LoadLocalClone` (0x105)
  clones, source stays live; `StoreLocalDrop` (0x106) drops old
  before storing; `DropSharedLocal` (0x139) releases shared locals.
  These opcodes ARE emitted by the compiler today and respected by
  the VM hot path.
- **Unwired (V1.1A planned):** `MoveLocal` (0x125), `CloneLocal`
  (0x126), `DropLocal` (0x127). Source comment: "UNWIRED — V1.1B
  adds handlers." Future generation, not load-bearing yet.

So Shape's compiler IS tracking ownership at the local variable
level and emitting move-vs-clone opcodes for local loads/stores.
Let-Rust-lifetime semantics are real at the bytecode level. The
marshal layer just doesn't leverage them at the stdlib FFI boundary.

**Considered:** extend the marshal layer to a `MoveFromSlot` /
`CloneFromSlot` distinction, parallel to the existing `FromSlot`.
The calling convention communicates per-arg ownership (move vs
clone) from the bytecode-emit-side to the dispatcher: each
`register_typed_fn_N` declares per-param whether the body takes
ownership or borrows. The dispatcher selects the appropriate slot
read at call time. Body-side: a body that wants to consume its
input declares `Vec<u8>` via `MoveFromSlot`; one that just iterates
declares `&[u8]` via `CloneFromSlot` (or a borrowed-slot equivalent).

**Pattern recognized:** the bytecode already has the move/clone
distinction; option β's always-clone marshal is "language has it,
runtime FFI boundary doesn't propagate it." Not a defection — both
shapes are statically typed and dispatch-safe — but a missed
optimization opportunity at the FFI boundary specifically. The
inner-loop hot paths (opcode dispatch, JIT hot loops) already
respect move-vs-clone via the wired `LoadLocalMove` / `LoadLocalClone`
opcodes, so the runtime as a whole already has the distinction;
it just stops at the marshal layer.

**Caveat (audit-discovered):** leveraging the bytecode-level
distinction at the marshal layer is **not** purely small plumbing.
The calling convention needs to communicate per-arg ownership
declaration from emit-side to dispatcher; FromSlot splits into
`MoveFromSlot` / `CloneFromSlot` (or grows an associated
`OWNERSHIP` const); every stdlib registration site declares
ownership per param. Real architectural piece — estimated 2-5 days
post-design once the trait shape is settled.

**Alternative taken (current):** option β always-clone marshal
(see cluster #3 entry above). Acceptable because (a) marshal is
the FFI boundary, not the hot path; (b) the inner-loop perf paths
already respect move-vs-clone at the bytecode level; (c) the
clone overhead at the FFI boundary is `Vec<T>::from(slice)`-class
memcpy, not Arc-clone-class atomic-refcount work, so the residual
is bounded.

**Trigger conditions for revisit (any one is sufficient):**

(a) **Profile shows marshal-clone overhead is significant in real
    programs.** Specifically: the per-call `Vec<u8>::from(&buf.data[..])`
    memcpy in `Vec<u8>::from_slot` registers on sampling profilers
    under realistic stdlib-heavy workloads, independent of the
    consistency-check cost from the maximalist-v2-redesign entry.
(b) **A stdlib function is identified as hot-path-critical and
    move-semantics would materially improve it.** Specifically:
    a single function (e.g. a streaming compression call, a hot
    decode loop) is shown to spend more cycles in marshal-boundary
    cloning than in its own body.
(c) **The maximalist-v2-redesign workstream lands.** That workstream
    naturally includes this one — dissolving `HeapValue` at the
    slot level lets the compiler emit move-vs-clone slot reads
    directly without a separate `MoveFromSlot` trait, since the
    slot bits already are typed pointers.

**Adjacent already-on-record entries:** the maximalist-v2-redesign
entry above is a strict superset; it would absorb this workstream.
The HeapKind trim (2026-05-06 entry) and NativeKind::Ptr(HeapKind)
extension are intermediate steps in the same direction.

**Cost saved:** 2-5 days of trait-redesign + per-stdlib-site
ownership-declaration work deferred from the current phase, where
the always-clone residual is spec-permitted and the bytecode
hot-path layer already respects ownership distinctions. Acknowledged:
each `Vec<T>::from_slot` impl in the marshal layer carries a
per-call `Vec::from(slice)` memcpy as long-term residual, with
the documented bytecode-already-has-move-clone signal as the
trigger-(a)-(b) reference for revisiting.

---

## 2026-05-06 — type_schema-cluster cross-crate migration — deferred to shape-vm cascade boundary

This is **not** a defection (no strict-typing compromise; no dispatch shape preserved). It is an **on-record deferral**: the migration is correctly typed, but the cluster's coherent migration unit straddles the shape-runtime / shape-vm boundary, and doing half of it from inside shape-runtime risks re-doing the work when the other half's consumer needs surface.

**Considered:** complete the type_schema-cluster migration in shape-runtime in this session. The four `type_schema::mod.rs` functions (`typed_object_from_pairs`, `typed_object_from_nb_pairs`, `typed_object_to_hashmap`, `typed_object_to_hashmap_nb`) need their `&[(&str, ValueWord)]` / `ValueWord` signatures updated to the strict-typed `&[(&str, ValueSlot)]` / `u64` raw-heap-pointer shape. The body simplification is mechanical (drop the `nb_to_slot` ValueWord-tag-decode dispatch; ValueSlot is already the slot). The signature update would propagate to:

- shape-runtime cluster (post-2026-05-06 sibling re-trace): the **schema_cache.rs `_nb`-suffixed helpers** (`source_schema_to_nb` / `source_schema_from_nb`) only — these are pure ValueWord serialize/deserialize wrappers around the type_schema helpers, with **zero non-test callers** workspace-wide (lsp/cli use `source_schema_from_wire`, not the `_nb` variants). Disposition is "dead-on-deleted-ValueWord" — either delete with the cluster #1 migration or earlier as standalone cleanup. ~~`simulation/engine.rs`~~ — deleted 2026-05-06 (separate defection entry above; entire engine subtree was domain-feature with no live consumer). ~~`const_eval.rs`~~, ~~`intrinsics/fft.rs`~~, ~~`stdlib_io/network_ops.rs`~~, ~~`multi_table/functions.rs`~~ — **mis-attributed to cluster #1 in the original entry**. Tracing each: const_eval's `ConstEvaluator` exposes `ValueWord` across its public API surface (it's its own coupling problem); fft.rs and multi_table/functions.rs are part of the **intrinsics-dispatch-table cluster** (a previously-unnamed sixth cluster: every `__intrinsic_*` function uses the `IntrinsicFn = fn(&[ValueWord], &mut ExecutionContext) -> Result<ValueWord>` calling convention, which is its own architectural decision separate from type_schema); network_ops.rs is part of **cluster #2 (IoHandle marshal)** because its returns wrap `IoHandle` via `ValueWord::from_io_handle`. None of these four are "tag-decode cleanup once helpers settle" sites — they each carry independent ValueWord-baked public API surfaces that need their own cluster decisions.
- shape-vm consumers (~25 sites): `executor/objects/{indexed_table_methods, datatable_methods/*, object_creation}.rs`, `executor/state_builtins/{introspection, core}.rs`, `executor/printing.rs`, `executor/builtins/special_ops.rs`, `executor/vm_impl/schemas.rs`, `compiler/{comptime, comptime_target}.rs`

**Rationalization:** "The migration is mechanical and the strict-typed shape is clear. Doing it now keeps the shape-runtime --lib count moving toward 0 and lands a coherent strict-typed type_schema before shape-vm session even starts."

**Pattern recognized:** the rationalization is correct that the migration is mechanical. The risk is signature-redo: shape-runtime can only see its own consumer's needs, not the 25 shape-vm consumer sites. Picking signatures for `typed_object_from_*` based on shape-runtime's needs alone risks discovering during shape-vm cascade that shape-vm's `executor/objects/datatable_methods/common.rs:303 typed_object_to_hashmap_nb_vm` (the VM-aware variant) wants a different shape — at which point either (a) the shape-runtime signatures get redone, with a forced re-touch of all six shape-runtime consumer sites, or (b) shape-vm builds an adapter layer reproducing what we already deleted. Both outcomes waste the migration work.

The cluster is **one coherent migration unit** spanning two crates because the helpers' contract is "construct/destructure heap-allocated TypedObject by name-keyed slot pairs" and shape-vm is the heaviest consumer. Migrating half forces signature decisions in a half-blind state.

**Alternative taken:** defer the entire type_schema-cluster (the four `type_schema::mod.rs` functions + their shape-runtime sibling consumers + their shape-vm consumers) to the shape-vm cascade boundary. The cluster lands as one coherent migration when shape-vm session starts, with full consumer context visible.

**Acknowledged immediate cost:** shape-runtime --lib does not reach 0 errors in this session. ~14 errors remain (the four type_schema functions' broken signatures + the sibling shape-runtime consumers' tag-decode patterns that would have been cleaned up in lockstep). The session-end summary commit revises the success criterion to "stdlib mass migration + misc cleanup complete; type_schema-cluster as documented next-session entry point."

**Watchlist distinction:** "skip" / "defer" are watchlist phrases for renamed-dynamic-dispatch retention. This is **not** that pattern. The deferred functions keep their current `ValueWord`-broken state (won't compile against deleted ValueWord type — by design, makes the migration boundary visible), not a renamed dispatch shim. No escape hatch is retained. No `RawBits`-style wrapper is introduced. The cluster simply doesn't compile until both halves migrate together. A reader running `cargo check -p shape-runtime --lib` sees the deferred work as `error[E0432]: unresolved imports shape_value::ValueWord` — exactly the kind of "make the absence visible" honest deletion that the bulldozer entries (set_module / parallel / plugin) used.

**Next-session entry point:** the type_schema-cluster migration is the **first** work of the next session, not buried in generic shape-vm cascade. The 4 shape-runtime helpers + 25 shape-vm consumer sites are one coherent migration unit. The cascade handover doc for the next session should call this out explicitly.

**Cost saved:** prevented signature-redo from picking shape-runtime-only-blind signatures and discovering shape-vm consumer-side mismatches during the next session. Estimate: 1-2 days of "audit consumer needs and re-touch shape-runtime signatures" follow-up avoided. Acknowledged: ~14 errors deferred from this session's drop target.

**2026-05-06 sibling-list correction (post-simulation-deletion calibration check):** the original "shape-runtime cluster: schema_cache, const_eval, fft, simulation/engine, network_ops, multi_table" framing was an undercount of cluster identity, not just a count. Once simulation/engine.rs was deleted and the remaining "siblings" were traced to their actual public-API-surface clusters, four of the six listed files turned out to belong to other clusters (intrinsics-dispatch-table for fft/multi_table; cluster #2 IoHandle for network_ops; const_eval-ValueWord-API for const_eval). Only schema_cache.rs's `_nb` helpers are genuine cluster #1 siblings, and even those are dead-code with zero non-test callers. The next session's actual cluster #1 work is therefore **smaller than this entry originally implied** at the shape-runtime side — the four `type_schema::mod.rs` helpers + the dead schema_cache `_nb` wrappers — and **larger at the shape-vm side** by the unchanged 25 sites. This calibration finding fits the meta-pattern in the Phase-2c handover: "mechanical mass migration claims systematically undercount architectural prerequisites" — except here the claim was structural mis-attribution, not under-count. **Watchlist addition:** when a deferral entry lists "sibling consumers" by file name, verify each file's actual public-API-shape cluster identity rather than assuming files near the helper-call site belong to the same cluster. File adjacency is not cluster identity.

**Newly-named cluster (intrinsics-dispatch-table):** the `IntrinsicFn = fn(&[ValueWord], &mut ExecutionContext) -> Result<ValueWord>` calling convention in `crates/shape-runtime/src/intrinsics/mod.rs` (line 34) is the legacy FFI-style intrinsics dispatch shape. It is its own architectural decision (typed marshal, slot-based, or deletion-by-replacement-with-method-dispatch) — independent of the five existing clusters. Affected files: `intrinsics/{fft,matrix,convolution,distributions,math,random,recurrence,rolling,scan,stochastic,vector,array_transforms}.rs` plus `multi_table/functions.rs`. Listed here as the sixth cluster surfaced by the cluster #1 sibling re-trace; deserves its own surface-and-decide round-trip in a future session before any of these files migrate.

---

## 2026-05-06 — simulation engine deletion — domain-specific feature deferred to extensions workstream

**Considered (option A, typed-state retrofit in shape-runtime):** redesign `crates/shape-runtime/src/simulation/engine.rs` to replace `type Value = ValueWord` with a slot-based or schema-keyed typed-state abstraction (e.g. `pub struct TypedState { schema: Arc<TypeSchema>, slots: Box<[ValueSlot]> }`), thread it through `StepResult.state`, the `StepHandler = Fn(&Value, &Value, usize) -> Result<StepResult>` callback, `SimulationEngineConfig::with_initial_state`, and `SimulationEngineResult.to_value`. The kernel modules (`dense_kernel.rs`, `hybrid_kernel.rs`, `correlated_kernel.rs`, `parallel.rs`, `event_scheduler.rs`, `validation.rs`) follow with their own typed-state contracts. Approximate scope: ~2,800 LoC rewrite across 8 simulation files + KernelCompiler trait redesign on `context/mod.rs` + `shape-jit::compiler::setup.rs`'s `JITKernelCompiler` impl + `benches/simulation_bench.rs`.

**Rationalization (option A):** "The simulation kernels are existing analytical infrastructure. A typed-state retrofit preserves them under the strict-typed shape so simulation-domain workloads (dense / hybrid / correlated / event-driven) continue to work end-to-end. Deleting working machinery feels wasteful when a mechanical retrofit keeps it operational, and shape-jit's `JITKernelCompiler` already implements the trait."

**Pattern recognized (option A):** identical structural shape to the AST-evaluation executors entry below (window/stream/join/pattern-state-machine deletion) and to the heap_value.rs option-A rejection: preserving runtime infrastructure under a typed-looking shape that no live core consumer exercises. Audited consumers of the simulation surface:

- `SimulationEngine` / `SimulationEngineConfig` / `StepResult` / `StepHandler` / `SimulationEvent` / `SimulationMode` — only out-of-tree references are `crates/shape-runtime/benches/simulation_bench.rs` (bench harness) and tests inside `engine.rs` itself. No `lib.rs`-level consumer reads `to_value()`; no production caller invokes `engine.run()`.
- `KernelCompiler` trait + `KernelCompileConfig` + `SimulationKernelFn` — implemented by `shape-jit::compiler::setup.rs::JITKernelCompiler`. The trait is wired through `ExecutionContext::set_kernel_compiler` / `ExecutionContext::kernel_compiler()` (`context/mod.rs:748-755`), but `set_kernel_compiler` has zero workspace callers and `kernel_compiler()` has zero workspace readers — the hook is dead-code from the strict-typed compile. The `JITKernelCompiler::compile_kernel` method has no production caller either; it sits as a trait impl waiting for a wiring path that never materialized.
- `dense_kernel`/`hybrid_kernel`/`correlated_kernel` / `parallel`/`event_scheduler` / `validation` — re-exported in `simulation/mod.rs:43-64` but no out-of-tree consumer references the re-exports. `shape-vm`'s `executor/objects/datatable_methods/simulation.rs::handle_simulate` is a separate VM-side simulation builtin that does NOT depend on `shape_runtime::simulation::*` (it imports only `shape_runtime::type_schema::FieldType` and `shape_runtime::context::ExecutionContext`).

The kernel-trait-on-Context pattern is exactly the W-series defection attractor: a typed shell with no live consumer that future sessions route new work through. Same shape as set_module / parallel_module / plugin / AST-executors. Domain-specific runtime feature, no live core consumer, preserving any part of it under any typed wrapper recreates the attractor.

**Considered (option B, partial-typed compromise):** delete the engine and kernel files but keep the `KernelCompiler` trait + `KernelCompileConfig` + `SimulationKernelFn` types as "shared infrastructure" so `shape-jit::JITKernelCompiler` continues to compile against a host-side trait. The `ExecutionContext` hook stays as a typed extension point.

**Pattern recognized (option B):** the optional-defection-becomes-default dynamic, applied to the kernel-compiler hook. Once `KernelCompiler` survives as a typed shell, the next session looking for "where does compiler-emitted simulation kernel registration go" will find it ready and route through it. The trait's `unsafe extern "C" fn` payload hard-codes the legacy two-pointer-state simulation ABI; preserving the trait locks in that ABI before the post-deletion strict-typed simulation ABI is even drafted. Dynamic-runtime API surface rebranded as typed extension point.

**Alternative taken (option C):** delete the entire `crates/shape-runtime/src/simulation/` subtree (8 files: `mod.rs`, `engine.rs`, `dense_kernel.rs`, `hybrid_kernel.rs`, `correlated_kernel.rs`, `parallel.rs`, `event_scheduler.rs`, `validation.rs`), `crates/shape-runtime/benches/simulation_bench.rs`, the `lib.rs:85` `pub mod simulation;` line, the `KernelCompiler` import + `kernel_compiler` field + `set_kernel_compiler`/`kernel_compiler()` methods + `kernel_compiler: None` initializers in `context/mod.rs` (lines 30/115/242/285/331/381/748-755), and the `kernel_compiler: self.kernel_compiler.clone()` field-clone in `context/scope.rs:77`. Add follow-up workstream `simulation-kernel-extension-rebuild`: simulation kernels rebuild as a domain extension on top of the strict-typed core when there is a real consumer driving the work, following the precedent of `extensions/python/` and `extensions/typescript/` (domain runtime workstreams live in extension crates, not embedded in the core's compile-blocking surface). `shape-jit::compiler::setup.rs`'s `JITKernelCompiler` impl + the corresponding `pub use` lines in `shape-jit::compiler::mod.rs:35` / `shape-jit::lib.rs:52` become orphaned and will be cleaned up during the shape-jit cascade pass — acknowledged: shape-jit cascade work shrinks by these sites rather than growing.

**Cost saved:** option A would re-create the W-series shape at the simulation-runtime layer — typed shells with no live consumer attract new code that hard-couples to the inadequate shape before the typing model is load-bearing. Option B preserves the kernel-compiler hook as a typed extension point that locks in the legacy two-pointer simulation ABI. The set_module / parallel_module / plugin / AST-executors precedent applies: honest deletion makes the absence visible; a typed shell hides the gap. Acknowledged user-visible cost: simulation kernels (dense / hybrid / correlated / event-driven / parameter-sweep) are non-functional from the strict-typed runtime until rebuilt as an extension; downstream benchmark / shape-jit kernel-compile sites either fail explicitly or get reworked alongside the rebuild workstream. Calibration prediction: shape-runtime --lib drops by ~10-12 errors from the simulation-cluster siblings' broken `ValueWord`/`ArgVec`/`vmarray_from_vec` references (engine.rs alone contributes 3+ errors plus its own re-export breakage). Estimate avoided: 2-3 weeks of "redesign simulation typed-state contract before any consumer drives the shape" follow-up.

---

## 2026-05-06 — AST-evaluation runtime executors deletion — no live consumer in strict-typed compile

**Considered (option A, typed-slot rewrite):** rewrite `crates/shape-runtime/src/window_executor.rs`, `stream_executor.rs`, `join_executor.rs`, `pattern_state_machine.rs`, the `ExecutionContext::variable_scopes` machinery (`context/variables.rs`'s `set_variable_nb` / `declare_pattern` / `set_pattern` and recursive destructure), and the lib.rs query-exec body to thread `(u64 bits, NativeKind kind)` typed slots instead of `ValueWord`. Replace `Variable.value: ValueWord` with `Variable.value: TypedSlot { bits: u64, kind: NativeKind }`; dispatch pattern-destructure on kind. Approximate scope: ~2,000 LoC rewrite across four executor files + ~150 LoC across context/variables.rs + the lib.rs stub bodies.

**Rationalization (option A):** "The executors and pattern-destructure paths are existing analytical infrastructure. A typed-slot rewrite preserves them under the strict-typed shape so the streaming/windowed/joined analytics are ready when downstream code wires them up. Deleting working machinery feels wasteful when a mechanical retrofit keeps it operational."

**Pattern recognized (option A):** identical structural shape to the option-A pattern from heap_value.rs reconstruction (2026-05-06): preserving runtime infrastructure under a typed-looking shape that no current consumer exercises. The cost is exactly the W-series defection attractor — typed-but-inadequate shells reliably attract new code routed through them before the typing is properly load-bearing. Same shape as the set_module / parallel_module / plugin entries: machinery whose polymorphism is the point, masquerading as typed surface. Audited consumers of the four executors:

- `WindowExecutor` / `StreamExecutor` / `JoinExecutor` / `PatternStateMachine` — only references outside their own files are `pub mod` / `pub use` re-exports in `lib.rs`, plus a doc comment in `engine/mod.rs:36`. All `::execute` / `::new` invocations are inside `#[cfg(test)]` blocks in those same files.
- shape-vm's window / join builtins (`crates/shape-vm/src/executor/window_join.rs:115, :266`) re-implement the work inline against `ValueWord` directly. The "delegate to the runtime WindowExecutor" comment at `vm_impl/builtins.rs:497` is a stale lie left over from a pre-bytecode era.
- `lib.rs::execute_query_with_context` and `execute_without_data` are routed only through `query_executor.rs:178`, which calls `execute_query` and then builds its public `QueryResult` from `matches`/`statistics` only — `RuntimeQueryResult.value: Option<ValueWord>` is never read by any non-test consumer.
- `set_variable_nb` is called from window_executor.rs:173 / :387, stream_executor.rs:323, join_executor.rs:207 / :212 — i.e., only from the four dead executors and the lib.rs stubs that pass `ValueWord::none()` literally.

**Considered (option B, partial-typed compromise):** keep `ExecutionContext::variable_scopes` and pattern destructure, type their storage with `(bits, kind)`, but delete only the four executor files. The variable-scope plumbing remains as "shared infrastructure" for whatever rebuilds streaming/windowed analytics.

**Pattern recognized (option B):** the optional-defection-becomes-default dynamic. Once `ExecutionContext::variable_scopes` survives as a typed shell with no consumer, the next session looking for "where do I store named bindings" will find it ready and route through it — and the typed-but-inadequate shell will become load-bearing before the typing model is ready. The strict-typed answer for variable storage is compiled stack slots, not a HashMap-keyed scope chain. Keeping the scope chain compiled-and-typed creates two storage models for variables, and the simpler one will win adoption regardless of fit.

**Alternative taken (option C):** delete `crates/shape-runtime/src/window_executor.rs`, `stream_executor.rs`, `join_executor.rs`, `pattern_state_machine.rs`, the lib.rs query-exec stub bodies (`execute_query_with_context`, `execute_without_data`, plus their `pub use` / `pub mod` lines), the `QueryResult.value` field (no live reader), and the `set_variable_nb` / `declare_pattern` / `set_pattern` methods plus their callers from `context/variables.rs`. Update the `vm_impl/builtins.rs:497` comment to drop the "delegate to runtime executor" lie. Add follow-up workstream `ast-walking-interpreter-strict-rebuild`: streaming/windowed/joined analytics will be rebuilt on compiled-bytecode + typed VM slots when there is a real consumer, not on a variable-scope HashMap. Estimated immediate impact: ~3,000 LoC deleted; lib.rs cascade collapses; calibration prediction -25 to -35 errors from the 172 baseline.

**Cost saved:** option A would re-create the W-series shape at the AST-evaluation layer — typed shells with no consumer attract new code that hard-couples to the inadequate shape before the typing is load-bearing. Option B preserves the same dynamic at the variable-scope layer specifically. The set_module / parallel_module / plugin precedent applies: honest deletion makes the absence visible; a typed shell would hide the gap. Acknowledged user-visible cost: the `Runtime::execute_query`, `Runtime::execute_without_data`, and the four executor types are non-functional from the strict-typed runtime until rebuilt; downstream callers (`query_executor.rs::execute`) need to either fail explicitly or be reworked alongside the rebuild workstream. Estimate: 2–3 weeks of "audit which code expected this typed shell" remediation avoided.

---

## 2026-05-06 — heap_value.rs Phase-2 reconstruction: rejected `u64` and `HeapValue` substitution

**Considered (option A):** mechanically replace every `ValueWord` reference in `crates/shape-value/src/heap_value.rs` and `heap_variants.rs` with `u64`. This unblocks the file compile fastest. The 13 heap-side data structures (`HashMapData`, `SetData`, `DequeData`, `PriorityQueueData`, `IteratorState`, `IteratorTransform`, `GeneratorState`, `ConcurrencyData`, `SimulationCallData`, `RefProjection::Index`, `ProjectedRefData`) keep their shape; the `Some`/`Ok`/`Err`/`Range`/`TraitObject`/`FunctionRef` variants keep their `Box<u64>` payloads.

**Rationalization (option A):** "It's the smallest mechanical change. Drop/Clone impls keep working — the `vw_clone`/`vw_drop` calls become bare bit copies. We can move on to shape-vm and clean up the semantics later."

**Pattern recognized (option A):** classic compromise pattern — keep the dynamic data structures, just rename the type to `u64` so it looks like primitive bits. The Drop/Clone refcount semantics quietly break (no longer paired retain/release on heap pointers stored in collections), and now the codebase has live ref leaks / double-frees in collection paths. This is option A from `~/.claude/plans/strict-typing-phase-2-handover.md`'s analysis. It is dynamic-runtime semantics rebranded as typed bits — the W-series footgun.

**Considered (option B):** substitute `Vec<HeapValue>` for `Vec<ValueWord>` and `Box<HeapValue>` for `Box<ValueWord>` throughout the heap-side data structures. The hetero-collections (`HashMapData`, etc.) stay, just become typed sum-type holding `HeapValue` recursively.

**Rationalization (option B):** "It's strict-typed in the sense that `HeapValue` is a typed enum. The collections become heterogeneous typed-sum-type containers, which is what the plan literature describes as the canonical encoding for heterogeneous data."

**Pattern recognized (option B):** misreads the plan. Heterogeneous collections aren't strict-typed in any meaningful sense — they preserve dynamic dispatch by promoting the runtime-tag-decode dispatch from `ValueWord`'s tag bits to the `HeapValue` enum's discriminant. The dispatch site in `find_key`/`contains`/`vw_hash` doesn't get cheaper; it just dispatches on `match heap_value { ... }` instead of `match tag { ... }`. The `runtime-v2-spec.md:180` direction (monomorphized typed buckets per `HashMap<K, V>` instantiation) is incompatible with this representation. Picking B locks in heterogeneous-by-default at the heap level, which is the very thing strict-typing exists to remove.

**Alternative taken (option C):** delete every HeapValue variant whose payload depends on `ValueWord` or holds a heterogeneous-typed collection. The variants `Some`/`Ok`/`Err`/`Range`/`TraitObject`/`FunctionRef`/`HashMap`/`Set`/`Deque`/`PriorityQueue`/`Iterator`/`Generator`/`ProjectedRef`/`Concurrency`/`SimulationCall` are removed from `HeapValue` along with their `*Data` structs. The cascade surfaces every consumer in shape-vm/shape-runtime/shape-jit; they will be redesigned as monomorphized typed structures (typed buckets for HashMap, monomorphized `Option<T>` / `Result<T, E>` / `Range<T>` as TypedStructs) in a later phase or as part of the cascade fix.

**Cost saved:** option A would have rebuilt the `vw_clone`/`vw_drop` machinery within months under a different name (the W-series pattern reproduced). Option B would have locked in heterogeneous-by-default heap representation, blocking the v2 typed-buckets migration. Option C aligns the bulldozer with `runtime-v2-spec.md`'s direction. Estimated avoided cost: 4–8 weeks of follow-up cleanup. Acknowledged immediate cost: significantly larger Phase 2 cascade in shape-vm.

## 2026-05-06 — shape-runtime Phase-2 reconstruction: TypedReturn ValueWord hatches deleted

**Considered:** retain `TypedReturn::ValueWord(ValueWord)`, `TypedReturn::ArrayValueWord(Vec<ValueWord>)`, and `TypedReturn::HashMapValueWord { keys, values }` as escape hatches in the typed stdlib return ABI, mechanically substituting `ValueWord` → `u64` so they compile against the post-bulldozer shape-value crate.

**Rationalization:** "TypedReturn already documents these as 'escape hatches narrowed module-by-module across the migration' (`typed_module_exports.rs:124-130`). They were never load-bearing for the typed return shapes; renaming the inner type to `u64` (with an attached `NativeKind` discriminator if needed) is mechanically the smallest change to keep the marshalling layer compiling. Each consumer (set/parallel/parsers/plugins) has a known follow-up workstream — we'd be honest about the deferral."

**Pattern recognized:** classic W-series rename. The variants exist precisely *because* the function bodies need a polymorphic return — `set_module` returns the user's element type, parsers return arbitrary user data trees, the plugin ABI is by definition opaque. Substituting `ValueWord` for `u64` does not remove the polymorphism; it relabels it. The `into_value_word()` marshalling boundary then has to dispatch on whatever kind discriminator `u64` carries, which means reintroducing tag-decode dispatch under a different name. This is "Rename to a less suspicious name" from the CLAUDE.md forbidden list, applied to the return-type ABI.

**Alternative taken:** delete `TypedReturn::ValueWord`, `TypedReturn::ArrayValueWord`, and `TypedReturn::HashMapValueWord`. `HashMapValueWord` has zero callers (already dead). For `ValueWord`/`ArrayValueWord`, every consumer falls into one of three buckets (audited 2026-05-06):

1. **Mechanically migratable** (13 sites in http/archive/csv/regex/arrow): use existing `TypedReturn::ObjectPairs` / `ArrayObjectPairs` / `DataTable` variants. Done in Step 3.
2. **Architecturally cut** (set/parallel/plugin): see follow-up entries below — the modules are deleted from the strict-typed build with explicit follow-up workstreams.
3. **Architecturally rebuilt** (json/yaml/toml/msgpack/xml parsers): see `JsonValue` entry below — typed sum-type enum replaces ValueWord-tree return.

**Cost saved:** keeping the hatches would have forced the marshalling boundary to carry `NativeKind` per `u64`, reproducing `ValueBits`-shim machinery under the `TypedReturn` enum. Estimate 2–4 weeks of follow-up cleanup avoided. Acknowledged immediate cost: ~30 consumer sites to migrate or delete, plus 4 follow-up workstreams logged.

---

## 2026-05-06 — shape-runtime: `set_module` deleted from strict-typed build

**Considered:** keep `crates/shape-runtime/src/stdlib/set_module.rs` and rename its `TypedReturn::ValueWord` returns to `TypedReturn::RawBits { kind, bits }` (or equivalent). The eight `Set<T>` operations (new/insert/delete/contains/union/intersect/difference/to_array) all return either a `Set` heap object or its element type, both of which are user-parametric.

**Rationalization:** "Sets are fundamental container types and shipping a strict-typed compile without `Set` is a feature regression. A `RawBits` discriminator wrapper around the existing implementation preserves the API."

**Pattern recognized:** `Set<T>` is parametric in element type. The strict-typed answer per `runtime-v2-spec.md:180` is monomorphized per-instantiation typed buckets — the same shape as the typed-`HashMap<K, V>` direction. A `RawBits` wrapper keeps the heterogeneous-by-default dispatch alive under a new name (the option-B pattern from heap_value.rs reconstruction, applied to a different layer). It also preserves the `HashMapValueWord`-shaped storage that the bulldozer just deleted from `HeapValue` — re-creating in stdlib what the bulldozer removed from the runtime would be the W-series defection in a different file.

**Alternative taken:** delete `crates/shape-runtime/src/stdlib/set_module.rs` and remove its registration from the stdlib registry. Add a follow-up workstream `set-module-strict-monomorphization` to `CLAUDE.md`'s "Known Constraints" section: rebuild Set as monomorphized per-element-type buckets when the compiler can pin element type at the registration site (same prerequisite as typed-buckets `HashMap`).

**Cost saved:** the `RawBits` rename would compound across the typed-collections subsystem (Deque, PriorityQueue, … all already deleted from `HeapValue` for the same reason). Honest deletion makes the absence visible; a renamed wrapper would hide the gap. Estimate: 2-week monomorphization workstream deferred, but cleanly. Acknowledged user-visible cost: `import { Set } from std::core::collections` stops working until the workstream lands.

---

## 2026-05-06 — shape-runtime: `parallel` module deleted from strict-typed build

**Considered:** keep `crates/shape-runtime/src/stdlib/parallel.rs` (parallel_map/filter/chunks/reduce/sort over a user closure) and have its `TypedReturn::ValueWord` returns dispatch on the closure's runtime return kind.

**Rationalization:** "Parallel collection ops are a perf headline feature. Closures already return `ValueWord`-shaped values via the VM call convention; the `parallel_*` wrapper just threads them through. A small dispatch on the closure's last-emitted kind is enough to pick the right typed marshal."

**Pattern recognized:** "small dispatch on the closure's last-emitted kind" is `last_program_return_kind` reborn — exactly the Pattern A defection that bulldozer commit `90fc2e9` removed. The closure return type is parametric; without monomorphizing the call wrapper per closure-return-type, any solution at the stdlib layer is dynamic dispatch on a kind discriminator. Identical structural shape to the `set_module` case.

**Alternative taken:** delete `crates/shape-runtime/src/stdlib/parallel.rs` and remove its registration. Add `parallel-module-strict-monomorphization` follow-up workstream alongside `set-module-strict-monomorphization`. Both share the same prerequisite (compiler pins element/return type at the registration site); they should land together.

**Cost saved:** preserved the bulldozer-deleted `last_program_return_kind` infrastructure from sneaking back in through the stdlib closure-call wrapper. Estimate: 1–2 week parallel-monomorphization workstream deferred. Acknowledged user-visible cost: `parallel_map`/`parallel_filter`/etc. unavailable until rebuilt.

---

## 2026-05-06 — shape-runtime: plugin native-call passthrough disabled

**Considered:** preserve `plugins/module_capability.rs:155` (`Result<ValueWord> → TypedReturn::ValueWord` passthrough) by routing the plugin's return through the renamed `RawBits` discriminator, since the plugin ABI is by definition opaque to the host runtime.

**Rationalization:** "The plugin returns whatever it wants — there is no static type for that. A passthrough `RawBits` is genuinely all the host can know."

**Pattern recognized:** the same dispatch-by-rename pattern. "The plugin ABI is opaque" is true today *because* it was designed to thread `ValueWord` through. The strict-typed answer is that plugins must declare typed signatures at registration, just like the typed-stdlib already does. Keeping a `RawBits` passthrough makes the typed registration optional — and optional defection mechanisms reliably become the default.

**Alternative taken:** delete the `TypedReturn::ValueWord` line at `plugins/module_capability.rs:155`. The single call site is the optional plugin native-call dispatcher; disabling it means plugins that registered for native-call routing no longer dispatch through this path. Add `plugin-typed-abi` follow-up workstream to `CLAUDE.md` Known Constraints. Plugins are not load-bearing for the strict-typed compile (extensions/python and extensions/typescript flow through `LanguageRuntimeVTable`, which is unaffected — `docs/strictly-typed-baseline.md:36` documents 0 ValueWord references in either extension).

**Cost saved:** prevented the optional-defection-becomes-default dynamic. Estimate: 1-week plugin typed-ABI workstream deferred. Acknowledged user-visible cost: the specific `register_plugin_native_call` codepath is non-functional until rebuilt; the broader plugin system remains intact.

---

## 2026-05-06 — shape-runtime parsers: typed `JsonValue` over ValueWord-tree return

**Considered (option α):** make `parse_json(s: string) -> ValueWord` (and parallels for yaml/toml/msgpack/xml) return a `ValueWord` whose tag bits encode the parsed shape (string/number/bool/array/object). The stdlib body would build the tree by `ValueWord::from_*` and `from_hashmap_pairs` — unchanged from pre-bulldozer code modulo the `ValueWord` type alias.

**Rationalization (option α):** "Parsers return arbitrary user data — there is *literally* no static type for the result of `parse_json` because the input can be anything. A `ValueWord`-tree return is honest about that. Trying to introduce a typed enum is just rebranding the same dynamic dispatch."

**Pattern recognized (option α):** confuses "the input is dynamic" with "the runtime representation must be dynamic." JSON's own specification has six concrete value kinds (null/bool/number/string/array/object) and pattern-matching on those six is exactly the strict-typed answer the plan calls out (`stop-native-vs-tagged-tax.md` line ~17, the parsers entry). Returning `ValueWord` makes `match parse_json(s)` impossible from Shape user code (no exhaustive case analysis); returning a typed enum makes it natural and forces the compiler to verify the user handled every variant.

**Considered (option β):** different per-parser typed enum (`JsonValue`, `YamlValue`, `TomlValue`, `MsgPackValue`, `XmlValue`) with each parser owning its own variant set.

**Rationalization (option β):** "TOML has a `DateTime` variant JSON doesn't have; MsgPack has a `Bytes` variant; YAML has tag annotations. Preserving each format's expressive surface lets users pattern-match on format-specific cases."

**Pattern recognized (option β):** five near-identical sum types with overlapping cases is structural duplication. Users serializing data through multiple formats would need conversion adapters between every pair. The right grain is *one* shared type with the union of variants — formats that don't have a given variant simply never construct it.

**Alternative taken (option γ):** define `crate::json_value::JsonValue` as a single concrete sum-type enum:
```rust
pub enum JsonValue {
    Null,
    Bool(bool),
    Int(i64),
    Number(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}
```
Each parser's stdlib body returns `Result<JsonValue, ParseError>`; the typed-stdlib registry materialises that into the user-facing `JsonValue` Shape type via a new `TypedReturn::JsonValue(JsonValue)` variant. Insertion order preserved in `Object` via `Vec<(String, JsonValue)>` (not `HashMap`). The name `JsonValue` (over `ParsedValue` / `DataTree`) follows the de-facto industry convention and matches the user's stated direction; format-specific extensions (TOML `DateTime`, YAML tag annotations) either project losslessly into existing variants (DateTime → `Object` with a sentinel field, or `String` ISO-8601) or trigger a follow-up if the lossy projection is unacceptable.

**Cost saved:** option α reproduces the dynamic-dispatch dispatch-on-tag pattern at the parser layer — exactly the W-series footgun in fresh skin. Option β fragments the parsed-value surface into 5 redundant types. Option γ matches `runtime-v2-spec.md` direction (concrete typed sum types over heterogeneous bytes). Estimate: ~1 week parser migration vs. ~3-4 weeks of follow-up cleanup if option α landed.

---

## 2026-05-06 — JsonValue user-facing surface: Shape enum over fluent accessor methods

**Considered:** expose `JsonValue` to Shape user code as a *fluent accessor object* — `parse_json(s).is_object()`, `parse_json(s).as_string()`, `parse_json(s).get("key")`, `parse_json(s).length()`. No exhaustive pattern matching from Shape; the type's "shape" is discovered at runtime via accessor probes.

**Rationalization:** "Pattern matching on a sum type with eight variants is ergonomic noise for the common case (`json["users"]["0"]["name"]`). Fluent accessors mirror what JS / Python / Ruby users expect from a JSON library. The exhaustive-match enum forces users to handle variants they don't care about."

**Pattern recognized:** the fluent surface preserves runtime dispatch *inside the accessor methods* — `as_string()` is a per-call tag-decode probe returning `Option<&str>`, identical to the `nb.as_str()` decoder pattern that the strict-typing plan deletes from the runtime. The compiler cannot verify exhaustiveness because there are no cases to verify; users discover their parser wasn't returning what they thought via runtime `None`. This is the runtime-tag-decode pattern at the Shape-language level — same shape as the `set_module` and `parallel_module` polymorphism that we deleted, just dressed up as method calls. Per CLAUDE.md "No `any` type" rule, dispatch on parsed-data shape is exactly the kind of "discover-at-runtime" pattern that Shape's static typing exists to remove.

**Alternative taken:** expose as a Shape-level typed sum-type enum (Phase 2c when wired up):
```shape
enum JsonValue {
    Null,
    Bool(bool),
    Int(int),
    Number(number),
    String(string),
    Bytes(Array<int>),
    Array(Array<JsonValue>),
    Object(HashMap<string, JsonValue>),
}
```
Users pattern-match exhaustively; the compiler verifies every case is handled. Convenience accessors (`obj.get("key")`, etc.) can be added as ordinary methods once the enum is in place — they compose on top, they don't replace exhaustive matching.

**Cost saved:** keeping fluent accessors as the only surface would have re-introduced runtime tag-decode at the language level — exactly what the strict-typing plan removes from the runtime. Estimate: 2-3 weeks of follow-up cleanup avoided when downstream user code starts pattern-matching parsed values exhaustively. Acknowledged immediate cost: Shape user code becomes more verbose for "I just want the string" cases until convenience methods land alongside the enum.

---

## 2026-05-06 — TypedReturn recursive variants: structural Concrete/Container split

**Considered:** keep `TypedReturn` as one flat enum; rely on registration-time validation to ensure that `TypedReturn::Ok(Box::new(TypedReturn::ValueWord(...)))` and similar nested-defection patterns never appear in production code. Add a debug assertion or test sweep that walks the enum tree and panics on `ValueWord` nested inside `Ok`/`Err`/`Some`/etc.

**Rationalization:** "The marshal layer can detect the violation at runtime and reject. Tests can sweep registered functions for the bad shape and fail loudly. The Rust type system has limits and a runtime assertion is good enough — every other strict-typed compiler invariant is enforced this way."

**Pattern recognized:** "trust registration validation" is the runtime-discipline pattern, and runtime discipline is the same shape as runtime tag-decode dispatch. The W-series defection ("ValueBits shim retained as documented FFI-boundary bridge") was protected by the same kind of runtime-discipline argument — "we'll narrow it module-by-module, validation will catch backsliding." Five sessions later it was permanent. The strict-typing plan's mechanical-enforcement section (`CLAUDE.md` line 261) is explicit: "make the forbidden state unrepresentable, not just unreachable" — the `ProofGap` private-constructor pattern. Applying that same discipline here means making `TypedReturn::Ok(Box::new(TypedReturn::ValueWord(...)))` a *type error*, not a runtime check.

**Alternative taken:** structurally split `TypedReturn` into a two-tier enum:
```rust
/// Strictly-typed leaf values. No recursion; no escape hatches.
pub enum ConcreteReturn {
    I64(i64), F64(f64), Bool(bool), Unit, String(String),
    Instant(std::time::Instant),
    ArrayI64(Vec<i64>), ArrayF64(Vec<f64>), ArrayString(Vec<String>),
    Bytes(Vec<u8>),
    HashMapStringString(Vec<(String, String)>),
    DataTable(std::sync::Arc<DataTable>),
    // (post Phase 2c) JsonValue(JsonValue) — typed-tree parsed data.
}

/// Container variants. Payload is *only* a ConcreteReturn — by construction.
pub enum TypedReturn {
    Concrete(ConcreteReturn),
    Ok(ConcreteReturn),
    Err(ConcreteReturn),
    Some(ConcreteReturn),
    None,
    ObjectPairs(Vec<(String, ConcreteReturn)>),
    TypedObject(Vec<(String, ConcreteReturn)>),
    ArrayObjectPairs(Vec<Vec<(String, ConcreteReturn)>>),
}
```
The Rust type system enforces that `Ok`/`Err`/`Some` cannot wrap another `Ok`/`Err`/`Some` (which is correct — `Result<Result<T,E>,F>` would be a registration bug regardless), and cannot wrap a `ValueWord` escape hatch (because no such variant exists in `ConcreteReturn`). Shape-language types like `Result<Result<T,E>,F>` aren't first-class today (`CLAUDE.md` Known Constraints: "Generic impls parse but are not first-class end-to-end"); if they become first-class later, the split grows a third tier rather than reverting.

**Cost saved:** prevented the optional-defection-becomes-default dynamic that put the prior plan in the W-series death spiral. Estimate: 1-2 weeks of "audit nested TypedReturn" follow-up cleanup avoided. Acknowledged immediate cost: every consumer that built `TypedReturn::Ok(Box::new(TypedReturn::String(...)))` becomes `TypedReturn::Ok(ConcreteReturn::String(...))` — slightly more verbose, but the verbosity *is* the proof.

---

## 2026-05-06 — Phase 2b unified marshal + wire/snapshot kind threading

The strict-typed runtime needs a single mechanism for projecting typed values across **every** ABI exit: the stdlib dispatch boundary (return side AND arg side), the wire-serialization boundary, and the snapshot/state-diff boundary. These are not three independent problems — they are three points where a typed slot crosses a non-typed boundary, and the strict-typed answer is the same at each point: **`(u64 bits, NativeKind kind)` paired**, threaded from compile-time slot-kind metadata, no runtime tag-decode.

This entry covers all four cuts and the alternatives rejected at each.

---

**Considered (option α, RETURN side):** restore `TypedReturn::into_value_word(self) -> ValueWord` (or its successor `into_some_intermediate_value`) — a synthesized 8-byte intermediate that the stack-push logic later decodes. Decode-on-push, encode-on-marshal.

**Rationalization (option α):** "The intermediate value is `u64`, not `ValueWord` — there's no tag dispatch, just a width-uniform transport. The stack-push logic already knows the slot's kind from the FunctionBlob. The intermediate is invisible to user code."

**Pattern recognized (option α):** identical to `ValueWord` semantically — an 8-byte word that carries a value whose interpretation is determined elsewhere. The fact that the discriminant moves from "tag bits in the same word" to "kind table in the FunctionBlob" doesn't change the dispatch shape. Worse: it adds a temporary that exists only at the marshal boundary, asking future readers to remember "this `u64` is post-marshal pre-push and the kind comes from a separate table." Identical defection shape to the W4 ConvertBoolToString opcode — synthesizing an intermediate to paper over a kind-tracker gap. The right fix is to project directly into the typed slot.

**Considered (option β, ARG side):** `TypedArgReader` trait with methods `read_i64(idx)`, `read_f64(idx)`, `read_str(idx)`, etc. Bodies pick the right reader per arg based on what they declared at registration. Registration validation enforces that the body's `read_*` calls match the declared param kinds.

**Rationalization (option β):** "The trait gives the body no way to *probe* — it must commit to a kind per call. That's structural enforcement at the call boundary."

**Pattern recognized (option β):** committal at the call site is not the same as committal at the type level. A body declared with `params: [Int]` that calls `read_f64(0)` is a registration-time bug, not a type-checker error — the trait permits it. "Registration validation catches it" is the runtime-discipline pattern; identical shape to the rejected "trust registration validation" approach for the recursive `TypedReturn` variants in the 2026-05-06 split entry. The same defection in a different file.

**Considered (option γ, ARG side):** macro-per-function that emits `fn read_arg0_i64(&self) -> i64`, `fn read_arg1_str(&self) -> &str`, etc., one per registered function, with kinds fixed at macro-expansion time.

**Rationalization (option γ):** "Macros emit per-function readers tied to the registration declaration, so kinds match by construction at the per-function call site."

**Pattern recognized (option γ):** structural enforcement, but with macro machinery doing what the type system can do directly. The trait-based generic approach below achieves the same property with no macros — and macros forfeit the readability of `fn parse_json(s: Arc<String>, ctx: &ModuleContext) -> Result<TypedReturn, MarshalError>`.

**Considered (option δ, RETURN+ARG):** one-tier discriminated union `enum SlotValue { Int(i64), Float(f64), Bool(bool), Heap(Arc<HeapValue>), Unit }` carried across the marshal boundary.

**Rationalization (option δ):** "It's a typed sum-type, not a tagged word. The variants are concrete; consumers `match` exhaustively."

**Pattern recognized (option δ):** `ValueWord` reborn. The dispatch moves from "tag bits in `u64`" to "enum discriminant in `SlotValue`," but the dispatch *exists* — every consumer pattern-matches on the discriminant. The entire deletion of HeapValue's `HashMap`/`Some`/`Ok`/`Err`/`Range`/etc. variants (commit `7d6dc27`, the option-C heap_value cut) was about removing exactly this kind of heterogeneous-by-default sum type from the runtime. Re-creating it at the marshal layer is the same defection in a higher layer.

**Considered (option ε):** Rust generics with phantom-typed `Slot<K: NativeKind>`, encoding the kind at compile time and eliminating the runtime discriminator entirely.

**Rationalization (option ε):** "Maximum strict-typing — the kind is in the type."

**Pattern recognized (option ε):** sound but out of scope. The VM stack is monomorphic 8-byte slots; phantom-typed slots would require a full executor-stack rewrite. The cost-benefit doesn't fit Phase 2b's budget. Filed as a hypothetical follow-up workstream `phantom-typed-stack` should the strict-typed approach show frequent reader-error patterns.

---

**Alternative taken (the unified Phase 2b shape):** every ABI exit becomes a `(u64 bits, NativeKind kind)` pair, threaded from compile-time `NativeKind` metadata on the calling side. Three concrete sub-mechanisms:

**Sub-mechanism A — stdlib dispatch (return side):**
```rust
pub fn marshal(ret: TypedReturn, expected: NativeKind, push: &mut SlotWriter)
    -> Result<(), MarshalError>;
```
`expected` comes from the function's registered `ConcreteType.to_native_kind()`. Mismatch is `MarshalError::Mismatch { expected, got }` — typed error, not panic. The marshaller projects directly to the typed slot via `push`; no synthesized intermediate.

**Sub-mechanism B — stdlib dispatch (arg side):**
```rust
pub trait FromSlot: Sized {
    const NATIVE_KIND: NativeKind;
    fn from_slot(bits: u64) -> Self;
}
impl FromSlot for i64    { const NATIVE_KIND: NativeKind = NativeKind::I64;  fn from_slot(bits: u64) -> Self { bits as i64 } }
impl FromSlot for f64    { const NATIVE_KIND: NativeKind = NativeKind::F64;  fn from_slot(bits: u64) -> Self { f64::from_bits(bits) } }
impl FromSlot for bool   { const NATIVE_KIND: NativeKind = NativeKind::Bool; fn from_slot(bits: u64) -> Self { bits != 0 } }
impl FromSlot for Arc<String>     { /* HeapValue::String pointer cast */ }
impl FromSlot for Arc<DataTable>  { /* HeapValue::DataTable pointer cast */ }
// …

pub trait TypedFn<Args, R>: Send + Sync + 'static {
    fn invoke(&self, slots: &[u64], ctx: &ModuleContext) -> Result<R, MarshalError>;
    fn arg_kinds() -> Vec<NativeKind>;
}
// blanket impl for Fn(P0) -> R, Fn(P0, P1) -> R, ..., Fn(P0..P7) -> R
// where each Pi: FromSlot, R: ToSlot.

pub fn register_typed_fn<F, Args, R>(
    module: &mut ModuleExports,
    name: &str,
    description: &str,
    param_names: &[&str],
    body: F,
) where F: TypedFn<Args, R>, R: ToSlot;
```
Param kinds derive from `Pi::NATIVE_KIND` at compile time. A body declared `fn parse_int(s: Arc<String>, base: i64) -> Result<i64, ParseError>` registers with arg kinds `[Ptr(HeapKind::String), I64]` automatically — the function's Rust argument types **are** the typed signature. A body declared `fn parse_int(s: Arc<String>, base: f64)` registered against `params: [string, int]` is a Rust trait-bound error at the `register_typed_fn` call site. No registration validator runs; the type system already did.

**Sub-mechanism C — wire/snapshot kind threading:**
```rust
pub fn slot_to_wire(bits: u64, kind: NativeKind, ctx: &Context) -> WireValue;
pub fn slot_to_serializable(bits: u64, kind: NativeKind, store: &SnapshotStore)
    -> SerializableVMValue;
pub fn slot_to_state_diff(bits: u64, kind: NativeKind, …) -> …;
```
Callers thread `kind` from the FunctionBlob's per-slot kind table (which already exists at compile time for typed-opcode emission). For heap kinds, `bits` is `Arc<HeapValue>` raw pointer; the per-`HeapValue` arms take over the dispatch.

---

**Why these three are one cut, not three:** the discriminator (`NativeKind`) is the same; the source of the discriminator (FunctionBlob's compile-time slot-kind metadata) is the same; the projection target differs only in the destination (typed VM slot vs. `WireValue` vs. `SerializableVMValue`). A single landing of `NativeKind` as the universal ABI-exit discriminator is the right granularity. Three separate landings would risk the discriminators drifting (one calling it `NativeKind`, another `SlotKind`, another `MarshalKind`) — the "two parallel discriminators" trap.

**Cost saved:** the trait-based arg side eliminates the entire `read_*` plumbing surface (~12 methods) of option β; eliminates the registration-validation runtime check; eliminates the macro infrastructure of option γ; and unifies the three boundaries into one mechanism (vs three near-identical implementations). Estimate: 5–8 days for full Phase 2b vs. ~3 weeks if each boundary is rebuilt independently with its own discriminator. Acknowledged immediate cost: every stdlib registration site rewrites from `|args, ctx| { let s = args[0].as_str()…; … }` to `|s: Arc<String>, ctx: &ModuleContext| -> Result<…> { … }` — verbose-once, structurally enforced thereafter.

**Calibration:** if the canary stdlib migration (chosen module: `random.rs`) does NOT drop the lib error count materially after marshal infra + one module's consumer migration, the diagnosis "most errors are downstream of the marshal layer" is wrong and we stop to surface before mass migration.

---

## 2026-05-06 — HeapKind trim + `NativeKind::Ptr(HeapKind)` extension

The wire/snapshot kind threading (Phase 2b sub-mechanism C) needs the
discriminator to express heap-pointer slots beyond the single
`NativeKind::String` variant. Today `NativeKind` has 24 variants — 23
scalar widths + `String`. It cannot express "this slot holds
`Arc<HeapValue>` whose discriminant is `DataTable`/`TypedArray`/`Instant`/etc."
The marshal layer (sub-mechanism A) hits the same gap when stdlib
functions return heap-allocated values. This entry covers:

- Trimming `HeapKind` to its surviving variants.
- Adding `NativeKind::Ptr(HeapKind)` as the unified heap-slot discriminator.

---

**Considered (option α, KEEP-AND-EXTEND):** keep `HeapKind` at its
77-variant size — including the 60 variants annotated `(removed)` or
`(deprecated)` — and add `NativeKind::Ptr(HeapKind)`. The extension
compiles cleanly without touching `HeapKind`.

**Rationalization (option α):** "The variant docstrings document
which are dead. The original `tags.rs` ABI-stability test (deleted
by the bulldozer) preserved ordinal positions; comments still imply
that contract. Trimming risks breaking some external consumer we
haven't audited."

**Pattern recognized (option α):** `NativeKind::Ptr(HeapKind::Some)`
would compile cleanly even though `Some` was deleted (option-C cut,
2026-05-06 entry). That's exactly the structurally-expressible-but-
forbidden state pattern that drove the `ConcreteReturn` /
`TypedReturn` split in commit `cd0479f` (and the `SlotKind` →
`NativeKind` rename in `381eff9`). Allowing dead variants to remain
expressible re-creates the same defection at a lower layer. The
"what if some external consumer" risk does not justify keeping
forbidden states reachable — audit, then trim.

**Considered (option β, PARALLEL TYPED-SUBSET):** introduce a smaller
`TypedHeapKind` enum in shape-value covering only the surviving
variants. `NativeKind::Ptr(TypedHeapKind)`. Original `HeapKind` keeps
its full 77-variant surface for the executor's runtime-tag-decode
paths.

**Rationalization (option β):** "Doesn't disturb existing HeapKind
consumers. Strict-typed boundary uses the typed subset; legacy paths
keep the full enum until they migrate."

**Pattern recognized (option β):** parallel-discriminator defection.
This is the same shape as the rejected "two NativeKind/SlotKind for
the marshal vs executor boundaries" — explicitly rejected in the
unified Phase 2b entry above. Two enums for the same domain
inevitably drift; the executor cascade work eventually has to map
between them, and the mapping itself becomes a dispatch.

---

**Alternative taken (option γ):** trim `HeapKind` to its 17 surviving
variants (one per surviving `HeapValue` arm), then add
`NativeKind::Ptr(HeapKind)`.

Trimmed `HeapKind` (renumbered sequentially, 0..16):
```
String        // 0
TypedObject   // 1
Closure       // 2  (matches HeapValue::ClosureRaw via the Closure ordinal)
Decimal       // 3
BigInt        // 4
DataTable     // 5
Future        // 6
TaskGroup     // 7
TypedArray    // 8
Temporal      // 9
TableView     // 10
Content       // 11
Instant       // 12
IoHandle      // 13
NativeScalar  // 14
NativeView    // 15
Char          // 16
```

The 60 deleted variants (`Array`, `HostClosure`, `TypedTable`/`RowView`/
`ColumnRef`/`IndexedTable`, `Range`, `Enum`, `Some`/`Ok`/`Err`,
`TraitObject`, `ExprProxy`/`FilterExpr`, the legacy temporal arms,
`TypeAnnotation`/`TypeAnnotatedValue`/`PrintResult`/`SimulationCall`/
`FunctionRef`/`DataReference`, `Number`/`Bool`/`None`/`Unit`/`Function`/
`ModuleFunction` (former ValueWord scalar discriminators), `HashMap`,
`SharedCell`, the typed-array width variants `IntArray`/`FloatArray`/
`BoolArray`/`Matrix`/`I8Array`..`U64Array`/`F32Array`/`FloatArraySlice`,
`Iterator`/`Generator`, `Mutex`/`Atomic`/`Lazy`/`Channel`,
`Set`/`Deque`/`PriorityQueue`, `ProjectedRef`, `Rare`/`Concurrency`)
are gone from the enum source. References to them (~10 sites in
shape-vm, all in code that's already broken from the bulldozer
cascade) become compile errors instead of compile-fine-but-
semantically-broken.

`NativeKind` extended:
```rust
pub enum NativeKind {
    Float64, NullableFloat64, Int8, ..., UIntSize, NullableUIntSize,
    Bool, String,
    Ptr(HeapKind),  // NEW
}
```

Wire/snapshot dispatch shape:
```rust
match kind {
    NativeKind::Int64        => WireValue::Integer(bits as i64),
    NativeKind::Float64      => WireValue::Number(f64::from_bits(bits)),
    NativeKind::Bool         => WireValue::Bool(bits != 0),
    NativeKind::String       => WireValue::String(arc_string_from(bits).to_string()),
    NativeKind::Ptr(hk)      => heap_slot_to_wire(bits, hk, ctx),
    /* … other scalar widths */
}
```

`heap_slot_to_wire(bits, hk, ctx)` casts `bits as *const HeapValue`,
debug-asserts `(*hv).kind() == hk`, then dispatches per HeapValue
arm. The `(kind == discriminant)` assert is sanity-only; production
dispatches on the precomputed `hk`, not on the heap object's
self-reported discriminant.

---

**Audit findings (ordinal-numbering risk surfaced before trimming):**

- `shape-wire/` has 0 HeapKind references. Wire format does not
  serialize HeapKind ordinals.
- Content-addressed bytecode hash includes instructions/strings/
  permissions; not HeapKind. Trim does not affect hash stability.
- `HeapHeader.kind: u16` is in-memory only; readers/writers share
  the same enum at runtime, so renumbering is safe.
- The `HEAP_KIND_V2_*` constants (80–84) live in
  `crates/shape-value/src/v2/heap_header.rs` and are a separate
  namespace from `HeapKind`. Unaffected by the trim.
- ~10 `HeapKind::X as u8` cast sites in shape-vm reference deleted
  variants — they are already broken from the bulldozer cascade
  (commits `7d6dc27` / `128cb8a`) and will be rewritten as part of
  the shape-vm reconstruction phase. Trim makes them
  compile-error-now rather than compile-fine-but-semantically-
  broken.

---

**TaskGroup / Future / inline-fit cases — surfaced before code:**

- `HeapValue::TaskGroup { kind: u8, task_ids: Vec<u64> }` is a struct
  variant with no corresponding `Arc<TaskGroup>` Rust type. For
  Phase 2b wire/snapshot READ side this is fine (cast bits to
  `*const HeapValue`, pattern-match TaskGroup arm). For Phase 2c
  marshal WRITE side, a stdlib body returning a TaskGroup-shape
  value will need a Rust struct + `From<TaskGroup> for HeapValue`
  adapter. **Not blocking Phase 2b.**
- `HeapValue::Future(u64)` is a u64 (FutureId), `HeapValue::Char(char)`
  is 4 bytes inline, `HeapValue::BigInt(i64)` is 8 bytes inline. They
  fit in a slot without heap allocation in principle. The current
  executor wraps them in `Arc<HeapValue>` anyway — Phase 2b matches
  the existing model rather than reshaping it. Inline-fit
  optimization is a Phase 3+ concern.

---

**Watchlist — the next defection attractor:**

When stdlib mass migration (Phase 2c) lands and bodies return
`Result<T, E>`, `Option<T>`, or `JsonValue`, the temptation will be
to add parametric NativeKind variants:
```
NativeKind::Result(ConcreteReturn, ConcreteReturn)  // FORBIDDEN
NativeKind::Option(ConcreteReturn)                  // FORBIDDEN
NativeKind::JsonValue                               // FORBIDDEN
```

That re-creates heterogeneous-by-default sum types at the discriminator
level — exactly the option-C cut for `HeapValue` reproduced one
layer up. The strict-typed answer is `HeapKind::TypedObject` plus a
`schema_id` per `runtime-v2-spec.md:180`: each `Result<T, E>` /
`Option<T>` / `JsonValue` instantiation gets its own monomorphized
`TypedObject` schema. The slot's `NativeKind::Ptr(HeapKind::TypedObject)`
plus the schema_id (from the function's registered `ConcreteType`)
fully determines the shape. No new NativeKind variants.

This is the same shape as the rejected `enum SlotValue { Int, Float,
Bool, Heap }` (option δ in the unified Phase 2b entry): heterogeneous
discriminator at the boundary, just at a different layer. Future
agents reading this should treat any "let's add `NativeKind::X` for
this parameterized return shape" reasoning as a structural defection
attempt and re-route to monomorphized `TypedObject` schemas.

---

**Cost saved:** option α ($keeping dead HeapKind variants) preserved
the structurally-expressible defection state for "audit later." The
prior plan's W-series cleanup is the cost of "audit later" extending
beyond the original scope. Trim cost: ~1 hour of source change + the
shape-vm cascade items already on the books. Estimated avoided
cleanup: 2–3 weeks of "we forgot HeapKind::Some isn't real anymore"
remediation across the next year.

---

## 2026-05-06 — Calibration finding #10 — stale-import count is not a cleanup-leverage proxy

This is **not** a defection. On-record calibration finding from Phase 2d
sub-cluster 1 (network_ops) follow-up audit, in service of the
predict-before-measure discipline.

**Considered (the bad heuristic):** if shape-runtime --lib has N errors
and ~M of them are "unresolved import shape_value::ValueWord /
ValueWordExt / ArgVec / value_word_drop / vw_clone / vmarray_from_vec
/ register_typed_function / ..." stale-symbol imports, then a fast
cleanup pass removing those imports should drop ~M errors and is a
high-leverage low-decision-cost workstream.

**Rationalization:** "imports are mechanical; just delete the unused
ones." Phase 2c handover predicted -9 to -12 for cluster #2 group 1
finish (network_ops migration) by extrapolating from file_ops's -15
result — but file_ops's drop came from deleting 1040 lines of
`wrap_legacy` bridging in `stdlib_io/mod.rs`, not from migrating bodies.
The stale-import view treated the remaining 96 errors as further
cleanup work of the same shape.

**Pattern recognized:** the W-series ValueWord removal was *deep*, not
boundary-only. Virtually every file with a stale ValueWord-family
import also USES that symbol structurally — in struct fields
(`closure.rs:39 pub value: ValueWord`), function signatures
(`type_methods.rs:69 fn get_value_type_name(value: &ValueWord)`),
return types (`plugins/data_source/providers.rs:256 -> Result<ValueWord>`),
enum variants (`module_loader/mod.rs:84 Value(ValueWord)`), or trait
methods (`plugins/data_source/mod.rs:153 -> Result<ValueWord>`).
Stale-import count predicts **architectural-cluster-membership**, not
cleanup work.

**Audit numbers:** of 47 error-bearing files, **0 (zero) were pure
stale-import cleanup candidates.** Predicted 5-10 (10-20%); measured 0
(0%). >100% miss in the predicted A category. Every error file is a
body-level structural dependency on a deleted symbol.

**Alternative taken:** stale-import count is now treated as a *cluster-
identity signal*, not a cleanup-leverage signal. The discipline:
before predicting that "the rest is cleanup," **audit each file's body
usage of the imported symbol**. If the body uses the symbol in a
signature/struct/variant, the file is blocked on the architectural
cluster that owns that symbol's replacement, not on cleanup. Bake
this into pre-cluster-execution work alongside Audit 1 + Audit 2.

**Cost saved:** prevented allocating a full session to "rapid stale-
import cleanup" that would have produced ~0 error drop and surfaced
the same architectural decisions one file at a time, in worse order.
The right next sub-cluster (B1 JsonValue, ~30 errors) was identified
by the audit instead.

---

## 2026-05-06 — Calibration finding #11 — scattered-cleanup buckets can be coherent multi-decision clusters

This is **not** a defection. On-record calibration finding from the
same Phase 2d audit. Companion to finding #10.

**Considered (the bad framing):** when cluster decomposition produces
a "misc cleanup" or "foundation cleanup" residual after the named
clusters are accounted for, treat the residual as miscellaneous —
files that need touch-ups but don't share architectural shape.

**Rationalization:** the named clusters (cluster #1 type_schema,
cluster #2 IoHandle, cluster #3 Array<T>, cluster #4 Option<T>,
cluster #5 JsonValue, intrinsics-dispatch-table) absorb the
"architecturally interesting" work; the rest is cleanup.

**Pattern recognized:** absence-of-naming is not absence-of-coherence.
The audit revealed a **23-file / ~38-error / 5-7-decision cluster**
that had been treated as foundation-cleanup spread across files.
Cross-cuts: closure capture storage (closure.rs), module-loader value
variants (module_loader/mod.rs), plugin-ABI return shape
(plugins/data_source/{mod,providers}.rs), event-queue payloads
(event_queue.rs), context dynamic values (context/{mod,variables}.rs,
const_eval.rs, annotation_context.rs), content builders/methods
(content_methods.rs, content_builders.rs), output adaptation
(output_adapter.rs), schema cache (schema_cache.rs), data-cache
serialization (data/cache.rs, data/load_query.rs, snapshot.rs
removed-fn references), module-export core (module_exports.rs,
module_bindings.rs), engine entry (engine/mod.rs), window manager
(window_manager.rs), type-introspection (type_methods.rs),
multiple-testing helpers (multiple_testing.rs), stdlib_time core
(stdlib_time.rs).

Each subsystem has its own architectural sub-decision: how to store
or marshal dynamic values without ValueWord. Closure captures need a
typed-slot storage decision. Module loader needs a typed-value enum.
Plugin ABI needs a typed return shape. Event payloads need typed
slots. None of these are "cleanup" — they're undone architectural
decisions clustered by *the runtime subsystem they live in*, which
the prior framings missed because none of the deferred clusters
(#1-#5 + intrinsics-dispatch) own the bridging surface.

**Combined insight (with finding #10):** architectural surface
analysis must precede ANY "cleanup" framing. Ten-plus calibration
findings in this work, all in the same direction: when analysis says
"the rest is mechanical/cleanup," treat that as a hypothesis
requiring consumer-body audit. Default expectation: **"this is
undone architectural decisions, not cleanup."**

**Alternative taken:** name the cluster on record as **B4 core-
foundation ValueWord-removal cluster**, with explicit sub-decision
shape (next entry). Document that the work decomposes across multiple
sessions with each sub-decision its own surface-decide-execute round
trip. Stop treating it as cleanup.

**Cost saved:** prevented a future session from picking up the
foundation work as a "misc errors, work through them" task — which
would have produced inconsistent per-subsystem decisions and
W-series-style scattered rationalizations. The cluster name lets
sub-decisions land coherently with watchlist discipline applied at
each step.

---

## 2026-05-06 — B4 core-foundation ValueWord-removal — named on-record cluster

On-record cluster naming. This is the largest single bucket in the
shape-runtime --lib error surface (audited 2026-05-06, Phase 2d sub-
cluster 1 follow-up): **~23 files, ~38 errors, ~5-7 sub-decisions**.

The cluster was previously implicit in the Phase 2a "shape-runtime
Phase-2 reconstruction: TypedReturn ValueWord hatches deleted" entry
and in Phase 2b's "stdlib mass migration ~80 errors mechanical"
miscalibration. Findings #10 + #11 give it explicit shape.

**Files in cluster (audit-grounded list):**

Foundation (core dynamic-value APIs):
- `closure.rs` — `Upvalue::Mutable(ValueWord)` storage shape (struct field)
- `module_loader/mod.rs` — `LoadedItem::Value(ValueWord)` enum variant
- `module_exports.rs` — core registry types still take `&[ValueWord]`
- `module_bindings.rs` — module-binding value carriers
- `event_queue.rs` — async event payload slots
- `context/{mod, variables}.rs` — dynamic context variable storage
- `const_eval.rs` — compile-time evaluation value model
- `annotation_context.rs` — annotation-evaluation value model
- `content_methods.rs` / `content_builders.rs` — content-block typed builders
- `output_adapter.rs` — print/output formatter
- `schema_cache.rs` — schema-cache value coercions
- `type_methods.rs` — type-introspection helpers (ValueWord ref API)

Cross-boundary (plugin/serialization/extension):
- `plugins/data_source/{mod, providers}.rs` — plugin ABI return shape
- `data/{cache, load_query}.rs` — data-cache serialize/deserialize
- `snapshot.rs` — snapshot serialize/deserialize references (warning
  only after #10 audit; included for completeness because adjacent
  decisions touch it)

Stdlib helpers (foundation-shaped, not parser-shaped):
- `stdlib_time.rs` — time module core (NOT in B1 JsonValue cluster)
- `multiple_testing.rs` — testing helpers
- `engine/mod.rs` — top-level engine entry
- `window_manager.rs` — window-manager value model
- `type_schema/registry.rs` — `shape_value::external_value::SchemaLookup` trait
  impl (1 error, distinct from `type_schema/mod.rs` which is cluster #1)

**Sub-decision shape (5-7 architectural decisions, each its own round-trip):**

1. **closure-captures** — `Upvalue::Mutable(ValueWord)` replacement.
   Options: typed slot per upvalue (per-NativeKind monomorphization);
   `HeapKind::Upvalue` with content header; or split into typed +
   dynamic-fallback paths. The dynamic-fallback option is the W-series
   defection-attractor — refuse on sight.
2. **module-loader-value** — `LoadedItem::Value(ValueWord)` replacement.
   Options: typed-value enum (`LoadedItem::Int(i64)`, `Float(f64)`, ...);
   `Arc<HeapValue>`-pointer; or restrict module-load values to a
   monomorphized shape.
3. **plugin-ABI-return** — `Result<shape_value::ValueWord>` in
   `plugins/data_source/{mod, providers}.rs`. Stable C ABI surface
   per shape-abi-v1, so this likely interlocks with plugin-ABI
   versioning. May warrant its own defection entry as an ABI break.
4. **event-payload** — `event_queue.rs` async-event slot storage.
   Options: typed event variants per producer; HeapKind::Event with
   schema; `Arc<HeapValue>` payload.
5. **snapshot-serialization** — replacement for deleted
   `nanboxed_to_serializable`/`serializable_to_nanboxed` per-slot-
   kind pair. The CLAUDE.md known-constraint comment notes "kind-
   threaded" replacement is intended; this sub-decision is the
   detail. data/{cache, load_query}.rs depend on this.
6. **content-builders-and-methods** — `content_builders.rs` /
   `content_methods.rs` — content-block construction at compile time.
   These build TypedObjects but use ValueWord intermediates;
   replacement is straightforward typed-builder surface.
7. **module-exports-core** — `module_exports.rs` registry types that
   still surface `&[ValueWord]`. The `register_test_function*`
   helpers (per `typed_module_exports.rs` doc comments) wrap legacy
   bodies into typed-passthrough; the question is whether to keep
   that legacy seam or close it.

**Watchlist (binding for all 5-7 sub-decisions):**

- Forbidden: re-introducing ValueWord under any rename
  (`SlotValue`/`DynamicValue`/`AnyValue`/etc.)
- Forbidden: parametric NativeKind variants for the cluster's payloads
- Forbidden: "split into typed + dynamic-fallback paths" — that is
  the W-series shape, just at a different layer
- Forbidden: sentinel values inline at any decision boundary

**Pacing expectation:** each sub-decision is its own surface-decide-
execute round trip. ~1 sub-decision per session is the conservative
estimate given Phase 2c/2d pacing (architectural cluster work is
decision-heavy, not code-heavy). Cluster total: 5-7 sessions to land
all sub-decisions.

**Sequencing within the cluster (suggestion, not binding):**

- Closure-captures and module-loader-value are smallest and most
  central; land first.
- Snapshot-serialization is largest scope (cross-cuts data caching);
  may want its own decomposition pass before any code change.
- Plugin-ABI-return likely needs to interlock with shape-abi-v1
  versioning policy and could be a separate ABI workstream.

**Cost saved (anticipatory):** the named cluster prevents per-file
cleanup commits from making inconsistent sub-decisions. Estimate:
2-4 weeks of "this file does X, this other file does Y, now we have
two ValueWord-replacement shapes" reconciliation work avoided across
the 5-7 sub-decisions, by forcing each to be a single surfaced choice
applied uniformly.

---

## 2026-05-07 — Calibration finding #12 — clusters form a DAG; leaf-first is the right priority heuristic

This is **not** a defection. On-record calibration finding from
Phase 2d sub-cluster B1 surface-and-defer.

**Considered (the bad heuristic):** when multiple deferred clusters
remain, prioritize by "highest leverage per architectural decision"
— measured as errors-dropped per single decision committed. Pick the
cluster that knocks out the most errors in one round-trip.

**Rationalization:** "knock out the biggest pile first" is the
intuitive answer when scope is decision-density-bound. Phase 2d's
recommendation framing (B1 picked for "highest error-drop per
decision") used this heuristic.

**Pattern recognized:** clusters are **not** independent leaf
candidates. They form a DAG with interlock relationships. The
"highest leverage" heuristic is incorrect because it ignores the
direction of dependency edges. A high-error-drop cluster that is
an INTERIOR node — depending on other open clusters — cannot be
landed in a single decision. Forcing it through the unresolved
interlocks is the W-series-shape risk: each forced sub-decision
under unsettled dependencies tempts a "decide later, commit a
provisional shape now" rationalization. That rationalization is
the defection-attractor.

**Audit grounding:** B1 was framed as a single-decision cluster.
Audit 2 (marshal-API surface) revealed it has 5 sub-decisions, of
which sub-decision #1 (`TypedArrayData::String`/`HeapValue`)
interlocks with Phase 2d Array (the leaf cluster owning that exact
decision) and sub-decision #4 (Shape-side enum visibility)
interlocks with Cluster #4 Option<T> (a leaf cluster owning the
prelude-vs-import question for sum-types broadly). Trying to land
B1 in one session would have required taking provisional positions
on those two sub-decisions before their owning clusters resolved
them — exactly the W-series shape.

**Cluster DAG (current state, audit-grounded):**

- **Phase 2d Array cluster** (TypedArrayData::String /
  TypedArrayData::HeapValue extension; per Phase 2d sub-cluster #3
  process_ops Array<string> entry + this entry's sub-decision #1)
  — **LEAF.** No open dependencies. Lands independently.
  Multi-cluster unblock when it lands: process_ops Array<string>
  consumers, csv_module rows, arrow_module Array<DataTable>,
  JsonValue::Array projection, plus Phase 2d sub-cluster #4 path
  utilities `io.join` varargs (related but separable).
- **Cluster #4 Option<T> + SomeObjectPairs / Shape-side sum-type
  language-feature representation** — **LEAF.** No open
  dependencies. Owns the prelude-vs-import question for sum-types
  generally. When it lands, B1 sub-decision #4 falls out for free.
- **B1 JsonValue cluster** — **INTERIOR.** Depends on Phase 2d
  Array (sub-decision #1) and Cluster #4 Option (sub-decision #4).
  After both leaves land, B1 residual is sub-decisions #2 (Object
  runtime shape), #3 (registration strategy), #5 (recursive
  projection). At that point B1 becomes a leaf and lands.
- **Intrinsics-dispatch-table cluster** (handover-named) —
  **probable LEAF** (depends on the IntrinsicFn calling-convention
  decision; no obvious cross-cluster interlock). Audit recommended
  before declaring leaf-status definitively.
- **B4 core-foundation ValueWord-removal cluster** (named
  2026-05-06 in this log) — **ROOT (likely).** 5-7 sub-decisions
  spanning closure-captures, module-loader-value, plugin-ABI-return,
  event-payload, snapshot-serialization, content-builder/method,
  module-exports-core. Several sub-decisions likely depend on
  Phase 2d Array (TypedArrayData shape) and possibly Cluster #4
  Option (Some-payload representation in
  TypedReturn::SomeObjectPairs). Each sub-decision needs its own
  DAG audit before scheduling.
- **Cluster #1 type_schema** — **deferred to shape-vm cascade
  boundary** (2026-05-06 entry). Cross-crate; not a same-crate
  decision. Last in priority order regardless of leaf/interior
  classification.

**Right priority heuristic: leaf-first, then interior nodes as
their dependencies resolve.** The cluster-DAG (not cluster-list) is
the actual structure. Multi-cluster unblock at leaf nodes is the
high-leverage outcome — *not* high error-drop at an interior node
that can't actually be committed without dependency resolution.

**Reordered next-session priority (binding):**

1. **Phase 2d Array cluster** — leaf, multi-cluster unblock.
2. **Cluster #4 Option<T>** — leaf, dual-cluster interlock
   resolution (Cluster #4 itself + B1 sub-decision #4).
3. **B1 JsonValue residual** — interior node now eligible; lands
   sub-decisions #2/#3/#5 + 5 parser-module migrations.
4. **B4 core-foundation cluster** — root; audit DAG before each
   sub-decision execution. Several sub-decisions likely become
   eligible after (1) and (2) land.
5. **Intrinsics-dispatch-table cluster** — leaf-or-near-leaf; can
   parallel-track with (4) once audited.
6. **Cluster #1 type_schema** — deferred-to-shape-vm-boundary;
   lands at the cross-crate cascade.

**Watchlist (binding addition):** before predicting scope or
ordering work for any cluster, **audit cluster dependencies**.
Asking "what does this cluster depend on" is now a binding
pre-cluster-execution check alongside Audit 1 (consumer-call-shape)
and Audit 2 (marshal-API surface).

**Affirm: surface-on-calibration-mismatch is binding pre-work.**
The discipline that produced findings #5, #9, #10, #11, #12 — the
"if you start a 'mechanical migration' task and discover within 30
minutes that it's actually a coupled cluster, surface and write an
on-record deferral entry" rule from the surface-or-proceed list —
is **not occasional defensive practice**. It is the now-baseline
pre-work for any cluster scope or execution decision. Twelve
calibration findings in this work, all in the same direction,
makes the pattern unambiguous: the default expectation when a task
is framed as "mechanical/cleanup/single-decision" is that audit
will reveal an unsettled architectural surface, and the right
response is to surface and re-decompose before executing. Past
sessions' calibration discipline has improved the predict-vs-
measure success rate from 1-of-8 (Phase 2c) to multiple in-window
predictions (Phase 2d sub-cluster 1; B1 audit identifying
interlock instead of executing prematurely). Maintain.

**Cost saved:** prevented forcing 5 sub-decisions through one
B1 round-trip — which would have either committed provisional
positions on sub-decisions #1 and #4 (locking out the leaf
clusters from clean independent landing) or rationalized half-
landed shapes ("we'll fix the marshal extension later") in the W-
series defection pattern. Estimated avoided cleanup: 1-2 weeks of
"untangle B1's interim shape from Phase 2d Array's eventual
shape" rework across the next year. Plus the meta-cost of
cluster-DAG awareness now being explicit in the log, which the
next session can use to schedule (1)+(2) without re-deriving the
ordering.

---

(Add new entries above this line. Newest first.)
