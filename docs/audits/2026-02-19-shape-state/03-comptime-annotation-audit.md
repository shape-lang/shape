# Comptime and Annotation Audit

## Product Target (as interpreted)
- Keep the power of Rust macro-style generation and validation.
- Keep Zig-style compile-time execution and target introspection.
- Remove incidental complexity and preserve natural source ergonomics.
- Keep LSP-aware workflows first-class.

## What Is Already Strong

1. **Comptime execution exists with clear language gate.**
   - Comptime-only builtins are recognized and blocked outside comptime blocks.
   - Evidence: `shape/shape-vm/src/compiler/expressions/function_calls.rs:485`, `shape/shape-runtime/src/builtin_metadata.rs:763`.

2. **Directive-driven compile-time mutation exists.**
   - Compiler tracks directives (`Extend`, `RemoveTarget`, `SetParamType`, `SetReturnType`, `ReplaceBody`).
   - Evidence: `shape/shape-vm/src/compiler/comptime_builtins.rs:21`.

3. **Comptime target introspection model is concrete.**
   - Function target object includes kind/name/params/return_type/annotations/captures.
   - Evidence: `shape/shape-vm/src/compiler/comptime_target.rs:24`.

4. **LSP integration for comptime workflows is unusually good for this stage.**
   - Comptime semantic tokens, hover, completion, and generated method flows are covered by integration tests.
   - Evidence: `shape/tools/shape-test/tests/lsp_comptime.rs:1`.

## Critical Gaps

1. **Runtime annotation context contract is incoherent.**
   - Current wrapper constructs `ctx` as a typed object with `{state, event_log}`.
   - Tests expecting `ctx.set(...)` fail with unknown method on TypedObject.
   - Evidence: `shape/shape-vm/src/compiler/functions.rs:531`, `shape/shape-vm/src/executor/tests/annotations.rs:145`.

2. **`on_define` is compiled but effectively not part of runtime lifecycle path.**
   - Annotation metadata stores `on_define_handler`.
   - Wrapper invocation path checks only `before/after`.
   - Evidence: `shape/shape-vm/src/bytecode.rs:874`, `shape/shape-vm/src/compiler/functions.rs:559`, `shape/shape-vm/src/compiler/functions.rs:783`.

3. **Meta-authoring friction from strict property resolution appears in annotation scenarios.**
   - Runtime-like dynamic property patterns are rejected when compile-time typing is unresolved.
   - This hits natural meta code such as `fn.name` and domain-style reflective access.
   - Evidence: `shape/shape-vm/src/compiler/expressions/property_access.rs:182`, failure trace in `shape/shape-vm/src/executor/tests/annotations.rs:404`.

4. **Phase model is powerful but under-specified.**
   - There is a real pipeline (`comptime pre -> compile/wrap -> comptime post`), but lifecycle and mutation guarantees are not codified as a user-facing contract.
   - Evidence: `shape/shape-vm/src/compiler/functions.rs:56`.

## Gap vs Desired “Rust Macros Without Macro Pain”

### Where Shape is already better
- No token-tree macro DSL burden.
- Direct AST-level target object available in comptime handlers.
- Better LSP continuity for many compile-time use-cases.

### Where it falls short
- Lifecycle hooks are not fully reliable (`ctx`, `on_define`).
- Inconsistency between documented and actual annotation APIs.
- Some compile-time transformations still feel constrained by runtime access rules.

## Gap vs Desired “Zig Comptime With Better Ergonomics”

### Where Shape is promising
- Dedicated comptime blocks and compiler messaging functions.
- Directive-based mutation avoids low-level metaprogramming ceremony.

### Where it falls short
- Comptime data model and annotation context semantics are not stable enough for routine advanced meta programming.
- Lack of clear, authoritative phase semantics increases mental overhead.

## Recommended Roadmap (Priority Order)

1. **Define and implement one annotation context model.**
   - Option A: `ctx` immutable-functional (`ctx2 = ctx.set(...)`).
   - Option B: `ctx` mutable API with clear internal mutation semantics.
   - Make tests and docs match one model.

2. **Wire `on_define` end-to-end or remove from public surface until done.**
   - Current half-state causes false expectations.

3. **Publish a formal lifecycle matrix.**
   - For each handler kind: execution phase, available target shape, allowed side effects, return contract.

4. **Add compile-time-friendly reflective access patterns for known meta objects.**
   - Keep strict typing, but make target metadata access ergonomic and guaranteed.

5. **Expand LSP coverage for annotation lifecycle edge cases.**
   - Add tests for `ctx` state persistence patterns and `on_define` completion/diagnostics.

## Immediate Validation Commands (already re-run)
- `cargo test -p shape-test --test lsp_comptime -- --nocapture` (passes)
- `cargo test -p shape-vm test_annotation_ctx_state_roundtrip -- --ignored --nocapture` (fails)
- `cargo test -p shape-vm test_annotation_arg_evaluation -- --ignored --nocapture` (fails)
- `cargo test -p shape-vm test_annotation_on_define_handler -- --ignored --nocapture` (fails)
