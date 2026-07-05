#!/usr/bin/env node
/**
 * run-diff.mjs — VM-vs-JIT differential runner (WF-0B).
 *
 * For every program in the corpus, runs
 *
 *     <shape-bin> run --mode vm  <file>
 *     <shape-bin> run --mode jit <file>
 *
 * each under a timeout (default 10s), then compares stdout bytes + exit code
 * and classifies:
 *
 *   MATCH     stdout identical AND exit code identical (incl. both failing
 *             identically — deterministic agreement is a match)
 *   VM_FAIL   vm exits non-zero, jit exits 0
 *   JIT_FAIL  jit exits non-zero, vm exits 0
 *   DIVERGED  stdout differs, or both non-zero with different exit codes
 *   TIMEOUT   either mode exceeded the timeout
 *
 * Known-red allowlist (tools/vmjit-diff/known-red.json) pins expected
 * divergences by corpus id: a non-MATCH on a known-red id is reported but
 * does not fail the run; a MATCH on a known-red id is flagged as
 * "known-red now matching" (candidate for removal from the allowlist).
 *
 * Reports: JSON (machine) + Markdown (human) — every non-MATCH is listed
 * with both stdouts/stderrs truncated. Exit code: 0 = all MATCH or known-red,
 * 1 = at least one unexpected non-MATCH, 2 = harness error.
 *
 * Usage:
 *   node tools/vmjit-diff/run-diff.mjs
 *     [--shape-bin <path>]      (default: $SHAPE_BIN, else <repo>/target/release/shape)
 *     [--corpus <dir>]          (default: tools/vmjit-diff/corpus)
 *     [--report <path.json>]    (default: tools/vmjit-diff/reports/report.json;
 *                                a sibling .md is always written)
 *     [--timeout-secs <n>]      (default: 10)
 *     [--tier book|acceptance|synthetic]
 *     [--filter <substring>]    (only ids containing the substring)
 *     [--limit <n>]             (first n after tier/filter, manifest order)
 *     [--progress <path.jsonl>] (default: tools/vmjit-diff/reports/progress.jsonl)
 *     [--fresh]                 (ignore + truncate any existing progress file)
 *
 * Resumability: every completed program is appended to the progress file
 * (JSONL; first line is a meta record pinning binary + corpus). On start,
 * programs already present in the progress file are skipped — so a killed
 * run (e.g. an outer 10-minute timeout) picks up where it left off on the
 * next invocation. Resume is the DEFAULT; pass --fresh to restart. A
 * progress file recorded against a different binary or corpus is discarded
 * automatically. The final report is always rebuilt from the full combined
 * result set; known-red classification is recomputed at report time.
 */

