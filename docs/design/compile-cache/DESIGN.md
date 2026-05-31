# DESIGN — `.shapec` resolved-interface compile-cache format extension

Status: **DRAFT — awaiting supervisor ratify.**
Scope: design only; no code. All `file:line` cites are against workspace HEAD
(`/home/dev/dev/shape-lang/shape`).

> **TEAM-LEAD AMENDMENTS 2026-05-31** (fold-in of the adversarial-verify pass;
> the design body below is updated in place, changes flagged inline as
> `[AMENDMENT A/B/C]`). Three results-correctness holes were found and closed:
> - **A (BREAK 1, run-verified):** the grouped-per-kind data model (§1.1) cannot
>   reproduce source-order-sensitive trait/impl/enum registration. **Verified at
>   HEAD:** `predeclare_item` (items.rs:18-36) has NO Trait/Impl/Enum arm
>   (`_ => Ok(())`); those register only in `infer_item` (items.rs:202-208) in
>   source order — so an impl textually before its trait diverges from a grouped
>   replay. Fix: §1.1 now carries a single **source-ordered `Vec<Item>`**.
> - **B (BREAK 2):** the cache key's `compiler_version` is `env!(CARGO_PKG_VERSION)`
>   (`bundle_compiler.rs:185`) — coarse semver that does NOT change across dev
>   rebuilds of the checker at the same version → stale-cache silent-wrong during
>   exactly the v0.3.3 checker churn. Fix: §2.2 now uses a build fingerprint.
> - **C (BREAK 3):** `source_hash` covers only the package's own `.shape` files;
>   path/workspace deps are keyed by version string only (`"local"`,
>   `bundle_compiler.rs:147-156`) → editing a path dep is a silent stale hit. Fix:
>   §2.2 now folds transitive dependency `source_hash`es into the key.
>
> None reopen the foundational feasibility (RESULTS-IDENTICAL still holds); each
> is a closure, not a scope change. The 5 supervisor decisions (§6) are unchanged.

Inputs: three feasibility findings —
- Area 1 (to_annotation lowering): `docs/design/compile-cache/feasibility-1-to-annotation-lowering.md` — verdict **gap**.
- Area 2 (.shapec bundle format / extension point): `docs/design/compile-cache/area-2-shapec-bundle-format.md` — verdict **clean** (+ scoped invalidation gap).
- Area 3 (resolved-interface data model): `docs/design/compile-cache/area-3-resolved-interface-data-model.md` — verdict **gap (bounded)**.

---

## 0. The load-bearing decision (read this first)

**The cache serializes the resolved *interface AST* (serde `TypeAnnotation`-level
item nodes), NOT the post-inference `Type`/`TypeScheme`/`MethodTable`, and NOT
`Type::to_annotation()` output.**

This single decision dissolves the Area-1 gap rather than working around it.
Area 1 proves `to_annotation()` is lossy (`Variable→None` at
`type_system/types/core.rs:286`; `Constrained→None` at `core.rs:301`;
`Function` substitutes `Basic("unknown")` at `core.rs:308-315`; the polymorphic
carriers `TypeScheme.quantified/trait_bounds/default_types` at `core.rs:115-125`
and `MethodTable`/`GenericMethodSignature`/`TypeParamExpr` at
`checking/method_table.rs:51-128` are not `TypeAnnotation` at all and have no
lowering). **We never call `to_annotation()` on a cached interface.** Instead we
serialize the exact AST nodes the inference engine already consumes, and on load
we re-run the existing predeclaration/registration passes — the same passes a
from-source compile runs — skipping only parsing and body inference.

This is sound because the registration passes read straight from the serde AST:
- `predeclare_function_signature` (`type_system/inference/items.rs:38-65`) maps
  `func.params[].type_annotation` / `func.return_type` to `Type` via
  `resolve_type_annotation` and only falls into `fresh_type_var()` when the
  annotation is **absent** (`items.rs:50-53`, `:56-59`). A fully-annotated
  signature resolves identically whether the `FunctionDef` came from a fresh
  parse or from the cache.
- `predeclare_struct_type` (`items.rs:88-112`) stores the AST `StructTypeDef`
  verbatim (`items.rs:92-93`) — comptime fields, `@range`/`@description`
  annotations, and defaults survive because they never touch `to_annotation()`
  (corroborated Area 1 §b5, Area 3 §c).
- `register_extend` / `register_trait` / `register_impl` / `register_enum` are
  dispatched from `predeclare_item` (`items.rs:18-36`) and `infer_item`
  (`items.rs:198-208`), all reading AST item nodes.

