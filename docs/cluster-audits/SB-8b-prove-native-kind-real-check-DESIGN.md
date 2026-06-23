# SB-8b — `prove_native_kind` real-check DESIGN (U2 keystone)

Status: DESIGN (read-only forecast). Implementation = U3.
Target: `crates/shape-vm/src/type_tracking.rs:1251-1257`.

## 0. The current lie

```rust
pub fn prove_native_kind(site, claimed_kind) -> Result<NativeKind, ProofGap> {
    Ok(claimed_kind)   // pass-through; ProofGap constructor NEVER invoked
}
```

It has **ZERO call sites** (`grep prove_native_kind(` → only the def + doc
comments). The only proof mechanism actually wired today is
`proof_gap_unresolved_operand` in `compiler/expressions/binary_ops.rs:347/2493`.
So `prove_native_kind` is theatrical twice over: it's a no-op AND nothing
calls it. SB-8b is the keystone that lets SB-9..SB-12/SB-15 ship: nothing
forces the runtime carrier-kind to be a faithful projection of the proven
static type.

## 1. What a REAL check needs (signature change)

A check that takes only `claimed_kind` is structurally incapable of proving
anything — there is nothing to compare against. The real predicate must
receive the **proven static type** for the slot/value at the emission site.
New signature:

```rust
pub fn prove_native_kind(
    site: &'static str,
    proven: &ConcreteType,   // the statically-proven type (NOT inferrable luck)
    claimed_kind: NativeKind,
) -> Result<NativeKind, ProofGap>
```

Body:

```rust
let expected = native_kind_from_concrete_type(proven); // canonical projection
if kinds_consistent(expected, claimed_kind) {
    Ok(claimed_kind)
} else {
    Err(proof_gap(site, format!(
        "claimed {claimed_kind:?} but proven static type {proven:?} projects to {expected:?}"
    )))
}
```

`proof_gap`'s constructor is **private to type-tracking** (`ProofGapSeal`),
so emit code cannot fabricate a proof — the Rust type system enforces that
only this body can mint a pass. That is the mechanical lever; the body must
USE it, not return `Ok` on the mismatch path.

### Canonical Type→NativeKind projection (resolves SB-17's 4 partial maps)

SB-17: four partial discriminators each with a partial map:
- `native_kind_from_storage_type` (`type_tracking.rs:88`) — **None** for
  Array/Table/Object/Result/TaggedUnion/Function/Struct/Dynamic (8 families
  unmapped). Unusable as the canonical map: it cannot project a heap type.
- `FieldType::to_native_kind` (`field_types.rs:258`) — **refuses**
  Option/HashMap/Set (Err). Also partial.
- `native_kind_from_concrete_type` (mir_compiler/types.rs:151) — Option<…>,
  partial.
