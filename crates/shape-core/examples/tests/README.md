# Shape Test Examples

This directory contains test files used for validating Shape language features and parser functionality.

## Test Categories

### Parser Tests
- `test_simple_add.shape` - Basic arithmetic
- `test_expr_*.shape` - Expression parsing
- `test_paren_*.shape` - Parentheses handling

### Variable Tests
- `test_variables.shape` - Variable declaration and usage
- `test_var_reassign.shape` - Variable reassignment
- `test_const_reassign.shape` - Const immutability
- `test_uninit_*.shape` - Uninitialized variable handling

### Scope Tests
- `test_block_scope.shape` - Block scoping rules
- `test_scope_shadowing.shape` - Variable shadowing

### Function Tests
- `test_simple_function.shape` - Basic function definition
- `test_functions_loops.shape` - Functions with loops

### Array/Object Tests
- `test_array_methods.shape` - Array operations
- `test_array_sum.shape` - Array aggregation
- `test_objects.shape` - Object literals

## Running Tests

These files are primarily used by the Shape test suite:

```bash
cargo test parser_tests
```

They serve as:
1. Regression tests for parser changes
2. Examples of valid/invalid syntax
3. Edge case documentation

## Note

These are not meant as learning examples. For tutorials, see `/tutorials/`.