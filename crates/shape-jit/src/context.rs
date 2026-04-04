//! JIT Context and Related Data Structures
//!
//! Contains the runtime context and data structures used by JIT-compiled code.

use super::nan_boxing::*;

// ============================================================================
// JITContext Field Offsets for Direct Memory Access
// ============================================================================
//
// These must match the #[repr(C)] struct layout of JITContext
// Regenerate with: rustc --edition 2024 scripts/jit_offsets.rs && ./jit_offsets

// Generic column access - columns are accessed via column_ptrs array indexed by column_map
// Timestamps pointer for time-based access
pub const TIMESTAMPS_PTR_OFFSET: i32 = 24;

// DataFrame access offsets
pub const COLUMN_PTRS_OFFSET: i32 = 32;
pub const COLUMN_COUNT_OFFSET: i32 = 40;
pub const ROW_COUNT_OFFSET: i32 = 48;
pub const CURRENT_ROW_OFFSET: i32 = 56;

// Locals and stack offsets
pub const LOCALS_OFFSET: i32 = 64;
pub const STACK_OFFSET: i32 = 2112; // 64 + (256 * 8)
pub const STACK_PTR_OFFSET: i32 = 6208; // 2112 + (512 * 8)

// GC safepoint flag pointer offset (for inline safepoint check)
pub const GC_SAFEPOINT_FLAG_PTR_OFFSET: i32 = 6328;

// ============================================================================
// Return Type Tags for stack[0]
// ============================================================================
//
// Used by `return_type_tag` field in JITContext to tell the executor how to
// interpret the raw bits in stack[0] without NaN-box decoding.

/// Legacy NaN-boxed value (default). Executor uses existing unboxing path.
pub const RETURN_TAG_NANBOXED: u8 = 0;
/// Raw f64 bits. Executor reads as `f64::from_bits(stack[0])`.
pub const RETURN_TAG_F64: u8 = 1;
/// Raw i64 bits. Executor reads as `stack[0] as i64`.
pub const RETURN_TAG_I64: u8 = 2;
/// Raw i32 bits (zero-extended to u64). Executor reads as `stack[0] as i32`.
pub const RETURN_TAG_I32: u8 = 3;
/// Raw bool (0 or 1). Executor reads as `stack[0] != 0`.
pub const RETURN_TAG_BOOL: u8 = 4;

// ============================================================================
// Compile-time layout verification for JITContext
// ============================================================================
//
// These assertions ensure the hardcoded byte offsets above remain in sync with
// the actual #[repr(C)] struct layout. A mismatch will produce a compile error.
const _: () = {
    assert!(
        std::mem::offset_of!(JITContext, timestamps_ptr) == TIMESTAMPS_PTR_OFFSET as usize,
        "TIMESTAMPS_PTR_OFFSET does not match JITContext layout"
    );
    assert!(
        std::mem::offset_of!(JITContext, column_ptrs) == COLUMN_PTRS_OFFSET as usize,
        "COLUMN_PTRS_OFFSET does not match JITContext layout"
    );
    assert!(
        std::mem::offset_of!(JITContext, column_count) == COLUMN_COUNT_OFFSET as usize,
        "COLUMN_COUNT_OFFSET does not match JITContext layout"
    );
    assert!(
        std::mem::offset_of!(JITContext, row_count) == ROW_COUNT_OFFSET as usize,
        "ROW_COUNT_OFFSET does not match JITContext layout"
    );
    assert!(
        std::mem::offset_of!(JITContext, current_row) == CURRENT_ROW_OFFSET as usize,
        "CURRENT_ROW_OFFSET does not match JITContext layout"
    );
    assert!(
        std::mem::offset_of!(JITContext, locals) == LOCALS_OFFSET as usize,
        "LOCALS_OFFSET does not match JITContext layout"
    );
    assert!(
        std::mem::offset_of!(JITContext, stack) == STACK_OFFSET as usize,
        "STACK_OFFSET does not match JITContext layout"
    );
    assert!(
        std::mem::offset_of!(JITContext, stack_ptr) == STACK_PTR_OFFSET as usize,
        "STACK_PTR_OFFSET does not match JITContext layout"
    );
    assert!(
        std::mem::offset_of!(JITContext, gc_safepoint_flag_ptr) == GC_SAFEPOINT_FLAG_PTR_OFFSET as usize,
        "GC_SAFEPOINT_FLAG_PTR_OFFSET does not match JITContext layout"
    );
};

// ============================================================================
// Type Aliases
// ============================================================================

/// Function pointer type for JIT-compiled strategy functions
pub type JittedStrategyFn = unsafe extern "C" fn(*mut JITContext) -> i32;

