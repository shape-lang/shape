# W17 Deep-Restore — STAGE Verify Status

**Date:** 2026-06-02
**Worktree:** `shape-strict-flip-collection-dispatch` (cumulative strict-flip)
**Verified HEAD:** `27badee3` — `feat(STAGE Snapshot-roundtrip): W17 deep-restore`
**Verifier disposition:** GREEN — all gates pass; core W17 DoD (non-empty deep
restore) is run-proven.

This is a verification artifact, not a fix. It records the gate results and the
keystone-arm landed/surfaced ledger for the K2 → K1 → snapshot stack at
`27badee3`, plus the remaining gap to full book-accurate deep-restore.

---

## 1. Gate results (all run-verified at `27badee3`)

| Gate | Command | Result |
|------|---------|--------|
| Workspace-clean | `just check-clean` | **EXIT 0** |
| Defection guard | `scripts/check-no-dynamic.sh` | **EXIT 0** (incl. Track-A `unwrap_or((0, NativeKind::Bool))` baseline row = 0, held) |
| no_dynamic sentinel | `cargo test -p shape-vm no_dynamic` | 1/1 |
| executor::snapshot | `cargo test -p shape-vm executor::snapshot::` | **25/25** |
| executor::resume | `cargo test -p shape-vm executor::resume::` | **9/9** |
| runtime snapshot | `cargo test -p shape-runtime snapshot::` | 2/2 |
| K1 module-return | `cargo test -p shape-vm vm_impl::modules` | **14/14** (stage_k1_tests) |
| K2 marshal | `cargo test -p shape-runtime marshal` | **5/5** (heap_value_vec_marshal_tests) |
| numeric_conversions | `--test numeric_conversions -- --test-threads=1` | **104/0** (preserved) |
| smoke VM==JIT | s1–s5 (+ s2-oneliner) release-binary `--mode vm` vs `--mode jit` | **5/5 converge** (s1=4950, s2=30, s2-oneliner=30, s3=x, s4=2, s5=x; ec=0 both modes) |

No new surface hit introduced. No forbidden pattern present (`ValueWord`,
Bool-default slot fabrication, synthesis/marshal/serialization shim,
parallel sum-type projecting 1:1 to HeapValue — all absent).

---

## 2. Keystone arm ledger — LANDED vs SURFACED

### LANDED (typed-Arc-direct, run-verified round-trip identity)

**K2 — `Vec<Arc<HeapValue>>` per-element-T marshal** (`crates/shape-runtime/src/marshal.rs:480`, both `FromSlot` + `ToSlot`)
- Reads the `stamp_elem_type` discriminant (HeapHeader offset 7) via the public
  `TypedArray::read_elem_type` accessor — existing discriminator, no side-channel,
  no kind fabrication.
- Per-element dispatch into the canonical `Arc<HeapValue>` arm (ADR-005 §1):
  `ELEM_TYPE_CHAR` → `HeapValue::Char`; `ELEM_TYPE_STRING` → `HeapValue::String`
  (owns-clone); `ELEM_TYPE_DECIMAL` → `HeapValue::Decimal` (owns-clone);
  `ELEM_TYPE_TYPED_OBJECT` → `HeapValue::TypedObject` (per-element `v2_retain`).
- Scalar-stamped / unstamped arrays SURFACE (precise panic — the established
  `from_slot` surface mechanism), never shim.

**K1 — `project_typed_return` container/wrapper + concrete leaf arms** (`crates/shape-vm/src/executor/vm_impl/modules.rs`)
- Leaf (`project_concrete_return`): I64/F64/Bool/Unit/String/OpaqueTypedObject/
  IoHandle/DataTable/ArrayI64/ArrayF64/Bytes/ArrayString/ArrayHeapValue
  (routed through K2)/HashMapStringString — typed-Arc carriers direct.
- Wrappers: Concrete / Ok / Err / Some / None (ResultData/OptionData), plus
  ObjectPairs / TypedObject / SomeObjectPairs / OkObjectPairs / ErrObjectPairs
  via `typed_object_from_pairs` (TypedObjectStorage builder).
- Retired the inner `ConcreteReturn` catch-all + the outer `TypedReturn`
  container catch-all NotImplemented stubs.

**Snapshot container arms** (`crates/shape-runtime/src/snapshot.rs` —
`slot_to_serializable` + `serializable_to_slot`)
- `TypedObject` — schema_id + per-field slots via field_kinds track, recursive;
  restore rebuilds via the v2-raw `_new` carrier (matches `release_elem` +
  carrier-side `_drop` allocator pair — avoids the `length_typed_object_empty`
  SIGABRT class).
- `TypedArray` — scalar element kinds project to `SV::Array`; restore rebuilds
  the monomorphic `TypedArray<T>` with matching ELEM_TYPE stamp.
- `HashMap` — **K1 string→string only** (`HashMapKindedRef::String`).
- `Range` — i64 start/end/inclusive via `RangeData`.

### SURFACED (clean — `VMError::NotImplemented` / `Err(String)`, precise message, no shim)

