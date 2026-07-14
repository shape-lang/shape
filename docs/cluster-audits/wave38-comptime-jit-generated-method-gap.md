# Wave-38D Comptime/JIT Generated-Method Residual Gap Scout

Date: 2026-07-10

Worker: Wave-38D comptime/JIT generated-method residual gap scout

Scope: static investigation only. I read the AGENTS resource/build policy and did
not run cargo, just, nextest, rustc, build, tests, or book-truth commands. I used
`rg`/`sed` for inspection and wrote only this report.

## Executive Answer

The residual `showcases::to_json_serializes_via_stdlib_import_jit` failure is
most likely a JIT generated-method dispatch/lowering gap, with a comptime
generated-method MIR gap as the enabling cause. It is unlikely to be a stdlib
import problem, and unlikely to be primarily an f-string escaping problem.

The failing showcase is the only stdlib showcase in
`tools/shape-test/tests/annotations_comptime/showcases.rs` that generates an
extension method instead of a free function:

- `@json_schema` imports `std::serde::derive`, emits `User_json_schema()`, and
  passes under JIT.
- `@llm_tool` imports `std::llm::tools`, emits `get_weather_tool_def()`, and
  passes under JIT.
- `@to_json` imports `std::serde::serialize`, emits
  `extend User { method to_json() -> string { ... } }`, then calls
  `u.to_json()`. The reported JIT result is `Null`.

That pattern points at generated method handling, not annotation import.

## Static Evidence

### Generated methods are compiled differently from generated free functions

`apply_comptime_extend_items` already has the Wave-37/earlier free-function JIT
fix: generated free functions use the full `compile_function(func_def)` path so
`Function.mir_data` is attached for MirToIR:

- `crates/shape-vm/src/compiler/functions_annotations.rs:1738`
- `crates/shape-vm/src/compiler/functions_annotations.rs:1787`

Generated methods still go through the bytecode-only body compiler in
`apply_comptime_extend`:

- `crates/shape-vm/src/compiler/functions_annotations.rs:1388`
- `crates/shape-vm/src/compiler/functions_annotations.rs:1416`

Normal source `extend` items do not have this asymmetry. The ordinary second
pass compiles desugared extend methods with `compile_function`:

- `crates/shape-vm/src/compiler/statements.rs:1798`

So the residual is specific to comptime-generated methods.

### Extend methods are dot-named, but the JIT runtime method fallback looks for `::`

`desugar_extend_method` names extend methods as `Type.method`:

- `crates/shape-vm/src/compiler/statements.rs:2513`
- `crates/shape-vm/src/compiler/statements.rs:2595`

But `jit_call_method`'s user-method fallback constructs only
`TypeName::method_name`:

- `crates/shape-jit/src/ffi/call_method/mod.rs:323`
- `crates/shape-jit/src/ffi/call_method/mod.rs:350`

For a `Ptr(HeapKind::TypedObject)` receiver, the JIT-format builtin branch
returns `TAG_NULL` and then tries that user-method fallback:

- `crates/shape-jit/src/ffi/call_method/mod.rs:1047`
- `crates/shape-jit/src/ffi/call_method/mod.rs:1197`

If the fallback misses `User.to_json`, the method call returns the `TAG_NULL`
placeholder. That matches the reported `Null` symptom.

### MIR method lowering does not preserve the bytecode compiler's direct UFCS choice

The bytecode compiler can resolve `receiver.method()` to a direct `Call` of
`Type.method` / `Type::method` when it sees a user method:

- `crates/shape-vm/src/compiler/expressions/function_calls.rs:5020`
- `crates/shape-vm/src/compiler/expressions/function_calls.rs:5211`
- `crates/shape-vm/src/compiler/expressions/function_calls.rs:5265`

MIR lowering, however, lowers ordinary method calls generically as
`MirConstant::Method(method)`:

- `crates/shape-vm/src/mir/lowering/expr.rs:2553`
- `crates/shape-vm/src/mir/lowering/expr.rs:2614`

The JIT only bypasses `jit_call_method` when the method call appears in
`monomorphized_method_call_sites`:

- `crates/shape-jit/src/mir_compiler/terminators.rs:325`

For a non-generic generated `User.to_json(self) -> string`, specialization can
return `None` because the generated method already has concrete `self` and return
annotations. That leaves the JIT on the generic runtime method path, where the
dot/colon mismatch above can return `Null`.

