# Shape Turing-Complete Features

Shape has been enhanced to be a fully Turing-complete domain-specific language for financial analysis. This document outlines the language features that enable Turing completeness.

## 1. Variables and Mutability

Shape supports three kinds of variable declarations:

```shape
let x = 10;        // Immutable binding (can be shadowed)
var y = 20;        // Mutable variable
const PI = 3.14;   // Constant (cannot be reassigned)
```

Variables have block scope and support shadowing in nested scopes.

## 2. Functions

Functions are first-class citizens with parameters and return values:

```shape
function calculate_sma(prices, period) -> number {
    let sum = 0;
    for (let i = 0; i < period; i = i + 1) {
        sum = sum + prices[i];
    }
    return sum / period;
}
```

Key features:
- Optional return type annotations
- Multiple statements in function bodies
- Return statements with optional values
- Function-local scope

## 3. Control Flow

### If-Else Statements
```shape
if condition {
    // then branch
} else {
    // else branch
}
```

### Loops

#### For-In Loops
```shape
for element in array {
    // Process each element
}
```

#### Traditional For Loops
```shape
for (let i = 0; i < 10; i = i + 1) {
    // Loop body
}
```

#### While Loops
```shape
while condition {
    // Loop body
}
```

### Break and Continue
```shape
for val in values {
    if val < 0 {
        continue;  // Skip negative values
    }
    if val > 100 {
        break;     // Exit loop early
    }
    // Process val
}
```

## 4. Arrays

Arrays are first-class data structures with built-in methods:

```shape
let numbers = [1, 2, 3, 4, 5];
let mixed = [42, "hello", true];  // Mixed types allowed

// Array indexing (0-based)
let first = numbers[0];    // 1
let last = numbers[-1];    // 5 (negative indexing)

// Array methods
let count = len(numbers);                    // 5
let extended = push(numbers, 6, 7);          // [1, 2, 3, 4, 5, 6, 7]
let result = pop(numbers);                   // [[1, 2, 3, 4], 5]
let subset = slice(numbers, 1, 4);           // [2, 3, 4]
let doubled = map(numbers, double_func);     // Transform each element
let evens = filter(numbers, is_even_func);   // Select matching elements

// Create arrays with range
let indices = range(10);             // [0, 1, 2, ..., 9]
let custom = range(5, 15, 2);        // [5, 7, 9, 11, 13]

// Arrays can be iterated
for num in numbers {
    // Process each number
}
```

### Array Methods:
- `len(array)` - Returns the length of the array
- `push(array, value1, ...)` - Returns new array with values appended
- `pop(array)` - Returns [new_array, popped_value] tuple
- `slice(array, start[, end])` - Returns subset of array (supports negative indices)
- `map(array, function_name)` - Transforms each element using the function
- `filter(array, function_name)` - Selects elements where function returns true
- `range([start,] stop[, step])` - Creates numeric array

## 6. Type System (In Progress)

Shape supports optional type annotations:

```shape
let x: number = 42;
let names: string[] = ["AAPL", "GOOGL"];

function add(a: number, b: number) -> number {
    return a + b;
}
```

## 7. Pattern Matching and Financial DSL

Shape retains its domain-specific features while being Turing complete:

```shape
// Define reusable patterns
pattern reversal_signal {
    candle[-2].close < candle[-1].close and
    candle[-1].close > candle[0].close and
    candle[0].volume > avg(candle[-10:-1].volume)
}

// Use in queries with full programmatic control
function analyze_reversals(symbols) {
    let results = [];
    
    for symbol in symbols {
        let matches = find reversal_signal 
                      where candle[0].volume > 1000000
                      last(30 days);
        
        if matches.length > 0 {
            // Process matches
        }
    }
    
    return results;
}
```

## 8. Recursion

Functions can call themselves, enabling recursive algorithms:

```shape
function factorial(n) {
    if n <= 1 {
        return 1;
    }
    return n * factorial(n - 1);
}
```

## 5. Objects/Maps

Objects are key-value data structures with dynamic property access:

```shape
// Object literal
let config = {
    symbol: "AAPL",
    max_position: 100,
    stop_loss: 0.02
};

// Property access
let symbol = config.symbol;          // Dot notation
let stop = config["stop_loss"];      // Bracket notation

// Dynamic property access
let field = "max_position";
let value = config[field];           // 100

// Object methods
let k = keys(config);                // ["symbol", "max_position", "stop_loss"]
let v = values(config);              // ["AAPL", 100, 0.02]
let e = entries(config);             // [["symbol", "AAPL"], ...]
let size = len(config);              // 3

// Iterate over object keys
for key in config {
    let val = config[key];
}
```

### Object Methods:
- `keys(object)` - Returns array of object keys
- `values(object)` - Returns array of object values
- `entries(object)` - Returns array of [key, value] pairs
- `len(object)` - Returns number of properties

## Implementation Status

### Completed:
- ✅ Variable declarations (let/var/const)
- ✅ Function definitions with statements
- ✅ Control flow (if/else)
- ✅ Loops (for-in, for, while)
- ✅ Break/continue statements
- ✅ Arrays and array indexing
- ✅ Array methods (push, pop, slice, map, filter, len, range)
- ✅ Object/map literals with property access
- ✅ Object methods (keys, values, entries)
- ✅ Block scoping
- ✅ Return statements
- ✅ Bytecode VM with stack-based execution
- ✅ Bytecode compiler from AST
- ✅ VM instruction set design

### In Progress:
- 🚧 Module system
- 🚧 Type checking
- 🚧 Standard library
- 🚧 VM debugging and profiling

### Planned:
- 📋 Closures with captured variables
- 📋 Error handling (try/catch)
- 📋 Async/await for real-time data
- 📋 JIT compilation for hot paths

## Examples

See the `examples/` directory for complete examples:
- `test_loops.shape` - Loop demonstrations
- `test_array_sum.shape` - Array operations
- `turing_complete_demo.shape` - Comprehensive feature showcase

## Next Steps

With Turing completeness achieved, Shape can now express any computable financial analysis algorithm while maintaining its domain-specific advantages for pattern matching and time series analysis.