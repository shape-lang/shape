# CLI Integration Tests

Integration tests for `shape-cli`, covering both language evaluation and CLI binary execution.

## Structure

```
tests/
├── common/
│   └── mod.rs                 # Eval helpers (init_runtime, eval, eval_to_number, etc.)
├── language/
│   ├── mod.rs                 # pub mod for each language module
│   ├── arithmetic.rs          # Arithmetic operations
│   ├── comparisons.rs         # Comparison operators
│   ├── variables.rs           # Variable declarations and scoping
│   ├── control_flow.rs        # If/else, while, for loops
│   ├── functions.rs           # Function definitions and calls
│   ├── strings.rs             # String operations
│   └── error_handling.rs      # Error cases
├── collections/
│   ├── mod.rs
│   ├── arrays.rs              # Array operations
│   └── objects.rs             # Object operations
├── type_system/
│   ├── mod.rs
│   └── vm_parity.rs           # VM type system parity checks
├── execution/
│   ├── mod.rs
│   └── execution_modes.rs     # Different execution modes
├── stdlib/
│   ├── mod.rs
│   ├── simulation.rs          # Stdlib simulation tests
│   ├── meta_system.rs         # Meta system tests
│   └── formatting.rs          # Print/format tests
├── cli/
│   ├── mod.rs
│   └── script_execution.rs    # Binary execution tests (assert_cmd)
├── language_tests.rs          # mod language; mod common;
├── collection_tests.rs        # mod collections; mod common;
├── type_system_tests.rs       # mod type_system; mod common;
├── execution_tests.rs         # mod execution; mod common;
├── stdlib_tests.rs            # mod stdlib; mod common;
├── cli_tests.rs               # mod cli;
└── smoke_test.rs              # Basic smoke test
```

Each top-level `*_tests.rs` file is a test crate entry point that declares `mod common;` and its category module.

## Running Tests

```bash
# All CLI tests
cargo test -p shape-cli

# A single test category
cargo test -p shape-cli --test language_tests
cargo test -p shape-cli --test collection_tests
cargo test -p shape-cli --test cli_tests

# A single test by name
cargo test -p shape-cli --test language_tests test_addition

# The smoke test
cargo test -p shape-cli --test smoke_test
```

## Writing a Language Test

Language tests evaluate Shape code through the engine and assert on the result. Every test file in a category starts by importing the common helpers through its parent module.

### Eval Helpers

All helpers are in `common/mod.rs`:

| Helper | Description |
|--------|-------------|
| `init_runtime()` | Initialize the shared runtime (call once if needed) |
| `eval(code)` | Evaluate Shape code, return `Result<serde_json::Value, String>` |
| `eval_with_stdlib(code)` | Evaluate with stdlib loaded, return `Result<serde_json::Value, String>` |
| `eval_to_number(code)` | Evaluate and extract result as `f64` (panics if not a number) |
| `eval_to_bool(code)` | Evaluate and extract result as `bool` (panics if not a bool) |
| `eval_to_string(code)` | Evaluate and extract result as `String` (panics if not a string) |
| `stdlib_eval_to_number(code)` | Like `eval_to_number` but with stdlib loaded |
| `stdlib_eval_to_bool(code)` | Like `eval_to_bool` but with stdlib loaded |

The `eval_to_*` helpers handle both direct values (`Value::Number`) and wrapped objects (`{"Integer": 42}`, `{"Number": 3.14}`, `{"Bool": true}`, `{"String": "hello"}`).

### Example: Language Test

```rust
// In language/arithmetic.rs
use crate::common;

#[test]
fn test_addition() {
    assert_eq!(common::eval_to_number("1 + 2"), 3.0);
}

#[test]
fn test_float_multiplication() {
    let result = common::eval_to_number("2.5 * 4.0");
    assert!((result - 10.0).abs() < f64::EPSILON);
}
```

### Example: Stdlib Test

```rust
// In stdlib/simulation.rs
use crate::common;

#[test]
fn test_stdlib_abs() {
    assert_eq!(common::stdlib_eval_to_number("abs(-5)"), 5.0);
}
```

### Example: Error Test

```rust
use crate::common;

#[test]
fn test_undefined_variable_error() {
    let result = common::eval("undefined_var");
    assert!(result.is_err());
}
```

## Writing a CLI Binary Test

Binary tests use `assert_cmd` and `predicates` to test the `shape` binary end-to-end. These live in `cli/script_execution.rs`.

### Dependencies

- `assert_cmd` - Command assertions
- `predicates` - Output matchers
- `tempfile` - Temporary script files

### Helper

```rust
use assert_cmd::Command;

fn shape_cmd() -> Command {
    Command::cargo_bin("shape").unwrap()
}
```

### Example: Script Execution

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use std::io::Write;
use tempfile::tempdir;

fn shape_cmd() -> Command {
    Command::cargo_bin("shape").unwrap()
}

#[test]
fn test_script_arithmetic() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("test.shape");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "let x = 1 + 2").unwrap();
        writeln!(f, "print(x)").unwrap();
    }

    shape_cmd()
        .args(["script", script.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("3"));
}
```

### Example: Error Case

```rust
#[test]
fn test_script_nonexistent_file() {
    shape_cmd()
        .args(["script", "/tmp/nonexistent_file.shape"])
        .assert()
        .failure();
}
```

## Adding a New Test Category

1. Create `tests/<category>/mod.rs` declaring submodules
2. Create test files in `tests/<category>/`
3. Create `tests/<category>_tests.rs` as the entry point:
   ```rust
   mod common;
   mod <category>;
   ```
4. Run `cargo test -p shape-cli --test <category>_tests`

## Tips

- **Use `eval_to_number`/`eval_to_bool`/`eval_to_string`** for simple assertions instead of matching on raw `serde_json::Value`.
- **Use `eval_with_stdlib` / `stdlib_eval_to_*`** when testing stdlib functions (abs, floor, etc.).
- **Use `eval` directly** when you need to inspect the raw JSON value or test error cases.
- **For float comparisons**, use `(result - expected).abs() < f64::EPSILON` instead of `assert_eq!`.
- **Binary tests need `tempfile`** for script files -- always use `tempdir()` for cleanup.
- **Script execution uses `["script", path]`** as the CLI args format.
