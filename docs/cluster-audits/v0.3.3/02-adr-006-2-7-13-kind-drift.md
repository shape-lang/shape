# v0.3.3 cluster #2 — ADR-006 §2.7.13 DerefStore kind-drift invariant violation

**HEAD at audit:** workspace tip (post-`70507224`, audit-day exception — no worktree).
**Mode:** AUDIT-ONLY. No source/fixture changes. No commits. No stash.
**Verification harness:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test <binary>` (debug build — the assertion is `debug_assert_eq!`, fires only in debug).

## Cluster sources (3 tests, two distinct sub-bugs sharing the §2.7.13 SURFACE)

| Test | Sub-bug |
|------|---------|
| `structs_types::structs::struct_field_mutation` | Sub-bug A — int→number assignment-side widening gap |
| `structs_types::structs::struct_field_mutation_second_field` | Sub-bug A (second field of same fixture) |
| `regression::tdd::bug10_nested_field_mutation` | Sub-bug B — nested `o.data.val` projection emits only one `MakeFieldRef` instead of a chain |

## Minimal repro (sub-bug A) — verified

```shape
type Point { x: number, y: number }
let mut p = Point { x: 1, y: 2 }
p.x = 10
p.x
```

```
$ direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test structs_types -- structs::struct_field_mutation --nocapture
thread 'structs::struct_field_mutation' panicked at crates/shape-vm/src/executor/variables/mod.rs:2718:9:
assertion `left == right` failed: DerefStore kind drift: popped Int64, place Float64 — ADR-006 §2.7.13 invariant violated
  left: Int64
 right: Float64
```

Release build (`./target/release/shape run /tmp/repro.shape`) prints `{"Integer": 10}` — the `debug_assert_eq!` is **stripped in release** and the writer silently lays Int64 bits in a Float64-kinded slot. **This is a release-build soundness hole** (subsequent reads via `field_kinds[0] = Float64` will reinterpret the Int64 bit pattern as f64 — `10i64` as `f64` is a denormal ≈ `5e-323`). The release-build accidental "success" reading back `10` happens only because the same assignment chain is followed by the immediate `p.x` read on bits still labeled in the `last_program_return_kind` path — change the program slightly and the silent corruption surfaces.

## Minimal repro (sub-bug B) — verified

```shape
type Inner { val: int }
type Outer { data: Inner }
let mut o = Outer { data: Inner { val: 1 } }
o.data.val = 42
```

```
thread 'tdd::bug10_nested_field_mutation' panicked at crates/shape-vm/src/executor/variables/mod.rs:3046:17:
assertion `left == right` failed: DerefStore: TypedField field_kinds[0] = Ptr(TypedObject) drift vs RefTarget captured kind Int64 — ADR-006 §2.7.13 / Q14
  left: Ptr(TypedObject)
 right: Int64
