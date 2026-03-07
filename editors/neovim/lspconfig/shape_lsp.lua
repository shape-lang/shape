local util = require 'lspconfig.util'

return {
  default_config = {
    cmd = { 'shape-lsp' },
    filetypes = { 'shape' },
    root_dir = util.root_pattern('shape.toml', '.git'),
    single_file_support = true,
  },
  docs = {
    description = [[
https://github.com/shape-lang/shape

Language server for the Shape programming language.

`shape-lsp` can be installed via `cargo`:
```sh
cargo install shape-lsp
```

Or via Mason:
```
:MasonInstall shape-lsp
```
]],
  },
}
