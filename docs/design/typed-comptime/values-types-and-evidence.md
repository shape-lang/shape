# Values, Types, And Evidence

[Back to the typed comptime overview](../typed-comptime.md) | [Next: Nominals And Members](nominals-and-members.md)

## Decision 47: Values Versus Code

Accepted: ordinary comptime values and generated code are separate typed
categories with explicit lifting.

**TARGET - proposed positive example**

```shape
comptime let count: int = 3

comptime let literal: CheckedExpr<int> = expr.literal(count)
comptime let plus_one: CheckedExpr<int> =
    expr { literal + 1 }
```

`count` is a comptime value. `literal` and `plus_one` are code fragments. The
contextual expression block accepts `CheckedExpr<int>` directly. Ordinary
comptime data enters generated code only through an explicit `ConstLift`
operation such as `expr.literal`.

**TARGET - required rejection**

```shape
comptime let source: string = "user.name"
comptime let generated = CheckedExpr.parse(source)
```

Required diagnostic: no text-to-code constructor exists. A string is data, not
an expression fragment.

## Decision 48: Canonical Type Identity

Accepted: `TypeRef<T>` is an opaque compiler-issued identity. It has no
text-to-type constructor and exposes no comparable semantic string.

**TARGET - proposed positive example**

```shape
comptime fn optional<T>(_: TypeRef<T>) -> TypeRef<Option<T>> {
    type_ref<Option<T>>()
}

comptime let user: TypeRef<User> = type_ref<User>()
comptime let maybe_user: TypeRef<Option<User>> = optional(user)
comptime let info: FrozenType<User> = reflect(user)
```

Legal origins are type syntax, generic substitution, and other typed compiler
descriptors. `reflect` preserves `T` in its result. Diagnostic code may pass a
`TypeRef<T>` directly to a diagnostic sink such as `ctx.note`, but cannot
recover a string suitable for equality or reconstruction.

**TARGET - required rejection**

```shape
comptime let parsed = TypeRef.parse("Option<User>")

if target.return_type.source == "User" {
    // Perform a semantic rewrite.
}
```

Required diagnostics: no text-to-type constructor exists, and type display is
diagnostic-only. Artifact serialization uses the canonical checked type
descriptor rather than an in-memory identity or rendered spelling. VM and JIT
therefore consume the same resolved type metadata.

## Decision 49: Trait Identity And Implementation Evidence

Accepted: traits use a distinct opaque `TraitRef<Tr>` kind. Implementation
lookup returns typed evidence rather than a boolean.

**TARGET - proposed positive example**

```shape
comptime let user: TypeRef<User> = type_ref<User>()
comptime let serializable: TraitRef<Serializable> =
    trait_ref<Serializable>()

comptime match find_impl(user, serializable) {
    Some(proof: ImplRef<User, Serializable>) => {
        generate_serializer(user, proof)
    }
    None => {
        ctx.error("User must implement Serializable")
    }
}
```

`ImplRef<T, Tr>` is compiler-issued evidence tied to the exact type and trait.
Checked builders that emit constrained declarations or trait dispatch consume
the evidence directly, so the successful fact remains scoped to the successful
match branch.

**TARGET - required rejection**

```shape
comptime let trait_name = "Serializable"

if implements(user, trait_name) {
    generate_serializer(user)
}

comptime let serializable: TypeRef<Serializable> =
    type_ref<Serializable>()
```

Required diagnostics: traits are not value types, trait lookup cannot use text,
and a boolean cannot authorize an operation that requires implementation
evidence. The compiler hashes canonical trait and implementation identities
into generated artifacts; VM and JIT never repeat name-based lookup.

## Decisions 50 and 94: Frozen Type Algebra and Final Catalog

Accepted: reflecting `TypeRef<T>` returns a sealed exhaustive indexed sum
`FrozenType<T>`, not a universal reflection record with a string category and
optional fields.

The final semantic catalog is:

```shape
sealed comptime enum FrozenType<T> {
    Primitive(FrozenPrimitive<T>),
    Never(FrozenNever<T>),
    Parameter(TypeParamDescriptor<T>),
    Nominal(FrozenNominal<T>),
    Tuple(FrozenTuple<T>),
    Record(FrozenRecord<T>),
    Callable(FrozenCallableType<T>),
    Reference(FrozenReference<T>),
    Union(FrozenUnion<T>),
    Erased(FrozenErased<T>),
}
```

