use super::super::*;
use shape_value::ValueWordExt;

impl VirtualMachine {
    // ========================================================================
    // Conversion and Helper Methods

    /// Create a TypedObject from field name-value pairs.
    ///
    /// Create a TypedObject from predeclared compile-time schemas.
    pub(crate) fn create_typed_object_from_pairs(
        &mut self,
        fields: &[(&str, ValueWord)],
    ) -> Result<ValueWord, VMError> {
        // Build field names for schema lookup
        let field_names: Vec<&str> = fields.iter().map(|(k, _)| *k).collect();
        let key: String = field_names.join(",");
        let schema_name = format!("__native_{}", key);

        // Runtime schema synthesis is retired: these object layouts must be
        // predeclared in compile-time registries.
        let schema_id = self
            .lookup_schema_by_name(&schema_name)
            .map(|s| s.id)
            .ok_or_else(|| {
                VMError::RuntimeError(format!(
                    "Missing predeclared schema '{}'. Runtime schema generation is disabled.",
                    schema_name
                ))
            })?;

        let field_types = self.lookup_schema(schema_id).map(|schema| {
            schema
                .fields
                .iter()
                .map(|f| f.field_type.clone())
                .collect::<Vec<_>>()
        });

        // Build slots and heap_mask.
        let mut slots = Vec::with_capacity(fields.len());
        let mut heap_mask: u64 = 0;
        for (i, (_name, nb)) in fields.iter().enumerate() {
            let field_type = field_types.as_ref().and_then(|types| types.get(i));
            let (slot, is_heap) =
                crate::executor::objects::object_creation::nb_to_slot_with_field_type(
                    nb, field_type,
                );
            slots.push(slot);
            if is_heap {
                heap_mask |= 1u64 << i;
            }
        }

        Ok(ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id: schema_id as u64,
            slots: slots.into_boxed_slice(),
            heap_mask,
        }))
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
