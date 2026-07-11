# Sequence Coverage

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Sequence Patterns](sequence-patterns.md) | [Next: Alias Patterns](alias-patterns.md)

## Decision 89: Exact Symbolic Sequence Languages

Accepted: the denotation of a homogeneous sequence pattern is a symbolic
regular language whose alphabet is the canonical pattern coverage algebra of
its exact element type.

```text
[P1, P2]               = L(P1) · L(P2)
[P1, ...rest, S1, S2] = L(P1) · Element* · L(S1) · L(S2)
```

`Element` is the complete `CoverageSet<ElementType>`, not a runtime wildcard,
iterator step, callback, or user-supplied classifier. Binder names, modes, and
rest ownership remain checked structure but do not change the set of sequence
values matched.

The compiler product is conceptual:

```shape
opaque comptime type SymbolicSequenceLanguage<Container, Element>

sealed comptime enum CoverageEmptiness<Language, Element> {
    Empty(EmptyCoverageProof<Language>),
    NonEmpty(SymbolicSequenceWitness<Element>),
}
```

Union, intersection, difference, and emptiness classification are typed
compiler queries over these identities. User code cannot construct automaton
states, forge an `EmptyCoverageProof`, assert completeness, or install a
transition label.

**TARGET - dynamic boolean sequence exhaustiveness**

```shape
match flags {
    [] => empty()
    [true, ...] => starts_true()
    [false, ...] => starts_false()
}
```

The first arm covers length zero. The other two cover every positive length
because the first element's boolean coverage is complete. The compiler proves
the union equal to `bool*`.

**TARGET - missing length witness**

```shape
match values {
    [] => empty()
    [only] => singleton(only)
    [first, second, ...tail] => many(first, second, tail)
}
```

This example is exhaustive: the rest may have length zero. Removing the final
arm instead produces the shortest uncovered length witness `[_, _]`.

**TARGET - missing element witness**

```shape
match bytes {
    [] => empty()
    [0u8, ...] => starts_with_zero()
    [1u8, ...] => starts_with_one()
}
```

The compiler reports a missing nonempty sequence whose first element is outside
`{0, 1}`. The structured witness records length one and a first-element
denotation such as `2u8..`; rendering never invents an authoritative error
string in place of the proof.

**TARGET - nested symbolic coverage**

```shape
match rows {
    [] => no_rows()
    [[], ...] => first_row_empty()
    [[_, ...], ...] => first_row_nonempty()
}
```

For `Array<Array<T>>`, transition labels are themselves canonical sequence
denotations. Empty versus nonempty inner arrays cover the complete first-row
domain, so the three arms are exhaustive.

**TARGET - guard does not erase a residual**

```shape
match values {
    [head, ...] where permitted(head) => accepted()
    [head, ...tail] => rejected(head, tail)
    [] => empty()
}
```

Unless the first guard is compiler-proven constantly true, it contributes no
sequence language to exhaustiveness. The second arm remains reachable over the
complete nonempty domain.

Required algebra:

1. An exact no-rest sequence pattern is a finite concatenation of child
   `CoverageSet<Element>` labels.
2. A one-rest pattern is its prefix concatenation, followed by `Element*`,
   followed by its suffix concatenation. A zero-length rest is included.
3. Dynamic sequence coverage is the resulting symbolic regular language.
   Fixed `Array<T, N>` coverage intersects that language with exact length `N`.
4. Or-pattern alternatives and match arms use language union. Reachability is
   the current arm language minus prior unguarded or constantly-true arm
   languages. Exhaustiveness is the sequence domain minus their union.
5. Fully unreachable arms and alternatives have empty residual languages.
   Partially covered arms retain their exact nonempty residual proof.
6. Nonconstant guards contribute no coverage. Constant-false guards are
   unreachable; constant-true guards contribute their structural language.
7. Symbolic transition labels support exact union, intersection, complement,
   emptiness, and witness queries through the sealed element denotation. No
   backend equality, user predicate, or approximation substitutes for them.
8. Nested sequence elements recursively use `SymbolicSequenceLanguage` labels.
   Compiler memoization and owner/type identities make recursive queries finite
   or terminate them through the deterministic coverage budget.
9. Determinization partitions overlapping element labels through canonical
   coverage operations. The denotation form has stable state ordering and
   minimization sufficient for equivalent languages to share a denotation hash.
10. Pattern structure, canonical language denotation, and expansion application
    retain the distinct hashes established by Decisions 67, 69, and 85.
11. The runtime decision tree is derived from the same checked patterns and
    coverage proof. VM and JIT need not execute an automaton, but cannot derive
    different matching semantics.

Witness and diagnostic requirements:

- Missing-length diagnostics identify the shortest uncovered length and a
  symbolic sequence shape.
- Missing-content diagnostics identify the earliest uncovered element path and
  its typed residual denotation.
- Unreachable diagnostics name the prior arm identities whose union covers the
  arm and provide an intersection witness.
- Partial residual diagnostics carry both overlap and live residual proof
  objects and use the non-blocking LSP policy accepted in Decision 93.
- Prefix/suffix and zero-length-rest witnesses preserve exact position mapping.
- LSP quick fixes may add an explicit proven residual arm or convert refutable
  unconditional binding to `match`/`let-else`; they never silently insert `_`
  or weaken a pattern.

## Deterministic coverage budget

Symbolic determinization, nested label partitioning, difference, minimization,
and witness search may grow exponentially. Every compiler query therefore uses
a deterministic coverage budget over stable dimensions such as generated
states, transition partitions, nested denotation operations, Or alternatives,
and witness steps.

Budget exhaustion is a blocking compile-time diagnostic whenever the compiler
must prove exhaustiveness, irrefutability, reachability, or a required edit
invariant. It reports:

- The consumed budget dimensions.
- The arm and nested typed pattern path that caused expansion.
- The semantic proof frontier reached.
- Deterministic refactorings such as splitting by length, factoring a common
  prefix/suffix, simplifying alternatives, or decomposing a nested match.

There is no switch to assume exhaustive, accept unknown reachability, emit a
runtime fallback, or classify the query as `NotApplicable`. The concrete
versioned budget policy is resolved separately; changing fuel can never change
the required proof or language semantics.

Current migration implications:

- Replace enum/union/catch-all-only exhaustiveness with a shared recursive
  coverage query that can host symbolic sequence languages.
- Legacy array matching's exact-length checks become one runtime lowering of
  the already-proven sequence structure rather than the coverage algorithm.
- Compiler diagnostics and LSP consume structured residual/witness proofs and
  stable codes, not message parsing or a second editor-side matcher.
- Proofs cover exact/minimum/fixed lengths, zero-length rests, prefix/suffix
  interactions, nested sequences, Or, guards, residuals, canonical hashes,
  deterministic witnesses, budget exhaustion, and VM/MIR-JIT/OSR parity.