The `Type`/`TypeScheme`/`MethodTable`/`TypeParamExpr` carriers are *derived,
in-memory, non-serde* (Area 3 §a; `core.rs:92,115`; `method_table.rs:51,78,96,118`
derive only `Debug, Clone`). Serializing them would require either inventing
serde for the inference layer (Area 3 Option B — strictly more surface, includes
the inexpressible `TypeParamExpr::ReceiverParam(i)`) or lowering through the
lossy `to_annotation()`. Both are rejected. We cache the AST that *produces*
those carriers and rebuild them deterministically.

**Consequence for the RESULTS-IDENTICAL binder:** because load replays the same
registration passes over the same AST, the rebuilt `TypeEnvironment` +
`TypeRegistry` + `MethodTable` + `struct_type_defs` are structurally identical to
a from-source prelude/package compile. No lossy variant survives — there is no
round-trip through `to_annotation()` to lose anything (§3 makes this precise and
gives the enforcement test).

---

## 1. Format extension

### 1.1 The new field — annotation-level data model

Add one serde struct, `ResolvedInterface`, defined in `shape-runtime` (alongside
`PackageBundle`). Per Area 3 Option A, every field is a serde-derived AST item
node from `shape-ast` (no inference types, no `to_annotation` output):

```rust
// crates/shape-runtime/src/package_bundle.rs (new struct)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResolvedInterface {
    /// Format-internal revision of the interface schema (independent of the
    /// SHAPEPKG container version; lets the interface evolve without a
    /// container bump and lets the loader hard-reject a too-new interface).
    #[serde(default)]
    pub interface_schema: u32,                  // current = 1

    // [AMENDMENT A] Interface-relevant item defs in EXACT SOURCE ORDER. A single
    // ordered list — NOT grouped-per-kind vectors — because registration is
    // source-order-sensitive (verified: predeclare_item items.rs:18-36 has no
    // Trait/Impl/Enum arm; they register in infer_item items.rs:202-208 in source
    // order). The loader (§2.4) re-runs predeclare_item over `items` then
    // infer_item over `items`, reproducing from-source order bug-for-bug.
    // Item is the shape-ast enum the two passes already match on (no new codec).
    pub items: Vec<shape_ast::ast::Item>,        // ast item enum; carries the
                                                 // FunctionDef / ForeignFunctionDef /
                                                 // StructTypeDef / EnumDef / TraitDef /
                                                 // ImplBlock / ExtendStatement /
                                                 // TypeAliasDef nodes below, in order.

    // module export surface: names + visibility (signatures live in `items`)
    pub exports: Vec<(String, ExportVisibility)>,
}
```

