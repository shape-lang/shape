use super::super::*;

impl VirtualMachine {
    // ========================================================================
    // Conversion and Helper Methods

    /// Create a TypedObject from field name-value pairs.
    ///
    /// Phase-2c surface (ADR-006 §2.7.4): the legacy `(field_name, ValueWord)`
    /// pair carrier and `ValueWord::from_heap_value(HeapValue::TypedObject{..})`
    /// return shape both depend on the deleted `ValueWord` runtime
    /// representation. The kinded rebuild takes `&[(&str, KindedSlot)]` /
    /// `&[(&str, u64, NativeKind)]` and returns
    /// `Arc<TypedObjectStorage>` directly (parallel-kind track + Arc
    /// payload per ADR-006 §2.7 / Q7).
    ///
    /// Until the host-API rebuild lands, the lone caller
    /// (`op_new_object` in `objects/object_creation.rs:168`) drains the
    /// popped key/value pairs and surfaces `VMError::NotImplemented`. The
    /// signature here is removed entirely so reintroducing a ValueWord-
    /// shaped pairs API requires re-adding the function (review-visible).
    pub(crate) fn create_typed_object_from_pairs_stub(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "create_typed_object_from_pairs: ad-hoc TypedObject construction \
             from `(field_name, ValueWord)` pairs depends on the deleted \
             ValueWord carrier and the deleted `from_heap_value` constructor. \
             The kinded rebuild (Arc<TypedObjectStorage> + parallel-kind \
             track) is Phase-2c — see ADR-006 §2.7.4."
                .to_string(),
        ))
    }

    /// Look up a schema by ID in the compiled program registry.
    pub(crate) fn lookup_schema(
        &self,
        schema_id: u32,
    ) -> Option<&shape_runtime::type_schema::TypeSchema> {
        self.program.type_schema_registry.get_by_id(schema_id)
    }

    pub(crate) fn lookup_schema_by_name(
        &self,
        name: &str,
    ) -> Option<&shape_runtime::type_schema::TypeSchema> {
        self.program.type_schema_registry.get(name)
    }

    /// Derive a merged schema from two existing schemas.
    /// Right fields overwrite left fields with the same name.
    /// Caches result by (left_schema_id, right_schema_id).
    pub(crate) fn derive_merged_schema(
        &mut self,
        left_id: u32,
        right_id: u32,
    ) -> Result<u32, VMError> {
        if let Some(&cached) = self.merged_schema_cache.get(&(left_id, right_id)) {
            return Ok(cached);
        }

        // Runtime schema synthesis is disabled; merged schemas must exist.
        let merged_name = format!("__merged_{}_{}", left_id, right_id);
        let intersection_name = format!("__intersection_{}_{}", left_id, right_id);
        let merged_id = self
            .lookup_schema_by_name(&merged_name)
            .or_else(|| self.lookup_schema_by_name(&intersection_name))
            .map(|s| s.id)
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "Missing predeclared merged schema for {} + {} (expected '{}' or '{}').",
                    left_id, right_id, merged_name, intersection_name
                ))
            })?;
        self.merged_schema_cache
            .insert((left_id, right_id), merged_id);

        Ok(merged_id)
    }

    /// Derive a subset schema: base schema minus excluded fields.
    /// Uses registry name-based lookup for caching.
    pub(crate) fn derive_subset_schema(
        &mut self,
        base_id: u32,
        exclude: &std::collections::HashSet<String>,
    ) -> Result<u32, VMError> {
        // Build deterministic cache name
        let mut excluded_sorted: Vec<&String> = exclude.iter().collect();
        excluded_sorted.sort();
        let cache_name = format!(
            "__sub_{}_exc_{}",
            base_id,
            excluded_sorted
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );

        // Runtime schema synthesis is disabled; subset schemas must be predeclared.
        if let Some(schema) = self.lookup_schema_by_name(&cache_name) {
            return Ok(schema.id);
        }
        Err(VMError::RuntimeError(format!(
            "Missing predeclared subset schema '{}' (runtime schema derivation is disabled).",
            cache_name
        )))
    }
}
