//! Bytecode instruction set for Shape VM

use serde::{Deserialize, Serialize};

/// Re-export `StringId` from `shape-value` — the canonical definition.
pub use shape_value::StringId;

/// Opcode category for classification and tooling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpcodeCategory {
    Stack,
    Arithmetic,
    Comparison,
    Logical,
    Control,
    Variable,
    Object,
    Loop,
    Builtin,
    Exception,
    DataFrame,
    Async,
    Trait,
    Special,
}

/// Macro to define the OpCode enum with metadata (category, stack effects).
///
/// Generates:
/// - `OpCode` enum with `#[repr(u8)]` and explicit byte values
/// - `OpCode::category()` returning `OpcodeCategory`
/// - `OpCode::stack_pops()` and `OpCode::stack_pushes()` returning `u8`
///
/// For opcodes with variable stack effects (Call, CallMethod, NewArray, etc.),
/// use 0/0 since the actual effect depends on runtime arity.
macro_rules! define_opcodes {
    ($($(#[doc = $doc:expr])* $name:ident = $byte:literal, $cat:ident, pops: $pops:expr, pushes: $pushes:expr);* $(;)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[repr(u8)]
        pub enum OpCode {
            $(
                $(#[doc = $doc])*
                $name = $byte,
            )*
        }

        impl OpCode {
            /// Returns the category this opcode belongs to.
            pub const fn category(self) -> OpcodeCategory {
                match self {
                    $( OpCode::$name => OpcodeCategory::$cat, )*
                }
            }

            /// Returns the number of values this opcode pops from the stack.
            /// Returns 0 for variable-arity opcodes (Call, CallMethod, NewArray, etc.).
            pub const fn stack_pops(self) -> u8 {
                match self {
                    $( OpCode::$name => $pops, )*
                }
            }

            /// Returns the number of values this opcode pushes onto the stack.
            /// Returns 0 for variable-arity opcodes.
            pub const fn stack_pushes(self) -> u8 {
                match self {
                    $( OpCode::$name => $pushes, )*
                }
            }
        }
    };
}

define_opcodes! {
    // ===== Stack Operations =====
    /// Push a constant onto the stack
    PushConst = 0x00, Stack, pops: 0, pushes: 1;
    /// Push null onto the stack
    PushNull = 0x01, Stack, pops: 0, pushes: 1;
    /// Pop value from stack
    Pop = 0x02, Stack, pops: 1, pushes: 0;
    /// Duplicate top of stack
    Dup = 0x03, Stack, pops: 1, pushes: 2;
    /// Swap top two values
    Swap = 0x04, Stack, pops: 2, pushes: 2;

    // ===== Arithmetic Operations =====
    /// Add two numbers
    Add = 0x10, Arithmetic, pops: 2, pushes: 1;
    /// Subtract two numbers
    Sub = 0x11, Arithmetic, pops: 2, pushes: 1;
    /// Multiply two numbers
    Mul = 0x12, Arithmetic, pops: 2, pushes: 1;
    /// Divide two numbers
    Div = 0x13, Arithmetic, pops: 2, pushes: 1;
    /// Modulo operation
    Mod = 0x14, Arithmetic, pops: 2, pushes: 1;
    /// Negate number
    Neg = 0x15, Arithmetic, pops: 1, pushes: 1;
    /// Power operation
    Pow = 0x16, Arithmetic, pops: 2, pushes: 1;
    /// Bitwise AND
    BitAnd = 0x17, Arithmetic, pops: 2, pushes: 1;
    /// Bitwise OR
    BitOr = 0x18, Arithmetic, pops: 2, pushes: 1;
    /// Bitwise shift left
    BitShl = 0x19, Arithmetic, pops: 2, pushes: 1;
    /// Bitwise shift right
    BitShr = 0x1A, Arithmetic, pops: 2, pushes: 1;
    /// Bitwise NOT
    BitNot = 0x1B, Arithmetic, pops: 1, pushes: 1;
    /// Bitwise XOR
    BitXor = 0x1C, Arithmetic, pops: 2, pushes: 1;

    // ===== Comparison Operations =====
    /// Greater than
    Gt = 0x20, Comparison, pops: 2, pushes: 1;
    /// Less than
    Lt = 0x21, Comparison, pops: 2, pushes: 1;
    /// Greater than or equal
    Gte = 0x22, Comparison, pops: 2, pushes: 1;
    /// Less than or equal
    Lte = 0x23, Comparison, pops: 2, pushes: 1;
    /// Equal
    Eq = 0x24, Comparison, pops: 2, pushes: 1;
    /// Not equal
    Neq = 0x25, Comparison, pops: 2, pushes: 1;

    // ===== Typed Comparison Operations (compiler-guaranteed types, zero dispatch) =====
    /// Greater than (int × int → bool)
    GtInt = 0x26, Comparison, pops: 2, pushes: 1;
    /// Greater than (f64 × f64 → bool)
    GtNumber = 0x27, Comparison, pops: 2, pushes: 1;
    /// Greater than (decimal × decimal → bool)
    GtDecimal = 0x28, Comparison, pops: 2, pushes: 1;
    /// Less than (int × int → bool)
    LtInt = 0x29, Comparison, pops: 2, pushes: 1;
    /// Less than (f64 × f64 → bool)
    LtNumber = 0x2A, Comparison, pops: 2, pushes: 1;
    /// Less than (decimal × decimal → bool)
    LtDecimal = 0x2B, Comparison, pops: 2, pushes: 1;
    /// Greater than or equal (int × int → bool)
    GteInt = 0x2C, Comparison, pops: 2, pushes: 1;
    /// Greater than or equal (f64 × f64 → bool)
    GteNumber = 0x2D, Comparison, pops: 2, pushes: 1;
    /// Greater than or equal (decimal × decimal → bool)
    GteDecimal = 0x2E, Comparison, pops: 2, pushes: 1;
    /// Less than or equal (int × int → bool)
    LteInt = 0x2F, Comparison, pops: 2, pushes: 1;

    // ===== Logical Operations =====
    /// Logical AND
    And = 0x30, Logical, pops: 2, pushes: 1;
    /// Logical OR
    Or = 0x31, Logical, pops: 2, pushes: 1;
    /// Logical NOT
    Not = 0x32, Logical, pops: 1, pushes: 1;

    // ===== Typed Arithmetic Operations (compiler-guaranteed types, zero dispatch) =====
    /// Add (int × int → int)
    AddInt = 0x33, Arithmetic, pops: 2, pushes: 1;
    /// Add (f64 × f64 → f64)
    AddNumber = 0x34, Arithmetic, pops: 2, pushes: 1;
    /// Add (decimal × decimal → decimal)
    AddDecimal = 0x35, Arithmetic, pops: 2, pushes: 1;
    /// Subtract (int × int → int)
    SubInt = 0x36, Arithmetic, pops: 2, pushes: 1;
    /// Subtract (f64 × f64 → f64)
    SubNumber = 0x37, Arithmetic, pops: 2, pushes: 1;
    /// Subtract (decimal × decimal → decimal)
    SubDecimal = 0x38, Arithmetic, pops: 2, pushes: 1;
    /// Multiply (int × int → int)
    MulInt = 0x39, Arithmetic, pops: 2, pushes: 1;
    /// Multiply (f64 × f64 → f64)
    MulNumber = 0x3A, Arithmetic, pops: 2, pushes: 1;
    /// Multiply (decimal × decimal → decimal)
    MulDecimal = 0x3B, Arithmetic, pops: 2, pushes: 1;
    /// Divide (int × int → int)
    DivInt = 0x3C, Arithmetic, pops: 2, pushes: 1;
    /// Divide (f64 × f64 → f64)
    DivNumber = 0x3D, Arithmetic, pops: 2, pushes: 1;
    /// Divide (decimal × decimal → decimal)
    DivDecimal = 0x3E, Arithmetic, pops: 2, pushes: 1;
    /// Modulo (int × int → int)
    ModInt = 0x3F, Arithmetic, pops: 2, pushes: 1;

    // ===== Control Flow =====
    /// Unconditional jump
    Jump = 0x40, Control, pops: 0, pushes: 0;
    /// Jump if false (pop condition)
    JumpIfFalse = 0x41, Control, pops: 1, pushes: 0;
    /// Jump if true (pop condition)
    JumpIfTrue = 0x42, Control, pops: 1, pushes: 0;
    /// Function call
    Call = 0x43, Control, pops: 0, pushes: 0;
    /// Return from function
    Return = 0x44, Control, pops: 0, pushes: 0;
    /// Return with value
    ReturnValue = 0x45, Control, pops: 1, pushes: 0;
    /// Call a value (function/closure) from the stack
    CallValue = 0x46, Control, pops: 0, pushes: 0;

    // ===== Variable Operations =====
    /// Load local variable
    LoadLocal = 0x50, Variable, pops: 0, pushes: 1;
    /// Store local variable
    StoreLocal = 0x51, Variable, pops: 1, pushes: 0;
    /// Load module_binding variable
    LoadModuleBinding = 0x52, Variable, pops: 0, pushes: 1;
    /// Store module_binding variable
    StoreModuleBinding = 0x53, Variable, pops: 1, pushes: 0;
    /// Load from closure upvalue
    LoadClosure = 0x54, Variable, pops: 0, pushes: 1;
    /// Store to closure upvalue
    StoreClosure = 0x55, Variable, pops: 1, pushes: 0;
    /// Create a closure with captured upvalues
    MakeClosure = 0x56, Variable, pops: 0, pushes: 1;
    /// Close upvalue - moves stack local to heap when leaving scope
    CloseUpvalue = 0x57, Variable, pops: 0, pushes: 0;
    /// Create a reference to a local variable's stack slot
    MakeRef = 0x58, Variable, pops: 0, pushes: 1;
    /// Load the value that a reference points to
    DerefLoad = 0x59, Variable, pops: 0, pushes: 1;
    /// Store a value through a reference
    DerefStore = 0x5A, Variable, pops: 1, pushes: 0;
    /// Set an index on the array that a reference points to (in-place mutation)
    SetIndexRef = 0x5B, Variable, pops: 2, pushes: 0;
    /// Box a local variable into a SharedCell for mutable closure capture.
    /// Converts the local slot to a SharedCell (if not already one), then pushes
    /// the SharedCell ValueWord onto the stack for MakeClosure to consume.
    BoxLocal = 0x5C, Variable, pops: 0, pushes: 1;
    /// Box a module binding into a SharedCell for mutable closure capture.
    /// Same as BoxLocal but operates on the module_bindings vector.
    BoxModuleBinding = 0x5D, Variable, pops: 0, pushes: 1;
    /// Create a projected typed-field reference from a base reference on the stack.
    MakeFieldRef = 0x5E, Variable, pops: 1, pushes: 1;
    /// Create a projected index reference: pops [base_ref, index] and pushes a
    /// projected reference whose `RefProjection::Index` stores the index value.
    MakeIndexRef = 0x5F, Variable, pops: 2, pushes: 1;

    // ===== Object/Array Operations =====
    /// Create new array
    NewArray = 0x60, Object, pops: 0, pushes: 1;
    /// Create new object
    NewObject = 0x61, Object, pops: 0, pushes: 1;
    /// Get property/index
    GetProp = 0x62, Object, pops: 2, pushes: 1;
    /// Set property/index
    SetProp = 0x63, Object, pops: 3, pushes: 0;
    /// Get array/object length
    Length = 0x64, Object, pops: 1, pushes: 1;
    /// Push value to array
    ArrayPush = 0x65, Object, pops: 2, pushes: 0;
    /// Pop value from array
    ArrayPop = 0x66, Object, pops: 1, pushes: 1;
    /// Merge object fields from stack into another object
    MergeObject = 0x67, Object, pops: 2, pushes: 1;
    /// Set index on a local array without loading/cloning through the stack
    SetLocalIndex = 0x68, Object, pops: 2, pushes: 0;
    /// Set index on a module_binding array without loading/cloning through the stack
    SetModuleBindingIndex = 0x69, Object, pops: 2, pushes: 0;
    /// Push value to array stored in a local variable, mutating in-place
    ArrayPushLocal = 0x6A, Object, pops: 1, pushes: 0;
    /// Create a new Matrix from rows*cols f64 values on the stack
    NewMatrix = 0x6B, Object, pops: 0, pushes: 1;
    /// Create a typed array (IntArray/FloatArray/BoolArray) from N homogeneous elements
    /// Operand: Count(n) — number of elements to pop
    /// At runtime, inspects element types and packs into the appropriate typed array
    NewTypedArray = 0x6C, Object, pops: 0, pushes: 1;

    // ===== Loop Operations =====
    /// Start of loop (for break/continue)
    LoopStart = 0x70, Loop, pops: 0, pushes: 0;
    /// End of loop
    LoopEnd = 0x71, Loop, pops: 0, pushes: 0;
    /// Break from loop
    Break = 0x72, Loop, pops: 0, pushes: 0;
    /// Continue to next iteration
    Continue = 0x73, Loop, pops: 0, pushes: 0;
    /// Iterator next: pops iterator + index, pushes next value
    IterNext = 0x74, Loop, pops: 2, pushes: 1;
    /// Check if iterator done: pops iterator + index, pushes bool
    IterDone = 0x75, Loop, pops: 2, pushes: 1;

    // ===== Typed Conversion Operations (direct, zero-dispatch) =====
    /// Convert value to int (infallible, panics on failure)
    ConvertToInt = 0x76, Arithmetic, pops: 1, pushes: 1;
    /// Convert value to number (infallible, panics on failure)
    ConvertToNumber = 0x77, Arithmetic, pops: 1, pushes: 1;
    /// Convert value to string (infallible, always succeeds)
    ConvertToString = 0x78, Arithmetic, pops: 1, pushes: 1;
    /// Convert value to bool (infallible, panics on failure)
    ConvertToBool = 0x79, Arithmetic, pops: 1, pushes: 1;
    /// Convert value to decimal (infallible, panics on failure)
    ConvertToDecimal = 0x7A, Arithmetic, pops: 1, pushes: 1;
    /// Convert value to char (infallible, panics on failure)
    ConvertToChar = 0x7B, Arithmetic, pops: 1, pushes: 1;
    /// Try convert value to int (fallible, pushes Result<int, AnyError>)
    TryConvertToInt = 0x7C, Arithmetic, pops: 1, pushes: 1;
    /// Try convert value to number (fallible, pushes Result<number, AnyError>)
    TryConvertToNumber = 0x7D, Arithmetic, pops: 1, pushes: 1;
    /// Try convert value to string (fallible, pushes Result<string, AnyError>)
    TryConvertToString = 0x7E, Arithmetic, pops: 1, pushes: 1;
    /// Try convert value to bool (fallible, pushes Result<bool, AnyError>)
    TryConvertToBool = 0x7F, Arithmetic, pops: 1, pushes: 1;
    /// Try convert value to decimal (fallible, pushes Result<decimal, AnyError>)
    TryConvertToDecimal = 0x80, Arithmetic, pops: 1, pushes: 1;
    /// Try convert value to char (fallible, pushes Result<char, AnyError>)
    TryConvertToChar = 0x81, Arithmetic, pops: 1, pushes: 1;

    // ===== Method Call =====
    /// Call method on value (array.map(), string.len(), etc.)
    CallMethod = 0x88, Builtin, pops: 0, pushes: 0;
    /// Push timeframe context
    PushTimeframe = 0x89, Builtin, pops: 1, pushes: 0;
    /// Pop timeframe context
    PopTimeframe = 0x8A, Builtin, pops: 0, pushes: 0;

    // ===== Built-in Functions =====
    /// Call built-in function
    BuiltinCall = 0x90, Builtin, pops: 0, pushes: 0;
    /// Type check
    TypeCheck = 0x91, Builtin, pops: 1, pushes: 1;
    /// Convert type
    Convert = 0x92, Builtin, pops: 1, pushes: 1;

    // ===== Typed Arithmetic (continued from 0x3F) =====
    /// Modulo (f64 × f64 → f64)
    ModNumber = 0x93, Arithmetic, pops: 2, pushes: 1;
    /// Modulo (decimal × decimal → decimal)
    ModDecimal = 0x94, Arithmetic, pops: 2, pushes: 1;
    /// Power (int × int → int)
    PowInt = 0x95, Arithmetic, pops: 2, pushes: 1;
    /// Power (f64 × f64 → f64)
    PowNumber = 0x96, Arithmetic, pops: 2, pushes: 1;
    /// Power (decimal × decimal → decimal)
    PowDecimal = 0x97, Arithmetic, pops: 2, pushes: 1;

    // ===== Typed Comparison (continued from 0x2F) =====
    /// Less than or equal (f64 × f64 → bool)
    LteNumber = 0x98, Comparison, pops: 2, pushes: 1;
    /// Less than or equal (decimal × decimal → bool)
    LteDecimal = 0x99, Comparison, pops: 2, pushes: 1;
    /// Equal (int × int → bool)
    EqInt = 0x9A, Comparison, pops: 2, pushes: 1;
    /// Equal (f64 × f64 → bool)
    EqNumber = 0x9B, Comparison, pops: 2, pushes: 1;
    /// Not equal (int × int → bool)
    NeqInt = 0x9C, Comparison, pops: 2, pushes: 1;
    /// Not equal (f64 × f64 → bool)
    NeqNumber = 0x9D, Comparison, pops: 2, pushes: 1;

    // ===== Exception Handling =====
    /// Set up try/catch block (operand: offset to catch handler)
    SetupTry = 0xA0, Exception, pops: 0, pushes: 0;
    /// Pop exception handler (successful try block completion)
    PopHandler = 0xA1, Exception, pops: 0, pushes: 0;
    /// Throw an exception (push error value first)
    Throw = 0xA2, Exception, pops: 1, pushes: 0;
    /// Try operator: unified Result/Option propagation with early return on Err/None
    TryUnwrap = 0xA3, Exception, pops: 1, pushes: 1;
    /// Unwrap Option: extract inner value from Some, panic on None
    UnwrapOption = 0xA4, Exception, pops: 1, pushes: 1;
    /// Add context to Result/Option failures and lift success into Result
    ErrorContext = 0xA5, Exception, pops: 2, pushes: 1;
    /// Check whether Result is Ok(value)
    IsOk = 0xA6, Exception, pops: 1, pushes: 1;
    /// Check whether Result is Err(error)
    IsErr = 0xA7, Exception, pops: 1, pushes: 1;
    /// Extract inner payload from Ok(value)
    UnwrapOk = 0xA8, Exception, pops: 1, pushes: 1;
    /// Extract inner payload from Err(error)
    UnwrapErr = 0xA9, Exception, pops: 1, pushes: 1;

    // ===== Additional Operations =====
    /// Slice access (array[start:end])
    SliceAccess = 0xB0, Object, pops: 3, pushes: 1;
    /// Null coalescing (a ?? b)
    NullCoalesce = 0xB1, Logical, pops: 2, pushes: 1;
    /// Range constructor (start..end)
    MakeRange = 0xB2, Object, pops: 2, pushes: 1;

    // ===== Compact Typed Arithmetic (width-parameterised, ABI-stable) =====
    /// Width-typed add: Operand::Width selects I8..F64
    AddTyped = 0xB3, Arithmetic, pops: 2, pushes: 1;
    /// Width-typed subtract: Operand::Width selects I8..F64
    SubTyped = 0xB4, Arithmetic, pops: 2, pushes: 1;
    /// Width-typed multiply: Operand::Width selects I8..F64
    MulTyped = 0xB5, Arithmetic, pops: 2, pushes: 1;
    /// Width-typed divide: Operand::Width selects I8..F64
    DivTyped = 0xB6, Arithmetic, pops: 2, pushes: 1;
    /// Width-typed modulo: Operand::Width selects I8..F64
    ModTyped = 0xB7, Arithmetic, pops: 2, pushes: 1;
    /// Width-typed comparison (ordered): Operand::Width selects I8..F64
    /// Result semantics: pushes -1 (a<b), 0 (a==b), or 1 (a>b)
    CmpTyped = 0xB8, Comparison, pops: 2, pushes: 1;

    // ===== DataFrame Operations =====
    /// Get field from data row by column index (generic, industry-agnostic)
    GetDataField = 0xC0, DataFrame, pops: 1, pushes: 1;
    /// Get row reference (lightweight, no data copy)
    GetDataRow = 0xC1, DataFrame, pops: 1, pushes: 1;

    // ===== Type-Specialized Operations (JIT Optimization) =====
    /// Get field from typed object using precomputed offset
    GetFieldTyped = 0xD0, Object, pops: 1, pushes: 1;
    /// Set field on typed object using precomputed offset
    SetFieldTyped = 0xD1, Object, pops: 2, pushes: 1;
    /// Create a new typed object with fields from stack
    NewTypedObject = 0xD2, Object, pops: 0, pushes: 1;
    /// Merge two typed objects into a new typed object
    TypedMergeObject = 0xD3, Object, pops: 2, pushes: 1;
    /// Wrap a value with a type annotation for meta formatting
    WrapTypeAnnotation = 0xD4, Object, pops: 1, pushes: 1;

    // ===== Async Operations (0xE0-0xEF) =====
    /// Yield to event loop for cooperative scheduling
    Yield = 0xE0, Async, pops: 0, pushes: 0;
    /// Suspend until a condition is met
    Suspend = 0xE1, Async, pops: 0, pushes: 0;
    /// Resume from suspension (internal use)
    Resume = 0xE2, Async, pops: 1, pushes: 0;
    /// Poll event queue
    Poll = 0xE3, Async, pops: 0, pushes: 1;
    /// Await next data bar from a source
    AwaitBar = 0xE4, Async, pops: 0, pushes: 1;
    /// Await next timer tick
    AwaitTick = 0xE5, Async, pops: 0, pushes: 0;
    /// General-purpose await: suspends on Future values
    Await = 0xE6, Async, pops: 1, pushes: 1;
    /// Spawn an async task from the expression on top of stack
    SpawnTask = 0xE7, Async, pops: 1, pushes: 1;

    // ===== Event Emission Operations =====
    /// Emit an alert to the alert pipeline
    EmitAlert = 0xE8, Async, pops: 1, pushes: 0;
    /// Emit a generic event to the event queue
    EmitEvent = 0xE9, Async, pops: 1, pushes: 0;
    /// Initialize a join group from spawned tasks on the stack
    JoinInit = 0xEA, Async, pops: 0, pushes: 1;
    /// Await a TaskGroup to completion according to its join strategy
    JoinAwait = 0xEB, Async, pops: 1, pushes: 1;
    /// Cancel a running task
    CancelTask = 0xEC, Async, pops: 1, pushes: 0;
    /// Enter an async scope (structured concurrency boundary)
    AsyncScopeEnter = 0xED, Async, pops: 0, pushes: 0;
    /// Exit an async scope (structured concurrency boundary)
    AsyncScopeExit = 0xEE, Async, pops: 0, pushes: 0;

    // ===== Typed Column Access (Arrow DataTable) =====
    /// Load f64 from typed column on a RowView
    LoadColF64 = 0xC2, DataFrame, pops: 1, pushes: 1;
    /// Load i64 from typed column on a RowView
    LoadColI64 = 0xC3, DataFrame, pops: 1, pushes: 1;
    /// Load bool from typed column on a RowView
    LoadColBool = 0xC4, DataFrame, pops: 1, pushes: 1;
    /// Load string from typed column on a RowView
    LoadColStr = 0xC5, DataFrame, pops: 1, pushes: 1;
    /// Bind a DataTable to a TypeSchema at runtime (safety net for dynamic paths)
    BindSchema = 0xC6, DataFrame, pops: 1, pushes: 1;

    // ===== Trait Object Operations =====
    /// Box a concrete value into a trait object with a vtable
    BoxTraitObject = 0xEF, Trait, pops: 1, pushes: 1;
    /// Call a method on a trait object via vtable dispatch
    DynMethodCall = 0xC7, Trait, pops: 0, pushes: 0;
    /// Call Drop::drop on the value at the top of stack (sync)
    DropCall = 0xC8, Trait, pops: 1, pushes: 0;
    /// Call Drop::drop on the value at the top of stack (async)
    DropCallAsync = 0xC9, Trait, pops: 1, pushes: 0;

    // NOTE: Trusted arithmetic opcodes (0xCA-0xCF, 0xD5-0xD6) were removed.
    // They were functionally identical to the typed variants (AddInt, etc.)
    // in release builds. The typed opcodes already skip runtime dispatch.

    // ===== Trusted Variable Operations (compiler-proved types, zero guard) =====
    /// LoadLocal (trusted) -- skips tag validation, reads slot directly
    LoadLocalTrusted = 0xD7, Variable, pops: 0, pushes: 1;

    // ===== Trusted Control Flow (compiler-proved types, zero guard) =====
    /// JumpIfFalse (trusted) -- condition is known bool, direct bool check
    JumpIfFalseTrusted = 0xD8, Control, pops: 1, pushes: 0;

    // NOTE: Trusted comparison opcodes (0xD9-0xDF, 0xF9) were removed.
    // They were functionally identical to the typed variants (GtInt, etc.)
    // in release builds. The typed opcodes already skip runtime dispatch.

    // ===== Special Operations =====
    /// No operation
    Nop = 0xF0, Special, pops: 0, pushes: 0;
    /// Halt execution
    Halt = 0xF1, Special, pops: 0, pushes: 0;
    /// Debug breakpoint
    Debug = 0xF2, Special, pops: 0, pushes: 0;

    // ===== Numeric Coercion Operations =====
    /// Coerce int to number (i64 -> f64)
    IntToNumber = 0xF3, Arithmetic, pops: 1, pushes: 1;
    /// Coerce number to int (f64 -> i64, truncating)
    NumberToInt = 0xF4, Arithmetic, pops: 1, pushes: 1;

    // ===== Foreign Function Operations =====
    /// Call a linked foreign function.
    /// Dispatches through language runtime extensions or the VM native C ABI path.
    /// Operand: ForeignFunction(u16) — index into program.foreign_functions
    /// Stack: pops N args (count pushed as a constant by the stub), pushes 1 result
    CallForeign = 0xF5, Control, pops: 0, pushes: 0;

    /// Store a local with width truncation.
    /// Operand: TypedLocal(u16, NumericWidth) — local index + width
    /// Pops one value, truncates to declared width, stores to local.
    StoreLocalTyped = 0xF6, Variable, pops: 1, pushes: 0;

    /// Cast a value to a specific integer width (bit-truncation, Rust-style `as`).
    /// Operand: Width(NumericWidth) — target width
    /// Pops one value, truncates, pushes result.
    CastWidth = 0xF7, Arithmetic, pops: 1, pushes: 1;

    /// Store a module binding with width truncation.
    /// Operand: TypedModuleBinding(u16, NumericWidth) — binding index + width
    /// Pops one value, truncates to declared width, stores to module binding.
    StoreModuleBindingTyped = 0xF8, Variable, pops: 1, pushes: 0;

    // ===== v2 Typed Array Operations =====
    /// Create a new TypedArray<f64> with given capacity. Operand: Count(capacity). Pushes ptr.
    NewTypedArrayF64 = 0x05, Object, pops: 0, pushes: 1;
    /// Create a new TypedArray<i64> with given capacity. Operand: Count(capacity). Pushes ptr.
    NewTypedArrayI64 = 0x06, Object, pops: 0, pushes: 1;
    /// Create a new TypedArray<i32> with given capacity. Operand: Count(capacity). Pushes ptr.
    NewTypedArrayI32 = 0x07, Object, pops: 0, pushes: 1;
    /// Get element from TypedArray<f64>: pops (arr_ptr, index), pushes f64 value
    TypedArrayGetF64 = 0x08, Object, pops: 2, pushes: 1;
    /// Get element from TypedArray<i64>: pops (arr_ptr, index), pushes i64 value
    TypedArrayGetI64 = 0x09, Object, pops: 2, pushes: 1;
    /// Get element from TypedArray<i32>: pops (arr_ptr, index), pushes i32 value
    TypedArrayGetI32 = 0x0A, Object, pops: 2, pushes: 1;
    /// Set element in TypedArray<f64>: pops (arr_ptr, index, value), pushes nothing
    TypedArraySetF64 = 0x0B, Object, pops: 3, pushes: 0;
    /// Push element to TypedArray<f64>: pops (arr_ptr, value), pushes nothing
    TypedArrayPushF64 = 0x0C, Object, pops: 2, pushes: 0;
    /// Push element to TypedArray<i64>: pops (arr_ptr, value), pushes nothing
    TypedArrayPushI64 = 0x0D, Object, pops: 2, pushes: 0;
    /// Get length of TypedArray: pops (arr_ptr), pushes len as int
    TypedArrayLen = 0x0E, Object, pops: 1, pushes: 1;

    // ===== v2 Typed Field Access Operations =====
    /// Load f64 field from typed struct at byte offset. Operand: FieldOffset(u16). Pops struct_ptr, pushes f64.
    FieldLoadF64 = 0x82, Object, pops: 1, pushes: 1;
    /// Load i64 field from typed struct at byte offset. Operand: FieldOffset(u16). Pops struct_ptr, pushes i64.
    FieldLoadI64 = 0x83, Object, pops: 1, pushes: 1;
    /// Load i32 field from typed struct at byte offset. Operand: FieldOffset(u16). Pops struct_ptr, pushes i32.
    FieldLoadI32 = 0x84, Object, pops: 1, pushes: 1;
    /// Load bool field from typed struct at byte offset. Operand: FieldOffset(u16). Pops struct_ptr, pushes bool.
    FieldLoadBool = 0x85, Object, pops: 1, pushes: 1;
    /// Load ptr field from typed struct at byte offset. Operand: FieldOffset(u16). Pops struct_ptr, pushes ptr.
    FieldLoadPtr = 0x86, Object, pops: 1, pushes: 1;
    /// Store f64 field to typed struct at byte offset. Operand: FieldOffset(u16). Pops (struct_ptr, value).
    FieldStoreF64 = 0x87, Object, pops: 2, pushes: 0;
    /// Store i64 field to typed struct at byte offset. Operand: FieldOffset(u16). Pops (struct_ptr, value).
    FieldStoreI64 = 0x8B, Object, pops: 2, pushes: 0;
    /// Store i32 field to typed struct at byte offset. Operand: FieldOffset(u16). Pops (struct_ptr, value).
    FieldStoreI32 = 0x8C, Object, pops: 2, pushes: 0;
    /// Allocate a new typed struct. Operand: TypedObjectAlloc{schema_id, field_count}. Pushes ptr.
    NewTypedStruct = 0x8D, Object, pops: 0, pushes: 1;

    // ===== v2 Sized Integer (i32) Arithmetic & Comparison =====
    /// Add (i32 x i32 -> i32)
    AddI32 = 0x1D, Arithmetic, pops: 2, pushes: 1;
    /// Subtract (i32 x i32 -> i32)
    SubI32 = 0x1E, Arithmetic, pops: 2, pushes: 1;
    /// Multiply (i32 x i32 -> i32)
    MulI32 = 0x1F, Arithmetic, pops: 2, pushes: 1;
    /// Divide (i32 x i32 -> i32)
    DivI32 = 0x9E, Arithmetic, pops: 2, pushes: 1;
    /// Modulo (i32 x i32 -> i32)
    ModI32 = 0x9F, Arithmetic, pops: 2, pushes: 1;
    /// Equal (i32 x i32 -> bool)
    EqI32 = 0xAA, Comparison, pops: 2, pushes: 1;
    /// Not equal (i32 x i32 -> bool)
    NeqI32 = 0xAB, Comparison, pops: 2, pushes: 1;
    /// Less than (i32 x i32 -> bool)
    LtI32 = 0xAC, Comparison, pops: 2, pushes: 1;
    /// Greater than (i32 x i32 -> bool)
    GtI32 = 0xAD, Comparison, pops: 2, pushes: 1;
    /// Less than or equal (i32 x i32 -> bool)
    LteI32 = 0xAE, Comparison, pops: 2, pushes: 1;
    /// Greater than or equal (i32 x i32 -> bool)
    GteI32 = 0xAF, Comparison, pops: 2, pushes: 1;

}

impl OpCode {
    /// Returns true if this is a trusted opcode variant (compiler-proved types, no runtime guard).
    pub const fn is_trusted(self) -> bool {
        matches!(
            self,
            OpCode::LoadLocalTrusted | OpCode::JumpIfFalseTrusted
        )
    }

    /// Map a trusted opcode back to its guarded (runtime-checked) counterpart.
    ///
    /// This is the inverse of `trusted_variant()`: given a trusted opcode, it
    /// returns the equivalent guarded opcode. Used for differential testing and
    /// bytecode post-processing.
    pub const fn guarded_variant(self) -> Option<OpCode> {
        match self {
            OpCode::LoadLocalTrusted => Some(OpCode::LoadLocal),
            OpCode::JumpIfFalseTrusted => Some(OpCode::JumpIfFalse),
            _ => None,
        }
    }

    /// Returns true if this is a v2 typed opcode (typed arrays, typed fields, sized integers).
    /// These opcodes carry their type in the opcode name and require the v2 runtime path.
    pub const fn is_v2_typed(self) -> bool {
        matches!(
            self,
            // Typed array operations
            OpCode::NewTypedArrayF64
            | OpCode::NewTypedArrayI64
            | OpCode::NewTypedArrayI32
            | OpCode::TypedArrayGetF64
            | OpCode::TypedArrayGetI64
            | OpCode::TypedArrayGetI32
            | OpCode::TypedArraySetF64
            | OpCode::TypedArrayPushF64
            | OpCode::TypedArrayPushI64
            | OpCode::TypedArrayLen
            // Typed field access
            | OpCode::FieldLoadF64
            | OpCode::FieldLoadI64
            | OpCode::FieldLoadI32
            | OpCode::FieldLoadBool
            | OpCode::FieldLoadPtr
            | OpCode::FieldStoreF64
            | OpCode::FieldStoreI64
            | OpCode::FieldStoreI32
            | OpCode::NewTypedStruct
            // Sized integer i32 arithmetic
            | OpCode::AddI32
            | OpCode::SubI32
            | OpCode::MulI32
            | OpCode::DivI32
            | OpCode::ModI32
            | OpCode::EqI32
            | OpCode::NeqI32
            | OpCode::LtI32
            | OpCode::GtI32
            | OpCode::LteI32
            | OpCode::GteI32
        )
    }

    /// Map a guarded typed opcode to its trusted variant (if one exists).
    pub const fn trusted_variant(self) -> Option<OpCode> {
        match self {
            OpCode::LoadLocal => Some(OpCode::LoadLocalTrusted),
            OpCode::JumpIfFalse => Some(OpCode::JumpIfFalseTrusted),
            _ => None,
        }
    }
}

/// Numeric width tag for compact typed opcodes (AddTyped, SubTyped, etc.).
///
/// Encodes the operand width so a single opcode family can handle all
/// numeric types.  The discriminant values are part of the bytecode ABI
/// and must remain stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum NumericWidth {
    I8 = 0,
    I16 = 1,
    I32 = 2,
    I64 = 3,
    U8 = 4,
    U16 = 5,
    U32 = 6,
    U64 = 7,
    F32 = 8,
    F64 = 9,
}

impl NumericWidth {
    pub const ALL: [Self; 10] = [
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::F32,
        Self::F64,
    ];

    #[inline(always)]
    pub const fn is_integer(self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::I16
                | Self::I32
                | Self::I64
                | Self::U8
                | Self::U16
                | Self::U32
                | Self::U64
        )
    }

    #[inline(always)]
    pub const fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    /// Whether this is a signed integer type.
    #[inline(always)]
    pub const fn is_signed(self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32 | Self::I64)
    }

    /// Whether this is an unsigned integer type.
    #[inline(always)]
    pub const fn is_unsigned(self) -> bool {
        matches!(self, Self::U8 | Self::U16 | Self::U32 | Self::U64)
    }

    /// Number of bits for this width.
    #[inline(always)]
    pub const fn bits(self) -> u32 {
        match self {
            Self::I8 | Self::U8 => 8,
            Self::I16 | Self::U16 => 16,
            Self::I32 | Self::U32 | Self::F32 => 32,
            Self::I64 | Self::U64 | Self::F64 => 64,
        }
    }

    /// Bit mask for the integer value range.
    #[inline(always)]
    pub const fn mask(self) -> u64 {
        match self {
            Self::I8 | Self::U8 => 0xFF,
            Self::I16 | Self::U16 => 0xFFFF,
            Self::I32 | Self::U32 | Self::F32 => 0xFFFF_FFFF,
            Self::I64 | Self::U64 | Self::F64 => u64::MAX,
        }
    }

    /// Convert from IntWidth (shape-ast) to NumericWidth.
    #[inline]
    pub fn from_int_width(w: shape_ast::IntWidth) -> Self {
        match w {
            shape_ast::IntWidth::I8 => Self::I8,
            shape_ast::IntWidth::U8 => Self::U8,
            shape_ast::IntWidth::I16 => Self::I16,
            shape_ast::IntWidth::U16 => Self::U16,
            shape_ast::IntWidth::I32 => Self::I32,
            shape_ast::IntWidth::U32 => Self::U32,
            shape_ast::IntWidth::U64 => Self::U64,
        }
    }

    /// Convert to IntWidth (shape-ast). Returns None for F32/F64/I64.
    #[inline]
    pub fn to_int_width(self) -> Option<shape_ast::IntWidth> {
        match self {
            Self::I8 => Some(shape_ast::IntWidth::I8),
            Self::U8 => Some(shape_ast::IntWidth::U8),
            Self::I16 => Some(shape_ast::IntWidth::I16),
            Self::U16 => Some(shape_ast::IntWidth::U16),
            Self::I32 => Some(shape_ast::IntWidth::I32),
            Self::U32 => Some(shape_ast::IntWidth::U32),
            Self::U64 => Some(shape_ast::IntWidth::U64),
            Self::I64 | Self::F32 | Self::F64 => None,
        }
    }
}

