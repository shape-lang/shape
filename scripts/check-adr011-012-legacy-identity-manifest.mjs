#!/usr/bin/env node
//
// ADR-011..016 step-5 — the identity-default guard (#136).
//
// Exit 0 = no new legacy privilege. Exit 1 = a new privileged identity,
// spelling, or mechanism site appeared without a manifest entry. Exit 2 = the
// manifest is unusable (missing, hand-edited, or the classifier shape changed).
//
// This is the mechanical half of R14's "an identity not listed there ...
// defaults to the new semantic pipeline. It is never 'legacy unless opted in.'"
// Adding a name-selected builtin now requires listing it, which is a reviewable
// diff, rather than one more arm nobody notices.
//
// Removal is progress and is reported, not failed — regenerate to tighten.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";
import {
  CLASSIFIER_FILE,
  MECHANISM_ENTRIES,
  extractNameSelectedIdentities,
} from "./lib/adr011-012-legacy-identity.mjs";
import { scanSet } from "./lib/adr011-012-legacy-scan.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const manifestPath = path.join(repositoryRoot, "docs/program/adr011-012/legacy-identity-manifest.json");

if (!fs.existsSync(manifestPath)) {
  console.error("FATAL  #136: docs/program/adr011-012/legacy-identity-manifest.json is missing.");
  console.error("       Generate it with: node scripts/generate-adr011-012-legacy-identity-manifest.mjs");
  process.exit(2);
}

const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
let growth = 0;
let progress = 0;

let extracted;
try {
  extracted = extractNameSelectedIdentities(fs.readFileSync(path.join(repositoryRoot, CLASSIFIER_FILE), "utf8"));
} catch (error) {
  console.error(`FATAL  #136: ${error.message}`);
  console.error("       The name-selection site moved or changed shape. Re-read it, then regenerate the manifest so");
  console.error("       the enumeration and the guard describe the same code.");
  process.exit(2);
}

const mechanisms = MECHANISM_ENTRIES.map((entry) => {
  const scanned = scanSet(repositoryRoot, { id: entry.id, scope: entry.scope, pattern: entry.pattern });
  return { id: entry.id, pattern: entry.pattern, scope: entry.scope, site_count: scanned.count, sites: scanned.owners };
});

// Reject a hand-edited manifest before comparing anything against it.
const canonical = JSON.stringify({
  identities: manifest.identity_entries.map((entry) => ({
    identity: entry.identity,
    legacy_spellings: entry.legacy_spellings,
    reachable_internal_only: entry.reachable_internal_only,
    reachable_from_surface: entry.reachable_from_surface,
  })),
  mechanisms: manifest.mechanism_entries.map((entry) => ({
    id: entry.id,
    pattern: entry.pattern,
    scope: entry.scope,
    site_count: entry.site_count,
    sites: entry.sites,
  })),
});
const recomputed = crypto.createHash("sha256").update(canonical).digest("hex");
if (recomputed !== manifest.manifest_sha256) {
  console.error(
    `FATAL  #136: manifest_sha256 does not match its own rows (recorded ${manifest.manifest_sha256}, recomputed ${recomputed}). The manifest was edited by hand; regenerate it.`,
  );
  process.exit(2);
}

const listed = new Map(manifest.identity_entries.map((entry) => [entry.identity, entry]));

for (const actual of extracted.identities) {
  const expected = listed.get(actual.identity);
  if (!expected) {
    console.error(`FAIL   #136: UNLISTED privileged identity ${actual.identity} (spellings: ${actual.legacy_spellings.join(", ")})`);
    console.error("         A new name-selected builtin gains legacy privilege only by being listed in the manifest.");
    console.error("         Default for anything unlisted is the resolved typed pipeline — see R14 and the manifest's default_rule.");
    growth += 1;
    continue;
  }
  const added = actual.legacy_spellings.filter((s) => !expected.legacy_spellings.includes(s));
  if (added.length > 0) {
    console.error(`FAIL   #136: ${actual.identity} gained ${added.length} legacy spelling(s): ${added.join(", ")}`);
    console.error("         A new spelling is new legacy surface even when the behavior is already listed.");
    growth += 1;
  }
  if (!expected.reachable_internal_only && actual.reachable_internal_only) {
    console.error(`FAIL   #136: ${actual.identity} became reachable through an internal-only spelling; privilege widened.`);
    growth += 1;
  }
  if (!expected.reachable_from_surface && actual.reachable_from_surface) {
    console.error(`FAIL   #136: ${actual.identity} became reachable from surface scope; privilege widened.`);
    growth += 1;
  }
}

for (const expected of manifest.identity_entries) {
  const actual = extracted.identities.find((entry) => entry.identity === expected.identity);
  if (!actual) {
    console.log(`OK     #136: ${expected.identity} no longer holds name-selected privilege (progress — regenerate to tighten)`);
    progress += 1;
    continue;
  }
  const removed = expected.legacy_spellings.filter((s) => !actual.legacy_spellings.includes(s));
  if (removed.length > 0) {
    console.log(`OK     #136: ${expected.identity} retired ${removed.length} spelling(s): ${removed.join(", ")} (progress — regenerate)`);
    progress += 1;
  }
}

const listedMechanisms = new Map(manifest.mechanism_entries.map((entry) => [entry.id, entry]));
for (const actual of mechanisms) {
  const expected = listedMechanisms.get(actual.id);
  if (!expected) {
    console.error(`FATAL  #136: mechanism '${actual.id}' is defined but absent from the manifest; regenerate.`);
    process.exit(2);
  }
  if (actual.site_count > expected.site_count) {
    console.error(
      `FAIL   #136: mechanism '${actual.id}' spread: ${expected.site_count} -> ${actual.site_count} sites. Legacy authority may not widen.`,
    );
    growth += 1;
  } else if (actual.site_count < expected.site_count) {
    console.log(`OK     #136: mechanism '${actual.id}' shrank ${expected.site_count} -> ${actual.site_count} (progress — regenerate)`);
    progress += 1;
  }
  const expectedSites = new Set(expected.sites.map((site) => site.path));
  const newSites = actual.sites.filter((site) => !expectedSites.has(site.path));
  if (newSites.length > 0) {
    console.error(`FAIL   #136: mechanism '${actual.id}' reached ${newSites.length} new file(s):`);
    for (const site of newSites) console.error(`         + ${site.path} (${site.count})`);
    growth += 1;
  }
}

if (growth > 0) {
  console.error(`\n#136 legacy identity manifest FAILED: ${growth} widening(s) of legacy authority.`);
  console.error("Route the new surface through the resolved typed pipeline, or, if the privilege is genuinely required,");
  console.error("add it to the manifest in the same commit so the grant is reviewable.");
  process.exit(1);
}
console.log(
  `#136 legacy identity manifest OK: ${extracted.identities.length} identities / ${extracted.spelling_count} spellings, ${mechanisms.length} mechanisms — no legacy authority widened${progress > 0 ? `, ${progress} narrowing(s) pending regeneration` : ""}.`,
);
