# Guards And Exhaustiveness

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Patterns And Control Flow](patterns-and-control-flow.md)

## Decision 77: Effect-Typed Pre-Commit Guard Views

Accepted: a guarded arm executes in four explicit phases:

1. Effect-free, non-consuming structural probe.
2. Effect-typed guard evaluation over temporary shared views.
3. Atomic final ownership commit only when the guard returns `true`.
4. Body execution with the final binder modes from Decision 76.

There is no ownership or effect rollback.

The compiler-issued guard capability is conceptually:

```shape
GuardView<T, FinalMode>
```

It is a read-only view of the matched projection plus evidence of the mode that
will be committed if the guard succeeds. It is not a runtime wrapper, user
reference type, or independently storable value.

**TARGET - moved payload with an asynchronous guard**

```shape
match job {
    Ready(move task)
        where await scheduler.allow(task.id()) => run(task)

    Ready(task) => defer(task)
    Pending => {}
}
```

During `scheduler.allow`, `task` is a shared guard view. If the guard returns
`true`, that view ends and `task` is atomically committed as an owned moved
binding for the body. If it returns `false`, no move occurred and the next arm
probes the original `job`.

**TARGET - exclusive body binding with shared guard access**

```shape
match &mut session {
    Some(connection)
        where connection.id() == wanted => connection.send(message)

    _ => {}
}
```

The final binder is exclusive because the scrutinee is `&mut`. The guard may
only read through a shared view. Its shared loan ends before the exclusive body
binding is committed.

**TARGET - generated arm uses one identity across phases**

```shape
let task = patterns.move_binder<Task>()

plan.arm(
    patterns.variant(ready_variant).payload(task),
    where: expr { await scheduler.allow(task.id()) },
) {
    run(task)
}
```

The same compiler-issued binder descriptor appears in guard and body contexts.
The guard sink resolves it to `GuardView<Task, Move>`; the body sink resolves
it to the committed owned `Task`. No name lookup or interpolation distinguishes
the phases.

**TARGET - required rejections**

```shape
match packet {
    Packet { body: move body }
        where consume(body) => done()

    _ => fallback(packet)
}

match &mut session {
    Some(connection)
        where connection.advance() => use(connection)

    _ => {}
}
```

The first guard attempts to consume a pre-commit projection. The second tries
to mutate through a shared guard view. Diagnostics report the final intended
mode and suggest moving the operation into the arm body or restructuring the
match.

Required semantics:

1. Structural probing executes no user code and creates no final binding,
   move, mutable loan, drop obligation, or externally observable effect.
2. Every matched projection is exposed to the guard through a read-only shared
   view, including projections whose final modes are `Move`, `Own`, or
   `ExclusiveBorrow`. A `Copy` value may be copied by ordinary read semantics,
   but the final binding is not committed early.
3. Guards are ordinary checked boolean expressions. Their declared effects,
   including `Suspend`, enter the enclosing callable and `CheckedArm` effect
   rows.
4. Effects completed by a guard remain observable when it returns `false`.
   Shape promises no transaction or rollback for external effects.
5. `false` ends guard views and guard temporaries, commits no binder, and
   continues arm selection with the unchanged ownership state.
6. `true` ends all guard views before atomically committing the complete final
   binder and partial-move plan. The body then sees only final binder modes.
7. Divergence, explicit failure, or cancellation exits on ordinary structured
   control-flow edges. Later arms do not run and no final binder was committed.
8. A guard view may survive `await` only when region and async-state analysis
   proves the scrutinee remains owned and immovable for the suspension. An
   unprovable self-reference or escape is a compile error.
9. Cancellation during suspension drops guard temporaries and the uncommitted
   scrutinee exactly once. Every synchronous `Drop` and `AsyncDrop` obligation
   remains on the structured cancellation edge.
10. Guards contribute no structural exhaustiveness coverage unless the
    compiler proves the guard constantly `true`. A constantly false guard is
    unreachable.
11. Or-pattern alternatives first satisfy the identity/type/final-mode rules
    from Decision 76; their common binders then produce identical guard views.
12. Native source and generated `CheckedArm` lower to one checked guard CFG
    consumed by VM and JIT. No backend may bind early or synthesize rollback.

LSP requirements:

- Hover distinguishes `task: &Task (guard view; commits as move Task)` from the
  body binding `task: Task` while preserving one navigation identity.
- Completion excludes consuming and mutating operations unavailable through a
  guard view and explains why they are unavailable.
- Effect and suspension diagnostics show both the guard and enclosing callable
  requirements.
- Borrow-across-await and cancellation-cleanup diagnostics link generated
  expansion locations back to their generator sites.
- Expansion views display probe, guard, commit, and body phases without
  exposing backend temporaries as source bindings.

Current migration implications:

- Existing `where` syntax is retained; no competing `if`-guard spelling is
  introduced.
