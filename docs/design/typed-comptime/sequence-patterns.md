# Sequence Patterns

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Decimal Domain](decimal-domain.md) | [Next: Sequence Coverage](sequence-coverage.md)

## Decision 87: Unified Homogeneous Sequence Algebra

Accepted: one sealed `FrozenPattern::Sequence` constructor represents every
structurally matchable homogeneous ordered sequence. `FixedArray` is no longer
a separate frozen pattern variant; fixed and dynamic containers are distinct
compiler-issued sequence domains under one algebra.

The public shape is conceptual:

```shape
opaque comptime type SequencePatternDomain<Container, Element>

sealed comptime enum SequenceRest<Segment, Bindings> {
    None,
    Ignore,
    Binder(some<Mode> PatternBinder<Segment, Mode>),
}

sealed comptime struct CheckedSequencePattern<
    Container,
    Element,
    Bindings,
> {
    domain: SequencePatternDomain<Container, Element>,
    prefix: PatternPack<Element>,
    rest: SequenceRest<_, _>,
    suffix: PatternPack<Element>,
}
```

Decision 88 fixes the exact segment type and ownership mode for a bound rest.
They cannot be erased to `Any` or selected later by backend behavior.

Initial admitted domains are:

- Dynamic `Array<T>`.
- Fixed `Array<T, N>` with exact const length `N`.
- The first-class homogeneous segment/view type required by rest ownership.

Shared and exclusive scrutinee access derive binder modes from Decision 76;
they do not create user-extensible sequence domains. Nominal wrappers must be
matched through visible `Newtype` or `Variant` structure containing a sequence.

**TARGET - exact and minimum lengths**

```shape
match values {
    [] => empty()
    [only] => singleton(only)
    [first, second] => pair(first, second)
    [head, ...tail] => longer(head, tail)
}
```

For a dynamic array, a pattern without `...` has exact length. A pattern with
`...` has minimum length equal to its prefix plus suffix count. The example's
last arm therefore covers every remaining sequence of length at least two.

**TARGET - prefix, suffix, and infix rest**

```shape
match path {
    [root, ...parents, leaf] => nested(root, parents, leaf)
    [...prefix, leaf] => direct_or_nested(prefix, leaf)
    [head, ...] => starts_with_head(head)
    [..., last] => ends_with(last)
}
```

Exactly one `...` rest slot may appear. It may bind its contiguous segment or
ignore it. Prefix and suffix child patterns retain source order and exact
element type.

**TARGET - fixed length remains in the domain**

```shape
fn inspect_rgb(channels: Array<u8, 3>) {
    match channels {
        [red, ...middle, blue] => use_channels(red, middle, blue)
    }
}
```

The fixed domain proves this pattern irrefutable and gives `middle` exact
length one. A fixed pattern whose prefix plus suffix exceeds `N` is invalid;
a no-rest pattern must contain exactly `N` child positions.

**TARGET - typed generation without source strings**

```shape
comptime fn starts_with_zero(
    domain: SequencePatternDomain<Array<int>, int>,
) -> CheckedPattern<Array<int>, []> {
    comptime let zero: ConstValue<int> = 0

    patterns.sequence(domain)
        .prefix(patterns.const(zero))
        .ignore_rest()
        .finish()
}
```

The builder produces the same structure as `[0, ...]`. It receives a compiler-
issued domain, typed child pattern, and explicit rest kind; it cannot invoke an
iterator, parse pattern text, or assert length coverage.

**TARGET - required domain rejections**

```shape
match tuple_value { (head, ...tail) => use(tail) }
match text { ['a', ...middle, 'z'] => use(middle) }
match map { [first, ...rest] => use(rest) }
match stream { [head, ...tail] => use(tail) }
```

Heterogeneous tuples need a separate variadic tuple algebra. Strings have no
implicit byte/code-point/grapheme segmentation. Maps are unordered. An
arbitrary `Iterable` or stream would execute user `iter`/`next` code, allocate,
suspend, or fail during structural probe. None creates a sequence-pattern
domain.

Required rules:

1. `SequencePatternDomain<Container, Element>` is compiler-issued and sealed.
   User traits, iterator implementations, index operators, purity claims, and
   callbacks cannot create structural sequence eligibility.
2. `Sequence` replaces the old separate `FixedArray` frozen variant. Exact
   container kind and fixed `N` remain visible through the typed domain.
3. `[]` is the exact empty-sequence pattern. Without rest, `k` children mean
   exact length `k`.
4. With one rest, `p` prefix and `s` suffix children mean length at least
   `p + s`; a fixed domain refines the rest to exact length `N - p - s`.
5. More than one rest is a compile-time error. A rest segment is contiguous and
   cannot reorder, filter, classify, or invoke user code.
6. Bare `[...]` is rejected as a duplicate spelling of `_`. A bound or
   structurally constrained rest remains a `Sequence` node even when its
   denotation covers a complete sequence domain.
7. Structural probe performs only sealed length and indexed-element checks. It
   creates no iterator, allocation, move, mutable borrow, callback, effect, or
   suspension.
8. Prefix and suffix subpatterns use the complete sealed pattern algebra and
   participate in the same probe/guard/atomic-commit phases.
9. Coverage preserves exact sequence-domain identity, exact/minimum length, and
   positional child denotations. Users cannot assert completeness or collapse
   unknown iterable behavior into coverage.
10. Or-pattern alternatives agree on hygienic binders, element types, modes,
    projection footprints, and bound-rest type. Fixed alternatives also agree
    on the exact remainder length.
11. Reflection exposes the typed domain, ordered prefix children, rest kind,
    and ordered suffix children. Binding-changing rest edits are arm-owned;
    same-binding child/rest edits remain pattern-owned under Decision 79.
12. The checked-structure hash includes domain identity, fixed length, child
    order, rest kind, and child hashes. Canonical sequence coverage has its own
    denotation hash under Decision 85.
13. LSP hover reports exact versus minimum length, fixed remainder length,
    element type, rest binder identity/mode/type, and generated provenance.
14. Native syntax and builders publish only after type, coverage, ownership,
    drop, backend, hash, provenance, and LSP contracts all check.

Current migration implications:

- The current match `Pattern::Array` and unconditional `DestructurePattern`
  array/rest families normalize into one semantic `Sequence` representation.
- Existing `[head, ...tail]` destructuring cannot remain an unconditional
  runtime-failing bind for dynamic arrays; it is refutable and requires an
  exhaustive match or diverging `let-else`.
- Current best-effort `tail: Array<T>` inference does not settle the target
  segment ownership/type contract.
- Parser, irrefutability, length/content coverage, atomic sequence projections,
  MIR/VM/JIT lowering, reflection, and LSP must migrate together rather than
  preserving match-versus-destructure drift.

## Decision 88: Allocation-Free Segment Ownership

Accepted: a bound sequence rest is one contiguous place projection whose final
ownership mode follows Decision 76. Pattern probing and successful ownership
commit never materialize, copy, clone, or allocate a replacement tail.

For dynamic arrays, bare binders have these source-visible types:

```shape
match values {
    [head, ...tail] => {
        // head: T
        // tail: Array<T>
    }
}

match &values {
    [head, ...tail] => {
        // head: &T
        // tail: &Slice<T>
    }
}

match &mut values {
    [head, ...tail] => {
        // head: &mut T
        // tail: &mut Slice<T>
    }
}
```

`Slice<T>` is the first-class non-owning contiguous sequence view. Its shared
or exclusive reference carries the ordinary region and mutability proof. An
owned dynamic segment remains an ordinary `Array<T>` value so rest capture does
not expose runtime storage machinery in application types.

For a fixed root `Array<T, N>`, a rest with `p` prefix and `s` suffix positions
has `K = N - p - s`:

```shape
match fixed_values {
    [first, ...middle, last] => {
        // owned root: middle: Array<T, K>
        // shared root: middle: &Array<T, K>
        // exclusive root: middle: &mut Array<T, K>
    }
}
```

The fixed segment remains length-indexed. It is not widened to a dynamic array
or erased to an untyped view.