```

## Root cause hypotheses

### Sub-bug A — assignment-side missing int→number widening

**ADR-006 §2.7.13 invariant** (`docs/adr/006-value-and-memory-model.md:2434-2440`): "each variant carries the `NativeKind` of the *projected slot* alongside the identifying place data, threaded from the producing-opcode emit per §2.7.7. Loading and storing through a ref dispatch on the carried kind ... no new dispatch surface." Storage discipline: the popped value's kind MUST equal the projection's captured kind. `debug_assert_eq!` at `crates/shape-vm/src/executor/variables/mod.rs:2718`.

Construction-side widens correctly. `kinded_to_slot` at `crates/shape-vm/src/executor/objects/object_creation.rs:448-487` recognises `FieldType::F64` and converts an `Int64`-kinded popped slot to a `Float64` via `bits as i64 as f64`, returning kind `Float64`. So `field_kinds[0]` of the post-construction `Point` is `Float64`. Likewise the MakeFieldRef-captured kind in `RefTarget::TypedField` is `Float64` (from `field.field_type_tag`).

Assignment-side does NOT widen. `compile_struct_property_assignment` at `crates/shape-vm/src/compiler/expressions/assignment.rs:498-553`: line 527 emits `MakeFieldRef place.typed_operand` (kind = `Float64` from the schema's `field_type_tag`), then line 538 compiles the RHS verbatim via `compile_expr(&assign_expr.value)` — for the `10` literal, that's a `PushInt`/inline Int64 producer, no coercion against `place.field_type_info`. The DerefStore at line 546 then pops Int64 from the stack but the captured place kind is Float64 — invariant violated.

**Compare:** the construction-side `is_compatible_with` check at `crates/shape-runtime/src/type_schema/field_types.rs:166-182` (called from `compile_struct_literal` at `crates/shape-vm/src/compiler/expressions/collections.rs:1038`) permits `F64` field with `I64` literal (line 175, `(F64, I64) => true`), and `kinded_to_slot` does the runtime widening. The assignment site never invokes either gate.

**Two valid fixes (pick one — strict-typing playbook = lift to compile-time):**
1. Reject the assignment at compile time (`p.x = 10` with `x: number` → require `10.0` or explicit `10 as number`). Cleanest under "NO runtime coercion" (CLAUDE.md Type System Rules). Breaks construction-side parity (which still accepts `Point { x: 1 }`); the fix should also flip construction to compile-error for symmetry.
2. Emit the same widening shape as `kinded_to_slot` at the assignment producer site (typed value-converter opcode before `DerefStore`). **REFUSE:** this is forbidden — adding `ConvertIntToNumber` is exactly the `Convert<X>To<Y>` defection-attractor named in CLAUDE.md §Forbidden Patterns ("paper over a kind-tracker gap"). The W4-δ `ConvertBoolToString` precedent is cited as canonical example of what not to do.

**Recommended:** fix #1 (reject at compile time) for BOTH construction AND assignment; treat the construction-side `is_compatible_with` permissiveness + `kinded_to_slot` widening as the same defection that needs unwinding, not as the model to mirror.

### Sub-bug B — nested-property assignment emits only one `MakeFieldRef`

`try_resolve_typed_field_place` at `crates/shape-vm/src/compiler/helpers_reference.rs:107-141` recurses for chained `o.data.val`, returns a `TypedFieldPlace` with:
- `slot = parent.slot` (the **root** `o`'s slot, not an intermediate),
- `typed_operand = Operand::TypedField { type_id: nested_schema.id, field_idx: nested_field_idx, field_type_tag: tag(leaf) }` — the LEAF (`val`/Int64).

Assignment then emits `MakeRef root_operand` + ONE `MakeFieldRef place.typed_operand` (assignment.rs:524-528). The resulting ref projects `o.<val_idx>` on `Outer`'s schema — `field_idx 0` of `Outer` is `data: Inner` (`Ptr(TypedObject)`). DerefStore validates `receiver.field_kinds[0] == captured kind` → `Ptr(TypedObject)` ≠ `Int64`. Invariant violated at `variables/mod.rs:3046`. The fix is to emit a MakeFieldRef **chain**: `MakeRef(o); MakeFieldRef(data); DerefLoad; MakeFieldRef(val)`. The current code shape supports neither chaining at this site nor returning the correct intermediate `slot`.

## Bisect anchors

```
$ git log --oneline -10 -- crates/shape-vm/src/executor/variables/mod.rs \
                          crates/shape-vm/src/compiler/expressions/assignment.rs \
                          crates/shape-vm/src/executor/objects/object_creation.rs