/// Legacy function signature for simple numeric computations
pub type JittedFn = unsafe extern "C" fn(*mut f64, *const f64, usize) -> f64;

/// OSR entry function signature.
///
/// This has the same binary signature as `JittedStrategyFn` -- the difference
/// is semantic: for OSR entry, the caller pre-fills `JITContext.locals` from
/// the interpreter's live frame before invocation, and reads modified locals
/// back on return.
///
/// # Arguments
/// * `ctx_ptr` - Pointer to a `JITContext` with locals pre-filled from the
///   interpreter frame (marshaled using the `OsrEntryPoint.local_kinds`).
///
/// # Returns
/// * `0`          - Success: execution completed. Modified locals are in
///                  `JITContext.locals`. The VM reads them back and continues
///                  at `OsrEntryPoint.exit_ip`.
/// * `i32::MIN+1` - Deopt requested: a type guard failed mid-loop. The VM
///                  reads locals from `JITContext.locals` and resumes at
///                  the `DeoptInfo.resume_ip` for the failing guard.
/// * Other negative - Error.
pub type OsrEntryFn = unsafe extern "C" fn(*mut JITContext) -> i32;

// ============================================================================
// Simulation Kernel ABI (Zero-Allocation Hot Path)
// ============================================================================

/// Function pointer type for simulation kernel functions (single series).
///
/// This is the "fused step" ABI that enables >10M ticks/sec by:
/// - Bypassing JITContext setup overhead
/// - Using direct pointer arithmetic for data access
/// - Avoiding all allocations in the hot loop
///
/// # Arguments
/// * `cursor_index` - Current position in the series (0-based)
/// * `series_ptrs` - Pointer to array of column pointers (*const *const f64)
/// * `state_ptr` - Pointer to TypedObject state (*mut u8)
///
/// # Returns
/// * 0 = continue execution
/// * 1 = signal generated (written to state)
/// * negative = error
///
/// # Safety
/// The caller must ensure:
/// - `cursor_index` is within bounds
/// - `series_ptrs` points to valid column pointer array
/// - `state_ptr` points to valid TypedObject with correct schema
pub type SimulationKernelFn = unsafe extern "C" fn(
    cursor_index: usize,
    series_ptrs: *const *const f64,
    state_ptr: *mut u8,
) -> i32;

/// Function pointer type for correlated (multi-series) kernel functions.
///
/// This extends the simulation kernel ABI to support multiple aligned time series.
/// Each series is accessed via compile-time resolved indices.
///
/// # Arguments
/// * `cursor_index` - Current position in all series (0-based, must be aligned)
/// * `series_ptrs` - Pointer to array of series data pointers (*const *const f64)
///                   Each pointer is a single f64 array (one series's data)
/// * `table_count` - Number of series (for bounds checking, known at compile time)
/// * `state_ptr` - Pointer to TypedObject state (*mut u8)
///
/// # Memory Layout
/// ```text
/// series_ptrs[0] -> [spy_close[0], spy_close[1], ..., spy_close[n-1]]
/// series_ptrs[1] -> [vix_close[0], vix_close[1], ..., vix_close[n-1]]
/// ...
/// ```
///
/// # JIT Access Pattern
/// ```asm
/// ; context.spy (series index 0)
/// mov rax, [series_ptrs + 0*8]     ; series pointer
/// mov xmm0, [rax + cursor_index*8] ; value at cursor
/// ```
///
/// # Returns
/// * 0 = continue execution
/// * 1 = signal generated (written to state)
/// * negative = error
///
/// # Safety
/// The caller must ensure:
/// - `cursor_index` is within bounds for ALL series
/// - `series_ptrs` points to valid array of `table_count` data pointers
/// - All series have the same length (aligned timestamps)
/// - `state_ptr` points to valid TypedObject with correct schema
pub type CorrelatedKernelFn = unsafe extern "C" fn(
    cursor_index: usize,
    series_ptrs: *const *const f64,
    table_count: usize,
    state_ptr: *mut u8,
) -> i32;

/// Configuration for compiling a simulation kernel.
///
/// This provides the field offset mappings needed to generate
/// direct memory access code for the kernel ABI.
///
/// Supports two modes:
/// - **Single-series**: `column_map` maps field names (close, volume) to column indices
/// - **Multi-series**: `table_map` maps series names (spy, vix) to series indices
#[derive(Debug, Clone)]
pub struct SimulationKernelConfig {
    /// Column index mappings: (field_name, column_index)
    /// e.g., [("close", 3), ("volume", 4)]
    /// Used in single-series mode for accessing columns within one series
    pub column_map: Vec<(String, usize)>,

