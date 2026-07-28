#!/usr/bin/env node
//
// JIT whole-program-bail baseline — the growth gate (#187, ADR-018 §2).
//
//   node scripts/check-jit-bail-baseline.mjs
//   node scripts/check-jit-bail-baseline.mjs --self-test
//
// Exit 0 = no bail was added. Exit 1 = growth (build must fail). Exit 2 = the
// baseline is unusable (missing, hand-edited, or the scanner cannot read the
// source it is meant to gate).
//
// ADR-018 §2: "The whole-program bail set is a generated shrink-only baseline
// ratcheted to zero; a new whole-program bail cannot be added." This gate is
// the mechanical half of that sentence. Four failure shapes, all growth:
//
//   1. a new whole-program bail site id;
//   2. a new `Program`-scoped residual — a construct that used to cost only
//      its enclosing function now costs the program;
//   3. an unmarked whole-program refusal — a bail added without declaring
//      itself, which is how a ratchet gets evaded rather than raised;
//   4. a site whose recorded reason changed, which is a marker retargeted at a
//      different construct while keeping the count flat.
//
// A decrease is progress: it is reported, not failed, with the regenerate
// command. Leaving a loose bound is how a ratchet stops ratcheting.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { hashInventory, scanWholeProgramBails } from "./lib/jit-whole-program-bails.mjs";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const BASELINE = "docs/program/adr011-012/baselines/jit-whole-program-bail-inventory.json";

function compare(committed, actual) {
  const failures = [];
  const progress = [];

  const committedSites = new Map(committed.sites.map((site) => [site.id, site]));
  const actualSites = new Map(actual.sites.map((site) => [site.id, site]));

  for (const [id, site] of actualSites) {
    const baselineSite = committedSites.get(id);
    if (!baselineSite) {
      failures.push(
        `GROWTH  new whole-program bail site '${id}' [${site.category}] (${site.file}) is not in ` +
          `the baseline. ADR-018 §2 forbids adding one: refuse the construct per-function instead.`,
      );
      continue;
    }
    if (baselineSite.category !== site.category) {
      failures.push(
        `DRIFT   bail site '${id}' changed category from '${baselineSite.category}' to ` +
          `'${site.category}'. Reclassifying a construct refusal as infrastructure removes it ` +
          `from the zero target without fixing anything.`,
      );
    }
    if (baselineSite.reason !== site.reason) {
      failures.push(
        `DRIFT   bail site '${id}' changed its recorded reason. The baseline says ` +
          `"${baselineSite.reason}"; the source says "${site.reason}". Regenerate only if the ` +
          `construct genuinely changed — a retargeted marker keeps the count flat while the ` +
          `refusal covers something else.`,
      );
    }
  }
  for (const id of committedSites.keys()) {
    if (!actualSites.has(id)) {
      progress.push(`removed bail site '${id}'`);
    }
  }

  const committedResiduals = new Set(committed.program_scoped_residuals.map((r) => r.id));
  const actualResiduals = new Set(actual.programScopedResiduals.map((r) => r.id));
  for (const id of actualResiduals) {
    if (!committedResiduals.has(id)) {
      failures.push(
        `GROWTH  residual '${id}' is now Program-scoped but is not in the baseline. ` +
          `Widening a residual from Owner to Program scope costs every program its native ` +
          `execution — the direction ADR-018 §2 ratchets against.`,
      );
    }
  }
  for (const id of committedResiduals) {
    if (!actualResiduals.has(id)) {
      progress.push(`residual '${id}' narrowed from Program scope to Owner scope`);
    }
  }

  for (const row of actual.unmarked) {
    failures.push(
      `UNMARKED whole-program refusal at ${row.file}:${row.line} in \`${row.owner}\` carries no ` +
        `\`// WHOLE-PROGRAM-BAIL: <id> — <reason>\` marker, so it is invisible to this gate. ` +
        `Declare it (and it will then fail as growth) or make the refusal per-function.`,
    );
  }

  return { failures, progress };
}

