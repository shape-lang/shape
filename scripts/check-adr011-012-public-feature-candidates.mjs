#!/usr/bin/env node
//
// ADR-016 §2 / §3 / R19 — the public feature candidate inventory gate
// (#114, PF-INVENTORY).
//
// Exit 0 = the committed inventory is exactly what the scanner derives from the
// current tree. Exit 1 = drift. Exit 2 = the inputs are unusable.
//
// ADR-016 §3 lets a complete inventory be implemented in bounded waves "only
// after their exact stable rows and content hash are committed". This gate is
// what makes "exact" mean something: the committed rows are re-derived from the
// grammar, the stdlib sources, the CLI enums, the LSP capabilities and the ABI
// on every run, so the file cannot drift away from the code it inventories, and
// the wave breakdown cannot be planned against a stale denominator.
//
// Growth and shrinkage are both drift and both fail, but they are reported
// separately and shrinkage first. A public surface that disappears is the one
// that matters: ADR-016 §2 requires a retired feature to become a removed row
// with a tombstone rather than to vanish, and a row that vanishes from the
// candidate inventory before it was ever entered in the manifest would never get
// that tombstone. Regenerate with --write and the diff is the review surface.
//
// `--self-test` runs the forced negatives, each asserting both that its mutation
// is rejected and that the unmutated input is accepted.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { buildInventory, canonicalJson, inventoryPath, repositoryRoot } from "./generate-adr011-012-public-feature-candidates.mjs";

function readCommitted() {
  try {
    return JSON.parse(fs.readFileSync(inventoryPath, "utf8"));
  } catch (error) {
    console.error(`FATAL  #114: cannot read ${inventoryPath}: ${error.message}`);
    process.exit(2);
  }
}

export function compare(committed, derived) {
  const errors = [];

  const committedIds = new Set((committed.rows ?? []).map((row) => row.candidate_id));
  const derivedIds = new Set((derived.rows ?? []).map((row) => row.candidate_id));

  const removed = [...committedIds].filter((id) => !derivedIds.has(id));
  const added = [...derivedIds].filter((id) => !committedIds.has(id));

  if (removed.length > 0) {
    errors.push(
      `${removed.length} committed candidate row(s) are no longer derivable from the tree: ${removed.slice(0, 10).join(", ")}${removed.length > 10 ? `, and ${removed.length - 10} more` : ""}. ` +
        "A public surface that disappears from the inventory never gets the removed-row tombstone ADR-016 §2 requires. " +
        "If the surface was genuinely retired, regenerate with --write and say so in the commit; if it was not, this is an accidental deletion.",
    );
  }
  if (added.length > 0) {
    errors.push(
      `${added.length} newly derivable candidate row(s) are not in the committed inventory: ${added.slice(0, 10).join(", ")}${added.length > 10 ? `, and ${added.length - 10} more` : ""}. ` +
        "Regenerate with --write so the wave breakdown is planned against the current denominator (ADR-016 §3).",
    );
  }

  // The stored hash is recomputed rather than trusted. Trusting it would let a
  // row's content be edited in place while the field kept its old value, which
  // is the one mutation an identity-set comparison cannot see.
  const committedHash = crypto.createHash("sha256").update(canonicalJson(committed.rows ?? [])).digest("hex");
  if (committed.rows_sha256 !== committedHash) {
    errors.push(
      `rows_sha256 records ${committed.rows_sha256?.slice(0, 16)} but the committed rows hash to ${committedHash.slice(0, 16)} — the rows were edited without regenerating`,
    );
  }

  // Row content can drift without the identity set changing — a moved source
  // file, a renamed component.
  if (committedHash !== derived.rows_sha256) {
    if (removed.length === 0 && added.length === 0) {
      const changed = (derived.rows ?? []).filter((row) => {
        const before = (committed.rows ?? []).find((other) => other.candidate_id === row.candidate_id);
        return before && canonicalJson(before) !== canonicalJson(row);
      });
      errors.push(
        `the row set is unchanged but ${changed.length} row(s) have different content — the committed rows hash to ${committedHash.slice(0, 16)}, the derived to ${derived.rows_sha256.slice(0, 16)}` +
          (changed.length > 0 ? `; first: ${changed[0].candidate_id}` : ""),
      );
    } else {
      errors.push(`the committed rows hash to ${committedHash.slice(0, 16)}, which does not match the derived ${derived.rows_sha256.slice(0, 16)}`);
    }
  }

  if (committed.count !== (committed.rows ?? []).length) {
    errors.push(`count says ${committed.count} but the file holds ${(committed.rows ?? []).length} rows`);
  }

  // The scan gaps are the inventory's statement about its own incompleteness.
  // Silently dropping one would turn a known hole into an invisible one, which
  // ADR-016 §2 treats as a blocking ambiguity rather than a clean inventory.
  const committedGaps = new Set((committed.unresolved_scan_gaps ?? []).map((gap) => gap.surface));
  for (const gap of derived.unresolved_scan_gaps ?? []) {
    if (!committedGaps.has(gap.surface)) {
      errors.push(`the scanner declares an unresolved gap the committed inventory does not record: ${gap.surface}`);
    }
  }

  // Classification is the P waves' work (ADR-016 §2 makes status evidence-
  // derived). A candidate row that had acquired a status here would be an
  // aspirational denominator with no evidence behind it.
  const CLASSIFICATION_FIELDS = ["status", "status_basis", "required_modes", "required_evidence_classes", "required_semantic_dimensions"];
  for (const row of committed.rows ?? []) {
    for (const field of CLASSIFICATION_FIELDS) {
      if (field in row) {
        errors.push(`${row.candidate_id}: carries "${field}". A candidate row records where a surface was found, not how mature it is; ADR-016 §2 makes status evidence-derived and #114 assigns classification to the P waves`);
      }
    }
  }

  return errors;
}