`FrozenPrimitive<T>` is itself sealed and exhaustive over unit, bool, char,
signed and unsigned integer families, binary floating-point families, exact
decimal, string, null, and undefined. Width and other semantic parameters are
typed descriptor data, not rendered names.

**TARGET - proposed positive example**

```shape
comptime fn derive<T>(
    ty: TypeRef<T>,
    ctx: ComptimeContext,
) -> CheckedItem<Derived<T>> {
    match reflect(ty) {
        FrozenType::Primitive(primitive) =>
            derive_primitive(primitive),
        FrozenType::Never(never) =>
            ctx.error(never, "This derive cannot construct never"),
        FrozenType::Parameter(parameter) =>
            derive_parameter(parameter),
        FrozenType::Nominal(nominal) =>
            derive_nominal(nominal),
        FrozenType::Tuple(tuple) =>
            derive_tuple(tuple),
        FrozenType::Record(record: FrozenRecord<T>) =>
            derive_record(record),
        FrozenType::Callable(callable: FrozenCallableType<T>) =>
            derive_callable(callable),
        FrozenType::Reference(reference) =>
            derive_reference(reference),
        FrozenType::Union(union) =>
            derive_union(union),
        FrozenType::Erased(erased) =>
            ctx.error(erased, "This derive does not support erased types"),
    }
}
```

Every payload retains `T`; category-specific operations become available only
after matching the corresponding variant. The freeze boundary applies these
canonicalization rules before constructing the sum:

1. Arrays, collections, `Option`, `Result`, `Future`, and every other applied
   builtin or user type are `Nominal` values with typed type/const arguments.
2. Struct, enum, newtype, and opaque structure is exposed only through the
   `NominalShape` carried by `FrozenNominal<T>`.
3. Structural object intersections normalize to `Record`; trait intersections
   become the bound set of `Erased`. Source intersection spelling does not
   survive as a semantic variant.
4. Explicit erased domains such as `any` and `dyn Trait` are `Erased`. Internal
   `Any`, unknowns, dynamic-schema fallbacks, and inference variables cannot
   cross the freeze boundary.
5. Transparent aliases normalize to their underlying semantic type. Traits
   themselves use `TraitRef`, and const parameters use value descriptors; they
   are not `FrozenType` variants.

**TARGET - required rejection**

```shape
comptime let info = reflect(type_ref<T>())

if info.kind == "record" {
    for field in info.fields ?? [] {
        generate_field(field)
    }
}
```

Required diagnostics: `FrozenType<T>` has no string kind, nullable category
fields, or `Any` payload. A new source-visible category changes the comptime ABI
and exposes consumers that neither handle nor explicitly reject it. Canonical
variant and payload descriptors, not rendered data, enter expansion hashes and
the common VM/JIT artifact metadata.

## Decision 51: Heterogeneous Descriptor Collections

Accepted: heterogeneous reflection collections use ordinary language-level
existential packages. Lexical `some` bindings provide ergonomic typed access;
they are not a compiler-only dynamic iterator.

**TARGET - proposed positive example**

```shape
type SomeField<Owner> =
    exists<I, F> FieldDescriptor<Owner, I, F>

comptime let FrozenType::Record(record) =
    reflect(type_ref<User>())

comptime for some<I, F> field in record.fields {
    comptime let field_type: TypeRef<F> = field.type_ref
    comptime let read: CheckedExpr<F> =
        expr.field(receiver, field)

    output.push(generate_field(field, read))
}
```

Each iteration introduces fresh hidden `I` and `F`. The same ordinary
existential mechanism represents tuple elements, enum variants, callable
parameters, and union members:

```shape
Array<exists<I, T> TupleElement<I, T>>
Array<exists<I, P> VariantDescriptor<Owner, I, P>>
Array<exists<I, T, Mode> ParamDescriptor<Sig, I, T, Mode>>
Array<exists<T> UnionMember<T>>
```

**TARGET - required rejection**

```shape
Array<FieldDescriptor<User, int, Any>>

comptime for some<I, F> field in record.fields {
    escaped = field.type_ref
}
use_type(escaped)
```

Required diagnostics: heterogeneous witnesses cannot be erased to `Any`, and
the hidden `F` cannot escape its opening scope unless explicitly repackaged in
an existential value. The core operation is a rank-2 generic callback; the
`for some` form is syntax sugar and therefore does not introduce a second
reflection protocol.

