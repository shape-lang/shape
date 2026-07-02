# W83B / W84B HeapKind Wildcard Residuals

Date: 2026-07-02

Initial guard worker: W83B (`strict-flip-w83b-heapkind-exhaustive`)

Latest reducer: W84B (`strict-flip-w84b-heapkind-wildcards`)

Guard landed:

- `scripts/check-heapkind-wildcards.sh`
- `scripts/verify-merge.sh` CHECK 14

The guard is non-cargo. It scans only HeapKind-adjacent wildcard absorbers:

- direct `NativeKind::Ptr(_) =>` match arms
- legacy JIT `match heap_kind(...) { ... _ => ... }` arms
- direct `match hk|heap_kind|expected_kind { HeapKind::..., _|other => ... }` arms

It intentionally does not reject unrelated `_ =>` arms.

## Current Residual Catalog

Current guard baseline after W84B: 29 residual patterns. The W83B starting
baseline was 39.

1. Direct `NativeKind::Ptr(_)` arms: 25 baseline sites.
   These are mostly generic pointer width, null/truthiness, method-delegation,
   or unsupported-container surfaces. They should not grow without review
   because a future HeapKind can inherit the wrong default behavior.

2. Legacy JIT `heap_kind(...)` catch-alls: 0 baseline sites.
   W84B removed the 10 direct legacy JIT residuals in:
   `crates/shape-jit/src/ffi/conversion.rs`,
   `crates/shape-jit/src/ffi/iterator.rs`,
   `crates/shape-jit/src/ffi/object/format.rs`,
   `crates/shape-jit/src/ffi/object/object_ops.rs`, and
   `crates/shape-jit/src/ffi/object/property_access.rs`.
   Supported legacy `HK_*` cases remain explicit. Unsupported or unstamped
   heap labels now route to cold `SURFACE` panics instead of silently becoming
   `unknown`, `[unknown]`, `TAG_NULL`, completed iterators, object defaults,
   or `NaN`.

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

- W84B intentionally did not broaden the scanner to reject all local-variable
  matches over raw `u16` JIT kind codes. The converted sites no longer use
  silent defaults: their catch-all paths are explicit unsupported-kind surfaces
  and preserve existing supported `HK_*` arms.
- `wire_conversion.rs::heap_to_wire` still has a broader typed-Arc hazard:
  after special-casing `Char`, `Result`, `Option`, `TypedObject`, `TypedArray`,
  and `HashMap`, it falls back to treating remaining HeapKinds as
  `*const HeapValue`. That is wrong for typed-Arc-only labels such as
  `Reference`, `SharedCell`, `Matrix`, and `MatrixSlice`. Fix requires
  per-HeapKind decode policy, so this worker only cataloged it.
- Prefer exhaustive `HeapKind` matches for dispatch tables. If a wildcard is
  needed as a structured surface, keep the error message explicit and update
  the checker baseline with the owning wave and rationale.
