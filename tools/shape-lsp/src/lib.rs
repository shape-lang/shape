#![allow(clippy::result_large_err)]

//! Shape Language Server Protocol (LSP) implementation
//!
//! This crate provides a fully-featured LSP server for the Shape language,
//! enabling rich IDE support including diagnostics, completion, hover information,
//! go-to-definition, and more.

pub mod analysis;
pub mod annotation_discovery;
pub mod call_hierarchy;
pub mod code_actions;
pub mod code_lens;
pub mod completion;
pub mod context;
pub mod definition;
pub mod diagnostics;
pub mod document;
pub mod document_symbols;
pub mod folding;
pub mod foreign_lsp;
pub mod formatting;
pub mod grammar_completion;
pub mod hover;
pub mod inlay_hints;
pub mod module_cache;
pub mod rename;
pub mod scope;
pub mod semantic_tokens;
pub mod server;
pub mod signature_help;
pub mod symbols;
pub mod toml_support;
pub mod trait_lookup;
pub mod type_inference;
pub(crate) mod util;

// Re-export main types
pub use document::DocumentManager;
pub use module_cache::ModuleCache;
pub use server::ShapeLanguageServer;
