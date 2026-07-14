export const meta = {
  name: 'wf-closure-typed-param-grammar',
  description: 'Tail #4 (verify-first resolved: fix Bug #2 now, DEFER Bug #1). Bug #2: an unbraced typed-param closure `|x: int| x + 1` mis-parses because pipe_lambda -> function_param -> type_annotation -> union_type is greedy (`("|" ~ intersection_type)*` never backtracks in PEG), so it swallows the closure-closing `|` reading `int | x` as a union type and the lambda fails. FIX (small, local, low-risk per scoping): give pipe_lambda a CLOSURE-SCOPED param whose type annotation uses intersection_type (NOT the top-level union_type), referenced ONLY from pipe_lambda — so the closing `|` is no longer swallowed. Union-typed closure params then require parens `|x: (int | str)|` (acceptable). Does NOT touch shared function_param (real fn signatures are paren-delimited so their union types stay unambiguous on type_annotation). Grammar: crates/shape-ast/src/shape.pest (pipe_lambda:492, function_param:461, union_type:824, intersection_type:829); parser: crates/shape-ast/src/parser/expressions/functions.rs:37 parse_pipe_lambda + functions.rs:67 parse_function_param. Bug #1 (tail-closure-eaten-as-bitwise-or across newline) is DEFERRED — it is a grammar-wide ASI restructure (\n is silent WHITESPACE everywhere), high-risk, with a `;` workaround; out of scope. Independent Opus verify (parses + runs + NO grammar regression + backtracking-timeout tests green).',
  phases: [
    { title: 'Fix', detail: 'closure-scoped param type (intersection_type, no top-level union) referenced only from pipe_lambda + matching parser tweak' },
    { title: 'Verify', detail: 'independent Opus: |x: int| x+1 parses+runs; |x:(int|str)| parens-union works; no grammar/backtracking regression' },
    { title: 'Finish', detail: 'gates + regression tests + note Bug #1 deferred' },
  ],
}

const WT = '/home/dev/dev/shape-lang/shape-w7-grammar'
const DX = 'direnv exec /home/dev/dev/shape-lang'

const CTX = [
  'Work IN ' + WT + ' (branch wave7/closure-typed-param-grammar, off main HEAD). Build/test via: ' + DX + ' <cmd>.',
  '',
  'BUG #2 (the only bug in scope): `|x: int| x + 1` fails to parse. Root: pipe_lambda (shape.pest:492) = `"|" ~ function_params? ~ "|" ~ ...`; function_param (:461) has `(":" ~ type_annotation)?`; type_annotation (:820) = union_type (:824) = `intersection_type ~ ("|" ~ intersection_type)*`. PEG `*` is greedy + never backtracks, so on `|x: int| x + 1` the union_type after `: int` swallows the closure-closing `|` (reads `int | x`), and pipe_lambda then can\'t find its closing `|` → fails.',
  '',
  'FIX (small + local, per the scoping): introduce a closure-scoped param + closure-scoped type annotation used ONLY by pipe_lambda, whose type is intersection_type (line 829) — NOT the top-level union_type — so a bare `|` after the param type is the closure terminator, not a type-union op. A union-typed closure param must then be parenthesized: `|x: (int | str)|` (grouped_type / paren type already exists in the grammar — confirm and reuse). Keep the shared function_param + type_annotation UNCHANGED (real fn signatures are `(...)`-delimited so their union types are unambiguous and must keep top-level union support). Update the parser: parse_pipe_lambda (parser/expressions/functions.rs:37) — if you add a new closure-param rule name, teach parse_pipe_lambda / parse_function_param (parser/functions.rs:67) to handle it (reuse the existing FunctionParameter AST shape if compatible). The default `||`/`|x|`/`|x,y|` untyped forms + the braced-body form `|x| { ... }` MUST keep working.',
  '',
  'CONSTRAINTS: NO forbidden patterns. Do NOT weaken top-level union_type. Preserve WF-0C backtracking behavior (the exponential-parse regression guard). ' + DX + ' just check-no-dynamic EXIT 0; ' + DX + ' just check-clean EXIT 0. If build-treesitter / grammar codegen is needed, run it.',
  '',
  'REGRESSION-TEST REQUIREMENT (user rule): tests that `|x: int| x + 1` parses AND runs (e.g. `[1,2,3].map(|x: int| x + 1)` or a `let f = |x: int| x + 1; f(4)`), that a parenthesized-union closure param `|x: (int | str)| ...` parses, that untyped `|x|`/`||`/`|x,y|` + braced-body still parse, and that top-level union types (`type Mixed = A | B`) + single-line bitwise-or (`let x = a | b`) still parse. No new #[ignore].',
  '',
  'STRUCTURED-OUTPUT: ONE clean JSON object, 1-4 plain sentences per field, NO XML/code blocks in fields.',
].join('\n')

