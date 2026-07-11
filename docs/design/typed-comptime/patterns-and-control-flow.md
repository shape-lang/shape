# Patterns And Control Flow

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Resources And Fragments](resources-and-fragments.md)

## Decision 74: Total Pattern Algebra

Accepted: source patterns and comptime-generated patterns share one checked
semantic algebra. Refutability, structural coverage, bindings, and ownership
use are compile-time facts. A pattern mismatch cannot become an implicit
runtime exception in an unconditional binding context.

The public structures are:

```shape
opaque comptime type CheckedPattern<T, Bindings>
opaque comptime type IrrefutablePattern<T, Bindings>
opaque comptime type CheckedArm<T, R, Effects>
opaque comptime type MatchPlan<T, R, Effects>
```

Every `CheckedPattern<T, Bindings>` contains compiler-sealed
`PatternCoverage<T>` and `PatternFootprint<T, Bindings>` evidence. These are
semantic compiler identities, not user-authored type-level set expressions.
`IrrefutablePattern<T, Bindings>` is a proof-bearing refinement whose coverage
is all of `T`. LSP may display the evidence, but ordinary source rarely names
it.

**TARGET - native complete-arm generation**

```shape
comptime fn map_option<T, U>(
    input: CheckedExpr<Option<T>>,
    f: FrozenCallable<fn(T) -> U>,
) -> CheckedExpr<Option<U>> {
    let plan = match_plan<Option<T>, Option<U>>(input)

    plan.arm {
        Some(value) => Some(f(value))
    }

    plan.arm {
        None => None
    }

    plan.finish()
}
```

`value` is an ordinary lexical pattern binder whose scope is the guard and
body of that arm. The whole arm is the preferred binderful generation unit, so
the common path is indistinguishable from native Shape pattern matching.

**TARGET - computed pattern structure**

```shape
let payload = patterns.binder<T>()

plan.arm(
    patterns.variant(some_variant).payload(payload)
) {
    Some(f(payload))
}
```

`some_variant` is a typed variant descriptor and `payload` is a typed hygienic
binder descriptor. In a pattern builder it declares that binder; in the arm
body's expression position it denotes the corresponding runtime binding. The
LSP exposes both staged roles through one compiler identity. There is no
string lookup, interpolation sigil, or spelling-based capture.

Standalone binderful patterns use explicit typed binder descriptors because a
new lexical binder cannot escape a standalone `pattern {}` block. Binderless
standalone patterns may use native syntax directly.

**TARGET - irrefutable native binding**

```shape
let Point { x, y } = point
```

The unified pattern checker proves that this pattern covers every value of the
known `Point` type before the unconditional binding is accepted. Function
parameters, assignments, and loop bindings use the same requirement.

**TARGET - required rejections**

```shape
let Some(value) = maybe_value
let [head] = values
```

The first pattern does not cover `None`. The second does not cover vectors of
other lengths. Both are compile errors unless an exhaustive `match` or the
diverging `let-else` from Decision 75 handles mismatch. The diagnostic reports
uncovered cases and never lowers to `throw "Pattern match failed"`.

Required rules:

1. The target compiler has one semantic pattern representation. Native source,
   contextual `pattern {}` blocks, and typed builders all produce it.
2. `CheckedPattern<T, Bindings>` records hygienic binder identities, exact
   bound types, copy/move/shared-borrow/exclusive-borrow modes, projection
   paths, structural coverage, and the ownership footprint on `T`.
3. Unconditional `let`, parameter, assignment, and loop-binding sinks accept
   only compiler-proven `IrrefutablePattern` values.
4. Refutable patterns are legal only in exhaustive `match` arms or the
   diverging `let-else` statement accepted in Decision 75.
5. Guards belong to `CheckedArm`, not `CheckedPattern`. Guarded coverage does
   not discharge exhaustiveness unless the compiler proves the guard is
   constantly true.
6. Or-pattern alternatives must bind the same hygienic identities with the
   same types, ownership modes, and projection compatibility.
7. A match plan is finite and atomic. `finish()` checks arm result types,
   effects, ownership, borrow validity, reachability, and exhaustive coverage
   before producing a `CheckedExpr<R>`.
8. Unknown coverage is a compile error. Open domains require a true catch-all;
   unresolved inference cannot silently classify exhaustiveness as not
   applicable.
9. Opaque representation remains inaccessible without the accepted
   `RepresentationAccess<T>` authority.
10. The installed match re-enters ordinary whole-context checking, and VM and
    JIT consume the same checked decision tree and ownership plan.

Current migration implications:

