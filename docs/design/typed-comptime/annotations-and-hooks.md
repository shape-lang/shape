# Annotations And Hooks

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Nominals And Members](nominals-and-members.md) | [Next: Expansion And Tooling](expansion-and-tooling.md)

## Decision 61: Typed Applied Annotations

Accepted: every annotation application is a compiler-issued typed descriptor
containing exact annotation identity, typed comptime arguments, permitted-target
proof, and declared multiplicity.

**TARGET - proposed positive example**

```shape
annotation json_name(label: CodecLabel<Json>) {
    targets: [field]
    multiplicity: once
}

type User {
    @json_name(json.label("user_name"))
    name: string,
}

comptime let json_name_annotation =
    annotation_ref<json_name>()

comptime match field.annotation(json_name_annotation) {
    Some(
        applied: AnnotationDescriptor<
            json_name,
            FieldTarget<User, #name, string>,
            [CodecLabel<Json>],
            Once
        >
    ) => {
        comptime let label: ConstValue<CodecLabel<Json>> =
            applied.argument(#label)
        generate_json_field(field, label)
    }
    None => {
        generate_json_field(
            field,
            default_json_label(field),
        )
    }
}
```

`"user_name"` is JSON-domain data inside a typed codec label; it never
identifies a Shape field or annotation. Repeatable annotations return a typed
collection rather than `Option`.

**TARGET - required rejection**

```shape
for annotation in field.annotations {
    if annotation.name == "json_name" {
        let label = annotation.args[0] as string
    }
}
```

Required diagnostics: annotation identity and argument selection cannot use
text or `Any`; wrong targets and duplicate `once` applications fail before any
hook executes. The identity, target proof, typed arguments, and multiplicity
enter canonical expansion hashes. Runtime hook state belongs to `HookPlan`, not
the applied metadata descriptor.

## Decision 62: Total Annotation Target Contracts

Accepted: an annotation's supported targets and phases are derived solely from
its typed handler clauses. A separate `targets: [...]` registry is removed.

**TARGET - proposed positive example**

```shape
annotation audit() {
    comptime post<T>(
        target: TypeTarget<T>,
        access: RepresentationAccess<T>,
        ctx: ComptimeContext,
    ) -> TypeTransform<T> {
        derive_audit_type(target, access)
    }

    comptime post<Owner, F, T>(
        target: FieldTarget<Owner, F, T>,
        ctx: ComptimeContext,
    ) -> FieldTransform<Owner, F, T> {
        FieldTransform::NoChange
    }
}
```

The clause set is the complete contract. An intentional no-op is explicit.
`@remote` uses the same ordinary annotation protocol:

```shape
annotation remote<P>(
    placement: Placement<P>,
    options: RemoteCallOptions = {},
) {
    comptime post<Sig>(
        target: CallableTarget<Sig>,
        ctx: ComptimeContext,
    ) -> CallableTransform<Sig> {
        build_remote_hook_plan(target, placement, options)
    }
}
```

**TARGET - required rejection**

```shape
annotation audit() {
    targets: [type, field]

    comptime post(target: ComptimeTarget) {
        if target.kind == "type" {
            derive_audit_type(target)
        }
    }
}
```

Required diagnostics: there is no universal optional-field target descriptor,
implicit handler, or separate support registry. Applying an annotation where no
typed clause exists fails compilation. The exact target/phase clause set enters
the annotation ABI and expansion hash.

## Decision 63: Parameter Identity

Accepted: callable parameters use signature-indexed positional identities
because order is part of positional calling and the callable ABI. Source names
resolve hygienically to those identities and are not semantic lookup text.

**TARGET - proposed positive example**

```shape
fn update(user: User, active: bool) -> User

comptime let callable: FrozenCallable<
    fn(User, bool) -> User
> = target.callable

comptime let user:
    ParamDescriptor<
        callable.signature,
        0,
        User,
        Owned
    > = callable.param<0>()

comptime let same_user = callable.param(#user)

comptime for some<I, T, Mode> parameter
    in callable.parameters
{
    inspect_parameter(parameter)
}

comptime let plan = RewritePlan::replace(
    user,
    validated_user_expr,
)
```

`#user` resolves to the same position identity. Reordering parameters changes
the callable signature and invalidates dependent expansions.

**TARGET - required rejection**

```shape
callable.param("user")
callable.parameters as Array<Any>
rewrite["user"] = expression
```

Required diagnostics: parameter selection and rewrites require a
signature-indexed descriptor. Named arguments compile to the same identity;
there is no runtime parameter lookup or homogeneous argument collection.

