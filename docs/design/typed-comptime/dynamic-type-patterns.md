# Dynamic Type Patterns

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Alias Patterns](alias-patterns.md) | [Next: Boolean Pattern Algebra](boolean-patterns.md)

## Decision 91: Exact Concrete Dynamic Refinement

Accepted: typed-pattern source is context-directed into one of three sealed
semantic forms:

1. If the target equals the ordinary static scrutinee type, it is an
   irrefutable typed `Binder`.
2. If the scrutinee is a closed union and the target is one exact member, it is
   `UnionMember`.
3. If the scrutinee is an open erased domain and the target is one fully
   reified concrete type, it is `ExactDynamicType`.

No form performs broad runtime kind compatibility, ignores generic arguments,
or invokes user conformance code.

The open-domain descriptor is conceptual:

```shape
opaque comptime type ReifiedRuntimeType<T>
opaque comptime type DynamicDomainRef<Erased>
opaque comptime type DynamicMemberEvidence<Erased, Concrete>

sealed comptime struct CheckedExactDynamicTypePattern<
    Erased,
    Concrete,
    Mode,
> {
    domain: DynamicDomainRef<Erased>,
    concrete: TypeRef<Concrete>,
    reified: ReifiedRuntimeType<Concrete>,
    membership: DynamicMemberEvidence<Erased, Concrete>,
    binder: PatternBinder<Concrete, Mode>,
}
```

`ReifiedRuntimeType<T>` proves that the complete semantic type has one canonical
execution descriptor and can be carried inside the erased domain. It is not a
native-kind tag, rendered type name, user hash, vtable pointer, or claim that a
trait implementation exists.

**TARGET - exact concrete refinement**

```shape
fn render(renderer: dyn Renderer) {
    match renderer {
        pdf: PdfRenderer => render_pdf(pdf)
        image: ImageRenderer => render_image(image)
        other => render_fallback(other)
    }
}
```

The first two arms compare exact execution type identities. They do not ask
whether the value implements `Renderer` at runtime; compile-time
`DynamicMemberEvidence` already proves that each concrete type is admissible in
the erased domain. A true catch-all such as `other` or `_` is mandatory because
the implementor set remains open.

**TARGET - closed union retains closed coverage**

```shape
match value: int | string {
    number: int => normalize_int(number)
    text: string => normalize_text(text)
}
```

These are `UnionMember` patterns, not dynamic type tests. Their union is
exhaustive because the scrutinee's type set is closed.

**TARGET - generic arguments are part of identity**

```shape
match erased_container {
    integers: Array<int> => sum(integers)
    names: Array<string> => join(names)
    other => unsupported(other)
}
```

This is legal only when the erased domain can carry both completely reified
container instantiations. `Array<int>` and `Array<string>` are distinct exact
identities. Testing only an outer array carrier is forbidden.

**TARGET - scrutinee-driven ownership refinement**

```shape
match &mut renderer {
    pdf: PdfRenderer => pdf.set_page(page)
    _ => {}
}
```

The successful binder is `&mut PdfRenderer`. Owned and shared scrutinees
similarly yield `PdfRenderer` and `&PdfRenderer` under Decision 76. Failure
commits no downcast, move, or loan.

**TARGET - typed generation**

```shape
let pdf = patterns.exact_dynamic_type(
    domain: renderer_domain,
    concrete: type_ref<PdfRenderer>(),
    reified: reified_type<PdfRenderer>(),
    membership: implements_renderer,
    binder: patterns.own_binder<PdfRenderer>(),
)
```

Every argument is compiler-issued and type-indexed. A generator cannot use a
type name, carrier kind, schema number, vtable address, uninstantiated generic,
or user-declared equality function.

**TARGET - required rejections**

```shape
match renderer {
    value: dyn Renderer => open_conformance(value)
    value: Printable => subtrait_membership(value)
    value: Array<_> => erased_generic(value)
    value: { page: int } => anonymous_structure(value)
}
```

Trait/dyn targets denote open sets, not exact identities. Generic holes are not
reified. Anonymous structural types have shape compatibility rather than one
portable nominal execution identity. Use an exact concrete type, a closed
union, a runtime classifier returning a closed value, or `_`.

Required rules:

1. `ExactDynamicType` tests equality with one complete compiler-issued runtime
   type identity. It never performs subtype, trait, protocol, structural, or
   native-kind compatibility.
2. Eligible targets are exact primitives, visible nominals, and fully
   instantiated core/container types for which the compiler issues
   `ReifiedRuntimeType<T>`. Type aliases normalize before identity formation.
3. Anonymous structural, open trait/dyn, union, intersection, function,
   reference, unresolved generic, inference-variable, and non-reified types
   cannot be exact dynamic targets.
4. `DynamicMemberEvidence<Erased, Concrete>` is proven statically from the
   erased domain contract. Runtime probing does not call an implementation,
   vtable, conversion, constructor, allocator, or user code.
5. The erased runtime value carries an execution descriptor sufficient to
   compare exact identity and recover the checked concrete carrier. Carrier
   layout alone is never evidence.
6. Successful ownership, shared-borrow, and exclusive-borrow refinements use
   the same root place and atomic commit rules as structural patterns. No copy,
   clone, reinterpret cast, or second drop obligation appears.
7. During a guard, the refined binder is a shared guard view. Guard false,
   failure, divergence, or cancellation leaves the erased value intact.
8. Each exact type contributes one singleton identity to open-domain coverage.
   Distinct complete type identities are disjoint. A true catch-all is required
   for exhaustiveness regardless of currently known implementations.
9. Listing every implementation visible in the current workspace never seals
   an open domain. LSP completion may suggest known admissible concrete types
   but cannot present that list as exhaustive evidence.
10. Private or hidden concrete types cannot be named without ordinary
    visibility. Exact identity does not grant `RepresentationAccess<T>` or
    permission to decompose opaque internals.
11. Generic targets are legal only after complete substitution. Their type and
    const arguments enter the exact execution descriptor, coverage identity,
    structure/denotation hashes, artifacts, and incremental dependencies.
12. Wire, snapshot, FFI, and remote erased values preserve or validate the same
    canonical execution descriptor. A peer missing the exact type/capability
    refuses admission before user execution under the Execution ABI rules.
13. The checked-structure hash includes the dynamic domain, concrete
    `TypeRef<T>`, reification/membership evidence identities, binder mode, and
    hygienic binder identity. Open coverage hashes exact identities plus an
    open remainder; source spelling, expansion origin, and native carrier
    layout are excluded.
14. VM, MIR JIT, and OSR consume one exact identity-test/refinement operation.
    Interpreter fallback cannot be the shipped semantic implementation.
15. LSP hover states `exact dynamic refinement`, the erased source domain,
    concrete result type/mode, open remainder, and generated provenance. It
    never labels carrier-kind compatibility as type safety.

Current migration implications:

- Replace annotation-only `Pattern::Typed` checking with context-directed
  semantic construction and scrutinee constraints before binding.
- Delete broad `type_check_kinded` behavior for pattern semantics: integer-kind
  families, outer-container-only checks, ignored generic arguments, and false
  `Dyn` branches do not survive as compatibility paths.
- Closed union matching, exact dynamic identity, and same-static-type binders
  share syntax but remain distinct typed constructors and coverage products.
- Introduce canonical runtime execution descriptors for erased values and one
  shared VM/MIR-JIT/OSR refinement operation.
- Positive and negative proofs cover primitives, nominals, complete generics,
  wrong generic arguments, open trait targets, privacy, ownership modes, guard
  failure, exact drops, hashes, wire/snapshot/remote admission, LSP, and native
  backend parity before the surface is enabled.