- The separate `DestructurePattern` and match `Pattern` AST families converge
  on the shared semantic algebra after parsing.
- Existing unconditional binding paths that emit runtime pattern-failure
  throws become compile-time irrefutability checks.
- The current shallow enum/closed-union exhaustiveness checker is replaced by
  a complete pattern-space matrix with reachability diagnostics.
- Generated binders carry expansion provenance for completion, hover,
  references, rename, ownership diagnostics, and virtual expansion views.

## Decision 75: Diverging Let-Else

Accepted: exhaustive `match` remains the sole semantic branching primitive.
Shape adds one statement-form convenience for linear early exit:

```shape
let PATTERN = VALUE else {
    NEVER
}
```

The `else` block must have the bottom type `never`. On success, the pattern's
bindings are in lexical scope after the statement. On mismatch, the block must
return, break, continue, loop forever, or otherwise be proven not to complete
normally.

**TARGET - ordinary early return**

```shape
fn require_user(maybe: Option<User>) -> Result<User, Error> {
    let Some(user) = maybe else {
        return Err(Error::missing_user())
    }

    Ok(user)
}
```

**TARGET - explicit loop filtering**

```shape
for candidate in candidates {
    let Some(value) = candidate else {
        continue
    }

    consume(value)
}
```

The mismatch path is visible and checked. A refutable loop-header pattern does
not silently skip elements.

**TARGET - value-producing recovery remains a match**

```shape
let user = match maybe_user {
    Some(user) => user
    None => User::guest()
}
```

A normally completing alternative is a branch whose result joins the success
result, so it is expressed by the existing exhaustive expression rather than a
second binding construct.

**TARGET - generated native body**

```shape
body {
    let Some(value) = input else {
        return None
    }

    Some(transform(value))
}
```

Binderful generated `let-else` lives in a whole body or another scope-owning
sink. A dynamic builder must take the successful continuation as a lexical
scope rather than returning a detached statement whose binders leak into an
unrelated fragment.

**TARGET - required rejections**

```shape
let Some(user) = maybe_user else {
    User::guest()
}

fn send(Some(connection): Option<Connection>, message: Message) {
    connection.send(message)
}

let matched = patterns.match(maybe_user, pattern { Some(user) })
```

The first `else` completes without binding `user`. The parameter hides a
failure path at the call boundary. The final form tries to leak a lexical
binder through an ordinary value. Diagnostics direct these cases to exhaustive
`match`, an irrefutable parameter plus body-local `let-else`, or a typed domain
value that does not pretend to be a lexical binding environment.

Required execution and ownership rules:

1. The scrutinee is evaluated exactly once into a compiler-owned temporary.
2. Structural matching is a non-consuming, effect-free probe. No user code,
   move, mutable borrow, or binding initialization occurs during the probe.
3. Success commits the complete checked move/borrow/binding plan atomically.
   Partial binding states are unrepresentable.
4. Mismatch commits no success binding. A previously named scrutinee remains
   available on the failure edge according to ordinary ownership rules.
5. Success and failure use explicit control-flow edges with ordinary drop
   obligations. Every initialized value is dropped exactly once.
6. An asynchronous `else` block may suspend before diverging. Scope exit,
   including `AsyncDrop`, remains part of the structured edge and cannot be
   bypassed by `return`, `break`, or `continue`.
7. Failure effects join the surrounding callable's effect row normally. The
   mismatch itself has no hidden panic, throw, callback, or effect.
8. Parameters, assignments, loop headers, and comprehension bindings remain
   irrefutable-only. Fixed-shape destructuring is accepted when its source type
   proves total coverage.
9. Non-diverging expression-valued `let-else` and binder-returning
   `Option<Bindings>`/`Result<Bindings, Mismatch>` are not core language
   surfaces; exhaustive `match` already models those paths without a second
   binding system.
10. Native source and generated bodies lower to the same checked decision tree
    consumed by VM and JIT. `let-else` introduces no dedicated runtime opcode.

LSP requirements:

- Completion and hover expose success binders only after the statement.
- The `else` block never sees those binders.
- A completing `else` diagnostic identifies its fallthrough path and required
  `never` type.
- Ownership and drop diagnostics show both generator and expansion sites.
- The exhaustive-match quick fix preserves the scrutinee's single evaluation.

## Decision 76: Scrutinee-Driven Binder Ownership

Accepted: a pattern binder's ownership disposition is fixed by the checked
scrutinee access and any explicit binder modifier. Downstream body use never
changes a borrow into a move or a move into a borrow, because that would make
partial-move state and synchronous or asynchronous drop timing depend on an
unrelated body edit.

The compiler-issued binder forms are conceptually:

