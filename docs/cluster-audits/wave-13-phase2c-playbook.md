# Wave 13 — Phase-2c Surface Closure Playbook

**Branch (parent):** `bulldozer-strictly-typed` HEAD `2b65a36` (post-W11-W12 close).
**Goal:** close the highest-value Phase-2c §2.7.4 surfaces deferred during W7-W12 so user-facing execution paths complete end-to-end.

W7-W12 delivered the architectural ABI rebuild (workspace --all-targets clean,
`--mode vm` runs simple programs end-to-end). Wave 13 fills the remaining
surfaces that block real programs:

- `print(x)` formatter (Wave 5e deferral)
- `--mode jit` init panic (W12 surface)
- AnyError TypedObject builder (W8-EX surface, 11 sites)
- HashMapData typed-buffer mutation API (W9-hashmap surface, ~3 sites)
- HashSet rebuild (W9-set Stage C surface, ~12 sites)
- IteratorState rebuild (W9-iterator surface, 19 sites)

---

## Round 1 — parallelizable, no cross-dependency

### W13-print-formatter — Wave 5e formatter rebuild

**Territory:** `crates/shape-vm/src/executor/printing.rs` + the ~15 `todo!()` /
`NotImplemented(SURFACE)` sites at `vm_impl/builtins.rs:315-468` that the
print/format handlers route through.

**Pattern:** the formatter dispatches on `args[0].kind` (the value to format)
per §2.7.6/Q8 heterogeneous-kind body, builds a `String` via per-arm rendering
(int → `i64::to_string`, float → `f64::to_string` w/ trailing-zero
normalization, bool → "true"/"false", string → escaped, `Ptr(HeapKind::*)` →
recover via `slot.as_heap_value()` + per-`HeapValue::*` match), passes through
`OutputAdapter::print`. Existing per-kind formatters live in
`shape-runtime/src/value_formatter.rs` if needed.

**Goal:** `shape run --mode vm` prints `print("hello")` → `hello`,
`print(42)` → `42`, `print([1,2,3])` → `[1, 2, 3]`, etc.

**Time:** 2-3h.

### W13-jit-init — `--mode jit` startup gap

**Territory:** investigate panic at JIT init (W12-host-boundary surfaced
`shape-jit/src/compiler/ffi_builder.rs:35` "no entry found for key" during
stdlib JIT compilation). Likely a registry-key mismatch from W10 deletions.

**Goal:** `shape run --mode jit 'let x = 1+2; x'` works (or fails with a
clear "JIT path stubbed pending §2.7.14 JitArray rebuild" error rather than
panicking).

**Time:** 1-2h investigation + small fix.

### W13-anyerror — W8-EX AnyError TypedObject builder

**Territory:** `crates/shape-vm/src/executor/exceptions/mod.rs` ~11
`NotImplemented(SURFACE)` sites all referencing "AnyError TypedObject
re-emission depends on D-raw-helpers cleanup".

**Pattern:** AnyError carrier per shape-runtime's `AnyError` schema.
Construct via the §2.7.10/Q11 dispatch precedent — kinded `KindedSlot` per
field, `HeapValue::TypedObject(Arc<TypedObjectStorage>)` envelope.

**Goal:** `try { ... } catch (e: AnyError) { ... }` paths execute.

**Time:** 2-3h.

### W13-hashmap-mutation — HashMapData typed-buffer mutation API

**Territory:** `crates/shape-value/src/heap_value.rs::HashMapData` —
add `set/delete/merge` methods that respect the typed-buffer invariant
(per-key kind, per-value kind tracked via parallel `Vec<NativeKind>` per
§2.7.7). Then unblock the 3 W9-hashmap-methods SURFACEs (`v2_set`,
`v2_delete`, `v2_merge`).

**Pattern:** mutation via `Arc::make_mut` (clone-on-write) so existing
shared references stay immutable. Per-key-kind dispatch on insert; drop
via `drop_with_kind` on the prior occupant per §2.7.7 retain/release.

**Goal:** `let m = HashMap(); m.set("a", 1); m.set("b", 2); m.delete("a");
m.size()` runs.

**Time:** 2-3h.

---

## Round 2 — depends on Round 1 (or can land in parallel)

### W13-hashset-rebuild — Stage C HeapKind::HashSet variant

**Territory:** new `HashSetData` struct in shape-value (mirror of
`HashMapData` per W9-set-methods agent's "Path A" recommendation), new
`HeapKind::HashSet` variant, new `HeapValue::HashSet(Arc<HashSetData>)`
arm. Unblocks ~12 W9-set-methods SURFACEs.

**ADR:** §2.7.15 / Q16 amendment (mirror of §2.7.9 FilterExpr precedent).

**Time:** 3-4h.

### W13-iterator-state — Lazy iterator carrier rebuild

**Territory:** new `IteratorState` enum + `HeapKind::Iterator` variant +
`HeapValue::Iterator(Arc<IteratorState>)` arm. Unblocks 19
W9-iterator-methods SURFACEs.

**ADR:** §2.7.16 / Q17 amendment.

**Time:** 4-5h. Largest Round-2 sub-cluster.

---

## Forbidden

All previous waves' forbidden patterns apply:
- ValueWord / ValueWordExt / ValueBits / tag_bits revival
- Bool-default fallback for unknown kind
- Defection-attractor framing (X bridge / probe / helper / etc.)
- Reintroducing deleted shapes under aliases

---

## Out of scope (deeper Phase-2c — Wave 14+)

- **§2.7.14 JitArray rebuild** (Q15) — multi-day, formal ADR amendment landed
- Snapshot/restore in-flight async — multi-week
- Stage C deleted-HeapKind family beyond Set (PriorityQueue, Deque, Channel,
  Column, Matrix, Range — each its own Stage C cluster)
- Foreign-fn kind-blind retain/release ABI (W10-misc surfaces)
- HOF closure-callback bodies still SURFACE (~150 W9 sites that surfaced
  with op_make_closure dependency — re-fillable now that W12 closed
  op_make_closure, but mechanical migration needs a follow-up Wave)

---

## Wave-level gates

- `cargo check --workspace --all-targets` exit 0 (preserve from W11/W12)
- `bash scripts/check-no-dynamic.sh` exit 0
- Smoke tests:
  - `print("hello")` outputs `hello`
  - `print(42)` outputs `42`
  - `--mode jit 'let x = 1; x'` either works OR fails cleanly with §2.7.14 marker
  - `try { error("x") } catch (e) { print(e.message) }` outputs `x`
  - `let m = HashMap(); m.set("a", 1); m["a"]` returns `1`

Each sub-cluster's close commit cites the playbook + ADR.

---

*Playbook closed for edits during fan-out.*
