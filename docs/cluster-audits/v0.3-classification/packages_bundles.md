# packages_bundles — all-green at HEAD 82f049dd. No classification needed.

**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test packages_bundles --no-fail-fast 2>&1`
**Result:** 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 278.62s.

Tests (all passed):
- `basic::mod_declaration_parses`
- `basic::bundle_contains_string_pool`
- `basic::compilation_produces_bytecode`
- `basic::content_hash_deterministic_for_same_source`
- `basic::dependency_resolution_concept`
- `basic::package_name_in_manifest_is_valid_identifier`
- `basic::package_version_semver_string`
- `basic::pub_function_is_exported`
