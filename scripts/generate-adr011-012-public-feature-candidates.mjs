#!/usr/bin/env node
//
// ADR-016 §2 / §3 / R19 — the public feature candidate inventory
// (#114, PF-INVENTORY).
//
// ADR-016 §2 requires the PublicFeatureManifest to hold an entry for every
// public language construct, annotation, standard-library callable or type,
// compiler-visible user behaviour, CLI workflow, LSP behaviour, execution
// provider surface, snapshot/resume operation and distributed operator
// workflow. §3 then says a complete inventory "may be implemented in bounded
// waves only after their exact stable rows and content hash are committed".
//
// This script produces those exact rows. It is a SCANNER, not a judgement: each
// row records where the surface was found and nothing about how mature it is.
// Deciding a row's status, required modes, semantic dimensions and evidence is
// classification work that ADR-016 §2 makes evidence-derived, and #114's
// acceptance criteria assign it to the P waves — so every row is emitted with
// its unresolved classification named rather than guessed. A scanner that
// guessed would be inventing the aspirational denominator the ADR exists to
// prevent.
//
// The scan rules are deliberately narrow and stated per source, because the
// value of a mechanical inventory is that a reviewer can re-derive it. Where a
// surface is not mechanically separable from internal machinery, the scan does
// not guess: it records the gap in `unresolved_scan_gaps` so the missing rows
// are visible as missing rather than absent.
//
// Usage:
//   node scripts/generate-adr011-012-public-feature-candidates.mjs [--write]
//
// Without --write it prints the row count per source and exits. With --write it
// regenerates docs/program/adr011-012/public-feature-candidates.json, whose
// diff is the review surface. check-adr011-012-public-feature-candidates.mjs
// verifies the committed file still matches this scanner and that the row set
// has not silently shrunk.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
export const repositoryRoot = path.resolve(scriptDirectory, "..");
export const inventoryPath = path.join(
  repositoryRoot,
  "docs/program/adr011-012/public-feature-candidates.json",
);

const GRAMMAR = "crates/shape-ast/src/shape.pest";
const STDLIB_ROOT = "crates/shape-runtime/stdlib-src";
const CLI_ARGS = "bin/shape-cli/src/cli_args.rs";
const LSP_SERVER = "tools/shape-lsp/src/server.rs";
const ABI = "crates/shape-abi-v1/src/lib.rs";

function read(relativePath) {
  return fs.readFileSync(path.join(repositoryRoot, relativePath), "utf8");
}

