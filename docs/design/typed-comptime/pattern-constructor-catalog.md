# Pattern Constructor Catalog

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Guards And Exhaustiveness](guards-and-exhaustiveness.md) | [Next: Range Patterns](range-patterns.md)

## Decision 80: Core Structural Pattern Basis

Accepted, as amended by Decisions 82, 87, 90, 91, and 92: the sealed
`FrozenPattern<T, Bindings>` catalog has these distinct semantic variants:

```shape
sealed comptime enum FrozenPattern<T, Bindings> {
    Wildcard(...),
    Binder(...),
    Const(...),
    Variant(...),
    Record(...),
    Newtype(...),
    Tuple(...),
    Range(...),
    Sequence(...),
    Alias(...),
    ExactDynamicType(...),
    Not(...),
    AllOf(...),
    UnionMember(...),
    Or(...),
}
```

Range semantics are specified by Decisions 82-85, homogeneous sequence
semantics by Decisions 87-89, whole-place alias semantics by Decision 90,
exact dynamic refinement by Decision 91, and restricted Boolean composition by
Decision 92. General conjunction of multiple binding producers remains
rejected rather than hidden behind flags, callbacks, or binder unification.

**TARGET - nested nominal structure**

```shape
match event {
    Event::Click {
        position: Point { x, y },
        button: MouseButton::Left,
    } => handle_click(x, y)

    Event::Close => stop()
}
```

`Event::Click` and `Event::Close` are `Variant` nodes. `Point` is a `Record`
node. `MouseButton::Left` is another `Variant`, not a string or runtime equality
test. Every member and constructor is a compiler-issued identity.

**TARGET - closed-union member refinement**

```shape
match value {
    number: int => normalize_int(number)
    text: string => normalize_text(text)
}
```

Each `UnionMember` carries proof that its selected type is a member of the
closed scrutinee union. Coverage is the exact member subset, and the binder has
the refined member type plus the mode from Decision 76.

**TARGET - compatible alternatives**

```shape
match cached {
    Present(value) | Cached(value) => use(value)
    Missing => fallback()
}
```

`Or` unions structural coverage. Both alternatives introduce the same
hygienic binder identity with the same type, final mode, guard view, and
compatible ownership footprint.

**TARGET - fixed products remain distinct**

```shape
match coordinate_and_color {
    [Point { x, y }, [red, green, blue]] => render(x, y, red, green, blue)
}
```

Expected type distinguishes the heterogeneous fixed tuple from the nested
homogeneous fixed-array sequence. Reflection preserves `Tuple` versus
`Sequence` and its exact fixed-array domain; neither is erased into a generic
positional list node.

**TARGET - newtype remains nominal**

```shape
match order_id {
    OrderId(value) => load_order(value)
}
```

`Newtype` uses its visible constructor identity. It never matches by pretending
that `OrderId` is the underlying integer or string.

Required variant semantics:

1. `Wildcard<T>` covers all `T`, creates no binder, and commits no projected
   ownership change.
2. `Binder<T, Mode>` covers all `T` and creates one hygienic binder with the
   stable mode and footprint from Decision 76.
3. `Const<T>` denotes one compiler-checked canonical constant. It never invokes
   runtime equality. Decision 81 defines the exact constant domain.
4. `Variant<T, V>` uses an exact enum/variant descriptor. Coverage is the
   variant tag combined with the product coverage of its payload patterns.
5. `Record<T>` uses owner-bound visible field descriptors. Omitted fields are
   structural wildcards; unknown dynamic fields do not exist.
6. `Newtype<T, U>` uses its nominal constructor and one underlying subpattern.
   Visibility and representation authority remain enforced.
7. `Tuple<Ts>` preserves heterogeneous element types and exact arity.
8. `Range<T>` preserves an admitted exact ordered domain plus two typed bounds;
   Decisions 82-85 define its structure and canonical denotation.
9. `Sequence<Container, Element>` preserves a compiler-issued homogeneous
   sequence domain, exact/minimum length structure, ordered children, and at
   most one rest slot under Decision 87.
10. `Alias<T>` gives one checked child pattern a hygienic name for the same
    root place without creating another ownership claim under Decision 90.
11. `ExactDynamicType<Domain, Concrete>` tests one reified concrete execution
    type identity inside an open erased domain under Decision 91.
12. `Not<T>` complements one binder-free, zero-footprint pattern constraint
    under Decision 92.
13. `AllOf<T, Bindings>` intersects constraints while allowing at most one
    binding/commit producer under Decision 92.
14. `UnionMember<Union, Member>` is legal only with compiler-issued membership
   evidence for a closed union. Open-world type sets cannot use this coverage.
15. `Or<T, Bindings>` requires every alternative to expose identical binder
    identities, types, final modes, guard capabilities, and compatible
    footprints. Coverage is their normalized union.

Cross-cutting guarantees:

- Native syntax and typed builders produce these same variants.
- Every variant has a typed frozen descriptor and typed child cursors.
- Coverage, effect-free probing, atomic commit, ownership, source maps,
  expansion hashes, and VM/JIT lowering are variant-defined and mandatory.
- Public/private visibility applies to source matching. Comptime generation
  additionally needs `RepresentationAccess<T>` for privileged structure.
- LSP completion enumerates legal variants, fields, union members, tuple slots,
  range endpoints, and sequence domains/positions from semantic descriptors.
- A new primitive variant is unavailable until all contracts above plus
  positive/negative source, comptime, VM/JIT, and LSP proofs land together.

**TARGET - required rejections**

```shape
match order_id {
    value: int => load_order(value)
}

match open_plugin {
    plugin: dyn Renderer => render(plugin)
}
```

The first attempts transparent newtype matching. The second requests open trait
conformance rather than one exact concrete identity. Diagnostics direct users
to the visible newtype constructor, an exact concrete refinement plus catch-all
under Decision 91, or an explicit runtime classifier result.

## Decision 81: Canonical Constant Patterns

Accepted: `FrozenPattern::Const<T>` is a scalar leaf denoting one canonical
equality singleton. Its equality is sealed compiler semantics and never invokes
runtime `Eq`, user trait dispatch, generic comparison, or numeric coercion.

The initial leaf domains are:

- `bool`.
- Every exact signed and unsigned integer width, including `int`.
- `char` as one Unicode scalar value.
- `string` as one exact code-point/byte sequence with no implicit Unicode
  normalization.
- `decimal` under canonical numeric equality.
- Non-NaN floating scalar values under exact-type IEEE equality.

**TARGET - literals remain native**

```shape
match token {
    "+" => Operator::Add
    "-" => Operator::Subtract
    _ => Operator::Unknown
}

match flag {
    true => enabled()
    false => disabled()
}
```

The string and boolean leaves have exact scrutinee types. The boolean pair is
exhaustive because the compiler knows that complete finite domain.

**TARGET - floating values preserve ordinary equality**

```shape
match value {
    candidate where candidate.is_nan() => handle_nan()
    0.0 => handle_zero()
    _ => handle_other()
}
```

`+0.0` and `-0.0` normalize to the same singleton because ordinary IEEE
equality considers them equal; a second signed-zero arm is unreachable.
Positive and negative infinity are legal exact values. NaN is rejected as a
`Const` because ordinary equality gives it no singleton; it remains an explicit
effect-typed guard predicate.

**TARGET - named constants are explicit**

```shape
match retries {
    const Limits::MAX_RETRIES => stop()
    current => retry(current)
}
```

A bare identifier in pattern position always declares a binder. `const PATH`
resolves a local, module, associated, or fully instantiated const-generic value
and gives LSP an unambiguous constant-completion context.

**TARGET - structural constant normalization**

```shape
const ORIGIN: Point = Point { x: 0, y: 0 }

match point {
    const ORIGIN => snap()
    _ => preserve(point)
}
```

Source accepts the structural constant ergonomically, then normalizes it to the
same `Record` plus child `Const` tree as `Point { x: 0, y: 0 }`. Unit variants,
payload variants, newtypes, records, tuples, and homogeneous arrays similarly
lower through their Decision-80 constructors rather than surviving as opaque
constant equality nodes.

**TARGET - generated explicit lifting**

```shape
comptime let maximum: ConstValue<u8> = 255u8

plan.arm(patterns.const(maximum)) {
    handle_maximum()
}
```

`patterns.const` is the pattern-position `ConstLift`. It accepts a frozen
`ConstValue<T>`, applies the same scalar-or-structural normalization, and never
parses source or captures a runtime value.

**TARGET - required rejections**

```shape
match value {
    const number::NAN => handle_nan()
}

match integer_value {
    1.0 => wrong_numeric_coercion()
}

match resource {
    const DEFAULT_CONNECTION => reuse()
}
```

NaN has no equality singleton. A `number` constant cannot match an `int`
scrutinee through coercion. Resources, references, handles, functions, secrets,
grants, and opaque identities cannot define structural singleton coverage.

Required rules:

1. The scrutinee and constant have the same frozen semantic type. Numeric
   widening, truthiness, representation casts, and user conversions do not
   participate.
2. Integer equality is exact in the declared width. Boolean equality is exact
   and cannot alias an integer bit pattern.
3. Character equality uses Unicode scalar identity. String equality uses exact
   content with no locale or normalization policy.
4. Decimal constants normalize to canonical numeric value, including canonical
   zero, before coverage and hashing.
5. Non-NaN floats use IEEE equality with signed zeros unified. NaN constants
   are compile errors; NaN classification is a guard.
6. `Const` never consults user `Eq`, `PartialEq`, hash, comparator, derive, or a
   claimed purity/totality proof.
7. A visible structural constant recursively lowers to sealed constructors and
   leaf constants. Its normalized tree, coverage, hash, backend plan, and LSP
   view equal the corresponding directly written structure.
