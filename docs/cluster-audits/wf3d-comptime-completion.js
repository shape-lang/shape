export const meta = {
  name: 'wf3d-comptime-completion',
  description: 'Comptime excellence completion: generated-fn visibility (flagship), type_info.fields+bare-ident+param.const, --diagnostics json LSDS surface, u.to_json() method emission; P1-P14 acceptance',
  phases: [
    { title: 'Diagnose', detail: 'pin the pass-1.5 visibility mechanism + confirm gap sites' },
    { title: 'Fix', detail: 'visibility (flagship) -> type_info -> method-emission -> diagnostics-json' },
    { title: 'Refute', detail: 'P1-P14 adversarial, esp P8 fn-main showcase vm+jit' },
    { title: 'Finisher', detail: 'gate + probe table + book flip' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf3d-comptime-excellence'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const COMMON = `You are an agent in workflow WF-3D (comptime excellence completion) for the Shape language.
GOAL (user priority #2, verbatim): "comptime is excellent and ergonomic." The correctness CORE already landed (WF-1B S1-S6: marshal Bool-default deleted, schema-identity fixed, set-return soundness, __original__ typed forwarding, LSDS diagnostics, jargon firewall — all verified working on merged main). This workflow closes the PRECISE remaining gaps a fresh recon found, led by the flagship generated-function-visibility bug.
DESIGN (binding): docs/design/comptime-excellence.md (ratified 2026-07-05; all 12 OQs adopted). Read §4.1 (introspection contract v1), §4.5 (directive safety + generation, esp §4.5.1 "pass 1.5" whole-program pre-pass and §4.5.7 extend(expr)), §4.9 (showcases: §4.9.1 to_json METHOD emission), §7 (acceptance probes P1-P14). Also fix-plan §0 rules.
RECON FINDINGS (merged main 05612c77, evidence-grounded — trust these as the starting map, re-verify as you go):
  GAP 1 [FLAGSHIP, correctness]: `extend (expr)`-emitted functions register AFTER user function BODIES are compiled, so they are invisible to `fn main()` and to any user function — ONLY top-level script statements can call them. Repro: `fn main() { print(User_json_schema()) }` -> "Undefined function 'User_json_schema'" (both vm+jit, script + project mode), while the same call at top-level works. The §4.5.1 "pass 1.5" invariant is only HALF-applied (top-level sees generated items; earlier-compiled function bodies do not). The two-pass compiler is "register functions, then compile"; comptime-generated items must land in the function/method table during the pre-pass BEFORE pass-2 compiles user bodies. This breaks the design's flagship showcase form.
  GAP 2 [correctness]: `type_info(T).fields` absent -> "Property 'fields' does not exist on type 'object'" (want the §4.1.1 fields Array<FieldDescriptor>, OR a CLEAN SURFACE message if the V3-S5 Array<TypedObject> carrier is genuinely absent — never a generic property error, never the `__type_info_marshal_pending__` sentinel at comptime_builtins.rs:495-532 which is stale dead code to delete). Also bare-identifier `type_info(User)` fails the OUTER typecheck ("User is not compatible with string"); only `type_info("User")` works — the `rewrite_type_info_ident_args` rewrite fires in the comptime mini-VM but not for the outer typecheck of a script-level comptime block. Also `ParamDescriptor.const` reads false even for a `const a: int` param (grammar shape.pest:465) — the const flag isn't threaded to the descriptor.
  GAP 3 [ergonomics]: `--diagnostics json` CLI surface does not exist ("unexpected argument '--diagnostics'"; no DiagnosticsFormat in shape-cli). shape-diagnostics types are already Serialize — this is flag + serializer wiring to emit LSDS JSON (severity, location, comptime_trace). P1's designed observation mechanism + any LSP/MCP consumer needs it.
  GAP 4 [ergonomics, flagship]: method emission not delivered — `@to_json` (stdlib-src/serde/serialize.shape) ships a FREE function `User_to_json(v)`, not the `u.to_json()` type-extension METHOD §4.9.1 specified. `extend (expr)` + apply_comptime_extend_items should emit a method on the type.
HARD RULES:
- Work ONLY in ${WT} (branch wave3/comptime-excellence). Run every build/test as: cd ${WT} && ${DX} <cmd>. Reading main (/home/dev/dev/shape-lang/shape) allowed; NEVER cd there, NEVER edit there.
- CLAUDE.md Forbidden Patterns bind. The comptime marshal already deleted its Bool-default — do NOT reintroduce any Bool-default/kind-from-bits/ValueWord/tag-decode under any name. Descriptor rows stay TypedObjectStorage behind HeapValue (ADR-005 §1). type_info fields, if built, use the same named-concrete-schema mechanism §4.3 established (never positional/field-set schema ids). No runtime coercion; directives that change signatures re-enter the checker. If a correct fix needs forbidden machinery, STOP and return blocked.
- Diagnostics stay LSDS-routed (ADR-006 §9). The jargon firewall must keep passing — no internal jargon (ckpt/ADR-/V3-S5/REFUSED/§/phase-2c/W-/WF-) may leak into any user-facing string you add.
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
TASK (diagnose only, do NOT fix): Build release (${DX} cargo build --release --bin shape; repeat on timeout). Build extensions if needed. Then:
1. FLAGSHIP (gap 1): trace the exact two-pass mechanism. Where are comptime `extend (expr)` items registered (apply_comptime_extend_items / __emit_extend_items)? Where do user function BODIES compile (pass 2)? Why do generated items land AFTER user bodies? What is the MINIMAL correct ordering fix so generated items register in the function/method table during pass 1.5 (before pass-2 body compilation) WITHOUT breaking annotation chaining, directive ordering, or the working top-level path? Assess risk.
2. Confirm gap 2 sites (type_info.fields projection + rewrite_type_info_ident_args outer-typecheck gap + ParamDescriptor.const threading from AST FunctionParameter.const flag). Is the V3-S5 Array<TypedObject> carrier available for fields, or must fields SURFACE cleanly?
3. Confirm gap 3 (shape-cli arg parsing + where the LSDS diagnostics are produced so a --diagnostics json can serialize them).
4. Confirm gap 4 (how apply_comptime_extend_items emits items; can it emit a type-extension METHOD vs a free fn; what serialize.shape currently generates).
Return: flagship_mechanism (file:line + the ordering fix + risk), gap2 (sites + fields-carrier availability), gap3 (site), gap4 (site + method-emission feasibility), order (recommended fix order).`,
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: { type: 'object', required: ['flagship_mechanism', 'gap2', 'gap3', 'gap4', 'order'], properties: {
    flagship_mechanism: { type: 'string' }, gap2: { type: 'string' }, gap3: { type: 'string' }, gap4: { type: 'string' }, order: { type: 'string' },
  }}})

log('Flagship: ' + String(diag.flagship_mechanism).slice(0, 160))

phase('Fix')
const fix1 = await agent(COMMON + `
Diagnosis (flagship): ${diag.flagship_mechanism}
TASK — FLAGSHIP (gap 1): make comptime `extend (expr)`-generated functions/methods visible to user function bodies and `fn main()`. Complete the §4.5.1 pass-1.5 invariant: generated items register in the function/method table BEFORE pass-2 compiles user bodies. Preserve the working top-level path, annotation chaining, and directive ordering. Build release; PROVE the flagship showcase form works: a program with `fn main() { print(User_json_schema()) }` (and a user fn calling a generated fn) prints the correct JSON under BOTH --mode vm and --mode jit, script AND project mode. Commit WIP (git add -A && git commit --no-verify -m 'WF-3D gap1 generated-fn visibility wip').
Return status + files_changed + the flagship program + its vm+jit output (both must be correct).`,
  { label: 'fix1-visibility', phase: 'Fix', effort: 'high', schema: FIX })

const fix2 = await agent(COMMON + `
Gap 1 done: ${fix1 && fix1.summary}
Diagnosis gap 2: ${diag.gap2}
TASK — gap 2 (type_info completeness): (a) `type_info(T).fields` returns the §4.1.1 Array<FieldDescriptor> if the V3-S5 carrier is available, ELSE a CLEAN SURFACE diagnostic (jargon-free, never a generic property error, never the sentinel) — delete the stale `__type_info_marshal_pending__` sentinel + "arrives as Bool" comment (comptime_builtins.rs:495-532); (b) bare-identifier `type_info(User)` works (fix the outer-typecheck rewrite, not just the mini-VM); (c) `ParamDescriptor.const` populated true for a `const` param. Build release; prove each vm+jit. Commit WIP (git commit --no-verify -m 'WF-3D gap2 type_info wip').
Return status + files_changed + evidence for (a)/(b)/(c).`,
  { label: 'fix2-typeinfo', phase: 'Fix', effort: 'high', schema: FIX })

const fix4 = await agent(COMMON + `
Gap 1: ${fix1 && fix1.status}; gap 2: ${fix2 && fix2.status}
Diagnosis gap 4: ${diag.gap4}
TASK — gap 4 (method emission, flagship ergonomics): make `@to_json` emit a type-extension METHOD so `u.to_json()` works (per §4.9.1), not a free `User_to_json(v)`. Update stdlib-src/serde/serialize.shape + apply_comptime_extend_items as needed to emit a method on the type. Build release; prove `let u = User{...}; print(u.to_json())` prints correct JSON vm+jit (this depends on gap 1 visibility). Keep the derive showcase (§4.9.1) working. Commit WIP (git commit --no-verify -m 'WF-3D gap4 method emission wip').
Return status + files_changed + the u.to_json() program + vm+jit output.`,
  { label: 'fix4-method-emission', phase: 'Fix', effort: 'high', schema: FIX })

const fix3 = await agent(COMMON + `
Gaps 1/2/4: ${fix1 && fix1.status}/${fix2 && fix2.status}/${fix4 && fix4.status}
Diagnosis gap 3: ${diag.gap3}
TASK — gap 3 (--diagnostics json LSDS surface): add a `--diagnostics json` CLI flag to shape-cli that emits the LSDS diagnostics (the shape-diagnostics Serialize types) as JSON to stdout/stderr on compile — including severity, a location with real file/line, and comptime_trace for comptime failures. This is P1's observation mechanism. Do NOT change the human-readable default. Build release; prove `comptime { error("field X needs a type") }` under `--diagnostics json` emits valid LSDS JSON with severity="error", a real location (not line 1), and a non-empty comptime_trace; and a `warning()` emits severity="warning". Commit WIP (git commit --no-verify -m 'WF-3D gap3 diagnostics-json wip').
Return status + files_changed + the JSON output samples (error + warning).`,
  { label: 'fix3-diagnostics-json', phase: 'Fix', effort: 'high', schema: FIX })

phase('Refute')
const PROBES = [
  { key: 'P8-flagship', prompt: 'P8 + gap1/gap4: the flagship showcase MUST work from fn main() AND as a method. Construct `type User { id: int, name: string }` with @json_schema + @to_json, then `fn main() { let u = User{id:1,name:"a"}; print(User_json_schema()); print(u.to_json()) }`. Run --mode vm AND --mode jit, 5 runs each, assert byte-identical correct JSON. A top-level-only pass is a REFUTE (the whole point of gap1 is fn-main visibility). Also confirm a user fn (not main) calling a generated fn works.' },
  { key: 'P3-typeinfo', prompt: 'P3 + gap2: `type_info(User)` bare-ident works; `.name`/`.kind` correct; `.fields` either returns FieldDescriptor rows with correct name/type/optional OR a clean jargon-free SURFACE (never a generic property error, never the sentinel). Run vm+jit.' },
  { key: 'P4-const', prompt: 'P4 + gap2c: an annotation handler iterating target.params on a function with a `const a: int` param asserts param.const == true (was false). Plus descriptor integrity under json-collision pressure (import std::core::json): {name,type,annotations,optional} exact. vm+jit.' },
  { key: 'P1-diag-json', prompt: 'P1 + gap3: `comptime { error("msg") }` under `--diagnostics json` emits LSDS JSON with severity=error, a real location (file/line, NOT line 1), non-empty comptime_trace; `warning()` -> severity=warning, build continues. No `<Bool>`, no `(line 1)`. Confirm the human-readable default is unchanged.' },
  { key: 'P10-jargon', prompt: 'P10 jargon firewall: a corpus of >=10 failing-comptime programs (type errors, watchdog, bad directives, missing schema ops, the NEW code paths you touched) — rendered stderr AND the new --diagnostics json output must contain NONE of: ckpt, ADR-, V3-S5, REFUSED, phase-2c, NotImplemented(SURFACE, W-<digit>, WF-, the § sign. Machine-grep.' },
  { key: 'P12-regression', prompt: 'P12 regression floor: implements(Dog,Greet)->true, build_config() works, set-return soundness (P5) still compile-errors not 139, __original__ base(5)=12 stable, state::hash 3-distinct still hold. The fixes must not regress the working core.' },
]
const verdicts = await parallel(PROBES.map(p => () =>
  agent(COMMON + `
ADVERSARIAL REFUTER for probe ${p.key}. Build release yourself. ${p.prompt}
Assume the claim is FALSE until your own run proves it. Return refuted=true with the exact failing command + output if it fails (or regresses), else refuted=false with the actual passing output (both modes where applicable).`,
    { label: 'refute:' + p.key, phase: 'Refute', effort: 'high', schema: { type: 'object', required: ['refuted', 'detail'], properties: { refuted: { type: 'boolean' }, detail: { type: 'string' } } } })
    .then(v => ({ probe: p.key, ...(v || { refuted: true, detail: 'agent died' }) }))
))

const survived = verdicts.filter(Boolean).filter(v => v.refuted)
log('Refuters: ' + verdicts.filter(Boolean).filter(v => !v.refuted).length + '/' + verdicts.length + ' passed; survived=' + survived.map(s => s.probe).join(','))

let repair = null
if (survived.length) {
  repair = await agent(COMMON + `
REFUTED probes: ${JSON.stringify(survived)}
TASK: Repair every refuted defect. Re-run each refuted probe's exact command yourself (vm+jit) until it holds. Commit WIP (git commit --no-verify -m 'WF-3D repair wip'). Blocked only on a genuine wall (surface with mechanism). No forbidden machinery; keep the jargon firewall passing.
Return status + summary + which probes now hold + issues.`,
    { label: 'repair', phase: 'Refute', effort: 'high', schema: FIX })
}

phase('Finisher')
const final = await agent(COMMON.replace('Only the finisher commits. Every agent\'s final message IS machine-consumed structured output.', 'You ARE the finisher — you commit. Your final message IS machine-consumed structured output.') + `
State: fixes visibility/type_info/method/diag-json = ${fix1 && fix1.status}/${fix2 && fix2.status}/${fix4 && fix4.status}/${fix3 && fix3.status}; survived-refuters=${JSON.stringify(survived.map(s => s.probe))}; repair=${JSON.stringify(repair && repair.summary)}. Prior stages left WIP commits + possibly uncommitted tail.
LONG-COMMAND PROTOCOL (mandatory): NEVER end your turn while a command runs; NEVER use run_in_background; run each build/test as a FOREGROUND call with a large timeout (up to 600000ms); on timeout RE-RUN the same command until it returns. Only then proceed.
STEPS (each a foreground call in ${WT}):
1. Stage+commit any uncommitted tail (git add -A && git commit --no-verify -m 'WF-3D finalize') after confirming content guards.
2. ${DX} just check-clean; ${DX} just check-no-dynamic; bash scripts/verify-merge.sh (expect 15/15).
3. ${DX} just test (pinned pre-existing OK: 4 shape-jit jit_closure_capture_* + pb3 flaky + 2 object-spread #[ignore]'d; anything new blocks).
4. ${DX} just diff-vmjit --fresh (build+run; repeat on timeout) — MATCH>=466, unexpected=0.
5. BOOK FLIP: update the comptime showcase gate corpus + book so the flagship fn-main / u.to_json() form is now runnable=true and gate-covered (it was dodged via top-level-only calls before). Run the book truth-gate on the adv-comptime chunk; it must stay green with the flipped examples.
6. Final commit trailer:
Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
Return status green/yellow/red + commit shas + a P1-P14 result table + residuals routed to named lanes (expect: JIT-generated-symbol P9/WF-1A(c), S6-packaging verification, expand-comptime generated-fn reporting, type_info.fields V3-S5-carrier if it SURFACEd).`,
  { label: 'finisher', phase: 'Finisher', effort: 'high', schema: { type: 'object', required: ['status', 'summary'], properties: {
    status: { enum: ['green', 'yellow', 'red'] }, summary: { type: 'string' }, commits: { type: 'array', items: { type: 'string' } }, probe_table: { type: 'string' }, residuals: { type: 'string' },
  }}})

return { flagship: String(diag.flagship_mechanism).slice(0, 150), fixes: { visibility: fix1 && fix1.status, type_info: fix2 && fix2.status, method: fix4 && fix4.status, diag_json: fix3 && fix3.status }, survived: survived.map(s => s.probe), final }
