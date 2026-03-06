//! Compile-time borrow checker for reference lifetime tracking.
//!
//! Enforces Rust-like aliasing rules:
//! - Shared refs (read-only): multiple `&` to same var allowed simultaneously
//! - Exclusive refs (mutating): only one `&` at a time; no other refs coexist
//! - References cannot escape their scope (no return, no store in array/object/closure)
//! - Original variable is frozen while borrowed

use shape_ast::ast::Span;
use shape_ast::error::{ErrorNote, ShapeError, SourceLocation};
use std::collections::{HashMap, HashSet};

/// Unique identifier for a lexical scope region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionId(pub u32);

/// Record of an active borrow.
#[derive(Debug, Clone)]
pub struct BorrowRecord {
    /// The local slot being borrowed (the original variable).
    pub borrowed_slot: u16,
    /// True if the callee mutates through this ref (exclusive borrow).
    pub is_exclusive: bool,
    /// The region where the borrowed variable was defined.
    pub origin_region: RegionId,
    /// The region where this borrow was created.
    pub borrow_region: RegionId,
    /// The local slot holding the reference value.
    pub ref_slot: u16,
    /// Source span for error reporting.
    pub span: Span,
    /// Source location for richer diagnostics.
    pub source_location: Option<SourceLocation>,
}

/// Borrow mode for a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowMode {
    Shared,
    Exclusive,
}

impl BorrowMode {
    fn is_exclusive(self) -> bool {
        matches!(self, Self::Exclusive)
    }
}

/// Compile-time borrow checker embedded in BytecodeCompiler.
///
/// Tracks active borrows per-slot and enforces aliasing rules.
/// Borrows are scoped to regions (lexical scopes) and automatically
/// released when their region exits.
pub struct BorrowChecker {
    /// Current region (innermost scope).
    current_region: RegionId,
    /// Stack of region IDs (for enter/exit).
    region_stack: Vec<RegionId>,
    /// Next region ID to allocate.
    next_region_id: u32,
    /// Active borrows per slot: slot -> list of active borrows.
    active_borrows: HashMap<u16, Vec<BorrowRecord>>,
    /// Slots with at least one exclusive (mutating) borrow.
    exclusively_borrowed: HashSet<u16>,
    /// Count of shared (non-mutating) borrows per slot.
    shared_borrow_count: HashMap<u16, u32>,
    /// Reference slots created in each region (for cleanup on scope exit).
    ref_slots_by_region: HashMap<RegionId, Vec<u16>>,
}

impl BorrowChecker {
    /// Create a new borrow checker starting at region 0 (module_binding scope).
    pub fn new() -> Self {
        Self {
            current_region: RegionId(0),
            region_stack: vec![RegionId(0)],
            next_region_id: 1,
            active_borrows: HashMap::new(),
            exclusively_borrowed: HashSet::new(),
            shared_borrow_count: HashMap::new(),
            ref_slots_by_region: HashMap::new(),
        }
    }

    /// Enter a new lexical scope (creates a new region).
    pub fn enter_region(&mut self) -> RegionId {
        let region = RegionId(self.next_region_id);
        self.next_region_id += 1;
        self.region_stack.push(region);
        self.current_region = region;
        region
    }

    /// Exit the current lexical scope, releasing all borrows created in it.
    pub fn exit_region(&mut self) {
        let exiting = self.current_region;

        // Release all borrows created in this region
        self.release_borrows_in_region(exiting);

        self.region_stack.pop();
        self.current_region = self.region_stack.last().copied().unwrap_or(RegionId(0));
    }

    /// Get the current region ID.
    pub fn current_region(&self) -> RegionId {
        self.current_region
    }

