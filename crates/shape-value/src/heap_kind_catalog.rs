//! Canonical ordinal catalog for [`HeapKind`].
//!
//! Cross-crate byte encodings must iterate this catalog rather than maintain a
//! second ordinal decoder. The compile-time assertions pin the `#[repr(u8)]`
//! order and reject omissions, duplicates, or gaps in the current catalog.

use crate::HeapKind;

impl HeapKind {
    /// Every `HeapKind` in `#[repr(u8)]` ordinal order.
    ///
    /// This is the canonical iteration surface for consumers that encode the
    /// discriminator as a compact ordinal. Adding a `HeapKind` requires adding
    /// it here in the same change; the compile-time ordinal proof below rejects
    /// an incomplete or misordered catalog.
    pub const ALL: [Self; 36] = [
        Self::String,
        Self::TypedObject,
        Self::Closure,
        Self::Decimal,
        Self::BigInt,
        Self::DataTable,
        Self::Future,
        Self::TaskGroup,
        Self::TypedArray,
        Self::Temporal,
        Self::TableView,
        Self::Content,
        Self::Instant,
        Self::IoHandle,
        Self::NativeScalar,
        Self::NativeView,
        Self::Char,
        Self::HashMap,
        Self::FilterExpr,
        Self::Reference,
        Self::SharedCell,
        Self::HashSet,
        Self::Iterator,
        Self::Deque,
        Self::Channel,
        Self::PriorityQueue,
        Self::Range,
        Self::Result,
        Self::Option,
        Self::TraitObject,
        Self::Mutex,
        Self::Atomic,
        Self::Lazy,
        Self::ModuleFn,
        Self::Matrix,
        Self::MatrixSlice,
    ];
}

const _: () = {
    assert!(HeapKind::ALL.len() == HeapKind::MatrixSlice as usize + 1);
    let mut ordinal = 0;
    while ordinal < HeapKind::ALL.len() {
        assert!(HeapKind::ALL[ordinal] as usize == ordinal);
        ordinal += 1;
    }
};
