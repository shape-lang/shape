# Cluster #10 — traits W1-operator-coverage regression (30/34 of traits)

**HEAD:** `7877fc6b` (post-v0.3.2, pre-v0.3.3 fix-cycle).
**Source classification audit:** `docs/cluster-audits/v0.3-classification/traits.md` @ `82f049dd`.
**Discipline:** AUDIT-ONLY. No source/fixture changes. No commits.

---

## 1. Three minimal repros (run-verified at HEAD)

### Shape A — operator-overload `+/-/*/`/neg unresolved on user `impl T`

Fixture `traits::stress_operators::impl_add_for_custom_type` (verbatim):

```shape
type Vec2 { x: number, y: number }
impl Add for Vec2 {
    method add(other: Vec2) -> Vec2 {
        Vec2 { x: self.x + other.x, y: self.y + other.y }
    }
}
let a = Vec2 { x: 1.0, y: 2.0 }
let b = Vec2 { x: 3.0, y: 4.0 }
let c = a + b
c.x + c.y
```

Run (`cargo test -p shape-test --test traits impl_add_for_custom_type -- --nocapture`):

```
Expected run ok, got error: Some("Semantic error: Cannot infer types for
binary operation `Add`: operand types are `unknown` and `unknown`.
Strict typing requires both operands to have a known concrete type at
compile time. Add a type annotation to disambiguate.")
```

**Mechanism (confirmed by source-read):** the `a + b` IS dispatched
through `emit_operator_trait_call` (`binary_ops.rs:120-142`) — but that
helper unconditionally clears `last_expr_schema = None`,
`last_expr_type_info = None`, `last_expr_numeric_type = None` after
emitting `CallMethod`. The binding `let c = a + b` therefore receives
**no type info** — slot tracker has no `schema_id`, no `type_name`.
The FOLLOW-ON `c.x + c.y` then infers both operands as `unknown` and
the strict-typing-sweep error fires on the OUTER `Add`. (`impl_sub_for_
custom_type` reports the same `Add` error for the same reason — `c =
a - b` loses type, `c.x + c.y` fails.)

### Shape B — `Display` impls compile but `.to_string()` returns debug repr

Fixture `traits::stress_operators::display_trait_basic`:

```shape
type User { name: string }
trait Display { method display() -> string }
impl Display for User {
    method display() { "User:" + self.name }
}
let u = User { name: "Alice" }
u.to_string()
```

Run:

```
assertion `left == right` failed:
Expected 'User:Alice', got '{name: "Alice"}'
```

**Mechanism:** `expr.to_string()` in
`crates/shape-vm/src/compiler/expressions/function_calls.rs:2555-2602`
short-circuits to `BuiltinFunction::FormatValueWithMeta` UNLESS
`has_any_user_defined_method("to_string")` returns true
(`helpers.rs:4642`). The user registered `User::display`, NOT
`User::to_string` — so the short-circuit fires. `FormatValueWithMeta`
calls `builtin_format` (`vm_impl/builtins.rs:351-359`) which uses
`ValueFormatter::format_kinded` → produces the default debug repr
`{name: "Alice"}`. **`builtin_format` never invokes
`try_dispatch_display`**; only the `print()` path (W18.6, R8 W3
2026-05-24) calls `try_dispatch_display` (`builtins.rs:1409,
1469-1521`). Display-trait dispatch was never wired into the
`.to_string()` lowering.

### Shape C — trait method return-type not threaded into binop inference

Fixture `traits::stress_default::trait_default_method_used_when_not_overridden`:

```shape
type Widget { label: string }
trait Describable {
    method name() -> string;
    method describe() { "Object: " + self.name() }
}
impl Describable for Widget {
    method name() { self.label }
}
let w = Widget { label: "Button" }
w.describe()
```

Run:

```
Semantic error: Cannot infer types for binary operation `Add`:
operand types are `string` and `unknown`.
```

