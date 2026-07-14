export const meta = {
  name: 'wf-polyglot-distributed-harden',
  description: 'Polyglot × distributed composition capstone (user priority-spine): the composition COMPOSES (WF-3E, ancestor of HEAD) but only extern-C×remote is committed as gated Rust integration tests; fn python / fn typescript × @remote were verified ONLY by a manual matrix (wf2f-close-matrix.md, Fable 9/9) and can silently regress. VERIFY-FIRST (execute at HEAD, do NOT trust the matrix — the WF-3D F1 over-claim + the 3c dead-barrier over-claim both slipped past inspection): reproduce a fn python and a fn typescript invoked inside an @remote fn transferred to a real serve node that has the extension loaded, under vm AND jit sender, with the genuineness check (server log shows blobs>=2 foreign_entries=1; rename the client .so aside -> remote cell still passes ⇒ the SERVE node executed it). Then HARDEN: commit gated Rust integration tests (mirroring test_remote_foreign_extern_c_transfer_over_tcp) for fn python + fn typescript remote-transfer, gated behind extension-.so availability so CI without extensions skips gracefully. If py/ts composition is BROKEN at HEAD (regression since the matrix), fix-forward at root. Extensions already built in-tree (extensions/libshape_ext_{python,typescript}.so). Independent Opus verify. The in-frame-snapshot PARTIAL (snapshot inside a remote/foreign frame) stays a documented known residual — out of scope.',
  phases: [
    { title: 'Verify', detail: 'run the 3 committed extern-C/result/perm tests + reproduce py/ts x @remote on a real serve node, vm+jit, genuineness-checked' },
    { title: 'Harden', detail: 'commit gated py/ts remote-transfer integration tests (skip-if-no-extension); fix-forward only if HEAD-broken' },
    { title: 'IndepVerify', detail: 'independent Opus fresh-context re-run of the composition + the new tests' },
    { title: 'Finish', detail: 'gates + committed py/ts remote gate tests + report (PARTIAL noted) + no new #[ignore]' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-polydist'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/polyglot-distributed-harden, off main HEAD). Build/run via: ' + DX + ' <cmd> (devenv toolchain not auto-loaded).',
  '',
  'THE COMPOSITION (verified genuine post-WF-3E, ancestor of HEAD): a Shape program with an inline polyglot fn (`fn python name(...) -> Result<int>`, `fn typescript name(...) -> Result<int>`, or `extern C fn`) invoked INSIDE an `@remote`-annotated fn is transferred to another node and EXECUTED THERE. The foreign SOURCE travels in the minimal blob (crates/shape-vm/src/remote.rs:729); the D1 fix (remote.rs:591 build_minimal_blobs_by_hash scanning LoadModuleBinding+CallValue) makes the foreign stub travel; the receiver initializes bindings with real typed values (executor/mod.rs:948 initialize_foreign_stub_bindings — the (0,Bool) sentinel was DELETED); invoke_foreign_kinded (executor/control_flow/mod.rs:870) dispatches with permission+ffi-language gating; the serve node loads its own extensions (bin/shape-cli/src/commands/serve_cmd.rs:1081/1169). Receiver needs the .so present + `shape serve --ffi-languages python,typescript`.',
  '',
  'EXISTING COMMITTED TESTS (extern C only) in bin/shape-cli/src/commands/serve_cmd.rs (#[tokio::test(multi_thread)], NOT ignored): test_remote_foreign_extern_c_transfer_over_tcp (:2098 — the blobs>=2 / foreign_functions-non-empty regression path, extern C via libc labs), test_remote_call_result_ok_and_err_over_tcp (:2144), test_remote_permission_refusal_over_wire (:2206). These are the harness PATTERN to mirror for py/ts (in-process serve node over TCP). Extensions already built: extensions/libshape_ext_python.so + libshape_ext_typescript.so (also target/debug/, target/release/). The authoritative manual matrix (py/ts cells return 105/21, extern C 42) is docs/cluster-audits/wf2f-close-matrix.md.',
  '',
  'PHASE 1 — VERIFY (execute, no harden yet): (1) run the 3 committed tests above at HEAD — all green? (2) Reproduce py AND ts × @remote: mirror the extern-C test harness (in-process serve node that loads the python/typescript extension via the executor scope ffi_languages + extension load path used by serve_cmd), send an @remote fn whose body calls a `fn python`/`fn typescript` fn, under BOTH vm and jit sender mode. GENUINENESS: confirm the foreign stub travelled (blobs>=2, foreign_entries=1) and that the SERVE side executed it (not a client fallback) — e.g. assert the returned value matches the matrix cell AND that a client without the language opted-in cannot produce it. Report the ACTUAL per-language execution verdict (extern-c / python / typescript: green-vm-jit / vm-only / broken + exact error). Do NOT commit.',
  '',
  'PHASE 2 — HARDEN: add COMMITTED gated Rust integration tests (in serve_cmd.rs, mirroring test_remote_foreign_extern_c_transfer_over_tcp) for `fn python` AND `fn typescript` remote-transfer — asserting the foreign body executes on the serve node and returns the matrix value, vm+jit. Gate each behind extension-.so availability (skip cleanly — a `println!` note + early return — when the extension is absent, so CI without extensions is green; do NOT #[ignore]). If Phase 1 found py/ts BROKEN at HEAD (a real regression since the matrix), root-fix it first (no band-aid, no forbidden pattern) then add the test.',
  '',
  'CONSTRAINTS (CLAUDE.md): NO forbidden patterns (is_heap/tag-decode/ValueWord/Bool-default/parallel-discriminator). Do NOT weaken the ffi-language gating or permission checks to make a test pass. Do NOT alter the in-frame-snapshot PARTIAL behavior (out of scope). ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. Integration tests may need --test-threads controlled; tcp serve tests bind ephemeral ports.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule — the whole point of this lane): the committed py/ts remote-transfer gate tests ARE the deliverable (close the CI-coverage gap so py/ts×remote cannot silently regress).',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Verify')
const V_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['externc', 'python', 'typescript', 'genuine_serverside', 'evidence'],
  properties: {
    externc: { type: 'string', enum: ['green-vm-jit', 'vm-only', 'broken'] },
    python: { type: 'string', enum: ['green-vm-jit', 'vm-only', 'broken', 'blocked-no-extension'] },
    typescript: { type: 'string', enum: ['green-vm-jit', 'vm-only', 'broken', 'blocked-no-extension'] },
    genuine_serverside: { type: 'boolean', description: 'proven the SERVE node executed the foreign body (blobs>=2 foreign_entries=1 + client-cannot-reproduce), not a client fallback' },
    evidence: { type: 'string', description: 'exact harness + commands + observed returns per language + genuineness proof' },
  },
}
const v = await agent(CTX + '\n\nPHASE 1 — VERIFY only (execute). Run the 3 committed tests + reproduce py/ts x @remote on a real serve node, vm+jit, genuineness-checked. Report ACTUAL per-language verdict. Do NOT commit.',
  { label: 'verify-execute', phase: 'Verify', effort: 'high', schema: V_SCHEMA })

phase('Harden')
const H_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'tests_committed', 'any_fix', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    tests_committed: { type: 'string', description: 'the gated py + ts remote-transfer tests added (names, gating mechanism)' },
    any_fix: { type: 'string', description: 'any root-fix applied if py/ts was HEAD-broken (or "none — composition genuine at HEAD")' },
    evidence: { type: 'string', description: 'the new tests pass (or skip cleanly w/o extension); check-no-dynamic EXIT 0' },
  },
}
const h = await agent(CTX + '\n\nPHASE 1 VERDICT: ' + JSON.stringify(v) + '\n\nPHASE 2 — HARDEN. Commit gated py/ts remote-transfer integration tests (skip-if-no-extension, not #[ignore]); root-fix only if HEAD-broken. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "Harden polyglot x distributed: committed gated fn python/typescript @remote-transfer integration tests (close CI-coverage gap)").',
  { label: 'harden', phase: 'Harden', effort: 'high', schema: H_SCHEMA })

