# ADR-008: Allocation-Free Sequence Segment Ownership

## Status

Accepted (2026-07-11)

## Decision

A bound sequence-rest pattern commits an allocation-free contiguous ownership
or borrow projection. An owned dynamic rest remains `Array<T>`; shared and
exclusive rests are `&Slice<T>` and `&mut Slice<T>`; fixed-array rests preserve
their exact const length. Probe and commit never copy or allocate a replacement
tail.

## Context

Eagerly materializing `[head, ...tail]` would hide allocation and possible
resource failure inside successful pattern matching, and would make ownership
and drop behavior backend-dependent. Borrow-only rest capture would instead
violate scrutinee-driven binder ownership and prevent ordinary consuming
destructuring.

## Consequences

The value/memory model must support disjoint element and backing-storage owner
fragments after atomic commit, while preserving exactly one owner per element,
ordinary borrow regions, and exact-once `Drop`/`AsyncDrop`. VM and JIT consume a
shared compiler split plan; runtime fragment layout remains private and cannot
be observed as value aliasing.
