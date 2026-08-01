#!/usr/bin/env node
// check-jit-ffi-reachability.mjs — #216 follow-up tripwire.
//
// Usage:
//   node scripts/check-jit-ffi-reachability.mjs
//   node scripts/check-jit-ffi-reachability.mjs --self-test
//
// WHY THIS EXISTS
//
//   Registration is not reachability, and neither is declaration into IR.
//   That has been re-derived by hand three times in one day:
//
//     1. `jit_v2_string_eq` — correct, byte-comparing, unit-tested, NEVER
//        registered. Its tests passed by calling it directly, so the symbol
//        read as covered while no compiled program could reach it. Meanwhile
//        the live string comparison had no lowering at all and fell through
//        to a raw pointer `icmp` (#232, silent wrong output).
//
//     2. `jit_typeof` / `jit_to_string` / `jit_to_number` / `jit_type_check` /
//        `jit_iter_done` / `jit_pattern_check_constructor` — declared, with
//        zero workspace call sites.
//
//     3. `jit_v2_retain` / `jit_v2_release` — registered AND declared with a
//        Cranelift signature, but never given a `FuncRef`. `jit_v2_release`
//        freed ANY v2 carrier with a hardcoded `Layout(8, 8)`. It was filed
//        as a live UB hazard; it was in fact unreachable (#216).
//
//   The motivating artifact is from (3). Its only non-registration caller was
//   a test that exercised `retain` while routing AROUND `release` with a
//   hand-written `Layout(24, 8)` dealloc, under the comment:
//
//       // Clean up manually (don't use jit_v2_release which would dealloc wrong size)
//
//   The defect was known, documented in a comment, and worked around rather
//   than removed — which is exactly how it survived long enough to be filed
//   as live UB. A symbol that LOOKS covered while nothing a user could run
//   can reach it is the shape this check exists to make impossible.
//
// WHAT MAKES THE CORE RELATION SOUND
//
//   A registered symbol becomes callable from emitted code only through a
//   Cranelift `FuncRef`. Every `FuncRef` in shape-jit comes from one of four
//   `declare_func_in_func` sites, and only two can name an FFI symbol:
//
//     - `compiler/ffi_builder.rs`  — the `r!("...")` macro, string literals only.
//     - `compiler/witness_emit.rs` — one const, `WITNESS_ENTRY_SYMBOL`.
//
//   The other two (`compiler/strategy.rs`, `compiler/program.rs`) iterate
//   `user_func_ids` — compiled SHAPE functions keyed by `u16` index, not FFI
//   symbols. There is no dynamically constructed `ffi_funcs` lookup. So
//   "registered ∧ ¬(r!() key ∨ witness const)" ⇒ unreachable from emitted code.
//
// RULES
//
//   R1  a `builder.symbol("X")` registration with no `r!("X")` key
//   R2  a `declare(..., "X")` / `declare_function("X")` with no `r!("X")` key
//       (strictly stronger than R1: someone wrote a Cranelift signature, which
//       only makes sense if emitted code was meant to call it — this is
//       `jit_v2_release`'s exact shape)
//   R3  an `r!("X")` bound to field `f` where `self.ffi.f` appears NOWHERE in
//       crates/shape-jit/src (the `jit_get_prop` shape — wired-looking, dead)
//
// DELIBERATELY NOT A RULE — and why, so nobody "completes" it later:
//
//   "has an `r!()` key and a referenced field, but the field never appears in
//   a `builder.ins().call(self.ffi.X, ...)`" is NOT checked. It is unsound as
//   a static test: FuncRefs are routinely selected into a local and called
//   indirectly — `retain_func_for_place` returns `self.ffi.arc_closure_retain`
//   and the caller does `.call(retain_func, ..)`. Measured on the seed tree
//   that formulation flagged 142 symbols, including retain/release entries
//   proven live by execution. R3's stricter "never referenced at all" form is
//   sound because no indirection can read a field that is never named.
//
// DIRECTION
//
//   The baseline is monotonic non-increasing. Debt entries may be removed
//   (the symbol got wired, or deleted); a NEW violating symbol fails, and a
//   STALE entry (listed but no longer violating) also fails, so fixing a
//   symbol forces the ratchet down instead of leaving slack behind.
//
// WHAT A GREEN CHECK DOES *NOT* MEAN — read this before citing it
//
//   This check mechanizes reachability tiers 1 -> 2 -> 3:
//
//     tier 1  registered  (`builder.symbol`)
//     tier 2  declared    (a Cranelift signature)
//     tier 3  given a `FuncRef` that is actually referenced
//
//   **Tier 4 — "actually reached by a real program" — is NOT checked, and is
//   not statically decidable.** It needs a corpus. So a green result here
//   means a symbol is CALLABLE, not that it is LIVE. The natural misreading
//   is that green means the FFI surface is exercised; it does not.
//
//   That gap is currently wide open. Per #260 the vm/jit corpus is mostly
//   vacuous — measured 2026-08-01 at 11 of 488 programs executing any native
//   code (2.3%), so nothing in the tree presently constitutes a working
//   instrument for "live". `run-diff.mjs` reports the native-executing
//   denominator per run; read it before treating any "0 corpus hits" result
//   as evidence a symbol is dead. Note also that `program_fallback == null`
//   is the WRONG liveness metric — a per-function bail leaves it null while
//   nothing runs natively. Assert `sum(native_dispatches) > 0` instead.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE = "docs/program/adr011-012/baselines/jit-ffi-reachability-inventory.json";

