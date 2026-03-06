# RFC: Comptime Transform API v1

Status: Accepted  
Date: 2026-02-18  
Authors: Shape runtime/compiler team

## Summary

This RFC defines a single, strict compile-time transformation model for annotations:

- One definition syntax: `annotation Name(...) { ... }`
- One compile-time hook shape: `comptime pre(...)` and `comptime post(...)`
- Typed transform directives (no user-facing string codegen APIs)
- Shared execution pipeline between `comptime { ... }` and annotation comptime handlers

The goal is to remove split-brain behavior, make transformations LSP-friendly, and keep the runtime fast and deterministic.

## Motivation

The old model mixed:

- legacy single-phase `comptime(target)` handlers,
- phase hooks (`comptime pre/post`),
- stringly mutation helpers (`set_param_type("x", "int")`, `replace_body("...")`).

That combination is hard to reason about, hard to type-check, and poor for editor tooling.

We want:

1. one transformation model,
2. typed syntax with AST spans,
3. no fallback parser/text-search behavior,
4. compile-time safety before runtime.

## Non-goals

- Runtime-generated schemas or dynamic typing.
- Special-case extension logic inside compiler/VM.
- Maintaining legacy `comptime(target)` syntax.

## Language Design

### Annotation definition

```shape
annotation derive_connection_schema() {
  targets: [function]

  comptime pre(target, ctx, ...args) {
    // Optional pre-pass
  }

  comptime post(target, ctx, ...args) {
    // Optional post-pass
  }
}
```

### Handler phases

- `comptime pre(...)`: runs before normal function compilation/inference finalization.
- `comptime post(...)`: runs after the pre-pass and before final function body emission.

Both phases can emit directives. If both exist, execution order is:

1. all `comptime pre` handlers in annotation order,
2. all `comptime post` handlers in annotation order.

### Parameters

Handlers accept:

- `target`: compile-time descriptor of the annotated item.
- `ctx`: compile-time context object.
- zero or more annotation arguments (including variadic last parameter).

### Typed transform directives

Inside comptime context, these statements are supported:

```shape
set param uri: string
set return DbConnection
replace body {
  return __runtime_connect(uri)
}
extend target {
  method ping() { true }
}
remove target
```

Directive semantics:

- `set param <name>: <Type>`: concretize an untyped parameter.
- `set return <Type>`: set return type when not explicitly declared.
- `replace body { ... }`: replace function body AST.
- `extend target { ... }`: inject methods onto target type.
- `remove target`: drop annotated target.

Invalid overrides (e.g. changing explicit types) are compile errors.

## Compatibility and Migration

### Removed

- `comptime(target) { ... }` handler form.
- User-level string mutation helpers:
  - `set_param_type(...)`
  - `set_return_type(...)`
  - `replace_body(...)`

### Required migration

Old:

```shape
annotation schema() {
  comptime(target) {
    set_param_type("uri", "string")
    set_return_type("DbConnection")
    replace_body("return runtime_connect(uri)")
  }
}
```

New:

```shape
annotation schema() {
  comptime post(target, ctx) {
    set param uri: string
    set return DbConnection
    replace body {
      return runtime_connect(uri)
    }
  }
}
```

## Compiler and VM Implementation

1. Parser/AST:
   - remove legacy comptime handler variant,
   - add directive statement nodes with spans.

2. Compiler:
   - validate target applicability once,
   - execute pre/post handlers via shared comptime pipeline,
   - process directives with strict checks.

3. VM comptime builtins:
   - keep internal directive channels only (`__emit_*`),
   - remove user-facing forwarders for mutation helpers.

4. LSP:
   - annotation discovery/docs reflect only pre/post comptime hooks,
   - diagnostics/hovers use AST spans from typed directives.

## Extension Use Case (Schema Derivation)

Extension-provided Shape source can define:

```shape
annotation schema() {
  targets: [function]
  comptime post(target, ctx) {
    set param uri: string
    set return DbConnection
  }
}

@schema()
fn connect(const uri) {
  // body replaced or validated in comptime
}
```

This keeps extension behavior generic: compiler handles annotation mechanics, extension logic lives in module source + intrinsics.

## Testing Requirements

- Parser rejects legacy `comptime(target)` definitions.
- Parser accepts typed directives (`set param`, `set return`, `replace body`).
- Compiler:
  - applies directives correctly,
  - errors on explicit-type override,
  - errors on invalid target use.
- LSP regression tests:
  - correct spans for directive-driven diagnostics,
  - no fallback-based positioning.

## Future Work

- Expand target descriptor API for non-function targets with typed patch ops.
- Add structured patch API (`insert_before`, `insert_after`, node-local rewrites).
- Keep same phase model; do not introduce new compile-time hook styles.
