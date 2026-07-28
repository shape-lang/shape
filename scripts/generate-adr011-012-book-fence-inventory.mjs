#!/usr/bin/env node
//
// ADR-016 §3 / §5 / R19 — the complete Book fence universe
// (#115, BOOK-FENCE-INVENTORY).
//
// ADR-016 §5 requires every Shape code fence to carry a stable explicit identity
// and exactly one classification, and §6 requires the gate to extract "the full
// Shape-fence universe" rather than a chosen subset. This scans that universe.
//
// It scans shape-web's COMMITTED content, via `git show HEAD:<path>`, not the
// working tree. The working tree of a documentation repository is routinely a
// lane's uncommitted draft; an inventory derived from it would describe a corpus
// nobody else can see and would change under its own feet.
//
// The inventory records NO shape-web revision. ADR-016 §3 and #115's own
// acceptance keep exact revisions external, in the PairCandidate and the
// attestation — a counterpart SHA committed into this repository is the
// reciprocal pin §7 exists to prevent. What pins the inventory instead is its
// own content hash over the extracted rows.
//
// Two things this scanner deliberately does not do. It does not classify a
// fence: §5's classification is `runnable-gated` with declared modes and an
// expected outcome, or `illustrative-only` with a nonempty reason and citation,
// and the corpus today carries neither — only the harness's `runnable=` flag,
// which is a different and weaker fact. Rows therefore record a classification
// CANDIDATE plus the markers saying what is missing. And it does not mint fence
// identities: it records that they are absent, which is the point.
//
// Usage:
//   node scripts/generate-adr011-012-book-fence-inventory.mjs [--shape-web <path>]
//
// Default --shape-web is ../shape-web relative to this repository.

import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
export const repositoryRoot = path.resolve(scriptDirectory, "..");
export const inventoryPath = path.join(
  repositoryRoot,
  "docs/program/adr011-012/book-fence-inventory.json",
);
export const defaultShapeWeb = path.resolve(repositoryRoot, "../shape-web");

const DOCS_ROOT = "book/book-site/src/content/docs";

