# Adversarial verification — RESULTS-IDENTICAL binder of the `.shapec` compile-cache

Scope: try to BREAK the binder in `docs/design/compile-cache/DESIGN.md` §3.1.
Read-only against workspace HEAD. Every claim cites `file:line`.

Verdict: **breaks_found = true; severity = minor** for the surviving cases.
The design's central move — serialize the *AST item nodes*, never call
`to_annotation()`, and replay the existing `predeclare_*`/`register_*` passes
(§0, §2.4) — is sound and genuinely dissolves the Area-1 lossy-`to_annotation`
class. I could NOT break the binder via the to_annotation losses, the
generic-method/`TypeParamExpr` bidirectional-inference case, comptime fields,
field annotations, default args, supertrait *carriage*, or associated types:
all of those are carried as serde AST and rebuilt by replay. The surviving
breaks are (1) a **cross-kind replay-order** gap that the chosen grouped-vector
data model cannot honor (R3 is unmitigated *by construction*), and (2) two
**invalidation holes** (coarse `compiler_version`; dependency-source drift not
in `source_hash`). All three are constructible but each requires a specific
source-order pathology or a path-dependency / same-version rebuild — hence
minor, not blocking, but they ARE results-not-identical / stale-cache-silent-
wrong, so they must be closed before the binder can be asserted unconditionally.

---

## What I could NOT break (the design holds here)

- **`to_annotation()` losses (Area-1) never reach a cached interface.** The
  cache stores AST `TypeAnnotation` nodes and load replays the *forward*
  `resolve_type_annotation` (`inference/items.rs:460-543`), never the reverse
  `Type::to_annotation()`. The lossy arms (`types/core.rs:286,301,308-315`) are
  off every cache path. Confirmed.
- **Generic method tables / bidirectional closure inference
  (`Vec.map<U>`).** `register_extend` (`inference/items.rs:807-915`) and
  `annotation_to_type_param_expr` (`items.rs:925`) build
  `GenericMethodSignature`/`TypeParamExpr` purely from the `ExtendStatement` AST
  + the `len<=2 && uppercase` type-param heuristic (`items.rs:826-827`). No body,
  no `to_annotation`. A replay over the same AST rebuilds the same
  `ReceiverParam(i)`/`MethodParam(i)` tree, so `arr.map(|x| ...)` infers `x`
  identically on both routes. This is the design's marquee corpus case and it
  survives.
- **Comptime fields + `@range`/`@description` annotations.** Stored verbatim in
  `struct_type_defs` via `predeclare_struct_type`
  (`items.rs:92-93`); the structural alias filters comptime fields and blanks
  `annotations` on BOTH routes identically (`items.rs:101-110`, `:185-194`).
  No route divergence.
