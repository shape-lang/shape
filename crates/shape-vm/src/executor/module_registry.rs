//! Module function registry for the VM
//!
//! This module previously housed a VMContext-based module function registry.
//! All module function dispatch now uses the ValueWord-based `module_fn_table`
//! (Vec<ModuleFn>) on VirtualMachine. This module is retained for API
//! compatibility but contains no active functionality.
