# Language Feature Reality (Code-Authoritative)

## Legend
- `Implemented`: behavior is present and test-backed.
- `Partial`: behavior exists but has constraints, inconsistencies, or major missing parts.
- `Gap`: expected language capability is not present yet.

## Feature Classification

| Feature Area | Status | Reality in Code | Evidence |
|---|---|---|---|
| Strong static typing | Partial | Compiler enforces compile-time field/property resolution and typed field opcodes, but some high-level method typing remains loose (`any`) and runtime dispatch still dominates. | `shape/shape-vm/src/compiler/expressions/property_access.rs:182`, `shape/shape-vm/src/bytecode.rs:306`, `shape/shape-runtime/src/type_system/checking/method_table.rs:166`, `shape/shape-vm/src/executor/objects/mod.rs:115` |
| References (`&`) | Implemented (constrained) | Language supports reference parameters and `&expr`; runtime opcodes and borrow checker are integrated. | `shape/shape-ast/src/shape.pest:363`, `shape/shape-ast/src/shape.pest:971`, `shape/shape-vm/src/bytecode.rs:169`, `shape/shape-vm/src/executor/variables/mod.rs:161` |
| Lifetime ergonomics | Partial | No explicit lifetime syntax for users; compiler has lexical borrow regions and non-escape checks. Ergonomics are good, but current model is limited to local variable borrowing in call-arg contexts. | `shape/shape-vm/src/borrow_checker.rs:13`, `shape/shape-vm/src/compiler/expressions/mod.rs:646`, `shape/shape-vm/src/compiler/expressions/mod.rs:687` |
| Borrow safety | Implemented (scope-limited) | Compile-time checker tracks active borrows, write conflicts, and escaping references. | `shape/shape-vm/src/borrow_checker.rs:169`, `shape/shape-vm/src/borrow_checker.rs:191` |
| Comptime blocks | Implemented | Comptime builtins and directives are available and gated to comptime mode. | `shape/shape-vm/src/compiler/comptime_builtins.rs:21`, `shape/shape-vm/src/compiler/expressions/function_calls.rs:485`, `shape/shape-runtime/src/builtin_metadata.rs:763` |
| Comptime annotation handlers | Implemented | `comptime pre/post` handlers run in function compilation pipeline and can mutate definitions via directives. | `shape/shape-vm/src/compiler/functions.rs:21`, `shape/shape-vm/src/compiler/functions.rs:56`, `shape/shape-vm/src/compiler/functions.rs:115` |
| Annotation runtime lifecycle | Partial | `before`/`after` wrapper flow works; `ctx` mutation semantics are inconsistent and `on_define` is compiled but not wired into invocation flow. | `shape/shape-vm/src/compiler/functions.rs:559`, `shape/shape-vm/src/compiler/functions.rs:783`, `shape/shape-vm/src/bytecode.rs:874`, `shape/shape-vm/src/executor/tests/annotations.rs:382` |
| LSP support for comptime/annotation workflows | Implemented (for covered scenarios) | LSP tests show semantic tokens, completion, hover, and generated method support for comptime paths. | `shape/tools/shape-test/tests/lsp_comptime.rs:1` |
| Module/import system | Implemented | Parser supports `from ... use ...` and namespace `use`; old `import` syntax is rejected in parser tests. | `shape/shape-ast/src/parser/tests/grammar_coverage.rs:109`, `shape/shape-ast/src/parser/tests/grammar_coverage.rs:120`, `shape/shape-ast/src/parser/tests/grammar_coverage.rs:174` |
| Parser/tooling syntax alignment | Gap | Tree-sitter still accepts deprecated `import` forms, creating editor/tooling drift from compiler behavior. | `shape/tree-sitter-shape/grammar.js:142`, `shape/tree-sitter-shape/grammar.js:147` |
| Async structured concurrency | Implemented (basic) | VM tracks async scopes/task groups and provides structured async opcodes. | `shape/shape-vm/src/executor/mod.rs:209`, `shape/shape-vm/src/bytecode.rs:333` |
| GC integration | Partial | GC feature exists with safepoint polling; integration path is incomplete (initializer has no call sites, stubs remain). | `shape/shape-vm/src/executor/mod.rs:314`, `shape/shape-vm/src/executor/dispatch.rs:103`, `shape/shape-vm/src/memory.rs:53` |

## Ergonomics Focus: Lifetimes Without Rust Pain

### What works well
- Users do not need lifetime annotations.
- Borrow safety checks are compile-time and lexical.

### Current ergonomic limits
- `&` is restricted to function call arguments and simple local names.
- References cannot be composed into richer expression forms today.
- This keeps the model learnable but narrows expressiveness for advanced dataflow patterns.

## Strong Typing vs Runtime Reality

### Good leverage already present
- Typed object field access/assignment is resolved at compile time where possible.
- Dynamic generic property lookup is intentionally rejected.

### Missed leverage opportunities
- Many table/query methods are typed as `any` in the method table.
- Runtime still performs generic method dispatch using method-name strings in hot loops.
- Row access still does repeated runtime Arrow downcasts per property access.

## Conclusion
Shape has a meaningful static typing and compile-time meta-programming core. The largest gap is not language absence; it is **incomplete end-to-end exploitation** of type information and annotation lifecycle semantics in runtime/JIT paths.