**[AMENDMENT A — closes adversarial BREAK 1; supersedes the grouped-vec model.]**
The cache carries ONE source-ordered `Vec<Item>`, not per-kind vectors. Rationale:
trait/impl/enum registration is order-sensitive (an `impl T for S` textually
before `trait T` sees `T` unregistered → arity check + default-method
registration silently skipped; `register_impl` `lookup_trait` returns `None`).
A grouped replay (all traits, then all impls) would register the trait first and
DIVERGE (fire the arity error, register the default method) — a real accept/reject
+ method-table divergence. A single source-ordered list replayed through the same
two-pass `predeclare_item`→`infer_item` sequence reproduces from-source behavior
exactly. **Sub-decision for the implementer (default stated):** carry ALL
interface-relevant defs in source order regardless of visibility — visibility
gates only the *consumer-visible* query surface (`exports`), NOT registration
order (a private trait can affect a public impl's registration). Item must derive
serde (confirm `shape_ast::ast::Item: Serialize+Deserialize`; if a variant holds a
non-serde field, carry a thin serde projection that preserves the same ordered set
+ the same `predeclare`/`infer` dispatch — still one ordered list).

Why each member is sound (Area 3 §b/§c, all serde-verified at the cited lines):
- `FunctionDef` carries `name` (`functions.rs:14`), `type_params: Option<Vec<TypeParam>>`
  (`functions.rs:24`), `params` each with `type_annotation: Option<TypeAnnotation>`
  (`functions.rs:172`), `return_type: Option<TypeAnnotation>` (`functions.rs:26`),
  `where_clause: Option<Vec<WherePredicate>>` (`functions.rs:27`). `TypeParam::Type`
  carries `trait_bounds: Vec<TypePath>` + `default_type` (`ast/types.rs:212/210`);
  `WherePredicate = {type_name, bounds}` (`ast/types.rs:297`). Generics, bounds,
  and defaults are all expressible at the annotation level — they are NOT taken
  from the lossy `TypeScheme.trait_bounds`/`default_types` (`core.rs:115-125`),
  so the Area-1 §b2 loss never arises.
- `StructTypeDef.fields: Vec<StructField>` (`ast/types.rs:643`); `StructField`
  (`ast/types.rs:674`) has `type_annotation`, `is_comptime`, `annotations`,
  `default_value`, `type_params` — comptime fields + `@range`/`@description`
  survive (Area 1 §b5).
- Method tables are carried *implicitly* by `impls`/`extends`/`structs[].methods`;
  each `MethodDef` (`ast/types.rs:570`) holds params/return as `TypeAnnotation`.
  On load, `register_extend` (`items.rs:807`) and the impl path rebuild
  `MethodTable` + `GenericMethodSignature` + `TypeParamExpr` exactly as a fresh
  compile does — the inexpressible `TypeParamExpr::ReceiverParam(i)`
  (`method_table.rs:52`) is *re-derived*, never serialized.
- Module exports: only **names + visibility** are cached. The runtime
  `ModuleExports` registry holds live fn-pointers (`module_exports.rs:271`) and
  is intentionally NOT serialized; exported signatures live in `functions`.

### 1.2 Where the field is attached (Area 2 §c)

`ResolvedInterface` is **per-module**, attached to `ModuleManifest`
(`crates/shape-runtime/src/module_manifest.rs:13`) as
`#[serde(default)] pub resolved_interface: Option<ResolvedInterface>`. This is the
semantically correct home: a manifest already maps a module's export names to
content hashes (`module_manifest.rs:17`); the resolved interface is "what this
module exposes to a type-checker," the same axis.

Rationale for manifest over the two alternatives (Area 2 §c):
- **Not `PackageBundle`-level**: a flat bundle-level vec loses module attribution
  and forces an extra keying scheme; the manifest is already per-module.
- **Not `FunctionBlob`**: the interface spans structs/enums/traits/impls, not just
  functions; `FunctionBlob` (`content_addressed.rs:33`) is the per-function
  bytecode unit, the wrong granularity. (We also do **not** want to perturb
  `content_hash` — see §1.4.)

**Manifest integrity-hash decision (supervisor item 2.b):** the manifest carries
its own `manifest_hash` over `ManifestHashInput` (`module_manifest.rs:43-51`,
`finalize` `:78`, `verify_integrity` `:104`). The `resolved_interface` field is
**NOT** added to `ManifestHashInput`. Reasons: (i) the interface is fully derived
from source, so the existing `source_hash` gate (§2) already covers staleness;
(ii) keeping it out of the manifest hash means the interface can be regenerated
or schema-migrated without invalidating content-addressed blob identity; (iii)
adding it would couple bytecode identity to type-checker-view encoding, which is
exactly the kind of drift ADR-005/006 warn against. The field is integrity-bound
transitively through `source_hash` instead.

### 1.3 Format-version bump (Area 2 §c; supervisor item 1)

Two independent version knobs, by design:
- **SHAPEPKG container version** `FORMAT_VERSION: u32` (`package_bundle.rs:15`):
  bump `3 → 4`. The new manifest field is `#[serde(default)]`, so this is purely
  additive — a v4 bundle round-trips on a v3 *serde* reader (missing field →
  `None`). But the explicit header range check rejects `version > FORMAT_VERSION`
  (`package_bundle.rs:140`), so a v3 *binary* refuses a v4 bundle. That is the
  honest signal and the recommended behavior: a binary that predates the
  interface schema must rebuild from source rather than silently run with a
  stale/absent interface. `MIN_FORMAT_VERSION` (`package_bundle.rs:17`) stays at
  1; v4 binaries still load v1–v3 bundles (manifest field defaults to `None`,
  triggering rebuild — §2).
- **Interface schema revision** `ResolvedInterface.interface_schema: u32`
  (current = 1): lets the *contents* of the interface evolve (e.g. a future
  let-generalization encoding — see §3.4) without a container bump, and lets a
  loader reject a too-new interface revision while still reading the rest of a
  same-container-version bundle. A loader treats `interface_schema >
  KNOWN_INTERFACE_SCHEMA` as "interface absent" → rebuild.

Net: container `3 → 4`; interface schema `1`.

### 1.4 What does NOT change

`FunctionBlob.content_hash` (`content_addressed.rs:35`) and its input
`FunctionBlobHashInput` (`content_addressed.rs:96-117`) are untouched — the
interface lives on the manifest, not the blob, and is not in any content-hash
input. Bytecode identity / dedup / linking are unaffected.

---

## 2. Cache flow: source-hash → load-or-rebuild

### 2.1 The gap being closed (Area 2 §d)

`source_hash` exists on both `BundleMetadata` (`package_bundle.rs:33`) and
`BundledModule` (`package_bundle.rs:67`) and is computed by the producer
(`bundle_compiler.rs:78-80` per-file, `:142-144` combined) — but it is
**write-only today**: zero read-consumers drive a rebuild decision (Area 2 §d).
The prelude is an unconditional `include_bytes!` with only a deserialize-failure
fallback. This design makes `source_hash` a *read* gate.

### 2.2 What feeds the source-hash (supervisor-visible composition)

The cache key for "is this `.shapec` fresh?" is a tuple, hashed SHA-256 (mirrors
`bundle_compiler.rs:80` formatting; mirrors the working vendored-native cache
precedent `script_cmd.rs:818-858`):

```
source_hash = SHA256(
    source_bytes         // exact UTF-8 of every source file in the module/package,
                         // in a deterministic (sorted-by-module-path) order
  ‖ compiler_fingerprint // [AMENDMENT B] build content-id, NOT coarse semver
  ‖ dep_source_hashes    // [AMENDMENT C] sorted transitive dep source_hashes
  ‖ permission_profile   // the resolved required_permissions surface (sorted
                         // permission name strings, same normalization the
                         // FunctionBlob hash uses, content_addressed.rs:124)
)
```

Four components, each load-bearing:
1. **source bytes** — any source edit changes the hash → rebuild. This is the
   primary trigger.
2. **compiler fingerprint [AMENDMENT B — closes adversarial BREAK 2].** The
   original design folded `BundleMetadata.compiler_version` =
   `env!(CARGO_PKG_VERSION)` (`bundle_compiler.rs:185`). That is too coarse: it
   stays `"0.3.3"` across every dev rebuild of the checker, so editing the
   inference engine / a diagnostic WITHOUT bumping the Cargo version (the normal
   dev cycle, and exactly when the v0.3.3 checker churns most) leaves the key
   unchanged → a stale interface produced by the OLD checker is served → silent
   RESULTS-IDENTICAL violation across checker revisions. Replace with a
   `compiler_fingerprint` that changes on every meaningful compiler rebuild: a
   `build.rs`-generated id = `git rev-parse --short HEAD` + a `--dirty` marker +
   (when dirty/uncommitted) a build timestamp, with a release-build fallback to
   the semver when no git is available. (`vergen`-style; emit as
   `env!("SHAPE_COMPILER_FINGERPRINT")`.) Semver alone is acceptable ONLY for
   immutable published releases; the fingerprint covers both. This forces rebuild
   across any checker change and protects §3 from comparing artifacts produced by
   *different* checkers.
3. **dependency source hashes [AMENDMENT C — closes adversarial BREAK 3].** The
   original key hashed only the package's OWN `.shape` files; dependencies were
   recorded by version string and a local path dep defaults to literal `"local"`
   (`bundle_compiler.rs:147-156`), so editing a path/workspace dep's source does
   NOT change the key → stale cross-package hit. Fold the **transitive**
   dependency `source_hash`es (each dep already computes one — `package_bundle.rs:67`)
   into the key, sorted by dependency module path. For a path/workspace dep this
   means its recomputed `source_hash` (over ITS source bytes), Merkle-style, so a
   dep-source edit propagates to every dependent's key. Registry deps pinned to an
   immutable published version may use the version string (immutable by registry
   contract); path/workspace/git deps MUST use the recomputed source hash.
4. **permission profile** — permissions are baked into `FunctionBlob.content_hash`
   (CLAUDE.md Security Model tier 1; `content_addressed.rs:116,124`). The same
   source under a different permission scope is a different artifact; folding the
   sorted permission names in keeps the cache key consistent with content-hash
   semantics and prevents a permission-narrowed rebuild from serving a
   broader-permission cache hit.

### 2.3 Load-or-rebuild decision (clean hook points, all from Area 2 §d)

```
on load(module / package):
    fresh_key = source_hash(source_bytes, compiler_version, permission_profile)
    if .shapec exists
       AND container_version in MIN..=FORMAT_VERSION
       AND bundle.metadata.source_hash == fresh_key
       AND manifest.resolved_interface is Some
       AND resolved_interface.interface_schema <= KNOWN_INTERFACE_SCHEMA:
          → LOAD: register interface via the replay passes (§2.4); skip parse+infer.
    else:
          → REBUILD from source; write fresh .shapec (producer stamps fresh_key).
```

Hook points (all single-function, verified clean in Area 2 §d):
- **Producer / write:** `BundleCompiler::compile` (`bundle_compiler.rs:24`) —
  already computes `source_hash` (`:78-80,142-144`) and stamps metadata
  (`:182-198`). Extend to compute the §2.2 *tuple* key (currently only source
  bytes), and to emit the `ResolvedInterface` per module into the manifest.
- **Consumer / load:** `ModuleLoader::set_dependency_paths`
  (`module_loader/mod.rs:324`) → `load_bundle` (`module_loader/mod.rs:372`), or a
  thin wrapper around `PackageBundle::read_from_file` (`package_bundle.rs:159`).
  This is where the freshness comparison and the LOAD-vs-REBUILD branch live.
- **Prelude:** `load_core_modules_best_effort` (`crates/shape-runtime/src/engine/stdlib.rs`)
  — single chokepoint; the source-hash compare slots next to the existing
  env-override + deserialize-fallback branches (see §4.3 for prelude
  unification).

### 2.4 LOAD path = replay, not re-infer

On a fresh cache hit, the loader deserializes `ResolvedInterface` and replays the
existing registration passes as a **two-pass walk over the single source-ordered
`items` list** (Amendment A): pass 1 = `predeclare_item` over `items[0..n]` in
order (`items.rs:18-36`: registers fn/foreign/struct/extend signatures), then
pass 2 = `infer_item` over `items[0..n]` in order (`items.rs:198-208`: registers
trait/impl/enum/extend, the order-sensitive ones). Because `items` is in exact
source order, both passes see items in the identical order a from-source compile
would — reproducing forward-reference / order-sensitive registration bug-for-bug.
The replay calls `predeclare_function_signature` / `predeclare_foreign_function` /
`predeclare_struct_type` / `register_extend` / `register_trait` /
`register_impl` / `register_enum` / `define_type_alias` — **No parse. No body
inference. No `infer_function`.** Bytecode comes from the existing
`blob_store`/`manifests` path (`module_loader/mod.rs:377-426`); the interface
comes from the replay. Both halves keyed by the same `source_hash`.

---

## 3. The RESULTS-IDENTICAL binder

### 3.1 Precise statement

> **BINDER.** For any module M whose `.shapec` is a fresh cache hit
> (`source_hash` tuple matches, §2.2), type-checking any consumer program P that
> imports M produces **byte-identical diagnostics** (same set, same order, same
> spans, same messages, same LSDS rendering) whether M's interface was obtained
> by (i) compiling M from source then checking P, or (ii) loading M's
> `ResolvedInterface` from the `.shapec` cache then checking P. Identical applies
> to: accept/reject verdict; every diagnostic; every inferred type surfaced to P
> (hover, inlay, completion); and bidirectional closure inference through M's
> generic method signatures.

The binder is scoped to the **interface** of M as seen by a consumer. It does NOT
claim M's *own body* re-checks identically from cache (bodies are not cached —
§5); it claims M's exposed signatures/types drive consumer checking identically.

