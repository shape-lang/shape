# Scoping and Name Resolution Refactor Plan

> Status: target design
> Scope: language surface, stdlib ownership, compiler resolution, module system, LSP, and docs
> Compatibility: none

## Locked Design

Shape should converge on this scope split:

1. `module scope`
   - top-level declarations
   - explicit named imports
   - namespace imports
   - builtin surface API owned by stdlib modules
2. `local scope`
   - parameters
   - `let` / `const`
   - pattern bindings
   - lambda bindings
3. `type and associated scope`
   - enum variants
   - associated constructors
   - associated items
   - methods
4. `syntax-reserved names`
   - keywords
   - literals
   - primitive type spellings like `int`, `number`, `string`, `bool`
5. `implicit prelude`
   - tiny implicit import set
   - target size: `print` only
6. `internal intrinsics`
   - `__intrinsic_*`
   - `__native_*`
   - `__json_*`
   - not callable from ordinary user code

The governing rule is lexical scope:

- all user-defined names are lexically scoped
- the outermost lexical scope is the module
- there is no user-defined global namespace

Additional locked decisions:

- builtin surface API is module-owned, not a separate magic-global class
- annotations are first-class module exports/imports and must be imported explicitly with `@`
- namespace calls use `ns::func(...)`
- namespace annotation references use `@ns::ann(...)`
- namespace imports do not leak bare names
- associated constructors belong to type scope
- preferred constructor surface is `Result::Ok`, `Result::Err`, `Option::Some`, `Option::None`
- no compatibility shims once the refactor lands

## Current Mismatches

The current implementation still diverges from the target in several ways:

- named import grammar only accepts bare identifiers
- annotations are not first-class exports in the AST/export model
- imported module AST is still inlined into callers, which leaks names
- compiler builtin fallback still exposes many user-facing names as globals
- the implicit prelude is much larger than the target design
- `Some` and `None` still have syntax-level special handling
- `Ok` and `Err` still behave like freestanding builtin constructors rather than associated constructors
- docs already describe some of the target model before the runtime fully enforces it

## End State

At the end of the refactor:

- every top-level user-defined name is module-scoped
- every builtin surface name is owned by a module
- only `print` is implicitly imported
- user code cannot call internal intrinsics directly
- imported types bring their associated namespace with them
- `Result::Ok` / `Result::Err` / `Option::Some` / `Option::None` resolve through type scope
- bare constructor globals like `Ok`, `Err`, `Some`, `None` do not exist as ordinary global bindings
- namespace imports require `ns::...`
- annotation imports require `@`

## Workstreams

The work is easiest to execute in six coordinated tracks:

1. parser and AST
2. module export/import model
3. compiler and runtime name resolution
4. stdlib ownership and prelude cleanup
5. associated-scope normalization
6. LSP, diagnostics, tests, and docs

## Phase 1: Encode the Scope Taxonomy in the Compiler

### Goal

Make the compiler speak in terms of real scope classes instead of ad hoc
fallbacks.

### Tasks

- introduce an internal resolution taxonomy:
  - `Local`
  - `ModuleBinding`
  - `NamedImport`
  - `NamespaceImport`
  - `TypeAssociated`
  - `Prelude`
  - `SyntaxReserved`
  - `InternalIntrinsic`
- audit existing identifier resolution paths and document which category each path currently uses
- separate "surface API name" from "internal intrinsic name" in helper tables
- make diagnostics report which scope category failed to resolve

### Acceptance

- name resolution code paths stop conflating module names, builtin globals, and internal helpers
- missing-name diagnostics can distinguish "missing import" from "not in associated scope" from "internal-only intrinsic"

### Regression Tests

- missing named import suggests `from module use { name }`
- missing namespace member suggests `use module as ns` plus `ns::name(...)`
- direct use of `__intrinsic_*` in user code is rejected with an internal-only diagnostic

## Phase 2: Make Imports and Exports First-Class

### Goal

Replace AST inlining semantics with a real export/import binding model.

### Tasks

- extend import grammar to support:
  - `from module use { name }`
  - `from module use { name as alias }`
  - `from module use { @ann }`
  - mixed named imports with regular names and annotations
- extend export model to support:
  - exported annotations
  - exported builtin surface declarations
- remove named-import validation against raw AST scanning
- validate imports against export tables only
- change namespace imports so they bind exactly one namespace symbol and nothing else
- stop letting imported module AST definitions become caller-local names

### Acceptance

- `use some_module` never binds bare functions, types, or annotations
- `from some_module use { ... }` binds exactly and only the requested names
- annotations participate in exports/imports the same way as other public API, with explicit `@`

### Regression Tests

- `from std::core::remote use { @remote }` parses and resolves
- `from std::core::remote use { execute, @remote }` parses and resolves
- private annotations fail to import
- namespace import followed by bare `@remote` fails
- namespace import followed by bare `execute()` fails

## Phase 3: Remove User-Facing Global Resolution

### Goal

Delete the compiler fallback that treats many builtin names as globals.

### Tasks

- shrink builtin helper tables to:
  - internal-only intrinsics
  - the tiny prelude allowlist
- remove global fallback for user-facing names such as:
  - `format`
  - `snapshot`
  - `HashMap`
  - `DateTime`
  - `Option`
  - `Result`
  - `Ok`
  - `Err`
- make surface builtins resolve only through:
  - explicit named import
  - namespace access
  - tiny implicit prelude
- remove dot-based module namespace call support
- keep namespace call support only through `ns::func(...)`

### Acceptance

- bare `format()` fails without import
- bare `snapshot()` fails without import
- bare `DateTime` type references fail without import
- only `print()` remains available without explicit import

