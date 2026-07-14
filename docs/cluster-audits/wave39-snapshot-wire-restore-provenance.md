# Wave 39 Snapshot/Wire Restore Provenance Bridge

Date: 2026-07-10  
Role: Wave-39W snapshot/wire restore provenance proof closeout

## Verified Architecture

Wave-39K added the owning `serializable_to_kinded_slot[_ctx]` restore path.
`RestoreLinkCtx` carries one base/child share ledger, so restored
`HeapNode`/`HeapRef` identity is preserved across typed arrays, typed-object
maps, and nested values. Restore-side Miri provenance is retained through
typed-object fields, typed arrays, typed-object maps, `HeapNode`/`HeapRef`, and
`Result`/`Option` normalization. `ExecutionContext` snapshot restore and the
first three runtime probes use owning source carriers.

Wave-39Q added the owning `kinded_slot_to_serializable[_ctx]` source path. It
threads the Miri sidecar through outer pointers for `String`, `TypedObject`,
`TypedArray`, `HashMap`, `Reference`, and `SharedCell`, and through typed-object
field provenance. Raw pointer APIs fail closed under Miri. Inline
`Ptr(Char/Future/ModuleFn)` values are correctly exempt; unsupported
`StringV2`/`DecimalV2` and unsupported heap carriers refuse serialization.

## Evidence

The normal final filter passed 4/4:

```text
run-p2213279-i32890334.service
20.0s, peak 2.4G, MemorySwapMax=0
```

The final exact-source three-mode Miri filter passed 4/4 under Stacked Borrows,
Tree Borrows, and Strict Provenance:

```text
run-p2222475-i32899747.service
2m00s, peak 1G, MemorySwapMax=0
```

There were no target-path integer-to-pointer warnings. The earlier strict
failure `run-p2063073-i32736411.service` is retained only as root-cause
history; it is not current evidence.

The focused runtime filter covers shared `HeapNode`/`HeapRef` typed-object
identity, typed-object elements in typed arrays, shared typed-object values in
maps, and legacy `Result`/`Option` normalization through the serde wire
round-trip.

## Proof Boundary

This is targeted snapshot/wire evidence, not whole-runtime UB-free evidence.
VM stack/module raw writers, closure/shared-cell nested interiors,
state/resume, remote serialization consumers, JIT/FFI, all
`SerializableVMValue` arms, and arbitrary programs remain Stage 2/open. In
particular, this does not claim full public `state.resume` or general snapshot
resumability.
