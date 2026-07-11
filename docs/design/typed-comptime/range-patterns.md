# Range Patterns

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Pattern Constructor Catalog](pattern-constructor-catalog.md) | [Next: Decimal Domain](decimal-domain.md)

## Decision 83: Complete Range Bound Algebra

Accepted: a checked range pattern stores two independently typed semantic
bounds. Each bound is unbounded, includes its endpoint, or excludes its
endpoint:

```shape
sealed comptime enum RangeBound<T> {
    Unbounded,
    Included(ConstValue<T>),
    Excluded(ConstValue<T>),
}

sealed comptime struct CheckedRangePattern<T> {
    lower: RangeBound<T>,
    upper: RangeBound<T>,
}
```

The native spellings are:

| Pattern | Interval |
|---|---|
| `a..b` | `a <= value < b` |
| `a..=b` | `a <= value <= b` |
| `a<..b` | `a < value < b` |
| `a<..=b` | `a < value <= b` |
| `..b` | `value < b` |
| `..=b` | `value <= b` |
| `a..` | `a <= value` |
| `a<..` | `a < value` |

`<..` and `<..=` are dedicated range-pattern operators. They mark an excluded
lower endpoint; the ordinary `..` forms include their lower endpoint. The
upper `=` continues to mean inclusion, matching the established range
spelling.

**TARGET - every boundary combination is native**

```shape
match temperature {
    ..0.00d => below_freezing()
    0.00d..100.00d => liquid_range()
    100.00d<.. => above_boiling()
    _ => boundary_value()
}

match percentage {
    0u8<..=100u8 => nonzero_valid_percentage()
    0u8 => zero()
    _ => invalid()
}
```

Dense domains such as `decimal` have no general successor operation. An
excluded lower endpoint therefore cannot be implemented by incrementing the
endpoint or left available only through a typed builder. It is part of the
semantic algebra and has an equally direct source spelling.

**TARGET - generated bounds remain explicit and typed**

```shape
comptime fn interior<T>(
    domain: RangePatternDomain<T>,
    lower: ConstValue<T>,
    upper: ConstValue<T>,
) -> CheckedPattern<T, []> {
    patterns.range(
        domain,
        lower: RangeBound::Excluded(lower),
        upper: RangeBound::Excluded(upper),
    )
}
```

The compiler-issued `RangePatternDomain<T>` is evidence, not a user predicate
or claimed trait implementation. The builder cannot smuggle a comparator,
callback, source string, or untyped endpoint into the pattern.

**TARGET - fully unbounded range is not a second wildcard**

```shape
match value {
    .. => consume(value)
}
```

The pattern above is rejected with a diagnostic to use `_`. The semantic
catalog has one wildcard constructor, one hash, and one LSP meaning; `Range`
does not duplicate it with two unbounded endpoints.

Required rules:

1. Both lower and upper bounds use the exact admitted range domain `T` from
   Decision 82.
2. Inclusion and exclusion are semantic bound variants, not arithmetic
   rewrites. Inclusive maxima, exclusive minima, decimal scale, and `char`
   scalar gaps cannot overflow or require a successor/predecessor operation.
3. Native syntax, typed builders, frozen reflection, edits, coverage, hashes,
   diagnostics, and VM/MIR-JIT/OSR plans preserve the same two-bound meaning.
4. The `<..` marker applies only to the lower bound. The `=` marker applies
   only to the upper bound. Their position makes all four closed/open bounded
   combinations unambiguous.
5. An omitted lower or upper endpoint produces `Unbounded` on that side. Two
   omitted endpoints are rejected and normalize conceptually to `Wildcard`,
   which users spell `_`.
6. A range remains a binderless structural leaf. Binding the complete matched
   value is a later whole-value-alias decision, not an implicit property of
   interval syntax.
7. LSP hover renders both the source spelling and semantic inequalities.
   Completion after `<..` or `..` remains endpoint-type-aware.

Current migration implications:

- Pattern parsing needs dedicated `..`, `..=`, `<..`, and `<..=` handling;
  current expression-range parsing is not the semantic implementation.
- Internal interval coverage must retain inclusive/exclusive flags rather than
  normalizing through endpoint arithmetic.
- Typed builders and `FrozenPattern::Range` expose both bound variants from the
  first public comptime ABI.

## Decision 84: Context-Typed Constant Endpoints

Accepted: every included or excluded range endpoint freezes to one exact
`ConstValue<T>`. Native source can produce that value in exactly three ways:

1. A contextually typed literal.
2. An explicit `const PATH`.
3. A context-indexed `comptime {}` block.

```shape
match percentage {
    0u8..=100u8 => valid()
    _ => invalid()
}

match retry_count {
    const Limits::MIN_RETRIES..=const Limits::MAX_RETRIES => valid()
    _ => invalid()
}

match shard {
    comptime { first_owned_shard<Node>() }
        ..=comptime { last_owned_shard<Node>() } => local()

    _ => remote()
}
```

The expected range domain provides the literal or block's exact `T`. Ordinary
contextual literal typing remains available, including representability checks;
it is not numeric coercion or a runtime conversion.

The endpoint expansion capability is conceptually:

```shape
opaque comptime type RangeEndpointSink<T> {
    fn finish(value: ConstValue<T>) -> RangeBound<T>
}
```

`comptime {}` in this position may evaluate typed comptime code and tracked
capabilities, but its sole output is a closed `ConstValue<T>`. It cannot emit a
runtime expression, declaration, statement, pattern binder, comparator, or
other artifact into the endpoint.

**TARGET - computed const-generic endpoint**

```shape
comptime fn final_index<const N: usize>() -> ConstValue<usize> {
    comptime let endpoint: ConstValue<usize> = N - 1
    endpoint
}

match index {
    0usize..=comptime { final_index<N>() } => in_bounds()
    _ => out_of_bounds()
}
```

`N` must already be completely instantiated. The block makes evaluation and
lifting explicit; no symbolic endpoint or inference hole survives into the
checked pattern.

**TARGET - required rejections**

```shape
match count {
    minimum..maximum => ambiguous_names()
    runtime_min()..runtime_max() => runtime_endpoints()
    const U8_LIMIT..=const U16_LIMIT => mixed_types()
}
```

Bare identifiers remain pattern binders and therefore cannot occupy endpoint
slots. Calls outside `comptime {}` are runtime expressions. Mixed exact types
do not widen or coerce merely because their values are representable.

Required rules:

1. Every endpoint has the exact admitted `T` after transparent alias
   normalization. Runtime conversion, numeric widening, truthiness, and user
   conversion traits never participate.
2. A literal is checked under the expected `T` and must be representable. An
   explicitly suffixed literal naming another type is a type error.
3. `const PATH` is the only named-constant form. It resolves local, module,
   associated, or fully instantiated const-generic values and never falls back
   to binder or runtime-name interpretation.
4. A range-endpoint `comptime {}` receives the expected `T` and only the typed
   capabilities legal in its context. Its result freezes before pattern
   checking and its dependencies enter the expansion hash.
5. Returning `CheckedExpr<T>` is insufficient: an endpoint is a compile-time
   constant, not generated runtime evaluation. Declaration and module emission
   are also unavailable from this sink.
6. Endpoint evaluation failure is a compile-time diagnostic with both the
   endpoint site and comptime provenance. It cannot become a runtime pattern-
   match error.
7. Typed builders consume the same `ConstValue<T>` values and produce the same
   bound nodes; native syntax has no privileged untyped path.
8. LSP completion inside `const PATH` is constant-only and exact-type-aware.
   Inside `comptime {}`, it exposes the expected type, legal capabilities,
   dependency provenance, and ordinary typed diagnostics.

Current migration implications:

- Pattern parsing must distinguish endpoint slots from binder positions; it
  never resolves a bare endpoint identifier heuristically.
- The comptime engine and LSP gain a `RangeEndpointSink<T>` alongside the
  already accepted context-indexed sinks.
- Tests must cover contextual literal typing, overflow, exact named constants,
  computed endpoints, const-generic instantiation, illegal runtime expressions,
  illegal artifact emission, hashing, and generated/native parity.

## Decision 85: Structural Range Identity And Canonical Denotation

Accepted: checked pattern structure and the set of values it denotes are
separate typed compiler products.

A valid authored or generated range remains `FrozenPattern::Range`, even when
its denotation contains one value or the complete domain. Empty ranges fail
before a `CheckedPattern` can be published.

```shape
match value: u8 {
    7u8..=7u8 => singleton_range()
    0u8.. => full_domain_range()
}
```

Reflection sees two `Range` nodes with their exact semantic bound variants.
Coverage sees `{7}` and the complete `u8` domain respectively. Backend planning
may use an exact constant test for the first and no test for the second without
changing either reflected node.

The compiler-derived denotation is conceptually:

