-- Add to lua/nvim-treesitter/parsers.lua in the nvim-treesitter repo
local M = {}

M.shape = {
  install_info = {
    url = 'https://github.com/shape-lang/tree-sitter-shape',
    files = { 'src/parser.c' },
  },
  filetype = 'shape',
  maintainers = { '@damesberger' },
}

return M
