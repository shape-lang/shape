# ADR-007: Arbitrary-Precision Exact Decimal

## Status

Accepted (2026-07-11)

## Decision

Shape `decimal` denotes canonical arbitrary-precision values with finite
base-10 expansions. It is not specified by the current `rust_decimal` carrier,
and fixed precision/scale remain nominal typed-library or comptime-generated
constraints rather than core-language type parameters.

## Context

Typed range-pattern design exposed that the current 96-bit, scale-28,
16-byte carrier is a finite irregular domain, while the intended decimal
semantics require exact canonical equality and a stable topology independent of
one Rust dependency. Making the carrier normative would leak implementation
limits into constants, comptime hashes, exhaustiveness, wire values, snapshots,
FFI, and the execution ABI. Making `decimal<P, S>` the core type would instead
specialize the language around storage/schema constraints that typed libraries
and generators can express nominally.

## Consequences

Decimal equality, ordering, hashing, constants, and ranges use canonical exact
numeric values; the domain is unbounded and dense. The runtime may optimize
small values but must replace the current fixed carrier and canonicalize wire,
snapshot, artifact, FFI, and remote representations under a new Execution ABI
ID. Division, rounding, resource limits, and the concrete fixed-decimal library
surface require separate decisions and cannot inherit host-crate behavior.
