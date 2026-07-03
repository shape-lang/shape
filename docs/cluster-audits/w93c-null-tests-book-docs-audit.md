# W93C Null Tests/Book Docs Audit

Date: 2026-07-03

Rule applied: Shape source uses `None`, not `null`. External JSON, wire,
protocol, C pointer, JavaScript, and historical internal sentinel wording can
still use null where it describes that external or implementation boundary.

## Touched Surfaces

| Surface | Classification | Action |
|---|---|---|
| `tools/shape-test/tests/{error_handling,operators,literals,type_inference,type_aliases_unions,variables_bindings,pattern_matching,strings_formatting,traits}/**` | Stale language fixture/test wording | Renamed source-level test prose and Rust test ids from null terminology to `None`/coalesce terminology. Shape snippets already used `None`. |
| `tools/shape-test/src/shape_test.rs` | Shape `None` assertion with wire compatibility | Changed public assertion wording to Shape `None`; kept JSON null / string `Null` compatibility in the matcher. |
| `tools/shape-test/tests/error_handling/diagnostics.rs` | Missing negative parser expectation | Added `parse_err_null_literal_rejected`; parser already rejects source `null`. |
| `tools/shape-test/tests/book_policy.rs` | Shape-source policy gate | Kept the `null` token ban for Shape snippets and clarified the error to use `None`, `Option`, or `Result`. |
| `docs/cluster-audits/v0.3.3-book-acceptance/programs/comptime/{pn,probe7b}.shape` | Stale Shape source fixture | Rewrote source comparisons from `null` to `None`; updated probe output labels to `is-none` / `not-none`. |
| `docs/cluster-audits/v0.3.3-book-acceptance/programs/annotations/large.shape` | Source-null parser contract comment | Reworded comment to say source `null` is rejected and the fixture uses `??`. |
| `docs/codebase-index/01-compilation.md` | Stale live source example plus internal type name | Rewrote narrowing example to `x != None`; clarified internal `TypeAnnotation::Null` rather than presenting `null` as a source primitive. |
| `docs/rfcs/008-realtime-llm-channel.md` | Stale Shape source embedded in JSON strings | Rewrote source strings to `user != None` and `if x == None`. Protocol JSON `snapshot_token: null` remains valid. |
| `docs/vision/distributed-comptime-async-vision.md` | Stale Shape source example | Rewrote `entry != null` to `entry != None`. |
| `docs/design/v0.3.3-reference-serialization/round2/adversarial/REVIEW-cycle-leak-and-drop-accounting.md` | Stale Shape source example | Rewrote the peer field as `Option<&Node>` and the store payload as `None`. |

## Classified And Not Rewritten

| Surface | Classification | Disposition |
|---|---|---|
| `tools/shape-test/tests/stdlib_json/parse.rs` `json::parse("null")` | JSON interop | Valid. |
| LSP tests mentioning hover/signature/navigation return `null` | JSON-RPC/protocol result | Valid. |
| `NativeKind::Null`, `WireValue::Null`, `Constant::Null`, hashmap `value kind Null` expectations | Internal runtime/wire sentinel wording | Valid outside Shape source syntax. |
| `docs/rfcs/008-realtime-llm-channel.md` `snapshot_token: null` | JSON protocol | Valid. |
| Native C / pointer docs and ABI/runtime docs using null pointer/null terminator/null bitmap | External or storage representation | Valid. |
| Historical cluster audits and defect logs describing previous `null` output, sentinels, or failures | Historical evidence | Left intact unless the file was an active `.shape` fixture. |
| `/home/dev/dev/shape-lang/shape-web/book/book-site` | Separate dirty `shape-web` repo | Audited only. Stale book-site surfaces found: `src/grammars/shape.tmLanguage.json` still highlights `null` as a Shape language constant, and `src/content/docs/fundamentals/operators.mdx` says `None` / `null`. Interop occurrences in JSON/YAML/native-C/TypeScript docs and JS/Svelte host code are valid. |

## Supervisor Follow-Up

Patch the separate `shape-web` worktree after coordinating with its current
dirty state:

- Remove the `constant.language.null.shape` grammar rule for `null`.
- Change `operators.mdx` coalesce prose from `None` / `null` to `None`.
- Regenerate derived public book artifacts if the book pipeline owns them.
