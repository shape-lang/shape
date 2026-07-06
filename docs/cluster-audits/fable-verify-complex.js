export const meta = {
  name: 'fable-verify-complex',
  description: 'Independent Fable-model adversarial verification of the highest-stakes features: polyglot (extern C / python / typescript + extension system), distributed transfer, polyglot x distributed composition, and the Wave-1 correctness core. READ-ONLY: refute done-right, do not fix.',
  phases: [
    { title: 'Verify', detail: 'Fable agents adversarially refute each domain against real serve nodes + extensions', model: 'fable' },
    { title: 'Synthesize', detail: 'Fable consolidates verdicts into a done-right / defect report', model: 'fable' },
  ],
}

const MAIN = '/home/dev/dev/shape-lang/shape'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const COMMON = `You are an INDEPENDENT adversarial verifier (running on the Fable model) for the Shape language. A separate engineering effort (on a different model) claims the complex features are "done right." Your job is to DOUBLE-CHECK by trying HARD to REFUTE that claim — assume each feature is broken, faked, or a local-fallback until your own hands-on run proves otherwise. You are READ-ONLY: do NOT edit any file, do NOT commit. You may read source, grep, and RUN the already-built release binary + start serve nodes.
WHY YOU: an independent model re-verifying the load-bearing features catches what the implementing model's own tests rationalize as green. Be skeptical and concrete.
ENV:
- Verify MERGED MAIN (currently eacbed65). Binary already built at ${MAIN}/target/release/shape — use it: ${MAIN}/target/release/shape run <file> --mode vm  and  --mode jit. Rebuild ONLY if genuinely missing/stale: cd ${MAIN} && ${DX} cargo build --release --bin shape (foreground).
- Extensions already built: ${MAIN}/extensions/libshape_ext_python.so, libshape_ext_typescript.so. If a run needs them and they are missing, ${DX} just build-extensions.
- Shape is namespaced (no global builtins); use std::core::... for stdlib; script mode runs top-level (fn main NOT auto-invoked). Serve: 'shape serve --address 127.0.0.1:<port> [--sandbox strict|permissive|none] [--ffi-languages python,typescript] [--tls-cert P --tls-key P] [--auth-token T]'.
GENUINENESS RULES (a "pass" that violates these is a REFUTE, not a pass):
- A distributed feature that works only because the CLIENT already has the code/extension is a FAKE — prove the work happens on the SERVE node (e.g. kill/rename the client-side body or extension and confirm the server still serves it, OR that it fails at the server, never silently local-executes).
- A foreign call that "succeeds" without the extension runtime actually executing is a FAKE.
- Silent-wrong output (differs vm vs jit, or wrong value with exit 0) is a DEFECT.
- Forbidden patterns (CLAUDE.md): grep the relevant source for ValueWord/ValueBits/Bool-default/tag-decode/synthesize_value_word/is_tagged/raw-u64 reinterpretation at the boundaries your domain touches; run ${DX} just check-no-dynamic. Any live forbidden dispatch = DEFECT.
Your final message IS a machine-consumed structured verdict. Be specific: exact commands, exact outputs, file:line for any defect.`

const VERDICT = { type: 'object', required: ['domain', 'verdict', 'evidence'], properties: {
  domain: { type: 'string' },
  verdict: { enum: ['SOUND', 'MIXED', 'DEFECTS'] },
  evidence: { type: 'string' },   // the concrete genuineness proof you gathered (commands + outputs)
  defects: { type: 'string' },    // any refutations, with file:line / repro; empty if none
  forbidden_clean: { type: 'boolean' },
}}

phase('Verify')

