# Native C Interop Production Checklist

Last updated: 2026-02-25

Legend:
- `[x]` complete
- `[~]` in progress
- `[ ]` not started

1. [x] Create canonical Native C Interop chapter as single normative source in the book.
2. [x] De-duplicate native interop content in other chapters and replace with canonical links.
3. [x] Add docs guard in scripts/CI path enforcing single-source native interop docs.
4. [x] Define and codify width-aware language semantics while preserving `int`/`number` aliases.
5. [x] Update type system and compiler to preserve concrete width-aware types (no global collapse to `int`).
6. [~] Extend bytecode and VM execution for typed width-aware ops/casts and `Vec<byte>` fast paths.
7. [ ] Bring JIT optimization parity for width-aware numeric paths using existing int/number optimization families.
8. [ ] Implement JIT lowering for `CallForeign`/native ABI with typed signatures, callbacks, and `cview`/`cmut`.
9. [~] Finalize `type C` runtime object model with explicit copy boundaries and verified auto `From`/`Into`.
10. [~] Harden native dependency lock/fingerprint/cache pipeline for `system`/`path`/`vendored`.
11. [ ] Add cross-platform conformance + perf suites (Linux/macOS/Windows) for VM/JIT native interop parity.
12. [ ] Enforce hard-cut release gates: no cffi fallback, no VM-only foreign-call restriction, docs/tests/perf green.

## Current Focus

- Active item: `6` (bytecode/VM typed width-aware execution and fast-path rollout)
- Completed within item 5:
  - semantic mapping preserves explicit widths (`i8/u8/.../f32/f64`) as concrete types
  - storage and typed-value inference recognize width-aware numeric families
  - compiler type tracker preserves concrete numeric runtime type metadata
- Started within item 6:
  - compiler numeric hint paths accept width-aware scalar names and still map to existing typed int/number opcode families
- Started within item 9:
  - runtime pointer-cell builtins added for C out-parameter APIs (`__native_ptr_new_cell`, `__native_ptr_free_cell`, `__native_ptr_write_ptr`)
  - Arrow C import builtins available for strict schema binding (`__native_table_from_arrow_c_typed`)
  - stdlib low-level facade added: `std::core::native`
- Started within item 10:
  - native dependency lock artifacts now include package identity keys (`<package>@<version>::<alias>`)
  - transitive dependency packages are scanned for `[native-dependencies]` and locked in one pass
  - external fingerprints include package namespace to avoid cross-package alias collisions
- Remaining for items 7/8 (JIT parity):
  - lower `CallForeign` into JIT with typed native ABI argument/return paths
  - preserve `cmut_slice<T>` reference/writeback semantics in JIT-generated call paths
  - keep width-aware integer optimization parity (`i8/u8/.../u64/isize/usize`) with VM behavior
- Parallel guardrails now active:
  - `cargo xtask native-docs check`
  - `cargo xtask workspace-smoke` invokes native docs guard
