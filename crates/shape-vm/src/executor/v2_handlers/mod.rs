//! v2 opcode handlers — typed struct field, typed array, and sized integer operations.

pub(crate) mod array;
pub(crate) mod field;
pub(crate) mod int;
pub(crate) mod typed_map;
pub(crate) mod v2_array_detect;

#[cfg(test)]
mod integration_tests;