    /// Series index mappings: (series_name, series_index)
    /// e.g., [("spy", 0), ("vix", 1), ("temperature", 2)]
    /// Used in multi-series mode for accessing multiple correlated series
    ///
    /// CRITICAL for JIT: Resolved at compile time, NOT runtime.
    /// `context.spy` → `series_ptrs[0][cursor_idx]`
    pub table_map: Vec<(String, usize)>,

    /// State field offsets: (field_name, byte_offset)
    /// e.g., [("cash", 0), ("position", 8), ("entry_price", 16)]
    pub state_field_offsets: Vec<(String, usize)>,

    /// Schema ID for the state TypedObject
    pub state_schema_id: u32,

    /// Total number of columns in the data (single-series mode)
    pub column_count: usize,

    /// Total number of series (multi-series mode)
    pub table_count: usize,
}

impl SimulationKernelConfig {
    /// Create a new kernel config for single-series mode.
    pub fn new(state_schema_id: u32, column_count: usize) -> Self {
        Self {
            column_map: Vec::new(),
            table_map: Vec::new(),
            state_field_offsets: Vec::new(),
            state_schema_id,
            column_count,
            table_count: 0,
        }
    }

    /// Create a new kernel config for multi-series (correlated) mode.
    ///
    /// Use this when simulating across multiple aligned time series
    /// (e.g., SPY vs VIX, temperature vs pressure).
    pub fn new_multi_table(state_schema_id: u32, table_count: usize) -> Self {
        Self {
            column_map: Vec::new(),
            table_map: Vec::new(),
            state_field_offsets: Vec::new(),
            state_schema_id,
            column_count: 0,
            table_count,
        }
    }

    /// Map a data field name to a column index (single-series mode).
    pub fn map_column(mut self, field_name: &str, column_index: usize) -> Self {
        self.column_map.push((field_name.to_string(), column_index));
        self
    }

    /// Map a series name to a series index (multi-series mode).
    ///
    /// CRITICAL: This mapping is resolved at compile time.
    /// `context.spy` in Shape → `series_ptrs[0][cursor_idx]` in generated code.
    pub fn map_series(mut self, series_name: &str, series_index: usize) -> Self {
        self.table_map.push((series_name.to_string(), series_index));
        self
    }

    /// Map a state field name to a byte offset.
    pub fn map_state_field(mut self, field_name: &str, offset: usize) -> Self {
        self.state_field_offsets
            .push((field_name.to_string(), offset));
        self
    }

    /// Get column index for a field name (single-series mode).
    pub fn get_column_index(&self, field_name: &str) -> Option<usize> {
        self.column_map
            .iter()
            .find(|(name, _)| name == field_name)
            .map(|(_, idx)| *idx)
    }

    /// Get series index for a series name (multi-series mode).
    ///
    /// This is used by the JIT compiler at compile time.
    pub fn get_series_index(&self, series_name: &str) -> Option<usize> {
        self.table_map
            .iter()
            .find(|(name, _)| name == series_name)
            .map(|(_, idx)| *idx)
    }

    /// Get state field offset for a field name.
    pub fn get_state_offset(&self, field_name: &str) -> Option<usize> {
        self.state_field_offsets
            .iter()
            .find(|(name, _)| name == field_name)
            .map(|(_, offset)| *offset)
    }

    /// Check if this config is for multi-series mode.
    pub fn is_multi_table(&self) -> bool {
        self.table_count > 0 || !self.table_map.is_empty()
    }
}

// ============================================================================
// JIT Data Structures
// ============================================================================

/// JIT-compatible closure structure
/// Holds function_id and a pointer to a heap-allocated array of captured values.
/// Supports unlimited captures (no fixed-size limit).
#[repr(C)]
pub struct JITClosure {
    pub function_id: u16,
    pub captures_count: u16,
    pub captures_ptr: *const u64, // Pointer to heap-allocated capture array (NaN-boxed)
}

impl JITClosure {
    /// Create a new JITClosure with dynamically allocated captures.
    ///
    /// The captures slice is copied into a heap-allocated `Box<[u64]>` that is
    /// leaked into a raw pointer. Call `drop_captures()` to reclaim the memory.
    pub fn new(function_id: u16, captures: &[u64]) -> Box<Self> {
        let captures_box: Box<[u64]> = captures.to_vec().into_boxed_slice();
        let captures_ptr = Box::into_raw(captures_box) as *const u64;
        Box::new(JITClosure {
            function_id,
            captures_count: captures.len() as u16,
            captures_ptr,
        })
    }