**TARGET - typed generated rest binder**

```shape
let head: PatternBinder<int, Own> =
    patterns.own_binder<int>()

let tail: PatternBinder<Array<int>, Own> =
    patterns.own_binder<Array<int>>()

let nonempty = patterns.sequence(array_int_domain)
    .prefix(head)
    .rest(tail)
    .finish()
```

The builder's tail type is fixed before arm/body checking. It cannot request
“whatever tail representation is cheapest,” and body use cannot retroactively
turn an owned tail into a borrow or clone.

The semantic split plan is compiler-owned:

```text
SequenceSplitPlan<Root, PrefixPlaces, RestPlace, SuffixPlaces, DropPlan>
```

This is proof metadata, not a Shape record or user-constructible runtime value.
It partitions element ownership and the backing-storage lifetime without
creating aliases to the same element.

Required execution order:

1. Evaluate the scrutinee once.
2. Probe length and every structural child without allocation, moves, mutable
   loans, callbacks, or user effects.
3. Evaluate a guard through shared guard views of element and rest projections.
4. On `false`, divergence, failure, or cancellation, end guard views and leave
   the original root completely intact.
5. On `true`, atomically commit every element move/borrow, the rest segment,
   unbound-element drop obligation, and backing-storage owner fragment.
6. Execute the body with the frozen binder types and modes.
7. Run every synchronous `Drop` and `AsyncDrop` exactly once, then release the
   backing storage exactly once after all owning/borrowing fragments permit it.

Required rules:

1. A dynamic owned rest binder has type `Array<T>`. Shared and exclusive rests
   have `&Slice<T>` and `&mut Slice<T>` respectively.
2. A fixed rest preserves exact `Array<T, K>` type and corresponding reference
   form. Const arithmetic is compiler-checked before publication.
3. The backing allocation may be represented by multiple disjoint owner
   fragments after commit, but no element has multiple owners and no owner
   fragment grants access outside its proven place interval.
4. Allocation-lifetime sharing required to free one backing store is not value
   aliasing: it cannot expose another fragment's elements or weaken ordinary
   move/borrow rules.
5. Probe and commit are allocation-free and cannot produce a resource failure.
   Any representation unable to split the existing owner infallibly does not
   implement this semantic contract.
6. A wildcard rest creates no user binder. Its elements remain in the compiler
   drop plan and are not copied into a hidden array.
7. Explicit `move`, shared borrow, and exclusive borrow modifiers apply to rest
   binders under Decision 76. Cloning remains an explicit body operation and
   may execute only after successful commit.
8. Mixed element/rest moves and borrows are accepted only when the ordinary
   place solver proves disjointness, regions, escape, and backing-owner lifetime.
9. A rest that escapes carries its owning array segment or proven borrow region;
   it cannot outlive hidden owner fragments needed by sibling projections.
10. Guards receive only shared views even when final rest mode is owned or
    exclusive. Suspension and cancellation retain the unsplit original root.
11. VM and JIT consume one checked `SequenceSplitPlan`; neither may implement
    tail capture by copying, eager allocation, early mutation, or backend-only
    ownership approximation.
12. Reflection exposes the exact rest binder type, mode, projection interval,
    and fixed length where applicable. Runtime owner-fragment metadata remains
    opaque.
13. LSP hover and ownership diagnostics show the root, projected interval,
    effective type/mode, region or owner transfer, and exact drop obligations.

Current migration implications:

- Current best-effort `tail: Array<T>` inference becomes a checked ownership
  result rather than an eager slice-copy convention.
- Dynamic arrays gain a semantic allocation-owner split operation plus shared
  and exclusive `Slice<T>` projections; runtime layout remains private.
- MIR owns the split/borrow/drop plan. Bytecode, JIT, OSR, cancellation, and
  snapshot boundaries consume or reject that plan consistently.
- Proofs cover prefix/suffix/infix rests, owned/shared/exclusive roots, mixed
  modes, guard false, async cancellation, escaping tails, zero-length rests,
  exact fixed lengths, and one-time element/backing-storage destruction.