- Guard binders must become phase-aware semantic identities rather than normal
  locals initialized before the condition.
- MIR owns the four-phase CFG, ownership commit, guard-view regions, and cleanup
  edges. Bytecode and JIT lower that shared proof rather than reconstructing
  order independently.
- Focused proofs must cover guard false followed by another arm, final move and
  exclusive modes, guard effects, suspension/cancellation, exact-once drop,
  or-patterns, exhaustiveness, and phase-aware LSP behavior.

## Decision 78: Closed Pattern Algebra

Accepted: structural matching is a sealed compiler semantic algebra. Libraries
extend pattern ergonomics by composing `CheckedPattern`, `CheckedArm`, and
`MatchPlan` values at comptime. They cannot install runtime user code as a new
structural probe operation.

**TARGET - typed comptime pattern synonym**

```shape
comptime fn add_zero(
    lhs: PatternBinder<Expr, SharedBorrow>,
) -> CheckedPattern<Expr, [lhs]> {
    pattern { Expr::Add(lhs, 0) }
}

let lhs = patterns.shared_binder<Expr>()

plan.arm(add_zero(lhs)) {
    simplify(lhs)
}
```

`add_zero` is an ordinary comptime function returning a checked semantic
pattern. Its output has the same sealed coverage, footprint, probe plan,
provenance, and backend lowering as a directly written pattern. It does not
define a runtime matching protocol or new syntax category.

**TARGET - explicit runtime classification**

```shape
match Email::parse(text) {
    Ok(email) => accept(email)
    Err(error) => reject(error)
}
```

Dynamic parsing, validation, normalization, regex matching, and extraction are
ordinary typed runtime operations. They return a closed domain enum, `Option`,
or `Result`, which source then matches with the normal structural algebra. The
classification call and all of its effects remain explicit.

**TARGET - runtime predicate remains a guard**

```shape
match request {
    Request { user: &user, body: move body }
        where policy.allows(user) => process(body)

    _ => reject(request)
}
```

Predicate-like conditions use the effect-typed guard phase from Decision 77.
They do not become structural coverage evidence.

**TARGET - required rejection**

```shape
active pattern ValidEmail(text: string) -> Option<Email> {
    Email::parse(text)
}

active pattern Simplified(node: Expr) -> Option<Expr> {
    normalize(node)
}
```

Required diagnostics explain that Shape has no active-pattern protocol. They
offer an ordinary classifier returning a typed sum, a `where` guard for a
predicate, or a comptime function that composes sealed pattern constructors.
Decision 91 separately admits exact concrete identity refinement inside an
open erased domain. That sealed intrinsic executes no user classifier or trait
conformance code and does not make the open domain exhaustive.

Required rules:

1. The compiler owns the complete structural pattern variant catalog and the
   semantics of every variant.
2. Structural probe plans contain no runtime callback, trait dispatch, user
   function, allocation, IO, suspension, or other effect.
3. A comptime pattern synonym may use only typed descriptors, hygienic binders,
   native contextual fragments, and sealed builders. Its result re-enters the
   ordinary pattern checker.
4. Synonyms cannot assert coverage, irrefutability, ownership, or purity. The
   compiler derives all evidence from the returned closed pattern.
5. Runtime classifiers expose their success, failure, effects, and produced
   values through ordinary function types. Matching their returned closed sum
   is exhaustive in the normal way.
6. Runtime predicates use `where` and the shared pre-commit guard capabilities
   from Decision 77. They contribute no structural coverage unless proven
   constantly true.
7. A string, regex, callback, trait method, or user-declared `pure`/`total`
   function cannot become a structural pattern node.
8. Private members require ordinary visibility. Opaque or privileged
   representation requires `RepresentationAccess<T>` during generation and
   remains inaccessible to unauthorized installed code.
9. Synonym expansion inputs, typed pattern output, dependencies, and provenance
   enter the normal expansion hash and incremental graph.
10. LSP completion, hover, navigation, rename, diagnostics, and expansion views
    operate on the resulting compiler identities, with links back to the
    synonym definition and invocation.
11. Adding a primitive pattern variant changes the language and comptime ABI.
    It lands only with complete type/binding rules, coverage algebra, ownership
    footprint, probe and commit semantics, diagnostics, VM/JIT lowering, LSP,
    examples, and positive/negative proofs.

Current migration implications:

- No public extractor trait or active-pattern declaration is introduced.
- Existing pattern source and typed builders normalize into the same sealed
  semantic representation before exhaustiveness and ownership analysis.
- Domain libraries that currently emulate extraction through generated source
  should migrate either to typed comptime pattern composition or explicit
  runtime classifier values.

## Decision 79: Reflectable Patterns And Scope-Owned Rewrites

