# Expansion And Tooling

[Back to the typed comptime overview](../typed-comptime.md) | [Previous: Annotations And Hooks](annotations-and-hooks.md) | [Next: Resources And Fragments](resources-and-fragments.md)

## Decision 66: Context-Indexed Comptime And LSP

Accepted: `comptime {}` is legal in every language context that provides a
typed expansion sink. The sink determines exactly what the block may inspect
and produce.

| Context | Legal output |
|---|---|
| Expression | Closed comptime value or `CheckedExpr<T>` |
| Type/signature | `TypeRef<T>` or typed shape/signature patch |
| Body/hook | Checked expression, statement, body, or hook fragment |
| Item/module | `CheckedItem<Decl>` or atomic checked module delta |
| Annotation | Exact-target transform and hook-template contributions |

**TARGET - context-sensitive positive examples**

```shape
let mask: int = comptime {
    permission_mask<CurrentModule>()
}

comptime {
    let schema = db.describe<AppDb>()
    emit(schema.checked_module())
}

annotation traced() {
    comptime {
        let hook = ctx.hooks.before(ctx.target)
        generate_trace_statements(hook, ctx.target.parameters)
    }
}
```

**TARGET - required rejection**

```shape
let value = comptime {
    emit(checked_type_declaration())
    42
}
```

Required diagnostic: the expression expansion sink cannot emit a declaration;
move the generation to an item/module comptime context.

LSP behavior is part of this contract:

1. Completion offers only builders and descriptors legal for the current sink.
2. Hover shows comptime stage, exact generic/descriptor type, and whether a
   value is compile-time data or generated runtime code.
3. Signature help understands typed builders, annotation clauses, hygienic
   identities, existential openings, and generated APIs.
4. Diagnostics use the compiler's real stage/type/effect/ownership results and
   identify both expansion and application sites.
5. Go-to-definition, references, rename, semantic tokens, and inlay hints cover
   source and generated declarations through stable symbol identities.
6. A virtual expansion view displays generated checked code with bidirectional
   source maps; it is an inspection surface, never reparsed compiler input.
7. SQL/schema-generated types become available to completion and navigation as
   soon as declaration discovery reaches its deterministic fixed point.

The LSP must consume compiler query results rather than implement a second
comptime evaluator or infer generated symbols from text.

## Decision 67: Declaration Discovery Fixed Point

Accepted: declaration-producing comptime runs to a deterministic monotonic
fixed point before ordinary function or expression bodies are checked.

**TARGET - proposed positive example**

```shape
comptime {
    let schema = db.describe<AppDb>()
    emit(schema.checked_module())
}

fn find_admin() -> db::UserRow {
    db::find_user(1)
}
```

Generated declarations are available to name resolution, type checking, and
LSP queries regardless of source order. Generated declarations may themselves
carry annotations or comptime expansion, producing additional fixed-point
work.

Required invariants:

1. Every emitted declaration receives a compiler-issued symbol identity.
2. Each expansion application runs once for its application identity and
   complete dependency hash.
3. Existing discovered headers cannot be changed or removed during discovery.
4. Every reserved generated identity receives exactly one complete definition.
5. Duplicate identities, conflicting definitions, cycles, oscillation, and
   unbounded generation are compile errors with expansion provenance.
6. External schemas, files, environment inputs, and provider descriptors enter
   incremental compilation and LSP invalidation hashes through typed comptime
   capabilities.
7. Ordinary body checking begins only after the declaration graph stabilizes.

**TARGET - required rejection**

```text
1. Check ordinary source bodies.
2. Run comptime generation afterward.
3. Patch unresolved names if generated code happens to define them.
```

Required diagnostic: generated declarations must participate in declaration
discovery before body analysis; late unresolved-name patching is not a compiler
stage. The compiler and LSP consume the same fixed-point query rather than
running separate speculative expansion passes.

## Decision 68: Generated Symbols And Expansion Provenance

Accepted: every generated declaration and internal node is an ordinary checked
compiler symbol with stable expansion provenance. Generated text and dummy
spans are not semantic representations.

