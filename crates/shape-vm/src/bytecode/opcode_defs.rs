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
        #[repr(u16)]
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

    // ===== Dynamic Arithmetic Operations (DELETED - strict-typing sweep Phase 2) =====
    // 0x10 (AddDynamic), 0x11 (SubDynamic), 0x12 (MulDynamic),
    // 0x13 (DivDynamic), 0x14 (ModDynamic), 0x16 (PowDynamic)
    // were deleted; the compiler now emits typed opcodes (AddInt/AddNumber/...)
    // exclusively, or fails with a strict-typing error.
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

    // ===== Dynamic Comparison Operations (DELETED - strict-typing sweep Phase 2) =====
    // 0x20 (GtDynamic), 0x21 (LtDynamic), 0x22 (GteDynamic),
    // 0x23 (LteDynamic), 0x24 (EqDynamic), 0x25 (NeqDynamic)
    // were deleted; the compiler now emits typed comparison opcodes
    // (GtInt/EqString/...) exclusively, or fails with a strict-typing error.

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
    /// Create a closure with captured upvalues.
    ///
    /// Operand encoding:
    /// - `Operand::Function(fid)`: non-escaping closure (stack-safe in JIT Phase E).
    /// - `Operand::ClosureAlloc { fid, escapes: true }`: escaping closure — always
    ///   heap-allocated via `TypedClosureHeader` (JIT Phase H2 path).
    /// - `Operand::ClosureAlloc { fid, escapes: false }`: non-escaping closure
    ///   (equivalent to `Function(fid)`; supported for uniform operand readers).
    ///
    /// Closure spec H5 merged the former `MakeClosureHeap` into this opcode.
    /// See `docs/v2-closure-specialization.md` §13 H5.
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

    /// Negate int (i64 → i64)
    NegInt = 0xCA, Arithmetic, pops: 1, pushes: 1;
    /// Negate number (f64 → f64)
    NegNumber = 0xCB, Arithmetic, pops: 1, pushes: 1;
    /// Negate decimal
    NegDecimal = 0xCC, Arithmetic, pops: 1, pushes: 1;

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
    // Slot 0xF2 reclaimed by Stage 2.6.5.0 (was: Debug breakpoint with no
    // compiler emission and only stale JIT classifier references). Reused
    // by IsNull in Stage 2.6.5.1.
    /// Stage 2.6.5: typed absence check. Pops one value, pushes a bool
    /// indicating whether the value is the None or Unit sentinel. Replaces
    /// the legacy `PushNull; Eq` and `emit_unit; Eq` patterns at the 16
    /// null/unit-check sites in the compiler.
    IsNull = 0xF2, Comparison, pops: 1, pushes: 1;

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
    /// Create a new TypedArray<bool> with given capacity. Operand: Count(capacity). Pushes ptr.
    NewTypedArrayBool = 0x0F, Object, pops: 0, pushes: 1;
    /// Get element from TypedArray<bool>: pops (arr_ptr, index), pushes bool value
    TypedArrayGetBool = 0x47, Object, pops: 2, pushes: 1;
    /// Push element to TypedArray<i32>: pops (arr_ptr, value), pushes nothing
    TypedArrayPushI32 = 0x48, Object, pops: 2, pushes: 0;
    /// Push element to TypedArray<bool>: pops (arr_ptr, value), pushes nothing
    TypedArrayPushBool = 0x49, Object, pops: 2, pushes: 0;
    /// Set element in TypedArray<i64>: pops (arr_ptr, index, value), pushes nothing
    TypedArraySetI64 = 0x4A, Object, pops: 3, pushes: 0;
    /// Set element in TypedArray<i32>: pops (arr_ptr, index, value), pushes nothing
    TypedArraySetI32 = 0x4B, Object, pops: 3, pushes: 0;
    /// Set element in TypedArray<bool>: pops (arr_ptr, index, value), pushes nothing
    TypedArraySetBool = 0x4C, Object, pops: 3, pushes: 0;

    // ===== v2 Typed Map Operations =====
    /// Allocate a new TypedMap<*const StringObj, f64>. Pushes ptr.
    NewTypedMapStringF64 = 0xCD, Object, pops: 0, pushes: 1;
    /// Allocate a new TypedMap<*const StringObj, i64>. Pushes ptr.
    NewTypedMapStringI64 = 0xCE, Object, pops: 0, pushes: 1;
    /// Allocate a new TypedMap<*const StringObj, *const u8>. Pushes ptr.
    NewTypedMapStringPtr = 0xCF, Object, pops: 0, pushes: 1;
    /// Allocate a new TypedMap<i64, f64>. Pushes ptr.
    NewTypedMapI64F64 = 0xD5, Object, pops: 0, pushes: 1;
    /// Allocate a new TypedMap<i64, i64>. Pushes ptr.
    NewTypedMapI64I64 = 0xD6, Object, pops: 0, pushes: 1;
    /// Allocate a new TypedMap<i64, *const u8>. Pushes ptr.
    NewTypedMapI64Ptr = 0xD9, Object, pops: 0, pushes: 1;
    /// String→f64 get: pops (map_ptr, key), pushes f64 (or null).
    TypedMapStringF64Get = 0xDA, Object, pops: 2, pushes: 1;
    /// String→i64 get: pops (map_ptr, key), pushes i64 (or null).
    TypedMapStringI64Get = 0xDB, Object, pops: 2, pushes: 1;
    /// String→Ptr get: pops (map_ptr, key), pushes ptr (or null).
    TypedMapStringPtrGet = 0xDC, Object, pops: 2, pushes: 1;
    /// I64→f64 get: pops (map_ptr, key), pushes f64 (or null).
    TypedMapI64F64Get = 0xDD, Object, pops: 2, pushes: 1;
    /// I64→i64 get: pops (map_ptr, key), pushes i64 (or null).
    TypedMapI64I64Get = 0xDE, Object, pops: 2, pushes: 1;
    /// I64→Ptr get: pops (map_ptr, key), pushes ptr (or null).
    TypedMapI64PtrGet = 0xDF, Object, pops: 2, pushes: 1;
    /// String→f64 set: pops (map_ptr, key, value).
    TypedMapStringF64Set = 0x4D, Object, pops: 3, pushes: 0;
    /// String→i64 set: pops (map_ptr, key, value).
    TypedMapStringI64Set = 0x4E, Object, pops: 3, pushes: 0;
    /// String→Ptr set: pops (map_ptr, key, value).
    TypedMapStringPtrSet = 0x4F, Object, pops: 3, pushes: 0;
    /// I64→f64 set: pops (map_ptr, key, value).
    TypedMapI64F64Set = 0x6D, Object, pops: 3, pushes: 0;
    /// I64→i64 set: pops (map_ptr, key, value).
    TypedMapI64I64Set = 0x6E, Object, pops: 3, pushes: 0;
    /// I64→Ptr set: pops (map_ptr, key, value).
    TypedMapI64PtrSet = 0x6F, Object, pops: 3, pushes: 0;
    /// String→f64 has: pops (map_ptr, key), pushes bool.
    TypedMapStringF64Has = 0x8E, Object, pops: 2, pushes: 1;
    /// String→i64 has: pops (map_ptr, key), pushes bool.
    TypedMapStringI64Has = 0x8F, Object, pops: 2, pushes: 1;
    /// String→Ptr has: pops (map_ptr, key), pushes bool.
    TypedMapStringPtrHas = 0xB9, Object, pops: 2, pushes: 1;
    /// I64→f64 has: pops (map_ptr, key), pushes bool.
    TypedMapI64F64Has = 0xBA, Object, pops: 2, pushes: 1;
    /// I64→i64 has: pops (map_ptr, key), pushes bool.
    TypedMapI64I64Has = 0xBB, Object, pops: 2, pushes: 1;
    /// I64→Ptr has: pops (map_ptr, key), pushes bool.
    TypedMapI64PtrHas = 0xBC, Object, pops: 2, pushes: 1;
    /// String→f64 delete: pops (map_ptr, key).
    TypedMapStringF64Delete = 0xBD, Object, pops: 2, pushes: 0;
    /// String→i64 delete: pops (map_ptr, key).
    TypedMapStringI64Delete = 0xBE, Object, pops: 2, pushes: 0;
    /// String→Ptr delete: pops (map_ptr, key).
    TypedMapStringPtrDelete = 0xBF, Object, pops: 2, pushes: 0;
    /// I64→f64 delete: pops (map_ptr, key).
    TypedMapI64F64Delete = 0xF9, Object, pops: 2, pushes: 0;
    /// I64→i64 delete: pops (map_ptr, key).
    TypedMapI64I64Delete = 0xFA, Object, pops: 2, pushes: 0;
    /// I64→Ptr delete: pops (map_ptr, key).
    TypedMapI64PtrDelete = 0xFB, Object, pops: 2, pushes: 0;

    // ===== v2 Concatenation Operations =====
    /// Concatenate two heap strings/chars, pushing a new string. Pops (a, b).
    StringConcat = 0xFC, Object, pops: 2, pushes: 1;
    /// Concatenate two arrays, pushing a new array. Pops (a, b).
    ArrayConcat = 0xFD, Object, pops: 2, pushes: 1;

    // ===== v2 Stage 2.6.3: Typed Equality for Heap Types =====
    /// Equal (string × string → bool). Pops two `*const StringObj`,
    /// content-compares the UTF-8 bytes, pushes bool. Both operands must be
    /// non-null v2 StringObj pointers. Use Neq via `EqString; Not`.
    EqString = 0xFE, Comparison, pops: 2, pushes: 1;
    /// Equal (decimal × decimal → bool). Pops two `*const DecimalObj`,
    /// content-compares the decimal payloads, pushes bool. Both operands
    /// must be non-null v2 DecimalObj pointers. Use Neq via `EqDecimal; Not`.
    EqDecimal = 0xFF, Comparison, pops: 2, pushes: 1;

    // ===== v2 Stage 4.2: Typed Ordered Comparison for Strings =====
    /// Greater than (string × string → bool). Lexicographic comparison.
    GtString = 0x100, Comparison, pops: 2, pushes: 1;
    /// Less than (string × string → bool). Lexicographic comparison.
    LtString = 0x101, Comparison, pops: 2, pushes: 1;
    /// Greater than or equal (string × string → bool). Lexicographic comparison.
    GteString = 0x102, Comparison, pops: 2, pushes: 1;
    /// Less than or equal (string × string → bool). Lexicographic comparison.
    LteString = 0x103, Comparison, pops: 2, pushes: 1;

    // ===== Ownership-Aware Variable Operations =====
    /// Load local with Move semantics — transfers ownership, source slot is zeroed.
    /// Used when the compiler proves the source binding is dead after this point.
    LoadLocalMove = 0x104, Variable, pops: 0, pushes: 1;
    /// Load local with Clone semantics — clones the value, source stays live.
    /// For heap-tagged values, this bumps the Arc refcount.
    LoadLocalClone = 0x105, Variable, pops: 0, pushes: 1;
    /// Store local with Drop semantics — drops the old value before storing.
    /// Respects ownership: if old value is uniquely owned, frees immediately.
    StoreLocalDrop = 0x106, Variable, pops: 1, pushes: 0;
    /// Promote top-of-stack from shared (Arc) to owned (Box) allocation if
    /// the refcount is 1.  No-op for inline values or already-owned values.
    /// Used by the compiler before StoreLocal for uniquely-owned let bindings.
    PromoteToOwned = 0x107, Stack, pops: 0, pushes: 0;

    // ===== Typed Array Element Access (local-slot based, skip HeapValue dispatch) =====
    /// Get i64 element from typed int array. Operand: local slot. Index on stack.
    GetElemI64 = 0x108, Object, pops: 1, pushes: 1;
    /// Get f64 element from typed float array. Operand: local slot. Index on stack.
    GetElemF64 = 0x109, Object, pops: 1, pushes: 1;
    /// Set i64 element in typed int array. Operand: local slot. Index and value on stack.
    SetElemI64 = 0x10A, Object, pops: 2, pushes: 0;
    /// Set f64 element in typed float array. Operand: local slot. Index and value on stack.
    SetElemF64 = 0x10B, Object, pops: 2, pushes: 0;
    /// Push i64 to typed int array. Operand: local slot. Value on stack.
    ArrayPushI64 = 0x10C, Object, pops: 1, pushes: 0;
    /// Push f64 to typed float array. Operand: local slot. Value on stack.
    ArrayPushF64 = 0x10D, Object, pops: 1, pushes: 0;
    /// Get length of typed array (any element type). Operand: local slot.
    ArrayLenTyped = 0x10E, Object, pops: 0, pushes: 1;

    // ===== Typed HashMap Access (local-slot based) =====
    /// Get value from HashMap<string, int>. Key on stack. Operand: map slot.
    MapGetStrI64 = 0x10F, Object, pops: 1, pushes: 1;
    /// Get value from HashMap<string, float>. Key on stack. Operand: map slot.
    MapGetStrF64 = 0x110, Object, pops: 1, pushes: 1;
    /// Set value in HashMap<string, int>. Key and value on stack. Operand: map slot.
    MapSetStrI64 = 0x111, Object, pops: 2, pushes: 0;
    /// Check if key exists in HashMap<string, *>. Key on stack. Operand: map slot.
    MapHasStr = 0x112, Object, pops: 1, pushes: 1;
    /// Get HashMap length. Operand: map slot.
    MapLenTyped = 0x113, Object, pops: 0, pushes: 1;

    // ===== Typed String Access (local-slot based) =====
    /// Get string length (chars). Operand: string slot.
    StringLenTyped = 0x114, Object, pops: 0, pushes: 1;
    /// Get char at index. Index on stack. Operand: string slot.
    StringCharAt = 0x115, Object, pops: 1, pushes: 1;
    /// Concatenate two strings. Both on stack.
    StringConcatTyped = 0x116, Object, pops: 2, pushes: 1;

    /// Phase 5.C: Return with owned semantics. Pops the top-of-stack return
    /// value, promotes Arc→Box when refcount is exactly 1 (as `PromoteToOwned`
    /// does), then falls through to the normal return path. Emitted by the
    /// compiler in place of the implicit return-slot store for functions
    /// whose inferred `ReturnOwnershipMode` is `NewlyOwned`, so the callee
    /// already hands a uniquely-owned value to the caller and the caller
    /// can skip its own `PromoteToOwned`.
    ///
    /// Stack effect is identical to `PromoteToOwned` — it operates on the
    /// value already on the stack and leaves it in place; the control flow
    /// is handled by the subsequent `Return` instruction or by the function
    /// epilogue, not by this opcode itself.
    ReturnOwned = 0x117, Stack, pops: 0, pushes: 0;

    // NOTE: Byte range 0x118..=0x121 was formerly occupied by the
    // Closure Spec Phase D typed mutable-capture opcodes
    // (`LoadCaptureMutPtr<T>` / `StoreCaptureMutPtr<T>` for
    // F64/I64/I32/Bool/Ptr). Track A.1C.3 retired them in favour of the
    // A.1B dynamic-ValueWord path (`LoadOwnedMutableCapture` /
    // `StoreOwnedMutableCapture`, `LoadSharedCapture` /
    // `StoreSharedCapture`) which handles every let-mut / var capture
    // universally. These byte values are intentionally left unassigned
    // to keep the A.1B opcodes below at their original values.

    // ===== Track A.1B: CaptureKind::OwnedMutable / CaptureKind::Shared =====
    //
    // These opcodes implement Track A's three-way CaptureKind split (see
    // `crates/shape-value/src/v2/closure_layout.rs` — `CaptureKind`):
    //
    // - `OwnedMutable`: `let mut` by-move captures. The closure's capture
    //   slot holds `*mut ValueWord` from `Box::into_raw(Box::new(initial))`.
    //   Exactly one closure owns the box; no sharing, no lock. Released by
    //   `release_typed_closure` via `Box::from_raw`.
    // - `Shared`: `var` captures shared across nested closures. The slot
    //   holds `*const SharedCell` from
    //   `Arc::into_raw(Arc::new(parking_lot::Mutex::new(initial)))`. Each
    //   reader/writer acquires the parking_lot mutex; refcount released by
    //   `Arc::from_raw` on closure Drop.
    //
    // A.1B's interpreter binds the raw pointer bits into
    // `frame.upvalues[i]` (bypassing `Upvalue::get`/`set`'s SharedCell
    // auto-deref — those are for the retired-in-A.1C legacy variant). The
    // new opcodes below read the raw bits directly and dereference the
    // pointer. Operand width: `Local(u16)` like every other capture op.
    //
    // SAFETY invariants (enforced per-opcode via `ClosureLayout`
    // `owned_mutable_capture_mask` / `shared_capture_mask` at compile
    // time):
    //   * `LoadOwnedMutableCapture{i}` / `StoreOwnedMutableCapture{i}` are
    //     only emitted when the current function's capture `i` has
    //     `CaptureKind::OwnedMutable` in its layout. The upvalue at index
    //     `i` MUST contain raw `*mut ValueWord` bits (see A.1B
    //     `op_make_closure` allocation path + A.1B
    //     `call_closure_with_nb_args` upvalue plumbing).
    //   * Likewise for `LoadSharedCapture` / `StoreSharedCapture` — the
    //     upvalue holds `*const SharedCell` bits and reads/writes take
    //     the parking_lot mutex.
    //
    // A.1D and A.1E add Cranelift lowerings; until then the JIT bails to
    // the interpreter for any function that contains these opcodes.
    //
    // See `docs/v2-closure-specialization.md` §14.7 for the landed Track A
    // plan.

    /// Load a ValueWord through an `OwnedMutable` capture's `*mut ValueWord`
    /// cell. Operand: Local(idx). Pushes the dereferenced value.
    LoadOwnedMutableCapture = 0x132, Variable, pops: 0, pushes: 1;
    /// Store a ValueWord through an `OwnedMutable` capture's `*mut ValueWord`
    /// cell. Operand: Local(idx). Pops the value to write.
    StoreOwnedMutableCapture = 0x133, Variable, pops: 1, pushes: 0;
    /// Load a ValueWord through a `Shared` capture's
    /// `*const parking_lot::Mutex<ValueWord>` cell — takes the mutex for
    /// the read only. Operand: Local(idx). Pushes the inner value.
    LoadSharedCapture = 0x134, Variable, pops: 0, pushes: 1;
    /// Store a ValueWord through a `Shared` capture's
    /// `*const parking_lot::Mutex<ValueWord>` cell — takes the mutex for
    /// the write only. Operand: Local(idx). Pops the value to write.
    StoreSharedCapture = 0x135, Variable, pops: 1, pushes: 0;

    // ===== Phase 3c Wave D.1: per-FieldKind OwnedMutable capture opcodes =====
    //
    // Typed counterparts of the legacy single-form `LoadOwnedMutableCapture`
    // (0x132) / `StoreOwnedMutableCapture` (0x133), which both treat the
    // closure cell as a `*mut ValueWord` (8-byte tagged word).
    //
    // The Wave B storage migration replaced the `Box<ValueWord>` cell with
    // per-FieldKind `Box<T>` cells (see
    // `shape_value::v2::closure_raw::alloc_owned_mutable_<kind>`). Each
    // typed cell holds a native scalar (`i64`, `f64`, `i8`, `bool`,
    // ValueWord-bits, ...) without any tag overhead. The 22 opcodes below
    // are the typed load/store path for those cells, one pair per
    // FieldKind, in the canonical order:
    //
    //   I64, U64, F64, I32, U32, I16, U16, I8, U8, Bool, Ptr
    //
    // Operand layout: `Local(u16)` — same capture-array index as the legacy
    // opcodes. The capture's `inner_kind` (recorded in
    // `ClosureLayout::capture_inner_kinds`) selects the typed opcode the
    // compiler emits at this site (Wave E).
    //
    // SAFETY invariants — enforced per-opcode at compile time via the
    // closure layout's `capture_inner_kind(i)` selector:
    //   * `Load/StoreOwnedMutableCapture<Kind>{i}` is emitted only when
    //     the current function's capture `i` has
    //     `CaptureKind::OwnedMutable` AND `inner_kind == FieldKind::<Kind>`.
    //     The upvalue at index `i` MUST contain raw `*mut <T>` bits matching
    //     `<Kind>` (see Wave B's `alloc_owned_mutable_<kind>` allocator and
    //     the per-kind init path in `op_make_closure`).
    //   * The legacy `LoadOwnedMutableCapture` / `StoreOwnedMutableCapture`
    //     (0x132/0x133) remain live and emit-compatible: any function the
    //     compiler has not yet migrated to the typed path keeps using the
    //     dynamic ValueWord cell + ValueWord opcodes. Wave G removes the
    //     legacy opcodes once Wave E flips every emit site.
    //
    // Stack effect: Load reads the typed cell and pushes a raw native value
    // onto the stack via the matching `push_raw_<kind>` / `push_raw_u64`
    // helper (sub-i64 ints sign- or zero-extended into the i64 path,
    // matching the existing typed-opcode convention in `arithmetic/`).
    // Store pops a native value via the matching `pop_raw_<kind>` /
    // `pop_raw_u64` helper, truncates as needed for sub-i64 widths, and
    // writes through the typed cell.

    /// Load `i64` through an `OwnedMutable` capture's `*mut i64` cell.
    /// Operand: Local(idx). Pushes the dereferenced i64 onto the stack as
    /// a raw i64.
    LoadOwnedMutableCaptureI64 = 0x140, Variable, pops: 0, pushes: 1;
    /// Load `u64` through an `OwnedMutable` capture's `*mut u64` cell.
    /// Operand: Local(idx). Pushes the dereferenced u64 bits onto the
    /// stack as raw u64.
    LoadOwnedMutableCaptureU64 = 0x141, Variable, pops: 0, pushes: 1;
    /// Load `f64` through an `OwnedMutable` capture's `*mut f64` cell.
    /// Operand: Local(idx). Pushes the dereferenced f64 onto the stack as
    /// a raw f64.
    LoadOwnedMutableCaptureF64 = 0x142, Variable, pops: 0, pushes: 1;
    /// Load `i32` through an `OwnedMutable` capture's `*mut i32` cell.
    /// Operand: Local(idx). Sign-extends the i32 to i64 and pushes it as
    /// a raw i64 (sub-i64 ints share the i64 stack convention).
    LoadOwnedMutableCaptureI32 = 0x143, Variable, pops: 0, pushes: 1;
    /// Load `u32` through an `OwnedMutable` capture's `*mut u32` cell.
    /// Operand: Local(idx). Zero-extends the u32 to i64 and pushes it as
    /// a raw i64.
    LoadOwnedMutableCaptureU32 = 0x144, Variable, pops: 0, pushes: 1;
    /// Load `i16` through an `OwnedMutable` capture's `*mut i16` cell.
    /// Operand: Local(idx). Sign-extends the i16 to i64 and pushes it as
    /// a raw i64.
    LoadOwnedMutableCaptureI16 = 0x145, Variable, pops: 0, pushes: 1;
    /// Load `u16` through an `OwnedMutable` capture's `*mut u16` cell.
    /// Operand: Local(idx). Zero-extends the u16 to i64 and pushes it as
    /// a raw i64.
    LoadOwnedMutableCaptureU16 = 0x146, Variable, pops: 0, pushes: 1;
    /// Load `i8` through an `OwnedMutable` capture's `*mut i8` cell.
    /// Operand: Local(idx). Sign-extends the i8 to i64 and pushes it as
    /// a raw i64.
    LoadOwnedMutableCaptureI8 = 0x147, Variable, pops: 0, pushes: 1;
    /// Load `u8` through an `OwnedMutable` capture's `*mut u8` cell.
    /// Operand: Local(idx). Zero-extends the u8 to i64 and pushes it as
    /// a raw i64.
    LoadOwnedMutableCaptureU8 = 0x148, Variable, pops: 0, pushes: 1;
    /// Load `bool` through an `OwnedMutable` capture's `*mut bool` cell.
    /// Operand: Local(idx). Pushes the dereferenced bool onto the stack
    /// via the typed bool-push helper.
    LoadOwnedMutableCaptureBool = 0x149, Variable, pops: 0, pushes: 1;
    /// Load `Ptr` through an `OwnedMutable` capture's `*mut u64` cell.
    /// Operand: Local(idx). Pushes the dereferenced 8-byte pointer-shaped
    /// payload as raw u64 (a ValueWord bit pattern carrying a NaN-boxed
    /// heap share or owned heap pointer). Refcount retain semantics for
    /// `Ptr` payloads are the caller's responsibility — matches the
    /// `read_owned_mutable_ptr` contract: this opcode does NOT clone.
    LoadOwnedMutableCapturePtr = 0x14A, Variable, pops: 0, pushes: 1;

    /// Store `i64` through an `OwnedMutable` capture's `*mut i64` cell.
    /// Operand: Local(idx). Pops a raw i64 and writes it into the cell.
    StoreOwnedMutableCaptureI64 = 0x14B, Variable, pops: 1, pushes: 0;
    /// Store `u64` through an `OwnedMutable` capture's `*mut u64` cell.
    /// Operand: Local(idx). Pops a raw u64 and writes it into the cell.
    StoreOwnedMutableCaptureU64 = 0x14C, Variable, pops: 1, pushes: 0;
    /// Store `f64` through an `OwnedMutable` capture's `*mut f64` cell.
    /// Operand: Local(idx). Pops a raw f64 and writes it into the cell.
    StoreOwnedMutableCaptureF64 = 0x14D, Variable, pops: 1, pushes: 0;
    /// Store `i32` through an `OwnedMutable` capture's `*mut i32` cell.
    /// Operand: Local(idx). Pops a raw i64 from the stack, truncates to
    /// the low 32 bits, and writes the i32 payload.
    StoreOwnedMutableCaptureI32 = 0x14E, Variable, pops: 1, pushes: 0;
    /// Store `u32` through an `OwnedMutable` capture's `*mut u32` cell.
    /// Operand: Local(idx). Pops a raw i64, truncates to the low 32 bits,
    /// and writes the u32 payload.
    StoreOwnedMutableCaptureU32 = 0x14F, Variable, pops: 1, pushes: 0;
    /// Store `i16` through an `OwnedMutable` capture's `*mut i16` cell.
    /// Operand: Local(idx). Pops a raw i64, truncates to the low 16 bits,
    /// and writes the i16 payload.
    StoreOwnedMutableCaptureI16 = 0x150, Variable, pops: 1, pushes: 0;
    /// Store `u16` through an `OwnedMutable` capture's `*mut u16` cell.
    /// Operand: Local(idx). Pops a raw i64, truncates to the low 16 bits,
    /// and writes the u16 payload.
    StoreOwnedMutableCaptureU16 = 0x151, Variable, pops: 1, pushes: 0;
    /// Store `i8` through an `OwnedMutable` capture's `*mut i8` cell.
    /// Operand: Local(idx). Pops a raw i64, truncates to the low 8 bits,
    /// and writes the i8 payload.
    StoreOwnedMutableCaptureI8 = 0x152, Variable, pops: 1, pushes: 0;
    /// Store `u8` through an `OwnedMutable` capture's `*mut u8` cell.
    /// Operand: Local(idx). Pops a raw i64, truncates to the low 8 bits,
    /// and writes the u8 payload.
    StoreOwnedMutableCaptureU8 = 0x153, Variable, pops: 1, pushes: 0;
    /// Store `bool` through an `OwnedMutable` capture's `*mut bool` cell.
    /// Operand: Local(idx). Pops a bool via the typed bool-pop helper and
    /// writes it into the cell.
    StoreOwnedMutableCaptureBool = 0x154, Variable, pops: 1, pushes: 0;
    /// Store `Ptr` through an `OwnedMutable` capture's `*mut u64` cell.
    /// Operand: Local(idx). Pops a raw u64 (a ValueWord bit pattern
    /// carrying a NaN-boxed heap share or owned heap pointer) and writes
    /// it into the cell. Refcount semantics are the caller's
    /// responsibility — matches the `write_owned_mutable_ptr` contract:
    /// this opcode does NOT release the previous payload nor retain the
    /// new one.
    StoreOwnedMutableCapturePtr = 0x155, Variable, pops: 1, pushes: 0;

    // ===== Track D.2: per-FieldKind typed Shared capture opcodes =====
    //
    // These are the typed counterparts of the legacy
    // `LoadSharedCapture` / `StoreSharedCapture` (0x134 / 0x135). For each
    // payload `FieldKind` (I64, U64, F64, I32, U32, I16, U16, I8, U8,
    // Bool, Ptr) we have a Load/Store pair that:
    //
    // * recovers the `*const SharedCell` pointer bits from the capture
    //   slot via `read_capture_raw_pointer_bits(idx)`,
    // * delegates to the lock-gated `read_shared_<kind>` /
    //   `write_shared_<kind>` helper in
    //   `shape_value::v2::closure_raw`. The helper acquires the
    //   `parking_lot::Mutex` internally, performs the typed access, and
    //   releases the lock before returning. The handler MUST NOT take
    //   the lock externally — that would double-lock.
    //
    // Stack effect mirrors D.1 (typed OwnedMutable opcodes 0x140-0x155):
    // Load reads from the lock-gated cell and pushes a raw native value
    // onto the stack via the matching `push_raw_<kind>`/`push_raw_u64`
    // helper. Store pops a native value via the matching
    // `pop_raw_<kind>`/`pop_raw_u64` helper, then writes it through the
    // lock-gated helper.
    //
    // SAFETY invariants — enforced by the compiler (Wave E codegen):
    //   * `LoadSharedCapture<Kind>` / `StoreSharedCapture<Kind>` are only
    //     emitted when the current function's capture `i` has
    //     `CaptureKind::Shared` and a payload `FieldKind` matching
    //     `<Kind>` in its layout.
    //   * The upvalue slot at index `i` MUST hold raw `*const SharedCell`
    //     bits produced by `Arc::into_raw(Arc::new(SharedCell::new(...)))`.
    //   * The cell's interior `FieldKind` must equal `<Kind>` — the
    //     helper writes the bit pattern matching the declared kind, and
    //     a mismatched reader will reinterpret bytes incorrectly.
    //
    // Opcode codes 0x156..=0x16B (22 codes total). Ordering matches D.1:
    // I64, U64, F64, I32, U32, I16, U16, I8, U8, Bool, Ptr — Load then
    // Store paired (LoadI64 = 0x156, StoreI64 = 0x161, LoadU64 = 0x157,
    // StoreU64 = 0x162, ...). We keep the kinds contiguous so the
    // dispatch table reads as two parallel ranges.

    /// Load an `i64` through a `Shared` capture cell — locks the mutex,
    /// reads the i64 payload, unlocks, pushes the raw i64 onto the stack.
    /// Operand: Local(idx).
    LoadSharedCaptureI64 = 0x156, Variable, pops: 0, pushes: 1;
    /// Load a `u64` through a `Shared` capture cell — locks, reads the
    /// u64 payload, unlocks, pushes the raw u64 bits.
    /// Operand: Local(idx).
    LoadSharedCaptureU64 = 0x157, Variable, pops: 0, pushes: 1;
    /// Load an `f64` through a `Shared` capture cell — locks, reads the
    /// f64 payload, unlocks, pushes the raw f64.
    /// Operand: Local(idx).
    LoadSharedCaptureF64 = 0x158, Variable, pops: 0, pushes: 1;
    /// Load an `i32` through a `Shared` capture cell — locks, reads the
    /// low 4 bytes of the payload as i32, unlocks, sign-extends, pushes.
    /// Operand: Local(idx).
    LoadSharedCaptureI32 = 0x159, Variable, pops: 0, pushes: 1;
    /// Load a `u32` through a `Shared` capture cell — locks, reads the
    /// low 4 bytes of the payload as u32, unlocks, zero-extends, pushes.
    /// Operand: Local(idx).
    LoadSharedCaptureU32 = 0x15A, Variable, pops: 0, pushes: 1;
    /// Load an `i16` through a `Shared` capture cell — locks, reads the
    /// low 2 bytes of the payload as i16, unlocks, sign-extends, pushes.
    /// Operand: Local(idx).
    LoadSharedCaptureI16 = 0x15B, Variable, pops: 0, pushes: 1;
    /// Load a `u16` through a `Shared` capture cell — locks, reads the
    /// low 2 bytes of the payload as u16, unlocks, zero-extends, pushes.
    /// Operand: Local(idx).
    LoadSharedCaptureU16 = 0x15C, Variable, pops: 0, pushes: 1;
    /// Load an `i8` through a `Shared` capture cell — locks, reads the
    /// low byte of the payload as i8, unlocks, sign-extends, pushes.
    /// Operand: Local(idx).
    LoadSharedCaptureI8 = 0x15D, Variable, pops: 0, pushes: 1;
    /// Load a `u8` through a `Shared` capture cell — locks, reads the
    /// low byte of the payload as u8, unlocks, zero-extends, pushes.
    /// Operand: Local(idx).
    LoadSharedCaptureU8 = 0x15E, Variable, pops: 0, pushes: 1;
    /// Load a `bool` through a `Shared` capture cell — locks, reads the
    /// low byte of the payload (zero ⇒ false; non-zero ⇒ true), unlocks,
    /// pushes a raw NaN-tagged bool onto the stack.
    /// Operand: Local(idx).
    LoadSharedCaptureBool = 0x15F, Variable, pops: 0, pushes: 1;
    /// Load a `Ptr` through a `Shared` capture cell — locks, reads the 8
    /// payload bytes as a raw u64 (a ValueWord bit pattern carrying a
    /// NaN-boxed Arc/Box pointer), unlocks, pushes the raw bits. Refcount
    /// retain semantics for `Ptr` payloads are the caller's
    /// responsibility (the helper does NOT clone — match the
    /// `read_shared_ptr` contract).
    /// Operand: Local(idx).
    LoadSharedCapturePtr = 0x160, Variable, pops: 0, pushes: 1;

    /// Store an `i64` through a `Shared` capture cell — pops a raw i64
    /// from the stack, locks, writes the 8-byte i64 payload, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureI64 = 0x161, Variable, pops: 1, pushes: 0;
    /// Store a `u64` through a `Shared` capture cell — pops a raw u64,
    /// locks, writes the 8-byte u64 payload, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureU64 = 0x162, Variable, pops: 1, pushes: 0;
    /// Store an `f64` through a `Shared` capture cell — pops a raw f64,
    /// locks, writes the 8-byte f64 payload, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureF64 = 0x163, Variable, pops: 1, pushes: 0;
    /// Store an `i32` through a `Shared` capture cell — pops a raw i32,
    /// sign-extends to 8 bytes, locks, writes payload, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureI32 = 0x164, Variable, pops: 1, pushes: 0;
    /// Store a `u32` through a `Shared` capture cell — pops a raw u32,
    /// zero-extends to 8 bytes, locks, writes payload, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureU32 = 0x165, Variable, pops: 1, pushes: 0;
    /// Store an `i16` through a `Shared` capture cell — pops a raw i16,
    /// sign-extends to 8 bytes, locks, writes payload, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureI16 = 0x166, Variable, pops: 1, pushes: 0;
    /// Store a `u16` through a `Shared` capture cell — pops a raw u16,
    /// zero-extends to 8 bytes, locks, writes payload, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureU16 = 0x167, Variable, pops: 1, pushes: 0;
    /// Store an `i8` through a `Shared` capture cell — pops a raw i8,
    /// sign-extends to 8 bytes, locks, writes payload, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureI8 = 0x168, Variable, pops: 1, pushes: 0;
    /// Store a `u8` through a `Shared` capture cell — pops a raw u8,
    /// zero-extends to 8 bytes, locks, writes payload, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureU8 = 0x169, Variable, pops: 1, pushes: 0;
    /// Store a `bool` through a `Shared` capture cell — pops a raw bool,
    /// locks, writes the 8-byte payload as 0 or 1, unlocks.
    /// Operand: Local(idx).
    StoreSharedCaptureBool = 0x16A, Variable, pops: 1, pushes: 0;
    /// Store a `Ptr` through a `Shared` capture cell — pops raw 8-byte
    /// bits (a ValueWord), locks, writes the payload, unlocks. The
    /// caller is responsible for refcount semantics on Ptr payloads —
    /// matches the `write_shared_ptr` contract: this opcode does NOT
    /// release the previous payload nor retain the new one.
    /// Operand: Local(idx).
    StoreSharedCapturePtr = 0x16B, Variable, pops: 1, pushes: 0;

    // ===== Wave E+3: per-FieldKind typed local load/store opcodes =====
    //
    // These are the typed counterparts of the legacy `LoadLocal` (0x50) /
    // `StoreLocal` (0x51). For each `FieldKind` (I64, U64, F64, I32, U32,
    // I16, U16, I8, U8, Bool, Ptr) we have a Load/Store pair that:
    //
    // * reads / writes the local slot at `bp + idx` directly as raw 8-byte
    //   bits, with no NaN-box tag check, no ValueWord wrapping, no
    //   `clone_from_bits`, no SharedCell auto-deref.
    // * skips refcount management even for the `Ptr` kind — refcount
    //   semantics are the IR's responsibility (matches D.1 / D.2 Ptr
    //   contract; the c-stdlib-msgpack pattern from commit afb1651 is the
    //   precedent).
    //
    // SAFETY invariants — enforced by the compiler (Wave E+ codegen):
    //   * The emitter only fires `LoadLocal<Kind>` / `StoreLocal<Kind>` on
    //     a slot whose proven SlotKind matches `<Kind>`. The slot's bits
    //     were last written by a matching-Kind `StoreLocal<Kind>` (or a
    //     producer that emitted matching native bits), so a raw read
    //     reinterprets the correct bit pattern.
    //   * Sub-i64 kinds (I32/U32/I16/U16/I8/U8/Bool) carry the value in
    //     the low N bits of the 8-byte slot; the upper bits are
    //     unspecified. Producers must zero/sign-extend appropriately
    //     (matches D.1 store-side truncation convention).
    //   * For `Ptr` slots, neither Load nor Store performs `vw_clone` /
    //     `vw_drop`. The IR pairs each typed Ptr load/store with the
    //     matching retain/release before/after.
    //
    // Stack effect mirrors D.1 (typed OwnedMutable opcodes 0x140-0x155):
    // Load reads from the local slot and pushes a raw native value onto
    // the stack via `push_raw_u64`. Store pops a native value via
    // `pop_raw_u64`, then writes the raw 8-byte bits to the slot.
    //
    // The legacy `LoadLocal` (0x50) / `StoreLocal` (0x51) stay live for
    // unproven-type positions; the typed forms are dead until Wave E+4
    // flips the emitter.
    //
    // Code range: 0x16C..=0x181 (22 codes total). Ordering: I64, U64, F64,
    // I32, U32, I16, U16, I8, U8, Bool, Ptr — Loads first (0x16C..=0x176),
    // Stores second (0x177..=0x181).

    /// Load `i64` from local slot — reads raw 8 bytes, pushes as i64.
    /// Operand: Local(idx).
    LoadLocalI64 = 0x16C, Variable, pops: 0, pushes: 1;
    /// Load `u64` from local slot — reads raw 8 bytes, pushes as u64.
    /// Operand: Local(idx).
    LoadLocalU64 = 0x16D, Variable, pops: 0, pushes: 1;
    /// Load `f64` from local slot — reads raw 8 bytes, pushes as f64.
    /// Operand: Local(idx).
    LoadLocalF64 = 0x16E, Variable, pops: 0, pushes: 1;
    /// Load `i32` from local slot — reads low 4 bytes, sign-extends to
    /// i64 in the 8-byte stack slot. Operand: Local(idx).
    LoadLocalI32 = 0x16F, Variable, pops: 0, pushes: 1;
    /// Load `u32` from local slot — reads low 4 bytes, zero-extends to
    /// u64 in the 8-byte stack slot. Operand: Local(idx).
    LoadLocalU32 = 0x170, Variable, pops: 0, pushes: 1;
    /// Load `i16` from local slot — reads low 2 bytes, sign-extends to
    /// i64 in the 8-byte stack slot. Operand: Local(idx).
    LoadLocalI16 = 0x171, Variable, pops: 0, pushes: 1;
    /// Load `u16` from local slot — reads low 2 bytes, zero-extends to
    /// u64 in the 8-byte stack slot. Operand: Local(idx).
    LoadLocalU16 = 0x172, Variable, pops: 0, pushes: 1;
    /// Load `i8` from local slot — reads low byte, sign-extends to i64
    /// in the 8-byte stack slot. Operand: Local(idx).
    LoadLocalI8 = 0x173, Variable, pops: 0, pushes: 1;
    /// Load `u8` from local slot — reads low byte, zero-extends to u64
    /// in the 8-byte stack slot. Operand: Local(idx).
    LoadLocalU8 = 0x174, Variable, pops: 0, pushes: 1;
    /// Load `bool` from local slot — reads low byte (zero ⇒ false;
    /// non-zero ⇒ true) and pushes the raw 8 bytes back. Operand: Local(idx).
    LoadLocalBool = 0x175, Variable, pops: 0, pushes: 1;
    /// Load `Ptr` from local slot — reads raw 8 bytes (a ValueWord bit
    /// pattern carrying a NaN-boxed Arc/Box pointer) and pushes them.
    /// The handler does NOT clone/retain — refcount semantics are the
    /// caller's responsibility. Operand: Local(idx).
    LoadLocalPtr = 0x176, Variable, pops: 0, pushes: 1;

    /// Store `i64` to local slot — pops raw i64, writes 8 bytes to slot.
    /// Operand: Local(idx).
    StoreLocalI64 = 0x177, Variable, pops: 1, pushes: 0;
    /// Store `u64` to local slot — pops raw u64, writes 8 bytes to slot.
    /// Operand: Local(idx).
    StoreLocalU64 = 0x178, Variable, pops: 1, pushes: 0;
    /// Store `f64` to local slot — pops raw f64, writes 8 bytes to slot.
    /// Operand: Local(idx).
    StoreLocalF64 = 0x179, Variable, pops: 1, pushes: 0;
    /// Store `i32` to local slot — pops 8-byte slot, truncates to i32
    /// (low 4 bytes, sign-extended back into 8-byte slot for storage).
    /// Operand: Local(idx).
    StoreLocalI32 = 0x17A, Variable, pops: 1, pushes: 0;
    /// Store `u32` to local slot — pops 8-byte slot, truncates to u32
    /// (low 4 bytes, zero-extended back into 8-byte slot for storage).
    /// Operand: Local(idx).
    StoreLocalU32 = 0x17B, Variable, pops: 1, pushes: 0;
    /// Store `i16` to local slot — pops 8-byte slot, truncates to i16
    /// (low 2 bytes, sign-extended back into 8-byte slot for storage).
    /// Operand: Local(idx).
    StoreLocalI16 = 0x17C, Variable, pops: 1, pushes: 0;
    /// Store `u16` to local slot — pops 8-byte slot, truncates to u16
    /// (low 2 bytes, zero-extended back into 8-byte slot for storage).
    /// Operand: Local(idx).
    StoreLocalU16 = 0x17D, Variable, pops: 1, pushes: 0;
    /// Store `i8` to local slot — pops 8-byte slot, truncates to i8
    /// (low byte, sign-extended back into 8-byte slot for storage).
    /// Operand: Local(idx).
    StoreLocalI8 = 0x17E, Variable, pops: 1, pushes: 0;
    /// Store `u8` to local slot — pops 8-byte slot, truncates to u8
    /// (low byte, zero-extended back into 8-byte slot for storage).
    /// Operand: Local(idx).
    StoreLocalU8 = 0x17F, Variable, pops: 1, pushes: 0;
    /// Store `bool` to local slot — pops raw 8-byte slot, writes a
    /// canonical 0 or 1 in the slot's low byte (any nonzero pop ⇒ 1).
    /// Operand: Local(idx).
    StoreLocalBool = 0x180, Variable, pops: 1, pushes: 0;
    /// Store `Ptr` to local slot — pops raw 8 bytes (a ValueWord bit
    /// pattern carrying a NaN-boxed Arc/Box pointer) and writes them
    /// to the slot. The handler does NOT release the previous payload
    /// nor retain the new one — refcount semantics are the caller's
    /// responsibility (matches the D.1 / D.2 Ptr contract).
    /// Operand: Local(idx).
    StoreLocalPtr = 0x181, Variable, pops: 1, pushes: 0;

    // ===== Wave E+3: per-FieldKind typed `ReturnValue<Kind>` opcodes =====
    //
    // Typed counterparts of the legacy `ReturnValue` (0x45). The handler
    // body is identical to `op_return_value` — pops the return value as
    // raw 8-byte bits, pops the call frame, releases the callee's
    // register window, then pushes the return value onto the caller's
    // stack. The encoded `<Kind>` carries no runtime difference; it is
    // a *static* annotation for the JIT and downstream consumers so the
    // caller's stack discipline is known at the call site.
    //
    // Stack effect: pops 1 (the return value of the matching native
    // kind), pushes 1 onto the caller's frame after frame cleanup. The
    // legacy `ReturnValue` (0x45) stays live for unproven-type return
    // positions.
    //
    // Code range: 0x198..=0x1A2 (11 codes total). Ordering matches the
    // FieldKind canonical ordering used elsewhere in this file (D.1 /
    // D.2): I64, U64, F64, I32, U32, I16, U16, I8, U8, Bool, Ptr.
    /// Return with `i64` value — pops 1 raw i64, frame-cleans, pushes 1.
    ReturnValueI64 = 0x198, Control, pops: 1, pushes: 0;
    /// Return with `u64` value — pops 1 raw u64, frame-cleans, pushes 1.
    ReturnValueU64 = 0x199, Control, pops: 1, pushes: 0;
    /// Return with `f64` value — pops 1 raw f64, frame-cleans, pushes 1.
    ReturnValueF64 = 0x19A, Control, pops: 1, pushes: 0;
    /// Return with `i32` value — pops 1 raw i32 (in i64 slot),
    /// frame-cleans, pushes 1.
    ReturnValueI32 = 0x19B, Control, pops: 1, pushes: 0;
    /// Return with `u32` value — pops 1 raw u32 (in i64 slot),
    /// frame-cleans, pushes 1.
    ReturnValueU32 = 0x19C, Control, pops: 1, pushes: 0;
    /// Return with `i16` value — pops 1 raw i16 (in i64 slot),
    /// frame-cleans, pushes 1.
    ReturnValueI16 = 0x19D, Control, pops: 1, pushes: 0;
    /// Return with `u16` value — pops 1 raw u16 (in i64 slot),
    /// frame-cleans, pushes 1.
    ReturnValueU16 = 0x19E, Control, pops: 1, pushes: 0;
    /// Return with `i8` value — pops 1 raw i8 (in i64 slot),
    /// frame-cleans, pushes 1.
    ReturnValueI8 = 0x19F, Control, pops: 1, pushes: 0;
    /// Return with `u8` value — pops 1 raw u8 (in i64 slot),
    /// frame-cleans, pushes 1.
    ReturnValueU8 = 0x1A0, Control, pops: 1, pushes: 0;
    /// Return with `bool` value — pops 1 raw bool, frame-cleans, pushes 1.
    ReturnValueBool = 0x1A1, Control, pops: 1, pushes: 0;
    /// Return with `Ptr` value — pops 1 raw 8-byte ValueWord pointer
    /// payload, frame-cleans, pushes 1. Ownership transfer is by raw
    /// bit-level pass-through; the handler does NOT retain or release.
    ReturnValuePtr = 0x1A2, Control, pops: 1, pushes: 0;

    // ===== Track A.1C.1: Shared outer-scope (`var`) cell opcodes =====
    //
    // These are the *outer-scope* counterpart to A.1B's capture-side
    // `LoadSharedCapture` / `StoreSharedCapture`. A.1B handles reads and
    // writes seen from *inside* a nested closure. A.1C handles the
    // owning-frame side of the same `Arc<parking_lot::Mutex<ValueWord>>`
    // cell: allocating it when the `var` binding is introduced, reading
    // and writing it from code executing in the declaring frame, and
    // releasing the Arc strong share at scope exit.
    //
    // Operand layout: `Local(u16)` — indexes the declaring frame's
    // **stack slots** (not a capture-array index), i.e. the same
    // addressing mode as `LoadLocal` / `StoreLocal`. After
    // `AllocSharedLocal`, slot `slot` holds raw `*const SharedCell`
    // pointer bits (NOT a NaN-tagged ValueWord). Neither `LoadLocal` nor
    // `StoreLocal` may be used on such a slot — only the four opcodes
    // below. The compiler in A.1C.2 is responsible for emitting the
    // right opcode per reference after a `var` binding is promoted to
    // Shared storage.
    //
    // Lifecycle:
    //   1. `AllocSharedLocal { slot }` — sole allocator. Pops the
    //      initial value, boxes it in an `Arc<SharedCell>`, writes the
    //      `Arc::into_raw`-produced pointer bits into `slot`. The slot
    //      now owns one strong-count share.
    //   2. `LoadSharedLocal { slot }` / `StoreSharedLocal { slot }` —
    //      ordinary read/write through the mutex. The slot's pointer
    //      bits are never modified by these opcodes. Concurrency is
    //      mediated by the parking_lot mutex; the slot is the sole
    //      legal entry point for data access after Alloc.
    //   3. `DropSharedLocal { slot }` — sole releaser. Reads pointer
    //      bits, reconstructs `Arc::from_raw`, drops it (one atomic
    //      strong-count decrement), then overwrites the slot with
    //      `NONE_BITS` to mark it spent. The compiler emits this at
    //      scope exit for every `var` binding that was promoted.
    //
    // SAFETY invariants (enforced by the compiler A.1C.2 — these
    // opcodes trust the emitter):
    //   * `AllocSharedLocal` is emitted exactly once per `var` slot.
    //   * `LoadSharedLocal` / `StoreSharedLocal` only fire on a slot
    //     whose bits were installed by `AllocSharedLocal` and not yet
    //     consumed by `DropSharedLocal`.
    //   * `DropSharedLocal` is emitted exactly once per `var` slot on
    //     every path that leaves the owning scope (normal, break,
    //     return, panic-via-unwind — handled by scope-exit bytecode).
    //
    // A.1D / A.1E will lower these into Cranelift IR. Until then, the
    // JIT preflight gate (see `vm_only_opcode_reason` in
    // `crates/shape-jit/src/compiler/accessors.rs`) rejects functions
    // containing any of these four opcodes so they run on the
    // interpreter.

    /// Pop the top-of-stack `ValueWord` as the initial value, allocate a
    /// fresh `Arc<parking_lot::Mutex<ValueWord>>`, and store the
    /// `Arc::into_raw` pointer bits into local slot `slot`. Operand:
    /// Local(idx). Sole allocator for Shared locals.
    AllocSharedLocal = 0x136, Variable, pops: 1, pushes: 0;
    /// Read the SharedCell pointer bits from local slot `slot`, take
    /// the parking_lot mutex for a read, clone the inner ValueWord bits,
    /// drop the guard, push onto the stack. Operand: Local(idx).
    LoadSharedLocal = 0x137, Variable, pops: 0, pushes: 1;
    /// Pop a ValueWord from the stack, read the SharedCell pointer bits
    /// from local slot `slot`, take the parking_lot mutex for a write,
    /// overwrite the inner ValueWord bits, drop the guard. The slot's
    /// pointer bits are NOT modified. Operand: Local(idx).
    StoreSharedLocal = 0x138, Variable, pops: 1, pushes: 0;
    /// Read the SharedCell pointer bits from local slot `slot`,
    /// reconstruct `Arc::from_raw`, drop the Arc (one atomic
    /// strong-count decrement), and overwrite the slot with NONE_BITS
    /// to mark it spent. Operand: Local(idx). Sole releaser for Shared
    /// locals — emitted by the compiler at scope exit.
    DropSharedLocal = 0x139, Variable, pops: 0, pushes: 0;

    // ===== Track A.1C.3: Shared outer-scope (`var`) cell opcodes =====
    // =====                for module-binding slots             =====
    //
    // Parallel module-binding counterpart to A.1C.1's local-slot Shared
    // opcodes. Module `var` bindings captured mutably by closures are
    // promoted into `Arc<parking_lot::Mutex<ValueWord>>` (same
    // `SharedCell` type) stored in `module_bindings[idx]` as raw
    // pointer bits. The addressing mode is `Operand::ModuleBinding(u16)`
    // instead of `Operand::Local(u16)`; semantics otherwise mirror the
    // local counterparts.
    //
    // Lifecycle:
    //   1. `AllocSharedModuleBinding { idx }` — sole allocator. Pops
    //      the initial value, boxes it in an `Arc<SharedCell>`, writes
    //      the `Arc::into_raw`-produced pointer bits into
    //      `module_bindings[idx]`. Registers `idx` with the VM so
    //      VM-drop releases the Arc.
    //   2. `LoadSharedModuleBinding { idx }` /
    //      `StoreSharedModuleBinding { idx }` — ordinary read/write
    //      through the mutex.
    //
    // Unlike the local-scope counterparts, there is no explicit
    // `DropSharedModuleBinding` opcode: module bindings live for the
    // program's lifetime, so their Arcs are released once, at VM drop,
    // via the `shared_module_bindings` side-table on the VM.
    //
    // SAFETY invariants (enforced by the compiler — these opcodes trust
    // the emitter):
    //   * `AllocSharedModuleBinding` is emitted exactly once per
    //     module-binding slot that gets promoted.
    //   * `LoadSharedModuleBinding` / `StoreSharedModuleBinding` only
    //     fire on a slot whose bits were installed by
    //     `AllocSharedModuleBinding`. Plain `LoadModuleBinding` /
    //     `StoreModuleBinding` must not be emitted for a promoted
    //     slot — they would read raw pointer bits as a ValueWord.
    //
    // A.1D / A.1E will lower these into Cranelift IR. Until then, the
    // JIT preflight gate rejects functions containing any of these
    // three opcodes so they run on the interpreter.

    /// Pop the top-of-stack `ValueWord` as the initial value, allocate a
    /// fresh `Arc<parking_lot::Mutex<ValueWord>>`, and store the
    /// `Arc::into_raw` pointer bits into `module_bindings[idx]`.
    /// Operand: ModuleBinding(idx). Sole allocator for Shared module
    /// bindings.
    AllocSharedModuleBinding = 0x13A, Variable, pops: 1, pushes: 0;
    /// Read the SharedCell pointer bits from `module_bindings[idx]`,
    /// take the parking_lot mutex for a read, clone the inner ValueWord
    /// bits, drop the guard, push onto the stack. Operand:
    /// ModuleBinding(idx).
    LoadSharedModuleBinding = 0x13B, Variable, pops: 0, pushes: 1;
    /// Pop a ValueWord from the stack, read the SharedCell pointer bits
    /// from `module_bindings[idx]`, take the parking_lot mutex for a
    /// write, overwrite the inner ValueWord bits, drop the guard. The
    /// slot's pointer bits are NOT modified. Operand:
    /// ModuleBinding(idx).
    StoreSharedModuleBinding = 0x13C, Variable, pops: 1, pushes: 0;

    // ===== Closure Spec Phase F: escape-fallback dispatch =====
    //
    // These opcodes implement the v2 escape-fallback ABI (see
    // `docs/v2-closure-specialization.md` §1.3, §5.3, §5.4).
    //
    // The former `MakeClosureHeap` opcode was merged into `MakeClosure` in
    // Phase H5 — escape status is now carried by the operand variant
    // (`Operand::ClosureAlloc { escapes }`). See `MakeClosure` above.
    //
    // - `CallClosure(arity)`: direct dispatch on a closure value whose
    //   `ClosureTypeId` is known at the call site. Pops the closure pointer
    //   and `arity` args, binds captures + args to the callee's leading
    //   locals, and jumps to the callee's entry point.
    //
    // - `CallFunctionIndirect(arity)`: polymorphic dispatch through a
    //   `Function<A, R>` value. The operand carries the number of args; the
    //   `FunctionTypeId` is implicit from type inference (the JIT uses it to
    //   pick a `call_indirect` signature; the VM treats it as a sanity tag).
    //   Falls back to the same runtime path as `CallClosure` when the callee
    //   is a closure or `CallValue` when it's a bare function id.
    /// Direct dispatch on a closure value whose `ClosureTypeId` is statically
    /// known at the call site. Operand: Count(arity).
    ///
    /// Stack layout before: `[closure, arg0, arg1, ..., argN-1]`.
    /// Stack layout after: `[result]`.
    CallClosure = 0x123, Control, pops: 0, pushes: 0;
    /// Polymorphic dispatch through a `Function<A, R>` value. Operand:
    /// Count(arity). Same stack layout as `CallClosure`.
    CallFunctionIndirect = 0x124, Control, pops: 0, pushes: 0;

    // ===== V1.1A: Ownership-aware Move/Clone/Drop (UNWIRED — V1.1B adds handlers) =====
    //
    // These opcodes are added to the enum table per the staged A/B/C/D gating
    // pattern described in `/home/dev/.claude/plans/i-want-a-complete-foamy-eich.md`
    // §V1.1A. V1.1A lands the enum variants only. V1.1B adds executor handlers,
    // V1.1C adds compiler emission behind a flag, V1.1D flips the default.
    //
    // See `docs/ownership-aware-runtime-v2.md` §Phase 1.1 for semantics.
    //
    // Operand: `Operand::Local(u16)` — the local slot to move/clone/drop.
    /// V1.1A (UNWIRED): Move value out of a local slot — transfers ownership
    /// without refcount bump, source slot is invalidated. Pushes the value.
    /// Executor handler added in V1.1B; currently unreachable via dispatch.
    MoveLocal = 0x125, Variable, pops: 0, pushes: 1;
    /// V1.1A (UNWIRED): Clone value from a local slot with refcount-aware
    /// semantics — for heap-tagged shared values bumps Arc refcount; for owned
    /// values performs a deep clone. Source stays live. Pushes the value.
    /// Executor handler added in V1.1B; currently unreachable via dispatch.
    CloneLocal = 0x126, Variable, pops: 0, pushes: 1;
    /// V1.1A (UNWIRED): Explicit drop of a local slot at scope exit — for
    /// owned heap values frees immediately; for shared values decrements the
    /// refcount. No stack effect.
    /// Executor handler added in V1.1B; currently unreachable via dispatch.
    DropLocal = 0x127, Variable, pops: 0, pushes: 0;

    // ===== V1.2A: PromoteToShared (UNWIRED — V1.2B adds handler) =====
    //
    // Inverse of `PromoteToOwned` (0x107). Converts a Box-owned heap value on
    // top-of-stack into an Arc-shared one on demand (used when the value is
    // captured by an escaping closure, stored into a SharedCow slot, or
    // passed to a function expecting Arc-shared ownership).
    //
    // Staged A/B/C/D rollout per
    // `/home/dev/.claude/plans/i-want-a-complete-foamy-eich.md` §V1.2A:
    //   - V1.2A (this commit): enum variant only, dead.
    //   - V1.2B: executor handler wired to dispatch, still unused.
    //   - V1.2C: compiler emission behind a gating flag.
    //   - V1.2D: default flip after soak.
    //
    // See `docs/ownership-aware-runtime-v2.md` §Phase 3 for semantics.
    //
    // Operand: none — operates on the value already at top-of-stack and
    // mutates it in place (identical stack shape to `PromoteToOwned`).
    /// V1.2A (UNWIRED): Demote/promote the top-of-stack value from Box-owned
    /// to Arc-shared allocation. No-op for inline values or already-shared
    /// heap values. Executor handler added in V1.2B; currently unreachable
    /// via dispatch — reaching this opcode panics.
    PromoteToShared = 0x128, Stack, pops: 0, pushes: 0;

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

    // ===== R5.1A: Typed bitwise opcodes (UNWIRED — R5.1B adds handlers) =====
    //
    // Phase R5.1A of the v2 residuals closeout. Mirrors the V1.1A staging
    // pattern: enum variants only, dead. R5.1B wires executor handlers,
    // R5.1C adds compiler emission behind `SHAPE_V2_TYPED_BITWISE=1`.
    //
    // These opcodes are the int-typed siblings of the existing dynamic
    // `BitAnd`/`BitOr`/`BitXor`/`BitShl`/`BitShr`/`BitNot` operations and
    // begin closing out the bitwise slice of `exec_arithmetic_dynamic_fallback`.
    //
    // Operand: none (simple instruction). Binary variants pop 2 / push 1;
    // `BitNotInt` is unary: pop 1 / push 1. Shift semantics match Shape's
    // existing `>>`/`<<` — `BitShrInt` is an arithmetic right-shift on i64,
    // matching the `a_int >> b_int` used by the dynamic `BitShr` handler.
    /// R5.1A (UNWIRED): Bitwise AND on two i64 values (int × int → int).
    /// Executor handler added in R5.1B; currently unreachable via dispatch.
    BitAndInt = 0x129, Arithmetic, pops: 2, pushes: 1;
    /// R5.1A (UNWIRED): Bitwise OR on two i64 values (int × int → int).
    /// Executor handler added in R5.1B; currently unreachable via dispatch.
    BitOrInt = 0x12A, Arithmetic, pops: 2, pushes: 1;
    /// R5.1A (UNWIRED): Bitwise XOR on two i64 values (int × int → int).
    /// Executor handler added in R5.1B; currently unreachable via dispatch.
    BitXorInt = 0x12B, Arithmetic, pops: 2, pushes: 1;
    /// R5.1A (UNWIRED): Bitwise shift-left on two i64 values (int × int → int).
    /// Executor handler added in R5.1B; currently unreachable via dispatch.
    BitShlInt = 0x12C, Arithmetic, pops: 2, pushes: 1;
    /// R5.1A (UNWIRED): Bitwise arithmetic shift-right on two i64 values
    /// (int × int → int). Matches Shape's `>>` operator semantics.
    /// Executor handler added in R5.1B; currently unreachable via dispatch.
    BitShrInt = 0x12D, Arithmetic, pops: 2, pushes: 1;
    /// R5.1A (UNWIRED): Bitwise NOT on an i64 value (int → int).
    /// Executor handler added in R5.1B; currently unreachable via dispatch.
    BitNotInt = 0x12E, Arithmetic, pops: 1, pushes: 1;

    // ===== R5.5: Typed string+scalar concatenation =====
    //
    // Typed siblings of the dynamic `AddDynamic` handler's string-coercion
    // branch (`exec_arithmetic_dynamic_fallback` → `try_heap_arithmetic`,
    // Case 2 "string + scalar"). Pops a heap-tagged string LHS and a
    // raw-scalar RHS, pushes a newly-allocated string. The compiler emits
    // these (R5.5) when `BinaryOp::Add` is proved to have a `string` LHS
    // and an `int` / `number` / `bool` RHS, bypassing the dynamic fallback.
    //
    // Semantics match the pre-R5.5 fallback for int/number:
    //   * int → `format!("{}{}", lhs, rhs_i64)`
    //   * number → integer-formatted if `rhs.fract() == 0.0`, else default
    //     float formatting (mirrors the `n.fract() == 0.0` branch in the
    //     legacy fallback at arithmetic/mod.rs:1821).
    //   * bool → `"true"` / `"false"` (pre-R5.5 fell through the fallback
    //     and returned a garbage numeric coercion; R5.5 produces the
    //     canonical textual form). See R5.5 commit body.
    //
    // Category: Object (shared with `StringConcat` / `StringConcatTyped`,
    // matches their single-allocation heap-producing shape). Operand: none
    // (simple instruction). Stack effect: pop 2 / push 1.
    /// R5.5: Concatenate a heap string with an `int` scalar. Pops (string,
    /// i64 raw int), formats the int via `format!("{}{}", s, i)`, pushes a
    /// newly-allocated string. Compile-time proof of operand types — no tag
    /// checks beyond the string decode.
    StringConcatInt = 0x12F, Object, pops: 2, pushes: 1;
    /// R5.5: Concatenate a heap string with a `number` scalar. Pops (string,
    /// raw f64), formats the number via the same integer-fast-path logic as
    /// the legacy fallback (whole numbers render without a decimal), pushes a
    /// newly-allocated string. Compile-time proof of operand types.
    StringConcatNumber = 0x130, Object, pops: 2, pushes: 1;
    /// R5.5: Concatenate a heap string with a `bool` scalar. Pops (string,
    /// raw bool), formats the bool as `"true"` / `"false"`, pushes a
    /// newly-allocated string. Compile-time proof of operand types.
    StringConcatBool = 0x131, Object, pops: 2, pushes: 1;

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
            | OpCode::NewTypedArrayBool
            | OpCode::TypedArrayGetF64
            | OpCode::TypedArrayGetI64
            | OpCode::TypedArrayGetI32
            | OpCode::TypedArrayGetBool
            | OpCode::TypedArraySetF64
            | OpCode::TypedArraySetI64
            | OpCode::TypedArraySetI32
            | OpCode::TypedArraySetBool
            | OpCode::TypedArrayPushF64
            | OpCode::TypedArrayPushI64
            | OpCode::TypedArrayPushI32
            | OpCode::TypedArrayPushBool
            | OpCode::TypedArrayLen
            // Local-slot-based typed array element access
            | OpCode::GetElemI64
            | OpCode::GetElemF64
            | OpCode::SetElemI64
            | OpCode::SetElemF64
            | OpCode::ArrayPushI64
            | OpCode::ArrayPushF64
            | OpCode::ArrayLenTyped
            // Local-slot-based typed HashMap access
            | OpCode::MapGetStrI64
            | OpCode::MapGetStrF64
            | OpCode::MapSetStrI64
            | OpCode::MapHasStr
            | OpCode::MapLenTyped
            // Local-slot-based typed String access
            | OpCode::StringLenTyped
            | OpCode::StringCharAt
            | OpCode::StringConcatTyped
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
        /// Compile-time resolved receiver type tag (ConcreteType::type_tag()).
        /// 0xFF = unknown (triggers runtime tag/HeapKind dispatch fallback).
        receiver_type_tag: u8,
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
    /// Closure allocation operand used exclusively by `MakeClosure` (Phase H5).
    ///
    /// Carries both the function id and a compile-time escape flag. The flag is
    /// read at MIR lowering time to pick between stack-allocated (Phase E) and
    /// heap-allocated (Phase H2) codegen; the interpreter ignores it (both
    /// variants build a heap closure in the VM).
    ///
    /// `Operand::Function(fid)` is also accepted by `MakeClosure` and is
    /// equivalent to `ClosureAlloc { fid, escapes: false }` — the compiler
    /// emits the richer form only when the storage planner has concluded the
    /// closure escapes.
    ClosureAlloc {
        fid: shape_value::FunctionId,
        escapes: bool,
    },
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
    // Len removed: use x.len() method form via per-type PHF dispatch
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
    /// `Vec<int> + Vec<int>` — element-wise, overflow-checked (R5.4D).
    /// Mirrors the dynamic-fallback `TypedArrayData::I64 + I64` arm:
    /// returns an `IntArray` and surfaces an overflow error when any
    /// element pair saturates (see `simd_vec_add_i64`). Wired up here as
    /// scaffolding; compiler emission arrives in R5.4E.
    IntrinsicVecAddI64,

    // Matrix intrinsics (4 functions)
    IntrinsicMatMulVec,
    IntrinsicMatMulMat,
    /// `Mat<number> + Mat<number>` — element-wise (R5.4D). Dispatches to
    /// `matrix_kernels::matrix_add` after extracting nested-array input
    /// via `extract_matrix_f64`. Returns a matrix in the nested-array
    /// shape that R5.4B's `Mat<number>` literals produce. Unwired from
    /// the compiler side; emission lands in R5.4E.
    IntrinsicMatAdd,
    /// `Mat<number> - Mat<number>` — element-wise (R5.4D). Companion to
    /// `IntrinsicMatAdd`, dispatches to `matrix_kernels::matrix_sub`.
    IntrinsicMatSub,

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
            // Utility (4)
            BuiltinFunction::Print,
            BuiltinFunction::Format,
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
            // Vector (12 — includes R5.4D IntrinsicVecAddI64)
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
            BuiltinFunction::IntrinsicVecAddI64,
            // Matrix (4 — includes R5.4D IntrinsicMatAdd / IntrinsicMatSub)
            BuiltinFunction::IntrinsicMatMulVec,
            BuiltinFunction::IntrinsicMatMulMat,
            BuiltinFunction::IntrinsicMatAdd,
            BuiltinFunction::IntrinsicMatSub,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// V1.1A: verify the new ownership opcodes round-trip through their u16
    /// discriminants — the opcode byte table assigns 0x125/0x126/0x127, and
    /// the `#[repr(u16)]` enum must produce exactly those values.
    #[test]
    fn v11a_move_local_discriminant() {
        let op = OpCode::MoveLocal;
        assert_eq!(op as u16, 0x125);
    }

    #[test]
    fn v11a_clone_local_discriminant() {
        let op = OpCode::CloneLocal;
        assert_eq!(op as u16, 0x126);
    }

    #[test]
    fn v11a_drop_local_discriminant() {
        let op = OpCode::DropLocal;
        assert_eq!(op as u16, 0x127);
    }

    /// V1.1A: the three ownership opcodes are classified as Variable-category
    /// (they operate on a local slot by index).
    #[test]
    fn v11a_ownership_opcodes_are_variable_category() {
        assert_eq!(OpCode::MoveLocal.category(), OpcodeCategory::Variable);
        assert_eq!(OpCode::CloneLocal.category(), OpcodeCategory::Variable);
        assert_eq!(OpCode::DropLocal.category(), OpcodeCategory::Variable);
    }

    /// V1.1A: stack effects — Move/Clone push one value, Drop is zero-effect.
    #[test]
    fn v11a_ownership_opcode_stack_effects() {
        // MoveLocal: reads local, pushes onto stack → 0 pops, 1 push
        assert_eq!(OpCode::MoveLocal.stack_pops(), 0);
        assert_eq!(OpCode::MoveLocal.stack_pushes(), 1);
        // CloneLocal: reads local, pushes onto stack → 0 pops, 1 push
        assert_eq!(OpCode::CloneLocal.stack_pops(), 0);
        assert_eq!(OpCode::CloneLocal.stack_pushes(), 1);
        // DropLocal: drops a local slot in place → 0 pops, 0 pushes
        assert_eq!(OpCode::DropLocal.stack_pops(), 0);
        assert_eq!(OpCode::DropLocal.stack_pushes(), 0);
    }

    /// V1.1A: the new opcodes are neither trusted nor v2-typed.
    /// They're ownership-aware variants that will be validated by a dedicated
    /// verifier pass once V1.1B/C land.
    #[test]
    fn v11a_ownership_opcodes_not_classified_as_trusted_or_v2() {
        for op in [OpCode::MoveLocal, OpCode::CloneLocal, OpCode::DropLocal] {
            assert!(!op.is_trusted(), "{:?} should not be trusted", op);
            assert!(!op.is_v2_typed(), "{:?} should not be v2_typed", op);
        }
    }

    /// V1.1A: Instruction-level construction round-trips the opcode and
    /// preserves the `Local(u16)` operand. Mirrors the "decoder on
    /// manually-encoded bytes" check requested by the V1.1A plan — Shape
    /// constructs Instruction values directly rather than byte-decoding.
    #[test]
    fn v11a_ownership_instructions_preserve_operand() {
        let m = Instruction::new(OpCode::MoveLocal, Some(Operand::Local(7)));
        assert_eq!(m.opcode, OpCode::MoveLocal);
        assert!(matches!(m.operand, Some(Operand::Local(7))));

        let c = Instruction::new(OpCode::CloneLocal, Some(Operand::Local(3)));
        assert_eq!(c.opcode, OpCode::CloneLocal);
        assert!(matches!(c.operand, Some(Operand::Local(3))));

        let d = Instruction::new(OpCode::DropLocal, Some(Operand::Local(42)));
        assert_eq!(d.opcode, OpCode::DropLocal);
        assert!(matches!(d.operand, Some(Operand::Local(42))));
    }

    /// V1.2A: the new `PromoteToShared` opcode is assigned 0x128, immediately
    /// after `DropLocal` (0x127). Mirrors the V1.1A discriminant pins.
    #[test]
    fn v12a_promote_to_shared_discriminant() {
        assert_eq!(OpCode::PromoteToShared as u16, 0x128);
    }

    /// V1.2A: `PromoteToShared` is the inverse of `PromoteToOwned` and shares
    /// its categorization — both live in the `Stack` category because they
    /// operate on top-of-stack without an operand.
    #[test]
    fn v12a_promote_to_shared_is_stack_category() {
        assert_eq!(OpCode::PromoteToShared.category(), OpcodeCategory::Stack);
        // Symmetry: PromoteToOwned is the companion and must share the category.
        assert_eq!(OpCode::PromoteToOwned.category(), OpcodeCategory::Stack);
    }

    /// V1.2A: stack effect is zero/zero — the opcode mutates the top-of-stack
    /// value in place (identical to `PromoteToOwned`).
    #[test]
    fn v12a_promote_to_shared_stack_effect() {
        assert_eq!(OpCode::PromoteToShared.stack_pops(), 0);
        assert_eq!(OpCode::PromoteToShared.stack_pushes(), 0);
        // Symmetry with the inverse opcode.
        assert_eq!(OpCode::PromoteToOwned.stack_pops(), 0);
        assert_eq!(OpCode::PromoteToOwned.stack_pushes(), 0);
    }

    /// V1.2A: `PromoteToShared` is neither a trusted opcode nor a v2-typed
    /// opcode. Ownership-aware opcodes will be validated by a dedicated
    /// ownership verifier pass in a later phase.
    #[test]
    fn v12a_promote_to_shared_not_trusted_or_v2() {
        let op = OpCode::PromoteToShared;
        assert!(!op.is_trusted(), "PromoteToShared should not be trusted");
        assert!(!op.is_v2_typed(), "PromoteToShared should not be v2_typed");
    }

    /// V1.2A: `Instruction::simple` constructs a `PromoteToShared` with no
    /// operand (same shape as `PromoteToOwned`) and round-trips the opcode.
    #[test]
    fn v12a_promote_to_shared_instruction_roundtrip() {
        let instr = Instruction::simple(OpCode::PromoteToShared);
        assert_eq!(instr.opcode, OpCode::PromoteToShared);
        assert!(
            instr.operand.is_none(),
            "PromoteToShared should have no operand (like PromoteToOwned)"
        );
    }

    // ===== R5.1A: Typed bitwise opcode tests =====

    /// R5.1A: pin each new bitwise opcode's u16 discriminant. The bytecode
    /// ABI is stable, so these IDs must not drift across phases. IDs were
    /// chosen sequentially above the highest existing discriminant at the
    /// time of landing (0x128 PromoteToShared).
    #[test]
    fn r51a_typed_bitwise_discriminants() {
        assert_eq!(OpCode::BitAndInt as u16, 0x129);
        assert_eq!(OpCode::BitOrInt as u16, 0x12A);
        assert_eq!(OpCode::BitXorInt as u16, 0x12B);
        assert_eq!(OpCode::BitShlInt as u16, 0x12C);
        assert_eq!(OpCode::BitShrInt as u16, 0x12D);
        assert_eq!(OpCode::BitNotInt as u16, 0x12E);
    }

    /// R5.1A: all six typed bitwise opcodes are Arithmetic-category, matching
    /// both their dynamic fallback counterparts (BitAnd/BitOr/BitXor/...) and
    /// the typed integer arithmetic opcodes (AddInt/SubInt/MulInt).
    #[test]
    fn r51a_typed_bitwise_opcodes_are_arithmetic_category() {
        assert_eq!(OpCode::BitAndInt.category(), OpcodeCategory::Arithmetic);
        assert_eq!(OpCode::BitOrInt.category(), OpcodeCategory::Arithmetic);
        assert_eq!(OpCode::BitXorInt.category(), OpcodeCategory::Arithmetic);
        assert_eq!(OpCode::BitShlInt.category(), OpcodeCategory::Arithmetic);
        assert_eq!(OpCode::BitShrInt.category(), OpcodeCategory::Arithmetic);
        assert_eq!(OpCode::BitNotInt.category(), OpcodeCategory::Arithmetic);
    }

    /// R5.1A: stack effects — binary bitwise ops pop two and push one;
    /// `BitNotInt` is unary (pop 1, push 1). Mirrors `AddInt`/`NegInt`.
    #[test]
    fn r51a_typed_bitwise_opcode_stack_effects() {
        for op in [
            OpCode::BitAndInt,
            OpCode::BitOrInt,
            OpCode::BitXorInt,
            OpCode::BitShlInt,
            OpCode::BitShrInt,
        ] {
            assert_eq!(op.stack_pops(), 2, "{:?} should pop 2", op);
            assert_eq!(op.stack_pushes(), 1, "{:?} should push 1", op);
        }
        assert_eq!(OpCode::BitNotInt.stack_pops(), 1);
        assert_eq!(OpCode::BitNotInt.stack_pushes(), 1);
    }

    /// R5.1A: the six new typed bitwise opcodes are neither trusted nor
    /// v2-typed, mirroring `AddInt`/`SubInt`/`MulInt` (the other int-typed
    /// arithmetic opcodes). The v2-typed classification is reserved for the
    /// sized-integer (i32) family and typed-array/typed-field ops, which
    /// require a FrameDescriptor. R5.1B/R5.1C may extend classification once
    /// handlers and compiler emission exist.
    #[test]
    fn r51a_typed_bitwise_opcodes_not_classified_as_trusted_or_v2() {
        for op in [
            OpCode::BitAndInt,
            OpCode::BitOrInt,
            OpCode::BitXorInt,
            OpCode::BitShlInt,
            OpCode::BitShrInt,
            OpCode::BitNotInt,
        ] {
            assert!(!op.is_trusted(), "{:?} should not be trusted", op);
            assert!(!op.is_v2_typed(), "{:?} should not be v2_typed", op);
        }
    }

    /// R5.1A: `Instruction::simple` constructs every typed bitwise opcode
    /// with no operand (same shape as `AddInt`/`BitAnd`) and round-trips the
    /// opcode field.
    #[test]
    fn r51a_typed_bitwise_instructions_have_no_operand() {
        for op in [
            OpCode::BitAndInt,
            OpCode::BitOrInt,
            OpCode::BitXorInt,
            OpCode::BitShlInt,
            OpCode::BitShrInt,
            OpCode::BitNotInt,
        ] {
            let instr = Instruction::simple(op);
            assert_eq!(instr.opcode, op);
            assert!(
                instr.operand.is_none(),
                "{:?} should have no operand (like AddInt/BitAnd)",
                op
            );
        }
    }

    // ===== R5.5: Typed string+scalar concat discriminant & shape tests =====

    /// R5.5: pin each new string+scalar concat opcode's u16 discriminant.
    /// The bytecode ABI is stable, so these IDs must not drift across phases.
    /// IDs were chosen sequentially above the last R5.1A discriminant (0x12E).
    #[test]
    fn r55_string_scalar_concat_discriminants() {
        assert_eq!(OpCode::StringConcatInt as u16, 0x12F);
        assert_eq!(OpCode::StringConcatNumber as u16, 0x130);
        assert_eq!(OpCode::StringConcatBool as u16, 0x131);
    }

    /// R5.5: the three new string+scalar concat opcodes belong to the
    /// `Object` category, matching their siblings `StringConcat` (0xFC)
    /// and `StringConcatTyped` (0x116).
    #[test]
    fn r55_string_scalar_concat_opcodes_are_object_category() {
        assert_eq!(OpCode::StringConcatInt.category(), OpcodeCategory::Object);
        assert_eq!(OpCode::StringConcatNumber.category(), OpcodeCategory::Object);
        assert_eq!(OpCode::StringConcatBool.category(), OpcodeCategory::Object);
    }

    /// R5.5: each typed string+scalar concat opcode pops two (string, scalar)
    /// and pushes one new string. Same stack effect as `StringConcatTyped`.
    #[test]
    fn r55_string_scalar_concat_stack_effects() {
        for op in [
            OpCode::StringConcatInt,
            OpCode::StringConcatNumber,
            OpCode::StringConcatBool,
        ] {
            assert_eq!(op.stack_pops(), 2, "{:?} should pop 2", op);
            assert_eq!(op.stack_pushes(), 1, "{:?} should push 1", op);
        }
    }

    /// R5.5: neither trusted nor v2-typed (they still allocate a heap
    /// `StringObj` through the regular ValueWord pipeline). Mirrors
    /// `StringConcatTyped`'s classification.
    #[test]
    fn r55_string_scalar_concat_opcodes_not_classified_as_trusted_or_v2() {
        for op in [
            OpCode::StringConcatInt,
            OpCode::StringConcatNumber,
            OpCode::StringConcatBool,
        ] {
            assert!(!op.is_trusted(), "{:?} should not be trusted", op);
            assert!(!op.is_v2_typed(), "{:?} should not be v2_typed", op);
        }
    }

    /// R5.5: `Instruction::simple` constructs every string+scalar concat
    /// opcode with no operand (same shape as `StringConcatTyped`).
    #[test]
    fn r55_string_scalar_concat_instructions_have_no_operand() {
        for op in [
            OpCode::StringConcatInt,
            OpCode::StringConcatNumber,
            OpCode::StringConcatBool,
        ] {
            let instr = Instruction::simple(op);
            assert_eq!(instr.opcode, op);
            assert!(
                instr.operand.is_none(),
                "{:?} should have no operand (like StringConcatTyped)",
                op
            );
        }
    }
}
