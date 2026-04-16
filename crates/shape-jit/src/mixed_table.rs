//! Mixed function table supporting both JIT-compiled and interpreter-fallback entries.
//!
//! When per-blob JIT preflight determines that some functions cannot be JIT-compiled
//! (e.g. they use async opcodes or unsupported builtins), we still want to JIT-compile
//! the functions that *can* be compiled. The `MixedFunctionTable` maps each function
//! to either a native code pointer or a marker indicating VM interpretation.

use shape_vm::bytecode::FunctionHash;
use shape_value::ValueWordExt;
use std::collections::HashMap;

/// Entry in the mixed function table supporting both JIT and interpreted functions.
#[derive(Debug, Clone)]
pub enum FunctionEntry {
    /// JIT-compiled native function pointer.
    Native(*const u8),
    /// Falls back to VM interpreter for this function.
    /// The `u16` is the function index in the linked program.
    Interpreted(u16),
    /// Awaiting background compilation.
    /// The `u16` is the function index in the linked program.
    Pending(u16),
}

// SAFETY: Function pointers from JIT are valid for the lifetime of the JITModule
// that produced them. The caller must ensure the JITModule outlives the table.
unsafe impl Send for FunctionEntry {}

/// Mixed function table mapping function IDs to either native or interpreted entries.
///
/// Supports lookup by both numeric index (for the flat instruction array) and
/// content hash (for the content-addressed blob store).
pub struct MixedFunctionTable {
    entries: Vec<FunctionEntry>,
    hash_to_entry: HashMap<FunctionHash, usize>,
}

impl MixedFunctionTable {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            hash_to_entry: HashMap::new(),
        }
    }

    /// Pre-allocate space for `capacity` function entries.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            hash_to_entry: HashMap::with_capacity(capacity),
        }
    }

    /// Insert or replace an entry at the given index.
    ///
    /// If `id` is beyond the current length, intermediate slots are filled
    /// with `Interpreted(0)` placeholders.
    pub fn insert(&mut self, id: usize, entry: FunctionEntry) {
        if id >= self.entries.len() {
            self.entries.resize(id + 1, FunctionEntry::Interpreted(0));
        }
        self.entries[id] = entry;
    }

    /// Look up an entry by numeric function index.
    pub fn get(&self, id: usize) -> Option<&FunctionEntry> {
        self.entries.get(id)
    }

    /// Insert an entry keyed by content hash, also storing it at the given index.
    pub fn insert_by_hash(&mut self, hash: FunctionHash, entry: FunctionEntry) {
        let id = self.entries.len();
        self.entries.push(entry);
        self.hash_to_entry.insert(hash, id);
    }

    /// Look up an entry by content hash.
    pub fn get_by_hash(&self, hash: &FunctionHash) -> Option<&FunctionEntry> {
        self.hash_to_entry
            .get(hash)
            .and_then(|&id| self.entries.get(id))
    }

    /// Total number of entries in the table.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the table contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Count of entries that are JIT-compiled native code.
    pub fn native_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, FunctionEntry::Native(_)))
            .count()
    }

    /// Count of entries that fall back to the VM interpreter.
    pub fn interpreted_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, FunctionEntry::Interpreted(_)))
            .count()
    }

    /// Count of entries that are awaiting background compilation.
    pub fn pending_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, FunctionEntry::Pending(_)))
            .count()
    }

    /// Iterate over all entries with their index.
    pub fn iter(&self) -> impl Iterator<Item = (usize, &FunctionEntry)> {
        self.entries.iter().enumerate()
    }

    /// Promote a pending entry to native after background compilation completes.
    pub fn promote_to_native(&mut self, id: usize, ptr: *const u8) {
        if id < self.entries.len() {
            self.entries[id] = FunctionEntry::Native(ptr);
        }
    }
}

impl Default for MixedFunctionTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_table() {
        let table = MixedFunctionTable::new();
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);
        assert_eq!(table.native_count(), 0);
        assert_eq!(table.interpreted_count(), 0);
    }

    #[test]
    fn insert_and_get() {
        let mut table = MixedFunctionTable::new();
        let fake_ptr = 0xDEAD_BEEF as *const u8;
        table.insert(0, FunctionEntry::Native(fake_ptr));
        table.insert(1, FunctionEntry::Interpreted(1));
        table.insert(2, FunctionEntry::Pending(2));

        assert_eq!(table.len(), 3);
        assert_eq!(table.native_count(), 1);
        assert_eq!(table.interpreted_count(), 1);
        assert_eq!(table.pending_count(), 1);

        assert!(matches!(table.get(0), Some(FunctionEntry::Native(_))));
        assert!(matches!(table.get(1), Some(FunctionEntry::Interpreted(1))));
        assert!(matches!(table.get(2), Some(FunctionEntry::Pending(2))));
        assert!(table.get(3).is_none());
    }

    #[test]
    fn insert_by_hash_and_lookup() {
        let mut table = MixedFunctionTable::new();
        let hash = FunctionHash([42u8; 32]);
        let fake_ptr = 0xCAFE as *const u8;
        table.insert_by_hash(hash, FunctionEntry::Native(fake_ptr));

        assert!(matches!(
            table.get_by_hash(&hash),
            Some(FunctionEntry::Native(_))
        ));
        assert!(table.get_by_hash(&FunctionHash::ZERO).is_none());
    }

    #[test]
    fn promote_pending_to_native() {
        let mut table = MixedFunctionTable::new();
        table.insert(0, FunctionEntry::Pending(0));
        assert_eq!(table.pending_count(), 1);

        let fake_ptr = 0xBEEF as *const u8;
        table.promote_to_native(0, fake_ptr);
        assert_eq!(table.pending_count(), 0);
        assert_eq!(table.native_count(), 1);
    }

    #[test]
    fn sparse_insert_fills_gaps() {
        let mut table = MixedFunctionTable::new();
        table.insert(5, FunctionEntry::Native(0x1 as *const u8));
        assert_eq!(table.len(), 6);
        // Slots 0-4 should be Interpreted(0) placeholders.
        assert!(matches!(table.get(0), Some(FunctionEntry::Interpreted(0))));
        assert!(matches!(table.get(4), Some(FunctionEntry::Interpreted(0))));
        assert!(matches!(table.get(5), Some(FunctionEntry::Native(_))));
    }
}
