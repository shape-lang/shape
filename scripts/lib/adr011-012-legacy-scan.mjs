// ADR-011..016 step-4 migration baselines — the scanner.
//
// One implementation, imported by both the generator and the growth check, so
// a committed baseline can never be produced by a rule the gate does not
// enforce. See scripts/lib/adr011-012-legacy-sets.mjs for the set definitions.
//
// Determinism contract (step 4 requires a regenerable artifact):
//   - owners are sorted by path, so ripgrep's parallel walk order cannot leak
//     into the output;
//   - no timestamp, hostname, absolute path, or duration is ever recorded;
//   - the content hash covers the set rows ONLY, never the source revision, so
//     regenerating at a later revision with unchanged code reproduces the same
//     hash.

import crypto from "node:crypto";
import { execFileSync } from "node:child_process";

// ripgrep is already a hard dependency of scripts/check-no-dynamic.sh; using it
// here keeps both ratchets counting the same way over the same working tree
// (respecting .gitignore, so build outputs are never counted).
export function scanSet(repositoryRoot, set) {
  const exclude = set.exclude ?? [];
  const excludeArguments = exclude.flatMap((glob) => ["--glob", `!${glob}`]);
  let stdout;
  try {
    stdout = execFileSync(
      "rg",
      ["--no-heading", "--count-matches", "--pcre2", ...excludeArguments, "--", set.pattern, ...set.scope],
      { cwd: repositoryRoot, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
    );
  } catch (error) {
    // ripgrep exits 1 for "no matches", which is a legitimate empty set — a
    // fully retired legacy class. Any other status is a real failure.
    if (error && error.status === 1 && !error.stderr) {
      stdout = "";
    } else {
      const detail = error && error.stderr ? String(error.stderr).trim() : String(error);
      throw new Error(`ripgrep failed for set '${set.id}': ${detail}`);
    }
  }

  const owners = [];
  for (const line of stdout.split("\n")) {
    if (line.length === 0) continue;
    const separator = line.lastIndexOf(":");
    if (separator < 0) throw new Error(`unparsable ripgrep row for set '${set.id}': ${line}`);
    const path = line.slice(0, separator);
    const count = Number.parseInt(line.slice(separator + 1), 10);
    if (!Number.isInteger(count)) throw new Error(`unparsable count for set '${set.id}': ${line}`);
    owners.push({ path, count });
  }
  owners.sort((left, right) => (left.path < right.path ? -1 : left.path > right.path ? 1 : 0));

  return {
    id: set.id,
    category: set.category,
    description: set.description,
    scope: [...set.scope],
    exclude: [...exclude],
    pattern: set.pattern,
    count: owners.reduce((total, owner) => total + owner.count, 0),
    owner_count: owners.length,
    owners,
  };
}

export function scanBaseline(repositoryRoot, baseline) {
  return baseline.sets.map((set) => scanSet(repositoryRoot, set));
}

// Canonical form for hashing: only the facts a later slice may change. The
// description is deliberately excluded so that clarifying prose never looks
// like legacy movement.
function canonicalSets(sets) {
  return sets.map((set) => ({
    id: set.id,
    pattern: set.pattern,
    scope: set.scope,
    exclude: set.exclude ?? [],
    count: set.count,
    owners: set.owners.map((owner) => ({ path: owner.path, count: owner.count })),
  }));
}

export function hashSets(sets) {
  return crypto.createHash("sha256").update(JSON.stringify(canonicalSets(sets))).digest("hex");
}

export function buildBaselineDocument(baseline, sets, sourceRevision) {
  return {
    record_kind: "LegacyMigrationBaseline",
    record_version: 1,
    program_id: "adr011-012",
    ticket: baseline.ticket,
    entry_id: baseline.id,
    manifest_ordinal: baseline.manifest_ordinal,
    title: baseline.title,
    territory: baseline.territory,
    authority: baseline.authority,
    step: "docs/design/typed-comptime/adr011-012-execution-rulings.md — ten required steps, step 4",
    source_revision: sourceRevision,
    regenerate_with: `node scripts/generate-adr011-012-legacy-baselines.mjs --ticket ${baseline.ticket}`,
    check_with: `node scripts/check-adr011-012-legacy-baselines.mjs --ticket ${baseline.ticket}`,
    direction_rule:
      "Monotonic non-increasing. A set count may fall (migration progress) and may never rise. A new owner path is growth even when the total is unchanged, because it is a new surface carrying old authority.",
    excluded_from_hash: ["source_revision", "description"],
    total_count: sets.reduce((total, set) => total + set.count, 0),
    total_count_note:
      "Sum of the set counts. Sets may deliberately overlap (a narrowly scoped set can re-count sites a broader set already counts), so this is a ratchet aggregate for reporting movement — never a count of distinct legacy sites.",
    sets_sha256: hashSets(sets),
    sets,
  };
}
