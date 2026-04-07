//! Enum layout registry for the v2 typed match dispatch path (Phase 3.3).
//!
//! When the bytecode compiler encounters an enum declaration it computes a
//! [`shape_value::v2::EnumLayout`] from the variant declarations and stores it
//! here, keyed by enum type name. The match-expression compiler then looks up
//! the layout to:
//!
//! - Map variant names to compile-time tag bytes (`variant_tag(name)`).
//! - Compute payload field byte offsets for `EnumPayloadField` operands.
//! - Eventually emit a tag-byte-driven jump table instead of `Eq`+`JumpIfFalse`
//!   chains.
//!
//! The registry is owned by [`crate::compiler::BytecodeCompiler`] and is empty
//! by default. Until [`BytecodeCompiler::use_typed_enum_dispatch`] is enabled
//! the registry is consulted but never required — the compiler still falls back
//! to the v1 string-based `__variant` schema dispatch path.

use std::collections::HashMap;

use shape_value::v2::EnumLayout;

/// In-memory registry of typed enum layouts keyed by enum type name.
///
/// This is a thin wrapper around `HashMap<String, EnumLayout>` so that the
/// public surface stays small and so future agents can swap in a
/// content-addressed or interner-backed implementation without touching
/// every call site.
#[derive(Debug, Default, Clone)]
pub struct EnumLayoutRegistry {
    layouts: HashMap<String, EnumLayout>,
}

