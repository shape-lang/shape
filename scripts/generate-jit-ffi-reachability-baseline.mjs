#!/usr/bin/env node
// generate-jit-ffi-reachability-baseline.mjs — #216 follow-up.
//
//   node scripts/generate-jit-ffi-reachability-baseline.mjs
//
// Regenerates the JIT FFI reachability baseline from the working tree, using
// the SAME scanner the check runs, so the two can never drift.
//
// Regenerate when a symbol is legitimately wired or deleted — the check FAILS
// on a stale entry precisely to force this, so the ratchet tightens instead of
// leaving slack behind. It also fails on a NEW violating symbol: regenerating
// to silence that is the failure mode this whole check exists to prevent, so
// the direction rule is recorded in the file itself and the debt lists must
// only ever shrink.
//
// Existing `intentionally_unwired` entries are PRESERVED across regeneration —
// their one-line reasons are hand-written and must not be clobbered.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { scan } from "./check-jit-ffi-reachability.mjs";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE = "docs/program/adr011-012/baselines/jit-ffi-reachability-inventory.json";
const baselinePath = path.join(repositoryRoot, BASELINE);

const previous = fs.existsSync(baselinePath)
  ? JSON.parse(fs.readFileSync(baselinePath, "utf8"))
  : {};
const intentionallyUnwired = previous.intentionally_unwired ?? [];
const allowed = new Set(intentionallyUnwired.map((e) => `${e.rule}:${e.symbol}`));

const actual = scan();
const minus = (list, rule) => list.filter((s) => !allowed.has(`${rule}:${s}`));

const record = {
  record_kind: "JitFfiReachabilityBaseline",
  record_version: 1,
  ticket: 216,
  entry_id: "JIT-FFI-REACHABILITY-INVENTORY",
  title: "JIT FFI reachability inventory (registration is not reachability)",
  generated_by: "scripts/generate-jit-ffi-reachability-baseline.mjs",
  checked_by: "scripts/check-jit-ffi-reachability.mjs",
  direction_rule:
    "Monotonic non-increasing. Every list under `debt` may shrink — a symbol got a FuncRef and a call site, or the registration was deleted — and may never grow. A NEW violating symbol is a check failure, not a baseline edit: adding one here to make the gate green is the exact defection this check exists to prevent. A STALE entry (listed but no longer violating) is also a failure, so a fix forces regeneration and the ratchet tightens rather than leaving slack. `intentionally_unwired` is NOT debt: it is the small curated set that is correct as-is, and every entry carries a one-line reason.",
  soundness_note:
    "A registered symbol becomes callable from emitted code only through a Cranelift FuncRef. Every FuncRef in shape-jit comes from one of four `declare_func_in_func` sites; only `compiler/ffi_builder.rs` (the `r!()` macro, string literals only) and `compiler/witness_emit.rs` (one const) can name an FFI symbol. `compiler/strategy.rs` and `compiler/program.rs` iterate `user_func_ids` — compiled Shape functions keyed by u16 index, not FFI symbols — and there is no dynamically constructed `ffi_funcs` lookup. So `registered AND NOT (r!() key OR witness const)` implies unreachable from emitted code.",
  not_a_rule:
    "\"has an r!() key and a referenced field, but never appears in a `builder.ins().call(self.ffi.X, ..)`\" is deliberately NOT checked: FuncRefs are routinely selected into a local and called indirectly (`retain_func_for_place` returns `self.ffi.arc_closure_retain`; the caller does `.call(retain_func, ..)`). On the seed tree that formulation flagged 142 symbols including retain/release entries proven live by execution. R3's stricter \"never referenced at all\" form is sound because no indirection can read a field that is never named.",
  rules: {
    R1: "a `builder.symbol(\"X\")` registration with no `r!(\"X\")` key",
    R2: "a `declare(..., \"X\")` / `declare_function(\"X\")` with no `r!(\"X\")` key — strictly stronger than R1, and `jit_v2_release`'s exact shape",
    R3: "an `r!(\"X\")` bound to a field that is never referenced anywhere in crates/shape-jit/src — the `jit_get_prop` shape",
  },
  scanned_counts: actual.counts,
  intentionally_unwired: intentionallyUnwired,
  debt: {
    registered_without_funcref: minus(actual.registeredWithoutFuncref, "R1"),
    declared_without_funcref: minus(actual.declaredWithoutFuncref, "R2"),
    funcref_never_referenced: minus(actual.funcrefNeverReferenced, "R3"),
  },
};

fs.mkdirSync(path.dirname(baselinePath), { recursive: true });
fs.writeFileSync(baselinePath, `${JSON.stringify(record, null, 2)}\n`);
console.log(
  `wrote ${BASELINE}: R1 ${record.debt.registered_without_funcref.length}, ` +
    `R2 ${record.debt.declared_without_funcref.length}, ` +
    `R3 ${record.debt.funcref_never_referenced.length}, ` +
    `intentionally unwired ${intentionallyUnwired.length}`,
);
