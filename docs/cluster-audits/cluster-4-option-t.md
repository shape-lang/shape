# Cluster 4 audit — Option<T> / SomeObjectPairs marshal residual

**Scope**: `crates/shape-runtime/src/typed_module_exports.rs` (TypedReturn /
ConcreteReturn / ConcreteType), `crates/shape-runtime/src/stdlib/regex.rs`,
plus any future stdlib body that returns `Some(typed_object)` /
`Ok(typed_object)` / `Err(typed_object)` (no current additional consumers
in shape-runtime --lib per audit).

**Predicted error drop**: 0 to -2 in shape-runtime --lib. The β shape
has already landed (`SomeObjectPairs` / `OkObjectPairs` /
`ErrObjectPairs` variants in commit `ed18cb8`; regex.match / regex.find
activated in `c07d18e` per defections.md, 2026-05-07 audit-grounded
correction). The architectural cluster is **closed** — what remains is
either deferred-consumer activation (no errors today) or a **dispatcher
projection follow-on** in shape-vm.

**Audit performed by**: scout-2026-05-07

## Audit 1 — consumer call shape

The Phase 2d handover lists Cluster #4 as carried-forward, but the
2026-05-07 audit-grounded correction subsection at
`docs/defections.md:4877-4972` records the cluster as **LANDED** with
β shape (3 flat per-wrapper variants). The handover text wasn't updated
in place; the actual current state is post-landing.

### Currently-active consumers

- `crates/shape-runtime/src/stdlib/regex.rs:52, 75` — `regex.match` /
  `regex.find` returning `Option<{matched: string, captures: Array<string>}>`.
  Calling shape:

  ```rust
  Some(m) => Ok(TypedReturn::SomeObjectPairs(match_to_pairs(&m, &caps)))
  None    => Ok(TypedReturn::None)
  ```

  Body produces `Vec<(String, ConcreteReturn)>` for the pair-list, exactly
  matching β's flat-variant payload contract. **Active and working.**

### Originally-blocked consumers (now reclassified or self-resolved)

Per `docs/defections.md` 2026-05-07 audit-grounded correction (b)+(c):

- `arrow_module` typed-row returns — **reclassified to Phase 2d Array
  cluster**, not Cluster #4. Already migrated via
  `ConcreteReturn::ArrayHeapValue` (commits `9fc35ac`/`9f6b1d3`/`29d61fa`).
- `csv_module` typed-row returns — same reclassification; migrated via
  `ConcreteReturn::ArrayHeapValue`.
- `regex.match` / `regex.find` — activated by `c07d18e`.

The audit-grounded correction's (c) finding: **0 currently-blocked
shape-runtime --lib errors** at the time of cluster close.

### Remaining latent consumers

Per N6 / N8 / B1 deferrals:

- HashMap-marshal cluster's `Some(HashMapStringHeapValue)` returns —
  not yet wired but covered by `TypedReturn::Some(ConcreteReturn::HashMapStringHeapValue(...))`
  (the leaf-only ConcreteReturn now includes HashMapStringHeapValue
  per Stage C P1(b) landing).
- Future structured-error returns (`Err(typed_object)`) — covered by
  `ErrObjectPairs`.
- B1 JsonValue's `Option<JsonValue>` shape — covered by
  `TypedReturn::Some(ConcreteReturn::JsonValue(...))`.

All three above are **architecturally covered** by the β shape; they're
mechanical migrations when the dependent cluster lands, not Cluster #4
sub-decisions.

### Dispatcher projection (shape-vm side)

`crates/shape-vm/src/executor/vm_impl/modules.rs:208`:
```rust
typed_result.map(|t| t.into_value_word())
```

`TypedReturn::into_value_word()` was deleted per
`typed_module_exports.rs:235`. The remaining shape-vm dispatcher call
site is **broken** — but this is part of the shape-vm cascade's
pre-existing breakage tail (per Phase 2d handover and Phase 2c
defections), not a Cluster #4 architectural question. The
`TypedReturn::SomeObjectPairs` / `OkObjectPairs` / `ErrObjectPairs`
variants are defined and constructed correctly; **the dispatcher
projection from `TypedReturn` → typed slot bits is the cluster #4
DOWNSTREAM question that must be answered when shape-vm cascade
runs.**

## Audit 2 — marshal-API readiness

### What's already in place (β LANDED)

- `TypedReturn::SomeObjectPairs(Vec<(String, ConcreteReturn)>)`
  (typed_module_exports.rs:215)
- `TypedReturn::OkObjectPairs(Vec<(String, ConcreteReturn)>)`
  (typed_module_exports.rs:221)
