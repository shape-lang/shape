// ADR-011..016 step-4 migration baselines — shared legacy-set definitions.
//
// Authority: docs/design/typed-comptime/adr011-012-execution-rulings.md, "#90
// authority enactment — ten required steps", step 4; ruling R14 ("Migration
// defaults new by identity and forbids bridges"); tickets #133, #134, #135.
//
// Step 4 requires a MECHANICAL inventory at an exact Shape revision, storing
// stable semantic owners plus generated counts and hashes, such that later
// slices may only REDUCE their assigned legacy sets. This module is the single
// definition of what is counted; the generator and the growth check both import
// it, so a baseline can never be produced by a rule the gate does not enforce.
//
// A "set" is one mechanically countable legacy-authority class:
//   - `pattern` is a PCRE2 regex counted with ripgrep, exactly as
//     scripts/check-no-dynamic.sh counts its ratchet symbols.
//   - `scope` is a list of repository-root-relative directories.
//   - the set SIZE is the total match count; the set OWNERS are the matching
//     files with their per-file counts.
//
// Owners are file paths, never line numbers: a path stays stable across
// unrelated edits, so the owner list stays reviewable, while per-file counts
// still expose movement inside a file.
//
// These definitions describe LEGACY AUTHORITY — the mechanisms ADR-011/012
// replace. They are not a defect list, and a nonzero count is not a bug. The
// gate's only claim is directional: these numbers may fall, never rise.

export const SOURCE_SCOPE = ["crates", "bin", "tools", "extensions"];
export const DOCS_SCOPE = ["docs"];

// #133 — SEMANTIC-LEGACY-INVENTORY.
// Territory: discovery producers, ambient comptime entry points and
// observations, live intrinsic selectors. (ADR-011, ADR-013; R1, R3, R14, R18,
// R20.)
const SEMANTIC_SETS = [
  {
    id: "ambient-builtin-name-selection",
    category: "live intrinsic selectors",
    description:
      "Terminal-name match arms selecting a BuiltinFunction. ADR-011 replaces name selection with a resolved IntrinsicId from the canonical catalog; every arm here is one spelling that currently carries authority.",
    scope: SOURCE_SCOPE,
    pattern: '"[A-Za-z_][A-Za-z0-9_]*"\\s*=>\\s*BuiltinFunction::',
  },
  {
    id: "internal-intrinsic-name-prefix-gates",
    category: "live intrinsic selectors",
    description:
      "Sites treating a `__native_` / `__intrinsic_` / `__json_` name prefix as authority. ADR-011 states these prefixes are presentation/hygiene, not authority.",
    scope: SOURCE_SCOPE,
    pattern: 'starts_with\\("__(native|intrinsic|json)_',
  },
  {
    id: "allow-internal-builtins-gates",
    category: "ambient comptime entry points",
    description:
      "Reads and writes of the `allow_internal_builtins` privilege flag. CLAUDE.md names this flag, with the terminal-name gates, as migration debt that must not grow.",
    scope: SOURCE_SCOPE,
    pattern: "allow_internal_builtins",
  },
  {
    id: "prelude-name-authority",
    category: "discovery producers",
    description:
      "Uses of `prepend_prelude_items` and its returned `stdlib_function_names` set — the name-based discovery authority ADR-011 migration deletes. CLAUDE.md forbids extending the set or using it to authorize new semantics.",
    scope: SOURCE_SCOPE,
    pattern: "prepend_prelude_items|stdlib_function_names",
  },
  {
    id: "declaration-discovery-producers",
    category: "discovery producers",
    description:
      "References to the current declaration-discovery producer. ADR-011 splits discovery into a bounded snapshot with one owner; this set is the surface that must collapse into it.",
    scope: SOURCE_SCOPE,
    pattern: "declaration_discovery|DeclarationDiscovery",
  },
  {
    id: "legacy-reflection-call-forms",
    category: "ambient comptime entry points",
    description:
      "Live `type_info(...)` and `implements(\"...\")` call sites — the name-selected reflection cascade and the string-typed trait query. Their deletion exists on the paused adr009/e6 branch (f58a0d85) and is NOT merged, so this surface is still live; see docs/program/adr011-012/e6-disposition.md.",
    scope: SOURCE_SCOPE,
    pattern: '\\btype_info\\s*\\(|\\bimplements\\s*\\(\\s*"',
  },
];

