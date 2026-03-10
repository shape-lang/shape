//! Core MIR types: Place, Statement, Terminator, BasicBlock.
//!
//! These represent the mid-level IR that the borrow solver operates on.
//! Places track what can be borrowed (locals, fields, indices).
//! Statements and terminators form basic blocks in a control flow graph.

use shape_ast::ast::Span;
use std::fmt;

// ── Identifiers ──────────────────────────────────────────────────────

/// Index of a local variable slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SlotId(pub u16);

/// Index of a struct/object field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldIdx(pub u16);

/// Index of a basic block within a MIR function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BasicBlockId(pub u32);

/// A program point (statement index within the function's linearized MIR).
/// Used as the "point" dimension in Datafrog relations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Point(pub u32);

/// Unique identifier for a loan (borrow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LoanId(pub u32);

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "_{}", self.0)
    }
}

impl fmt::Display for BasicBlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "p{}", self.0)
    }
}

impl fmt::Display for LoanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "L{}", self.0)
    }
}

// ── Places ───────────────────────────────────────────────────────────

/// A place is something that can be borrowed or assigned to.
/// Tracks granular access paths for disjoint borrow analysis.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Place {
    /// A local variable: `x`
    Local(SlotId),
    /// A field of a place: `x.field_name`
    Field(Box<Place>, FieldIdx),
    /// An index into a place: `x[i]` — index analysis is conservative in v1.
    /// The index operand is boxed to break the recursive type cycle (Place → Operand → Place).
    Index(Box<Place>, Box<Operand>),
    /// Dereferencing a reference: `*r`
    Deref(Box<Place>),
}

impl Place {
    /// Get the root local of this place (e.g., `x.a.b` → `x`).
    pub fn root_local(&self) -> SlotId {
        match self {
            Place::Local(slot) => *slot,
            Place::Field(base, _) | Place::Index(base, _) | Place::Deref(base) => base.root_local(),
        }
    }

    /// Check if this place is a prefix of another (for conflict detection).
    /// `x` is a prefix of `x.a`, `x` is a prefix of `x[i]`, etc.
    pub fn is_prefix_of(&self, other: &Place) -> bool {
        if self == other {
            return true;
        }
        match other {
            Place::Local(_) => false,
            Place::Field(base, _) | Place::Index(base, _) | Place::Deref(base) => {
                self.is_prefix_of(base)
            }
        }
    }

    /// Check whether two places conflict (one borrows/writes something the other uses).
    /// Two places conflict if one is a prefix of the other, or they're the same.
    /// In v1, disjoint field borrows are tracked (x.a and x.b don't conflict),
    /// but index borrows are conservative (x[i] and x[j] always conflict).
    pub fn conflicts_with(&self, other: &Place) -> bool {
        // Same root?
        if self.root_local() != other.root_local() {
            return false;
        }
        // Walk both paths to check overlap
        self.is_prefix_of(other) || other.is_prefix_of(self) || self.overlaps(other)
    }

    fn overlaps(&self, other: &Place) -> bool {
        match (self, other) {
            (Place::Local(a), Place::Local(b)) => a == b,
            // Disjoint fields: x.a and x.b do NOT conflict
            (Place::Field(base_a, field_a), Place::Field(base_b, field_b)) => {
                if base_a == base_b {
                    field_a == field_b
                } else {
                    base_a.overlaps(base_b)
                }
            }
            // Conservative: x[i] and x[j] always conflict
            (Place::Index(base_a, _), Place::Index(base_b, _)) => base_a.overlaps(base_b),
            _ => self.is_prefix_of(other) || other.is_prefix_of(self),
        }
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Place::Local(slot) => write!(f, "{}", slot),
            Place::Field(base, field) => write!(f, "{}.{}", base, field.0),
            Place::Index(base, idx) => write!(f, "{}[{}]", base, idx),
            Place::Deref(base) => write!(f, "*{}", base),
        }
    }
}

// ── Operands ─────────────────────────────────────────────────────────