const WITNESS_ENTRY_SYMBOL = "jit_witness_native_entry";

// ---------------------------------------------------------------------------
// Scan
// ---------------------------------------------------------------------------

function readRustFiles(dir) {
  const out = [];
  const walk = (d) => {
    for (const entry of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (entry.name.endsWith(".rs")) out.push(p);
    }
  };
  if (fs.existsSync(dir)) walk(dir);
  return out;
}

/// Drop whole-line `//` comments so a deletion note naming a removed symbol
/// (exactly what #216 left behind) cannot re-summon it as a live registration.
function stripLineComments(source) {
  return source
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n");
}

function readStripped(files) {
  return files.map((f) => stripLineComments(fs.readFileSync(f, "utf8"))).join("\n");
}

export function scan(root = repositoryRoot) {
  const jitSrc = path.join(root, "crates/shape-jit/src");
  const symbolsText = readStripped(readRustFiles(path.join(jitSrc, "ffi_symbols")));
  const builderText = stripLineComments(
    fs.readFileSync(path.join(jitSrc, "compiler/ffi_builder.rs"), "utf8"),
  );
  const allJitText = readStripped(readRustFiles(jitSrc));

  const collect = (text, re) => {
    const found = new Set();
    for (const m of text.matchAll(re)) found.add(m[1]);
    return found;
  };

  const registered = collect(symbolsText, /builder\.symbol\(\s*"([A-Za-z0-9_]+)"/g);
  const declared = new Set([
    ...collect(symbolsText, /declare\(\s*module,\s*ffi_funcs,\s*"([A-Za-z0-9_]+)"/g),
    ...collect(symbolsText, /declare_function\(\s*"([A-Za-z0-9_]+)"/g),
  ]);

  const fieldOf = new Map();
  for (const m of builderText.matchAll(/(\w+)\s*:\s*r!\("([A-Za-z0-9_]+)"\)/g)) {
    fieldOf.set(m[2], m[1]);
  }

  const usedFields = collect(allJitText, /self\.ffi\.(\w+)/g);

  const callable = new Set([...fieldOf.keys(), WITNESS_ENTRY_SYMBOL]);

  const r1 = [...registered].filter((s) => !callable.has(s)).sort();
  const r2 = [...declared].filter((s) => !callable.has(s)).sort();
  const r3 = [...fieldOf.entries()]
    .filter(([, field]) => !usedFields.has(field))
    .map(([symbol]) => symbol)
    .sort();

  return {
    counts: {
      registered: registered.size,
      declared: declared.size,
      funcrefs: fieldOf.size,
    },
    registeredWithoutFuncref: r1,
    declaredWithoutFuncref: r2,
    funcrefNeverReferenced: r3,
    fieldOf,
  };
}

// ---------------------------------------------------------------------------
// Compare
// ---------------------------------------------------------------------------

const RULES = [
  ["registeredWithoutFuncref", "registered_without_funcref", "R1", "registered with the JIT but never given a FuncRef — emitted code cannot name it"],
  ["declaredWithoutFuncref", "declared_without_funcref", "R2", "declared with a Cranelift signature but never given a FuncRef — the jit_v2_release shape"],
  ["funcrefNeverReferenced", "funcref_never_referenced", "R3", "has an r!() key but its FFIFuncRefs field is never referenced — wired-looking and dead"],
];

export function compare(baseline, actual) {
  const failures = [];

  const allowed = new Map();
  for (const entry of baseline.intentionally_unwired ?? []) {
    if (!entry.reason || !entry.reason.trim()) {
      failures.push(
        `intentionally_unwired entry \`${entry.symbol}\` has no reason. Every allow-listed ` +
          `symbol must say in one line why it is registered but unreachable, or the list ` +
          `becomes a dumping ground.`,
      );
    }
    allowed.set(`${entry.rule}:${entry.symbol}`, entry);
  }

  for (const [actualKey, baselineKey, rule, description] of RULES) {
    const observed = new Set(actual[actualKey]);
    const recorded = new Set(baseline.debt?.[baselineKey] ?? []);

    for (const symbol of observed) {
      if (recorded.has(symbol)) continue;
      if (allowed.has(`${rule}:${symbol}`)) continue;
      failures.push(
        `${rule} NEW: \`${symbol}\` is ${description}. Wire it (add an \`r!("${symbol}")\` ` +
          `key and call the FuncRef), delete the registration, or add it to ` +
          `intentionally_unwired with a one-line reason. Do NOT add it to debt — that list ` +
          `only shrinks.`,
      );
    }

    for (const symbol of recorded) {
      if (observed.has(symbol)) continue;
      failures.push(
        `${rule} STALE: \`${symbol}\` is listed as debt but no longer violates. It was wired ` +
          `or deleted — regenerate the baseline ` +
          `(node scripts/generate-jit-ffi-reachability-baseline.mjs) so the ratchet tightens.`,
      );
    }

    for (const symbol of recorded) {
      if (allowed.has(`${rule}:${symbol}`)) {
        failures.push(
          `${rule} DOUBLE-LISTED: \`${symbol}\` appears in both debt and ` +
            `intentionally_unwired. Pick one — debt is "known-bad, shrinking", ` +
            `intentionally_unwired is "correct as-is".`,
        );
      }
    }
  }

  return { failures };
}

// ---------------------------------------------------------------------------
// Self-test — every forced negative must fail, and the control must pass.
// ---------------------------------------------------------------------------

function selfTest(baseline, actual) {
  const control = compare(baseline, actual);
  if (control.failures.length > 0) {
    console.error("SELF-TEST FATAL: the unmutated control does not pass:");
    for (const failure of control.failures) console.error(`  ${failure}`);
    return false;
  }

  const negatives = [
    [
      "a newly registered symbol with no FuncRef fails",
      () => ({ ...actual, registeredWithoutFuncref: [...actual.registeredWithoutFuncref, "jit_fabricated_symbol"] }),
    ],
    [
      "a newly declared symbol with no FuncRef fails",
      () => ({ ...actual, declaredWithoutFuncref: [...actual.declaredWithoutFuncref, "jit_fabricated_declared"] }),
    ],
    [
      "a new never-referenced FuncRef fails",
      () => ({ ...actual, funcrefNeverReferenced: [...actual.funcrefNeverReferenced, "jit_fabricated_funcref"] }),
    ],
    [
      "a stale debt entry (fixed but still listed) fails",
      () => ({ ...actual, registeredWithoutFuncref: actual.registeredWithoutFuncref.slice(1) }),
    ],
  ];

  let ok = true;
  for (const [name, mutate] of negatives) {
    if (compare(baseline, mutate()).failures.length === 0) {
      console.error(`SELF-TEST FAILED: ${name} — the tripwire did not fire.`);
      ok = false;
    } else {
      console.log(`  self-test ok: ${name}`);
    }
  }

  // Allow-list hygiene is enforced against a mutated BASELINE, not a mutated scan.
  const baselineNegatives = [
    [
      "an allow-list entry with no reason fails",
      () => ({
        ...baseline,
        intentionally_unwired: [
          ...(baseline.intentionally_unwired ?? []),
          { symbol: "jit_reasonless", rule: "R1", reason: "" },
        ],
      }),
    ],
    [
      "a symbol in both debt and the allow-list fails",
      () => ({
        ...baseline,
        intentionally_unwired: [
          ...(baseline.intentionally_unwired ?? []),
          {
            symbol: baseline.debt.registered_without_funcref[0],
            rule: "R1",
            reason: "double-listed on purpose, for the self-test",
          },
        ],
      }),
    ],
  ];

  for (const [name, mutate] of baselineNegatives) {
    if (compare(mutate(), actual).failures.length === 0) {
      console.error(`SELF-TEST FAILED: ${name} — the tripwire did not fire.`);
      ok = false;
    } else {
      console.log(`  self-test ok: ${name}`);
    }
  }

  return ok;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const baselinePath = path.join(repositoryRoot, BASELINE);
  if (!fs.existsSync(baselinePath)) {
    console.error(`FATAL: baseline not found at ${BASELINE}`);
    console.error("Generate it: node scripts/generate-jit-ffi-reachability-baseline.mjs");
    process.exit(2);
  }
  const baseline = JSON.parse(fs.readFileSync(baselinePath, "utf8"));
  const actual = scan();

  if (process.argv.includes("--self-test") && !selfTest(baseline, actual)) {
    process.exit(1);
  }

  const { failures } = compare(baseline, actual);
  if (failures.length > 0) {
    console.error("JIT FFI reachability check FAILED:");
    for (const failure of failures) console.error(`  - ${failure}`);
    process.exit(1);
  }

  const d = baseline.debt;
  console.log(
    `registered=${actual.counts.registered} declared=${actual.counts.declared} ` +
      `funcrefs=${actual.counts.funcrefs} | unreachable debt: ` +
      `R1 ${d.registered_without_funcref.length} (R2 ${d.declared_without_funcref.length}), ` +
      `R3 ${d.funcref_never_referenced.length}; ` +
      `intentionally unwired ${(baseline.intentionally_unwired ?? []).length}.`,
  );
  process.exit(0);
}