const domains = await parallel([
  // 1. POLYGLOT (no serve needed)
  () => agent(COMMON + `
DOMAIN: POLYGLOT + modular extension system.
Verify each of these is GENUINELY working on merged main (vm AND jit):
1. extern C: write a tiny C .so (or use libc, e.g. labs) and an 'extern C fn' declaration; call it, assert the correct value. Test an 'out' param if you can. Confirm a declaration alone no longer aborts the program (the audit's fatal-declaration bug).
2. fn python: a 'fn python name(...) -> T { ... }' actually executes in the python extension runtime and returns the correct typed value.
3. fn typescript: same for 'fn typescript'.
4. Extension system: the extensions genuinely load (LanguageRuntimeVTable). GENUINENESS: rename ${MAIN}/extensions/libshape_ext_python.so aside, re-run the python program, confirm it now FAILS with a clean 'extension not available' error (proving the .so was actually driving execution, not a stub) — then restore the name. (Do the same reasoning for typescript.)
5. Q13: a foreign fn returning a value that does NOT match its declared return type surfaces as a CATCHABLE Err on the user's Result (class-1), not a RuntimeError/crash.
Refute if any language is stubbed/faked, if the extension isn't actually required, if vm != jit, or if forbidden marshal is present at the foreign boundary (functions_foreign.rs / native_abi.rs / marshal).
Return domain='polyglot' + verdict + evidence + defects + forbidden_clean.`,
    { label: 'fable:polyglot', phase: 'Verify', model: 'fable', effort: 'high', schema: VERDICT }),

  // 2. DISTRIBUTED TRANSFER (serve ports 9740-9749)
  () => agent(COMMON + `
DOMAIN: DISTRIBUTED per-function transfer (@remote + remote::call). Use serve ports in 9740-9749 (distinct from sibling verifiers).
Verify GENUINELY working on merged main (vm AND jit):
1. @remote: a function annotated @remote transfers to a real 'shape serve' node and EXECUTES THERE, correct value back to the sender.
2. remote::call(addr, fn, args): the direct imperative form works; an argument whose type mismatches the callee's declared param is a COMPILE ERROR (not a runtime coercion, not a silent pass).
3. GENUINE TRANSFER (critical): make the client NOT have the function body resolvable except via transfer, or kill it after transfer, and confirm the SERVER serves it — a local-fallback pass is a REFUTE. Confirm the MissingModuleFunction retry-once resupply path.
4. Heap-shaped returns: a @remote fn returning Array<int> and one returning a TypedObject round-trip with correct element/field values (not a 1-slot/zeroed projection).
5. Permission-over-wire: the RECEIVER owns permissions (zero sender trust) — a strict serve node refuses a transfer needing perms it lacks; the receiver's own PermissionSet governs.
6. TLS: a serve node with a self-signed cert (generate a throwaway cert in /tmp, do not commit) ACTUALLY terminates TLS (handshake completes, transfer over the encrypted channel); a plaintext client to the TLS node is rejected; a no-cert non-loopback bind is honestly refused.
Refute on: local-fallback masquerading as transfer, silent coercion, wrong heap-return values, sender-trust bypass, TLS that accepts plaintext, or forbidden marshal (remote.rs / remote_builtins.rs / serve_cmd.rs / call_convention).
Return domain='distributed-transfer' + verdict + evidence + defects + forbidden_clean.`,
    { label: 'fable:distributed', phase: 'Verify', model: 'fable', effort: 'high', schema: VERDICT }),

  // 3. POLYGLOT x DISTRIBUTED COMPOSE (serve ports 9760-9769)
  () => agent(COMMON + `
DOMAIN: POLYGLOT x DISTRIBUTED composition (the user's #1 priority: "polyglot works with distributed computing together"). Use serve ports 9760-9769. Extensions required — ${DX} just build-extensions if missing; serve with --ffi-languages python,typescript.
Verify GENUINELY working on merged main (vm AND jit):
1. A foreign-function-bearing program (extern C / fn python / fn typescript) transfers @remote to a serve node and the FOREIGN BODY EXECUTES ON THE SERVE NODE (not the client). GENUINENESS: rename the CLIENT's extension .so aside; if a remote foreign cell still works, the SERVER ran it (good); if it only worked with the client extension present, that is a FAKE — REFUTE.
2. Ffi permission union + receiver enforcement: a serve node with strict-empty ffi_languages REFUSES a transferred 'fn python' with a clean language-named message; a node started --ffi-languages python EXECUTES it. Zero sender trust.
3. Hash coverage: changing only the foreign source (or the extension version) changes the blob content_hash (two nodes with different ext versions are detected, not silently mismatched).
4. Snapshot across the composition: snapshot()/--resume of a foreign-bearing program; and the COMBINED cell (@remote fn that snapshot()s mid-execution). NOTE: WF-2G is in-flight closing the combined-persist gap — the combined cell today should return a CLEAN BARRIER Err (surfaced, never silent corruption), not a persistable hash. Verify it is a CLEAN barrier (execution correct, honest refusal), NOT silent state corruption. If you observe silent corruption or wrong resumed state, that is a critical REFUTE.
Return domain='polyglot-distributed' + verdict + evidence + defects + forbidden_clean.`,
    { label: 'fable:compose', phase: 'Verify', model: 'fable', effort: 'high', schema: VERDICT }),

  // 4. CORRECTNESS CORE (Wave-1; mostly no serve)
  () => agent(COMMON + `
DOMAIN: CORRECTNESS CORE (Wave-1 fixes + strict-typing enforcement). Verify each audit-critical bug is genuinely FIXED (vm AND jit unless noted):
1. JIT double-execution of side effects: a program with a side effect (e.g. print("X")) at a call-count that triggers JIT prints the effect EXACTLY ONCE (audit: printed twice). Use ${DX} just diff-vmjit --fresh and confirm MATCH>=466 / unexpected=0.
2. HashMap.filter under JIT: arity-2 filter/map produces IDENTICAL correct output vm vs jit (audit: default-jit printed run-varying pointer garbage).
3. i64 overflow: add_pair(i64::MAX, 1) behaves the SAME under vm and jit per the ruling (no silent wrap-to-negative under jit while vm errors).
4. Drop at escape boundaries: a returned value's Drop runs correctly; a returned closure's captures are NOT dropped prematurely (no use-after-finalize — the audit's "dropped 9" before the closure reads the capture).
5. Security wiring: 'shape serve --sandbox strict' actually BLOCKS a wire-executed file::write_text (audit: it was a no-op that wrote to disk with success:true). The load-time permission check has real call sites.
6. STRICT-TYPING ENFORCEMENT (the catastrophic ReliableOnly bypass): confirm the shipped binary REJECTS bad types at compile time — 'let x: int = "hello"' and a string-where-int (e.g. 3 + "x") are COMPILE ERRORS, not silent reinterpretation of a heap pointer as i64. Also: 'if 5 { }' (non-bool condition) is an error (no truthiness); an implicit lossy numeric (int->number narrowing / number->int) requires explicit 'as'; a bare unparameterized generic ('let o: Option = ...') is an error (generics require <T>).
Refute on any: double-exec, vm!=jit divergence, silent overflow, premature/last-drop-lost, sandbox escape, or ANY of the strict-typing checks compiling when it must error. Run ${DX} just check-no-dynamic.
Return domain='correctness-core' + verdict + evidence + defects + forbidden_clean.`,
    { label: 'fable:correctness', phase: 'Verify', model: 'fable', effort: 'high', schema: VERDICT }),
])