// --- forced negatives ----------------------------------------------------

function tripwires(derived) {
  const clone = () => JSON.parse(JSON.stringify(derived));
  const firstId = derived.rows[0].candidate_id;
  return [
    {
      id: "T1 a public surface silently dropped",
      committed: (() => {
        const value = clone();
        value.rows.push({
          candidate_id: "selftest.retired.surface",
          public_name: "Self-test retired surface",
          family: "self-test",
          surface_authority: { kind: "source", reference: "self-test fixture" },
          owner: { repository: "shape-lang/shape", component: "self-test fixture" },
        });
        value.count = value.rows.length;
        return value;
      })(),
      expect: "never gets the removed-row tombstone",
    },
    {
      id: "T2 a new surface not yet in the committed denominator",
      committed: (() => {
        const value = clone();
        value.rows = value.rows.filter((row) => row.candidate_id !== firstId);
        value.count = value.rows.length;
        return value;
      })(),
      expect: "not in the committed inventory",
    },
    {
      id: "T3 row content changed without the identity set changing",
      committed: (() => {
        const value = clone();
        value.rows[0].surface_authority.reference = "somewhere else entirely";
        return value;
      })(),
      expect: "have different content",
    },
    {
      id: "T4 a declared scan gap quietly dropped",
      committed: (() => {
        const value = clone();
        value.unresolved_scan_gaps = value.unresolved_scan_gaps.slice(1);
        return value;
      })(),
      expect: "the committed inventory does not record",
    },
    {
      id: "T5 a candidate row classified in the inventory",
      committed: (() => {
        const value = clone();
        value.rows[0].status = "public";
        return value;
      })(),
      expect: "assigns classification to the P waves",
    },
    {
      id: "T6 count disagreeing with the rows",
      committed: (() => {
        const value = clone();
        value.count += 1;
        return value;
      })(),
      expect: "but the file holds",
    },
  ];
}

// --- cli -----------------------------------------------------------------

const argv = process.argv.slice(2);
if (argv.includes("--help")) {
  console.log("Usage: node scripts/check-adr011-012-public-feature-candidates.mjs [--write] [--self-test]");
  console.log("");
  console.log("  --write      regenerate the committed inventory from the current tree.");
  console.log("  --self-test  run the forced negatives.");
  process.exit(0);
}

const derived = buildInventory();

if (argv.includes("--write")) {
  fs.writeFileSync(inventoryPath, `${JSON.stringify(derived, null, 2)}\n`);
  console.log(`Wrote ${path.relative(repositoryRoot, inventoryPath)}: ${derived.count} candidate rows.`);
  process.exit(0);
}

const committed = readCommitted();
const errors = compare(committed, derived);
if (errors.length > 0) {
  console.error(`ADR-016 public feature candidate inventory DRIFTED (${errors.length} error${errors.length === 1 ? "" : "s"}):`);
  for (const error of errors) console.error(`- ${error}`);
  console.error("");
  console.error("Regenerate with: node scripts/check-adr011-012-public-feature-candidates.mjs --write");
  process.exit(1);
}

const byFamily = new Map();
for (const row of committed.rows) byFamily.set(row.family, (byFamily.get(row.family) ?? 0) + 1);
console.log(
  `ADR-016 public feature candidate inventory OK: ${committed.count} candidate rows across ${byFamily.size} families, ` +
    `${committed.unresolved_scan_gaps.length} unresolved scan gap(s), rows_sha256 ${committed.rows_sha256.slice(0, 16)}.`,
);

if (argv.includes("--self-test")) {
  console.log("Forced negatives:");
  const failures = [];
  for (const tripwire of tripwires(derived)) {
    const controlErrors = compare(derived, derived);
    if (controlErrors.length > 0) {
      failures.push(`${tripwire.id}: positive control was rejected — ${controlErrors[0]}`);
    }
    const tripwireErrors = compare(tripwire.committed, derived);
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