### 3.2 Why it holds — the no-loss argument (head-on with Area 1)

The binder requires that **no lossy `to_annotation()` variant ever appears in a
resolved interface.** This is satisfied by construction, route (a) of the prompt's
dichotomy ("prove it never appears"), NOT route (b) (extend to_annotation):

1. `to_annotation()` is **never called on cached-interface data.** The cache
   stores AST `TypeAnnotation` nodes (§1.1) and load replays
   `resolve_type_annotation` (`items.rs:51,57`), which goes
   `TypeAnnotation → Type` (the *forward* direction), never `Type → TypeAnnotation`.
   The lossy arms (`core.rs:286,301,308-315`) are on the reverse direction and are
   not on any cache path.
2. The polymorphic carriers Area 1 flagged as having *no* lowering
   (`TypeScheme.quantified/trait_bounds/default_types`; `MethodTable`;
   `TypeParamExpr`) are **rebuilt** by the replay passes from `TypeParam`/
   `WherePredicate`/`MethodDef` AST, identically to a fresh compile. They are
   never serialized, so their lack of lowering is irrelevant.
3. The remaining Area-1/Area-3 loss class is **unannotated public signatures**:
   `predeclare_function_signature` falls into `fresh_type_var()` only when an
   annotation is absent (`items.rs:50-53,56-59`). For these, the *fresh compile*
   itself produces a `Type::Variable` that `to_annotation()` can't express — i.e.
   the lossy shape is in the source, not introduced by the cache. Two
   sub-dispositions (§3.4): for the prelude/foreign fns this never occurs
   (annotation-required, `functions.rs:110`; stdlib fully annotated, Area 3 §c);
   for user packages, the encoding for unannotated public signatures is the
   subject of the let-generalization decision (§3.4) and is gated behind
   `interface_schema` until ratified.

