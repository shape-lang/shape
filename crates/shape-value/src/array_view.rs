//! Unified read/write views over all array variants (generic, int, float, bool, width-specific).
//!
//! Extracted from `value_word.rs` to reduce file size.

use crate::typed_buffer::{AlignedTypedBuffer, TypedBuffer};
use crate::value::VMArrayBuf;
use crate::value_word::{ValueWord, ValueWordExt};
use std::sync::Arc;

// ===== ArrayView: unified read-only view over all array variants =====

/// Unified read-only view over all array variants (generic, int, float, bool, width-specific).
///
/// Returned by `ValueWord::as_any_array()`. Use typed fast-path methods
/// (`as_f64_slice()`, `as_i64_slice()`) for hot paths, or `to_generic()`
/// / `get_nb()` for cold paths that need ValueWord values.
#[derive(Debug)]
pub enum ArrayView<'a> {
    Generic(&'a Arc<VMArrayBuf>),
    Int(&'a Arc<TypedBuffer<i64>>),
    Float(&'a Arc<AlignedTypedBuffer>),
    Bool(&'a Arc<TypedBuffer<u8>>),
    I8(&'a Arc<TypedBuffer<i8>>),
    I16(&'a Arc<TypedBuffer<i16>>),
    I32(&'a Arc<TypedBuffer<i32>>),
    U8(&'a Arc<TypedBuffer<u8>>),
    U16(&'a Arc<TypedBuffer<u16>>),
    U32(&'a Arc<TypedBuffer<u32>>),
    U64(&'a Arc<TypedBuffer<u64>>),
    F32(&'a Arc<TypedBuffer<f32>>),
}

impl<'a> ArrayView<'a> {
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            ArrayView::Generic(a) => a.len(),
            ArrayView::Int(a) => a.len(),
            ArrayView::Float(a) => a.len(),
            ArrayView::Bool(a) => a.len(),
            ArrayView::I8(a) => a.len(),
            ArrayView::I16(a) => a.len(),
            ArrayView::I32(a) => a.len(),
            ArrayView::U8(a) => a.len(),
            ArrayView::U16(a) => a.len(),
            ArrayView::U32(a) => a.len(),
            ArrayView::U64(a) => a.len(),
            ArrayView::F32(a) => a.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get element at index as ValueWord (boxes typed elements — use for cold paths).
    #[inline]
    pub fn get_nb(&self, idx: usize) -> Option<ValueWord> {
        match self {
            ArrayView::Generic(a) => a.get(idx).copied(),
            ArrayView::Int(a) => a.get(idx).map(|&i| ValueWord::from_i64(i)),
            ArrayView::Float(a) => a.get(idx).map(|&f| ValueWord::from_f64(f)),
            ArrayView::Bool(a) => a.get(idx).map(|&b| ValueWord::from_bool(b != 0)),
            ArrayView::I8(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::I16(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::I32(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::U8(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::U16(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::U32(a) => a.get(idx).map(|&v| ValueWord::from_i64(v as i64)),
            ArrayView::U64(a) => a.get(idx).map(|&v| {
                if v <= i64::MAX as u64 {
                    ValueWord::from_i64(v as i64)
                } else {
                    ValueWord::from_native_u64(v)
                }
            }),
            ArrayView::F32(a) => a.get(idx).map(|&v| ValueWord::from_f64(v as f64)),
        }
    }

    #[inline]
    pub fn first_nb(&self) -> Option<ValueWord> {
        self.get_nb(0)
    }

    #[inline]
    pub fn last_nb(&self) -> Option<ValueWord> {
        if self.is_empty() {
            None
        } else {
            self.get_nb(self.len() - 1)
        }
    }

    /// Materialize into a generic ValueWord array. Cheap Arc clone for Generic variant.
    pub fn to_generic(&self) -> Arc<VMArrayBuf> {
        match self {
            ArrayView::Generic(a) => (*a).clone(),
            ArrayView::Int(a) => Arc::new(a.iter().map(|&i| ValueWord::from_i64(i)).collect()),
            ArrayView::Float(a) => Arc::new(
                a.as_slice()
                    .iter()
                    .map(|&f| ValueWord::from_f64(f))
                    .collect(),
            ),
            ArrayView::Bool(a) => {
                Arc::new(a.iter().map(|&b| ValueWord::from_bool(b != 0)).collect())
            }
            ArrayView::I8(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::I16(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::I32(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::U8(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::U16(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::U32(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_i64(v as i64)).collect())
            }
            ArrayView::U64(a) => Arc::new(
                a.iter()
                    .map(|&v| {
                        if v <= i64::MAX as u64 {
                            ValueWord::from_i64(v as i64)
                        } else {
                            ValueWord::from_native_u64(v)
                        }
                    })
                    .collect(),
            ),
            ArrayView::F32(a) => {
                Arc::new(a.iter().map(|&v| ValueWord::from_f64(v as f64)).collect())
            }
        }
    }

    #[inline]
    pub fn as_i64_slice(&self) -> Option<&[i64]> {
        if let ArrayView::Int(a) = self {
            Some(a.as_slice())
        } else {
            None
        }
    }

    #[inline]
    pub fn as_f64_slice(&self) -> Option<&[f64]> {
        if let ArrayView::Float(a) = self {
            Some(a.as_slice())
        } else {
            None
        }
    }

    #[inline]
    pub fn as_bool_slice(&self) -> Option<&[u8]> {
        if let ArrayView::Bool(a) = self {
            Some(a.as_slice())
        } else {
            None
        }
    }

    #[inline]
    pub fn as_generic(&self) -> Option<&Arc<VMArrayBuf>> {
        if let ArrayView::Generic(a) = self {
            Some(a)
        } else {
            None
        }
    }

    #[inline]
    pub fn iter_i64(&self) -> Option<std::slice::Iter<'_, i64>> {
        if let ArrayView::Int(a) = self {
            Some(a.iter())
        } else {
            None
        }
    }

    #[inline]
    pub fn iter_f64(&self) -> Option<std::slice::Iter<'_, f64>> {
        if let ArrayView::Float(a) = self {
            Some(a.as_slice().iter())
        } else {
            None
        }
    }
}

/// Mutable view over all array variants. Uses Arc::make_mut for COW semantics.
pub enum ArrayViewMut<'a> {
    Generic(&'a mut Arc<VMArrayBuf>),
    Int(&'a mut Arc<TypedBuffer<i64>>),
    Float(&'a mut Arc<AlignedTypedBuffer>),
    Bool(&'a mut Arc<TypedBuffer<u8>>),
}

impl ArrayViewMut<'_> {
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            ArrayViewMut::Generic(a) => a.len(),
            ArrayViewMut::Int(a) => a.len(),
            ArrayViewMut::Float(a) => a.len(),
            ArrayViewMut::Bool(a) => a.len(),
        }
    }

    pub fn pop_vw(&mut self) -> Option<ValueWord> {
        match self {
            ArrayViewMut::Generic(a) => Arc::make_mut(a).pop(),
            ArrayViewMut::Int(a) => Arc::make_mut(a).data.pop().map(ValueWord::from_i64),
            ArrayViewMut::Float(a) => Arc::make_mut(a).pop().map(ValueWord::from_f64),
            ArrayViewMut::Bool(a) => Arc::make_mut(a)
                .data
                .pop()
                .map(|b| ValueWord::from_bool(b != 0)),
        }
    }

    pub fn push_vw(&mut self, val: ValueWord) -> Result<(), crate::context::VMError> {
        match self {
            ArrayViewMut::Generic(a) => {
                Arc::make_mut(a).push(val);
                Ok(())
            }
            ArrayViewMut::Int(a) => {
                if let Some(i) = val.as_i64() {
                    Arc::make_mut(a).push(i);
                    Ok(())
                } else {
                    Err(crate::context::VMError::TypeError {
                        expected: "int",
                        got: val.type_name(),
                    })
                }
            }
            ArrayViewMut::Float(a) => {
                if let Some(f) = val.as_number_coerce() {
                    Arc::make_mut(a).push(f);
                    Ok(())
                } else {
                    Err(crate::context::VMError::TypeError {
                        expected: "number",
                        got: val.type_name(),
                    })
                }
            }
            ArrayViewMut::Bool(a) => {
                if let Some(b) = val.as_bool() {
                    Arc::make_mut(a).push(if b { 1 } else { 0 });
                    Ok(())
                } else {
                    Err(crate::context::VMError::TypeError {
                        expected: "bool",
                        got: val.type_name(),
                    })
                }
            }
        }
    }
}