phase('IndepVerify')
const IV_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'composition_genuine', 'tests_real', 'gating_correct', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    composition_genuine: { type: 'boolean', description: 'from YOUR OWN run: py AND ts foreign bodies execute server-side across @remote (or cleanly blocked-no-extension, documented)' },
    tests_real: { type: 'boolean', description: 'the new tests exercise the REAL remote-transfer path (not a mirror/stub) and would FAIL if the foreign stub stopped travelling' },
    gating_correct: { type: 'boolean', description: 'tests skip cleanly without the extension (CI-green) and run when present; no #[ignore]; no gating/permission weakening' },
    evidence: { type: 'string', description: 'your own from-scratch run + a probe that the test actually catches a broken-transfer (e.g. revert the D1 fix -> test fails); concise' },
  },
}
const iv = await agent(CTX + '\n\nPHASE 1: ' + JSON.stringify(v) + '\nHARDEN: ' + JSON.stringify(h) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume the "composes" + "test added" claims are UNPROVEN until you run them. From scratch: (1) do the py AND ts foreign bodies actually execute SERVER-SIDE across @remote (blobs>=2, foreign_entries=1, client-cannot-reproduce) under vm AND jit — or are they cleanly blocked-no-extension? (2) Do the new tests exercise the REAL transfer path — would they FAIL if the foreign stub stopped travelling (e.g. mentally/actually revert the remote.rs:591 D1 LoadModuleBinding scan → the test must break)? A test that passes even with a stubbed transfer = REFUTED. (3) Do the tests skip cleanly without the extension (no #[ignore], CI-green) and NOT weaken any ffi-language/permission gating? Any silent-pass test, weakened gating, or non-server-side execution = REFUTED.',
  { label: 'indep-verify', phase: 'IndepVerify', effort: 'high', schema: IV_SCHEMA })

phase('Finish')
const F_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests_added', 'partial_noted', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + the shape-cli remote tests (extern-c + py + ts), brief' },
    tests_added: { type: 'string', description: 'the committed gated py/ts remote-transfer tests' },
    partial_noted: { type: 'string', description: 'the in-frame-snapshot PARTIAL documented as known residual (unchanged)' },
    merge_ready: { type: 'boolean' },
  },
}
const f = await agent(CTX + '\n\nHARDEN: ' + JSON.stringify(h) + '\nVERDICT: ' + JSON.stringify(iv) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Ensure the gated py/ts remote-transfer tests are committed (skip-clean w/o extension); note the in-frame-snapshot PARTIAL as a known residual; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-cli (the remote/serve tests). Commit (git commit --no-verify -m "Polyglot x distributed harden finalize: py/ts remote-transfer gate tests committed").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: F_SCHEMA })

return { v, h, iv, f }
