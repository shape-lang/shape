#!/usr/bin/env node
//
// ADR-011..016 step-5 — regenerate the finite legacy identity manifest (#136).
//
// Usage:
//   node scripts/generate-adr011-012-legacy-identity-manifest.mjs
//   node scripts/generate-adr011-012-legacy-identity-manifest.mjs --stdout
//
// See scripts/lib/adr011-012-legacy-identity.mjs for the key scheme and
// docs/program/adr011-012/legacy-identity-manifest.md for what this manifest
// does and does not claim.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  CLASSIFIER_FILE,
  CLASSIFIER_FN,
  MECHANISM_ENTRIES,
  extractNameSelectedIdentities,
} from "./lib/adr011-012-legacy-identity.mjs";
import { scanSet } from "./lib/adr011-012-legacy-scan.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const manifestPath = "docs/program/adr011-012/legacy-identity-manifest.json";

function sourceRevision() {
  try {
    return execFileSync("git", ["rev-parse", "HEAD"], { cwd: repositoryRoot, encoding: "utf8" }).trim();
  } catch {
    return "unknown";
  }
}

const classifierSource = fs.readFileSync(path.join(repositoryRoot, CLASSIFIER_FILE), "utf8");
const extracted = extractNameSelectedIdentities(classifierSource);

const mechanisms = MECHANISM_ENTRIES.map((entry) => {
  const scanned = scanSet(repositoryRoot, { id: entry.id, scope: entry.scope, pattern: entry.pattern });
  return {
    id: entry.id,
    authority: entry.authority,
    population_finite: entry.finite,
    why_not_identity_keyed: entry.why_not_identity_keyed,
    successor: entry.successor,
    pattern: entry.pattern,
    scope: entry.scope,
    site_count: scanned.count,
    sites: scanned.owners,
  };
});

// Hash covers what a later slice may change: the identity set with its
// spellings, and each mechanism's pinned site counts. Prose fields are
// excluded so that clarifying a description never looks like drift.
const canonical = JSON.stringify({
  identities: extracted.identities.map((entry) => ({
    identity: entry.identity,
    legacy_spellings: entry.legacy_spellings,
    reachable_internal_only: entry.reachable_internal_only,
    reachable_from_surface: entry.reachable_from_surface,
  })),
  mechanisms: mechanisms.map((entry) => ({
    id: entry.id,
    pattern: entry.pattern,
    scope: entry.scope,
    site_count: entry.site_count,
    sites: entry.sites,
  })),
});

const document = {
  record_kind: "LegacyIdentityManifest",
  record_version: 1,
  program_id: "adr011-012",
  ticket: 136,
  entry_id: "MIGRATION-GUARD",
  manifest_ordinal: 26,
  step: "docs/design/typed-comptime/adr011-012-execution-rulings.md — ten required steps, step 5",
  authority: { adrs: ["ADR-011", "ADR-012", "ADR-013"], rulings: ["R14", "R20"] },
  source_revision: sourceRevision(),
  regenerate_with: "node scripts/generate-adr011-012-legacy-identity-manifest.mjs",
  check_with: "node scripts/check-adr011-012-legacy-identity-manifest.mjs",

  key_scheme: {
    identity_key: "BuiltinFunction::<Variant>",
    rationale:
      "Names the behavior, not the spelling: it lives in the compiler's own namespace, is unreachable from Shape source, survives any rename of a surface name, and collapses multi-spelling arms to one entry. A same-spelled user declaration cannot collide with it.",
    stand_in_for: "the ADR-011 §1 resolved definition identity issued by the semantic database",
    stand_in_because:
      "No semantic database exists at this revision: IntrinsicId, IntrinsicCatalog, DefinitionId and SemanticIdentity are absent from crates/, bin/, tools/ and extensions/. #92 introduces the catalog seam; #177 freezes the catalog program.",
    forward_mapping:
      "Each entry is exactly one behavior, so it maps to exactly one catalog identity when #92 lands. The mapping refines; it never merges two entries or splits one across a spelling boundary.",
    not_spelling_keyed_evidence: `${extracted.spelling_count} live spellings collapse to ${extracted.identities.length} identities; six behaviors are reachable through both a surface and an internal spelling at once.`,
  },

  default_rule: {
    statement:
      "An identity absent from this manifest receives NO legacy privilege and resolves through the ordinary typed pipeline. It is never 'legacy unless opted in.'",
    how_enforced_today:
      "Structurally, by the classifier itself: a name matching no arm hits `_ => return None` and gains no builtin privilege. Mechanically, by check-adr011-012-legacy-identity-manifest.mjs, which fails the build when a NEW privileged identity or spelling appears without a manifest entry.",
    not_claimed:
      "This manifest is not consulted by the compiler at run time, and must not become a JSON-driven resolution table — that would be the adapter R14 forbids. It is an enumeration plus a guard; the catalog seam (#92) is the routing change.",
  },

  counts: {
    identities: extracted.identities.length,
    classifier_arms: extracted.arm_count,
    live_spellings: extracted.spelling_count,
    identities_reachable_both_internally_and_from_surface: extracted.identities.filter(
      (entry) => entry.reachable_internal_only && entry.reachable_from_surface,
    ).length,
    mechanisms: mechanisms.length,
    mechanisms_with_open_population: mechanisms.filter((entry) => !entry.population_finite).length,
  },

  open_population_finding: {
    summary:
      "R14's finiteness requirement is NOT satisfiable today for the module-builtin export route, and no amount of enumeration here can fix it.",
    detail:
      "`resolve_scoped_module_builtin_function` (crates/shape-vm/src/compiler/expressions/function_calls.rs) is consulted BEFORE the name table and privileges `source_module_path::export_name`. Its membership test `is_native_module_export` reads the runtime `extension_registry`, so the privileged population depends on which extension modules are registered in a given compilation rather than on anything committed. Any extension that registers an export gains module-builtin privilege for that name.",
    consequence:
      "The identity KEY for this route is already the forward-stable one ADR-011 wants (declaration site). The POPULATION is open, so this manifest pins the mechanism and its exact sites instead of pretending to enumerate it.",
    owner: "#92 (catalog seam) and #105 (close the target matrix and delete parallel routing)",
  },

  identity_entries: extracted.identities,
  mechanism_entries: mechanisms,
  manifest_sha256: crypto.createHash("sha256").update(canonical).digest("hex"),
};

const serialized = `${JSON.stringify(document, null, 2)}\n`;
if (process.argv.includes("--stdout")) {
  process.stdout.write(serialized);
} else {
  fs.writeFileSync(path.join(repositoryRoot, manifestPath), serialized);
  console.log(
    `#136 legacy identity manifest: ${document.counts.identities} identities from ${document.counts.classifier_arms} arms / ${document.counts.live_spellings} spellings, ${document.counts.mechanisms} mechanisms (${document.counts.mechanisms_with_open_population} with an open population), manifest_sha256=${document.manifest_sha256}`,
  );
  console.log(`  -> ${manifestPath}`);
  console.log(`  source of truth: ${CLASSIFIER_FILE}::${CLASSIFIER_FN}`);
}
