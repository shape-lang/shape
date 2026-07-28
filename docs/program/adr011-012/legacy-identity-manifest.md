# The finite legacy identity manifest (#136)

**Authority:** rulings file, "#90 authority enactment — ten required steps",
step 5; ruling R14; ADR-011 §1.
**Artifact:** `docs/program/adr011-012/legacy-identity-manifest.json`.
**Source revision:** `11d9c546c811f0bb2e7522635ba05e6ddc4e2cb0`.
**Commands:** `just check-legacy-identity` / `just regen-legacy-identity`.

R14 requires a legacy manifest "keyed by resolved semantic identity, never
spelling". The #133/#134/#135 baselines are spelling-keyed censuses — the right
instrument for step 4's mechanical inventory, and explicitly not this. This
document records how the spelling→identity translation was done, what it
proves, and the two parts of step 5 that could not be delivered honestly at this
revision.

## The key scheme

Identity key is `BuiltinFunction::<Variant>`.

It is not a Shape source spelling. It names the *behavior*, lives in the
compiler's own namespace, is unreachable from user code, and survives any rename
of a surface name. A same-spelled user declaration cannot collide with it, which
is the property spelling lacks and the reason ADR-011 rejects spelling as
authority.

The translation does observable work rather than relabelling a census:

- **149 live spellings collapse to 131 identities.** Twelve arms carry two
  spellings for one behavior (`isNaN`/`is_nan`, `to_string`/`toString`, and ten
  more). A spelling-keyed list would report a legacy population 14% larger than
  it is.
- **Six behaviors are reachable through two privilege scopes at once.** `sin`
  and `__intrinsic_sin` both select `BuiltinFunction::Sin`, but the first
  resolves at surface scope and the second at `InternalIntrinsic` scope, gated
  by `allow_internal_builtins`. The same is true of `cos`, `tan`, `asin`,
  `acos`, and `atan`. That fact is invisible in a spelling census and is exactly
  the kind of split authority ADR-011 §1 exists to eliminate.

**It is a stand-in, and the manifest says so in its own fields.** ADR-011 §1
requires identity "issued by the semantic database, never fabricated by a
backend", and no semantic database exists at this revision: `IntrinsicId`,
`IntrinsicCatalog`, `DefinitionId`, and `SemanticIdentity` appear nowhere in
`crates/`, `bin/`, `tools/`, or `extensions/`. `BuiltinFunction` is the current
de-facto catalog of builtin behaviors. #92 introduces the real catalog seam and
#177 freezes the catalog program; because each manifest entry is already exactly
one behavior, the mapping to a catalog identity refines — it never merges two
entries or splits one across a spelling boundary.

## What the manifest contains

- **131 identity entries**, each with its legacy spellings and whether the
  behavior is reachable from surface scope, internal-only scope, or both.
- **4 mechanism entries** for legacy authority that is not an enumerable
  identity population: the `__native_`/`__intrinsic_`/`__json_` prefix gate, the
  `allow_internal_builtins` privilege flag, `stdlib_function_names` membership,
  and the module-builtin export route. Each pins its exact current sites and
  names the ticket that deletes it.

Entry kinds are labeled, so nobody can later read the mechanism entries as
identities or the identity count as the whole legacy surface.

## The default rule, and how it is actually enforced

An identity absent from the manifest receives no legacy privilege and resolves
through the ordinary typed pipeline. It is never "legacy unless opted in."

Today that default is already structurally true: `classify_builtin_function`
ends in `_ => return None`, so a name matching no arm gains no builtin
privilege. What was missing is that a future edit could add an arm and nobody
would notice. `scripts/check-adr011-012-legacy-identity-manifest.mjs` closes
that: a new privileged identity, a new spelling on a listed identity, a widening
of a behavior's reachable scope, or a legacy mechanism spreading to a new file
all fail the build until the change is listed in the same commit. Removal is
progress and is reported, not failed.

Four failure paths were exercised against the real tree and each produced its
expected exit code: an unlisted arm (exit 1), a new spelling on
`BuiltinFunction::Reflect` (exit 1, and it separately reported the scope
widening), a mechanism spreading to a new file (exit 1), and a hand-edited
manifest (exit 2).

**The manifest is not consulted by the compiler at run time and must not
become one.** A JSON-driven resolution table would be precisely the adapter R14
forbids — an old path wearing a migrated face. This is an enumeration plus a
guard; #92's catalog seam is the routing change.

## Two things step 5 asks for that were not delivered

Both are reported rather than approximated, because the approximation in each
case would be the forbidden adapter.

### 1. R14 finiteness is not satisfiable for the module-builtin route

`resolve_scoped_module_builtin_function`
(`crates/shape-vm/src/compiler/expressions/function_calls.rs:1216`) is consulted
**before** the name table and privileges `source_module_path::export_name`. Its
membership test `is_native_module_export` (`:1601`) reads the runtime
`extension_registry`. The privileged population therefore depends on which
extension modules are registered in a given compilation, not on anything
committed to the repository: any extension that registers an export gains
module-builtin privilege for that name.

The identity *key* for this route is already the forward-stable one ADR-011
wants — declaration site. The *population* is open. R14's "explicit finite
legacy set" cannot be satisfied for it by enumeration, and writing a snapshot of
one machine's registered extensions into a manifest would be a false claim of
finiteness. The manifest pins the mechanism and its exact sites instead, and
records the finding in `open_population_finding`.

Closing it is a decision, not a census: either extension exports stop conferring
implicit builtin privilege, or the catalog admits them explicitly. That belongs
to #92 with #105.

### 2. "Reject old untyped values at the new boundary" has no boundary yet

Step 5(c) requires rejecting old untyped values at the new boundary with a
structured diagnostic. At this revision there is no new boundary: no catalog, no
resolved-identity type, and no site where a typed intrinsic identity meets an
untyped legacy value. #92 ("resolve one live intrinsic by catalog identity")
builds the first one.

Inventing that boundary now would mean constructing a typed-looking façade in
front of the existing name table and rejecting values at it — a translation
layer between the old authority and a new one that does not exist. That is the
bridge R14 and the CLAUDE.md forbidden-patterns section prohibit, and it would
also make the old path look migrated. The rejection diagnostic is therefore
owned by #92, which should assign it a code at the moment the boundary is real.

The existing internal-intrinsic rejection (`functions.rs:3837`,
"'X' resolves to internal intrinsic scope") is the *old* boundary's diagnostic
and carries no structured code. Giving it one is a small, real improvement, but
it is not step 5(c) and was deliberately not done under that label.
