mod core;
mod introspection;

pub use core::create_state_module;
#[cfg(test)]
pub(crate) use core::*;
#[cfg(test)]
pub(crate) use introspection::*;

#[cfg(test)]
#[path = "state_builtins_tests.rs"]
mod tests;
