# Phase 1.B-vm Wave 10 — shape-jit Consumer Migration Playbook

**Branch (parent):** `bulldozer-strictly-typed` HEAD `0277734` (W9 close).
**ADR binding:** §2.7.5 (JIT FFI = stable raw u64 + parallel `NativeKind`;
consumer-side translates to `KindedSlot` internally), §2.7.6/Q8, §2.7.10/Q11,
§2.7.11/Q12.

213 compile errors in `crates/shape-jit/`. All same shape: references to
deleted `ValueWord`, `ValueWordExt`, `ValueBits`, `tag_bits::*`,
`unified_string`, `unified_wrapper`, `unified_array`, `unified_matrix`,
`value_word_drop`, `vmarray_from_vec`. Replace per §2.7.5: at the FFI
boundary, callers pass `(u64 bits, NativeKind kind)`; consumer assembles
`KindedSlot` if needed for downstream dispatch.

---

## 1. Sub-clusters

### Band 1 — Foundation (sequential, blocks Band 2)

| Sub-cluster | Files | Errors |
|---|---|---|
| **W10-jit-kinds** | `ffi/jit_kinds.rs` | 12 |

Defines the kind-aware FFI shapes. Replace `shape_value::tag_bits::*` references with `NativeKind`-keyed dispatch; remove `ValueBits` references. Provide helpers consumers can use to build `KindedSlot` from `(u64, NativeKind)` pairs.

### Band 2 — Parallel consumers (after Band 1 merges)

| Sub-cluster | Files | Errors |
|---|---|---|
| **W10-value-ffi** | `ffi/value_ffi.rs` | 33 |
| **W10-ffi-object** | `ffi/object/{conversion,property_access,closure}.rs` + `ffi/typed_object/allocation.rs` | ~46 |
| **W10-ffi-control** | `ffi/control/mod.rs` + `ffi/call_method/mod.rs` + `ffi/result.rs` | ~48 |
| **W10-mir-compiler** | `mir_compiler/{rvalues,places,v2_int,ownership,blocks}.rs` | ~37 |
| **W10-ffi-symbols** | `ffi_symbols/{vector,data_access,object_symbols}/*` | ~28 |
| **W10-misc** | `jit_array.rs`, `ffi/v2/mod.rs`, `ffi/v2_typed.rs` | ~9 |

---

## 2. Translation rules

| Deleted | Replacement |
|---|---|
| `shape_value::ValueWord` | raw `u64` (FFI boundary) + `NativeKind` companion |
| `shape_value::ValueWordExt` | `KindedSlot` accessors per §2.7.6/Q8 (`as_str`, `as_int`, etc.) |
| `shape_value::ValueBits` | `(u64, NativeKind)` pair |
| `shape_value::tag_bits::*` | `NativeKind` discriminant arms |
| `shape_value::unified_*` | per-arm `TypedArrayData::*` constructors / `HeapValue::*` arms |
| `shape_value::value_word_drop(bits)` | `drop_with_kind(bits, kind)` per §2.7.7 |
| `shape_value::vmarray_from_vec` | `TypedArrayData::from_vec_*` per element kind |

Per §2.7.5: at the FFI boundary, the JIT produces raw `u64` results that the
VM-side caller wraps as `KindedSlot::new(ValueSlot::from_raw(bits), kind)`
where the kind is statically known from the JIT-emitted call signature
(stamped at JIT compile time, not runtime-discovered).

---

## 3. Forbidden (refuse on sight)

- All §2.7.7 / §2.7.8 / §2.7.10 / §2.7.11 forbidden patterns.
- "tag_bits restoration" / "ValueWord shim" / "ValueBits adapter" — refuse.
- Bool-default fallback for unknown kind at FFI boundary — surface-and-stop.
- Tag-bits decode in JIT codegen — replace with NativeKind-discriminator dispatch.

---

## 4. Wave-level gates

- `cargo build -p shape-jit --lib` succeeds.
- `cargo build --workspace` succeeds (the keystone gate — all crates compile).
- `bash scripts/check-no-dynamic.sh` exit 0.

---

## 5. Surface-and-stop triggers

- A site requires snapshot/restore — leave `todo!("phase-2c")` with §2.7.4 surface.
- A site needs a deleted-HeapKind variant (Stage C cluster — Set, PriorityQueue, etc.) — leave SURFACE.
- A consumer expects `ValueWord`-shape input from VM-side (e.g., a JIT trampoline call site that hasn't been migrated) — surface; this is W10's downstream blocker.

---

## 6. What's NOT in W10

- Full JIT execution path testing (W11 territory; many JIT tests are deep-tests-gated).
- Snapshot/restore of in-flight JIT state (Phase-2c §2.7.4).
- shape-test 333 compile errors (W11).

---

*Playbook closed for edits during fan-out.*
