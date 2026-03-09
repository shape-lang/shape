//! Shape Wire Format
//!
//! Universal serialization format for Shape values with type metadata.
//! This crate provides the data exchange format for:
//! - REPL communication (engine <-> CLI)
//! - fchart interoperability
//! - Plugin data exchange
//! - External tool integration
//!
//! # Design Goals
//!
//! 1. **Fast**: Uses MessagePack for compact binary serialization
//! 2. **Type-aware**: Carries type metadata for proper display/parsing
//! 3. **Format-flexible**: Supports multiple display formats per type
//! 4. **Lossless**: Raw values can be round-tripped without data loss

pub mod any_error;
pub mod codec;
pub mod envelope;
pub mod error;
pub mod formatter;
pub mod metadata;
pub mod print_result;
pub mod render;
pub mod transport;
pub mod value;

pub use any_error::{
    AnsiAnyErrorRenderer, AnyError, AnyErrorFrame, AnyErrorRenderer, HtmlAnyErrorRenderer,
    PlainAnyErrorRenderer, render_any_error_ansi, render_any_error_html, render_any_error_plain,
    render_any_error_terminal, render_any_error_with,
};
pub use codec::{
    decode, decode_message, encode, encode_message, from_json, from_json_string, to_json,
    to_json_string, to_json_string_pretty,
};
pub use envelope::ValueEnvelope;
pub use error::{Result, WireError};
pub use formatter::{format_value, parse_value};
pub use metadata::{FieldInfo, TypeInfo, TypeKind, TypeMetadata, TypeRegistry};
pub use print_result::{WirePrintResult, WirePrintSpan};
pub use render::{
    AnyErrorWireRenderAdapter, TerminalRenderCaps, WireRenderAdapter, WireRenderer,
    render_wire_html, render_wire_terminal,
};
pub use value::{DurationUnit, WireColumn, WireTable, WireValue};

/// Re-export content types used in `WireValue::Content`.
pub use shape_value::content::{self as content, ContentNode};

/// Wire protocol version constant.
///
/// Used by external tools (e.g. shape-mcp) to verify protocol compatibility
/// with the shape CLI. Bump this when the wire framing or message format changes
/// in a backward-incompatible way.
pub const WIRE_PROTOCOL_V1: u32 = 1;

/// Wire protocol version 2: adds Execute, Validate, Auth, Ping/Pong messages
/// and JSON framing support for lightweight clients.
pub const WIRE_PROTOCOL_V2: u32 = 2;
