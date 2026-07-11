# Resources And Fragments

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Expansion And Tooling](expansion-and-tooling.md)

## Decision 70: Domain-General Excellence Bar

Accepted: SQL/schema generation is one stress test, not a Shape language
feature. The comptime architecture must support broad library-defined staged
programming without compiler branches for showcase domains.

Representative required families include:

- Serialization, validation, parsers, formatters, and derives.
- CLI, routing, dependency injection, RPC, protocols, and API clients.
- FFI/binding generation, ABI validation, and embedded register maps.
- Database, GraphQL, OpenAPI, protobuf, and other external schemas.
- Units, state machines, ECS, UI/templates, and domain-specific static models.
- Shader, GPU, SIMD, tensor, and hardware specialization.
- Configuration, build-time resources, feature selection, and partial evaluation.
- Generated tests, property matrices, fixtures, diagnostics, and documentation.
- Security/permission manifests, observability instrumentation, and distributed
  artifact preparation.

The excellence bar is:

1. Semantic compiler identities and type/effect/ownership information rather
   than token streams or source text.
2. General hygienic creation and editing of declarations, signatures, bodies,
   modules, hooks, metadata, and artifacts.
3. Explicit deterministic compile-time capabilities, dependency hashing, and
   incremental invalidation.
4. Complete re-entry through ordinary language checking.
5. Context-sensitive completion, hover, diagnostics, navigation, rename,
   source maps, and expansion inspection for every generated construct.
6. Concrete positive and negative VM/JIT/compiler/LSP proofs across unrelated
   domains before the feature is called complete.

## Decision 71: Explicit Comptime Effects

Accepted: comptime is pure by default. Every interaction with external state or
host tooling requires a typed comptime capability and becomes a tracked build
dependency.

**TARGET - cross-domain positive examples**

```shape
comptime {
    let grammar = resources.read(#parser_grammar)
    emit(parser.generate(grammar))
}

comptime {
    let headers = toolchain.run(
        #clang,
        bindgen.request(#native_headers),
    )
    emit(bindgen.generate(headers))
}

comptime {
    let shader = resources.read(#shader_source)
    emit(gpu.specialize<TargetGpu>(shader))
}
```

Required semantics:

1. Pure evaluation requires no authority.
2. Files/package resources, selected environment configuration, subprocesses,
   network/provider queries, target facts, clock, randomness, and secrets each
   require explicit stage-specific capabilities and effects.
3. Every provider, normalized request, input, transitive file, tool version,
   target configuration, and content digest enters the dependency graph and
   expansion hash.
4. Providers are ordinary privileged libraries over generic comptime host
   capabilities; their domains are not compiler features.
5. Process execution is sandboxed, cancellable, quota-bound, explicitly
   granted, and fully fingerprinted.
6. Live external data needs a content-addressed snapshot/lock representation
   before it can contribute to a reproducible release build.
7. Compiler and LSP share cached provider/resource queries. The LSP shows grant,
   freshness, stale-input, refresh, quota, and failure state and does not
   silently start expensive or networked work.
8. Runtime effects and capabilities do not authorize compile-time access.

Secrets are opaque grants, never values. They cannot enter generated code,
virtual documents, diagnostics, logs, dependency hashes, or artifacts.

**TARGET - required rejection**

```shape
comptime {
    let token = env("DATABASE_TOKEN")
    let bytes = fs.read("/arbitrary/path")
    let nonce = random()
}
```

Required diagnostics identify the missing typed capability and provide a
context-sensitive configuration action where policy permits. There is no
ambient filesystem, environment, process, network, clock, or entropy surface.

## Decision 72: Typed Artifact Sinks

Accepted: comptime may produce typed code and non-code artifacts through
category-specific logical sinks. It cannot write arbitrary filesystem paths.

**TARGET - GPU example**

```shape
comptime {
    let shader = gpu.compile<TargetGpu>(#vertex_shader)

    emit artifacts {
        code(shader.generated_bindings())
        binary(#vertex_spirv, shader.binary)
        metadata(#vertex_reflection, shader.reflection)
    }
}
```