    /// Create a borrow of `slot` into `ref_slot`.
    ///
    /// If `is_exclusive` is true (callee mutates), enforces:
    /// - No other borrows (shared or exclusive) exist for `slot`
    ///
    /// If `is_exclusive` is false (callee reads only), enforces:
    /// - No exclusive borrows exist for `slot`
    pub fn create_borrow(
        &mut self,
        slot: u16,
        ref_slot: u16,
        mode: BorrowMode,
        span: Span,
        source_location: Option<SourceLocation>,
    ) -> Result<(), ShapeError> {
        if mode.is_exclusive() {
            // Exclusive borrow: no other borrows allowed
            if self.exclusively_borrowed.contains(&slot) {
                return Err(self.make_borrow_conflict_error(
                    "B0001",
                    slot,
                    source_location,
                    "cannot mutably borrow this value because it is already borrowed",
                    "end the previous borrow before creating a mutable borrow, or use a shared borrow",
                ));
            }
            if self.shared_borrow_count.get(&slot).copied().unwrap_or(0) > 0 {
                return Err(self.make_borrow_conflict_error(
                    "B0001",
                    slot,
                    source_location,
                    "cannot mutably borrow this value while shared borrows are active",
                    "move the mutable borrow later, or make prior borrows immutable-only reads",
                ));
            }
            self.exclusively_borrowed.insert(slot);
        } else {
            // Shared borrow: no exclusive borrows allowed
            if self.exclusively_borrowed.contains(&slot) {
                return Err(self.make_borrow_conflict_error(
                    "B0001",
                    slot,
                    source_location,
                    "cannot immutably borrow this value because it is mutably borrowed",
                    "drop the mutable borrow before taking an immutable borrow",
                ));
            }
            *self.shared_borrow_count.entry(slot).or_insert(0) += 1;
        }

        let record = BorrowRecord {
            borrowed_slot: slot,
            is_exclusive: mode.is_exclusive(),
            origin_region: self.current_region,
            borrow_region: self.current_region,
            ref_slot,
            span,
            source_location,
        };

        self.active_borrows.entry(slot).or_default().push(record);

        self.ref_slots_by_region
            .entry(self.current_region)
            .or_default()
            .push(slot);

        Ok(())
    }

    /// Check whether a write to `slot` is allowed (fails if any borrow exists).
    pub fn check_write_allowed(
        &self,
        slot: u16,
        source_location: Option<SourceLocation>,
    ) -> Result<(), ShapeError> {
        if let Some(borrows) = self.active_borrows.get(&slot) {
            if !borrows.is_empty() {
                return Err(self.make_borrow_conflict_error(
                    "B0002",
                    slot,
                    source_location,
                    "cannot write to this value while it is borrowed",
                    "move this write after the borrow ends",
                ));
            }
        }
        Ok(())
    }

    /// Check whether a direct read from `slot` is allowed.
    ///
    /// Reads are blocked while the slot has an active exclusive borrow.
    pub fn check_read_allowed(
        &self,
        slot: u16,
        source_location: Option<SourceLocation>,
    ) -> Result<(), ShapeError> {
        if self.exclusively_borrowed.contains(&slot) {
            return Err(self.make_borrow_conflict_error(
                "B0001",
                slot,
                source_location,
                "cannot read this value while it is mutably borrowed",
                "read through the existing reference, or move the read after the borrow ends",
            ));
        }
        Ok(())
    }

    /// Check that a reference does not escape its scope.
    /// Called when a ref_slot might be returned or stored.
    pub fn check_no_escape(
        &self,
        ref_slot: u16,
        source_location: Option<SourceLocation>,
    ) -> Result<(), ShapeError> {
        // Check if this ref_slot is in any active borrow
        for borrows in self.active_borrows.values() {
            for borrow in borrows {
                if borrow.ref_slot == ref_slot {
                    let mut location = source_location;
                    if let Some(loc) = location.as_mut() {
                        loc.hints.push(
                            "keep references within the call/lexical scope where they were created"
                                .to_string(),
                        );
                        loc.notes.push(ErrorNote {
                            message: "borrow originates here".to_string(),
                            location: borrow.source_location.clone(),
                        });
                    }
                    return Err(ShapeError::SemanticError {
                        message: "[B0003] reference cannot escape its scope".to_string(),
                        location,
                    });
                }
            }
        }
        Ok(())
    }

    /// Release all borrows created in a specific region.
    fn release_borrows_in_region(&mut self, region: RegionId) {
        if let Some(slots) = self.ref_slots_by_region.remove(&region) {
            for slot in slots {
                if let Some(borrows) = self.active_borrows.get_mut(&slot) {
                    borrows.retain(|b| b.borrow_region != region);

                    // Update exclusive/shared tracking
                    let has_exclusive = borrows.iter().any(|b| b.is_exclusive);
                    let shared_count = borrows.iter().filter(|b| !b.is_exclusive).count() as u32;

                    if !has_exclusive {
                        self.exclusively_borrowed.remove(&slot);
                    }
                    if shared_count == 0 {
                        self.shared_borrow_count.remove(&slot);
                    } else {
                        self.shared_borrow_count.insert(slot, shared_count);
                    }

                    if borrows.is_empty() {
                        self.active_borrows.remove(&slot);
                    }
                }
            }
        }
    }

