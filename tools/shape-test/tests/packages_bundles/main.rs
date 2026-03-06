//! Integration tests for packages and bundles.
//!
//! Shape packages use shape.toml manifests and content-addressed bytecode
//! bundles. Most tests are TDD since the shapec compiler tooling is not
//! yet exposed through the ShapeTest builder.

mod basic;