- **Default args.** Consumer-relevant info is `default_value.is_some()` →
  `callable_param_defaults` (`items.rs:39-45`), rebuilt by the replayed
  predeclare. The default *expression* is type-checked only inside M's own
  `infer_function` body (`items.rs:340-345`), which the consumer never re-runs
  and the binder explicitly does not cover (M's body, §3.1). No divergence.
- **Supertrait *carriage* (`extends`).** `TraitDef.super_traits`
  (`shape-ast/.../types.rs:514`) is serialized in full; `TypeAliasDef`
  carries `meta_param_overrides: Option<HashMap<String, Expr>>`
  (`types.rs:351`); `Expr` is fully serde. No data loss. (But see Break 1 — the
  *order* in which supertraits are visible at scheme-build time IS lossy.)
- **Fully-annotated exported fn signatures are scheme-stable.** For an
  annotated fn, `infer_function` derives params/return from the same
  `resolve_type_annotation` calls (`items.rs:314-316,348-349`) the predeclare
  pass uses, so the final `make_function_scheme` output equals the predeclare
  output — replay-without-`infer_function` reproduces it. (This is exactly why
  v1 is annotation-required, §3.4 / decision 6.5; the design is internally
  consistent here.)

---

## BREAK 1 — cross-kind replay order is not preserved; trait/impl/scheme registration is order-sensitive

This is the strongest surviving break. It is a results-NOT-identical case
(accept/reject AND method-table divergence), but it requires a specific
source-order pathology, so I rate the *cluster* minor rather than blocking.

### The order-sensitivity (from-source behavior)

From-source runs ONE interleaved pass per phase in **source order**:
`predeclare_item` over all items, then `infer_item` over all items
(`inference/mod.rs:1246-1256`). Trait registration happens only in `infer_item`
(`items.rs:200-202` → `register_trait` → `env.define_trait`,
`items.rs:546-548`); it is NOT in `predeclare_item` (`items.rs:18-36` has no
`Trait` arm). Two registration paths read the trait registry *as it stands at
that moment*:

1. **`register_impl`** (`items.rs:552-715`, called from `infer_item`
   `items.rs:203-205`) does `self.env.lookup_trait(&trait_name)` THREE times:
   - arity validation (`items.rs:584`),
   - comptime-alignment check (`items.rs:563`),
   - default-method registration into the method table (`items.rs:673-688`).
   Each is `if let Some(trait_def) = ...` — i.e. **silently skipped when the
   trait is not yet registered.** `lookup_trait` returns `None` for an
   unregistered name (`environment/registry.rs:195`, `:425-427`).

2. **`make_function_scheme`** (`items.rs:1107-1178`) expands trait bounds with
   `self.env.get_transitive_supertrait_names(trait_name)`
   (`items.rs:1133`, `:1156`), which reads `self.traits` and returns `[]` for an
   unregistered trait (`environment/registry.rs:425-427,443-462`). The expanded
   set is stored in `TypeScheme.trait_bounds` and is later emitted as
   `ImplementsTrait` constraints **at the consumer's call site**
   (`types/core.rs:200-214`).

### Why the cached data model cannot reproduce it

`ResolvedInterface` (DESIGN §1.1) stores **separate per-kind vectors**
(`functions`, `structs`, `enums`, `traits`, `impls`, `extends`, `type_aliases`),
with NO unified item ordering and no instruction to reinterleave. §2.4 tells the
loader to replay in "the same order `predeclare_item`/`infer_item` use" — but
that order is *source order across kinds*, which the grouped vectors have thrown
away. R3 (DESIGN §7) says "the loader MUST replay in the same two-pass order"
yet the data model it chose in §1.1 does not carry the cross-kind interleaving
needed to do so. The mitigation and the data model are mutually incompatible.
(Per-def `Span`s exist — `functions.rs:15`, `types.rs:431,575` — so a span-sort
*could* reconstruct order, but the design does not specify it, and span-sort is
unsound for synthesized items with `Span::DUMMY` (`types.rs:288`) and for
multi-file modules whose per-file byte offsets are not globally comparable.)

### Repro shape (impl-before-trait → accept/reject + method-table divergence)

```shape
// module M — impl textually precedes the trait it implements
impl Greet for Dog {
    fn hello(self) -> string { "woof" }
    // NOTE: arity here intentionally disagrees with the trait below
}

trait Greet {
    fn hello(self, loudness: int) -> string;   // arity 2 (self, loudness)
    fn polite(self) -> string { "good day" }   // default method
}
```

- **From-source** (source order): when `register_impl` runs, `Greet` is not yet
  registered → `lookup_trait` is `None` → arity mismatch NOT diagnosed
  (`items.rs:584`), and the default `polite` is NOT added to `Dog`'s method
  table (`items.rs:673`). A consumer `dog.polite()` → "method not found".
- **Cache replay** (grouped: traits before impls): `Greet` is registered first →
  `register_impl` finds it → arity mismatch `TraitImplArityMismatch` FIRES
  (`items.rs:604-611`), `polite` IS registered. A consumer `dog.polite()`
  succeeds, but M now fails to load with a type error it did not have from
  source.

That is **accept-vs-reject divergence** plus a **method-table content
divergence** — squarely a binder violation under §3.1 ("same accept/reject
verdict; every diagnostic; every inferred type surfaced to P").

### Repro shape (supertrait expansion → consumer call-site accept/reject)

```shape
// module M
fn need_display<T: Display>(x: T) -> string { ... }   // bound: T: Display

trait Display: Debug { fn show(self) -> string; }      // Display extends Debug
```

`make_function_scheme` for `need_display` expands `Display` to also require
`Debug` only if `Display` (with its `super_traits`) is already in the registry
when the scheme is (re)built. From-source predeclare builds the scheme before
`Display` is registered (predeclare has no Trait arm), so the *predeclared*
scheme has bounds `{Display}` only; the authoritative `infer_item` rebuild
(`items.rs:124`) re-runs `make_function_scheme` and picks up `Debug` IF `Display`
was registered earlier in source order. A grouped replay that registers all
traits before re-deriving the fn scheme always sees `{Display, Debug}`. A
consumer calling `need_display(x)` where `x: Display` but `x: !Debug` then gets
DIFFERENT `ImplementsTrait` constraints (`core.rs:200-214`) → different
reject/accept. Constructible whenever the supertrait `extends` is textually
after the bounded generic fn.

**Design mitigation status: NOT mitigated.** R3 names the risk but the §1.1 data
model cannot honor R3. Fix requires either (a) serializing a single ordered
`Vec<Item>` (or an ordering index) instead of per-kind vectors, or (b)
specifying a deterministic, source-order-faithful replay order and proving it
matches from-source for all cross-kind interleavings — neither is in the design.

---

## BREAK 2 — `compiler_version` is `CARGO_PKG_VERSION`, too coarse to gate checker drift

`source_hash` folds in `compiler_version` (DESIGN §2.2, R4). The actual value is
`env!("CARGO_PKG_VERSION")` (`crates/shape-vm/src/bundle_compiler.rs:185`) — the
crate semver string, e.g. `"0.3.3"`. It does NOT include a git hash, build
timestamp, or content hash of the checker.

Repro shape (stale-cache silent-wrong):
1. Build compiler at version `0.3.3`. Compile package M → `.shapec` stamped
   `compiler_version = "0.3.3"`.
2. Edit the inference engine (e.g. change supertrait expansion, a narrowing
   rule, or a diagnostic) WITHOUT bumping `Cargo.toml` version — the normal state
   of every pre-release dev cycle. Rebuild.
3. Recompile a consumer P importing M. The fresh-key tuple recomputes to the
   same `(source_bytes, "0.3.3", perms)` → **cache HIT** → P is checked against
   M's interface as produced by the *old* checker. Diagnostics diverge from a
   from-source compile by the new checker.

This is precisely the R4 failure ("a compiler upgrade serves a stale interface")
that the design claims is mitigated, but the mitigation relies on
`compiler_version` changing — and it does not change across same-version
rebuilds, which is the dominant case during development (MEMORY notes the v0.3.3
checker is under active, heavy churn: strict-flip, let-gen, etc.).

**Design mitigation status: under-mitigated.** Fix: fold a build-identity that
changes on any checker rebuild — a git commit hash, a content hash of the
compiler binary, or an interface-encoder version distinct from semver — into the
key. The existing `interface_schema` knob (§1.3) only versions the *encoding*,
not the *checker behavior*, so it does not close this.

---

## BREAK 3 — dependency source drift is not in `source_hash`

DESIGN §2.2 says `source_hash` is over "the exact UTF-8 of every source file in
the module/package." The actual computation hashes ONLY the package's own
discovered `.shape` files: `all_sources` accumulates `source` per discovered file
(`bundle_compiler.rs:83`) and the combined hash is `SHA256(all_sources)`
(`bundle_compiler.rs:142-144`). Dependencies are recorded separately, by **version
string only**, and a local `path` dependency defaults to the literal
`"local"` (`bundle_compiler.rs:147-156`, esp. `:152`). Dependency source bytes
never enter `source_hash`.

Repro shape (stale-cache silent-wrong across a path dependency):
1. Package M depends on a local path dependency N (`shape.toml`
   `N = { path = "../N" }`). M's `.shapec` stamps
   `dependencies["N"] = "local"`.
2. Edit `../N/*.shape` to change N's exported interface (e.g. change a return
   type, drop an export, add a trait bound). N's "version" is still `"local"`.
3. Recompile M. M's own source bytes are unchanged → `source_hash` unchanged,
   `dependencies["N"]` still `"local"` (and `dependencies` is not even in the
   §2.2 tuple) → **cache HIT** → M is checked / served against the OLD N
   interface. A consumer of M (or M's own re-check) accepts/rejects against a
   stale N, diverging from a from-source compile.

This is broader than R7 (R7 only covers within-bundle bytecode-vs-interface
consistency under one `BundleCompiler::compile`). Cross-package dependency drift
is unaddressed by the §2.2 tuple.

**Design mitigation status: NOT mitigated** for path/workspace deps. Fix: the
freshness key must include the resolved interface-hash (or source-hash) of every
transitive dependency, not a per-dep version string that is `"local"` for path
deps. Registry deps pinned by exact immutable version are safer, but path/git
deps are the common dev case.

---

## Lower-severity observations (not independent breaks)

- **`fresh_type_var()` counter skew.** Replay paths call `self.fresh_type_var()`
  for unannotated params (`items.rs:747,756,860,869,895,900,318,351`). The
  global var counter is at a different value on the replay route (M's body
  inference never ran) than from-source, so any `TypeVar` id that leaks into a
  rendered diagnostic or hover would differ byte-for-byte — violating the
  "byte-identical … LSDS rendering" clause of §3.1. For v1 annotation-required
  interfaces this path is not hit (annotated → no fresh var in the signature),
  so it is latent, but the §3.3 differential test must assert TypeVar ids are
  normalized or it could fail spuriously / mask the issue. Tighten the binder to
  "byte-identical modulo TypeVar id renumbering" or normalize before compare.
- **`type_aliases`/`structs` define-order vs. `Reference` resolution.**
  `resolve_type_annotation` follows aliases eagerly (`items.rs:471-473`); if a
  cached alias references another alias, grouped replay order within
  `type_aliases`/`structs` must match source order or alias resolution can
  differ. Same root cause as Break 1; same fix (ordered replay).

---

## Bottom line

The annotation-level / replay-the-passes architecture defeats the Area-1
`to_annotation` loss class — that part of the binder is solid and I could not
break it. What survives:

- **Break 1 (results-NOT-identical):** the grouped-per-kind data model
  structurally cannot honor R3's "replay in source order," and trait/impl/scheme
  registration is genuinely source-order-sensitive (arity reject, default-method
  registration, transitive-supertrait bound expansion → consumer call-site
  accept/reject). Constructible with an impl-before-trait or
  fn-before-supertrait-extends ordering.
- **Break 2 (stale-cache silent-wrong):** `compiler_version` =
  `CARGO_PKG_VERSION` does not change across same-version checker rebuilds.
- **Break 3 (stale-cache silent-wrong):** dependency source drift (path deps
  recorded as `"local"`) is absent from `source_hash`.

None is unconditionally always-on, so I rate the set **minor**, but Break 1 is a
true results-not-identical hazard and Breaks 2–3 are silent-wrong invalidation
holes — all three must be closed (ordered replay; finer build identity;
dependency-interface-hash in the key) before the §3.1 binder can be asserted
without caveats.