- **`HashMapStringHeapValue` = K3** — the polymorphic-value HashMap. Needs the
  ADR-006 `HashMapData` kinded-value-track amendment (a parallel `Vec<NativeKind>`
  over values) before it can carry `Arc<HeapValue>` payloads without a
  Bool-default kind. Surfaces in BOTH the K1 module-return projector
  (`project_concrete_return`) and the snapshot `slot_to_serializable` HashMap arm.
  Proven-clean by `hashmap_string_heap_value_surfaces_clean_k3`. **Pending ADR.**
- **`JsonValue`** (module-return) — needs the runtime `Json` enum-construction
  subsystem (schema-registry-backed enum construction + recursive descent).
- **`TypedReturn::ArrayObjectPairs`** (array of typed objects) — needs the
  typed-object-array element-construction path that pairs with the K2 producer.
- **Non-empty VmState `frames`** (resume path, `decode_vmstate_frames`) — the
  read-only `FrameState` introspection schema carries `{ function_name, blob_hash,
  ip, locals, args, upvalues }` and CANNOT supply the `return_ip` / `locals_base`
  / `locals_count` that `SerializableCallFrame` requires. SURFACE rather than
  fabricate offsets (W17-snapshot-resume-frames-schema follow-up).
- **Resume IP** (VmState path) — `VmState` schema carries no resume-IP field; IP
  stays 0 (W17-snapshot-resume-ip follow-up).
- Pre-existing complex shapes (DataTable/TableView/Temporal/TaskGroup/IoHandle/
  NativeView/NativeScalar/Content/ClosureRaw) — own multi-step landing paths.

No NEW surface beyond the pre-disposed K3 / Json / ArrayObjectPairs / frames-schema
/ resume-IP set.

---

## 3. Core DoD — does whole-VM DEEP restore now produce a NON-EMPTY call_stack + module_bindings?

**YES — via the VM-native snapshot path. Proving test:**
`crates/shape-vm/src/executor/snapshot.rs::test_snapshot_roundtrip_multiframe_bindings_nonempty`

The test builds a live VM with TWO module bindings (`Int(99)`, `String("world")`)
and a TWO-frame call stack, then `vm.snapshot()` → `VirtualMachine::from_snapshot()`
→ re-snapshot, asserting:
- `restored.call_stack.len() == 2` (NON-EMPTY) with correct `return_ip` (5),
  `base_pointer` (0/2), `locals_count` (2).
- `re.module_bindings.len() >= 2` with `Int(99)` + `String("world")` round-tripped.

This is round-trip identity, not a structural-envelope stub. The earlier
landing-scope test (`apply_pending_resume_vmstate_typed_object_restores_end_to_end`,
resume.rs:832) still asserts empty restore — but that exercises the *resume-via-
VmState-introspection* path, whose `frames`/`module_bindings` fields are
`FieldType::Any` opaque at that scope. The NON-EMPTY deep restore is proven on
the native `snapshot()`/`from_snapshot()` carrier, which serializes
`SerializableCallFrame` directly (so it has the structural fields the
introspection `FrameState` schema lacks).

The container/value arms additionally round-trip as live stack slots, proven by
`test_snapshot_roundtrip_container_arms` (Range / TypedArray<i64> /
HashMap<string,string> / TypedObject — serialize + restore, no SIGABRT).

---

## 4. Remaining gap to full book-accurate deep-restore

1. **K3 — `HashMap` polymorphic value monomorphizations.** Blocked on the
   ADR-006 `HashMapData` kinded-value-track amendment (parallel `Vec<NativeKind>`
   over values). Until then, only `HashMap<string,string>` round-trips; every
   other value type surfaces clean. **This is the one keystone arm SURFACED
   pending ADR.**
2. **Resume-via-VmState non-empty `frames`.** The read-only `FrameState`
   introspection schema must grow `return_ip` / `locals_base` / `locals_count`
   (or `state.capture_all` must emit `SerializableCallFrame` directly) before a
   non-empty frames array round-trips through the *resume* path. The native
   `snapshot()` path already round-trips multi-frame call stacks (proven, §3).
3. **Resume-IP.** The `VmState` schema carries no resume-IP field; the
   resume-path IP stays 0 (no fabricated IP). Native-snapshot IP relocation is
   covered separately (`test_snapshot_ip_relocation_fields_present`).
4. **Loop / timeframe / exception stacks.** Not exercised by the current
   proving tests; status as deep-restore targets is unconfirmed at this scope —
   tracked as the residual W17 deep-restore tail beyond the call_stack +
   module_bindings core DoD.

---

## Verdict

Core W17 DoD met: whole-VM deep restore produces a non-empty call_stack AND
module_bindings, run-proven. K2 + K1 + snapshot container arms LANDED
typed-Arc-direct. K3 (HashMapStringHeapValue) is the sole keystone arm SURFACED
pending an ADR amendment. All gates green; numeric_conversions 104/0 and smoke
5/5 VM==JIT preserved.