## Decision 64: Exact Target Descriptors

Accepted: annotation clauses accept concrete target descriptor types. They may
be grouped conceptually, but are never erased into an `AnnotationTarget` value.

The target catalog is:

```text
Declaration targets
- ModuleTarget<M>
- NominalTarget<T>
- TraitTarget<Tr>
- AliasTarget<A, T>
- CallableTarget<Sig, Origin>
- FieldTarget<Owner, F, T>
- VariantTarget<Owner, V, Payload>
- ParamTarget<Sig, I, T, Mode>
- ReturnTarget<Sig, R>
- AssociatedConstTarget<Owner, C, T>

Expression targets
- ValueExprTarget<T, Effects, Ownership>
- BlockExprTarget<T, Effects, Flow>
- AwaitExprTarget<Future<T>, T, Effects>
```

**TARGET - proposed positive examples**

```shape
comptime post<T>(
    target: NominalTarget<T>,
    access: RepresentationAccess<T>,
    ctx: ComptimeContext,
) -> NominalTransform<T> {
    derive_json(target, access)
}

comptime post<T>(
    target: AwaitExprTarget<Future<T>, T, { Suspend }>,
    ctx: ComptimeContext,
) -> ExprTransform<T> {
    wrap_timeout(target)
}

comptime post<Owner, F, T>(
    target: FieldTarget<Owner, F, T>,
    ctx: ComptimeContext,
) -> FieldTransform<Owner, F, T> {
    apply_redaction(target)
}
```

Current `function` migrates to `CallableTarget`; `type` splits into nominal,
trait, and alias targets; `expression`, `block`, and `await_expr` become exact
expression descriptors. Methods are callable origins, and closures are callable
expressions. The current empty/parser-only binding target is deleted.

**TARGET - required rejection**

```shape
comptime post(target: AnnotationTarget) {
    match target.kind {
        "type" => derive_type(target),
        "await_expr" => wrap_await(target),
    }
}
```

Required diagnostic: annotation clauses require one exact target descriptor;
there is no universal target or string kind. A future target lands only with a
complete descriptor, legal transform algebra, diagnostics, examples, and
VM/JIT proof, and changes the comptime ABI.

## Decision 65: Annotations And Comptime Are Independent

Accepted: annotations provide target attachment and hook composition. The
comptime engine performs compile-time evaluation and typed code expansion.
Either mechanism can be used alone, and annotation compilation may invoke
comptime without making runtime hook execution part of comptime.

**TARGET - standalone comptime generation**

```shape
comptime {
    let schema: DatabaseSchema<AppDb> =
        db.describe<AppDb>()

    emit(schema.checked_module())
}
```

At module scope, the block runs during declaration discovery and emits checked
types, functions, impls, and dependency hashes visible to later source.

**TARGET - comptime specialization inside a runtime hook template**

```shape
annotation traced() {
    comptime {
        let hook = ctx.hooks.before(ctx.target)

        for some<I, T, Mode> param
            in ctx.target.parameters
        {
            hook.emit {
                trace_value(hook.argument(param))
            }
        }

        hook.proceed()
    }
}
```

The nested block runs once while the hook is specialized. It emits one typed
runtime trace statement per explicit generated wrapper parameter.
`hook.argument(param)` is a comptime checked-expression builder tied to that
parameter descriptor, not a source-visible runtime argument pack. No comptime
evaluator runs when the annotated function is called.

**TARGET - combined remote annotation**

```shape
annotation remote<P>(placement: Placement<P>) {
    comptime {
        require_transferable(ctx.target)
        add_effects(ctx.target, { Remote<P>, Suspend })

        let hook = ctx.hooks.before(ctx.target)
        let call = expr.call_each(
            remote::dispatch,
            ctx.target.parameters,
            |param| hook.argument(param),
        )

        hook.return(call)
    }
}
```

The validation, signature expansion, and hook construction are compile-time
operations. The emitted wrapper has ordinary explicit parameters. Remote
dispatch is ordinary generated runtime behavior.

**TARGET - required rejection**

```shape
fn generated_runtime_hook(request: Request) {
    let result = comptime {
        remote::dispatch(request)
    }
    result
}
```

Required diagnostic: comptime cannot consume runtime arguments or perform an
invocation-time operation. In every placement, `comptime {}` receives only the
typed capabilities available at that compiler stage and returns a typed value,
checked fragment, or atomic expansion delta.
