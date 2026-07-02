# W83B HeapKind Wildcard Residuals

Date: 2026-07-02

Worker: W83B (`strict-flip-w83b-heapkind-exhaustive`)

Guard landed:

- `scripts/check-heapkind-wildcards.sh`
- `scripts/verify-merge.sh` CHECK 14

The guard is non-cargo. It scans only HeapKind-adjacent wildcard absorbers:

- direct `NativeKind::Ptr(_) =>` match arms
- legacy JIT `match heap_kind(...) { ... _ => ... }` arms
- direct `match hk|heap_kind|expected_kind { HeapKind::..., _|other => ... }` arms

It intentionally does not reject unrelated `_ =>` arms.

## Current Residual Catalog

1. Direct `NativeKind::Ptr(_)` arms: 25 baseline sites.
   These are mostly generic pointer width, null/truthiness, method-delegation,
   or unsupported-container surfaces. They should not grow without review
   because a future HeapKind can inherit the wrong default behavior.

2. Legacy JIT `heap_kind(...)` catch-alls: 10 baseline sites.
   Concentrated in `crates/shape-jit/src/ffi/{conversion,iterator,object}/`.
   These return `unknown`, `[unknown]`, `TAG_NULL`, `true`, object defaults,
   or `NaN`. Wave 2 should either make the HK dispatch exhaustive or route
   through `NativeKind::Ptr(HeapKind::*)` where available.

3. Direct HeapKind catch-alls: 4 baseline sites.
   - `wire_conversion.rs::slot_extract_content`: non-content HeapKinds fall
     through to no extracted content.
   - `snapshot.rs::slot_heap_to_serializable`: unimplemented HeapKinds surface
     as structured errors.
   - `foreign_marshal.rs::heap_slot_to_msgpack`: unimplemented HeapKinds
     surface as structured errors.
   - `compiler/comptime.rs`: unsupported HeapKinds return a KindedSlot without
     an extra retain; this is only safe while those kinds are not produced by
     the comptime predeclared schemas.

## Wave 2 Notes

- `wire_conversion.rs::heap_to_wire` still has a broader typed-Arc hazard:
  after special-casing `Char`, `Result`, `Option`, `TypedObject`, `TypedArray`,
  and `HashMap`, it falls back to treating remaining HeapKinds as
  `*const HeapValue`. That is wrong for typed-Arc-only labels such as
  `Reference`, `SharedCell`, `Matrix`, and `MatrixSlice`. Fix requires
  per-HeapKind decode policy, so this worker only cataloged it.
- Prefer exhaustive `HeapKind` matches for dispatch tables. If a wildcard is
  needed as a structured surface, keep the error message explicit and update
  the checker baseline with the owning wave and rationale.