Net: **no lossy `to_annotation()` variant survives in a cached interface**,
because no `to_annotation()` call is on the cache path and the only source-side
loss (unannotated publics) is either excluded by precondition (prelude) or
deferred behind a schema gate (user packages) — never silently lossy.

### 3.3 Enforcement test design

A `#[cfg(test)]` differential test in `shape-runtime` (or `shape-test` integration
tier), per CLAUDE.md testing conventions (unit tests in-source):

```
fn results_identical(module_src: &str, consumer_src: &str) {
    // Route A: from source
    let engine_a = fresh_engine();
    engine_a.compile_and_register_interface_from_source(module_src);
    let diags_a = engine_a.check(consumer_src);   // collect ALL diagnostics + inferred-type surface

    // Route B: via .shapec round-trip
    let bundle = BundleCompiler::compile(module_src);     // produces ResolvedInterface
    let bytes  = bundle.to_bytes().unwrap();              // SHAPEPKG container
    let bundle2 = PackageBundle::from_bytes(&bytes).unwrap();
    let engine_b = fresh_engine();
    engine_b.load_and_replay_interface(bundle2.manifest_for(module).resolved_interface);
    let diags_b = engine_b.check(consumer_src);

    assert_eq!(render_lsds(diags_a), render_lsds(diags_b));  // byte-identical
    assert_eq!(inferred_surface(engine_a), inferred_surface(engine_b));
}
```

