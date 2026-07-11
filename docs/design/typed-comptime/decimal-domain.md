# Decimal Domain

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Range Patterns](range-patterns.md) | [Next: Sequence Patterns](sequence-patterns.md)

## Decision 86: Arbitrary-Precision Exact Decimal

Accepted: Shape `decimal` denotes an arbitrary-precision number with a finite
base-10 expansion. Its semantic value is a canonical coefficient and
nonnegative scale:

```text
coefficient * 10^(-scale)
```

The coefficient has no language-level width limit. Canonicalization removes
trailing decimal zeros, canonicalizes every zero to one positive zero, and
uses the smallest nonnegative scale representing the same numeric value.

```shape
1.0d == 1.00d
-0.000d == 0d
```

Both comparisons are true. Equality, ordering, constant coverage, hashing,
artifact identity, wire values, and snapshots operate on the canonical numeric
value rather than literal scale or runtime storage.

**TARGET - exact values do not inherit a host crate's limits**

```shape
let tiny = 0.0000000000000000000000000000000000000001d
let huge = 9999999999999999999999999999999999999999d

assert(tiny > 0d)
assert(huge + 1d > huge)
```

These values are valid subject only to ordinary compile-time or runtime
resource budgets. A 96-bit coefficient, scale 28, 16-byte host representation,
or third-party crate version is not part of Shape's decimal semantics.

**TARGET - dense range topology**

```shape
match amount {
    1.20d<..1.21d => interior()
    1.20d | 1.21d => boundary()
    _ => outside()
}
```

For any two distinct Shape decimals `a < b`, another representable decimal
exists strictly between them. Therefore:

- `lower > upper` is reversed and invalid.
- Equal endpoints denote a singleton only when both bounds are included;
  every other equal-endpoint interval is empty and invalid.
- When `lower < upper`, all four included/excluded combinations are nonempty.
- No bounded decimal range covers the complete domain because the domain is
  unbounded in both directions.

No decimal successor or predecessor operation participates in range checking,
coverage, hashing, or backend lowering.

**TARGET - fixed precision is a nominal library constraint**

```shape
comptime {
    emit(finance.fixed_decimal_type(
        name: #Money,
        precision: 19,
        scale: 4,
    ))
}

let price: Money = Money.checked(12.3400d)?
```

This is an illustrative typed-library generator, not a built-in SQL or finance
feature. A library may provide a reusable nominal `FixedDecimal<P, S>` instead.
Either form owns checked construction, rounding policy, serialization adapters,
and domain operations without changing core `decimal` identity or silently
converting arbitrary values.

Matching a constrained nominal preserves Decision 82's visible structure:

```shape
match price {
    Money(0d<..) => positive()
    _ => other()
}
```

The exact generated constructor syntax is library-owned. The core rule is that
the nominal wrapper contains a `decimal` range; it does not acquire transparent
range semantics.

Required rules:

1. Shape decimal values are exactly the finite base-10 expansions, with an
   arbitrary signed coefficient and arbitrary nonnegative scale.
2. Canonical equality strips trailing zeros and unifies signed/scaled zeros.
   Canonical ordering is ordinary exact numeric ordering with no NaN or
   infinity values.
3. Decimal is unbounded and dense as a semantic domain. Resource budgets may
   bound one evaluation, but they do not change which values the type denotes.
4. Literals, `ConstValue<decimal>`, constant patterns, and range endpoints all
   use the same canonical value. Literal spelling and presentation scale remain
   provenance only.
5. Hashes and portable encodings use a canonical sign/coefficient/scale form.
   They never hash host object bytes, allocation identity, or crate-specific
   serialization.
6. Runtime representation is private to an execution ABI. VM and JIT may use
   inline small-decimal optimization plus heap-backed large values, but every
   tier observes the same exact semantics.
7. Pattern comparison is a sealed exact decimal intrinsic and performs no user
   dispatch or allocation. Backends consume the shared checked compare plan.
8. Fixed precision, fixed scale, currency, database column constraints, and
   rounding policies are nominal typed-library abstractions or generated types,
   not parameters secretly attached to core `decimal`.
9. Conversions between unconstrained and constrained decimal types are
   explicit and checked. Representation transparency cannot bypass validation.
10. LSP renders canonical value/type information while preserving literal
    spelling and generated nominal provenance. It never derives semantics from
    a host decimal library.

Execution-ABI consequences:

- The current 16-byte `rust_decimal::Decimal` carrier is transitional and does
  not define the language. Replacing it changes the exact Execution ABI ID.
- Wire, snapshot, content-addressed constants, function blobs, FFI adapters,
  and remote peers move to one canonical arbitrary-length decimal encoding.
- Native C and extension boundaries use explicit decimal ABI descriptors and
  adapters rather than exposing a Rust struct layout.
- JIT fast paths may specialize small canonical coefficients, but deopt or
  helper calls preserve exact results and never round into the old carrier.

Deliberately unresolved here:

- Division of values whose quotient has no finite decimal expansion.
- Rounding contexts and rounding-mode types.
- Resource-budget failure for very large arithmetic.
- The concrete stdlib interface for fixed precision/scale nominal types.

Those are separate user-visible decisions. This decision forbids inheriting
implicit rounding, overflow, panic, or scale behavior from the current host
crate while they remain unresolved.

Current migration implications:

- Replace parser, AST literal, constant, runtime value, MIR/JIT carrier, wire,
  snapshot, serde, FFI, and test assumptions tied to `rust_decimal`.
- Introduce a canonical decimal value module owned by Shape semantics, with a
  portable encoding independent of runtime layout.
- Keep a temporary adapter to the current carrier only behind explicit checked
  conversions; it cannot remain the semantic source of truth.
- Add cross-tier and cross-process proofs for canonical equality, order, hashes,
  arbitrary precision, dense ranges, ABI rejection, and deterministic resource
  accounting before decimal range patterns are enabled.