    /// Safely read a capture value by index.
    ///
    /// # Safety
    /// The captures_ptr must be valid and index must be < captures_count.
    #[inline]
    pub unsafe fn get_capture(&self, index: usize) -> u64 {
        debug_assert!(index < self.captures_count as usize);
        unsafe { *self.captures_ptr.add(index) }
    }

    /// Free the heap-allocated captures array.
    ///
    /// Idempotent: safe to call multiple times (no-op after first call).
    ///
    /// # Safety
    /// The captures_ptr must point to a valid allocation created by `new()`,
    /// or be null (no-op).
    pub unsafe fn drop_captures(&mut self) {
        if !self.captures_ptr.is_null() && self.captures_count > 0 {
            let count = self.captures_count as usize;
            let _ = unsafe {
                Box::from_raw(std::slice::from_raw_parts_mut(
                    self.captures_ptr as *mut u64,
                    count,
                ))
            };
            self.captures_ptr = std::ptr::null();
        }
    }
}

impl Drop for JITClosure {
    fn drop(&mut self) {
        // SAFETY: drop_captures is idempotent — if captures_ptr is already null
        // (e.g. from an explicit drop_captures() call), this is a no-op.
        unsafe { self.drop_captures() };
    }
}

/// JIT-compatible duration structure
#[repr(C)]
pub struct JITDuration {
    pub value: f64,
    pub unit: u8, // 0=seconds, 1=minutes, 2=hours, 3=days, 4=weeks, 5=bars
}

impl JITDuration {
    pub fn new(value: f64, unit: u8) -> Box<Self> {
        Box::new(JITDuration { value, unit })
    }

    pub fn box_duration(duration: Box<JITDuration>) -> u64 {
        use crate::nan_boxing::{HK_DURATION, jit_box};
        jit_box(HK_DURATION, *duration)
    }
}

/// JIT-compatible range structure
/// Represents a range with start and end values (both NaN-boxed)
#[repr(C)]
pub struct JITRange {
    pub start: u64, // NaN-boxed start value
    pub end: u64,   // NaN-boxed end value
}

impl JITRange {
    pub fn new(start: u64, end: u64) -> Box<Self> {
        Box::new(JITRange { start, end })
    }

    pub fn box_range(range: Box<JITRange>) -> u64 {
        use crate::nan_boxing::{HK_RANGE, jit_box};
        jit_box(HK_RANGE, *range)
    }
}

/// JIT-compatible SignalBuilder structure
/// Represents a signal builder for method chaining (series.where().then().capture())
#[repr(C)]
pub struct JITSignalBuilder {
    pub series: u64,                  // NaN-boxed TAG_TABLE
    pub conditions: Vec<u64>,         // Array of (condition_type, condition_series) pairs
    pub captures: Vec<(String, u64)>, // (name, value) pairs for captured values
}

impl JITSignalBuilder {
    pub fn new(series: u64) -> Box<Self> {
        Box::new(JITSignalBuilder {
            series,
            conditions: Vec::new(),
            captures: Vec::new(),
        })
    }

    pub fn add_where(&mut self, condition_series: u64) {
        // 0 = WHERE condition
        self.conditions.push(0);
        self.conditions.push(condition_series);
    }

    pub fn add_then(&mut self, condition_series: u64, max_gap: u64) {
        // 1 = THEN condition
        self.conditions.push(1);
        self.conditions.push(condition_series);
        self.conditions.push(max_gap);
    }

    pub fn add_capture(&mut self, name: String, value: u64) {
        self.captures.push((name, value));
    }

    pub fn box_builder(builder: Box<JITSignalBuilder>) -> u64 {
        use crate::nan_boxing::{HK_JIT_SIGNAL_BUILDER, jit_box};
        jit_box(HK_JIT_SIGNAL_BUILDER, *builder)
    }
}

/// JIT-compatible data reference structure
/// Represents a reference to a specific data row in time
#[repr(C)]
pub struct JITDataReference {
    pub timestamp: i64,
    pub symbol: *const String, // Pointer to symbol string
    pub timeframe_value: u32,  // Timeframe value
    pub timeframe_unit: u8,    // 0=Second, 1=Minute, 2=Hour, 3=Day, 4=Week, 5=Month, 6=Bar
    pub has_timezone: bool,
    pub timezone: *const String, // Pointer to timezone string (may be null)
}

impl JITDataReference {
    pub fn box_data_ref(data_ref: Box<JITDataReference>) -> u64 {
        use crate::nan_boxing::{HK_DATA_REFERENCE, jit_box};
        jit_box(HK_DATA_REFERENCE, *data_ref)
    }
}

// ============================================================================
// JITContext - Main Execution Context
// ============================================================================

