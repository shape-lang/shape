export const meta = {
  name: 'wf2afu-extension-hardening',
  description: 'CRIT x2 (user priority #3: polyglot + the modular extension system). (A) shape/extensions/*.so built by the DOCUMENTED flow (just build-extensions + --extension-dir) SIGSEGVs the host on load — debug/.so-vs-release-host profile skew that the ABI-version gate (plugins/loader.rs:118-153) does not catch. (B) [native-dependencies] alias resolution is DEAD: resolve_library_target (executor/control_flow/native_abi.rs:607) is a hardcoded c/m table returning any other alias verbatim, never consulting the project resolved native_dependency_scopes/lock — so every path/vendored alias (e.g. duckdb) is uncallable. Fix both; Fable independently re-proves the EXACT documented flows.',
  phases: [
    { title: 'Diagnose', detail: 'confirm the SIGSEGV root (profile/allocator skew) + the native-deps alias resolution gap' },
    { title: 'Fix', detail: 'A: matching-profile build + clean-fail load validation; B: thread resolved native-deps into resolve_library_target' },
    { title: 'Fable-verify', detail: 'independent: the documented build-extensions+--extension-dir flow runs; a [native-dependencies] alias is callable' },
    { title: 'Finish', detail: 'gates + regression tests' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-wf2afu-ext'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = `
Work IN ${WT} (branch wave3/extension-hardening, off main @800fb6b9). Build/test via: ${DX} <cmd>.

TWO CRITICAL DEFECTS (user priority #3 = polyglot works with the modular extension system):

CRIT-A [extension .so load SIGSEGV on the DOCUMENTED flow]: the book documents building extensions with 'just build-extensions' and loading them with 'shape run --extension-dir ./extensions prog.shape' (a program using 'fn python'/'fn typescript'). Loading the shape/extensions/*.so produced by that flow SIGSEGVs the host process. Suspected root: 'just build-extensions' builds the .so in a profile (debug) that is ABI/allocator/panic-strategy-incompatible with the (release) host binary; the loader's ABI-version symbol gate (crates/shape-runtime/src/plugins/loader.rs:118-153) checks a version integer but NOT profile/allocator compatibility, so a skewed .so passes the gate then crashes. CONFIRM the exact root (is it profile skew? a missing symbol? allocator mismatch?).
ACCEPTANCE-A: the documented flow (just build-extensions -> shape run --extension-dir ./extensions <prog using fn python>) executes the foreign body and returns the correct value with NO SIGSEGV, in both --mode vm and --mode jit. AND a genuinely-incompatible .so fails CLEANLY at load (clear diagnostic), never a segfault. Prefer: (i) make build-extensions produce a host-compatible artifact (matching profile), AND (ii) add load-time validation that rejects an incompatible .so cleanly (extend the loader.rs gate — e.g. a build-profile/rustc-version/abi-tag symbol the extension exports and the host checks) so the SIGSEGV path becomes a clean Err.

CRIT-B [[native-dependencies] alias resolution dead]: resolve_library_target (crates/shape-vm/src/executor/control_flow/native_abi.rs:607) hardcodes only "c"/"libc"/"m"/"libm" -> system libs and returns any OTHER alias verbatim (Library::new("duckdb") -> fails). It never consults the project's resolved native_dependency_scopes (collected in bundle_compiler.rs:532 via native_resolution.rs::resolve_native_dependencies_for_project + shape.toml [native-dependencies] + lock). So an 'extern C fn' whose 'library' is a declared [native-dependencies] alias (path/vendored, e.g. the packages/duckdb/ package or a project-local .so) is uncallable at runtime.
ACCEPTANCE-B: an extern C fn declared against a [native-dependencies] alias resolves the alias to its real (path/vendored) library target via the resolved native-dependency scopes and CALLS it successfully. link_native_function (native_abi.rs:629) must receive + consult the resolved native-dependency map, not just the hardcoded c/m table. Build a minimal repro: a tiny local C lib (cc -shared) declared as a [native-dependencies] path alias in a shape.toml project, an 'extern C fn' bound to it, called from main — returns the correct value.

HARD CONSTRAINTS (CLAUDE.md): stay on typed KindedSlot/NativeKind carriers; no ValueWord/tag-decode/Bool-default/raw-u64. ${DX} just check-no-dynamic must stay EXIT 0. Do not weaken the ABI-version gate; ADD compatibility validation, don't remove checks.
GENUINENESS: prove the foreign/native body actually executed (real value, not a stub/0). For CRIT-A prove no SIGSEGV via exit code 0 (not 139) across repeated runs.
`

phase('Diagnose')
const DIAG_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['crit_a_root', 'crit_b_root', 'crit_a_repro', 'crit_b_repro', 'fix_plan'],
  properties: {
    crit_a_root: { type: 'string', description: 'exact reason the documented-flow .so SIGSEGVs (profile skew? symbol? allocator?)' },
    crit_b_root: { type: 'string', description: 'exact reason a [native-dependencies] alias is uncallable + where the resolved scopes fail to reach resolve_library_target' },
    crit_a_repro: { type: 'string', description: 'commands that reproduce the SIGSEGV (exit 139) on current HEAD' },
    crit_b_repro: { type: 'string', description: 'commands that reproduce the alias-uncallable failure on current HEAD' },
    fix_plan: { type: 'string', description: 'concrete fix for each: build-profile change + loader validation (A); thread resolved native-deps into link_native_function (B)' },
  },
}
const diag = await agent(`${CTX}\n\nDIAGNOSE ONLY (no fix). Reproduce BOTH crits from scratch (capture exit 139 for A; the alias failure for B). Pinpoint exact roots + a concrete fix plan for each. Do NOT commit.`,
  { label: 'diagnose', phase: 'Diagnose', effort: 'high', schema: DIAG_SCHEMA })

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'crit_a_evidence', 'crit_b_evidence', 'no_forbidden'],
  properties: {
    status: { type: 'string', enum: ['both-fixed', 'partial', 'blocked'] },
    files_changed: { type: 'string' },
    crit_a_evidence: { type: 'string', description: 'documented flow now exit 0 (not 139), correct value, vm+jit; AND an incompatible .so fails cleanly' },
    crit_b_evidence: { type: 'string', description: 'a [native-dependencies] path alias extern C fn now resolves + returns the correct value' },
    no_forbidden: { type: 'boolean', description: 'true iff check-no-dynamic EXIT 0 and no Bool-default/ValueWord/tag-decode introduced' },
  },
}
const fix = await agent(`${CTX}\n\nDIAGNOSIS: ${JSON.stringify(diag)}\n\nIMPLEMENT both fixes. Build release + extensions. Prove BOTH acceptance criteria from scratch (A: no SIGSEGV + clean-fail on incompatible; B: alias callable). ${DX} just check-no-dynamic EXIT 0. Commit WIP (git add -A && git commit --no-verify -m 'WF-2A-fu extension load + native-deps alias wip').`,
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Fable-verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'crit_a', 'crit_b', 'no_segfault', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'MIXED'] },
    crit_a: { type: 'string', enum: ['fixed', 'still-segfaults', 'partial'] },
    crit_b: { type: 'string', enum: ['fixed', 'still-uncallable', 'partial'] },
    no_segfault: { type: 'boolean', description: 'true iff NO invocation of the documented flow exits 139 across your repeated runs' },
    evidence: { type: 'string', description: 'your own from-scratch runs of the EXACT documented flow + a fresh [native-dependencies] alias project; captured exit codes + values' },
  },
}
const verify = await agent(`${CTX}\n\nFIX CLAIM: ${JSON.stringify(fix)}\n\nYou are Fable, an INDEPENDENT adversarial verifier. Assume the fix is INSUFFICIENT until your own hands-on runs prove otherwise. From scratch: (A) run the EXACT book-documented flow (just build-extensions, then shape run --extension-dir ./extensions on a fn python + a fn typescript program) MANY times, vm+jit — any single exit 139 = REFUTED for A; also build a deliberately-incompatible .so and confirm it fails CLEANLY not via segfault. (B) create your OWN minimal [native-dependencies] project (tiny cc -shared C lib as a path alias, extern C fn bound to it) and confirm it's callable + returns the right value. Report exactly what you saw.`,
  { label: 'fable-verify', phase: 'Fable-verify', model: 'fable', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['check_clean', 'check_no_dynamic', 'tests_added', 'tests', 'merge_ready'],
  properties: {
    check_clean: { type: 'string' },
    check_no_dynamic: { type: 'string' },
    tests_added: { type: 'string', description: 'regression tests: an extension-load compatibility test + a [native-dependencies] alias resolution test — names + locations' },
    tests: { type: 'string', description: 'just test: new failures beyond pinned pre-existing, or "only pinned"' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(`${CTX}\n\nFIX: ${JSON.stringify(fix)}\nFABLE: ${JSON.stringify(verify)}\n\nFINISH (only if Fable CONFIRMED both; else merge_ready:false + what remains). Add regression tests: (i) extension-load compatibility validation (an incompatible/mismatched .so is rejected cleanly, not a segfault — use a test double if a real skewed .so is impractical), (ii) resolve_library_target/link_native_function resolves a [native-dependencies] alias to its declared target. Run ${DX} just check-clean, ${DX} just check-no-dynamic, ${DX} just test --no-fail-fast (or per-crate to dodge the shape-jit fail-fast). Commit (git commit --no-verify -m 'WF-2A-fu finalize: extension-load validation + native-deps alias resolution + tests'). Report merge readiness.`,
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { diag, fix, verify, finish }
