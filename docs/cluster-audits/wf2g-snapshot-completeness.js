export const meta = {
  name: 'wf2g-snapshot-completeness',
  description: 'Snapshot projection completeness: ModuleFn-by-content-hash (WF-2F combined cells -> persistable) + heap-element-array projection (WF-2B Defect 1); silent-corruption is the critical refute class',
  phases: [
    { title: 'Diagnose', detail: 'repro both projection gaps; locate opaque arms + resume rebind' },
    { title: 'Implement', detail: 'A: ModuleFn-by-hash projection+rebind; B: heap-element-array projection' },
    { title: 'Refute', detail: 'adversarial resume in fresh process; silent-corruption / Bool-default / ghost-share' },
    { title: 'Finisher', detail: 'gate + WF-2F combined cells green + heap-array round-trip' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf2g-snapshot-completeness'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const COMMON = `You are an agent in workflow WF-2G (snapshot projection completeness) for the Shape language.
GOAL: close the two remaining snapshot-serialization projection gaps so the resumability x distributed composition is fully persistable (user TOP priority: "resumability and distributed execution works ... polyglot works with distributed computing together").
The SerializableVMValue ENUM (crates/shape-runtime/src/snapshot.rs:502) already has the needed variants (ModuleFunction(String), Closure{function_id,type_id,upvalues}, Array(Vec<SV>), TypedObject{...}, Decimal, String, ...). BOTH gaps are in the PROJECTION (runtime value -> SerializableVMValue) and its resume rebind, NOT the enum, and live in crates/shape-vm/src/executor/snapshot.rs (see the opaque/surface-and-stop arms ~964-1086 and the fix-options note ~117-121):
  GAP A (ModuleFn-by-content-hash): a RECEIVER-populated / remote-TRANSFERRED HeapKind::ModuleFn binding is content-hash-identified and has NO local module name, so ModuleFunction(String) cannot carry it -> today snapshot() taken INSIDE a remote-transferred function returns a clean barrier Err (snapstate=0), surfaced-not-silent (design §4.5). This is exactly the WF-2F yellow: the 3 combined matrix cells (C/python/typescript @remote fn that snapshot()s mid-exec) EXECUTE correctly but do not produce a persistable Snapshot::Hash. Fix pattern is the SAME content_hash-carrier the Closure/Q53(b) closure-over-wire arm uses (serialized arm carrying the entry content_hash, rebound via ordinal<->hash on resume). Make the snapshot SELF-CONTAINED enough that resume can re-resolve the transferred function (carry the blob or its hash + require the content-addressed store; per the ratified design pick the robust choice and cite it).
  GAP B (heap-element arrays): Array(Vec<SV>) exists and element arms (String/Decimal/TypedObject) exist, but the PROJECTION of a runtime TypedArray whose elements are heap pointers (Array<string> / Array<Decimal> / Array<TypedObject>) hits an opaque/refuse arm -> scalar arrays round-trip, heap-element arrays do not (WF-2B Defect 1, ADR-006 §2.7.5.1). Walk the TypedArray<Ptr> elements into Array(Vec<SerializableVMValue>) via typed carriers.
DESIGN (binding): docs/design/snapshot-resume.md + docs/design/distributed-function-transfer.md + docs/design/polyglot-distributed-integration.md (all ratified 2026-07-05). Read the relevant sections.
HARD RULES:
- Work ONLY in ${WT} (branch wave2/snapshot-completeness). Run every build/test as: cd ${WT} && ${DX} <cmd>. Reading main (/home/dev/dev/shape-lang/shape) allowed; NEVER cd there, NEVER edit there.
- CLAUDE.md Forbidden Patterns bind ABSOLUTELY, and snapshot serialization is the single highest-risk site for them. NO ValueWord/ValueBits revival "for serialization" (the canonical walk-back — refuse on sight); NO Bool-default for Load*Ptr (surface-and-stop with NotImplemented(SURFACE) instead per ADR-006 §2.7.8/Q10); NO tag-decode/bridge/probe/helper/hop/translator/adapter/shim; NO raw-u64 slot reinterpretation. Projection MUST read typed slots via as_heap_value() + HeapValue match (ADR-005 §1 single-discriminator) and the parallel NativeKind track (§2.7.7), never fabricate a kind from bits. If a correct fix seems to need forbidden machinery, STOP and return blocked with the exact wall — do NOT rationalize a "one decode at the boundary".
- Share-accounting: snapshot clone/drop must balance refcounts (the cluster-1.5 / v2-raw-heap ghost-share class caused SIGABRT at snapshot drop). Explicit clone_with_kind retain before any KindedSlot::new claim (mirror executor/mod.rs module_binding_read_owned_kinded).
- SHARED .git STASH DISCIPLINE: never 'git stash' in this shared worktree; never 'git stash clear'/'git stash pop'. Stage/commit WIP to your own branch. Finisher commits 'git commit --no-verify' ONLY after manually confirming pre-commit content guards pass.
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
TASK (diagnose only): Build release (${DX} cargo build --release --bin shape; repeat on timeout). Build extensions if needed (${DX} just build-extensions).
GAP A repro: reproduce the WF-2F combined cell — a @remote function that calls a foreign (or plain) fn, snapshot()s mid-execution on the receiver. Confirm it returns a clean barrier Err (snapstate=0) not Ok(Snapshot::Hash). Trace the exact projection arm in executor/snapshot.rs that refuses the receiver-populated HeapKind::ModuleFn binding, and the resume rebind path (deserialize -> re-resolve function). Read docs/design/distributed-function-transfer.md + snapshot-resume.md for the ratified persist/resume semantics (does the snapshot carry the blob, or require the content-addressed store? what happens if the blob is absent at resume?).
GAP B repro: snapshot() a program holding an Array<string> (and Array<Decimal>, Array<TypedObject>); confirm the heap-element array refuses to round-trip while a scalar Array<int> does. Locate the projection arm.
Return: gap_a (repro + exact refuse arm file:line + resume rebind path + the ratified persist semantics to implement), gap_b (repro + exact refuse arm file:line), plan (minimal correct implementation for both, citing the content_hash-carrier pattern for A).`,
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: { type: 'object', required: ['gap_a', 'gap_b', 'plan'], properties: {
    gap_a: { type: 'string' }, gap_b: { type: 'string' }, plan: { type: 'string' },
  }}})

log('Gap A: ' + String(diag.gap_a).slice(0, 160))

phase('Implement')
const implA = await agent(COMMON + `
Diagnosis gap A: ${diag.gap_a}
Plan: ${diag.plan}
TASK — GAP A: implement ModuleFn-by-content-hash snapshot projection + resume rebind. A receiver-populated / remote-transferred HeapKind::ModuleFn binding must serialize (by content_hash, using the Closure/Q53(b) content_hash-carrier pattern) so snapshot() taken inside a remote-transferred function returns Ok(Snapshot::Hash), and a fresh-process 'shape --resume <hash>' rebinds the function and continues correctly. Make the snapshot self-contained per the ratified design (carry the blob or require+cite the content-addressed store; clean documented refusal if genuinely unresolvable — never silent). Typed carriers only; balance refcounts. Build release; prove the 3 WF-2F combined cells (C/python/typescript) now produce a persistable Snapshot::Hash AND resume in a fresh process with the arithmetically-correct tail value. Commit WIP (git add -A && git commit --no-verify -m 'WF-2G gap A ModuleFn-by-hash wip').
Return status + files_changed + per-cell (C/py/ts combined) persist+resume evidence (the hash + the resumed value).`,
  { label: 'implA-modulefn-hash', phase: 'Implement', effort: 'high', schema: FIX })

const implB = await agent(COMMON + `
Gap A done: ${implA && implA.summary}
Diagnosis gap B: ${diag.gap_b}
TASK — GAP B: implement heap-element-array snapshot projection. A runtime TypedArray whose elements are heap pointers (Array<string>, Array<Decimal>, Array<TypedObject>) must project each element into Array(Vec<SerializableVMValue>) via typed carriers (as_heap_value() + HeapValue match; NO Bool-default, NO raw-bits reinterpretation) and round-trip through snapshot->resume. Build release; prove Array<string> ["a","b","c"], Array<Decimal>, and Array<TypedObject> each snapshot to Ok(Snapshot::Hash) and resume in a fresh process with element-equal values. Commit WIP (git commit --no-verify -m 'WF-2G gap B heap-element-array wip').
Return status + files_changed + per-type (string/Decimal/TypedObject) round-trip evidence.`,
  { label: 'implB-heap-arrays', phase: 'Implement', effort: 'high', schema: FIX })

phase('Refute')
const refute = await agent(COMMON + `
ADVERSARIAL REFUTER. Claimed: gap A=${implA && implA.status}, gap B=${implB && implB.status}.
Build release yourself. Snapshot serialization is where SILENT CORRUPTION hides — assume it is corrupt until proven. Try HARD to REFUTE:
1. GAP A GENUINE RESUME: for each of the 3 combined cells, take the snapshot, then resume in a genuinely FRESH process (new 'shape --resume <hash>', no source file) and byte-compare the tail value against a non-snapshot run. A resume that returns wrong/zeroed/ghost state, or that silently recomputes from scratch instead of restoring, is a CRITICAL refute. Kill the transferred blob from the store before resume and confirm EITHER it still resumes (snapshot self-contained) OR it refuses cleanly (never silent-wrong).
2. GAP B GENUINE: resume each heap-element array in a fresh process and compare EVERY element (not just length); a 1-element/zeroed/duplicated projection is a refute.
3. SHARE ACCOUNTING: run each snapshot+resume under a debug build / with repeated snapshot() calls in a loop; any SIGABRT / double-free / refcount imbalance at snapshot drop time is a refute (the v2-raw ghost-share class).
4. FORBIDDEN PATTERNS: grep the diff for ValueWord/ValueBits/raw-u64/Bool-default/tag-decode/is_tagged/synthesize_value_word at the projection+rebind boundary. Any hit = refute. ${DX} just check-no-dynamic.
5. ${DX} just test-fast.
Return refuted=true with the exact gap + mechanism if ANY fails; else refuted=false with the fresh-process resume evidence.`,
  { label: 'refute', phase: 'Refute', effort: 'high', schema: { type: 'object', required: ['refuted', 'detail'], properties: { refuted: { type: 'boolean' }, detail: { type: 'string' } } } })

let repair = null
if (refute && refute.refuted) {
  repair = await agent(COMMON + `
REFUTED: ${refute.detail}
TASK: Repair the refuted defect(s). Re-run the fresh-process resume proof + the refuter's checks yourself until they hold. Commit WIP (git commit --no-verify -m 'WF-2G repair wip'). Blocked only on a genuine wall (surface with exact mechanism). NO forbidden machinery — surface-and-stop instead.
Return status + summary + which gaps now hold + issues.`,
    { label: 'repair', phase: 'Refute', effort: 'high', schema: FIX })
}

phase('Finisher')
const final = await agent(COMMON.replace('Only the finisher commits. Every agent\'s final message IS machine-consumed structured output.', 'You ARE the finisher — you commit. Your final message IS machine-consumed structured output.') + `
State: gap A=${implA && implA.status}, gap B=${implB && implB.status}; refuted=${refute && refute.refuted}; repair=${JSON.stringify(repair && repair.summary)}. Prior stages left WIP commits + possibly uncommitted tail.
LONG-COMMAND PROTOCOL (mandatory): NEVER end your turn while a command runs; NEVER use run_in_background; run each build/test as a FOREGROUND call with a large timeout (up to 600000ms); on timeout RE-RUN the same command until it returns. Only then proceed.
STEPS (each a foreground call in ${WT}):
1. Stage+commit any uncommitted tail (git add -A && git commit --no-verify -m 'WF-2G finalize') after confirming content guards.
2. ${DX} just check-clean; ${DX} just check-no-dynamic; bash scripts/verify-merge.sh (expect 15/15).
3. ${DX} just test (pinned pre-existing OK: 4 shape-jit jit_closure_capture_* + pb3 flaky + 2 object-spread #[ignore]'d; anything new blocks).
4. ${DX} just diff-vmjit --fresh (build+run; repeat on timeout) — MATCH>=466, unexpected=0.
5. Re-run the 3 WF-2F combined cells + the heap-element-array round-trips one final time; record the authoritative persist+resume table. Update docs/cluster-audits/wf2f-close-matrix.md (or a sibling) to flip the combined cells from barrier-Err to persistable-and-resumed.
6. Final commit trailer:
Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
Return status green/yellow/red + commit shas + the combined-cell persist+resume table + residuals routed to a named lane.`,
  { label: 'finisher', phase: 'Finisher', effort: 'high', schema: { type: 'object', required: ['status', 'summary'], properties: {
    status: { enum: ['green', 'yellow', 'red'] }, summary: { type: 'string' }, commits: { type: 'array', items: { type: 'string' } }, resume_table: { type: 'string' }, residuals: { type: 'string' },
  }}})

return { gap_a: String(diag.gap_a).slice(0, 150), impl: { A: implA && implA.status, B: implB && implB.status }, refuted: refute && refute.refuted, final }
