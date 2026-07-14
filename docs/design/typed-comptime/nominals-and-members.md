# Nominals And Members

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Values, Types, And Evidence](values-types-and-evidence.md) | [Next: Annotations And Hooks](annotations-and-hooks.md)

> **Implementation status (ADR009-B5, Dec 55-58): CURRENT / VM+JIT+LSP.**
> The sealed `NominalShape` (`Struct`/`Enum`/`Newtype`/`Opaque`),
> `FieldDescriptor<Owner, #f, T>` / `VariantDescriptor` /
> `AssociatedConstDescriptor` / `FieldInitialization`, generic substitution
> before descriptor issuance (Dec 55 R10), derive-style field/variant
> iteration, and the authority-gated `reflect_repr` + `RepresentationAccess<T>`
> (Dec 56) are implemented and proven on VM, JIT, and the LSP (hover renders
> the descriptor types + owner; the reflection vocabularies complete through
> the one shared `reflection_enum_variant_names` catalog). The hard member-
> selection rejection matrix R1-R11 fires named diagnostics. See the design-
> index rows in [../typed-comptime.md](../typed-comptime.md) for the per-
> structure CURRENT labels + evidence. Two forms remain CURRENT-vs-TARGET,
> logged in [../../defections.md](../../defections.md): `#name` explicit member
> selection (grammar-blocked B7 — the descriptor TYPE carries the hygienic
> member position `#f`, but no `record.field(#name)` call spelling exists), and
> the public-vs-complete field-set distinction (field-visibility syntax pending
> — S3 delivers the authority gate, not a filtered partial view).

## Decision 55: Nominal Declaration Shapes

Accepted: an applied nominal type has one of four sealed semantic declaration
shapes: `Struct`, `Enum`, `Newtype`, or `Opaque`.

**TARGET - proposed positive example**

```shape
comptime fn derive_nominal<T>(
    nominal: FrozenNominal<T>,
    ctx: ComptimeContext,
) -> CheckedItem<Derived<T>> {
    match nominal.shape() {
        NominalShape::Struct(
            record: StructDescriptor<T>
        ) => derive_struct(record),

        NominalShape::Enum(
            sum: EnumDescriptor<T>
        ) => derive_enum(sum),

        NominalShape::Newtype(
            some<U> wrapper: NewtypeDescriptor<T, U>
        ) => derive_newtype(wrapper),

        NominalShape::Opaque(
            opaque: OpaqueTypeDescriptor<T>
        ) => ctx.error(
            opaque,
            "This derive requires visible representation",
        ),
    }
}
```

Generic substitution precedes descriptor issuance. Reflecting `Page<User>`
therefore produces field descriptors containing `TypeRef<Array<User>>`, never
an unresolved parameter or rendered type spelling.

**TARGET - required rejection**

```shape
if nominal.kind == "struct" {
    for field in nominal.fields ?? [] {
        generate_field(field)
    }
}

if nominal.is_builtin {
    inspect_native_layout(nominal)
}
```

Required diagnostics: nominal shape selection is exhaustive and typed, and
runtime representation classes are not semantic reflection categories. A new
nominal shape changes the comptime ABI. Canonical shape descriptors enter the
expansion hash, while `NativeKind`, heap layout, and other backend details do
not.

## Decision 56: Representation Authority

Accepted: complete nominal-shape reflection requires a compiler-issued
`RepresentationAccess<T>`. Ordinary reflection never returns a filtered or
context-dependent partial representation.

**TARGET - proposed positive example**

```shape
annotation derive_json() {
    targets: [type]

    comptime expand<T>(
        target: TypeTarget<T>,
        access: RepresentationAccess<T>,
        ctx: ComptimeContext,
    ) {
        comptime let shape: NominalShape<T> =
            reflect_repr(target.type_ref, access)

        match shape {
            NominalShape::Struct(record) =>
                derive_struct(record, access),
            NominalShape::Enum(sum) =>
                derive_enum(sum, access),
            NominalShape::Newtype(wrapper) =>
                derive_newtype(wrapper, access),
            NominalShape::Opaque(opaque) =>
                ctx.error(
                    opaque,
                    "Representation is not decomposable",
                ),
        }
    }
}

@derive_json
type User {
    public name: string,
    private token: string,
}
```

Applying the annotation at the declaration is explicit author consent and
provides complete representation access to that transform.

**TARGET - required rejection**

```shape
comptime let user = reflect(type_ref<User>())
comptime let shape = reflect_repr(
    type_ref<User>(),
    /* no RepresentationAccess<User> */
)
```

Required diagnostics: representation reflection requires explicit authority,
and ordinary reflection cannot substitute a filtered field list or misleading
`Opaque` shape. `Opaque` means semantically non-decomposable. Private field
references in emitted fragments remain subject to installation-scope checking;
the reflection capability is not ambient runtime access authority.

## Decision 57: Field Identity

Accepted: every named field uses an opaque compiler-issued member identity,
never its source name or ordinal.

**TARGET - proposed positive example**