phase('Fix')
const FIX_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['status', 'grammar_change', 'parser_change', 'parens_union_form', 'evidence'],
  properties: {
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    grammar_change: { type: 'string', description: 'the closure-scoped param/type rules added to shape.pest, brief' },
    parser_change: { type: 'string', description: 'the parse_pipe_lambda / parse_function_param tweak, brief' },
    parens_union_form: { type: 'string', description: 'how a union-typed closure param is now written (parens)' },
    evidence: { type: 'string', description: '|x: int| x+1 parses+runs; untyped + braced still work; check-no-dynamic EXIT 0' },
  },
}
const fix = await agent(CTX + '\n\nPHASE 1 — FIX Bug #2 (closure-scoped param type). ' + DX + ' just check-no-dynamic EXIT 0. Commit (git add -A && git commit --no-verify -m "Grammar: fix unbraced typed-param closure |x: int| x+1 (closure-scoped param type, no top-level union swallow) — tail #4 Bug 2").',
  { label: 'fix', phase: 'Fix', effort: 'high', schema: FIX_SCHEMA })

phase('Verify')
const V_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['verdict', 'typed_closure_works', 'no_grammar_regression', 'backtracking_ok', 'evidence'],
  properties: {
    verdict: { type: 'string', enum: ['CONFIRMED', 'REFUTED', 'PARTIAL'] },
    typed_closure_works: { type: 'boolean', description: 'from YOUR OWN run: |x: int| x+1 parses AND runs; parens-union closure param parses' },
    no_grammar_regression: { type: 'boolean', description: 'untyped/braced closures + top-level union types + single-line bitwise-or still parse; full shape-ast tests green' },
    backtracking_ok: { type: 'boolean', description: 'WF-0C backtracking-timeout tests still green (no exponential-parse regression)' },
    evidence: { type: 'string', description: 'your own from-scratch parse+run + the regression spread; concise' },
  },
}
const verify = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\n\nYou are an INDEPENDENT adversarial reviewer, FRESH context. From scratch: (1) does `|x: int| x + 1` parse AND execute (run a program using it)? does `|x: (int | str)| ...` parse? (2) REGRESSION: untyped `|x|`/`||`/`|x,y|` + braced `|x| {..}` closures, top-level union types (`type Mixed = A | B`), single-line bitwise-or (`let x = a | b`) — all still parse? Full `cargo test -p shape-ast` green? (3) WF-0C backtracking-timeout tests (parser/tests/backtracking.rs) still green? Any parse/run failure, any grammar regression, or any backtracking regression = REFUTED.',
  { label: 'independent-verify', phase: 'Verify', effort: 'high', schema: V_SCHEMA })

phase('Finish')
const F_SCHEMA = {
  type: 'object', additionalProperties: false,
  required: ['gates', 'tests_added', 'bug1_note', 'merge_ready'],
  properties: {
    gates: { type: 'string', description: 'check-clean + check-no-dynamic + shape-ast tests, brief' },
    tests_added: { type: 'string', description: 'the typed-closure + parens-union + no-regression tests' },
    bug1_note: { type: 'string', description: 'confirm Bug #1 (tail-closure ASI) left deferred + the ; workaround' },
    merge_ready: { type: 'boolean' },
  },
}
const finish = await agent(CTX + '\n\nFIX: ' + JSON.stringify(fix) + '\nVERDICT: ' + JSON.stringify(verify) + '\n\nFINISH (only if CONFIRMED; else merge_ready:false + what remains). Ensure the regression tests are committed; note Bug #1 deferred (ASI restructure, `;` workaround); no new #[ignore]. Run ' + DX + ' just check-clean, ' + DX + ' just check-no-dynamic, ' + DX + ' cargo test -p shape-ast. Commit any added tests (git commit --no-verify -m "Closure typed-param grammar finalize: regression tests + Bug #1 deferred note").',
  { label: 'finish', phase: 'Finish', effort: 'medium', schema: F_SCHEMA })

return { fix, verify, finish }