Required corpus (must include the Area-1 hard cases so the test would FAIL if a
`to_annotation()` round-trip ever crept in):
- generic stdlib fns with bounds + defaults: `clamp<T: Ord>(x:T,lo:T,hi:T)->T`
  (`stdlib-src/core/math.shape:183`); a fn with a default generic param.
- generic method tables (the bidirectional-inference case): `Vec.map<U>(f:(T)=>U)->Vec<U>`,
  `reduce<U>`, `filter` (`stdlib-src/core/vec.shape:46-64`) — consumer does
  `arr.map(|x| ...)` and the test asserts `x`'s inferred type is identical on both
  routes (this is the case that would regress to `(unknown)=>unknown` under a
  `to_annotation()` seam — Area 1 §c.3).
- `Some/Ok/Err` constructors (`environment/mod.rs:935-988`) with the `E=AnyError`
  default.
- struct with comptime field + `@range`/`@description` annotation
  (`StructField.is_comptime`/`annotations`, `ast/types.rs:674`).
- trait + impl + extend exercising `TypeParamExpr::ReceiverParam`/`MethodParam`.
- a negative case: a consumer that SHOULD get a type error against M's interface —
  assert the same error fires on both routes (proves the cache doesn't silently
  widen to `unknown` and accept).

A second guard test asserts the **container round-trip is byte-stable**:
`bundle.to_bytes()` → `from_bytes` → `to_bytes()` is identical (catches
nondeterministic map ordering in the MessagePack encoding of `ResolvedInterface`;
mitigated by using `Vec<(K,V)>` not `HashMap` for the exports surface — already
done in §1.1).

**[AMENDMENT, lower-severity adversarial finding] TypeVar-id normalization in the
differential test.** The two routes start their `fresh_type_var()` counter
(`inference/items.rs:747` etc.) at potentially different points, so any inferred
`Type::Variable` whose id leaks into a rendered diagnostic/hover string would make
`render_lsds(diags_a) != render_lsds(diags_b)` *spuriously* (a false binder
failure) — or, worse, mask a real difference if ids happen to coincide. For v1
(`interface_schema = 1`, annotation-required) this path is not hit (no
`fresh_type_var` for fully-annotated cached sigs), but the test MUST either (i)
normalize TypeVar ids in the rendered output before comparison, or (ii) reset the
counter to the same seed on both routes. State this explicitly so the test neither
passes nor fails for the wrong reason.

### 3.4 The unannotated-public-signature disposition (supervisor item, coupled to let-generalization)

This is the ONLY residual where the binder needs an explicit ruling:
- **Prelude + foreign fns:** excluded by precondition. Foreign fns are
  annotation-required (`functions.rs:110`); stdlib is fully annotated (Area 3 §c).
  Binder holds unconditionally here. `interface_schema = 1` covers this.
- **User packages with unannotated public fns:** per MEMORY
  `project_let_generalization.md` (user ruling 2026-05-31), Shape does HM
  let-generalization — an unconstrained fn-return type var becomes an inferred
  generic param (`fn get_none(){None}` ⇒ `fn get_none<T>()->Option<T>`). Under
  that rule, an unannotated public signature resolves to a *generalized scheme*,
  which is expressible at the annotation level as an inferred `<T>` generic
  param + `TypeAnnotation::Generic`/`Reference`. The clean encoding is: the
  producer writes the **generalized** form into `ResolvedInterface.functions`
  (synthesizing the inferred `TypeParam`s), so the cached AST is fully annotated
  even though the source wasn't. This is NOT a runtime escape hatch and respects
  no-dynamic-fallback — it is a compile-time lowering of an inferred scheme back
  to annotation-level generics, performed by the producer, replayed identically
  by the loader.
  - **Gate:** this generalized-encoding path lands under `interface_schema = 2`
    and is **out of scope for the initial cache** (`interface_schema = 1`
    requires fully-annotated public signatures, mirroring `ForeignFunctionDef`).
    Until ratified + implemented, a package with unannotated public fns either
    (i) is required to annotate, or (ii) does not get an interface cached (its
    manifest `resolved_interface = None` → consumers rebuild it from source). No
    `Basic("unknown")` is ever written to a cached interface. **Supervisor must
    ratify whether v1 ships annotation-required or waits for the let-gen
    encoding.**

