# Shape Virtual Machine Architecture

## Overview

The Shape VM is a stack-based bytecode virtual machine designed for efficient execution of Shape programs. It provides:
- Fast execution through bytecode compilation
- Support for all Shape language features
- Domain-specific optimizations for financial computations
- Debugging and profiling capabilities

## Architecture

### Components

1. **Bytecode Compiler** (`compiler.rs`)
   - Translates AST to bytecode instructions
   - Performs basic optimizations
   - Manages constant pool and string interning

2. **VM Executor** (`executor.rs`)
   - Stack-based execution engine
   - Call stack management
   - Built-in function implementations

3. **Value Representation** (`value.rs`)
   - Efficient tagged union for all Shape types
   - Reference counting for arrays and objects
   - Native function interface

4. **Bytecode Format** (`bytecode.rs`)
   - Compact instruction encoding
   - Constant pool for literals
   - Debug information support

### Execution Model

The VM uses a stack-based execution model:
- **Value Stack**: Operands and intermediate results
- **Local Variables**: Function-local storage
- **Global Variables**: Module-level storage
- **Call Stack**: Function call frames

### Memory Layout

```
┌─────────────────┐
│   Constants     │ <- Immutable literals
├─────────────────┤
│   Strings       │ <- Interned strings
├─────────────────┤
│   Functions     │ <- Function metadata
├─────────────────┤
│   Instructions  │ <- Bytecode stream
├─────────────────┤
│   Globals       │ <- Global variables
├─────────────────┤
│   Stack         │ <- Computation stack
├─────────────────┤
│   Locals        │ <- Local variables
└─────────────────┘
```

## Instruction Set

### Stack Operations
- `PUSH_CONST` - Push constant from pool
- `PUSH_NULL` - Push null value
- `POP` - Remove top of stack
- `DUP` - Duplicate top value
- `SWAP` - Swap top two values

### Arithmetic Operations
- `ADD`, `SUB`, `MUL`, `DIV`, `MOD` - Basic arithmetic
- `NEG` - Negate number

### Comparison Operations
- `GT`, `LT`, `GTE`, `LTE` - Numeric comparison
- `EQ`, `NEQ` - Equality checks
- `FUZZY_EQ`, `FUZZY_GT`, `FUZZY_LT` - Fuzzy comparisons

### Logical Operations
- `AND`, `OR`, `NOT` - Boolean logic

### Control Flow
- `JUMP` - Unconditional jump
- `JUMP_IF_FALSE` - Conditional jump
- `JUMP_IF_TRUE` - Conditional jump
- `CALL` - Function call
- `RETURN` - Return from function
- `RETURN_VALUE` - Return with value

### Variable Access
- `LOAD_LOCAL` - Load local variable
- `STORE_LOCAL` - Store local variable
- `LOAD_MODULE_BINDING` - Load module-scope binding
- `STORE_MODULE_BINDING` - Store module-scope binding

### Object/Array Operations
- `NEW_ARRAY` - Create array
- `NEW_OBJECT` - Create object
- `GET_PROP` - Get property/index
- `SET_PROP` - Set property/index
- `LENGTH` - Get length

### Domain-Specific
- `LOAD_CANDLE` - Load candle data
- `CANDLE_PROP` - Get candle property
- `INDICATOR` - Call indicator function
- `PATTERN` - Pattern matching

## Bytecode Format

### Instruction Encoding

Each instruction consists of:
- **Opcode** (1 byte): The operation to perform
- **Operand** (variable): Optional data for the operation

```
┌──────────┬─────────────────┐
│  Opcode  │     Operand     │
│ (1 byte) │  (0-4 bytes)    │
└──────────┴─────────────────┘
```

### Operand Types

- **Const(u16)**: Constant pool index
- **Local(u16)**: Local variable index  
- **Global(u16)**: Global variable index
- **Offset(i32)**: Jump offset
- **Function(u16)**: Function index
- **Count(u16)**: Element count

### Example Bytecode

Shape source:
```shape
let x = 10;
let y = x * 2;
if y > 15 {
    return y;
}
```

Bytecode:
```
0000: PUSH_CONST 0      ; Push 10
0002: STORE_LOCAL 0     ; Store in x
0004: LOAD_LOCAL 0      ; Load x
0006: PUSH_CONST 1      ; Push 2
0008: MUL               ; Multiply
0009: STORE_LOCAL 1     ; Store in y
0011: LOAD_LOCAL 1      ; Load y
0013: PUSH_CONST 2      ; Push 15
0015: GT                ; Compare
0016: JUMP_IF_FALSE 21  ; Skip if false
0019: LOAD_LOCAL 1      ; Load y
0021: RETURN_VALUE      ; Return
```

## Performance Optimizations

1. **Constant Folding**: Compile-time evaluation of constant expressions
2. **String Interning**: Deduplication of string literals
3. **Inline Caching**: Fast property access for objects
4. **Specialized Instructions**: Domain-specific operations (candles, indicators)

## Future Enhancements

1. **JIT Compilation**: Generate native code for hot paths
2. **Register-Based VM**: More efficient than stack-based
3. **Lazy Evaluation**: Defer computation until needed
4. **Parallel Execution**: Multi-threaded backtesting
5. **Memory Pool**: Reduce allocation overhead

## Usage Example

```rust
use shape::vm::{BytecodeCompiler, VirtualMachine, VMConfig};
use shape::parser::parse_program;

// Parse Shape source
let source = "let x = 10; return x * 2;";
let ast = parse_program(source)?;

// Compile to bytecode
let compiler = BytecodeCompiler::new();
let bytecode = compiler.compile(&ast)?;

// Execute in VM
let mut vm = VirtualMachine::new(VMConfig::default());
vm.load_program(bytecode);
let result = vm.execute()?;
```
