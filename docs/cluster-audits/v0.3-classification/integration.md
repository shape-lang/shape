# integration classification

**HEAD:** 82f049dd
**Total tests in binary:** 17
**Passed:** 16 / Failed: 1 / Ignored: 0
**Run command:** `direnv exec /home/dev/dev/shape-lang cargo test -p shape-test --test integration --no-fail-fast 2>&1`

## Summary

| Class | Count |
|---|---|
| FN-REG-CORRECTNESS | 1 |
| FN-REG-DIAGNOSTIC  | 0 |
| SCOPE-RECLAIM      | 0 |
| V0.4-DEFER         | 0 |
| INFRA-FLAKY        | 0 |
| UNKNOWN            | 0 |

## Per-test classification

### trait_program_parses_and_has_tokens

Class: **FN-REG-CORRECTNESS**

```
Expected parse ok, got error: ... StructuredParseError { kind: UnexpectedToken { found: TokenInfo { text: "filter", ... }, expected: [] }, ...
  trait Queryable {
      filter(pred): any;       <-- parser rejects this trait body
      select(cols): any
  }
```

- Minimal repro:
  ```shape
  trait Queryable {
      filter(pred): any;
      select(cols): any
  }
  ```
- Bisect: not run (audit-only).
- Affected subsystem: Pest grammar — `trait` body method declarations using `name(params): type;` shape are rejected (parser expects `fn name(...) -> Type;`). Plausibly the test fixture was written against an older grammar shape; OR the grammar previously accepted both forms. Listed as FN-REG-CORRECTNESS because parser rejects the input outright (not a diagnostic-text mismatch). Same parse-failure shape appears in `trait_system::trait_with_default_method_parses`.
