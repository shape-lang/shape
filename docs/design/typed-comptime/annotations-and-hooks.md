# Annotations And Hooks

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Nominals And Members](nominals-and-members.md) | [Next: Expansion And Tooling](expansion-and-tooling.md)

> **Architecture correction (2026-07-25).** ADR-011 and ADR-012 are binding
> over dated C3/E4/E6 implementation decisions. `on` may remain concise source
> sugar, but annotation support comes only from resolved typed clauses.
> Spelling-recognized decisions, pseudo-tuples, universal targets, marker
> substitution, and string-backed applied metadata are migration paths to
> delete. Contract clauses contribute effects, outcomes, ownership, and
> lifecycle requirements before the effective target contract and its callers
> are checked; plan/body elaboration follows that freeze.

## Decision 61: Typed Applied Annotations

Accepted: every annotation application is a compiler-issued typed descriptor
containing exact annotation identity, typed comptime arguments, permitted-target
proof, and declared multiplicity.

**TARGET - proposed positive example**

```shape
annotation json_name(label: CodecLabel<Json>) {
    multiplicity: once

    comptime post<Owner, F, T>(
        target: FieldTarget<Owner, F, T>,
        ctx: ComptimeContext,
    ) -> FieldTransform<Owner, F, T> {
        FieldTransform::NoChange
    }
}

type User {
    @json_name(json.label("user_name"))
    name: string,
}

comptime let json_name_annotation =
    annotation_ref<json_name>()

comptime match field.annotation(json_name_annotation) {
    Some(
        applied: AppliedAnnotation<
            json_name,
            FieldTarget<User, #name, string>,
            JsonNameArgs,
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

`JsonNameArgs` is the generated nominal ConstLift argument record for this
annotation; its `label` field is selected by hygienic parameter identity.
The exact field clause is an explicit typed metadata/no-op contribution, so
field support follows from the same clause set as transforming annotations.
`"user_name"` is JSON-domain data inside a typed codec label; it never
identifies a Shape field or annotation. Repeatable annotations return a typed
collection rather than `Option`. Source applications compose first-written
outermost. Generated applications insert at an explicit outer/inner or
before/after exact-application position; ambiguous ties and cycles reject.
Every occurrence has a stable application identity, and the resulting total
order enters expansion hashes and shared compiler/LSP facts.

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
enter canonical expansion hashes. Runtime hook state belongs to the lexical
Callable Transform and `CheckedAnnotationPlan`, not the applied metadata
descriptor.

## Decision 62: Total Annotation Target Contracts

Accepted: an annotation's supported targets and phases are derived solely from
its typed handler clauses. A separate `targets: [...]` registry is removed.

**TARGET - proposed positive example**

```shape
annotation audit() {
    comptime post<T>(
        target: NominalTarget<T>,
        access: RepresentationAccess<T>,
        ctx: ComptimeContext,
    ) -> NominalTransform<T> {
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
        CallableTransform::around(
            hook fn(
                args: ArgumentPack<Sig>,
                next: Next<Sig>,
            ) -> ReturnOf<Sig> ! { Remote<P>, Suspend } {
                let portable: PortableContinuationArtifact<Sig> =
                    next.into_portable()
                let admitted: AdmittedExecution<Sig, P> =
                    remote::admit(
                        placement,
                        options,
                        portable,
                    )
                remote::dispatch_transparent(
                    admitted,
                    args,
                )
            },
        )
    }
}
```

The `around` callable is ordinary typed Shape code. `next.into_portable()`
consumes the affine continuation but grants no execution authority;
`remote::admit` separately validates the artifact at the placement and returns
single-attempt admitted authority. Neither the annotation name nor a marker call
has compiler privilege. The example abbreviates the `CheckedTemplate` capture
clause: `placement` and `options` are explicit typed ConstLift captures, never
ambient runtime bindings. `dispatch_transparent` preserves OutcomeUnknown as
suspended recovery or a receipted obligation transfer; it never drops the
obligation into an ordinary error.

When declarative `before` and `after` clauses are used, elaboration creates an
ordinary typed success join. Both a same-layer exact short-circuit and a
completion from `next` pass through that layer's `after` clause once. An
explicit early return in a raw `around` function retains ordinary control-flow
semantics and does not run skipped source code.

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
    comptime post<Sig>(
        target: CallableTarget<Sig>,
        ctx: ComptimeContext,
    ) -> CallableTransform<Sig> {
        CallableTransform::around(
            hook fn(
                args: ArgumentPack<Sig>,
                next: Next<Sig>,
            ) -> ReturnOf<Sig> {
                comptime for some<I, T, Mode> param
                    in target.parameters
                {
                    emit {
                        trace_value(args.get(param))
                    }
                }

                next.call(args)
            }
        )
    }
}
```

The nested block runs once while the hook is specialized. It emits one typed
runtime trace statement per exact parameter. `args.get(param)` is typed by the
signature-indexed descriptor; it is not a pseudo-tuple or homogeneous
collection. No comptime evaluator runs when the annotated function is called.

**TARGET - combined remote annotation**

```shape
annotation remote<P>(placement: Placement<P>) {
    comptime post<Sig>(
        target: CallableTarget<Sig>,
        ctx: ComptimeContext,
    ) -> CallableTransform<Sig> {
        require_transferable(target)

        CallableTransform::around(
            hook fn(
                args: ArgumentPack<Sig>,
                next: Next<Sig>,
            ) -> ReturnOf<Sig> ! { Remote<P>, Suspend } {
                let portable: PortableContinuationArtifact<Sig> =
                    next.into_portable()
                let admitted: AdmittedExecution<Sig, P> =
                    remote::admit(
                        placement,
                        RemoteCallOptions::default(),
                        portable,
                    )
                remote::dispatch_transparent(
                    admitted,
                    args,
                )
            },
        )
    }
}
```

The validation, signature expansion, and transform construction are
compile-time operations. The emitted Core/MIR has ordinary exact parameters.
Remote dispatch is ordinary typed runtime behavior. `placement` is an explicit
ConstLift capture in the checked template, abbreviated in the example.

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