// #134 — ELABORATION-LEGACY-INVENTORY.
// Territory: annotation identities and routes, universal and string
// descriptors, generated-type parser consumers, annotation/backend exceptions.
// (ADR-011, ADR-012, ADR-014; R2, R4, R5, R11, R12, R14, R20.)
const ELABORATION_SETS = [
  {
    id: "universal-comptime-target",
    category: "universal descriptors",
    description:
      "References to the universal `ComptimeTarget` / `__ComptimeTarget` carrier. CLAUDE.md forbids a universal ComptimeTarget; ADR-012 replaces it with typed ArgumentPack<Sig> and Next<Sig>. #110 owns the deletion.",
    scope: SOURCE_SCOPE,
    pattern: "\\bComptimeTarget\\b",
  },
  {
    id: "string-backed-construction",
    category: "string descriptors",
    description:
      "String-backed generated-item construction and its parser consumers: `string_lit`, `render_shape_string_literal`, and the retained `parse_type_annotation_payload` string arm. R11/R14 forbid string-backed typed construction and any later parsing bridge; #106 (typed construction) unblocks the deletion.",
    scope: SOURCE_SCOPE,
    pattern: "\\bstring_lit\\b|render_shape_string_literal|parse_type_annotation_payload",
  },
  {
    id: "pseudo-pack-and-marker-substitution",
    category: "annotation routes",
    description:
      "Pseudo-pack carriers and marker substitution: `pseudo_tuple`, `substitute_*_markers`, `build_impl_shadow_call`. CLAUDE.md forbids pseudo-tuple carriers and marker-call substitution; R10 requires packs that preserve exact passing and ownership modes.",
    scope: SOURCE_SCOPE,
    pattern: "pseudo_tuple|substitute_\\w*markers?|build_impl_shadow_call",
  },
  {
    id: "hook-decision-protocol",
    category: "annotation identities",
    description:
      "References to the spelling-recognized `HookDecision` protocol. ADR-012 replaces it with ordinary typed Callable Transforms; CLAUDE.md forbids extending it. Issue #20 is superseded by #147/#97/#148/#149/#109.",
    scope: SOURCE_SCOPE,
    pattern: "HookDecision",
  },
  {
    id: "any-typed-carriers",
    category: "string descriptors",
    description:
      "`FieldType::Any` carrier sites — the untyped hook/descriptor payload ADR-012 replaces with typed per-layer state. Not every occurrence is an annotation carrier; the ratchet's claim is that the total may not grow.",
    scope: SOURCE_SCOPE,
    pattern: "FieldType::Any",
  },
  {
    id: "raw-generated-name-minting",
    category: "annotation routes",
    description:
      "Sites minting a raw `__`-prefixed generated name via `format!`. R14 lists raw generated names as an old authority class; ADR-011 requires stable structural identities instead of rendered names.",
    scope: SOURCE_SCOPE,
    pattern: 'format!\\("__',
  },
  {
    id: "annotation-lowering-exceptions",
    category: "annotation/backend exceptions",
    description:
      "Annotation-specific lowering machinery in the bytecode compiler (template specialization weave, pseudo-pack, decision protocol). ADR-012 requires Annotation Elaboration to emit ordinary annotation-free typed Core/MIR, leaving no annotation-specific lowering path.",
    scope: ["crates/shape-vm/src/compiler"],
    pattern: "HookDecision|pseudo_tuple|substitute_\\w*markers?|build_impl_shadow_call",
  },
  {
    id: "backend-annotation-recognizers",
    category: "annotation/backend exceptions",
    description:
      "Annotation-mechanism names reachable inside the VM executor and the JIT — the backend recognizers CLAUDE.md forbids. This set is already near zero and most of its remaining weight is the `no_legacy_annotation_weave` sentinel that enforces the absence; the ratchet pins it there.",
    scope: ["crates/shape-vm/src/executor", "crates/shape-jit/src"],
    pattern: "HookDecision|annotation_weave|AnnotationDescriptor|ComptimeTarget|pseudo_tuple",
  },
];