```

| Commit | Relevance |
|--------|-----------|
| `ae9dd9f2` (2026-05-18) | **Strong sister-class anchor.** "Phase 4b Round 4 W14.2-G4-derefstore-drift fix" — exact same §2.7.13 SURFACE for ref-param chains. Fix at `compiler/functions.rs:1339` (producer-side stamp). Closed a sibling shape (ref-param int-only inference widened to number). **Did not close struct-literal-assignment shape (sub-bug A) nor nested-property shape (sub-bug B).** |
| `005b5170` (2026-05-18) | Merge of above — confirms ADR-006 §2.7.13 is the canonical SURFACE for this family. |
| `623ddf05` (2026-05-19) | "R5c-2-α jit-ref-param-chain-stamp" — extends the W14.2-G4 fix to the JIT consumer side, again ref-param-shaped. |
| `e4bb4757` (Phase 4b Round 3 W15.2-LANG-8) | Touched the `compile_struct_property_assignment` site for FIELD_TAG_ANY gating — last commit on `assignment.rs`. Did not address int/number widening at this site. |
| `a287c795` | RefTarget::TypedField double-free fix (migrates receiver to TypedObjectPtr) — close ancestor of the receiver-chain path under sub-bug B but does NOT address projection-chain emission. |

The DerefStore SURFACE itself was wired in pre-`82f049dd` per the §2.7.13 amendment landing (`45dedd02`). Sub-bug A + sub-bug B are not regressions of a previously-passing state visible in `git log` — they are **residual gaps** in two separate code-paths that the W14.2-G4 fix closed for the ref-param shape only.

## Affected subsystem (file:line citations)

**Runtime panic sites (both debug-assert; release silently corrupts):**
- `crates/shape-vm/src/executor/variables/mod.rs:2718` — Sub-bug A panic site (`DerefStore kind drift: popped Int64, place Float64`).
- `crates/shape-vm/src/executor/variables/mod.rs:3046` — Sub-bug B panic site (`TypedField field_kinds[i] drift vs RefTarget captured kind`).

**Construction-side (works correctly — widens):**
- `crates/shape-vm/src/executor/objects/object_creation.rs:143` (`op_new_typed_object`).
- `crates/shape-vm/src/executor/objects/object_creation.rs:448-487` (`kinded_to_slot` — widens Int64→Float64 for `FieldType::F64`).
- `crates/shape-runtime/src/type_schema/field_types.rs:166-182` (`FieldType::is_compatible_with` — permits `F64` accepts `I64`).
- `crates/shape-vm/src/compiler/expressions/collections.rs:933-1063` (`compile_struct_literal` — emits the construction-side type-check + value compile).

**Sub-bug A defective code-path (assignment):**
- `crates/shape-vm/src/compiler/expressions/assignment.rs:498-553` — `PropertyAccess` arm. Specifically:
  - line 524: `MakeRef root_operand`
  - line 525-528: `MakeFieldRef place.typed_operand` (kind = `Float64`)
  - line 538: `self.compile_expr(&assign_expr.value)` — **NO coercion** against `place.field_type_info`
  - line 545-548: `DerefStore field_ref` — pops Int64, panics at runtime invariant
- `crates/shape-vm/src/compiler/helpers_reference.rs:163-174` (`try_resolve_typed_field_place` non-recursive case) — exposes `field_type_info` to the assignment but it is not consulted for value coercion.

**Sub-bug B defective code-path (nested projection):**
- `crates/shape-vm/src/compiler/helpers_reference.rs:107-141` — the recursive arm returns parent's `slot` + leaf's `typed_operand`, producing a `(root, leaf_field_idx)` MakeFieldRef pair that mis-projects across schemas.
- `crates/shape-vm/src/compiler/expressions/assignment.rs:524-528` — emits only one `MakeFieldRef`, no projection chain.
- `crates/shape-value/src/reference.rs` (`RefTarget::TypedField`) — single-level only; nested would need a `Projected { root, projection_chain }` shape OR a load-store chain at the emitter.

## Sub-cluster name + size estimate

**Sub-cluster name:** `v0.3.3-cluster-2-adr-006-2-7-13-kind-drift`
**Size estimate:** **M (medium)** — two distinct sub-bugs (A: assignment-side widening, B: nested-projection emission). Each is 1-2 file fix in isolation, but sub-bug A demands a directional decision (compile-error vs widen — see "Recommended" above) that should be applied symmetrically to `compile_struct_literal` for consistency, expanding the blast radius. Sub-bug B requires either a `MakeFieldRef`-chain emission shape at `assignment.rs` (no `RefTarget` schema change) or a new `RefTarget::Projected` variant (ADR-006 §2.7.13 amendment).

## Dependencies — overlap with cluster #1 SIGABRT

**Yes — partial overlap.** `struct_nested_string_field` (cluster #1, SIGABRT with ~130 TB alloc) reads `cfg.server.host` through the **read-side** of the same nested-projection path (`try_resolve_typed_field_place` recursive case + downstream property access emission). The two clusters do not fix each other:
- Cluster #2 sub-bug B is the **write-side** symptom of the same nested-projection emission bug.
- Cluster #1 SIGABRT is the **read-side** symptom (likely a similar mis-projection feeding into `clone_with_kind` with a kind-tag misaligned against the bits — drives the OOM by interpreting heap-pointer bits as a length).

If the underlying field-resolution chain at `helpers_reference.rs:107-141` is rebuilt to emit a proper projection chain (or to return the correct `slot` for nested), **both clusters benefit**, and the fix is best landed once with paired test coverage from both sides. Sub-bug A (int→number widening) is **independent** — orthogonal to cluster #1.

The W14.2-G4 close commit `ae9dd9f2` already demonstrates the audit pattern (per-function compile-entry override) that should be mirrored for the struct-property-assignment and nested-projection sites here.
