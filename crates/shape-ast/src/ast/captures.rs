//! ADR-009 ticket C1 — the **declared capture clause**.
//!
//! A closure literal may carry an explicit capture clause after a `;` inside
//! its parameter pipe:
//!
//! ```text
//! |acc, item; move cfg, share total| acc + item * cfg
//! |; move handle| handle.close()
//! ```
//!
//! # Surface scope (posted rider 1)
//!
//! The clause is a **generated-code-only** surface. An ordinary source closure
//! that carries one is a named compile-time rejection (`[C0903]`); source
//! closures keep capture inference. The clause exists so that comptime-produced
//! code — where the Wave-46 gate forbids *implicit* capture entirely — can say
//! what it means.
//!
//! # One carrier, two producers
//!
//! [`CaptureClause`] on `Expr::FunctionExpr` is the canonical carrier. Producer
//! #1 is the parser (this ticket). Producer #2 will be C2's `CheckedBody` /
//! closure-fragment staging, which populates the *same* field rather than
//! introducing a second mechanism. See `docs/defections.md`.
//!
//! # Declared word == emitted `CaptureKind`, always (rulings 1 + 2)
//!
//! There is deliberately no mode whose declared spelling differs from the
//! capture kind it lowers to:
//!
//! | declared    | binding                        | emitted `CaptureKind` |
//! |-------------|--------------------------------|-----------------------|
//! | `move`      | local `let`                    | `Immutable`           |
//! | `move`      | local `let mut`                | `OwnedMutable`        |
//! | `move`      | module binding                 | **`[C0906]` reject**  |
//! | `share`     | `var` / SharedCell / module    | `Shared`              |
//! | `share`     | plain local `let` / `let mut`  | **`[C0908]` reject**  |
//! | `&` / `&mut`| any                            | **`[C0902]` reject**  |
//!
//! If a declared mode and its lowered kind can ever disagree, the model is
//! wrong — do not add a `lowered != declared` field to "surface" the gap.

use serde::{Deserialize, Serialize};

use super::span::Span;

/// How a declared capture enters the closure.
///
/// FOUR arms (user ruling 2). `Move` never lies — there is no `Move -> Shared`
/// lowering anywhere; shared ownership has its own word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptureMode {
    /// `move x` — the closure takes the value. `let` snapshots (`Immutable`),
    /// `let mut` moves the owner into the closure's cell (`OwnedMutable`).
    Move,
    /// `share x` — the closure takes a share of an existing shared-ownership
    /// cell (`var`, a sibling-promoted `SharedCell`, or a module binding).
    /// Lowers to `CaptureKind::Shared`, always.
    Share,
    /// `&x` — reserved spelling. Rejected (`[C0902]`) until Shape has a region
    /// story: a borrow that escapes into a closure has no lifetime to check.
    SharedBorrow,
    /// `&mut x` — reserved spelling. Rejected (`[C0902]`), as `&x`.
    ExclusiveBorrow,
}

impl CaptureMode {
    /// The source spelling, for diagnostics. Round-trips through the parser.
    pub fn spelling(self) -> &'static str {
        match self {
            CaptureMode::Move => "move",
            CaptureMode::Share => "share",
            CaptureMode::SharedBorrow => "&",
            CaptureMode::ExclusiveBorrow => "&mut",
        }
    }

    /// True for the two reference spellings, which are a total rejection.
    pub fn is_borrow(self) -> bool {
        matches!(
            self,
            CaptureMode::SharedBorrow | CaptureMode::ExclusiveBorrow
        )
    }
}

/// One `<mode> <name>` entry of a capture clause.
///
/// `name` is a **syntactic reference** — exactly like `Expr::Identifier`. It is
/// resolved ONCE, at the capture gate, into a compiler-issued structural
/// `CaptureTarget` (a slot index), and thereafter survives only as diagnostic
/// prose. The declared-vs-discovered set diff is over targets, never names
/// (rework invariant R1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureEntry {
    pub mode: CaptureMode,
    pub name: String,
    pub span: Span,
}

/// The `; move a, share b` tail of a closure's parameter pipe.
///
/// An EMPTY clause (`|x;|`) is meaningful: it declares that the closure
/// captures nothing, and a closure that then captures something is a
/// used-but-undeclared error. `None` (no clause at all) is a different thing
/// entirely — in generated code it is the Wave-46 implicit-capture rejection,
/// and in source code it means "infer".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureClause {
    pub entries: Vec<CaptureEntry>,
    pub span: Span,
}

impl CaptureClause {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}
