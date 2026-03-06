# Architecture, Maintainability, and DRY Audit

## Executive Judgement
The codebase has strong technical ambition and significant infrastructure depth, but maintainability is currently constrained by oversized modules, duplicated semantic registries, and mixed architecture layers that make changes high-risk.

## Maintainability Signals

### 1. File-size concentration in core modules
Large files are carrying too many responsibilities:
- `shape/shape-vm/src/compiler/mod.rs` (2335 LOC)
- `shape/shape-vm/src/lib.rs` (2252 LOC)
- `shape/shape-runtime/src/type_system/inference/mod.rs` (2903 LOC)
- `shape/tools/shape-lsp/src/hover.rs` (3644 LOC)

Impact:
- Harder reviewability.
- Higher regression risk from local edits.
- Lower discoverability of invariants.

### 2. Semantic duplication (DRY concerns)

#### Method capability described in multiple places
- Type checker method registry: `shape/shape-runtime/src/type_system/checking/method_table.rs:166`
- Runtime method dispatch registries: `shape/shape-vm/src/executor/objects/method_registry.rs:112`

Risk:
- Drift between what type checker allows and what runtime actually supports.

#### Parser/compiler vs tree-sitter grammar divergence
- Compiler-side parser rejects `import` keyword forms (`shape/shape-ast/src/parser/tests/grammar_coverage.rs:120`).
- Tree-sitter still accepts deprecated forms (`shape/tree-sitter-shape/grammar.js:142`).

Risk:
- Editor truth diverges from compiler truth.

#### Legacy semantic layer + newer type inference coexistence
- Semantic bridge code still actively converts between representations, increasing complexity and coupling.
- Evidence: `shape/shape-runtime/src/semantic/mod.rs:165`, `shape/shape-runtime/src/semantic/types.rs:266`.

### 3. Mixed abstraction depth in hot runtime code
- Some paths are optimized and cleanly registry-driven.
- Other paths still rely on string method names and dynamic behavior in performance-sensitive loops.
- Evidence: `shape/shape-vm/src/executor/objects/mod.rs:115`.

## Rust Expressiveness / Idiomaticity Assessment

## What is strong
1. Use of PHF maps for O(1) method resolution is a good direction.
2. Separation into crates (`shape-value`, `shape-runtime`, `shape-vm`, `shape-jit`) is conceptually sound.
3. Typed opcodes and compile-time schema tracking show intent to shift work from runtime to compile time.

## What is currently unidiomatic or under-leveraged
1. Very large modules suggest insufficient trait/module decomposition.
2. Some architecture still relies on stringly-typed dispatch where enum/typed call-site forms could be used.
3. Runtime and typechecker representations are not fully unified, forcing bridge code.
4. Placeholder/incomplete sections in JIT and GC integration remain in primary paths, which weakens confidence boundaries.

## DRY Scorecard (qualitative)
- Parser/AST/compiler core: **Medium** (fairly coherent, but some drift at ecosystem boundaries).
- Type system/runtime execution contract: **Low-Medium** (duplicate method descriptions, loose typing in table methods).
- Tooling alignment (parser vs tree-sitter/LSP): **Low** (known syntax drift).

## Recommended Refactor Program

1. **Split giant modules by stable responsibility boundaries.**
   - Example: split `shape-vm` compiler into `type_flow`, `borrow`, `comptime`, `annotation_wrap`, `call_lowering`.

2. **Create a single source of truth for method signatures/capabilities.**
   - Generate both type-check registry and runtime method table from one descriptor.

3. **Unify parser and tree-sitter grammar contracts with CI parity tests.**
   - Reject grammar drifts at PR time.

4. **Define explicit architecture invariants in docs near code.**
   - For value representation boundaries, annotation lifecycle phases, and typed dispatch guarantees.

5. **Stabilize internal APIs before new feature fan-out.**
   - Especially around annotation context and JIT FFI surfaces.

## Bottom Line
This is not “implemented without thinking,” but it is currently in a **transition-heavy architecture phase** where abstractions have outgrown module boundaries. Rust expressiveness is visible in parts; it is not yet consistently leveraged across the full system.
