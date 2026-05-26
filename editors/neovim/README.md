# Neovim / Vim Integration for Shape

Configuration snippets for using the [Shape](https://github.com/shape-lang/shape)
statically-typed programming language with Neovim (Lua) and Vim (legacy VimL).

## Quick start (Neovim 0.10+, native `vim.lsp.start`)

1. Install the language server:

   ```sh
   cargo install shape-lsp
   ```

   Confirm it's on your `$PATH` — `shape-lsp --version` should print
   `shape-lsp <version>`.

2. Drop the filetype-detection snippet into your config so `*.shape` files
   pick up the `shape` filetype:

   ```sh
   cp editors/neovim/ftdetect/shape.lua ~/.config/nvim/ftdetect/
   ```

3. Start the server when a `*.shape` buffer opens. Minimal `init.lua`:

   ```lua
   vim.api.nvim_create_autocmd('FileType', {
     pattern = 'shape',
     callback = function(args)
       vim.lsp.start({
         name = 'shape-lsp',
         cmd = { 'shape-lsp' },
         root_dir = vim.fs.root(args.buf, { 'shape.toml', '.git' }),
         init_options = {
           shape = {
             inlayHints = {
               enable = true,
               -- Shape-unique opt-in (default OFF). See "Settings" below.
               bindingStorageClass = { enable = false },
             },
           },
         },
       })
     end,
   })
   ```

4. (Optional) Turn on inlay hints once the client attaches:

   ```lua
   vim.api.nvim_create_autocmd('LspAttach', {
     callback = function(args)
       if vim.lsp.inlay_hint then
         vim.lsp.inlay_hint.enable(true, { bufnr = args.buf })
       end
     end,
   })
   ```

## Quick start (via `nvim-lspconfig`)

If you already use [`neovim/nvim-lspconfig`](https://github.com/neovim/nvim-lspconfig)
and the `shape_lsp` server lands upstream (PR pending; the snippet in
[`lspconfig/shape_lsp.lua`](lspconfig/shape_lsp.lua) is the source of truth):

```lua
require('lspconfig').shape_lsp.setup({
  init_options = {
    shape = {
      inlayHints = {
        enable = true,
        bindingStorageClass = { enable = false },  -- opt-in default OFF
      },
    },
  },
})
```

## Settings

The Shape LSP honors a `shape.*` settings tree, sent via the
`initializationOptions` slot or — for live updates — via
`workspace/didChangeConfiguration`. Per the standing pattern (2026-05-26
binding), the inlay-hint master is ON by default; Shape-unique opt-in
categories are OFF by default.

| Key | Default | Description |
|---|---|---|
| `shape.inlayHints.enable` | `true` | Master toggle. |
| `shape.inlayHints.typeHints` | `true` | Inferred type hints on `let` bindings. |
| `shape.inlayHints.parameterHints` | `true` | Parameter-name hints at call sites. |
| `shape.inlayHints.variableTypeHints` | `true` | Inferred variable types where elided. |
| `shape.inlayHints.returnTypeHints` | `true` | Inferred return-type hints on `fn` declarations. |
| `shape.inlayHints.chainHints` | `true` | Intermediate-type hints on method-chain expressions. |
| `shape.inlayHints.bindingStorageClass.enable` | **`false` (opt-in)** | Shape-unique: LSP-side approximation of `BindingStorageClass` (`Direct` / `UniqueHeap` / `SharedCow` / `SharedAtomic` / `SharedAtomicMut` per ADR-006). Always rendered with `[… approx]`. |

The full LSP capability matrix is the authoritative spec; see
[`tools/shape-lsp/src/server.rs`](../../tools/shape-lsp/src/server.rs) for the
complete `ServerCapabilities` block (hover, completions, signatureHelp,
definition / declaration / typeDefinition / implementation, references,
documentHighlight, rename with prepare, documentSymbol, workspaceSymbol,
semanticTokens with delta+range, inlayHint with resolve, codeAction, codeLens,
foldingRange, documentLink, callHierarchy, pull+push diagnostics, formatting,
workspace willRename).

## Files in this directory

### [`lspconfig/shape_lsp.lua`](lspconfig/shape_lsp.lua)

Server configuration for nvim-lspconfig. Submit as a PR to
[neovim/nvim-lspconfig](https://github.com/neovim/nvim-lspconfig) under
`lua/lspconfig/configs/`.

### [`mason/package.yaml`](mason/package.yaml)

Mason registry entry for `shape-lsp`. Submit as a PR to
[mason-org/mason-registry](https://github.com/mason-org/mason-registry) under
`packages/shape-lsp/`.

### [`treesitter/shape.lua`](treesitter/shape.lua)

Reference snippet for the tree-sitter parser registration. Use as a guide when
submitting a PR to
[nvim-treesitter/nvim-treesitter](https://github.com/nvim-treesitter/nvim-treesitter).

### [`ftdetect/shape.lua`](ftdetect/shape.lua) and [`ftdetect/shape.vim`](ftdetect/shape.vim)

Filetype detection for Neovim (Lua) and Vim (legacy VimL). Drop directly into
`~/.config/nvim/ftdetect/` or submit upstream to
[vim/vim](https://github.com/vim/vim).
