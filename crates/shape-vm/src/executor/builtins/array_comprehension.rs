//! Closure-driven array comprehension builtins (Wave 5d scope).
//!
//! `map`, `filter`, `reduce`, `forEach`, `find`, `findIndex`, `some`, `every`
//! are deferred to Wave 5d body migration. The previous bodies called into
//! deleted `ValueWord` / `ValueWordExt` / `ArgVec` machinery and are now
//! dead under the `todo!`-stubbed dispatch (`vm_impl/builtins.rs`'s
//! Wave 5d arms).
//!
//! This file is empty post-Wave 5b. Kept as a module placeholder so
//! `mod array_comprehension;` keeps resolving until the deferred bodies
//! land in Wave 5d.