// A candidate_id is shaped exactly like a feature_id (ADR-016 §2), so a row
// promoted into the manifest keeps its identity rather than being renamed at the
// boundary — a rename there would be an identity migration before the identity
// had ever been published.
function identifier(value) {
  return value
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

function row({ family, name, publicName, authority, component, notes }) {
  return {
    candidate_id: `${family.replace(/^language-/, "language.").replace(/^stdlib-/, "stdlib.").replace(/^tooling-/, "tooling.").replace(/^runtime-/, "runtime.")}.${identifier(name)}`
      .replace(/\.\./g, "."),
    public_name: publicName,
    family,
    surface_authority: { kind: "source", reference: authority },
    owner: { repository: "shape-lang/shape", component },
    ...(notes ? { notes } : {}),
  };
}

// --- S1/S2: grammar alternatives ----------------------------------------

// The alternatives of `item` and `statement` are the grammar's own statement of
// what a user may write at those levels, so they are the public construct set by
// construction rather than by a reviewer's list. Rules reached only from inside
// another construct are not separately public and are not scanned.
function grammarAlternatives(grammar, ruleName) {
  const start = grammar.indexOf(`\n${ruleName} = {`);
  if (start === -1) throw new Error(`grammar rule ${ruleName} not found`);
  let depth = 0;
  let index = start + 1;
  for (; index < grammar.length; index += 1) {
    if (grammar[index] === "{") depth += 1;
    else if (grammar[index] === "}") {
      depth -= 1;
      if (depth === 0) break;
    }
  }
  const body = grammar.slice(grammar.indexOf("{", start) + 1, index);
  const names = [];
  for (const line of body.split("\n")) {
    const withoutComment = line.replace(/\/\/.*$/, "");
    for (const match of withoutComment.matchAll(/(?:^|\|)\s*([a-z][a-z0-9_]*)\??\s*(?:~|$|\|)/g)) {
      if (!names.includes(match[1])) names.push(match[1]);
    }
  }
  return names;
}

function scanGrammarItems(grammar) {
  // `item` is `doc_comment? ~ item_core`, so the alternatives live one level
  // down. `statement` appears among them; it is a level, not a construct, and
  // its own alternatives are scanned separately.
  return grammarAlternatives(grammar, "item_core")
    .filter((name) => name !== "statement")
    .map((name) =>
      row({
        family: "language-declarations",
        name,
        publicName: name.replace(/_/g, " "),
        authority: `${GRAMMAR} rule \`${name}\` (alternative of \`item_core\`)`,
        component: "parser — top-level declaration forms",
      }),
    );
}

function scanGrammarStatements(grammar) {
  return grammarAlternatives(grammar, "statement").map((name) =>
    row({
      family: "language-statements",
      name,
      publicName: name.replace(/_/g, " "),
      authority: `${GRAMMAR} rule \`${name}\` (alternative of \`statement\`)`,
      component: "parser — statement forms",
    }),
  );
}

// Operators are scanned from the grammar's `*_op` rules plus the operator
// literals of the precedence chain. Each distinct spelling is one row, because
// ADR-016 §2 makes the inventory member-complete even where a family shares
// documentation — `language.pipe-operator` is exactly such a row.
const OPERATOR_RULES = [
  "compound_assign_op", "assign_op", "fuzzy_op", "range_op", "comparison_op",
];

// Operators the precedence chain spells inline rather than through a named `_op`
// rule. Each is verified present in the grammar before a row is emitted, so this
// list cannot drift into naming an operator the language does not have.
const INLINE_OPERATORS = [
  ["|>", "pipe", "pipe_expr"],
  ["??", "null-coalescing", "null_coalesce_expr"],
  ["?", "error-propagation", "try_operator"],
  ["!!", "error-context", "context_expr"],
  ["&&", "logical-and", "and_expr"],
  ["and", "logical-and-word", "and_expr"],
  ["||", "logical-or", "or_expr"],
  ["or", "logical-or-word", "or_expr"],
  ["!", "logical-not", "unary_expr"],
  ["&", "bitwise-and", "bitwise_and_expr"],
  ["|", "bitwise-or", "bitwise_or_expr"],
  ["^", "bitwise-xor", "bitwise_xor_expr"],
  ["~", "bitwise-not", "unary_expr"],
  ["<<", "left-shift", "shift_expr"],
  [">>", "right-shift", "shift_expr"],
  ["+", "addition", "additive_expr"],
  ["-", "subtraction", "additive_expr"],
  ["*", "multiplication", "multiplicative_expr"],
  ["/", "division", "multiplicative_expr"],
  ["%", "remainder", "multiplicative_expr"],
  ["**", "exponentiation", "exponential_expr"],
  ["as", "type-assertion", "as_keyword"],
  ["instanceof", "instance-test", "comparison_expr"],
];

const OPERATOR_NAMES = new Map([
  ["=", "assignment"], ["+=", "add-assign"], ["-=", "subtract-assign"],
  ["*=", "multiply-assign"], ["/=", "divide-assign"], ["%=", "remainder-assign"],
  ["**=", "exponent-assign"], ["<<=", "left-shift-assign"], [">>=", "right-shift-assign"],
  ["&=", "bitwise-and-assign"], ["|=", "bitwise-or-assign"], ["^=", "bitwise-xor-assign"],
  ["~=", "fuzzy-equal"], ["~<", "fuzzy-less"], ["~>", "fuzzy-greater"],
  ["..", "exclusive-range"], ["..=", "inclusive-range"],
  ["==", "equal"], ["!=", "not-equal"], ["<", "less-than"], [">", "greater-than"],
  ["<=", "less-or-equal"], [">=", "greater-or-equal"],
  ["approaching", "approaching"],
]);

function scanGrammarOperators(grammar) {
  const rows = [];
  const seen = new Set();
  const emit = (spelling, name, authority) => {
    if (seen.has(spelling)) return;
    seen.add(spelling);
    rows.push(
      row({
        family: "language-operators",
        name,
        publicName: `${name.replace(/-/g, " ")} operator (${spelling})`,
        authority,
        component: "language operators — parser, bytecode compiler, MIR lowering",
      }),
    );
  };

  for (const ruleName of OPERATOR_RULES) {
    const match = grammar.match(new RegExp(`\\n${ruleName} = \\{([^}]*)\\}`));
    if (!match) throw new Error(`grammar operator rule ${ruleName} not found`);
    for (const literal of match[1].matchAll(/"([^"]+)"/g)) {
      const spelling = literal[1];
      const name = OPERATOR_NAMES.get(spelling);
      if (!name) throw new Error(`operator ${spelling} in ${ruleName} has no recorded name`);
      emit(spelling, name, `${GRAMMAR} rule \`${ruleName}\` literal "${spelling}"`);
    }
  }

  for (const [spelling, name, ruleName] of INLINE_OPERATORS) {
    if (!new RegExp(`\\n${ruleName}\\b`).test(grammar)) {
      throw new Error(`inline operator ${spelling} names grammar rule ${ruleName}, which does not exist`);
    }
    emit(spelling, name, `${GRAMMAR} rule \`${ruleName}\` — operator "${spelling}"`);
  }

  return rows;
}