### Regression Tests

- `print("ok")` works with no imports
- `format(x, "%Y")` fails without import
- `snapshot()` fails without import
- `use std::core::intrinsics as core` then `core::format(...)` works
- `use std::core::remote as remote` then `remote.execute(...)` fails
- `use std::core::remote as remote` then `remote::execute(...)` works

## Phase 4: Normalize Type and Associated Scope

### Goal

Move constructors and variants into type-associated scope instead of letting
them behave like global-ish names.

### Tasks

- treat enum variants uniformly as associated names under their parent type
- normalize `Option` and `Result` constructors to associated scope:
  - `Option::Some`
  - `Option::None`
  - `Result::Ok`
  - `Result::Err`
- remove expression grammar special cases that make `Some(...)` and `None` look like standalone syntax
- remove compiler builtin-constructor special cases that let `Ok(...)` and `Err(...)` behave like ordinary globals
- update pattern grammar and pattern resolution to use associated constructors
- ensure importing the type is sufficient to use its associated namespace

### Acceptance

- constructors no longer exist as freestanding global bindings
- associated constructors resolve because the type is in scope, not because a separate constructor name was imported
- match patterns work in associated form

### Regression Tests

- `from std::core::intrinsics use { Result }` then `Result::Ok(1)` works
- bare `Ok(1)` fails
- `from std::core::intrinsics use { Option }` then `Option::None` works
- bare `None` fails as a freestanding constructor name
- `match value { Result::Ok(v) => v, Result::Err(e) => 0 }` works
- `match value { Ok(v) => v, Err(e) => 0 }` fails

## Phase 5: Shrink the Prelude and Re-Own the Stdlib Surface

### Goal

Make the stdlib the actual owner of surface API and make the prelude tiny.

### Tasks

- reduce `std::core::prelude` to `print` only
- mark public builtin declarations as explicit module exports
- ensure modules such as `std::core::intrinsics`, `std::core::snapshot`, and `std::core::remote` are the canonical owners of their surface names
- keep internal intrinsics callable only from stdlib/compiler-managed paths
- update stdlib wrappers so public docs point users at module-owned names, not compiler fallbacks

### Acceptance

- prelude contains only `print`
- stdlib docs use explicit imports or namespaces for every non-prelude surface symbol
- user code cannot access internal intrinsics even if it guesses their names

### Regression Tests

- explicit imports for `Result`, `Option`, `DateTime`, `Snapshot`, `HashMap`, `format`, and annotation modules all work
- prelude no longer injects traits, snapshot helpers, math functions, or types
- direct calls to `__intrinsic_*` fail in user code
- stdlib wrappers using internal intrinsics still work

## Phase 6: Annotations, LSP, Diagnostics, and Docs

### Goal

Bring the editor and documentation model into line with the language model.

### Tasks

- add first-class annotation import specs in AST and parser
- add annotation namespace references `@ns::ann`
- update annotation discovery to read exported annotations instead of blindly loading all annotation definitions from imported modules
- update completion, hover, definition, semantic tokens, and code actions for:
  - `from module use { @ann }`
  - `@ns::ann`
  - `ns::func(...)`
  - associated constructors and variants through type scope
- update the book and examples to consistently describe:
  - lexical scope
  - module-owned builtin API
  - tiny prelude
  - associated constructors

### Acceptance

- LSP suggestions match the target surface
- diagnostics suggest imports instead of relying on global fallback assumptions
- docs no longer imply that user-facing builtins are globals

### Regression Tests

- completion inside named import lists includes `@ann`
- hover/definition on `@ns::ann` work
- hover/definition on `Result::Ok` and `Option::Some` work
- missing symbol diagnostics recommend the owning module or owning type

## Phase 7: Final Cleanup and Enforcement

### Goal

Delete every compatibility branch and enforce the model consistently.

### Tasks

- remove dead parser rules and dead compiler fallback branches
- remove outdated tests that still encode global behavior
- unignore the clean-break scoped import contract tests and expand them
- add a final audit pass for stray global resolution paths

### Acceptance

- no user-facing global resolution remains except the tiny implicit prelude
- all clean-break import tests pass
- docs and code agree on the same scope model

### Regression Tests

- activate the full scoped-contract suite
- add end-to-end integration tests covering:
  - named imports
  - namespace imports
  - annotation imports
  - type-associated constructors
  - tiny prelude
  - internal intrinsic rejection

## Execution Order

Recommended order:

1. Phase 1: scope taxonomy
2. Phase 2: import/export model
3. Phase 3: remove user-facing globals
4. Phase 4: normalize associated constructors
5. Phase 5: shrink prelude and re-own stdlib surface
6. Phase 6: LSP, diagnostics, and docs
7. Phase 7: cleanup and enforcement

Phase 4 is intentionally after Phase 3. Once module and global resolution are
cleaned up, constructor normalization becomes much easier to reason about.

## Non-Goals

These are explicitly out of scope for this refactor:

- qualified type paths beyond ordinary associated scope if they require a larger type-system redesign
- backward compatibility shims
- dual syntax periods
- keeping old dot namespace access alive
- exposing internal intrinsics as public API

## Final Acceptance Criteria

The refactor is complete when all of the following are true:

- all user-defined names are lexically scoped
- top-level names live in modules
- builtin surface API is module-owned
- only `print` is implicitly imported
- annotations are explicit module imports with `@`
- namespace access uses `::`
- associated constructors live in type scope
- internal intrinsics are unavailable to ordinary user code
- the book, compiler, stdlib, tests, and LSP all describe and enforce the same model
