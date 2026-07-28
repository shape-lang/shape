// Scanner for the JIT whole-program-bail inventory (#187, ADR-018 §2).
//
// A "whole-program bail" is a refusal that abandons native execution of an
// ENTIRE program because one construct somewhere in it cannot be lowered
// soundly. ADR-018 §2 ratchets this set to zero: an unsupported construct must
// cost its enclosing function native execution, never the program.
//
// The inventory is scanned from two independent sources, and BOTH must agree
// with the committed baseline:
//
//   1. `sites` — every refusal point in the JIT compile path, each declaring
//      itself with a `// WHOLE-PROGRAM-BAIL[<category>]: <id> — <reason>`
//      marker. The scanner also reports UNMARKED whole-program refusals in the
//      same files, so a new bail cannot be added by omitting the marker.
//
//      Two categories. `construct` is a refusal driven by something the source
//      program contains — the set ADR-018 §2 ratchets to zero. `infra` is a
//      failure of the compilation machinery itself (Cranelift codegen, a
//      caught panic) or a runtime surface reached after execution; those
//      abandon the program too, but removing them is a different problem from
//      per-function granularity, so they are inventoried and reported rather
//      than counted against the zero target. Adding an `infra` site is still
//      growth: it must appear in the baseline.
//
//   2. `program-scoped-residuals` — the residual constructs whose
//      `JitResidual::scope()` is `Program`, read from the
//      `program_scope_reason` match arms in `jit_residual.rs`. Those arms are
//      the ones that must record WHY the refusal cannot be narrowed to the
//      owning function, and a Rust unit test
//      (`every_program_scoped_residual_records_why_it_cannot_be_narrowed`)
//      keeps them in lockstep with `scope()` itself.
//
// Scanning source text is the same mechanism `check-no-dynamic.sh` and the
// #133-family baselines use. It is deliberate: the gate must work on a diff
// without building or running the compiler.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";

/// Files that may contain a whole-program bail. A refusal added anywhere else
/// in the JIT compile path is invisible to this gate, so this list is part of
/// the baseline's hashed content.
export const BAIL_SOURCE_FILES = [
  "crates/shape-jit/src/executor.rs",
  "crates/shape-jit/src/compiler/strategy.rs",
  "crates/shape-jit/src/compiler/program.rs",
];

export const RESIDUAL_SOURCE_FILE = "crates/shape-vm/src/bytecode/jit_residual.rs";

const MARKER = /^\s*\/\/ WHOLE-PROGRAM-BAIL\[(construct|infra)\]:\s*([a-z0-9-]+)\s*—\s*(.+)$/;

/// A refusal expression that abandons the whole program. `compile_program_selective`
/// and `compile_strategy*` return `Err(String)`; `execute_with_jit` returns
/// `Err(ShapeError)`. Both reach the same `[jit-fallback]` path.
const REFUSAL = /^\s*return Err\(/;

/// Refusals that demote a single function rather than the program, so they are
/// not bails at all. Excluded by the enclosing function's name.
const NON_BAIL_FUNCTIONS = new Set([
  "compile_function_with_user_funcs",
  "compile_correlated_kernel",
]);

function enclosingFunction(lines, index) {
  for (let i = index; i >= 0; i -= 1) {
    const match = /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?fn\s+([A-Za-z0-9_]+)/.exec(lines[i]);
    if (match) return match[1];
  }
  return "<module>";
}

/// Scan one file for marked bails and for refusals that carry no marker.
function scanFile(repositoryRoot, relativePath) {
  const absolute = path.join(repositoryRoot, relativePath);
  if (!fs.existsSync(absolute)) {
    throw new Error(`bail source file is missing: ${relativePath}`);
  }
  const lines = fs.readFileSync(absolute, "utf8").split("\n");

  const marked = [];
  const unmarked = [];

  for (let index = 0; index < lines.length; index += 1) {
    const markerMatch = MARKER.exec(lines[index]);
    if (markerMatch) {
      marked.push({
        id: markerMatch[2],
        category: markerMatch[1],
        reason: markerMatch[3].trim(),
        file: relativePath,
      });
      continue;
    }

    if (!REFUSAL.test(lines[index])) continue;
    const owner = enclosingFunction(lines, index);
    if (NON_BAIL_FUNCTIONS.has(owner)) continue;

    // A refusal belongs to the marker that introduces its guard. Walk back over
    // the guard's condition and its explanatory comment block; if a marker is
    // found before the previous statement ends, this refusal is accounted for.
    let accounted = false;
    for (let back = index - 1; back >= 0 && index - back < 60; back -= 1) {
      if (MARKER.test(lines[back])) {
        accounted = true;
        break;
      }
      // A blank line inside a comment block is still part of it; a closing
      // brace at lower indentation means we left the guard.
      if (/^\s*\}\s*$/.test(lines[back])) break;
    }
    if (!accounted) {
      unmarked.push({ file: relativePath, line: index + 1, owner, text: lines[index].trim() });
    }
  }

  return { marked, unmarked };
}

