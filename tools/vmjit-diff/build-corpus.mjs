#!/usr/bin/env node
/**
 * build-corpus.mjs — materialize the VM-vs-JIT differential corpus (WF-0B).
 *
 * Sources, in tier order:
 *
 *   1. tier "book"       — every `runnable === true` ```shape fence in the
 *      book (shape-web/book/book-site/src/content/docs/**\/*.mdx), extracted
 *      by re-using the committed canonical extractor
 *      `book-site/scripts/extract-shape-snippets.mjs` (snippet-id convention
 *      `<slice>__<page-slug>__<pos>__L<line>.shape` — traceable to page+line).
 *   2. tier "acceptance" — the self-asserting machine-proofable programs at
 *      docs/cluster-audits/v0.3.3-book-acceptance/programs/<slice>/*.shape.
 *      These define `fn main()` which script mode does NOT auto-invoke, so a
 *      trailing `main()` call is appended when the file defines `fn main`
 *      and never calls it (transform recorded in the manifest).
 *   3. tier "synthetic"  — hand-written known-divergence repros from
 *      tools/vmjit-diff/synthetic/ (copied verbatim).
 *
 * Skip policy: ONLY programs whose content matches the network / user-input
 * heuristics below are skipped, and every skip is logged to SKIPPED.md with
 * the matched pattern — no silent caps.
 *
 * Outputs (all under tools/vmjit-diff/):
 *   corpus/<id>.shape       one file per program
 *   corpus/manifest.json    provenance manifest (id, tier, source, transform)
 *   SKIPPED.md              skip log
 *
 * Usage:
 *   node tools/vmjit-diff/build-corpus.mjs [--book-site <dir>] [--out <dir>]
 */

import {
  readFileSync,
  writeFileSync,
  mkdirSync,
  readdirSync,
  statSync,
  rmSync,
  existsSync,
  mkdtempSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve, join, dirname, basename, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { tmpdir } from 'node:os';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..', '..');

// ---------- args -------------------------------------------------------------

function parseArgs(argv) {
  const out = { bookSite: null, outDir: join(__dirname, 'corpus') };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--book-site') out.bookSite = argv[++i];
    else if (a === '--out') out.outDir = resolve(argv[++i]);
    else if (a === '--help' || a === '-h') {
      console.log('Usage: build-corpus.mjs [--book-site <dir>] [--out <dir>]');
      process.exit(0);
    }
  }
  return out;
}

function findBookSite(explicit) {
  const candidates = explicit
    ? [explicit]
    : [
        // main-repo layout: shape/ next to shape-web/
        resolve(repoRoot, '..', 'shape-web', 'book', 'book-site'),
        // worktree layout: worktrees/<name>/ two levels below shape-lang/
        resolve(repoRoot, '..', '..', 'shape-web', 'book', 'book-site'),
      ];
  for (const c of candidates) {
    if (existsSync(join(c, 'scripts', 'extract-shape-snippets.mjs'))) return c;
  }
  throw new Error(
    `book-site not found; tried:\n  ${candidates.join('\n  ')}\nPass --book-site <dir>.`,
  );
}

// ---------- skip heuristics (network / user input only) ----------------------

