#!/usr/bin/env node
//
// ADR-011..016 step-4 migration baselines — the growth gate.
//
// Usage:
//   node scripts/check-adr011-012-legacy-baselines.mjs               # all three
//   node scripts/check-adr011-012-legacy-baselines.mjs --ticket 134  # one
//
// Exit 0 = no legacy set grew. Exit 1 = growth (build must fail). Exit 2 = the
// baselines are unusable (missing file, unknown set, scanner failure).
//
// R14: "Each tracer slice moves identities out of the legacy manifest and
// lowers the generated baseline. The set may only shrink." This gate is the
// mechanical half of that sentence. It rescans the working tree with the same
// definitions that produced the committed baseline and refuses any rise.
//
// Three failure shapes, all growth:
//   1. a set's total count rose;
//   2. a set gained an owner path that the baseline does not list — a new
//      surface carrying old authority, even when the total is flat;
//   3. an existing owner's count rose while another fell — movement that hides
//      growth behind a flat total.
//
// A decrease is progress and is reported, not failed. It is a prompt to
// regenerate so the ratchet tightens; leaving a loose bound is exactly how the
// capture_as_value baseline sat at 12 while the tree had 4.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { BASELINES, baselineForTicket } from "./lib/adr011-012-legacy-sets.mjs";
import { scanBaseline, hashSets } from "./lib/adr011-012-legacy-scan.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");

function parseArguments(argv) {
  let ticket = null;
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === "--ticket") {
      ticket = argv[index + 1];
      index += 1;
    } else {
      console.error(`unknown argument: ${argv[index]}`);
      process.exit(2);
    }
  }
  return { ticket };
}

const { ticket } = parseArguments(process.argv.slice(2));
const selected = ticket ? [baselineForTicket(ticket)] : BASELINES;
if (selected.some((baseline) => !baseline)) {
  console.error(`no baseline for ticket ${ticket}; known tickets: ${BASELINES.map((b) => b.ticket).join(", ")}`);
  process.exit(2);
}

let growth = 0;
let progress = 0;
let fatal = 0;

for (const baseline of selected) {
  const baselinePath = path.join(repositoryRoot, baseline.file);
  if (!fs.existsSync(baselinePath)) {
    console.error(`FATAL  #${baseline.ticket}: baseline ${baseline.file} is missing.`);
    console.error(`       Generate it with: node scripts/generate-adr011-012-legacy-baselines.mjs --ticket ${baseline.ticket}`);
    fatal += 1;
    continue;
  }

  const committed = JSON.parse(fs.readFileSync(baselinePath, "utf8"));
  const committedSets = new Map(committed.sets.map((set) => [set.id, set]));
  const actualSets = scanBaseline(repositoryRoot, baseline);

  // The committed file must match the definitions that are about to be
  // enforced. A baseline hand-edited to raise a limit, or left behind when a
  // set definition changed, is a gate failure and not a silent pass.
  const recomputed = hashSets(committed.sets);
  if (recomputed !== committed.sets_sha256) {
    console.error(
      `FATAL  #${baseline.ticket}: ${baseline.file} sets_sha256 does not match its own rows (recorded ${committed.sets_sha256}, recomputed ${recomputed}). The file was edited by hand; regenerate it.`,
    );
    fatal += 1;
    continue;
  }
  for (const set of actualSets) {
    if (!committedSets.has(set.id)) {
      console.error(`FATAL  #${baseline.ticket}: set '${set.id}' is defined but absent from the committed baseline; regenerate.`);
      fatal += 1;
    }
  }
  for (const set of committed.sets) {
    if (!actualSets.some((actual) => actual.id === set.id)) {
      console.error(`FATAL  #${baseline.ticket}: committed set '${set.id}' has no definition; a set was renamed or dropped without regenerating.`);
      fatal += 1;
    }
    const definition = actualSets.find((actual) => actual.id === set.id);
    if (definition && definition.pattern !== set.pattern) {
      console.error(
        `FATAL  #${baseline.ticket}: set '${set.id}' pattern changed since the baseline was taken; regenerate so the recorded rows match the enforced rule.`,
      );
      fatal += 1;
    }
    // Scope and exclusions are as much a part of the counting rule as the
    // pattern. Without this, widening an exclusion could retire legacy sites
    // from view behind an unchanged-looking count.
    if (definition && JSON.stringify(definition.scope) !== JSON.stringify(set.scope)) {
      console.error(
        `FATAL  #${baseline.ticket}: set '${set.id}' scope changed since the baseline was taken; regenerate so the recorded rows match the enforced rule.`,
      );
      fatal += 1;
    }
    if (definition && JSON.stringify(definition.exclude ?? []) !== JSON.stringify(set.exclude ?? [])) {
      console.error(
        `FATAL  #${baseline.ticket}: set '${set.id}' exclusion list changed since the baseline was taken; regenerate so the recorded rows match the enforced rule, and review that diff — a widened exclusion can hide legacy sites behind an unchanged count.`,
      );
      fatal += 1;
    }
  }
  if (fatal > 0) continue;

  for (const actual of actualSets) {
    const expected = committedSets.get(actual.id);
    const expectedOwners = new Map(expected.owners.map((owner) => [owner.path, owner.count]));

    if (actual.count > expected.count) {
      console.error(
        `FAIL   #${baseline.ticket} ${actual.id}: baseline=${expected.count} actual=${actual.count} (regression: +${actual.count - expected.count})`,
      );
      growth += 1;
    }

    const newOwners = actual.owners.filter((owner) => !expectedOwners.has(owner.path));
    if (newOwners.length > 0) {
      console.error(
        `FAIL   #${baseline.ticket} ${actual.id}: ${newOwners.length} new owner${newOwners.length === 1 ? "" : "s"} carrying legacy authority:`,
      );
      for (const owner of newOwners) console.error(`         + ${owner.path} (${owner.count})`);
      growth += 1;
    }

    const risenOwners = actual.owners.filter(
      (owner) => expectedOwners.has(owner.path) && owner.count > expectedOwners.get(owner.path),
    );
    if (risenOwners.length > 0) {
      console.error(`FAIL   #${baseline.ticket} ${actual.id}: ${risenOwners.length} owner count${risenOwners.length === 1 ? "" : "s"} rose:`);
      for (const owner of risenOwners) {
        console.error(`         ~ ${owner.path} ${expectedOwners.get(owner.path)} -> ${owner.count}`);
      }
      growth += 1;
    }

    if (actual.count < expected.count) {
      console.log(
        `OK     #${baseline.ticket} ${actual.id}: baseline=${expected.count} actual=${actual.count} (progress: -${expected.count - actual.count} — regenerate to tighten)`,
      );
      progress += 1;
    }
  }
}

if (fatal > 0) {
  console.error(`\nADR-011–016 legacy baselines UNUSABLE (${fatal} fatal).`);
  process.exit(2);
}
if (growth > 0) {
  console.error(
    `\nADR-011–016 legacy baselines FAILED: ${growth} legacy set${growth === 1 ? "" : "s"} grew.`,
  );
  console.error("A legacy set may only shrink (R14). Route the new surface through the resolved typed pipeline,");
  console.error("or, if a set legitimately moved, regenerate the baseline so the change is reviewable in the diff.");
  process.exit(1);
}
const scanned = selected.map((baseline) => `#${baseline.ticket}`).join(" ");
console.log(`ADR-011–016 legacy baselines OK: ${scanned} — no set grew${progress > 0 ? `, ${progress} set(s) shrank` : ""}.`);