// #135 — TOOLING-EVIDENCE-INVENTORY.
// Territory: duplicate LSP semantics, stale tests, old documentation claims.
// (ADR-011, ADR-012, ADR-013, ADR-016; R14, R19, R20.)
const TOOLING_SETS = [
  {
    id: "lsp-parallel-validators",
    category: "duplicate LSP semantics",
    description:
      "Hand-written `validate_*` functions in the language server. R23 names the LSP's parallel validators as duplicate semantics that must become generated shrink-only baselines driven by the shared query surface.",
    scope: ["tools/shape-lsp/src"],
    pattern: "fn validate_\\w+",
  },
  {
    id: "lsp-message-scraping",
    category: "duplicate LSP semantics",
    description:
      "Language-server sites branching on rendered diagnostic prose (`.contains(...)` over a message). R23 requires fixes to be single-sourced at the diagnostic emitter through a structured edit field, retiring the message-scraping fix-extractors.",
    scope: ["tools/shape-lsp/src"],
    pattern: "(message|msg|text|diagnostic)\\s*\\.\\s*contains\\(",
  },
  {
    id: "ignored-tests",
    category: "stale tests",
    description:
      "Total `#[ignore]` attributes in the source trees. This is a SUPERSET of stale evidence — some ignores are classified live gaps — so the count is a growth ratchet only; per-bucket classification is owned by scripts/check-ignored-test-classification.py.",
    scope: SOURCE_SCOPE,
    pattern: "#\\[ignore",
  },
  {
    id: "tests-asserting-legacy-mechanisms",
    category: "stale tests",
    description:
      "Test-tree sites naming a mechanism ADR-011/012 deletes. Each is evidence that will either move to the replacement contract or be deleted with its mechanism; a rise means new tests were written against superseded authority.",
    scope: ["tools/shape-test/tests", "bin/shape-cli/tests"],
    pattern:
      "HookDecision|ComptimeTarget|\\bstring_lit\\b|pseudo_tuple|\\btype_info\\s*\\(|\\bimplements\\s*\\(\\s*\"",
  },
  {
    id: "legacy-mechanism-doc-claims",
    category: "old documentation claims",
    description:
      "Documentation occurrences of a superseded mechanism name. Scanned over docs/ ONLY, and deliberately unlike scripts/check-no-dynamic.sh, which excludes documentation: #135's territory is exactly old documentation claims. Enforcement prose that names a mechanism in order to forbid it is counted too — the ratchet measures direction, and the authority set is expected to shrink this as ADR-011/012 documentation lands.",
    scope: DOCS_SCOPE,
    // The generated baselines live under docs/ and quote these very names in
    // their patterns and descriptions. Counting them would make the instrument
    // measure itself: regenerating would change the number it just recorded.
    exclude: ["docs/program/adr011-012/baselines/**"],
    pattern: "HookDecision|__ComptimeTarget|\\bstring_lit\\b|pseudo-tuple|pseudo_tuple|marker substitution",
  },
];

export const BASELINES = [
  {
    ticket: 133,
    id: "SEMANTIC-LEGACY-INVENTORY",
    manifest_ordinal: 23,
    title: "Freeze legacy discovery, comptime, and intrinsic authority",
    territory:
      "discovery producers, ambient comptime entry points and observations, live intrinsic selectors",
    authority: { adrs: ["ADR-011", "ADR-013"], rulings: ["R1", "R3", "R14", "R18", "R20"] },
    file: "docs/program/adr011-012/baselines/semantic-legacy-inventory.json",
    sets: SEMANTIC_SETS,
  },
  {
    ticket: 134,
    id: "ELABORATION-LEGACY-INVENTORY",
    manifest_ordinal: 24,
    title: "Freeze legacy elaboration and carrier authority",
    territory:
      "annotation identities and routes, universal and string descriptors, generated-type parser consumers, annotation/backend exceptions",
    authority: {
      adrs: ["ADR-011", "ADR-012", "ADR-014"],
      rulings: ["R2", "R4", "R5", "R11", "R12", "R14", "R20"],
    },
    file: "docs/program/adr011-012/baselines/elaboration-legacy-inventory.json",
    sets: ELABORATION_SETS,
  },
  {
    ticket: 135,
    id: "TOOLING-EVIDENCE-INVENTORY",
    manifest_ordinal: 25,
    title: "Freeze duplicate tooling and stale evidence authority",
    territory: "duplicate LSP semantics, stale tests, old documentation claims",
    authority: {
      adrs: ["ADR-011", "ADR-012", "ADR-013", "ADR-016"],
      rulings: ["R14", "R19", "R20"],
    },
    file: "docs/program/adr011-012/baselines/tooling-evidence-inventory.json",
    sets: TOOLING_SETS,
  },
];

export function baselineForTicket(ticket) {
  return BASELINES.find((baseline) => String(baseline.ticket) === String(ticket));
}
