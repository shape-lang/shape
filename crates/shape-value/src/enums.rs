//! Custom enum types for Shape

use crate::value_word::{ValueWord, ValueWordExt};
use std::collections::HashMap;

/// Payload for custom enum variants
#[derive(Debug, Clone)]
pub enum EnumPayload {
    Unit,
    Tuple(Vec<ValueWord>),
    Struct(HashMap<String, ValueWord>),
}

/// Custom enum value (enum name + variant + payload)
#[derive(Debug, Clone)]
pub struct EnumValue {
    pub enum_name: String,
    pub variant: String,
    pub payload: EnumPayload,
}