/// A bytecode instruction with its operands
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Instruction {
    pub opcode: OpCode,
    pub operand: Option<Operand>,
}

/// Instruction operands
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Operand {
    /// Constant pool index
    Const(u16),
    /// Local variable index
    Local(u16),
    /// ModuleBinding variable index
    ModuleBinding(u16),
    /// Jump offset (can be negative)
    Offset(i32),
    /// Function index
    Function(shape_value::FunctionId),
    /// Built-in function ID
    Builtin(BuiltinFunction),
    /// Number of arguments/elements
    Count(u16),
    /// Property name index
    Property(u16),
    /// Column index for DataFrame field access (compile-time resolved)
    ColumnIndex(u32),
    /// Typed field access (type_id, field_idx, field_type_tag)
    /// Used with GetFieldTyped/SetFieldTyped for optimized field access.
    /// field_type_tag encodes the FieldType so the executor can read slots
    /// without a runtime schema lookup.
    TypedField {
        type_id: u16,
        field_idx: u16,
        field_type_tag: u16,
    },
    /// Typed object allocation
    /// Used with NewTypedObject for creating TypedObject instances
    TypedObjectAlloc {
        /// Schema ID identifying the type layout
        schema_id: u16,
        /// Number of fields to pop from stack
        field_count: u16,
    },
    /// Typed object merge (compile-time registered intersection schema)
    /// Used with TypedMergeObject for O(1) merge operations
    TypedMerge {
        /// Schema ID for the merged result (pre-registered at compile time)
        target_schema_id: u16,
        /// Byte size of left operand data
        left_size: u16,
        /// Byte size of right operand data
        right_size: u16,
    },
    /// Typed column access on a RowView
    /// Used with LoadColF64/I64/Bool/Str for direct Arrow buffer reads
    ColumnAccess {
        /// Column index in the Arrow schema
        col_id: u32,
    },
    /// A named reference (e.g., trait name for BoxTraitObject)
    Name(StringId),
    /// Typed method call using compile-time resolved MethodId.
    /// For `MethodId::DYNAMIC`, the VM falls back to string lookup
    /// using `string_id` from the string pool.
    TypedMethodCall {
        /// Compile-time resolved method identifier
        method_id: u16,
        /// Number of arguments (not counting receiver)
        arg_count: u16,
        /// String pool index for the method name (used for dynamic fallback
        /// and error messages)
        string_id: u16,
    },
    /// Foreign function index — indexes into program.foreign_functions
    ForeignFunction(u16),
    /// Matrix dimensions (rows, cols) for NewMatrix opcode
    MatrixDims { rows: u16, cols: u16 },
    /// Numeric width tag for compact typed opcodes (AddTyped, SubTyped, etc.)
    Width(NumericWidth),
    /// Local index + width for StoreLocalTyped
    TypedLocal(u16, NumericWidth),
    /// Module binding index + width for StoreModuleBindingTyped
    TypedModuleBinding(u16, NumericWidth),
    /// Byte offset into a typed struct for FieldLoad/FieldStore v2 opcodes
    FieldOffset(u16),
}