**TARGET - FFI example**

```shape
comptime {
    let bindings = bindgen.generate(#native_headers)

    emit artifacts {
        code(bindings.module)
        link(bindings.link_requirements)
        metadata(#abi_manifest, bindings.abi)
        tests(bindings.layout_tests)
    }
}
```

Artifact categories include checked code/module deltas,
`TextArtifact<Format>`, `BinaryArtifact<Format, Target>`,
`MetadataArtifact<Schema>`, `LinkRequirement<Target>`, generated test sets,
compatibility reports, verified program manifests, and atomic heterogeneous
artifact sets.

Required guarantees:

1. Logical artifact identities replace output paths; the build host chooses
   physical placement.
2. Format, target, ABI, schema, provenance, dependency hashes, source maps, and
   permissions are typed and validated.
3. A heterogeneous output set commits atomically or publishes nothing.
4. Generated tests enter the normal test/symbol graph with LSP navigation.
5. Link requirements are target-specific and permission-checked.
6. Host and target toolchains/configurations cannot be substituted silently.
7. Compiler/LSP exposes an artifact tree, previews supported formats, tracks
   stale inputs, and links diagnostics to generators and external documents.

**TARGET - required rejection**

```shape
comptime {
    fs.write("out/shader.spv", bytes)
    fs.write("out/bindings.shape", source_text)
}
```

Required diagnostic: comptime has no arbitrary output filesystem; emit a typed
logical artifact through the sink available in this build context.

## Decision 73: Native Contextual Fragments

Accepted: public quotation and splice syntax is removed. Checked fragments use
ordinary Shape grammar inside an exact typed expansion sink. A standalone
fragment uses an explicit category block only when no surrounding sink fixes
whether the fragment is an expression, pattern, statement, body, item, or
module.

**TARGET - declaration sink supplies the item context**

```shape
extend target {
    fn summary(self: User) -> string {
        self.name
    }
}
```

**TARGET - a computed member identity with a native method body**

```shape
for some<F, T> field in record.fields {
    target.methods.add(names.getter(field)) {
        fn (self: Owner) -> T {
            field.read(self)
        }
    }
}
```

`names.getter(field)` returns a typed deterministic name policy result. The
field descriptor is inserted directly where an expression of type `T` is
expected; no textual field name or interpolation sigil is involved.

**TARGET - semantic body editing**

```shape
let edit = target.body.edit()

for site in target.body.return_sites() {
    edit.replace(site.value()) {
        ensure_valid(site.value())
    }
}

edit.finish()
```

Edits address semantic cursors owned by the original body. `finish` applies the
finite edit set atomically and reruns whole-body flow, effect, ownership, borrow,
and return checking.

**TARGET - standalone expression fragment**

```shape
comptime fn add_one(value: CheckedExpr<int>) -> CheckedExpr<int> {
    expr { value + 1 }
}
```

Required rules:

1. Static generated structure uses the ordinary Shape parser and checker.
2. The surrounding sink supplies the fragment category whenever possible.
   Standalone ambiguous values use `expr {}`, `pattern {}`, `stmt {}`,
   `body {}`, `item {}`, or `module {}`.
3. A compatible checked fragment or compiler descriptor is inserted directly
   in a position whose expected type and role accept it. There is no `$`
   interpolation syntax and no implicit textual conversion.
4. Ordinary comptime data enters generated code only through an explicit
   `ConstLift` operation. Runtime values cannot cross the stage boundary.
5. Dynamic names, member paths, and declarations use typed name policies,
   descriptors, binders, and builders. Raw strings never become symbols.
6. Typed builders remain available for genuinely computed variable structure;
   semantic cursors are the editing surface for existing checked code.
7. LSP completion, hover, navigation, rename, and diagnostics are the ordinary
   Shape services plus the sink's exact expected fragment type and stage.

**TARGET - required rejection**

