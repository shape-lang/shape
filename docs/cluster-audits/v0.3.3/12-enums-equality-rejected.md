# Cluster #12 — Enum equality `==`/`!=` rejected at compile time

**HEAD:** 82f049dd (post-v0.3.2; same head as classification source)
**Source cluster:** `docs/cluster-audits/v0.3-classification/enums.md` Group B (32 tests, FN-REG-CORRECTNESS).
**Class:** FN-REG-CORRECTNESS — sub-cluster size S–M (32 tests).

## 1. Minimal repro (run-verified)

```shape
enum Color { Red, Green, Blue }
fn main() {
    let a = Color::Red
    let b = Color::Red
    print(a == b)
}
```

Actual at HEAD `82f049dd` (via `direnv exec /home/dev/dev/shape-lang cargo run --bin shape -- run /tmp/enum_eq_repro.shape`):

```
Error: Runtime error: Bytecode compilation failed: Semantic error: Cannot infer types for binary operation `Equal`: operand types are `unknown` and `unknown`. Strict typing requires both operands to have a known concrete type at compile time. Add a type annotation to disambiguate.
```

Expected: prints `true`. The 32 Group-B failures all reduce to this shape (same-enum operands, unit or struct/tuple payload, via locals or function-return values).

## 2. Root cause

`compile_typed_equality` at `crates/shape-vm/src/compiler/expressions/binary_ops.rs:610-833` dispatches in four tiers and rejects everything else:

1. `None`-literal desugar to `IsNull` (L623-646).
2. Scalar typed opcodes for `int / number / decimal / string / bool` (L664-694) via `EqOperandType` enum at L203 — **enum types have no variant in `EqOperandType`** so `resolve_eq_type` (L837) returns `None` for both operands.
3. Cross-numeric literal coercion (L724-761) — N/A for enums.
4. **W1.7 user-`impl Eq for X` dispatch** (L777-824, `6990b1de` 2026 W1.7 commit). Guarded by `self.type_inference.env.type_implements_trait(name, "Eq")` at L795 / L800.

Path 4 is the only opening for user types. `EnumDef` registration at `crates/shape-runtime/src/type_system/environment/registry.rs:530-533::register_enum` inserts the enum into `enum_defs` but **never registers an `impl Eq for <EnumName>`** — no auto-derive, no synthetic impl. So `type_implements_trait("Color", "Eq")` returns `false` and `compile_typed_equality` falls through to the strict-typing hard error at L831-832 (`strict_typing_binop_error`).

The two failure-text shapes from `enums.md` Group B are the same gap surfaced via two diagnostic paths:

- `'unknown' and 'unknown'` — `resolve_eq_type` returned `None/None` (most enums via untyped locals).
- `'Concrete(Reference(TypePath { … Color }))'` — inference *did* resolve `Reference(Color)` but neither `EqOperandType` nor the `Eq`-trait check accepts it.

W1.7 (`docs/v0.3-close-summary.md:189`) shipped `Eq` only as an opt-in trait for `type X { … }` user-defined structs. Enums were not covered — enum equality requires either (a) auto-deriving `impl Eq` for every registered `EnumDef` (variant-tag compare + recursive payload compare), or (b) extending `EqOperandType` with an `Enum` variant + emitting a discriminant-compare opcode.

## 3. Bisect anchor

```
$ git log --oneline -- crates/shape-runtime/stdlib-src/core/eq.shape \
                       crates/shape-vm/src/compiler/expressions/binary_ops.rs \
                       crates/shape-runtime/src/type_system/inference/
```

Relevant commits:

- `84446d82` — `v0.3 R2 Phase 3b W1.7 — Eq operator trait for user-defined types` (introduces `compile_typed_equality` Path 4 user-trait dispatch — shipped opt-in for `type X`, **not** for `enum X`).
- `6990b1de` — `Merge w1-7-eq` (W1.7 landing; cited at `docs/v0.3-close-summary.md:189`).
- `39ff7bdb` — `WS-8 — kind-generic header handlers + string/bool eq + decimal MIR + map-result type propagation` (added `Bool` to `EqOperandType` Path 2 fast-path; did not touch enums).
- `e447d181` — `R8 W7 match enum-payload tuple-binder type-inference` (enum-payload inference fix in `match`, distinct subsystem).

