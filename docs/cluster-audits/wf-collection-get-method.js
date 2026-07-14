export const meta = {
  name: 'wf-collection-get-method',
  description: 'Book cluster C4 (user-greenlit): Array.get(i)/Vec.get(i) are missing — the book (traits.mdx:172) shows `.get(i)` returning Option<T> but it errors "Method not found". Add a bounds-safe `.get(index: int) -> Option<T>` PHF method to Array (and Vec if distinct) returning Some(elem) in-bounds / None out-of-bounds — the standard safe indexed accessor (vs `arr[i]` which is the checked direct index). Method registry: crates/shape-vm/src/executor/objects/method_registry.rs (has first/last/push arms + handle_* dispatch). Independent Opus verify vm+jit.',
  phases: [
    { title: 'Fix', detail: 'add .get(i:int)->Option<T> PHF entry + handler (Some in-bounds / None OOB) for Array/Vec' },
    { title: 'Verify+Finish', detail: 'independent Opus: get returns Some/None correctly vm+jit; the book fence green; gates + tests' },
  ],
}
const WT = '/home/dev/dev/shape-lang/shape-w7-arrayget'
const DX = 'direnv exec /home/dev/dev/shape-lang'
const CTX = [
  'Work IN ' + WT + ' (branch wave7/collection-get-method, off main HEAD). Build/run via: ' + DX + ' <cmd>.',
  'TASK: add a bounds-safe `.get(index: int) -> Option<T>` method to Array (and Vec if it is a distinct dispatch) — Some(elem) when 0<=index<len, None otherwise. It is the safe accessor complementing `arr[i]` (checked direct index) and `.first()`/`.last()`. Method registry is crates/shape-vm/src/executor/objects/method_registry.rs (PHF names ~line 103-137 + handle_* arms ~287; mirror handle_first_v2/handle_last_v2 in crates/shape-vm/src/executor/objects/array_basic.rs). Return type is Option<T> where T is the array element type (generic method signature per the existing generic-method-signature system). The index param is `int` (strict — not number). Works vm AND jit.',
  'CONSTRAINTS (CLAUDE.md): NO forbidden patterns; strict typing (index:int, Option<T> element-typed); NO Bool-default; NO dynamic fallback. ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0.',
  'REGRESSION-TEST (user rule): `[10,20,30].get(1)` == Some(20), `.get(5)` == None, `.get(-1)` == None (or per the chosen OOB rule — document), vm+jit; the traits.mdx:172 book fence runs green. No new #[ignore].',
  'STRUCTURED-OUTPUT: ONE clean JSON object, 1-4 plain sentences per field, NO XML/code blocks.',
].join('\n')
phase('Fix')
const F = { type:'object', additionalProperties:false, required:['status','files_changed','get_works','evidence'], properties:{ status:{type:'string',enum:['done','partial','blocked']}, files_changed:{type:'string',description:'the .get PHF entry + handler'}, get_works:{type:'boolean',description:'.get returns Some in-bounds / None OOB, element-typed, vm+jit'}, evidence:{type:'string',description:'get(1)=Some(20), get(5)=None vm+jit; check-no-dynamic EXIT 0'} } }
const fix = await agent(CTX + '\n\nPHASE 1 — FIX: add .get(i:int)->Option<T> for Array/Vec. ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "Add Array/Vec .get(i:int)->Option<T> bounds-safe accessor (book C4)").', { label:'fix', phase:'Fix', effort:'high', schema:F })
phase('Verify+Finish')
const V = { type:'object', additionalProperties:false, required:['verdict','get_works','no_regression','gates','merge_ready'], properties:{ verdict:{type:'string',enum:['CONFIRMED','REFUTED','PARTIAL']}, get_works:{type:'boolean',description:'from YOUR OWN run: get Some/None correct, element-typed, vm+jit'}, no_regression:{type:'boolean',description:'first/last/push/index still work; no method-registry regression'}, gates:{type:'string',description:'check-clean + check-no-dynamic + the get tests'}, merge_ready:{type:'boolean'} } }
const v = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context. From scratch: .get(i) returns Some(elem) in-bounds + None OOB, element-typed, identical vm+jit? first/last/push/[i] unregressed? Add the regression tests + confirm the traits.mdx:172 fence; no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-vm (array areas). Any wrong result, type-erasure, or regression = REFUTED. Commit tests (git commit --no-verify -m "Array/Vec .get finalize: regression tests").', { label:'verify-finish', phase:'Verify+Finish', effort:'high', schema:V })
return { fix, v }
