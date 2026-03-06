# shape-cli

Command-line interface for Shape.

## Usage

```bash
# Run the REPL
cargo run -p shape-cli --bin shape

# Run a Shape script
cargo run -p shape-cli --bin shape -- <path/to/file.shape>

# Expand comptime-generated code
cargo run -p shape-cli --bin shape -- expand-comptime <path/to/file.shape>

# Expand comptime-generated code (shorthand)
cargo run -p shape-cli --bin shape -- <path/to/file.shape> --expand

# Filter expansion output
cargo run -p shape-cli --bin shape -- expand-comptime <path/to/file.shape> --module duckdb --function connect
```

## Features

- Interactive REPL for Shape queries
- Script execution from files
- Comptime expansion inspection (`expand-comptime`)