function git(shapeWeb, args) {
  return execFileSync("git", ["-C", shapeWeb, ...args], { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
}

export function listPages(shapeWeb) {
  return git(shapeWeb, ["ls-tree", "-r", "--name-only", "HEAD", "--", DOCS_ROOT])
    .split("\n")
    .filter((line) => line.endsWith(".mdx"))
    .sort();
}

// --- slugging -------------------------------------------------------------

// The anchor an Astro/Starlight heading receives. Derived rather than assumed,
// and validated against the built site by --validate-anchors: the pipeline is
// github-slugger over the heading's rendered text, minus one trailing hyphen.
// The trailing-hyphen rule is not cosmetic — `## Pipe (\`|>\`)` slugs to `pipe-`
// under github-slugger alone but is `pipe` in the built page, and #113's schema
// rejects a trailing hyphen, so guessing here would have produced an anchor the
// coverage manifest cannot express.
export function headingText(raw) {
  return raw
    .replace(/`([^`]*)`/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[*_]{1,2}([^*_]+)[*_]{1,2}/g, "$1")
    .trim();
}

export function slugify(text, seen) {
  const base = text
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\p{M}\p{Pc} -]/gu, "")
    .replace(/ /g, "-")
    .replace(/-$/, "");
  if (!seen) return base;
  const count = seen.get(base) ?? 0;
  seen.set(base, count + 1);
  return count === 0 ? base : `${base}-${count}`;
}

// --- fence-info parsing ---------------------------------------------------

function decodeFenceString(value) {
  return value.replace(/\\([ntr"\\])/g, (_, character) =>
    character === "n" ? "\n" : character === "t" ? "\t" : character === "r" ? "\r" : character,
  );
}

// Mirrors the conventions the committed extractor and harness already use
// (book/book-site/scripts/MANIFEST_SCHEMA.md), plus the `id=` token ADR-016 §5
// requires and the corpus does not yet carry.
export function parseFenceInfo(info) {
  const language = /^([A-Za-z][A-Za-z0-9_-]*)/.exec(info)?.[1] ?? null;
  const meta = language ? info.slice(language.length) : info;
  const flags = { language };

  const runnable = /(?:^|\s)runnable=(true|false|deferred)(?=\s|$)/.exec(meta);
  flags.runnable = runnable ? runnable[1] : null;
  const declaredId = /(?:^|\s)id=([A-Za-z0-9_.-]+)(?=\s|$)/.exec(meta);
  flags.declared_id = declaredId ? declaredId[1] : null;
  const cite = /(?:^|\s)cite=([A-Za-z0-9_\-.:#/]+)(?=\s|$)/.exec(meta);
  if (cite) flags.cite = cite[1];
  const expected = /(?:^|\s)expected="((?:[^"\\]|\\.)*)"(?=\s|$)/.exec(meta);
  if (expected) flags.expected = decodeFenceString(expected[1]);
  const expectedFail = /(?:^|\s)expected-fail="((?:[^"\\]|\\.)*)"(?=\s|$)/.exec(meta);
  if (expectedFail) flags.expected_fail = decodeFenceString(expectedFail[1]);
  const fixture = /(?:^|\s)fixture=([A-Za-z0-9_-]+)(?=\s|$)/.exec(meta);
  if (fixture) flags.fixture = fixture[1];
  const serveSandbox = /(?:^|\s)serve-sandbox=(none|strict|permissive)(?=\s|$)/.exec(meta);
  if (serveSandbox) flags.serve_sandbox = serveSandbox[1];
  return flags;
}

// --- the scan -------------------------------------------------------------

// Slice attribution, copied from the committed extractor so the inventory and
// the existing harness partition the corpus identically. It is recorded, not
// relied on: ADR-016 §6 rejects a curated subset, and a slice is a subset.
export function sliceFor(relativePath) {
  const base = path.basename(relativePath, ".mdx");
  if (relativePath === "index.mdx") return "C";
  if (relativePath.startsWith("appendix/")) return "A";
  if (relativePath.startsWith("fundamentals/")) {
    return ["datetime", "tables", "content"].includes(base) ? "B" : "A";
  }
  if (relativePath.startsWith("stdlib/")) return "B";
  if (relativePath.startsWith("getting-started/") || relativePath.startsWith("examples/")) return "C";
  if (relativePath.startsWith("advanced/")) {
    if (base === "ownership-deep-dive") return "A";
    return ["comptime", "annotations", "comptime-annotations-cookbook", "native-c-interop"].includes(base) ? "D" : "E";
  }
  if (relativePath.startsWith("tooling/")) {
    return ["polyglot", "python-extension", "typescript-extension", "extensions"].includes(base) ? "D" : "E";
  }
  return "A";
}

function pageSlug(relativePath) {
  return relativePath.replace(/\.mdx$/, "").replace(/[/\\]/g, "__").replace(/[^A-Za-z0-9_]/g, "-");
}

function stripFrontmatter(raw) {
  const trimmed = raw.trimStart();
  if (!trimmed.startsWith("---")) return { body: raw, lineOffset: 0 };
  const close = trimmed.indexOf("\n---", 3);
  if (close === -1) return { body: raw, lineOffset: 0 };
  const prefix = raw.slice(0, raw.length - trimmed.length) + trimmed.slice(0, close + 4);
  return { body: trimmed.slice(close + 4), lineOffset: prefix.split("\n").length - 1 };
}

function scanPage(relativePath, raw, sections, fences) {
  const { body, lineOffset } = stripFrontmatter(raw);
  const lines = body.split("\n");
  const slug = pageSlug(relativePath);
  const slice = sliceFor(relativePath);
  const seenAnchors = new Map();

  let openDelimiter = null;
  let openInfo = null;
  let openLine = 0;
  let openBody = [];
  let currentSection = null;
  let shapePosition = 0;
  let fencePosition = 0;

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const fenceMatch = /^[ \t]*(`{3,})(.*)$/.exec(line);

    if (openDelimiter) {
      // A closing delimiter is a run at least as long as the opening one and
      // carries no info string. Anything else is fence content.
      if (fenceMatch && fenceMatch[1].length >= openDelimiter.length && fenceMatch[2].trim() === "") {
        const flags = parseFenceInfo(openInfo.trim());
        const isShape = flags.language === "shape";
        fences.push(buildFence({
          relativePath, slug, slice, flags, isShape,
          harnessLine: openLine + lineOffset,
          fencePosition,
          shapePosition: isShape ? shapePosition : null,
          section: currentSection,
          bodyLines: openBody.length,
        }));
        fencePosition += 1;
        if (isShape) shapePosition += 1;
        openDelimiter = null;
        openInfo = null;
        openBody = [];
      } else {
        openBody.push(line);
      }
      continue;
    }

    if (fenceMatch && fenceMatch[2].trim() !== "") {
      openDelimiter = fenceMatch[1];
      openInfo = fenceMatch[2];
      openLine = index + 1;
      continue;
    }
    if (fenceMatch) {
      openDelimiter = fenceMatch[1];
      openInfo = "";
      openLine = index + 1;
      continue;
    }

    // h1 included: a page body may carry its own `# Title` heading, which takes
    // the base slug and pushes a later same-named h2 to `-1`. Scanning from h2
    // produced exactly one wrong anchor across the built site.
    const heading = /^(#{1,6})\s+(.+?)\s*$/.exec(line);
    if (heading) {
      const text = headingText(heading[2]);
      const anchor = slugify(text, seenAnchors);
      currentSection = { anchor, title: text, depth: heading[1].length };
      sections.push({
        page: relativePath,
        anchor,
        title: text,
        depth: heading[1].length,
        markers: anchor === "" ? ["empty-anchor"] : [],
      });
    }
  }

  return openDelimiter !== null ? relativePath : null;
}

