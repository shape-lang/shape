export const meta = {
  name: 'wf-xml-empty-array',
  description: 'Tail #2 (user-greenlit 2026-07-07): fix the #14 xml::stringify core-dump = the empty-typed-array construction gap. Root (VM-only, NO shape-jit — confirmed by scoping): compile_expr_array (crates/shape-vm/src/compiler/expressions/collections.rs:671) emits a generic OpCode::NewArray Count(0) for an empty literal whose element type it cannot resolve; the VM op_new_array handler (crates/shape-vm/src/executor/objects/object_creation.rs:358-378) is a hard NotImplemented surface (ckpt5_surface) because the polymorphic TypedArrayData carrier was deleted in V3-S5 (every array must now be a monomorphic TypedArray<T>, and count==0 gives no element to infer T from). The typed empty opcodes NewTypedArray*(0) ALREADY work in VM+JIT; and `let x: Array<int> = []` / `f([])` already resolve T from the binding/param. The GAP: `let n: Node = { kids: [] }` (struct-field type known) and `xml::stringify({children: []})` (element type known from the param/marshal boundary) do NOT thread the target type into the object-literal field / arg, so `[]` stays generic and surfaces. STRICT-SAFE DESIGN (aligned with the user\'s let-generalization ruling): give an empty `[]` a fresh element type variable Array<T> and RESOLVE T from context via unification — binding annotation, STRUCT-FIELD type, PARAM type (bidirectional), a later push, or the marshal boundary — then emit the typed NewTypedArray*(0) opcode with the resolved T. If T is genuinely unresolvable by end of scope, it stays a COMPILE ERROR (strict — like the let-gen value restriction), NEVER an untyped/any array. DESIGN-FIRST: confirm this approach + flag any genuine strict-typing fork for ratification before implementing. Independent Opus verify vm+jit.',
  phases: [
    { title: 'Design', detail: 'map each failing case to a context source; confirm fresh-tyvar+unify strict-safe design; flag any fork' },
    { title: 'Implement', detail: 'empty-array element-type inference from context + emit typed opcode; unresolvable → compile error' },
    { title: 'Verify', detail: 'independent Opus: xml::stringify + struct-field-empty + various contexts work vm+jit; unresolvable errors; no untyped array' },
    { title: 'Finish', detail: 'gates + regression tests (incl. xml::stringify green) + no new #[ignore]' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-xml'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/xml-empty-array, off main HEAD). Build/run via: ' + DX + ' <cmd>. This is VM/COMPILER-side only — the scoping confirmed NO shape-jit change is needed (the JIT deopts the generic NewArray to the VM; the typed NewTypedArray*(0) path already works in both). Do NOT touch shape-jit.',
  '',
  'THE GAP (scoped): compile_expr_array (crates/shape-vm/src/compiler/expressions/collections.rs:671) emits generic NewArray Count(0) when it cannot resolve the element type of an empty `[]`. compile_typed_object_literal (collections.rs:775) infers field types from field EXPRESSIONS only and never threads a declared/target field type into a field value. The re-keying into empty_array_accumulators that patches a placeholder into a typed opcode happens ONLY in VariableDecl (statements.rs:1391/6558/7055), so an empty `[]` inside an object literal (or a bare arg) never gets patched and reaches op_new_array(0) -> ckpt5_surface (object_creation.rs:378). WORKING cases (element type resolved): `let x: Array<int> = []`, `f([])` with `f(xs: Array<int>)`. FAILING cases: `let n: Node = { kids: [] }` (Node.kids declared Array<int>), and `xml::stringify({children: []})` (element type known only from stringify\'s param/marshal boundary).',
  '',
  'STRICT-SAFE APPROACH (design + confirm): treat an empty `[]` as Array<T> for a FRESH type variable T (Type::Variable(TypeVar::fresh()) — the standard no-any inference path), and resolve T by UNIFICATION with every available context: the binding annotation, the enclosing STRUCT-FIELD type (thread the target field type from the object literal\'s expected/declared type into compile_typed_object_literal), the PARAM type when the object/array is an argument (bidirectional inference from the callee signature — the marshal boundary for xml::stringify), a subsequent push (element type), or a return-type annotation. Once T resolves to a concrete FieldType at compile time, emit the already-working typed NewTypedArray*(0) opcode for that T. If T is genuinely UNRESOLVABLE by end of scope, it is a COMPILE ERROR (strict — mirror the let-generalization value-restriction discipline), NEVER an untyped/any/Bool-default array. This aligns with the ruled HM let-generalization + generic-types-require-args stance.',
  '',
  'PHASE 1 DESIGN (no impl): for EACH failing case (`let n: Node = {kids:[]}`, `xml::stringify({children:[]})`, and any other empty-in-context form you find), identify the exact context source for T and confirm the fresh-tyvar+unify path can resolve it. Confirm the xml case specifically: does bidirectional inference from stringify\'s param type reach the `{children: []}` field? If a case is genuinely unresolvable-but-should-work (a real strict-typing fork — e.g. the design would need to allow a deferred-element-type empty array persisting past scope), FLAG it for ratification rather than deciding it. Otherwise proceed.',
  '',
  'PHASE 2 IMPLEMENT: thread the target/expected element type into empty-array construction in object-literal fields + call arguments (bidirectional), resolve via the fresh-tyvar unify path, emit the typed opcode. Genuinely-unresolvable empty array -> a clear compile error. NO forbidden patterns (no untyped-array escape hatch, no Bool-default, no runtime coercion). Strict-typing intact (int/number separate).',
  '',
  'CONSTRAINTS: ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. Do NOT touch shape-jit. Do NOT reintroduce a polymorphic/untyped array carrier. Verify vm AND jit.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): xml::stringify on a node with empty children/attributes runs green vm+jit; `let n: Node = { kids: [] }` (struct field Array<int>) compiles+runs; `[]` resolves T from binding/param/push in several forms; a genuinely-unresolvable empty array is a clean compile error (not a crash, not an untyped array). No new #[ignore].',
  '',
  'STRUCTURED-OUTPUT: ONE clean JSON object, 1-4 plain sentences per field, NO XML/code blocks in fields.',
].join('\n')

phase('Design')
const D_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['case_map', 'xml_resolvable', 'strict_fork', 'plan'],
  properties: {
    case_map: { type: 'string', description: 'each failing empty-array case -> its context source for T (binding/struct-field/param/push/return)' },
    xml_resolvable: { type: 'boolean', description: 'the xml::stringify({children:[]}) case is resolvable strict-safely via bidirectional param inference' },
    strict_fork: { type: 'string', description: 'any genuinely-unresolvable-but-should-work case that needs a user strict-typing ruling (or "none — all resolvable or correctly-error")' },
    plan: { type: 'string', description: 'the fresh-tyvar+unify threading plan + emit-typed-opcode' },
  },
}
const d = await agent(CTX + '\n\nPHASE 1 — DESIGN (no impl). Map cases, confirm xml resolvable, flag any strict fork. Do NOT commit.',
  { label: 'design', phase: 'Design', effort: 'xhigh', schema: D_SCHEMA })

// RATIFIED 2026-07-07 (user): Option (a) canonical-instantiate (HM-consistent).
const RULING = [
  'USER RATIFICATION (2026-07-07) — the strict_fork is DECIDED: Option (a) CANONICAL-INSTANTIATE (HM-consistent).',
  'RULE: an empty array literal `[]` is a generalized value `∀T. Array<T>`; when it reaches an UNCONSTRAINED monomorphic boundary (a polymorphic `_`/PolymorphicArg param, or the marshal sink) with NO element-type context, instantiate T to a CANONICAL unit type and lower to the canonical typed empty array (the already-working NewTypedArray*(0) form for that canonical T). This is SOUND because an empty, never-pushed array\'s T is provably unobserved at such a sink; and it is consistent with the ruled let-generalization (a generalized empty array instantiated at a canonical T at an unconstrained monomorphic boundary). It is NOT an untyped/any/Bool-default carrier — it is a concrete monomorphic TypedArray<Unit> (or the canonical unit element chosen). Add a BOUNDED ADR-006 note documenting exactly this (context-free empty array at an unconstrained sink → canonical unit instantiation; T-observed cases still require + get a real T).',
  'SO: implement BOTH — (F1/F2/F4/F5) the strict-safe struct-field/param/return empty-array element-type threading (fresh tyvar + unify), AND (F3 xml) the canonical-instantiate rule so `xml::stringify({children: []})` compiles + runs green vm+jit. Genuinely-unresolvable-AND-observed empties (e.g. `let xs = []` with a later push of mismatched type, or an empty whose T IS observed but unconstrained) keep the existing clean compile error — canonical-instantiate applies ONLY where T is provably unobserved at an unconstrained sink. Do NOT let canonical-instantiate leak into a position where T is later observed (that must still resolve or error).',
].join('\n')

phase('Implement')
const I_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'files_changed', 'xml_works', 'unresolvable_errors', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    files_changed: { type: 'string', description: 'the element-type-inference threading + typed-opcode emission, brief' },
    xml_works: { type: 'boolean', description: 'xml::stringify on empty children/attributes runs green vm+jit' },
    unresolvable_errors: { type: 'boolean', description: 'a genuinely-unresolvable empty array is a clean compile error (not crash, not untyped)' },
    evidence: { type: 'string', description: 'xml + struct-field-empty + context forms work; unresolvable errors cleanly; check-no-dynamic EXIT 0' },
  },
}
const impl = await agent(CTX + '\n\nDESIGN: ' + JSON.stringify(d) + '\n\n' + RULING + '\n\nPHASE 2 — IMPLEMENT per the RULING: the fresh-tyvar+unify empty-array element-type inference (F1/F2/F4/F5) AND the canonical-instantiate rule for F3 (context-free empty at an unconstrained `_`/marshal sink → canonical unit typed empty). ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "Fix #14 xml empty-array: context element-type inference (fresh tyvar+unify) + canonical-unit instantiate at unconstrained sinks (user-ratified); typed opcode; observed-unresolvable -> compile error").',
  { label: 'implement', phase: 'Implement', effort: 'xhigh', schema: I_SCHEMA })