### F-string/self capture is probably a covered subcase, not the root

The stdlib generator emits Shape source with runtime f-string interpolation over
`self.id` and `self.name`:

- `crates/shape-runtime/stdlib-src/serde/serialize.shape:55`

The VM test passes, so the generated source, field introspection, escaping, and
annotation import path are coherent enough for the interpreter. The JIT may still
need a regression that exercises `self` field reads inside a generated f-string,
because the existing green generated-method flagship only returns a constant:

- `tools/shape-test/tests/comptime/flagship_wf3d.rs:129`

But the observed `Null` is better explained before entering the generated method
body: JIT method dispatch either cannot resolve `User.to_json`, or resolves a
bytecode-only method with no native function-table entry and silently falls back
to `TAG_NULL`.

## Root-Cause Hypothesis

Primary hypothesis:

1. The comptime post handler successfully generates `extend User { method
   to_json() -> string { ... } }`.
2. Pass-1/pre-pass registers the method signature, so VM and bytecode compilation
   can see it.
3. Pass-2 compiles the generated method with `compile_function_body`, leaving
   `Function.mir_data` missing for the JIT.
4. MIR for `u.to_json()` still lowers as `MirConstant::Method("to_json")`, not as
   a direct `Function("User.to_json")` call.
5. `jit_call_method` sees a typed-object receiver. Its JIT-format builtin branch
   produces `TAG_NULL`; its user-method lookup tries `User::to_json`, not
   `User.to_json`; it therefore returns the placeholder.
6. The top-level JIT frame writes/returns `TAG_NULL`, which the harness observes
   as `Null`.

Secondary hypothesis to test after the primary fix:

- Once the method resolves and carries MIR, native compilation of the generated
  body may expose a separate f-string/self-field carrier issue. If so, the next
  failure should no longer be silent `Null`; it should be a compile-stage JIT
  bail or a focused string/typed-object field-read failure.

## Smallest Implementation Lane

Recommended first lane: JIT generated-method parity, owned by one worker.

Primary files:

- `crates/shape-vm/src/compiler/functions_annotations.rs`
- `crates/shape-jit/src/ffi/call_method/mod.rs`
- Optional narrow `crates/shape-vm/src/compiler/expressions/function_calls.rs`
  only if the worker chooses compile-time side-table routing instead of runtime
  fallback lookup.

Tests:

- `tools/shape-test/tests/annotations_comptime/showcases.rs`
- Optional focused no-stdlib regression in
  `tools/shape-test/tests/comptime/flagship_wf3d.rs` or
  `tools/shape-test/tests/annotations_comptime/on_define.rs` that generates a
  method returning an f-string with `self` field reads under JIT.

Suggested patch shape:

1. In `apply_comptime_extend`, compile generated methods through
   `compile_function(&func_def)` instead of `compile_function_body(&func_def)`,
   mirroring `apply_comptime_extend_items` for generated free functions. Keep the
   existing pre-registered-signature skip so the method is not registered twice.
2. Teach `try_call_user_method` to resolve both impl-style `Type::method` and
   extend-style `Type.method`. If a candidate name exists but its function-table
   entry is null, set `pending_call_error` for deopt instead of silently returning
   `None` and allowing `TAG_NULL` to escape.
3. If runtime fallback feels too late, alternatively record the direct
   bytecode-resolved user-method target into a JIT routing side table when
   `compile_expr_method_call` emits the direct `Call`. That is semantically
   cleaner but likely touches more compiler plumbing.

Do not start in `crates/shape-runtime/stdlib-src/serde/serialize.shape`; the
stdlib source is probably the reproducer, not the cause.

## Verification Plan

First supervisor command, after implementation:

```bash
systemd-run --user --wait --collect --pipe -p MemorySwapMax=0 -p MemoryMax=12G -p TasksMax=256 env CARGO_BUILD_JOBS=2 cargo test -p shape-test --test annotations_comptime showcases::to_json_serializes_via_stdlib_import_jit -- --exact --nocapture
```

Then run the paired VM/JIT showcase filter and the focused generated-method
flagship/on-define regressions under the same cgroup envelope. If those pass,
run the broader annotations-comptime suite as a supervisor-owned lane because
Wave-37C found this gap only in that broader sweep.

No verification commands were run by this scout.