const results = domains.filter(Boolean)
log('Fable verdicts: ' + results.map(r => r.domain + '=' + r.verdict).join(', '))

phase('Synthesize')
const report = await agent(COMMON.replace('Your final message IS a machine-consumed structured verdict.', 'Your final message IS a machine-consumed structured synthesis.') + `
TASK — SYNTHESIS (Fable, read-only, return the report; do NOT write files). The four domain verdicts:
${JSON.stringify(results)}
Consolidate into an independent done-right assessment:
1. overall: is the distributed + polyglot + correctness-core genuinely done right, based on the hands-on evidence? One paragraph, blunt.
2. sound: the features you independently CONFIRMED genuinely work (with the single strongest piece of evidence each).
3. defects: every refutation found, ranked by severity, each with file:line / repro / why it matters — these are the items the implementing model must fix. Distinguish real defects from the known in-flight WF-2G combined-persist gap.
4. residual_doubt: anything you could NOT fully verify (env limit, couldn't construct the probe) — so it is not mistaken for a clean pass.
Return overall + sound + defects + residual_doubt.`,
  { label: 'fable:synthesis', phase: 'Synthesize', model: 'fable', effort: 'high', schema: { type: 'object', required: ['overall', 'sound', 'defects', 'residual_doubt'], properties: {
    overall: { type: 'string' }, sound: { type: 'string' }, defects: { type: 'string' }, residual_doubt: { type: 'string' },
  }}})

return {
  verdicts: results.map(r => ({ domain: r.domain, verdict: r.verdict, forbidden_clean: r.forbidden_clean })),
  synthesis: report,
}
