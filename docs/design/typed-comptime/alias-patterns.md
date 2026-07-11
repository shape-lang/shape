# Alias Patterns

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Sequence Coverage](sequence-coverage.md) | [Next: Dynamic Type Patterns](dynamic-type-patterns.md)

## Decision 90: Whole-Place Alias Patterns

Accepted: `alias @ PATTERN` gives the child pattern's exact root place a
hygienic source identity. It does not bind a second value, duplicate ownership,
copy or clone the scrutinee, or add another drop obligation.

The sealed public structure is conceptual:

```shape
opaque comptime type PatternPlaceAlias<T, Access>

sealed comptime struct CheckedAliasPattern<T, Access, Bindings> {
    alias: PatternPlaceAlias<T, Access>,
    child: CheckedPattern<T, Bindings>,
}

sealed comptime enum FrozenPattern<T, Bindings> {
    Alias(CheckedAliasPattern<T, _, Bindings>),
    // ...other sealed constructors
}
```

`PatternPlaceAlias` is deliberately distinct from `PatternBinder<T, Mode>`.
A binder commits a value or loan projection under Decision 76. A place alias
names the already-existing root capability and exposes its post-commit
initialization and loan state.

**TARGET - partial move remains visible through the alias**

```shape
match packet {
    whole @ Packet {
        header: &header,
        body: move body,
    } => {
        inspect(header)
        consume(body)
        audit(whole.metadata)
        send(whole) // error: whole.body was moved
    }
}
```

The body may use initialized disjoint projections through `whole`. A whole-
value move, borrow, method, or drop requiring the moved projection is rejected.
If the place is legally mutable and fully reinitialized, ordinary place-state
analysis may make whole-place operations available again.

**TARGET - child loans are loans from the aliased place**

```shape
match document {
    whole @ Document { title: &title } => {
        render(title)
        archive(whole) // legal only after the title loan ends
    }
}
```

NLL determines loan duration from ordinary uses. Moving `archive(whole)` before
the last use of `title` is a borrow error; aliasing does not create a self-
referential value or extend a temporary lifetime.

**TARGET - explicit root capability narrowing**

```shape
match owned_point {
    &whole @ Point { x, y } => inspect(whole, x, y)
}

match &mut point {
    &mut whole @ Point { x: &mut x, y: &y } => {
        update(x)
        inspect(y)
    }
}
```

`&whole @ PATTERN` checks the child through shared root access.
`&mut whole @ PATTERN` checks it through an exclusive root reborrow. Bare
`whole @ PATTERN` inherits the already-frozen scrutinee access.

There is no `move whole @ PATTERN` form. An alias is a place identity, not a
second destination binding. Users explicitly move `whole` in the body when its
post-commit place state permits it.

**TARGET - typed generation**

```shape
let whole: PatternPlaceAlias<Packet, InheritedAccess> =
    patterns.place_alias<Packet>()

let child = pattern {
    Packet { header: &header, body: move body }
}

let aliased = patterns.alias(whole, child)
```

The contextual builder gives `InheritedAccess` a compiler-issued identity
derived from the checked scrutinee. It cannot construct an alias from a name,
source offset, runtime reference, or claimed ownership mode.

**TARGET - required rejections**

```shape
match shared_packet {
    &whole @ Packet { body: move body } => consume(body)
}

match packet {
    move whole @ Packet {} => consume(whole)
}
```

A shared root cannot yield a moved child. The second spelling is rejected with
a diagnostic to use `whole @ ...` and move `whole` in the body after the
pattern's complete footprint is known.

Required rules:

1. `Alias<T>` has exactly the child pattern's structural denotation. It adds a
   place identity and capability, not another match test.
2. Bare alias access inherits the checked scrutinee root. `&` and `&mut`
   explicitly narrow or reborrow that root before checking every child.
3. Alias and descendant projections must form one atomic root commit plan with
   no duplicate ownership, conflicting loans, or inaccessible place interval.
4. A moved descendant marks the corresponding alias projection uninitialized.
   Whole-place operations remain illegal until every required projection is
   initialized and all conflicting loans end.
5. Descendant shared/exclusive borrows are ordinary loans from the aliased root.
   They cannot outlive its owner, escape a temporary, or permit the root to move
   while live.
6. Probe creates neither alias binding nor child ownership. During a guard, the
   alias and all child binders are shared guard views under Decision 77.
7. Guard false, divergence, failure, or cancellation commits nothing. Guard
   true atomically publishes the alias identity, child moves/loans, partial-
   initialization state, and one shared drop plan.
8. The alias adds no `Drop` or `AsyncDrop`. Moved descendants follow their new
   bindings; remaining initialized root fragments drop exactly once.
9. Nested aliases are legal when the place solver can derive compatible root
   and projected capabilities. They remain separate hygienic place identities,
   never ownership copies.
10. Or-pattern alternatives expose the same alias identities, exact types,
    access capabilities, child binder interfaces, and compatible post-commit
    footprint summaries.
11. A pattern-owned same-interface edit may replace the alias child. Adding,
    removing, or changing an externally visible alias is arm-owned under
    Decision 79.
12. The checked-structure hash includes the `Alias` tag, hygienic alias identity,
    exact type/access descriptor, and child structure hash. The denotation hash
    equals the child's denotation hash.
13. LSP hover shows that the alias names the root place, its effective access,
    moved projections, active loans, remaining legal operations, and generated
    provenance. Rename follows the hygienic identity, not text.
14. Native and generated aliases lower to one MIR place/ownership plan consumed
    by VM, JIT, OSR, cleanup, and snapshot-safety checks.

Current migration implications:

- Add a contextual infix-`@` parser form and one semantic `Alias` constructor;
  current prefix annotation/time/import uses remain separate grammar contexts.
- Do not reuse postfix `as`, which already denotes type assertions and several
  declaration/import aliases.
- Replace name-only pattern collection with hygienic root-place identities and
  recursive place-state checking.
- Refutable unconditional aliases follow the same exhaustive `match` or
  diverging `let-else` rules as every other pattern; no runtime “pattern match
  failed” path remains.
- Parser, inference, ownership, MIR/VM/JIT/OSR, coverage, reflection, LSP, and
  book proofs are all new work; no current alias behavior requires compatibility.
