export const meta = {
  name: 'wf3a-tail-repair-timemillis',
  description: 'Repair the PARTIAL time::millis operand-position fix (independent verify REFUTED it). The reported idiom `let dt = time::millis() - start` still fails ("operand types are unknown and number"): the declared return type is recovered at the EMIT path (native_module_declared_return_type_info) + let-binding, but the INFERENCE tier returns a fresh var for a qualified module-call, so a bare module::fn() used directly as a binary operand infers to unknown. Complete the propagation into inference so a module-qualified builtin call gets its declared scalar return type in ANY position. Do NOT regress the CONFIRMED json/msgpack navigation fix. Independent Opus re-proof.',
  phases: [
    { title: 'Repair', detail: 'route the module-call declared return type into inference/operand position' },
    { title: 'Verify', detail: 'independent Opus: millis()-start compiles+infers number; json/msgpack still work' },
    { title: 'Finish', detail: 'gates + time-millis operand regression test' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3a-edges'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = `
Work IN ${WT} (branch wave3/stdlib-tail; HEAD 5f67895e has the CONFIRMED json/msgpack fix + tests stacked on the PARTIAL time::millis fix 63cd9cae). Build/test via: ${DX} <cmd>.

THE REMAINING BUG (independent-verify REFUTED the partial fix): the reported timing idiom still fails to compile:
  let start = time::millis()
  // ... work ...
  let dt = time::millis() - start   // ERROR: "Cannot infer types for binary operation Sub: operand types are unknown and number"
The partial fix (63cd9cae) recovers a module-qualified builtin's declared scalar return type at the EMIT path (native_module_declared_return_type_info, function_calls.rs:5968, used at :5927 compile_module_namespace_call) and stamps the let-binding slot — so 'let start = time::millis()' works and 'start' is number. BUT a BARE 'time::millis()' used DIRECTLY as a binary operand infers to UNKNOWN, because the INFERENCE tier (infer_expr_type on the synthesized QualifiedFunctionCall) returns a fresh type var (the inference tier holds no module-export signatures). So the left operand of Sub is unknown, the right is number, and inference fails.

ROOT TO COMPLETE: make a module-qualified builtin call expression carry its DECLARED return type in the INFERENCE tier too — not only at emit / let-assignment. Route native_module_declared_return_type_info (or the module schema's declared return type) into infer_expr_type for a QualifiedFunctionCall so the call has the correct ConcreteType in ANY position (binary operand, fn argument, return expr, index, etc.), not just when bound to a let. Then 'time::millis() - start' and 'time::millis() - time::millis()' both compile and infer number.

CONSTRAINTS (CLAUDE.md, CRITICAL): this is strict typing — the recovered type must be the ACTUAL declared return type (number for time::millis), proven, never fabricated. NO IntToNumber/NumberToInt/Convert*To coercion opcodes; NO Bool-default; NO dynamic fallback; int vs number stay separate. Do NOT weaken inference to "make it pass" — recover the true declared type. ${DX} just check-no-dynamic EXIT 0. Do NOT regress the json/msgpack navigation fix (Result/Option wrapper return-type propagation must still work).

ACCEPTANCE: (1) 'let dt = time::millis() - start' compiles and dt is number (prints a number, e.g. elapsed ms). (2) 'time::millis() - time::millis()' (both bare) compiles to number. (3) json parse + Ok(v)=>v.is_null()/v.get(...) still navigates correctly (no regression). (4) a genuine type error still errors (e.g. time::millis() used where a string is required).

STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.
`

phase('Repair')
const REPAIR_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'evidence', 'no_regression'],
  properties: {
    status: { type: 'string', enum: ['fixed', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the inference-tier change, brief' },
    evidence: { type: 'string', description: 'millis()-start compiles+prints a number; millis()-millis() too; captured' },
    no_regression: { type: 'boolean', description: 'true iff json/msgpack navigation still works + check-no-dynamic EXIT 0' },
  },
}
const repair = await agent(`${CTX}\n\nREPAIR: complete the module-call declared-return-type propagation into the INFERENCE tier so a bare module::fn() infers its declared type in operand position. Build release; prove millis()-start and millis()-millis() compile+infer number AND json/msgpack navigation still works. ${DX} just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m 'WF-3A-tail: complete time::millis operand-position inference').`,
  { label: 'repair', phase: 'Repair', effort: 'high', schema: REPAIR_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'time_millis_operand', 'json_msgpack_intact', 'no_strict_weakening', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED'] },
    time_millis_operand: { type: 'boolean', description: 'millis()-start AND millis()-millis() compile+infer number' },
    json_msgpack_intact: { type: 'boolean', description: 'json/msgpack navigation still works (no regression)' },
    no_strict_weakening: { type: 'boolean', description: 'no coercion opcode / dynamic fallback / relaxed check introduced' },
    evidence: { type: 'string', description: 'your own from-scratch repros incl. a negative control (type error still errors); concise' },
  },
}
const verify = await agent(`${CTX}\n\nREPAIR: ${JSON.stringify(repair)}\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume INSUFFICIENT until proven. From scratch: (1) 'let start=time::millis(); let dt=time::millis()-start; print(dt)' compiles and prints a number? (2) 'time::millis()-time::millis()' compiles to number? (3) json parse + Ok(v)=>v.is_null()/v.get still navigates (no regression)? (4) negative control: does a genuine type error (e.g. a bare int operand where number required, or millis() where a string is needed) STILL error (no over-permissive inference)? Grep the diff for coercion/dynamic-fallback/Bool-default. Any failure or strict-weakening = REFUTED.`,
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'test_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + just test (new failures beyond pinned, or "only pinned"), brief' },
    test_added: { type: 'string', description: 'time::millis operand-position regression test name + location' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(`${CTX}\n\nREPAIR: ${JSON.stringify(repair)}\nVERDICT: ${JSON.stringify(verify)}\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add a regression test: a module-qualified builtin call in operand position infers its declared return type (e.g. time::millis() - start is number; a wrong-type use still errors). Run ${DX} just check-clean, ${DX} just check-no-dynamic, ${DX} just test --no-fail-fast. Commit (git commit --no-verify -m 'WF-3A-tail finalize: time::millis operand inference + regression test').`,
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { repair, verify, finish }