Accepted: sealed structural patterns are fully reflectable and editable through
typed semantic views and owner-bound cursors. Sealed means that the compiler
owns the variant semantics; it does not mean that comptime transforms receive
an opaque, non-rewritable value.

The public structures are conceptually:

```shape
sealed comptime enum FrozenPattern<T, Bindings> { ... }

opaque comptime type PatternRoot<T, Bindings>
opaque comptime type PatternCursor<Root, Node, NodeBindings>
opaque comptime type PatternEdit<Root, T, Bindings>
```

The exact `FrozenPattern` variants are resolved by subsequent constructor
decisions. Every variant preserves the input type and complete binding
environment rather than erasing them into a common AST node.

**TARGET - typed reflection and same-binding edit**

```shape
let edit = checked_pattern.edit()

for some<Node, NodeBindings> child
    in checked_pattern.cursor().children()
{
    match child.freeze() {
        FrozenPattern::Const(constant)
            where constant.is_int(0) => {
                edit.replace_same(child, pattern { 1 })
            }

        _ => {}
    }
}

let updated = edit.finish()
```

`replace_same` accepts only a `CheckedPattern<Node, NodeBindings>`. The
replacement may change structural coverage, but it cannot add, remove, or
change a binder visible outside that subtree. The compiler recomputes coverage,
probe, and ownership evidence rather than trusting the transform.

**TARGET - binding change replaces the complete lexical arm**

```shape
plan.replace_arm(user_arm) {
    Some(user) where user.active() => user.name()
}
```

Pattern, guard, body, guard views, final binder modes, and body references are
one lexical unit. An edit that changes its binding interface therefore replaces
that complete arm. Shape does not offer positional, ordinal, or name-based
binder remapping.

**TARGET - installed coverage remains match-owned**

```shape
let edit = checked_match.edit()

edit.replace_pattern_same(ready_arm, ready_case, replacement)

let updated_match = edit.finish()
```

Even an exact-binding pattern replacement can change coverage and reachability.
When the pattern belongs to an installed match, the enclosing `MatchPlan`
transaction owns the edit and reruns whole-match validation before publication.

**TARGET - required rejections**

```shape
let left = pattern { [0, 1] }
let right = pattern { [2, 3] }

let edit = left.edit()
edit.replace_same(right.cursor(), pattern { [4, 5] })
```

The cursor belongs to another root.

```shape
let user = patterns.move_binder<User>()
let some_user = pattern { Some(user) }

let edit = some_user.edit()
edit.replace_same(some_user.cursor(), pattern { None })
```

The replacement removes the `user` binding. It must replace the owning arm so
guard and body references cannot become stale.

Required rules:

1. `FrozenPattern<T, Bindings>` is an exhaustive indexed sum over the accepted
   closed pattern catalog. It exposes semantic descriptors, never parser nodes
   or source text.
2. Reflection preserves heterogeneous child witnesses through existential
   packages such as `exists<Node, NodeBindings> PatternCursor<...>`.
3. A cursor is tied to one immutable root identity and root revision. It cannot
   address another root or survive publication of an edited revision.
4. Cursors are ephemeral compiler capabilities. They never serialize, cross a
   module/remote boundary, or enter artifact and expansion hashes.
5. Reflection exposes binder identities and modes, compiler-derived coverage
   and ownership footprints, and source/generated provenance. Transforms may
   compare or inspect that evidence but cannot construct or overwrite it.
6. A detached `CheckedPattern` owns traversal and exact-binding subtree edits.
7. `CheckedArm` owns every rewrite that changes binder identities, types, or
   modes because its pattern, guard, and body share one lexical scope.
8. `MatchPlan` owns installed arm insertion, replacement, ordering, cumulative
   coverage, reachability, result/effect joins, and final publication.
9. Overlapping edits, stale cursors, ambiguous parent/child replacement, and
   incompatible binding interfaces are compile errors with both edit origins.
10. `finish()` applies the finite edit set atomically, rebuilds semantic binder
    and projection graphs, reruns all enclosing checks, and either publishes a
    complete immutable result or publishes nothing.
11. Expansion hashes contain the resulting semantic pattern/arm/match, typed
    dependencies, provenance, and canonical edit intent; cursor addresses and
    rendered names are excluded.
12. Compiler and LSP consume the same binder and cursor identity graph. Rename,
    references, and navigation for pattern binders never fall back to textual
    search.
13. Virtual expansion documents are read-only. A generator-controlled binder
    navigates to its binder/name policy source rather than accepting an edit to
    rendered virtual text.

Current migration implications:

- The current whole-pattern span and text-oriented LSP scope model is
  insufficient for nested binders, guard views, and generated provenance.
- Pattern transforms wait for semantic binder identities and source maps shared
  by compiler and LSP; a span-only intermediate API is not a valid subset.
- Existing source/JSON AST rewrite paths migrate to typed cursors and atomic
  owner-level plans rather than being wrapped as compatibility helpers.