/// JIT execution context passed to compiled functions
/// This struct must be C-compatible (#[repr(C)]) for FFI
///
/// Uses NaN-boxing for full type support
#[repr(C)]
#[derive(Debug, Clone)]
pub struct JITContext {
    // Position state
    pub in_position: bool,
    pub position_side: i8,       // 0=None, 1=Long, -1=Short
    pub entry_price: u64,        // NaN-boxed f64
    pub unrealized_pnl_pct: u64, // NaN-boxed f64

    // Timestamps pointer for time-based data access
    pub timestamps_ptr: *const i64,

    // ========== Generic DataFrame Access (industry-agnostic) ==========
    /// Array of column pointers (SIMD-aligned f64 arrays)
    /// Column order matches DataFrameSchema.column_names
    pub column_ptrs: *const *const f64,
    /// Number of columns in the DataFrame
    pub column_count: usize,
    /// Number of rows in the DataFrame
    pub row_count: usize,
    /// Current row index (for backtest iteration)
    pub current_row: usize,

    // Local variables (NaN-boxed values)
    pub locals: [u64; 256],

    // NaN-boxed stack for JIT execution
    pub stack: [u64; 512],
    pub stack_ptr: usize,

    // Heap object storage (owned by VM, JIT just holds pointers)
    pub heap_ptr: *mut std::ffi::c_void,

    // Function table for Call opcode (pointer to array of function pointers)
    pub function_table: *const JittedStrategyFn,
    pub function_table_len: usize,

    // ExecutionContext pointer for fallback to interpreter
    pub exec_context_ptr: *mut std::ffi::c_void,

    // Function names for closure-to-Value conversion
    // Points to contiguous String array from BytecodeProgram.functions
    pub function_names_ptr: *const String,
    pub function_names_len: usize,

    // ========== Async Execution Support ==========
    /// Pointer to event queue (for FFI calls to poll/push events)
    /// Points to a SharedEventQueue behind the scenes
    pub event_queue_ptr: *mut std::ffi::c_void,

    /// Suspension state: 0 = running, 1 = yielded, 2 = suspended
    pub suspension_state: u32,

    /// Iterations since last yield (for cooperative scheduling)
    pub iterations_since_yield: u64,

    /// Yield threshold - yield after this many iterations
    /// 0 = never yield automatically
    pub yield_threshold: u64,

    /// Alert pipeline pointer (for FFI calls to emit alerts)
    /// Points to AlertRouter behind the scenes
    pub alert_pipeline_ptr: *mut std::ffi::c_void,

    // ========== Simulation Mode Support ==========
    /// Simulation mode: 0 = disabled, 1 = DenseKernel, 2 = HybridKernel
    pub simulation_mode: u32,

    /// Pointer to simulation state (TypedObject for DenseKernel)
    /// JIT code accesses state fields via direct memory offset
    pub simulation_state_ptr: *mut u8,

    /// Size of simulation state data (for deallocation)
    pub simulation_state_size: usize,

    // ========== GC Integration ==========
    /// Pointer to GC safepoint flag (AtomicBool raw pointer).
    /// Null when GC is not enabled. The JIT safepoint function reads this
    /// to determine if a GC cycle is requested.
    pub gc_safepoint_flag_ptr: *const u8,

    /// Pointer to GcHeap for allocation fast path.
    /// Null when GC is not enabled.
    pub gc_heap_ptr: *mut std::ffi::c_void,

    /// Opaque pointer to JIT foreign-call bridge state.
    /// Null when no foreign functions are linked for this execution.
    pub foreign_bridge_ptr: *const std::ffi::c_void,

    /// v2: type tag for the return value in stack[0].
    /// 0 = NaN-boxed (legacy), 1 = raw f64, 2 = raw i64, 3 = raw i32, 4 = raw bool
    pub return_type_tag: u8,
}

impl Default for JITContext {
    fn default() -> Self {
        Self {
            in_position: false,
            position_side: 0,
            entry_price: box_number(0.0),
            unrealized_pnl_pct: box_number(0.0),
            // Timestamps pointer
            timestamps_ptr: std::ptr::null(),
            // Generic DataFrame access
            column_ptrs: std::ptr::null(),
            column_count: 0,
            row_count: 0,
            current_row: 0,
            // Local variables and stack
            locals: [TAG_NULL; 256],
            stack: [TAG_NULL; 512],
            stack_ptr: 0,
            heap_ptr: std::ptr::null_mut(),
            function_table: std::ptr::null(),
            function_table_len: 0,
            exec_context_ptr: std::ptr::null_mut(),
            function_names_ptr: std::ptr::null(),
            function_names_len: 0,
            // Async execution support
            event_queue_ptr: std::ptr::null_mut(),
            suspension_state: 0,
            iterations_since_yield: 0,
            yield_threshold: 0, // 0 = no automatic yielding
            alert_pipeline_ptr: std::ptr::null_mut(),
            // Simulation mode support
            simulation_mode: 0,
            simulation_state_ptr: std::ptr::null_mut(),
            simulation_state_size: 0,
            // GC integration
            gc_safepoint_flag_ptr: std::ptr::null(),
            gc_heap_ptr: std::ptr::null_mut(),
            foreign_bridge_ptr: std::ptr::null(),
            return_type_tag: 0,
        }
    }
}

