# W83B / W84B / W86B HeapKind Wildcard Residuals

Date: 2026-07-02

Initial guard worker: W83B (`strict-flip-w83b-heapkind-exhaustive`)

Latest reducer: W87B (`strict-flip-w87b-heapkind-residual-hardening`)

Guard landed:

- `scripts/check-heapkind-wildcards.sh`
- `scripts/verify-merge.sh` CHECK 14

The guard is non-cargo. It scans only HeapKind-adjacent wildcard absorbers:

- direct `NativeKind::Ptr(_) =>` match arms
- legacy JIT `match heap_kind(...) { ... _ => ... }` arms
- direct `match hk|heap_kind|expected_kind { HeapKind::..., _|other => ... }` arms

It intentionally does not reject unrelated `_ =>` arms.

## Current Residual Catalog

Current guard baseline after W87B: 0 residual patterns. The W83B starting
baseline was 39; the W84B baseline was 29; the W86B baseline was 11.

1. Direct `NativeKind::Ptr(_)` arms: 0 baseline sites.
   W87B eliminated the remaining 11 direct anonymous arms. Each former
   width/null/truthiness residual now binds `NativeKind::Ptr(heap_kind)` and
   routes through a local exhaustive helper whose `HeapKind` arms all return
   the same result:
   - JIT ABI/layout width-only rows map every current `HeapKind` to pointer
     width (`I64` / 8 bytes).
   - Null-sentinel helpers map every current `HeapKind` to `bits == 0`.
   - Truthiness helpers map every current `HeapKind` to `bits != 0`.
   The helpers do not inspect payload bits, call tag readers, or infer heap
   kind at runtime; the `HeapKind` label is only matched exhaustively so a
   future enum variant must update these non-dispatch helpers intentionally.

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

3. Direct HeapKind catch-alls: 0 baseline sites.
   W86B made the four W84B direct catch-alls exhaustive while preserving
   behavior:
   - `wire_conversion.rs::slot_extract_content`: non-content HeapKinds still
     return no extracted content, but every non-content `HeapKind` is named.
   - `snapshot.rs::slot_heap_to_serializable`: unimplemented HeapKinds still
     surface as structured errors, but the unsupported group is explicit.
   - `foreign_marshal.rs::heap_slot_to_msgpack`: unsupported FFI projections
     still surface as structured errors, with every unsupported label named.
   - `compiler/comptime.rs`: unsupported comptime HeapKinds still return the
     slot without an extra retain, but the non-retained group is explicit.

## W86B Reductions

W86B removed 18 residuals from the guard baseline:

- Direct `HeapKind` catch-alls: 4 -> 0.
- JIT method-call `Ptr(_)` dispatch surfaces: 2 -> 0, via an exhaustive
  classifier for kinded heap receivers that remain on the legacy
  fallback/surface path.
- Heap ownership / retain-mask decisions: 3 -> 0, via explicit false
  inline labels (`Future`, `ModuleFn`, `Char`, `NativeScalar`) and explicit
  true heap-owning labels.
- Generic heap-element conversion helpers: 3 -> 0, via
  `array_ops::ptr_slot_to_heap_arc`, which supports only carrier labels that
  can be safely projected into a `HeapValue` wrapper and surfaces the rest.
- Unsupported dispatch/conversion surfaces in loops, property access,
  method registry fallback, `Array.join`, and typed-object field comparison:
  6 -> 0, via exhaustive `HeapKind` groups.

## W87B Reductions

W87B removed the final 11 residuals from the guard baseline:

- JIT width/layout residuals: 4 -> 0, via bound `Ptr(heap_kind)` arms and
  exhaustive same-result helpers in `v2_array.rs`, `v2_call_abi.rs`, and
  `v2_field.rs`.
- VM null-sentinel residuals: 3 -> 0, via exhaustive same-result helpers in
  `comparison/mod.rs`, `exceptions/mod.rs`, and `logical/mod.rs`.
- VM truthiness residuals: 4 -> 0, via exhaustive same-result helpers in
  `control_flow/mod.rs`, `logical/mod.rs`, `array_aggregation.rs`, and
  `array_query.rs`.

The checker's known baseline is intentionally empty after W87B. Any new
anonymous direct `NativeKind::Ptr(_) =>` arm, legacy JIT `heap_kind(...)`
catch-all, or direct `HeapKind` match catch-all fails the guard.

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
