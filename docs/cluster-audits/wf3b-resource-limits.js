export const meta = {
  name: 'wf3b-resource-limits-dos',
  description: 'HIGH (meta-audit): two sandbox resource-limit defects on a serve/untrusted-code node. (A) --max-output-bytes is INERT: ResourceTracker::record_output (crates/shape-vm/src/resource_limits.rs:152) has ZERO call sites, so unbounded output is never capped (the sandboxed() default is 1 MB, silently unenforced). (B) --max-memory-bytes exceed is a process-killing DoS: it surfaces via panic!/exit-101 instead of a clean surfaced ResourceLimitExceeded, so untrusted code can kill the host/serve process. Fix both cleanly (surface, never panic); independent Opus adversarial re-proof. Disjoint from the schema-identity lane.',
  phases: [
    { title: 'Diagnose', detail: 'confirm record_output inert + the exact max-memory panic/exit-101 path' },
    { title: 'Fix', detail: 'wire record_output at the output write path; convert memory-exceed to a clean surfaced error' },
    { title: 'Verify', detail: 'independent Opus adversarial: both limits enforced cleanly, no panic/exit-101' },
    { title: 'Finish', detail: 'gates + regression tests' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3b-reslimits'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = `
Work IN ${WT} (branch wave3/resource-limits-dos, off main @8b9ef0d7). Build/test via: ${DX} <cmd>. This lane is DISJOINT from the schema-identity lane — do NOT touch type_schema/ or the compiler type core.

TWO HIGH-severity sandbox defects (a serve node runs untrusted transferred code; these are its containment):

DEFECT A [--max-output-bytes INERT]: ResourceTracker::record_output (crates/shape-vm/src/resource_limits.rs:152) exists and correctly returns Err(ResourceLimitExceeded::OutputLimit) past the cap, but has ZERO call sites anywhere (confirmed: grep for .record_output( outside the def is empty). So every print / output write bypasses the limiter; the sandboxed() preset's max_output_bytes=1 MB is silently unenforced. FIX: call record_output(bytes) at THE output write path (the print/stdout/capture sink — inspect crates/shape-vm/src/executor/vm_impl/output.rs and the print builtin) so exceeding the cap surfaces a clean ResourceLimitExceeded (the OutputLimited permission / OutputLimit error), NOT a silent pass and NOT a panic. Ensure both the direct-run and the serve/capture paths enforce it.

DEFECT B [--max-memory-bytes = process-killing DoS]: --max-memory-bytes is wired into ResourceLimits (bin/shape-cli/src/commands/script_cmd.rs:82-93) and checked (resource_limits.rs:140-142 returns Err(ResourceLimitExceeded::MemoryLimit)), but exceeding it on a real run surfaces as a panic!/process abort (exit 101) rather than a clean surfaced error. That lets untrusted code KILL the host/serve process (DoS). FIND the exact site where the MemoryLimit Err becomes a panic/unwrap/expect/exit-101 (grep the allocation + limit-check path + how ResourceLimitExceeded is propagated to the CLI/serve boundary). FIX: memory-limit-exceeded must surface as a clean, defined outcome — a ResourceLimitExceeded error propagated to a graceful CLI exit (a defined non-101 exit code) and, on a serve node, a clean per-request failure that does NOT kill the server process. NEVER a panic on untrusted input.

ACCEPTANCE: (A) a program that prints > max_output_bytes is stopped at the cap with a clean OutputLimit error (exit is graceful, output truncated at the cap), proven with a small --max-output-bytes. (B) a program that allocates > max_memory_bytes exits with a clean defined error/exit code (NOT 101/panic), proven with a small --max-memory-bytes; AND on a serve node the request fails cleanly without killing the server (the next request still succeeds). Regression tests for both.

HARD CONSTRAINTS (CLAUDE.md): stay on typed carriers; no ValueWord/tag-decode/Bool-default; ${DX} just check-no-dynamic EXIT 0. Do not weaken any existing limit; ADD enforcement + graceful surfacing.

STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.
`

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['output_root', 'memory_root', 'repro', 'fix_plan'],
  properties: {
    output_root: { type: 'string', description: 'where record_output must be called (the output write path), brief' },
    memory_root: { type: 'string', description: 'the exact site where MemoryLimit becomes panic/exit-101, file:line' },
    repro: { type: 'string', description: 'commands reproducing both (unbounded output; exit-101 on memory) on current HEAD' },
    fix_plan: { type: 'string', description: 'concrete fix for each, brief' },
  },
}
const diag = await agent(`${CTX}\n\nDIAGNOSE ONLY (no fix). Reproduce both from scratch (unbounded output despite a small --max-output-bytes; exit-101/panic on a small --max-memory-bytes). Pinpoint the output write path needing record_output + the exact memory-panic site. Do NOT commit.`,
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'output_evidence', 'memory_evidence'],
  properties: {
    status: { type: 'string', enum: ['both-fixed', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'brief' },
    output_evidence: { type: 'string', description: 'output capped cleanly at the limit (exit graceful), captured' },
    memory_evidence: { type: 'string', description: 'memory-exceed = clean defined exit (NOT 101), captured; serve survives' },
  },
}
const fix = await agent(`${CTX}\n\nDIAGNOSIS: ${JSON.stringify(diag)}\n\nIMPLEMENT both fixes. Build release. Prove: output capped cleanly; memory-exceed exits with a clean defined code (not 101/panic) and a serve node survives it. ${DX} just check-no-dynamic EXIT 0. Commit WIP (git add -A && git commit --no-verify -m 'WF-3B resource-limits enforcement wip').`,
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'output_enforced', 'memory_graceful', 'no_panic', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED'] },
    output_enforced: { type: 'boolean' },
    memory_graceful: { type: 'boolean', description: 'memory-exceed is a clean defined exit, never 101/panic' },
    no_panic: { type: 'boolean', description: 'true iff NO untrusted-input path panics the process' },
    evidence: { type: 'string', description: 'your own from-scratch repros: output cap, memory cap exit code, serve-survives-DoS; concise' },
  },
}
const verify = await agent(`${CTX}\n\nFIX: ${JSON.stringify(fix)}\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. From scratch: (A) print >cap with a small --max-output-bytes -> stopped cleanly at the cap? (B) allocate >cap with a small --max-memory-bytes -> exit code is a clean defined value, NOT 101, no panic? (C) on a real serve node, does an over-limit request kill the server or fail cleanly (next request still works)? Any panic/exit-101 on untrusted input = REFUTED. check-no-dynamic EXIT 0.`,
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    tests_added: { type: 'string', description: 'regression tests for output-cap + memory-graceful, names' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(`${CTX}\n\nFIX: ${JSON.stringify(fix)}\nVERDICT: ${JSON.stringify(verify)}\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add regression tests: an output-cap-enforced test + a memory-exceed-is-graceful (not panic) test. Run ${DX} just check-clean, ${DX} just check-no-dynamic, ${DX} just test --no-fail-fast. Commit (git commit --no-verify -m 'WF-3B finalize: resource-limit enforcement + tests').`,
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, fix, verify, finish }