/// An operand in an Rvalue or terminator.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Operand {
    /// Copy the value from a place (for Copy types).
    Copy(Place),
    /// Move the value from a place (invalidates the source).
    Move(Place),
    /// Explicit source-level move (`move x`) that must not be rewritten into a clone.
    MoveExplicit(Place),
    /// A constant value.
    Constant(MirConstant),
}

impl fmt::Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Copy(p) => write!(f, "copy {}", p),
            Operand::Move(p) => write!(f, "move {}", p),
            Operand::MoveExplicit(p) => write!(f, "move! {}", p),
            Operand::Constant(c) => write!(f, "{}", c),
        }
    }
}

/// A constant value in MIR.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MirConstant {
    Int(i64),
    Bool(bool),
    None,
    /// String interned index
    StringId(u32),
    /// Float (stored as bits for Eq/Hash)
    Float(u64),
    /// Function reference by name
    Function(String),
}

impl fmt::Display for MirConstant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MirConstant::Int(v) => write!(f, "{}", v),
            MirConstant::Bool(v) => write!(f, "{}", v),
            MirConstant::None => write!(f, "none"),
            MirConstant::StringId(id) => write!(f, "str#{}", id),
            MirConstant::Float(bits) => write!(f, "{}", f64::from_bits(*bits)),
            MirConstant::Function(name) => write!(f, "fn:{}", name),
        }
    }
}

// ── Rvalues ──────────────────────────────────────────────────────────

/// The kind of borrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BorrowKind {
    /// Shared (immutable) borrow: `&x`
    Shared,
    /// Exclusive (mutable) borrow: `&mut x`
    Exclusive,
}

impl fmt::Display for BorrowKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BorrowKind::Shared => write!(f, "&"),
            BorrowKind::Exclusive => write!(f, "&mut"),
        }
    }
}

/// Right-hand side of an assignment.
#[derive(Debug, Clone, PartialEq)]
pub enum Rvalue {
    /// Use an operand directly.
    Use(Operand),
    /// Create a borrow: `&place` or `&mut place`
    Borrow(BorrowKind, Place),
    /// Binary operation.
    BinaryOp(BinOp, Operand, Operand),
    /// Unary operation.
    UnaryOp(UnOp, Operand),
    /// Function call result (arguments passed via terminator).
    /// This is a placeholder — actual calls use Call terminators.
    Aggregate(Vec<Operand>),
    /// Clone of a value (explicit or auto-inferred).
    Clone(Operand),
}

/// Binary operations in MIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// Unary operations in MIR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

// ── Statements ───────────────────────────────────────────────────────

/// A statement within a basic block (doesn't affect control flow).
#[derive(Debug, Clone, PartialEq)]
pub struct MirStatement {
    pub kind: StatementKind,
    pub span: Span,
    /// The program point of this statement (assigned during linearization).
    pub point: Point,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatementKind {
    /// Assign a value to a place: `place = rvalue`
    Assign(Place, Rvalue),
    /// Drop a place (scope exit, explicit drop).
    /// Generates invalidation facts for any loans on this place.
    Drop(Place),
    /// Cross a task boundary (spawn/join branch capture).
    /// Operands are the values flowing into the spawned task.
    TaskBoundary(Vec<Operand>),
    /// Capture values into a closure environment.
    /// Operands are the outer values flowing into the closure.
    ClosureCapture(Vec<Operand>),
    /// Store values into an array literal.
    /// Operands are the array elements being stored.
    ArrayStore(Vec<Operand>),
    /// No-op (placeholder, padding).
    Nop,
}

// ── Terminators ──────────────────────────────────────────────────────

/// A block terminator (controls flow between basic blocks).
#[derive(Debug, Clone, PartialEq)]
pub struct Terminator {
    pub kind: TerminatorKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerminatorKind {
    /// Unconditional jump.
    Goto(BasicBlockId),
    /// Conditional branch.
    SwitchBool {
        operand: Operand,
        true_bb: BasicBlockId,
        false_bb: BasicBlockId,
    },
    /// Function call.
    Call {
        func: Operand,
        args: Vec<Operand>,
        /// Where to store the return value.
        destination: Place,
        /// Block to jump to after the call returns.
        next: BasicBlockId,
    },
    /// Return from function.
    Return,
    /// Unreachable (after diverging calls, infinite loops).
    Unreachable,
}

// ── Basic Blocks ─────────────────────────────────────────────────────

/// A basic block: a sequence of statements ending in a terminator.
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BasicBlockId,
    pub statements: Vec<MirStatement>,
    pub terminator: Terminator,
}

// ── MIR Function ─────────────────────────────────────────────────────

/// The MIR representation of a single function.
#[derive(Debug, Clone)]
pub struct MirFunction {
    pub name: String,
    /// The basic blocks forming the CFG.
    pub blocks: Vec<BasicBlock>,
    /// Number of local variable slots.
    pub num_locals: u16,
    /// Which locals are function parameters.
    pub param_slots: Vec<SlotId>,
    /// Type information for locals (for Copy/Clone inference).
    pub local_types: Vec<LocalTypeInfo>,
    /// Source span of the function.
    pub span: Span,
}

/// Type information for a local variable, used for Copy/Clone inference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalTypeInfo {
    /// Primitive (int, number, bool, none) — implicitly Copy, no borrow tracking.
    Copy,
    /// Heap type (String, Array, TypedObject, etc.) — requires borrow/move/clone tracking.
    NonCopy,
    /// Unknown type (will be resolved during analysis).
    Unknown,
}

