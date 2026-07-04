# A-final ROOT C — namespaced-call return mis-typed as the MODULE

**Verdict:** `FP_fix_checker` (valid code, correct at runtime, over-rejected by the strict checker)

**Baseline:** strict-flip worktree `shape-strict-flip-collection-dispatch` @ `f01e8323`
(let-gen landed; ROOT A cleared). Binary:
`/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape`

## Failing tests cleared

- `stdlib_modules::crypto_tests::crypto_random_bytes_length`
- `stdlib_modules::crypto_tests::crypto_random_bytes_zero`
- `stdlib_crypto::hashing::crypto_hmac_sha256_produces_64_hex_chars`

All three share the shape `crypto::fn(...)` followed by a member access
(`.length()`) on the result.

## Reproduction (run-verified verbatim on the strict-flip binary)

Program reconstructed from `crypto_random_bytes_length`:

```shape
use std::core::crypto
let bytes = crypto::random_bytes(16)
print(bytes.length())
```

```
$ target/release/shape run /tmp/root_c_1.shape
Error: Runtime error: Bytecode compilation failed: Semantic error: Type constraint violation: Concrete(Reference(TypePath { segments: ["crypto"], qualified: "crypto" })) cannot have fields
```

Identical rejection for the `hmac_sha256` program (`let mac = crypto::hmac_sha256("data","key"); let len = mac.length(); print(len)`).

### Cross-check: same program on the main / non-strict binary PASSES

```
$ /home/dev/dev/shape-lang/shape/target/release/shape run /tmp/root_c_1.shape
... (JIT surface-and-stop note, falls to interpreter) ...
32
```

The non-strict binary runs the program and prints the correct length. The
strict-flip binary rejects it. The strict flip did not introduce the defect —
it un-suppressed a pre-existing type-inference mis-typing.

### Cross-check: the rejection requires the member access

```shape
use std::core::crypto
print(crypto::random_bytes(16))     # COMPILES + RUNS on strict-flip → prints the hex bytes
```

The bare `crypto::random_bytes(16)` is accepted on strict-flip; only adding
`.length()` (member access on the result) triggers the rejection. This proves
the call RESULT is typed as `Reference("crypto")` (the module), and the
`.length()` member access is what surfaces the bad type.

Generalizes across the module: `crypto::sha256("x").length()` rejects the same way.

## Root cause / the seam

`crypto::random_bytes(16)` is type-inferred as
`Type::Concrete(TypeAnnotation::Reference("crypto"))` — i.e. the MODULE itself —
instead of the function's `string` return type. The subsequent `.length()`
member access pushes a `HasField("length", _)` constraint against that
`Reference("crypto")` type; the constraint solver's `HasField` arm has no
`Reference` case, so it falls to the `_ =>` arm and errors
"…Reference(crypto) cannot have fields".

### Seam 1 — qualified-call inference (the producer of the bad type)

`crates/shape-runtime/src/type_system/inference/expressions.rs:149-184`
(`Expr::QualifiedFunctionCall`). When the namespace is NOT an enum / value
binding / struct / type-alias / `DateTime`|`Content`, control reaches the
`else` arm:

```rust
// expressions.rs:178-183
} else {
    for arg in args {
        self.infer_expr(arg)?;
    }
    Ok(Type::Concrete(TypeAnnotation::Reference(namespace.as_str().into())))   // ← line 182: returns the MODULE name as the call's value type
}
```

For a module-qualified call (`crypto::random_bytes`) the engine has NO signature
for the callee — `predeclare_item` treats `Item::Import` as a no-op
(`inference/items.rs:34 _ => Ok(())`), and `register_known_bindings`
(`inference/mod.rs:227-237`) registers the namespace only as a bare fresh type
variable, never its exports. So the qualified call has nothing to resolve a
return type from and the arm returns the namespace `Reference` as the value
type — which is wrong: the value of a CALL is its return value, never the module.

(Sibling line 163 has the structurally identical
`Ok(Reference(namespace))` for the enum-tuple-constructor case; that one is
legitimate — an enum constructor's value really is of the enum type. Only the
module-call `else` arm at line 182 is wrong.)