/// Read the residual kinds whose scope is `Program` from the
/// `program_scope_reason` match arms.
function scanProgramScopedResiduals(repositoryRoot) {
  const absolute = path.join(repositoryRoot, RESIDUAL_SOURCE_FILE);
  if (!fs.existsSync(absolute)) {
    throw new Error(`residual source file is missing: ${RESIDUAL_SOURCE_FILE}`);
  }
  const source = fs.readFileSync(absolute, "utf8");

  const body = /pub fn program_scope_reason\(&self\)[^{]*\{([\s\S]*?)\n    \}/.exec(source);
  if (!body) {
    throw new Error(
      `could not locate \`program_scope_reason\` in ${RESIDUAL_SOURCE_FILE}; the scanner and the source have drifted`,
    );
  }

  const stableIds = new Map();
  const idBody = /pub fn stable_id\(&self\)[^{]*\{([\s\S]*?)\n    \}/.exec(source);
  if (!idBody) {
    throw new Error(`could not locate \`stable_id\` in ${RESIDUAL_SOURCE_FILE}`);
  }
  for (const arm of idBody[1].matchAll(/JitResidual::([A-Za-z0-9_]+)\s*=>\s*"([a-z0-9-]+)"/g)) {
    stableIds.set(arm[1], arm[2]);
  }

  const scoped = [];
  for (const arm of body[1].matchAll(/JitResidual::([A-Za-z0-9_]+)\s*=>\s*Some\(/g)) {
    const variant = arm[1];
    const stableId = stableIds.get(variant);
    if (!stableId) {
      throw new Error(`residual variant ${variant} has no \`stable_id\` arm`);
    }
    scoped.push({ id: stableId, variant });
  }
  scoped.sort((a, b) => a.id.localeCompare(b.id));
  return scoped;
}

/// The full inventory at the current working tree.
export function scanWholeProgramBails(repositoryRoot) {
  const sites = [];
  const unmarked = [];
  for (const file of BAIL_SOURCE_FILES) {
    const scanned = scanFile(repositoryRoot, file);
    sites.push(...scanned.marked);
    unmarked.push(...scanned.unmarked);
  }
  sites.sort((a, b) => a.id.localeCompare(b.id));

  const duplicates = sites
    .map((site) => site.id)
    .filter((id, index, all) => all.indexOf(id) !== index);
  if (duplicates.length > 0) {
    throw new Error(`duplicate WHOLE-PROGRAM-BAIL ids: ${[...new Set(duplicates)].join(", ")}`);
  }

  return {
    sites,
    unmarked,
    programScopedResiduals: scanProgramScopedResiduals(repositoryRoot),
  };
}

/// Content hash over the parts a hand-edit could weaken. `reason` text is
/// included so a marker cannot be silently retargeted at a different construct.
export function hashInventory(inventory) {
  const canonical = JSON.stringify({
    files: BAIL_SOURCE_FILES,
    sites: inventory.sites.map((site) => [site.id, site.category, site.file, site.reason]),
    programScopedResiduals: inventory.programScopedResiduals.map((r) => [r.id, r.variant]),
  });
  return crypto.createHash("sha256").update(canonical).digest("hex");
}
