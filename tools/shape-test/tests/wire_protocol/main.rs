//! Integration tests for the wire protocol.
//!
//! Shape uses a length-prefixed MessagePack wire format for inter-process
//! communication. Most tests are TDD since the wire protocol is
//! infrastructure-level and not directly exposed through ShapeTest.

mod encoding;
mod format;
