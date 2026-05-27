# module_distribution — all-green at HEAD 82f049dd. No classification needed.

**HEAD:** 82f049dd
**Total tests in binary:** 8
**Passed:** 8 / Failed: 0 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test module_distribution --no-fail-fast 2>&1`

All 8 tests in `basic::` passed (finished in 237.24s):
- `unsigned_module_loads_in_permissive_mode`
- `blob_store_deduplicates_identical_content`
- `content_addressed_blob_concept`
- `manifest_requires_name_field`
- `manifest_version_field`
- `semver_compatible_range`
- `signature_verification_concept`
- `version_conflict_detection_concept`

No FN-REG-CORRECTNESS, FN-REG-DIAGNOSTIC, SCOPE-RECLAIM, V0.4-DEFER, INFRA-FLAKY, or UNKNOWN entries.
