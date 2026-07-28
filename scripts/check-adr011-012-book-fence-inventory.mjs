#!/usr/bin/env node
//
// ADR-016 §3 / §5 / §6 / R19 — the Book fence universe gate
// (#115, BOOK-FENCE-INVENTORY).
//
// Exit 0 = the committed inventory is internally consistent, and — when a
// shape-web checkout is supplied — still matches the corpus. Exit 1 = drift.
// Exit 2 = the inputs are unusable.
//
// The gate has two halves because the two repositories are not always both
// present:
//
//   * the INTEGRITY half needs only the committed file and runs everywhere,
//     including this repository's CI. It recomputes both content hashes rather
//     than trusting the stored fields, checks the counts against the rows, and
//     refuses a row that has quietly acquired a real ADR-016 §5 classification
//     instead of the candidate the scan is entitled to record.
//
//   * the CURRENCY half needs `--shape-web <path>` and re-derives every row from
//     the corpus. It runs locally and inside BookTruthGate, which ADR-016 §7
//     already requires to have both revisions checked out.
//
// Splitting them this way keeps the CI half honest: it defends what it can
// actually see, and does not pretend a skipped re-derivation was a passing one.
//
// `--self-test` runs the forced negatives, each asserting both that its mutation
// is rejected and that the unmutated input is accepted.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import {
  buildInventory,
  canonicalJson,
  defaultShapeWeb,
  inventoryPath,
  repositoryRoot,
} from "./generate-adr011-012-book-fence-inventory.mjs";

// A fence row records where a fence is and what it is missing. ADR-016 §5's real
// classification carries a reason and a citation, or declared modes and an
// expected outcome; a scanner cannot supply either, and a row that had them
// would be asserting review that never happened.
const CLASSIFICATION_FIELDS = ["classification", "illustrative", "expectation", "evidence_role"];

function readCommitted() {
  try {
    return JSON.parse(fs.readFileSync(inventoryPath, "utf8"));
  } catch (error) {
    console.error(`FATAL  #115: cannot read ${inventoryPath}: ${error.message}`);
    process.exit(2);
  }
}

function digest(value) {
  return crypto.createHash("sha256").update(canonicalJson(value)).digest("hex");
}