impl JITContext {
    /// Get column value at offset from current row
    /// column_index is the column index in the DataFrame schema
    pub fn get_column_value(&self, column_index: usize, offset: i32) -> f64 {
        if self.column_ptrs.is_null() || column_index >= self.column_count {
            return 0.0;
        }
        let row_idx = (self.current_row as i32 + offset) as usize;
        if row_idx < self.row_count {
            unsafe {
                let col_ptr = *self.column_ptrs.add(column_index);
                if !col_ptr.is_null() {
                    *col_ptr.add(row_idx)
                } else {
                    0.0
                }
            }
        } else {
            0.0
        }
    }

    /// Update current row index for DataFrame iteration
    #[inline]
    pub fn set_current_row(&mut self, index: usize) {
        self.current_row = index;
    }

    /// Update current row for backtest iteration (alias for backward compatibility)
    #[inline]
    pub fn update_current_row(&mut self, index: usize) {
        self.current_row = index;
    }

    // ========================================================================
    // Simulation Mode Methods
    // ========================================================================

    /// Check if in simulation mode
    #[inline]
    pub fn is_simulation_mode(&self) -> bool {
        self.simulation_mode > 0
    }

    /// Set up context for DenseKernel simulation.
    ///
    /// # Arguments
    /// * `state_ptr` - Pointer to TypedObject state
    /// * `state_size` - Size of state data
    /// * `column_ptrs` - Pointers to data columns
    /// * `column_count` - Number of columns
    /// * `row_count` - Number of rows
    /// * `timestamps` - Pointer to timestamp array
    pub fn setup_simulation(
        &mut self,
        state_ptr: *mut u8,
        state_size: usize,
        column_ptrs: *const *const f64,
        column_count: usize,
        row_count: usize,
        timestamps: *const i64,
    ) {
        self.simulation_mode = 1; // DenseKernel mode
        self.simulation_state_ptr = state_ptr;
        self.simulation_state_size = state_size;
        self.column_ptrs = column_ptrs;
        self.column_count = column_count;
        self.row_count = row_count;
        self.current_row = 0;
        self.timestamps_ptr = timestamps;
    }

    /// Get simulation state field as f64.
    ///
    /// # Safety
    /// Caller must ensure offset is valid for the state TypedObject.
    #[inline]
    pub unsafe fn get_state_field_f64(&self, offset: usize) -> f64 {
        if self.simulation_state_ptr.is_null() {
            return 0.0;
        }
        let field_ptr = unsafe { self.simulation_state_ptr.add(8 + offset) } as *const u64;
        let bits = unsafe { *field_ptr };
        unbox_number(bits)
    }

    /// Set simulation state field as f64.
    ///
    /// # Safety
    /// Caller must ensure offset is valid for the state TypedObject.
    #[inline]
    pub unsafe fn set_state_field_f64(&mut self, offset: usize, value: f64) {
        if self.simulation_state_ptr.is_null() {
            return;
        }
        let field_ptr = unsafe { self.simulation_state_ptr.add(8 + offset) } as *mut u64;
        unsafe { *field_ptr = box_number(value) };
    }

    /// Clear simulation mode.
    pub fn clear_simulation(&mut self) {
        self.simulation_mode = 0;
        self.simulation_state_ptr = std::ptr::null_mut();
        self.simulation_state_size = 0;
    }
}

// ============================================================================
// JITDataFrame - Generic DataFrame for JIT (industry-agnostic)
// ============================================================================

/// Generic DataFrame storage for JIT execution.
/// Stores data as an array of columns, matching the generic column_ptrs
/// design in JITContext.
///
/// Column order MUST match the DataFrameSchema used during compilation.
pub struct JITDataFrame {
    /// Column data arrays (each Vec is one column)
    /// Columns are ordered by index as defined in DataFrameSchema
    pub columns: Vec<Vec<f64>>,
    /// Pointers to column data (for JITContext.column_ptrs)
    pub column_ptrs: Vec<*const f64>,
    /// Timestamps (always present, column 0 equivalent)
    pub timestamps: Vec<i64>,
    /// Number of rows
    pub row_count: usize,
}

