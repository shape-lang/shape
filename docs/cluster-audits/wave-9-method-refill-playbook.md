# Phase 1.B-vm Wave 9 — Method-Body Re-Fill Playbook

**Branch (parent):** `bulldozer-strictly-typed` HEAD `c5b396d` (W8 close).
**ADR binding:** §2.7.6/Q8 (carrier-API-bound), §2.7.10/Q11 (MethodFnV2 ABI),
§2.7.11/Q12 (value-call ABI — closure-callback now LIVE).

This is a **mechanical migration wave**. ~150 method bodies in
`executor/objects/*.rs` (and a few elsewhere) are currently
`NotImplemented(SURFACE)` with messages like "closure-callback path
unmigrated" or "Phase-2c §2.7.4 method body". Now that W7 (closure-callback
ABI) and W8 (T25/T26/EX/WJ/AS) have landed, every dependency is live.

---

## 1. Body recipe (canonical)

Every body follows the `array_sort.rs::handle_join_str_v2` recipe (W6.5
§2.7.10 close):

```rust
pub(crate) fn handle_X_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    // 1. Receiver classification on args[0].kind:
    let arc = match args[0].slot.as_heap_value() {
        HeapValue::TypedArray(arc) => arc,        // or whichever HeapValue arm
        _ => return Err(VMError::RuntimeError(format!("X: receiver must be Array"))),
    };

    // 2. Per-element / per-arg dispatch (§2.7.6/Q8 heterogeneous-kind body):
    //    - For closure-callback ops (.map, .filter, .reduce, .sort, .find,
    //      .some, .every, .forEach):
    //        for i in 0..len {
    //            let elem_carrier = KindedSlot::new(elem_bits, elem_kind);
    //            let result = vm.call_value_immediate_nb(&closure, &[elem_carrier], ctx)?;
    //            // use result.slot, result.kind
    //        }
    //    - For non-closure ops: per-arm match on TypedArrayData::*.

    // 3. Build result, return as KindedSlot:
    Ok(KindedSlot::from_X(...))
}
```

---

## 2. Sub-clusters (fan out massively in parallel)

| Sub-cluster | File | Stubs | Pattern |
|---|---|---|---|
| W9-array-transform | `executor/objects/array_transform.rs` | 18 | `.map`, `.filter`, `.flatten`, `.flatMap`, `.groupBy`, `.concat`, `.take`, `.drop`, `.skip` |
| W9-array-query | `executor/objects/array_query.rs` | 16 | `.find`, `.findIndex`, `.some`, `.every`, `.indexOf`, `.lastIndexOf`, `.includes` |
| W9-array-aggregation | `executor/objects/array_aggregation.rs` | 11 | `.sum`, `.avg`, `.min`, `.max`, `.count`, `.reduce` (closure-callback) |
| W9-array-basic | `executor/objects/array_basic.rs` + `array_operations.rs` + `array_joins.rs` + `array_sets.rs` | ~25 | `.len`, `.first`, `.last`, `.reverse`, `.zip`, `.push`, `.pop`, `.union`, `.intersect`, `.except`, `.unique`, `.distinct`, joins |
| W9-iterator-methods | `executor/objects/iterator_methods.rs` | 19 | Custom iterators (closure-callback heavy) |
| W9-hashmap-methods | `executor/objects/hashmap_methods.rs` | 12 | `.get`, `.set`, `.has`, `.delete`, `.keys`, `.values`, `.entries`, `.merge`, `.forEach`, `.map`, `.filter`, `.reduce`, `.groupBy` |
| W9-typed-array-methods | `executor/objects/typed_array_methods.rs` + `typed_number_array_methods.rs` | ~30 | numeric reductions, closure-callback transforms |
| W9-string-methods | `executor/objects/string_methods.rs` | ~12 | trim/split/replace/pad/etc. |
| W9-set-methods | `executor/objects/set_methods.rs` | ~12 | set ops + closure-callback |
| W9-content-methods | `executor/objects/content_methods.rs` | ~9 | content tree ops |
| W9-concurrency-methods | `executor/objects/concurrency_methods.rs` | ~10 | mutex/atomic/lazy ops |
| W9-misc-methods | `priority_queue_methods.rs` + `deque_methods.rs` + `channel_methods.rs` + `column_methods.rs` + `matrix_methods.rs` + `instant_methods.rs` + `range_methods.rs` + `bool/char/number_methods.rs` | ~25 | mechanical |
| W9-datatable | `executor/objects/datatable_methods/{joins,query,aggregation,simulation,rolling,indexing}.rs` | ~12 | table ops |
| W9-property-access | `executor/objects/property_access.rs` + `object_creation.rs` + `concat.rs` | ~12 | object/property ops |
| W9-builtins-type-ops | `executor/builtins/type_ops.rs` | 16 | type-cast / coercion bodies |

**~15 sub-clusters, ~225 sites.** Some files contain multiple clusters of
related handlers; agents own per-file.

---

## 3. Forbidden (refuse on sight)

All Wave 7 §6 + Wave 8 §3 forbidden patterns apply. Specifically:

1. `Vec<KindedSlot>` by-move into `call_value_immediate_nb` — borrow `&args[..]`.
2. Bool-default fallback for unknown kinds.
3. Tag_bits decode.
4. Defection-attractor framing (any "X bridge / probe / helper / hop /
   translator / adapter / shim" descriptor of deleted / kind-blind shapes).
5. Reintroducing `Upvalue::Immutable` / `vw_clone` / `vw_drop` / `as_heap_ref`.
6. Skipping a body to "TODO later" without an explicit Phase-2c surface
   comment citing ADR-006 §2.7.4.

---

## 4. Surface-and-stop triggers

- Body genuinely depends on Phase-2c work (snapshot suspension, transport
  rebuild, etc.) — leave `NotImplemented(SURFACE)` with explicit §2.7.4
  comment. These are **acceptable** stubs (architectural deferrals,
  documented).
- API gap (e.g., `TypedArrayData::variant_kind()` doesn't exist) → surface.
- Cross-cluster cascade (e.g., body needs a method registry change
  affecting other sub-clusters) → surface.

---

## 5. Wave-level gates

- **Gate 1**: zero `todo!()` in `executor/objects/*.rs` (only
  `NotImplemented(SURFACE)` allowed, all with explicit Phase-2c surface
  comments).
- **Gate 2**: `cargo build -p shape-vm --lib` succeeds.
- **Gate 3**: `bash scripts/check-no-dynamic.sh` exit 0.
- **Gate 4**: total stub count in `executor/objects/` drops by ≥120
  (informational, not strict).

---

## 6. Dispatch shape

All ~15 sub-clusters fan out **in parallel**. No cross-file overlap; no
ordering dependency. Each agent owns 1 file or 1 file-group. Same merge
protocol as W7/W8 — supervisor merges as they close, resolves any
ordinal-style collisions on the fly.

**Time budget per agent:** 1-3 hours. Most are mechanical.

**Total wall time:** dominated by slowest agent (3h).

---

*Playbook closed for edits during fan-out.*
