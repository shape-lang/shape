export const meta = {
  name: 'wf3d-comptime-flagship-verify',
  description: 'WF-3D comptime excellence (priority-spine): the four flagship comptime features (1 generated-function visibility, 2 type_info reflection, 3 diagnostics-json/LSDS, 4 impl-level method emission) APPEAR already-landed at HEAD per an architecture scan — but that verdict was NOT executed. This is a VERIFY-FIRST lane: independently EXECUTE all four end-to-end under --mode vm AND --mode jit (assert green + VM==JIT), because "X works" claims that were never run are the WF-2D/WF-2F over-claim trap. Fix at ROOT anything not actually green. Then do the DECIDED type_info drift-cleanup (TypeKind Float->Number, TypeVar+Unknown->Unresolved per ratified §4.1.2; reconcile the stale stdlib TypeInfo decl to single-owner lockstep §4.3.6) — a single-owner lockstep edit touching the KIND STRING NAMES comptime user code compares against, so update every producer/consumer together + check blast radius. Land gate-runnable shape-test regression tests exercising all four vm+jit. Independent Opus verify. Element-type threading for generic params stays deferred (design-first, out of v1).',
  phases: [
    { title: 'Execute-Verify', detail: 'run the 3 book snippets + 3 stdlib showcases + type_info/diagnostics probes under vm AND jit; per-feature ACTUAL verdict' },
    { title: 'Fix-Cleanup', detail: 'root-fix any non-green feature; type_info TypeKind rename + TypeInfo reconcile (lockstep); gate tests' },
    { title: 'Verify', detail: 'independent Opus fresh-context re-execute all 4 vm+jit + rename blast radius' },
    { title: 'Finish', detail: 'gates + 4-feature vm+jit regression tests + any shape-web snippet-sync note' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-comptime'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/comptime-flagship-verify, off main HEAD). Build/run via: ' + DX + ' <cmd> (devenv toolchain not auto-loaded).',
  '',
  'FOUR FLAGSHIP COMPTIME FEATURES (each reportedly already-landed at HEAD — VERIFY BY EXECUTION, do not trust the architecture claim):',
  '  1. Generated-function visibility: a type-targeting @comptime handler materializes a generated FREE FUNCTION visible/callable from user code (incl. fn main), vm AND jit. Machinery: materialize_computed_comptime_extends (crates/shape-vm/src/compiler/functions_annotations.rs:1193). Showcases: stdlib-src/serde/derive.shape:95 emits {Type}_json_schema(); stdlib-src/llm/tools.shape:67 emits {fn}_tool_def().',
  '  2. type_info reflection: comptime builtin type_info(T) returns {name, kind, fields:Array<FieldDescriptor>}; type_info(T).fields[i].name must resolve (not unknown). build_type_info_heap_value (crates/shape-vm/src/compiler/comptime_builtins.rs:841); inference arm crates/shape-runtime/src/type_system/inference/access.rs:869. Reserved schema __ComptimeTypeInfo (type_schema/builtin_schemas.rs:288).',
  '  3. diagnostics-json/LSDS: comptime error()/warning() render machine-readable LSDS/JSON when output_format()==Json. crates/shape-vm/src/compiler/comptime_diagnostics.rs (build_comptime_failure:52, surface_comptime_warnings:95). CLI bin/shape-cli/src/diagnostics_json.rs.',
  '  4. impl-level method emission: @annotation emits `extend {Type} { method m() -> T { .. } }`, dispatchable via value.m(), vm AND jit. Showcase: stdlib-src/serde/serialize.shape:55 (@to_json). apply_comptime_extend (functions_annotations.rs:1121).',
  '',
  'BOOK SNIPPETS (in ../shape-web/book/snippets/advanced/, each with a .expected): derive_json_schema.shape, derive_to_json.shape, llm_tool_schema.shape. Run each: ' + DX + ' cargo run --bin shape -- run <snippet> --mode vm  AND  --mode jit; the stdout must match the .expected AND vm==jit. These are INPUT files — run them, do not rewrite them to pass.',
  '',
  'PHASE 1 — EXECUTE-VERIFY (no fix yet): run all 3 book snippets + drive the 3 stdlib showcases (write a tiny user program that imports each showcase and calls the generated fn/method, e.g. a type with @derive_json_schema then call {Type}_json_schema(); a @to_json type then value.to_json()) under --mode vm AND --mode jit. Also probe: type_info(SomeType).fields[0].name prints correctly; a comptime error()/warning() with --format json (or the equivalent flag) emits JSON/LSDS not human text. For EACH of the 4 features record the ACTUAL execution verdict (GREEN vm+jit / VM-only / BROKEN + exact error), NOT an architecture guess. Do NOT commit.',
  '',
  'PHASE 2 — FIX + CLEANUP: (a) root-fix any feature Phase 1 found not actually green vm+jit (no band-aid, no forbidden patterns). (b) type_info DRIFT-CLEANUP (decided, mechanical but LOCKSTEP): in crates/shape-runtime/stdlib-src/core/types.shape rename TypeKind Float->Number and collapse TypeVar+Unknown->Unresolved (ratified §4.1.2); reconcile the user-facing TypeInfo decl (types.shape:95) so it carries `fields` OR retire the dead 2-field decl and document __ComptimeTypeInfo as the single owner (§4.3.6 single-owner lockstep). CRITICAL: these variant names are also KIND STRING NAMES compared by comptime user code (types.shape:85 comment) — grep EVERY producer/consumer of "Float"/"TypeVar"/"Unknown" as a TypeKind (Rust build_type_info_heap_value kind strings, builtin_schemas, any stdlib/test/snippet string compare) and update them in lockstep so no site is left on the old name. Verify no blast-radius breakage.',
  '',
  'CONSTRAINTS (CLAUDE.md): NO forbidden patterns (is_heap/tag-decode/ValueWord/Bool-default/generic-opcode/parallel-discriminator/Convert*To). NO runtime coercion. int/number separate. ADR-005/006 preserved. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule + book gate): land gate-runnable shape-test regression tests (tools/shape-test/) that exercise ALL FOUR features and assert GREEN + VM==JIT: generated-fn call, method emission dispatch, type_info(T).fields[i].name, and diagnostics-json shape. Keep all COMMITTED changes inside the shape/ repo; if a ../shape-web book snippet .expected genuinely must change due to the kind-string rename, apply it on disk and REPORT it clearly in the finish output (it will be committed to shape-web separately, outside this merge).',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Execute-Verify')
const V1_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['f1_genfn', 'f2_typeinfo', 'f3_diagnostics', 'f4_method', 'any_broken', 'evidence'],
  properties: {
    f1_genfn: { type: 'string', enum: ['green-vm-jit', 'vm-only', 'broken'] },
    f2_typeinfo: { type: 'string', enum: ['green-vm-jit', 'vm-only', 'broken'] },
    f3_diagnostics: { type: 'string', enum: ['green-vm-jit', 'vm-only', 'broken'] },
    f4_method: { type: 'string', enum: ['green-vm-jit', 'vm-only', 'broken'] },
    any_broken: { type: 'boolean', description: 'true if ANY feature is not green vm+jit (needs a root fix in Phase 2)' },
    evidence: { type: 'string', description: 'the exact commands run + observed output/exit per feature; VM==JIT results; any error' },
  },
}
const v1 = await agent(CTX + '\n\nPHASE 1 — EXECUTE-VERIFY only. Run the snippets + showcases + probes under vm AND jit; report the ACTUAL per-feature execution verdict. Do NOT commit.',
  { label: 'execute-verify', phase: 'Execute-Verify', effort: 'high', schema: V1_SCHEMA })

phase('Fix-Cleanup')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'fixes', 'rename_done', 'lockstep_sites', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    fixes: { type: 'string', description: 'any root-fix applied for a non-green feature (or "none — all 4 already green")' },
    rename_done: { type: 'boolean', description: 'TypeKind Float->Number + TypeVar+Unknown->Unresolved + TypeInfo decl reconciled, lockstep' },
    lockstep_sites: { type: 'string', description: 'every producer/consumer of the renamed kind strings updated (list them); blast radius clean' },
    evidence: { type: 'string', description: 'captured: all 4 green vm+jit; rename has no broken site; check-no-dynamic EXIT 0' },
  },
}
const fix = await agent(CTX + '\n\nPHASE 1 VERDICT: ' + JSON.stringify(v1) + '\n\nPHASE 2 — FIX + CLEANUP. Root-fix any non-green feature; do the type_info TypeKind rename + TypeInfo reconcile in strict lockstep (update EVERY kind-string producer/consumer); add the 4-feature vm+jit gate tests. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "WF-3D comptime flagship: verify 4 features green vm+jit + type_info TypeKind rename lockstep + gate tests").',
  { label: 'fix-cleanup', phase: 'Fix-Cleanup', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'all_four_green', 'rename_no_regression', 'vm_jit_parity', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    all_four_green: { type: 'boolean', description: 'from YOUR OWN scratch runs: all 4 features green under vm AND jit' },
    rename_no_regression: { type: 'boolean', description: 'the TypeKind rename left no site on the old name; kind strings consistent everywhere' },
    vm_jit_parity: { type: 'boolean', description: 'each feature produces identical output vm vs jit' },
    evidence: { type: 'string', description: 'your own from-scratch command runs + a grep proving no stale Float/TypeVar/Unknown kind-string; concise' },
  },
}
const verify = await agent(CTX + '\n\nPHASE 1: ' + JSON.stringify(v1) + '\nFIX: ' + JSON.stringify(fix) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). Assume the "already working" claim is UNPROVEN until you personally run it. From scratch, under --mode vm AND --mode jit: (1) run the 3 book snippets — do they exit 0 with .expected output AND vm==jit? (2) call a generated free fn + a @-emitted method from user code — dispatch works both modes? (3) type_info(T).fields[i].name resolves + prints? (4) comptime error/warning emits JSON/LSDS? (5) grep the whole tree for stale "Float"/"TypeVar"/"Unknown" TypeKind kind-strings — any left = incomplete lockstep. Any feature vm-only or broken, any vm!=jit, any stale kind-string = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests_added', 'shapeweb_sync', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-runtime/shape-vm + the 4-feature tests, brief' },
    tests_added: { type: 'string', description: 'the 4-feature vm+jit gate tests' },
    shapeweb_sync: { type: 'string', description: 'any ../shape-web book snippet .expected change needed (or "none")' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Ensure the 4-feature vm+jit gate tests are committed; report any ../shape-web snippet sync needed; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-runtime and -p shape-vm (comptime/type areas) + the new shape-test regression tests. Commit (git commit --no-verify -m "WF-3D comptime flagship finalize: 4-feature vm+jit gate tests").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { v1, fix, verify, finish }