const SKIP_PATTERNS = [
  { name: 'http-call', re: /\bhttp::(get|post|put|delete|request)\b|\bhttp\.(get|post|put|delete|request)\(/ },
  { name: 'net-connect', re: /\bnet::(connect|listen)\b|\bTcpListener\b|\bTcpStream\b|\bUdpSocket\b/ },
  { name: 'wire-transport', re: /\bwire::(serve|connect|dial)\b|\bquic::/ },
  { name: 'user-input', re: /\bread_line\b|\breadline\b|\bstdin\b|\bprompt\(/ },
];

/** Strip // line comments and /* *\/ block comments so that determinism
 *  notes like "no stdin, no live network" don't false-positive the skip
 *  heuristics. (Rough lexer — does not special-case comment markers inside
 *  string literals; acceptable for a skip heuristic that is logged, never
 *  silent.) */
function stripComments(content) {
  return content.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/[^\n]*/g, '');
}

function skipReason(content) {
  const code = stripComments(content);
  for (const p of SKIP_PATTERNS) {
    const m = code.match(p.re);
    if (m) return `${p.name}: matched \`${m[0]}\``;
  }
  return null;
}

// ---------- tier 1: book snippets via the canonical extractor ----------------

function extractBookSnippets(bookSite) {
  const tmp = mkdtempSync(join(tmpdir(), 'vmjit-diff-extract-'));
  const res = spawnSync(
    process.execPath,
    [join(bookSite, 'scripts', 'extract-shape-snippets.mjs'), '--out', tmp],
    { cwd: bookSite, encoding: 'utf-8' },
  );
  if (res.status !== 0) {
    throw new Error(
      `extract-shape-snippets.mjs failed (exit ${res.status}):\n${res.stdout}\n${res.stderr}`,
    );
  }
  const manifest = JSON.parse(readFileSync(join(tmp, 'manifest.json'), 'utf-8'));
  return { tmp, manifest };
}

// ---------- tier 2: acceptance programs --------------------------------------

function findAcceptancePrograms() {
  const root = join(
    repoRoot,
    'docs',
    'cluster-audits',
    'v0.3.3-book-acceptance',
    'programs',
  );
  if (!existsSync(root)) return { root, files: [] };
  const files = [];
  for (const slice of readdirSync(root).sort()) {
    const dir = join(root, slice);
    if (!statSync(dir).isDirectory()) continue;
    for (const f of readdirSync(dir).sort()) {
      if (f.endsWith('.shape')) files.push({ slice, file: join(dir, f) });
    }
  }
  return { root, files };
}

/** Append `main()` iff the program defines fn main and never invokes it. */
function ensureMainInvoked(content) {
  const definesMain = /\bfn\s+main\s*\(/.test(content);
  const invokesMain = /^\s*main\s*\(\s*\)/m.test(content);
  if (definesMain && !invokesMain) {
    return {
      content: content + (content.endsWith('\n') ? '' : '\n') + '\nmain()\n',
      transform: 'appended top-level main() call (script mode does not auto-invoke fn main)',
    };
  }
  return { content, transform: null };
}

// ---------- main --------------------------------------------------------------

const args = parseArgs(process.argv.slice(2));
const bookSite = findBookSite(args.bookSite);
const outDir = args.outDir;

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

const entries = [];
const skipped = [];

// Tier 1: book
const { tmp, manifest: bookManifest } = extractBookSnippets(bookSite);
let bookRunnable = 0;
for (const s of bookManifest.snippets) {
  if (!s.runnable) continue;
  bookRunnable++;
  const content = readFileSync(join(tmp, s.id), 'utf-8');
  const reason = skipReason(content);
  const source = `${s.page}:L${s.line} (fence #${s.position}, slice ${s.slice})`;
  if (reason) {
    skipped.push({ id: s.id, tier: 'book', source, reason });
    continue;
  }
  writeFileSync(join(outDir, s.id), content);
  entries.push({ id: s.id, tier: 'book', source });
}

// Tier 2: acceptance
const acc = findAcceptancePrograms();
for (const { slice, file } of acc.files) {
  const id = `ACC__${slice}__${basename(file)}`;
  const raw = readFileSync(file, 'utf-8');
  const source = relative(repoRoot, file);
  const reason = skipReason(raw);
  if (reason) {
    skipped.push({ id, tier: 'acceptance', source, reason });
    continue;
  }
  const { content, transform } = ensureMainInvoked(raw);
  writeFileSync(join(outDir, id), content);
  const entry = { id, tier: 'acceptance', source };
  if (transform) entry.transform = transform;
  entries.push(entry);
}

// Tier 3: synthetic
const synDir = join(__dirname, 'synthetic');
if (existsSync(synDir)) {
  for (const f of readdirSync(synDir).sort()) {
    if (!f.endsWith('.shape')) continue;
    writeFileSync(join(outDir, f), readFileSync(join(synDir, f), 'utf-8'));
    entries.push({
      id: f,
      tier: 'synthetic',
      source: relative(repoRoot, join(synDir, f)),
    });
  }
}

rmSync(tmp, { recursive: true, force: true });

// Manifest + skip log
const corpusManifest = {
  version: 1,
  generatedAt: new Date().toISOString(),
  bookSite: relative(repoRoot, bookSite),
  bookRunnableTotal: bookRunnable,
  counts: entries.reduce((a, e) => ((a[e.tier] = (a[e.tier] ?? 0) + 1), a), {}),
  skippedCount: skipped.length,
  entries,
};
writeFileSync(join(outDir, 'manifest.json'), JSON.stringify(corpusManifest, null, 2) + '\n');

const skippedMd = [
  '# vmjit-diff corpus — skipped programs',
  '',
  'Programs excluded from `tools/vmjit-diff/corpus/` because they require',
  'network access or user input (the ONLY allowed skip reasons; see',
  'build-corpus.mjs SKIP_PATTERNS). Regenerated by build-corpus.mjs — do not',
  'edit by hand.',
  '',
  `Generated: ${corpusManifest.generatedAt}`,
  '',
  skipped.length === 0
    ? '_No programs skipped: no runnable book example or acceptance program matched the network/user-input heuristics._'
    : ['| id | tier | source | reason |', '|---|---|---|---|']
        .concat(skipped.map((s) => `| ${s.id} | ${s.tier} | ${s.source} | ${s.reason} |`))
        .join('\n'),
  '',
].join('\n');
writeFileSync(join(__dirname, 'SKIPPED.md'), skippedMd);

console.log(`corpus: ${entries.length} programs -> ${relative(process.cwd(), outDir)}`);
console.log(`  by tier: ${JSON.stringify(corpusManifest.counts)}`);
console.log(`  book runnable fences: ${bookRunnable} (skipped ${skipped.filter((s) => s.tier === 'book').length})`);
console.log(`  skipped total: ${skipped.length} (see tools/vmjit-diff/SKIPPED.md)`);