---

## 4. Serves-three-purposes note

The `.shapec` + `ResolvedInterface` artifact is the Shape analogue of Rust
`.rmeta` and TypeScript `.d.ts`. One artifact, three consumers:

1. **Compile-cache (10–50 ms).** A consumer importing M skips parse + body
   inference for M; it deserializes `ResolvedInterface` and replays signature-only
   registration (§2.4). The 10–50 ms is the target for swapping a from-source
   prelude/dependency compile (parse + full inference of every stdlib/dep module)
   for a deserialize + replay. The prelude is the highest-leverage case (every
   compile pays for it today — Area 2 §d, `include_bytes!` + best-effort
   compile).
2. **Cross-machine signature discovery / debug.** Because the interface is
   annotation-level, registry-ID-free serde (no `ConcreteType`/`StructLayoutId`/
   `Arc<VTable>` — the `#[serde(skip)]` cautionary precedent, Area 2 §c), a
   `.shapec` is portable: an LSP/registry on machine B can read M's full public
   signature surface (fns, types, traits, generic methods, doc annotations)
   without M's source. This is the `.d.ts` role.
3. **Execution.** The `.shapec` already carries `blob_store` + `manifests` of
   `FunctionBlob` bytecode (Area 2 §a). The interface field rides alongside; the
   same artifact type-checks AND runs. This is what distinguishes it from a
   pure-metadata `.rmeta` — it is `.rmeta` + `.d.ts` + an executable image in one
   versioned container.

The annotation-level choice (§0) is what makes purposes 2 and 3 coherent with 1:
a `Type`/`TypeScheme`-level cache (Area 3 Option B) would carry inference-internal,
registry-bound, non-portable state — unusable cross-machine and a drift hazard.

---

## 5. Scope boundary

- **Annotation-level interface ONLY.** The cache stores serde AST item nodes
  (`FunctionDef`/`StructTypeDef`/`EnumDef`/`TraitDef`/`ImplBlock`/`ExtendStatement`/
  `TypeAliasDef`/`ForeignFunctionDef`) + export names/visibility (§1.1). Nothing
  below the annotation layer.
- **NOT the raw inference engine.** `TypeInferenceEngine` (`inference/mod.rs:56`)
  and its derived carriers — `TypeScheme` (`core.rs:116`), `Type` (`core.rs:92`),
  `TypeVar`/`TypeConstraint` (`core.rs:46`/`constraints.rs:9`), `MethodTable`/
  `MethodSignature`/`GenericMethodSignature`/`TypeParamExpr`
  (`method_table.rs:51-128`), `callsite_type_args` and the callsite scratch maps
  (`inference/mod.rs:88-131`), `HoistedField` (`environment/mod.rs:23`) — are
  **never serialized**. They are rebuilt by the replay passes. None of them derive
  serde and that stays true.
- **NOT an in-memory VM snapshot.** This is unrelated to `snapshot()` /
  `vm_state_snapshot` / W17 whole-VM restore. The interface cache is a
  compile-time type-checker artifact; it carries no live `KindedSlot`,
  `NativeKind`, heap, or stack state. (Distinct from MEMORY
  `project_w17_snapshot_completion.md`.)
- **NOT method implementations.** Builtin method impls are `&'static phf::Map`
  fn-pointers baked into the binary (`method_registry.rs:260+`; Area 3 §a) —
  identical across any process on the same binary, never serialized. Only the
  type-checker's *view* of method signatures (rebuilt via `register_extend`) is
  interface state.

---

## 6. Supervisor decisions required (consolidated)

1. **Format-version mechanism.** Ratify the dual-knob scheme: SHAPEPKG container
   `FORMAT_VERSION 3→4` (`package_bundle.rs:15`) + a separate
   `ResolvedInterface.interface_schema` (= 1) revision. Confirm the forward-compat
   policy: a v3 binary **rejects** v4 bundles (current `>FORMAT_VERSION` reject at
   `package_bundle.rs:140`) rather than silently tolerating — design recommends
   keeping the reject (forces rebuild on stale binary).
2. **Where `to_annotation` must be extended.** Design answer: **nowhere.**
   `to_annotation()` is off the cache path entirely (§0, §3.2); the seam is the
   forward `resolve_type_annotation` replay. Ratify that we do NOT extend
   `to_annotation()` and do NOT add serde to the inference layer (rejects Area 3
   Option B). If the supervisor instead wants a `Type`-level cache, that reopens
   the Area-1 lossy-variant problem and the binder cannot hold without inventing
   serde for `TypeParamExpr::ReceiverParam` — design recommends against.
3. **Manifest integrity-hash coverage.** Ratify that `resolved_interface` is NOT
   added to `ManifestHashInput` (`module_manifest.rs:43-51`); it is integrity-bound
   transitively via `source_hash` (§1.2).