impl JITDataFrame {
    /// Create an empty JITDataFrame
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            column_ptrs: Vec::new(),
            timestamps: Vec::new(),
            row_count: 0,
        }
    }

    /// Create from ExecutionContext using a schema mapping.
    /// The schema determines which columns to extract and their order.
    pub fn from_execution_context(
        ctx: &shape_runtime::context::ExecutionContext,
        schema: &shape_vm::bytecode::DataFrameSchema,
    ) -> Self {
        let mut data = Self::new();

        // NOTE: Series caching not yet implemented in ExecutionContext
        // For now, initialize empty columns for each schema column
        // TODO: Implement series caching when available
        let _ = (ctx, schema); // Suppress unused warnings
        for _ in 0..schema.column_names.len() {
            data.columns.push(Vec::new());
            data.column_ptrs.push(std::ptr::null());
        }

        data
    }

    /// Populate a JITContext with generic DataFrame pointers.
    /// This sets column_ptrs, column_count, row_count, and timestamps_ptr.
    pub fn populate_context(&self, ctx: &mut JITContext) {
        if !self.column_ptrs.is_empty() {
            ctx.column_ptrs = self.column_ptrs.as_ptr();
            ctx.column_count = self.column_ptrs.len();
        }
        ctx.row_count = self.row_count;

        if !self.timestamps.is_empty() {
            ctx.timestamps_ptr = self.timestamps.as_ptr();
        }
    }

    /// Get the number of rows
    pub fn len(&self) -> usize {
        self.row_count
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.row_count == 0
    }

    /// Get number of columns
    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    /// Create from a DataTable by extracting f64 columns and an optional timestamp column.
    ///
    /// All f64 columns are copied into SIMD-aligned buffers. If a column named
    /// "timestamp" (or typed as Timestamp) exists, it is extracted as i64.
    pub fn from_datatable(dt: &shape_value::DataTable) -> Self {
        use arrow_array::cast::AsArray;
        use arrow_schema::{DataType, TimeUnit};

        let batch = dt.inner();
        let schema = batch.schema();
        let num_rows = batch.num_rows();
        let mut columns = Vec::new();
        let mut timestamps = Vec::new();

        for (i, field) in schema.fields().iter().enumerate() {
            match field.data_type() {
                DataType::Float64 => {
                    let arr = batch
                        .column(i)
                        .as_primitive::<arrow_array::types::Float64Type>();
                    let col: Vec<f64> = (0..num_rows).map(|r| arr.value(r)).collect();
                    columns.push(col);
                }
                DataType::Timestamp(TimeUnit::Microsecond, _) => {
                    let arr = batch
                        .column(i)
                        .as_primitive::<arrow_array::types::TimestampMicrosecondType>();
                    timestamps = (0..num_rows).map(|r| arr.value(r)).collect();
                }
                DataType::Int64 => {
                    // Convert i64 to f64 for JIT column access
                    let arr = batch
                        .column(i)
                        .as_primitive::<arrow_array::types::Int64Type>();
                    let col: Vec<f64> = (0..num_rows).map(|r| arr.value(r) as f64).collect();
                    columns.push(col);
                }
                _ => {
                    // Skip non-numeric columns (strings, bools, etc.)
                }
            }
        }

        let column_ptrs: Vec<*const f64> = columns.iter().map(|c| c.as_ptr()).collect();

        Self {
            columns,
            column_ptrs,
            timestamps,
            row_count: num_rows,
        }
    }
}