export function checkIntegrity(committed) {
  const errors = [];
  const fences = committed.fences ?? [];
  const sections = committed.sections ?? [];

  // Recomputed, never trusted: a stored hash that is not recomputed is a claim.
  for (const [field, rows] of [["sections_sha256", sections], ["fences_sha256", fences]]) {
    const actual = digest(rows);
    if (committed[field] !== actual) {
      errors.push(`${field} records ${committed[field]?.slice(0, 16)} but the committed rows hash to ${actual.slice(0, 16)} — the rows were edited without regenerating`);
    }
  }

  const counts = committed.counts ?? {};
  const expectations = {
    sections: sections.length,
    fences_all_languages: fences.length,
    shape_fences: fences.filter((fence) => fence.language === "shape").length,
    shape_runnable_gated_candidates: fences.filter((fence) => fence.classification_candidate === "runnable-gated").length,
    shape_illustrative_only_candidates: fences.filter((fence) => fence.classification_candidate === "illustrative-only").length,
    shape_with_declared_identity: fences.filter((fence) => fence.language === "shape" && fence.declared_id).length,
  };
  for (const [key, expected] of Object.entries(expectations)) {
    if (counts[key] !== expected) {
      errors.push(`counts.${key} says ${counts[key]} but the rows hold ${expected}`);
    }
  }

  // ADR-016 §6 rejects a percentage over a curated subset, so the executed
  // subset must stay strictly smaller than the universe and both must be
  // reported. If they ever coincide, the note claiming one is a subset is false.
  if (counts.shape_runnable_gated_candidates > counts.shape_fences) {
    errors.push("counts.shape_runnable_gated_candidates exceeds counts.shape_fences — the executed set cannot be larger than the Shape universe");
  }
  if (counts.shape_fences > counts.fences_all_languages) {
    errors.push("counts.shape_fences exceeds counts.fences_all_languages");
  }
  if (!committed.executed_subset_note) {
    errors.push("executed_subset_note is missing — ADR-016 §6 rejects a curated subset presented as the universe, so the inventory must say which number is which");
  }

  const markerCounts = {};
  for (const fence of fences) {
    for (const marker of fence.markers ?? []) markerCounts[marker] = (markerCounts[marker] ?? 0) + 1;
    for (const field of CLASSIFICATION_FIELDS) {
      if (field in fence) {
        errors.push(`${fence.page}#${fence.fence_position}: carries "${field}". A scanned row records a classification CANDIDATE and the markers saying what is missing; ADR-016 §5's classification needs a reason and a citation, or declared modes and an expected outcome, and neither is derivable from a fence-info string`);
      }
    }
  }
  for (const [marker, count] of Object.entries(markerCounts)) {
    if (committed.marker_counts?.[marker] !== count) {
      errors.push(`marker_counts.${marker} says ${committed.marker_counts?.[marker]} but the rows carry ${count}`);
    }
  }
  for (const marker of Object.keys(committed.marker_counts ?? {})) {
    if (!(marker in markerCounts)) {
      errors.push(`marker_counts records ${marker}, which no row carries`);
    }
  }

  // §3: a stable identity is never an ordinal or a line number, and never
  // collides. Today none exist, so both rules are latent — implemented and
  // tripwired now so that turning them on is data, not design.
  const declared = new Map();
  for (const fence of fences) {
    if (!fence.declared_id) continue;
    if (/(?:^|[._-])(?:[0-9]+|l[0-9]+|line[._-]?[0-9]+|position[._-]?[0-9]+|ordinal[._-]?[0-9]+)(?:$|[._-])/.test(fence.declared_id)) {
      errors.push(`${fence.page}: fence identity "${fence.declared_id}" contains a line number or ordinal position — ADR-016 §3 forbids both, so that moving prose does not read as removing and adding a feature`);
    }
    if (declared.has(fence.declared_id)) {
      errors.push(`fence identity "${fence.declared_id}" is declared by both ${declared.get(fence.declared_id)} and ${fence.page} — a fence is one physical block`);
    }
    declared.set(fence.declared_id, fence.page);
  }

  if ((committed.unresolved_gaps ?? []).length === 0) {
    errors.push("unresolved_gaps is empty — the corpus carries no stable fence identities and no illustrative reasons, so an inventory claiming no gaps is claiming something untrue");
  }
  if ((committed.unterminated_fence_pages ?? []).length > 0) {
    errors.push(`unterminated fence(s) in ${committed.unterminated_fence_pages.join(", ")} — the scan could not find a closing delimiter, so its fence boundaries in those pages are unreliable`);
  }
  if (JSON.stringify(committed.corpus?.repository) !== JSON.stringify("shape-lang/shape-web")) {
    errors.push("corpus.repository must name shape-lang/shape-web");
  }
  // ADR-016 §3 / §7: no counterpart revision may be committed here.
  const smuggled = /(?:^|[^0-9a-fA-F])([0-9a-fA-F]{40}|[0-9a-fA-F]{64})(?:[^0-9a-fA-F]|$)/;
  for (const [key, value] of Object.entries(committed.corpus ?? {})) {
    if (typeof value === "string" && smuggled.test(value) && key !== "revision_policy") {
      errors.push(`corpus.${key} embeds a bare digest — ADR-016 §7 keeps the exact counterpart revision in the external PairCandidate, not in a source revision`);
    }
  }

  return errors;
}