**Mechanism:** identical to Shape A — `self.name()` is a method call
dispatched via `CallMethod`. The CallMethod-emission site (same
`emit_operator_trait_call` family pattern via the StringConcatTyped
path's lookup, OR the generic method-call lowering) does not consult
the trait's declared method signature (`method name() -> string`) to
stamp `last_expr_type_info`. The `+` binop sees the LHS string literal
typed as `string` and the RHS call as `unknown` → strict-typing-sweep
fires with operand types `string` and `unknown`.

---

## 2. Root-cause hypothesis (single-locus driving 30 of 34)

Shapes A + C share a single root cause; Shape B is a sibling but
distinct.

**Shape A + C single locus** — `emit_operator_trait_call`
(`crates/shape-vm/src/compiler/expressions/binary_ops.rs:120-142`):

```rust
fn emit_operator_trait_call(compiler: &mut BytecodeCompiler, method_name: &'static str, op_span: Span) {
    // … CallMethod emit + operator_trait_dispatch_sites insert …
    compiler.last_expr_schema = None;        // <-- clears
    compiler.last_expr_type_info = None;     // <-- clears  ← FN-REG-CORRECTNESS
    compiler.last_expr_numeric_type = None;  // <-- clears
}
```

Every W1.1–W1.11 sub-cluster (Sub, Mul, Div, Mod, Neg, Not, Eq, Ord,
BitAnd/Or/Xor, Shl/Shr, Index) routes through this helper. The trait
method's declared return type (e.g. `method add(other: Vec2) -> Vec2`)
is known at compile time via `program.functions` / the trait
declaration in the inference env, but `emit_operator_trait_call`
neither receives a `return_type` argument nor consults the registered
function to stamp `last_expr_type_info`. Net effect: every chained use
of an operator-trait result fails inference at the next binop.

Shape C is the same locus generalized — trait method call return type
isn't threaded back into the binop inference. The `Eq` path at
`binary_ops.rs:815-822` DOES correctly stamp `bool` after the trait
call — proof the fix shape is "stamp the known return type"; that
pattern just wasn't applied to the general operator-trait dispatch
helper.

**Shape B distinct locus** — `compiler/expressions/function_calls.rs:2555`
short-circuit:

```rust
if (method == "to_string" || method == "toString")
    && !self.has_any_user_defined_method(method)
{
    // FormatValueWithMeta short-circuit fires; bypasses Display dispatch
}
```

W18.6 (commit `036144d5`, 2026-05-24) wired Display.display() → content
and added `try_dispatch_display` for the `print()` builtin
(`builtins.rs:1469`), but **never extended `builtin_format` (the
FormatValueWithMeta backing) to consult `try_dispatch_display` first**.
The fix shape is one of: (a) extend `builtin_format` to call
`try_dispatch_display` for TypedObject args before falling through to
`ValueFormatter::format_kinded`, OR (b) change the `to_string`
short-circuit guard to also skip when the receiver type implements
Display.

---

## 3. Bisect anchor

```
git log --oneline -- crates/shape-vm/src/compiler/expressions/binary_ops.rs
```

Top candidate commit (introduction of the type-clearing pattern):

- **`c6ec6f51` "Stage 2.5: dispatch operator traits via CallMethod,
  eliminate generic arith opcode emission"** (Apr 8 2026) — introduced
  the CallMethod-based operator-trait dispatch path. `git log -S
  'last_expr_type_info = None;' -- binary_ops.rs` doesn't list it
  (the pattern was already in the codebase elsewhere), so the
  type-clearing in `emit_operator_trait_call` has been present **since
  its introduction**. W1.1–W1.11 each ADDED a new operator to the
  helper's dispatch table over Apr–May 2026 without anyone noticing the
  return-type stamp was missing — the regression is therefore not a
  "bisect" but a "never-correct since c6ec6f51".

Shape B (W18.6 close, commit `036144d5` 2026-05-24): added
`try_dispatch_display` but only wired it into `print()`, never into
`builtin_format` / FormatValueWithMeta. Same "never-correct since
introduction" shape.

W17.3-4 per-container FieldType (`d748b5b1`, `a4b38c76`) is NOT
implicated — its diffs touch `type_schema/` and `compiler/` but not
`emit_operator_trait_call`'s body. Eliminated as anchor.

---

## 4. Affected subsystem (file:line)

| Shape | File | Line | Symbol |
|---|---|---|---|
| A + C | `crates/shape-vm/src/compiler/expressions/binary_ops.rs` | 120-142 | `emit_operator_trait_call` |
| A | `crates/shape-vm/src/compiler/expressions/binary_ops.rs` | 1097 | `Add` arm `if left_has_add` branch — calls `emit_operator_trait_call(self, "add", op_span)`, return-type not stamped |
| A | `crates/shape-vm/src/compiler/expressions/binary_ops.rs` | 1449, 1533, 1894 | non-Add operator arms — same `emit_operator_trait_call` invocation site |
| C | `crates/shape-vm/src/compiler/expressions/binary_ops.rs` | 807 | Eq path — stamps `bool` correctly at L815-822, **proof the stamp shape is the right fix**; needs generalization to all operator-trait arms |
| B | `crates/shape-vm/src/compiler/expressions/function_calls.rs` | 2555-2602 | `to_string` short-circuit; Display-trait redirect never wired |
| B | `crates/shape-vm/src/executor/vm_impl/builtins.rs` | 1528-1539 | `builtin_format` — never invokes `try_dispatch_display` (W18.6 wiring partial) |

Cross-reference: the W10 audit (commit `bbeac650` "v0.3 R2 Phase 3b W10
JIT user-type operator-trait dispatch — bytecode->MIR->JIT conduit
close") added the `operator_trait_dispatch_sites` HashMap so the JIT
could re-lift the dispatch decision. The bytecode-level dispatch
worked correctly at runtime; what's broken is the COMPILE-TIME type
threading for the result of the dispatch.

---

## 5. Sub-cluster name + size estimate

**Sub-cluster name:** `10-traits-W1-operator-coverage` (Shapes A + C
single-locus + Shape B sibling-locus).

**Size: M** (medium). Two coupled fix sites:

1. (A + C) Refactor `emit_operator_trait_call` to accept the trait
   method's declared return type and stamp it onto `last_expr_type_info`
   / `last_expr_schema` / `last_expr_numeric_type`. Look up the
   registered impl function in `program.functions` by name
   `"{TypeName}::{method_name}"` and read its declared return type from
   the trait declaration (mirrors the Eq path at L815-822). ~9 call
   sites updated (the Add explicit arm at L1097 + the centralized arms
   at L1449 / L1533 / L1894 + the Eq path which is already correct).
   Closes ~22 of 34 traits failures (Shapes A + C across W1.1–W1.11
   surface).

2. (B) Extend `builtin_format` to invoke `try_dispatch_display` for
   `Ptr(HeapKind::TypedObject)` args before falling through to
   `ValueFormatter::format_kinded`, OR equivalently change the
   `to_string` short-circuit to skip when the receiver type implements
   the Display trait. Closes ~8 of 34 (display_trait_* + named_impl_* +
   operator_and_display_on_same_type).

Estimated **30 of 34** total traits failures close on these two fixes.
Remaining 4: `impl_method_with_extra_param` (trait-method-named-`add`
collision with operator dispatch — separate audit), 2 SCOPE-RECLAIM
(V3-S5 ckpt cascade), `trait_method_returns_array_length`
(`items: any` dispatching `.length()` to DataTable builtin — separate
`any`-typed receiver dispatch bug).

---

## 6. Dependencies / cluster overlap

**Cluster #11 (variables_bindings, 20 tests) — INDEPENDENT.** That
cluster's root is width-typed locals (`i8`/`i16`/`u32` etc.) leaking
the tagged-wrapper `Object {"I8": Number(100)}` instead of projecting
to declared `-> int`. Mechanism: Q11/Q12 marshal-return path for
narrow-int return kinds — completely different locus from the
operator-trait dispatch helper. No code-touch overlap.

**Cluster #12 (enums equality `==/!=`) — PARTIAL OVERLAP, not
shared-fix.** Enum equality routes through `compile_typed_equality`
(`binary_ops.rs:610`) — same FILE, but the Eq-trait dispatch path at
L763-822 already stamps `bool` correctly when `has_eq_impl` is true.
Enums don't have a user `impl Eq for E { method eq … }`; they expect
auto-derived equality. The audit's enum-equality root cause is likely
"`type_implements_trait(&enum_name, "Eq")` returns false because no
auto-derive is registered for the enum type" — distinct from cluster
#10's return-type-stamping fix. Verifying this would require a
separate audit on the enums binary. Conservative call: **fix cluster
#10 does NOT close cluster #12.**

**No code-touch territory overlap** with #11 or #12 for the proposed
two-locus fix. Cluster #10 fix can land independently.

---

## Discipline confirmation

- Audit-only. No source / fixture changes. No commits.
- All three repros run-verified against HEAD `7877fc6b` (the
  `cargo test -p shape-test --test traits …` excerpts above are
  verbatim from live runs in this session).
- No `git stash` used.
- CLAUDE.md Forbidden Patterns: the proposed fix does NOT introduce
  any `ValueWord` / dynamic dispatch / `Convert<X>To<Y>` / SlotKind
  patterns. Stamping the trait method's declared return type onto
  `last_expr_type_info` uses existing `VariableTypeInfo::named(...)` +
  `schema_id` lookup machinery (exact pattern already in use at
  `binary_ops.rs:815-822` for Eq + at `function_calls.rs:2597-2599`
  for `.to_string`).
