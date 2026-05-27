# trait_system classification

**HEAD:** 82f049dd
**Total tests in binary:** 20
**Passed:** 13 / Failed: 7 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test trait_system --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 1 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 6 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### trait_with_default_method_parses

Class: **FN-REG-CORRECTNESS**

```
Expected parse ok, got error: UnexpectedToken { found: TokenInfo { text: "filter", ... }
  trait Queryable {
      filter(pred): any;      <-- parser rejects
      method execute() { ... }
  }
```

- Minimal repro:
  ```shape
  trait Queryable {
      filter(pred): any;
      method execute() { return self }
  }
  ```
- Affected subsystem: Pest grammar — trait body `name(params): type;` declaration shape rejected (same shape as `integration::trait_program_parses_and_has_tokens`). The grammar may have lost an alternative path or the fixture was never current.

### definition_from_trait_name_in_impl

Class: **SCOPE-RECLAIM**

```
Expected definition at (3, 5)
```

- Dated user pull-in: 2026-05-26 LSP-parity-with-rust-analyzer.
- Test asserts LSP "go-to-definition" jumps from a trait name in an `impl Trait for Type` block to the trait definition. LSP regression — failure shape consistent with trait-name navigation not wired through.
- Test asserts on LSP user-facing semantics; stays the same after fix.

### definition_from_method_in_impl

Class: **SCOPE-RECLAIM**

```
Expected definition at (4, 11)
```

- Dated user pull-in: 2026-05-26 LSP-parity. Same family as `definition_from_trait_name_in_impl` — go-to-definition on an impl method name. SCOPE-RECLAIM.

### code_lens_on_trait_definition

Class: **SCOPE-RECLAIM**

```
Expected at least one code lens
```

- Dated user pull-in: 2026-05-26 LSP-parity. Code-lens on trait definition (lists implementors); not currently emitted. LSP territory.

### completion_inside_impl_suggests_methods

Class: **SCOPE-RECLAIM**

```
Expected completion 'select' in []
```

- Dated user pull-in: 2026-05-26 LSP-parity. Completion engine should suggest unimplemented trait methods inside `impl Trait for Type { ... }`; returns empty list. LSP territory.

### hover_on_impl_method_shows_trait_info

Class: **SCOPE-RECLAIM**

```
Expected hover at (4, 11)
```

- Dated user pull-in: 2026-05-26 LSP-parity. Hover on impl method should surface the originating trait. Same LSP family.

### hover_on_bounded_type_param

Class: **SCOPE-RECLAIM**

```
Expected hover at (3, 7)
```

- Dated user pull-in: 2026-05-26 LSP-parity. Hover on a bounded type-param like `T: Display` should surface the bound. LSP territory.