```shape
type User {
    name: string,
    age: int,
}

comptime let record: StructDescriptor<User> =
    reflect_repr(type_ref<User>(), access)

comptime let name:
    FieldDescriptor<User, #name, string> =
        record.field(#name)

comptime let read: CheckedExpr<string> =
    expr.field(receiver, name)

comptime for some<F, T> field in record.fields {
    comptime let value: CheckedExpr<T> =
        expr.field(receiver, field)
    output.push(generate_field(field, value))
}
```

`#name` resolves in `User`'s member scope to a hygienic identity. Source order
still defines canonical iteration but cannot select a named member. Tuples use
separate positional `TupleElement<I, T>` descriptors because position is their
semantics.

**TARGET - required rejection**

```shape
record.field("name")
record.fields[0]
expr.field(receiver, field.name)
```

Required diagnostics: named-field selection requires an owner-bound member
identity, and neither source spelling nor declaration position is such an
identity. Diagnostics render the descriptor through a sink. External names
such as JSON keys come from an explicit typed codec-label policy and never feed
back into field resolution.

## Decision 58: Comptime Fields Are Not Fields

Accepted: zero-slot "comptime fields" are removed. Configurable type-level
values use const generic parameters; computed or read-only type members use
associated constants.

**TARGET - proposed positive example**

```shape
type Currency<
    const Code: string = "USD",
    const Decimals: int = 2,
> {
    amount: number,

    const code: string = Code,
    const decimals: int = Decimals,
}

type Yen = Currency<"JPY", 0>
```

Representation and declaration reflection remain separate:

```shape
comptime let currency =
    reflect(type_ref<Currency<"USD", 2>>())

currency.shape.fields
// FieldDescriptor<Currency<"USD", 2>, #amount, number>

currency.associated_constants
// AssociatedConstDescriptor<Currency<"USD", 2>, #code, string>
// AssociatedConstDescriptor<Currency<"USD", 2>, #decimals, int>

comptime let code: ConstValue<string> =
    currency.constant(#code).value

comptime let literal: CheckedExpr<string> =
    expr.literal(code)
```

**TARGET - required rejection**

```shape
type Currency {
    comptime code: string = "USD",
    amount: number,
}

type Yen = Currency { code: "JPY" }

if field.is_comptime {
    use_constant_field(field)
}
```

Required migration: remove `StructField.is_comptime` and alias override syntax.
Runtime fields are the only members of `StructDescriptor.fields`; const
parameters are part of `TypeConstructorRef`, and associated constants are
declaration-interface members. No reflection or backend path retains a
zero-runtime-slot field special case.

## Decision 59: Total Records

Accepted: every runtime record field always exists. Semantic absence uses an
explicit `Option<T>` field type; a default affects construction policy only.

**TARGET - proposed positive example**

```shape
type Config {
    endpoint: Option<string> = None,
    retries: int = 3,
}

type Request {
    token: Option<string>,
}

let request = Request {
    token: None,
}
```

Reflection preserves exact stored types and uses a sealed initialization
policy:

```shape
FieldDescriptor<Config, #endpoint, Option<string>>
FieldInitialization::Defaulted(
    DefaultInitializer<Config, #endpoint, Option<string>>
)

FieldDescriptor<Request, #token, Option<string>>
FieldInitialization::Required
```

Reading `Request.token` always produces `CheckedExpr<Option<string>>`; there is
no hidden missing-field branch.

**TARGET - required rejection**

```shape
type Request {
    token?: string,
}

if field.optional {
    generate_optional_access(field)
}
```

Required diagnostics: optional field syntax is removed, and field descriptors
have no optional-presence bit. A decoder maps missing input either to an
explicit default/`None` under its typed codec policy or to a typed decode
failure for a required field; it never creates a partial record value.

## Decision 60: Default Initializers

Accepted: every field default is a closed checked thunk with an explicit effect
row. It cannot inspect partial `self` or sibling fields.

**TARGET - proposed positive example**

```shape
fn fresh_id() -> Id ! { State<IdPool> } {
    ...
}

type Job {
    id: Id = fresh_id(),
    retries: int = 3,
}
```

Reflection exposes the complete initializer type:

```shape
DefaultInitializer<
    Job,
    #id,
    Id,
    { State<IdPool> }
>
```

The synthesized constructor inherits all effects of defaults it may evaluate:

```shape
fn Job::new(
    id: Id = fresh_id(),
    retries: int = 3,
) -> Job ! { State<IdPool> }
```

Omitted defaults execute in declaration order. Their bodies re-enter ordinary
type, effect, ownership, borrow, failure, suspension, and cleanup checking.
Dependent initialization uses an explicit constructor.

**TARGET - required rejection**

```shape
type Range {
    start: int,
    end: int = self.start + 1,
}
```

Required diagnostic: a field default cannot observe partially initialized
`self`; use an explicit constructor for dependent initialization. Source-string
defaults and unchecked AST payloads are likewise invalid. Canonical checked
initializer bodies and effects enter expansion/artifact hashes for common VM
and JIT behavior.
