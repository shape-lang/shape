export const meta = {
  name: 'wf3b-lsp-extern-c',
  description: 'MED/UX (audit finding #32): the LSP reports a FALSE error diagnostic on a VALID `extern C fn` declaration (the compiler accepts it and it runs, but the editor red-squiggles it). Improves polyglot/FFI UX (priority #3). Diagnose-first (confirm/refute the finding + pin the exact false-diagnostic site), fix so a valid extern C fn produces NO diagnostic while genuinely-malformed ones still error, independent Opus adversarial re-proof. Disjoint (shape-lsp crate) from the schema-identity lane.',
  phases: [
    { title: 'Diagnose', detail: 'confirm the false diagnostic on valid extern C + pin the site in shape-lsp' },
    { title: 'Fix', detail: 'recognize extern C fn as valid in LSP analysis; keep genuine errors' },
    { title: 'Verify', detail: 'independent Opus: valid extern C = no diagnostic; malformed = still errors' },
    { title: 'Finish', detail: 'gates + regression test' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3b-lsp'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = `
Work IN ${WT} (branch wave3/lsp-extern-c, off main). Build/test via: ${DX} <cmd>. Touch ONLY the shape-lsp crate (tools/shape-lsp) + a test — disjoint from every other lane.

DEFECT (audit finding #32): the Shape LSP (tools/shape-lsp) emits a FALSE error diagnostic on a VALID extern-C foreign-function declaration, e.g.:
  extern C fn labs(x: int) -> int from "c" as "labs";
The compiler ACCEPTS this and it links+runs at runtime (dlopen), but the editor shows a red error (likely "Undefined function" / unresolved-symbol / a type or body error) — a false positive that degrades the FFI authoring UX.

FIRST confirm-or-refute: reproduce the LSP diagnostics on a small file containing a valid extern C fn (drive the analysis path that server.rs / diagnostics.rs runs on didChange/didOpen, or the relevant unit entry). If the finding does NOT reproduce (maybe already fixed / mis-scoped), say so with evidence and stop. If it reproduces, pin the EXACT site that emits the false diagnostic (candidates: diagnostics.rs Undefined-function/variable analysis treating the extern body/linkage as unresolved; foreign_lsp.rs virtual-document handling wrongly applied to extern C which has NO Shape/foreign body; the symbol/name-resolution pass not registering the extern C fn as a defined callable).

FIX: LSP analysis must treat a well-formed 'extern C fn name(params) -> Ret from "lib" as "symbol"' declaration as a VALID defined function (no diagnostic) — its resolution is deferred to runtime dlopen, exactly as the compiler treats it. Do NOT suppress diagnostics for genuinely-malformed declarations (missing return type where required, bad syntax, calling an undeclared function) — those must still error. Mirror how the compiler's analysis accepts extern C (the compiler is the oracle for what is valid).

ACCEPTANCE: (1) a file with a valid extern C fn declaration + a call to it produces ZERO error diagnostics from the LSP analysis. (2) a genuinely-malformed case still produces the appropriate diagnostic (prove one negative control). (3) if the fn is also callable, no false 'undefined' on the call site.

HARD CONSTRAINTS (CLAUDE.md): shape-lsp only; no ValueWord/tag-decode/Bool-default (unlikely to be relevant here). ${DX} just check-no-dynamic EXIT 0; ${DX} just check-clean EXIT 0.

STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.
`

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['reproduced', 'site', 'false_message', 'fix_plan'],
  properties: {
    reproduced: { type: 'boolean', description: 'true iff the false diagnostic on valid extern C reproduces' },
    site: { type: 'string', description: 'file:line emitting the false diagnostic (or "n/a — not reproduced")' },
    false_message: { type: 'string', description: 'the exact false diagnostic text observed' },
    fix_plan: { type: 'string', description: 'concrete fix, brief' },
  },
}
const diag = await agent(`${CTX}\n\nDIAGNOSE ONLY (no fix). Confirm or refute the false diagnostic on a valid extern C fn; if confirmed, pin the exact emitting site + the false message. Do NOT commit.`,
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['fixed', 'not-a-bug', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'brief' },
    evidence: { type: 'string', description: 'valid extern C = zero diagnostics; malformed control still errors; captured' },
  },
}
const fix = await agent(`${CTX}\n\nDIAGNOSIS: ${JSON.stringify(diag)}\n\nIf reproduced, IMPLEMENT the fix (if diag says not-a-bug, return status:not-a-bug with evidence and skip). Prove valid extern C = zero diagnostics AND a malformed control still errors. ${DX} just check-clean + check-no-dynamic EXIT 0. Commit WIP (git add -A && git commit --no-verify -m 'WF-3B LSP extern-C valid-recognition wip').`,
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'valid_clean', 'malformed_still_errors', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'not-a-bug'] },
    valid_clean: { type: 'boolean', description: 'valid extern C produces no error diagnostic' },
    malformed_still_errors: { type: 'boolean', description: 'a genuinely-malformed decl still errors' },
    evidence: { type: 'string', description: 'your own from-scratch checks, concise' },
  },
}
const verify = await agent(`${CTX}\n\nFIX: ${JSON.stringify(fix)}\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. From scratch: (1) a valid extern C fn + a call to it -> ZERO error diagnostics? (2) a malformed variant (e.g. bad syntax / undeclared-callee) -> still errors appropriately (no over-suppression)? Report exactly what you saw.`,
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-lsp tests, brief' },
    test_added: { type: 'string', description: 'regression test name + location' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(`${CTX}\n\nFIX: ${JSON.stringify(fix)}\nVERDICT: ${JSON.stringify(verify)}\n\nFINISH (only if CONFIRMED; if not-a-bug, report merge_ready:false + note it was already correct). Add a regression test: valid extern C fn yields no error diagnostic (+ a malformed control still errors). Run ${DX} just check-clean, ${DX} just check-no-dynamic, ${DX} cargo test -p shape-lsp. Commit (git commit --no-verify -m 'WF-3B finalize: LSP extern-C recognition + test').`,
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, fix, verify, finish }