- **`native_kind_from_concrete_type` (`closure_layout.rs:944`)** — TOTAL over
  ConcreteType, every heap family → its dedicated `Ptr(HeapKind::X)`
  (HashMap→`Ptr(HeapKind::HashMap)`, String→`String`, Array→
  `Ptr(HeapKind::TypedArray)`, … HashSet/Deque/Channel/Mutex/… each its own
  ordinal), and **refuses Void by panic** (no sentinel — ADR-006 §2.7.7
  forbidden #9). This is the ONE canonical projection. The real check reuses
  it; the other three are the partial maps the audit flags — U3 should route
  callers to this one (or surface where ConcreteType isn't available).

`kinds_consistent(expected, claimed)`: exact equality, with the ONE bounded
allowance that `expected == claimed`. No int↔number relaxation (CLAUDE.md:
"int and number are separate"). No Bool-default. No width-narrowing. A
`Ptr(HeapKind::HashMap)` proven type with a `UInt64` claim is a HARD reject —
that is exactly SB-10.

## 2. Every `prove_native_kind` caller — TODAY: none

The check is wired NOWHERE. So U3 is two jobs:
(a) make the body a real check (§1), and
(b) wire it at the carrier-stamp sites that today push a kind without proof.

The carrier-lie sites are NOT emit-side today — they are **runtime handler
hardcodes** that call `push_kinded(bits, KIND)` directly. The handler knows
the opcode (hence the static K/V types the opcode was selected for at emit
time), so the proof obligation is: the kind stamped on `push_kinded` for a
typed-collection result must equal `native_kind_from_concrete_type(static V)`.
Two wiring strategies for U3 to choose (surface to user — this is a decision):

- **Emit-side (preferred, matches §1 signature):** stamp the proven result
  kind into the opcode operand at emit time (e.g. a `value_kind` byte on
  `TypedMap*Get`), have the handler push THAT kind, and call
  `prove_native_kind` at emit when computing it. This is the only place
  `ConcreteType` is in scope. The runtime handler then carries no kind
  literal at all.
- **Handler-side assert:** keep `push_kinded(bits, kind)` but route through a
  `prove_native_kind`-backed helper. Weaker — the handler only has the opcode,
  not the full ConcreteType — so it can only prove the opcode-implied kind,
  which still catches SB-10/11/12 (UInt64/Bool literals) but not the SB-11
  value-type collapse (the type was already thrown away at the opcode).

The SB-11 collapse (`value_fits_ptr_slot` flattening 10 heap V-types into one
untyped `StringPtr`/`I64Ptr` carrier) is **un-provable at readback** because
the static V is gone by then. The real check forces this into the open: U3
must either (i) carry the value `HeapKind` in the opcode/operand so the
readback can stamp the true `Ptr(HeapKind::X)`, or (ii) surface-and-stop that
the typed-map-of-heap-values carrier is lossy. Either way the lie stops being
silent.

## 3. SURFACED-VIOLATION CATALOG (the U3 work-list)

The real gate REJECTS these. Each is a place where the stamped kind ≠
`native_kind_from_concrete_type(proven static type)`:

| # | Site | Proven static type | Stamped kind (lie) | Canonical projection (truth) | Verdict |
|---|------|--------------------|--------------------|------------------------------|---------|
| SB-10 | `v2_handlers/typed_map.rs:77,82,87,92,97,102` (all 6 `NewTypedMap*`) | `HashMap<K,V>` | `UInt64` (no refcount) | `Ptr(HeapKind::HashMap)` | **REJECT** — `is_refcounted(UInt64)==false` skips retain/release; the HashMap arm of `clone_with_kind` would corrupt it if it ever saw it. Lifetime split with compiler-emitted scope-drop opcodes (`compiler/mod.rs:1205`). |
| SB-11 | `v2_handlers/typed_map.rs:156,197` (`*PtrGet` value readback) | `string`/`Array<T>`/`Struct`/`Enum`/`Closure`/`BigInt`/`Decimal`/`DateTime` (any `value_fits_ptr_slot`, `v2_typed_map_emission.rs:125-139`) | `UInt64` (hardcoded, value-type erased) | the proven V's `Ptr(HeapKind::X)` (e.g. `String`, `Ptr(HeapKind::TypedArray)`, …) | **REJECT** — irreversible static-type loss at the carrier. Compiler says `string`; readback says `UInt64`. |
| SB-12 | `v2_handlers/typed_map.rs:121,139,157,172,185,198` (every `*Get` None-arm) | `Option<V>` → the None case | `(NONE_BITS=0, NativeKind::Bool)` | `Null` (post-R5b stack-track encoding; `native_kind.rs:112-153` names `(0,Bool)⇔false` as the EXACT removed unsound collision) | **REJECT** — present-but-`false` map value bit-indistinguishable from absent. Regressed forbidden sentinel. |
| SB-12 (sibling) | `executor/typed_handlers/typed_map.rs:109-186` | same | same `(0,Bool)` | `Null` | **REJECT** — second copy of the sentinel. |
| SB-15 | closure capture: `resolve_capture_concrete_type` stamps `Ptr(HeapKind::NativeView)` for a transitively-captured unproven param that arrives scalar `Int64` (`control_flow/mod.rs:719-737`) | scalar `Int64` (runtime track) | `Ptr(HeapKind::NativeView)` (compile-time stamp) | scalar `Int64` | **REJECT/SURFACE** — already surface-and-stops at HEAD (would drop a small int as Arc → SIGSEGV). The real check makes this an L4 ProofGap at the stamp site, not an L5 runtime refusal. Two sources for one capture's kind (`capture_native_kind(i)` vs popped §2.7.7 kind). |

Sentinel/`call_convention.rs` `(NONE_BITS, Bool)` frame-fill (lines 221/245/600/731/805/1402) is a SEPARATE legitimate use — it fills *unused* frame slots that are never read as values (Drop/Clone-no-op). The real check does NOT touch those because no `ConcreteType` claim flows through them; they are not value reads. (Risk note §5.)

## 4. How a mismatch SURFACES (no crash, no silence)

- Emit-side caller: `prove_native_kind(...)?` returns `ProofGap`; the caller
  converts to a clean compile error exactly as `numeric_operand_proof_gap`
  already does in `binary_ops.rs:2493` — `E_TYPED_OPCODE_WITHOUT_PROOF at
  <site>: claimed X but proven type T projects to Y`. The program does not
  compile. This is the CLAUDE.md contract: "if the type can't be proven, it
  is a compile error."
- Where the kind genuinely cannot be carried to the proof point (SB-11
  readback with the static V already erased), the obligation is a
  `NotImplemented(SURFACE)` surface-and-stop at the carrier-selection site
  (`should_use_typed_map` / `value_fits_ptr_slot`), NOT a runtime push of a
  fabricated kind. The lossy carrier is refused at emit, surfaced to the user.
- **Forbidden escapes (refused on sight):** keeping the pass-through "as a
  fallback"; a "warn but allow" mode; relaxing `kinds_consistent` to accept
  `UInt64` for any `Ptr(...)`; adding a `ConvertUInt64ToPtr` opcode; Bool/
  Float64 default on the unknown path. The POINT is to make the lies visible.

## 5. Risk — currently-passing paths that legitimately HAVE a proven kind

Must NOT false-reject:
- **Scalar typed-map values** (`TypedMap*F64Get` → `Float64` at
  `typed_map.rs:120,171`; `*I64Get` → `Int64` at `:138,184`): these stamp the
  correct projection of the proven V (`F64`→`Float64`, `I64`→`Int64`). The
  Some-arm of these is FINE; only their None-arm (SB-12) and the *Ptr value
  arms (SB-11) lie. The check must pass the scalar Some-arms.
- **Frame-fill sentinels** (`call_convention.rs`): `(NONE_BITS, Bool)` in
  unread slots — no value claim, not a proof site. Don't wire the check here.
- **The `binary_ops` numeric proof** already in place: unaffected; it uses
  `proof_gap_unresolved_operand` for the `Type::Variable` unresolved case,
  orthogonal to the carrier-projection check.

Must REJECT (the catalog §3): SB-10 alloc-kind, SB-11 value-collapse, SB-12
None-sentinel (both copies), SB-15 capture-stamp.

## 6. U3 work-list summary

1. Change `prove_native_kind` signature to take `&ConcreteType`; body =
   `native_kind_from_concrete_type` projection + exact-match (mint ProofGap on
   mismatch via the sealed constructor). Add `kinds_consistent`.
2. Pick wiring strategy (§2) — **surface the emit-side-vs-handler-side
   decision to the user**; emit-side is preferred but requires opcode-operand
   widening for typed-map value kind.
3. Stamp truth at the 6 `NewTypedMap*` allocs → `Ptr(HeapKind::HashMap)`
   (fixes SB-10; also unifies the split lifetime authority onto the §2.7.7
   kind track).
4. Carry the value `HeapKind` to the `*PtrGet` readback or surface-and-stop
   the lossy carrier (fixes SB-11).
5. None-arm → `Null` not `(0,Bool)` in both typed-map handler copies
   (fixes SB-12).
6. Convert the SB-15 runtime refusal into an L4 ProofGap at the capture-stamp
   site.
7. Delete the three non-canonical partial Type→kind maps' use at these sites
   (route to `native_kind_from_concrete_type`); leave SB-17 cleanup of the
   remaining callers as a tracked follow-up if out of U3 blast radius —
   surface, don't silently broaden.

No regressions expected on the scalar Some-arms or frame-fill sentinels.
The catalog rows are clean compile-errors / SURFACE refusals that the gate
SHOULD now produce — they are the U2 deliverable, not regressions.
