export const meta = {
  name: 'wf6-finance-stack-overflow',
  description: 'Audit finding #15: std::finance is UNUSABLE — importing/using a std::finance function triggers a COMPILER stack-overflow (compile-time recursion: type inference / comptime / import cycle). Diagnose-first (reproduce the overflow from scratch, pin the recursion root), fix at the root so std::finance compiles + a finance function runs, independent Opus verify, gates + regression test.',
  phases: [
    { title: 'Fix', detail: 'add occurs-check into Type::Concrete; std::finance compiles + a fn runs' },
    { title: 'Verify', detail: 'independent Opus: finance usable; the fix is a real termination, not a limit bump' },
    { title: 'Finish', detail: 'gates + regression test (finance program compiles+runs)' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w6-finance'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave6/finance, off main). Build/test via: ' + DX + ' <cmd>. Release binary at ' + WT + '/target/release/shape after you build.',
  '',
  'DEFECT (audit #15): std::finance is UNUSABLE — a program that imports/uses a std::finance function triggers a COMPILER stack-overflow (SIGSEGV/stack overflow at COMPILE time, before execution). The finance stdlib lives at crates/shape-runtime/stdlib-src/finance/. Root is likely an unbounded compile-time recursion: a self/mutually-recursive type definition or inference, a comptime block that recurses, or an import/module cycle in the finance sources.',
  '',
  'DIAGNOSE FIRST: reproduce from scratch — write a minimal program that uses one std::finance function (find the module path + an exported fn by reading finance/*.shape) and compile it; confirm the stack-overflow. Then pin the EXACT recursion: is it type inference chasing a recursive type, a comptime evaluation loop, a generic-instantiation cycle, or an import cycle? Get a concrete stack signature / the recursive call.',
  '',
  'FIX AT THE ROOT: break the actual unbounded recursion (e.g. add the missing base case / cycle-detection / memoization at the recursion site; fix the recursive type definition; break the import cycle). This is a REAL termination fix, NOT merely raising a recursion/stack limit and NOT deleting the finance feature. If the finance source itself is malformed (a genuinely circular type/import), fix the source; if the compiler lacks cycle detection at that site, add it (bounded, principled).',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): strict typing (no runtime coercion, no dynamic fallback); no ValueWord/tag-decode/Bool-default; ' + DX + ' just check-no-dynamic EXIT 0. Do NOT paper over the overflow with a bigger stack or a recursion-depth cap that just moves the cliff — fix the termination. Do NOT weaken type checking to dodge the recursion.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule 2026-07-07): add a regression test that a std::finance program compiles + runs (the overflow can never silently return).',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

// Diagnosis already established (recovered from the prior run's transcript, gdb-confirmed):
const diag = {
  reproduced: true,
  recursion_root: 'Infinite recursion: Unifier::apply_to_annotation (unifier.rs:100, calls apply_substitutions) <-> Unifier::apply_substitutions (unifier.rs:55/86, calls apply_to_annotation for Type::Concrete) chasing a CYCLIC substitution binding X |-> Type::Concrete(Object{...X...}) that transitively embeds X own tyvar marker. ROOT: the occurs-check occurs_in (constraints.rs:467-479) returns false for the Type::Concrete(_) arm (line 477) — it never traverses INTO an annotation to find embedded tyvar markers — so the guard at constraints.rs:328 fails to reject X |-> Concrete(...X...) and bind (constraints.rs:332 / unifier.rs:30) stores the cycle. Unifier::bind has no occurs-check beyond the trivial Variable(v)==var case, and most bind call sites are unguarded. Triggered by engine.shape open object-literal state maps flowing through data.simulate(|candle, state, idx| ...) closures where state and the returned object literal unify a var against an object type containing itself.',
  fix_plan: 'Extend occurs_in to recurse into Type::Concrete(annotation), detecting any embedded tyvar marker (walk the annotation tree via annotation_as_tyvar) equal to var, so the constraints.rs:328 guard rejects the cyclic binding as TypeError::InfiniteType. Additionally harden Unifier::bind with the same occurs-check so EVERY bind site (not only the one guarded one) is protected. Real termination (honest infinite-type error), not a stack/depth cap, not a type-check weakening.',
  repro: 'from std::finance::backtest::engine use { backtest }  ->  thread main has overflowed its stack; fatal runtime error: stack overflow (exit 134). Only backtest::engine triggers it; other finance modules give ordinary compile errors.',
}

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'real_termination', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['fixed', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the root termination fix, brief' },
    real_termination: { type: 'boolean', description: 'true iff the recursion genuinely terminates now (not a stack/limit bump)' },
    evidence: { type: 'string', description: 'finance program compiles + a finance fn runs (captured output); check-no-dynamic EXIT' },
  },
}
const fix = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\n\nPHASE 2 — FIX at the root (real termination, not a limit bump). Build release; prove a std::finance program compiles and a finance function runs. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-6: fix std::finance compiler stack-overflow at the recursion root").',
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'finance_usable', 'real_termination', 'no_strict_weakening', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED'] },
    finance_usable: { type: 'boolean', description: 'a finance program compiles + runs from your OWN scratch repro' },
    real_termination: { type: 'boolean', description: 'the fix is a genuine termination (base case / cycle detection), not a stack/depth-limit bump' },
    no_strict_weakening: { type: 'boolean', description: 'no coercion / dynamic fallback / relaxed type check introduced' },
    evidence: { type: 'string', description: 'your own from-scratch finance compile+run + inspection of the fix; concise' },
  },
}
const verify = await agent(CTX + '\n\nDIAGNOSIS: ' + JSON.stringify(diag) + '\nFIX: ' + JSON.stringify(fix) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. From scratch: (1) a std::finance program compiles + runs (no compiler stack-overflow)? (2) is the fix a REAL termination (added base case / cycle detection / memoization) rather than a raised stack size or a depth cap that just relocates the crash? Read the diff. (3) grep the diff for coercion/dynamic-fallback/Bool-default + confirm no type-checking was weakened. Any remaining overflow, a mere limit bump, or strict-weakening = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    test_added: { type: 'string', description: 'regression test: a std::finance program compiles+runs, name + location' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add a regression test that a std::finance program compiles + runs (guards the overflow). Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' just test --no-fail-fast. Commit (git commit --no-verify -m "WF-6 finalize: std::finance usable + regression test").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, fix, verify, finish }
