# Shape Language — VS Code extension

Editor integration for the [Shape](https://github.com/shape-lang/shape)
statically-typed programming language. Provides:

- Syntax highlighting (TextMate grammar)
- Language Server Protocol client wired to `shape-lsp`
- File-icon + language-configuration (comments, brackets, autoclosing, indent
  rules)

## What you get from the LSP

The extension advertises the full set of capabilities exposed by `shape-lsp`
([source](../../tools/shape-lsp/src/server.rs)). The capability matrix is the
authoritative spec; this README is a quick subset:

| Feature | Method |
|---|---|
| Hover | `textDocument/hover` |
| Completions (lazy resolve) | `textDocument/completion` |
| Signature help | `textDocument/signatureHelp` |
| Go-to-definition / declaration / type-definition / implementation | `textDocument/definition`, `declaration`, `typeDefinition`, `implementation` |
| Find references / highlight | `textDocument/references`, `documentHighlight` |
| Rename (with prepare) | `textDocument/prepareRename`, `rename` |
| Document & workspace symbols | `textDocument/documentSymbol`, `workspace/symbol` |
| Diagnostics (push + pull) | `textDocument/publishDiagnostics`, `textDocument/diagnostic`, `workspace/diagnostic` |
| Code actions, code lens | `textDocument/codeAction`, `codeLens` |
| Inlay hints (opt-in categories below) | `textDocument/inlayHint` |
| Semantic tokens (range + delta) | `textDocument/semanticTokens/*` |
| Folding, document links, call hierarchy | `textDocument/foldingRange`, `documentLink`, `prepareCallHierarchy` |
| Formatting (document, range, on-type) | `textDocument/formatting`, `rangeFormatting`, `onTypeFormatting` |
| Workspace `willRename` for `.shape` files | `workspace/willRenameFiles` |

## Installing the language server

The extension launches `shape-lsp` from your `PATH`. On first activation, if
`shape-lsp` is missing, the extension offers to install it via `cargo install
shape-lsp`. You can also install it yourself:

```sh
cargo install shape-lsp
```

When a newer version is available on crates.io, the extension surfaces a
notification and offers an in-place `cargo install` update.

## Settings

All settings live under the `shape.*` namespace and are forwarded to the
language server via `initializationOptions` (on first contact) and
`workspace/didChangeConfiguration` (live updates). The inlay-hint master is
ON by default; Shape-unique opt-in categories are OFF by default per the
standing pattern (2026-05-26 binding).

| Setting | Default | Description |
|---|---|---|
| `shape.inlayHints.enable` | `true` | Master toggle. False suppresses every category. |
| `shape.inlayHints.typeHints` | `true` | Inferred type hints on `let` bindings. |
| `shape.inlayHints.parameterHints` | `true` | Parameter-name hints at call sites. |
| `shape.inlayHints.variableTypeHints` | `true` | Inferred variable types where elided. |
| `shape.inlayHints.returnTypeHints` | `true` | Inferred return-type hints on `fn` declarations. |
| `shape.inlayHints.chainHints` | `true` | Intermediate-type hints on method-chain expressions. |
| `shape.inlayHints.bindingStorageClass.enable` | **`false` (opt-in)** | Shape-unique: LSP-side approximation of `BindingStorageClass` (`Direct` / `UniqueHeap` / `SharedCow` / `SharedAtomic` / `SharedAtomicMut` per ADR-006). Always rendered with `[… approx]` because the authoritative classifier lives in the bytecode compiler. |

## Building from source

```sh
cd editors/vscode
npm install
npm run compile        # tsc -> out/extension.js
npm run package        # vsce package -> shape-lang-<version>.vsix
```

Then `Extensions: Install from VSIX…` in VS Code, pointing at the produced
`.vsix`.

## License

MIT.