function buildFence({ relativePath, slug, slice, flags, isShape, harnessLine, fencePosition, shapePosition, section, bodyLines }) {
  const markers = [];
  let classificationCandidate;
  let expectationKind = null;
  let declaredModes = [];

  if (!isShape) {
    classificationCandidate = "not-a-shape-fence";
  } else {
    // The harness's `runnable=` flag is the only classification signal the
    // corpus carries. It is not ADR-016 §5's classification: `runnable=false`
    // has no reason and no citation, so it cannot be `illustrative-only` yet.
    if (flags.runnable === "false" || flags.runnable === "deferred") {
      classificationCandidate = "illustrative-only";
      markers.push("no-illustrative-reason", "no-illustrative-citation");
      if (flags.runnable === "deferred") markers.push("deferred");
    } else {
      classificationCandidate = "runnable-gated";
      declaredModes = flags.fixture ? ["vm"] : ["vm", "jit"];
      if (flags.expected_fail !== undefined) {
        expectationKind = "diagnostic-substring";
        // §5: a negative example asserts a structured diagnostic identity and
        // essential typed payload, not only a rendered sentence.
        markers.push("expectation-is-a-rendered-substring");
      } else if (flags.expected !== undefined) {
        expectationKind = "stdout";
      } else {
        expectationKind = "exit-success-and-vm-jit-equality";
        markers.push("no-declared-expected-value");
      }
    }
    if (!flags.declared_id) markers.push("missing-stable-identity");
  }

  if (!section) markers.push("no-owning-section");

  return {
    harness_id: isShape ? `${slice}__${slug}__${shapePosition}__L${harnessLine}.shape` : null,
    page: relativePath,
    slice,
    fence_position: fencePosition,
    language: flags.language,
    declared_id: flags.declared_id,
    section_anchor: section?.anchor ?? null,
    section_title: section?.title ?? null,
    classification_candidate: classificationCandidate,
    runnable_flag: flags.runnable,
    declared_modes: declaredModes,
    expectation_kind: expectationKind,
    ...(flags.fixture ? { fixture: flags.fixture } : {}),
    ...(flags.serve_sandbox ? { serve_sandbox: flags.serve_sandbox } : {}),
    ...(flags.cite ? { cite: flags.cite } : {}),
    body_lines: bodyLines,
    markers,
  };
}

// --- assembly -------------------------------------------------------------

export function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

const UNRESOLVED_GAPS = [
  {
    surface: "stable fence identities",
    reason:
      "ADR-016 §5 requires every Shape fence to carry a stable explicit identity and §3 forbids line numbers and ordinal positions in it. No fence in the corpus declares an `id=` token, and the committed extractor mints `<slice>__<page-slug>__<position>__L<line>.shape`, which is both. Every Shape fence therefore carries the missing-stable-identity marker.",
    owner_hint: "#116 — minting identities is a Book edit, not an inventory",
  },
  {
    surface: "illustrative-only reasons and citations",
    reason:
      "ADR-016 §5 requires a nonempty reason and an issue or semantic-authority citation on every illustrative fence, and ratchets the set. The corpus expresses non-execution as a bare `runnable=false`, which is the shape §5's rejected-alternatives list names as hiding a broken implementation behind documentation. Every such fence carries the no-illustrative-reason and no-illustrative-citation markers.",
    owner_hint: "#116 — each fence needs a reviewed reason, which is a judgement per fence",
  },
  {
    surface: "structured negative expectations",
    reason:
      "ADR-016 §5 requires a negative example to assert a structured diagnostic identity and essential typed payload rather than a mutable rendered sentence. `expected-fail=` is a substring of rendered output, so every fence using it carries the expectation-is-a-rendered-substring marker.",
    owner_hint: "#116, blocked on the ADR-017 / R23 diagnostic catalog supplying stable identities",
  },
  {
    surface: "declared modes",
    reason:
      "The harness runs `runnable=true` fences under VM and JIT, and fixture-backed fences under VM only. Those are the modes the harness happens to run, not modes the fence declares, and ADR-016 §5 makes declared modes part of the classification. `declared_modes` therefore records the harness's behaviour, not a fence's claim.",
    owner_hint: "#116 — declaring modes per fence is part of classification",
  },
  {
    surface: "native-execution claims",
    reason:
      "ADR-016 §5 and R15 require a fence claiming native execution to name the exact function or realization and carry a NativeExecutionWitness. No fence-info token expresses that, so a prose native claim in the corpus is invisible to this scan.",
    owner_hint: "#116 plus the R15 / PERF-CLOSURE-NATIVE witness lane",
  },
];

