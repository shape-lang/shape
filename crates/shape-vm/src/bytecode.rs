//! Bytecode instruction set for Shape VM

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shape_abi_v1::PermissionSet;
use shape_ast::ast::{DataDateTimeRef, DateTimeExpr, Duration, TimeReference, TypeAnnotation};
use shape_ast::data::Timeframe;
use shape_value::ValueWord;
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
