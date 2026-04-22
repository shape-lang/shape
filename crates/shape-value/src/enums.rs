//! Custom enum types for Shape

use crate::value_word::ValueWord;
use crate::value_word_drop::{vw_clone, vw_drop, vw_drop_slice};
use std::collections::HashMap;

/// Payload for custom enum variants.
///
/// Wave 4 WC.3: `Tuple` and `Struct` hold `ValueWord` bit patterns
/// that may carry a heap tag. A manual `Clone` runs `vw_clone` per
/// element (shared Arc refcount bump / owned Box deep-clone) and a
/// manual `Drop` runs `vw_drop` per element so refcounts stay
/// paired.
#[derive(Debug)]
pub enum EnumPayload {
    Unit,
    Tuple(Vec<ValueWord>),
    Struct(HashMap<String, ValueWord>),
}

impl Clone for EnumPayload {
    fn clone(&self) -> Self {
        match self {
            EnumPayload::Unit => EnumPayload::Unit,
            EnumPayload::Tuple(v) => {
                EnumPayload::Tuple(v.iter().map(|&b| vw_clone(b)).collect())
            }
            EnumPayload::Struct(m) => EnumPayload::Struct(
                m.iter()
                    .map(|(k, &b)| (k.clone(), vw_clone(b)))
                    .collect(),
            ),
        }
    }
}

impl Drop for EnumPayload {
    fn drop(&mut self) {
        match self {
            EnumPayload::Unit => {}
            EnumPayload::Tuple(v) => vw_drop_slice(v),
            EnumPayload::Struct(m) => {
                for (_, &b) in m.iter() {
                    vw_drop(b);
                }
            }
        }
    }
}

/// Custom enum value (enum name + variant + payload)
#[derive(Debug, Clone)]
pub struct EnumValue {
    pub enum_name: String,
    pub variant: String,
    pub payload: EnumPayload,
}