## Decision 52: Semantic Freeze Boundary

Accepted: comptime reflection and annotation specialization begin only after
the target's semantic types are completely resolved. Declared generic
parameters are valid stable semantic identities; solver holes and storage
fallbacks are not.

**TARGET - proposed positive example**

```shape
comptime fn derive<T>(
    ty: TypeRef<T>,
    ctx: ComptimeContext,
) -> CheckedItem<Derived<T>> {
    match reflect(ty) {
        FrozenType::Parameter(
            param: TypeParamDescriptor<T>
        ) => derive_generic(param),

        concrete => derive_concrete(concrete),
    }
}
```

`TypeParamDescriptor<T>` carries compiler identity, bounds, defaults, variance,
and ownership constraints. It does not claim to know a later substitution.

**TARGET - required rejection**

```shape
FrozenType::Unknown(...)
FrozenType::Any(...)
FrozenType::InferenceVariable(...)

@derive
fn transform(value) -> _ {
    value
}
```

Required diagnostic, emitted before the hook executes: the target cannot be
frozen because its return type contains an unresolved inference variable. The
compiler never issues `TypeRef`, `FrozenType`, or `FrozenCallable` capabilities
for a partial semantic state, so a user hook cannot encounter a late missing
field or unknown-kind failure.

## Decision 53: Aliases Versus Nominal Types

Accepted: `type A = T` is always a transparent declaration synonym. Distinct
semantic identity requires an explicit nominal declaration; the exact nominal
surface syntax remains a separate language decision.

**TARGET - proposed positive example**

```shape
type UserId = int
newtype OrderId = int

comptime let user_id: TypeRef<int> =
    type_ref<UserId>()

comptime let order_id: TypeRef<OrderId> =
    type_ref<OrderId>()
```

`UserId` is definitionally equal to `int`; `OrderId` owns a nominal declaration
identity. Alias declarations remain available through declaration reflection:

```shape
comptime let alias: AliasDescriptor<#UserId, int> =
    reflect_decl(#UserId)
```

This preserves documentation, generic parameters, and provenance without
adding an alias variant to the semantic type algebra.

**TARGET - required rejection**

```shape
match reflect(type_ref<UserId>()) {
    FrozenType::Alias(alias) => use_alias(alias),
}
```

Required diagnostic: transparent aliases do not introduce semantic type
identity; use declaration reflection for alias metadata or a nominal declaration
for distinct identity. Metadata overrides cannot silently turn the same alias
syntax nominal. Expansion hashes use the normalized type identity and
separately hash declaration metadata only when a transform targets the alias
declaration itself.

## Decision 54: Uniform Type Constructors

Accepted: every nominal type uses one canonical constructor/application model,
including zero-argument nominals, user generics, explicit nominal wrappers,
language-provided types, collections, and const-generic applications.

**TARGET - proposed positive example**

```shape
comptime let option:
    TypeConstructorRef<Option, [TypeParam]> =
        type_constructor<Option>()

comptime let option_user: TypeRef<Option<User>> =
    option.apply(type_ref<User>())

comptime match reflect(option_user) {
    FrozenType::Nominal(nominal) => {
        comptime match nominal.refine(option) {
            Some(
                some<T> applied:
                    AppliedType<Option<T>, Option, [type T]>
            ) => {
                comptime let inner: TypeRef<T> =
                    applied.type_argument<0>()
                derive_option(inner)
            }
            None => derive_other_nominal(nominal),
        }
    }
    other => derive_structural(other),
}
```

Type and const argument kinds are part of the constructor signature:

```shape
comptime let page: TypeRef<Page<User, 32>> =
    type_constructor<Page>().apply(
        type_ref<User>(),
        const_arg(32),
    )
```

**TARGET - required rejection**

```shape
FrozenType::Option(...)
FrozenType::Result(...)
FrozenType::HashMap(...)

if nominal.constructor.name == "Option" {
    let inner = nominal.arguments[0]
}
```

Required diagnostics: nominal constructors are opaque typed identities, and
their ordered argument packs enforce arity plus type-versus-const kinds.
Language-provided runtime layouts do not create additional semantic reflection
variants. Canonical constructor identity and typed arguments enter expansion
and execution-ABI hashes shared by VM and JIT.
