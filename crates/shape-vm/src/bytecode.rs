//! Bytecode instruction set for Shape VM

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shape_abi_v1::PermissionSet;
use shape_ast::ast::{DataDateTimeRef, DateTimeExpr, Duration, TimeReference, TypeAnnotation};
use shape_ast::data::Timeframe;
// `ValueWord` re-export removed: the v1 dynamic-tag word was deleted by the
// strict-typing bulldozer (see shape-value/src/lib.rs and CLAUDE.md "Forbidden
// Patterns"). The legacy `Constant::Value(ValueWord)` arm in `core_types.rs`
// is a Phase-2c surface — see ADR-006 §2.7.4 for the kinded constant variant
// rebuild.
use std::collections::HashMap;

const DEFAULT_TRAIT_IMPL_SELECTOR: &str = "__default__";

mod content_addressed;
mod core_types;
mod opcode_defs;
mod program_impl;
pub mod verifier;

pub use content_addressed::*;
pub use core_types::*;
pub use opcode_defs::*;
