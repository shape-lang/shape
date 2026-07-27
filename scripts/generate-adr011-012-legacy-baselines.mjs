#!/usr/bin/env node
//
// ADR-011..016 step-4 migration baselines — regenerator.
//
// Usage:
//   node scripts/generate-adr011-012-legacy-baselines.mjs               # all three
//   node scripts/generate-adr011-012-legacy-baselines.mjs --ticket 133  # one
//   node scripts/generate-adr011-012-legacy-baselines.mjs --stdout      # print, write nothing
//
// Regenerating is legitimate and expected: it is how a slice records migration
// progress. It is never silent — the baseline diff shows exactly which owners
// and counts moved, and the growth check refuses a rise regardless of who
// regenerated. See tickets #133, #134, #135 and ruling R14.

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { BASELINES, baselineForTicket } from "./lib/adr011-012-legacy-sets.mjs";
import { scanBaseline, buildBaselineDocument } from "./lib/adr011-012-legacy-scan.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");

function parseArguments(argv) {
  let ticket = null;
  let stdout = false;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--ticket") {
      ticket = argv[index + 1];
      index += 1;
    } else if (argument === "--stdout") {
      stdout = true;
    } else {
      console.error(`unknown argument: ${argument}`);
      process.exit(2);
    }
  }
  return { ticket, stdout };
}

// The revision the inventory was taken at. Step 4 requires the baseline to be
// revision-bound; HEAD here is the parent of the commit that will carry the
// file, so the record never names its own commit.
function sourceRevision() {
  try {
    return execFileSync("git", ["rev-parse", "HEAD"], {
      cwd: repositoryRoot,
      encoding: "utf8",
    }).trim();
  } catch {
    return "unknown";
  }
}

const { ticket, stdout: toStdout } = parseArguments(process.argv.slice(2));
const selected = ticket ? [baselineForTicket(ticket)] : BASELINES;
if (selected.some((baseline) => !baseline)) {
  console.error(`no baseline for ticket ${ticket}; known tickets: ${BASELINES.map((b) => b.ticket).join(", ")}`);
  process.exit(2);
}

const revision = sourceRevision();
for (const baseline of selected) {
  const sets = scanBaseline(repositoryRoot, baseline);
  const document = buildBaselineDocument(baseline, sets, revision);
  const serialized = `${JSON.stringify(document, null, 2)}\n`;
  if (toStdout) {
    process.stdout.write(serialized);
    continue;
  }
  const target = path.join(repositoryRoot, baseline.file);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, serialized);
  const owners = sets.reduce((total, set) => total + set.owner_count, 0);
  console.log(
    `#${baseline.ticket} ${baseline.id}: ${sets.length} sets, ${document.total_count} occurrences, ${owners} owners, sets_sha256=${document.sets_sha256}`,
  );
  console.log(`  -> ${baseline.file}`);
}
