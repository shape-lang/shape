# Boolean Pattern Algebra

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Dynamic Type Patterns](dynamic-type-patterns.md)

## Decision 92: Binder-Free Negation And Single-Binding Conjunction

Accepted: Shape patterns support exact complement and intersection through two
sealed constructors:

- `!CONSTRAINT` complements one binder-free, zero-commit-footprint pattern.
- `P & Q & ...` intersects patterns while permitting at most one operand to
  introduce bindings or a commit footprint.

The compiler-issued constraint category is conceptual:

```shape
opaque comptime type PatternConstraint<T> {
    pattern: CheckedPattern<T, []>,
    footprint: EmptyPatternFootprint<T>,
}

sealed comptime struct CheckedNotPattern<T> {
    child: PatternConstraint<T>,
}

sealed comptime struct CheckedAllOfPattern<T, Bindings> {
    binding_pattern: Option<CheckedPattern<T, Bindings>>,
    constraints: PatternPack<PatternConstraint<T>>,
}
```

`PatternConstraint<T>` is stronger than “the current syntax happens not to
declare a name.” It proves there is no move, loan, place alias, rest transfer,
drop fragment, or other final commit effect.

**TARGET - exact scalar set algebra**

```shape
match percentage {
    0u8..=100u8 & !42u8 => ordinary()
    42u8 => special()
    _ => invalid()
}
```

The first arm denotes `[0, 100]` minus `{42}`. `!42u8` runs no user equality;
it complements the canonical `Const<u8>` singleton. Coverage, reachability,
and exhaustiveness use exact set intersection/complement.

**TARGET - one binding producer plus structural constraint**

```shape
match point {
    Point { x, y }
        & Point { x: 0..=100 } => inside_x_band(x, y)

    _ => outside()
}
```

The first operand introduces `x` and `y`. The second is binder-free because its
omitted `y` field and range child commit nothing. Both probe the same `Point`
place; only the first operand contributes the atomic binding plan.

**TARGET - whole value binding remains explicit**

```shape
match digit {
    whole @ !(0..=9) => non_digit(whole)
    _ => digit()
}
```

`Not` itself has no binder. Decision 90's `Alias` outside it names the matched
place and keeps ownership visible rather than giving negation special binding
semantics.

**TARGET - typed composition**

```shape
comptime let bounded: PatternConstraint<u8> =
    pattern { 0u8..=100u8 }

comptime let excluded: PatternConstraint<u8> =
    patterns.not(pattern { 42u8 })

let ordinary = patterns.all_of(
    binding_pattern: None,
    constraints: [bounded, excluded],
)
```

Builders accept typed constraints and checked patterns only. They cannot infer
emptiness, binder compatibility, or purity from strings, callbacks, user
traits, or declared booleans.

**TARGET - required rejections**

```shape
match option {
    !Some(value) => no_value_binding(value)
}

match point {
    Point { x, y: _ } & Point { x: _, y } => duplicate_producers(x, y)
}

match value {
    !(whole @ 0..=9) => alias_inside_negation(whole)
}
```

Negated binders have no success value to initialize. Multiple binding
producers require binder identity and ownership unification that Shape does not
perform. A place alias carries a root capability and therefore is not a
zero-footprint constraint.

Required rules:

1. `PatternConstraint<T>` contains no binder and has compiler-proven empty
   commit/ownership/drop footprint. Users cannot construct or assert that
   evidence.
2. `Not<T>` accepts exactly one `PatternConstraint<T>`. Its denotation is the
   exact complement within the child's complete static pattern domain.
3. `AllOf<T, Bindings>` contains zero or one binding pattern plus any finite
   number of constraints. Every operand has the same exact root type `T`.
4. Native `P & Q` construction rejects more than one operand with a nonempty
   binder interface or commit footprint. Matching names or apparently disjoint
   source fields do not cause implicit binder merging.
5. Probe evaluates every operand through the sealed effect-free pattern plan.
   Because constraints have no effects or commit, the compiler may reorder or
   share their tests without observable behavior.
6. Guard evaluation remains outside Boolean patterns at `CheckedArm`. A guard
   cannot be hidden in `Not` or `AllOf`, and its effects/coverage retain
   Decision 77 semantics.
7. Successful `AllOf` commit publishes only its optional binding pattern's
   already-checked atomic plan. Failure commits nothing. `Not` never commits.
8. An intersection or complement proven empty is a compile-time invalid
   pattern; Shape does not add an `EmptyPattern` constructor. Proof budget
   exhaustion is the blocking diagnostic from Decision 89, not permission to
   publish an unknown pattern.
9. `Or` remains the union constructor and still requires identical binder
   interfaces. The native precedence order is unary `!`, then infix `&`, then
   infix `|`; parentheses override grouping.
10. Reflection exposes `Not`'s typed child and `AllOf`'s optional binding
    pattern plus ordered constraints. Source order is preserved for editing and
    provenance even though probe order is not observable.
11. Adding/removing/changing the one externally visible binding pattern is
    arm-owned. Same-interface constraint edits are pattern-owned under Decision
    79 and recompute complete denotation/reachability atomically.
12. Checked-structure hashes include constructor, operand order, child
    structures, binder interface, and empty-footprint evidence identities.
    Denotation hashes canonicalize complement/intersection independently of
    source order.
13. LSP hover shows each operand's exact domain, bindings, empty footprint,
    normalized set operation, residual coverage, and generated provenance.
    Diagnostics point to the second binding producer or the binder hidden under
    negation.
14. Native syntax and typed builders lower to one checked Boolean coverage and
    probe plan consumed by VM, MIR JIT, and OSR JIT. Backends cannot substitute
    short-circuit user expressions or generic equality.

Current migration implications:

- Add a real pattern precedence layer for `!`, `&`, and `|`; expression
  negation/bitwise operators, borrow binders, and type intersections remain
  separate parser contexts.
- Introduce `PatternConstraint<T>` and empty-footprint evidence after unified
  pattern ownership checking; current AST name counting is insufficient.
- Extend canonical coverage and structured diagnostics with complement,
  intersection, empty-pattern rejection, and residual witnesses.
- Prove native/builder/reflection/edit parity, scalar/range/nominal/sequence/
  exact-dynamic constraints, ownership rejections, guard separation, hashes,
  LSP precedence, and VM/JIT/OSR parity before enabling the syntax.

## Decision 93: Exact Residual Reachability

Accepted: match arms retain ordered first-match semantics. The compiler derives
each arm's residual denotation after prior unguarded or constantly-true arms.

```shape
match value {
    Some(0) => zero()
    Some(value) => other_present(value)
    None => missing()
}
```

The second arm is valid because its residual is nonempty. Its binder keeps its
ordinary static type; residual coverage is proof/tooling information unless the
normal type system independently represents that refinement.

Required policy:

1. An arm or Or alternative with empty residual coverage is a hard unreachable
   compile error.
2. A constantly false guard is unreachable. A nonconstant guard contributes no
   shadowing coverage; a constantly true guard contributes normal coverage.
3. Partial shadowing is valid. LSP exposes the shadowed and live residual sets
   without requiring users to rewrite ordinary specific-before-general matches.
4. Missing exhaustiveness and coverage-budget exhaustion remain hard errors.
5. No suppression may accept unreachable or unknown coverage.

```shape
match number {
    0..10 => first()
    5..15 => second() // live residual: 10..15
}
```

Compiler diagnostics carry structured prior-arm identities, overlap, residual,
and witness proofs. VM and JIT consume the already checked arm order; they do
not recompute reachability.