import {
  readFileSync,
  writeFileSync,
  appendFileSync,
  mkdirSync,
  existsSync,
  rmSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import { resolve, join, dirname, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..', '..');

// ---------- args -------------------------------------------------------------

function parseArgs(argv) {
  const out = {
    shapeBin: process.env.SHAPE_BIN || null,
    corpus: join(__dirname, 'corpus'),
    report: join(__dirname, 'reports', 'report.json'),
    timeoutSecs: 10,
    tier: null,
    filter: null,
    limit: null,
    knownRed: join(__dirname, 'known-red.json'),
    progress: join(__dirname, 'reports', 'progress.jsonl'),
    fresh: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--shape-bin') out.shapeBin = argv[++i];
    else if (a === '--corpus') out.corpus = resolve(argv[++i]);
    else if (a === '--report') out.report = resolve(argv[++i]);
    else if (a === '--timeout-secs') out.timeoutSecs = Number(argv[++i]);
    else if (a === '--tier') out.tier = argv[++i];
    else if (a === '--filter') out.filter = argv[++i];
    else if (a === '--limit') out.limit = Number(argv[++i]);
    else if (a === '--known-red') out.knownRed = resolve(argv[++i]);
    else if (a === '--progress') out.progress = resolve(argv[++i]);
    else if (a === '--fresh') out.fresh = true;
    else if (a === '--help' || a === '-h') {
      console.log(readFileSync(fileURLToPath(import.meta.url), 'utf-8').split('*/')[0]);
      process.exit(0);
    } else {
      console.error(`unknown arg: ${a}`);
      process.exit(2);
    }
  }
  return out;
}

function fail(msg) {
  console.error(`[vmjit-diff] harness error: ${msg}`);
  process.exit(2);
}

// ---------- run one mode -------------------------------------------------------

function runMode(bin, mode, file, timeoutSecs, cwd) {
  const t0 = process.hrtime.bigint();
  const res = spawnSync(bin, ['run', '--mode', mode, file], {
    cwd,
    encoding: 'utf-8',
    timeout: timeoutSecs * 1000,
    killSignal: 'SIGKILL',
    maxBuffer: 16 * 1024 * 1024,
  });
  const ms = Number(process.hrtime.bigint() - t0) / 1e6;
  const timedOut =
    (res.error && res.error.code === 'ETIMEDOUT') || res.signal === 'SIGKILL';
  return {
    mode,
    exit: res.status,
    signal: res.signal ?? null,
    timedOut,
    stdout: res.stdout ?? '',
    stderr: res.stderr ?? '',
    ms: Math.round(ms),
    spawnError: res.error && res.error.code !== 'ETIMEDOUT' ? String(res.error) : null,
  };
}

// ---------- classification -----------------------------------------------------

function classify(vm, jit) {
  if (vm.timedOut || jit.timedOut) return 'TIMEOUT';
  const vmOk = vm.exit === 0;
  const jitOk = jit.exit === 0;
  if (!vmOk && jitOk) return 'VM_FAIL';
  if (vmOk && !jitOk) return 'JIT_FAIL';
  if (vm.stdout === jit.stdout && vm.exit === jit.exit) return 'MATCH';
  return 'DIVERGED';
}

function truncate(s, maxLines = 6, maxChars = 500) {
  if (!s) return '';
  let out = s.split('\n').slice(0, maxLines).join('\n');
  if (out.length > maxChars) out = out.slice(0, maxChars);
  const cut = out.length < s.length;
  return cut ? out + `\n… [truncated, ${s.length} bytes total]` : out;
}

// ---------- main -----------------------------------------------------------------

const args = parseArgs(process.argv.slice(2));

// Locate binary. Build hint mirrors CLAUDE.md/devenv convention.
let bin = args.shapeBin;
if (!bin) {
  const candidate = join(repoRoot, 'target', 'release', 'shape');
  if (existsSync(candidate)) bin = candidate;
  else
    fail(
      `no shape binary: set SHAPE_BIN / --shape-bin, or build one:\n` +
        `  direnv exec /home/dev/dev/shape-lang cargo build --release --bin shape\n` +
        `  (expected at ${candidate})`,
    );
}
bin = resolve(bin);
if (!existsSync(bin)) fail(`shape binary not found: ${bin}`);

const manifestPath = join(args.corpus, 'manifest.json');
if (!existsSync(manifestPath))
  fail(`corpus manifest missing: ${manifestPath} — run build-corpus.mjs first`);
const corpusManifest = JSON.parse(readFileSync(manifestPath, 'utf-8'));

let knownRed = { entries: [] };
if (existsSync(args.knownRed))
  knownRed = JSON.parse(readFileSync(args.knownRed, 'utf-8'));
const knownById = new Map(knownRed.entries.map((e) => [e.id, e]));

let programs = corpusManifest.entries;
if (args.tier) programs = programs.filter((e) => e.tier === args.tier);
if (args.filter) programs = programs.filter((e) => e.id.includes(args.filter));
if (args.limit != null) programs = programs.slice(0, args.limit);
if (programs.length === 0) fail('no programs selected');

const binVersion = spawnSync(bin, ['--version'], { encoding: 'utf-8' }).stdout?.trim() ?? 'unknown';
console.log(`[vmjit-diff] binary: ${bin} (${binVersion})`);

// ---------- resume: load progress file ------------------------------------------
// progress.jsonl: first line {type:"meta", bin, binVersion, corpus}; then one
// {type:"result", ...} line per completed program. Raw results only — known-red
// status is recomputed at report time so allowlist edits between resumed calls
// take effect on the whole result set.

const corpusKey = relative(repoRoot, args.corpus);
const resumed = new Map(); // id -> raw result
let progressValid = false;
if (args.fresh && existsSync(args.progress)) {
  rmSync(args.progress);
  console.log(`[vmjit-diff] --fresh: removed ${args.progress}`);
}
if (existsSync(args.progress)) {
  const lines = readFileSync(args.progress, 'utf-8').split('\n').filter((l) => l.trim());
  let meta = null;
  try {
    meta = lines.length > 0 ? JSON.parse(lines[0]) : null;
  } catch {
    meta = null;
  }
  if (meta && meta.type === 'meta' && meta.bin === bin && meta.corpus === corpusKey) {
    progressValid = true;
    for (const line of lines.slice(1)) {
      let rec;
      try {
        rec = JSON.parse(line);
      } catch {
        continue; // torn tail write from a killed run — redo that program
      }
      if (rec.type === 'result' && rec.id) resumed.set(rec.id, rec);
    }
    console.log(`[vmjit-diff] resume: ${resumed.size} completed results loaded from ${args.progress}`);
  } else {
    console.log(`[vmjit-diff] progress file is for a different binary/corpus — starting fresh`);
    rmSync(args.progress);
  }
}
if (!progressValid) {
  mkdirSync(dirname(args.progress), { recursive: true });
  writeFileSync(
    args.progress,
    JSON.stringify({ type: 'meta', bin, binVersion, corpus: corpusKey, startedAt: new Date().toISOString() }) + '\n',
  );
}

const pending = programs.filter((e) => !resumed.has(e.id));
console.log(
  `[vmjit-diff] corpus: ${args.corpus} — ${programs.length} selected, ${pending.length} to run (${programs.length - pending.length} resumed), timeout ${args.timeoutSecs}s per mode`,
);

for (const [i, entry] of pending.entries()) {
  const file = join(args.corpus, entry.id);
  if (!existsSync(file)) fail(`corpus file missing: ${file}`);
  const vm = runMode(bin, 'vm', file, args.timeoutSecs, args.corpus);
  const jit = runMode(bin, 'jit', file, args.timeoutSecs, args.corpus);
  if (vm.spawnError || jit.spawnError)
    fail(`spawn failed for ${entry.id}: ${vm.spawnError ?? jit.spawnError}`);
  const classification = classify(vm, jit);
  const isMatch = classification === 'MATCH';

  const rec = {
    type: 'result',
    id: entry.id,
    tier: entry.tier,
    source: entry.source,
    classification,
    vm: { exit: vm.exit, signal: vm.signal, timedOut: vm.timedOut, ms: vm.ms },
    jit: { exit: jit.exit, signal: jit.signal, timedOut: jit.timedOut, ms: jit.ms },
    // Full streams kept only for non-MATCH to keep the report small.
    ...(isMatch
      ? {}
      : {
          vmStdout: truncate(vm.stdout),
          vmStderr: truncate(vm.stderr),
          jitStdout: truncate(jit.stdout),
          jitStderr: truncate(jit.stderr),
        }),
  };
  resumed.set(entry.id, rec);
  appendFileSync(args.progress, JSON.stringify(rec) + '\n');

  const known = knownById.get(entry.id) ?? null;
  const marker = isMatch ? '.' : ` ${classification}${known ? '(known)' : ''} ${entry.id} `;
  process.stdout.write(marker);
  if ((i + 1) % 80 === 0) process.stdout.write('\n');
}
process.stdout.write('\n');

// ---------- combine + classify against known-red --------------------------------

const results = [];
const counts = { MATCH: 0, DIVERGED: 0, VM_FAIL: 0, JIT_FAIL: 0, TIMEOUT: 0 };
let knownRedHits = 0;
let knownRedNowMatching = 0;
let unexpected = 0;

for (const entry of programs) {
  const rec = resumed.get(entry.id);
  if (!rec) fail(`internal: missing result for ${entry.id}`);
  const { type: _type, ...rest } = rec;
  const known = knownById.get(entry.id) ?? null;
  const isMatch = rec.classification === 'MATCH';
  counts[rec.classification]++;
  if (known && !isMatch) knownRedHits++;
  if (known && isMatch) knownRedNowMatching++;
  if (!known && !isMatch) unexpected++;
  results.push({
    ...rest,
    knownRed: known ? { class: known.class, reason: known.reason } : null,
  });
}

const report = {
  version: 1,
  generatedAt: new Date().toISOString(),
  shapeBin: bin,
  shapeBinVersion: binVersion,
  corpus: relative(repoRoot, args.corpus),
  corpusGeneratedAt: corpusManifest.generatedAt,
  timeoutSecs: args.timeoutSecs,
  selection: { tier: args.tier, filter: args.filter, limit: args.limit },
  progressFile: relative(repoRoot, args.progress),
  resumedFromProgress: programs.length - pending.length,
  total: programs.length,
  counts,
  knownRedHits,
  knownRedNowMatching,
  unexpectedNonMatch: unexpected,
  results,
};

mkdirSync(dirname(args.report), { recursive: true });
writeFileSync(args.report, JSON.stringify(report, null, 2) + '\n');

// ---------- markdown report ----------------------------------------------------

const nonMatch = results.filter((r) => r.classification !== 'MATCH');
const md = [];
md.push('# VM-vs-JIT differential report');
md.push('');
md.push(`- generated: ${report.generatedAt}`);
md.push(`- binary: \`${bin}\` (${binVersion})`);
md.push(`- corpus: ${report.corpus} (generated ${report.corpusGeneratedAt})`);
md.push(`- programs run: ${report.total}, timeout ${args.timeoutSecs}s/mode`);
md.push(`- counts: ${Object.entries(counts).map(([k, v]) => `${k}=${v}`).join(', ')}`);
md.push(`- known-red hits: ${knownRedHits}; known-red now matching: ${knownRedNowMatching}; **unexpected non-MATCH: ${unexpected}**`);
md.push('');
if (nonMatch.length === 0) {
  md.push('All programs MATCH.');
} else {
  md.push('## Non-MATCH programs');
  for (const r of nonMatch) {
    md.push('');
    md.push(`### ${r.classification}${r.knownRed ? ` (known-red: ${r.knownRed.class})` : ''} — \`${r.id}\``);
    md.push(`- tier: ${r.tier}; source: ${r.source}`);
    if (r.knownRed) md.push(`- known-red reason: ${r.knownRed.reason}`);
    md.push(`- vm: exit=${r.vm.exit} signal=${r.vm.signal} timedOut=${r.vm.timedOut} (${r.vm.ms}ms)`);
    md.push(`- jit: exit=${r.jit.exit} signal=${r.jit.signal} timedOut=${r.jit.timedOut} (${r.jit.ms}ms)`);
    for (const [label, text] of [
      ['vm stdout', r.vmStdout],
      ['vm stderr', r.vmStderr],
      ['jit stdout', r.jitStdout],
      ['jit stderr', r.jitStderr],
    ]) {
      if (text) md.push(`- ${label}:`, '```', text, '```');
    }
  }
}
const nowMatching = results.filter((r) => r.classification === 'MATCH' && knownById.has(r.id));
if (nowMatching.length > 0) {
  md.push('', '## Known-red entries now MATCHING (candidates for allowlist removal)');
  for (const r of nowMatching) md.push(`- ${r.id}`);
}
md.push('');
const mdPath = args.report.replace(/\.json$/, '.md');
writeFileSync(mdPath, md.join('\n'));

console.log(`[vmjit-diff] ${Object.entries(counts).map(([k, v]) => `${k}=${v}`).join(' ')} | known-red=${knownRedHits} now-matching=${knownRedNowMatching} unexpected=${unexpected}`);
console.log(`[vmjit-diff] report: ${args.report}`);
console.log(`[vmjit-diff] report: ${mdPath}`);

process.exit(unexpected > 0 ? 1 : 0);