- `TypedReturn::ErrObjectPairs(Vec<(String, ConcreteReturn)>)`
  (typed_module_exports.rs:226)
- `TypedReturn::None` (typed_module_exports.rs:202) — the absent
  constructor.
- `TypedReturn::Some(ConcreteReturn)` — the present constructor for
  non-typed-object `Some` payloads (e.g. `Some(int)`, `Some(string)`).
- `ConcreteType::Option(Box<ConcreteType>)` (line 310) — return-type
  descriptor used at registration; the LSP-side type-name is
  `Option<inner.shape_type_name()>`.
- `ConcreteType::Result(Box<ConcreteType>)` /
  `ConcreteType::Result2(Box<ConcreteType>, Box<ConcreteType>)` —
  Result variants used identically.
- regex.rs's `match_to_pairs` helper builds the pair-list.

### What's missing (residual surface)

- **Dispatcher projection at shape-vm side**: there is no current
  `TypedReturn::SomeObjectPairs → ValueWord` (or `→ typed slot bits`)
  projection — the shape-vm `into_value_word()` call site is broken
  pre-Cluster-#4 and stays broken until the shape-vm cascade lands.
  The cluster's β architectural shape is correct, but the projection
  step needs to handle:
  1. Running `lookup_schema_for_fields(&[field_names])` to resolve the
     anonymous TypedObject schema (mirrors `typed_object_from_pairs`'s
     existing logic).
  2. Calling `typed_object_from_pairs` with the resolved schema.
  3. Wrapping the resulting `ValueWord`/`Arc<HeapValue>` in the Some /
     Ok / Err discriminator (the **Option / Result runtime
     representation question**).
- **Option / Result runtime representation**: `Option<T>` / `Result<T,E>`
  on the runtime side are encoded today as either:
  - `NativeKind::NullableInt64` / `NullableFloat64` (sentinel-NaN, etc.)
    — leaf scalars only.
  - Heap-pointer-with-null-discriminator — `null` raw bits = None;
    valid `Arc<HeapValue>` = Some.
  - Shape-side enum representation (`builtin type Option`,
    `intrinsics.shape:31`).

  The dispatcher projection step needs to pick **one** of these for
  `Some(typed_object)`. The most likely answer is the heap-pointer-
  with-null-discriminator (because typed objects are heap-resident),
  but this question is **owned by the shape-vm cascade**, not by the
  shape-runtime cluster.

## Architectural-shape options

The architectural decision for the **marshal-layer surface** has
already landed (β). Three sub-decisions remain for the **dispatcher
projection** that the shape-vm cascade will need to answer.

### Option SP-α — Pre-materialize ValueWord in the dispatcher

**Shape**: dispatcher's `TypedReturn::SomeObjectPairs(pairs) → slot bits`
projection runs `typed_object_from_pairs(&pairs)` to materialize the
TypedObject as `ValueWord::TypedObject{schema_id, slots, heap_mask}`,
then wraps the resulting `Arc<HeapValue>` raw pointer in the slot's
typed-pointer kind.

**Pros**:
- Reuses `typed_object_from_pairs` end-to-end — no parallel
  schema-walk code.
- Mirrors the `ConcreteReturn::OpaqueTypedObject` projection that
  `json.__parse_typed` already uses (Stage B+D close-out, N8 sign-off).
- One-step projection in the dispatcher.

**Cons / risks**:
- Forces the dispatcher to depend on `typed_object_from_pairs`'s
  current ValueWord input type — interlocks with cluster #1's
  signature flip. SP-α is "fine until cluster #1 lands; rewrites
  when it does."
- Doesn't address the Option / Result discriminator encoding on
  its own; still needs a discriminator-bit choice.

**Effort**: 2-3 days (when shape-vm cascade runs).

### Option SP-β — Heap-pointer-with-null-discriminator

**Shape**: `Some(payload)` projects payload to its typed slot bits
(per the inner `ConcreteType`'s rules), `None` projects to a sentinel
null-pointer raw bits. Caller-side Shape code pattern-matches via
`if x != null { ... }` (existing flow-sensitive narrowing — see CLAUDE.md
"Flow-sensitive narrowing").

**Pros**:
- Uses an existing language mechanism (null-narrowing).
- Zero discriminator-bit overhead.
- Same shape as `Option<DataTable>` / `Option<IoHandle>` / etc. would
  use uniformly.

**Cons / risks**:
- Conflates `Option<T>` with `T?` (T-or-null) at the runtime layer.
  This is **already the language's choice** at the surface level
  (no separate Option enum at the bytecode level today), so it
  may not be a "con" — but the architectural question is whether
  cluster #4's dispatcher projection cements this conflation.
- Doesn't easily extend to `Result<T, E>` (which has two payloads,
  not one).