4. **Prelude-vs-package cache unification.** Two artifacts share the inner
   `BytecodeProgram` codec but NOT the container today (Area 2 §e: prelude =
   header-less `include_bytes!` bare `BytecodeProgram`; `.shapec` = versioned
   `SHAPEPKG`). Choose: **(4a)** wrap the prelude as a degenerate `SHAPEPKG`
   bundle (one container, one `ResolvedInterface` schema, one source-hash gate —
   design's recommendation; highest leverage since every compile pays the prelude
   cost), or **(4b)** keep two thin readers sharing the inner codec + the
   source-hash helper. (4a) is more invasive (touches the `include_bytes!` baking
   in `engine/stdlib.rs` + `stdlib_gen`) but unifies the binder's enforcement to a
   single path.
5. **Unannotated-public-signature disposition (coupled to let-generalization).**
   Ratify: v1 (`interface_schema = 1`) is **annotation-required** for cached public
   signatures (no interface cached otherwise → consumer rebuilds from source);
   the let-generalization producer-side generalized encoding (§3.4) lands later
   under `interface_schema = 2`. Confirm no `Basic("unknown")` is ever written to
   a cached interface under either schema.

---

## 7. Risks

- **R1 — silent `to_annotation()` reintroduction.** A future "optimization" that
  caches `Type`/`TypeScheme` directly (Area 3 Option B) reopens every Area-1 loss
  and breaks the binder. Mitigation: the differential test §3.3 (with the
  `Vec.map<U>` bidirectional-inference case) fails loudly if a lossy round-trip
  appears; the no-serde-on-inference-layer rule (decision 6.2) is the structural
  guard. This is a defection-attractor of the same family CLAUDE.md warns about
  (the "rename a deletion into a permanent shim" pattern) — flag in review.
- **R2 — MessagePack map nondeterminism.** `HashMap` fields serialize in
  arbitrary order, breaking byte-identical cache keys / round-trip stability.
  Mitigation: `ResolvedInterface` uses `Vec<(K,V)>` for exports (§1.1); the
  byte-stable round-trip guard test (§3.3) catches regressions. Note the existing
  `FunctionBlobHashInput` already relies on deterministic struct-as-array
  encoding (`content_addressed.rs:144-146`).
- **R3 — replay-order divergence [CLOSED by Amendment A].** Trait/impl/enum
  registration is source-order-sensitive (verified: predeclare has no arm for
  them; they register in `infer_item` in source order). The original grouped-vec
  model could NOT reproduce a from-source interleaving (impl before trait). Closed:
  §1.1 now carries a single source-ordered `Vec<Item>` and §2.4 replays it through
  the same two-pass `predeclare_item`→`infer_item` walk; the differential test
  §3.3 MUST include an impl-before-trait + forward-reference corpus case to guard
  against regression to a reordering encoding.
- **R4 — compiler-identity too coarse [CLOSED by Amendment B].** `compiler_version`
  = `env!(CARGO_PKG_VERSION)` does NOT change across dev rebuilds of the checker at
  the same semver → stale-interface served by a changed checker. Closed: §2.2 now
  keys on a `compiler_fingerprint` (build-time git/dirty/timestamp id) that changes
  on every meaningful compiler rebuild.
- **R8 — cross-package dependency staleness [CLOSED by Amendment C].** The original
  key hashed only the package's own source; path/workspace deps were keyed by
  version string (`"local"`), so a dep-source edit was a silent stale hit. Closed:
  §2.2 folds transitive dependency `source_hash`es (Merkle-style) into the key for
  path/workspace/git deps; the differential test should include a 2-package
  path-dep fixture where editing the dep must force a dependent rebuild.
- **R5 — let-generalization encoding scope creep.** The §3.4 generalized-encoding
  path (interface_schema 2) is genuinely more work and couples to an in-flight
  type-system feature. Risk: it gets pulled into v1 and stalls the cache.
  Mitigation: v1 is annotation-required (decision 6.5); the let-gen path is gated
  behind `interface_schema` and is strictly additive later.
- **R6 — prelude unification invasiveness (decision 4a).** Re-baking the prelude
  as a `SHAPEPKG` bundle touches `engine/stdlib.rs` `include_bytes!` and the
  offline `stdlib_gen --verify` path. Risk of breaking the existing
  deserialize-failure fallback. Mitigation: keep the bare-`BytecodeProgram`
  fallback as a last resort; gate the unified path behind the source-hash check so
  failure degrades to today's behavior.
- **R7 — partial-bundle staleness.** A bundle where bytecode (`blob_store`) and
  `resolved_interface` were produced at different times could disagree.
  Mitigation: both are produced in one `BundleCompiler::compile` pass under one
  `source_hash` (§2.3); never written independently. The freshness gate keys both
  halves on the same hash.