export function checkCurrency(committed, derived) {
  const errors = [];
  const key = (fence) => `${fence.page}#${fence.fence_position}`;
  const committedByKey = new Map((committed.fences ?? []).map((fence) => [key(fence), fence]));
  const derivedByKey = new Map((derived.fences ?? []).map((fence) => [key(fence), fence]));

  const removed = [...committedByKey.keys()].filter((id) => !derivedByKey.has(id));
  const added = [...derivedByKey.keys()].filter((id) => !committedByKey.has(id));
  if (removed.length > 0) {
    errors.push(`${removed.length} committed fence(s) are gone from the corpus: ${removed.slice(0, 8).join(", ")}${removed.length > 8 ? `, and ${removed.length - 8} more` : ""}`);
  }
  if (added.length > 0) {
    errors.push(`${added.length} corpus fence(s) are absent from the committed inventory: ${added.slice(0, 8).join(", ")}${added.length > 8 ? `, and ${added.length - 8} more` : ""}`);
  }
  if (digest(committed.fences ?? []) !== derived.fences_sha256) {
    const changed = (derived.fences ?? []).filter((fence) => {
      const before = committedByKey.get(key(fence));
      return before && canonicalJson(before) !== canonicalJson(fence);
    });
    errors.push(`the committed fences hash to ${digest(committed.fences ?? []).slice(0, 16)}, the corpus derives ${derived.fences_sha256.slice(0, 16)}${changed.length > 0 ? `; ${changed.length} row(s) differ, first ${key(changed[0])}` : ""}`);
  }
  if (digest(committed.sections ?? []) !== derived.sections_sha256) {
    errors.push(`the committed sections hash to ${digest(committed.sections ?? []).slice(0, 16)}, the corpus derives ${derived.sections_sha256.slice(0, 16)}`);
  }
  return errors;
}

// --- forced negatives ----------------------------------------------------

function tripwires(committed) {
  const clone = () => JSON.parse(JSON.stringify(committed));
  return [
    {
      id: "T1 rows edited without regenerating",
      mutate: (value) => {
        value.fences[0].page = "somewhere/else.mdx";
      },
      expect: "the rows were edited without regenerating",
    },
    {
      id: "T2 the executed subset presented as the universe",
      mutate: (value) => {
        value.counts.shape_fences = value.counts.shape_runnable_gated_candidates;
      },
      expect: "counts.shape_fences says",
    },
    {
      id: "T3 the subset note removed",
      mutate: (value) => {
        delete value.executed_subset_note;
      },
      expect: "rejects a curated subset presented as the universe",
    },
    {
      id: "T4 a scanned row given a real classification",
      mutate: (value) => {
        value.fences[0].classification = "illustrative-only";
      },
      expect: "records a classification CANDIDATE",
    },
    {
      id: "T5 a fence identity built from a line number",
      mutate: (value) => {
        const fence = value.fences.find((row) => row.language === "shape");
        fence.declared_id = "fundamentals.operators.l354";
        fence.markers = fence.markers.filter((marker) => marker !== "missing-stable-identity");
      },
      expect: "contains a line number or ordinal position",
    },
    {
      id: "T6 one fence identity declared twice",
      mutate: (value) => {
        const shape = value.fences.filter((row) => row.language === "shape").slice(0, 2);
        for (const fence of shape) {
          fence.declared_id = "fundamentals.operators.pipe";
          fence.markers = fence.markers.filter((marker) => marker !== "missing-stable-identity");
        }
      },
      expect: "a fence is one physical block",
    },
    {
      id: "T7 the gap list emptied",
      mutate: (value) => {
        value.unresolved_gaps = [];
      },
      expect: "claiming something untrue",
    },
    {
      id: "T8 a marker count quietly lowered",
      mutate: (value) => {
        value.marker_counts["missing-stable-identity"] -= 1;
      },
      expect: "marker_counts.missing-stable-identity says",
    },
    {
      id: "T9 a counterpart revision smuggled into the corpus record",
      mutate: (value) => {
        value.corpus.read_from = `committed content at ${"a1b2c3d4".repeat(5)}`;
      },
      expect: "keeps the exact counterpart revision in the external PairCandidate",
    },
  ];
}

// --- cli -----------------------------------------------------------------

