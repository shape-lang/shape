# SB-8b / U2 — prove_native_kind made REAL + surfaced-violation catalog

Status: U2 LANDED (gate real + cataloged). U3 = wire the gate at the carrier
sites + fix the lies below. This doc is the U3 work-list.

## What U2 did

`crates/shape-vm/src/type_tracking.rs:1260` — `prove_native_kind` is no longer
a pass-through stub. New signature:

```rust
pub fn prove_native_kind(site, proven: &ConcreteType, claimed_kind: NativeKind)
    -> Result<NativeKind, ProofGap>
```

Body = `native_kind_from_concrete_type(proven)` (the ONE canonical, total
`ConcreteType → NativeKind` map, `closure_layout.rs:944`) + EXACT-equality
`kinds_consistent`. On mismatch it mints a `ProofGap` via the module-private
sealed `proof_gap(...)` — emit code cannot fabricate a pass. NO relaxation:
no int↔number, no width-narrow, no Bool-default, no UInt64-for-Ptr, no
pass-through fallback. This is the lie-detector; U3 wires it.

Gate-real proof (unit tests, `type_tracking.rs` tests module, all GREEN):
- `prove_native_kind_accepts_faithful_scalar_projection` — I64/F64/String pass.
- `prove_native_kind_accepts_faithful_heap_projection` — HashMap→Ptr(HashMap) passes.
- `prove_native_kind_rejects_sb10_uint64_for_hashmap` — UInt64-for-HashMap REFUSED.
- `prove_native_kind_rejects_sb12_bool_for_null` — Bool-for-Option REFUSED.
- `prove_native_kind_does_not_unify_int_and_number` — F64↔I64 REFUSED both ways.

## SURFACED-VIOLATION CATALOG (the U3 work-list)

These are the carrier-lie sites the real gate now refuses once wired. U2 makes
the refusal correct; U3 fixes the lie (silent-corruption → visible-refusal).
Each row: stamped kind ≠ `native_kind_from_concrete_type(proven static type)`.

| # | Site | Proven type | Stamped (lie) | Canonical (truth) | U3 fix |
|---|------|-------------|---------------|-------------------|--------|
| SB-10 | `v2_handlers/typed_map.rs` 6× `NewTypedMap*` | `HashMap<K,V>` | `UInt64` (no RC) | `Ptr(HeapKind::HashMap)` | stamp Ptr(HashMap); drop the UInt64 literal; unify lifetime onto §2.7.7 kind track |
| SB-11 | `v2_handlers/typed_map.rs:156,197` `*PtrGet` value readback | heap V (`string`/`Array`/`Struct`/`Enum`/`Closure`/`BigInt`/`Decimal`/`DateTime`) | `UInt64` (V erased) | the proven V's `Ptr(HeapKind::X)` | carry value HeapKind in opcode/operand to readback, OR surface-and-stop the lossy `value_fits_ptr_slot` carrier |
| SB-12 | `v2_handlers/typed_map.rs` 6× `*Get` None-arm | `Option<V>` None | `(0, Bool)` sentinel | `Null` | None-arm pushes `Null` not `(0,Bool)` |
| SB-12 sib | `executor/typed_handlers/typed_map.rs:109-186` | same | `(0, Bool)` | `Null` | second copy — same fix |
| SB-15 | `control_flow/mod.rs:719-737` `resolve_capture_concrete_type` | scalar `Int64` | `Ptr(HeapKind::NativeView)` | scalar `Int64` | make L4 ProofGap at the capture-stamp site (today an L5 runtime refusal) |

## Wiring DECISION to surface to user (U3)

Emit-side vs handler-side stamp:
- **Emit-side (preferred, matches the new signature):** add a `value_kind`
  operand byte to `TypedMap*Get`, stamp the proven result kind at emit, call
  `prove_native_kind` there (only place `ConcreteType` is in scope). Handler
  carries no kind literal. Catches SB-11 (value-type collapse).
- **Handler-side assert:** route `push_kinded(bits, kind)` through a
  `prove_native_kind`-backed helper. Weaker — only the opcode-implied kind is
  available, so it catches SB-10/SB-12 but NOT the SB-11 value collapse (the V
  type is already gone at the opcode).

SB-11's `value_fits_ptr_slot` (`v2_typed_map_emission.rs:125-139`) flattens 10
heap V-types into one untyped carrier — un-provable at readback unless U3
carries the value HeapKind forward, else it must surface-and-stop the carrier.

## NOT touched (legitimately-proven kinds — must NOT false-reject)

- Scalar typed-map Some-arms (`*F64Get`→Float64, `*I64Get`→Int64): faithful
  projections, pass the gate.
- `call_convention.rs` `(NONE_BITS, Bool)` frame-fill in UNREAD slots — no
  `ConcreteType` claim flows; not a proof site.
- The `binary_ops` `proof_gap_unresolved_operand` numeric proof — orthogonal.