```shape
PatternBinder<T, Own>
PatternBinder<T, Move>
PatternBinder<T, SharedBorrow>
PatternBinder<T, ExclusiveBorrow>
```

`Own` is the native bare-binder disposition for an owned place. It copies a
language `Copy` type and otherwise moves it. Explicit `move` suppresses that
implicit copy choice and consumes the matched projection. `SharedBorrow` and
`ExclusiveBorrow` create ordinary place-based loans whose duration is solved
by NLL.

**TARGET - native defaults**

```shape
match owned_result {
    Ok(value) => consume(value) // Copy if Copy; otherwise move
    Err(error) => report(error)
}

match &maybe_user {
    Some(user) => inspect(user) // user: &User
    None => {}
}

match &mut maybe_user {
    Some(user) => update(user)  // user: &mut User
    None => {}
}
```

Bare binders inherit owned, shared, or exclusive access from the matched
projection. The body may reborrow or auto-dereference normally, but cannot
retroactively select a different binder mode.

**TARGET - mixed explicit projections**

```shape
match packet {
    Packet {
        header: &header,
        stats: &mut stats,
        body: move body,
    } => process(header, stats, body)
}
```

`&header` forces a shared projected borrow. `&mut stats` requires mutable
access and forces an exclusive projected borrow. `move body` forces ownership
transfer. Disjointness and overlap are proved by the ordinary place solver.

**TARGET - parameter mode remains separate**

```shape
fn inspect(&Point { x, &label }: Point) {
    // Outer &: callable parameter ABI/root loan.
    // x: &int inherited from the root loan.
    // label: an explicitly shared projected binder.
}
```

The callable's parameter pass mode freezes before its irrefutable pattern is
checked. The outer mode determines the root place capability; inner binder
modes determine projection commits. Compiler and LSP must consume the same
frozen mode rather than maintaining an execution mode plus a display-only
guess.

**TARGET - computed standalone binders**

```shape
let payload = patterns.move_binder<Payload>()
let metadata = patterns.shared_binder<Metadata>()

let computed = patterns.variant(message_variant)
    .field(payload_field, payload)
    .field(metadata_field, metadata)
    .finish()
```

A binder created outside a native whole-arm sink chooses a typed mode when it
is created. A contextual builder may inherit only from an already frozen
scrutinee access; it cannot defer the choice to later body analysis.

**TARGET - required rejections**

```shape
match message {
    A(value) | B(&value) => use(value)
}

match &message {
    Data(move payload) => consume(payload)
}

let Some(&user) = load_user() else {
    return Err(Error::missing_user())
}
```

Or-pattern alternatives disagree on binder mode. A shared scrutinee cannot
yield an owned move. The final success binding would outlive the temporary it
borrows. Diagnostics name the exact projection and suggest borrowing the
scrutinee, moving an owned scrutinee, or introducing a sufficiently long-lived
owner.

Required rules:

1. Bare owned binders copy `Copy` projections and move all other projections.
2. Bare binders from `&T` and `&mut T` project shared and exclusive borrows,
   respectively.
3. Per-binder `move`, `&`, and `&mut` are authoritative overrides. `clone` is
   not a pattern mode because cloning may execute user code or effects; clone
   explicitly after binding.
4. Binder mode freezes before arm/body checking. NLL infers only loan duration,
   and ordinary use checking may reborrow or auto-dereference.
5. Atomic pattern commit records every moved projection and every loan at once.
   Failure records none.
6. After a partial move, disjoint initialized projections remain usable. The
   whole owner and moved projections remain unusable until reinitialized.
7. Normal drop order applies only to still-initialized projections. Moved
   values follow their new bindings, so `Drop` and `AsyncDrop` execute exactly
   once at stable, mode-determined points.
8. Or-pattern alternatives must agree on binder identity, type, mode, and
   compatible projection footprints.
9. Temporary lifetime extension never fabricates a region long enough for a
   success binding. Arm-local borrows may use a temporary only within its
   proven lifetime.
10. Guards may not change binder modes. Decision 77 supplies their exact shared
    pre-commit access.

Current migration implications:

- The unified semantic binder gains one compiler-issued mode instead of the
  current name-only match/destructure binder variants.
- Projected binding must lower through shared MIR ownership/place operations;
  the current copy-only MIR projection path is insufficient.
- Bytecode and JIT consume the same move/loan/drop plan. No bytecode-only slot
  nulling or JIT-only ownership approximation is acceptable.
- LSP scope, hover, inlay, references, and rename must support nested binders
  and show effective mode, referent type, source projection, region, and
  partial-move footprint.