W1.7 is the regression-introducing commit class — it tightened the dispatch from "fall through to dynamic Eq/Neq opcodes" to "hard-error if no recognized typed path", but did not extend coverage to enums. Pre-W1.7 the `EqDynamic/NeqDynamic` fallback would have handled enum equality via runtime variant compare; post-W1.7 (strict-typing sweep Phase 1) that fallback is deleted by design (L826-832 comment).

## 4. Affected subsystem

- `crates/shape-vm/src/compiler/expressions/binary_ops.rs:610-833` — `compile_typed_equality`. Specifically:
  - L203 `enum EqOperandType` — needs `Enum(String)` variant (or equivalent discriminator) for Path-2 fast-emit.
  - L777-824 — W1.7 user-trait dispatch only matches when `type_implements_trait(_, "Eq")` is true; enums never satisfy this.
- `crates/shape-runtime/src/type_system/environment/registry.rs:530-533` — `register_enum` does not synthesize an `Eq` impl. Either fix here (auto-derive at registration time) or fix at the binary_ops dispatch site (treat `EnumDef` membership as proof-of-`Eq`).
- `crates/shape-runtime/src/type_system/environment/mod.rs:1210-1211` — `register_enum` wrapper on `TypeEnvironment`, same gap.

Fix shape (lowest-risk): in `compile_typed_equality`, after L803, add a fourth check — if `self.type_inference.env.get_enum(name).is_some()` for both operand type-names, emit a typed enum-discriminant comparison (analogous to Path 2's `EqInt` on tag bytes for unit variants, with structural recursion required for payload variants — but unit variants alone cover the majority of Group-B tests). For the variant-payload case, the right shape is auto-derive `impl Eq for <EnumName>` at `register_enum` time and let Path 4 dispatch handle it.

## 5. Sub-cluster + size

- **Sub-cluster name:** `enum-eq-dispatch-gap` (W1.7 follow-up).
- **Size:** S–M. 32 tests in `enums.md` Group B. The fix is single-site (binary_ops `compile_typed_equality` + `register_enum` auto-derive) — bounded blast radius. Variant-payload comparisons add structural recursion complexity but reuse the existing W1.7 trait-dispatch plumbing.

## 6. Dependencies

- **Cluster #10 traits-W1 (related, NOT shared root):** Same operator-trait dispatch *shape* as W1.7 — both flow through `emit_operator_trait_call` and `type_implements_trait`. The fix-pattern is identical (extend `type_implements_trait` semantics for enums, or pre-register synthetic `Eq` impls). If cluster #10 is "trait method dispatch on user types broken", the bytecode/MIR/JIT conduit (`bbeac650` W10) is shared; if cluster #10 is "Eq trait dispatch on `type X` broken", we are deeper-coupled. Either way, both clusters touch the same `binary_ops.rs:777-824` code region — coordinate fix sequencing to avoid merge conflicts.
- **Cluster #3 enum-discriminant panic / wire_conversion (DISTINCT root):** Cluster #3 is the `wire_conversion.rs:201` panic family (also surfaces in `modules_visibility.md`, `closures_hof.md` S9). That cluster is a *runtime* slot-kind ↔ HeapValue mismatch (TypedObject vs Decimal); cluster #12 is a *compile-time* type-inference rejection that never reaches runtime. Group-D in `enums.md` (2 tests: `basics_decl::test_enum_unit_variant_definition`, `basics_programs::enum_unit_variants_declaration`) is cluster #3 territory and is NOT in cluster #12's 32-test set. Different subsystems, different fixes, fix-order independent.
- **B2 EnumPayload preflight (§5.16 follow-up):** Explicitly out of scope per `enums.md` Group-B header. Group-C (27 tests, match-arm payload binding losing type) is B2 territory and routes to V0.4-DEFER under the §5.16 dated authorization.
