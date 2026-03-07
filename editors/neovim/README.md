# Neovim / Vim Integration for Shape

Config files for integrating the Shape language with Neovim and Vim ecosystems.

## Files

### `mason/package.yaml`

Mason registry entry for `shape-lsp`. Submit as a PR to
[mason-org/mason-registry](https://github.com/mason-org/mason-registry) under
`packages/shape-lsp/`.

### `lspconfig/shape_lsp.lua`

Server configuration for nvim-lspconfig. Submit as a PR to
[neovim/nvim-lspconfig](https://github.com/neovim/nvim-lspconfig) under
`lua/lspconfig/configs/`.

### `treesitter/shape.lua`

Reference snippet for the tree-sitter parser registration. Use as a guide when
submitting a PR to
[nvim-treesitter/nvim-treesitter](https://github.com/nvim-treesitter/nvim-treesitter).

### `ftdetect/shape.lua` and `ftdetect/shape.vim`

Filetype detection for Neovim (Lua) and Vim (legacy VimL). These can be:

- Dropped directly into your Neovim config (`~/.config/nvim/ftdetect/`)
- Submitted as a PR to [vim/vim](https://github.com/vim/vim)