**Effort**: 1-2 days.

### Option SP-γ — Shape-side enum representation (TypedObject with discriminant slot)

**Shape**: `Some(payload)` and `None` materialize as `TypedObject` with
a `_variant: i64` discriminant slot + a payload slot. Same for
`Ok` / `Err`. Mirrors the Shape-side `enum Option { Some(T), None }`
declaration's natural representation.

**Pros**:
- Strict-typed: every Option / Result value has the same TypedObject
  shape regardless of what `T` is.
- Pattern-matches user-side `match opt { Some(x) => ..., None => ... }`
  code without special-casing.
- Extends naturally to `Result<T, E>`, `enum Foo { ... }`, etc.

**Cons / risks**:
- Requires schema registration for every `Option<T>` / `Result<T, E>`
  instantiation — schema-id explosion or anonymous-schema indirection.
- TypedObject layout overhead for what's currently a one-bit
  discriminator — perf regression vs SP-β.
- **Watchlist match (partial)**: "Smaller subset enum of NativeKind
  for Option-only" pattern — SP-γ is the reverse direction
  (representing Option as a **larger** TypedObject), but the parallel-
  discriminator drift risk is mirror-imaged: every consumer that takes
  an Option<T> needs to know the schema_id.

**Effort**: 4-5 days.

### Option SP-δ — Discriminator-tagged native pair (NativeKind extension)

**Shape**: introduce `NativeKind::Option(Box<NativeKind>)` /
`NativeKind::Result(Box<NativeKind>, Box<NativeKind>)` as parametric
typed kinds; slot bits carry a discriminator + payload.

**Pros**:
- Tight runtime representation.
- No schema explosion.

**Cons / risks**:
- **DIRECTLY MATCHES CLAUDE.md / Phase 2d-handover Forbidden Pattern**:
  "Parametric NativeKind variants for Array/Option/JsonValue/IoHandle.
  Adding `NativeKind::TypedArrayI64` / `NativeKind::Option(...)` /
  `NativeKind::JsonValue` is the rejected pattern." Already explicitly
  refused on the Cluster #4 watchlist (`docs/defections.md:4975-4977`).
- **REJECT ON SIGHT.**

**Effort**: N/A (forbidden).

## Recommendation

**Rank**: SP-β > SP-α > SP-γ; SP-δ rejected.

If I were the supervisor I'd take SP-β: heap-pointer-with-null-
discriminator for `Some(typed_object)` / `None`, and pair-tagged
heap pointer for `Ok` / `Err` (extending the discriminator to two-bit
values). This matches Shape's existing language-level conflation of
`Option<T>` and `T?` (per CLAUDE.md flow-sensitive narrowing) and avoids
both the schema-id explosion of SP-γ and the Forbidden NativeKind
parametrization of SP-δ. SP-α is acceptable as an intermediate step —
defer the discriminator encoding to the dispatcher's projection step
and use `ConcreteReturn::OpaqueTypedObject` semantics for now.

**This decision is owned by shape-vm cascade**, not by the shape-runtime
Cluster #4 surface (which is closed). The architectural-shape options
are surfaced here so the supervisor can disposition them when the cascade
runs.

## Open questions for supervisor

1. **(A/B)** Is Cluster #4 considered closed (β LANDED 2026-05-07,
   `ed18cb8`/`c07d18e`), and the residual is only the shape-vm
   dispatcher projection? Or are there latent shape-runtime --lib
   consumers that still need a Cluster #4 sub-decision?
2. **(SP-α/SP-β/SP-γ)** Which dispatcher projection shape gets named
   when the shape-vm cascade reaches `TypedReturn::SomeObjectPairs`?
   This is a leaf decision but it interlocks with the
   Option/Result/sum-type runtime representation broadly.
3. **(yes/no)** Should SP-β extend to `Result<T, E>` via a 2-bit
   discriminator slot (Some/None/Ok/Err) or via separate
   pointer-vs-error-pair shapes per wrapper? The marshal-side
   variants are already split (3 wrapper variants); the runtime-side
   discriminator can be either unified or per-wrapper.
4. **(yes/no)** Is the currently-broken `into_value_word()` call at
   `crates/shape-vm/src/executor/vm_impl/modules.rs:208` part of
   Cluster #4's downstream surface, or part of the broader shape-vm
   cascade's `TypedReturn → slot bits` projection (which is then a
   superset cluster of multiple wrapper-variant projections)?
5. **(yes/no)** When Cluster #5 (JsonValue marshal) lands, does it
   need its own dispatcher projection sub-decision, or does it
   inherit Cluster #4's SP-N choice automatically?