Status (2026-07-13, ADR009-D1): CURRENT on the existing extend/materialization
path — compiler-issued `SymbolId` / `ExpansionIdentity` / `GeneratedOrigin`
(`crates/shape-vm/src/compiler/comptime_builtins/expansion_provenance.rs`),
real source anchors (no `Span::DUMMY`), identity-keyed dedup with named
conflict/duplicate rejections, and LSP behaviors 1-3 and 5 below plus
identity-controlled rename (source binders rename by recomputation; wholly
generator-controlled names report that fact and link the generator
definition), all served from `BytecodeCompiler::generated_symbol_query()`.
Behavior 4 (`shape-expansion://` virtual documents) and the Decision 67
declaration-discovery fixed point remain TARGET (ticket D2). Evidence:
`docs/cluster-audits/wave46-typed-comptime-first-tracers.md` (D1 addendum).

Each generated node carries the equivalent of:

```shape
ExpansionIdentity {
    generator: GeneratorRef,
    application: ApplicationId,
    target: TargetIdentity,
    stage: ComptimeStage,
    arguments_hash: Hash,
    dependencies_hash: Hash,
}

GeneratedOrigin {
    expansion: ExpansionIdentity,
    node_path: GeneratedNodePath,
    source_anchor: SourceSpan,
}
```

Required LSP behavior:

1. Go-to-definition opens the checked generated declaration and links to the
   source application and generator definition.
2. Diagnostics show the failing generated node plus related generator,
   application, and external-dependency locations.
3. References, workspace symbols, completion, semantic tokens, hover, signature
   help, and inlay hints operate on `SymbolId`, not rendered text.
4. A read-only URI such as `shape-expansion://<hash>/db.shape` renders checked
   generated code for inspection. It is never parsed as compiler input.
5. Generated fields, parameters, references, hook statements, and metadata all
   retain structured bidirectional source mappings.

**TARGET - identity-controlled rename**

```shape
comptime {
    emit(sql.table(
        source: sql.table_name("users"),
        output: #UserRow,
    ))
}
```

Because `#UserRow` is an explicit source binder, rename edits that binder and
recomputes the expansion. When a name is wholly generator-controlled, rename
reports that fact and navigates to the generator configuration; it never edits
virtual expansion text.

**TARGET - required rejection**

```text
Create a generated AST node with name "UserRow" and Span::DUMMY,
then let the LSP rediscover it by scanning rendered source.
```

Required diagnostic/invariant failure: generated nodes require compiler symbol
identity and expansion provenance. Absence of navigation, source mapping, or
context-sensitive editor support is a language-completeness defect and blocks
the feature from being considered implemented.

## Decision 69: Ergonomic Typed Name Generation

Accepted: a public generated name originates either from an explicit hygienic
source binder or from a typed deterministic naming policy over a branded
external identifier. There is no general string-to-symbol API.

**TARGET - explicit mapping**

```shape
comptime {
    emit(
        schema.table(sql.table_name("user_accounts"))
            .as(#UserAccount)
            .column(sql.column_name("user_id")).as(#id)
    )
}
```

**TARGET - automatic policy**

```shape
comptime {
    emit(schema.generate(
        type_names: names.pascal_case<SqlIdentifier>(),
        field_names: names.snake_case<SqlIdentifier>(),
    ))
}
```

`SqlIdentifier` is domain data; the general mechanism is
`NamePolicy<Domain, Namespace>`. Policies are deterministic, versioned,
namespace-aware, and included in expansion hashes.

Required ergonomics and guarantees:

1. LSP previews generated names and exposes generated symbols immediately after
   declaration discovery.
2. Hover shows the external identifier, policy, and explicit overrides.
3. Collisions, reserved words, and invalid identifiers produce diagnostics with
   code actions that insert explicit aliases.
4. Policy-derived rename offers to pin an explicit binder before renaming.
5. No policy silently appends numeric suffixes.
6. Private temporaries may use fresh hygienic identities without public names.

**TARGET - required rejection**

```shape
let symbol = compiler.symbol(table_name_string)
```

Required diagnostic: raw text cannot become a declaration identity; use an
explicit binder or a typed name policy for the external identifier domain.
