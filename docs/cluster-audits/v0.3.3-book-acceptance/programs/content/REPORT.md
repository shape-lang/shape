# Book-Acceptance Report — slice: content

Book chapter (PRIMARY source):
`/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/fundamentals/content.mdx`

Binary: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape`
Modes: `--mode vm`, `--mode jit`. Harness: F' release-binary, absolute path.

## Headline

The ENTIRE Content chapter is BOOK-WRONG relative to the shipped binary. Every
named surface the chapter teaches is absent. A real user following this chapter
cannot write a single working line of it. CLAUDE.md corroborates the root cause:
"The legacy `c\"...\"` syntax was retired in W18.3."

## Per-program result

### small.shape (56 LOC) — FAIL (BOOK-WRONG), first-run truth
- VM  ec=1: `error[E0101]: Undefined variable: 'c'` at `let greeting = c"Name: {name}"`
- JIT ec=1: `Bytecode compilation failed: Semantic error: Undefined variable: 'c'`
- Fails at the very first book idiom (§Content Strings). Not author error: the
  syntax is taken verbatim from the chapter's first runnable example
  (`print(c"Name: {name}")`).

### large.shape (377 LOC) — FAIL (BOOK-WRONG), first-run truth
- VM  ec=1: `error[E0001]: unexpected '}', expected something else` at
  `let d = c#"cd #{path}"` (§Content Strings, c#"..." form).
- JIT ec=1: identical parse error, same location.
- The parser does not recognize the `c"..."` / `c$"..."` / `c#"..."` literal
  family at all, so the file never reaches semantic analysis of the builder API.

vm_jit_byte_identical: true for BOTH programs (identical parse/compile error,
identical location, under both modes).

## Probe matrix (what actually exists)

Isolated one-liners, `--mode vm`:

| Book construct (chapter §)                  | Result on shipped binary |
|---------------------------------------------|--------------------------|
| `c"Name: {name}"` (§Content Strings)        | PARSE ERROR `unexpected '}'` / `Undefined variable 'c'` |
| `c$"..."`, `c#"..."` (§Content Strings)     | PARSE ERROR (same) |
| `Content.text("x")` (§Styled Text)          | SEMANTIC: `Method 'text' not found on type 'Content'` |
| `Content.table(rows)` (§Tables)             | depends on `Border` → `Undefined variable: 'Border'` |
| `Content.chart("line")` (§Charts)           | SEMANTIC: `Content ... cannot have fields` |
| `Content.fragment([])` (§Composition)       | SEMANTIC: `Method 'fragment' not found on type 'Content'` |
| `Color.red` / `.rgb` (§Colors)              | `Undefined variable: 'Color'` |
| `Border.rounded` (§Border Styles)           | `Undefined variable: 'Border'` |
| `ChartType.line` (§Chart Types)             | `Undefined variable: 'ChartType'` |
| auto-table `print([Row{..}])` (§Auto-Table) | NO table — plain debug `[{...}]` print, no box-drawing |
| `f"Name: {name}"` (baseline, §Strings)      | WORKS — `Name: Alice` |
| `trait Content { fn render(self)->X; }`     | parses, but is a *user* trait, not the built-in |

### Secondary anomaly (auto-table probe)
`print([ Row { name: "test", value: 142.5 } ])` printed
`[{timestamp: "test", fields: 142.5}]` — note the field names `name`/`value`
are displayed as `timestamp`/`fields`. Besides the missing auto-table
(headline), the struct debug-printer appears to emit wrong/placeholder field
names. Logged as a sub-finding; not the slice's primary defect.

## Expected-value rationale (derived from BOOK SEMANTICS, pre-run)

All expected values were written before the first run, from the chapter text:
- `score_plain(3.2) == "+3.2"` — §Content Trait Score example (signed, fixed(1))
  + §Inline Styling `fixed(n)` precision; §Adapter Matrix "Plain = No formatting"
  so color is dropped, text survives.
- `c"Name: {name}".render(Plain) == "Name: Alice"` — §Content Strings basic
  interpolation + Plain no-formatting.
- `c$"${user}"`, `c#"#{path}"` — §Content Strings interpolation-mode table.
- `Content.text("Error")...render(Plain) == "Error"` — §Styled Text + Plain.
- Table headers = field names; Plain table = "ASCII borders" (§Adapter Matrix);
  `max_rows(1)` truncates to first row (§Table Methods).
- Charts: `.title(t)` round-trips into Plain "Text description" (§Adapter Matrix);
  `ChartType.bar` string form == namespace form (§Chart Types).
- Fragment renders parts sequentially header→table→chart (§Composition).

NONE of these could be exercised — the programs fail before any assertion runs.
No expected value was ever back-filled from observed output.

## Classifications

- small.shape: BOOK-WRONG (followed §Content Strings verbatim; does not parse).
- large.shape: BOOK-WRONG (same root: c-string family unrecognized).

These are NOT FN-regressions in the strict-flip sense — the feature does not
exist in the binary at all (retired per CLAUDE.md W18.3), so there is no working
behavior to have regressed. The defect is purely book-vs-implementation drift.

## book_wrong (chapter claims the language does not do)

1. `c"..."` content strings — §Content Strings. Parser rejects the `c` prefix
   entirely (`unexpected '}'`). Retired W18.3 per CLAUDE.md; chapter still
   teaches it as the foundational primitive.
2. `c$"..."` and `c#"..."` alternate interpolation modes — §Content Strings.
3. Inline styling hints `{x: fg(green), fixed(2)}` — §Inline Styling. Unreachable
   (host literal doesn't parse).
4. `Content.text(...)` builder + `.fg/.bg/.bold/.italic/.underline/.dim` chain —
   §Styled Text. `Method 'text' not found on type 'Content'`.
5. `Content.table(rows)` + `.border(..)` + `.max_rows(n)` — §Tables. Absent.
6. `Content.chart(...)` + `.add/.title/.x_label/.y_label/.width/.height` —
   §Charts. `Content ... cannot have fields`.
7. `Content.fragment([...])` — §Composition. `Method 'fragment' not found`.
8. `Color` namespace (`Color.red`, `Color.rgb(r,g,b)`) — §Colors. Undefined.
9. `Border` namespace (`Border.rounded` … `Border.none`) — §Border Styles. Undefined.
10. `ChartType` namespace (`ChartType.line` …) — §Chart Types. Undefined.
11. Auto-table for `Vec<T: struct>` via `print(...)` — §Auto-Table for
    Collections. No table rendered; plain debug output instead.
12. `ContentFor<Adapter>` + adapters `Terminal/Html/Markdown/Json/Plain` —
    §ContentFor / §Available Adapters / §Adapter Behavior Matrix. Unreachable;
    no `Plain`/`Terminal`/etc. adapter symbols exist.
13. Wire `WireValue::Content(ContentNode::Chart{..})` — §Wire Serialization.
    No way to construct a ContentNode to serialize.

## book_gaps (chapter silent on something needed)

The chapter is not so much "silent" as wholly non-functional, so the usual
gap-vs-wrong split collapses into book_wrong. Two true gaps:
1. The chapter never states a render entry point on a `ContentNode` (e.g. how
   `print(node)` chooses an adapter, or whether `node.render(Adapter)` exists).
   I had to infer `node.render(Plain)` from the Adapter Matrix; the API for
   turning a ContentNode into a string for a chosen adapter is undocumented.
2. No `import`/`use` line is shown for `Content`, `Color`, `Border`,
   `ChartType`, or the adapters. The chapter implies they are ambient globals;
   the binary has no such globals, and the chapter gives no module path to try.

## Recommendation

This chapter must be rewritten to match the post-W18.3 reality (c-string syntax
retired) or the Content system must be re-shipped. As written, fundamentals/
content.mdx is 100% non-runnable on the v0.3.3 strict-flip binary.
