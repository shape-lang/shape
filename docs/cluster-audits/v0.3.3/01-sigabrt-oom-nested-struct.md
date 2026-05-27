# v0.3.3 fix-dispatch cluster #1 — SIGABRT 130-137 TB OOM on nested-struct field access

**HEAD at audit:** `7877fc6b` (post-v0.3.2 audit-doc refresh).
**Status:** AUDIT-ONLY (no source / fixture changes; no commits; no worktree per audit-day exception).
**Sub-cluster name:** `R8-W9-G.3-A` — `sigabrt-oom-nested-struct-string-final-expr`.
**Size estimate:** **S** (single root cause; bounded to the `slot_to_wire` String/StringV2 arm + the producer-side kind-stamp on the program's terminal expression).
**Source taxonomy entries:**
- `structs_types.md` — `structs::struct_nested_string_field` (FN-REG-CORRECTNESS; ~136 TB).
- `complex_integration.md` Class M (×2): `test_complex_deep_nested_struct_access` (`timeout: monitored command dumped core`), `test_complex_nested_typed_objects` (~138 TB).
- `regression.md` — `qa::regression_crit_1_nested_property_access` (~135 TB; binary-killing).

## 1. Minimal repro (4 lines, reproducible at `./target/release/shape run`)

```shape
type Server { host: string, port: int }
type Config { server: Server, debug: bool }
let cfg = Config { server: Server { host: "localhost", port: 8080 }, debug: false }
cfg.server.host
```

`./target/release/shape run /tmp/repro_struct_nested_string_field.shape` (binary built 2026-05-26 at canonical HEAD) **reproduces the SIGABRT-OOM in isolation** under release build:

```
[jit-fallback] function main failed JIT compile: Runtime error: JIT-FFI return path:
RETURN_TAG_NANBOXED reached the host boundary without a stamped NativeKind
(raw_bits=0x648c47f708f0). Per ADR-006 §2.7.5 / §2.7.5.1 the return tag must be
a typed variant; this is a kind-source gap (W10 jit-playbook §5 surface-and-stop).
See executor.rs:267 comment.; running under interpreter
memory allocation of 110553680820064 bytes failed
... timeout: the monitored command dumped core
```

Allocation size varies between runs (`136617682127488`, `110553680820064`, etc.) because the offending bits are an address. All in the 100-137 TB range — i.e. the high 16 bits of a 64-bit userspace pointer (`0x0000_5xxx_xxxx_xxxx` × 8) interpreted as a `usize` length.

### Reduction lattice (run-binding evidence)

| Variant | Final expr | Result | File |
|---|---|---|---|
| Nested 2-level, string-field as **terminal expr** | `cfg.server.host` | **SIGABRT 110 TB** | `/tmp/repro_struct_nested_string_field.shape` |
| Nested 2-level, string-field as `print(...)` arg | `print(cfg.server.host)` | `localhost` (OK) | `/tmp/repro_v2_print_only_no_expr.shape` |
| Nested 2-level, **int**-field as terminal expr | `cfg.server.port` | `{"Integer": 8080}` (OK) | `/tmp/repro_v3_int_only.shape` |
| Single-level, string-field as terminal expr | `s.host` | `{"String":"localhost"}` (OK) | `/tmp/repro_v4_one_level.shape` |
| Nested 3-level, three `print(...)`s | Person/Address `print(p.addr.city)` | `Bob/LA/90001` (OK) | `/tmp/repro_nested_typed_objects.shape` |
| Deep-3-level Outer/Mid/Inner, three `print(...)`s | `print(o.mid.inner.val)` | `42/deep/1` (OK) | `/tmp/repro_deep_nested.shape` |

**Signature:** the OOM triggers when (a) the receiver chain is two or more TypedObject hops deep AND (b) the program's **terminal expression** (no `print()` wrapper) projects a `string`-typed nested field. Inner `int` fields don't OOM. Single-level string fields don't OOM. `print()` wrapper doesn't OOM (because `print()` re-renders through the VM-internal printer; only the top-level program-completion `slot_to_wire` path hits the bug). Note: `regression.md`'s sibling fixture `regression_crit_1_nested_property_access` uses `print(cfg.server.host)` AND still OOMs — so the trigger is broader than "terminal expr only"; under `cargo test` invocation the same bytecode path is also reached from the `expect_output_contains` host-boundary capture. Confirmed by `regression.md`'s 135 TB allocation under serial `--test-threads=1`.

## 2. Root cause hypothesis (backtrace-anchored)

`RUST_BACKTRACE=full` on the repro lands the OOM at `slot_to_wire`:

```
10:  <alloc::string::String as core::clone::Clone>::clone
11:  shape_runtime::wire_conversion::slot_to_wire
12:  shape_runtime::wire_conversion::slot_to_envelope
13:  shape_vm::execution::ProgramExecutor::execute_program
14:  shape_jit::executor::JITExecutor::execute_program
```

In `slot_to_wire` (`crates/shape-runtime/src/wire_conversion.rs:98-104`), the `NativeKind::String` arm does:

```rust
NativeKind::String => {
    // bits is an Arc<String> raw pointer
    let ptr = bits as *const String;
    // SAFETY: kind contract pins this slot to an Arc<String> raw ptr.
    let s = unsafe { &*ptr };
    WireValue::String(s.clone())
}
```

`String::clone` reads the source `String`'s `(ptr, len, cap)` triple at offsets 0/8/16 of `*ptr` and `alloc::raw_vec::handle_error` (stack frame 9) fires when the `len`/`cap` field — read from the wrong memory — appears as ~10¹³-bit count. **The slot's `bits` are NOT an `Arc<String>` raw pointer despite the kind label `NativeKind::String`.**

This is a **producer-side kind-stamp drift**: the program's top-of-stack KindedSlot has been stamped `NativeKind::String` from the schema's declared field-type (`FieldType::String` → `FIELD_TAG_STRING` → `NativeKind::String` per `typed_object_ops.rs:169`), but the slot's `bits` carry a different shape (the high address-pattern `0x5xxx_xxxx_xxxx` strongly suggests it's a raw `*const TypedObjectStorage` from a v2-raw `_new` allocation in `object_creation.rs:214` — i.e. the **inner TypedObject pointer is being read as if it were the host String's Arc pointer**).

Three candidate kind-drift sites on the read path through `op_get_field_typed` (`crates/shape-vm/src/executor/typed_object_ops.rs:355-624`):

a. **Hot path (schema match)** at line 596-624: `push_field_value(&storage.slots[field_index], is_heap, *field_type_tag)` sources the kind from the **operand-encoded `field_type_tag`** via `field_tag_to_heap_native_kind` (`typed_object_ops.rs:204-211`), NOT from `storage.field_kinds[field_index]`. If the operand's `field_type_tag` and `storage.field_kinds[idx]` disagree — for example because the W17.3-4 per-container FieldType migration left the schema with `FieldType::Object("Server")` (→ tag OBJECT) while the storage's `field_kinds[idx]` carries `Ptr(TypedObject)`, but the *outer* read uses the *inner* schema's per-field tag, OR the compiler emits the inner-schema's slot-0 (`host: string`) tag against the *outer* receiver — the producer stamps `NativeKind::String` against bits that are the inner TypedObject pointer.

b. **IC fast path** at line 452-471: identical to (a) — kind sourced from `hit.field_type_tag`, not the storage's parallel `field_kinds` track.

c. **Megamorphic / name-based fallback** at line 506+: source from `source.field_by_index(src_field_idx).field_type` then mapped via `field_type_to_tag`. Same drift if the wrong schema is looked up.

**Specifically, the bug is most likely:** the compiler emits a **two-instruction** field-projection chain for `cfg.server.host` — `GetFieldTyped(Config, idx=0, FIELD_TAG_OBJECT)` then `GetFieldTyped(Server, idx=0, FIELD_TAG_STRING)`. If between the two opcodes the intermediate value is **NOT** correctly pushed as `Ptr(HeapKind::TypedObject)` with the inner Server pointer (i.e. if the host-bits AND the kind-label drift — e.g. the bits are the inner Server pointer but the kind label is propagated from the outer Config schema's slot-0 declared field-type, which would be `FieldType::Object("Server")`), the second opcode reads the wrong receiver and ultimately pushes Server's storage pointer with kind=String. The `NativeKind::String` arm of `slot_to_wire` then dereferences as `*const String` → 110 TB malloc.

Either the producer-side kind-stamp is wrong on the intermediate, OR the second `GetFieldTyped` is reading from the wrong storage offset (treating the Server schema's `slots[0]` as if it were already an Arc<String> but it's actually a raw `*const TypedObjectStorage`-shaped bits that came from the v2-raw `_new` allocator at `object_creation.rs:214`).

**Producer-of-drift candidates** (file:line citations below):
- `crates/shape-vm/src/executor/typed_object_ops.rs:238-283` — `push_field_value` sources kind from operand-encoded `field_type_tag`. For heap-backed slots with `FIELD_TAG_OBJECT` it pushes `Ptr(HeapKind::TypedObject)`; for `FIELD_TAG_STRING` it pushes `NativeKind::String`. **No cross-check against `storage.field_kinds[idx]`** — operand label is trusted unconditionally.
- `crates/shape-vm/src/executor/typed_object_ops.rs:447-449` — `schema_id != *type_id` falls through to name-based lookup that can re-resolve the type-tag from the **source** schema. If the operand's `type_id` is the inner schema (Server) but the receiver is somehow the outer schema (Config), the lookup mis-routes.
- `crates/shape-vm/src/executor/objects/object_creation.rs:214-221` — `TypedObjectStorage::_new` (v2-raw) writes the receiver as `ptr as u64` and stamps `NativeKind::Ptr(HeapKind::TypedObject)`. The `field_kinds` parallel track is set per `kinded_to_slot`'s `resolved_kind` (line 207), which for `is_heap=true` returns the popped `kind` verbatim (line 497). For nested struct construction the popped kind for an inner TypedObject is `Ptr(HeapKind::TypedObject)` — correct. So the storage side **looks right**; the drift is on the read side.

## 3. Bisect commit anchor (deferred — narrowed search-space)

**Bisect deferred — needs dedicated session.** A full `git bisect` between `v0.2.0` and `v0.3.2` (substance HEAD `82f049dd`) on this repro is multi-hour work because each step requires a release rebuild (`cargo build --release` on a workspace this large is 4-7 minutes per bisect step × ~12 steps = 50-80 min minimum). Search-space narrowed below.

### Candidate bisect anchors (most-recent-first; ordered by likelihood)

1. **`b101b5ec` — `W17-typed-object-mutation: kinded field write paths (close)`** (~2026-05-11). Touched `crates/shape-vm/src/executor/typed_object_ops.rs` + sibling files; introduced the §2.7.13 `DerefStore` kind-invariance assertion that cluster #2 trips in the SAME 4-fixture file (`struct_field_mutation`). Likely candidate for the kind-stamp-on-read regression.
2. **`0214f107` — `Phase 3 cluster-0+1 Wave 2 Round 4 D4 ckpt-1: 11 TypedObjectStorage producer-site migrations Arc::new → _new`** (~2026-05-13). Switched `op_new_typed_object` to v2-raw `_new` allocator (no Arc wrapper). The receiver pointer layout changed — old `Arc::from_raw` recovery is now wrong-type recovery (the typed_object_ops.rs:389-410 comment names this exact failure mode for the WRITE path). The READ path may not have fully audited the layout switch — specifically `push_field_value` (line 280) reads `slot.raw()` and trusts the operand-encoded tag, which is correct for v2-raw, but the `kinded_to_slot` round-trip during nested construction may be reading the outer-storage layout incorrectly.
3. **`abec57d0` — `W17.3-4.3 runtime dispatch + snapshot/wire for per-container FieldType variants`** (~2026-05-22). Extended `FieldType` with per-container variants (HashMap/Set). Less likely (touches container-typed fields, not `FieldType::Object` arms), but the schema-table runtime-dispatch surface was rebuilt across the same files.
4. **`a287c795` — `R5c-2-β1 RefTarget::TypedField double-free — migrate receiver to v2-raw TypedObjectPtr carrier`** (~2026-05-20). Switched references-to-TypedObject-field carriers to v2-raw. Plausibly correlated; the BUG comment in the source above `struct_nested_string_field` (`structs.rs:111` — "nested typed struct field access (cfg.server.host) returns the inner object instead of the field") suggests the latent bug existed BEFORE the v2-raw migration but didn't OOM — only after the layout change did the kind-drift land in the `Arc<String>::clone` deref-as-len trap.

### Bisect search-space binding (for the dedicated session)

- **Good:** `v0.2.0` (pre-strict-typing / pre-v2-raw migration; OOM did not exist per pre-existing test passing).
- **Bad:** `82f049dd` (HEAD at audit start; OOM reproduces).
- **Suspect-window narrow:** `2026-05-11` (W17-typed-object-mutation) → `2026-05-22` (W17.3-4.3). 6-8 bisect steps.
- **Probe binary:** `cargo build --release --bin shape` then `./target/release/shape run /tmp/repro_struct_nested_string_field.shape` (exit 0 + "localhost" output = GOOD; SIGABRT 100+ TB malloc-failed = BAD).
- **Bisect-skip filter:** commits touching ONLY docs / non-vm crates can be skipped.

## 4. Affected subsystem — file:line citations

- **Crash site:** `crates/shape-runtime/src/wire_conversion.rs:98-104` — `slot_to_wire` `NativeKind::String` arm; trusts `bits as *const String` unconditionally; reads `(len, cap)` from misaligned/wrong memory; `String::clone` allocates `cap` bytes → OOM.
- **Sibling crash sites (same class):** `wire_conversion.rs:113-124` (`NativeKind::StringV2` reads StringObj at offset 16 — same shape under StringV2 carrier); `wire_conversion.rs:130-140` (`NativeKind::DecimalV2` — would crash with the same drift on a `decimal` field).
- **Producer site (most likely drift origin):** `crates/shape-vm/src/executor/typed_object_ops.rs:238-283` — `push_field_value` sources kind from operand-encoded `field_type_tag` with no cross-check against `storage.field_kinds[idx]`. The `FIELD_TAG_ANY` arm at line 458-471 / 608-615 DOES cross-check; the typed-tag fast paths do NOT.
- **Producer site (secondary):** `crates/shape-vm/src/executor/typed_object_ops.rs:447-471` — IC hot path same dispatch shape.
- **Receiver-recovery sound site (NOT the bug, but proves the v2-raw layout switch):** `typed_object_ops.rs:389-435` — verbose comment names the v2-raw layout switch and `ReceiverGuard` RAII pattern for safe drop. **No equivalent kind cross-check exists on the READ side for inner field push.**
- **Top-level program-completion projection:** `crates/shape-vm/src/execution.rs:563-566` — pulls `(bits, kind)` off the KindedSlot from `vm.execute()` and feeds `slot_to_envelope(bits, kind, "", ctx)` → enters `slot_to_wire` on the String arm.

## 5. Sub-cluster classification + size

| Field | Value |
|---|---|
| Sub-cluster name | `R8-W9-G.3-A: sigabrt-oom-nested-struct-string-final-expr` |
| Size estimate | **S** (single root cause + 4 affected tests + bounded fix surface: cross-check `storage.field_kinds[idx]` against operand-encoded `field_type_tag` in `push_field_value` + IC hot path; surface-and-stop on drift) |
| Release-gating? | **YES** — SIGABRT-class memory corruption per TAXONOMY ("silent-wrong-output; SIGABRT / SEGFAULT") + user 2026-05-27 binding "correctness is key" |
| Affected tests | 4: `structs_types::structs::struct_nested_string_field`, `complex_integration::test_complex_deep_nested_struct_access`, `complex_integration::test_complex_nested_typed_objects`, `regression::qa::regression_crit_1_nested_property_access` |
| Fix scope | Add producer-side kind-drift check at `push_field_value` (lines 244-282) and the IC hot path (lines 460-471) of `typed_object_ops.rs` — when `is_heap=true`, cross-check `storage.field_kinds[idx]` against `field_tag_to_heap_native_kind(field_type_tag)`. On mismatch: surface-and-stop with `VMError::RuntimeError("op_get_field_typed kind drift: operand tag {} (kind {:?}) disagrees with storage.field_kinds[{}] ({:?})", ...)`. Producer-side kind correctness is the canonical §2.7.5 invariant; reading the wrong kind into a typed slot is exactly the class CLAUDE.md §Forbidden Patterns names. |

## 6. Dependencies on other v0.3.3 fix-dispatch clusters

**Same kind-source-drift family as the following clusters; share root-cause neighborhood but DISTINCT trigger paths — fixes can land independently.**

- **Cluster #2 — ADR-006 §2.7.13 `DerefStore kind drift: popped Int64, place Float64`** (per structs_types audit). Same `crates/shape-vm/src/executor/variables/mod.rs:2718` invariant fires on `struct_field_mutation` / `struct_field_mutation_second_field` (the immediately-adjacent fixtures in `structs_types/structs.rs:127-148`). **Different mechanism:** §2.7.13 fires on the WRITE path (constructor-side `Int64` literal not widened to declared `Float64` field-type). The READ-path bug here (#1) is the DUAL: the read trusts the operand-tag rather than the storage's parallel `field_kinds` track. Both classes are §2.7.5 producer-stamp violations but on opposite sides of the slot lifecycle. Fixing #2 (widening at construction) does NOT fix #1 (read-side kind cross-check). **Independent fixes.**

- **Cluster #4 — pointer-as-float silent-wrong-output** (`regression.md` `tdd::bug5_named_fn_as_argument`; returns 2.08e-322 = raw fn-ptr address re-interpreted as f64). **Same broad family** (a heap-pointer bit-pattern getting decoded under a wrong NativeKind), but the leak path is different: named-fn-as-value lowering + `CallValue` return-path projection vs. the nested TypedObject field-read projection here. The pointer-as-float case lands a *legal-shaped* f64 value (denormalized garbage) and quietly returns instead of OOM-ing; the nested-string case lands an *illegal-shaped* String len/cap and OOMs. Fix surfaces are unrelated (CallValue + closure carrier vs. typed_object_ops + push_field_value). **Independent fixes.**

- **No dependency on V3-S5 ckpt-5/ckpt-6 construction-cascade clusters.** Those failures are SCOPE-RECLAIM (per `TRUTH-SET.md` — 340 + 180 tests), and they surface upstream of the nested-string read path (the construction site itself doesn't materialize a TypedObject). The repro here succeeds at construction (Config and Server typed-objects are built correctly — sibling repros without `print()` wrappers but with **int** fields return `{"Integer": 8080}` correctly) and crashes only on the read+wire-projection downstream.

- **JIT JIT-FFI `RETURN_TAG_NANBOXED` SURFACE in `crates/shape-jit/src/executor.rs:802-810`** also fires on this repro (the line `[jit-fallback] ... RETURN_TAG_NANBOXED reached the host boundary without a stamped NativeKind` precedes the OOM line). This is a DOWNSTREAM symptom — when the JIT lowering for the nested-field chain fails to stamp `return_kind` for the program's top-level frame, the JIT compilation surface-and-stops, the executor falls back to the VM interpreter, and the **interpreter's** read-side bug then OOMs. The JIT surface is not the bug; the VM interpreter is. The W10 jit-playbook §5 follow-up workstream may still need to land `FrameDescriptor.return_kind = NativeKind::String` for the top-level frame, but that's a separate issue (JIT capability, not VM correctness).

## Appendix — repro fixtures (under `/tmp/`, not committed)

- `/tmp/repro_struct_nested_string_field.shape` — `structs_types::struct_nested_string_field` (4 lines).
- `/tmp/repro_v2_print_only_no_expr.shape` — same but `print(...)` wrapper; OK.
- `/tmp/repro_v3_int_only.shape` — int-typed terminal field; OK.
- `/tmp/repro_v4_one_level.shape` — single-level string; OK (but JIT fallback fires).
- `/tmp/repro_nested_typed_objects.shape` — `complex_integration::test_complex_nested_typed_objects` (with `print()`s, so OK in standalone — fails under `cargo test` host-boundary capture).
- `/tmp/repro_deep_nested.shape` — `complex_integration::test_complex_deep_nested_struct_access` (with `print()`s; OK standalone — fails under `cargo test`).
