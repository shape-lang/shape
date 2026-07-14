export const meta = {
  name: 'wf-bool-default-guard-gap',
  description: 'Close a GUARD GAP in the forbidden-pattern enforcement machinery: `just check-no-dynamic` greps only the two-tuple `unwrap_or((0, NativeKind::Bool))` Bool-default form, so the SINGLE-ARG `unwrap_or(NativeKind::Bool)` read-fallback form slips through uncaught — 6 live occurrences (executor/vm_state_snapshot.rs:102/191/213, executor/objects/object_operations.rs:349, executor/mod.rs:906/1042). Each is a "kinds track SHOULD be lockstep-parallel to data; fall back to Bool if short" defensive sentinel — the exact CLAUDE.md-forbidden "soft-fail Bool-default for now / no-op sentinel rather than panic" shape. Per ADR-006 §2.7.7 the parallel-kind track is provably in-window, so the fallback never fires — the correct fix is surface-and-stop (panic loudly / the codebase surface idiom) so a real lockstep violation surfaces instead of silently fabricating Bool. FIX all 6 (with a per-site in-window proof; if any site is NOT provably in-window it is a REAL fabrication → thread the real kind, do NOT panic), then EXTEND check-no-dynamic + baseline to catch the single-arg form at limit 0. ASSESS + REPORT (do not necessarily fix) the adjacent Bool-as-kind uses (OOB `return (0u64, NativeKind::Bool)`, clear-write `module_binding_kinds[i] = NativeKind::Bool`, poison-hint `_ => NativeKind::Bool`) — flag any that are ALSO forbidden fabrications for a follow-up vs legitimate. Independent Opus verify (the guard actually catches a planted violation; no fix fires on any real path).',
  phases: [
    { title: 'Fix', detail: 'convert 6 single-arg Bool-default read-fallbacks to surface-and-stop (per-site in-window proof) + extend check-no-dynamic guard to limit 0' },
    { title: 'Verify', detail: 'independent Opus: guard catches a planted single-arg Bool-default; no surface fires on any real path; behavior unchanged' },
    { title: 'Finish', detail: 'gates + full shape-vm tests + baseline row + adjacent-Bool assessment report' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-booldefault'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/bool-default-guard-gap, off main HEAD). Build/test via: ' + DX + ' <cmd>. This is FORBIDDEN-PATTERN ENFORCEMENT work — precision matters; CLAUDE.md §Forbidden Patterns + the Bool-default rules are binding.',
  '',
  'THE GAP: `just check-no-dynamic` (scripts/check-no-dynamic.sh, per-symbol monotonic-non-increasing vs docs/check-no-dynamic-baseline.txt; format `<limit>\\t<ripgrep PCRE>\\t<note>`, "once 0 stays 0", adding a pattern starts at current count) greps for the TWO-TUPLE `unwrap_or\\(\\(0,\\s*NativeKind::Bool\\)\\)` form (baseline row ~39) but NOT the SINGLE-ARG `unwrap_or(NativeKind::Bool)` form. So single-arg Bool-default read-fallbacks slip through.',
  '',
  'THE 6 SINGLE-ARG SITES (all the same shape — `X.get(i).copied().unwrap_or(NativeKind::Bool)` where the kinds track is meant to be lockstep-parallel to the data slots): crates/shape-vm/src/executor/vm_state_snapshot.rs:102, :191, :213 (snapshot introspection accessors, `i` in `base..end.min(stack.len())`, kinds parallel to stack); crates/shape-vm/src/executor/objects/object_operations.rs:349 (`orig_idx` guarded `< src_slots.len()` at :340, src_kinds parallel to src_slots); crates/shape-vm/src/executor/mod.rs:906 (module_binding_kinds parallel to module_bindings; comment literally says "if the kinds vec is short ... falls back to the no-op sentinel rather than panicking on a release build" = the forbidden rationalization) and :1042 (guarded `index < module_bindings.len()` at :1035).',
  '',
  'FIX (per site): PROVE the ADR-006 §2.7.7 lockstep invariant makes the index in-window for that site (kinds vec is constructed parallel to / same length as the data vec, and the index is bounded by the data vec length). If provable-in-window → replace `.get(i).copied().unwrap_or(NativeKind::Bool)` with a SURFACE-AND-STOP that loudly fails if the invariant is ever violated instead of fabricating Bool: prefer direct index `X[i]` (panics on OOB — the surface) OR `.get(i).copied().expect("ADR-006 §2.7.7 lockstep: <vec> is parallel to <data>; index in-window")`. Match the surrounding surface idiom. If a site is NOT provably in-window (the kinds vec genuinely could be short in some real path) → that is a REAL Bool-default fabrication: thread the correct per-slot NativeKind from its true source (do NOT convert to panic — that would be a real crash; do NOT keep Bool). State which sites were provable vs real-fabrication.',
  '',
  'GUARD: add a baseline row to docs/check-no-dynamic-baseline.txt for the single-arg form — pattern `unwrap_or\\(NativeKind::Bool\\)` (this must NOT also match the two-tuple `unwrap_or((0, NativeKind::Bool))` form — verify the regex distinguishes them), limit 0 (after your fixes the count is 0), note "single-arg Bool-default read-fallback — ADR-006 §2.7.7 forbidden". Confirm ' + DX + ' just check-no-dynamic EXIT 0 after fixes + baseline row.',
  '',
  'ASSESS + REPORT (do NOT necessarily fix this lane — surface for a follow-up): the adjacent Bool-as-kind uses near these sites — the OOB `return (0u64, NativeKind::Bool)` (mod.rs:1035), the clear-write `module_binding_kinds[index] = NativeKind::Bool` (mod.rs:1043), and the `_ => NativeKind::Bool` poison-hint wildcards (snapshot paths, documented surface-and-stop). Classify each as (forbidden fabrication → route follow-up) or (legitimate: Bool is the real kind of a bool value / a documented poison-hint / the true NONE-slot kind). Do NOT fabricate a fix for a legitimate one.',
  '',
  'CONSTRAINTS (CLAUDE.md, CRITICAL): NO forbidden patterns introduced. Do NOT weaken the enforcement gate (the new baseline row must genuinely catch the form). The surface-and-stop conversions must NOT introduce a real crash on any reachable path — the whole point is they are unreachable-by-lockstep. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. Run ' + DX + ' cargo test -p shape-vm --lib to confirm NO new panic fires on any real path.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule + this IS enforcement): the deliverable is the extended check-no-dynamic guard row (limit 0) that catches the single-arg form — prove it catches a PLANTED single-arg `unwrap_or(NativeKind::Bool)` (add one temporarily → check-no-dynamic FAILs → remove it). No new #[ignore].',
  '',
  'STRUCTURED-OUTPUT: emit ONE clean JSON object, 1-4 plain sentences per field, NO XML tags / code blocks in fields.',
].join('\n')

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'sites_fixed', 'all_provable_inwindow', 'guard_added', 'adjacent_assessment', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    sites_fixed: { type: 'string', description: 'how each of the 6 was converted (surface-and-stop vs real kind-thread), brief' },
    all_provable_inwindow: { type: 'boolean', description: 'true if all 6 were provably-in-window (surface-and-stop); false if any was a real fabrication needing a kind-thread' },
    guard_added: { type: 'boolean', description: 'check-no-dynamic baseline row for single-arg form added at limit 0, distinguishes from two-tuple form, EXIT 0' },
    adjacent_assessment: { type: 'string', description: 'classification of the OOB-return / clear-write / poison-hint Bool uses (forbidden->follow-up vs legitimate)' },
    evidence: { type: 'string', description: 'captured: check-no-dynamic EXIT 0; shape-vm lib tests green (no new panic); the distinguishing-regex check' },
  },
}
const fix = await agent(CTX + '\n\nPHASE 1 — FIX the 6 sites + extend the guard + assess adjacents. Per-site in-window proof. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' cargo test -p shape-vm --lib green. Commit (git add -A && git commit --no-verify -m "Close Bool-default guard gap: 6 single-arg unwrap_or(NativeKind::Bool) read-fallbacks -> surface-and-stop + check-no-dynamic catches single-arg form").',
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const VERIFY_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'guard_catches_planted', 'no_surface_fires', 'no_behavior_change', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    guard_catches_planted: { type: 'boolean', description: 'you PLANTED a single-arg unwrap_or(NativeKind::Bool) and confirmed check-no-dynamic FAILs on it, then removed it' },
    no_surface_fires: { type: 'boolean', description: 'no converted surface-and-stop fires on any real path — full shape-vm lib + relevant integration tests green, no new panic' },
    no_behavior_change: { type: 'boolean', description: 'the 6 conversions are behavior-preserving (unreachable-by-lockstep); no functional change' },
    evidence: { type: 'string', description: 'your own planted-violation run + full test run + regex-distinguishes-two-forms check; concise' },
  },
}
const verify = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context (you did NOT write this). This is ENFORCEMENT-GATE work — be maximally skeptical. (1) PLANT a single-arg `unwrap_or(NativeKind::Bool)` in a shape-vm source file and run ' + DX + ' just check-no-dynamic — it MUST FAIL (catch it); then remove the plant. If the guard does NOT catch the planted form, REFUTED. (2) Confirm the new regex does NOT double-count / mis-catch the legitimate two-tuple form or match-arm/poison-hint Bool uses. (3) Confirm NO converted surface-and-stop fires on any real path: run ' + DX + ' cargo test -p shape-vm --lib (and any snapshot/module-binding integration tests) — any new panic/failure = REFUTED (means a site was NOT actually in-window and the fix introduced a crash — it should have been a kind-thread instead). (4) Confirm the 6 conversions are behavior-preserving. Any uncaught planted violation, new panic, or mis-classified adjacent = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: VERIFY_SCHEMA })

phase('Finish')
const FINISH_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'guard_row', 'followups', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-vm lib, brief' },
    guard_row: { type: 'string', description: 'the baseline row added (pattern + limit 0)' },
    followups: { type: 'string', description: 'any adjacent Bool-use routed as a follow-up (or "none — all adjacents legitimate")' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Ensure the baseline guard row is committed (limit 0); report any adjacent-Bool follow-up; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-vm --lib. Commit (git commit --no-verify -m "Bool-default guard-gap finalize: single-arg form guarded at 0 + adjacent assessment").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: FINISH_SCHEMA })

return { fix, verify, finish }
