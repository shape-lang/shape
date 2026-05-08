//! Renderers for [`crate::Diagnostic`].
//!
//! Each renderer consumes an LSDS [`crate::Diagnostic`] and produces a
//! representation suited to its medium. **The renderer never modifies the
//! diagnostic**; it is read-only output formatting.
//!
//! # Renderers
//!
//! - [`terminal`] — human-readable plain-text output. Approximates the
//!   shape of today's `ShapeError::SemanticError` text rendering for
//!   borrow errors.
//!
//! Future renderers (post first session) per ADR-006 §9.1:
//!
//! - `lsp` — produces `lsp_types::Diagnostic`.
//! - `mcp` — produces structured MCP tool responses.

pub mod terminal;