/// Built-in functions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltinFunction {
    // Math functions
    Abs,
    Sqrt,
    Ln,
    Pow,
    Exp,
    Log,
    Min,
    Max,
    Floor,
    Ceil,
    Round,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,

    // Statistical functions
    StdDev,

    // Array functions
    Range,
    Slice,
    Push,
    Pop,
    First,
    Last,
    Zip,
    Filled,

    // Array method-style functions
    Map,
    Filter,
    Reduce,
    ForEach,
    Find,
    FindIndex,
    Some,
    Every,

    // Utility functions
    Print,
    Format,
    Len,
    // Throw removed: Shape uses Result types
    Snapshot,
    Exit,

    // Object functions
    ObjectRest,

    // Control flow functions
    ControlFold,

    // Type functions
    TypeOf,
    IsNumber,
    IsString,
    IsBool,
    IsArray,
    IsObject,
    IsDataRow,

    // Conversion
    ToString,
    ToNumber,
    ToBool,

    // Native C/Arrow interop helpers
    NativePtrSize,
    NativePtrNewCell,
    NativePtrFreeCell,
    NativePtrReadPtr,
    NativePtrWritePtr,
    NativeTableFromArrowC,
    NativeTableFromArrowCTyped,
    NativeTableBindType,
    /// Format a value respecting meta formatting for TypeAnnotatedValues.
    /// Used by string interpolation to apply custom formatters.
    FormatValueWithMeta,
    /// Format a value using a typed interpolation format spec.
    /// Used by string interpolation for spec-aware rendering (fixed/table).
    FormatValueWithSpec,

    // Optimization
    IntrinsicMinimize,
    // Math intrinsics (7 functions)
    IntrinsicBspline2_3dBatch,
    IntrinsicSum,
    IntrinsicMean,
    IntrinsicMin,
    IntrinsicMax,
    IntrinsicStd,
    IntrinsicVariance,

    // Random number generation intrinsics (5 functions)
    IntrinsicRandom,
    IntrinsicRandomInt,
    IntrinsicRandomSeed,
    IntrinsicRandomNormal,
    IntrinsicRandomArray,

    // Distribution intrinsics (5 functions)
    IntrinsicDistUniform,
    IntrinsicDistLognormal,
    IntrinsicDistExponential,
    IntrinsicDistPoisson,
    IntrinsicDistSampleN,

    // Stochastic process intrinsics (4 functions)
    IntrinsicBrownianMotion,
    IntrinsicGbm,
    IntrinsicOuProcess,
    IntrinsicRandomWalk,

    // Rolling window intrinsics (6 functions)
    IntrinsicRollingSum,
    IntrinsicRollingMean,
    IntrinsicRollingStd,
    IntrinsicRollingMin,
    IntrinsicRollingMax,
    IntrinsicEma,
    IntrinsicLinearRecurrence,

    // Series transformation intrinsics (7 functions)
    IntrinsicShift,
    IntrinsicDiff,
    IntrinsicPctChange,
    IntrinsicFillna,
    IntrinsicCumsum,
    IntrinsicCumprod,
    IntrinsicClip,

    // Statistical intrinsics (4 functions)
    IntrinsicCorrelation,
    IntrinsicCovariance,
    IntrinsicPercentile,
    IntrinsicMedian,

    // Trigonometric intrinsics (4 functions)
    IntrinsicAtan2,
    IntrinsicSinh,
    IntrinsicCosh,
    IntrinsicTanh,

    // Character code intrinsics
    IntrinsicCharCode,
    IntrinsicFromCharCode,

    // Series access (critical for backtesting!)
    IntrinsicSeries,

    // Vector intrinsics (10 functions)
    IntrinsicVecAbs,
    IntrinsicVecSqrt,
    IntrinsicVecLn,
    IntrinsicVecExp,
    IntrinsicVecAdd,
    IntrinsicVecSub,
    IntrinsicVecMul,
    IntrinsicVecDiv,
    IntrinsicVecMax,
    IntrinsicVecMin,
    IntrinsicVecSelect,

    // Matrix intrinsics (2 functions)
    IntrinsicMatMulVec,
    IntrinsicMatMulMat,

    // Internal evaluation helpers
    EvalTimeRef,
    EvalDateTimeExpr,
    EvalDataDateTimeRef,
    EvalDataSet,
    EvalDataRelative,
    EvalDataRelativeRange,

    // Option type constructors
    SomeCtor,
    OkCtor,
    ErrCtor,

    // Collection constructors
    HashMapCtor,
    SetCtor,
    DequeCtor,
    PriorityQueueCtor,

    // Json navigation helpers (used by std::core::json_value extend block)
    JsonObjectGet,
    JsonArrayAt,
    JsonObjectKeys,
    JsonArrayLen,
    JsonObjectLen,

    // Window functions (SQL-style)
    WindowRowNumber,
    WindowRank,
    WindowDenseRank,
    WindowNtile,
    WindowLag,
    WindowLead,
    WindowFirstValue,
    WindowLastValue,
    WindowNthValue,
    WindowSum,
    WindowAvg,
    WindowMin,
    WindowMax,
    WindowCount,

    // JOIN operations
    JoinExecute,

    // Reflection
    Reflect,

    // Content string builtins
    /// Wrap a string value as ContentNode::plain(text)
    MakeContentText,
    /// Collect N ContentNodes from the stack into a ContentNode::Fragment
    MakeContentFragment,
    /// Apply a ContentFormatSpec (encoded as ints/bools on stack) to a ContentNode
    ApplyContentStyle,
    /// Create a chart ContentNode from a table/array value using column specs
    MakeContentChartFromValue,

    // Content namespace constructors
    /// Content.chart(type_str) — create a chart ContentNode
    ContentChart,
    /// Content.text(str) — create a plain text ContentNode
    ContentTextCtor,
    /// Content.table(headers, rows) — create a table ContentNode
    ContentTableCtor,
    /// Content.code(language, source) — create a code block ContentNode
    ContentCodeCtor,
    /// Content.kv(pairs) — create a key-value ContentNode
    ContentKvCtor,
    /// Content.fragment(parts) — create a fragment ContentNode
    ContentFragmentCtor,

    // DateTime constructors
    /// DateTime.now() — current local time as DateTime<FixedOffset>
    DateTimeNow,
    /// DateTime.utc() — current UTC time as DateTime<FixedOffset> at +00:00
    DateTimeUtc,
    /// DateTime.parse(str) — parse from string (ISO 8601, RFC 2822, common formats)
    DateTimeParse,
    /// DateTime.from_epoch(ms) — from milliseconds since Unix epoch
    DateTimeFromEpoch,
    /// DateTime.from_parts(year, month, day, hour?, minute?, second?) — construct from components
    DateTimeFromParts,
    /// DateTime.from_unix_secs(secs) — from seconds since Unix epoch
    DateTimeFromUnixSecs,

    // Concurrency primitive constructors
    /// Mutex(value) — create a new mutex wrapping the given value
    MutexCtor,
    /// Atomic(value) — create a new atomic integer with the given initial value
    AtomicCtor,
    /// Lazy(initializer) — create a lazy value with the given initializer closure
    LazyCtor,
    /// Channel() — create a new MPSC channel, returns [sender, receiver] array
    ChannelCtor,

    // Additional math builtins
    /// sign(x) — returns -1, 0, or 1
    Sign,
    /// gcd(a, b) — greatest common divisor
    Gcd,
    /// lcm(a, b) — least common multiple
    Lcm,
    /// hypot(a, b) — hypotenuse sqrt(a^2 + b^2)
    Hypot,
    /// clamp(x, min, max) — clamp value between min and max
    Clamp,
    /// isNaN(x) — check if value is NaN
    IsNaN,
    /// isFinite(x) — check if value is finite
    IsFinite,

    /// mat(rows, cols, ...values) — create a Matrix from flat f64 values
    MatFromFlat,

    // Table construction (1)
    /// Build a TypedTable from inline row values: args = [schema_id, row_count, field_count, val1, val2, ...]
    MakeTableFromRows,
}

