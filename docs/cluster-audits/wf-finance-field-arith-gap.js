export const meta = {
  name: 'wf-finance-field-arith-gap',
  description: 'User-greenlit 2026-07-07 with a HARD BINDER: the finance sweep surfaced a GENERAL LANGUAGE/COMPILER soundness gap (NOT a finance feature). An object-field NUMBER arithmetic on an untyped/generic param the type-checker cannot prove is `number` does NOT error — it SILENTLY DROPS an operand (`fn candle_range(row){ row.high - row.low }` returned just row.high, dropping `- row.low`, no diagnostic) or lowers to a runtime DYNAMIC `sub` dispatch ("no method sub on receiver kind Float64"). Both are dynamic-fallback-shaped outcomes strict typing must reject up front (the reliableonly_strict_bypass class — a SILENT WRONG VALUE). USER CONSTRAINT (binding): finance is PURE STDLIB and must NEVER bleed into the language — the fix is to the GENERAL compiler/type-system ONLY (any untyped/generic-param object-field arithmetic → clean COMPILE ERROR, no silent-drop, no dynamic-dispatch), with ZERO finance-specific coupling. If diagnosis shows the gap is somehow finance-specific rather than a general language gap, HALT and ESCALATE to the user. The fix must NOT touch crates/shape-runtime/stdlib-src/finance/** at all — it is a compiler fix, proven on a minimal NON-finance repro. Independent Opus verify.',
  phases: [
    { title: 'Diagnose', detail: 'confirm it is a GENERAL language gap (minimal non-finance repro) + pin the compiler mechanism; escalate if finance-specific' },
    { title: 'Fix', detail: 'general compiler: unprovable object-field arithmetic -> clean compile error (no silent-drop, no dynamic-dispatch); NO finance coupling' },
    { title: 'Verify+Finish', detail: 'independent Opus: silent-drop gone generally, non-finance repro errors cleanly, no regression, finance untouched by the fix' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-fieldarith'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/finance-field-arith-gap, off main HEAD). Build/run via: ' + DX + ' <cmd>. This is a GENERAL LANGUAGE/COMPILER fix — the reliableonly_strict_bypass class (a silent wrong value is worse than a crash).',
  '',
  'THE GAP (general, surfaced by finance): an object-field NUMBER subtraction/arithmetic where the receiver param is UNTYPED/GENERIC (the checker cannot prove the field is `number`) does not error. Two observed dynamic-fallback-shaped outcomes: (i) SILENT operand-drop — `fn candle_range(row){ row.high - row.low }` compiled+ran returning just `row.high` (12.0), dropping `- row.low`, NO diagnostic; (ii) runtime DYNAMIC dispatch — `fn body(row){ abs(row.close - row.open) }` lowered to a dynamic `sub` method dispatch that fails at runtime ("no method \'sub\' on receiver kind Float64"). Both violate strict typing, which must reject an unprovable-type arithmetic at COMPILE time. Root hint from the finance diagnosis: "the OP0 no-return-annotation skip at crates/shape-vm/src/compiler/..." + the untyped-param not being monomorphized in a dependency-module compile so the field type stays unproven and arithmetic lowers to a dynamic/dropped path.',
  '',
  'BINDING USER CONSTRAINT (2026-07-07): finance is PURE STDLIB. The fix is to the GENERAL compiler/type-system ONLY. It must NOT touch crates/shape-runtime/stdlib-src/finance/** (or add ANY finance-specific language code). Prove the fix on a MINIMAL NON-FINANCE repro (a plain `type P { high: number, low: number }` + `fn f(row) { row.high - row.low }` untyped param, or the equivalent generic-param form). If your diagnosis finds the gap is somehow finance-SPECIFIC (not reproducible with a plain non-finance type + untyped-param field arithmetic), HALT and set escalate=true — do NOT proceed; the supervisor must escalate to the user.',
  '',
  'PHASE 1 DIAGNOSE (no fix): build a MINIMAL NON-FINANCE repro that reproduces BOTH the silent-operand-drop and the dynamic-`sub` outcomes with an untyped/generic-param object-field arithmetic. Pin the exact compiler mechanism (where field arithmetic on an unproven-type receiver lowers to a drop/dynamic path instead of a type-check error). CONFIRM it is a general language gap (reproduces with a plain non-finance type). If it does NOT reproduce without finance (finance-specific), set escalate=true + STOP.',
  '',
  'PHASE 2 FIX (general compiler, no finance): make an object-field arithmetic whose operand type the checker CANNOT prove to be a numeric type a CLEAN COMPILE ERROR (a clear diagnostic: the field/receiver type is unproven, annotate the param). NO silent operand-drop, NO runtime dynamic `sub`/arithmetic dispatch, NO coercion, NO forbidden pattern. The remedy for user code is to annotate the param (which already works). Ensure the error fires for ANY unprovable object-field arithmetic (general), and that already-provable cases (annotated params, monomorphized calls, same-module) still compile+run. Do NOT touch stdlib-src/finance/**.',
  '',
  'CONSTRAINTS (CLAUDE.md): NO forbidden patterns (no dynamic-dispatch fallback, no Bool-default, no coercion). int/number separate. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. NOTE: this may make some currently-compiling untyped-param code now error (correctly) — that is the point; but verify the blast radius is untyped-param-field-arithmetic only (not a broad over-rejection of valid code).',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): a NON-FINANCE test that an untyped/generic-param object-field arithmetic is a clean compile error (no silent-drop, no dynamic dispatch, no crash); and that annotating the param compiles+runs correctly (the operand is NOT dropped — full arithmetic). No new #[ignore].',
  '',
  'STRUCTURED-OUTPUT: ONE clean JSON object, 1-4 plain sentences per field, NO XML/code blocks in fields.',
].join('\n')

phase('Diagnose')
const D_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['is_general_gap', 'escalate', 'mechanism', 'repro'],
  properties: {
    is_general_gap: { type: 'boolean', description: 'reproduces with a plain NON-finance type + untyped-param field arithmetic (general language gap)' },
    escalate: { type: 'boolean', description: 'true if the gap is finance-specific (NOT general) → HALT + escalate to user' },
    mechanism: { type: 'string', description: 'the exact compiler mechanism where unproven-type object-field arithmetic lowers to drop/dynamic instead of a type error' },
    repro: { type: 'string', description: 'the minimal non-finance repro (both silent-drop + dynamic-sub forms)' },
  },
}
const d = await agent(CTX + '\n\nPHASE 1 — DIAGNOSE (no fix). Confirm general-not-finance-specific + pin the mechanism. Do NOT commit.',
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: D_SCHEMA })

