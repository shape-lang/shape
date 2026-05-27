# Cluster #11 — variables_bindings width-typed locals leak tagged wrapper

**HEAD:** 70507224 (post-`docs(v0.3): R8 W9 G.2 Step 2 close`).
**Source binary:** `tools/shape-test/tests/variables_bindings/stress_let_basic.rs`.
**Audit-only.** No source / fixture changes. No commits. No `git stash`.
**Per-binary classification:** `docs/cluster-audits/v0.3-classification/variables_bindings.md` (TRUTH-SET cluster #4, 20 tests; this audit
addresses the 19 width-typed-locals + 3 compile-time-gate sub-cluster
within that count).

## 1. Minimal repro

```shape
fn ret_i8() -> int { let x: i8 = 100; return x }
ret_i8()
```

Expected return: `WireValue::Integer(100)` rendered as `100`.
Actual return: `WireValue::I8(100)` rendered as `Object {"I8": Number(100)}`
(externally-tagged serde encoding of the `WireValue::I8(i8)` variant —
`shape-wire/src/value.rs:43`).

Detector: `tools/shape-test/src/shape_test.rs:269` panic
`Expected number, got: Object {"I8": Number(100)}`. `extract_number`
recognises `serde_json::Value::Number`, `{"Integer": n}`, and `{"Number": n}`
projections (lines 256-270) — width-typed projections (`I8`/`I16`/`I32`/
`U8`/`U16`/`U32`/`U64`) were never wired.

Companion sub-cluster — compile-time width gates regressed
(`test_width_i8_overflow_compile_error`, `test_width_u8_negative_compile_error`,
`test_width_u16_overflow_compile_error`):

```shape
fn test() { let x: i8 = 128; return x }   // 128 > i8::MAX, should be compile error
```

Now compiles + executes; the per-binary audit shows the test harness
materialises `Object {"Bool": Bool(false)}` (the `expect_run_err` negative
assertion fires on a successful run that returns a bool sentinel).

## 2. Root-cause hypothesis

The wire-projection at the program-return host boundary
(`crates/shape-vm/src/execution.rs:557-568`) takes the completion
`KindedSlot` straight off the VM stack and feeds `(bits, kind)` into
`shape_runtime::wire_conversion::slot_to_wire` (`crates/shape-runtime/
src/wire_conversion.rs:44`) using **the slot's runtime kind**, not the
function's declared `-> int` return type. The slot kind at function
return is the storage kind of the `let x: i8` binding (`NativeKind::Int8`),
so `slot_to_wire` lines 57-65 dispatch:

```rust
NativeKind::Int8  => WireValue::I8(bits as i8),
NativeKind::Int16 => WireValue::I16(bits as i16),
NativeKind::Int32 => WireValue::I32(bits as i32),
NativeKind::UInt8 => WireValue::U8(bits as u8),
... etc.
```

This is **ADR-006 §2.7.5 producer-side stamp behaving correctly** — the
slot does carry kind `Int8`, the wire-projection is faithful. What is
missing is a **return-type widening coercion in the function-return
projection**: when the declared return is `int`, the i8/i16/.../u64
slot must be re-stamped to `NativeKind::Int64` (sign-extended) /
`NativeKind::UInt64` (zero-extended) at the `return` opcode emission
site **or** the host-boundary projector must consult the program's
declared return signature and widen.

Sister symptom — compile-time width-overflow gate disappearance — is
plausibly the same producer-side regression family: integer-literal
type-inference now stamps the literal's kind from the binding
annotation (so `let x: i8 = 128` stamps `128` as `Int8` and silently
wraps to `-128` or similar) without first validating the literal's
unsigned/signed value fits the width's range. The negative-on-unsigned
gate (`let x: u8 = -1`) and large-overflow (`65536` into `u16`) both
fail the same way.

This is a **single regression family** affecting both the wire-emit
path AND the literal-range gate. Sub-cluster #5 estimate stands.

## 3. Bisect anchor

Recent commits touching width-type handling
(`git log --oneline -- crates/shape-value/src/native_kind.rs
crates/shape-runtime/src/type_schema/ crates/shape-runtime/src/
wire_conversion.rs`):

- `32b9caaa` R5c-2-β-γ (b) **complete the u64 full-range carrier path** —
  added `WireValue::U64` projection; this is the commit that introduced
  the dedicated wire variants currently leaking.
- `68099161` R5c-2-β-γ (c) **JIT narrow-integer arithmetic wraps
  two's-complement** — width-type runtime semantics.
- `9a62dc4b` R5c-2-β-γ (a) **VM exact wrapping i64 arithmetic**.
- `bca3778f` **u64-literal kind-inference in u64 binary-op context** —
  literal-kind-from-annotation stamping; **prime suspect for the
  compile-time-gate regression** (`fix(vm)`: this stamping now propagates
  to other widths without the range-check counterpart).
- `c2825f93` **disambiguate u64-scalar from v2-typed-array carrier
  (CKPT-C)** — touched wire carrier discrimination.
