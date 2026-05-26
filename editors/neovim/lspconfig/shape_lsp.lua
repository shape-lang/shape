local util = require 'lspconfig.util'

return {
  default_config = {
    cmd = { 'shape-lsp' },
    filetypes = { 'shape' },
    root_dir = util.root_pattern('shape.toml', '.git'),
    single_file_support = true,
    init_options = {
      shape = {
        inlayHints = {
          enable = true,
          -- Shape-unique opt-in (default OFF per 2026-05-26 standing pattern).
          -- Set to true to surface BindingStorageClass approximations.
          bindingStorageClass = { enable = false },
        },
      },
    },
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

Inlay-hint settings are tuned via the `shape.inlayHints.*` tree under
`init_options` (sent at initialize) or via `workspace/didChangeConfiguration`
for live updates. The Shape-unique `bindingStorageClass` hint is opt-in
default-OFF; flip `bindingStorageClass.enable = true` to see LSP-side
approximations of the binding-storage class per ADR-006.
]],
  },
}
