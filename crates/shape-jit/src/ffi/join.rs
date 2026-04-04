// v2-boundary: JOIN FFI functions deleted — no callers from MirToIR or executor.
// All functions (jit_join_values_equal, jit_temporal_match, jit_join_is_null,
// jit_join_null, jit_join_coalesce) were only self-contained. No ffi_symbols registration.
// If JOIN helpers are needed in v2, implement them against the new typed runtime.
