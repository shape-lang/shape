# Strict-Negative Suite — Category: not-callable-not-indexable

Negative type-safety acceptance programs. Each program is **code a strict type
system is REQUIRED to reject** (CLAUDE.md §Type System Rules: "If the type can't
be proven, it is a compile error... no escape hatch"; §Forbidden Patterns: "No
dynamic fallback"). The programs are top-level (script mode does not auto-invoke
`fn main`), so the offending expression executes if the compiler lets it through.

**Error modes covered:**
- Calling a non-function value `x()` where `x` is a scalar (`int` / `number` / `bool`).
- Indexing a non-indexable value `x[0]` where `x` is a scalar (`int` / `bool` / `number`).
- Member access on a scalar `x.field` where `x` is a scalar (`int` / `bool`).

**Classification rubric (CURRENT behavior — a measurement of today's breach):**
- `REJECTS_CLEAN` = ec != 0, clean type/semantic error, no value printed, no crash.
- `LEAKS_RUN` = ec == 0, program executed/printed (often a reinterpreted pointer). THE BREACH.
- `CRASHES` = panic / SIGABRT / SIGSEGV (ec ~134/139) instead of a clean compile-reject. ALSO a breach.

> **Note on "REJECTS_CLEAN" here:** every rejection in this category happens at
> **RUNTIME** (`Error: Runtime error: ...`), not at compile time. A strict type
> system is required to reject these at **COMPILE** time. They still classify as
> REJECTS_CLEAN under the observed-behavior rubric (ec!=0, nothing printed, no
> crash), but the fact that they reach the runtime at all is the underlying
> defect. Programs 04 and 07 escalate past clean rejection entirely.

Binary: `./target/release/shape run --mode {vm,jit} <file>`.

## Per-program results

| # | File | Error mode | VM | JIT | agree | Observed |
|---|------|-----------|----|----|-------|----------|
| 01 | `not-callable-not-indexable_01.shape` | call `int` value `x()` | REJECTS_CLEAN | REJECTS_CLEAN | yes | Both ec=1, `Runtime error: call_value_immediate_nb: callee must be ... got Int64 (line 5)`. JIT deopts to interpreter (`[jit-fallback]`) then surfaces same error. Nothing printed. |
| 02 | `not-callable-not-indexable_02.shape` | call `number` value `x()` | REJECTS_CLEAN | REJECTS_CLEAN | yes | Both ec=1, `Runtime error: call_value_immediate_nb: ... got Float64 (line 5)`. Nothing printed. |
| 03 | `not-callable-not-indexable_03.shape` | call `bool` value `b()` | REJECTS_CLEAN | REJECTS_CLEAN | yes | Both ec=1, `Runtime error: call_value_immediate_nb: ... got Bool (line 5)`. Nothing printed. |
| 04 | `not-callable-not-indexable_04.shape` | index `int` `x[0]` | REJECTS_CLEAN | **CRASHES** | no | VM ec=1 `Runtime error: TypeError: expected object, array, string, or other heap value, got scalar (line 5)`. JIT **ec=139 Segmentation fault (dumped core)** — deterministic across reruns. THE BREACH: JIT reinterprets the int scalar as a heap pointer and dereferences it. |
| 05 | `not-callable-not-indexable_05.shape` | index `bool` `b[0]` | REJECTS_CLEAN | REJECTS_CLEAN | yes | Both ec=1, `Runtime error: TypeError: expected object, array, string, or other heap value, got scalar (line 5)`. Nothing printed. |
| 06 | `not-callable-not-indexable_06.shape` | index `number` `x[1]` | REJECTS_CLEAN | REJECTS_CLEAN | yes | Both ec=1, `Runtime error: TypeError: ... got scalar (line 5)`. Nothing printed. |
| 07 | `not-callable-not-indexable_07.shape` | member access `int` `x.value` | REJECTS_CLEAN | **LEAKS_RUN** | no | VM ec=1 `Runtime error: TypeError: ... got scalar (line 5)`. JIT **ec=0, printed `-1407374883553280`** (the int scalar reinterpreted as a heap pointer / garbage) — deterministic across reruns and independent of field name (`.value`, `.foo` both leak). THE BREACH. |
| 08 | `not-callable-not-indexable_08.shape` | member access `bool` `b.field` | REJECTS_CLEAN | REJECTS_CLEAN | yes | Both ec=1, `Runtime error: TypeError: ... got scalar (line 5)`. Nothing printed. |

## Why each must reject (type rule)

- **01/02/03 (call scalar):** Only function types (closures, `ModuleFn`) are
  callable. `int` / `number` / `bool` are scalar value types with no call
  signature. CLAUDE.md §Language Features lists callables as `fn`/closures/
  polyglot fns only; calling a scalar has no typed opcode and cannot be proven
  via `prove_native_kind()`. A strict type system must reject at compile time —
  there is "no escape hatch" (§Type System Rules: No `any` type).
- **04/05/06 (index scalar):** Index syntax `x[i]` is only valid on indexable
  heap types — `Array<T>` (→ `TypedArray<T>`), `String`, `HashMap<K,V>`.
  Scalars (`int`/`number`/`bool`) carry no `data` pointer or length; the VM's own
  message names this: "expected object, array, string, or other heap value, got
  scalar". A strict type system must reject indexing a scalar at compile time.
  Program 04 shows the catastrophic consequence of not doing so: the JIT treats
  the `int` bit-pattern as a heap pointer and dereferences it -> SIGSEGV. This is
  exactly the §Value Representation invariant being violated — `NativeKind` is
  "stamped at compile time... never fabricated from raw bits", and `as_heap_value()`
  is unsound on non-`Ptr` slots (ADR-006).
- **07/08 (member access on scalar):** Field/member access `x.field` requires the
  receiver to be a `TypedObject`/struct with a compile-time field offset
  (§Value Representation: TypedStruct "C-compatible fixed layout with compile-time
  field offsets"). Scalars have no fields. A strict type system must reject
  `int.value` / `bool.field` at compile time. Program 07 shows the JIT
  reinterpreting the scalar's bits as a heap pointer and printing garbage
  (`-1407374883553280`) — a silent type-confusion leak, the precise breach the
  strict-typing plan exists to eliminate.

## Tally (16 runs = 8 programs x 2 modes)

- REJECTS_CLEAN: 13
- LEAKS_RUN: 1 (program 07, JIT)
- CRASHES: 1 (program 04, JIT — SIGSEGV)

**VM/JIT divergence:** 2 of 8 programs disagree (04, 07). In both, the VM
runtime-rejects cleanly while the JIT either crashes (04) or leaks a
reinterpreted pointer (07). The VM never leaks or crashes in this category, but
*every* program in this category is rejected only at runtime — a strict type
system must reject all 8 at compile time.