### Seam 2 — where the bad type detonates (symptom site, do NOT "fix" here)

`crates/shape-runtime/src/type_system/constraints.rs:819-863`
(`TypeConstraint::HasField`). The `_ =>` arm at lines 859-862 emits the verbatim
error. This is a correct, conservative arm — patching it to accept
`Reference(_)` would re-introduce a dynamic/structural escape hatch and mask
real "x cannot have fields" bugs. Leave it.

## Why FP (not TP)

The code is valid and runs correctly under the existing runtime: the bytecode
compiler resolves module-namespaced calls through its OWN schema registry
(`compile_module_namespace_call`, scoped name `crypto::random_bytes`,
`function_calls.rs:1700-1738`) and produces the right `string` result — hence
the non-strict binary prints `32`/`64`. The rejection is purely an
inference-side mis-typing the strict flip stopped suppressing
(`should_emit_type_diagnostic` now returns `true` for all errors —
`compiler_impl_initialization.rs:863-867`). Strict typing should accept this
program; the checker is wrong, so fix the checker.

## Minimal fix (exact edit)

Stop the module-call inference arm from returning the namespace as the value
type. Return a fresh, unconstrained result type variable instead — matching how
the engine already handles an unresolved method result (the `HasMethod`
fallback at `expressions.rs:614` returns a fresh var). A fresh var lets the
later `.length()` push its `HasField` against an unresolved variable, which the
solver tolerates (it only `check_constraint`s once a type resolves concretely —
`constraints.rs:768`), so compilation proceeds. No concrete type is fabricated
(no dynamic fallback, no coercion, no `ValueWord`-class shim) — the result is
simply "unknown here", which is the truthful inference-tier statement given the
engine has no module-export signatures.

File: `crates/shape-runtime/src/type_system/inference/expressions.rs`
Arm: `Expr::QualifiedFunctionCall` `else` branch, lines 178-183.

```rust
            } else {
                for arg in args {
                    self.infer_expr(arg)?;
                }
                // A module-qualified call's value is its RETURN value, never the
                // module. The inference tier has no module-export signatures
                // (Item::Import is a no-op in predeclare_item; known bindings are
                // bare fresh vars), so the precise return type is unknown HERE —
                // return a fresh result var rather than the namespace Reference,
                // which would wrongly type the result as the module and reject any
                // member access on it. The bytecode compiler resolves the real
                // signature via its module schema registry. No concrete type is
                // fabricated.
                Ok(self.fresh_type_var())
            }
```

(One-line essence: replace
`Ok(Type::Concrete(TypeAnnotation::Reference(namespace.as_str().into())))`
at expressions.rs:182 with `Ok(self.fresh_type_var())`.)

### Note for the implementer

`crypto` is collected into `known_bindings`
(`collect_namespace_import_bindings`, `compiler_impl_initialization.rs:4-16`)
and registered by `with_known_bindings` →
`register_known_bindings` (a bare fresh var). When that registration is in
effect, the lookup at expressions.rs:164 succeeds and control takes the
synthetic-`MethodCall` branch (169-177), whose `HasMethod` fallback already
returns a fresh var — so that path is already FP-safe. The empirically observed
`Reference("crypto")` result proves the operative path is the `else` arm at
182 (the call reaches inference with `crypto` unregistered as a value). Editing
line 182 fixes the operative path; the synthetic branch needs no change. If a
future change re-routes module calls through the synthetic branch, that branch
is already correct, so the single-line edit is sufficient and not fragile.

A more precise (larger, optional, NOT required to clear these FPs) fix would
teach the inference engine the module's export signatures (mirror the bytecode
compiler's `__mod_{namespace}` schema registry) so `crypto::random_bytes`
infers `string` directly. That is a feature-level enhancement, out of scope for
clearing this FP root; the fresh-var edit is the minimal correct change.

## Files the fix touches (for conflict-grouping)

- `crates/shape-runtime/src/type_system/inference/expressions.rs` (single arm, expressions.rs:178-183)

No change to `constraints.rs` (symptom site, leave conservative).
No test-baseline change (these are FPs to be cleared, not re-baselined).