impl Default for JITDataFrame {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// JITConfig - Compilation Configuration
// ============================================================================

/// JIT compilation configuration
#[derive(Debug, Clone)]
pub struct JITConfig {
    /// Optimization level (0-3)
    pub opt_level: u8,
    /// Enable debug symbols
    pub debug_symbols: bool,
    /// Minimum execution count before JIT compilation
    pub jit_threshold: usize,
}

impl Default for JITConfig {
    fn default() -> Self {
        Self {
            opt_level: 3,
            debug_symbols: false,
            jit_threshold: 100,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_closure_dynamic_captures_0() {
        // Zero captures — captures_ptr should be a valid (empty) allocation
        let closure = JITClosure::new(42, &[]);
        assert_eq!(closure.function_id, 42);
        assert_eq!(closure.captures_count, 0);
        // Drop is safe even with 0 captures
        let mut closure = closure;
        unsafe { closure.drop_captures() };
    }

    #[test]
    fn test_closure_dynamic_captures_5() {
        // Typical case: 5 captures
        let captures = [
            box_number(1.0),
            box_number(2.0),
            box_number(3.0),
            TAG_BOOL_TRUE,
            TAG_NULL,
        ];
        let closure = JITClosure::new(7, &captures);
        assert_eq!(closure.function_id, 7);
        assert_eq!(closure.captures_count, 5);

        unsafe {
            assert_eq!(unbox_number(closure.get_capture(0)), 1.0);
            assert_eq!(unbox_number(closure.get_capture(1)), 2.0);
            assert_eq!(unbox_number(closure.get_capture(2)), 3.0);
            assert_eq!(closure.get_capture(3), TAG_BOOL_TRUE);
            assert_eq!(closure.get_capture(4), TAG_NULL);
        }
    }

    #[test]
    fn test_closure_dynamic_captures_20() {
        // Exceeds old 16-capture limit
        let captures: Vec<u64> = (0..20).map(|i| box_number(i as f64)).collect();
        let closure = JITClosure::new(99, &captures);
        assert_eq!(closure.captures_count, 20);

        unsafe {
            for i in 0..20 {
                assert_eq!(unbox_number(closure.get_capture(i)), i as f64);
            }
        }
    }

    #[test]
    fn test_closure_dynamic_captures_64() {
        // Stress test: 64 captures
        let captures: Vec<u64> = (0..64).map(|i| box_number(i as f64 * 10.0)).collect();
        let closure = JITClosure::new(1, &captures);
        assert_eq!(closure.captures_count, 64);

        unsafe {
            for i in 0..64 {
                assert_eq!(unbox_number(closure.get_capture(i)), i as f64 * 10.0);
            }
        }
    }

    #[test]
    fn test_closure_captures_drop() {
        // Verify memory is properly freed (no leak under Miri/ASAN)
        let captures: Vec<u64> = (0..32).map(|i| box_number(i as f64)).collect();
        let mut closure = JITClosure::new(5, &captures);
        assert_eq!(closure.captures_count, 32);

        // Verify captures are valid before drop
        unsafe {
            assert_eq!(unbox_number(closure.get_capture(0)), 0.0);
            assert_eq!(unbox_number(closure.get_capture(31)), 31.0);
        }

        // Drop captures
        unsafe { closure.drop_captures() };
        assert!(closure.captures_ptr.is_null());
        assert_eq!(closure.captures_count, 32); // count unchanged, ptr nulled
    }

    #[test]
    fn test_closure_jit_box_roundtrip() {
        // Verify JITClosure survives jit_box/jit_unbox roundtrip
        let captures = [box_number(42.0), TAG_BOOL_FALSE];
        let closure = JITClosure::new(10, &captures);
        let bits = jit_box(HK_CLOSURE, *closure);

        assert!(is_heap_kind(bits, HK_CLOSURE));

        let recovered = unsafe { jit_unbox::<JITClosure>(bits) };
        assert_eq!(recovered.function_id, 10);
        assert_eq!(recovered.captures_count, 2);
        unsafe {
            assert_eq!(unbox_number(recovered.get_capture(0)), 42.0);
            assert_eq!(recovered.get_capture(1), TAG_BOOL_FALSE);
        }
    }

    #[test]
    fn test_closure_drop_impl_frees_captures_via_jit_drop() {
        // Verify the Drop impl on JITClosure frees the captures array
        // when the owning JitAlloc is freed via jit_drop.
        // Under Miri/ASAN this would catch a leak if Drop didn't work.
        let captures: Vec<u64> = (0..24).map(|i| box_number(i as f64)).collect();
        let closure = JITClosure::new(3, &captures);
        let bits = jit_box(HK_CLOSURE, *closure);

        // Read captures to confirm they're valid
        let recovered = unsafe { jit_unbox::<JITClosure>(bits) };
        assert_eq!(recovered.captures_count, 24);
        unsafe {
            assert_eq!(unbox_number(recovered.get_capture(23)), 23.0);
        }

        // jit_drop frees JitAlloc<JITClosure>, which calls Drop::drop on
        // JITClosure, which frees the captures array.
        unsafe { jit_drop::<JITClosure>(bits) };
    }

    #[test]
    fn test_closure_implicit_drop_on_box() {
        // Verify that simply dropping a Box<JITClosure> frees the captures.
        // (This tests the Drop impl without jit_box involvement.)
        let captures: Vec<u64> = (0..10).map(|i| box_number(i as f64)).collect();
        let closure = JITClosure::new(1, &captures);
        // closure is Box<JITClosure>, dropping it should free captures via Drop
        drop(closure);
        // No leak under Miri/ASAN
    }
}
