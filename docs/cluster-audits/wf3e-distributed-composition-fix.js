export const meta = {
  name: 'wf3e-distributed-composition-fix',
  description: 'CRITICAL (user #1 priority): fix @remote x foreign composition broken at receiver + remote::call Result contract + permission/ffi-languages over-wire enforcement. Fable found 9 defects; this makes the composition genuinely work. Fable re-verifies the fix.',
  phases: [
    { title: 'Diagnose', detail: 'confirm the blob-closure + receiver-init + lowering mechanisms' },
    { title: 'Fix', detail: 'AB transfer+receiver-init, C remote::call Result, D perms, E ffi-languages' },
    { title: 'Refute', detail: 'Fable re-verifies its own 9 defects are fixed', model: 'fable' },
    { title: 'Finisher', detail: 'gate + re-prove 9-cell matrix FROM SCRATCH + correct matrix/book' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3e-distributed'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const COMMON = `You are an agent in workflow WF-3E (distributed-composition-fix) for the Shape language — the user's #1 priority: "polyglot works with distributed computing together." Independent Fable verification (docs/cluster-audits/fable-verify-results.md) proved the composition is BROKEN at the receiver on merged main; this workflow makes it genuinely work.
WHAT WORKS (do NOT regress): local polyglot (extern C / fn python / fn typescript), plain non-foreign @remote transfer (genuinely server-side), snapshot->resume across foreign frames, TLS server-side, the whole correctness core, strict typing. Fable confirmed all of these SOUND.
DEFECTS TO FIX (Fable-reproduced on merged main, with file:line):
  D1 [CRITICAL] @remote x foreign broken at receiver. Any @remote fn python/typescript/extern C fails: arity>=1 -> "frame_descriptor has 0 slots but arity is 1" (crates/shape-vm/src/remote.rs:1139); arity 0 -> server-side "call_value_immediate_nb: callee must be ... got Bool" (crates/shape-vm/src/executor/call_convention.rs:1150). ROOT: sender's build_minimal_blobs_by_hash (remote.rs:583-618) follows ONLY static blob.dependencies edges, so the foreign stub (reached via LoadModuleBinding+CallValue) never travels (serve log shows blobs=1 foreign_entries=1). Receiver's initialize_foreign_stub_bindings (crates/shape-vm/src/executor/mod.rs:943-973) silently `continue`s at :957/:965, leaving a (0, NativeKind::Bool) sentinel consumed at dispatch — a Bool-default-family FORBIDDEN-PATTERN violation (ADR-006 §2.7.8). Fix BOTH sides: sender packs the full reachable closure (foreign stubs + every LoadModuleBinding target); receiver initializes every referenced module binding with a REAL typed value — NO Bool-default sentinel (surface-and-stop if genuinely unresolvable, never a fabricated kind).
  D3 [CRITICAL] Transferred fn calling a stdlib MODULE fn (env::cwd/file::write_text/http) dies "call_value_immediate_nb: callee must be ... got Null" (receiver only inits foreign-stub bindings, remote.rs:1027; native stdlib module bindings stay Null because per-function dispatch never runs top-level module init). Fix: the receiver must initialize the native stdlib module bindings the transferred fn references.
  D7 [MED] Local `use std::core::snapshot` + bare snapshot() fails "got Bool" (same uninitialized-module-binding class as D1b); only `from std::core::snapshot use {snapshot}` works. Same root as D1b/D3 — fixing the module-binding init should fix this too; verify.
  D4 [HIGH] remote::call's public Result<R, RemoteError> contract is FICTION. Compiler lowers to __call_raising typed at the BARE return type (crates/shape-vm/src/compiler/function_calls.rs:6027-6047), so remote::call(...) yields a bare value and the DOCUMENTED `match { Ok(v)=>, Err(e)=> }` type-checks then CRASHES ("No match arm matched"). Fix: lower remote::call to the Result<R, RemoteError> surface (transport/remote failure -> Err(RemoteError::...), success -> Ok(R)) per stdlib remote.shape + design §4.1.1 / Q26. Same class as `as`->__into_*: compiler-recognized elaboration.
  D5 [HIGH] Receiver permission-over-wire refusal non-functional: transferred per-function blobs carry EMPTY required_permissions (record_blob_permissions fires only for NAMED top-level imports, statements.rs:1939 / compiler_impl_initialization.rs:324-332; namespace imports + the callee's own body permissions record nothing). Fix: transferred blobs carry their real derived required_permissions so the §4.6 load-refusal (remote.rs:977 load_linked_program_with_permissions) works on real data — a strict (granted=[]) node refuses a transferred fs.write fn at LOAD.
  D6 [HIGH] ffi_languages enforcement unreachable e2e: (a) @remote to a strict-empty node returns the D1 error (indistinguishable from opt-in); (b) even a node started --ffi-languages python errors "no extension provides language 'python'" via the Execute path — the serve node's loaded language runtimes are not wired into the executing engine; (c) default strict + --ffi-languages python still refuses at load "requires permissions not granted: ffi.call" — the opt-in flag does not grant the permission it gates on. Fix all three: the opted-in node EXECUTES the foreign fn, the strict-empty node CLEANLY refuses (distinguishable from D1), and --ffi-languages grants ffi.call.
  D8 [MED] Extension-version skew is design-only: ForeignFunctionEntry::compute_content_hash (crates/shape-vm/src/bytecode/core_types.rs:200-231) omits extension version; ExtensionReq (polyglot-distributed-integration.md A2) has zero code hits. Fix: include the extension version in the foreign entry content hash. (Lower priority — if it risks scope, surface and route to a follow-up.)
  Also fold in (recon-surfaced, same family): @remote Array-typed param = compile error (heap-param carrier); @remote module-global capture returns 0 e2e; remote::execute returns constant {bindings,schemas} metadata not the computed value.
HARD RULES:
- Work ONLY in ${WT} (branch wave3/distributed-composition-fix). Run every build/test as: cd ${WT} && ${DX} <cmd>. Reading main (/home/dev/dev/shape-lang/shape) allowed; NEVER cd there, NEVER edit there.
- CLAUDE.md Forbidden Patterns bind ABSOLUTELY. The D1b receiver sentinel is a LIVE Bool-default violation — the fix DELETES it (real typed init or surface-and-stop), it does NOT wrap/rename it. No ValueWord/tag-decode/raw-u64/Bool-default under any name. Marshal via typed KindedSlot/NativeKind carriers. remote::call Result lowering uses typed carriers + the TypedObject arg-pack pattern (same as WF-2C-fu R1). If a correct fix needs forbidden machinery, STOP and return blocked.
- SHARED .git STASH DISCIPLINE: never 'git stash' in this shared worktree; never 'git stash clear'/'pop'. Stage/commit WIP to your own branch. Finisher commits 'git commit --no-verify' ONLY after manually confirming pre-commit content guards pass.
- Extensions: ${DX} just build-extensions if the .so are missing. Serve genuineness: prove the foreign body runs ON the serve node (server log inbound Call + blobs>1 foreign_entries carried), never a client-side local fallback.
- Only the finisher commits. Every agent's final message IS machine-consumed structured output.`

const FIX = { type: 'object', required: ['status', 'summary'], properties: {
  status: { enum: ['done', 'partial', 'blocked'] },
  summary: { type: 'string' },
  files_changed: { type: 'array', items: { type: 'string' } },
  evidence: { type: 'string' },
  issues: { type: 'string' },
}}

phase('Diagnose')
const diag = await agent(COMMON + `
TASK (diagnose only): Build release (${DX} cargo build --release --bin shape; repeat on timeout) + ${DX} just build-extensions. Reproduce D1 (arity>=1 and arity 0), D3, D4, D6 hands-on to confirm Fable's file:lines on THIS worktree. Then determine the minimal correct fix for each root:
1. D1/D3/D7 (blob closure + receiver module-binding init): what exactly must build_minimal_blobs_by_hash add to the transitive closure (trace how the foreign stub + native stdlib module bindings are referenced — LoadModuleBinding/CallValue)? How must the receiver initialize those bindings with real typed values (foreign stub value + native stdlib module value) so dispatch finds a real callee, NOT a Bool/Null sentinel? Confirm the sentinel deletion path (surface-and-stop vs real init).
2. D4 (remote::call Result): where/how to change the lowering (function_calls.rs:6027-6047) so remote::call yields Result<R, RemoteError>.
3. D5 (permissions): how transferred blobs get real required_permissions.
4. D6 (ffi_languages e2e): the three sub-fixes.
Return: root_ab (the closure + receiver-init fix), root_c (remote::call Result), root_d (perms), root_e (ffi_languages), order.`,
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: { type: 'object', required: ['root_ab', 'root_c', 'root_d', 'root_e', 'order'], properties: {
    root_ab: { type: 'string' }, root_c: { type: 'string' }, root_d: { type: 'string' }, root_e: { type: 'string' }, order: { type: 'string' },
  }}})

log('Root AB: ' + String(diag.root_ab).slice(0, 160))

phase('Fix')
const fixAB = await agent(COMMON + `
Diagnosis (root AB): ${diag.root_ab}
TASK — FIX AB (the flagship: @remote x foreign transfer + receiver init). Sender: build_minimal_blobs_by_hash packs the full reachable closure (foreign stubs + every LoadModuleBinding target the transferred fn references). Receiver: initialize every referenced module binding (foreign stub AND native stdlib module) with a REAL typed value — DELETE the (0, NativeKind::Bool) sentinel (executor/mod.rs:943-973); surface-and-stop cleanly if a binding is genuinely unresolvable, never a fabricated Bool/Null. Build release + extensions. PROVE from scratch on real serve nodes (loopback, distinct ports 9780+): @remote fn python, @remote fn typescript, @remote extern C each transfer and EXECUTE ON THE SERVE NODE (server log blobs>1 foreign_entries carried) returning the correct value, vm+jit; AND a transferred fn calling env::cwd()/file::write_text works (D3); AND local bare snapshot() with 'use std::core::snapshot' works (D7); AND @remote Array-param + @remote module-global capture work. Commit WIP (git add -A && git commit --no-verify -m 'WF-3E fixAB transfer+receiver-init wip').
Return status + files_changed + per-language (py/ts/C) @remote foreign transfer evidence (server log + value) + D3/D7 evidence.`,
  { label: 'fixAB-transfer', phase: 'Fix', effort: 'high', schema: FIX })

const fixC = await agent(COMMON + `
Fix AB done: ${fixAB && fixAB.summary}
Diagnosis (root C): ${diag.root_c}
TASK — FIX C (remote::call Result<R, RemoteError> contract). Lower remote::call so the DOCUMENTED `match remote::call(addr, fn, args) { Ok(v)=>.., Err(e)=>.. }` works: success -> Ok(R) typed at the callee return type, transport/remote failure -> Err(RemoteError::...) per stdlib remote.shape. Also fix remote::execute to return the computed value, not the {bindings,schemas} metadata (or if execute's contract IS metadata, correct the docs — decide per stdlib/design and state which). Build release. PROVE: remote::call(node, mul, 6, 7) -> Ok(42), match Ok/Err both compile AND run (no "No match arm matched" crash), vm+jit; a connect failure -> Err(RemoteError::...). Commit WIP (git commit --no-verify -m 'WF-3E fixC remote-call-Result wip').
Return status + files_changed + the Ok + Err evidence.`,
  { label: 'fixC-result', phase: 'Fix', effort: 'high', schema: FIX })

const fixDE = await agent(COMMON + `
Fix AB: ${fixAB && fixAB.status}; Fix C: ${fixC && fixC.status}
Diagnosis (root D perms): ${diag.root_d}
Diagnosis (root E ffi_languages): ${diag.root_e}
TASK — FIX D + E (over-wire enforcement). D: transferred per-function blobs carry their real derived required_permissions (record for namespace imports + the callee body's own permissions), so a strict (granted=[]) serve node REFUSES a transferred fs.write fn at LOAD with PermissionDenied (not by dying on some other error). E: (a) an opted-in node (--ffi-languages python) EXECUTES a transferred/executed foreign fn (wire the serve node's loaded language runtimes into the executing engine); (b) a strict-empty-ffi_languages node CLEANLY refuses a foreign transfer with a language-named message DISTINGUISHABLE from the D1 error; (c) --ffi-languages python GRANTS the ffi.call permission it gates on (no "requires permissions not granted: ffi.call" for an opted-in node). Build release. PROVE all of D + E(a,b,c) on real nodes. Commit WIP (git commit --no-verify -m 'WF-3E fixDE perms+ffi-languages wip').
Return status + files_changed + D refusal evidence + E(a/b/c) evidence.`,
  { label: 'fixDE-enforcement', phase: 'Fix', effort: 'high', schema: FIX })

phase('Refute')
const refute = await agent(COMMON + `
INDEPENDENT FABLE RE-VERIFICATION. You are the SAME model (Fable) that originally found these 9 defects. Re-run YOUR OWN original probes from docs/cluster-audits/fable-verify-results.md against this worktree's release build + extensions, on real serve nodes (distinct ports). For EACH defect D1(arity 0 + arity>=1)/D3/D4/D5/D6(a,b,c)/D7/D8 and the recon items (@remote Array-param, module-global-capture, remote::execute), confirm it is now GENUINELY fixed — assume it is still broken until your own hands-on run proves otherwise. GENUINENESS: the foreign body must run ON the serve node (rename the CLIENT extension .so aside; a remote foreign cell that still works proves the server ran it; a client-only pass is a REFUTE). Grep the diff for any reintroduced Bool-default/ValueWord/tag-decode/raw-u64 at the receiver-init + marshal boundaries; ${DX} just check-no-dynamic. Re-prove the full 9-cell matrix ({C,py,ts}x{transfer,snapshot,combined}) FROM SCRATCH.
Return refuted=true with the exact still-broken defect + repro if ANY survives (or a regression to the SOUND features appears), else refuted=false with the from-scratch 9-cell evidence + per-defect fixed-proof.`,
  { label: 'fable-refute', phase: 'Refute', model: 'fable', effort: 'high', schema: { type: 'object', required: ['refuted', 'detail'], properties: { refuted: { type: 'boolean' }, detail: { type: 'string' } } } })

let repair = null
if (refute && refute.refuted) {
  repair = await agent(COMMON + `
FABLE REFUTED: ${refute.detail}
TASK: Repair every surviving defect. Re-run the affected probe on real serve nodes yourself until it holds (vm+jit, genuine server-side). Commit WIP (git commit --no-verify -m 'WF-3E repair wip'). Blocked only on a genuine wall. NO forbidden machinery — the D1b sentinel stays deleted.
Return status + summary + which defects now hold + issues.`,
    { label: 'repair', phase: 'Refute', effort: 'high', schema: FIX })
}

phase('Finisher')
const final = await agent(COMMON.replace('Only the finisher commits. Every agent\'s final message IS machine-consumed structured output.', 'You ARE the finisher — you commit. Your final message IS machine-consumed structured output.') + `
State: fixAB/C/DE = ${fixAB && fixAB.status}/${fixC && fixC.status}/${fixDE && fixDE.status}; fable-refuted=${refute && refute.refuted}; repair=${JSON.stringify(repair && repair.summary)}. Prior stages left WIP commits + possibly uncommitted tail.
LONG-COMMAND PROTOCOL (mandatory): NEVER end your turn while a command runs; NEVER use run_in_background; run each build/test as a FOREGROUND call with a large timeout (up to 600000ms); on timeout RE-RUN the same command until it returns. Only then proceed.
STEPS (each a foreground call in ${WT}):
1. Stage+commit any uncommitted tail (git add -A && git commit --no-verify -m 'WF-3E finalize') after confirming content guards.
2. ${DX} just check-clean; ${DX} just check-no-dynamic; bash scripts/verify-merge.sh (expect 15/15).
3. ${DX} just test (pinned pre-existing OK: 4 shape-jit jit_closure_capture_* + pb3 flaky + 2 object-spread ignored; anything new blocks). ADD tests: an @remote-foreign transfer integration test (the blobs=1 minimal-blob path with foreign_functions non-empty — the untested regression path), a remote::call Ok/Err test, a permission-refusal-over-wire test, an ffi_languages opt-in/refuse test.
4. ${DX} just diff-vmjit --fresh (build+run; repeat on timeout) — MATCH>=466, unexpected=0.
5. Re-prove the FULL 9-cell matrix FROM SCRATCH on real serve nodes; record the authoritative result. REWRITE docs/cluster-audits/wf2f-close-matrix.md replacing the corrected/over-claimed matrix with the now-genuine reproduced one (or honest per-cell status). Flip the polyglot-distributed.mdx book examples to match reality.
6. Final commit trailer:
Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
Return status green/yellow/red + commit shas + the from-scratch 9-cell table + residuals routed to named lanes.`,
  { label: 'finisher', phase: 'Finisher', effort: 'high', schema: { type: 'object', required: ['status', 'summary'], properties: {
    status: { enum: ['green', 'yellow', 'red'] }, summary: { type: 'string' }, commits: { type: 'array', items: { type: 'string' } }, matrix_final: { type: 'string' }, residuals: { type: 'string' },
  }}})

return { root_ab: String(diag.root_ab).slice(0, 150), fixes: { AB: fixAB && fixAB.status, C: fixC && fixC.status, DE: fixDE && fixDE.status }, fable_refuted: refute && refute.refuted, final }