- `aefe77e5` R5b-2 bool/null sentinel cluster — added `NativeKind::Null`
  + `slot_to_wire` Null projection; touched `wire_conversion.rs:75`
  region. Not a direct cause but is the closest neighbour edit.

The first commit that landed the dedicated `WireValue::I8`/`I16`/`I32`/`U8`/
`U16`/`U32`/`U64` projections without a return-widening coercion is the
bisect anchor — most likely `32b9caaa` plus the wave that authored the
`WireValue` width variants (`shape-wire/src/value.rs:43-57`; git-log on
that file would pinpoint within one bisect step). Audit-only — not run.

## 4. Affected subsystem (file:line)

Primary fix site — function-return wire projection:

- `crates/shape-vm/src/execution.rs:557-568` — program-return host
  boundary; takes completion `KindedSlot` and feeds raw kind into
  `slot_to_envelope` / `slot_to_wire` without consulting the program's
  declared return signature.
- `crates/shape-runtime/src/wire_conversion.rs:57-65` — `slot_to_wire`
  width-variant arms; faithful to the §2.7.5 stamp, so the fix
  is **upstream of this call**, not in this function.

Fix shape (two equally valid options):
1. **Compiler-side (preferred):** the `return` opcode emission for a
   width-typed slot in a function whose declared return type is `int`
   inserts a sign/zero-extension widening op that re-stamps the slot
   kind to `Int64`/`UInt64`. Symmetric to existing `let a: int = x_i8`
   widening at binding assignment.
2. **Host-boundary-side:** `execution.rs:557-568` consults
   `program.entry_function_return_type` (or equivalent) and calls a
   widening helper before `slot_to_envelope`. Less invasive but
   leaks the discipline outside ADR-006 §2.7.5 (kind IS the
   discriminator) — option 1 is more correct.

Companion fix site — compile-time width-overflow gate:

- Integer-literal kind-inference site (likely
  `crates/shape-runtime/src/type_system/inference/` near the
  `bca3778f` u64-literal kind-inference edit). The literal-from-
  annotation stamping must call a range-check on the literal's
  `i128`/`u128` value against the target width's `MIN..=MAX` range
  before stamping. Negative literals on unsigned widths surface as
  the same class.

## 5. Sub-cluster name + size estimate

**Sub-cluster name:** `width-typed-return-projection-and-compile-gate`

**Size:** S (one return-projection fix + one literal-range-check fix
likely closes all 19+3 = 22 tests in the per-binary doc — although TRUTH-
SET cluster #4 counts the family as 20 because 2 entries overlap with
SCOPE-RECLAIM / destructuring in the same binary). The return-projection
fix alone closes the 19 plain-width tests; the literal-range-check fix
closes the 3 compile-error tests.

## 6. Dependencies

**Cluster #3 (`wire_conversion` panic on enum-discriminant mismatch)** —
**touches same file** (`crates/shape-runtime/src/wire_conversion.rs`) but
**distinct root cause** (enum-tag drift between producer-stamped
`HeapKind::Enum` payload and consumer's expected schema). Fixes are
independent; no shared edit. The two clusters share the same single
audit-and-fix file for code review economy.

**Cluster #4 (`pointer-as-float` silent-wrong-output, `regression:1`
bug5 returning 2.08e-322)** — **distinct symptom, related family.**
That cluster's root cause is heap-pointer bits being projected through
`NativeKind::Float64` (kind-mis-stamp at producer site); this cluster's
root cause is correct-kind-stamp at producer + faithful-projection at
consumer, but missing function-return widening. Both are in the
ADR-006 §2.7.5 stamp-discipline family. A unified §2.7.5 audit pass
would catch both, but the fixes do not share code.

**No overlap with NaN-boxing** — strict-typing post-W-series deleted
`ValueWord`; the `Object {"I8": ...}` shape is **serde external-tagged
encoding of a `WireValue::I8(i8)` enum variant**, NOT a NaN-box leak.
This is critical: the prompt mentions "NaN-box/wire-layer" but the
correct framing is "wire-layer projection missing return-type widening".
No NaN-box involvement.

**Test-harness adjacency** — `tools/shape-test/src/shape_test.rs:256-270`
`extract_number` could be extended to accept width-typed projections,
which would mask the bug. **DO NOT.** The wire-layer regression is
real and the harness assertion is correct: `fn() -> int` must project
to a single canonical `int`-shaped wire value at the host boundary.

## Discipline footer

- Run-verify binding: `slot_to_wire` line-numbers, `execution.rs`
  line-numbers, and `WireValue` variant definitions were read at
  HEAD `70507224`. Per-binary classification doc
  (`docs/cluster-audits/v0.3-classification/variables_bindings.md`)
  was read at the same HEAD.
- No source / fixture changes. No commits. No `git stash`.
- No CLAUDE.md Forbidden Patterns triggered — the proposed fix lives
  inside ADR-006 §2.7.5 producer-side stamp discipline (option 1)
  or the §2.7.4 host-boundary projector (option 2).
- No defection-attractor framings used; the symptom is named by the
  underlying mechanism (function-return widening missing) not by a
  "bridge / probe / helper" rename.