8. A named constant cannot launder private or opaque representation. Ordinary
   source visibility applies, and privileged comptime lowering additionally
   requires `RepresentationAccess<T>`.
9. Const-generic values must be completely instantiated and frozen before
   pattern construction. No symbolic inference hole reaches `Const`.
10. Canonical hashing includes the `Const` tag, exact `TypeRef<T>`, and canonical
    scalar payload. Structural constants hash as their normalized trees, not
    their declaration names or source spellings.
11. Coverage records one singleton for every admitted value and full finite
    coverage for complete boolean alternatives.
12. VM, MIR JIT, and OSR JIT consume a dedicated exact-typed constant-test
    operation. No backend may substitute generic equality, mixed numeric
    comparison, bitwise dynamic comparison, or an interpreter-only deopt as the
    semantic implementation.

Current migration implications:

- The current broad literal AST plus narrower ad hoc pattern opcode table is
  replaced by semantic `Const` normalization and validation.
- Named `const PATH` patterns are added; bare identifiers remain binders.
- Existing float patterns become exact-typed and reject NaN; mixed int/number
  behavior is a compile error rather than backend-dependent coercion.
- Char, decimal, string, bool, integer, float, named-constant, structural-
  normalization, hashing, exhaustiveness, and VM/MIR-JIT/OSR parity proofs land
  as one feature surface.

## Decision 82: Sealed Range Pattern Domains

Accepted: `FrozenPattern::Range<T>` exists only when the compiler can issue
sealed evidence for one exact, language-defined ordering of `T`. The initial
domains are:

- Every exact signed and unsigned integer type, including `int`.
- `char`, ordered by Unicode scalar value.
- `decimal`, ordered by canonical numeric value.

Type aliases normalize transparently before domain selection. Nominal newtypes
do not inherit a range domain merely because their representation has one;
their visible constructor must contain the underlying range pattern.

**TARGET - primitive ordered domains**

```shape
match response_code {
    200..300 => ok()
    300..=399 => redirect()
    _ => other()
}

match character {
    '0'..='9' => digit()
    _ => other()
}

match amount {
    0.00d..1000.00d => small()
    _ => large()
}
```

Decisions 83-84 define these bound spellings and typed endpoint forms. Native
syntax and builders normalize to the same sealed `Range` node and exact domain
evidence.

**TARGET - nominal structure stays visible**

```shape
match order_id {
    OrderId(1000..=1999) => reserved()
    _ => normal()
}
```

`OrderId(1000..=1999)` is a `Newtype` node containing an integer `Range` node.
The direct pattern `1000..=1999` cannot match an `OrderId`, even when its
runtime representation is an integer.

**TARGET - required domain rejections**

```shape
match ratio { 0.0..1.0 => unit_interval() }
match name { "A".."Z" => initial() }
match flag { false..true => any_flag() }
match phase { Phase::Queued..Phase::Done => active() }
```

Floating ranges are deferred because NaN is outside ordinary floating order
and would make interval coverage non-total. Strings have no implicit language
ordering for patterns. Boolean ranges obscure their finite constant domain.
Enum declaration order is not a semantic ordering contract.

Required rules:

1. Range eligibility is compiler-sealed. User `Ord`, `PartialOrd`, `Eq`,
   comparator functions, coercions, derives, and claimed purity or totality do
   not create a range domain.
2. The scrutinee, endpoints, coverage set, and backend comparison plan share
   one exact semantic type after transparent alias normalization.
3. Integer ordering is the mathematical order of the declared exact width;
   signedness and width never coerce during matching.
4. Character ordering is Unicode scalar-value order. Surrogate code points are
   not holes requiring runtime treatment because they are not `char` values.
5. Decimal ordering uses canonical numeric values, so alternate scales for the
   same number do not create distinct boundaries.
6. Floating, string, boolean, enum, function, reference, resource, authority,
   opaque, and open-world dynamic types have no initial range-pattern domain.
7. A newtype is matched structurally through a visible `Newtype` constructor
   containing a range over its representation. Privileged generation still
   requires `RepresentationAccess<T>` for non-public structure.
8. The semantic node is a pattern interval, not a runtime `Range` value. Its
   probe cannot allocate, dispatch user code, suspend, or invoke runtime range
   membership.
9. Const-generic endpoints are eligible only after complete instantiation into
   an admitted exact domain; symbolic type or value holes cannot enter a
   checked range pattern.
10. Adding another admitted domain changes the language and comptime ABI and
    requires exact coverage, hashing, VM/MIR-JIT/OSR, diagnostics, and LSP
    semantics together.

Current migration implications:

- Current expression and loop ranges are not reused as the semantic pattern
  representation; their `Int64` runtime path and coercive loop specialization
  have different contracts.
- Parser, typed pattern checking, interval coverage, dedicated backend
  comparisons, comptime reflection, LSP support, and proofs are all new work.
- Integer support may be implemented as the first tracer slice, but the public
  semantic design reserves `char` and `decimal` as equally valid initial
  domains rather than deriving the language contract from current runtime
  limitations.