// --- S3: stdlib exports ---------------------------------------------------

// `pub` is the stdlib's own export marker, so it is the boundary between what a
// user can call and what a module keeps to itself. A non-`pub` declaration is
// not a public surface and is not scanned. Traits are scanned without requiring
// `pub` because the stdlib declares them unmarked and they are nonetheless
// user-implementable.
function scanStdlibExports() {
  const rows = [];
  const walk = (directory) => {
    for (const entry of fs.readdirSync(path.join(repositoryRoot, directory)).sort()) {
      const relative = `${directory}/${entry}`;
      if (fs.statSync(path.join(repositoryRoot, relative)).isDirectory()) walk(relative);
      else if (entry.endsWith(".shape")) scanStdlibModule(relative, rows);
    }
  };
  walk(STDLIB_ROOT);
  return rows;
}

function scanStdlibModule(relativePath, rows) {
  const source = read(relativePath);
  const moduleMatch = source.match(/^\/\/\/ @module ([A-Za-z0-9_:]+)/m);
  const modulePath = moduleMatch
    ? moduleMatch[1]
    : relativePath.slice(`${STDLIB_ROOT}/`.length).replace(/\.shape$/, "").replace(/\//g, "::");
  const moduleId = identifier(modulePath.replace(/^std::/, "").replace(/::/g, "-"));

  const patterns = [
    [/^[ \t]*pub[ \t]+(?:async[ \t]+|comptime[ \t]+)*fn[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, "callable", "fn"],
    [/^[ \t]*pub[ \t]+type[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, "type", "type"],
    [/^[ \t]*pub[ \t]+enum[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, "type", "enum"],
    [/^[ \t]*pub[ \t]+const[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, "constant", "const"],
    [/^[ \t]*(?:pub[ \t]+)?trait[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, "trait", "trait"],
    [/^[ \t]*(?:pub[ \t]+)?annotation[ \t]+([A-Za-z_][A-Za-z0-9_]*)/gm, "annotation", "annotation"],
  ];

  for (const [pattern, kind, keyword] of patterns) {
    for (const match of source.matchAll(pattern)) {
      const name = match[1];
      const family = kind === "annotation" ? "language-annotations" : `stdlib-${kind}s`;
      rows.push(
        row({
          family,
          name: kind === "annotation" ? name : `${moduleId}-${name}`,
          publicName: kind === "annotation" ? `@${name}` : `${modulePath}::${name}`,
          authority: `${relativePath} \`${keyword} ${name}\``,
          component: `standard library — ${modulePath}`,
        }),
      );
    }
  }
}

// --- S4: CLI commands -----------------------------------------------------

// Every user-invocable subcommand, from the clap enums themselves. A workflow a
// user can run is a public feature under ADR-016 §2 whether or not the Book
// currently mentions it.
function scanCliCommands() {
  const source = read(CLI_ARGS);
  const rows = [];
  const enums = [...source.matchAll(/pub enum ((?:Commands|[A-Za-z]+Action)) \{([\s\S]*?)\n\}/g)];
  if (enums.length === 0) throw new Error("no clap command enums found");
  for (const [, enumName, body] of enums) {
    for (const match of body.matchAll(/^ {4}([A-Z][A-Za-z0-9]*)[\s,({]/gm)) {
      const variant = match[1];
      const group = enumName === "Commands" ? "" : `${identifier(enumName.replace(/Action$/, ""))}-`;
      rows.push(
        row({
          family: "tooling-cli",
          name: `${group}${variant}`,
          publicName: `shape ${group.replace(/-$/, " ")}${identifier(variant)}`.replace(/\s+/g, " "),
          authority: `${CLI_ARGS} \`${enumName}::${variant}\``,
          component: "command-line interface",
        }),
      );
    }
  }
  return rows;
}

// --- S5: LSP capabilities -------------------------------------------------

// The capabilities the server actually advertises. `prepare_provider` and
// `resolve_provider` are excluded because they are fields of RenameOptions and
// CompletionOptions rather than server capabilities: they refine a capability
// already scanned, so a row for each would double-count one behaviour.
const LSP_SUBFIELDS = new Set(["prepare_provider", "resolve_provider"]);

function scanLspCapabilities() {
  const source = read(LSP_SERVER);
  const names = [...new Set([...source.matchAll(/([a-z_]+_provider):/g)].map((match) => match[1]))]
    .filter((name) => !LSP_SUBFIELDS.has(name))
    .sort();
  if (names.length === 0) throw new Error("no LSP capabilities found");
  return names.map((name) =>
    row({
      family: "tooling-lsp",
      name: name.replace(/_provider$/, ""),
      publicName: `LSP ${name.replace(/_provider$/, "").replace(/_/g, " ")}`,
      authority: `${LSP_SERVER} \`ServerCapabilities::${name}\``,
      component: "language server",
    }),
  );
}

// --- S6: permissions ------------------------------------------------------

// A permission is user-observable: it is what a program is refused, what a
// sandbox grants, and what a content hash is computed over.
function scanPermissions() {
  const source = read(ABI);
  const start = source.indexOf("pub enum Permission {");
  if (start === -1) throw new Error("Permission enum not found");
  const body = source.slice(start, source.indexOf("\n}", start));
  const names = [...body.matchAll(/^ {4}([A-Z][A-Za-z0-9]*)/gm)].map((match) => match[1]);
  return names.map((name) =>
    row({
      family: "runtime-permissions",
      name,
      publicName: `${name} permission`,
      authority: `${ABI} \`Permission::${name}\``,
      component: "capability model — compile-time derivation, runtime gating, sandbox grants",
    }),
  );
}

// --- assembly -------------------------------------------------------------

// What the scan cannot mechanically separate from internal machinery. ADR-016 §2
// makes an ambiguous inventory a blocking gap, so these are recorded as named
// holes rather than left as silence — the rows are missing, and the inventory
// says so.
const UNRESOLVED_SCAN_GAPS = [
  {
    surface: "builtin type methods",
    reason:
      "Methods on Array, String, HashMap, DateTime and friends are registered through compile-time PHF maps in crates/shape-vm/src/executor/objects/method_registry.rs and through stdlib impl blocks. The PHF construction is not a flat literal table this scanner can read without evaluating the macro, so no rows are emitted and the count is unknown.",
    owner_hint: "a P wave that can enumerate the registry from the built binary or from a generated table",
  },
  {
    surface: "compiler diagnostics",
    reason:
      "ADR-016 §2 makes compiler-visible user behaviour a public surface and ADR-016 §10 gives diagnostics stable concept identities, but there is no single catalog to scan: identities are minted at emitter sites. R23 makes that catalog an ADR-017 deliverable.",
    owner_hint: "the ADR-017 / R23 diagnostic-catalog lane",
  },
  {
    surface: "polyglot and foreign-target surfaces",
    reason:
      "`fn python` / `fn typescript` / `extern C fn` are scanned as grammar constructs, but the per-target callable surfaces they expose are declared by the extension crates rather than by a Shape-side table.",
    owner_hint: "the ADR-019 / R25 polyglot lane (#163, #164)",
  },
  {
    surface: "execution providers, snapshot/resume operations and distributed operator workflows",
    reason:
      "ADR-016 §2 names these explicitly. `shape serve`, `shape wire-serve` and the snapshot subcommands are scanned as CLI rows, but the operator workflows of §4 are procedures rather than symbols and have no enumerable declaration site.",
    owner_hint: "the ADR-015 / recovery-and-operations lane",
  },
  {
    surface: "builtin global callables",
    reason:
      "Globals such as `print` are declared in crates/shape-runtime/stdlib-src/core/intrinsics.shape and are scanned when marked `pub`, but the BuiltinFunction registry also admits identities with no Shape-side declaration. Those are not separable from internal intrinsics without the ADR-011 IntrinsicCatalog.",
    owner_hint: "the ADR-011 / R18 IntrinsicCatalog rollout (#110)",
  },
];

// Canonical JSON with sorted keys, matching the sibling manifest gates, so that
// reformatting the file does not read as a different row set.
function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

export function buildInventory() {
  const grammar = read(GRAMMAR);
  const sources = [
    ["grammar-declarations", `${GRAMMAR}: alternatives of the \`item\` rule`, scanGrammarItems(grammar)],
    ["grammar-statements", `${GRAMMAR}: alternatives of the \`statement\` rule`, scanGrammarStatements(grammar)],
    ["grammar-operators", `${GRAMMAR}: \`*_op\` rule literals plus the precedence chain's inline operator spellings`, scanGrammarOperators(grammar)],
    ["stdlib-exports", `${STDLIB_ROOT}/**/*.shape: \`pub fn|type|enum|const\`, plus \`trait\` and \`annotation\` declarations`, scanStdlibExports()],
    ["cli-commands", `${CLI_ARGS}: variants of \`Commands\` and the \`*Action\` subcommand enums`, scanCliCommands()],
    ["lsp-capabilities", `${LSP_SERVER}: \`*_provider\` fields of the advertised ServerCapabilities`, scanLspCapabilities()],
    ["permissions", `${ABI}: variants of \`Permission\``, scanPermissions()],
  ];

  const rows = [];
  const duplicates = [];
  const byId = new Map();
  for (const [, , sourceRows] of sources) {
    for (const candidate of sourceRows) {
      const existing = byId.get(candidate.candidate_id);
      if (existing) {
        duplicates.push({
          candidate_id: candidate.candidate_id,
          first: existing.surface_authority.reference,
          second: candidate.surface_authority.reference,
        });
        continue;
      }
      byId.set(candidate.candidate_id, candidate);
      rows.push(candidate);
    }
  }
  rows.sort((left, right) => (left.candidate_id < right.candidate_id ? -1 : left.candidate_id > right.candidate_id ? 1 : 0));

  const inventory = {
    inventory_version: 1,
    generated_by: "scripts/generate-adr011-012-public-feature-candidates.mjs",
    unresolved_classification: [
      "status",
      "required_modes",
      "required_evidence_classes",
      "required_semantic_dimensions",
      "distributed_semantics_required",
    ],
    sources: sources.map(([id, scanRule, sourceRows]) => ({
      id,
      scan_rule: scanRule,
      count: sourceRows.length,
    })),
    unresolved_scan_gaps: UNRESOLVED_SCAN_GAPS,
    duplicate_candidate_ids: duplicates,
    count: rows.length,
    rows_sha256: crypto.createHash("sha256").update(canonicalJson(rows)).digest("hex"),
    rows,
  };
  return inventory;
}

export { canonicalJson };

const isMain = import.meta.url === `file://${process.argv[1]}`;
if (isMain) {
  const inventory = buildInventory();
  for (const source of inventory.sources) {
    console.log(`  ${String(source.count).padStart(4)}  ${source.id}`);
  }
  console.log(`  ${String(inventory.count).padStart(4)}  TOTAL (${inventory.duplicate_candidate_ids.length} duplicate id(s) folded)`);
  console.log(`  rows_sha256 ${inventory.rows_sha256}`);
  console.log(`  ${inventory.unresolved_scan_gaps.length} unresolved scan gap(s): ${inventory.unresolved_scan_gaps.map((gap) => gap.surface).join("; ")}`);
  if (process.argv.includes("--write")) {
    fs.writeFileSync(inventoryPath, `${JSON.stringify(inventory, null, 2)}\n`);
    console.log(`Wrote ${path.relative(repositoryRoot, inventoryPath)}`);
  }
}
