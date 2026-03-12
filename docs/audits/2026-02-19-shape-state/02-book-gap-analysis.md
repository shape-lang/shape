# Book Gap Analysis (Code Is Authority)

## Evaluation Rule
This analysis treats code as source of truth. Book drift is only flagged when it hides or misrepresents a **real language capability/behavior gap**, not when syntax has merely evolved.

## Not Counted as Language Gaps

1. **Old import syntax examples** are documentation drift, not a missing feature.
   - Book still shows `import csv` in one chapter (`shape/docs/book/src/advanced/modules.md:16`).
   - Compiler parser rejects `import` keyword forms (`shape/shape-ast/src/parser/tests/grammar_coverage.rs:120`).

2. **Book chapter age/complexity mismatch** by itself is not a language defect.

## Real Gaps the Book Currently Fails to Cover

1. **~~References and borrow model are effectively undocumented.~~** **(Resolved 2026-03-11)**
   - Book chapters `fundamentals/references-borrowing.mdx` and `advanced/ownership-deep-dive.mdx` now document the full MIR-based NLL borrow checker with Datafrog solver, disjoint field borrows, index borrowing, task boundary rules, and reference capabilities/limits.
   - RFC `rfc-borrow-lifetimes-ergonomics-v1.md` updated to Implemented status.

2. **Annotation context API semantics are misleading.**
   - Book presents mutable-style `ctx.set/get/...` as direct runtime methods (`shape/docs/book/src/fundamentals/annotations.md:51`).
   - Current ignored tests fail with `Unknown method 'set' on TypedObject` when using this style.
   - Evidence: failing tests in `shape/shape-vm/src/executor/tests/annotations.rs:145`, `shape/shape-vm/src/executor/tests/annotations.rs:205`.

3. **`on_define` lifecycle is represented as available but not reliably realized.**
   - Annotation structure stores `on_define_handler`, but wrapper execution path only handles `before/after`.
   - `on_define` test remains ignored/failing.
   - Evidence: `shape/shape-vm/src/bytecode.rs:874`, `shape/shape-vm/src/compiler/functions.rs:559`, `shape/shape-vm/src/compiler/functions.rs:783`, `shape/shape-vm/src/executor/tests/annotations.rs:382`.

4. **Compile-time-only property resolution constraints are under-documented.**
   - Current language forbids generic runtime property lookup in many contexts.
   - This significantly impacts how annotations/meta code must be authored.
   - Evidence: `shape/shape-vm/src/compiler/expressions/property_access.rs:182`, `shape/shape-vm/src/compiler/expressions/assignment.rs:79`.

5. **Comptime directive surface exists but lacks authoritative behavioral spec coverage.**
   - Builtins and directive emission hooks are implemented (`implements`, `warning`, `error`, `build_config`, and internal directive emitters).
   - Docs are not yet aligned to directive ordering and mutation semantics in function compilation pipeline.
   - Evidence: `shape/shape-vm/src/compiler/comptime_builtins.rs:117`, `shape/shape-vm/src/compiler/functions.rs:56`.

## Tooling Drift Worth Tracking (Not a Language Gap, but High Impact)
- Tree-sitter grammar still accepts deprecated `import` forms that compiler rejects.
- This creates LSP/editor confusion and damages user trust in diagnostics/completion.
- Evidence: `shape/tree-sitter-shape/grammar.js:142` vs parser tests in `shape/shape-ast/src/parser/tests/grammar_coverage.rs:120`.

## Recommended Doc Update Priority
1. References/borrowing semantics and restrictions.
2. Annotation `ctx` semantics and immutable/mutable contract.
3. Annotation lifecycle truth table (`before`, `after`, `metadata`, `on_define`, `comptime pre/post`).
4. Compile-time property resolution rules and design rationale.
5. Comptime directive behavior and ordering.
