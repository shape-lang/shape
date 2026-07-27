// ADR-011..016 step-5 — the finite legacy identity manifest, extraction rules.
//
// Authority: rulings file, "#90 authority enactment — ten required steps",
// step 5; ruling R14; ticket #136 (MIGRATION-GUARD) and the supervisor caution
// on it (2026-07-27).
//
// R14 requires a legacy manifest "keyed by resolved semantic identity, never
// spelling". The #133/#134/#135 baselines are spelling-keyed censuses — the
// right instrument for step 4's mechanical inventory, and explicitly NOT this.
// This module performs the spelling -> identity translation for the population
// that admits one, and records the population that does not as a mechanism
// with its exact current authority.
//
// KEY SCHEME
//
// `BuiltinFunction::<Variant>` is the identity key for name-selected builtins.
// It is not a Shape source spelling: it names the BEHAVIOR, lives in the
// compiler's own namespace, is unreachable from user code, and survives any
// rename of the surface name. The translation does observable work — 149 live
// spellings collapse to 131 identities, and six behaviors turn out to be
// reachable through two different privilege scopes at once (`sin` and
// `__intrinsic_sin` both select `BuiltinFunction::Sin`). A spelling-keyed list
// cannot express either fact.
//
// It is a stand-in, and the manifest says so in its own fields: ADR-011 §1
// requires identity "issued by the semantic database", and no semantic
// database exists yet — `IntrinsicId`, `IntrinsicCatalog`, `DefinitionId` and
// `SemanticIdentity` are absent from the tree at this revision. #92 introduces
// the catalog seam and #177 freezes the catalog program; each entry here maps
// forward to exactly one catalog identity, because each is already exactly one
// behavior.

export const CLASSIFIER_FILE = "crates/shape-vm/src/compiler/helpers.rs";
export const CLASSIFIER_FN = "classify_builtin_function";

// The single name-selection site. Everything between this function's opening
// and `is_internal_intrinsic_name` is the privileged population for the
// name-table route: a name that matches no arm falls through to `return None`
// and receives no builtin privilege at all, which is why this route — unlike
// the module-builtin route below — is genuinely finite.
const ARM_PATTERN = /^\s*((?:"[^"]+"\s*\|\s*)*"[^"]+")\s*=>\s*BuiltinFunction::(\w+)\s*,/gm;

export function extractNameSelectedIdentities(classifierSource) {
  const start = classifierSource.indexOf(`pub(super) fn ${CLASSIFIER_FN}`);
  if (start < 0) throw new Error(`${CLASSIFIER_FN} not found in ${CLASSIFIER_FILE}`);
  const end = classifierSource.indexOf("pub(super) fn is_internal_intrinsic_name", start);
  if (end < 0) throw new Error("is_internal_intrinsic_name terminator not found; the classifier shape changed");
  const body = classifierSource.slice(start, end);

  const byIdentity = new Map();
  let arms = 0;
  ARM_PATTERN.lastIndex = 0;
  for (let match = ARM_PATTERN.exec(body); match !== null; match = ARM_PATTERN.exec(body)) {
    arms += 1;
    const spellings = [...match[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    const variant = match[2];
    const key = `BuiltinFunction::${variant}`;
    const entry = byIdentity.get(key) ?? { identity: key, variant, spellings: [], arms: 0 };
    entry.spellings.push(...spellings);
    entry.arms += 1;
    byIdentity.set(key, entry);
  }

  const identities = [...byIdentity.values()].map((entry) => {
    const spellings = [...entry.spellings].sort();
    const internal = spellings.filter((s) => /^__(native|intrinsic|json)_/.test(s));
    return {
      identity: entry.identity,
      legacy_spellings: spellings,
      // Derived from the classifier's own scope match, which routes a
      // __-prefixed spelling to InternalIntrinsic and everything else to a
      // surface scope. A behavior with both is reachable through two privilege
      // levels at once.
      reachable_internal_only: internal.length > 0,
      reachable_from_surface: spellings.length > internal.length,
      arms: entry.arms,
    };
  });
  identities.sort((a, b) => (a.identity < b.identity ? -1 : a.identity > b.identity ? 1 : 0));

  return {
    identities,
    arm_count: arms,
    spelling_count: identities.reduce((total, entry) => total + entry.legacy_spellings.length, 0),
  };
}

// Legacy authority that is NOT an enumerable identity population. Each entry
// pins its exact current sites so the guard fails when the mechanism spreads.
// `sites` are counted with the same ripgrep idiom as the step-4 baselines.
export const MECHANISM_ENTRIES = [
  {
    id: "internal-intrinsic-name-prefix-gate",
    authority: "`__native_` / `__intrinsic_` / `__json_` name prefix selects InternalIntrinsic resolution scope",
    finite: true,
    why_not_identity_keyed:
      "A prefix is a spelling rule, not an identity. It is finite only because it can fire exclusively on names that already matched a classifier arm — the identities it can reach are therefore a subset of the name-selected manifest above.",
    scope: ["crates", "bin", "tools", "extensions"],
    pattern: "is_internal_intrinsic_name",
    successor: "#92 (resolve one live intrinsic by catalog identity), #177 (INTRINSIC-INVENTORY)",
  },
  {
    id: "allow-internal-builtins-privilege-flag",
    authority: "`allow_internal_builtins` compiler flag lifts the InternalOnly rejection wholesale",
    finite: true,
    why_not_identity_keyed:
      "A single boolean that re-privileges every internal identity at once. ADR-011 names it, with the terminal-name gates, as migration debt.",
    scope: ["crates", "bin", "tools", "extensions"],
    pattern: "allow_internal_builtins",
    successor: "#92, #177",
  },
  {
    id: "stdlib-function-name-membership",
    authority: "`stdlib_function_names` string-set membership decides annotation-planner and pseudo-pack behavior",
    finite: true,
    why_not_identity_keyed:
      "Authority is a raw `.contains(name)` over a set populated per-compilation from whatever the stdlib declares; there is no resolution behind the name to key on. Both read sites are in machinery slated for deletion, one of them in pseudo_tuple.rs.",
    scope: ["crates", "bin", "tools", "extensions"],
    pattern: "stdlib_function_names\\s*\\.\\s*contains",
    successor: "#110 (delete universal, string-backed, and duplicate tooling authority)",
  },
  {
    id: "module-builtin-export-route",
    authority:
      "`resolve_scoped_module_builtin_function` privileges `source_module_path::export_name`, resolved against the runtime `extension_registry` before the name table is consulted",
    finite: false,
    why_not_identity_keyed:
      "The identity key IS forward-stable — declaration site (source module path + export name) is exactly what ADR-011 asks for. The POPULATION is not enumerable at build time: `is_native_module_export` consults a runtime extension registry, so any extension that registers an export gains this privilege for that name. See the manifest's open_population_finding.",
    scope: ["crates", "bin", "tools", "extensions"],
    pattern: "resolve_scoped_module_builtin_function|is_native_module_export",
    successor: "#92, #105 (close the target matrix and delete parallel routing)",
  },
];
