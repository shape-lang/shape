# Wave 46: Typed Comptime First Tracers

Date: 2026-07-11

## Scope

This wave moved two accepted ADR-009 invariants into executable compiler
behavior:

1. generated code cannot acquire an undeclared closure environment; and
2. comptime type categorization no longer requires a public string kind or a
   user-constructible type descriptor.

It is intentionally not a claim that the complete typed comptime algebra is
implemented.

## Landed

### Generated Capture Boundary

Annotation/comptime-generated functions and methods are marked before their
bodies compile. Closure lowering checks that marker after deterministic capture
discovery. A non-empty inferred capture set is a compile-time semantic error:

```text
generated closure implicitly captures 'value'; generated captures must be explicit
```

The previous public proof accepted the generated function and returned `Null`.
The new proof fails before runtime. A focused ordinary source-closure test still
passes, so this does not change current source closure inference.

### Typed Type Identity and Category

`type_ref(T)` accepts bare compiler-resolved type syntax. The comptime rewrite
maps that syntax to a canonical 128-bit SHA-256 descriptor fingerprint in the
immutable type-reflection snapshot. Its runtime carrier and native forwarding
entry have unspellable names and expose only the two fingerprint halves; source
code cannot forge it and strings are rejected. Adding unrelated declarations
cannot renumber existing identities.

`type_category(type_ref(T))` returns the schema-backed exhaustive enum:

```shape
pub enum FrozenTypeCategory {
    Primitive,
    Never,
    Parameter,
    Nominal,
    Tuple,
    Record,
    Callable,
    Reference,
    Union,
    Erased,
}
```

The enum has no `Unknown`, `Any`, or string arm. Unresolved names fail the
freeze boundary. Transparent aliases map to the underlying identity. Active
function generic parameters are added to the snapshot and receive scoped
`Parameter` identities. Primitive and user-nominal proofs are enabled, and
compiler/LSP metadata exposes both builtins with completion and hover
documentation.

The comptime-to-runtime materialization boundary rejects raw `TypeRef` and
`FrozenTypeCategory` values. They are compiler capabilities/reflection data,
not `ConstLift` values; callers must consume them inside comptime and lift an
ordinary closed result instead.

LSP completion now recognizes ordinary comptime blocks, comptime functions,
and annotation `comptime pre/post` hooks. Enum namespace completion is a
general `Enum::prefix` path for user enums, with `FrozenTypeCategory` variants
sourced from the same runtime-owned catalog used by the compiler and schema.

The reflection implementation moved into
`compiler/comptime_builtins/type_reflection.rs` (under 500 lines), reducing
growth pressure on the pre-existing oversized `comptime_builtins.rs`.

## Verification

- Generated implicit capture red proof: previously returned `Null`; now passes
  as an expected compile error.
- Public typed-reflection behavior and rejection matrix: 16 passed, covering
  VM/JIT exhaustive consumption, primitive/never/erased/nominal reachability,
  enums/builtin nominals, alias chains, arity/type/stage errors, legacy-carrier
  rejection, non-liftable capabilities, and exhaustiveness.
- Reflection identity unit matrix: 9 passed, covering active generic-function
  discovery, function-scoped parameters, synonym/alias normalization,
  declaration-order stability, distinct nominals, and unknown identities.
- Runtime catalog and opaque schema matrix: 3 passed.
- Generated capture matrix: 8 passed, covering generated free functions and
  methods, local/parameter/multiple/`self` captures, deterministic ordering,
  capture-free closures, closure parameters, VM/JIT consumers, and ordinary
  source closure control behavior.
- `annotations_comptime`: 60 passed, 0 failed with two test threads.
- `comptime`: 109 passed, 0 failed with two test threads.
- Typed comptime LSP completion/hover/signature/diagnostic/enum matrix: 14
  passed, 0 failed; the neighboring completion-context unit module passed
  58/58.
- The combined broad ShapeTest gate passed 568/568: 60 annotation-comptime,
  109 comptime, and 399 LSP tests. Peak memory was 3.1G with swap disabled.
- All gates ran under the single cgroup lane with swap disabled. The first
  unconstrained-harness run hit `TasksMax=256`; the deterministic rerun used
  `--test-threads=2` and passed.

## Still Missing

1. Payload-bearing and type-indexed `FrozenType<T>` descriptors for all ten
   categories.
2. The sealed `FrozenPrimitive<T>` sub-algebra and exact width/domain payloads.
3. Native type-expression syntax that can form `TypeRef` for tuples, records,
   callables, references, unions, erased domains, and applied generic types.
4. Complete semantic normalization for applied nominals, object intersections,
   and trait intersections on every call path. The declared-generic-parameter
   half of this gap is CLOSED (2026-07-12, ADR009-A3): the generic-call
   specialization gap is fixed — a scoped compiler overlay carries the base
   definition's declared type parameters into the reflection snapshot when the
   monomorphized body compiles, and hard specialized-body compile errors now
   propagate instead of being masked as "cannot infer type argument(s)".
   Public e2es (VM+JIT) in `tools/shape-test/tests/comptime/frozen_type.rs`:
   `generic_body_observes_parameter_category_for_its_own_type_param`
   (positive `Parameter` proof),
   `undeclared_name_in_generic_body_still_fails_the_freeze` (negative:
   `type_ref(U)` in `fn f<T>` → "unknown semantic type identity"),
   `parameter_category_is_stable_across_instantiations_of_one_generic_fn`,
   `distinct_generic_fns_each_observe_parameter_for_their_own_type_param`,
   plus the full rejection matrix re-fired inside generic bodies
   (`*_inside_generic_bodies` / `*_generic_bodies_*` tests). Specialization
   unit pins in
   `crates/shape-vm/src/compiler/comptime_builtins/type_reflection/tests.rs`
   and `crates/shape-vm/src/compiler/monomorphization/cache.rs`; LSP
   generic-body matrix in `tools/shape-test/tests/lsp/typed_comptime.rs`.
5. Public `CaptureDescriptor<Sig, I, T, Mode>`, heterogeneous capture packs,
   `CheckedBody<Sig, Captures>`, and `CheckedTemplate<Sig, Captures>`.
6. Explicit generated closure capture syntax/builders and full ownership,
   lifetime, suspension, `Send`, cleanup, `Drop`, and `AsyncDrop` proofs.
7. Migration of legacy `type_info`, string/JSON/source generation directives,
   string-backed older descriptor rows, and runtime-hook `Any` carriers.
8. Generated-symbol/source-map integration beyond metadata-backed builtin
   completion and hover.

These gaps remain compile-time capability gaps. This wave does not add runtime
fallbacks or claim partially populated descriptors.