```shape
let generated = quote item { fn answer() -> int { 42 } }
let inserted = $generated
let parsed = CheckedBody.parse(source)
```

Required diagnostics explain the available contextual sink or exact role
block. Shape exposes no quotation sublanguage, splice sigil, token tree, source
parser, or unchecked AST representation as a public comptime code value.

## Decision 95: Complete Generated Capture Environments

Accepted: checked generated bodies and templates never capture lexically by
accident. Their types carry the complete runtime capture environment:

```shape
CheckedBody<Sig, Captures>
CheckedTemplate<Sig, Captures>
CaptureDescriptor<Sig, I, T, Mode>
```

`Captures` is a compiler-checked heterogeneous pack. Each descriptor identifies
one binding in its exact owner/signature and fixes its mode as `Move`,
`SharedBorrow`, or `ExclusiveBorrow`. There is no name-based lookup, inferred
ambient environment, homogeneous capture array, or unchecked closure payload.

**TARGET - explicit generated closure captures**

```shape
comptime let job = scope.local<#job>()
comptime let config = scope.local<#config>()
comptime let metrics = scope.local<#metrics>()

comptime let captures = scope.captures(
    move job,
    &config,
    &mut metrics,
)

comptime let worker = body(captures) {
    run(job, config)
    metrics.completed += 1
}
```

The names above resolve to compiler-issued binding identities before the body
is checked. The capture list is ordinary typed staging syntax; it neither
copies source text nor reconstructs a local by spelling. The compiler derives
the capture pack type and checks every use against its declared mode.

**TARGET - explicit comptime-to-runtime lifting**

```shape
comptime let attempts: ConstValue<int> = const_value(3)
comptime let retry_count: CheckedExpr<int> = expr.literal(attempts)

comptime let wrapper = body(target.capture_set()) {
    retry(retry_count) {
        target.call()
    }
}
```

`expr.literal` is `ConstLift`. A comptime value is not an ambient runtime
capture. References, resources, capabilities, functions, provider grants,
compiler descriptors, and arbitrary runtime handles are not liftable.

**TARGET - atomic body edit**

```shape
let edit = target.body.edit()
let original = edit.capture_set()

edit.replace_body(body(original) {
    validate(target.params)
    target.original_body()
})

edit.finish()
```

Editing begins with the original body's entire capture set. Adding or removing
a capture produces a different indexed set; `finish` checks and installs the
body, environment layout, ownership/drop plan, and generated references as one
atomic change. It cannot publish a body whose environment is only partly
updated.

**TARGET - required rejections**

```shape
comptime let body = body {
    send(caller_local)       // no implicit runtime capture
    send(comptime_config)    // no implicit stage crossing
}

comptime let capture = scope.capture("caller_local")
comptime let value = expr.literal(provider_grant)
```

Required rules:

1. Runtime parameters, locals, existing captures, hook inputs, and generated
   binders enter fragments only through compiler-issued typed descriptors.
2. Generated closures declare every capture and mode explicitly. Source
   closures may retain their ordinary ergonomic inference; once reflected or
   rewritten, the compiler exposes the resulting complete typed capture set.
3. Annotation hooks receive invocation values only through their exact
   signature-indexed hook inputs. Annotation configuration crosses into
   runtime code only as a valid `ConstLift` value.
4. Installation reruns type, effect, ownership, borrow, lifetime, suspension,
   `Send`, cleanup, drop, and `AsyncDrop` checking for the body and complete
   environment. Failure is a compile-time diagnostic and commits nothing.
5. Existing-body edits preserve all captures unless an explicit typed edit
   changes the set. Capture-set changes and body changes share one rewrite
   transaction and one expansion identity.
6. Capture identities, modes, types, lifted-constant hashes, environment layout,
   and provenance enter checked-structure and artifact hashes. Source spelling
   does not.
7. Compiler and LSP use the same binding graph. Hover shows capture type/mode
   and owner; references and rename follow identity; diagnostics identify both
   the generated use and originating binding.