phase('Verify')
const V_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'xml_works', 'contexts_work', 'strict_intact', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    xml_works: { type: 'boolean', description: 'from YOUR OWN run: xml::stringify with empty children/attributes green vm+jit' },
    contexts_work: { type: 'boolean', description: 'let n: Node = {kids:[]} + []-from-binding/param/push all resolve + run vm+jit' },
    strict_intact: { type: 'boolean', description: 'genuinely-unresolvable empty array = clean compile error; no untyped/any array; no forbidden pattern; int/number separate' },
    evidence: { type: 'string', description: 'your own from-scratch runs incl. an unresolvable-error probe + a no-regression spread; concise' },
  },
}
const v = await agent(CTX + '\n\nDESIGN: ' + JSON.stringify(d) + '\nIMPL: ' + JSON.stringify(impl) + '\n\n' + RULING + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context. From scratch: (1) xml::stringify on a node with empty children/attributes green vm AND jit (canonical-instantiate rule)? (2) `let n: Node = {kids:[]}` + `[]` resolving T from binding/param/return/push — all compile+run vm+jit? (3) STRICT: is the canonical-instantiate a CONCRETE monomorphic TypedArray<Unit> (NOT an untyped/any/Bool-default carrier), and does it apply ONLY where T is provably unobserved at an unconstrained sink — i.e. an empty array whose T IS later observed but unconstrained still cleanly compile-errors (canonical-instantiate must NOT leak into an observed position)? Did the fix add any untyped-array escape hatch / Bool-default / coercion / forbidden pattern? (4) no regression: existing array construction (non-empty, annotated-empty) still works. Any crash, untyped-array leak into an observed position, forbidden pattern, or regression = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'xhigh', schema: V_SCHEMA })

phase('Finish')
const F_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests_added', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-vm/shape-runtime array/xml tests, brief' },
    tests_added: { type: 'string', description: 'xml::stringify-green + struct-field-empty + unresolvable-error regression tests' },
    merge_ready: { type: 'boolean' },
  },
}
const f = await agent(CTX + '\n\nIMPL: ' + JSON.stringify(impl) + '\nVERDICT: ' + JSON.stringify(v) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Add the regression tests (xml::stringify green vm+jit; struct-field empty; unresolvable-error); no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-vm and -p shape-runtime (array/xml/collection areas). Commit (git commit --no-verify -m "xml empty-array finalize: regression tests (#14)").',
  { label: 'finish', phase: 'Finish', effort: 'high', schema: F_SCHEMA })

return { d, impl, v, f }