// GATE: if finance-specific, HALT and escalate.
if (d && d.escalate) {
  log('ESCALATE: the field-arith gap appears finance-SPECIFIC, not a general language gap: ' + (d.mechanism || '') + ' — STOPPING for supervisor to escalate to the user per the binding constraint.')
  return { d, halted_for_escalation: true }
}

phase('Fix')
const F_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'no_finance_touch', 'errors_cleanly', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the general compiler/type-system fix, brief' },
    no_finance_touch: { type: 'boolean', description: 'the fix touches NO stdlib-src/finance/** and adds NO finance-specific language code' },
    errors_cleanly: { type: 'boolean', description: 'unprovable object-field arithmetic is now a clean compile error (no silent-drop, no dynamic dispatch)' },
    evidence: { type: 'string', description: 'non-finance repro errors cleanly; annotated form computes full arithmetic; check-no-dynamic EXIT 0' },
  },
}
const REPAIR = [
  'REPAIR (2026-07-08): the first fix (committed 92b914d8 on this branch) had the RIGHT diagnosis + general-not-finance direction, BUT it OVER-REJECTED — independent verify found it regressed shape-test closures_hof::test_named_fn_as_map_arg (a NAMED FUNCTION passed to map). The identifiers.rs capture-guard refuses capturing an implicit-generic function-as-value whose body "requires concrete emission" — but that ALSO catches the LEGITIMATE case where the function-value flows to a HOF (map/filter/reduce/forEach) that MONOMORPHIZES it with a concrete element type. Those must WORK.',
  'NARROW the fix so: (KEEP) the genuinely-unsound reachability still errors — an implicit-generic fn whose body has an UNPROVABLE object-field/scalar arithmetic, reached via an indirect CallValue with un-monomorphizable args (bare `let f = fn; f(untyped)`), no longer silent-Pop-drops or dynamic-subs; (FIX) a function-as-value that flows to a HOF/consumer which monomorphizes it (map/filter/reduce/etc., or any call site that supplies concrete arg types) COMPILES + RUNS the full arithmetic. Investigate whether Shape\'s map monomorphizes the passed fn (re-emits its body with the concrete element type) or runs the deferred template blob — and design accordingly. PREFERRED: make the un-monomorphizable template arithmetic surface the strict error at EMISSION so monomorphizing paths (direct call, HOF that supplies concrete types) re-emit with concrete types and only the truly-unresolvable indirect path errors; alternatively narrow the capture guard to exclude flows that reach a monomorphizing consumer. Do NOT revert the correct general-gap fix; REPAIR the over-rejection. Still: compiler-only, NO stdlib-src/finance/** touch, no forbidden pattern.',
].join('\n')
const f = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(d) + '\n\n' + REPAIR + '\n\nPHASE 2 — FIX + REPAIR (general compiler, NO finance): unprovable object-field/scalar arithmetic reached un-monomorphized -> clean compile error, WHILE HOF/map-monomorphized function-values work. ' + DX + ' just check-no-dynamic EXIT 0 AND ' + DX + ' cargo test -p shape-test --test closures_hof green. Commit (git add -A && git commit --no-verify -m "Fix general strict-typing gap (narrowed): unprovable un-monomorphized field/scalar arithmetic -> compile error, not silent-drop/dynamic-sub; HOF-monomorphized function-values preserved (reliableonly_strict_bypass class; compiler-only, no finance coupling)").',
  { label: 'fix-repair', phase: 'Fix', effort: 'xhigh', schema: F_SCHEMA })