impl EnumLayoutRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or overwrite) the layout for an enum type.
    ///
    /// The compiler calls this once per enum declaration after computing the
    /// layout. Re-registering the same name (e.g. when re-compiling a module)
    /// silently replaces the prior entry.
    pub fn register(&mut self, name: String, layout: EnumLayout) {
        self.layouts.insert(name, layout);
    }

    /// Look up the layout for an enum type by name.
    pub fn get(&self, name: &str) -> Option<&EnumLayout> {
        self.layouts.get(name)
    }

    /// Whether a layout is registered for the given enum type name.
    pub fn contains(&self, name: &str) -> bool {
        self.layouts.contains_key(name)
    }

    /// Number of registered enum layouts.
    pub fn len(&self) -> usize {
        self.layouts.len()
    }

    /// Whether the registry has no entries.
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }

    /// Iterator over all `(name, layout)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &EnumLayout)> {
        self.layouts.iter()
    }

    /// Remove every registered layout. Used by test helpers and incremental
    /// recompilation paths.
    pub fn clear(&mut self) {
        self.layouts.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::v2::enum_layout::compute_enum_layout;
    use shape_value::v2::struct_layout::FieldKind;

    fn color_layout() -> EnumLayout {
        compute_enum_layout(
            "Color",
            &[
                ("Red".to_string(), vec![]),
                ("Green".to_string(), vec![]),
                ("Blue".to_string(), vec![]),
            ],
        )
    }

    #[test]
    fn registry_starts_empty() {
        let r = EnumLayoutRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert!(r.get("Color").is_none());
        assert!(!r.contains("Color"));
    }

    #[test]
    fn register_then_get() {
        let mut r = EnumLayoutRegistry::new();
        r.register("Color".to_string(), color_layout());

        assert_eq!(r.len(), 1);
        assert!(r.contains("Color"));
        let layout = r.get("Color").expect("Color must be registered");
        assert_eq!(layout.variant_count(), 3);
        assert_eq!(layout.variant_tag("Red"), Some(0));
        assert_eq!(layout.variant_tag("Green"), Some(1));
        assert_eq!(layout.variant_tag("Blue"), Some(2));
    }

    #[test]
    fn register_overwrites_existing_entry() {
        let mut r = EnumLayoutRegistry::new();
        r.register("E".to_string(), color_layout());
        assert_eq!(r.get("E").unwrap().variant_count(), 3);

        // Re-register with a totally different shape.
        let two_variant = compute_enum_layout(
            "E",
            &[
                ("On".to_string(), vec![FieldKind::Bool]),
                ("Off".to_string(), vec![]),
            ],
        );
        r.register("E".to_string(), two_variant);
        assert_eq!(r.len(), 1);
        let layout = r.get("E").unwrap();
        assert_eq!(layout.variant_count(), 2);
        assert_eq!(layout.variant_tag("On"), Some(0));
        assert_eq!(layout.variant_tag("Off"), Some(1));
    }

    #[test]
    fn miss_returns_none() {
        let mut r = EnumLayoutRegistry::new();
        r.register("Color".to_string(), color_layout());
        assert!(r.get("Shape").is_none());
        assert!(!r.contains("Shape"));
    }

    #[test]
    fn payload_offsets_round_trip_through_registry() {
        // enum Shape { Circle(f64), Rectangle(f64, f64) }
        let layout = compute_enum_layout(
            "Shape",
            &[
                ("Circle".to_string(), vec![FieldKind::F64]),
                ("Rectangle".to_string(), vec![FieldKind::F64, FieldKind::F64]),
            ],
        );

        let mut r = EnumLayoutRegistry::new();
        r.register("Shape".to_string(), layout);

        let stored = r.get("Shape").unwrap();
        let circle = stored.variant_by_name("Circle").unwrap();
        assert_eq!(circle.tag, 0);
        assert_eq!(circle.field_offsets, vec![0]);

        let rectangle = stored.variant_by_name("Rectangle").unwrap();
        assert_eq!(rectangle.tag, 1);
        assert_eq!(rectangle.field_offsets, vec![0, 8]);
    }

    #[test]
    fn iter_yields_registered_entries() {
        let mut r = EnumLayoutRegistry::new();
        r.register("A".to_string(), color_layout());
        r.register("B".to_string(), color_layout());

        let names: std::collections::HashSet<&str> =
            r.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains("A"));
        assert!(names.contains("B"));
    }

    #[test]
    fn clear_removes_all_entries() {
        let mut r = EnumLayoutRegistry::new();
        r.register("Color".to_string(), color_layout());
        assert_eq!(r.len(), 1);
        r.clear();
        assert!(r.is_empty());
        assert!(r.get("Color").is_none());
    }

    /// Sanity check the concrete deliverable: registering Color { Red, Green, Blue }
    /// and verifying the registry holds it with three variants and tags 0, 1, 2.
    #[test]
    fn deliverable_color_three_variants() {
        let mut r = EnumLayoutRegistry::new();
        r.register("Color".to_string(), color_layout());

        let layout = r.get("Color").expect("Color must be registered");
        assert_eq!(layout.name, "Color");
        assert_eq!(layout.variant_count(), 3);
        assert_eq!(layout.variants[0].tag, 0);
        assert_eq!(layout.variants[1].tag, 1);
        assert_eq!(layout.variants[2].tag, 2);
        assert_eq!(layout.variants[0].name, "Red");
        assert_eq!(layout.variants[1].name, "Green");
        assert_eq!(layout.variants[2].name, "Blue");
    }

    /// Verify the registry and `use_typed_enum_dispatch` flag are wired into
    /// `BytecodeCompiler` and reachable from the surrounding code.
    ///
    /// This is the gate test for Phase 4 work: when the flag is enabled and a
    /// layout is registered, the match-expression compiler is supposed to take
    /// the new `EnumTagLoad`/`EnumPayloadField` path. The full compile pipeline
    /// is exercised by Phase 4 integration tests; this test just confirms the
    /// fields are in place and behave like a normal HashMap.
    #[test]
    fn bytecode_compiler_owns_registry_and_flag() {
        let mut compiler = crate::compiler::BytecodeCompiler::new();

        // Defaults: empty registry, gate off.
        assert!(compiler.enum_layouts.is_empty());
        assert!(!compiler.use_typed_enum_dispatch);

        // Register a Color layout via the compiler-owned registry.
        compiler
            .enum_layouts
            .register("Color".to_string(), color_layout());
        assert_eq!(compiler.enum_layouts.len(), 1);
        assert!(compiler.enum_layouts.contains("Color"));

        let stored = compiler
            .enum_layouts
            .get("Color")
            .expect("Color must be registered");
        assert_eq!(stored.variant_count(), 3);
        assert_eq!(stored.variant_tag("Red"), Some(0));
        assert_eq!(stored.variant_tag("Green"), Some(1));
        assert_eq!(stored.variant_tag("Blue"), Some(2));

        // Toggle the gate on/off — purely a flag, no other side effects.
        compiler.use_typed_enum_dispatch = true;
        assert!(compiler.use_typed_enum_dispatch);
        compiler.use_typed_enum_dispatch = false;
        assert!(!compiler.use_typed_enum_dispatch);
    }
}
