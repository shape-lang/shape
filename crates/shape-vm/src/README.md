# Shape Virtual Machine

The Shape VM is a stack-based bytecode interpreter that provides efficient execution of Shape programs.

## Architecture

### Components

- **`bytecode.rs`** - Instruction set and bytecode format
  - Defines opcodes for all operations
  - Manages constant pool and function table
  - Provides compact instruction encoding

- **`compiler.rs`** - AST to bytecode compiler
  - Traverses AST and emits instructions
  - Manages variable scoping and resolution
  - Performs basic optimizations

- **`executor.rs`** - Virtual machine execution engine
  - Stack-based execution model
  - Call stack management
  - Built-in function implementations

- **`value.rs`** - Runtime value representation
  - Efficient tagged union for all types
  - Reference counting for collections
  - Type conversion utilities

## Instruction Set Overview

### Stack Operations
- `PUSH_CONST`, `PUSH_NULL`, `POP`, `DUP`, `SWAP`

### Arithmetic & Logic
- `ADD`, `SUB`, `MUL`, `DIV`, `MOD`, `NEG`
- `GT`, `LT`, `GTE`, `LTE`, `EQ`, `NEQ`
- `AND`, `OR`, `NOT`

### Control Flow
- `JUMP`, `JUMP_IF_FALSE`, `JUMP_IF_TRUE`
- `CALL`, `RETURN`, `RETURN_VALUE`

### Variables
- `LOAD_LOCAL`, `STORE_LOCAL`
- `LOAD_GLOBAL`, `STORE_GLOBAL`

### Objects & Arrays
- `NEW_ARRAY`, `NEW_OBJECT`
- `GET_PROP`, `SET_PROP`, `LENGTH`

### Domain-Specific
- `LOAD_CANDLE`, `CANDLE_PROP`
- `INDICATOR`, `PATTERN`
- `FUZZY_EQ`, `FUZZY_GT`, `FUZZY_LT`

## Usage

```rust
use shape::vm::{BytecodeCompiler, VirtualMachine, VMConfig};

// Compile AST to bytecode
let compiler = BytecodeCompiler::new();
let bytecode = compiler.compile(&ast)?;

// Execute in VM
let mut vm = VirtualMachine::new(VMConfig::default());
vm.load_program(bytecode);
let result = vm.execute()?;
```

## Performance

The VM provides several performance benefits:
- Faster execution than tree-walking interpreter
- Reduced memory allocation during execution
- Efficient instruction dispatch
- Constant pool deduplication
- Future JIT compilation support

## Future Enhancements

1. **Debugging Support**
   - Breakpoints and stepping
   - Stack inspection
   - Variable watches

2. **Optimizations**
   - Constant folding
   - Dead code elimination
   - Inline caching

3. **Advanced Features**
   - Closures with captured variables
   - Coroutines for async execution
   - JIT compilation for hot paths