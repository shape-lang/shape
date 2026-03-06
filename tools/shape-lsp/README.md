# Shape Language Server

A fully-featured Language Server Protocol (LSP) implementation for Shape, providing rich IDE support including diagnostics, completion, hover information, go-to-definition, and more.

## Features

✅ **Real-time Diagnostics** - Parse, semantic, and type errors as you type
✅ **Intelligent Completions** - Context-aware autocomplete with 80+ built-ins + user-defined symbols
✅ **Hover Information** - Documentation, signatures, and type info on hover
✅ **Signature Help** - Parameter hints while typing function calls
✅ **Go-to-Definition** - Jump to symbol definitions (Ctrl+Click)
✅ **Find References** - See all usages of a symbol
✅ **Document Symbols** - Outline view of functions, patterns, variables
✅ **Workspace Symbols** - Search symbols across entire workspace

## Architecture

**Single Source of Truth**: All language information comes from `shape-core/metadata.rs` - no hardcoded language features in the LSP. Adding a new built-in to Shape automatically makes it available in the LSP.

## Installation

### Building from Source

```bash
# Build the LSP server
cargo build --release -p shape-lsp

# The binary will be at:
# target/release/shape-lsp
```

### Editor Integration

#### VSCode

Create `.vscode/settings.json` in your workspace:

```json
{
  "shape.languageServer": {
    "enabled": true,
    "path": "/path/to/target/release/shape-lsp"
  }
}
```

Or use the generic LSP client extension:

1. Install "Generic LSP Client" extension
2. Add to settings.json:

```json
{
  "genericLanguageServer.languageServerConfigs": {
    "shape": {
      "command": "/path/to/target/release/shape-lsp",
      "args": [],
      "filetypes": ["cql"],
      "initializationOptions": {}
    }
  }
}
```

#### Neovim (with nvim-lspconfig)

Add to your Neovim config:

```lua
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')

-- Define Shape LSP if not already defined
if not configs.shape then
  configs.shape = {
    default_config = {
      cmd = {'/path/to/target/release/shape-lsp'},
      filetypes = {'cql'},
      root_dir = lspconfig.util.root_pattern('.git', 'Cargo.toml'),
      settings = {},
    },
  }
end

-- Enable Shape LSP
lspconfig.shape.setup{}
```

#### Emacs (with lsp-mode)

Add to your Emacs config:

```elisp
(require 'lsp-mode)

(add-to-list 'lsp-language-id-configuration '(shape-mode . "shape"))

(lsp-register-client
 (make-lsp-client
  :new-connection (lsp-stdio-connection "/path/to/target/release/shape-lsp")
  :major-modes '(shape-mode)
  :server-id 'shape-lsp))
```

## Usage

Once installed, the LSP provides:

### Diagnostics
Errors appear as red squiggles in real-time:
- Parse errors (syntax mistakes)
- Semantic errors (undefined variables)
- Type errors (type mismatches)

### Completions
Type to get intelligent suggestions:
- User-defined variables, functions, patterns
- Built-in functions (sma, ema, rsi, etc.)
- Keywords (let, const, if, pattern, backtest, etc.)
- Snippets (pattern-def, function-def, strategy-template, etc.)
- Property completion: `candle[0].` → open, close, high, low, volume

### Hover
Hover over any symbol to see:
- Function signatures with parameter details
- Type information
- Documentation and examples
- Variable/constant types

### Signature Help
Type `sma(` to see:
- Function signature
- Parameter names and types
- Documentation for each parameter
- Current parameter highlighted

### Navigation
- **Go-to-Definition**: Ctrl+Click or F12 on any symbol
- **Find References**: Shift+F12 to see all usages
- **Document Symbols**: Ctrl+Shift+O for outline view
- **Workspace Symbols**: Ctrl+T to search across files

## Development

### Running Tests

```bash
cargo test -p shape-lsp
```

### Running the LSP Server

```bash
# Stdio mode (for editor integration)
cargo run -p shape-lsp --bin shape-lsp

# With debug logging
RUST_LOG=shape_lsp=debug cargo run -p shape-lsp --bin shape-lsp
```

## Architecture

```
shape-lsp/
├── src/
│   ├── main.rs              # Binary entrypoint (stdio server)
│   ├── lib.rs               # Library exports
│   ├── server.rs            # Main LSP server implementation
│   ├── document.rs          # Document manager (Rope-based storage)
│   ├── diagnostics.rs       # Error → LSP diagnostics converter
│   ├── completion.rs        # Intelligent completion provider
│   ├── hover.rs             # Hover information provider
│   ├── signature_help.rs    # Function signature hints
│   ├── definition.rs        # Go-to-definition & find references
│   ├── document_symbols.rs  # Outline view & symbol search
│   ├── symbols.rs           # Symbol extraction from AST
│   └── context.rs           # Context-aware completion filtering
└── Cargo.toml
```

### Key Design Principles

1. **Single Source of Truth**: All language metadata comes from `shape-core`
2. **Performance**: Document caching, efficient text operations with Rope
3. **Robustness**: Graceful degradation when parsing fails
4. **Simplicity**: Minimal code, maximum functionality

## Protocol Support

Implements LSP specification with:
- Document synchronization (incremental)
- Diagnostics publishing
- Completion (with trigger characters: `.`, `(`, ` `)
- Hover
- Signature Help (triggers: `(`, `,`)
- Go-to-Definition
- Find References
- Document Symbols
- Workspace Symbols

## License

MIT OR Apache-2.0

## Contributing

The LSP queries `shape-core/metadata.rs` for all language information. To add new built-in functions or keywords, update the metadata API - the LSP will automatically pick them up.
