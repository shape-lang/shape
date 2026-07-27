# ADR-003: Method Registry Single-Source -- Unified Method Descriptors

## Status

Accepted (2026-02-19)

Clarified by ADR-010 and ADR-011 (2026-07-25): method semantics are selected by
resolved definition identity and carry the exact callable lifecycle contract.
Names are lookup/presentation data, and dense numeric method IDs are local
execution handles rather than portable identities.

## Context

Method capabilities for Shape's built-in types are currently described in two
independent registries that must be kept in sync manually:

### Type-checker registry (`shape-runtime/src/type_system/checking/method_table.rs`)

- `MethodTable` struct with a `HashMap<(String, String), Vec<MethodSignature>>`.
- Populated imperatively in `register_builtin_methods()` via repeated
  `self.register_method(receiver, name, param_types, return_type, is_fallible)` calls.
- Used at compile time to validate method calls and infer return types.
- Many Table/query methods are typed as `any` -> `any` (line 166+), providing
  no useful type narrowing.

### Runtime dispatch registry (`shape-vm/src/executor/objects/method_registry.rs`)

- PHF (perfect hash function) maps: `ARRAY_METHODS`, `DATATABLE_METHODS`, etc.
- Maps method name strings to `MethodFn` handler function pointers.
- Used at runtime to dispatch method calls on receiver objects.
- 47 array methods, plus DataTable, String, and typed-object methods defined
  in separate PHF maps.

### Problems

1. **Drift risk**: a method can exist in the runtime PHF map without a
   corresponding type-checker entry, or vice versa. The type checker may accept
   a call the runtime cannot handle, or reject one it can.

2. **Duplicate maintenance**: adding a new method requires edits in at least two
   files with different formats (imperative registration vs. PHF map literal).
   The information (method name, parameter count, receiver type) is stated
   independently in both places.

3. **Weak typing in type checker**: Table methods are typed as `any` -> `any`
   (per audit finding in `04-architecture-maintainability-dry.md`), defeating
   the purpose of static checking. This is partly because maintaining precise
   types in a separate registry is high-effort with no enforcement.

4. **String-based dispatch**: runtime method lookup uses string method names
   in hot paths (`shape-vm/src/executor/objects/mod.rs:115`), even though the
   compiler already knows the receiver type and method identity at compile time.

## Decision

### 1. Single declarative method descriptor source

Introduce one resolved method-descriptor source (location implementation
dependent) that declares every built-in method once:

```rust
pub struct MethodDescriptor {
    pub identity: MethodIdentity,
    pub receiver: TypeIdentity,
    pub display_name: &'static str,
    pub contract: CallableLifecycleContract,
    pub implementation: MethodImplementationIdentity,
}
```

`MethodIdentity` is derived from the resolved declaration and exact callable
contract under ADR-011. The contract includes parameters, result, receiver
mode, ownership/write-back disposition, effects, failure outcomes, and
lifecycle ABI. Display names never select behavior. Core descriptors may be
compiled into static tables, but those tables are projections of the resolved
semantic source.

### 2. Generate both registries from descriptors

- **Type checker**: consumes the descriptor's exact callable contract. No
  hand-written parallel signature registration exists.
- **Runtime dispatch**: admission resolves the implementation identity to a
  dense local handle or relocation and validates it against the same contract.
  Any implementation without a descriptor, or descriptor without an
  implementation, is a compile error.
- **Dynamic reflection**: an explicitly dynamic name lookup may resolve text
  to a descriptor before invocation. It does not bypass resolution or become
  the static-call path.

### 3. Introduce `MethodId` for typed dispatch

Each admitted artifact/runtime generation may intern a `MethodIdentity` into a
dense numeric `MethodId` for O(1) dispatch:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MethodId(pub u16); // local handle, not semantic identity

// Runtime lookup becomes:
static ARRAY_DISPATCH: [MethodFn; N] = [/* indexed by MethodId */];
```

Portable bytecode and artifacts bind the canonical method identity and exact
contract. A loader may relocate that identity to a local `MethodId`; serialized
meaning never depends on the numeric table position. String-based lookup is
retained only for explicit dynamic/reflective dispatch paths.

### 4. Strengthen Table method types

With a single descriptor source, Table method types are upgraded from `any` to
precise signatures. Receiver mode, ownership/write-back, effects, failures, and
lifecycle outcomes are part of the same contract rather than side tables.
Conceptually:

```text
MethodDescriptor {
    identity: resolved_method_identity(Table, #filter),
    receiver: type_identity(Table),
    contract: fn(&Table, fn(Row) -> bool) -> Table ! {},
    implementation: resolved_implementation_identity(table_filter),
}
```

### 5. No method can exist in only one registry

This is enforced structurally: checking facts and runtime adapters are both
derived from the same resolved descriptor. Adding a method means adding one
typed declaration/descriptor and one matching implementation. Name sets,
receiver-mutation sets, and backend-specific method classifiers are forbidden
as parallel semantic registries.

## Consequences

### Positive

- **Zero drift**: it is structurally impossible for the type checker and runtime
  to disagree on which methods exist or their arity.
- **Single edit point**: adding a method is one descriptor + one handler. No
  synchronized edits across files.
- **Typed dispatch**: a relocated `MethodId` eliminates string hashing in hot
  paths without pretending that a table ordinal is portable identity.
- **Better type inference**: precise method types propagate through the type
  system, enabling downstream optimizations and better error messages.
- **Auditability**: a single descriptor list serves as human-readable
  documentation of every built-in method's contract.

### Negative

- **Upfront migration cost**: existing `register_builtin_methods()` calls
  (~100+ entries) and PHF maps must be converted to descriptor format.
- **Build complexity**: if a build script generates the PHF map from
  descriptors, the build becomes slightly more complex.
- **Identity/versioning cost**: portable method identities and callable
  contracts must be hash-covered and admitted; local `MethodId` values may
  change between runtime generations.

### Risks

- **Incomplete migration**: if some methods remain hand-registered during
  migration, drift can still occur. Mitigation: CI check that counts methods
  in each registry and asserts equality.
- **MethodId exhaustion**: a local `u16` table supports 65536 admitted methods.
  Exhaustion is an admission failure or a reason to widen the local handle; it
  does not change semantic identity.

## References

- Audit: `shape/docs/audits/2026-02-19-shape-state/04-architecture-maintainability-dry.md`
- Audit: `shape/docs/audits/2026-02-19-shape-state/05-performance-bottlenecks.md`
- Type-checker registry: `shape/shape-runtime/src/type_system/checking/method_table.rs`
- Runtime dispatch registry: `shape/shape-vm/src/executor/objects/method_registry.rs`
- String dispatch hot path: `shape/shape-vm/src/executor/objects/mod.rs:115`
- ADR-010: callable lifecycle contracts and admission
- ADR-011: resolved semantic identity and typed elaboration