function selfTest(committed, actual) {
  // Each forced negative must fail, and its unmutated control must pass —
  // a tripwire that cannot fire is worse than no tripwire.
  const control = compare(committed, actual);
  if (control.failures.length > 0) {
    console.error("SELF-TEST FATAL: the unmutated control does not pass:");
    for (const failure of control.failures) console.error(`  ${failure}`);
    return false;
  }

  const negatives = [
    [
      "a new bail site fails",
      () => ({
        ...actual,
        sites: [
          ...actual.sites,
          { id: "fabricated-bail", category: "construct", reason: "invented", file: "x.rs" },
        ],
      }),
    ],
    [
      "reclassifying a construct bail as infra fails",
      () => ({
        ...actual,
        sites: actual.sites.map((site) =>
          site.category === "construct" ? { ...site, category: "infra" } : site,
        ),
      }),
    ],
    [
      "a retargeted marker fails",
      () => ({
        ...actual,
        sites: actual.sites.map((site, index) =>
          index === 0 ? { ...site, reason: "something else entirely" } : site,
        ),
      }),
    ],
    [
      "a newly Program-scoped residual fails",
      () => ({
        ...actual,
        programScopedResiduals: [
          ...actual.programScopedResiduals,
          { id: "fabricated-residual", variant: "Fabricated" },
        ],
      }),
    ],
    [
      "an unmarked refusal fails",
      () => ({
        ...actual,
        unmarked: [{ file: "x.rs", line: 1, owner: "f", text: "return Err(...)" }],
      }),
    ],
  ];

  let ok = true;
  for (const [name, mutate] of negatives) {
    const result = compare(committed, mutate());
    if (result.failures.length === 0) {
      console.error(`SELF-TEST FAILED: ${name} — the tripwire did not fire.`);
      ok = false;
    } else {
      console.log(`  self-test ok: ${name}`);
    }
  }
  return ok;
}

const wantSelfTest = process.argv.includes("--self-test");

const baselinePath = path.join(repositoryRoot, BASELINE);
if (!fs.existsSync(baselinePath)) {
  console.error(`FATAL: ${BASELINE} is missing.`);
  console.error(`       Generate it with: node scripts/generate-jit-bail-baseline.mjs`);
  process.exit(2);
}

const committed = JSON.parse(fs.readFileSync(baselinePath, "utf8"));

let actual;
try {
  actual = scanWholeProgramBails(repositoryRoot);
} catch (error) {
  console.error(`FATAL: ${error.message}`);
  process.exit(2);
}

// The committed file must match its own contents. A baseline edited by hand to
// admit a new bail is a gate failure, not a silent pass.
const recomputed = hashInventory({
  sites: committed.sites,
  programScopedResiduals: committed.program_scoped_residuals,
});
if (recomputed !== committed.inventory_sha256) {
  console.error(
    `FATAL: ${BASELINE} inventory_sha256 does not match its own rows ` +
      `(recorded ${committed.inventory_sha256}, recomputed ${recomputed}). ` +
      `The file was edited by hand; regenerate it.`,
  );
  process.exit(2);
}

if (wantSelfTest && !selfTest(committed, actual)) {
  process.exit(2);
}

const { failures, progress } = compare(committed, actual);

const constructCount = actual.sites.filter((site) => site.category === "construct").length;
const infraCount = actual.sites.filter((site) => site.category === "infra").length;
console.log(
  `whole-program JIT bails: ${constructCount} construct site(s) ` +
    `(baseline ${committed.construct_site_count}, target zero), ` +
    `${infraCount} infra site(s) (baseline ${committed.infra_site_count}), ` +
    `${actual.programScopedResiduals.length} Program-scoped residual(s) ` +
    `(baseline ${committed.program_scoped_residual_count}).`,
);

for (const line of progress) {
  console.log(`PROGRESS ${line}`);
}
if (progress.length > 0) {
  console.log(
    `         The ratchet is now loose. Tighten it: node scripts/generate-jit-bail-baseline.mjs`,
  );
}

for (const failure of failures) {
  console.error(failure);
}

process.exit(failures.length > 0 ? 1 : 0);