impl BuiltinFunction {
    /// Convert a discriminant value back to a BuiltinFunction variant.
    ///
    /// Used by the JIT generic builtin trampoline: the translator encodes
    /// the builtin as `*builtin as u16` and the FFI function converts it
    /// back at runtime.
    pub fn from_discriminant(id: u16) -> Option<Self> {
        // Ordered to match the enum declaration order (discriminants 0, 1, 2, ...)
        const VARIANTS: &[BuiltinFunction] = &[
            // Math (18) — discriminants 0..17
            BuiltinFunction::Abs,
            BuiltinFunction::Sqrt,
            BuiltinFunction::Ln,
            BuiltinFunction::Pow,
            BuiltinFunction::Exp,
            BuiltinFunction::Log,
            BuiltinFunction::Min,
            BuiltinFunction::Max,
            BuiltinFunction::Floor,
            BuiltinFunction::Ceil,
            BuiltinFunction::Round,
            BuiltinFunction::Sin,
            BuiltinFunction::Cos,
            BuiltinFunction::Tan,
            BuiltinFunction::Asin,
            BuiltinFunction::Acos,
            BuiltinFunction::Atan,
            // Stats (1)
            BuiltinFunction::StdDev,
            // Array (8)
            BuiltinFunction::Range,
            BuiltinFunction::Slice,
            BuiltinFunction::Push,
            BuiltinFunction::Pop,
            BuiltinFunction::First,
            BuiltinFunction::Last,
            BuiltinFunction::Zip,
            BuiltinFunction::Filled,
            // HOF (8)
            BuiltinFunction::Map,
            BuiltinFunction::Filter,
            BuiltinFunction::Reduce,
            BuiltinFunction::ForEach,
            BuiltinFunction::Find,
            BuiltinFunction::FindIndex,
            BuiltinFunction::Some,
            BuiltinFunction::Every,
            // Utility (5)
            BuiltinFunction::Print,
            BuiltinFunction::Format,
            BuiltinFunction::Len,
            BuiltinFunction::Snapshot,
            BuiltinFunction::Exit,
            // Object (1)
            BuiltinFunction::ObjectRest,
            // Control (1)
            BuiltinFunction::ControlFold,
            // Type (7)
            BuiltinFunction::TypeOf,
            BuiltinFunction::IsNumber,
            BuiltinFunction::IsString,
            BuiltinFunction::IsBool,
            BuiltinFunction::IsArray,
            BuiltinFunction::IsObject,
            BuiltinFunction::IsDataRow,
            // Conversion (3)
            BuiltinFunction::ToString,
            BuiltinFunction::ToNumber,
            BuiltinFunction::ToBool,
            // Native ptr (8)
            BuiltinFunction::NativePtrSize,
            BuiltinFunction::NativePtrNewCell,
            BuiltinFunction::NativePtrFreeCell,
            BuiltinFunction::NativePtrReadPtr,
            BuiltinFunction::NativePtrWritePtr,
            BuiltinFunction::NativeTableFromArrowC,
            BuiltinFunction::NativeTableFromArrowCTyped,
            BuiltinFunction::NativeTableBindType,
            // Format (2)
            BuiltinFunction::FormatValueWithMeta,
            BuiltinFunction::FormatValueWithSpec,
            // Optimization
            BuiltinFunction::IntrinsicMinimize,
            // Math intrinsics (7)
            BuiltinFunction::IntrinsicBspline2_3dBatch,
            BuiltinFunction::IntrinsicSum,
            BuiltinFunction::IntrinsicMean,
            BuiltinFunction::IntrinsicMin,
            BuiltinFunction::IntrinsicMax,
            BuiltinFunction::IntrinsicStd,
            BuiltinFunction::IntrinsicVariance,
            // Random (5)
            BuiltinFunction::IntrinsicRandom,
            BuiltinFunction::IntrinsicRandomInt,
            BuiltinFunction::IntrinsicRandomSeed,
            BuiltinFunction::IntrinsicRandomNormal,
            BuiltinFunction::IntrinsicRandomArray,
            // Distribution (5)
            BuiltinFunction::IntrinsicDistUniform,
            BuiltinFunction::IntrinsicDistLognormal,
            BuiltinFunction::IntrinsicDistExponential,
            BuiltinFunction::IntrinsicDistPoisson,
            BuiltinFunction::IntrinsicDistSampleN,
            // Stochastic (4)
            BuiltinFunction::IntrinsicBrownianMotion,
            BuiltinFunction::IntrinsicGbm,
            BuiltinFunction::IntrinsicOuProcess,
            BuiltinFunction::IntrinsicRandomWalk,
            // Rolling (7)
            BuiltinFunction::IntrinsicRollingSum,
            BuiltinFunction::IntrinsicRollingMean,
            BuiltinFunction::IntrinsicRollingStd,
            BuiltinFunction::IntrinsicRollingMin,
            BuiltinFunction::IntrinsicRollingMax,
            BuiltinFunction::IntrinsicEma,
            BuiltinFunction::IntrinsicLinearRecurrence,
            // Series transform (7)
            BuiltinFunction::IntrinsicShift,
            BuiltinFunction::IntrinsicDiff,
            BuiltinFunction::IntrinsicPctChange,
            BuiltinFunction::IntrinsicFillna,
            BuiltinFunction::IntrinsicCumsum,
            BuiltinFunction::IntrinsicCumprod,
            BuiltinFunction::IntrinsicClip,
            // Statistics (4)
            BuiltinFunction::IntrinsicCorrelation,
            BuiltinFunction::IntrinsicCovariance,
            BuiltinFunction::IntrinsicPercentile,
            BuiltinFunction::IntrinsicMedian,
            // Trigonometric (4)
            BuiltinFunction::IntrinsicAtan2,
            BuiltinFunction::IntrinsicSinh,
            BuiltinFunction::IntrinsicCosh,
            BuiltinFunction::IntrinsicTanh,
            // Char codes (2)
            BuiltinFunction::IntrinsicCharCode,
            BuiltinFunction::IntrinsicFromCharCode,
            // Series (1)
            BuiltinFunction::IntrinsicSeries,
            // Vector (11)
            BuiltinFunction::IntrinsicVecAbs,
            BuiltinFunction::IntrinsicVecSqrt,
            BuiltinFunction::IntrinsicVecLn,
            BuiltinFunction::IntrinsicVecExp,
            BuiltinFunction::IntrinsicVecAdd,
            BuiltinFunction::IntrinsicVecSub,
            BuiltinFunction::IntrinsicVecMul,
            BuiltinFunction::IntrinsicVecDiv,
            BuiltinFunction::IntrinsicVecMax,
            BuiltinFunction::IntrinsicVecMin,
            BuiltinFunction::IntrinsicVecSelect,
            // Matrix (2)
            BuiltinFunction::IntrinsicMatMulVec,
            BuiltinFunction::IntrinsicMatMulMat,
            // Eval helpers (6)
            BuiltinFunction::EvalTimeRef,
            BuiltinFunction::EvalDateTimeExpr,
            BuiltinFunction::EvalDataDateTimeRef,
            BuiltinFunction::EvalDataSet,
            BuiltinFunction::EvalDataRelative,
            BuiltinFunction::EvalDataRelativeRange,
            // Ctors (7)
            BuiltinFunction::SomeCtor,
            BuiltinFunction::OkCtor,
            BuiltinFunction::ErrCtor,
            BuiltinFunction::HashMapCtor,
            BuiltinFunction::SetCtor,
            BuiltinFunction::DequeCtor,
            BuiltinFunction::PriorityQueueCtor,
            // JSON (5)
            BuiltinFunction::JsonObjectGet,
            BuiltinFunction::JsonArrayAt,
            BuiltinFunction::JsonObjectKeys,
            BuiltinFunction::JsonArrayLen,
            BuiltinFunction::JsonObjectLen,
            // Window (14)
            BuiltinFunction::WindowRowNumber,
            BuiltinFunction::WindowRank,
            BuiltinFunction::WindowDenseRank,
            BuiltinFunction::WindowNtile,
            BuiltinFunction::WindowLag,
            BuiltinFunction::WindowLead,
            BuiltinFunction::WindowFirstValue,
            BuiltinFunction::WindowLastValue,
            BuiltinFunction::WindowNthValue,
            BuiltinFunction::WindowSum,
            BuiltinFunction::WindowAvg,
            BuiltinFunction::WindowMin,
            BuiltinFunction::WindowMax,
            BuiltinFunction::WindowCount,
            // Join/Reflect (2)
            BuiltinFunction::JoinExecute,
            BuiltinFunction::Reflect,
            // Content (3 + 6 constructors)
            BuiltinFunction::MakeContentText,
            BuiltinFunction::MakeContentFragment,
            BuiltinFunction::ApplyContentStyle,
            BuiltinFunction::MakeContentChartFromValue,
            BuiltinFunction::ContentChart,
            BuiltinFunction::ContentTextCtor,
            BuiltinFunction::ContentTableCtor,
            BuiltinFunction::ContentCodeCtor,
            BuiltinFunction::ContentKvCtor,
            BuiltinFunction::ContentFragmentCtor,
            // DateTime (6)
            BuiltinFunction::DateTimeNow,
            BuiltinFunction::DateTimeUtc,
            BuiltinFunction::DateTimeParse,
            BuiltinFunction::DateTimeFromEpoch,
            BuiltinFunction::DateTimeFromParts,
            BuiltinFunction::DateTimeFromUnixSecs,
            // Concurrency (4)
            BuiltinFunction::MutexCtor,
            BuiltinFunction::AtomicCtor,
            BuiltinFunction::LazyCtor,
            BuiltinFunction::ChannelCtor,
            // Math extras (7)
            BuiltinFunction::Sign,
            BuiltinFunction::Gcd,
            BuiltinFunction::Lcm,
            BuiltinFunction::Hypot,
            BuiltinFunction::Clamp,
            BuiltinFunction::IsNaN,
            BuiltinFunction::IsFinite,
            // Matrix (1)
            BuiltinFunction::MatFromFlat,
            // Table construction (1)
            BuiltinFunction::MakeTableFromRows,
        ];
        VARIANTS.get(id as usize).copied()
    }
}