phase('Verify+Finish')
const V_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'silent_drop_gone', 'general_not_finance', 'no_regression', 'gates', 'merge_ready'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    silent_drop_gone: { type: 'boolean', description: 'from YOUR OWN non-finance repro: unprovable object-field arithmetic errors cleanly (no silent-drop, no dynamic-sub, no crash)' },
    general_not_finance: { type: 'boolean', description: 'the fix is compiler-only, touches NO finance source, adds NO finance-specific language code' },
    no_regression: { type: 'boolean', description: 'provable cases (annotated params, monomorphized calls) still compile+run; blast radius is untyped-param-field-arith only' },
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-vm/shape-runtime + the regression test, brief' },
    merge_ready: { type: 'boolean' },
  },
}
const v = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(d) + '\nFIX: ' + JSON.stringify(f) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context. The FIRST attempt regressed closures_hof::test_named_fn_as_map_arg — the repair must fix that WITHOUT losing the general-gap fix. From scratch: (1) does a NON-finance untyped/generic-param object-field/scalar arithmetic reached UN-MONOMORPHIZED (bare `let f = fn; f(untyped)`) now cleanly compile-ERROR (not silent-drop, not dynamic-`sub`, not crash)? (2) is the fix COMPILER-ONLY — zero stdlib-src/finance/** changes, zero finance-specific language code (grep the diff)? (3) HOF REGRESSION (load-bearing): does a named/implicit-generic function passed to map/filter/reduce/forEach still COMPILE + RUN — specifically run ' + DX + ' cargo test -p shape-test --test closures_hof and confirm test_named_fn_as_map_arg + the whole suite PASS; and your own `[1,2,3].map(double)` with `fn double(x){x*2}` works? (4) do provable cases (annotated params, same-module, monomorphized direct calls, cross-module finance import) still compute the FULL arithmetic? (5) blast radius: shape-vm + shape-runtime lib + a shape-test spread green — over-rejection confined to un-monomorphized unprovable arithmetic only? Add the non-finance regression test (both the error case AND the HOF-still-works case) if missing; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic. ANY surviving silent-drop/dynamic-dispatch, ANY closures_hof/HOF regression, any finance coupling, or broad over-rejection = REFUTED. Commit any added test (git commit --no-verify -m "Field-arith strict gap finalize: non-finance regression test").',
  { label: 'verify-finish', phase: 'Verify+Finish', effort: 'high', schema: V_SCHEMA })

return { d, f, v }
