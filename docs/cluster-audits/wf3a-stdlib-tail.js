export const meta = {
  name: 'wf3a-stdlib-tail',
  description: 'Stdlib/edge audit tail (user-chosen focus, second half). Triage 7 candidate bugs against post-M1 main (some — json/xml — may be resolved by the schema-identity fix), then fix the survivors, independent Opus re-proof, gates. Candidates: std::finance compiler stack-overflow (#15); json navigation (#16); xml empty-children crash (#14); msgpack decode never decodes (#17); bigint unconstructible (#29/D2); time::millis()->float inference; two closure-return compile bugs.',
  phases: [
    { title: 'Triage', detail: 're-run all 7 repros post-M1; classify fixed-by-M1 vs still-broken + root' },
    { title: 'Fix', detail: 'fix each surviving bug in turn, commit each' },
    { title: 'Verify', detail: 'independent Opus adversarial re-proof of each fix' },
    { title: 'Finish', detail: 'gates + regression tests' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3a-edges'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = `
Work IN ${WT} (branch wave3/stdlib-tail, off post-M1 main). Build/test via: ${DX} <cmd>.

Seven CANDIDATE audit-tail bugs. NOTE: WF-3A M1 (content-derived schema identity) just merged and deleted the WF-2E resolve_typed_object_schema arity heuristic + the schema-id collisions — so the json/xml ones MAY already be fixed. Triage each against current HEAD before fixing.

CANDIDATES (repro each from a small program via ${WT}/target/release/shape; strict Shape, foreign fns need -> Result<T>):
1. std::finance UNUSABLE (#15): importing/using std::finance triggers a COMPILER stack-overflow. Repro: a program that uses a std::finance function. Root likely a compile-time recursion (type inference / comptime / import cycle).
2. json navigation (#16): json parse/navigation broken (accessing parsed json fields). Repro: parse a json string, navigate to a nested field, print it. (May be M1-fixed — the id-41 XmlNode/Json collision is gone.)
3. xml empty-children crash (#14): xml::stringify or parse crashes on an element with empty children. Repro: build/parse an xml node with no children, stringify it. (May be M1-fixed.)
4. msgpack decode (#17): msgpack::decode never actually decodes (encode works, decode returns wrong/empty). Repro: encode a value, decode it, compare.
5. bigint unconstructible (#29/D2): a bigint literal/constructor doesn't work. Repro: construct a bigint, do arithmetic, print. Determine the intended construction syntax (bigint literal? BigInt(...)? a suffix?) from the grammar/stdlib and make it work.
6. time::millis()->float inference: time::millis() returns a value whose type breaks inference in operand position, e.g. 'millis() - start' won't compile (float vs number alias not applied). Repro: let start = time::millis(); ... let dt = time::millis() - start.
7. two closure-return compile bugs: (a) a tail closure in a multi-statement fn body misresolves its own params ("Undefined variable 'b'"); (b) a returned parameterized closure mis-proves its return type as 'number'. Repro each minimal closure-return.

APPROACH: for EACH, first reproduce on current HEAD. If it already works (M1-fixed or never-broken), mark fixed-by-M1/not-a-bug with evidence — do NOT invent a fix. For genuine survivors, fix at the ROOT (no point-patches; if a bug shares a root with another, fix once). Prefer compiler/type-system correctness over stdlib band-aids. Commit each fix separately (git commit --no-verify -m 'WF-3A-tail: <bug> fix').

HARD CONSTRAINTS (CLAUDE.md): strict typing (no runtime coercion, no dynamic fallback); no ValueWord/tag-decode/Bool-default; ${DX} just check-no-dynamic EXIT 0. int vs number are separate; numeric conversion implicit only if lossless (else explicit 'as'). Do NOT weaken strict typing to make a repro pass.

STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields. Put per-bug detail in commit messages, not schema fields.
`

phase('Triage')
const TRIAGE_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['finance', 'json', 'xml', 'msgpack', 'bigint', 'time_millis', 'closure_returns'],
  properties: {
    finance: { type: 'string', description: 'still-broken(root) | fixed-by-M1 | not-a-bug — brief' },
    json: { type: 'string', description: 'still-broken(root) | fixed-by-M1 | not-a-bug' },
    xml: { type: 'string', description: 'still-broken(root) | fixed-by-M1 | not-a-bug' },
    msgpack: { type: 'string', description: 'still-broken(root) | fixed-by-M1 | not-a-bug' },
    bigint: { type: 'string', description: 'still-broken(root) | fixed-by-M1 | not-a-bug' },
    time_millis: { type: 'string', description: 'still-broken(root) | fixed-by-M1 | not-a-bug' },
    closure_returns: { type: 'string', description: 'both bugs status + roots, brief' },
  },
}
const triage = await agent(`${CTX}\n\nPHASE 1 — TRIAGE ONLY (no fix). Reproduce all 7 from scratch on current HEAD; classify each still-broken(+root) / fixed-by-M1 / not-a-bug with evidence. Do NOT commit.`,
  { label: 'triage', phase: 'Triage', effort: 'high', schema: TRIAGE_SCHEMA })

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['fixed', 'skipped', 'files_changed', 'evidence'],
  properties: {
    fixed: { type: 'string', description: 'which bugs were fixed, brief' },
    skipped: { type: 'string', description: 'which were already-fixed/not-a-bug (not touched)' },
    files_changed: { type: 'string', description: 'brief' },
    evidence: { type: 'string', description: 'each fixed bug now works — captured repro outputs; check-no-dynamic EXIT' },
  },
}
const fix = await agent(`${CTX}\n\nTRIAGE: ${JSON.stringify(triage)}\n\nPHASE 2 — FIX the SURVIVORS (still-broken only; skip fixed-by-M1/not-a-bug). Fix each at the root, in turn, committing each separately. Build release; prove each survivor's repro now works. ${DX} just check-no-dynamic EXIT 0.`,
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'per_bug', 'no_strict_weakening', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'MIXED'] },
    per_bug: { type: 'string', description: 'each survivor: fixed/still-broken, brief' },
    no_strict_weakening: { type: 'boolean', description: 'true iff no fix weakened strict typing / added coercion / dynamic fallback' },
    evidence: { type: 'string', description: 'your own from-scratch repros of each claimed-fixed bug; concise' },
  },
}
const verify = await agent(`${CTX}\n\nTRIAGE: ${JSON.stringify(triage)}\nFIX: ${JSON.stringify(fix)}\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write these fixes). Assume INSUFFICIENT until proven. Re-run each claimed-fixed bug's repro from scratch; confirm it works AND that the fix did NOT weaken strict typing (grep the diff for new coercion opcodes / dynamic fallback / relaxed type checks / Bool-default). Any regression or strict-weakening = REFUTED for that bug.`,
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    tests_added: { type: 'string', description: 'regression tests per fixed bug' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(`${CTX}\n\nFIX: ${JSON.stringify(fix)}\nVERDICT: ${JSON.stringify(verify)}\n\nFINISH (merge_ready true only for the CONFIRMED subset; note any REFUTED still-open). Add a regression test per fixed bug. Run ${DX} just check-clean, ${DX} just check-no-dynamic, ${DX} just test --no-fail-fast. Commit (git commit --no-verify -m 'WF-3A-tail finalize: stdlib/edge fixes + regression tests').`,
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { triage, fix, verify, finish }