export function buildInventory(shapeWeb) {
  const pages = listPages(shapeWeb);
  const sections = [];
  const fences = [];
  const unterminated = [];
  for (const page of pages) {
    const relative = page.slice(`${DOCS_ROOT}/`.length);
    const raw = git(shapeWeb, ["show", `HEAD:${page}`]);
    const unclosed = scanPage(relative, raw, sections, fences);
    if (unclosed) unterminated.push(unclosed);
  }

  const shapeFences = fences.filter((fence) => fence.language === "shape");
  const byLanguage = {};
  for (const fence of fences) {
    const key = fence.language ?? "(none)";
    byLanguage[key] = (byLanguage[key] ?? 0) + 1;
  }

  // Duplicate stable identities, once any exist. Duplicate anchors within a page
  // are already disambiguated by the slugger, so a duplicate here would be a
  // real collision across pages, which #113's contract forbids.
  const declaredIds = new Map();
  const duplicateDeclaredIds = [];
  for (const fence of shapeFences) {
    if (!fence.declared_id) continue;
    if (declaredIds.has(fence.declared_id)) {
      duplicateDeclaredIds.push({ declared_id: fence.declared_id, first: declaredIds.get(fence.declared_id), second: fence.page });
    } else {
      declaredIds.set(fence.declared_id, fence.page);
    }
  }

  const markerCounts = {};
  for (const fence of fences) {
    for (const marker of fence.markers) markerCounts[marker] = (markerCounts[marker] ?? 0) + 1;
  }

  // What the committed harness executes: run-book-truth-gate.mjs line 577 keeps
  // only `runnable === true && !deferred`. ADR-016 §6 rejects a curated subset,
  // so both numbers are reported and neither is called the universe.
  const gateExecuted = shapeFences.filter((fence) => fence.classification_candidate === "runnable-gated");

  return {
    inventory_version: 1,
    generated_by: "scripts/generate-adr011-012-book-fence-inventory.mjs",
    corpus: {
      repository: "shape-lang/shape-web",
      docs_root: DOCS_ROOT,
      read_from: "committed HEAD content via `git show`, not the working tree",
      revision_policy:
        "This inventory records no shape-web revision. ADR-016 §3 and §7 keep exact revisions in the external PairCandidate and attestation; a counterpart SHA committed here would be the reciprocal pin §7 exists to prevent. The rows' content hash is what pins it.",
    },
    counts: {
      pages: pages.length,
      sections: sections.length,
      fences_all_languages: fences.length,
      fences_by_language: Object.fromEntries(Object.entries(byLanguage).sort()),
      shape_fences: shapeFences.length,
      shape_runnable_gated_candidates: gateExecuted.length,
      shape_illustrative_only_candidates: shapeFences.length - gateExecuted.length,
      shape_deferred: shapeFences.filter((fence) => fence.runnable_flag === "deferred").length,
      shape_with_expected_value: shapeFences.filter((fence) => fence.expectation_kind === "stdout").length,
      shape_with_expected_fail: shapeFences.filter((fence) => fence.expectation_kind === "diagnostic-substring").length,
      shape_with_fixture: shapeFences.filter((fence) => "fixture" in fence).length,
      shape_with_declared_identity: shapeFences.filter((fence) => fence.declared_id).length,
    },
    executed_subset_note:
      "shape_runnable_gated_candidates is the set book/book-site/scripts/run-book-truth-gate.mjs actually executes (it keeps runnable === true && !deferred). It is a subset of shape_fences, which is itself a subset of fences_all_languages. ADR-016 §6 rejects a percentage over a curated subset, so all three are reported and none is called the universe on its own.",
    marker_counts: markerCounts,
    duplicate_declared_ids: duplicateDeclaredIds,
    unterminated_fence_pages: unterminated,
    unresolved_gaps: UNRESOLVED_GAPS,
    sections_sha256: crypto.createHash("sha256").update(canonicalJson(sections)).digest("hex"),
    fences_sha256: crypto.createHash("sha256").update(canonicalJson(fences)).digest("hex"),
    sections,
    fences,
  };
}

const isMain = import.meta.url === `file://${process.argv[1]}`;
if (isMain) {
  const argv = process.argv.slice(2);
  const shapeWebIndex = argv.indexOf("--shape-web");
  const shapeWeb = shapeWebIndex === -1 ? defaultShapeWeb : path.resolve(argv[shapeWebIndex + 1]);
  const inventory = buildInventory(shapeWeb);
  console.log(JSON.stringify(inventory.counts, null, 2));
  console.log("markers:", JSON.stringify(inventory.marker_counts, null, 2));
  console.log("unterminated:", inventory.unterminated_fence_pages);
}