const argv = process.argv.slice(2);
if (argv.includes("--help")) {
  console.log("Usage: node scripts/check-adr011-012-book-fence-inventory.mjs [--shape-web <path>] [--write] [--self-test]");
  console.log("");
  console.log("  --shape-web  re-derive from a shape-web checkout and diff (the currency half).");
  console.log("               Without it only the integrity half runs, and says so.");
  console.log("  --write      regenerate the committed inventory. Requires a shape-web checkout.");
  console.log("  --self-test  run the forced negatives.");
  process.exit(0);
}

const shapeWebIndex = argv.indexOf("--shape-web");
const shapeWeb = shapeWebIndex === -1 ? undefined : path.resolve(argv[shapeWebIndex + 1]);

if (argv.includes("--write")) {
  const inventory = buildInventory(shapeWeb ?? defaultShapeWeb);
  fs.writeFileSync(inventoryPath, `${JSON.stringify(inventory, null, 2)}\n`);
  console.log(`Wrote ${path.relative(repositoryRoot, inventoryPath)}: ${inventory.counts.fences_all_languages} fences, ${inventory.counts.sections} sections.`);
  process.exit(0);
}

const committed = readCommitted();
const errors = checkIntegrity(committed);

let currencyChecked = false;
if (shapeWeb !== undefined || fs.existsSync(path.join(defaultShapeWeb, ".git"))) {
  const root = shapeWeb ?? defaultShapeWeb;
  try {
    errors.push(...checkCurrency(committed, buildInventory(root)));
    currencyChecked = true;
  } catch (error) {
    if (shapeWeb !== undefined) {
      console.error(`FATAL  #115: cannot scan the shape-web checkout at ${root}: ${error.message}`);
      process.exit(2);
    }
  }
}

if (errors.length > 0) {
  console.error(`ADR-016 Book fence inventory INVALID (${errors.length} error${errors.length === 1 ? "" : "s"}):`);
  for (const error of errors) console.error(`- ${error}`);
  console.error("");
  console.error("Regenerate with: node scripts/check-adr011-012-book-fence-inventory.mjs --write --shape-web <path>");
  process.exit(1);
}

const counts = committed.counts;
console.log(
  `ADR-016 Book fence inventory OK: ${counts.fences_all_languages} fences across ${counts.pages} pages and ${counts.sections} sections; ` +
    `${counts.shape_fences} are Shape, of which ${counts.shape_runnable_gated_candidates} are runnable-gated candidates and ` +
    `${counts.shape_illustrative_only_candidates} are illustrative-only candidates; ${counts.shape_with_declared_identity} carry a stable identity. ` +
    `${currencyChecked ? "Re-derived from the corpus." : "Integrity only — no shape-web checkout, so currency was NOT verified."}`,
);

if (argv.includes("--self-test")) {
  console.log("Forced negatives:");
  const failures = [];
  for (const tripwire of tripwires(committed)) {
    const controlErrors = checkIntegrity(committed);
    if (controlErrors.length > 0) {
      failures.push(`${tripwire.id}: positive control was rejected — ${controlErrors[0]}`);
    }
    const candidate = JSON.parse(JSON.stringify(committed));
    tripwire.mutate(candidate);
    const tripwireErrors = checkIntegrity(candidate);
    if (tripwireErrors.length === 0) {
      failures.push(`${tripwire.id}: forced negative was ACCEPTED`);
    } else if (!tripwireErrors.some((error) => error.includes(tripwire.expect))) {
      failures.push(`${tripwire.id}: rejected, but for the wrong reason — expected "${tripwire.expect}", got: ${tripwireErrors[0]}`);
    } else {
      console.log(`  ${tripwire.id}: rejected`);
    }
  }
  if (failures.length > 0) {
    console.error(`Self-test FAILED (${failures.length}):`);
    for (const failure of failures) console.error(`- ${failure}`);
    process.exit(1);
  }
  console.log("Self-test OK: every tripwire rejected its forced negative and accepted its positive control.");
}
