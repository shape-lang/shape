#!/usr/bin/env node
//
// Generate the JIT whole-program-bail baseline (#187, ADR-018 §2).
//
//   node scripts/generate-jit-bail-baseline.mjs
//
// Run this after a bail is genuinely removed — a construct gained a sound
// per-function lowering, or a `Program`-scoped residual was narrowed to
// `Owner`. Regenerating tightens the ratchet. It is NOT a way to accept a new
// bail: the checker reports growth before you get here, and a reviewer seeing
// this file's counts RISE in a diff is looking at exactly the thing ADR-018 §2
// says must not happen.

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  BAIL_SOURCE_FILES,
  RESIDUAL_SOURCE_FILE,
  hashInventory,
  scanWholeProgramBails,
} from "./lib/jit-whole-program-bails.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const OUTPUT = "docs/program/adr011-012/baselines/jit-whole-program-bail-inventory.json";

const inventory = scanWholeProgramBails(repositoryRoot);

if (inventory.unmarked.length > 0) {
  console.error("REFUSED: unmarked whole-program refusals found; mark them before generating:");
  for (const row of inventory.unmarked) {
    console.error(`  ${row.file}:${row.line} in ${row.owner}: ${row.text}`);
  }
  process.exit(2);
}

let sourceRevision = "unknown";
try {
  sourceRevision = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: repositoryRoot,
    encoding: "utf8",
  }).trim();
} catch {
  // A generated baseline outside a git checkout still records its content hash,
  // which is the part the gate enforces.
}

const record = {
  record_kind: "JitWholeProgramBailBaseline",
  record_version: 1,
  program_id: "adr017-019",
  ticket: 187,
  entry_id: "JIT-WHOLE-PROGRAM-BAIL-INVENTORY",
  title: "Whole-program JIT bail inventory (ratchet to zero)",
  authority: {
    adrs: ["ADR-018"],
    rulings: ["R15", "R24"],
    section: "ADR-018 §2 — deopt granularity becomes per-function",
  },
  direction_rule:
    "Monotonic non-increasing. `sites` and `program_scoped_residuals` may fall (a construct gained a sound per-function lowering, or a residual was narrowed from Program scope to Owner scope) and may never rise. A new site id is growth even when the total is unchanged, because it is a new refusal costing the whole program its native execution. ADR-018 §2's zero target is the `construct` category — refusals driven by something the source program contains. `infra` sites (a Cranelift failure, a caught panic, a post-execution kind-source gap) abandon the program too, but removing them is a different problem from per-function granularity: they are inventoried and reported, and adding one is still growth.",
  scanned_files: BAIL_SOURCE_FILES,
  residual_source_file: RESIDUAL_SOURCE_FILE,
  regenerate_with: "node scripts/generate-jit-bail-baseline.mjs",
  check_with: "node scripts/check-jit-bail-baseline.mjs",
  excluded_from_hash: ["source_revision"],
  source_revision: sourceRevision,
  site_count: inventory.sites.length,
  construct_site_count: inventory.sites.filter((site) => site.category === "construct").length,
  infra_site_count: inventory.sites.filter((site) => site.category === "infra").length,
  program_scoped_residual_count: inventory.programScopedResiduals.length,
  inventory_sha256: hashInventory(inventory),
  sites: inventory.sites,
  program_scoped_residuals: inventory.programScopedResiduals,
};

fs.writeFileSync(path.join(repositoryRoot, OUTPUT), `${JSON.stringify(record, null, 2)}\n`);
console.log(
  `wrote ${OUTPUT}: ${record.construct_site_count} construct bail site(s) ` +
    `(target zero), ${record.infra_site_count} infra site(s), ` +
    `${record.program_scoped_residual_count} Program-scoped residual(s)`,
);