impl MirFunction {
    /// Get the entry block (always block 0).
    pub fn entry_block(&self) -> BasicBlockId {
        BasicBlockId(0)
    }

    /// Iterate over all blocks.
    pub fn iter_blocks(&self) -> impl Iterator<Item = &BasicBlock> {
        self.blocks.iter()
    }

    /// Get a block by ID.
    pub fn block(&self, id: BasicBlockId) -> &BasicBlock {
        &self.blocks[id.0 as usize]
    }

    /// Linearize all statements into a flat list of points.
    /// Returns (point, block_id, statement_index) triples.
    pub fn all_points(&self) -> Vec<(Point, BasicBlockId, usize)> {
        let mut points = Vec::new();
        for block in &self.blocks {
            for (i, stmt) in block.statements.iter().enumerate() {
                points.push((stmt.point, block.id, i));
            }
        }
        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_place_root_local() {
        let p = Place::Field(Box::new(Place::Local(SlotId(0))), FieldIdx(1));
        assert_eq!(p.root_local(), SlotId(0));
    }

    #[test]
    fn test_place_prefix() {
        let x = Place::Local(SlotId(0));
        let xa = Place::Field(Box::new(Place::Local(SlotId(0))), FieldIdx(0));
        assert!(x.is_prefix_of(&xa));
        assert!(!xa.is_prefix_of(&x));
    }

    #[test]
    fn test_disjoint_fields_no_conflict() {
        let xa = Place::Field(Box::new(Place::Local(SlotId(0))), FieldIdx(0));
        let xb = Place::Field(Box::new(Place::Local(SlotId(0))), FieldIdx(1));
        // Disjoint fields should not overlap
        assert!(!xa.overlaps(&xb));
    }

    #[test]
    fn test_same_field_conflicts() {
        let xa1 = Place::Field(Box::new(Place::Local(SlotId(0))), FieldIdx(0));
        let xa2 = Place::Field(Box::new(Place::Local(SlotId(0))), FieldIdx(0));
        assert!(xa1.conflicts_with(&xa2));
    }

    #[test]
    fn test_different_locals_no_conflict() {
        let x = Place::Local(SlotId(0));
        let y = Place::Local(SlotId(1));
        assert!(!x.conflicts_with(&y));
    }

    #[test]
    fn test_parent_child_conflict() {
        let x = Place::Local(SlotId(0));
        let xa = Place::Field(Box::new(Place::Local(SlotId(0))), FieldIdx(0));
        assert!(x.conflicts_with(&xa));
        assert!(xa.conflicts_with(&x));
    }
}
