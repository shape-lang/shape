# Shape Codebase State Audit (2026-02-19)

## Scope
This audit treats code as the authority and uses book/docs only to identify true language understanding gaps.

## Headline Findings

1. **Shape is strongly typed, but performance-critical paths still underuse static type information.**
   - The compiler enforces compile-time field resolution (`shape/shape-vm/src/compiler/expressions/property_access.rs:182`, `shape/shape-vm/src/compiler/expressions/assignment.rs:79`).
   - However, many table/query methods are still type-checked as `any` in core method typing (`shape/shape-runtime/src/type_system/checking/method_table.rs:166`).
   - Runtime method dispatch is still string-based in hot paths (`shape/shape-vm/src/executor/objects/mod.rs:115`).

2. **References/borrowing exist (without explicit lifetime syntax), but are intentionally constrained.**
   - Reference syntax exists (`shape/shape-ast/src/shape.pest:367`, `shape/shape-ast/src/shape.pest:971`).
   - Compiler currently restricts `&` to call arguments and simple local identifiers (`shape/shape-vm/src/compiler/expressions/mod.rs:646`, `shape/shape-vm/src/compiler/expressions/mod.rs:695`).
   - Borrow checker exists and enforces non-escape/alias rules (`shape/shape-vm/src/borrow_checker.rs:1`).

3. **Comptime and annotation infrastructure is substantial, but annotation lifecycle semantics are incomplete.**
   - Comptime builtins/directives are implemented (`shape/shape-vm/src/compiler/comptime_builtins.rs:21`, `shape/shape-vm/src/compiler/comptime_builtins.rs:117`).
   - LSP support is strong in comptime workflows (13 tests passing in `shape/tools/shape-test/tests/lsp_comptime.rs`).
   - Ignored annotation tests fail around `ctx.set/get` and `on_define` behavior (`shape/shape-vm/src/executor/tests/annotations.rs:145`, `shape/shape-vm/src/executor/tests/annotations.rs:382`).

4. **VMValue is not retired.**
   - Inventory: 1345 references in 69 files; large usage remains in runtime/hot path crates.
   - VMValue guard currently prevents *new* spread but does not indicate removal completion.

5. **JIT is the main blocker for competitiveness with Node/V8 today.**
   - Updated benchmark re-check (2026-02-19): `1` JIT win vs Node, `9` losses, geometric mean `3.31x` slower.
   - Former crash cases now execute (`03_sieve`, `05_spectral`, `09_matrix_mul`), but two are still pathological (`03_sieve` ~`6.8x` slower than Node, `09_matrix_mul` ~`11.0x` slower).

6. **Maintainability risk is concentrated in very large modules and duplicated semantic surfaces.**
   - Several core files exceed 2k-3.6k LOC (`shape/shape-vm/src/compiler/mod.rs`, `shape/shape-vm/src/lib.rs`, `shape/shape-runtime/src/type_system/inference/mod.rs`, `shape/tools/shape-lsp/src/hover.rs`).
   - Parser and tree-sitter grammar have diverged for imports (`shape/shape-ast/src/parser/tests/grammar_coverage.rs:120` vs `shape/tree-sitter-shape/grammar.js:142`).

## What Matters Most Right Now

1. Sustain JIT correctness and remove pathological slow paths/placeholder fallbacks.
2. Make annotation lifecycle behavior coherent (`ctx` mutability semantics + `on_define` execution).
3. Push static type knowledge deeper into runtime dispatch (remove stringly hot-path method calls where possible).
4. Decide VMValue strategy explicitly (coexistence boundary vs full retirement) and align tooling/guards accordingly.

## Report Set
- `01-language-feature-reality.md`
- `02-book-gap-analysis.md`
- `03-comptime-annotation-audit.md`
- `04-architecture-maintainability-dry.md`
- `05-performance-bottlenecks.md`
- `06-vmvalue-gc-status.md`