```shape
opaque comptime type CoverageSet<T>

comptime fn coverage<T, Bindings>(
    pattern: CheckedPattern<T, Bindings>,
) -> CoverageSet<T>
```

`CoverageSet<T>` is inspectable only through typed compiler queries. Users
cannot construct one, assert that it is complete, or supply custom set
arithmetic.

**TARGET - invalid intervals never become patterns**

```shape
match value: u8 {
    9u8..3u8 => reversed()
    7u8..7u8 => empty_equal_bounds()
    255u8<.. => empty_above_maximum()
}
```

All three are compile-time errors. Validation uses the exact topology of the
sealed `RangePatternDomain<T>`, including finite minima/maxima and adjacent
values. It never waits for runtime comparison or backend execution to discover
that an arm cannot match.

**TARGET - equivalent denotations retain editable structure**

```shape
match value: u8 {
    0u8..1u8 => first_spelling()
    0u8..=0u8 => second_spelling()
}
```

Each arm retains its authored `Range` bounds in `FrozenPattern`. Their
`CoverageSet<u8>` values are equal, so ordinary arm-order analysis reports the
second as unreachable. A transform may still inspect or edit either range by
typed bound cursor; the compiler does not silently substitute `Const(0u8)`.

Two hashes make the distinction explicit:

1. The checked-structure hash includes the `Range` tag, exact `TypeRef<T>`,
   range-domain identity, bound tags, and canonical endpoint values.
2. The denotation hash identifies the normalized `CoverageSet<T>` used by
   exhaustiveness, reachability, and reusable backend plans.

Consequently, `0u8..1u8` and `0u8..=0u8` have different checked-structure
hashes but the same denotation hash. `1.0d..=2.0d` and `1.00d..=2.00d` have the
same checked-structure hash because decimal endpoint values canonicalize and
the bound variants are identical. Literal, `const PATH`, native fragment,
builder, and synonym origins never enter either semantic hash.

**TARGET - atomic typed edit**

```shape
let edit = checked_pattern.edit()

edit.replace_lower(
    range_cursor,
    RangeBound::Excluded(new_lower),
)

let updated = edit.finish()
```

`finish()` validates root/revision ownership, exact type and domain, endpoint
ordering, domain topology, and non-emptiness before publication. Editing an
installed arm remains match-owned under Decision 79, so coverage, reachability,
effects, ownership, hashes, and backend plans update atomically with it.

Required rules:

1. Reversed and mathematically empty intervals are hard compile-time errors.
   No public `EmptyPattern` or match-nothing `Range` exists.
2. Equal endpoints are valid only when both are included. The resulting node
   remains `Range` with singleton denotation.
3. Exact domain topology decides emptiness beyond endpoint ordering. An open
   interval between adjacent integer or `char` values is empty even though its
   lower endpoint compares less than its upper endpoint.
4. A range covering the complete finite domain remains `Range`. Only the
   endpoint-free `..` spelling is rejected as a duplicate spelling of `_`.
5. `FrozenPattern::Range` exposes canonical endpoint values and semantic bound
   variants, never parser nodes, source text, constant names, or builder origin.
6. Source spelling and expansion ancestry live in provenance queries keyed by
   owner-bound identities. They remain available to diagnostics and LSP but do
   not affect semantic hashes.
7. Coverage union, intersection, difference, completeness, and residual
   calculations operate only on canonical `CoverageSet<T>` values.
8. Backend lowering consumes the checked pattern plus compiler-derived
   denotation plan. It may optimize singleton or complete ranges without
   rewriting the reflected pattern or independently deriving interval meaning.
9. Native syntax and typed generation producing the same bound structure and
   canonical values have identical checked-structure hashes. Extensionally
   equivalent but structurally different patterns share only the denotation
   hash.
10. LSP hover may show source spelling, semantic inequalities, and normalized
    coverage together. Virtual expansion documents are read-only and preserve
    bidirectional provenance even when backend planning simplifies the test.

Current migration implications:

- The pattern checker produces a persistent checked `Range` descriptor and a
  separate canonical coverage query instead of destructively normalizing the
  constructor catalog.
- Compiler caches distinguish structure-sensitive comptime expansion from
  denotation-sensitive coverage and backend work.
- Proofs cover reversed, equal, adjacent, singleton, full-domain, native versus
  builder, equivalent-denotation/different-structure, atomic edit, LSP, and
  VM/MIR-JIT/OSR plan-parity cases.