    /// Reset the borrow checker state (e.g., when entering a new function body).
    pub fn reset(&mut self) {
        self.current_region = RegionId(0);
        self.region_stack = vec![RegionId(0)];
        self.next_region_id = 1;
        self.active_borrows.clear();
        self.exclusively_borrowed.clear();
        self.shared_borrow_count.clear();
        self.ref_slots_by_region.clear();
    }

    fn first_conflicting_borrow(&self, slot: u16) -> Option<&BorrowRecord> {
        self.active_borrows
            .get(&slot)
            .and_then(|borrows| borrows.first())
    }

    fn make_borrow_conflict_error(
        &self,
        code: &str,
        slot: u16,
        source_location: Option<SourceLocation>,
        message: &str,
        help: &str,
    ) -> ShapeError {
        let mut location = source_location;
        if let Some(loc) = location.as_mut() {
            loc.hints.push(help.to_string());
            if let Some(conflict) = self.first_conflicting_borrow(slot) {
                loc.notes.push(ErrorNote {
                    message: "first conflicting borrow occurs here".to_string(),
                    location: conflict.source_location.clone(),
                });
            }
        }
        ShapeError::SemanticError {
            message: format!("[{}] {} (slot {})", code, message, slot),
            location,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span { start: 0, end: 1 }
    }

    #[test]
    fn test_single_exclusive_borrow_ok() {
        let mut bc = BorrowChecker::new();
        assert!(
            bc.create_borrow(0, 0, BorrowMode::Exclusive, span(), None)
                .is_ok()
        );
    }

    #[test]
    fn test_double_exclusive_borrow_rejected() {
        let mut bc = BorrowChecker::new();
        bc.create_borrow(0, 0, BorrowMode::Exclusive, span(), None)
            .unwrap();
        let err = bc.create_borrow(0, 1, BorrowMode::Exclusive, span(), None);
        assert!(err.is_err());
        let msg = format!("{:?}", err.unwrap_err());
        assert!(msg.contains("[B0001]"), "got: {}", msg);
    }

    #[test]
    fn test_multiple_shared_borrows_ok() {
        let mut bc = BorrowChecker::new();
        assert!(
            bc.create_borrow(0, 0, BorrowMode::Shared, span(), None)
                .is_ok()
        );
        assert!(
            bc.create_borrow(0, 1, BorrowMode::Shared, span(), None)
                .is_ok()
        );
        assert!(
            bc.create_borrow(0, 2, BorrowMode::Shared, span(), None)
                .is_ok()
        );
    }

    #[test]
    fn test_exclusive_after_shared_rejected() {
        let mut bc = BorrowChecker::new();
        bc.create_borrow(0, 0, BorrowMode::Shared, span(), None)
            .unwrap();
        let err = bc.create_borrow(0, 1, BorrowMode::Exclusive, span(), None);
        assert!(err.is_err());
        let msg = format!("{:?}", err.unwrap_err());
        assert!(msg.contains("[B0001]"), "got: {}", msg);
    }

    #[test]
    fn test_shared_after_exclusive_rejected() {
        let mut bc = BorrowChecker::new();
        bc.create_borrow(0, 0, BorrowMode::Exclusive, span(), None)
            .unwrap();
        let err = bc.create_borrow(0, 1, BorrowMode::Shared, span(), None);
        assert!(err.is_err());
        let msg = format!("{:?}", err.unwrap_err());
        assert!(msg.contains("[B0001]"), "got: {}", msg);
    }

    #[test]
    fn test_write_blocked_while_borrowed() {
        let bc_shared = {
            let mut bc = BorrowChecker::new();
            bc.create_borrow(0, 0, BorrowMode::Shared, span(), None)
                .unwrap();
            bc
        };
        let err = bc_shared.check_write_allowed(0, None);
        assert!(err.is_err());
        let msg = format!("{:?}", err.unwrap_err());
        assert!(msg.contains("[B0002]"), "got: {}", msg);
    }

    #[test]
    fn test_write_allowed_when_no_borrows() {
        let bc = BorrowChecker::new();
        assert!(bc.check_write_allowed(0, None).is_ok());
    }

    #[test]
    fn test_borrows_released_on_scope_exit() {
        let mut bc = BorrowChecker::new();
        bc.enter_region();
        bc.create_borrow(0, 0, BorrowMode::Exclusive, span(), None)
            .unwrap();
        // Write blocked while borrowed
        assert!(bc.check_write_allowed(0, None).is_err());
        // Exit scope → borrow released
        bc.exit_region();
        assert!(bc.check_write_allowed(0, None).is_ok());
        // Can borrow again after release
        assert!(
            bc.create_borrow(0, 1, BorrowMode::Exclusive, span(), None)
                .is_ok()
        );
    }

    #[test]
    fn test_nested_scopes() {
        let mut bc = BorrowChecker::new();
        bc.enter_region(); // region 1
        bc.create_borrow(0, 0, BorrowMode::Exclusive, span(), None)
            .unwrap();
        bc.enter_region(); // region 2
        bc.create_borrow(1, 1, BorrowMode::Exclusive, span(), None)
            .unwrap();
        // slot 0 still borrowed
        assert!(bc.check_write_allowed(0, None).is_err());
        bc.exit_region(); // exit region 2 → slot 1 released
        assert!(bc.check_write_allowed(1, None).is_ok());
        // slot 0 still borrowed (region 1 still active)
        assert!(bc.check_write_allowed(0, None).is_err());
        bc.exit_region(); // exit region 1 → slot 0 released
        assert!(bc.check_write_allowed(0, None).is_ok());
    }

    #[test]
    fn test_different_slots_independent() {
        let mut bc = BorrowChecker::new();
        bc.create_borrow(0, 0, BorrowMode::Exclusive, span(), None)
            .unwrap();
        // Different slot is fine
        assert!(
            bc.create_borrow(1, 1, BorrowMode::Exclusive, span(), None)
                .is_ok()
        );
        assert!(bc.check_write_allowed(1, None).is_err());
        assert!(bc.check_write_allowed(2, None).is_ok());
    }

    #[test]
    fn test_check_no_escape() {
        let mut bc = BorrowChecker::new();
        bc.create_borrow(0, 5, BorrowMode::Exclusive, span(), None)
            .unwrap();
        // ref_slot 5 should not escape
        assert!(bc.check_no_escape(5, None).is_err());
        // ref_slot 99 is not in any borrow
        assert!(bc.check_no_escape(99, None).is_ok());
    }

    #[test]
    fn test_reset_clears_all_state() {
        let mut bc = BorrowChecker::new();
        bc.enter_region();
        bc.create_borrow(0, 0, BorrowMode::Exclusive, span(), None)
            .unwrap();
        bc.reset();
        // All borrows cleared
        assert!(bc.check_write_allowed(0, None).is_ok());
        assert!(
            bc.create_borrow(0, 0, BorrowMode::Exclusive, span(), None)
                .is_ok()
        );
    }

    #[test]
    fn test_region_ids_are_unique() {
        let mut bc = BorrowChecker::new();
        let r1 = bc.enter_region();
        let r2 = bc.enter_region();
        assert_ne!(r1, r2);
        bc.exit_region();
        let r3 = bc.enter_region();
        assert_ne!(r2, r3);
        assert_ne!(r1, r3);
    }

    #[test]
    fn test_error_carries_source_location() {
        let mut bc = BorrowChecker::new();
        let loc = SourceLocation::new(10, 5);
        bc.create_borrow(0, 0, BorrowMode::Exclusive, span(), Some(loc.clone()))
            .unwrap();
        let err = bc.create_borrow(0, 1, BorrowMode::Exclusive, span(), Some(loc));
        match err {
            Err(ShapeError::SemanticError { location, .. }) => {
                let loc = location.expect("error should carry source location");
                assert_eq!(loc.line, 10);
                assert_eq!(loc.column, 5);
            }
            other => panic!("expected SemanticError, got: {:?}", other),
        }
    }
}
